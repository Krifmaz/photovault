//! The catalog sweep: lists the primary store and reconciles it with the
//! catalog.
//!
//! * Objects the catalog does not know (uploaded straight to S3 through a
//!   presigned URL, written before durability existed, restored by hand) are
//!   recorded as `unhashed`; the scrubber hashes them on its next pass.
//! * Objects whose listed size differs from the recorded one are marked
//!   `corrupt` for the scrubber to verify and repair.
//! * Live objects that a complete listing did not find, and that were not
//!   written while it ran, are marked `missing` (critical event) and repaired
//!   from a replica by the scrubber. Deletes through the server are recorded
//!   before they happen, so they never show up as missing.
//! * Catalog rows of deleted objects are purged once no replica is left.

use super::catalog::state;
use super::events::{self, Severity};
use crate::db::now_micros;
use crate::state::AppState;
use std::time::Duration;

/// Objects listed per page.
const PAGE: usize = 1000;
/// At most this many missing objects are reported one by one per sweep.
const MAX_MISSING_EVENTS: usize = 20;

/// Statistics of one sweep.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct SweepStats {
    /// Objects listed in storage.
    pub listed: u64,
    /// Bytes listed.
    pub bytes: u64,
    /// Objects newly recorded (unhashed).
    pub new: u64,
    /// Objects whose size differed from the catalog.
    pub size_mismatch: u64,
    /// Objects newly found missing.
    pub missing: u64,
    /// Deleted rows purged.
    pub purged: u64,
}

/// Start the periodic sweep (first run one minute after start).
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let every = Duration::from_secs(state.config.durability.sweep_interval_minutes.max(1) * 60);
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Err(e) = run_once(&state).await {
                tracing::warn!(error = %e, "durability sweep failed");
            }
            tokio::select! {
                () = tokio::time::sleep(every) => {}
                () = state.durability.triggers.sweep.notified() => {}
            }
        }
    });
}

/// One sweep now; returns statistics (as JSON, for the CLI and job record).
pub async fn run_once(state: &AppState) -> anyhow::Result<serde_json::Value> {
    super::jobs::start(&state.db, "sweep").await;
    let r = sweep(state).await;
    let (res, stats) = match &r {
        Ok(s) => (Ok(()), serde_json::to_value(s)?),
        Err(e) => (Err(e.to_string()), serde_json::Value::Null),
    };
    super::jobs::finish(&state.db, "sweep", res, stats.clone()).await;
    if let Err(e) = &r {
        events::record(
            &state.db,
            "sweep.failed",
            Severity::Warning,
            "sweep",
            &e.to_string(),
        )
        .await;
    }
    r.map(|_| stats)
}

async fn sweep(state: &AppState) -> anyhow::Result<SweepStats> {
    let start = now_micros();
    let catalog = &state.catalog;
    let mut stats = SweepStats::default();
    let mut after: Option<String> = None;
    loop {
        let page = state
            .store
            .list(after.as_deref(), PAGE)
            .await
            .map_err(|e| anyhow::anyhow!("listing storage: {e}"))?;
        let Some(last) = page.last() else { break };
        after = Some(last.key.clone());
        let keys: Vec<String> = page.iter().map(|o| o.key.clone()).collect();
        let known = catalog.lookup_many(&keys).await?;
        for o in &page {
            stats.listed += 1;
            stats.bytes += o.size;
            match known.get(&o.key) {
                None => {
                    catalog.record_seen(&o.key, o.size).await?;
                    stats.new += 1;
                }
                Some((st, _)) if st == state::DELETED => {
                    // Deleted through the server but still present: the
                    // delete failed or the key was written again. Re-check it.
                    if catalog.record_seen(&o.key, o.size).await? {
                        stats.new += 1;
                    }
                }
                Some((st, size)) if st == state::OK && *size != o.size => {
                    catalog
                        .set_state(
                            &o.key,
                            state::CORRUPT,
                            Some(&format!("size {} in storage, {size} recorded", o.size)),
                        )
                        .await?;
                    stats.size_mismatch += 1;
                    events::record(
                        &state.db,
                        "sweep.size_mismatch",
                        Severity::Critical,
                        &o.key,
                        &format!("storage has {} bytes, catalog recorded {size}", o.size),
                    )
                    .await;
                }
                Some(_) => {}
            }
        }
        catalog.touch_seen(&keys, start).await?;
        if page.len() < PAGE {
            break;
        }
    }
    // Only a complete listing may declare objects missing.
    let missing = catalog.mark_unseen_missing(start).await?;
    stats.missing = missing.len() as u64;
    for k in missing.iter().take(MAX_MISSING_EVENTS) {
        events::record(
            &state.db,
            "sweep.missing",
            Severity::Critical,
            k,
            "object is gone from storage without having been deleted; the scrubber will restore it from a replica",
        )
        .await;
    }
    if missing.len() > MAX_MISSING_EVENTS {
        events::record(
            &state.db,
            "sweep.missing",
            Severity::Critical,
            "storage",
            &format!(
                "{} more objects are missing from storage",
                missing.len() - MAX_MISSING_EVENTS
            ),
        )
        .await;
    }
    let keep_days = state
        .config
        .durability
        .targets
        .iter()
        .map(|t| t.deleted_retention_days)
        .max()
        .unwrap_or(0) as i64;
    stats.purged = catalog
        .purge_deleted(now_micros() - keep_days * 86_400_000_000)
        .await?;
    tracing::info!(
        listed = stats.listed,
        new = stats.new,
        missing = stats.missing,
        size_mismatch = stats.size_mismatch,
        "durability sweep finished"
    );
    Ok(stats)
}
