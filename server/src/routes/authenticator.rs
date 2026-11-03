//! Endpoints: /authenticator/* — encrypted TOTP entries of the Auth app.
//!
//! Mirrors museum's `pkg/api/authenticator.go`, `pkg/controller/authenticator`
//! and `pkg/repo/authenticator` over `authenticator_key` / `authenticator_entity`.
//! Only the Auth app may call these (museum's `RequireAuthApp`).

use crate::db::now_micros;
use crate::entities::http::{AuthAppUser, JsonBody, QueryMap};
use crate::entities::ids::{new_uuid, parse_uuid};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/authenticator/key", post(create_key).get(get_key))
        .route(
            "/authenticator/entity",
            post(create_entity).put(update_entity).delete(delete_entity),
        )
        .route("/authenticator/entity/diff", get(diff))
}

fn bind_error() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

fn not_found_typed() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Key {
    #[serde(rename = "userID")]
    user_id: i64,
    encrypted_key: String,
    header: String,
    created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entity {
    id: String,
    #[serde(rename = "userID")]
    user_id: i64,
    encrypted_data: Option<String>,
    header: Option<String>,
    is_deleted: bool,
    created_at: i64,
    updated_at: i64,
}

type EntityTuple = (String, i64, Option<String>, Option<String>, i64, i64, i64);
const COLS: &str = "id, user_id, encrypted_data, header, CASE WHEN is_deleted THEN 1 ELSE 0 END, created_at, updated_at";

impl From<EntityTuple> for Entity {
    fn from(t: EntityTuple) -> Self {
        Self {
            id: t.0,
            user_id: t.1,
            encrypted_data: t.2,
            header: t.3,
            is_deleted: t.4 != 0,
            created_at: t.5,
            updated_at: t.6,
        }
    }
}

async fn fetch_key(st: &AppState, user_id: i64) -> ApiResult<Option<Key>> {
    let row: Option<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT user_id, encrypted_key, header, created_at FROM authenticator_key WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    Ok(row.map(|r| Key {
        user_id: r.0,
        encrypted_key: r.1,
        header: r.2,
        created_at: r.3,
    }))
}

async fn fetch_entity(st: &AppState, user_id: i64, id: &str) -> ApiResult<Entity> {
    let row: Option<EntityTuple> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM authenticator_entity WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    row.map(Into::into).ok_or_else(not_found_typed)
}

/// museum's `validateKey`: the Auth key must exist before entities.
async fn require_key(st: &AppState, user_id: i64) -> ApiResult<()> {
    if fetch_key(st, user_id).await?.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "AUTH_KEY_NOT_CREATED",
            "AuthKey is not created",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateKeyRequest {
    #[serde(default)]
    encrypted_key: String,
    #[serde(default)]
    header: String,
}

async fn create_key(
    State(st): State<AppState>,
    AuthAppUser(user): AuthAppUser,
    JsonBody(req): JsonBody<CreateKeyRequest>,
) -> ApiResult<StatusCode> {
    if req.encrypted_key.is_empty() || req.header.is_empty() {
        return Err(bind_error());
    }
    if crate::legacy::users::key_attributes(&st, user.user_id)
        .await?
        .is_none()
    {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO authenticator_key(user_id, encrypted_key, header, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4) ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user.user_id)
    .bind(&req.encrypted_key)
    .bind(&req.header)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::conflict("Key already exists"));
    }
    Ok(StatusCode::OK)
}

async fn get_key(
    State(st): State<AppState>,
    AuthAppUser(user): AuthAppUser,
) -> ApiResult<Json<Key>> {
    fetch_key(&st, user.user_id)
        .await?
        .map(Json)
        .ok_or_else(not_found_typed)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateEntityRequest {
    #[serde(default)]
    encrypted_data: String,
    #[serde(default)]
    header: String,
}

async fn create_entity(
    State(st): State<AppState>,
    AuthAppUser(user): AuthAppUser,
    JsonBody(req): JsonBody<CreateEntityRequest>,
) -> ApiResult<Json<Entity>> {
    if req.encrypted_data.is_empty() || req.header.is_empty() {
        return Err(bind_error());
    }
    require_key(&st, user.user_id).await?;
    let id = new_uuid();
    let now = now_micros();
    sqlx::query(
        "INSERT INTO authenticator_entity(id, user_id, encrypted_data, header, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, $5, $5)",
    )
    .bind(&id)
    .bind(user.user_id)
    .bind(&req.encrypted_data)
    .bind(&req.header)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    Ok(Json(fetch_entity(&st, user.user_id, &id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEntityRequest {
    #[serde(default)]
    id: String,
    #[serde(default)]
    encrypted_data: String,
    #[serde(default)]
    header: String,
}

async fn update_entity(
    State(st): State<AppState>,
    AuthAppUser(user): AuthAppUser,
    JsonBody(req): JsonBody<UpdateEntityRequest>,
) -> ApiResult<StatusCode> {
    let id = parse_uuid(&req.id).ok_or_else(bind_error)?;
    if req.encrypted_data.is_empty() || req.header.is_empty() {
        return Err(bind_error());
    }
    require_key(&st, user.user_id).await?;
    let r = sqlx::query(
        "UPDATE authenticator_entity SET encrypted_data = $1, header = $2, updated_at = $3 \
         WHERE id = $4 AND user_id = $5 AND is_deleted = FALSE",
    )
    .bind(&req.encrypted_data)
    .bind(&req.header)
    .bind(now_micros())
    .bind(&id)
    .bind(user.user_id)
    .execute(&st.db.pool)
    .await?;
    if r.rows_affected() != 1 {
        let existing = fetch_entity(&st, user.user_id, &id).await?;
        if existing.is_deleted {
            return Err(ApiError::bad_request("entity is already deleted"));
        }
        if existing.encrypted_data.as_deref() != Some(req.encrypted_data.as_str())
            || existing.header.as_deref() != Some(req.header.as_str())
        {
            return Err(ApiError::internal("exactly one row should be updated"));
        }
    }
    Ok(StatusCode::OK)
}

async fn delete_entity(
    State(st): State<AppState>,
    AuthAppUser(user): AuthAppUser,
    q: QueryMap,
) -> ApiResult<StatusCode> {
    let id = q.str("id").and_then(parse_uuid).ok_or_else(bind_error)?;
    sqlx::query(
        "UPDATE authenticator_entity SET is_deleted = TRUE, encrypted_data = NULL, header = NULL, updated_at = $1 \
         WHERE id = $2 AND user_id = $3",
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
    AuthAppUser(user): AuthAppUser,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let since = q.int("sinceTime", i64::MIN, i64::MAX)?;
    let limit = q.int("limit", i16::MIN as i64, i16::MAX as i64)?;
    if limit == 0 {
        return Err(bind_error());
    }
    if !(1..=5000).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 5000"));
    }
    let rows: Vec<EntityTuple> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM authenticator_entity WHERE user_id = $1 AND updated_at > $2 ORDER BY updated_at LIMIT $3"
    ))
    .bind(user.user_id)
    .bind(since)
    .bind(limit)
    .fetch_all(&st.db.pool)
    .await?;
    let diff: Vec<Entity> = rows.into_iter().map(Into::into).collect();
    Ok(Json(json!({ "diff": diff, "timestamp": now_micros() })))
}
