//! Map Codex app-server JSON-RPC messages onto `agent_events v0`.
//!
//! Unknown item types are dropped. This module does not invent event types.
//!
//! MCP startup observability: Codex reports `mcpServer/startupStatus/updated`
//! for the `mathscape` server it spawns. A failed start, and the tool
//! inventory read back with `mcpServerStatus/list` once it is ready, are
//! surfaced as one synthetic `tool_call` / `tool_result` pair
//! (`call_id = mcp-startup`, `tool = mcp.startup`, `server = codex`) whose
//! `raw.mcp` carries `{server, status, tools, expected, missing, error}`.
//! It is buffered until `turn_start` so the round stays well-formed.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use chrono::{DateTime, FixedOffset, Local};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::approval::explain_codex_error;
use crate::{
    AgentBackend, AgentEvent, CallId, ErrorCode, EventPayload, RoundId, ToolResultStatus, TurnId,
    TurnOutcome,
    event::{Cancel, CancelRequester, ErrorEvent, ToolCall, ToolResult, TurnEnd, TurnStart, Usage},
};

pub(crate) struct CodexEventMapper {
    round_id: RoundId,
    input_ref: String,
    seq: u64,
    thread_id: Option<String>,
    turn_id: Option<TurnId>,
    turn_open: bool,
    turn_closed: bool,
    cancel_requested: bool,
    cancel_acked: bool,
    fatal_emitted: bool,
    open_calls: BTreeMap<CallId, Instant>,
    seen_calls: BTreeSet<CallId>,
    agent_text: String,
    agent_text_final: bool,
    /// Message supplied by [`Self::force_stop`] when the process dies.
    pending_note: Option<(ErrorCode, String)>,
    /// When Codex reported `starting` for the mathscape MCP server.
    mcp_started: Option<Instant>,
    /// The `mcp-startup` pair was emitted (or buffered); report only once.
    mcp_reported: bool,
    /// MCP startup report that arrived before `turn_start`.
    pending_mcp: Option<McpReport>,
}

/// Name the adapter registers the Mathscape MCP server under (`thread/start`).
pub(crate) const MATHSCAPE_MCP_SERVER: &str = "mathscape";
/// Synthetic call id / tool name for the MCP startup step.
pub(crate) const MCP_STARTUP_CALL_ID: &str = "mcp-startup";
pub(crate) const MCP_STARTUP_TOOL: &str = "mcp.startup";

#[derive(Debug)]
struct McpReport {
    status: ToolResultStatus,
    duration_ms: u64,
    raw: Value,
}

/// `Some(thread_id)` when `message` says the mathscape MCP server is ready.
/// The session then asks `mcpServerStatus/list` for its tool inventory.
pub(crate) fn mcp_ready_thread(message: &Value) -> Option<String> {
    if message.get("method").and_then(Value::as_str) != Some("mcpServer/startupStatus/updated") {
        return None;
    }
    let params = message.get("params")?;
    if params.get("name").and_then(Value::as_str) != Some(MATHSCAPE_MCP_SERVER)
        || params.get("status").and_then(Value::as_str) != Some("ready")
    {
        return None;
    }
    params
        .get("threadId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `mcpServerStatus/list` request for the mathscape server's tool inventory.
pub(crate) fn mcp_status_list_message(id: u64, thread_id: &str) -> Value {
    serde_json::json!({
        "method": "mcpServerStatus/list",
        "id": id,
        "params": {
            "threadId": thread_id,
            "serverName": MATHSCAPE_MCP_SERVER,
            "detail": "toolsAndAuthOnly"
        }
    })
}

impl CodexEventMapper {
    pub(crate) fn new(round_id: RoundId, input_ref: String) -> Self {
        Self {
            round_id,
            input_ref,
            seq: 0,
            thread_id: None,
            turn_id: None,
            turn_open: false,
            turn_closed: false,
            cancel_requested: false,
            cancel_acked: false,
            fatal_emitted: false,
            open_calls: BTreeMap::new(),
            seen_calls: BTreeSet::new(),
            agent_text: String::new(),
            agent_text_final: false,
            pending_note: None,
            mcp_started: None,
            mcp_reported: false,
            pending_mcp: None,
        }
    }

    /// Final (or accumulated) agent message text of the turn, if any. Its
    /// sha256 is `turn_end.final_text_ref`.
    pub(crate) fn agent_text(&self) -> Option<&str> {
        (!self.agent_text.is_empty()).then_some(self.agent_text.as_str())
    }

    pub(crate) fn turn_closed(&self) -> bool {
        self.turn_closed
    }

    pub(crate) fn note_thread_id(&mut self, id: &str) {
        if self.thread_id.is_none() {
            self.thread_id = Some(id.to_owned());
        }
    }

    /// `turn/start` succeeded. Emits `turn_start` once.
    pub(crate) fn accept_turn(&mut self, turn_id: &str, raw: &Value) -> Vec<AgentEvent> {
        self.ensure_turn(turn_id, raw.clone())
    }

    pub(crate) fn on_notification(&mut self, message: &Value) -> Vec<AgentEvent> {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "thread/started" => {
                if let Some(id) = params.pointer("/thread/id").and_then(Value::as_str) {
                    self.note_thread_id(id);
                }
                Vec::new()
            }
            "turn/started" => {
                let id = turn_object(&params)
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("turn-1")
                    .to_owned();
                self.ensure_turn(&id, params)
            }
            "turn/completed" => {
                let id = turn_object(&params)
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("turn-1")
                    .to_owned();
                let status = turn_object(&params)
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let error = turn_object(&params).get("error").cloned();
                let mut events = self.ensure_turn(&id, Value::Null);
                events.extend(self.finish_turn(&status, error.as_ref(), params));
                events
            }
            "error" => self.on_error_notification(&params),
            "mcpServer/startupStatus/updated" => self.on_mcp_startup(&params),
            "thread/tokenUsage/updated" => self.on_usage(&params),
            "item/agentMessage/delta" => {
                if !self.agent_text_final
                    && let Some(delta) = params.get("delta").and_then(Value::as_str)
                {
                    self.agent_text.push_str(delta);
                }
                Vec::new()
            }
            "item/started" => self.on_item(ItemPhase::Started, &params),
            "item/completed" => self.on_item(ItemPhase::Completed, &params),
            _ => Vec::new(),
        }
    }

    pub(crate) fn on_cancel_request(&mut self) -> Vec<AgentEvent> {
        if self.turn_closed || self.cancel_requested {
            return Vec::new();
        }
        self.cancel_requested = true;
        vec![self.emit(EventPayload::Cancel(Cancel {
            requested_by: CancelRequester::User,
            ack: false,
            raw: None,
        }))]
    }

    pub(crate) fn note_protocol(&mut self, message: impl Into<String>) -> Vec<AgentEvent> {
        vec![self.emit(EventPayload::Error(ErrorEvent {
            code: ErrorCode::Protocol,
            message: message.into(),
            fatal: false,
            raw: None,
        }))]
    }

    /// Non-fatal, readable approval note. Does not open a turn by itself:
    /// handshake can answer a request before `turn/start` is accepted.
    pub(crate) fn note_approval(
        &mut self,
        message: impl Into<String>,
        raw: Value,
    ) -> Vec<AgentEvent> {
        if self.turn_closed {
            return Vec::new();
        }
        vec![self.emit(EventPayload::Error(ErrorEvent {
            code: ErrorCode::Protocol,
            message: message.into(),
            fatal: false,
            raw: Some(raw),
        }))]
    }

    pub(crate) fn force_stop(
        &mut self,
        status: &str,
        message: &str,
        code: ErrorCode,
    ) -> Vec<AgentEvent> {
        if self.turn_closed {
            return Vec::new();
        }
        self.pending_note = Some((code, message.to_owned()));
        self.finish_turn(status, None, serde_json::json!({ "message": message }))
    }

    fn on_error_notification(&mut self, params: &Value) -> Vec<AgentEvent> {
        if self.turn_closed {
            return Vec::new();
        }
        let error = params
            .get("error")
            .cloned()
            .unwrap_or_else(|| params.clone());
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("codex error")
            .to_owned();
        let code = classify_codex_failure(&format!("{message} {error}"));
        let (code, message) = explain_codex_error(code, &message);
        // `ErrorNotification { error, willRetry, threadId, turnId }`: Codex
        // sends `willRetry: true` for transient failures it is retrying
        // ("Reconnecting... n/5"). Those must not fail the turn. Only a final
        // (`willRetry: false` or absent) error is fatal, and at most once.
        let will_retry = params
            .get("willRetry")
            .or_else(|| params.get("will_retry"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut events = Vec::new();
        if !self.turn_open {
            events.extend(self.ensure_turn("turn-1", Value::Null));
        }
        if will_retry || self.fatal_emitted {
            events.push(self.emit(EventPayload::Error(ErrorEvent {
                code,
                message,
                fatal: false,
                raw: Some(params.clone()),
            })));
        } else {
            events.push(self.emit_fatal(code, message, error));
        }
        events
    }

    fn on_mcp_startup(&mut self, params: &Value) -> Vec<AgentEvent> {
        if params.get("name").and_then(Value::as_str) != Some(MATHSCAPE_MCP_SERVER) {
            return Vec::new();
        }
        let status = params.get("status").and_then(Value::as_str).unwrap_or("");
        match status {
            "starting" => {
                self.mcp_started.get_or_insert_with(Instant::now);
                Vec::new()
            }
            "failed" | "cancelled" => {
                let error = params
                    .get("error")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("MCP 服务 {MATHSCAPE_MCP_SERVER} 状态 {status}"));
                let raw = serde_json::json!({
                    "mcp": {
                        "server": MATHSCAPE_MCP_SERVER,
                        "status": status,
                        "error": error,
                        "failureReason": params.get("failureReason").cloned().unwrap_or(Value::Null),
                    }
                });
                self.report_mcp(ToolResultStatus::Error, raw)
            }
            // `ready`: the session follows up with `mcpServerStatus/list`
            // and calls [`Self::on_mcp_inventory`].
            _ => Vec::new(),
        }
    }

    /// Response to `mcpServerStatus/list`. `expected` is the round allow-list
    /// (Desktop passes the fixed read-only catalog); empty = no expectation.
    pub(crate) fn on_mcp_inventory(
        &mut self,
        response: &Value,
        expected: &[String],
    ) -> Vec<AgentEvent> {
        let entry = response
            .pointer("/result/data")
            .and_then(Value::as_array)
            .and_then(|servers| {
                servers.iter().find(|server| {
                    server.get("name").and_then(Value::as_str) == Some(MATHSCAPE_MCP_SERVER)
                })
            });
        let rpc_error = response
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(entry) = entry else {
            // Older Codex without `mcpServerStatus/list`, or no entry: the
            // server is ready but the inventory is unknown. Not a failure.
            let raw = serde_json::json!({
                "mcp": {
                    "server": MATHSCAPE_MCP_SERVER,
                    "status": "ready",
                    "tools": Value::Null,
                    "expected": expected,
                    "error": rpc_error,
                }
            });
            return self.report_mcp(ToolResultStatus::Ok, raw);
        };
        let mut tools: Vec<String> = entry
            .get("tools")
            .and_then(Value::as_object)
            .map(|tools| tools.keys().cloned().collect())
            .unwrap_or_default();
        tools.sort();
        let tools_error = entry
            .get("toolsError")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned);
        let missing: Vec<&String> = expected
            .iter()
            .filter(|name| !tools.contains(name))
            .collect();
        let status = if tools_error.is_some() || !missing.is_empty() {
            ToolResultStatus::Error
        } else {
            ToolResultStatus::Ok
        };
        let raw = serde_json::json!({
            "mcp": {
                "server": MATHSCAPE_MCP_SERVER,
                "status": if tools_error.is_some() { "failed" } else { "ready" },
                "tools": tools,
                "expected": expected,
                "missing": missing,
                "error": tools_error,
            }
        });
        self.report_mcp(status, raw)
    }

    fn report_mcp(&mut self, status: ToolResultStatus, raw: Value) -> Vec<AgentEvent> {
        if self.mcp_reported || self.turn_closed {
            return Vec::new();
        }
        self.mcp_reported = true;
        let report = McpReport {
            status,
            duration_ms: self.mcp_started.map(elapsed_ms).unwrap_or(0),
            raw,
        };
        if !self.turn_open {
            self.pending_mcp = Some(report);
            return Vec::new();
        }
        self.emit_mcp(report)
    }

    fn emit_mcp(&mut self, report: McpReport) -> Vec<AgentEvent> {
        let call_id = CallId::parse(MCP_STARTUP_CALL_ID).expect("mcp startup call id");
        self.seen_calls.insert(call_id.clone());
        vec![
            self.emit(EventPayload::ToolCall(ToolCall {
                call_id: call_id.clone(),
                tool: MCP_STARTUP_TOOL.to_owned(),
                server: "codex".to_owned(),
                args_summary: MATHSCAPE_MCP_SERVER.to_owned(),
                raw: Some(report.raw.clone()),
            })),
            self.emit(EventPayload::ToolResult(ToolResult {
                call_id,
                status: report.status,
                duration_ms: report.duration_ms,
                output_hash: None,
                raw: Some(report.raw),
            })),
        ]
    }

    fn on_usage(&mut self, params: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        if let Some(id) = params.get("turnId").and_then(Value::as_str) {
            events.extend(self.ensure_turn(id, Value::Null));
        }
        let Some(usage) = params
            .get("tokenUsage")
            .or_else(|| params.get("token_usage"))
        else {
            return events;
        };
        let Some(breakdown) = usage.get("last").or_else(|| usage.get("total")) else {
            return events;
        };
        let cached = json_u64(breakdown.get("cachedInputTokens"));
        events.push(self.emit(EventPayload::Usage(Usage {
            input_tokens: json_u64(breakdown.get("inputTokens")).unwrap_or(0),
            output_tokens: json_u64(breakdown.get("outputTokens")).unwrap_or(0),
            cached_input_tokens: cached,
            cost_usd: None,
            raw: Some(usage.clone()),
        })));
        events
    }

    fn on_item(&mut self, phase: ItemPhase, params: &Value) -> Vec<AgentEvent> {
        let Some(item) = item_of(params) else {
            return Vec::new();
        };
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "mcpToolCall" => self.on_tool(phase, item, None),
            "dynamicToolCall" => self.on_tool(phase, item, Some("dynamic")),
            "commandExecution" => self.on_side_effect(phase, item, "shell"),
            "fileChange" => self.on_side_effect(phase, item, "file_change"),
            "agentMessage" if phase == ItemPhase::Completed => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    self.agent_text = text.to_owned();
                    self.agent_text_final = true;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_tool(
        &mut self,
        phase: ItemPhase,
        item: &Value,
        server_override: Option<&str>,
    ) -> Vec<AgentEvent> {
        let Some(tool) = item.get("tool").and_then(Value::as_str) else {
            return Vec::new();
        };
        if tool.is_empty() {
            return Vec::new();
        }
        let server = server_override
            .or_else(|| item.get("server").and_then(Value::as_str))
            .filter(|server| !server.is_empty())
            .unwrap_or("codex")
            .to_owned();
        let arguments = item.get("arguments").cloned().unwrap_or(Value::Null);
        self.track_call(phase, item, tool, &server, args_summary(&arguments))
    }

    fn on_side_effect(&mut self, phase: ItemPhase, item: &Value, tool: &str) -> Vec<AgentEvent> {
        let summary = if tool == "shell" {
            let command = item
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if command.is_empty() {
                "shell".to_owned()
            } else {
                truncate_chars(command, 240)
            }
        } else {
            let count = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| changes.len())
                .unwrap_or(0);
            if count == 0 {
                "文件变更".to_owned()
            } else {
                format!("{count} 个文件变更")
            }
        };
        self.track_call(phase, item, tool, "codex", summary)
    }

    fn track_call(
        &mut self,
        phase: ItemPhase,
        item: &Value,
        tool: &str,
        server: &str,
        args_summary: String,
    ) -> Vec<AgentEvent> {
        let call_id = self.call_id(item);
        let mut events = Vec::new();
        if self.seen_calls.insert(call_id.clone()) {
            if !self.turn_open && !self.turn_closed {
                events.extend(self.ensure_turn("turn-1", Value::Null));
            }
            self.open_calls.insert(call_id.clone(), Instant::now());
            events.push(self.emit(EventPayload::ToolCall(ToolCall {
                call_id: call_id.clone(),
                tool: tool.to_owned(),
                server: server.to_owned(),
                args_summary,
                raw: Some(item.clone()),
            })));
        }
        if phase == ItemPhase::Completed && self.open_calls.contains_key(&call_id) {
            events.push(self.tool_result(call_id, item));
        }
        events
    }

    fn tool_result(&mut self, call_id: CallId, item: &Value) -> AgentEvent {
        let started = self.open_calls.remove(&call_id);
        self.emit(EventPayload::ToolResult(ToolResult {
            call_id,
            status: tool_result_status(item),
            duration_ms: duration_ms(item, started),
            output_hash: output_hash(item),
            raw: Some(item.clone()),
        }))
    }

    fn call_id(&mut self, item: &Value) -> CallId {
        let raw = item.get("id").and_then(Value::as_str).unwrap_or("");
        let cleaned = sanitize_id(raw, &format!("call-{}", self.seq));
        CallId::parse(cleaned).unwrap_or_else(|_| {
            CallId::parse(format!("call-{}", self.seq)).expect("fallback call id")
        })
    }

    fn ensure_turn(&mut self, turn_id: &str, raw: Value) -> Vec<AgentEvent> {
        if self.turn_closed {
            return Vec::new();
        }
        let parsed = TurnId::parse(sanitize_id(turn_id, "turn-1"))
            .unwrap_or_else(|_| TurnId::parse("turn-1").expect("fallback turn id"));
        if self.turn_open {
            if self.turn_id.as_ref() == Some(&parsed) {
                return Vec::new();
            }
            return vec![self.emit(EventPayload::Error(ErrorEvent {
                code: ErrorCode::Protocol,
                message: format!("codex started turn {parsed} while another turn was open"),
                fatal: false,
                raw: Some(raw),
            }))];
        }
        self.turn_id = Some(parsed);
        self.turn_open = true;
        let mut body = Map::new();
        if let Some(thread) = &self.thread_id {
            body.insert("codexThreadId".into(), Value::String(thread.clone()));
        }
        body.insert("codex".into(), raw);
        let mut events = vec![self.emit(EventPayload::TurnStart(TurnStart {
            input_ref: self.input_ref.clone(),
            raw: Some(Value::Object(body)),
        }))];
        if let Some(report) = self.pending_mcp.take() {
            events.extend(self.emit_mcp(report));
        }
        events
    }

    fn finish_turn(&mut self, status: &str, error: Option<&Value>, raw: Value) -> Vec<AgentEvent> {
        if self.turn_closed {
            return Vec::new();
        }
        let mut events = Vec::new();
        if !self.turn_open {
            events.extend(self.ensure_turn("turn-1", Value::Null));
        }
        events.extend(self.close_open_calls());
        let failed = self.fatal_emitted
            || matches!(status, "failed" | "")
            || (status != "completed" && status != "interrupted");
        if failed {
            if !self.fatal_emitted {
                let (code, message) = self.failure_message(status, error);
                events.push(self.emit_fatal(code, message, raw.clone()));
            }
            self.ack_cancel(&mut events, CancelRequester::User);
            events.push(self.turn_end(TurnOutcome::Failed, raw));
        } else if status == "interrupted" {
            let requester = if self.cancel_requested {
                CancelRequester::User
            } else {
                self.cancel_requested = true;
                events.push(self.emit(EventPayload::Cancel(Cancel {
                    requested_by: CancelRequester::System,
                    ack: false,
                    raw: None,
                })));
                CancelRequester::System
            };
            self.ack_cancel(&mut events, requester);
            events.push(self.turn_end(TurnOutcome::Cancelled, raw));
        } else {
            self.ack_cancel(&mut events, CancelRequester::User);
            events.push(self.turn_end(TurnOutcome::Completed, raw));
        }
        self.turn_open = false;
        self.turn_closed = true;
        events
    }

    fn failure_message(&mut self, status: &str, error: Option<&Value>) -> (ErrorCode, String) {
        if let Some((code, message)) = self.pending_note.take() {
            return (code, message);
        }
        let message = error
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if status.is_empty() {
                    "codex turn ended without a status".into()
                } else {
                    format!("codex turn status {status}")
                }
            });
        let code = if status == "failed" {
            classify_codex_failure(&format!(
                "{message} {}",
                error.map(Value::to_string).unwrap_or_default()
            ))
        } else {
            ErrorCode::Protocol
        };
        (code, message)
    }

    fn ack_cancel(&mut self, events: &mut Vec<AgentEvent>, requester: CancelRequester) {
        if self.cancel_requested && !self.cancel_acked {
            self.cancel_acked = true;
            events.push(self.emit(EventPayload::Cancel(Cancel {
                requested_by: requester,
                ack: true,
                raw: None,
            })));
        }
    }

    fn close_open_calls(&mut self) -> Vec<AgentEvent> {
        let open: Vec<_> = self.open_calls.keys().cloned().collect();
        open.into_iter()
            .map(|call_id| {
                let started = self.open_calls.remove(&call_id);
                self.emit(EventPayload::ToolResult(ToolResult {
                    call_id,
                    status: ToolResultStatus::Error,
                    duration_ms: started.map(elapsed_ms).unwrap_or(0),
                    output_hash: None,
                    raw: Some(serde_json::json!({
                        "closed": "turn ended before the tool result"
                    })),
                }))
            })
            .collect()
    }

    fn turn_end(&mut self, outcome: TurnOutcome, raw: Value) -> AgentEvent {
        let final_text_ref =
            (!self.agent_text.is_empty()).then(|| sha256_prefixed(self.agent_text.as_bytes()));
        self.emit(EventPayload::TurnEnd(TurnEnd {
            outcome,
            final_text_ref,
            raw: Some(raw),
        }))
    }

    fn emit_fatal(&mut self, code: ErrorCode, message: String, raw: Value) -> AgentEvent {
        self.fatal_emitted = true;
        self.emit(EventPayload::Error(ErrorEvent {
            code,
            message,
            fatal: true,
            raw: Some(raw),
        }))
    }

    fn emit(&mut self, payload: EventPayload) -> AgentEvent {
        let seq = self.seq;
        self.seq += 1;
        AgentEvent {
            round_id: self.round_id.clone(),
            turn_id: self.turn_id.clone(),
            seq,
            ts: now_ts(),
            backend: AgentBackend::Codex,
            payload,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemPhase {
    Started,
    Completed,
}

fn turn_object(params: &Value) -> &Value {
    params.get("turn").unwrap_or(params)
}

fn item_of(params: &Value) -> Option<&Value> {
    params
        .get("item")
        .or_else(|| params.get("type").map(|_| params))
}

pub(crate) fn classify_codex_failure(text: &str) -> ErrorCode {
    let lower = text.to_ascii_lowercase();
    if lower.contains("unauthorized")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("sign in with chatgpt")
        || lower.contains("usagelimitexceeded")
        || lower.contains("usage limit")
    {
        ErrorCode::Auth
    } else if lower.contains("enoent") || (lower.contains("not found") && lower.contains("codex")) {
        ErrorCode::BackendUnavailable
    } else if lower.contains("parse error")
        || lower.contains("invalid json")
        || lower.contains("protocol")
    {
        ErrorCode::Protocol
    } else {
        ErrorCode::Internal
    }
}

fn tool_result_status(item: &Value) -> ToolResultStatus {
    // Codex serialises `mcpToolCall.error: None` as `null` on success.
    if item.get("error").filter(|v| !v.is_null()).is_some() {
        return ToolResultStatus::Error;
    }
    match item.get("status").and_then(Value::as_str).unwrap_or("") {
        "failed" | "declined" => return ToolResultStatus::Error,
        _ => {}
    }
    let result = item.get("result").cloned().unwrap_or(Value::Null);
    if result
        .get("isError")
        .or_else(|| result.get("is_error"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return ToolResultStatus::Error;
    }
    if let Some(status) = structured_status(&result) {
        return status;
    }
    ToolResultStatus::Ok
}

fn structured_status(result: &Value) -> Option<ToolResultStatus> {
    let structured = result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"));
    if let Some(status) = structured
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
    {
        return Some(status_from_wire(status));
    }
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .find_map(|block| block.get("text").and_then(Value::as_str))
        })?;
    let parsed: Value = serde_json::from_str(text).ok()?;
    parsed
        .get("status")
        .and_then(Value::as_str)
        .map(status_from_wire)
}

fn status_from_wire(status: &str) -> ToolResultStatus {
    match status {
        "ok" => ToolResultStatus::Ok,
        "not_found" => ToolResultStatus::NotFound,
        "ambiguous" => ToolResultStatus::Ambiguous,
        "truncated" => ToolResultStatus::Truncated,
        "capability_unavailable" => ToolResultStatus::CapabilityUnavailable,
        "error" => ToolResultStatus::Error,
        _ => ToolResultStatus::Ok,
    }
}

fn duration_ms(item: &Value, started: Option<Instant>) -> u64 {
    json_u64(item.get("durationMs"))
        .or_else(|| json_u64(item.get("duration_ms")))
        .or_else(|| started.map(elapsed_ms))
        .unwrap_or(0)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn output_hash(item: &Value) -> Option<String> {
    if let Some(result) = item.get("result").filter(|value| !value.is_null()) {
        let structured = result
            .get("structuredContent")
            .or_else(|| result.get("structured_content"));
        return Some(hash_value(structured.unwrap_or(result)));
    }
    item.get("error").filter(|v| !v.is_null()).map(hash_value)
}

fn hash_value(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    sha256_prefixed(&bytes)
}

pub(crate) fn sha256_prefixed(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("sha256:{hex}")
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
}

fn args_summary(arguments: &Value) -> String {
    let text = match arguments {
        Value::Null => String::new(),
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            keys.into_iter()
                .map(|key| {
                    let rendered = match &map[&key] {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    format!("{key}={rendered}")
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        other => other.to_string(),
    };
    truncate_chars(&text, 240)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn sanitize_id(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| {
            if ch.is_whitespace() || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect();
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned
    }
}

fn now_ts() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read_jsonl, validate_round, write_jsonl};

    fn feed(lines: &str) -> Vec<AgentEvent> {
        let mut mapper = CodexEventMapper::new(
            RoundId::parse("codex-fixture").unwrap(),
            "sha256:input".into(),
        );
        let mut events = Vec::new();
        for line in lines.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let message: Value = serde_json::from_str(line).expect("fixture line");
            events.extend(mapper.on_notification(&message));
        }
        events
    }

    #[test]
    fn fixture_round_maps_mcp_statuses_and_shell() {
        let events = feed(include_str!(
            "../../tests/fixtures/codex_app_server_round.jsonl"
        ));
        assert_eq!(
            validate_round(&events),
            vec![],
            "{events:?} issues={:?}",
            validate_round(&events)
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
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "usage",
                "turn_end",
            ]
        );
        assert!(events.iter().all(|event| {
            event.backend == AgentBackend::Codex && event.round_id.as_str() == "codex-fixture"
        }));
        let EventPayload::ToolCall(search) = &events[1].payload else {
            panic!("search call");
        };
        assert_eq!(search.tool, "workspace.search");
        assert_eq!(search.server, "mathscape");
        assert_eq!(search.args_summary, "query=紧集");
        let shell = events.iter().find_map(|event| match &event.payload {
            EventPayload::ToolCall(call) if call.tool == "shell" => Some(call),
            _ => None,
        });
        let shell = shell.expect("shell item");
        assert_eq!(shell.server, "codex");
        assert_eq!(shell.args_summary, "ls");
        assert_eq!(
            search
                .raw
                .as_ref()
                .and_then(|raw| raw.get("type"))
                .and_then(Value::as_str),
            Some("mcpToolCall")
        );
        let statuses: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::ToolResult(result) => Some(result.status),
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            [
                ToolResultStatus::Ok,
                ToolResultStatus::Ok,
                ToolResultStatus::Truncated,
                ToolResultStatus::NotFound,
                ToolResultStatus::Ambiguous,
                ToolResultStatus::CapabilityUnavailable,
                ToolResultStatus::Error,
            ]
        );
        let EventPayload::ToolResult(first) = &events[2].payload else {
            panic!("first result");
        };
        assert_eq!(first.duration_ms, 12);
        assert!(first.output_hash.as_deref().unwrap().starts_with("sha256:"));
        let EventPayload::Usage(usage) = events
            .iter()
            .find(|e| e.type_name() == "usage")
            .unwrap()
            .payload
            .clone()
        else {
            unreachable!()
        };
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.cached_input_tokens, Some(800));
        assert_eq!(usage.cost_usd, None);
        let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
            panic!("turn end");
        };
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert_eq!(
            end.final_text_ref.as_deref(),
            Some(sha256_prefixed(b"done").as_str())
        );
        let mut buffer = Vec::new();
        write_jsonl(&mut buffer, &events).unwrap();
        assert_eq!(read_jsonl(buffer.as_slice()).unwrap(), events);
        let line = crate::to_jsonl_line(&events[0]).unwrap();
        assert!(line.contains("\"backend\":\"codex\""), "{line}");
        assert!(
            line.contains("+") || line.contains('Z'),
            "timestamp needs an offset: {line}"
        );
    }

    #[test]
    fn auth_failure_is_fatal_and_closes_the_turn() {
        let events = feed(include_str!(
            "../../tests/fixtures/codex_app_server_auth.jsonl"
        ));
        assert_eq!(validate_round(&events), vec![]);
        let EventPayload::Error(error) = &events[1].payload else {
            panic!("error event: {events:?}");
        };
        assert_eq!(error.code, ErrorCode::Auth);
        assert!(error.fatal);
        let EventPayload::TurnEnd(end) = &events[2].payload else {
            panic!("turn end");
        };
        assert_eq!(end.outcome, TurnOutcome::Failed);
    }

    #[test]
    fn cancel_then_interrupted_acks_and_cancels() {
        let mut mapper = CodexEventMapper::new(RoundId::parse("r").unwrap(), "prompt".into());
        let mut events = mapper.on_notification(&serde_json::json!({
            "method": "turn/started",
            "params": { "turn": { "id": "turn_9", "status": "inProgress" } }
        }));
        events.extend(mapper.on_cancel_request());
        events.extend(mapper.on_notification(&serde_json::json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_9", "status": "interrupted" } }
        })));
        assert_eq!(validate_round(&events), vec![]);
        let acks: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::Cancel(cancel) => Some(cancel.ack),
                _ => None,
            })
            .collect();
        assert_eq!(acks, [false, true]);
        let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
            panic!("end");
        };
        assert_eq!(end.outcome, TurnOutcome::Cancelled);
    }

    #[test]
    fn open_tool_is_closed_when_the_turn_is_interrupted() {
        let mut mapper = CodexEventMapper::new(RoundId::parse("r").unwrap(), "prompt".into());
        let mut events = mapper.accept_turn("turn_1", &Value::Null);
        let started = serde_json::json!({
            "method": "item/started",
            "params": {
                "item": {
                    "type": "mcpToolCall",
                    "id": "c1",
                    "server": "mathscape",
                    "tool": "workspace.search",
                    "status": "inProgress",
                    "arguments": {}
                }
            }
        });
        events.extend(mapper.on_notification(&started));
        events.extend(mapper.force_stop("interrupted", "killed", ErrorCode::Internal));
        assert_eq!(validate_round(&events), vec![], "{events:?}");
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::ToolResult(result) if result.status == ToolResultStatus::Error
        )));
    }

    #[test]
    fn will_retry_errors_are_not_fatal_and_only_one_fatal_is_emitted() {
        let mut mapper = CodexEventMapper::new(RoundId::parse("r").unwrap(), "prompt".into());
        let mut events = mapper.accept_turn("turn_1", &Value::Null);
        for n in 1..=3 {
            events.extend(mapper.on_notification(&serde_json::json!({
                "method": "error",
                "params": {
                    "error": { "message": format!("Reconnecting... {n}/5") },
                    "willRetry": true,
                    "threadId": "thr_1",
                    "turnId": "turn_1"
                }
            })));
        }
        assert!(!mapper.turn_closed(), "willRetry must not close the turn");
        assert!(events.iter().all(|event| !matches!(
            &event.payload,
            EventPayload::Error(error) if error.fatal
        )));
        assert!(!events.iter().any(|event| event.type_name() == "turn_end"));
        let retries = events
            .iter()
            .filter(|event| matches!(&event.payload, EventPayload::Error(e) if !e.fatal))
            .count();
        assert_eq!(retries, 3);

        // A retried turn may still complete normally.
        let mut ok_mapper = CodexEventMapper::new(RoundId::parse("r2").unwrap(), "p".into());
        let mut ok_events = ok_mapper.accept_turn("turn_1", &Value::Null);
        ok_events.extend(ok_mapper.on_notification(&serde_json::json!({
            "method": "error",
            "params": { "error": { "message": "Reconnecting... 1/5" }, "willRetry": true }
        })));
        ok_events.extend(ok_mapper.on_notification(&serde_json::json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "completed", "error": null } }
        })));
        assert_eq!(validate_round(&ok_events), vec![], "{ok_events:?}");
        let EventPayload::TurnEnd(end) = &ok_events.last().unwrap().payload else {
            panic!("turn end");
        };
        assert_eq!(end.outcome, TurnOutcome::Completed);

        for _ in 0..2 {
            events.extend(mapper.on_notification(&serde_json::json!({
                "method": "error",
                "params": {
                    "error": { "message": "Unauthorized: not logged in" },
                    "willRetry": false
                }
            })));
        }
        events.extend(mapper.on_notification(&serde_json::json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "failed",
                "error": { "message": "Unauthorized: not logged in" } } }
        })));
        assert_eq!(validate_round(&events), vec![], "{events:?}");
        let fatals = events
            .iter()
            .filter(|event| matches!(&event.payload, EventPayload::Error(e) if e.fatal))
            .count();
        assert_eq!(fatals, 1, "{events:?}");
        let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
            panic!("turn end");
        };
        assert_eq!(end.outcome, TurnOutcome::Failed);
    }

    #[test]
    fn null_error_on_successful_mcp_item_is_ok() {
        let item = serde_json::json!({
            "type": "mcpToolCall",
            "status": "completed",
            "error": null,
            "result": { "structuredContent": { "status": "ok" }, "isError": false }
        });
        assert_eq!(tool_result_status(&item), ToolResultStatus::Ok);
        assert_eq!(
            output_hash(&item),
            Some(hash_value(&serde_json::json!({"status": "ok"})))
        );
        let no_result = serde_json::json!({ "type": "mcpToolCall", "error": null });
        assert_eq!(output_hash(&no_result), None);
        let failed = serde_json::json!({
            "type": "mcpToolCall",
            "status": "failed",
            "error": { "message": "boom" }
        });
        assert_eq!(tool_result_status(&failed), ToolResultStatus::Error);
        assert!(output_hash(&failed).is_some());
    }

    #[test]
    fn classify_auth_and_protocol() {
        assert_eq!(
            classify_codex_failure("Unauthorized: not logged in"),
            ErrorCode::Auth
        );
        assert_eq!(
            classify_codex_failure("parse error: invalid json"),
            ErrorCode::Protocol
        );
        assert_eq!(classify_codex_failure("boom"), ErrorCode::Internal);
    }

    fn mapper() -> CodexEventMapper {
        CodexEventMapper::new(RoundId::parse("r-mcp").unwrap(), "sha256:input".into())
    }

    fn startup(status: &str, error: Value) -> Value {
        serde_json::json!({
            "method": "mcpServer/startupStatus/updated",
            "params": { "threadId": "thr", "name": "mathscape", "status": status,
                        "error": error, "failureReason": null }
        })
    }

    fn turn_completed() -> Value {
        serde_json::json!({"method": "turn/completed",
            "params": {"turn": {"id": "t1", "status": "completed", "error": null}}})
    }

    #[test]
    fn mcp_failure_before_turn_is_buffered_until_turn_start() {
        let mut m = mapper();
        assert!(
            m.on_notification(&startup("starting", Value::Null))
                .is_empty()
        );
        let other = serde_json::json!({"method": "mcpServer/startupStatus/updated",
            "params": {"name": "codex_apps", "status": "failed", "error": "x"}});
        assert!(m.on_notification(&other).is_empty());
        assert!(
            m.on_notification(&startup("failed", serde_json::json!("No such file")))
                .is_empty()
        );
        let mut events = m.accept_turn("t1", &Value::Null);
        events.extend(m.on_notification(&turn_completed()));
        assert_eq!(validate_round(&events), vec![]);
        let types: Vec<_> = events.iter().map(AgentEvent::type_name).collect();
        assert_eq!(
            types,
            ["turn_start", "tool_call", "tool_result", "turn_end"]
        );
        let EventPayload::ToolCall(call) = &events[1].payload else {
            panic!("call");
        };
        assert_eq!(call.tool, MCP_STARTUP_TOOL);
        assert_eq!(call.call_id.as_str(), MCP_STARTUP_CALL_ID);
        let EventPayload::ToolResult(result) = &events[2].payload else {
            panic!("result");
        };
        assert_eq!(result.status, ToolResultStatus::Error);
        let raw = result.raw.as_ref().unwrap();
        assert_eq!(raw["mcp"]["status"], "failed");
        assert_eq!(raw["mcp"]["error"], "No such file");
    }

    #[test]
    fn mcp_ready_thread_only_for_mathscape_ready() {
        assert_eq!(
            mcp_ready_thread(&startup("ready", Value::Null)).as_deref(),
            Some("thr")
        );
        assert_eq!(mcp_ready_thread(&startup("starting", Value::Null)), None);
        let list = mcp_status_list_message(7, "thr");
        assert_eq!(list["method"], "mcpServerStatus/list");
        assert_eq!(list["params"]["serverName"], "mathscape");
    }

    #[test]
    fn mcp_inventory_reports_count_and_missing_tools_once() {
        let expected: Vec<String> = ["workspace.search", "graph.get", "ink.page_image"]
            .map(str::to_owned)
            .to_vec();
        let response = serde_json::json!({"id": 4, "result": {"data": [
            {"name": "mathscape", "tools": {"workspace.search": {}}, "toolsError": null}
        ]}});
        let mut m = mapper();
        let mut events = m.accept_turn("t1", &Value::Null);
        events.extend(m.on_mcp_inventory(&response, &expected));
        assert!(m.on_mcp_inventory(&response, &expected).is_empty());
        events.extend(m.on_notification(&turn_completed()));
        assert_eq!(validate_round(&events), vec![]);
        let EventPayload::ToolResult(result) = &events[2].payload else {
            panic!("result");
        };
        assert_eq!(result.status, ToolResultStatus::Error);
        let mcp = &result.raw.as_ref().unwrap()["mcp"];
        assert_eq!(mcp["tools"], serde_json::json!(["workspace.search"]));
        assert_eq!(
            mcp["missing"],
            serde_json::json!(["graph.get", "ink.page_image"])
        );

        let full = serde_json::json!({"id": 4, "result": {"data": [{"name": "mathscape",
            "tools": {"workspace.search": {}, "graph.get": {}, "ink.page_image": {}}}]}});
        let mut m = mapper();
        let _ = m.accept_turn("t1", &Value::Null);
        let events = m.on_mcp_inventory(&full, &expected);
        let EventPayload::ToolResult(result) = &events[1].payload else {
            panic!("result");
        };
        assert_eq!(result.status, ToolResultStatus::Ok);

        // Older Codex: method not found -> ready, inventory unknown, not an error.
        let unsupported =
            serde_json::json!({"id": 4, "error": {"code": -32601, "message": "unknown"}});
        let mut m = mapper();
        let _ = m.accept_turn("t1", &Value::Null);
        let events = m.on_mcp_inventory(&unsupported, &expected);
        let EventPayload::ToolResult(result) = &events[1].payload else {
            panic!("result");
        };
        assert_eq!(result.status, ToolResultStatus::Ok);
        assert!(result.raw.as_ref().unwrap()["mcp"]["tools"].is_null());
    }
}
