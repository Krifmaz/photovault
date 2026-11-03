//! Pure Run/Pause decision logic with hysteresis.
//!
//! [`Decider`] turns a stream of [`Sample`]s (what the probes measured) into
//! [`Decision`]s. It has no I/O and takes time as an input, so every rule is
//! unit-testable.
//!
//! Rules:
//!
//! - **Immediate pause**: a known game or launcher is running, the machine is
//!   on battery, or the local time is inside quiet hours.
//! - **Sustained load pause**: CPU load from *other* processes (the node's own
//!   process and its children such as ffmpeg are subtracted) or GPU
//!   utilisation stays above the pause threshold for `sustain`.
//! - **Resume** only after every condition has been clear, and load has been
//!   below the (lower) resume thresholds, continuously for `resume_after`.
//!   The gap between pause and resume thresholds plus the cool-down are the
//!   hysteresis that stops the node from flapping.
//!
//! GPU utilisation cannot be attributed to processes portably, so while the
//! node itself is running GPU work the GPU threshold is not used to *pause*
//! (it would pause on its own load). Games are still caught by the process
//! list, and once paused the GPU threshold does gate resuming.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One measurement of the machine, as produced by the probes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    /// Monotonic time of the sample (any fixed origin).
    pub at: Duration,
    /// Average CPU load across all cores caused by other processes, 0–100.
    pub cpu_external_percent: f32,
    /// Highest GPU utilisation across GPUs, 0–100, if it could be read.
    pub gpu_percent: Option<f32>,
    /// Whether the node itself is currently running GPU work.
    pub own_gpu_active: bool,
    /// A running process that matched the game/launcher list.
    pub game: Option<String>,
    /// The machine runs on battery power.
    pub on_battery: bool,
    /// Local wall-clock time as minutes since midnight, if known.
    pub minute_of_day: Option<u16>,
}

/// A daily window, in local time, during which the node does not work.
///
/// `start == end` means the whole day. A window may wrap midnight
/// (for example 22:00–07:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// Start of the window, minutes since midnight (inclusive).
    pub start: u16,
    /// End of the window, minutes since midnight (exclusive).
    pub end: u16,
}

impl QuietHours {
    /// Parses `"HH:MM-HH:MM"`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (a, b) = s
            .split_once('-')
            .ok_or_else(|| format!("quiet hours {s:?}: expected HH:MM-HH:MM"))?;
        Ok(Self {
            start: parse_hhmm(a.trim())?,
            end: parse_hhmm(b.trim())?,
        })
    }

    /// Whether `minute` (since midnight) is inside the window.
    pub fn contains(&self, minute: u16) -> bool {
        if self.start == self.end {
            true
        } else if self.start < self.end {
            (self.start..self.end).contains(&minute)
        } else {
            minute >= self.start || minute < self.end
        }
    }
}

impl std::fmt::Display for QuietHours {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02}:{:02}-{:02}:{:02}",
            self.start / 60,
            self.start % 60,
            self.end / 60,
            self.end % 60
        )
    }
}

/// Parses `"HH:MM"` into minutes since midnight.
fn parse_hhmm(s: &str) -> Result<u16, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("time {s:?}: expected HH:MM"))?;
    let h: u16 = h.parse().map_err(|_| format!("time {s:?}: bad hour"))?;
    let m: u16 = m.parse().map_err(|_| format!("time {s:?}: bad minute"))?;
    if h > 23 || m > 59 {
        return Err(format!("time {s:?}: out of range"));
    }
    Ok(h * 60 + m)
}

/// Thresholds and timings for the [`Decider`].
#[derive(Debug, Clone, PartialEq)]
pub struct DecideParams {
    /// Pause when external CPU load stays above this (percent).
    pub cpu_pause_percent: f32,
    /// Resume only when external CPU load is below this (percent).
    pub cpu_resume_percent: f32,
    /// Pause when GPU utilisation stays above this (percent).
    pub gpu_pause_percent: f32,
    /// Resume only when GPU utilisation is below this (percent).
    pub gpu_resume_percent: f32,
    /// How long load must stay high before pausing.
    pub sustain: Duration,
    /// How long everything must stay clear before resuming.
    pub resume_after: Duration,
    /// Pause while on battery.
    pub pause_on_battery: bool,
    /// Pause while a game or launcher is running.
    pub pause_for_games: bool,
    /// Optional quiet hours.
    pub quiet_hours: Option<QuietHours>,
}

impl Default for DecideParams {
    fn default() -> Self {
        Self {
            cpu_pause_percent: 60.0,
            cpu_resume_percent: 35.0,
            gpu_pause_percent: 40.0,
            gpu_resume_percent: 15.0,
            sustain: Duration::from_secs(20),
            resume_after: Duration::from_secs(120),
            pause_on_battery: true,
            pause_for_games: true,
            quiet_hours: None,
        }
    }
}

/// Why the node is paused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    /// A game or launcher process is running.
    Game {
        /// The matching process.
        process: String,
    },
    /// Other processes keep the CPU busy.
    CpuBusy {
        /// External CPU load when the decision was made.
        percent: f32,
    },
    /// The GPU is busy.
    GpuBusy {
        /// GPU utilisation when the decision was made.
        percent: f32,
    },
    /// Running on battery power.
    OnBattery,
    /// Inside the configured quiet hours.
    QuietHours,
    /// The cause is gone; waiting for the cool-down before resuming.
    CoolingDown {
        /// Seconds until the node resumes if nothing changes.
        remaining_secs: u64,
    },
    /// Paused by the operator or a test.
    Manual,
}

impl std::fmt::Display for PauseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Game { process } => write!(f, "game or launcher running ({process})"),
            Self::CpuBusy { percent } => write!(f, "CPU busy ({percent:.0}%)"),
            Self::GpuBusy { percent } => write!(f, "GPU busy ({percent:.0}%)"),
            Self::OnBattery => write!(f, "on battery"),
            Self::QuietHours => write!(f, "quiet hours"),
            Self::CoolingDown { remaining_secs } => {
                write!(f, "cooling down, resuming in {remaining_secs}s")
            }
            Self::Manual => write!(f, "paused manually"),
        }
    }
}

/// Whether the node may work right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Decision {
    /// Work may proceed.
    Run,
    /// Work must wait.
    Pause {
        /// Why.
        reason: PauseReason,
    },
}

impl Decision {
    /// True for [`Decision::Run`].
    pub fn is_run(&self) -> bool {
        matches!(self, Self::Run)
    }
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Run => write!(f, "running"),
            Self::Pause { reason } => write!(f, "paused: {reason}"),
        }
    }
}

/// Stateful decision maker. Feed it samples in time order.
#[derive(Debug, Clone)]
pub struct Decider {
    params: DecideParams,
    paused: bool,
    /// Since when CPU load has been above the pause threshold.
    cpu_high_since: Option<Duration>,
    /// Since when GPU load has been above the pause threshold.
    gpu_high_since: Option<Duration>,
    /// Since when every resume condition has held (only while paused).
    clear_since: Option<Duration>,
}

impl Decider {
    /// Creates a decider that starts in the running state.
    pub fn new(params: DecideParams) -> Self {
        Self {
            params,
            paused: false,
            cpu_high_since: None,
            gpu_high_since: None,
            clear_since: None,
        }
    }

    /// The parameters in use.
    pub fn params(&self) -> &DecideParams {
        &self.params
    }

    /// Feeds one sample and returns the decision for it.
    pub fn update(&mut self, s: &Sample) -> Decision {
        let p = &self.params;

        // Track sustained high load.
        track(
            &mut self.cpu_high_since,
            s.cpu_external_percent > p.cpu_pause_percent,
            s.at,
        );
        let gpu_high = s
            .gpu_percent
            .is_some_and(|g| g > p.gpu_pause_percent && !s.own_gpu_active);
        track(&mut self.gpu_high_since, gpu_high, s.at);

        if let Some(reason) = self.immediate_reason(s) {
            return self.pause(reason);
        }
        if let Some(since) = self.cpu_high_since
            && s.at.saturating_sub(since) >= p.sustain
        {
            return self.pause(PauseReason::CpuBusy {
                percent: s.cpu_external_percent,
            });
        }
        if let Some(since) = self.gpu_high_since
            && s.at.saturating_sub(since) >= p.sustain
        {
            return self.pause(PauseReason::GpuBusy {
                percent: s.gpu_percent.unwrap_or_default(),
            });
        }

        if !self.paused {
            return Decision::Run;
        }

        // Paused: resume only once below the resume thresholds for long enough.
        if s.cpu_external_percent >= p.cpu_resume_percent {
            self.clear_since = None;
            return Decision::Pause {
                reason: PauseReason::CpuBusy {
                    percent: s.cpu_external_percent,
                },
            };
        }
        if let Some(g) = s.gpu_percent
            && g >= p.gpu_resume_percent
            && !s.own_gpu_active
        {
            self.clear_since = None;
            return Decision::Pause {
                reason: PauseReason::GpuBusy { percent: g },
            };
        }
        let since = *self.clear_since.get_or_insert(s.at);
        let clear_for = s.at.saturating_sub(since);
        if clear_for >= p.resume_after {
            self.paused = false;
            self.clear_since = None;
            Decision::Run
        } else {
            Decision::Pause {
                reason: PauseReason::CoolingDown {
                    remaining_secs: (p.resume_after - clear_for).as_secs_f64().ceil() as u64,
                },
            }
        }
    }

    /// Conditions that pause at once, without a sustain period.
    fn immediate_reason(&self, s: &Sample) -> Option<PauseReason> {
        let p = &self.params;
        if p.pause_for_games
            && let Some(g) = &s.game
        {
            return Some(PauseReason::Game { process: g.clone() });
        }
        if p.pause_on_battery && s.on_battery {
            return Some(PauseReason::OnBattery);
        }
        if let (Some(q), Some(m)) = (p.quiet_hours, s.minute_of_day)
            && q.contains(m)
        {
            return Some(PauseReason::QuietHours);
        }
        None
    }

    fn pause(&mut self, reason: PauseReason) -> Decision {
        self.paused = true;
        self.clear_since = None;
        Decision::Pause { reason }
    }
}

/// Sets `since` to `now` when `high` starts, clears it when `high` ends.
fn track(since: &mut Option<Duration>, high: bool, now: Duration) {
    match (high, since.is_some()) {
        (true, false) => *since = Some(now),
        (false, true) => *since = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn sample(at: u64, cpu: f32) -> Sample {
        Sample {
            at: secs(at),
            cpu_external_percent: cpu,
            ..Default::default()
        }
    }

    fn params() -> DecideParams {
        DecideParams {
            sustain: secs(10),
            resume_after: secs(30),
            ..Default::default()
        }
    }

    #[test]
    fn idle_machine_runs() {
        let mut d = Decider::new(params());
        for t in 0..100 {
            assert_eq!(d.update(&sample(t, 5.0)), Decision::Run);
        }
    }

    #[test]
    fn short_cpu_spike_does_not_pause() {
        let mut d = Decider::new(params());
        assert!(d.update(&sample(0, 90.0)).is_run());
        assert!(d.update(&sample(5, 90.0)).is_run());
        assert!(d.update(&sample(9, 90.0)).is_run());
        assert!(d.update(&sample(10, 10.0)).is_run());
        // The spike timer restarted.
        assert!(d.update(&sample(11, 90.0)).is_run());
        assert!(d.update(&sample(20, 90.0)).is_run());
    }

    #[test]
    fn sustained_cpu_pauses_and_resumes_after_cooldown() {
        let mut d = Decider::new(params());
        d.update(&sample(0, 80.0));
        let dec = d.update(&sample(10, 80.0));
        assert!(matches!(
            dec,
            Decision::Pause {
                reason: PauseReason::CpuBusy { .. }
            }
        ));
        // Between resume and pause thresholds: still paused (hysteresis).
        assert!(matches!(
            d.update(&sample(20, 50.0)),
            Decision::Pause {
                reason: PauseReason::CpuBusy { .. }
            }
        ));
        // Below resume threshold: cooling down.
        assert_eq!(
            d.update(&sample(30, 10.0)),
            Decision::Pause {
                reason: PauseReason::CoolingDown { remaining_secs: 30 }
            }
        );
        assert!(!d.update(&sample(59, 10.0)).is_run());
        assert!(d.update(&sample(60, 10.0)).is_run());
    }

    #[test]
    fn load_during_cooldown_restarts_it() {
        let mut d = Decider::new(params());
        d.update(&sample(0, 80.0));
        d.update(&sample(10, 80.0));
        d.update(&sample(20, 10.0));
        d.update(&sample(40, 40.0)); // above resume threshold
        assert!(!d.update(&sample(55, 10.0)).is_run());
        assert!(!d.update(&sample(84, 10.0)).is_run());
        assert!(d.update(&sample(85, 10.0)).is_run());
    }

    #[test]
    fn game_pauses_immediately_and_cools_down() {
        let mut d = Decider::new(params());
        let mut s = sample(0, 5.0);
        s.game = Some("Cyberpunk2077.exe".into());
        assert_eq!(
            d.update(&s),
            Decision::Pause {
                reason: PauseReason::Game {
                    process: "Cyberpunk2077.exe".into()
                }
            }
        );
        assert!(!d.update(&sample(1, 5.0)).is_run());
        assert!(d.update(&sample(31, 5.0)).is_run());
    }

    #[test]
    fn games_ignored_when_disabled() {
        let mut d = Decider::new(DecideParams {
            pause_for_games: false,
            ..params()
        });
        let mut s = sample(0, 5.0);
        s.game = Some("gamescope".into());
        assert!(d.update(&s).is_run());
    }

    #[test]
    fn battery_pauses() {
        let mut d = Decider::new(params());
        let mut s = sample(0, 0.0);
        s.on_battery = true;
        assert_eq!(
            d.update(&s),
            Decision::Pause {
                reason: PauseReason::OnBattery
            }
        );
        let mut d = Decider::new(DecideParams {
            pause_on_battery: false,
            ..params()
        });
        assert!(d.update(&s).is_run());
    }

    #[test]
    fn own_gpu_work_does_not_pause_but_gates_resume() {
        let mut d = Decider::new(params());
        let mut s = sample(0, 0.0);
        s.gpu_percent = Some(99.0);
        s.own_gpu_active = true;
        assert!(d.update(&s).is_run());
        s.at = secs(60);
        assert!(d.update(&s).is_run());

        // Someone else uses the GPU while the node is idle.
        let mut s = sample(100, 0.0);
        s.gpu_percent = Some(90.0);
        d.update(&s);
        s.at = secs(110);
        assert!(matches!(
            d.update(&s),
            Decision::Pause {
                reason: PauseReason::GpuBusy { .. }
            }
        ));
        s.at = secs(120);
        s.gpu_percent = Some(20.0); // above resume threshold
        assert!(!d.update(&s).is_run());
        s.gpu_percent = Some(5.0);
        s.at = secs(130);
        assert!(!d.update(&s).is_run());
        s.at = secs(160);
        assert!(d.update(&s).is_run());
    }

    #[test]
    fn quiet_hours() {
        let q = QuietHours::parse("22:00-07:30").unwrap();
        assert_eq!(q.to_string(), "22:00-07:30");
        assert!(q.contains(22 * 60));
        assert!(q.contains(3 * 60));
        assert!(!q.contains(7 * 60 + 30));
        assert!(!q.contains(12 * 60));
        let day = QuietHours::parse("09:00-17:00").unwrap();
        assert!(day.contains(9 * 60));
        assert!(!day.contains(17 * 60));
        assert!(QuietHours::parse("00:00-00:00").unwrap().contains(500));
        assert!(QuietHours::parse("25:00-01:00").is_err());
        assert!(QuietHours::parse("nope").is_err());

        let mut d = Decider::new(DecideParams {
            quiet_hours: Some(q),
            ..params()
        });
        let mut s = sample(0, 0.0);
        s.minute_of_day = Some(23 * 60);
        assert_eq!(
            d.update(&s),
            Decision::Pause {
                reason: PauseReason::QuietHours
            }
        );
        s.minute_of_day = Some(8 * 60);
        s.at = secs(1);
        assert!(!d.update(&s).is_run());
        s.at = secs(31);
        assert!(d.update(&s).is_run());
    }

    #[test]
    fn decision_serialises() {
        let d = Decision::Pause {
            reason: PauseReason::Game {
                process: "x".into(),
            },
        };
        let j = serde_json::to_string(&d).unwrap();
        assert_eq!(
            j,
            r#"{"state":"pause","reason":{"kind":"game","process":"x"}}"#
        );
        assert_eq!(serde_json::from_str::<Decision>(&j).unwrap(), d);
        assert_eq!(d.to_string(), "paused: game or launcher running (x)");
    }
}
