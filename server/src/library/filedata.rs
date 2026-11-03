//! File data (museum's `filedata` controller): encrypted per-file derived data stored as
//! objects next to the file, tracked in `file_data`.
//!
//! Types (`data_type`):
//! * `mldata`: ML embeddings/faces. One object `"<owner>/file-data/<fileID>/mldata"` holding
//!   the JSON `{"v","encryptedData","header","client"}`.
//! * `vid_preview`: streamable video preview. The client uploads the encrypted stream to
//!   `"<owner>/file-data/<fileID>/vid_preview/<objectID>"`, then registers it with its
//!   playlist, which is stored as `<that key>_playlist` in the same JSON shape.
//! * `img_preview`: image preview, uploaded to `".../img_preview/<objectID>"`.
//!
//! When a file is deleted, its rows are marked `is_deleted` (see
//! [`super::trash::schedule_deletion`]) and the background task
//! ([`process_deleted`]) removes the objects and rows.

use super::errors;
use super::files::{self, bind_ids};
use super::links::new_id;
use super::objects::{self, MAX_PART_SIZE, TempObject, URL_TTL};
use super::sql::{db_err, get_bool, get_i64, get_opt_i64, get_opt_str, get_str, ids_list};
use crate::auth::App;
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::any::AnyRow;
use std::collections::HashSet;

/// ML data.
pub const ML_DATA: &str = "mldata";
/// Video preview.
pub const PREVIEW_VIDEO: &str = "vid_preview";
/// Image preview.
pub const PREVIEW_IMAGE: &str = "img_preview";
/// Rows per status diff page.
pub const STATUS_DIFF_LIMIT: i64 = 5000;
/// Most ids per `/files/data/fetch`.
pub const MAX_FETCH_IDS: usize = 200;
const MAX_PREVIEW_UPLOAD_SIZE: i64 = 10 << 30;
const SMALL_VIDEO_PREVIEW_SIZE: i64 = 200_000_000;

/// `filedata.BasePrefix`.
pub fn base_prefix(file_id: i64, owner_id: i64) -> String {
    format!("{owner_id}/file-data/{file_id}/")
}

/// `filedata.ObjectKey`: the uploaded preview object.
pub fn object_key(file_id: i64, owner_id: i64, kind: &str, id: &str) -> String {
    format!("{}{kind}/{id}", base_prefix(file_id, owner_id))
}

/// `filedata.ObjectMetadataKey`: the JSON object (`mldata` or the video playlist).
pub fn metadata_key(file_id: i64, owner_id: i64, kind: &str, id: Option<&str>) -> String {
    match (kind, id) {
        (PREVIEW_VIDEO, Some(id)) => {
            format!("{}_playlist", object_key(file_id, owner_id, kind, id))
        }
        _ => format!("{}{kind}", base_prefix(file_id, owner_id)),
    }
}

/// `filedata.S3FileMetadata`: the stored JSON object.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StoredMetadata {
    /// Version.
    #[serde(rename = "v")]
    pub version: i64,
    /// Encrypted data.
    #[serde(rename = "encryptedData")]
    pub encrypted_data: String,
    /// Decryption header.
    #[serde(rename = "header")]
    pub decryption_header: String,
    /// Client that wrote it.
    pub client: String,
}

/// `filedata.Entity`.
#[derive(Debug, Clone, Serialize)]
pub struct Entity {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Data type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Encrypted data.
    #[serde(rename = "encryptedData")]
    pub encrypted_data: String,
    /// Decryption header.
    #[serde(rename = "decryptionHeader")]
    pub decryption_header: String,
    /// Version (`file_data.updated_at`).
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// `filedata.FDStatus`.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Owner.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// Data type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Deleted.
    #[serde(rename = "isDeleted")]
    pub is_deleted: bool,
    /// Preview object id.
    #[serde(rename = "objectID")]
    pub object_id: Option<String>,
    /// Preview object nonce.
    #[serde(rename = "objectNonce")]
    pub object_nonce: Option<String>,
    /// Bytes.
    pub size: i64,
    /// Version.
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// A `file_data` row.
#[derive(Debug, Clone)]
pub struct Row {
    /// File id.
    pub file_id: i64,
    /// Owner.
    pub user_id: i64,
    /// Data type.
    pub kind: String,
    /// Bytes.
    pub size: i64,
    /// Store the object lives in.
    pub latest_bucket: String,
    /// Deleted.
    pub is_deleted: bool,
    /// Preview object id.
    pub obj_id: Option<String>,
    /// Preview object nonce.
    pub obj_nonce: Option<String>,
    /// Preview object size.
    pub obj_size: Option<i64>,
    /// Version.
    pub updated_at: i64,
}

impl Row {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            file_id: get_i64(r, "file_id")?,
            user_id: get_i64(r, "user_id")?,
            kind: get_str(r, "data_type")?,
            size: get_i64(r, "size")?,
            latest_bucket: get_str(r, "latest_bucket")?,
            is_deleted: get_bool(r, "is_deleted")?,
            obj_id: get_opt_str(r, "obj_id")?,
            obj_nonce: get_opt_str(r, "obj_nonce")?,
            obj_size: get_opt_i64(r, "obj_size")?,
            updated_at: get_i64(r, "updated_at")?,
        })
    }

    /// Key of the JSON object (`mldata`, or the video playlist).
    pub fn metadata_key(&self) -> String {
        metadata_key(
            self.file_id,
            self.user_id,
            &self.kind,
            self.obj_id.as_deref(),
        )
    }

    /// Every object key this row owns.
    pub fn all_keys(&self) -> Vec<String> {
        match (self.kind.as_str(), self.obj_id.as_deref()) {
            (ML_DATA, _) => vec![self.metadata_key()],
            (PREVIEW_VIDEO, Some(id)) => vec![
                object_key(self.file_id, self.user_id, PREVIEW_VIDEO, id),
                self.metadata_key(),
            ],
            (k, Some(id)) => vec![object_key(self.file_id, self.user_id, k, id)],
            _ => Vec::new(),
        }
    }
}

const ROW_COLUMNS: &str = "file_id, user_id, data_type, size, latest_bucket, is_deleted, obj_id, obj_nonce, obj_size, updated_at";

/// `Repository.GetFilesData`: rows of `kind` for `ids`.
pub async fn rows_for(state: &AppState, kind: &str, ids: &[i64]) -> ApiResult<Vec<Row>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {ROW_COLUMNS} FROM file_data WHERE data_type = $1 AND file_id IN ({})",
        ids_list(2, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(kind.to_string()), ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?
        .iter()
        .map(Row::from_row)
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

fn invalid_version(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "INVALID_VERSION", msg)
}

async fn validate_last_updated_at(
    state: &AppState,
    last: Option<i64>,
    file_id: i64,
    kind: &str,
) -> ApiResult<()> {
    let Some(last) = last else { return Ok(()) };
    let rows = rows_for(state, kind, &[file_id]).await?;
    match rows.first() {
        None if last == 0 => Ok(()),
        None => Err(invalid_version("non zero version empty data")),
        Some(r) if r.is_deleted => Err(invalid_version("data deleted")),
        Some(r) if r.updated_at != last => Err(invalid_version(format!(
            "version mismatch expected {}, found {last}",
            r.updated_at
        ))),
        Some(_) => Ok(()),
    }
}

async fn upload_json(state: &AppState, key: &str, obj: &StoredMetadata) -> ApiResult<i64> {
    let body = serde_json::to_vec(obj).map_err(errors::internal)?;
    let len = body.len() as i64;
    state
        .store
        .put_bytes(key, bytes::Bytes::from(body))
        .await
        .map_err(errors::storage)?;
    Ok(len)
}

/// Upsert a row; a deleted row is left as it is. Old preview objects that the new row no
/// longer references are queued for deletion.
async fn upsert_row(state: &AppState, row: &Row) -> ApiResult<u64> {
    let now = now_micros();
    let old = rows_for(state, &row.kind, &[row.file_id]).await?;
    let r = sqlx::query(
        "INSERT INTO file_data (file_id, user_id, data_type, size, latest_bucket, obj_id, obj_nonce,
            obj_size, sync_locked_till, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, $9, $9)
         ON CONFLICT (file_id, data_type) DO UPDATE SET size = $4, latest_bucket = $5, obj_id = $6,
            obj_nonce = $7, obj_size = $8, pending_sync = TRUE, updated_at = $9
         WHERE file_data.is_deleted = FALSE",
    )
    .bind(row.file_id)
    .bind(row.user_id)
    .bind(row.kind.clone())
    .bind(row.size)
    .bind(row.latest_bucket.clone())
    .bind(row.obj_id.clone())
    .bind(row.obj_nonce.clone())
    .bind(row.obj_size)
    .bind(now)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    if r.rows_affected() > 0
        && let Some(o) = old.first()
    {
        {
            let keep: HashSet<String> = row.all_keys().into_iter().collect();
            let stale: Vec<String> = o
                .all_keys()
                .into_iter()
                .filter(|k| !keep.contains(k))
                .collect();
            if !stale.is_empty() {
                let mut conn = state.db.pool.acquire().await.map_err(db_err)?;
                objects::queue_object_deletion(&mut conn, &stale)
                    .await
                    .map_err(db_err)?;
            }
        }
    }
    Ok(r.rows_affected())
}

/// `PUT /files/data` request (`PutFileDataRequest`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PutRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Data type (only `mldata`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Encrypted data.
    #[serde(rename = "encryptedData")]
    pub encrypted_data: Option<String>,
    /// Decryption header.
    #[serde(rename = "decryptionHeader")]
    pub decryption_header: Option<String>,
    /// Data version (defaults to 1).
    pub version: Option<i64>,
    /// Optimistic concurrency: the `updatedAt` the client last saw.
    #[serde(rename = "lastUpdatedAt")]
    pub last_updated_at: Option<i64>,
}

impl PutRequest {
    /// `PutFileDataRequest.Validate` (gin `required` on fileID/type included).
    pub fn validate(&self) -> ApiResult<()> {
        if self.file_id == 0 || self.kind.is_empty() {
            return Err(errors::bad_msg("fileID and type are required"));
        }
        if self.kind != ML_DATA {
            return Err(errors::bad_msg(format!(
                "invalid object type {}",
                self.kind
            )));
        }
        let present = matches!((&self.encrypted_data, &self.decryption_header),
            (Some(d), Some(h)) if !d.is_empty() && !h.is_empty());
        if !present {
            return Err(errors::bad_msg(
                "encryptedData and decryptionHeader (only) are required for derived meta",
            ));
        }
        Ok(())
    }
}

/// `InsertOrUpdateMetadata` (`PUT /files/data`).
pub async fn put(state: &AppState, actor: i64, client: &str, req: &PutRequest) -> ApiResult<()> {
    req.validate()?;
    let (owner, _) = files::owner_and_app(state, req.file_id).await?;
    if owner != actor {
        files::can_access_files(state, actor, &[req.file_id]).await?;
    }
    validate_last_updated_at(state, req.last_updated_at, req.file_id, &req.kind).await?;
    let key = metadata_key(req.file_id, owner, &req.kind, None);
    let size = upload_json(
        state,
        &key,
        &StoredMetadata {
            version: req.version.unwrap_or(1),
            encrypted_data: req.encrypted_data.clone().unwrap_or_default(),
            decryption_header: req.decryption_header.clone().unwrap_or_default(),
            client: client.to_string(),
        },
    )
    .await?;
    upsert_row(
        state,
        &Row {
            file_id: req.file_id,
            user_id: owner,
            kind: req.kind.clone(),
            size,
            latest_bucket: state.store.name().to_string(),
            is_deleted: false,
            obj_id: None,
            obj_nonce: None,
            obj_size: None,
            updated_at: 0,
        },
    )
    .await?;
    Ok(())
}

/// `PUT /files/video-data` request (`VidPreviewRequest`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VideoRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// `pv_…` id from the preview upload URL.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Bytes uploaded.
    #[serde(rename = "objectSize")]
    pub object_size: i64,
    /// Encrypted playlist.
    pub playlist: String,
    /// Playlist header.
    #[serde(rename = "playlistHeader")]
    pub playlist_header: String,
    /// Version (defaults to 1).
    pub version: Option<i64>,
}

fn is_video_object_id(s: &str) -> bool {
    s.len() == 25 && s.starts_with("pv_") && s[3..].bytes().all(|b| b.is_ascii_alphanumeric())
}

impl VideoRequest {
    /// `VidPreviewRequest.Validate`.
    pub fn validate(&self) -> ApiResult<()> {
        if self.file_id == 0 || self.object_id.is_empty() || self.object_size == 0 {
            return Err(errors::bad_msg("invalid request body"));
        }
        if self.object_size <= 0 {
            return Err(errors::bad_msg(
                "objectSize must be positive for preview video",
            ));
        }
        if self.playlist.is_empty() || self.playlist_header.is_empty() {
            return Err(errors::bad_msg(
                "playlist and playListHeader are required for preview video",
            ));
        }
        if !is_video_object_id(&self.object_id) {
            return Err(errors::bad_msg("invalid video preview object ID"));
        }
        Ok(())
    }
}

/// `VidPreviewRequest.ValidateSize`: a preview may be at most 3× the original, or just under
/// 4× when it is smaller than 200 MB.
pub fn validate_video_size(original: i64, preview: i64) -> ApiResult<()> {
    if original <= 0 {
        return Err(errors::bad_msg(
            "original file size is unavailable for preview video",
        ));
    }
    if preview <= 0 {
        return Err(errors::bad_msg(
            "objectSize must be positive for preview video",
        ));
    }
    if original > i64::MAX / 3 || preview <= 3 * original {
        return Ok(());
    }
    if preview < SMALL_VIDEO_PREVIEW_SIZE && preview / 4 < original {
        return Ok(());
    }
    Err(errors::bad_msg(
        "video preview exceeds the allowed size relative to the original file",
    ))
}

async fn check_video_size(
    state: &AppState,
    actor: i64,
    file_id: i64,
    preview: i64,
) -> ApiResult<()> {
    let original = match files::files_info(state, actor, &[file_id]).await {
        Ok(v) => v.first().map(|(_, i)| i.file_size).unwrap_or(0),
        Err(e) => {
            tracing::warn!(file_id, error = %e.message, "could not check video preview size limit");
            return Ok(());
        }
    };
    if original <= 0 {
        return Ok(());
    }
    validate_video_size(original, preview)
}

/// `_checkPreviewWritePerm`: the actor owns the file and it is in at least one collection.
async fn check_preview_write(state: &AppState, actor: i64, file_id: i64) -> ApiResult<()> {
    files::verify_file_ownership(state, actor, &[file_id]).await?;
    let r = sqlx::query(
        "SELECT COUNT(*) AS n FROM collection_files WHERE file_id = $1 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    if get_i64(&r, "n").map_err(db_err)? < 1 {
        return Err(errors::not_found());
    }
    Ok(())
}

/// `InsertVideoPreview` (`PUT /files/video-data`).
pub async fn put_video(
    state: &AppState,
    actor: i64,
    client: &str,
    req: &VideoRequest,
) -> ApiResult<()> {
    req.validate()?;
    check_preview_write(state, actor, req.file_id).await?;
    let owner = actor;
    let file_key = object_key(req.file_id, owner, PREVIEW_VIDEO, &req.object_id);
    let meta_key = metadata_key(req.file_id, owner, PREVIEW_VIDEO, Some(&req.object_id));
    match state.store.head(&file_key).await.map_err(errors::storage)? {
        Some(n) if n as i64 == req.object_size => {}
        Some(n) => {
            return Err(errors::internal(format!(
                "size of the uploaded file ({n}) does not match the expected size ({})",
                req.object_size
            )));
        }
        None => return Err(errors::internal("uploaded preview object not found")),
    }
    check_video_size(state, actor, req.file_id, req.object_size).await?;
    let size = upload_json(
        state,
        &meta_key,
        &StoredMetadata {
            version: req.version.unwrap_or(1),
            encrypted_data: req.playlist.clone(),
            decryption_header: req.playlist_header.clone(),
            client: client.to_string(),
        },
    )
    .await?;
    let row = Row {
        file_id: req.file_id,
        user_id: owner,
        kind: PREVIEW_VIDEO.into(),
        size: size + req.object_size,
        latest_bucket: state.store.name().to_string(),
        is_deleted: false,
        obj_id: Some(req.object_id.clone()),
        obj_nonce: None,
        obj_size: Some(req.object_size),
        updated_at: 0,
    };
    let mut conn = state.db.pool.acquire().await.map_err(db_err)?;
    let removed = objects::remove_temp_object(&mut conn, &file_key).await?;
    drop(conn);
    if removed != 1 {
        // A retried registration: accept it when the stored row already matches.
        let existing = rows_for(state, PREVIEW_VIDEO, &[req.file_id]).await?;
        let dup = existing.first().is_some_and(|e| {
            e.obj_id.as_deref() == Some(req.object_id.as_str())
                && e.obj_size == Some(req.object_size)
        });
        if dup {
            return Ok(());
        }
        return Err(errors::internal("failed to remove object from tempObjects"));
    }
    upsert_row(state, &row).await?;
    Ok(())
}

/// `POST /files/data/fetch` request (`GetFilesData`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FetchRequest {
    /// File ids.
    #[serde(rename = "fileIDs")]
    pub file_ids: Option<Vec<i64>>,
    /// Data type.
    #[serde(rename = "type")]
    pub kind: String,
}

/// `GetFilesDataResponse`.
#[derive(Debug, Clone, Serialize)]
pub struct FetchResponse {
    /// Entities found.
    pub data: Vec<Entity>,
    /// Requested files with no row yet.
    #[serde(rename = "pendingIndexFileIDs")]
    pub pending_index_file_ids: Vec<i64>,
    /// Rows whose object could not be read.
    #[serde(rename = "errFileIDs")]
    pub err_file_ids: Vec<i64>,
}

/// Validate a `/files/data/fetch` batch (`GetFilesData.Validate`).
pub fn validate_fetch(kind: &str, ids: &[i64]) -> ApiResult<()> {
    if kind != PREVIEW_VIDEO && kind != ML_DATA {
        return Err(errors::bad_msg(format!("unsupported object type {kind}")));
    }
    if ids.is_empty() {
        return Err(errors::bad_msg("fileIDs are required"));
    }
    if ids.len() > MAX_FETCH_IDS {
        return Err(errors::bad_msg(
            "fileIDs should be less than or equal to 200",
        ));
    }
    Ok(())
}

/// Read a row's JSON object.
pub async fn read_metadata(state: &AppState, row: &Row) -> Result<StoredMetadata, String> {
    let key = row.metadata_key();
    let mut last = String::new();
    for attempt in 0..3 {
        match state.store.get_bytes(&key).await {
            Ok(b) => return serde_json::from_slice(&b).map_err(|e| e.to_string()),
            Err(crate::storage::StorageError::NotFound) => return Err("object not found".into()),
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt + 1))).await;
    }
    Err(last)
}

/// The fetch part of `GetFilesData` without the access check, for callers that have
/// authorised `ids` themselves (e.g. public album links).
pub async fn fetch_unchecked(
    state: &AppState,
    kind: &str,
    ids: &[i64],
) -> ApiResult<FetchResponse> {
    let rows = rows_for(state, kind, ids).await?;
    let in_db: HashSet<i64> = rows.iter().map(|r| r.file_id).collect();
    let pending: Vec<i64> = ids.iter().copied().filter(|i| !in_db.contains(i)).collect();
    let active: Vec<Row> = rows.into_iter().filter(|r| !r.is_deleted).collect();
    let results =
        futures_util::future::join_all(active.iter().map(|r| read_metadata(state, r))).await;
    let mut data = Vec::new();
    let mut errs = Vec::new();
    for (row, res) in active.iter().zip(results) {
        match res {
            Ok(m) => data.push(Entity {
                file_id: row.file_id,
                kind: row.kind.clone(),
                encrypted_data: m.encrypted_data,
                decryption_header: m.decryption_header,
                updated_at: row.updated_at,
            }),
            Err(e) => {
                tracing::error!(file_id = row.file_id, error = %e, "error fetching file data object");
                errs.push(row.file_id);
            }
        }
    }
    Ok(FetchResponse {
        data,
        pending_index_file_ids: pending,
        err_file_ids: errs,
    })
}

/// `GetFilesData` (`POST /files/data/fetch`).
pub async fn fetch(state: &AppState, actor: i64, req: &FetchRequest) -> ApiResult<FetchResponse> {
    let ids = req.file_ids.clone().unwrap_or_default();
    if req.file_ids.is_none() || req.kind.is_empty() {
        return Err(errors::bad_msg("fileIDs and type are required"));
    }
    validate_fetch(&req.kind, &ids)?;
    files::can_access_files(state, actor, &ids).await?;
    fetch_unchecked(state, &req.kind, &ids).await
}

/// `GetFileData` (`GET /files/data/fetch`). `Ok(None)` means 204 (`preferNoContent`).
pub async fn fetch_one(
    state: &AppState,
    actor: i64,
    file_id: i64,
    kind: &str,
    prefer_no_content: bool,
) -> ApiResult<Option<Entity>> {
    if kind != PREVIEW_VIDEO && kind != ML_DATA {
        return Err(errors::bad_msg(format!("unsupported object type {kind}")));
    }
    files::can_access_files(state, actor, &[file_id]).await?;
    let rows = rows_for(state, kind, &[file_id]).await?;
    let row = match rows.into_iter().next() {
        Some(r) if !r.is_deleted => r,
        _ if prefer_no_content => return Ok(None),
        _ => return Err(errors::not_found_coded()),
    };
    let m = read_metadata(state, &row).await.map_err(errors::internal)?;
    Ok(Some(Entity {
        file_id: row.file_id,
        kind: row.kind,
        encrypted_data: m.encrypted_data,
        decryption_header: m.decryption_header,
        updated_at: row.updated_at,
    }))
}

/// `GetFDForUser` (`POST /files/data/status-diff`): `None` serialises as `null` like museum
/// when nothing changed.
pub async fn status_diff(
    state: &AppState,
    user_id: i64,
    since: i64,
) -> ApiResult<Option<Vec<Status>>> {
    let rows = sqlx::query(
        "SELECT file_id, user_id, data_type, size, is_deleted, obj_id, obj_nonce, updated_at
         FROM file_data WHERE user_id = $1 AND updated_at > $2 ORDER BY updated_at, file_id LIMIT $3",
    )
    .bind(user_id)
    .bind(since)
    .bind(STATUS_DIFF_LIMIT)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(Status {
            file_id: get_i64(r, "file_id").map_err(db_err)?,
            user_id: get_i64(r, "user_id").map_err(db_err)?,
            kind: get_str(r, "data_type").map_err(db_err)?,
            is_deleted: get_bool(r, "is_deleted").map_err(db_err)?,
            object_id: get_opt_str(r, "obj_id").map_err(db_err)?,
            object_nonce: get_opt_str(r, "obj_nonce").map_err(db_err)?,
            size: get_i64(r, "size").map_err(db_err)?,
            updated_at: get_i64(r, "updated_at").map_err(db_err)?,
        });
    }
    Ok(Some(out))
}

/// `filedata.PreviewUploadUrl`.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewUploadUrl {
    /// `pv_…`/`pi_…` id to register later.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Single PUT URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Multipart part URLs.
    #[serde(rename = "partURLs", skip_serializing_if = "Option::is_none")]
    pub part_urls: Option<Vec<String>>,
    /// Multipart completion URL.
    #[serde(rename = "completeURL", skip_serializing_if = "Option::is_none")]
    pub complete_url: Option<String>,
}

fn validate_preview_kind(kind: &str) -> ApiResult<()> {
    if kind != PREVIEW_VIDEO && kind != PREVIEW_IMAGE {
        return Err(errors::bad_msg(format!("unsupported object type {kind}")));
    }
    Ok(())
}

fn validate_preview_upload(file_id: i64, kind: &str, len: i64) -> ApiResult<()> {
    if file_id <= 0 {
        return Err(errors::bad_msg("fileID must be greater than 0"));
    }
    validate_preview_kind(kind)?;
    if len <= 0 || len > MAX_PREVIEW_UPLOAD_SIZE {
        return Err(errors::bad_msg(
            "contentLength must be between 1 byte and 10 GiB",
        ));
    }
    Ok(())
}

async fn preview_object(
    state: &AppState,
    actor: i64,
    app: App,
    client: &str,
    file_id: i64,
    kind: &str,
    len: Option<i64>,
) -> ApiResult<(String, TempObject)> {
    check_preview_write(state, actor, file_id).await?;
    if kind == PREVIEW_VIDEO
        && let Some(l) = len
    {
        check_video_size(state, actor, file_id, l).await?;
    }
    let id = new_id(if kind == PREVIEW_VIDEO { "pv" } else { "pi" });
    Ok((
        id.clone(),
        TempObject {
            object_key: object_key(file_id, actor, kind, &id),
            user_id: Some(actor),
            app: Some(app.as_str().into()),
            purpose: Some(kind.into()),
            client: Some(client.into()),
            ..Default::default()
        },
    ))
}

/// `PreviewUploadURL` (`GET /files/data/preview-upload-url`).
#[allow(clippy::too_many_arguments)]
pub async fn preview_upload_url(
    state: &AppState,
    actor: i64,
    app: App,
    client: &str,
    file_id: i64,
    kind: &str,
    multipart: bool,
    count: Option<i64>,
) -> ApiResult<PreviewUploadUrl> {
    validate_preview_kind(kind)?;
    let parts = if multipart {
        match count {
            None => return Err(errors::bad_msg("count is required for multipart upload")),
            Some(c) if c <= 0 || c > 10_000 => {
                return Err(errors::bad_msg(
                    "invalid count, should be between 1 and 10000",
                ));
            }
            Some(c) => Some(c as u32),
        }
    } else {
        None
    };
    let (id, temp) = preview_object(state, actor, app, client, file_id, kind, None).await?;
    match parts {
        Some(p) => {
            let m = objects::issue_multipart(state, temp, p).await?;
            Ok(PreviewUploadUrl {
                object_id: id,
                url: None,
                part_urls: Some(m.part_urls),
                complete_url: Some(m.complete_url),
            })
        }
        None => {
            let u = objects::issue_put_url(state, temp).await?;
            Ok(PreviewUploadUrl {
                object_id: id,
                url: Some(u.url),
                part_urls: None,
                complete_url: None,
            })
        }
    }
}

/// `POST /files/data/preview-upload-url` request (`PreviewUploadRequest`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PreviewUploadRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// `vid_preview` or `img_preview`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Bytes.
    #[serde(rename = "contentLength")]
    pub content_length: i64,
    /// Base64 MD5.
    #[serde(rename = "contentMD5")]
    pub content_md5: String,
}

/// `PreviewUploadURLWithMetadata`.
pub async fn preview_upload_url_v2(
    state: &AppState,
    actor: i64,
    app: App,
    client: &str,
    req: &PreviewUploadRequest,
) -> ApiResult<PreviewUploadUrl> {
    if req.file_id == 0
        || req.kind.is_empty()
        || req.content_length == 0
        || req.content_md5.is_empty()
    {
        return Err(errors::bad());
    }
    validate_preview_upload(req.file_id, &req.kind, req.content_length)?;
    if req.content_length > MAX_PART_SIZE {
        return Err(errors::bad_msg(
            "previews larger than 5 GiB require multipart upload",
        ));
    }
    objects::validate_md5(&req.content_md5)?;
    let (id, mut temp) = preview_object(
        state,
        actor,
        app,
        client,
        req.file_id,
        &req.kind,
        Some(req.content_length),
    )
    .await?;
    temp.content_length = Some(req.content_length);
    temp.content_md5 = Some(req.content_md5.clone());
    let u = objects::issue_put_url(state, temp).await?;
    Ok(PreviewUploadUrl {
        object_id: id,
        url: Some(u.url),
        part_urls: None,
        complete_url: None,
    })
}

/// `POST /files/data/multipart-preview-upload-url` request.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MultipartPreviewUploadRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// `vid_preview` or `img_preview`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Total bytes.
    #[serde(rename = "contentLength")]
    pub content_length: i64,
    /// Bytes per part.
    #[serde(rename = "partLength")]
    pub part_length: i64,
    /// Base64 MD5 per part.
    #[serde(rename = "partMd5s")]
    pub part_md5s: Option<Vec<String>>,
}

/// `MultipartPreviewUploadURLWithMetadata`.
pub async fn multipart_preview_upload_url(
    state: &AppState,
    actor: i64,
    app: App,
    client: &str,
    req: &MultipartPreviewUploadRequest,
) -> ApiResult<PreviewUploadUrl> {
    let md5s = match &req.part_md5s {
        Some(v)
            if req.file_id != 0
                && !req.kind.is_empty()
                && req.content_length != 0
                && req.part_length != 0 =>
        {
            v.clone()
        }
        _ => return Err(errors::bad()),
    };
    validate_preview_upload(req.file_id, &req.kind, req.content_length)?;
    objects::validate_part_length(req.content_length, req.part_length)?;
    let count = (req.content_length - 1) / req.part_length + 1;
    if md5s.len() as i64 != count {
        return Err(errors::bad_msg(format!(
            "partMd5s must contain exactly {count} checksums"
        )));
    }
    for m in &md5s {
        objects::validate_md5(m)?;
    }
    let (id, mut temp) = preview_object(
        state,
        actor,
        app,
        client,
        req.file_id,
        &req.kind,
        Some(req.content_length),
    )
    .await?;
    temp.content_length = Some(req.content_length);
    let specs = objects::part_specs(req.content_length, req.part_length, count, Some(md5s));
    let m = objects::issue_multipart_with(state, temp, &specs).await?;
    Ok(PreviewUploadUrl {
        object_id: id,
        url: None,
        part_urls: Some(m.part_urls),
        complete_url: Some(m.complete_url),
    })
}

/// `GetPreviewUrl` (`GET /files/data/preview`): a signed GET URL for the preview object.
pub async fn preview_url(
    state: &AppState,
    actor: i64,
    file_id: i64,
    kind: &str,
) -> ApiResult<String> {
    validate_preview_kind(kind)?;
    files::can_access_files(state, actor, &[file_id]).await?;
    preview_url_unchecked(state, file_id, kind).await
}

/// [`preview_url`] without the access check (for callers that authorised the file).
pub async fn preview_url_unchecked(
    state: &AppState,
    file_id: i64,
    kind: &str,
) -> ApiResult<String> {
    let rows = rows_for(state, kind, &[file_id]).await?;
    let row = match rows.into_iter().next() {
        Some(r) if !r.is_deleted => r,
        _ => return Err(errors::not_found_coded()),
    };
    let id = row.obj_id.clone().ok_or_else(errors::not_found_coded)?;
    state
        .store
        .presign_get(
            &object_key(row.file_id, row.user_id, &row.kind, &id),
            URL_TTL,
        )
        .await
        .map_err(errors::storage)
}

/// Background: delete objects and rows of deleted file data. Returns rows processed.
pub async fn process_deleted(state: &AppState, limit: i64) -> ApiResult<usize> {
    let rows: Vec<Row> = sqlx::query(&format!(
        "SELECT {ROW_COLUMNS} FROM file_data WHERE is_deleted = TRUE ORDER BY updated_at LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?
    .iter()
    .map(Row::from_row)
    .collect::<Result<_, _>>()
    .map_err(db_err)?;
    let mut n = 0;
    for row in rows {
        let mut ok = true;
        for k in row.all_keys() {
            if let Err(e) = state.store.delete(&k).await {
                tracing::error!(key = %k, error = %e, "failed to delete file data object");
                ok = false;
            }
        }
        if ok {
            sqlx::query(
                "DELETE FROM file_data WHERE file_id = $1 AND data_type = $2 AND is_deleted = TRUE",
            )
            .bind(row.file_id)
            .bind(row.kind.clone())
            .execute(&state.db.pool)
            .await
            .map_err(db_err)?;
            n += 1;
        }
    }
    Ok(n)
}
