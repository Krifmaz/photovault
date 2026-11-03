//! Deciding which hooks run, and running them with failures isolated.
//!
//! The embedding app calls [`Dispatcher::dispatch_upload`] after each new
//! upload and [`Dispatcher::run_due`] periodically (for example from a
//! background task). One plugin failing, timing out or even panicking the
//! host side never stops the others; each gets its own [`HookRun`] result.

use crate::library::Library;
use crate::registry::{InstalledPlugin, Registry};
use crate::runtime::{HostLimits, ProgressFn, RuntimeError};
use photovault_plugin_manifest::Schedule;
use photovault_plugin_manifest::abi::{FileId, HookOutput, Runner};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

const MICROS_PER_MINUTE: i64 = 60_000_000;

/// Whether a schedule is due at `now`, given the last successful run (all
/// µs since the epoch). A plugin that never ran is due immediately.
pub fn is_due(schedule: &Schedule, last_run: Option<i64>, now: i64) -> bool {
    match last_run {
        None => true,
        Some(t) => now.saturating_sub(t) >= i64::from(schedule.every_minutes) * MICROS_PER_MINUTE,
    }
}

/// The ids of enabled plugins with a schedule that is due at `now`.
pub fn due_plugins(plugins: &[InstalledPlugin], now: i64) -> Vec<String> {
    plugins
        .iter()
        .filter(|p| p.enabled)
        .filter(|p| {
            p.manifest
                .hooks
                .schedule
                .is_some_and(|s| is_due(&s, p.last_run, now))
        })
        .map(|p| p.manifest.plugin.id.clone())
        .collect()
}

/// The outcome of one plugin's hook.
#[derive(Debug)]
pub struct HookRun {
    /// Plugin id.
    pub plugin_id: String,
    /// What the hook returned, or why it failed.
    pub result: Result<HookOutput, RuntimeError>,
}

/// Runs hooks of every installed plugin against one library.
#[derive(Clone)]
pub struct Dispatcher {
    /// The plugin store.
    pub registry: Arc<Registry>,
    /// The user's library.
    pub library: Arc<dyn Library>,
    /// Host limits for every instance.
    pub limits: HostLimits,
    /// Where plugins run.
    pub runner: Runner,
    /// Progress callback.
    pub progress: Option<ProgressFn>,
}

impl std::fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("registry", &self.registry)
            .field("runner", &self.runner)
            .finish_non_exhaustive()
    }
}

impl Dispatcher {
    fn run_one(
        &self,
        id: &str,
        f: impl FnOnce(&mut crate::runtime::PluginInstance) -> Result<HookOutput, RuntimeError>,
    ) -> HookRun {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut instance = self
                .registry
                .instantiate(
                    id,
                    self.library.clone(),
                    self.limits.clone(),
                    self.runner,
                    self.progress.clone(),
                )
                .map_err(|e| match e {
                    crate::registry::RegistryError::Runtime(r) => r,
                    other => RuntimeError::Load(other.to_string()),
                })?;
            f(&mut instance)
        }))
        .unwrap_or_else(|_| {
            Err(RuntimeError::Failed(
                "the host panicked running this plugin".into(),
            ))
        });
        HookRun {
            plugin_id: id.to_owned(),
            result,
        }
    }

    /// Run `on_upload` of every enabled plugin that declares it.
    pub fn dispatch_upload(&self, file_id: FileId) -> Vec<HookRun> {
        self.registry
            .list()
            .into_iter()
            .filter(|p| p.enabled && p.manifest.hooks.on_upload)
            .map(|p| self.run_one(&p.manifest.plugin.id, |i| i.run_upload(file_id)))
            .collect()
    }

    /// Run `on_schedule` of every plugin that is due at `now`, recording
    /// `now` as the last run of each one that succeeds.
    pub fn run_due(&self, now: i64) -> Vec<HookRun> {
        due_plugins(&self.registry.list(), now)
            .into_iter()
            .map(|id| {
                let mut run = self.run_one(&id, |i| i.run_schedule());
                if run.result.is_ok()
                    && let Err(e) = self.registry.record_run(&id, now)
                {
                    run.result = Err(RuntimeError::Failed(format!(
                        "could not record the run: {e}"
                    )));
                }
                run
            })
            .collect()
    }

    /// Run one plugin's on-demand action.
    pub fn run_on_demand(&self, plugin_id: &str, action: &str) -> HookRun {
        self.run_one(plugin_id, |i| i.run_on_demand(action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_logic() {
        let s = Schedule { every_minutes: 60 };
        let hour = 60 * MICROS_PER_MINUTE;
        assert!(is_due(&s, None, 0));
        assert!(!is_due(&s, Some(1_000), 1_000 + hour - 1));
        assert!(is_due(&s, Some(1_000), 1_000 + hour));
        // A clock that went backwards is not due.
        assert!(!is_due(&s, Some(5 * hour), hour));
    }
}
