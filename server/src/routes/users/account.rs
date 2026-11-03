//! Key attributes, public keys, details, deletion and recovery (museum `pkg/api/user.go`,
//! `user.go`, `user_details.go`, `user_delete.go`, `account_recovery.go`).

use super::srp::{UpdateKeys, validate_b64_len};
use super::*;
use crate::db::now_micros;
use crate::users::KeyAttributes;
use crate::users::lifecycle;
use axum::response::{Html, IntoResponse};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetAttributesRequest {
    key_attributes: KeyAttributes,
}

fn validate_recovery(
    master_enc: &str,
    master_nonce: &str,
    recovery_enc: &str,
    recovery_nonce: &str,
) -> ApiResult<()> {
    validate_b64_len(master_enc, 48)?;
    validate_b64_len(master_nonce, 24)?;
    validate_b64_len(recovery_enc, 48)?;
    validate_b64_len(recovery_nonce, 24)
}

pub(super) async fn set_attributes(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<SetAttributesRequest>,
) -> ApiResult<StatusCode> {
    let k = &req.key_attributes;
    UpdateKeys {
        kek_salt: k.kek_salt.clone(),
        encrypted_key: k.encrypted_key.clone(),
        key_decryption_nonce: k.key_decryption_nonce.clone(),
        mem_limit: k.mem_limit,
        ops_limit: k.ops_limit,
    }
    .validate()?;
    validate_b64_len(&k.public_key, 32)?;
    validate_b64_len(&k.encrypted_secret_key, 48)?;
    validate_b64_len(&k.secret_key_decryption_nonce, 24)?;
    validate_recovery(
        &k.master_key_encrypted_with_recovery_key,
        &k.master_key_decryption_nonce,
        &k.recovery_key_encrypted_with_master_key,
        &k.recovery_key_decryption_nonce,
    )?;
    if users::key_attributes(&state.db, user.user_id)
        .await?
        .is_some()
    {
        return Err(users::permission_denied());
    }
    users::insert_key_attributes(&state.db, user.user_id, k).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EmailMfaRequest {
    is_enabled: Option<bool>,
}

pub(super) async fn update_email_mfa(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<EmailMfaRequest>,
) -> ApiResult<StatusCode> {
    let enabled = req.is_enabled.ok_or_else(users::bad_request_plain)?;
    if !enabled {
        let srp: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM srp_auth WHERE user_id = $1")
            .bind(user.user_id)
            .fetch_optional(&state.db.pool)
            .await?;
        if srp.is_none() {
            return Err(users::conflict("SRP setup incomplete"));
        }
    }
    sqlx::query("UPDATE users SET email_mfa = $1 WHERE user_id = $2")
        .bind(enabled)
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct RecoveryKeyRequest {
    master_key_encrypted_with_recovery_key: String,
    master_key_decryption_nonce: String,
    recovery_key_encrypted_with_master_key: String,
    recovery_key_decryption_nonce: String,
}

pub(super) async fn set_recovery_key(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<RecoveryKeyRequest>,
) -> ApiResult<StatusCode> {
    validate_recovery(
        &req.master_key_encrypted_with_recovery_key,
        &req.master_key_decryption_nonce,
        &req.recovery_key_encrypted_with_master_key,
        &req.recovery_key_decryption_nonce,
    )?;
    let k = users::key_attributes(&state.db, user.user_id)
        .await?
        .ok_or_else(users::not_found_plain)?;
    if !k.recovery_key_encrypted_with_master_key.is_empty() {
        return Err(crate::error::ApiError::internal(
            "recovery key is already set",
        ));
    }
    sqlx::query("UPDATE key_attributes SET master_key_encrypted_with_recovery_key = $1, master_key_decryption_nonce = $2, recovery_key_encrypted_with_master_key = $3, recovery_key_decryption_nonce = $4 WHERE user_id = $5")
        .bind(&req.master_key_encrypted_with_recovery_key)
        .bind(&req.master_key_decryption_nonce)
        .bind(&req.recovery_key_encrypted_with_master_key)
        .bind(&req.recovery_key_decryption_nonce)
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(StatusCode::OK)
}

async fn lookup_public_key(state: &AppState, email: &str) -> ApiResult<String> {
    let id = users::user_id_for_email(state, email)
        .await?
        .ok_or_else(users::not_found_plain)?;
    users::public_key(&state.db, id).await
}

pub(super) async fn public_key(
    State(state): State<AppState>,
    _user: AuthUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "publicKey": lookup_public_key(&state, q.get("email")).await? }),
    ))
}

#[derive(Deserialize)]
pub(super) struct PublicKeysRequest {
    emails: Vec<String>,
}

pub(super) async fn public_keys(
    State(state): State<AppState>,
    _user: AuthUser,
    Body(req): Body<PublicKeysRequest>,
) -> ApiResult<Json<Value>> {
    if req.emails.is_empty() {
        return Err(users::bad_request_plain());
    }
    if req.emails.len() > 10 {
        return Err(users::batch_too_large());
    }
    let mut seen = std::collections::HashSet::new();
    for e in &req.emails {
        let n = users::normalize_email(e);
        if n.is_empty() || !seen.insert(n) {
            return Err(users::bad_request_plain());
        }
    }
    let mut keys = Vec::new();
    for e in &req.emails {
        keys.push(lookup_public_key(&state, e).await?);
    }
    Ok(Json(json!({ "publicKeys": keys })))
}

pub(super) async fn session_validity(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        match users::key_attributes(&state.db, user.user_id).await? {
            Some(k) => json!({ "hasSetKeys": true, "keyAttributes": k }),
            None => json!({ "hasSetKeys": false }),
        },
    ))
}

pub(super) async fn details(
    State(state): State<AppState>,
    user: AuthUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let fetch_count = !matches!(
        q.get("memoryCount"),
        "false" | "0" | "f" | "F" | "FALSE" | "False"
    );
    Ok(Json(
        details_v2(&state, user.user_id, user.app, fetch_count).await?,
    ))
}

/// Museum `UserController.GetDetailsV2` (also attached to `/admin/user`).
pub(crate) async fn details_v2(
    state: &AppState,
    user_id: i64,
    app: crate::auth::App,
    fetch_count: bool,
) -> ApiResult<Value> {
    let u = users::get_user_not_deleted(state, user_id).await?;
    let db = &state.db;
    let bonus_user = u.family_admin_id.unwrap_or(u.id);
    let mut family_data = match u.family_admin_id {
        Some(admin) => Some(users::family::fetch_members_for_admin(state, admin).await?),
        None => None,
    };
    let bonus = users::active_bonus(db, bonus_user).await?;
    let sub = crate::capacity::subscription(state, u.id).await?;
    let can_disable_mfa =
        sqlx::query_as::<_, (i64,)>("SELECT user_id FROM srp_auth WHERE user_id = $1")
            .bind(u.id)
            .fetch_optional(&db.pool)
            .await?
            .is_some();
    let app_name = app.as_str();
    let now = now_micros();
    let mut plan_storage = if sub.expiry_time < now {
        0
    } else {
        sub.storage
    };
    if let Some(f) = family_data.as_mut() {
        if f.expiry_time < now {
            f.storage = 0;
        } else {
            plan_storage = f.storage;
        }
    }
    let (_, locker_bytes) = users::app_file_stats(db, u.id, "locker").await?;
    let mut usage = users::usage(db, u.id).await?;
    if app != crate::auth::App::Auth {
        usage = (usage - locker_bytes).max(0);
    }
    let mut out = json!({
        "email": u.email,
        "usage": usage,
        "subscription": sub,
        "storageBonus": bonus.usable_bonus(plan_storage),
        "profileData": {
            "canDisableEmailMFA": can_disable_mfa,
            "isEmailMFAEnabled": u.email_mfa,
            "isTwoFactorEnabled": u.is_two_factor_enabled,
            "passkeyCount": users::passkeys::count(state, u.id).await?,
        },
        "bonusData": bonus,
    });
    if let Some(f) = family_data {
        out["familyData"] = serde_json::to_value(f).map_err(crate::error::ApiError::internal)?;
    }
    if fetch_count {
        out["fileCount"] = json!(users::file_count(db, u.id, app_name).await?);
        out["sharedCollectionsCount"] = json!(0);
    }
    if app == crate::auth::App::Locker {
        let mut total = 0;
        let members = match u.family_admin_id {
            Some(a) => users::family::active_members(db, a)
                .await?
                .into_iter()
                .map(|m| m.0)
                .collect(),
            None => vec![u.id],
        };
        for m in members {
            total += users::app_file_stats(db, m, "locker").await?.0;
        }
        out["lockerFamilyUsage"] = json!({ "familyFileCount": total });
    }
    Ok(out)
}

pub(super) async fn deletion_summary(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    let db = &state.db;
    Ok(Json(json!({
        "photosAndVideosCount": users::file_count(db, user.user_id, "photos").await?,
        "authenticatorCodesCount": users::authenticator_count(db, user.user_id).await?,
        "lockerRecordsCount": users::file_count(db, user.user_id, "locker").await?,
    })))
}

/// Locker tier limits (museum `GetLockerLimitsForTier(paid)`); self-hosted accounts get the
/// paid tier, matching the files workstream's upload checks.
const LOCKER_FILE_LIMIT: i64 = 1000;
const LOCKER_STORAGE_LIMIT: i64 = 10 * 1024 * 1024 * 1024;

pub(super) async fn locker_usage(
    State(state): State<AppState>,
    user: crate::auth::StorageUser,
) -> ApiResult<Json<Value>> {
    let user = user.0;
    let u = users::get_user_not_deleted(&state, user.user_id).await?;
    let db = &state.db;
    let members = match u.family_admin_id {
        Some(a) => users::family::active_members(db, a)
            .await?
            .into_iter()
            .map(|m| m.0)
            .collect(),
        None => vec![u.id],
    };
    let (mut files, mut bytes, mut mine_files, mut mine_bytes) = (0, 0, 0, 0);
    for m in members {
        let (f, b) = users::app_file_stats(db, m, "locker").await?;
        files += f;
        bytes += b;
        if m == u.id {
            mine_files = f;
            mine_bytes = b;
        }
    }
    Ok(Json(json!({
        "isPaid": true,
        "isFamily": u.family_admin_id.is_some(),
        "usedFileCount": files,
        "fileLimit": LOCKER_FILE_LIMIT,
        "remainingFileCount": (LOCKER_FILE_LIMIT - files).max(0),
        "usedStorage": bytes,
        "storageLimit": LOCKER_STORAGE_LIMIT,
        "remainingStorage": (LOCKER_STORAGE_LIMIT - bytes).max(0),
        "userFileCount": mine_files,
        "userStorage": mine_bytes,
    })))
}

const DELETE_CHALLENGE_PREFIX: &str = "delete-jwt";

pub(super) async fn delete_challenge(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    users::get_user(&state, user.user_id).await?;
    let keys = users::key_attributes(&state.db, user.user_id)
        .await?
        .ok_or_else(users::not_found_plain)?;
    let jwt = login::session_jwt(&state, user.user_id, scope::DELETE_ACCOUNT, None, None);
    let wrapped = crate::crypto::b64url(format!("{DELETE_CHALLENGE_PREFIX}{jwt}").as_bytes());
    let challenge = users::seal_token(&wrapped, &keys.public_key)?;
    let apps: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT app FROM tokens WHERE user_id = $1")
        .bind(user.user_id)
        .fetch_all(&state.db.pool)
        .await?;
    Ok(Json(json!({
        "allowDelete": true,
        "encryptedChallenge": challenge,
        "apps": apps.into_iter().map(|a| a.0).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct DeleteRequest {
    challenge: String,
    feedback: Option<String>,
    reason_category: Option<String>,
    reason: Option<String>,
}

pub(super) async fn delete_account(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<DeleteRequest>,
) -> ApiResult<Json<Value>> {
    let jwt = req
        .challenge
        .strip_prefix(DELETE_CHALLENGE_PREFIX)
        .unwrap_or(&req.challenge);
    let claim = login::validate_jwt(&state, jwt, scope::DELETE_ACCOUNT)?;
    if claim.user_id != user.user_id {
        return Err(users::permission_denied());
    }
    users::get_user(&state, user.user_id).await?;
    let cancelled = lifecycle::delete_account(&state, user.user_id, true).await?;
    let mut feedback = serde_json::Map::new();
    for (k, v) in [
        ("feedback", &req.feedback),
        ("reasonCategory", &req.reason_category),
        ("reason", &req.reason),
    ] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            feedback.insert(k.into(), json!(v));
        }
    }
    sqlx::query("UPDATE users SET delete_feedback = $1 WHERE user_id = $2")
        .bind(Value::Object(feedback).to_string())
        .bind(user.user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(Json(
        json!({ "isSubscriptionCancelled": cancelled, "userID": user.user_id }),
    ))
}

const RECOVERED_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Account recovered</title></head><body><h1>Account recovered</h1><p>Your account has been recovered. You can log in again.</p></body></html>";
const RECOVERY_ERROR_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Account recovery failed</title></head><body><h1>Account recovery failed</h1><p>This link is invalid or has expired.</p></body></html>";

pub(super) async fn recover_account_link(
    State(state): State<AppState>,
    q: QueryMap,
) -> ApiResult<axum::response::Response> {
    let token = q.get("token");
    if token.is_empty() {
        return Err(users::bad_request("token missing"));
    }
    let result = match lifecycle::parse_recovery_token(&state, token) {
        Ok((id, email)) => lifecycle::recover_account(&state, id, &email, true).await,
        Err(e) => Err(e),
    };
    Ok(match result {
        Ok(_) => Html(RECOVERED_HTML).into_response(),
        Err(_) => Html(RECOVERY_ERROR_HTML).into_response(),
    })
}

#[derive(Deserialize)]
pub(super) struct RecoveryRequest {
    token: String,
}

pub(super) async fn validate_recover_account(
    State(state): State<AppState>,
    Body(req): Body<RecoveryRequest>,
) -> ApiResult<Json<Value>> {
    let (id, email) = lifecycle::parse_recovery_token(&state, &req.token)?;
    let status = lifecycle::recovery_status(&state, id, &email, true).await?;
    Ok(Json(json!({ "status": status })))
}

pub(super) async fn recover_account(
    State(state): State<AppState>,
    Body(req): Body<RecoveryRequest>,
) -> ApiResult<Json<Value>> {
    let (id, email) = lifecycle::parse_recovery_token(&state, &req.token)?;
    let status = lifecycle::recover_account(&state, id, &email, true).await?;
    Ok(Json(json!({ "status": status })))
}
