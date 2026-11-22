//! 语义层的只读视图：学习视图（契约 S9.1）与单个语义对象的详情。
//!
//! 只计算、不写入任何对象；不推断数学内容，也不提升可信度。

use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{MapEditObject, MapEditSnapshot};
use serde::Serialize;
use serde_json::Value;

fn payload(o: &MapEditObject) -> &Value {
    o.payload.get("payload").unwrap_or(&o.payload)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn live(o: &MapEditObject) -> bool {
    crate::source_integrity::is_live(o)
}

/// 共享对象按显示语言取的名称（见 `semantic_display_label`）。
pub fn entity_label(o: &MapEditObject, lang: mathscape_domain::DisplayLang) -> String {
    mathscape_domain::semantic_display_label(payload(o), lang)
        .unwrap_or_else(|| lang.fallback_entity_label().to_owned())
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LearningUnit {
    pub unit_id: String,
    pub unit_title: String,
    pub alignment_id: String,
    pub entity_id: String,
    pub entity_label: String,
    pub relation: String,
    /// 「别的材料里见过」或「新的」。
    pub label: String,
    /// 还有哪些材料对齐到同一个共享对象（材料 id）。
    pub seen_in: Vec<String>,
    /// `lost_information[].short`，显示为“只差：…”。
    pub only_differs_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LearningConvention {
    pub scope_id: String,
    pub scope_label: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LearningDebt {
    pub debt_id: String,
    pub obligation: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StudyOutlineTarget {
    pub alignment_id: String,
    pub entity_id: String,
    pub entity_label: String,
    pub relation: String,
    pub only_differs_by: Vec<String>,
    pub seen_in: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mastered_in: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub staged: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StudyOutlineUnit {
    pub unit_id: String,
    pub unit_title: String,
    pub unit_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_unit: Option<String>,
    /// `unread` | `studying` | `mastered`
    pub study_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_note: Option<String>,
    /// `mastered_elsewhere` | `seen_elsewhere` | `fresh` | `unaligned`
    pub category: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub staged: bool,
    pub targets: Vec<StudyOutlineTarget>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StudyConventionItem {
    pub scope_id: String,
    pub scope_label: String,
    pub key: String,
    pub text: String,
    /// `true` when `(key, value)` matches the shared baseline convention on an aligned entity.
    pub is_baseline_match: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct StudyMarkEntry {
    pub status: String,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct LearningViewContext {
    /// Physical snapshot artifact id → folded canonical artifact id.
    pub artifact_aliases: BTreeMap<String, String>,
    /// User-local study progress sidecar keyed by `object_id`.
    pub study_marks: BTreeMap<String, StudyMarkEntry>,
    /// Staged object ids when `includes_staged` preview is enabled.
    pub staged_ids: BTreeSet<String>,
    pub includes_staged: bool,
    /// Language in which shared objects are named.
    pub display_lang: mathscape_domain::DisplayLang,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LearningMaterial {
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliased_artifact_ids: Vec<String>,
    pub units: Vec<LearningUnit>,
    /// 本材料约定卡上的约定，标「新约定」（保留契约向后兼容）。
    pub new_conventions: Vec<LearningConvention>,
    /// 经过基线与已掌握材料比对后的约定明细（区分「同共享基线」与「本材料特有约定」）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conventions_detail: Vec<StudyConventionItem>,
    /// 按材料内物理位置（页码、字节偏移）排序的完整注册单元提纲（含未对齐单元与学习标记）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline_units: Vec<StudyOutlineUnit>,
    /// 挂在本材料相关对齐线 / 共享对象上的未还清账（open + claimed_paid）。
    pub open_debts: Vec<LearningDebt>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
pub struct LearningView {
    pub materials: Vec<LearningMaterial>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub includes_staged: bool,
}

struct SpanLocatorMeta {
    material_id: String,
    raw_artifact: String,
    page: u64,
    byte_start: u64,
}

fn span_locators(
    snapshot: &MapEditSnapshot,
    aliases: &BTreeMap<String, String>,
) -> BTreeMap<String, SpanLocatorMeta> {
    snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SourceSpan")
        .filter_map(|o| {
            let p = payload(o);
            let raw_artifact = p
                .pointer("/markdown/artifact")
                .or_else(|| p.pointer("/ink/snapshot_artifact"))?
                .as_str()?
                .to_owned();
            let material_id = aliases
                .get(&raw_artifact)
                .cloned()
                .unwrap_or_else(|| raw_artifact.clone());
            let page = p
                .pointer("/page/pdf_page_index")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            let byte_start = p
                .pointer("/markdown/byte_range/start")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            Some((
                o.object_id.clone(),
                SpanLocatorMeta {
                    material_id,
                    raw_artifact,
                    page,
                    byte_start,
                },
            ))
        })
        .collect()
}

/// 计算学习视图（无额外上下文，保持原有无状态行为）。
pub fn learning_view(snapshot: &MapEditSnapshot) -> LearningView {
    learning_view_with_context(snapshot, &LearningViewContext::default())
}

/// 计算增强版材料研读概览（支持 canonical 折叠、完整单元大纲、本地学习进度与暂存预览）。
pub fn learning_view_with_context(
    snapshot: &MapEditSnapshot,
    ctx: &LearningViewContext,
) -> LearningView {
    let objects: BTreeMap<&str, &MapEditObject> = snapshot
        .objects
        .iter()
        .map(|o| (o.object_id.as_str(), o))
        .collect();
    let locators = span_locators(snapshot, &ctx.artifact_aliases);

    // 来源单元 → (折叠后材料 id, (最小页码, 最小字节偏移))
    let mut unit_material: BTreeMap<String, String> = BTreeMap::new();
    let mut unit_position: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut material_raw_artifacts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for o in snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SourceSemanticUnit" && live(o))
    {
        let p = payload(o);
        let spans: Vec<&SpanLocatorMeta> = p
            .get("source_spans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|span_id| locators.get(span_id))
            .collect();
        let Some(&first) = spans.first() else {
            continue;
        };
        let pos = spans
            .iter()
            .map(|s| (s.page, s.byte_start))
            .min()
            .unwrap_or((first.page, first.byte_start));
        unit_material.insert(o.object_id.clone(), first.material_id.clone());
        unit_position.insert(o.object_id.clone(), pos);
        for s in &spans {
            if s.raw_artifact != first.material_id {
                material_raw_artifacts
                    .entry(first.material_id.clone())
                    .or_default()
                    .insert(s.raw_artifact.clone());
            }
        }
    }
    for (raw, canonical) in &ctx.artifact_aliases {
        if raw != canonical {
            material_raw_artifacts
                .entry(canonical.clone())
                .or_default()
                .insert(raw.clone());
        }
    }

    let status_of = |id: &str| -> &str {
        ctx.study_marks
            .get(id)
            .map(|m| m.status.as_str())
            .unwrap_or("unread")
    };

    let alignments: Vec<&MapEditObject> = snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "Alignment" && live(o))
        .collect();

    // 共享对象 → 对齐到它的材料集合 & 已掌握该对象的材料集合
    let mut entity_materials: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut entity_mastered_materials: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for a in &alignments {
        let p = payload(a);
        if let (Some(source), Some(target)) = (text(p, "source"), text(p, "target"))
            && let Some(material) = unit_material.get(source)
        {
            entity_materials
                .entry(target)
                .or_default()
                .insert(material.as_str());
            if status_of(source) == "mastered" || status_of(target) == "mastered" {
                entity_mastered_materials
                    .entry(target)
                    .or_default()
                    .insert(material.as_str());
            }
        }
    }

    let make_material = |material_id: &str| -> LearningMaterial {
        let aliased = material_raw_artifacts
            .get(material_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        LearningMaterial {
            artifact_id: material_id.to_owned(),
            aliased_artifact_ids: aliased,
            units: Vec::new(),
            new_conventions: Vec::new(),
            conventions_detail: Vec::new(),
            outline_units: Vec::new(),
            open_debts: Vec::new(),
        }
    };

    let mut by_material: BTreeMap<&str, LearningMaterial> = BTreeMap::new();
    for material in unit_material.values() {
        by_material
            .entry(material.as_str())
            .or_insert_with(|| make_material(material));
    }

    let mut material_holders: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut unit_targets: BTreeMap<&str, Vec<StudyOutlineTarget>> = BTreeMap::new();
    let mut material_entities: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();

    for a in &alignments {
        let p = payload(a);
        let (Some(source), Some(target)) = (text(p, "source"), text(p, "target")) else {
            continue;
        };
        let (Some(material), Some(unit), Some(entity)) = (
            unit_material.get(source),
            objects.get(source),
            objects.get(target),
        ) else {
            continue;
        };
        if !live(entity) {
            continue;
        }
        let seen_in: Vec<String> = entity_materials
            .get(target)
            .into_iter()
            .flatten()
            .filter(|m| **m != material.as_str())
            .map(|m| (*m).to_owned())
            .collect();
        let mastered_in: Vec<String> = entity_mastered_materials
            .get(target)
            .into_iter()
            .flatten()
            .filter(|m| **m != material.as_str())
            .map(|m| (*m).to_owned())
            .collect();
        let only_differs_by: Vec<String> = p
            .get("lost_information")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|l| text(l, "short").map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let holders = material_holders.entry(material.as_str()).or_default();
        holders.insert(a.object_id.as_str());
        holders.insert(target);
        material_entities
            .entry(material.as_str())
            .or_default()
            .insert(target);

        let ent_label = entity_label(entity, ctx.display_lang);
        let rel = text(p, "relation").unwrap_or("").to_owned();
        let is_staged =
            ctx.staged_ids.contains(a.object_id.as_str()) || ctx.staged_ids.contains(target);

        unit_targets
            .entry(source)
            .or_default()
            .push(StudyOutlineTarget {
                alignment_id: a.object_id.clone(),
                entity_id: target.to_owned(),
                entity_label: ent_label.clone(),
                relation: rel.clone(),
                only_differs_by: only_differs_by.clone(),
                seen_in: seen_in.clone(),
                mastered_in,
                staged: is_staged,
            });

        by_material
            .entry(material.as_str())
            .or_insert_with(|| make_material(material))
            .units
            .push(LearningUnit {
                unit_id: source.to_owned(),
                unit_title: text(payload(unit), "display_title")
                    .unwrap_or("来源单元")
                    .to_owned(),
                alignment_id: a.object_id.clone(),
                entity_id: target.to_owned(),
                entity_label: ent_label,
                relation: rel,
                label: if seen_in.is_empty() {
                    "新的".to_owned()
                } else {
                    "别的材料里见过".to_owned()
                },
                seen_in,
                only_differs_by,
            });
    }

    for u in snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SourceSemanticUnit" && live(o))
    {
        let Some(material) = unit_material.get(&u.object_id) else {
            continue;
        };
        let p = payload(u);
        let targets = unit_targets
            .remove(u.object_id.as_str())
            .unwrap_or_default();
        let category = if targets.is_empty() {
            "unaligned"
        } else if targets.iter().any(|t| !t.mastered_in.is_empty()) {
            "mastered_elsewhere"
        } else if targets.iter().any(|t| !t.seen_in.is_empty()) {
            "seen_elsewhere"
        } else {
            "fresh"
        };
        let mark = ctx.study_marks.get(&u.object_id);
        let parent_unit = text(p, "parent_unit")
            .or_else(|| text(p, "parent"))
            .map(str::to_owned);
        let unit_staged = ctx.staged_ids.contains(&u.object_id) || targets.iter().any(|t| t.staged);

        if let Some(entry) = by_material.get_mut(material.as_str()) {
            entry.outline_units.push(StudyOutlineUnit {
                unit_id: u.object_id.clone(),
                unit_title: text(p, "display_title").unwrap_or("来源单元").to_owned(),
                unit_kind: text(p, "kind").unwrap_or("statement").to_owned(),
                parent_unit,
                study_status: mark
                    .map(|m| m.status.clone())
                    .unwrap_or_else(|| "unread".to_owned()),
                study_note: mark.and_then(|m| m.note.clone()),
                category: category.to_owned(),
                staged: unit_staged,
                targets,
            });
        }
    }

    let mut baseline_conventions: BTreeMap<(&str, String), BTreeSet<String>> = BTreeMap::new();
    for (mat, ent_ids) in &material_entities {
        for ent_id in ent_ids {
            if let Some(ent) = objects.get(ent_id) {
                for conv in payload(ent)
                    .get("conventions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let (Some(k), Some(v)) = (text(conv, "key"), conv.get("value")) {
                        let val_str = match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        baseline_conventions
                            .entry((*mat, k.to_owned()))
                            .or_default()
                            .insert(val_str);
                    }
                }
            }
        }
    }

    for scope in snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "DialectScope" && live(o))
    {
        let p = payload(scope);
        let Some(material) = text(p, "anchor_unit").and_then(|u| unit_material.get(u)) else {
            continue;
        };
        let scope_label = text(p, "label").unwrap_or("约定卡").to_owned();
        let entry = by_material
            .entry(material.as_str())
            .or_insert_with(|| make_material(material));
        for convention in p
            .get("conventions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(t) = text(convention, "text") {
                entry.new_conventions.push(LearningConvention {
                    scope_id: scope.object_id.clone(),
                    scope_label: scope_label.clone(),
                    text: t.to_owned(),
                });
                let key = text(convention, "key").unwrap_or("other").to_owned();
                let val_str = convention
                    .get("value")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                let baseline_vals = baseline_conventions.get(&(material.as_str(), key.clone()));
                let is_baseline_match = convention.get("basis").and_then(Value::as_str)
                    == Some("mathlib")
                    || baseline_vals.is_some_and(|vals| vals.contains(&val_str));
                let diff_note = if !is_baseline_match
                    && let Some(vals) = baseline_vals
                    && !vals.is_empty()
                {
                    let joined = vals.iter().cloned().collect::<Vec<_>>().join(" / ");
                    Some(format!("与共享基线取值（{joined}）不同"))
                } else {
                    None
                };
                entry.conventions_detail.push(StudyConventionItem {
                    scope_id: scope.object_id.clone(),
                    scope_label: scope_label.clone(),
                    key,
                    text: t.to_owned(),
                    is_baseline_match,
                    diff_note,
                });
            }
        }
    }

    for debt in snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "Debt" && live(o))
    {
        let p = payload(debt);
        let status = text(p, "status").unwrap_or("");
        if !matches!(status, "open" | "claimed_paid") {
            continue;
        }
        let Some(attached) = text(p, "attached_to") else {
            continue;
        };
        for (material, holders) in &material_holders {
            if holders.contains(attached)
                && let Some(entry) = by_material.get_mut(material)
            {
                entry.open_debts.push(LearningDebt {
                    debt_id: debt.object_id.clone(),
                    obligation: text(p, "obligation").unwrap_or("").to_owned(),
                    status: status.to_owned(),
                });
            }
        }
    }

    let mut materials: Vec<LearningMaterial> = by_material.into_values().collect();
    for m in &mut materials {
        m.units.sort_by(|a, b| {
            (a.unit_title.as_str(), a.entity_label.as_str())
                .cmp(&(b.unit_title.as_str(), b.entity_label.as_str()))
        });
        m.outline_units.sort_by(|a, b| {
            let pos_a = unit_position
                .get(&a.unit_id)
                .copied()
                .unwrap_or((u64::MAX, u64::MAX));
            let pos_b = unit_position
                .get(&b.unit_id)
                .copied()
                .unwrap_or((u64::MAX, u64::MAX));
            (pos_a.0, pos_a.1, a.unit_title.as_str()).cmp(&(
                pos_b.0,
                pos_b.1,
                b.unit_title.as_str(),
            ))
        });
    }
    LearningView {
        materials,
        includes_staged: ctx.includes_staged,
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct StagedSemanticMeta {
    pub change_set_id: String,
    pub change_set_version: i64,
    pub change_sets: Vec<String>,
    pub replaces_revision: Option<String>,
    pub accepted_payload: Option<Value>,
}

/// 一个语义对象的详情：原始 payload 加上它引用的对象的标题。
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SemanticObjectDetail {
    pub object_id: String,
    pub kind: String,
    pub lifecycle: String,
    pub revision: String,
    pub payload: Value,
    /// payload 里出现的对象 id → 标题。
    pub titles: BTreeMap<String, String>,
    /// 挂在它上面的账（对齐线、共享对象）。
    pub debts: Vec<Value>,
    /// 若该对象来自当前调查的未审批提案，附带暂存归属与已接受旧版本。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged: Option<StagedSemanticMeta>,
}

pub fn semantic_object_detail(
    snapshot: &MapEditSnapshot,
    object_id: &str,
    lang: mathscape_domain::DisplayLang,
) -> Option<SemanticObjectDetail> {
    let o = snapshot.objects.iter().find(|o| o.object_id == object_id)?;
    if !matches!(
        o.kind.as_str(),
        "SemanticEntity" | "Alignment" | "Debt" | "DialectScope"
    ) {
        return None;
    }
    let mut ids = BTreeSet::new();
    collect_ids(payload(o), &mut ids);
    let debts: Vec<Value> = snapshot
        .objects
        .iter()
        .filter(|d| d.kind == "Debt" && live(d))
        .filter(|d| text(payload(d), "attached_to") == Some(object_id))
        .map(|d| {
            collect_ids(payload(d), &mut ids);
            let mut value = payload(d).clone();
            if let Some(map) = value.as_object_mut() {
                map.insert("object_id".into(), Value::from(d.object_id.clone()));
            }
            value
        })
        .collect();
    let titles = snapshot
        .objects
        .iter()
        .filter(|other| ids.contains(other.object_id.as_str()))
        .map(|other| {
            (
                other.object_id.clone(),
                semantic_object_title_in(snapshot, other, lang),
            )
        })
        .collect();
    Some(SemanticObjectDetail {
        object_id: o.object_id.clone(),
        kind: o.kind.clone(),
        lifecycle: o.lifecycle.clone(),
        revision: o.revision.clone(),
        payload: payload(o).clone(),
        titles,
        debts,
        staged: None,
    })
}

pub fn semantic_object_title(o: &MapEditObject, lang: mathscape_domain::DisplayLang) -> String {
    title(o, lang)
}

pub fn semantic_object_title_in(
    snapshot: &MapEditSnapshot,
    o: &MapEditObject,
    lang: mathscape_domain::DisplayLang,
) -> String {
    if o.kind == "Alignment" {
        let p = payload(o);
        let rel = relation_plain(text(p, "relation").unwrap_or(""));
        let label = text(p, "map_label").unwrap_or(rel);
        let src = text(p, "source").and_then(|id| {
            snapshot
                .objects
                .iter()
                .find(|cand| cand.object_id == id)
                .map(|cand| title(cand, lang))
        });
        let dst = text(p, "target").and_then(|id| {
            snapshot
                .objects
                .iter()
                .find(|cand| cand.object_id == id)
                .map(|cand| title(cand, lang))
        });
        if let (Some(src), Some(dst)) = (src, dst) {
            return format!("{src} —{label}→ {dst}");
        }
    }
    title(o, lang)
}

fn relation_plain(relation: &str) -> &'static str {
    match relation {
        "expresses" => "说的就是它",
        "specializes" => "特例",
        "generalizes" => "推广",
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

fn title(o: &MapEditObject, lang: mathscape_domain::DisplayLang) -> String {
    let p = payload(o);
    match o.kind.as_str() {
        "SemanticEntity" => entity_label(o, lang),
        "Alignment" => p
            .get("map_label")
            .and_then(Value::as_str)
            .or_else(|| p.get("relation").and_then(Value::as_str))
            .unwrap_or("对齐线")
            .to_owned(),
        "DialectScope" => text(p, "label").unwrap_or("约定卡").to_owned(),
        "Debt" => text(p, "obligation").unwrap_or("一笔账").to_owned(),
        "SourceSpan" => text(p, "quote")
            .or_else(|| p.pointer("/markdown/quote").and_then(Value::as_str))
            .unwrap_or("原文证据")
            .to_owned(),
        _ => text(p, "display_title")
            .or_else(|| p.pointer("/source_title/text").and_then(Value::as_str))
            .or_else(|| {
                p.get("source_labels")?
                    .as_array()?
                    .first()?
                    .get("text")?
                    .as_str()
            })
            .or_else(|| text(p, "map_label"))
            .unwrap_or(&o.object_id)
            .to_owned(),
    }
}

fn collect_ids<'a>(v: &'a Value, out: &mut BTreeSet<&'a str>) {
    match v {
        Value::String(s) if s.starts_with("mathscape:") => {
            out.insert(s.as_str());
        }
        Value::Array(items) => items.iter().for_each(|i| collect_ids(i, out)),
        Value::Object(map) => map.values().for_each(|i| collect_ids(i, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(id: &str, kind: &str, payload: Value) -> MapEditObject {
        MapEditObject {
            object_id: id.into(),
            revision: "r1".into(),
            kind: kind.into(),
            lifecycle: "active".into(),
            payload: json!({ "payload": payload }),
            endpoints: vec![],
        }
    }

    #[test]
    fn folds_canonical_segments_orders_outline_by_page_and_tracks_mastered_and_conventions() {
        let snap = MapEditSnapshot {
            base_commit: None,
            adopted_by: Default::default(),
            objects: vec![
                // Book A has two physical chapter snapshots folded into one canonical markdown id
                obj(
                    "mathscape:source-span/a1",
                    "SourceSpan",
                    json!({ "markdown": { "artifact": "md-a-ch1", "byte_range": { "start": 100, "end": 120 } }, "page": { "pdf_page_index": 2 } }),
                ),
                obj(
                    "mathscape:source-span/a2",
                    "SourceSpan",
                    json!({ "markdown": { "artifact": "md-a-ch2", "byte_range": { "start": 10, "end": 40 } }, "page": { "pdf_page_index": 9 } }),
                ),
                obj(
                    "mathscape:source-span/a0",
                    "SourceSpan",
                    json!({ "markdown": { "artifact": "md-a-ch1", "byte_range": { "start": 0, "end": 20 } }, "page": { "pdf_page_index": 1 } }),
                ),
                // Book B
                obj(
                    "mathscape:source-span/b1",
                    "SourceSpan",
                    json!({ "markdown": { "artifact": "md-b", "byte_range": { "start": 0, "end": 30 } }, "page": { "pdf_page_index": 0 } }),
                ),
                // Units in Book A: section (unaligned), def (page 2), corollary (page 9, ch2)
                obj(
                    "mathscape:source-unit/a-sec",
                    "SourceSemanticUnit",
                    json!({ "kind": "section", "display_title": "§1 数列基础", "source_spans": ["mathscape:source-span/a0"] }),
                ),
                obj(
                    "mathscape:source-unit/a-cor",
                    "SourceSemanticUnit",
                    json!({ "kind": "statement", "display_title": "推论 2.1", "parent_unit": "mathscape:source-unit/a-sec", "source_spans": ["mathscape:source-span/a2"] }),
                ),
                obj(
                    "mathscape:source-unit/a-def",
                    "SourceSemanticUnit",
                    json!({ "kind": "definition", "display_title": "定义 1.1", "parent_unit": "mathscape:source-unit/a-sec", "source_spans": ["mathscape:source-span/a1"] }),
                ),
                // Unit in Book B
                obj(
                    "mathscape:source-unit/b-def",
                    "SourceSemanticUnit",
                    json!({ "kind": "definition", "display_title": "定义 B.1", "source_spans": ["mathscape:source-span/b1"] }),
                ),
                // Shared entity with baseline convention 0
                obj(
                    "mathscape:semantic-entity/fib",
                    "SemanticEntity",
                    json!({
                        "semantic_kind": "concept",
                        "plain_label": "Fibonacci 数列",
                        "conventions": [{ "key": "sequence_index_origin", "value": 0, "text": "下标从 0 起" }]
                    }),
                ),
                // Both ch1 and ch2 of Book A align to `fib`, plus Book B aligns to `fib`
                obj(
                    "mathscape:alignment/al-a1",
                    "Alignment",
                    json!({ "source": "mathscape:source-unit/a-def", "target": "mathscape:semantic-entity/fib", "relation": "expresses" }),
                ),
                obj(
                    "mathscape:alignment/al-a2",
                    "Alignment",
                    json!({ "source": "mathscape:source-unit/a-cor", "target": "mathscape:semantic-entity/fib", "relation": "specializes" }),
                ),
                obj(
                    "mathscape:alignment/al-b1",
                    "Alignment",
                    json!({ "source": "mathscape:source-unit/b-def", "target": "mathscape:semantic-entity/fib", "relation": "overlaps_with", "lost_information": [{ "short": "无第 0 项" }] }),
                ),
                // DialectScopes: Book A uses 0 (matches baseline), Book B uses 1 (differs from baseline)
                obj(
                    "mathscape:dialect-scope/sc-a",
                    "DialectScope",
                    json!({
                        "anchor_unit": "mathscape:source-unit/a-sec",
                        "label": "甲书约定",
                        "conventions": [{ "key": "sequence_index_origin", "value": 0, "text": "下标从 0 起" }]
                    }),
                ),
                obj(
                    "mathscape:dialect-scope/sc-b",
                    "DialectScope",
                    json!({
                        "anchor_unit": "mathscape:source-unit/b-def",
                        "label": "乙书约定",
                        "conventions": [{ "key": "sequence_index_origin", "value": 1, "text": "下标从 1 起" }]
                    }),
                ),
            ],
        };

        let mut ctx = LearningViewContext::default();
        ctx.artifact_aliases
            .insert("md-a-ch1".into(), "canon-a".into());
        ctx.artifact_aliases
            .insert("md-a-ch2".into(), "canon-a".into());
        ctx.study_marks.insert(
            "mathscape:source-unit/a-def".into(),
            StudyMarkEntry {
                status: "mastered".into(),
                note: None,
            },
        );

        let view = learning_view_with_context(&snap, &ctx);
        assert_eq!(view.materials.len(), 2);
        let book_a = view
            .materials
            .iter()
            .find(|m| m.artifact_id == "canon-a")
            .unwrap();
        let book_b = view
            .materials
            .iter()
            .find(|m| m.artifact_id == "md-b")
            .unwrap();

        // 1. Canonical folding merged ch1 & ch2 into `canon-a`
        assert_eq!(book_a.aliased_artifact_ids, vec!["md-a-ch1", "md-a-ch2"]);
        // 2. Outline units ordered by (page, byte_start), including unaligned section!
        let outline_titles: Vec<&str> = book_a
            .outline_units
            .iter()
            .map(|u| u.unit_title.as_str())
            .collect();
        assert_eq!(outline_titles, vec!["§1 数列基础", "定义 1.1", "推论 2.1"]);
        assert_eq!(book_a.outline_units[0].category, "unaligned");
        assert_eq!(book_a.outline_units[1].study_status, "mastered");
        // 3. Book B sees `fib` as `mastered_elsewhere` because Book A's `a-def` is mastered!
        assert_eq!(book_b.outline_units[0].category, "mastered_elsewhere");
        assert_eq!(
            book_b.outline_units[0].targets[0].mastered_in,
            vec!["canon-a"]
        );
        // 4. Convention baseline comparison
        assert!(book_a.conventions_detail[0].is_baseline_match);
        assert!(!book_b.conventions_detail[0].is_baseline_match);
        assert!(
            book_b.conventions_detail[0]
                .diff_note
                .as_deref()
                .unwrap()
                .contains('0')
        );
    }
}
