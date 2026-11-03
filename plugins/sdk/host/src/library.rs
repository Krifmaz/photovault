//! The [`Library`] trait: what a plugin host can do with the user's photos.
//!
//! The app or compute node implements it over the state it already holds
//! **decrypted, on the device** (Ente's local database and file cache). The
//! host runtime never sees keys or ciphertext and never talks to the server;
//! writes such as [`Library::add_to_album`] go through the app's normal,
//! end-to-end-encrypted code paths, exactly as if the user had made them.
//!
//! The runtime performs every permission and limit check *before* calling
//! into the library, so implementations only have to do the operation.

use photovault_plugin_manifest::abi::{
    AlbumId, AlbumInfo, FileId, FileInfo, FilePage, FileQuery, HostError,
};
use std::io::Write;

/// The user's library as a plugin host sees it.
pub trait Library: Send + Sync {
    /// One page of files matching `query`, ordered stably so that
    /// [`FilePage::next_cursor`] continues where this page ended.
    /// `query.limit` is always set and at most the host's page cap.
    fn list_files(&self, query: &FileQuery) -> Result<FilePage, HostError>;

    /// One file's metadata, or `None` if it does not exist or is not visible.
    fn get_file(&self, file_id: FileId) -> Result<Option<FileInfo>, HostError>;

    /// Every album visible to the user.
    fn list_albums(&self) -> Result<Vec<AlbumInfo>, HostError>;

    /// The decrypted thumbnail.
    fn thumbnail(&self, file_id: FileId) -> Result<Vec<u8>, HostError>;

    /// The decrypted original.
    fn original(&self, file_id: FileId) -> Result<Vec<u8>, HostError>;

    /// Stream the decrypted original into `out` and return the number of
    /// bytes written. Used for exports so large videos need not be held in
    /// memory; the default implementation buffers [`Library::original`].
    fn write_original(&self, file_id: FileId, out: &mut dyn Write) -> Result<u64, HostError> {
        let bytes = self.original(file_id)?;
        out.write_all(&bytes).map_err(|e| {
            HostError::new(
                photovault_plugin_manifest::abi::ErrorCode::Internal,
                format!("write failed: {e}"),
            )
        })?;
        Ok(bytes.len() as u64)
    }

    /// Add `tags` to a file, keeping existing tags.
    fn add_tags(&self, file_id: FileId, tags: &[String]) -> Result<(), HostError>;

    /// The user's owned album named exactly `name`, created if missing.
    fn ensure_album(&self, name: &str) -> Result<AlbumInfo, HostError>;

    /// Add files to an owned album.
    fn add_to_album(&self, album_id: AlbumId, file_ids: &[FileId]) -> Result<(), HostError>;

    /// Remove files from an owned album.
    fn remove_from_album(&self, album_id: AlbumId, file_ids: &[FileId]) -> Result<(), HostError>;

    /// Move files to the trash (recoverable, as in Ente).
    fn trash_files(&self, file_ids: &[FileId]) -> Result<(), HostError>;
}

#[cfg(feature = "testing")]
pub use memory::{LibraryWrite, MemoryLibrary};

#[cfg(feature = "testing")]
mod memory {
    use super::*;
    use photovault_plugin_manifest::abi::{AlbumKind, ErrorCode};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// A write a plugin made through a [`MemoryLibrary`], for assertions.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LibraryWrite {
        /// [`Library::add_tags`].
        AddTags {
            /// File.
            file_id: FileId,
            /// Tags added.
            tags: Vec<String>,
        },
        /// [`Library::ensure_album`] created a new album.
        CreateAlbum {
            /// New album id.
            album_id: AlbumId,
            /// Name.
            name: String,
        },
        /// [`Library::add_to_album`].
        AddToAlbum {
            /// Album.
            album_id: AlbumId,
            /// Files.
            file_ids: Vec<FileId>,
        },
        /// [`Library::remove_from_album`].
        RemoveFromAlbum {
            /// Album.
            album_id: AlbumId,
            /// Files.
            file_ids: Vec<FileId>,
        },
        /// [`Library::trash_files`].
        Trash {
            /// Files.
            file_ids: Vec<FileId>,
        },
    }

    #[derive(Default)]
    struct State {
        files: BTreeMap<FileId, FileInfo>,
        albums: BTreeMap<AlbumId, AlbumInfo>,
        thumbnails: BTreeMap<FileId, Vec<u8>>,
        originals: BTreeMap<FileId, Vec<u8>>,
        trashed: Vec<FileId>,
        writes: Vec<LibraryWrite>,
        next_album: AlbumId,
    }

    /// An in-memory [`Library`] for tests and examples. It records every
    /// write so tests can assert on what a plugin did.
    #[derive(Default)]
    pub struct MemoryLibrary {
        state: Mutex<State>,
    }

    fn not_found(what: &str) -> HostError {
        HostError::new(ErrorCode::NotFound, format!("{what} not found"))
    }

    impl MemoryLibrary {
        /// An empty library.
        pub fn new() -> Self {
            Self::default()
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// Add or replace a file, with optional thumbnail and original bytes.
        pub fn add_file(
            &self,
            file: FileInfo,
            thumbnail: Option<Vec<u8>>,
            original: Option<Vec<u8>>,
        ) {
            let mut s = self.lock();
            let id = file.id;
            s.files.insert(id, file);
            if let Some(t) = thumbnail {
                s.thumbnails.insert(id, t);
            }
            if let Some(o) = original {
                s.originals.insert(id, o);
            }
        }

        /// Add or replace an album.
        pub fn add_album(&self, album: AlbumInfo) {
            let mut s = self.lock();
            s.next_album = s.next_album.max(album.id + 1);
            s.albums.insert(album.id, album);
        }

        /// A simple image file for tests.
        pub fn sample_file(id: FileId, title: &str) -> FileInfo {
            FileInfo {
                id,
                album_ids: vec![],
                title: title.into(),
                file_type: photovault_plugin_manifest::abi::FileType::Image,
                creation_time: 1_700_000_000_000_000 + id,
                modification_time: 1_700_000_000_000_000 + id,
                latitude: None,
                longitude: None,
                width: None,
                height: None,
                size: None,
                device_folder: None,
                caption: None,
                tags: vec![],
                is_favorite: false,
                is_archived: false,
            }
        }

        /// Every write so far, in order.
        pub fn writes(&self) -> Vec<LibraryWrite> {
            self.lock().writes.clone()
        }

        /// Files currently in the trash.
        pub fn trashed(&self) -> Vec<FileId> {
            self.lock().trashed.clone()
        }

        /// A file's current metadata (including trashed files).
        pub fn file(&self, id: FileId) -> Option<FileInfo> {
            self.lock().files.get(&id).cloned()
        }

        fn owned_album(s: &State, id: AlbumId) -> Result<(), HostError> {
            match s.albums.get(&id) {
                Some(a) if a.is_owned => Ok(()),
                Some(_) => Err(HostError::new(
                    ErrorCode::PermissionDenied,
                    "album is not owned by the user",
                )),
                None => Err(not_found("album")),
            }
        }
    }

    impl Library for MemoryLibrary {
        fn list_files(&self, q: &FileQuery) -> Result<FilePage, HostError> {
            let s = self.lock();
            let after: Option<FileId> = match &q.cursor {
                Some(c) => Some(
                    c.parse()
                        .map_err(|_| HostError::new(ErrorCode::InvalidRequest, "bad cursor"))?,
                ),
                None => None,
            };
            let limit = q.limit.unwrap_or(500) as usize;
            let mut it = s
                .files
                .values()
                .filter(|f| !s.trashed.contains(&f.id))
                .filter(|f| after.is_none_or(|a| f.id > a))
                .filter(|f| q.album_id.is_none_or(|a| f.album_ids.contains(&a)))
                .filter(|f| q.created_after.is_none_or(|t| f.creation_time >= t))
                .filter(|f| q.created_before.is_none_or(|t| f.creation_time < t))
                .filter(|f| q.file_type.is_none_or(|t| f.file_type == t));
            let files: Vec<FileInfo> = it.by_ref().take(limit).cloned().collect();
            let more = it.next().is_some();
            let next_cursor = if more {
                files.last().map(|f| f.id.to_string())
            } else {
                None
            };
            Ok(FilePage { files, next_cursor })
        }

        fn get_file(&self, id: FileId) -> Result<Option<FileInfo>, HostError> {
            let s = self.lock();
            Ok(s.files
                .get(&id)
                .filter(|_| !s.trashed.contains(&id))
                .cloned())
        }

        fn list_albums(&self) -> Result<Vec<AlbumInfo>, HostError> {
            let s = self.lock();
            Ok(s.albums
                .values()
                .map(|a| {
                    let mut a = a.clone();
                    let n = s
                        .files
                        .values()
                        .filter(|f| f.album_ids.contains(&a.id) && !s.trashed.contains(&f.id))
                        .count();
                    a.file_count = Some(n as u64);
                    a
                })
                .collect())
        }

        fn thumbnail(&self, id: FileId) -> Result<Vec<u8>, HostError> {
            self.lock()
                .thumbnails
                .get(&id)
                .cloned()
                .ok_or_else(|| not_found("thumbnail"))
        }

        fn original(&self, id: FileId) -> Result<Vec<u8>, HostError> {
            self.lock()
                .originals
                .get(&id)
                .cloned()
                .ok_or_else(|| not_found("original"))
        }

        fn add_tags(&self, id: FileId, tags: &[String]) -> Result<(), HostError> {
            let mut s = self.lock();
            let f = s.files.get_mut(&id).ok_or_else(|| not_found("file"))?;
            for t in tags {
                if !f.tags.contains(t) {
                    f.tags.push(t.clone());
                }
            }
            s.writes.push(LibraryWrite::AddTags {
                file_id: id,
                tags: tags.to_vec(),
            });
            Ok(())
        }

        fn ensure_album(&self, name: &str) -> Result<AlbumInfo, HostError> {
            let mut s = self.lock();
            if let Some(a) = s.albums.values().find(|a| a.is_owned && a.name == name) {
                return Ok(a.clone());
            }
            let id = s.next_album.max(1);
            s.next_album = id + 1;
            let a = AlbumInfo {
                id,
                name: name.into(),
                kind: AlbumKind::Album,
                is_owned: true,
                file_count: Some(0),
            };
            s.albums.insert(id, a.clone());
            s.writes.push(LibraryWrite::CreateAlbum {
                album_id: id,
                name: name.into(),
            });
            Ok(a)
        }

        fn add_to_album(&self, album_id: AlbumId, file_ids: &[FileId]) -> Result<(), HostError> {
            let mut s = self.lock();
            Self::owned_album(&s, album_id)?;
            for id in file_ids {
                let f = s.files.get_mut(id).ok_or_else(|| not_found("file"))?;
                if !f.album_ids.contains(&album_id) {
                    f.album_ids.push(album_id);
                }
            }
            s.writes.push(LibraryWrite::AddToAlbum {
                album_id,
                file_ids: file_ids.to_vec(),
            });
            Ok(())
        }

        fn remove_from_album(
            &self,
            album_id: AlbumId,
            file_ids: &[FileId],
        ) -> Result<(), HostError> {
            let mut s = self.lock();
            Self::owned_album(&s, album_id)?;
            for id in file_ids {
                if let Some(f) = s.files.get_mut(id) {
                    f.album_ids.retain(|a| *a != album_id);
                }
            }
            s.writes.push(LibraryWrite::RemoveFromAlbum {
                album_id,
                file_ids: file_ids.to_vec(),
            });
            Ok(())
        }

        fn trash_files(&self, file_ids: &[FileId]) -> Result<(), HostError> {
            let mut s = self.lock();
            for id in file_ids {
                if !s.files.contains_key(id) {
                    return Err(not_found("file"));
                }
            }
            for id in file_ids {
                if !s.trashed.contains(id) {
                    s.trashed.push(*id);
                }
            }
            s.writes.push(LibraryWrite::Trash {
                file_ids: file_ids.to_vec(),
            });
            Ok(())
        }
    }
}
