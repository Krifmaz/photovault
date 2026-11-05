//! Servers: `/photovault/info` (contracts/server-info.md), the stock-server
//! fallback to `/ping`, home/public address switching, the "Connect" QR code
//! and LAN discovery.

use crate::api::{display_address, probe_client};
use crate::model::PhotovaultInfo;
use crate::{CoreError, Result, ServerInfo};
use std::time::Duration;

/// How long the home URL gets to answer before the public URL is used.
pub const HOME_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long the public URL gets to answer when only checking that it does
/// (to remember when the server was last reached); it is used either way.
pub const PUBLIC_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
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

pub(crate) fn trim_url(u: &str) -> String {
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
    fetch_server_info_at(address).await.map(|(info, _)| info)
}

/// [`fetch_server_info`], and the base URL that answered.
pub async fn fetch_server_info_at(address: &str) -> Result<(ServerInfo, String)> {
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
            Probe::Photovault(p) => return Ok((info_from(p, base), base.clone())),
            Probe::Stock => {
                return Ok((
                    ServerInfo {
                        name: String::new(),
                        version: String::new(),
                        public_url: base.clone(),
                        home_url: None,
                        signups_allowed: true,
                        is_photovault: false,
                    },
                    base.clone(),
                ));
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

/// What [`choose_address`] picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    /// The base URL to use.
    pub url: String,
    /// Whether `url` answered (as this server) while choosing.
    pub answered: bool,
}

/// The addresses [`choose_address`] probes before falling back to the public
/// URL, in order: the last address that answered (contract v1.3, e.g. a
/// Tailscale address; only when it is neither the home nor the public URL),
/// then the home URL.
///
/// These carry the session token and the SRP login, and anyone on a network
/// that reuses the same private address can answer `/photovault/info` with
/// the (public) `publicUrl` (a carrier's CGNAT range overlaps Tailscale's).
/// So one is only considered when its identity is checked (`https://`,
/// verified against the web PKI like the public URL) or when the user
/// explicitly allowed this server's plain-http addresses (`allow_http`).
pub fn direct_candidates(info: &ServerInfo, allow_http: bool, last: Option<&str>) -> Vec<String> {
    let public = trim_url(&info.public_url);
    let usable =
        |u: &str| !u.is_empty() && (allow_http || u.to_ascii_lowercase().starts_with("https://"));
    let home = info.home_url.as_deref().map(trim_url);
    let mut out = Vec::new();
    if let Some(l) = last.map(trim_url)
        && l != public
        && Some(&l) != home.as_ref()
        && usable(&l)
    {
        out.push(l);
    }
    if let Some(h) = home
        && h != public
        && usable(&h)
    {
        out.push(h);
    }
    out
}

/// Whether `base` answers `/photovault/info` within [`HOME_PROBE_TIMEOUT`]
/// as the server whose public URL is `public`.
async fn answers_as(client: &reqwest::Client, base: &str, public: &str) -> bool {
    let fut = client.get(format!("{base}/photovault/info")).send();
    if let Ok(Ok(res)) = tokio::time::timeout(HOME_PROBE_TIMEOUT, fut).await
        && res.status().is_success()
        && let Ok(Ok(p)) =
            tokio::time::timeout(HOME_PROBE_TIMEOUT, res.json::<PhotovaultInfo>()).await
    {
        return p.product == "photovault" && trim_url(&p.public_url) == trim_url(public);
    }
    false
}

/// The base URL for `info`: the first of [`direct_candidates`] (the last
/// address that answered, then the home URL) that answers `/photovault/info`
/// within [`HOME_PROBE_TIMEOUT`] with the same public URL, otherwise the
/// public URL. With `check_public`, a public URL chosen this way is probed
/// too (at most [`PUBLIC_PROBE_TIMEOUT`]) so the caller knows whether it
/// answered; the choice is the same either way.
pub async fn choose_address(
    info: &ServerInfo,
    allow_http: bool,
    last: Option<&str>,
    check_public: bool,
) -> Chosen {
    let public = trim_url(&info.public_url);
    let candidates = direct_candidates(info, allow_http, last);
    if !candidates.is_empty() {
        let client = probe_client(HOME_PROBE_TIMEOUT);
        for c in candidates {
            if answers_as(&client, &c, &public).await {
                return Chosen {
                    url: c,
                    answered: true,
                };
            }
        }
    }
    let answered = if check_public {
        let client = probe_client(PUBLIC_PROBE_TIMEOUT);
        match tokio::time::timeout(PUBLIC_PROBE_TIMEOUT, probe(&client, &public)).await {
            Ok(Probe::Photovault(p)) => info_from(p, &public).public_url == public,
            Ok(Probe::Stock) => !info.is_photovault,
            _ => false,
        }
    } else {
        false
    };
    Chosen {
        url: public,
        answered,
    }
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
            normalize_address("192.168.1.10:8080"),
            vec!["https://192.168.1.10:8080", "http://192.168.1.10:8080"]
        );
        assert!(normalize_address("  ").is_empty());
    }

    #[test]
    fn qr_payloads() {
        assert_eq!(
            parse_connect_qr_url(
                "photovault://connect?url=https%3A%2F%2Fphotos.example.com%2F&home=http%3A%2F%2F192.168.1.10%3A8080&name=Home"
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

    fn info(home: Option<&str>) -> ServerInfo {
        ServerInfo {
            name: "Home vault".into(),
            version: "0.2.0".into(),
            public_url: "https://photos.example.com".into(),
            home_url: home.map(str::to_string),
            signups_allowed: false,
            is_photovault: true,
        }
    }

    #[test]
    fn last_address_is_tried_before_home() {
        let i = info(Some("http://192.168.1.10:8080"));
        assert_eq!(
            direct_candidates(&i, true, Some("http://100.101.102.103:8080/")),
            vec!["http://100.101.102.103:8080", "http://192.168.1.10:8080"]
        );
        // The last address is the home or the public URL: probed once, or not at all.
        assert_eq!(
            direct_candidates(&i, true, Some("http://192.168.1.10:8080")),
            vec!["http://192.168.1.10:8080"]
        );
        assert_eq!(
            direct_candidates(&i, true, Some("https://photos.example.com")),
            vec!["http://192.168.1.10:8080"]
        );
        assert!(direct_candidates(&info(None), true, None).is_empty());
    }

    #[test]
    fn plain_http_direct_addresses_need_approval() {
        let i = info(Some("http://192.168.1.10:8080"));
        assert!(direct_candidates(&i, false, Some("http://100.101.102.103:8080")).is_empty());
        // https is checked against the web PKI: no approval needed.
        assert_eq!(
            direct_candidates(&i, false, Some("https://vault.tail1234.ts.net")),
            vec!["https://vault.tail1234.ts.net"]
        );
    }

    #[tokio::test]
    async fn unreachable_direct_addresses_fall_back_to_public() {
        // Nothing listens on these ports: the public URL is chosen without waiting for long.
        let i = ServerInfo {
            public_url: "http://127.0.0.1:9".into(),
            ..info(Some("http://127.0.0.1:7"))
        };
        let started = std::time::Instant::now();
        let c = choose_address(&i, true, Some("http://127.0.0.1:5"), true).await;
        assert_eq!(c.url, "http://127.0.0.1:9");
        assert!(!c.answered);
        assert!(started.elapsed() < Duration::from_secs(8));
    }
}
