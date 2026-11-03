//! Endpoints: /users/* (OTT, email verify, SRP, 2FA/TOTP, passkey 2FA ceremony, attributes, recovery key, public keys, sessions, logout, details, delete, change email, account recovery, tokens).
//!
//! Reference: museum `cmd/museum/main.go` and the matching `pkg/api` / `pkg/controller` files.

use crate::state::AppState;
use axum::Router;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
}
