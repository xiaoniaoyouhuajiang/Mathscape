//! Stdio peer for the Codex app-server subset [`super::CodexExecutor`] speaks.
//!
//! This is not a second agent protocol and not a Codex implementation. It lets
//! tests and a local dry run exercise the adapter without a ChatGPT login.
//! The `mathscape-codex-fixture` binary is a thin wrapper around
//! [`serve_codex_app_server_fixture_with`].

use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CodexFixtureMode {
    /// Search, then read, then usage, then `turn/completed`.
    #[default]
    Scripted,
    /// One in-progress MCP call, then block until `turn/interrupt`.
    HoldUntilInterrupt,
    /// `turn/start` returns HTTP-style unauthorized. No turn is accepted.
    RejectTurnAuth,
}

#[derive(Clone, Debug, Default)]
pub struct CodexFixtureOptions {
    pub mode: CodexFixtureMode,
    /// Append each inbound JSON line here (adapter request log).
    pub log_path: Option<PathBuf>,
}

/// Scripted peer with no request log.
pub fn serve_codex_app_server_fixture(reader: impl BufRead, writer: impl Write) -> io::Result<()> {
    serve_codex_app_server_fixture_with(reader, writer, CodexFixtureOptions::default())
}

pub fn serve_codex_app_server_fixture_with(
    mut reader: impl BufRead,
    mut writer: impl Write,
    options: CodexFixtureOptions,
) -> io::Result<()> {
    let mut line = String::new();
    let thread_id = "thr_fixture".to_owned();
    let turn_id = "turn_fixture".to_owned();
    let mut user_text = String::new();
    let mut script_sent = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(path) = &options.log_path {
            append_log(path, trimmed);
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        match method {
            "initialize" => write_json(
                &mut writer,
                &json!({
                    "id": id,
                    "result": {
                        "userAgent": "mathscape-codex-fixture/0.0.0",
                        "platformFamily": "unix",
                        "platformOs": "linux"
                    }
                }),
            )?,
            "initialized" => {}
            "thread/start" => {
                write_json(
                    &mut writer,
                    &json!({
                        "id": id,
                        "result": {
                            "thread": {
                                "id": thread_id,
                                "sessionId": thread_id,
                                "ephemeral": true,
                                "modelProvider": "openai"
                            }
                        }
                    }),
                )?;
                write_json(
                    &mut writer,
                    &json!({
                        "method": "thread/started",
                        "params": { "thread": { "id": thread_id } }
                    }),
                )?;
            }
            "turn/start" => {
                if let Some(text) = message
                    .pointer("/params/input")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items
                            .iter()
                            .find_map(|item| item.get("text").and_then(Value::as_str))
                    })
                {
                    user_text = text.to_owned();
                }
                if options.mode == CodexFixtureMode::RejectTurnAuth {
                    write_json(
                        &mut writer,
                        &json!({
                            "id": id,
                            "error": { "code": 401, "message": "Unauthorized: not logged in" }
                        }),
                    )?;
                    continue;
                }
                write_json(
                    &mut writer,
                    &json!({
                        "id": id,
                        "result": {
                            "turn": {
                                "id": turn_id,
                                "status": "inProgress",
                                "items": [],
                                "error": null
                            }
                        }
                    }),
                )?;
                if !script_sent {
                    script_sent = true;
                    emit_script(&mut writer, options.mode, &thread_id, &turn_id, &user_text)?;
                }
            }
            "turn/interrupt" => {
                write_json(&mut writer, &json!({ "id": id, "result": {} }))?;
                write_json(
                    &mut writer,
                    &json!({
                        "method": "turn/completed",
                        "params": {
                            "threadId": thread_id,
                            "turn": {
                                "id": turn_id,
                                "status": "interrupted",
                                "items": [],
                                "error": null
                            }
                        }
                    }),
                )?;
            }
            _ if id.is_some() && !method.is_empty() => {
                write_json(&mut writer, &json!({ "id": id, "result": {} }))?;
            }
            _ => {}
        }
    }
}

fn emit_script(
    writer: &mut impl Write,
    mode: CodexFixtureMode,
    thread_id: &str,
    turn_id: &str,
    user_text: &str,
) -> io::Result<()> {
    write_json(
        writer,
        &json!({
            "method": "turn/started",
            "params": { "turn": { "id": turn_id, "status": "inProgress", "items": [] } }
        }),
    )?;
    write_json(
        writer,
        &mcp_started("call_search", "workspace.search", json!({"query": "紧集"})),
    )?;
    if mode == CodexFixtureMode::HoldUntilInterrupt {
        return Ok(());
    }
    write_json(
        writer,
        &mcp_completed(
            "call_search",
            "workspace.search",
            json!({"query": "紧集"}),
            "completed",
            json!({"status": "ok"}),
            false,
            12,
        ),
    )?;
    // Harness shell item: the adapter must drop it (no invented tool name).
    write_json(
        writer,
        &json!({
            "method": "item/started",
            "params": { "item": {
                "type": "commandExecution",
                "id": "cmd_1",
                "command": "ls",
                "cwd": "/tmp",
                "status": "inProgress"
            }}
        }),
    )?;
    write_json(
        writer,
        &json!({
            "method": "item/completed",
            "params": { "item": {
                "type": "commandExecution",
                "id": "cmd_1",
                "command": "ls",
                "cwd": "/tmp",
                "status": "completed",
                "exitCode": 0,
                "durationMs": 3
            }}
        }),
    )?;
    write_json(
        writer,
        &mcp_started("call_read", "material.read", json!({"artifact_id": "a1"})),
    )?;
    write_json(
        writer,
        &mcp_completed(
            "call_read",
            "material.read",
            json!({"artifact_id": "a1"}),
            "completed",
            json!({"status": "truncated", "truncated": true}),
            false,
            20,
        ),
    )?;
    let text = if user_text.is_empty() {
        "done"
    } else {
        user_text
    };
    write_json(
        writer,
        &json!({
            "method": "item/completed",
            "params": { "item": { "type": "agentMessage", "id": "msg_1", "text": text } }
        }),
    )?;
    write_json(
        writer,
        &json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "tokenUsage": {
                    "total": usage_breakdown(1200, 300, 800),
                    "last": usage_breakdown(1200, 300, 800),
                    "modelContextWindow": 200000
                }
            }
        }),
    )?;
    write_json(
        writer,
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": { "id": turn_id, "status": "completed", "items": [], "error": null }
            }
        }),
    )?;
    Ok(())
}

fn usage_breakdown(input: u64, output: u64, cached: u64) -> Value {
    json!({
        "totalTokens": input + output,
        "inputTokens": input,
        "cachedInputTokens": cached,
        "outputTokens": output,
        "reasoningOutputTokens": 0
    })
}

fn mcp_started(id: &str, tool: &str, arguments: Value) -> Value {
    json!({
        "method": "item/started",
        "params": { "item": {
            "type": "mcpToolCall",
            "id": id,
            "server": "mathscape",
            "tool": tool,
            "status": "inProgress",
            "arguments": arguments
        }}
    })
}

fn mcp_completed(
    id: &str,
    tool: &str,
    arguments: Value,
    status: &str,
    structured: Value,
    is_error: bool,
    duration_ms: u64,
) -> Value {
    json!({
        "method": "item/completed",
        "params": { "item": {
            "type": "mcpToolCall",
            "id": id,
            "server": "mathscape",
            "tool": tool,
            "status": status,
            "arguments": arguments,
            "durationMs": duration_ms,
            "error": null,
            "result": {
                "content": [{ "type": "text", "text": structured.to_string() }],
                "structuredContent": structured,
                "isError": is_error
            }
        }}
    })
}

fn write_json(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn append_log(path: &std::path::Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}
