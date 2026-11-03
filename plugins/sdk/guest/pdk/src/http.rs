//! Outbound HTTP(S), for plugins granted `network` hosts.
//!
//! This is a thin layer over Extism's `http_request`. The host only lets a
//! request through when its host name matches one of the patterns the user
//! granted (`[permissions] network = ["api.example.com"]`); anything else
//! fails. Keep in mind that whatever a plugin sends leaves the user's device:
//! never send photo contents or metadata to a service the user has not been
//! told about in the plugin's description.
//!
//! ```no_run
//! # use photovault_plugin_pdk as pdk;
//! # fn f() -> Result<(), pdk::Error> {
//! let req = pdk::http::HttpRequest::new("https://api.example.com/v1/status");
//! let res = pdk::http::request(&req, None)?;
//! let status: serde_json::Value = res.json()?;
//! # Ok(()) }
//! ```

use crate::abi::HostError;
use std::collections::BTreeMap;

/// The request type, re-exported from Extism: `HttpRequest::new(url)`,
/// `.with_method("POST")`, `.with_header("k", "v")`.
pub use extism_pdk::HttpRequest;

/// A completed HTTP exchange.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Response headers, names in lower case.
    pub headers: BTreeMap<String, String>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Parse the body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// The body as text (invalid UTF-8 is replaced).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// A header by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Whether the status is 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Send `request` with an optional body.
pub fn request(request: &HttpRequest, body: Option<&[u8]>) -> Result<HttpResponse, HostError> {
    crate::backend::http(request, body)
}

/// The lower-case host name of an `http(s)://` URL, without port or userinfo.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Whether `host` matches a granted pattern (`api.example.com` matches only
/// itself; `*.example.com` matches any subdomain but not `example.com`).
pub fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(suffix) => host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.len() > 1 && prefix.ends_with('.')),
        None => pattern == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            url_host("https://user@API.example.com:8443/x?y").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(url_host("ftp://x.com"), None);
        assert!(host_matches("api.example.com", "api.example.com"));
        assert!(host_matches("*.example.com", "a.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "badexample.com"));
    }
}
