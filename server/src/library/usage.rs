//! Storage usage and quota (museum's `UsageController.CanUploadFile`, simplified).
//!
//! * Quota is `users.storage_quota` (no subscriptions, bonuses or families).
//! * Usage is `usage.storage_consumed`, kept in step with `object_keys` by the
//!   library's writes (file create/update, permanent delete).
//! * Locker uploads are limited by museum's paid Locker tier (1000 files, 10 GiB)
//!   and not by the Photos quota.

use super::errors;
use super::sql::{db_err, get_i64};
use crate::auth::App;
use crate::error::ApiResult;
use crate::state::AppState;
use sqlx::AnyConnection;

/// museum's `StorageOverflowAboveSubscriptionLimit` (50 MiB).
pub const OVERFLOW: i64 = 50 * 1024 * 1024;
/// Locker file limit.
pub const LOCKER_FILE_LIMIT: i64 = 1000;
/// Locker storage limit.
pub const LOCKER_STORAGE_LIMIT: i64 = 10 * 1024 * 1024 * 1024;

/// Bytes currently attributed to `user_id`.
pub async fn usage(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let row = sqlx::query("SELECT storage_consumed FROM usage WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?;
    match row {
        Some(r) => get_i64(&r, "storage_consumed").map_err(db_err),
        None => Ok(0),
    }
}

/// Add `delta` (may be negative) to the user's usage.
pub async fn adjust(conn: &mut AnyConnection, user_id: i64, delta: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO usage (user_id, storage_consumed) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET storage_consumed = usage.storage_consumed + $2",
    )
    .bind(user_id)
    .bind(delta)
    .execute(conn)
    .await?;
    Ok(())
}

/// The user's quota in bytes (`users.storage_quota`).
pub async fn quota(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let row = sqlx::query("SELECT storage_quota FROM users WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?;
    match row {
        Some(r) => get_i64(&r, "storage_quota").map_err(db_err),
        None => Err(errors::not_found()),
    }
}

/// Locker usage: (live Locker file count, their bytes).
pub async fn locker_usage(state: &AppState, user_id: i64) -> ApiResult<(i64, i64)> {
    let row = sqlx::query(
        "SELECT COUNT(DISTINCT f.file_id) AS n, CAST(COALESCE(SUM(ok.size), 0) AS BIGINT) AS total
         FROM files f
         JOIN object_keys ok ON ok.file_id = f.file_id AND ok.is_deleted = FALSE
         WHERE f.owner_id = $1 AND f.app = 'locker'
           AND EXISTS (SELECT 1 FROM collection_files cf WHERE cf.file_id = f.file_id AND cf.is_deleted = FALSE)",
    )
    .bind(user_id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok((
        get_i64(&row, "n").map_err(db_err)?,
        get_i64(&row, "total").map_err(db_err)?,
    ))
}

/// `CanUploadFile`: `ErrStorageLimitExceeded` (426) when the upload would not fit.
/// `size` is the extra bytes (or `None` for a URL request without a known size).
pub async fn can_upload(
    state: &AppState,
    user_id: i64,
    size: Option<i64>,
    app: App,
) -> ApiResult<()> {
    if app == App::Locker {
        let (count, total) = locker_usage(state, user_id).await?;
        if count >= LOCKER_FILE_LIMIT {
            return Err(errors::file_limit_reached());
        }
        if total + size.unwrap_or(0) >= LOCKER_STORAGE_LIMIT {
            return Err(errors::storage_limit());
        }
        return Ok(());
    }
    let mut limit = quota(state, user_id).await?;
    let mut new_usage = usage(state, user_id).await?;
    if let Some(s) = size {
        new_usage += s;
        limit += OVERFLOW;
    }
    if new_usage > limit {
        // Locker bytes do not count against the Photos quota.
        let (_, locker) = locker_usage(state, user_id).await?;
        if new_usage - locker > limit {
            return Err(errors::storage_limit());
        }
    }
    Ok(())
}
