//! Sync of collections and file diffs into the local store, the in-memory
//! timeline index, thumbnails and downloads.
//!
//! Sync follows the apps (`pullCollectionFiles` in Ente's web app and the
//! mobile `CollectionsService`): `GET /collections/v2?sinceTime` for changed
//! collections, then `GET /collections/v2/diff` for every collection whose
//! `updationTime` moved past what we have. Collection keys are opened with
//! the master key (owner) or the key pair (shared with us, sealed box), file
//! keys with the collection key, metadata and magic metadata with the file
//! key. Everything decrypted is sealed under the device key before it is
//! written ([`crate::store`]).
//!
//! The timeline is served from memory: on open and after every change the
//! index is rebuilt from the store, so `timeline_page` is a slice copy.

use crate::client::{Inner, Session};
use crate::crypto::{self, DECRYPTION_CHUNK_SIZE};
use crate::metadata::{
    SUB_TYPE_DEFAULT_HIDDEN, VISIBILITY_ARCHIVED, VISIBILITY_HIDDEN, decrypt_json, decrypt_magic,
};
use crate::model::*;
use crate::{
    Collection, CoreError, FileKind, MapPoint, Result, SyncListener, SyncProgress, TimelineItem,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;

/// kv key of the `/collections/v2` cursor (plain).
const KV_COLLECTIONS_SINCE: &str = "collections_since";
/// Thumbnail cache cap.
const THUMB_CACHE_MAX: u64 = 512 * 1024 * 1024;
/// Original-file cache cap.
const FILE_CACHE_MAX: u64 = 2 * 1024 * 1024 * 1024;

/// How many syncs in a row may stop at the same entry that fails to
/// decrypt before the cursor moves past it (a damaged entry must not make
/// every sync re-read the rest of the collection forever).
const MAX_DECRYPT_RETRIES: u32 = 3;
/// kv key prefix of the retry state of a cursor held back by such an entry.
const KV_RETRY: &str = "retry:";

/// A decrypted collection, as sealed in the store.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CollectionRec {
    pub id: i64,
    pub key: Vec<u8>,
    pub name: String,
    pub kind: String,
    pub owner_id: i64,
    pub owned: bool,
    pub sub_type: Option<i64>,
    pub visibility: Option<i64>,
    pub updation_time: i64,
    /// Owner's email (shared with us).
    #[serde(default)]
    pub owner_email: String,
    /// People it is shared with (owned albums).
    #[serde(default)]
    pub sharees: Vec<crate::albums::ShareeRec>,
    /// The public link, if any.
    #[serde(default)]
    pub public_link: Option<crate::albums::PublicLinkRec>,
    /// The owner's decrypted private magic metadata and its server version
    /// (kept whole so an update preserves keys other apps set).
    #[serde(default)]
    pub magic: Map<String, Value>,
    #[serde(default)]
    pub magic_version: i64,
}

impl std::fmt::Debug for CollectionRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectionRec")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Drop for CollectionRec {
    fn drop(&mut self) {
        crypto::wipe(&mut self.key);
    }
}

impl CollectionRec {
    pub(crate) fn hidden(&self) -> bool {
        self.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN) || self.visibility == Some(VISIBILITY_HIDDEN)
    }
    pub(crate) fn archived(&self) -> bool {
        self.visibility == Some(VISIBILITY_ARCHIVED)
    }
}

/// A decrypted file, as sealed in the store.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct FileRec {
    pub id: i64,
    pub owner_id: i64,
    pub key: Vec<u8>,
    pub file_header: String,
    pub thumb_header: String,
    pub file_size: Option<u64>,
    /// Decrypted metadata JSON.
    pub metadata: Value,
    /// Decrypted public magic metadata.
    pub pub_magic: Map<String, Value>,
    /// Decrypted private magic metadata.
    pub magic: Map<String, Value>,
}

impl std::fmt::Debug for FileRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRec")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for FileRec {
    fn drop(&mut self) {
        crypto::wipe(&mut self.key);
    }
}

impl FileRec {
    fn kind(&self) -> FileKind {
        match self.metadata.get("fileType").and_then(Value::as_i64) {
            Some(0) => FileKind::Image,
            Some(1) => FileKind::Video,
            Some(2) => FileKind::LivePhoto,
            _ => FileKind::Unknown,
        }
    }
    fn i64_of(m: &Map<String, Value>, k: &str) -> Option<i64> {
        m.get(k)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    }
    pub(crate) fn item(&self, collection_id: i64) -> TimelineItem {
        let md = self.metadata.as_object().cloned().unwrap_or_default();
        let taken = Self::i64_of(&self.pub_magic, "editedTime")
            .filter(|t| *t != 0)
            .or_else(|| Self::i64_of(&md, "creationTime"))
            .unwrap_or(0);
        let title = self
            .pub_magic
            .get("editedName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| md.get("title").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let dim = |k: &str| {
            Self::i64_of(&self.pub_magic, k)
                .filter(|v| *v > 0)
                .map(|v| v.min(u32::MAX as i64) as u32)
        };
        let duration_ms = Self::i64_of(&md, "duration")
            .filter(|d| *d > 0)
            .map(|d| (d.saturating_mul(1000)).min(u32::MAX as i64) as u32);
        TimelineItem {
            id: self.id,
            collection_id,
            kind: self.kind(),
            taken_at_us: taken,
            width: dim("w").unwrap_or(0),
            height: dim("h").unwrap_or(0),
            duration_ms,
            has_stream: false,
            title,
        }
    }
    pub(crate) fn hash(&self) -> Option<String> {
        crate::metadata::metadata_hash(&self.metadata)
    }
    fn archived(&self) -> bool {
        Self::i64_of(&self.magic, "visibility") == Some(VISIBILITY_ARCHIVED)
    }
    /// Where the photo was taken: the user's edit (public magic `lat`/`long`)
    /// over the metadata's `latitude`/`longitude`, as Ente's apps read it.
    fn location(&self) -> Option<(f64, f64)> {
        crate::places::location(&self.pub_magic, &self.metadata)
    }
}

/// The in-memory library index.
#[derive(Debug, Default)]
pub struct Index {
    /// Every file once, newest first (collection id = a representative).
    pub(crate) all: Vec<TimelineItem>,
    /// Positions in `all` shown in the timeline.
    timeline: Vec<u32>,
    /// Positions in `all` per collection.
    pub(crate) by_collection: HashMap<i64, Vec<u32>>,
    /// Visible collections, ready to return.
    pub(crate) collections: Vec<Collection>,
    /// Timeline files with a location, newest first.
    located: Vec<MapPoint>,
    /// Albums, sharing and trash ([`crate::albums`]).
    pub(crate) albums: crate::albums::AlbumIndex,
    /// Owned files by content hash, with their collections.
    pub(crate) hashes: HashMap<String, (i64, Vec<i64>)>,
}

impl Index {
    /// Every file once, newest first (used by search, faces and memories).
    pub(crate) fn all_items(&self) -> &[TimelineItem] {
        &self.all
    }

    /// Files shown in the timeline, newest first.
    pub(crate) fn timeline_items(&self) -> impl Iterator<Item = &TimelineItem> {
        self.timeline.iter().map(|&p| &self.all[p as usize])
    }

    /// Visible collections.
    pub(crate) fn visible_collections(&self) -> &[Collection] {
        &self.collections
    }

    /// Files of a collection, newest first.
    pub(crate) fn collection_items(&self, id: i64) -> impl Iterator<Item = &TimelineItem> {
        self.by_collection
            .get(&id)
            .into_iter()
            .flatten()
            .map(|&p| &self.all[p as usize])
    }
}

/// Decrypt a remote collection with the session's keys.
pub(crate) fn decrypt_collection(c: &RemoteCollection, s: &Session) -> Result<CollectionRec> {
    let owned = c.owner.id == s.account.user_id;
    let key = if owned {
        let nonce = c
            .key_decryption_nonce
            .as_deref()
            .ok_or_else(|| CoreError::crypto("owned collection without keyDecryptionNonce"))?;
        crypto::secretbox_open(
            &crypto::unb64(&c.encrypted_key)?,
            &crypto::unb64(nonce)?,
            &s.master_key,
        )?
    } else {
        crypto::box_seal_open(
            &crypto::unb64(&c.encrypted_key)?,
            &s.public_key,
            &s.secret_key,
        )?
    };
    let name = match (&c.name, &c.encrypted_name, &c.name_decryption_nonce) {
        (Some(n), _, _) if !n.is_empty() => n.clone(),
        (_, Some(en), Some(nn)) if !en.is_empty() => String::from_utf8_lossy(
            &crypto::secretbox_open(&crypto::unb64(en)?, &crypto::unb64(nn)?, &key)?,
        )
        .into_owned(),
        _ => String::new(),
    };
    let (sub_type, visibility, magic, magic_version) = match &c.magic_metadata {
        Some(mm) if owned && !mm.data.is_empty() => {
            let m = decrypt_magic(mm, &key)?;
            (
                m.data.get("subType").and_then(Value::as_i64),
                m.data.get("visibility").and_then(Value::as_i64),
                m.data,
                m.version,
            )
        }
        Some(mm) if owned => (None, None, Map::new(), mm.version),
        _ => (None, None, Map::new(), 0),
    };
    let public_link = crate::albums::public_link_of(c, &key);
    Ok(CollectionRec {
        id: c.id,
        key,
        name,
        kind: c.kind.clone(),
        owner_id: c.owner.id,
        owned,
        sub_type,
        visibility,
        updation_time: c.updation_time,
        owner_email: c.owner.email.clone(),
        sharees: crate::albums::sharees_of(c),
        public_link,
        magic,
        magic_version,
    })
}

/// Decrypt one diff entry with its collection's key.
pub(crate) fn decrypt_file(f: &RemoteFile, collection_key: &[u8]) -> Result<FileRec> {
    let key = crypto::secretbox_open(
        &crypto::unb64(&f.encrypted_key)?,
        &crypto::unb64(&f.key_decryption_nonce)?,
        collection_key,
    )?;
    let md = f
        .metadata
        .as_ref()
        .ok_or_else(|| CoreError::crypto("file without metadata"))?;
    let metadata = decrypt_json(
        md.encrypted_data.as_deref().unwrap_or_default(),
        &md.decryption_header,
        &key,
    )?;
    let magic = |m: &Option<RemoteMagicMetadata>| -> Result<Map<String, Value>> {
        Ok(match m {
            Some(m) if !m.data.is_empty() => decrypt_magic(m, &key)?.data,
            _ => Map::new(),
        })
    };
    let pub_magic = magic(&f.pub_magic_metadata)?;
    let priv_magic = magic(&f.magic_metadata)?;
    Ok(FileRec {
        id: f.id,
        owner_id: f.owner_id,
        file_header: f
            .file
            .as_ref()
            .map(|a| a.decryption_header.clone())
            .unwrap_or_default(),
        thumb_header: f
            .thumbnail
            .as_ref()
            .map(|a| a.decryption_header.clone())
            .unwrap_or_default(),
        file_size: f.file.as_ref().and_then(|a| a.size),
        key,
        metadata,
        pub_magic,
        magic: priv_magic,
    })
}

/// Total size of a directory's files, and remove the oldest until under `max`.
fn trim_dir(dir: &Path, max: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| {
                (
                    m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    m.len(),
                    e.path(),
                )
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort();
    for (_, len, p) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Write `data` to `path` atomically.
async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        tokio::fs::create_dir_all(d).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", hex::encode(crypto::random_bytes(4))));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

impl Inner {
    // ---- store access ----

    pub(crate) fn collection_rec(&self, id: i64) -> Result<Option<CollectionRec>> {
        let st = self.store();
        let blob: Option<Vec<u8>> = st
            .conn()
            .query_row(
                "SELECT sealed FROM collections WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        blob.map(|b| st.open_sealed(&b)).transpose()
    }

    pub(crate) fn file_rec(&self, id: i64) -> Result<FileRec> {
        let st = self.store();
        let blob: Option<Vec<u8>> = st
            .conn()
            .query_row("SELECT sealed FROM files WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .optional()?;
        match blob {
            Some(b) => st.open_sealed(&b),
            None => Err(CoreError::storage(format!("unknown file {id}"))),
        }
    }

    pub(crate) fn all_collections(&self) -> Result<Vec<CollectionRec>> {
        let (blobs, opener) = {
            let st = self.store();
            let mut q = st
                .conn()
                .prepare("SELECT sealed FROM collections ORDER BY id")?;
            let blobs: Vec<Vec<u8>> = q
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            (blobs, st.opener())
        };
        blobs.iter().map(|b| opener.open_sealed(b)).collect()
    }

    pub(crate) fn save_collection(&self, c: &CollectionRec) -> Result<()> {
        let st = self.store();
        let blob = st.seal(c)?;
        st.conn().execute(
            "INSERT INTO collections (id, updation_time, sealed) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET updation_time = excluded.updation_time, sealed = excluded.sealed",
            params![c.id, c.updation_time, blob],
        )?;
        Ok(())
    }

    /// Store a decrypted file and its membership in `collection_id`.
    pub(crate) fn save_file(&self, f: &FileRec, collection_id: i64) -> Result<()> {
        let mut st = self.store();
        let blob = st.seal(f)?;
        let tx = st.conn_mut().transaction()?;
        tx.execute(
            "INSERT INTO files (id, sealed) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET sealed = excluded.sealed",
            params![f.id, blob],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO collection_files (collection_id, file_id) VALUES (?1, ?2)",
            params![collection_id, f.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove a file from a collection; drop it entirely when it is in none.
    pub(crate) fn remove_membership(&self, collection_id: i64, file_id: i64) -> Result<()> {
        let gone = {
            let mut st = self.store();
            let tx = st.conn_mut().transaction()?;
            tx.execute(
                "DELETE FROM collection_files WHERE collection_id = ?1 AND file_id = ?2",
                params![collection_id, file_id],
            )?;
            let left: i64 = tx.query_row(
                "SELECT COUNT(*) FROM collection_files WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )?;
            if left == 0 {
                tx.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
            }
            tx.commit()?;
            left == 0
        };
        if gone {
            self.drop_cached(file_id);
        }
        Ok(())
    }

    fn drop_cached(&self, file_id: i64) {
        let _ = std::fs::remove_file(self.thumb_path(file_id));
        let _ = std::fs::remove_file(self.file_cache_path(file_id));
    }

    pub(crate) fn delete_collection(&self, id: i64) -> Result<()> {
        let orphans: Vec<i64> = {
            let mut st = self.store();
            let tx = st.conn_mut().transaction()?;
            let files: Vec<i64> = {
                let mut q =
                    tx.prepare("SELECT file_id FROM collection_files WHERE collection_id = ?1")?;
                q.query_map(params![id], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            tx.execute(
                "DELETE FROM collection_files WHERE collection_id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
            let mut orphans = Vec::new();
            for f in files {
                let left: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM collection_files WHERE file_id = ?1",
                    params![f],
                    |r| r.get(0),
                )?;
                if left == 0 {
                    tx.execute("DELETE FROM files WHERE id = ?1", params![f])?;
                    orphans.push(f);
                }
            }
            tx.commit()?;
            orphans
        };
        for f in orphans {
            self.drop_cached(f);
        }
        Ok(())
    }

    // ---- sync ----

    pub async fn sync(&self, listener: Option<Arc<dyn SyncListener>>) -> Result<SyncProgress> {
        let _guard = self.sync_lock.lock().await;
        let s = self.session()?;
        let api = self.api()?;
        let since: i64 = self
            .store()
            .get_plain(KV_COLLECTIONS_SINCE)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let res: CollectionsResponse = api
            .get_json("/collections/v2", &[("sinceTime", since.to_string())])
            .await?;
        let mut max_since = since;
        let mut first_failed: Option<i64> = None;
        for c in &res.collections {
            max_since = max_since.max(c.updation_time);
            if c.is_deleted {
                // Keep the key: its files may sit in the trash.
                self.remember_deleted_key(c, &s);
                self.delete_collection(c.id)?;
                continue;
            }
            match decrypt_collection(c, &s) {
                Ok(rec) => self.save_collection(&rec)?,
                // A collection we cannot open (e.g. a share whose key we lack
                // yet): keep the cursor before it so the next sync retries.
                Err(_) => {
                    first_failed =
                        Some(first_failed.map_or(c.updation_time, |f| f.min(c.updation_time)));
                }
            }
        }
        let cursor = self.cursor_after_failures("collections", since, max_since, first_failed)?;
        self.store()
            .put_plain(KV_COLLECTIONS_SINCE, &cursor.to_string())?;

        let todo: Vec<(i64, i64, i64)> = {
            let st = self.store();
            let mut q = st
                .conn()
                .prepare("SELECT id, updation_time, synced_until FROM collections ORDER BY id")?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut done_collections = 0u32;
        for (cid, updated, synced) in todo {
            if synced == 0 || updated > synced {
                self.sync_collection(&api, cid, synced, updated).await?;
            }
            done_collections += 1;
            if let Some(l) = &listener {
                l.on_progress(SyncProgress {
                    collections: done_collections,
                    files: self.file_count()?,
                    done: false,
                });
            }
        }
        // Which videos have a stream (best effort: older servers lack the endpoint).
        let _ = self.sync_stream_status(&api).await;
        self.rebuild_index()?;
        crate::offload(|| trim_dir(&self.cache_dir.join("thumbs"), THUMB_CACHE_MAX));
        let p = SyncProgress {
            collections: done_collections,
            files: self.file_count()?,
            done: true,
        };
        if let Some(l) = &listener {
            l.on_progress(p.clone());
        }
        Ok(p)
    }

    fn file_count(&self) -> Result<u64> {
        let n: i64 = self
            .store()
            .conn()
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Where a cursor may move after a pass from `from` that reached
    /// `advanced`: not past the first entry that failed to decrypt, unless
    /// that entry has already held it back [`MAX_DECRYPT_RETRIES`] times.
    fn cursor_after_failures(
        &self,
        name: &str,
        from: i64,
        advanced: i64,
        first_failed: Option<i64>,
    ) -> Result<i64> {
        let key = format!("{KV_RETRY}{name}");
        let st = self.store();
        let Some(failed) = first_failed else {
            st.delete(&key)?;
            return Ok(advanced);
        };
        // `sinceTime` is exclusive: stop just before the failed entry.
        let stuck = (failed - 1).max(from);
        let tries = st
            .get_plain(&key)?
            .and_then(|v| {
                let (at, n) = v.split_once(':')?;
                (at.parse::<i64>().ok()? == stuck).then(|| n.parse::<u32>().ok())?
            })
            .unwrap_or(0)
            + 1;
        if tries > MAX_DECRYPT_RETRIES {
            st.delete(&key)?;
            return Ok(advanced);
        }
        st.put_plain(&key, &format!("{stuck}:{tries}"))?;
        Ok(advanced.min(stuck))
    }

    /// Pull the diff of collection `cid` from `since`. `updated` is the
    /// collection's `updationTime` when this sync started: once the diff is
    /// drained the collection counts as synced up to it, so a rename or an
    /// empty collection (no file changes) does not trigger a diff every sync.
    async fn sync_collection(
        &self,
        api: &crate::api::Api,
        cid: i64,
        mut since: i64,
        updated: i64,
    ) -> Result<()> {
        let Some(c) = self.collection_rec(cid)? else {
            return Ok(());
        };
        let from = since;
        let mut first_failed: Option<i64> = None;
        loop {
            let page: DiffResponse = api
                .get_json(
                    "/collections/v2/diff",
                    &[
                        ("collectionID", cid.to_string()),
                        ("sinceTime", since.to_string()),
                    ],
                )
                .await?;
            for f in &page.diff {
                since = since.max(f.updation_time);
                if f.is_deleted {
                    self.remove_membership(cid, f.id)?;
                    continue;
                }
                match decrypt_file(f, &c.key) {
                    Ok(rec) => {
                        self.save_file(&rec, cid)?;
                    }
                    // Keep the stored cursor before it: retried next sync.
                    Err(_) => {
                        first_failed =
                            Some(first_failed.map_or(f.updation_time, |x| x.min(f.updation_time)));
                    }
                }
            }
            let finished = !page.has_more || page.diff.is_empty();
            let reached = if finished { since.max(updated) } else { since };
            let mark = if first_failed.is_some() || finished {
                self.cursor_after_failures(&format!("c{cid}"), from, reached, first_failed)?
            } else {
                reached
            };
            // Never store 0: it means "never synced".
            let mark = mark.max(1);
            self.store().conn().execute(
                "UPDATE collections SET synced_until = ?1 WHERE id = ?2",
                params![mark, cid],
            )?;
            if finished {
                break;
            }
        }
        Ok(())
    }

    // ---- index ----

    /// Rebuild the in-memory index from the store. Opening every sealed
    /// record is CPU work: it runs off the async worker, and the store lock
    /// is held only while the blobs are read.
    pub(crate) fn rebuild_index(&self) -> Result<()> {
        let idx = if self.session().is_ok() {
            crate::offload(|| self.build_index())?
        } else {
            Index::default()
        };
        *self.index.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(idx);
        Ok(())
    }

    fn build_index(&self) -> Result<Index> {
        let user_id = self.session()?.account.user_id;
        let cols: HashMap<i64, CollectionRec> = self
            .all_collections()?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        let (members, blobs, streams) = {
            let st = self.store();
            let streams = Inner::known_streams(st.conn())?;
            let mut members: HashMap<i64, Vec<i64>> = HashMap::new();
            {
                let mut q = st.conn().prepare(
                    "SELECT file_id, collection_id FROM collection_files ORDER BY collection_id",
                )?;
                for r in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                    let (f, c) = r?;
                    members.entry(f).or_default().push(c);
                }
            }
            let mut q = st.conn().prepare("SELECT sealed FROM files")?;
            let blobs: Vec<Vec<u8>> = q
                .query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<_>>()?;
            (members, blobs, streams)
        };
        let opener = self.store().opener();
        let files: Vec<FileRec> = blobs
            .iter()
            .filter_map(|b| opener.open_sealed::<FileRec>(b).ok())
            .collect();
        drop(opener);
        // (item, collections, in timeline, location)
        type Entry = (TimelineItem, Vec<i64>, bool, Option<(f64, f64)>);
        let mut entries: Vec<Entry> = Vec::with_capacity(files.len());
        let mut hashes = HashMap::new();
        for f in files {
            let in_cols: Vec<i64> = members.get(&f.id).cloned().unwrap_or_default();
            let known: Vec<&CollectionRec> = in_cols.iter().filter_map(|c| cols.get(c)).collect();
            if known.is_empty() {
                continue;
            }
            let visible_home: Vec<&&CollectionRec> = known
                .iter()
                .filter(|c| c.owned && !c.hidden() && !c.archived())
                .collect();
            let in_timeline = !f.archived() && !visible_home.is_empty();
            let rep = visible_home.first().map(|c| c.id).unwrap_or(known[0].id);
            if f.owner_id == user_id
                && let Some(h) = f.hash()
            {
                hashes.entry(h).or_insert((f.id, in_cols.clone()));
            }
            let loc = if in_timeline { f.location() } else { None };
            let mut item = f.item(rep);
            item.has_stream = Inner::has_stream(&f, &streams);
            entries.push((item, in_cols, in_timeline, loc));
        }
        entries.sort_by_key(|e| std::cmp::Reverse((e.0.taken_at_us, e.0.id)));
        let mut idx = Index {
            hashes,
            ..Default::default()
        };
        for (pos, (item, in_cols, in_timeline, loc)) in entries.into_iter().enumerate() {
            let pos = pos as u32;
            if in_timeline {
                idx.timeline.push(pos);
            }
            if let Some((latitude, longitude)) = loc {
                idx.located.push(MapPoint {
                    file_id: item.id,
                    latitude,
                    longitude,
                    taken_at_us: item.taken_at_us,
                });
            }
            for c in in_cols {
                idx.by_collection.entry(c).or_default().push(pos);
            }
            idx.all.push(item);
        }
        let mut visible: Vec<(i64, Collection)> = cols
            .values()
            .filter(|c| !c.hidden())
            .filter_map(|c| {
                let list = idx.by_collection.get(&c.id);
                let count = list.map_or(0, Vec::len) as u64;
                if c.kind == "uncategorized" && count == 0 {
                    return None;
                }
                let newest = list.and_then(|l| l.first()).map(|&p| &idx.all[p as usize]);
                Some((
                    newest.map_or(c.updation_time, |n| n.taken_at_us),
                    Collection {
                        id: c.id,
                        name: c.name.clone(),
                        kind: c.kind.clone(),
                        cover_file_id: newest.map(|n| n.id),
                        count,
                        updated_at_us: c.updation_time,
                    },
                ))
            })
            .collect();
        visible.sort_by_key(|v| std::cmp::Reverse((v.0, v.1.id)));
        idx.collections = visible.into_iter().map(|(_, c)| c).collect();
        crate::albums::build_album_index(&mut idx, &cols, user_id);
        idx.albums.trash = self.load_trash(&idx)?;
        Ok(idx)
    }

    pub(crate) fn index(&self) -> Arc<Index> {
        self.index.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn timeline_count(&self) -> u64 {
        self.index().timeline.len() as u64
    }

    pub fn timeline_page(&self, offset: u64, limit: u32) -> Vec<TimelineItem> {
        let idx = self.index();
        let start = (offset.min(idx.timeline.len() as u64)) as usize;
        idx.timeline[start..]
            .iter()
            .take(limit as usize)
            .map(|&p| idx.all[p as usize].clone())
            .collect()
    }

    pub fn collections(&self) -> Vec<Collection> {
        self.index().collections.clone()
    }

    pub fn map_points(&self) -> Vec<MapPoint> {
        self.index().located.clone()
    }

    pub fn on_this_day(&self, now_us: i64, utc_offset_secs: i32, limit: u32) -> Vec<TimelineItem> {
        let idx = self.index();
        crate::places::on_this_day(
            idx.timeline.iter().map(|&p| &idx.all[p as usize]),
            now_us,
            utc_offset_secs,
            limit,
        )
    }

    pub fn collection_page(
        &self,
        collection_id: i64,
        offset: u64,
        limit: u32,
    ) -> Vec<TimelineItem> {
        let idx = self.index();
        let Some(list) = idx.by_collection.get(&collection_id) else {
            return Vec::new();
        };
        let start = (offset.min(list.len() as u64)) as usize;
        list[start..]
            .iter()
            .take(limit as usize)
            .map(|&p| {
                let mut it = idx.all[p as usize].clone();
                it.collection_id = collection_id;
                it
            })
            .collect()
    }

    // ---- objects ----

    fn thumb_path(&self, id: i64) -> PathBuf {
        self.cache_dir.join("thumbs").join(id.to_string())
    }

    fn file_cache_path(&self, id: i64) -> PathBuf {
        self.cache_dir.join("files").join(id.to_string())
    }

    pub async fn thumbnail(&self, file_id: i64) -> Result<Vec<u8>> {
        self.session()?;
        // Trashed files are no longer in the library, but the trash shows them.
        let f = match self.file_rec(file_id) {
            Ok(f) => f,
            Err(e) => self.trashed_file_rec(file_id)?.ok_or(e)?,
        };
        let header = crypto::unb64(&f.thumb_header)?;
        let path = self.thumb_path(file_id);
        // The cache holds the server's ciphertext: encrypted at rest under the file key.
        if let Ok(ct) = tokio::fs::read(&path).await {
            if let Ok(t) = crypto::decrypt_blob(&ct, &header, &f.key) {
                return Ok(t);
            }
            let _ = tokio::fs::remove_file(&path).await;
        }
        let api = self.api()?;
        let res = api.get_object(&format!("/files/preview/{file_id}")).await?;
        let ct = res.bytes().await.map_err(CoreError::network)?;
        let plain = crypto::decrypt_blob(&ct, &header, &f.key)?;
        write_atomic(&path, &ct).await?;
        Ok(plain)
    }

    pub async fn download(&self, file_id: i64, dest_path: &str) -> Result<String> {
        self.session()?;
        let f = self.file_rec(file_id)?;
        let header = crypto::unb64(&f.file_header)?;
        let dest = PathBuf::from(dest_path);
        if let Some(d) = dest.parent() {
            tokio::fs::create_dir_all(d).await?;
        }
        let tmp_dest = dest.with_extension(format!("part{}", hex::encode(crypto::random_bytes(4))));
        let cache = self.file_cache_path(file_id);
        let result = async {
            if tokio::fs::try_exists(&cache).await.unwrap_or(false) {
                let ok = decrypt_file_to(&cache, &tmp_dest, &header, &f.key).await;
                if ok.is_ok() {
                    return Ok(());
                }
                let _ = tokio::fs::remove_file(&cache).await;
            }
            self.fetch_original(file_id, &cache, &tmp_dest, &header, &f.key)
                .await
        }
        .await;
        match result {
            Ok(()) => {
                tokio::fs::rename(&tmp_dest, &dest).await?;
                crate::offload(|| trim_dir(&self.cache_dir.join("files"), FILE_CACHE_MAX));
                Ok(dest.to_string_lossy().into_owned())
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_dest).await;
                Err(e)
            }
        }
    }

    /// Stream the original: ciphertext to the cache, plaintext to `dest`.
    async fn fetch_original(
        &self,
        file_id: i64,
        cache: &Path,
        dest: &Path,
        header: &[u8],
        key: &[u8],
    ) -> Result<()> {
        let api = self.api()?;
        let mut res = api
            .get_object(&format!("/files/download/{file_id}"))
            .await?;
        if let Some(d) = cache.parent() {
            tokio::fs::create_dir_all(d).await?;
        }
        let cache_tmp =
            cache.with_extension(format!("tmp{}", hex::encode(crypto::random_bytes(4))));
        let mut cache_out = tokio::fs::File::create(&cache_tmp).await?;
        let mut out = tokio::fs::File::create(dest).await?;
        let mut dec = crypto::StreamDecryptor::new(key, header)?;
        let mut buf: Vec<u8> = Vec::with_capacity(DECRYPTION_CHUNK_SIZE);
        let r: Result<()> = async {
            while let Some(piece) = res.chunk().await.map_err(CoreError::network)? {
                cache_out.write_all(&piece).await?;
                buf.extend_from_slice(&piece);
                while buf.len() > DECRYPTION_CHUNK_SIZE {
                    let rest = buf.split_off(DECRYPTION_CHUNK_SIZE);
                    let (pt, fin) = dec.pull(&buf)?;
                    if fin {
                        return Err(CoreError::crypto("data after the final chunk"));
                    }
                    out.write_all(&pt).await?;
                    buf = rest;
                }
            }
            let (pt, fin) = dec.pull(&buf)?;
            if !fin {
                return Err(CoreError::crypto("file is truncated"));
            }
            out.write_all(&pt).await?;
            out.flush().await?;
            cache_out.flush().await?;
            Ok(())
        }
        .await;
        drop(cache_out);
        match r {
            Ok(()) => {
                tokio::fs::rename(&cache_tmp, cache).await?;
                Ok(())
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&cache_tmp).await;
                Err(e)
            }
        }
    }

    pub async fn stream_playlist(&self, file_id: i64) -> Result<Option<String>> {
        // Ente's streamable videos: crate::video.
        self.stream_playlist_url(file_id).await
    }
}

/// Decrypt a cached ciphertext file into `dest`, chunk by chunk.
async fn decrypt_file_to(src: &Path, dest: &Path, header: &[u8], key: &[u8]) -> Result<()> {
    use tokio::io::AsyncReadExt as _;
    let mut input = tokio::fs::File::open(src).await?;
    let total = input.metadata().await?.len();
    let mut out = tokio::fs::File::create(dest).await?;
    let mut dec = crypto::StreamDecryptor::new(key, header)?;
    let mut read = 0u64;
    let mut buf = vec![0u8; DECRYPTION_CHUNK_SIZE];
    loop {
        let want = (total - read).min(DECRYPTION_CHUNK_SIZE as u64) as usize;
        input.read_exact(&mut buf[..want]).await?;
        read += want as u64;
        let (pt, fin) = dec.pull(&buf[..want])?;
        out.write_all(&pt).await?;
        match (fin, read == total) {
            (true, true) => break,
            (false, false) => continue,
            _ => return Err(CoreError::crypto("cached file is damaged")),
        }
    }
    out.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::client::Inner;

    #[test]
    fn a_failed_entry_holds_the_cursor_back_a_few_times_only() {
        let data = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let inner = Inner::open(data.path().into(), cache.path().into(), None).unwrap();
        // No failure: the cursor moves to where the pass got.
        assert_eq!(inner.cursor_after_failures("c1", 10, 50, None).unwrap(), 50);
        // An entry at 30 failed: stop just before it, for a few passes.
        for _ in 0..super::MAX_DECRYPT_RETRIES {
            assert_eq!(
                inner.cursor_after_failures("c1", 10, 50, Some(30)).unwrap(),
                29
            );
        }
        // Then give up on it rather than re-reading the collection forever.
        assert_eq!(
            inner.cursor_after_failures("c1", 10, 50, Some(30)).unwrap(),
            50
        );
        // Never behind where the pass started.
        assert_eq!(
            inner.cursor_after_failures("c2", 40, 50, Some(30)).unwrap(),
            40
        );
    }
}
