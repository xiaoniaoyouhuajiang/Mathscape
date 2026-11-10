//! 真实 `codex app-server` + 本地 Responses API mock 的会话持久化集成测试。
//!
//! 默认跳过：需要本机装有 codex-cli（0.166 验证过）和 python3。
//!
//! ```sh
//! MATHSCAPE_CODEX_MOCK=1 [MATHSCAPE_CODEX_BIN=/path/to/codex] \
//!   cargo test -p mathscape-agent --test codex_live_mock -- --ignored --nocapture
//! ```
//!
//! 不碰用户的 `~/.codex`：Codex home 是临时目录（`CodexConfig::home`），
//! 模型走 `tests/fixtures/codex_mock_responses.py`。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use futures::{StreamExt, executor::block_on};
use mathscape_agent::{
    AgentBackend, AgentEvent, AgentExecutor, CodexConfig, CodexExecutor, ContextNote, EventPayload,
    HarnessContext, LoadThread, Prompt, RoundId, SessionSpec, ThreadContinuity, TurnOutcome,
};
use serde_json::Value;

struct Mock {
    child: Child,
    log: PathBuf,
    port: u16,
}

impl Mock {
    fn start(log: PathBuf) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex_mock_responses.py");
        let child = Command::new("python3")
            .arg(script)
            .arg(port.to_string())
            .arg(&log)
            .spawn()
            .expect("python3");
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "mock did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        Self { child, log, port }
    }

    /// Bodies of POST /v1/responses in order.
    fn requests(&self) -> Vec<Value> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.log)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().ends_with("_POST.json"))
            .collect();
        files.sort();
        files
            .iter()
            .filter_map(|path| serde_json::from_slice::<Value>(&std::fs::read(path).ok()?).ok())
            .filter(|request| {
                request["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("/responses"))
            })
            .map(|request| request["body"].clone())
            .collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_home(home: &Path, port: u16) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"model = "mock-model"
model_provider = "mock"
check_for_update_on_startup = false

[model_providers.mock]
name = "Mock"
base_url = "http://127.0.0.1:{port}/v1"
env_key = "MOCK_API_KEY"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[features]
enable_request_compression = false

[analytics]
enabled = false
"#
        ),
    )
    .unwrap();
}

fn codex_executor(home: &Path) -> CodexExecutor {
    let program = std::env::var_os("MATHSCAPE_CODEX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"));
    let mcp_program = std::env::var_os("MATHSCAPE_MCP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/bin/false"));
    CodexExecutor::new(CodexConfig {
        program,
        mcp_program,
        handshake_timeout: Duration::from_secs(60),
        home: Some(home.to_path_buf()),
        ..CodexConfig::default()
    })
}

fn spec(root: &Path) -> SessionSpec {
    SessionSpec {
        backend: AgentBackend::Codex,
        session_key: "sandbox-live".into(),
        workspace: root.join("ws"),
        tools_allow: vec!["workspace.search".into()],
        harness: HarnessContext {
            env: vec![("MOCK_API_KEY".into(), "mock".into())],
            ..HarnessContext::default()
        },
        round_file: Some(root.join("session/current_round")),
        ephemeral: false,
    }
}

fn run(executor: &CodexExecutor, round: &RoundId) -> Vec<AgentEvent> {
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
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let end = event.type_name() == "turn_end";
        events.push(event);
        if end {
            break;
        }
    }
    let Some(EventPayload::TurnEnd(end)) = events.last().map(|event| &event.payload) else {
        panic!("no turn_end: {events:?}");
    };
    assert_eq!(end.outcome, TurnOutcome::Completed, "{events:?}");
    events
}

fn ask(
    executor: &CodexExecutor,
    thread: &str,
    text: &str,
    context: Vec<ContextNote>,
) -> Vec<AgentEvent> {
    let round = block_on(executor.prompt(
        "sandbox-live",
        Prompt {
            thread_id: thread.into(),
            text: text.into(),
            context,
        },
    ))
    .unwrap();
    run(executor, &round)
}

/// With a real `mathscape` binary (MATHSCAPE_MCP_BIN) the thread's MCP
/// server must come up with `--round-file`.
fn assert_mcp_ready(events: &[AgentEvent]) {
    if std::env::var_os("MATHSCAPE_MCP_BIN").is_none() {
        return;
    }
    let startup = events.iter().find_map(|event| match &event.payload {
        EventPayload::ToolResult(result) if result.call_id.as_str() == "mcp-startup" => {
            Some(result)
        }
        _ => None,
    });
    let startup = startup.unwrap_or_else(|| panic!("no mcp-startup step: {events:?}"));
    assert_eq!(
        startup.status,
        mathscape_agent::ToolResultStatus::Ok,
        "{:?}",
        startup.raw
    );
}

fn user_texts(request: &Value) -> String {
    request["input"].to_string()
}

#[test]
#[ignore = "needs codex-cli + python3; MATHSCAPE_CODEX_MOCK=1 cargo test -p mathscape-agent --test codex_live_mock -- --ignored"]
fn real_codex_threads_persist_resume_and_restore() {
    if std::env::var("MATHSCAPE_CODEX_MOCK").ok().as_deref() != Some("1") {
        eprintln!("skip: set MATHSCAPE_CODEX_MOCK=1");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    mathscape_artifact::ArtifactPool::create(root.path().join("ws")).unwrap();
    std::fs::create_dir_all(root.path().join("session")).unwrap();
    let mock = Mock::start(root.path().join("mocklog"));
    let home = root.path().join("codex-home");
    write_home(&home, mock.port);

    let executor = codex_executor(&home);
    block_on(executor.open_session(spec(root.path()))).unwrap();
    let thread = block_on(executor.new_thread("sandbox-live")).unwrap();
    let events = ask(&executor, &thread.thread_id, "ALPHA_MARK NOTOOL", vec![]);
    assert_mcp_ready(&events);
    ask(
        &executor,
        &thread.thread_id,
        "BETA_MARK NOTOOL",
        vec![ContextNote {
            key: "mathscape.workspace_delta".into(),
            text: "DELTA_MARK".into(),
        }],
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "{requests:#?}");
    // Same thread: turn 2 carries turn 1, and the cached prefix is unchanged.
    assert!(user_texts(&requests[1]).contains("ALPHA_MARK"));
    assert!(user_texts(&requests[1]).contains("DELTA_MARK"));
    assert!(user_texts(&requests[1]).contains("<mathscape_turn_context>"));
    assert_eq!(requests[0]["instructions"], requests[1]["instructions"]);
    let first_input = requests[0]["input"].as_array().unwrap();
    let second_input = requests[1]["input"].as_array().unwrap();
    assert!(second_input.len() > first_input.len());
    assert_eq!(
        &second_input[..first_input.len()],
        first_input.as_slice(),
        "turn 2 must extend turn 1's input unchanged (prompt cache prefix)"
    );

    let rollout = executor
        .thread_state_path("sandbox-live", &thread.thread_id)
        .expect("rollout");
    assert!(rollout.starts_with(&home));
    let copy_dir = root.path().join("session/harness/codex");
    std::fs::create_dir_all(&copy_dir).unwrap();
    let copy = copy_dir.join(rollout.file_name().unwrap());
    std::fs::copy(&rollout, &copy).unwrap();
    block_on(executor.close_session("sandbox-live")).unwrap();

    // Resume by id in a new app-server.
    block_on(executor.open_session(spec(root.path()))).unwrap();
    let resumed = block_on(executor.load_thread(
        "sandbox-live",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: Some(copy.clone()),
        },
    ))
    .unwrap();
    assert_eq!(resumed.continuity, ThreadContinuity::Resumed);
    let events = ask(&executor, &thread.thread_id, "GAMMA_MARK NOTOOL", vec![]);
    assert_mcp_ready(&events);
    let requests = mock.requests();
    assert!(user_texts(requests.last().unwrap()).contains("ALPHA_MARK"));
    assert!(user_texts(requests.last().unwrap()).contains("BETA_MARK"));
    executor.close_all();
    drop(executor);

    // Another machine: empty home + the workspace copy.
    let home2 = root.path().join("codex-home-2");
    write_home(&home2, mock.port);
    let executor = codex_executor(&home2);
    block_on(executor.open_session(spec(root.path()))).unwrap();
    let restored = block_on(executor.load_thread(
        "sandbox-live",
        LoadThread {
            thread_id: thread.thread_id.clone(),
            state_copy: Some(copy.clone()),
        },
    ))
    .unwrap();
    assert_eq!(restored.continuity, ThreadContinuity::Restored);
    ask(&executor, &thread.thread_id, "DELTA2_MARK NOTOOL", vec![]);
    let requests = mock.requests();
    assert!(user_texts(requests.last().unwrap()).contains("ALPHA_MARK"));
    executor.close_all();
}
