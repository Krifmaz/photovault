//! HTTP client for the Ente Photos API (as served by Photovault's server or museum).
//!
//! The node talks to the server exactly like an Ente app: the same endpoints,
//! the same JSON, the auth token in `X-Auth-Token`, and its own
//! `X-Client-Package` so the server can tell it apart in the device list.
//!
//! - [`auth`]: sign in as a new device (SRP, email code, TOTP), [`Session`], logout.
//! - [`files`]: collections, collection diffs, downloads, file data (ML data,
//!   video previews) and presigned uploads.
//! - [`magic`]: public magic metadata updates (e.g. the `sv` stream flag).
//! - [`models`]: the JSON types, field-for-field as museum sends them.
//! - [`srp`]: the SRP-6a client maths.
//!
//! Requests are retried with exponential backoff on network errors, `429`
//! and `5xx`, except for the few non-idempotent login steps.
//!
//! OWNER: crypto/api agent.

pub mod auth;
pub mod files;
pub mod magic;
pub mod models;
pub mod srp;

pub use auth::{LoginFlow, LoginStep, Session};
pub use files::{Download, FileDataType};

use crate::crypto::CryptoError;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use std::time::Duration;

/// The `X-Client-Package` the node identifies itself with.
pub const CLIENT_PACKAGE: &str = "io.photovault.node";

/// Errors from API calls.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The server answered with a non-success status.
    #[error("HTTP {status} from {path}: {code} {message}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Request path (no query string, no secrets).
        path: String,
        /// Museum's error `code`, if the body had one.
        code: String,
        /// Museum's error `message`, if the body had one.
        message: String,
    },
    /// The token is missing, expired or revoked (`401`). Sign in again.
    #[error("not signed in or session expired (HTTP 401)")]
    Unauthorized,
    /// The connection failed (after retries).
    #[error("network error: {0}")]
    Network(#[source] reqwest::Error),
    /// The response body could not be parsed.
    #[error("unexpected response from {path}: {detail}")]
    Decode {
        /// Request path.
        path: String,
        /// What went wrong.
        detail: String,
    },
    /// Decryption failed: wrong password, or corrupt or foreign data.
    #[error("crypto: {0}")]
    Crypto(#[from] CryptoError),
    /// The password is wrong.
    #[error("incorrect password")]
    IncorrectPassword,
    /// The account only has passkey two-factor, which the node cannot perform.
    #[error(
        "this account requires a passkey for two-factor authentication, which the compute node cannot use; \
         enable an authenticator app (TOTP) as a second factor in the Ente app, then sign in again"
    )]
    PasskeyRequired,
    /// The server misbehaved (bad SRP proof, missing fields, cursor not advancing).
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The caller used the API in the wrong order or with bad input.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Local I/O failed (e.g. writing a download).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl ApiError {
    /// HTTP status, if this was an HTTP error.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            Self::Unauthorized => Some(401),
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Http { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, ApiError>;

/// Retry schedule for transient failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first (1 disables retries).
    pub max_attempts: u32,
    /// Delay before the first retry; doubles each time.
    pub base_delay: Duration,
    /// Upper bound on a single delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// An Ente API client bound to one server and (once signed in) one token.
#[derive(Clone)]
pub struct ApiClient {
    base: url::Url,
    /// Client for the API: carries headers, never follows redirects (so the
    /// token is never sent to a storage host).
    http: reqwest::Client,
    /// Client for presigned storage URLs: no auth headers, follows redirects.
    storage: reqwest::Client,
    token: Option<String>,
    client_version: String,
    retry: RetryPolicy,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("base", &self.base.as_str())
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl ApiClient {
    /// A client for the server at `base_url` (e.g. `https://photos.example.org`
    /// or `https://api.ente.io`), identifying as version `client_version`.
    pub fn new(base_url: &str, client_version: &str) -> Result<Self> {
        let mut base = url::Url::parse(base_url)
            .map_err(|e| ApiError::InvalidInput(format!("server URL {base_url:?}: {e}")))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(ApiError::InvalidInput(format!(
                "server URL must be http(s): {base_url}"
            )));
        }
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .user_agent(format!("photovault-node/{client_version}"))
            .build()
            .map_err(ApiError::Network)?;
        let storage = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .user_agent(format!("photovault-node/{client_version}"))
            .build()
            .map_err(ApiError::Network)?;
        Ok(Self {
            base,
            http,
            storage,
            token: None,
            client_version: client_version.to_owned(),
            retry: RetryPolicy::default(),
        })
    }

    /// A signed-in client for `session`.
    pub fn for_session(session: &Session, client_version: &str) -> Result<Self> {
        Ok(Self::new(&session.endpoint, client_version)?.with_token(session.token.clone()))
    }

    /// Use `token` (URL-safe base64, as sent in `X-Auth-Token`).
    pub fn with_token(mut self, token: String) -> Self {
        self.token = Some(token);
        self
    }

    /// Replace the retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The server's base URL.
    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    /// Whether a token is set.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn url(&self, path: &str) -> Result<url::Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|e| ApiError::InvalidInput(format!("path {path}: {e}")))
    }

    /// A request to `path` on the API with the standard headers and a
    /// 2-minute overall timeout (for JSON calls).
    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        Ok(self
            .request_untimed(method, path)?
            .timeout(Duration::from_secs(120)))
    }

    /// Like [`Self::request`] without the overall timeout (for downloads of
    /// large objects; stalls are still caught by the read timeout).
    fn request_untimed(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let mut rb = self
            .http
            .request(method, self.url(path)?)
            .header("X-Client-Package", CLIENT_PACKAGE)
            .header("X-Client-Version", &self.client_version);
        if let Some(t) = &self.token {
            rb = rb.header("X-Auth-Token", t);
        }
        Ok(rb)
    }

    /// Send `build()` with retries (if `retry`), mapping non-2xx to [`ApiError`].
    /// 3xx responses are returned as-is (callers that expect redirects handle them).
    async fn send<F>(&self, path: &str, retry: bool, build: F) -> Result<Response>
    where
        F: Fn() -> Result<RequestBuilder>,
    {
        let attempts = if retry {
            self.retry.max_attempts.max(1)
        } else {
            1
        };
        let mut delay = self.retry.base_delay;
        let mut attempt = 1;
        loop {
            let result = match build()?.send().await {
                Ok(resp) => check_status(path, resp).await,
                Err(e) => Err(ApiError::Network(e)),
            };
            match result {
                Err(e) if attempt < attempts && e.is_retryable() => {
                    tracing::debug!(path, attempt, error = %e, "retrying request");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(self.retry.max_delay);
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// GET `path` with `query`, parse JSON.
    async fn get_json<T: serde::de::DeserializeOwned, Q: serde::Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
    ) -> Result<T> {
        let resp = self
            .send(path, true, || {
                Ok(self.request(Method::GET, path)?.query(query))
            })
            .await?;
        parse_json(path, resp).await
    }

    /// `method` `path` with a JSON body; parse the JSON answer.
    async fn send_json<T, B>(&self, method: Method, path: &str, body: &B, retry: bool) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let resp = self
            .send(path, retry, || {
                Ok(self.request(method.clone(), path)?.json(body))
            })
            .await?;
        parse_json(path, resp).await
    }

    /// `method` `path` with a JSON body; ignore the answer's body.
    async fn send_json_no_content<B: serde::Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: &B,
        retry: bool,
    ) -> Result<()> {
        self.send(path, retry, || {
            Ok(self.request(method.clone(), path)?.json(body))
        })
        .await?;
        Ok(())
    }
}

/// Map a non-success response to an error (3xx is passed through).
async fn check_status(path: &str, resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() || status.is_redirection() {
        return Ok(resp);
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    let body = resp.text().await.unwrap_or_default();
    let (code, message) = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => (
            v.get("code")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_owned(),
            v.get("message")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_owned(),
        ),
        Err(_) => (String::new(), body.chars().take(200).collect()),
    };
    Err(ApiError::Http {
        status: status.as_u16(),
        path: path.to_owned(),
        code,
        message,
    })
}

async fn parse_json<T: serde::de::DeserializeOwned>(path: &str, resp: Response) -> Result<T> {
    let bytes = resp.bytes().await.map_err(ApiError::Network)?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode {
        path: path.to_owned(),
        detail: e.to_string(),
    })
}
