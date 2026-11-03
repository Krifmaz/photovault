//! Endpoints: /memory-share (owner) and /public-memory/* (viewers).
//!
//! Reference: museum `cmd/museum/main.go`, `pkg/api/memory_share.go`,
//! `pkg/api/public_memory_share.go`, `pkg/controller/memory_share`,
//! `pkg/controller/public/memory_share.go`, `pkg/middleware/memory_share.go`.

use crate::auth::StorageUser;
use crate::error::{ApiError, ApiResult};
use crate::library::files as library_files;
use crate::public::files::{self, ObjectType, parse_file_id};
use crate::public::http::{PJson, bad_request_msg, new_access_token, url_redirect, url_v3};
use crate::public::library_sql;
use crate::public::links::MemoryShareLink;
use crate::public::memory::{self, CreateError, MEMORY_SHARE_TTL, ShareFile};
use crate::public::models::MemoryShare;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/memory-share", get(list).post(create))
        .route("/memory-share/{shareID}", get(get_one).delete(delete))
        .route("/public-memory/info", get(public_info))
        .route("/public-memory/files", get(public_files))
        .route("/public-memory/files/preview/{fileID}", get(thumbnail))
        .route(
            "/public-memory/files/thumbnail/v3/{fileID}",
            get(thumbnail_v3),
        )
        .route("/public-memory/files/download/{fileID}", get(download))
        .route(
            "/public-memory/files/download/v3/{fileID}",
            get(download_v3),
        )
        .route("/public-memory/file-data", get(file_data))
        .route("/public-memory/files/data/preview", get(preview_url))
}

/// museum `ente.CreateMemoryShareRequest`.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateRequest {
    /// `share` (default) or `lane`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Client memory hash.
    pub memory_hash: String,
    /// Encrypted metadata.
    pub metadata_cipher: String,
    /// Nonce for metadata.
    pub metadata_nonce: String,
    /// Share key, encrypted.
    pub encrypted_key: String,
    /// Nonce for the share key.
    pub key_decryption_nonce: String,
    /// Files (at least one).
    pub files: Option<Vec<CreateFileItem>>,
}

/// museum `ente.MemoryShareFileItem`.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateFileItem {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Explicit position (defaults to the index).
    pub position: Option<i64>,
    /// File key encrypted with the share key.
    pub encrypted_key: String,
    /// Nonce for that.
    pub key_decryption_nonce: String,
}

fn bad() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

async fn create(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
    PJson(req): PJson<CreateRequest>,
) -> ApiResult<Json<Value>> {
    let items = req.files.as_deref().unwrap_or_default();
    if req.encrypted_key.is_empty() || req.key_decryption_nonce.is_empty() || items.is_empty() {
        return Err(bad());
    }
    let kind = if req.kind.is_empty() {
        "share".to_string()
    } else {
        req.kind.clone()
    };
    if kind != "share" && kind != "lane" {
        return Err(bad());
    }
    let file_ids: Vec<i64> = items.iter().map(|f| f.file_id).collect();
    if file_ids.iter().collect::<HashSet<_>>().len() != file_ids.len() {
        return Err(bad());
    }
    library_files::can_access_files(&state, user.user_id, &file_ids).await?;
    let owners: HashMap<i64, i64> = library_files::owner_to_file_ids(&state, &file_ids)
        .await?
        .into_iter()
        .flat_map(|(owner, ids)| ids.into_iter().map(move |f| (f, owner)))
        .collect();
    if owners.len() != file_ids.len() {
        return Err(bad());
    }
    let mut share_files = Vec::with_capacity(items.len());
    for (i, f) in items.iter().enumerate() {
        let position = match f.position {
            Some(p) if p < 0 => return Err(bad()),
            Some(p) => p,
            None => i as i64,
        };
        share_files.push(ShareFile {
            file_id: f.file_id,
            file_owner_id: owners.get(&f.file_id).copied().unwrap_or_default(),
            position,
            encrypted_key: f.encrypted_key.clone(),
            key_decryption_nonce: f.key_decryption_nonce.clone(),
        });
    }
    for _ in 0..5 {
        let share = MemoryShare {
            user_id: user.user_id,
            kind: kind.clone(),
            memory_hash: req.memory_hash.clone(),
            metadata_cipher: req.metadata_cipher.clone(),
            metadata_nonce: req.metadata_nonce.clone(),
            encrypted_key: req.encrypted_key.clone(),
            key_decryption_nonce: req.key_decryption_nonce.clone(),
            access_token: new_access_token(),
            ..Default::default()
        };
        match memory::create(&state, share, &share_files).await {
            Ok(share) => return Ok(Json(json!({ "memoryShare": share }))),
            Err(CreateError::TokenInUse) => continue,
            Err(CreateError::Db(e)) => return Err(e.into()),
        }
    }
    Err(ApiError::internal("failed to generate unique access token"))
}

async fn list(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
) -> ApiResult<Json<Value>> {
    let shares = memory::for_user(&state, user.user_id).await?;
    Ok(Json(json!({ "memoryShares": shares })))
}

fn parse_share_id(raw: &str) -> ApiResult<i64> {
    raw.parse().map_err(|_| bad())
}

async fn get_one(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
    Path(raw): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_share_id(&raw)?;
    let share = memory::by_id(&state, id)
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    if share.user_id != user.user_id {
        return Err(ApiError::status(StatusCode::FORBIDDEN));
    }
    if share.is_deleted || share.created_at + MEMORY_SHARE_TTL <= crate::db::now_micros() {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(Json(json!({ "memoryShare": share })))
}

async fn delete(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
    Path(raw): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_share_id(&raw)?;
    if !memory::delete(&state.db.pool, id, user.user_id).await? {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(Json(json!({})))
}

async fn public_info(link: MemoryShareLink) -> Json<Value> {
    Json(json!({ "memoryShare": link.share }))
}

async fn public_files(
    State(state): State<AppState>,
    link: MemoryShareLink,
) -> ApiResult<Json<Value>> {
    let mut out = Vec::new();
    for sf in memory::files(&state.db.pool, link.share.id).await? {
        // Skip files that left every collection, were deleted, or are placeholders.
        if library_sql::active_collection_count(&state, sf.file_id).await? == 0 {
            continue;
        }
        let Some(mut file) = library_sql::file_attributes(&state, sf.file_id).await? else {
            continue;
        };
        if file.metadata.encrypted_data == "-" {
            continue;
        }
        file.magic_metadata = None;
        out.push(json!({
            "file": file,
            "position": sf.position,
            "encryptedKey": sf.encrypted_key,
            "keyDecryptionNonce": sf.key_decryption_nonce,
        }));
    }
    Ok(Json(json!({ "files": out })))
}

/// museum `GetAccessibleFileOwnerID`: the file is in the share and still in some collection.
async fn accessible_owner(state: &AppState, share_id: i64, file_id: i64) -> ApiResult<i64> {
    let db = &state.db.pool;
    let owner = memory::file_owner_in_share(db, share_id, file_id)
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    if library_sql::active_collection_count(state, file_id).await? == 0 {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(owner)
}

async fn file_url(
    state: &AppState,
    link: &MemoryShareLink,
    raw_id: &str,
    kind: ObjectType,
) -> ApiResult<String> {
    let file_id = parse_file_id(raw_id)?;
    accessible_owner(state, link.share.id, file_id).await?;
    let file = library_sql::file_attributes(state, file_id)
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    if file.metadata.encrypted_data == "-" {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    files::file_url(state, file_id, kind).await
}

async fn thumbnail(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &link, &id, ObjectType::Thumbnail).await)
}

async fn download(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &link, &id, ObjectType::File).await)
}

async fn thumbnail_v3(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &link, &id, ObjectType::Thumbnail).await)
}

async fn download_v3(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &link, &id, ObjectType::File).await)
}

async fn file_data(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let q = files::parse_file_data_query(&q).map_err(bad_request_msg)?;
    if q.kind != "vid_preview" {
        return Err(bad_request_msg("unsupported object type"));
    }
    let owner = accessible_owner(&state, link.share.id, q.file_id).await?;
    Ok(match files::file_data(&state, owner, &q).await? {
        Some(data) => Json(json!({ "data": data })).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn preview_url(
    State(state): State<AppState>,
    link: MemoryShareLink,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let (file_id, kind) =
        files::parse_preview_query(&q).ok_or_else(|| bad_request_msg("invalid preview request"))?;
    let owner = accessible_owner(&state, link.share.id, file_id).await?;
    let url = files::preview_url(&state, owner, file_id, &kind).await?;
    Ok(Json(json!({ "url": url })))
}
