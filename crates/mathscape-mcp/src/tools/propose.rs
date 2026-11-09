//! Phase2-0 source-layer proposal tools.
//!
//! `propose_changes` validates source-unit and source-relation operations with
//! the existing pure map planner, then persists only an auditable candidate.
//! `get_change_set` reads that candidate back; neither tool mutates the map.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::{
    ActorRef, SourceRelationChanges, SourceRelationDraft, SourceUnitChanges, SourceUnitDraft,
    StableId, UserChangeOperation, UserChangeRequest,
};
use mathscape_orchestrator::map_edit::plan_map_change;
use mathscape_store_sqlite::ProposedChangeRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{GET_CHANGE_SET, PROPOSE_CHANGES};
use crate::McpWorkspace;

const MAX_REQUEST_ID_CHARS: usize = 128;
const MAX_LOCAL_REF_CHARS: usize = 128;
const MAX_RATIONALE_CHARS: usize = 4000;
const MAX_ITEMS: usize = 1000;

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

#[derive(Clone, Debug)]
struct Diagnostic {
    code: String,
    message: String,
    local_ref: Option<String>,
}

pub(super) fn definition() -> Value {
    json!({
        "name": PROPOSE_CHANGES,
        "description": "Validate and persist a Phase2-0 source-layer proposal. It supports source_unit/source_relation create/edit only, returns staged or blocked, and never changes the accepted map. Requires a bound round token.",
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
    let investigation_id = round.to_owned();
    let proposal_fingerprint = proposal_fingerprint(&args)?;
    let request_fingerprint = request_fingerprint(&args)?;
    let mut store = workspace.store()?;

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
    let internal_request_id = request_id_for_planner(&round, &args.request_id);
    let actor = ActorRef::parse("agent:custom").map_err(internal)?;
    let snapshot = store.map_edit_snapshot().map_err(internal)?;
    let base_commit = snapshot.base_commit.clone();
    let mut diagnostics = validate_refs(&args.existing_refs);
    diagnostics.extend(compile_diagnostics(&args, &ids));

    let operations = if diagnostics.is_empty() {
        match compile_operations(&args, &ids) {
            Ok(operations) => operations,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    code: "invalid_operation".to_owned(),
                    message: error,
                    local_ref: None,
                });
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    if diagnostics.is_empty() {
        let request = UserChangeRequest {
            request_id: internal_request_id,
            base_commit: base_commit.clone(),
            reason: args.rationale.clone(),
            operations,
        };
        let planned = plan_map_change(&snapshot, &request, &actor);
        diagnostics.extend(
            planned
                .impact
                .diagnostics
                .into_iter()
                .map(|diagnostic| Diagnostic {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    local_ref: reverse_lookup(&ids, diagnostic.object_id.as_deref()),
                }),
        );
    }

    let agent_debts = collect_agent_debts(&args);
    let validation_debts = diagnostics
        .iter()
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
    let mut debts = validation_debts;
    debts.extend(
        args.unresolved
            .iter()
            .map(|item| json!({ "kind": "unresolved", "value": item })),
    );
    debts.extend(agent_debts);
    let status = if diagnostics.is_empty() {
        ToolStatus::Staged
    } else {
        ToolStatus::Blocked
    };
    let result = json!({
        "status": status,
        "change_set_id": change_set_id,
        "version": version,
        "proposal_fingerprint": proposal_fingerprint,
        "base_commit": base_commit,
        "staged_ids": if diagnostics.is_empty() { json!(ids) } else { Value::Null },
        "debts": debts,
        "diagnostics": diagnostics.iter().map(diagnostic_value).collect::<Vec<_>>(),
        "supersedes": args.supersedes.as_ref().map(|s| json!({
            "change_set_id": s.change_set_id,
            "version": s.version
        }))
    });
    let state = if diagnostics.is_empty() {
        "staged"
    } else {
        "blocked"
    };
    let record = ProposedChangeRecord {
        change_set_id,
        version,
        investigation_id: investigation_id.clone(),
        round_id: round.to_owned(),
        generation: round.to_owned(),
        request_id: args.request_id.clone(),
        request_fingerprint,
        proposal: serde_json::to_value(&args).map_err(internal)?,
        result: result.clone(),
        state: state.to_owned(),
        base_commit,
        created_at: Utc::now().to_rfc3339(),
    };
    store
        .insert_proposed_change(&record, supersedes_version)
        .map_err(|error| {
            if matches!(error, mathscape_store_sqlite::SqliteStoreError::Sqlite(_)) {
                internal(error)
            } else {
                ToolError::new(ToolErrorCode::InvalidArg, error.to_string())
                    .with_details(json!({ "reason": "supersede_conflict" }))
            }
        })?;
    Ok(result)
}

pub(super) fn get(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: GetArgs = super::parse_args(arguments)?;
    let Some(round) = workspace.round().filter(|value| !value.trim().is_empty()) else {
        return Ok(json!({
            "status": ToolStatus::NotFound,
            "change_set_id": args.change_set_id,
            "change_set": Value::Null
        }));
    };
    let store = workspace.store()?;
    let Some((record, versions)) = store
        .proposed_change(round, &args.change_set_id, args.version)
        .map_err(internal)?
    else {
        return Ok(json!({
            "status": ToolStatus::NotFound,
            "change_set_id": args.change_set_id,
            "change_set": Value::Null
        }));
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
        "proposal_fingerprint": record
            .result
            .get("proposal_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        "proposed_by": "agent:custom",
        "round_id": record.round_id,
        "proposal": record.proposal,
        "staged_ids": record.result.get("staged_ids").cloned().unwrap_or(Value::Null),
        "debts": record.result.get("debts").cloned().unwrap_or_else(|| json!([])),
        "diagnostics": record.result.get("diagnostics").cloned().unwrap_or_else(|| json!([])),
        "decision": Value::Null
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
        if object.kind != "source_unit" {
            result.push(Diagnostic {
                code: "kind_not_supported".to_owned(),
                message: "Phase2-0 only supports source_unit and source_relation".to_owned(),
                local_ref: Some(object.local_ref.clone()),
            });
        }
        if !matches!(object.op.as_str(), "create" | "edit") {
            result.push(Diagnostic {
                code: "operation_not_supported".to_owned(),
                message: "Phase2-0 only supports create and edit".to_owned(),
                local_ref: Some(object.local_ref.clone()),
            });
        }
        match object.op.as_str() {
            "create" if object.payload.is_none() => result.push(Diagnostic {
                code: "missing_payload".to_owned(),
                message: "create requires payload".to_owned(),
                local_ref: Some(object.local_ref.clone()),
            }),
            "edit" if object.target_id.is_none() || object.expected_revision.is_none() => result
                .push(Diagnostic {
                    code: "missing_revision_guard".to_owned(),
                    message: "edit requires target_id and expected_revision".to_owned(),
                    local_ref: Some(object.local_ref.clone()),
                }),
            _ => {}
        }
    }
    for relation in &args.proposed_relations {
        check_local_ref(&mut result, &mut refs, &relation.local_ref);
        if relation.kind != "source_relation" {
            result.push(Diagnostic {
                code: "kind_not_supported".to_owned(),
                message: "relation kind must be source_relation".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
        if !matches!(relation.op.as_str(), "create" | "edit") {
            result.push(Diagnostic {
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
                code: "missing_relation_fields".to_owned(),
                message: "relation create requires relation, source and target".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
        if relation.op == "edit"
            && (relation.target_id.is_none() || relation.expected_revision.is_none())
        {
            result.push(Diagnostic {
                code: "missing_revision_guard".to_owned(),
                message: "relation edit requires target_id and expected_revision".to_owned(),
                local_ref: Some(relation.local_ref.clone()),
            });
        }
    }
    for (local_ref, id) in ids {
        if local_ref.chars().count() > MAX_LOCAL_REF_CHARS
            || local_ref.chars().any(char::is_control)
        {
            result.push(Diagnostic {
                code: "invalid_local_ref".to_owned(),
                message: format!("local_ref {local_ref:?} is invalid"),
                local_ref: Some(local_ref.clone()),
            });
        }
        if StableId::parse(id.clone()).is_err() {
            result.push(Diagnostic {
                code: "invalid_staged_id".to_owned(),
                message: "server-generated staged id is invalid".to_owned(),
                local_ref: Some(local_ref.clone()),
            });
        }
    }
    result
}

fn check_local_ref(result: &mut Vec<Diagnostic>, refs: &mut BTreeSet<String>, local_ref: &str) {
    if local_ref.trim().is_empty()
        || local_ref.chars().count() > MAX_LOCAL_REF_CHARS
        || local_ref.chars().any(char::is_control)
        || !refs.insert(local_ref.to_owned())
    {
        result.push(Diagnostic {
            code: "invalid_local_ref".to_owned(),
            message: "local_ref must be unique, non-empty and bounded".to_owned(),
            local_ref: Some(local_ref.to_owned()),
        });
    }
}

fn compile_operations(
    args: &ProposeArgs,
    ids: &BTreeMap<String, String>,
) -> Result<Vec<UserChangeOperation>, String> {
    let mut operations = Vec::new();
    for object in &args.proposed_objects {
        if object.kind != "source_unit" {
            continue;
        }
        let object_id = object
            .target_id
            .clone()
            .or_else(|| ids.get(&object.local_ref).cloned())
            .ok_or_else(|| format!("missing object id for {}", object.local_ref))?;
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
                    evidence_spans: relation.evidence_spans.clone(),
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
    Ok(())
}

fn resolve_ref(value: &str, ids: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(id) = ids.get(value) {
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

fn diagnostic_value(item: &Diagnostic) -> Value {
    json!({
        "level": "error",
        "code": item.code,
        "message": item.message,
        "local_ref": item.local_ref
    })
}

fn internal(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(ToolErrorCode::Internal, error.to_string())
}
