//! Signing in as a new device, exactly as the Ente apps do, and the resulting [`Session`].
//!
//! Flow (mirrors Ente's `LoginFlow`):
//!
//! 1. `GET /users/srp/attributes?email=…`. If SRP is not set up, or the
//!    account has email MFA on, the server first mails a code
//!    (`POST /users/ott`) which is checked with `POST /users/verify-email`.
//! 2. Otherwise the password is derived with Argon2id into the key-encryption
//!    key (KEK), its `crypto_kdf` login sub-key is the SRP password, and
//!    `POST /users/srp/create-session` + `verify-session` prove it. The
//!    server's `M2` is verified so a fake server cannot pretend to accept us.
//! 3. If two-factor is on, a TOTP code goes to `POST /users/two-factor/verify`.
//!    Passkey-only accounts get [`ApiError::PasskeyRequired`].
//! 4. The KEK opens the master key, the master key opens the X25519 secret
//!    key, and that opens the sealed auth token.

use super::models::{
    AuthResponse, CreateSrpSessionRequest, CreateSrpSessionResponse, KeyAttributes, SrpAttributes,
    VerifySrpSessionRequest,
};
use super::srp::SrpClient;
use super::{ApiClient, ApiError, Result};
use crate::crypto::{self, KdfParams, SecretKey};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// A signed-in device: everything the node needs to act for the user.
///
/// Contains the master key: persist it only with owner-only permissions (0600).
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// Server base URL.
    pub endpoint: String,
    /// The account's email.
    pub email: String,
    /// User id.
    pub user_id: i64,
    /// Auth token, URL-safe base64, as sent in `X-Auth-Token`.
    pub token: String,
    /// The account master key.
    pub master_key: SecretKey,
    /// The account's X25519 public key (standard base64).
    pub public_key: String,
    /// The account's X25519 secret key.
    pub secret_key: SecretKey,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("endpoint", &self.endpoint)
            .field("email", &self.email)
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// The public key as raw bytes.
    pub fn public_key_bytes(&self) -> Result<Vec<u8>> {
        Ok(crypto::b64_decode(&self.public_key)?)
    }
}

/// What the sign-in flow needs next.
#[derive(Debug)]
pub enum LoginStep {
    /// A code was emailed; call [`LoginFlow::submit_code`].
    EmailCode,
    /// Call [`LoginFlow::submit_password`].
    Password,
    /// A TOTP code from the authenticator app; call [`LoginFlow::submit_code`].
    Totp,
    /// Signed in.
    Complete(Box<Session>),
}

enum State {
    EmailCode,
    /// Password needed; `Some` when the email step already returned the keys.
    Password(Option<Box<AuthResponse>>),
    Totp {
        session_id: String,
        kek: Option<SecretKey>,
    },
    Complete,
}

/// The interactive sign-in state machine.
pub struct LoginFlow {
    email: String,
    attributes: Option<SrpAttributes>,
    state: State,
}

impl std::fmt::Debug for LoginFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginFlow")
            .field("email", &self.email)
            .finish_non_exhaustive()
    }
}

impl LoginFlow {
    /// Begin signing in `email`. Sends the email code if one is required.
    pub async fn start(client: &ApiClient, email: &str) -> Result<(Self, LoginStep)> {
        let email = email.trim().to_owned();
        let attributes = client.get_srp_attributes(&email).await?;
        let flow = Self {
            email,
            state: if attributes.as_ref().is_none_or(|a| a.is_email_mfa_enabled) {
                State::EmailCode
            } else {
                State::Password(None)
            },
            attributes,
        };
        let step = match flow.state {
            State::EmailCode => {
                client.send_login_otp(&flow.email).await?;
                LoginStep::EmailCode
            }
            _ => LoginStep::Password,
        };
        Ok((flow, step))
    }

    /// The email being signed in.
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Submit the account password.
    pub async fn submit_password(
        &mut self,
        client: &ApiClient,
        password: &str,
    ) -> Result<LoginStep> {
        let State::Password(response) = &self.state else {
            return Err(ApiError::InvalidInput(
                "a password is not expected now".into(),
            ));
        };
        if let Some(response) = response.as_deref().cloned() {
            // Keep the state on failure so a mistyped password can be retried.
            let attrs = response
                .key_attributes
                .as_ref()
                .ok_or_else(|| ApiError::Protocol("account has no key attributes".into()))?;
            let kek = derive_kek(password, &attrs.kek_salt, attrs.mem_limit, attrs.ops_limit)?;
            return self.advance(client, response, Some(kek));
        }
        let attrs = self
            .attributes
            .clone()
            .ok_or_else(|| ApiError::Protocol("missing SRP attributes".into()))?;
        let kek = derive_kek(password, &attrs.kek_salt, attrs.mem_limit, attrs.ops_limit)?;
        let response = client.login_srp(&attrs, &kek).await?;
        self.advance(client, response, Some(kek))
    }

    /// Submit an email code or TOTP code, whichever was asked for.
    pub async fn submit_code(&mut self, client: &ApiClient, code: &str) -> Result<LoginStep> {
        let code = code.trim();
        match std::mem::replace(&mut self.state, State::Complete) {
            State::EmailCode => {
                let r = client.verify_email_otp(&self.email, code).await;
                let response = r.inspect_err(|_| self.state = State::EmailCode)?;
                self.advance(client, response, None)
            }
            State::Totp { session_id, kek } => match client.verify_totp(&session_id, code).await {
                Ok(response) => self.advance(client, response, kek),
                Err(e) => {
                    self.state = State::Totp { session_id, kek };
                    Err(e)
                }
            },
            other => {
                self.state = other;
                Err(ApiError::InvalidInput("a code is not expected now".into()))
            }
        }
    }

    /// Ask the server to email a new code (only while an email code is expected).
    pub async fn resend_code(&self, client: &ApiClient) -> Result<()> {
        if !matches!(self.state, State::EmailCode) {
            return Err(ApiError::InvalidInput(
                "an email code is not expected now".into(),
            ));
        }
        client.send_login_otp(&self.email).await
    }

    fn advance(
        &mut self,
        client: &ApiClient,
        response: AuthResponse,
        kek: Option<SecretKey>,
    ) -> Result<LoginStep> {
        if let Some(id) = response.totp_session_id() {
            self.state = State::Totp {
                session_id: id.to_owned(),
                kek,
            };
            return Ok(LoginStep::Totp);
        }
        if response.passkey_required() {
            self.state = State::Complete;
            return Err(ApiError::PasskeyRequired);
        }
        match kek {
            Some(kek) => {
                let session = unlock(&response, &kek, client.base_url(), &self.email)?;
                self.state = State::Complete;
                Ok(LoginStep::Complete(Box::new(session)))
            }
            None => {
                self.state = State::Password(Some(Box::new(response)));
                Ok(LoginStep::Password)
            }
        }
    }
}

/// Ente `deriveKey` for the account: Argon2id(password, kekSalt, memLimit, opsLimit).
pub fn derive_kek(
    password: &str,
    kek_salt_b64: &str,
    mem_limit: u64,
    ops_limit: u32,
) -> Result<SecretKey> {
    let salt = crypto::b64_decode(kek_salt_b64)?;
    Ok(crypto::derive_key(
        password,
        &salt,
        KdfParams {
            mem_limit,
            ops_limit,
        },
    )?)
}

/// Open the master key and keypair with the KEK.
pub fn decrypt_keys(kek: &SecretKey, attrs: &KeyAttributes) -> Result<(SecretKey, SecretKey)> {
    let master_key =
        crypto::secretbox_open_key_b64(&attrs.encrypted_key, &attrs.key_decryption_nonce, kek)
            .map_err(|_| ApiError::IncorrectPassword)?;
    let secret_key = crypto::secretbox_open_key_b64(
        &attrs.encrypted_secret_key,
        &attrs.secret_key_decryption_nonce,
        &master_key,
    )?;
    let public_key = crypto::b64_decode(&attrs.public_key)?;
    if crypto::public_key_from_secret(&secret_key)?.as_slice() != public_key.as_slice() {
        return Err(ApiError::Protocol(
            "secret key does not match public key".into(),
        ));
    }
    Ok((master_key, secret_key))
}

/// Finish sign-in: open the keys and the sealed token from an [`AuthResponse`].
pub fn unlock(
    response: &AuthResponse,
    kek: &SecretKey,
    endpoint: &str,
    email: &str,
) -> Result<Session> {
    if response.totp_session_id().is_some() || response.passkey_required() {
        return Err(ApiError::InvalidInput(
            "second factor still required".into(),
        ));
    }
    let attrs = response.key_attributes.as_ref().ok_or_else(|| {
        ApiError::Protocol(
            "account has no key attributes (finish sign-up in an Ente app first)".into(),
        )
    })?;
    let (master_key, secret_key) = decrypt_keys(kek, attrs)?;
    let token = if let Some(enc) = &response.encrypted_token {
        let public_key = crypto::b64_decode(&attrs.public_key)?;
        let raw = Zeroizing::new(crypto::sealed_box_open(
            &crypto::b64_decode(enc)?,
            &public_key,
            &secret_key,
        )?);
        crypto::b64url_encode(&raw)
    } else if let Some(t) = &response.token {
        // Plain tokens are already URL-safe base64; normalise the encoding.
        let raw = Zeroizing::new(crypto::b64url_decode(t).or_else(|_| crypto::b64_decode(t))?);
        crypto::b64url_encode(&raw)
    } else {
        return Err(ApiError::Protocol("no token in response".into()));
    };
    Ok(Session {
        endpoint: endpoint.to_owned(),
        email: email.to_owned(),
        user_id: response.id,
        token,
        master_key,
        public_key: attrs.public_key.clone(),
        secret_key,
    })
}

#[derive(Serialize)]
struct EmailOtt<'a> {
    email: &'a str,
    purpose: &'a str,
}

#[derive(Serialize)]
struct VerifyEmail<'a> {
    email: &'a str,
    ott: &'a str,
}

#[derive(Serialize)]
struct VerifyTotp<'a> {
    #[serde(rename = "sessionID")]
    session_id: &'a str,
    code: &'a str,
}

#[derive(Deserialize)]
struct SrpAttributesResponse {
    attributes: SrpAttributes,
}

impl ApiClient {
    /// `GET /users/srp/attributes`. `None` when the account has no SRP set up
    /// (sign in by email code then).
    pub async fn get_srp_attributes(&self, email: &str) -> Result<Option<SrpAttributes>> {
        match self
            .get_json::<SrpAttributesResponse, _>("/users/srp/attributes", &[("email", email)])
            .await
        {
            Ok(r) => Ok(Some(r.attributes)),
            Err(ApiError::Http { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `POST /users/ott` with purpose `login`: email a sign-in code.
    pub async fn send_login_otp(&self, email: &str) -> Result<()> {
        self.send_json_no_content(
            Method::POST,
            "/users/ott",
            &EmailOtt {
                email,
                purpose: "login",
            },
            true,
        )
        .await
    }

    /// `POST /users/verify-email`: check the emailed code (not retried).
    pub async fn verify_email_otp(&self, email: &str, ott: &str) -> Result<AuthResponse> {
        self.send_json(
            Method::POST,
            "/users/verify-email",
            &VerifyEmail { email, ott },
            false,
        )
        .await
    }

    /// SRP sign-in with the KEK: create-session, verify-session, check `M2`.
    pub async fn login_srp(&self, attrs: &SrpAttributes, kek: &SecretKey) -> Result<AuthResponse> {
        let login_key = crypto::derive_login_key(kek)?;
        let salt = crypto::b64_decode(&attrs.srp_salt)?;
        let mut srp = SrpClient::new(&attrs.srp_user_id, &salt, &login_key);
        let created: CreateSrpSessionResponse = self
            .send_json(
                Method::POST,
                "/users/srp/create-session",
                &CreateSrpSessionRequest {
                    srp_user_id: attrs.srp_user_id.clone(),
                    srp_a: crypto::b64_encode(srp.public_a()),
                },
                true,
            )
            .await?;
        let b = crypto::b64_decode(&created.srp_b)?;
        let m1 = srp
            .compute_m1(&b)
            .map_err(|e| ApiError::Protocol(e.to_string()))?;
        let verified = self
            .send_json::<AuthResponse, _>(
                Method::POST,
                "/users/srp/verify-session",
                &VerifySrpSessionRequest {
                    srp_user_id: attrs.srp_user_id.clone(),
                    session_id: created.session_id,
                    srp_m1: crypto::b64_encode(&m1),
                },
                false,
            )
            .await;
        let response = match verified {
            Ok(r) => r,
            // museum answers 401 for a wrong proof, i.e. a wrong password.
            Err(ApiError::Unauthorized) => return Err(ApiError::IncorrectPassword),
            Err(e) => return Err(e),
        };
        let m2 = response
            .srp_m2
            .as_deref()
            .ok_or_else(|| ApiError::Protocol("server did not send srpM2".into()))?;
        srp.verify_m2(&crypto::b64_decode(m2)?)
            .map_err(|e| ApiError::Protocol(e.to_string()))?;
        Ok(response)
    }

    /// `POST /users/two-factor/verify` with a TOTP code (not retried).
    pub async fn verify_totp(&self, session_id: &str, code: &str) -> Result<AuthResponse> {
        self.send_json(
            Method::POST,
            "/users/two-factor/verify",
            &VerifyTotp { session_id, code },
            false,
        )
        .await
    }

    /// `POST /users/logout`: revoke this device's token.
    pub async fn logout(&self) -> Result<()> {
        if !self.has_token() {
            return Err(ApiError::Unauthorized);
        }
        self.send_json_no_content(Method::POST, "/users/logout", &serde_json::json!({}), true)
            .await
    }
}
