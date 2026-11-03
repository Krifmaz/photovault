//! Cast sessions in the `casting` table (museum `pkg/repo/cast`), plus the
//! `X-Cast-Access-Token` extractor (museum `CastAuthMiddleware`).

use super::http::{header_or_query, json_response, random_string};
use crate::auth::App;
use crate::library::collections;
use crate::db::now_micros;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use serde::Serialize;
use sqlx::AnyPool;

/// Unclaimed pairing codes are dropped after an hour (museum cron `DeleteUnclaimedCodes`).
const UNCLAIMED_CODE_TTL: i64 = 60 * 60 * 1_000_000;
/// Sessions unused for a week are dropped (museum cron `DeleteOldSessions`).
const OLD_SESSION_TTL: i64 = 7 * 24 * 60 * 60 * 1_000_000;

/// museum `random.GenerateAlphaNumString(6)`: first char a letter, no `O`/`0`.
pub fn new_device_code() -> String {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNPQRSTUVWXYZ";
    const ALNUM: &[u8] = b"ABCDEFGHIJKLMNPQRSTUVWXYZ123456789";
    let mut s = random_string(LETTERS, 1);
    s.push_str(&random_string(ALNUM, 5));
    s
}

/// Lazily apply museum's periodic cleanups.
pub async fn cleanup(db: &AnyPool) -> sqlx::Result<()> {
    let now = now_micros();
    sqlx::query(
        "DELETE FROM casting WHERE last_used_at < $1 AND is_deleted = FALSE AND collection_id IS NULL",
    )
    .bind(now - UNCLAIMED_CODE_TTL)
    .execute(db)
    .await?;
    sqlx::query("DELETE FROM casting WHERE last_used_at < $1")
        .bind(now - OLD_SESSION_TTL)
        .execute(db)
        .await?;
    Ok(())
}

/// Register a TV/browser waiting to be paired; returns its code (museum `AddCode`).
pub async fn add_code(
    db: &AnyPool,
    public_key: &str,
    pq_public_key: Option<&str>,
    ip: &str,
    device_name: &str,
) -> sqlx::Result<String> {
    let now = now_micros();
    let mut last_err = None;
    // The partial unique index on live codes can collide; retry with a fresh code.
    for _ in 0..5 {
        let code = new_device_code();
        let r = sqlx::query(
            "INSERT INTO casting (id, code, public_key, pq_public_key, ip, device_name, is_deleted, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(super::http::new_uuid())
        .bind(&code)
        .bind(public_key)
        .bind(pq_public_key)
        .bind(ip)
        .bind(device_name)
        .bind(false)
        .bind(now)
        .bind(now)
        .execute(db)
        .await;
        match r {
            Ok(_) => return Ok(code),
            Err(sqlx::Error::Database(d)) if d.is_unique_violation() => {
                last_err = Some(sqlx::Error::Database(d));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or(sqlx::Error::RowNotFound))
}

/// museum `cast.CastInfo`.
#[derive(Debug, Serialize)]
pub struct CastInfo {
    /// Session id.
    #[serde(rename = "deviceID")]
    pub device_id: String,
    /// Collection being cast.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Device IP at registration.
    #[serde(rename = "deviceIP")]
    pub device_ip: String,
    /// Last use, microseconds.
    #[serde(rename = "lastUsedAt")]
    pub last_used_at: i64,
    /// Device type or IP.
    #[serde(rename = "deviceName")]
    pub device_name: Option<String>,
}

type DeviceRow = (String, Option<i64>, String, i64, Option<String>);

/// The user's live cast sessions, newest first.
pub async fn devices_for_user(db: &AnyPool, user_id: i64) -> sqlx::Result<Vec<CastInfo>> {
    let rows: Vec<DeviceRow> = sqlx::query_as(
        "SELECT id, collection_id, ip, last_used_at, device_name FROM casting \
         WHERE cast_user = $1 AND is_deleted = FALSE ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(device_id, cid, device_ip, last_used_at, device_name)| CastInfo {
                device_id,
                collection_id: cid.unwrap_or(0),
                device_ip,
                last_used_at,
                device_name,
            },
        )
        .collect())
}

/// `(public_key, pq_public_key, ip)` for a live code.
pub async fn device_info(
    db: &AnyPool,
    code: &str,
) -> sqlx::Result<Option<(String, Option<String>, String)>> {
    sqlx::query_as(
        "SELECT public_key, pq_public_key, ip FROM casting WHERE code = $1 AND is_deleted = FALSE",
    )
    .bind(code.to_uppercase())
    .fetch_optional(db)
    .await
}

/// `Some(payload)` for a live code (`Some(None)` while unpaired), `None` when unknown.
pub async fn enc_cast_data(db: &AnyPool, code: &str) -> sqlx::Result<Option<Option<String>>> {
    let r: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT encrypted_payload FROM casting WHERE code = $1 AND is_deleted = FALSE",
    )
    .bind(code.to_uppercase())
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| r.0))
}

/// Pair an unclaimed code with a collection; returns the session id, or `None` when the
/// code is unknown or already paired (museum `InsertCastData`).
pub async fn insert_cast_data(
    db: &AnyPool,
    user_id: i64,
    code: &str,
    collection_id: i64,
    cast_token: &str,
    payload: &str,
) -> sqlx::Result<Option<String>> {
    let r: Option<(String,)> = sqlx::query_as(
        "UPDATE casting SET collection_id = $1, cast_user = $2, token = $3, encrypted_payload = $4, last_used_at = $5 \
         WHERE code = $6 AND is_deleted = FALSE AND cast_user IS NULL RETURNING id",
    )
    .bind(collection_id)
    .bind(user_id)
    .bind(cast_token)
    .bind(payload)
    .bind(now_micros())
    .bind(code.to_uppercase())
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| r.0))
}

/// Revoke all of a user's sessions.
pub async fn revoke_for_user(db: &AnyPool, user_id: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE casting SET is_deleted = TRUE WHERE cast_user = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Revoke one of a user's sessions.
pub async fn revoke_device(db: &AnyPool, user_id: i64, device_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE casting SET is_deleted = TRUE WHERE id = $1 AND cast_user = $2")
        .bind(device_id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Revoke every session casting a collection (museum `RevokeTokenForCollection`; for
/// the collections workstream to call when a collection is deleted or unshared).
pub async fn revoke_for_collection(db: &AnyPool, collection_id: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE casting SET is_deleted = TRUE WHERE collection_id = $1")
        .bind(collection_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Revoke a user's sessions for one collection (museum `RevokeForGivenUserAndCollection`;
/// for the collections workstream to call when a sharee leaves).
pub async fn revoke_for_user_and_collection(
    db: &AnyPool,
    collection_id: i64,
    user_id: i64,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE casting SET is_deleted = TRUE WHERE collection_id = $1 AND cast_user = $2")
        .bind(collection_id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Rough device type from a user agent, standing in for museum's device detector
/// (`Tv`, `Smartphone`, `Tablet`, `Desktop`); `None` when unknown.
pub fn device_type(ua: &str) -> Option<&'static str> {
    let u = ua.to_ascii_lowercase();
    if u.is_empty() {
        return None;
    }
    if [
        "smart-tv",
        "smarttv",
        "googletv",
        "android tv",
        "appletv",
        "crkey",
        "tizen",
        "webos",
        "hbbtv",
        "bravia",
        "aftb",
        "aftm",
        "afts",
        "roku",
    ]
    .iter()
    .any(|k| u.contains(k))
    {
        return Some("Tv");
    }
    if u.contains("ipad") || u.contains("tablet") {
        return Some("Tablet");
    }
    if u.contains("mobile") || u.contains("iphone") || u.contains("android") {
        return Some("Smartphone");
    }
    if u.contains("windows") || u.contains("macintosh") || u.contains("x11") || u.contains("linux")
    {
        return Some("Desktop");
    }
    None
}

/// A request authenticated with a cast token (museum `cast.AuthContext`).
#[derive(Debug, Clone)]
pub struct CastSession {
    /// Collection being cast.
    pub collection_id: i64,
    /// User who started the cast.
    pub user_id: i64,
}

impl FromRequestParts<AppState> for CastSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token =
            header_or_query(parts, "X-Cast-Access-Token", "castToken").ok_or_else(|| {
                json_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({"error": "cast access token missing"}),
                )
            })?;
        let invalid = || {
            json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"error": "invalid token"}),
            )
        };
        let db = &state.db.pool;
        let row: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT collection_id, cast_user FROM casting WHERE token = $1 AND is_deleted = FALSE",
        )
        .bind(&token)
        .fetch_optional(db)
        .await
        .map_err(|_| invalid())?;
        let Some((Some(collection_id), Some(user_id))) = row else {
            return Err(invalid());
        };
        // museum verifies the caster still has access (`AccessCtrl.GetCollection`).
        if collections::access(state, collection_id, user_id, App::from_parts(parts), false, false)
            .await
            .is_err()
        {
            return Err(invalid());
        }
        let db = db.clone();
        tokio::spawn(async move {
            let _ = sqlx::query("UPDATE casting SET last_used_at = $1 WHERE token = $2")
                .bind(now_micros())
                .bind(token)
                .execute(&db)
                .await;
        });
        Ok(CastSession {
            collection_id,
            user_id,
        })
    }
}
