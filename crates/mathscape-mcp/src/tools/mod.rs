//! Tool catalog and dispatch. The order and text of [`tool_definitions`] are
//! part of the contract (§2.1): fixed, independent of the task, so harness
//! prompt caches stay stable. Change them only together with the contract.

mod grep;
mod read;
mod search;

use mathscape_agent::{ToolError, ToolErrorCode};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::McpWorkspace;

pub use grep::{MAX_GREP_HITS, MAX_SCAN_BYTES};
pub use read::MAX_READ_BYTES;

pub const WORKSPACE_SEARCH: &str = "workspace.search";
pub const MATERIAL_READ: &str = "material.read";
pub const MATERIAL_GREP: &str = "material.grep";

/// Stable `tools/list` order. New tools are appended; existing entries stay put.
pub const TOOL_NAMES: [&str; 3] = [WORKSPACE_SEARCH, MATERIAL_READ, MATERIAL_GREP];

/// `tools/list` entries, in [`TOOL_NAMES`] order.
pub fn tool_definitions() -> Vec<Value> {
    vec![search::definition(), read::definition(), grep::definition()]
}

/// Runs one tool. `None` means the tool name is unknown (a protocol error,
/// not a tool result). `Ok` carries the structured result with its business
/// `status`; `Err` is a call error (`invalid_arg|forbidden|internal`).
pub fn call(
    workspace: &McpWorkspace,
    name: &str,
    arguments: Value,
) -> Option<Result<Value, ToolError>> {
    match name {
        WORKSPACE_SEARCH => Some(search::call(workspace, arguments)),
        MATERIAL_READ => Some(read::call(workspace, arguments)),
        MATERIAL_GREP => Some(grep::call(workspace, arguments)),
        _ => None,
    }
}

/// `{ code, message }` diagnostic entry attached to a result.
pub(crate) fn diagnostic(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

/// Missing / null arguments are treated as `{}`; anything that does not
/// match the input DTO is `invalid_arg`.
pub(crate) fn parse_args<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    let arguments = match arguments {
        Value::Null => Value::Object(Map::new()),
        Value::Object(_) => arguments,
        _ => {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "tool arguments must be a JSON object",
            ));
        }
    };
    serde_json::from_value(arguments).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("invalid arguments: {error}"),
        )
    })
}

/// Serializes a result DTO into a JSON object.
pub(crate) fn to_object(value: impl serde::Serialize) -> Result<Map<String, Value>, ToolError> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(ToolError::new(
            ToolErrorCode::Internal,
            "tool result did not serialize to an object",
        )),
        Err(error) => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to serialize tool result: {error}"),
        )),
    }
}
