//! Endpoints: /user-entity/* — per-user encrypted entities of the Photos app
//! (people/face-cluster names, location tags, smart albums, memories, …).
//!
//! Mirrors museum's `pkg/api/userentity.go`, `pkg/controller/userentity` and
//! `pkg/repo/userentity` over the `entity_key` and `entity_data` tables.
//! Diffs return rows (deleted ones included, with `null` data/header) whose
//! `updated_at > sinceTime`, ordered by `updated_at`, at most `limit`.

use crate::auth::StorageUser;
use crate::db::now_micros;
use crate::entities::http::{JsonBody, QueryMap};
use crate::entities::ids::new_nano_id;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/user-entity/key", post(create_key).get(get_key))
        .route("/user-entity/key/ensure", post(ensure_key))
        .route(
            "/user-entity/entity",
            post(create_entity).put(update_entity).delete(delete_entity),
        )
        .route("/user-entity/entity/diff", get(diff))
}

const TYPES: [&str; 8] = [
    "location",
    "person",
    "cgroup",
    "smart_album",
    "memory",
    "contact",
    "space",
    "library_share",
];

fn validate_type(t: &str) -> ApiResult<()> {
    if TYPES.contains(&t) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!("Invalid EntityType: {t}")))
    }
}

fn bind_error() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

/// museum's `EntityKey`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityKey {
    #[serde(rename = "userID")]
    user_id: i64,
    #[serde(rename = "type")]
    kind: String,
    encrypted_key: String,
    header: String,
    created_at: i64,
}

/// museum's `EntityData`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityData {
    id: String,
    #[serde(rename = "userID")]
    user_id: i64,
    #[serde(rename = "type")]
    kind: String,
    encrypted_data: Option<String>,
    header: Option<String>,
    is_deleted: bool,
    created_at: i64,
    updated_at: i64,
}

type DataTuple = (String, i64, String, Option<String>, Option<String>, bool, i64, i64);
const DATA_COLS: &str =
    "id, user_id, type, encrypted_data, header, is_deleted, created_at, updated_at";

impl From<DataTuple> for EntityData {
    fn from(t: DataTuple) -> Self {
        Self {
            id: t.0,
            user_id: t.1,
            kind: t.2,
            encrypted_data: t.3,
            header: t.4,
            is_deleted: t.5,
            created_at: t.6,
            updated_at: t.7,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyRequest {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    encrypted_key: String,
    #[serde(default)]
    header: String,
}

impl KeyRequest {
    fn check(&self) -> ApiResult<()> {
        if self.kind.is_empty() || self.encrypted_key.is_empty() || self.header.is_empty() {
            return Err(bind_error());
        }
        validate_type(&self.kind)
    }
}

async fn fetch_key(st: &AppState, user_id: i64, kind: &str) -> ApiResult<Option<EntityKey>> {
    let row: Option<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT user_id, type, encrypted_key, header, created_at FROM entity_key WHERE user_id = $1 AND type = $2",
    )
    .bind(user_id)
    .bind(kind)
    .fetch_optional(&st.db.pool)
    .await?;
    Ok(row.map(|r| EntityKey {
        user_id: r.0,
        kind: r.1,
        encrypted_key: r.2,
        header: r.3,
        created_at: r.4,
    }))
}

async fn insert_key_if_absent(st: &AppState, user_id: i64, req: &KeyRequest) -> ApiResult<u64> {
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO entity_key(user_id, type, encrypted_key, header, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (user_id, type) DO NOTHING",
    )
    .bind(user_id)
    .bind(&req.kind)
    .bind(&req.encrypted_key)
    .bind(&req.header)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    Ok(r.rows_affected())
}

async fn create_key(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    JsonBody(req): JsonBody<KeyRequest>,
) -> ApiResult<StatusCode> {
    req.check()?;
    if insert_key_if_absent(&st, user.user_id, &req).await? == 1 {
        return Ok(StatusCode::OK);
    }
    let existing = fetch_key(&st, user.user_id, &req.kind)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    if existing.encrypted_key == req.encrypted_key && existing.header == req.header {
        return Ok(StatusCode::OK);
    }
    Err(ApiError::new(
        StatusCode::CONFLICT,
        "ALREADY_EXISTS",
        "Key already exists",
    ))
}

async fn ensure_key(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    JsonBody(req): JsonBody<KeyRequest>,
) -> ApiResult<impl IntoResponse> {
    req.check()?;
    insert_key_if_absent(&st, user.user_id, &req).await?;
    let key = fetch_key(&st, user.user_id, &req.kind)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(key)))
}

async fn get_key(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<EntityKey>> {
    let kind = q.str("type").ok_or_else(bind_error)?;
    let key = fetch_key(&st, user.user_id, kind)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    Ok(Json(key))
}

async fn fetch_entity(st: &AppState, user_id: i64, id: &str) -> ApiResult<EntityData> {
    let row: Option<DataTuple> = sqlx::query_as(&format!(
        "SELECT {DATA_COLS} FROM entity_data WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    row.map(Into::into)
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    encrypted_data: String,
    #[serde(default)]
    header: String,
    #[serde(default)]
    id: Option<String>,
}

/// museum's `EntityDataRequest.IsValid`.
fn validate_create(req: &CreateRequest, user_id: i64) -> ApiResult<()> {
    validate_type(&req.kind)?;
    match req.kind.as_str() {
        "smart_album" => {
            let id = req.id.as_deref().ok_or_else(|| {
                ApiError::bad_request("ID is required for SmartAlbum entity type")
            })?;
            if !id.starts_with(&format!("sa_{user_id}_")) {
                return Err(ApiError::bad_request(format!(
                    "ID {id} is not valid for SmartAlbum entity type"
                )));
            }
        }
        "library_share" => {
            let id = req.id.as_deref().ok_or_else(|| {
                ApiError::bad_request("ID is required for LibraryShare entity type")
            })?;
            let invalid = || {
                ApiError::bad_request(format!("ID {id} is not valid for LibraryShare entity type"))
            };
            let parts: Vec<&str> = id.split('_').collect();
            if parts.len() != 3 || parts[0] != "ls" {
                return Err(invalid());
            }
            let owner: i64 = parts[1].parse().map_err(|_| invalid())?;
            let recipient: i64 = parts[2].parse().map_err(|_| invalid())?;
            if owner != user_id
                || recipient <= 0
                || recipient == user_id
                || id != format!("ls_{owner}_{recipient}")
            {
                return Err(invalid());
            }
        }
        _ => {}
    }
    Ok(())
}

async fn create_entity(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    JsonBody(req): JsonBody<CreateRequest>,
) -> ApiResult<Json<EntityData>> {
    if req.kind.is_empty() || req.encrypted_data.is_empty() || req.header.is_empty() {
        return Err(bind_error());
    }
    validate_create(&req, user.user_id)?;
    let id = match &req.id {
        Some(id) => id.clone(),
        None => new_nano_id(&req.kind.to_lowercase())
            .ok_or_else(|| ApiError::internal("failed to generate new id"))?,
    };
    let now = now_micros();
    if req.kind == "smart_album" || req.kind == "library_share" {
        // Deleted rows of these types may be restored by re-creating them.
        let row: Option<(String,)> = sqlx::query_as(
            "INSERT INTO entity_data(id, user_id, type, encrypted_data, header, is_deleted, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, FALSE, $6, $6) \
             ON CONFLICT (id) DO UPDATE SET encrypted_data = EXCLUDED.encrypted_data, header = EXCLUDED.header, \
             is_deleted = FALSE, updated_at = EXCLUDED.updated_at \
             WHERE entity_data.user_id = EXCLUDED.user_id AND entity_data.type = EXCLUDED.type AND entity_data.is_deleted = TRUE \
             RETURNING id",
        )
        .bind(&id)
        .bind(user.user_id)
        .bind(&req.kind)
        .bind(&req.encrypted_data)
        .bind(&req.header)
        .bind(now)
        .fetch_optional(&st.db.pool)
        .await?;
        if row.is_none() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "ALREADY_EXISTS",
                "Entity already exists",
            ));
        }
    } else {
        sqlx::query(
            "INSERT INTO entity_data(id, user_id, type, encrypted_data, header, is_deleted, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, FALSE, $6, $6)",
        )
        .bind(&id)
        .bind(user.user_id)
        .bind(&req.kind)
        .bind(&req.encrypted_data)
        .bind(&req.header)
        .bind(now)
        .execute(&st.db.pool)
        .await?;
    }
    Ok(Json(fetch_entity(&st, user.user_id, &id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    encrypted_data: String,
    #[serde(default)]
    header: String,
    #[serde(default)]
    expected_updated_at: Option<i64>,
}

async fn update_entity(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    JsonBody(req): JsonBody<UpdateRequest>,
) -> ApiResult<Json<EntityData>> {
    if req.id.is_empty() || req.kind.is_empty() || req.encrypted_data.is_empty() || req.header.is_empty() {
        return Err(bind_error());
    }
    validate_type(&req.kind)?;
    if req.kind == "library_share" && req.expected_updated_at.is_none_or(|v| v <= 0) {
        return Err(ApiError::bad_request(
            "expectedUpdatedAt is required for LibraryShare entity type",
        ));
    }
    let base = "UPDATE entity_data SET encrypted_data = $1, header = $2, updated_at = $3 \
                WHERE id = $4 AND user_id = $5 AND type = $6 AND is_deleted = FALSE";
    let r = match req.expected_updated_at {
        Some(expected) => {
            sqlx::query(&format!("{base} AND updated_at = $7"))
                .bind(&req.encrypted_data)
                .bind(&req.header)
                .bind(now_micros())
                .bind(&req.id)
                .bind(user.user_id)
                .bind(&req.kind)
                .bind(expected)
                .execute(&st.db.pool)
                .await?
        }
        None => {
            sqlx::query(base)
                .bind(&req.encrypted_data)
                .bind(&req.header)
                .bind(now_micros())
                .bind(&req.id)
                .bind(user.user_id)
                .bind(&req.kind)
                .execute(&st.db.pool)
                .await?
        }
    };
    if r.rows_affected() != 1 {
        let existing = fetch_entity(&st, user.user_id, &req.id).await?;
        if existing.is_deleted {
            return Err(ApiError::bad_request("entity is already deleted"));
        }
        if existing.kind != req.kind {
            return Err(ApiError::bad_request("entity type does not match"));
        }
        let same = existing.encrypted_data.as_deref() == Some(req.encrypted_data.as_str())
            && existing.header.as_deref() == Some(req.header.as_str());
        if !same {
            if req
                .expected_updated_at
                .is_some_and(|e| e != existing.updated_at)
            {
                // museum's ErrVersionMismatch.
                return Err(ApiError::status(StatusCode::CONFLICT));
            }
            return Err(ApiError::internal("exactly one row should be updated"));
        }
    }
    Ok(Json(fetch_entity(&st, user.user_id, &req.id).await?))
}

async fn delete_entity(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    q: QueryMap,
) -> ApiResult<StatusCode> {
    let id = q.0.get("id").cloned().unwrap_or_default();
    sqlx::query(
        "UPDATE entity_data SET is_deleted = TRUE, encrypted_data = NULL, header = NULL, updated_at = $1 WHERE id = $2 AND user_id = $3",
    )
    .bind(now_micros())
    .bind(&id)
    .bind(user.user_id)
    .execute(&st.db.pool)
    .await?;
    Ok(StatusCode::OK)
}

async fn diff(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let kind = q.str("type").ok_or_else(bind_error)?;
    let since = q.int("sinceTime", i64::MIN, i64::MAX)?;
    let limit = q.int("limit", i16::MIN as i64, i16::MAX as i64)?;
    if limit == 0 {
        return Err(bind_error());
    }
    if !(1..=5000).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 5000"));
    }
    let rows: Vec<DataTuple> = sqlx::query_as(&format!(
        "SELECT {DATA_COLS} FROM entity_data WHERE user_id = $1 AND type = $2 AND updated_at > $3 ORDER BY updated_at LIMIT $4"
    ))
    .bind(user.user_id)
    .bind(kind)
    .bind(since)
    .bind(limit)
    .fetch_all(&st.db.pool)
    .await?;
    let diff: Vec<EntityData> = rows.into_iter().map(Into::into).collect();
    Ok(Json(json!({ "diff": diff })))
}
