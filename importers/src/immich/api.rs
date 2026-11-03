//! A small Immich REST client: JSON calls and streaming downloads, authorised
//! with an API key (`x-api-key` header).

use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use futures_util::StreamExt as _;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt as _;
use url::Url;

/// An HTTP error from Immich that carries the status, so callers can tell
/// "not found" or "forbidden" from transient failures.
#[derive(Debug, thiserror::Error)]
#[error("Immich {method} {path} returned {status}: {body}")]
pub struct HttpError {
    /// Request method.
    pub method: Method,
    /// Request path below the API root.
    pub path: String,
    /// Response status.
    pub status: StatusCode,
    /// Start of the response body, for diagnostics.
    pub body: String,
}

/// Status of an error, if it is an [`HttpError`].
pub fn status_of(e: &anyhow::Error) -> Option<StatusCode> {
    e.downcast_ref::<HttpError>().map(|h| h.status)
}

/// True for errors worth retrying: network failures, 5xx and 429.
pub fn is_transient(e: &anyhow::Error) -> bool {
    match status_of(e) {
        Some(s) => s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS,
        None => e.chain().any(|c| {
            c.downcast_ref::<reqwest::Error>().is_some()
                || c.downcast_ref::<std::io::Error>().is_some()
        }),
    }
}

/// A connection to one Immich server.
#[derive(Debug, Clone)]
pub struct ImmichClient {
    http: reqwest::Client,
    /// API root, always ending in `/api/`.
    api: Url,
    api_key: String,
}

impl ImmichClient {
    /// Connect to `base_url`, which may be the web address
    /// (`https://photos.example.com`) or the API root (`.../api`).
    pub fn new(
        base_url: &str,
        api_key: impl Into<String>,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let api = api_root(base_url)?;
        crate::tls::install_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(timeout)
            .user_agent(concat!("photovault-import/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            http,
            api,
            api_key: api_key.into(),
        })
    }

    /// The API root URL (ends in `/api/`).
    pub fn api_root(&self) -> &Url {
        &self.api
    }

    fn url(&self, path: &str) -> anyhow::Result<Url> {
        self.api
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("bad Immich path {path}"))
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut req = self
            .http
            .request(method.clone(), self.url(path)?)
            .header("x-api-key", &self.api_key);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("Immich {method} {path}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(HttpError {
                method,
                path: path.to_owned(),
                status,
                body: text.chars().take(300).collect(),
            }
            .into());
        }
        Ok(resp)
    }

    /// `GET path` and decode JSON.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let resp = self.send(Method::GET, path, None).await?;
        resp.json()
            .await
            .with_context(|| format!("decoding Immich GET {path}"))
    }

    /// `POST path` with a JSON body and decode JSON.
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<T> {
        let resp = self.send(Method::POST, path, Some(body)).await?;
        resp.json()
            .await
            .with_context(|| format!("decoding Immich POST {path}"))
    }

    /// `GET path` and stream the body into `dest` (created or truncated).
    /// Returns the response `Content-Type`, if any. On error `dest` may hold
    /// a partial file; the caller removes it.
    pub async fn download(&self, path: &str, dest: &Path) -> anyhow::Result<Option<String>> {
        let resp = self.send(Method::GET, path, None).await?;
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("creating {}", dest.display()))?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("downloading Immich {path}"))?;
            file.write_all(&chunk)
                .await
                .with_context(|| format!("writing {}", dest.display()))?;
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(content_type)
    }
}

/// Normalise a user-given Immich address to the API root ending in `/api/`.
pub fn api_root(base_url: &str) -> anyhow::Result<Url> {
    let mut u =
        Url::parse(base_url.trim()).with_context(|| format!("invalid Immich URL {base_url:?}"))?;
    anyhow::ensure!(
        matches!(u.scheme(), "http" | "https"),
        "Immich URL must be http(s): {base_url}"
    );
    u.set_query(None);
    u.set_fragment(None);
    let mut path = u.path().trim_end_matches('/').to_owned();
    if !path.ends_with("/api") {
        path.push_str("/api");
    }
    path.push('/');
    u.set_path(&path);
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_root_normalises() {
        for (i, o) in [
            ("https://p.example.com", "https://p.example.com/api/"),
            ("https://p.example.com/", "https://p.example.com/api/"),
            ("https://p.example.com/api", "https://p.example.com/api/"),
            ("https://p.example.com/api/", "https://p.example.com/api/"),
            (
                "http://10.0.0.2:2283/immich/",
                "http://10.0.0.2:2283/immich/api/",
            ),
        ] {
            assert_eq!(api_root(i).expect(i).as_str(), o);
        }
        assert!(api_root("ftp://x").is_err());
        assert!(api_root("not a url").is_err());
    }
}
