//! Endpoints: /family/* (create, invite, accept, members, remove, revoke, leave, storage limits).
//!
//! Reference: museum `pkg/api/family.go`, `pkg/controller/family`, `pkg/repo/family.go`.

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::users::extract::{Body, FamilyAuthUser};
use crate::users::{self, family};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/family/invite-info/{token}", get(invite_info))
        .route("/family/accept-invite", post(accept_invite))
        .route("/family/leave", delete(leave))
        .route("/family/create", post(create))
        .route("/family/add-member", post(add_member))
        .route("/family/members", get(members))
        .route("/family/remove-member/{id}", delete(remove_member))
        .route("/family/revoke-invite/{id}", delete(revoke_invite))
        .route("/family/modify-storage", post(modify_storage))
}

fn member_uuid(s: &str) -> ApiResult<String> {
    uuid::Uuid::parse_str(s)
        .map(|u| u.to_string())
        .map_err(|_| users::bad_request_plain())
}

async fn invite_info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Json<family::InviteInfo>> {
    Ok(Json(family::invite_info(&state, &token).await?))
}

#[derive(Deserialize)]
struct AcceptRequest {
    token: String,
}

async fn accept_invite(
    State(state): State<AppState>,
    Body(req): Body<AcceptRequest>,
) -> ApiResult<Json<family::AcceptInviteResponse>> {
    if req.token.is_empty() {
        return Err(users::bad_request_plain());
    }
    Ok(Json(family::accept_invite(&state, &req.token).await?))
}

async fn leave(State(state): State<AppState>, user: AuthUser) -> ApiResult<StatusCode> {
    family::leave_family(&state, user.user_id).await?;
    Ok(StatusCode::OK)
}

async fn create(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
) -> ApiResult<StatusCode> {
    family::create_family(&state, u.user_id).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteRequest {
    email: String,
    storage_limit: Option<i64>,
}

async fn add_member(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
    Body(req): Body<InviteRequest>,
) -> ApiResult<StatusCode> {
    if req.email.is_empty() {
        return Err(users::bad_request_plain());
    }
    family::invite_member(&state, u.user_id, &req.email, req.storage_limit).await?;
    Ok(StatusCode::OK)
}

async fn members(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
) -> ApiResult<Json<family::FamilyMemberResponse>> {
    Ok(Json(family::fetch_members(&state, u.user_id).await?))
}

async fn remove_member(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    family::remove_member(&state, u.user_id, &member_uuid(&id)?).await?;
    Ok(StatusCode::OK)
}

async fn revoke_invite(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    family::revoke_invite(&state, u.user_id, &member_uuid(&id)?).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModifyStorageRequest {
    id: String,
    storage_limit: Option<i64>,
}

async fn modify_storage(
    State(state): State<AppState>,
    FamilyAuthUser(u): FamilyAuthUser,
    Body(req): Body<ModifyStorageRequest>,
) -> ApiResult<StatusCode> {
    family::modify_member_storage(&state, u.user_id, &member_uuid(&req.id)?, req.storage_limit)
        .await?;
    Ok(StatusCode::OK)
}
