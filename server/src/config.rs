//! Server configuration.
//!
//! Loaded from an optional TOML file (`--config` / `PHOTOVAULT_CONFIG`), then
//! overridden by environment variables (see [`Config::apply_env`]).

use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Address the HTTP API listens on, e.g. `0.0.0.0:8080`.
    pub listen: String,
    /// Public base URL of the API as seen by clients, e.g. `https://photos.example.com`.
    pub public_url: String,
    /// Database URL: `postgres://…` or `sqlite:///path/db.sqlite?mode=rwc`.
    pub database_url: String,
    /// Object storage configuration.
    pub storage: StorageConfig,
    /// Secrets used to sign tokens and URLs.
    pub secrets: Secrets,
    /// Outgoing mail. When unset, OTP codes are printed to the log.
    pub smtp: Option<SmtpConfig>,
    /// Default storage quota (bytes) for new users. Self-hosted default: 100 TB like museum.
    pub default_storage_bytes: i64,
    /// User IDs allowed to call `/admin/*` endpoints.
    pub admins: Vec<i64>,
    /// When false, only the admin CLI can create accounts (OTT is only sent to existing emails).
    pub allow_signups: bool,
    /// Hard-coded OTTs per email for testing/self-hosting (`email = "123456"`), like museum's `internal.hardcoded-ott`.
    pub hardcoded_ott: std::collections::BTreeMap<String, String>,
    /// Log output format: `json` (default) or `pretty`.
    pub log_format: String,
    /// Companion web-app URLs and WebAuthn relying party (`[accounts]`).
    pub accounts: crate::users::settings::AccountsSettings,
    /// Checksums, scrubbing, 3-2-1 replication, alerts and FSView (`[durability]`).
    pub durability: crate::durability::config::DurabilityConfig,
}

/// Which object store backs the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StorageConfig {
    /// Local directory (may be a mergerfs pool). The server signs its own URLs.
    Local {
        /// Root directory for objects.
        path: PathBuf,
        /// Public base URL for object up/downloads; defaults to `public_url`.
        public_url: Option<String>,
    },
    /// Any S3-compatible service.
    S3(S3Config),
}

/// `[storage] kind = "s3"` settings.
#[derive(Clone, Deserialize)]
pub struct S3Config {
    /// Endpoint the server uses, e.g. `https://s3.eu-central-003.backblazeb2.com`
    /// or `http://garage:3900` on a Docker network.
    pub endpoint: String,
    /// Endpoint put into presigned URLs handed to the apps, when it differs
    /// from `endpoint` (e.g. internal `http://garage:3900` vs public
    /// `https://s3.example.com`). Defaults to `endpoint`.
    #[serde(default)]
    pub public_endpoint: Option<String>,
    /// Region, e.g. `eu-central-003` (`us-east-1` for MinIO, `garage` for Garage).
    #[serde(default = "default_region")]
    pub region: String,
    /// Bucket name.
    pub bucket: String,
    /// Access key id.
    pub access_key: String,
    /// Secret access key.
    #[serde(default)]
    pub secret_key: String,
    /// Read the secret access key from this file instead (Docker secrets).
    #[serde(default)]
    pub secret_key_file: Option<PathBuf>,
    /// Use path-style URLs (`endpoint/bucket/key`, default) instead of
    /// virtual-host style (`bucket.endpoint/key`).
    #[serde(default = "default_true")]
    pub path_style: bool,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("public_endpoint", &self.public_endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .field("path_style", &self.path_style)
            .finish()
    }
}

fn default_true() -> bool {
    true
}

fn default_region() -> String {
    "us-east-1".into()
}

/// Secrets. All are base64 strings; generated and printed on first start if empty.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Secrets {
    /// HMAC key for JWTs (payment/family/accounts/public-link tokens) and signed storage URLs.
    pub jwt: String,
    /// Key used for server-side encryption of user emails at rest (libsodium secretbox, 32 bytes).
    pub encryption: String,
    /// Key used to hash emails for lookup (libsodium generichash key, 32+ bytes).
    pub hashing: String,
}

/// SMTP relay settings.
#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    /// Hostname.
    pub host: String,
    /// Port (465 implicit TLS, 587 STARTTLS).
    pub port: u16,
    /// Username.
    pub username: Option<String>,
    /// Password.
    pub password: Option<String>,
    /// From address.
    pub from: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            public_url: "http://localhost:8080".into(),
            database_url: "sqlite://photovault.sqlite?mode=rwc".into(),
            storage: StorageConfig::Local {
                path: "data".into(),
                public_url: None,
            },
            secrets: Secrets::default(),
            smtp: None,
            default_storage_bytes: 100 * 1024 * 1024 * 1024 * 1024,
            admins: vec![],
            allow_signups: true,
            hardcoded_ott: Default::default(),
            log_format: "json".into(),
            accounts: Default::default(),
            durability: Default::default(),
        }
    }
}

impl Config {
    /// Load from an optional file then apply environment overrides.
    pub fn load(path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let mut cfg: Config = match path {
            Some(p) => toml::from_str(&std::fs::read_to_string(p)?)?,
            None => Config::default(),
        };
        cfg.apply_env();
        Ok(cfg)
    }

    /// Apply `PHOTOVAULT_*` environment overrides for the common keys.
    pub fn apply_env(&mut self) {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = env("PHOTOVAULT_LISTEN") {
            self.listen = v;
        }
        if let Some(v) = env("PHOTOVAULT_PUBLIC_URL") {
            self.public_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_DATABASE_URL") {
            self.database_url = v;
        }
        self.apply_storage_env(&env);
        if let Some(v) = env("PHOTOVAULT_JWT_SECRET") {
            self.secrets.jwt = v;
        }
        if let Some(v) = env("PHOTOVAULT_ENCRYPTION_KEY") {
            self.secrets.encryption = v;
        }
        if let Some(v) = env("PHOTOVAULT_HASHING_KEY") {
            self.secrets.hashing = v;
        }
        if let Some(v) = env("PHOTOVAULT_LOG_FORMAT") {
            self.log_format = v;
        }
        if let Some(v) = env("PHOTOVAULT_ALLOW_SIGNUPS") {
            self.allow_signups = v == "true" || v == "1";
        }
        self.accounts.apply_env();
        if let Some(v) = env("PHOTOVAULT_ADMINS") {
            self.admins = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
    }

    /// Storage overrides. `PHOTOVAULT_STORAGE_KIND` (`local`/`s3`) switches
    /// backend; otherwise the individual variables patch the configured one.
    /// Local: `PHOTOVAULT_STORAGE_PATH`, `PHOTOVAULT_STORAGE_PUBLIC_URL`.
    /// S3: `PHOTOVAULT_S3_ENDPOINT`, `PHOTOVAULT_S3_PUBLIC_ENDPOINT`,
    /// `PHOTOVAULT_S3_REGION`, `PHOTOVAULT_S3_BUCKET`, `PHOTOVAULT_S3_ACCESS_KEY`,
    /// `PHOTOVAULT_S3_SECRET_KEY`, `PHOTOVAULT_S3_SECRET_KEY_FILE`,
    /// `PHOTOVAULT_S3_PATH_STYLE`.
    fn apply_storage_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        let kind = env("PHOTOVAULT_STORAGE_KIND").map(|k| k.to_ascii_lowercase());
        let want_s3 = match kind.as_deref() {
            Some("s3") => true,
            Some(_) => false,
            None => matches!(self.storage, StorageConfig::S3(_)),
        };
        if want_s3 {
            let mut c = match &self.storage {
                StorageConfig::S3(c) => c.clone(),
                StorageConfig::Local { .. } => S3Config {
                    endpoint: String::new(),
                    public_endpoint: None,
                    region: default_region(),
                    bucket: String::new(),
                    access_key: String::new(),
                    secret_key: String::new(),
                    secret_key_file: None,
                    path_style: true,
                },
            };
            if let Some(v) = env("PHOTOVAULT_S3_ENDPOINT") {
                c.endpoint = v;
            }
            if let Some(v) = env("PHOTOVAULT_S3_PUBLIC_ENDPOINT") {
                c.public_endpoint = Some(v);
            }
            if let Some(v) = env("PHOTOVAULT_S3_REGION") {
                c.region = v;
            }
            if let Some(v) = env("PHOTOVAULT_S3_BUCKET") {
                c.bucket = v;
            }
            if let Some(v) = env("PHOTOVAULT_S3_ACCESS_KEY") {
                c.access_key = v;
            }
            if let Some(v) = env("PHOTOVAULT_S3_SECRET_KEY") {
                c.secret_key = v;
            }
            if let Some(v) = env("PHOTOVAULT_S3_SECRET_KEY_FILE") {
                c.secret_key_file = Some(v.into());
            }
            if let Some(v) = env("PHOTOVAULT_S3_PATH_STYLE") {
                c.path_style = !(v == "false" || v == "0");
            }
            self.storage = StorageConfig::S3(c);
        } else {
            let (mut path, mut public_url) = match &self.storage {
                StorageConfig::Local { path, public_url } => (path.clone(), public_url.clone()),
                StorageConfig::S3(_) => (PathBuf::from("data"), None),
            };
            if let Some(v) = env("PHOTOVAULT_STORAGE_PATH") {
                path = v.into();
            }
            if let Some(v) = env("PHOTOVAULT_STORAGE_PUBLIC_URL") {
                public_url = Some(v);
            }
            self.storage = StorageConfig::Local { path, public_url };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_toml_and_env() {
        let cfg: Config = toml::from_str(
            r#"
            [storage]
            kind = "s3"
            endpoint = "http://garage:3900"
            public_endpoint = "https://s3.example.com"
            region = "garage"
            bucket = "photos"
            access_key = "GK1"
            secret_key = "s3cr3t"
            "#,
        )
        .expect("toml");
        let StorageConfig::S3(c) = &cfg.storage else {
            panic!("expected s3")
        };
        assert!(c.path_style);
        assert_eq!(c.public_endpoint.as_deref(), Some("https://s3.example.com"));
        assert!(!format!("{cfg:?}").contains("s3cr3t"));

        let mut cfg = Config::default();
        let vars: std::collections::HashMap<&str, &str> = [
            ("PHOTOVAULT_STORAGE_KIND", "s3"),
            (
                "PHOTOVAULT_S3_ENDPOINT",
                "https://s3.eu-central-003.backblazeb2.com",
            ),
            ("PHOTOVAULT_S3_BUCKET", "b"),
            ("PHOTOVAULT_S3_PATH_STYLE", "false"),
        ]
        .into_iter()
        .collect();
        cfg.apply_storage_env(&|k| vars.get(k).map(|v| v.to_string()));
        let StorageConfig::S3(c) = &cfg.storage else {
            panic!("expected s3")
        };
        assert_eq!((c.bucket.as_str(), c.path_style), ("b", false));
        assert_eq!(c.region, "us-east-1");

        let mut cfg = Config::default();
        cfg.apply_storage_env(&|k| (k == "PHOTOVAULT_STORAGE_PATH").then(|| "/data".into()));
        assert!(
            matches!(&cfg.storage, StorageConfig::Local { path, .. } if path == std::path::Path::new("/data"))
        );
    }
}
