//! Dry-run Codex app-server peer.
//!
//! Speaks the JSON-RPC subset `CodexExecutor` implements, on stdio. It does
//! not log in and does not call a model.
//!
//! ```text
//! mathscape-codex-fixture [--hold | --reject-auth | --ask-shell] [--log PATH]
//!                         [--mcp-ready TOOL,TOOL | --mcp-fail MESSAGE] [app-server]
//! mathscape-codex-fixture --version
//! ```
//!
//! `--hold` emits one in-progress MCP call and waits for `turn/interrupt`.
//! `--reject-auth` fails `turn/start` with `Unauthorized: not logged in`.
//! `--ask-shell` requests command approval and echoes the client's decision.
//! `--mcp-ready` / `--mcp-fail` script `mcpServer/startupStatus/updated` for
//! the `mathscape` server (and answer `mcpServerStatus/list`).

use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use mathscape_agent::{
    CodexFixtureMcp, CodexFixtureMode, CodexFixtureOptions, serve_codex_app_server_fixture_with,
};

/// `$CODEX_HOME` for fake rollouts, honoured only under the system temp dir
/// so a test run can never write into a real Codex home.
fn fixture_codex_home() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("CODEX_HOME")?);
    let home = std::fs::canonicalize(&home).unwrap_or(home);
    let temp = std::env::temp_dir();
    let temp = std::fs::canonicalize(&temp).unwrap_or(temp);
    home.starts_with(&temp).then_some(home)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("codex-cli 0.0.0-mathscape-fixture");
        return ExitCode::SUCCESS;
    }
    let mut options = CodexFixtureOptions {
        codex_home: fixture_codex_home(),
        ..CodexFixtureOptions::default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--hold" => options.mode = CodexFixtureMode::HoldUntilInterrupt,
            "--reject-auth" => options.mode = CodexFixtureMode::RejectTurnAuth,
            "--ask-shell" => options.mode = CodexFixtureMode::AskShell,
            "--mcp-ready" => {
                index += 1;
                let tools = args
                    .get(index)
                    .map(|list| {
                        list.split(',')
                            .filter(|name| !name.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                options.mcp = Some(CodexFixtureMcp::Ready(tools));
            }
            "--mcp-fail" => {
                index += 1;
                let message = args.get(index).cloned().unwrap_or_default();
                options.mcp = Some(CodexFixtureMcp::Failed(message));
            }
            "--log" => {
                index += 1;
                if let Some(path) = args.get(index) {
                    options.log_path = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
        index += 1;
    }
    let stdin = BufReader::new(io::stdin());
    let stdout = io::stdout();
    if let Err(error) = serve_codex_app_server_fixture_with(stdin, stdout, options) {
        let _ = writeln!(io::stderr(), "codex fixture: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
