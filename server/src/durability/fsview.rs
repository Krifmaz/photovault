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
