//! TODO(evacuate): move objects off a mergerfs branch.

use crate::state::AppState;

/// Running evacuations.
#[derive(Debug, Default)]
pub struct Evacuations {}

impl Evacuations {
    /// Progress of every evacuation this process started, as JSON.
    pub fn snapshot_json(&self) -> serde_json::Value {
        serde_json::Value::Array(vec![])
    }
}

/// Start moving every object off the branch FSView calls `host_path`, in
/// the background. Errors when the branch is unknown or one is already running.
pub async fn start(state: &AppState, host_path: &str) -> anyhow::Result<()> {
    let _ = (state, host_path);
    anyhow::bail!("not implemented yet")
}

/// `photovault admin durability evacuate BRANCH` (runs in the foreground).
pub async fn cli_evacuate(state: &AppState, host_path: &str) -> anyhow::Result<()> {
    let _ = (state, host_path);
    anyhow::bail!("not implemented yet")
}
