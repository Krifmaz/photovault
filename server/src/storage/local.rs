//! Local-disk object store (plain directory or a mergerfs pool) with self-signed URLs.
//!
//! URLs look like `{public}/storage/{key}?pv-exp=…&pv-sig=…` and are served by
//! [`crate::routes::objects`]. The signature is HMAC-SHA256 over the method,
//! key, expiry and (for multipart) upload id and part number, so a URL only
//! permits the one operation it was issued for.

use super::{MultipartUrls, ObjectStore, StorageError, StorageResult, validate_key};
use bytes::Bytes;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Operation a signed URL authorises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Upload a whole object, or one part when `upload_id`/`part` are set.
    Put,
    /// Download.
    Get,
    /// Complete a multipart upload.
    Complete,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Put => "PUT",
            Op::Get => "GET",
            Op::Complete => "COMPLETE",
        }
    }
}

/// Directory-backed store.
#[derive(Clone)]
pub struct LocalStore {
    root: PathBuf,
    public_url: String,
    key: Vec<u8>,
}

impl std::fmt::Debug for LocalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalStore")
            .field("root", &self.root)
            .field("public_url", &self.public_url)
            .finish()
    }
}

/// Parameters carried by a signed URL.
#[derive(Debug, Clone, Default)]
pub struct Signed {
    /// Expiry, unix seconds.
    pub exp: i64,
    /// Multipart upload id.
    pub upload_id: Option<String>,
    /// Multipart part number (1-based).
    pub part: Option<u32>,
    /// Bound content length, if any.
    pub len: Option<i64>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl LocalStore {
    /// Create a store rooted at `root`, issuing URLs under `public_url`.
    pub fn new(
        root: impl Into<PathBuf>,
        public_url: impl Into<String>,
        key: Vec<u8>,
    ) -> StorageResult<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join(".multipart"))
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(Self {
            root,
            public_url: public_url.into().trim_end_matches('/').to_string(),
            key,
        })
    }

    /// Filesystem path for `key`.
    pub fn path(&self, key: &str) -> StorageResult<PathBuf> {
        validate_key(key)?;
        Ok(self.root.join(key))
    }

    /// Directory holding parts of an in-progress multipart upload.
    pub fn multipart_dir(&self, upload_id: &str) -> StorageResult<PathBuf> {
        if upload_id.is_empty() || !upload_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StorageError::InvalidKey);
        }
        Ok(self.root.join(".multipart").join(upload_id))
    }

    fn mac(&self, op: Op, key: &str, s: &Signed) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac accepts any key length");
        let msg = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            op.as_str(),
            key,
            s.exp,
            s.upload_id.as_deref().unwrap_or(""),
            s.part.map(|p| p.to_string()).unwrap_or_default(),
            s.len.map(|p| p.to_string()).unwrap_or_default()
        );
        m.update(msg.as_bytes());
        hex::encode(m.finalize().into_bytes())
    }

    /// Build a signed URL.
    pub fn sign(&self, op: Op, key: &str, s: &Signed) -> String {
        let sig = self.mac(op, key, s);
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair("pv-exp", &s.exp.to_string());
        if let Some(u) = &s.upload_id {
            q.append_pair("uploadId", u);
        }
        if let Some(p) = s.part {
            q.append_pair("partNumber", &p.to_string());
        }
        if let Some(l) = s.len {
            q.append_pair("pv-len", &l.to_string());
        }
        if op == Op::Complete {
            q.append_pair("pv-op", "complete");
        }
        q.append_pair("pv-sig", &sig);
        let enc_key: String = key
            .split('/')
            .map(|seg| url::form_urlencoded::byte_serialize(seg.as_bytes()).collect::<String>())
            .collect::<Vec<_>>()
            .join("/");
        format!("{}/storage/{}?{}", self.public_url, enc_key, q.finish())
    }

    /// Verify a signature (constant time) and expiry.
    pub fn verify(&self, op: Op, key: &str, s: &Signed, sig: &str) -> bool {
        use subtle::ConstantTimeEq;
        if s.exp < now_secs() {
            return false;
        }
        let want = self.mac(op, key, s);
        want.as_bytes().ct_eq(sig.as_bytes()).into()
    }

    fn exp(ttl: Duration) -> i64 {
        now_secs() + ttl.as_secs() as i64
    }
}

fn io(e: std::io::Error) -> StorageError {
    if e.kind() == std::io::ErrorKind::NotFound {
        StorageError::NotFound
    } else {
        StorageError::Backend(e.to_string())
    }
}

/// Write `data` to `path` atomically (temp file + fsync + rename).
pub async fn write_atomic(path: &Path, data: &[u8]) -> StorageResult<()> {
    use tokio::io::AsyncWriteExt;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io)?;
    }
    let tmp = path.with_extension(format!(
        "tmp-{}",
        hex::encode(crate::crypto::random_bytes(6))
    ));
    let mut f = tokio::fs::File::create(&tmp).await.map_err(io)?;
    f.write_all(data).await.map_err(io)?;
    f.sync_all().await.map_err(io)?;
    drop(f);
    tokio::fs::rename(&tmp, path).await.map_err(io)
}

#[async_trait::async_trait]
impl ObjectStore for LocalStore {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        content_length: Option<i64>,
        _md5: Option<&str>,
    ) -> StorageResult<String> {
        validate_key(key)?;
        Ok(self.sign(
            Op::Put,
            key,
            &Signed {
                exp: Self::exp(ttl),
                len: content_length,
                ..Default::default()
            },
        ))
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        validate_key(key)?;
        Ok(self.sign(
            Op::Get,
            key,
            &Signed {
                exp: Self::exp(ttl),
                ..Default::default()
            },
        ))
    }

    async fn create_multipart(
        &self,
        key: &str,
        parts: u32,
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        validate_key(key)?;
        let upload_id = hex::encode(crate::crypto::random_bytes(16));
        let dir = self.multipart_dir(&upload_id)?;
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        tokio::fs::write(dir.join("key"), key.as_bytes())
            .await
            .map_err(io)?;
        let exp = Self::exp(ttl);
        let part_urls = (1..=parts)
            .map(|p| {
                self.sign(
                    Op::Put,
                    key,
                    &Signed {
                        exp,
                        upload_id: Some(upload_id.clone()),
                        part: Some(p),
                        len: None,
                    },
                )
            })
            .collect();
        let complete_url = self.sign(
            Op::Complete,
            key,
            &Signed {
                exp,
                upload_id: Some(upload_id),
                ..Default::default()
            },
        );
        Ok(MultipartUrls {
            object_key: key.to_string(),
            part_urls,
            complete_url,
        })
    }

    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        match tokio::fs::metadata(self.path(key)?).await {
            Ok(m) => Ok(Some(m.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        match tokio::fs::remove_file(self.path(key)?).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(e)),
        }
    }

    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        let (s, d) = (self.path(src)?, self.path(dst)?);
        if let Some(parent) = d.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let tmp = d.with_extension("tmp-copy");
        tokio::fs::copy(&s, &tmp).await.map_err(io)?;
        tokio::fs::rename(&tmp, &d).await.map_err(io)
    }

    async fn put_bytes(&self, key: &str, data: Bytes) -> StorageResult<()> {
        write_atomic(&self.path(key)?, &data).await
    }

    async fn get_bytes(&self, key: &str) -> StorageResult<Bytes> {
        tokio::fs::read(self.path(key)?)
            .await
            .map(Bytes::from)
            .map_err(io)
    }
}
