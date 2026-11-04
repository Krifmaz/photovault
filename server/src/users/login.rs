//! Email OTTs and the shared "login succeeded" path (museum `userauth.go`,
//! `onVerificationSuccess`, `GetKeyAttributeAndToken`).

use super::extract::ClientInfo;
use super::*;
use crate::auth::App;
use crate::db::now_micros;
use axum::http::StatusCode;
use serde::Serialize;

/// Museum `OTTValidityDurationInMicroSeconds` (1 hour).
pub const OTT_VALIDITY: i64 = 60 * 60 * 1_000_000;
/// Museum `OTTWrongAttemptLimit`.
pub const OTT_WRONG_ATTEMPT_LIMIT: i64 = 20;
/// Museum `OTTActiveCodeLimit`.
pub const OTT_ACTIVE_CODE_LIMIT: usize = 10;
/// Museum `TwoFactorValidityDurationInMicroSeconds` (10 minutes).
pub const TWO_FACTOR_VALIDITY: i64 = 10 * 60 * 1_000_000;

/// OTT purposes.
pub mod purpose {
    /// Change email.
    pub const CHANGE: &str = "change";
    /// Sign up.
    pub const SIGNUP: &str = "signup";
    /// Log in.
    pub const LOGIN: &str = "login";
}

#[derive(PartialEq, Eq)]
enum SignUpState {
    NoAccount,
    Incomplete,
    Complete,
}

async fn sign_up_state(state: &AppState, email: &str) -> ApiResult<SignUpState> {
    let Some(id) = user_id_for_email(state, email).await? else {
        return Ok(SignUpState::NoAccount);
    };
    Ok(if key_attributes(&state.db, id).await?.is_some() {
        SignUpState::Complete
    } else {
        SignUpState::Incomplete
    })
}

/// Hard-coded OTT for `email` from `config.hardcoded_ott`.
pub fn hardcoded_ott(state: &AppState, email: &str) -> Option<String> {
    state
        .config
        .hardcoded_ott
        .iter()
        .find(|(k, _)| normalize_email(k) == email)
        .map(|(_, v)| v.clone())
}

/// Insert an OTT row (museum `AddOTT`; a duplicate code is ignored).
pub async fn add_ott(
    state: &AppState,
    email: &str,
    app: App,
    ott: &str,
    expiry: i64,
) -> ApiResult<()> {
    let hash = email_hash(&state.keys, &normalize_email(email));
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM otts WHERE email_hash = $1 AND app = $2 AND ott = $3")
            .bind(&hash)
            .bind(app.as_str())
            .bind(ott)
            .fetch_optional(&state.db.pool)
            .await?;
    if exists.is_some() {
        sqlx::query(
            "UPDATE otts SET expiration_time = $1 WHERE email_hash = $2 AND app = $3 AND ott = $4",
        )
        .bind(expiry)
        .bind(&hash)
        .bind(app.as_str())
        .bind(ott)
        .execute(&state.db.pool)
        .await?;
        return Ok(());
    }
    sqlx::query("INSERT INTO otts (email_hash, ott, app, creation_time, expiration_time) VALUES ($1, $2, $3, $4, $5)")
        .bind(&hash)
        .bind(ott)
        .bind(app.as_str())
        .bind(now_micros())
        .bind(expiry)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Unexpired OTTs for an email (newest first), as `(ott, app, expiration_time)`.
pub async fn pending_otts(state: &AppState, email: &str) -> ApiResult<Vec<(String, String, i64)>> {
    let hash = email_hash(&state.keys, &normalize_email(email));
    Ok(sqlx::query_as(
        "SELECT ott, app, expiration_time FROM otts WHERE email_hash = $1 AND expiration_time > $2 ORDER BY creation_time DESC",
    )
    .bind(hash)
    .bind(now_micros())
    .fetch_all(&state.db.pool)
    .await?)
}

/// Museum `SendEmailOTT`.
pub async fn send_email_ott(
    state: &AppState,
    client: &ClientInfo,
    email: &str,
    purpose: &str,
) -> ApiResult<()> {
    let email = normalize_email(email);
    if purpose == purpose::CHANGE {
        if user_id_for_email(state, &email).await?.is_some() {
            return Err(permission_denied());
        }
    } else {
        let st = sign_up_state(state, &email).await?;
        // An account an admin created (`photovault admin create-user`) is `Incomplete`:
        // it may finish signing up while signups are closed.
        if purpose == purpose::SIGNUP
            && st == SignUpState::NoAccount
            && !crate::settings::signups_allowed(state).await
        {
            return Err(permission_denied());
        }
        if purpose == purpose::SIGNUP && st == SignUpState::Complete {
            return Err(user_already_registered());
        }
        if purpose == purpose::LOGIN && st == SignUpState::NoAccount {
            return Err(user_not_registered());
        }
        if purpose == purpose::LOGIN && st == SignUpState::Incomplete {
            return Err(user_signup_incomplete());
        }
    }
    let app = client.app();
    let hash = email_hash(&state.keys, &email);
    let (active,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM otts WHERE email_hash = $1 AND app = $2 AND expiration_time > $3",
    )
    .bind(&hash)
    .bind(app.as_str())
    .bind(now_micros())
    .fetch_one(&state.db.pool)
    .await?;
    if active as usize >= OTT_ACTIVE_CODE_LIMIT {
        return Err(too_many_plain());
    }
    let hard = if purpose != purpose::CHANGE {
        hardcoded_ott(state, &email)
    } else {
        None
    };
    let ott = hard.clone().unwrap_or_else(six_digit_code);
    add_ott(state, &email, app, &ott, now_micros() + OTT_VALIDITY).await?;
    if hard.is_none() {
        if purpose != purpose::CHANGE && skip_applies(state, &email).await? {
            tracing::info!(
                "Email verification skipped for {email} (PHOTOVAULT_SKIP_EMAIL_VERIFICATION): any code is accepted"
            );
            return Ok(());
        }
        let body = if purpose == purpose::CHANGE {
            format!(
                "Use this code to confirm your new email address for Photovault: {ott}\n\nThe code expires in one hour.\n"
            )
        } else {
            format!(
                "Your Photovault verification code is {ott}\n\nThe code expires in one hour. If you did not request it, ignore this email.\n"
            )
        };
        crate::mail::deliver_code(
            &state.config,
            &email,
            &ott,
            &format!("Photovault verification code: {ott}"),
            &body,
        )
        .await;
    }
    Ok(())
}

/// `PHOTOVAULT_SKIP_EMAIL_VERIFICATION` applies to `email`: on, and the account (if any)
/// has not turned on email two-factor (that code is always required).
pub async fn skip_applies(state: &AppState, email: &str) -> ApiResult<bool> {
    if !state.config.skip_email_verification {
        return Ok(false);
    }
    let Some(id) = user_id_for_email(state, email).await? else {
        return Ok(true);
    };
    let mfa: Option<(i64,)> = sqlx::query_as(
        "SELECT CASE WHEN email_mfa THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db.pool)
    .await?;
    Ok(!matches!(mfa, Some((n,)) if n != 0))
}

/// [`verify_email_ott`] for sign-up and log-in. With `PHOTOVAULT_SKIP_EMAIL_VERIFICATION`
/// any code is accepted, but only while a code requested through `/users/ott` is pending,
/// so the sign-up policy checked there still holds.
pub async fn verify_email_ott_or_skip(
    state: &AppState,
    app: App,
    email: &str,
    ott: &str,
) -> ApiResult<()> {
    if !skip_applies(state, email).await? {
        return verify_email_ott(state, app, email, ott).await;
    }
    let hash = email_hash(&state.keys, email);
    let r =
        sqlx::query("DELETE FROM otts WHERE email_hash = $1 AND app = $2 AND expiration_time > $3")
            .bind(&hash)
            .bind(app.as_str())
            .bind(now_micros())
            .execute(&state.db.pool)
            .await?;
    if r.rows_affected() == 0 {
        return Err(gone_plain());
    }
    Ok(())
}

/// Museum `verifyEmailOtt`.
pub async fn verify_email_ott(state: &AppState, app: App, email: &str, ott: &str) -> ApiResult<()> {
    let ott = ott.trim();
    let hash = email_hash(&state.keys, email);
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ott, wrong_attempt FROM otts WHERE email_hash = $1 AND app = $2 AND expiration_time > $3",
    )
    .bind(&hash)
    .bind(app.as_str())
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;
    if rows.iter().any(|r| r.1 >= OTT_WRONG_ATTEMPT_LIMIT) {
        return Err(too_many_plain());
    }
    if rows.is_empty() {
        return Err(gone_plain());
    }
    if !rows.iter().any(|r| r.0 == ott) {
        sqlx::query("UPDATE otts SET wrong_attempt = wrong_attempt + 1 WHERE email_hash = $1 AND app = $2 AND expiration_time > $3")
            .bind(&hash)
            .bind(app.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(unauthorized_plain());
    }
    let r = sqlx::query("DELETE FROM otts WHERE email_hash = $1 AND app = $2 AND ott = $3")
        .bind(&hash)
        .bind(app.as_str())
        .bind(ott)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if r.rows_affected() == 0 {
        return Err(gone_plain());
    }
    Ok(())
}

/// Museum `ente.EmailAuthorizationResponse`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmailAuthorizationResponse {
    /// User id.
    pub id: i64,
    /// Key attributes (absent before signup completes or when a second factor is pending).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_attributes: Option<KeyAttributes>,
    /// Token sealed to the user's public key.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_token: String,
    /// Plain token (only before key attributes exist).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// Passkey 2FA session.
    #[serde(rename = "passkeySessionID")]
    pub passkey_session_id: String,
    /// Accounts app URL for the passkey ceremony.
    pub accounts_url: String,
    /// TOTP session (when TOTP is the only second factor).
    #[serde(rename = "twoFactorSessionID")]
    pub two_factor_session_id: String,
    /// TOTP session when passkeys are also enabled.
    #[serde(rename = "twoFactorSessionIDV2")]
    pub two_factor_session_id_v2: String,
    /// SRP M2 (SRP logins).
    #[serde(rename = "srpM2", skip_serializing_if = "Option::is_none")]
    pub srp_m2: Option<String>,
}

/// Museum `ente.TwoFactorAuthorizationResponse`.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorAuthorizationResponse {
    /// User id.
    pub id: i64,
    /// Key attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_attributes: Option<KeyAttributes>,
    /// Sealed token.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encrypted_token: String,
}

/// Museum `PasskeyRepo.AccountsURLForUser`.
pub fn accounts_url(state: &AppState) -> String {
    state.config.accounts.accounts_url.clone()
}

async fn has_passkeys(state: &AppState, user_id: i64) -> ApiResult<bool> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM passkeys WHERE user_id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_one(&state.db.pool)
            .await?;
    Ok(n > 0)
}

async fn ensure_active(state: &AppState, user_id: i64) -> ApiResult<()> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT CASE WHEN is_deleted THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    match row {
        Some((0,)) => Ok(()),
        Some(_) => Err(gone_plain()),
        None => Err(not_found_plain()),
    }
}

/// Museum `onVerificationSuccess` (email OTT or SRP proof accepted). `srp_login` rejects
/// accounts that turned on email MFA meanwhile.
pub async fn on_verification_success(
    state: &AppState,
    client: &ClientInfo,
    email: &str,
    source: Option<&str>,
    srp_login: bool,
) -> ApiResult<EmailAuthorizationResponse> {
    let app = client.app();
    let (user_id, two_factor) = match user_id_for_email(state, email).await? {
        Some(id) => {
            let (tfa,): (i64,) =
                sqlx::query_as("SELECT CASE WHEN is_two_factor_enabled THEN 1 ELSE 0 END FROM users WHERE user_id = $1")
                    .bind(id)
                    .fetch_one(&state.db.pool)
                    .await?;
            (id, tfa != 0)
        }
        None => {
            if !crate::settings::signups_allowed(state).await {
                return Err(permission_denied());
            }
            (create_user(state, email, source).await?, false)
        }
    };
    ensure_active(state, user_id).await?;
    if srp_login {
        let (mfa,): (i64,) = sqlx::query_as(
            "SELECT CASE WHEN email_mfa THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&state.db.pool)
        .await?;
        if mfa != 0 {
            return Err(email_mfa_enabled());
        }
    }
    let passkeys = has_passkeys(state, user_id).await?;
    let keys = key_attributes(&state.db, user_id).await?;
    let now = now_micros();
    let mut resp = EmailAuthorizationResponse {
        id: user_id,
        ..Default::default()
    };
    if passkeys || two_factor {
        if passkeys {
            resp.passkey_session_id = random_url_safe(32);
            resp.accounts_url = accounts_url(state);
            sqlx::query("INSERT INTO passkey_login_sessions (user_id, session_id, creation_time, expiration_time) VALUES ($1, $2, $3, $4)")
                .bind(user_id)
                .bind(&resp.passkey_session_id)
                .bind(now)
                .bind(now + TWO_FACTOR_VALIDITY)
                .execute(&state.db.pool)
                .await?;
        }
        if two_factor {
            let sid = random_url_safe(32);
            sqlx::query("INSERT INTO two_factor_sessions (user_id, session_id, creation_time, expiration_time) VALUES ($1, $2, $3, $4)")
                .bind(user_id)
                .bind(&sid)
                .bind(now)
                .bind(now + TWO_FACTOR_VALIDITY)
                .execute(&state.db.pool)
                .await?;
            if passkeys {
                resp.two_factor_session_id_v2 = sid;
            } else {
                resp.two_factor_session_id = sid;
            }
        }
        return Ok(resp);
    }
    match keys {
        None => {
            let token = random_url_safe(32);
            insert_token(
                &state.db.pool,
                user_id,
                app,
                &token,
                &client.ip,
                &client.user_agent,
            )
            .await?;
            resp.token = token;
        }
        Some(k) => {
            let token = generate_auth_token();
            resp.encrypted_token = seal_token(&token, &k.public_key)?;
            insert_token(
                &state.db.pool,
                user_id,
                app,
                &token,
                &client.ip,
                &client.user_agent,
            )
            .await?;
            resp.key_attributes = Some(k);
            notify_login(state, user_id).await;
        }
    }
    Ok(resp)
}

/// Museum `notifyLogin`: a "new login" email unless email MFA is on (email MFA users
/// already received an OTT).
pub async fn notify_login(state: &AppState, user_id: i64) {
    let mfa: Option<(i64,)> = sqlx::query_as(
        "SELECT CASE WHEN email_mfa THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await
    .ok()
    .flatten();
    if matches!(mfa, Some((n,)) if n != 0) {
        return;
    }
    if let Ok(Some(email)) = email_for_user(state, user_id).await {
        let date = chrono_like_now();
        crate::mail::deliver(
            &state.config,
            &email,
            "New login to your account",
            &format!("Your account was logged into on {date} (UTC). If this was not you, change your password and log out other sessions.\n"),
        )
        .await;
    }
}

fn chrono_like_now() -> String {
    let secs = now_micros() / 1_000_000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Which pending-login table a second-factor session lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingLogin {
    /// No pending session (direct token).
    None,
    /// `two_factor_sessions`.
    Totp,
    /// `passkey_login_sessions`.
    Passkey,
}

/// Museum `GetKeyAttributeAndToken`: consume the pending session and mint a sealed token.
/// `publish` stores the response for `/users/two-factor/passkeys/get-token` polling.
pub async fn key_attributes_and_token(
    state: &AppState,
    client: &ClientInfo,
    user_id: i64,
    session_id: &str,
    session: PendingLogin,
    disable_two_factor: bool,
    publish: bool,
) -> ApiResult<TwoFactorAuthorizationResponse> {
    let keys = key_attributes(&state.db, user_id)
        .await?
        .ok_or_else(not_found_plain)?;
    let token = generate_auth_token();
    let resp = TwoFactorAuthorizationResponse {
        id: user_id,
        encrypted_token: seal_token(&token, &keys.public_key)?,
        key_attributes: Some(keys),
    };
    ensure_active(state, user_id).await?;
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    let consumed = match session {
        PendingLogin::None => 1,
        PendingLogin::Totp => sqlx::query(
            "DELETE FROM two_factor_sessions WHERE session_id = $1 AND user_id = $2 AND expiration_time > $3",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
        PendingLogin::Passkey if publish => sqlx::query(
            "UPDATE passkey_login_sessions SET token_data = $1, verified_at = $2 WHERE session_id = $3 AND user_id = $4 AND expiration_time > $5 AND verified_at IS NULL",
        )
        .bind(serde_json::to_string(&resp).map_err(ApiError::internal)?)
        .bind(now)
        .bind(session_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
        PendingLogin::Passkey => sqlx::query(
            "DELETE FROM passkey_login_sessions WHERE session_id = $1 AND user_id = $2 AND expiration_time > $3 AND verified_at IS NULL",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
    };
    if consumed == 0 {
        // sql.ErrNoRows from the RETURNING scan.
        return Err(not_found_plain());
    }
    if disable_two_factor {
        sqlx::query("UPDATE users SET is_two_factor_enabled = FALSE WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    insert_token(
        &mut *tx,
        user_id,
        client.app(),
        &token,
        &client.ip,
        &client.user_agent,
    )
    .await?;
    tx.commit().await?;
    notify_login(state, user_id).await;
    Ok(resp)
}

/// 404 `{}` or the user id of a live TOTP session (museum `GetUserIDWithTwoFactorSession`).
pub async fn user_for_two_factor_session(state: &AppState, session_id: &str) -> ApiResult<i64> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM two_factor_sessions WHERE session_id = $1 AND expiration_time > $2",
    )
    .bind(session_id)
    .bind(now_micros())
    .fetch_optional(&state.db.pool)
    .await?;
    row.map(|r| r.0).ok_or_else(not_found_plain)
}

/// 404 `{}` or the user id of a passkey login session.
pub async fn user_for_passkey_session(state: &AppState, session_id: &str) -> ApiResult<i64> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM passkey_login_sessions WHERE session_id = $1")
            .bind(session_id)
            .fetch_optional(&state.db.pool)
            .await?;
    row.map(|r| r.0).ok_or_else(not_found_plain)
}

/// Museum's `errJWTExpired` (400 `BAD_REQUEST`).
pub fn jwt_expired() -> ApiError {
    api(StatusCode::BAD_REQUEST, "BAD_REQUEST", "token expired")
}

/// Mint a museum web JWT for `user_id` (`GetJWTToken` / `GetSessionJWTToken`).
pub fn session_jwt(
    state: &AppState,
    user_id: i64,
    scope: &str,
    session_token_hash_hex: Option<&str>,
    app: Option<App>,
) -> String {
    let expiry = if scope == crate::jwt::scope::ACCOUNTS {
        now_micros() + 30 * 60 * 1_000_000
    } else {
        now_micros() + 24 * 3600 * 1_000_000
    };
    let claim = crate::jwt::WebClaim {
        user_id,
        expiry_time: expiry,
        email: String::new(),
        claim_scope: Some(scope.to_string()),
        session_token_hash: session_token_hash_hex
            .and_then(|h| hex::decode(h).ok())
            .map(|b| crate::crypto::b64(&b))
            .unwrap_or_default(),
        session_app: app.map(|a| a.as_str().to_string()).unwrap_or_default(),
    };
    crate::jwt::sign(&state.keys.jwt, &claim)
}

/// Validate a web JWT like museum's `ValidateJWTToken`.
pub fn validate_jwt(state: &AppState, token: &str, scope: &str) -> ApiResult<crate::jwt::WebClaim> {
    crate::jwt::verify_web(&state.keys.jwt, token, scope).map_err(|e| match e {
        crate::jwt::JwtError::Expired => jwt_expired(),
        _ => ApiError::internal(format!("jwt validation failed: {e}")),
    })
}
