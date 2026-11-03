//! Login, exactly as the Ente apps and CLI do it (`cli/pkg/sign_in.go`,
//! `cli/pkg/account.go`):
//!
//! 1. `GET /users/srp/attributes` gives the SRP identity/salt and the Argon2id
//!    parameters of the key-encryption key (KEK).
//! 2. With SRP: derive the KEK, take its login sub-key as the SRP password,
//!    run `create-session`/`verify-session`. Without SRP (404) or with email
//!    MFA on: email one-time code (`/users/ott`, `/users/verify-email`).
//! 3. Second factor: TOTP via `/users/two-factor/verify`. Passkeys need a
//!    browser and are not supported here.
//! 4. Open the master key with the KEK, the secret key with the master key,
//!    and the sealed auth token with the key pair.

use super::api::{Api, status_of};
use super::model::*;
use crate::crypto;
use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use reqwest::Method;

/// A prompt callback (TOTP or email code).
pub type Prompt<'a> = &'a (dyn Fn() -> anyhow::Result<String> + Sync);

/// The decrypted secrets of a logged-in account.
#[derive(Clone)]
pub struct Session {
    /// User id.
    pub user_id: i64,
    /// Value for `X-Auth-Token`.
    pub token: String,
    /// The 32-byte master key.
    pub master_key: Vec<u8>,
    /// X25519 public key.
    pub public_key: Vec<u8>,
    /// X25519 secret key.
    pub secret_key: Vec<u8>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

/// How many wrong TOTP / email codes we accept before giving up.
const CODE_ATTEMPTS: usize = 3;

/// Log in and decrypt the account keys.
pub async fn login(
    api: &Api,
    email: &str,
    password: &str,
    totp: Prompt<'_>,
    otp: Prompt<'_>,
) -> anyhow::Result<Session> {
    crypto::init();
    let attrs = match api
        .get_json::<SrpAttributesResponse>("/users/srp/attributes", &[("email", email.to_string())])
        .await
    {
        Ok(r) => Some(r.attributes),
        Err(e) if status_of(&e) == Some(404) => None,
        Err(e) => return Err(e.context("fetching SRP attributes")),
    };

    let (mut auth, mut kek) = match &attrs {
        Some(a) if !a.is_email_mfa_enabled => {
            let kek = crypto::derive_argon_key(
                password.as_bytes(),
                &crypto::unb64(&a.kek_salt)?,
                a.ops_limit,
                a.mem_limit,
            )?;
            (srp_login(api, a, &kek).await?, Some(kek))
        }
        _ => (email_login(api, email, otp).await?, None),
    };

    if non_empty(&auth.passkey_session_id) {
        bail!(
            "this account uses a passkey as its second factor, which needs a browser; \
             the importer only supports TOTP. Temporarily switch the account to TOTP \
             (or disable 2FA) in the app, run the import, then switch back"
        );
    }
    if let Some(session_id) = auth.two_factor_session_id.clone().filter(|s| !s.is_empty()) {
        auth = verify_totp(api, &session_id, totp).await?;
    }

    let ka = auth.key_attributes.clone().ok_or_else(|| {
        anyhow!("the server sent no key attributes: set the account up in an Ente app first")
    })?;
    if kek.is_none() {
        kek = Some(crypto::derive_argon_key(
            password.as_bytes(),
            &crypto::unb64(&ka.kek_salt)?,
            ka.ops_limit,
            ka.mem_limit,
        )?);
    }
    let kek = kek.unwrap_or_default();
    let master_key = crypto::secretbox_open(
        &crypto::unb64(&ka.encrypted_key)?,
        &crypto::unb64(&ka.key_decryption_nonce)?,
        &kek,
    )
    .map_err(|_| anyhow!("incorrect password (could not decrypt the master key)"))?;
    let secret_key = crypto::secretbox_open(
        &crypto::unb64(&ka.encrypted_secret_key)?,
        &crypto::unb64(&ka.secret_key_decryption_nonce)?,
        &master_key,
    )
    .context("decrypting the secret key")?;
    let public_key = crypto::unb64(&ka.public_key)?;

    let token = match (&auth.encrypted_token, &auth.token) {
        (Some(et), _) if !et.is_empty() => {
            let raw = crypto::box_seal_open(&crypto::unb64(et)?, &public_key, &secret_key)
                .context("decrypting the auth token")?;
            // Ente encodes the token URL-safe with padding (Go's URLEncoding).
            base64::engine::general_purpose::URL_SAFE.encode(raw)
        }
        (_, Some(t)) if !t.is_empty() => t.clone(),
        _ => bail!("the server sent no auth token"),
    };

    tracing::info!(user_id = auth.id, "logged in");
    Ok(Session {
        user_id: auth.id,
        token,
        master_key,
        public_key,
        secret_key,
    })
}

async fn srp_login(
    api: &Api,
    a: &SrpAttributes,
    kek: &[u8],
) -> anyhow::Result<AuthorizationResponse> {
    let login_key = crypto::derive_login_key(kek)?;
    let salt = crypto::unb64(&a.srp_salt)?;
    let mut client = crate::srp::SrpClient::new(a.srp_user_id.as_bytes(), &salt, &login_key);
    let session: CreateSrpSessionResponse = api
        .json(
            Method::POST,
            "/users/srp/create-session",
            &CreateSrpSessionRequest {
                srp_user_id: &a.srp_user_id,
                srp_a: crypto::b64(&client.public_a()),
            },
        )
        .await
        .context("creating SRP session")?;
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
        Err(e) if status_of(&e) == Some(401) => bail!("incorrect email or password"),
        Err(e) => return Err(e.context("verifying SRP session")),
    };
    if let Some(m2) = auth.srp_m2.as_deref().filter(|s| !s.is_empty()) {
        client
            .verify_m2(&crypto::unb64(m2)?)
            .map_err(|_| anyhow!("the server failed to prove it knows the SRP verifier"))?;
    }
    Ok(auth)
}

async fn email_login(
    api: &Api,
    email: &str,
    otp: Prompt<'_>,
) -> anyhow::Result<AuthorizationResponse> {
    api.json_no_content(
        Method::POST,
        "/users/ott",
        &SendOttRequest {
            email,
            purpose: "login",
        },
    )
    .await
    .context("requesting an email login code")?;
    tracing::info!("a login code was sent to {email}");
    let mut last = None;
    for _ in 0..CODE_ATTEMPTS {
        let code = otp()?;
        match api
            .json(
                Method::POST,
                "/users/verify-email",
                &VerifyEmailRequest {
                    email,
                    ott: code.trim(),
                },
            )
            .await
        {
            Ok(r) => return Ok(r),
            Err(e) if matches!(status_of(&e), Some(400 | 401 | 403)) => {
                tracing::warn!("wrong or expired email code");
                last = Some(e);
            }
            Err(e) => return Err(e.context("verifying email code")),
        }
    }
    Err(last
        .unwrap_or_else(|| anyhow!("no code"))
        .context("email code rejected"))
}

async fn verify_totp(
    api: &Api,
    session_id: &str,
    totp: Prompt<'_>,
) -> anyhow::Result<AuthorizationResponse> {
    let mut last = None;
    for _ in 0..CODE_ATTEMPTS {
        let code = totp()?;
        match api
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
            Ok(r) => return Ok(r),
            Err(e) if matches!(status_of(&e), Some(400 | 401 | 403)) => {
                tracing::warn!("wrong TOTP code");
                last = Some(e);
            }
            Err(e) => return Err(e.context("verifying TOTP code")),
        }
    }
    Err(last
        .unwrap_or_else(|| anyhow!("no code"))
        .context("TOTP code rejected"))
}

fn non_empty(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.is_empty())
}
