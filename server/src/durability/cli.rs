//! `photovault admin durability …`. Commands run against the database and
//! storage directly (the server may be running at the same time).

use crate::state::AppState;
use clap::Subcommand;

/// Durability subcommands.
#[derive(Subcommand, Debug)]
pub enum DurabilityCmd {
    /// Show catalog, replication, scrub and FSView status.
    Status {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List recent durability events (newest first).
    Events {
        /// How many.
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Run one catalog sweep now (find unrecorded and missing objects).
    Sweep,
    /// Verify objects now: all due ones, or one key.
    Scrub {
        /// Only this object key.
        #[arg(long)]
        key: Option<String>,
        /// Stop after this many objects.
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Run one replication pass now (all targets, or one).
    Replicate {
        /// Target name.
        #[arg(long)]
        target: Option<String>,
    },
    /// Generate a database backup key (base64) for `[durability.database_backup] key`.
    GenKey,
    /// Take an encrypted database snapshot now and upload it to every target.
    BackupDb,
    /// List the database snapshots on a target.
    ListDbBackups {
        /// Target name.
        target: String,
    },
    /// Download and decrypt a database snapshot to a local file.
    RestoreDb {
        /// Target name.
        target: String,
        /// Snapshot name (from `list-db-backups`); default the newest.
        #[arg(long)]
        snapshot: Option<String>,
        /// Output file (SQLite database, or a Postgres dump to load with `load-db-dump`).
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Load a decrypted Postgres dump (from `restore-db`) into the configured,
    /// freshly migrated and empty database.
    LoadDbDump {
        /// Dump file.
        file: std::path::PathBuf,
    },
    /// Show FSView pool health as Photovault sees it.
    Pool,
    /// Move every object off a mergerfs branch onto healthy branches.
    Evacuate {
        /// Branch host path as FSView reports it, e.g. `/mnt/disk3`.
        branch: String,
    },
}

/// Run a durability command.
pub async fn run(state: &AppState, cmd: DurabilityCmd) -> anyhow::Result<()> {
    match cmd {
        DurabilityCmd::Status { json } => super::status::cli_status(state, json).await,
        DurabilityCmd::Events { limit } => {
            for e in super::events::recent(&state.db, limit).await? {
                println!(
                    "{}  {:<8} {:<28} {}  {}",
                    e.at / 1_000_000,
                    e.severity.as_str(),
                    e.kind,
                    e.subject,
                    e.detail
                );
            }
            Ok(())
        }
        DurabilityCmd::Sweep => {
            let s = super::sweep::run_once(state).await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
            Ok(())
        }
        DurabilityCmd::Scrub { key, limit } => {
            let s = super::scrub::cli_scrub(state, key.as_deref(), limit).await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
            Ok(())
        }
        DurabilityCmd::Replicate { target } => {
            super::replicate::cli_replicate(state, target.as_deref()).await
        }
        DurabilityCmd::GenKey => {
            println!("{}", super::dbbackup::gen_key());
            Ok(())
        }
        DurabilityCmd::BackupDb => super::dbbackup::cli_backup(state).await,
        DurabilityCmd::ListDbBackups { target } => super::dbbackup::cli_list(state, &target).await,
        DurabilityCmd::RestoreDb {
            target,
            snapshot,
            out,
        } => super::dbbackup::cli_restore(state, &target, snapshot.as_deref(), &out).await,
        DurabilityCmd::LoadDbDump { file } => super::dbbackup::cli_load_dump(state, &file).await,
        DurabilityCmd::Pool => super::fsview::cli_pool(state).await,
        DurabilityCmd::Evacuate { branch } => super::evacuate::cli_evacuate(state, &branch).await,
    }
}
