//! `workspace.search`: a thin wrapper over the existing unified search
//! (`SearchRegistry::with_defaults()` + `search()`), exactly as the desktop
//! `search_workspace` command runs it. No second matcher lives here.

use mathscape_agent::{Scope, ToolError, ToolErrorCode, ToolStatus};
use mathscape_query::search::{
    MAX_GROUP_LIMIT, MAX_QUERY_CHARS, SearchKind, SearchQuery, SearchRegistry, search,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::canonical::CanonicalIndex;
use super::{WORKSPACE_SEARCH, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

const DESCRIPTION: &str = "Search the Mathscape workspace (notes, source units, materials, snapshots) with the same unified matcher as the desktop search box. Returns hits grouped by kind; highlights are UTF-16 [start,end) offsets into each hit title. Material hits that belong to a canonical markdown view include canonical_id, coverage and partial. Read-only; this tool never runs Marker.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    kinds: Vec<SearchKind>,
    #[serde(default)]
    scope: Scope,
    #[serde(default)]
    include_trashed: bool,
    #[serde(default)]
    limit_per_group: Option<usize>,
}

pub(super) fn definition() -> Value {
    json!({
        "name": WORKSPACE_SEARCH,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": format!("Search text; truncated to {MAX_QUERY_CHARS} characters."),
                },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["note", "source_unit", "material", "snapshot"] },
                    "description": "Restrict to these kinds; empty or omitted means all.",
                },
                "scope": {
                    "type": "string",
                    "enum": ["accepted", "investigation"],
                    "default": "accepted",
                    "description": "Phase1 implements only accepted; investigation is answered as accepted with a scope_not_supported diagnostic.",
                },
                "include_trashed": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include trashed notes and their snapshots.",
                },
                "limit_per_group": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_GROUP_LIMIT,
                    "description": "Maximum hits per kind group; clamped by the search engine.",
                },
            },
            "required": ["query"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: SearchArgs = parse_args(arguments)?;
    let mut diagnostics = Vec::new();
    if args.scope == Scope::Investigation {
        diagnostics.push(diagnostic(
            "scope_not_supported",
            "scope=investigation is not implemented in Phase1; answered with scope=accepted",
        ));
    }
    let query = SearchQuery {
        text: args.query,
        include_trashed: args.include_trashed,
        kinds: args.kinds,
        limit_per_group: args.limit_per_group,
    };

    let corpus = workspace.store()?.read_search_corpus().map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to read search corpus: {error}"),
        )
    })?;
    let mut response = search(&SearchRegistry::with_defaults(), &corpus, &query);
    let index = CanonicalIndex::load(workspace)?;
    for group in &mut response.groups {
        for hit in &mut group.hits {
            // Folded 「规范解析正文」 hits carry the canonical id itself.
            let view = index
                .by_id(&hit.id)
                .or_else(|| index.for_pdf(&hit.id, None))
                .or_else(|| index.for_markdown(&hit.id));
            if let Some(view) = view {
                let coverage = view.coverage(None);
                hit.canonical_id = Some(view.canonical_id.to_string());
                hit.partial = Some(coverage.partial);
                hit.coverage = Some(coverage);
            }
        }
    }

    let mut result = to_object(&response)?;
    result.insert(
        "status".into(),
        serde_json::to_value(ToolStatus::Ok).unwrap_or_default(),
    );
    result.insert("scope".into(), json!("accepted"));
    if !diagnostics.is_empty() {
        result.insert("diagnostics".into(), Value::Array(diagnostics));
    }
    Ok(Value::Object(result))
}
