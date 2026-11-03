//! Persistent progress: what the node has already done, so restarts resume
//! where they stopped.
//!
//! Stored as JSON in `<data_dir>/state.json`, written atomically (temp file,
//! fsync, rename) with mode 0600. It holds only file IDs, timestamps, counts
//! and error messages, never keys or plaintext metadata.
//!
//! Each file's record is tied to the file's `updation_time`: when a file
//! changes on the server (new version, edited metadata) its previous record no
//! longer applies and the file is processed again.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::RetryConfig;

/// Format version of the state file.
pub const STATE_VERSION: u32 = 1;

/// Which kind of result a task produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Face + CLIP indexes (`mldata`).
    Ml,
    /// Streamable video (`vid_preview`).
    Video,
}

impl std::fmt::Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ml => "ml",
            Self::Video => "video",
        })
    }
}

/// How a task was completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// The node produced and uploaded the result.
    Uploaded,
    /// The server already had a current result (from another device).
    AlreadyRemote,
    /// The upload was answered with 409 Conflict: someone else stored it
    /// first. Never re-uploaded.
    Conflict,
    /// The file does not need this result (e.g. video already streamable).
    NotNeeded,
}

/// Versions of the result that was stored, for files that must be redone
/// when an indexer version increases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    /// Face index version, if part of the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<i32>,
    /// CLIP index version, if part of the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<i32>,
}

/// Outcome of a task for one file version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskRecord {
    /// Finished.
    Done {
        /// Unix seconds.
        at: i64,
        /// How.
        how: Completion,
        /// Result versions (ML only).
        #[serde(default, skip_serializing_if = "is_default")]
        versions: Versions,
    },
    /// Failed; retried at `next_retry_at`, or never (until the file changes)
    /// when that is `None`.
    Failed {
        /// Failures so far.
        attempts: u32,
        /// Last error message.
        last_error: String,
        /// Unix seconds of the next attempt, `None` = given up.
        next_retry_at: Option<i64>,
    },
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A file version: its ID plus its `updationTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileKey {
    /// File ID.
    pub id: i64,
    /// `updationTime` in microseconds.
    pub updation_time: i64,
}

impl FileKey {
    /// Creates a key.
    pub const fn new(id: i64, updation_time: i64) -> Self {
        Self { id, updation_time }
    }
}

/// Everything recorded for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// The file's `updationTime` (µs) the records below refer to.
    pub updation_time: i64,
    /// ML result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ml: Option<TaskRecord>,
    /// Video result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<TaskRecord>,
}

impl FileRecord {
    fn slot(&mut self, kind: TaskKind) -> &mut Option<TaskRecord> {
        match kind {
            TaskKind::Ml => &mut self.ml,
            TaskKind::Video => &mut self.video,
        }
    }
}

/// Running totals for `status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counters {
    /// Files indexed and uploaded.
    pub ml_uploaded: u64,
    /// Files whose ML data was already on the server.
    pub ml_already_remote: u64,
    /// ML uploads answered with 409.
    pub ml_conflicts: u64,
    /// Video previews generated and uploaded.
    pub video_uploaded: u64,
    /// Videos whose preview was already on the server.
    pub video_already_remote: u64,
    /// Video preview uploads answered with 409.
    pub video_conflicts: u64,
    /// Videos that need no preview.
    pub video_not_needed: u64,
    /// Failed attempts (all kinds).
    pub failures: u64,
    /// Completed passes over the library.
    pub passes: u64,
    /// Unix seconds when the last pass finished.
    pub last_pass_at: Option<i64>,
}

/// The persisted state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// [`STATE_VERSION`].
    pub version: u32,
    /// Opaque library sync cursor from [`crate::worker::LibrarySource`].
    #[serde(default)]
    pub library_cursor: Option<serde_json::Value>,
    /// Per-file records, by file ID.
    #[serde(default)]
    pub files: BTreeMap<i64, FileRecord>,
    /// Totals.
    #[serde(default)]
    pub counters: Counters,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            library_cursor: None,
            files: BTreeMap::new(),
            counters: Counters::default(),
        }
    }
}

impl State {
    /// The record of `kind` for this file version, if any.
    pub fn task(&self, key: FileKey, kind: TaskKind) -> Option<&TaskRecord> {
        let r = self.files.get(&key.id)?;
        if r.updation_time != key.updation_time {
            return None;
        }
        match kind {
            TaskKind::Ml => r.ml.as_ref(),
            TaskKind::Video => r.video.as_ref(),
        }
    }

    fn record_mut(&mut self, key: FileKey) -> &mut FileRecord {
        let r = self.files.entry(key.id).or_default();
        if r.updation_time != key.updation_time {
            *r = FileRecord {
                updation_time: key.updation_time,
                ..Default::default()
            };
        }
        r
    }

    /// Records a finished task and bumps the matching counter.
    pub fn mark_done(
        &mut self,
        key: FileKey,
        kind: TaskKind,
        how: Completion,
        versions: Versions,
        now: i64,
    ) {
        *self.record_mut(key).slot(kind) = Some(TaskRecord::Done {
            at: now,
            how,
            versions,
        });
        let c = &mut self.counters;
        let counter = match (kind, how) {
            (TaskKind::Ml, Completion::Uploaded) => &mut c.ml_uploaded,
            (TaskKind::Ml, Completion::AlreadyRemote) => &mut c.ml_already_remote,
            (TaskKind::Ml, Completion::Conflict) => &mut c.ml_conflicts,
            (TaskKind::Ml, Completion::NotNeeded) => &mut c.ml_already_remote,
            (TaskKind::Video, Completion::Uploaded) => &mut c.video_uploaded,
            (TaskKind::Video, Completion::AlreadyRemote) => &mut c.video_already_remote,
            (TaskKind::Video, Completion::Conflict) => &mut c.video_conflicts,
            (TaskKind::Video, Completion::NotNeeded) => &mut c.video_not_needed,
        };
        *counter += 1;
    }

    /// Records a failure. `permanent` gives up at once; otherwise the next
    /// attempt is scheduled with exponential backoff until
    /// `retry.max_attempts` is reached. Returns the new record.
    pub fn mark_failed(
        &mut self,
        key: FileKey,
        kind: TaskKind,
        error: &str,
        permanent: bool,
        retry: &RetryConfig,
        now: i64,
    ) -> TaskRecord {
        let slot = self.record_mut(key).slot(kind);
        let attempts = match slot {
            Some(TaskRecord::Failed { attempts, .. }) => *attempts + 1,
            _ => 1,
        };
        let next_retry_at = (!permanent && attempts < retry.max_attempts)
            .then(|| now + retry.delay(attempts).as_secs() as i64);
        let rec = TaskRecord::Failed {
            attempts,
            last_error: truncate(error, 500),
            next_retry_at,
        };
        *slot = Some(rec.clone());
        self.counters.failures += 1;
        rec
    }

    /// Whether `kind` should be attempted for this file version at `now`:
    /// no record, a failure whose retry time has come, or a `Done` whose
    /// versions are below `min_versions` (ML).
    pub fn is_due(&self, key: FileKey, kind: TaskKind, min_versions: Versions, now: i64) -> bool {
        match self.task(key, kind) {
            None => true,
            Some(TaskRecord::Failed { next_retry_at, .. }) => {
                next_retry_at.is_some_and(|t| t <= now)
            }
            Some(TaskRecord::Done { versions, how, .. }) => {
                // Results the node did not create itself were checked
                // against the versions at the time; recheck after upgrades.
                let below = |have: Option<i32>, want: Option<i32>| match want {
                    Some(w) => have.is_none_or(|h| h < w),
                    None => false,
                };
                *how != Completion::NotNeeded
                    && (below(versions.face, min_versions.face)
                        || below(versions.clip, min_versions.clip))
            }
        }
    }

    /// Drops records of files that are no longer in the library.
    pub fn retain_files(&mut self, live: &std::collections::HashSet<i64>) {
        self.files.retain(|id, _| live.contains(id));
    }

    /// Counts records by outcome: `(done, failed_retrying, given_up)` for `kind`.
    pub fn tally(&self, kind: TaskKind) -> (u64, u64, u64) {
        let mut t = (0, 0, 0);
        for r in self.files.values() {
            match match kind {
                TaskKind::Ml => &r.ml,
                TaskKind::Video => &r.video,
            } {
                Some(TaskRecord::Done { .. }) => t.0 += 1,
                Some(TaskRecord::Failed {
                    next_retry_at: Some(_),
                    ..
                }) => t.1 += 1,
                Some(TaskRecord::Failed { .. }) => t.2 += 1,
                None => {}
            }
        }
        t
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// [`State`] bound to its file, with change tracking.
#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    /// The in-memory state.
    pub state: State,
    dirty: bool,
}

impl StateStore {
    /// Loads `path`, or starts empty if it does not exist.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let state = match std::fs::read(&path) {
            Ok(bytes) => {
                let s: State = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing state {}", path.display()))?;
                if s.version > STATE_VERSION {
                    anyhow::bail!(
                        "state {} has version {}, newer than this node supports ({STATE_VERSION})",
                        path.display(),
                        s.version
                    );
                }
                s
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(e).with_context(|| format!("reading state {}", path.display())),
        };
        Ok(Self {
            path,
            state,
            dirty: false,
        })
    }

    /// The file this store writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Mutable access; marks the state as changed.
    pub fn edit(&mut self) -> &mut State {
        self.dirty = true;
        &mut self.state
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Writes the state if it changed since the last save.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&self.state)?;
        write_atomic(&self.path, &bytes)?;
        self.dirty = false;
        Ok(())
    }
}

/// Writes `bytes` to `path` atomically with mode 0600: a temporary file in
/// the same directory is written, flushed to disk and renamed over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    let res = (|| -> Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.with_context(|| format!("writing {}", path.display()))
}

/// Current Unix time in seconds.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: Versions = Versions {
        face: Some(1),
        clip: Some(1),
    };

    #[test]
    fn done_is_tied_to_updation_time() {
        let mut s = State::default();
        s.mark_done(
            FileKey::new(7, 100),
            TaskKind::Ml,
            Completion::Uploaded,
            V1,
            0,
        );
        assert!(!s.is_due(FileKey::new(7, 100), TaskKind::Ml, V1, 0));
        assert!(s.is_due(FileKey::new(7, 100), TaskKind::Video, V1, 0));
        // File changed: everything is due again.
        assert!(s.is_due(FileKey::new(7, 101), TaskKind::Ml, V1, 0));
        s.mark_done(
            FileKey::new(7, 101),
            TaskKind::Video,
            Completion::Uploaded,
            Versions::default(),
            0,
        );
        assert_eq!(s.files[&7].ml, None);
        assert_eq!(s.counters.ml_uploaded, 1);
        assert_eq!(s.counters.video_uploaded, 1);
    }

    #[test]
    fn version_bump_redoes_ml() {
        let mut s = State::default();
        s.mark_done(
            FileKey::new(1, 1),
            TaskKind::Ml,
            Completion::Uploaded,
            V1,
            0,
        );
        let v2 = Versions {
            face: Some(2),
            clip: Some(1),
        };
        assert!(s.is_due(FileKey::new(1, 1), TaskKind::Ml, v2, 0));
        // Only CLIP enabled: face version irrelevant.
        let clip_only = Versions {
            face: None,
            clip: Some(1),
        };
        assert!(!s.is_due(FileKey::new(1, 1), TaskKind::Ml, clip_only, 0));
    }

    #[test]
    fn failures_back_off_and_give_up() {
        let retry = RetryConfig {
            max_attempts: 3,
            base_delay_secs: 10,
            max_delay_secs: 1000,
        };
        let mut s = State::default();
        let r = s.mark_failed(
            FileKey::new(1, 1),
            TaskKind::Ml,
            "boom",
            false,
            &retry,
            1000,
        );
        assert_eq!(
            r,
            TaskRecord::Failed {
                attempts: 1,
                last_error: "boom".into(),
                next_retry_at: Some(1010)
            }
        );
        assert!(!s.is_due(FileKey::new(1, 1), TaskKind::Ml, V1, 1009));
        assert!(s.is_due(FileKey::new(1, 1), TaskKind::Ml, V1, 1010));
        let r = s.mark_failed(
            FileKey::new(1, 1),
            TaskKind::Ml,
            "boom",
            false,
            &retry,
            1010,
        );
        assert!(matches!(
            r,
            TaskRecord::Failed {
                attempts: 2,
                next_retry_at: Some(1030),
                ..
            }
        ));
        let r = s.mark_failed(
            FileKey::new(1, 1),
            TaskKind::Ml,
            "boom",
            false,
            &retry,
            1030,
        );
        assert!(matches!(
            r,
            TaskRecord::Failed {
                attempts: 3,
                next_retry_at: None,
                ..
            }
        ));
        assert!(!s.is_due(FileKey::new(1, 1), TaskKind::Ml, V1, i64::MAX));
        assert_eq!(s.tally(TaskKind::Ml), (0, 0, 1));
        // A new version of the file starts over.
        assert!(s.is_due(FileKey::new(1, 2), TaskKind::Ml, V1, 0));
        // Permanent errors give up at once.
        let r = s.mark_failed(FileKey::new(2, 1), TaskKind::Video, "bad", true, &retry, 0);
        assert!(matches!(
            r,
            TaskRecord::Failed {
                attempts: 1,
                next_retry_at: None,
                ..
            }
        ));
        assert_eq!(s.counters.failures, 4);
    }

    #[test]
    fn long_errors_truncated() {
        let mut s = State::default();
        let e = "é".repeat(600);
        let r = s.mark_failed(
            FileKey::new(1, 1),
            TaskKind::Ml,
            &e,
            false,
            &RetryConfig::default(),
            0,
        );
        let TaskRecord::Failed { last_error, .. } = r else {
            panic!()
        };
        assert!(last_error.len() <= 504);
    }

    #[test]
    fn persistence_roundtrip_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/state.json");
        let mut st = StateStore::load(&path).unwrap();
        assert_eq!(st.state, State::default());
        st.save().unwrap();
        assert!(!path.exists(), "clean state is not written");

        st.edit().library_cursor = Some(serde_json::json!({"sinceTime": 42}));
        st.edit().mark_done(
            FileKey::new(5, 9),
            TaskKind::Ml,
            Completion::Conflict,
            V1,
            3,
        );
        st.edit().mark_failed(
            FileKey::new(6, 9),
            TaskKind::Video,
            "x",
            false,
            &RetryConfig::default(),
            3,
        );
        st.save().unwrap();
        assert!(!st.is_dirty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());

        let again = StateStore::load(&path).unwrap();
        assert_eq!(again.state, st.state);
        assert_eq!(again.state.counters.ml_conflicts, 1);
    }

    #[test]
    fn retain_and_newer_version() {
        let mut s = State::default();
        s.mark_done(
            FileKey::new(1, 1),
            TaskKind::Ml,
            Completion::Uploaded,
            V1,
            0,
        );
        s.mark_done(
            FileKey::new(2, 1),
            TaskKind::Ml,
            Completion::Uploaded,
            V1,
            0,
        );
        s.retain_files(&[2].into_iter().collect());
        assert_eq!(s.files.keys().copied().collect::<Vec<_>>(), vec![2]);

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        std::fs::write(&p, r#"{"version": 99}"#).unwrap();
        assert!(StateStore::load(&p).is_err());
        std::fs::write(&p, "not json").unwrap();
        assert!(StateStore::load(&p).is_err());
    }
}
