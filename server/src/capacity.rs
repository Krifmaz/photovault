//! Real storage numbers for the apps.
//!
//! Museum reports the subscription's storage as the limit; a self-hosted museum
//! shows a fixed 100 TB placeholder. Photovault reports the disk that holds the
//! photos instead, like Immich's server stats (contracts/storage-usage.md):
//!
//! * **disk**: `statvfs` of the local storage path: total size, used (the whole
//!   filesystem, as `df` counts it) and space still available to the server.
//!   `PHOTOVAULT_STORAGE_LIMIT` replaces the disk with that budget (needed for
//!   real numbers on S3; also caps a shared local pool). S3 without it: none.
//! * **limit** shown to Ente's endpoints: the disk's total size, or the user's
//!   quota when an admin set one (`users.storage_quota > 0`; `0` = no quota).
//! * **used** stays the bytes the user (or their family) stores.
//!
//! Uploads are still refused when the upload is larger than the space
//! available ([`free_bytes`]) or exceeds the quota.

use crate::config::StorageConfig;
use crate::db::Db;
use crate::error::ApiResult;
use crate::state::AppState;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What the apps are told when neither a quota nor the backend sets a limit
/// (S3 without `PHOTOVAULT_STORAGE_LIMIT`): museum's self-hosted 100 TiB.
pub const UNLIMITED: i64 = 100 * 1024 * 1024 * 1024 * 1024;

/// The apps warn from this fill level on (percent).
pub const NEARLY_FULL_PERCENT: f64 = 90.0;

/// How long one `statvfs` reading is reused.
const CACHE_FOR: Duration = Duration::from_secs(10);

/// Where the disk numbers come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskSource {
    /// `statvfs` of the local storage path.
    Disk,
    /// `PHOTOVAULT_STORAGE_LIMIT`.
    StorageLimit,
}

/// The filesystem (or configured budget) that holds the photos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disk {
    /// Size of the filesystem.
    pub total: i64,
    /// Bytes in use on the whole filesystem, by anything (`df`'s "Used").
    pub used: i64,
    /// Bytes the server can still write (`df`'s "Avail").
    pub available: i64,
    /// Where these numbers come from.
    pub source: DiskSource,
}

impl Disk {
    /// Fill level in percent, computed like `df`'s `Use%`: `used / (used + available)`,
    /// so space reserved for root counts as full.
    pub fn used_percent(&self) -> f64 {
        let denom = self.used.saturating_add(self.available);
        if denom <= 0 {
            return 0.0;
        }
        (self.used as f64 / denom as f64 * 100.0).clamp(0.0, 100.0)
    }

    /// True from [`NEARLY_FULL_PERCENT`] on.
    pub fn nearly_full(&self) -> bool {
        self.used_percent() >= NEARLY_FULL_PERCENT
    }

    /// A configured budget of `limit` bytes of which `stored` are taken, on a
    /// disk with `disk_available` bytes left (`None` on S3).
    pub fn budget(limit: i64, stored: i64, disk_available: Option<i64>) -> Self {
        let mut available = (limit - stored).max(0);
        if let Some(a) = disk_available {
            available = available.min(a.max(0));
        }
        Disk {
            total: limit,
            used: (limit - available).max(0),
            available,
            source: DiskSource::StorageLimit,
        }
    }
}

/// Cached disk reading.
#[derive(Debug, Default)]
pub struct Cache(Mutex<Option<(Instant, Option<Disk>)>>);

/// `statvfs` of the filesystem holding `path`.
pub fn statvfs_disk(path: &std::path::Path) -> std::io::Result<Disk> {
    let s = rustix::fs::statvfs(path).map_err(std::io::Error::from)?;
    let unit = u128::from(s.f_frsize.max(1));
    let bytes = |blocks: u64| i64::try_from(u128::from(blocks) * unit).unwrap_or(i64::MAX);
    Ok(Disk {
        total: bytes(s.f_blocks),
        used: bytes(s.f_blocks.saturating_sub(s.f_bfree)),
        available: bytes(s.f_bavail),
        source: DiskSource::Disk,
    })
}

/// Free bytes on the filesystem holding `path` (space available to unprivileged users).
pub fn statvfs_free(path: &std::path::Path) -> std::io::Result<i64> {
    statvfs_disk(path).map(|d| d.available)
}

/// Total stored bytes across all users.
pub async fn total_usage(db: &Db) -> ApiResult<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT CAST(COALESCE(SUM(storage_consumed), 0) AS BIGINT) FROM usage")
            .fetch_one(&db.pool)
            .await?;
    Ok(n)
}

/// The disk that holds the photos, or `None` when there is none to report
/// (S3 without `PHOTOVAULT_STORAGE_LIMIT`, or `statvfs` failed).
pub async fn disk(state: &AppState) -> ApiResult<Option<Disk>> {
    if let Ok(g) = state.capacity.0.lock()
        && let Some((at, v)) = *g
        && at.elapsed() < CACHE_FOR
    {
        return Ok(v);
    }
    let c = &state.config;
    let physical = if let Some(d) = c.disk_override {
        Some(d)
    } else if let Some(f) = c.free_space_override {
        // Tests: a disk with exactly `f` bytes left and only our files on it.
        let used = total_usage(&state.db).await?;
        Some(Disk {
            total: used.saturating_add(f),
            used,
            available: f,
            source: DiskSource::Disk,
        })
    } else {
        match &c.storage {
            StorageConfig::Local { path, .. } => {
                let p = path.clone();
                match tokio::task::spawn_blocking(move || statvfs_disk(&p)).await {
                    Ok(Ok(d)) => Some(d),
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, path = %path.display(), "cannot read disk usage");
                        None
                    }
                    Err(_) => None,
                }
            }
            StorageConfig::S3(_) => None,
        }
    };
    let disk = match c.storage_limit {
        Some(limit) => {
            let limit = physical.map_or(limit, |d| limit.min(d.total));
            Some(Disk::budget(
                limit,
                total_usage(&state.db).await?,
                physical.map(|d| d.available),
            ))
        }
        None => physical,
    };
    if let Ok(mut g) = state.capacity.0.lock() {
        *g = Some((Instant::now(), disk));
    }
    Ok(disk)
}

/// Free bytes the backend can still take, or `None` when unlimited.
pub async fn free_bytes(state: &AppState) -> ApiResult<Option<i64>> {
    Ok(disk(state).await?.map(|d| d.available))
}

/// The limit to report for an account with plan quota `quota`.
pub async fn effective_limit(state: &AppState, quota: i64) -> ApiResult<i64> {
    Ok(combine(quota, disk(state).await?.map(|d| d.total)))
}

/// The quota when an admin set one, else the disk's size; `quota <= 0` and
/// `disk_total = None` mean "none". A quota larger than the disk is capped.
pub fn combine(quota: i64, disk_total: Option<i64>) -> i64 {
    let q = if quota > 0 { quota } else { i64::MAX };
    let d = disk_total.unwrap_or(i64::MAX);
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
    sub.storage = effective_limit(state, quota).await?;
    Ok(sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_rules() {
        // No quota: the disk's size, whatever is on it.
        assert_eq!(combine(0, Some(100)), 100);
        // Quota below disk.
        assert_eq!(combine(50, Some(100)), 50);
        // Quota above disk: the disk wins.
        assert_eq!(combine(500, Some(100)), 100);
        // S3 without a limit and no quota.
        assert_eq!(combine(0, None), UNLIMITED);
        assert_eq!(combine(42, None), 42);
    }

    fn disk(total: i64, used: i64, available: i64) -> Disk {
        Disk {
            total,
            used,
            available,
            source: DiskSource::Disk,
        }
    }

    #[test]
    fn used_percent_matches_df() {
        // 1000 blocks, 50 reserved for root: 855 used, 95 available -> 90 %.
        let d = disk(1000, 855, 95);
        assert_eq!(d.used_percent().round(), 90.0);
        assert!(d.nearly_full());
        assert!(!disk(1000, 500, 500).nearly_full());
        assert!(disk(1000, 900, 100).nearly_full());
        assert!(!disk(1000, 899, 101).nearly_full());
        assert_eq!(disk(0, 0, 0).used_percent(), 0.0);
    }

    #[test]
    fn budget_caps_by_disk() {
        let b = Disk::budget(100, 30, None);
        assert_eq!((b.total, b.used, b.available), (100, 30, 70));
        assert_eq!(b.source, DiskSource::StorageLimit);
        // The real disk has less left than the budget: it counts as used.
        let b = Disk::budget(100, 30, Some(10));
        assert_eq!((b.total, b.used, b.available), (100, 90, 10));
        // Over budget.
        let b = Disk::budget(100, 130, None);
        assert_eq!((b.total, b.used, b.available), (100, 100, 0));
    }

    #[test]
    fn statvfs_reads_the_real_disk() {
        let d = statvfs_disk(std::path::Path::new(".")).expect("statvfs");
        assert!(d.total > 0);
        assert!(d.available > 0);
        assert!(d.used >= 0 && d.used <= d.total);
        assert!(statvfs_free(std::path::Path::new(".")).unwrap() > 0);
    }
}
