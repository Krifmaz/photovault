//! Endpoints: /collections/* (albums: create, list, share, links, add/move/remove files,
//! diff, delete, rename, magic metadata).
//!
//! Reference: museum `pkg/api/collection.go` and `pkg/controller/collections`. The logic
//! lives in [`crate::library::collections`] and [`crate::library::links`].

use crate::auth::StorageUser;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::library::collections::{
    self, BulkShareRequest, BulkUnshareRequest, CreateRequest, ShareRequest,
};
use crate::library::errors;
use crate::library::http::{Empty200, JsonBody, QueryMap, empty_json, header, path_i64};
use crate::library::links::{self, CreateLinkRequest, UpdateLinkRequest};
use crate::library::model::{CollectionFileItem, MagicMetadata, filter_public_urls_for_role, role};
use crate::library::sql::{begin, db_err};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// museum's `DefaultMaxBatchSize`.
const MAX_BATCH: usize = 1000;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/collections", post(create).get(list_v2))
        .route("/collections/{collection_id}", get(get_by_id))
        .route("/collections/v2", get(list_v2))
        .route("/collections/v3", get(list_v3))
        .route("/collections/share", post(share))
        .route("/collections/share/batch", post(batch_share))
        .route("/collections/share/bulk", post(bulk_share))
        .route("/collections/join-link", post(join_link))
        .route("/collections/share-url", post(create_link).put(update_link))
        .route(
            "/collections/share-url/{collection_id}",
            delete(disable_link),
        )
        .route("/collections/unshare", post(unshare))
        .route("/collections/unshare/bulk", post(bulk_unshare))
        .route("/collections/leave/{collection_id}", post(leave))
        .route("/collections/add-files", post(add_files))
        .route("/collections/move-files", post(move_files))
        .route("/collections/restore-files", post(restore_files))
        .route("/collections/v3/remove-files", post(remove_files))
        .route("/collections/suggest-delete", post(suggest_delete))
        .route("/collections/v2/diff", get(diff))
        .route("/collections/file", get(get_file))
        .route("/collections/sharees", get(sharees))
        .route("/collections/v3/{collection_id}", delete(trash_v3))
        .route("/collections/rename", post(rename))
        .route("/collections/magic-metadata", put(magic_metadata))
        .route(
            "/collections/public-magic-metadata",
            put(public_magic_metadata),
        )
        .route(
            "/collections/sharee-magic-metadata",
            put(sharee_magic_metadata),
        )
}

async fn create(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<CreateRequest>,
) -> ApiResult<Json<Value>> {
    let c = collections::create(&state, u.user_id, u.app, req).await?;
    Ok(Json(json!({ "collection": c })))
}

async fn get_by_id(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let cid = path_i64(&id)?;
    let a = collections::access(&state, cid, u.user_id, u.app, true, false).await?;
    let r = a.role;
    let mut c = collections::with_sharing_details(&state, a, u.user_id).await?;
    if r != role::OWNER {
        c.magic_metadata = None;
        c.public_urls = c.public_urls.map(|v| filter_public_urls_for_role(v, r));
    }
    Ok(Json(json!({ "collection": c })))
}

async fn list_v2(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let since = q.i64("sinceTime");
    let owned = collections::owned_since(&state, u.user_id, since, u.app, None).await?;
    let shared = collections::shared_since(&state, u.user_id, since, u.app, None).await?;
    let mut all: Vec<Value> = Vec::with_capacity(owned.len() + shared.len());
    for c in owned {
        all.push(serde_json::to_value(c).map_err(errors::internal)?);
    }
    all.extend(shared.into_iter().map(collections::shared_response));
    Ok(Json(json!({ "collections": all })))
}

async fn list_v3(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let since = q.i64("sinceTime");
    let shared_since = q.i64("sharedSinceTime");
    let limit = if q.str("limit").is_empty() {
        1000
    } else {
        q.i64("limit").min(1000)
    };
    let owned = collections::owned_since(&state, u.user_id, since, u.app, Some(limit)).await?;
    let shared =
        collections::shared_since(&state, u.user_id, shared_since, u.app, Some(limit)).await?;
    let shared: Vec<Value> = shared
        .into_iter()
        .map(collections::shared_response)
        .collect();
    Ok(Json(json!({ "owned": owned, "shared": shared })))
}

fn check_share(r: &ShareRequest) -> ApiResult<()> {
    if r.collection_id == 0 || r.email.is_empty() {
        return Err(errors::bad());
    }
    Ok(())
}

async fn share(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<ShareRequest>,
) -> ApiResult<Json<Value>> {
    check_share(&req)?;
    let s = collections::share(&state, u.user_id, u.app, &req).await?;
    Ok(Json(json!({ "sharees": s })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BatchShareRequest {
    shares: Option<Vec<ShareRequest>>,
}

async fn batch_share(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<BatchShareRequest>,
) -> ApiResult<Json<Value>> {
    let shares = req.shares.ok_or_else(errors::bad)?;
    for s in &shares {
        check_share(s)?;
    }
    let s = collections::batch_share(&state, u.user_id, u.app, &shares).await?;
    Ok(Json(json!({ "sharees": s })))
}

async fn bulk_share(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<BulkShareRequest>,
) -> ApiResult<Json<Value>> {
    let r = collections::bulk_share(&state, u.user_id, &req).await?;
    Ok(Json(json!({ "results": r })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct JoinLinkRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "encryptedKey")]
    encrypted_key: String,
}

fn token_from(headers: &HeaderMap, q: &QueryMap, h: &str, param: &str) -> String {
    let v = header(headers, h);
    if v.is_empty() {
        q.str(param).to_string()
    } else {
        v.to_string()
    }
}

async fn join_link(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    headers: HeaderMap,
    q: QueryMap,
    JsonBody(req): JsonBody<JoinLinkRequest>,
) -> ApiResult<Json<Value>> {
    if req.collection_id == 0 || req.encrypted_key.is_empty() {
        return Err(errors::bad());
    }
    let token = token_from(&headers, &q, "X-Auth-Access-Token", "accessToken");
    let jwt = token_from(&headers, &q, "X-Auth-Access-Token-JWT", "accessTokenJWT");
    collections::join_link(
        &state,
        u.user_id,
        req.collection_id,
        &req.encrypted_key,
        &token,
        &jwt,
    )
    .await?;
    Ok(empty_json())
}

async fn verify_owner(
    state: &AppState,
    cid: i64,
    uid: i64,
) -> ApiResult<collections::CollectionRow> {
    let c = collections::get_row(state, cid).await?;
    if c.owner_id != uid {
        return Err(errors::denied());
    }
    Ok(c)
}

async fn create_link(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<CreateLinkRequest>,
) -> ApiResult<Json<Value>> {
    if req.collection_id == 0 {
        return Err(errors::bad());
    }
    req.validate()?;
    let c = collections::get_row(&state, req.collection_id).await?;
    if !c.allow_sharing() {
        return Err(errors::bad());
    }
    if c.owner_id != u.user_id {
        return Err(errors::denied());
    }
    let r = links::create_collection_link(&state, c.app(), &req).await?;
    Ok(Json(json!({ "result": r })))
}

async fn update_link(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<UpdateLinkRequest>,
) -> ApiResult<Json<Value>> {
    if req.collection_id == 0 {
        return Err(errors::bad());
    }
    req.validate()?;
    let c = verify_owner(&state, req.collection_id, u.user_id).await?;
    let r = links::update_collection_link(&state, c.app(), &req).await?;
    Ok(Json(json!({ "result": r })))
}

async fn disable_link(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Empty200> {
    let cid = path_i64(&id)?;
    verify_owner(&state, cid, u.user_id).await?;
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    links::disable_collection_link(tx.conn(), cid, now_micros())
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(Empty200)
}

async fn unshare(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<ShareRequest>,
) -> ApiResult<Json<Value>> {
    check_share(&req)?;
    let s = collections::unshare(&state, u.user_id, req.collection_id, &req.email).await?;
    Ok(Json(json!({ "sharees": s })))
}

async fn bulk_unshare(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<BulkUnshareRequest>,
) -> ApiResult<Json<Value>> {
    let r = collections::bulk_unshare(&state, u.user_id, &req).await?;
    Ok(Json(json!({ "results": r })))
}

async fn leave(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Empty200> {
    let cid = path_i64(&id)?;
    collections::leave(&state, u.user_id, cid).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FilesRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    files: Option<Vec<CollectionFileItem>>,
}

impl FilesRequest {
    fn checked(self) -> ApiResult<(i64, Vec<CollectionFileItem>)> {
        let files = self.files.ok_or_else(errors::bad)?;
        if self.collection_id == 0 {
            return Err(errors::bad());
        }
        if files.len() > MAX_BATCH {
            return Err(errors::too_large());
        }
        Ok((self.collection_id, files))
    }
}

async fn add_files(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<FilesRequest>,
) -> ApiResult<Empty200> {
    let (cid, files) = req.checked()?;
    collections::add_files(&state, u.user_id, u.app, cid, &files).await?;
    Ok(Empty200)
}

async fn restore_files(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<FilesRequest>,
) -> ApiResult<Empty200> {
    let (cid, files) = req.checked()?;
    collections::restore_files(&state, u.user_id, u.app, cid, &files).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MoveFilesRequest {
    #[serde(rename = "fromCollectionID")]
    from: i64,
    #[serde(rename = "toCollectionID")]
    to: i64,
    files: Option<Vec<CollectionFileItem>>,
}

async fn move_files(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MoveFilesRequest>,
) -> ApiResult<Empty200> {
    let files = req.files.ok_or_else(errors::bad)?;
    if req.from == 0 || req.to == 0 {
        return Err(errors::bad());
    }
    if files.len() > MAX_BATCH {
        return Err(errors::too_large());
    }
    collections::move_files(&state, u.user_id, u.app, req.from, req.to, &files).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FileIdsRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "fileIDs")]
    file_ids: Option<Vec<i64>>,
}

impl FileIdsRequest {
    fn checked(self) -> ApiResult<(i64, Vec<i64>)> {
        let ids = self.file_ids.ok_or_else(errors::bad)?;
        if self.collection_id == 0 {
            return Err(errors::bad());
        }
        if ids.len() > MAX_BATCH {
            return Err(errors::too_large());
        }
        Ok((self.collection_id, ids))
    }
}

async fn remove_files(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<FileIdsRequest>,
) -> ApiResult<Empty200> {
    let (cid, ids) = req.checked()?;
    collections::remove_files_v3(&state, u.user_id, u.app, cid, &ids).await?;
    Ok(Empty200)
}

async fn suggest_delete(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<FileIdsRequest>,
) -> ApiResult<Empty200> {
    let (cid, ids) = req.checked()?;
    collections::suggest_delete(&state, u.user_id, u.app, cid, &ids).await?;
    Ok(Empty200)
}

async fn diff(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let (files, has_more) = collections::diff(
        &state,
        u.user_id,
        u.app,
        q.i64("collectionID"),
        q.i64("sinceTime"),
    )
    .await?;
    Ok(Json(json!({ "diff": files, "hasMore": has_more })))
}

async fn get_file(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let f =
        collections::get_file(&state, u.user_id, q.i64("collectionID"), q.i64("fileID")).await?;
    Ok(Json(json!({ "file": f })))
}

async fn sharees(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let cid = q.i64("collectionID");
    collections::access(&state, cid, u.user_id, u.app, false, false).await?;
    let s = collections::sharees(&state, cid).await?;
    Ok(Json(json!({ "sharees": s })))
}

async fn trash_v3(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    Path(id): Path<String>,
    q: QueryMap,
) -> ApiResult<Empty200> {
    // museum binds `collectionID` and `keepFiles` from the query; both are required.
    let cid = match q.opt_i64("collectionID") {
        Some(v) if v != 0 => v,
        _ => path_i64(&id)
            .ok()
            .filter(|v| *v != 0)
            .ok_or_else(errors::bad)?,
    };
    let keep = q.opt_bool("keepFiles")?.ok_or_else(errors::bad)?;
    collections::trash_v3(&state, u.user_id, u.app, cid, keep).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RenameRequest {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "encryptedName")]
    encrypted_name: String,
    #[serde(rename = "nameDecryptionNonce")]
    name_decryption_nonce: String,
}

async fn rename(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<RenameRequest>,
) -> ApiResult<Empty200> {
    if req.collection_id == 0
        || req.encrypted_name.is_empty()
        || req.name_decryption_nonce.is_empty()
    {
        return Err(errors::bad());
    }
    collections::rename(
        &state,
        u.user_id,
        req.collection_id,
        &req.encrypted_name,
        &req.name_decryption_nonce,
    )
    .await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MagicMetadataRequest {
    id: i64,
    #[serde(rename = "magicMetadata")]
    magic_metadata: MagicMetadata,
}

impl MagicMetadataRequest {
    fn checked(self) -> ApiResult<(i64, MagicMetadata)> {
        if self.id == 0 || !self.magic_metadata.is_valid() {
            return Err(errors::bad());
        }
        Ok((self.id, self.magic_metadata))
    }
}

async fn magic_metadata(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MagicMetadataRequest>,
) -> ApiResult<Empty200> {
    let (cid, m) = req.checked()?;
    collections::update_magic_metadata(&state, u.user_id, cid, m, false).await?;
    Ok(Empty200)
}

async fn public_magic_metadata(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MagicMetadataRequest>,
) -> ApiResult<Empty200> {
    let (cid, m) = req.checked()?;
    collections::update_magic_metadata(&state, u.user_id, cid, m, true).await?;
    Ok(Empty200)
}

async fn sharee_magic_metadata(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<MagicMetadataRequest>,
) -> ApiResult<Empty200> {
    let (cid, m) = req.checked()?;
    collections::update_sharee_magic_metadata(&state, u.user_id, u.app, cid, &m).await?;
    Ok(Empty200)
}
