//! Create an account, exactly as Ente's apps do it (contract v1.2, "signup").
//!
//! The steps and their wire formats follow Ente's current account code
//! (`rust/crates/accounts/src/signup.rs` and `auth/key_gen.rs`, used by the
//! photos app through `UserService.generateKey` / `setAttributes`):
//!
//! 1. `POST /users/ott` with `purpose: "signup"` emails a code
//!    ([`Core::signup_send_code`]); a server that closed sign-ups is refused
//!    before anything is sent.
//! 2. `POST /users/verify-email` checks it ([`Core::signup_verify_code`]). A new
//!    account answers with a plain `token` and no key attributes.
//! 3. [`Core::signup_prepare`] makes the keys on the device, nothing leaves it:
//!    a random master key and recovery key, each secretbox'd under the other;
//!    the key-encryption key from the password with Ente's "sensitive"
//!    Argon2id ([`derive_sensitive_kek`]) and the master key secretbox'd under
//!    it; an X25519 key pair with the secret key secretbox'd under the master
//!    key; the SRP login sub-key of the KEK, a fresh SRP identity (UUID v4),
//!    salt and verifier. It returns the recovery key as 24 BIP39 words, which
//!    the app shows before anything is uploaded (Ente's `RecoveryKeyPage`).
//! 4. [`Core::signup_finish`] uploads: `PUT /users/attributes` with the key
//!    attributes (after `GET /users/session-validity/v2`, so a retry after a
//!    lost answer does not fail), then `POST /users/srp/setup` +
//!    `POST /users/srp/complete` (the server's `M2` is checked), then
//!    `GET /users/srp/attributes` must show what was set up. The account is
//!    then signed in on this device.
//!
//! The account works in the stock Ente apps and the other Photovault app.

use crate::api::Api;
use crate::client::Session;
use crate::crypto;
use crate::model::{
    AuthorizationResponse, KeyAttributes, SetupSrpRequest, SetupSrpResponse, SrpAttributes,
    SrpAttributesResponse,
};
use crate::srp::SrpClient;
use crate::{Account, Core, CoreError, Result, run};
use reqwest::Method;
use serde::{Deserialize, Serialize};

/// libsodium's `crypto_pwhash_MEMLIMIT_SENSITIVE` (1 GiB).
pub const MEMLIMIT_SENSITIVE: u64 = 1024 * 1024 * 1024;
/// libsodium's `crypto_pwhash_OPSLIMIT_SENSITIVE` (4).
pub const OPSLIMIT_SENSITIVE: u64 = 4;
/// libsodium's `crypto_pwhash_MEMLIMIT_MODERATE` (256 MiB): where low-memory
/// devices start, with the ops limit raised so the product stays the same.
pub const MEMLIMIT_MODERATE: u64 = 256 * 1024 * 1024;
/// The lowest memory limit Ente's server accepts for account keys (128 MiB);
/// Ente's account code stops halving there.
pub const MEMLIMIT_ACCOUNT_MIN: u64 = 128 * 1024 * 1024;

/// What the server says when sign-ups are closed.
const SIGNUPS_CLOSED: &str = "this server is not accepting new accounts";
/// What the server says when the email already has an account.
const ALREADY_REGISTERED: &str = "an account with this email already exists";

fn signups_closed() -> CoreError {
    CoreError::Server {
        status: 403,
        message: SIGNUPS_CLOSED.into(),
    }
}

fn already_registered() -> CoreError {
    CoreError::Server {
        status: 409,
        message: ALREADY_REGISTERED.into(),
    }
}

/// The Argon2id limits Ente's `deriveSensitiveKey` starts with: libsodium's
/// "sensitive" limits (1 GiB, 4 ops), or on a low-memory device 256 MiB with
/// 16 ops (the same work, as `ente_crypto_dart` does on low-spec devices and
/// Ente's Rust account code always does). Returns `(ops_limit, mem_limit)`.
pub fn sensitive_start(low_memory: bool) -> (u64, u64) {
    if low_memory {
        (
            OPSLIMIT_SENSITIVE * (MEMLIMIT_SENSITIVE / MEMLIMIT_MODERATE),
            MEMLIMIT_MODERATE,
        )
    } else {
        (OPSLIMIT_SENSITIVE, MEMLIMIT_SENSITIVE)
    }
}

/// Ente's `deriveSensitiveKey` with the derivation passed in: start at
/// [`sensitive_start`], and while the derivation fails (out of memory) halve
/// the memory and double the ops, down to [`MEMLIMIT_ACCOUNT_MIN`]. The
/// product `mem × ops` is always 4 GiB, Ente's required strength. Returns
/// `(key, ops_limit, mem_limit)`.
pub fn derive_sensitive_with<E>(
    low_memory: bool,
    mut derive: impl FnMut(u64, u64) -> std::result::Result<Vec<u8>, E>,
) -> std::result::Result<(Vec<u8>, u64, u64), E> {
    let (mut ops, mut mem) = sensitive_start(low_memory);
    loop {
        match derive(ops, mem) {
            Ok(k) => return Ok((k, ops, mem)),
            Err(_) if mem / 2 >= MEMLIMIT_ACCOUNT_MIN => {
                mem /= 2;
                ops *= 2;
            }
            Err(e) => return Err(e),
        }
    }
}

/// The key-encryption key of a new password ([`derive_sensitive_with`] over
/// libsodium's Argon2id). Returns `(key, ops_limit, mem_limit)`.
pub fn derive_sensitive_kek(
    password: &[u8],
    salt: &[u8],
    low_memory: bool,
) -> crypto::Result<(Vec<u8>, u64, u64)> {
    derive_sensitive_with(low_memory, |ops, mem| {
        crypto::derive_argon_key(password, salt, ops, mem)
    })
}

/// A sign-up between its steps (kept in memory only).
pub(crate) struct PendingSignup {
    email: String,
    user_id: i64,
    /// The plain token from `verify-email`.
    token: String,
    keys: Option<PreparedKeys>,
}

/// Keys made by [`Core::signup_prepare`].
struct PreparedKeys {
    attributes: KeyAttributes,
    master_key: Vec<u8>,
    secret_key: Vec<u8>,
    public_key: Vec<u8>,
    recovery_key: Vec<u8>,
    login_key: Vec<u8>,
    srp_user_id: String,
    srp_salt: Vec<u8>,
    srp_verifier: Vec<u8>,
    /// `PUT /users/attributes` succeeded: the keys can no longer change.
    uploaded: bool,
}

impl Drop for PreparedKeys {
    fn drop(&mut self) {
        crypto::wipe(&mut self.master_key);
        crypto::wipe(&mut self.secret_key);
        crypto::wipe(&mut self.recovery_key);
        crypto::wipe(&mut self.login_key);
    }
}

/// Ente's `generate_keys_with_strength` + `generate_srp_setup_with_login_key`.
fn generate_keys(password: &[u8], low_memory: bool) -> Result<PreparedKeys> {
    crypto::init();
    let mut master_key = crypto::generate_key();
    let mut recovery_key = crypto::generate_key();
    let made = (|| -> Result<PreparedKeys> {
        let (mk_rk, mk_rk_nonce) = crypto::secretbox_seal(&master_key, &recovery_key)?;
        let (rk_mk, rk_mk_nonce) = crypto::secretbox_seal(&recovery_key, &master_key)?;
        let kek_salt = crypto::random_bytes(16);
        let (mut kek, ops, mem) =
            derive_sensitive_kek(password, &kek_salt, low_memory).map_err(|_| {
                CoreError::crypto("this device does not have enough memory to make secure keys")
            })?;
        let login_key = crypto::derive_login_key(&kek);
        let sealed = crypto::secretbox_seal(&master_key, &kek);
        crypto::wipe(&mut kek);
        let login_key = login_key?;
        let (enc_key, key_nonce) = sealed?;
        let (public_key, mut secret_key) = crypto::box_keypair();
        let (enc_sk, sk_nonce) = match crypto::secretbox_seal(&secret_key, &master_key) {
            Ok(v) => v,
            Err(e) => {
                crypto::wipe(&mut secret_key);
                return Err(e.into());
            }
        };
        let srp_user_id = crate::auth::uuid_v4();
        let srp_salt = crypto::random_bytes(16);
        let srp_verifier = crate::srp::verifier(srp_user_id.as_bytes(), &srp_salt, &login_key);
        let attributes = KeyAttributes {
            kek_salt: crypto::b64(&kek_salt),
            encrypted_key: crypto::b64(&enc_key),
            key_decryption_nonce: crypto::b64(&key_nonce),
            public_key: crypto::b64(&public_key),
            encrypted_secret_key: crypto::b64(&enc_sk),
            secret_key_decryption_nonce: crypto::b64(&sk_nonce),
            mem_limit: mem,
            ops_limit: ops,
            master_key_encrypted_with_recovery_key: Some(crypto::b64(&mk_rk)),
            master_key_decryption_nonce: Some(crypto::b64(&mk_rk_nonce)),
            recovery_key_encrypted_with_master_key: Some(crypto::b64(&rk_mk)),
            recovery_key_decryption_nonce: Some(crypto::b64(&rk_mk_nonce)),
        };
        Ok(PreparedKeys {
            attributes,
            master_key: std::mem::take(&mut master_key),
            secret_key,
            public_key,
            recovery_key: std::mem::take(&mut recovery_key),
            login_key,
            srp_user_id,
            srp_salt,
            srp_verifier,
            uploaded: false,
        })
    })();
    crypto::wipe(&mut master_key);
    crypto::wipe(&mut recovery_key);
    made
}

/// Whether `password` opens `keys` (same KEK salt and limits).
fn password_opens(password: &[u8], keys: &PreparedKeys) -> Result<bool> {
    let a = &keys.attributes;
    let mut kek = crypto::derive_argon_key(
        password,
        &crypto::unb64(&a.kek_salt)?,
        a.ops_limit,
        a.mem_limit,
    )?;
    let opened = crypto::secretbox_open(
        &crypto::unb64(&a.encrypted_key)?,
        &crypto::unb64(&a.key_decryption_nonce)?,
        &kek,
    );
    crypto::wipe(&mut kek);
    Ok(match opened {
        Ok(mut m) => {
            let same = crypto::ct_eq(&m, &keys.master_key);
            crypto::wipe(&mut m);
            same
        }
        Err(_) => false,
    })
}

/// `POST /users/ott` request with Ente's `mobile` flag.
#[derive(Serialize)]
struct SignupOttRequest<'a> {
    email: &'a str,
    purpose: &'a str,
    mobile: bool,
}

/// `GET /users/session-validity/v2` answer.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct SessionValidity {
    has_set_keys: bool,
    key_attributes: Option<KeyAttributes>,
}

/// `PUT /users/attributes` request.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetAttributesRequest<'a> {
    key_attributes: &'a KeyAttributes,
}

/// `POST /users/srp/complete` request.
#[derive(Serialize)]
struct CompleteSrpSetupRequest<'a> {
    #[serde(rename = "setupID")]
    setup_id: &'a str,
    #[serde(rename = "srpM1")]
    srp_m1: String,
}

/// `POST /users/srp/complete` answer.
#[derive(Deserialize)]
struct CompleteSrpSetupResponse {
    #[serde(rename = "srpM2", default)]
    srp_m2: Option<String>,
}

fn same_keys(a: &KeyAttributes, b: &KeyAttributes) -> bool {
    a.kek_salt == b.kek_salt
        && a.encrypted_key == b.encrypted_key
        && a.key_decryption_nonce == b.key_decryption_nonce
        && a.public_key == b.public_key
        && a.encrypted_secret_key == b.encrypted_secret_key
        && a.mem_limit == b.mem_limit
        && a.ops_limit == b.ops_limit
}

impl crate::client::Inner {
    fn take_signup(&self) -> Option<PendingSignup> {
        self.signup.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    fn put_signup(&self, s: PendingSignup) {
        *self.signup.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
    }

    /// The active server's sign-up setting, asked fresh (the saved copy as a
    /// fallback when the info endpoint does not answer).
    async fn signups_allowed(&self) -> Result<bool> {
        let base = self.active_base_url().ok_or_else(|| CoreError::Network {
            message: "no server selected".into(),
        })?;
        match crate::server::fetch_server_info(&base).await {
            Ok(info) => Ok(info.signups_allowed),
            Err(e) => {
                let public = self.active_public_url();
                self.saved_servers()
                    .into_iter()
                    .find(|s| Some(&s.public_url) == public.as_ref())
                    .map(|s| s.signups_allowed)
                    .ok_or(e)
            }
        }
    }

    pub(crate) async fn signup_send_code(&self, email: &str) -> Result<()> {
        let email = email.trim();
        if email.is_empty() {
            return Err(CoreError::Server {
                status: 400,
                message: "enter an email address".into(),
            });
        }
        if !self.signups_allowed().await? {
            return Err(signups_closed());
        }
        let api = self.anon_api()?;
        match api
            .json_no_content(
                Method::POST,
                "/users/ott",
                &SignupOttRequest {
                    email,
                    purpose: "signup",
                    mobile: true,
                },
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if e.status() == Some(409) => Err(already_registered()),
            Err(e) if e.status() == Some(403) => Err(signups_closed()),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn signup_verify_code(&self, email: &str, code: &str) -> Result<()> {
        let email = email.trim().to_string();
        let api = self.anon_api()?;
        let auth: AuthorizationResponse = match api
            .json(
                Method::POST,
                "/users/verify-email",
                &crate::model::VerifyEmailRequest {
                    email: &email,
                    ott: code.trim(),
                },
            )
            .await
        {
            Ok(r) => r,
            Err(e) if e.status() == Some(403) => return Err(signups_closed()),
            Err(e) if matches!(e.status(), Some(400 | 401 | 404 | 410)) => {
                return Err(CoreError::IncorrectCode);
            }
            Err(e) => return Err(e),
        };
        // Ente's `Signup::verified`: an account with keys (or a second
        // factor) is an existing account, not a new one.
        let nonempty = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.is_empty());
        if auth.key_attributes.is_some()
            || nonempty(&auth.encrypted_token)
            || nonempty(&auth.two_factor_session_id)
            || nonempty(&auth.passkey_session_id)
        {
            return Err(already_registered());
        }
        let token = auth
            .token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| CoreError::protocol("the server sent no token"))?;
        self.put_signup(PendingSignup {
            email,
            user_id: auth.id,
            token,
            keys: None,
        });
        Ok(())
    }

    pub(crate) async fn signup_prepare(&self, password: &str, low_memory: bool) -> Result<String> {
        if password.is_empty() {
            return Err(CoreError::IncorrectPassword);
        }
        let mut s = self.take_signup().ok_or(CoreError::NotLoggedIn)?;
        let pw = password.as_bytes().to_vec();
        let old = s.keys.take();
        let (keys, result) = tokio::task::spawn_blocking(move || {
            let mut pw = pw;
            let out = match old {
                // Same password again (the user came back): keep the keys.
                Some(k) => match password_opens(&pw, &k) {
                    Ok(true) => (Some(k), Ok(())),
                    Ok(false) if !k.uploaded => match generate_keys(&pw, low_memory) {
                        Ok(n) => (Some(n), Ok(())),
                        Err(e) => (None, Err(e)),
                    },
                    // Uploaded keys are final: only their password opens them.
                    Ok(false) => (Some(k), Err(CoreError::IncorrectPassword)),
                    Err(e) => (Some(k), Err(e)),
                },
                None => match generate_keys(&pw, low_memory) {
                    Ok(n) => (Some(n), Ok(())),
                    Err(e) => (None, Err(e)),
                },
            };
            crypto::wipe(&mut pw);
            out
        })
        .await
        .map_err(|e| CoreError::crypto(e.to_string()))?;
        s.keys = keys;
        let words = s
            .keys
            .as_ref()
            .map(|k| crypto::recovery_key_to_mnemonic(&k.recovery_key));
        self.put_signup(s);
        result?;
        Ok(words.ok_or(CoreError::NotLoggedIn)??)
    }

    pub(crate) async fn signup_finish(&self) -> Result<Account> {
        let mut s = self.take_signup().ok_or(CoreError::NotLoggedIn)?;
        if s.keys.is_none() {
            self.put_signup(s);
            return Err(CoreError::NotLoggedIn);
        }
        let api = self.anon_api()?.with_token(s.token.clone());
        let r = upload(&api, &s.email, s.keys.as_mut().expect("checked")).await;
        if let Err(e) = r {
            self.put_signup(s);
            return Err(e);
        }
        let keys = s.keys.take().expect("checked");
        let server_url = self
            .active_public_url()
            .or_else(|| self.active_base_url())
            .unwrap_or_default();
        let account = Account {
            user_id: s.user_id,
            email: s.email.clone(),
            server_url,
        };
        self.set_session(Session {
            account: account.clone(),
            token: std::mem::take(&mut s.token),
            master_key: keys.master_key.clone(),
            secret_key: keys.secret_key.clone(),
            public_key: keys.public_key.clone(),
            key_attributes: keys.attributes.clone(),
        })?;
        // A sign-in that was half done before is void now.
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(account)
    }

    /// The signed-in account's recovery key as 24 words.
    pub(crate) fn recovery_key_words(&self) -> Result<String> {
        let s = self.session()?;
        let ka = &s.key_attributes;
        let (Some(enc), Some(nonce)) = (
            ka.recovery_key_encrypted_with_master_key.as_deref(),
            ka.recovery_key_decryption_nonce.as_deref(),
        ) else {
            return Err(CoreError::crypto("this account has no recovery key"));
        };
        let mut rk =
            crypto::secretbox_open(&crypto::unb64(enc)?, &crypto::unb64(nonce)?, &s.master_key)
                .map_err(|_| CoreError::crypto("the recovery key does not decrypt"))?;
        let words = crypto::recovery_key_to_mnemonic(&rk);
        crypto::wipe(&mut rk);
        Ok(words?)
    }
}

/// Ente's `complete_signup`: key attributes, SRP, then check what the server
/// stored.
async fn upload(api: &Api, email: &str, keys: &mut PreparedKeys) -> Result<()> {
    if !keys.uploaded {
        let remote: SessionValidity = api.get_json("/users/session-validity/v2", &[]).await?;
        match (remote.has_set_keys, remote.key_attributes) {
            (false, None) => {
                api.json_no_content(
                    Method::PUT,
                    "/users/attributes",
                    &SetAttributesRequest {
                        key_attributes: &keys.attributes,
                    },
                )
                .await?;
            }
            (true, Some(a)) if same_keys(&a, &keys.attributes) => {}
            _ => return Err(already_registered()),
        }
        keys.uploaded = true;
    }
    // A 400 means this SRP setup was done already (a retry after a lost
    // answer); the attributes check below decides.
    match setup_srp(api, keys).await {
        Ok(()) => {}
        Err(e) if e.status() == Some(400) => {}
        Err(e) => return Err(e),
    }
    let remote: SrpAttributesResponse = api
        .get_json("/users/srp/attributes", &[("email", email.to_string())])
        .await?;
    check_srp_attributes(&remote.attributes, keys)
}

/// `POST /users/srp/setup` + `POST /users/srp/complete`, checking `M2`.
async fn setup_srp(api: &Api, keys: &PreparedKeys) -> Result<()> {
    let (id, salt, login_key) = (&keys.srp_user_id, &keys.srp_salt, &keys.login_key);
    let mut client = crate::offload(|| SrpClient::new(id.as_bytes(), salt, login_key));
    let setup: SetupSrpResponse = api
        .json(
            Method::POST,
            "/users/srp/setup",
            &SetupSrpRequest {
                srp_user_id: keys.srp_user_id.clone(),
                srp_salt: crypto::b64(&keys.srp_salt),
                srp_verifier: crypto::b64(&keys.srp_verifier),
                srp_a: crypto::b64(&client.public_a()),
            },
        )
        .await?;
    let srp_b = crypto::unb64(&setup.srp_b)?;
    let m1 = crate::offload(|| client.compute_m1(&srp_b))?;
    let done: CompleteSrpSetupResponse = api
        .json(
            Method::POST,
            "/users/srp/complete",
            &CompleteSrpSetupRequest {
                setup_id: &setup.setup_id,
                srp_m1: crypto::b64(&m1),
            },
        )
        .await?;
    let m2 = done
        .srp_m2
        .filter(|m| !m.is_empty())
        .ok_or_else(|| CoreError::crypto("server sent no SRP proof"))?;
    client
        .verify_m2(&crypto::unb64(&m2)?)
        .map_err(|_| CoreError::crypto("the server failed to prove it stored the verifier"))
}

/// Ente's `SrpAttributes::validate_setup`.
fn check_srp_attributes(a: &SrpAttributes, keys: &PreparedKeys) -> Result<()> {
    let mut bad = Vec::new();
    if a.srp_user_id != keys.srp_user_id {
        bad.push("srpUserID");
    }
    if a.srp_salt != crypto::b64(&keys.srp_salt) {
        bad.push("srpSalt");
    }
    if a.kek_salt != keys.attributes.kek_salt {
        bad.push("kekSalt");
    }
    if a.mem_limit != keys.attributes.mem_limit {
        bad.push("memLimit");
    }
    if a.ops_limit != keys.attributes.ops_limit {
        bad.push("opsLimit");
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(CoreError::protocol(format!(
            "the server's SRP attributes do not match the sign-up: {}",
            bad.join(", ")
        )))
    }
}

/// Create account (contract v1.2, "signup").
#[uniffi::export]
impl Core {
    /// Sign-up step 1: email a code to `email` (`POST /users/ott`, purpose
    /// `signup`) on the active server. Fails with `Server { status: 403 }`
    /// when the server does not accept new accounts
    /// (`ServerInfo.signups_allowed`) and `Server { status: 409 }` when the
    /// email already has an account.
    pub async fn signup_send_code(&self, email: String) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.signup_send_code(&email).await }).await
    }

    /// Sign-up step 2: check the emailed code. `IncorrectCode` if wrong or
    /// expired; `Server { status: 409 }` if the email has an account already.
    pub async fn signup_verify_code(&self, email: String, code: String) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.signup_verify_code(&email, &code).await }).await
    }

    /// Sign-up step 3: make the account's keys on this device from
    /// `password` (Argon2id "sensitive": 1 GiB / 4 ops, or 256 MiB / 16 ops
    /// when `low_memory`, halving memory on failure) and return the recovery
    /// key as 24 words for the user to save. Nothing is uploaded yet. Calling
    /// it again with the same password returns the same words.
    pub async fn signup_prepare(
        &self,
        password: String,
        low_memory: bool,
    ) -> Result<String, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.signup_prepare(&password, low_memory).await }).await
    }

    /// Sign-up step 4 (after the user saved the recovery key): upload the
    /// key attributes, set up SRP, and sign in. Safe to retry.
    pub async fn signup_finish(&self) -> Result<Account, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.signup_finish().await }).await
    }

    /// The signed-in account's recovery key as 24 words (local, no network).
    pub fn recovery_key_words(&self) -> Result<String, CoreError> {
        self.inner.recovery_key_words()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_limits_are_entes() {
        assert_eq!(sensitive_start(false), (4, 1 << 30));
        assert_eq!(sensitive_start(true), (16, 256 << 20));
        for low in [false, true] {
            let (ops, mem) = sensitive_start(low);
            assert_eq!(ops * mem, crypto::SENSITIVE_KDF_STRENGTH);
        }
    }

    #[test]
    fn falls_back_to_less_memory() {
        let mut tried = Vec::new();
        let (k, ops, mem) = derive_sensitive_with(false, |ops, mem| {
            tried.push((ops, mem));
            if mem > MEMLIMIT_MODERATE {
                Err("out of memory")
            } else {
                Ok(vec![1])
            }
        })
        .unwrap();
        assert_eq!(k, vec![1]);
        assert_eq!((ops, mem), (16, 256 << 20));
        assert_eq!(tried, vec![(4, 1 << 30), (8, 512 << 20), (16, 256 << 20)]);
        // Never below the server's minimum.
        let mut tried = Vec::new();
        let r: std::result::Result<_, &str> = derive_sensitive_with(true, |ops, mem| {
            tried.push((ops, mem));
            Err("out of memory")
        });
        assert!(r.is_err());
        assert_eq!(tried, vec![(16, 256 << 20), (32, 128 << 20)]);
    }

    #[test]
    fn generated_keys_open_like_entes() {
        let keys = generate_keys(b"pw", true).unwrap();
        let a = &keys.attributes;
        assert_eq!(a.ops_limit * a.mem_limit, crypto::SENSITIVE_KDF_STRENGTH);
        assert!(password_opens(b"pw", &keys).unwrap());
        assert!(!password_opens(b"other", &keys).unwrap());
        let rk = crypto::parse_recovery_key(
            &crypto::recovery_key_to_mnemonic(&keys.recovery_key).unwrap(),
        )
        .unwrap();
        let mk = crypto::secretbox_open(
            &crypto::unb64(a.master_key_encrypted_with_recovery_key.as_ref().unwrap()).unwrap(),
            &crypto::unb64(a.master_key_decryption_nonce.as_ref().unwrap()).unwrap(),
            &rk,
        )
        .unwrap();
        assert_eq!(mk, keys.master_key);
        let sk = crypto::secretbox_open(
            &crypto::unb64(&a.encrypted_secret_key).unwrap(),
            &crypto::unb64(&a.secret_key_decryption_nonce).unwrap(),
            &mk,
        )
        .unwrap();
        assert_eq!(
            crypto::public_key_from_secret(&sk).unwrap(),
            keys.public_key
        );
        assert_eq!(keys.srp_salt.len(), 16);
        assert_eq!(keys.login_key.len(), 16);
    }
}
