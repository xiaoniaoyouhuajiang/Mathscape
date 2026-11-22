//! Semantic-layer planning (docs/semantic-write-contract.md S4–S7).
//!
//! Pure functions shared by `propose_changes` (staging), the Desktop
//! preview / accept pipeline and tests. Only deterministic checks block;
//! mathematical judgement becomes a map debt or a non-blocking review item.
use mathscape_domain::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Blocking diagnostic with a contract failure code (S5.1).
fn fail(id: &str, code: &str, message: impl Into<String>) -> ImpactDiagnostic {
    ImpactDiagnostic {
        code: code.into(),
        object_id: Some(id.into()),
        message: message.into(),
    }
}

pub const SEMANTIC_KINDS: [&str; 4] = ["SemanticEntity", "Alignment", "Debt", "DialectScope"];

pub fn is_semantic_kind(kind: &str) -> bool {
    SEMANTIC_KINDS.contains(&kind)
}

fn active(o: &MapEditObject) -> bool {
    crate::source_integrity::is_live(o)
}

fn semantic<T: serde::de::DeserializeOwned>(o: &MapEditObject) -> Result<T, String> {
    serde_json::from_value(o.payload["payload"].clone()).map_err(|e| e.to_string())
}

fn store(out: &mut MapEditObject, kind: &str, payload: SemanticPayload) {
    out.kind = kind.into();
    out.payload = serde_json::to_value(payload).expect("semantic payload serializes");
}

fn baseline_is_agent_writable(baseline: &Option<MathlibBaseline>) -> bool {
    baseline
        .as_ref()
        .is_none_or(|b| b.status == BaselineStatus::Unchecked && b.checked.is_none())
}

/// Applies one semantic operation to `out` (the current object, or an empty
/// shell for a create). Returns `None` for non-semantic operations.
pub fn apply(
    op: &UserChangeOperation,
    out: &mut MapEditObject,
    actor: &ActorRef,
) -> Option<Result<(), ImpactDiagnostic>> {
    if matches!(
        op,
        UserChangeOperation::CreateSourceUnit { .. }
            | UserChangeOperation::CreateSourceRelation { .. }
            | UserChangeOperation::CreateSourceSpan { .. }
            | UserChangeOperation::EditSourceUnit { .. }
            | UserChangeOperation::EditSourceRelation { .. }
            | UserChangeOperation::RetireObject { .. }
    ) {
        return None;
    }
    let id = op.object_id().to_owned();
    let expect_kind = |out: &MapEditObject, kind: &str| -> Result<(), ImpactDiagnostic> {
        if out.kind == kind {
            Ok(())
        } else {
            Err(fail(
                &id,
                "invalid_reference_kind",
                format!("目标不是 {kind}"),
            ))
        }
    };
    let result = (|| -> Result<(), ImpactDiagnostic> {
        if op.is_server_only() && actor.as_str() != MATHLIB_BASELINE_ACTOR {
            return Err(fail(
                &id,
                "server_only_operation",
                "mathlib 核对结果只能由服务端写入",
            ));
        }
        if op.is_user_only() && actor.as_str().starts_with("agent:") {
            return Err(fail(
                &id,
                "user_only_operation",
                "审阅结论与结账只能由用户在审阅页写入",
            ));
        }
        match op {
            UserChangeOperation::CreateSemanticEntity { draft, .. } => {
                if !baseline_is_agent_writable(&draft.mathlib_baseline) {
                    return Err(fail(
                        &id,
                        "forbidden_baseline_status",
                        "mathlib 基线只能写 unchecked；confirmed / not_found 只由服务端核对后写入",
                    ));
                }
                store(
                    out,
                    "SemanticEntity",
                    SemanticPayload::SemanticEntity(SemanticEntity {
                        draft: draft.clone(),
                        trust: semantic_trust(AlignmentStatus::Unmatched, actor),
                    }),
                );
            }
            UserChangeOperation::EditSemanticEntity { changes, .. } => {
                expect_kind(out, "SemanticEntity")?;
                let mut entity: SemanticEntity =
                    semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                if let Some(labels) = &changes.preferred_labels {
                    entity.draft.preferred_labels = labels.clone();
                }
                if let Some(aliases) = &changes.aliases {
                    entity.draft.aliases = aliases.clone();
                }
                if let Some(home_theory) = &changes.home_theory {
                    if entity.draft.semantic_kind == SemanticKind::Theory {
                        return Err(fail(
                            &id,
                            "invalid_arg",
                            "理论本身不能挂在另一个 home_theory 下",
                        ));
                    }
                    entity.draft.home_theory = Some(home_theory.clone());
                }
                changes.plain_label.apply_to(&mut entity.draft.plain_label);
                changes.description.apply_to(&mut entity.draft.description);
                if let Some(conventions) = &changes.conventions {
                    let old = &entity.draft.conventions;
                    let keeps_values = conventions.len() >= old.len()
                        && old.iter().zip(conventions).all(|(a, b)| {
                            a.key == b.key && a.subject == b.subject && a.value == b.value
                        });
                    if !keeps_values {
                        return Err(fail(
                            &id,
                            "identity_field_edit",
                            "约定注记只能补充说明或追加，不能改动已有约定的 key / subject / value；要改约定请新建共享对象",
                        ));
                    }
                    entity.draft.conventions = conventions.clone();
                }
                if !changes.mathlib_baseline.is_keep() {
                    let mut baseline = entity.draft.mathlib_baseline.clone();
                    changes.mathlib_baseline.apply_to(&mut baseline);
                    if !baseline_is_agent_writable(&baseline) {
                        return Err(fail(
                            &id,
                            "forbidden_baseline_status",
                            "mathlib 基线只能写 unchecked；confirmed / not_found 只由服务端核对后写入",
                        ));
                    }
                    entity.draft.mathlib_baseline = baseline;
                }
                if let Some(uses) = &changes.uses_entities {
                    if let Some(d) = entity.draft.declaration.as_mut() {
                        d.uses_entities = uses.clone();
                    } else if let Some(p) = entity.draft.problem_schema.as_mut() {
                        p.uses_entities = uses.clone();
                    } else {
                        return Err(fail(
                            &id,
                            "invalid_arg",
                            "只有 declaration / problem_schema 有 uses_entities",
                        ));
                    }
                }
                store(
                    out,
                    "SemanticEntity",
                    SemanticPayload::SemanticEntity(entity),
                );
            }
            UserChangeOperation::CreateAlignment { draft, .. } => {
                if !draft.status.agent_writable() {
                    return Err(fail(
                        &id,
                        "forbidden_alignment_status",
                        "对齐状态不能直接写成已审核 / 已确认 / 形式相关 / 拒绝 / 取代；这些只由用户或系统设置",
                    ));
                }
                store(
                    out,
                    "Alignment",
                    SemanticPayload::Alignment(Alignment {
                        trust: semantic_trust(draft.status.trust_axis(), actor),
                        draft: draft.clone(),
                        reviewed: None,
                    }),
                );
            }
            UserChangeOperation::EditAlignment { changes, .. } => {
                expect_kind(out, "Alignment")?;
                let mut a: Alignment = semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                let before = a.draft.clone();
                let d = &mut a.draft;
                if let Some(v) = changes.relation {
                    d.relation = v;
                }
                if let Some(v) = &changes.source_context {
                    d.source_context = v.clone();
                }
                changes.target_context.apply_to(&mut d.target_context);
                if let Some(v) = &changes.parameter_map {
                    d.parameter_map = v.clone();
                }
                if let Some(v) = &changes.term_index_map {
                    d.term_index_map = v.clone();
                }
                if let Some(v) = &changes.required_assumptions {
                    d.required_assumptions = v.clone();
                }
                if let Some(v) = &changes.lost_information {
                    d.lost_information = v.clone();
                }
                if let Some(v) = &changes.evidence_spans {
                    d.evidence_spans = v.clone();
                }
                changes.map_label.apply_to(&mut d.map_label);
                changes.plain_note.apply_to(&mut d.plain_note);
                match changes.status {
                    Some(status) if !status.agent_writable() => {
                        return Err(fail(
                            &id,
                            "forbidden_alignment_status",
                            "edit 只能写 Agent 可写的对齐状态",
                        ));
                    }
                    Some(status) => d.status = status,
                    None => {
                        // A user's review covers the content it saw. When the
                        // content changes, the review no longer applies.
                        let content_changed = {
                            let mut a = before.clone();
                            let mut b = d.clone();
                            a.map_label = None;
                            b.map_label = None;
                            a.plain_note = None;
                            b.plain_note = None;
                            a != b
                        };
                        if content_changed && !d.status.agent_writable() {
                            d.status = AlignmentReviewStatus::RetrievedCandidate;
                        }
                    }
                }
                if !d.status.user_reviewable() || d.status.agent_writable() {
                    a.reviewed = None;
                }
                a.trust = semantic_trust(a.draft.status.trust_axis(), actor);
                store(out, "Alignment", SemanticPayload::Alignment(a));
            }
            UserChangeOperation::CreateDebt { draft, .. } => {
                if !draft.status.agent_writable() {
                    return Err(fail(
                        &id,
                        "forbidden_debt_status",
                        "新建的账只能是 open 或 claimed_paid；paid / waived 只能由用户在审核页设置",
                    ));
                }
                store(
                    out,
                    "Debt",
                    SemanticPayload::Debt(DebtRecord {
                        draft: draft.clone(),
                        settled: None,
                    }),
                );
            }
            UserChangeOperation::EditDebt { changes, .. } => {
                expect_kind(out, "Debt")?;
                let mut debt: DebtRecord =
                    semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                if let Some(status) = changes.status {
                    if !status.agent_writable() || !debt.draft.status.agent_writable() {
                        return Err(fail(
                            &id,
                            "forbidden_debt_status",
                            "edit 只能在 open 与 claimed_paid 之间切换；paid / waived 只能由用户设置",
                        ));
                    }
                    debt.draft.status = status;
                }
                if let Some(v) = &changes.paid_by {
                    debt.draft.paid_by = v.clone();
                }
                if let Some(v) = &changes.obligation {
                    debt.draft.obligation = v.clone();
                }
                changes.failure_note.apply_to(&mut debt.draft.failure_note);
                changes.payoff_hint.apply_to(&mut debt.draft.payoff_hint);
                store(out, "Debt", SemanticPayload::Debt(debt));
            }
            UserChangeOperation::CreateDialectScope { draft, .. } => {
                if draft.parent_scope.is_some() {
                    return Err(fail(
                        &id,
                        "invalid_arg",
                        "第一期约定卡没有继承树，parent_scope 必须为 null",
                    ));
                }
                store(
                    out,
                    "DialectScope",
                    SemanticPayload::DialectScope(DialectScope {
                        draft: draft.clone(),
                    }),
                );
            }
            UserChangeOperation::EditDialectScope { changes, .. } => {
                expect_kind(out, "DialectScope")?;
                let mut scope: DialectScope =
                    semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                if let Some(v) = &changes.label {
                    scope.draft.label = v.clone();
                }
                if let Some(v) = &changes.conventions {
                    scope.draft.conventions = v.clone();
                }
                store(out, "DialectScope", SemanticPayload::DialectScope(scope));
            }
            UserChangeOperation::SetAlignmentReview { status, note, .. } => {
                expect_kind(out, "Alignment")?;
                if !status.user_reviewable() {
                    return Err(fail(
                        &id,
                        "forbidden_alignment_status",
                        "formally_related 本期不开放；rejected / superseded 由系统设置",
                    ));
                }
                let mut a: Alignment = semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                a.draft.status = *status;
                a.reviewed = Some(ReviewMark {
                    by: actor.to_string(),
                    note: note.clone(),
                });
                let mut trust = semantic_trust(status.trust_axis(), actor);
                // Keep the original proposer visible next to the reviewer.
                if let Ok(old) =
                    serde_json::from_value::<TrustRecord>(out.payload["payload"]["trust"].clone())
                {
                    let mut reviewers = old.reviewers().to_vec();
                    if !reviewers.contains(actor) {
                        reviewers.push(actor.clone());
                    }
                    trust = TrustRecord::new(
                        old.provenance(),
                        old.parse(),
                        old.segmentation(),
                        old.typing(),
                        old.proof(),
                        status.trust_axis(),
                        old.curation(),
                        old.confidence(),
                        old.evidence().to_vec(),
                        reviewers,
                    );
                }
                a.trust = trust;
                store(out, "Alignment", SemanticPayload::Alignment(a));
            }
            UserChangeOperation::SetDebtSettlement {
                settlement, note, ..
            } => {
                expect_kind(out, "Debt")?;
                let mut debt: DebtRecord =
                    semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                let from = debt.draft.status;
                let to = match settlement {
                    DebtSettlement::Paid if from == DebtStatus::ClaimedPaid => DebtStatus::Paid,
                    DebtSettlement::Waived
                        if from.is_outstanding()
                            && note.as_deref().is_some_and(|n| !n.trim().is_empty()) =>
                    {
                        DebtStatus::Waived
                    }
                    DebtSettlement::Open if from != DebtStatus::Obsolete => DebtStatus::Open,
                    _ => {
                        return Err(fail(
                            &id,
                            "forbidden_debt_status",
                            "只能把「待确认已还」确认为已还清；豁免需要理由；已失效的账不能重开",
                        ));
                    }
                };
                debt.draft.status = to;
                debt.settled = Some(ReviewMark {
                    by: actor.to_string(),
                    note: note.clone(),
                });
                store(out, "Debt", SemanticPayload::Debt(debt));
            }
            UserChangeOperation::RecordMathlibBaseline { outcome, .. } => {
                expect_kind(out, "SemanticEntity")?;
                if outcome.status == BaselineStatus::Unchecked {
                    return Err(fail(
                        &id,
                        "invalid_arg",
                        "核对结果只能是 confirmed 或 not_found",
                    ));
                }
                let mut entity: SemanticEntity =
                    semantic(out).map_err(|e| fail(&id, "invalid_object", e))?;
                let Some(baseline) = entity
                    .draft
                    .mathlib_baseline
                    .as_mut()
                    .filter(|b| b.decl.is_some())
                else {
                    return Err(fail(
                        &id,
                        "missing_required_field",
                        "该共享对象没有写 mathlib 名称",
                    ));
                };
                baseline.status = outcome.status;
                baseline.checked = Some(outcome.check.clone());
                if outcome.status == BaselineStatus::Confirmed && outcome.check.module.is_some() {
                    baseline.module = outcome.check.module.clone();
                }
                store(
                    out,
                    "SemanticEntity",
                    SemanticPayload::SemanticEntity(entity),
                );
            }
            _ => unreachable!("only semantic operations reach apply"),
        }
        Ok(())
    })();
    Some(result)
}

/// Write order inside one patch (contract S7): scopes, then shared objects
/// (theories first, then by reference depth), alignments, debts.
pub fn write_rank(final_map: &BTreeMap<String, MapEditObject>, o: &MapEditObject) -> (u8, usize) {
    match o.kind.as_str() {
        "DialectScope" => (3, 0),
        "SemanticEntity" => {
            let mut depth = 0;
            let mut seen = BTreeSet::from([o.object_id.clone()]);
            let mut frontier = entity_dependencies(o);
            while !frontier.is_empty() && depth < 64 {
                depth += 1;
                frontier = frontier
                    .into_iter()
                    .filter(|id| seen.insert(id.clone()))
                    .filter_map(|id| final_map.get(&id))
                    .flat_map(entity_dependencies)
                    .collect();
            }
            (4, depth)
        }
        "Alignment" => (5, 0),
        "Debt" => (6, 0),
        _ => (7, 0),
    }
}

fn entity_dependencies(o: &MapEditObject) -> Vec<String> {
    let p = &o.payload["payload"];
    let mut out: Vec<String> = p["home_theory"]
        .as_str()
        .map(str::to_owned)
        .into_iter()
        .collect();
    for key in ["declaration", "problem_schema"] {
        out.extend(strings(&p[key]["uses_entities"]));
    }
    out.extend(p["declaration"]["concept"].as_str().map(str::to_owned));
    out
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Typed references of a semantic object: `(target id, role)`. The role
/// decides the expected kind (see [`expected_kinds`]).
pub fn references(o: &MapEditObject) -> Vec<(String, String)> {
    let p = &o.payload["payload"];
    let mut out = Vec::new();
    let mut add = |v: &Value, role: &str| {
        if let Some(id) = v.as_str() {
            out.push((id.to_owned(), role.to_owned()));
        }
    };
    let text_spans = |v: &Value, out: &mut Vec<(String, String)>| {
        for id in strings(&v["evidence_spans"]) {
            out.push((id, "wording_span".into()));
        }
    };
    match o.kind.as_str() {
        "SemanticEntity" => {
            add(&p["home_theory"], "home_theory");
            for key in ["declaration", "problem_schema"] {
                for v in p[key]["uses_entities"].as_array().into_iter().flatten() {
                    add(v, "uses_entity");
                }
            }
            add(&p["declaration"]["concept"], "concept");
            let mut texts = vec![];
            for key in ["statement", "definiens"] {
                texts.push(p["declaration"][key].clone());
            }
            texts.push(p["problem_schema"]["task"].clone());
            for key in ["declaration", "problem_schema"] {
                texts.extend(
                    p[key]["assumptions"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
            texts.extend(p["conventions"].as_array().cloned().unwrap_or_default());
            for t in &texts {
                text_spans(t, &mut out);
            }
        }
        "Alignment" => {
            add(&p["source"], "alignment_source");
            add(&p["target"], "alignment_target");
            add(&p["target_context"], "target_context");
            add(&p["source_context"]["dialect_scope"], "dialect_scope");
            for v in p["required_assumptions"].as_array().into_iter().flatten() {
                add(&v["debt"], "required_debt");
            }
            for v in p["lost_information"].as_array().into_iter().flatten() {
                add(&v["see_also"], "see_also");
            }
            for v in p["evidence_spans"].as_array().into_iter().flatten() {
                add(v, "evidence_span");
            }
        }
        "Debt" => {
            add(&p["attached_to"], "attached_to");
            for v in p["evidence_spans"].as_array().into_iter().flatten() {
                add(v, "evidence_span");
            }
            for v in p["paid_by"].as_array().into_iter().flatten() {
                add(&v["ref"], "paid_by");
            }
        }
        "DialectScope" => {
            add(&p["anchor_unit"], "anchor_unit");
            add(&p["introduced_by_unit"], "introduced_by_unit");
            add(&p["parent_scope"], "parent_scope");
            for c in p["conventions"].as_array().into_iter().flatten() {
                for v in c["evidence_spans"].as_array().into_iter().flatten() {
                    add(v, "evidence_span");
                }
            }
        }
        _ => {}
    }
    out
}

/// Expected target kinds of a semantic reference role; empty = any live
/// object.
pub fn expected_kinds(role: &str) -> &'static [&'static str] {
    match role {
        "home_theory" | "uses_entity" | "concept" | "alignment_target" | "target_context" => {
            &["SemanticEntity"]
        }
        "alignment_source" | "anchor_unit" | "introduced_by_unit" => &["SourceSemanticUnit"],
        "wording_span" | "evidence_span" => &["SourceSpan"],
        "dialect_scope" | "parent_scope" => &["DialectScope"],
        "required_debt" => &["Debt"],
        "attached_to" => &["Alignment", "SemanticEntity"],
        _ => &[],
    }
}

fn semantic_kind_of(o: &MapEditObject) -> Option<&str> {
    (o.kind == "SemanticEntity")
        .then(|| o.payload["payload"]["semantic_kind"].as_str())
        .flatten()
}

fn blank(v: &Value) -> bool {
    v.as_str().is_none_or(|s| s.trim().is_empty())
}

/// Deterministic final-graph checks of active semantic objects (S5.1
/// 3, 4, 9, 12). Reference liveness and kinds are checked by
/// [`crate::source_integrity::validate_live_references`].
pub fn graph_diagnostics(final_map: &BTreeMap<String, MapEditObject>) -> Vec<ImpactDiagnostic> {
    let mut out = vec![];
    for o in final_map
        .values()
        .filter(|o| active(o) && is_semantic_kind(&o.kind))
    {
        let id = o.object_id.as_str();
        let p = &o.payload["payload"];
        let missing = |what: &str| {
            fail(
                id,
                "missing_required_field",
                format!("缺少必填字段：{what}"),
            )
        };
        // Theory-typed references.
        for (target, role) in references(o) {
            if matches!(role.as_str(), "home_theory" | "target_context")
                && final_map
                    .get(&target)
                    .filter(|t| active(t))
                    .and_then(semantic_kind_of)
                    .is_some_and(|k| k != "theory")
            {
                out.push(fail(
                    id,
                    "invalid_reference_kind",
                    format!("{role} 必须指向 semantic_kind = theory 的共享对象"),
                ));
            }
        }
        match o.kind.as_str() {
            "SemanticEntity" => {
                if p["preferred_labels"]
                    .as_array()
                    .is_none_or(|v| v.is_empty() || v.iter().any(|l| blank(&l["label"])))
                {
                    out.push(missing("preferred_labels"));
                }
                let kind = p["semantic_kind"].as_str().unwrap_or_default();
                if kind == "theory" && !p["home_theory"].is_null() {
                    out.push(fail(
                        id,
                        "invalid_arg",
                        "理论本身不能设置 home_theory（理论保持扁平，不建父子树）",
                    ));
                }
                if kind != "theory" && p["home_theory"].is_null() {
                    out.push(missing("home_theory（除 theory 外都要挂在一个理论下）"));
                }
                if p["aliases"]
                    .as_array()
                    .is_some_and(|aliases| aliases.iter().any(blank))
                {
                    out.push(fail(id, "invalid_arg", "aliases 不能包含空字符串"));
                }
                match kind {
                    "declaration" => {
                        let d = &p["declaration"];
                        if d.is_null() {
                            out.push(missing("declaration"));
                        } else if d["declaration_kind"] == "definition" {
                            if d["definiens"].is_null() {
                                out.push(missing("declaration.definiens（definition）"));
                            }
                        } else if d["statement"].is_null() {
                            out.push(missing("declaration.statement"));
                        }
                    }
                    "problem_schema" if p["problem_schema"].is_null() => {
                        out.push(missing("problem_schema"));
                    }
                    _ => {}
                }
                if (kind != "declaration" && !p["declaration"].is_null())
                    || (kind != "problem_schema" && !p["problem_schema"].is_null())
                {
                    out.push(fail(
                        id,
                        "invalid_arg",
                        "declaration / problem_schema 只能出现在对应 semantic_kind 上",
                    ));
                }
                if let Some(d) = p["declaration"]["concept"].as_str()
                    && final_map
                        .get(d)
                        .filter(|t| active(t))
                        .and_then(semantic_kind_of)
                        .is_some_and(|k| k != "concept")
                {
                    out.push(fail(
                        id,
                        "invalid_reference_kind",
                        "declaration.concept 必须指向概念（concept）",
                    ));
                }
            }
            "Alignment" => {
                if p["evidence_spans"].as_array().is_none_or(Vec::is_empty) {
                    out.push(missing("evidence_spans（每条对齐线至少 1 条原文证据）"));
                }
            }
            "Debt" => {
                for key in ["obligation", "quantifier"] {
                    if blank(&p[key]) {
                        out.push(missing(key));
                    }
                }
                if matches!(p["status"].as_str(), Some("claimed_paid" | "paid"))
                    && p["paid_by"].as_array().is_none_or(Vec::is_empty)
                {
                    out.push(missing("paid_by（声明已还清时必须引用还清它的对象）"));
                }
            }
            "DialectScope" => {
                let conventions = p["conventions"].as_array();
                if conventions.is_none_or(|c| {
                    c.is_empty()
                        || c.iter()
                            .any(|c| c["evidence_spans"].as_array().is_none_or(Vec::is_empty))
                }) {
                    out.push(missing("conventions（至少一条，且每条都有原文证据）"));
                }
                if blank(&p["label"]) {
                    out.push(missing("label"));
                }
            }
            _ => {}
        }
    }
    out
}

/// Debts attached to an object retired by this request become obsolete
/// (system transition, S6.5). Returns the ids it changed.
pub fn obsolete_orphaned_debts(final_map: &mut BTreeMap<String, MapEditObject>) -> Vec<String> {
    let inactive: BTreeSet<String> = final_map
        .values()
        .filter(|o| !active(o))
        .map(|o| o.object_id.clone())
        .collect();
    let mut changed = vec![];
    for o in final_map.values_mut() {
        if o.kind == "Debt"
            && active(o)
            && o.payload["payload"]["attached_to"]
                .as_str()
                .is_some_and(|t| inactive.contains(t))
        {
            o.payload["payload"]["status"] = json!("obsolete");
            o.lifecycle = "deprecated".into();
            changed.push(o.object_id.clone());
        }
    }
    changed
}

/// Is `unit` the anchor or inside the anchor's subtree (parent chain)?
pub fn scope_covers(
    final_map: &BTreeMap<String, MapEditObject>,
    scope: &MapEditObject,
    unit: &str,
) -> bool {
    let Some(anchor) = scope.payload["payload"]["anchor_unit"].as_str() else {
        return false;
    };
    let mut seen = BTreeSet::new();
    let mut current = Some(unit.to_owned());
    while let Some(id) = current {
        if id == anchor {
            return true;
        }
        if !seen.insert(id.clone()) {
            return false;
        }
        current = final_map
            .get(&id)
            .and_then(|o| o.payload["payload"]["parent_unit"].as_str())
            .map(str::to_owned);
    }
    false
}

/// Non-blocking review item (S5.2): `level` is `warn` or `info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewItem {
    pub level: &'static str,
    pub code: &'static str,
    pub object_id: String,
    pub message: String,
}

/// Review items of the objects in `own` (this proposal's semantic objects)
/// over the final graph. Never blocks.
pub fn review_items(
    final_map: &BTreeMap<String, MapEditObject>,
    own: &BTreeSet<String>,
    target_object: Option<&str>,
) -> Vec<ReviewItem> {
    let mut out = vec![];
    let mut item = |level, code, id: &str, message: String| {
        out.push(ReviewItem {
            level,
            code,
            object_id: id.to_owned(),
            message,
        })
    };
    let mine: Vec<&MapEditObject> = own
        .iter()
        .filter_map(|id| final_map.get(id))
        .filter(|o| active(o) && is_semantic_kind(&o.kind))
        .collect();
    for o in &mine {
        let id = o.object_id.as_str();
        let p = &o.payload["payload"];
        match o.kind.as_str() {
            "Alignment" => {
                let relation = p["relation"].as_str().unwrap_or_default();
                let lost = p["lost_information"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty());
                let params = p["parameter_map"].as_array().is_some_and(|v| !v.is_empty());
                if matches!(relation, "specializes" | "generalizes") && !params && !lost {
                    item(
                        "warn",
                        "relation_detail_missing",
                        id,
                        "特例 / 推广没有写参数对照或对不上的部分".into(),
                    );
                }
                if relation == "overlaps_with" && !lost {
                    item(
                        "warn",
                        "overlap_scope_missing",
                        id,
                        "部分重合却没有写对不上的部分".into(),
                    );
                }
                let needs_certificate = matches!(
                    relation,
                    "specializes"
                        | "generalizes"
                        | "logically_equivalent"
                        | "definitionally_equivalent"
                        | "exact_realization"
                );
                let has_debt = p["required_assumptions"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|a| a["debt"].is_string()))
                    || final_map.values().any(|d| {
                        d.kind == "Debt"
                            && active(d)
                            && d.payload["payload"]["attached_to"] == json!(id)
                    });
                if needs_certificate && !has_debt {
                    item(
                        "warn",
                        "certificate_debt_suggested",
                        id,
                        "这种关系需要证明两边确实相符，建议补一笔对齐账".into(),
                    );
                }
                match p["source_context"]["dialect_scope"].as_str() {
                    Some(scope) => {
                        let source = p["source"].as_str().unwrap_or_default();
                        if final_map
                            .get(scope)
                            .is_some_and(|s| !scope_covers(final_map, s, source))
                        {
                            item(
                                "warn",
                                "dialect_scope_mismatch",
                                id,
                                "引用的约定卡不覆盖这条对齐线的来源单元".into(),
                            );
                        }
                    }
                    None => item(
                        "info",
                        "uncontextualized",
                        id,
                        "这条对齐线没有挂约定卡".into(),
                    ),
                }
                let pair = (p["source"].clone(), p["target"].clone());
                if final_map.values().any(|other| {
                    other.object_id != id
                        && other.kind == "Alignment"
                        && active(other)
                        && (
                            other.payload["payload"]["source"].clone(),
                            other.payload["payload"]["target"].clone(),
                        ) == pair
                }) {
                    item(
                        "warn",
                        "duplicate_alignment",
                        id,
                        "同一来源单元对同一共享对象已有另一条活动对齐线".into(),
                    );
                }
            }
            "DialectScope" => {
                if p["conventions"].as_array().is_some_and(|c| {
                    c.iter().any(|c| {
                        matches!(
                            c["explicitness"].as_str(),
                            Some("inferred" | "conventional")
                        )
                    })
                }) {
                    item(
                        "info",
                        "inferred_convention",
                        id,
                        "约定卡里有推断出来的约定，对齐时不会自动视为已确认".into(),
                    );
                }
            }
            "SemanticEntity" => {
                if p["mathlib_baseline"]["decl"].is_string()
                    && p["mathlib_baseline"]["status"] == "unchecked"
                {
                    item(
                        "info",
                        "mathlib_baseline_unchecked",
                        id,
                        "写了 mathlib 名称但尚未核对；接受后可在审核页「重新核对 mathlib 名称」"
                            .into(),
                    );
                }
                let missing = missing_label_langs(p);
                if !missing.is_empty() {
                    item(
                        "warn",
                        "label_lang_missing",
                        id,
                        format!(
                            "共享对象缺少{}名称；preferred_labels 需同时写 zh 和 en，不论材料用什么语言",
                            missing.join("、")
                        ),
                    );
                }
                let kind = p["semantic_kind"].as_str().unwrap_or_default();
                let same_kind = |other: &&MapEditObject| {
                    other.object_id != id
                        && other.kind == "SemanticEntity"
                        && active(other)
                        && other.payload["payload"]["semantic_kind"] == p["semantic_kind"]
                };
                if matches!(kind, "theory" | "concept") {
                    let keys = name_keys(p);
                    let clashes: Vec<&str> = final_map
                        .values()
                        .filter(same_kind)
                        .filter(|other| {
                            name_keys(&other.payload["payload"])
                                .iter()
                                .any(|k| keys.contains(k))
                        })
                        .map(|other| other.object_id.as_str())
                        .collect();
                    if !clashes.is_empty() {
                        item(
                            "warn",
                            "possible_duplicate_name",
                            id,
                            format!(
                                "名称或别名与已有的同类对象相同：{}；如果是同一个，请复用已有对象",
                                clashes.join("、")
                            ),
                        );
                    }
                }
                if matches!(kind, "concept" | "declaration")
                    && let Some(decl) = p["mathlib_baseline"]["decl"]
                        .as_str()
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                {
                    let clashes: Vec<&str> = final_map
                        .values()
                        .filter(same_kind)
                        .filter(|other| {
                            other.payload["payload"]["mathlib_baseline"]["decl"]
                                .as_str()
                                .map(str::trim)
                                == Some(decl)
                        })
                        .map(|other| other.object_id.as_str())
                        .collect();
                    if !clashes.is_empty() {
                        item(
                            "warn",
                            "duplicate_mathlib_decl",
                            id,
                            format!(
                                "mathlib 名称 {decl} 已被同类对象使用：{}；如果是同一个，请复用已有对象",
                                clashes.join("、")
                            ),
                        );
                    }
                }
                if p["semantic_kind"] == "theory" {
                    let aliases = p["aliases"].as_array().map_or(0, Vec::len);
                    if aliases == 0 {
                        item(
                            "info",
                            "theory_aliases_missing",
                            id,
                            "理论没有写检索同义词（aliases），后续按中英文别称搜索可能不容易找到它"
                                .into(),
                        );
                    }
                    let names: Vec<&str> = p["preferred_labels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l["label"].as_str())
                        .chain(p["plain_label"].as_str())
                        .collect();
                    let formula_like = names.iter().any(|name| {
                        ["=", "＝", "$", "\\", "≤", "≥", "<", ">"]
                            .iter()
                            .any(|mark| name.contains(mark))
                    });
                    if formula_like {
                        item(
                            "warn",
                            "theory_name_too_specific",
                            id,
                            "理论名称里带有具体算式或公式符号；理论应按一类专题命名（如「线性递推数列」），不要写具体式子".into(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(target) = target_object {
        let entities: Vec<&&MapEditObject> =
            mine.iter().filter(|o| o.kind == "SemanticEntity").collect();
        // One target plus its context (theory / concept / definition it
        // uses). More unrelated shared objects → organisational hint.
        let context: BTreeSet<String> = final_map
            .get(target)
            .map(entity_dependencies)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let extra = entities
            .iter()
            .filter(|o| o.object_id != target && !context.contains(&o.object_id))
            .filter(|o| {
                // Context may also be a dependency of the context.
                !entities.iter().any(|c| {
                    context.contains(&c.object_id) && entity_dependencies(c).contains(&o.object_id)
                })
            })
            .count();
        if extra > 0 {
            item(
                "info",
                "multi_target_change_set",
                target,
                format!(
                    "这个提案除了目标对象还带了 {extra} 个无直接关系的共享对象，建议拆成多个提案"
                ),
            );
        }
    }
    out
}

/// Languages every shared object names itself in (contract S2.2).
const REQUIRED_LABEL_LANGS: [(&str, &str); 2] = [("zh", "中文"), ("en", "英文")];

fn missing_label_langs(p: &Value) -> Vec<&'static str> {
    let langs: BTreeSet<&str> = p["preferred_labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["label"].as_str().is_some_and(|s| !s.trim().is_empty()))
        .filter_map(|l| l["lang"].as_str())
        .collect();
    REQUIRED_LABEL_LANGS
        .iter()
        .filter(|(lang, _)| !langs.contains(lang))
        .map(|(_, name)| *name)
        .collect()
}

/// Preferred labels and aliases, folded so that width, case, spaces and
/// punctuation do not tell two names apart.
fn name_keys(p: &Value) -> BTreeSet<String> {
    p["preferred_labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["label"].as_str())
        .chain(
            p["aliases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .map(name_key)
        .filter(|k| !k.is_empty())
        .collect()
}

/// Same folding as workspace search (`mathscape-query` `search::normalize`):
/// width, case, whitespace and punctuation are ignored; math marks such as
/// `/`, `+`, `=` stay significant.
fn name_key(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .filter(|&c| !name_char_ignorable(c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn name_char_ignorable(c: char) -> bool {
    if c.is_whitespace() || c.is_control() {
        return true;
    }
    if c.is_ascii() {
        return c.is_ascii_punctuation() && !".^_+=<>*/'|".contains(c);
    }
    matches!(c,
        '\u{00B7}'
        | '\u{2010}'..='\u{2027}'
        | '\u{2030}'..='\u{205E}'
        | '\u{3001}'..='\u{3006}'
        | '\u{3008}'..='\u{301F}'
        | '\u{30FB}'
        | '\u{FE10}'..='\u{FE6F}'
        | '\u{FF61}'..='\u{FF65}'
    )
}

/// 一个待核对的 mathlib 名称：共享对象 id、当前 revision、名称。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineCandidate {
    pub object_id: String,
    pub revision: String,
    pub decl: String,
    pub current: MathlibBaseline,
}

/// 地图里所有写了 mathlib 名称、仍有效的共享对象。
pub fn baseline_candidates(snapshot: &MapEditSnapshot) -> Vec<BaselineCandidate> {
    snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SemanticEntity" && o.lifecycle == "active")
        .filter_map(|o| {
            let entity: SemanticEntity = semantic(o).ok()?;
            let baseline = entity.draft.mathlib_baseline?;
            let decl = baseline.decl.clone()?;
            Some(BaselineCandidate {
                object_id: o.object_id.clone(),
                revision: o.revision.clone(),
                decl,
                current: baseline,
            })
        })
        .collect()
}

/// 把查找结果编成 `RecordMathlibBaseline` 操作。结果和当前记录完全一样的
/// 对象跳过（不制造空修订）。`lookup` 里没有的名称视为没查（保持原样）。
pub fn baseline_operations(
    candidates: &[BaselineCandidate],
    lookup: &BTreeMap<String, Option<(String, String)>>,
    profile_id: &str,
    mathlib_version: &str,
    method: BaselineCheckMethod,
) -> Vec<UserChangeOperation> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let found = lookup.get(&candidate.decl)?;
            let (status, module, location) = match found {
                Some((module, location)) => (
                    BaselineStatus::Confirmed,
                    Some(module.clone()),
                    Some(location.clone()),
                ),
                None => (BaselineStatus::NotFound, None, None),
            };
            let check = BaselineCheck {
                mathlib_version: mathlib_version.to_owned(),
                profile_id: profile_id.to_owned(),
                method,
                module,
                location,
            };
            if candidate.current.status == status
                && candidate.current.checked.as_ref() == Some(&check)
            {
                return None;
            }
            Some(UserChangeOperation::RecordMathlibBaseline {
                object_id: candidate.object_id.clone(),
                expected_revision: candidate.revision.clone(),
                outcome: BaselineCheckOutcome { status, check },
            })
        })
        .collect()
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn name_key_folds_like_search_and_keeps_math_marks() {
        assert_eq!(name_key("Cauchy–Schwarz"), name_key("cauchy-schwarz"));
        assert_eq!(
            name_key("Cauchy—Schwarz 不等式"),
            name_key("ＣＡＵＣＨＹ　SCHWARZ不等式")
        );
        assert_eq!(name_key("《线性递推》"), name_key("线性递推"));
        assert_ne!(name_key("ℤ/nℤ"), name_key("ℤnℤ"));
        assert_ne!(name_key("a+b"), name_key("ab"));
    }

    #[test]
    fn only_exact_zh_and_en_count_as_label_languages() {
        let labels = |a: &str, b: &str| {
            json!({ "preferred_labels": [
                { "lang": a, "label": "x" }, { "lang": b, "label": "y" }
            ] })
        };
        assert!(missing_label_langs(&labels("zh", "en")).is_empty());
        assert_eq!(missing_label_langs(&labels("zh-CN", "en")), vec!["中文"]);
        assert_eq!(missing_label_langs(&labels("zh", "en-US")), vec!["英文"]);
    }
}
