//! Read-only Mathscape MCP server (Phase1 vertical slice).
//!
//! `mathscape mcp --workspace <path> [--round <token>]` speaks a minimal
//! stdio JSON-RPC 2.0 subset of MCP: `initialize`, `notifications/*`,
//! `ping`, `tools/list` and `tools/call`. Messages are newline-delimited
//! JSON, one per line (MCP stdio transport).
//!
//! Tools (fixed order, see [`tools::TOOL_NAMES`]):
//! - `workspace.search` wraps [`mathscape_query::search::SearchRegistry`];
//! - `material.read` reads Markdown by `md_byte_range` and text-layer PDFs by
//!   `pdf_pages`, capped at [`tools::MAX_READ_BYTES`];
//! - `material.grep` linearly scans markdown bodies of the given artifact ids.
//!
//! Wire envelope follows `docs/phase0-contract.md` §2 and reuses the DTOs of
//! `mathscape-agent` ([`mathscape_agent::ToolStatus`], [`mathscape_agent::Locator`],
//! [`mathscape_agent::ToolError`], [`mathscape_agent::CapabilityProbe`]).
//! Business outcomes (`ok|not_found|ambiguous|truncated|capability_unavailable`)
//! are normal tool results; call errors (`invalid_arg|forbidden|internal`)
//! are tool results with `isError: true` and `{ "error": { … } }` content.

mod protocol;
pub mod tools;
mod workspace;

pub use protocol::{
    JSONRPC_VERSION, LATEST_PROTOCOL_VERSION, McpServer, SERVER_NAME, SUPPORTED_PROTOCOL_VERSIONS,
};
pub use workspace::{McpWorkspace, WorkspaceOpenError};
