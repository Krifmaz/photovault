//! HTTP client for Ente's API: the headers the apps send, JSON helpers,
//! retries with backoff on 429 / 5xx / network errors (idempotent requests
//! only; POSTs just on connect failures and 429), redirects followed
//! without the auth token, and TLS via rustls + ring with Mozilla's roots.
//!
//! Adapted from `importers/src/ente/api.rs` (branch `claude/project-thread-cp9x81`)
//! and `node/src/api/mod.rs` (branch `claude/project-thread-bmpbpi`).

use crate::{CoreError, Result};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// `X-Client-Package`: the native app's application id (contracts/apps.json).
/// Museum maps any package that is not an Auth or Locker one to Photos.
pub const CLIENT_PACKAGE: &str = "app.photovault.native";
/// `X-Client-Version`.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// rustls configuration: ring provider, Mozilla roots, h2 + http/1.1.
#[doc(hidden)]
pub fn tls_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    cfg
}

fn builder() -> reqwest::ClientBuilder {
    // A process-wide default provider, for anything that asks for one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config())
        .user_agent(concat!("photovault-core/", env!("CARGO_PKG_VERSION")))
}

/// Shared HTTP clients (connection pools): `(api, storage)`. The API client
/// never follows redirects (so the token never reaches a storage host); the
/// storage client follows them and never carries auth headers.
fn clients() -> &'static (reqwest::Client, reqwest::Client) {
    static C: OnceLock<(reqwest::Client, reqwest::Client)> = OnceLock::new();
    C.get_or_init(|| {
        let api = builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120))
            .build()
            .expect("HTTP client");
        let storage = builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120))
            .build()
            .expect("HTTP client");
        (api, storage)
    })
}

/// A plain client with a short overall timeout (server probing).
pub fn probe_client(timeout: Duration) -> reqwest::Client {
    builder()
        .redirect(reqwest::redirect::Policy::limited(3))
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .expect("HTTP client")
}

/// Map a reqwest error to a core error. `base` names the server.
pub fn net_error(e: &reqwest::Error, base: &str) -> CoreError {
    if e.is_connect() {
        CoreError::Unreachable {
            address: display_address(base),
        }
    } else {
        CoreError::network(e)
    }
}

/// `host[:port]` of a URL, for "can't reach …" messages.
pub fn display_address(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

/// How often to retry.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts, including the first.
    pub attempts: u32,
    /// First delay; doubled each time.
    pub base_delay: Duration,
    /// Upper bound of one delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_millis(400),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// A client for one Ente API base URL.
#[derive(Debug, Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    retry: RetryPolicy,
}

impl Api {
    /// A client for `base` (e.g. `https://photos.example.com`).
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: None,
            retry: RetryPolicy::default(),
        }
    }

    /// Use `token` as `X-Auth-Token`.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// The base URL.
    pub fn base(&self) -> &str {
        &self.base
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let mut rb = clients()
            .0
            .request(method, format!("{}{}", self.base, path))
            .header("X-Client-Package", CLIENT_PACKAGE)
            .header("X-Client-Version", CLIENT_VERSION);
        if let Some(t) = &self.token {
            rb = rb.header("X-Auth-Token", t);
        }
        rb
    }

    /// A request to a presigned storage URL: no auth token.
    pub fn storage_request(&self, method: Method, url: &str) -> RequestBuilder {
        clients()
            .1
            .request(method, url)
            .header("X-Client-Package", CLIENT_PACKAGE)
    }

    /// Send what `make` builds. Idempotent requests (GET, HEAD, PUT, DELETE,
    /// OPTIONS) are retried on 429, 5xx and network errors. Others (POST,
    /// PATCH: create a file or collection, SRP steps) are retried only when
    /// nothing reached the server: a failed connect, or a 429 refusal. A lost
    /// answer to a POST that the server did process must not be replayed
    /// (duplicate file, used SRP session). Non-2xx answers (other than 3xx)
    /// become [`CoreError`]s.
    pub async fn send(&self, make: impl Fn() -> RequestBuilder) -> Result<Response> {
        let mut retry = 0u32;
        loop {
            let last = retry + 1 >= self.retry.attempts;
            let delay = self
                .retry
                .base_delay
                .saturating_mul(2u32.saturating_pow(retry))
                .min(self.retry.max_delay);
            let (client, req) = make().build_split();
            let req = req.map_err(|e| net_error(&e, &self.base))?;
            let idempotent = is_idempotent(req.method());
            match client.execute(req).await {
                Ok(res) if res.status().is_success() || res.status().is_redirection() => {
                    return Ok(res);
                }
                Ok(res) => {
                    let status = res.status();
                    let retryable = status == StatusCode::TOO_MANY_REQUESTS
                        || (idempotent && status.is_server_error());
                    if retryable && !last {
                        tokio::time::sleep(delay).await;
                        retry += 1;
                        continue;
                    }
                    return Err(error_from(res).await);
                }
                Err(e) if !last && !e.is_builder() && (idempotent || e.is_connect()) => {
                    tokio::time::sleep(delay).await;
                    retry += 1;
                }
                Err(e) => return Err(net_error(&e, &self.base)),
            }
        }
    }

    /// `GET path?query`, parse JSON.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let res = self
            .send(|| self.request(Method::GET, path).query(query))
            .await?;
        parse(res).await
    }

    /// `GET path?query`, the raw answer (for 204-aware callers).
    pub async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Response> {
        self.send(|| self.request(Method::GET, path).query(query))
            .await
    }

    /// `method path` with a JSON body, parse the JSON answer.
    pub async fn json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let res = self.send_json(method, path, body).await?;
        parse(res).await
    }

    /// `method path` with a JSON body, ignore the answer.
    pub async fn json_no_content<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<()> {
        self.send_json(method, path, body).await.map(drop)
    }

    async fn send_json<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<Response> {
        let bytes = bytes::Bytes::from(serde_json::to_vec(body)?);
        self.send(|| {
            self.request(method.clone(), path)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes.clone())
        })
        .await
    }

    /// `method path?query` without a body; ignore the answer.
    pub async fn send_empty(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<()> {
        self.send(|| self.request(method.clone(), path).query(query))
            .await
            .map(drop)
    }

    /// `GET path` of an object (file, thumbnail). A redirect is followed with
    /// the storage client, without the auth token.
    pub async fn get_object(&self, path: &str) -> Result<Response> {
        let res = self.send(|| self.request(Method::GET, path)).await?;
        if !res.status().is_redirection() {
            return Ok(res);
        }
        let location = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| CoreError::protocol("redirect without Location"))?;
        let target = url::Url::parse(&format!("{}{}", self.base, path))
            .and_then(|u| u.join(location))
            .map_err(CoreError::protocol)?;
        self.send(|| self.storage_request(Method::GET, target.as_str()))
            .await
    }
}

/// Whether replaying `m` after a lost answer is harmless.
fn is_idempotent(m: &Method) -> bool {
    matches!(
        *m,
        Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS
    )
}

async fn error_from(res: Response) -> CoreError {
    let status = res.status();
    if status == StatusCode::UNAUTHORIZED {
        return CoreError::Unauthorized;
    }
    let body = res.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .or_else(|| v.get("code").and_then(|m| m.as_str()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    CoreError::Server {
        status: status.as_u16(),
        message,
    }
}

async fn parse<T: DeserializeOwned>(res: Response) -> Result<T> {
    let bytes = res.bytes().await.map_err(CoreError::network)?;
    serde_json::from_slice(&bytes).map_err(|e| CoreError::protocol(format!("bad answer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            display_address("http://192.168.1.10:8080"),
            "192.168.1.10:8080"
        );
        assert_eq!(
            display_address("https://photos.example.com/"),
            "photos.example.com"
        );
        assert_eq!(display_address("nonsense"), "nonsense");
    }

    #[test]
    fn only_idempotent_methods_are_replayed() {
        assert!(is_idempotent(&Method::GET));
        assert!(is_idempotent(&Method::PUT));
        assert!(!is_idempotent(&Method::POST));
        assert!(!is_idempotent(&Method::PATCH));
    }

    #[test]
    fn tls_config_builds() {
        let c = tls_config();
        assert_eq!(c.alpn_protocols.len(), 2);
    }
}
