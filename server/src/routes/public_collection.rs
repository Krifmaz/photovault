//! Endpoints: /public-collection/* (public album links).
//!
//! Reference: museum `cmd/museum/main.go` (publicCollectionAPI), `pkg/api/public_collection.go`,
//! `pkg/middleware/collection_link.go`, `pkg/controller/public/collection_link.go`.
//! The social endpoints under this prefix (comments, reactions, anon identity) belong to
//! the social routes, which can authenticate with [`CollectionLink`].

use crate::error::{ApiError, ApiResult};
use crate::public::collect::{self, MultipartUploadUrlRequest, UploadUrlRequest};
use crate::public::files::{self, ObjectType, parse_file_id};
use crate::public::http::{PJson, bad_request_msg, url_redirect, url_v3};
use crate::public::jwt;
use crate::public::library_sql;
use crate::public::links::CollectionLink;
use crate::public::models::{Collection, File};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/public-collection/files/preview/{fileID}", get(thumbnail))
        .route(
            "/public-collection/files/thumbnail/v3/{fileID}",
            get(thumbnail_v3),
        )
        .route("/public-collection/files/download/{fileID}", get(download))
        .route(
            "/public-collection/files/download/v3/{fileID}",
            get(download_v3),
        )
        .route("/public-collection/files/data/fetch", get(file_data))
        .route("/public-collection/files/data/preview", get(preview_url))
        .route("/public-collection/diff", get(diff))
        .route("/public-collection/info", get(info))
        .route("/public-collection/upload-url", post(upload_url))
        .route(
            "/public-collection/multipart-upload-url",
            post(multipart_upload_url),
        )
        .route("/public-collection/file", post(create_file))
        .route("/public-collection/verify-password", post(verify_password))
}

/// museum `GetPublicCollection`: the shared collection without private metadata;
/// with `must_allow_collect`, also requires the link to permit uploads.
pub async fn public_collection(
    state: &AppState,
    link: &CollectionLink,
    must_allow_collect: bool,
) -> ApiResult<Collection> {
    let mut c = library_sql::collection(&state.db.pool, link.collection_id)
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    if c.is_deleted {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    c.sharees = None;
    c.magic_metadata = None;
    if must_allow_collect {
        let urls = c.public_urls.as_deref().unwrap_or_default();
        if urls.len() != 1 {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                format!("Unexpected number of public urls: {}", urls.len()),
            ));
        }
        if !urls[0].enable_collect {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "PUBLIC_COLLECT_DISABLED",
                "User has not enabled public collect for this url",
            ));
        }
    }
    Ok(c)
}

async fn file_url(
    state: &AppState,
    link: &CollectionLink,
    raw_id: &str,
    kind: ObjectType,
) -> ApiResult<String> {
    let file_id = parse_file_id(raw_id)?;
    files::collection_file_url(state, link.collection_id, file_id, kind).await
}

async fn thumbnail(
    State(state): State<AppState>,
    link: CollectionLink,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &link, &id, ObjectType::Thumbnail).await)
}

async fn download(
    State(state): State<AppState>,
    link: CollectionLink,
    Path(id): Path<String>,
) -> Response {
    url_redirect(file_url(&state, &link, &id, ObjectType::File).await)
}

async fn thumbnail_v3(
    State(state): State<AppState>,
    link: CollectionLink,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &link, &id, ObjectType::Thumbnail).await)
}

async fn download_v3(
    State(state): State<AppState>,
    link: CollectionLink,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    url_v3(file_url(&state, &link, &id, ObjectType::File).await)
}

/// museum `getCollectionOwnerAndVerifyAccess`.
async fn owner_for_file(state: &AppState, link: &CollectionLink, file_id: i64) -> ApiResult<i64> {
    if !library_sql::file_active_in_collection(&state.db.pool, link.collection_id, file_id).await? {
        return Err(ApiError::status(StatusCode::FORBIDDEN));
    }
    Ok(public_collection(state, link, false).await?.owner.id)
}

async fn file_data(
    State(state): State<AppState>,
    link: CollectionLink,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let q = files::parse_file_data_query(&q).map_err(bad_request_msg)?;
    if q.kind != "vid_preview" {
        return Err(bad_request_msg(format!(
            "unsupported object type {}",
            q.kind
        )));
    }
    let owner = owner_for_file(&state, &link, q.file_id).await?;
    Ok(match files::file_data(&state, owner, &q).await? {
        Some(data) => Json(json!({ "data": data })).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn preview_url(
    State(state): State<AppState>,
    link: CollectionLink,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let (file_id, kind) =
        files::parse_preview_query(&q).ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST))?;
    let owner = owner_for_file(&state, &link, file_id).await?;
    let url = files::preview_url(&state, owner, file_id, &kind).await?;
    Ok(Json(json!({ "url": url })))
}

async fn diff(
    State(state): State<AppState>,
    link: CollectionLink,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let raw = q.get("sinceTime").cloned().unwrap_or_default();
    let since: i64 = raw
        .parse()
        .map_err(|_| bad_request_msg(format!("invalid sinceTime val: {raw}")))?;
    let (diff, has_more) = files::public_diff(&state, link.collection_id, since).await?;
    Ok(Json(json!({ "diff": diff, "hasMore": has_more })))
}

async fn info(State(state): State<AppState>, link: CollectionLink) -> ApiResult<Response> {
    let collection = public_collection(&state, &link, false).await?;
    // Referral codes belong to the storage-bonus feature, which self-hosting does not offer.
    let mut res = Json(json!({ "collection": collection, "referralCode": "" })).into_response();
    if let Some(t) = &link.device_token
        && let Ok(v) = HeaderValue::from_str(t)
    {
        res.headers_mut().insert("X-Link-Device-Token", v);
    }
    Ok(res)
}

fn app_name(headers: &HeaderMap) -> &'static str {
    let pkg = headers
        .get("X-Client-Package")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if pkg.starts_with("io.ente.auth") {
        "auth"
    } else if pkg.starts_with("io.ente.locker") {
        "locker"
    } else {
        "photos"
    }
}

async fn upload_url(
    State(state): State<AppState>,
    link: CollectionLink,
    PJson(req): PJson<UploadUrlRequest>,
) -> ApiResult<Json<Value>> {
    let c = public_collection(&state, &link, true).await?;
    Ok(Json(collect::upload_url(&state, c.owner.id, &req).await?))
}

async fn multipart_upload_url(
    State(state): State<AppState>,
    link: CollectionLink,
    PJson(req): PJson<MultipartUploadUrlRequest>,
) -> ApiResult<Json<Value>> {
    // museum requires part checksums on public uploads.
    if req.part_md5s.as_ref().is_none_or(|m| m.is_empty()) {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let c = public_collection(&state, &link, true).await?;
    Ok(Json(
        collect::multipart_upload_url(&state, c.owner.id, &req).await?,
    ))
}

async fn create_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    link: CollectionLink,
    PJson(file): PJson<File>,
) -> ApiResult<Json<File>> {
    if file.file.decryption_header.is_empty()
        || file.thumbnail.decryption_header.is_empty()
        || file.metadata.decryption_header.is_empty()
    {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    if file.collection_id != link.collection_id {
        return Err(bad_request_msg("can only update to associated collection"));
    }
    let c = public_collection(&state, &link, true).await?;
    let app = app_name(&headers);
    Ok(Json(
        collect::create_file(&state, c.owner.id, &c.app, app, file).await?,
    ))
}

/// museum `ente.VerifyPasswordRequest`.
#[derive(Debug, Deserialize)]
pub struct VerifyPasswordRequest {
    /// Password hash derived by the client.
    #[serde(rename = "passHash", default)]
    pub pass_hash: String,
}

async fn verify_password(
    State(state): State<AppState>,
    link: CollectionLink,
    PJson(req): PJson<VerifyPasswordRequest>,
) -> ApiResult<Json<Value>> {
    if req.pass_hash.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let row = library_sql::active_collection_links(&state.db.pool, link.collection_id)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND))?;
    verify_link_password(&state, row.pw_hash.as_deref(), &req.pass_hash)
}

/// museum `verifyPassword`: 400 when the link has no password, 401 when wrong,
/// else a 30-day JWT.
pub fn verify_link_password(
    state: &AppState,
    expected: Option<&str>,
    given: &str,
) -> ApiResult<Json<Value>> {
    let expected = expected
        .filter(|h| !h.is_empty())
        .ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST))?;
    let ok: bool = subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), given.as_bytes()).into();
    if !ok {
        return Err(ApiError::status(StatusCode::UNAUTHORIZED));
    }
    Ok(Json(
        json!({ "jwtToken": jwt::new_password_token(&state.keys.jwt, given) }),
    ))
}
