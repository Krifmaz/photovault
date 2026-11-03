//! S3-compatible object store with SigV4 presigning. Implemented by the storage workstream.

use super::{MultipartUrls, ObjectStore, StorageError, StorageResult};
use std::time::Duration;

/// S3 backend (placeholder until the storage workstream lands).
#[derive(Debug)]
pub struct S3Store;

impl S3Store {
    /// Build from config.
    pub fn new(_cfg: &crate::config::StorageConfig) -> anyhow::Result<Self> {
        anyhow::bail!("S3 storage is not implemented yet")
    }
}

#[async_trait::async_trait]
impl ObjectStore for S3Store {
    fn name(&self) -> &'static str {
        "s3"
    }
    async fn presign_put(
        &self,
        _: &str,
        _: Duration,
        _: Option<i64>,
        _: Option<&str>,
    ) -> StorageResult<String> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn presign_get(&self, _: &str, _: Duration) -> StorageResult<String> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn create_multipart(&self, _: &str, _: u32, _: Duration) -> StorageResult<MultipartUrls> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn head(&self, _: &str) -> StorageResult<Option<u64>> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn delete(&self, _: &str) -> StorageResult<()> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn copy(&self, _: &str, _: &str) -> StorageResult<()> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn put_bytes(&self, _: &str, _: bytes::Bytes) -> StorageResult<()> {
        Err(StorageError::Backend("unimplemented".into()))
    }
    async fn get_bytes(&self, _: &str) -> StorageResult<bytes::Bytes> {
        Err(StorageError::Backend("unimplemented".into()))
    }
}
