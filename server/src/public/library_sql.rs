//! Every query this workstream makes against tables owned by the files/collections
//! workstream (`files`, `object_keys`, `collections`, `collection_files`,
//! `collection_shares`, `public_collection_tokens`, `public_file_tokens`,
//! `file_data`, `usage`). Kept in one module so it can be swapped for the
//! library helpers. Table and column names are museum's.

use super::models::{
    Collection, CollectionAttributes, CollectionUser, File, FileAttributes, FileInfo,
    MagicMetadata, PublicUrl,
};
use crate::db::now_micros;
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};
use std::collections::HashMap;

/// SQL expression reading a BOOLEAN column as BIGINT 0/1 (NULL → 0). The sqlx Any
/// driver cannot decode SQLite BOOLEAN columns, so booleans are always selected this way.
pub fn bool_col(col: &str) -> String {
    format!("CAST(CASE WHEN {col} THEN 1 ELSE 0 END AS BIGINT)")
}

/// Decode a column selected with [`bool_col`].
pub fn get_bool(r: &AnyRow, idx: usize) -> sqlx::Result<bool> {
    Ok(r.try_get::<Option<i64>, _>(idx)?.unwrap_or(0) != 0)
}

/// `$a, $b, …` placeholders starting at `$start`.
fn placeholders(start: usize, n: usize) -> String {
    (start..start + n)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------- album links

/// Row of `public_collection_tokens` as needed by the link middleware.
#[derive(Debug, Clone)]
pub struct CollectionLinkRow {
    /// Link id.
    pub id: i64,
    /// Collection id.
    pub collection_id: i64,
    /// Access token.
    pub access_token: String,
    /// Disabled.
    pub is_disabled: bool,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Device limit (0 = unlimited).
    pub device_limit: i64,
    /// Password hash.
    pub pw_hash: Option<String>,
    /// KDF nonce.
    pub pw_nonce: Option<String>,
    /// KDF mem limit.
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    pub ops_limit: Option<i64>,
    /// Downloads allowed.
    pub enable_download: bool,
    /// Collect allowed.
    pub enable_collect: bool,
    /// Comments allowed.
    pub enable_comment: bool,
    /// Join allowed.
    pub enable_join: bool,
    /// Minimum role.
    pub min_role: Option<String>,
}

const LINK_COLS: &str = "id, collection_id, access_token, CAST(CASE WHEN is_disabled THEN 1 ELSE 0 END AS BIGINT), valid_till, device_limit, pw_hash, pw_nonce, mem_limit, ops_limit, CAST(CASE WHEN enable_download THEN 1 ELSE 0 END AS BIGINT), CAST(CASE WHEN enable_collect THEN 1 ELSE 0 END AS BIGINT), CAST(CASE WHEN enable_comment THEN 1 ELSE 0 END AS BIGINT), CAST(CASE WHEN enable_join THEN 1 ELSE 0 END AS BIGINT), min_role";

fn link_row(r: &AnyRow) -> sqlx::Result<CollectionLinkRow> {
    Ok(CollectionLinkRow {
        id: r.try_get(0)?,
        collection_id: r.try_get(1)?,
        access_token: r.try_get(2)?,
        is_disabled: get_bool(r, 3)?,
        valid_till: r.try_get(4)?,
        device_limit: r.try_get(5)?,
        pw_hash: r.try_get(6)?,
        pw_nonce: r.try_get(7)?,
        mem_limit: r.try_get(8)?,
        ops_limit: r.try_get(9)?,
        enable_download: get_bool(r, 10)?,
        enable_collect: get_bool(r, 11)?,
        enable_comment: get_bool(r, 12)?,
        enable_join: get_bool(r, 13)?,
        min_role: r.try_get(14)?,
    })
}

/// Link by access token (disabled ones included, like museum's summary lookup).
pub async fn collection_link_by_token(
    db: &AnyPool,
    token: &str,
) -> sqlx::Result<Option<CollectionLinkRow>> {
    let q = format!("SELECT {LINK_COLS} FROM public_collection_tokens WHERE access_token = $1");
    match sqlx::query(&q).bind(token).fetch_optional(db).await? {
        Some(r) => Ok(Some(link_row(&r)?)),
        None => Ok(None),
    }
}

/// The collection's active (not disabled) links.
pub async fn active_collection_links(
    db: &AnyPool,
    collection_id: i64,
) -> sqlx::Result<Vec<CollectionLinkRow>> {
    let q = format!(
        "SELECT {LINK_COLS} FROM public_collection_tokens WHERE collection_id = $1 AND is_disabled = FALSE ORDER BY id"
    );
    sqlx::query(&q)
        .bind(collection_id)
        .fetch_all(db)
        .await?
        .iter()
        .map(link_row)
        .collect()
}

// ---------------------------------------------------------------- collections

/// A collection as museum's `CollectionRepo.Get` returns it (no sharees, no owner email),
/// with `publicURLs` in the limited form link viewers get.
pub async fn collection(db: &AnyPool, collection_id: i64) -> sqlx::Result<Option<Collection>> {
    let row = sqlx::query(
        "SELECT collection_id, app, owner_id, encrypted_key, key_decryption_nonce, name, encrypted_name, \
         name_decryption_nonce, type, attributes, updation_time, CAST(CASE WHEN is_deleted THEN 1 ELSE 0 END AS BIGINT), magic_metadata, pub_magic_metadata \
         FROM collections WHERE collection_id = $1",
    )
    .bind(collection_id)
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let name: Option<String> = r.try_get(5)?;
    let encrypted_name: Option<String> = r.try_get(6)?;
    let name_nonce: Option<String> = r.try_get(7)?;
    let attributes: Option<String> = r.try_get(9)?;
    let mut c = Collection {
        id: r.try_get(0)?,
        app: r.try_get::<Option<String>, _>(1)?.unwrap_or_default(),
        owner: CollectionUser {
            id: r.try_get(2)?,
            ..Default::default()
        },
        encrypted_key: r.try_get(3)?,
        key_decryption_nonce: r.try_get::<Option<String>, _>(4)?.unwrap_or_default(),
        kind: r.try_get(8)?,
        attributes: attributes
            .and_then(|a| serde_json::from_str::<CollectionAttributes>(&a).ok())
            .unwrap_or_default(),
        updation_time: r.try_get(10)?,
        is_deleted: get_bool(&r, 11)?,
        magic_metadata: MagicMetadata::from_db(r.try_get(12)?),
        pub_magic_metadata: MagicMetadata::from_db(r.try_get(13)?),
        ..Default::default()
    };
    match name.filter(|n| !n.is_empty()) {
        Some(n) => c.name = n,
        None => {
            c.encrypted_name = encrypted_name.unwrap_or_default();
            c.name_decryption_nonce = name_nonce.unwrap_or_default();
        }
    }
    let urls = active_collection_links(db, collection_id)
        .await?
        .into_iter()
        .map(|l| {
            let pw = l.pw_nonce.is_some();
            PublicUrl {
                enable_download: l.enable_download,
                enable_collect: l.enable_collect,
                enable_comment: l.enable_comment,
                password_enabled: pw,
                nonce: l.pw_nonce,
                mem_limit: if pw { l.mem_limit } else { None },
                ops_limit: if pw { l.ops_limit } else { None },
                enable_join: l.enable_join,
                ..Default::default()
            }
        })
        .collect();
    c.public_urls = Some(urls);
    Ok(Some(c))
}

/// The collection's owner.
pub async fn collection_owner(db: &AnyPool, collection_id: i64) -> sqlx::Result<Option<i64>> {
    let r: Option<(i64,)> =
        sqlx::query_as("SELECT owner_id FROM collections WHERE collection_id = $1")
            .bind(collection_id)
            .fetch_optional(db)
            .await?;
    Ok(r.map(|r| r.0))
}

/// True when `user_id` owns the (not deleted) collection or is an active sharee of it
/// (museum `access.GetCollection`).
pub async fn can_view_collection(
    db: &AnyPool,
    collection_id: i64,
    user_id: i64,
) -> sqlx::Result<bool> {
    let q = format!(
        "SELECT owner_id, {} FROM collections WHERE collection_id = $1",
        bool_col("is_deleted")
    );
    let r: Option<(i64, i64)> = sqlx::query_as(&q)
        .bind(collection_id)
        .fetch_optional(db)
        .await?;
    let Some((owner, deleted)) = r else {
        return Ok(false);
    };
    if deleted != 0 {
        return Ok(false);
    }
    if owner == user_id {
        return Ok(true);
    }
    let s: Option<(i64,)> = sqlx::query_as(
        "SELECT collection_id FROM collection_shares WHERE collection_id = $1 AND to_user_id = $2 AND is_deleted = FALSE",
    )
    .bind(collection_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(s.is_some())
}

// ---------------------------------------------------------------- files

const DIFF_COLS: &str = "files.file_id, files.owner_id, collection_files.collection_id, collection_files.c_owner_id, \
    collection_files.encrypted_key, collection_files.key_decryption_nonce, \
    files.file_decryption_header, files.thumbnail_decryption_header, \
    files.metadata_decryption_header, files.encrypted_metadata, files.magic_metadata, files.pub_magic_metadata, \
    files.info, collection_files.action_user, collection_files.action, CAST(CASE WHEN collection_files.is_deleted THEN 1 ELSE 0 END AS BIGINT), \
    collection_files.updation_time, collection_files.created_at";

fn diff_row(r: &AnyRow) -> sqlx::Result<File> {
    Ok(File {
        id: r.try_get(0)?,
        owner_id: r.try_get(1)?,
        collection_id: r.try_get(2)?,
        collection_owner_id: r.try_get(3)?,
        encrypted_key: r.try_get(4)?,
        key_decryption_nonce: r.try_get(5)?,
        file: FileAttributes {
            decryption_header: r.try_get(6)?,
            ..Default::default()
        },
        thumbnail: FileAttributes {
            decryption_header: r.try_get(7)?,
            ..Default::default()
        },
        metadata: FileAttributes {
            decryption_header: r.try_get(8)?,
            encrypted_data: r.try_get(9)?,
            ..Default::default()
        },
        magic_metadata: MagicMetadata::from_db(r.try_get(10)?),
        pub_magic_metadata: MagicMetadata::from_db(r.try_get(11)?),
        info: FileInfo::from_db(r.try_get(12)?),
        action_user: r.try_get(13)?,
        action: r.try_get(14)?,
        is_deleted: get_bool(r, 15)?,
        updation_time: r.try_get(16)?,
        collection_added_at: r.try_get(17)?,
    })
}

/// Collection entries changed after `since`, oldest first, at most `limit`
/// (museum `CollectionRepo.GetDiff`).
pub async fn collection_diff(
    db: &AnyPool,
    collection_id: i64,
    since: i64,
    limit: i64,
) -> sqlx::Result<Vec<File>> {
    let q = format!(
        "SELECT {DIFF_COLS} FROM files INNER JOIN collection_files ON collection_files.file_id = files.file_id \
         AND collection_files.collection_id = $1 AND collection_files.updation_time > $2 \
         ORDER BY collection_files.updation_time LIMIT $3"
    );
    sqlx::query(&q)
        .bind(collection_id)
        .bind(since)
        .bind(limit)
        .fetch_all(db)
        .await?
        .iter()
        .map(diff_row)
        .collect()
}

/// All collection entries with exactly this version (museum `GetFilesWithVersion`).
pub async fn collection_files_with_version(
    db: &AnyPool,
    collection_id: i64,
    version: i64,
) -> sqlx::Result<Vec<File>> {
    let q = format!(
        "SELECT {DIFF_COLS} FROM files INNER JOIN collection_files ON collection_files.file_id = files.file_id \
         AND collection_files.collection_id = $1 AND collection_files.updation_time = $2"
    );
    sqlx::query(&q)
        .bind(collection_id)
        .bind(version)
        .fetch_all(db)
        .await?
        .iter()
        .map(diff_row)
        .collect()
}

/// True when the file is an active member of the collection (not deleted, no pending REMOVE).
pub async fn file_active_in_collection(
    db: &AnyPool,
    collection_id: i64,
    file_id: i64,
) -> sqlx::Result<bool> {
    let q = format!(
        "SELECT {}, action FROM collection_files WHERE collection_id = $1 AND file_id = $2",
        bool_col("is_deleted")
    );
    let r: Option<(i64, Option<String>)> = sqlx::query_as(&q)
        .bind(collection_id)
        .bind(file_id)
        .fetch_optional(db)
        .await?;
    Ok(matches!(r, Some((d, a)) if d == 0 && a.as_deref() != Some("REMOVE")))
}

/// Outcome of an object lookup scoped to a collection or owner.
#[derive(Debug, PartialEq, Eq)]
pub enum ObjectLookup {
    /// The file is not reachable through the given scope (museum: permission denied).
    Denied,
    /// The file is reachable but has no live object of that type (museum: no rows).
    Missing,
    /// Object key.
    Found(String),
}

async fn object_key_for(db: &AnyPool, file_id: i64, o_type: &str) -> sqlx::Result<Option<String>> {
    let r: Option<(String,)> = sqlx::query_as(
        "SELECT object_key FROM object_keys WHERE file_id = $1 AND o_type = $2 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .bind(o_type)
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| r.0))
}

/// Object of a file reached through a collection (museum `GetCollectionObject`).
pub async fn collection_object(
    db: &AnyPool,
    collection_id: i64,
    file_id: i64,
    o_type: &str,
) -> sqlx::Result<ObjectLookup> {
    if !file_active_in_collection(db, collection_id, file_id).await? {
        return Ok(ObjectLookup::Denied);
    }
    Ok(match object_key_for(db, file_id, o_type).await? {
        Some(k) => ObjectLookup::Found(k),
        None => ObjectLookup::Missing,
    })
}

/// Object of a file that must be owned by `owner_id` (museum `GetOwnedObject`).
pub async fn owned_object(
    db: &AnyPool,
    owner_id: i64,
    file_id: i64,
    o_type: &str,
) -> sqlx::Result<ObjectLookup> {
    let r: Option<(i64,)> = sqlx::query_as("SELECT owner_id FROM files WHERE file_id = $1")
        .bind(file_id)
        .fetch_optional(db)
        .await?;
    match r {
        None => Ok(ObjectLookup::Missing),
        Some((o,)) if o != owner_id => Ok(ObjectLookup::Denied),
        Some(_) => Ok(match object_key_for(db, file_id, o_type).await? {
            Some(k) => ObjectLookup::Found(k),
            None => ObjectLookup::Missing,
        }),
    }
}

/// Object of a file with no access scoping (museum `ObjectRepo.GetObject`); callers check access.
pub async fn object(db: &AnyPool, file_id: i64, o_type: &str) -> sqlx::Result<Option<String>> {
    object_key_for(db, file_id, o_type).await
}

/// museum `FileRepo.GetFileAttributes`.
pub async fn file_attributes(db: &AnyPool, file_id: i64) -> sqlx::Result<Option<File>> {
    let r = sqlx::query(
        "SELECT file_id, owner_id, file_decryption_header, thumbnail_decryption_header, metadata_decryption_header, \
         encrypted_metadata, pub_magic_metadata, info FROM files WHERE file_id = $1",
    )
    .bind(file_id)
    .fetch_optional(db)
    .await?;
    let Some(r) = r else { return Ok(None) };
    Ok(Some(File {
        id: r.try_get(0)?,
        owner_id: r.try_get(1)?,
        file: FileAttributes {
            decryption_header: r.try_get(2)?,
            ..Default::default()
        },
        thumbnail: FileAttributes {
            decryption_header: r.try_get(3)?,
            ..Default::default()
        },
        metadata: FileAttributes {
            decryption_header: r.try_get(4)?,
            encrypted_data: r.try_get(5)?,
            ..Default::default()
        },
        pub_magic_metadata: MagicMetadata::from_db(r.try_get(6)?),
        info: FileInfo::from_db(r.try_get(7)?),
        ..Default::default()
    }))
}

/// Number of collections the file is actively in (museum `GetCollectionCount`).
pub async fn active_collection_count(db: &AnyPool, file_id: i64) -> sqlx::Result<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM collection_files WHERE file_id = $1 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .fetch_one(db)
    .await?;
    Ok(n)
}

/// file id → owner id for the files that exist.
pub async fn file_owners(db: &AnyPool, file_ids: &[i64]) -> sqlx::Result<HashMap<i64, i64>> {
    if file_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let q = format!(
        "SELECT file_id, owner_id FROM files WHERE file_id IN ({})",
        placeholders(1, file_ids.len())
    );
    let mut query = sqlx::query_as::<_, (i64, i64)>(&q);
    for id in file_ids {
        query = query.bind(*id);
    }
    Ok(query.fetch_all(db).await?.into_iter().collect())
}

/// museum `access.CanAccessFile` (minus the duplicate check): every existing file is
/// owned by `user_id` or actively in a collection shared with or by them.
pub async fn can_access_files(db: &AnyPool, user_id: i64, file_ids: &[i64]) -> sqlx::Result<bool> {
    let owners = file_owners(db, file_ids).await?;
    let foreign: Vec<i64> = owners
        .iter()
        .filter(|(_, o)| **o != user_id)
        .map(|(f, _)| *f)
        .collect();
    if foreign.is_empty() {
        return Ok(true);
    }
    let q = format!(
        "SELECT DISTINCT cf.file_id FROM collection_files cf \
         WHERE cf.is_deleted = FALSE AND (cf.action IS NULL OR cf.action <> 'REMOVE') \
         AND cf.collection_id IN (SELECT collection_id FROM collection_shares WHERE is_deleted = FALSE AND (to_user_id = $1 OR from_user_id = $1)) \
         AND cf.file_id IN ({})",
        placeholders(2, foreign.len())
    );
    let mut query = sqlx::query_as::<_, (i64,)>(&q).bind(user_id);
    for id in &foreign {
        query = query.bind(*id);
    }
    let found = query.fetch_all(db).await?.len();
    Ok(found == foreign.len())
}

// ---------------------------------------------------------------- single-file links

/// Row of `public_file_tokens`.
#[derive(Debug, Clone)]
pub struct FileLinkRow {
    /// Link id.
    pub link_id: String,
    /// File id.
    pub file_id: i64,
    /// File owner.
    pub owner_id: i64,
    /// Access token.
    pub access_token: String,
    /// Disabled.
    pub is_disabled: bool,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Device limit.
    pub device_limit: i64,
    /// Password hash.
    pub pw_hash: Option<String>,
    /// KDF nonce.
    pub pw_nonce: Option<String>,
    /// KDF mem limit.
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    pub ops_limit: Option<i64>,
    /// Downloads allowed.
    pub enable_download: bool,
    /// File key encrypted for the link.
    pub encrypted_file_key: Option<String>,
    /// Nonce for that.
    pub encrypted_file_key_nonce: Option<String>,
    /// KDF nonce for the link secret.
    pub kdf_nonce: Option<String>,
    /// KDF mem limit for the link secret.
    pub kdf_mem_limit: Option<i64>,
    /// KDF ops limit for the link secret.
    pub kdf_ops_limit: Option<i64>,
}

const FILE_LINK_COLS: &str = "id, file_id, owner_id, access_token, CAST(CASE WHEN is_disabled THEN 1 ELSE 0 END AS BIGINT), valid_till, device_limit, pw_hash, pw_nonce, mem_limit, ops_limit, CAST(CASE WHEN enable_download THEN 1 ELSE 0 END AS BIGINT), encrypted_file_key, encrypted_file_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit";

fn file_link_row(r: &AnyRow) -> sqlx::Result<FileLinkRow> {
    Ok(FileLinkRow {
        link_id: r.try_get(0)?,
        file_id: r.try_get(1)?,
        owner_id: r.try_get(2)?,
        access_token: r.try_get(3)?,
        is_disabled: get_bool(r, 4)?,
        valid_till: r.try_get(5)?,
        device_limit: r.try_get(6)?,
        pw_hash: r.try_get(7)?,
        pw_nonce: r.try_get(8)?,
        mem_limit: r.try_get(9)?,
        ops_limit: r.try_get(10)?,
        enable_download: get_bool(r, 11)?,
        encrypted_file_key: r.try_get(12)?,
        encrypted_file_key_nonce: r.try_get(13)?,
        kdf_nonce: r.try_get(14)?,
        kdf_mem_limit: r.try_get(15)?,
        kdf_ops_limit: r.try_get(16)?,
    })
}

/// Link by token: the active one, else a disabled one (museum `GetFileUrlRowByToken`).
pub async fn file_link_by_token(db: &AnyPool, token: &str) -> sqlx::Result<Option<FileLinkRow>> {
    let q = format!(
        "SELECT {FILE_LINK_COLS} FROM public_file_tokens WHERE access_token = $1 ORDER BY CASE WHEN is_disabled THEN 1 ELSE 0 END, created_at DESC LIMIT 1"
    );
    match sqlx::query(&q).bind(token).fetch_optional(db).await? {
        Some(r) => Ok(Some(file_link_row(&r)?)),
        None => Ok(None),
    }
}

/// The file's active link.
pub async fn active_file_link(db: &AnyPool, file_id: i64) -> sqlx::Result<Option<FileLinkRow>> {
    let q = format!(
        "SELECT {FILE_LINK_COLS} FROM public_file_tokens WHERE file_id = $1 AND is_disabled = FALSE"
    );
    match sqlx::query(&q).bind(file_id).fetch_optional(db).await? {
        Some(r) => Ok(Some(file_link_row(&r)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------- file data

/// Row of `file_data`.
#[derive(Debug, Clone)]
pub struct FileDataRow {
    /// File id.
    pub file_id: i64,
    /// Owner of the derived data.
    pub user_id: i64,
    /// Type (`vid_preview`, `img_preview`, `mldata`).
    pub data_type: String,
    /// Object id for previews.
    pub obj_id: Option<String>,
    /// Deleted.
    pub is_deleted: bool,
    /// Updated, microseconds.
    pub updated_at: i64,
}

/// museum `filedata.Repository.GetFilesData` for one file.
pub async fn file_data(
    db: &AnyPool,
    file_id: i64,
    data_type: &str,
) -> sqlx::Result<Option<FileDataRow>> {
    let r = sqlx::query(
        "SELECT file_id, user_id, data_type, obj_id, CAST(CASE WHEN is_deleted THEN 1 ELSE 0 END AS BIGINT), updated_at FROM file_data WHERE data_type = $1 AND file_id = $2",
    )
    .bind(data_type)
    .bind(file_id)
    .fetch_optional(db)
    .await?;
    let Some(r) = r else { return Ok(None) };
    Ok(Some(FileDataRow {
        file_id: r.try_get(0)?,
        user_id: r.try_get(1)?,
        data_type: r.try_get(2)?,
        obj_id: r.try_get(3)?,
        is_deleted: get_bool(&r, 4)?,
        updated_at: r.try_get(5)?,
    }))
}

// ---------------------------------------------------------------- quota and file creation

/// Bytes the user may still store: `users.storage_quota - usage.storage_consumed`.
pub async fn storage_available(db: &AnyPool, user_id: i64) -> sqlx::Result<i64> {
    let quota: Option<(i64,)> =
        sqlx::query_as("SELECT storage_quota FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    let used: Option<(i64,)> =
        sqlx::query_as("SELECT storage_consumed FROM usage WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(quota.map(|q| q.0).unwrap_or(0) - used.map(|u| u.0).unwrap_or(0))
}

/// Error from [`create_file`].
#[derive(Debug)]
pub enum CreateFileError {
    /// An object key is already used by another file.
    DuplicateObject,
    /// Database failure.
    Db(sqlx::Error),
}

impl From<sqlx::Error> for CreateFileError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => CreateFileError::DuplicateObject,
            _ => CreateFileError::Db(e),
        }
    }
}

/// Insert a new file owned by the collection owner into the collection, with its two
/// objects, and charge the owner's usage (museum `FileRepo.Create`). `file` must already
/// carry validated sizes, `info`, owner and collection.
pub async fn create_file(db: &AnyPool, file: &mut File, app: &str) -> Result<(), CreateFileError> {
    let to_json =
        |m: &Option<MagicMetadata>| m.as_ref().and_then(|m| serde_json::to_string(m).ok());
    let mut tx = db.begin().await?;
    let (file_id,): (i64,) = sqlx::query_as(
        "INSERT INTO files (owner_id, app, encrypted_metadata, file_decryption_header, thumbnail_decryption_header, \
         metadata_decryption_header, magic_metadata, pub_magic_metadata, info, updation_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING file_id",
    )
    .bind(file.owner_id)
    .bind(app)
    .bind(&file.metadata.encrypted_data)
    .bind(&file.file.decryption_header)
    .bind(&file.thumbnail.decryption_header)
    .bind(&file.metadata.decryption_header)
    .bind(to_json(&file.magic_metadata))
    .bind(to_json(&file.pub_magic_metadata))
    .bind(file.info.as_ref().and_then(|i| serde_json::to_string(i).ok()))
    .bind(file.updation_time)
    .fetch_one(&mut *tx)
    .await?;
    file.id = file_id;
    sqlx::query(
        "INSERT INTO collection_files (collection_id, file_id, encrypted_key, key_decryption_nonce, is_deleted, \
         updation_time, c_owner_id, f_owner_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(file.collection_id)
    .bind(file_id)
    .bind(&file.encrypted_key)
    .bind(&file.key_decryption_nonce)
    .bind(false)
    .bind(file.updation_time)
    .bind(file.owner_id)
    .bind(file.owner_id)
    .bind(now_micros())
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE collections SET updation_time = $1 WHERE collection_id = $2")
        .bind(file.updation_time)
        .bind(file.collection_id)
        .execute(&mut *tx)
        .await?;
    for (o_type, attrs) in [("file", &file.file), ("thumbnail", &file.thumbnail)] {
        sqlx::query(
            "INSERT INTO object_keys (file_id, o_type, object_key, size, is_deleted) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(file_id)
        .bind(o_type)
        .bind(&attrs.object_key)
        .bind(attrs.size)
        .bind(false)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO usage (user_id, storage_consumed) VALUES ($1, $2) \
         ON CONFLICT (user_id) DO UPDATE SET storage_consumed = usage.storage_consumed + $2",
    )
    .bind(file.owner_id)
    .bind(file.file.size + file.thumbnail.size)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
