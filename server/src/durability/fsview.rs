//! TODO(fsview): implemented in a follow-up commit.

use super::config::FsviewConfig;
use crate::state::AppState;

/// FSView client and latest pool health.
#[derive(Debug)]
pub struct FsviewState {
    /// Configuration.
    pub config: FsviewConfig,
}

impl FsviewState {
    /// Build from config (does not contact FSView).
    pub fn new(config: &FsviewConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.clone(),
        })
    }
}

/// Start polling FSView.
pub fn spawn(state: AppState) {
    let _ = state;
}

impl FsviewState {
    /// Latest pool health and placement decisions as JSON for the status
    /// page and admin API (`null` before the first poll).
    pub fn snapshot_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

/// `photovault admin durability pool`.
pub async fn cli_pool(state: &AppState) -> anyhow::Result<()> {
    let _ = state;
    anyhow::bail!("not implemented yet")
}
