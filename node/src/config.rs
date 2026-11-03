//! Node configuration: a TOML file with defaults and environment overrides.
//!
//! Lookup order (later wins):
//!
//! 1. built-in defaults ([`Config::default`]);
//! 2. the TOML file: `--config <path>` / `PHOTOVAULT_NODE_CONFIG`, else
//!    `<config dir>/photovault-node/config.toml` if it exists
//!    (`~/.config/photovault-node/config.toml` on Linux);
//! 3. `PHOTOVAULT_NODE_*` environment variables ([`Config::apply_env`]).
//!
//! Every field is optional in the file. Example:
//!
//! ```toml
//! server_url = "https://photos.example.com"
//!
//! [tasks]
//! faces = true
//! clip = true
//! video = false
//!
//! [gpu]
//! provider = "rocm"
//!
//! [busy]
//! cpu_pause_percent = 50
//! quiet_hours = "18:00-23:30"
//! games = ["cmd:SteamLaunch", "*.exe", "MyEmulator"]
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::schedule::decide::{DecideParams, QuietHours};
use crate::schedule::games::{default_game_patterns, default_ignore_patterns};

/// Prefix of every environment override.
pub const ENV_PREFIX: &str = "PHOTOVAULT_NODE_";

/// The whole node configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Base URL of the Photovault (or Ente-compatible) API server.
    /// Env: `PHOTOVAULT_NODE_SERVER_URL`.
    pub server_url: String,
    /// Where state, temporary plaintext files (mode 0700, deleted after use)
    /// and the status file live. Default: `<data dir>/photovault-node`
    /// (`~/.local/share/photovault-node` on Linux).
    /// Env: `PHOTOVAULT_NODE_DATA_DIR`.
    pub data_dir: PathBuf,
    /// Encrypted session written by `login`. Default:
    /// `<data_dir>/session.json`. Env: `PHOTOVAULT_NODE_SESSION_FILE`.
    pub session_file: Option<PathBuf>,
    /// Seconds between library syncs when the node is idle. Default 300.
    /// Env: `PHOTOVAULT_NODE_POLL_INTERVAL_SECS`.
    pub poll_interval_secs: u64,
    /// Which results to produce.
    pub tasks: TasksConfig,
    /// GPU / inference provider selection.
    pub gpu: GpuConfig,
    /// Parallelism.
    pub concurrency: ConcurrencyConfig,
    /// When to pause because the machine is busy.
    pub busy: BusyConfig,
    /// Streamable-video generation.
    pub video: VideoConfig,
    /// Network limits.
    pub network: NetworkConfig,
    /// Retry policy for failed files.
    pub retry: RetryConfig,
    /// Logging.
    pub log: LogConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:8080".into(),
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("photovault-node"),
            session_file: None,
            poll_interval_secs: 300,
            tasks: TasksConfig::default(),
            gpu: GpuConfig::default(),
            concurrency: ConcurrencyConfig::default(),
            busy: BusyConfig::default(),
            video: VideoConfig::default(),
            network: NetworkConfig::default(),
            retry: RetryConfig::default(),
            log: LogConfig::default(),
        }
    }
}

/// Which results the node produces. All on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TasksConfig {
    /// Face detection + embeddings (the `face` part of `mldata`).
    pub faces: bool,
    /// CLIP image embeddings for magic search (the `clip` part of `mldata`).
    pub clip: bool,
    /// Streamable HLS video previews (`vid_preview`).
    pub video: bool,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            faces: true,
            clip: true,
            video: true,
        }
    }
}

impl TasksConfig {
    /// Whether any ML task is enabled.
    pub fn ml(&self) -> bool {
        self.faces || self.clip
    }
}

/// Inference provider for ML (and hardware hints for video).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuProvider {
    /// Pick the best available: cuda, rocm, migraphx, webgpu, then cpu.
    #[default]
    Auto,
    /// NVIDIA CUDA.
    Cuda,
    /// AMD ROCm.
    Rocm,
    /// AMD MIGraphX (on ROCm).
    Migraphx,
    /// WebGPU (Vulkan/Metal/DX12) — vendor-neutral.
    Webgpu,
    /// CPU only.
    Cpu,
}

impl GpuProvider {
    /// All concrete providers, best first (excludes `Auto`).
    pub const PREFERENCE: [GpuProvider; 5] = [
        GpuProvider::Cuda,
        GpuProvider::Rocm,
        GpuProvider::Migraphx,
        GpuProvider::Webgpu,
        GpuProvider::Cpu,
    ];

    /// Lower-case name as used in the config file.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Migraphx => "migraphx",
            Self::Webgpu => "webgpu",
            Self::Cpu => "cpu",
        }
    }

    /// Whether this provider uses a GPU.
    pub fn is_gpu(&self) -> bool {
        !matches!(self, Self::Cpu)
    }
}

impl std::fmt::Display for GpuProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for GpuProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "cuda" => Self::Cuda,
            "rocm" => Self::Rocm,
            "migraphx" => Self::Migraphx,
            "webgpu" | "vulkan" => Self::Webgpu,
            "cpu" => Self::Cpu,
            other => return Err(format!("unknown GPU provider {other:?}")),
        })
    }
}

/// GPU selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GpuConfig {
    /// `auto` (default), `cuda`, `rocm`, `migraphx`, `webgpu` or `cpu`.
    /// An explicit choice that is not available falls back to `cpu` with a
    /// warning. Env: `PHOTOVAULT_NODE_GPU_PROVIDER`.
    pub provider: GpuProvider,
    /// Index of the GPU to use when there are several. Default 0.
    pub device: u32,
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            provider: GpuProvider::Auto,
            device: 0,
        }
    }
}

/// Parallelism limits. Each must be at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConcurrencyConfig {
    /// Simultaneous downloads (originals/thumbnails). Default 4, as the
    /// Ente web indexer. Env: `PHOTOVAULT_NODE_CONCURRENCY_DOWNLOADS`.
    pub downloads: usize,
    /// Files indexed (faces/CLIP) at once. Default 2.
    /// Env: `PHOTOVAULT_NODE_CONCURRENCY_ML`.
    pub ml: usize,
    /// Videos transcoded at once. Default 1.
    /// Env: `PHOTOVAULT_NODE_CONCURRENCY_VIDEO`.
    pub video: usize,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            downloads: 4,
            ml: 2,
            video: 1,
        }
    }
}

/// When the node pauses (see [`crate::schedule`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BusyConfig {
    /// Master switch: `false` never pauses. Default true.
    pub enabled: bool,
    /// Seconds between measurements. Default 5.
    pub sample_secs: u64,
    /// Pause when other processes use more than this much CPU (percent of
    /// the whole machine) for `sustain_secs`. Default 60.
    /// Env: `PHOTOVAULT_NODE_CPU_PAUSE_PERCENT`.
    pub cpu_pause_percent: f32,
    /// Resume only below this CPU load. Default 35.
    pub cpu_resume_percent: f32,
    /// Pause when GPU utilisation exceeds this for `sustain_secs` (only
    /// while the node is not using the GPU itself). Default 40.
    /// Env: `PHOTOVAULT_NODE_GPU_PAUSE_PERCENT`.
    pub gpu_pause_percent: f32,
    /// Resume only below this GPU utilisation. Default 15.
    pub gpu_resume_percent: f32,
    /// Seconds load must stay high before pausing. Default 20.
    pub sustain_secs: u64,
    /// Seconds everything must stay clear before resuming. Default 120.
    pub resume_after_secs: u64,
    /// Pause while on battery. Default true.
    /// Env: `PHOTOVAULT_NODE_PAUSE_ON_BATTERY`.
    pub pause_on_battery: bool,
    /// Pause while a process from `games` runs. Default true.
    pub pause_for_games: bool,
    /// Game/launcher patterns (see [`crate::schedule::games`]): a glob
    /// over the process name, or `cmd:<text>` for a command-line substring.
    /// Default: [`default_game_patterns`].
    pub games: Vec<String>,
    /// Patterns never treated as games. Default: Wine background helpers.
    pub ignore_processes: Vec<String>,
    /// Optional local-time window with no work, `"HH:MM-HH:MM"` (may wrap
    /// midnight). Env: `PHOTOVAULT_NODE_QUIET_HOURS` (empty disables).
    pub quiet_hours: Option<String>,
}

impl Default for BusyConfig {
    fn default() -> Self {
        let d = DecideParams::default();
        Self {
            enabled: true,
            sample_secs: 5,
            cpu_pause_percent: d.cpu_pause_percent,
            cpu_resume_percent: d.cpu_resume_percent,
            gpu_pause_percent: d.gpu_pause_percent,
            gpu_resume_percent: d.gpu_resume_percent,
            sustain_secs: d.sustain.as_secs(),
            resume_after_secs: d.resume_after.as_secs(),
            pause_on_battery: d.pause_on_battery,
            pause_for_games: d.pause_for_games,
            games: default_game_patterns(),
            ignore_processes: default_ignore_patterns(),
            quiet_hours: None,
        }
    }
}

impl BusyConfig {
    /// Decision parameters for [`crate::schedule::decide::Decider`].
    pub fn decide_params(&self) -> Result<DecideParams> {
        Ok(DecideParams {
            cpu_pause_percent: self.cpu_pause_percent,
            cpu_resume_percent: self.cpu_resume_percent,
            gpu_pause_percent: self.gpu_pause_percent,
            gpu_resume_percent: self.gpu_resume_percent,
            sustain: Duration::from_secs(self.sustain_secs),
            resume_after: Duration::from_secs(self.resume_after_secs),
            pause_on_battery: self.pause_on_battery,
            pause_for_games: self.pause_for_games,
            quiet_hours: match self.quiet_hours.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(q) => Some(QuietHours::parse(q).map_err(anyhow::Error::msg)?),
            },
        })
    }
}

/// Streamable video (HLS) settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VideoConfig {
    /// ffmpeg executable (looked up in `PATH` when not absolute).
    /// Env: `PHOTOVAULT_NODE_FFMPEG`.
    pub ffmpeg: PathBuf,
    /// ffprobe executable. Env: `PHOTOVAULT_NODE_FFPROBE`.
    pub ffprobe: PathBuf,
    /// Skip videos of this size or larger, in MiB; 0 = no limit.
    /// Default 500, as the Ente mobile app.
    pub max_size_mib: u64,
    /// Skip videos longer than this, in seconds; 0 = no limit.
    /// Default 60, as the Ente mobile app's automatic processing.
    pub max_duration_secs: u64,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            max_size_mib: 500,
            max_duration_secs: 60,
        }
    }
}

/// Network limits, passed to the API client and uploader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Combined download+upload limit in KiB/s; 0 = unlimited (default).
    /// Env: `PHOTOVAULT_NODE_BANDWIDTH_LIMIT_KIBPS`.
    pub bandwidth_limit_kibps: u64,
}

/// Retry policy for files that fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetryConfig {
    /// Attempts per file (and per file version) before giving up until the
    /// file changes. Default 5.
    pub max_attempts: u32,
    /// Delay after the first failure, in seconds; doubles each attempt.
    /// Default 300.
    pub base_delay_secs: u64,
    /// Upper bound of the delay, in seconds. Default 86400 (one day).
    pub max_delay_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_secs: 300,
            max_delay_secs: 86_400,
        }
    }
}

impl RetryConfig {
    /// Delay before retrying after `attempts` failures (1-based).
    pub fn delay(&self, attempts: u32) -> Duration {
        let exp = attempts.saturating_sub(1).min(32);
        let secs = self
            .base_delay_secs
            .saturating_mul(1u64 << exp)
            .min(self.max_delay_secs);
        Duration::from_secs(secs)
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable lines (default).
    #[default]
    Text,
    /// One JSON object per line.
    Json,
}

impl std::str::FromStr for LogFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            o => Err(format!("unknown log format {o:?}")),
        }
    }
}

/// Logging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// `text` (default) or `json`. Env: `PHOTOVAULT_NODE_LOG_FORMAT`.
    pub format: LogFormat,
    /// `tracing` filter, e.g. `info` or `photovault_node=debug`. `RUST_LOG`
    /// overrides it. Default `info`. Env: `PHOTOVAULT_NODE_LOG_LEVEL`.
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Text,
            level: "info".into(),
        }
    }
}

impl Config {
    /// The default config file location, if the platform has a config dir.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("photovault-node").join("config.toml"))
    }

    /// Loads the configuration: defaults, then `path` (or the default path
    /// if it exists), then the process environment.
    ///
    /// An explicitly given path must exist.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut cfg = match path {
            Some(p) => Self::from_file(p)?,
            None => match Self::default_path() {
                Some(p) if p.exists() => Self::from_file(&p)?,
                _ => Self::default(),
            },
        };
        cfg.apply_env(std::env::vars())?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reads and parses one TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Self::from_toml(&text).with_context(|| format!("parsing config {}", path.display()))
    }

    /// Parses TOML text; absent fields take their defaults.
    pub fn from_toml(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// Serialises the effective configuration as TOML.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Applies `PHOTOVAULT_NODE_*` overrides from `vars`.
    ///
    /// Recognised (after the prefix): `SERVER_URL`, `DATA_DIR`,
    /// `SESSION_FILE`, `POLL_INTERVAL_SECS`, `TASKS` (comma list of
    /// `faces,clip,video`, or empty for none), `GPU_PROVIDER`,
    /// `CONCURRENCY_DOWNLOADS`, `CONCURRENCY_ML`, `CONCURRENCY_VIDEO`,
    /// `CPU_PAUSE_PERCENT`, `GPU_PAUSE_PERCENT`, `PAUSE_ON_BATTERY`,
    /// `QUIET_HOURS`, `FFMPEG`, `FFPROBE`, `BANDWIDTH_LIMIT_KIBPS`,
    /// `LOG_FORMAT`, `LOG_LEVEL`. `CONFIG` is read by the CLI. Unknown
    /// variables are ignored.
    pub fn apply_env<I: IntoIterator<Item = (String, String)>>(&mut self, vars: I) -> Result<()> {
        for (k, v) in vars {
            let Some(key) = k.strip_prefix(ENV_PREFIX) else {
                continue;
            };
            let ctx = || format!("environment variable {k}");
            match key {
                "SERVER_URL" => self.server_url = v,
                "DATA_DIR" => self.data_dir = v.into(),
                "SESSION_FILE" => self.session_file = Some(v.into()),
                "POLL_INTERVAL_SECS" => self.poll_interval_secs = v.parse().with_context(ctx)?,
                "TASKS" => {
                    let mut t = TasksConfig {
                        faces: false,
                        clip: false,
                        video: false,
                    };
                    for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                        match part {
                            "faces" => t.faces = true,
                            "clip" => t.clip = true,
                            "video" => t.video = true,
                            o => bail!("{}: unknown task {o:?}", ctx()),
                        }
                    }
                    self.tasks = t;
                }
                "GPU_PROVIDER" => {
                    self.gpu.provider = v.parse().map_err(anyhow::Error::msg).with_context(ctx)?
                }
                "CONCURRENCY_DOWNLOADS" => {
                    self.concurrency.downloads = v.parse().with_context(ctx)?
                }
                "CONCURRENCY_ML" => self.concurrency.ml = v.parse().with_context(ctx)?,
                "CONCURRENCY_VIDEO" => self.concurrency.video = v.parse().with_context(ctx)?,
                "CPU_PAUSE_PERCENT" => self.busy.cpu_pause_percent = v.parse().with_context(ctx)?,
                "GPU_PAUSE_PERCENT" => self.busy.gpu_pause_percent = v.parse().with_context(ctx)?,
                "PAUSE_ON_BATTERY" => {
                    self.busy.pause_on_battery = parse_bool(&v).with_context(ctx)?
                }
                "QUIET_HOURS" => {
                    self.busy.quiet_hours = (!v.trim().is_empty()).then_some(v);
                }
                "FFMPEG" => self.video.ffmpeg = v.into(),
                "FFPROBE" => self.video.ffprobe = v.into(),
                "BANDWIDTH_LIMIT_KIBPS" => {
                    self.network.bandwidth_limit_kibps = v.parse().with_context(ctx)?
                }
                "LOG_FORMAT" => {
                    self.log.format = v.parse().map_err(anyhow::Error::msg).with_context(ctx)?
                }
                "LOG_LEVEL" => self.log.level = v,
                _ => {}
            }
        }
        Ok(())
    }

    /// Checks invariants (non-zero concurrency, threshold order, URL).
    pub fn validate(&self) -> Result<()> {
        url::Url::parse(&self.server_url)
            .with_context(|| format!("server_url {:?} is not a URL", self.server_url))?;
        let c = &self.concurrency;
        if c.downloads == 0 || c.ml == 0 || c.video == 0 {
            bail!("concurrency values must be at least 1");
        }
        let b = &self.busy;
        if b.cpu_resume_percent > b.cpu_pause_percent {
            bail!("busy.cpu_resume_percent must not exceed busy.cpu_pause_percent");
        }
        if b.gpu_resume_percent > b.gpu_pause_percent {
            bail!("busy.gpu_resume_percent must not exceed busy.gpu_pause_percent");
        }
        if b.sample_secs == 0 {
            bail!("busy.sample_secs must be at least 1");
        }
        b.decide_params()?;
        if self.retry.max_attempts == 0 {
            bail!("retry.max_attempts must be at least 1");
        }
        Ok(())
    }

    /// The session file path (explicit or `<data_dir>/session.json`).
    pub fn session_path(&self) -> PathBuf {
        self.session_file
            .clone()
            .unwrap_or_else(|| self.data_dir.join("session.json"))
    }

    /// Path of the persistent progress file.
    pub fn state_path(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    /// Path of the status snapshot written by `run`/`once` for `status`.
    pub fn status_path(&self) -> PathBuf {
        self.data_dir.join("status.json")
    }

    /// Directory for temporary plaintext files.
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    /// Path of the single-instance lock file.
    pub fn lock_path(&self) -> PathBuf {
        self.data_dir.join("node.pid")
    }

    /// Interval between idle library syncs.
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs.max(1))
    }
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        o => bail!("expected a boolean, got {o:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn empty_toml_is_default() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
        Config::default().validate().unwrap();
    }

    #[test]
    fn partial_toml() {
        let c = Config::from_toml(
            r#"
            server_url = "https://photos.example.com"
            [tasks]
            video = false
            [gpu]
            provider = "rocm"
            [busy]
            quiet_hours = "22:00-06:00"
            games = ["foo"]
            [log]
            format = "json"
            "#,
        )
        .unwrap();
        assert_eq!(c.server_url, "https://photos.example.com");
        assert!(c.tasks.faces && c.tasks.clip && !c.tasks.video);
        assert_eq!(c.gpu.provider, GpuProvider::Rocm);
        assert_eq!(c.busy.games, vec!["foo"]);
        assert_eq!(c.busy.cpu_pause_percent, 60.0);
        assert_eq!(c.log.format, LogFormat::Json);
        assert!(c.busy.decide_params().unwrap().quiet_hours.is_some());
        c.validate().unwrap();
    }

    #[test]
    fn unknown_fields_rejected() {
        assert!(Config::from_toml("bogus = 1").is_err());
        assert!(Config::from_toml("[gpu]\nprovider = \"tpu\"").is_err());
    }

    #[test]
    fn roundtrip() {
        let c = Config::default();
        assert_eq!(Config::from_toml(&c.to_toml().unwrap()).unwrap(), c);
    }

    #[test]
    fn env_overrides() {
        let mut c = Config::default();
        c.apply_env(env(&[
            ("PHOTOVAULT_NODE_SERVER_URL", "https://x.test"),
            ("PHOTOVAULT_NODE_TASKS", "clip, video"),
            ("PHOTOVAULT_NODE_GPU_PROVIDER", "CUDA"),
            ("PHOTOVAULT_NODE_CONCURRENCY_ML", "3"),
            ("PHOTOVAULT_NODE_PAUSE_ON_BATTERY", "no"),
            ("PHOTOVAULT_NODE_QUIET_HOURS", "01:00-02:00"),
            ("PHOTOVAULT_NODE_LOG_FORMAT", "json"),
            ("PHOTOVAULT_NODE_BANDWIDTH_LIMIT_KIBPS", "2048"),
            ("UNRELATED", "x"),
            ("PHOTOVAULT_NODE_SOMETHING_ELSE", "x"),
        ]))
        .unwrap();
        assert_eq!(c.server_url, "https://x.test");
        assert_eq!(
            c.tasks,
            TasksConfig {
                faces: false,
                clip: true,
                video: true
            }
        );
        assert_eq!(c.gpu.provider, GpuProvider::Cuda);
        assert_eq!(c.concurrency.ml, 3);
        assert!(!c.busy.pause_on_battery);
        assert_eq!(c.busy.quiet_hours.as_deref(), Some("01:00-02:00"));
        assert_eq!(c.log.format, LogFormat::Json);
        assert_eq!(c.network.bandwidth_limit_kibps, 2048);
        c.validate().unwrap();

        c.apply_env(env(&[("PHOTOVAULT_NODE_QUIET_HOURS", "")]))
            .unwrap();
        assert_eq!(c.busy.quiet_hours, None);
        assert!(
            c.apply_env(env(&[("PHOTOVAULT_NODE_TASKS", "nope")]))
                .is_err()
        );
        assert!(
            c.apply_env(env(&[("PHOTOVAULT_NODE_CONCURRENCY_ML", "x")]))
                .is_err()
        );
    }

    #[test]
    fn validation() {
        let mut c = Config::default();
        c.concurrency.ml = 0;
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.busy.cpu_resume_percent = 90.0;
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.busy.quiet_hours = Some("7-8".into());
        assert!(c.validate().is_err());
        let c = Config {
            server_url: "not a url".into(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn retry_delay_backs_off() {
        let r = RetryConfig::default();
        assert_eq!(r.delay(1).as_secs(), 300);
        assert_eq!(r.delay(2).as_secs(), 600);
        assert_eq!(r.delay(3).as_secs(), 1200);
        assert_eq!(r.delay(100).as_secs(), 86_400);
    }

    #[test]
    fn paths() {
        let mut c = Config {
            data_dir: "/d".into(),
            ..Default::default()
        };
        assert_eq!(c.session_path(), PathBuf::from("/d/session.json"));
        c.session_file = Some("/s".into());
        assert_eq!(c.session_path(), PathBuf::from("/s"));
        assert_eq!(c.state_path(), PathBuf::from("/d/state.json"));
    }

    #[test]
    fn provider_parse() {
        assert_eq!(
            "vulkan".parse::<GpuProvider>().unwrap(),
            GpuProvider::Webgpu
        );
        assert!("x".parse::<GpuProvider>().is_err());
        assert!(!GpuProvider::Cpu.is_gpu());
    }
}
