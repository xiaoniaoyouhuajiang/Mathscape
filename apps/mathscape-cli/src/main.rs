//! `mathscape` command-line entry point.
//!
//! Subcommands:
//! - `mathscape mcp --workspace <path> [--round <token>]`: read-only MCP
//!   server on stdio (Phase1). `--round` is an accepted placeholder.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use mathscape_mcp::{McpServer, McpWorkspace};

const USAGE: &str = "用法：\n  mathscape mcp --workspace <path> [--round <token>]\n\n子命令：\n  mcp    以 stdio 运行只读 MCP 服务（workspace.search、material.read、material.grep）";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Mcp {
        workspace: PathBuf,
        round: Option<String>,
    },
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    match args.next().as_deref() {
        None | Some("-h" | "--help" | "help") => Ok(Command::Help),
        Some("mcp") => {
            let mut workspace = None;
            let mut round = None;
            while let Some(arg) = args.next() {
                let (flag, inline) = match arg.split_once('=') {
                    Some((flag, value)) => (flag.to_owned(), Some(value.to_owned())),
                    None => (arg, None),
                };
                let mut value = |name: &str| {
                    inline
                        .clone()
                        .or_else(|| args.next())
                        .ok_or_else(|| format!("{name} 需要一个值"))
                };
                match flag.as_str() {
                    "--workspace" => {
                        let path = value("--workspace")?;
                        if path.is_empty() {
                            return Err("--workspace 需要一个非空路径".to_owned());
                        }
                        workspace = Some(PathBuf::from(path));
                    }
                    "--round" => round = Some(value("--round")?),
                    "-h" | "--help" => return Ok(Command::Help),
                    other => return Err(format!("未知参数：{other}")),
                }
            }
            let workspace = workspace.ok_or("缺少 --workspace <path>")?;
            Ok(Command::Mcp { workspace, round })
        }
        Some(other) => Err(format!("未知子命令：{other}")),
    }
}

fn main() -> ExitCode {
    let command = match parse_args(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Mcp { workspace, round } => run_mcp(workspace, round),
    }
}

fn run_mcp(workspace: PathBuf, round: Option<String>) -> ExitCode {
    let workspace = match McpWorkspace::open(&workspace) {
        Ok(opened) => opened.with_round(round),
        Err(error) => {
            eprintln!("{error}（{}）", workspace.display());
            return ExitCode::FAILURE;
        }
    };
    // stdout carries protocol messages only; diagnostics go to stderr.
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match McpServer::new(workspace).serve(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "MCP 服务异常退出：{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn parses_mcp_with_workspace_and_round() {
        assert_eq!(
            parse(&["mcp", "--workspace", "/tmp/ws", "--round", "r1"]),
            Ok(Command::Mcp {
                workspace: PathBuf::from("/tmp/ws"),
                round: Some("r1".into())
            })
        );
        assert_eq!(
            parse(&["mcp", "--workspace=/tmp/ws"]),
            Ok(Command::Mcp {
                workspace: PathBuf::from("/tmp/ws"),
                round: None
            })
        );
    }

    #[test]
    fn rejects_missing_workspace_and_unknown_flags() {
        assert!(parse(&["mcp"]).is_err());
        assert!(parse(&["mcp", "--workspace"]).is_err());
        assert!(parse(&["mcp", "--workspace", "/tmp/ws", "--bogus"]).is_err());
        assert!(parse(&["bogus"]).is_err());
        assert_eq!(parse(&[]), Ok(Command::Help));
    }

    #[test]
    fn rejects_empty_workspace_path() {
        for args in [&["mcp", "--workspace="][..], &["mcp", "--workspace", ""]] {
            let message = parse(args).unwrap_err();
            assert!(
                message.contains("非空"),
                "expected a non-empty path error for {args:?}, got {message}"
            );
        }
    }
}
