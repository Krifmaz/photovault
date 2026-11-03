//! Endpoints: /collections/* (albums, sharing with users, public URLs, diff, magic metadata).
//!
//! Reference: museum `cmd/museum/main.go` and the matching `pkg/api` / `pkg/controller` files.

use crate::state::AppState;
use axum::Router;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
}
