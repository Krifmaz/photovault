//! Real storage numbers for the apps.
//!
//! Museum reports the subscription's storage as the limit; a self-hosted museum
//! shows a fixed 100 TB placeholder. Photovault reports what the server can
//! actually hold instead:
//!
//! * **used**: the bytes the user (or their family) stores, unchanged.
//! * **limit**: `used + free space on the storage backend`, capped by the
//!   user's quota when one is set (`users.storage_quota > 0`; `0` = no quota).
//!
//! Free space comes from `statvfs` on the local storage path. For S3 there is no
//! such call, so it is `PHOTOVAULT_STORAGE_LIMIT` minus everything stored, or
//! unlimited when that is unset. `PHOTOVAULT_STORAGE_LIMIT` also caps the local
//! backend (e.g. to keep part of a shared pool free).

use crate::config::StorageConfig;
use crate::db::Db;
use crate::error::ApiResult;
use crate::state::AppState;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What the apps are told when neither a quota nor the backend sets a limit
/// (S3 without `PHOTOVAULT_STORAGE_LIMIT`): museum's self-hosted 100 TiB.
pub const UNLIMITED: i64 = 100 * 1024 * 1024 * 1024 * 1024;

/// How long one `statvfs` reading is reused.
const CACHE_FOR: Duration = Duration::from_secs(10);

/// Cached free-space reading.
#[derive(Debug, Default)]
pub struct Cache(Mutex<Option<(Instant, Option<i64>)>>);

/// Free bytes on the filesystem holding `path` (space available to unprivileged users).
pub fn statvfs_free(path: &std::path::Path) -> std::io::Result<i64> {
    let s = rustix::fs::statvfs(path).map_err(std::io::Error::from)?;
    let free = u128::from(s.f_bavail) * u128::from(s.f_frsize.max(1));
    Ok(i64::try_from(free).unwrap_or(i64::MAX))
}

/// Total stored bytes across all users.
pub async fn total_usage(db: &Db) -> ApiResult<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT CAST(COALESCE(SUM(storage_consumed), 0) AS BIGINT) FROM usage")
            .fetch_one(&db.pool)
            .await?;
    Ok(n)
}

/// Free bytes the backend can still take, or `None` when unlimited.
pub async fn free_bytes(state: &AppState) -> ApiResult<Option<i64>> {
    if let Ok(g) = state.capacity.0.lock()
        && let Some((at, v)) = *g
        && at.elapsed() < CACHE_FOR
    {
        return Ok(v);
    }
    if let Some(f) = state.config.free_space_override {
        return Ok(Some(f));
    }
    let disk = match &state.config.storage {
        StorageConfig::Local { path, .. } => {
            let p = path.clone();
            match tokio::task::spawn_blocking(move || statvfs_free(&p)).await {
                Ok(Ok(f)) => Some(f),
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, path = %path.display(), "cannot read free space");
                    None
                }
                Err(_) => None,
            }
        }
        StorageConfig::S3(_) => None,
    };
    let configured = match state.config.storage_limit {
        Some(limit) => Some((limit - total_usage(&state.db).await?).max(0)),
        None => None,
    };
    let free = match (disk, configured) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if let Ok(mut g) = state.capacity.0.lock() {
        *g = Some((Instant::now(), free));
    }
    Ok(free)
}

/// The limit to report for an account whose plan is held by `admin_id` (the user
/// itself, or their family admin), given that account's stored `quota`.
pub async fn effective_limit(state: &AppState, admin_id: i64, quota: i64) -> ApiResult<i64> {
    let free = free_bytes(state).await?;
    let used = match free {
        Some(_) => {
            let mut members: Vec<i64> = crate::users::family::active_members(&state.db, admin_id)
                .await?
                .into_iter()
                .map(|m| m.0)
                .collect();
            if members.is_empty() {
                members.push(admin_id);
            }
            crate::users::store::combined_usage(&state.db, &members).await?
        }
        None => 0,
    };
    Ok(combine(quota, used, free))
}

/// `min(quota, used + free)`, where `quota <= 0` and `free = None` mean "no limit".
pub fn combine(quota: i64, used: i64, free: Option<i64>) -> i64 {
    let q = if quota > 0 { quota } else { i64::MAX };
    let d = free.map_or(i64::MAX, |f| used.saturating_add(f));
    match q.min(d) {
        i64::MAX => UNLIMITED,
        n => n,
    }
}

/// [`crate::users::subscription`] with `storage` replaced by [`effective_limit`].
pub async fn subscription(
    state: &AppState,
    user_id: i64,
) -> ApiResult<crate::users::store::Subscription> {
    let mut sub = crate::users::subscription(&state.db, user_id).await?;
    let admin: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    let admin = admin.and_then(|a| a.0).unwrap_or(user_id);
    // A family member's own row is not their plan; the admin's quota applies.
    let quota = if admin == user_id {
        sub.storage
    } else {
        crate::users::subscription(&state.db, admin).await?.storage
    };
    sub.storage = effective_limit(state, admin, quota).await?;
    Ok(sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_rules() {
        // No quota: used + free.
        assert_eq!(combine(0, 10, Some(90)), 100);
        // Quota below disk.
        assert_eq!(combine(50, 10, Some(90)), 50);
        // Quota above disk: the disk wins.
        assert_eq!(combine(500, 10, Some(90)), 100);
        // S3 without a limit and no quota.
        assert_eq!(combine(0, 10, None), UNLIMITED);
        assert_eq!(combine(42, 10, None), 42);
    }

    #[test]
    fn statvfs_reads_something() {
        let free = statvfs_free(std::path::Path::new(".")).expect("statvfs");
        assert!(free > 0);
    }
}
