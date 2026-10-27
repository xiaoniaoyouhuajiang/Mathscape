//! Dry-run Codex app-server peer.
//!
//! Speaks the JSON-RPC subset `CodexExecutor` implements, on stdio. It does
//! not log in and does not call a model.
//!
//! ```text
//! mathscape-codex-fixture [--hold | --reject-auth] [--log PATH] [app-server]
//! mathscape-codex-fixture --version
//! ```
//!
//! `--hold` emits one in-progress MCP call and waits for `turn/interrupt`.
//! `--reject-auth` fails `turn/start` with `Unauthorized: not logged in`.

use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use mathscape_agent::{CodexFixtureMode, CodexFixtureOptions, serve_codex_app_server_fixture_with};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("codex-cli 0.0.0-mathscape-fixture");
        return ExitCode::SUCCESS;
    }
    let mut options = CodexFixtureOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--hold" => options.mode = CodexFixtureMode::HoldUntilInterrupt,
            "--reject-auth" => options.mode = CodexFixtureMode::RejectTurnAuth,
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
