//! Object storage behind one trait.
//!
//! Objects are always ciphertext produced by the Ente apps. Clients upload and
//! download directly using short-lived signed URLs issued by the API, exactly
//! like museum does with S3 presigned URLs. Backends:
//! * [`local::LocalStore`]: a directory (plain disk or a mergerfs pool). The
//!   server serves its own signed URLs under `/storage/…`, including an
//!   S3-compatible multipart flow so the stock apps work unchanged.
//! * [`s3::S3Store`]: any S3-compatible service, presigned with SigV4.
//!
//! [`open`] wraps the configured backend in [`metered::Metered`].

pub mod local;
pub mod metered;
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

/// URLs for one multipart upload, in museum's `MultipartUploadURLs` shape
/// (`{"objectKey","partURLs","completeURL"}`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MultipartUrls {
    /// Object key the parts assemble into.
    #[serde(rename = "objectKey")]
    pub object_key: String,
    /// One signed PUT URL per part (1-based part numbers, in order).
    #[serde(rename = "partURLs")]
    pub part_urls: Vec<String>,
    /// Signed POST URL that takes an S3 `CompleteMultipartUpload` XML body.
    #[serde(rename = "completeURL")]
    pub complete_url: String,
    /// Backend upload id. Never sent to clients; record it (museum's
    /// `temp_objects.upload_id`) so a stale upload can be aborted later with
    /// [`ObjectStore::abort_multipart`].
    #[serde(skip)]
    pub upload_id: String,
}

/// What one multipart part URL binds, like museum's `UploadPartInput`
/// `ContentLength`/`ContentMD5` (from `POST /files/multipart-upload-url`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartSpec {
    /// Exact part length in bytes.
    pub len: Option<i64>,
    /// Base64 MD5 of the part, as the client sends it in `Content-MD5`.
    pub md5: Option<String>,
}

/// Normalise a base64 MD5 like museum's `ente.NormalizeMD5`: trims whitespace,
/// requires standard base64 of exactly 16 bytes, returns it re-encoded.
pub fn normalize_md5(value: &str) -> Option<String> {
    use base64::Engine;
    let b = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    (b.len() == 16).then(|| base64::engine::general_purpose::STANDARD.encode(b))
}

/// An object store. Keys are opaque strings like `"<userID>/<uuid>"`.
#[async_trait::async_trait]
pub trait ObjectStore: Send + Sync + std::fmt::Debug {
    /// Short backend name for logs/metrics (`local`, `s3`).
    fn name(&self) -> &'static str;
    /// Signed URL the client can `PUT` the whole object to.
    /// `content_length`/`content_md5` (base64) are bound into the signature
    /// when given (museum's v2 upload URLs); the client must then send exactly
    /// that `Content-Length` and `Content-MD5`, and the body must match.
    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        content_length: Option<i64>,
        content_md5: Option<&str>,
    ) -> StorageResult<String>;
    /// Signed URL the client can `GET` (or `HEAD`, with `Range`) the object from.
    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String>;
    /// Start a multipart upload of `parts` parts and sign every part URL plus
    /// the completion URL (museum's `GET /files/multipart-upload-urls`).
    async fn create_multipart(
        &self,
        key: &str,
        parts: u32,
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        let specs = vec![PartSpec::default(); parts as usize];
        self.create_multipart_with(key, &specs, ttl).await
    }
    /// Start a multipart upload with one part URL per entry of `parts`,
    /// binding each part's length/MD5 when given (museum's
    /// `POST /files/multipart-upload-url`).
    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls>;
    /// Abort an unfinished multipart upload and drop its parts. Aborting an
    /// unknown or already completed upload is not an error.
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> StorageResult<()>;
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

/// Validate an object key: non-empty, no `..`, no leading `/`, no NUL or
/// backslash, and not under the local backend's reserved `.multipart/`.
pub fn validate_key(key: &str) -> StorageResult<()> {
    if key.is_empty()
        || key.len() > 1024
        || key.starts_with('/')
        || key.starts_with(".multipart/")
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

/// Open the configured backend, wrapped in [`metered::Metered`] so every
/// operation shows up in `/metrics`. `sign_key` signs local-store URLs.
pub fn open(
    config: &crate::config::Config,
    sign_key: &[u8],
) -> anyhow::Result<Box<dyn ObjectStore>> {
    let inner: Box<dyn ObjectStore> = match &config.storage {
        crate::config::StorageConfig::Local { path, public_url } => {
            let base = public_url
                .clone()
                .unwrap_or_else(|| config.public_url.clone());
            Box::new(local::LocalStore::new(path, base, sign_key.to_vec())?)
        }
        crate::config::StorageConfig::S3(cfg) => Box::new(s3::S3Store::new(cfg)?),
    };
    Ok(Box::new(metered::Metered::new(inner)))
}

/// Start background storage housekeeping for `serve`: with the local backend,
/// hourly removal of stale multipart uploads and orphaned temp files
/// ([`local::LocalStore::cleanup_stale`]). S3 needs nothing here: abandoned
/// uploads are aborted by the files workstream's temp-object cleanup via
/// [`ObjectStore::abort_multipart`] (or a bucket lifecycle rule).
pub fn spawn_maintenance(state: &crate::state::AppState) {
    let Some(local) = state.local_store.clone() else {
        return;
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let l = local.clone();
            match tokio::task::spawn_blocking(move || l.cleanup_stale()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(
                    removed = n,
                    "local storage: removed stale uploads/temp files"
                ),
                Err(e) => tracing::warn!(error = %e, "local storage cleanup task failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{normalize_md5, validate_key};

    #[test]
    fn rejects_traversal() {
        assert!(validate_key("1/abc").is_ok());
        assert!(validate_key("../x").is_err());
        assert!(validate_key("/x").is_err());
        assert!(validate_key("a//b").is_err());
        assert!(validate_key("a\\b").is_err());
        assert!(validate_key(".multipart/x").is_err());
    }

    #[test]
    fn md5_normalisation() {
        assert_eq!(
            normalize_md5(" XUFAKrxLKna5cZ2REBfFkg==\n").as_deref(),
            Some("XUFAKrxLKna5cZ2REBfFkg==")
        );
        assert!(normalize_md5("aGVsbG8=").is_none());
        assert!(normalize_md5("not-md5").is_none());
    }
}
