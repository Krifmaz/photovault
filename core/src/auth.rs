//! Login, exactly as the Ente apps do it, as resumable steps:
//!
//! 1. `GET /users/srp/attributes` gives the SRP identity/salt and the
//!    Argon2id parameters of the key-encryption key (KEK). With SRP (and no
//!    email MFA) the KEK's login sub-key is the SRP password for
//!    `create-session`/`verify-session`; otherwise an email code is sent
//!    (`/users/ott`, `/users/verify-email`).
//! 2. Second factor: TOTP via `/users/two-factor/verify`; passkeys are
//!    handed to the app as a URL.
//! 3. The master key is opened with the KEK (or the recovery key), the
//!    secret key with the master key, the sealed auth token with the key pair.
//! 4. Recovery (`recover_with_key`) then sets a new password as the apps'
//!    change-password does: new KEK (Ente's "sensitive" Argon2id), new SRP
//!    verifier via `/users/srp/setup` + `/users/srp/update`.
//!
//! Adapted from `importers/src/ente/auth.rs` (branch `claude/project-thread-cp9x81`)
//! and `node/src/api/auth.rs` (branch `claude/project-thread-bmpbpi`).

use crate::api::Api;
use crate::client::{Inner, Session};
use crate::crypto;
use crate::model::*;
use crate::srp::SrpClient;
use crate::{Account, CoreError, LoginStep, Result, StorageUsage};
use base64::Engine as _;
use reqwest::Method;

/// Ente's placeholder for "unlimited" storage (contracts/storage-usage.md).
const UNLIMITED_BYTES: i64 = 109_951_162_777_600;

/// A login between steps.
#[derive(Default)]
pub(crate) struct Pending {
    email: String,
    /// The KEK, once derived from the password.
    kek: Option<Vec<u8>>,
    /// The last authorization answer (for `NeedsRecoveryKey`).
    auth: Option<AuthorizationResponse>,
}

impl Pending {
    fn new(email: String) -> Self {
        Self {
            email,
            kek: None,
            auth: None,
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(k) = self.kek.as_mut() {
            crypto::wipe(k);
        }
    }
}

/// Argon2id off the async threads.
async fn derive_kek(password: &str, salt_b64: &str, ops: u64, mem: u64) -> Result<Vec<u8>> {
    let pw = password.as_bytes().to_vec();
    let salt = crypto::unb64(salt_b64)?;
    tokio::task::spawn_blocking(move || crypto::derive_argon_key(&pw, &salt, ops, mem))
        .await
        .map_err(|e| CoreError::crypto(e.to_string()))?
        .map_err(Into::into)
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// A v4 UUID from libsodium randomness (SRP identities).
fn uuid_v4() -> String {
    let mut b = crypto::random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Master key, secret key, public key and token of an unlocked account.
type Unlocked = (Vec<u8>, Vec<u8>, Vec<u8>, String);

/// Open the account keys and the token. `Err(IncorrectPassword)` if `kek`
/// does not open the master key.
fn unlock_with_master(
    master_key: Vec<u8>,
    ka: &KeyAttributes,
    auth: &AuthorizationResponse,
) -> Result<Unlocked> {
    let secret_key = crypto::secretbox_open(
        &crypto::unb64(&ka.encrypted_secret_key)?,
        &crypto::unb64(&ka.secret_key_decryption_nonce)?,
        &master_key,
    )
    .map_err(|_| CoreError::crypto("the secret key does not decrypt"))?;
    let public_key = crypto::unb64(&ka.public_key)?;
    let token = match (non_empty(&auth.encrypted_token), non_empty(&auth.token)) {
        (Some(et), _) => {
            let mut raw = crypto::box_seal_open(&crypto::unb64(et)?, &public_key, &secret_key)
                .map_err(|_| CoreError::crypto("the auth token does not decrypt"))?;
            // Museum encodes the token URL-safe with padding (Go's URLEncoding).
            let t = base64::engine::general_purpose::URL_SAFE.encode(&raw);
            crypto::wipe(&mut raw);
            t
        }
        (None, Some(t)) => t.to_string(),
        _ => return Err(CoreError::crypto("the server sent no auth token")),
    };
    Ok((master_key, secret_key, public_key, token))
}

/// Decrypt the master key with the KEK.
fn open_master(kek: &[u8], ka: &KeyAttributes) -> Result<Vec<u8>> {
    crypto::secretbox_open(
        &crypto::unb64(&ka.encrypted_key)?,
        &crypto::unb64(&ka.key_decryption_nonce)?,
        kek,
    )
    .map_err(|_| CoreError::IncorrectPassword)
}

impl Inner {
    fn set_pending(&self, p: Pending) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
    }

    fn take_pending(&self) -> Option<Pending> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginStep> {
        crypto::init();
        let email = email.trim().to_string();
        let api = self.anon_api()?;
        let attrs = match api
            .get_json::<SrpAttributesResponse>("/users/srp/attributes", &[("email", email.clone())])
            .await
        {
            Ok(r) => Some(r.attributes),
            Err(e) if e.status() == Some(404) => None,
            Err(e) => return Err(e),
        };
        let Some(a) = attrs.filter(|a| !a.is_email_mfa_enabled) else {
            // No SRP (or email MFA): the email code is the first factor.
            self.send_ott(&api, &email).await?;
            self.set_pending(Pending::new(email));
            return Ok(LoginStep::EmailOtt);
        };
        let kek = derive_kek(password, &a.kek_salt, a.ops_limit, a.mem_limit).await?;
        let login_key = crypto::derive_login_key(&kek)?;
        let salt = crypto::unb64(&a.srp_salt)?;
        let mut client = SrpClient::new(a.srp_user_id.as_bytes(), &salt, &login_key);
        let session: CreateSrpSessionResponse = api
            .json(
                Method::POST,
                "/users/srp/create-session",
                &CreateSrpSessionRequest {
                    srp_user_id: &a.srp_user_id,
                    srp_a: crypto::b64(&client.public_a()),
                },
            )
            .await?;
        let m1 = client.compute_m1(&crypto::unb64(&session.srp_b)?)?;
        let auth: AuthorizationResponse = match api
            .json(
                Method::POST,
                "/users/srp/verify-session",
                &VerifySrpSessionRequest {
                    srp_user_id: &a.srp_user_id,
                    session_id: &session.session_id,
                    srp_m1: crypto::b64(&m1),
                },
            )
            .await
        {
            Ok(r) => r,
            Err(e) if matches!(e.status(), Some(401 | 403)) => {
                return Err(CoreError::IncorrectPassword);
            }
            Err(e) => return Err(e),
        };
        if let Some(m2) = non_empty(&auth.srp_m2) {
            client.verify_m2(&crypto::unb64(m2)?).map_err(|_| {
                CoreError::crypto("the server failed to prove it knows the SRP verifier")
            })?;
        }
        self.after_first_factor(
            Pending {
                email,
                kek: Some(kek),
                auth: None,
            },
            auth,
            password,
        )
        .await
    }

    async fn send_ott(&self, api: &Api, email: &str) -> Result<()> {
        api.json_no_content(
            Method::POST,
            "/users/ott",
            &SendOttRequest {
                email,
                purpose: "login",
            },
        )
        .await
    }

    pub async fn send_email_ott(&self, email: &str) -> Result<()> {
        let api = self.anon_api()?;
        let email = email.trim();
        self.send_ott(&api, email).await?;
        let keep = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|p| p.email == email);
        if !keep {
            self.set_pending(Pending::new(email.to_string()));
        }
        Ok(())
    }

    pub async fn verify_email_ott(
        &self,
        email: &str,
        code: &str,
        password: &str,
    ) -> Result<LoginStep> {
        let api = self.anon_api()?;
        let email = email.trim().to_string();
        let auth: AuthorizationResponse = match api
            .json(
                Method::POST,
                "/users/verify-email",
                &VerifyEmailRequest {
                    email: &email,
                    ott: code.trim(),
                },
            )
            .await
        {
            Ok(r) => r,
            Err(e) if matches!(e.status(), Some(400 | 401 | 403 | 404 | 410)) => {
                return Err(CoreError::IncorrectCode);
            }
            Err(e) => return Err(e),
        };
        let pending = self
            .take_pending()
            .filter(|p| p.email == email)
            .unwrap_or_else(|| Pending::new(email));
        self.after_first_factor(pending, auth, password).await
    }

    pub async fn verify_totp(
        &self,
        session_id: &str,
        code: &str,
        password: &str,
    ) -> Result<LoginStep> {
        let api = self.anon_api()?;
        let auth: AuthorizationResponse = match api
            .json(
                Method::POST,
                "/users/two-factor/verify",
                &TwoFactorVerifyRequest {
                    session_id,
                    code: code.trim(),
                },
            )
            .await
        {
            Ok(r) => r,
            Err(e) if matches!(e.status(), Some(400 | 401 | 403 | 404 | 410)) => {
                return Err(CoreError::IncorrectCode);
            }
            Err(e) => return Err(e),
        };
        let pending = self.take_pending().unwrap_or_default();
        self.after_first_factor(pending, auth, password).await
    }

    /// Second factor, then key decryption.
    async fn after_first_factor(
        &self,
        mut p: Pending,
        auth: AuthorizationResponse,
        password: &str,
    ) -> Result<LoginStep> {
        if let Some(sid) = non_empty(&auth.passkey_session_id) {
            let fallback = format!("{}/accounts", self.active_base_url().unwrap_or_default());
            let accounts = non_empty(&auth.accounts_url)
                .unwrap_or(&fallback)
                .trim_end_matches('/');
            let url = format!(
                "{accounts}/passkeys/verify?passkeySessionID={sid}&redirect=photovault://passkey&clientPackage={}",
                crate::api::CLIENT_PACKAGE
            );
            let step = LoginStep::Passkey {
                session_id: sid.to_string(),
                url,
            };
            self.set_pending(p);
            return Ok(step);
        }
        if let Some(sid) = non_empty(&auth.two_factor_session_id) {
            let step = LoginStep::TwoFactorTotp {
                session_id: sid.to_string(),
            };
            self.set_pending(p);
            return Ok(step);
        }
        let Some(ka) = auth.key_attributes.clone() else {
            return Err(CoreError::crypto(
                "this account has no keys yet: finish setting it up in an app first",
            ));
        };
        if p.kek.is_none() && password.is_empty() {
            p.auth = Some(auth);
            self.set_pending(p);
            return Ok(LoginStep::NeedsRecoveryKey);
        }
        let kek = match p.kek.take() {
            Some(k) => k,
            None => derive_kek(password, &ka.kek_salt, ka.ops_limit, ka.mem_limit).await?,
        };
        let master = match open_master(&kek, &ka) {
            Ok(m) => m,
            Err(e) => {
                // Keep the flow alive so the user can retry the password or recover.
                p.auth = Some(auth);
                self.set_pending(p);
                return Err(e);
            }
        };
        let (master_key, secret_key, public_key, token) = unlock_with_master(master, &ka, &auth)?;
        let account = self.finish(
            p.email.clone(),
            auth.id,
            token,
            master_key,
            secret_key,
            public_key,
            ka,
        )?;
        Ok(LoginStep::Done { account })
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        email: String,
        user_id: i64,
        token: String,
        master_key: Vec<u8>,
        secret_key: Vec<u8>,
        public_key: Vec<u8>,
        key_attributes: KeyAttributes,
    ) -> Result<Account> {
        let server_url = self
            .active_public_url()
            .or_else(|| self.active_base_url())
            .unwrap_or_default();
        let account = Account {
            user_id,
            email,
            server_url,
        };
        self.set_session(Session {
            account: account.clone(),
            token,
            master_key,
            secret_key,
            public_key,
            key_attributes,
        })?;
        Ok(account)
    }

    pub async fn recover_with_key(
        &self,
        recovery_key: &str,
        new_password: &str,
    ) -> Result<Account> {
        if new_password.is_empty() {
            return Err(CoreError::IncorrectPassword);
        }
        let mut p = self.take_pending().ok_or(CoreError::NotLoggedIn)?;
        let Some(auth) = p.auth.take() else {
            self.set_pending(p);
            return Err(CoreError::NotLoggedIn);
        };
        let restore = |p: Pending, auth: AuthorizationResponse| {
            let mut p = p;
            p.auth = Some(auth);
            self.set_pending(p);
        };
        let ka = auth.key_attributes.clone().ok_or(CoreError::NotLoggedIn)?;
        let (Some(enc), Some(nonce)) = (
            ka.master_key_encrypted_with_recovery_key.as_deref(),
            ka.master_key_decryption_nonce.as_deref(),
        ) else {
            restore(p, auth);
            return Err(CoreError::crypto("this account has no recovery key"));
        };
        let Ok(mut rk) = crypto::parse_recovery_key(recovery_key) else {
            restore(p, auth);
            return Err(CoreError::IncorrectCode);
        };
        let master = crypto::secretbox_open(&crypto::unb64(enc)?, &crypto::unb64(nonce)?, &rk);
        crypto::wipe(&mut rk);
        let Ok(master) = master else {
            restore(p, auth);
            return Err(CoreError::IncorrectCode);
        };
        let (master_key, secret_key, public_key, token) = unlock_with_master(master, &ka, &auth)?;

        // Set the new password (Ente's change-password with the recovered keys).
        let api = self.anon_api()?.with_token(token.clone());
        let new_ka = set_password(&api, &master_key, &ka, new_password).await?;
        let account = self.finish(
            p.email.clone(),
            auth.id,
            token,
            master_key,
            secret_key,
            public_key,
            new_ka,
        )?;
        Ok(account)
    }

    pub async fn logout(&self) -> Result<()> {
        if let Ok(api) = self.api() {
            let _ = api
                .json_no_content(Method::POST, "/users/logout", &serde_json::json!({}))
                .await;
        }
        let _sync = self.sync_lock.lock().await;
        let _up = self.upload_lock.lock().await;
        self.store().wipe_account()?;
        self.wipe_cache();
        *self.session.write().unwrap_or_else(|e| e.into_inner()) = None;
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.rebuild_index()
    }

    pub async fn storage_usage(&self) -> Result<StorageUsage> {
        let api = self.api()?;
        match api
            .get_json::<PhotovaultUsage>("/photovault/usage", &[])
            .await
        {
            Ok(u) => {
                let limit = if u.unlimited { None } else { u.limit_bytes };
                let free = if u.unlimited {
                    None
                } else {
                    u.free_bytes
                        .or_else(|| limit.map(|l| l.saturating_sub(u.used_bytes)))
                };
                return Ok(StorageUsage {
                    used_bytes: u.used_bytes,
                    limit_bytes: limit,
                    free_bytes: free,
                });
            }
            Err(e) if matches!(e.status(), Some(404 | 405 | 501)) => {}
            Err(e) => return Err(e),
        }
        let d: UserDetails = api
            .get_json("/users/details/v2", &[("memoryCount", "false".to_string())])
            .await?;
        Ok(usage_from_details(&d))
    }
}

/// `used`/`limit` from `/users/details/v2`, as the apps compute it.
pub(crate) fn usage_from_details(d: &UserDetails) -> StorageUsage {
    let (used, limit) = match &d.family_data {
        Some(f) if !f.members.is_empty() => (
            f.members.iter().map(|m| m.usage.max(0)).sum::<i64>(),
            f.storage + d.storage_bonus,
        ),
        _ => (
            d.usage,
            d.subscription.as_ref().map(|s| s.storage).unwrap_or(0) + d.storage_bonus,
        ),
    };
    let used = used.max(0) as u64;
    let limit = (limit > 0 && limit < UNLIMITED_BYTES).then_some(limit as u64);
    StorageUsage {
        used_bytes: used,
        limit_bytes: limit,
        free_bytes: limit.map(|l| l.saturating_sub(used)),
    }
}

/// Re-key the password: new KEK, new SRP verifier, `/users/srp/setup` and
/// `/users/srp/update`. Returns the updated key attributes.
pub(crate) async fn set_password(
    api: &Api,
    master_key: &[u8],
    ka: &KeyAttributes,
    password: &str,
) -> Result<KeyAttributes> {
    let pw = password.as_bytes().to_vec();
    let salt = crypto::random_bytes(16);
    let s2 = salt.clone();
    let (mut kek, ops, mem) =
        tokio::task::spawn_blocking(move || crypto::derive_sensitive_key(&pw, &s2))
            .await
            .map_err(|e| CoreError::crypto(e.to_string()))??;
    let (enc_key, nonce) = crypto::secretbox_seal(master_key, &kek)?;
    let login_key = crypto::derive_login_key(&kek)?;
    crypto::wipe(&mut kek);
    let srp_user_id = uuid_v4();
    let srp_salt = crypto::random_bytes(16);
    let verifier = crate::srp::verifier(srp_user_id.as_bytes(), &srp_salt, &login_key);
    let mut client = SrpClient::new(srp_user_id.as_bytes(), &srp_salt, &login_key);
    let setup: SetupSrpResponse = api
        .json(
            Method::POST,
            "/users/srp/setup",
            &SetupSrpRequest {
                srp_user_id: srp_user_id.clone(),
                srp_salt: crypto::b64(&srp_salt),
                srp_verifier: crypto::b64(&verifier),
                srp_a: crypto::b64(&client.public_a()),
            },
        )
        .await?;
    let m1 = client.compute_m1(&crypto::unb64(&setup.srp_b)?)?;
    let updated = UpdatedKeyAttributes {
        kek_salt: crypto::b64(&salt),
        encrypted_key: crypto::b64(&enc_key),
        key_decryption_nonce: crypto::b64(&nonce),
        mem_limit: mem,
        ops_limit: ops,
    };
    let res: UpdateSrpAndKeysResponse = api
        .json(
            Method::POST,
            "/users/srp/update",
            &UpdateSrpAndKeysRequest {
                setup_id: setup.setup_id,
                srp_m1: crypto::b64(&m1),
                updated_key_attr: updated.clone(),
                log_out_other_devices: false,
            },
        )
        .await?;
    if let Some(m2) = non_empty(&res.srp_m2) {
        client.verify_m2(&crypto::unb64(m2)?).map_err(|_| {
            CoreError::crypto("the server failed to prove it stored the new verifier")
        })?;
    }
    let mut out = ka.clone();
    out.kek_salt = updated.kek_salt;
    out.encrypted_key = updated.encrypted_key;
    out.key_decryption_nonce = updated.key_decryption_nonce;
    out.mem_limit = mem;
    out.ops_limit = ops;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_v4() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn usage_rules() {
        let d = UserDetails {
            usage: 10,
            subscription: Some(Subscription { storage: 100 }),
            storage_bonus: 5,
            ..Default::default()
        };
        assert_eq!(
            usage_from_details(&d),
            StorageUsage {
                used_bytes: 10,
                limit_bytes: Some(105),
                free_bytes: Some(95)
            }
        );
        let d = UserDetails {
            usage: 10,
            subscription: Some(Subscription {
                storage: UNLIMITED_BYTES,
            }),
            ..Default::default()
        };
        assert_eq!(usage_from_details(&d).limit_bytes, None);
        let d = UserDetails {
            usage: 1,
            family_data: Some(FamilyData {
                members: vec![FamilyMember { usage: 3 }, FamilyMember { usage: 4 }],
                storage: 50,
            }),
            ..Default::default()
        };
        assert_eq!(usage_from_details(&d).used_bytes, 7);
        assert_eq!(usage_from_details(&d).limit_bytes, Some(50));
    }
}
