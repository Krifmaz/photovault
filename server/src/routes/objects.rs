//! Endpoints: `/storage/*`, the signed object URLs issued by [`crate::storage::local::LocalStore`].
//!
//! Behaves like the subset of S3 the Ente apps use through presigned URLs:
//! * `PUT` whole object; when the URL bound a length/MD5 (v2 upload URLs) the
//!   request must carry that `Content-MD5` and the body must match, like a
//!   presigned S3 URL with `content-md5` in its signed headers. A
//!   `Content-MD5` sent without being bound is still verified (S3 does too).
//! * `PUT ?uploadId&partNumber` stores one part and returns its `ETag`
//!   (quoted MD5 hex, as S3 does); `POST` with the client's
//!   `CompleteMultipartUpload` XML assembles the parts (streamed, not buffered),
//!   checking order and ETags.
//! * `GET`/`HEAD` with single `Range` support (`206`/`416`).
//!
//! Errors use S3's XML error shape and codes. Signed query strings are never logged.

use crate::state::AppState;
use crate::storage::local::{LocalStore, Op, Signed, WriteError};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Largest `CompleteMultipartUpload` body accepted (10,000 parts fit easily).
const MAX_COMPLETE_XML: usize = 2 * 1024 * 1024;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/storage/{*key}",
        put(put_object)
            .get(get_object)
            .post(complete)
            .head(get_object),
    )
}

fn local(state: &AppState) -> Option<LocalStore> {
    state.local_store.clone()
}

fn signed(q: &HashMap<String, String>) -> Option<(Signed, String)> {
    let exp = q.get("pv-exp")?.parse().ok()?;
    let sig = q.get("pv-sig")?.clone();
    let upload_id = q.get("uploadId").cloned();
    let part = match q.get("partNumber") {
        Some(p) => Some(p.parse().ok()?),
        None => None,
    };
    let len = match q.get("pv-len") {
        Some(p) => Some(p.parse().ok()?),
        None => None,
    };
    let md5 = q.get("pv-md5").cloned();
    Some((
        Signed {
            exp,
            upload_id,
            part,
            len,
            md5,
        },
        sig,
    ))
}

/// An S3-style XML error response.
fn s3_error(status: StatusCode, code: &str, message: &str) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{code}</Code><Message>{message}</Message></Error>"
    );
    (status, [(header::CONTENT_TYPE, "application/xml")], xml).into_response()
}

fn forbidden() -> Response {
    s3_error(
        StatusCode::FORBIDDEN,
        "SignatureDoesNotMatch",
        "The request signature is invalid or has expired.",
    )
}

fn write_error(e: WriteError) -> Response {
    use WriteError::*;
    match e {
        LengthMismatch => s3_error(
            StatusCode::BAD_REQUEST,
            "IncompleteBody",
            "The body length does not match the signed Content-Length.",
        ),
        BadDigest => s3_error(
            StatusCode::BAD_REQUEST,
            "BadDigest",
            "The Content-MD5 you specified did not match what we received.",
        ),
        InvalidDigest => s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidDigest",
            "The Content-MD5 you specified is not valid.",
        ),
        Body => s3_error(
            StatusCode::BAD_REQUEST,
            "IncompleteBody",
            "The request body was not received completely.",
        ),
        NoSuchUpload => s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "The specified multipart upload does not exist.",
        ),
        InvalidPart => s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidPart",
            "One or more of the specified parts could not be found or its ETag did not match.",
        ),
        InvalidPartOrder => s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidPartOrder",
            "The list of parts was not in ascending order.",
        ),
        InvalidKey => s3_error(StatusCode::BAD_REQUEST, "InvalidArgument", "Invalid key."),
        Io(e) => {
            tracing::error!(error = %e, "local storage write failed");
            s3_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                "We encountered an internal error. Please try again.",
            )
        }
    }
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

async fn put_object(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(store) = local(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((s, sig)) = signed(&q) else {
        return forbidden();
    };
    if !store.verify(Op::Put, &key, &s, &sig) {
        return forbidden();
    }
    let sent_md5 = header_str(&headers, header::HeaderName::from_static("content-md5"));
    // A bound MD5 is a signed header: the request must present the same value.
    if let Some(bound) = &s.md5
        && sent_md5.map(str::trim) != Some(bound.as_str())
    {
        return forbidden();
    }
    // A bound length must match the declared one too (chunked bodies are checked while streaming).
    if let (Some(want), Some(have)) = (
        s.len,
        header_str(&headers, header::CONTENT_LENGTH).and_then(|v| v.parse::<i64>().ok()),
    ) && want != have
    {
        return forbidden();
    }
    let stream = body.into_data_stream();
    let res = match (&s.upload_id, s.part) {
        (Some(u), Some(p)) => store.write_part(u, p, stream, s.len, sent_md5).await,
        (None, None) => store.write_object(&key, stream, s.len, sent_md5).await,
        _ => return forbidden(),
    };
    match res {
        Ok(md5) => (StatusCode::OK, [(header::ETAG, format!("\"{md5}\""))]).into_response(),
        Err(e) => write_error(e),
    }
}

/// Parse `(PartNumber, ETag)` pairs from a `CompleteMultipartUpload` body.
fn parse_complete_xml(body: &str) -> Option<Vec<(u32, Option<String>)>> {
    let mut out = Vec::new();
    for chunk in body.split("<Part>").skip(1) {
        let part = chunk.split("</Part>").next()?;
        let num = between(part, "<PartNumber>", "</PartNumber>")?
            .trim()
            .parse()
            .ok()?;
        let etag = between(part, "<ETag>", "</ETag>").map(|e| {
            e.trim()
                .replace("&quot;", "\"")
                .replace("&#34;", "\"")
                .to_string()
        });
        out.push((num, etag));
    }
    Some(out)
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let rest = s.split_once(open)?.1;
    Some(rest.split_once(close)?.0)
}

async fn complete(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    let Some(store) = local(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((s, sig)) = signed(&q) else {
        return forbidden();
    };
    if !store.verify(Op::Complete, &key, &s, &sig) {
        return forbidden();
    }
    let Some(upload_id) = s.upload_id.as_deref() else {
        return forbidden();
    };
    let Ok(bytes) = axum::body::to_bytes(body, MAX_COMPLETE_XML).await else {
        return s3_error(
            StatusCode::BAD_REQUEST,
            "MalformedXML",
            "The XML you provided was not well-formed.",
        );
    };
    let Some(parts) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(parse_complete_xml)
    else {
        return s3_error(
            StatusCode::BAD_REQUEST,
            "MalformedXML",
            "The XML you provided was not well-formed.",
        );
    };
    match store.complete_multipart(&key, upload_id, &parts).await {
        Ok(etag) => complete_ok(&key, &etag),
        // A retried completion whose first attempt succeeded (response lost):
        // the upload is gone but the object exists. S3 also answers 200 here.
        Err(WriteError::NoSuchUpload)
            if matches!(
                crate::storage::ObjectStore::head(&store, &key).await,
                Ok(Some(_))
            ) =>
        {
            complete_ok(&key, "")
        }
        Err(e) => write_error(e),
    }
}

fn complete_ok(key: &str, etag: &str) -> Response {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompleteMultipartUploadResult><Key>{}</Key><ETag>&quot;{}&quot;</ETag></CompleteMultipartUploadResult>",
        esc(key),
        esc(etag)
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

/// Parse a single `bytes=` range against `size`. `Ok(None)`: no (or an
/// ignorable multi-) range; `Err(())`: unsatisfiable (416).
fn parse_range(v: &str, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = v.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None); // Multi-range: S3 ignores it and returns the whole object.
    }
    let Some((a, b)) = spec.trim().split_once('-') else {
        return Ok(None);
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 || size == 0 {
            return Err(());
        }
        return Ok(Some((size.saturating_sub(n), size - 1)));
    }
    let start: u64 = a.parse().map_err(|_| ())?;
    if start >= size {
        return Err(());
    }
    let end = if b.is_empty() {
        size - 1
    } else {
        let e: u64 = b.parse().map_err(|_| ())?;
        if e < start {
            return Ok(None);
        }
        e.min(size - 1)
    };
    Ok(Some((start, end)))
}

async fn get_object(
    State(state): State<AppState>,
    method: Method,
    Path(key): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(store) = local(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((s, sig)) = signed(&q) else {
        return forbidden();
    };
    if !store.verify(Op::Get, &key, &s, &sig) {
        return forbidden();
    }
    let Ok(path) = store.path(&key) else {
        return write_error(WriteError::InvalidKey);
    };
    let Ok(mut f) = tokio::fs::File::open(&path).await else {
        return s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchKey",
            "The specified key does not exist.",
        );
    };
    let Ok(meta) = f.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if !meta.is_file() {
        return s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchKey",
            "The specified key does not exist.",
        );
    }
    let size = meta.len();
    // Weak validator from size+mtime: objects are immutable once written.
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let etag = format!("\"{size:x}-{mtime:x}\"");
    let range = match header_str(&headers, header::RANGE) {
        Some(v) => match parse_range(v, size) {
            Ok(r) => r,
            Err(()) => {
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(header::CONTENT_RANGE, format!("bytes */{size}"))],
                )
                    .into_response();
            }
        },
        None => None,
    };
    let common = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
        (header::ETAG, etag),
    ];
    let head = method == Method::HEAD;
    let (status, start, len, content_range) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            start,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{size}")),
        ),
        None => (StatusCode::OK, 0, size, None),
    };
    let body = if head {
        Body::empty()
    } else {
        if start > 0 && f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        crate::metrics::STORAGE
            .bytes
            .get_or_create(&crate::metrics::DirectionLabels {
                direction: "download",
            })
            .inc_by(len);
        let stream = tokio_util::io::ReaderStream::with_capacity(f.take(len), 256 * 1024)
            .map(|r| r.map_err(axum::Error::new));
        Body::from_stream(stream)
    };
    let mut res = (status, common, body).into_response();
    let h = res.headers_mut();
    if let Ok(v) = len.to_string().parse() {
        h.insert(header::CONTENT_LENGTH, v);
    }
    if let Some(cr) = content_range
        && let Ok(v) = cr.parse()
    {
        h.insert(header::CONTENT_RANGE, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::{parse_complete_xml, parse_range};

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Ok(Some((0, 9))));
        assert_eq!(parse_range("bytes=90-", 100), Ok(Some((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Ok(Some((90, 99))));
        assert_eq!(parse_range("bytes=-200", 100), Ok(Some((0, 99))));
        assert_eq!(parse_range("bytes=50-500", 100), Ok(Some((50, 99))));
        assert_eq!(parse_range("bytes=100-", 100), Err(()));
        assert_eq!(parse_range("bytes=-0", 100), Err(()));
        assert_eq!(parse_range("bytes=0-1,5-6", 100), Ok(None));
        assert_eq!(parse_range("items=0-1", 100), Ok(None));
        assert_eq!(parse_range("bytes=0-", 0), Err(()));
    }

    #[test]
    fn complete_xml_from_web_client() {
        // Exactly what web/packages/gallery/services/upload/remote.ts builds.
        let xml = "<CompleteMultipartUpload>\n<Part><PartNumber>1</PartNumber><ETag>\"abc\"</ETag></Part>\n<Part><PartNumber>2</PartNumber><ETag>&quot;def&quot;</ETag></Part>\n</CompleteMultipartUpload>";
        assert_eq!(
            parse_complete_xml(xml),
            Some(vec![
                (1, Some("\"abc\"".into())),
                (2, Some("\"def\"".into()))
            ])
        );
        assert_eq!(
            parse_complete_xml("<CompleteMultipartUpload/>"),
            Some(vec![])
        );
        assert_eq!(
            parse_complete_xml("<Part><PartNumber>x</PartNumber></Part>"),
            None
        );
    }
}
