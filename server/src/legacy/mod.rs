//! Account recovery features ("Legacy"): trusted contacts, legacy kits, and the
//! SRP/key-attribute reset they perform for a recovered account.

pub mod emergency;
pub mod kit;
pub mod password;
pub mod srp;
pub mod users;

use crate::state::AppState;
use std::time::Duration;

/// Spawn this workstream's background jobs:
/// * hourly (and once at start): trusted-contact recovery reminders, promoting
///   finished notice periods to READY (museum's `SendRecoveryReminder`);
/// * every 10 minutes: removal of deleted contact attachments and of expired,
///   never-committed attachment uploads from the object store.
pub fn spawn_background(state: AppState) {
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = emergency::run_reminders_once(&st).await {
                tracing::error!(error = ?e, "recovery reminder job failed");
            }
        }
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        loop {
            tick.tick().await;
            if let Err(e) = crate::routes::contacts::cleanup_attachments(&state).await {
                tracing::error!(error = ?e, "attachment cleanup job failed");
            }
        }
    });
}
