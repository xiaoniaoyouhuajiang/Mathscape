//! Stdio peer for the Codex app-server subset [`super::CodexExecutor`] speaks.
//!
//! This is not a second agent protocol and not a Codex implementation. It lets
//! tests and a local dry run exercise the adapter without a ChatGPT login.
//! The `mathscape-codex-fixture` binary is a thin wrapper around
//! [`serve_codex_app_server_fixture_with`].

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

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
    /// After the turn starts, ask for shell approval and wait for the client.
    AskShell,
}

/// Optional `mcpServer/startupStatus/updated` script for the `mathscape`
/// MCP server (and the matching `mcpServerStatus/list` answer).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexFixtureMcp {
    /// `starting` then `ready`; `mcpServerStatus/list` reports these tools.
    Ready(Vec<String>),
    /// `starting` then `failed` with this error text.
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct CodexFixtureOptions {
    pub mode: CodexFixtureMode,
    /// Append each inbound JSON line here (adapter request log).
    pub log_path: Option<PathBuf>,
    /// MCP startup notifications. `None` = Codex sends none (older CLI).
    pub mcp: Option<CodexFixtureMcp>,
    /// Fake `$CODEX_HOME`: non-ephemeral threads append a rollout under
    /// `sessions/2026/01/01/` after each turn, and `thread/resume` of a
    /// thread this process does not know succeeds only when such a rollout
    /// exists anywhere under `sessions/` (as real Codex 0.166 does).
    pub codex_home: Option<PathBuf>,
}

struct FixtureThread {
    ephemeral: bool,
    rollout: Option<PathBuf>,
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
    let mut threads: HashMap<String, FixtureThread> = HashMap::new();
    let mut thread_count = 0_u32;
    let mut turn_count = 0_u32;
    let mut thread_id = String::new();
    let mut turn_id = String::new();
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
            // Same response shape as Codex app-server v2 `model/list`. Ids are
            // fixture placeholders, not a real model catalog.
            "model/list" => write_json(
                &mut writer,
                &json!({
                    "id": id,
                    "result": {
                        "data": [
                            { "id": "fixture-a", "model": "fixture-a", "displayName": "Fixture A",
                              "hidden": false, "isDefault": true, "description": "",
                              "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [
                                { "reasoningEffort": "low", "description": "fixture low" },
                                { "reasoningEffort": "medium", "description": "fixture medium" }
                              ] },
                            { "id": "fixture-b", "model": "fixture-b", "displayName": "Fixture B",
                              "hidden": false, "isDefault": false, "description": "",
                              "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [
                                { "reasoningEffort": "low", "description": "fixture low" },
                                { "reasoningEffort": "medium", "description": "fixture medium" },
                                { "reasoningEffort": "xhigh", "description": "fixture xhigh" }
                              ] },
                            { "id": "fixture-hidden", "model": "fixture-hidden", "displayName": "H",
                              "hidden": true, "isDefault": false, "description": "",
                              "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [] }
                        ],
                        "nextCursor": null
                    }
                }),
            )?,
            "thread/start" => {
                thread_count += 1;
                let id_text = if thread_count == 1 {
                    "thr_fixture".to_owned()
                } else {
                    format!("thr_fixture-{thread_count}")
                };
                let ephemeral = message
                    .pointer("/params/ephemeral")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let rollout = (!ephemeral)
                    .then(|| {
                        options
                            .codex_home
                            .as_deref()
                            .map(|home| rollout_path(home, &id_text))
                    })
                    .flatten();
                threads.insert(
                    id_text.clone(),
                    FixtureThread {
                        ephemeral,
                        rollout: rollout.clone(),
                    },
                );
                // Real Codex sends these right after `thread/started`. The
                // fixture sends them first so the inventory answer lands
                // before its instant scripted turn completes.
                if let Some(mcp) = &options.mcp {
                    write_mcp_startup(&mut writer, &id_text, mcp)?;
                }
                write_json(
                    &mut writer,
                    &json!({
                        "id": id,
                        "result": {
                            "thread": {
                                "id": id_text,
                                "sessionId": id_text,
                                "ephemeral": ephemeral,
                                "path": rollout.map(|path| path.display().to_string()),
                                "modelProvider": "openai"
                            }
                        }
                    }),
                )?;
                write_json(
                    &mut writer,
                    &json!({
                        "method": "thread/started",
                        "params": { "thread": { "id": id_text } }
                    }),
                )?;
            }
            "thread/resume" => {
                let wanted = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let found = if threads.contains_key(&wanted) {
                    true
                } else if let Some(path) = options
                    .codex_home
                    .as_deref()
                    .and_then(|home| find_fixture_rollout(home, &wanted))
                {
                    threads.insert(
                        wanted.clone(),
                        FixtureThread {
                            ephemeral: false,
                            rollout: Some(path),
                        },
                    );
                    true
                } else {
                    false
                };
                if !found {
                    write_json(
                        &mut writer,
                        &json!({
                            "id": id,
                            "error": { "code": -32600,
                                       "message": format!("no rollout found for thread id {wanted}") }
                        }),
                    )?;
                    continue;
                }
                if let Some(mcp) = &options.mcp {
                    write_mcp_startup(&mut writer, &wanted, mcp)?;
                }
                let path = threads
                    .get(&wanted)
                    .and_then(|thread| thread.rollout.clone());
                write_json(
                    &mut writer,
                    &json!({
                        "id": id,
                        "result": {
                            "thread": {
                                "id": wanted,
                                "ephemeral": false,
                                "path": path.map(|path| path.display().to_string()),
                                "turns": []
                            }
                        }
                    }),
                )?;
            }
            "turn/start" => {
                thread_id = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or("thr_fixture")
                    .to_owned();
                let user_text = message
                    .pointer("/params/input")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items
                            .iter()
                            .rev()
                            .find_map(|item| item.get("text").and_then(Value::as_str))
                    })
                    .unwrap_or("")
                    .to_owned();
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
                if !threads.contains_key(&thread_id) {
                    write_json(
                        &mut writer,
                        &json!({
                            "id": id,
                            "error": { "code": -32600, "message": format!("thread not found: {thread_id}") }
                        }),
                    )?;
                    continue;
                }
                turn_count += 1;
                turn_id = if turn_count == 1 {
                    "turn_fixture".to_owned()
                } else {
                    format!("turn_fixture-{turn_count}")
                };
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
                if let Some(thread) = threads.get(&thread_id)
                    && !thread.ephemeral
                    && let Some(path) = &thread.rollout
                {
                    append_rollout(path, &thread_id, &user_text);
                }
                if options.mode == CodexFixtureMode::AskShell {
                    ask_shell(
                        &mut reader,
                        &mut writer,
                        &options.log_path,
                        &thread_id,
                        &turn_id,
                    )?;
                } else {
                    emit_script(&mut writer, options.mode, &thread_id, &turn_id, &user_text)?;
                }
            }
            "mcpServerStatus/list" => {
                let data = match &options.mcp {
                    Some(CodexFixtureMcp::Ready(tools)) => {
                        let tools: serde_json::Map<String, Value> = tools
                            .iter()
                            .map(|name| (name.clone(), json!({ "name": name, "inputSchema": {} })))
                            .collect();
                        json!([{ "name": "mathscape", "runtimeStatus": "connected",
                                 "tools": tools, "toolsError": null }])
                    }
                    Some(CodexFixtureMcp::Failed(error)) => {
                        json!([{ "name": "mathscape", "runtimeStatus": "failed",
                                 "tools": {}, "toolsError": error }])
                    }
                    None => json!([]),
                };
                write_json(
                    &mut writer,
                    &json!({ "id": id, "result": { "data": data, "nextCursor": null } }),
                )?;
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

fn rollout_path(home: &Path, thread_id: &str) -> PathBuf {
    home.join("sessions/2026/01/01")
        .join(format!("rollout-2026-01-01T00-00-00-{thread_id}.jsonl"))
}

fn find_fixture_rollout(home: &Path, thread_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{thread_id}.jsonl");
    let mut stack = vec![home.join("sessions")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
            {
                return Some(path);
            }
        }
    }
    None
}

fn append_rollout(path: &Path, thread_id: &str, user_text: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let line = json!({ "type": "fixture_turn", "threadId": thread_id, "text": user_text });
        let _ = writeln!(file, "{line}");
    }
}

fn write_mcp_startup(
    writer: &mut impl Write,
    thread_id: &str,
    mcp: &CodexFixtureMcp,
) -> io::Result<()> {
    let status = |status: &str, error: Value| {
        json!({
            "method": "mcpServer/startupStatus/updated",
            "params": { "threadId": thread_id, "name": "mathscape", "status": status,
                        "error": error, "failureReason": null }
        })
    };
    write_json(writer, &status("starting", Value::Null))?;
    match mcp {
        CodexFixtureMcp::Ready(_) => write_json(writer, &status("ready", Value::Null)),
        CodexFixtureMcp::Failed(error) => write_json(writer, &status("failed", json!(error))),
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
    // Harness shell item: mapped as tool `shell` (server `codex`), not an MCP tool.
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

/// Sends one shell approval request and finishes the turn from the client's decision.
fn ask_shell(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    log: &Option<PathBuf>,
    thread_id: &str,
    turn_id: &str,
) -> io::Result<()> {
    let request_id = 9001_i64;
    write_json(
        writer,
        &json!({
            "method": "item/commandExecution/requestApproval",
            "id": request_id,
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "itemId": "cmd_ask",
                "command": "rm -rf /tmp/mathscape-sandbox",
                "cwd": "/tmp",
                "reason": "fixture"
            }
        }),
    )?;
    let response = read_response(reader, log, request_id)?;
    let decision = response
        .pointer("/result/decision")
        .and_then(Value::as_str)
        .unwrap_or("");
    let declined = decision == "decline";
    write_json(
        writer,
        &json!({
            "method": "item/started",
            "params": { "item": {
                "type": "commandExecution",
                "id": "cmd_ask",
                "command": "rm -rf /tmp/mathscape-sandbox",
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
                "id": "cmd_ask",
                "command": "rm -rf /tmp/mathscape-sandbox",
                "cwd": "/tmp",
                "status": if declined { "declined" } else { "completed" }
            }}
        }),
    )?;
    write_json(
        writer,
        &json!({
            "method": "item/completed",
            "params": { "item": {
                "type": "agentMessage",
                "id": "msg_1",
                "text": format!("decision:{decision}")
            }}
        }),
    )?;
    write_json(
        writer,
        &json!({
            "method": "turn/completed",
            "params": { "turn": { "id": turn_id, "status": "completed", "items": [], "error": null } }
        }),
    )?;
    Ok(())
}

fn read_response(reader: &mut impl BufRead, log: &Option<PathBuf>, id: i64) -> io::Result<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "approval response missing",
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(path) = log {
            append_log(path, trimmed);
        }
        let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if message.get("method").is_some() {
            continue;
        }
        if message.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(message);
        }
    }
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
