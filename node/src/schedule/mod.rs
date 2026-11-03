//! Pausing while the machine is busy (gaming, high CPU/GPU load, on battery,
//! quiet hours).
//!
//! The pieces:
//!
//! - [`decide`]: the pure decision logic with hysteresis ([`Decider`]).
//! - [`games`]: matching running processes against the game/launcher list.
//! - [`probe`]: CPU, process, power and clock probes.
//! - [`gpu`]: GPU detection, utilisation and inference-provider selection.
//! - [`BusyMonitor`]: samples the probes on a timer and publishes the current
//!   [`Decision`] on a `tokio::sync::watch` channel that the worker follows.
//!
//! Linux gets every probe. On Windows and macOS the CPU and process checks
//! work through `sysinfo`, NVIDIA utilisation through `nvidia-smi`, and the
//! rest (battery, AMD/Intel utilisation) reads as "unknown" and never pauses.

pub mod decide;
pub mod games;
pub mod gpu;
pub mod probe;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub use decide::{DecideParams, Decider, Decision, PauseReason, QuietHours, Sample};

use crate::config::BusyConfig;
use games::GameMatcher;
use gpu::GpuReport;
use probe::ProcessProbe;

/// Number of GPU jobs the node is running right now. The worker increments it
/// around GPU inference so the monitor does not pause on the node's own load.
pub type GpuJobs = Arc<AtomicUsize>;

/// Samples the machine and decides whether the node may work.
pub struct BusyMonitor {
    decider: Decider,
    games: GameMatcher,
    procs: ProcessProbe,
    gpu: GpuReport,
    gpu_jobs: GpuJobs,
    interval: Duration,
    origin: Instant,
}

impl BusyMonitor {
    /// Creates a monitor from the `[busy]` config, the detected GPUs and the
    /// worker's GPU job counter.
    pub fn new(cfg: &BusyConfig, gpu: GpuReport, gpu_jobs: GpuJobs) -> Result<Self> {
        Ok(Self {
            decider: Decider::new(cfg.decide_params()?),
            games: GameMatcher::new(&cfg.games, &cfg.ignore_processes),
            procs: ProcessProbe::new(),
            gpu,
            gpu_jobs,
            interval: Duration::from_secs(cfg.sample_secs.max(1)),
            origin: Instant::now(),
        })
    }

    /// Takes one measurement of the machine.
    pub async fn sample(&mut self) -> Sample {
        let snap = self.procs.sample();
        let game = self
            .games
            .first_match(snap.processes.iter().map(|(n, c)| (n.as_str(), c.as_str())));
        Sample {
            at: self.origin.elapsed(),
            cpu_external_percent: snap.cpu_external_percent(),
            gpu_percent: gpu::utilisation(&self.gpu).await,
            own_gpu_active: self.gpu_jobs.load(Ordering::Relaxed) > 0,
            game,
            on_battery: probe::on_battery(),
            minute_of_day: probe::local_minute_of_day(),
        }
    }

    /// Samples once and returns the decision.
    pub async fn check(&mut self) -> Decision {
        let s = self.sample().await;
        let d = self.decider.update(&s);
        tracing::trace!(?s, %d, "busy sample");
        d
    }

    /// Runs the sampling loop in a task until `shutdown`, publishing every
    /// change of decision. The first decision is taken before returning.
    pub async fn spawn(mut self, shutdown: CancellationToken) -> watch::Receiver<Decision> {
        let first = self.check().await;
        log_decision(None, &first);
        let (tx, rx) = watch::channel(first);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(self.interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tick.tick() => {}
                }
                let d = self.check().await;
                tx.send_if_modified(|cur| {
                    if same_state(cur, &d)
                        && !matches!(
                            d,
                            Decision::Pause {
                                reason: PauseReason::CoolingDown { .. }
                            }
                        )
                    {
                        return false;
                    }
                    if *cur != d {
                        log_decision(Some(cur), &d);
                        *cur = d;
                        true
                    } else {
                        false
                    }
                });
                if tx.is_closed() {
                    break;
                }
            }
        });
        rx
    }
}

/// Both run, or both pause for the same kind of reason.
fn same_state(a: &Decision, b: &Decision) -> bool {
    match (a, b) {
        (Decision::Run, Decision::Run) => true,
        (Decision::Pause { reason: x }, Decision::Pause { reason: y }) => {
            std::mem::discriminant(x) == std::mem::discriminant(y)
        }
        _ => false,
    }
}

fn log_decision(prev: Option<&Decision>, d: &Decision) {
    let was_run = prev.is_none_or(Decision::is_run);
    match d {
        Decision::Run if !was_run => tracing::info!("resuming work"),
        Decision::Run => {}
        Decision::Pause {
            reason: PauseReason::CoolingDown { .. },
        } => tracing::debug!(%d, "busy monitor"),
        Decision::Pause { .. } => tracing::info!(%d, "pausing work"),
    }
}

/// A channel that always says [`Decision::Run`] (busy monitoring disabled
/// or `--no-pause`). Keep the sender alive for as long as the receiver is
/// used.
pub fn always_run() -> (watch::Sender<Decision>, watch::Receiver<Decision>) {
    watch::channel(Decision::Run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_state_compares_reason_kind() {
        let cpu = |p| Decision::Pause {
            reason: PauseReason::CpuBusy { percent: p },
        };
        assert!(same_state(&cpu(70.0), &cpu(80.0)));
        assert!(!same_state(&cpu(70.0), &Decision::Run));
        assert!(same_state(&Decision::Run, &Decision::Run));
    }

    #[tokio::test]
    async fn monitor_samples() {
        let cfg = BusyConfig {
            games: vec![],
            ..Default::default()
        };
        let mut m = BusyMonitor::new(&cfg, GpuReport::default(), GpuJobs::default()).unwrap();
        let s = m.sample().await;
        assert!(s.game.is_none());
        assert_eq!(s.gpu_percent, None);
        let _ = m.check().await;
    }
}
