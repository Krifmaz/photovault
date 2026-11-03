//! [`Metered`]: wraps any [`ObjectStore`] and records each operation in
//! [`crate::metrics::STORAGE`]. Keys and URLs are never logged or labelled.

use super::{
    ByteStream, ListedObject, MultipartUrls, ObjectStore, PartSpec, StorageError, StorageResult,
};
use crate::metrics::{STORAGE, StorageLabels};
use std::time::{Duration, Instant};

/// Metrics-recording wrapper around a backend.
#[derive(Debug)]
pub struct Metered {
    inner: Box<dyn ObjectStore>,
}

impl Metered {
    /// Wrap `inner`.
    pub fn new(inner: Box<dyn ObjectStore>) -> Self {
        Self { inner }
    }

    fn record<T>(&self, op: &'static str, start: Instant, r: &StorageResult<T>) {
        let result = match r {
            Ok(_) => "ok",
            Err(StorageError::NotFound) => "not_found",
            Err(_) => "error",
        };
        let labels = StorageLabels {
            backend: self.inner.name(),
            op,
            result,
        };
        STORAGE.ops.get_or_create(&labels).inc();
        STORAGE
            .latency
            .get_or_create(&labels)
            .observe(start.elapsed().as_secs_f64());
        if let Err(StorageError::Backend(e)) = r {
            // Backend errors never contain signed URLs or credentials (see s3::client).
            tracing::warn!(backend = self.inner.name(), op, error = %e, "storage operation failed");
        }
    }
}

macro_rules! metered {
    ($self:ident, $op:literal, $call:expr) => {{
        let start = Instant::now();
        let r = $call.await;
        $self.record($op, start, &r);
        r
    }};
}

#[async_trait::async_trait]
impl ObjectStore for Metered {
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
        metered!(
            self,
            "presign_put",
            self.inner
                .presign_put(key, ttl, content_length, content_md5)
        )
    }
    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        metered!(self, "presign_get", self.inner.presign_get(key, ttl))
    }
    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        metered!(
            self,
            "create_multipart",
            self.inner.create_multipart_with(key, parts, ttl)
        )
    }
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> StorageResult<()> {
        metered!(
            self,
            "abort_multipart",
            self.inner.abort_multipart(key, upload_id)
        )
    }
    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        metered!(self, "head", self.inner.head(key))
    }
    async fn delete(&self, key: &str) -> StorageResult<()> {
        metered!(self, "delete", self.inner.delete(key))
    }
    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        metered!(self, "copy", self.inner.copy(src, dst))
    }
    async fn put_bytes(&self, key: &str, data: bytes::Bytes) -> StorageResult<()> {
        metered!(self, "put_bytes", self.inner.put_bytes(key, data))
    }
    async fn get_bytes(&self, key: &str) -> StorageResult<bytes::Bytes> {
        metered!(self, "get_bytes", self.inner.get_bytes(key))
    }
    async fn list(&self, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
        metered!(self, "list", self.inner.list(after, limit))
    }
    async fn get_stream(&self, key: &str) -> StorageResult<ByteStream> {
        metered!(self, "get_stream", self.inner.get_stream(key))
    }
    async fn put_stream(&self, key: &str, size: u64, body: ByteStream) -> StorageResult<()> {
        metered!(self, "put_stream", self.inner.put_stream(key, size, body))
    }
}
