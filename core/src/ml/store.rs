//! Local storage of ML data: tables added to the core store by
//! [`migrate`] (run on every open after the base schema, idempotent).
//!
//! Like the rest of the store, everything derived from a user's photos is
//! sealed under the device key: `mldata` objects (face boxes and
//! embeddings, CLIP embeddings) and user entities (people names, cluster
//! assignments). Only file ids, server timestamps, version booleans and
//! indexing states, which the server knows anyway, are kept in the clear.
//!
//! Tables:
//!
//! - `ml_data(file_id, updated_at, face_ok, clip_ok, sealed)`: the decrypted
//!   `mldata` object of a file (sealed JSON object, unknown keys kept), the
//!   server's `updatedAt` (`NULL` while only local), and whether the face
//!   and CLIP indexes are current ([`RemoteMlData::plan`]).
//! - `ml_local(file_id, state, attempts, error, updated_at)`: on-device
//!   indexing state of files that had no current remote `mldata`.
//! - `ml_entities(type, id, updated_at, sealed)`: decrypted user entities
//!   (`cgroup`, `location`, …) as sealed JSON `data`.
//!
//! kv keys used by the ML modules all start with `ml:` (see [`kv`]).

use super::mldata::RemoteMlData;
use crate::store::{Opener, Store};
use crate::{CoreError, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ml_data (
  file_id INTEGER PRIMARY KEY, updated_at INTEGER,
  face_ok INTEGER NOT NULL, clip_ok INTEGER NOT NULL, sealed BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS ml_local (
  file_id INTEGER PRIMARY KEY, state TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, error TEXT, updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS ml_entities (
  type TEXT NOT NULL, id TEXT NOT NULL, updated_at INTEGER NOT NULL,
  sealed BLOB NOT NULL, PRIMARY KEY (type, id));
";

/// kv keys (all under `ml:`). Values marked sealed go through
/// [`Store::put_sealed`], the others through [`Store::put_plain`].
pub mod kv {
    /// Plain `"1"`/`"0"`: ML (faces + magic search) is enabled on this device.
    pub const ENABLED: &str = "ml:enabled";
    /// Plain i64: `/files/data/status-diff` cursor (`lastUpdatedAt`).
    pub const STATUS_SINCE: &str = "ml:status_since";
    /// Sealed bytes: the decrypted user-entity key of an entity type;
    /// full key is `ml:entity_key:<type>`.
    pub const ENTITY_KEY_PREFIX: &str = "ml:entity_key:";
    /// Plain i64: user-entity diff cursor; full key `ml:entity_since:<type>`.
    pub const ENTITY_SINCE_PREFIX: &str = "ml:entity_since:";
    /// Sealed: local face clustering result (owned by `faces`).
    pub const CLUSTERS: &str = "ml:clusters";
    /// Sealed: memories already shown (owned by `memories`).
    pub const MEMORIES_SEEN: &str = "ml:memories_seen";
}

/// Local indexing states (`ml_local.state`).
pub mod local_state {
    /// Waiting to be indexed on this device.
    pub const PENDING: &str = "pending";
    /// Indexed here; `mldata` not yet uploaded.
    pub const INDEXED: &str = "indexed";
    /// Indexed and uploaded.
    pub const DONE: &str = "done";
    /// Failed; retried until `attempts` reaches the indexer's limit.
    pub const FAILED: &str = "failed";
    /// Not indexable (unsupported type, too large, …).
    pub const SKIPPED: &str = "skipped";
}

/// Create the ML tables. Called from [`Store::open_with`] after the base
/// schema.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Delete all ML rows (logout / lost device key). kv rows go with the
/// account's kv wipe.
pub(crate) fn wipe(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM ml_data; DELETE FROM ml_local; DELETE FROM ml_entities;")?;
    Ok(())
}

/// One stored `mldata` row, still sealed. Open it with [`MlRow::open`]
/// after releasing the store lock.
pub(crate) struct MlRow {
    pub file_id: i64,
    pub updated_at: Option<i64>,
    pub face_ok: bool,
    pub clip_ok: bool,
    sealed: Vec<u8>,
}

impl MlRow {
    /// Decrypt into a [`RemoteMlData`] (validates the known indexes).
    pub fn open(&self, opener: &Opener) -> Result<RemoteMlData> {
        let raw: Map<String, Value> = opener.open_sealed(&self.sealed)?;
        let mut d = RemoteMlData::from_json(&serde_json::to_vec(&raw)?, self.updated_at)
            .map_err(|e| CoreError::storage(format!("stored mldata: {e}")))?;
        d.updated_at = self.updated_at;
        Ok(d)
    }
}

impl Store {
    /// Save the (decrypted) `mldata` of a file.
    pub(crate) fn put_ml_data(&self, file_id: i64, d: &RemoteMlData) -> Result<()> {
        let plan = d.plan();
        let sealed = self.seal(&d.raw)?;
        self.conn().execute(
            "INSERT INTO ml_data (file_id, updated_at, face_ok, clip_ok, sealed) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(file_id) DO UPDATE SET \
             updated_at = excluded.updated_at, face_ok = excluded.face_ok, \
             clip_ok = excluded.clip_ok, sealed = excluded.sealed",
            params![
                file_id,
                d.updated_at,
                !plan.run_faces,
                !plan.run_clip,
                sealed
            ],
        )?;
        Ok(())
    }

    /// The `mldata` of one file.
    pub(crate) fn ml_data(&self, file_id: i64) -> Result<Option<RemoteMlData>> {
        let row = self
            .conn()
            .query_row(
                "SELECT file_id, updated_at, face_ok, clip_ok, sealed FROM ml_data WHERE file_id = ?1",
                params![file_id],
                row_from,
            )
            .optional()?;
        row.map(|r| r.open(&self.opener())).transpose()
    }

    /// Every stored `mldata` row, sealed (decrypt outside the lock).
    pub(crate) fn ml_rows(&self) -> Result<Vec<MlRow>> {
        let mut q = self
            .conn()
            .prepare("SELECT file_id, updated_at, face_ok, clip_ok, sealed FROM ml_data")?;
        let rows = q
            .query_map([], row_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Forget the `mldata` of a file.
    pub(crate) fn delete_ml_data(&self, file_id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM ml_data WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Set the local indexing state of a file.
    pub(crate) fn set_ml_local(
        &self,
        file_id: i64,
        state: &str,
        attempts: u32,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO ml_local (file_id, state, attempts, error, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(file_id) DO UPDATE SET \
             state = excluded.state, attempts = excluded.attempts, \
             error = excluded.error, updated_at = excluded.updated_at",
            params![file_id, state, attempts, error, crate::store::now_us()],
        )?;
        Ok(())
    }

    /// The local indexing state and attempts of a file.
    pub(crate) fn ml_local(&self, file_id: i64) -> Result<Option<(String, u32)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT state, attempts FROM ml_local WHERE file_id = ?1",
                params![file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Save a decrypted user entity (`data` is the entity's JSON).
    pub(crate) fn put_entity(
        &self,
        kind: &str,
        id: &str,
        updated_at: i64,
        data: &Value,
    ) -> Result<()> {
        let sealed = self.seal(data)?;
        self.conn().execute(
            "INSERT INTO ml_entities (type, id, updated_at, sealed) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(type, id) DO UPDATE SET updated_at = excluded.updated_at, \
             sealed = excluded.sealed",
            params![kind, id, updated_at, sealed],
        )?;
        Ok(())
    }

    /// Forget a user entity.
    pub(crate) fn delete_entity(&self, kind: &str, id: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM ml_entities WHERE type = ?1 AND id = ?2",
            params![kind, id],
        )?;
        Ok(())
    }

    /// Every user entity of a type: `(id, updated_at, data)`.
    pub(crate) fn entities(&self, kind: &str) -> Result<Vec<(String, i64, Value)>> {
        let rows: Vec<(String, i64, Vec<u8>)> = {
            let mut q = self
                .conn()
                .prepare("SELECT id, updated_at, sealed FROM ml_entities WHERE type = ?1")?;
            q.query_map(params![kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        rows.into_iter()
            .map(|(id, t, b)| Ok((id, t, self.open_sealed(&b)?)))
            .collect()
    }
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<MlRow> {
    Ok(MlRow {
        file_id: r.get(0)?,
        updated_at: r.get(1)?,
        face_ok: r.get(2)?,
        clip_ok: r.get(3)?,
        sealed: r.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ml::mldata::RemoteMlData;
    use serde_json::json;

    #[test]
    fn ml_rows_are_sealed_and_round_trip() {
        crate::crypto::init();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let raw = json!({"clip": {"version": 1, "client": "c", "embedding": [0.5]}, "x": 1});
        let d = RemoteMlData::from_json(raw.to_string().as_bytes(), Some(7)).unwrap();
        store.put_ml_data(42, &d).unwrap();
        let back = store.ml_data(42).unwrap().unwrap();
        assert_eq!(back.raw, d.raw);
        assert_eq!(back.updated_at, Some(7));
        let rows = store.ml_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].clip_ok && !rows[0].face_ok);
        // Nothing readable in the clear.
        let blob: Vec<u8> = store
            .conn()
            .query_row("SELECT sealed FROM ml_data", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("embedding"));

        store
            .put_entity("cgroup", "p1", 5, &json!({"name": "Ana"}))
            .unwrap();
        assert_eq!(store.entities("cgroup").unwrap()[0].2["name"], "Ana");
        wipe(store.conn()).unwrap();
        assert!(store.ml_rows().unwrap().is_empty());
        assert!(store.entities("cgroup").unwrap().is_empty());
    }
}
