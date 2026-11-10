//! Local Codex app-server adapter ([`CodexExecutor`]).
//!
//! Mathscape launches the user's Codex CLI (`codex app-server` by default) and
//! speaks the documented JSON-RPC subset. It does not store, refresh, or proxy
//! a ChatGPT subscription: the child process inherits the environment so the
//! CLI can use its own vendor login. MCP tools need no model credentials.
//!
//! hmz is not consulted. [`crate::FakeBackend`] and [`crate::UnimplementedExecutor`]
//! stay available for tests and for backends that still have no adapter.
//!
//! # Event flow
//!
//! `start_round` probes the configured binary with `--version` (no network),
//! then:
//!
//! 1. `initialize` / `initialized`
//! 2. `thread/start` with `cwd`, `sandbox: "read-only"`, `approvalPolicy: on-request`,
//!    `ephemeral: true`, and dotted `mcp_servers.mathscape.*` overrides so
//!    Codex spawns `mathscape mcp --workspace … --round …`. The mathscape
//!    server is whitelisted (`enabled_tools`) with
//!    `default_tools_approval_mode = approve`. `never` is not used: it
//!    rejects MCP calls Codex considers approval-gated.
//! 3. `turn/start` whose user text is [`crate::StartRoundRequest::input_ref`]
//!    (the contract has no separate prompt field)
//!
//! Notifications become `agent_events v0`. An `mcpToolCall` item is the mirror
//! of an MCP `tools/call`: `item/started` → `tool_call`, `item/completed` →
//! `tool_result` (business `status` taken from `structuredContent` when present).
//! `thread/tokenUsage/updated` → `usage`. `turn/completed` closes the turn.
//! `cancel` writes `turn/interrupt`.
//!
//! # Process lifecycle
//!
//! On Unix the app-server is spawned as its own process-group leader. Shutdown
//! (end of turn, failed handshake, or `Drop`) closes stdin, sends SIGTERM to
//! the whole group, then SIGKILL after a short grace, so the native process
//! behind npm's node wrapper and its `mathscape mcp` child do not outlive the
//! round.
//!
//! Shell and file-change items are mirrored as `tool_call` / `tool_result`
//! with stable names `shell` and `file_change` (server `codex`) so the
//! dialogue can show them. Approval requests for those side effects are
//! declined. Read-only Mathscape MCP elicitations are accepted. An approval
//! this adapter cannot classify is held and auto-declined after 60s with
//! 「等待审批已超时」. `user.ask` is not implemented.
//!
//! # Dry run
//!
//! [`serve_codex_app_server_fixture`] is a stdio peer that speaks this subset
//! without a Codex login. The `mathscape-codex-fixture` binary is the same
//! peer for subprocess tests. A real login is `#[ignore]` and
//! `MATHSCAPE_CODEX_LIVE=1`.
//!
//! # TODOs
//!
//! - Split the user prompt from `input_ref` when the contract grows a field.
//! - One turn per `start_round`; multi-turn steering is not exposed.
//! - Interactive approval cards (allow once) are not in S1; shell and writes stay declined.
//! - Confirm dotted `thread/start` `config` overrides against the installed
//!   Codex version (`load_for_cwd` uses `mcp_servers.<name>.command`).

mod approval;
mod fixture;
mod host;
mod map;
mod model;
mod rollout;
mod session;

pub use fixture::{
    CodexFixtureMcp, CodexFixtureMode, CodexFixtureOptions, serve_codex_app_server_fixture,
    serve_codex_app_server_fixture_with,
};
pub use session::{CodexConfig, CodexExecutor};
