use std::collections::{HashMap, HashSet};
use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AgentBackend, AgentEvent, CallId, Cancel, CancelRequester, ErrorCode, ErrorEvent, EventPayload,
    RoundId, ToolCall, ToolResult, ToolResultStatus, TurnEnd, TurnId, TurnOutcome, TurnStart,
    Usage,
};

struct Call {
    raw: Value,
    start: Instant,
}

pub(crate) struct Mapper {
    round: RoundId,
    turn: TurnId,
    backend: AgentBackend,
    server: String,
    names: HashMap<String, String>,
    seq: u64,
    calls: HashMap<String, Call>,
    finished: HashSet<String>,
    pub(crate) text: String,
    pub(crate) saw_tool: bool,
    cancelling: Option<CancelRequester>,
}

impl Mapper {
    pub(crate) fn new(
        round: RoundId,
        backend: AgentBackend,
        server: &str,
        names: HashMap<String, String>,
    ) -> Self {
        let turn = TurnId::parse(format!("acp-{}", round.as_str())).expect("valid ACP turn id");
        Self {
            round,
            turn,
            backend,
            server: server.into(),
            names,
            seq: 0,
            calls: HashMap::new(),
            finished: HashSet::new(),
            text: String::new(),
            saw_tool: false,
            cancelling: None,
        }
    }

    fn event(&mut self, payload: EventPayload) -> AgentEvent {
        self.seq += 1;
        AgentEvent {
            round_id: self.round.clone(),
            turn_id: Some(self.turn.clone()),
            seq: self.seq,
            ts: chrono::Utc::now().fixed_offset(),
            backend: self.backend,
            payload,
        }
    }

    pub(crate) fn start(&mut self, input: String) -> AgentEvent {
        self.event(EventPayload::TurnStart(TurnStart {
            input_ref: input,
            raw: None,
        }))
    }

    pub(crate) fn update(&mut self, update: &Value) -> Vec<AgentEvent> {
        let kind = update["sessionUpdate"].as_str().unwrap_or("");
        if kind == "agent_message_chunk" {
            if update.pointer("/content/type").and_then(Value::as_str) == Some("text") {
                self.text.push_str(
                    update
                        .pointer("/content/text")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
            }
            return Vec::new();
        }
        if !matches!(kind, "tool_call" | "tool_call_update") {
            return Vec::new();
        }
        let Some(id) = update["toolCallId"].as_str() else {
            return Vec::new();
        };
        if self.finished.contains(id) {
            return Vec::new();
        }
        let Ok(call_id) = CallId::parse(id) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if !self.calls.contains_key(id) {
            self.saw_tool = true;
            let title = update["title"].as_str().unwrap_or("unknown");
            let (server, tool) = if let Some(tool) = self.names.get(title) {
                ("mathscape".to_owned(), tool.clone())
            } else {
                (self.server.clone(), native_name(update).to_owned())
            };
            events.push(self.event(EventPayload::ToolCall(ToolCall {
                call_id: call_id.clone(),
                tool,
                server,
                args_summary: update.get("rawInput").unwrap_or(&Value::Null).to_string(),
                raw: Some(update.clone()),
            })));
            self.calls.insert(
                id.into(),
                Call {
                    raw: update.clone(),
                    start: Instant::now(),
                },
            );
        }
        let call = self.calls.get_mut(id).expect("cached call");
        if let (Some(old), Some(new)) = (call.raw.as_object_mut(), update.as_object()) {
            for (key, value) in new {
                old.insert(key.clone(), value.clone());
            }
        }
        if matches!(update["status"].as_str(), Some("completed" | "failed")) {
            let call = self.calls.remove(id).expect("cached call");
            let status = if update["status"] == "failed" {
                ToolResultStatus::Error
            } else {
                business_status(&call.raw)
            };
            events.push(self.event(EventPayload::ToolResult(ToolResult {
                call_id,
                status,
                duration_ms: call.start.elapsed().as_millis().min(u64::MAX as u128) as u64,
                output_hash: None,
                raw: Some(call.raw),
            })));
            self.finished.insert(id.into());
        }
        events
    }

    pub(crate) fn note(&mut self, message: String, raw: Value) -> AgentEvent {
        self.event(EventPayload::Error(ErrorEvent {
            code: ErrorCode::Protocol,
            message,
            fatal: false,
            raw: Some(raw),
        }))
    }

    pub(crate) fn cancel(&mut self, by: CancelRequester) -> Vec<AgentEvent> {
        if self.cancelling.is_some() {
            return Vec::new();
        }
        self.cancelling = Some(by);
        vec![self.event(EventPayload::Cancel(Cancel {
            requested_by: by,
            ack: false,
            raw: None,
        }))]
    }

    pub(crate) fn finish(
        &mut self,
        result: Option<&Value>,
        failure: Option<String>,
    ) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        let output = result
            .and_then(|r| r.pointer("/usage/outputTokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let failure=failure.or_else(||{
            if self.cancelling.is_none() && result.is_some_and(|r|r["stopReason"]=="end_turn") && self.text.trim().is_empty() && !self.saw_tool && output==0 {
                Some("OpenCode 没有返回文本或工具调用，可能是模型额度耗尽或认证问题，请检查 OpenCode。".to_owned())
            } else {None}
        });
        for (id, call) in std::mem::take(&mut self.calls) {
            events.push(self.event(EventPayload::ToolResult(ToolResult {
                call_id: CallId::parse(id).expect("validated call id"),
                status: ToolResultStatus::Error,
                duration_ms: call.start.elapsed().as_millis().min(u64::MAX as u128) as u64,
                output_hash: None,
                raw: Some(call.raw),
            })));
        }
        if let Some(usage) = result.and_then(|r| r.get("usage")) {
            events.push(self.event(EventPayload::Usage(Usage {
                input_tokens: usage["inputTokens"].as_u64().unwrap_or(0),
                output_tokens: output,
                cached_input_tokens: usage["cachedReadTokens"].as_u64(),
                cost_usd: None,
                raw: Some(usage.clone()),
            })));
        }
        if let Some(by) = self.cancelling {
            events.push(self.event(EventPayload::Cancel(Cancel {
                requested_by: by,
                ack: true,
                raw: None,
            })));
        }
        let outcome = if let Some(message) = failure {
            events.push(self.event(EventPayload::Error(ErrorEvent {
                code: ErrorCode::Protocol,
                message,
                fatal: true,
                raw: result.cloned(),
            })));
            TurnOutcome::Failed
        } else if self.cancelling.is_some()
            || result.is_some_and(|r| r["stopReason"] == "cancelled")
        {
            TurnOutcome::Cancelled
        } else if result.is_some_and(|r| r["stopReason"] == "end_turn") {
            TurnOutcome::Completed
        } else {
            TurnOutcome::Failed
        };
        let hash = (!self.text.is_empty())
            .then(|| format!("sha256:{:x}", Sha256::digest(self.text.as_bytes())));
        events.push(self.event(EventPayload::TurnEnd(TurnEnd {
            outcome,
            final_text_ref: hash,
            raw: result.cloned(),
        })));
        events
    }
}

pub(crate) fn native_name(value: &Value) -> &'static str {
    match value["kind"].as_str().unwrap_or("") {
        "execute" => "shell",
        "edit" | "delete" | "move" => "file_change",
        "read" | "search" => "file_read",
        _ => match value["title"].as_str().unwrap_or("") {
            "bash" | "shell" => "shell",
            "write" | "edit" | "delete" | "move" | "apply_patch" => "file_change",
            "read" | "glob" | "grep" | "list" => "file_read",
            _ => "unknown",
        },
    }
}

fn business_status(value: &Value) -> ToolResultStatus {
    let output = value.pointer("/rawOutput/output");
    let parsed = output
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let status = parsed
        .as_ref()
        .or(output)
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("ok");
    serde_json::from_value(json!(status)).unwrap_or(ToolResultStatus::Error)
}
