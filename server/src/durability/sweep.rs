//! TODO(sweep).

use crate::state::AppState;

/// Start the periodic sweep.
pub fn spawn(state: AppState) {
    let _ = state;
}

/// One sweep now; returns statistics.
pub async fn run_once(state: &AppState) -> anyhow::Result<serde_json::Value> {
    let _ = state;
    anyhow::bail!("not implemented yet")
}
