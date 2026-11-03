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

pub mod journaled;
pub mod local;
pub mod metered;
pub mod s3;

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// A stream of object bytes (streaming reads and writes of large objects).
pub type ByteStream =
    Pin<Box<dyn futures_util::Stream<Item = std::io::Result<bytes::Bytes>> + Send>>;

/// One object returned by [`ObjectStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedObject {
    /// Object key.
    pub key: String,
    /// Size in bytes.
    pub size: u64,
}

/// SHA-256 of an object.
pub type Sha256Digest = [u8; 32];

/// Told about every object write and delete the server performs, so the
/// durability catalog can record checksums at write time
/// ([`crate::durability::catalog::Catalog`] implements it). Calls are awaited
/// inline; implementations log their own errors and never fail the write.
#[async_trait::async_trait]
pub trait WriteObserver: Send + Sync + std::fmt::Debug {
    /// `key` now holds `size` bytes with SHA-256 `sha256`.
    async fn written(&self, key: &str, size: u64, sha256: Sha256Digest);
    /// `dst` is now a copy of `src`.
    async fn copied(&self, src: &str, dst: &str);
    /// `key` is about to be deleted on purpose (called before the delete,
    /// so a crash in between never looks like data loss).
    async fn deleting(&self, key: &str);
}

/// Chooses where the local backend creates **new** objects. Used by the
/// FSView integration to keep writes off mergerfs branches that are failing
/// ([`crate::durability::fsview`]).
pub trait Placement: Send + Sync + std::fmt::Debug {
    /// Directory corresponding to the store root on a healthy branch to
    /// create a new object in, or `None` to write through the pool as usual.
    fn write_root(&self) -> Option<PathBuf>;
}

/// Shared handle to an optional [`Placement`], settable after the store is built.
pub type PlacementSlot = Arc<std::sync::RwLock<Option<Arc<dyn Placement>>>>;

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
    /// List objects with keys strictly greater than `after` in byte order,
    /// at most `limit`, sorted. Internal files (temp files, multipart parts)
    /// are never listed.
    async fn list(&self, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
        let _ = (after, limit);
        Err(StorageError::Backend(format!(
            "{} backend cannot list objects",
            self.name()
        )))
    }
    /// Stream an object's bytes. The default reads it into memory.
    async fn get_stream(&self, key: &str) -> StorageResult<ByteStream> {
        let b = self.get_bytes(key).await?;
        Ok(Box::pin(futures_util::stream::once(async move { Ok(b) })))
    }
    /// Write an object of exactly `size` bytes from a stream, atomically
    /// (readers never see a partial object). The default buffers in memory.
    async fn put_stream(&self, key: &str, size: u64, mut body: ByteStream) -> StorageResult<()> {
        use futures_util::StreamExt;
        let mut buf = bytes::BytesMut::with_capacity(size.min(64 << 20) as usize);
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk.map_err(|e| StorageError::Backend(e.to_string()))?);
        }
        if buf.len() as u64 != size {
            return Err(StorageError::Backend("size mismatch".into()));
        }
        self.put_bytes(key, buf.freeze()).await
    }
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

/// Open the configured backend, wrapped in [`journaled::Journaled`] (write
/// checksums for the durability catalog, when `observer` is given) and
/// [`metered::Metered`] so every operation shows up in `/metrics`.
/// `sign_key` signs local-store URLs; `local` is the local store to use
/// (shared with [`crate::state::Inner::local_store`]) when that backend is active.
pub fn open(
    config: &crate::config::Config,
    local: Option<local::LocalStore>,
    observer: Option<Arc<dyn WriteObserver>>,
) -> anyhow::Result<Box<dyn ObjectStore>> {
    let mut inner: Box<dyn ObjectStore> = match (&config.storage, local) {
        (crate::config::StorageConfig::Local { .. }, Some(l)) => Box::new(l),
        (crate::config::StorageConfig::Local { .. }, None) => {
            anyhow::bail!("local storage configured but no local store given")
        }
        (crate::config::StorageConfig::S3(cfg), _) => Box::new(s3::S3Store::new(cfg)?),
    };
    if let Some(o) = observer {
        inner = Box::new(journaled::Journaled::new(inner, o));
    }
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
