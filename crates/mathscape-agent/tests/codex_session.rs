//! 常驻会话（ACP 形状接口）对 `mathscape-codex-fixture` 的端到端测试：
//! 一个会话一个 app-server、多线程多轮、`--round-file`、轮次上下文条目、
//! 按 id 续接、从工作区副本恢复 rollout、空闲回收后自动续接。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::{StreamExt, executor::block_on};
use mathscape_agent::{
    AgentBackend, AgentEvent, AgentExecutor, CodexConfig, CodexExecutor, ContextNote, EventPayload,
    HarnessContext, LoadThread, Prompt, RoundId, SessionSpec, ThreadContinuity, TurnOutcome,
    validate_round,
};

fn fixture_program() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mathscape-codex-fixture"))
}

static N: AtomicU64 = AtomicU64::new(0);

struct Env {
    root: tempfile::TempDir,
    log: PathBuf,
}

impl Env {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("ws")).unwrap();
        std::fs::create_dir_all(root.path().join("session")).unwrap();
        let log = root
            .path()
            .join(format!("fixture-{}.log", N.fetch_add(1, Ordering::Relaxed)));
        Self { root, log }
    }

    fn home(&self, name: &str) -> PathBuf {
        let home = self.root.path().join(name);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn executor(&self, home: &Path, idle: Option<Duration>) -> CodexExecutor {
        CodexExecutor::new(CodexConfig {
            program: fixture_program(),
            args: vec![
                "--mcp-ready".into(),
                "workspace.search,material.read".into(),
                "--log".into(),
                self.log.display().to_string(),
                "app-server".into(),
            ],
            handshake_timeout: Duration::from_secs(5),
            interrupt_grace: Duration::from_secs(2),
            idle_timeout: idle,
            home: Some(home.to_path_buf()),
            ..CodexConfig::default()
        })
    }

    fn round_file(&self) -> PathBuf {
        self.root.path().join("session/current_round")
    }

    fn spec(&self) -> SessionSpec {
        SessionSpec {
            backend: AgentBackend::Codex,
            session_key: "sandbox-1".into(),
            workspace: self.root.path().join("ws"),
            tools_allow: vec!["workspace.search".into(), "material.read".into()],
            harness: HarnessContext::default(),
            round_file: Some(self.round_file()),
            ephemeral: false,
        }
    }

    fn log_lines(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn methods(&self, method: &str) -> Vec<serde_json::Value> {
        self.log_lines()
            .into_iter()
            .filter(|line| line["method"] == method)
            .collect()
    }
}

fn run_turn(executor: &CodexExecutor, round: &RoundId) -> Vec<AgentEvent> {
    let stream = executor.events(round).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        block_on(async move {
            let mut stream = std::pin::pin!(stream);
            while let Some(event) = stream.next().await {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let end = event.type_name() == "turn_end";
        events.push(event);
        if end {
            break;
        }
    }
    assert_eq!(
        events.last().map(AgentEvent::type_name),
        Some("turn_end"),
        "{events:?}"
    );
    assert_eq!(validate_round(&events), vec![], "{events:?}");
    let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
        unreachable!()
    };
    assert_eq!(end.outcome, TurnOutcome::Completed);
    events
}

fn prompt(thread: &str, text: &str, context: Vec<ContextNote>) -> Prompt {
    Prompt {
        thread_id: thread.into(),
        text: text.into(),
        context,
    }
}

fn has_mcp_step(events: &[AgentEvent]) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::ToolResult(result) if result.call_id.as_str() == "mcp-startup")
    })
}

#[test]
fn one_process_serves_many_turns_and_threads() {
    let env = Env::new();
    let home = env.home("codex-home");
    let executor = env.executor(&home, None);
    block_on(executor.open_session(env.spec())).unwrap();
    // Re-opening with the same spec is a no-op.
    block_on(executor.open_session(env.spec())).unwrap();

    let first = block_on(executor.new_thread("sandbox-1")).unwrap();
    assert_eq!(first.continuity, ThreadContinuity::Started);
    let round1 =
        block_on(executor.prompt("sandbox-1", prompt(&first.thread_id, "第一问", vec![]))).unwrap();
    let events1 = run_turn(&executor, &round1);
    assert!(has_mcp_step(&events1), "first turn shows the MCP step");
    assert_eq!(executor.final_text(&round1).as_deref(), Some("第一问"));
    assert_eq!(
        std::fs::read_to_string(env.round_file()).unwrap().trim(),
        round1.as_str()
    );

    let note = ContextNote {
        key: "mathscape.workspace_delta".into(),
        text: "自上轮以来工作区变化：新增节点 n2".into(),
    };
    let round2 =
        block_on(executor.prompt("sandbox-1", prompt(&first.thread_id, "第二问", vec![note])))
            .unwrap();
    let events2 = run_turn(&executor, &round2);
    assert!(
        !has_mcp_step(&events2),
        "MCP step only on a thread's first turn"
    );
    assert_eq!(executor.final_text(&round2).as_deref(), Some("第二问"));
    assert_eq!(
        std::fs::read_to_string(env.round_file()).unwrap().trim(),
        round2.as_str()
    );

    let second = block_on(executor.new_thread("sandbox-1")).unwrap();
    assert_ne!(second.thread_id, first.thread_id);
    let round3 =
        block_on(executor.prompt("sandbox-1", prompt(&second.thread_id, "另一对话", vec![])))
            .unwrap();
    run_turn(&executor, &round3);

    // One app-server for the whole session.
    assert_eq!(env.methods("initialize").len(), 1);
    let starts = env.methods("thread/start");
    assert_eq!(starts.len(), 2);
    for start in &starts {
        let params = &start["params"];
        assert_eq!(params["ephemeral"], false);
        assert_eq!(params["threadSource"], "mathscape");
        let args = params["config"]["mcp_servers.mathscape.args"].as_array().unwrap();
        assert_eq!(args.len(), 5);
        assert_eq!(args[0], "mcp");
        assert_eq!(args[1], "--workspace");
        assert_eq!(
            Path::new(args[2].as_str().unwrap()).canonicalize().unwrap(),
            env.root.path().join("ws").canonicalize().unwrap()
        );
        assert_eq!(args[3], "--round-file");
        assert_eq!(
            Path::new(args[4].as_str().unwrap()).canonicalize().unwrap(),
            env.round_file().canonicalize().unwrap()
        );
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(!instructions.contains(round1.as_str()));
    }
    let turns = env.methods("turn/start");
    assert_eq!(turns.len(), 3);
    let input = turns[1]["params"]["input"].as_array().unwrap();
    assert_eq!(input.len(), 2, "context item + user text");
    let context = input[0]["text"].as_str().unwrap();
    assert!(context.starts_with("<mathscape_turn_context>"), "{context}");
    assert!(context.contains("新增节点 n2"));
    assert!(context.contains(&format!("round_id: {round2}")));
    assert_eq!(input[1]["text"], "第二问");
    assert!(turns[1]["params"].get("additionalContext").is_none());

    // Codex persisted the thread under the configured home.
    let path = executor
        .thread_state_path("sandbox-1", &first.thread_id)
        .expect("rollout path");
    assert!(path.is_file());
    assert!(
        path.canonicalize()
            .unwrap()
            .starts_with(home.canonicalize().unwrap()),
        "{} is not under {}",
        path.display(),
        home.display()
    );

    block_on(executor.close_session("sandbox-1")).unwrap();
    assert!(executor.live_sessions().is_empty());
}

#[test]
fn closed_sessions_resume_by_id_and_restore_from_a_workspace_copy() {
    let env = Env::new();
    let home = env.home("codex-home");
    let executor = env.executor(&home, None);
    block_on(executor.open_session(env.spec())).unwrap();
    let thread = block_on(executor.new_thread("sandbox-1")).unwrap();
    let round = block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "记住 X", vec![])))
        .unwrap();
    run_turn(&executor, &round);
    let rollout = executor
        .thread_state_path("sandbox-1", &thread.thread_id)
        .unwrap();
    // Mathscape keeps a copy in the session dir.
    let copy_dir = env.root.path().join("session/harness/codex");
    std::fs::create_dir_all(&copy_dir).unwrap();
    let copy = copy_dir.join(rollout.file_name().unwrap());
    std::fs::copy(&rollout, &copy).unwrap();
    block_on(executor.close_session("sandbox-1")).unwrap();

    // Same machine, new process: resume by id from ~/.codex.
    block_on(executor.open_session(env.spec())).unwrap();
    let resumed = block_on(executor.load_thread(
        "sandbox-1",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: Some(copy.clone()),
        },
    ))
    .unwrap();
    assert_eq!(resumed.continuity, ThreadContinuity::Resumed);
    // Loading again while the process lives is Live.
    let live = block_on(executor.load_thread(
        "sandbox-1",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: None,
        },
    ))
    .unwrap();
    assert_eq!(live.continuity, ThreadContinuity::Live);
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "X 是什么", vec![])))
            .unwrap();
    let events = run_turn(&executor, &round);
    assert!(
        has_mcp_step(&events),
        "resumed thread restarts its MCP server"
    );
    let resume = env.methods("thread/resume");
    assert_eq!(resume.len(), 1);
    assert_eq!(resume[0]["params"]["threadId"], thread.thread_id.as_str());
    assert!(resume[0]["params"]["cwd"].is_string());
    assert!(
        resume[0]["params"]["config"]["mcp_servers.mathscape.args"]
            .to_string()
            .contains("--round-file")
    );
    block_on(executor.close_session("sandbox-1")).unwrap();
    drop(executor);

    // Another machine (empty Codex home): without a copy the thread is gone …
    let other_home = env.home("other-home");
    let executor = env.executor(&other_home, None);
    block_on(executor.open_session(env.spec())).unwrap();
    let missing = block_on(executor.load_thread(
        "sandbox-1",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: None,
        },
    ));
    assert!(missing.is_err());
    // … with the workspace copy it is restored under sessions/mathscape/.
    let restored = block_on(executor.load_thread(
        "sandbox-1",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: Some(copy.clone()),
        },
    ))
    .unwrap();
    assert_eq!(restored.continuity, ThreadContinuity::Restored);
    let target = other_home
        .join("sessions/mathscape")
        .join(copy.file_name().unwrap());
    assert!(target.is_file());
    // The caller's copy is never moved or changed.
    assert!(copy.is_file());
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "继续", vec![]))).unwrap();
    run_turn(&executor, &round);
    executor.close_all();
    assert!(executor.live_sessions().is_empty());
}

#[test]
fn idle_reclaim_stops_the_process_and_the_next_prompt_resumes() {
    let env = Env::new();
    let home = env.home("codex-home");
    let executor = env.executor(&home, Some(Duration::from_millis(300)));
    block_on(executor.open_session(env.spec())).unwrap();
    let thread = block_on(executor.new_thread("sandbox-1")).unwrap();
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "一", vec![]))).unwrap();
    run_turn(&executor, &round);
    std::thread::sleep(Duration::from_millis(900));
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "二", vec![]))).unwrap();
    run_turn(&executor, &round);
    assert_eq!(env.methods("initialize").len(), 2, "restarted after idle");
    assert_eq!(env.methods("thread/resume").len(), 1, "auto-resumed by id");
    block_on(executor.close_session("sandbox-1")).unwrap();
}

#[test]
fn changing_the_tool_whitelist_restarts_the_process() {
    let env = Env::new();
    let home = env.home("codex-home");
    let executor = env.executor(&home, None);
    block_on(executor.open_session(env.spec())).unwrap();
    let thread = block_on(executor.new_thread("sandbox-1")).unwrap();
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "一", vec![]))).unwrap();
    run_turn(&executor, &round);
    let mut spec = env.spec();
    spec.tools_allow.push("graph.query".into());
    block_on(executor.open_session(spec)).unwrap();
    let round =
        block_on(executor.prompt("sandbox-1", prompt(&thread.thread_id, "二", vec![]))).unwrap();
    run_turn(&executor, &round);
    assert_eq!(env.methods("initialize").len(), 2);
    let resume = env.methods("thread/resume");
    assert_eq!(resume.len(), 1);
    assert!(
        resume[0]["params"]["config"]["mcp_servers.mathscape.enabled_tools"]
            .to_string()
            .contains("graph.query")
    );
    block_on(executor.close_session("sandbox-1")).unwrap();
}

#[test]
fn unknown_sessions_and_wrong_backends_are_refused() {
    let env = Env::new();
    let home = env.home("codex-home");
    let executor = env.executor(&home, None);
    assert!(matches!(
        block_on(executor.new_thread("nope")),
        Err(mathscape_agent::AgentError::UnknownSession(_))
    ));
    let mut spec = env.spec();
    spec.backend = AgentBackend::Hmz;
    assert!(matches!(
        block_on(executor.open_session(spec)),
        Err(mathscape_agent::AgentError::BackendMismatch { .. })
    ));
}
