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

/// 共享对象的白话名：`plain_label`，否则中文首选名，否则第一个首选名。
pub fn entity_label(o: &MapEditObject) -> String {
    let p = payload(o);
    text(p, "plain_label")
        .map(str::to_owned)
        .or_else(|| {
            let labels = p.get("preferred_labels")?.as_array()?;
            labels
                .iter()
                .find(|l| text(l, "lang") == Some("zh"))
                .or_else(|| labels.first())
                .and_then(|l| text(l, "label"))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "共享对象".to_owned())
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
pub struct LearningMaterial {
    pub artifact_id: String,
    pub units: Vec<LearningUnit>,
    /// 本材料约定卡上的约定，标「新约定」。
    pub new_conventions: Vec<LearningConvention>,
    /// 挂在本材料相关对齐线 / 共享对象上的未还清账（open + claimed_paid）。
    pub open_debts: Vec<LearningDebt>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
pub struct LearningView {
    pub materials: Vec<LearningMaterial>,
}

/// 来源单元 → 它的原文所在的材料（取第一个能定位的 span）。
fn unit_materials(snapshot: &MapEditSnapshot) -> BTreeMap<String, String> {
    let span_artifact: BTreeMap<&str, &str> = snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SourceSpan")
        .filter_map(|o| {
            let artifact = payload(o)
                .pointer("/markdown/artifact")
                .or_else(|| payload(o).pointer("/ink/snapshot_artifact"))?
                .as_str()?;
            Some((o.object_id.as_str(), artifact))
        })
        .collect();
    snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "SourceSemanticUnit")
        .filter_map(|o| {
            let artifact = payload(o)
                .get("source_spans")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .find_map(|span| span_artifact.get(span))?;
            Some((o.object_id.clone(), (*artifact).to_owned()))
        })
        .collect()
}

/// 计算学习视图。材料按 id 排序；单元按标题排序。
pub fn learning_view(snapshot: &MapEditSnapshot) -> LearningView {
    let objects: BTreeMap<&str, &MapEditObject> = snapshot
        .objects
        .iter()
        .map(|o| (o.object_id.as_str(), o))
        .collect();
    let unit_material = unit_materials(snapshot);
    let alignments: Vec<&MapEditObject> = snapshot
        .objects
        .iter()
        .filter(|o| o.kind == "Alignment" && live(o))
        .collect();
    // 共享对象 → 对齐到它的材料集合
    let mut entity_materials: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for a in &alignments {
        let p = payload(a);
        if let (Some(source), Some(target)) = (text(p, "source"), text(p, "target"))
            && let Some(material) = unit_material.get(source)
        {
            entity_materials
                .entry(target)
                .or_default()
                .insert(material.as_str());
        }
    }
    let mut by_material: BTreeMap<&str, LearningMaterial> = BTreeMap::new();
    let mut material_holders: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
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
        let only_differs_by = p
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
        by_material
            .entry(material.as_str())
            .or_insert_with(|| LearningMaterial {
                artifact_id: material.clone(),
                units: Vec::new(),
                new_conventions: Vec::new(),
                open_debts: Vec::new(),
            })
            .units
            .push(LearningUnit {
                unit_id: source.to_owned(),
                unit_title: text(payload(unit), "display_title")
                    .unwrap_or("来源单元")
                    .to_owned(),
                alignment_id: a.object_id.clone(),
                entity_id: target.to_owned(),
                entity_label: entity_label(entity),
                relation: text(p, "relation").unwrap_or("").to_owned(),
                label: if seen_in.is_empty() {
                    "新的".to_owned()
                } else {
                    "别的材料里见过".to_owned()
                },
                seen_in,
                only_differs_by,
            });
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
        let entry = by_material
            .entry(material.as_str())
            .or_insert_with(|| LearningMaterial {
                artifact_id: material.clone(),
                units: Vec::new(),
                new_conventions: Vec::new(),
                open_debts: Vec::new(),
            });
        for convention in p
            .get("conventions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(t) = text(convention, "text") {
                entry.new_conventions.push(LearningConvention {
                    scope_id: scope.object_id.clone(),
                    scope_label: text(p, "label").unwrap_or("约定卡").to_owned(),
                    text: t.to_owned(),
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
    }
    LearningView { materials }
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
}

pub fn semantic_object_detail(
    snapshot: &MapEditSnapshot,
    object_id: &str,
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
    let titles = snapshot
        .objects
        .iter()
        .filter(|other| ids.contains(other.object_id.as_str()))
        .map(|other| (other.object_id.clone(), title(other)))
        .collect();
    let debts = snapshot
        .objects
        .iter()
        .filter(|d| d.kind == "Debt" && live(d))
        .filter(|d| text(payload(d), "attached_to") == Some(object_id))
        .map(|d| {
            let mut value = payload(d).clone();
            if let Some(map) = value.as_object_mut() {
                map.insert("object_id".into(), Value::from(d.object_id.clone()));
            }
            value
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
    })
}

pub fn semantic_object_title(o: &MapEditObject) -> String {
    title(o)
}

fn title(o: &MapEditObject) -> String {
    let p = payload(o);
    match o.kind.as_str() {
        "SemanticEntity" => entity_label(o),
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
