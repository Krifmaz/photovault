//! Endpoints: `/healthz` (liveness + DB check) and `/metrics` (Prometheus).

use crate::state::AppState;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db.pool).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => {
            tracing::error!(error = %e, "health check: database unreachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"status": "db_unreachable"})),
            )
        }
    }
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let m = &state.metrics;
    let pool = &state.db.pool;
    m.db_pool_connections.set(i64::from(pool.size()));
    m.db_pool_idle.set(pool.num_idle() as i64);
    m.db_pool_max
        .set(i64::from(pool.options().get_max_connections()));
    let start = std::time::Instant::now();
    if sqlx::query("SELECT 1").execute(pool).await.is_ok() {
        m.db_ping_seconds.observe(start.elapsed().as_secs_f64());
    }
    (
        [(
            header::CONTENT_TYPE,
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )],
        state.metrics.render(),
    )
}
