//! The adapter must not hang or leak processes when `codex` is a wrapper
//! (npm's node shim) that spawns the real app-server as a grandchild sharing
//! our stdin/stdout/stderr pipes.
//!
//! The wrapper here is a generated `/bin/sh` script: it starts
//! `mathscape-codex-fixture` plus two long-lived `sleep` grandchildren (one
//! ignores SIGTERM) that inherit the pipes, records their pids, and `wait`s.
//! Killing only the direct child would leave them holding stderr forever.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::{FutureExt, StreamExt, executor::block_on};
use mathscape_agent::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, CodexConfig, CodexExecutor, ErrorCode,
    EventPayload, StartRoundRequest, TurnOutcome,
};

fn fixture_program() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_mathscape-codex-fixture")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_mathscape_codex_fixture"))
    {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    let exe = std::env::current_exe().expect("current_exe");
    let candidate = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps")
        .join("mathscape-codex-fixture");
    assert!(candidate.is_file(), "missing {}", candidate.display());
    candidate
}

struct Wrapper {
    dir: PathBuf,
    script: PathBuf,
    pids: PathBuf,
}

impl Wrapper {
    fn new(tag: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("mathscape-codex-pgrp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("codex-wrapper.sh");
        let pids = dir.join("pids");
        // A non-interactive shell gives background jobs /dev/null as stdin, so
        // stash the pipe on fd 3 and hand it to every grandchild explicitly.
        let body = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then exec '{fixture}' --version; fi\n\
             exec 3<&0\n\
             '{fixture}' \"$@\" <&3 3<&- &\n\
             echo $! >> '{pids}'\n\
             sleep 30 <&3 3<&- &\n\
             echo $! >> '{pids}'\n\
             sh -c 'trap \"\" TERM; exec sleep 30' <&3 3<&- &\n\
             echo $! >> '{pids}'\n\
             wait\n",
            fixture = fixture_program().display(),
            pids = pids.display(),
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Another test thread may fork while our write fd is still open; the
        // child then briefly holds it and exec fails with ETXTBSY. Wait until
        // the script is executable before handing it to the adapter.
        let deadline = Instant::now() + Duration::from_secs(3);
        while std::process::Command::new(&script)
            .arg("--version")
            .output()
            .map(|out| !out.status.success())
            .unwrap_or(true)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        Self { dir, script, pids }
    }

    fn executor(&self, args: &[&str]) -> CodexExecutor {
        CodexExecutor::new(CodexConfig {
            program: self.script.clone(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            handshake_timeout: Duration::from_secs(3),
            interrupt_grace: Duration::from_millis(800),
            ..CodexConfig::default()
        })
    }

    fn grandchildren(&self) -> Vec<u32> {
        std::fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    /// Grandchildren that are still running (zombies awaiting a reaper count
    /// as dead: they hold no pipes).
    fn wait_until_gone(&self, within: Duration) -> Vec<u32> {
        let pids = self.grandchildren();
        assert_eq!(pids.len(), 3, "wrapper did not start its grandchildren");
        let deadline = Instant::now() + within;
        loop {
            let alive: Vec<u32> = pids.iter().copied().filter(|pid| running(*pid)).collect();
            if alive.is_empty() || Instant::now() >= deadline {
                return alive;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Wrapper {
    fn drop(&mut self) {
        for pid in self.grandchildren() {
            if running(pid) {
                let _ = std::process::Command::new("kill")
                    .args(["-s", "KILL", &pid.to_string()])
                    .status();
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn running(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => false,
        Ok(stat) => {
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next());
            !matches!(state, Some('Z' | 'X'))
        }
    }
}

fn request() -> StartRoundRequest {
    StartRoundRequest {
        backend: AgentBackend::Codex,
        workspace: std::env::temp_dir(),
        input_ref: "explain 紧集".into(),
        tools_allow: vec![],
        harness: mathscape_agent::HarnessContext::default(),
    }
}

/// Drain `stream` until it ends (the session thread finished) or `timeout`.
fn drain(
    stream: impl futures::Stream<Item = AgentEvent> + Send + 'static,
    timeout: Duration,
    on_event: impl Fn(&AgentEvent) + Send + 'static,
) -> (Vec<AgentEvent>, bool) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        block_on(async move {
            let mut stream = std::pin::pin!(stream);
            while let Some(event) = stream.next().await {
                on_event(&event);
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
    });
    let deadline = Instant::now() + timeout;
    let mut events = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return (events, false);
        }
        match rx.recv_timeout(remaining) {
            Ok(event) => events.push(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return (events, false),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return (events, true),
        }
    }
}

#[test]
fn failed_handshake_through_wrapper_returns_promptly_without_orphans() {
    let wrapper = Wrapper::new("auth");
    let executor = wrapper.executor(&["--reject-auth", "app-server"]);
    let started = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(block_on(executor.start_round(request())));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("start_round hung on a wrapped codex");
    match result {
        Err(AgentError::Harness { code, message, .. }) => {
            assert_eq!(code, ErrorCode::Auth, "{message}");
        }
        other => panic!("expected auth harness error, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let alive = wrapper.wait_until_gone(Duration::from_secs(1));
    assert!(
        alive.is_empty(),
        "orphaned grandchildren still running: {alive:?}"
    );
}

#[test]
fn cancel_through_wrapper_ends_the_stream_without_orphans() {
    let wrapper = Wrapper::new("cancel");
    let executor = std::sync::Arc::new(wrapper.executor(&["--hold", "app-server"]));
    let round = block_on(executor.start_round(request())).expect("start");
    let started = Instant::now();
    let canceller = std::sync::Arc::clone(&executor);
    let round_for_cancel = round.clone();
    let (events, ended) = drain(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        move |event| {
            if event.type_name() == "tool_call" {
                // `cancel` resolves immediately; no nested executor needed.
                let _ = canceller.cancel(&round_for_cancel).now_or_never();
            }
        },
    );
    assert!(
        ended,
        "event stream never finished (session thread hung): {events:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let EventPayload::TurnEnd(end) = &events.last().expect("events").payload else {
        panic!("last event is not turn_end: {events:?}");
    };
    assert_eq!(end.outcome, TurnOutcome::Cancelled);
    let alive = wrapper.wait_until_gone(Duration::from_secs(1));
    assert!(
        alive.is_empty(),
        "orphaned grandchildren still running: {alive:?}"
    );
}

fn session_spec(key: &str) -> mathscape_agent::SessionSpec {
    mathscape_agent::SessionSpec {
        backend: AgentBackend::Codex,
        session_key: key.into(),
        workspace: std::env::temp_dir(),
        tools_allow: vec![],
        harness: mathscape_agent::HarnessContext::default(),
        round_file: None,
        ephemeral: false,
    }
}

fn one_turn(executor: &CodexExecutor, key: &str) {
    block_on(executor.open_session(session_spec(key))).unwrap();
    let thread = block_on(executor.new_thread(key)).unwrap();
    let round = block_on(executor.prompt(
        key,
        mathscape_agent::Prompt {
            thread_id: thread.thread_id,
            text: "hi".into(),
            context: vec![],
        },
    ))
    .unwrap();
    let (events, _) = drain(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        |_| {},
    );
    assert!(
        events.iter().any(|event| event.type_name() == "turn_end"),
        "{events:?}"
    );
}

#[test]
fn persistent_session_survives_the_turn_and_close_all_kills_the_group() {
    let wrapper = Wrapper::new("close-all");
    let executor = wrapper.executor(&["app-server"]);
    one_turn(&executor, "s-close");
    // The app-server stays up between turns …
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        wrapper
            .grandchildren()
            .iter()
            .filter(|pid| running(**pid))
            .count(),
        3
    );
    // … and App exit stops every process in its group.
    executor.close_all();
    let alive = wrapper.wait_until_gone(Duration::from_secs(3));
    assert!(alive.is_empty(), "orphans after close_all: {alive:?}");
}

#[test]
fn idle_reclaim_kills_the_group() {
    let wrapper = Wrapper::new("idle");
    let executor = CodexExecutor::new(CodexConfig {
        program: wrapper.script.clone(),
        args: vec!["app-server".into()],
        handshake_timeout: Duration::from_secs(3),
        interrupt_grace: Duration::from_millis(800),
        idle_timeout: Some(Duration::from_millis(300)),
        ..CodexConfig::default()
    });
    one_turn(&executor, "s-idle");
    let alive = wrapper.wait_until_gone(Duration::from_secs(5));
    assert!(alive.is_empty(), "orphans after idle reclaim: {alive:?}");
}

#[test]
fn dropping_the_executor_kills_the_group() {
    let wrapper = Wrapper::new("drop");
    let executor = wrapper.executor(&["app-server"]);
    one_turn(&executor, "s-drop");
    drop(executor);
    let alive = wrapper.wait_until_gone(Duration::from_secs(3));
    assert!(alive.is_empty(), "orphans after drop: {alive:?}");
}
