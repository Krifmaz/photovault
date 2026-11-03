//! Scrubbing: re-read objects, verify their SHA-256, repair from a replica.
//!
//! The scrubber works through [`super::catalog::Catalog::due_for_scrub`]:
//! first `unhashed` objects (hashed on first read), then `corrupt` and
//! `missing` ones (repair), then `ok` objects not verified for
//! `[durability.scrub] interval_days`, oldest first. Reads go through a
//! rate limiter and only happen inside the configured daily window, so a
//! full pass is spread over the interval instead of hammering the disks.
//!
//! Repair takes a replica from a target (local targets first, then
//! remote), streams it into the primary store and checks its SHA-256 on the
//! way: a replica that does not match aborts the write before it is
//! committed, so a bad copy can never replace anything.

use super::catalog::{ObjectRow, state};
use super::config::TargetKind;
use super::events::{self, Severity};
use super::ratelimit::{RateLimiter, throttle};
use super::schedule::Window;
use crate::db::now_micros;
use crate::state::AppState;
use crate::storage::{ByteStream, StorageError};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

/// Objects fetched from the catalog per batch.
const BATCH: i64 = 100;
/// Idle wait when nothing is due.
const IDLE: Duration = Duration::from_secs(600);

/// Statistics of a scrub run.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ScrubStats {
    /// Objects read and found intact.
    pub verified: u64,
    /// Previously unhashed objects hashed for the first time.
    pub hashed: u64,
    /// Bytes read.
    pub bytes: u64,
    /// Objects found corrupt (checksum or size mismatch, unreadable).
    pub corrupt: u64,
    /// Objects found missing.
    pub missing: u64,
    /// Objects repaired from a replica.
    pub repaired: u64,
    /// Objects that could not be repaired.
    pub unrepairable: u64,
    /// Transient read errors (left for the next pass).
    pub errors: u64,
}

/// Outcome of verifying one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Matched (or hashed for the first time).
    Ok,
    /// Checksum/size mismatch or unreadable; repaired from `target`.
    Repaired(String),
    /// Corrupt or missing and not repairable (why).
    Unrepairable(String),
    /// Corrupt or missing; repair is off.
    Damaged(String),
    /// Transient backend error; try again later.
    Error(String),
}

/// What reading an object produced.
enum Read {
    Ok { sha256: String, size: u64 },
    NotFound,
    Unreadable(String),
    Transient(String),
}

/// Hash a stream to the end.
async fn hash_stream(mut s: ByteStream) -> Result<(String, u64), std::io::Error> {
    let mut h = Sha256::new();
    let mut n = 0u64;
    while let Some(c) = s.next().await {
        let c = c?;
        n += c.len() as u64;
        h.update(&c);
    }
    Ok((hex::encode(h.finalize()), n))
}

async fn read_primary(state: &AppState, key: &str, limiter: &Arc<RateLimiter>) -> Read {
    // Errors on the local backend come from the disk under this object;
    // remote errors are more likely the network, so they are retried later.
    let local = state.store.name() == "local";
    let stream = match state.store.get_stream(key).await {
        Ok(s) => s,
        Err(StorageError::NotFound) => return Read::NotFound,
        Err(e) if local => return Read::Unreadable(e.to_string()),
        Err(e) => return Read::Transient(e.to_string()),
    };
    match hash_stream(throttle(stream, limiter.clone())).await {
        Ok((sha256, size)) => Read::Ok { sha256, size },
        Err(e) if local => Read::Unreadable(e.to_string()),
        Err(e) => Read::Transient(e.to_string()),
    }
}

/// A stream adapter that checks length and SHA-256 at the end and turns a
/// mismatch into a final error item, so the receiving store aborts the
/// write instead of committing it.
pub fn verified(body: ByteStream, size: u64, sha256_hex: String) -> ByteStream {
    struct St {
        body: ByteStream,
        h: Sha256,
        n: u64,
        size: u64,
        want: String,
        done: bool,
    }
    let st = St {
        body,
        h: Sha256::new(),
        n: 0,
        size,
        want: sha256_hex,
        done: false,
    };
    Box::pin(futures_util::stream::unfold(st, |mut st| async move {
        if st.done {
            return None;
        }
        match st.body.next().await {
            Some(Ok(c)) => {
                st.n += c.len() as u64;
                st.h.update(&c);
                if st.n > st.size {
                    st.done = true;
                    return Some((
                        Err(std::io::Error::other("replica longer than expected")),
                        st,
                    ));
                }
                Some((Ok(c), st))
            }
            Some(Err(e)) => {
                st.done = true;
                Some((Err(e), st))
            }
            None => {
                st.done = true;
                let got = hex::encode(st.h.clone().finalize());
                if st.n != st.size || got != st.want {
                    Some((Err(std::io::Error::other("replica checksum mismatch")), st))
                } else {
                    None
                }
            }
        }
    }))
}

/// Restore `row` from the first target whose copy matches its checksum.
/// Returns the target's name, or why nothing worked.
pub async fn repair(
    state: &AppState,
    row: &ObjectRow,
    limiter: &Arc<RateLimiter>,
) -> Result<String, String> {
    let Some(sha) = row.sha256.clone() else {
        return Err("no checksum recorded".into());
    };
    let mut targets = state.durability.targets.clone();
    // Local copies first: faster, and no egress fees.
    targets.sort_by_key(|t| !matches!(t.config.kind, TargetKind::Local { .. }));
    if targets.is_empty() {
        return Err("no replication targets configured".into());
    }
    let mut tried = Vec::new();
    for t in targets {
        let body = match t.store.get_stream(&row.key).await {
            Ok(b) => b,
            Err(StorageError::NotFound) => {
                tried.push(format!("{}: no copy", t.name));
                continue;
            }
            Err(e) => {
                tried.push(format!("{}: {e}", t.name));
                continue;
            }
        };
        let body = verified(throttle(body, limiter.clone()), row.size, sha.clone());
        match state.store.put_stream(&row.key, row.size, body).await {
            Ok(()) => return Ok(t.name.clone()),
            Err(e) => tried.push(format!("{}: {e}", t.name)),
        }
    }
    Err(tried.join("; "))
}

/// Verify one object and repair it if needed (and allowed).
pub async fn verify_one(
    state: &AppState,
    row: &ObjectRow,
    limiter: &Arc<RateLimiter>,
    stats: &mut ScrubStats,
) -> anyhow::Result<Outcome> {
    let catalog = &state.catalog;
    let problem = match read_primary(state, &row.key, limiter).await {
        Read::Ok { sha256, size } => {
            stats.bytes += size;
            match &row.sha256 {
                None => {
                    catalog.record_written(&row.key, size, &sha256).await?;
                    stats.hashed += 1;
                    return Ok(Outcome::Ok);
                }
                Some(want) if *want == sha256 && size == row.size => {
                    catalog.mark_verified(&row.key).await?;
                    if row.state != state::OK {
                        events::record(
                            &state.db,
                            "scrub.recovered",
                            Severity::Info,
                            &row.key,
                            "object reads correctly again",
                        )
                        .await;
                    }
                    stats.verified += 1;
                    return Ok(Outcome::Ok);
                }
                Some(want) => {
                    stats.corrupt += 1;
                    (
                        state::CORRUPT,
                        format!(
                            "checksum {sha256} ({size} bytes), expected {want} ({} bytes)",
                            row.size
                        ),
                    )
                }
            }
        }
        Read::NotFound => {
            stats.missing += 1;
            (state::MISSING, "object not found in storage".to_string())
        }
        Read::Unreadable(e) => {
            stats.corrupt += 1;
            (state::CORRUPT, format!("read failed: {e}"))
        }
        Read::Transient(e) => {
            stats.errors += 1;
            sqlx::query("UPDATE durability_objects SET last_error = $1 WHERE object_key = $2")
                .bind(&e)
                .bind(&row.key)
                .execute(&state.db.pool)
                .await?;
            return Ok(Outcome::Error(e));
        }
    };
    let (new_state, why) = problem;
    if row.sha256.is_none() {
        // Never hashed and now gone or unreadable: nothing to verify a copy against.
        catalog.set_state(&row.key, new_state, Some(&why)).await?;
        events::record(
            &state.db,
            "scrub.unrepairable",
            Severity::Critical,
            &row.key,
            &format!("{why}; no checksum was ever recorded"),
        )
        .await;
        stats.unrepairable += 1;
        return Ok(Outcome::Unrepairable(why));
    }
    catalog.set_state(&row.key, new_state, Some(&why)).await?;
    if row.state == state::OK || row.state == state::UNHASHED {
        let kind = if new_state == state::MISSING {
            "scrub.missing"
        } else {
            "scrub.corrupt"
        };
        events::record(&state.db, kind, Severity::Critical, &row.key, &why).await;
    }
    if !state.config.durability.scrub.repair {
        return Ok(Outcome::Damaged(why));
    }
    match repair(state, row, limiter).await {
        Ok(target) => {
            catalog.mark_repaired(&row.key).await?;
            stats.repaired += 1;
            events::record(
                &state.db,
                "scrub.repaired",
                Severity::Warning,
                &row.key,
                &format!("{why}; restored from target {target}"),
            )
            .await;
            Ok(Outcome::Repaired(target))
        }
        Err(e) => {
            catalog
                .set_state(
                    &row.key,
                    new_state,
                    Some(&format!("{why}; repair failed: {e}")),
                )
                .await?;
            stats.unrepairable += 1;
            events::record(
                &state.db,
                "scrub.unrepairable",
                Severity::Critical,
                &row.key,
                &format!("{why}; repair failed: {e}"),
            )
            .await;
            Ok(Outcome::Unrepairable(e))
        }
    }
}

fn limiter(state: &AppState) -> Arc<RateLimiter> {
    RateLimiter::new(state.config.durability.scrub.max_read_mib_per_sec * 1024 * 1024)
}

fn verified_before(state: &AppState) -> i64 {
    now_micros() - state.config.durability.scrub.interval_days as i64 * 86_400_000_000
}

/// Start the scrubber. It records a `scrub` job run per batch of work done,
/// waits for its window, and idles when nothing is due. A damaged object
/// that cannot be repaired is retried at most once per idle period.
pub fn spawn(state: AppState) {
    let cfg = state.config.durability.scrub.clone();
    let window = match Window::parse(&cfg.window, &cfg.utc_offset) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "invalid [durability.scrub] window; scrubbing disabled");
            return;
        }
    };
    tokio::spawn(async move {
        let limiter = limiter(&state);
        tokio::time::sleep(Duration::from_secs(120)).await;
        loop {
            window.wait_open().await;
            super::jobs::start(&state.db, "scrub").await;
            let mut stats = ScrubStats::default();
            let r = run_until_idle(&state, &window, &limiter, &mut stats).await;
            let res = r.map_err(|e| e.to_string());
            if let Err(e) = &res {
                tracing::warn!(error = %e, "scrub failed");
            }
            super::jobs::finish(
                &state.db,
                "scrub",
                res,
                serde_json::to_value(&stats).unwrap_or_default(),
            )
            .await;
            tokio::select! {
                () = tokio::time::sleep(IDLE) => {}
                () = state.durability.triggers.scrub.notified() => {}
            }
        }
    });
}

/// Work through due objects while the window is open. Damaged objects that
/// stay damaged are skipped for the rest of this run.
async fn run_until_idle(
    state: &AppState,
    window: &Window,
    limiter: &Arc<RateLimiter>,
    stats: &mut ScrubStats,
) -> anyhow::Result<()> {
    let mut skip = std::collections::HashSet::new();
    loop {
        if !window.contains(std::time::SystemTime::now()) {
            return Ok(());
        }
        let due = state
            .catalog
            .due_for_scrub(verified_before(state), BATCH + skip.len() as i64)
            .await?;
        let todo: Vec<_> = due.into_iter().filter(|r| !skip.contains(&r.key)).collect();
        if todo.is_empty() {
            return Ok(());
        }
        for row in todo {
            match verify_one(state, &row, limiter, stats).await? {
                Outcome::Ok | Outcome::Repaired(_) => {}
                _ => {
                    skip.insert(row.key.clone());
                }
            }
            if !window.contains(std::time::SystemTime::now()) {
                return Ok(());
            }
        }
    }
}

/// `photovault admin durability scrub`: verify one key, or every due object
/// (up to `limit`), ignoring the window.
pub async fn cli_scrub(
    state: &AppState,
    key: Option<&str>,
    limit: Option<i64>,
) -> anyhow::Result<serde_json::Value> {
    let limiter = limiter(state);
    let mut stats = ScrubStats::default();
    if let Some(k) = key {
        let row = match state.catalog.get(k).await? {
            Some(r) => r,
            None => {
                let size = state
                    .store
                    .head(k)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no such object"))?;
                state.catalog.record_seen(k, size).await?;
                state
                    .catalog
                    .get(k)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no such object"))?
            }
        };
        let outcome = verify_one(state, &row, &limiter, &mut stats).await?;
        return Ok(serde_json::json!({ "outcome": format!("{outcome:?}"), "stats": stats }));
    }
    let mut left = limit.unwrap_or(i64::MAX);
    super::jobs::start(&state.db, "scrub").await;
    let mut skip = std::collections::HashSet::new();
    while left > 0 {
        let due = state
            .catalog
            .due_for_scrub(verified_before(state), BATCH.min(left) + skip.len() as i64)
            .await?;
        let todo: Vec<_> = due.into_iter().filter(|r| !skip.contains(&r.key)).collect();
        if todo.is_empty() {
            break;
        }
        for row in todo {
            left -= 1;
            if !matches!(
                verify_one(state, &row, &limiter, &mut stats).await?,
                Outcome::Ok | Outcome::Repaired(_)
            ) {
                skip.insert(row.key.clone());
            }
            if left == 0 {
                break;
            }
        }
    }
    let v = serde_json::to_value(&stats)?;
    super::jobs::finish(&state.db, "scrub", Ok(()), v.clone()).await;
    Ok(v)
}
