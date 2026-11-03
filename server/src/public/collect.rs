//! Public "collect" uploads: viewers of an album link with `enableCollect` upload
//! files into the album, owned by and charged to the album owner
//! (museum `GetUploadURLWithMetadata`, `GetMultipartUploadURLWithMetadata`, `FileController.Create`).

use super::http::{SIGNED_URL_TTL, bad_request_msg};
use super::library_sql::{self, CreateFileError};
use super::models::{File, FileInfo};
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

/// museum `MaxFileSize` (10 GiB).
pub const MAX_FILE_SIZE: i64 = 10 * 1024 * 1024 * 1024;
const MIN_PART: i64 = 5 << 20;
const MAX_PART: i64 = 5 << 30;
const MAX_PARTS: i64 = 10_000;

/// museum `ente.UploadURLRequest`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadUrlRequest {
    /// Exact object size.
    #[serde(default)]
    pub content_length: i64,
    /// Base64 MD5 of the object.
    #[serde(rename = "contentMD5", default)]
    pub content_md5: String,
}

/// museum `ente.MultipartUploadURLRequest`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartUploadUrlRequest {
    /// Total size.
    #[serde(default)]
    pub content_length: i64,
    /// Size of every part but the last.
    #[serde(default)]
    pub part_length: i64,
    /// Base64 MD5 of each part.
    #[serde(rename = "partMd5s", default)]
    pub part_md5s: Option<Vec<String>>,
}

fn storage_limit_exceeded() -> ApiError {
    // museum ErrStorageLimitExceeded → 426 with `{}`.
    ApiError::status(StatusCode::UPGRADE_REQUIRED)
}

/// museum `ente.NormalizeMD5`.
fn normalize_md5(v: &str) -> ApiResult<String> {
    let bytes = STANDARD
        .decode(v.trim())
        .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))?;
    if bytes.len() != 16 {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    Ok(STANDARD.encode(bytes))
}

async fn ensure_quota(state: &AppState, owner_id: i64, size: Option<i64>) -> ApiResult<()> {
    let available = library_sql::storage_available(&state.db.pool, owner_id).await?;
    if available <= 0 || size.is_some_and(|s| s > available) {
        return Err(storage_limit_exceeded());
    }
    Ok(())
}

fn new_object_key(owner_id: i64) -> String {
    format!("{owner_id}/{}", super::http::new_uuid())
}

/// `{"objectKey","url"}` for a single-PUT upload charged to `owner_id`.
pub async fn upload_url(
    state: &AppState,
    owner_id: i64,
    req: &UploadUrlRequest,
) -> ApiResult<serde_json::Value> {
    if req.content_length == 0 || req.content_md5.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if req.content_length < 0 || req.content_length > MAX_FILE_SIZE {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let md5 = normalize_md5(&req.content_md5)?;
    ensure_quota(state, owner_id, Some(req.content_length)).await?;
    let key = new_object_key(owner_id);
    let url = state
        .store
        .presign_put(&key, SIGNED_URL_TTL, Some(req.content_length), Some(&md5))
        .await?;
    Ok(serde_json::json!({"objectKey": key, "url": url}))
}

/// `{"objectKey","partURLs","completeURL"}` for a multipart upload charged to `owner_id`.
pub async fn multipart_upload_url(
    state: &AppState,
    owner_id: i64,
    req: &MultipartUploadUrlRequest,
) -> ApiResult<serde_json::Value> {
    if req.content_length == 0 || req.part_length == 0 {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if req.content_length < 0 || req.content_length > MAX_FILE_SIZE {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if req.part_length <= 0 || req.part_length > MAX_PART {
        return Err(bad_request_msg(
            "partLength must be between 1 byte and 5 GiB",
        ));
    }
    if req.content_length > req.part_length && req.part_length < MIN_PART {
        return Err(bad_request_msg(
            "partLength must be at least 5 MiB when more than one part is required",
        ));
    }
    let parts = (req.content_length + req.part_length - 1) / req.part_length;
    if parts > MAX_PARTS {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if let Some(md5s) = &req.part_md5s {
        if md5s.len() as i64 != parts {
            return Err(ApiError::status(StatusCode::BAD_REQUEST));
        }
        for m in md5s {
            normalize_md5(m)?;
        }
    }
    ensure_quota(state, owner_id, None).await?;
    let key = new_object_key(owner_id);
    let mp = state
        .store
        .create_multipart(&key, parts as u32, SIGNED_URL_TTL)
        .await?;
    Ok(serde_json::json!({
        "objectKey": mp.object_key,
        "partURLs": mp.part_urls,
        "completeURL": mp.complete_url,
    }))
}

/// Create `file` in the owner's collection from objects already uploaded
/// (museum `CollectionLinkController.CreateFile` → `FileController.Create`).
pub async fn create_file(
    state: &AppState,
    owner_id: i64,
    collection_app: &str,
    app: &str,
    mut file: File,
) -> ApiResult<File> {
    // Public links may only create, never update, and always as the owner.
    file.id = 0;
    file.owner_id = owner_id;
    file.updation_time = now_micros();
    file.is_deleted = false;

    let prefix = format!("{owner_id}/");
    if !file.file.object_key.starts_with(&prefix) || !file.thumbnail.object_key.starts_with(&prefix)
    {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if file.file.object_key == file.thumbnail.object_key {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if file.encrypted_key.is_empty() || file.key_decryption_nonce.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if file.file.decryption_header.is_empty() || file.thumbnail.decryption_header.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if collection_app != app {
        // museum ErrInvalidApp is an untyped error → 500 `{}`.
        return Err(ApiError::internal("collection app mismatch"));
    }
    let object_size_failed = || {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OBJECT_SIZE_FETCH_FAILED",
            "",
        )
    };
    let file_size = state
        .store
        .head(&file.file.object_key)
        .await
        .map_err(|_| object_size_failed())?
        .ok_or_else(object_size_failed)? as i64;
    let thumb_size = state
        .store
        .head(&file.thumbnail.object_key)
        .await
        .map_err(|_| object_size_failed())?
        .ok_or_else(object_size_failed)? as i64;
    if file_size > MAX_FILE_SIZE {
        return Err(ApiError::status(StatusCode::PAYLOAD_TOO_LARGE));
    }
    if file.file.size != 0 && file.file.size != file_size {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if file.thumbnail.size != 0 && file.thumbnail.size != thumb_size {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    file.file.size = file_size;
    file.thumbnail.size = thumb_size;
    ensure_quota(state, owner_id, Some(file_size + thumb_size)).await?;
    file.info = Some(FileInfo {
        file_size,
        thumb_size,
    });
    file.action = None;
    file.action_user = None;
    match library_sql::create_file(&state.db.pool, &mut file, app).await {
        Ok(()) => Ok(file),
        // museum reconciles duplicate objects with an existing file of the same owner;
        // a public uploader never legitimately reuses keys, so refuse.
        Err(CreateFileError::DuplicateObject) => Err(ApiError::status(StatusCode::BAD_REQUEST)),
        Err(CreateFileError::Db(e)) => Err(e.into()),
    }
}
