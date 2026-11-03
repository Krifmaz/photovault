//! Durability (section 2): every stored object gets a SHA-256 checksum,
//! scheduled scrubbing re-reads and verifies objects and repairs them from a
//! replica, objects (already ciphertext) and encrypted database snapshots are
//! replicated to a second local target and off-site S3/B2 (3-2-1), a status
//! page and alerts report on all of it, and the optional FSView integration
//! keeps writes off failing mergerfs branches.
//!
//! Nothing here ever needs plaintext: objects are opaque ciphertext and are
//! copied byte for byte.
//!
//! Module map:
//! * [`catalog`]: the `durability_objects` table; write-time checksums.
//! * [`sweep`]: lists the primary store to catalog objects it did not see
//!   being written (S3 presigned uploads) and to notice missing ones.
//! * [`scrub`]: re-reads objects, verifies checksums, repairs from replicas.
//! * [`targets`], [`replicate`], [`dbbackup`]: 3-2-1 replication.
//! * [`fsview`], [`evacuate`]: FSView pool health, write placement, moving
//!   objects off a disk.
//! * [`events`], [`alerts`], [`status`], [`metrics`]: reporting.
//! * [`schedule`], [`ratelimit`], [`jobs`]: shared plumbing.

pub mod alerts;
pub mod catalog;
pub mod config;
pub mod dbbackup;
pub mod evacuate;
pub mod events;
pub mod fsview;
pub mod jobs;
pub mod metrics;
pub mod ratelimit;
pub mod replicate;
pub mod schedule;
pub mod scrub;
pub mod status;
pub mod sweep;
pub mod targets;

use crate::state::AppState;
use std::sync::Arc;

/// Live durability state shared by the background jobs, the status page and
/// the admin API ([`crate::state::Inner::durability`]).
#[derive(Debug, Default)]
pub struct Runtime {
    /// Replication targets opened from `[[durability.targets]]`.
    pub targets: Vec<Arc<targets::Target>>,
    /// Latest FSView pool health, when the integration is configured.
    pub fsview: Option<Arc<fsview::FsviewState>>,
    /// Running "move objects off this disk" jobs.
    pub evacuations: evacuate::Evacuations,
}

impl Runtime {
    /// Build from configuration (opens targets; does not start anything).
    pub fn new(config: &crate::config::Config) -> anyhow::Result<Self> {
        let targets = targets::open_all(&config.durability)?;
        let fsview = config
            .durability
            .fsview
            .as_ref()
            .map(fsview::FsviewState::new)
            .transpose()?
            .map(Arc::new);
        Ok(Self {
            targets,
            fsview,
            evacuations: Default::default(),
        })
    }

    /// The target called `name`.
    pub fn target(&self, name: &str) -> Option<Arc<targets::Target>> {
        self.targets.iter().find(|t| t.name == name).cloned()
    }
}

/// Start every durability background job for `serve`. Does nothing when
/// `[durability] enabled = false`.
pub fn spawn_background(state: &AppState) {
    if !state.config.durability.enabled {
        tracing::info!("durability jobs disabled");
        return;
    }
    sweep::spawn(state.clone());
    scrub::spawn(state.clone());
    replicate::spawn(state.clone());
    dbbackup::spawn(state.clone());
    fsview::spawn(state.clone());
    alerts::spawn(state.clone());
    status::spawn(state.clone());
}
