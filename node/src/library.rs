//! The user's library as seen by the node: collections, files and their keys.
//!
//! Ente's key hierarchy:
//!
//! - **Collection key**: for collections the user owns, secretbox-encrypted
//!   with the master key (`encryptedKey` + `keyDecryptionNonce`); for
//!   collections shared with the user, a sealed box to the user's public key.
//! - **File key**: secretbox-encrypted with the key of the collection the
//!   diff entry came from. The same file in several collections has the same
//!   file key, wrapped once per collection.
//! - **Metadata**: a blob (secretstream, one `FINAL` message) of JSON,
//!   encrypted with the file key; so is public magic metadata.
//! - **Original / thumbnail**: secretstream with the file key and the
//!   `file.decryptionHeader` / `thumbnail.decryptionHeader`.
//!
//! [`SyncState`] keeps only what the server sent (ciphertext), so it is safe
//! to persist; plaintext keys are re-derived from the [`Session`] on demand.
//!
//! OWNER: crypto/api agent.

use crate::api::models::{RemoteCollection, RemoteFile};
use crate::api::{ApiClient, ApiError, Result, Session};
use crate::crypto::{self, ChunkedDecryptor, SecretKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

/// Ente's file types (`fileType` in file metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileType {
    /// 0: a still image.
    Image,
    /// 1: a video.
    Video,
    /// 2: a Live Photo (zip of image + video).
    LivePhoto,
    /// 3 or unknown.
    Other(i64),
}

impl FileType {
    /// From Ente's numeric `fileType`.
    pub fn from_code(code: i64) -> Self {
        match code {
            0 => Self::Image,
            1 => Self::Video,
            2 => Self::LivePhoto,
            n => Self::Other(n),
        }
    }

    /// Ente's numeric `fileType`.
    pub fn code(self) -> i64 {
        match self {
            Self::Image => 0,
            Self::Video => 1,
            Self::LivePhoto => 2,
            Self::Other(n) => n,
        }
    }
}

/// Decrypted file metadata. Known fields are typed; everything is kept in `raw`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    /// Original file name.
    #[serde(default)]
    pub title: String,
    /// Ente `fileType` code (see [`FileType`]).
    #[serde(default)]
    pub file_type: i64,
    /// Capture time, microseconds since epoch.
    #[serde(default)]
    pub creation_time: i64,
    /// Modification time, microseconds since epoch.
    #[serde(default)]
    pub modification_time: Option<i64>,
    /// Latitude, if known.
    #[serde(default)]
    pub latitude: Option<f64>,
    /// Longitude, if known.
    #[serde(default)]
    pub longitude: Option<f64>,
    /// Video duration in seconds, if known.
    #[serde(default)]
    pub duration: Option<f64>,
    /// Content hash (base64 BLAKE2b), if known.
    #[serde(default)]
    pub hash: Option<String>,
    /// The whole decrypted JSON object.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

impl FileMetadata {
    /// Parse decrypted metadata JSON.
    pub fn from_json(bytes: &[u8]) -> std::result::Result<Self, serde_json::Error> {
        let raw: serde_json::Value = serde_json::from_slice(bytes)?;
        let mut m: Self = serde_json::from_value(raw.clone())?;
        m.raw = raw;
        Ok(m)
    }
}

/// A decrypted collection.
#[derive(Debug, Clone)]
pub struct Collection {
    /// Collection id.
    pub id: i64,
    /// Owner's user id.
    pub owner_id: i64,
    /// Whether the signed-in user owns it (false: shared with them).
    pub owned: bool,
    /// `album`, `folder`, `favorites`, `uncategorized`.
    pub kind: String,
    /// Decrypted name (`None` if it could not be decrypted).
    pub name: Option<String>,
    /// The collection key.
    pub key: SecretKey,
    /// Last change, microseconds.
    pub updation_time: i64,
}

/// A decrypted, live file.
#[derive(Debug, Clone)]
pub struct EnteFile {
    /// File id.
    pub id: i64,
    /// Owner's user id.
    pub owner_id: i64,
    /// The collection the key was taken from (lowest owned id, else lowest id).
    pub collection_id: i64,
    /// All live collections containing the file.
    pub collection_ids: Vec<i64>,
    /// The file key: encrypts the original, thumbnail, metadata and file data.
    pub key: SecretKey,
    /// File type from metadata.
    pub file_type: FileType,
    /// Decrypted metadata.
    pub metadata: FileMetadata,
    /// Decrypted public magic metadata (`w`, `h`, `sv`, edited name, ...), if any.
    pub pub_magic_metadata: Option<serde_json::Value>,
    /// Secretstream header of the original.
    pub file_decryption_header: Vec<u8>,
    /// Secretstream header of the thumbnail.
    pub thumbnail_decryption_header: Vec<u8>,
    /// Encrypted original size, if the server reported it.
    pub file_size: Option<i64>,
    /// Encrypted thumbnail size, if the server reported it.
    pub thumbnail_size: Option<i64>,
    /// Last change across its collections, microseconds.
    pub updation_time: i64,
}

/// Persistent sync cursor and ciphertext cache. Safe to store: it holds only
/// what the server sent (no plaintext keys or metadata).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    /// `sinceTime` for `GET /collections/v2`.
    pub collections_since: i64,
    /// Live collections by id.
    pub collections: BTreeMap<i64, RemoteCollection>,
    /// `sinceTime` per collection for `GET /collections/v2/diff`.
    pub collection_since: BTreeMap<i64, i64>,
    /// Live diff entries: file id → collection id → entry.
    pub files: BTreeMap<i64, BTreeMap<i64, RemoteFile>>,
}

/// What one [`sync`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Files that became live or changed (deduplicated, sorted).
    pub updated: Vec<i64>,
    /// Files that are no longer live in any collection (sorted).
    pub removed: Vec<i64>,
}

/// Bring `state` up to date with the server.
///
/// Fetches changed collections, then each collection's file diff from its
/// last `sinceTime` until `hasMore` is false. Deleted collections drop their
/// files; deleted entries (removed, trashed or permanently deleted) are
/// dropped. A file stays live while any collection still holds it.
pub async fn sync(client: &ApiClient, state: &mut SyncState) -> Result<SyncReport> {
    let mut touched: BTreeSet<i64> = BTreeSet::new();
    let mut was_live: BTreeSet<i64> = BTreeSet::new();

    let changed = client.get_collections(state.collections_since).await?;
    for c in changed {
        state.collections_since = state.collections_since.max(c.updation_time);
        if c.is_deleted {
            state.collections.remove(&c.id);
            state.collection_since.remove(&c.id);
            drop_collection_files(state, c.id, &mut touched, &mut was_live);
        } else {
            state.collections.insert(c.id, c);
        }
    }

    let ids: Vec<i64> = state.collections.keys().copied().collect();
    for cid in ids {
        let mut since = state.collection_since.get(&cid).copied().unwrap_or(0);
        loop {
            let page = client.get_collection_diff(cid, since).await?;
            let max = page
                .diff
                .iter()
                .map(|f| f.updation_time)
                .max()
                .unwrap_or(since);
            for f in page.diff {
                touched.insert(f.id);
                if state.files.contains_key(&f.id) {
                    was_live.insert(f.id);
                }
                if f.is_removed() {
                    if let Some(per) = state.files.get_mut(&f.id) {
                        per.remove(&cid);
                        if per.is_empty() {
                            state.files.remove(&f.id);
                        }
                    }
                } else {
                    state.files.entry(f.id).or_default().insert(cid, f);
                }
            }
            if page.has_more && max <= since {
                return Err(ApiError::Protocol(format!(
                    "collection {cid} diff did not advance past {since}"
                )));
            }
            since = max;
            state.collection_since.insert(cid, since);
            if !page.has_more {
                break;
            }
        }
    }

    let mut report = SyncReport::default();
    for id in touched {
        if state.files.contains_key(&id) {
            report.updated.push(id);
        } else if was_live.contains(&id) {
            report.removed.push(id);
        }
    }
    Ok(report)
}

fn drop_collection_files(
    state: &mut SyncState,
    cid: i64,
    touched: &mut BTreeSet<i64>,
    was_live: &mut BTreeSet<i64>,
) {
    state.files.retain(|id, per| {
        if per.remove(&cid).is_some() {
            touched.insert(*id);
            was_live.insert(*id);
        }
        !per.is_empty()
    });
}

/// Open a collection's key with the session's keys.
pub fn collection_key(session: &Session, c: &RemoteCollection) -> Result<SecretKey> {
    let enc = crypto::b64_decode(&c.encrypted_key)?;
    let bytes = if c.owner.id == session.user_id {
        let nonce = c.key_decryption_nonce.as_deref().ok_or_else(|| {
            ApiError::Protocol(format!("owned collection {} has no key nonce", c.id))
        })?;
        Zeroizing::new(crypto::secretbox_open(
            &enc,
            &crypto::b64_decode(nonce)?,
            &session.master_key,
        )?)
    } else {
        let pk = session.public_key_bytes()?;
        Zeroizing::new(crypto::sealed_box_open(&enc, &pk, &session.secret_key)?)
    };
    Ok(SecretKey::from_slice(&bytes)?)
}

/// Decrypt a collection (key and name).
pub fn decrypt_collection(session: &Session, c: &RemoteCollection) -> Result<Collection> {
    let key = collection_key(session, c)?;
    let name = match (&c.encrypted_name, &c.name_decryption_nonce) {
        (Some(n), Some(nonce)) if !n.is_empty() => {
            crypto::secretbox_open(&crypto::b64_decode(n)?, &crypto::b64_decode(nonce)?, &key)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
        }
        _ => c.name.clone().filter(|n| !n.is_empty()),
    };
    Ok(Collection {
        id: c.id,
        owner_id: c.owner.id,
        owned: c.owner.id == session.user_id,
        kind: c.kind.clone(),
        name,
        key,
        updation_time: c.updation_time,
    })
}

/// Decrypt one diff entry with its collection's key.
pub fn decrypt_file(collection_key: &SecretKey, f: &RemoteFile) -> Result<EnteFile> {
    let key =
        crypto::secretbox_open_key_b64(&f.encrypted_key, &f.key_decryption_nonce, collection_key)?;
    let md_ct = f
        .metadata
        .encrypted_data
        .as_deref()
        .ok_or_else(|| ApiError::Protocol(format!("file {} has no metadata", f.id)))?;
    let md_plain = crypto::decrypt_blob(
        &crypto::b64_decode(md_ct)?,
        &crypto::b64_decode(&f.metadata.decryption_header)?,
        &key,
    )?;
    let metadata = FileMetadata::from_json(&md_plain).map_err(|e| ApiError::Decode {
        path: format!("file {} metadata", f.id),
        detail: e.to_string(),
    })?;
    let pub_magic_metadata = match &f.pub_magic_metadata {
        Some(m) if !m.data.is_empty() => {
            let plain = crypto::decrypt_blob(
                &crypto::b64_decode(&m.data)?,
                &crypto::b64_decode(&m.header)?,
                &key,
            )?;
            serde_json::from_slice(&plain).ok()
        }
        _ => None,
    };
    Ok(EnteFile {
        id: f.id,
        owner_id: f.owner_id,
        collection_id: f.collection_id,
        collection_ids: vec![f.collection_id],
        key,
        file_type: FileType::from_code(metadata.file_type),
        metadata,
        pub_magic_metadata,
        file_decryption_header: crypto::b64_decode(&f.file.decryption_header)?,
        thumbnail_decryption_header: crypto::b64_decode(&f.thumbnail.decryption_header)?,
        file_size: f
            .info
            .map(|i| i.file_size)
            .filter(|s| *s > 0)
            .or(f.file.size),
        thumbnail_size: f
            .info
            .map(|i| i.thumb_size)
            .filter(|s| *s > 0)
            .or(f.thumbnail.size),
        updation_time: f.updation_time,
    })
}

/// A decrypted view of a [`SyncState`].
pub struct Library<'a> {
    session: &'a Session,
    state: &'a SyncState,
    keys: BTreeMap<i64, SecretKey>,
}

impl std::fmt::Debug for Library<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("collections", &self.state.collections.len())
            .field("files", &self.state.files.len())
            .finish()
    }
}

impl<'a> Library<'a> {
    /// Open all collection keys. Collections whose key cannot be opened are
    /// skipped (with a warning) rather than failing the whole library.
    pub fn open(session: &'a Session, state: &'a SyncState) -> Self {
        let mut keys = BTreeMap::new();
        for (id, c) in &state.collections {
            match collection_key(session, c) {
                Ok(k) => {
                    keys.insert(*id, k);
                }
                Err(e) => tracing::warn!(collection = id, error = %e, "cannot open collection key"),
            }
        }
        Self {
            session,
            state,
            keys,
        }
    }

    /// All decryptable live collections.
    pub fn collections(&self) -> Vec<Collection> {
        self.state
            .collections
            .values()
            .filter(|c| self.keys.contains_key(&c.id))
            .filter_map(|c| decrypt_collection(self.session, c).ok())
            .collect()
    }

    /// Number of live files (by id).
    pub fn len(&self) -> usize {
        self.state.files.len()
    }

    /// Whether there are no live files.
    pub fn is_empty(&self) -> bool {
        self.state.files.is_empty()
    }

    /// Decrypt one live file by id.
    pub fn file(&self, id: i64) -> Result<Option<EnteFile>> {
        let Some(per) = self.state.files.get(&id) else {
            return Ok(None);
        };
        // Prefer an owned collection's entry, then the lowest collection id.
        let mut order: Vec<&i64> = per.keys().collect();
        order.sort_by_key(|cid| {
            let owned = self
                .state
                .collections
                .get(cid)
                .is_some_and(|c| c.owner.id == self.session.user_id);
            (!owned, **cid)
        });
        let mut last_err = None;
        for cid in order {
            let Some(key) = self.keys.get(cid) else {
                continue;
            };
            match decrypt_file(key, &per[cid]) {
                Ok(mut f) => {
                    f.collection_id = *cid;
                    f.collection_ids = per.keys().copied().collect();
                    f.updation_time = per.values().map(|e| e.updation_time).max().unwrap_or(0);
                    return Ok(Some(f));
                }
                Err(e) => last_err = Some(e),
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }

    /// Decrypt every live file, deduplicated by id. Files that fail to
    /// decrypt are logged and skipped.
    pub fn files(&self) -> Vec<EnteFile> {
        self.state
            .files
            .keys()
            .filter_map(|id| match self.file(*id) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(file = id, error = %e, "cannot decrypt file");
                    None
                }
            })
            .collect()
    }

    /// Live files owned by the signed-in user (the ones Ente's apps index).
    pub fn owned_files(&self) -> Vec<EnteFile> {
        let me = self.session.user_id;
        self.files()
            .into_iter()
            .filter(|f| f.owner_id == me)
            .collect()
    }
}

/// Download and decrypt a file's original into memory.
pub async fn download_original(client: &ApiClient, file: &EnteFile) -> Result<Vec<u8>> {
    let dl = client.download_file(file.id).await?;
    decrypt_download(dl, &file.file_decryption_header, &file.key).await
}

/// Download and decrypt a file's thumbnail (JPEG) into memory.
pub async fn download_thumbnail(client: &ApiClient, file: &EnteFile) -> Result<Vec<u8>> {
    let dl = client.download_thumbnail(file.id).await?;
    decrypt_download(dl, &file.thumbnail_decryption_header, &file.key).await
}

/// Download and decrypt a file's original into `path`, streaming in 4 MiB
/// chunks. The file is created (truncated) with mode 0600 on Unix; on
/// failure the partial file is removed.
pub async fn download_original_to(
    client: &ApiClient,
    file: &EnteFile,
    path: &std::path::Path,
) -> Result<u64> {
    use tokio::io::AsyncWriteExt;
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut out = opts.open(path).await?;
    let result = async {
        let mut dl = client.download_file(file.id).await?;
        let mut dec = ChunkedDecryptor::new(&file.file_decryption_header, &file.key)?;
        let mut written = 0u64;
        while let Some(chunk) = dl.chunk().await? {
            let pt = Zeroizing::new(dec.update(&chunk)?);
            out.write_all(&pt).await?;
            written += pt.len() as u64;
        }
        let pt = Zeroizing::new(dec.finish()?);
        out.write_all(&pt).await?;
        out.flush().await?;
        Ok::<u64, ApiError>(written + pt.len() as u64)
    }
    .await;
    if result.is_err() {
        drop(out);
        let _ = tokio::fs::remove_file(path).await;
    }
    result
}

async fn decrypt_download(
    mut dl: crate::api::Download,
    header: &[u8],
    key: &SecretKey,
) -> Result<Vec<u8>> {
    let mut dec = ChunkedDecryptor::new(header, key)?;
    let mut out = Vec::with_capacity(dl.content_length().unwrap_or(0).min(64 << 20) as usize);
    while let Some(chunk) = dl.chunk().await? {
        out.extend(dec.update(&chunk)?);
    }
    out.extend(dec.finish()?);
    Ok(out)
}
