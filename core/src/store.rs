//! The local store: SQLite (rusqlite, bundled) in the app's data directory.
//!
//! No plaintext key or decrypted metadata is ever written: every sensitive
//! value is sealed with [`crypto::seal_local`] (secretbox, fresh nonce) under
//! the 32-byte device key in `device.key` next to the database (contract v1;
//! later wrapped by the Android Keystore). Only ids, server timestamps and
//! upload states, which the server knows anyway, are stored in the clear.

use crate::crypto;
use crate::{CoreError, Result, ServerInfo};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS servers (
  public_url TEXT PRIMARY KEY, info TEXT NOT NULL, added_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS collections (
  id INTEGER PRIMARY KEY, updation_time INTEGER NOT NULL,
  synced_until INTEGER NOT NULL DEFAULT 0, sealed BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS files (id INTEGER PRIMARY KEY, sealed BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS collection_files (
  collection_id INTEGER NOT NULL, file_id INTEGER NOT NULL,
  PRIMARY KEY (collection_id, file_id));
CREATE INDEX IF NOT EXISTS collection_files_file ON collection_files (file_id);
CREATE TABLE IF NOT EXISTS uploads (
  id INTEGER PRIMARY KEY AUTOINCREMENT, state TEXT NOT NULL,
  collection_id INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
  bytes_total INTEGER NOT NULL DEFAULT 0, sealed BLOB NOT NULL,
  error TEXT, created_at INTEGER NOT NULL);
PRAGMA user_version = 1;
";

/// Upload row states.
pub const UP_PENDING: &str = "pending";
/// In progress (or interrupted by process death).
pub const UP_UPLOADING: &str = "uploading";
/// Failed; retried by the next run.
pub const UP_FAILED: &str = "failed";
/// Finished; removed when the queue drains.
pub const UP_DONE: &str = "done";

/// The store.
pub struct Store {
    conn: Connection,
    key: Vec<u8>,
}

/// Load or create the device key.
fn device_key(dir: &Path) -> Result<Vec<u8>> {
    let path = dir.join("device.key");
    match std::fs::read(&path) {
        Ok(k) if k.len() == 32 => return Ok(k),
        Ok(_) => return Err(CoreError::storage("device.key is corrupt")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let key = crypto::generate_key();
    let tmp = dir.join("device.key.tmp");
    {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&key)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(key)
}

impl Store {
    /// Open the store in `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = device_key(dir)?;
        let conn = Connection::open(dir.join("photovault-core.sqlite"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, key })
    }

    /// The raw connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The raw connection, for transactions.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Seal a JSON-serialisable value under the device key.
    pub fn seal<T: Serialize>(&self, v: &T) -> Result<Vec<u8>> {
        let mut json = serde_json::to_vec(v)?;
        let out = crypto::seal_local(&json, &self.key);
        crypto::wipe(&mut json);
        Ok(out?)
    }

    /// Open a sealed value.
    pub fn open_sealed<T: DeserializeOwned>(&self, blob: &[u8]) -> Result<T> {
        let mut json = crypto::open_local(blob, &self.key)
            .map_err(|_| CoreError::storage("local data does not decrypt (device key changed?)"))?;
        let v = serde_json::from_slice(&json);
        crypto::wipe(&mut json);
        Ok(v?)
    }

    // ---- key/value ----

    /// Put a sealed value.
    pub fn put_sealed<T: Serialize>(&self, k: &str, v: &T) -> Result<()> {
        let blob = self.seal(v)?;
        self.conn.execute(
            "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![k, blob],
        )?;
        Ok(())
    }

    /// Get a sealed value.
    pub fn get_sealed<T: DeserializeOwned>(&self, k: &str) -> Result<Option<T>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT v FROM kv WHERE k = ?1", params![k], |r| r.get(0))
            .optional()?;
        blob.map(|b| self.open_sealed(&b)).transpose()
    }

    /// Put a plain (non-sensitive) string.
    pub fn put_plain(&self, k: &str, v: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![k, v.as_bytes()],
        )?;
        Ok(())
    }

    /// Get a plain string.
    pub fn get_plain(&self, k: &str) -> Result<Option<String>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT v FROM kv WHERE k = ?1", params![k], |r| r.get(0))
            .optional()?;
        Ok(blob.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    /// Delete a key.
    pub fn delete(&self, k: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM kv WHERE k = ?1", params![k])?;
        Ok(())
    }

    // ---- servers ----

    /// Saved servers, oldest first.
    pub fn servers(&self) -> Result<Vec<ServerInfo>> {
        let mut st = self
            .conn
            .prepare("SELECT info FROM servers ORDER BY added_at, public_url")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(i) = serde_json::from_str(&r?) {
                out.push(i);
            }
        }
        Ok(out)
    }

    /// Insert or update a saved server.
    pub fn save_server(&self, info: &ServerInfo) -> Result<()> {
        self.conn.execute(
            "INSERT INTO servers (public_url, info, added_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(public_url) DO UPDATE SET info = excluded.info",
            params![info.public_url, serde_json::to_string(info)?, now_us()],
        )?;
        Ok(())
    }

    /// Forget a server.
    pub fn remove_server(&self, public_url: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM servers WHERE public_url = ?1",
            params![public_url],
        )?;
        Ok(())
    }

    /// Wipe all account data (keeps servers and the device key).
    pub fn wipe_account(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM collections; DELETE FROM files; DELETE FROM collection_files; \
             DELETE FROM uploads; DELETE FROM kv WHERE k NOT IN ('active_server');",
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Now, µs since the epoch.
pub fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_values_roundtrip_and_are_not_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Store::open(dir.path()).unwrap();
            s.put_sealed("secret", &"very secret title").unwrap();
            assert_eq!(
                s.get_sealed::<String>("secret").unwrap().as_deref(),
                Some("very secret title")
            );
            s.put_plain("active_server", "https://a").unwrap();
        }
        let raw = std::fs::read(dir.path().join("photovault-core.sqlite")).unwrap();
        let wal = std::fs::read(dir.path().join("photovault-core.sqlite-wal")).unwrap_or_default();
        let hay = [raw, wal].concat();
        assert!(!hay.windows(17).any(|w| w == b"very secret title"));
        // Reopen with the same device key.
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(
            s.get_sealed::<String>("secret").unwrap().as_deref(),
            Some("very secret title")
        );
        assert_eq!(
            s.get_plain("active_server").unwrap().as_deref(),
            Some("https://a")
        );
    }
}
