//! Cache-through mirror for the files the apps download from Ente's CDN, so
//! phones and desktops talk only to their own server:
//!
//! * `GET /photovault/models/<file>`: the on-device ML models (faces, pets,
//!   magic search, text recognition, document scan) from `models.ente.com`.
//!   Only the files in [`MODELS`] are served, and each download is checked
//!   against the size and SHA-256 the apps themselves pin
//!   (`rust/crates/ml/src/{assets.rs,ocr/assets.rs,scan/mod.rs}`).
//! * `GET /photovault/music/tracks.json`: the memories music manifest from
//!   `music.ente.com`, with each track URL rewritten to this server.
//! * `GET /photovault/music/<file>.mp3`: a track listed in that manifest.
//!
//! The first request downloads the file into `<config_dir>/assets/` and every
//! later one is served from there, with `Range`, a strong `ETag` (the file's
//! SHA-256) and `If-None-Match` / `If-Range`. The files are public on Ente's
//! CDN and carry no user data, so these routes need no login.
//! `PHOTOVAULT_ASSET_MIRROR=false` turns them off.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// One model file the apps download.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// Path on `models.ente.com` (and under `/photovault/models/`).
    pub path: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
}

const fn m(path: &'static str, size: u64, sha256: &'static str) -> ModelFile {
    ModelFile { path, size, sha256 }
}

/// Every model file the Photos apps fetch, with the size and hash they pin.
pub const MODELS: &[ModelFile] = &[
    // rust/crates/ml/src/assets.rs: faces, magic search, pets.
    m(
        "yolov5s_face_640_640_static_b1.onnx",
        32_355_091,
        "e047647409403d52696035ecd445792173e50d7fbdcccac97b958a585db9aa3d",
    ),
    m(
        "mobilefacenet_portable_static_b1.onnx",
        5_278_803,
        "0763fc33f54e138476194da95987e133b3e976075a6b1d3e1b2caedb251b1a36",
    ),
    m(
        "mobileclip_s2_image_gelu_opset20.onnx",
        143_057_352,
        "205a430af825e501c5138e5bb9abea942482a7a4fd4a680e98e47cf0830dce7e",
    ),
    m(
        "mobileclip_s2_text_opset18_quant.onnx",
        67_144_712,
        "d92f33dfcff83077fc2e0d3414250710efbb51795dfd89767bdbefb5fdc47322",
    ),
    m(
        "bpe_simple_vocab_16e6.txt",
        3_194_984,
        "67603cfda2e032ad77b5f8808af37789d590db664b26df8705d2bf8b3c553fc8",
    ),
    m(
        "yolov5s_pet_face_fp16_V2.onnx",
        14_758_020,
        "7876d97992eeb5f3a9f3b35eff5e0e133012928172a8b005093108d8c3ad2d1c",
    ),
    m(
        "dog_face_embedding128.onnx",
        4_141_071,
        "fb04d781eb1f7adf6ce3432dc0c5873f16cc051b5c98c14c754afb39e2b92462",
    ),
    m(
        "cat_face_embedding128.onnx",
        4_141_071,
        "32b10694a27f6404d2beaddbd64f07ad555f72dccb12ee60a7afe5dcf6aad6cd",
    ),
    m(
        "yolov5s_object_fp16.onnx",
        14_987_107,
        "113f0c18632eb2c4f6deebcd40eb01c676492e9b43923c2d336e1b4012fce9ef",
    ),
    m(
        "dog_body_embedding192.onnx",
        4_569_361,
        "1d85aa20358137e30f11c2d0baa9a2248b9997928d501fe15365d1fc57522770",
    ),
    m(
        "cat_body_embedding192.onnx",
        4_569_361,
        "62fb5891e61be69a96510d8ec56e7525a9541b0283e54574d27c86c9b4a26ddf",
    ),
    // rust/crates/ml/src/ocr/assets.rs: text recognition.
    m(
        "det_fixed_v1.onnx",
        5_548_458,
        "f655f119225b579fa8c3cbf64f6bb7cf56a26c9dc211706a25234d70a543ec8c",
    ),
    m(
        "cls_fixed_v1.onnx",
        590_475,
        "378d52a73263828d08d4dda37f1d0aac2e36cbd486fdc6351bf309d515610654",
    ),
    m(
        "rec_fixed_v1.onnx",
        17_031_601,
        "6dda4c0891af5a70c5b0f618b62588df7f3140616d1c560be5ec6496747b54fd",
    ),
    m(
        "PP-OCRv5/ppocrv5_dict.txt",
        74_012,
        "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b",
    ),
    // rust/crates/ml/src/scan/mod.rs: document scan.
    m(
        "document_segmentation_opt.onnx",
        5_085_426,
        "5ddcb87c70cb7674189e6fc148e84a490ca65b282276534210255a777d48a808",
    ),
];

/// Largest music track accepted from upstream.
const MAX_TRACK_BYTES: u64 = 64 * 1024 * 1024;
/// Largest manifest accepted from upstream.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// How long a fetched manifest is used before asking upstream again.
const MANIFEST_TTL: Duration = Duration::from_secs(60 * 60);
const MANIFEST_FILE: &str = "tracks.json";

/// Shared state: one download per file at a time, the HTTP client, and the
/// last manifest.
#[derive(Default)]
pub struct Mirror {
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    client: OnceLock<reqwest::Client>,
    manifest: tokio::sync::Mutex<Option<(Instant, serde_json::Value)>>,
}

impl std::fmt::Debug for Mirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mirror").finish_non_exhaustive()
    }
}

impl Mirror {
    fn client(&self) -> ApiResult<&reqwest::Client> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let c = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("photovault/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ApiError::internal)?;
        Ok(self.client.get_or_init(|| c))
    }

    fn lock(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        let mut g = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        g.entry(path.to_path_buf()).or_default().clone()
    }
}

/// Routes (empty when the mirror is off).
pub fn router(enabled: bool) -> Router<AppState> {
    if !enabled {
        return Router::new();
    }
    Router::new()
        .route("/photovault/models/{*path}", get(model))
        .route("/photovault/music/tracks.json", get(manifest))
        .route("/photovault/music/{*path}", get(track))
}

fn root(state: &AppState) -> PathBuf {
    state.config.config_dir.join("assets")
}

fn bad_gateway(msg: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "UPSTREAM_UNAVAILABLE", msg)
}

/// What the mirror expects of a downloaded file.
struct Expect<'a> {
    size: Option<u64>,
    sha256: Option<&'a str>,
    max: u64,
}

/// Download `url` to `dest` (via `dest.part`), checking `expect`. Returns the SHA-256.
async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expect: &Expect<'_>,
) -> ApiResult<String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| bad_gateway(format!("cannot fetch {url}: {e}")))?;
    if let (Some(want), Some(len)) = (expect.size, resp.content_length())
        && want != len
    {
        return Err(bad_gateway(format!(
            "{url}: expected {want} bytes, upstream sends {len}"
        )));
    }
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(ApiError::internal)?;
    }
    let part = with_suffix(dest, ".part");
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(ApiError::internal)?;
    let mut hasher = Sha256::new();
    let mut n: u64 = 0;
    let result: ApiResult<()> = async {
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| bad_gateway(format!("{url}: download interrupted: {e}")))?
        {
            n += chunk.len() as u64;
            if n > expect.max || expect.size.is_some_and(|s| n > s) {
                return Err(bad_gateway(format!("{url}: larger than expected")));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
        file.flush().await.map_err(ApiError::internal)?;
        file.sync_all().await.map_err(ApiError::internal)?;
        Ok(())
    }
    .await;
    drop(file);
    let sha = hex::encode(hasher.finalize());
    let check = result.and_then(|()| {
        if expect.size.is_some_and(|s| s != n) {
            return Err(bad_gateway(format!(
                "{url}: got {n} bytes, expected {}",
                expect.size.unwrap_or(0)
            )));
        }
        if expect.sha256.is_some_and(|h| !h.eq_ignore_ascii_case(&sha)) {
            return Err(bad_gateway(format!(
                "{url}: SHA-256 does not match the one the apps pin"
            )));
        }
        Ok(())
    });
    if let Err(e) = check {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::write(sidecar(dest), &sha)
        .await
        .map_err(ApiError::internal)?;
    tokio::fs::rename(&part, dest)
        .await
        .map_err(ApiError::internal)?;
    tracing::info!("cached {url} ({n} bytes)");
    Ok(sha)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn sidecar(dest: &Path) -> PathBuf {
    with_suffix(dest, ".sha256")
}

/// The cached file's SHA-256, downloading it first when missing.
async fn ensure(
    state: &AppState,
    url: &str,
    dest: &Path,
    expect: &Expect<'_>,
) -> ApiResult<String> {
    let lock = state.asset_mirror.lock(dest);
    let _g = lock.lock().await;
    if tokio::fs::metadata(dest).await.is_ok_and(|m| m.is_file()) {
        if let Some(h) = expect.sha256 {
            return Ok(h.to_ascii_lowercase());
        }
        if let Ok(h) = tokio::fs::read_to_string(sidecar(dest)).await {
            return Ok(h.trim().to_string());
        }
    }
    download(state.asset_mirror.client()?, url, dest, expect).await
}

/// Serve `file` with Range support and a strong ETag of `sha`.
async fn serve(file: &Path, sha: &str, immutable: bool, mut req: Request) -> Response {
    let etag = format!("\"{sha}\"");
    let h = req.headers();
    let matches = |v: &HeaderValue| {
        v.to_str()
            .is_ok_and(|s| s.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
    };
    if h.get(header::IF_NONE_MATCH).is_some_and(matches) {
        let mut r = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(v) = HeaderValue::from_str(&etag) {
            r.headers_mut().insert(header::ETAG, v);
        }
        return r;
    }
    // If-Range with our ETag: tower-http only understands dates, so decide here.
    if let Some(v) = req.headers_mut().remove(header::IF_RANGE)
        && v.to_str().ok() != Some(etag.as_str())
    {
        req.headers_mut().remove(header::RANGE);
    }
    req.headers_mut().remove(header::IF_MODIFIED_SINCE);
    req.headers_mut().remove(header::IF_UNMODIFIED_SINCE);
    let mut resp = match ServeFile::new(file).oneshot(req).await {
        Ok(r) => r.map(axum::body::Body::new),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let hs = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        hs.insert(header::ETAG, v);
    }
    hs.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=86400"
        }),
    );
    hs.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp
}

async fn model(
    State(state): State<AppState>,
    UrlPath(path): UrlPath<String>,
    req: Request,
) -> Response {
    let Some(spec) = MODELS.iter().find(|m| m.path == path) else {
        return ApiError::not_found("unknown model file").into_response();
    };
    let dest = root(&state).join("models").join(spec.path);
    let url = format!(
        "{}/{}",
        state.config.models_upstream.trim_end_matches('/'),
        spec.path
    );
    let expect = Expect {
        size: Some(spec.size),
        sha256: Some(spec.sha256),
        max: spec.size,
    };
    match ensure(&state, &url, &dest, &expect).await {
        Ok(sha) => serve(&dest, &sha, true, req).await,
        Err(e) => e.into_response(),
    }
}

/// The upstream manifest: fresh copy, else the last good one on disk.
async fn upstream_manifest(state: &AppState) -> ApiResult<serde_json::Value> {
    let mut cached = state.asset_mirror.manifest.lock().await;
    if let Some((at, v)) = cached.as_ref()
        && at.elapsed() < MANIFEST_TTL
    {
        return Ok(v.clone());
    }
    let file = root(state).join("music").join(MANIFEST_FILE);
    let url = format!(
        "{}/{MANIFEST_FILE}",
        state.config.music_upstream.trim_end_matches('/')
    );
    let fetched: ApiResult<serde_json::Value> = async {
        let resp = state
            .asset_mirror
            .client()?
            .get(&url)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| bad_gateway(format!("cannot fetch {url}: {e}")))?;
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_MANIFEST_BYTES)
        {
            return Err(bad_gateway(format!("{url}: too large")));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| bad_gateway(format!("{url}: {e}")))?;
        let v: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| bad_gateway(format!("{url}: not JSON: {e}")))?;
        if !v.get("tracks").is_some_and(serde_json::Value::is_array) {
            return Err(bad_gateway(format!("{url}: no tracks list")));
        }
        if let Some(dir) = file.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(ApiError::internal)?;
        }
        tokio::fs::write(&file, &body)
            .await
            .map_err(ApiError::internal)?;
        Ok(v)
    }
    .await;
    let v = match fetched {
        Ok(v) => v,
        Err(e) => match tokio::fs::read(&file)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            Some(v) => {
                tracing::warn!("{}; using the cached music manifest", e.message);
                v
            }
            None => return Err(e),
        },
    };
    *cached = Some((Instant::now(), v.clone()));
    Ok(v)
}

/// The path of `url` under the music upstream, if it is a track we can mirror.
fn track_path(upstream: &str, url: &str) -> Option<String> {
    let base = url::Url::parse(upstream).ok()?;
    let u = url::Url::parse(url).ok()?;
    if u.host_str() != base.host_str() || u.port_or_known_default() != base.port_or_known_default()
    {
        return None;
    }
    let prefix = base.path().trim_end_matches('/');
    let p = u.path().strip_prefix(prefix)?.trim_start_matches('/');
    let ok = !p.is_empty()
        && p.to_ascii_lowercase().ends_with(".mp3")
        && p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
        && u.query().is_none();
    ok.then(|| p.to_string())
}

/// Rewrite the manifest for this server: tracks that live on the music
/// upstream point at `/photovault/music/...`; any other track is dropped, so
/// the apps fetch nothing from elsewhere.
pub fn rewrite_manifest(
    v: &serde_json::Value,
    upstream: &str,
    public_url: &str,
) -> serde_json::Value {
    let base = format!("{}/photovault/music", public_url.trim_end_matches('/'));
    let mut out = v.clone();
    if let Some(tracks) = out
        .get_mut("tracks")
        .and_then(serde_json::Value::as_array_mut)
    {
        tracks.retain_mut(|t| {
            let Some(p) = t
                .get("url")
                .and_then(|u| u.as_str())
                .and_then(|u| track_path(upstream, u))
            else {
                return false;
            };
            t["url"] = serde_json::Value::String(format!("{base}/{p}"));
            true
        });
    }
    out
}

async fn manifest(State(state): State<AppState>) -> ApiResult<Response> {
    let v = upstream_manifest(&state).await?;
    let out = rewrite_manifest(&v, &state.config.music_upstream, &state.config.public_url);
    let mut r = Json(out).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    Ok(r)
}

async fn track(
    State(state): State<AppState>,
    UrlPath(path): UrlPath<String>,
    req: Request,
) -> Response {
    let upstream = state
        .config
        .music_upstream
        .trim_end_matches('/')
        .to_string();
    let url = format!("{upstream}/{path}");
    // Only tracks the manifest lists (and only safe relative paths).
    let listed = match upstream_manifest(&state).await {
        Ok(v) => v
            .get("tracks")
            .and_then(|t| t.as_array())
            .is_some_and(|ts| {
                ts.iter().any(|t| {
                    t.get("url")
                        .and_then(|u| u.as_str())
                        .and_then(|u| track_path(&upstream, u))
                        .is_some_and(|p| p == path)
                })
            }),
        Err(e) => return e.into_response(),
    };
    if !listed {
        return ApiError::not_found("unknown track").into_response();
    }
    let dest = root(&state).join("music").join("tracks").join(&path);
    let expect = Expect {
        size: None,
        sha256: None,
        max: MAX_TRACK_BYTES,
    };
    match ensure(&state, &url, &dest, &expect).await {
        Ok(sha) => serve(&dest, &sha, true, req).await,
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_unique_and_plain() {
        let mut seen = std::collections::HashSet::new();
        for m in MODELS {
            assert!(seen.insert(m.path), "duplicate {}", m.path);
            assert_eq!(m.sha256.len(), 64);
            assert!(!m.path.contains(".."));
        }
    }

    #[test]
    fn track_paths() {
        let up = "https://music.ente.com";
        assert_eq!(
            track_path(up, "https://music.ente.com/a/b.mp3").as_deref(),
            Some("a/b.mp3")
        );
        assert_eq!(track_path(up, "https://evil.example/a.mp3"), None);
        assert_eq!(track_path(up, "https://music.ente.com/a.wav"), None);
        assert_eq!(track_path(up, "https://music.ente.com/a.mp3?x=1"), None);
        assert_eq!(track_path(up, "https://music.ente.com:444/a.mp3"), None);
        assert_eq!(
            track_path("http://127.0.0.1:9/m", "http://127.0.0.1:9/m/t.mp3").as_deref(),
            Some("t.mp3")
        );
    }

    #[test]
    fn manifest_rewrite() {
        let v = serde_json::json!({"version": 3, "tracks": [
            {"id": "a", "url": "https://music.ente.com/tracks/a.mp3", "title": "A"},
            {"id": "b", "url": "https://elsewhere.example/b.mp3"}
        ]});
        let out = rewrite_manifest(&v, "https://music.ente.com", "https://photos.example.com/");
        assert_eq!(out["version"], 3);
        let t = out["tracks"].as_array().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(
            t[0]["url"],
            "https://photos.example.com/photovault/music/tracks/a.mp3"
        );
        assert_eq!(t[0]["title"], "A");
    }
}
