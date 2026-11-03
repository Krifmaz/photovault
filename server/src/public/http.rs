//! Small HTTP helpers shared by the public-link and cast routes.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use std::time::Duration;

/// museum's `PreSignedRequestValidityDuration` (7 days).
pub const SIGNED_URL_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Client IP like gin's `ClientIP()` with all proxies trusted: first `X-Forwarded-For`
/// entry, then `X-Real-IP`, then the socket address when known.
pub fn client_ip(parts: &Parts) -> String {
    let h = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    if let Some(v) = h("X-Forwarded-For").and_then(|v| v.split(',').next())
        && !v.trim().is_empty()
    {
        return v.trim().to_string();
    }
    if let Some(v) = h("X-Real-IP") {
        return v.to_string();
    }
    parts
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_default()
}

/// `User-Agent` header or empty.
pub fn user_agent(parts: &Parts) -> String {
    header_value(parts, "User-Agent").unwrap_or_default()
}

/// A header's value when present and non-empty.
pub fn header_value(parts: &Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// A query parameter's value when present and non-empty.
pub fn query_value(parts: &Parts, name: &str) -> Option<String> {
    parts.uri.query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, v)| k == name && !v.is_empty())
            .map(|(_, v)| v.into_owned())
    })
}

/// Header, falling back to a query parameter (museum `GetAccessToken` & co.).
pub fn header_or_query(parts: &Parts, header: &str, query: &str) -> Option<String> {
    header_value(parts, header).or_else(|| query_value(parts, query))
}

/// The matched route template, e.g. `/public-collection/info`.
pub fn route_path(parts: &Parts) -> String {
    parts
        .extensions
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string())
}

/// A raw JSON response for middleware-style rejections (museum's `gin.H{...}` bodies).
pub fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

/// JSON body extractor whose rejection is museum's bind error: 400 with `{}`.
#[derive(Debug)]
pub struct PJson<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for PJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))?;
        serde_json::from_slice(&bytes)
            .map(PJson)
            .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST))
    }
}

/// museum `ente.NewBadRequestWithMessage`: 400 `{"code":"BAD_REQUEST","message":…}`.
pub fn bad_request_msg(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", msg)
}

/// museum `ErrNotFoundError`: 404 `{"code":"NOT_FOUND","message":…}`.
pub fn not_found_code(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", msg)
}

/// Signed download URL for an object.
pub async fn signed_get(state: &AppState, key: &str) -> ApiResult<String> {
    Ok(state.store.presign_get(key, SIGNED_URL_TTL).await?)
}

/// 307 redirect (museum's legacy download endpoints).
pub fn redirect(url: &str) -> Response {
    let mut res = StatusCode::TEMPORARY_REDIRECT.into_response();
    if let Ok(v) = HeaderValue::from_str(url) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

/// museum `writeFileURLV3`: `{"url": …}`, with "not found" turned into 400 `NOT_FOUND`
/// (so clients can tell a missing object from a missing endpoint).
pub fn url_v3(r: ApiResult<String>) -> ApiResult<Json<serde_json::Value>> {
    match r {
        Ok(url) => Ok(Json(serde_json::json!({ "url": url }))),
        Err(e) if e.status == StatusCode::NOT_FOUND => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "NOT_FOUND",
            "requested object was not found",
        )),
        Err(e) => Err(e),
    }
}

/// Legacy endpoint: redirect to the URL or return the error.
pub fn url_redirect(r: ApiResult<String>) -> Response {
    match r {
        Ok(url) => redirect(&url),
        Err(e) => e.into_response(),
    }
}

/// Random access token like museum's `strings.ToUpper(shortuuid.New()[0:10])`.
pub fn new_access_token() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    random_string(ALPHABET, 10)
}

/// `n` characters drawn uniformly from `alphabet` with libsodium randomness.
pub fn random_string(alphabet: &[u8], n: usize) -> String {
    let mut out = String::with_capacity(n);
    while out.len() < n {
        for b in crate::crypto::random_bytes(n * 2) {
            // Rejection sampling keeps the distribution uniform.
            let limit = 256 - (256 % alphabet.len());
            if (b as usize) < limit && out.len() < n {
                out.push(alphabet[b as usize % alphabet.len()] as char);
            }
        }
    }
    out
}

/// A random RFC 4122 version-4 UUID string.
pub fn new_uuid() -> String {
    let mut b = crate::crypto::random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Parse a UUID in any case, with or without braces/`urn:uuid:` like Go's `uuid.Parse`,
/// returning the canonical lowercase hyphenated form.
pub fn parse_uuid(s: &str) -> Option<String> {
    let s = s.strip_prefix("urn:uuid:").unwrap_or(s);
    let s = s
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(s);
    let hexs: String = if s.len() == 36 {
        let b = s.as_bytes();
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        s.replace('-', "")
    } else if s.len() == 32 {
        s.to_string()
    } else {
        return None;
    };
    if hexs.len() != 32 || !hexs.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let h = hexs.to_ascii_lowercase();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids() {
        let u = new_uuid();
        assert_eq!(parse_uuid(&u).as_deref(), Some(u.as_str()));
        assert_eq!(
            parse_uuid("6BA7B810-9DAD-11D1-80B4-00C04FD430C8").as_deref(),
            Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        );
        assert!(parse_uuid("nope").is_none());
        assert!(parse_uuid("6ba7b810x9dad-11d1-80b4-00c04fd430c8").is_none());
    }

    #[test]
    fn tokens() {
        let t = new_access_token();
        assert_eq!(t.len(), 10);
        assert!(
            t.bytes()
                .all(|c| c.is_ascii_uppercase() || (b'2'..=b'9').contains(&c))
        );
    }
}
