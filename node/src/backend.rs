//! The concrete backend: the worker's traits implemented on the Ente API.
//!
//! [`EnteBackend`] is what the `photovault-node` binary uses (see
//! [`crate::cli::default_backend`]). It signs in like a new Ente device
//! ([`LoginFlow`]), stores the [`Session`] next to the node's state (mode
//! 0600), and builds the worker's [`Services`] from these adapters:
//!
//! | Trait                 | Adapter              | Built on                                  |
//! |-----------------------|----------------------|-------------------------------------------|
//! | [`LibrarySource`]     | [`EnteLibrary`]      | [`library::sync`], [`Library`]            |
//! | [`FileFetcher`]       | [`EnteFetcher`]      | downloads + [`ChunkedDecryptor`], live-photo zip |
//! | [`FileDataStore`]     | [`EnteFileData`]     | `/files/data/*`, [`mldata`], [`video::UploadPlan`] |
//! | [`VideoTranscoder`]   | [`EnteTranscoder`]   | [`video::generate`], public magic metadata |
//! | [`crate::worker::MlIndexer`] | `EnteMlIndexer`      | `ml::indexer` (cargo feature `ml`)        |
//!
//! The adapters share a [`FileCache`]: the last sync's decrypted files (file
//! keys, decryption headers, public magic metadata) by ID, so downloads and
//! uploads never need to decrypt the library again. The cache lives only in
//! memory; what is persisted (the worker's cursor) is the ciphertext-only
//! [`SyncState`].
//!
//! Plaintext never leaves the node: originals and thumbnails are decrypted
//! into the worker's private temporary directories, and every upload is
//! encrypted with the file key first (Ente's own formats, unchanged).
//!
//! All transfers share one [`RateLimiter`] when
//! `network.bandwidth_limit_kibps` is set.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::api::magic::encrypt_public_magic_metadata;
use crate::api::models::PreviewUploadUrl;
use crate::api::{ApiClient, ApiError, Download, FileDataType, LoginFlow, LoginStep, Session};
use crate::cli::{Backend, LoginRequest, LoginSummary, Prompts};
use crate::config::{Config, GpuProvider};
use crate::crypto::{self, ChunkedDecryptor, EncryptedBlob, SecretKey};
use crate::library::{self, EnteFile, Library, SyncState};
use crate::ml::mldata;
use crate::state::write_atomic;
use crate::video::{self, FfmpegConfig, GenerateOutcome, HlsOutput, PlaylistJson, UploadPlan};
use crate::worker::{
    FetchKind, FileDataStore, FileFetcher, FileType, LibraryFile, LibrarySnapshot, LibrarySource,
    PutOutcome, RemoteMlData, Services, TranscodeOutcome, VideoPreview, VideoTranscoder, permanent,
};

/// The version the node reports in `X-Client-Version`.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How many wrong passwords or codes `login` accepts before giving up.
const MAX_LOGIN_ATTEMPTS: u32 = 5;

/// Largest still image extracted from a live-photo zip (the web indexer's
/// `maxIndexableFileSize`); bigger entries are rejected as permanent errors,
/// which also bounds a hostile zip's expansion.
const MAX_LIVE_PHOTO_IMAGE: u64 = crate::worker::MAX_INDEXABLE_FILE_SIZE;

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

/// The production [`Backend`]: Ente API client, library, ML and video.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnteBackend;

impl EnteBackend {
    /// A new backend.
    pub fn new() -> Self {
        Self
    }
}

/// Writes `session` to `path` atomically with mode 0600 (Unix). The file
/// holds the master key, so it is never world- or group-readable.
pub fn save_session(path: &Path, session: &Session) -> Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec_pretty(session).context("encoding session")?);
    write_atomic(path, &bytes)
}

/// Reads the session written by `login`. A missing file is reported as
/// "not signed in".
pub fn load_session(path: &Path) -> Result<Session> {
    let bytes = match std::fs::read(path) {
        Ok(b) => Zeroizing::new(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "not signed in (no session at {}); run `photovault-node login` first",
            path.display()
        ),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

/// Whether a sign-in error is worth asking again for (wrong password or
/// code) rather than aborting.
fn retryable_login_error(e: &ApiError) -> bool {
    matches!(
        e,
        ApiError::IncorrectPassword | ApiError::Unauthorized | ApiError::Http { .. }
    )
}

#[async_trait]
impl Backend for EnteBackend {
    async fn login(
        &self,
        cfg: &Config,
        req: &LoginRequest,
        prompts: &dyn Prompts,
    ) -> Result<LoginSummary> {
        let client = ApiClient::new(&req.server_url, CLIENT_VERSION)?;
        let (mut flow, mut step) = LoginFlow::start(&client, &req.email)
            .await
            .context("starting sign-in")?;
        let mut failures = 0u32;
        let mut fail = |e: ApiError, prompts: &dyn Prompts| -> Result<()> {
            failures += 1;
            if failures >= MAX_LOGIN_ATTEMPTS || !retryable_login_error(&e) {
                return Err(anyhow::Error::new(e).context("signing in"));
            }
            prompts.info(&format!("{e}; please try again."));
            Ok(())
        };
        let session = loop {
            match step {
                LoginStep::EmailCode => {
                    prompts.info(&format!(
                        "A verification code was sent to {}.",
                        flow.email()
                    ));
                    let code = prompts.ask("Email code (or \"resend\"): ")?;
                    if code.eq_ignore_ascii_case("resend") || code.eq_ignore_ascii_case("r") {
                        flow.resend_code(&client).await.context("resending code")?;
                        continue;
                    }
                    match flow.submit_code(&client, &code).await {
                        Ok(next) => step = next,
                        Err(e) => {
                            fail(e, prompts)?;
                            step = LoginStep::EmailCode;
                        }
                    }
                }
                LoginStep::Password => {
                    let password = Zeroizing::new(prompts.ask_secret("Password: ")?);
                    match flow.submit_password(&client, &password).await {
                        Ok(next) => step = next,
                        Err(e) => {
                            fail(e, prompts)?;
                            step = LoginStep::Password;
                        }
                    }
                }
                LoginStep::Totp => {
                    let code = prompts.ask("Two-factor code: ")?;
                    match flow.submit_code(&client, &code).await {
                        Ok(next) => step = next,
                        Err(e) => {
                            fail(e, prompts)?;
                            step = LoginStep::Totp;
                        }
                    }
                }
                LoginStep::Complete(session) => break session,
            }
        };
        save_session(&cfg.session_path(), &session)?;
        Ok(LoginSummary {
            email: session.email.clone(),
            user_id: session.user_id,
        })
    }

    async fn logout(&self, cfg: &Config) -> Result<()> {
        let path = cfg.session_path();
        if !path.exists() {
            return Ok(());
        }
        let session = load_session(&path)?;
        let client = ApiClient::for_session(&session, CLIENT_VERSION)?;
        match client.logout().await {
            Ok(()) | Err(ApiError::Unauthorized) => {}
            // Deleting the local copy of the keys matters more than the
            // server-side revoke; the device can also be removed in the app.
            Err(e) => tracing::warn!(
                error = %e,
                "could not revoke the session on the server; remove this device in the Ente app"
            ),
        }
        Ok(())
    }

    async fn services(&self, cfg: &Config, provider: GpuProvider) -> Result<Services> {
        let session = load_session(&cfg.session_path())?;
        let client = ApiClient::for_session(&session, CLIENT_VERSION)?;
        let ctx = Shared::new(session, client, cfg.network.bandwidth_limit_kibps);
        let mut services = ctx.services();
        if cfg.tasks.video {
            let ffmpeg = FfmpegConfig {
                ffmpeg: cfg.video.ffmpeg.clone(),
                ffprobe: cfg.video.ffprobe.clone(),
            };
            services.video = Some(Arc::new(EnteTranscoder::new(ctx.clone(), ffmpeg)));
        }
        services.ml = ml_indexer(cfg, provider).await?;
        Ok(services)
    }
}

#[cfg(feature = "ml")]
async fn ml_indexer(
    cfg: &Config,
    provider: GpuProvider,
) -> Result<Option<Arc<dyn crate::worker::MlIndexer>>> {
    if !cfg.tasks.ml() {
        return Ok(None);
    }
    let indexer = EnteMlIndexer::new(cfg, provider).await.context(
        "starting the ML indexer (install ONNX Runtime or set ORT_DYLIB_PATH, \
         or disable tasks.faces and tasks.clip)",
    )?;
    Ok(Some(Arc::new(indexer)))
}

#[cfg(not(feature = "ml"))]
async fn ml_indexer(
    cfg: &Config,
    _provider: GpuProvider,
) -> Result<Option<Arc<dyn crate::worker::MlIndexer>>> {
    if cfg.tasks.ml() {
        tracing::warn!(
            "face and search indexing need a build with the `ml` feature; \
             only streamable videos will be produced"
        );
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// What the adapters share.
#[derive(Debug)]
pub struct Shared {
    /// Signed-in API client.
    pub client: ApiClient,
    /// The session (keys).
    pub session: Arc<Session>,
    /// Decrypted files of the last sync.
    pub cache: FileCache,
    /// Bandwidth limit for downloads and uploads.
    pub limiter: RateLimiter,
}

impl Shared {
    /// Builds the shared state for a signed-in `session` (tests use this to
    /// point the adapters at a mock server).
    pub fn new(session: Session, client: ApiClient, bandwidth_limit_kibps: u64) -> Arc<Self> {
        Arc::new(Self {
            client,
            session: Arc::new(session),
            cache: FileCache::default(),
            limiter: RateLimiter::new(bandwidth_limit_kibps),
        })
    }

    /// All adapters over this state, without ML or video (tests add their own).
    pub fn services(self: &Arc<Self>) -> Services {
        Services {
            library: Arc::new(EnteLibrary { ctx: self.clone() }),
            fetcher: Arc::new(EnteFetcher { ctx: self.clone() }),
            store: Arc::new(EnteFileData::new(self.clone())),
            ml: None,
            video: None,
        }
    }
}

/// A decrypted file plus the public-magic-metadata version needed to update it.
#[derive(Debug, Clone)]
pub struct CachedFile {
    /// The decrypted file.
    pub file: EnteFile,
    /// `pubMagicMetadata.version` on the server, if the file has any.
    pub pub_magic_version: Option<i64>,
}

/// The last sync's decrypted, owned, live files by ID (memory only).
#[derive(Debug, Default)]
pub struct FileCache(RwLock<HashMap<i64, Arc<CachedFile>>>);

impl FileCache {
    /// The cached file `id`.
    pub fn get(&self, id: i64) -> Result<Arc<CachedFile>> {
        self.0
            .read()
            .map_err(|_| anyhow!("file cache poisoned"))?
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("file {id} is not in the synced library"))
    }

    fn replace(&self, files: HashMap<i64, Arc<CachedFile>>) -> Result<()> {
        *self.0.write().map_err(|_| anyhow!("file cache poisoned"))? = files;
        Ok(())
    }

    fn update(&self, id: i64, f: CachedFile) {
        if let Ok(mut m) = self.0.write() {
            m.insert(id, Arc::new(f));
        }
    }
}

/// A simple shared bandwidth limiter (leaky bucket): each transfer reserves
/// its bytes on a common timeline and waits until its slot comes, with up to
/// one second of burst.
#[derive(Debug)]
pub struct RateLimiter {
    /// Bytes per second; `None` = unlimited.
    rate: Option<f64>,
    /// When the bytes reserved so far have been "sent".
    next: Mutex<Instant>,
}

impl RateLimiter {
    /// A limiter of `kibps` KiB/s; 0 means unlimited.
    pub fn new(kibps: u64) -> Self {
        Self {
            rate: (kibps > 0).then_some(kibps as f64 * 1024.0),
            next: Mutex::new(Instant::now()),
        }
    }

    /// Whether a limit is set.
    pub fn is_limited(&self) -> bool {
        self.rate.is_some()
    }

    /// The delay before `bytes` may be transferred, reserving them.
    fn reserve(&self, bytes: usize, now: Instant) -> Duration {
        let Some(rate) = self.rate else {
            return Duration::ZERO;
        };
        let Ok(mut next) = self.next.lock() else {
            return Duration::ZERO;
        };
        let burst = Duration::from_secs(1);
        let start = (*next).max(now.checked_sub(burst).unwrap_or(now));
        *next = start + Duration::from_secs_f64(bytes as f64 / rate);
        next.saturating_duration_since(now + burst)
    }

    /// Waits until `bytes` more may be transferred.
    pub async fn consume(&self, bytes: usize) {
        let wait = self.reserve(bytes, Instant::now());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Maps an error to [`permanent`] when retrying cannot help: the data does
/// not decrypt with the file's key (corrupt or foreign).
fn classify(e: ApiError) -> anyhow::Error {
    match e {
        ApiError::Crypto(c) => permanent(format!("cannot decrypt: {c}")),
        other => other.into(),
    }
}

fn is_conflict(e: &ApiError) -> bool {
    e.status() == Some(409)
}

// ---------------------------------------------------------------------------
// LibrarySource
// ---------------------------------------------------------------------------

/// [`LibrarySource`] over [`library::sync`]. The cursor is the serialized
/// [`SyncState`] (ciphertext only).
#[derive(Debug)]
pub struct EnteLibrary {
    ctx: Arc<Shared>,
}

/// Maps a decrypted file to the worker's view.
pub fn library_file(f: &EnteFile) -> LibraryFile {
    let duration_secs = f
        .metadata
        .duration
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| d.round() as u64);
    let stream_version = f
        .pub_magic_metadata
        .as_ref()
        .and_then(|m| m.get("sv"))
        .and_then(Value::as_i64);
    LibraryFile {
        id: f.id,
        file_type: FileType::from_ente(f.file_type.code()),
        size: f.file_size.and_then(|s| u64::try_from(s).ok()),
        updation_time: f.updation_time,
        duration_secs,
        stream_version,
    }
}

#[async_trait]
impl LibrarySource for EnteLibrary {
    async fn sync(&self, cursor: Option<Value>) -> Result<LibrarySnapshot> {
        let mut state = match cursor {
            Some(v) => serde_json::from_value::<SyncState>(v).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "discarding unreadable sync cursor; full resync");
                SyncState::default()
            }),
            None => SyncState::default(),
        };
        let report = library::sync(&self.ctx.client, &mut state)
            .await
            .context("syncing library")?;
        tracing::debug!(
            updated = report.updated.len(),
            removed = report.removed.len(),
            "library synced"
        );
        let lib = Library::open(&self.ctx.session, &state);
        let owned = lib.owned_files();
        let mut files = Vec::with_capacity(owned.len());
        let mut cache = HashMap::with_capacity(owned.len());
        for f in owned {
            files.push(library_file(&f));
            let pub_magic_version = state
                .files
                .get(&f.id)
                .and_then(|per| per.get(&f.collection_id))
                .and_then(|e| e.pub_magic_metadata.as_ref())
                .map(|m| m.version);
            cache.insert(
                f.id,
                Arc::new(CachedFile {
                    file: f,
                    pub_magic_version,
                }),
            );
        }
        self.ctx.cache.replace(cache)?;
        Ok(LibrarySnapshot {
            files,
            cursor: serde_json::to_value(&state).context("encoding sync state")?,
        })
    }
}

// ---------------------------------------------------------------------------
// FileFetcher
// ---------------------------------------------------------------------------

/// [`FileFetcher`]: downloads, decrypts (streaming, 4 MiB chunks) and, for
/// live photos, extracts the still image.
#[derive(Debug)]
pub struct EnteFetcher {
    ctx: Arc<Shared>,
}

/// Streams a download through the file's secretstream into `dest` (mode
/// 0600), applying the bandwidth limit. Removes `dest` on failure. This is
/// [`library::download_original_to`] with throttling, for both originals
/// and thumbnails.
async fn download_decrypt_to(
    limiter: &RateLimiter,
    mut dl: Download,
    header: &[u8],
    key: &SecretKey,
    dest: &Path,
) -> std::result::Result<u64, ApiError> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut out = opts.open(dest).await?;
    let result = async {
        let mut dec = ChunkedDecryptor::new(header, key)?;
        let mut written = 0u64;
        while let Some(chunk) = dl.chunk().await? {
            limiter.consume(chunk.len()).await;
            let pt = Zeroizing::new(dec.update(&chunk)?);
            out.write_all(&pt).await?;
            written += pt.len() as u64;
        }
        let pt = Zeroizing::new(dec.finish()?);
        out.write_all(&pt).await?;
        out.flush().await?;
        Ok::<u64, ApiError>(written + pt.len() as u64)
    }
    .await;
    if result.is_err() {
        drop(out);
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// Extracts the still image of an Ente live photo (a zip with an
/// `image.*` and a `video.*` entry; web `decodeLivePhoto`) to `dest`.
///
/// Falls back to the first entry that is not the video when no name starts
/// with `image`. Errors are [`permanent`]: the zip is what it is.
pub fn extract_live_photo_image(zip_path: &Path, dest: &Path) -> Result<()> {
    use std::io::Read;
    let f = std::fs::File::open(zip_path).context("opening live photo")?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| permanent(format!("live photo is not a valid zip: {e}")))?;
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
    let name = names
        .iter()
        .find(|n| base(n).starts_with("image"))
        .or_else(|| {
            names
                .iter()
                .find(|n| !base(n).starts_with("video") && !n.ends_with('/'))
        })
        .ok_or_else(|| permanent("live photo zip has no image"))?
        .clone();
    let entry = zip
        .by_name(&name)
        .map_err(|e| permanent(format!("live photo image: {e}")))?;
    if entry.size() > MAX_LIVE_PHOTO_IMAGE {
        return Err(permanent(format!(
            "live photo image is too large ({} bytes)",
            entry.size()
        )));
    }
    let mut buf = Zeroizing::new(Vec::with_capacity(entry.size() as usize));
    entry
        .take(MAX_LIVE_PHOTO_IMAGE + 1)
        .read_to_end(&mut buf)
        .map_err(|e| permanent(format!("live photo image: {e}")))?;
    if buf.len() as u64 > MAX_LIVE_PHOTO_IMAGE {
        return Err(permanent("live photo image is too large"));
    }
    write_private(dest, &buf)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

#[async_trait]
impl FileFetcher for EnteFetcher {
    async fn fetch(&self, file: &LibraryFile, kind: FetchKind, dest: &Path) -> Result<()> {
        let cached = self.ctx.cache.get(file.id)?;
        let f = &cached.file;
        let client = &self.ctx.client;
        let limiter = &self.ctx.limiter;
        match kind {
            FetchKind::Thumbnail => {
                let dl = client.download_thumbnail(f.id).await?;
                download_decrypt_to(limiter, dl, &f.thumbnail_decryption_header, &f.key, dest)
                    .await
                    .map_err(classify)?;
            }
            FetchKind::Original if file.file_type == FileType::LivePhoto => {
                let mut zip_path = dest.as_os_str().to_owned();
                zip_path.push(".zip");
                let zip_path = PathBuf::from(zip_path);
                let dl = client.download_file(f.id).await?;
                let res = async {
                    download_decrypt_to(limiter, dl, &f.file_decryption_header, &f.key, &zip_path)
                        .await
                        .map_err(classify)?;
                    let (z, d) = (zip_path.clone(), dest.to_path_buf());
                    tokio::task::spawn_blocking(move || extract_live_photo_image(&z, &d))
                        .await
                        .context("live photo extraction panicked")?
                }
                .await;
                let _ = tokio::fs::remove_file(&zip_path).await;
                if res.is_err() {
                    let _ = tokio::fs::remove_file(dest).await;
                }
                res?;
            }
            FetchKind::Original => {
                let dl = client.download_file(f.id).await?;
                download_decrypt_to(limiter, dl, &f.file_decryption_header, &f.key, dest)
                    .await
                    .map_err(classify)?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FileDataStore
// ---------------------------------------------------------------------------

/// [`FileDataStore`] over `/files/data/*` and `/files/video-data`.
#[derive(Debug)]
pub struct EnteFileData {
    ctx: Arc<Shared>,
    /// Which files have a live `vid_preview`, from `/files/data/status-diff`,
    /// with the diff cursor.
    previews: tokio::sync::Mutex<PreviewIndex>,
}

#[derive(Debug, Default)]
struct PreviewIndex {
    since: i64,
    have: HashSet<i64>,
}

impl EnteFileData {
    fn new(ctx: Arc<Shared>) -> Self {
        Self {
            ctx,
            previews: tokio::sync::Mutex::new(PreviewIndex::default()),
        }
    }

    /// Requests upload URLs for `plan`: upload V2 (bound to MD5s), falling
    /// back to the legacy `GET` when the server does not have V2.
    async fn upload_urls(
        &self,
        file_id: i64,
        plan: &UploadPlan,
    ) -> std::result::Result<(PreviewUploadUrl, bool), ApiError> {
        let c = &self.ctx.client;
        let v2 = if plan.is_multipart() {
            let md5s: Vec<String> = plan.parts.iter().map(|p| p.md5_b64.clone()).collect();
            c.multipart_preview_upload_url(
                file_id,
                FileDataType::VideoPreview,
                plan.size as i64,
                plan.part_size as i64,
                &md5s,
            )
            .await
        } else {
            c.preview_upload_url_v2(
                file_id,
                FileDataType::VideoPreview,
                plan.size as i64,
                &plan.parts[0].md5_b64,
            )
            .await
        };
        match v2 {
            Err(ApiError::Http {
                status: 404 | 405 | 501,
                ..
            }) => {
                let parts = plan.is_multipart().then_some(plan.parts.len() as u32);
                let r = c
                    .preview_upload_url(file_id, FileDataType::VideoPreview, parts)
                    .await?;
                Ok((r, false))
            }
            other => other.map(|r| (r, true)),
        }
    }

    /// Uploads the object of `plan` to the URLs in `urls`.
    async fn upload_object(
        &self,
        plan: &UploadPlan,
        urls: &PreviewUploadUrl,
        with_md5: bool,
    ) -> Result<()> {
        let c = &self.ctx.client;
        let check = video::PreviewUploadUrlResponse {
            object_id: urls.object_id.clone(),
            url: urls.url.clone(),
            part_urls: urls.part_urls.clone(),
            complete_url: urls.complete_url.clone(),
        };
        plan.check_response(&check)?;
        let single = (!plan.is_multipart())
            .then_some(urls.url.as_deref())
            .flatten();
        if let Some(url) = single {
            let body = plan.read_part(&plan.parts[0]).await?;
            self.ctx.limiter.consume(body.len()).await;
            let md5 = with_md5.then_some(plan.parts[0].md5_b64.as_str());
            c.put_presigned(url, body.into(), md5).await?;
            return Ok(());
        }
        let part_urls = urls
            .part_urls
            .as_ref()
            .ok_or_else(|| anyhow!("no part URLs"))?;
        let complete = urls
            .complete_url
            .as_deref()
            .ok_or_else(|| anyhow!("no completion URL"))?;
        let mut etags = Vec::with_capacity(plan.parts.len());
        for (part, url) in plan.parts.iter().zip(part_urls) {
            let body = plan.read_part(part).await?;
            self.ctx.limiter.consume(body.len()).await;
            let md5 = with_md5.then_some(part.md5_b64.as_str());
            let etag = c
                .put_presigned(url, body.into(), md5)
                .await?
                .ok_or_else(|| anyhow!("storage sent no ETag for part {}", part.number))?;
            etags.push(etag);
        }
        c.complete_multipart(complete, &etags).await?;
        Ok(())
    }
}

/// What [`EnteTranscoder`] puts in [`VideoPreview::details`]: where the
/// generated files are (inside the item's private directory) and their
/// properties. The playlist (which holds the one-time AES key) stays in its
/// file and is never copied into the details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewDetails {
    /// Playlist file name in [`VideoPreview::dir`].
    pub playlist: String,
    /// Encrypted MPEG-TS object file name in [`VideoPreview::dir`].
    pub video: String,
    /// Output width.
    pub width: u32,
    /// Output height.
    pub height: u32,
    /// Object size in bytes.
    pub size: u64,
}

impl PreviewDetails {
    fn from_output(dir: &Path, out: &HlsOutput) -> Result<Self> {
        let video = out
            .encrypted_video_path
            .strip_prefix(dir)
            .context("preview outside its directory")?
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            playlist: "output.m3u8".into(),
            video,
            width: out.width,
            height: out.height,
            size: out.size,
        })
    }

    /// Reads the generated stream back from `dir`.
    async fn load(&self, dir: &Path) -> Result<HlsOutput> {
        let safe = |n: &str| {
            let p = Path::new(n);
            p.components().count() == 1 && p.file_name().is_some()
        };
        if !safe(&self.playlist) || !safe(&self.video) {
            bail!("invalid preview file names");
        }
        Ok(HlsOutput {
            playlist_template: tokio::fs::read_to_string(dir.join(&self.playlist))
                .await
                .context("reading playlist")?,
            encrypted_video_path: dir.join(&self.video),
            width: self.width,
            height: self.height,
            size: self.size,
        })
    }
}

#[async_trait]
impl FileDataStore for EnteFileData {
    async fn remote_ml(&self, files: &[LibraryFile]) -> Result<HashMap<i64, RemoteMlData>> {
        let ids: Vec<i64> = files.iter().map(|f| f.id).collect();
        let mut out = HashMap::new();
        for chunk in ids.chunks(crate::api::files::MAX_FETCH_FILE_IDS) {
            let r = self
                .ctx
                .client
                .fetch_files_data(FileDataType::MlData, chunk)
                .await
                .context("fetching ML data")?;
            for d in r.data {
                let Ok(cached) = self.ctx.cache.get(d.file_id) else {
                    continue;
                };
                let opened = (|| -> Result<mldata::RemoteMlData> {
                    let data = crypto::b64_decode(&d.encrypted_data)?;
                    let header = crypto::b64_decode(&d.decryption_header)?;
                    Ok(mldata::open(
                        &data,
                        &header,
                        &cached.file.key,
                        d.updated_at,
                    )?)
                })();
                match opened {
                    Ok(m) => {
                        out.insert(
                            d.file_id,
                            RemoteMlData {
                                raw: m.raw,
                                updated_at: m.updated_at.unwrap_or(0),
                            },
                        );
                    }
                    // As the web app: unreadable remote data counts as absent.
                    Err(e) => tracing::warn!(file = d.file_id, error = %format!("{e:#}"),
                        "ignoring unreadable remote ML data"),
                }
            }
        }
        Ok(out)
    }

    async fn video_previews(&self, files: &[LibraryFile]) -> Result<HashSet<i64>> {
        let mut idx = self.previews.lock().await;
        loop {
            let page = self
                .ctx
                .client
                .file_data_status_diff(idx.since)
                .await
                .context("fetching file data status")?;
            let Some(max) = page.iter().map(|s| s.updated_at).max() else {
                break;
            };
            for s in &page {
                if s.kind != FileDataType::VideoPreview.as_str() {
                    continue;
                }
                if s.is_deleted {
                    idx.have.remove(&s.file_id);
                } else {
                    idx.have.insert(s.file_id);
                }
            }
            if max <= idx.since {
                break;
            }
            idx.since = max;
        }
        Ok(files
            .iter()
            .map(|f| f.id)
            .filter(|id| idx.have.contains(id))
            .collect())
    }

    async fn put_ml(
        &self,
        file: &LibraryFile,
        data: &Map<String, Value>,
        last_updated_at: i64,
    ) -> Result<PutOutcome> {
        let cached = self.ctx.cache.get(file.id)?;
        let blob = mldata::seal(data, &cached.file.key)?;
        self.ctx
            .limiter
            .consume(blob.encrypted_data.len() * 4 / 3)
            .await;
        match self
            .ctx
            .client
            .put_file_data(
                file.id,
                FileDataType::MlData,
                &blob,
                (last_updated_at > 0).then_some(last_updated_at),
                None,
            )
            .await
        {
            Ok(()) => Ok(PutOutcome::Stored),
            Err(e) if is_conflict(&e) => Ok(PutOutcome::Conflict),
            Err(e) => Err(e.into()),
        }
    }

    async fn put_video_preview(
        &self,
        file: &LibraryFile,
        preview: &VideoPreview,
    ) -> Result<PutOutcome> {
        let cached = self.ctx.cache.get(file.id)?;
        let details: PreviewDetails =
            serde_json::from_value(preview.details.clone()).context("preview details")?;
        let out = details.load(&preview.dir).await?;
        let original = file.size.unwrap_or(0);
        if !video::preview_size_allowed(out.size, original) {
            return Err(permanent(format!(
                "preview of {} bytes is too large for an original of {original} bytes",
                out.size
            )));
        }
        let plan = UploadPlan::for_file(&out).await?;
        let (urls, with_md5) = match self.upload_urls(file.id, &plan).await {
            Ok(u) => u,
            Err(e) if is_conflict(&e) => return Ok(PutOutcome::Conflict),
            Err(e) => return Err(anyhow::Error::new(e).context("requesting upload URL")),
        };
        self.upload_object(&plan, &urls, with_md5)
            .await
            .context("uploading preview object")?;
        let playlist: EncryptedBlob = PlaylistJson::for_output(&out).encrypt(&cached.file.key)?;
        match self
            .ctx
            .client
            .put_video_data(file.id, &urls.object_id, out.size as i64, &playlist, None)
            .await
        {
            Ok(()) => Ok(PutOutcome::Stored),
            Err(e) if is_conflict(&e) => Ok(PutOutcome::Conflict),
            Err(e) => Err(anyhow::Error::new(e).context("registering video data")),
        }
    }
}

// ---------------------------------------------------------------------------
// VideoTranscoder
// ---------------------------------------------------------------------------

/// [`VideoTranscoder`] over [`video::generate`]. When no stream is needed
/// it sets `pubMagicMetadata.sv = 1` on the file, as the Ente apps do.
#[derive(Debug)]
pub struct EnteTranscoder {
    ctx: Arc<Shared>,
    ffmpeg: FfmpegConfig,
}

impl EnteTranscoder {
    /// A transcoder over `ctx` running the given `ffmpeg`/`ffprobe`.
    pub fn new(ctx: Arc<Shared>, ffmpeg: FfmpegConfig) -> Self {
        Self { ctx, ffmpeg }
    }

    /// Sets `sv = 1` in the file's public magic metadata. A `409` means
    /// another client changed the metadata first; the next sync sees the
    /// new version and decides again.
    async fn mark_not_required(&self, id: i64) -> Result<()> {
        let cached = self.ctx.cache.get(id)?;
        let mut up = Map::new();
        up.insert("sv".into(), json!(1));
        let mm = encrypt_public_magic_metadata(
            cached.file.pub_magic_metadata.as_ref(),
            cached.pub_magic_version,
            &up,
            &cached.file.key,
        )?;
        match self.ctx.client.update_public_magic_metadata(id, &mm).await {
            Ok(()) => {
                let mut f = (*cached).clone();
                let mut pm = f
                    .file
                    .pub_magic_metadata
                    .take()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                pm.insert("sv".into(), json!(1));
                f.file.pub_magic_metadata = Some(Value::Object(pm));
                f.pub_magic_version = Some(mm.version + 1);
                self.ctx.cache.update(id, f);
                Ok(())
            }
            Err(e) if is_conflict(&e) => {
                tracing::debug!(
                    file = id,
                    "magic metadata changed elsewhere; not setting sv"
                );
                Ok(())
            }
            Err(e) => Err(anyhow::Error::new(e).context("setting sv = 1")),
        }
    }
}

#[async_trait]
impl VideoTranscoder for EnteTranscoder {
    async fn transcode(
        &self,
        file: &LibraryFile,
        input: &Path,
        out_dir: &Path,
        cancel: CancellationToken,
    ) -> Result<TranscodeOutcome> {
        let outcome = video::generate(&self.ffmpeg, input, out_dir, &cancel)
            .await
            .map_err(|e| match e {
                video::VideoError::NoVideoStream
                | video::VideoError::Size(_)
                | video::VideoError::InvalidPlaylist(_)
                | video::VideoError::Dimensions => permanent(e.to_string()),
                other => other.into(),
            })?;
        match outcome {
            GenerateOutcome::NotRequired => {
                self.mark_not_required(file.id).await?;
                Ok(TranscodeOutcome::NotNeeded(
                    "H.264 original small enough to play as is".into(),
                ))
            }
            GenerateOutcome::Generated(out) => Ok(TranscodeOutcome::Preview(VideoPreview {
                dir: out_dir.to_path_buf(),
                details: serde_json::to_value(PreviewDetails::from_output(out_dir, &out)?)?,
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// MlIndexer
// ---------------------------------------------------------------------------

#[cfg(feature = "ml")]
pub use ml_adapter::EnteMlIndexer;

#[cfg(feature = "ml")]
mod ml_adapter {
    use super::*;
    use crate::ml::indexer::{Indexer, IndexerConfig};
    use crate::ml::{ProviderPreference, mldata};
    use crate::worker::{MlIndexOutput, MlIndexer, MlRun, MlVersions};

    /// [`MlIndexer`] over Ente's own Rust ML pipeline (`ml::indexer`).
    #[derive(Debug)]
    pub struct EnteMlIndexer {
        indexer: Arc<Indexer>,
        gpu: bool,
    }

    /// The indexer's provider preference for the node's resolved provider.
    pub fn provider_preference(p: GpuProvider) -> ProviderPreference {
        match p {
            GpuProvider::Auto => ProviderPreference::Auto,
            GpuProvider::Cuda => ProviderPreference::Cuda,
            GpuProvider::Rocm => ProviderPreference::Rocm,
            GpuProvider::Migraphx => ProviderPreference::Migraphx,
            GpuProvider::Webgpu => ProviderPreference::Webgpu,
            GpuProvider::Cpu => ProviderPreference::Cpu,
        }
    }

    impl EnteMlIndexer {
        /// Loads ONNX Runtime (`ORT_DYLIB_PATH` or the default library name)
        /// and the models (cached under `<data_dir>/models`).
        pub async fn new(cfg: &Config, provider: GpuProvider) -> Result<Self> {
            let indexer = Indexer::new(IndexerConfig {
                models_dir: cfg.data_dir.join("models"),
                provider: provider_preference(provider),
                onnxruntime_path: None,
            })
            .await?;
            Ok(Self {
                indexer: Arc::new(indexer),
                gpu: provider.is_gpu(),
            })
        }
    }

    #[async_trait]
    impl MlIndexer for EnteMlIndexer {
        fn versions(&self) -> MlVersions {
            MlVersions {
                face: mldata::FACE_INDEXING_VERSION as i32,
                clip: mldata::CLIP_INDEXING_VERSION as i32,
            }
        }

        fn uses_gpu(&self) -> bool {
            self.gpu
        }

        async fn index(
            &self,
            file: &LibraryFile,
            image: &Path,
            run: MlRun,
        ) -> Result<MlIndexOutput> {
            let bytes = Zeroizing::new(tokio::fs::read(image).await.context("reading image")?);
            let indexer = self.indexer.clone();
            let id = file.id;
            let out =
                tokio::task::spawn_blocking(move || indexer.index(id, &bytes, run.faces, run.clip))
                    .await
                    .context("indexer panicked")?
                    .map_err(|e| {
                        if e.is_permanent() {
                            permanent(e.to_string())
                        } else {
                            e.into()
                        }
                    })?;
            Ok(MlIndexOutput {
                face: out.face.map(serde_json::to_value).transpose()?,
                clip: out.clip.map(serde_json::to_value).transpose()?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoError;

    #[test]
    fn rate_limiter_paces() {
        let r = RateLimiter::new(0);
        assert!(!r.is_limited());
        assert_eq!(r.reserve(1 << 30, Instant::now()), Duration::ZERO);

        let r = RateLimiter::new(1); // 1 KiB/s
        let now = Instant::now();
        // One second of burst is free.
        assert_eq!(r.reserve(1024, now), Duration::ZERO);
        // The next 2 KiB wait ~2 s beyond the burst.
        let w = r.reserve(2048, now);
        assert!(
            w > Duration::from_millis(1900) && w <= Duration::from_secs(2),
            "{w:?}"
        );
    }

    #[test]
    fn session_roundtrip_is_private() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        assert!(
            load_session(&p)
                .unwrap_err()
                .to_string()
                .contains("not signed in")
        );
        let s = Session {
            endpoint: "http://x/".into(),
            email: "a@b.c".into(),
            user_id: 5,
            token: "dA==".into(),
            master_key: SecretKey::generate(),
            public_key: "cA==".into(),
            secret_key: SecretKey::generate(),
        };
        save_session(&p, &s).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let l = load_session(&p).unwrap();
        assert_eq!(l.user_id, 5);
        assert_eq!(l.master_key.as_bytes(), s.master_key.as_bytes());
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (n, b) in entries {
                w.start_file(*n, opts).unwrap();
                w.write_all(b).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn live_photo_image_extraction() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("lp.zip");
        let out = d.path().join("img");
        std::fs::write(
            &z,
            zip_with(&[("video.mov", b"VIDEO"), ("image.heic", b"IMAGE")]),
        )
        .unwrap();
        extract_live_photo_image(&z, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"IMAGE");

        // Fallback: first non-video entry.
        std::fs::write(&z, zip_with(&[("video.mov", b"V"), ("x.jpg", b"J")])).unwrap();
        extract_live_photo_image(&z, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"J");

        // Garbage and image-less zips are permanent.
        std::fs::write(&z, b"not a zip").unwrap();
        let e = extract_live_photo_image(&z, &out).unwrap_err();
        assert!(e.chain().any(|c| c.is::<crate::worker::Permanent>()));
        std::fs::write(&z, zip_with(&[("video.mov", b"V")])).unwrap();
        assert!(extract_live_photo_image(&z, &out).is_err());
    }

    #[test]
    fn maps_library_files() {
        let f = EnteFile {
            id: 9,
            owner_id: 1,
            collection_id: 1,
            collection_ids: vec![1],
            key: SecretKey::generate(),
            file_type: library::FileType::Video,
            metadata: library::FileMetadata {
                duration: Some(12.6),
                ..Default::default()
            },
            pub_magic_metadata: Some(json!({"sv": 1})),
            file_decryption_header: vec![],
            thumbnail_decryption_header: vec![],
            file_size: Some(1000),
            thumbnail_size: None,
            updation_time: 77,
        };
        assert_eq!(
            library_file(&f),
            LibraryFile {
                id: 9,
                file_type: FileType::Video,
                size: Some(1000),
                updation_time: 77,
                duration_secs: Some(13),
                stream_version: Some(1),
            }
        );
    }

    #[test]
    fn crypto_errors_are_permanent() {
        let e = classify(ApiError::Crypto(CryptoError::Decrypt));
        assert!(e.chain().any(|c| c.is::<crate::worker::Permanent>()));
        let e = classify(ApiError::Unauthorized);
        assert!(!e.chain().any(|c| c.is::<crate::worker::Permanent>()));
    }
}
