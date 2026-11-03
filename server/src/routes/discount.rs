//! Endpoints: /discount/claim (public) and /discount/add-coupons (admin).
//!
//! Claims run in the background and always answer with the same message, so the endpoint
//! does not reveal whether an email belongs to a paid account. Coupon codes are mailed via
//! [`crate::mail`] (logged when SMTP is not configured).
//!
//! Reference: museum `pkg/api/discountcoupon.go`, `pkg/controller/discountcoupon`.

use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::users::extract::{Admin, Body};
use crate::users::{self, family};
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Museum `MaxSendCount`.
const MAX_SEND_COUNT: i64 = 10;

/// Museum `AllowedProviders`.
const PROVIDERS: &[&str] = &[
    "Kagi",
    "Tuta",
    "Notesnook",
    "Windscribe",
    "LeaveMeAlone",
    "Obscura",
    "Addy",
    "Test",
];

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discount/claim", post(claim))
        .route("/discount/add-coupons", post(add_coupons))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimRequest {
    provider_name: String,
    email: String,
}

async fn claim(
    State(state): State<AppState>,
    Body(req): Body<ClaimRequest>,
) -> ApiResult<Json<Value>> {
    if req.provider_name.is_empty() || req.email.is_empty() {
        return Err(users::bad_request_plain());
    }
    if !PROVIDERS.contains(&req.provider_name.as_str()) {
        return Err(users::bad_request("Invalid provider name"));
    }
    tokio::spawn(async move {
        if let Err(e) = process_claim(&state, &req.provider_name, &req.email).await {
            tracing::warn!(provider = %req.provider_name, error = %e.message, "discount claim failed");
        }
    });
    Ok(Json(json!({
        "message": "If you are paid subscriber, you should shortly get an email."
    })))
}

async fn process_claim(state: &AppState, provider: &str, email: &str) -> ApiResult<()> {
    let Some(uid) = users::user_id_for_email(state, email).await? else {
        tracing::info!(provider, "user not found for discount coupon claim");
        return Ok(());
    };
    let user = users::get_user(state, uid).await?;
    if matches!(user.family_admin_id, Some(a) if a != user.id) {
        tracing::info!(uid, "family member, not eligible for discount coupon");
        return Ok(());
    }
    // Museum `HasActiveSelfOrFamilySubscription(user, true)`: a paid plan is required.
    if family::require_paid(&state.db, user.id).await.is_err() {
        tracing::info!(uid, "no active paid subscription for discount coupon");
        return Ok(());
    }
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT code, sent_count FROM discount_coupons WHERE provider_name = $1 AND claimed_by_user_id = $2",
    )
    .bind(provider)
    .bind(user.id)
    .fetch_optional(&state.db.pool)
    .await?;
    let code = match existing {
        Some((_, sent)) if sent >= MAX_SEND_COUNT => {
            tracing::info!(uid, "maximum send count for coupon reached");
            return Ok(());
        }
        Some((code, _)) => code,
        None => {
            let now = now_micros();
            // Claim the oldest unclaimed coupon atomically.
            let claimed: Option<(String,)> = sqlx::query_as(
                "UPDATE discount_coupons SET claimed_by_user_id = $1, claimed_at = $2, updated_at = $3 \
                 WHERE provider_name = $4 AND code = (SELECT code FROM discount_coupons WHERE provider_name = $5 \
                 AND claimed_by_user_id IS NULL ORDER BY created_at LIMIT 1) AND claimed_by_user_id IS NULL RETURNING code",
            )
            .bind(user.id)
            .bind(now)
            .bind(now)
            .bind(provider)
            .bind(provider)
            .fetch_optional(&state.db.pool)
            .await?;
            let Some((code,)) = claimed else {
                tracing::warn!(provider, "no unclaimed discount coupons available");
                return Ok(());
            };
            code
        }
    };
    let body =
        format!("Here is your {provider} code: {code}\n\nThank you for supporting this server.");
    crate::mail::deliver(
        &state.config,
        &user.email,
        &format!("Your {provider} code"),
        &body,
    )
    .await;
    sqlx::query(
        "UPDATE discount_coupons SET sent_count = sent_count + 1, updated_at = $1 WHERE provider_name = $2 AND code = $3",
    )
    .bind(now_micros())
    .bind(provider)
    .bind(&code)
    .execute(&state.db.pool)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddCouponsRequest {
    provider_name: String,
    codes: Option<Vec<String>>,
}

async fn add_coupons(
    State(state): State<AppState>,
    _admin: Admin,
    Body(req): Body<AddCouponsRequest>,
) -> ApiResult<Json<Value>> {
    let Some(codes) = req.codes else {
        return Err(users::bad_request_plain());
    };
    if req.provider_name.is_empty() {
        return Err(users::bad_request_plain());
    }
    if !PROVIDERS.contains(&req.provider_name.as_str()) {
        return Err(users::bad_request("Invalid provider name"));
    }
    if codes.is_empty() {
        return Err(users::bad_request("No coupon codes provided"));
    }
    let now = now_micros();
    for code in codes.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        sqlx::query(
            "INSERT INTO discount_coupons (provider_name, code, sent_count, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (provider_name, code) DO NOTHING",
        )
        .bind(&req.provider_name)
        .bind(code)
        .bind(0_i64)
        .bind(now)
        .bind(now)
        .execute(&state.db.pool)
        .await?;
    }
    Ok(Json(json!({ "message": "Coupons added successfully" })))
}
