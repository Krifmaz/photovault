//! Photovault server: a Rust re-implementation of the Ente Photos server API
//! ("museum"), compatible with the unmodified Ente apps. Based on Ente (ente.com).
//!
//! The server only ever stores ciphertext produced by the apps; it never sees
//! photos, thumbnails, names, faces or locations.

#![warn(missing_docs)]

pub mod admin_cli;
pub mod auth;
pub mod capacity;
pub mod config;
pub mod crypto;
pub mod db;
pub mod discovery;
pub mod entities;
pub mod error;
pub mod gojson;
pub mod jwt;
pub mod keyfile;
pub mod legacy;
pub mod library;
pub mod mail;
pub mod metrics;
pub mod migrate_db;
pub mod preflight;
pub mod public;
pub mod routes;
pub mod settings;
pub mod social;
pub mod state;
pub mod storage;
pub mod testing;
pub mod users;
pub mod web;

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
    // museum's CORS: any origin, any request headers (the apps send X-Auth-Token,
    // X-Client-Package, Content-MD5, …), and ETag exposed so the web app can read
    // multipart part ETags from `/storage/*` PUTs. No credentials: auth is a header.
    // `PHOTOVAULT_WEB_ORIGINS` narrows that to the listed web app origins.
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins(&state.config.web_origins))
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(AllowHeaders::mirror_request())
        .expose_headers([
            HeaderName::from_static("etag"),
            HeaderName::from_static("content-length"),
            HeaderName::from_static("content-range"),
            HeaderName::from_static("accept-ranges"),
            x_request_id.clone(),
            // Public links hand device tokens back in this header (museum's CORS exposes it).
            HeaderName::from_static("x-link-device-token"),
        ])
        .max_age(std::time::Duration::from_secs(86400));
    let mut router = routes::api(&state);
    if let Some(dir) = state
        .config
        .web_dir
        .clone()
        .filter(|d| d.join("index.html").is_file())
    {
        router = router.fallback(move |req: Request| web::serve(dir.clone(), req));
    }
    router
        .layer(axum::middleware::from_fn(gojson::middleware))
        .layer(axum::middleware::from_fn_with_state(state.clone(), observe))
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid))
        .layer(cors)
        .with_state(state)
}

/// CORS origins: the configured list (plus any origin for `/photovault/info`, which
/// the apps' connect screens call from anywhere), or any origin when the list is empty.
fn allowed_origins(origins: &[String]) -> AllowOrigin {
    if origins.is_empty() {
        return AllowOrigin::mirror_request();
    }
    let list: Vec<axum::http::HeaderValue> = origins
        .iter()
        .filter_map(|o| match axum::http::HeaderValue::from_str(o) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!("ignoring invalid entry in PHOTOVAULT_WEB_ORIGINS: {o}");
                None
            }
        })
        .collect();
    AllowOrigin::predicate(move |origin, parts| {
        parts.uri.path() == "/photovault/info" || list.iter().any(|o| o == origin)
    })
}

/// Resolve on Ctrl-C or SIGTERM (what `docker stop` sends).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
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
    let ms = (secs * 10_000.0).round() / 10.0;
    // Docker's health probe every 30 s would drown the log at info level.
    if route == "/healthz" && status == 200 {
        tracing::debug!(target: "access", req_id = %req_id, "{method} {route} {status} {ms} ms");
    } else {
        tracing::info!(target: "access", req_id = %req_id, "{method} {route} {status} {ms} ms");
    }
    res
}
