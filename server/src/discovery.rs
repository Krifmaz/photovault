//! LAN discovery: announce the server with mDNS/DNS-SD as `_photovault._tcp`.
//!
//! TXT records:
//! * `name`: server name (`PHOTOVAULT_SERVER_NAME`)
//! * `version`: server version
//! * `home`: home address (`PHOTOVAULT_HOME_URL`), if set
//! * `public`: outside address (`PHOTOVAULT_PUBLIC_URL`)
//! * `info`: path of the info endpoint (`/photovault/info`)
//!
//! Multicast does not leave a Docker bridge network. In Docker either run the
//! server with `network_mode: host`, run an mDNS reflector, or run the
//! `photovault announce` sidecar on the host network (see `deploy/`).

use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;

/// DNS-SD service type.
pub const SERVICE_TYPE: &str = "_photovault._tcp.local.";

/// What gets announced.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Announcement {
    /// Instance name (shown in the apps' server list).
    pub name: String,
    /// Server version.
    pub version: String,
    /// Home address, if known.
    pub home_url: Option<String>,
    /// Outside address.
    pub public_url: String,
    /// TCP port in the SRV record.
    pub port: u16,
}

impl Announcement {
    /// TXT key/values.
    pub fn txt(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("name".into(), self.name.clone());
        m.insert("version".into(), self.version.clone());
        m.insert("public".into(), self.public_url.clone());
        m.insert("info".into(), "/photovault/info".into());
        if let Some(h) = &self.home_url {
            m.insert("home".into(), h.clone());
        }
        m
    }

    /// The SRV port: the home address's port when it has one, else `fallback`.
    pub fn port_from(home_url: Option<&str>, fallback: u16) -> u16 {
        home_url
            .and_then(|h| url::Url::parse(h).ok())
            .and_then(|u| u.port_or_known_default())
            .unwrap_or(fallback)
    }
}

/// A running announcement; dropping it withdraws the service.
pub struct Announcer {
    daemon: ServiceDaemon,
    fullname: String,
}

impl std::fmt::Debug for Announcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Announcer")
            .field("fullname", &self.fullname)
            .finish()
    }
}

fn host_label(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s = s.trim_matches('-').to_string();
    if s.is_empty() {
        s = "photovault".into();
    }
    s.truncate(40);
    format!("{s}-photovault.local.")
}

/// Start announcing on every interface (addresses follow interface changes).
pub fn announce(a: &Announcement) -> anyhow::Result<Announcer> {
    let daemon = ServiceDaemon::new()?;
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &a.name,
        &host_label(&a.name),
        (),
        a.port,
        a.txt(),
    )?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info)?;
    Ok(Announcer { daemon, fullname })
}

impl Announcer {
    /// Full DNS-SD name, e.g. `Photovault._photovault._tcp.local.`.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for Announcer {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// `photovault announce`: fetch `/photovault/info` from `server` and announce it, refreshing
/// every minute so a renamed server or changed address is picked up. For the
/// `photovault-mdns` sidecar on the host network.
pub async fn run_sidecar(server: &str, port: Option<u16>) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let info_url = format!("{}/photovault/info", server.trim_end_matches('/'));
    let server_port = url::Url::parse(server)?
        .port_or_known_default()
        .unwrap_or(80);
    let mut current: Option<(Announcement, Announcer)> = None;
    let mut warned = false;
    loop {
        match fetch(&client, &info_url).await {
            Ok(info) => {
                warned = false;
                let a = Announcement {
                    port: port.unwrap_or_else(|| {
                        Announcement::port_from(info.home_url.as_deref(), server_port)
                    }),
                    ..info
                };
                if current.as_ref().map(|c| &c.0) != Some(&a) {
                    drop(current.take()); // withdraw the old record first
                    let ann = announce(&a)?;
                    tracing::info!("announcing {} on the LAN (port {})", ann.fullname(), a.port);
                    current = Some((a, ann));
                }
            }
            Err(e) if !warned => {
                tracing::warn!("cannot read {info_url}: {e:#}; will retry every 60 s");
                warned = true;
            }
            Err(_) => {}
        }
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            () = crate::shutdown_signal() => return Ok(()),
        }
    }
}

async fn fetch(client: &reqwest::Client, url: &str) -> anyhow::Result<Announcement> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Info {
        name: String,
        version: String,
        home_url: Option<String>,
        public_url: String,
    }
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let i: Info = serde_json::from_slice(&body)?;
    Ok(Announcement {
        name: i.name,
        version: i.version,
        home_url: i.home_url,
        public_url: i.public_url,
        port: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Announce and find it again with a browser on the same host (needs multicast).
    #[test]
    #[ignore = "needs a network interface with multicast"]
    fn announce_and_browse() {
        let a = Announcement {
            name: "Test vault".into(),
            version: "9.9.9".into(),
            home_url: Some("http://10.0.1.7:8095".into()),
            public_url: "https://photos.example.com".into(),
            port: 8095,
        };
        let _ann = announce(&a).expect("announce");
        let browser = ServiceDaemon::new().expect("daemon");
        let rx = browser.browse(SERVICE_TYPE).expect("browse");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(mdns_sd::ServiceEvent::ServiceResolved(s)) =
                rx.recv_timeout(std::time::Duration::from_millis(500))
                && s.get_property_val_str("name") == Some("Test vault")
            {
                assert_eq!(s.get_port(), 8095);
                assert_eq!(s.get_property_val_str("home"), Some("http://10.0.1.7:8095"));
                let _ = browser.shutdown();
                return;
            }
        }
        panic!("service not found");
    }

    #[test]
    fn txt_and_port() {
        let a = Announcement {
            name: "Home photos".into(),
            version: "0.1.0".into(),
            home_url: Some("http://10.0.1.7:8095".into()),
            public_url: "https://photos.example.com".into(),
            port: 8095,
        };
        let t = a.txt();
        assert_eq!(t["name"], "Home photos");
        assert_eq!(t["home"], "http://10.0.1.7:8095");
        assert_eq!(t["public"], "https://photos.example.com");
        assert_eq!(
            Announcement::port_from(Some("http://10.0.1.7:8095"), 1),
            8095
        );
        assert_eq!(Announcement::port_from(Some("https://x.lan"), 1), 443);
        assert_eq!(Announcement::port_from(None, 8080), 8080);
        assert_eq!(host_label("Home photos!"), "home-photos-photovault.local.");
    }
}
