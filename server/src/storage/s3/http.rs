//! The one place that touches TLS: an HTTP client for the S3 backend.
//!
//! `reqwest` with `rustls` on the `ring` crypto provider (no aws-lc, no
//! OpenSSL). Certificates are verified against the system trust store via
//! `rustls-platform-verifier` (`/etc/ssl/certs` in the Docker image;
//! `SSL_CERT_FILE` is honoured). Plain `http://` endpoints (a Garage/MinIO on
//! the same Docker network) need no TLS at all.
//!
//! Errors are flattened to strings **without URLs**, so nothing that reaches
//! a log can carry a signed query string.

use bytes::Bytes;
use std::time::Duration;

/// A response with its body read into memory (S3 control responses are small;
/// object bodies are only read by `get_bytes`).
#[derive(Debug)]
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// Response headers.
    pub headers: reqwest::header::HeaderMap,
    /// Body.
    pub body: Bytes,
}

/// Thin wrapper over `reqwest::Client`.
#[derive(Debug, Clone)]
pub struct Client {
    inner: reqwest::Client,
}

impl Client {
    /// Build a client. Installs `ring` as the process's rustls provider.
    pub fn new() -> anyhow::Result<Self> {
        // Ignore "already installed" (several stores in one process, e.g. tests).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let inner = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(concat!("photovault/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| anyhow::anyhow!("building HTTP client: {}", e.without_url()))?;
        Ok(Self { inner })
    }

    /// Send one request and read the whole response.
    pub async fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Bytes,
    ) -> Result<Response, String> {
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
        let mut req = self.inner.request(method, url);
        for (k, v) in headers {
            // `host` is derived from the URL by the client; sending it twice confuses some servers.
            if !k.eq_ignore_ascii_case("host") {
                req = req.header(k, v);
            }
        }
        let res = req
            .body(body)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let body = res.bytes().await.map_err(|e| e.without_url().to_string())?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}
