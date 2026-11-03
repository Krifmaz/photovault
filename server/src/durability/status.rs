//! TODO(status): status page, admin API and `durability status`.

use crate::state::AppState;

/// Start the status page listener (when `[durability.status] listen` is set).
pub fn spawn(state: AppState) {
    let _ = state;
}

/// `photovault admin durability status`.
pub async fn cli_status(state: &AppState, json: bool) -> anyhow::Result<()> {
    let _ = (state, json);
    anyhow::bail!("not implemented yet")
}
