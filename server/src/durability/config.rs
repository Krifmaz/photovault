//! `[durability]` configuration: checksums and scrubbing, encrypted 3-2-1
//! replication, the status page, alerts and the optional FSView integration.
//!
//! Everything is off unless configured, except checksumming and scrubbing of
//! the primary store, which are on by default (they only read).

use crate::config::S3Config;
use serde::Deserialize;
use std::path::PathBuf;

/// A configured secret: deserialises from a string, never printed by `Debug`.
#[derive(Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Secret(pub String);

impl Secret {
    /// The secret value.
    pub fn expose(&self) -> &str {
        &self.0
    }
    /// The inline value, or the trimmed contents of `file` when set.
    pub fn resolve(&self, file: Option<&std::path::Path>) -> anyhow::Result<String> {
        match file {
            Some(p) => Ok(std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", p.display()))?
                .trim()
                .to_string()),
            None => Ok(self.0.clone()),
        }
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "\"\""
        } else {
            "<redacted>"
        })
    }
}

/// `[durability]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DurabilityConfig {
    /// Master switch for the background durability jobs (catalog sweep,
    /// scrub, replication, FSView polling, alerts). Write-time checksums are
    /// always recorded.
    pub enabled: bool,
    /// How often (minutes) the catalog sweep lists the primary store to find
    /// objects written without the server seeing their bytes (S3 presigned
    /// uploads) and objects that went missing.
    pub sweep_interval_minutes: u64,
    /// Scrubbing of the primary store and of local replica targets.
    pub scrub: ScrubConfig,
    /// Replication targets (the "2" and the "1" of 3-2-1).
    pub targets: Vec<TargetConfig>,
    /// Encrypted database snapshots written to every target.
    pub database_backup: DatabaseBackupConfig,
    /// Alert delivery.
    pub alerts: AlertsConfig,
    /// The status page and its JSON API.
    pub status: StatusConfig,
    /// Optional FSView integration (mergerfs pool health).
    pub fsview: Option<FsviewConfig>,
}

impl Default for DurabilityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sweep_interval_minutes: 60,
            scrub: ScrubConfig::default(),
            targets: Vec::new(),
            database_backup: DatabaseBackupConfig::default(),
            alerts: AlertsConfig::default(),
            status: StatusConfig::default(),
            fsview: None,
        }
    }
}

/// `[durability.scrub]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScrubConfig {
    /// Re-read and verify every object at least this often (days).
    pub interval_days: u64,
    /// Read rate limit for scrubbing, MiB/s (0 = unlimited).
    pub max_read_mib_per_sec: u64,
    /// Only scrub inside this daily window, e.g. `"01:00-07:00"` (see
    /// [`ScrubConfig::utc_offset`]). Empty = any time.
    pub window: String,
    /// Offset of the window's clock from UTC, e.g. `"+01:00"`.
    pub utc_offset: String,
    /// Repair a corrupt or missing primary object from a verified replica.
    pub repair: bool,
}

impl Default for ScrubConfig {
    fn default() -> Self {
        Self {
            interval_days: 30,
            max_read_mib_per_sec: 50,
            window: String::new(),
            utc_offset: "+00:00".into(),
            repair: true,
        }
    }
}

/// How replicas on a target are verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VerifyMode {
    /// Re-read every replica and compare its SHA-256 (default for local targets).
    #[default]
    Full,
    /// Check size every pass and fully re-read `verify_sample_percent` of the
    /// replicas per pass (cheap on egress-billed remotes).
    Sample,
}

/// What a target stores objects in.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TargetKind {
    /// A directory: a second disk, USB drive or NAS mount.
    Local {
        /// Directory that receives the objects.
        path: PathBuf,
    },
    /// Any S3-compatible service (Backblaze B2, Wasabi, Garage, AWS…).
    S3 {
        /// Connection settings (same keys as `[storage] kind = "s3"`).
        #[serde(flatten)]
        s3: S3Config,
        /// Key prefix inside the bucket, e.g. `"photovault/"`.
        #[serde(default)]
        prefix: String,
    },
}

/// `[[durability.targets]]`: one replication target.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetConfig {
    /// Unique short name (`usb`, `b2`); used in the database, status page and metrics.
    pub name: String,
    /// Where the copies go.
    #[serde(flatten)]
    pub kind: TargetKind,
    /// Replication runs every this many minutes (inside `window`, if set).
    #[serde(default = "default_replicate_minutes")]
    pub interval_minutes: u64,
    /// Daily window, e.g. `"01:00-07:00"`; empty = any time.
    #[serde(default)]
    pub window: String,
    /// Offset of the window's clock from UTC, e.g. `"+01:00"`.
    #[serde(default = "default_offset")]
    pub utc_offset: String,
    /// Upload bandwidth limit, KiB/s (0 = unlimited).
    #[serde(default)]
    pub max_kib_per_sec: u64,
    /// Parallel transfers.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// How replicas are verified.
    #[serde(default)]
    pub verify: Option<VerifyMode>,
    /// Percentage of replicas fully re-read per verification pass in `sample` mode.
    #[serde(default = "default_sample")]
    pub verify_sample_percent: u8,
    /// Days between verification passes.
    #[serde(default = "default_verify_days")]
    pub verify_interval_days: u64,
    /// Keep a replica this many days after the object was deleted from the
    /// primary (protects against accidental or malicious deletion). 0 deletes
    /// replicas on the next run.
    #[serde(default = "default_retention")]
    pub deleted_retention_days: u64,
    /// Alert when the newest object is not replicated after this many hours.
    #[serde(default = "default_lag_hours")]
    pub max_lag_hours: u64,
}

impl TargetConfig {
    /// Effective verify mode: `full` for local targets, `sample` for S3,
    /// unless set.
    pub fn verify_mode(&self) -> VerifyMode {
        self.verify.unwrap_or(match self.kind {
            TargetKind::Local { .. } => VerifyMode::Full,
            TargetKind::S3 { .. } => VerifyMode::Sample,
        })
    }
}

fn default_replicate_minutes() -> u64 {
    15
}
fn default_offset() -> String {
    "+00:00".into()
}
fn default_concurrency() -> usize {
    2
}
fn default_sample() -> u8 {
    2
}
fn default_verify_days() -> u64 {
    30
}
fn default_retention() -> u64 {
    30
}
fn default_lag_hours() -> u64 {
    48
}

/// `[durability.database_backup]`: encrypted snapshots of the database,
/// uploaded to every target under `_photovault/db/`.
///
/// Objects are already ciphertext; the database holds the wrapped keys and
/// encrypted metadata needed to use them, so it is backed up too, encrypted
/// with libsodium `secretstream` under a key only the operator holds.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseBackupConfig {
    /// Take snapshots (needs `key` or `key_file`).
    pub enabled: bool,
    /// Hours between snapshots.
    pub interval_hours: u64,
    /// Snapshots kept per target.
    pub keep: usize,
    /// Base64 32-byte key (`photovault admin durability gen-key`).
    pub key: Secret,
    /// Read the key from this file instead (Docker secrets).
    pub key_file: Option<PathBuf>,
}

impl Default for DatabaseBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 24,
            keep: 14,
            key: Secret::default(),
            key_file: None,
        }
    }
}

/// `[durability.alerts]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AlertsConfig {
    /// POST a JSON document to this URL for every alert (Gotify, Home
    /// Assistant, n8n, a Discord/Slack bridge…).
    pub webhook_url: Option<String>,
    /// Publish a plain-text message to this ntfy topic URL.
    pub ntfy_url: Option<String>,
    /// Email these addresses (uses `[smtp]`; logged when no mail server is set).
    pub email: Vec<String>,
    /// Lowest severity sent: `info`, `warning` or `critical`.
    pub min_severity: String,
    /// Do not repeat the same alert (kind + subject) within this many minutes.
    pub repeat_minutes: u64,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            webhook_url: None,
            ntfy_url: None,
            email: Vec::new(),
            min_severity: "warning".into(),
            repeat_minutes: 360,
        }
    }
}

/// `[durability.status]`: a small status page with a JSON API on its own
/// listener, so it is never exposed through the public reverse proxy by accident.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StatusConfig {
    /// Listen address, e.g. `0.0.0.0:8081`. Empty disables the page (the
    /// admin API `/admin/durability/*` and the CLI still work).
    pub listen: String,
    /// HTTP Basic password (user `admin`). Required when `listen` is set.
    pub password: Secret,
    /// Read the password from this file instead.
    pub password_file: Option<PathBuf>,
}

/// `[durability.fsview]`: read pool health from an FSView instance on the
/// storage host and steer writes away from failing mergerfs branches.
#[derive(Debug, Clone, Deserialize)]
pub struct FsviewConfig {
    /// FSView base URL, e.g. `http://192.168.1.10:9200`.
    pub url: String,
    /// FSView user (read-only use; create one with `fsview user add`).
    pub username: String,
    /// Password.
    #[serde(default)]
    pub password: Secret,
    /// Read the password from this file instead.
    #[serde(default)]
    pub password_file: Option<PathBuf>,
    /// Pool mountpoint as FSView sees it on the host, e.g. `/mnt/drivepool`.
    pub pool: String,
    /// The pool's branches as this server sees them (bind-mounted into the
    /// container). Without them Photovault reports pool health and alerts
    /// but cannot steer writes or move objects.
    #[serde(default)]
    pub branches: Vec<BranchMapping>,
    /// Poll interval, seconds.
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
}

/// One mergerfs branch: FSView's path and the matching storage root here.
#[derive(Debug, Clone, Deserialize)]
pub struct BranchMapping {
    /// Branch path on the host as FSView reports it, e.g. `/mnt/disk3`.
    pub host_path: String,
    /// Directory in this container that corresponds to the storage root
    /// (`[storage] path`) on that branch, e.g. `/branches/disk3/photovault`.
    pub path: PathBuf,
}

fn default_poll() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_example() {
        let c: crate::config::Config = toml::from_str(
            r#"
            [durability]
            sweep_interval_minutes = 30
            [durability.scrub]
            interval_days = 14
            window = "01:00-07:00"
            utc_offset = "+01:00"
            [[durability.targets]]
            name = "usb"
            kind = "local"
            path = "/backup/photovault"
            [[durability.targets]]
            name = "b2"
            kind = "s3"
            endpoint = "https://s3.eu-central-003.backblazeb2.com"
            region = "eu-central-003"
            bucket = "pv"
            access_key = "k"
            secret_key = "s"
            prefix = "photovault/"
            max_kib_per_sec = 2048
            window = "00:00-06:00"
            [durability.database_backup]
            enabled = true
            key_file = "/run/secrets/backup_key"
            [durability.alerts]
            ntfy_url = "https://ntfy.sh/my-topic"
            [durability.status]
            listen = "0.0.0.0:8081"
            password = "pw"
            [durability.fsview]
            url = "http://192.168.1.10:9200"
            username = "photovault"
            password = "x"
            pool = "/mnt/drivepool"
            [[durability.fsview.branches]]
            host_path = "/mnt/disk1"
            path = "/branches/disk1/photovault"
            "#,
        )
        .expect("toml");
        let d = &c.durability;
        assert_eq!(d.targets.len(), 2);
        assert_eq!(d.targets[0].verify_mode(), VerifyMode::Full);
        assert_eq!(d.targets[1].verify_mode(), VerifyMode::Sample);
        assert!(
            matches!(&d.targets[1].kind, TargetKind::S3 { prefix, .. } if prefix == "photovault/")
        );
        assert_eq!(d.fsview.as_ref().map(|f| f.branches.len()), Some(1));
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("\"pw\"") && !dbg.contains("\"x\""));
    }
}
