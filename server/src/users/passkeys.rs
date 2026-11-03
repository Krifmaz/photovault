//! Passkey storage and ceremonies (museum `pkg/repo/passkey`), on top of [`super::webauthn`].

use super::webauthn::{self, CredentialJson, encode_b64url};
use super::*;
use crate::db::now_micros;
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};

/// Museum `ente.MaxPasskeys`.
pub const MAX_PASSKEYS: i64 = 10;
const LOGIN_TIMEOUT_MS: i64 = 2 * 60 * 1000;
const REGISTRATION_TIMEOUT_MS: i64 = 5 * 60 * 1000;

/// Museum `ente.Passkey`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passkey {
    /// Id (UUID).
    pub id: String,
    /// Owner.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// Name.
    pub friendly_name: String,
    /// Creation (µs).
    pub created_at: i64,
}

/// Active passkeys of a user.
pub async fn list(state: &AppState, user_id: i64) -> ApiResult<Vec<Passkey>> {
    let rows: Vec<(String, i64, String, i64)> = sqlx::query_as(
        "SELECT id, user_id, friendly_name, created_at FROM passkeys WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(&state.db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, user_id, friendly_name, created_at)| Passkey {
            id,
            user_id,
            friendly_name,
            created_at,
        })
        .collect())
}

/// Number of active passkeys.
pub async fn count(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM passkeys WHERE user_id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_one(&state.db.pool)
            .await?;
    Ok(n)
}

/// `(credential id b64url, COSE key b64, sign count)` of the user's active credentials.
async fn credentials(state: &AppState, user_id: i64) -> ApiResult<Vec<(String, String, String)>> {
    Ok(sqlx::query_as(
        "SELECT c.credential_id, c.public_key, c.authenticator FROM passkey_credentials c JOIN passkeys p ON p.id = c.passkey_id \
         WHERE p.user_id = $1 AND p.deleted_at IS NULL ORDER BY c.created_at",
    )
    .bind(user_id)
    .fetch_all(&state.db.pool)
    .await?)
}

async fn save_session(
    state: &AppState,
    user_id: i64,
    challenge: &str,
    allowed: &[String],
    timeout_ms: i64,
) -> ApiResult<String> {
    let id = new_uuid();
    let now = now_micros();
    sqlx::query("INSERT INTO webauthn_sessions (id, challenge, user_id, rp_id, allowed_credential_ids, expires_at, user_verification_requirement, extensions, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(&id)
        .bind(challenge)
        .bind(user_id)
        .bind(&state.config.accounts.webauthn_rp_id)
        .bind(serde_json::to_string(allowed).map_err(ApiError::internal)?)
        .bind(now + timeout_ms * 1000)
        .bind("preferred")
        .bind("{}")
        .bind(now)
        .execute(&state.db.pool)
        .await?;
    Ok(id)
}

struct Session {
    challenge: String,
    user_id: i64,
    rp_id: String,
    expires_at: i64,
}

async fn load_session(state: &AppState, id: &str) -> ApiResult<Session> {
    let row: Option<(String, i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT challenge, user_id, rp_id, expires_at FROM webauthn_sessions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db.pool)
    .await?;
    let (challenge, user_id, rp_id, expires_at) = row.ok_or_else(not_found_plain)?;
    Ok(Session {
        challenge,
        user_id,
        rp_id: rp_id.unwrap_or_else(|| state.config.accounts.webauthn_rp_id.clone()),
        expires_at,
    })
}

/// `POST /passkeys/registration/begin`: `(options, sessionID)`.
pub async fn begin_registration(state: &AppState, user: &User) -> ApiResult<(Value, String)> {
    if count(state, user.id).await? >= MAX_PASSKEYS {
        return Err(api(
            StatusCode::CONFLICT,
            "MAX_PASSKEYS_REACHED",
            "Max passkeys limit reached",
        ));
    }
    let challenge = encode_b64url(&crate::crypto::random_bytes(32));
    let existing: Vec<Value> = credentials(state, user.id)
        .await?
        .into_iter()
        .map(|c| json!({"type": "public-key", "id": c.0}))
        .collect();
    let s = &state.config.accounts;
    let options = json!({
        "publicKey": {
            "rp": {"name": s.webauthn_rp_name, "id": s.webauthn_rp_id},
            "user": {
                "name": user.email,
                "displayName": user.email,
                "id": encode_b64url(&user.id.to_be_bytes()),
            },
            "challenge": challenge,
            "pubKeyCredParams": webauthn::pub_key_cred_params(),
            "timeout": REGISTRATION_TIMEOUT_MS,
            "excludeCredentials": existing,
            "authenticatorSelection": {"userVerification": "preferred"},
            "attestation": "none",
        }
    });
    let id = save_session(state, user.id, &challenge, &[], REGISTRATION_TIMEOUT_MS).await?;
    Ok((options, id))
}

/// `POST /passkeys/registration/finish`.
pub async fn finish_registration(
    state: &AppState,
    user: &User,
    session_id: &str,
    friendly_name: &str,
    cred: &CredentialJson,
) -> ApiResult<()> {
    let s = load_session(state, session_id).await?;
    if s.user_id != user.id {
        return Err(ApiError::internal("session does not belong to user"));
    }
    if s.expires_at < now_micros() {
        return Err(ApiError::internal("session expired"));
    }
    let nc = webauthn::verify_registration(
        cred,
        &s.challenge,
        &s.rp_id,
        &state.config.accounts.webauthn_rp_origins,
    )
    .map_err(|e| bad_request(format!("Error parsing attestation response: {e}")))?;
    let passkey_id = new_uuid();
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    sqlx::query(
        "INSERT INTO passkeys (id, user_id, friendly_name, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&passkey_id)
    .bind(user.id)
    .bind(friendly_name)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO passkey_credentials (passkey_id, credential_id, public_key, attestation_type, authenticator_transports, credential_flags, authenticator, rp_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(&passkey_id)
        .bind(encode_b64url(&nc.id))
        .bind(crate::crypto::b64(&nc.cose_key))
        .bind(&nc.fmt)
        .bind(serde_json::to_string(&nc.transports).map_err(ApiError::internal)?)
        .bind(nc.flags.to_string())
        .bind(json!({"signCount": nc.sign_count}).to_string())
        .bind(&s.rp_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM webauthn_sessions WHERE id = $1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `POST /users/two-factor/passkeys/begin`: `(options, ceremonySessionID)`.
pub async fn begin_authentication(state: &AppState, user_id: i64) -> ApiResult<(Value, String)> {
    let creds = credentials(state, user_id).await?;
    if creds.is_empty() {
        return Err(bad_request("No passkey found for user"));
    }
    let challenge = encode_b64url(&crate::crypto::random_bytes(32));
    let ids: Vec<String> = creds.iter().map(|c| c.0.clone()).collect();
    let options = json!({
        "publicKey": {
            "challenge": challenge,
            "timeout": LOGIN_TIMEOUT_MS,
            "rpId": state.config.accounts.webauthn_rp_id,
            "allowCredentials": ids.iter().map(|i| json!({"type": "public-key", "id": i})).collect::<Vec<_>>(),
            "userVerification": "preferred",
        }
    });
    let id = save_session(state, user_id, &challenge, &ids, LOGIN_TIMEOUT_MS).await?;
    Ok((options, id))
}

/// `POST /users/two-factor/passkeys/finish`: verify the assertion.
pub async fn finish_authentication(
    state: &AppState,
    user_id: i64,
    ceremony_id: &str,
    cred: &CredentialJson,
) -> ApiResult<()> {
    let s = load_session(state, ceremony_id).await?;
    if s.user_id != user_id {
        return Err(ApiError::internal("session does not belong to user"));
    }
    if s.expires_at < now_micros() {
        return Err(api(StatusCode::GONE, "SESSION_EXPIRED", "Session expired"));
    }
    let invalid = || api(StatusCode::UNAUTHORIZED, "BAD_REQUEST", "Invalid signature");
    let wanted = webauthn::decode_b64url(if cred.raw_id.is_empty() {
        &cred.id
    } else {
        &cred.raw_id
    })
    .map_err(|_| invalid())?;
    let creds = credentials(state, user_id).await?;
    let Some((cid, key, auth)) = creds
        .into_iter()
        .find(|c| webauthn::decode_b64url(&c.0).ok().as_deref() == Some(wanted.as_slice()))
    else {
        return Err(invalid());
    };
    let key = crate::crypto::unb64(&key).map_err(ApiError::internal)?;
    let stored: u32 = serde_json::from_str::<Value>(&auth)
        .ok()
        .and_then(|v| v.get("signCount").and_then(Value::as_u64))
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let new_count = webauthn::verify_assertion(
        cred,
        &s.challenge,
        &s.rp_id,
        &state.config.accounts.webauthn_rp_origins,
        &key,
        stored,
    )
    .map_err(|e| {
        tracing::warn!(error = %e, "could not finish passkey authentication");
        invalid()
    })?;
    sqlx::query("UPDATE passkey_credentials SET authenticator = $1 WHERE credential_id = $2")
        .bind(json!({"signCount": new_count}).to_string())
        .bind(&cid)
        .execute(&state.db.pool)
        .await?;
    sqlx::query("DELETE FROM webauthn_sessions WHERE id = $1")
        .bind(ceremony_id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Soft-delete one passkey (museum `DeletePasskey`).
pub async fn delete(state: &AppState, user_id: i64, passkey_id: &str) -> ApiResult<()> {
    sqlx::query("UPDATE passkeys SET friendly_name = $1, deleted_at = $2 WHERE id = $3 AND user_id = $4 AND deleted_at IS NULL")
        .bind(passkey_id)
        .bind(now_micros())
        .bind(passkey_id)
        .bind(user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Soft-delete every passkey of a user (admin `disable-passkeys`, password recovery).
pub async fn delete_all(state: &AppState, user_id: i64) -> ApiResult<()> {
    sqlx::query("UPDATE passkeys SET friendly_name = id, deleted_at = $1 WHERE user_id = $2 AND deleted_at IS NULL")
        .bind(now_micros())
        .bind(user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Rename a passkey (museum `RenamePasskey`).
pub async fn rename(state: &AppState, user_id: i64, passkey_id: &str, name: &str) -> ApiResult<()> {
    sqlx::query("UPDATE passkeys SET friendly_name = $1 WHERE id = $2 AND user_id = $3 AND deleted_at IS NULL")
        .bind(name)
        .bind(passkey_id)
        .bind(user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}
