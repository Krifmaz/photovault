//! `photovault` binary: API server and admin CLI.

use clap::{Parser, Subcommand};
use photovault_server::{config::Config, state::build_state};
use std::path::PathBuf;

/// Photovault server (based on Ente, ente.com).
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Path to a TOML config file.
    #[arg(long, short, env = "PHOTOVAULT_CONFIG", global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the API server (default).
    Serve,
    /// Run database migrations and exit.
    Migrate,
    /// Probe `/healthz` on the local server (for Docker HEALTHCHECK; no shell or curl needed).
    Healthcheck {
        /// URL to probe.
        #[arg(
            long,
            env = "PHOTOVAULT_HEALTHCHECK_URL",
            default_value = "http://127.0.0.1:8080/healthz"
        )]
        url: String,
    },
    /// Administration commands.
    #[command(subcommand)]
    Admin(photovault_server::admin_cli::AdminCmd),
}

fn init_logging(format: &str) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if format == "pretty" {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_env_filter(filter)
            .init();
    }
}

/// Resolve on Ctrl-C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
    tracing::info!("shutdown signal received, draining connections");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    init_logging(&config.log_format);
    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => {
            let listen = config.listen.clone();
            let state = build_state(config).await?;
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            tracing::info!(%listen, storage = state.store.name(), "photovault listening");
            photovault_server::storage::spawn_maintenance(&state);
            photovault_server::library::spawn_background(state.clone());
            photovault_server::legacy::spawn_background(state.clone());
            axum::serve(listener, photovault_server::app(state))
                .with_graceful_shutdown(shutdown_signal())
                .await?;
            tracing::info!("photovault stopped");
        }
        Cmd::Migrate => {
            let db = photovault_server::db::Db::connect(&config.database_url).await?;
            db.migrate(&config.database_url).await?;
            tracing::info!("migrations applied");
        }
        Cmd::Healthcheck { url } => {
            photovault_server::admin_cli::healthcheck(&url).await?;
        }
        Cmd::Admin(cmd) => {
            let state = build_state(config).await?;
            photovault_server::admin_cli::run(&state, cmd).await?;
        }
    }
    Ok(())
}
