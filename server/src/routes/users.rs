//! Endpoints: /users/* (OTT, email verify, SRP, 2FA/TOTP, passkey 2FA ceremony, attributes, recovery key, public keys, sessions, logout, details, delete, change email, account recovery, tokens).
//!
//! Reference: museum `cmd/museum/main.go` and the matching `pkg/api` / `pkg/controller` files.

mod account;
mod srp;
mod twofactor;

pub(crate) use account::details_v2;

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::jwt::scope;
use crate::state::AppState;
use crate::users::extract::{Body, ClientInfo, QueryMap};
use crate::users::login;
use crate::users::{self, Revoke};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/ott", post(send_ott))
        .route("/users/verify-email", post(verify_email))
        .route("/users/change-email", post(change_email))
        .route("/users/logout", post(logout))
        .route("/users/sessions", get(sessions))
        .route("/users/session", delete(terminate_session))
        .route("/users/payment-token", get(payment_token))
        .route("/users/families-token", get(families_token))
        .route("/users/accounts-token", get(accounts_token))
        .route("/users/event", post(report_event))
        // Two-factor
        .route("/users/two-factor/verify", post(twofactor::verify))
        .route("/users/two-factor/recover", get(twofactor::recover))
        .route("/users/two-factor/remove", post(twofactor::remove))
        .route(
            "/users/two-factor/passkeys/begin",
            post(twofactor::passkey_begin),
        )
        .route(
            "/users/two-factor/passkeys/finish",
            post(twofactor::passkey_finish),
        )
        .route(
            "/users/two-factor/passkeys/get-token",
            get(twofactor::passkey_get_token),
        )
        .route(
            "/users/two-factor/recovery-status",
            get(twofactor::recovery_status),
        )
        .route(
            "/users/two-factor/passkeys/configure-recovery",
            post(twofactor::configure_passkey_recovery),
        )
        .route("/users/two-factor/status", get(twofactor::status))
        .route("/users/two-factor/setup", post(twofactor::setup))
        .route("/users/two-factor/enable", post(twofactor::enable))
        .route("/users/two-factor/disable", post(twofactor::disable))
        // SRP
        .route("/users/srp/setup", post(srp::setup))
        .route("/users/srp/complete", post(srp::complete))
        .route("/users/srp/update", post(srp::update))
        .route("/users/srp/attributes", get(srp::attributes))
        .route("/users/srp/create-session", post(srp::create_session))
        .route("/users/srp/verify-session", post(srp::verify_session))
        // Account
        .route("/users/attributes", put(account::set_attributes))
        .route("/users/email-mfa", put(account::update_email_mfa))
        .route("/users/recovery-key", put(account::set_recovery_key))
        .route("/users/public-key", get(account::public_key))
        .route("/users/public-keys", post(account::public_keys))
        .route("/users/session-validity/v2", get(account::session_validity))
        .route("/users/details/v2", get(account::details))
        .route("/users/deletion-summary", get(account::deletion_summary))
        .route("/users/locker-usage", get(account::locker_usage))
        .route("/users/delete-challenge", get(account::delete_challenge))
        .route("/users/delete", delete(account::delete_account))
        .route(
            "/users/recover-account",
            get(account::recover_account_link).post(account::recover_account),
        )
        .route(
            "/users/recover-account/validate",
            post(account::validate_recover_account),
        )
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SendOttRequest {
    email: String,
    purpose: String,
    #[allow(dead_code)]
    mobile: bool,
}

async fn send_ott(
    State(state): State<AppState>,
    client: ClientInfo,
    Body(req): Body<SendOttRequest>,
) -> ApiResult<StatusCode> {
    let email = users::normalize_email(&req.email);
    if email.is_empty() {
        return Err(users::bad_request_plain());
    }
    login::send_email_ott(&state, &client, &email, &req.purpose).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EmailVerificationRequest {
    email: String,
    ott: String,
    source: Option<String>,
}

async fn verify_email(
    State(state): State<AppState>,
    client: ClientInfo,
    Body(req): Body<EmailVerificationRequest>,
) -> ApiResult<Json<login::EmailAuthorizationResponse>> {
    let email = users::normalize_email(&req.email);
    login::verify_email_ott_or_skip(&state, client.app(), &email, &req.ott).await?;
    Ok(Json(
        login::on_verification_success(&state, &client, &email, req.source.as_deref(), false)
            .await?,
    ))
}

async fn change_email(
    State(state): State<AppState>,
    user: AuthUser,
    client: ClientInfo,
    Body(req): Body<EmailVerificationRequest>,
) -> ApiResult<StatusCode> {
    let email = users::normalize_email(&req.email);
    login::verify_email_ott(&state, client.app(), &email, &req.ott).await?;
    update_email(&state, user.user_id, &email).await?;
    Ok(StatusCode::OK)
}

/// Museum `UpdateEmail` (also used by the admin API).
pub(crate) async fn update_email(state: &AppState, user_id: i64, email: &str) -> ApiResult<()> {
    let email = users::normalize_email(email);
    if users::user_id_for_email(state, &email).await?.is_some() {
        return Err(users::permission_denied());
    }
    let user = users::get_user(state, user_id).await?;
    let (ct, nonce) = users::encrypt_email(state, &email)?;
    sqlx::query("UPDATE users SET encrypted_email = $1, email_decryption_nonce = $2, email_hash = $3 WHERE user_id = $4")
        .bind(ct)
        .bind(nonce)
        .bind(users::email_hash(&state.keys, &email))
        .bind(user_id)
        .execute(&state.db.pool)
        .await?;
    crate::mail::deliver(
        &state.config,
        &user.email,
        "Email address updated",
        &format!("The email address of your account was changed to {email}.\n"),
    )
    .await;
    Ok(())
}

async fn logout(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    users::revoke_tokens(&state.db, user.user_id, Revoke::One(&user.token_hash)).await?;
    Ok(Json(json!({})))
}

/// Short device description (museum uses uap-go's `GetPrettyUA`).
pub(crate) fn pretty_ua(ua: &str) -> String {
    let os = if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iOS") {
        "iOS"
    } else if ua.contains("Mac OS X") || ua.contains("Darwin") {
        "Mac OS X"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Other"
    };
    let family = if ua.contains("Electron") {
        "Desktop App"
    } else if ua.contains("Dart") || ua.contains("okhttp") {
        "Mobile App"
    } else if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Edg") {
        "Edge"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        "Other"
    };
    format!("{family}, {os}")
}

async fn sessions(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    type Row = (String, i64, Option<String>, Option<String>, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT token_hash, creation_time, ip, user_agent, last_used_at FROM tokens WHERE user_id = $1 AND app = $2 AND is_deleted = FALSE ORDER BY creation_time",
    )
    .bind(user.user_id)
    .bind(user.app.as_str())
    .fetch_all(&state.db.pool)
    .await?;
    let list: Vec<Value> = rows
        .into_iter()
        .map(|(hash, created, ip, ua, last)| {
            let raw = hex::decode(&hash).unwrap_or_default();
            let (ua, pretty) = match ua.filter(|u| !u.is_empty()) {
                Some(u) => {
                    let p = pretty_ua(&u);
                    (u, p)
                }
                None => ("Unknown Device".to_string(), "Unknown Device".to_string()),
            };
            json!({
                "token": format!("th:{}", users::webauthn::encode_b64url(&raw)),
                "isCurrent": hash == user.token_hash,
                "creationTime": created,
                "ip": ip.filter(|i| !i.is_empty()).unwrap_or_else(|| "Unknown IP".into()),
                "ua": ua,
                "prettyUA": pretty,
                "lastUsedTime": last,
            })
        })
        .collect();
    Ok(Json(json!({ "sessions": list })))
}

async fn terminate_session(
    State(state): State<AppState>,
    user: AuthUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let ident = q.get("token");
    let hash = match ident.strip_prefix("th:") {
        Some(enc) => {
            let raw = users::webauthn::decode_b64url(enc)
                .ok()
                .filter(|r| r.len() == 32)
                .ok_or_else(|| users::bad_request("invalid session identifier"))?;
            hex::encode(raw)
        }
        None => crate::auth::hash_token(ident),
    };
    if hash == user.token_hash {
        return Err(users::bad_request(
            "use logout to terminate the current session",
        ));
    }
    users::revoke_tokens(&state.db, user.user_id, Revoke::One(&hash)).await?;
    Ok(Json(json!({})))
}

async fn payment_token(State(state): State<AppState>, user: AuthUser) -> Json<Value> {
    let t = login::session_jwt(
        &state,
        user.user_id,
        scope::PAYMENT,
        Some(&user.token_hash),
        Some(user.app),
    );
    Json(json!({ "paymentToken": t }))
}

async fn families_token(State(state): State<AppState>, user: AuthUser) -> Json<Value> {
    let t = login::session_jwt(
        &state,
        user.user_id,
        scope::FAMILIES,
        Some(&user.token_hash),
        Some(user.app),
    );
    Json(json!({ "familiesToken": t, "familyUrl": state.config.accounts.family_url }))
}

async fn accounts_token(State(state): State<AppState>, user: AuthUser) -> Json<Value> {
    let t = login::session_jwt(
        &state,
        user.user_id,
        scope::ACCOUNTS,
        Some(&user.token_hash),
        Some(user.app),
    );
    Json(json!({ "accountsToken": t, "accountsUrl": login::accounts_url(&state) }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EventReport {
    #[allow(dead_code)]
    event: String,
}

async fn report_event(_user: AuthUser, Body(_req): Body<EventReport>) -> StatusCode {
    StatusCode::OK
}
