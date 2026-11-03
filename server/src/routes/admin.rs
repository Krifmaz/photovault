//! Endpoints: /admin/* (user lookup and support operations).
//!
//! Every route requires an admin session ([`Admin`]: ids in config `admins`, users marked
//! with `photovault admin mark-admin`).
//! Destructive operations additionally require a session created in the last 10 minutes,
//! like museum's `isFreshAdminToken`.
//!
//! Self-hosted differences: listmonk endpoints answer as museum does when listmonk is not
//! configured; queue, collection-share repair and file-count initialisation belong to other
//! subsystems and answer 501.
//!
//! Reference: museum `pkg/api/admin.go`, `pkg/api/admin_listmonk.go`, `ente/admin.go`.

use crate::auth::App;
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::users::extract::{Admin, Body, QueryMap};
use crate::users::{self, Revoke, family, lifecycle, login, passkeys};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Museum `AdminTokenValidityInMinutes`.
const FRESH_TOKEN_MINUTES: i64 = 10;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/mail", post(send_mail))
        .route("/admin/mail/subscribe", post(mailing_list))
        .route("/admin/mail/unsubscribe", post(mailing_list))
        .route(
            "/admin/listmonk/missing-subscribers/count",
            get(listmonk_count),
        )
        .route("/admin/users", get(list_users))
        .route("/admin/user", get(get_user))
        .route("/admin/user/scheduled-deletions", get(scheduled_deletions))
        .route("/admin/user/disable-2fa", post(disable_2fa))
        .route("/admin/user/update-referral", post(update_referral))
        .route("/admin/user/disable-passkeys", post(disable_passkeys))
        .route("/admin/user/update-email-mfa", post(update_email_mfa))
        .route(
            "/admin/user/unblock-storage-warning-login",
            post(unblock_storage_warning),
        )
        .route("/admin/user/add-ott", post(add_ott))
        .route("/admin/user/terminate-session", post(terminate_session))
        .route("/admin/user/close-family", post(close_family))
        .route("/admin/user/change-email", put(change_email))
        .route("/admin/user/delete", delete(delete_user))
        .route("/admin/user/recover", post(recover))
        .route("/admin/user/update-flag", post(update_flag))
        .route("/admin/email-hash", get(email_hash))
        .route("/admin/emails-from-hashes", post(emails_from_hashes))
        .route("/admin/user/subscription", put(update_subscription))
        .route("/admin/queue/re-queue", post(not_here))
        .route("/admin/collection/repair-deleted-share", post(not_here))
        .route("/admin/user/init-file-counts", post(not_here))
        .route("/admin/user/bonus", post(update_bonus))
}

/// Museum `isFreshAdminToken`: 400 "Token is too old" for sessions older than 10 minutes.
async fn require_fresh(state: &AppState, admin: &Admin) -> ApiResult<()> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT creation_time FROM tokens WHERE token_hash = $1")
            .bind(&admin.0.token_hash)
            .fetch_optional(&state.db.pool)
            .await?;
    let created = row.ok_or_else(users::not_found_plain)?.0;
    if created + FRESH_TOKEN_MINUTES * 60 * 1_000_000 < now_micros() {
        return Err(users::bad_request("Token is too old"));
    }
    Ok(())
}

fn audit(admin: &Admin, action: impl std::fmt::Display) {
    tracing::info!(admin_id = admin.0.user_id, "admin action: {action}");
}

/// Museum `ente.User` as serialised by the admin API (note the untagged `ID`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminUser {
    #[serde(rename = "ID")]
    id: i64,
    email: String,
    name: String,
    hash: String,
    creation_time: i64,
    #[serde(rename = "familyAdminID")]
    family_admin_id: Option<i64>,
    is_two_factor_enabled: bool,
    #[serde(rename = "isEmailMFAEnabled")]
    is_email_mfa_enabled: bool,
}

type RawUser = (i64, String, String, String, i64, Option<i64>, i64, i64);
const RAW_COLS: &str = "user_id, encrypted_email, email_decryption_nonce, email_hash, creation_time, family_admin_id, \
     CASE WHEN is_two_factor_enabled THEN 1 ELSE 0 END, CASE WHEN email_mfa THEN 1 ELSE 0 END";

fn to_admin_user(state: &AppState, r: RawUser) -> AdminUser {
    let (id, ct, nonce, hash, created, fam, tfa, mfa) = r;
    // Deleted accounts have no email left to decrypt.
    let email = if ct.is_empty() {
        String::new()
    } else {
        users::decrypt_email(state, &ct, &nonce).unwrap_or_default()
    };
    AdminUser {
        id,
        email,
        name: String::new(),
        hash,
        creation_time: created,
        family_admin_id: fam,
        is_two_factor_enabled: tfa != 0,
        is_email_mfa_enabled: mfa != 0,
    }
}

/// Museum `GetUserByIDInternal` (deleted users included).
async fn raw_user(state: &AppState, user_id: i64) -> ApiResult<AdminUser> {
    let row: Option<RawUser> =
        sqlx::query_as(&format!("SELECT {RAW_COLS} FROM users WHERE user_id = $1"))
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    Ok(to_admin_user(
        state,
        row.ok_or_else(users::not_found_plain)?,
    ))
}

/// Museum `GetUserByEmailHash`.
async fn user_by_email(state: &AppState, email: &str) -> ApiResult<AdminUser> {
    let hash = users::email_hash(&state.keys, &users::normalize_email(email));
    let row: Option<RawUser> = sqlx::query_as(&format!(
        "SELECT {RAW_COLS} FROM users WHERE email_hash = $1"
    ))
    .bind(&hash)
    .fetch_optional(&state.db.pool)
    .await?;
    Ok(to_admin_user(
        state,
        row.ok_or_else(users::not_found_plain)?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMailRequest {
    to: Option<Vec<String>>,
    #[serde(default)]
    from_name: String,
    #[serde(default)]
    from_email: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

async fn send_mail(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<SendMailRequest>,
) -> ApiResult<Json<Value>> {
    let to = req.to.unwrap_or_default();
    if to.is_empty()
        || req.from_name.is_empty()
        || req.from_email.is_empty()
        || req.subject.is_empty()
        || req.body.is_empty()
    {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("sending mail to {} recipients", to.len()),
    );
    for t in &to {
        crate::mail::send(&state.config, t, &req.subject, &req.body)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(Json(json!({})))
}

/// Without listmonk configured museum's mailing-list calls are no-ops.
async fn mailing_list(_admin: Admin) -> StatusCode {
    StatusCode::OK
}

async fn listmonk_count(_admin: Admin) -> ApiError {
    users::bad_request(
        "listmonk is not configured (server-url, username, password, list-ids are required)",
    )
}

async fn not_here(_admin: Admin) -> ApiError {
    users::not_implemented()
}

async fn list_users(
    State(state): State<AppState>,
    admin: Admin,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    require_fresh(&state, &admin).await?;
    let since: i64 = q
        .get("sinceTime")
        .parse()
        .map_err(|_| ApiError::internal("invalid sinceTime"))?;
    let rows: Vec<RawUser> = sqlx::query_as(&format!(
        "SELECT {RAW_COLS} FROM users WHERE creation_time > $1 AND creation_time < $2 AND encrypted_email <> '' ORDER BY creation_time"
    ))
    .bind(since)
    .bind(now_micros())
    .fetch_all(&state.db.pool)
    .await?;
    let list: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            // `GetAll` fills only id, email, hash and creation time.
            let u = to_admin_user(&state, r);
            json!({
                "ID": u.id, "email": u.email, "name": "", "hash": u.hash,
                "creationTime": u.creation_time, "familyAdminID": null,
                "isTwoFactorEnabled": null, "isEmailMFAEnabled": null,
            })
        })
        .collect();
    Ok(Json(json!({ "users": list })))
}

async fn get_user(
    State(state): State<AppState>,
    _admin: Admin,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let email = users::normalize_email(q.get("email"));
    let user = if email.is_empty() {
        let id: i64 = q
            .get("id")
            .parse()
            .map_err(|_| users::bad_request_plain())?;
        raw_user(&state, id).await?
    } else {
        let mut u = user_by_email(&state, &email).await?;
        u.email = email;
        u
    };
    let id = user.id;
    let mut out = json!({ "user": user });
    // Museum `attachSubscription`: each part is best-effort.
    match file_stats(&state, id).await {
        Ok((bytes, photos, locker)) => {
            out["storageConsumed"] = json!(bytes);
            out["storageConsumedStatus"] = json!("available");
            out["photosFileCount"] = json!(photos);
            out["lockerFileCount"] = json!(locker);
        }
        Err(_) => out["storageConsumedStatus"] = json!("unavailable"),
    }
    if let Ok(sub) = users::subscription(&state.db, id).await {
        out["subscription"] = json!(sub);
    }
    if let Ok(d) = crate::routes::users::details_v2(&state, id, App::Photos, false).await {
        out["details"] = d;
    }
    if let Ok(t) = token_info(&state, id).await {
        out["tokens"] = t;
    }
    if let Ok(n) = users::authenticator_count(&state.db, id).await {
        out["authCodes"] = json!(n);
    }
    Ok(Json(out))
}

async fn file_stats(state: &AppState, id: i64) -> ApiResult<(i64, i64, i64)> {
    let bytes = users::usage(&state.db, id).await?;
    let photos = users::file_count(&state.db, id, "photos").await?;
    let locker = users::file_count(&state.db, id, "locker").await?;
    Ok((bytes, photos, locker))
}

/// Museum `GetUserTokenInfo`.
async fn token_info(state: &AppState, id: i64) -> ApiResult<Value> {
    let rows: Vec<(i64, Option<i64>, Option<String>, String)> = sqlx::query_as(
        "SELECT creation_time, last_used_at, user_agent, app FROM tokens WHERE user_id = $1 AND is_deleted = $2",
    )
    .bind(id)
    .bind(false)
    .fetch_all(&state.db.pool)
    .await?;
    Ok(json!(
        rows.into_iter()
            .map(|(c, l, ua, app)| json!({
                "creationTime": c,
                "lastUsedTime": l.unwrap_or(0),
                "ua": ua.unwrap_or_default(),
                "isDeleted": false,
                "app": app,
            }))
            .collect::<Vec<_>>()
    ))
}

async fn scheduled_deletions(
    State(state): State<AppState>,
    _admin: Admin,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let email = users::normalize_email(q.get("email"));
    if email.is_empty() {
        return Err(users::bad_request("email is required"));
    }
    let hash = users::email_hash(&state.keys, &email);
    let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT d.user_id, u.creation_time, d.created_at, d.stage_schedule_time FROM data_cleanup d \
         JOIN users u ON u.user_id = d.user_id WHERE d.stage = $1 AND d.email_hash = $2 \
         ORDER BY d.created_at DESC, d.user_id DESC",
    )
    .bind("scheduled")
    .bind(&hash)
    .fetch_all(&state.db.pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for (uid, created, scheduled, starts) in rows {
        items.push(json!({
            "userID": uid,
            "userCreatedAt": created,
            "scheduledAt": scheduled,
            "deletionStartsAt": starts,
            "storageConsumed": users::usage(&state.db, uid).await?,
            "authenticatorEntryCount": users::authenticator_count(&state.db, uid).await?,
        }));
    }
    Ok(Json(json!({ "scheduledDeletions": items })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserOp {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(rename = "emailMFA")]
    email_mfa: Option<bool>,
}

fn user_op(req: &UserOp) -> ApiResult<i64> {
    // `binding:"required"` rejects a missing or zero id.
    if req.user_id == 0 {
        return Err(users::bad_request_plain());
    }
    Ok(req.user_id)
}

/// Museum `DisableTwoFactor` (TOTP only).
pub async fn disable_totp(state: &AppState, user_id: i64) -> ApiResult<()> {
    sqlx::query("UPDATE users SET is_two_factor_enabled = $1 WHERE user_id = $2")
        .bind(false)
        .bind(user_id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

async fn disable_2fa(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UserOp>,
) -> ApiResult<Json<Value>> {
    require_fresh(&state, &admin).await?;
    let uid = user_op(&req)?;
    audit(&admin, format_args!("disabling 2FA for account {uid}"));
    disable_totp(&state, uid).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct UpdateReferralRequest {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(default)]
    code: String,
}

async fn update_referral(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UpdateReferralRequest>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.code.is_empty() {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("updating referral code for {} to {}", req.user_id, req.code),
    );
    crate::routes::storage_bonus::update_code(&state, req.user_id, &req.code, true).await?;
    Ok(Json(json!({})))
}

async fn disable_passkeys(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UserOp>,
) -> ApiResult<Json<Value>> {
    require_fresh(&state, &admin).await?;
    let uid = user_op(&req)?;
    audit(&admin, format_args!("removing passkeys for account {uid}"));
    passkeys::delete_all(&state, uid).await?;
    Ok(Json(json!({})))
}

async fn update_email_mfa(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UserOp>,
) -> ApiResult<Json<Value>> {
    require_fresh(&state, &admin).await?;
    let uid = user_op(&req)?;
    let Some(enabled) = req.email_mfa else {
        return Err(users::bad_request("emailMFA is required"));
    };
    audit(
        &admin,
        format_args!("updating email mfa ({enabled}) for account {uid}"),
    );
    if !enabled {
        let srp: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM srp_auth WHERE user_id = $1")
            .bind(uid)
            .fetch_optional(&state.db.pool)
            .await?;
        if srp.is_none() {
            return Err(users::conflict("SRP setup incomplete"));
        }
    }
    sqlx::query("UPDATE users SET email_mfa = $1 WHERE user_id = $2")
        .bind(enabled)
        .bind(uid)
        .execute(&state.db.pool)
        .await?;
    Ok(Json(json!({})))
}

/// This server never blocks logins over storage warnings; the call only checks the user.
async fn unblock_storage_warning(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UserOp>,
) -> ApiResult<Json<Value>> {
    let uid = user_op(&req)?;
    audit(
        &admin,
        format_args!("unblocking storage warning login for account {uid}"),
    );
    users::get_user(&state, uid).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddOttRequest {
    #[serde(default)]
    email: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    expiry_time: i64,
}

async fn add_ott(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<AddOttRequest>,
) -> ApiResult<Json<Value>> {
    if req.email.is_empty() || req.code.is_empty() || req.app.is_empty() || req.expiry_time == 0 {
        return Err(users::bad_request_plain());
    }
    if !matches!(req.app.as_str(), "photos" | "auth" | "locker") {
        return Err(users::bad_request("invalid app"));
    }
    if req.expiry_time < now_micros() {
        return Err(users::bad_request("expiry time should be in future"));
    }
    if req.code.len() < 6 {
        return Err(users::bad_request(
            "invalid code length, should be at least 6 digit",
        ));
    }
    audit(&admin, "adding custom ott");
    login::add_ott(
        &state,
        &req.email,
        App::parse(&req.app),
        &req.code,
        req.expiry_time,
    )
    .await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct TerminateRequest {
    #[serde(default)]
    token: String,
    #[serde(rename = "userID", default)]
    user_id: i64,
}

async fn terminate_session(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<TerminateRequest>,
) -> ApiResult<Json<Value>> {
    if req.token.is_empty() || req.user_id == 0 {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("terminating session for user {}", req.user_id),
    );
    let hash = crate::auth::hash_token(&req.token);
    users::revoke_tokens(&state.db, req.user_id, Revoke::One(&hash)).await?;
    Ok(Json(json!({})))
}

async fn close_family(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UserOp>,
) -> ApiResult<Json<Value>> {
    let uid = user_op(&req)?;
    audit(&admin, format_args!("closing family for account {uid}"));
    family::close_family(&state, uid).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct ChangeEmailRequest {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(default)]
    email: String,
}

async fn change_email(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<ChangeEmailRequest>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.email.is_empty() {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("updating email for user: {}", req.user_id),
    );
    crate::routes::users::update_email(&state, req.user_id, &req.email).await?;
    Ok(Json(json!({})))
}

async fn delete_user(
    State(state): State<AppState>,
    admin: Admin,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    require_fresh(&state, &admin).await?;
    let email = users::normalize_email(q.get("email"));
    if email.is_empty() {
        return Err(users::bad_request_plain());
    }
    let user = user_by_email(&state, &email).await?;
    audit(&admin, format_args!("deleting account for {}", user.id));
    let cancelled = lifecycle::delete_account(&state, user.id, true).await?;
    Ok(Json(json!({
        "isSubscriptionCancelled": cancelled,
        "userID": user.id,
    })))
}

#[derive(Deserialize)]
struct RecoverRequest {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(rename = "emailID", default)]
    email_id: String,
}

async fn recover(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<RecoverRequest>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.email_id.is_empty() {
        return Err(users::bad_request_plain());
    }
    if !req.email_id.contains('@') {
        return Err(users::bad_request("invalid email"));
    }
    audit(
        &admin,
        format_args!("recovering account for {}", req.user_id),
    );
    lifecycle::recover_account(&state, req.user_id, &req.email_id, false).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct UpdateFlagRequest {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(default)]
    key: String,
    value: Option<String>,
}

async fn update_flag(
    State(state): State<AppState>,
    admin: Admin,
    Body(req): Body<UpdateFlagRequest>,
) -> ApiResult<Json<Value>> {
    if req.user_id == 0 || req.key.is_empty() || req.value.is_none() {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("updating flag:{} for {}", req.key, req.user_id),
    );
    crate::routes::remote_store::insert_or_update(
        &state,
        req.user_id,
        &req.key,
        req.value.as_deref(),
        true,
    )
    .await?;
    Ok(Json(json!({})))
}

async fn email_hash(State(state): State<AppState>, _admin: Admin, q: QueryMap) -> Json<Value> {
    let e = users::normalize_email(q.get("email"));
    Json(json!({ "hash": users::email_hash(&state.keys, &e) }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct HashesRequest {
    hashes: Vec<String>,
}

async fn emails_from_hashes(
    State(state): State<AppState>,
    _admin: Admin,
    Body(req): Body<HashesRequest>,
) -> ApiResult<Json<Value>> {
    let mut emails = Vec::new();
    for h in &req.hashes {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT encrypted_email, email_decryption_nonce FROM users WHERE email_hash = $1",
        )
        .bind(h)
        .fetch_optional(&state.db.pool)
        .await?;
        if let Some((ct, nonce)) = row.filter(|r| !r.0.is_empty()) {
            emails.push(users::decrypt_email(&state, &ct, &nonce)?);
        }
    }
    Ok(Json(json!({ "emails": emails })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSubscriptionRequest {
    #[serde(rename = "userID", default)]
    user_id: i64,
    #[serde(default)]
    storage: i64,
    #[serde(default)]
    payment_provider: String,
    #[serde(rename = "transactionID", default)]
    transaction_id: String,
    #[serde(rename = "productID", default)]
    product_id: String,
    #[serde(default)]
    expiry_time: i64,
    #[serde(default)]
    attributes: users::SubscriptionAttributes,
}

/// Museum `BillingController.UpdateSubscription`: replaces the plan stored on the user row.
async fn set_subscription(state: &AppState, r: &UpdateSubscriptionRequest) -> ApiResult<()> {
    let res = sqlx::query(
        "UPDATE users SET storage_quota = $1, subscription_product_id = $2, subscription_expiry = $3, \
         subscription_payment_provider = $4, subscription_transaction_id = $5, subscription_attributes = $6 \
         WHERE user_id = $7",
    )
    .bind(r.storage)
    .bind(&r.product_id)
    .bind(r.expiry_time)
    .bind(&r.payment_provider)
    .bind(&r.transaction_id)
    .bind(serde_json::to_string(&r.attributes).map_err(ApiError::internal)?)
    .bind(r.user_id)
    .execute(&state.db.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(users::not_found_plain());
    }
    Ok(())
}

async fn update_subscription(
    State(state): State<AppState>,
    admin: Admin,
    Body(r): Body<UpdateSubscriptionRequest>,
) -> ApiResult<Json<Value>> {
    if r.user_id == 0
        || r.storage == 0
        || r.transaction_id.is_empty()
        || r.product_id.is_empty()
        || r.expiry_time == 0
    {
        return Err(users::bad_request_plain());
    }
    audit(
        &admin,
        format_args!("updating subscription for user: {}", r.user_id),
    );
    set_subscription(&state, &r).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BonusRequest {
    bonus_type: String,
    action: String,
    #[serde(rename = "userID")]
    user_id: i64,
    year: i64,
    #[serde(rename = "storageInGB")]
    storage_in_gb: i64,
    testing: bool,
    #[serde(rename = "storageInMB")]
    storage_in_mb: i64,
    minute: i64,
}

impl BonusRequest {
    /// Museum `SupportUpdateBonus.Validate`.
    fn validate(&self) -> Result<(), &'static str> {
        let flexible = matches!(
            self.bonus_type.as_str(),
            "ADD_ON_SUPPORT" | "ADD_ON_NON_PROFIT"
        );
        if !flexible
            && !matches!(
                self.bonus_type.as_str(),
                "ADD_ON_BF_2023" | "ADD_ON_BF_2024"
            )
        {
            return Err("invalid bonus type");
        }
        if self.action == "ADD" || self.action == "UPDATE" {
            if self.testing {
                if self.storage_in_mb == 0 && self.minute == 0 {
                    return Err("invalid input, set in MB and minute for test");
                }
            } else if flexible {
                if self.year == 0 || self.year > 100 {
                    return Err("invalid input for year, only 1-100");
                }
                if self.storage_in_gb == 0 || self.storage_in_gb > 2000 {
                    return Err("invalid GB storage, only 1-2000");
                }
            } else {
                if ![50, 200, 500, 1000, 2000].contains(&self.storage_in_gb) {
                    return Err("invalid input for deal, only 50, 200, 500, 1000, 2000 allowed");
                }
                if ![3, 5, 10].contains(&self.year) {
                    return Err("invalid input for year, only 3 or 5 or 10");
                }
            }
        }
        Ok(())
    }
}

/// Go `time.AddDate(years, 0, 0)` on a µs UTC timestamp (Feb 29 rolls over to Mar 1).
pub fn add_years(micros: i64, years: i64) -> i64 {
    const DAY: i64 = 86_400 * 1_000_000;
    let days = micros.div_euclid(DAY);
    let rest = micros.rem_euclid(DAY);
    // Howard Hinnant's civil-from-days / days-from-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2) + years;
    let from_civil = |y: i64, m: i64, d: i64| {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    // Day overflow (Feb 29 in a non-leap year) normalises forward like Go.
    from_civil(y, m, 1) * DAY + (d - 1) * DAY + rest
}

async fn update_bonus(
    State(state): State<AppState>,
    admin: Admin,
    Body(r): Body<BonusRequest>,
) -> ApiResult<Json<Value>> {
    if r.bonus_type.is_empty() || r.action.is_empty() || r.user_id == 0 {
        return Err(users::bad_request_plain());
    }
    r.validate().map_err(users::bad_request)?;
    let now = now_micros();
    let (storage, valid_till) = if r.testing {
        (
            r.storage_in_mb * 1024 * 1024,
            now + r.minute * 60 * 1_000_000,
        )
    } else {
        (r.storage_in_gb * 1024 * 1024 * 1024, add_years(now, r.year))
    };
    let bonus_id = format!("{}-{}", r.bonus_type, r.user_id);
    match r.action.as_str() {
        "ADD" => {
            sqlx::query(
                "INSERT INTO storage_bonus (bonus_id, type, user_id, storage, valid_till, is_revoked, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&bonus_id)
            .bind(&r.bonus_type)
            .bind(r.user_id)
            .bind(storage)
            .bind(valid_till)
            .bind(false)
            .bind(now)
            .bind(now)
            .execute(&state.db.pool)
            .await?;
        }
        "UPDATE" => {
            sqlx::query(
                "UPDATE storage_bonus SET storage = $1, valid_till = $2, updated_at = $3 WHERE bonus_id = $4",
            )
            .bind(storage)
            .bind(valid_till)
            .bind(now)
            .bind(&bonus_id)
            .execute(&state.db.pool)
            .await?;
        }
        "REMOVE" => {
            let res = sqlx::query("DELETE FROM storage_bonus WHERE bonus_id = $1")
                .bind(&bonus_id)
                .execute(&state.db.pool)
                .await?;
            if res.rows_affected() == 0 {
                return Err(users::bad_request(format!(
                    "bonus not found for user {} with bonusID {bonus_id}",
                    r.user_id
                )));
            }
        }
        // Museum ignores unknown actions.
        _ => {}
    }
    audit(
        &admin,
        format_args!("User {} bonus {} {}", r.user_id, r.action, r.bonus_type),
    );
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::add_years;

    #[test]
    fn add_years_matches_go_add_date() {
        // 2024-02-29T12:00:00Z + 1y = 2025-03-01T12:00:00Z (Go normalises).
        let leap = 1_709_208_000_000_000;
        assert_eq!(add_years(leap, 1), 1_740_830_400_000_000);
        // 2023-06-15T00:00:00Z + 10y = 2033-06-15T00:00:00Z.
        assert_eq!(add_years(1_686_787_200_000_000, 10), 2_002_406_400_000_000);
    }
}
