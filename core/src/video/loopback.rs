//! The loopback HTTP server that plays streamable videos.
//!
//! Bound to `127.0.0.1` on a random port, started on the first stream. Every
//! URL carries the server's secret token (32 random bytes, hex) and a
//! per-stream id (16 random bytes, hex):
//!
//! * `GET /<token>/<stream>/index.m3u8`: the playlist, without any key
//!   (segment `n` is `n.ts`, relative).
//! * `GET /<token>/<stream>/<n>.ts`: segment `n`, fetched from the preview
//!   object with an HTTP range request and decrypted (AES-128-CBC) in memory.
//!   A single `Range: bytes=` is honoured (`206`).
//!
//! Decrypted video only ever exists in memory (the last few segments per
//! stream) and in the player's buffer: nothing is written to disk. Keys stay
//! in core. Streams end with [`stop_all`] (the app calls it when the viewer
//! closes), are evicted beyond [`MAX_STREAMS`], and expire after
//! [`IDLE_EXPIRY`] without requests.

use super::playlist::Preview;
use crate::api::Api;
use crate::{CoreError, Result, crypto};
use reqwest::Method;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Streams kept at once (the viewer plays one; a few for quick swipes back).
pub const MAX_STREAMS: usize = 4;
/// A stream without requests for this long is dropped.
pub const IDLE_EXPIRY: Duration = Duration::from_secs(30 * 60);
/// Decrypted segments kept per stream (re-reads after a seek, prefetch).
const SEGMENT_CACHE: usize = 3;
/// Largest request head accepted.
const MAX_HEAD: usize = 16 * 1024;
/// Time to send a request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Where the segment object is, and how to ask for a fresh URL.
pub struct Source {
    /// Client for the account's server (refreshes the presigned URL).
    pub api: Api,
    /// File id.
    pub file_id: i64,
    /// Current presigned URL of the segment object.
    pub url: String,
}

/// One playable stream.
pub struct Stream {
    id: String,
    playlist: String,
    preview: Preview,
    source: tokio::sync::Mutex<Source>,
    cache: Mutex<VecDeque<(usize, Arc<Vec<u8>>)>>,
    inflight: tokio::sync::Mutex<()>,
    last_used: Mutex<Instant>,
}

impl Stream {
    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    fn cached(&self, n: usize) -> Option<Arc<Vec<u8>>> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(i, _)| *i == n)
            .map(|(_, b)| b.clone())
    }

    fn remember(&self, n: usize, b: Arc<Vec<u8>>) {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.iter().any(|(i, _)| *i == n) {
            return;
        }
        if c.len() >= SEGMENT_CACHE {
            c.pop_front();
        }
        c.push_back((n, b));
    }

    /// Segment `n`, decrypted.
    pub async fn segment(&self, n: usize) -> Result<Arc<Vec<u8>>> {
        if let Some(b) = self.cached(n) {
            return Ok(b);
        }
        // One fetch at a time per stream: the player's request and the
        // prefetch of the same segment do not both download it.
        let _one = self.inflight.lock().await;
        if let Some(b) = self.cached(n) {
            return Ok(b);
        }
        let seg = self
            .preview
            .segments
            .get(n)
            .ok_or_else(|| CoreError::storage(format!("no segment {n}")))?;
        let ct = self.fetch(seg.range).await?;
        let pt = Arc::new(crypto_offload(|| seg.decrypt(&ct))?);
        self.remember(n, pt.clone());
        Ok(pt)
    }

    /// Ciphertext of `range` of the segment object. An expired presigned URL
    /// (S3 answers 403, sometimes 400) is renewed once.
    async fn fetch(&self, range: Option<(u64, u64)>) -> Result<Vec<u8>> {
        let mut renewed = false;
        loop {
            let (api, url) = {
                let s = self.source.lock().await;
                (s.api.clone(), s.url.clone())
            };
            let r = get_range(&api, &url, range).await;
            match r {
                Err(CoreError::Server { status, .. })
                    if !renewed && matches!(status, 400 | 401 | 403 | 404) =>
                {
                    renewed = true;
                    let mut s = self.source.lock().await;
                    if s.url == url {
                        s.url = super::preview_url(&s.api, s.file_id).await?.ok_or(
                            CoreError::Server {
                                status: 404,
                                message: "the stream is gone".into(),
                            },
                        )?;
                    }
                }
                other => return other,
            }
        }
    }
}

fn crypto_offload<T>(f: impl FnOnce() -> T) -> T {
    crate::offload(f)
}

async fn get_range(api: &Api, url: &str, range: Option<(u64, u64)>) -> Result<Vec<u8>> {
    let res = api
        .send(|| {
            let rb = api.storage_request(Method::GET, url);
            match range {
                Some((off, len)) if len > 0 => rb.header(
                    reqwest::header::RANGE,
                    format!("bytes={off}-{}", off + len - 1),
                ),
                _ => rb,
            }
        })
        .await?;
    let partial = res.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let body = res.bytes().await.map_err(CoreError::network)?;
    match range {
        // The store ignored the range: cut it out ourselves.
        Some((off, len)) if !partial => {
            let end = off.saturating_add(len);
            if (body.len() as u64) < end {
                return Err(CoreError::protocol("segment object is truncated"));
            }
            Ok(body[off as usize..end as usize].to_vec())
        }
        Some((_, len)) if body.len() as u64 != len => {
            Err(CoreError::protocol("short segment range"))
        }
        _ => Ok(body.to_vec()),
    }
}

/// The running server.
pub struct Loopback {
    port: u16,
    token: String,
    streams: Mutex<VecDeque<Arc<Stream>>>,
}

static SERVER: OnceLock<Result<Arc<Loopback>, String>> = OnceLock::new();

/// The server, started on first use.
pub fn server() -> Result<Arc<Loopback>> {
    SERVER
        .get_or_init(|| start().map_err(|e| e.to_string()))
        .clone()
        .map_err(|e| CoreError::storage(format!("cannot start the video server: {e}")))
}

/// The server if it was started.
pub fn running() -> Option<Arc<Loopback>> {
    SERVER.get().and_then(|r| r.as_ref().ok().cloned())
}

fn start() -> std::io::Result<Arc<Loopback>> {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    std_listener.set_nonblocking(true)?;
    let port = std_listener.local_addr()?.port();
    let lb = Arc::new(Loopback {
        port,
        token: hex::encode(crypto::random_bytes(32)),
        streams: Mutex::new(VecDeque::new()),
    });
    let me = lb.clone();
    crate::runtime().spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(_) => return,
        };
        loop {
            let Ok((sock, peer)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let me = me.clone();
            tokio::spawn(async move {
                let _ = me.serve(sock).await;
            });
        }
    });
    Ok(lb)
}

impl Loopback {
    /// Register a stream; returns its playlist URL.
    pub fn add(&self, preview: Preview, source: Source) -> String {
        let id = hex::encode(crypto::random_bytes(16));
        let s = Arc::new(Stream {
            id: id.clone(),
            playlist: preview.local_playlist(),
            preview,
            source: tokio::sync::Mutex::new(source),
            cache: Mutex::new(VecDeque::new()),
            inflight: tokio::sync::Mutex::new(()),
            last_used: Mutex::new(Instant::now()),
        });
        let mut st = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        st.retain(|s| s.idle_for() < IDLE_EXPIRY);
        while st.len() >= MAX_STREAMS {
            st.pop_front();
        }
        st.push_back(s);
        format!(
            "http://127.0.0.1:{}/{}/{id}/index.m3u8",
            self.port, self.token
        )
    }

    /// Drop every stream (their keys and decrypted segments go with them).
    pub fn stop_all(&self) {
        self.streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn stream(&self, id: &str) -> Option<Arc<Stream>> {
        let st = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        let s = st
            .iter()
            .find(|s| crypto::ct_eq(s.id.as_bytes(), id.as_bytes()))?;
        if s.idle_for() >= IDLE_EXPIRY {
            return None;
        }
        Some(s.clone())
    }

    async fn serve(self: Arc<Self>, mut sock: tokio::net::TcpStream) -> std::io::Result<()> {
        let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut sock)).await {
            Ok(Ok(Some(h))) => h,
            _ => return respond(&mut sock, 400, "text/plain", &[], None, false).await,
        };
        let Some(req) = parse_head(&head) else {
            return respond(&mut sock, 400, "text/plain", &[], None, false).await;
        };
        let head_only = req.method == "HEAD";
        if req.method != "GET" && !head_only {
            return respond(&mut sock, 405, "text/plain", &[], None, false).await;
        }
        let mut parts = req.path.trim_start_matches('/').split('/');
        let (Some(token), Some(id), Some(name), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return respond(&mut sock, 404, "text/plain", &[], None, false).await;
        };
        if !crypto::ct_eq(token.as_bytes(), self.token.as_bytes()) {
            return respond(&mut sock, 404, "text/plain", &[], None, false).await;
        }
        let Some(stream) = self.stream(id) else {
            return respond(&mut sock, 404, "text/plain", &[], None, false).await;
        };
        stream.touch();
        if name == "index.m3u8" {
            let body = stream.playlist.as_bytes().to_vec();
            return respond(
                &mut sock,
                200,
                "application/vnd.apple.mpegurl",
                &body,
                None,
                head_only,
            )
            .await;
        }
        let Some(n) = name
            .strip_suffix(".ts")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n < stream.preview.segments.len())
        else {
            return respond(&mut sock, 404, "text/plain", &[], None, false).await;
        };
        let body = match stream.segment(n).await {
            Ok(b) => b,
            Err(_) => return respond(&mut sock, 502, "text/plain", &[], None, false).await,
        };
        // Prefetch the next segment while the player reads this one.
        if n + 1 < stream.preview.segments.len() && stream.cached(n + 1).is_none() {
            let s = stream.clone();
            tokio::spawn(async move {
                let _ = s.segment(n + 1).await;
            });
        }
        match req.range.map(|r| resolve_range(r, body.len() as u64)) {
            Some(Some((a, b))) => {
                let total = body.len();
                respond(
                    &mut sock,
                    206,
                    "video/mp2t",
                    &body[a as usize..=b as usize],
                    Some(format!("bytes {a}-{b}/{total}")),
                    head_only,
                )
                .await
            }
            Some(None) => {
                let cr = format!("bytes */{}", body.len());
                respond(&mut sock, 416, "text/plain", &[], Some(cr), false).await
            }
            None => respond(&mut sock, 200, "video/mp2t", &body, None, head_only).await,
        }
    }
}

/// Read up to the end of the request head (`\r\n\r\n`).
async fn read_head(sock: &mut tokio::net::TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(Some(buf));
        }
        if buf.len() > MAX_HEAD {
            return Ok(None);
        }
    }
}

/// A parsed request head.
#[derive(Debug, PartialEq)]
struct Head {
    method: String,
    path: String,
    range: Option<(Option<u64>, Option<u64>)>,
}

fn parse_head(head: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let path = target.split('?').next()?.to_string();
    let mut range = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case("range") {
            range = parse_range(v.trim());
        }
    }
    Some(Head {
        method,
        path,
        range,
    })
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n`; anything else (multi-range) is ignored.
fn parse_range(v: &str) -> Option<(Option<u64>, Option<u64>)> {
    let spec = v.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let a = if a.is_empty() {
        None
    } else {
        Some(a.trim().parse().ok()?)
    };
    let b = if b.is_empty() {
        None
    } else {
        Some(b.trim().parse().ok()?)
    };
    if a.is_none() && b.is_none() {
        return None;
    }
    Some((a, b))
}

/// Inclusive `(first, last)` within `size`, or `None` when unsatisfiable.
fn resolve_range(r: (Option<u64>, Option<u64>), size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    match r {
        (Some(a), b) if a < size => {
            Some((a, b.unwrap_or(size - 1).min(size - 1))).filter(|(a, b)| a <= b)
        }
        (None, Some(n)) if n > 0 => Some((size.saturating_sub(n), size - 1)),
        _ => None,
    }
}

async fn respond(
    sock: &mut tokio::net::TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    content_range: Option<String>,
    head_only: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Bad Gateway",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Accept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(cr) = content_range {
        head.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes()).await?;
    if !head_only {
        sock.write_all(body).await?;
    }
    sock.flush().await?;
    let _ = sock.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_and_ranges() {
        let h = parse_head(
            b"GET /t/s/0.ts?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=10-19\r\n\r\n",
        )
        .unwrap();
        assert_eq!(h.method, "GET");
        assert_eq!(h.path, "/t/s/0.ts");
        assert_eq!(h.range, Some((Some(10), Some(19))));
        assert_eq!(parse_range("bytes=5-"), Some((Some(5), None)));
        assert_eq!(parse_range("bytes=-5"), Some((None, Some(5))));
        assert_eq!(parse_range("bytes=1-2,4-5"), None);
        assert_eq!(parse_range("items=1-2"), None);
        assert_eq!(resolve_range((Some(10), Some(19)), 100), Some((10, 19)));
        assert_eq!(resolve_range((Some(90), Some(500)), 100), Some((90, 99)));
        assert_eq!(resolve_range((Some(10), None), 100), Some((10, 99)));
        assert_eq!(resolve_range((None, Some(10)), 100), Some((90, 99)));
        assert_eq!(resolve_range((None, Some(500)), 100), Some((0, 99)));
        assert_eq!(resolve_range((Some(100), None), 100), None);
        assert_eq!(resolve_range((Some(20), Some(10)), 100), None);
    }
}
