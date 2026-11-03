//! Request helpers with museum's (gin's) error behaviour: a body or query that
//! fails to bind is a bare `400 {}` instead of axum's plain-text rejections.

use crate::auth::{App, AuthRejection, AuthUser};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

/// JSON body; malformed or missing JSON is rejected with `400 {}` like gin's `ShouldBindJSON`.
#[derive(Debug)]
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = bytes::Bytes::from_request(req, state)
            .await
            .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))?;
        serde_json::from_slice(&bytes)
            .map(JsonBody)
            .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
    }
}

/// JSON body whose bind failures are museum's typed
/// `400 {"code":"BAD_REQUEST","message":"failed to validate req param"}`
/// (handlers that wrap bind errors in `NewBadRequestWithMessage`).
#[derive(Debug)]
pub struct CodedJson<T>(pub T);

/// The error [`CodedJson`] and its callers' `required` checks return.
pub fn invalid_param() -> ApiError {
    ApiError::bad_request("failed to validate req param")
}

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for CodedJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = bytes::Bytes::from_request(req, state)
            .await
            .map_err(|_| invalid_param())?;
        serde_json::from_slice(&bytes)
            .map(CodedJson)
            .map_err(|_| invalid_param())
    }
}

/// All query parameters (last value wins), never rejecting.
#[derive(Debug, Default)]
pub struct QueryMap(pub HashMap<String, String>);

impl<S: Send + Sync> FromRequestParts<S> for QueryMap {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let map = parts
            .uri
            .query()
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(QueryMap(map))
    }
}

impl QueryMap {
    /// A non-empty string parameter.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// A required integer parameter parsed like gin's form binding into Go's
    /// `int64`/`int16`: missing, empty, malformed or out-of-range is `400 {}`.
    pub fn int(&self, key: &str, min: i64, max: i64) -> Result<i64, ApiError> {
        self.str(key)
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| (min..=max).contains(v))
            .ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST))
    }
}

/// museum's `RequireAuthApp`: an authenticated user calling from the Auth app.
#[derive(Debug, Clone)]
pub struct AuthAppUser(pub AuthUser);

impl FromRequestParts<AppState> for AuthAppUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let u = AuthUser::from_request_parts(parts, state).await?;
        if u.app != App::Auth {
            return Err(AuthRejection(
                StatusCode::FORBIDDEN,
                "invalid app for endpoint",
            ));
        }
        Ok(AuthAppUser(u))
    }
}

/// Best-effort client IP (first `X-Forwarded-For` hop, else `X-Real-IP`).
pub fn client_ip(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default()
}
