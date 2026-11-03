//! `photovault-import`: import an Immich library or an iCloud Photos export
//! into Photovault (or any Ente-compatible server).
//!
//! Everything is encrypted on this machine before it is uploaded.
//! Based on Ente (ente.com).

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use photovault_import::ente::{Credentials, PeopleMode, UploadOptions, Uploader};
use photovault_import::icloud::{ICloudOptions, ICloudSource};
use photovault_import::immich::{ImmichOptions, ImmichSource, NaiveTimes};
use photovault_import::item::Source;
use photovault_import::state::Journal;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// Import photos into Photovault, encrypted on this machine.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    target: Target,
    #[command(subcommand)]
    source: SourceCmd,
}

/// Where to import to.
#[derive(Debug, Args)]
struct Target {
    /// Photovault (or Ente) API endpoint, e.g. https://photos.example.com.
    #[arg(long, env = "PHOTOVAULT_ENDPOINT", global = true)]
    endpoint: Option<url::Url>,
    /// Account email.
    #[arg(long, env = "PHOTOVAULT_EMAIL", global = true)]
    email: Option<String>,
    /// Read the password from this environment variable instead of prompting.
    #[arg(long, default_value = "PHOTOVAULT_PASSWORD", global = true)]
    password_env: String,
    /// Resume journal (JSON lines). Items listed here are skipped.
    #[arg(long, default_value = "photovault-import.jsonl", global = true)]
    journal: PathBuf,
    /// Album for items that are in no album in the source.
    #[arg(long, global = true)]
    default_album: Option<String>,
    /// How to import people names.
    #[arg(long, value_enum, default_value_t = PeopleMode::Albums, global = true)]
    people: PeopleMode,
    /// Prefix for person albums, e.g. "Person: ".
    #[arg(long, default_value = "", global = true)]
    people_prefix: String,
    /// Prepare and encrypt everything but upload nothing.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Log as JSON lines instead of text.
    #[arg(long, global = true)]
    json_logs: bool,
}

#[derive(Debug, Subcommand)]
enum SourceCmd {
    /// Import from an Immich server using an API key.
    Immich(ImmichArgs),
    /// Import an iCloud Photos export from privacy.apple.com (zips or folders).
    Icloud(ICloudArgs),
}

#[derive(Debug, Args)]
struct ImmichArgs {
    /// Immich address, e.g. https://immich.example.com (or .../api).
    #[arg(long, env = "IMMICH_URL")]
    url: String,
    /// Immich API key. Prefer the environment variable over the flag.
    #[arg(long, env = "IMMICH_API_KEY", hide_env_values = true)]
    api_key: String,
    /// Also import the Locked Folder (into the hidden album).
    #[arg(long)]
    include_locked: bool,
    /// Generate thumbnails locally instead of using Immich's previews.
    #[arg(long)]
    no_immich_thumbnails: bool,
    /// Assets per search request (1-1000).
    #[arg(long, default_value_t = 250)]
    page_size: u32,
    /// How to read capture times that have no time zone.
    #[arg(long, value_enum, default_value_t = NaiveArg::AsImmich)]
    naive_times: NaiveArg,
    /// Where originals are downloaded while importing.
    #[arg(long)]
    temp_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum NaiveArg {
    /// Trust Immich's interpretation (its server zone, usually UTC).
    AsImmich,
    /// Use this computer's local time zone, as the Ente apps do.
    Local,
}

#[derive(Debug, Args)]
struct ICloudArgs {
    /// "iCloud Photos Part N of M.zip" files, extracted part folders, or a
    /// folder that holds them.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Import Live Photo stills and videos as separate files.
    #[arg(long)]
    no_live_photos: bool,
    /// Skip shared albums.
    #[arg(long)]
    no_shared_albums: bool,
    /// Turn each memory into an album "Memory: <name>".
    #[arg(long)]
    memories_as_albums: bool,
    /// Also import edited copies (IMG_E1234.*) next to the originals.
    #[arg(long)]
    include_edited: bool,
    /// Where zip entries are extracted while importing.
    #[arg(long)]
    temp_dir: Option<PathBuf>,
}

/// Prompt on the terminal and read one trimmed line.
fn prompt_line(label: &str) -> anyhow::Result<String> {
    let mut out = std::io::stderr();
    write!(out, "{label}: ")?;
    out.flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn init_logs(json: bool) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let b = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    if json {
        b.json().init();
    } else {
        b.init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logs(cli.target.json_logs);
    photovault_import::tls::install_provider();

    let (mut source, default_album): (Box<dyn Source>, &str) = match &cli.source {
        SourceCmd::Immich(a) => {
            let opts = ImmichOptions {
                include_locked: a.include_locked,
                thumbnails: !a.no_immich_thumbnails,
                page_size: a.page_size,
                temp_dir: a.temp_dir.clone(),
                naive_times: match a.naive_times {
                    NaiveArg::AsImmich => NaiveTimes::AsImmich,
                    NaiveArg::Local => NaiveTimes::Local,
                },
                ..ImmichOptions::default()
            };
            (
                Box::new(ImmichSource::new(&a.url, a.api_key.clone(), opts)?),
                "Imported from Immich",
            )
        }
        SourceCmd::Icloud(a) => {
            let opts = ICloudOptions {
                live_photos: !a.no_live_photos,
                shared_albums: !a.no_shared_albums,
                memories_as_albums: a.memories_as_albums,
                include_edited: a.include_edited,
                temp_dir: a.temp_dir.clone(),
            };
            (
                Box::new(ICloudSource::open(&a.inputs, opts)?),
                "Imported from iCloud",
            )
        }
    };

    let t = &cli.target;
    let endpoint = t
        .endpoint
        .clone()
        .context("--endpoint (or PHOTOVAULT_ENDPOINT) is required")?;
    let email = match &t.email {
        Some(e) => e.clone(),
        None => prompt_line("Email")?,
    };
    let password = match std::env::var(&t.password_env) {
        Ok(p) if !p.is_empty() => p,
        _ => rpassword::prompt_password("Password: ")?,
    };
    let opts = UploadOptions {
        default_album: t
            .default_album
            .clone()
            .unwrap_or_else(|| default_album.to_string()),
        people: t.people,
        people_album_prefix: t.people_prefix.clone(),
        dry_run: t.dry_run,
    };
    let creds = Credentials {
        endpoint,
        email,
        password,
    };
    let totp = || prompt_line("Two-factor code");
    let otp = || prompt_line("Code from the email we just sent");
    let mut uploader = Uploader::connect(creds, opts, &totp, &otp).await?;

    let mut journal = Journal::open(&t.journal, source.name())?;
    let summary = photovault_import::run::run(source.as_mut(), &mut uploader, &mut journal).await?;
    eprintln!(
        "Done: {} uploaded, {} already present, {} skipped (journal), {} failed{}",
        summary.uploaded,
        summary.duplicates,
        summary.skipped,
        summary.failed,
        if t.dry_run {
            format!(", {} prepared (dry run)", summary.dry_run)
        } else {
            String::new()
        }
    );
    if summary.failed > 0 {
        std::process::exit(2);
    }
    Ok(())
}
