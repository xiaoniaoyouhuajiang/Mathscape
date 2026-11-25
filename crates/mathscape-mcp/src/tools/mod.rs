//! Tool catalog and dispatch. The order and text of [`tool_definitions`] are
//! part of the contract (§2.1): fixed, independent of the task, so harness
//! prompt caches stay stable. Change them only together with the contract.

mod ask;
mod canonical;
mod ensure;
mod errata;
mod graph;
mod grep;
mod ink;
mod lean;
mod locate;
mod material;
mod outline;
mod pdf_image;
mod propose;
mod read;
mod search;
mod stage;

use mathscape_agent::{ToolError, ToolErrorCode};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::McpWorkspace;

pub use ask::{MAX_OPTION_CHARS, MAX_OPTIONS, MAX_QUESTION_CHARS, MIN_OPTIONS};
pub use ensure::{
    ENSURE_POLICY_VERSION, EnsurePolicy, EnsureReport, EnsureRun, EnsureStatus,
    ensure_canonical_pages,
};
pub use graph::{
    DEFAULT_EDGE_TYPES, DEFAULT_MAX_EDGES, DEFAULT_MAX_NODES, MAX_DEPTH, MAX_EDGES, MAX_NODES,
    MAX_SEEDS, MAX_VISITED, RELATION_TYPES,
};
pub use grep::{MAX_GREP_HITS, MAX_SCAN_BYTES};
pub use ink::{
    CROP_MARGIN, DEFAULT_MAX_BYTES, DEFAULT_MAX_EDGE_PX, FADED_OPACITY, MAX_MAX_BYTES,
    MAX_MAX_EDGE_PX, MAX_NODE_ID_CHARS, MAX_PAGE_INDEX, MIN_MAX_BYTES, MIN_MAX_EDGE_PX,
};
pub use locate::MAX_LOCATE_CANDIDATES;
pub use outline::{
    MAX_DIAGNOSTIC_MESSAGE_CHARS, MAX_OUTLINE_DIAGNOSTICS, MAX_OUTLINE_ENTRIES,
    MAX_OUTLINE_ENTRIES_BYTES, MAX_OUTLINE_TITLE_CHARS,
};
pub use propose::MAX_LIST_CHANGE_SETS;
pub use read::MAX_READ_BYTES;

pub const WORKSPACE_SEARCH: &str = "workspace.search";
pub const MATERIAL_READ: &str = "material.read";
pub const MATERIAL_GREP: &str = "material.grep";
pub const PDF_OUTLINE: &str = "pdf.outline";
pub const GRAPH_GET: &str = "graph.get";
pub const GRAPH_NEIGHBORHOOD: &str = "graph.neighborhood";
pub const INK_PAGE_IMAGE: &str = "ink.page_image";
pub const PDF_PAGE_IMAGE: &str = "pdf.page_image";
pub const MATHLIB_SEARCH: &str = "mathlib.search";
pub const LEAN_CHECK: &str = "lean.check";
pub const ARTIFACT_STAGE: &str = "artifact.stage";
pub const PROPOSE_CHANGES: &str = "propose_changes";
pub const GET_CHANGE_SET: &str = "get_change_set";
pub const MATERIAL_LOCATE: &str = "material.locate";
pub const LIST_CHANGE_SETS: &str = "list_change_sets";
pub const USER_ASK: &str = "user.ask";

/// Version of the `tools/list` table (names, descriptions, input schemas)
/// that the bundled `mathscape-worknet` skill is written against; the skill
/// frontmatter's `tool_schema_version` must equal it. Bump it whenever
/// [`tool_definitions`] changes — `tool_table_fingerprint_is_pinned` fails
/// until the version and the pinned fingerprint are updated together.
///
/// `/14`: `propose_changes` gained `source_span`, `$new:` references and
/// cross-proposal references (PR-B); the table still has 13 tools.
/// `/15`: new read-only `material.locate` (14 tools); `material.read`
/// documents single-snapshot `served_segments` and `page_markers_absent`.
/// `/16`: new read-only `list_change_sets` (15 tools) so a later round can
/// find its own change sets and revise one with `supersedes`;
/// `material.locate` documents `page_markers_absent` / `page_unresolved`;
/// `propose_changes` documents page granularity and `supersedes`.
/// `/17`: `propose_changes` accepts the semantic layer (semantic_entity,
/// alignment, debt, dialect_scope; retire of semantic objects; editable
/// `home_theory` and `aliases`), `workspace.search` searches `theory` and
/// `semantic_entity` (incl. `scope="investigation"`), and `graph.get` /
/// `graph.neighborhood` navigate semantic nodes and relations
/// (docs/semantic-write-contract.md).
/// `/18`: `graph.neighborhood` adds typed-hop `preset` (`heuristic` default
/// when `edge_types` is omitted, plus `structural`, `semantic_only`, `raw`)
/// and projects `uses_entity`, `of_concept`, and `paid_by` relations.
/// `/19`: `propose_changes` accepts `retire` of a `source_relation` (in
/// either array) and reports a misplaced relation create / edit as
/// `wrong_slot`.
/// `/20`: new non-blocking `user.ask` (16 tools) for structured user
/// clarification (`pending_questions` + `next: "end_turn"` + same-round
/// `awaiting_user` guard on `propose_changes`).
/// `/21`: Stage 2 (S2 complete + S3 first half): `propose_changes` supports
/// cohesive `guards` / `branches` / `exclusions`, `target_branches`,
/// `local_symbols`, `representation_core` and `representation_binding`;
/// `workspace.search` adds `representation_core`; `graph.get` /
/// `graph.neighborhood` add `representation_core` and `binds_representation`.
pub const TOOL_SCHEMA_VERSION: &str = "mathscape-mcp/21";

/// sha256 of the serialized [`tool_definitions`] with object keys sorted (so
/// the value does not depend on serde_json's `preserve_order` feature), for
/// pinning the schema.
pub fn tool_table_fingerprint() -> String {
    use sha2::{Digest, Sha256};
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let table = Value::Array(tool_definitions().iter().map(sorted).collect());
    let bytes = serde_json::to_vec(&table).expect("tool definitions serialize");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stable `tools/list` order. New tools are appended; existing entries stay put.
pub const TOOL_NAMES: [&str; 16] = [
    WORKSPACE_SEARCH,
    MATERIAL_READ,
    MATERIAL_GREP,
    PDF_OUTLINE,
    GRAPH_GET,
    GRAPH_NEIGHBORHOOD,
    INK_PAGE_IMAGE,
    PDF_PAGE_IMAGE,
    MATHLIB_SEARCH,
    LEAN_CHECK,
    ARTIFACT_STAGE,
    PROPOSE_CHANGES,
    GET_CHANGE_SET,
    MATERIAL_LOCATE,
    LIST_CHANGE_SETS,
    USER_ASK,
];

/// MCP `annotations` for the read-only tools.
///
/// Codex treats a missing `readOnlyHint` as destructive (`destructiveHint`
/// defaults to true). Under `approvalPolicy: never` that becomes
/// "MCP tool call requires approval, but approval policy is never" and the
/// call never runs. The catalog is read-only except for the bounded
/// `material.read` ensure and `artifact.stage` staging exception; see their
/// specific annotations below.
pub fn read_only_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "openWorldHint": false,
    })
}

/// `material.read` may run a bounded Marker ensure on a scanned PDF and add a
/// new immutable snapshot to the workspace parse cache, so it is honestly not
/// read-only. It never rewrites or deletes existing data
/// (`destructiveHint: false`), repeating a call reuses the snapshot
/// (`idempotentHint: true`) and Marker runs locally (`openWorldHint: false`).
pub fn material_read_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

pub fn artifact_stage_annotations() -> Value {
    stage::annotations()
}

fn with_annotations(mut definition: Value) -> Value {
    let annotations = match definition.get("name").and_then(Value::as_str) {
        Some(MATERIAL_READ) => material_read_annotations(),
        Some(ARTIFACT_STAGE) => artifact_stage_annotations(),
        Some(PROPOSE_CHANGES) => propose::propose_annotations(),
        Some(GET_CHANGE_SET) | Some(LIST_CHANGE_SETS) => propose::get_annotations(),
        Some(USER_ASK) => ask::annotations(),
        _ => read_only_annotations(),
    };
    if let Some(object) = definition.as_object_mut() {
        object.insert("annotations".into(), annotations);
    }
    definition
}

/// `tools/list` entries, in [`TOOL_NAMES`] order.
pub fn tool_definitions() -> Vec<Value> {
    [
        search::definition(),
        read::definition(),
        grep::definition(),
        outline::definition(),
        graph::get_definition(),
        graph::neighborhood_definition(),
        ink::definition(),
        pdf_image::definition(),
        lean::search_definition(),
        lean::check_definition(),
        stage::definition(),
        propose::definition(),
        propose::get_definition(),
        locate::definition(),
        propose::list_definition(),
        ask::definition(),
    ]
    .into_iter()
    .map(with_annotations)
    .collect()
}

/// What a successful tool call returns.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolOutput {
    /// The usual Phase0 envelope: the text block mirrors `structured`, which
    /// is also sent as `structuredContent`.
    Structured(Value),
    /// Same envelope with a pre-serialized text block (field order kept).
    Text { text: String, structured: Value },
    /// `ink.page_image` / `pdf.page_image` success (contract Phase1d §3.4):
    /// a metadata text block, then one image block, and **no**
    /// `structuredContent`.
    Image {
        text: String,
        png: Vec<u8>,
        mime_type: &'static str,
    },
}

/// Runs one tool. `None` means the tool name is unknown (a protocol error,
/// not a tool result). `Ok` carries the result with its business `status`;
/// `Err` is a call error (`invalid_arg|forbidden|internal`).
pub fn call(
    workspace: &McpWorkspace,
    name: &str,
    arguments: Value,
) -> Option<Result<ToolOutput, ToolError>> {
    let structured = |result: Result<Value, ToolError>| Some(result.map(ToolOutput::Structured));
    match name {
        WORKSPACE_SEARCH => structured(search::call(workspace, arguments)),
        MATERIAL_READ => structured(read::call(workspace, arguments)),
        MATERIAL_GREP => structured(grep::call(workspace, arguments)),
        PDF_OUTLINE => structured(outline::call(workspace, arguments)),
        GRAPH_GET => structured(graph::call_get(workspace, arguments)),
        GRAPH_NEIGHBORHOOD => structured(graph::call_neighborhood(workspace, arguments)),
        INK_PAGE_IMAGE => Some(ink::call(workspace, arguments)),
        PDF_PAGE_IMAGE => Some(pdf_image::call(workspace, arguments)),
        MATHLIB_SEARCH => structured(lean::search(workspace, arguments)),
        LEAN_CHECK => structured(lean::check(workspace, arguments)),
        ARTIFACT_STAGE => structured(stage::call(workspace, arguments)),
        PROPOSE_CHANGES => structured(propose::propose(workspace, arguments)),
        GET_CHANGE_SET => structured(propose::get(workspace, arguments)),
        MATERIAL_LOCATE => structured(locate::call(workspace, arguments)),
        LIST_CHANGE_SETS => structured(propose::list(workspace, arguments)),
        USER_ASK => structured(ask::call(workspace, arguments)),
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

#[cfg(test)]
mod schema_version_tests {
    use super::*;

    /// The tool table a given [`TOOL_SCHEMA_VERSION`] stands for. Changing a
    /// tool name, description or schema changes the fingerprint: bump
    /// `TOOL_SCHEMA_VERSION` (and the skill frontmatter) and re-pin.
    const PINNED: (&str, &str) = (
        "mathscape-mcp/21",
        "1ff8d0e5607165909bcc8c70742c1e5c59f06820deb5cb2bea82ef2e5c00f87e",
    );

    #[test]
    fn tool_table_fingerprint_is_pinned() {
        assert_eq!(
            (TOOL_SCHEMA_VERSION, tool_table_fingerprint().as_str()),
            PINNED,
            "the MCP tool table changed: bump TOOL_SCHEMA_VERSION and re-pin"
        );
    }
}
