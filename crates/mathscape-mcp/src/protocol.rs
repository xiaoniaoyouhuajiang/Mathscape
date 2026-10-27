//! Minimal MCP stdio server: newline-delimited JSON-RPC 2.0.
//!
//! Implemented methods: `initialize`, `notifications/*` (ignored), `ping`,
//! `tools/list`, `tools/call`. Everything else is `-32601`. Tool business
//! statuses and call errors travel inside a normal `tools/call` result; only
//! protocol faults (bad JSON, unknown method/tool, malformed params) are
//! JSON-RPC errors.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::McpWorkspace;
use crate::tools;

pub const JSONRPC_VERSION: &str = "2.0";
pub const SERVER_NAME: &str = "mathscape";
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
/// Protocol versions this subset is compatible with; a client asking for one
/// of these gets it echoed back, anything else gets [`LATEST_PROTOCOL_VERSION`].
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

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

    /// Handles one decoded JSON-RPC message; `None` for notifications.
    pub fn handle_message(&self, message: Value) -> Option<Value> {
        let Value::Object(message) = message else {
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "batch and non-object messages are not supported",
            ));
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response from the client (we never send requests) or garbage.
            return id.map(|id| error_response(id, INVALID_REQUEST, "missing method"));
        };
        let Some(id) = id else {
            // Notifications (`notifications/initialized`, `…/cancelled`) need no reply.
            return None;
        };
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
        let (structured, is_error) = match outcome {
            Ok(result) => (result, false),
            Err(error) => (
                serde_json::to_value(error.into_response()).unwrap_or_else(|_| {
                    json!({ "error": { "code": "internal", "message": "failed to encode error" } })
                }),
                true,
            ),
        };
        Ok(json!({
            "content": [{ "type": "text", "text": structured.to_string() }],
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
        "instructions": "Read-only Mathscape workspace tools. Use workspace.search to find materials, then material.read with a locator to quote exact passages.",
    })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}
