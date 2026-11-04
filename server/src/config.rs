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
    /// Database URL: `postgres://…` or `sqlite:///config/photovault.db?mode=rwc`.
    /// Empty (default) = SQLite in the config folder (`<config_dir>/photovault.sqlite`).
    pub database_url: String,
    /// Folder for `keys.toml` and the default SQLite database (`PHOTOVAULT_CONFIG_DIR`).
    pub config_dir: PathBuf,
    /// Built web app to serve on the API port (`PHOTOVAULT_WEB_DIR`). Unset or missing = API only.
    pub web_dir: Option<PathBuf>,
    /// Object storage configuration.
    pub storage: StorageConfig,
    /// Secrets used to sign tokens and URLs.
    pub secrets: Secrets,
    /// Outgoing mail. When unset, OTP codes are printed to the log.
    pub smtp: Option<SmtpConfig>,
    /// Accept any code at the email verification step and send no email
    /// (`PHOTOVAULT_SKIP_EMAIL_VERIFICATION`). Unset = skip exactly when no SMTP
    /// server is configured (there is no way to deliver a code then); see
    /// [`Config::skips_email_verification`].
    pub skip_email_verification: Option<bool>,
    /// Default storage quota (bytes) for new users (`PHOTOVAULT_DEFAULT_QUOTA`).
    /// `0` (default) = no quota: the storage backend's free space is the limit.
    pub default_storage_bytes: i64,
    /// Total bytes the server may store (`PHOTOVAULT_STORAGE_LIMIT`). Needed for
    /// real numbers on S3; also caps a local disk. Unset = disk free space (local)
    /// or unlimited (S3).
    pub storage_limit: Option<i64>,
    /// Server name shown in the apps and announced on the LAN (`PHOTOVAULT_SERVER_NAME`).
    pub server_name: String,
    /// Address of the server on the home network, e.g. `http://10.0.1.7:8095`
    /// (`PHOTOVAULT_HOME_URL`). Optional.
    pub home_url: Option<String>,
    /// Web app origins allowed by CORS (`PHOTOVAULT_WEB_ORIGINS`, comma-separated).
    /// Empty = any origin (museum's behaviour).
    pub web_origins: Vec<String>,
    /// Announce the server on the LAN with mDNS/DNS-SD (`PHOTOVAULT_MDNS`, default true).
    pub mdns: bool,
    /// Serve the ML models and memory music the apps would otherwise fetch from
    /// Ente's CDN, caching each file in `<config_dir>/assets` on first use
    /// (`PHOTOVAULT_ASSET_MIRROR`, default true). See `crate::asset_mirror`.
    pub asset_mirror: bool,
    /// Where the mirror fetches models (`PHOTOVAULT_MODELS_UPSTREAM`).
    pub models_upstream: String,
    /// Where the mirror fetches the memory music (`PHOTOVAULT_MUSIC_UPSTREAM`).
    pub music_upstream: String,
    /// Tests only: report this many free bytes instead of asking the storage backend.
    #[serde(skip)]
    pub free_space_override: Option<i64>,
    /// User IDs allowed to call `/admin/*` endpoints.
    pub admins: Vec<i64>,
    /// When false, only the admin CLI can create accounts (OTT is only sent to existing emails).
    pub allow_signups: bool,
    /// Hard-coded OTTs per email for testing/self-hosting (`email = "123456"`), like museum's `internal.hardcoded-ott`.
    pub hardcoded_ott: std::collections::BTreeMap<String, String>,
    /// Log output format: `pretty` (default) or `json`.
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
    /// From address, e.g. `Photovault <photos@example.com>`.
    pub from: String,
    /// `starttls`, `tls` (implicit, port 465) or `none`. Default: `tls` on 465, else `starttls`.
    #[serde(default)]
    pub tls: Option<SmtpTls>,
}

/// SMTP transport security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpTls {
    /// Plain connection upgraded with STARTTLS (required).
    Starttls,
    /// TLS from the first byte (port 465).
    Tls,
    /// No encryption (only for a relay on the same host or Docker network).
    None,
}

impl SmtpTls {
    /// Parse `starttls` / `tls` / `ssl` / `none`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "starttls" => Some(Self::Starttls),
            "tls" | "ssl" | "implicit" => Some(Self::Tls),
            "none" | "plain" | "off" | "false" => Some(Self::None),
            _ => None,
        }
    }
}

impl SmtpConfig {
    /// The configured TLS mode, or the port's usual one.
    pub fn tls_mode(&self) -> SmtpTls {
        self.tls.unwrap_or(if self.port == 465 {
            SmtpTls::Tls
        } else {
            SmtpTls::Starttls
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            public_url: "http://localhost:8080".into(),
            database_url: String::new(),
            config_dir: "/config".into(),
            web_dir: None,
            storage: StorageConfig::Local {
                path: "data".into(),
                public_url: None,
            },
            secrets: Secrets::default(),
            smtp: None,
            skip_email_verification: None,
            default_storage_bytes: 0,
            storage_limit: None,
            server_name: "Photovault".into(),
            home_url: None,
            web_origins: vec![],
            mdns: true,
            asset_mirror: true,
            models_upstream: "https://models.ente.com".into(),
            music_upstream: "https://music.ente.com".into(),
            free_space_override: None,
            admins: vec![],
            allow_signups: false,
            hardcoded_ott: Default::default(),
            log_format: "pretty".into(),
            accounts: Default::default(),
        }
    }
}

impl Config {
    /// Whether sign-up / log-in accept any email code: `PHOTOVAULT_SKIP_EMAIL_VERIFICATION`
    /// when set, otherwise on exactly when no SMTP server is configured. Codes for
    /// accounts with email two-factor are always checked (`users::login::skip_applies`).
    pub fn skips_email_verification(&self) -> bool {
        self.skip_email_verification.unwrap_or(self.smtp.is_none())
    }

    /// Load from an optional file then apply environment overrides.
    pub fn load(path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let mut cfg: Config = match path {
            Some(p) => toml::from_str(&std::fs::read_to_string(p)?)?,
            None => Config::default(),
        };
        cfg.apply_env()?;
        if cfg.database_url.trim().is_empty() {
            cfg.database_url = cfg.default_database_url();
        }
        cfg.derive_companion_urls();
        Ok(cfg)
    }

    /// One origin: companion web apps that were left at museum's localhost defaults live
    /// under the public URL (`/accounts`, `/family`, `/cast`), and passkeys use its host.
    pub fn derive_companion_urls(&mut self) {
        let public = self.public_url.trim_end_matches('/').to_string();
        let a = &mut self.accounts;
        let d = crate::users::settings::AccountsSettings::default();
        if a.accounts_url == d.accounts_url {
            a.accounts_url = format!("{public}/accounts");
        }
        if a.family_url == d.family_url {
            a.family_url = format!("{public}/family");
        }
        if a.cast_url.is_empty() {
            a.cast_url = format!("{public}/cast");
        }
        // Share links: `<public>/albums/?t=<token>#<key>` (the albums export in the web dir).
        if a.public_albums_url.is_empty() {
            a.public_albums_url = format!("{public}/albums");
        }
        if let Ok(u) = url::Url::parse(&public) {
            if a.webauthn_rp_id == d.webauthn_rp_id
                && let Some(h) = u.host_str()
            {
                a.webauthn_rp_id = h.to_string();
            }
            if a.webauthn_rp_origins == d.webauthn_rp_origins {
                a.webauthn_rp_origins = vec![u.origin().ascii_serialization()];
            }
        }
    }

    /// SQLite in the config folder.
    pub fn default_database_url(&self) -> String {
        format!(
            "sqlite://{}/photovault.db?mode=rwc",
            self.config_dir.display()
        )
    }

    /// Apply `PHOTOVAULT_*` environment overrides for the common keys.
    pub fn apply_env(&mut self) -> anyhow::Result<()> {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        self.apply_env_from(&env)
    }

    /// [`Config::apply_env`] reading variables through `env` (tests pass a map).
    pub fn apply_env_from(&mut self, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<()> {
        let flag = |v: &str| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        };
        if let Some(v) = env("PHOTOVAULT_LISTEN") {
            self.listen = v;
        }
        if let Some(v) = env("PHOTOVAULT_PUBLIC_URL") {
            self.public_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_DATABASE_URL") {
            self.database_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_CONFIG_DIR") {
            self.config_dir = v.into();
        }
        if let Some(v) = env("PHOTOVAULT_WEB_DIR") {
            self.web_dir = Some(v.into());
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
            self.allow_signups = flag(&v);
        }
        self.accounts.apply_env();
        if let Some(v) = env("PHOTOVAULT_ADMINS") {
            self.admins = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        if let Some(v) = env("PHOTOVAULT_SKIP_EMAIL_VERIFICATION") {
            let v = v.trim();
            self.skip_email_verification = if v.is_empty() || v.eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(flag(v))
            };
        }
        if let Some(v) = env("PHOTOVAULT_DEFAULT_QUOTA") {
            self.default_storage_bytes =
                parse_quota(&v).map_err(|e| anyhow::anyhow!("PHOTOVAULT_DEFAULT_QUOTA: {e}"))?;
        }
        if let Some(v) = env("PHOTOVAULT_STORAGE_LIMIT") {
            let n =
                parse_quota(&v).map_err(|e| anyhow::anyhow!("PHOTOVAULT_STORAGE_LIMIT: {e}"))?;
            self.storage_limit = (n > 0).then_some(n);
        }
        if let Some(v) = env("PHOTOVAULT_SERVER_NAME") {
            self.server_name = v;
        }
        if let Some(v) = env("PHOTOVAULT_HOME_URL") {
            self.home_url = Some(v.trim_end_matches('/').to_string());
        }
        if let Some(v) = env("PHOTOVAULT_WEB_ORIGINS") {
            self.web_origins = v
                .split(',')
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(v) = env("PHOTOVAULT_MDNS") {
            self.mdns = flag(&v);
        }
        if let Some(v) = env("PHOTOVAULT_ASSET_MIRROR") {
            self.asset_mirror = flag(&v);
        }
        if let Some(v) = env("PHOTOVAULT_MODELS_UPSTREAM") {
            self.models_upstream = v.trim().trim_end_matches('/').to_string();
        }
        if let Some(v) = env("PHOTOVAULT_MUSIC_UPSTREAM") {
            self.music_upstream = v.trim().trim_end_matches('/').to_string();
        }
        self.apply_smtp_env(env)?;
        Ok(())
    }

    /// `PHOTOVAULT_SMTP_HOST`, `_PORT`, `_USERNAME`, `_PASSWORD`, `_PASSWORD_FILE`,
    /// `_FROM`, `_TLS`. Setting the host enables SMTP.
    fn apply_smtp_env(&mut self, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<()> {
        let host = env("PHOTOVAULT_SMTP_HOST");
        if host.is_none() && self.smtp.is_none() {
            return Ok(());
        }
        let mut c = self.smtp.clone().unwrap_or(SmtpConfig {
            host: String::new(),
            port: 587,
            username: None,
            password: None,
            from: String::new(),
            tls: None,
        });
        if let Some(v) = host {
            c.host = v;
        }
        if let Some(v) = env("PHOTOVAULT_SMTP_PORT") {
            c.port = v
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("PHOTOVAULT_SMTP_PORT: not a port number: {v}"))?;
        }
        if let Some(v) = env("PHOTOVAULT_SMTP_USERNAME") {
            c.username = Some(v);
        }
        if let Some(v) = env("PHOTOVAULT_SMTP_PASSWORD") {
            c.password = Some(v);
        }
        if let Some(p) = env("PHOTOVAULT_SMTP_PASSWORD_FILE") {
            let v = std::fs::read_to_string(&p)
                .map_err(|e| anyhow::anyhow!("PHOTOVAULT_SMTP_PASSWORD_FILE {p}: {e}"))?;
            c.password = Some(v.trim_end_matches(['\r', '\n']).to_string());
        }
        if let Some(v) = env("PHOTOVAULT_SMTP_FROM") {
            c.from = v;
        }
        if let Some(v) = env("PHOTOVAULT_SMTP_TLS") {
            c.tls = Some(SmtpTls::parse(&v).ok_or_else(|| {
                anyhow::anyhow!("PHOTOVAULT_SMTP_TLS: use starttls, tls or none (got {v})")
            })?);
        }
        if c.from.is_empty()
            && let Some(u) = c.username.as_deref().filter(|u| u.contains('@'))
        {
            c.from = format!("Photovault <{u}>");
        }
        self.smtp = Some(c);
        Ok(())
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

/// A size such as `500GB`, `2TB` or `123` (bytes); `none`/`unlimited`/`0` give 0 (no quota).
pub fn parse_quota(s: &str) -> anyhow::Result<i64> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "unlimited" | "off" | "0" => Ok(0),
        _ => crate::admin_cli::parse_size(s),
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
