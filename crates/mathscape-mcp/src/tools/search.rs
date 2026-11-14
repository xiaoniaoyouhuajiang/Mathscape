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
use crate::investigation::{investigation_overlay, overlay_search_corpus};

const DESCRIPTION: &str = "Search the Mathscape workspace (notes, source units, theories, shared objects, materials, snapshots) with the same unified matcher as the desktop search box. Returns hits grouped by kind; highlights are UTF-16 [start,end) offsets into each hit title. Material hits that belong to a canonical markdown view include canonical_id, coverage and partial. Use kinds=[\"theory\"] (often with scope=\"investigation\") to find existing or staged theories before proposing a new one. Read-only; this tool never runs Marker.";

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
                    "items": { "type": "string", "enum": ["note", "source_unit", "theory", "semantic_entity", "material", "snapshot"] },
                    "description": "Restrict to these kinds; empty or omitted means all.",
                },
                "scope": {
                    "type": "string",
                    "enum": ["accepted", "investigation"],
                    "default": "accepted",
                    "description": "accepted searches the formal map; investigation also includes active staged source units, theories and shared objects of the current investigation.",
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

fn projected_investigation(workspace: &McpWorkspace, scope: Scope) -> Option<String> {
    if scope != Scope::Investigation {
        return None;
    }
    if let Some(investigation) = workspace.investigation() {
        return Some(investigation.to_owned());
    }
    let round = workspace.round().filter(|round| !round.trim().is_empty())?;
    workspace
        .store()
        .ok()?
        .round_investigation(&round)
        .ok()
        .flatten()
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: SearchArgs = parse_args(arguments)?;
    let mut diagnostics = Vec::new();
    let investigation = projected_investigation(workspace, args.scope);
    if args.scope == Scope::Investigation && investigation.is_none() {
        diagnostics.push(diagnostic(
            "scope_not_supported",
            "no investigation is bound to this session; answered with scope=accepted",
        ));
    }
    let query = SearchQuery {
        text: args.query,
        include_trashed: args.include_trashed,
        kinds: args.kinds,
        limit_per_group: args.limit_per_group,
    };

    let store = workspace.store()?;
    let mut corpus = store.read_search_corpus().map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to read search corpus: {error}"),
        )
    })?;
    if let Some(investigation_id) = investigation.as_deref() {
        let overlay = investigation_overlay(&store, investigation_id).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to project the investigation: {error}"),
            )
        })?;
        overlay_search_corpus(&mut corpus, &overlay);
    }
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
    result.insert(
        "scope".into(),
        json!(if investigation.is_some() {
            "investigation"
        } else {
            "accepted"
        }),
    );
    if !diagnostics.is_empty() {
        result.insert("diagnostics".into(), Value::Array(diagnostics));
    }
    Ok(Value::Object(result))
}
