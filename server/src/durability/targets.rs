//! Replication targets: a second local directory and/or S3-compatible
//! services, each an [`ObjectStore`] holding byte-for-byte copies of the
//! primary's objects (ciphertext) under the same keys, plus encrypted
//! database snapshots under [`DB_PREFIX`].

use super::config::{DurabilityConfig, TargetConfig, TargetKind};
use crate::storage::{
    ByteStream, ListedObject, MultipartUrls, ObjectStore, PartSpec, StorageResult,
};
use std::sync::Arc;
use std::time::Duration;

/// Key prefix for database snapshots on targets. Never a valid object key
/// on the primary (object keys start with a user id).
pub const DB_PREFIX: &str = "_photovault/db/";

/// One opened target.
#[derive(Debug)]
pub struct Target {
    /// Name from the config.
    pub name: String,
    /// Its configuration.
    pub config: TargetConfig,
    /// The store copies are written to.
    pub store: Arc<dyn ObjectStore>,
}

/// Open every configured target. Names must be unique and non-empty.
pub fn open_all(cfg: &DurabilityConfig) -> anyhow::Result<Vec<Arc<Target>>> {
    let mut out: Vec<Arc<Target>> = Vec::new();
    for t in &cfg.targets {
        anyhow::ensure!(
            !t.name.is_empty()
                && t.name.len() <= 64
                && t.name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "durability target name {:?} must be 1-64 letters, digits, - or _",
            t.name
        );
        anyhow::ensure!(
            out.iter().all(|o| o.name != t.name),
            "durability target name {:?} is used twice",
            t.name
        );
        out.push(Arc::new(open(t)?));
    }
    Ok(out)
}

/// Open one target.
pub fn open(t: &TargetConfig) -> anyhow::Result<Target> {
    let store: Arc<dyn ObjectStore> = match &t.kind {
        TargetKind::Local { path } => Arc::new(crate::storage::local::LocalStore::new(
            path,
            String::new(),
            crate::crypto::random_bytes(32),
        )?),
        TargetKind::S3 { s3, prefix } => {
            let inner: Arc<dyn ObjectStore> = Arc::new(crate::storage::s3::S3Store::new(s3)?);
            if prefix.is_empty() {
                inner
            } else {
                Arc::new(Prefixed::new(inner, prefix))
            }
        }
    };
    Ok(Target {
        name: t.name.clone(),
        config: t.clone(),
        store,
    })
}

/// Puts every key under a fixed prefix (several servers or other data in
/// one bucket). Only the operations replication uses are meaningful;
/// presigning is passed through with the prefixed key.
#[derive(Debug)]
pub struct Prefixed {
    inner: Arc<dyn ObjectStore>,
    prefix: String,
}

impl Prefixed {
    /// Wrap `inner`; `prefix` gets a trailing `/` if it lacks one.
    pub fn new(inner: Arc<dyn ObjectStore>, prefix: &str) -> Self {
        let mut prefix = prefix.trim_start_matches('/').to_string();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Self { inner, prefix }
    }

    fn k(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }
}

#[async_trait::async_trait]
impl ObjectStore for Prefixed {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        content_length: Option<i64>,
        content_md5: Option<&str>,
    ) -> StorageResult<String> {
        self.inner
            .presign_put(&self.k(key), ttl, content_length, content_md5)
            .await
    }
    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        self.inner.presign_get(&self.k(key), ttl).await
    }
    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        self.inner
            .create_multipart_with(&self.k(key), parts, ttl)
            .await
    }
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> StorageResult<()> {
        self.inner.abort_multipart(&self.k(key), upload_id).await
    }
    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        self.inner.head(&self.k(key)).await
    }
    async fn delete(&self, key: &str) -> StorageResult<()> {
        self.inner.delete(&self.k(key)).await
    }
    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        self.inner.copy(&self.k(src), &self.k(dst)).await
    }
    async fn put_bytes(&self, key: &str, data: bytes::Bytes) -> StorageResult<()> {
        self.inner.put_bytes(&self.k(key), data).await
    }
    async fn get_bytes(&self, key: &str) -> StorageResult<bytes::Bytes> {
        self.inner.get_bytes(&self.k(key)).await
    }
    async fn list(&self, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
        // Listing starts after the prefix itself when no cursor is given, and
        // stops at the first key outside the prefix.
        let start = match after {
            Some(a) => self.k(a),
            None => self.prefix.clone(),
        };
        let page = self.inner.list(Some(&start), limit).await?;
        Ok(page
            .into_iter()
            .map_while(|o| {
                o.key.strip_prefix(&self.prefix).map(|k| ListedObject {
                    key: k.to_string(),
                    size: o.size,
                })
            })
            .collect())
    }
    async fn get_stream(&self, key: &str) -> StorageResult<ByteStream> {
        self.inner.get_stream(&self.k(key)).await
    }
    async fn put_stream(&self, key: &str, size: u64, body: ByteStream) -> StorageResult<()> {
        self.inner.put_stream(&self.k(key), size, body).await
    }
}
