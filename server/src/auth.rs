//! Request authentication, mirroring museum's middleware.
//!
//! * Token: `X-Auth-Token` header or `?token=` query. Tokens are looked up by
//!   their SHA-256 hash in `tokens`, like museum.
//! * App: `X-Client-Package` starting with `io.ente.auth` → [`App::Auth`],
//!   `io.ente.locker` → [`App::Locker`], else [`App::Photos`].
//! * [`AuthUser`] = museum's `privateAPI`; [`StorageUser`] = `storageAPI`
//!   (rejects the Auth app); [`AdminUser`] = `adminAPI`.

use crate::db::now_micros;
use crate::state::AppState;
use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

/// Which Ente app is calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    /// Ente Photos (mobile, web, desktop).
    Photos,
    /// Ente Auth.
    Auth,
    /// Ente Locker.
    Locker,
}

impl App {
    /// Stored name (museum's `ente.App` string).
    pub fn as_str(self) -> &'static str {
        match self {
            App::Photos => "photos",
            App::Auth => "auth",
            App::Locker => "locker",
        }
    }
    /// Parse a stored name.
    pub fn parse(s: &str) -> Self {
        match s {
            "auth" => App::Auth,
            "locker" => App::Locker,
            _ => App::Photos,
        }
    }
    /// From request headers.
    pub fn from_parts(parts: &Parts) -> Self {
        let pkg = parts
            .headers
            .get("X-Client-Package")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if pkg.starts_with("io.ente.auth") {
            App::Auth
        } else if pkg.starts_with("io.ente.locker") {
            App::Locker
        } else {
            App::Photos
        }
    }
}

/// Rejection in museum's `{"error": "..."}` shape.
#[derive(Debug)]
pub struct AuthRejection(pub StatusCode, pub &'static str);

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

/// Raw token from header or query string.
pub fn token_from_parts(parts: &Parts) -> Option<String> {
    if let Some(v) = parts
        .headers
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        return Some(v.to_string());
    }
    parts.uri.query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned())
    })
}

/// SHA-256 of the token string, hex-encoded (column `tokens.token_hash`).
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// An authenticated user (any app).
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// User id.
    pub user_id: i64,
    /// Calling app.
    pub app: App,
    /// Hash of the session token (identifies the session).
    pub token_hash: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_parts(parts)
            .ok_or(AuthRejection(StatusCode::UNAUTHORIZED, "missing token"))?;
        let app = App::from_parts(parts);
        let token_hash = hash_token(&token);
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM tokens WHERE token_hash = $1 AND app = $2 AND is_deleted = FALSE",
        )
        .bind(&token_hash)
        .bind(app.as_str())
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "token lookup failed");
            AuthRejection(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to validate token",
            )
        })?;
        let (user_id,) = row.ok_or(AuthRejection(StatusCode::UNAUTHORIZED, "invalid token"))?;
        let ip = parts
            .headers
            .get("X-Forwarded-For")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .unwrap_or("")
            .trim()
            .to_string();
        let ua = parts
            .headers
            .get("User-Agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // Record session use inline (not in a detached task): on SQLite a concurrent
        // writer can make the request's own transaction fail with SQLITE_BUSY. Writes
        // are throttled to once a minute per session, so this is usually a no-op.
        let now = now_micros();
        let _ = sqlx::query(
            "UPDATE tokens SET last_used_at = $1, ip = $2, user_agent = $3 WHERE token_hash = $4 AND last_used_at < $5",
        )
        .bind(now)
        .bind(ip)
        .bind(ua)
        .bind(&token_hash)
        .bind(now - 60_000_000)
        .execute(&state.db.pool)
        .await;
        Ok(AuthUser {
            user_id,
            app,
            token_hash,
        })
    }
}

/// Authenticated user on a Photos/Locker endpoint (museum `storageAPI`): rejects the Auth app.
#[derive(Debug, Clone)]
pub struct StorageUser(pub AuthUser);

impl FromRequestParts<AppState> for StorageUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let u = AuthUser::from_request_parts(parts, state).await?;
        if u.app == App::Auth {
            return Err(AuthRejection(
                StatusCode::FORBIDDEN,
                "invalid app for endpoint",
            ));
        }
        Ok(StorageUser(u))
    }
}

/// Authenticated admin (user id listed in `config.admins`).
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let u = AuthUser::from_request_parts(parts, state).await?;
        if !state.config.admins.contains(&u.user_id) {
            return Err(AuthRejection(
                StatusCode::UNAUTHORIZED,
                "insufficient permissions",
            ));
        }
        Ok(AdminUser(u))
    }
}
