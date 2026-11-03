//! TODO(dbbackup): encrypted database snapshots on every target.

use crate::state::AppState;
use std::path::Path;

/// Start the snapshot loop.
pub fn spawn(state: AppState) {
    let _ = state;
}

/// A new random backup key, base64.
pub fn gen_key() -> String {
    crate::crypto::b64(&crate::crypto::random_bytes(32))
}

/// `backup-db`.
pub async fn cli_backup(state: &AppState) -> anyhow::Result<()> {
    let _ = state;
    anyhow::bail!("not implemented yet")
}

/// `list-db-backups`.
pub async fn cli_list(state: &AppState, target: &str) -> anyhow::Result<()> {
    let _ = (state, target);
    anyhow::bail!("not implemented yet")
}

/// `restore-db`.
pub async fn cli_restore(
    state: &AppState,
    target: &str,
    snapshot: Option<&str>,
    out: &Path,
) -> anyhow::Result<()> {
    let _ = (state, target, snapshot, out);
    anyhow::bail!("not implemented yet")
}

/// `load-db-dump`.
pub async fn cli_load_dump(state: &AppState, file: &Path) -> anyhow::Result<()> {
    let _ = (state, file);
    anyhow::bail!("not implemented yet")
}
