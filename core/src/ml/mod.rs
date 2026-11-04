//! Search, faces and memories (contract v1.2, `contracts/photovault-core.md`).
//!
//! Everything here works on Ente's own data, so results are shared with
//! the stock Ente apps and the compute node in both directions:
//!
//! - [`mldata`]: Ente's per-file `mldata` (face + CLIP indexes), gzipped and
//!   encrypted with the file key, stored through `/files/data`.
//! - [`index`]: pulls `mldata` other devices computed, downloads the models
//!   from the server's mirror (`/photovault/models/<file>`), indexes the
//!   rest on device with Ente's Rust ML crate (`ente-ml` on ONNX Runtime,
//!   cargo feature `ml`) and uploads the results.
//! - [`entity`]: Ente's encrypted user entities (`/user-entity/...`):
//!   people (`cgroup`) and location tags (`location`).
//! - [`faces`]: face clustering exactly like Ente's, people (named
//!   clusters as `cgroup` entities), face crops.
//! - [`search`]: magic search (CLIP text vs image embeddings) and the
//!   metadata search (names, captions, albums, dates, people, types,
//!   places).
//! - [`memories`]: Ente's smart memories (on this day, people, trips, …).
//! - [`store`]: the sealed local tables.
//!
//! Each module owns its own `#[uniffi::export] impl Core` block.
//!
//! Based on Ente (ente.com).

pub mod entity;
pub mod faces;
pub(crate) mod filedata;
pub mod index;
pub mod memories;
pub mod mldata;
pub(crate) mod models;
pub mod search;
pub(crate) mod store;

use crate::TimelineItem;
use std::path::PathBuf;

/// In-memory ML state of a [`crate::client::Inner`]: one cache per module,
/// each rebuilt by its owner when the data under it changes.
#[derive(Debug)]
pub(crate) struct MlState {
    /// Where the downloaded models live (`<data_dir>/models`).
    pub models_dir: PathBuf,
    /// Indexer state (owned by [`index`]).
    pub index: index::IndexerState,
    /// Search caches (owned by [`search`]).
    pub search: search::SearchCache,
    /// Clusters and people (owned by [`faces`]).
    pub faces: faces::FacesCache,
    /// Memories of the day (owned by [`memories`]).
    pub memories: memories::MemoriesCache,
}

impl MlState {
    pub(crate) fn new(data_dir: &std::path::Path) -> Self {
        Self {
            models_dir: data_dir.join("models"),
            index: index::IndexerState::default(),
            search: search::SearchCache::default(),
            faces: faces::FacesCache::default(),
            memories: memories::MemoriesCache::default(),
        }
    }
}

// ---------------------------------------------------------------- records

/// State of on-device ML.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct MlStatus {
    /// ML (faces + magic search) is enabled for this account on this device.
    pub enabled: bool,
    /// This build can run the models (cargo feature `ml` and ONNX Runtime
    /// loads). Without it, results from other devices still work.
    pub inference_available: bool,
    /// All models are downloaded and verified.
    pub models_ready: bool,
    /// Model bytes downloaded so far / in total.
    pub model_bytes_done: u64,
    /// Total model bytes.
    pub model_bytes_total: u64,
    /// Indexable files in the library (images, videos, live photos).
    pub files_total: u64,
    /// Files with current face and CLIP indexes, from any device.
    pub files_indexed: u64,
    /// Files still to index on this device.
    pub files_pending: u64,
    /// Indexing is running now.
    pub running: bool,
    /// The last indexing error, if the last run stopped on one.
    pub last_error: Option<String>,
}

/// Receives ML progress (implemented in Kotlin).
#[uniffi::export(with_foreign)]
pub trait MlListener: Send + Sync {
    /// Called as models download, files are indexed and data is pulled.
    fn on_status(&self, status: MlStatus);
}

/// What a search suggestion stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SearchKind {
    /// Magic search: CLIP text query (key = the query text).
    Magic,
    /// A person or unnamed face cluster (key = person id).
    Person,
    /// An album (key = collection id).
    Album,
    /// A date: a day, month or year, or a day of year in any year
    /// (key: `y:2024`, `ym:2024-05`, `ymd:2024-05-17`, `md:05-17`, `m:05`).
    Date,
    /// A location tag or city (key = location tag entity id or `city:<name>`).
    Place,
    /// File name match (key = the query text).
    FileName,
    /// Caption / description match (key = the query text).
    Caption,
    /// File type (key: `image`, `video`, `live`, `screenshot`, `raw`, …).
    FileType,
}

/// One suggestion row of the search screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchSuggestion {
    /// Kind.
    pub kind: SearchKind,
    /// Opaque key to pass back to [`crate::Core::search_results`].
    pub key: String,
    /// Display label ("Ana", "Summer 2024", "IMG_0042.jpg", …).
    pub label: String,
    /// Matching files (0 when unknown before running, e.g. magic search).
    pub count: u64,
    /// File for the row's thumbnail.
    pub cover_file_id: Option<i64>,
    /// Face for a person's round avatar (see [`crate::Core::face_thumbnail`]).
    pub cover_face_id: Option<String>,
}

/// A face on a file. Coordinates are normalised to `[0, 1]`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FaceRef {
    /// Ente face id (`<fileID>_<xMin>_<yMin>_<xMax>_<yMax>`).
    pub face_id: String,
    /// File it is on.
    pub file_id: i64,
    /// Box left.
    pub x: f32,
    /// Box top.
    pub y: f32,
    /// Box width.
    pub width: f32,
    /// Box height.
    pub height: f32,
    /// Detection score.
    pub score: f32,
    /// Blur variance (higher is sharper).
    pub blur: f32,
}

/// A person: a named `cgroup` (shared with the Ente apps), or an unnamed
/// local face cluster.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Person {
    /// `cgroup` entity id, or `cluster:<clusterID>` for an unnamed cluster.
    pub id: String,
    /// Name; `None` for an unnamed cluster.
    pub name: Option<String>,
    /// Hidden by the user ("ignored" in Ente).
    pub is_hidden: bool,
    /// Files with this person.
    pub file_count: u64,
    /// Avatar face (the user's choice, else the best face).
    pub cover: Option<FaceRef>,
    /// Birth date, `YYYY-MM-DD`, if set.
    pub birth_date: Option<String>,
}

/// A face on a file and who it is.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FileFace {
    /// The face.
    pub face: FaceRef,
    /// Person or cluster id, if clustered.
    pub person_id: Option<String>,
    /// Person name, if named.
    pub person_name: Option<String>,
}

/// Kind of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MemoryKind {
    /// This day in earlier years.
    OnThisDay,
    /// A person (also "spotlight", "you and them", birthdays).
    People,
    /// A trip away from home.
    Trip,
    /// This week / month in earlier years.
    Time,
    /// A theme found with CLIP (sunsets, food, …).
    Clip,
    /// Filler: a random throwback.
    Filler,
}

/// One memory: a short story of photos.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Memory {
    /// Stable id (for "seen" state and list keys).
    pub id: String,
    /// Kind.
    pub kind: MemoryKind,
    /// Title ("2 years ago", "Trip to Lisbon", "Ana", …).
    pub title: String,
    /// Subtitle, if any.
    pub subtitle: Option<String>,
    /// The photos, in story order.
    pub items: Vec<TimelineItem>,
    /// Cover.
    pub cover_file_id: i64,
    /// Already seen by the user.
    pub seen: bool,
    /// The person, for [`MemoryKind::People`].
    pub person_id: Option<String>,
}
