//! TODO(replicate): replication to targets, replica verification, retention.

use crate::state::AppState;

/// Start one replication loop per target.
pub fn spawn(state: AppState) {
    let _ = state;
}

/// `photovault admin durability replicate [--target T]`: one pass now.
pub async fn cli_replicate(state: &AppState, target: Option<&str>) -> anyhow::Result<()> {
    let _ = (state, target);
    anyhow::bail!("not implemented yet")
}
