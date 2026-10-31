//! Subprocess round trip against `mathscape-codex-fixture`.
//!
//! The fixture speaks the Codex app-server subset the adapter implements.
//! It does not log in and does not call a model. A real Codex login is the
//! ignored test at the bottom (`MATHSCAPE_CODEX_LIVE=1`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::{StreamExt, executor::block_on};
use mathscape_agent::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, CodexConfig, CodexExecutor, ErrorCode,
    EventPayload, StartRoundRequest, TurnOutcome, validate_round,
};

fn fixture_program() -> PathBuf {
    // Cargo documents `CARGO_BIN_EXE_<name>` for integration tests. This
    // workspace's Cargo 1.88 does not export it, so fall back to the bin that
    // sits beside the test harness (`target/<profile>/mathscape-codex-fixture`).
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_mathscape_codex_fixture") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    let exe = std::env::current_exe().expect("integration test current_exe");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("test binary lives in target/<profile>/deps");
    let candidate = profile_dir.join("mathscape-codex-fixture");
    assert!(
        candidate.is_file(),
        "fixture binary missing at {} (build mathscape-agent's bin target first)",
        candidate.display()
    );
    candidate
}

fn fixture_executor(args: Vec<String>) -> CodexExecutor {
    CodexExecutor::new(CodexConfig {
        program: fixture_program(),
        args,
        handshake_timeout: Duration::from_secs(5),
        interrupt_grace: Duration::from_secs(2),
        ..CodexConfig::default()
    })
}

fn request() -> StartRoundRequest {
    StartRoundRequest {
        backend: AgentBackend::Codex,
        workspace: std::env::temp_dir(),
        input_ref: "explain 紧集".into(),
        tools_allow: vec!["workspace.search".into(), "material.read".into()],
    }
}

fn collect_until(
    stream: impl futures::Stream<Item = AgentEvent> + Send + 'static,
    timeout: Duration,
    mut done: impl FnMut(&AgentEvent) -> bool + Send + 'static,
) -> Vec<AgentEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        block_on(async move {
            let mut stream = std::pin::pin!(stream);
            while let Some(event) = stream.next().await {
                let stop = done(&event);
                if tx.send(event).is_err() {
                    break;
                }
                if stop {
                    break;
                }
            }
        });
    });
    let deadline = Instant::now() + timeout;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => events.push(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    events
}

#[test]
fn fixture_probe_reports_a_local_version() {
    let probes = fixture_executor(vec!["app-server".into()]).probe();
    assert_eq!(probes.len(), 1);
    assert!(probes[0].is_available(), "{probes:?}");
    assert_eq!(
        probes[0].found_version.as_deref(),
        Some("0.0.0-mathscape-fixture")
    );
}

#[test]
fn fixture_round_trip_mirrors_mcp_calls_into_agent_events() {
    let log = std::env::temp_dir().join(format!(
        "mathscape-codex-roundtrip-{}-{}.log",
        std::process::id(),
        unique()
    ));
    let _ = std::fs::remove_file(&log);
    let executor = fixture_executor(vec![
        "--log".into(),
        log.display().to_string(),
        "app-server".into(),
    ]);
    let round = block_on(executor.start_round(request())).expect("start round");
    let events = collect_until(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        |event| event.type_name() == "turn_end",
    );
    assert!(
        events
            .last()
            .is_some_and(|event| event.type_name() == "turn_end"),
        "timed out waiting for turn_end: {events:?}"
    );
    assert_eq!(validate_round(&events), vec![], "{events:?}");
    assert!(
        events
            .iter()
            .all(|event| event.round_id == round && event.backend == AgentBackend::Codex)
    );
    let types: Vec<_> = events.iter().map(AgentEvent::type_name).collect();
    assert_eq!(
        types,
        [
            "turn_start",
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "usage",
            "turn_end",
        ]
    );
    let EventPayload::TurnStart(start) = &events[0].payload else {
        panic!("turn_start");
    };
    assert_eq!(start.input_ref, "explain 紧集");
    // The fixture echoes the user text as its agent message; `final_text`
    // resolves `turn_end.final_text_ref` to that visible reply.
    assert_eq!(executor.final_text(&round).as_deref(), Some("explain 紧集"));
    assert_eq!(
        start
            .raw
            .as_ref()
            .and_then(|raw| raw.get("codexThreadId"))
            .and_then(|id| id.as_str()),
        Some("thr_fixture")
    );
    let EventPayload::ToolCall(search) = &events[1].payload else {
        panic!("search");
    };
    assert_eq!(search.server, "mathscape");
    assert_eq!(search.tool, "workspace.search");
    assert_eq!(search.args_summary, "query=紧集");
    let EventPayload::ToolResult(read) = &events[4].payload else {
        panic!("read result");
    };
    assert_eq!(read.status, mathscape_agent::ToolResultStatus::Truncated);
    let EventPayload::Usage(usage) = &events[5].payload else {
        panic!("usage");
    };
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.output_tokens, 300);
    assert_eq!(usage.cached_input_tokens, Some(800));
    assert_eq!(usage.cost_usd, None);
    let EventPayload::TurnEnd(end) = &events[6].payload else {
        panic!("end");
    };
    assert_eq!(end.outcome, TurnOutcome::Completed);
    assert!(
        end.final_text_ref
            .as_deref()
            .unwrap()
            .starts_with("sha256:")
    );

    let mut buffer = Vec::new();
    mathscape_agent::write_jsonl(&mut buffer, &events).unwrap();
    assert_eq!(
        mathscape_agent::read_jsonl(buffer.as_slice()).unwrap(),
        events
    );

    let log_text = std::fs::read_to_string(&log).expect("fixture request log");
    let _ = std::fs::remove_file(&log);
    assert!(log_text.contains("\"method\":\"initialize\""));
    assert!(log_text.contains("\"method\":\"initialized\""));
    assert!(log_text.contains("\"method\":\"thread/start\""));
    assert!(log_text.contains("\"method\":\"turn/start\""));
    assert!(log_text.contains("explain 紧集"));
    assert!(log_text.contains("mcp_servers.mathscape.command"));
    assert!(log_text.contains(&format!("\"--round\",\"{}\"", round.as_str())));
    assert!(log_text.contains("mathscape"));
    let lower = log_text.to_ascii_lowercase();
    assert!(!lower.contains("account/login"));
    assert!(!lower.contains("apikey"));
    assert!(!lower.contains("authorization"));
    assert!(!lower.contains("experimentalapi"));
}

#[test]
fn fixture_cancel_acks_and_ends_cancelled() {
    let executor = fixture_executor(vec!["--hold".into(), "app-server".into()]);
    let round = block_on(executor.start_round(request())).expect("start");
    let opened = collect_until(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        |event| event.type_name() == "tool_call",
    );
    assert!(
        opened.iter().any(|event| event.type_name() == "tool_call"),
        "timed out waiting for tool_call: {opened:?}"
    );
    block_on(executor.cancel(&round)).unwrap();
    let rest = collect_until(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        |event| event.type_name() == "turn_end",
    );
    assert!(
        rest.last()
            .is_some_and(|event| event.type_name() == "turn_end"),
        "timed out waiting for cancel turn_end: {rest:?}"
    );
    let mut events = opened;
    // The second subscribe replays the prefix. Keep the longer suffix only.
    if rest.len() >= events.len() && rest.iter().zip(&events).all(|(a, b)| a == b) {
        events = rest;
    } else {
        events.extend(rest);
    }
    assert_eq!(validate_round(&events), vec![], "{events:?}");
    let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
        panic!("end");
    };
    assert_eq!(end.outcome, TurnOutcome::Cancelled);
}

#[test]
fn fixture_auth_failure_does_not_start_login() {
    let log = std::env::temp_dir().join(format!(
        "mathscape-codex-auth-{}-{}.log",
        std::process::id(),
        unique()
    ));
    let _ = std::fs::remove_file(&log);
    let executor = fixture_executor(vec![
        "--reject-auth".into(),
        "--log".into(),
        log.display().to_string(),
    ]);
    let error = block_on(executor.start_round(request())).expect_err("auth");
    match error {
        AgentError::Harness { code, message, .. } => {
            assert_eq!(code, ErrorCode::Auth, "{message}");
            assert!(message.contains("not logged in"), "{message}");
        }
        other => panic!("expected auth harness error, got {other:?}"),
    }
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    assert!(!log_text.contains("account/login"), "{log_text}");
}

#[test]
#[ignore = "needs a logged-in local Codex CLI; MATHSCAPE_CODEX_LIVE=1 cargo test -p mathscape-agent --test codex_roundtrip -- --ignored live_codex"]
fn live_codex_app_server_round_trip() {
    if std::env::var("MATHSCAPE_CODEX_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skip: set MATHSCAPE_CODEX_LIVE=1 to run against a local Codex login");
        return;
    }
    let executor = CodexExecutor::new(CodexConfig::default());
    let probes = executor.probe();
    assert!(
        probes.first().is_some_and(|probe| probe.is_available()),
        "codex CLI is not on PATH (probe is local --version only): {probes:?}"
    );
    let request = StartRoundRequest {
        backend: AgentBackend::Codex,
        workspace: std::env::temp_dir(),
        input_ref: "Reply with the single word pong. Do not call tools.".into(),
        tools_allow: vec![],
    };
    let round = block_on(executor.start_round(request)).expect("start live codex round");
    let events = collect_until(
        executor.events(&round).unwrap(),
        Duration::from_secs(90),
        |event| event.type_name() == "turn_end",
    );
    assert!(
        events
            .last()
            .is_some_and(|event| event.type_name() == "turn_end"),
        "live Codex did not finish a turn: {events:?}"
    );
    assert_eq!(validate_round(&events), vec![], "{events:?}");
    assert!(
        events
            .iter()
            .all(|event| event.backend == AgentBackend::Codex)
    );
}

fn unique() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Cache missing → `list_models` asks the app-server (`model/list`) instead of
/// leaving the picker with only the current model.
#[test]
fn list_models_falls_back_to_app_server_model_list() {
    let home = std::env::temp_dir().join(format!("mathscape-model-list-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"fixture-b\"\n").unwrap();
    let executor = CodexExecutor::new(CodexConfig {
        program: fixture_program(),
        args: Vec::new(),
        handshake_timeout: Duration::from_secs(5),
        home: Some(home.clone()),
        ..CodexConfig::default()
    });
    let models = executor.list_models().unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["fixture-a", "fixture-b"]);
    assert_eq!(executor.current_model().as_deref(), Some("fixture-b"));
    let _ = std::fs::remove_dir_all(&home);
}

/// Effort follows the model trait end to end: list from the selected model's
/// catalog entry (here via app-server `model/list`), persist as top-level
/// `model_reasoning_effort`, and forward as `effort` on the next `turn/start`.
#[test]
fn effort_is_listed_persisted_and_forwarded_on_turn_start() {
    let home = std::env::temp_dir().join(format!(
        "mathscape-effort-{}-{}",
        std::process::id(),
        unique()
    ));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"fixture-b\"\n").unwrap();
    let log = home.join("requests.log");
    let executor = CodexExecutor::new(CodexConfig {
        program: fixture_program(),
        args: vec![
            "--log".into(),
            log.display().to_string(),
            "app-server".into(),
        ],
        handshake_timeout: Duration::from_secs(5),
        interrupt_grace: Duration::from_secs(2),
        home: Some(home.clone()),
        ..CodexConfig::default()
    });
    let efforts: Vec<_> = executor
        .list_efforts(None)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(efforts, ["low", "medium", "xhigh"]);
    let for_a: Vec<_> = executor
        .list_efforts(Some("fixture-a"))
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(for_a, ["low", "medium"]);

    assert!(executor.set_effort("ultra").is_err(), "not advertised");
    executor.set_effort("xhigh").unwrap();
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(
        config.contains("model_reasoning_effort = \"xhigh\""),
        "{config}"
    );
    assert_eq!(executor.current_effort().as_deref(), Some("xhigh"));

    let _ = std::fs::remove_file(&log);
    let round = block_on(executor.start_round(request())).expect("start round");
    let events = collect_until(
        executor.events(&round).unwrap(),
        Duration::from_secs(5),
        |event| event.type_name() == "turn_end",
    );
    assert!(
        events.iter().any(|e| e.type_name() == "turn_end"),
        "{events:?}"
    );
    let log_text = std::fs::read_to_string(&log).expect("fixture request log");
    let turn_start: serde_json::Value = log_text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| v["method"] == "turn/start")
        .expect("turn/start logged");
    assert_eq!(turn_start["params"]["effort"], "xhigh");
    let thread_start: serde_json::Value = log_text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| v["method"] == "thread/start")
        .expect("thread/start logged");
    assert_eq!(thread_start["params"]["model"], "fixture-b");

    // Switching to a model without `xhigh` resets the persisted effort to
    // that model's default, as the Codex picker would.
    executor.set_model("fixture-a").unwrap();
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("model = \"fixture-a\""), "{config}");
    assert!(
        config.contains("model_reasoning_effort = \"medium\""),
        "{config}"
    );
    let _ = std::fs::remove_dir_all(&home);
}
