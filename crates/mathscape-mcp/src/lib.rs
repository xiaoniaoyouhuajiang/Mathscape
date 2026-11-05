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
//!   `pdf_pages` (1-based), capped at [`tools::MAX_READ_BYTES`];
//! - `material.grep` linearly scans markdown bodies of the given artifact ids
//!   (a PDF id scans its current canonical markdown head when one exists,
//!   otherwise its native text layer);
//! - `pdf.outline` returns a PDF's embedded bookmarks. `pdf_page_index` is
//!   0-based and is not written back to the workspace;
//! - `graph.get` / `graph.neighborhood` (Phase1c, `docs/phase1c-graph-contract.md`)
//!   resolve one id or run a bounded BFS over the material landscape
//!   projection ([`mathscape_query::project_landscape`], rebuilt per call,
//!   synthetic bridges included). Nodes carry `content_ref` locators for
//!   `material.read`, never body text;
//! - `ink.page_image` (Phase1d-b) renders one ink page — a note's live head,
//!   an ink snapshot, or the snapshot behind an ink evidence span — to a PNG
//!   with [`mathscape_ink_raster`]. Image results carry a text metadata block
//!   plus an `image` block and omit `structuredContent`;
//! - `pdf.page_image` renders one PDF page (1-based, like `pdf_pages`) or a
//!   normalized crop of it to a PNG with the pure-Rust rasterizer behind
//!   [`mathscape_pdf::PdfEngine::render_page`], with the same image result
//!   shape. Read-only: no Marker, no markdown write, no PDF mutation.
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
pub use tools::EnsurePolicy;
pub use workspace::{McpWorkspace, WorkspaceOpenError};
