//! Object storage behind one trait.
//!
//! Objects are always ciphertext produced by the Ente apps. Clients upload and
//! download directly using short-lived signed URLs issued by the API, exactly
//! like museum does with S3 presigned URLs. Backends:
//! * [`local::LocalStore`]: a directory (plain disk or a mergerfs pool). The
//!   server serves its own signed URLs under `/storage/…`, including an
//!   S3-compatible multipart flow so the stock apps work unchanged.
//! * [`s3::S3Store`]: any S3-compatible service, presigned with SigV4.

pub mod local;
pub mod s3;

use std::time::Duration;

/// Errors from a storage backend.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Object does not exist.
    #[error("object not found")]
    NotFound,
    /// Key failed validation (e.g. path traversal).
    #[error("invalid object key")]
    InvalidKey,
    /// I/O or remote error.
    #[error("storage backend error: {0}")]
    Backend(String),
}

/// Result alias.
pub type StorageResult<T> = Result<T, StorageError>;

/// URLs for one multipart upload, in museum's `MultipartUploadURLs` shape.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartUrls {
    /// Object key the parts assemble into.
    pub object_key: String,
    /// One signed PUT URL per part (1-based part numbers, in order).
    pub part_urls: Vec<String>,
    /// Signed POST URL that takes an S3 `CompleteMultipartUpload` XML body.
    pub complete_url: String,
}

/// An object store. Keys are opaque strings like `"<userID>/<uuid>"`.
#[async_trait::async_trait]
pub trait ObjectStore: Send + Sync + std::fmt::Debug {
    /// Short backend name for logs/metrics (`local`, `s3`).
    fn name(&self) -> &'static str;
    /// Signed URL the client can `PUT` the whole object to.
    /// `content_length`/`content_md5` are bound into the signature when given (museum's v2 upload URLs).
    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        content_length: Option<i64>,
        content_md5: Option<&str>,
    ) -> StorageResult<String>;
    /// Signed URL the client can `GET` the object from.
    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String>;
    /// Start a multipart upload of `parts` parts and sign every part URL plus the completion URL.
    async fn create_multipart(
        &self,
        key: &str,
        parts: u32,
        ttl: Duration,
    ) -> StorageResult<MultipartUrls>;
    /// Size in bytes, or `None` when the object does not exist.
    async fn head(&self, key: &str) -> StorageResult<Option<u64>>;
    /// Delete an object. Deleting a missing object is not an error.
    async fn delete(&self, key: &str) -> StorageResult<()>;
    /// Server-side copy (used by `/files/copy`).
    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()>;
    /// Write a whole object from memory (server-generated objects, tests).
    async fn put_bytes(&self, key: &str, data: bytes::Bytes) -> StorageResult<()>;
    /// Read a whole object into memory (tests, small objects).
    async fn get_bytes(&self, key: &str) -> StorageResult<bytes::Bytes>;
}

/// Validate an object key: non-empty, no `..`, no leading `/`, no NUL or backslash.
pub fn validate_key(key: &str) -> StorageResult<()> {
    if key.is_empty()
        || key.len() > 1024
        || key.starts_with('/')
        || key.contains('\0')
        || key.contains('\\')
        || key
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(StorageError::InvalidKey);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_key;

    #[test]
    fn rejects_traversal() {
        assert!(validate_key("1/abc").is_ok());
        assert!(validate_key("../x").is_err());
        assert!(validate_key("/x").is_err());
        assert!(validate_key("a//b").is_err());
        assert!(validate_key("a\\b").is_err());
    }
}

/// Open the configured backend. `sign_key` signs local-store URLs.
pub fn open(
    config: &crate::config::Config,
    sign_key: &[u8],
) -> anyhow::Result<Box<dyn ObjectStore>> {
    match &config.storage {
        crate::config::StorageConfig::Local { path, public_url } => {
            let base = public_url
                .clone()
                .unwrap_or_else(|| config.public_url.clone());
            Ok(Box::new(local::LocalStore::new(
                path,
                base,
                sign_key.to_vec(),
            )?))
        }
        crate::config::StorageConfig::S3 { .. } => Ok(Box::new(s3::S3Store::new(&config.storage)?)),
    }
}
