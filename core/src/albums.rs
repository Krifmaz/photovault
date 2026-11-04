//! Albums, sharing (people and public links), hidden albums and trash
//! (contracts/photovault-core.md, "Albums, sharing and trash").
//!
//! Every request and every key operation is the one Ente's apps make
//! (`web/packages/new/photos/services/collection.ts`, `trash.ts`, the mobile
//! `CollectionsService` and `TrashSyncService`):
//!
//! * a new album key is secretbox'd with the master key, its name with the
//!   album key;
//! * adding, moving or restoring files re-encrypts each file key with the
//!   target album's key (secretbox);
//! * sharing seals the album key to the sharee's public key (sealed box,
//!   `GET /users/public-key`);
//! * a public link carries the album key in its fragment, base58 encoded
//!   (`#<base58(key)>`), so the server never sees it; a link password is
//!   Ente's `deriveInteractiveKey` (Argon2id, interactive limits, random salt
//!   sent as `nonce`);
//! * hiding an album sets `visibility = 2` in its private magic metadata,
//!   keeping every other key; hiding files moves them into the default hidden
//!   collection (`subType = 1`), created as the apps create it;
//! * removing the owner's own files from an album moves them to another album
//!   that has them, else to Uncategorized (`removeUserFiles`).
//!
//! Trash entries are sealed kv rows (`trash:<file id>`), and so are the keys
//! of deleted albums (`ckey:<collection id>`), whose files can still be in
//! the trash. Every mutation updates the local store and rebuilds the index
//! before it returns.

use crate::api::Api;
use crate::client::{Inner, Session};
use crate::library::{CollectionRec, FileRec, Index, decrypt_collection, decrypt_file};
use crate::metadata::{SUB_TYPE_DEFAULT_HIDDEN, VISIBILITY_HIDDEN, encrypt_magic};
use crate::model::*;
use crate::{Collection, Core, CoreError, Result, TimelineItem, crypto, run};
use reqwest::Method;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

/// kv key prefix of a sealed trash entry.
const KV_TRASH: &str = "trash:";
/// kv key prefix of a sealed key of a deleted collection.
const KV_DELETED_KEY: &str = "ckey:";
/// kv key of the `/trash/v2/diff` cursor (plain).
const KV_TRASH_SINCE: &str = "trash_since";
/// Name of the default hidden collection, as the apps create it.
const DEFAULT_HIDDEN_NAME: &str = ".Hidden";
/// Files per request (museum accepts up to 1000).
const BATCH: usize = 500;
/// How long the server keeps trashed files (shown until the next trash sync).
const TRASH_DAYS_US: i64 = 30 * 24 * 3600 * 1_000_000;
/// libsodium `crypto_pwhash_OPSLIMIT_INTERACTIVE` / `MEMLIMIT_INTERACTIVE`
/// (Ente's `deriveInteractiveKey`).
const INTERACTIVE_OPS: u64 = 2;
const INTERACTIVE_MEM: u64 = 64 * 1024 * 1024;

// ---- records ----

/// A participant's role (Ente's `CollectionParticipantRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShareRole {
    /// Can view.
    Viewer,
    /// Can view and add photos.
    Collaborator,
    /// Can also manage participants and remove photos.
    Admin,
    /// The album's owner.
    Owner,
    /// Anything else.
    Unknown,
}

impl ShareRole {
    fn parse(s: &str) -> Self {
        match s {
            "VIEWER" => Self::Viewer,
            "COLLABORATOR" => Self::Collaborator,
            "ADMIN" => Self::Admin,
            "OWNER" => Self::Owner,
            _ => Self::Unknown,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "VIEWER",
            Self::Collaborator => "COLLABORATOR",
            Self::Admin => "ADMIN",
            Self::Owner => "OWNER",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Someone an album is shared with.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Sharee {
    /// User id.
    pub user_id: i64,
    /// Email.
    pub email: String,
    /// Role.
    pub role: ShareRole,
}

/// An album's public link.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PublicLink {
    /// The full link, with the album key in its fragment (`#<base58 key>`).
    pub url: String,
    /// Expiry, µs since the epoch; 0 = never.
    pub valid_till_us: i64,
    /// Devices that may open it; 0 = unlimited.
    pub device_limit: u32,
    /// Viewers may download originals.
    pub enable_download: bool,
    /// Viewers may add photos.
    pub enable_collect: bool,
    /// Signed-in viewers may join the album.
    pub enable_join: bool,
    /// The link asks for a password.
    pub password_enabled: bool,
}

/// Changes to a public link: only `Some` fields change.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct PublicLinkUpdate {
    /// New expiry (µs; 0 = never).
    pub valid_till_us: Option<i64>,
    /// New device limit (0 = unlimited).
    pub device_limit: Option<u32>,
    /// Allow downloads.
    pub enable_download: Option<bool>,
    /// Allow adding photos.
    pub enable_collect: Option<bool>,
    /// Allow joining.
    pub enable_join: Option<bool>,
    /// A new password; `Some("")` removes the password.
    pub password: Option<String>,
}

/// Everything the album screens show about an album.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AlbumDetails {
    /// The album as `collections()` lists it.
    pub collection: Collection,
    /// The signed-in user owns it.
    pub owned: bool,
    /// The owner's email (empty for the user's own albums).
    pub owner_email: String,
    /// The user's role (`Owner` for their own albums).
    pub my_role: ShareRole,
    /// Hidden (visibility 2, or the default hidden collection).
    pub hidden: bool,
    /// Archived.
    pub archived: bool,
    /// The default hidden collection (individually hidden files).
    pub is_default_hidden: bool,
    /// People it is shared with (owner's view).
    pub sharees: Vec<Sharee>,
    /// The public link, if any.
    pub public_link: Option<PublicLink>,
}

/// A trashed file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrashEntry {
    /// The file (its collection is the one it was trashed from).
    pub item: TimelineItem,
    /// When the server deletes it for good, µs since the epoch.
    pub delete_by_us: i64,
}

// ---- sealed forms ----

/// A sharee, as sealed with its collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ShareeRec {
    pub id: i64,
    pub email: String,
    pub role: String,
}

/// A public link, as sealed with its collection (the URL includes the key).
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PublicLinkRec {
    pub url: String,
    pub valid_till: i64,
    pub device_limit: i64,
    pub enable_download: bool,
    pub enable_collect: bool,
    pub enable_join: bool,
    pub password_enabled: bool,
}

impl std::fmt::Debug for PublicLinkRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicLinkRec").finish_non_exhaustive()
    }
}

impl PublicLinkRec {
    fn from_remote(u: &RemotePublicUrl, key: &[u8]) -> Self {
        Self {
            url: link_with_key(&u.url, key),
            valid_till: u.valid_till,
            device_limit: u.device_limit,
            enable_download: u.enable_download,
            enable_collect: u.enable_collect,
            enable_join: u.enable_join,
            password_enabled: u.password_enabled,
        }
    }
    fn to_api(&self) -> PublicLink {
        PublicLink {
            url: self.url.clone(),
            valid_till_us: self.valid_till,
            device_limit: self.device_limit.clamp(0, u32::MAX as i64) as u32,
            enable_download: self.enable_download,
            enable_collect: self.enable_collect,
            enable_join: self.enable_join,
            password_enabled: self.password_enabled,
        }
    }
}

/// A trashed file, sealed in kv.
#[derive(Clone, Serialize, Deserialize)]
struct TrashRec {
    file: FileRec,
    collection_id: i64,
    delete_by: i64,
    updated_at: i64,
}

/// Sharees of a remote collection.
pub(crate) fn sharees_of(c: &RemoteCollection) -> Vec<ShareeRec> {
    c.sharees
        .iter()
        .flatten()
        .map(|u| ShareeRec {
            id: u.id,
            email: u.email.clone(),
            role: u.role.clone(),
        })
        .collect()
}

/// The (first) public link of a remote collection, with the key appended.
pub(crate) fn public_link_of(c: &RemoteCollection, key: &[u8]) -> Option<PublicLinkRec> {
    c.public_urls
        .iter()
        .flatten()
        .next()
        .map(|u| PublicLinkRec::from_remote(u, key))
}

/// `url#<base58(key)>`, as the apps build the shareable link.
fn link_with_key(url: &str, key: &[u8]) -> String {
    let base = url.split('#').next().unwrap_or(url);
    format!("{base}#{}", base58(key))
}

/// Base58 with the Bitcoin alphabet (the `bs58` package Ente's apps use).
pub(crate) fn base58(data: &[u8]) -> String {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let zeros = data.iter().take_while(|b| **b == 0).count();
    // Little-endian base-58 digits.
    let mut digits: Vec<u8> = Vec::with_capacity(data.len() * 138 / 100 + 1);
    for &byte in &data[zeros..] {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|&d| ALPHABET[d as usize] as char));
    out
}

// ---- index ----

/// The album part of the in-memory index.
#[derive(Debug, Default)]
pub struct AlbumIndex {
    /// Details of every collection we have, visible or hidden.
    details: HashMap<i64, AlbumDetails>,
    /// Visible albums, in `collections()` order.
    visible: Vec<i64>,
    /// Hidden albums (the default hidden collection first), newest first.
    hidden: Vec<i64>,
    /// Trash, most recently trashed first.
    pub(crate) trash: Vec<TrashEntry>,
}

/// The `Collection` of `c`, as `collections()` would list it, and its sort key.
fn collection_of(c: &CollectionRec, idx: &Index) -> (i64, Collection) {
    let list = idx.by_collection.get(&c.id);
    let newest = list.and_then(|l| l.first()).map(|&p| &idx.all[p as usize]);
    (
        newest.map_or(c.updation_time, |n| n.taken_at_us),
        Collection {
            id: c.id,
            name: c.name.clone(),
            kind: c.kind.clone(),
            cover_file_id: newest.map(|n| n.id),
            count: list.map_or(0, Vec::len) as u64,
            updated_at_us: c.updation_time,
        },
    )
}

fn details_of(c: &CollectionRec, collection: Collection, user_id: i64) -> AlbumDetails {
    let my_role = if c.owned {
        ShareRole::Owner
    } else {
        c.sharees
            .iter()
            .find(|s| s.id == user_id)
            .map_or(ShareRole::Viewer, |s| ShareRole::parse(&s.role))
    };
    AlbumDetails {
        collection,
        owned: c.owned,
        owner_email: if c.owned {
            String::new()
        } else {
            c.owner_email.clone()
        },
        my_role,
        hidden: c.hidden(),
        archived: c.archived(),
        is_default_hidden: c.owned && c.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN),
        sharees: if c.owned {
            c.sharees
                .iter()
                .map(|s| Sharee {
                    user_id: s.id,
                    email: s.email.clone(),
                    role: ShareRole::parse(&s.role),
                })
                .collect()
        } else {
            Vec::new()
        },
        public_link: if c.owned {
            c.public_link.as_ref().map(PublicLinkRec::to_api)
        } else {
            None
        },
    }
}

/// Fill the album part of `idx` (after files and visible collections).
pub(crate) fn build_album_index(idx: &mut Index, cols: &HashMap<i64, CollectionRec>, user_id: i64) {
    let mut a = AlbumIndex::default();
    let visible: HashMap<i64, Collection> =
        idx.collections.iter().map(|c| (c.id, c.clone())).collect();
    a.visible = idx.collections.iter().map(|c| c.id).collect();
    let mut hidden: Vec<(bool, i64, i64)> = Vec::new();
    for c in cols.values() {
        let (sort, collection) = match visible.get(&c.id) {
            Some(v) => (0, v.clone()),
            None => collection_of(c, idx),
        };
        let d = details_of(c, collection, user_id);
        if c.hidden() {
            hidden.push((!d.is_default_hidden, -sort, c.id));
        }
        a.details.insert(c.id, d);
    }
    hidden.sort();
    a.hidden = hidden.into_iter().map(|h| h.2).collect();
    idx.albums = a;
}

// ---- requests ----

#[derive(Serialize)]
struct RenameRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "encryptedName")]
    encrypted_name: String,
    #[serde(rename = "nameDecryptionNonce")]
    name_decryption_nonce: String,
}

#[derive(Serialize)]
struct MoveFilesRequest {
    #[serde(rename = "fromCollectionID")]
    from: i64,
    #[serde(rename = "toCollectionID")]
    to: i64,
    files: Vec<CollectionFileItem>,
}

#[derive(Serialize)]
struct FileIdsRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "fileIDs")]
    file_ids: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicKeyResponse {
    public_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareRequest<'a> {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    email: &'a str,
    encrypted_key: String,
    role: &'static str,
}

#[derive(Serialize)]
struct UnshareRequest<'a> {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    email: &'a str,
}

#[derive(Deserialize)]
struct ShareesResponse {
    #[serde(default)]
    sharees: Vec<CollectionUser>,
}

#[derive(Deserialize)]
struct LinkResponse {
    result: RemotePublicUrl,
}

#[derive(Serialize)]
struct UpdateCollectionMagic {
    id: i64,
    #[serde(rename = "magicMetadata")]
    magic_metadata: RemoteMagicMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteTrashEntry {
    file: RemoteFile,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    is_restored: bool,
    #[serde(default)]
    delete_by: i64,
    #[serde(default)]
    updated_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrashDiff {
    #[serde(default)]
    diff: Vec<RemoteTrashEntry>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Serialize)]
struct TrashItemRequest {
    #[serde(rename = "fileID")]
    file_id: i64,
    #[serde(rename = "collectionID")]
    collection_id: i64,
}

#[derive(Serialize)]
struct TrashRequest {
    items: Vec<TrashItemRequest>,
}

fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as i64)
}

/// A file key re-encrypted for `target`.
fn item_for(f: &FileRec, target: &CollectionRec) -> Result<CollectionFileItem> {
    let (enc_key, nonce) = crypto::secretbox_seal(&f.key, &target.key)?;
    Ok(CollectionFileItem {
        id: f.id,
        encrypted_key: crypto::b64(&enc_key),
        key_decryption_nonce: crypto::b64(&nonce),
    })
}

fn not_found(what: &str, id: i64) -> CoreError {
    CoreError::storage(format!("unknown {what} {id}"))
}

fn not_owner() -> CoreError {
    CoreError::Server {
        status: 403,
        message: "only the album's owner can do this".into(),
    }
}

// ---- implementation ----

impl Inner {
    fn owned_collection(&self, id: i64) -> Result<CollectionRec> {
        let c = self
            .collection_rec(id)?
            .ok_or_else(|| not_found("album", id))?;
        if !c.owned {
            return Err(not_owner());
        }
        Ok(c)
    }

    /// Collections that contain `file_id`, from the store.
    fn memberships(&self, file_id: i64) -> Result<Vec<i64>> {
        let st = self.store();
        let mut q = st
            .conn()
            .prepare("SELECT collection_id FROM collection_files WHERE file_id = ?1")?;
        let ids = q
            .query_map(params![file_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Keep a deleted collection's key (its files may be in the trash).
    pub(crate) fn remember_deleted_key(&self, c: &RemoteCollection, s: &Session) {
        if c.encrypted_key.is_empty() {
            return;
        }
        let key = match self.collection_rec(c.id) {
            Ok(Some(rec)) => Some(rec.key.clone()),
            _ => decrypt_collection(c, s).ok().map(|r| r.key.clone()),
        };
        if let Some(mut k) = key {
            let _ = self
                .store()
                .put_sealed(&format!("{KV_DELETED_KEY}{}", c.id), &k);
            crypto::wipe(&mut k);
        }
    }

    fn collection_key(&self, id: i64) -> Result<Option<Vec<u8>>> {
        if let Some(c) = self.collection_rec(id)? {
            return Ok(Some(c.key.clone()));
        }
        self.store().get_sealed(&format!("{KV_DELETED_KEY}{id}"))
    }

    fn trash_rec(&self, file_id: i64) -> Result<Option<TrashRec>> {
        self.store().get_sealed(&format!("{KV_TRASH}{file_id}"))
    }

    /// A trashed file's record (for its thumbnail).
    pub(crate) fn trashed_file_rec(&self, file_id: i64) -> Result<Option<FileRec>> {
        Ok(self.trash_rec(file_id)?.map(|t| t.file))
    }

    fn all_trash(&self) -> Result<Vec<TrashRec>> {
        let (blobs, opener) = {
            let st = self.store();
            let mut q = st
                .conn()
                .prepare("SELECT v FROM kv WHERE k LIKE 'trash:%'")?;
            let blobs: Vec<Vec<u8>> = q
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            (blobs, st.opener())
        };
        Ok(blobs
            .iter()
            .filter_map(|b| opener.open_sealed::<TrashRec>(b).ok())
            .collect())
    }

    /// The trash, for the index: most recently trashed first.
    pub(crate) fn load_trash(&self, _idx: &Index) -> Result<Vec<TrashEntry>> {
        let mut recs = self.all_trash()?;
        recs.sort_by_key(|t| std::cmp::Reverse((t.updated_at, t.file.id)));
        Ok(recs
            .iter()
            .map(|t| TrashEntry {
                item: t.file.item(t.collection_id),
                delete_by_us: t.delete_by,
            })
            .collect())
    }

    fn put_trash(&self, t: &TrashRec) -> Result<()> {
        self.store()
            .put_sealed(&format!("{KV_TRASH}{}", t.file.id), t)
    }

    fn drop_trash(&self, file_id: i64) -> Result<()> {
        self.store().delete(&format!("{KV_TRASH}{file_id}"))
    }

    // ---- reads ----

    pub fn albums(&self) -> Vec<AlbumDetails> {
        let idx = self.index();
        idx.albums
            .visible
            .iter()
            .filter_map(|id| idx.albums.details.get(id).cloned())
            .collect()
    }

    pub fn hidden_albums(&self) -> Vec<AlbumDetails> {
        let idx = self.index();
        idx.albums
            .hidden
            .iter()
            .filter_map(|id| idx.albums.details.get(id).cloned())
            .collect()
    }

    pub fn album_details(&self, id: i64) -> Option<AlbumDetails> {
        self.index().albums.details.get(&id).cloned()
    }

    pub fn trash_count(&self) -> u64 {
        self.index().albums.trash.len() as u64
    }

    pub fn trash_page(&self, offset: u64, limit: u32) -> Vec<TrashEntry> {
        let idx = self.index();
        let t = &idx.albums.trash;
        let start = offset.min(t.len() as u64) as usize;
        t[start..].iter().take(limit as usize).cloned().collect()
    }

    // ---- albums ----

    async fn create_collection(
        &self,
        api: &Api,
        name: &str,
        magic: Option<Map<String, Value>>,
    ) -> Result<CollectionRec> {
        let s = self.session()?;
        let mut key = crypto::generate_key();
        let (enc_key, key_nonce) = crypto::secretbox_seal(&key, &s.master_key)?;
        let (enc_name, name_nonce) = crypto::secretbox_seal(name.as_bytes(), &key)?;
        let magic_metadata = magic.map(|m| encrypt_magic(&m, 1, &key)).transpose()?;
        crypto::wipe(&mut key);
        let res: CollectionResponse = api
            .json(
                Method::POST,
                "/collections",
                &CreateCollectionRequest {
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&key_nonce),
                    encrypted_name: crypto::b64(&enc_name),
                    name_decryption_nonce: crypto::b64(&name_nonce),
                    kind: "album".into(),
                    magic_metadata,
                },
            )
            .await?;
        let rec = decrypt_collection(&res.collection, &s)?;
        self.save_collection(&rec)?;
        Ok(rec)
    }

    pub async fn create_album(&self, name: &str) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::Server {
                status: 400,
                message: "the album needs a name".into(),
            });
        }
        let api = self.api()?;
        let rec = self.create_collection(&api, name, None).await?;
        self.rebuild_index()?;
        Ok(self
            .album_details(rec.id)
            .map(|d| d.collection)
            .unwrap_or(Collection {
                id: rec.id,
                name: rec.name.clone(),
                kind: rec.kind.clone(),
                cover_file_id: None,
                count: 0,
                updated_at_us: rec.updation_time,
            }))
    }

    pub async fn rename_album(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        let mut c = self.owned_collection(id)?;
        let (enc, nonce) = crypto::secretbox_seal(name.as_bytes(), &c.key)?;
        self.api()?
            .json_no_content(
                Method::POST,
                "/collections/rename",
                &RenameRequest {
                    collection_id: id,
                    encrypted_name: crypto::b64(&enc),
                    name_decryption_nonce: crypto::b64(&nonce),
                },
            )
            .await?;
        c.name = name.to_string();
        self.save_collection(&c)?;
        self.rebuild_index()
    }

    /// Owned, visible albums other than `except` that contain `file_id`.
    fn other_home(&self, file_id: i64, except: i64) -> Result<Option<CollectionRec>> {
        for cid in self.memberships(file_id)? {
            if cid == except {
                continue;
            }
            if let Some(c) = self.collection_rec(cid)?
                && c.owned
                && !c.hidden()
            {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// Move `files` (owned) from `from` to `to`; local store follows.
    async fn move_files(
        &self,
        api: &Api,
        from: i64,
        to: &CollectionRec,
        files: &[FileRec],
    ) -> Result<()> {
        for chunk in files.chunks(BATCH) {
            let items = chunk
                .iter()
                .map(|f| item_for(f, to))
                .collect::<Result<Vec<_>>>()?;
            api.json_no_content(
                Method::POST,
                "/collections/move-files",
                &MoveFilesRequest {
                    from,
                    to: to.id,
                    files: items,
                },
            )
            .await?;
            for f in chunk {
                self.save_file(f, to.id)?;
                self.remove_membership(from, f.id)?;
            }
        }
        Ok(())
    }

    /// Remove files from an album as Ente's apps do: the owner's own files
    /// move to another album that has them, else to Uncategorized; anyone
    /// else's are removed (`/collections/v3/remove-files`).
    async fn remove_files(&self, api: &Api, c: &CollectionRec, file_ids: &[i64]) -> Result<()> {
        let me = self.session()?.account.user_id;
        let mut by_target: HashMap<i64, (CollectionRec, Vec<FileRec>)> = HashMap::new();
        let mut leftover: Vec<FileRec> = Vec::new();
        let mut plain: Vec<i64> = Vec::new();
        for &id in file_ids {
            let f = self.file_rec(id)?;
            if c.owned && f.owner_id == me {
                match self.other_home(id, c.id)? {
                    Some(t) => by_target.entry(t.id).or_insert((t, Vec::new())).1.push(f),
                    None => leftover.push(f),
                }
            } else {
                plain.push(id);
            }
        }
        for (_, (target, files)) in by_target {
            self.move_files(api, c.id, &target, &files).await?;
        }
        if !leftover.is_empty() {
            let unc = self.uncategorized(api).await?;
            if unc.id == c.id {
                // Removing from Uncategorized itself: nowhere to move them.
                return Err(CoreError::Server {
                    status: 400,
                    message: "photos only in Uncategorized can be moved to the trash instead"
                        .into(),
                });
            }
            self.move_files(api, c.id, &unc, &leftover).await?;
        }
        for chunk in plain.chunks(BATCH) {
            api.json_no_content(
                Method::POST,
                "/collections/v3/remove-files",
                &FileIdsRequest {
                    collection_id: c.id,
                    file_ids: chunk.to_vec(),
                },
            )
            .await?;
            for &id in chunk {
                self.remove_membership(c.id, id)?;
            }
        }
        Ok(())
    }

    pub async fn delete_album(&self, id: i64, keep_files: bool) -> Result<()> {
        let c = self.owned_collection(id)?;
        if c.kind == "uncategorized" || c.kind == "favorites" {
            return Err(CoreError::Server {
                status: 400,
                message: format!("the {} album can't be deleted", c.kind),
            });
        }
        let api = self.api()?;
        if keep_files {
            let ids: Vec<i64> = {
                let idx = self.index();
                idx.by_collection
                    .get(&id)
                    .map(|l| l.iter().map(|&p| idx.all[p as usize].id).collect())
                    .unwrap_or_default()
            };
            self.remove_files(&api, &c, &ids).await?;
        }
        api.send_empty(
            Method::DELETE,
            &format!("/collections/v3/{id}"),
            &[
                ("collectionID", id.to_string()),
                (
                    "keepFiles",
                    if keep_files { "True" } else { "False" }.to_string(),
                ),
            ],
        )
        .await?;
        let _ = self
            .store()
            .put_sealed(&format!("{KV_DELETED_KEY}{id}"), &c.key);
        self.delete_collection(id)?;
        self.rebuild_index()
    }

    pub async fn add_to_album(&self, id: i64, file_ids: &[i64]) -> Result<()> {
        let target = self
            .collection_rec(id)?
            .ok_or_else(|| not_found("album", id))?;
        let me = self.session()?.account.user_id;
        let api = self.api()?;
        let files: Vec<FileRec> = file_ids
            .iter()
            .map(|&i| self.file_rec(i))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|f| f.owner_id == me)
            .collect();
        if files.is_empty() && !file_ids.is_empty() {
            return Err(CoreError::Server {
                status: 403,
                message: "only your own photos can be added to an album".into(),
            });
        }
        for chunk in files.chunks(BATCH) {
            let items = chunk
                .iter()
                .map(|f| item_for(f, &target))
                .collect::<Result<Vec<_>>>()?;
            api.json_no_content(
                Method::POST,
                "/collections/add-files",
                &AddFilesRequest {
                    collection_id: id,
                    files: items,
                },
            )
            .await?;
            for f in chunk {
                self.save_file(f, id)?;
            }
        }
        self.rebuild_index()
    }

    pub async fn remove_from_album(&self, id: i64, file_ids: &[i64]) -> Result<()> {
        let c = self
            .collection_rec(id)?
            .ok_or_else(|| not_found("album", id))?;
        let api = self.api()?;
        let r = self.remove_files(&api, &c, file_ids).await;
        self.rebuild_index()?;
        r
    }

    async fn update_magic(&self, c: &mut CollectionRec, set: &[(&str, Value)]) -> Result<()> {
        let mut m = c.magic.clone();
        for (k, v) in set {
            m.insert((*k).to_string(), v.clone());
        }
        let version = c.magic_version.max(1);
        let mm = encrypt_magic(&m, version, &c.key)?;
        self.api()?
            .json_no_content(
                Method::PUT,
                "/collections/magic-metadata",
                &UpdateCollectionMagic {
                    id: c.id,
                    magic_metadata: mm,
                },
            )
            .await?;
        c.sub_type = m.get("subType").and_then(Value::as_i64);
        c.visibility = m.get("visibility").and_then(Value::as_i64);
        c.magic = m;
        c.magic_version = version + 1;
        self.save_collection(c)
    }

    pub async fn set_album_hidden(&self, id: i64, hidden: bool) -> Result<()> {
        let mut c = self.owned_collection(id)?;
        if c.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN) {
            return Err(CoreError::Server {
                status: 400,
                message: "hidden photos stay hidden; unhide the photos instead".into(),
            });
        }
        let v = if hidden { VISIBILITY_HIDDEN } else { 0 };
        self.update_magic(&mut c, &[("visibility", json!(v))])
            .await?;
        self.rebuild_index()
    }

    /// The default hidden collection, created as the apps create it.
    async fn default_hidden(&self, api: &Api) -> Result<CollectionRec> {
        if let Some(c) = self
            .all_collections()?
            .into_iter()
            .find(|c| c.owned && c.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN))
        {
            return Ok(c);
        }
        let mut m = Map::new();
        m.insert("subType".into(), json!(SUB_TYPE_DEFAULT_HIDDEN));
        m.insert("visibility".into(), json!(VISIBILITY_HIDDEN));
        self.create_collection(api, DEFAULT_HIDDEN_NAME, Some(m))
            .await
    }

    pub async fn hide_files(&self, file_ids: &[i64]) -> Result<()> {
        let me = self.session()?.account.user_id;
        let api = self.api()?;
        let hidden = self.default_hidden(&api).await?;
        // Group by every owned source collection the file is in.
        let mut by_source: HashMap<i64, Vec<FileRec>> = HashMap::new();
        for &id in file_ids {
            let f = self.file_rec(id)?;
            if f.owner_id != me {
                continue;
            }
            for cid in self.memberships(id)? {
                if cid == hidden.id {
                    continue;
                }
                if self.collection_rec(cid)?.is_some_and(|c| c.owned) {
                    by_source.entry(cid).or_default().push(f.clone());
                }
            }
        }
        let mut r = Ok(());
        for (from, files) in by_source {
            if let Err(e) = self.move_files(&api, from, &hidden, &files).await {
                r = Err(e);
                break;
            }
        }
        self.rebuild_index()?;
        r
    }

    pub async fn unhide_files(&self, file_ids: &[i64], to: Option<i64>) -> Result<()> {
        let api = self.api()?;
        let Some(hidden) = self
            .all_collections()?
            .into_iter()
            .find(|c| c.owned && c.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN))
        else {
            return Ok(());
        };
        let target = match to {
            Some(id) => self
                .collection_rec(id)?
                .ok_or_else(|| not_found("album", id))?,
            None => self.uncategorized(&api).await?,
        };
        let files = file_ids
            .iter()
            .map(|&i| self.file_rec(i))
            .collect::<Result<Vec<_>>>()?;
        let r = self.move_files(&api, hidden.id, &target, &files).await;
        self.rebuild_index()?;
        r
    }

    // ---- sharing ----

    fn set_sharees(&self, id: i64, users: Vec<CollectionUser>) -> Result<Vec<Sharee>> {
        let sharees: Vec<ShareeRec> = users
            .into_iter()
            .map(|u| ShareeRec {
                id: u.id,
                email: u.email,
                role: u.role,
            })
            .collect();
        if let Some(mut c) = self.collection_rec(id)? {
            c.sharees = sharees.clone();
            self.save_collection(&c)?;
        }
        self.rebuild_index()?;
        Ok(sharees
            .into_iter()
            .map(|s| Sharee {
                user_id: s.id,
                role: ShareRole::parse(&s.role),
                email: s.email,
            })
            .collect())
    }

    pub async fn share_album(&self, id: i64, email: &str, role: ShareRole) -> Result<Vec<Sharee>> {
        let email = email.trim();
        if !matches!(
            role,
            ShareRole::Viewer | ShareRole::Collaborator | ShareRole::Admin
        ) {
            return Err(CoreError::Server {
                status: 400,
                message: "share as viewer, collaborator or admin".into(),
            });
        }
        let c = self
            .collection_rec(id)?
            .ok_or_else(|| not_found("album", id))?;
        let api = self.api()?;
        let pk: PublicKeyResponse = api
            .get_json("/users/public-key", &[("email", email.to_string())])
            .await
            .map_err(|e| match e {
                CoreError::Server { status: 404, .. } => CoreError::Server {
                    status: 404,
                    message: format!("{email} has no account on this server"),
                },
                e => e,
            })?;
        let sealed = crypto::box_seal(&c.key, &crypto::unb64(&pk.public_key)?)?;
        let res: ShareesResponse = api
            .json(
                Method::POST,
                "/collections/share",
                &ShareRequest {
                    collection_id: id,
                    email,
                    encrypted_key: crypto::b64(&sealed),
                    role: role.as_str(),
                },
            )
            .await?;
        self.set_sharees(id, res.sharees)
    }

    pub async fn unshare_album(&self, id: i64, email: &str) -> Result<Vec<Sharee>> {
        let res: ShareesResponse = self
            .api()?
            .json(
                Method::POST,
                "/collections/unshare",
                &UnshareRequest {
                    collection_id: id,
                    email: email.trim(),
                },
            )
            .await?;
        self.set_sharees(id, res.sharees)
    }

    pub async fn leave_album(&self, id: i64) -> Result<()> {
        self.api()?
            .send_empty(Method::POST, &format!("/collections/leave/{id}"), &[])
            .await?;
        self.delete_collection(id)?;
        self.rebuild_index()
    }

    fn set_link(&self, c: &mut CollectionRec, link: Option<PublicLinkRec>) -> Result<()> {
        c.public_link = link;
        self.save_collection(c)?;
        self.rebuild_index()
    }

    pub async fn create_public_link(&self, id: i64) -> Result<PublicLink> {
        let mut c = self.owned_collection(id)?;
        let res: LinkResponse = self
            .api()?
            .json(
                Method::POST,
                "/collections/share-url",
                &json!({ "collectionID": id }),
            )
            .await?;
        let link = PublicLinkRec::from_remote(&res.result, &c.key);
        let out = link.to_api();
        self.set_link(&mut c, Some(link))?;
        Ok(out)
    }

    pub async fn update_public_link(&self, id: i64, u: PublicLinkUpdate) -> Result<PublicLink> {
        let mut c = self.owned_collection(id)?;
        let mut body = Map::new();
        body.insert("collectionID".into(), json!(id));
        if let Some(v) = u.valid_till_us {
            body.insert("validTill".into(), json!(v.max(0)));
        }
        if let Some(v) = u.device_limit {
            body.insert("deviceLimit".into(), json!(v));
        }
        if let Some(v) = u.enable_download {
            body.insert("enableDownload".into(), json!(v));
        }
        if let Some(v) = u.enable_collect {
            body.insert("enableCollect".into(), json!(v));
        }
        if let Some(v) = u.enable_join {
            body.insert("enableJoin".into(), json!(v));
        }
        match u.password.as_deref() {
            Some("") => {
                body.insert("disablePassword".into(), json!(true));
            }
            Some(p) => {
                let salt = crypto::random_bytes(16);
                let pw = p.as_bytes().to_vec();
                let salt2 = salt.clone();
                let mut hash = tokio::task::spawn_blocking(move || {
                    crypto::derive_argon_key(&pw, &salt2, INTERACTIVE_OPS, INTERACTIVE_MEM)
                })
                .await
                .map_err(|e| CoreError::crypto(e.to_string()))??;
                body.insert("passHash".into(), json!(crypto::b64(&hash)));
                body.insert("nonce".into(), json!(crypto::b64(&salt)));
                body.insert("memLimit".into(), json!(INTERACTIVE_MEM));
                body.insert("opsLimit".into(), json!(INTERACTIVE_OPS));
                crypto::wipe(&mut hash);
            }
            None => {}
        }
        let res: LinkResponse = self
            .api()?
            .json(Method::PUT, "/collections/share-url", &Value::Object(body))
            .await?;
        let link = PublicLinkRec::from_remote(&res.result, &c.key);
        let out = link.to_api();
        self.set_link(&mut c, Some(link))?;
        Ok(out)
    }

    pub async fn delete_public_link(&self, id: i64) -> Result<()> {
        let mut c = self.owned_collection(id)?;
        self.api()?
            .send_empty(Method::DELETE, &format!("/collections/share-url/{id}"), &[])
            .await?;
        self.set_link(&mut c, None)
    }

    // ---- trash ----

    pub async fn sync_trash(&self) -> Result<u64> {
        let api = self.api()?;
        let mut since: i64 = self
            .store()
            .get_plain(KV_TRASH_SINCE)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut keys: HashMap<i64, Option<Vec<u8>>> = HashMap::new();
        loop {
            let page: TrashDiff = api
                .get_json("/trash/v2/diff", &[("sinceTime", since.to_string())])
                .await?;
            for e in &page.diff {
                since = since.max(e.updated_at);
                let fid = e.file.id;
                if e.is_deleted || e.is_restored {
                    self.drop_trash(fid)?;
                    continue;
                }
                let cid = e.file.collection_id;
                let key = match keys.entry(cid) {
                    std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                    std::collections::hash_map::Entry::Vacant(v) => {
                        v.insert(self.collection_key(cid)?)
                    }
                };
                let Some(key) = key.as_deref() else {
                    continue;
                };
                // An entry we cannot open is skipped (it stays on the server).
                if let Ok(file) = decrypt_file(&e.file, key) {
                    self.put_trash(&TrashRec {
                        file,
                        collection_id: cid,
                        delete_by: e.delete_by,
                        updated_at: e.updated_at,
                    })?;
                }
            }
            self.store().put_plain(KV_TRASH_SINCE, &since.to_string())?;
            if !page.has_more || page.diff.is_empty() {
                break;
            }
        }
        for k in keys.values_mut().flatten() {
            crypto::wipe(k);
        }
        self.rebuild_index()?;
        Ok(self.trash_count())
    }

    pub async fn move_to_trash(&self, file_ids: &[i64]) -> Result<()> {
        let me = self.session()?.account.user_id;
        let api = self.api()?;
        let mut items = Vec::new();
        let mut recs: Vec<(FileRec, Vec<i64>, i64)> = Vec::new();
        for &id in file_ids {
            // Already gone (trashed or deleted on another device): nothing to do.
            let Ok(f) = self.file_rec(id) else {
                continue;
            };
            if f.owner_id != me {
                continue;
            }
            let cols = self.memberships(id)?;
            let mut first_owned = None;
            for &cid in &cols {
                if self.collection_rec(cid)?.is_some_and(|c| c.owned) {
                    items.push(TrashItemRequest {
                        file_id: id,
                        collection_id: cid,
                    });
                    first_owned.get_or_insert(cid);
                }
            }
            if let Some(cid) = first_owned {
                recs.push((f, cols, cid));
            }
        }
        if recs.is_empty() {
            return if file_ids.is_empty() {
                Ok(())
            } else {
                Err(CoreError::Server {
                    status: 403,
                    message: "only your own photos can be moved to the trash".into(),
                })
            };
        }
        // Keep each file's items in one request (the server trashes per file).
        let mut batch: Vec<TrashItemRequest> = Vec::new();
        let mut last = None;
        for it in items {
            if batch.len() >= BATCH && last != Some(it.file_id) {
                api.json_no_content(
                    Method::POST,
                    "/files/trash",
                    &TrashRequest {
                        items: std::mem::take(&mut batch),
                    },
                )
                .await?;
            }
            last = Some(it.file_id);
            batch.push(it);
        }
        if !batch.is_empty() {
            api.json_no_content(Method::POST, "/files/trash", &TrashRequest { items: batch })
                .await?;
        }
        let now = now_us();
        for (f, cols, cid) in recs {
            self.put_trash(&TrashRec {
                file: f.clone(),
                collection_id: cid,
                delete_by: now + TRASH_DAYS_US,
                updated_at: now,
            })?;
            for c in cols {
                self.remove_membership(c, f.id)?;
            }
        }
        self.rebuild_index()
    }

    pub async fn restore_from_trash(&self, file_ids: &[i64], to: Option<i64>) -> Result<()> {
        let api = self.api()?;
        let target = match to {
            Some(id) => self.owned_collection(id)?,
            None => self.uncategorized(&api).await?,
        };
        let recs: Vec<TrashRec> = file_ids
            .iter()
            .map(|&i| {
                self.trash_rec(i)?
                    .ok_or_else(|| not_found("trashed file", i))
            })
            .collect::<Result<Vec<_>>>()?;
        for chunk in recs.chunks(BATCH) {
            let items = chunk
                .iter()
                .map(|t| item_for(&t.file, &target))
                .collect::<Result<Vec<_>>>()?;
            api.json_no_content(
                Method::POST,
                "/collections/restore-files",
                &AddFilesRequest {
                    collection_id: target.id,
                    files: items,
                },
            )
            .await?;
            for t in chunk {
                self.save_file(&t.file, target.id)?;
                self.drop_trash(t.file.id)?;
            }
        }
        self.rebuild_index()
    }

    pub async fn delete_from_trash(&self, file_ids: &[i64]) -> Result<()> {
        let api = self.api()?;
        let unique: Vec<i64> = file_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for chunk in unique.chunks(BATCH) {
            api.json_no_content(Method::POST, "/trash/delete", &json!({ "fileIDs": chunk }))
                .await?;
            for &id in chunk {
                self.drop_trash(id)?;
            }
        }
        self.rebuild_index()
    }

    pub async fn empty_trash(&self) -> Result<()> {
        let recs = self.all_trash()?;
        let Some(last) = recs.iter().map(|t| t.updated_at).max() else {
            return Ok(());
        };
        self.api()?
            .json_no_content(
                Method::POST,
                "/trash/empty",
                &json!({ "lastUpdatedAt": last.max(1) }),
            )
            .await?;
        for t in &recs {
            self.drop_trash(t.file.id)?;
        }
        self.rebuild_index()
    }
}

// ---- export ----

#[uniffi::export]
impl Core {
    /// Visible albums with sharing details, in `collections()` order.
    pub fn albums(&self) -> Vec<AlbumDetails> {
        self.inner.albums()
    }

    /// Hidden albums, the default hidden collection (hidden photos) first.
    pub fn hidden_albums(&self) -> Vec<AlbumDetails> {
        self.inner.hidden_albums()
    }

    /// One album (visible or hidden).
    pub fn album_details(&self, collection_id: i64) -> Option<AlbumDetails> {
        self.inner.album_details(collection_id)
    }

    /// Create an album.
    pub async fn create_album(&self, name: String) -> Result<Collection, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.create_album(&name).await }).await
    }

    /// Rename one of the user's albums.
    pub async fn rename_album(&self, collection_id: i64, name: String) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.rename_album(collection_id, &name).await }).await
    }

    /// Delete one of the user's albums. `keep_files`: photos only in it move
    /// to Uncategorized; otherwise the server moves them to the trash.
    pub async fn delete_album(
        &self,
        collection_id: i64,
        keep_files: bool,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.delete_album(collection_id, keep_files).await }).await
    }

    /// Add the user's own files to an album.
    pub async fn add_to_album(
        &self,
        collection_id: i64,
        file_ids: Vec<i64>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.add_to_album(collection_id, &file_ids).await }).await
    }

    /// Remove files from an album (the owner's own files move to another
    /// album that has them, else to Uncategorized).
    pub async fn remove_from_album(
        &self,
        collection_id: i64,
        file_ids: Vec<i64>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.remove_from_album(collection_id, &file_ids).await }).await
    }

    /// Hide or unhide one of the user's albums.
    pub async fn set_album_hidden(
        &self,
        collection_id: i64,
        hidden: bool,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.set_album_hidden(collection_id, hidden).await }).await
    }

    /// Hide files (move them to the default hidden collection).
    pub async fn hide_files(&self, file_ids: Vec<i64>) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.hide_files(&file_ids).await }).await
    }

    /// Unhide files into `collection_id`, or Uncategorized.
    pub async fn unhide_files(
        &self,
        file_ids: Vec<i64>,
        collection_id: Option<i64>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.unhide_files(&file_ids, collection_id).await }).await
    }

    /// Share an album with someone on the same server.
    pub async fn share_album(
        &self,
        collection_id: i64,
        email: String,
        role: ShareRole,
    ) -> Result<Vec<Sharee>, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.share_album(collection_id, &email, role).await }).await
    }

    /// Stop sharing an album with someone.
    pub async fn unshare_album(
        &self,
        collection_id: i64,
        email: String,
    ) -> Result<Vec<Sharee>, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.unshare_album(collection_id, &email).await }).await
    }

    /// Leave an album shared with the user.
    pub async fn leave_album(&self, collection_id: i64) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.leave_album(collection_id).await }).await
    }

    /// Create the album's public link.
    pub async fn create_public_link(&self, collection_id: i64) -> Result<PublicLink, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.create_public_link(collection_id).await }).await
    }

    /// Change the album's public link.
    pub async fn update_public_link(
        &self,
        collection_id: i64,
        update: PublicLinkUpdate,
    ) -> Result<PublicLink, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.update_public_link(collection_id, update).await }).await
    }

    /// Remove the album's public link.
    pub async fn delete_public_link(&self, collection_id: i64) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.delete_public_link(collection_id).await }).await
    }

    /// Pull the trash; returns the number of trashed files.
    pub async fn sync_trash(&self) -> Result<u64, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.sync_trash().await }).await
    }

    /// Number of trashed files (local).
    pub fn trash_count(&self) -> u64 {
        self.inner.trash_count()
    }

    /// A page of the trash, most recently trashed first (local).
    pub fn trash_page(&self, offset: u64, limit: u32) -> Vec<TrashEntry> {
        self.inner.trash_page(offset, limit)
    }

    /// Move the user's own files to the trash.
    pub async fn move_to_trash(&self, file_ids: Vec<i64>) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.move_to_trash(&file_ids).await }).await
    }

    /// Restore trashed files into `collection_id`, or Uncategorized.
    pub async fn restore_from_trash(
        &self,
        file_ids: Vec<i64>,
        collection_id: Option<i64>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.restore_from_trash(&file_ids, collection_id).await }).await
    }

    /// Delete trashed files for good.
    pub async fn delete_from_trash(&self, file_ids: Vec<i64>) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.delete_from_trash(&file_ids).await }).await
    }

    /// Delete everything in the trash for good.
    pub async fn empty_trash(&self) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.empty_trash().await }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base58_matches_bitcoin_vectors() {
        assert_eq!(base58(b""), "");
        assert_eq!(base58(&[0]), "1");
        assert_eq!(base58(&[0, 0, 1]), "112");
        assert_eq!(base58(b"hello world"), "StV1DL6CwTryKyV");
        assert_eq!(
            base58(&hex::decode("00eb15231dfceb60925886b67d065299925915aeb172c06647").unwrap()),
            "1NS17iag9jJgTHD1VXjvLCEnZuQ3rJDE9L"
        );
    }

    #[test]
    fn link_gets_the_key_fragment_once() {
        let k = [1u8, 2, 3];
        assert_eq!(link_with_key("http://h/?t=abc", &k), "http://h/?t=abc#Ldp");
        assert_eq!(
            link_with_key("http://h/?t=abc#old", &k),
            "http://h/?t=abc#Ldp"
        );
    }

    #[test]
    fn roles_round_trip() {
        for r in [
            ShareRole::Viewer,
            ShareRole::Collaborator,
            ShareRole::Admin,
            ShareRole::Owner,
        ] {
            assert_eq!(ShareRole::parse(r.as_str()), r);
        }
        assert_eq!(ShareRole::parse("x"), ShareRole::Unknown);
    }
}
