//! 语义层在来源图上的投影：共享对象、约定卡、一笔账是节点；对齐线、
//! “属于理论”、“挂账”、“约定适用于”是关系。
//!
//! 只读投影：文字全部来自已接受的地图对象，不推断、不提升可信度。

use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{EndpointRole, EntityKind, LifecycleStatus, StableId};
use mathscape_graph::{
    IncidenceDirection, ProjectionEndpoint, ProjectionGroupId, ProjectionNode, ProjectionRelation,
    ProjectionRelationKind, ViewLayer,
};
use mathscape_store_sqlite::StoredMapObject;
use serde_json::Value;

use crate::source_graph::{SourceGraphProjectionError, object_payload, synthetic_relation_id};

pub fn semantic_kind_label(kind: &str) -> &'static str {
    match kind {
        "concept" => "概念",
        "declaration" => "精确说法",
        "theory" => "理论",
        "problem_schema" => "题型",
        _ => "共享对象",
    }
}

pub fn relation_label(relation: &str) -> &'static str {
    match relation {
        "expresses" => "说的就是",
        "specializes" => "是特例",
        "generalizes" => "是推广",
        "instance_of" => "是实例",
        "representation_of" => "换一种表示",
        "logically_equivalent" => "逻辑等价",
        "definitionally_equivalent" => "定义等价",
        "exact_realization" => "精确实现",
        "notation_for" => "是记号",
        "overlaps_with" => "部分重合",
        "incompatible_with" => "对不上",
        _ => "对齐",
    }
}

pub fn alignment_status_label(status: &str) -> &'static str {
    match status {
        "unmatched" => "未匹配",
        "retrieved_candidate" => "候选",
        "context_checked" => "已核对上下文",
        "partially_aligned" => "部分对上",
        "ambiguous" => "有歧义",
        "disputed" => "有争议",
        "incompatible" => "对不上",
        "formally_related" => "形式化相关",
        "semantically_reviewed" => "你已审阅",
        "accepted" => "你已确认",
        "rejected" => "你已否决",
        "superseded" => "已被替代",
        _ => "未知状态",
    }
}

pub fn debt_status_label(status: &str) -> &'static str {
    match status {
        "open" => "还欠着",
        "claimed_paid" => "Agent 说已还清，待你确认",
        "paid" => "你已确认还清",
        "waived" => "你已免除",
        "obsolete" => "已失效",
        _ => "未知状态",
    }
}

pub fn baseline_status_label(status: &str) -> &'static str {
    match status {
        "unchecked" => "未核对",
        "confirmed" => "已在本机 mathlib 中找到",
        "not_found" => "本机 mathlib 中没找到",
        _ => "未知",
    }
}

/// 对齐线的白话标签（契约 S9.1）。Agent 给的 `map_label` 只在模板没有
/// 更具体的说法时作备选。
pub fn alignment_plain_label(payload: &Value) -> String {
    let relation = text(payload, "relation").unwrap_or("");
    let parameter_note = |role: Option<&str>| {
        payload
            .get("parameter_map")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| role.is_none_or(|role| text(entry, "role") == Some(role)))
            .find_map(|entry| {
                let map = text(entry, "map")?.trim();
                let compact: String = map.chars().filter(|c| !c.is_whitespace()).collect();
                let source = text(entry, "source").unwrap_or("");
                let target = text(entry, "target").unwrap_or("");
                let identity = source == target
                    && (compact.is_empty()
                        || ["=", "↦", "→"]
                            .iter()
                            .any(|arrow| compact == format!("{source}{arrow}{target}")));
                (!identity && !map.is_empty()).then(|| map.to_owned())
            })
    };
    let lost_first = payload
        .get("lost_information")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| text(item, "short"))
        .map(str::to_owned);
    let with_note = |base: &str, note: Option<String>| match note {
        Some(note) => format!("{base}：{note}"),
        None => base.to_owned(),
    };
    match relation {
        "expresses" => with_note("说的就是它", parameter_note(None)),
        "specializes" => with_note("特例", parameter_note(Some("statement_parameter"))),
        "generalizes" => with_note("推广", parameter_note(Some("statement_parameter"))),
        "overlaps_with" => with_note("部分重合", lost_first),
        _ => text(payload, "map_label")
            .map(str::to_owned)
            .unwrap_or_else(|| relation_label(relation).to_owned()),
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str)
}

fn entity_label(payload: &Value, lang: mathscape_domain::DisplayLang) -> String {
    mathscape_domain::semantic_display_label(payload, lang)
        .unwrap_or_else(|| lang.fallback_entity_label().to_owned())
}

fn shorten(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

fn entity_summary(payload: &Value) -> String {
    let kind = semantic_kind_label(text(payload, "semantic_kind").unwrap_or(""));
    let mut parts = vec![kind.to_owned()];
    if let Some(baseline) = payload.get("mathlib_baseline") {
        let status = baseline_status_label(text(baseline, "status").unwrap_or(""));
        match text(baseline, "decl") {
            Some(decl) => parts.push(format!("mathlib：{decl}（{status}）")),
            None => parts.push("mathlib：暂无对应名称".to_owned()),
        }
    }
    let conventions = payload
        .get("conventions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if conventions > 0 {
        parts.push(format!("{conventions} 条约定"));
    }
    parts.join(" · ")
}

fn id_at(payload: &Value, key: &str) -> Option<StableId> {
    text(payload, key).and_then(|value| StableId::parse(value).ok())
}

fn endpoint(
    id: StableId,
    role: &str,
    direction: IncidenceDirection,
) -> Result<ProjectionEndpoint, SourceGraphProjectionError> {
    Ok(ProjectionEndpoint::new(
        id,
        EndpointRole::parse(role)?,
        None,
        direction,
    ))
}

/// 把语义对象加到 `nodes` / `relations` 里。端点不在图中的关系跳过。
pub(crate) fn project_semantic_layer(
    objects: &BTreeMap<StableId, &StoredMapObject>,
    group: &ProjectionGroupId,
    nodes: &mut Vec<ProjectionNode>,
    relations: &mut Vec<ProjectionRelation>,
    lang: mathscape_domain::DisplayLang,
) -> Result<(), SourceGraphProjectionError> {
    let of_kind = |kind: EntityKind| {
        objects
            .values()
            .copied()
            .filter(move |object| object.kind == kind)
    };
    for object in of_kind(EntityKind::SemanticEntity) {
        let payload = object_payload(object);
        nodes.push(
            ProjectionNode::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::GlobalSemantic,
                entity_label(payload, lang),
            )?
            .group(group.clone())
            .summary(entity_summary(payload))?
            .lifecycle(object.lifecycle),
        );
    }
    for object in of_kind(EntityKind::DialectScope) {
        let payload = object_payload(object);
        let conventions: Vec<&str> = payload
            .get("conventions")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|c| text(c, "text")).collect())
            .unwrap_or_default();
        nodes.push(
            ProjectionNode::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::LocalSemantic,
                text(payload, "label").unwrap_or("约定卡"),
            )?
            .group(group.clone())
            .summary(format!("约定卡 · {}", conventions.join("；")))?
            .lifecycle(object.lifecycle),
        );
    }
    for object in of_kind(EntityKind::Debt) {
        let payload = object_payload(object);
        let obligation = text(payload, "obligation").unwrap_or("待补的检查");
        nodes.push(
            ProjectionNode::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::Rdtx,
                format!("一笔账：{}", shorten(obligation, 24)),
            )?
            .group(group.clone())
            .summary(format!(
                "{} · {}",
                debt_status_label(text(payload, "status").unwrap_or("")),
                obligation
            ))?
            .lifecycle(object.lifecycle),
        );
    }
    let node_ids: BTreeSet<StableId> = nodes.iter().map(|n| n.entity_id().clone()).collect();
    let mut alignment_target: BTreeMap<StableId, (StableId, String)> = BTreeMap::new();

    for object in of_kind(EntityKind::Alignment) {
        let payload = object_payload(object);
        let (Some(source), Some(target)) = (id_at(payload, "source"), id_at(payload, "target"))
        else {
            continue;
        };
        let relation = relation_label(text(payload, "relation").unwrap_or(""));
        let label = alignment_plain_label(payload);
        alignment_target.insert(object.stable_id.clone(), (target.clone(), label.clone()));
        if !node_ids.contains(&source) || !node_ids.contains(&target) {
            continue;
        }
        let lost = payload
            .get("lost_information")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|l| text(l, "short"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut summary = format!(
            "对齐线 · {relation} · {}",
            alignment_status_label(text(payload, "status").unwrap_or(""))
        );
        if !lost.is_empty() {
            summary.push_str(&format!(" · 对不上的部分：{}", lost.join("；")));
        }
        for entry in payload
            .get("term_index_map")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("index_shift").and_then(Value::as_i64) == Some(0))
        {
            summary.push_str(&format!(
                " · 数列逐项对齐：{} = {}",
                text(entry, "source").unwrap_or("?"),
                text(entry, "target").unwrap_or("?")
            ));
        }
        relations.push(
            ProjectionRelation::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::Alignment,
                label,
                vec![
                    endpoint(source, "source", IncidenceDirection::IntoRelation)?,
                    endpoint(target, "target", IncidenceDirection::OutOfRelation)?,
                ],
            )?
            .relation_kind(ProjectionRelationKind::Aligns)
            .summary(summary)?
            .group(group.clone())
            .lifecycle(object.lifecycle),
        );
    }

    let mut synthetic = |prefix: &str,
                         kind: EntityKind,
                         relation_kind: ProjectionRelationKind,
                         label: &str,
                         summary: String,
                         from: (&StableId, &str),
                         to: (&StableId, &str),
                         lifecycle: LifecycleStatus|
     -> Result<(), SourceGraphProjectionError> {
        if !node_ids.contains(from.0) || !node_ids.contains(to.0) {
            return Ok(());
        }
        relations.push(
            ProjectionRelation::new(
                synthetic_relation_id(prefix, from.0, to.0),
                kind,
                ViewLayer::GlobalSemantic,
                label,
                vec![
                    endpoint(from.0.clone(), from.1, IncidenceDirection::IntoRelation)?,
                    endpoint(to.0.clone(), to.1, IncidenceDirection::OutOfRelation)?,
                ],
            )?
            .relation_kind(relation_kind)
            .summary(summary)?
            .group(group.clone())
            .lifecycle(lifecycle),
        );
        Ok(())
    };

    for object in of_kind(EntityKind::SemanticEntity) {
        let payload = object_payload(object);
        if let Some(theory) = id_at(payload, "home_theory") {
            synthetic(
                "in-theory",
                EntityKind::SemanticEntity,
                ProjectionRelationKind::InTheory,
                "属于理论",
                "共享对象所在的理论".to_owned(),
                (&object.stable_id, "member"),
                (&theory, "theory"),
                object.lifecycle,
            )?;
        }
        if let Some(concept) = payload
            .pointer("/declaration/concept")
            .and_then(Value::as_str)
            .and_then(|value| StableId::parse(value).ok())
            && concept != object.stable_id
        {
            synthetic(
                "of-concept",
                EntityKind::SemanticEntity,
                ProjectionRelationKind::OfConcept,
                "归属概念",
                "精确说法所属的概念".to_owned(),
                (&object.stable_id, "declaration"),
                (&concept, "concept"),
                object.lifecycle,
            )?;
        }
        let mut seen_used = BTreeSet::new();
        for ptr in [
            "/declaration/uses_entities",
            "/problem_schema/uses_entities",
        ] {
            for used in payload
                .pointer(ptr)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|value| StableId::parse(value).ok())
            {
                if used == object.stable_id || !seen_used.insert(used.clone()) {
                    continue;
                }
                synthetic(
                    "uses-entity",
                    EntityKind::SemanticEntity,
                    ProjectionRelationKind::UsesEntity,
                    "引用对象",
                    "该共享对象引用的其他共享对象".to_owned(),
                    (&object.stable_id, "user"),
                    (&used, "used"),
                    object.lifecycle,
                )?;
            }
        }
    }
    for object in of_kind(EntityKind::DialectScope) {
        if let Some(anchor) = id_at(object_payload(object), "anchor_unit") {
            synthetic(
                "scope-of",
                EntityKind::DialectScope,
                ProjectionRelationKind::ScopeOf,
                "约定适用于",
                "约定卡适用于这一节及其下的内容".to_owned(),
                (&object.stable_id, "scope"),
                (&anchor, "anchor"),
                object.lifecycle,
            )?;
        }
    }
    for object in of_kind(EntityKind::Debt) {
        let payload = object_payload(object);
        if let Some(attached) = id_at(payload, "attached_to") {
            // 挂在对齐线上的账，画到对齐线指向的共享对象旁边。
            let (holder, summary) = match alignment_target.get(&attached) {
                Some((target, label)) => (target.clone(), format!("挂在对齐线「{label}」上")),
                None => (attached, "挂在这个对象上".to_owned()),
            };
            synthetic(
                "debt-on",
                EntityKind::Debt,
                ProjectionRelationKind::DebtOn,
                "挂账",
                summary,
                (&object.stable_id, "debt"),
                (&holder, "holder"),
                object.lifecycle,
            )?;
        }
        let mut seen_payers = BTreeSet::new();
        for entry in payload
            .get("paid_by")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(payer) = id_at(entry, "ref") else {
                continue;
            };
            if payer == object.stable_id || !seen_payers.insert(payer.clone()) {
                continue;
            };
            let summary = match text(entry, "note").filter(|s| !s.trim().is_empty()) {
                Some(note) => format!("由该对象偿还：{note}"),
                None => "偿还这笔账的对象".to_owned(),
            };
            synthetic(
                "paid-by",
                EntityKind::Debt,
                ProjectionRelationKind::PaidBy,
                "由…偿还",
                summary,
                (&object.stable_id, "debt"),
                (&payer, "payer"),
                object.lifecycle,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mathscape_domain::{EntityKind, LifecycleStatus, RevisionId, StableId};
    use mathscape_graph::{ProjectionLens, ProjectionRelationKind};
    use mathscape_store_sqlite::{SourceMapSnapshot, StoredMapObject};
    use serde_json::{Value, json};

    use crate::source_graph::project_source_map;

    fn object(id: &str, n: usize, kind: EntityKind, payload: Value) -> StoredMapObject {
        StoredMapObject {
            stable_id: StableId::parse(id).unwrap(),
            revision_id: RevisionId::parse(format!("sha256:{:064x}", n)).unwrap(),
            kind,
            relation: false,
            payload: json!({ "payload": payload }),
            lifecycle: LifecycleStatus::Active,
        }
    }

    #[test]
    fn projects_shared_objects_alignments_debts_and_scopes() {
        let snapshot = SourceMapSnapshot {
            objects: vec![
                object(
                    "mathscape:source-unit/sec",
                    1,
                    EntityKind::SourceSemanticUnit,
                    json!({ "display_title": "第一节", "kind": "section" }),
                ),
                object(
                    "mathscape:source-unit/def",
                    2,
                    EntityKind::SourceSemanticUnit,
                    json!({ "display_title": "定义 1", "kind": "definition" }),
                ),
                object(
                    "mathscape:semantic-entity/theory",
                    3,
                    EntityKind::SemanticEntity,
                    json!({ "semantic_kind": "theory",
                            "preferred_labels": [{ "lang": "zh", "label": "斐波那契数列" }] }),
                ),
                object(
                    "mathscape:semantic-entity/fib",
                    4,
                    EntityKind::SemanticEntity,
                    json!({ "semantic_kind": "concept",
                            "home_theory": "mathscape:semantic-entity/theory",
                            "preferred_labels": [{ "lang": "en", "label": "Fibonacci" }],
                            "plain_label": "斐波那契数",
                            "mathlib_baseline": { "decl": "Nat.fib", "status": "confirmed" } }),
                ),
                object(
                    "mathscape:semantic-entity/cassini",
                    8,
                    EntityKind::SemanticEntity,
                    json!({ "semantic_kind": "declaration",
                            "home_theory": "mathscape:semantic-entity/theory",
                            "plain_label": "Cassini 恒等式",
                            "declaration": {
                                "concept": "mathscape:semantic-entity/fib",
                                "uses_entities": ["mathscape:semantic-entity/fib"]
                            } }),
                ),
                object(
                    "mathscape:alignment/a",
                    5,
                    EntityKind::Alignment,
                    json!({ "source": "mathscape:source-unit/def",
                            "target": "mathscape:semantic-entity/fib",
                            "relation": "expresses", "status": "retrieved_candidate",
                            "map_label": "说的就是",
                            "lost_information": [{ "short": "甲从 F₁ 开始" }],
                            "term_index_map": [{ "source": "f_k", "target": "F_k", "index_shift": 0 }] }),
                ),
                object(
                    "mathscape:debt/d",
                    6,
                    EntityKind::Debt,
                    json!({ "attached_to": "mathscape:alignment/a", "status": "claimed_paid",
                            "obligation": "核对首项",
                            "paid_by": [{ "ref": "mathscape:semantic-entity/cassini", "note": "归纳可证" }] }),
                ),
                object(
                    "mathscape:dialect-scope/s",
                    7,
                    EntityKind::DialectScope,
                    json!({ "label": "甲书约定", "anchor_unit": "mathscape:source-unit/sec",
                            "conventions": [{ "text": "数列从 F₁ = 1 开始" }] }),
                ),
            ],
            relation_endpoints: Default::default(),
        };
        let projection = project_source_map(&snapshot, ProjectionLens::Source).unwrap();
        assert_eq!(projection.nodes().len(), 7);
        let fib = projection
            .nodes()
            .iter()
            .find(|n| n.entity_id().as_str() == "mathscape:semantic-entity/fib")
            .unwrap();
        assert_eq!(fib.label(), "斐波那契数");
        assert_eq!(
            fib.summary_ref(),
            Some("概念 · mathlib：Nat.fib（已在本机 mathlib 中找到）")
        );
        let kinds: Vec<ProjectionRelationKind> = projection
            .relations()
            .iter()
            .map(|r| r.relation_kind_value())
            .collect();
        for expected in [
            ProjectionRelationKind::Aligns,
            ProjectionRelationKind::InTheory,
            ProjectionRelationKind::DebtOn,
            ProjectionRelationKind::ScopeOf,
            ProjectionRelationKind::UsesEntity,
            ProjectionRelationKind::OfConcept,
            ProjectionRelationKind::PaidBy,
        ] {
            assert!(kinds.contains(&expected), "{expected:?} missing: {kinds:?}");
        }
        let alignment = projection
            .relations()
            .iter()
            .find(|r| r.relation_kind_value() == ProjectionRelationKind::Aligns)
            .unwrap();
        assert_eq!(alignment.label(), "说的就是它");
        assert_eq!(
            alignment.summary_ref(),
            Some("对齐线 · 说的就是 · 候选 · 对不上的部分：甲从 F₁ 开始 · 数列逐项对齐：f_k = F_k")
        );
    }

    #[test]
    fn plain_labels_follow_the_contract_templates() {
        use super::alignment_plain_label;
        assert_eq!(
            alignment_plain_label(&json!({ "relation": "specializes",
                "parameter_map": [{ "source": "n", "target": "n", "role": "statement_parameter",
                                    "map": "乙的第 n 条 ＝ 共享的第 n+1 条" }] })),
            "特例：乙的第 n 条 ＝ 共享的第 n+1 条"
        );
        assert_eq!(
            alignment_plain_label(&json!({ "relation": "expresses",
                "parameter_map": [{ "source": "n", "target": "n", "role": "statement_parameter", "map": "n = n" }] })),
            "说的就是它"
        );
        assert_eq!(
            alignment_plain_label(&json!({ "relation": "expresses",
                "parameter_map": [{ "source": "n", "target": "n", "role": "statement_parameter", "map": "n ↦ n" }] })),
            "说的就是它"
        );
        assert_eq!(
            alignment_plain_label(&json!({ "relation": "overlaps_with",
                "lost_information": [{ "short": "缺 n = 1" }] })),
            "部分重合：缺 n = 1"
        );
        for label in [
            alignment_plain_label(&json!({ "relation": "generalizes" })),
            alignment_plain_label(&json!({ "relation": "instance_of" })),
        ] {
            for banned in ["平移", "错开", "滑动"] {
                assert!(!label.contains(banned));
            }
        }
    }
}
