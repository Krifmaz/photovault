//! HTTP plumbing: base URL, the headers the Ente apps send, JSON helpers and
//! retries with exponential backoff on 429, 5xx and network errors.

use anyhow::Context as _;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// `X-Client-Package` sent with every request. Museum maps any package that
/// is not an Auth or Locker one to the Photos app, which is what we want.
pub const CLIENT_PACKAGE: &str = "io.photovault.importer";

/// An HTTP error answer from the server.
#[derive(Debug, Clone, thiserror::Error)]
#[error("HTTP {status} from {method} {path}: {body}")]
pub struct ApiError {
    /// Status code.
    pub status: u16,
    /// Request method.
    pub method: String,
    /// Request path (no query string, so no secrets).
    pub path: String,
    /// Response body, truncated.
    pub body: String,
}

/// The HTTP status of `err` if it is (or wraps) an [`ApiError`].
pub fn status_of(err: &anyhow::Error) -> Option<u16> {
    err.chain()
        .find_map(|e| e.downcast_ref::<ApiError>())
        .map(|e| e.status)
}

/// How often and how patiently to retry.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts, including the first.
    pub attempts: u32,
    /// Delay before the first retry; doubled each time.
    pub base_delay: Duration,
    /// Upper bound for one delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 6,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    fn delay(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(retry.min(16)));
        retry_after.unwrap_or(exp).min(self.max_delay)
    }
}

/// A client for one Ente API endpoint.
#[derive(Debug, Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    retry: RetryPolicy,
}

impl Api {
    /// A client for `endpoint` (e.g. `https://photos.example.com` or
    /// `https://example.com/api`).
    pub fn new(endpoint: &url::Url) -> anyhow::Result<Self> {
        crate::tls::install_provider();
        let http = reqwest::Client::builder()
            .user_agent(concat!("photovault-import/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(300))
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            http,
            base: endpoint.as_str().trim_end_matches('/').to_string(),
            token: None,
            retry: RetryPolicy::default(),
        })
    }

    /// Use `token` as `X-Auth-Token` from now on.
    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// Replace the retry policy.
    pub fn set_retry_policy(&mut self, retry: RetryPolicy) {
        self.retry = retry;
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// A request to the API with the app headers (and the token, if any).
    fn api_request(&self, method: Method, path: &str) -> RequestBuilder {
        let mut rb = self
            .http
            .request(method, self.url(path))
            .header("X-Client-Package", CLIENT_PACKAGE)
            .header("X-Client-Version", env!("CARGO_PKG_VERSION"));
        if let Some(t) = &self.token {
            rb = rb.header("X-Auth-Token", t);
        }
        rb
    }

    /// A request to a presigned storage URL: only the public headers, never
    /// the auth token (the URL may point at another host).
    pub fn storage_request(&self, method: Method, url: &str) -> RequestBuilder {
        self.http
            .request(method, url)
            .header("X-Client-Package", CLIENT_PACKAGE)
    }

    /// Send the request `make` builds, retrying on 429, 5xx and network
    /// errors. Any other non-2xx answer is returned as an [`ApiError`].
    pub async fn send(
        &self,
        method: &Method,
        label: &str,
        make: impl Fn() -> RequestBuilder,
    ) -> anyhow::Result<Response> {
        let mut retry = 0u32;
        loop {
            let last = retry + 1 >= self.retry.attempts;
            match make().send().await {
                Ok(res) if res.status().is_success() => return Ok(res),
                Ok(res) => {
                    let status = res.status();
                    let retryable =
                        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                    if retryable && !last {
                        let after = retry_after(&res);
                        let d = self.retry.delay(retry, after);
                        tracing::warn!(%status, request = label, delay_ms = d.as_millis() as u64, "retrying");
                        tokio::time::sleep(d).await;
                        retry += 1;
                        continue;
                    }
                    let mut body = res.text().await.unwrap_or_default();
                    body.truncate(500);
                    return Err(ApiError {
                        status: status.as_u16(),
                        method: method.to_string(),
                        path: label.to_string(),
                        body,
                    }
                    .into());
                }
                Err(e) if !last && !e.is_builder() => {
                    let d = self.retry.delay(retry, None);
                    tracing::warn!(error = %e, request = label, delay_ms = d.as_millis() as u64, "network error, retrying");
                    tokio::time::sleep(d).await;
                    retry += 1;
                }
                Err(e) => return Err(anyhow::Error::new(e).context(format!("{method} {label}"))),
            }
        }
    }

    /// `GET path?query` and parse the JSON answer.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<T> {
        let res = self
            .send(&Method::GET, path, || {
                self.api_request(Method::GET, path).query(query)
            })
            .await?;
        parse(res, path).await
    }

    /// `method path` with a JSON body; parse the JSON answer.
    pub async fn json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let res = self.send_json(method, path, body).await?;
        parse(res, path).await
    }

    /// `method path` with a JSON body; ignore the answer's body.
    pub async fn json_no_content<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> anyhow::Result<()> {
        self.send_json(method, path, body).await.map(drop)
    }

    async fn send_json<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> anyhow::Result<Response> {
        let bytes = bytes::Bytes::from(serde_json::to_vec(body)?);
        self.send(&method, path, || {
            self.api_request(method.clone(), path)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes.clone())
        })
        .await
    }
}

async fn parse<T: DeserializeOwned>(res: Response, path: &str) -> anyhow::Result<T> {
    let bytes = res
        .bytes()
        .await
        .with_context(|| format!("reading response of {path}"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing response of {path}"))
}

fn retry_after(res: &Response) -> Option<Duration> {
    res.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let p = RetryPolicy {
            attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
        };
        assert_eq!(p.delay(0, None), Duration::from_millis(100));
        assert_eq!(p.delay(1, None), Duration::from_millis(200));
        assert_eq!(p.delay(2, None), Duration::from_millis(400));
        assert_eq!(p.delay(3, None), Duration::from_millis(500));
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(9))),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn status_is_found_through_context() {
        let e: anyhow::Error = ApiError {
            status: 404,
            method: "GET".into(),
            path: "/x".into(),
            body: String::new(),
        }
        .into();
        let e = e.context("outer");
        assert_eq!(status_of(&e), Some(404));
    }
}
