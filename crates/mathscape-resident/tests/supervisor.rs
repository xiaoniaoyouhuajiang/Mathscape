//! 生命周期状态机：懒启动、握手、超时、崩溃重启与退避、连续失败降级、
//! 空闲回收（可注入时钟）、身份变化重启、开关，以及整组清理。

#![cfg(unix)]

use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use mathscape_resident::{
    AcquireError, Clock, EventSink, ManualClock, ProcessSpec, ResidentChild, ResidentEvent,
    ResidentEventKind, ResidentPolicy, ResidentState, Supervisor,
};

const ECHO: &str =
    "echo ready; while IFS= read -r l; do [ -z \"$l\" ] && exit 0; echo \"echo:$l\"; done";

fn sh(script: &str) -> ProcessSpec {
    ProcessSpec::new("/bin/sh").arg("-c").arg(script)
}

fn handshake(child: &mut ResidentChild, timeout: Duration) -> Result<(), String> {
    child
        .read_until(timeout, |line| line == "ready")
        .map_err(|error| error.to_string())
}

fn policy() -> ResidentPolicy {
    ResidentPolicy {
        startup_timeout: Duration::from_millis(400),
        request_timeout: Duration::from_millis(400),
        idle_timeout: Some(Duration::from_secs(15 * 60)),
        backoff_initial: Duration::from_millis(100),
        backoff_max: Duration::from_secs(1),
        shutdown_grace: Duration::from_millis(200),
        farewell: Some(String::new()),
        ..ResidentPolicy::default()
    }
}

struct Harness {
    clock: Arc<ManualClock>,
    events: Arc<Mutex<Vec<ResidentEvent>>>,
    supervisor: Supervisor<u32>,
}

impl Harness {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: EventSink = {
            let events = Arc::clone(&events);
            Arc::new(move |event: &ResidentEvent| events.lock().unwrap().push(event.clone()))
        };
        let supervisor =
            Supervisor::new("test", policy(), Arc::clone(&clock) as Arc<dyn Clock>, sink);
        Self {
            clock,
            events,
            supervisor,
        }
    }

    fn kinds(&self) -> Vec<ResidentEventKind> {
        self.events.lock().unwrap().iter().map(|e| e.kind).collect()
    }

    fn acquire(&mut self, key: u32, script: &str) -> Result<u32, AcquireError> {
        let script = script.to_owned();
        self.supervisor
            .acquire(&key, move || Ok(sh(&script)), handshake)
            .map(|child| child.pid())
    }

    fn roundtrip(&mut self, key: u32, text: &str) -> String {
        let child = self
            .supervisor
            .acquire(&key, || Ok(sh(ECHO)), handshake)
            .unwrap();
        child.write_line(text).unwrap();
        let line = child.read_line(Duration::from_secs(2)).unwrap();
        self.supervisor.report_success();
        line
    }
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_dead(pid: u32) -> bool {
    for _ in 0..100 {
        if !alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn lazy_spawn_ready_and_reuse() {
    let mut h = Harness::new();
    assert_eq!(h.supervisor.state(), ResidentState::Stopped);
    assert!(
        h.supervisor.pid().is_none(),
        "nothing starts before the first request"
    );
    assert_eq!(h.roundtrip(1, "a"), "echo:a");
    let pid = h.supervisor.pid().unwrap();
    assert_eq!(h.supervisor.state(), ResidentState::Ready);
    assert_eq!(h.roundtrip(1, "b"), "echo:b");
    assert_eq!(
        h.supervisor.pid(),
        Some(pid),
        "the same process serves both"
    );
    assert_eq!(
        h.kinds(),
        vec![ResidentEventKind::Spawn, ResidentEventKind::Ready]
    );
    h.supervisor.shutdown();
    assert!(wait_dead(pid));
}

#[test]
fn startup_timeout_counts_as_a_failure_and_backs_off() {
    let mut h = Harness::new();
    let error = h.acquire(1, "sleep 5").unwrap_err();
    assert!(matches!(error, AcquireError::Startup { .. }), "{error:?}");
    assert_eq!(h.supervisor.consecutive_failures(), 1);
    assert_eq!(h.supervisor.state(), ResidentState::Stopped);
    // The next acquire waits out the backoff on the injected clock, then restarts.
    let before = h.clock.elapsed();
    h.acquire(1, ECHO).unwrap();
    assert!(h.clock.elapsed() - before >= Duration::from_millis(100));
    let kinds = h.kinds();
    assert!(kinds.contains(&ResidentEventKind::StartupFailed));
    assert!(kinds.contains(&ResidentEventKind::Restart));
    assert_eq!(h.supervisor.state(), ResidentState::Ready);
}

#[test]
fn three_consecutive_failures_fall_back_and_stay_there() {
    let mut h = Harness::new();
    for attempt in 1..=3 {
        let error = h.acquire(1, "exit 3").unwrap_err();
        if attempt < 3 {
            assert!(matches!(error, AcquireError::Startup { .. }));
        } else {
            assert!(matches!(error, AcquireError::Fallback { .. }), "{error:?}");
        }
    }
    assert_eq!(h.supervisor.state(), ResidentState::Failed);
    let spawns = h
        .kinds()
        .iter()
        .filter(|kind| **kind == ResidentEventKind::Spawn)
        .count();
    assert!(matches!(
        h.acquire(1, ECHO),
        Err(AcquireError::Fallback { .. })
    ));
    assert_eq!(
        h.kinds()
            .iter()
            .filter(|kind| **kind == ResidentEventKind::Spawn)
            .count(),
        spawns,
        "no spawn after falling back"
    );
    assert!(h.kinds().contains(&ResidentEventKind::Fallback));
}

#[test]
fn request_failures_count_until_success_resets() {
    let mut h = Harness::new();
    h.acquire(1, ECHO).unwrap();
    assert!(!h.supervisor.report_failure("timeout"));
    h.acquire(1, ECHO).unwrap();
    assert!(!h.supervisor.report_failure("timeout"));
    h.acquire(1, ECHO).unwrap();
    h.supervisor.report_success();
    assert_eq!(h.supervisor.consecutive_failures(), 0);
    assert!(!h.supervisor.report_failure("timeout"));
    h.acquire(1, ECHO).unwrap();
    assert!(!h.supervisor.report_failure("timeout"));
    h.acquire(1, ECHO).unwrap();
    assert!(h.supervisor.report_failure("timeout"), "third in a row");
    assert_eq!(h.supervisor.state(), ResidentState::Failed);
}

#[test]
fn crash_is_detected_and_the_process_restarts() {
    let mut h = Harness::new();
    let first = h.acquire(1, ECHO).unwrap();
    Command::new("kill")
        .args(["-9", &first.to_string()])
        .status()
        .unwrap();
    assert!(wait_dead(first) || h.supervisor.pid().is_some());
    thread::sleep(Duration::from_millis(50));
    let second = h.acquire(1, ECHO).unwrap();
    assert_ne!(first, second);
    let kinds = h.kinds();
    assert!(kinds.contains(&ResidentEventKind::Exit));
    assert!(kinds.contains(&ResidentEventKind::Restart));
    assert_eq!(h.supervisor.consecutive_failures(), 1);
    assert_eq!(h.roundtrip(1, "x"), "echo:x");
    assert_eq!(h.supervisor.consecutive_failures(), 0);
}

#[test]
fn idle_reclaim_uses_the_injected_clock() {
    let mut h = Harness::new();
    let pid = h.acquire(1, ECHO).unwrap();
    h.supervisor.report_success();
    h.clock.advance(Duration::from_secs(14 * 60));
    assert!(!h.supervisor.reap_idle());
    assert_eq!(h.supervisor.pid(), Some(pid));
    h.clock.advance(Duration::from_secs(60));
    assert!(h.supervisor.reap_idle());
    assert_eq!(h.supervisor.state(), ResidentState::Stopped);
    assert!(h.supervisor.pid().is_none());
    assert!(wait_dead(pid));
    assert!(h.kinds().contains(&ResidentEventKind::IdleReclaim));
    // Reclaim is not a failure; the next request starts a fresh process.
    assert_eq!(h.supervisor.consecutive_failures(), 0);
    assert_eq!(h.roundtrip(1, "again"), "echo:again");
}

#[test]
fn key_change_stops_the_old_process() {
    let mut h = Harness::new();
    let old = h.acquire(1, ECHO).unwrap();
    let new = h.acquire(2, ECHO).unwrap();
    assert_ne!(old, new);
    assert!(wait_dead(old));
    assert!(h.kinds().contains(&ResidentEventKind::KeyChanged));
    assert_eq!(h.supervisor.key(), Some(&2));
}

#[test]
fn disabling_stops_the_process_and_reenabling_clears_fallback() {
    let mut h = Harness::new();
    let pid = h.acquire(1, ECHO).unwrap();
    h.supervisor.set_enabled(false);
    assert!(wait_dead(pid));
    assert_eq!(h.acquire(1, ECHO), Err(AcquireError::Disabled));
    h.supervisor.set_enabled(true);
    for _ in 0..3 {
        let _ = h.acquire(1, "exit 1");
    }
    assert_eq!(h.supervisor.state(), ResidentState::Failed);
    h.supervisor.set_enabled(false);
    h.supervisor.set_enabled(true);
    assert_eq!(h.supervisor.state(), ResidentState::Stopped);
    h.acquire(1, ECHO).unwrap();
}

#[test]
fn shutdown_and_drop_kill_the_whole_process_group() {
    let dir = std::env::temp_dir().join(format!("mathscape-resident-pgrp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pids: PathBuf = dir.join("pids");
    let _ = std::fs::write(&pids, "");
    // A grandchild that ignores TERM and would hold the pipes forever.
    let script = format!(
        "sh -c 'trap \"\" TERM; exec sleep 30' </dev/null & echo $! >> '{p}'; \
         sleep 30 </dev/null & echo $! >> '{p}'; {ECHO}",
        p = pids.display()
    );
    let grandchildren = |path: &PathBuf| -> Vec<u32> {
        std::fs::read_to_string(path)
            .unwrap()
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect()
    };
    {
        let mut h = Harness::new();
        let leader = h.acquire(1, &script).unwrap();
        let spawned = grandchildren(&pids);
        assert_eq!(spawned.len(), 2);
        assert!(spawned.iter().all(|pid| alive(*pid)));
        h.supervisor.shutdown();
        assert!(wait_dead(leader));
        for pid in spawned {
            assert!(wait_dead(pid), "grandchild {pid} outlived shutdown");
        }
    }
    std::fs::write(&pids, "").unwrap();
    let spawned = {
        let mut h = Harness::new();
        h.acquire(1, &script).unwrap();
        grandchildren(&pids)
        // dropped here without an explicit shutdown
    };
    for pid in spawned {
        assert!(wait_dead(pid), "grandchild {pid} outlived drop");
    }
}
