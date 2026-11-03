//! Endpoints: /storage-bonus/* (details, referral view, change code, claim referral).
//!
//! Reference: museum `pkg/api/storage_bonus.go`, `pkg/controller/storagebonus`,
//! `pkg/repo/storagebonus`.

use crate::auth::AuthUser;
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::users::extract::{Body, QueryMap};
use crate::users::{self, api};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Museum `codeLength`.
const CODE_LENGTH: usize = 6;
/// Museum `referralAmountInGb`.
const REFERRAL_GB: i64 = 10;
/// Museum `maxClaimableReferralAmount`.
const MAX_CLAIMABLE_GB: i64 = 2000;
/// Museum `entity.TenGbOnUpgrade`.
const PLAN_TYPE: &str = "10_GB_ON_UPGRADE";
/// Museum `MaxReferralCodeChangeAllowed`.
pub const MAX_CODE_CHANGES: i64 = 3;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/storage-bonus/details", get(details))
        .route("/storage-bonus/change-code", post(change_code))
        .route("/storage-bonus/referral-view", get(referral_view))
        .route("/storage-bonus/referral-claim", post(claim))
}

/// One row of `storage_bonus` as museum serialises it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bonus {
    storage: i64,
    #[serde(rename = "type")]
    bonus_type: String,
    created_at: i64,
    valid_till: i64,
    is_revoked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanStat {
    plan_type: String,
    total_count: i64,
    upgraded_count: i64,
}

/// Museum `random.GenerateAlphaNumString`: a letter followed by letters/digits, without the
/// easily confused `O` and `0`.
fn random_code(len: usize) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNPQRSTUVWXYZ";
    const ALNUM: &[u8] = b"ABCDEFGHIJKLMNPQRSTUVWXYZ123456789";
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while out.len() < len {
        let set = if i == 0 { ALPHA } else { ALNUM };
        // Rejection sampling keeps the distribution uniform.
        let limit = 256 - (256 % set.len());
        let b = crate::crypto::random_bytes(1)[0] as usize;
        if b < limit {
            out.push(set[b % set.len()] as char);
            i += 1;
        }
    }
    out
}

/// Museum `GetOrCreateReferralCode`.
pub async fn get_or_create_code(state: &AppState, user_id: i64) -> ApiResult<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT code FROM referral_codes WHERE user_id = $1 AND is_active = $2")
            .bind(user_id)
            .bind(true)
            .fetch_optional(&state.db.pool)
            .await?;
    if let Some((code,)) = row {
        return Ok(code);
    }
    let code = random_code(CODE_LENGTH);
    sqlx::query(
        "INSERT INTO referral_codes (code, user_id, is_active, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&code)
    .bind(user_id)
    .bind(true)
    .bind(now_micros())
    .execute(&state.db.pool)
    .await?;
    Ok(code)
}

async fn code_count(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM referral_codes WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&state.db.pool)
        .await?;
    Ok(n)
}

/// Museum `UpdateReferralCode` + `AddNewCode`. `admin_edit` skips the change limit.
pub async fn update_code(
    state: &AppState,
    user_id: i64,
    code: &str,
    admin_edit: bool,
) -> ApiResult<()> {
    let code = code.to_uppercase();
    if !code.chars().all(char::is_alphanumeric) {
        return Err(users::bad_request("code is not alphanumeric"));
    }
    let n = code.chars().count();
    if !(4..=20).contains(&n) {
        return Err(users::bad_request("code length should be between 4 and 8"));
    }
    if !admin_edit && code_count(state, user_id).await? > MAX_CODE_CHANGES {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "REFERRAL_CHANGE_LIMIT_REACHED",
            format!("max referral code change limit {MAX_CODE_CHANGES} reached"),
        ));
    }
    let (exists,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM referral_codes WHERE code = $1")
        .bind(&code)
        .fetch_one(&state.db.pool)
        .await?;
    if exists > 0 {
        return Err(api(
            StatusCode::BAD_REQUEST,
            "CODE_ALREADY_EXISTS",
            "This code already exists",
        ));
    }
    let mut tx = state.db.pool.begin().await?;
    sqlx::query("UPDATE referral_codes SET is_active = $1 WHERE user_id = $2")
        .bind(false)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO referral_codes (code, user_id, is_active, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&code)
    .bind(user_id)
    .bind(true)
    .bind(now_micros())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn has_applied(state: &AppState, user_id: i64) -> ApiResult<bool> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM referral_tracking WHERE invitee_id = $1")
            .bind(user_id)
            .fetch_one(&state.db.pool)
            .await?;
    Ok(n > 0)
}

/// Museum `ActiveStorageSurplusOfType(REFERRAL, SIGN_UP)`.
async fn active_referral_bonus(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(storage), 0) AS BIGINT) FROM storage_bonus WHERE user_id = $1 \
         AND type IN ('REFERRAL', 'SIGN_UP') AND is_revoked = $2 AND (valid_till = 0 OR valid_till > $3)",
    )
    .bind(user_id)
    .bind(false)
    .bind(now_micros())
    .fetch_one(&state.db.pool)
    .await?;
    Ok(n)
}

async fn details(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let u = users::get_user(&state, user.user_id).await?;
    let bonus_user = u.family_admin_id.unwrap_or(u.id);
    let rows: Vec<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT storage, type, created_at, valid_till, CASE WHEN is_revoked THEN 1 ELSE 0 END \
         FROM storage_bonus WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(bonus_user)
    .fetch_all(&state.db.pool)
    .await?;
    let bonuses: Vec<Bonus> = rows
        .into_iter()
        .map(|(storage, bonus_type, created_at, valid_till, r)| Bonus {
            storage,
            bonus_type,
            created_at,
            valid_till,
            is_revoked: r != 0,
        })
        .collect();
    let stats: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT plan_type, COUNT(*), CAST(COALESCE(SUM(CASE WHEN invitee_on_paid_plan THEN 1 ELSE 0 END), 0) AS BIGINT) \
         FROM referral_tracking WHERE invitor_id = $1 GROUP BY plan_type",
    )
    .bind(bonus_user)
    .fetch_all(&state.db.pool)
    .await?;
    let stats: Vec<PlanStat> = stats
        .into_iter()
        .map(|(plan_type, total_count, upgraded_count)| PlanStat {
            plan_type,
            total_count,
            upgraded_count,
        })
        .collect();
    let has_applied_code = bonuses.iter().any(|b| b.bonus_type == "SIGN_UP");
    let ref_count: i64 = stats.iter().map(|s| s.total_count).sum();
    let ref_upgrade: i64 = stats.iter().map(|s| s.upgraded_count).sum();
    Ok(Json(json!({
        "referralStats": stats,
        "bonuses": bonuses,
        "refCount": ref_count,
        "refUpgradeCount": ref_upgrade,
        "hasAppliedCode": has_applied_code,
    })))
}

#[derive(Deserialize)]
struct ChangeCodeRequest {
    code: String,
}

async fn change_code(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<ChangeCodeRequest>,
) -> ApiResult<Json<Value>> {
    if req.code.is_empty() {
        return Err(users::bad_request_plain());
    }
    update_code(&state, user.user_id, &req.code, false).await?;
    Ok(Json(json!({})))
}

async fn referral_view(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let u = users::get_user(&state, user.user_id).await?;
    let applied = has_applied(&state, u.id).await?;
    let is_family_member = matches!(u.family_admin_id, Some(a) if a != u.id);
    let code_user = if is_family_member {
        u.family_admin_id.unwrap_or(u.id)
    } else {
        u.id
    };
    let code = get_or_create_code(&state, code_user).await?;
    let claimed = active_referral_bonus(&state, code_user).await?;
    let changes = (code_count(&state, code_user).await? - 1).max(0);
    Ok(Json(json!({
        "planInfo": {
            "isEnabled": true,
            "planType": PLAN_TYPE,
            "storageInGB": REFERRAL_GB,
            "maxClaimableStorageInGB": MAX_CLAIMABLE_GB,
        },
        "code": code,
        "enableApplyCode": !applied && !is_family_member,
        "hasAppliedCode": applied,
        "claimedStorage": claimed,
        "isFamilyMember": is_family_member,
        "codeChangeAttempts": changes,
        "remainingCodeChangeAttempts": (MAX_CODE_CHANGES - changes).max(0),
    })))
}

async fn claim(
    State(state): State<AppState>,
    user: AuthUser,
    q: QueryMap,
) -> ApiResult<StatusCode> {
    let code = q.get("code");
    if code.is_empty() {
        return Err(users::bad_request_plain());
    }
    let u = users::get_user(&state, user.user_id).await?;
    let invalid = || api(StatusCode::NOT_FOUND, "INVALID_CODE", "Invalid code");
    let owner: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM referral_codes WHERE code = $1")
            .bind(code)
            .fetch_optional(&state.db.pool)
            .await?;
    let owner = owner.ok_or_else(invalid)?.0;
    match users::get_user(&state, owner).await {
        Ok(_) => {}
        Err(e) if e.status == StatusCode::GONE => return Err(invalid()),
        Err(e) => return Err(e),
    }
    if matches!(u.family_admin_id, Some(a) if a != u.id) {
        return Err(api(
            StatusCode::BAD_REQUEST,
            "ACCOUNT_NOT_ELIGIBLE",
            "User is not eligible to apply referral code",
        ));
    }
    if owner == u.id {
        return Err(users::bad_request_plain());
    }
    let already = || {
        api(
            StatusCode::CONFLICT,
            "CODE_ALREADY_APPLIED",
            "User has already applied code",
        )
    };
    if has_applied(&state, u.id).await? {
        return Err(already());
    }
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO referral_tracking (invitor_id, invitee_id, plan_type, invitee_on_paid_plan, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(owner)
    .bind(u.id)
    .bind(PLAN_TYPE)
    .bind(false)
    .bind(now)
    .execute(&mut *tx)
    .await;
    if let Err(e) = inserted {
        // A concurrent claim hit the UNIQUE(invitee_id) constraint.
        if e.as_database_error()
            .is_some_and(|d| d.is_unique_violation())
        {
            return Err(already());
        }
        return Err(e.into());
    }
    sqlx::query(
        "INSERT INTO storage_bonus (bonus_id, type, user_id, storage, valid_till, is_revoked, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(format!("SIGN_UP-{}", u.id))
    .bind("SIGN_UP")
    .bind(u.id)
    .bind(REFERRAL_GB * 1024 * 1024 * 1024)
    .bind(0_i64)
    .bind(false)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::OK)
}
