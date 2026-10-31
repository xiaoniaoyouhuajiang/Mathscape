//! Phase0 agent contract for Mathscape (`agent_events v0` + `mcp_envelope v0`).
//!
//! The single source of truth is `docs/phase0-contract.md`. This crate only
//! freezes *how things talk*: MCP envelope DTOs, the `AgentExecutor` event
//! stream, backend identity, and external capability probing. It contains no
//! business orchestration. [`CodexExecutor`] is the Phase1b local Codex
//! app-server adapter (BYO vendor login). Claude Code and optional hmz still
//! have no adapter; hmz is not required for any default path.

mod backend;
mod capability;
mod codex;
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
    SystemCommandRunner, probe_all, probe_capability, probe_program,
};
pub use codex::{
    CodexConfig, CodexExecutor, CodexFixtureMcp, CodexFixtureMode, CodexFixtureOptions,
    serve_codex_app_server_fixture, serve_codex_app_server_fixture_with,
};
pub use envelope::{
    Locator, MCP_ENVELOPE_VERSION, Scope, ToolError, ToolErrorCode, ToolErrorResponse, ToolStatus,
};
pub use event::{
    AGENT_EVENTS_VERSION, AgentEvent, Cancel, CancelRequester, ErrorCode, ErrorEvent, EventPayload,
    ToolCall, ToolResult, ToolResultStatus, TurnEnd, TurnOutcome, TurnStart, Usage,
};
pub use executor::{
    AgentEffort, AgentError, AgentExecutor, AgentModel, StartRoundRequest, UnimplementedExecutor,
};
pub use fake::{FakeBackend, scripted_round};
pub use id::{CallId, IdError, RoundId, TurnId};
pub use jsonl::{JsonlError, from_jsonl_line, read_jsonl, to_jsonl_line, write_jsonl};
pub use validate::{ValidationIssue, validate_round};
