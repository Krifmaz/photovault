//! [`Journaled`]: wraps any [`ObjectStore`] and reports every write and
//! delete made through the trait to a [`WriteObserver`], with the SHA-256 of
//! what was written. This is how the durability catalog gets a checksum for
//! objects the server writes itself (`put_bytes`, `put_stream`, `copy`).
//! Objects the apps upload through signed URLs are reported by the local
//! backend's upload path, or found by the catalog sweep (S3).

use super::{
    ByteStream, ListedObject, MultipartUrls, ObjectStore, PartSpec, Sha256Digest, StorageResult,
    WriteObserver,
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Observer-notifying wrapper around a backend.
#[derive(Debug)]
pub struct Journaled {
    inner: Box<dyn ObjectStore>,
    observer: Arc<dyn WriteObserver>,
}

impl Journaled {
    /// Wrap `inner`, reporting to `observer`.
    pub fn new(inner: Box<dyn ObjectStore>, observer: Arc<dyn WriteObserver>) -> Self {
        Self { inner, observer }
    }
}

/// SHA-256 of a byte slice.
pub fn sha256(data: &[u8]) -> Sha256Digest {
    Sha256::digest(data).into()
}

#[async_trait::async_trait]
impl ObjectStore for Journaled {
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
            .presign_put(key, ttl, content_length, content_md5)
            .await
    }
    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        self.inner.presign_get(key, ttl).await
    }
    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        self.inner.create_multipart_with(key, parts, ttl).await
    }
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> StorageResult<()> {
        self.inner.abort_multipart(key, upload_id).await
    }
    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        self.inner.head(key).await
    }
    async fn delete(&self, key: &str) -> StorageResult<()> {
        self.observer.deleting(key).await;
        self.inner.delete(key).await
    }
    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        self.inner.copy(src, dst).await?;
        self.observer.copied(src, dst).await;
        Ok(())
    }
    async fn put_bytes(&self, key: &str, data: bytes::Bytes) -> StorageResult<()> {
        let (size, digest) = (data.len() as u64, sha256(&data));
        self.inner.put_bytes(key, data).await?;
        self.observer.written(key, size, digest).await;
        Ok(())
    }
    async fn get_bytes(&self, key: &str) -> StorageResult<bytes::Bytes> {
        self.inner.get_bytes(key).await
    }
    async fn list(&self, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
        self.inner.list(after, limit).await
    }
    async fn get_stream(&self, key: &str) -> StorageResult<ByteStream> {
        self.inner.get_stream(key).await
    }
    async fn put_stream(&self, key: &str, size: u64, body: ByteStream) -> StorageResult<()> {
        let hasher = Arc::new(Mutex::new(Sha256::new()));
        let h = hasher.clone();
        let body: ByteStream = Box::pin(body.inspect(move |c| {
            if let (Ok(c), Ok(mut h)) = (c, h.lock()) {
                h.update(c);
            }
        }));
        self.inner.put_stream(key, size, body).await?;
        let digest: Sha256Digest = hasher
            .lock()
            .map(|h| h.clone().finalize().into())
            .unwrap_or_default();
        self.observer.written(key, size, digest).await;
        Ok(())
    }
}
