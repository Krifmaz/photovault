//! Endpoints: /passkeys/* (list, rename, delete, registration begin/finish). Authenticated with
//! an ACCOUNTS-scoped JWT from `/users/accounts-token` (museum `accountsJwtAuthAPI`).
//!
//! Reference: museum `pkg/api/passkeys.go`, `pkg/controller/passkeys.go`, `pkg/repo/passkey`.

use crate::error::ApiResult;
use crate::state::AppState;
use crate::users::extract::{AccountsJwtUser, QueryMap};
use crate::users::webauthn::CredentialJson;
use crate::users::{self, login, passkeys};
use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::{Value, json};

/// Museum `_passKeyNameMaxLength`.
const NAME_MAX: usize = 256;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/passkeys", get(list))
        .route("/passkeys/{passkey_id}", patch(rename).delete(remove))
        .route("/passkeys/registration/begin", post(begin_registration))
        .route("/passkeys/registration/finish", post(finish_registration))
}

fn check_name(name: &str) -> ApiResult<()> {
    let n = name.chars().count();
    if !(1..=NAME_MAX).contains(&n) {
        return Err(users::bad_request(format!(
            "friendlyName must be between 1 and {NAME_MAX} characters"
        )));
    }
    Ok(())
}

fn passkey_uuid(s: &str) -> ApiResult<String> {
    uuid::Uuid::parse_str(s)
        .map(|u| u.to_string())
        .map_err(|_| users::bad_request_plain())
}

async fn list(
    State(state): State<AppState>,
    AccountsJwtUser(u): AccountsJwtUser,
) -> ApiResult<Json<Value>> {
    users::get_user(&state, u.user_id).await?;
    let list = passkeys::list(&state, u.user_id).await?;
    Ok(Json(json!({
        "accountsUrl": login::accounts_url(&state),
        // Go marshals an empty slice from a nil result as null.
        "passkeys": if list.is_empty() { Value::Null } else { json!(list) },
    })))
}

async fn rename(
    State(state): State<AppState>,
    AccountsJwtUser(u): AccountsJwtUser,
    Path(id): Path<String>,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let id = passkey_uuid(&id)?;
    let name = q.get("friendlyName");
    check_name(name)?;
    users::get_user(&state, u.user_id).await?;
    passkeys::rename(&state, u.user_id, &id, name).await?;
    Ok(Json(json!({})))
}

async fn remove(
    State(state): State<AppState>,
    AccountsJwtUser(u): AccountsJwtUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = passkey_uuid(&id)?;
    users::get_user(&state, u.user_id).await?;
    passkeys::delete(&state, u.user_id, &id).await?;
    Ok(Json(json!({})))
}

async fn begin_registration(
    State(state): State<AppState>,
    AccountsJwtUser(u): AccountsJwtUser,
) -> ApiResult<Json<Value>> {
    let user = users::get_user(&state, u.user_id).await?;
    let (options, session_id) = passkeys::begin_registration(&state, &user).await?;
    Ok(Json(json!({ "options": options, "sessionID": session_id })))
}

async fn finish_registration(
    State(state): State<AppState>,
    AccountsJwtUser(u): AccountsJwtUser,
    q: QueryMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let session_id = passkey_uuid(q.get("sessionID"))?;
    let name = q.get("friendlyName");
    check_name(name)?;
    let user = users::get_user(&state, u.user_id).await?;
    let cred: CredentialJson = serde_json::from_slice(&body)
        .map_err(|_| users::bad_request("Error parsing attestation response"))?;
    passkeys::finish_registration(&state, &user, &session_id, name, &cred).await?;
    Ok(Json(json!({})))
}
