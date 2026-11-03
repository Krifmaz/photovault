//! System probes: CPU load, processes, power source and local time.
//!
//! These read the live system (via `sysinfo` and Linux sysfs) and are kept
//! thin; parsing helpers take paths or strings so they can be tested.

use std::collections::HashSet;
use std::path::Path;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// CPU and process information from one refresh.
#[derive(Debug, Clone, Default)]
pub struct ProcessSnapshot {
    /// Average CPU load of the whole machine, 0–100.
    pub cpu_total_percent: f32,
    /// CPU load caused by this process and its descendants (e.g. ffmpeg),
    /// as a share of the whole machine, 0–100.
    pub cpu_own_percent: f32,
    /// `(name, command line)` of every other process.
    pub processes: Vec<(String, String)>,
}

impl ProcessSnapshot {
    /// CPU load caused by other processes, 0–100.
    pub fn cpu_external_percent(&self) -> f32 {
        (self.cpu_total_percent - self.cpu_own_percent).clamp(0.0, 100.0)
    }
}

/// Keeps a `sysinfo::System` between samples (CPU usage is a delta).
pub struct ProcessProbe {
    sys: System,
    own_pid: Option<Pid>,
}

impl Default for ProcessProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessProbe {
    /// Creates the probe and takes a first reading.
    pub fn new() -> Self {
        let mut p = Self {
            sys: System::new(),
            own_pid: sysinfo::get_current_pid().ok(),
        };
        p.refresh();
        p
    }

    fn refresh(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
    }

    /// Refreshes and returns the current snapshot.
    pub fn sample(&mut self) -> ProcessSnapshot {
        self.refresh();
        let ncpu = self.sys.cpus().len().max(1) as f32;
        let procs = self.sys.processes();
        let own = self
            .own_pid
            .map(|me| descendants(procs, me))
            .unwrap_or_default();
        let mut cpu_own = 0.0f32;
        let mut processes = Vec::with_capacity(procs.len());
        for (pid, p) in procs {
            if own.contains(pid) {
                cpu_own += p.cpu_usage();
                continue;
            }
            let cmd = p
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            processes.push((p.name().to_string_lossy().into_owned(), cmd));
        }
        ProcessSnapshot {
            cpu_total_percent: self.sys.global_cpu_usage(),
            cpu_own_percent: cpu_own / ncpu,
            processes,
        }
    }
}

/// `me` plus every process whose ancestor chain contains `me`.
fn descendants(procs: &std::collections::HashMap<Pid, sysinfo::Process>, me: Pid) -> HashSet<Pid> {
    let mut out = HashSet::from([me]);
    for &pid in procs.keys() {
        let mut cur = Some(pid);
        let mut chain = Vec::new();
        for _ in 0..16 {
            let Some(c) = cur else { break };
            if out.contains(&c) {
                out.extend(chain);
                break;
            }
            chain.push(c);
            cur = procs.get(&c).and_then(|p| p.parent());
        }
    }
    out
}

/// Whether the machine runs on battery, from a `power_supply` sysfs
/// directory (`/sys/class/power_supply`). Peripheral batteries (mice,
/// headsets: `scope` = `Device`) are ignored. Any online mains/USB supply
/// means "not on battery".
pub fn on_battery_sysfs(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    let read = |p: &Path, f: &str| {
        std::fs::read_to_string(p.join(f))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut discharging = false;
    for e in rd.flatten() {
        let p = e.path();
        if read(&p, "scope").eq_ignore_ascii_case("device") {
            continue;
        }
        match read(&p, "type").as_str() {
            "Mains" | "USB" | "USB_C" | "USB_PD" if read(&p, "online") == "1" => return false,
            "Battery" if read(&p, "status").eq_ignore_ascii_case("discharging") => {
                discharging = true;
            }
            _ => {}
        }
    }
    discharging
}

/// Whether the machine runs on battery (Linux; `false` elsewhere).
pub fn on_battery() -> bool {
    if cfg!(target_os = "linux") {
        on_battery_sysfs(Path::new("/sys/class/power_supply"))
    } else {
        false
    }
}

/// Local time as minutes since midnight, if the time zone is known.
pub fn local_minute_of_day() -> Option<u16> {
    let now = jiff::Zoned::now();
    Some(now.hour() as u16 * 60 + now.minute() as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (f, v) in files {
            std::fs::write(d.join(f), format!("{v}\n")).unwrap();
        }
    }

    #[test]
    fn battery_detection() {
        let t = tempfile::tempdir().unwrap();
        assert!(!on_battery_sysfs(t.path()));
        assert!(!on_battery_sysfs(&t.path().join("missing")));

        // Desktop with a wireless mouse that is discharging: not on battery.
        supply(
            t.path(),
            "hidpp_battery_0",
            &[
                ("type", "Battery"),
                ("scope", "Device"),
                ("status", "Discharging"),
            ],
        );
        assert!(!on_battery_sysfs(t.path()));

        // Laptop unplugged.
        supply(
            t.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Discharging")],
        );
        supply(t.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        assert!(on_battery_sysfs(t.path()));

        // Plugged in again.
        supply(t.path(), "AC", &[("type", "Mains"), ("online", "1")]);
        assert!(!on_battery_sysfs(t.path()));
    }

    #[test]
    fn process_probe_runs() {
        let mut p = ProcessProbe::new();
        let s = p.sample();
        assert!((0.0..=100.0).contains(&s.cpu_external_percent()));
        assert!(!s.processes.is_empty() || cfg!(not(target_os = "linux")));
    }

    #[test]
    fn local_time_known() {
        assert!(local_minute_of_day().is_some_and(|m| m < 24 * 60));
    }
}
