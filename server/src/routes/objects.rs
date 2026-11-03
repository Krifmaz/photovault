//! Endpoints: `/storage/*`, the signed object URLs issued by [`crate::storage::local::LocalStore`].
//!
//! Supports whole-object `PUT`, `GET` (with `Range`), and S3-style multipart:
//! `PUT ?uploadId&partNumber` (returns an `ETag`) then `POST` with a
//! `CompleteMultipartUpload` XML body, which is what the Ente apps send.

use crate::state::AppState;
use crate::storage::local::{LocalStore, Op, Signed, write_atomic};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

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
    Some((
        Signed {
            exp,
            upload_id,
            part,
            len,
        },
        sig,
    ))
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "signature mismatch or expired").into_response()
}

async fn put_object(
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
    if !store.verify(Op::Put, &key, &s, &sig) {
        return forbidden();
    }
    let target = match (&s.upload_id, s.part) {
        (Some(u), Some(p)) => match store.multipart_dir(u) {
            Ok(d) if d.exists() => d.join(format!("part-{p:05}")),
            _ => return StatusCode::NOT_FOUND.into_response(),
        },
        _ => match store.path(&key) {
            Ok(p) => p,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        },
    };
    match stream_to_file(&target, body, s.len).await {
        Ok(md5ish) => {
            let etag = format!("\"{md5ish}\"");
            (StatusCode::OK, [(header::ETAG, etag)]).into_response()
        }
        Err(code) => code.into_response(),
    }
}

/// Stream a body to `target` atomically. Returns a hex SHA-256 used as ETag.
async fn stream_to_file(
    target: &std::path::Path,
    body: Body,
    want_len: Option<i64>,
) -> Result<String, StatusCode> {
    use sha2::{Digest, Sha256};
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let tmp = target.with_extension(format!(
        "tmp-{}",
        hex::encode(crate::crypto::random_bytes(6))
    ));
    let mut f = tokio::fs::File::create(&tmp)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut n: i64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| StatusCode::BAD_REQUEST)?;
        n += chunk.len() as i64;
        if want_len.is_some_and(|w| n > w) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(StatusCode::BAD_REQUEST);
        }
        hasher.update(&chunk);
        f.write_all(&chunk)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    if want_len.is_some_and(|w| n != w) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(StatusCode::BAD_REQUEST);
    }
    f.sync_all()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(f);
    tokio::fs::rename(&tmp, target)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(hex::encode(&hasher.finalize()[..16]))
}

async fn complete(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    body: String,
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
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(dir) = store.multipart_dir(upload_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // Part numbers from the XML body, in order. Missing body → all parts in order.
    let mut parts: Vec<u32> = body
        .split("<PartNumber>")
        .skip(1)
        .filter_map(|s| s.split("</PartNumber>").next()?.trim().parse().ok())
        .collect();
    if parts.is_empty() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            if let Some(n) = e
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("part-"))
                .and_then(|n| n.parse().ok())
            {
                parts.push(n);
            }
        }
        parts.sort_unstable();
    }
    let Ok(target) = store.path(&key) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut buf = Vec::new();
    for p in &parts {
        match tokio::fs::read(dir.join(format!("part-{p:05}"))).await {
            Ok(b) => buf.extend_from_slice(&b),
            Err(_) => return (StatusCode::BAD_REQUEST, "missing part").into_response(),
        }
    }
    if write_atomic(&target, &buf).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult><Key>{}</Key></CompleteMultipartUploadResult>",
        key.replace('&', "&amp;").replace('<', "&lt;")
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

async fn get_object(
    State(state): State<AppState>,
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
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(mut f) = tokio::fs::File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(meta) = f.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let size = meta.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| {
            let (a, b) = v.split_once('-')?;
            let start: u64 = if a.is_empty() {
                size.saturating_sub(b.parse().ok()?)
            } else {
                a.parse().ok()?
            };
            let end: u64 = if a.is_empty() || b.is_empty() {
                size.saturating_sub(1)
            } else {
                b.parse::<u64>().ok()?.min(size.saturating_sub(1))
            };
            (start <= end && start < size).then_some((start, end))
        });
    let common = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
    ];
    match range {
        Some((start, end)) => {
            if f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let len = end - start + 1;
            let stream = tokio_util::io::ReaderStream::new(f.take(len));
            (
                StatusCode::PARTIAL_CONTENT,
                common,
                [
                    (header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}")),
                    (header::CONTENT_LENGTH, len.to_string()),
                ],
                Body::from_stream(stream),
            )
                .into_response()
        }
        None => {
            let stream = tokio_util::io::ReaderStream::new(f);
            (
                StatusCode::OK,
                common,
                [(header::CONTENT_LENGTH, size.to_string())],
                Body::from_stream(stream),
            )
                .into_response()
        }
    }
}
