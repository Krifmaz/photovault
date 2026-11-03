//! Endpoints: /cast/* (casting an album to a TV or browser).
//!
//! Reference: museum `cmd/museum/main.go` (castAPI), `pkg/api/cast.go`,
//! `pkg/controller/cast`, `pkg/repo/cast`, `pkg/middleware/cast_auth.go`.
//! museum registers several of these both with and without a trailing slash; so do we.

use crate::auth::StorageUser;
use crate::error::{ApiError, ApiResult};
use crate::library::collections;
use crate::public::cast::{self, CastSession};
use crate::public::files::{self, ObjectType, parse_file_id};
use crate::public::http::{
    PJson, client_ip, not_found_code, parse_uuid, url_redirect, url_v3, user_agent,
};
use crate::state::AppState;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const MAX_USER_AGENT_BYTES: usize = 1000;
const MAX_ENCRYPTED_PAYLOAD_BYTES: usize = 2 * 1024;
const PQ_PUBLIC_KEY_BYTES: usize = 1216;
const MAX_DEVICES: usize = 50;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/cast/device-info", post(register).get(list_devices))
        .route("/cast/device-info/", post(register))
        .route(
            "/cast/device-info/{code}",
            get(device_info).delete(delete_device),
        )
        .route("/cast/cast-data/{deviceCode}", get(cast_data))
        .route("/cast/cast-data", post(insert_cast_data))
        .route("/cast/cast-data/", post(insert_cast_data))
        .route("/cast/revoke-all-tokens", delete(revoke_all))
        .route("/cast/revoke-all-tokens/", delete(revoke_all))
        .route("/cast/files/preview/{fileID}", get(thumbnail))
        .route("/cast/files/thumbnail/v3/{fileID}", get(thumbnail_v3))
        .route("/cast/files/download/{fileID}", get(download))
        .route("/cast/files/download/v3/{fileID}", get(download_v3))
        .route("/cast/diff", get(diff))
        .route("/cast/info", get(info))
}

/// Caller IP and user agent.
struct Client {
    ip: String,
    ua: String,
}

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Client {
            ip: client_ip(parts),
            ua: user_agent(parts),
        })
    }
}

/// museum `cast.RegisterDeviceRequest`.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RegisterRequest {
    public_key: String,
    pq_public_key: Option<String>,
}

async fn register(
    State(state): State<AppState>,
    client: Client,
    PJson(req): PJson<RegisterRequest>,
) -> ApiResult<Json<Value>> {
    if req.public_key.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if let Some(pq) = &req.pq_public_key {
        let expected_len = PQ_PUBLIC_KEY_BYTES.div_ceil(3) * 4;
        let decoded = base64::engine::general_purpose::STANDARD.decode(pq);
        if pq.len() != expected_len || decoded.map(|d| d.len()).ok() != Some(PQ_PUBLIC_KEY_BYTES) {
            return Err(ApiError::status(StatusCode::BAD_REQUEST));
        }
    }
    if client.ua.len() > MAX_USER_AGENT_BYTES {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            "user agent too long",
        ));
    }
    let device_name = cast::device_type(&client.ua)
        .map(str::to_string)
        .unwrap_or_else(|| client.ip.clone());
    let db = &state.db.pool;
    if let Err(e) = cast::cleanup(db).await {
        tracing::warn!(error = %e, "cast cleanup failed");
    }
    let code = cast::add_code(
        db,
        &req.public_key,
        req.pq_public_key.as_deref(),
        &client.ip,
        &device_name,
    )
    .await?;
    Ok(Json(json!({ "deviceCode": code })))
}

async fn list_devices(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
) -> ApiResult<Json<Value>> {
    let devices = cast::devices_for_user(&state.db.pool, user.user_id).await?;
    Ok(Json(json!({ "devices": devices })))
}

async fn device_info(
    State(state): State<AppState>,
    StorageUser(_user): StorageUser,
    client: Client,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    let (public_key, pq, ip) = cast::device_info(&state.db.pool, &code)
        .await?
        .ok_or_else(|| not_found_code("code not found"))?;
    if ip != client.ip {
        tracing::warn!(device_ip = %ip, client_ip = %client.ip, "cast device info: IP mismatch");
    }
    let mut body = json!({ "publicKey": public_key });
    if let Some(pq) = pq {
        body["pqPublicKey"] = Value::from(pq);
    }
    Ok(Json(body))
}

async fn delete_device(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
    Path(raw): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_uuid(&raw).ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST))?;
    cast::revoke_device(&state.db.pool, user.user_id, &id).await?;
    Ok(Json(json!({})))
}

async fn cast_data(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    let payload = cast::enc_cast_data(&state.db.pool, &code)
        .await?
        .ok_or_else(|| not_found_code("active code not found"))?;
    Ok(Json(json!({ "encCastData": payload })))
}

/// museum `cast.CastRequest`.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct CastRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    cast_token: String,
    enc_payload: String,
    device_code: String,
}

async fn insert_cast_data(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
    PJson(req): PJson<CastRequest>,
) -> ApiResult<Json<Value>> {
    if req.collection_id == 0
        || req.cast_token.is_empty()
        || req.enc_payload.is_empty()
        || req.device_code.is_empty()
    {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if req.enc_payload.len() > MAX_ENCRYPTED_PAYLOAD_BYTES {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let db = &state.db.pool;
    if cast::devices_for_user(db, user.user_id).await?.len() >= MAX_DEVICES {
        return Err(ApiError::internal("cast device limit reached"));
    }
    let id = cast::insert_cast_data(
        db,
        user.user_id,
        &req.device_code,
        req.collection_id,
        &req.cast_token,
        &req.enc_payload,
    )
    .await?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "CONFLICT",
            "cast code unavailable or already paired",
        )
    })?;
    Ok(Json(json!({ "deviceID": id })))
}

async fn revoke_all(
    State(state): State<AppState>,
    StorageUser(user): StorageUser,
) -> ApiResult<Json<Value>> {
    cast::revoke_for_user(&state.db.pool, user.user_id).await?;
    Ok(Json(json!({})))
}

async fn file_url(
    state: &AppState,
    session: &CastSession,
    raw_id: &str,
    kind: ObjectType,
) -> ApiResult<String> {
    let file_id = parse_file_id(raw_id)?;
    files::collection_file_url(state, session.collection_id, file_id, kind).await
}

async fn thumbnail(
    State(state): State<AppState>,
    session: CastSession,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &session, &id, ObjectType::Thumbnail).await)
}

async fn download(
    State(state): State<AppState>,
    session: CastSession,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &session, &id, ObjectType::File).await)
}

async fn thumbnail_v3(
    State(state): State<AppState>,
    session: CastSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &session, &id, ObjectType::Thumbnail).await)
}

async fn download_v3(
    State(state): State<AppState>,
    session: CastSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &session, &id, ObjectType::File).await)
}

async fn diff(
    State(state): State<AppState>,
    session: CastSession,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    // museum propagates the strconv error untyped → 500 `{}`.
    let since: i64 = q
        .get("sinceTime")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ApiError::internal("invalid sinceTime"))?;
    let (diff, has_more) = files::public_diff(&state, session.collection_id, since).await?;
    Ok(Json(json!({ "diff": diff, "hasMore": has_more })))
}

async fn info(State(state): State<AppState>, session: CastSession) -> ApiResult<Json<Value>> {
    let (_, mut c) = collections::get(&state, session.collection_id).await?;
    if c.is_deleted {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    c.magic_metadata = None;
    c.public_urls = Some(vec![]);
    Ok(Json(json!({ "collection": c })))
}
