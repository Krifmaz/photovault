//! Endpoints: /file-link/* (public single-file links).
//!
//! Reference: museum `cmd/museum/main.go` (fileLinkApi), `pkg/api/file_link.go`,
//! `pkg/middleware/file_link.go`, `pkg/controller/public/file_link.go`.
//! Creating and managing these links (`/files/share-url`) is part of the files routes.

use crate::error::{ApiError, ApiResult};
use crate::library::links;
use crate::public::files::{self, ObjectType};
use crate::public::http::{PJson, url_redirect, url_v3};
use crate::public::library_sql;
use crate::public::links::FileLink;
use crate::routes::public_collection::{VerifyPasswordRequest, verify_link_password};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Map, Value, json};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/file-link/info", get(info))
        .route("/file-link/pass-info", get(pass_info))
        .route("/file-link/thumbnail", get(thumbnail))
        .route("/file-link/thumbnail/v3", get(thumbnail_v3))
        .route("/file-link/file", get(file))
        .route("/file-link/file/v3", get(file_v3))
        .route("/file-link/verify-password", post(verify_password))
}

fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}

async fn info(State(state): State<AppState>, link: FileLink) -> ApiResult<Response> {
    let file = library_sql::file_attributes(&state, link.file_id)
        .await?
        .ok_or_else(not_found)?;
    let row = links::active_file_link(&state, link.file_id)
        .await?
        .ok_or_else(not_found)?;
    // museum `ente.FileLinkSecret`: every field is omitted when unset.
    let mut secret = Map::new();
    let mut put = |k: &str, v: Option<Value>| {
        if let Some(v) = v {
            secret.insert(k.into(), v);
        }
    };
    put("encryptedFileKey", row.encrypted_file_key.map(Value::from));
    put(
        "encryptedFileKeyNonce",
        row.encrypted_file_key_nonce.map(Value::from),
    );
    put("kdfNonce", row.kdf_nonce.map(Value::from));
    put("kdfMemLimit", row.kdf_mem_limit.map(Value::from));
    put("kdfOpsLimit", row.kdf_ops_limit.map(Value::from));
    let mut res = Json(json!({ "file": file, "link": secret })).into_response();
    if let Some(t) = &link.device_token
        && let Ok(v) = HeaderValue::from_str(t)
    {
        res.headers_mut().insert("X-Link-Device-Token", v);
    }
    Ok(res)
}

async fn pass_info(State(state): State<AppState>, link: FileLink) -> ApiResult<Json<Value>> {
    let row = links::active_file_link(&state, link.file_id)
        .await?
        .ok_or_else(not_found)?;
    Ok(Json(json!({
        "nonce": row.pw_nonce,
        "opsLimit": row.ops_limit,
        "memLimit": row.mem_limit,
    })))
}

async fn url(state: &AppState, link: &FileLink, kind: ObjectType) -> ApiResult<String> {
    files::owned_file_url(state, link.owner_id, link.file_id, kind).await
}

async fn thumbnail(State(state): State<AppState>, link: FileLink) -> Response {
    url_redirect(url(&state, &link, ObjectType::Thumbnail).await)
}

async fn file(State(state): State<AppState>, link: FileLink) -> Response {
    url_redirect(url(&state, &link, ObjectType::File).await)
}

async fn thumbnail_v3(State(state): State<AppState>, link: FileLink) -> ApiResult<Json<Value>> {
    url_v3(url(&state, &link, ObjectType::Thumbnail).await)
}

async fn file_v3(State(state): State<AppState>, link: FileLink) -> ApiResult<Json<Value>> {
    url_v3(url(&state, &link, ObjectType::File).await)
}

async fn verify_password(
    State(state): State<AppState>,
    link: FileLink,
    PJson(req): PJson<VerifyPasswordRequest>,
) -> ApiResult<Json<Value>> {
    if req.pass_hash.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let row = links::active_file_link(&state, link.file_id)
        .await?
        .ok_or_else(not_found)?;
    verify_link_password(&state, row.pw_hash.as_deref(), &req.pass_hash)
}
