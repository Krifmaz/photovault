//! The source-neutral description of one thing to import.
//!
//! Every source (Immich, iCloud) turns its own records into [`ImportItem`]s.
//! The uploader in [`crate::ente`] only ever sees this type.

use std::path::PathBuf;

/// What kind of Ente file an item becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MediaKind {
    /// A still image (Ente `fileType` 0).
    Image,
    /// A video (Ente `fileType` 1).
    Video,
    /// A Live Photo: still image plus a short video (Ente `fileType` 2).
    LivePhoto,
}

/// Where the bytes of a media component are. Sources download remote
/// originals to a temporary file first, so the uploader always reads a path.
#[derive(Debug, Clone)]
pub struct MediaFile {
    /// Path to the bytes on local disk.
    pub path: PathBuf,
    /// Original file name as the user knew it (e.g. `IMG_0001.HEIC`).
    pub file_name: String,
}

/// One photo, video or Live Photo to import, with everything we know about it.
#[derive(Debug, Clone)]
pub struct ImportItem {
    /// Stable id in the source (Immich asset id, iCloud file name/checksum).
    /// Used by the resume journal.
    pub source_id: String,
    /// What it becomes in Ente.
    pub kind: MediaKind,
    /// The main file: the image, the video, or the still of a Live Photo.
    pub primary: MediaFile,
    /// The motion part of a Live Photo. `Some` only for [`MediaKind::LivePhoto`].
    pub live_video: Option<MediaFile>,
    /// Optional ready-made thumbnail (JPEG/WebP/PNG) from the source, e.g.
    /// Immich's preview. If `None`, the uploader generates one locally.
    pub thumbnail: Option<PathBuf>,
    /// Capture time, microseconds since the Unix epoch (UTC).
    pub creation_time_us: Option<i64>,
    /// Last modification time, microseconds since the Unix epoch (UTC).
    pub modification_time_us: Option<i64>,
    /// GPS position.
    pub location: Option<GeoPoint>,
    /// Favourite in the source.
    pub favorite: bool,
    /// Archived (hidden from the timeline but not deleted) in the source.
    pub archived: bool,
    /// Hidden in the source (iCloud "Hidden"); goes to Ente's hidden album.
    pub hidden: bool,
    /// Free-text description / caption.
    pub description: Option<String>,
    /// Names of the source albums this item belongs to.
    pub albums: Vec<String>,
    /// Names of the people recognised in this item (names only; no face data).
    pub people: Vec<String>,
    /// Pixel width and height if known.
    pub dimensions: Option<(u32, u32)>,
}

/// A WGS84 position.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeoPoint {
    /// Latitude in degrees.
    pub latitude: f64,
    /// Longitude in degrees.
    pub longitude: f64,
}

impl ImportItem {
    /// A minimal item with only the required fields set.
    pub fn new(source_id: impl Into<String>, kind: MediaKind, primary: MediaFile) -> Self {
        Self {
            source_id: source_id.into(),
            kind,
            primary,
            live_video: None,
            thumbnail: None,
            creation_time_us: None,
            modification_time_us: None,
            location: None,
            favorite: false,
            archived: false,
            hidden: false,
            description: None,
            albums: Vec::new(),
            people: Vec::new(),
            dimensions: None,
        }
    }
}

/// A source of [`ImportItem`]s.
///
/// Sources are pulled one item at a time so a large library never has to be
/// held in memory or on disk at once. Temporary files an item refers to are
/// owned by the source and may be removed after [`Source::done`] is called.
#[async_trait::async_trait]
pub trait Source: Send {
    /// Short name for logs and the journal ("immich", "icloud").
    fn name(&self) -> &'static str;

    /// Total number of items if known up front (for progress output).
    async fn total(&mut self) -> anyhow::Result<Option<u64>>;

    /// Next item, or `None` when the source is exhausted. `skip` is consulted
    /// before any download so already-imported items cost nothing.
    async fn next(
        &mut self,
        skip: &(dyn Fn(&str) -> bool + Sync),
    ) -> anyhow::Result<Option<ImportItem>>;

    /// Called after an item was uploaded (or failed) so the source can delete
    /// temporary files belonging to it.
    async fn done(&mut self, item: &ImportItem) -> anyhow::Result<()>;
}
