//! Busy-monitor decisions over realistic timelines, driven from the config.

use std::time::Duration;

use photovault_node::config::Config;
use photovault_node::schedule::games::GameMatcher;
use photovault_node::schedule::{Decider, Decision, PauseReason, Sample};

fn at(secs: u64) -> Sample {
    Sample {
        at: Duration::from_secs(secs),
        cpu_external_percent: 5.0,
        gpu_percent: Some(3.0),
        ..Default::default()
    }
}

/// An evening of gaming: launcher starts, game runs, game exits, launcher
/// stays open; the node only resumes after the cool-down once the launcher
/// is closed too.
#[test]
fn gaming_session() {
    let cfg = Config::default();
    let matcher = GameMatcher::new(&cfg.busy.games, &cfg.busy.ignore_processes);
    let mut d = Decider::new(cfg.busy.decide_params().unwrap());
    let resume = cfg.busy.resume_after_secs;

    let procs_idle = vec![("firefox", "/usr/bin/firefox"), ("steam", "steam")];
    let mut procs_game = procs_idle.clone();
    procs_game.push((
        "reaper",
        "/home/u/.local/share/Steam/ubuntu12_32/reaper SteamLaunch AppId=570 -- dota2",
    ));

    let mut t = 0;
    let mut step = |procs: &[(&str, &str)], gpu: f32, d: &mut Decider| {
        t += 5;
        let mut s = at(t);
        s.game = matcher.first_match(procs.iter().copied());
        s.gpu_percent = Some(gpu);
        d.update(&s)
    };

    assert!(step(&procs_idle, 2.0, &mut d).is_run());
    assert!(matches!(
        step(&procs_game, 95.0, &mut d),
        Decision::Pause {
            reason: PauseReason::Game { .. }
        }
    ));
    for _ in 0..20 {
        assert!(!step(&procs_game, 95.0, &mut d).is_run());
    }
    // Game closed, GPU settles: cool-down, then run.
    let mut runs_after = None;
    for i in 0..100 {
        if step(&procs_idle, 1.0, &mut d).is_run() {
            runs_after = Some((i + 1) * 5);
            break;
        }
    }
    let waited = runs_after.expect("resumes eventually");
    assert!(waited >= resume, "waited {waited}s < {resume}s");
    assert!(waited <= resume + 5);
}

/// The node's own ffmpeg/ML load (already subtracted from CPU, and GPU work
/// flagged as its own) never pauses it.
#[test]
fn own_load_never_pauses() {
    let mut d = Decider::new(Config::default().busy.decide_params().unwrap());
    for t in 0..200 {
        let mut s = at(t * 5);
        s.gpu_percent = Some(100.0);
        s.own_gpu_active = true;
        assert!(d.update(&s).is_run());
    }
}

/// A compile job (sustained external CPU) pauses after the sustain period.
#[test]
fn sustained_external_cpu() {
    let cfg = Config::default();
    let sustain = cfg.busy.sustain_secs;
    let mut d = Decider::new(cfg.busy.decide_params().unwrap());
    let mut paused_at = None;
    for t in (0..300).step_by(5) {
        let mut s = at(t);
        s.cpu_external_percent = 95.0;
        if !d.update(&s).is_run() {
            paused_at = Some(t);
            break;
        }
    }
    assert_eq!(paused_at, Some(sustain));
}
