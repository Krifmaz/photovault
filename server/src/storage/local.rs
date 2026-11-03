//! Local-disk object store (plain directory or a mergerfs pool) with self-signed URLs.
//!
//! URLs look like `{public}/storage/{key}?pv-exp=…&pv-sig=…` and are served by
//! [`crate::routes::objects`]. The signature is HMAC-SHA256 over the
//! operation, key, expiry, multipart upload id and part number, and any bound
//! length/MD5, so a URL only permits the one operation it was issued for.
//!
//! # Durability and mergerfs
//! Every write goes to a hidden temp file **in the target's own directory**
//! (`.<name>.pv-tmp-<random>`), is `fsync`ed, renamed over the target (an
//! atomic `rename(2)` within one directory, which mergerfs performs on the
//! branch holding the temp file), and then the directory itself is `fsync`ed
//! so the rename survives a power cut. Filesystems that refuse a directory
//! `fsync` (some FUSE setups) are tolerated.
//!
//! In-progress multipart uploads live in `<root>/.multipart/<uploadId>/`
//! (`key`, `part-00001`, `part-00001.md5`, …). Completion streams the parts
//! into the final object; nothing is held in memory. [`LocalStore::cleanup_stale`]
//! removes uploads older than [`STALE_MULTIPART_AGE`] and orphaned temp files.

use super::{
    ByteStream, ListedObject, MultipartUrls, ObjectStore, PartSpec, PlacementSlot, Sha256Digest,
    StorageError, StorageResult, WriteObserver, validate_key,
};
use base64::Engine;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Multipart uploads untouched for this long are deleted by
/// [`LocalStore::cleanup_stale`]. Longer than the 7-day validity museum gives
/// part URLs, so no URL that could still be used ever loses its upload.
pub const STALE_MULTIPART_AGE: Duration = Duration::from_secs(8 * 24 * 3600);
/// Orphaned temp files (from a crash mid-write) older than this are deleted.
pub const STALE_TMP_AGE: Duration = Duration::from_secs(24 * 3600);
/// Marker in temp file names; keys containing it are rejected.
const TMP_MARK: &str = ".pv-tmp-";
const MULTIPART_DIR: &str = ".multipart";

/// Operation a signed URL authorises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Upload a whole object, or one part when `upload_id`/`part` are set.
    Put,
    /// Download (`GET`/`HEAD`).
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
    observer: Option<Arc<dyn WriteObserver>>,
    placement: PlacementSlot,
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
    /// Bound base64 Content-MD5, if any.
    pub md5: Option<String>,
}

/// Why a streamed write or a multipart completion was refused. Mirrors the
/// S3 error codes the apps would get from a real bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// Body longer or shorter than the bound/declared length (`IncompleteBody`/`BadRequest`).
    LengthMismatch,
    /// Body MD5 differs from `Content-MD5` (`BadDigest`).
    BadDigest,
    /// `Content-MD5` header is not a base64 16-byte digest (`InvalidDigest`).
    InvalidDigest,
    /// Client disconnected or sent a malformed body.
    Body,
    /// Upload id unknown or already completed/aborted (`NoSuchUpload`).
    NoSuchUpload,
    /// A listed part is missing or its ETag does not match (`InvalidPart`).
    InvalidPart,
    /// Parts not in ascending order (`InvalidPartOrder`).
    InvalidPartOrder,
    /// Invalid object key.
    InvalidKey,
    /// Disk error.
    Io(String),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e.to_string())
    }
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
        std::fs::create_dir_all(root.join(MULTIPART_DIR))
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(Self {
            root,
            public_url: public_url.into().trim_end_matches('/').to_string(),
            key,
            observer: None,
            placement: PlacementSlot::default(),
        })
    }

    /// Report uploads completed through signed URLs ([`Self::write_object`],
    /// [`Self::complete_multipart`]) to `observer`, with their SHA-256.
    pub fn with_observer(mut self, observer: Arc<dyn WriteObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Share `slot` as this store's write placement (clones of a store share
    /// one slot; pass the same slot to every store over the same root).
    pub fn with_placement_slot(mut self, slot: PlacementSlot) -> Self {
        self.placement = slot;
        self
    }

    /// The write placement slot ([`super::Placement`]).
    pub fn placement_slot(&self) -> PlacementSlot {
        self.placement.clone()
    }

    /// Where a write of `key` should land: the existing object (overwrites
    /// stay in place), else under the placement's chosen branch root, else
    /// under the store root.
    pub fn write_path(&self, key: &str) -> StorageResult<PathBuf> {
        let pooled = self.path(key)?;
        if pooled.exists() {
            return Ok(pooled);
        }
        let chosen = self
            .placement
            .read()
            .ok()
            .and_then(|p| p.as_ref().and_then(|p| p.write_root()));
        Ok(match chosen {
            Some(root) => root.join(key),
            None => pooled,
        })
    }

    async fn notify_written(&self, key: &str, size: u64, sha256: Sha256Digest) {
        if let Some(o) = &self.observer {
            o.written(key, size, sha256).await;
        }
    }

    /// Root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Filesystem path for `key`.
    pub fn path(&self, key: &str) -> StorageResult<PathBuf> {
        validate_key(key)?;
        if key.contains(TMP_MARK) {
            return Err(StorageError::InvalidKey);
        }
        Ok(self.root.join(key))
    }

    /// Directory holding parts of an in-progress multipart upload.
    pub fn multipart_dir(&self, upload_id: &str) -> StorageResult<PathBuf> {
        if upload_id.is_empty()
            || upload_id.len() > 64
            || !upload_id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(StorageError::InvalidKey);
        }
        Ok(self.root.join(MULTIPART_DIR).join(upload_id))
    }

    fn mac(&self, op: Op, key: &str, s: &Signed) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac accepts any key length");
        let msg = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            op.as_str(),
            key,
            s.exp,
            s.upload_id.as_deref().unwrap_or(""),
            s.part.map(|p| p.to_string()).unwrap_or_default(),
            s.len.map(|p| p.to_string()).unwrap_or_default(),
            s.md5.as_deref().unwrap_or(""),
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
        if let Some(m) = &s.md5 {
            q.append_pair("pv-md5", m);
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

    /// Stream `body` into `key`, enforcing a bound length and MD5 (base64)
    /// when given. Returns the body's MD5 (the S3 ETag, unquoted hex).
    pub async fn write_object<S, E>(
        &self,
        key: &str,
        body: S,
        want_len: Option<i64>,
        want_md5: Option<&str>,
    ) -> Result<String, WriteError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        let target = self.write_path(key).map_err(|_| WriteError::InvalidKey)?;
        let w = write_stream(&target, body, want_len, want_md5).await?;
        self.notify_written(key, w.len, w.sha256).await;
        Ok(w.md5_hex)
    }

    /// Stream one multipart part to disk. Returns its MD5 hex (the part ETag).
    pub async fn write_part<S, E>(
        &self,
        upload_id: &str,
        part: u32,
        body: S,
        want_len: Option<i64>,
        want_md5: Option<&str>,
    ) -> Result<String, WriteError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        if part == 0 || part > 10_000 {
            return Err(WriteError::InvalidPart);
        }
        let dir = self
            .multipart_dir(upload_id)
            .map_err(|_| WriteError::NoSuchUpload)?;
        if !tokio::fs::try_exists(dir.join("key"))
            .await
            .unwrap_or(false)
        {
            return Err(WriteError::NoSuchUpload);
        }
        let etag = write_stream(&part_path(&dir, part), body, want_len, want_md5)
            .await?
            .md5_hex;
        // The sidecar is written after the part; a part without one is re-hashed at completion.
        write_atomic(&part_md5_path(&dir, part), etag.as_bytes())
            .await
            .map_err(|e| WriteError::Io(e.to_string()))?;
        Ok(etag)
    }

    /// Assemble a multipart upload from `parts` (`(partNumber, ETag)` as listed
    /// in the client's `CompleteMultipartUpload` XML; an empty list means every
    /// uploaded part in order). Streams parts into the object, then deletes
    /// the upload. Returns the S3-style multipart ETag (`<md5-of-md5s>-<n>`).
    pub async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, Option<String>)],
    ) -> Result<String, WriteError> {
        let target = self.write_path(key).map_err(|_| WriteError::InvalidKey)?;
        let dir = self
            .multipart_dir(upload_id)
            .map_err(|_| WriteError::NoSuchUpload)?;
        match tokio::fs::read(dir.join("key")).await {
            Ok(k) if k == key.as_bytes() => {}
            Ok(_) => return Err(WriteError::NoSuchUpload),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(WriteError::NoSuchUpload);
            }
            Err(e) => return Err(e.into()),
        }
        let mut list: Vec<(u32, Option<String>)> = parts.to_vec();
        if list.is_empty() {
            let mut rd = tokio::fs::read_dir(&dir).await?;
            while let Some(e) = rd.next_entry().await? {
                if let Some(n) = e
                    .file_name()
                    .to_str()
                    .and_then(|n| n.strip_prefix("part-"))
                    .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|n| n.parse().ok())
                {
                    list.push((n, None));
                }
            }
            list.sort_unstable();
            if list.is_empty() {
                return Err(WriteError::InvalidPart);
            }
        }
        if list.windows(2).any(|w| w[0].0 >= w[1].0) {
            return Err(WriteError::InvalidPartOrder);
        }
        // Validate every part (exists, ETag matches) before writing anything.
        let mut md5s = Vec::with_capacity(list.len() * 16);
        for (n, etag) in &list {
            let p = part_path(&dir, *n);
            if !tokio::fs::try_exists(&p).await.unwrap_or(false) {
                return Err(WriteError::InvalidPart);
            }
            let have = match tokio::fs::read_to_string(part_md5_path(&dir, *n)).await {
                Ok(s) => s,
                Err(_) => md5_file(&p).await?,
            };
            if let Some(want) = etag {
                let want = want.trim().trim_matches('"').to_ascii_lowercase();
                if want != have {
                    return Err(WriteError::InvalidPart);
                }
            }
            md5s.extend(hex::decode(have.trim()).map_err(|_| WriteError::InvalidPart)?);
        }
        let parent = target.parent().ok_or(WriteError::InvalidKey)?.to_path_buf();
        tokio::fs::create_dir_all(&parent).await?;
        let tmp = TmpFile::new(&target);
        let mut out = tokio::fs::File::create(&tmp.0).await?;
        let mut sha = Sha256::new();
        let mut total: u64 = 0;
        let mut buf = vec![0u8; 256 * 1024];
        for (n, _) in &list {
            use tokio::io::AsyncReadExt;
            let mut f = tokio::fs::File::open(part_path(&dir, *n)).await?;
            loop {
                let r = f.read(&mut buf).await?;
                if r == 0 {
                    break;
                }
                sha.update(&buf[..r]);
                out.write_all(&buf[..r]).await?;
                total += r as u64;
            }
        }
        out.flush().await?;
        out.sync_all().await?;
        drop(out);
        tmp.commit(&target).await?;
        let _ = tokio::fs::remove_dir_all(&dir).await;
        self.notify_written(key, total, sha.finalize().into()).await;
        Ok(format!(
            "{}-{}",
            hex::encode(Md5::digest(&md5s)),
            list.len()
        ))
    }

    /// Delete multipart uploads older than [`STALE_MULTIPART_AGE`] and orphaned
    /// temp files older than [`STALE_TMP_AGE`]. Returns how many entries were
    /// removed. Runs blocking I/O; call from `spawn_blocking`.
    pub fn cleanup_stale(&self) -> usize {
        let mut removed = 0;
        let older_than = |m: &std::fs::Metadata, age: Duration| {
            m.modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|e| e > age)
        };
        if let Ok(rd) = std::fs::read_dir(self.root.join(MULTIPART_DIR)) {
            for e in rd.flatten() {
                // A directory's mtime moves on every part written into it.
                if e.metadata()
                    .is_ok_and(|m| m.is_dir() && older_than(&m, STALE_MULTIPART_AGE))
                    && std::fs::remove_dir_all(e.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        // Orphaned temp files anywhere in the tree (a crash between create and rename).
        let mut stack = vec![self.root.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                let name = e.file_name();
                let name = name.to_string_lossy();
                if ft.is_dir() {
                    if !(d == self.root && name == MULTIPART_DIR) {
                        stack.push(e.path());
                    }
                } else if ft.is_file()
                    && name.contains(TMP_MARK)
                    && e.metadata().is_ok_and(|m| older_than(&m, STALE_TMP_AGE))
                    && std::fs::remove_file(e.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        removed
    }
}

fn part_path(dir: &Path, part: u32) -> PathBuf {
    dir.join(format!("part-{part:05}"))
}

fn part_md5_path(dir: &Path, part: u32) -> PathBuf {
    dir.join(format!("part-{part:05}.md5"))
}

async fn md5_file(p: &Path) -> Result<String, WriteError> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(p).await?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// A temp file next to its target, removed on drop unless committed.
struct TmpFile(PathBuf);

impl TmpFile {
    fn new(target: &Path) -> Self {
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = format!(
            ".{name}{TMP_MARK}{}",
            hex::encode(crate::crypto::random_bytes(6))
        );
        Self(target.with_file_name(tmp))
    }

    /// Rename over `target` and fsync the directory.
    async fn commit(self, target: &Path) -> std::io::Result<()> {
        tokio::fs::rename(&self.0, target).await?;
        std::mem::forget(self);
        if let Some(parent) = target.parent() {
            fsync_dir(parent).await;
        }
        Ok(())
    }
}

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `fsync` a directory so a completed rename is durable. Errors are ignored:
/// some FUSE filesystems (including some mergerfs configurations) reject it.
pub async fn fsync_dir(dir: &Path) {
    let dir = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(f) = std::fs::File::open(&dir) {
            let _ = f.sync_all();
        }
    })
    .await;
}

fn io(e: std::io::Error) -> StorageError {
    if e.kind() == std::io::ErrorKind::NotFound {
        StorageError::NotFound
    } else {
        StorageError::Backend(e.to_string())
    }
}

/// Write `data` to `path` atomically (temp file in the same directory, fsync,
/// rename, fsync of the directory).
pub async fn write_atomic(path: &Path, data: &[u8]) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io)?;
    }
    let tmp = TmpFile::new(path);
    let mut f = tokio::fs::File::create(&tmp.0).await.map_err(io)?;
    f.write_all(data).await.map_err(io)?;
    f.sync_all().await.map_err(io)?;
    drop(f);
    tmp.commit(path).await.map_err(io)
}

/// What [`write_stream`] wrote.
struct Written {
    md5_hex: String,
    sha256: Sha256Digest,
    len: u64,
}

/// Stream a body to `target` atomically, checking length and MD5 like S3.
async fn write_stream<S, E>(
    target: &Path,
    mut body: S,
    want_len: Option<i64>,
    want_md5: Option<&str>,
) -> Result<Written, WriteError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    let want_digest = match want_md5 {
        Some(m) => Some(
            base64::engine::general_purpose::STANDARD
                .decode(m.trim())
                .ok()
                .filter(|d| d.len() == 16)
                .ok_or(WriteError::InvalidDigest)?,
        ),
        None => None,
    };
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = TmpFile::new(target);
    let mut f = tokio::fs::File::create(&tmp.0).await?;
    let mut hasher = Md5::new();
    let mut sha = Sha256::new();
    let mut n: i64 = 0;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| WriteError::Body)?;
        n += chunk.len() as i64;
        if want_len.is_some_and(|w| n > w) {
            return Err(WriteError::LengthMismatch);
        }
        hasher.update(&chunk);
        sha.update(&chunk);
        f.write_all(&chunk).await?;
    }
    if want_len.is_some_and(|w| n != w) {
        return Err(WriteError::LengthMismatch);
    }
    let digest = hasher.finalize();
    if want_digest.is_some_and(|d| d.as_slice() != digest.as_slice()) {
        return Err(WriteError::BadDigest);
    }
    crate::metrics::STORAGE
        .bytes
        .get_or_create(&crate::metrics::DirectionLabels {
            direction: "upload",
        })
        .inc_by(n as u64);
    f.flush().await?;
    f.sync_all().await?;
    drop(f);
    tmp.commit(target).await?;
    Ok(Written {
        md5_hex: hex::encode(digest),
        sha256: sha.finalize().into(),
        len: n as u64,
    })
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
        content_md5: Option<&str>,
    ) -> StorageResult<String> {
        self.path(key)?;
        Ok(self.sign(
            Op::Put,
            key,
            &Signed {
                exp: Self::exp(ttl),
                len: content_length,
                md5: content_md5.map(str::to_string),
                ..Default::default()
            },
        ))
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        self.path(key)?;
        Ok(self.sign(
            Op::Get,
            key,
            &Signed {
                exp: Self::exp(ttl),
                ..Default::default()
            },
        ))
    }

    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        self.path(key)?;
        if parts.is_empty() || parts.len() > 10_000 {
            return Err(StorageError::Backend("part count must be 1..=10000".into()));
        }
        let upload_id = hex::encode(crate::crypto::random_bytes(16));
        let dir = self.multipart_dir(&upload_id)?;
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        write_atomic(&dir.join("key"), key.as_bytes()).await?;
        let exp = Self::exp(ttl);
        let part_urls = parts
            .iter()
            .zip(1u32..)
            .map(|(spec, p)| {
                self.sign(
                    Op::Put,
                    key,
                    &Signed {
                        exp,
                        upload_id: Some(upload_id.clone()),
                        part: Some(p),
                        len: spec.len,
                        md5: spec.md5.clone(),
                    },
                )
            })
            .collect();
        let complete_url = self.sign(
            Op::Complete,
            key,
            &Signed {
                exp,
                upload_id: Some(upload_id.clone()),
                ..Default::default()
            },
        );
        Ok(MultipartUrls {
            object_key: key.to_string(),
            part_urls,
            complete_url,
            upload_id,
        })
    }

    async fn abort_multipart(&self, _key: &str, upload_id: &str) -> StorageResult<()> {
        let dir = self.multipart_dir(upload_id)?;
        match tokio::fs::remove_dir_all(dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(e)),
        }
    }

    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        match tokio::fs::metadata(self.path(key)?).await {
            Ok(m) if m.is_file() => Ok(Some(m.len())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        let p = self.path(key)?;
        match tokio::fs::remove_file(&p).await {
            Ok(()) => {
                if let Some(parent) = p.parent() {
                    fsync_dir(parent).await;
                }
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(e)),
        }
    }

    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        let (s, d) = (self.path(src)?, self.write_path(dst)?);
        if let Some(parent) = d.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let tmp = TmpFile::new(&d);
        tokio::fs::copy(&s, &tmp.0).await.map_err(io)?;
        let f = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&tmp.0)
            .await
            .map_err(io)?;
        f.sync_all().await.map_err(io)?;
        drop(f);
        tmp.commit(&d).await.map_err(io)
    }

    async fn put_bytes(&self, key: &str, data: Bytes) -> StorageResult<()> {
        write_atomic(&self.write_path(key)?, &data).await
    }

    async fn get_bytes(&self, key: &str) -> StorageResult<Bytes> {
        tokio::fs::read(self.path(key)?)
            .await
            .map(Bytes::from)
            .map_err(io)
    }

    async fn list(&self, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
        let root = self.root.clone();
        let after = after.map(str::to_string);
        tokio::task::spawn_blocking(move || list_dir(&root, after.as_deref(), limit))
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?
    }

    async fn get_stream(&self, key: &str) -> StorageResult<ByteStream> {
        let f = tokio::fs::File::open(self.path(key)?).await.map_err(io)?;
        Ok(Box::pin(tokio_util::io::ReaderStream::with_capacity(
            f,
            256 * 1024,
        )))
    }

    async fn put_stream(&self, key: &str, size: u64, body: ByteStream) -> StorageResult<()> {
        let target = self.write_path(key)?;
        write_stream(&target, body, Some(size as i64), None)
            .await
            .map(|_| ())
            .map_err(|e| match e {
                WriteError::InvalidKey => StorageError::InvalidKey,
                other => StorageError::Backend(format!("{other:?}")),
            })
    }
}

/// Sorted listing of the object files under `root` with keys after `after`.
///
/// Walks the tree depth-first, ordering each directory's entries so that
/// the walk yields keys in byte order (a directory `d` sorts as `d/`), and
/// skips subtrees that end before `after`. Temp files and `.multipart/` are
/// never listed.
fn list_dir(root: &Path, after: Option<&str>, limit: usize) -> StorageResult<Vec<ListedObject>> {
    let mut out = Vec::new();
    walk(root, "", after, limit, &mut out)?;
    Ok(out)
}

fn walk(
    dir: &Path,
    prefix: &str,
    after: Option<&str>,
    limit: usize,
    out: &mut Vec<ListedObject>,
) -> StorageResult<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(e)),
    };
    // (sort name, key or key prefix, is_dir)
    let mut entries: Vec<(String, String, bool)> = Vec::new();
    for e in rd {
        let e = e.map_err(io)?;
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let ft = e.file_type().map_err(io)?;
        if name.contains(TMP_MARK) || (prefix.is_empty() && name == MULTIPART_DIR) {
            continue;
        }
        let key = format!("{prefix}{name}");
        if ft.is_dir() {
            entries.push((format!("{key}/"), key, true));
        } else if ft.is_file() {
            entries.push((key.clone(), key, false));
        }
    }
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    for (sort, key, is_dir) in entries {
        if out.len() >= limit {
            return Ok(());
        }
        if is_dir {
            // Skip a subtree whose keys all sort at or before `after`.
            if let Some(a) = after
                && sort.as_str() < a
                && !a.starts_with(&sort)
            {
                continue;
            }
            walk(&dir.join(&key[prefix.len()..]), &sort, after, limit, out)?;
        } else {
            if after.is_some_and(|a| key.as_str() <= a) || validate_key(&key).is_err() {
                continue;
            }
            let size = std::fs::metadata(dir.join(&key[prefix.len()..]))
                .map_err(io)?
                .len();
            out.push(ListedObject { key, size });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (crate::testing::tempdir::TempDir, LocalStore) {
        let d = crate::testing::tempdir::TempDir::new().expect("tempdir");
        let s = LocalStore::new(d.0.join("o"), "http://x", vec![7; 32]).expect("store");
        (d, s)
    }

    fn body(chunks: &[&'static [u8]]) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
        futures_util::stream::iter(
            chunks
                .iter()
                .map(|c| Ok(Bytes::from_static(c)))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn md5_and_length_enforced() {
        crate::crypto::init();
        let (_d, s) = store();
        let md5_hello = "XUFAKrxLKna5cZ2REBfFkg==";
        assert_eq!(
            s.write_object("1/a", body(&[b"hel", b"lo"]), Some(5), Some(md5_hello))
                .await
                .as_deref(),
            Ok("5d41402abc4b2a76b9719d911017c592")
        );
        assert_eq!(
            s.write_object("1/b", body(&[b"hellO"]), Some(5), Some(md5_hello))
                .await,
            Err(WriteError::BadDigest)
        );
        assert_eq!(
            s.write_object("1/b", body(&[b"hello!"]), Some(5), None)
                .await,
            Err(WriteError::LengthMismatch)
        );
        assert_eq!(
            s.write_object("1/b", body(&[b"hell"]), Some(5), None).await,
            Err(WriteError::LengthMismatch)
        );
        assert_eq!(
            s.write_object("1/b", body(&[b"x"]), None, Some("nope"))
                .await,
            Err(WriteError::InvalidDigest)
        );
        // Failed writes leave nothing behind.
        assert_eq!(s.head("1/b").await.ok().flatten(), None);
        let names: Vec<_> = std::fs::read_dir(s.root().join("1"))
            .expect("dir")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a")]);
    }

    #[tokio::test]
    async fn multipart_complete_and_cleanup() {
        crate::crypto::init();
        let (_d, s) = store();
        let mp = s
            .create_multipart("1/big", 3, Duration::from_secs(60))
            .await
            .expect("create");
        let e1 = s
            .write_part(&mp.upload_id, 1, body(&[b"ab"]), None, None)
            .await
            .expect("p1");
        let e2 = s
            .write_part(&mp.upload_id, 2, body(&[b"cd"]), None, None)
            .await
            .expect("p2");
        assert_eq!(
            s.complete_multipart("1/big", &mp.upload_id, &[(2, None), (1, None)])
                .await,
            Err(WriteError::InvalidPartOrder)
        );
        assert_eq!(
            s.complete_multipart("1/big", &mp.upload_id, &[(1, Some("\"00\"".into()))])
                .await,
            Err(WriteError::InvalidPart)
        );
        assert_eq!(
            s.complete_multipart("1/other", &mp.upload_id, &[(1, None)])
                .await,
            Err(WriteError::NoSuchUpload)
        );
        let etag = s
            .complete_multipart(
                "1/big",
                &mp.upload_id,
                &[(1, Some(format!("\"{e1}\""))), (2, Some(e2.to_uppercase()))],
            )
            .await
            .expect("complete");
        assert!(etag.ends_with("-2"));
        assert_eq!(s.get_bytes("1/big").await.expect("get").as_ref(), b"abcd");
        assert_eq!(
            s.complete_multipart("1/big", &mp.upload_id, &[]).await,
            Err(WriteError::NoSuchUpload)
        );
        // Abort of a fresh upload removes it; stale cleanup of fresh items is a no-op.
        let mp2 = s
            .create_multipart("1/c", 1, Duration::from_secs(60))
            .await
            .expect("create");
        assert_eq!(s.cleanup_stale(), 0);
        s.abort_multipart("1/c", &mp2.upload_id)
            .await
            .expect("abort");
        assert!(!s.multipart_dir(&mp2.upload_id).expect("dir").exists());
        s.abort_multipart("1/c", &mp2.upload_id)
            .await
            .expect("abort twice");
    }

    #[test]
    fn tmp_marker_keys_rejected() {
        crate::crypto::init();
        let (_d, s) = store();
        assert!(s.path("1/.x.pv-tmp-abc").is_err());
        assert!(s.path("1/x.bin").is_ok());
    }
}
