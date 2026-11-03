//! API errors serialised exactly like museum's `ente.ApiError`: `{"code": "...", "message": "..."}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Result alias for handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// An error returned to clients. `code` values match museum's `ente.ErrorCode` strings,
/// since the apps branch on them (e.g. `USER_NOT_FOUND`, `STORAGE_LIMIT_EXCEEDED`).
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// Machine-readable code; `None` serialises an empty body `{}` like museum's untyped errors.
    pub code: Option<&'static str>,
    /// Human-readable detail (clients never display it).
    pub message: String,
}

impl ApiError {
    /// Build an error with a code.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: Some(code),
            message: message.into(),
        }
    }
    /// Bare status with `{}` body.
    pub fn status(status: StatusCode) -> Self {
        Self {
            status,
            code: None,
            message: String::new(),
        }
    }
    /// 400 `BAD_REQUEST`.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", msg)
    }
    /// 401 with museum's `{"error": ...}`-style body is produced by the auth layer; this is the typed variant.
    pub fn unauthorized() -> Self {
        Self::status(StatusCode::UNAUTHORIZED)
    }
    /// 403 `FORBIDDEN`.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "FORBIDDEN", msg)
    }
    /// 404 `NOT_FOUND`.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", msg)
    }
    /// 409 `CONFLICT`.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "CONFLICT", msg)
    }
    /// 500 with empty body; the cause is logged.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::status(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self.code {
            Some(code) => (
                self.status,
                Json(serde_json::json!({"code": code, "message": self.message})),
            )
                .into_response(),
            None => (self.status, Json(serde_json::json!({}))).into_response(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => ApiError::not_found("not found"),
            e => ApiError::internal(e),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(e)
    }
}

impl From<crate::storage::StorageError> for ApiError {
    fn from(e: crate::storage::StorageError) -> Self {
        match e {
            crate::storage::StorageError::NotFound => ApiError::not_found("object not found"),
            e => ApiError::internal(e),
        }
    }
}
