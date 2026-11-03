//! Request plumbing shared by the library routes.
//!
//! * [`JsonBody`] parses a JSON body like gin's `ShouldBindJSON`: any parse failure,
//!   an empty body included, is a 400 with an empty `{}` body.
//! * [`QueryMap`] gives museum's lenient query parsing (`strconv.ParseInt` errors ignored → 0).
//! * [`Empty200`] is gin's `c.Status(http.StatusOK)`: a 200 with no body.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use std::collections::HashMap;

/// A JSON request body; rejection is museum's 400 `{}`.
#[derive(Debug)]
pub struct JsonBody<T>(pub T);

impl<T: DeserializeOwned> FromRequest<AppState> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| {
                if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::status(StatusCode::PAYLOAD_TOO_LARGE)
                } else {
                    ApiError::status(StatusCode::BAD_REQUEST)
                }
            })?;
        serde_json::from_slice(&bytes)
            .map(JsonBody)
            .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
    }
}

/// Raw query parameters (first value wins, like gin's `c.Query`).
#[derive(Debug, Default)]
pub struct QueryMap(pub HashMap<String, String>);

impl<S: Send + Sync> FromRequestParts<S> for QueryMap {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let mut m = HashMap::new();
        if let Some(q) = parts.uri.query() {
            for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
                m.entry(k.into_owned()).or_insert_with(|| v.into_owned());
            }
        }
        Ok(QueryMap(m))
    }
}

impl QueryMap {
    /// The raw value, or `""` (gin's `c.Query`).
    pub fn str(&self, k: &str) -> &str {
        self.0.get(k).map(String::as_str).unwrap_or("")
    }
    /// `strconv.ParseInt(c.Query(k))` with the error ignored.
    pub fn i64(&self, k: &str) -> i64 {
        self.str(k).parse().unwrap_or(0)
    }
    /// Strict integer: missing or malformed is `None`.
    pub fn opt_i64(&self, k: &str) -> Option<i64> {
        self.0.get(k).and_then(|v| v.parse().ok())
    }
    /// gin form binding of a bool (`strconv.ParseBool`): `None` when missing, error when malformed.
    pub fn opt_bool(&self, k: &str) -> Result<Option<bool>, ApiError> {
        match self.0.get(k).map(String::as_str) {
            None => Ok(None),
            Some("1" | "t" | "T" | "true" | "TRUE" | "True") => Ok(Some(true)),
            Some("0" | "f" | "F" | "false" | "FALSE" | "False") => Ok(Some(false)),
            Some(_) => Err(ApiError::status(StatusCode::BAD_REQUEST)),
        }
    }
}

/// Parse a numeric path segment (`strconv.ParseInt` failure → 400 `{}`).
pub fn path_i64(s: &str) -> Result<i64, ApiError> {
    s.parse()
        .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
}

/// A 200 response with an empty body (gin's `c.Status(http.StatusOK)`).
#[derive(Debug, Clone, Copy)]
pub struct Empty200;

impl IntoResponse for Empty200 {
    fn into_response(self) -> Response {
        StatusCode::OK.into_response()
    }
}

/// `c.JSON(200, gin.H{})`.
pub fn empty_json() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({}))
}

/// A header value, or `""`.
pub fn header<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}
