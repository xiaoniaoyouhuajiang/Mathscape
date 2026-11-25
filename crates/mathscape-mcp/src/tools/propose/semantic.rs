//! Semantic-layer items of `propose_changes`
//! (docs/semantic-write-contract.md S4–S5).
//!
//! Only deterministic checks block. The Agent may stage shared objects,
//! alignments, map debts and dialect scopes, edit them within a whitelist
//! and propose retiring them; it can never author user-only or server-only
//! state (reviewed alignments, paid / waived debts, confirmed mathlib
//! baselines, trust).
use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{
    AlignmentChanges, AlignmentDraft, AlignmentRelation, DebtChanges, DebtDraft,
    DialectScopeChanges, DialectScopeDraft, MapEditObject, MapEditSnapshot,
    RepresentationBindingChanges, RepresentationBindingDraft, RepresentationCoreChanges,
    RepresentationCoreDraft, SemanticEntityChanges, SemanticEntityDraft, UserChangeOperation,
};
use serde_json::Value;

use super::{Diagnostic, Level, ProposeArgs, ProposedObject};

pub(super) const SEMANTIC_KINDS: [&str; 6] = [
    "semantic_entity",
    "alignment",
    "debt",
    "dialect_scope",
    "representation_core",
    "representation_binding",
];

pub(super) fn is_semantic(kind: &str) -> bool {
    SEMANTIC_KINDS.contains(&kind)
}

/// Stable-id kind segment of a newly created object (contract S3 & S14).
pub(super) fn id_kind(kind: &str) -> &'static str {
    match kind {
        "semantic_entity" => mathscape_domain::SEMANTIC_ENTITY_ID_KIND,
        "alignment" => mathscape_domain::ALIGNMENT_ID_KIND,
        "debt" => mathscape_domain::DEBT_ID_KIND,
        "representation_core" => mathscape_domain::REPRESENTATION_CORE_ID_KIND,
        "representation_binding" => mathscape_domain::REPRESENTATION_BINDING_ID_KIND,
        _ => mathscape_domain::DIALECT_SCOPE_ID_KIND,
    }
}

/// Map entity kind of a proposal item kind.
fn entity_kind(kind: &str) -> &'static str {
    match kind {
        "semantic_entity" => "SemanticEntity",
        "alignment" => "Alignment",
        "debt" => "Debt",
        "representation_core" => "RepresentationCore",
        "representation_binding" => "RepresentationBinding",
        _ => "DialectScope",
    }
}

/// Keys whose string values (or arrays of strings) are object references
/// in semantic payloads (contract S4.5 & S14).
pub(super) const REFERENCE_KEYS: [&str; 14] = [
    "home_theory",
    "uses_entities",
    "concept",
    "represented_object",
    "representation",
    "target_context",
    "dialect_scope",
    "debt",
    "see_also",
    "attached_to",
    "ref",
    "anchor_unit",
    "introduced_by_unit",
    "parent_scope",
];

const EDITABLE: [(&str, &[&str]); 6] = [
    (
        "semantic_entity",
        &[
            "preferred_labels",
            "aliases",
            "plain_label",
            "home_theory",
            "conventions",
            "mathlib_baseline",
            "description",
            "uses_entities",
            "exclusions",
        ],
    ),
    (
        "alignment",
        &[
            "relation",
            "source_context",
            "target_context",
            "target_branches",
            "parameter_map",
            "term_index_map",
            "required_assumptions",
            "lost_information",
            "evidence_spans",
            "status",
            "map_label",
            "plain_note",
        ],
    ),
    (
        "debt",
        &[
            "status",
            "paid_by",
            "obligation",
            "failure_note",
            "payoff_hint",
        ],
    ),
    ("dialect_scope", &["label", "conventions", "local_symbols"]),
    (
        "representation_core",
        &[
            "home_theory",
            "preferred_labels",
            "aliases",
            "plain_label",
            "carrier",
            "represented_object",
            "binders",
            "operations",
            "invariants_and_domain",
            "conventions",
            "mathlib_baseline",
            "uses_entities",
            "description",
        ],
    ),
    (
        "representation_binding",
        &[
            "symbol_map",
            "index_and_domain_adaptations",
            "lost_information",
            "evidence_spans",
            "status",
            "map_label",
            "plain_note",
        ],
    ),
];

fn error(code: &str, message: impl Into<String>, local_ref: &str) -> Diagnostic {
    Diagnostic::error(code, message, Some(local_ref))
}

fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(k, v)| k == key || contains_key(v, key)),
        Value::Array(values) => values.iter().any(|v| contains_key(v, key)),
        _ => false,
    }
}

/// Deterministic checks of one semantic item before compilation
/// (S5.1 checks 1, 5, 6, 7, 10 with their own codes).
pub(super) fn precheck(object: &ProposedObject, snapshot: &MapEditSnapshot) -> Vec<Diagnostic> {
    let lr = object.local_ref.as_str();
    let mut out = vec![];
    if !matches!(object.op.as_str(), "create" | "edit" | "retire") {
        out.push(error(
            "operation_not_supported",
            "semantic items support create, edit and retire",
            lr,
        ));
        return out;
    }
    let body = match object.op.as_str() {
        "create" => object.payload.as_ref(),
        "edit" => object.changes.as_ref(),
        _ => None,
    };
    if object.op == "retire" && (object.payload.is_some() || object.changes.is_some()) {
        out.push(error(
            "invalid_arg",
            "retire takes only target_id and expected_revision",
            lr,
        ));
    }
    if matches!(object.op.as_str(), "edit" | "retire")
        && let Some(target) = object.target_id.as_deref()
        && let Some(current) = snapshot.objects.iter().find(|o| o.object_id == target)
        && current.kind != entity_kind(&object.kind)
    {
        out.push(error(
            "invalid_reference_kind",
            format!("target_id is a {}, not a {}", current.kind, object.kind),
            lr,
        ));
    }
    let Some(body) = body else {
        if object.op == "edit" {
            out.push(error("missing_payload", "edit requires changes", lr));
        }
        return out;
    };
    if contains_key(body, "trust") {
        out.push(error(
            "forbidden_trust_claim",
            "trust is server-authored; proposals never set any trust axis",
            lr,
        ));
    }
    match body.get("relation").and_then(Value::as_str) {
        Some("same_as") => out.push(error(
            "same_as_forbidden",
            "same_as is not an alignment relation; use expresses / overlaps_with / … and say what does not match",
            lr,
        )),
        Some(relation)
            if object.kind == "alignment"
                && serde_json::from_value::<AlignmentRelation>(Value::from(relation)).is_err() =>
        {
            out.push(error(
                "invalid_relation",
                format!(
                    "relation {relation} is not one of the 11 alignment relations (restriction is written overlaps_with)"
                ),
                lr,
            ))
        }
        _ => {}
    }
    if object.kind == "semantic_entity"
        && object.op == "create"
        && let Some(kind) = body.get("semantic_kind").and_then(Value::as_str)
        && !matches!(
            kind,
            "concept" | "declaration" | "theory" | "problem_schema"
        )
    {
        out.push(error(
            "kind_not_supported",
            format!("semantic_kind {kind} is not open yet (concept, declaration, theory, problem_schema)"),
            lr,
        ));
    }
    if matches!(object.kind.as_str(), "alignment" | "representation_binding")
        && let Some(status) = body.get("status").and_then(Value::as_str)
        && !matches!(
            status,
            "unmatched"
                | "retrieved_candidate"
                | "context_checked"
                | "partially_aligned"
                | "ambiguous"
                | "disputed"
                | "incompatible"
        )
    {
        out.push(error(
            "forbidden_alignment_status",
            format!(
                "an Agent cannot set status {status}; reviewed / accepted are explicit user actions"
            ),
            lr,
        ));
    }
    if object.kind == "debt"
        && let Some(status) = body.get("status").and_then(Value::as_str)
        && !matches!(status, "open" | "claimed_paid")
    {
        out.push(error(
            "forbidden_debt_status",
            format!("an Agent writes at most claimed_paid; {status} is set by the user (or the system for obsolete)"),
            lr,
        ));
    }
    if matches!(
        object.kind.as_str(),
        "semantic_entity" | "representation_core"
    ) {
        let baseline = body.get("mathlib_baseline");
        // In changes the baseline is a FieldChange {action, value}.
        let baseline = baseline
            .and_then(|b| b.get("value").filter(|_| b.get("action").is_some()))
            .or(baseline);
        if let Some(baseline) = baseline.filter(|b| b.is_object()) {
            let status = baseline.get("status").and_then(Value::as_str);
            if status != Some("unchecked") || baseline.get("checked").is_some_and(|c| !c.is_null())
            {
                out.push(error(
                    "forbidden_baseline_status",
                    "mathlib_baseline.status must be unchecked; confirmed / not_found are written only by the server's mathlib re-check",
                    lr,
                ));
            }
        }
    }
    if object.op == "edit"
        && let Some(map) = body.as_object()
    {
        let allowed = EDITABLE
            .iter()
            .find(|(kind, _)| *kind == object.kind)
            .map(|(_, keys)| *keys)
            .unwrap_or_default();
        for key in map.keys().filter(|k| !allowed.contains(&k.as_str())) {
            out.push(error(
                "identity_field_edit",
                format!(
                    "{key} cannot be edited on a {}; propose a new object instead",
                    object.kind
                ),
                lr,
            ));
        }
    }
    if !out.is_empty() {
        return out;
    }
    // Typed shape (enums, required fields, field types).
    let mut resolved = body.clone();
    blank_new_refs(&mut resolved);
    let parsed = match (object.kind.as_str(), object.op.as_str()) {
        ("semantic_entity", "create") => {
            serde_json::from_value::<SemanticEntityDraft>(resolved).err()
        }
        ("semantic_entity", _) => serde_json::from_value::<SemanticEntityChanges>(resolved).err(),
        ("alignment", "create") => serde_json::from_value::<AlignmentDraft>(resolved).err(),
        ("alignment", _) => serde_json::from_value::<AlignmentChanges>(resolved).err(),
        ("debt", "create") => serde_json::from_value::<DebtDraft>(resolved).err(),
        ("debt", _) => serde_json::from_value::<DebtChanges>(resolved).err(),
        ("dialect_scope", "create") => serde_json::from_value::<DialectScopeDraft>(resolved).err(),
        ("dialect_scope", _) => serde_json::from_value::<DialectScopeChanges>(resolved).err(),
        ("representation_core", "create") => {
            serde_json::from_value::<RepresentationCoreDraft>(resolved).err()
        }
        ("representation_core", _) => {
            serde_json::from_value::<RepresentationCoreChanges>(resolved).err()
        }
        ("representation_binding", "create") => {
            serde_json::from_value::<RepresentationBindingDraft>(resolved).err()
        }
        _ => serde_json::from_value::<RepresentationBindingChanges>(resolved).err(),
    };
    if let Some(e) = parsed {
        let code = if e.to_string().contains("same_as") {
            "same_as_forbidden"
        } else if e.to_string().contains("unknown variant") && e.to_string().contains("relation") {
            "invalid_relation"
        } else {
            "invalid_arg"
        };
        out.push(error(code, format!("{} payload: {e}", object.kind), lr));
    }
    out
}

/// Replace `$new:` references by a syntactically valid placeholder so the
/// typed shape can be checked before ids are allocated.
fn blank_new_refs(value: &mut Value) {
    match value {
        Value::String(s) if s.starts_with("$new:") => *s = "mathscape:pending/ref".into(),
        Value::Array(values) => values.iter_mut().for_each(blank_new_refs),
        Value::Object(map) => map.values_mut().for_each(blank_new_refs),
        _ => {}
    }
}

/// Resolves every `$new:<local_ref>` anywhere in a semantic payload.
pub(super) fn resolve_new_refs(
    value: &mut Value,
    ids: &BTreeMap<String, String>,
) -> Result<(), String> {
    match value {
        Value::String(s) if s.starts_with("$new:") => {
            let local = &s["$new:".len()..];
            *s = ids
                .get(local)
                .cloned()
                .ok_or_else(|| format!("$new:{local} does not name an item in this version"))?;
        }
        Value::Array(values) => {
            for v in values {
                resolve_new_refs(v, ids)?;
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                resolve_new_refs(v, ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Compiles one semantic item to a planner operation.
pub(super) fn compile(
    object: &ProposedObject,
    object_id: String,
    ids: &BTreeMap<String, String>,
) -> Result<UserChangeOperation, String> {
    let body = |value: &Option<Value>| -> Result<Value, String> {
        let mut value = value.clone().ok_or("missing payload")?;
        resolve_new_refs(&mut value, ids)?;
        Ok(value)
    };
    let expected = || {
        object
            .expected_revision
            .clone()
            .ok_or_else(|| "missing expected_revision".to_owned())
    };
    let parse = |e: serde_json::Error| e.to_string();
    Ok(match (object.kind.as_str(), object.op.as_str()) {
        (_, "retire") => UserChangeOperation::RetireObject {
            object_id,
            expected_revision: expected()?,
        },
        ("semantic_entity", "create") => UserChangeOperation::CreateSemanticEntity {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("semantic_entity", "edit") => UserChangeOperation::EditSemanticEntity {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        ("alignment", "create") => UserChangeOperation::CreateAlignment {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("alignment", "edit") => UserChangeOperation::EditAlignment {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        ("debt", "create") => UserChangeOperation::CreateDebt {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("debt", "edit") => UserChangeOperation::EditDebt {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        ("dialect_scope", "create") => UserChangeOperation::CreateDialectScope {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("dialect_scope", "edit") => UserChangeOperation::EditDialectScope {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        ("representation_core", "create") => UserChangeOperation::CreateRepresentationCore {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("representation_core", "edit") => UserChangeOperation::EditRepresentationCore {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        ("representation_binding", "create") => UserChangeOperation::CreateRepresentationBinding {
            object_id,
            draft: serde_json::from_value(body(&object.payload)?).map_err(parse)?,
        },
        ("representation_binding", "edit") => UserChangeOperation::EditRepresentationBinding {
            object_id,
            expected_revision: expected()?,
            changes: serde_json::from_value(body(&object.changes)?).map_err(parse)?,
        },
        (kind, op) => return Err(format!("unsupported {kind} op {op}")),
    })
}

/// Non-blocking review items (S5.2) for this proposal's semantic objects,
/// computed over the head plus the planned changes.
pub(super) fn review_diagnostics(
    snapshot: &MapEditSnapshot,
    changed: &[MapEditObject],
    args: &ProposeArgs,
    ids: &BTreeMap<String, String>,
) -> Vec<Diagnostic> {
    let mut final_map: BTreeMap<String, MapEditObject> = snapshot
        .objects
        .iter()
        .cloned()
        .map(|o| (o.object_id.clone(), o))
        .collect();
    for o in changed {
        final_map.insert(o.object_id.clone(), o.clone());
    }
    let own: BTreeSet<String> = args
        .proposed_objects
        .iter()
        .filter(|o| is_semantic(&o.kind))
        .filter_map(|o| ids.get(&o.local_ref).cloned())
        .collect();
    let target = args.target_object.as_deref().and_then(|t| {
        let bare = t.strip_prefix("$new:").unwrap_or(t);
        ids.get(bare)
            .cloned()
            .or_else(|| t.starts_with("mathscape:").then(|| t.to_owned()))
    });
    mathscape_orchestrator::semantic_plan::review_items(&final_map, &own, target.as_deref())
        .into_iter()
        .map(|item| {
            let mut d = Diagnostic::with_level(
                if item.level == "warn" {
                    Level::Warn
                } else {
                    Level::Info
                },
                item.code,
                item.message,
                super::reverse_lookup(ids, Some(&item.object_id)).as_deref(),
            );
            // Proposal-wide organisational hints (such as multi_target_change_set)
            // are not complaints about an existing workspace object: keep
            // `target_id` only when `local_ref` points at an item of this proposal
            // or the code is specifically about an external object.
            if d.local_ref.is_some() || item.code != "multi_target_change_set" {
                d.target_id = Some(item.object_id);
            }
            d
        })
        .collect()
}
