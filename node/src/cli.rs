//! Command-line interface of `photovault-node`.
//!
//! ```text
//! photovault-node [--config <path>] <command>
//!
//!   login    sign in to the account (server URL, email, password, 2FA)
//!   logout   sign out and delete the local session
//!   run      process the library continuously (daemon)
//!   once     one pass over the library, then exit
//!   status   show what a running (or the last) node is doing
//!   gpu      show detected GPUs and inference providers
//!   config   print the effective configuration
//! ```
//!
//! The account-specific parts (sign-in and building the Ente API client,
//! ML indexer and transcoder) are behind the [`Backend`] trait; the binary
//! uses [`default_backend`].

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, GpuProvider, LogConfig, LogFormat};
use crate::schedule::{self, BusyMonitor, GpuJobs, gpu};
use crate::state::{StateStore, TaskKind, unix_now, write_atomic};
use crate::worker::{Services, StatusSnapshot, Worker, WorkerOptions, clean_tmp_dir};

/// Photovault compute node: indexes faces and search and makes videos
/// streamable on hardware you own. Based on Ente (ente.com).
#[derive(Debug, Parser)]
#[command(name = "photovault-node", version, about, long_about = None)]
pub struct Cli {
    /// Configuration file (TOML). Default: the platform config dir, e.g.
    /// ~/.config/photovault-node/config.toml.
    #[arg(long, short = 'c', global = true, env = "PHOTOVAULT_NODE_CONFIG")]
    pub config: Option<PathBuf>,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in to the account, like adding another device.
    Login {
        /// Server URL (asked for when omitted; default from config).
        #[arg(long)]
        server: Option<String>,
        /// Account email (asked for when omitted).
        #[arg(long)]
        email: Option<String>,
    },
    /// Sign out and delete the local session.
    Logout,
    /// Process the library continuously.
    Run {
        /// Never pause for games, load, battery or quiet hours.
        #[arg(long)]
        no_pause: bool,
    },
    /// Process the library once, then exit.
    Once {
        /// Never pause for games, load, battery or quiet hours.
        #[arg(long)]
        no_pause: bool,
    },
    /// Show the node's status.
    Status {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show detected GPUs and inference providers.
    Gpu {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the effective configuration as TOML.
    Config,
}

// ---------------------------------------------------------------------------
// Backend: what the lead wires to the Ente API client, ML and video modules
// ---------------------------------------------------------------------------

/// Interactive prompts used during sign-in. Methods block on the terminal.
pub trait Prompts: Send + Sync {
    /// Asks a question and returns the trimmed answer.
    fn ask(&self, question: &str) -> Result<String>;
    /// Asks for a secret without echo (password).
    fn ask_secret(&self, question: &str) -> Result<String>;
    /// Shows a message.
    fn info(&self, message: &str);
}

/// What `login` asks the backend to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRequest {
    /// API server URL.
    pub server_url: String,
    /// Account email.
    pub email: String,
}

/// Result of a successful sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSummary {
    /// Account email.
    pub email: String,
    /// Ente user ID.
    pub user_id: i64,
}

/// Account-specific operations.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Signs in: SRP (or email OTT) with the password from
    /// `prompts.ask_secret`, then TOTP via `prompts.ask` if the account
    /// has two-factor auth; decrypts the master key and writes the session
    /// to `cfg.session_path()` (mode 0600), recording the server URL.
    async fn login(
        &self,
        cfg: &Config,
        req: &LoginRequest,
        prompts: &dyn Prompts,
    ) -> Result<LoginSummary>;
    /// Invalidates the session token on the server. The CLI deletes the
    /// session file afterwards.
    async fn logout(&self, cfg: &Config) -> Result<()>;
    /// Loads the session and builds the worker services: library sync,
    /// fetcher, file-data store, and (when enabled in `cfg.tasks`) the ML
    /// indexer on `provider` and the video transcoder.
    async fn services(&self, cfg: &Config, provider: GpuProvider) -> Result<Services>;
}

/// The backend used by the binary.
///
/// TODO(lead): return the concrete backend built on `crate::api`,
/// `crate::library`, `crate::ml` and `crate::video`. Until then every
/// account command fails with a clear error, while `status`, `gpu` and
/// `config` work.
pub fn default_backend() -> Box<dyn Backend> {
    Box::new(UnwiredBackend)
}

/// Placeholder backend that reports it is not wired yet.
#[derive(Debug, Default)]
pub struct UnwiredBackend;

#[async_trait]
impl Backend for UnwiredBackend {
    async fn login(&self, _: &Config, _: &LoginRequest, _: &dyn Prompts) -> Result<LoginSummary> {
        bail!("sign-in is not available in this build (backend not wired)")
    }
    async fn logout(&self, _: &Config) -> Result<()> {
        Ok(())
    }
    async fn services(&self, _: &Config, _: GpuProvider) -> Result<Services> {
        bail!("processing is not available in this build (backend not wired)")
    }
}

/// Prompts on the controlling terminal (stderr + stdin; passwords via
/// `rpassword`, without echo).
#[derive(Debug, Default)]
pub struct TerminalPrompts;

impl Prompts for TerminalPrompts {
    fn ask(&self, question: &str) -> Result<String> {
        let mut err = std::io::stderr();
        write!(err, "{question}")?;
        err.flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            bail!("no input");
        }
        Ok(line.trim().to_string())
    }
    fn ask_secret(&self, question: &str) -> Result<String> {
        Ok(rpassword::prompt_password(question)?)
    }
    fn info(&self, message: &str) {
        eprintln!("{message}");
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Binary entry point.
pub fn main() -> ExitCode {
    main_with(default_backend().as_ref())
}

/// Entry point with an explicit backend.
pub fn main_with(backend: &dyn Backend) -> ExitCode {
    let cli = Cli::parse();
    let cfg = match Config::load(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = init_tracing(&cfg.log) {
        eprintln!("warning: logging setup failed: {e:#}");
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: starting runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(dispatch(cli.command, cfg, backend)) {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "exiting");
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Sets up `tracing` on stderr in the configured format. `RUST_LOG`
/// overrides the configured level.
pub fn init_tracing(log: &LogConfig) -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&log.level))
        .context("invalid log level")?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let res = match log.format {
        LogFormat::Text => builder.try_init(),
        LogFormat::Json => builder.json().flatten_event(true).try_init(),
    };
    res.map_err(|e| anyhow::anyhow!("{e}"))
}

/// Runs one command.
pub async fn dispatch(cmd: Command, cfg: Config, backend: &dyn Backend) -> Result<ExitCode> {
    match cmd {
        Command::Login { server, email } => login(cfg, backend, server, email).await,
        Command::Logout => {
            backend.logout(&cfg).await?;
            let p = cfg.session_path();
            match std::fs::remove_file(&p) {
                Ok(()) => println!("Signed out; removed {}", p.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("Not signed in."),
                Err(e) => return Err(e).with_context(|| format!("removing {}", p.display())),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Run { no_pause } => process(cfg, backend, no_pause, false).await,
        Command::Once { no_pause } => process(cfg, backend, no_pause, true).await,
        Command::Status { json } => status(&cfg, json),
        Command::Gpu { json } => gpu_cmd(&cfg, json).await,
        Command::Config => {
            print!("{}", cfg.to_toml()?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn login(
    mut cfg: Config,
    backend: &dyn Backend,
    server: Option<String>,
    email: Option<String>,
) -> Result<ExitCode> {
    let prompts = TerminalPrompts;
    let server_url = match server {
        Some(s) => s,
        None => {
            let a = prompts.ask(&format!("Server URL [{}]: ", cfg.server_url))?;
            if a.is_empty() {
                cfg.server_url.clone()
            } else {
                a
            }
        }
    };
    url::Url::parse(&server_url).with_context(|| format!("{server_url:?} is not a URL"))?;
    cfg.server_url = server_url.clone();
    let email = match email {
        Some(e) => e,
        None => prompts.ask("Email: ")?,
    };
    if !email.contains('@') {
        bail!("{email:?} is not an email address");
    }
    create_private_dir(&cfg.data_dir)?;
    let summary = backend
        .login(&cfg, &LoginRequest { server_url, email }, &prompts)
        .await?;
    println!(
        "Signed in as {} (user {}). Session saved to {}.",
        summary.email,
        summary.user_id,
        cfg.session_path().display()
    );
    Ok(ExitCode::SUCCESS)
}

async fn process(
    cfg: Config,
    backend: &dyn Backend,
    no_pause: bool,
    once: bool,
) -> Result<ExitCode> {
    create_private_dir(&cfg.data_dir)?;
    let _lock = InstanceLock::acquire(&cfg.lock_path())?;
    clean_tmp_dir(&cfg.tmp_dir());

    let report = gpu::detect().await;
    let provider = report.resolve(cfg.gpu.provider);
    if cfg.gpu.provider != GpuProvider::Auto && provider != cfg.gpu.provider {
        tracing::warn!(requested = %cfg.gpu.provider, using = %provider, "GPU provider not available");
    }
    tracing::info!(%provider, gpus = report.gpus.len(), "inference provider");

    let services = backend.services(&cfg, provider).await?;
    let shutdown = CancellationToken::new();
    tokio::spawn(watch_signals(shutdown.clone()));

    let gpu_jobs = GpuJobs::default();
    let mut _always = None;
    let gate = if no_pause || !cfg.busy.enabled {
        let (tx, rx) = schedule::always_run();
        _always = Some(tx);
        rx
    } else {
        BusyMonitor::new(&cfg.busy, report, gpu_jobs.clone())?
            .spawn(shutdown.clone())
            .await
    };

    let store = StateStore::load(cfg.state_path())?;
    let mut worker = Worker::new(
        services,
        WorkerOptions::from_config(&cfg),
        store,
        gate,
        gpu_jobs,
    );
    if once {
        let s = worker.run_once(shutdown.clone()).await?;
        println!(
            "Pass finished: {} files, {} completed, {} already on server, {} failed, {} unfinished.",
            s.files, s.completed, s.already_remote, s.failed, s.unfinished
        );
        Ok(if s.failed > 0 {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        })
    } else {
        worker.run(shutdown.clone()).await?;
        Ok(ExitCode::SUCCESS)
    }
}

/// Cancels `shutdown` on SIGINT/SIGTERM; a second signal exits at once.
async fn watch_signals(shutdown: CancellationToken) {
    let mut count = 0;
    loop {
        wait_signal().await;
        count += 1;
        if count == 1 {
            tracing::info!("signal received; finishing (press Ctrl-C again to exit now)");
            shutdown.cancel();
        } else {
            eprintln!("exiting immediately");
            std::process::exit(130);
        }
    }
}

#[cfg(unix)]
async fn wait_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn status(cfg: &Config, json: bool) -> Result<ExitCode> {
    let snap: Option<StatusSnapshot> = std::fs::read(cfg.status_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let state = StateStore::load(cfg.state_path())?.state;
    let running = snap
        .as_ref()
        .is_some_and(|s| !matches!(s.phase, crate::worker::Phase::Stopped) && pid_alive(s.pid));
    if json {
        let v = serde_json::json!({
            "running": running,
            "status": snap,
            "counters": state.counters,
            "ml": tally_json(&state, TaskKind::Ml),
            "video": tally_json(&state, TaskKind::Video),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(ExitCode::SUCCESS);
    }
    print!(
        "{}",
        render_status(snap.as_ref(), running, &state, unix_now())
    );
    Ok(ExitCode::SUCCESS)
}

fn tally_json(state: &crate::state::State, kind: TaskKind) -> serde_json::Value {
    let (done, retrying, given_up) = state.tally(kind);
    serde_json::json!({"done": done, "retrying": retrying, "given_up": given_up})
}

/// Human-readable status report.
pub fn render_status(
    snap: Option<&StatusSnapshot>,
    running: bool,
    state: &crate::state::State,
    now: i64,
) -> String {
    let mut o = String::new();
    let ago = |t: i64| human_secs(now - t);
    match snap {
        Some(s) if running => {
            o += &format!(
                "Node: running (pid {}), {:?}, {}\n",
                s.pid, s.phase, s.decision
            );
            o += &format!(
                "Library: {} files; queued: {} ML, {} video; in progress: {}\n",
                s.library_files,
                s.queued_ml,
                s.queued_video,
                s.in_flight.len()
            );
            for i in &s.in_flight {
                o += &format!(
                    "  - file {} ({}) for {}\n",
                    i.file_id,
                    i.kind,
                    ago(i.started_at)
                );
            }
            if let Some(n) = s.next_sync_at {
                o += &format!("Next sync in {}\n", human_secs(n - now));
            }
        }
        Some(s) => {
            o += "Node: not running\n";
            if s.library_files > 0 {
                o += &format!("Library at last run: {} files\n", s.library_files);
            }
        }
        None => o += "Node: not running (no status yet)\n",
    }
    if let Some(s) = snap {
        if let Some(t) = s.last_sync_at {
            o += &format!("Last sync: {} ago\n", ago(t));
        }
        if let Some(e) = &s.last_error {
            o += &format!("Last error: {e}\n");
        }
    }
    let c = &state.counters;
    let (ml_done, ml_retry, ml_gave_up) = state.tally(TaskKind::Ml);
    let (v_done, v_retry, v_gave_up) = state.tally(TaskKind::Video);
    o += &format!(
        "Faces/search: {ml_done} done ({} indexed here, {} already on server, {} conflicts); {ml_retry} retrying, {ml_gave_up} given up\n",
        c.ml_uploaded, c.ml_already_remote, c.ml_conflicts
    );
    o += &format!(
        "Video: {v_done} done ({} transcoded here, {} already on server, {} not needed, {} conflicts); {v_retry} retrying, {v_gave_up} given up\n",
        c.video_uploaded, c.video_already_remote, c.video_not_needed, c.video_conflicts
    );
    if let Some(t) = c.last_pass_at {
        o += &format!("Passes: {} (last finished {} ago)\n", c.passes, ago(t));
    }
    o
}

fn human_secs(s: i64) -> String {
    let s = s.max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m{:02}s", s / 60, s % 60),
        3600..86400 => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d{:02}h", s / 86400, (s % 86400) / 3600),
    }
}

async fn gpu_cmd(cfg: &Config, json: bool) -> Result<ExitCode> {
    let report = gpu::detect().await;
    let selected = report.resolve(cfg.gpu.provider);
    let util = gpu::utilisation(&report).await;
    if json {
        let v = serde_json::json!({
            "gpus": report.gpus,
            "providers": report.providers,
            "configured": cfg.gpu.provider,
            "selected": selected,
            "utilisation_percent": util,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(ExitCode::SUCCESS);
    }
    if report.gpus.is_empty() {
        println!("No GPUs detected.");
    }
    for (i, g) in report.gpus.iter().enumerate() {
        let mem = g
            .memory_mib
            .map(|m| format!(", {m} MiB"))
            .unwrap_or_default();
        let drv = g
            .driver
            .as_deref()
            .map(|d| format!(", driver {d}"))
            .unwrap_or_default();
        println!("GPU {i}: {} ({:?}{mem}{drv})", g.name, g.vendor);
    }
    let providers: Vec<_> = report.providers.iter().map(|p| p.as_str()).collect();
    println!("Available providers: {}", providers.join(", "));
    println!("Configured: {}  ->  using: {selected}", cfg.gpu.provider);
    if let Some(u) = util {
        println!("Current GPU utilisation: {u:.0}%");
    }
    Ok(ExitCode::SUCCESS)
}

fn create_private_dir(p: &Path) -> Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)
        .with_context(|| format!("creating {}", p.display()))
}

fn pid_alive(pid: u32) -> bool {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).is_some()
}

/// A PID file that keeps two nodes from sharing one data directory.
#[derive(Debug)]
pub struct InstanceLock(PathBuf);

impl InstanceLock {
    /// Takes the lock, replacing a stale PID file whose process is gone.
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Ok(s) = std::fs::read_to_string(path)
            && let Ok(pid) = s.trim().parse::<u32>()
            && pid != std::process::id()
            && pid_alive(pid)
        {
            bail!(
                "another node (pid {pid}) is using this data directory; stop it or remove {}",
                path.display()
            );
        }
        write_atomic(path, std::process::id().to_string().as_bytes())?;
        Ok(Self(path.to_path_buf()))
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Completion, FileKey, State, Versions};

    #[test]
    fn parses_commands() {
        let c = Cli::try_parse_from([
            "photovault-node",
            "--config",
            "/x.toml",
            "run",
            "--no-pause",
        ])
        .unwrap();
        assert_eq!(c.config.as_deref(), Some(Path::new("/x.toml")));
        assert!(matches!(c.command, Command::Run { no_pause: true }));
        let c = Cli::try_parse_from(["photovault-node", "login", "--email", "a@b.c"]).unwrap();
        assert!(matches!(
            c.command,
            Command::Login {
                email: Some(_),
                server: None
            }
        ));
        let c = Cli::try_parse_from(["photovault-node", "status", "--json", "-c", "y"]).unwrap();
        assert!(matches!(c.command, Command::Status { json: true }));
        assert!(Cli::try_parse_from(["photovault-node"]).is_err());
        assert!(Cli::try_parse_from(["photovault-node", "bogus"]).is_err());
    }

    #[test]
    fn human_durations() {
        assert_eq!(human_secs(5), "5s");
        assert_eq!(human_secs(65), "1m05s");
        assert_eq!(human_secs(7260), "2h01m");
        assert_eq!(human_secs(90000), "1d01h");
        assert_eq!(human_secs(-3), "0s");
    }

    #[test]
    fn status_rendering() {
        let mut st = State::default();
        st.mark_done(
            FileKey::new(1, 1),
            TaskKind::Ml,
            Completion::Uploaded,
            Versions::default(),
            0,
        );
        let out = render_status(None, false, &st, 100);
        assert!(out.contains("not running"));
        assert!(out.contains("Faces/search: 1 done (1 indexed here"));
    }

    #[test]
    fn instance_lock() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("node.pid");
        {
            let _l = InstanceLock::acquire(&p).unwrap();
            assert!(p.exists());
            // Same process may re-acquire (stale file from itself).
            let _l2 = InstanceLock::acquire(&p).unwrap();
        }
        assert!(!p.exists());
        // A dead PID is stale.
        std::fs::write(&p, "4000000000").unwrap();
        let _l = InstanceLock::acquire(&p).unwrap();
    }

    #[tokio::test]
    async fn unwired_backend_errors_cleanly() {
        let cfg = Config::default();
        let b = UnwiredBackend;
        assert!(b.services(&cfg, GpuProvider::Cpu).await.is_err());
        assert!(b.logout(&cfg).await.is_ok());
    }
}
