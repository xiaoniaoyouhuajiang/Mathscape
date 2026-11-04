//! Minimal MCP stdio server: newline-delimited JSON-RPC 2.0.
//!
//! Implemented methods: `initialize`, `notifications/*` (ignored), `ping`,
//! `tools/list`, `tools/call`. Everything else is `-32601`. Tool business
//! statuses and call errors travel inside a normal `tools/call` result; only
//! protocol faults (bad JSON, invalid envelope, unknown method/tool,
//! malformed params) are JSON-RPC errors.

use std::io::{self, BufRead, Write};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use crate::McpWorkspace;
use crate::tools::{self, ToolOutput};

pub const JSONRPC_VERSION: &str = "2.0";
pub const SERVER_NAME: &str = "mathscape";
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
/// Protocol versions this subset actually implements. A client asking for one
/// of these gets it echoed back; anything else, including a newer date this
/// server does not implement, gets [`LATEST_PROTOCOL_VERSION`].
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

pub struct McpServer {
    workspace: McpWorkspace,
}

impl McpServer {
    pub fn new(workspace: McpWorkspace) -> Self {
        Self { workspace }
    }

    pub fn workspace(&self) -> &McpWorkspace {
        &self.workspace
    }

    /// Serves requests until `reader` reaches EOF. Each response is written
    /// as one line and flushed immediately.
    pub fn serve(&self, reader: impl BufRead, mut writer: impl Write) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(response) = self.handle_line(&line) {
                serde_json::to_writer(&mut writer, &response)?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Handles one raw JSON-RPC line; `None` for notifications.
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle_message(message),
            Err(error) => Some(error_response(
                Value::Null,
                PARSE_ERROR,
                format!("parse error: {error}"),
            )),
        }
    }

    /// Handles one decoded JSON-RPC message.
    ///
    /// Returns `None` for notifications and for client responses (an `id`
    /// and no `method` — this server never sends requests, so it must not
    /// answer those). Invalid envelopes are `-32600` with a null id.
    pub fn handle_message(&self, message: Value) -> Option<Value> {
        let Value::Object(message) = message else {
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "batch and non-object messages are not supported",
            ));
        };
        // A client response has an id and no method. Do not reply, even when
        // the envelope is otherwise wrong.
        if message.contains_key("id") && !message.contains_key("method") {
            return None;
        }
        if message.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
            return Some(invalid_request("jsonrpc must be \"2.0\""));
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Some(invalid_request("method must be a string"));
        };
        let Some(id) = message.get("id") else {
            // Notifications (`notifications/initialized`, `…/cancelled`) need no reply.
            return None;
        };
        if !is_request_id(id) {
            // JSON-RPC allows string, number, or null; MCP requests must not use null.
            return Some(invalid_request("request id must be a string or number"));
        }
        let id = id.clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.dispatch(method, params) {
            Ok(result) => json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "result": result }),
            Err((code, text)) => error_response(id, code, text),
        })
    }

    fn dispatch(&self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::tool_definitions() })),
            "tools/call" => self.call_tool(params),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }

    fn call_tool(&self, params: Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).ok_or((
            INVALID_PARAMS,
            "tools/call requires a string `name`".to_owned(),
        ))?;
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let outcome = tools::call(&self.workspace, name, arguments)
            .ok_or_else(|| (INVALID_PARAMS, format!("unknown tool: {name}")))?;
        let (text, structured, is_error) = match outcome {
            Ok(ToolOutput::Structured(result)) => (result.to_string(), result, false),
            Ok(ToolOutput::Text { text, structured }) => (text, structured, false),
            Ok(ToolOutput::Image {
                text,
                png,
                mime_type,
            }) => {
                // Image results carry no structuredContent: MCP clients that
                // see it drop content[] and the image with it.
                return Ok(json!({
                    "content": [
                        { "type": "text", "text": text },
                        { "type": "image", "data": BASE64.encode(png), "mimeType": mime_type },
                    ],
                    "isError": false,
                }));
            }
            Err(error) => {
                let structured = serde_json::to_value(error.into_response()).unwrap_or_else(|_| {
                    json!({ "error": { "code": "internal", "message": "failed to encode error" } })
                });
                (structured.to_string(), structured, true)
            }
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": structured,
            "isError": is_error,
        }))
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Mathscape workspace tools (read-only except that material.read may parse scanned PDF pages). Use workspace.search to find materials, material.grep for coarse location inside known artifact ids (markdown bodies, or a PDF's native text layer: PDF hits carry surface pdf_text_layer and a pdf_pages locator of the matching page; their snippets are not canonical text), pdf.outline for a PDF table of contents (pdf_page_index is 0-based; nothing is persisted), then material.read with a locator to quote exact passages (pdf_pages is 1-based). When a canonical markdown view exists, those tools include canonical_id, coverage and partial; partial true means the request or the book is not fully covered. material.read of a PDF always returns canonical Marker markdown, never the raw text layer, even when the pages have one: it serves an immutable markdown snapshot that contains every requested page; if none does, the same call runs Marker locally on just those pages and waits for it (synchronous, may take minutes; keep ranges small, at most 12 pages), then returns the markdown. A read of a PDF that is already being parsed waits for that parse. details.ensure.status is reused or expanded on success, otherwise blocked_budget, capability_unavailable or failed (with a reason; nothing is left running), and no text-layer fallback is returned. Never poll or re-read to wait for parsing. Search, grep and outline never run Marker. graph.get and graph.neighborhood navigate the material landscape: a read-time projection of artifacts, source spans, source units, ink snapshots and notes, including synthetic bridges (contains_span unit->span, has_part parent->child, located_in span->artifact, has_snapshot note->snapshot, note_link note->linked target) next to conversion and semantic relations. Graph nodes carry no body text; pass content_ref.artifact_id and content_ref.locator to material.read unchanged. Recipes: R1 search hit -> graph.get -> material.read(content_ref), and if content_ref is null read content_unavailable.reason instead of guessing byte ranges; R2 graph.neighborhood(node_id, depth=1) with defaults for structural neighbors; R3 proofs are opt-in, e.g. edge_types=[contains_span, has_part, proves, states], depth=2; R4 add cites only when crossing works, keep depth <= 2 with an explicit max_nodes, and on truncated narrow edge_types or depth before raising caps; R5 a Markdown artifact's units: direction=in, depth=2, node_kinds=[source_unit] (unit->artifact is direction=out; from a PDF use direction=both, depth=3); R6 notes, snapshots and ink spans: content_ref.type=ink_page -> call ink.page_image(content_ref.node_id, page_id); notes link to sources via note_link (opt-in edge) and to frozen evidence via has_snapshot (default edge). Read relation endpoints (role, side), not only from/to, which are null for multi-endpoint relations.",
    })
}

fn invalid_request(message: impl Into<String>) -> Value {
    error_response(Value::Null, INVALID_REQUEST, message)
}

/// Request ids are JSON strings or numbers. Null, bool, object, and array are rejected.
fn is_request_id(id: &Value) -> bool {
    matches!(id, Value::String(_) | Value::Number(_))
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}
