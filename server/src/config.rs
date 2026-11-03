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
    S3 {
        /// Endpoint, e.g. `https://s3.eu-central-003.backblazeb2.com`.
        endpoint: String,
        /// Region, e.g. `eu-central-003`.
        region: String,
        /// Bucket name.
        bucket: String,
        /// Access key id.
        access_key: String,
        /// Secret access key.
        secret_key: String,
        /// Use path-style URLs (`endpoint/bucket/key`) instead of virtual-host style.
        #[serde(default = "default_true")]
        path_style: bool,
    },
}

fn default_true() -> bool {
    true
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
        if let Some(v) = env("PHOTOVAULT_STORAGE_PATH") {
            self.storage = StorageConfig::Local {
                path: v.into(),
                public_url: env("PHOTOVAULT_STORAGE_PUBLIC_URL"),
            };
        }
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
}
