//! TODO(scrub).

use crate::state::AppState;

/// Start the scrubber.
pub fn spawn(state: AppState) {
    let _ = state;
}

/// `photovault admin durability scrub`.
pub async fn cli_scrub(
    state: &AppState,
    key: Option<&str>,
    limit: Option<i64>,
) -> anyhow::Result<serde_json::Value> {
    let _ = (state, key, limit);
    anyhow::bail!("not implemented yet")
}
