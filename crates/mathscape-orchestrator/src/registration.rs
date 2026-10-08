//! Pure registration binding and registration-scoped retirement.
use crate::map_edit::edit_fingerprint;
use crate::source_integrity::*;
use mathscape_domain::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_REGISTRATION_OBJECTS: usize = 50_000;
pub struct RegistrationExecution<T> {
    pub plan: T,
    pub patch: Option<MapPatch<MapEditPayload>>,
}

fn diagnostic(id: Option<String>, code: &str, message: impl Into<String>) -> ImpactDiagnostic {
    ImpactDiagnostic {
        code: code.into(),
        object_id: id,
        message: message.into(),
    }
}
fn ordered(snapshot: &RegistrationSnapshot) -> RegistrationSnapshot {
    let mut s = snapshot.clone();
    s.map.objects.sort_by(|a, b| a.object_id.cmp(&b.object_id));
    for o in &mut s.map.objects {
        o.endpoints.sort_by_key(|e| {
            (
                e.role().as_str().to_owned(),
                e.ordinal(),
                e.entity().to_string(),
            )
        });
    }
    s.registrations
        .sort_by(|a, b| a.summary.decision_id.cmp(&b.summary.decision_id));
    for r in &mut s.registrations {
        r.members.sort_by(|a, b| a.object_id.cmp(&b.object_id));
    }
    s
}
fn fingerprint_snapshot(s: &RegistrationSnapshot) -> serde_json::Value {
    // Audit timestamps, review text and old withdrawal receipts are not execution state.
    json!({"map":s.map,"adoptions":s.registrations.iter().map(|r| json!({"decision":r.summary.decision_id,"commit":r.summary.registration_commit_id,"state":r.summary.state,"members":r.members})).collect::<Vec<_>>()})
}
fn claims(snapshot: &RegistrationSnapshot, id: &str, except: &str) -> Vec<String> {
    snapshot
        .registrations
        .iter()
        .filter(|r| {
            r.summary.state == RegistrationState::Registered
                && r.summary.decision_id != except
                && r.members.iter().any(|m| m.object_id == id)
        })
        .map(|r| r.summary.decision_id.clone())
        .collect()
}
fn item(o: &MapEditObject) -> ImpactItem {
    ImpactItem {
        object_id: o.object_id.clone(),
        kind: o.kind.clone(),
        action: "preserve".into(),
        reason: String::new(),
        current_revision: Some(o.revision.clone()),
        proposed_revision: None,
        display_title: object_title(o),
        reason_codes: vec![],
        related_objects: vec![],
        protected_by_decisions: vec![],
    }
}
fn materialize(
    snapshot: &MapEditSnapshot,
    changed: &[MapEditObject],
    request_id: &str,
    reason: &str,
    actor: &ActorRef,
) -> Result<Option<MapPatch<MapEditPayload>>, DomainError> {
    if changed.is_empty() {
        return Ok(None);
    }
    let mut ops = vec![];
    for o in changed {
        let old = snapshot.objects.iter().find(|v| v.object_id == o.object_id);
        let id = StableId::parse(&o.object_id)?;
        let p = MapEditPayload(o.payload.clone());
        let revision = if old.is_none() {
            o.revision.clone()
        } else {
            edit_fingerprint(&(
                "registration-revision-1",
                &id,
                &o.payload,
                &o.endpoints,
                &o.lifecycle,
                request_id,
                actor,
            ))
        };
        let h = EntityHeader::new(
            id.clone(),
            RevisionId::parse(revision)?,
            p.entity_kind(),
            SchemaVersion::parse("mathscape-source-1")?,
            serde_json::from_value(json!(o.lifecycle)).expect("checked lifecycle"),
            Timestamp::parse("unix:0")?,
            actor.clone(),
            None,
        );
        ops.push(if o.kind == "SourceUnitRelation" {
            let r = RelationRecord::new(h, o.endpoints.clone(), p)?;
            if let Some(old) = old {
                MapOperation::ReviseRelation {
                    stable_id: id,
                    expected_revision: RevisionId::parse(&old.revision)?,
                    replacement: r,
                }
            } else {
                MapOperation::CreateRelation { relation: r }
            }
        } else {
            let e = EntityEnvelope::new(h, p)?;
            if let Some(old) = old {
                MapOperation::ReviseEntity {
                    stable_id: id,
                    expected_revision: RevisionId::parse(&old.revision)?,
                    replacement: e,
                }
            } else {
                MapOperation::CreateEntity { entity: e }
            }
        });
    }
    MapPatch::new(
        snapshot
            .base_commit
            .as_ref()
            .map(|v| RevisionId::parse(v).map(CommitId::from_revision))
            .transpose()?,
        actor.clone(),
        None,
        Some(reason.into()),
        ops,
    )
    .map(Some)
}
fn set_revisions(items: &mut [ImpactItem], patch: &Option<MapPatch<MapEditPayload>>) {
    for op in patch.iter().flat_map(|p| p.operations()) {
        let h = match op {
            MapOperation::CreateEntity { entity } => entity.header(),
            MapOperation::CreateRelation { relation } => relation.header(),
            MapOperation::ReviseEntity { replacement, .. } => replacement.header(),
            MapOperation::ReviseRelation { replacement, .. } => replacement.header(),
        };
        if let Some(i) = items
            .iter_mut()
            .find(|i| i.object_id == h.stable_id().as_str())
        {
            i.proposed_revision = Some(h.revision_id().to_string());
        }
    }
}
fn impact(
    request_id: &str,
    head: &Option<String>,
    fp: String,
    conflicts: bool,
    items: Vec<ImpactItem>,
    diagnostics: Vec<ImpactDiagnostic>,
) -> ImpactPlan {
    ImpactPlan {
        version: "mathscape-impact-plan-1".into(),
        request_id: request_id.into(),
        base_commit: head.clone(),
        plan_fingerprint: fp,
        status: if conflicts {
            ImpactPlanStatus::Conflicted
        } else if diagnostics.is_empty() {
            ImpactPlanStatus::Ready
        } else {
            ImpactPlanStatus::Blocked
        },
        direct_changes: items,
        affected_objects: vec![],
        diagnostics,
    }
}

pub fn plan_registration_reversal(
    snapshot: &RegistrationSnapshot,
    request: &ReverseDiscoveryRequest,
    actor: &ActorRef,
) -> RegistrationExecution<DiscoveryReversalPlan> {
    let s = ordered(snapshot);
    let mut errors = vec![];
    let conflicts = request.base_commit != s.map.base_commit;
    if conflicts {
        errors.push(diagnostic(
            None,
            "base_commit_conflict",
            "地图已变化，请重新计算影响",
        ));
    }
    if StableId::parse(&request.request_id).is_err() || request.reason.trim().is_empty() {
        errors.push(diagnostic(
            None,
            "invalid_request",
            "需要有效请求 ID 和撤回原因",
        ));
    }
    let map: BTreeMap<_, _> = s
        .map
        .objects
        .iter()
        .cloned()
        .map(|o| (o.object_id.clone(), o))
        .collect();
    // Withdrawal is not an implicit repair of a previously invalid live graph.
    errors.extend(validate_live_references(&map));
    let registration = s.registrations.iter().find(|r| {
        r.summary.decision_id == request.decision_id
            && r.summary.registration_commit_id == request.registration_commit_id
    });
    let mut items = BTreeMap::new();
    let mut retire = BTreeSet::new();
    if let Some(r) = registration {
        if r.summary.state == RegistrationState::Withdrawn {
            errors.push(diagnostic(
                None,
                "already_withdrawn",
                "此登记已经撤回，请查看原回执",
            ));
        }
        if r.members.len() > MAX_REGISTRATION_OBJECTS {
            errors.push(diagnostic(
                None,
                "registration_limit",
                "登记对象超过撤回上限",
            ));
        }
        for m in &r.members {
            let Some(o) = map.get(&m.object_id) else {
                errors.push(diagnostic(
                    Some(m.object_id.clone()),
                    "missing_member",
                    "登记对象不存在",
                ));
                continue;
            };
            let mut i = item(o);
            if let Ok(refs) = source_references(o) {
                for (target, role) in refs {
                    if let Some(target) = map.get(&target) {
                        i.related_objects.push(related_object(target, &role));
                    }
                }
            }
            i.protected_by_decisions = claims(&s, &o.object_id, &request.decision_id);
            if !is_live(o) {
                i.reason_codes.push("already_inactive".into());
            } else {
                if !matches!(m.outcome.as_str(), "created" | "reactivated") {
                    i.reason_codes.push(
                        if m.outcome == "reused" {
                            "reused_object"
                        } else {
                            "preexisting_revised_object"
                        }
                        .into(),
                    );
                }
                if o.revision != m.revision_id {
                    i.reason_codes.push("subsequently_revised".into());
                }
                if !i.protected_by_decisions.is_empty() {
                    i.reason_codes.push("adopted_by_other_registration".into());
                }
                if i.reason_codes.is_empty() {
                    retire.insert(o.object_id.clone());
                    i.reason_codes.push(
                        if m.outcome == "created" {
                            "created_by_registration"
                        } else {
                            "reactivated_by_registration"
                        }
                        .into(),
                    );
                }
            }
            if items.insert(o.object_id.clone(), i).is_some() {
                errors.push(diagnostic(
                    Some(o.object_id.clone()),
                    "duplicate_member",
                    "登记成员重复",
                ));
            }
        }
    } else {
        errors.push(diagnostic(
            None,
            "registration_not_found",
            "找不到匹配的实际登记回执，禁止猜测范围",
        ));
    }
    // Fixed point: preserved/outside live objects protect their entire dependency closure.
    let candidates = retire.clone();
    let mut queue = map
        .values()
        .filter(|o| is_live(o) && !retire.contains(&o.object_id))
        .map(|o| o.object_id.clone())
        .collect::<std::collections::VecDeque<_>>();
    while let Some(id) = queue.pop_front() {
        let o = &map[&id];
        match source_references(o) {
            Err(message) => {
                errors.push(diagnostic(
                    Some(o.object_id.clone()),
                    "unsupported_reference_kind",
                    message,
                ));
            }
            Ok(refs) => {
                for (target, role) in refs {
                    if candidates.contains(&target) {
                        if retire.remove(&target) {
                            queue.push_back(target.clone());
                        }
                        let i = items.get_mut(&target).expect("retirement member");
                        let code = if items_contains_registration(registration, &o.object_id) {
                            "required_by_preserved_object"
                        } else {
                            "referenced_by_live_object"
                        };
                        if !i.reason_codes.iter().any(|v| v == code) {
                            i.reason_codes.push(code.into());
                        }
                        let related = related_object(o, &role);
                        if !i
                            .related_objects
                            .iter()
                            .any(|v| v.object_id == related.object_id && v.reference_kind == role)
                        {
                            i.related_objects.push(related);
                        }
                    }
                }
            }
        }
    }
    let mut final_map = map.clone();
    let mut changed = vec![];
    for id in &retire {
        let o = final_map.get_mut(id).unwrap();
        o.lifecycle = "deprecated".into();
        changed.push(o.clone());
    }
    errors.extend(validate_live_references(&final_map));
    let mut values: Vec<_> = items.into_values().collect();
    let mut counts = ReversalCounts {
        total: values.len() as u32,
        ..Default::default()
    };
    for i in &mut values {
        if retire.contains(&i.object_id) {
            i.action = "retire".into();
            i.reason = "本次登记创建或重新激活，且没有外部活动依赖".into();
            counts.retire += 1;
        } else if i.reason_codes.iter().any(|c| c == "already_inactive") {
            i.reason = "已经退役，不生成重复修订".into();
            counts.already_inactive += 1;
        } else {
            i.reason = "保留共享、后续修改或被活动对象依赖的对象".into();
            counts.preserve += 1;
        }
    }
    let patch = if errors.is_empty() {
        materialize(
            &s.map,
            &changed,
            &request.request_id,
            &request.reason,
            actor,
        )
        .map_err(|e| errors.push(diagnostic(None, "materialization", e.to_string())))
        .ok()
        .flatten()
    } else {
        None
    };
    set_revisions(&mut values, &patch);
    let fp = edit_fingerprint(&(
        "reversal-plan-1",
        fingerprint_snapshot(&s),
        request,
        actor,
        &values,
        &errors,
    ));
    RegistrationExecution {
        plan: DiscoveryReversalPlan {
            version: "mathscape-discovery-reversal-plan-1".into(),
            request: request.clone(),
            impact: impact(
                &request.request_id,
                &s.map.base_commit,
                fp,
                conflicts,
                values,
                errors,
            ),
            summary: counts,
        },
        patch,
    }
}
fn items_contains_registration(r: Option<&RegistrationAdoption>, id: &str) -> bool {
    r.is_some_and(|r| r.members.iter().any(|m| m.object_id == id))
}

pub fn plan_registration_commit(
    snapshot: &RegistrationSnapshot,
    request: &SourceRegistrationCommitRequest,
    candidate: &MapPatch<SourcePayload>,
    actor: &ActorRef,
) -> RegistrationExecution<SourceRegistrationCommitPlan> {
    let s = ordered(snapshot);
    let conflicts = request.base_commit != s.map.base_commit;
    let mut errors = vec![];
    if conflicts {
        errors.push(diagnostic(
            None,
            "base_commit_conflict",
            "地图已变化，请重新预览登记",
        ));
    }
    if StableId::parse(&request.request_id).is_err()
        || StableId::parse(&request.decision_id).is_err()
        || ContentHash::parse(&request.acceptance_plan_fingerprint).is_err()
    {
        errors.push(diagnostic(None, "invalid_request", "登记请求身份无效"));
    }
    let mut approvals = BTreeMap::new();
    for a in &request.replacement_approvals {
        if approvals
            .insert(a.object_id.clone(), a.expected_revision.clone())
            .is_some()
        {
            errors.push(diagnostic(
                Some(a.object_id.clone()),
                "duplicate_approval",
                "重复的替换批准",
            ));
        }
    }
    let mut map: BTreeMap<_, _> = s
        .map
        .objects
        .iter()
        .cloned()
        .map(|o| (o.object_id.clone(), o))
        .collect();
    let mut changed = vec![];
    let mut objects = vec![];
    let mut items = vec![];
    if candidate.operations().len() > MAX_REGISTRATION_OBJECTS {
        errors.push(diagnostic(None, "registration_limit", "登记对象超限"));
    }
    for op in candidate.operations() {
        let (h, p, endpoints) = match op {
            MapOperation::CreateEntity { entity } => (entity.header(), entity.payload(), vec![]),
            MapOperation::CreateRelation { relation } => (
                relation.header(),
                relation.payload(),
                relation.endpoints().to_vec(),
            ),
            _ => {
                errors.push(diagnostic(
                    None,
                    "invalid_candidate",
                    "接受结果必须是原始来源创建提案",
                ));
                continue;
            }
        };
        let id = h.stable_id().to_string();
        let mut desired = MapEditObject {
            object_id: id.clone(),
            revision: h.revision_id().to_string(),
            kind: serde_json::to_value(h.kind())
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            lifecycle: serde_json::to_value(h.lifecycle())
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            payload: serde_json::to_value(p).unwrap(),
            endpoints,
        };
        desired.endpoints.sort_by_key(|e| {
            (
                e.role().as_str().to_owned(),
                e.ordinal(),
                e.entity().to_string(),
            )
        });
        let old = map.get(&id).cloned();
        let mut action = RegistrationObjectAction::Create;
        let mut fields = vec![];
        let mut i = item(&desired);
        i.current_revision = old.as_ref().map(|o| o.revision.clone());
        i.protected_by_decisions = claims(&s, &id, &request.decision_id);
        if let Some(o) = &old {
            let same = o.payload == desired.payload && o.endpoints == desired.endpoints;
            let identity_ok = o.kind == desired.kind
                && if o.kind == "SourceSpan" {
                    // Span locators and exact content identity cannot be overwritten by approval.
                    o.payload["payload"] == desired.payload["payload"]
                } else {
                    o.payload["payload"]["surface_content_artifact"]
                        == desired.payload["payload"]["surface_content_artifact"]
                };
            let approval = approvals.remove(&id);
            if approval.as_ref().is_some_and(|rev| rev != &o.revision) {
                errors.push(diagnostic(
                    Some(id.clone()),
                    "revision_conflict",
                    "替换批准的 revision 已失效",
                ));
                action = RegistrationObjectAction::Block;
            } else if !identity_ok {
                errors.push(diagnostic(
                    Some(id.clone()),
                    "identity_conflict",
                    "来源身份、定位或实体类型冲突，不可覆盖",
                ));
                action = RegistrationObjectAction::Block;
            } else if same {
                action = if is_live(o) {
                    RegistrationObjectAction::Reuse
                } else {
                    RegistrationObjectAction::Reactivate
                };
            } else if !i.protected_by_decisions.is_empty() {
                errors.push(diagnostic(
                    Some(id.clone()),
                    "shared_replacement",
                    "其他有效登记仍采纳此对象，请用独立人工编辑处理",
                ));
                action = RegistrationObjectAction::Block;
            } else if approval.is_none() {
                errors.push(diagnostic(
                    Some(id.clone()),
                    "replacement_approval_required",
                    "内容不同，必须逐对象审查并批准替换",
                ));
                action = RegistrationObjectAction::Block;
            } else {
                action = if is_live(o) {
                    RegistrationObjectAction::Revise
                } else {
                    RegistrationObjectAction::Reactivate
                };
            }
            if o.payload != desired.payload {
                if let (Some(before), Some(after)) = (
                    o.payload["payload"].as_object(),
                    desired.payload["payload"].as_object(),
                ) {
                    fields.extend(
                        before
                            .keys()
                            .chain(after.keys())
                            .filter(|k| before.get(*k) != after.get(*k))
                            .cloned()
                            .collect::<BTreeSet<_>>(),
                    );
                } else {
                    fields.push("payload".into());
                }
            }
            if o.endpoints != desired.endpoints {
                fields.push("endpoints".into());
            }
        }
        if action == RegistrationObjectAction::Reuse {
            desired = old.clone().unwrap();
        }
        if matches!(
            action,
            RegistrationObjectAction::Create
                | RegistrationObjectAction::Reactivate
                | RegistrationObjectAction::Revise
        ) {
            changed.push(desired.clone());
            map.insert(id.clone(), desired.clone());
        }
        i.action = serde_json::to_value(action)
            .unwrap()
            .as_str()
            .unwrap()
            .into();
        i.reason = if action == RegistrationObjectAction::Reuse {
            "复用当前 revision，不重复写入"
        } else {
            "按已接受的来源提案登记"
        }
        .into();
        objects.push(RegistrationObjectChange { object_id:id, action, display_title:object_title(&desired), current_revision:old.as_ref().map(|o|o.revision.clone()), proposed_revision:None, changed_fields:fields, before:old.map(|o|json!({"payload":o.payload,"endpoints":o.endpoints,"lifecycle":o.lifecycle})), proposed:json!({"payload":desired.payload,"endpoints":desired.endpoints,"lifecycle":desired.lifecycle}) });
        items.push(i);
    }
    for id in approvals.keys() {
        errors.push(diagnostic(
            Some(id.clone()),
            "unknown_approval",
            "批准对象不属于此接受提案",
        ));
    }
    errors.extend(validate_live_references(&map));
    let patch = if errors.is_empty() {
        materialize(
            &s.map,
            &changed,
            &request.request_id,
            "登记已审核来源结果",
            actor,
        )
        .map_err(|e| errors.push(diagnostic(None, "materialization", e.to_string())))
        .ok()
        .flatten()
    } else {
        None
    };
    set_revisions(&mut items, &patch);
    for (o, i) in objects.iter_mut().zip(&items) {
        o.proposed_revision = i
            .proposed_revision
            .clone()
            .or_else(|| i.current_revision.clone());
    }
    let requires_confirmation = objects.iter().any(|o| {
        matches!(
            o.action,
            RegistrationObjectAction::Reactivate
                | RegistrationObjectAction::Revise
                | RegistrationObjectAction::Block
        )
    });
    let mut canonical_request = request.clone();
    canonical_request
        .replacement_approvals
        .sort_by(|a, b| a.object_id.cmp(&b.object_id));
    let fp = edit_fingerprint(&(
        "registration-commit-plan-1",
        fingerprint_snapshot(&s),
        canonical_request,
        actor,
        &objects,
        &errors,
    ));
    RegistrationExecution {
        plan: SourceRegistrationCommitPlan {
            version: "mathscape-source-registration-commit-plan-1".into(),
            request: request.clone(),
            impact: impact(
                &request.request_id,
                &s.map.base_commit,
                fp,
                conflicts,
                items,
                errors,
            ),
            requires_confirmation,
            objects,
        },
        patch,
    }
}
