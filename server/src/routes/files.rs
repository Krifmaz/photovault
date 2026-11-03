//! Endpoints: /files/* (upload URLs, multipart, create/update, download, thumbnails, copy,
//! trash, size, info, duplicates, magic metadata, share-url, count).
//!
//! Reference: museum `cmd/museum/main.go`, `pkg/api/file.go`, `pkg/api/file_link.go` and
//! `pkg/controller/file.go`. The logic lives in [`crate::library`].

use crate::auth::{App, StorageUser};
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::library::errors;
use crate::library::files::{self, CopyRequest, MagicMetadataUpdate, MetaFile, TrashItem};
use crate::library::http::{Empty200, JsonBody, QueryMap, empty_json, header, path_i64};
use crate::library::links::{self, CreateFileUrlRequest, UpdateFileUrlRequest};
use crate::library::model::{File, FileAttributes};
use crate::library::objects::{
    self, MAX_FILE_SIZE, MAX_PARTS, MultipartUploadUrls, PURPOSE_FILE_UPLOAD, TempObject, URL_TTL,
    UploadUrl,
};
use crate::library::usage;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header as h};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

/// museum's `MaxUploadURLsLimit`.
const MAX_UPLOAD_URLS: i64 = 50;
/// museum's `DefaultMaxBatchSize`.
const MAX_BATCH: usize = 1000;
/// museum's `DefaultCopyBatchSize`.
const MAX_COPY_BATCH: usize = 100;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/files/upload-eligibility", get(upload_eligibility))
        .route("/files/upload-urls", get(upload_urls))
        .route("/files/multipart-upload-urls", get(multipart_upload_urls))
        .route("/files/upload-url", post(upload_url_v2))
        .route("/files/multipart-upload-url", post(multipart_upload_url_v2))
        .route("/files/download/{file_id}", get(download))
        .route("/files/download/v2/{file_id}", get(download_v2))
        .route("/files/download/v3/{file_id}", get(download_v3))
        .route("/files/preview/{file_id}", get(preview))
        .route("/files/preview/v2/{file_id}", get(preview_v2))
        .route("/files/thumbnail/v3/{file_id}", get(thumbnail_v3))
        .route(
            "/files/share-url",
            post(create_share_url)
                .get(list_share_urls)
                .put(update_share_url),
        )
        .route("/files/share-url/{file_id}", delete(disable_share_url))
        .route("/files/share-urls/", get(list_share_urls))
        .route("/files/share-urls", get(list_share_urls))
        .route("/files", post(create_or_update))
        .route("/files/meta", post(create_meta_file))
        .route("/files/copy", post(copy_files))
        .route("/files/update", put(update))
        .route("/files/trash", post(trash))
        .route("/files/size", post(size))
        .route("/files/info", post(info))
        .route("/files/duplicates", get(duplicates))
        .route("/files/thumbnail", put(update_thumbnail))
        .route("/files/magic-metadata", put(update_magic_metadata))
        .route(
            "/files/public-magic-metadata",
            put(update_public_magic_metadata),
        )
        .route("/files/count", get(count))
}

fn temp_object(user_id: i64, app: App, headers: &HeaderMap) -> TempObject {
    TempObject {
        object_key: objects::new_object_key(user_id),
        user_id: Some(user_id),
        app: Some(app.as_str().into()),
        purpose: Some(PURPOSE_FILE_UPLOAD.into()),
        client: Some(objects::client_info(headers)),
        ..Default::default()
    }
}

async fn upload_eligibility(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
) -> ApiResult<Empty200> {
    usage::can_upload(&state, u.user_id, None, u.app).await?;
    Ok(Empty200)
}

async fn upload_urls(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    usage::can_upload(&state, u.user_id, None, u.app).await?;
    let count = q.i64("count").min(MAX_UPLOAD_URLS);
    let mut urls: Vec<UploadUrl> = Vec::new();
    for _ in 0..count.max(0) {
        urls.push(objects::issue_put_url(&state, temp_object(u.user_id, u.app, &headers)).await?);
    }
    Ok(Json(json!({ "urls": urls })))
}

async fn multipart_upload_urls(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let count = q.i64("count");
    if count <= 0 || count > MAX_PARTS {
        return Err(errors::bad());
    }
    usage::can_upload(&state, u.user_id, None, u.app).await?;
    let urls = objects::issue_multipart(
        &state,
        temp_object(u.user_id, u.app, &headers),
        count as u32,
    )
    .await?;
    Ok(Json(json!({ "urls": urls })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UploadUrlRequest {
    #[serde(rename = "contentLength")]
    content_length: i64,
    #[serde(rename = "contentMD5")]
    content_md5: String,
}

async fn upload_url_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<UploadUrlRequest>,
) -> ApiResult<Json<UploadUrl>> {
    if req.content_length == 0 || req.content_md5.is_empty() {
        return Err(errors::bad());
    }
    if req.content_length < 0 || req.content_length > MAX_FILE_SIZE {
        return Err(errors::bad());
    }
    let md5 = objects::normalize_md5(&req.content_md5)?;
    usage::can_upload(&state, u.user_id, Some(req.content_length), u.app).await?;
    let mut t = temp_object(u.user_id, u.app, &headers);
    t.content_length = Some(req.content_length);
    t.content_md5 = Some(md5);
    Ok(Json(objects::issue_put_url(&state, t).await?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MultipartUploadUrlRequest {
    #[serde(rename = "contentLength")]
    content_length: i64,
    #[serde(rename = "partLength")]
    part_length: i64,
    #[serde(rename = "partMd5s")]
    part_md5s: Option<Vec<String>>,
}

async fn multipart_upload_url_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<MultipartUploadUrlRequest>,
) -> ApiResult<Json<MultipartUploadUrls>> {
    if req.content_length == 0 || req.part_length == 0 {
        return Err(errors::bad());
    }
    if req.content_length < 0 || req.content_length > MAX_FILE_SIZE {
        return Err(errors::bad());
    }
    if req.part_md5s.as_ref().is_some_and(|v| v.is_empty()) {
        return Err(errors::bad());
    }
    if objects::validate_part_length(req.content_length, req.part_length).is_err() {
        return Err(errors::bad());
    }
    let parts = (req.content_length - 1) / req.part_length + 1;
    if parts > MAX_PARTS {
        return Err(errors::bad());
    }
    if let Some(m) = &req.part_md5s {
        if m.len() as i64 != parts {
            return Err(errors::bad());
        }
        for c in m {
            objects::normalize_md5(c)?;
        }
    }
    usage::can_upload(&state, u.user_id, None, u.app).await?;
    let mut t = temp_object(u.user_id, u.app, &headers);
    t.content_length = Some(req.content_length);
    Ok(Json(
        objects::issue_multipart(&state, t, parts as u32).await?,
    ))
}

async fn signed_url(
    state: &AppState,
    actor: i64,
    file_id: &str,
    o_type: &str,
) -> ApiResult<String> {
    // museum ignores the parse error (fileID 0 → not found).
    let id: i64 = file_id.parse().unwrap_or(0);
    let key = files::accessible_object(state, id, actor, o_type).await?;
    state
        .store
        .presign_get(&key, URL_TTL)
        .await
        .map_err(errors::storage)
}

fn redirect(url: String) -> Response {
    (StatusCode::TEMPORARY_REDIRECT, [(h::LOCATION, url)]).into_response()
}

/// `fileURLV3Error`: a missing object is a 400 with code `NOT_FOUND`.
fn v3(res: ApiResult<String>) -> ApiResult<Json<serde_json::Value>> {
    match res {
        Ok(url) => Ok(Json(json!({ "url": url }))),
        Err(e) if e.status == StatusCode::NOT_FOUND => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "NOT_FOUND",
            "requested object was not found",
        )),
        Err(e) => Err(e),
    }
}

async fn download(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    Ok(redirect(
        signed_url(&state, u.user_id, &id, files::O_FILE).await?,
    ))
}

async fn download_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let url = signed_url(&state, u.user_id, &id, files::O_FILE).await?;
    Ok(Json(json!({ "url": url })))
}

async fn download_v3(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    v3(signed_url(&state, u.user_id, &id, files::O_FILE).await)
}

async fn preview(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    Ok(redirect(
        signed_url(&state, u.user_id, &id, files::O_THUMBNAIL).await?,
    ))
}

async fn preview_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let url = signed_url(&state, u.user_id, &id, files::O_THUMBNAIL).await?;
    Ok(Json(json!({ "url": url })))
}

async fn thumbnail_v3(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    v3(signed_url(&state, u.user_id, &id, files::O_THUMBNAIL).await)
}

async fn create_share_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<CreateFileUrlRequest>,
) -> ApiResult<Json<links::FileUrl>> {
    Ok(Json(
        links::create_file_link(&state, u.user_id, u.app, &req).await?,
    ))
}

async fn list_share_urls(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let since: i64 = q.str("sinceTime").parse().map_err(|_| errors::bad())?;
    let limit = if q.str("limit").is_empty() {
        500
    } else {
        match q.str("limit").parse::<i64>() {
            Ok(l) if l >= 1 => l,
            _ => return Err(errors::bad()),
        }
    };
    let diff = links::list_file_links(&state, u.user_id, u.app, since, limit).await?;
    Ok(Json(json!({ "diff": diff })))
}

async fn update_share_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<UpdateFileUrlRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let r = links::update_file_link(&state, u.user_id, u.app, &req).await?;
    Ok(Json(json!({ "result": r })))
}

async fn disable_share_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = path_i64(&id)?;
    links::disable_file_link(&state, u.user_id, id).await?;
    Ok(empty_json())
}

/// gin's `binding:"required"` on `file`, `thumbnail` and `metadata` (and their headers).
fn check_file_binding(f: &File) -> ApiResult<()> {
    let attrs: [&FileAttributes; 3] = [&f.file, &f.thumbnail, &f.metadata];
    if attrs.iter().any(|a| a.decryption_header.is_empty()) {
        return Err(errors::bad());
    }
    Ok(())
}

async fn create_or_update(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(mut file): JsonBody<File>,
) -> ApiResult<Response> {
    check_file_binding(&file)?;
    file.updation_time = now_micros();
    if file.id == 0 {
        file.owner_id = u.user_id;
        file.is_deleted = false;
        let f = files::create(&state, u.user_id, u.app, file).await?;
        return Ok(Json(f).into_response());
    }
    let (id, t) = files::update(&state, u.user_id, u.app, file).await?;
    Ok(Json(json!({ "id": id, "updationTime": t })).into_response())
}

async fn create_meta_file(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(m): JsonBody<MetaFile>,
) -> ApiResult<Json<File>> {
    if m.metadata.decryption_header.is_empty() {
        return Err(errors::bad());
    }
    if m.id != 0 {
        return Err(errors::bad());
    }
    Ok(Json(
        files::create_meta_file(&state, u.user_id, u.app, m).await?,
    ))
}

async fn copy_files(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<CopyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    if req.src_collection_id == 0 || req.dst_collection_id == 0 || req.files.is_none() {
        return Err(errors::bad());
    }
    if req.files.as_ref().is_some_and(|f| f.len() > MAX_COPY_BATCH) {
        return Err(errors::bad_msg(format!("more than {MAX_COPY_BATCH} items")));
    }
    let client = objects::client_info(&headers);
    let map = files::copy_files(&state, u.user_id, u.app, &client, &req).await?;
    Ok(Json(json!({ "oldToNewFileIDMap": map })))
}

async fn update(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(mut file): JsonBody<File>,
) -> ApiResult<Json<serde_json::Value>> {
    check_file_binding(&file)?;
    file.updation_time = now_micros();
    if file.id <= 0 {
        return Err(errors::bad());
    }
    let (id, t) = files::update(&state, u.user_id, u.app, file).await?;
    Ok(Json(json!({ "id": id, "updationTime": t })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TrashRequest {
    items: Option<Vec<TrashItem>>,
}

async fn trash(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<TrashRequest>,
) -> ApiResult<Empty200> {
    let items = req.items.ok_or_else(errors::bad)?;
    if items.iter().any(|i| i.file_id == 0 || i.collection_id == 0) {
        return Err(errors::bad());
    }
    if items.len() > MAX_BATCH {
        return Err(errors::too_large());
    }
    files::trash(&state, u.user_id, &items).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FileIdsRequest {
    #[serde(rename = "fileIDs")]
    file_ids: Option<Vec<i64>>,
}

/// museum's `shouldRejectRequest`: Android builds up to 0.5.35 get 426.
fn is_old_android(headers: &HeaderMap) -> bool {
    if !header(headers, "User-Agent")
        .to_lowercase()
        .contains("android")
    {
        return false;
    }
    if header(headers, "X-Client-Package") == "io.ente.photos.fdroid" {
        return false;
    }
    let v: Vec<&str> = header(headers, "X-Client-Version").split('.').collect();
    if v.len() != 3 || v[0] != "0" {
        return false;
    }
    match (v[1].parse::<i64>(), v[2].parse::<i64>()) {
        (Ok(minor), Ok(patch)) => minor <= 5 && patch <= 35,
        _ => false,
    }
}

async fn size(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<FileIdsRequest>,
) -> ApiResult<Response> {
    let ids = req.file_ids.ok_or_else(errors::bad)?;
    if is_old_android(&headers) {
        return Ok(StatusCode::UPGRADE_REQUIRED.into_response());
    }
    let size = files::total_size(&state, u.user_id, &ids).await?;
    Ok(Json(json!({ "size": size })).into_response())
}

async fn info(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<FileIdsRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let ids = req.file_ids.ok_or_else(errors::bad)?;
    let infos = files::files_info(&state, u.user_id, &ids).await?;
    let list: Vec<serde_json::Value> = infos
        .into_iter()
        .map(|(id, i)| json!({ "id": id, "fileInfo": i }))
        .collect();
    Ok(Json(json!({ "filesInfo": list })))
}

async fn duplicates(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
) -> ApiResult<Json<serde_json::Value>> {
    let d: Vec<serde_json::Value> = files::duplicates(&state, u.user_id)
        .await?
        .into_iter()
        .map(|(ids, size)| json!({ "fileIDs": ids, "size": size }))
        .collect();
    Ok(Json(json!({ "duplicates": d })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UpdateThumbnailRequest {
    #[serde(rename = "fileID")]
    file_id: i64,
    thumbnail: FileAttributes,
}

async fn update_thumbnail(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<UpdateThumbnailRequest>,
) -> ApiResult<Empty200> {
    if req.file_id == 0 || req.thumbnail.decryption_header.is_empty() {
        return Err(errors::bad());
    }
    files::update_thumbnail(&state, u.user_id, u.app, req.file_id, &req.thumbnail).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MagicMetadataRequest {
    #[serde(rename = "metadataList")]
    metadata_list: Option<Vec<MagicMetadataUpdate>>,
    #[serde(rename = "skipVersion")]
    skip_version: Option<bool>,
}

fn check_magic_list(req: &MagicMetadataRequest) -> ApiResult<&[MagicMetadataUpdate]> {
    let list = req.metadata_list.as_deref().ok_or_else(errors::bad)?;
    if list
        .iter()
        .any(|m| m.id == 0 || !m.magic_metadata.is_valid())
    {
        return Err(errors::bad());
    }
    Ok(list)
}

async fn update_magic_metadata(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MagicMetadataRequest>,
) -> ApiResult<Empty200> {
    let list = check_magic_list(&req)?;
    if list.len() > MAX_BATCH {
        return Err(errors::too_large());
    }
    files::update_magic_metadata(
        &state,
        u.user_id,
        list,
        false,
        req.skip_version.unwrap_or(false),
    )
    .await?;
    Ok(Empty200)
}

async fn update_public_magic_metadata(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MagicMetadataRequest>,
) -> ApiResult<Empty200> {
    let list = check_magic_list(&req)?;
    files::update_magic_metadata(
        &state,
        u.user_id,
        list,
        true,
        req.skip_version.unwrap_or(false),
    )
    .await?;
    Ok(Empty200)
}

async fn count(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let n = files::total_file_count(&state).await?;
    Ok(Json(json!({ "count": n })))
}
