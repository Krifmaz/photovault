//! Local media handling: type detection, content hashes, thumbnails, EXIF,
//! and Live Photo bundles in Ente's format.
//!
//! STUB: signatures are fixed; bodies are filled in by the media work item.

use crate::item::{GeoPoint, MediaFile, MediaKind};
use std::path::Path;

/// Longest edge of a generated thumbnail, matching the Ente apps.
pub const THUMBNAIL_MAX_DIMENSION: u32 = 720;

/// What EXIF/container metadata could tell us about a file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddedInfo {
    /// Capture time (DateTimeOriginal, with offset if present), µs since epoch.
    pub creation_time_us: Option<i64>,
    /// GPS position.
    pub location: Option<GeoPoint>,
    /// Pixel dimensions.
    pub dimensions: Option<(u32, u32)>,
}

/// Image or video, from the file extension (and magic bytes if ambiguous).
/// Returns `None` for files that are not media (e.g. `.csv`, `.json`).
pub fn detect_kind(path: &Path) -> Option<MediaKind> {
    todo!("{path:?}")
}

/// Ente's content hash of a file: base64 of the 64-byte BLAKE2b digest,
/// computed in [`crate::crypto::ENCRYPTION_CHUNK_SIZE`] chunks.
pub fn hash_file(path: &Path) -> anyhow::Result<String> {
    todo!("{path:?}")
}

/// Read EXIF (JPEG, HEIC, PNG, TIFF) or, for videos, the container creation
/// time and location via `ffprobe` if available. Never fails: unknown is `Default`.
pub fn read_embedded_info(path: &Path) -> EmbeddedInfo {
    todo!("{path:?}")
}

/// Make a JPEG thumbnail (longest edge [`THUMBNAIL_MAX_DIMENSION`]) for an
/// item. Tries, in order: `source_thumbnail`, decoding `media` with the
/// `image` crate, then `ffmpeg` (HEIC, RAW, video frames). Falls back to a
/// small neutral placeholder so an upload never fails just for a thumbnail.
pub async fn make_thumbnail(
    media: &MediaFile,
    kind: MediaKind,
    source_thumbnail: Option<&Path>,
) -> Vec<u8> {
    todo!("{media:?}{kind:?}{source_thumbnail:?}")
}

/// Bundle a Live Photo the way the Ente apps do (a zip holding the still and
/// the video). The returned temp file is deleted when dropped.
pub fn live_photo_bundle(image: &MediaFile, video: &MediaFile) -> anyhow::Result<tempfile::NamedTempFile> {
    todo!("{image:?}{video:?}")
}
