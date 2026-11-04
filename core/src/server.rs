//! Servers: `/photovault/info` (contracts/server-info.md), the stock-server
//! fallback to `/ping`, home/public address switching, the "Connect" QR code
//! and LAN discovery.

use crate::api::{display_address, probe_client};
use crate::model::PhotovaultInfo;
use crate::{CoreError, Result, ServerInfo};
use std::time::Duration;

/// How long the home URL gets to answer before the public URL is used.
pub const HOME_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Timeout for looking up a server the user typed.
const INFO_TIMEOUT: Duration = Duration::from_secs(8);
/// mDNS / DNS-SD service type.
const SERVICE_TYPE: &str = "_photovault._tcp.local.";

/// Normalise a user-typed address: trim, drop a trailing slash. Without a
/// scheme the candidates are `https://` then `http://`.
pub fn normalize_address(address: &str) -> Vec<String> {
    let a = address.trim().trim_end_matches('/');
    if a.is_empty() {
        return Vec::new();
    }
    let lower = a.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        vec![a.to_string()]
    } else {
        vec![format!("https://{a}"), format!("http://{a}")]
    }
}

fn trim_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

/// What answers at one base URL.
enum Probe {
    Photovault(PhotovaultInfo),
    Stock,
    Other,
    Unreachable,
}

async fn probe(client: &reqwest::Client, base: &str) -> Probe {
    match client.get(format!("{base}/photovault/info")).send().await {
        Ok(res) if res.status().is_success() => {
            if let Ok(info) = res.json::<PhotovaultInfo>().await
                && info.product == "photovault"
            {
                return Probe::Photovault(info);
            }
        }
        Ok(_) => {}
        Err(e) if e.is_connect() || e.is_timeout() => return Probe::Unreachable,
        Err(_) => return Probe::Other,
    }
    match client.get(format!("{base}/ping")).send().await {
        Ok(res) if res.status().is_success() => match res.json::<serde_json::Value>().await {
            Ok(v) if v.get("message").and_then(|m| m.as_str()) == Some("pong") => Probe::Stock,
            _ => Probe::Other,
        },
        Ok(_) => Probe::Other,
        Err(e) if e.is_connect() || e.is_timeout() => Probe::Unreachable,
        Err(_) => Probe::Other,
    }
}

fn info_from(p: PhotovaultInfo, base: &str) -> ServerInfo {
    let public_url = if p.public_url.trim().is_empty() {
        base.to_string()
    } else {
        trim_url(&p.public_url)
    };
    ServerInfo {
        name: p.name,
        version: p.version,
        public_url,
        home_url: p.home_url.map(|h| trim_url(&h)).filter(|h| !h.is_empty()),
        signups_allowed: p.signups_allowed,
        is_photovault: true,
    }
}

/// Describe the server at `address`.
pub async fn fetch_server_info(address: &str) -> Result<ServerInfo> {
    let candidates = normalize_address(address);
    if candidates.is_empty() {
        return Err(CoreError::Unreachable {
            address: address.to_string(),
        });
    }
    let client = probe_client(INFO_TIMEOUT);
    let mut answered = false;
    for base in &candidates {
        match probe(&client, base).await {
            Probe::Photovault(p) => return Ok(info_from(p, base)),
            Probe::Stock => {
                return Ok(ServerInfo {
                    name: String::new(),
                    version: String::new(),
                    public_url: base.clone(),
                    home_url: None,
                    signups_allowed: true,
                    is_photovault: false,
                });
            }
            Probe::Other => answered = true,
            Probe::Unreachable => {}
        }
    }
    let shown = display_address(&candidates[0]);
    Err(if answered {
        CoreError::NotPhotovault { address: shown }
    } else {
        CoreError::Unreachable { address: shown }
    })
}

/// The base URL to use for `info`: the home URL if it answers
/// `/photovault/info` within [`HOME_PROBE_TIMEOUT`] with the same public URL,
/// otherwise the public URL.
///
/// The home URL carries the session token and the SRP login, and anyone on
/// a network that reuses the same private address can answer
/// `/photovault/info` with the (public) `publicUrl`. So a home URL is only
/// considered when its identity is checked (`https://`, verified against the
/// web PKI like the public URL) or when the user explicitly allowed this
/// server's plain-http home address (`allow_http`).
pub async fn choose_base(info: &ServerInfo, allow_http: bool) -> String {
    if let Some(home) = info
        .home_url
        .as_deref()
        .filter(|h| !h.is_empty())
        .filter(|h| allow_http || h.to_ascii_lowercase().starts_with("https://"))
    {
        let client = probe_client(HOME_PROBE_TIMEOUT);
        let fut = client
            .get(format!("{}/photovault/info", trim_url(home)))
            .send();
        if let Ok(Ok(res)) = tokio::time::timeout(HOME_PROBE_TIMEOUT, fut).await
            && res.status().is_success()
            && let Ok(Ok(p)) =
                tokio::time::timeout(HOME_PROBE_TIMEOUT, res.json::<PhotovaultInfo>()).await
            && p.product == "photovault"
            && trim_url(&p.public_url) == trim_url(&info.public_url)
        {
            return trim_url(home);
        }
    }
    trim_url(&info.public_url)
}

/// The server address in a Connect QR payload.
pub fn parse_connect_qr_url(payload: &str) -> Option<String> {
    let p = payload.trim();
    let u = url::Url::parse(p).ok()?;
    match u.scheme() {
        "photovault" => {
            if u.host_str() != Some("connect") && u.path().trim_matches('/') != "connect" {
                return None;
            }
            let url = u
                .query_pairs()
                .find(|(k, _)| k == "url")
                .map(|(_, v)| v.into_owned())?;
            let parsed = url::Url::parse(url.trim()).ok()?;
            matches!(parsed.scheme(), "http" | "https").then(|| trim_url(&url))
        }
        "http" | "https" if u.host_str().is_some() => Some(trim_url(p)),
        _ => None,
    }
}

/// Browse `_photovault._tcp` for `timeout_ms`; returns LAN addresses.
pub fn discover(timeout_ms: u32) -> Vec<String> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    let Ok(daemon) = ServiceDaemon::new() else {
        return Vec::new();
    };
    let Ok(rx) = daemon.browse(SERVICE_TYPE) else {
        let _ = daemon.shutdown();
        return Vec::new();
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut found: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let txt = |k: &str| {
                    info.get_property_val_str(k)
                        .map(trim_url)
                        .filter(|s| !s.is_empty())
                };
                let addr = txt("home").or_else(|| {
                    info.get_addresses_v4()
                        .into_iter()
                        .next()
                        .map(|ip| format!("http://{ip}:{}", info.get_port()))
                });
                if let Some(a) = addr.or_else(|| txt("public"))
                    && !found.contains(&a)
                {
                    found.push(a);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalised() {
        assert_eq!(
            normalize_address(" https://a.example/ "),
            vec!["https://a.example"]
        );
        assert_eq!(
            normalize_address("10.0.1.7:8095"),
            vec!["https://10.0.1.7:8095", "http://10.0.1.7:8095"]
        );
        assert!(normalize_address("  ").is_empty());
    }

    #[test]
    fn qr_payloads() {
        assert_eq!(
            parse_connect_qr_url(
                "photovault://connect?url=https%3A%2F%2Fphotos.example.com%2F&home=http%3A%2F%2F10.0.1.7%3A8095&name=Home"
            )
            .as_deref(),
            Some("https://photos.example.com")
        );
        assert_eq!(
            parse_connect_qr_url("https://photos.example.com/").as_deref(),
            Some("https://photos.example.com")
        );
        assert_eq!(parse_connect_qr_url("photovault://connect?home=x"), None);
        assert_eq!(
            parse_connect_qr_url("photovault://other?url=https%3A%2F%2Fa"),
            None
        );
        assert_eq!(
            parse_connect_qr_url("photovault://connect?url=javascript%3Aalert(1)"),
            None
        );
        assert_eq!(parse_connect_qr_url("hello"), None);
        assert_eq!(parse_connect_qr_url("ftp://a.example"), None);
    }
}
