//! Durability events: the `durability_events` table. Every job records what
//! the operator should know about here; the status page lists them and
//! [`super::alerts`] delivers the ones at or above the configured severity.

use crate::db::{Db, now_micros};
use serde::Serialize;

/// Event severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Routine (a job finished, an object was repaired).
    Info,
    /// Needs attention soon (replication lagging, a branch read-only).
    Warning,
    /// Data at risk (unrepairable object, a target unreachable for long).
    Critical,
}

impl Severity {
    /// Lowercase name, as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
    /// Parse a stored or configured name (unknown → `Warning`).
    pub fn parse(s: &str) -> Self {
        match s {
            "info" => Severity::Info,
            "critical" => Severity::Critical,
            _ => Severity::Warning,
        }
    }
}

/// A stored event.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Row id.
    pub id: i64,
    /// Time, microseconds since the epoch.
    pub at: i64,
    /// Machine-readable kind, e.g. `scrub.corrupt`, `replicate.failed`,
    /// `fsview.branch_unhealthy`.
    pub kind: String,
    /// Severity.
    pub severity: Severity,
    /// What it is about: an object key, a target name, a branch path.
    pub subject: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Record an event (errors are logged, never returned). Also logged at the
/// matching tracing level.
pub async fn record(db: &Db, kind: &str, severity: Severity, subject: &str, detail: &str) {
    match severity {
        Severity::Info => tracing::info!(kind, subject, detail, "durability event"),
        Severity::Warning => tracing::warn!(kind, subject, detail, "durability event"),
        Severity::Critical => tracing::error!(kind, subject, detail, "durability event"),
    }
    let r = sqlx::query(
        "INSERT INTO durability_events (at, kind, severity, subject, detail, alerted) VALUES ($1, $2, $3, $4, $5, FALSE)",
    )
    .bind(now_micros())
    .bind(kind)
    .bind(severity.as_str())
    .bind(subject)
    .bind(detail)
    .execute(&db.pool)
    .await;
    if let Err(e) = r {
        tracing::warn!(error = %e, "recording durability event failed");
    }
}

fn from_row(r: &sqlx::any::AnyRow) -> Result<Event, sqlx::Error> {
    use sqlx::Row;
    Ok(Event {
        id: r.try_get("id")?,
        at: r.try_get("at")?,
        kind: r.try_get("kind")?,
        severity: Severity::parse(&r.try_get::<String, _>("severity")?),
        subject: r.try_get("subject")?,
        detail: r.try_get("detail")?,
    })
}

/// Newest events first, at most `limit`.
pub async fn recent(db: &Db, limit: i64) -> anyhow::Result<Vec<Event>> {
    let rows = sqlx::query(
        "SELECT id, at, kind, severity, subject, detail FROM durability_events ORDER BY id DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(from_row).collect::<Result<_, _>>()?)
}

/// Events not yet handed to the alerter, oldest first.
pub async fn unalerted(db: &Db, limit: i64) -> anyhow::Result<Vec<Event>> {
    let rows = sqlx::query(
        "SELECT id, at, kind, severity, subject, detail FROM durability_events WHERE alerted = FALSE ORDER BY id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(from_row).collect::<Result<_, _>>()?)
}

/// Mark an event as handled by the alerter.
pub async fn mark_alerted(db: &Db, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE durability_events SET alerted = TRUE WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Delete events older than `days`.
pub async fn prune(db: &Db, days: i64) -> anyhow::Result<u64> {
    let cutoff = now_micros() - days * 86_400_000_000;
    Ok(sqlx::query("DELETE FROM durability_events WHERE at < $1")
        .bind(cutoff)
        .execute(&db.pool)
        .await?
        .rows_affected())
}
