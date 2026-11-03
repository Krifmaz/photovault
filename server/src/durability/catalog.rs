//! The durability catalog: one `durability_objects` row per object in the
//! primary store, with its size, SHA-256 and verification state.
//!
//! Rows are written at write time through [`WriteObserver`] (the local
//! backend hashes uploads as it streams them to disk; [`crate::storage::journaled`]
//! hashes server-side writes), and by [`super::sweep`] for objects written
//! without the server seeing the bytes (S3 presigned uploads), which are
//! hashed on first read.

use crate::db::{Db, now_micros};
use crate::storage::{Sha256Digest, WriteObserver};
use serde::Serialize;
use sqlx::Row;

/// Object states, as stored in `durability_objects.state`.
pub mod state {
    /// Checksum known and the last read matched.
    pub const OK: &str = "ok";
    /// Seen in storage, not hashed yet.
    pub const UNHASHED: &str = "unhashed";
    /// The last read did not match the checksum (or failed to read).
    pub const CORRUPT: &str = "corrupt";
    /// Gone from storage without being deleted through the server.
    pub const MISSING: &str = "missing";
    /// Deleted through the server; replicas are kept for the retention period.
    pub const DELETED: &str = "deleted";
}

/// One catalog row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectRow {
    /// Object key.
    pub key: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 (hex), once known.
    pub sha256: Option<String>,
    /// One of [`state`].
    pub state: String,
    /// First recorded, µs.
    pub created_at: i64,
    /// Last successful verification (or write), µs; 0 = never.
    pub verified_at: i64,
    /// When it was deleted through the server, µs.
    pub deleted_at: Option<i64>,
    /// Last repair, µs.
    pub repaired_at: Option<i64>,
    /// Last error seen.
    pub last_error: Option<String>,
}

/// Object counts and bytes per state.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CatalogStats {
    /// `(state, objects, bytes)`.
    pub by_state: Vec<(String, i64, i64)>,
    /// Oldest `verified_at` among live (`ok`) objects, µs; 0 when none.
    pub oldest_verified_at: i64,
}

/// Catalog access. Cheap to clone (holds the pool).
#[derive(Debug, Clone)]
pub struct Catalog {
    db: Db,
}

const COLS: &str =
    "object_key, size, sha256, state, created_at, verified_at, deleted_at, repaired_at, last_error";

fn row(r: &sqlx::any::AnyRow) -> Result<ObjectRow, sqlx::Error> {
    Ok(ObjectRow {
        key: r.try_get("object_key")?,
        size: r.try_get::<i64, _>("size")?.max(0) as u64,
        sha256: r.try_get("sha256")?,
        state: r.try_get("state")?,
        created_at: r.try_get("created_at")?,
        verified_at: r.try_get("verified_at")?,
        deleted_at: r.try_get("deleted_at")?,
        repaired_at: r.try_get("repaired_at")?,
        last_error: r.try_get("last_error")?,
    })
}

impl Catalog {
    /// Catalog over `db`.
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// The database.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Record that `key` holds `size` bytes with `sha256`, verified now.
    pub async fn record_written(&self, key: &str, size: u64, sha256: &str) -> anyhow::Result<()> {
        let now = now_micros();
        sqlx::query(
            "INSERT INTO durability_objects (object_key, size, sha256, state, created_at, verified_at, updated_at) \
             VALUES ($1, $2, $3, 'ok', $4, $4, $4) \
             ON CONFLICT (object_key) DO UPDATE SET size = excluded.size, sha256 = excluded.sha256, \
             state = 'ok', verified_at = excluded.verified_at, deleted_at = NULL, last_error = NULL, \
             updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(size as i64)
        .bind(sha256)
        .bind(now)
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    /// Record an object seen in storage that the catalog does not know (or
    /// knows as deleted/missing): it becomes `unhashed` until read. Known
    /// live rows are left alone. Returns true when a row was inserted or revived.
    pub async fn record_seen(&self, key: &str, size: u64) -> anyhow::Result<bool> {
        let now = now_micros();
        let r = sqlx::query(
            "INSERT INTO durability_objects (object_key, size, sha256, state, created_at, verified_at, updated_at) \
             VALUES ($1, $2, NULL, 'unhashed', $3, 0, $3) \
             ON CONFLICT (object_key) DO UPDATE SET size = excluded.size, sha256 = NULL, state = 'unhashed', \
             verified_at = 0, deleted_at = NULL, updated_at = excluded.updated_at \
             WHERE durability_objects.state = 'deleted'",
        )
        .bind(key)
        .bind(size as i64)
        .bind(now)
        .execute(&self.db.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Set `state` (and `last_error`) of `key`.
    pub async fn set_state(
        &self,
        key: &str,
        state: &str,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE durability_objects SET state = $1, last_error = $2, updated_at = $3 WHERE object_key = $4",
        )
        .bind(state)
        .bind(error)
        .bind(now_micros())
        .bind(key)
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    /// Mark `key` verified now (state `ok`).
    pub async fn mark_verified(&self, key: &str) -> anyhow::Result<()> {
        let now = now_micros();
        sqlx::query(
            "UPDATE durability_objects SET state = 'ok', verified_at = $1, last_error = NULL, updated_at = $1 WHERE object_key = $2",
        )
        .bind(now)
        .bind(key)
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    /// Mark `key` repaired and verified now.
    pub async fn mark_repaired(&self, key: &str) -> anyhow::Result<()> {
        let now = now_micros();
        sqlx::query(
            "UPDATE durability_objects SET state = 'ok', verified_at = $1, repaired_at = $1, last_error = NULL, updated_at = $1 WHERE object_key = $2",
        )
        .bind(now)
        .bind(key)
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    /// One row.
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<ObjectRow>> {
        let r = sqlx::query(&format!(
            "SELECT {COLS} FROM durability_objects WHERE object_key = $1"
        ))
        .bind(key)
        .fetch_optional(&self.db.pool)
        .await?;
        Ok(r.as_ref().map(row).transpose()?)
    }

    /// Rows in `state` with keys after `after`, in key order (for paging).
    pub async fn by_state(
        &self,
        state: &str,
        after: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<ObjectRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM durability_objects WHERE state = $1 AND object_key > $2 ORDER BY object_key LIMIT $3"
        ))
        .bind(state)
        .bind(after)
        .bind(limit)
        .fetch_all(&self.db.pool)
        .await?;
        Ok(rows.iter().map(row).collect::<Result<_, _>>()?)
    }

    /// Live rows (not deleted) with keys after `after`, in key order. Used by
    /// the sweep to find objects that vanished.
    pub async fn live_after(&self, after: &str, limit: i64) -> anyhow::Result<Vec<ObjectRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM durability_objects WHERE state <> 'deleted' AND object_key > $1 ORDER BY object_key LIMIT $2"
        ))
        .bind(after)
        .bind(limit)
        .fetch_all(&self.db.pool)
        .await?;
        Ok(rows.iter().map(row).collect::<Result<_, _>>()?)
    }

    /// Objects that need reading: `unhashed` ones first, then `corrupt` and
    /// `missing` ones (for repair), then `ok` ones last verified before
    /// `verified_before`, oldest first.
    pub async fn due_for_scrub(
        &self,
        verified_before: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<ObjectRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM durability_objects \
             WHERE state IN ('unhashed', 'corrupt', 'missing') OR (state = 'ok' AND verified_at < $1) \
             ORDER BY CASE state WHEN 'unhashed' THEN 0 WHEN 'corrupt' THEN 1 WHEN 'missing' THEN 1 ELSE 2 END, verified_at \
             LIMIT $2"
        ))
        .bind(verified_before)
        .bind(limit)
        .fetch_all(&self.db.pool)
        .await?;
        Ok(rows.iter().map(row).collect::<Result<_, _>>()?)
    }

    /// Remove `deleted` rows deleted before `cutoff` (µs) that no target
    /// still holds a replica of.
    pub async fn purge_deleted(&self, cutoff: i64) -> anyhow::Result<u64> {
        Ok(sqlx::query(
            "DELETE FROM durability_objects WHERE state = 'deleted' AND deleted_at < $1 \
             AND NOT EXISTS (SELECT 1 FROM durability_replicas r WHERE r.object_key = durability_objects.object_key)",
        )
        .bind(cutoff)
        .execute(&self.db.pool)
        .await?
        .rows_affected())
    }

    /// For each of `keys` the catalog knows: `(state, size)`.
    pub async fn lookup_many(
        &self,
        keys: &[String],
    ) -> anyhow::Result<std::collections::HashMap<String, (String, u64)>> {
        let mut out = std::collections::HashMap::new();
        for chunk in keys.chunks(500) {
            let ph: Vec<String> = (1..=chunk.len()).map(|i| format!("${i}")).collect();
            let sql = format!(
                "SELECT object_key, state, size FROM durability_objects WHERE object_key IN ({})",
                ph.join(", ")
            );
            let mut q = sqlx::query(&sql);
            for k in chunk {
                q = q.bind(k);
            }
            for r in q.fetch_all(&self.db.pool).await? {
                out.insert(
                    r.try_get::<String, _>("object_key")?,
                    (
                        r.try_get::<String, _>("state")?,
                        r.try_get::<i64, _>("size")?.max(0) as u64,
                    ),
                );
            }
        }
        Ok(out)
    }

    /// Set `seen_at` of `keys` to `at` (the sweep saw them in storage).
    pub async fn touch_seen(&self, keys: &[String], at: i64) -> anyhow::Result<()> {
        for chunk in keys.chunks(500) {
            let ph: Vec<String> = (2..=chunk.len() + 1).map(|i| format!("${i}")).collect();
            let sql = format!(
                "UPDATE durability_objects SET seen_at = $1 WHERE object_key IN ({})",
                ph.join(", ")
            );
            let mut q = sqlx::query(&sql).bind(at);
            for k in chunk {
                q = q.bind(k);
            }
            q.execute(&self.db.pool).await?;
        }
        Ok(())
    }

    /// Live rows a complete sweep that started at `sweep_start` did not see
    /// and that were not written or changed since: they vanished from
    /// storage. Marks them `missing` and returns their keys.
    pub async fn mark_unseen_missing(&self, sweep_start: i64) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT object_key FROM durability_objects WHERE state IN ('ok', 'unhashed', 'corrupt')              AND seen_at < $1 AND updated_at < $1",
        )
        .bind(sweep_start)
        .fetch_all(&self.db.pool)
        .await?;
        let keys: Vec<String> = rows
            .iter()
            .map(|r| r.try_get::<String, _>("object_key"))
            .collect::<Result<_, _>>()?;
        for k in &keys {
            sqlx::query(
                "UPDATE durability_objects SET state = 'missing', last_error = 'not found by sweep', updated_at = $1 \
                 WHERE object_key = $2 AND seen_at < $3 AND updated_at < $3",
            )
            .bind(now_micros())
            .bind(k)
            .bind(sweep_start)
            .execute(&self.db.pool)
            .await?;
        }
        Ok(keys)
    }

    /// Counts and bytes per state.
    pub async fn stats(&self) -> anyhow::Result<CatalogStats> {
        let rows = sqlx::query(
            "SELECT state, COUNT(*) AS n, COALESCE(SUM(size), 0) AS bytes FROM durability_objects GROUP BY state ORDER BY state",
        )
        .fetch_all(&self.db.pool)
        .await?;
        let mut by_state = Vec::new();
        for r in &rows {
            by_state.push((
                r.try_get::<String, _>("state")?,
                crate::library::sql::get_i64(r, "n")?,
                crate::library::sql::get_i64(r, "bytes")?,
            ));
        }
        let oldest = sqlx::query(
            "SELECT COALESCE(MIN(verified_at), 0) AS v FROM durability_objects WHERE state = 'ok'",
        )
        .fetch_one(&self.db.pool)
        .await?;
        Ok(CatalogStats {
            by_state,
            oldest_verified_at: crate::library::sql::get_i64(&oldest, "v")?,
        })
    }
}

#[async_trait::async_trait]
impl WriteObserver for Catalog {
    async fn written(&self, key: &str, size: u64, sha256: Sha256Digest) {
        if let Err(e) = self.record_written(key, size, &hex::encode(sha256)).await {
            tracing::warn!(error = %e, "durability: recording checksum failed");
        }
    }

    async fn copied(&self, src: &str, dst: &str) {
        let r = async {
            match self.get(src).await? {
                Some(ObjectRow {
                    sha256: Some(sha),
                    size,
                    state,
                    ..
                }) if state == state::OK => self.record_written(dst, size, &sha).await,
                // Source not verified: let the scrubber hash the copy.
                other => self
                    .record_seen(dst, other.map(|o| o.size).unwrap_or(0))
                    .await
                    .map(|_| ()),
            }
        }
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "durability: recording copy failed");
        }
    }

    async fn deleting(&self, key: &str) {
        let now = now_micros();
        let r = sqlx::query(
            "UPDATE durability_objects SET state = 'deleted', deleted_at = $1, updated_at = $1 WHERE object_key = $2",
        )
        .bind(now)
        .bind(key)
        .execute(&self.db.pool)
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "durability: recording delete failed");
        }
    }
}
