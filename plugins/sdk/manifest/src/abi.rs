//! The host/guest ABI, version [`crate::API_VERSION`].
//!
//! # Exports (guest → host)
//!
//! A plugin is an Extism module. It exports up to three functions, matching
//! the `[hooks]` table of its manifest. Each takes a JSON [`HookInput`] and
//! returns a JSON [`HookOutput`]:
//!
//! | Export          | Manifest key       | `HookInput::event`        |
//! |-----------------|--------------------|---------------------------|
//! | `on_upload`     | `on_upload = true` | [`HookEvent::Upload`]     |
//! | `on_schedule`   | `schedule = {..}`  | [`HookEvent::Schedule`]   |
//! | `on_demand`     | `on_demand = [..]` | [`HookEvent::OnDemand`]   |
//!
//! # Imports (host → guest)
//!
//! The host provides two functions in Extism's `extism:host/user` namespace:
//!
//! * `photovault_call(request: JSON) -> JSON`: a [`Request`] in, a
//!   [`Response`] out. Every library operation goes through this one function so
//!   the permission check lives in one place.
//! * `photovault_read(request: JSON) -> bytes`: a [`ReadRequest`] in, the raw
//!   decrypted bytes out (thumbnails and originals; JSON-wrapping them would
//!   waste memory). On failure the guest gets an Extism error whose message is
//!   a JSON [`HostError`].
//!
//! Outbound HTTP uses Extism's built-in `http_request`, restricted by the host
//! to the granted `network` hosts. There is no file system access.

use crate::settings::Values;
use serde::{Deserialize, Serialize};

/// Name of the JSON call host function.
pub const HOST_CALL: &str = "photovault_call";
/// Name of the binary read host function.
pub const HOST_READ: &str = "photovault_read";
/// Name of the upload hook export.
pub const EXPORT_ON_UPLOAD: &str = "on_upload";
/// Name of the schedule hook export.
pub const EXPORT_ON_SCHEDULE: &str = "on_schedule";
/// Name of the on-demand hook export.
pub const EXPORT_ON_DEMAND: &str = "on_demand";

/// Ente file id.
pub type FileId = i64;
/// Ente collection (album) id.
pub type AlbumId = i64;

// ---------------------------------------------------------------------------
// Library types
// ---------------------------------------------------------------------------

/// Kind of file, as in Ente's metadata `fileType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    /// Still image.
    Image,
    /// Video.
    Video,
    /// Live Photo (image plus short video).
    LivePhoto,
    /// Anything else.
    Other,
}

/// Decrypted file metadata, as the client already holds it. Requires
/// `read:metadata`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    /// File id.
    pub id: FileId,
    /// Albums the file is in.
    pub album_ids: Vec<AlbumId>,
    /// File name, e.g. `IMG_0001.HEIC`.
    pub title: String,
    /// File type.
    pub file_type: FileType,
    /// Capture time, microseconds since the Unix epoch (Ente's convention).
    pub creation_time: i64,
    /// Last modification time, microseconds since the Unix epoch.
    pub modification_time: i64,
    /// Latitude in degrees, if known.
    #[serde(default)]
    pub latitude: Option<f64>,
    /// Longitude in degrees, if known.
    #[serde(default)]
    pub longitude: Option<f64>,
    /// Pixel width, if known.
    #[serde(default)]
    pub width: Option<u32>,
    /// Pixel height, if known.
    #[serde(default)]
    pub height: Option<u32>,
    /// Size of the original in bytes, if known.
    #[serde(default)]
    pub size: Option<u64>,
    /// Device folder the file was backed up from (e.g. `Screenshots`).
    #[serde(default)]
    pub device_folder: Option<String>,
    /// Caption / description.
    #[serde(default)]
    pub caption: Option<String>,
    /// Tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// In the user's favourites.
    #[serde(default)]
    pub is_favorite: bool,
    /// Archived.
    #[serde(default)]
    pub is_archived: bool,
}

/// What kind of collection an album is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlbumKind {
    /// Ordinary album.
    Album,
    /// Device folder album.
    Folder,
    /// Favourites.
    Favorites,
    /// Uncategorised.
    Uncategorized,
}

/// Album metadata. Requires `read:metadata`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumInfo {
    /// Album id.
    pub id: AlbumId,
    /// Decrypted name.
    pub name: String,
    /// Kind.
    pub kind: AlbumKind,
    /// Owned by the user (plugins may only change owned albums).
    pub is_owned: bool,
    /// Number of files, if known.
    #[serde(default)]
    pub file_count: Option<u64>,
}

/// Filter for [`Request::ListFiles`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileQuery {
    /// Only files in this album.
    #[serde(default)]
    pub album_id: Option<AlbumId>,
    /// Only files created at or after this time (µs).
    #[serde(default)]
    pub created_after: Option<i64>,
    /// Only files created before this time (µs).
    #[serde(default)]
    pub created_before: Option<i64>,
    /// Only files of this type.
    #[serde(default)]
    pub file_type: Option<FileType>,
    /// Page size; the host caps it (default and maximum 500).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Cursor from the previous [`FilePage::next_cursor`].
    #[serde(default)]
    pub cursor: Option<String>,
}

/// One page of [`Request::ListFiles`] results.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FilePage {
    /// Files on this page.
    pub files: Vec<FileInfo>,
    /// Pass back to get the next page; `None` on the last page.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Host calls
// ---------------------------------------------------------------------------

/// A request to `photovault_call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    /// `read:metadata` → [`FilePage`].
    ListFiles(FileQuery),
    /// `read:metadata` → `Option<FileInfo>`.
    GetFile {
        /// File id.
        file_id: FileId,
    },
    /// `read:metadata` → `Vec<AlbumInfo>`.
    ListAlbums,
    /// `write:tags` → `()`. Adds tags; existing tags are kept.
    AddTags {
        /// File id.
        file_id: FileId,
        /// Tags to add.
        tags: Vec<String>,
    },
    /// `write:albums` → [`AlbumInfo`]. Returns the user's owned album with this
    /// exact name if one exists, otherwise creates it.
    EnsureAlbum {
        /// Album name.
        name: String,
    },
    /// `write:albums` → `()`. Album must be owned.
    AddToAlbum {
        /// Album id.
        album_id: AlbumId,
        /// Files to add.
        file_ids: Vec<FileId>,
    },
    /// `write:albums` → `()`. Album must be owned.
    RemoveFromAlbum {
        /// Album id.
        album_id: AlbumId,
        /// Files to remove.
        file_ids: Vec<FileId>,
    },
    /// `write:trash` → `()`. Files go to the trash, recoverable as in Ente.
    TrashFiles {
        /// Files to trash.
        file_ids: Vec<FileId>,
    },
    /// `write:export` (and `read:originals` is **not** needed: the host copies
    /// the file without handing bytes to the plugin) → [`ExportResult`].
    ExportFile {
        /// File id.
        file_id: FileId,
        /// Path inside the export folder, `/`-separated, relative, no `..`.
        path: String,
        /// Overwrite an existing file at `path`.
        #[serde(default)]
        overwrite: bool,
    },
    /// Always allowed → `Option<serde_json::Value>`. Per-plugin key/value
    /// storage kept by the host on the device (e.g. a "last run" cursor).
    KvGet {
        /// Key, at most 128 bytes.
        key: String,
    },
    /// Always allowed → `()`. `value: null` deletes. The host caps the total
    /// size per plugin.
    KvSet {
        /// Key, at most 128 bytes.
        key: String,
        /// Value.
        value: serde_json::Value,
    },
    /// Always allowed → `()`. Reports progress for long on-demand runs.
    Progress {
        /// Items done.
        done: u64,
        /// Items in total, if known.
        #[serde(default)]
        total: Option<u64>,
        /// Short status text.
        #[serde(default)]
        message: Option<String>,
    },
}

impl Request {
    /// The permission this request needs, or `None` if always allowed.
    pub fn required_permission(&self) -> Option<crate::Permission> {
        use crate::{Permission::*, ReadScope as R, WriteScope as W};
        Some(match self {
            Request::ListFiles(_) | Request::GetFile { .. } | Request::ListAlbums => {
                Read(R::Metadata)
            }
            Request::AddTags { .. } => Write(W::Tags),
            Request::EnsureAlbum { .. }
            | Request::AddToAlbum { .. }
            | Request::RemoveFromAlbum { .. } => Write(W::Albums),
            Request::TrashFiles { .. } => Write(W::Trash),
            Request::ExportFile { .. } => Write(W::Export),
            Request::KvGet { .. } | Request::KvSet { .. } | Request::Progress { .. } => {
                return None;
            }
        })
    }
}

/// Result of [`Request::ExportFile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    /// Whether a file was written (`false` if it existed and `overwrite` was
    /// not set).
    pub written: bool,
    /// Bytes written.
    pub bytes: u64,
}

/// Which bytes `photovault_read` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadKind {
    /// Decrypted thumbnail (`read:thumbnails`).
    Thumbnail,
    /// Decrypted original (`read:originals`).
    Original,
}

/// A request to `photovault_read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    /// File id.
    pub file_id: FileId,
    /// Thumbnail or original.
    pub kind: ReadKind,
}

/// Machine-readable error kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The user has not granted (or has revoked) the needed permission.
    PermissionDenied,
    /// The file or album does not exist or is not visible.
    NotFound,
    /// The request is malformed or a value is not allowed.
    InvalidRequest,
    /// A host limit was hit (page size, KV size, read size).
    LimitExceeded,
    /// Something failed on the host.
    Internal,
}

/// An error returned by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct HostError {
    /// Kind.
    pub code: ErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl HostError {
    /// Construct an error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// The reply from `photovault_call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// Success; the value's shape depends on the request (see [`Request`]).
    Ok(serde_json::Value),
    /// Failure.
    Err(HostError),
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/// What triggered a hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookEvent {
    /// A file was uploaded. `file` is present only with `read:metadata`.
    Upload {
        /// File id.
        file_id: FileId,
        /// Metadata, if the plugin may read it.
        #[serde(default)]
        file: Option<FileInfo>,
    },
    /// The schedule fired.
    Schedule {
        /// When the hook last ran successfully (µs since epoch), if ever.
        #[serde(default)]
        last_run: Option<i64>,
    },
    /// The user pressed an on-demand button.
    OnDemand {
        /// The [`crate::OnDemandAction::id`].
        action: String,
    },
}

/// The JSON input of every hook export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookInput {
    /// What happened.
    pub event: HookEvent,
    /// Current settings, complete and valid (defaults filled in).
    pub settings: Values,
    /// Current time, µs since epoch, from the host.
    pub now: i64,
    /// Where the plugin runs.
    pub runner: Runner,
}

/// Where a plugin runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runner {
    /// Phone app.
    Mobile,
    /// Desktop app.
    Desktop,
    /// Web app.
    Web,
    /// Compute node.
    Node,
    /// Test harness.
    Test,
}

/// The JSON output of every hook export.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HookOutput {
    /// One line shown in the plugin's activity log, e.g. "Added 12 photos to 3
    /// albums".
    #[serde(default)]
    pub summary: Option<String>,
    /// Number of files the hook changed, for the activity log.
    #[serde(default)]
    pub changed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_wire_format() {
        let r = Request::AddToAlbum {
            album_id: 7,
            file_ids: vec![1, 2],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({"method": "add_to_album", "params": {"album_id": 7, "file_ids": [1, 2]}})
        );
        assert_eq!(serde_json::from_value::<Request>(v).unwrap(), r);
        let v = serde_json::to_value(Request::ListAlbums).unwrap();
        assert_eq!(v, json!({"method": "list_albums"}));
    }

    #[test]
    fn response_wire_format() {
        let ok = Response::Ok(json!(1));
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"ok": 1}));
        let err = Response::Err(HostError::new(ErrorCode::PermissionDenied, "no"));
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({"err": {"code": "permission_denied", "message": "no"}})
        );
    }

    #[test]
    fn hook_input_wire_format() {
        let i = HookInput {
            event: HookEvent::OnDemand {
                action: "run".into(),
            },
            settings: Values::new(),
            now: 5,
            runner: Runner::Test,
        };
        assert_eq!(
            serde_json::to_value(&i).unwrap(),
            json!({"event": {"type": "on_demand", "action": "run"}, "settings": {}, "now": 5, "runner": "test"})
        );
    }

    #[test]
    fn permissions_per_request() {
        assert!(
            Request::KvGet { key: "a".into() }
                .required_permission()
                .is_none()
        );
        assert_eq!(
            Request::TrashFiles { file_ids: vec![] }
                .required_permission()
                .unwrap()
                .to_string(),
            "write:trash"
        );
    }
}
