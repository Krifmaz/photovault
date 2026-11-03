//! Last-run bookkeeping for background jobs (`durability_jobs`), shown on
//! the status page: when each job last started and finished, whether it
//! succeeded, and its statistics.

use crate::db::{Db, now_micros};
use serde::Serialize;

/// A job's last run.
#[derive(Debug, Clone, Serialize)]
pub struct JobStatus {
    /// Job name: `sweep`, `scrub`, `replicate:<target>`, `verify:<target>`,
    /// `dbbackup`, `fsview`, `evacuate:<branch>`.
    pub job: String,
    /// Start, microseconds since the epoch.
    pub started_at: i64,
    /// End, or `None` while running.
    pub finished_at: Option<i64>,
    /// Whether the last finished run succeeded.
    pub ok: bool,
    /// Job-specific statistics.
    pub stats: serde_json::Value,
    /// Error of the last failed run.
    pub error: Option<String>,
}

/// Mark `job` as started now.
pub async fn start(db: &Db, job: &str) {
    let r = sqlx::query(
        "INSERT INTO durability_jobs (job, started_at, finished_at, ok, stats, error) VALUES ($1, $2, NULL, TRUE, '{}', NULL) \
         ON CONFLICT (job) DO UPDATE SET started_at = excluded.started_at, finished_at = NULL",
    )
    .bind(job)
    .bind(now_micros())
    .execute(&db.pool)
    .await;
    if let Err(e) = r {
        tracing::warn!(job, error = %e, "recording job start failed");
    }
}

/// Mark `job` as finished with `result` and `stats`.
pub async fn finish(db: &Db, job: &str, result: Result<(), String>, stats: serde_json::Value) {
    let (ok, err) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    let r = sqlx::query(
        "UPDATE durability_jobs SET finished_at = $1, ok = $2, stats = $3, error = $4 WHERE job = $5",
    )
    .bind(now_micros())
    .bind(ok)
    .bind(stats.to_string())
    .bind(err)
    .bind(job)
    .execute(&db.pool)
    .await;
    if let Err(e) = r {
        tracing::warn!(job, error = %e, "recording job finish failed");
    }
}

/// Every job's last run, by name.
pub async fn all(db: &Db) -> anyhow::Result<Vec<JobStatus>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT job, started_at, finished_at, ok, stats, error FROM durability_jobs ORDER BY job",
    )
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(JobStatus {
                job: r.try_get("job")?,
                started_at: r.try_get("started_at")?,
                finished_at: r.try_get("finished_at")?,
                ok: crate::library::sql::get_bool(r, "ok")?,
                stats: serde_json::from_str(&r.try_get::<String, _>("stats")?).unwrap_or_default(),
                error: r.try_get("error")?,
            })
        })
        .collect()
}
