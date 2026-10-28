//! `graph.get` / `graph.neighborhood`: thin read-only navigation over the
//! material landscape projection (`docs/phase1c-graph-contract.md`).
//!
//! Every call rebuilds the projection in memory from
//! `SqliteArtifactStore::landscape_snapshot()` with
//! [`mathscape_query::project_landscape`]; nothing is cached or written. The
//! projection already contains the synthetic bridges (`contains_span`,
//! `has_part` from `parent_unit`, `located_in`), so the tools add no edges of
//! their own. Notes are not landscape nodes: they are resolved from the ink
//! store as isolated nodes.
//!
//! Nodes never carry body text. Instead they carry `parent_artifact_id` and a
//! `content_ref` that can be passed to `material.read` unchanged.
//!
//! Traversal (`graph.neighborhood`) is a BFS over entity hops: a relation is a
//! hyperedge that does not consume depth but counts toward `max_edges`.
//! `unknown` relations are never traversed or returned (fail-closed).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use mathscape_agent::{Scope, ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::{EntityKind, LifecycleStatus};
use mathscape_graph::{GraphProjection, IncidenceDirection, ProjectionRelation};
use mathscape_query::{LandscapeProjectionInput, SourceGraphProjectionError, project_landscape};
use mathscape_store_sqlite::{SourceMapSnapshot, SqliteArtifactStore, StoredMapObject};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::material::{MARKDOWN_MEDIA_TYPE, PDF_MEDIA_TYPE};
use super::{GRAPH_GET, GRAPH_NEIGHBORHOOD, diagnostic, parse_args};
use crate::McpWorkspace;

const GET_DESCRIPTION: &str = "Resolve one id from the material landscape graph (artifacts, source units, source spans, ink snapshots, plus notes from the ink store) without expanding neighbors. Accepts any workspace.search hit id as-is (a snapshot_id is canonicalized to its snapshot artifact id; use canonical_id afterwards) and any graph id (source-span, source-relation, projection-relation, conversion run). Returns kind (artifact|source_span|source_unit|snapshot|note|relation|unsupported), entity_kind, label, lifecycle, parent_artifact_id and content_ref, plus edge_summary counts per relation and direction. No body text: when content_ref is not null, call material.read with content_ref.artifact_id and content_ref.locator unchanged (md_byte_range, or pdf_pages 1-based). content_ref is null with a content_unavailable diagnostic for handwriting (ink_out_of_scope), units without located spans (no_span_locator; do not guess byte ranges), unsupported media, and relations. A missing id is status not_found with node null; an existing id of an unsupported kind is status ok with kind unsupported. Read-only.";

const NEIGHBORHOOD_DESCRIPTION: &str = "Bounded BFS from one node_id or 1-8 seed_ids over the material landscape graph (a read-time projection with synthetic bridges: contains_span unit->span, has_part parent->child, located_in span->artifact, conversion pdf->markdown). Relations are returned in edges[] (each counts toward max_edges, keeps every endpoint with role and side; from/to are null for multi-endpoint relations, so read endpoints) and do not consume depth: unit->span->artifact is 2 hops. depth defaults to 1 (max 3; 0 = seeds only). direction defaults to both; out follows from-side to to-side (unit out -> spans -> artifact), in goes the other way (markdown artifact in -> spans -> units); undirected roles such as evidence are only reached with both. edge_types defaults to the structural four [conversion, located_in, contains_span, has_part]; semantic relations (states, proves, poses, hints, solves, explains, applies_method, cites) are opt-in and must be listed together with any structural ones you still want; [] walks no edges. Prefer depth <= 2 and an explicit max_nodes when adding cites. node_kinds (artifact|source_span|source_unit|snapshot|note) filters the output only; traversal still passes through filtered nodes. max_nodes defaults to 40 (max 100), max_edges to 80 (max 200); values over the hard cap are invalid_arg. Over a cap the result is status truncated with truncated true, shallower layers first, and never a dangling edge. Retired (superseded|rejected|deprecated) and trashed ink nodes are hidden unless include_trashed; include_virtual must be false. Nodes carry depth, via_edge, parent_artifact_id and content_ref for material.read, never body text. Any missing seed makes the whole result not_found. Read-only.";

const SNAPSHOT_MEDIA_TYPE: &str = "application/vnd.mathscape.ink-snapshot+json";
const SYNTHETIC_PREFIX: &str = "mathscape:projection-relation/";
const INK_SNAPSHOT_PREFIX: &str = "mathscape:ink-snapshot/";

pub const DEFAULT_DEPTH: u32 = 1;
pub const MAX_DEPTH: u32 = 3;
pub const DEFAULT_MAX_NODES: usize = 40;
pub const MAX_NODES: usize = 100;
pub const DEFAULT_MAX_EDGES: usize = 80;
pub const MAX_EDGES: usize = 200;
pub const MAX_SEEDS: usize = 8;
/// Entities visited by one BFS, including filtered and hidden ones.
pub const MAX_VISITED: usize = 2000;
/// Bytes offered by an `artifact_head` Markdown window (= `MAX_READ_BYTES`).
const ARTIFACT_HEAD_BYTES: u64 = 32 * 1024;
/// Most dangling-endpoint items listed in one diagnostic.
const MAX_DANGLING_LISTED: usize = 20;

/// Closed relation enum (contract §7), in contract order.
pub const RELATION_TYPES: [&str; 12] = [
    "conversion",
    "located_in",
    "contains_span",
    "has_part",
    "states",
    "proves",
    "poses",
    "hints",
    "solves",
    "explains",
    "applies_method",
    "cites",
];
pub const DEFAULT_EDGE_TYPES: [&str; 4] = ["conversion", "located_in", "contains_span", "has_part"];
const NODE_KINDS: [&str; 5] = ["artifact", "source_span", "source_unit", "snapshot", "note"];
const DIRECTIONS: [&str; 3] = ["both", "in", "out"];

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn scope_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["accepted", "investigation"],
        "default": "accepted",
        "description": "Phase1 implements only accepted; investigation is answered as accepted with a scope_not_supported diagnostic.",
    })
}

pub(super) fn get_definition() -> Value {
    json!({
        "name": GRAPH_GET,
        "description": GET_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "Any workspace.search hit id (material, source_unit, snapshot, note) or graph id.",
                },
                "scope": scope_schema(),
            },
            "required": ["node_id"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn neighborhood_definition() -> Value {
    json!({
        "name": GRAPH_NEIGHBORHOOD,
        "description": NEIGHBORHOOD_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "Single seed. Give exactly one of node_id or seed_ids.",
                },
                "seed_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": MAX_SEEDS,
                    "uniqueItems": true,
                    "description": "1-8 distinct seeds. Give exactly one of node_id or seed_ids.",
                },
                "depth": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_DEPTH,
                    "default": DEFAULT_DEPTH,
                    "description": "Entity hops; relations do not consume depth. 0 returns the seeds only.",
                },
                "direction": {
                    "type": "string",
                    "enum": DIRECTIONS,
                    "default": "both",
                    "description": "out: from-side to to-side; in: the reverse; both also reaches undirected roles.",
                },
                "edge_types": {
                    "type": "array",
                    "items": { "type": "string", "enum": RELATION_TYPES },
                    "default": DEFAULT_EDGE_TYPES,
                    "description": "Relations to traverse and return. Omitted = the structural four; [] = no edges.",
                },
                "node_kinds": {
                    "type": "array",
                    "items": { "type": "string", "enum": NODE_KINDS },
                    "description": "Filter returned nodes (seeds are always returned); traversal is not cut. Omitted = all.",
                },
                "max_nodes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_NODES,
                    "default": DEFAULT_MAX_NODES,
                },
                "max_edges": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_EDGES,
                    "default": DEFAULT_MAX_EDGES,
                },
                "include_trashed": {
                    "type": "boolean",
                    "default": false,
                    "description": "Also traverse superseded/rejected/deprecated objects and trashed ink.",
                },
                "include_virtual": {
                    "type": "boolean",
                    "default": false,
                    "description": "Must be false or omitted; virtual contraction edges are not offered.",
                },
                "scope": scope_schema(),
            },
            "additionalProperties": false,
        },
    })
}

// ---------------------------------------------------------------------------
// Loaded graph
// ---------------------------------------------------------------------------

struct ArtifactInfo {
    media_type: String,
    revision: String,
    byte_size: u64,
}

struct SnapshotInfo {
    snapshot_id: String,
    document_id: String,
    trashed: bool,
}

struct NoteInfo {
    title: String,
    trashed: bool,
}

struct Graph {
    projection: GraphProjection,
    node_index: HashMap<String, usize>,
    relation_index: HashMap<String, usize>,
    /// Node index -> incident relation indices (ascending, deduplicated).
    incident: Vec<Vec<usize>>,
    /// Original (undegraded) source-map objects, by id.
    objects: HashMap<String, StoredMapObject>,
    artifacts: HashMap<String, ArtifactInfo>,
    /// Snapshot artifact id -> ink snapshot.
    snapshots: HashMap<String, SnapshotInfo>,
    /// Snapshot id -> snapshot artifact id.
    snapshot_artifacts: HashMap<String, String>,
    notes: HashMap<String, NoteInfo>,
    relation_revisions: HashMap<String, String>,
    /// Diagnostics produced while building the projection (degraded data).
    load_diagnostics: Vec<Value>,
}

fn internal(context: &str, error: impl std::fmt::Display) -> ToolError {
    ToolError::new(ToolErrorCode::Internal, format!("{context}: {error}"))
}

fn inner_payload(value: &Value) -> &Value {
    value.get("payload").unwrap_or(value)
}

fn inner_payload_mut(value: &mut Value) -> &mut Value {
    if value.get("payload").is_some() {
        value.get_mut("payload").expect("checked")
    } else {
        value
    }
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn to_string_value(value: impl serde::Serialize) -> Option<String> {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => Some(text),
        _ => None,
    }
}

fn lifecycle_retired(lifecycle: Option<LifecycleStatus>) -> bool {
    matches!(
        lifecycle,
        Some(LifecycleStatus::Superseded | LifecycleStatus::Rejected | LifecycleStatus::Deprecated)
    )
}

fn as_of() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// Removes the cause of one dangling endpoint from the projection input.
/// Returns false when nothing could be removed (the caller then gives up).
fn degrade(source_map: &mut SourceMapSnapshot, culprit: &str, entity: Option<&str>) -> bool {
    let Some(position) = source_map
        .objects
        .iter()
        .position(|object| object.stable_id.as_str() == culprit)
    else {
        return false;
    };
    let object = &mut source_map.objects[position];
    if object.kind == EntityKind::SourceSpan && !object.relation {
        let payload = inner_payload_mut(&mut object.payload);
        if let Some(entity) = entity {
            if str_at(payload, "/markdown/artifact") == Some(entity) {
                payload["markdown"] = Value::Null;
                return true;
            }
            if str_at(payload, "/ink/snapshot_artifact") == Some(entity) {
                payload["ink"] = Value::Null;
                return true;
            }
        }
    }
    let removed = source_map.objects.remove(position);
    source_map.relation_endpoints.remove(&removed.stable_id);
    true
}

fn load(workspace: &McpWorkspace) -> Result<Graph, ToolError> {
    let store: SqliteArtifactStore = workspace.store()?;
    let landscape = store
        .landscape_snapshot()
        .map_err(|error| internal("failed to read the landscape snapshot", error))?;
    let corpus = store
        .read_search_corpus()
        .map_err(|error| internal("failed to read ink notes", error))?;

    let mut source_map = landscape.sources().clone();
    let mut dangling: Vec<Value> = Vec::new();
    let mut attempts = source_map.objects.len() + 1;
    let projection = loop {
        let input = LandscapeProjectionInput::new(landscape.artifacts(), &source_map)
            .with_ink_snapshot_labels(landscape.ink_snapshot_labels());
        let (culprit, entity) = match project_landscape(input) {
            Ok(projection) => break projection,
            Err(SourceGraphProjectionError::UnknownEndpoint { relation, entity }) => {
                (relation.to_string(), Some(entity.to_string()))
            }
            Err(SourceGraphProjectionError::MissingEndpoints(relation)) => {
                (relation.to_string(), None)
            }
            Err(error) => return Err(internal("failed to project the landscape", error)),
        };
        attempts = attempts.saturating_sub(1);
        if attempts == 0 || !degrade(&mut source_map, &culprit, entity.as_deref()) {
            return Err(internal(
                "failed to project the landscape",
                format!("unrecoverable dangling endpoint at {culprit}"),
            ));
        }
        dangling.push(json!({ "object_id": culprit, "missing_entity": entity }));
    };

    let mut load_diagnostics = Vec::new();
    if !dangling.is_empty() {
        let count = dangling.len();
        dangling.truncate(MAX_DANGLING_LISTED);
        load_diagnostics.push(json!({
            "code": "dangling_endpoint_skipped",
            "message": format!("{count} projection link(s) pointing at missing objects were skipped; affected objects stay readable but lose that edge"),
            "details": { "count": count, "items": dangling },
        }));
    }

    let mut node_index = HashMap::new();
    for (index, node) in projection.nodes().iter().enumerate() {
        node_index.insert(node.entity_id().to_string(), index);
    }
    let mut relation_index = HashMap::new();
    let mut incident = vec![Vec::new(); projection.nodes().len()];
    for (index, relation) in projection.relations().iter().enumerate() {
        relation_index.insert(relation.relation_id().to_string(), index);
        for endpoint in relation.endpoints() {
            if let Some(&node) = node_index.get(endpoint.node().as_str()) {
                let list: &mut Vec<usize> = &mut incident[node];
                if list.last() != Some(&index) {
                    list.push(index);
                }
            }
        }
    }

    let objects = landscape
        .sources()
        .objects
        .iter()
        .map(|object| (object.stable_id.to_string(), object.clone()))
        .collect::<HashMap<_, _>>();
    let mut relation_revisions = HashMap::new();
    for object in landscape.sources().objects.iter().filter(|o| o.relation) {
        relation_revisions.insert(object.stable_id.to_string(), object.revision_id.to_string());
    }
    for conversion in landscape.artifacts().conversions() {
        relation_revisions.insert(
            conversion.header().stable_id().to_string(),
            conversion.header().revision_id().to_string(),
        );
    }

    let notes = corpus
        .notes
        .iter()
        .map(|note| {
            (
                note.document_id.clone(),
                NoteInfo {
                    title: note.title.clone(),
                    trashed: note.trashed,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let mut artifacts = HashMap::new();
    let mut snapshots = HashMap::new();
    let mut snapshot_artifacts = HashMap::new();
    for row in &corpus.snapshots {
        snapshot_artifacts.insert(row.snapshot_id.clone(), row.artifact_id.clone());
        snapshots.insert(
            row.artifact_id.clone(),
            SnapshotInfo {
                snapshot_id: row.snapshot_id.clone(),
                document_id: row.document_id.clone(),
                trashed: row.note_trashed,
            },
        );
    }
    for record in landscape.artifacts().artifacts() {
        let header = record.entity().header();
        let payload = record.entity().payload();
        let id = header.stable_id().to_string();
        let media_type = payload.media_type().as_str().to_owned();
        if media_type == SNAPSHOT_MEDIA_TYPE
            && !snapshots.contains_key(&id)
            && let Some(stored) = store
                .ink_snapshot_for_artifact(&id)
                .map_err(|error| internal("failed to read ink snapshots", error))?
        {
            let trashed = notes.get(&stored.document_id).is_some_and(|n| n.trashed);
            snapshot_artifacts.insert(stored.snapshot_id.clone(), id.clone());
            snapshots.insert(
                id.clone(),
                SnapshotInfo {
                    snapshot_id: stored.snapshot_id,
                    document_id: stored.document_id,
                    trashed,
                },
            );
        }
        artifacts.insert(
            id,
            ArtifactInfo {
                media_type,
                revision: header.revision_id().to_string(),
                byte_size: payload.byte_size(),
            },
        );
    }

    Ok(Graph {
        projection,
        node_index,
        relation_index,
        incident,
        objects,
        artifacts,
        snapshots,
        snapshot_artifacts,
        notes,
        relation_revisions,
        load_diagnostics,
    })
}

// ---------------------------------------------------------------------------
// Resolution and node / edge views
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Node(usize),
    Relation(usize),
    /// A source-map object that is not a landscape node or relation.
    Object,
    Note,
}

struct Resolved {
    input: String,
    canonical: String,
    target: Target,
}

impl Graph {
    fn find(&self, canonical: &str) -> Option<Target> {
        if let Some(&index) = self.node_index.get(canonical) {
            Some(Target::Node(index))
        } else if let Some(&index) = self.relation_index.get(canonical) {
            Some(Target::Relation(index))
        } else if self.objects.contains_key(canonical) {
            Some(Target::Object)
        } else if self.notes.contains_key(canonical) {
            Some(Target::Note)
        } else {
            None
        }
    }

    fn resolve(
        &self,
        workspace: &McpWorkspace,
        input: &str,
    ) -> Result<Option<Resolved>, ToolError> {
        if let Some(target) = self.find(input) {
            return Ok(Some(Resolved {
                input: input.to_owned(),
                canonical: input.to_owned(),
                target,
            }));
        }
        let mut artifact = self.snapshot_artifacts.get(input).cloned();
        if artifact.is_none() && input.starts_with(INK_SNAPSHOT_PREFIX) {
            artifact = workspace
                .store()?
                .open_ink_snapshot(input)
                .map_err(|error| internal("failed to read ink snapshots", error))?
                .map(|snapshot| snapshot.artifact_id);
        }
        Ok(artifact.and_then(|canonical| {
            self.find(&canonical).map(|target| Resolved {
                input: input.to_owned(),
                canonical,
                target,
            })
        }))
    }

    fn snapshot_of(&self, index: usize) -> Option<&SnapshotInfo> {
        let node = &self.projection.nodes()[index];
        self.snapshots.get(node.entity_id().as_str())
    }

    fn is_snapshot_node(&self, index: usize) -> bool {
        let node = &self.projection.nodes()[index];
        node.kind() == EntityKind::SourceArtifact
            && (self.snapshots.contains_key(node.entity_id().as_str())
                || self
                    .artifacts
                    .get(node.entity_id().as_str())
                    .is_some_and(|a| a.media_type == SNAPSHOT_MEDIA_TYPE))
    }

    fn node_kind(&self, index: usize) -> &'static str {
        match self.projection.nodes()[index].kind() {
            EntityKind::SourceArtifact if self.is_snapshot_node(index) => "snapshot",
            EntityKind::SourceArtifact => "artifact",
            EntityKind::SourceSpan => "source_span",
            EntityKind::SourceSemanticUnit => "source_unit",
            _ => "unsupported",
        }
    }

    fn node_retired(&self, index: usize) -> bool {
        lifecycle_retired(self.projection.nodes()[index].lifecycle_status())
            || self.snapshot_of(index).is_some_and(|s| s.trashed)
    }

    fn relation_kind(&self, index: usize) -> String {
        to_string_value(self.projection.relations()[index].relation_kind_value())
            .unwrap_or_else(|| "unknown".to_owned())
    }

    fn relation_retired(&self, index: usize) -> bool {
        lifecycle_retired(self.projection.relations()[index].lifecycle_status())
    }

    /// `(parent_artifact_id, content_ref or the reason it is null)` of a span.
    fn span_ref(&self, span_id: &str) -> (Option<String>, Result<Value, &'static str>) {
        let Some(object) = self.objects.get(span_id) else {
            return (None, Err("no_span_locator"));
        };
        let payload = inner_payload(&object.payload);
        let markdown = str_at(payload, "/markdown/artifact");
        let page = str_at(payload, "/page/artifact");
        let ink = str_at(payload, "/ink/snapshot_artifact");
        let parent = markdown.or(page).or(ink).map(str::to_owned);
        let mut unsupported = false;
        if let Some(artifact) = markdown {
            let start = payload
                .pointer("/markdown/byte_range/start")
                .and_then(Value::as_u64);
            let end = payload
                .pointer("/markdown/byte_range/end")
                .and_then(Value::as_u64);
            match (self.artifacts.get(artifact), start, end) {
                (Some(info), Some(start), Some(end)) if info.media_type == MARKDOWN_MEDIA_TYPE => {
                    return (
                        parent,
                        Ok(json!({
                            "type": "md_byte_range",
                            "artifact_id": artifact,
                            "locator": { "md_byte_range": { "start": start, "end": end } },
                            "media_type": info.media_type,
                            "via": Value::Null,
                        })),
                    );
                }
                (Some(_), Some(_), Some(_)) => unsupported = true,
                _ => {}
            }
        }
        if let Some(artifact) = page {
            let index = payload
                .pointer("/page/pdf_page_index")
                .and_then(Value::as_u64);
            match (self.artifacts.get(artifact), index) {
                (Some(info), Some(index)) if info.media_type == PDF_MEDIA_TYPE => {
                    let page = index + 1;
                    return (
                        parent,
                        Ok(json!({
                            "type": "pdf_pages",
                            "artifact_id": artifact,
                            "locator": { "pdf_pages": { "from": page, "to": page } },
                            "media_type": info.media_type,
                            "via": Value::Null,
                        })),
                    );
                }
                (Some(_), Some(_)) => unsupported = true,
                _ => {}
            }
        }
        let reason = if unsupported {
            "unsupported_media_type"
        } else if ink.is_some() {
            "ink_out_of_scope"
        } else {
            "no_span_locator"
        };
        (parent, Err(reason))
    }

    /// Unit: lowest-ordinal Markdown span, else lowest-ordinal PDF page span.
    /// Returns `(parent, content_ref|reason, span_count)`.
    fn unit_ref(&self, unit_id: &str) -> (Option<String>, Result<Value, &'static str>, usize) {
        let spans: Vec<&str> = self
            .objects
            .get(unit_id)
            .and_then(|object| inner_payload(&object.payload).get("source_spans"))
            .and_then(Value::as_array)
            .map(|spans| spans.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let resolved: Vec<(&str, Option<String>, Result<Value, &'static str>)> = spans
            .iter()
            .map(|span| {
                let (parent, content) = self.span_ref(span);
                (*span, parent, content)
            })
            .collect();
        let pick = |kind: &str| {
            resolved
                .iter()
                .find(|(_, _, content)| content.as_ref().is_ok_and(|value| value["type"] == kind))
        };
        if let Some((span, parent, Ok(content))) =
            pick("md_byte_range").or_else(|| pick("pdf_pages"))
        {
            let mut content = content.clone();
            content["via"] = json!(span);
            return (parent.clone(), Ok(content), spans.len());
        }
        let parent = resolved.iter().find_map(|(_, parent, _)| parent.clone());
        let has = |reason: &str| {
            resolved
                .iter()
                .any(|(_, _, content)| content.as_ref().err() == Some(&reason))
        };
        let reason = if has("ink_out_of_scope") {
            "ink_out_of_scope"
        } else if has("unsupported_media_type") {
            "unsupported_media_type"
        } else {
            "no_span_locator"
        };
        (parent, Err(reason), spans.len())
    }

    /// Base node view: the object, the `content_unavailable` reason (if any),
    /// and the span count of a unit (for `multiple_spans`).
    fn view(
        &self,
        canonical: &str,
        target: Target,
    ) -> (Map<String, Value>, Option<&'static str>, usize) {
        let mut node = Map::new();
        node.insert("id".into(), json!(canonical));
        let mut reason = None;
        let mut span_count = 0;
        let mut content = Value::Null;
        let mut parent = Value::Null;
        match target {
            Target::Node(index) => {
                let projected = &self.projection.nodes()[index];
                let kind = self.node_kind(index);
                node.insert("kind".into(), json!(kind));
                node.insert("entity_kind".into(), json!(projected.kind()));
                node.insert("label".into(), json!(projected.label()));
                match kind {
                    "snapshot" => {
                        if let Some(info) = self.artifacts.get(canonical) {
                            node.insert("revision".into(), json!(info.revision));
                            node.insert("media_type".into(), json!(info.media_type));
                        }
                        let snapshot = self.snapshot_of(index);
                        let trashed = snapshot.is_some_and(|s| s.trashed);
                        node.insert(
                            "lifecycle".into(),
                            json!(if trashed { "trashed" } else { "active" }),
                        );
                        if let Some(snapshot) = snapshot {
                            node.insert("document_id".into(), json!(snapshot.document_id));
                            node.insert("snapshot_id".into(), json!(snapshot.snapshot_id));
                        }
                        reason = Some("ink_out_of_scope");
                    }
                    "artifact" => {
                        node.insert("lifecycle".into(), json!(projected.lifecycle_status()));
                        let info = self.artifacts.get(canonical);
                        if let Some(info) = info {
                            node.insert("revision".into(), json!(info.revision));
                            node.insert("media_type".into(), json!(info.media_type));
                        }
                        match info {
                            Some(info) if info.media_type == MARKDOWN_MEDIA_TYPE => {
                                content = json!({
                                    "type": "artifact_head",
                                    "artifact_id": canonical,
                                    "locator": { "md_byte_range": { "start": 0, "end": info.byte_size.min(ARTIFACT_HEAD_BYTES) } },
                                    "media_type": MARKDOWN_MEDIA_TYPE,
                                    "via": Value::Null,
                                });
                            }
                            Some(info) if info.media_type == PDF_MEDIA_TYPE => {
                                content = json!({
                                    "type": "artifact_head",
                                    "artifact_id": canonical,
                                    "locator": { "pdf_pages": { "from": 1, "to": 1 } },
                                    "media_type": PDF_MEDIA_TYPE,
                                    "via": Value::Null,
                                });
                            }
                            _ => reason = Some("unsupported_media_type"),
                        }
                    }
                    "source_span" => {
                        node.insert("lifecycle".into(), json!(projected.lifecycle_status()));
                        let (p, c) = self.span_ref(canonical);
                        parent = json!(p);
                        match c {
                            Ok(c) => content = c,
                            Err(r) => reason = Some(r),
                        }
                    }
                    "source_unit" => {
                        node.insert("lifecycle".into(), json!(projected.lifecycle_status()));
                        if let Some(unit_kind) = self
                            .objects
                            .get(canonical)
                            .and_then(|object| str_at(inner_payload(&object.payload), "/kind"))
                        {
                            node.insert("unit_kind".into(), json!(unit_kind));
                        }
                        let (p, c, count) = self.unit_ref(canonical);
                        parent = json!(p);
                        span_count = count;
                        match c {
                            Ok(c) => content = c,
                            Err(r) => reason = Some(r),
                        }
                    }
                    _ => {
                        node.insert("lifecycle".into(), json!(projected.lifecycle_status()));
                    }
                }
                if !matches!(kind, "artifact" | "snapshot")
                    && let Some(object) = self.objects.get(canonical)
                {
                    node.insert("revision".into(), json!(object.revision_id.to_string()));
                }
                if let Some(summary) = projected.summary_ref() {
                    node.insert("summary".into(), json!(summary));
                }
            }
            Target::Relation(index) => {
                let relation = &self.projection.relations()[index];
                node.insert("kind".into(), json!("relation"));
                node.insert("entity_kind".into(), json!(relation.kind()));
                node.insert("label".into(), json!(relation.label()));
                node.insert("lifecycle".into(), json!(relation.lifecycle_status()));
                if let Some(revision) = self.relation_revisions.get(canonical) {
                    node.insert("revision".into(), json!(revision));
                }
                node.insert("relation".into(), json!(self.relation_kind(index)));
                node.insert(
                    "synthetic".into(),
                    json!(canonical.starts_with(SYNTHETIC_PREFIX)),
                );
                node.insert("endpoints".into(), endpoints_json(relation));
                reason = Some("relation_node");
            }
            Target::Object => {
                let object = &self.objects[canonical];
                let payload = inner_payload(&object.payload);
                let label = str_at(payload, "/display_title")
                    .or_else(|| str_at(payload, "/local_title"))
                    .or_else(|| str_at(payload, "/title"))
                    .map(str::to_owned)
                    .unwrap_or_else(|| id_tail(canonical));
                node.insert("kind".into(), json!("unsupported"));
                node.insert("entity_kind".into(), json!(object.kind));
                node.insert("label".into(), json!(label));
                node.insert("lifecycle".into(), json!(object.lifecycle));
                node.insert("revision".into(), json!(object.revision_id.to_string()));
            }
            Target::Note => {
                let note = &self.notes[canonical];
                node.insert("kind".into(), json!("note"));
                node.insert("entity_kind".into(), json!("InkDocument"));
                let label = if note.title.is_empty() {
                    id_tail(canonical)
                } else {
                    note.title.clone()
                };
                node.insert("label".into(), json!(label));
                node.insert(
                    "lifecycle".into(),
                    json!(if note.trashed { "trashed" } else { "active" }),
                );
                reason = Some("ink_out_of_scope");
            }
        }
        node.insert("parent_artifact_id".into(), parent);
        node.insert("content_ref".into(), content);
        (node, reason, span_count)
    }

    fn target_retired(&self, canonical: &str, target: Target) -> bool {
        match target {
            Target::Node(index) => self.node_retired(index),
            Target::Relation(index) => self.relation_retired(index),
            Target::Object => lifecycle_retired(Some(self.objects[canonical].lifecycle)),
            Target::Note => self.notes[canonical].trashed,
        }
    }

    fn edge_json(&self, index: usize) -> Value {
        let relation = &self.projection.relations()[index];
        let id = relation.relation_id().to_string();
        let side = |direction: IncidenceDirection| -> Vec<&str> {
            relation
                .endpoints()
                .iter()
                .filter(|e| e.direction() == direction)
                .map(|e| e.node().as_str())
                .collect()
        };
        let from = side(IncidenceDirection::IntoRelation);
        let to = side(IncidenceDirection::OutOfRelation);
        let single = from.len() == 1 && to.len() == 1 && relation.endpoints().len() == 2;
        let mut edge = json!({
            "id": id,
            "relation": self.relation_kind(index),
            "entity_kind": relation.kind(),
            "synthetic": id.starts_with(SYNTHETIC_PREFIX),
            "label": relation.label(),
            "lifecycle": relation.lifecycle_status(),
            "from": if single { json!(from[0]) } else { Value::Null },
            "to": if single { json!(to[0]) } else { Value::Null },
            "endpoints": endpoints_json(relation),
        });
        if let Some(revision) = self.relation_revisions.get(&id) {
            edge["revision"] = json!(revision);
        }
        edge
    }

    fn edge_summary(&self, index: usize) -> Value {
        let id = self.projection.nodes()[index].entity_id();
        let mut summary: BTreeMap<String, BTreeMap<&'static str, u64>> = BTreeMap::new();
        for &relation_index in &self.incident[index] {
            let kind = self.relation_kind(relation_index);
            if kind == "unknown" {
                continue;
            }
            let relation = &self.projection.relations()[relation_index];
            let mut sides = BTreeSet::new();
            for endpoint in relation.endpoints().iter().filter(|e| e.node() == id) {
                sides.insert(match endpoint.direction() {
                    IncidenceDirection::IntoRelation => "out",
                    IncidenceDirection::OutOfRelation => "in",
                    IncidenceDirection::Undirected => "undirected",
                });
            }
            let entry = summary.entry(kind).or_default();
            entry.entry("in").or_default();
            entry.entry("out").or_default();
            for side in sides {
                *entry.entry(side).or_default() += 1;
            }
        }
        json!(summary)
    }
}

fn id_tail(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_owned()
}

fn side_name(direction: IncidenceDirection) -> &'static str {
    match direction {
        IncidenceDirection::IntoRelation => "from",
        IncidenceDirection::OutOfRelation => "to",
        IncidenceDirection::Undirected => "undirected",
    }
}

fn endpoints_json(relation: &ProjectionRelation) -> Value {
    Value::Array(
        relation
            .endpoints()
            .iter()
            .map(|endpoint| {
                json!({
                    "node_id": endpoint.node().as_str(),
                    "role": endpoint.role().as_str(),
                    "side": side_name(endpoint.direction()),
                    "ordinal": endpoint.ordinal(),
                })
            })
            .collect(),
    )
}

fn content_unavailable(reason: &str, node_ids: Vec<String>) -> Value {
    let message = match reason {
        "ink_out_of_scope" => "handwriting content is not readable in Phase1c",
        "no_span_locator" => {
            "no source span with a Markdown or PDF page locator; do not guess a byte range"
        }
        "unsupported_media_type" => "material.read does not support this media type",
        _ => "relations have no readable content; inspect their endpoints",
    };
    json!({
        "code": "content_unavailable",
        "message": message,
        "details": { "node_ids": node_ids, "reason": reason },
    })
}

fn unsupported_diagnostic(entity_kind: &Value) -> Value {
    let name = entity_kind.as_str().unwrap_or("this kind");
    json!({
        "code": "unsupported_node_kind",
        "message": format!("{name} is not navigable in Phase1c"),
        "details": { "entity_kind": entity_kind },
    })
}

fn canonicalized(input: &str, canonical: &str) -> Value {
    json!({
        "code": "id_canonicalized",
        "message": "input id was mapped to its graph id; use canonical_id in later calls",
        "details": { "input_id": input, "canonical_id": canonical },
    })
}

fn scope_diagnostic() -> Value {
    diagnostic(
        "scope_not_supported",
        "scope=investigation is not implemented in Phase1; answered with scope=accepted",
    )
}

fn invalid(message: impl Into<String>, details: Value) -> ToolError {
    ToolError::new(ToolErrorCode::InvalidArg, message).with_details(details)
}

fn status_value(status: ToolStatus) -> Value {
    serde_json::to_value(status).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// graph.get
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    node_id: String,
    #[serde(default)]
    scope: Scope,
}

pub(super) fn call_get(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: GetArgs = parse_args(arguments)?;
    if args.node_id.is_empty() {
        return Err(invalid(
            "node_id must not be empty",
            json!({ "field": "node_id" }),
        ));
    }
    let mut diagnostics = Vec::new();
    if args.scope == Scope::Investigation {
        diagnostics.push(scope_diagnostic());
    }
    let graph = load(workspace)?;
    let as_of = as_of();
    let Some(resolved) = graph.resolve(workspace, &args.node_id)? else {
        diagnostics.push(json!({
            "code": "node_not_found",
            "message": "no graph node or catalog entry with this id",
            "details": { "missing_ids": [args.node_id] },
        }));
        return Ok(json!({
            "status": status_value(ToolStatus::NotFound),
            "truncated": false,
            "as_of": as_of,
            "node": Value::Null,
            "diagnostics": diagnostics,
        }));
    };
    diagnostics.extend(graph.load_diagnostics.iter().cloned());
    if resolved.canonical != resolved.input {
        diagnostics.push(canonicalized(&resolved.input, &resolved.canonical));
    }
    let (mut node, reason, span_count) = graph.view(&resolved.canonical, resolved.target);
    node.insert("canonical_id".into(), json!(resolved.canonical));
    let unsupported = node["kind"] == "unsupported";
    if unsupported {
        diagnostics.push(unsupported_diagnostic(&node["entity_kind"]));
    }
    if let Target::Node(index) = resolved.target
        && !unsupported
    {
        node.insert("edge_summary".into(), graph.edge_summary(index));
    }
    if span_count > 1 && !node["content_ref"].is_null() {
        let which = if node["content_ref"]["type"] == "md_byte_range" {
            "Markdown"
        } else {
            "PDF page"
        };
        diagnostics.push(json!({
            "code": "multiple_spans",
            "message": format!("unit has {span_count} spans; content_ref is the lowest-ordinal {which} span (see content_ref.via)"),
            "details": { "span_count": span_count },
        }));
    }
    if let Some(reason) = reason {
        diagnostics.push(content_unavailable(
            reason,
            vec![resolved.canonical.clone()],
        ));
    }
    if graph.target_retired(&resolved.canonical, resolved.target) {
        diagnostics.push(json!({
            "code": "seed_retired",
            "message": "the requested object is retired or trashed; it is returned with its lifecycle",
            "details": { "ids": [resolved.canonical] },
        }));
    }
    Ok(json!({
        "status": status_value(ToolStatus::Ok),
        "truncated": false,
        "as_of": as_of,
        "node": node,
        "diagnostics": diagnostics,
    }))
}

// ---------------------------------------------------------------------------
// graph.neighborhood
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeighborhoodArgs {
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    seed_ids: Option<Vec<String>>,
    #[serde(default)]
    depth: Option<u64>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    edge_types: Option<Vec<String>>,
    #[serde(default)]
    node_kinds: Option<Vec<String>>,
    #[serde(default)]
    max_nodes: Option<u64>,
    #[serde(default)]
    max_edges: Option<u64>,
    #[serde(default)]
    include_trashed: bool,
    #[serde(default)]
    include_virtual: bool,
    #[serde(default)]
    scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Both,
    In,
    Out,
}

struct Applied {
    seeds: Vec<String>,
    depth: u32,
    direction: Direction,
    direction_name: String,
    edge_types: Vec<String>,
    node_kinds: Option<Vec<String>>,
    max_nodes: usize,
    max_edges: usize,
    include_trashed: bool,
    scope: Scope,
}

fn validate_enum(
    field: &str,
    values: &[String],
    allowed: &[&str],
) -> Result<Vec<String>, ToolError> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        if !allowed.contains(&value.as_str()) {
            return Err(invalid(
                format!("{field} contains unknown value `{value}`"),
                json!({ "field": field, "value": value, "allowed": allowed }),
            ));
        }
        if !out.contains(value) {
            out.push(value.clone());
        }
    }
    Ok(out)
}

fn bounded(
    field: &str,
    value: Option<u64>,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, ToolError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = usize::try_from(value).unwrap_or(usize::MAX);
    if value < min || value > max {
        return Err(invalid(
            format!("{field} must be between {min} and {max}, got {value}"),
            json!({ "field": field, "value": value, "min": min, "max": max }),
        ));
    }
    Ok(value)
}

fn validate(args: NeighborhoodArgs) -> Result<Applied, ToolError> {
    let seeds = match (args.node_id, args.seed_ids) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(invalid(
                "give exactly one of node_id or seed_ids",
                json!({ "fields": ["node_id", "seed_ids"] }),
            ));
        }
        (Some(id), None) => vec![id],
        (None, Some(ids)) => {
            if ids.is_empty() || ids.len() > MAX_SEEDS {
                return Err(invalid(
                    format!("seed_ids must hold 1 to {MAX_SEEDS} ids, got {}", ids.len()),
                    json!({ "field": "seed_ids", "count": ids.len(), "max": MAX_SEEDS }),
                ));
            }
            let mut seen = HashSet::new();
            for id in &ids {
                if !seen.insert(id.as_str()) {
                    return Err(invalid(
                        format!("seed_ids contains `{id}` twice"),
                        json!({ "field": "seed_ids", "duplicate": id }),
                    ));
                }
            }
            ids
        }
    };
    if seeds.iter().any(String::is_empty) {
        return Err(invalid(
            "seed ids must not be empty",
            json!({ "field": "node_id" }),
        ));
    }
    if args.include_virtual {
        return Err(invalid(
            "include_virtual must be false: virtual contraction edges are not offered in Phase1c",
            json!({ "field": "include_virtual" }),
        ));
    }
    let depth = bounded(
        "depth",
        args.depth,
        DEFAULT_DEPTH as usize,
        0,
        MAX_DEPTH as usize,
    )? as u32;
    let direction_name = args.direction.unwrap_or_else(|| "both".to_owned());
    let direction = match direction_name.as_str() {
        "both" => Direction::Both,
        "in" => Direction::In,
        "out" => Direction::Out,
        other => {
            return Err(invalid(
                format!("direction must be one of both, in, out; got `{other}`"),
                json!({ "field": "direction", "value": other, "allowed": DIRECTIONS }),
            ));
        }
    };
    let edge_types = match args.edge_types {
        None => DEFAULT_EDGE_TYPES.iter().map(|s| (*s).to_owned()).collect(),
        Some(values) => validate_enum("edge_types", &values, &RELATION_TYPES)?,
    };
    let node_kinds = match args.node_kinds {
        None => None,
        Some(values) => Some(validate_enum("node_kinds", &values, &NODE_KINDS)?),
    };
    let max_nodes = bounded("max_nodes", args.max_nodes, DEFAULT_MAX_NODES, 1, MAX_NODES)?;
    let max_edges = bounded("max_edges", args.max_edges, DEFAULT_MAX_EDGES, 1, MAX_EDGES)?;
    Ok(Applied {
        seeds,
        depth,
        direction,
        direction_name,
        edge_types,
        node_kinds,
        max_nodes,
        max_edges,
        include_trashed: args.include_trashed,
        scope: args.scope,
    })
}

struct Visit {
    depth: u32,
    via: Option<usize>,
    seed: usize,
}

/// Whether `other` is reachable from `own` (two endpoints of one relation)
/// under `direction` (contract §5.4).
fn reachable(direction: Direction, own: IncidenceDirection, other: IncidenceDirection) -> bool {
    match direction {
        Direction::Both => true,
        Direction::Out => {
            own == IncidenceDirection::IntoRelation && other == IncidenceDirection::OutOfRelation
        }
        Direction::In => {
            own == IncidenceDirection::OutOfRelation && other == IncidenceDirection::IntoRelation
        }
    }
}

pub(super) fn call_neighborhood(
    workspace: &McpWorkspace,
    arguments: Value,
) -> Result<Value, ToolError> {
    let args: NeighborhoodArgs = parse_args(arguments)?;
    let applied = validate(args)?;
    let graph = load(workspace)?;
    let as_of = as_of();
    let applied_json = json!({
        "depth": applied.depth,
        "direction": applied.direction_name,
        "edge_types": applied.edge_types,
        "node_kinds": applied.node_kinds,
        "max_nodes": applied.max_nodes,
        "max_edges": applied.max_edges,
        "include_trashed": applied.include_trashed,
        "include_virtual": false,
        "scope": "accepted",
    });
    let mut diagnostics = Vec::new();
    if applied.scope == Scope::Investigation {
        diagnostics.push(scope_diagnostic());
    }

    // Resolve every seed first: any miss is an all-or-nothing not_found.
    let mut resolved = Vec::new();
    let mut missing = Vec::new();
    for seed in &applied.seeds {
        match graph.resolve(workspace, seed)? {
            Some(found) => resolved.push(found),
            None => missing.push(seed.clone()),
        }
    }
    if !missing.is_empty() {
        diagnostics.push(json!({
            "code": "node_not_found",
            "message": format!("{} of {} seed(s) not found; nothing was expanded", missing.len(), applied.seeds.len()),
            "details": { "missing_ids": missing },
        }));
        return Ok(json!({
            "status": status_value(ToolStatus::NotFound),
            "truncated": false,
            "as_of": as_of,
            "seed_ids": applied.seeds,
            "applied": applied_json,
            "nodes": [],
            "edges": [],
            "counts": { "nodes_returned": 0, "edges_returned": 0 },
            "diagnostics": diagnostics,
        }));
    }
    diagnostics.extend(graph.load_diagnostics.iter().cloned());

    let mut seed_ids: Vec<String> = Vec::new();
    // Seeds returned as-is without expansion (note / unsupported).
    let mut static_seeds: Vec<(String, Target)> = Vec::new();
    let mut visits: BTreeMap<usize, Visit> = BTreeMap::new();
    let mut frontier: Vec<usize> = Vec::new();
    let mut retired_seeds = Vec::new();
    for found in &resolved {
        if seed_ids.contains(&found.canonical) {
            continue;
        }
        seed_ids.push(found.canonical.clone());
        if found.canonical != found.input {
            diagnostics.push(canonicalized(&found.input, &found.canonical));
        }
        if graph.target_retired(&found.canonical, found.target) {
            retired_seeds.push(found.canonical.clone());
        }
        match found.target {
            Target::Node(index) if graph.node_kind(index) != "unsupported" => {
                let seed = seed_ids.len() - 1;
                visits.insert(
                    index,
                    Visit {
                        depth: 0,
                        via: None,
                        seed,
                    },
                );
                frontier.push(index);
            }
            Target::Relation(_) => {
                diagnostics.push(json!({
                    "code": "relation_seed",
                    "message": "seed is a relation and is not expanded; use graph.get to inspect its endpoints, or seed from an endpoint",
                    "details": { "relation_id": found.canonical },
                }));
            }
            target => static_seeds.push((found.canonical.clone(), target)),
        }
    }
    if !retired_seeds.is_empty() {
        diagnostics.push(json!({
            "code": "seed_retired",
            "message": "a requested seed is retired or trashed; it is returned with its lifecycle",
            "details": { "ids": retired_seeds },
        }));
    }

    let allowed: HashSet<&str> = applied.edge_types.iter().map(String::as_str).collect();
    let mut hidden_nodes: BTreeSet<usize> = BTreeSet::new();
    let mut hidden_relations: BTreeSet<usize> = BTreeSet::new();
    let mut unknown_relations: BTreeSet<usize> = BTreeSet::new();
    let mut filtered_by_type: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut unexpanded: BTreeSet<usize> = BTreeSet::new();
    let mut visit_limited = false;
    let mut visited_count = visits.len();

    // Relations around the seeds that edge_types excludes (one hop).
    for &seed in &frontier {
        for &relation in &graph.incident[seed] {
            let kind = graph.relation_kind(relation);
            if kind != "unknown" && !allowed.contains(kind.as_str()) {
                filtered_by_type.entry(kind).or_default().insert(relation);
            }
        }
    }

    let mut depth = 0;
    'bfs: while !frontier.is_empty() {
        let expanding = depth < applied.depth;
        let mut next = Vec::new();
        for &current in &frontier {
            let current_id = graph.projection.nodes()[current].entity_id().clone();
            let seed = visits[&current].seed;
            for &relation_index in &graph.incident[current] {
                let kind = graph.relation_kind(relation_index);
                if kind == "unknown" {
                    if expanding {
                        unknown_relations.insert(relation_index);
                    }
                    continue;
                }
                if !allowed.contains(kind.as_str()) {
                    continue;
                }
                if !applied.include_trashed && graph.relation_retired(relation_index) {
                    if expanding {
                        hidden_relations.insert(relation_index);
                    }
                    continue;
                }
                let relation = &graph.projection.relations()[relation_index];
                let mut targets = BTreeSet::new();
                for own in relation
                    .endpoints()
                    .iter()
                    .filter(|e| e.node() == &current_id)
                {
                    for other in relation
                        .endpoints()
                        .iter()
                        .filter(|e| e.node() != &current_id)
                    {
                        if reachable(applied.direction, own.direction(), other.direction())
                            && let Some(&index) = graph.node_index.get(other.node().as_str())
                        {
                            targets.insert(index);
                        }
                    }
                }
                for target in targets {
                    if visits.contains_key(&target) || hidden_nodes.contains(&target) {
                        continue;
                    }
                    if !expanding {
                        unexpanded.insert(relation_index);
                        continue;
                    }
                    if visited_count >= MAX_VISITED {
                        visit_limited = true;
                        break 'bfs;
                    }
                    visited_count += 1;
                    if !applied.include_trashed && graph.node_retired(target) {
                        hidden_nodes.insert(target);
                        continue;
                    }
                    visits.insert(
                        target,
                        Visit {
                            depth: depth + 1,
                            via: Some(relation_index),
                            seed,
                        },
                    );
                    next.push(target);
                }
            }
        }
        if !expanding {
            break;
        }
        frontier = next;
        depth += 1;
    }

    // Output nodes: node_kinds filter (seeds exempt), sort, layer-wise cap.
    let multi_seed = seed_ids.len() > 1;
    let mut filtered_nodes = 0usize;
    // (depth, kind, id, landscape index or None for static seeds)
    let mut candidates: Vec<(u32, &'static str, String, Option<usize>)> = Vec::new();
    for (&index, visit) in &visits {
        let kind = graph.node_kind(index);
        if visit.depth > 0
            && let Some(kinds) = &applied.node_kinds
            && !kinds.iter().any(|k| k == kind)
        {
            filtered_nodes += 1;
            continue;
        }
        let id = graph.projection.nodes()[index].entity_id().to_string();
        candidates.push((visit.depth, kind, id, Some(index)));
    }
    for (canonical, target) in &static_seeds {
        let kind = match target {
            Target::Note => "note",
            _ => "unsupported",
        };
        candidates.push((0, kind, canonical.clone(), None));
    }
    candidates.sort();
    let nodes_seen = candidates.len();
    let nodes_cut = candidates.len() > applied.max_nodes;
    candidates.truncate(applied.max_nodes);
    let returned: HashMap<usize, u32> = candidates
        .iter()
        .filter_map(|(depth, _, _, index)| index.map(|index| (index, *depth)))
        .collect();

    // Output edges: allowed kind, visible, every endpoint returned.
    let mut edge_candidates: BTreeSet<(u32, String, String, usize)> = BTreeSet::new();
    for &index in returned.keys() {
        for &relation_index in &graph.incident[index] {
            let kind = graph.relation_kind(relation_index);
            if kind == "unknown" || !allowed.contains(kind.as_str()) {
                continue;
            }
            if !applied.include_trashed && graph.relation_retired(relation_index) {
                continue;
            }
            let relation = &graph.projection.relations()[relation_index];
            let depths: Option<Vec<u32>> = relation
                .endpoints()
                .iter()
                .map(|endpoint| {
                    graph
                        .node_index
                        .get(endpoint.node().as_str())
                        .and_then(|i| returned.get(i))
                        .copied()
                })
                .collect();
            if let Some(depths) = depths {
                let min_depth = depths.into_iter().min().unwrap_or(0);
                edge_candidates.insert((
                    min_depth,
                    kind,
                    relation.relation_id().to_string(),
                    relation_index,
                ));
            }
        }
    }
    let edges_cut = edge_candidates.len() > applied.max_edges;
    let edges: Vec<Value> = edge_candidates
        .iter()
        .take(applied.max_edges)
        .map(|(_, _, _, index)| graph.edge_json(*index))
        .collect();

    let mut unavailable: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut unsupported_kinds: BTreeSet<String> = BTreeSet::new();
    let mut nodes: Vec<Value> = Vec::new();
    for (depth, _, id, index) in &candidates {
        let (target, via, seed_of) = match index {
            Some(index) => {
                let visit = &visits[index];
                (
                    Target::Node(*index),
                    visit.via,
                    seed_ids[visit.seed].clone(),
                )
            }
            None => {
                let target = static_seeds
                    .iter()
                    .find(|(seed, _)| seed == id)
                    .map(|(_, target)| *target)
                    .expect("static seed");
                (target, None, id.clone())
            }
        };
        let (mut view, reason, _) = graph.view(id, target);
        view.insert("depth".into(), json!(depth));
        view.insert(
            "via_edge".into(),
            json!(via.map(|r| graph.projection.relations()[r].relation_id().to_string())),
        );
        if multi_seed {
            view.insert("seed_of".into(), json!(seed_of));
        }
        if view["kind"] == "unsupported"
            && let Some(entity_kind) = view["entity_kind"].as_str()
        {
            unsupported_kinds.insert(entity_kind.to_owned());
        }
        if let Some(reason) = reason {
            unavailable.entry(reason).or_default().push(id.clone());
        }
        nodes.push(Value::Object(view));
    }

    for entity_kind in unsupported_kinds {
        diagnostics.push(unsupported_diagnostic(&json!(entity_kind)));
    }
    for (reason, ids) in unavailable {
        diagnostics.push(content_unavailable(reason, ids));
    }
    if nodes_cut {
        diagnostics.push(json!({
            "code": "max_nodes_reached",
            "message": format!("stopped at max_nodes={}; deeper layers were cut first. Narrow edge_types, node_kinds or depth before raising the cap", applied.max_nodes),
            "details": { "limit": applied.max_nodes, "returned": nodes.len() },
        }));
    }
    if edges_cut {
        diagnostics.push(json!({
            "code": "max_edges_reached",
            "message": format!("stopped at max_edges={}; remaining relations omitted", applied.max_edges),
            "details": { "limit": applied.max_edges, "returned": edges.len() },
        }));
    }
    if visit_limited {
        diagnostics.push(json!({
            "code": "visit_limit_reached",
            "message": format!("traversal stopped after visiting {MAX_VISITED} entities"),
            "details": { "limit": MAX_VISITED },
        }));
    }
    if !unexpanded.is_empty() {
        diagnostics.push(json!({
            "code": "depth_frontier",
            "message": format!("{} relation(s) lead beyond depth {}; raise depth to follow them", unexpanded.len(), applied.depth),
            "details": { "unexpanded_edges": unexpanded.len() },
        }));
    }
    if !filtered_by_type.is_empty() {
        let counts: BTreeMap<String, usize> = filtered_by_type
            .into_iter()
            .map(|(kind, set)| (kind, set.len()))
            .collect();
        diagnostics.push(json!({
            "code": "edge_types_filtered",
            "message": "the seed has relations excluded by edge_types; list them in edge_types to follow them",
            "details": { "counts": counts },
        }));
    }
    if filtered_nodes > 0 {
        diagnostics.push(json!({
            "code": "nodes_filtered",
            "message": format!("{filtered_nodes} node(s) were traversed but omitted by node_kinds; edges touching them are omitted too"),
            "details": { "count": filtered_nodes },
        }));
    }
    if !unknown_relations.is_empty() {
        diagnostics.push(json!({
            "code": "unknown_relation_skipped",
            "message": "relations of unknown kind are never traversed or returned",
            "details": { "count": unknown_relations.len() },
        }));
    }
    if !hidden_nodes.is_empty() || !hidden_relations.is_empty() {
        diagnostics.push(json!({
            "code": "retired_hidden",
            "message": "retired or trashed objects were hidden; pass include_trashed to include them",
            "details": {
                "count": hidden_nodes.len() + hidden_relations.len(),
                "nodes": hidden_nodes.len(),
                "relations": hidden_relations.len(),
            },
        }));
    }

    let truncated = nodes_cut || edges_cut || visit_limited;
    let mut counts = json!({ "nodes_returned": nodes.len(), "edges_returned": edges.len() });
    if truncated {
        counts["nodes_seen"] = json!(nodes_seen);
    }
    let status = if truncated {
        ToolStatus::Truncated
    } else {
        ToolStatus::Ok
    };
    Ok(json!({
        "status": status_value(status),
        "truncated": truncated,
        "as_of": as_of,
        "seed_ids": seed_ids,
        "applied": applied_json,
        "nodes": nodes,
        "edges": edges,
        "counts": counts,
        "diagnostics": diagnostics,
    }))
}
