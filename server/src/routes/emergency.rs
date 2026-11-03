//! Endpoints: /emergency-contacts/* (Legacy / trusted contacts).
//!
//! Mirrors museum's `pkg/api/emergency.go` and `pkg/controller/emergency`; the
//! logic lives in [`crate::legacy::emergency`].

use crate::auth::AuthUser;
use crate::entities::http::{CodedJson, JsonBody, invalid_param};
use crate::entities::ids::parse_uuid;
use crate::error::{ApiError, ApiResult};
use crate::legacy::emergency as em;
use crate::legacy::password::{
    RecoveryAuthorization, SetupSrpRequest, SetupSrpResponse, UpdateSrpAndKeysRequest,
    UpdateSrpSetupResponse, recover_srp_and_key_attributes, setup_srp,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/emergency-contacts/add", post(add))
        .route("/emergency-contacts/info", get(info))
        .route("/emergency-contacts/update", post(update))
        .route(
            "/emergency-contacts/update-recovery-notice",
            post(update_recovery_notice),
        )
        .route("/emergency-contacts/start-recovery", post(start_recovery))
        .route("/emergency-contacts/stop-recovery", post(stop_recovery))
        .route("/emergency-contacts/reject-recovery", post(reject_recovery))
        .route("/emergency-contacts/approve-recovery", post(approve_recovery))
        .route("/emergency-contacts/recovery-info/{id}", get(recovery_info))
        .route(
            "/emergency-contacts/init-change-password",
            post(init_change_password),
        )
        .route("/emergency-contacts/change-password", post(change_password))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddContact {
    #[serde(default)]
    email: String,
    #[serde(default)]
    encrypted_key: String,
    #[serde(default)]
    recovery_notice_in_days: Option<i64>,
}

async fn add(
    State(st): State<AppState>,
    user: AuthUser,
    JsonBody(req): JsonBody<AddContact>,
) -> ApiResult<StatusCode> {
    if req.email.is_empty() || req.encrypted_key.is_empty() {
        return Err(ApiError::status(StatusCode::BAD_REQUEST));
    }
    let contact_id = crate::legacy::users::user_id_by_email(&st, &req.email)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    let notice_hrs = req.recovery_notice_in_days.unwrap_or(30) * 24;
    let changed = em::add_contact(
        &st,
        user.user_id,
        contact_id,
        &req.encrypted_key,
        notice_hrs,
    )
    .await?;
    if changed {
        em::update_notification_invited(&st, user.user_id, contact_id);
    }
    Ok(StatusCode::OK)
}

async fn info(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<em::InfoResponse>> {
    Ok(Json(em::info(&st, user.user_id).await?))
}

#[derive(Deserialize)]
struct UpdateContact {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(rename = "emergencyContactID", default)]
    emergency_contact_id: i64,
    #[serde(default)]
    state: String,
}

async fn update(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<UpdateContact>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.emergency_contact_id == 0 || req.state.is_empty() {
        return Err(invalid_param());
    }
    em::update_contact(
        &st,
        user.user_id,
        req.user_id,
        req.emergency_contact_id,
        &req.state,
    )
    .await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRecoveryNotice {
    #[serde(rename = "emergencyContactID", default)]
    emergency_contact_id: i64,
    #[serde(default)]
    recovery_notice_in_days: i64,
}

async fn update_recovery_notice(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<UpdateRecoveryNotice>,
) -> ApiResult<Json<Value>> {
    if req.emergency_contact_id == 0 || req.recovery_notice_in_days == 0 {
        return Err(invalid_param());
    }
    em::update_recovery_notice(
        &st,
        user.user_id,
        req.emergency_contact_id,
        req.recovery_notice_in_days,
    )
    .await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct ContactIdentifier {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(rename = "emergencyContactID", default)]
    emergency_contact_id: i64,
}

async fn start_recovery(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<ContactIdentifier>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.emergency_contact_id == 0 {
        return Err(invalid_param());
    }
    em::start_recovery(&st, user.user_id, req.user_id, req.emergency_contact_id).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct RecoveryIdentifier {
    #[serde(default)]
    id: String,
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(rename = "emergencyContactID", default)]
    emergency_contact_id: i64,
}

impl RecoveryIdentifier {
    fn checked(&self) -> ApiResult<String> {
        if self.user_id == 0 || self.emergency_contact_id == 0 {
            return Err(invalid_param());
        }
        parse_uuid(&self.id).ok_or_else(invalid_param)
    }
}

async fn stop_recovery(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<RecoveryIdentifier>,
) -> ApiResult<Json<Value>> {
    let id = req.checked()?;
    em::stop_recovery(&st, user.user_id, &id, req.user_id, req.emergency_contact_id).await?;
    Ok(Json(json!({})))
}

async fn reject_recovery(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<RecoveryIdentifier>,
) -> ApiResult<Json<Value>> {
    let id = req.checked()?;
    em::reject_recovery(&st, user.user_id, &id, req.user_id, req.emergency_contact_id).await?;
    Ok(Json(json!({})))
}

async fn approve_recovery(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<RecoveryIdentifier>,
) -> ApiResult<Json<Value>> {
    let id = req.checked()?;
    em::approve_recovery(&st, user.user_id, &id, req.user_id, req.emergency_contact_id).await?;
    Ok(Json(json!({})))
}

async fn recovery_info(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_uuid(&id).ok_or_else(invalid_param)?;
    let contact = em::check_recovery(&st, user.user_id, &id).await?;
    let key_attr = crate::legacy::users::key_attributes(&st, contact.user_id)
        .await?
        .ok_or(ApiError::status(StatusCode::NOT_FOUND))?;
    Ok(Json(json!({
        "encryptedKey": contact.encrypted_key,
        "userKeyAttr": key_attr,
    })))
}

#[derive(Deserialize)]
struct RecoverySrpSetupRequest {
    #[serde(rename = "recoveryID", default)]
    recovery_id: String,
    #[serde(rename = "setupSRPRequest")]
    setup: SetupSrpRequest,
}

async fn init_change_password(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<RecoverySrpSetupRequest>,
) -> ApiResult<Json<SetupSrpResponse>> {
    let id = parse_uuid(&req.recovery_id).ok_or_else(invalid_param)?;
    let contact = em::check_recovery(&st, user.user_id, &id).await?;
    Ok(Json(setup_srp(&st, contact.user_id, &req.setup).await?))
}

#[derive(Deserialize)]
struct RecoveryUpdateSrpAndKeysRequest {
    #[serde(rename = "recoveryID", default)]
    recovery_id: String,
    #[serde(rename = "updateSrpAndKeysRequest")]
    update: UpdateSrpAndKeysRequest,
}

async fn change_password(
    State(st): State<AppState>,
    user: AuthUser,
    CodedJson(req): CodedJson<RecoveryUpdateSrpAndKeysRequest>,
) -> ApiResult<Json<UpdateSrpSetupResponse>> {
    let id = parse_uuid(&req.recovery_id).ok_or_else(invalid_param)?;
    let contact = em::check_recovery(&st, user.user_id, &id).await?;
    let log_out_all = req.update.log_out_other_devices.unwrap_or(true);
    let resp = recover_srp_and_key_attributes(
        &st,
        contact.user_id,
        &req.update,
        log_out_all,
        RecoveryAuthorization::Emergency {
            session_id: id,
            user_id: contact.user_id,
            emergency_contact_id: contact.emergency_contact_id,
        },
    )
    .await?;
    em::notify_recovered(&st, contact.user_id, contact.emergency_contact_id);
    Ok(Json(resp))
}
