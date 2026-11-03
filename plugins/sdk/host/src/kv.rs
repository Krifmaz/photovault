//! Per-plugin key/value storage, kept by the host in one JSON file per plugin.

use crate::fsutil::write_atomic;
use photovault_plugin_manifest::abi::{ErrorCode, HostError};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// A plugin's key/value store: a JSON object in one file, rewritten
/// atomically on every change. The serialised size of the whole object is
/// capped.
#[derive(Debug, Clone)]
pub struct KvStore {
    path: PathBuf,
    max_bytes: u64,
    max_key_bytes: usize,
}

fn internal(e: impl std::fmt::Display) -> HostError {
    HostError::new(ErrorCode::Internal, format!("plugin storage: {e}"))
}

impl KvStore {
    /// A store backed by `path` (created on first write).
    pub fn new(path: impl Into<PathBuf>, max_bytes: u64, max_key_bytes: usize) -> Self {
        Self {
            path: path.into(),
            max_bytes,
            max_key_bytes,
        }
    }

    /// The backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<Map<String, Value>, HostError> {
        match std::fs::read(&self.path) {
            Ok(b) => serde_json::from_slice(&b).map_err(internal),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
            Err(e) => Err(internal(e)),
        }
    }

    fn check_key(&self, key: &str) -> Result<(), HostError> {
        if key.is_empty() || key.len() > self.max_key_bytes {
            return Err(HostError::new(
                ErrorCode::InvalidRequest,
                format!("key must be 1-{} bytes", self.max_key_bytes),
            ));
        }
        Ok(())
    }

    /// The value under `key`, if any.
    pub fn get(&self, key: &str) -> Result<Option<Value>, HostError> {
        self.check_key(key)?;
        Ok(self.load()?.get(key).cloned())
    }

    /// Set `key` to `value`; `null` deletes. Fails with
    /// [`ErrorCode::LimitExceeded`] if the store would grow past its cap.
    pub fn set(&self, key: &str, value: Value) -> Result<(), HostError> {
        self.check_key(key)?;
        let mut map = self.load()?;
        if value.is_null() {
            if map.remove(key).is_none() {
                return Ok(());
            }
        } else {
            map.insert(key.to_owned(), value);
        }
        let bytes = serde_json::to_vec(&map).map_err(internal)?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(HostError::new(
                ErrorCode::LimitExceeded,
                format!("plugin storage is limited to {} bytes", self.max_bytes),
            ));
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(internal)?;
        }
        write_atomic(&self.path, &bytes).map_err(internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_set_delete_and_cap() {
        let d = tempfile::tempdir().unwrap();
        let kv = KvStore::new(d.path().join("p/kv.json"), 64, 8);
        assert_eq!(kv.get("a").unwrap(), None);
        kv.set("a", json!({"x": 1})).unwrap();
        assert_eq!(kv.get("a").unwrap(), Some(json!({"x": 1})));
        // Survives a new handle.
        let kv2 = KvStore::new(d.path().join("p/kv.json"), 64, 8);
        assert_eq!(kv2.get("a").unwrap(), Some(json!({"x": 1})));
        let e = kv.set("b", json!("x".repeat(100))).unwrap_err();
        assert_eq!(e.code, ErrorCode::LimitExceeded);
        assert_eq!(kv.get("b").unwrap(), None);
        assert_eq!(
            kv.set("toolongkey", json!(1)).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        kv.set("a", Value::Null).unwrap();
        assert_eq!(kv.get("a").unwrap(), None);
    }
}
