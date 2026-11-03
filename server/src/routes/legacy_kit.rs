//! Endpoints: /legacy-kits/* (owner endpoints and the public recovery flow).
//!
//! Mirrors museum's `pkg/api/legacy_kit.go` and `pkg/controller/legacy_kit`;
//! the logic lives in [`crate::legacy::kit`].

use crate::auth::AuthUser;
use crate::entities::http::{JsonBody, client_ip};
use crate::entities::ids::parse_uuid;
use crate::error::{ApiError, ApiResult};
use crate::legacy::kit::{self, Initiator, KitRow, LegacyKit};
use crate::legacy::password::{
    RecoveryAuthorization, SetupSrpRequest, SetupSrpResponse, UpdateSrpAndKeysRequest,
    UpdateSrpSetupResponse, recover_srp_and_key_attributes, setup_srp,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/legacy-kits", post(create).get(list))
        .route("/legacy-kits/{id}/download", get(download))
        .route("/legacy-kits/{id}/download-content", get(download))
        .route("/legacy-kits/{id}/recovery-session", get(owner_session))
        .route("/legacy-kits/update-recovery-notice", post(update_notice))
        .route("/legacy-kits/block-recovery", post(block))
        .route("/legacy-kits/{id}", delete(remove))
        .route("/legacy-kits/recovery/challenge", post(challenge))
        .route("/legacy-kits/recovery/open", post(open))
        .route("/legacy-kits/recovery/session", post(session))
        .route("/legacy-kits/recovery/info", post(recovery_info))
        .route(
            "/legacy-kits/recovery/init-change-password",
            post(init_change_password),
        )
        .route("/legacy-kits/recovery/change-password", post(change_password))
}

fn bind_error() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

fn kit_id_param(raw: &str) -> ApiResult<String> {
    parse_uuid(raw).ok_or_else(|| ApiError::bad_request("invalid legacy kit id"))
}

fn body_uuid(raw: &str) -> ApiResult<String> {
    parse_uuid(raw).ok_or_else(bind_error)
}

fn validate_notice(hours: i64) -> ApiResult<()> {
    if kit::VALID_NOTICE_HOURS.contains(&hours) {
        Ok(())
    } else {
        Err(ApiError::bad_request("invalid legacy kit notice period"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    #[serde(default)]
    id: String,
    #[serde(default)]
    variant: i64,
    #[serde(default)]
    notice_period_in_hours: Option<i64>,
    #[serde(default)]
    encrypted_recovery_blob: String,
    #[serde(default)]
    auth_public_key: String,
    #[serde(default)]
    encrypted_owner_blob: String,
}

async fn create(
    State(st): State<AppState>,
    user: AuthUser,
    JsonBody(req): JsonBody<CreateRequest>,
) -> ApiResult<Json<LegacyKit>> {
    let id = body_uuid(&req.id)?;
    let notice = req.notice_period_in_hours.ok_or_else(bind_error)?;
    if req.variant == 0
        || req.encrypted_recovery_blob.is_empty()
        || req.auth_public_key.is_empty()
        || req.encrypted_owner_blob.is_empty()
    {
        return Err(bind_error());
    }
    if req.variant != kit::VARIANT_TWO_OF_THREE {
        return Err(ApiError::bad_request("invalid legacy kit variant"));
    }
    validate_notice(notice)?;
    if !kit::validate_public_key(&req.auth_public_key) {
        return Err(ApiError::bad_request("invalid legacy kit auth public key"));
    }
    let row = KitRow {
        id: id.clone(),
        user_id: user.user_id,
        variant: req.variant,
        notice_period_in_hrs: notice,
        encrypted_recovery_blob: req.encrypted_recovery_blob,
        auth_public_key: req.auth_public_key,
        encrypted_owner_blob: req.encrypted_owner_blob,
        is_deleted: false,
        created_at: 0,
        updated_at: 0,
    };
    kit::create_kit(&st, user.user_id, &row).await?;
    kit::notify_created(&st, user.user_id);
    let saved = kit::kit_for_owner(&st, user.user_id, &id).await?;
    Ok(Json(kit::to_api(&saved, None)))
}

async fn list(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let kits = kit::list_kits(&st, user.user_id).await?;
    let sessions: HashMap<String, kit::SessionRow> = kit::active_sessions_for_user(&st, user.user_id)
        .await?
        .into_iter()
        .map(|s| (s.kit_id.clone(), s))
        .collect();
    let out: Vec<LegacyKit> = kits
        .iter()
        .map(|k| kit::to_api(k, sessions.get(&k.id)))
        .collect();
    Ok(Json(json!({ "kits": out })))
}

async fn download(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = kit_id_param(&id)?;
    let k = kit::kit_for_owner(&st, user.user_id, &id).await?;
    Ok(Json(json!({
        "id": k.id,
        "variant": k.variant,
        "encryptedOwnerBlob": k.encrypted_owner_blob,
    })))
}

async fn owner_session(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = kit_id_param(&id)?;
    kit::kit_for_owner(&st, user.user_id, &id).await?;
    Ok(Json(match kit::active_session_for_kit(&st, &id).await? {
        None => json!({ "session": null, "initiators": [] }),
        Some(s) => json!({ "session": s.to_api(), "initiators": s.initiators }),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateNoticeRequest {
    #[serde(rename = "kitID", default)]
    kit_id: String,
    #[serde(default)]
    notice_period_in_hours: Option<i64>,
}

async fn update_notice(
    State(st): State<AppState>,
    user: AuthUser,
    JsonBody(req): JsonBody<UpdateNoticeRequest>,
) -> ApiResult<StatusCode> {
    let id = body_uuid(&req.kit_id)?;
    let hours = req.notice_period_in_hours.ok_or_else(bind_error)?;
    validate_notice(hours)?;
    if !kit::update_notice(&st, user.user_id, &id, hours).await? {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct KitIdRequest {
    #[serde(rename = "kitID", default)]
    kit_id: String,
}

async fn block(
    State(st): State<AppState>,
    user: AuthUser,
    JsonBody(req): JsonBody<KitIdRequest>,
) -> ApiResult<StatusCode> {
    let id = body_uuid(&req.kit_id)?;
    if !kit::block_recovery(&st, user.user_id, &id).await? {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(StatusCode::OK)
}

async fn remove(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let id = kit_id_param(&id)?;
    if !kit::delete_kit(&st, user.user_id, &id).await? {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    Ok(StatusCode::OK)
}

async fn challenge(
    State(st): State<AppState>,
    JsonBody(req): JsonBody<KitIdRequest>,
) -> ApiResult<Json<Value>> {
    let id = body_uuid(&req.kit_id)?;
    let k = kit::kit_by_id(&st, &id).await?;
    if k.is_deleted {
        return Err(ApiError::status(StatusCode::NOT_FOUND));
    }
    let (sealed, expires_at) = kit::create_challenge(&st, &k).await?;
    Ok(Json(json!({
        "kitID": k.id,
        "encryptedChallenge": sealed,
        "expiresAt": expires_at,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRequest {
    #[serde(rename = "kitID", default)]
    kit_id: String,
    #[serde(default)]
    challenge: String,
    #[serde(default)]
    used_part_indexes: Option<Vec<i64>>,
    #[serde(default)]
    email: Option<String>,
}

fn validate_used_parts(idx: &[i64]) -> ApiResult<()> {
    if idx.is_empty() {
        return Ok(());
    }
    let (threshold, parts) = (2usize, 3i64);
    if idx.len() != threshold {
        return Err(ApiError::bad_request(
            "legacy kit usedPartIndexes count does not match kit variant",
        ));
    }
    let mut seen = HashSet::new();
    for &i in idx {
        if !(1..=parts).contains(&i) {
            return Err(ApiError::bad_request(
                "legacy kit usedPartIndexes is outside kit variant range",
            ));
        }
        if !seen.insert(i) {
            return Err(ApiError::bad_request(
                "legacy kit usedPartIndexes must be unique",
            ));
        }
    }
    Ok(())
}

async fn open(
    State(st): State<AppState>,
    headers: HeaderMap,
    JsonBody(req): JsonBody<OpenRequest>,
) -> ApiResult<Json<Value>> {
    let id = body_uuid(&req.kit_id)?;
    if req.challenge.is_empty() {
        return Err(bind_error());
    }
    let used = req.used_part_indexes.unwrap_or_default();
    validate_used_parts(&used)?;
    let initiator = Initiator {
        used_part_indexes: used,
        ip: client_ip(&headers),
        user_agent: headers
            .get("User-Agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim()
            .to_string(),
    };
    let opened = kit::open_or_resume(&st, &id, &req.challenge, initiator).await?;
    if opened.created {
        kit::notify_started(
            &st,
            opened.user_id,
            opened.session.effective_notice_period_in_hrs,
            req.email,
        );
    }
    Ok(Json(json!({
        "session": opened.session.to_api(),
        "sessionToken": opened.token,
    })))
}

#[derive(Deserialize)]
struct SessionRequest {
    #[serde(rename = "sessionID", default)]
    session_id: String,
    #[serde(rename = "sessionToken", default)]
    session_token: String,
}

/// Validate a session request and load its session (404 `{}` for unknown tokens).
async fn load_session(
    st: &AppState,
    session_id: &str,
    token: &str,
) -> ApiResult<(String, String, kit::SessionRow)> {
    let id = body_uuid(session_id)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(bind_error());
    }
    let s = kit::session_for_use(st, &id, &token)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    Ok((id, token, s))
}

fn require_ready(s: &kit::SessionRow) -> ApiResult<()> {
    if s.status != kit::status::READY {
        return Err(ApiError::bad_request("legacy kit recovery is not ready"));
    }
    Ok(())
}

async fn session(
    State(st): State<AppState>,
    JsonBody(req): JsonBody<SessionRequest>,
) -> ApiResult<Json<kit::RecoverySession>> {
    let (_, _, s) = load_session(&st, &req.session_id, &req.session_token).await?;
    Ok(Json(s.to_api()))
}

async fn recovery_info(
    State(st): State<AppState>,
    JsonBody(req): JsonBody<SessionRequest>,
) -> ApiResult<Json<Value>> {
    let (_, _, s) = load_session(&st, &req.session_id, &req.session_token).await?;
    require_ready(&s)?;
    let k = kit::kit_by_id(&st, &s.kit_id).await?;
    let key_attr = crate::legacy::users::key_attributes(&st, s.user_id)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    Ok(Json(json!({
        "encryptedRecoveryBlob": k.encrypted_recovery_blob,
        "userKeyAttr": key_attr,
    })))
}

#[derive(Deserialize)]
struct SrpSetupRequest {
    #[serde(rename = "sessionID", default)]
    session_id: String,
    #[serde(rename = "sessionToken", default)]
    session_token: String,
    #[serde(rename = "setupSRPRequest")]
    setup: SetupSrpRequest,
}

async fn init_change_password(
    State(st): State<AppState>,
    JsonBody(req): JsonBody<SrpSetupRequest>,
) -> ApiResult<Json<SetupSrpResponse>> {
    let (_, _, s) = load_session(&st, &req.session_id, &req.session_token).await?;
    require_ready(&s)?;
    Ok(Json(setup_srp(&st, s.user_id, &req.setup).await?))
}

#[derive(Deserialize)]
struct UpdateSrpRequest {
    #[serde(rename = "sessionID", default)]
    session_id: String,
    #[serde(rename = "sessionToken", default)]
    session_token: String,
    #[serde(rename = "updateSrpAndKeysRequest")]
    update: UpdateSrpAndKeysRequest,
}

async fn change_password(
    State(st): State<AppState>,
    JsonBody(req): JsonBody<UpdateSrpRequest>,
) -> ApiResult<Json<UpdateSrpSetupResponse>> {
    let (id, token, s) = load_session(&st, &req.session_id, &req.session_token).await?;
    require_ready(&s)?;
    let log_out_all = req.update.log_out_other_devices.unwrap_or(true);
    let resp = recover_srp_and_key_attributes(
        &st,
        s.user_id,
        &req.update,
        log_out_all,
        RecoveryAuthorization::Kit {
            session_id: id,
            kit_id: s.kit_id.clone(),
            user_id: s.user_id,
            session_token: token,
        },
    )
    .await?;
    kit::notify_completed(&st, s.user_id);
    Ok(Json(resp))
}
