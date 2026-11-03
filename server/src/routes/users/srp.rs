//! SRP endpoints (museum `pkg/controller/user/srp.go`, `pkg/repo/srp.go`). Byte-compatible
//! with the apps' go-srp / `srp` crate client: 4096-bit group, SHA-256, 512-byte A/B.

use super::*;
use crate::crypto::{b64, random_bytes, unb64};
use crate::db::now_micros;
use crate::error::ApiError;
use crate::users::srp as srp_math;

/// Museum `MaxUnverifiedSessionInAnHour`.
const MAX_UNVERIFIED_SESSIONS_PER_HOUR: i64 = 10;
/// Museum's attempt limit per SRP session.
const MAX_ATTEMPTS: i64 = 5;
const HOUR: i64 = 3600 * 1_000_000;

fn parse_uuid(s: &str) -> ApiResult<String> {
    uuid::Uuid::parse_str(s)
        .map(|u| u.to_string())
        .map_err(|_| users::bad_request_plain())
}

fn decode_b64(s: &str) -> ApiResult<Vec<u8>> {
    unb64(s).map_err(|_| users::bad_request("invalid base64 encoding"))
}

fn api_code(status: StatusCode, code: &'static str) -> ApiError {
    users::api(status, code, "")
}

/// Museum `createAndInsertSRPSession`: returns `(srpB b64, sessionID)`.
async fn create_session_row(
    state: &AppState,
    srp_user_id: &str,
    verifier_b64: &str,
    srp_a: &str,
) -> ApiResult<(String, String)> {
    let a = decode_b64(srp_a)?;
    if a.len() != srp_math::N_BYTES {
        return Err(users::bad_request("Invalid length for srpA"));
    }
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM srp_sessions WHERE srp_user_id = $1 AND has_verified = FALSE AND created_at > $2",
    )
    .bind(srp_user_id)
    .bind(now_micros() - HOUR)
    .fetch_one(&state.db.pool)
    .await?;
    if n >= MAX_UNVERIFIED_SESSIONS_PER_HOUR {
        return Err(api_code(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_UNVERIFIED_SESSIONS",
        ));
    }
    let verifier = decode_b64(verifier_b64)?;
    let b_secret = random_bytes(32);
    let b = srp_math::server_b(&verifier, &b_secret);
    let id = users::new_uuid();
    sqlx::query("INSERT INTO srp_sessions (id, srp_user_id, server_key, srp_a, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5)")
        .bind(&id)
        .bind(srp_user_id)
        .bind(b64(&b_secret))
        .bind(srp_a)
        .bind(now_micros())
        .execute(&state.db.pool)
        .await?;
    Ok((b64(&b), id))
}

/// Museum `verifySRPSession`. `verifier` is `None` for unknown SRP users (always fails).
async fn verify_session_row(
    state: &AppState,
    srp_user_id: &str,
    verifier: Option<&str>,
    session_id: &str,
    m1_b64: &str,
) -> ApiResult<String> {
    let m1 = decode_b64(m1_b64)?;
    if m1.len() != 32 {
        return Err(users::bad_request(format!(
            "srpM1 size is {}, expected 32",
            m1.len()
        )));
    }
    let r = sqlx::query(
        "UPDATE srp_sessions SET attempt_count = attempt_count + 1 WHERE id = $1 AND has_verified = FALSE AND attempt_count < $2",
    )
    .bind(session_id)
    .bind(MAX_ATTEMPTS)
    .execute(&state.db.pool)
    .await?;
    let row: Option<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT srp_user_id, server_key, srp_a, CASE WHEN has_verified THEN 1 ELSE 0 END, CASE WHEN is_fake THEN 1 ELSE 0 END FROM srp_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let Some((sess_user, server_key, srp_a, verified, fake)) = row else {
        return Err(users::unauthorized_plain());
    };
    if r.rows_affected() == 0 {
        return Err(if verified != 0 {
            api_code(StatusCode::GONE, "SESSION_ALREADY_VERIFIED")
        } else {
            api_code(StatusCode::GONE, "TOO_MANY_WRONG_ATTEMPTS")
        });
    }
    if fake != 0 || sess_user != srp_user_id {
        return Err(users::unauthorized_plain());
    }
    let Some(verifier) = verifier else {
        return Err(users::unauthorized_plain());
    };
    let m2 = srp_math::server_check_m1(
        &decode_b64(verifier)?,
        &decode_b64(&server_key)?,
        &decode_b64(&srp_a)?,
        &m1,
    )
    .map_err(|_| users::unauthorized_plain())?;
    let claimed = sqlx::query(
        "UPDATE srp_sessions SET has_verified = TRUE WHERE id = $1 AND has_verified = FALSE",
    )
    .bind(session_id)
    .execute(&state.db.pool)
    .await?;
    if claimed.rows_affected() == 0 {
        return Err(api_code(StatusCode::GONE, "SESSION_ALREADY_VERIFIED"));
    }
    Ok(b64(&m2))
}

#[derive(Deserialize)]
pub(super) struct SetupSrpRequest {
    #[serde(rename = "srpUserID")]
    srp_user_id: String,
    #[serde(rename = "srpSalt")]
    srp_salt: String,
    #[serde(rename = "srpVerifier")]
    srp_verifier: String,
    #[serde(rename = "srpA")]
    srp_a: String,
}

pub(super) async fn setup(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<SetupSrpRequest>,
) -> ApiResult<Json<Value>> {
    let srp_user_id = parse_uuid(&req.srp_user_id)?;
    if req.srp_salt.is_empty() || req.srp_verifier.is_empty() {
        return Err(users::bad_request_plain());
    }
    let (b, session_id) =
        create_session_row(&state, &srp_user_id, &req.srp_verifier, &req.srp_a).await?;
    let setup_id = users::new_uuid();
    sqlx::query("INSERT INTO temp_srp_setup (id, session_id, user_id, srp_user_id, salt, verifier, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&setup_id)
        .bind(&session_id)
        .bind(user.user_id)
        .bind(&srp_user_id)
        .bind(&req.srp_salt)
        .bind(&req.srp_verifier)
        .bind(now_micros())
        .execute(&state.db.pool)
        .await?;
    Ok(Json(json!({ "setupID": setup_id, "srpB": b })))
}

struct TempSetup {
    session_id: String,
    srp_user_id: String,
    salt: String,
    verifier: String,
}

async fn temp_setup(state: &AppState, setup_id: &str, user_id: i64) -> ApiResult<TempSetup> {
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT session_id, srp_user_id, salt, verifier FROM temp_srp_setup WHERE id = $1 AND user_id = $2",
    )
    .bind(setup_id)
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (session_id, srp_user_id, salt, verifier) = row.ok_or_else(users::not_found_plain)?;
    Ok(TempSetup {
        session_id,
        srp_user_id,
        salt,
        verifier,
    })
}

#[derive(Deserialize)]
pub(super) struct CompleteRequest {
    #[serde(rename = "setupID")]
    setup_id: String,
    #[serde(rename = "srpM1")]
    srp_m1: String,
}

pub(super) async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<CompleteRequest>,
) -> ApiResult<Json<Value>> {
    let setup_id = parse_uuid(&req.setup_id)?;
    let s = temp_setup(&state, &setup_id, user.user_id).await?;
    let m2 = verify_session_row(
        &state,
        &s.srp_user_id,
        Some(&s.verifier),
        &s.session_id,
        &req.srp_m1,
    )
    .await?;
    let now = now_micros();
    sqlx::query("INSERT INTO srp_auth (user_id, srp_user_id, salt, verifier, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(user.user_id)
        .bind(&s.srp_user_id)
        .bind(&s.salt)
        .bind(&s.verifier)
        .bind(now)
        .bind(now)
        .execute(&state.db.pool)
        .await?;
    Ok(Json(json!({ "setupID": setup_id, "srpM2": m2 })))
}

/// Museum `ente.UpdateKeysRequest`.
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct UpdateKeys {
    pub(crate) kek_salt: String,
    pub(crate) encrypted_key: String,
    pub(crate) key_decryption_nonce: String,
    pub(crate) mem_limit: i64,
    pub(crate) ops_limit: i64,
}

/// Museum `ente.ExpectedKDFStrength`.
pub(crate) const EXPECTED_KDF_STRENGTH: i64 = 1_073_741_824 * 4;

/// Museum `ValidateBase64Length`.
pub(crate) fn validate_b64_len(v: &str, size: usize) -> ApiResult<()> {
    use base64::Engine;
    if v.len()
        != base64::engine::general_purpose::STANDARD
            .encode(vec![0u8; size])
            .len()
    {
        return Err(users::bad_request("invalid length"));
    }
    match unb64(v) {
        Ok(d) if d.len() == size => Ok(()),
        Ok(_) => Err(users::bad_request("invalid length")),
        Err(_) => Err(users::bad_request("invalid base64")),
    }
}

impl UpdateKeys {
    pub(crate) fn validate(&self) -> ApiResult<()> {
        if self.mem_limit.checked_mul(self.ops_limit) != Some(EXPECTED_KDF_STRENGTH) {
            return Err(users::bad_request("Unexpected KDF strength"));
        }
        if self.mem_limit < 128 * 1024 * 1024 {
            return Err(users::bad_request("memory limit must be at least 128MB"));
        }
        validate_b64_len(&self.kek_salt, 16)?;
        validate_b64_len(&self.encrypted_key, 48)?;
        validate_b64_len(&self.key_decryption_nonce, 24)
    }
}

/// Go's JSON decoding matches field names case-insensitively, and the apps' Rust
/// client sends `setupId` here (camelCase), so both spellings are accepted.
#[derive(Deserialize)]
pub(super) struct UpdateRequest {
    #[serde(rename = "setupID", alias = "setupId")]
    setup_id: String,
    #[serde(rename = "srpM1")]
    srp_m1: String,
    #[serde(rename = "updatedKeyAttr")]
    updated_key_attr: Option<UpdateKeys>,
    #[serde(rename = "logOutOtherDevices")]
    log_out_other_devices: Option<bool>,
}

pub(super) async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<UpdateRequest>,
) -> ApiResult<Json<Value>> {
    let keys = req
        .updated_key_attr
        .clone()
        .ok_or_else(|| users::bad_request("updatedKeyAttr is required"))?;
    keys.validate()?;
    let clear = req.log_out_other_devices.unwrap_or(true);
    let setup_id = parse_uuid(&req.setup_id)?;
    let s = temp_setup(&state, &setup_id, user.user_id).await?;
    let m2 = verify_session_row(
        &state,
        &s.srp_user_id,
        Some(&s.verifier),
        &s.session_id,
        &req.srp_m1,
    )
    .await?;
    apply_srp_and_keys(
        &state,
        user.user_id,
        &s.srp_user_id,
        &s.salt,
        &s.verifier,
        &keys,
    )
    .await?;
    sqlx::query("DELETE FROM two_factor_sessions WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    if clear {
        sqlx::query("DELETE FROM passkey_login_sessions WHERE user_id = $1")
            .bind(user.user_id)
            .execute(&state.db.pool)
            .await?;
        users::revoke_tokens(&state.db, user.user_id, Revoke::AllExcept(&user.token_hash)).await?;
    } else {
        sqlx::query(
            "DELETE FROM passkey_login_sessions WHERE user_id = $1 AND verified_at IS NULL",
        )
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    }
    Ok(Json(json!({ "setupID": setup_id, "srpM2": m2 })))
}

/// Upsert `srp_auth` and update the password-derived key attributes.
pub(crate) async fn apply_srp_and_keys(
    state: &AppState,
    user_id: i64,
    srp_user_id: &str,
    salt: &str,
    verifier: &str,
    keys: &UpdateKeys,
) -> ApiResult<()> {
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    let exists: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM srp_auth WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        sqlx::query("UPDATE srp_auth SET srp_user_id = $1, salt = $2, verifier = $3, updated_at = $4 WHERE user_id = $5")
            .bind(srp_user_id)
            .bind(salt)
            .bind(verifier)
            .bind(now)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO srp_auth (user_id, srp_user_id, salt, verifier, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(user_id)
            .bind(srp_user_id)
            .bind(salt)
            .bind(verifier)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE key_attributes SET kek_salt = $1, encrypted_key = $2, key_decryption_nonce = $3, mem_limit = $4, ops_limit = $5 WHERE user_id = $6")
        .bind(&keys.kek_salt)
        .bind(&keys.encrypted_key)
        .bind(&keys.key_decryption_nonce)
        .bind(keys.mem_limit)
        .bind(keys.ops_limit)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn attributes(
    State(state): State<AppState>,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let email = users::normalize_email(q.get("email"));
    if email.is_empty() {
        return Err(users::bad_request_plain());
    }
    let Some(user_id) = users::user_id_for_email(&state, &email).await? else {
        // Museum answers with deterministic fake attributes only once its per-IP limiter
        // trips; normally an unknown account is a plain 404, which the apps handle.
        return Err(users::not_found_plain());
    };
    type Row = (
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<String>,
        i64,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT s.srp_user_id, s.salt, k.mem_limit, k.ops_limit, k.kek_salt, CASE WHEN u.email_mfa THEN 1 ELSE 0 END FROM srp_auth s \
         LEFT JOIN key_attributes k ON k.user_id = s.user_id JOIN users u ON u.user_id = s.user_id WHERE s.user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let Some((srp_user_id, salt, mem, ops, kek, mfa)) = row else {
        return Err(users::not_found_typed("srp attributes are not present"));
    };
    let (Some(mem), Some(ops), Some(kek)) = (mem, ops, kek) else {
        sqlx::query("DELETE FROM srp_auth WHERE user_id = $1")
            .bind(user_id)
            .execute(&state.db.pool)
            .await?;
        return Err(users::not_found_typed("key attributes are not present"));
    };
    Ok(Json(json!({
        "attributes": {
            "srpUserID": srp_user_id,
            "srpSalt": salt,
            "memLimit": mem,
            "opsLimit": ops,
            "kekSalt": kek,
            "isEmailMFAEnabled": mfa != 0,
        }
    })))
}

#[derive(Deserialize)]
pub(super) struct CreateSessionRequest {
    #[serde(rename = "srpUserID")]
    srp_user_id: String,
    #[serde(rename = "srpA")]
    srp_a: String,
}

async fn srp_auth_by_srp_user(
    state: &AppState,
    srp_user_id: &str,
) -> ApiResult<Option<(i64, String)>> {
    Ok(
        sqlx::query_as("SELECT user_id, verifier FROM srp_auth WHERE srp_user_id = $1")
            .bind(srp_user_id)
            .fetch_optional(&state.db.pool)
            .await?,
    )
}

async fn email_mfa_on(state: &AppState, user_id: i64) -> ApiResult<bool> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT CASE WHEN email_mfa THEN 1 ELSE 0 END FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    Ok(matches!(row, Some((n,)) if n != 0))
}

pub(super) async fn create_session(
    State(state): State<AppState>,
    Body(req): Body<CreateSessionRequest>,
) -> ApiResult<Json<Value>> {
    let srp_user_id = parse_uuid(&req.srp_user_id)?;
    let Some((user_id, verifier)) = srp_auth_by_srp_user(&state, &srp_user_id).await? else {
        // Museum `fCreateSession`: a fake session so unknown ids look like real ones.
        let a = decode_b64(&req.srp_a)?;
        if a.len() != srp_math::N_BYTES {
            return Err(users::bad_request("Invalid length for srpA"));
        }
        let id = users::new_uuid();
        sqlx::query("INSERT INTO srp_sessions (id, srp_user_id, server_key, srp_a, is_fake, created_at, updated_at) VALUES ($1, $2, $3, $4, TRUE, $5, $5)")
            .bind(&id)
            .bind(&srp_user_id)
            .bind(b64(&random_bytes(64)))
            .bind(&req.srp_a)
            .bind(now_micros())
            .execute(&state.db.pool)
            .await?;
        return Ok(Json(
            json!({ "sessionID": id, "srpB": b64(&random_bytes(srp_math::N_BYTES)) }),
        ));
    };
    if email_mfa_on(&state, user_id).await? {
        return Err(users::email_mfa_enabled());
    }
    let (b, id) = create_session_row(&state, &srp_user_id, &verifier, &req.srp_a).await?;
    Ok(Json(json!({ "sessionID": id, "srpB": b })))
}

#[derive(Deserialize)]
pub(super) struct VerifySessionRequest {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "srpUserID")]
    srp_user_id: String,
    #[serde(rename = "srpM1", default)]
    srp_m1: String,
}

pub(super) async fn verify_session(
    State(state): State<AppState>,
    client: ClientInfo,
    Body(req): Body<VerifySessionRequest>,
) -> ApiResult<Json<login::EmailAuthorizationResponse>> {
    let session_id = parse_uuid(&req.session_id)?;
    let srp_user_id = parse_uuid(&req.srp_user_id)?;
    let auth = srp_auth_by_srp_user(&state, &srp_user_id).await?;
    let m2 = verify_session_row(
        &state,
        &srp_user_id,
        auth.as_ref().map(|a| a.1.as_str()),
        &session_id,
        &req.srp_m1,
    )
    .await?;
    let Some((user_id, _)) = auth else {
        return Err(users::unauthorized_plain());
    };
    let user = users::get_user(&state, user_id).await?;
    if user.email_mfa {
        return Err(users::email_mfa_enabled());
    }
    let mut resp = login::on_verification_success(&state, &client, &user.email, None, true).await?;
    resp.srp_m2 = Some(m2);
    Ok(Json(resp))
}
