//! Photovault server: a Rust re-implementation of the Ente Photos server API
//! ("museum"), compatible with the unmodified Ente apps. Based on Ente (ente.com).
//!
//! The server only ever stores ciphertext produced by the apps; it never sees
//! photos, thumbnails, names, faces or locations.

#![warn(missing_docs)]

pub mod admin_cli;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod jwt;
pub mod mail;
pub mod metrics;
pub mod routes;
pub mod state;
pub mod storage;
pub mod testing;
pub mod users;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, Method};
use axum::middleware::Next;
use axum::response::Response;
use state::AppState;
use std::time::Instant;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

/// Build the full HTTP application with global middleware.
pub fn app(state: AppState) -> Router {
    let x_request_id = HeaderName::from_static("x-request-id");
    // museum's CORS: any origin, the headers the apps send, and ETag exposed for multipart uploads.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(AllowHeaders::mirror_request())
        .expose_headers([HeaderName::from_static("etag"), x_request_id.clone()])
        .max_age(std::time::Duration::from_secs(86400));
    routes::api()
        .layer(axum::middleware::from_fn_with_state(state.clone(), observe))
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid))
        .layer(cors)
        .with_state(state)
}

/// Structured access log + Prometheus metrics, labelled by route template (never the raw path).
async fn observe(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".into());
    let req_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let res = next.run(req).await;
    let status = res.status().as_u16();
    let secs = start.elapsed().as_secs_f64();
    let labels = metrics::HttpLabels {
        method: method.clone(),
        route: route.clone(),
        status,
    };
    state.metrics.http_requests.get_or_create(&labels).inc();
    state
        .metrics
        .http_latency
        .get_or_create(&labels)
        .observe(secs);
    tracing::info!(target: "access", method = %method, route = %route, status, latency_ms = secs * 1000.0, req_id = %req_id);
    res
}
