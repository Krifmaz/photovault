//! ML data sync and on-device indexing.
//!
//! - Switch: Ente's remote flag `faceSearchEnabled` (`/remote-store`),
//!   cached in kv [`kv::ENABLED`] (web `services/ml/index.ts`).
//! - Pull: `/files/data/status-diff` from the cursor in
//!   [`kv::STATUS_SINCE`], then `/files/data/fetch` in batches of 200,
//!   decrypted with the file key ([`super::mldata::open`]) and stored
//!   sealed ([`crate::store::Store::put_ml_data`]). The diff lists only the
//!   account's own files, so library files that were never looked up are
//!   "hydrated" with a direct fetch, and files shared by others are
//!   re-checked once a day (mobile `ml_util.dart`).
//! - Models: [`super::models`], from the user's server only.
//! - Index (feature `ml`): Ente's `ente_ml::indexing::analyze_image` on the
//!   account's own images, Live Photos (image part) and videos (thumbnail),
//!   at most 100 MB, one file at a time on a blocking thread; the result is
//!   merged into the existing object ([`super::mldata::merge`]), uploaded
//!   with `PUT /files/data` (`lastUpdatedAt` for optimistic concurrency)
//!   and stored. Mirrors web `services/ml/worker.ts` (`index`) and mobile
//!   `ml_service.dart`.
//! - After data changes: [`super::entity::sync`] (`cgroup`, `location`),
//!   [`super::faces::refresh`], [`super::search::invalidate`],
//!   [`super::memories::invalidate`].
//!
//! Local indexing state lives in `ml_local` ([`local_state`]): `pending`
//! (to index here), `failed` (retried until [`MAX_ATTEMPTS`]), `indexed`
//! (computed, upload still to do), `done`, `skipped` (not ours, too large,
//! unsupported).
//!
//! Based on Ente (ente.com).

mod livephoto;

use super::filedata::{self, ML_REMOTE_KEY};
#[cfg(feature = "ml")]
use super::mldata::flags;
use super::mldata::{self, Face, IndexPlan, RemoteClipIndex, RemoteFaceIndex, RemoteMlData};
use super::models;
use super::store::{kv, local_state};
use super::{MlListener, MlStatus};
use crate::client::Inner;
use crate::library::FileRec;
use crate::{Core, CoreError, FileKind, Result};
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Failed attempts after which a file is no longer retried.
pub(crate) const MAX_ATTEMPTS: u32 = 3;
/// Largest original indexed (`maxIndexableFileSize` on web,
/// `maxFileDownloadSize` on mobile). Videos use their thumbnail instead.
const MAX_INDEXABLE_FILE_SIZE: u64 = 100_000_000;
/// How often files shared by others are looked up again (mobile: daily).
const OTHERS_RECHECK_US: i64 = 24 * 3600 * 1_000_000;
/// Plain i64 kv: when files shared by others were last looked up.
const KV_OTHERS_CHECKED: &str = "ml:others_checked";
/// `ml_local.error` of files owned by someone else (never indexed here).
const NOT_OWNED: &str = "not owned";

/// Indexer state kept in memory.
#[derive(Debug, Default)]
pub(crate) struct IndexerState {
    live: Mutex<Live>,
    /// One pull at a time.
    sync_lock: tokio::sync::Mutex<()>,
    /// One indexing run at a time.
    run_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Default)]
struct Live {
    running: bool,
    last_error: Option<String>,
    /// ONNX Runtime failed to load or run: no inference in this process.
    runtime_broken: bool,
    /// Cached counts and the library index they were computed for.
    counts: Option<(usize, Counts)>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    total: u64,
    indexed: u64,
    pending: u64,
}

impl IndexerState {
    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ------------------------------------------------------------ analysis

/// Pixels to analyse.
#[doc(hidden)]
#[derive(Debug)]
pub enum ImageInput {
    /// A decrypted file on disk (deleted after the analysis).
    Path(PathBuf),
    /// Encoded image bytes (a thumbnail, a Live Photo's image).
    Bytes(Vec<u8>),
}

/// What the models found in one image.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct Analysis {
    /// Width of the decoded image.
    pub width: u32,
    /// Height of the decoded image.
    pub height: u32,
    /// Faces, when faces were requested.
    pub faces: Option<Vec<Face>>,
    /// CLIP image embedding, when requested.
    pub clip: Option<Vec<f64>>,
    /// Index flags ([`super::mldata::flags`]).
    pub flags: i64,
}

/// Why an analysis failed.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum AnalyzeError {
    /// Something about this image (decode, unsupported format): the file is
    /// marked failed and retried at most [`MAX_ATTEMPTS`] times.
    Image(String),
    /// The runtime or a model is broken: the run stops.
    Runtime(String),
}

/// Runs the models (Ente's `ente-ml` in the app; a stand-in in tests).
#[doc(hidden)]
pub trait Analyzer: Send + Sync {
    /// Analyse one image. Called on a blocking thread, one at a time.
    fn analyze(
        &self,
        file_id: i64,
        input: ImageInput,
        run_faces: bool,
        run_clip: bool,
    ) -> std::result::Result<Analysis, AnalyzeError>;
    /// Called on a blocking thread before the first and after the last
    /// file of a run.
    fn prepare(&self) {}
    /// See [`Analyzer::prepare`].
    fn release(&self) {}
}

/// Ente's Rust ML on ONNX Runtime.
#[cfg(feature = "ml")]
struct EnteAnalyzer {
    paths: ente_ml::ModelPaths,
}

#[cfg(feature = "ml")]
impl Analyzer for EnteAnalyzer {
    fn analyze(
        &self,
        file_id: i64,
        input: ImageInput,
        run_faces: bool,
        run_clip: bool,
    ) -> std::result::Result<Analysis, AnalyzeError> {
        use ente_ml::error::MlError;
        use ente_ml::indexing::{AnalyzeImageRequest, ImageSource, analyze_image};
        let source = match input {
            ImageInput::Path(p) => ImageSource::Path(p.to_string_lossy().into_owned()),
            ImageInput::Bytes(b) => ImageSource::Bytes(b),
        };
        let r = analyze_image(AnalyzeImageRequest {
            file_id,
            source,
            run_faces,
            run_clip,
            run_pets: false,
            generate_face_crops: false,
            model_paths: self.paths.clone(),
        })
        .map_err(|e| match e {
            MlError::Ort(_) | MlError::CorruptModel(_) | MlError::Runtime(_) => {
                AnalyzeError::Runtime(e.to_string())
            }
            _ => AnalyzeError::Image(e.to_string()),
        })?;
        // Exactly the web worker's `faceIndexFromAnalysisResult` and
        // `clipIndexFromAnalysisResult`.
        let faces = r.faces.map(|fs| {
            fs.iter()
                .map(|f| {
                    Face::from_rust_result(
                        f.face_id.clone(),
                        f.detection.box_xyxy,
                        &f.detection.keypoints,
                        f.detection.score,
                        f.blur_value,
                        &f.embedding,
                    )
                })
                .collect()
        });
        Ok(Analysis {
            width: r.decoded_image_size.width,
            height: r.decoded_image_size.height,
            faces,
            clip: r
                .clip
                .map(|c| c.embedding.iter().copied().map(f64::from).collect()),
            flags: flags::for_rust_result(r.used_coreml, r.used_webgpu),
        })
    }

    fn prepare(&self) {
        ente_ml::indexing::init_ml_runtime(self.paths.clone());
    }

    fn release(&self) {
        ente_ml::indexing::release_ml_runtime();
    }
}

// ------------------------------------------------------------ exports

#[uniffi::export]
impl Core {
    /// Current ML state (fast; from the store and memory).
    pub fn ml_status(&self) -> MlStatus {
        status(&self.inner)
    }

    /// Turn ML on or off for this account (Ente's remote flag
    /// `faceSearchEnabled`, shared with the Ente apps).
    pub async fn set_ml_enabled(&self, enabled: bool) -> Result<MlStatus, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move {
            let api = inner.api()?;
            filedata::update_remote_flag(&api, ML_REMOTE_KEY, enabled).await?;
            inner
                .store()
                .put_plain(kv::ENABLED, if enabled { "1" } else { "0" })?;
            Ok(status(&inner))
        })
        .await
    }

    /// Pull the `mldata` and people other devices produced, then recluster.
    /// Cheap when nothing changed; call after every library sync.
    pub async fn sync_ml(
        &self,
        listener: Option<Arc<dyn MlListener>>,
    ) -> Result<MlStatus, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move {
            sync(&inner, listener.as_ref()).await?;
            Ok(status(&inner))
        })
        .await
    }

    /// Download the models if needed and index up to `max_files` files on
    /// this device, uploading each result. Cancel by cancelling the
    /// coroutine. Returns the final status.
    pub async fn run_ml_indexing(
        &self,
        max_files: u32,
        listener: Option<Arc<dyn MlListener>>,
    ) -> Result<MlStatus, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move {
            sync(&inner, listener.as_ref()).await?;
            #[cfg(feature = "ml")]
            {
                let analyzer = Arc::new(EnteAnalyzer {
                    paths: models::indexing_paths(&inner.ml.models_dir),
                });
                run_index(&inner, max_files, listener.as_ref(), analyzer, true).await?;
            }
            #[cfg(not(feature = "ml"))]
            let _ = max_files;
            Ok(status(&inner))
        })
        .await
    }
}

impl Core {
    /// [`Core::run_ml_indexing`] with another analyzer and without the
    /// model download, for tests of everything around inference.
    #[doc(hidden)]
    pub async fn run_ml_indexing_with(
        &self,
        max_files: u32,
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<MlStatus, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move {
            sync(&inner, None).await?;
            run_index(&inner, max_files, None, analyzer, false).await?;
            Ok(status(&inner))
        })
        .await
    }
}

// ------------------------------------------------------------ status

fn is_indexable(kind: FileKind) -> bool {
    matches!(
        kind,
        FileKind::Image | FileKind::Video | FileKind::LivePhoto
    )
}

/// Forget cached counts (after the ML tables changed).
fn invalidate_counts(inner: &Inner) {
    inner.ml.index.live().counts = None;
}

fn compute_counts(inner: &Inner) -> Result<Counts> {
    let idx = inner.index();
    let ids: HashSet<i64> = idx
        .all_items()
        .iter()
        .filter(|i| is_indexable(i.kind))
        .map(|i| i.id)
        .collect();
    let st = inner.store();
    let mut indexed = HashSet::new();
    {
        let mut q = st
            .conn()
            .prepare("SELECT file_id FROM ml_data WHERE face_ok = 1 AND clip_ok = 1")?;
        for id in q.query_map([], |r| r.get::<_, i64>(0))? {
            let id = id?;
            if ids.contains(&id) {
                indexed.insert(id);
            }
        }
    }
    let mut pending = 0u64;
    {
        let mut q = st.conn().prepare(
            "SELECT file_id FROM ml_local WHERE state = ?1 OR (state = ?2 AND attempts < ?3)",
        )?;
        for id in q.query_map(
            params![local_state::PENDING, local_state::FAILED, MAX_ATTEMPTS],
            |r| r.get::<_, i64>(0),
        )? {
            let id = id?;
            if ids.contains(&id) && !indexed.contains(&id) {
                pending += 1;
            }
        }
    }
    Ok(Counts {
        total: ids.len() as u64,
        indexed: indexed.len() as u64,
        pending,
    })
}

fn counts(inner: &Inner) -> Counts {
    let key = Arc::as_ptr(&inner.index()) as usize;
    if let Some((k, c)) = inner.ml.index.live().counts
        && k == key
    {
        return c;
    }
    let c = compute_counts(inner).unwrap_or_default();
    inner.ml.index.live().counts = Some((key, c));
    c
}

fn is_enabled(inner: &Inner) -> bool {
    inner
        .store()
        .get_plain(kv::ENABLED)
        .ok()
        .flatten()
        .is_some_and(|v| v == "1")
}

/// The current [`MlStatus`]: two indexed queries (cached until the ML
/// tables or the library change) and a few `stat`s.
pub(crate) fn status(inner: &Inner) -> MlStatus {
    let c = counts(inner);
    let dir = &inner.ml.models_dir;
    let (done, total) = models::progress(dir, models::MODELS);
    let live = inner.ml.index.live();
    MlStatus {
        enabled: is_enabled(inner),
        inference_available: cfg!(feature = "ml") && !live.runtime_broken,
        models_ready: models::all_ready(dir, models::MODELS),
        model_bytes_done: done,
        model_bytes_total: total,
        files_total: c.total,
        files_indexed: c.indexed,
        files_pending: c.pending,
        running: live.running,
        last_error: live.last_error.clone(),
    }
}

fn notify(inner: &Inner, listener: Option<&Arc<dyn MlListener>>) {
    if let Some(l) = listener {
        l.on_status(status(inner));
    }
}

// ------------------------------------------------------------ store helpers

/// `(state, attempts, error)` of an `ml_local` row.
type LocalRow = (String, u32, Option<String>);

/// Every `ml_local` row.
fn local_rows(inner: &Inner) -> Result<HashMap<i64, LocalRow>> {
    let st = inner.store();
    let mut q = st
        .conn()
        .prepare("SELECT file_id, state, attempts, error FROM ml_local")?;
    let rows = q
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// `file_id -> complete` of every `ml_data` row (no unsealing).
fn data_rows(inner: &Inner) -> Result<HashMap<i64, bool>> {
    let st = inner.store();
    let mut q = st
        .conn()
        .prepare("SELECT file_id, face_ok AND clip_ok FROM ml_data")?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn delete_local(inner: &Inner, file_id: i64) -> Result<()> {
    inner
        .store()
        .conn()
        .execute("DELETE FROM ml_local WHERE file_id = ?1", params![file_id])?;
    Ok(())
}

fn set_local(
    inner: &Inner,
    file_id: i64,
    state: &str,
    attempts: u32,
    err: Option<&str>,
) -> Result<()> {
    inner.store().set_ml_local(file_id, state, attempts, err)
}

/// Run blocking work on the blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CoreError::storage(format!("task failed: {e}")))
}

fn file_type(rec: &FileRec) -> Option<i64> {
    rec.metadata
        .get("fileType")
        .and_then(serde_json::Value::as_i64)
}

// ------------------------------------------------------------ pull

/// Pull the switch, changed `mldata`, never-seen files and entities, then
/// rebuild what depends on them.
async fn sync(inner: &Arc<Inner>, listener: Option<&Arc<dyn MlListener>>) -> Result<()> {
    let _g = inner.ml.index.sync_lock.lock().await;
    let api = inner.api()?;
    match filedata::get_remote_flag(&api, ML_REMOTE_KEY).await {
        Ok(on) => inner
            .store()
            .put_plain(kv::ENABLED, if on { "1" } else { "0" })?,
        // Offline: keep the cached value.
        Err(e) if filedata::is_transient(&e) => {}
        Err(e) => return Err(e),
    }
    if !is_enabled(inner) {
        notify(inner, listener);
        return Ok(());
    }
    let user_id = inner.session()?.account.user_id;
    let mut changed = pull_diff(inner, &api, user_id, listener).await?;
    changed |= hydrate(inner, &api, user_id, listener).await?;
    let me = inner.clone();
    changed |= blocking(move || prune(&me)).await??;
    invalidate_counts(inner);
    notify(inner, listener);

    let cgroups = super::entity::sync(inner, super::entity::CGROUP).await;
    let locations = super::entity::sync(inner, super::entity::LOCATION).await;
    changed |= matches!(cgroups, Ok(true)) || matches!(locations, Ok(true));
    // Reclustering and the people snapshot cost seconds on a large
    // library: only when something changed, the library index moved, or
    // a push to people is still due (this runs before every indexing
    // batch too).
    if changed || super::faces::needs_refresh(inner) {
        data_changed(inner).await?;
    }
    cgroups?;
    locations?;
    notify(inner, listener);
    Ok(())
}

/// Recluster, push faces new to people's clusters to their `cgroup`s
/// ([`super::faces::refresh`]), and drop the search and memories caches.
/// When the push fails (offline) the local rebuild still runs, so new
/// faces show up; the push is retried on the next sync.
async fn data_changed(inner: &Arc<Inner>) -> Result<()> {
    let refreshed = super::faces::refresh(inner).await;
    let result = match refreshed {
        Ok(()) => Ok(()),
        Err(e) => {
            super::faces::rebuild_now(inner).await?;
            Err(e)
        }
    };
    super::search::invalidate(inner);
    super::memories::invalidate(inner);
    result
}

/// `/files/data/status-diff` from the saved cursor to the end. Returns
/// whether the diff listed anything.
async fn pull_diff(
    inner: &Arc<Inner>,
    api: &crate::api::Api,
    user_id: i64,
    listener: Option<&Arc<dyn MlListener>>,
) -> Result<bool> {
    let mut changed = false;
    let mut since: i64 = inner
        .store()
        .get_plain(kv::STATUS_SINCE)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if since > 0 {
        // The ML tables were wiped (device key reset) but the plain cursor
        // survived: pull everything again.
        let empty: bool = inner.store().conn().query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM ml_data) AND NOT EXISTS (SELECT 1 FROM ml_local)",
            [],
            |r| r.get(0),
        )?;
        if empty {
            since = 0;
        }
    }
    loop {
        let page = filedata::status_diff(api, since).await?;
        if page.is_empty() {
            break;
        }
        changed = true;
        let mut next = since;
        let mut want = Vec::new();
        let mut deleted = Vec::new();
        for s in &page {
            next = next.max(s.updated_at);
            if s.kind != mldata::FILE_DATA_TYPE {
                continue;
            }
            if s.is_deleted {
                deleted.push(s.file_id);
            } else {
                want.push(s.file_id);
            }
        }
        want.sort_unstable();
        want.dedup();
        for chunk in want.chunks(filedata::FETCH_BATCH) {
            fetch_and_store(inner, api, user_id, chunk, false).await?;
        }
        for id in deleted {
            inner.store().delete_ml_data(id)?;
            delete_local(inner, id)?;
        }
        inner
            .store()
            .put_plain(kv::STATUS_SINCE, &next.to_string())?;
        invalidate_counts(inner);
        notify(inner, listener);
        if next <= since {
            break;
        }
        since = next;
    }
    Ok(changed)
}

/// Fetch, decrypt and store the `mldata` of `ids` (files of the library;
/// others are skipped). With `mark_missing`, files without usable remote
/// data are queued for indexing (own files) or marked as someone else's.
/// Returns the ids that had data.
async fn fetch_and_store(
    inner: &Arc<Inner>,
    api: &crate::api::Api,
    user_id: i64,
    ids: &[i64],
    mark_missing: bool,
) -> Result<HashSet<i64>> {
    let me = inner.clone();
    let wanted = ids.to_vec();
    let recs: HashMap<i64, FileRec> = blocking(move || {
        wanted
            .into_iter()
            .filter_map(|id| me.file_rec(id).ok().map(|r| (id, r)))
            .collect()
    })
    .await?;
    let known: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|i| recs.contains_key(i))
        .collect();
    let remote = filedata::fetch(api, mldata::FILE_DATA_TYPE, &known).await?;
    let me = inner.clone();
    blocking(move || -> Result<HashSet<i64>> {
        let local = local_rows(&me)?;
        let mut found = HashSet::new();
        for fd in remote {
            let Some(rec) = recs.get(&fd.file_id) else {
                continue;
            };
            let Some(d) = open_remote(&fd, &rec.key) else {
                // Corrupt remote data is ignored so indexing replaces it.
                continue;
            };
            me.store().put_ml_data(fd.file_id, &d)?;
            found.insert(fd.file_id);
            let owned = rec.owner_id == user_id;
            let row = local.get(&fd.file_id);
            if d.plan().is_done() {
                if row.is_some_and(|r| r.0 != local_state::SKIPPED) {
                    delete_local(&me, fd.file_id)?;
                }
            } else if owned {
                let (state, attempts) = row.map_or((None, 0), |r| (Some(r.0.as_str()), r.1));
                let retry_out = state == Some(local_state::FAILED) && attempts >= MAX_ATTEMPTS;
                if state != Some(local_state::SKIPPED) && !retry_out {
                    set_local(&me, fd.file_id, local_state::PENDING, attempts, None)?;
                }
            } else if row.is_none() {
                set_local(&me, fd.file_id, local_state::SKIPPED, 0, Some(NOT_OWNED))?;
            }
        }
        if mark_missing {
            for (id, rec) in &recs {
                if found.contains(id) || local.contains_key(id) {
                    continue;
                }
                if rec.owner_id == user_id {
                    set_local(&me, *id, local_state::PENDING, 0, None)?;
                } else {
                    set_local(&me, *id, local_state::SKIPPED, 0, Some(NOT_OWNED))?;
                }
            }
        }
        Ok(found)
    })
    .await?
}

fn open_remote(fd: &filedata::RemoteFileData, key: &[u8]) -> Option<RemoteMlData> {
    let ct = crate::crypto::unb64(&fd.encrypted_data).ok()?;
    let header = crate::crypto::unb64(&fd.decryption_header).ok()?;
    mldata::open(&ct, &header, key, fd.updated_at).ok()
}

/// Look up library files that were never looked up, and once a day the
/// files shared by others (whose `mldata` the diff does not list).
/// Returns whether any `mldata` was stored.
async fn hydrate(
    inner: &Arc<Inner>,
    api: &crate::api::Api,
    user_id: i64,
    listener: Option<&Arc<dyn MlListener>>,
) -> Result<bool> {
    let mut changed = false;
    let now = crate::store::now_us();
    let me = inner.clone();
    let (unchecked, others) = blocking(move || -> Result<(Vec<i64>, Option<Vec<i64>>)> {
        let data = data_rows(&me)?;
        let local = local_rows(&me)?;
        let last: i64 = me
            .store()
            .get_plain(KV_OTHERS_CHECKED)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let others_due = now.saturating_sub(last) >= OTHERS_RECHECK_US;
        let idx = me.index();
        let mut unchecked = Vec::new();
        let mut others = Vec::new();
        for it in idx.all_items().iter().filter(|i| is_indexable(i.kind)) {
            if data.contains_key(&it.id) {
                continue;
            }
            match local.get(&it.id) {
                None => unchecked.push(it.id),
                Some((s, _, Some(e))) if s == local_state::SKIPPED && e == NOT_OWNED => {
                    others.push(it.id);
                }
                _ => {}
            }
        }
        Ok((unchecked, others_due.then_some(others)))
    })
    .await??;
    for chunk in unchecked.chunks(filedata::FETCH_BATCH) {
        changed |= !fetch_and_store(inner, api, user_id, chunk, true)
            .await?
            .is_empty();
        invalidate_counts(inner);
        notify(inner, listener);
    }
    if let Some(others) = others {
        for chunk in others.chunks(filedata::FETCH_BATCH) {
            changed |= !fetch_and_store(inner, api, user_id, chunk, false)
                .await?
                .is_empty();
        }
        inner
            .store()
            .put_plain(KV_OTHERS_CHECKED, &now.to_string())?;
    }
    Ok(changed)
}

/// Drop ML rows of files no longer in the library. Returns whether any
/// `mldata` was dropped.
fn prune(inner: &Inner) -> Result<bool> {
    let idx = inner.index();
    let ids: HashSet<i64> = idx.all_items().iter().map(|i| i.id).collect();
    let data = data_rows(inner)?;
    let local = local_rows(inner)?;
    let st = inner.store();
    let mut changed = false;
    for id in data.keys().filter(|i| !ids.contains(i)) {
        st.delete_ml_data(*id)?;
        changed = true;
    }
    for id in local.keys().filter(|i| !ids.contains(i)) {
        st.conn()
            .execute("DELETE FROM ml_local WHERE file_id = ?1", params![id])?;
    }
    Ok(changed)
}

// ------------------------------------------------------------ indexing

/// Clears `running` when the run ends or is cancelled.
struct Running<'a>(&'a Inner);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.ml.index.live().running = false;
    }
}

/// Deletes a decrypted temporary file when dropped (also on cancel).
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What happened to one file.
enum Outcome {
    /// Indexed (and uploaded, or kept for upload).
    Indexed,
    /// Nothing to do after all (remote data is current, not ours, …).
    Skipped,
    /// The file failed; the run goes on.
    Failed,
}

/// Why a run stops early.
enum Stop {
    /// Network or server trouble: retry later.
    Transient(CoreError),
    /// The ML runtime is broken.
    Runtime(String),
    /// Local storage error.
    Fatal(CoreError),
}

impl From<CoreError> for Stop {
    fn from(e: CoreError) -> Self {
        if filedata::is_transient(&e) {
            Stop::Transient(e)
        } else {
            Stop::Fatal(e)
        }
    }
}

/// Serialises inference across runs (a cancelled run's blocking analysis
/// may still be finishing when the next run starts).
static INFERENCE: Mutex<()> = Mutex::new(());

async fn run_index(
    inner: &Arc<Inner>,
    max_files: u32,
    listener: Option<&Arc<dyn MlListener>>,
    analyzer: Arc<dyn Analyzer>,
    needs_models: bool,
) -> Result<()> {
    if !is_enabled(inner) || inner.ml.index.live().runtime_broken {
        return Ok(());
    }
    let Ok(_g) = inner.ml.index.run_lock.try_lock() else {
        // Another run is going on.
        return Ok(());
    };
    inner.ml.index.live().running = true;
    let _running = Running(inner);
    notify(inner, listener);

    let tmp_dir = inner.cache_dir.join("ml-tmp");
    // Left over from a crash: decrypted originals must not linger.
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let r = index_files(
        inner,
        max_files,
        listener,
        &analyzer,
        needs_models,
        &tmp_dir,
    )
    .await;
    let (indexed, result) = match r {
        Ok(n) => {
            inner.ml.index.live().last_error = None;
            (n, Ok(()))
        }
        Err((n, Stop::Runtime(m))) => {
            let mut live = inner.ml.index.live();
            live.runtime_broken = true;
            live.last_error = Some(m);
            (n, Ok(()))
        }
        Err((n, Stop::Transient(e) | Stop::Fatal(e))) => {
            inner.ml.index.live().last_error = Some(e.to_string());
            (n, Err(e))
        }
    };
    let a = analyzer.clone();
    let _ = blocking(move || {
        let _l = INFERENCE.lock().unwrap_or_else(|e| e.into_inner());
        a.release();
    })
    .await;
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    invalidate_counts(inner);
    if indexed > 0 {
        data_changed(inner).await?;
    }
    drop(_running);
    notify(inner, listener);
    result
}

/// Models, then pending uploads, then up to `max_files` files. Returns how
/// many files were indexed.
async fn index_files(
    inner: &Arc<Inner>,
    max_files: u32,
    listener: Option<&Arc<dyn MlListener>>,
    analyzer: &Arc<dyn Analyzer>,
    needs_models: bool,
    tmp_dir: &std::path::Path,
) -> std::result::Result<u32, (u32, Stop)> {
    let mut n = 0u32;
    if needs_models {
        let base = inner
            .active_base_url()
            .ok_or((0, Stop::Fatal(CoreError::NotLoggedIn)))?;
        let last = Mutex::new(0u64);
        let me: &Inner = inner;
        let on = move |done: u64, total: u64| {
            let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
            // About every 1% (and at the end).
            if done == total || done.saturating_sub(*l) >= total / 100 {
                *l = done;
                notify(me, listener);
            }
        };
        models::ensure(&base, &me.ml.models_dir, models::MODELS, &on)
            .await
            .map_err(|e| (0, Stop::from(e)))?;
    }
    let api = inner.api().map_err(|e| (0, Stop::Fatal(e)))?;
    let user_id = inner
        .session()
        .map_err(|e| (0, Stop::Fatal(e)))?
        .account
        .user_id;

    upload_pending(inner, &api).await.map_err(|s| (0, s))?;

    let me = inner.clone();
    let candidates = blocking(move || candidates(&me, max_files as usize))
        .await
        .and_then(|r| r)
        .map_err(|e| (0, Stop::Fatal(e)))?;
    if candidates.is_empty() {
        return Ok(0);
    }
    let a = analyzer.clone();
    blocking(move || {
        let _l = INFERENCE.lock().unwrap_or_else(|e| e.into_inner());
        a.prepare();
    })
    .await
    .map_err(|e| (0, Stop::Fatal(e)))?;
    for (file_id, attempts) in candidates {
        match index_one(inner, &api, user_id, analyzer, file_id, attempts, tmp_dir).await {
            Ok(Outcome::Indexed) => n += 1,
            Ok(Outcome::Skipped | Outcome::Failed) => {}
            Err(stop) => return Err((n, stop)),
        }
        invalidate_counts(inner);
        notify(inner, listener);
    }
    Ok(n)
}

/// Own files to index now, newest first: `pending`, or `failed` with
/// attempts left, still in the library and without current indexes.
fn candidates(inner: &Inner, max: usize) -> Result<Vec<(i64, u32)>> {
    let data = data_rows(inner)?;
    let local = local_rows(inner)?;
    let idx = inner.index();
    Ok(idx
        .all_items()
        .iter()
        .filter(|i| is_indexable(i.kind) && data.get(&i.id) != Some(&true))
        .filter_map(|i| {
            let (state, attempts, _) = local.get(&i.id)?;
            let go = state == local_state::PENDING
                || (state == local_state::FAILED && *attempts < MAX_ATTEMPTS);
            go.then_some((i.id, *attempts))
        })
        .take(max)
        .collect())
}

/// Upload results that were computed but not uploaded (`indexed`).
async fn upload_pending(
    inner: &Arc<Inner>,
    api: &crate::api::Api,
) -> std::result::Result<(), Stop> {
    let ids: Vec<i64> = local_rows(inner)?
        .into_iter()
        .filter(|(_, r)| r.0 == local_state::INDEXED)
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        let me = inner.clone();
        let loaded = blocking(move || -> Result<Option<(FileRec, RemoteMlData)>> {
            let Ok(rec) = me.file_rec(id) else {
                return Ok(None);
            };
            Ok(me.store().ml_data(id)?.map(|d| (rec, d)))
        })
        .await??;
        let Some((rec, d)) = loaded else {
            delete_local(inner, id)?;
            continue;
        };
        match upload(api, &rec, &d.raw, d.updated_at.unwrap_or(0)).await {
            Ok(()) => {
                let mut d = d;
                d.updated_at = None;
                inner.store().put_ml_data(id, &d)?;
                set_local(inner, id, local_state::DONE, 0, None)?;
            }
            Err(e) if filedata::is_conflict(&e) => {
                // Changed elsewhere meanwhile: fetch it and redo what is missing.
                inner.store().delete_ml_data(id)?;
                set_local(inner, id, local_state::PENDING, 0, None)?;
            }
            Err(e) if filedata::is_transient(&e) => return Err(Stop::Transient(e)),
            Err(e) => {
                inner.store().delete_ml_data(id)?;
                set_local(
                    inner,
                    id,
                    local_state::FAILED,
                    MAX_ATTEMPTS,
                    Some(&e.to_string()),
                )?;
            }
        }
    }
    Ok(())
}

/// Gzip, encrypt with the file key and `PUT /files/data`.
async fn upload(
    api: &crate::api::Api,
    rec: &FileRec,
    raw: &serde_json::Map<String, serde_json::Value>,
    last_updated_at: i64,
) -> Result<()> {
    let (raw, key) = (raw.clone(), rec.key.clone());
    let sealed = blocking(move || {
        let mut key = key;
        let r = mldata::seal(&raw, &key);
        crate::crypto::wipe(&mut key);
        r
    })
    .await?
    .map_err(|e| CoreError::crypto(e.to_string()))?;
    filedata::put(
        api,
        rec.id,
        mldata::FILE_DATA_TYPE,
        &sealed.0,
        &sealed.1,
        last_updated_at,
    )
    .await
}

/// Index one file (web `worker.ts` `index`).
#[allow(clippy::too_many_arguments)]
async fn index_one(
    inner: &Arc<Inner>,
    api: &crate::api::Api,
    user_id: i64,
    analyzer: &Arc<dyn Analyzer>,
    file_id: i64,
    attempts: u32,
    tmp_dir: &std::path::Path,
) -> std::result::Result<Outcome, Stop> {
    let me = inner.clone();
    let Ok(rec) = blocking(move || me.file_rec(file_id)).await? else {
        delete_local(inner, file_id)?;
        return Ok(Outcome::Skipped);
    };
    let fail = |msg: &str, permanent: bool| -> std::result::Result<Outcome, Stop> {
        let attempts = if permanent {
            MAX_ATTEMPTS
        } else {
            attempts + 1
        };
        set_local(inner, file_id, local_state::FAILED, attempts, Some(msg))?;
        Ok(Outcome::Failed)
    };
    if rec.owner_id != user_id {
        set_local(inner, file_id, local_state::SKIPPED, 0, Some(NOT_OWNED))?;
        return Ok(Outcome::Skipped);
    }
    let ft = file_type(&rec);
    if !matches!(ft, Some(0..=2)) {
        set_local(
            inner,
            file_id,
            local_state::SKIPPED,
            0,
            Some("unsupported type"),
        )?;
        return Ok(Outcome::Skipped);
    }
    let is_video = ft == Some(1);
    if !is_video && rec.file_size.is_some_and(|s| s > MAX_INDEXABLE_FILE_SIZE) {
        set_local(
            inner,
            file_id,
            local_state::SKIPPED,
            0,
            Some("file too large to index"),
        )?;
        return Ok(Outcome::Skipped);
    }

    // Fresh remote state (another device may have indexed it meanwhile).
    let existing = filedata::fetch(api, mldata::FILE_DATA_TYPE, &[file_id])
        .await?
        .into_iter()
        .find(|fd| fd.file_id == file_id)
        .and_then(|fd| open_remote(&fd, &rec.key));
    if let Some(d) = &existing {
        inner.store().put_ml_data(file_id, d)?;
        if d.plan().is_done() {
            delete_local(inner, file_id)?;
            return Ok(Outcome::Skipped);
        }
    }
    let plan = IndexPlan::for_remote(existing.as_ref());

    // Pixels.
    let input = match ft {
        Some(1) => match inner.thumbnail(file_id).await {
            Ok(b) => (ImageInput::Bytes(b), None),
            Err(e) if filedata::is_transient(&e) => return Err(Stop::Transient(e)),
            Err(e) => return fail(&e.to_string(), false),
        },
        _ => {
            tokio::fs::create_dir_all(tmp_dir)
                .await
                .map_err(CoreError::from)?;
            let path = tmp_dir.join(format!(
                "{file_id}-{}",
                hex::encode(crate::crypto::random_bytes(6))
            ));
            let guard = TempFile(path.clone());
            match inner.download(file_id, &path.to_string_lossy()).await {
                Ok(_) => {}
                Err(e) if filedata::is_transient(&e) => return Err(Stop::Transient(e)),
                Err(e) => return fail(&e.to_string(), false),
            }
            if ft == Some(2) {
                let p = path.clone();
                let image = blocking(move || -> std::result::Result<Vec<u8>, String> {
                    let zip = std::fs::read(&p).map_err(|e| e.to_string())?;
                    livephoto::image_of(&zip)
                })
                .await?;
                drop(guard);
                match image {
                    Ok(b) => (ImageInput::Bytes(b), None),
                    Err(e) => return fail(&e, true),
                }
            } else {
                (ImageInput::Path(path), Some(guard))
            }
        }
    };
    let (input, guard) = input;

    // Inference, one at a time, on a blocking thread.
    let a = analyzer.clone();
    let (run_faces, run_clip) = (plan.run_faces, plan.run_clip);
    let result = blocking(move || {
        let _l = INFERENCE.lock().unwrap_or_else(|e| e.into_inner());
        a.analyze(file_id, input, run_faces, run_clip)
    })
    .await?;
    drop(guard);
    let analysis = match result {
        Ok(a) => a,
        Err(AnalyzeError::Image(m)) => return fail(&m, false),
        Err(AnalyzeError::Runtime(m)) => return Err(Stop::Runtime(m)),
    };

    // The new indexes, exactly as the web worker builds them.
    let face = if plan.run_faces {
        let Some(faces) = analysis.faces else {
            return fail("analysis result is missing the face index", false);
        };
        Some(RemoteFaceIndex::new(
            analysis.width,
            analysis.height,
            faces,
            analysis.flags,
        ))
    } else {
        None
    };
    let clip = if plan.run_clip {
        let Some(embedding) = analysis.clip else {
            return fail("analysis result is missing the CLIP embedding", false);
        };
        Some(RemoteClipIndex::new(embedding, analysis.flags))
    } else {
        None
    };
    let raw = match mldata::merge(existing.as_ref(), face.as_ref(), clip.as_ref()) {
        Ok(r) => r,
        Err(e) => return fail(&e.to_string(), false),
    };
    let base = existing.as_ref().and_then(|d| d.updated_at);
    let local = RemoteMlData::from_json(&serde_json::to_vec(&raw).map_err(CoreError::from)?, base)
        .map_err(|e| CoreError::storage(e.to_string()))?;

    match upload(api, &rec, &raw, base.unwrap_or(0)).await {
        Ok(()) => {
            let mut d = local;
            // The server's new `updatedAt` comes with the next status diff.
            d.updated_at = None;
            inner.store().put_ml_data(file_id, &d)?;
            set_local(inner, file_id, local_state::DONE, 0, None)?;
            Ok(Outcome::Indexed)
        }
        Err(e) if filedata::is_conflict(&e) => {
            // Written elsewhere since our fetch: the next run starts from it.
            Ok(Outcome::Skipped)
        }
        Err(e) if filedata::is_transient(&e) => {
            // Keep the result (search works now) and upload it later.
            inner.store().put_ml_data(file_id, &local)?;
            set_local(inner, file_id, local_state::INDEXED, attempts, None)?;
            Err(Stop::Transient(e))
        }
        Err(e) => fail(&e.to_string(), true),
    }
}
