//! Go-compatible JSON request decoding.
//!
//! museum decodes bodies with Go's `encoding/json`, where `null` for a value-typed
//! field leaves its zero value. serde instead rejects `null` for non-`Option`
//! fields even with `#[serde(default)]`, and the apps do send explicit nulls
//! (e.g. Android sends `"apnsToken": null` to `/push/token`). This middleware
//! removes `null` members from JSON objects in request bodies before handlers
//! see them, which is exactly Go's behaviour for struct fields: a `null` and a
//! missing member decode the same way. Array elements are left untouched.

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::Response;

/// Largest JSON body rewritten (API bodies are small; object uploads go to `/storage`).
const MAX_JSON: usize = 64 * 1024 * 1024;

/// Remove `null` object members, recursively.
pub fn strip_nulls(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.retain(|_, val| !val.is_null());
            map.values_mut().for_each(strip_nulls);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

/// Middleware: rewrite JSON request bodies with nulls stripped (see module docs).
pub async fn middleware(req: Request, next: Next) -> Response {
    let is_json = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            ct.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        });
    if !is_json || req.uri().path().starts_with("/storage/") {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    let bytes = match to_bytes(body, MAX_JSON).await {
        Ok(b) => b,
        Err(_) => {
            return Response::builder()
                .status(axum::http::StatusCode::PAYLOAD_TOO_LARGE)
                .body(Body::from("{}"))
                .unwrap_or_default();
        }
    };
    let new_body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(mut v) if contains_null_member(&v) => {
            strip_nulls(&mut v);
            let out = serde_json::to_vec(&v).unwrap_or_else(|_| bytes.to_vec());
            parts.headers.remove(axum::http::header::CONTENT_LENGTH);
            Body::from(out)
        }
        // Unchanged (or not valid JSON: let the handler produce its usual error).
        _ => Body::from(bytes),
    };
    next.run(Request::from_parts(parts, new_body)).await
}

fn contains_null_member(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.values().any(|x| x.is_null() || contains_null_member(x)),
        serde_json::Value::Array(a) => a.iter().any(contains_null_member),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::strip_nulls;

    #[test]
    fn strips_object_nulls_only() {
        let mut v =
            serde_json::json!({"a": null, "b": {"c": null, "d": 1}, "e": [null, {"f": null}]});
        strip_nulls(&mut v);
        assert_eq!(v, serde_json::json!({"b": {"d": 1}, "e": [null, {}]}));
    }
}
