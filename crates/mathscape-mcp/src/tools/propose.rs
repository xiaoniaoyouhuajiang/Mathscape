//! Phase2-0 source-layer proposal tools.
//!
//! `propose_changes` validates source-unit and source-relation operations with
//! the existing pure map planner, then persists only an auditable candidate.
//! `get_change_set` reads that candidate back; neither tool mutates the map.
//!
//! Investigation identity is server-owned: a round is bound to exactly one
//! investigation (`investigation_rounds`), and a sandbox session is one
//! investigation (`sandbox:{session}`); dialogue threads are metadata only.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::Utc;
use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::{
    ActorRef, ByteRange, LayoutKind, MapEditSnapshot, MarkdownBlock, MarkdownLocator,
    PageCoordinate, SourceRelationChanges, SourceRelationDraft, SourceSpan, SourceSpanDraft,
    SourceUnitChanges, SourceUnitDraft, SpanGranularity, StableId, TextByteRange,
    UserChangeOperation, UserChangeRequest,
};
use mathscape_orchestrator::change_set_plan::{
    ChangeSetKey, ClosureError, StagedChangeSet, combined_operations, staged_dependency_closure,
};
use mathscape_orchestrator::map_edit::plan_map_change;
use mathscape_orchestrator::{
    PAGE_MARKER_STRUCTURAL_KIND, anchor_markdown, markdown_span_id, page_marker_id,
    quote_matches_range, source_text_hash, valid_temp_id,
};
use mathscape_store_sqlite::{
    ChangeSetDependency, PendingChangeSet, ProposedChangeRecord, SqliteArtifactStore,
    SqliteStoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{GET_CHANGE_SET, PROPOSE_CHANGES};
use crate::McpWorkspace;
use crate::investigation::staged_change_sets;

const MAX_REQUEST_ID_CHARS: usize = 128;
const MAX_RATIONALE_CHARS: usize = 4000;
const MAX_ITEMS: usize = 1000;
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposeArgs {
    request_id: String,
    #[serde(default)]
    supersedes: Option<Supersedes>,
    rationale: String,
    #[serde(default)]
    existing_refs: Vec<ExistingRef>,
    #[serde(default)]
    proposed_objects: Vec<ProposedObject>,
    #[serde(default)]
    proposed_relations: Vec<ProposedRelation>,
    #[serde(default)]
    unresolved: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Supersedes {
    change_set_id: String,
    version: i64,
    proposal_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExistingRef {
    id: String,
    revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedObject {
    local_ref: String,
    kind: String,
    op: String,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    expected_revision: Option<String>,
    #[serde(default)]
    changes: Option<Value>,
    #[serde(default)]
    evidence: Vec<Value>,
    #[serde(default)]
    debts: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceSpanInput {
    artifact_id: String,
    md_byte_range: ByteRangeInput,
    granularity: SpanGranularity,
    quote: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ByteRangeInput {
    start: u64,
    end: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedRelation {
    local_ref: String,
    kind: String,
    op: String,
    #[serde(default)]
    relation: Option<mathscape_domain::DiscoveryRelationKind>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    evidence_spans: Vec<String>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    expected_revision: Option<String>,
    #[serde(default)]
    changes: Option<Value>,
    #[serde(default)]
    debts: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Error,
    Warn,
    Info,
}

#[derive(Clone, Debug)]
struct Diagnostic {
    level: Level,
    code: String,
    message: String,
    local_ref: Option<String>,
}

impl Diagnostic {
    fn error(code: &str, message: impl Into<String>, local_ref: Option<&str>) -> Self {
        Self::with_level(Level::Error, code, message, local_ref)
    }

    fn with_level(
        level: Level,
        code: &str,
        message: impl Into<String>,
        local_ref: Option<&str>,
    ) -> Self {
        Self {
            level,
            code: code.to_owned(),
            message: message.into(),
            local_ref: local_ref.map(str::to_owned),
        }
    }

    fn is_error(&self) -> bool {
        self.level == Level::Error
    }
}

pub(super) fn definition() -> Value {
    json!({
        "name": PROPOSE_CHANGES,
        "description": "Validate and persist a Phase2 source-layer proposal. It supports immutable source_span creation plus source_unit/source_relation create/edit, returns staged or blocked, and never changes the accepted map. Requires a bound round token.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "request_id": { "type": "string", "minLength": 1, "maxLength": MAX_REQUEST_ID_CHARS },
                "supersedes": {
                    "type": "object",
                    "properties": {
                        "change_set_id": { "type": "string" },
                        "version": { "type": "integer", "minimum": 1 },
                        "proposal_fingerprint": { "type": "string" }
                    },
                    "required": ["change_set_id", "version", "proposal_fingerprint"],
                    "additionalProperties": false
                },
                "rationale": { "type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_CHARS },
                "existing_refs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "id": { "type": "string" }, "revision": { "type": "string" } },
                        "required": ["id", "revision"],
                        "additionalProperties": false
                    }
                },
                "proposed_objects": { "type": "array", "items": { "type": "object" }, "maxItems": MAX_ITEMS },
                "proposed_relations": { "type": "array", "items": { "type": "object" }, "maxItems": MAX_ITEMS },
                "unresolved": { "type": "array", "items": {} }
            },
            "required": ["request_id", "rationale"],
            "additionalProperties": false
        }
    })
}

pub(super) fn get_definition() -> Value {
    json!({
        "name": GET_CHANGE_SET,
        "description": "Read a Phase2 change set belonging to the current investigation. It returns normalized proposal text, version states, debts and diagnostics without changing anything.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "change_set_id": { "type": "string", "minLength": 1 },
                "version": { "type": "integer", "minimum": 1 }
            },
            "required": ["change_set_id"],
            "additionalProperties": false
        }
    })
}

pub(super) fn propose_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

pub(super) fn get_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

pub(super) fn propose(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: ProposeArgs = super::parse_args(arguments)?;
    let round = required_round(workspace)?;
    validate_basic_args(&args)?;
    let proposal_fingerprint = proposal_fingerprint(&args)?;
    let request_fingerprint = request_fingerprint(&args)?;
    let mut store = workspace.store()?;
    let investigation_id = bind_investigation(workspace, &mut store, &round)?;

    if let Some(existing) = store
        .proposed_change_by_request(&investigation_id, &args.request_id)
        .map_err(internal)?
    {
        if existing.request_fingerprint == request_fingerprint {
            return Ok(existing.result);
        }
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "request_id was already used with different proposal content",
        )
        .with_details(json!({ "reason": "idempotency_conflict" })));
    }

    let (change_set_id, version, supersedes_version) =
        resolve_version(&store, &investigation_id, &args)?;
    let ids = allocate_ids(&change_set_id, &args)?;
    let actor = ActorRef::parse(agent_actor(workspace)).map_err(internal)?;
    let snapshot = store.map_edit_snapshot().map_err(internal)?;
    let base_commit = snapshot.base_commit.clone();
    let versions = store
        .list_change_set_versions(&investigation_id)
        .map_err(internal)?;

    let mut diagnostics = validate_refs(&args.existing_refs);
    diagnostics.extend(compile_diagnostics(&args, &ids));
    let (dependencies, dependency_diagnostics) = resolve_staged_dependencies(
        &versions,
        &snapshot,
        &investigation_id,
        &change_set_id,
        version,
        &args,
        &ids,
    );
    diagnostics.extend(dependency_diagnostics);
    let (span_drafts, span_diagnostics) =
        materialize_span_drafts(workspace, &store, &snapshot, &args, &ids);
    diagnostics.extend(span_diagnostics);

    let operations = if !has_errors(&diagnostics) {
        match compile_operations(&args, &ids, &span_drafts) {
            Ok(operations) => operations,
            Err(error) => {
                diagnostics.push(Diagnostic::error("invalid_operation", error, None));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let compiled_operations = serde_json::to_value(&operations).map_err(internal)?;

    if !has_errors(&diagnostics) {
        // Plan against the accepted head plus the full not-yet-accepted
        // dependency closure, in topological order.
        let mut sets = staged_change_sets(&versions);
        let own_key: ChangeSetKey = (change_set_id.clone(), version);
        sets.push(StagedChangeSet {
            change_set_id: change_set_id.clone(),
            version,
            state: "staged".into(),
            created_at: String::new(),
            operations: operations.clone(),
            depends_on: dependencies
                .iter()
                .map(|dep| (dep.depends_on_change_set_id.clone(), dep.depends_on_version))
                .collect(),
        });
        match staged_dependency_closure(&sets, std::slice::from_ref(&own_key)) {
            Ok(closure) if closure.iter().any(|set| set.change_set_id == change_set_id) => {
                diagnostics.push(Diagnostic::error(
                    "change_set_dependency_cycle",
                    "依赖闭包回到了本 change set 的旧版本（替换形成环）",
                    None,
                ));
            }
            Ok(closure) => {
                let request = UserChangeRequest {
                    request_id: request_id_for_planner(&round, &args.request_id),
                    base_commit: base_commit.clone(),
                    reason: args.rationale.clone(),
                    operations: combined_operations(closure, &operations),
                };
                let planned = plan_map_change(&snapshot, &request, &actor);
                diagnostics.extend(planned.impact.diagnostics.into_iter().map(|diagnostic| {
                    Diagnostic {
                        level: Level::Error,
                        local_ref: reverse_lookup(&ids, diagnostic.object_id.as_deref()),
                        code: diagnostic.code,
                        message: diagnostic.message,
                    }
                }));
            }
            Err(error) => diagnostics.push(closure_diagnostic(&error)),
        }
    }

    let agent_debts = collect_agent_debts(&args);
    let mut debts = diagnostics
        .iter()
        .filter(|item| item.is_error())
        .map(|item| {
            json!({
                "debt_id": format!("validation:{}", item.code),
                "kind": "validation",
                "code": item.code,
                "local_ref": item.local_ref,
                "target_id": Value::Null,
                "message": item.message
            })
        })
        .collect::<Vec<_>>();
    debts.extend(
        args.unresolved
            .iter()
            .map(|item| json!({ "kind": "unresolved", "value": item })),
    );
    debts.extend(agent_debts);
    let blocked = has_errors(&diagnostics);
    let status = if blocked {
        ToolStatus::Blocked
    } else {
        ToolStatus::Staged
    };
    let result = json!({
        "status": status,
        "investigation_id": investigation_id,
        "change_set_id": change_set_id,
        "version": version,
        "proposal_fingerprint": proposal_fingerprint,
        "base_commit": base_commit,
        "staged_ids": if blocked { Value::Null } else { json!(ids) },
        "debts": debts,
        "diagnostics": diagnostics.iter().map(diagnostic_value).collect::<Vec<_>>(),
        "compiled_operations": compiled_operations,
        "supersedes": args.supersedes.as_ref().map(|s| json!({
            "change_set_id": s.change_set_id,
            "version": s.version
        })),
        "dependencies": dependencies.iter().map(|dependency| json!({
            "change_set_id": dependency.depends_on_change_set_id,
            "version": dependency.depends_on_version,
            "referenced_ids": dependency.referenced_ids
        })).collect::<Vec<_>>()
    });
    let record = ProposedChangeRecord {
        change_set_id,
        version,
        investigation_id: investigation_id.clone(),
        round_id: round.clone(),
        generation: round,
        request_id: args.request_id.clone(),
        request_fingerprint,
        proposal: serde_json::to_value(&args).map_err(internal)?,
        result: result.clone(),
        state: if blocked { "blocked" } else { "staged" }.to_owned(),
        base_commit,
        created_at: Utc::now().to_rfc3339(),
        proposed_by: agent_actor(workspace).to_owned(),
    };
    store
        .insert_proposed_change(&record, supersedes_version, &dependencies)
        .map_err(|error| match error {
            SqliteStoreError::ChangeSetConflict(message) => {
                ToolError::new(ToolErrorCode::InvalidArg, message)
                    .with_details(json!({ "reason": "supersede_conflict" }))
            }
            other => internal(other),
        })?;
    Ok(result)
}

/// Resolves the round's investigation and records the binding.
///
/// * the adapter's investigation and a registered binding disagree →
///   `investigation_mismatch`;
/// * only the adapter's value → bind it (first writer wins);
/// * only a registered binding → use it;
/// * neither (direct CLI) → the round is its own investigation.
fn bind_investigation(
    workspace: &McpWorkspace,
    store: &mut SqliteArtifactStore,
    round: &str,
) -> Result<String, ToolError> {
    let registered = store.round_investigation(round).map_err(internal)?;
    let investigation = match (workspace.investigation(), registered) {
        (Some(declared), Some(registered)) if declared != registered => {
            return Err(investigation_mismatch(&registered, declared));
        }
        (Some(declared), _) => declared.to_owned(),
        (None, Some(registered)) => return Ok(registered),
        (None, None) => round.to_owned(),
    };
    store
        .bind_round_investigation(
            round,
            &investigation,
            agent_actor(workspace),
            &Utc::now().to_rfc3339(),
        )
        .map_err(|error| match error {
            SqliteStoreError::ChangeSetConflict(message) => {
                ToolError::new(ToolErrorCode::Forbidden, message)
                    .with_details(json!({ "reason": "investigation_mismatch" }))
            }
            other => internal(other),
        })?;
    Ok(investigation)
}

/// Read-only variant for `get_change_set`.
fn current_investigation(
    workspace: &McpWorkspace,
    store: &SqliteArtifactStore,
) -> Result<Option<String>, ToolError> {
    let Some(round) = workspace.round().filter(|value| !value.trim().is_empty()) else {
        return Ok(workspace.investigation().map(str::to_owned));
    };
    let registered = store.round_investigation(round).map_err(internal)?;
    match (workspace.investigation(), registered) {
        (Some(declared), Some(registered)) if declared != registered => {
            Err(investigation_mismatch(&registered, declared))
        }
        (Some(declared), _) => Ok(Some(declared.to_owned())),
        (None, Some(registered)) => Ok(Some(registered)),
        (None, None) => Ok(Some(round.to_owned())),
    }
}

fn investigation_mismatch(registered: &str, declared: &str) -> ToolError {
    ToolError::new(
        ToolErrorCode::Forbidden,
        format!("investigation_mismatch: round 已绑定到 {registered}，与 {declared} 不一致"),
    )
    .with_details(json!({ "reason": "investigation_mismatch" }))
}

pub(super) fn get(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: GetArgs = super::parse_args(arguments)?;
    let store = workspace.store()?;
    let not_found = || {
        json!({
            "status": ToolStatus::NotFound,
            "change_set_id": args.change_set_id,
            "change_set": Value::Null
        })
    };
    let Some(investigation) = current_investigation(workspace, &store)? else {
        return Ok(not_found());
    };
    let Some((record, versions)) = store
        .proposed_change(&investigation, &args.change_set_id, args.version)
        .map_err(internal)?
    else {
        return Ok(not_found());
    };
    Ok(json!({
        "status": ToolStatus::Ok,
        "change_set_id": record.change_set_id,
        "investigation_id": record.investigation_id,
        "version": record.version,
        "versions": versions.into_iter().map(|(version, state)| json!({
            "version": version,
            "state": state
        })).collect::<Vec<_>>(),
        "state": record.state,
        "proposal_fingerprint": record.proposal_fingerprint(),
        "proposed_by": record.proposed_by,
        "round_id": record.round_id,
        "proposal": record.proposal,
        "staged_ids": record.result.get("staged_ids").cloned().unwrap_or(Value::Null),
        "dependencies": record.result.get("dependencies").cloned().unwrap_or_else(|| json!([])),
        "debts": record.result.get("debts").cloned().unwrap_or_else(|| json!([])),
        "diagnostics": record.result.get("diagnostics").cloned().unwrap_or_else(|| json!([])),
        "decision": record.result.get("decision").cloned().unwrap_or(Value::Null)
    }))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    change_set_id: String,
    #[serde(default)]
    version: Option<i64>,
}

fn required_round(workspace: &McpWorkspace) -> Result<String, ToolError> {
    workspace
        .round()
        .filter(|round| !round.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Forbidden,
                "propose_changes requires a bound round token",
            )
            .with_details(json!({ "reason": "round_not_bound" }))
        })
}

fn agent_actor(_workspace: &McpWorkspace) -> &'static str {
    // The current Codex adapter is the only production writer. Keep the
    // value server-authored even if a future adapter adds a backend label;
    // the Agent cannot provide this field in tool input.
    "agent:codex"
}

fn validate_basic_args(args: &ProposeArgs) -> Result<(), ToolError> {
    if args.request_id.trim().is_empty()
        || args.request_id.chars().count() > MAX_REQUEST_ID_CHARS
        || args.request_id.chars().any(char::is_control)
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "request_id must be non-empty, bounded and contain no control characters",
        ));
    }
    if args.rationale.trim().is_empty()
        || args.rationale.chars().count() > MAX_RATIONALE_CHARS
        || args.rationale.chars().any(char::is_control)
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "rationale must be non-empty, bounded and contain no control characters",
        ));
    }
    if args.proposed_objects.len() + args.proposed_relations.len() > MAX_ITEMS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "a proposal may contain at most 1000 items",
        ));
    }
    if let Some(supersedes) = &args.supersedes
        && (supersedes.version < 1
            || supersedes.change_set_id.trim().is_empty()
            || supersedes.proposal_fingerprint.trim().is_empty())
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "supersedes must identify a positive version and proposal fingerprint",
        ));
    }
    Ok(())
}

fn validate_refs(refs: &[ExistingRef]) -> Vec<Diagnostic> {
    refs.iter()
        .filter_map(|reference| {
            if StableId::parse(reference.id.clone()).is_err()
                || !reference.revision.starts_with("sha256:")
            {
                Some(Diagnostic {
                    level: Level::Error,
                    code: "invalid_existing_ref".to_owned(),
                    message: "existing_refs needs a stable id and sha256 revision".to_owned(),
                    local_ref: None,
                })
            } else {
                None
            }
        })
        .collect()
}
fn compile_diagnostics(args: &ProposeArgs, ids: &BTreeMap<String, String>) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    let mut refs = BTreeSet::new();
    for object in &args.proposed_objects {
        check_local_ref(&mut result, &mut refs, &object.local_ref);
        if !matches!(object.kind.as_str(), "source_unit" | "source_span") {
            result.push(Diagnostic {
                level: Level::Error,
                code: "kind_not_supported".to_owned(),
                message: "Phase2-0 only supports source_unit, source_span and source_relation"
                    .to_owned(),
                local_ref: Some(object.local_ref.clone()),
            });
        }
        if !matches!(object.op.as_str(), "create" | "edit") {
            result.push(Diagnostic {
                level: Level::Error,
                code: "operation_not_supported".to_owned(),
                message: "Phase2-0 only supports create and edit".to_owned(),
                local_ref: Some(object.local_ref.clone()),
            });
        }
        match object.op.as_str() {
            "create" if object.payload.is_none() => result.push(Diagnostic {
                level: Level::Error,
                code: "missing_payload".to_owned(),
                message: "create requires payload".to_owned(),
                local_ref: Some(object.local_ref.clone()),
            }),
            "edit" if object.target_id.is_none() || object.expected_revision.is_none() => result
                .push(Diagnostic {
                    level: Level::Error,
                    code: "missing_revision_guard".to_owned(),
                    message: "edit requires target_id and expected_revision".to_owned(),
                    local_ref: Some(object.local_ref.clone()),
                }),
            _ => {}
        }
        if object.kind == "source_span" {
            if object.op != "create" {
                result.push(Diagnostic {
                    level: Level::Error,
                    code: "operation_not_supported".to_owned(),
                    message: "source_span only supports create".to_owned(),
                    local_ref: Some(object.local_ref.clone()),
                });
            }
            if object.payload.is_none() {
                result.push(Diagnostic {
                    level: Level::Error,
                    code: "missing_payload".to_owned(),
                    message: "source_span/create requires payload".to_owned(),
                    local_ref: Some(object.local_ref.clone()),
                });
            }
        }
    }
    for relation in &args.proposed_relations {
        check_local_ref(&mut result, &mut refs, &relation.local_ref);
        if relation.kind != "source_relation" {
            result.push(Diagnostic {
                level: Level::Error,
                code: "kind_not_supported".to_owned(),
                message: "relation kind must be source_relation".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
        if !matches!(relation.op.as_str(), "create" | "edit") {
            result.push(Diagnostic {
                level: Level::Error,
                code: "operation_not_supported".to_owned(),
                message: "Phase2-0 only supports create and edit".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
        if relation.op == "create"
            && (relation.relation.is_none()
                || relation.source.is_none()
                || relation.target.is_none())
        {
            result.push(Diagnostic {
                level: Level::Error,
                code: "missing_relation_fields".to_owned(),
                message: "relation create requires relation, source and target".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
        if relation.op == "edit"
            && (relation.target_id.is_none() || relation.expected_revision.is_none())
        {
            result.push(Diagnostic {
                level: Level::Error,
                code: "missing_revision_guard".to_owned(),
                message: "relation edit requires target_id and expected_revision".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
    }
    for (local_ref, id) in ids {
        if StableId::parse(id.clone()).is_err() {
            result.push(Diagnostic {
                level: Level::Error,
                code: "invalid_staged_id".to_owned(),
                message: "server-generated staged id is invalid".to_owned(),
                local_ref: Some(local_ref.clone()),
            });
        }
    }
    let local_refs = args
        .proposed_objects
        .iter()
        .map(|item| item.local_ref.as_str())
        .chain(
            args.proposed_relations
                .iter()
                .map(|item| item.local_ref.as_str()),
        )
        .collect::<BTreeSet<_>>();
    let mut inspect = Vec::<(Option<String>, String)>::new();
    for object in &args.proposed_objects {
        if let Some(payload) = &object.payload {
            inspect_refs(payload, Some(&object.local_ref), &mut inspect);
        }
        if let Some(changes) = &object.changes {
            inspect_refs(changes, Some(&object.local_ref), &mut inspect);
        }
    }
    for relation in &args.proposed_relations {
        relation
            .source
            .iter()
            .for_each(|value| inspect.push((Some(relation.local_ref.clone()), value.clone())));
        relation
            .target
            .iter()
            .for_each(|value| inspect.push((Some(relation.local_ref.clone()), value.clone())));
        inspect.extend(
            relation
                .evidence_spans
                .iter()
                .map(|value| (Some(relation.local_ref.clone()), value.clone())),
        );
    }
    for (owner, value) in inspect {
        if let Some(local) = value.strip_prefix("$new:") {
            if !local_refs.contains(local) {
                result.push(Diagnostic {
                    level: Level::Error,
                    code: "dangling_new_ref".into(),
                    message: format!("$new:{local} does not name an item in this version"),
                    local_ref: owner,
                });
            }
        } else if !value.starts_with("mathscape:") && !value.is_empty() {
            result.push(Diagnostic {
                level: Level::Warn,
                code: "bare_local_ref_deprecated".into(),
                message: "bare local_ref is accepted for compatibility; use $new:<local_ref>"
                    .into(),
                local_ref: owner,
            });
        }
    }
    result
}

fn inspect_refs(value: &Value, owner: Option<&str>, output: &mut Vec<(Option<String>, String)>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "parent" | "source" | "target")
                    && value.as_str().is_some()
                {
                    output.push((owner.map(str::to_owned), value.as_str().unwrap().to_owned()));
                } else if key == "source_spans" || key == "evidence_spans" {
                    if let Some(values) = value.as_array() {
                        output.extend(
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|value| (owner.map(str::to_owned), value.to_owned())),
                        );
                    }
                } else {
                    inspect_refs(value, owner, output);
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| inspect_refs(value, owner, output)),
        _ => {}
    }
}

fn check_local_ref(result: &mut Vec<Diagnostic>, refs: &mut BTreeSet<String>, local_ref: &str) {
    // valid_temp_id bounds the length (64) and the character set.
    if !valid_temp_id(local_ref) || !refs.insert(local_ref.to_owned()) {
        result.push(Diagnostic {
            level: Level::Error,
            code: "invalid_local_ref".to_owned(),
            message: "local_ref must be unique and match [A-Za-z0-9][A-Za-z0-9_.-]{0,63}"
                .to_owned(),
            local_ref: Some(local_ref.to_owned()),
        });
    }
}
/// One Markdown snapshot read once per proposal: bytes, anchored blocks,
/// page markers and its PDF mapping.
struct SnapshotContext {
    source: String,
    blocks: Vec<MarkdownBlock>,
    /// `(marker block start, marker block end, zero-based PDF page index)`.
    /// Marker's `{N}------` ids are zero-based PDF page indices.
    markers: Vec<(u64, u64, u32)>,
    /// The unique source PDF and the snapshot's 1-based covered pages.
    pdf: Result<(StableId, Option<(u32, u32)>), String>,
}

fn load_snapshot(
    workspace: &McpWorkspace,
    store: &SqliteArtifactStore,
    artifact: &StableId,
) -> Result<SnapshotContext, Diagnostic> {
    let catalog = store
        .read_catalog_snapshot()
        .map_err(|error| Diagnostic::error("snapshot_not_found", error.to_string(), None))?;
    let Some(record) = catalog
        .artifacts()
        .iter()
        .find(|record| record.entity().header().stable_id() == artifact)
    else {
        return Err(Diagnostic::error(
            "snapshot_not_found",
            "artifact_id is not a registered immutable artifact",
            None,
        ));
    };
    if record.entity().payload().media_type().as_str() != "text/markdown" {
        return Err(Diagnostic::error(
            "snapshot_not_immutable",
            "SourceSpan requires a text/markdown snapshot",
            None,
        ));
    }
    let size = record.entity().payload().byte_size();
    if size > MAX_SNAPSHOT_BYTES {
        return Err(Diagnostic::error(
            "snapshot_not_immutable",
            "Markdown snapshot exceeds the bounded locator validation size",
            None,
        ));
    }
    let pool = workspace
        .pool()
        .map_err(|error| Diagnostic::error("snapshot_not_found", error.to_string(), None))?;
    let bytes = pool
        .read_bytes(artifact, 0, size as u32)
        .map_err(|error| Diagnostic::error("snapshot_not_found", error.to_string(), None))?;
    let source = String::from_utf8(bytes).map_err(|_| {
        Diagnostic::error(
            "snapshot_not_immutable",
            "Markdown snapshot is not valid UTF-8",
            None,
        )
    })?;
    let anchored = anchor_markdown(artifact.clone(), None, source.clone())
        .map_err(|error| Diagnostic::error("snapshot_not_immutable", error.to_string(), None))?;
    let markers = anchored
        .blocks
        .iter()
        .filter(|block| block.kind == PAGE_MARKER_STRUCTURAL_KIND)
        .filter_map(|block| {
            let text = source.get(block.range.start as usize..block.range.end as usize)?;
            page_marker_id(text).map(|page| (block.range.start, block.range.end, page))
        })
        .collect();
    let canonical = store.list_canonical_markdown().unwrap_or_default();
    let mut pdfs = BTreeMap::<String, (StableId, Option<(u32, u32)>)>::new();
    for view in &canonical {
        if let Some(segment) = view
            .segments
            .iter()
            .find(|segment| &segment.markdown_artifact_id == artifact)
        {
            pdfs.insert(
                view.source_pdf_artifact_id.to_string(),
                (
                    view.source_pdf_artifact_id.clone(),
                    segment.pages.map(|pages| (pages.from, pages.to)),
                ),
            );
        } else if view.head_artifact_id.as_ref() == Some(artifact) {
            pdfs.entry(view.source_pdf_artifact_id.to_string())
                .or_insert((view.source_pdf_artifact_id.clone(), None));
        }
    }
    let pdf = match pdfs.len() {
        1 => Ok(pdfs.into_values().next().expect("one entry")),
        0 => Err("快照不属于任何 canonical Markdown 视图，无法映射到 PDF".to_owned()),
        _ => Err("快照属于多个 PDF 的 canonical 视图，映射不唯一".to_owned()),
    };
    Ok(SnapshotContext {
        source,
        blocks: anchored.blocks,
        markers,
        pdf,
    })
}

/// Maps `[start, end)` to exactly one PDF page, or explains why not.
fn map_page(context: &SnapshotContext, range: &ByteRangeInput) -> Result<PageCoordinate, String> {
    let (pdf, pages) = context.pdf.clone()?;
    let page = if context.markers.is_empty() {
        // No page markers: only a single-page snapshot maps uniquely.
        // Canonical segment pages are 1-based; PDF page indices 0-based.
        match pages {
            Some((from, to)) if from == to && from >= 1 => (from - 1, None),
            Some(_) => return Err("快照覆盖多页且没有分页标记，页映射不唯一".to_owned()),
            None => return Err("快照没有页范围也没有分页标记".to_owned()),
        }
    } else {
        if context
            .markers
            .iter()
            .any(|(start, _, _)| *start > range.start && *start < range.end)
        {
            return Err("范围跨越分页标记，覆盖多页".to_owned());
        }
        // Content before the first marker belongs to the first marker's page.
        let marker = context
            .markers
            .iter()
            .rev()
            .find(|(start, _, _)| *start <= range.start)
            .or_else(|| context.markers.first())
            .map(|(_, _, page)| *page)
            .expect("markers is non-empty");
        if let Some((from, to)) = pages
            && !(from.saturating_sub(1) <= marker && marker <= to.saturating_sub(1))
        {
            return Err(format!(
                "分页标记 {marker}（0 基）不在快照覆盖的第 {from}–{to} 页（1 基）内"
            ));
        }
        (marker, Some(marker))
    };
    PageCoordinate::new(pdf, page.0, page.1, None).map_err(|error| error.to_string())
}

/// Validate and materialize the server-owned SourceSpan fields. This reads
/// only the immutable artifact pool and never writes the map. Every span is
/// either materialized or explained by an error diagnostic; none is dropped.
fn materialize_span_drafts(
    workspace: &McpWorkspace,
    store: &SqliteArtifactStore,
    map_snapshot: &MapEditSnapshot,
    args: &ProposeArgs,
    ids: &BTreeMap<String, String>,
) -> (BTreeMap<String, SourceSpanDraft>, Vec<Diagnostic>) {
    let mut drafts = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut contexts = HashMap::<StableId, Result<SnapshotContext, Diagnostic>>::new();

    for object in &args.proposed_objects {
        if object.kind != "source_span" || object.op != "create" {
            continue;
        }
        let local = Some(object.local_ref.as_str());
        let Some(payload) = object.payload.clone() else {
            continue;
        };
        let input: SourceSpanInput = match serde_json::from_value(payload) {
            Ok(input) => input,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "invalid_span_locator",
                    format!("source_span payload is invalid: {error}"),
                    local,
                ));
                continue;
            }
        };
        let artifact = match StableId::parse(input.artifact_id.clone()) {
            Ok(id) => id,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "snapshot_not_found",
                    error.to_string(),
                    local,
                ));
                continue;
            }
        };
        let context = contexts
            .entry(artifact.clone())
            .or_insert_with(|| load_snapshot(workspace, store, &artifact));
        let context = match context {
            Ok(context) => &*context,
            Err(error) => {
                let mut error = error.clone();
                error.local_ref = Some(object.local_ref.clone());
                diagnostics.push(error);
                continue;
            }
        };
        let range = &input.md_byte_range;
        if range.start >= range.end || range.end > context.source.len() as u64 {
            diagnostics.push(Diagnostic::error(
                "range_out_of_bounds",
                "md_byte_range must be a non-empty range inside the snapshot",
                local,
            ));
            continue;
        }
        let (start, end) = (range.start as usize, range.end as usize);
        if !context.source.is_char_boundary(start) || !context.source.is_char_boundary(end) {
            diagnostics.push(Diagnostic::error(
                "invalid_utf8_boundary",
                "md_byte_range must start and end on UTF-8 boundaries",
                local,
            ));
            continue;
        }
        // Cut [start, end) first, then compare: exact, else normalized on
        // both sides. Never search the rest of the snapshot.
        let selected = &context.source[start..end];
        if !quote_matches_range(selected, &input.quote) {
            diagnostics.push(Diagnostic::error(
                "quote_mismatch",
                "quote does not match the text of the supplied byte range",
                local,
            ));
            continue;
        }
        let block_ids = context
            .blocks
            .iter()
            .filter(|block| {
                block.kind != PAGE_MARKER_STRUCTURAL_KIND
                    && block.range.start < range.end
                    && block.range.end > range.start
            })
            .map(|block| block.id.clone())
            .collect::<Vec<_>>();
        if input.granularity == SpanGranularity::Block && block_ids.is_empty() {
            diagnostics.push(Diagnostic::error(
                "block_locator_unavailable",
                "block granularity requires a canonical Markdown block in the range",
                local,
            ));
            continue;
        }
        let page = if input.granularity == SpanGranularity::Page {
            match map_page(context, range) {
                Ok(page) => Some(page),
                Err(reason) => {
                    diagnostics.push(Diagnostic::error(
                        "page_mapping_unavailable",
                        format!("page granularity requires one uniquely mapped PDF page: {reason}"),
                        local,
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        // Same semantics as discovery: `fingerprint(&text)`.
        let hash = source_text_hash(selected);
        let markdown = MarkdownLocator::new(
            artifact,
            ByteRange::new(range.start, range.end).expect("validated range"),
            None,
        );
        let draft = SourceSpanDraft {
            granularity: input.granularity,
            layout: LayoutKind::Other,
            page,
            bounding_boxes: vec![],
            marker_block_ids: block_ids,
            markdown: Some(markdown),
            exact_text_artifact: None,
            normalized_text_hash: Some(hash),
        };
        if let Err(error) = validate_span_draft(&draft) {
            diagnostics.push(Diagnostic::error("invalid_span_locator", error, local));
            continue;
        }
        let span_id = ids.get(&object.local_ref).cloned().unwrap_or_default();
        if let Some(existing) = map_snapshot
            .objects
            .iter()
            .find(|candidate| candidate.object_id == span_id)
        {
            if is_retired(&existing.lifecycle) {
                diagnostics.push(Diagnostic::error(
                    "span_inactive",
                    "同 id 的 SourceSpan 已退役；需要显式重新激活操作",
                    local,
                ));
                continue;
            }
            diagnostics.push(Diagnostic::with_level(
                Level::Info,
                "span_already_exists",
                "SourceSpan 已存在，将作为已有证据引用",
                local,
            ));
            if existing.payload["payload"]["granularity"] != json!(input.granularity) {
                diagnostics.push(Diagnostic::with_level(
                    Level::Warn,
                    "span_granularity_ignored",
                    "已有 SourceSpan 的 granularity 优先于本次提议",
                    local,
                ));
            }
        }
        drafts.insert(object.local_ref.clone(), draft);
    }
    // A newly proposed span must have a consumer in this version. Existing
    // spans may be referenced directly and therefore do not trigger this.
    let referenced = collect_span_references(args);
    for object in &args.proposed_objects {
        if object.kind == "source_span"
            && object.op == "create"
            && !referenced.contains(&object.local_ref)
        {
            diagnostics.push(Diagnostic::error(
                "unused_span",
                "new SourceSpan is not referenced by a unit or relation",
                Some(&object.local_ref),
            ));
        }
    }
    (drafts, diagnostics)
}

/// Runs the domain constructor so the draft can never fail later at write
/// time (page granularity needs a page, block needs a locator, ...).
fn validate_span_draft(draft: &SourceSpanDraft) -> Result<(), String> {
    SourceSpan::new(
        draft.granularity,
        draft.layout,
        draft.page.clone(),
        draft.bounding_boxes.clone(),
        draft.marker_block_ids.clone(),
        draft.markdown.clone(),
        draft.exact_text_artifact.clone(),
        draft.normalized_text_hash.clone(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn is_retired(lifecycle: &str) -> bool {
    matches!(lifecycle, "deprecated" | "superseded" | "rejected")
}

fn collect_span_references(args: &ProposeArgs) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for object in &args.proposed_objects {
        if object.kind != "source_unit" {
            continue;
        }
        if let Some(payload) = &object.payload
            && let Some(values) = payload.get("source_spans").and_then(Value::as_array)
        {
            refs.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(reference_local_key),
            );
        }
        if let Some(changes) = &object.changes
            && let Some(values) = changes.get("source_spans").and_then(Value::as_array)
        {
            refs.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(reference_local_key),
            );
        }
    }
    for relation in &args.proposed_relations {
        refs.extend(
            relation
                .evidence_spans
                .iter()
                .filter_map(|value| reference_local_key(value)),
        );
    }
    refs
}

fn reference_local_key(value: &str) -> Option<String> {
    value
        .strip_prefix("$new:")
        .or(Some(value))
        .map(str::to_owned)
}

fn compile_operations(
    args: &ProposeArgs,
    ids: &BTreeMap<String, String>,
    span_drafts: &BTreeMap<String, SourceSpanDraft>,
) -> Result<Vec<UserChangeOperation>, String> {
    let mut operations = Vec::new();
    for object in &args.proposed_objects {
        if !matches!(object.kind.as_str(), "source_unit" | "source_span") {
            continue;
        }
        let object_id = object
            .target_id
            .clone()
            .or_else(|| ids.get(&object.local_ref).cloned())
            .ok_or_else(|| format!("missing object id for {}", object.local_ref))?;
        if object.kind == "source_span" {
            let draft = span_drafts
                .get(&object.local_ref)
                .cloned()
                .ok_or_else(|| format!("missing validated span for {}", object.local_ref))?;
            operations.push(UserChangeOperation::CreateSourceSpan { object_id, draft });
            continue;
        }
        match object.op.as_str() {
            "create" => {
                let mut payload = object.payload.clone().ok_or("missing payload")?;
                resolve_payload_refs(&mut payload, ids)?;
                let draft: SourceUnitDraft =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                operations.push(UserChangeOperation::CreateSourceUnit { object_id, draft });
            }
            "edit" => {
                let mut changes = object.changes.clone().ok_or("missing changes")?;
                resolve_change_refs(&mut changes, ids)?;
                let changes: SourceUnitChanges =
                    serde_json::from_value(changes).map_err(|e| e.to_string())?;
                operations.push(UserChangeOperation::EditSourceUnit {
                    object_id,
                    expected_revision: object
                        .expected_revision
                        .clone()
                        .ok_or("missing expected_revision")?,
                    changes,
                });
            }
            _ => return Err(format!("unsupported source_unit op {}", object.op)),
        }
    }
    for relation in &args.proposed_relations {
        if relation.kind != "source_relation" {
            continue;
        }
        let object_id = relation
            .target_id
            .clone()
            .or_else(|| ids.get(&relation.local_ref).cloned())
            .ok_or_else(|| format!("missing relation id for {}", relation.local_ref))?;
        match relation.op.as_str() {
            "create" => operations.push(UserChangeOperation::CreateSourceRelation {
                object_id,
                draft: SourceRelationDraft {
                    relation: relation.relation.ok_or("missing relation")?,
                    source: resolve_ref(relation.source.as_deref().ok_or("missing source")?, ids)?,
                    target: resolve_ref(relation.target.as_deref().ok_or("missing target")?, ids)?,
                    evidence_spans: relation
                        .evidence_spans
                        .iter()
                        .map(|value| resolve_ref(value, ids))
                        .collect::<Result<Vec<_>, _>>()?,
                },
            }),
            "edit" => {
                let mut changes = relation.changes.clone().ok_or("missing changes")?;
                resolve_change_refs(&mut changes, ids)?;
                let changes: SourceRelationChanges =
                    serde_json::from_value(changes).map_err(|e| e.to_string())?;
                operations.push(UserChangeOperation::EditSourceRelation {
                    object_id,
                    expected_revision: relation
                        .expected_revision
                        .clone()
                        .ok_or("missing expected_revision")?,
                    changes,
                });
            }
            _ => return Err(format!("unsupported source_relation op {}", relation.op)),
        }
    }
    Ok(operations)
}

fn resolve_payload_refs(payload: &mut Value, ids: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(parent) = payload.get_mut("parent")
        && let Some(value) = parent.as_str()
    {
        *parent = Value::String(resolve_ref(value, ids)?);
    }
    if let Some(spans) = payload.get_mut("source_spans")
        && let Some(values) = spans.as_array_mut()
    {
        for value in values {
            if let Some(text) = value.as_str() {
                *value = Value::String(resolve_ref(text, ids)?);
            }
        }
    }
    Ok(())
}

fn resolve_change_refs(changes: &mut Value, ids: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(parent) = changes.get_mut("parent")
        && parent.get("action").and_then(Value::as_str) == Some("set")
        && let Some(value) = parent.get_mut("value")
        && let Some(value_text) = value.as_str()
    {
        let resolved = resolve_ref(value_text, ids)?;
        *value = Value::String(resolved);
    }
    for key in ["source", "target"] {
        if let Some(value) = changes.get_mut(key)
            && let Some(value_text) = value.as_str()
        {
            let resolved = resolve_ref(value_text, ids)?;
            *value = Value::String(resolved);
        }
    }
    if let Some(spans) = changes.get_mut("source_spans")
        && let Some(values) = spans.as_array_mut()
    {
        for value in values {
            if let Some(text) = value.as_str() {
                *value = Value::String(resolve_ref(text, ids)?);
            }
        }
    }
    if let Some(spans) = changes.get_mut("evidence_spans")
        && let Some(values) = spans.as_array_mut()
    {
        for value in values {
            if let Some(text) = value.as_str() {
                *value = Value::String(resolve_ref(text, ids)?);
            }
        }
    }
    Ok(())
}

fn resolve_ref(value: &str, ids: &BTreeMap<String, String>) -> Result<String, String> {
    let local = value.strip_prefix("$new:").unwrap_or(value);
    if let Some(id) = ids.get(local) {
        Ok(id.clone())
    } else {
        StableId::parse(value.to_owned())
            .map(|id| id.to_string())
            .map_err(|error| error.to_string())
    }
}

fn allocate_ids(
    change_set_id: &str,
    args: &ProposeArgs,
) -> Result<BTreeMap<String, String>, ToolError> {
    let mut ids = BTreeMap::new();
    for object in &args.proposed_objects {
        let id = if object.op == "edit" {
            object.target_id.clone().ok_or_else(|| {
                ToolError::new(ToolErrorCode::InvalidArg, "edit requires target_id")
            })?
        } else if object.kind == "source_span" {
            object
                .payload
                .as_ref()
                .and_then(|payload| serde_json::from_value::<SourceSpanInput>(payload.clone()).ok())
                .and_then(|span| {
                    let artifact = StableId::parse(span.artifact_id).ok()?;
                    let range = TextByteRange {
                        start: span.md_byte_range.start,
                        end: span.md_byte_range.end,
                    };
                    (range.start < range.end)
                        .then(|| markdown_span_id(&artifact, &range).to_string())
                })
                .unwrap_or_else(|| generated_id("source-span", change_set_id, &object.local_ref))
        } else {
            generated_id("source-unit", change_set_id, &object.local_ref)
        };
        if ids.insert(object.local_ref.clone(), id).is_some() {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "duplicate local_ref",
            ));
        }
    }
    for relation in &args.proposed_relations {
        let id = if relation.op == "edit" {
            relation.target_id.clone().ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::InvalidArg,
                    "relation edit requires target_id",
                )
            })?
        } else {
            generated_id("source-relation", change_set_id, &relation.local_ref)
        };
        if ids.insert(relation.local_ref.clone(), id).is_some() {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "duplicate local_ref",
            ));
        }
    }
    Ok(ids)
}

fn resolve_version(
    store: &mathscape_store_sqlite::SqliteArtifactStore,
    investigation_id: &str,
    args: &ProposeArgs,
) -> Result<(String, i64, Option<i64>), ToolError> {
    let Some(supersedes) = &args.supersedes else {
        let id = format!(
            "mathscape:change-set/{}",
            digest(format!("{investigation_id}\0{}", args.request_id).as_bytes())
        );
        return Ok((id, 1, None));
    };
    let Some((active, _)) = store
        .proposed_change(investigation_id, &supersedes.change_set_id, None)
        .map_err(internal)?
    else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "supersedes change_set was not found",
        )
        .with_details(json!({ "reason": "supersede_conflict" })));
    };
    let active_proposal_fingerprint = active
        .result
        .get("proposal_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if active.version != supersedes.version
        || active_proposal_fingerprint != supersedes.proposal_fingerprint
        || !matches!(active.state.as_str(), "staged" | "blocked")
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "supersedes does not match the active proposal",
        )
        .with_details(json!({ "reason": "supersede_conflict" })));
    }
    Ok((
        supersedes.change_set_id.clone(),
        active.version + 1,
        Some(active.version),
    ))
}

fn proposal_fingerprint(args: &ProposeArgs) -> Result<String, ToolError> {
    let mut value = serde_json::to_value(args).map_err(internal)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("request_id");
        object.remove("supersedes");
    }
    Ok(format!(
        "sha256:{}",
        digest(canonical_json(&value).as_bytes())
    ))
}

fn request_fingerprint(args: &ProposeArgs) -> Result<String, ToolError> {
    let value = serde_json::to_value(args).map_err(internal)?;
    Ok(format!(
        "sha256:{}",
        digest(canonical_json(&value).as_bytes())
    ))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let mut output = String::from("{");
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).unwrap());
                output.push(':');
                output.push_str(&canonical_json(value));
            }
            output.push('}');
            output
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).unwrap(),
    }
}

fn request_id_for_planner(round: &str, request_id: &str) -> String {
    format!(
        "mathscape:proposal-request/{}",
        digest(format!("{round}\0{request_id}").as_bytes())
    )
}

fn generated_id(kind: &str, change_set_id: &str, local_ref: &str) -> String {
    format!(
        "mathscape:{kind}/{}",
        digest(format!("{change_set_id}\0{local_ref}").as_bytes())
    )
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn reverse_lookup(ids: &BTreeMap<String, String>, object_id: Option<&str>) -> Option<String> {
    object_id.and_then(|object_id| {
        ids.iter()
            .find_map(|(local_ref, id)| (id == object_id).then(|| local_ref.clone()))
    })
}

fn collect_agent_debts(args: &ProposeArgs) -> Vec<Value> {
    args.proposed_objects
        .iter()
        .flat_map(|item| {
            item.debts.iter().map(|debt| {
                json!({
                    "kind": "agent_declared",
                    "local_ref": item.local_ref,
                    "value": debt
                })
            })
        })
        .chain(args.proposed_relations.iter().flat_map(|item| {
            item.debts.iter().map(|debt| {
                json!({
                    "kind": "agent_declared",
                    "local_ref": item.local_ref,
                    "value": debt
                })
            })
        }))
        .collect()
}
/// Records which other change-set versions this proposal builds on.
///
/// Every `mathscape:` reference must be either an object of this proposal,
/// present in the accepted map, or staged by an active version of the same
/// investigation; otherwise `dangling_staged_ref`. A reference staged only by
/// a `blocked` version yields `dependency_blocked`. A span id staged by
/// several versions depends on the earliest creator (span ids are
/// content-addressed, so they are the same span).
fn resolve_staged_dependencies(
    versions: &[PendingChangeSet],
    snapshot: &MapEditSnapshot,
    investigation_id: &str,
    current_change_set_id: &str,
    current_version: i64,
    args: &ProposeArgs,
    ids: &BTreeMap<String, String>,
) -> (Vec<ChangeSetDependency>, Vec<Diagnostic>) {
    let own: BTreeSet<&String> = ids.values().collect();
    let in_head: BTreeSet<&str> = snapshot
        .objects
        .iter()
        .map(|object| object.object_id.as_str())
        .collect();
    let mut candidates = versions
        .iter()
        .filter(|item| {
            item.record.change_set_id != current_change_set_id
                && matches!(item.record.state.as_str(), "staged" | "blocked")
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        (
            a.record.state != "staged",
            &a.record.created_at,
            &a.record.change_set_id,
        )
            .cmp(&(
                b.record.state != "staged",
                &b.record.created_at,
                &b.record.change_set_id,
            ))
    });
    let mut dependencies = BTreeMap::<ChangeSetKey, BTreeSet<String>>::new();
    let mut diagnostics = Vec::new();
    for value in proposal_reference_values(args) {
        if own.contains(&value) || in_head.contains(value.as_str()) {
            continue;
        }
        let target = candidates.iter().find(|candidate| {
            staged_ids_of(&candidate.record).any(|id| id == value)
                || operation_targets(&candidate.record).contains(&value)
        });
        let Some(target) = target else {
            diagnostics.push(Diagnostic::error(
                "dangling_staged_ref",
                format!("{value} 既不在正式地图中，也不是本调查活动 staged 提案的对象"),
                None,
            ));
            continue;
        };
        if target.record.state != "staged" {
            diagnostics.push(Diagnostic::error(
                "dependency_blocked",
                format!(
                    "{value} 由 {}@{} 提出，该版本当前 {}",
                    target.record.change_set_id, target.record.version, target.record.state
                ),
                None,
            ));
        }
        dependencies
            .entry((target.record.change_set_id.clone(), target.record.version))
            .or_default()
            .insert(value);
    }
    let dependencies = dependencies
        .into_iter()
        .map(
            |((change_set_id, version), referenced)| ChangeSetDependency {
                dependent_change_set_id: current_change_set_id.to_owned(),
                dependent_version: current_version,
                depends_on_change_set_id: change_set_id,
                depends_on_version: version,
                investigation_id: investigation_id.to_owned(),
                referenced_ids: referenced.into_iter().collect(),
            },
        )
        .collect();
    (dependencies, diagnostics)
}

fn staged_ids_of(record: &ProposedChangeRecord) -> impl Iterator<Item = &str> {
    record
        .result
        .get("staged_ids")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|ids| ids.values().filter_map(Value::as_str))
}

/// Object ids created by a version's compiled operations (covers versions
/// whose `staged_ids` were cleared when they became blocked later).
fn operation_targets(record: &ProposedChangeRecord) -> BTreeSet<String> {
    crate::investigation::record_operations(record)
        .iter()
        .filter_map(|operation| match operation {
            UserChangeOperation::CreateSourceSpan { object_id, .. }
            | UserChangeOperation::CreateSourceUnit { object_id, .. }
            | UserChangeOperation::CreateSourceRelation { object_id, .. } => {
                Some(object_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn closure_diagnostic(error: &ClosureError) -> Diagnostic {
    Diagnostic::error(error.code(), error.message(), None)
}

fn proposal_reference_values(args: &ProposeArgs) -> Vec<String> {
    let mut refs = Vec::new();
    for object in &args.proposed_objects {
        object
            .payload
            .as_ref()
            .into_iter()
            .for_each(|value| collect_string_refs(value, &mut refs));
        object
            .changes
            .as_ref()
            .into_iter()
            .for_each(|value| collect_string_refs(value, &mut refs));
    }
    for relation in &args.proposed_relations {
        relation
            .source
            .iter()
            .chain(relation.target.iter())
            .for_each(|value| refs.push(value.clone()));
        refs.extend(relation.evidence_spans.iter().cloned());
    }
    refs.into_iter()
        .filter(|value| value.starts_with("mathscape:"))
        .collect()
}

fn collect_string_refs(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "parent" | "source" | "target")
                    || key == "source_spans"
                    || key == "evidence_spans"
                {
                    match value {
                        Value::String(value) => output.push(value.clone()),
                        Value::Array(values) => output
                            .extend(values.iter().filter_map(Value::as_str).map(str::to_owned)),
                        _ => {}
                    }
                } else {
                    collect_string_refs(value, output);
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_string_refs(value, output)),
        _ => {}
    }
}

fn diagnostic_value(item: &Diagnostic) -> Value {
    json!({
        "level": match item.level {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
        },
        "code": item.code,
        "message": item.message,
        "local_ref": item.local_ref
    })
}

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(Diagnostic::is_error)
}

fn internal(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(ToolErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf() -> StableId {
        StableId::parse("mathscape:artifact/sha256/".to_owned() + &"1".repeat(64)).unwrap()
    }

    fn context(markers: Vec<(u64, u64, u32)>, pages: Option<(u32, u32)>) -> SnapshotContext {
        SnapshotContext {
            source: String::new(),
            blocks: Vec::new(),
            markers,
            pdf: Ok((pdf(), pages)),
        }
    }

    fn range(start: u64, end: u64) -> ByteRangeInput {
        ByteRangeInput { start, end }
    }

    #[test]
    fn marker_ids_are_zero_based_pdf_indices_on_multi_page_snapshots() {
        // Pages 31–32 (1-based) were converted; Marker numbers them {30}, {31}.
        let ctx = context(vec![(0, 20, 30), (100, 120, 31)], Some((31, 32)));
        let first = map_page(&ctx, &range(25, 60)).unwrap();
        assert_eq!(first.pdf_page_index(), 30);
        assert_eq!(first.marker_page_id(), Some(30));
        let second = map_page(&ctx, &range(130, 160)).unwrap();
        assert_eq!(second.pdf_page_index(), 31);
        // Crossing the {31} marker spans two pages.
        assert!(map_page(&ctx, &range(60, 130)).is_err());
        // A marker outside the snapshot's covered pages is inconsistent.
        let bad = context(vec![(0, 20, 5)], Some((31, 32)));
        assert!(map_page(&bad, &range(25, 30)).is_err());
    }

    #[test]
    fn unmarked_snapshots_map_only_when_they_cover_one_page() {
        let single = map_page(&context(vec![], Some((7, 7))), &range(0, 5)).unwrap();
        assert_eq!(single.pdf_page_index(), 6);
        assert_eq!(single.marker_page_id(), None);
        assert!(map_page(&context(vec![], Some((7, 9))), &range(0, 5)).is_err());
        assert!(map_page(&context(vec![], None), &range(0, 5)).is_err());
        let mut unknown = context(vec![(0, 5, 1)], None);
        unknown.pdf = Err("no view".into());
        assert!(map_page(&unknown, &range(6, 9)).is_err());
    }
}
