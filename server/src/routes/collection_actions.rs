//! Endpoints: /collection-actions/* (pending removals and delete suggestions).
//!
//! Reference: museum `pkg/api/collection_actions.go`. The logic lives in
//! [`crate::library::actions`].

use crate::auth::StorageUser;
use crate::error::ApiResult;
use crate::library::actions::{self, LIST_LIMIT};
use crate::library::errors;
use crate::library::http::{JsonBody, QueryMap};
use crate::state::AppState;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/collection-actions/pending-remove", get(pending_remove))
        .route(
            "/collection-actions/delete-suggestions",
            get(delete_suggestions),
        )
        .route(
            "/collection-actions/reject-delete-suggestions",
            post(reject_delete_suggestions),
        )
}

/// `parseSinceTime`: empty → 0, malformed → 400 `{}`.
fn since(q: &QueryMap) -> ApiResult<i64> {
    match q.str("sinceTime") {
        "" => Ok(0),
        s => s.parse().map_err(|_| errors::bad()),
    }
}

async fn pending_remove(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let a = actions::list_pending_remove(&state, u.user_id, since(&q)?, LIST_LIMIT).await?;
    let more = a.len() as i64 >= LIST_LIMIT;
    Ok(Json(json!({ "actions": a, "hasMore": more })))
}

async fn delete_suggestions(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let a = actions::list_delete_suggestions(&state, u.user_id, since(&q)?, LIST_LIMIT).await?;
    let more = a.len() as i64 >= LIST_LIMIT;
    Ok(Json(json!({ "actions": a, "hasMore": more })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RejectRequest {
    #[serde(rename = "fileIDs")]
    file_ids: Option<Vec<i64>>,
}

async fn reject_delete_suggestions(
    State(state): State<AppState>,
    StorageUser(u): StorageUser,
    JsonBody(req): JsonBody<RejectRequest>,
) -> ApiResult<Json<Value>> {
    let ids = req.file_ids.ok_or_else(errors::bad)?;
    if ids.is_empty() {
        return Err(errors::bad_msg("fileIDs must not be empty"));
    }
    if ids.len() as i64 > LIST_LIMIT {
        return Err(errors::bad_msg(format!(
            "can not reject more than {LIST_LIMIT} items in one request"
        )));
    }
    let n = actions::reject_delete_suggestions(&state, u.user_id, &ids).await?;
    Ok(Json(json!({ "updated": n })))
}
