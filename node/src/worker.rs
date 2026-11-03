//! The main loop: sync the library, work out what is missing, and produce
//! and upload it while the machine is not busy.
//!
//! One pass:
//!
//! 1. [`LibrarySource::sync`] returns every live file the account owns.
//! 2. [`plan`] picks the files that need ML indexes or a streamable video,
//!    using the local [`State`] (done / failed-with-backoff, keyed by file ID
//!    and `updationTime`) and the eligibility rules of the Ente apps.
//! 3. The remote status is checked in batches: files whose remote `mldata`
//!    already has face and CLIP indexes at or above the current versions are
//!    skipped, as the Ente web indexer does, and so are videos that already
//!    have a `vid_preview`.
//! 4. Remaining items run with bounded concurrency. Before each item starts
//!    the worker waits until the [`crate::schedule`] decision is
//!    [`Decision::Run`]; when it turns to pause, in-flight video transcodes
//!    are cancelled (and re-queued), ML items finish. Each item is fetched,
//!    processed, uploaded and recorded.
//! 5. A 409 Conflict on upload means another device stored the result
//!    first: it is recorded as done and never re-uploaded.
//!
//! The worker codes against the small traits below; the concrete Ente API
//! client, ML indexer and transcoder implement them. Plaintext only exists
//! in per-item temporary directories (mode 0700) that are removed when the
//! item ends, however it ends.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::{ConcurrencyConfig, Config, RetryConfig, TasksConfig};
use crate::schedule::{Decision, GpuJobs};
use crate::state::{
    Completion, Counters, FileKey, State, StateStore, TaskKind, Versions, unix_now, write_atomic,
};

/// Largest original the ML indexer downloads (non-video), as in the Ente web
/// app (`maxIndexableFileSize`).
pub const MAX_INDEXABLE_FILE_SIZE: u64 = 100 * 1000 * 1000;

/// Files per remote-status request.
pub const REMOTE_BATCH: usize = 100;

// ---------------------------------------------------------------------------
// Library model and traits
// ---------------------------------------------------------------------------

/// Ente file type (`metadata.fileType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    /// 0: image.
    Image,
    /// 1: video.
    Video,
    /// 2: live photo (image + video in a zip).
    LivePhoto,
    /// 3 or unknown.
    Other,
}

impl FileType {
    /// Maps Ente's numeric `fileType`.
    pub fn from_ente(v: i64) -> Self {
        match v {
            0 => Self::Image,
            1 => Self::Video,
            2 => Self::LivePhoto,
            _ => Self::Other,
        }
    }
}

/// What the worker needs to know about one file. Owned files only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFile {
    /// File ID.
    pub id: i64,
    /// File type.
    pub file_type: FileType,
    /// Size of the original in bytes (`info.fileSize`), if known.
    pub size: Option<u64>,
    /// `updationTime` in microseconds.
    pub updation_time: i64,
    /// Video duration in seconds (`metadata.duration`), if known.
    pub duration_secs: Option<u64>,
    /// Stream version from public magic metadata (`sv`); `1` means the
    /// apps decided the video needs no preview.
    pub stream_version: Option<i64>,
}

impl LibraryFile {
    /// The file-version key used in [`State`].
    pub fn key(&self) -> FileKey {
        FileKey::new(self.id, self.updation_time)
    }
}

/// Result of a library sync.
#[derive(Debug, Clone, Default)]
pub struct LibrarySnapshot {
    /// Every live (not trashed, not deleted) file owned by the account,
    /// each listed once even if it is in several collections.
    pub files: Vec<LibraryFile>,
    /// Cursor to pass to the next [`LibrarySource::sync`]; stored in the
    /// state file as-is.
    pub cursor: Value,
}

/// The user's library.
#[async_trait]
pub trait LibrarySource: Send + Sync {
    /// Pulls changes since `cursor` (from the previous snapshot, `None` the
    /// first time) and returns the complete current file list.
    async fn sync(&self, cursor: Option<Value>) -> Result<LibrarySnapshot>;
}

/// Which bytes of a file to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchKind {
    /// The original, decrypted. For a live photo: its still image.
    Original,
    /// The decrypted thumbnail (JPEG).
    Thumbnail,
}

/// Downloads and decrypts files.
#[async_trait]
pub trait FileFetcher: Send + Sync {
    /// Writes the decrypted `kind` of `file` to `dest` (a path inside a
    /// private temporary directory). Return a [`Permanent`] error for
    /// failures inherent to the file (corrupt, undecryptable).
    async fn fetch(&self, file: &LibraryFile, kind: FetchKind, dest: &Path) -> Result<()>;
}

/// A file's decrypted remote `mldata` (the JSON object the apps store).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteMlData {
    /// The whole decrypted object (`face`, `clip`, and any keys from newer
    /// clients, which are preserved on upload).
    pub raw: Map<String, Value>,
    /// The server's `updatedAt` for this data, sent back on upload for
    /// optimistic concurrency.
    pub updated_at: i64,
}

impl RemoteMlData {
    /// `raw[part].version`, if present.
    pub fn version(&self, part: &str) -> Option<i32> {
        version_of(&self.raw, part)
    }
}

fn version_of(raw: &Map<String, Value>, part: &str) -> Option<i32> {
    raw.get(part)?
        .get("version")?
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
}

/// Result of an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    /// Stored.
    Stored,
    /// The server answered 409 Conflict: the data exists or changed since
    /// it was read. The worker records the item as done and never retries.
    Conflict,
}

/// Remote derived data (`/files/data/*`).
#[async_trait]
pub trait FileDataStore: Send + Sync {
    /// Fetches and decrypts the `mldata` of up to [`REMOTE_BATCH`] files.
    /// Files without ML data are absent from the map.
    async fn remote_ml(&self, files: &[LibraryFile]) -> Result<HashMap<i64, RemoteMlData>>;
    /// Which of these files (up to [`REMOTE_BATCH`]) already have a
    /// `vid_preview`.
    async fn video_previews(&self, files: &[LibraryFile]) -> Result<HashSet<i64>>;
    /// Encrypts `data` with the file key and stores it as the file's
    /// `mldata`. `last_updated_at` is the `updated_at` of the remote data it
    /// replaces (0 if none). Map HTTP 409 to [`PutOutcome::Conflict`].
    async fn put_ml(
        &self,
        file: &LibraryFile,
        data: &Map<String, Value>,
        last_updated_at: i64,
    ) -> Result<PutOutcome>;
    /// Uploads a generated preview as the file's `vid_preview`. Map HTTP
    /// 409 to [`PutOutcome::Conflict`].
    async fn put_video_preview(
        &self,
        file: &LibraryFile,
        preview: &VideoPreview,
    ) -> Result<PutOutcome>;
}

/// Current indexer versions (`faceIndexingVersion`, `clipIndexingVersion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MlVersions {
    /// Face index version.
    pub face: i32,
    /// CLIP index version.
    pub clip: i32,
}

/// Which indexes to compute for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MlRun {
    /// Compute the face index.
    pub faces: bool,
    /// Compute the CLIP index.
    pub clip: bool,
}

/// Indexes produced for one file, each the complete JSON object stored
/// under `face` / `clip` in `mldata` (including `version` and `client`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MlIndexOutput {
    /// Present when [`MlRun::faces`] was set.
    pub face: Option<Value>,
    /// Present when [`MlRun::clip`] was set.
    pub clip: Option<Value>,
}

/// Face and CLIP indexing.
#[async_trait]
pub trait MlIndexer: Send + Sync {
    /// The versions this indexer produces.
    fn versions(&self) -> MlVersions;
    /// Whether indexing runs on a GPU (so the busy monitor ignores GPU load
    /// while it runs).
    fn uses_gpu(&self) -> bool {
        false
    }
    /// Indexes the image at `image` (the original for images and live
    /// photos, the thumbnail for videos). Return a [`Permanent`] error for
    /// undecodable images.
    async fn index(&self, file: &LibraryFile, image: &Path, run: MlRun) -> Result<MlIndexOutput>;
}

/// A generated streamable video, ready for [`FileDataStore::put_video_preview`].
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPreview {
    /// Directory with the transcoder's output files.
    pub dir: PathBuf,
    /// Transcoder-defined details (playlist name, size, dimensions, …).
    pub details: Value,
}

/// Result of a transcode.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscodeOutcome {
    /// A preview was produced.
    Preview(VideoPreview),
    /// The video needs no preview (reason given).
    NotNeeded(String),
}

/// Streamable-video generation.
#[async_trait]
pub trait VideoTranscoder: Send + Sync {
    /// Transcodes `input` into `out_dir`. Must stop promptly when `cancel`
    /// fires (the worker also drops the future, so child processes must be
    /// spawned with `kill_on_drop(true)`).
    async fn transcode(
        &self,
        file: &LibraryFile,
        input: &Path,
        out_dir: &Path,
        cancel: CancellationToken,
    ) -> Result<TranscodeOutcome>;
}

/// Marks an error as inherent to the file: retrying will not help, so the
/// worker gives up on this file version at once.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Permanent(pub String);

/// Builds a [`Permanent`] error.
pub fn permanent(msg: impl Into<String>) -> anyhow::Error {
    Permanent(msg.into()).into()
}

fn is_permanent(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<Permanent>())
}

/// The implementations the worker uses.
#[derive(Clone)]
pub struct Services {
    /// Library sync.
    pub library: Arc<dyn LibrarySource>,
    /// Downloads.
    pub fetcher: Arc<dyn FileFetcher>,
    /// Remote derived data.
    pub store: Arc<dyn FileDataStore>,
    /// ML indexer; `None` disables faces and CLIP.
    pub ml: Option<Arc<dyn MlIndexer>>,
    /// Transcoder; `None` disables video.
    pub video: Option<Arc<dyn VideoTranscoder>>,
}

// ---------------------------------------------------------------------------
// Options, status
// ---------------------------------------------------------------------------

/// Worker settings (derived from [`Config`]).
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Enabled tasks.
    pub tasks: TasksConfig,
    /// Concurrency limits.
    pub concurrency: ConcurrencyConfig,
    /// Retry policy.
    pub retry: RetryConfig,
    /// Skip videos of at least this many bytes.
    pub video_max_size: Option<u64>,
    /// Skip videos longer than this.
    pub video_max_duration_secs: Option<u64>,
    /// Time between passes in [`Worker::run`].
    pub poll_interval: Duration,
    /// Parent of per-item temporary directories.
    pub tmp_dir: PathBuf,
    /// Where to write [`StatusSnapshot`] JSON for `status`, if anywhere.
    pub status_file: Option<PathBuf>,
    /// Save the state at least this often while working.
    pub save_every: Duration,
}

impl WorkerOptions {
    /// Options from the node configuration.
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            tasks: cfg.tasks,
            concurrency: cfg.concurrency,
            retry: cfg.retry,
            video_max_size: (cfg.video.max_size_mib > 0)
                .then(|| cfg.video.max_size_mib * 1024 * 1024),
            video_max_duration_secs: (cfg.video.max_duration_secs > 0)
                .then_some(cfg.video.max_duration_secs),
            poll_interval: cfg.poll_interval(),
            tmp_dir: cfg.tmp_dir(),
            status_file: Some(cfg.status_path()),
            save_every: Duration::from_secs(15),
        }
    }
}

/// What the worker is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Not started.
    Starting,
    /// Syncing the library.
    Syncing,
    /// Checking which results already exist remotely.
    Checking,
    /// Processing items.
    Working,
    /// Items are waiting because the machine is busy.
    Paused,
    /// Nothing to do until the next sync.
    Idle,
    /// Shutting down.
    Stopping,
    /// Stopped.
    Stopped,
}

/// An item being processed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlight {
    /// File ID.
    pub file_id: i64,
    /// Task.
    pub kind: TaskKind,
    /// Unix seconds when it started.
    pub started_at: i64,
}

/// A point-in-time view of the worker for `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Unix seconds of this snapshot.
    pub updated_at: i64,
    /// Process ID of the node.
    pub pid: u32,
    /// Phase.
    pub phase: Phase,
    /// Current busy-monitor decision.
    pub decision: Decision,
    /// Owned files in the library at the last sync.
    pub library_files: usize,
    /// ML items waiting.
    pub queued_ml: usize,
    /// Video items waiting.
    pub queued_video: usize,
    /// Items being processed.
    pub in_flight: Vec<InFlight>,
    /// Totals.
    pub counters: Counters,
    /// Unix seconds of the last successful sync.
    pub last_sync_at: Option<i64>,
    /// Unix seconds of the next planned sync while idle.
    pub next_sync_at: Option<i64>,
    /// Last error, if the last pass failed.
    pub last_error: Option<String>,
}

impl StatusSnapshot {
    fn initial(counters: Counters) -> Self {
        Self {
            updated_at: unix_now(),
            pid: std::process::id(),
            phase: Phase::Starting,
            decision: Decision::Run,
            library_files: 0,
            queued_ml: 0,
            queued_video: 0,
            in_flight: Vec::new(),
            counters,
            last_sync_at: None,
            next_sync_at: None,
            last_error: None,
        }
    }
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassSummary {
    /// Owned files in the library.
    pub files: usize,
    /// Items found already done remotely.
    pub already_remote: usize,
    /// Items processed to completion (uploaded, conflict or not needed).
    pub completed: usize,
    /// Items that failed.
    pub failed: usize,
    /// Items left unfinished because of shutdown.
    pub unfinished: usize,
    /// Files skipped as ineligible (too large, unsupported type, …).
    pub ineligible: usize,
}

// ---------------------------------------------------------------------------
// Planning (pure)
// ---------------------------------------------------------------------------

/// Files to check remotely and possibly process, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Need ML indexes.
    pub ml: Vec<LibraryFile>,
    /// Need a streamable video.
    pub video: Vec<LibraryFile>,
    /// Count of ineligible (file, task) pairs.
    pub ineligible: usize,
}

/// The minimum versions a done ML result must have.
pub fn min_ml_versions(tasks: &TasksConfig, v: MlVersions) -> Versions {
    Versions {
        face: tasks.faces.then_some(v.face),
        clip: tasks.clip.then_some(v.clip),
    }
}

/// Whether the file can be ML-indexed (as the Ente web indexer decides).
pub fn ml_eligible(f: &LibraryFile) -> bool {
    match f.file_type {
        FileType::Video => true,
        FileType::Image | FileType::LivePhoto => {
            f.size.is_none_or(|s| s <= MAX_INDEXABLE_FILE_SIZE)
        }
        FileType::Other => false,
    }
}

/// Whether the video gets a streamable preview (Ente mobile rules: not
/// marked `sv = 1`, size and duration known and within limits).
pub fn video_eligible(f: &LibraryFile, opts: &WorkerOptions) -> bool {
    f.file_type == FileType::Video
        && f.stream_version != Some(1)
        && opts
            .video_max_size
            .is_none_or(|m| f.size.is_some_and(|s| s < m))
        && opts
            .video_max_duration_secs
            .is_none_or(|m| f.duration_secs.is_some_and(|d| d <= m))
}

/// Picks the files that need work according to local state.
pub fn plan(
    files: &[LibraryFile],
    state: &State,
    opts: &WorkerOptions,
    ml_versions: Option<MlVersions>,
    video_enabled: bool,
    now: i64,
) -> Plan {
    let mut p = Plan::default();
    let ml_min = ml_versions
        .filter(|_| opts.tasks.ml())
        .map(|v| min_ml_versions(&opts.tasks, v));
    for f in files {
        if let Some(min) = ml_min {
            if !ml_eligible(f) {
                p.ineligible += 1;
            } else if state.is_due(f.key(), TaskKind::Ml, min, now) {
                p.ml.push(f.clone());
            }
        }
        if video_enabled && opts.tasks.video && f.file_type == FileType::Video {
            if !video_eligible(f, opts) {
                p.ineligible += 1;
            } else if state.is_due(f.key(), TaskKind::Video, Versions::default(), now) {
                p.video.push(f.clone());
            }
        }
    }
    let newest_first = |a: &LibraryFile, b: &LibraryFile| {
        b.updation_time.cmp(&a.updation_time).then(b.id.cmp(&a.id))
    };
    p.ml.sort_by(newest_first);
    p.video.sort_by(newest_first);
    p
}

/// Whether remote ML data already satisfies the enabled tasks.
pub fn remote_ml_satisfies(remote: &RemoteMlData, min: Versions) -> bool {
    let ok = |part: &str, want: Option<i32>| {
        want.is_none_or(|w| remote.version(part).is_some_and(|h| h >= w))
    };
    ok("face", min.face) && ok("clip", min.clip)
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// One unit of work.
#[derive(Debug, Clone)]
struct WorkItem {
    file: LibraryFile,
    kind: TaskKind,
    remote_ml: Option<RemoteMlData>,
}

/// How an item ended.
#[derive(Debug)]
enum ItemOutcome {
    Done { how: Completion, versions: Versions },
    Failed { error: String, permanent: bool },
    Cancelled,
}

/// Shared, cheaply clonable context for item tasks.
#[derive(Clone)]
struct TaskCtx {
    services: Services,
    tasks: TasksConfig,
    tmp_dir: PathBuf,
    downloads: Arc<Semaphore>,
    gpu_jobs: GpuJobs,
}

/// The compute node's work loop.
pub struct Worker {
    services: Services,
    opts: WorkerOptions,
    store: StateStore,
    gate: watch::Receiver<Decision>,
    gate_closed: bool,
    gpu_jobs: GpuJobs,
    status_tx: watch::Sender<StatusSnapshot>,
    last_status_write: Option<Instant>,
}

impl Worker {
    /// Creates a worker. `gate` carries the busy monitor's decisions;
    /// `gpu_jobs` is the counter the monitor reads.
    pub fn new(
        services: Services,
        opts: WorkerOptions,
        store: StateStore,
        gate: watch::Receiver<Decision>,
        gpu_jobs: GpuJobs,
    ) -> Self {
        let (status_tx, _) = watch::channel(StatusSnapshot::initial(store.state.counters.clone()));
        Self {
            services,
            opts,
            store,
            gate,
            gate_closed: false,
            gpu_jobs,
            status_tx,
            last_status_write: None,
        }
    }

    /// Live status updates.
    pub fn subscribe(&self) -> watch::Receiver<StatusSnapshot> {
        self.status_tx.subscribe()
    }

    /// The persisted state.
    pub fn state(&self) -> &State {
        &self.store.state
    }

    /// Runs passes until `shutdown`, sleeping `poll_interval` between them
    /// and backing off after errors. Saves the state before returning.
    pub async fn run(&mut self, shutdown: CancellationToken) -> Result<()> {
        let mut failures = 0u32;
        while !shutdown.is_cancelled() {
            let wait = match self.run_pass(&shutdown).await {
                Ok(s) => {
                    failures = 0;
                    tracing::info!(?s, "pass finished");
                    self.opts.poll_interval
                }
                Err(e) => {
                    failures += 1;
                    tracing::warn!(error = %format!("{e:#}"), "pass failed");
                    self.status(|s| s.last_error = Some(format!("{e:#}")));
                    let backoff = Duration::from_secs(30u64 << failures.min(6));
                    backoff.min(self.opts.poll_interval.max(Duration::from_secs(30)))
                }
            };
            if shutdown.is_cancelled() {
                break;
            }
            let next = unix_now() + wait.as_secs() as i64;
            self.status(|s| {
                s.phase = Phase::Idle;
                s.next_sync_at = Some(next);
            });
            tokio::select! {
                _ = shutdown.cancelled() => {}
                _ = tokio::time::sleep(wait) => {}
            }
        }
        self.finish()
    }

    /// Runs a single pass (for `once`) and saves the state.
    pub async fn run_once(&mut self, shutdown: CancellationToken) -> Result<PassSummary> {
        let res = self.run_pass(&shutdown).await;
        if let Err(e) = &res {
            self.status(|s| s.last_error = Some(format!("{e:#}")));
        }
        self.finish()?;
        res
    }

    fn finish(&mut self) -> Result<()> {
        self.store.save()?;
        self.status(|s| {
            s.phase = Phase::Stopped;
            s.in_flight.clear();
            s.next_sync_at = None;
        });
        self.write_status(true);
        Ok(())
    }

    /// One pass: sync, plan, check remote, process. Stops dispatching on
    /// `shutdown`, cancels in-flight items and saves state.
    pub async fn run_pass(&mut self, shutdown: &CancellationToken) -> Result<PassSummary> {
        self.status(|s| {
            s.phase = Phase::Syncing;
            s.next_sync_at = None;
        });
        let mut summary = PassSummary::default();

        let cursor = self.store.state.library_cursor.clone();
        let snapshot = tokio::select! {
            r = self.services.library.sync(cursor) => r.context("library sync")?,
            _ = shutdown.cancelled() => return Ok(summary),
        };
        summary.files = snapshot.files.len();
        let live: HashSet<i64> = snapshot.files.iter().map(|f| f.id).collect();
        {
            let st = self.store.edit();
            st.library_cursor = Some(snapshot.cursor);
            st.retain_files(&live);
        }
        let now = unix_now();
        self.status(|s| {
            s.library_files = summary.files;
            s.last_sync_at = Some(now);
            s.last_error = None;
            s.phase = Phase::Checking;
        });

        let ml_versions = self
            .services
            .ml
            .as_ref()
            .filter(|_| self.opts.tasks.ml())
            .map(|m| m.versions());
        let video_enabled = self.services.video.is_some() && self.opts.tasks.video;
        let p = plan(
            &snapshot.files,
            &self.store.state,
            &self.opts,
            ml_versions,
            video_enabled,
            now,
        );
        summary.ineligible = p.ineligible;
        tracing::info!(
            files = summary.files,
            ml_candidates = p.ml.len(),
            video_candidates = p.video.len(),
            "library synced"
        );

        let mut ml_q = VecDeque::new();
        if let Some(v) = ml_versions {
            let min = min_ml_versions(&self.opts.tasks, v);
            for chunk in p.ml.chunks(REMOTE_BATCH) {
                if shutdown.is_cancelled() {
                    return Ok(summary);
                }
                let mut remote = self
                    .services
                    .store
                    .remote_ml(chunk)
                    .await
                    .context("fetching remote ML data")?;
                for f in chunk {
                    let r = remote.remove(&f.id);
                    if let Some(r) = r.as_ref().filter(|r| remote_ml_satisfies(r, min)) {
                        let versions = Versions {
                            face: r.version("face"),
                            clip: r.version("clip"),
                        };
                        self.store.edit().mark_done(
                            f.key(),
                            TaskKind::Ml,
                            Completion::AlreadyRemote,
                            versions,
                            now,
                        );
                        summary.already_remote += 1;
                    } else {
                        ml_q.push_back(WorkItem {
                            file: f.clone(),
                            kind: TaskKind::Ml,
                            remote_ml: r,
                        });
                    }
                }
            }
        }
        let mut video_q = VecDeque::new();
        for chunk in p.video.chunks(REMOTE_BATCH) {
            if shutdown.is_cancelled() {
                return Ok(summary);
            }
            let have = self
                .services
                .store
                .video_previews(chunk)
                .await
                .context("fetching video preview status")?;
            for f in chunk {
                if have.contains(&f.id) {
                    self.store.edit().mark_done(
                        f.key(),
                        TaskKind::Video,
                        Completion::AlreadyRemote,
                        Versions::default(),
                        now,
                    );
                    summary.already_remote += 1;
                } else {
                    video_q.push_back(WorkItem {
                        file: f.clone(),
                        kind: TaskKind::Video,
                        remote_ml: None,
                    });
                }
            }
        }
        self.save_quietly();

        self.process_queues(ml_q, video_q, shutdown, &mut summary)
            .await;

        let st = self.store.edit();
        st.counters.passes += 1;
        st.counters.last_pass_at = Some(unix_now());
        self.store.save()?;
        let counters = self.store.state.counters.clone();
        self.status(|s| s.counters = counters);
        self.write_status(true);
        Ok(summary)
    }

    /// Dispatches queued items with bounded concurrency, following the gate.
    async fn process_queues(
        &mut self,
        mut ml_q: VecDeque<WorkItem>,
        mut video_q: VecDeque<WorkItem>,
        shutdown: &CancellationToken,
        summary: &mut PassSummary,
    ) {
        if let Err(e) = create_private_dir(&self.opts.tmp_dir) {
            tracing::warn!(error = %format!("{e:#}"), "cannot create temporary directory");
        }
        let ctx = TaskCtx {
            services: self.services.clone(),
            tasks: self.opts.tasks,
            tmp_dir: self.opts.tmp_dir.clone(),
            downloads: Arc::new(Semaphore::new(self.opts.concurrency.downloads.max(1))),
            gpu_jobs: self.gpu_jobs.clone(),
        };
        let mut set: JoinSet<(WorkItem, ItemOutcome)> = JoinSet::new();
        let mut tokens: HashMap<(i64, TaskKind), CancellationToken> = HashMap::new();
        let mut in_flight: Vec<InFlight> = Vec::new();
        let (mut ml_running, mut video_running) = (0usize, 0usize);
        let mut stopping = false;
        let mut last_save = Instant::now();

        loop {
            let running = self.gate_closed || self.gate.borrow().is_run();
            if running && !stopping {
                loop {
                    let item = if ml_running < self.opts.concurrency.ml && !ml_q.is_empty() {
                        ml_q.pop_front()
                    } else if video_running < self.opts.concurrency.video && !video_q.is_empty() {
                        video_q.pop_front()
                    } else {
                        None
                    };
                    let Some(item) = item else { break };
                    match item.kind {
                        TaskKind::Ml => ml_running += 1,
                        TaskKind::Video => video_running += 1,
                    }
                    let token = shutdown.child_token();
                    tokens.insert((item.file.id, item.kind), token.clone());
                    in_flight.push(InFlight {
                        file_id: item.file.id,
                        kind: item.kind,
                        started_at: unix_now(),
                    });
                    let ctx = ctx.clone();
                    set.spawn(async move {
                        let outcome = tokio::select! {
                            biased;
                            _ = token.cancelled() => ItemOutcome::Cancelled,
                            o = process_item(&ctx, &item, token.clone()) => o,
                        };
                        // An error caused by the cancellation (e.g. ffmpeg
                        // killed) is a pause or shutdown, not a failure.
                        let outcome = match outcome {
                            ItemOutcome::Failed { .. } if token.is_cancelled() => {
                                ItemOutcome::Cancelled
                            }
                            o => o,
                        };
                        (item, outcome)
                    });
                }
            }

            let phase = if stopping {
                Phase::Stopping
            } else if running {
                Phase::Working
            } else {
                Phase::Paused
            };
            let decision = self.gate.borrow().clone();
            let (qm, qv, inf) = (ml_q.len(), video_q.len(), in_flight.clone());
            let counters = self.store.state.counters.clone();
            self.status(|s| {
                s.phase = phase;
                s.decision = decision;
                s.queued_ml = qm;
                s.queued_video = qv;
                s.in_flight = inf;
                s.counters = counters;
            });

            if set.is_empty() && (stopping || (ml_q.is_empty() && video_q.is_empty())) {
                break;
            }

            tokio::select! {
                biased;
                _ = shutdown.cancelled(), if !stopping => {
                    stopping = true;
                    tracing::info!("shutting down: cancelling in-flight items");
                    for t in tokens.values() {
                        t.cancel();
                    }
                }
                changed = self.gate.changed(), if !self.gate_closed => {
                    if changed.is_err() {
                        self.gate_closed = true;
                    } else if !self.gate.borrow_and_update().is_run() {
                        for ((_, kind), t) in &tokens {
                            if *kind == TaskKind::Video {
                                t.cancel();
                            }
                        }
                    }
                }
                Some(joined) = set.join_next(), if !set.is_empty() => {
                    let (item, outcome) = match joined {
                        Ok(r) => r,
                        Err(e) => {
                            // A panicking task: its item is lost for this pass
                            // and retried on the next.
                            tracing::error!(error = %e, "work item task panicked");
                            summary.failed += 1;
                            continue;
                        }
                    };
                    tokens.remove(&(item.file.id, item.kind));
                    in_flight.retain(|i| !(i.file_id == item.file.id && i.kind == item.kind));
                    match item.kind {
                        TaskKind::Ml => ml_running -= 1,
                        TaskKind::Video => video_running -= 1,
                    }
                    self.record(item, outcome, &mut ml_q, &mut video_q, stopping, summary);
                    if last_save.elapsed() >= self.opts.save_every {
                        self.save_quietly();
                        last_save = Instant::now();
                    }
                }
            }
        }
        summary.unfinished += ml_q.len() + video_q.len();
        self.save_quietly();
    }

    /// Records an item's outcome in the state (or re-queues it).
    fn record(
        &mut self,
        item: WorkItem,
        outcome: ItemOutcome,
        ml_q: &mut VecDeque<WorkItem>,
        video_q: &mut VecDeque<WorkItem>,
        stopping: bool,
        summary: &mut PassSummary,
    ) {
        let now = unix_now();
        let key = item.file.key();
        match outcome {
            ItemOutcome::Done { how, versions } => {
                tracing::info!(file = item.file.id, kind = %item.kind, ?how, "done");
                self.store
                    .edit()
                    .mark_done(key, item.kind, how, versions, now);
                summary.completed += 1;
            }
            ItemOutcome::Failed { error, permanent } => {
                let rec = self.store.edit().mark_failed(
                    key,
                    item.kind,
                    &error,
                    permanent,
                    &self.opts.retry,
                    now,
                );
                tracing::warn!(file = item.file.id, kind = %item.kind, %error, ?rec, "failed");
                summary.failed += 1;
            }
            ItemOutcome::Cancelled if stopping => summary.unfinished += 1,
            ItemOutcome::Cancelled => {
                tracing::debug!(file = item.file.id, kind = %item.kind, "paused; re-queued");
                match item.kind {
                    TaskKind::Ml => ml_q.push_front(item),
                    TaskKind::Video => video_q.push_front(item),
                }
            }
        }
    }

    fn save_quietly(&mut self) {
        if let Err(e) = self.store.save() {
            tracing::error!(error = %format!("{e:#}"), "saving state");
        }
    }

    /// Updates the status snapshot and publishes it.
    fn status(&mut self, f: impl FnOnce(&mut StatusSnapshot)) {
        let mut phase_changed = false;
        self.status_tx.send_modify(|s| {
            let before = s.phase;
            f(s);
            s.updated_at = unix_now();
            phase_changed = before != s.phase;
        });
        self.write_status(phase_changed);
    }

    /// Writes the status file, at most once a second unless forced.
    fn write_status(&mut self, force: bool) {
        let Some(path) = &self.opts.status_file else {
            return;
        };
        if !force
            && self
                .last_status_write
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.last_status_write = Some(Instant::now());
        let snap = self.status_tx.borrow().clone();
        match serde_json::to_vec_pretty(&snap) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(path, &bytes) {
                    tracing::debug!(error = %format!("{e:#}"), "writing status file");
                }
            }
            Err(e) => tracing::debug!(error = %e, "serialising status"),
        }
    }
}

// ---------------------------------------------------------------------------
// Item processing
// ---------------------------------------------------------------------------

async fn process_item(ctx: &TaskCtx, item: &WorkItem, cancel: CancellationToken) -> ItemOutcome {
    let res = match item.kind {
        TaskKind::Ml => process_ml(ctx, item).await,
        TaskKind::Video => process_video(ctx, item, cancel).await,
    };
    match res {
        Ok((how, versions)) => ItemOutcome::Done { how, versions },
        Err(e) => ItemOutcome::Failed {
            permanent: is_permanent(&e),
            error: format!("{e:#}"),
        },
    }
}

async fn process_ml(ctx: &TaskCtx, item: &WorkItem) -> Result<(Completion, Versions)> {
    let indexer = ctx
        .services
        .ml
        .as_ref()
        .ok_or_else(|| anyhow!("no ML indexer configured"))?;
    let v = indexer.versions();
    let file = &item.file;
    let remote = item.remote_ml.as_ref();
    let have = |part: &str, want: i32| {
        remote
            .and_then(|r| r.version(part))
            .is_some_and(|h| h >= want)
    };
    let run = MlRun {
        faces: ctx.tasks.faces && !have("face", v.face),
        clip: ctx.tasks.clip && !have("clip", v.clip),
    };
    if !run.faces && !run.clip {
        return Ok((Completion::AlreadyRemote, Versions::default()));
    }

    let dir = TempDir::create(&ctx.tmp_dir, &format!("ml-{}", file.id))?;
    let input = dir.path().join("input");
    let kind = if file.file_type == FileType::Video {
        FetchKind::Thumbnail
    } else {
        FetchKind::Original
    };
    {
        let _permit = ctx.downloads.acquire().await?;
        ctx.services
            .fetcher
            .fetch(file, kind, &input)
            .await
            .context("fetching file")?;
    }
    let out = {
        let _gpu = GpuGuard::new(&ctx.gpu_jobs, indexer.uses_gpu());
        indexer.index(file, &input, run).await.context("indexing")?
    };
    drop(dir);

    let mut raw = remote.map(|r| r.raw.clone()).unwrap_or_default();
    if run.faces {
        raw.insert(
            "face".into(),
            out.face
                .ok_or_else(|| anyhow!("indexer returned no face index"))?,
        );
    }
    if run.clip {
        raw.insert(
            "clip".into(),
            out.clip
                .ok_or_else(|| anyhow!("indexer returned no CLIP index"))?,
        );
    }
    let versions = Versions {
        face: version_of(&raw, "face"),
        clip: version_of(&raw, "clip"),
    };
    let last = remote.map_or(0, |r| r.updated_at);
    match ctx
        .services
        .store
        .put_ml(file, &raw, last)
        .await
        .context("uploading ML data")?
    {
        PutOutcome::Stored => Ok((Completion::Uploaded, versions)),
        PutOutcome::Conflict => Ok((Completion::Conflict, min_ml_versions(&ctx.tasks, v))),
    }
}

async fn process_video(
    ctx: &TaskCtx,
    item: &WorkItem,
    cancel: CancellationToken,
) -> Result<(Completion, Versions)> {
    let transcoder = ctx
        .services
        .video
        .as_ref()
        .ok_or_else(|| anyhow!("no video transcoder configured"))?;
    let file = &item.file;
    let dir = TempDir::create(&ctx.tmp_dir, &format!("video-{}", file.id))?;
    let input = dir.path().join("input");
    let out_dir = dir.path().join("out");
    create_private_dir(&out_dir)?;
    {
        let _permit = ctx.downloads.acquire().await?;
        ctx.services
            .fetcher
            .fetch(file, FetchKind::Original, &input)
            .await
            .context("fetching video")?;
    }
    let outcome = transcoder
        .transcode(file, &input, &out_dir, cancel)
        .await
        .context("transcoding")?;
    let preview = match outcome {
        TranscodeOutcome::NotNeeded(reason) => {
            tracing::debug!(file = file.id, %reason, "no preview needed");
            return Ok((Completion::NotNeeded, Versions::default()));
        }
        TranscodeOutcome::Preview(p) => p,
    };
    let how = match ctx
        .services
        .store
        .put_video_preview(file, &preview)
        .await
        .context("uploading video preview")?
    {
        PutOutcome::Stored => Completion::Uploaded,
        PutOutcome::Conflict => Completion::Conflict,
    };
    Ok((how, Versions::default()))
}

/// Counts a running GPU job for the busy monitor.
struct GpuGuard<'a>(Option<&'a GpuJobs>);

impl<'a> GpuGuard<'a> {
    fn new(jobs: &'a GpuJobs, active: bool) -> Self {
        if active {
            jobs.fetch_add(1, Ordering::Relaxed);
            Self(Some(jobs))
        } else {
            Self(None)
        }
    }
}

impl Drop for GpuGuard<'_> {
    fn drop(&mut self) {
        if let Some(j) = self.0 {
            j.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// A private (0700) directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn create(parent: &Path, name: &str) -> Result<Self> {
        let p = parent.join(name);
        if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
        }
        create_private_dir(&p)?;
        Ok(Self(p))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a directory (and parents) with mode 0700 on Unix.
fn create_private_dir(p: &Path) -> Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)
        .with_context(|| format!("creating {}", p.display()))
}

/// Removes leftovers in the temporary directory (from a crash).
pub fn clean_tmp_dir(dir: &Path) {
    if dir.exists()
        && let Err(e) = std::fs::remove_dir_all(dir)
    {
        tracing::warn!(error = %e, dir = %dir.display(), "cannot clean temporary directory");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> WorkerOptions {
        WorkerOptions::from_config(&Config::default())
    }

    fn file(id: i64, t: FileType) -> LibraryFile {
        LibraryFile {
            id,
            file_type: t,
            size: Some(1000),
            updation_time: id,
            duration_secs: Some(10),
            stream_version: None,
        }
    }

    const V: MlVersions = MlVersions { face: 1, clip: 1 };

    #[test]
    fn plan_respects_types_and_limits() {
        let mut big = file(5, FileType::Image);
        big.size = Some(MAX_INDEXABLE_FILE_SIZE + 1);
        let mut long = file(6, FileType::Video);
        long.duration_secs = Some(61);
        let mut sv1 = file(7, FileType::Video);
        sv1.stream_version = Some(1);
        let files = vec![
            file(1, FileType::Image),
            file(2, FileType::Video),
            file(3, FileType::LivePhoto),
            file(4, FileType::Other),
            big,
            long,
            sv1,
        ];
        let p = plan(&files, &State::default(), &opts(), Some(V), true, 0);
        let ids = |v: &[LibraryFile]| v.iter().map(|f| f.id).collect::<Vec<_>>();
        assert_eq!(ids(&p.ml), vec![7, 6, 3, 2, 1]);
        assert_eq!(ids(&p.video), vec![2]);
        assert_eq!(p.ineligible, 4);

        let p = plan(&files, &State::default(), &opts(), None, false, 0);
        assert!(p.ml.is_empty() && p.video.is_empty());
    }

    #[test]
    fn plan_skips_done() {
        let mut st = State::default();
        let f = file(1, FileType::Image);
        st.mark_done(
            f.key(),
            TaskKind::Ml,
            Completion::Uploaded,
            Versions {
                face: Some(1),
                clip: Some(1),
            },
            0,
        );
        assert!(
            plan(std::slice::from_ref(&f), &st, &opts(), Some(V), true, 0)
                .ml
                .is_empty()
        );
        let mut changed = f;
        changed.updation_time += 1;
        assert_eq!(plan(&[changed], &st, &opts(), Some(V), true, 0).ml.len(), 1);
    }

    #[test]
    fn remote_satisfaction() {
        let r = RemoteMlData {
            raw: serde_json::json!({"face": {"version": 1}, "clip": {"version": 1}})
                .as_object()
                .unwrap()
                .clone(),
            updated_at: 5,
        };
        let both = Versions {
            face: Some(1),
            clip: Some(1),
        };
        assert!(remote_ml_satisfies(&r, both));
        assert!(!remote_ml_satisfies(
            &r,
            Versions {
                face: Some(2),
                clip: Some(1)
            }
        ));
        let only_face = RemoteMlData {
            raw: serde_json::json!({"face": {"version": 1}})
                .as_object()
                .unwrap()
                .clone(),
            updated_at: 0,
        };
        assert!(!remote_ml_satisfies(&only_face, both));
        assert!(remote_ml_satisfies(
            &only_face,
            Versions {
                face: Some(1),
                clip: None
            }
        ));
    }

    #[test]
    fn permanent_detection() {
        assert!(is_permanent(&permanent("x")));
        assert!(is_permanent(&permanent("x").context("outer")));
        assert!(!is_permanent(&anyhow!("x")));
    }
}
