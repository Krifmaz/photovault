//! The ML models, downloaded only from the user's own server
//! (`GET <server>/photovault/models/<file>`, the server's mirror of Ente's
//! model CDN, see `server/src/asset_mirror.rs`).
//!
//! The list, sizes and SHA-256 hashes are exactly the ones Ente pins in
//! `rust/crates/ml/src/assets.rs` for faces and magic search (face
//! detection, face embedding, CLIP image, CLIP text and its BPE vocabulary;
//! no pets, OCR or document scan).
//!
//! Path convention other modules rely on: a model is ready when
//! `<models_dir>/<file name>` exists. A file appears there only once it is
//! complete and verified: downloads go to `<file name>.part` (resumed with
//! `Range` / `If-Range`), are checked against the pinned size and SHA-256,
//! fsynced, and then renamed into place.
//!
//! Based on Ente (ente.com).

use crate::api::Api;
use crate::{CoreError, Result};
use reqwest::{Method, StatusCode, header};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// A model file Ente pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelFile {
    /// File name, on the mirror and in the models directory.
    pub name: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// SHA-256, lower-case hex.
    pub sha256: &'static str,
}

/// Face detection (YOLOv5s face).
pub(crate) const FACE_DETECTION: ModelFile = ModelFile {
    name: "yolov5s_face_640_640_static_b1.onnx",
    size: 32_355_091,
    sha256: "e047647409403d52696035ecd445792173e50d7fbdcccac97b958a585db9aa3d",
};
/// Face embedding (MobileFaceNet).
pub(crate) const FACE_EMBEDDING: ModelFile = ModelFile {
    name: "mobilefacenet_portable_static_b1.onnx",
    size: 5_278_803,
    sha256: "0763fc33f54e138476194da95987e133b3e976075a6b1d3e1b2caedb251b1a36",
};
/// CLIP image encoder (MobileCLIP S2).
pub(crate) const CLIP_IMAGE: ModelFile = ModelFile {
    name: "mobileclip_s2_image_gelu_opset20.onnx",
    size: 143_057_352,
    sha256: "205a430af825e501c5138e5bb9abea942482a7a4fd4a680e98e47cf0830dce7e",
};
/// CLIP text encoder (MobileCLIP S2, quantised).
pub(crate) const CLIP_TEXT: ModelFile = ModelFile {
    name: "mobileclip_s2_text_opset18_quant.onnx",
    size: 67_144_712,
    sha256: "d92f33dfcff83077fc2e0d3414250710efbb51795dfd89767bdbefb5fdc47322",
};
/// CLIP text tokenizer vocabulary.
pub(crate) const CLIP_VOCAB: ModelFile = ModelFile {
    name: "bpe_simple_vocab_16e6.txt",
    size: 3_194_984,
    sha256: "67603cfda2e032ad77b5f8808af37789d590db664b26df8705d2bf8b3c553fc8",
};

/// Every model faces and magic search need, in download order (small
/// face models first so faces can start soonest).
pub(crate) const MODELS: &[ModelFile] = &[
    FACE_EMBEDDING,
    FACE_DETECTION,
    CLIP_VOCAB,
    CLIP_TEXT,
    CLIP_IMAGE,
];

/// Where a model lives once complete and verified.
pub(crate) fn model_path(dir: &Path, m: &ModelFile) -> PathBuf {
    dir.join(m.name)
}

fn part_path(dir: &Path, m: &ModelFile) -> PathBuf {
    dir.join(format!("{}.part", m.name))
}

/// Whether every model of `list` is in place.
pub(crate) fn all_ready(dir: &Path, list: &[ModelFile]) -> bool {
    list.iter().all(|m| model_path(dir, m).is_file())
}

/// `(bytes done, bytes total)` of `list`: complete files plus partial
/// downloads. A few `stat` calls; cheap enough for status reads.
pub(crate) fn progress(dir: &Path, list: &[ModelFile]) -> (u64, u64) {
    let mut done = 0;
    let mut total = 0;
    for m in list {
        total += m.size;
        if model_path(dir, m).is_file() {
            done += m.size;
        } else if let Ok(md) = std::fs::metadata(part_path(dir, m)) {
            done += md.len().min(m.size);
        }
    }
    (done, total)
}

/// The paths `ente-ml` needs for faces + CLIP indexing.
#[cfg(feature = "ml")]
pub(crate) fn indexing_paths(dir: &Path) -> ente_ml::ModelPaths {
    let p = |m: &ModelFile| model_path(dir, m).to_string_lossy().into_owned();
    ente_ml::ModelPaths {
        face_detection: p(&FACE_DETECTION),
        face_embedding: p(&FACE_EMBEDDING),
        clip_image: p(&CLIP_IMAGE),
        clip_text: p(&CLIP_TEXT),
        ..Default::default()
    }
}

/// Download every missing model of `list` from `<base>/photovault/models/`
/// into `dir`. `on_progress(done, total)` is called as bytes arrive.
pub(crate) async fn ensure(
    base: &str,
    dir: &Path,
    list: &[ModelFile],
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let total: u64 = list.iter().map(|m| m.size).sum();
    let mut finished = 0u64;
    let api = Api::new(base);
    for m in list {
        if !model_path(dir, m).is_file() {
            download_one(&api, dir, m, &|got| on_progress(finished + got, total)).await?;
        }
        finished += m.size;
        on_progress(finished, total);
    }
    Ok(())
}

/// Download one model, resuming a partial download.
async fn download_one(
    api: &Api,
    dir: &Path,
    m: &ModelFile,
    on_bytes: &(dyn Fn(u64) + Send + Sync),
) -> Result<()> {
    let part = part_path(dir, m);
    let url = format!("{}/photovault/models/{}", api.base(), m.name);
    let mut have = tokio::fs::metadata(&part).await.map_or(0, |md| md.len());
    if have > m.size {
        tokio::fs::remove_file(&part).await?;
        have = 0;
    }
    if have < m.size {
        let mut rb = api.storage_request(Method::GET, &url);
        if have > 0 {
            rb = rb
                .header(header::RANGE, format!("bytes={have}-"))
                // Only resume the same bytes (the mirror's ETag is the SHA-256).
                .header(header::IF_RANGE, format!("\"{}\"", m.sha256));
        }
        let mut res = rb
            .send()
            .await
            .map_err(|e| crate::api::net_error(&e, api.base()))?;
        let status = res.status();
        let append = match status {
            StatusCode::PARTIAL_CONTENT if have > 0 && content_range_start(&res) == Some(have) => {
                true
            }
            StatusCode::OK => false,
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // The partial file is not a prefix we can extend: start over.
                let _ = tokio::fs::remove_file(&part).await;
                return Err(CoreError::network(format!(
                    "{}: download restarted",
                    m.name
                )));
            }
            s => {
                return Err(CoreError::Server {
                    status: s.as_u16(),
                    message: format!("model {}: {s}", m.name),
                });
            }
        };
        let mut out = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .await?;
        if !append {
            have = 0;
        }
        on_bytes(have);
        loop {
            let chunk = match res.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => {
                    out.flush().await?;
                    return Err(CoreError::network(e));
                }
            };
            have += chunk.len() as u64;
            if have > m.size {
                drop(out);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(CoreError::protocol(format!(
                    "model {}: larger than {} bytes",
                    m.name, m.size
                )));
            }
            out.write_all(&chunk).await?;
            on_bytes(have);
        }
        out.flush().await?;
        out.sync_all().await?;
    }
    if have != m.size {
        return Err(CoreError::network(format!(
            "model {}: incomplete ({have} of {} bytes)",
            m.name, m.size
        )));
    }
    let p = part.clone();
    let sha = tokio::task::spawn_blocking(move || sha256_file(&p))
        .await
        .map_err(|e| CoreError::storage(format!("hash task: {e}")))??;
    if !sha.eq_ignore_ascii_case(m.sha256) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(CoreError::protocol(format!(
            "model {}: SHA-256 mismatch",
            m.name
        )));
    }
    tokio::fs::rename(&part, model_path(dir, m)).await?;
    Ok(())
}

/// Start offset of a `Content-Range: bytes <start>-<end>/<len>` answer.
fn content_range_start(res: &reqwest::Response) -> Option<u64> {
    let v = res.headers().get(header::CONTENT_RANGE)?.to_str().ok()?;
    let rest = v.trim().strip_prefix("bytes ")?;
    rest.split('-').next()?.trim().parse().ok()
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn list_matches_ente_pins() {
        // Exactly the faces + CLIP files of rust/crates/ml/src/assets.rs.
        let names: Vec<_> = MODELS.iter().map(|m| m.name).collect();
        assert_eq!(names.len(), 5);
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.size > 1_000_000);
        }
        let total: u64 = MODELS.iter().map(|m| m.size).sum();
        assert_eq!(total, 251_030_942);
    }

    /// How the test server answers.
    #[derive(Clone, Copy)]
    enum Mode {
        /// Serve with Range support.
        Good,
        /// Send headers for the whole file, then only half of it, then close.
        CutFirst,
    }

    /// A tiny HTTP/1.1 server for one file at `/photovault/models/<name>`.
    async fn serve(name: &'static str, body: Vec<u8>, mode: Mode) -> (String, Arc<Log>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let log = Arc::new(Log::default());
        let log2 = log.clone();
        tokio::spawn(async move {
            let n = AtomicUsize::new(0);
            loop {
                let Ok((mut s, _)) = l.accept().await else {
                    return;
                };
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let k = s.read(&mut buf).await.unwrap();
                    if k == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..k]);
                }
                let req = String::from_utf8_lossy(&req).to_lowercase();
                let path_ok = req.starts_with(&format!("get /photovault/models/{name} "));
                let range: Option<u64> = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim().trim_end_matches('-').parse().ok());
                log2.0.lock().unwrap().push(range);
                let i = n.fetch_add(1, Ordering::SeqCst);
                if !path_ok {
                    let _ = s
                        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                        .await;
                    continue;
                }
                match (mode, i, range) {
                    (Mode::CutFirst, 0, _) => {
                        let head =
                            format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                        s.write_all(head.as_bytes()).await.unwrap();
                        s.write_all(&body[..body.len() / 2]).await.unwrap();
                        s.flush().await.unwrap();
                        drop(s);
                    }
                    (_, _, Some(start)) => {
                        let start = start as usize;
                        let part = &body[start..];
                        let head = format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{}/{}\r\n\r\n",
                            part.len(),
                            body.len() - 1,
                            body.len()
                        );
                        s.write_all(head.as_bytes()).await.unwrap();
                        s.write_all(part).await.unwrap();
                    }
                    _ => {
                        let head =
                            format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                        s.write_all(head.as_bytes()).await.unwrap();
                        s.write_all(&body).await.unwrap();
                    }
                }
            }
        });
        (url, log)
    }

    #[derive(Default)]
    struct Log(std::sync::Mutex<Vec<Option<u64>>>);

    fn spec(name: &'static str, body: &[u8]) -> ModelFile {
        ModelFile {
            name,
            size: body.len() as u64,
            sha256: Box::leak(hex::encode(Sha256::digest(body)).into_boxed_str()),
        }
    }

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[tokio::test]
    async fn download_resumes_with_range_and_verifies() {
        let data = body(300_000);
        let m = spec("tiny.onnx", &data);
        let (url, log) = serve("tiny.onnx", data.clone(), Mode::CutFirst).await;
        let dir = tempfile::tempdir().unwrap();
        let list = [m];
        let seen = std::sync::Mutex::new(Vec::new());
        let on = |d: u64, t: u64| seen.lock().unwrap().push((d, t));

        // The first answer is cut short: an error, a partial file, no model.
        assert!(ensure(&url, dir.path(), &list, &on).await.is_err());
        assert!(!all_ready(dir.path(), &list));
        let (done, total) = progress(dir.path(), &list);
        assert_eq!(total, data.len() as u64);
        assert!(done > 0 && done < total, "{done}");

        // The second run asks only for the rest and completes.
        ensure(&url, dir.path(), &list, &on).await.unwrap();
        assert!(all_ready(dir.path(), &list));
        assert_eq!(std::fs::read(model_path(dir.path(), &m)).unwrap(), data);
        assert!(!part_path(dir.path(), &m).exists());
        let ranges = log.0.lock().unwrap().clone();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0], None);
        assert_eq!(ranges[1], Some(done));
        assert_eq!(progress(dir.path(), &list), (total, total));
        assert_eq!(seen.lock().unwrap().last(), Some(&(total, total)));

        // Nothing more to fetch.
        ensure(&url, dir.path(), &list, &on).await.unwrap();
        assert_eq!(log.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn hash_mismatch_is_rejected_and_nothing_is_kept() {
        let data = body(10_000);
        let mut m = spec("bad.onnx", &data);
        // Same size, different content.
        let other = spec("bad.onnx", &body(10_001)[1..]);
        m.sha256 = other.sha256;
        let (url, _) = serve("bad.onnx", data, Mode::Good).await;
        let dir = tempfile::tempdir().unwrap();
        let err = ensure(&url, dir.path(), &[m], &|_, _| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SHA-256"), "{err}");
        assert!(!model_path(dir.path(), &m).exists());
        assert!(!part_path(dir.path(), &m).exists());
    }

    #[tokio::test]
    async fn oversized_and_missing_files_fail() {
        let data = body(5_000);
        let mut m = spec("big.onnx", &data);
        m.size = 4_000;
        let (url, _) = serve("big.onnx", data, Mode::Good).await;
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure(&url, dir.path(), &[m], &|_, _| {}).await.is_err());
        assert!(!model_path(dir.path(), &m).exists());
        assert!(!part_path(dir.path(), &m).exists());

        let missing = spec("other.onnx", b"x");
        let err = ensure(&url, dir.path(), &[missing], &|_, _| {})
            .await
            .unwrap_err();
        assert_eq!(err.status(), Some(404));
    }
}
