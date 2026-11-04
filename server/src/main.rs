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
    /// Copy a PostgreSQL database into a new SQLite database (one time), and save the
    /// current keys to keys.toml next to it if that file does not exist.
    MigrateToSqlite {
        /// Source, e.g. postgres://photovault:pw@db:5432/photovault (default: PHOTOVAULT_DATABASE_URL).
        #[arg(long)]
        from: Option<String>,
        /// Target SQLite file (default: <config dir>/photovault.db).
        #[arg(long)]
        to: Option<std::path::PathBuf>,
    },
    /// Print three fresh secrets (PHOTOVAULT_JWT_SECRET, _ENCRYPTION_KEY, _HASHING_KEY).
    GenKeys,
    /// Announce a server on the LAN via mDNS (the `photovault-mdns` sidecar; run it on
    /// the host network).
    Announce {
        /// Base URL of the server to announce, as reachable from this container.
        #[arg(
            long,
            env = "PHOTOVAULT_ANNOUNCE_SERVER",
            default_value = "http://127.0.0.1:8080"
        )]
        server: String,
        /// SRV port to announce (default: the port of the server's home address).
        #[arg(long, env = "PHOTOVAULT_ANNOUNCE_PORT")]
        port: Option<u16>,
    },
}

/// Install the one global log subscriber: one line per event, `pretty` (default) or `json`.
fn init_logging(format: &str) {
    use std::io::IsTerminal;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let result = match format {
        "pretty" | "text" => builder
            .compact()
            .with_ansi(std::io::stderr().is_terminal())
            .with_target(false)
            .try_init(),
        _ => builder
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .try_init(),
    };
    if result.is_err() {
        eprintln!("logging was already initialised; keeping the first subscriber");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    let config = match Config::load(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("invalid configuration: {e:#}");
            std::process::exit(photovault_server::preflight::EX_CONFIG);
        }
    };
    let cmd = cli.cmd.unwrap_or(Cmd::Serve);
    if matches!(cmd, Cmd::GenKeys) {
        photovault_server::crypto::init();
        for name in [
            "PHOTOVAULT_JWT_SECRET",
            "PHOTOVAULT_ENCRYPTION_KEY",
            "PHOTOVAULT_HASHING_KEY",
        ] {
            let k = photovault_server::crypto::b64(&photovault_server::crypto::random_bytes(32));
            println!("{name}={k}");
        }
        return Ok(());
    }
    init_logging(&config.log_format);
    match cmd {
        Cmd::Serve => serve(config).await?,
        Cmd::Migrate => {
            let db = photovault_server::db::Db::connect(&config.database_url).await?;
            db.migrate(&config.database_url).await?;
            tracing::info!("migrations applied");
        }
        Cmd::Healthcheck { url } => {
            photovault_server::admin_cli::healthcheck(&url).await?;
        }
        Cmd::Admin(cmd) => {
            let mut config = config;
            if let Err(f) = photovault_server::keyfile::ensure(&mut config)
                .and_then(|_| photovault_server::preflight::check_keys(&config))
            {
                eprintln!("{f}");
                std::process::exit(photovault_server::preflight::EX_CONFIG);
            }
            let state = build_state(config).await?;
            photovault_server::admin_cli::run(&state, cmd).await?;
        }
        Cmd::Announce { server, port } => {
            photovault_server::discovery::run_sidecar(&server, port).await?;
        }
        Cmd::MigrateToSqlite { from, to } => {
            let from = from.unwrap_or_else(|| config.database_url.clone());
            let to = to.unwrap_or_else(|| config.config_dir.join("photovault.db"));
            anyhow::ensure!(
                !to.exists(),
                "{} already exists; move it away or pick another --to",
                to.display()
            );
            let to_url = format!("sqlite://{}?mode=rwc", to.display());
            let tables = photovault_server::migrate_db::copy(&from, &to_url).await?;
            let rows: u64 = tables.iter().map(|t| t.rows).sum();
            for t in tables.iter().filter(|t| t.rows > 0) {
                println!("{}\t{}", t.table, t.rows);
            }
            println!(
                "copied {rows} rows in {} tables to {}",
                tables.len(),
                to.display()
            );
            let mut kc = config.clone();
            kc.config_dir = to
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| ".".into());
            if photovault_server::keyfile::ensure(&mut kc).is_ok()
                && photovault_server::keyfile::save_if_missing(&kc)?
            {
                println!(
                    "saved the current keys to {}",
                    photovault_server::keyfile::path(&kc).display()
                );
            }
            println!(
                "next: remove PHOTOVAULT_DATABASE_URL (SQLite in the config folder is the default) and restart"
            );
        }
        Cmd::GenKeys => unreachable!(),
    }
    Ok(())
}

async fn serve(mut config: Config) -> anyhow::Result<()> {
    use photovault_server::{capacity, discovery, preflight};
    preflight::run_or_exit(&mut config).await;
    let listen = config.listen.clone();
    let state = match build_state(config).await {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("{e:#}").replace('\n', " ");
            preflight::fail(&preflight::Failure(format!(
                "cannot prepare the database: {msg}; fix: restore the database from a backup or report this error"
            )))
            .await
        }
    };
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            preflight::fail(&preflight::Failure(format!(
                "cannot listen on {listen}: {e}; fix: free the port or change PHOTOVAULT_LISTEN"
            )))
            .await
        }
    };
    let c = &state.config;
    let email = match (&c.smtp, c.skips_email_verification()) {
        (_, true) => "verification skipped".into(),
        (Some(s), false) => format!("smtp {}:{}", s.host, s.port),
        (None, false) => "codes written to this log".into(),
    };
    let free = capacity::free_bytes(&state).await.ok().flatten();
    tracing::info!(
        "photovault {} listening on {listen} (storage {}, free {}, email: {email}, signups {})",
        env!("CARGO_PKG_VERSION"),
        state.store.name(),
        free.map_or("unlimited".into(), |f| format!("{:.1} GB", f as f64 / 1e9)),
        if c.allow_signups { "open" } else { "closed" },
    );
    if let Some(smtp) = c.smtp.clone() {
        tokio::spawn(async move {
            if let Err(e) = photovault_server::mail::test_connection(&smtp).await {
                tracing::warn!(
                    "cannot connect to SMTP {}:{}: {e:#}; verification emails will fail",
                    smtp.host,
                    smtp.port
                );
            }
        });
    }
    let _announcer = if c.mdns {
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(8080);
        let a = discovery::Announcement {
            name: c.server_name.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            home_url: c.home_url.clone(),
            public_url: c.public_url.trim_end_matches('/').to_string(),
            port: discovery::Announcement::port_from(c.home_url.as_deref(), port),
        };
        match discovery::announce(&a) {
            Ok(ann) => {
                tracing::info!("announcing {} on the local network (mDNS)", ann.fullname());
                Some(ann)
            }
            Err(e) => {
                tracing::warn!(
                    "mDNS announcement unavailable: {e}; set PHOTOVAULT_MDNS=false to silence"
                );
                None
            }
        }
    } else {
        None
    };
    photovault_server::storage::spawn_maintenance(&state);
    photovault_server::library::spawn_background(state.clone());
    photovault_server::legacy::spawn_background(state.clone());
    axum::serve(listener, photovault_server::app(state))
        .with_graceful_shutdown(async {
            photovault_server::shutdown_signal().await;
            tracing::info!("shutdown signal received, draining connections");
        })
        .await?;
    tracing::info!("photovault stopped");
    Ok(())
}
