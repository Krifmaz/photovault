//! Worker tests against mock library, fetcher, store, indexer and transcoder:
//! full passes, remote-skip rules, 409 handling, retries, pause/resume and
//! cancellation of in-flight transcodes, shutdown and concurrency bounds.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use photovault_node::config::Config;
use photovault_node::schedule::{Decision, GpuJobs, PauseReason};
use photovault_node::state::{Completion, StateStore, TaskKind, TaskRecord};
use photovault_node::worker::{
    FetchKind, FileDataStore, FileFetcher, FileType, LibraryFile, LibrarySnapshot, LibrarySource,
    MlIndexOutput, MlIndexer, MlRun, MlVersions, Phase, PutOutcome, RemoteMlData, Services,
    TranscodeOutcome, VideoPreview, VideoTranscoder, Worker, WorkerOptions, permanent,
};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockLibrary {
    files: Mutex<Vec<LibraryFile>>,
    syncs: AtomicUsize,
    cursors: Mutex<Vec<Option<Value>>>,
}

#[async_trait]
impl LibrarySource for MockLibrary {
    async fn sync(&self, cursor: Option<Value>) -> Result<LibrarySnapshot> {
        let n = self.syncs.fetch_add(1, Ordering::SeqCst);
        self.cursors.lock().unwrap().push(cursor);
        Ok(LibrarySnapshot {
            files: self.files.lock().unwrap().clone(),
            cursor: json!({ "sync": n + 1 }),
        })
    }
}

#[derive(Default)]
struct MockFetcher {
    calls: Mutex<Vec<(i64, FetchKind)>>,
    transient: Mutex<HashSet<i64>>,
    permanent: Mutex<HashSet<i64>>,
}

#[async_trait]
impl FileFetcher for MockFetcher {
    async fn fetch(&self, file: &LibraryFile, kind: FetchKind, dest: &Path) -> Result<()> {
        self.calls.lock().unwrap().push((file.id, kind));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dest.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "plaintext dir must be private");
        }
        if self.permanent.lock().unwrap().contains(&file.id) {
            return Err(permanent("cannot decrypt"));
        }
        if self.transient.lock().unwrap().contains(&file.id) {
            return Err(anyhow!("connection reset"));
        }
        std::fs::write(dest, b"plaintext")?;
        Ok(())
    }
}

/// `(file id, data, last_updated_at)` of one `put_ml` call.
type PutMlCall = (i64, Map<String, Value>, i64);

#[derive(Default)]
struct MockStore {
    remote_ml: Mutex<HashMap<i64, RemoteMlData>>,
    previews: Mutex<HashSet<i64>>,
    put_ml: Mutex<Vec<PutMlCall>>,
    put_video: Mutex<Vec<i64>>,
    conflict: Mutex<HashSet<i64>>,
}

#[async_trait]
impl FileDataStore for MockStore {
    async fn remote_ml(&self, files: &[LibraryFile]) -> Result<HashMap<i64, RemoteMlData>> {
        assert!(files.len() <= photovault_node::worker::REMOTE_BATCH);
        let m = self.remote_ml.lock().unwrap();
        Ok(files
            .iter()
            .filter_map(|f| m.get(&f.id).map(|r| (f.id, r.clone())))
            .collect())
    }
    async fn video_previews(&self, files: &[LibraryFile]) -> Result<HashSet<i64>> {
        let p = self.previews.lock().unwrap();
        Ok(files
            .iter()
            .map(|f| f.id)
            .filter(|id| p.contains(id))
            .collect())
    }
    async fn put_ml(
        &self,
        file: &LibraryFile,
        data: &Map<String, Value>,
        last_updated_at: i64,
    ) -> Result<PutOutcome> {
        self.put_ml
            .lock()
            .unwrap()
            .push((file.id, data.clone(), last_updated_at));
        if self.conflict.lock().unwrap().contains(&file.id) {
            return Ok(PutOutcome::Conflict);
        }
        Ok(PutOutcome::Stored)
    }
    async fn put_video_preview(
        &self,
        file: &LibraryFile,
        preview: &VideoPreview,
    ) -> Result<PutOutcome> {
        assert!(preview.dir.join("playlist.m3u8").exists());
        self.put_video.lock().unwrap().push(file.id);
        if self.conflict.lock().unwrap().contains(&file.id) {
            return Ok(PutOutcome::Conflict);
        }
        Ok(PutOutcome::Stored)
    }
}

#[derive(Default)]
struct MockIndexer {
    runs: Mutex<Vec<(i64, MlRun)>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    delay_ms: u64,
}

#[async_trait]
impl MlIndexer for MockIndexer {
    fn versions(&self) -> MlVersions {
        MlVersions { face: 1, clip: 1 }
    }
    async fn index(&self, file: &LibraryFile, image: &Path, run: MlRun) -> Result<MlIndexOutput> {
        assert_eq!(std::fs::read(image)?, b"plaintext");
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now, Ordering::SeqCst);
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.runs.lock().unwrap().push((file.id, run));
        Ok(MlIndexOutput {
            face: run
                .faces
                .then(|| json!({"version": 1, "client": "photovault-node", "faces": []})),
            clip: run
                .clip
                .then(|| json!({"version": 1, "client": "photovault-node", "embedding": [0.0]})),
        })
    }
}

/// Blocks the first `block_first` transcodes until cancelled.
#[derive(Default)]
struct MockTranscoder {
    started: AtomicUsize,
    cancelled: AtomicUsize,
    completed: AtomicUsize,
    block_first: usize,
    started_notify: Notify,
    not_needed: Mutex<HashSet<i64>>,
}

#[async_trait]
impl VideoTranscoder for MockTranscoder {
    async fn transcode(
        &self,
        file: &LibraryFile,
        _input: &Path,
        out_dir: &Path,
        cancel: CancellationToken,
    ) -> Result<TranscodeOutcome> {
        let n = self.started.fetch_add(1, Ordering::SeqCst);
        self.started_notify.notify_waiters();
        if n < self.block_first {
            // Counted whether the worker drops this future or it returns.
            let _guard = DropCounter(&self.cancelled);
            cancel.cancelled().await;
            return Err(anyhow!("cancelled"));
        }
        if self.not_needed.lock().unwrap().contains(&file.id) {
            return Ok(TranscodeOutcome::NotNeeded("already h264".into()));
        }
        std::fs::write(out_dir.join("playlist.m3u8"), "#EXTM3U")?;
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(TranscodeOutcome::Preview(VideoPreview {
            dir: out_dir.to_path_buf(),
            details: json!({}),
        }))
    }
}

/// Increments a counter when dropped.
struct DropCounter<'a>(&'a AtomicUsize);

impl Drop for DropCounter<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    lib: Arc<MockLibrary>,
    fetcher: Arc<MockFetcher>,
    store: Arc<MockStore>,
    indexer: Arc<MockIndexer>,
    transcoder: Arc<MockTranscoder>,
    gate_tx: watch::Sender<Decision>,
    dir: tempfile::TempDir,
}

impl Harness {
    fn new(indexer: MockIndexer, transcoder: MockTranscoder) -> Self {
        Self {
            lib: Arc::default(),
            fetcher: Arc::default(),
            store: Arc::default(),
            indexer: Arc::new(indexer),
            transcoder: Arc::new(transcoder),
            gate_tx: watch::channel(Decision::Run).0,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn set_files(&self, files: Vec<LibraryFile>) {
        *self.lib.files.lock().unwrap() = files;
    }

    fn opts(&self) -> WorkerOptions {
        let cfg = Config {
            data_dir: self.dir.path().to_path_buf(),
            ..Default::default()
        };
        let mut o = WorkerOptions::from_config(&cfg);
        o.save_every = Duration::ZERO;
        o
    }

    fn worker_with(&self, opts: WorkerOptions) -> Worker {
        let services = Services {
            library: self.lib.clone(),
            fetcher: self.fetcher.clone(),
            store: self.store.clone(),
            ml: Some(self.indexer.clone()),
            video: Some(self.transcoder.clone()),
        };
        let store = StateStore::load(self.dir.path().join("state.json")).unwrap();
        Worker::new(
            services,
            opts,
            store,
            self.gate_tx.subscribe(),
            GpuJobs::default(),
        )
    }

    fn worker(&self) -> Worker {
        self.worker_with(self.opts())
    }

    fn fetches(&self) -> usize {
        self.fetcher.calls.lock().unwrap().len()
    }

    fn pause(&self) {
        self.gate_tx.send_replace(Decision::Pause {
            reason: PauseReason::Game {
                process: "game.exe".into(),
            },
        });
    }

    fn resume(&self) {
        self.gate_tx.send_replace(Decision::Run);
    }
}

fn image(id: i64) -> LibraryFile {
    LibraryFile {
        id,
        file_type: FileType::Image,
        size: Some(2_000_000),
        updation_time: 1_000 + id,
        duration_secs: None,
        stream_version: None,
    }
}

fn video(id: i64) -> LibraryFile {
    LibraryFile {
        id,
        file_type: FileType::Video,
        size: Some(20_000_000),
        updation_time: 1_000 + id,
        duration_secs: Some(30),
        stream_version: None,
    }
}

async fn wait_until(mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_pass_then_nothing_to_do() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1), image(2), video(3)]);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.files, 3);
    assert_eq!(s.completed, 4, "3 ML + 1 video");
    assert_eq!(s.failed, 0);

    // Videos are indexed from the thumbnail, images from the original.
    let calls = h.fetcher.calls.lock().unwrap().clone();
    assert!(calls.contains(&(3, FetchKind::Thumbnail)));
    assert!(calls.contains(&(3, FetchKind::Original)));
    assert!(calls.contains(&(1, FetchKind::Original)));
    assert_eq!(h.store.put_ml.lock().unwrap().len(), 3);
    assert_eq!(*h.store.put_video.lock().unwrap(), vec![3]);
    assert_eq!(w.state().counters.ml_uploaded, 3);
    assert_eq!(w.state().counters.video_uploaded, 1);

    // Temporary plaintext is gone.
    let tmp = h.dir.path().join("tmp");
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);

    // State persisted; a new worker does nothing and passes the cursor on.
    drop(w);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.completed, 0);
    assert_eq!(h.fetches(), 4);
    let cursors = h.lib.cursors.lock().unwrap().clone();
    assert_eq!(cursors, vec![None, Some(json!({"sync": 1}))]);

    // Status file written for `status`.
    let st: serde_json::Value =
        serde_json::from_slice(&std::fs::read(h.dir.path().join("status.json")).unwrap()).unwrap();
    assert_eq!(st["phase"], "stopped");
}

#[tokio::test]
async fn changed_file_is_redone() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1)]);
    let mut w = h.worker();
    w.run_once(CancellationToken::new()).await.unwrap();
    let mut f = image(1);
    f.updation_time += 5;
    h.set_files(vec![f]);
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.completed, 1);
    assert_eq!(h.store.put_ml.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn remote_data_is_respected_and_preserved() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1), image(2), video(3)]);
    {
        let mut r = h.store.remote_ml.lock().unwrap();
        // Complete: skipped without download.
        r.insert(
            1,
            RemoteMlData {
                raw: json!({"face": {"version": 1}, "clip": {"version": 1}})
                    .as_object()
                    .unwrap()
                    .clone(),
                updated_at: 10,
            },
        );
        // Only faces, plus a key from a newer client: CLIP only, keys kept.
        r.insert(
            2,
            RemoteMlData {
                raw: json!({"face": {"version": 1, "faces": [1]}, "future": {"x": 1}})
                    .as_object()
                    .unwrap()
                    .clone(),
                updated_at: 77,
            },
        );
    }
    h.store.previews.lock().unwrap().insert(3);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.already_remote, 2, "file 1 ML and file 3 video");
    let calls = h.fetcher.calls.lock().unwrap().clone();
    assert!(!calls.iter().any(|(id, _)| *id == 1));
    assert!(!calls.contains(&(3, FetchKind::Original)));

    let runs = h.indexer.runs.lock().unwrap().clone();
    let run2 = runs.iter().find(|(id, _)| *id == 2).unwrap().1;
    assert!(!run2.faces && run2.clip);

    let puts = h.store.put_ml.lock().unwrap().clone();
    let (_, data, last) = puts.iter().find(|(id, _, _)| *id == 2).unwrap();
    assert_eq!(*last, 77);
    assert_eq!(data["face"]["faces"], json!([1]));
    assert_eq!(data["future"], json!({"x": 1}));
    assert_eq!(data["clip"]["version"], json!(1));
    assert_eq!(w.state().counters.ml_already_remote, 1);
    assert_eq!(w.state().counters.video_already_remote, 1);
}

#[tokio::test]
async fn conflict_is_never_reuploaded() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1), video(2)]);
    h.store.conflict.lock().unwrap().extend([1, 2]);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.failed, 0);
    // The video is ML-indexed too (from its thumbnail): two ML conflicts.
    assert_eq!(w.state().counters.ml_conflicts, 2);
    assert_eq!(w.state().counters.video_conflicts, 1);
    let rec = w
        .state()
        .task(image(1).key(), TaskKind::Ml)
        .cloned()
        .unwrap();
    assert!(matches!(
        rec,
        TaskRecord::Done {
            how: Completion::Conflict,
            ..
        }
    ));
    for _ in 0..2 {
        w.run_once(CancellationToken::new()).await.unwrap();
    }
    assert_eq!(h.store.put_ml.lock().unwrap().len(), 2);
    assert_eq!(h.store.put_video.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failures_retry_with_backoff_and_permanent_gives_up() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1), image(2), image(3)]);
    h.fetcher.transient.lock().unwrap().insert(1);
    h.fetcher.permanent.lock().unwrap().insert(2);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!((s.completed, s.failed), (1, 2));
    match w.state().task(image(1).key(), TaskKind::Ml).unwrap() {
        TaskRecord::Failed {
            attempts,
            next_retry_at,
            last_error,
        } => {
            assert_eq!(*attempts, 1);
            assert!(next_retry_at.is_some());
            assert!(last_error.contains("connection reset"), "{last_error}");
        }
        r => panic!("{r:?}"),
    }
    assert!(matches!(
        w.state().task(image(2).key(), TaskKind::Ml).unwrap(),
        TaskRecord::Failed {
            next_retry_at: None,
            ..
        }
    ));
    // Not due yet: the next pass does not touch them.
    let before = h.fetches();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.failed, 0);
    assert_eq!(h.fetches(), before);
}

#[tokio::test]
async fn video_not_needed_and_ineligible() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    let mut long = video(2);
    long.duration_secs = Some(600);
    let mut marked = video(3);
    marked.stream_version = Some(1);
    h.set_files(vec![video(1), long, marked]);
    h.transcoder.not_needed.lock().unwrap().insert(1);
    let mut w = h.worker();
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.ineligible, 2);
    assert_eq!(w.state().counters.video_not_needed, 1);
    assert!(h.store.put_video.lock().unwrap().is_empty());

    // With limits off, the long video is processed.
    let mut o = h.opts();
    o.video_max_duration_secs = None;
    o.tasks.faces = false;
    o.tasks.clip = false;
    let mut w = h.worker_with(o);
    w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(*h.store.put_video.lock().unwrap(), vec![2]);
}

#[tokio::test]
async fn waits_while_paused_then_resumes() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1), image(2)]);
    h.pause();
    let mut w = h.worker();
    let mut status = w.subscribe();
    let task = tokio::spawn(async move {
        let s = w.run_once(CancellationToken::new()).await.unwrap();
        (w, s)
    });
    // Reaches the paused phase without fetching anything.
    tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| s.phase == Phase::Paused),
    )
    .await
    .unwrap()
    .unwrap();
    let snap = status.borrow().clone();
    assert_eq!(snap.queued_ml, 2);
    assert!(!snap.decision.is_run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.fetches(), 0);

    h.resume();
    let (_w, s) = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.completed, 2);
}

#[tokio::test]
async fn pause_cancels_inflight_transcode_and_requeues() {
    let h = Harness::new(
        MockIndexer::default(),
        MockTranscoder {
            block_first: 1,
            ..Default::default()
        },
    );
    h.set_files(vec![video(1)]);
    let mut o = h.opts();
    o.tasks.faces = false;
    o.tasks.clip = false;
    let mut w = h.worker_with(o);
    let task = tokio::spawn(async move { w.run_once(CancellationToken::new()).await.unwrap() });

    let t = h.transcoder.clone();
    wait_until(|| t.started.load(Ordering::SeqCst) == 1).await;
    h.pause();
    wait_until(|| t.cancelled.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        t.started.load(Ordering::SeqCst),
        1,
        "no restart while paused"
    );

    h.resume();
    let s = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.completed, 1);
    assert_eq!(s.failed, 0, "a pause is not a failure");
    assert_eq!(t.started.load(Ordering::SeqCst), 2);
    assert_eq!(*h.store.put_video.lock().unwrap(), vec![1]);
}

#[tokio::test]
async fn shutdown_stops_promptly_and_saves() {
    let h = Harness::new(
        MockIndexer::default(),
        MockTranscoder {
            block_first: usize::MAX,
            ..Default::default()
        },
    );
    h.set_files(vec![video(1), video(2)]);
    let mut o = h.opts();
    o.tasks.faces = false;
    o.tasks.clip = false;
    let mut w = h.worker_with(o);
    let shutdown = CancellationToken::new();
    let sd = shutdown.clone();
    let task = tokio::spawn(async move {
        let s = w.run_once(sd).await.unwrap();
        (w, s)
    });
    let t = h.transcoder.clone();
    wait_until(|| t.started.load(Ordering::SeqCst) == 1).await;
    shutdown.cancel();
    let (w, s) = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.completed, 0);
    assert_eq!(s.failed, 0);
    assert_eq!(s.unfinished, 2);
    assert!(w.state().task(video(1).key(), TaskKind::Video).is_none());
    assert!(h.dir.path().join("state.json").exists());
    assert_eq!(
        std::fs::read_dir(h.dir.path().join("tmp")).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn concurrency_is_bounded() {
    let h = Harness::new(
        MockIndexer {
            delay_ms: 20,
            ..Default::default()
        },
        MockTranscoder::default(),
    );
    h.set_files((1..=12).map(image).collect());
    let mut o = h.opts();
    o.concurrency.ml = 3;
    let mut w = h.worker_with(o);
    let s = w.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(s.completed, 12);
    let max = h.indexer.max_active.load(Ordering::SeqCst);
    assert!((2..=3).contains(&max), "max concurrent = {max}");
}

#[tokio::test]
async fn run_loop_stops_on_shutdown() {
    let h = Harness::new(MockIndexer::default(), MockTranscoder::default());
    h.set_files(vec![image(1)]);
    let mut o = h.opts();
    o.poll_interval = Duration::from_millis(20);
    let mut w = h.worker_with(o);
    let shutdown = CancellationToken::new();
    let sd = shutdown.clone();
    let task = tokio::spawn(async move { w.run(sd).await });
    let lib = h.lib.clone();
    wait_until(|| lib.syncs.load(Ordering::SeqCst) >= 3).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // Indexed once, then only re-synced.
    assert_eq!(h.store.put_ml.lock().unwrap().len(), 1);
}
