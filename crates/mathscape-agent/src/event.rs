//! `AgentEvent` stream DTOs (contract §3, `agent_events v0`).
//!
//! Wire shape (one JSON object per JSONL line):
//!
//! ```json
//! {"round_id":"r1","turn_id":"t1","seq":3,"ts":"2026-10-27T15:00:00+08:00",
//!  "backend":"codex","type":"tool_call",
//!  "payload":{"call_id":"c1","tool":"workspace.search","server":"mathscape","args_summary":"query=紧集"}}
//! ```
//!
//! Common fields are `{type, round_id, turn_id?, seq, ts, backend}`; the
//! type-specific fields live under `payload`, which may carry an optional
//! `raw` value preserved verbatim from the harness. `turn_id` is a common
//! field only (not repeated inside the payload) and is required for
//! `turn_start` / `turn_end` — see [`crate::validate_round`].

use std::fmt;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentBackend, CallId, RoundId, ToolStatus, TurnId};

pub const AGENT_EVENTS_VERSION: &str = "agent_events v0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub round_id: RoundId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    /// Strictly increasing within a round.
    pub seq: u64,
    /// RFC3339 with an explicit offset.
    pub ts: DateTime<FixedOffset>,
    pub backend: AgentBackend,
    #[serde(flatten)]
    pub payload: EventPayload,
}

impl AgentEvent {
    pub fn type_name(&self) -> &'static str {
        self.payload.type_name()
    }
}

/// The seven `agent_events v0` types. Adapters must not invent new types;
/// harness-specific data goes in a payload's `raw`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum EventPayload {
    TurnStart(TurnStart),
    TurnEnd(TurnEnd),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    Usage(Usage),
    Cancel(Cancel),
    Error(ErrorEvent),
}

impl EventPayload {
    pub const TYPE_NAMES: [&'static str; 7] = [
        "turn_start",
        "turn_end",
        "tool_call",
        "tool_result",
        "usage",
        "cancel",
        "error",
    ];

    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::TurnStart(_) => "turn_start",
            Self::TurnEnd(_) => "turn_end",
            Self::ToolCall(_) => "tool_call",
            Self::ToolResult(_) => "tool_result",
            Self::Usage(_) => "usage",
            Self::Cancel(_) => "cancel",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnStart {
    /// Hash or reference of the turn's input bundle.
    pub input_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnEnd {
    pub outcome: TurnOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_text_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub call_id: CallId,
    /// Tool name as the server knows it, e.g. `workspace.search`.
    pub tool: String,
    /// `mathscape` or the external MCP server name.
    pub server: String,
    pub args_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// `tool_result.status`: a [`ToolStatus`] or `error` (call error).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Ok,
    NotFound,
    Ambiguous,
    Truncated,
    CapabilityUnavailable,
    Staged,
    Blocked,
    Error,
}

impl From<ToolStatus> for ToolResultStatus {
    fn from(status: ToolStatus) -> Self {
        match status {
            ToolStatus::Ok => Self::Ok,
            ToolStatus::NotFound => Self::NotFound,
            ToolStatus::Ambiguous => Self::Ambiguous,
            ToolStatus::Truncated => Self::Truncated,
            ToolStatus::CapabilityUnavailable => Self::CapabilityUnavailable,
            ToolStatus::Staged => Self::Staged,
            ToolStatus::Blocked => Self::Blocked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub call_id: CallId,
    pub status: ToolResultStatus,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Token usage; field names follow `mathscape_llm::LlmUsage`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelRequester {
    User,
    System,
}

/// `ack=false` is the request; `ack=true` confirms the harness stopped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub requested_by: CancelRequester,
    pub ack: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BackendUnavailable,
    Auth,
    Protocol,
    Internal,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackendUnavailable => "backend_unavailable",
            Self::Auth => "auth",
            Self::Protocol => "protocol",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `fatal=true` must be followed by `turn_end(failed)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorEvent {
    pub code: ErrorCode,
    pub message: String,
    pub fatal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ts() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-27T15:00:00+08:00").unwrap()
    }

    #[test]
    fn tool_call_wire_shape() {
        let event = AgentEvent {
            round_id: RoundId::parse("r1").unwrap(),
            turn_id: Some(TurnId::parse("t1").unwrap()),
            seq: 3,
            ts: ts(),
            backend: AgentBackend::Codex,
            payload: EventPayload::ToolCall(ToolCall {
                call_id: CallId::parse("c1").unwrap(),
                tool: "workspace.search".into(),
                server: "mathscape".into(),
                args_summary: "query=紧集".into(),
                raw: None,
            }),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            json!({
                "type": "tool_call",
                "round_id": "r1",
                "turn_id": "t1",
                "seq": 3,
                "ts": "2026-10-27T15:00:00+08:00",
                "backend": "codex",
                "payload": {
                    "call_id": "c1",
                    "tool": "workspace.search",
                    "server": "mathscape",
                    "args_summary": "query=紧集"
                }
            })
        );
        assert_eq!(serde_json::from_value::<AgentEvent>(value).unwrap(), event);
    }

    #[test]
    fn rejects_unknown_type_and_payload_fields() {
        let base = json!({
            "type": "message_delta", "round_id": "r1", "seq": 0,
            "ts": "2026-10-27T15:00:00+08:00", "backend": "custom", "payload": {}
        });
        assert!(serde_json::from_value::<AgentEvent>(base).is_err());
        let extra = json!({
            "type": "cancel", "round_id": "r1", "seq": 0,
            "ts": "2026-10-27T15:00:00+08:00", "backend": "custom",
            "payload": {"requested_by": "user", "ack": false, "surprise": 1}
        });
        assert!(serde_json::from_value::<AgentEvent>(extra).is_err());
    }

    #[test]
    fn raw_is_preserved() {
        let value = json!({
            "type": "error", "round_id": "r1", "turn_id": "t1", "seq": 9,
            "ts": "2026-10-27T07:00:00Z", "backend": "claude_code",
            "payload": {"code": "protocol", "message": "bad frame", "fatal": true,
                        "raw": {"native": ["x", 1]}}
        });
        let event: AgentEvent = serde_json::from_value(value).unwrap();
        let EventPayload::Error(error) = &event.payload else {
            panic!("expected error payload");
        };
        assert_eq!(error.raw, Some(json!({"native": ["x", 1]})));
        assert_eq!(event.ts.offset().local_minus_utc(), 0);
    }

    #[test]
    fn tool_result_status_accepts_error() {
        assert_eq!(
            serde_json::to_value(ToolResultStatus::from(ToolStatus::CapabilityUnavailable))
                .unwrap(),
            json!("capability_unavailable")
        );
        assert_eq!(
            serde_json::to_value(ToolResultStatus::Error).unwrap(),
            json!("error")
        );
    }
}
