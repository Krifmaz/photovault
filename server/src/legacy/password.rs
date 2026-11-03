//! Password reset on behalf of a recovered account (trusted contact or legacy
//! kit): museum's `UserController.SetupSRP` and `RecoverSrpAndKeyAttributes`,
//! implemented with direct SQL against museum's tables:
//! `srp_sessions`, `temp_srp_setup`, `srp_auth`, `key_attributes`, `tokens`,
//! and, when present, `users.is_two_factor_enabled`, `passkeys`,
//! `two_factor_sessions`, `passkey_login_sessions`.

use crate::db::now_micros;
use crate::entities::ids::{new_uuid, parse_uuid};
use crate::entities::schema::{begin_write, has_column, has_table, lock_user};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

/// museum's `ente.SetupSRPRequest`.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupSrpRequest {
    /// SRP identity (UUID).
    #[serde(rename = "srpUserID", default)]
    pub srp_user_id: String,
    /// Salt (base64).
    #[serde(rename = "srpSalt", default)]
    pub srp_salt: String,
    /// Verifier (base64).
    #[serde(rename = "srpVerifier", default)]
    pub srp_verifier: String,
    /// Client public value A (base64, 512 bytes).
    #[serde(rename = "srpA", default)]
    pub srp_a: String,
}

/// museum's `ente.SetupSRPResponse`.
#[derive(Debug, Clone, Serialize)]
pub struct SetupSrpResponse {
    /// Id of the pending setup.
    #[serde(rename = "setupID")]
    pub setup_id: String,
    /// Server public value B (base64).
    #[serde(rename = "srpB")]
    pub srp_b: String,
}

/// museum's `ente.UpdateKeysRequest`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKeysRequest {
    /// New KEK salt.
    #[serde(default)]
    pub kek_salt: String,
    /// Master key encrypted with the new KEK.
    #[serde(default)]
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    #[serde(default)]
    pub key_decryption_nonce: String,
    /// Argon2 memory limit.
    #[serde(default)]
    pub mem_limit: i64,
    /// Argon2 ops limit.
    #[serde(default)]
    pub ops_limit: i64,
}

/// museum's `ente.UpdateSRPAndKeysRequest`.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSrpAndKeysRequest {
    /// Setup id from [`SetupSrpResponse`].
    #[serde(rename = "setupID", default)]
    pub setup_id: String,
    /// Client proof (base64, 32 bytes).
    #[serde(rename = "srpM1", default)]
    pub srp_m1: String,
    /// New key attributes.
    #[serde(rename = "updatedKeyAttr", default)]
    pub updated_key_attr: Option<UpdateKeysRequest>,
    /// Whether to log out other sessions (default true for recovery).
    #[serde(rename = "logOutOtherDevices", default)]
    pub log_out_other_devices: Option<bool>,
}

/// museum's `ente.UpdateSRPSetupResponse`.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateSrpSetupResponse {
    /// Echoed setup id.
    #[serde(rename = "setupID")]
    pub setup_id: String,
    /// Server proof (base64).
    #[serde(rename = "srpM2")]
    pub srp_m2: String,
}

const EXPECTED_KDF_STRENGTH: i64 = 1_073_741_824 * 4;
const MAX_UNVERIFIED_SESSIONS_PER_HOUR: i64 = 10;
const MAX_VERIFY_ATTEMPTS: i64 = 5;

fn bad(msg: &str) -> ApiError {
    ApiError::bad_request(msg.to_string())
}

/// museum's `ente.ValidateBase64Length`.
pub fn validate_b64_len(value: &str, size: usize) -> ApiResult<()> {
    if value.len() != size.div_ceil(3) * 4 {
        return Err(bad("invalid length"));
    }
    let d = crate::crypto::unb64(value).map_err(|_| bad("invalid base64"))?;
    if d.len() != size {
        return Err(bad("invalid length"));
    }
    Ok(())
}

impl UpdateKeysRequest {
    /// museum's `UpdateKeysRequest.Validate`.
    pub fn validate(&self) -> ApiResult<()> {
        if self.mem_limit.checked_mul(self.ops_limit) != Some(EXPECTED_KDF_STRENGTH) {
            return Err(bad("Unexpected KDF strength"));
        }
        if self.mem_limit < 128 * 1024 * 1024 {
            return Err(bad("memory limit must be at least 128MB"));
        }
        validate_b64_len(&self.kek_salt, 16)?;
        validate_b64_len(&self.encrypted_key, 48)?;
        validate_b64_len(&self.key_decryption_nonce, 24)
    }
}

/// museum's `SetupSRP`: start an SRP session against the new verifier and stage it.
pub async fn setup_srp(
    state: &AppState,
    user_id: i64,
    req: &SetupSrpRequest,
) -> ApiResult<SetupSrpResponse> {
    let srp_user_id = parse_uuid(&req.srp_user_id).ok_or_else(|| bad("invalid srpUserID"))?;
    if req.srp_salt.is_empty() || req.srp_verifier.is_empty() || req.srp_a.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let a = crate::crypto::unb64(&req.srp_a).map_err(|_| bad("invalid srpA"))?;
    if a.len() != crate::legacy::srp::N_LEN {
        return Err(bad("Invalid length for srpA"));
    }
    let verifier = crate::crypto::unb64(&req.srp_verifier).map_err(|_| bad("invalid verifier"))?;
    let now = now_micros();
    let (unverified,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM srp_sessions WHERE srp_user_id = $1 AND has_verified = FALSE AND created_at > $2",
    )
    .bind(&srp_user_id)
    .bind(now - 3_600_000_000)
    .fetch_one(&state.db.pool)
    .await?;
    if unverified >= MAX_UNVERIFIED_SESSIONS_PER_HOUR {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_UNVERIFIED_SESSIONS",
            "",
        ));
    }
    let secret = crate::crypto::random_bytes(32);
    let b = crate::legacy::srp::compute_b(&verifier, &secret);
    let session_id = new_uuid();
    sqlx::query(
        "INSERT INTO srp_sessions(id, srp_user_id, server_key, srp_a, has_verified, attempt_count, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, 0, $5, $5)",
    )
    .bind(&session_id)
    .bind(&srp_user_id)
    .bind(crate::crypto::b64(&secret))
    .bind(&req.srp_a)
    .bind(now)
    .execute(&state.db.pool)
    .await?;
    let setup_id = new_uuid();
    sqlx::query(
        "INSERT INTO temp_srp_setup(id, session_id, user_id, srp_user_id, salt, verifier, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&setup_id)
    .bind(&session_id)
    .bind(user_id)
    .bind(&srp_user_id)
    .bind(&req.srp_salt)
    .bind(&req.srp_verifier)
    .bind(now)
    .execute(&state.db.pool)
    .await?;
    Ok(SetupSrpResponse {
        setup_id,
        srp_b: crate::crypto::b64(&b),
    })
}

fn invalid_password() -> ApiError {
    ApiError::status(StatusCode::UNAUTHORIZED)
}

/// museum's `verifySRPSession`: check `M1` against a stored session; returns `M2` (base64).
async fn verify_session(
    state: &AppState,
    srp_user_id: &str,
    verifier_b64: &str,
    session_id: &str,
    m1_b64: &str,
) -> ApiResult<String> {
    let m1 = crate::crypto::unb64(m1_b64).map_err(|_| bad("invalid srpM1"))?;
    if m1.len() != 32 {
        return Err(bad(&format!("srpM1 size is {}, expected 32", m1.len())));
    }
    let reserved: Option<(String, String, String)> = sqlx::query_as(
        "UPDATE srp_sessions SET attempt_count = attempt_count + 1, updated_at = $1 \
         WHERE id = $2 AND has_verified = FALSE AND attempt_count < $3 \
         RETURNING srp_user_id, server_key, srp_a",
    )
    .bind(now_micros())
    .bind(session_id)
    .bind(MAX_VERIFY_ATTEMPTS)
    .fetch_optional(&state.db.pool)
    .await?;
    let Some((session_srp_user, server_key, srp_a)) = reserved else {
        let row: Option<(bool,)> =
            sqlx::query_as("SELECT has_verified FROM srp_sessions WHERE id = $1")
                .bind(session_id)
                .fetch_optional(&state.db.pool)
                .await?;
        return Err(match row {
            None => invalid_password(),
            Some((true,)) => ApiError::new(StatusCode::GONE, "SESSION_ALREADY_VERIFIED", ""),
            Some((false,)) => ApiError::new(StatusCode::GONE, "TOO_MANY_WRONG_ATTEMPTS", ""),
        });
    };
    if parse_uuid(&session_srp_user).as_deref() != Some(srp_user_id) {
        return Err(invalid_password());
    }
    let verifier = crate::crypto::unb64(verifier_b64).map_err(ApiError::internal)?;
    let secret = crate::crypto::unb64(&server_key).map_err(ApiError::internal)?;
    let a = crate::crypto::unb64(&srp_a).map_err(ApiError::internal)?;
    let m2 = crate::legacy::srp::verify_m1(&verifier, &secret, &a, &m1).ok_or_else(invalid_password)?;
    let claimed = sqlx::query(
        "UPDATE srp_sessions SET has_verified = TRUE, updated_at = $1 WHERE id = $2 AND has_verified = FALSE",
    )
    .bind(now_micros())
    .bind(session_id)
    .execute(&state.db.pool)
    .await?;
    if claimed.rows_affected() != 1 {
        return Err(ApiError::new(StatusCode::GONE, "SESSION_ALREADY_VERIFIED", ""));
    }
    Ok(crate::crypto::b64(&m2))
}

/// What authorises a recovery password reset; checked inside the update transaction.
#[derive(Debug, Clone)]
pub enum RecoveryAuthorization {
    /// A trusted-contact recovery session.
    Emergency {
        /// `emergency_recovery.id`.
        session_id: String,
        /// Account being recovered.
        user_id: i64,
        /// The trusted contact.
        emergency_contact_id: i64,
    },
    /// A legacy-kit recovery session.
    Kit {
        /// `legacy_kit_recovery_session.id`.
        session_id: String,
        /// `legacy_kit.id`.
        kit_id: String,
        /// Account being recovered.
        user_id: i64,
        /// Raw session token presented by the recovering browser.
        session_token: String,
    },
}

/// museum's `RecoverSrpAndKeyAttributes`: verify the SRP proof for the staged
/// setup, then in one transaction re-check the recovery authorisation, disable
/// second factors, replace `srp_auth` and the KEK-wrapped master key, and
/// (when `log_out_all`) revoke every session of the account.
pub async fn recover_srp_and_key_attributes(
    state: &AppState,
    user_id: i64,
    req: &UpdateSrpAndKeysRequest,
    log_out_all: bool,
    authorize: RecoveryAuthorization,
) -> ApiResult<UpdateSrpSetupResponse> {
    let keys = req
        .updated_key_attr
        .as_ref()
        .ok_or_else(|| bad("updatedKeyAttr is required"))?;
    keys.validate()?;
    let setup_id = parse_uuid(&req.setup_id).ok_or(ApiError::status(StatusCode::BAD_REQUEST))?;
    if req.srp_m1.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let setup: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT session_id, srp_user_id, salt, verifier FROM temp_srp_setup WHERE id = $1 AND user_id = $2",
    )
    .bind(&setup_id)
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (session_id, srp_user_id, salt, verifier) =
        setup.ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    let srp_user_id = parse_uuid(&srp_user_id).unwrap_or(srp_user_id);
    let m2 = verify_session(state, &srp_user_id, &verifier, &session_id, &req.srp_m1).await?;

    let db = &state.db;
    let has_2fa_col = has_column(db, "users", "is_two_factor_enabled").await;
    let has_passkeys = has_table(db, "passkeys").await;
    let has_2fa_sessions = has_table(db, "two_factor_sessions").await;
    let has_passkey_sessions = has_table(db, "passkey_login_sessions").await;

    let now = now_micros();
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    match &authorize {
        RecoveryAuthorization::Emergency {
            session_id,
            user_id,
            emergency_contact_id,
        } => {
            crate::legacy::emergency::complete_recovery_tx(
                &mut tx,
                db.engine,
                session_id,
                *user_id,
                *emergency_contact_id,
            )
            .await?
        }
        RecoveryAuthorization::Kit {
            session_id,
            kit_id,
            user_id,
            session_token,
        } => {
            crate::legacy::kit::complete_recovery_tx(
                &mut tx,
                db.engine,
                session_id,
                kit_id,
                *user_id,
                session_token,
            )
            .await?
        }
    }
    // Recovery replaces the password, so second factors the owner may have lost go too.
    if has_2fa_col {
        sqlx::query("UPDATE users SET is_two_factor_enabled = FALSE WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    if has_passkeys {
        sqlx::query(
            "UPDATE passkeys SET friendly_name = CAST(id AS TEXT), deleted_at = $1 WHERE user_id = $2 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO srp_auth(user_id, srp_user_id, salt, verifier, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5) \
         ON CONFLICT (user_id) DO UPDATE SET srp_user_id = EXCLUDED.srp_user_id, salt = EXCLUDED.salt, \
         verifier = EXCLUDED.verifier, updated_at = EXCLUDED.updated_at",
    )
    .bind(user_id)
    .bind(&srp_user_id)
    .bind(&salt)
    .bind(&verifier)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE key_attributes SET kek_salt = $1, encrypted_key = $2, key_decryption_nonce = $3, mem_limit = $4, ops_limit = $5 WHERE user_id = $6",
    )
    .bind(&keys.kek_salt)
    .bind(&keys.encrypted_key)
    .bind(&keys.key_decryption_nonce)
    .bind(keys.mem_limit)
    .bind(keys.ops_limit)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if has_2fa_sessions {
        sqlx::query("DELETE FROM two_factor_sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    if has_passkey_sessions {
        let sql = if log_out_all {
            "DELETE FROM passkey_login_sessions WHERE user_id = $1"
        } else {
            "DELETE FROM passkey_login_sessions WHERE user_id = $1 AND verified_at IS NULL"
        };
        sqlx::query(sql).bind(user_id).execute(&mut *tx).await?;
    }
    if log_out_all {
        sqlx::query("UPDATE tokens SET is_deleted = TRUE WHERE user_id = $1 AND is_deleted = FALSE")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(UpdateSrpSetupResponse {
        setup_id,
        srp_m2: m2,
    })
}
