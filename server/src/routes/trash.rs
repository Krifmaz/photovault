//! Endpoints: /trash/*.
//!
//! Reference: museum `pkg/api/trash.go` and `pkg/controller/trash.go`. The logic lives in
//! [`crate::library::trash`].

use crate::auth::StorageUser;
use crate::error::ApiResult;
use crate::library::errors;
use crate::library::http::{Empty200, JsonBody, QueryMap};
use crate::library::trash;
use crate::state::AppState;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/trash/v2/diff", get(diff))
        .route("/trash/delete", post(delete))
        .route("/trash/empty", post(empty))
}

async fn diff(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<serde_json::Value>> {
    let (diff, has_more) = trash::diff(&state, u.user_id, q.i64("sinceTime"), u.app).await?;
    Ok(Json(json!({ "diff": diff, "hasMore": has_more })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DeleteRequest {
    #[serde(rename = "fileIDs")]
    file_ids: Option<Vec<i64>>,
}

async fn delete(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<DeleteRequest>,
) -> ApiResult<Empty200> {
    let ids = req.file_ids.ok_or_else(errors::bad)?;
    trash::delete(&state, u.user_id, &ids).await?;
    Ok(Empty200)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EmptyRequest {
    #[serde(rename = "lastUpdatedAt")]
    last_updated_at: i64,
}

async fn empty(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<EmptyRequest>,
) -> ApiResult<Empty200> {
    if req.last_updated_at == 0 {
        return Err(errors::bad());
    }
    trash::empty(&state, u.user_id, req.last_updated_at, u.app).await?;
    Ok(Empty200)
}
