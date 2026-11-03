//! Typed wrappers for every host call.
//!
//! Each function builds one [`Request`], sends it through `photovault_call`
//! (or `photovault_read`) and decodes the reply. The permission each call needs
//! is listed on the function; without it the host answers
//! [`ErrorCode::PermissionDenied`]. Errors come back as [`HostError`], which
//! converts into [`crate::Error`] with `?`.

use crate::abi::{
    AlbumId, AlbumInfo, ErrorCode, ExportResult, FileId, FileInfo, FilePage, FileQuery, HostError,
    ReadKind, ReadRequest, Request, Response,
};
use crate::backend;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Send a raw [`Request`] and decode the `ok` value as `T`.
///
/// The typed functions below are thin wrappers over this; use it directly
/// only for requests newer than this PDK.
pub fn call<T: DeserializeOwned>(request: &Request) -> Result<T, HostError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|e| HostError::new(ErrorCode::InvalidRequest, e.to_string()))?;
    let reply = backend::call(&bytes)?;
    let response: Response = serde_json::from_slice(&reply)
        .map_err(|e| backend::internal(format!("bad response from host: {e}")))?;
    match response {
        Response::Ok(value) => serde_json::from_value(value)
            .map_err(|e| backend::internal(format!("unexpected response from host: {e}"))),
        Response::Err(e) => Err(e),
    }
}

/// Like [`call`] for requests whose result is `()`. Accepts any `ok` value so
/// a host that returns `null` or `{}` both work.
fn call_unit(request: &Request) -> Result<(), HostError> {
    call::<serde_json::Value>(request).map(drop)
}

// ---------------------------------------------------------------------------
// Reading metadata (`read:metadata`)
// ---------------------------------------------------------------------------

/// One page of files matching `query`. Needs `read:metadata`.
///
/// Most plugins want [`files`], which walks every page.
pub fn list_files(query: &FileQuery) -> Result<FilePage, HostError> {
    call(&Request::ListFiles(query.clone()))
}

/// Every file matching `query`, fetching pages lazily. Needs `read:metadata`.
///
/// `query.cursor` is the starting point (normally `None`). The iterator yields
/// one `Err` and then stops if a page fails.
///
/// ```no_run
/// # use photovault_plugin_pdk as pdk;
/// # fn f() -> Result<(), pdk::Error> {
/// for file in pdk::host::files(Default::default()) {
///     let file = file?;
///     pdk::info!("{}", file.title);
/// }
/// # Ok(()) }
/// ```
pub fn files(query: FileQuery) -> Files {
    Files {
        query,
        buffer: Vec::new().into_iter(),
        done: false,
    }
}

/// Iterator returned by [`files`].
#[derive(Debug)]
pub struct Files {
    query: FileQuery,
    buffer: std::vec::IntoIter<FileInfo>,
    done: bool,
}

impl Iterator for Files {
    type Item = Result<FileInfo, HostError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(file) = self.buffer.next() {
                return Some(Ok(file));
            }
            if self.done {
                return None;
            }
            match list_files(&self.query) {
                Ok(page) => {
                    self.buffer = page.files.into_iter();
                    match page.next_cursor {
                        // A host that repeats a cursor would loop forever.
                        Some(c) if self.query.cursor.as_ref() != Some(&c) => {
                            self.query.cursor = Some(c)
                        }
                        _ => self.done = true,
                    }
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// One file's metadata, or `None` if it does not exist (or is trashed).
/// Needs `read:metadata`.
pub fn get_file(file_id: FileId) -> Result<Option<FileInfo>, HostError> {
    call(&Request::GetFile { file_id })
}

/// Every album the user can see, owned and shared. Needs `read:metadata`.
pub fn list_albums() -> Result<Vec<AlbumInfo>, HostError> {
    call(&Request::ListAlbums)
}

// ---------------------------------------------------------------------------
// Changing the library
// ---------------------------------------------------------------------------

/// Add `tags` to a file, keeping its existing tags. Needs `write:tags`.
pub fn add_tags<S: AsRef<str>>(file_id: FileId, tags: &[S]) -> Result<(), HostError> {
    call_unit(&Request::AddTags {
        file_id,
        tags: tags.iter().map(|t| t.as_ref().to_owned()).collect(),
    })
}

/// The user's owned album called exactly `name`, created if missing. Needs
/// `write:albums`.
pub fn ensure_album(name: &str) -> Result<AlbumInfo, HostError> {
    call(&Request::EnsureAlbum {
        name: name.to_owned(),
    })
}

/// Add files to an owned album. Files already in it are left alone. Needs
/// `write:albums`. Does nothing for an empty list.
pub fn add_to_album(album_id: AlbumId, file_ids: &[FileId]) -> Result<(), HostError> {
    if file_ids.is_empty() {
        return Ok(());
    }
    call_unit(&Request::AddToAlbum {
        album_id,
        file_ids: file_ids.to_vec(),
    })
}

/// Remove files from an owned album. Needs `write:albums`. Does nothing for an
/// empty list.
pub fn remove_from_album(album_id: AlbumId, file_ids: &[FileId]) -> Result<(), HostError> {
    if file_ids.is_empty() {
        return Ok(());
    }
    call_unit(&Request::RemoveFromAlbum {
        album_id,
        file_ids: file_ids.to_vec(),
    })
}

/// Move files to the trash (recoverable, as in Ente). Needs `write:trash`.
/// Does nothing for an empty list.
pub fn trash_files(file_ids: &[FileId]) -> Result<(), HostError> {
    if file_ids.is_empty() {
        return Ok(());
    }
    call_unit(&Request::TrashFiles {
        file_ids: file_ids.to_vec(),
    })
}

/// Ask the host to copy a file into the export folder the user picked, at
/// `path` (relative, `/`-separated, no `..`). The plugin never sees the bytes
/// or the real folder. Needs `write:export`.
///
/// Use [`crate::path::sanitize_component`] on names you put into `path`.
pub fn export_file(
    file_id: FileId,
    path: &str,
    overwrite: bool,
) -> Result<ExportResult, HostError> {
    call(&Request::ExportFile {
        file_id,
        path: path.to_owned(),
        overwrite,
    })
}

// ---------------------------------------------------------------------------
// Bytes (`photovault_read`)
// ---------------------------------------------------------------------------

fn read(file_id: FileId, kind: ReadKind) -> Result<Vec<u8>, HostError> {
    let bytes = serde_json::to_vec(&ReadRequest { file_id, kind })
        .map_err(|e| backend::internal(e.to_string()))?;
    backend::read(&bytes)
}

/// The decrypted thumbnail (JPEG) of a file. Needs `read:thumbnails`.
pub fn read_thumbnail(file_id: FileId) -> Result<Vec<u8>, HostError> {
    read(file_id, ReadKind::Thumbnail)
}

/// The decrypted original of a file. Needs `read:originals`; the host may
/// refuse files bigger than its read limit with
/// [`ErrorCode::LimitExceeded`].
pub fn read_original(file_id: FileId) -> Result<Vec<u8>, HostError> {
    read(file_id, ReadKind::Original)
}

// ---------------------------------------------------------------------------
// Key/value storage and progress (always allowed)
// ---------------------------------------------------------------------------

/// Read a value this plugin stored with [`kv_set`], decoded as `T`. `None` if
/// the key is unset. Keys are at most 128 bytes.
pub fn kv_get<T: DeserializeOwned>(key: &str) -> Result<Option<T>, HostError> {
    let value: Option<serde_json::Value> = call(&Request::KvGet {
        key: key.to_owned(),
    })?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| HostError::new(ErrorCode::InvalidRequest, format!("kv `{key}`: {e}"))),
    }
}

/// Store `value` under `key`, on this device, for this plugin only. The host
/// caps the total size per plugin ([`ErrorCode::LimitExceeded`]).
pub fn kv_set<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<(), HostError> {
    let value = serde_json::to_value(value)
        .map_err(|e| HostError::new(ErrorCode::InvalidRequest, e.to_string()))?;
    call_unit(&Request::KvSet {
        key: key.to_owned(),
        value,
    })
}

/// Delete `key`.
pub fn kv_delete(key: &str) -> Result<(), HostError> {
    call_unit(&Request::KvSet {
        key: key.to_owned(),
        value: serde_json::Value::Null,
    })
}

/// Report progress of a long run (shown on the plugin's page).
pub fn progress(done: u64, total: Option<u64>, message: Option<&str>) -> Result<(), HostError> {
    call_unit(&Request::Progress {
        done,
        total,
        message: message.map(str::to_owned),
    })
}
