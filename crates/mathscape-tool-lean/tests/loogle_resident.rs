//! Resident Loogle against `mathscape-loogle-fixture` (no Lean, no Mathlib):
//! handshake, reuse, truncation and `count`, rejections, timeouts and crashes
//! with restart, the three-failure fallback, idle reclaim, key changes, and
//! the desktop socket round trip.

#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use mathscape_resident::{
    Clock, EventSink, ManualClock, ResidentEvent, ResidentEventKind, ResidentPolicy, ResidentState,
};
use mathscape_tool_lean::{
    LOOGLE_IPC_VERSION, LeanLoogleError, LoogleIpcError, LoogleIpcRequest, LoogleIpcResponse,
    LoogleResidentHost, LoogleTarget, ResidentLoogle, ResidentLoogleOutcome,
    loogle_resident_policy, request_over_socket,
};

fn fixture_program() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mathscape-loogle-fixture"))
}

struct Env {
    dir: PathBuf,
    target: LoogleTarget,
    pids: PathBuf,
    log: PathBuf,
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Env {
    fn new(tag: &str, start: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mathscape-loogle-resident-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let project = dir.join("project");
        let cache = dir.join("cache");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(cache.join("loogle")).unwrap();
        let index = cache.join("loogle/mathlib.loogle-index");
        fs::write(&index, b"fake index").unwrap();
        let lake = dir.join("lake");
        let lake_log = dir.join("lake.log");
        write_script(
            &lake,
            &format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$1 $2 $3\" = \"env printenv LEAN_PATH\" ] && echo /fake/lean/lib && exit 0\nexit 1\n",
                lake_log.display()
            ),
        );
        let pids = dir.join("pids");
        let log = dir.join("queries");
        let loogle = dir.join("loogle");
        write_script(
            &loogle,
            &format!(
                "#!/bin/sh\nLOOGLE_FIXTURE_START='{start}' LOOGLE_FIXTURE_PIDS='{}' LOOGLE_FIXTURE_LOG='{}' exec '{}' \"$@\"\n",
                pids.display(),
                log.display(),
                fixture_program().display()
            ),
        );
        let target = LoogleTarget {
            profile_fingerprint: "sha256:profile".to_owned(),
            project_root: project,
            lake_executable: lake,
            loogle_executable: loogle,
            index_file: index,
            managed_cache_root: cache,
        };
        Self {
            dir,
            target,
            pids,
            log,
        }
    }

    fn spawned(&self) -> Vec<u32> {
        fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect()
    }

    fn lake_calls(&self) -> usize {
        fs::read_to_string(self.dir.join("lake.log"))
            .unwrap_or_default()
            .lines()
            .count()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Instance {
    clock: Arc<ManualClock>,
    events: Arc<Mutex<Vec<ResidentEvent>>>,
    loogle: ResidentLoogle,
}

fn instance(policy: ResidentPolicy) -> Instance {
    let clock = Arc::new(ManualClock::new());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let events = Arc::clone(&events);
        Arc::new(move |event: &ResidentEvent| events.lock().unwrap().push(event.clone()))
    };
    let loogle = ResidentLoogle::with_policy(policy, Arc::clone(&clock) as Arc<dyn Clock>, sink);
    Instance {
        clock,
        events,
        loogle,
    }
}

fn fast_policy() -> ResidentPolicy {
    ResidentPolicy {
        startup_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_millis(500),
        backoff_initial: Duration::from_millis(10),
        backoff_max: Duration::from_millis(50),
        shutdown_grace: Duration::from_millis(500),
        ..loogle_resident_policy()
    }
}

impl Instance {
    fn kinds(&self) -> Vec<ResidentEventKind> {
        self.events.lock().unwrap().iter().map(|e| e.kind).collect()
    }

    fn count(&self, kind: ResidentEventKind) -> usize {
        self.kinds().iter().filter(|k| **k == kind).count()
    }
}

fn done(outcome: ResidentLoogleOutcome) -> mathscape_tool_lean::ResidentLoogleResult {
    match outcome {
        ResidentLoogleOutcome::Done(done) => done,
        other => panic!("expected a resident answer, got {other:?}"),
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
    (0..100).any(|_| {
        if alive(pid) {
            thread::sleep(Duration::from_millis(20));
            false
        } else {
            true
        }
    })
}

#[test]
fn lazy_start_then_reuse_with_identical_count_semantics() {
    let env = Env::new("reuse", "");
    let mut i = instance(fast_policy());
    assert!(env.spawned().is_empty(), "no prewarm");
    let first = done(
        i.loogle
            .search(&env.target, "Nat.add_comm", "Mathlib", 50)
            .unwrap(),
    );
    assert_eq!(first.result.total, 3);
    assert_eq!(first.result.candidates.len(), 3);
    assert!(!first.result.truncated);
    assert_eq!(first.result.profile_fingerprint, "sha256:profile");
    assert!(first.index_fingerprint.starts_with("sha256:"));
    assert_eq!(first.result.backend, "loogle");

    // `many`: Loogle counts 250, the process returns 200, the request wants 7.
    let many = done(i.loogle.search(&env.target, "many", "Mathlib", 7).unwrap());
    assert_eq!(many.result.total, 250);
    assert_eq!(many.result.candidates.len(), 7);
    assert!(many.result.truncated);
    assert_eq!(many.result.candidates[0].name, "Fixture.hit0");

    let none = done(i.loogle.search(&env.target, "none", "Mathlib", 50).unwrap());
    assert_eq!(none.result.total, 0);
    assert!(none.result.candidates.is_empty());
    assert!(!none.result.truncated);

    assert_eq!(env.spawned().len(), 1, "one process served every query");
    assert_eq!(env.lake_calls(), 1, "LEAN_PATH is resolved once");
    assert_eq!(i.loogle.state(), ResidentState::Ready);
    assert_eq!(i.count(ResidentEventKind::Spawn), 1);
    assert_eq!(i.count(ResidentEventKind::Ready), 1);
}

#[test]
fn newline_queries_are_rejected_before_reaching_the_process() {
    let env = Env::new("newline", "");
    let mut i = instance(fast_policy());
    for query in ["a\nb", "a\rb"] {
        assert!(matches!(
            i.loogle.search(&env.target, query, "Mathlib", 10),
            Err(LeanLoogleError::InvalidQuery(_))
        ));
    }
    assert!(env.spawned().is_empty());
    assert!(!env.log.exists(), "nothing was written to Loogle");
}

#[test]
fn rejected_query_keeps_the_process() {
    let env = Env::new("rejected", "");
    let mut i = instance(fast_policy());
    match i.loogle.search(&env.target, "unknown", "Mathlib", 10) {
        Err(LeanLoogleError::QueryRejected { suggestions, .. }) => {
            assert_eq!(suggestions, vec!["InformationTheory.klDiv".to_owned()]);
        }
        other => panic!("unexpected {other:?}"),
    }
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    assert_eq!(env.spawned().len(), 1);
}

#[test]
fn query_timeout_kills_and_the_next_query_restarts() {
    let env = Env::new("timeout", "");
    let mut i = instance(fast_policy());
    done(i.loogle.search(&env.target, "warm", "Mathlib", 10).unwrap());
    let first = env.spawned()[0];
    match i.loogle.search(&env.target, "hang", "Mathlib", 10) {
        Err(LeanLoogleError::TimedOut { stage, .. }) => assert_eq!(stage, "query"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(wait_dead(first), "timed-out process is killed");
    let again = done(
        i.loogle
            .search(&env.target, "after", "Mathlib", 10)
            .unwrap(),
    );
    assert_eq!(again.result.total, 3);
    let spawned = env.spawned();
    assert_eq!(spawned.len(), 2);
    assert!(i.kinds().contains(&ResidentEventKind::Restart));
}

#[test]
fn crash_and_garbage_restart_the_process() {
    let env = Env::new("crash", "");
    let mut i = instance(fast_policy());
    assert!(matches!(
        i.loogle.search(&env.target, "crash", "Mathlib", 10),
        Err(LeanLoogleError::RunFailed { .. })
    ));
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    assert!(matches!(
        i.loogle.search(&env.target, "garbage", "Mathlib", 10),
        Err(LeanLoogleError::InvalidJson(_))
    ));
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    assert_eq!(env.spawned().len(), 3);
    assert_eq!(i.loogle.state(), ResidentState::Ready);
}

#[test]
fn manual_kill_is_noticed_and_restarted() {
    let env = Env::new("killed", "");
    let mut i = instance(fast_policy());
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    let first = env.spawned()[0];
    Command::new("kill")
        .args(["-9", &first.to_string()])
        .status()
        .unwrap();
    // Still a zombie until the supervisor reaps it on the next acquire.
    thread::sleep(Duration::from_millis(100));
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    assert_eq!(env.spawned().len(), 2);
    assert!(wait_dead(first), "the killed process was reaped");
}

#[test]
fn three_consecutive_failures_fall_back_to_oneshot() {
    let env = Env::new("fallback", "");
    let mut i = instance(fast_policy());
    for _ in 0..3 {
        assert!(i.loogle.search(&env.target, "hang", "Mathlib", 10).is_err());
    }
    assert_eq!(i.loogle.state(), ResidentState::Failed);
    let outcome = i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap();
    assert!(matches!(outcome, ResidentLoogleOutcome::Fallback { .. }));
    assert_eq!(
        env.spawned().len(),
        3,
        "no further spawn after the fallback"
    );
    assert!(i.kinds().contains(&ResidentEventKind::Fallback));
}

#[test]
fn stale_index_is_unavailable_and_never_rebuilt() {
    let env = Env::new("stale", "exit");
    let mut i = instance(fast_policy());
    match i.loogle.search(&env.target, "ok", "Mathlib", 10) {
        Err(LeanLoogleError::ResidentUnavailable { reason }) => {
            assert!(reason.contains("stale"), "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(fs::read(&env.target.index_file).unwrap(), b"fake index");
}

#[test]
fn startup_timeout_is_reported() {
    let env = Env::new("never-ready", "never_ready");
    let mut i = instance(ResidentPolicy {
        startup_timeout: Duration::from_millis(300),
        ..fast_policy()
    });
    match i.loogle.search(&env.target, "ok", "Mathlib", 10) {
        Err(LeanLoogleError::ResidentUnavailable { reason }) => {
            assert!(reason.contains("Loogle is ready."), "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    let pid = env.spawned()[0];
    assert!(
        wait_dead(pid),
        "a process that never became ready is killed"
    );
}

#[test]
fn idle_reclaim_after_fifteen_minutes() {
    let env = Env::new("idle", "");
    let mut i = instance(fast_policy());
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    let pid = env.spawned()[0];
    i.clock.advance(Duration::from_secs(14 * 60 + 59));
    assert!(!i.loogle.reap_idle());
    i.clock.advance(Duration::from_secs(1));
    assert!(i.loogle.reap_idle());
    assert!(wait_dead(pid));
    assert_eq!(i.loogle.state(), ResidentState::Stopped);
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    assert_eq!(env.spawned().len(), 2);
}

#[test]
fn module_or_index_change_restarts_with_a_new_key() {
    let env = Env::new("key", "");
    let mut i = instance(fast_policy());
    done(i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap());
    let first = env.spawned()[0];
    done(
        i.loogle
            .search(&env.target, "ok", "Mathlib.Algebra", 10)
            .unwrap(),
    );
    assert!(wait_dead(first), "only one instance at a time");
    assert_eq!(env.spawned().len(), 2);
    // Same path, new bytes (Mathlib update rebuilt the index): new fingerprint.
    fs::write(&env.target.index_file, b"rebuilt index, different size").unwrap();
    let rebuilt = done(
        i.loogle
            .search(&env.target, "ok", "Mathlib.Algebra", 10)
            .unwrap(),
    );
    assert!(rebuilt.index_fingerprint.starts_with("sha256:"));
    assert_eq!(i.count(ResidentEventKind::KeyChanged), 1);
    // A different profile fingerprint is a new key too.
    let mut other = env.target.clone();
    other.profile_fingerprint = "sha256:other".to_owned();
    done(
        i.loogle
            .search(&other, "ok", "Mathlib.Algebra", 10)
            .unwrap(),
    );
    assert_eq!(i.count(ResidentEventKind::KeyChanged), 2);
    assert_eq!(env.spawned().len(), 3);
}

#[test]
fn index_outside_the_managed_cache_is_refused() {
    let env = Env::new("outside", "");
    let mut i = instance(fast_policy());
    let mut target = env.target.clone();
    let outside = env.dir.join("elsewhere.loogle-index");
    fs::write(&outside, b"x").unwrap();
    target.index_file = outside;
    assert!(matches!(
        i.loogle.search(&target, "ok", "Mathlib", 10),
        Err(LeanLoogleError::Access { .. })
    ));
    assert!(env.spawned().is_empty());
}

#[test]
fn disabled_instance_falls_back_without_spawning() {
    let env = Env::new("disabled", "");
    let mut i = instance(fast_policy());
    i.loogle.set_enabled(false);
    assert_eq!(
        i.loogle.search(&env.target, "ok", "Mathlib", 10).unwrap(),
        ResidentLoogleOutcome::Fallback {
            reason: "disabled".to_owned()
        }
    );
    assert!(env.spawned().is_empty());
}

#[test]
fn socket_round_trip_toggle_and_shutdown() {
    let env = Env::new("socket", "");
    let socket = env.dir.join("sock").join("loogle.sock");
    let host = LoogleResidentHost::start(
        socket.clone(),
        ResidentLoogle::with_policy(
            fast_policy(),
            Arc::new(ManualClock::new()),
            mathscape_resident::null_sink(),
        ),
        false,
    )
    .unwrap();
    let parent_mode = fs::metadata(socket.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(parent_mode & 0o777, 0o700);
    let request = |query: &str| LoogleIpcRequest {
        v: LOOGLE_IPC_VERSION,
        target: env.target.clone(),
        query: query.to_owned(),
        module: "Mathlib".to_owned(),
        max_results: 5,
    };
    match request_over_socket(&socket, &request("many"), Duration::from_secs(10)).unwrap() {
        LoogleIpcResponse::Ok { result, .. } => {
            assert_eq!(result.total, 250);
            assert_eq!(result.candidates.len(), 5);
            assert!(result.truncated);
        }
        other => panic!("unexpected {other:?}"),
    }
    match request_over_socket(&socket, &request("unknown"), Duration::from_secs(10)).unwrap() {
        LoogleIpcResponse::Error {
            error: LoogleIpcError::QueryRejected { suggestions, .. },
        } => assert_eq!(suggestions.len(), 1),
        other => panic!("unexpected {other:?}"),
    }
    // Two clients at once are served one after the other.
    let workers: Vec<_> = (0..2)
        .map(|n| {
            let (socket, request) = (socket.clone(), request(&format!("q{n}")));
            thread::spawn(move || request_over_socket(&socket, &request, Duration::from_secs(10)))
        })
        .collect();
    for worker in workers {
        assert!(matches!(
            worker.join().unwrap().unwrap(),
            LoogleIpcResponse::Ok { .. }
        ));
    }
    let pid = env.spawned()[0];
    host.set_enabled(false);
    assert!(wait_dead(pid), "turning the setting off stops the process");
    assert!(matches!(
        request_over_socket(&socket, &request("ok"), Duration::from_secs(10)).unwrap(),
        LoogleIpcResponse::Fallback { .. }
    ));
    host.set_enabled(true);
    assert!(matches!(
        request_over_socket(&socket, &request("ok"), Duration::from_secs(10)).unwrap(),
        LoogleIpcResponse::Ok { .. }
    ));
    let pid = *env.spawned().last().unwrap();
    host.shutdown();
    assert!(wait_dead(pid), "app exit leaves no Loogle behind");
    assert!(!socket.exists(), "socket removed on shutdown");
    assert!(request_over_socket(&socket, &request("ok"), Duration::from_secs(1)).is_err());
}

#[test]
fn forced_off_host_ignores_the_setting() {
    let env = Env::new("forced", "");
    let socket = env.dir.join("sock").join("loogle.sock");
    let host = LoogleResidentHost::start(
        socket.clone(),
        ResidentLoogle::new(mathscape_resident::null_sink()),
        true,
    )
    .unwrap();
    host.set_enabled(true);
    let response = request_over_socket(
        &socket,
        &LoogleIpcRequest {
            v: LOOGLE_IPC_VERSION,
            target: env.target.clone(),
            query: "ok".to_owned(),
            module: "Mathlib".to_owned(),
            max_results: 5,
        },
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(matches!(response, LoogleIpcResponse::Fallback { .. }));
    assert!(env.spawned().is_empty());
    host.shutdown();
}
