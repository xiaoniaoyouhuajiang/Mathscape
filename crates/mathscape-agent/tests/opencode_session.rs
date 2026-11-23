//! ACP subprocess round trips and resident-session contract tests; no model login.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use futures::{StreamExt, executor::block_on};
use mathscape_agent::{
    AgentBackend, AgentEvent, AgentExecutor, ApprovalDecision, ApprovalHandler, ApprovalRequest,
    ContextNote, EventPayload, HarnessContext, LoadThread, OpenCodeConfig, OpenCodeExecutor,
    Prompt, RoundId, SessionSpec, StartRoundRequest, ThreadContinuity, ToolResultStatus,
    TurnOutcome, validate_round,
};
use mathscape_resident::ResidentEventKind;
use serde_json::{Value, json};

struct Env {
    root: tempfile::TempDir,
    log: PathBuf,
}
impl Env {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("ws/scratch")).unwrap();
        std::fs::create_dir(root.path().join("outside")).unwrap();
        std::fs::write(root.path().join("outside/sentinel"), "SECRET_MUST_NOT_LEAK").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            root.path().join("outside"),
            root.path().join("ws/scratch/link"),
        )
        .unwrap();
        let log = root.path().join("wire.jsonl");
        Self { root, log }
    }
    fn config(&self, mode: &str) -> OpenCodeConfig {
        OpenCodeConfig {
            program: env!("CARGO_BIN_EXE_mathscape-acp-fixture").into(),
            args: vec![
                "--mode".into(),
                mode.into(),
                "--log".into(),
                self.log.display().to_string(),
                "acp".into(),
            ],
            state_dir: self.root.path().join("state"),
            handshake_timeout: Duration::from_secs(2),
            interrupt_grace: Duration::from_millis(80),
            no_progress_timeout: Duration::from_secs(2),
            ..Default::default()
        }
    }
    fn executor(&self, mode: &str) -> OpenCodeExecutor {
        OpenCodeExecutor::new(self.config(mode))
    }
    fn spec(&self) -> SessionSpec {
        SessionSpec {
            backend: AgentBackend::OpenCode,
            session_key: "test-session".into(),
            workspace: self.root.path().join("ws"),
            tools_allow: vec![],
            harness: HarnessContext {
                scratch_dir: Some(self.root.path().join("ws/scratch")),
                env: vec![("MATHSCAPE_ACP_TEST_ENV".into(), "forwarded".into())],
                instructions: Some("Extra session instructions".into()),
                ..Default::default()
            },
            round_file: Some(self.root.path().join("round/current")),
            ephemeral: false,
        }
    }
    fn lines(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    fn calls(&self, method: &str) -> Vec<Value> {
        self.lines()
            .into_iter()
            .filter(|v| v["method"] == method)
            .collect()
    }
    fn startups(&self) -> Vec<Value> {
        self.lines()
            .into_iter()
            .filter(|v| v["fixture"] == "startup")
            .collect()
    }
    fn open(&self, executor: &OpenCodeExecutor) -> String {
        block_on(executor.open_session(self.spec())).unwrap();
        block_on(executor.new_thread("test-session"))
            .unwrap()
            .thread_id
    }
}
fn prompt(executor: &OpenCodeExecutor, thread: &str) -> RoundId {
    block_on(executor.prompt(
        "test-session",
        Prompt {
            thread_id: thread.into(),
            text: "hello".into(),
            context: vec![ContextNote {
                key: "mathscape.workspace_delta".into(),
                text: "new context".into(),
            }],
        },
    ))
    .unwrap()
}
fn collect(executor: &OpenCodeExecutor, round: &RoundId) -> Vec<AgentEvent> {
    let stream = executor.events(round).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        block_on(async move {
            let events: Vec<_> = stream.collect().await;
            let _ = tx.send(events);
        });
    });
    let events = rx
        .recv_timeout(Duration::from_secs(8))
        .expect("ACP round hung");
    assert!(validate_round(&events).is_empty(), "{events:?}");
    events
}
fn outcome(events: &[AgentEvent]) -> TurnOutcome {
    match &events.last().expect("events").payload {
        EventPayload::TurnEnd(end) => end.outcome,
        _ => panic!("no turn end"),
    }
}

#[test]
fn roundtrip_mcp_mapping_usage_text_and_metadata_cache() {
    let env = Env::new();
    let executor = env.executor("normal");
    let round = block_on(executor.start_round(StartRoundRequest {
        backend: AgentBackend::OpenCode,
        workspace: env.spec().workspace,
        input_ref: "hello".into(),
        tools_allow: vec![],
        harness: env.spec().harness,
    }))
    .unwrap();
    let events = collect(&executor, &round);
    assert_eq!(outcome(&events), TurnOutcome::Completed);
    assert!(events.iter().all(|e| e.backend == AgentBackend::OpenCode));
    let calls: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let EventPayload::ToolCall(c) = &e.payload {
                Some(c)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        calls
            .iter()
            .map(|c| (c.server.as_str(), c.tool.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("mathscape", "workspace.search"),
            ("mathscape", "ink.page_image"),
            ("mathscape", "material.read")
        ]
    );
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let EventPayload::ToolResult(r) = &e.payload {
                Some(r)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        results.iter().map(|r| r.status).collect::<Vec<_>>(),
        vec![
            ToolResultStatus::Ok,
            ToolResultStatus::NotFound,
            ToolResultStatus::Error
        ]
    );
    assert_eq!(
        results[1].raw.as_ref().unwrap()["title"],
        "mathscape_ink_page_image"
    );
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::Usage(u) if u.input_tokens==10 && u.output_tokens==3 && u.cached_input_tokens==Some(2))));
    assert_eq!(
        executor.final_text(&round).as_deref(),
        Some("fixture reply")
    );
    assert_eq!(
        block_on(executor.events(&round).unwrap().collect::<Vec<_>>()),
        events
    );
}

#[test]
fn lazy_resident_sessions_round_file_context_and_injected_config() {
    let env = Env::new();
    let executor = env.executor("normal");
    block_on(executor.open_session(env.spec())).unwrap();
    assert!(env.startups().is_empty());
    let thread = block_on(executor.new_thread("test-session")).unwrap();
    assert_eq!(thread.continuity, ThreadContinuity::Started);
    for _ in 0..2 {
        let round = prompt(&executor, &thread.thread_id);
        collect(&executor, &round);
        assert_eq!(
            std::fs::read_to_string(env.spec().round_file.unwrap()).unwrap(),
            round.as_str()
        );
    }
    assert_eq!(env.startups().len(), 1);
    assert_eq!(env.calls("initialize").len(), 1);
    assert_eq!(env.calls("session/new").len(), 1);
    let new = &env.calls("session/new")[0];
    assert_eq!(new["params"]["mcpServers"][0]["name"], "mathscape");
    assert_eq!(
        new["params"]["mcpServers"][0]["args"],
        json!([
            "mcp",
            "--workspace",
            env.spec().workspace,
            "--round-file",
            env.spec().round_file.unwrap()
        ])
    );
    assert_eq!(
        new["params"]["mcpServers"][0]["env"],
        json!([{"name":"MATHSCAPE_ACP_TEST_ENV","value":"forwarded"}])
    );
    let startup = &env.startups()[0];
    let config = &startup["config"];
    assert_eq!(startup["forwarded"], "forwarded");
    for name in [
        "*",
        "read",
        "edit",
        "glob",
        "grep",
        "list",
        "bash",
        "external_directory",
    ] {
        assert_eq!(config["permission"][name], "ask");
    }
    for name in ["webfetch", "websearch", "mathscape_*"] {
        assert_eq!(config["permission"][name], "deny");
    }
    assert_eq!(config["permission"]["mathscape_ink_page_image"], "allow");
    assert!(
        config["permission"]
            .get("mathscape_artifact_stage")
            .is_none()
    );
    let path = Path::new(config["instructions"][0].as_str().unwrap());
    assert!(path.starts_with(env.root.path().join("state")));
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("Extra session instructions")
    );
    assert!(!env.spec().workspace.join("opencode.json").exists());
    let prompts = env.calls("session/prompt");
    let blocks = prompts[0]["params"]["prompt"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0]["text"].as_str().unwrap().contains("new context"));
    assert_eq!(blocks[1]["text"], "hello");
    block_on(executor.close_session("test-session")).unwrap();
    assert!(executor.live_sessions().is_empty());
}

#[test]
fn live_load_resume_replay_swallowed_and_missing_thread_errors() {
    let env = Env::new();
    let executor = env.executor("normal");
    let thread = env.open(&executor);
    let load = LoadThread {
        thread_id: thread.clone(),
        state_copy: Some(env.root.path().join("unused-state-copy")),
    };
    assert_eq!(
        block_on(executor.load_thread("test-session", load.clone()))
            .unwrap()
            .continuity,
        ThreadContinuity::Live
    );
    assert!(env.calls("session/load").is_empty());
    assert_eq!(executor.thread_state_path("test-session", &thread), None);
    block_on(executor.close_session("test-session")).unwrap();
    block_on(executor.open_session(env.spec())).unwrap();
    assert_eq!(
        block_on(executor.load_thread("test-session", load))
            .unwrap()
            .continuity,
        ThreadContinuity::Resumed
    );
    let round = prompt(&executor, &thread);
    let events = collect(&executor, &round);
    assert!(
        !serde_json::to_string(&events)
            .unwrap()
            .contains("OLD REPLAY")
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.type_name() == "tool_call")
            .count(),
        3
    );
    assert_eq!(
        executor.final_text(&round).as_deref(),
        Some("fixture reply")
    );
    assert!(
        block_on(executor.load_thread(
            "test-session",
            LoadThread {
                thread_id: "missing".into(),
                state_copy: None
            }
        ))
        .is_err()
    );
}

#[test]
fn spec_changes_restart_and_idle_reclaim_resumes() {
    let env = Env::new();
    let mut config = env.config("normal");
    config.idle_timeout = Some(Duration::from_millis(120));
    let (tx, rx) = mpsc::channel();
    let kinds = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&kinds);
    let executor = OpenCodeExecutor::new(config).with_event_sink(Arc::new(move |e| {
        captured.lock().unwrap().push(e.kind);
        let _ = tx.send(e.kind);
    }));
    let thread = env.open(&executor);
    let mut spec = env.spec();
    spec.harness.instructions = Some("changed".into());
    block_on(executor.open_session(spec)).unwrap();
    assert_eq!(
        block_on(executor.load_thread(
            "test-session",
            LoadThread {
                thread_id: thread.clone(),
                state_copy: None
            }
        ))
        .unwrap()
        .continuity,
        ThreadContinuity::Resumed
    );
    assert_eq!(env.startups().len(), 2);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let kind = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("no idle reclaim");
        if kind == ResidentEventKind::IdleReclaim {
            break;
        }
    }
    assert_eq!(
        block_on(executor.load_thread(
            "test-session",
            LoadThread {
                thread_id: thread,
                state_copy: None
            }
        ))
        .unwrap()
        .continuity,
        ThreadContinuity::Resumed
    );
    assert_eq!(env.startups().len(), 3);
    assert!(
        kinds
            .lock()
            .unwrap()
            .contains(&ResidentEventKind::KeyChanged)
    );
}

#[test]
fn model_probe_cache_persistence_and_live_switch_without_process_restart() {
    let env = Env::new();
    let executor = env.executor("normal");
    assert_eq!(executor.current_model(), None);
    let models = executor.list_models().unwrap();
    assert_eq!(models[0].id, "fixture/a");
    assert_eq!(models[1].display_name.as_deref(), Some("Fixture B"));
    assert_eq!(executor.advertised_model().as_deref(), Some("fixture/a"));
    assert_eq!(executor.list_models().unwrap(), models);
    assert_eq!(env.startups().len(), 1);
    assert!(executor.set_model("missing").is_err());
    executor.set_model("fixture/b").unwrap();
    assert_eq!(executor.current_model().as_deref(), Some("fixture/b"));
    let thread = env.open(&executor);
    assert_eq!(env.startups()[1]["config"]["model"], "fixture/b");
    executor.set_model("fixture/a").unwrap();
    assert_eq!(executor.current_model().as_deref(), Some("fixture/a"));
    let round = prompt(&executor, &thread);
    collect(&executor, &round);
    assert_eq!(env.startups().len(), 2);
    assert!(
        env.calls("session/set_config_option")
            .iter()
            .any(|c| c["params"]["configId"] == "model" && c["params"]["value"] == "fixture/a")
    );
    assert!(executor.list_efforts(None).is_err());
    assert!(executor.set_effort("high").is_err());
    assert_eq!(executor.current_effort(), None);
    block_on(executor.close_session("test-session")).unwrap();
    let next = env.executor("normal");
    assert_eq!(next.current_model().as_deref(), Some("fixture/a"));
}

#[test]
fn user_cancel_contract_and_forced_shutdown_of_unresponsive_peer() {
    for mode in ["hold", "ignore-cancel"] {
        let env = Env::new();
        let executor = env.executor(mode);
        let thread = env.open(&executor);
        let round = prompt(&executor, &thread);
        block_on(executor.cancel(&round)).unwrap();
        let events = collect(&executor, &round);
        let acks: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let EventPayload::Cancel(c) = &e.payload {
                    Some(c.ack)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(acks, vec![false, true]);
        assert_eq!(outcome(&events), TurnOutcome::Cancelled);
        assert_eq!(env.calls("session/cancel").len(), 1);
    }
}

#[test]
fn no_progress_cancels_then_fails_and_empty_quota_is_not_success() {
    let env = Env::new();
    let mut config = env.config("ignore-cancel");
    config.no_progress_timeout = Duration::from_millis(100);
    let executor = OpenCodeExecutor::new(config);
    let thread = env.open(&executor);
    let round = prompt(&executor, &thread);
    let events = collect(&executor, &round);
    assert_eq!(outcome(&events), TurnOutcome::Failed);
    assert_eq!(env.calls("session/cancel").len(), 1);
    assert_eq!(
        events.iter().filter(|e| e.type_name() == "cancel").count(),
        2
    );
    let env = Env::new();
    let executor = env.executor("quota");
    let thread = env.open(&executor);
    let round = prompt(&executor, &thread);
    let events = collect(&executor, &round);
    assert_eq!(outcome(&events), TurnOutcome::Failed);
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::Error(error) if error.fatal && error.message.contains("额度") && error.message.contains("认证") && error.message.contains("OpenCode"))));
    assert_eq!(executor.final_text(&round), None);
}

#[test]
fn provider_and_initialization_errors_are_reported() {
    let env = Env::new();
    let executor = env.executor("bad-initialize");
    block_on(executor.open_session(env.spec())).unwrap();
    assert!(block_on(executor.new_thread("test-session")).is_err());
    let env = Env::new();
    let executor = env.executor("rpc-error");
    let thread = env.open(&executor);
    let round = prompt(&executor, &thread);
    let events = collect(&executor, &round);
    assert_eq!(outcome(&events), TurnOutcome::Failed);
    assert!(
        serde_json::to_string(&events)
            .unwrap()
            .contains("fixture provider error")
    );
}

#[cfg(unix)]
#[test]
fn permissions_choose_kinds_deny_escapes_and_publish_denial_notes() {
    let env = Env::new();
    let executor = env.executor("permissions");
    let thread = env.open(&executor);
    let round = prompt(&executor, &thread);
    let events = collect(&executor, &round);
    assert_eq!(outcome(&events), TurnOutcome::Completed);
    assert_eq!(
        std::fs::read_to_string(env.root.path().join("ws/scratch/ok.txt")).unwrap(),
        "SCRATCH_OK"
    );
    assert!(!env.root.path().join("ws/no.txt").exists());
    assert!(!env.root.path().join("outside/no.txt").exists());
    assert!(
        !serde_json::to_string(&events)
            .unwrap()
            .contains("SECRET_MUST_NOT_LEAK")
    );
    let replies: Vec<_> = env
        .lines()
        .into_iter()
        .filter(|v| v.get("result").is_some() && v.get("method").is_none())
        .collect();
    assert_eq!(replies.len(), 10);
    for reply in replies {
        assert_eq!(
            reply["result"]["outcome"]["optionId"],
            if matches!(reply["id"].as_str(), Some("scratch" | "mcp-allowed")) {
                "custom-yes"
            } else {
                "custom-no"
            }
        );
    }
    assert_eq!(events.iter().filter(|e|matches!(&e.payload,EventPayload::Error(error) if error.raw.as_ref().and_then(|r|r.pointer("/approval/phase")).and_then(Value::as_str)==Some("denied"))).count(),8);
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::ToolCall(c) if c.server=="opencode" && c.tool=="file_change")));
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::ToolCall(c) if c.server=="opencode" && c.tool=="file_read")));
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::ToolCall(c) if c.server=="opencode" && c.tool=="shell")));
}

struct Decline;
impl ApprovalHandler for Decline {
    fn decide(&self, _: &ApprovalRequest<'_>) -> ApprovalDecision {
        ApprovalDecision::Decline
    }
}
#[cfg(unix)]
#[test]
fn approval_callback_decline_wins_over_scratch_and_mathscape_allow() {
    let env = Env::new();
    let executor = env
        .executor("permissions")
        .with_approvals(Arc::new(Decline));
    let thread = env.open(&executor);
    let round = prompt(&executor, &thread);
    collect(&executor, &round);
    assert!(!env.root.path().join("ws/scratch/ok.txt").exists());
    assert!(
        env.lines()
            .iter()
            .filter_map(|v| v.pointer("/result/outcome/optionId"))
            .all(|id| id == "custom-no")
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires OpenCode login/model quota; MATHSCAPE_OPENCODE_LIVE=1 and MATHSCAPE_OPENCODE_MODEL"]
fn opencode_live_native_policy() {
    assert_eq!(std::env::var("MATHSCAPE_OPENCODE_LIVE").as_deref(), Ok("1"));
    let model =
        std::env::var("MATHSCAPE_OPENCODE_MODEL").expect("MATHSCAPE_OPENCODE_MODEL required");
    let env = Env::new();
    let ws = env.root.path().join("ws");
    let outside = env.root.path().join("outside");
    std::fs::create_dir_all(env.root.path().join("state")).unwrap();
    std::fs::write(env.root.path().join("state/model"), model).unwrap();
    let mcp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/mathscape");
    let executor = OpenCodeExecutor::new(OpenCodeConfig {
        state_dir: env.root.path().join("state"),
        mcp_program: mcp,
        interrupt_grace: Duration::from_secs(2),
        no_progress_timeout: Duration::from_secs(90),
        ..Default::default()
    });
    let thread = env.open(&executor);
    let text = format!(
        "Test native file-tool permissions, not Mathscape MCP tools. Attempt each separately, continue after permission rejection; do not claim success without calling the tool: (1) native write SCRATCH_OK to {ws}/scratch/ok.txt; (2) native write DENIED to {ws}/no.txt; (3) native write DENIED to {outside}/no.txt; (4) native read {ws}/scratch/link/sentinel; (5) native write DENIED to {ws}/scratch/link/no.txt; (6) native bash `touch {outside}/bash.txt`; (7) native read {outside}/sentinel. The symlink is deliberate. Do not use bash as a fallback for file tools. Finish after all attempts.",
        ws = ws.display(),
        outside = outside.display()
    );
    let round = block_on(executor.prompt(
        "test-session",
        Prompt {
            thread_id: thread,
            text,
            context: vec![],
        },
    ))
    .unwrap();
    let stream = executor.events(&round).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(block_on(stream.collect::<Vec<_>>()));
    });
    let events = rx
        .recv_timeout(Duration::from_secs(110))
        .expect("live turn exceeded deadline");
    println!("live outcome: {:?}", outcome(&events));
    for event in &events {
        if let EventPayload::Error(error) = &event.payload {
            println!("OpenCode: {}", error.message);
        }
    }
    assert!(!outside.join("no.txt").exists());
    assert!(!outside.join("bash.txt").exists());
    assert!(!ws.join("no.txt").exists());
    assert_eq!(
        std::fs::read_to_string(outside.join("sentinel")).unwrap(),
        "SECRET_MUST_NOT_LEAK"
    );
    let raw = serde_json::to_string(&events).unwrap();
    assert!(!raw.contains("SECRET_MUST_NOT_LEAK"));
    assert!(
        !executor
            .final_text(&round)
            .unwrap_or_default()
            .contains("SECRET_MUST_NOT_LEAK")
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("scratch/ok.txt")).unwrap_or_default(),
        "SCRATCH_OK",
        "scratch write did not execute; live policy validation is incomplete"
    );
    for expected in [
        ws.join("no.txt"),
        outside.join("no.txt"),
        ws.join("scratch/link/sentinel"),
        ws.join("scratch/link/no.txt"),
        outside.join("sentinel"),
    ] {
        assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::ToolCall(c) if c.args_summary.contains(expected.to_str().unwrap()))),"not attempted: {}",expected.display());
    }
    assert!(events.iter().any(|e|matches!(&e.payload,EventPayload::ToolCall(c) if c.server=="opencode" && c.tool=="shell")),"bash not attempted");
}
