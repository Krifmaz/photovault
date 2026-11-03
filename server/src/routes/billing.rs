//! Endpoints: /billing/* (plans, user plans, subscription, verify, store notifications,
//! Stripe endpoints) with self-hosted behaviour: no payment providers are configured, every
//! account is on the free plan (`users.storage_quota`) unless an admin changes it.
//!
//! Reference: museum `pkg/api/billing.go`, `pkg/controller/billing.go`.

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::users::extract::{Body, PaymentJwtUser};
use crate::users::{self, FREE_PLAN_YEARS};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/billing/plans/v2", get(plans))
        .route("/billing/user-plans", get(user_plans))
        .route("/billing/subscription", get(subscription))
        .route("/billing/verify-subscription", post(verify_subscription))
        .route("/billing/notify/android", post(provider_unavailable))
        .route("/billing/notify/ios", post(provider_unavailable))
        .route("/billing/notify/stripe", post(provider_unavailable))
        .route("/billing/notify/stripe/us", post(provider_unavailable))
        .route("/billing/stripe/customer-portal", get(stripe_private))
        .route("/billing/stripe/cancel-subscription", post(stripe_private))
        .route(
            "/billing/stripe/activate-subscription",
            post(stripe_private),
        )
        .route(
            "/billing/stripe-account-country",
            get(stripe_account_country),
        )
        .route("/billing/stripe/checkout-session", get(stripe_payment))
        .route("/billing/stripe/update-subscription", post(stripe_payment))
}

/// Museum `billing.GetFreePlan` with this instance's default quota.
fn free_plan(state: &AppState) -> Value {
    json!({
        "storage": if state.config.default_storage_bytes > 0 { state.config.default_storage_bytes } else { crate::capacity::UNLIMITED },
        "duration": FREE_PLAN_YEARS,
        "period": "year",
    })
}

async fn plans(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "plans": [], "freePlan": free_plan(&state) }))
}

async fn user_plans(State(state): State<AppState>, _user: AuthUser) -> Json<Value> {
    Json(json!({ "plans": [], "freePlan": free_plan(&state) }))
}

async fn subscription(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let sub = crate::capacity::subscription(&state, user.user_id).await?;
    Ok(Json(json!({ "subscription": sub })))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct VerifyRequest {
    #[serde(rename = "productID")]
    product_id: String,
    #[allow(dead_code)]
    payment_provider: String,
    #[allow(dead_code)]
    verification_data: String,
}

async fn verify_subscription(
    State(state): State<AppState>,
    user: AuthUser,
    Body(req): Body<VerifyRequest>,
) -> ApiResult<Json<Value>> {
    if req.product_id == "free" {
        let sub = crate::capacity::subscription(&state, user.user_id).await?;
        return Ok(Json(json!({ "subscription": sub })));
    }
    // No app store / play store verification on a self-hosted server.
    Err(users::bad_request("payment provider is not configured"))
}

/// Store notifications cannot be verified without provider credentials: 400 `{}`.
async fn provider_unavailable() -> crate::error::ApiError {
    users::bad_request_plain()
}

async fn stripe_private(_user: AuthUser) -> crate::error::ApiError {
    users::bad_request("payment provider is not configured")
}

async fn stripe_payment(_user: PaymentJwtUser) -> crate::error::ApiError {
    users::bad_request("payment provider is not configured")
}

async fn stripe_account_country(_user: PaymentJwtUser) -> Json<Value> {
    // Museum's `DefaultStripeAccountCountry` for non-Stripe subscriptions.
    Json(json!({ "stripeAccountCountry": "IN" }))
}
