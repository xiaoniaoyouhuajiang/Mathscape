//! Phase0 agent contract for Mathscape (`agent_events v0` + `mcp_envelope v0`).
//!
//! The single source of truth is `docs/phase0-contract.md`. This crate only
//! freezes *how things talk*: MCP envelope DTOs, the `AgentExecutor` event
//! stream, backend identity, and external capability probing. It contains no
//! business logic, no MCP server and no real harness adapter; adapters for
//! Codex / Claude Code / hmz arrive in Phase1+ and implement [`AgentExecutor`].

mod backend;
mod capability;
mod envelope;
mod event;
mod executor;
mod fake;
mod id;
mod jsonl;
mod validate;

pub use backend::AgentBackend;
pub use capability::{
    Capability, CapabilityProbe, CapabilityState, CommandOutput, CommandRunner,
    SystemCommandRunner, probe_all, probe_capability,
};
pub use envelope::{
    Locator, MCP_ENVELOPE_VERSION, Scope, ToolError, ToolErrorCode, ToolErrorResponse, ToolStatus,
};
pub use event::{
    AGENT_EVENTS_VERSION, AgentEvent, Cancel, CancelRequester, ErrorCode, ErrorEvent, EventPayload,
    ToolCall, ToolResult, ToolResultStatus, TurnEnd, TurnOutcome, TurnStart, Usage,
};
pub use executor::{AgentError, AgentExecutor, StartRoundRequest, UnimplementedExecutor};
pub use fake::{FakeBackend, scripted_round};
pub use id::{CallId, IdError, RoundId, TurnId};
pub use jsonl::{JsonlError, from_jsonl_line, read_jsonl, to_jsonl_line, write_jsonl};
pub use validate::{ValidationIssue, validate_round};
