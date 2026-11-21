//! Pure user-intent planning. Both preview and commit invoke this same code.
use mathscape_domain::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub struct PlannedMapChange {
    pub impact: ImpactPlan,
    pub patch: Option<MapPatch<MapEditPayload>>,
    /// Objects whose payload, endpoints or lifecycle differ from the input
    /// snapshot, with their proposed revision. Used by the read-only
    /// investigation projection; never written by itself.
    pub changed_objects: Vec<MapEditObject>,
}

/// Write order of one patch: SourceSpan first, then source units ordered by
/// parent depth in the final graph (parents before children), then
/// relations. Ties fall back to the stable id.
fn write_rank(final_map: &BTreeMap<String, MapEditObject>, o: &MapEditObject) -> (u8, usize) {
    match o.kind.as_str() {
        "SourceSpan" => (0, 0),
        "SourceSemanticUnit" => {
            let mut depth = 0;
            let mut seen = BTreeSet::from([o.object_id.clone()]);
            let mut parent = body(o)["parent_unit"].as_str();
            while let Some(id) = parent {
                if !seen.insert(id.to_owned()) {
                    break;
                }
                depth += 1;
                parent = final_map
                    .get(id)
                    .and_then(|p| body(p)["parent_unit"].as_str());
            }
            (1, depth)
        }
        "SourceUnitRelation" => (2, 0),
        _ => crate::semantic_plan::write_rank(final_map, o),
    }
}
/// Order in which [`plan_map_change`] applies one request's operations.
///
/// Validation runs once on the final graph, so the written order of a
/// request must not matter. The only operation that reads another one's
/// result while being applied is `CreateSourceUnit`, whose child inherits
/// the source context of a parent created in the same request. Spans are
/// therefore applied first, then created units parent-first (depth counted
/// over the request's own creations), then everything else; ties keep the
/// written order.
fn application_order(operations: &[UserChangeOperation]) -> Vec<usize> {
    let created_parents: BTreeMap<&str, Option<&str>> = operations
        .iter()
        .filter_map(|op| match op {
            UserChangeOperation::CreateSourceUnit { object_id, draft } => {
                Some((object_id.as_str(), draft.parent.as_deref()))
            }
            _ => None,
        })
        .collect();
    fn depth<'a>(created_parents: &BTreeMap<&'a str, Option<&'a str>>, id: &'a str) -> usize {
        let mut depth = 0;
        let mut seen = BTreeSet::from([id]);
        let mut parent = created_parents.get(id).copied().flatten();
        while let Some(current) = parent {
            if !created_parents.contains_key(current) || !seen.insert(current) {
                break;
            }
            depth += 1;
            parent = created_parents.get(current).copied().flatten();
        }
        depth
    }
    let mut order: Vec<(u8, usize, usize)> = operations
        .iter()
        .enumerate()
        .map(|(index, op)| match op {
            UserChangeOperation::CreateSourceSpan { .. } => (0, 0, index),
            UserChangeOperation::CreateSourceUnit { object_id, .. } => {
                (1, depth(&created_parents, object_id), index)
            }
            _ => (2, 0, index),
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, _, index)| index).collect()
}
pub fn edit_fingerprint(value: &impl serde::Serialize) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("serializable edit value"))
    )
}
fn active(o: &MapEditObject) -> bool {
    !matches!(
        o.lifecycle.as_str(),
        "deprecated" | "superseded" | "rejected"
    )
}
fn body(o: &MapEditObject) -> &Value {
    &o.payload["payload"]
}
fn source_context(o: &MapEditObject) -> Option<String> {
    let p = body(o);
    p["surface_content_artifact"]
        .as_str()
        .map(|id| format!("artifact:{id}"))
        .or_else(|| {
            p["edition_binding"]["edition"]
                .as_str()
                .map(|id| format!("edition:{id}"))
        })
        .or_else(|| {
            p["markdown"]["artifact"]
                .as_str()
                .map(|id| format!("artifact:{id}"))
        })
}
fn error(id: &str, code: &str, message: impl Into<String>) -> ImpactDiagnostic {
    ImpactDiagnostic {
        code: code.into(),
        object_id: Some(id.into()),
        message: message.into(),
    }
}
fn refs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn require(map: &BTreeMap<String, MapEditObject>, id: &str, kind: &str) -> bool {
    map.get(id).is_some_and(|o| o.kind == kind && active(o))
}
fn endpoint(o: &MapEditObject, role: &str) -> Option<String> {
    o.endpoints
        .iter()
        .find(|e| e.role().as_str() == role)
        .map(|e| e.entity().to_string())
}
fn endpoints(d: &SourceRelationDraft) -> Result<Vec<RelationEndpoint>, String> {
    let mut result = vec![];
    for (role, ids) in [
        ("source", vec![d.source.clone()]),
        ("target", vec![d.target.clone()]),
        ("evidence", d.evidence_spans.clone()),
    ] {
        for (i, id) in ids.iter().enumerate() {
            result.push(RelationEndpoint::new(
                EndpointRole::parse(role).map_err(|e| e.to_string())?,
                StableId::parse(id).map_err(|e| e.to_string())?,
                if role == "evidence" {
                    Some(i as u32)
                } else {
                    None
                },
            ));
        }
    }
    Ok(result)
}
fn trust(actor: &ActorRef) -> TrustRecord {
    TrustRecord::new(
        ProvenanceStatus::Located,
        ParseStatus::Parsed,
        SegmentationStatus::Proposed,
        TypingStatus::Unchecked,
        ProofStatus::None,
        AlignmentStatus::Unmatched,
        CurationStatus::Accepted,
        Confidence::new(0.0).unwrap(),
        vec![],
        vec![actor.clone()],
    )
}
fn apply_field(v: &mut Value, key: &str, change: &FieldChange<String>) {
    match change {
        FieldChange::Keep => {}
        FieldChange::Set(s) => v[key] = json!(s),
        FieldChange::Clear => v[key] = Value::Null,
    }
}

/// Integrity checks over a whole (final) graph. `retired` are the objects the
/// request itself retires. Run on the unmodified head (with nothing retired)
/// it reports the problems that already exist before a request.
fn final_graph_diagnostics(
    final_map: &BTreeMap<String, MapEditObject>,
    retired: &BTreeSet<String>,
) -> Vec<ImpactDiagnostic> {
    let mut diagnostics = crate::source_integrity::validate_live_references(final_map);
    diagnostics.extend(crate::semantic_plan::graph_diagnostics(final_map));
    for o in final_map.values().filter(|o| active(o)) {
        if o.kind == "SourceSemanticUnit" {
            let p = body(o);
            if p["display_title"]
                .as_str()
                .is_some_and(|s| s.trim().is_empty() || s.chars().any(char::is_control))
            {
                diagnostics.push(error(
                    &o.object_id,
                    "invalid_title",
                    "标题不能为空白或包含控制字符",
                ));
            }
            let spans = refs(&p["source_spans"]);
            if spans.is_empty()
                || spans.iter().collect::<BTreeSet<_>>().len() != spans.len()
                || spans.iter().any(|s| !require(final_map, s, "SourceSpan"))
            {
                diagnostics.push(error(
                    &o.object_id,
                    "invalid_evidence",
                    "需要有效且唯一的原文证据",
                ));
            }
            if p["surface_content_artifact"].is_null() {
                let contexts = spans
                    .iter()
                    .filter_map(|id| final_map.get(id))
                    .filter_map(source_context)
                    .collect::<BTreeSet<_>>();
                if contexts.len() > 1 {
                    diagnostics.push(error(
                        &o.object_id,
                        "mixed_source_context",
                        "一个来源单元不能混合多个 Markdown 来源",
                    ));
                }
            }
            let mut seen = BTreeSet::from([o.object_id.clone()]);
            let mut parent = p["parent_unit"].as_str();
            while let Some(id) = parent {
                if !require(final_map, id, "SourceSemanticUnit") {
                    diagnostics.push(error(
                        &o.object_id,
                        "invalid_parent",
                        "父单元不可用；请改挂、设为根节点或显式退役子单元",
                    ));
                    break;
                }
                if source_context(o) != final_map.get(id).and_then(source_context) {
                    diagnostics.push(error(
                        &o.object_id,
                        "cross_source_parent",
                        "父单元必须来自同一 Markdown 来源",
                    ));
                    break;
                }
                if !seen.insert(id.into()) {
                    diagnostics.push(error(&o.object_id, "parent_cycle", "父链不能形成环"));
                    break;
                }
                parent = body(&final_map[id])["parent_unit"].as_str();
            }
            for r in p["local_references"].as_array().into_iter().flatten() {
                if refs(&r["candidate_targets"])
                    .iter()
                    .any(|t| retired.contains(t))
                {
                    diagnostics.push(error(
                        &o.object_id,
                        "referenced_target_retired",
                        "此单元仍引用退役目标；需先处理引用或退役引用单元",
                    ));
                }
            }
        } else if o.kind == "SourceUnitRelation" {
            let source = endpoint(o, "source");
            let target = endpoint(o, "target");
            let evidence: Vec<_> = o
                .endpoints
                .iter()
                .filter(|e| e.role().as_str() == "evidence")
                .collect();
            if source == target
                || source
                    .as_ref()
                    .is_none_or(|s| !require(final_map, s, "SourceSemanticUnit"))
                || target
                    .as_ref()
                    .is_none_or(|s| !require(final_map, s, "SourceSemanticUnit"))
                || evidence.is_empty()
                || evidence
                    .iter()
                    .any(|e| !require(final_map, e.entity().as_str(), "SourceSpan"))
                || evidence
                    .iter()
                    .map(|e| e.entity())
                    .collect::<BTreeSet<_>>()
                    .len()
                    != evidence.len()
            {
                diagnostics.push(error(
                    &o.object_id,
                    "invalid_relation",
                    "关系需两个不同的活动单元和唯一的有效证据",
                ));
            }
        }
    }
    diagnostics
}

pub fn plan_map_change(
    snapshot: &MapEditSnapshot,
    request: &UserChangeRequest,
    actor: &ActorRef,
) -> PlannedMapChange {
    plan(snapshot, request, actor, false)
}

/// Like [`plan_map_change`], but a problem that already exists in the head on
/// an object the request neither touches nor changes does not block: it is
/// reported in [`ImpactPlan::head_issues`] instead. Used for staged proposals
/// (propose, preview, batch plan and accept), so an unrelated pre-existing
/// problem of the workspace never blocks every proposal. A problem the
/// request causes on an untouched object (one that is not already in the
/// head) still blocks.
pub fn plan_map_change_over_head(
    snapshot: &MapEditSnapshot,
    request: &UserChangeRequest,
    actor: &ActorRef,
) -> PlannedMapChange {
    plan(snapshot, request, actor, true)
}

/// The problems of the unmodified head graph, keyed by `(code, object_id)`.
pub fn head_integrity_diagnostics(snapshot: &MapEditSnapshot) -> Vec<ImpactDiagnostic> {
    let map: BTreeMap<_, _> = snapshot
        .objects
        .iter()
        .cloned()
        .map(|o| (o.object_id.clone(), o))
        .collect();
    final_graph_diagnostics(&map, &BTreeSet::new())
}

fn plan(
    snapshot: &MapEditSnapshot,
    request: &UserChangeRequest,
    actor: &ActorRef,
    tolerate_head: bool,
) -> PlannedMapChange {
    let mut diagnostics = vec![];
    let mut conflicted = false;
    if snapshot.base_commit != request.base_commit {
        conflicted = true;
        diagnostics.push(error("", "base_commit_conflict", "地图已更新，请重新预览"));
    }
    if StableId::parse(&request.request_id).is_err()
        || request.reason.trim().is_empty()
        || request.operations.is_empty()
        || request.operations.len() > 1000
    {
        diagnostics.push(error(
            "",
            "invalid_request",
            "需要有效 request_id、变更原因和 1–1000 项操作",
        ));
    }
    let original: BTreeMap<_, _> = snapshot
        .objects
        .iter()
        .cloned()
        .map(|o| (o.object_id.clone(), o))
        .collect();
    let mut final_map = original.clone();
    let mut touched = BTreeSet::new();
    for op in application_order(&request.operations)
        .into_iter()
        .map(|index| &request.operations[index])
    {
        let id = op.object_id();
        if StableId::parse(id).is_err() || !touched.insert(id.to_owned()) {
            diagnostics.push(error(
                id,
                "invalid_target",
                "对象标识无效或同一对象重复操作",
            ));
            continue;
        }
        let current = original.get(id);
        // SourceSpan identities are content-addressed. Re-proposing an
        // already active span is an idempotent reference rather than a
        // duplicate create operation.
        let existing_active_span = matches!(op, UserChangeOperation::CreateSourceSpan { .. })
            && current.is_some_and(|object| object.kind == "SourceSpan" && active(object));
        if let Some(expected) = op.expected_revision() {
            if !current.is_some_and(|o| o.revision == expected) {
                conflicted = true;
                diagnostics.push(error(
                    id,
                    "revision_conflict",
                    "目标 revision 已变化或对象不存在",
                ));
                continue;
            }
            if !active(current.unwrap()) {
                diagnostics.push(error(id, "inactive_target", "已退役对象不可编辑"));
                continue;
            }
        } else if current.is_some() && !existing_active_span {
            diagnostics.push(error(id, "existing_target", "创建目标已存在"));
            continue;
        }
        if !existing_active_span {
            let mut shell = current.cloned().unwrap_or(MapEditObject {
                object_id: id.into(),
                revision: String::new(),
                kind: String::new(),
                lifecycle: "active".into(),
                payload: Value::Null,
                endpoints: vec![],
            });
            if let Some(result) = crate::semantic_plan::apply(op, &mut shell, actor) {
                match result {
                    Ok(()) => {
                        final_map.insert(id.into(), shell);
                    }
                    Err(diagnostic) => diagnostics.push(diagnostic),
                }
                continue;
            }
        }
        let result = (|| -> Result<MapEditObject, String> {
            if existing_active_span {
                return Ok(current.cloned().expect("checked above"));
            }
            let mut out = current.cloned().unwrap_or(MapEditObject {
                object_id: id.into(),
                revision: String::new(),
                kind: String::new(),
                lifecycle: "active".into(),
                payload: Value::Null,
                endpoints: vec![],
            });
            match op {
                UserChangeOperation::CreateSourceUnit { draft, .. } => {
                    let spans = draft
                        .source_spans
                        .iter()
                        .map(StableId::parse)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    let unit = SourceSemanticUnit::new(
                        SourceEditionBinding::Unresolved,
                        draft.kind.into(),
                        draft.display_title.clone(),
                        None,
                        vec![],
                        draft
                            .parent
                            .as_ref()
                            .map(StableId::parse)
                            .transpose()
                            .map_err(|e| e.to_string())?,
                        spans,
                        DialectScopeBinding::Uncontextualized,
                        None,
                        None,
                        vec![],
                        draft.completeness,
                        true,
                        trust(actor),
                    )
                    .map_err(|e| e.to_string())?;
                    out.kind = "SourceSemanticUnit".into();
                    out.payload = serde_json::to_value(SourcePayload::SemanticUnit(unit)).unwrap();
                    // A created child inherits the source context of its
                    // selected parent. Root creation is checked below against
                    // the selected source spans.
                    if let Some(parent) = draft.parent.as_deref().and_then(|id| final_map.get(id)) {
                        if let Some(artifact) = body(parent)["surface_content_artifact"].as_str() {
                            out.payload["payload"]["surface_content_artifact"] = json!(artifact);
                        }
                        out.payload["payload"]["edition_binding"] =
                            body(parent)["edition_binding"].clone();
                    } else {
                        let contexts = draft
                            .source_spans
                            .iter()
                            .filter_map(|id| final_map.get(id))
                            .filter_map(source_context)
                            .collect::<BTreeSet<_>>();
                        if contexts.len() == 1
                            && let Some(artifact) = contexts
                                .iter()
                                .find_map(|value| value.strip_prefix("artifact:"))
                        {
                            out.payload["payload"]["surface_content_artifact"] = json!(artifact);
                        }
                    }
                }
                UserChangeOperation::CreateSourceRelation { draft, .. } => {
                    out.kind = "SourceUnitRelation".into();
                    out.endpoints = endpoints(draft)?;
                    out.payload = serde_json::to_value(SourcePayload::UnitRelation(
                        SourceUnitRelation::new(draft.relation.into(), trust(actor)),
                    ))
                    .unwrap();
                }
                UserChangeOperation::CreateSourceSpan { draft, .. } => {
                    let span = SourceSpan::new(
                        draft.granularity,
                        draft.layout,
                        draft.page.clone(),
                        draft.bounding_boxes.clone(),
                        draft.marker_block_ids.clone(),
                        draft.markdown.clone(),
                        draft.exact_text_artifact.clone(),
                        draft.normalized_text_hash.clone(),
                    )
                    .map_err(|error| error.to_string())?;
                    out.kind = "SourceSpan".into();
                    out.payload = serde_json::to_value(SourcePayload::Span(span)).unwrap();
                }
                UserChangeOperation::EditSourceUnit { changes, .. } => {
                    if out.kind != "SourceSemanticUnit" {
                        return Err("目标不是内容单元".into());
                    }
                    let p = &mut out.payload["payload"];
                    apply_field(p, "display_title", &changes.display_title);
                    apply_field(p, "parent_unit", &changes.parent);
                    if let Some(c) = changes.completeness {
                        p["completeness"] = json!(c);
                    }
                }
                UserChangeOperation::EditSourceRelation { changes, .. } => {
                    if out.kind != "SourceUnitRelation" {
                        return Err("目标不是持久化来源关系".into());
                    }
                    let relation: DiscoveryRelationKind = changes.relation.unwrap_or(
                        serde_json::from_value(out.payload["payload"]["relation"].clone())
                            .map_err(|e| e.to_string())?,
                    );
                    let draft = SourceRelationDraft {
                        relation,
                        source: changes
                            .source
                            .clone()
                            .or_else(|| endpoint(&out, "source"))
                            .ok_or("缺少 source")?,
                        target: changes
                            .target
                            .clone()
                            .or_else(|| endpoint(&out, "target"))
                            .ok_or("缺少 target")?,
                        evidence_spans: changes.evidence_spans.clone().unwrap_or_else(|| {
                            out.endpoints
                                .iter()
                                .filter(|e| e.role().as_str() == "evidence")
                                .map(|e| e.entity().to_string())
                                .collect()
                        }),
                    };
                    out.endpoints = endpoints(&draft)?;
                    out.payload["payload"]["relation"] = json!(relation);
                    // Changed evidence invalidates evidence-specific trust claims.
                    out.payload["payload"]["trust"] = json!(trust(actor));
                }
                UserChangeOperation::RetireObject { .. } => {
                    if !matches!(
                        out.kind.as_str(),
                        "SourceSemanticUnit" | "SourceUnitRelation" | "SourceSpan"
                    ) && !crate::semantic_plan::is_semantic_kind(&out.kind)
                    {
                        return Err("只允许退役来源单元、关系、原文证据和语义层对象".into());
                    }
                    out.lifecycle = "deprecated".into();
                }
                _ => unreachable!("semantic operations are planned by semantic_plan"),
            }
            Ok(out)
        })();
        match result {
            Ok(o) => {
                final_map.insert(id.into(), o);
            }
            Err(e) => diagnostics.push(error(id, "invalid_operation", e)),
        }
    }
    // Incident persisted relations retire with their endpoint; no traversal
    // through citations, no cascading of other content units.
    let retired: BTreeSet<_> = final_map
        .values()
        .filter(|o| touched.contains(&o.object_id) && !active(o))
        .map(|o| o.object_id.clone())
        .collect();
    for o in final_map.values_mut() {
        if o.kind == "SourceUnitRelation"
            && active(o)
            && o.endpoints
                .iter()
                .any(|e| retired.contains(e.entity().as_str()))
        {
            o.lifecycle = "deprecated".into();
        }
    }
    // Debts attached to a retired alignment or shared object become
    // obsolete (system transition) and retire with it.
    crate::semantic_plan::obsolete_orphaned_debts(&mut final_map);
    // Check final graph, including dependent objects not explicitly edited.
    diagnostics.extend(final_graph_diagnostics(&final_map, &retired));
    let mut operations = vec![];
    let mut changed_objects = vec![];
    let ranks: BTreeMap<String, (u8, usize)> = final_map
        .iter()
        .map(|(id, o)| (id.clone(), write_rank(&final_map, o)))
        .collect();
    let mut direct = vec![];
    let mut affected = vec![];
    for (id, o) in &mut final_map {
        let old = original.get(id);
        if old.is_some_and(|old| {
            old.payload == o.payload && old.endpoints == o.endpoints && old.lifecycle == o.lifecycle
        }) {
            continue;
        }
        o.revision = edit_fingerprint(&(
            "map-edit-revision-1",
            id,
            &o.payload,
            &o.endpoints,
            &o.lifecycle,
            &request.request_id,
            actor,
        ));
        let item = ImpactItem {
            object_id: id.clone(),
            kind: o.kind.clone(),
            action: if !active(o) {
                "retire"
            } else if old.is_none() {
                "create"
            } else {
                "revise"
            }
            .into(),
            reason: request.reason.clone(),
            current_revision: old.map(|v| v.revision.clone()),
            proposed_revision: Some(o.revision.clone()),
            display_title: crate::source_integrity::object_title(o),
            reason_codes: vec![],
            related_objects: original
                .values()
                .filter_map(|v| {
                    crate::source_integrity::source_references(v)
                        .ok()?
                        .into_iter()
                        .find(|(target, _)| target == id)
                        .map(|(_, role)| crate::source_integrity::related_object(v, &role))
                })
                .collect(),
            protected_by_decisions: snapshot.adopted_by.get(id).cloned().unwrap_or_default(),
        };
        if touched.contains(id) {
            direct.push(item);
        } else {
            affected.push(item);
        }
        if !matches!(
            o.kind.as_str(),
            "SourceSemanticUnit" | "SourceUnitRelation" | "SourceSpan"
        ) && !crate::semantic_plan::is_semantic_kind(&o.kind)
        {
            continue;
        }
        let make = (|| -> Result<MapOperation<MapEditPayload>, DomainError> {
            let stable = StableId::parse(id)?;
            let payload = MapEditPayload(o.payload.clone());
            let h = EntityHeader::new(
                stable.clone(),
                RevisionId::parse(&o.revision)?,
                payload.entity_kind(),
                SchemaVersion::parse(if crate::semantic_plan::is_semantic_kind(&o.kind) {
                    SEMANTIC_SCHEMA_VERSION
                } else {
                    "mathscape-source-1"
                })?,
                if active(o) {
                    LifecycleStatus::Active
                } else {
                    LifecycleStatus::Deprecated
                },
                Timestamp::parse("unix:0")?,
                actor.clone(),
                None,
            );
            if o.kind == "SourceUnitRelation" {
                let relation = RelationRecord::new(h, o.endpoints.clone(), payload)?;
                Ok(if let Some(old) = old {
                    MapOperation::ReviseRelation {
                        stable_id: stable,
                        expected_revision: RevisionId::parse(&old.revision)?,
                        replacement: relation,
                    }
                } else {
                    MapOperation::CreateRelation { relation }
                })
            } else {
                let entity = EntityEnvelope::new(h, payload)?;
                Ok(if let Some(old) = old {
                    MapOperation::ReviseEntity {
                        stable_id: stable,
                        expected_revision: RevisionId::parse(&old.revision)?,
                        replacement: entity,
                    }
                } else {
                    MapOperation::CreateEntity { entity }
                })
            }
        })();
        match make {
            Ok(op) => operations.push((ranks[id], id.clone(), op)),
            Err(e) => diagnostics.push(error(id, "invalid_object", e.to_string())),
        }
        changed_objects.push(o.clone());
    }
    operations.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let operations: Vec<_> = operations.into_iter().map(|(_, _, op)| op).collect();
    if operations.is_empty() {
        diagnostics.push(error("", "no_changes", "没有可提交的变化"));
    }
    let mut head_issues = vec![];
    if tolerate_head {
        let changed: BTreeSet<&str> = changed_objects
            .iter()
            .map(|o| o.object_id.as_str())
            .collect();
        let existing: BTreeSet<(String, String)> =
            final_graph_diagnostics(&original, &BTreeSet::new())
                .into_iter()
                .filter_map(|d| Some((d.code, d.object_id?)))
                .collect();
        let (blocking, pre_existing): (Vec<_>, Vec<_>) = diagnostics.into_iter().partition(|d| {
            let Some(id) = d.object_id.as_deref().filter(|id| !id.is_empty()) else {
                return true;
            };
            touched.contains(id)
                || changed.contains(id)
                || !existing.contains(&(d.code.clone(), id.to_owned()))
        });
        diagnostics = blocking;
        head_issues = pre_existing;
    }
    let status = if conflicted {
        ImpactPlanStatus::Conflicted
    } else if diagnostics.is_empty() {
        ImpactPlanStatus::Ready
    } else {
        ImpactPlanStatus::Blocked
    };
    let fp = if head_issues.is_empty() {
        edit_fingerprint(&(
            "map-edit-plan-1",
            snapshot,
            request,
            actor,
            &direct,
            &affected,
            &diagnostics,
        ))
    } else {
        edit_fingerprint(&(
            "map-edit-plan-1",
            snapshot,
            request,
            actor,
            &direct,
            &affected,
            &diagnostics,
            &head_issues,
        ))
    };
    let patch = if status == ImpactPlanStatus::Ready {
        MapPatch::new(
            request
                .base_commit
                .as_ref()
                .map(|v| RevisionId::parse(v).map(CommitId::from_revision))
                .transpose()
                .unwrap(),
            actor.clone(),
            None,
            Some(request.reason.clone()),
            operations,
        )
        .ok()
    } else {
        None
    };
    PlannedMapChange {
        impact: ImpactPlan {
            version: "mathscape-impact-plan-1".into(),
            request_id: request.request_id.clone(),
            base_commit: snapshot.base_commit.clone(),
            plan_fingerprint: fp,
            status,
            direct_changes: direct,
            affected_objects: affected,
            diagnostics,
            head_issues,
        },
        patch,
        changed_objects,
    }
}

pub fn get_edit_context(snapshot: &MapEditSnapshot, id: &str) -> Result<EditContext, String> {
    let o = snapshot
        .objects
        .iter()
        .find(|o| o.object_id == id)
        .ok_or("对象不存在")?;
    let option = |o: &MapEditObject| EditOption {
        object_id: o.object_id.clone(),
        label: body(o)["display_title"]
            .as_str()
            .unwrap_or(&o.object_id)
            .into(),
    };
    let mut fields = vec![];
    if active(o) && o.kind == "SourceSemanticUnit" {
        for (name, key, ty) in [
            ("display_title", "display_title", "nullable_text"),
            ("parent", "parent_unit", "nullable_unit"),
            ("completeness", "completeness", "unit_completeness"),
        ] {
            fields.push(EditableField {
                name: name.into(),
                field_type: ty.into(),
                value: body(o)[key].clone(),
                constraints: vec![],
            });
        }
    } else if active(o) && o.kind == "SourceUnitRelation" {
        fields.push(EditableField {
            name: "relation".into(),
            field_type: "source_relation_kind".into(),
            value: body(o)["relation"].clone(),
            constraints: vec!["has_part 使用 parent 编辑".into()],
        });
        for role in ["source", "target", "evidence"] {
            fields.push(EditableField {
                name: if role == "evidence" {
                    "evidence_spans"
                } else {
                    role
                }
                .into(),
                field_type: if role == "evidence" {
                    "span_list"
                } else {
                    "unit"
                }
                .into(),
                value: if role == "evidence" {
                    json!(
                        o.endpoints
                            .iter()
                            .filter(|e| e.role().as_str() == role)
                            .map(|e| e.entity().as_str())
                            .collect::<Vec<_>>()
                    )
                } else {
                    json!(endpoint(o, role))
                },
                constraints: vec![],
            });
        }
    }
    let current_context = if o.kind == "SourceUnitRelation" {
        endpoint(o, "source")
            .and_then(|source| {
                snapshot
                    .objects
                    .iter()
                    .find(|candidate| candidate.object_id == source)
            })
            .and_then(source_context)
    } else {
        source_context(o)
    };
    let available_parents = snapshot
        .objects
        .iter()
        .filter(|x| active(x) && x.kind == "SourceSemanticUnit" && x.object_id != id)
        .filter(|x| {
            current_context
                .as_ref()
                .is_some_and(|context| source_context(x).as_ref() == Some(context))
        })
        .map(option)
        .collect();
    let evidence_spans = snapshot
        .objects
        .iter()
        .filter(|x| active(x) && x.kind == "SourceSpan")
        .filter(|x| {
            current_context
                .as_ref()
                .is_some_and(|context| source_context(x).as_ref() == Some(context))
        })
        .map(option)
        .collect();
    Ok(EditContext {
        version: "mathscape-edit-context-1".into(),
        base_commit: snapshot.base_commit.clone(),
        object_id: id.into(),
        current_revision: o.revision.clone(),
        kind: o.kind.clone(),
        lifecycle: o.lifecycle.clone(),
        editable_fields: fields,
        available_parents,
        related_relations: snapshot
            .objects
            .iter()
            .filter(|x| active(x) && x.endpoints.iter().any(|e| e.entity().as_str() == id))
            .map(option)
            .collect(),
        evidence_spans,
        can_retire: active(o)
            && matches!(
                o.kind.as_str(),
                "SourceSemanticUnit"
                    | "SourceUnitRelation"
                    | "SourceSpan"
                    | "SemanticEntity"
                    | "Alignment"
                    | "Debt"
                    | "DialectScope"
            ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str, artifact: &str) -> MapEditObject {
        MapEditObject {
            object_id: id.into(),
            revision: format!("sha256:{}", "1".repeat(64)),
            kind: "SourceSemanticUnit".into(),
            lifecycle: "active".into(),
            payload: json!({
                "payload_kind": "semantic_unit",
                "payload": {
                    "display_title": id,
                    "surface_content_artifact": artifact,
                    "source_spans": ["span"],
                    "parent_unit": null
                }
            }),
            endpoints: vec![],
        }
    }

    #[test]
    fn edit_context_limits_parents_to_the_same_artifact() {
        let snapshot = MapEditSnapshot {
            base_commit: None,
            adopted_by: Default::default(),
            objects: vec![
                unit("unit-a", "mathscape:artifact/a"),
                unit("unit-b", "mathscape:artifact/a"),
                unit("unit-c", "mathscape:artifact/b"),
            ],
        };
        let context = get_edit_context(&snapshot, "unit-a").unwrap();
        let parents = context
            .available_parents
            .iter()
            .map(|option| option.object_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(parents, BTreeSet::from(["unit-b"]));
    }

    fn span_op(id: &str, artifact: &str) -> UserChangeOperation {
        UserChangeOperation::CreateSourceSpan {
            object_id: id.into(),
            draft: SourceSpanDraft {
                granularity: SpanGranularity::Inline,
                layout: LayoutKind::Other,
                page: None,
                bounding_boxes: vec![],
                marker_block_ids: vec![],
                markdown: Some(MarkdownLocator::new(
                    StableId::parse(artifact).unwrap(),
                    ByteRange::new(0, 4).unwrap(),
                    None,
                )),
                exact_text_artifact: None,
                normalized_text_hash: None,
            },
        }
    }

    fn unit_op(id: &str, parent: Option<&str>, spans: &[&str]) -> UserChangeOperation {
        UserChangeOperation::CreateSourceUnit {
            object_id: id.into(),
            draft: SourceUnitDraft {
                kind: DiscoveryUnitKind::Statement,
                display_title: Some(id.into()),
                parent: parent.map(str::to_owned),
                source_spans: spans.iter().map(|s| (*s).to_owned()).collect(),
                completeness: UnitCompleteness::Complete,
            },
        }
    }

    /// The planner applies a request in dependency order, so the written
    /// order (child before parent, relation before its units, spans last)
    /// never changes the result; the patch is written span → unit by parent
    /// depth → relation.
    #[test]
    fn written_order_does_not_change_the_plan() {
        let a = format!("mathscape:artifact/sha256/{}", "a".repeat(64));
        let b = format!("mathscape:artifact/sha256/{}", "b".repeat(64));
        let (sa, sb) = ("mathscape:source-span/a", "mathscape:source-span/b");
        let (parent, child, grandchild) = (
            "mathscape:source-unit/p",
            "mathscape:source-unit/c",
            "mathscape:source-unit/g",
        );
        let relation = UserChangeOperation::CreateSourceRelation {
            object_id: "mathscape:source-relation/r".into(),
            draft: SourceRelationDraft {
                relation: DiscoveryRelationKind::Proves,
                source: child.into(),
                target: parent.into(),
                evidence_spans: vec![sa.into()],
            },
        };
        // The child inherits the source context of a parent created in the
        // same request, which needs the parent applied first.
        let natural = vec![
            span_op(sa, &a),
            span_op(sb, &b),
            unit_op(parent, None, &[sa]),
            unit_op(child, Some(parent), &[sa]),
            unit_op(grandchild, Some(child), &[sa]),
            unit_op("mathscape:source-unit/other", None, &[sb]),
            relation.clone(),
        ];
        let mut reversed = natural.clone();
        reversed.reverse();
        let snapshot = MapEditSnapshot {
            base_commit: None,
            adopted_by: Default::default(),
            objects: vec![],
        };
        let actor = ActorRef::parse("tester").unwrap();
        let plan = |operations: Vec<UserChangeOperation>| {
            plan_map_change(
                &snapshot,
                &UserChangeRequest {
                    request_id: "mathscape:request/order".into(),
                    base_commit: None,
                    reason: "order".into(),
                    operations,
                },
                &actor,
            )
        };
        let forward = plan(natural);
        let backward = plan(reversed);
        assert_eq!(
            forward.impact.status,
            ImpactPlanStatus::Ready,
            "{:?}",
            forward.impact.diagnostics
        );
        assert_eq!(backward.impact.status, ImpactPlanStatus::Ready);
        let objects = |planned: &PlannedMapChange| {
            planned
                .changed_objects
                .iter()
                .map(|o| (o.object_id.clone(), o.payload.clone(), o.endpoints.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(objects(&forward), objects(&backward));
        let written = |planned: &PlannedMapChange| {
            planned
                .patch
                .as_ref()
                .unwrap()
                .operations()
                .iter()
                .map(|op| match op {
                    MapOperation::CreateEntity { entity } => {
                        entity.header().stable_id().to_string()
                    }
                    MapOperation::CreateRelation { relation } => {
                        relation.header().stable_id().to_string()
                    }
                    other => panic!("unexpected {other:?}"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(written(&forward), written(&backward));
        assert_eq!(
            written(&forward),
            vec![
                sa,
                sb,
                "mathscape:source-unit/other",
                parent,
                child,
                grandchild,
                "mathscape:source-relation/r"
            ]
        );
    }

    fn span_object(id: &str, payload: Value) -> MapEditObject {
        MapEditObject {
            object_id: id.into(),
            revision: format!("sha256:{}", "2".repeat(64)),
            kind: "SourceSpan".into(),
            lifecycle: "active".into(),
            payload: json!({ "payload_kind": "span", "payload": payload }),
            endpoints: vec![],
        }
    }

    fn ink_span(id: &str) -> MapEditObject {
        span_object(
            id,
            json!({
                "granularity": "inline", "layout": "other", "page": null,
                "bounding_boxes": [], "marker_block_ids": [], "markdown": null,
                "exact_text_artifact": null, "normalized_text_hash": null,
                "ink": {
                    "snapshot_artifact": format!("mathscape:artifact/sha256/{}", "9".repeat(64)),
                    "page_id": "p0", "stroke_ids": ["s1", "s2"], "selection_mode": "lasso"
                }
            }),
        )
    }

    fn create_request(operations: Vec<UserChangeOperation>) -> UserChangeRequest {
        UserChangeRequest {
            request_id: "mathscape:request/head".into(),
            base_commit: None,
            reason: "head".into(),
            operations,
        }
    }

    /// A handwritten-manuscript span is a physical locator: it neither
    /// blocks a manual map edit nor the staged-proposal planner.
    #[test]
    fn ink_evidence_in_the_head_is_a_valid_locator() {
        let a = format!("mathscape:artifact/sha256/{}", "a".repeat(64));
        let snapshot = MapEditSnapshot {
            base_commit: None,
            adopted_by: Default::default(),
            objects: vec![ink_span("mathscape:ink-evidence/x")],
        };
        assert!(head_integrity_diagnostics(&snapshot).is_empty());
        let request = create_request(vec![
            span_op("mathscape:source-span/a", &a),
            unit_op(
                "mathscape:source-unit/u",
                None,
                &["mathscape:source-span/a"],
            ),
        ]);
        let actor = ActorRef::parse("tester").unwrap();
        let strict = plan_map_change(&snapshot, &request, &actor);
        assert_eq!(
            strict.impact.status,
            ImpactPlanStatus::Ready,
            "{:?}",
            strict.impact.diagnostics
        );
        let staged = plan_map_change_over_head(&snapshot, &request, &actor);
        assert_eq!(staged.impact.status, ImpactPlanStatus::Ready);
        assert!(staged.impact.head_issues.is_empty());
        assert_eq!(
            staged.impact.plan_fingerprint,
            strict.impact.plan_fingerprint
        );
    }

    /// A pre-existing problem of an untouched head object blocks a manual
    /// edit (no implicit repair) but only warns a staged proposal; touching
    /// the broken object makes it blocking again.
    #[test]
    fn pre_existing_head_problems_only_block_requests_that_touch_them() {
        let a = format!("mathscape:artifact/sha256/{}", "a".repeat(64));
        let broken = "mathscape:source-span/broken";
        let snapshot = MapEditSnapshot {
            base_commit: None,
            adopted_by: Default::default(),
            objects: vec![span_object(
                broken,
                json!({ "granularity": "inline", "markdown": null, "page": null }),
            )],
        };
        let head = head_integrity_diagnostics(&snapshot);
        assert_eq!(head.len(), 1);
        assert_eq!(head[0].code, "invalid_locator");
        assert_eq!(head[0].object_id.as_deref(), Some(broken));
        let actor = ActorRef::parse("tester").unwrap();
        let unrelated = create_request(vec![
            span_op("mathscape:source-span/a", &a),
            unit_op(
                "mathscape:source-unit/u",
                None,
                &["mathscape:source-span/a"],
            ),
        ]);
        let strict = plan_map_change(&snapshot, &unrelated, &actor);
        assert_eq!(strict.impact.status, ImpactPlanStatus::Blocked);
        let staged = plan_map_change_over_head(&snapshot, &unrelated, &actor);
        assert_eq!(
            staged.impact.status,
            ImpactPlanStatus::Ready,
            "{:?}",
            staged.impact.diagnostics
        );
        assert!(staged.patch.is_some());
        assert_eq!(staged.impact.head_issues.len(), 1);
        assert_eq!(
            staged.impact.head_issues[0].object_id.as_deref(),
            Some(broken)
        );
        // Retiring the broken span is a request on it, so it is never
        // reported as somebody else's head issue.
        let touching = create_request(vec![UserChangeOperation::RetireObject {
            object_id: broken.into(),
            expected_revision: format!("sha256:{}", "2".repeat(64)),
        }]);
        let planned = plan_map_change_over_head(&snapshot, &touching, &actor);
        assert!(planned.impact.head_issues.is_empty());
    }
}
