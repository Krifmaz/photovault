//! TOTP two-factor, passkey login ceremony and second-factor recovery (museum
//! `pkg/controller/user/twofactor.go`, `passkey.go`, `pkg/api/user.go`).

use super::*;
use crate::crypto::{b64, unb64};
use crate::db::now_micros;
use crate::error::ApiError;
use crate::users::login::PendingLogin;
use crate::users::totp;
use crate::users::webauthn::CredentialJson;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub(super) async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    let (s,): (i64,) = sqlx::query_as(
        "SELECT CASE WHEN is_two_factor_enabled THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&state.db.pool)
    .await?;
    Ok(Json(json!({ "status": s != 0 })))
}

pub(super) async fn setup(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let u = users::get_user(&state, user.user_id).await?;
    if users::key_attributes(&state.db, u.id).await?.is_none() {
        return Err(users::not_found_plain());
    }
    let secret = totp::generate_secret();
    let (ct, nonce) = users::encrypt_secret(&state, &secret)?;
    let hash = users::keyed_hash(&state, &secret);
    let now = now_micros();
    sqlx::query("INSERT INTO temp_two_factor (user_id, encrypted_two_factor_secret, two_factor_secret_decryption_nonce, two_factor_secret_hash, creation_time, expiration_time) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(u.id)
        .bind(ct)
        .bind(nonce)
        .bind(hash)
        .bind(now)
        .bind(now + login::TWO_FACTOR_VALIDITY)
        .execute(&state.db.pool)
        .await?;
    let png = totp::qr_png(&totp::uri("ente", &u.email, &secret), 200)
        .ok_or_else(|| ApiError::internal("QR encoding failed"))?;
    Ok(Json(json!({ "secretCode": secret, "qrCode": b64(&png) })))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct EnableRequest {
    code: String,
    encrypted_two_factor_secret: String,
    two_factor_secret_decryption_nonce: String,
}

pub(super) async fn enable(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<EnableRequest>,
) -> ApiResult<StatusCode> {
    if unb64(&req.encrypted_two_factor_secret)
        .map(|b| b.len())
        .ok()
        != Some(40)
    {
        return Err(users::bad_request(
            "Invalid two-factor recovery data. Update your client and try again.",
        ));
    }
    if unb64(&req.two_factor_secret_decryption_nonce)
        .map(|b| b.len())
        .ok()
        != Some(24)
    {
        return Err(users::bad_request("Invalid two-factor recovery nonce"));
    }
    let temps: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT encrypted_two_factor_secret, two_factor_secret_decryption_nonce, two_factor_secret_hash FROM temp_two_factor WHERE user_id = $1 AND expiration_time > $2",
    )
    .bind(user.user_id)
    .bind(now_micros())
    .fetch_all(&state.db.pool)
    .await?;
    let mut found = None;
    for (ct, nonce, hash) in temps {
        let secret = users::decrypt_secret(&state, &ct, &nonce)?;
        if totp::validate(&req.code, &secret) {
            found = Some((ct, nonce, hash));
            break;
        }
    }
    let Some((ct, nonce, hash)) = found else {
        return Err(users::unauthorized_plain());
    };
    let mut tx = state.db.pool.begin().await?;
    sqlx::query("DELETE FROM two_factor WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO two_factor (user_id, encrypted_two_factor_secret, two_factor_secret_decryption_nonce, two_factor_secret_hash, recovery_encrypted_two_factor_secret, recovery_two_factor_secret_decryption_nonce) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(user.user_id)
        .bind(ct)
        .bind(nonce)
        .bind(&hash)
        .bind(&req.encrypted_two_factor_secret)
        .bind(&req.two_factor_secret_decryption_nonce)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET is_two_factor_enabled = TRUE WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM temp_two_factor WHERE two_factor_secret_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::OK)
}

pub(super) async fn disable(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<StatusCode> {
    sqlx::query("UPDATE users SET is_two_factor_enabled = FALSE WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(StatusCode::OK)
}

async fn totp_secret(state: &AppState, user_id: i64) -> ApiResult<String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT encrypted_two_factor_secret, two_factor_secret_decryption_nonce FROM two_factor WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (ct, nonce) = row.ok_or_else(users::not_found_plain)?;
    users::decrypt_secret(state, &ct, &nonce)
}

#[derive(Deserialize)]
pub(super) struct VerifyRequest {
    #[serde(rename = "sessionID")]
    session_id: String,
    code: String,
}

pub(super) async fn verify(
    State(state): State<AppState>,
    client: ClientInfo,
    Body(req): Body<VerifyRequest>,
) -> ApiResult<Json<login::TwoFactorAuthorizationResponse>> {
    let user_id = login::user_for_two_factor_session(&state, &req.session_id).await?;
    let (enabled,): (i64,) = sqlx::query_as(
        "SELECT CASE WHEN is_two_factor_enabled THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db.pool)
    .await?;
    if enabled == 0 {
        return Err(users::bad_request_plain());
    }
    let secret = totp_secret(&state, user_id).await?;
    let reserved = sqlx::query("UPDATE two_factor_sessions SET wrong_attempt = wrong_attempt + 1 WHERE session_id = $1 AND expiration_time > $2 AND wrong_attempt < 10")
        .bind(&req.session_id)
        .bind(now_micros())
        .execute(&state.db.pool)
        .await?;
    if reserved.rows_affected() != 1 {
        return Err(users::too_many_plain());
    }
    if !totp::validate(&req.code, &secret) {
        return Err(users::unauthorized_plain());
    }
    let code_hash = hex::encode(Sha256::digest(format!("{user_id}:{}", req.code).as_bytes()));
    let used: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM two_factor_used_codes WHERE user_id = $1 AND code_hash = $2",
    )
    .bind(user_id)
    .bind(&code_hash)
    .fetch_optional(&state.db.pool)
    .await?;
    if used.is_some() {
        tracing::warn!(user_id, "TOTP code replay rejected");
        return Err(users::unauthorized_plain());
    }
    sqlx::query(
        "INSERT INTO two_factor_used_codes (user_id, code_hash, used_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(&code_hash)
    .bind(now_micros())
    .execute(&state.db.pool)
    .await?;
    // Codes are only valid for ±1 step, so older entries can go.
    sqlx::query("DELETE FROM two_factor_used_codes WHERE used_at < $1")
        .bind(now_micros() - 10 * 60 * 1_000_000)
        .execute(&state.db.pool)
        .await?;
    Ok(Json(
        login::key_attributes_and_token(
            &state,
            &client,
            user_id,
            &req.session_id,
            PendingLogin::Totp,
            false,
            false,
        )
        .await?,
    ))
}

async fn recovery_row(
    state: &AppState,
    user_id: i64,
) -> ApiResult<
    Option<(
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
> {
    Ok(sqlx::query_as(
        "SELECT CASE WHEN enable_admin_mfa_reset THEN 1 ELSE 0 END, server_passkey_secret_data, server_passkey_secret_nonce, user_passkey_secret_data, user_passkey_secret_nonce FROM two_factor_recovery WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?)
}

pub(super) async fn recover(State(state): State<AppState>, q: QueryMap) -> ApiResult<Json<Value>> {
    let session_id = q.get("sessionID");
    if q.get("twoFactorType") == "passkey" {
        let user_id = login::user_for_passkey_session(&state, session_id).await?;
        let row = recovery_row(&state, user_id).await?;
        let Some((_, Some(_), _, Some(data), Some(nonce))) = row else {
            return Err(users::bad_request("Passkey reset is not configured"));
        };
        return Ok(Json(
            json!({ "encryptedSecret": data, "secretDecryptionNonce": nonce }),
        ));
    }
    let user_id = login::user_for_two_factor_session(&state, session_id).await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT recovery_encrypted_two_factor_secret, recovery_two_factor_secret_decryption_nonce FROM two_factor WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (data, nonce) = row.ok_or_else(users::not_found_plain)?;
    Ok(Json(
        json!({ "encryptedSecret": data, "secretDecryptionNonce": nonce }),
    ))
}

/// Museum `verifyRecoveryProof`.
fn verify_recovery_proof(secret: &str, factor: &str, session_id: &str, proof: &str) -> bool {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(format!("recovery:{factor}:{session_id}").as_bytes());
    let expected = b64(&mac.finalize().into_bytes());
    bool::from(subtle::ConstantTimeEq::ct_eq(
        expected.as_bytes(),
        proof.as_bytes(),
    ))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct RemoveRequest {
    secret: String,
    proof: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    two_factor_type: String,
}

pub(super) async fn remove(
    State(state): State<AppState>,
    client: ClientInfo,
    Body(req): Body<RemoveRequest>,
) -> ApiResult<Json<login::TwoFactorAuthorizationResponse>> {
    if req.secret.is_empty() == req.proof.is_empty() {
        return Err(users::bad_request("provide exactly one of secret or proof"));
    }
    if !req.proof.is_empty() && req.two_factor_type != "totp" && req.two_factor_type != "passkey" {
        return Err(users::bad_request(
            "proof requires twoFactorType totp or passkey",
        ));
    }
    if req.two_factor_type == "passkey" {
        let user_id = login::user_for_passkey_session(&state, &req.session_id).await?;
        let row = recovery_row(&state, user_id).await?;
        let Some((_, Some(data), Some(nonce), _, _)) = row else {
            return Err(users::not_found_plain());
        };
        let server_secret = users::decrypt_secret(&state, &data, &nonce)?;
        let valid = if req.proof.is_empty() {
            bool::from(subtle::ConstantTimeEq::ct_eq(
                server_secret.as_bytes(),
                req.secret.as_bytes(),
            ))
        } else {
            verify_recovery_proof(&server_secret, "passkey", &req.session_id, &req.proof)
        };
        if !valid {
            return Err(users::permission_denied());
        }
        return Ok(Json(
            login::key_attributes_and_token(
                &state,
                &client,
                user_id,
                &req.session_id,
                PendingLogin::Passkey,
                false,
                false,
            )
            .await?,
        ));
    }
    let user_id = login::user_for_two_factor_session(&state, &req.session_id).await?;
    let valid = if req.proof.is_empty() {
        let hash = users::keyed_hash(&state, &req.secret);
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM two_factor WHERE user_id = $1 AND two_factor_secret_hash = $2",
        )
        .bind(user_id)
        .bind(hash)
        .fetch_optional(&state.db.pool)
        .await?;
        row.is_some()
    } else {
        let secret = totp_secret(&state, user_id).await?;
        verify_recovery_proof(&secret, "totp", &req.session_id, &req.proof)
    };
    if !valid {
        return Err(users::permission_denied());
    }
    Ok(Json(
        login::key_attributes_and_token(
            &state,
            &client,
            user_id,
            &req.session_id,
            PendingLogin::Totp,
            true,
            false,
        )
        .await?,
    ))
}

pub(super) async fn recovery_status(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    let row = recovery_row(&state, user.user_id).await?;
    Ok(Json(match row {
        None => json!({ "allowAdminReset": true, "isPasskeyRecoveryEnabled": false }),
        Some((admin, server, ..)) => json!({
            "allowAdminReset": admin != 0,
            "isPasskeyRecoveryEnabled": server.is_some_and(|s| !s.is_empty()),
        }),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PasskeyRecoveryRequest {
    secret: String,
    user_secret_cipher: String,
    user_secret_nonce: String,
}

pub(super) async fn configure_passkey_recovery(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<PasskeyRecoveryRequest>,
) -> ApiResult<Json<Value>> {
    if req.secret.is_empty()
        || req.user_secret_cipher.is_empty()
        || req.user_secret_nonce.is_empty()
    {
        return Err(users::bad_request_plain());
    }
    let (ct, nonce) = users::encrypt_secret(&state, &req.secret)?;
    let now = now_micros();
    let row = recovery_row(&state, user.user_id).await?;
    match row {
        None => {
            sqlx::query("INSERT INTO two_factor_recovery (user_id, server_passkey_secret_data, server_passkey_secret_nonce, user_passkey_secret_data, user_passkey_secret_nonce, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                .bind(user.user_id)
                .bind(ct)
                .bind(nonce)
                .bind(&req.user_secret_cipher)
                .bind(&req.user_secret_nonce)
                .bind(now)
                .bind(now)
                .execute(&state.db.pool)
                .await?;
        }
        // Museum only fills the secret once (ON CONFLICT ... WHERE both are NULL).
        Some((_, None, _, None, _)) => {
            sqlx::query("UPDATE two_factor_recovery SET server_passkey_secret_data = $1, server_passkey_secret_nonce = $2, user_passkey_secret_data = $3, user_passkey_secret_nonce = $4, updated_at = $5 WHERE user_id = $6")
                .bind(ct)
                .bind(nonce)
                .bind(&req.user_secret_cipher)
                .bind(&req.user_secret_nonce)
                .bind(now)
                .bind(user.user_id)
                .execute(&state.db.pool)
                .await?;
        }
        Some(_) => {}
    }
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
pub(super) struct BeginRequest {
    #[serde(rename = "sessionID")]
    session_id: String,
}

pub(super) async fn passkey_begin(
    State(state): State<AppState>,
    Body(req): Body<BeginRequest>,
) -> ApiResult<Json<Value>> {
    let user_id = login::user_for_passkey_session(&state, &req.session_id).await?;
    let claimed: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT verified_at FROM passkey_login_sessions WHERE session_id = $1")
            .bind(&req.session_id)
            .fetch_optional(&state.db.pool)
            .await?;
    if matches!(claimed, Some((Some(_),))) {
        return Err(users::api(
            StatusCode::CONFLICT,
            "SESSION_ALREADY_CLAIMED",
            "Session is already claimed",
        ));
    }
    users::get_user(&state, user_id).await?;
    let (options, ceremony) = users::passkeys::begin_authentication(&state, user_id).await?;
    Ok(Json(
        json!({ "options": options, "ceremonySessionID": ceremony }),
    ))
}

pub(super) async fn passkey_finish(
    State(state): State<AppState>,
    client: ClientInfo,
    q: QueryMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<login::TwoFactorAuthorizationResponse>> {
    let session_id = q.get("sessionID").to_string();
    let ceremony = q.get("ceremonySessionID");
    if session_id.is_empty() || ceremony.is_empty() {
        return Err(users::bad_request_plain());
    }
    let ceremony = uuid::Uuid::parse_str(ceremony)
        .map_err(|_| users::bad_request_plain())?
        .to_string();
    let user_id = login::user_for_passkey_session(&state, &session_id).await?;
    users::get_user_not_deleted(&state, user_id).await?;
    let cred: CredentialJson =
        serde_json::from_slice(&body).map_err(|_| users::bad_request("Error parsing assertion"))?;
    users::passkeys::finish_authentication(&state, user_id, &ceremony, &cred).await?;
    Ok(Json(
        login::key_attributes_and_token(
            &state,
            &client,
            user_id,
            &session_id,
            PendingLogin::Passkey,
            false,
            true,
        )
        .await?,
    ))
}

pub(super) async fn passkey_get_token(
    State(state): State<AppState>,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let session_id = q.get("sessionID");
    if session_id.is_empty() {
        return Err(users::bad_request("sessionID is required"));
    }
    let row: Option<(Option<String>, Option<i64>, i64)> = sqlx::query_as(
        "SELECT token_data, verified_at, token_fetch_cnt FROM passkey_login_sessions WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (data, verified, fetches) = row.ok_or_else(users::not_found_plain)?;
    let Some(verified) = verified else {
        return Err(users::api(
            StatusCode::BAD_REQUEST,
            "SESSION_NOT_VERIFIED",
            "Session is not verified yet",
        ));
    };
    if verified < now_micros() - 2 * 60 * 1_000_000 {
        return Err(users::api(
            StatusCode::GONE,
            "INVALID_SESSION",
            "Session verified but expired now",
        ));
    }
    if fetches >= 2 {
        return Err(users::api(
            StatusCode::GONE,
            "INVALID_SESSION",
            "Token fetch limit reached",
        ));
    }
    let data: Value =
        serde_json::from_str(data.as_deref().unwrap_or("null")).map_err(ApiError::internal)?;
    sqlx::query("UPDATE passkey_login_sessions SET token_fetch_cnt = token_fetch_cnt + 1 WHERE session_id = $1")
        .bind(session_id)
        .execute(&state.db.pool)
        .await?;
    Ok(Json(data))
}
