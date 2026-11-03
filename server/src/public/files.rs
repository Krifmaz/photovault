//! File access shared by album links, memory links and cast: signed object URLs,
//! collection diffs, and derived file data (video/image previews).

use super::http::signed_get;
use super::models::{File, for_public_diff};
use crate::error::{ApiError, ApiResult};
use crate::library::files as library_files;
use crate::library::{collections, filedata};
use crate::state::AppState;
use axum::http::StatusCode;
use std::collections::HashMap;

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

/// Parse the `{fileID}` path segment (museum: 400 `{}` when invalid).
pub fn parse_file_id(raw: &str) -> ApiResult<i64> {
    raw.parse()
        .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
}

/// Signed URL for a file reached through a collection (museum `GetPublicOrCastFileURL`,
/// `ObjectRepository.GetCollectionObject`): 403 unless the file is an active member,
/// 404 when it has no live object of that type.
pub async fn collection_file_url(
    state: &AppState,
    collection_id: i64,
    file_id: i64,
    kind: ObjectType,
) -> ApiResult<String> {
    if !collections::is_active_member(state, collection_id, file_id).await? {
        return Err(ApiError::status(StatusCode::FORBIDDEN));
    }
    file_url(state, file_id, kind).await
}

/// Signed URL for a file that must belong to `owner_id` (museum `GetFileURLForOwner`,
/// `ObjectRepository.GetOwnedObject`).
pub async fn owned_file_url(
    state: &AppState,
    owner_id: i64,
    file_id: i64,
    kind: ObjectType,
) -> ApiResult<String> {
    let (owner, _) = library_files::owner_and_app(state, file_id).await?;
    if owner != owner_id {
        return Err(ApiError::status(StatusCode::FORBIDDEN));
    }
    file_url(state, file_id, kind).await
}

/// Signed URL without scoping; the caller has checked access (museum `GetSignedURLForPublicFile`).
pub async fn file_url(state: &AppState, file_id: i64, kind: ObjectType) -> ApiResult<String> {
    let (key, _) = library_files::get_object(state, file_id, kind.as_str())
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
    let (rows, has_more) = collections::diff_page(state, collection_id, since).await?;
    Ok((rows.into_iter().map(for_public_diff).collect(), has_more))
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

/// Encrypted file data (only `vid_preview` playlists via public routes), read through
/// `actor`'s access (museum `filedata.Controller.GetFileData`). `Ok(None)` means 204.
pub async fn file_data(
    state: &AppState,
    actor: i64,
    q: &FileDataQuery,
) -> ApiResult<Option<filedata::Entity>> {
    filedata::fetch_one(state, actor, q.file_id, &q.kind, q.prefer_no_content).await
}

/// Signed URL for a video/image preview object (museum `GetPreviewUrl`).
pub async fn preview_url(
    state: &AppState,
    actor: i64,
    file_id: i64,
    kind: &str,
) -> ApiResult<String> {
    filedata::preview_url(state, actor, file_id, kind).await
}
