//! Endpoints: /files/* (upload URLs, multipart, create/update, download, thumbnails, copy, trash, size, info, duplicates, magic metadata, share-url, count).
//!
//! Reference: museum `cmd/museum/main.go` and the matching `pkg/api` / `pkg/controller` files.

use crate::state::AppState;
use axum::Router;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
}
