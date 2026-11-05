//! The local store: SQLite (rusqlite, bundled) in the app's data directory.
//!
//! No plaintext key or decrypted metadata is ever written: every sensitive
//! value is sealed with [`crypto::seal_local`] (secretbox, fresh nonce) under
//! a 32-byte device key. The apps supply that key themselves and keep it
//! wrapped by the platform keystore (Android: a non-exportable AES key in
//! the Android Keystore), so the data directory alone does not open the
//! store. Without a supplied key (tests, tools) the legacy `device.key` file
//! next to the database is used; a store found with such a file is re-sealed
//! under the supplied key and the file deleted. Only ids, server timestamps
//! and upload states, which the server knows anyway, are stored in the clear.

use crate::crypto;
use crate::{CoreError, RecentServer, Result, ServerInfo};
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
";

/// Schema steps after [`SCHEMA`] (`user_version` 1), appended by feature
/// tracks: step `i` brings the store to `user_version` `i + 2`. Keep every
/// step idempotent (`IF NOT EXISTS`): tracks merged in another order replay
/// cleanly.
const MIGRATIONS: &[&str] = &[
    // 2: streamable videos (crate::video): the account's `vid_preview` file data status.
    "CREATE TABLE IF NOT EXISTS video_streams (
       file_id INTEGER PRIMARY KEY, object_id TEXT, size INTEGER NOT NULL DEFAULT 0,
       updated_at INTEGER NOT NULL);",
    // 3: recent servers (contract v1.3): when a saved server last answered and at which address.
    // A table of its own (not new `servers` columns), so stores saved before it read unchanged.
    "CREATE TABLE IF NOT EXISTS server_seen (
       public_url TEXT PRIMARY KEY, last_connected_at INTEGER NOT NULL, last_address TEXT);",
];

/// Create the schema and run the migrations it has not seen yet.
fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    let mut v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v < 1 {
        v = 1;
        conn.pragma_update(None, "user_version", v)?;
    }
    for (i, step) in MIGRATIONS.iter().enumerate() {
        let to = i as i64 + 2;
        if v < to {
            conn.execute_batch(step)?;
            conn.pragma_update(None, "user_version", to)?;
            v = to;
        }
    }
    Ok(())
}

/// Upload row states.
pub const UP_PENDING: &str = "pending";
/// In progress (or interrupted by process death).
pub const UP_UPLOADING: &str = "uploading";
/// Failed; retried by the next run.
pub const UP_FAILED: &str = "failed";
/// Finished; removed when the queue drains.
pub const UP_DONE: &str = "done";

/// kv key of a sealed constant: tells whether a supplied key is the one the
/// store was sealed with.
const KV_KEY_CHECK: &str = "key_check";
const KEY_CHECK: &str = "photovault-store-v1";
/// The legacy device key file.
const LEGACY_KEY_FILE: &str = "device.key";

/// The store.
pub struct Store {
    conn: Connection,
    key: Vec<u8>,
}

impl Drop for Store {
    fn drop(&mut self) {
        crypto::wipe(&mut self.key);
    }
}

/// Load the legacy device key file, if there is one.
fn legacy_key(dir: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(dir.join(LEGACY_KEY_FILE)) {
        Ok(k) if k.len() == 32 => Ok(Some(k)),
        Ok(_) => Err(CoreError::storage("device.key is corrupt")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Load or create the legacy device key file.
fn device_key(dir: &Path) -> Result<Vec<u8>> {
    let path = dir.join(LEGACY_KEY_FILE);
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
    /// Open the store in `dir` with the legacy `device.key` file.
    #[cfg(test)]
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with(dir, None).map(|(s, _)| s)
    }

    /// Open the store in `dir`, sealed under `key` (the platform-supplied
    /// device key) or, without one, the legacy `device.key` file. Returns
    /// whether the account data had to be dropped because it was sealed
    /// under a key that is gone (keystore reset): the user signs in again.
    pub fn open_with(dir: &Path, key: Option<Vec<u8>>) -> Result<(Self, bool)> {
        std::fs::create_dir_all(dir)?;
        let supplied = key.is_some();
        let key = match key {
            Some(k) if k.len() == 32 => k,
            Some(mut k) => {
                crypto::wipe(&mut k);
                return Err(CoreError::storage("the device key must be 32 bytes"));
            }
            None => device_key(dir)?,
        };
        let conn = Connection::open(dir.join("photovault-core.sqlite"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        let mut store = Self { conn, key };
        let reset = if supplied {
            store.adopt_key(dir)?
        } else {
            false
        };
        Ok((store, reset))
    }

    /// Make sure everything is sealed under the supplied key: re-seal a
    /// legacy store, or drop account data sealed under a lost key.
    fn adopt_key(&mut self, dir: &Path) -> Result<bool> {
        // `Some("")`: a check value that does not open under this key.
        let check: Option<String> = self
            .get_sealed(KV_KEY_CHECK)
            .unwrap_or_else(|_| Some(String::new()));
        if check.as_deref() == Some(KEY_CHECK) {
            if legacy_key(dir)?.is_some() {
                std::fs::remove_file(dir.join(LEGACY_KEY_FILE))?;
            }
            return Ok(false);
        }
        let mut reset = false;
        match legacy_key(dir)? {
            Some(mut old) if check.is_none() => {
                let r = self.reseal_from(&old);
                crypto::wipe(&mut old);
                r?;
            }
            _ if check.is_some() || self.has_account_data()? => {
                // Sealed under a key we do not have any more.
                self.wipe_account()?;
                reset = true;
            }
            _ => {}
        }
        self.put_sealed(KV_KEY_CHECK, &KEY_CHECK)?;
        let legacy = dir.join(LEGACY_KEY_FILE);
        if legacy.exists() {
            std::fs::remove_file(legacy)?;
        }
        Ok(reset)
    }

    fn has_account_data(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM kv WHERE k = 'session')              + (SELECT COUNT(*) FROM files) + (SELECT COUNT(*) FROM collections)              + (SELECT COUNT(*) FROM uploads)",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Re-seal every sealed value from `old` to the current key. kv values
    /// that do not open under `old` are plain and kept; damaged rows of the
    /// sealed tables are dropped (the next sync fetches them again).
    fn reseal_from(&mut self, old: &[u8]) -> Result<()> {
        let mut key = self.key.clone();
        let r = reseal_tx(&mut self.conn, old, &key);
        crypto::wipe(&mut key);
        r
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
        open_sealed_with(blob, &self.key)
    }

    /// Something that opens sealed values without holding the store (and
    /// its lock): read blobs under the lock, decrypt after releasing it.
    pub fn opener(&self) -> Opener {
        Opener {
            key: self.key.clone(),
        }
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

    /// Saved servers, most recently reached first; servers never reached
    /// since recent servers exist come last, oldest first.
    pub fn recent_servers(&self) -> Result<Vec<RecentServer>> {
        let mut st = self.conn.prepare(
            "SELECT s.info, r.last_connected_at, r.last_address FROM servers s \
             LEFT JOIN server_seen r ON r.public_url = s.public_url \
             ORDER BY r.last_connected_at IS NULL, r.last_connected_at DESC, s.added_at, s.public_url",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (info, at, address) = r?;
            if let Ok(info) = serde_json::from_str(&info) {
                out.push(RecentServer {
                    info,
                    last_connected_us: at.unwrap_or(0),
                    last_address: address.filter(|a| !a.is_empty()),
                });
            }
        }
        Ok(out)
    }

    /// Remember that `address` of the saved server `public_url` answered at `at_us`.
    pub fn record_server_seen(&self, public_url: &str, address: &str, at_us: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO server_seen (public_url, last_connected_at, last_address) \
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM servers WHERE public_url = ?1) \
             ON CONFLICT(public_url) DO UPDATE SET \
             last_connected_at = excluded.last_connected_at, last_address = excluded.last_address",
            params![public_url, at_us, address],
        )?;
        Ok(())
    }

    /// The address of `public_url` that answered last.
    pub fn last_server_address(&self, public_url: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_address FROM server_seen WHERE public_url = ?1",
                params![public_url],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .filter(|a| !a.is_empty()))
    }

    /// Forget a server.
    pub fn remove_server(&self, public_url: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM servers WHERE public_url = ?1",
            params![public_url],
        )?;
        self.conn.execute(
            "DELETE FROM server_seen WHERE public_url = ?1",
            params![public_url],
        )?;
        Ok(())
    }

    /// Wipe all account data (keeps servers and the device key).
    pub fn wipe_account(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM collections; DELETE FROM files; DELETE FROM collection_files; \
             DELETE FROM uploads; DELETE FROM video_streams; \
             DELETE FROM kv WHERE k NOT IN ('active_server', 'key_check') \
             AND k NOT LIKE 'home_allowed:%';",
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn reseal_tx(conn: &mut Connection, old: &[u8], key: &[u8]) -> Result<()> {
    let tx = conn.transaction()?;
    let reseal = |blob: &[u8]| -> Option<Vec<u8>> {
        let mut plain = crypto::open_local(blob, old).ok()?;
        let out = crypto::seal_local(&plain, key).ok();
        crypto::wipe(&mut plain);
        out
    };
    let kv: Vec<(String, Vec<u8>)> = {
        let mut q = tx.prepare("SELECT k, v FROM kv")?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (k, v) in kv {
        if let Some(n) = reseal(&v) {
            tx.execute("UPDATE kv SET v = ?1 WHERE k = ?2", params![n, k])?;
        }
    }
    for table in ["collections", "files", "uploads"] {
        let rows: Vec<(i64, Vec<u8>)> = {
            let mut q = tx.prepare(&format!("SELECT id, sealed FROM {table}"))?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (id, v) in rows {
            match reseal(&v) {
                Some(n) => tx.execute(
                    &format!("UPDATE {table} SET sealed = ?1 WHERE id = ?2"),
                    params![n, id],
                )?,
                None => tx.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])?,
            };
        }
    }
    tx.commit()?;
    Ok(())
}

/// Opens sealed values; see [`Store::opener`].
pub struct Opener {
    key: Vec<u8>,
}

impl Opener {
    /// Open a sealed value.
    pub fn open_sealed<T: DeserializeOwned>(&self, blob: &[u8]) -> Result<T> {
        open_sealed_with(blob, &self.key)
    }
}

impl Drop for Opener {
    fn drop(&mut self) {
        crypto::wipe(&mut self.key);
    }
}

fn open_sealed_with<T: DeserializeOwned>(blob: &[u8], key: &[u8]) -> Result<T> {
    let mut json = crypto::open_local(blob, key)
        .map_err(|_| CoreError::storage("local data does not decrypt (device key changed?)"))?;
    let v = serde_json::from_slice(&json);
    crypto::wipe(&mut json);
    Ok(v?)
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
    fn supplied_key_reseals_a_legacy_store_and_detects_a_lost_key() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Store::open(dir.path()).unwrap();
            s.put_sealed("session", &"kept across the migration")
                .unwrap();
            s.put_plain("active_server", "https://a").unwrap();
        }
        assert!(dir.path().join(LEGACY_KEY_FILE).exists());
        let k1 = crypto::generate_key();
        {
            let (s, reset) = Store::open_with(dir.path(), Some(k1.clone())).unwrap();
            assert!(!reset);
            assert_eq!(
                s.get_sealed::<String>("session").unwrap().as_deref(),
                Some("kept across the migration")
            );
            assert_eq!(
                s.get_plain("active_server").unwrap().as_deref(),
                Some("https://a")
            );
        }
        // The key file is gone: the data directory alone no longer opens the store.
        assert!(!dir.path().join(LEGACY_KEY_FILE).exists());
        {
            let (s, reset) = Store::open_with(dir.path(), Some(k1.clone())).unwrap();
            assert!(!reset);
            assert!(s.get_sealed::<String>("session").unwrap().is_some());
        }
        // A different key (keystore reset): account data is dropped, servers kept.
        let (s, reset) = Store::open_with(dir.path(), Some(crypto::generate_key())).unwrap();
        assert!(reset);
        assert!(s.get_sealed::<String>("session").unwrap().is_none());
        assert_eq!(
            s.get_plain("active_server").unwrap().as_deref(),
            Some("https://a")
        );
        drop(s);
        assert!(Store::open_with(dir.path(), Some(vec![1; 5])).is_err());
    }

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

    fn server(public: &str, home: Option<&str>) -> ServerInfo {
        ServerInfo {
            name: "Home vault".into(),
            version: "0.2.0".into(),
            public_url: public.into(),
            home_url: home.map(str::to_string),
            signups_allowed: false,
            is_photovault: true,
        }
    }

    #[test]
    fn servers_saved_before_recent_servers_still_load() {
        let dir = tempfile::tempdir().unwrap();
        {
            // A store as an app before contract v1.3 left it: schema step 2, no server_seen table,
            // a server saved with the v1 JSON.
            let s = Store::open(dir.path()).unwrap();
            s.conn
                .execute_batch("DROP TABLE server_seen; PRAGMA user_version = 2;")
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO servers (public_url, info, added_at) VALUES (?1, ?2, 1)",
                    params![
                        "https://photos.example.com",
                        r#"{"name":"Home vault","version":"0.2.0","public_url":"https://photos.example.com","home_url":"http://192.168.1.10:8080","signups_allowed":false,"is_photovault":true}"#
                    ],
                )
                .unwrap();
        }
        let s = Store::open(dir.path()).unwrap();
        let v: i64 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 3);
        assert_eq!(
            s.servers().unwrap(),
            vec![server(
                "https://photos.example.com",
                Some("http://192.168.1.10:8080")
            )]
        );
        let recent = s.recent_servers().unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].last_connected_us, 0);
        assert_eq!(recent[0].last_address, None);
        assert_eq!(
            s.last_server_address("https://photos.example.com").unwrap(),
            None
        );
    }

    #[test]
    fn recent_servers_are_most_recent_first() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        s.save_server(&server("https://a.example.com", None))
            .unwrap();
        s.save_server(&server("https://b.example.com", None))
            .unwrap();
        s.save_server(&server("https://c.example.com", None))
            .unwrap();
        s.record_server_seen("https://a.example.com", "https://a.example.com", 10)
            .unwrap();
        s.record_server_seen("https://b.example.com", "http://100.101.102.103:8080", 20)
            .unwrap();
        // Not a saved server: not remembered.
        s.record_server_seen("https://x.example.com", "https://x.example.com", 30)
            .unwrap();
        let order = |s: &Store| {
            s.recent_servers()
                .unwrap()
                .into_iter()
                .map(|r| (r.info.public_url, r.last_connected_us, r.last_address))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            order(&s),
            vec![
                (
                    "https://b.example.com".to_string(),
                    20,
                    Some("http://100.101.102.103:8080".to_string())
                ),
                (
                    "https://a.example.com".to_string(),
                    10,
                    Some("https://a.example.com".to_string())
                ),
                ("https://c.example.com".to_string(), 0, None),
            ]
        );
        // Seen again at another address; saving the server again keeps what was seen.
        s.record_server_seen("https://a.example.com", "http://192.168.1.10:8080", 40)
            .unwrap();
        s.save_server(&server(
            "https://a.example.com",
            Some("http://192.168.1.10:8080"),
        ))
        .unwrap();
        assert_eq!(order(&s)[0].0, "https://a.example.com");
        assert_eq!(
            s.last_server_address("https://a.example.com")
                .unwrap()
                .as_deref(),
            Some("http://192.168.1.10:8080")
        );
        // Forgetting a server forgets when it was seen; wiping the account does not.
        s.remove_server("https://b.example.com").unwrap();
        assert_eq!(
            s.last_server_address("https://b.example.com").unwrap(),
            None
        );
        let mut s = s;
        s.wipe_account().unwrap();
        assert_eq!(s.recent_servers().unwrap().len(), 2);
        assert_eq!(s.recent_servers().unwrap()[0].last_connected_us, 40);
        // Saved again after removal: starts unseen.
        s.save_server(&server("https://b.example.com", None))
            .unwrap();
        assert_eq!(order(&s)[2], ("https://b.example.com".to_string(), 0, None));
    }
}
