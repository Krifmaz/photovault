//! Endpoints: /files/data/*, /files/video-data (derived file data: ML data, previews).
//!
//! Reference: museum `pkg/api/file_data.go` and `pkg/controller/filedata`. The logic lives
//! in [`crate::library::filedata`].

use crate::auth::StorageUser;
use crate::error::{ApiError, ApiResult};
use crate::library::errors;
use crate::library::filedata::{
    self, FetchRequest, MultipartPreviewUploadRequest, PreviewUploadRequest, PreviewUploadUrl,
    PutRequest, VideoRequest,
};
use crate::library::http::{JsonBody, QueryMap, empty_json};
use crate::library::objects;
use crate::state::AppState;
use axum::extract::{FromRequest, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde_json::json;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/files/data", put(put_file_data))
        .route("/files/video-data", put(put_video_data))
        .route("/files/data/status-diff", post(status_diff))
        .route("/files/data/fetch", post(fetch_many).get(fetch_one))
        .route(
            "/files/data/preview-upload-url",
            get(preview_upload_url).post(preview_upload_url_v2),
        )
        .route(
            "/files/data/multipart-preview-upload-url",
            post(multipart_preview_upload_url),
        )
        .route("/files/data/preview", get(preview_url))
}

/// A JSON body whose parse failure is `400 {"code":"BAD_REQUEST","message":…}`
/// (museum's file data handlers wrap bind errors with `NewBadRequestWithMessage`).
struct CodedJson<T>(T);

impl<T: DeserializeOwned> FromRequest<AppState> for CodedJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|_| errors::bad_msg("invalid request body"))?;
        serde_json::from_slice(&bytes)
            .map(CodedJson)
            .map_err(|e| errors::bad_msg(e.to_string()))
    }
}

async fn put_file_data(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    CodedJson(req): CodedJson<PutRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let client = objects::client_info(&headers);
    filedata::put(&state, u.user_id, &client, &req).await?;
    Ok(empty_json())
}

async fn put_video_data(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    CodedJson(req): CodedJson<VideoRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let client = objects::client_info(&headers);
    filedata::put_video(&state, u.user_id, &client, &req).await?;
    Ok(empty_json())
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct DiffRequest {
    #[serde(rename = "lastUpdatedAt", alias = "LastUpdatedAt")]
    last_updated_at: Option<i64>,
}

async fn status_diff(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    CodedJson(req): CodedJson<DiffRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let since = match req.last_updated_at {
        Some(v) if v >= 0 => v,
        _ => {
            return Err(errors::bad_msg(
                "lastUpdated is required and should be greater than or equal to 0",
            ));
        }
    };
    let diff = filedata::status_diff(&state, u.user_id, since).await?;
    Ok(Json(json!({ "diff": diff })))
}

async fn fetch_many(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    CodedJson(req): CodedJson<FetchRequest>,
) -> ApiResult<Json<filedata::FetchResponse>> {
    Ok(Json(filedata::fetch(&state, u.user_id, &req).await?))
}

async fn fetch_one(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Response> {
    let file_id = q.opt_i64("fileID").filter(|v| *v != 0);
    let kind = q.str("type");
    let (Some(file_id), false) = (file_id, kind.is_empty()) else {
        return Err(errors::bad_msg("fileID and type are required"));
    };
    let prefer = q
        .opt_bool("preferNoContent")
        .map_err(|_| errors::bad_msg("invalid preferNoContent"))?
        .unwrap_or(false);
    match filedata::fetch_one(&state, u.user_id, file_id, kind, prefer).await? {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(e) => Ok(Json(json!({ "data": e })).into_response()),
    }
}

async fn preview_upload_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    q: QueryMap,
) -> ApiResult<Json<PreviewUploadUrl>> {
    // gin's ShouldBindQuery failures are ErrBadRequest → 400 `{}`.
    let file_id = q
        .opt_i64("fileID")
        .filter(|v| *v != 0)
        .ok_or_else(errors::bad)?;
    let kind = q.str("type");
    if kind.is_empty() {
        return Err(errors::bad());
    }
    let multipart = q.opt_bool("isMultiPart")?.unwrap_or(false);
    let count = match q.str("count") {
        "" => None,
        s => Some(s.parse::<i64>().map_err(|_| errors::bad())?),
    };
    let client = objects::client_info(&headers);
    Ok(Json(
        filedata::preview_upload_url(
            &state, u.user_id, u.app, &client, file_id, kind, multipart, count,
        )
        .await?,
    ))
}

async fn preview_upload_url_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<PreviewUploadRequest>,
) -> ApiResult<Json<PreviewUploadUrl>> {
    let client = objects::client_info(&headers);
    Ok(Json(
        filedata::preview_upload_url_v2(&state, u.user_id, u.app, &client, &req).await?,
    ))
}

async fn multipart_preview_upload_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    JsonBody(req): JsonBody<MultipartPreviewUploadRequest>,
) -> ApiResult<Json<PreviewUploadUrl>> {
    let client = objects::client_info(&headers);
    Ok(Json(
        filedata::multipart_preview_upload_url(&state, u.user_id, u.app, &client, &req).await?,
    ))
}

async fn preview_url(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let file_id = q
        .opt_i64("fileID")
        .filter(|v| *v != 0)
        .ok_or_else(errors::bad)?;
    let kind = q.str("type");
    if kind.is_empty() {
        return Err(errors::bad());
    }
    let url = filedata::preview_url(&state, u.user_id, file_id, kind).await?;
    Ok(Json(json!({ "url": url })))
}
