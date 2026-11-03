//! File access shared by album links, memory links and cast: signed object URLs,
//! collection diffs, and derived file data (video/image previews).

use super::http::{bad_request_msg, not_found_code, signed_get};
use super::library_sql::{self, ObjectLookup};
use super::models::File;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use std::collections::HashMap;

/// museum `CollectionDiffLimit`.
pub const DIFF_LIMIT: i64 = 2500;

/// `file` or `thumbnail`.
#[derive(Debug, Clone, Copy)]
pub enum ObjectType {
    /// The encrypted file.
    File,
    /// The encrypted thumbnail.
    Thumbnail,
}

impl ObjectType {
    /// museum's `o_type` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectType::File => "file",
            ObjectType::Thumbnail => "thumbnail",
        }
    }
}

fn lookup_url(l: ObjectLookup) -> Result<String, ApiError> {
    match l {
        ObjectLookup::Found(k) => Ok(k),
        ObjectLookup::Denied => Err(ApiError::status(StatusCode::FORBIDDEN)),
        ObjectLookup::Missing => Err(ApiError::status(StatusCode::NOT_FOUND)),
    }
}

/// Parse the `{fileID}` path segment (museum: 400 `{}` when invalid).
pub fn parse_file_id(raw: &str) -> ApiResult<i64> {
    raw.parse()
        .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
}

/// Signed URL for a file reached through a collection (museum `GetPublicOrCastFileURL`).
pub async fn collection_file_url(
    state: &AppState,
    collection_id: i64,
    file_id: i64,
    kind: ObjectType,
) -> ApiResult<String> {
    let key = lookup_url(
        library_sql::collection_object(&state.db.pool, collection_id, file_id, kind.as_str())
            .await?,
    )?;
    signed_get(state, &key).await
}

/// Signed URL for a file that must belong to `owner_id` (museum `GetFileURLForOwner`).
pub async fn owned_file_url(
    state: &AppState,
    owner_id: i64,
    file_id: i64,
    kind: ObjectType,
) -> ApiResult<String> {
    let key = lookup_url(
        library_sql::owned_object(&state.db.pool, owner_id, file_id, kind.as_str()).await?,
    )?;
    signed_get(state, &key).await
}

/// Signed URL without scoping; the caller has checked access (museum `GetSignedURLForPublicFile`).
pub async fn file_url(state: &AppState, file_id: i64, kind: ObjectType) -> ApiResult<String> {
    let key = library_sql::object(&state.db.pool, file_id, kind.as_str())
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    signed_get(state, &key).await
}

/// museum `getDiff` + the public/cast post-processing: never splits a version across
/// pages, strips private metadata and actions, and scrubs deleted entries.
pub async fn public_diff(
    state: &AppState,
    collection_id: i64,
    since: i64,
) -> ApiResult<(Vec<File>, bool)> {
    let db = &state.db.pool;
    let mut rows = library_sql::collection_diff(db, collection_id, since, DIFF_LIMIT + 1).await?;
    let has_more = rows.len() as i64 > DIFF_LIMIT;
    if has_more {
        let last = rows[DIFF_LIMIT as usize].updation_time;
        while rows.last().is_some_and(|f| f.updation_time == last) {
            rows.pop();
        }
        if rows.is_empty() {
            rows = library_sql::collection_files_with_version(db, collection_id, last).await?;
        }
    }
    Ok((
        rows.into_iter().map(File::for_public_diff).collect(),
        has_more,
    ))
}

/// Parsed `GET …/files/data/fetch` / `…/file-data` query (museum `filedata.GetFileData`).
#[derive(Debug)]
pub struct FileDataQuery {
    /// File id.
    pub file_id: i64,
    /// Data type.
    pub kind: String,
    /// Reply 204 instead of 404 when absent.
    pub prefer_no_content: bool,
}

fn required_msg(field: &str) -> String {
    format!(
        "Key: 'GetFileData.{field}' Error:Field validation for '{field}' failed on the 'required' tag"
    )
}

/// Bind the file-data query like gin's `ShouldBindQuery`; the error is the bind message.
pub fn parse_file_data_query(q: &HashMap<String, String>) -> Result<FileDataQuery, String> {
    let file_id = match q.get("fileID").filter(|v| !v.is_empty()) {
        None => return Err(required_msg("FileID")),
        Some(v) => v
            .parse::<i64>()
            .map_err(|e| format!("strconv.ParseInt: parsing {v:?}: {e}"))?,
    };
    if file_id == 0 {
        return Err(required_msg("FileID"));
    }
    let kind = q
        .get("type")
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(|| required_msg("Type"))?;
    let prefer_no_content = match q.get("preferNoContent").map(String::as_str) {
        None | Some("") => false,
        Some(v) => v
            .parse::<bool>()
            .map_err(|_| format!("strconv.ParseBool: parsing {v:?}: invalid syntax"))?,
    };
    Ok(FileDataQuery {
        file_id,
        kind,
        prefer_no_content,
    })
}

/// Parsed preview-URL query (museum `filedata.GetPreviewURLRequest`); `None` on bind failure.
pub fn parse_preview_query(q: &HashMap<String, String>) -> Option<(i64, String)> {
    let file_id: i64 = q.get("fileID")?.parse().ok().filter(|v| *v != 0)?;
    let kind = q.get("type").filter(|v| !v.is_empty())?.clone();
    Some((file_id, kind))
}

async fn ensure_access(state: &AppState, actor: i64, file_id: i64) -> ApiResult<()> {
    if library_sql::can_access_files(&state.db.pool, actor, &[file_id]).await? {
        Ok(())
    } else {
        Err(ApiError::status(StatusCode::FORBIDDEN))
    }
}

/// Encrypted file data (only `vid_preview` playlists via public routes), read through
/// `actor`'s access (museum `filedata.Controller.GetFileData`). `Ok(None)` means 204.
pub async fn file_data(
    state: &AppState,
    actor: i64,
    q: &FileDataQuery,
) -> ApiResult<Option<serde_json::Value>> {
    if q.kind != "vid_preview" && q.kind != "mldata" {
        return Err(bad_request_msg(format!(
            "unsupported object type {}",
            q.kind
        )));
    }
    ensure_access(state, actor, q.file_id).await?;
    let row = library_sql::file_data(&state.db.pool, q.file_id, &q.kind).await?;
    let Some(row) = row.filter(|r| !r.is_deleted) else {
        if q.prefer_no_content {
            return Ok(None);
        }
        return Err(not_found_code(""));
    };
    let key = if row.data_type == "mldata" {
        format!("{}/file-data/{}/mldata", row.user_id, row.file_id)
    } else {
        format!(
            "{}/file-data/{}/{}/{}_playlist",
            row.user_id,
            row.file_id,
            row.data_type,
            row.obj_id.clone().unwrap_or_default()
        )
    };
    let bytes = state.store.get_bytes(&key).await?;
    #[derive(serde::Deserialize)]
    struct S3FileMetadata {
        #[serde(rename = "encryptedData", default)]
        encrypted_data: String,
        #[serde(rename = "header", default)]
        decryption_header: String,
    }
    let meta: S3FileMetadata = serde_json::from_slice(&bytes).map_err(ApiError::internal)?;
    Ok(Some(serde_json::json!({
        "fileID": row.file_id,
        "type": row.data_type,
        "encryptedData": meta.encrypted_data,
        "decryptionHeader": meta.decryption_header,
        "updatedAt": row.updated_at,
    })))
}

/// Signed URL for a video/image preview object (museum `GetPreviewUrl`).
pub async fn preview_url(
    state: &AppState,
    actor: i64,
    file_id: i64,
    kind: &str,
) -> ApiResult<String> {
    if kind != "vid_preview" && kind != "img_preview" {
        return Err(bad_request_msg(format!("unsupported object type {kind}")));
    }
    ensure_access(state, actor, file_id).await?;
    let row = library_sql::file_data(&state.db.pool, file_id, kind)
        .await?
        .filter(|r| !r.is_deleted)
        .ok_or_else(|| not_found_code(""))?;
    let key = format!(
        "{}/file-data/{}/{}/{}",
        row.user_id,
        row.file_id,
        row.data_type,
        row.obj_id.unwrap_or_default()
    );
    signed_get(state, &key).await
}
