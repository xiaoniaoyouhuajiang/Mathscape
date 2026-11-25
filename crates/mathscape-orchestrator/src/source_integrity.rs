//! Current-revision references shared by retirement and registration. No graph/UI heuristics.
use mathscape_domain::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn is_live(o: &MapEditObject) -> bool {
    !matches!(
        o.lifecycle.as_str(),
        "deprecated" | "superseded" | "rejected"
    )
}
pub fn object_title(o: &MapEditObject) -> Option<String> {
    let p = &o.payload["payload"];
    let semantic = match o.kind.as_str() {
        "SemanticEntity" | "RepresentationCore" => {
            // Impact titles are backend prose like the prefixes below: Chinese.
            mathscape_domain::semantic_display_label(p, mathscape_domain::DisplayLang::Zh)
        }
        "Alignment" => Some(
            p["map_label"]
                .as_str()
                .map(|label| format!("对齐线：{label}"))
                .unwrap_or_else(|| "对齐线".into()),
        ),
        "RepresentationBinding" => Some(
            p["map_label"]
                .as_str()
                .map(|label| format!("表示绑定：{label}"))
                .unwrap_or_else(|| "表示绑定".into()),
        ),
        "Debt" => p["obligation"]
            .as_str()
            .map(|text| format!("一笔账：{text}")),
        "DialectScope" => p["label"].as_str().map(|label| format!("约定卡：{label}")),
        _ => None,
    };
    if semantic.is_some() {
        return semantic;
    }
    o.payload["payload"]["display_title"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| (o.kind == "SourceSpan").then(|| "原文证据".into()))
        .or_else(|| {
            (o.kind == "SourceUnitRelation").then(|| {
                let relation = o.payload["payload"]["relation"]
                    .as_str()
                    .unwrap_or("来源关系");
                format!(
                    "{}关系",
                    match relation {
                        "proves" => "证明",
                        "solves" => "解答",
                        "cites" => "引用",
                        "explains" => "解释",
                        "states" => "陈述",
                        "hints" => "提示",
                        "poses" => "提出",
                        "applies_method" => "应用方法",
                        _ => relation,
                    }
                )
            })
        })
}
fn markdown_artifact(o: &MapEditObject) -> Option<&str> {
    o.payload["payload"]["surface_content_artifact"]
        .as_str()
        .or_else(|| o.payload["payload"]["markdown"]["artifact"].as_str())
}
fn context<'a>(o: &'a MapEditObject, map: &'a BTreeMap<String, MapEditObject>) -> Option<&'a str> {
    markdown_artifact(o).or_else(|| {
        o.payload["payload"]["source_spans"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find_map(|s| map.get(s).and_then(markdown_artifact))
    })
}
pub fn source_references(o: &MapEditObject) -> Result<Vec<(String, String)>, String> {
    let p = &o.payload["payload"];
    let mut out = Vec::new();
    let mut add = |v: &Value, role: &str| {
        if let Some(id) = v.as_str() {
            out.push((id.to_owned(), role.to_owned()));
        }
    };
    match o.kind.as_str() {
        "SourceSemanticUnit" => {
            add(&p["parent_unit"], "parent");
            for v in p["source_spans"].as_array().into_iter().flatten() {
                add(v, "source_span");
            }
            add(&p["source_title"]["source_span"], "title_span");
            for v in p["source_labels"].as_array().into_iter().flatten() {
                add(&v["source_span"], "label_span");
            }
            for r in p["local_references"].as_array().into_iter().flatten() {
                add(&r["source_span"], "reference_span");
                for v in r["candidate_targets"].as_array().into_iter().flatten() {
                    add(v, "reference_target");
                }
            }
        }
        "SourceSpan" => {}
        "SourceUnitRelation" => {
            for e in &o.endpoints {
                out.push((
                    e.entity().to_string(),
                    format!("endpoint:{}", e.role().as_str()),
                ));
            }
        }
        kind if crate::semantic_plan::is_semantic_kind(kind) => {
            out.extend(crate::semantic_plan::references(o));
        }
        _ => return Err(format!("unsupported_reference_kind: {}", o.kind)),
    }
    for v in p["trust"]["evidence"].as_array().into_iter().flatten() {
        if let Some(id) = v.as_str() {
            out.push((id.into(), "trust_evidence".into()));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
pub fn related_object(o: &MapEditObject, role: &str) -> ImpactObjectRef {
    ImpactObjectRef {
        object_id: o.object_id.clone(),
        revision_id: o.revision.clone(),
        kind: o.kind.clone(),
        display_title: object_title(o),
        reference_kind: role.into(),
    }
}
/// A handwritten-manuscript span (ink evidence) is located physically by its
/// immutable ink snapshot artifact and the selected stroke ids. It is a valid
/// locator for integrity checks only; it never enters a source payload.
pub fn has_ink_locator(span_payload: &Value) -> bool {
    let ink = &span_payload["ink"];
    ink["snapshot_artifact"]
        .as_str()
        .is_some_and(|id| !id.trim().is_empty())
        && ink["stroke_ids"].as_array().is_some_and(|ids| {
            !ids.is_empty()
                && ids
                    .iter()
                    .all(|id| id.as_str().is_some_and(|id| !id.trim().is_empty()))
        })
}
pub fn validate_live_references(map: &BTreeMap<String, MapEditObject>) -> Vec<ImpactDiagnostic> {
    let mut errors = vec![];
    for o in map.values().filter(|o| is_live(o)) {
        match source_references(o) {
            Err(message) => errors.push(ImpactDiagnostic {
                code: "unsupported_reference_kind".into(),
                object_id: Some(o.object_id.clone()),
                message,
            }),
            Ok(refs) => {
                for (id, role) in refs {
                    let semantic = crate::semantic_plan::expected_kinds(&role);
                    if !semantic.is_empty() || crate::semantic_plan::is_semantic_kind(&o.kind) {
                        match map.get(&id).filter(|t| is_live(t)) {
                            None => errors.push(ImpactDiagnostic {
                                code: "inactive_or_invalid_reference".into(),
                                object_id: Some(o.object_id.clone()),
                                message: format!(
                                    "{role} 指向不存在或已退役的对象 {id}；需先处理依赖者"
                                ),
                            }),
                            Some(t)
                                if !semantic.is_empty() && !semantic.contains(&t.kind.as_str()) =>
                            {
                                errors.push(ImpactDiagnostic {
                                    code: "invalid_reference_kind".into(),
                                    object_id: Some(o.object_id.clone()),
                                    message: format!(
                                        "{role} 必须指向 {}，实际是 {}",
                                        semantic.join(" / "),
                                        t.kind
                                    ),
                                })
                            }
                            Some(_) => {}
                        }
                        continue;
                    }
                    let expected = match role.as_str() {
                        "parent" | "reference_target" | "endpoint:source" | "endpoint:target" => {
                            Some("SourceSemanticUnit")
                        }
                        "source_span" | "title_span" | "label_span" | "reference_span"
                        | "endpoint:evidence" => Some("SourceSpan"),
                        _ => None,
                    };
                    if !map
                        .get(&id)
                        .is_some_and(|t| is_live(t) && expected.is_none_or(|k| t.kind == k))
                    {
                        errors.push(ImpactDiagnostic {
                            code: "inactive_or_invalid_reference".into(),
                            object_id: Some(o.object_id.clone()),
                            message: format!("{role} 依赖不可用对象 {id}；需先处理依赖者"),
                        });
                    }
                }
            }
        }
        if o.kind == "SourceSemanticUnit" {
            let p = &o.payload["payload"];
            let spans = p["source_spans"].as_array();
            if spans.is_none_or(|v| v.is_empty()) {
                errors.push(ImpactDiagnostic {
                    code: "invalid_evidence".into(),
                    object_id: Some(o.object_id.clone()),
                    message: "来源单元必须保留非空的原文证据".into(),
                });
            }
            if let Some(expected) = context(o, map) {
                if spans
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(|id| map.get(id))
                    .filter_map(markdown_artifact)
                    .any(|id| id != expected)
                {
                    errors.push(ImpactDiagnostic {
                        code: "mixed_source_context".into(),
                        object_id: Some(o.object_id.clone()),
                        message: "原文证据必须属于同一 Markdown 来源".into(),
                    });
                }
                if p["parent_unit"]
                    .as_str()
                    .and_then(|id| map.get(id))
                    .is_some_and(|parent| context(parent, map) != Some(expected))
                {
                    errors.push(ImpactDiagnostic {
                        code: "cross_source_parent".into(),
                        object_id: Some(o.object_id.clone()),
                        message: "父单元必须属于同一 Markdown 来源".into(),
                    });
                }
            }
            let mut seen = std::collections::BTreeSet::from([o.object_id.as_str()]);
            let mut p = o.payload["payload"]["parent_unit"].as_str();
            while let Some(id) = p {
                if !seen.insert(id) {
                    errors.push(ImpactDiagnostic {
                        code: "parent_cycle".into(),
                        object_id: Some(o.object_id.clone()),
                        message: "父单元链成环".into(),
                    });
                    break;
                }
                p = map
                    .get(id)
                    .and_then(|o| o.payload["payload"]["parent_unit"].as_str());
            }
        } else if o.kind == "SourceSpan" {
            let p = &o.payload["payload"];
            if p["markdown"].is_null() && p["page"].is_null() && !has_ink_locator(p) {
                errors.push(ImpactDiagnostic {
                    code: "invalid_locator".into(),
                    object_id: Some(o.object_id.clone()),
                    message: "原文证据缺少物理定位".into(),
                });
            }
        } else if o.kind == "SourceUnitRelation"
            && (o
                .endpoints
                .iter()
                .filter(|e| e.role().as_str() == "source")
                .count()
                != 1
                || o.endpoints
                    .iter()
                    .filter(|e| e.role().as_str() == "target")
                    .count()
                    != 1
                || !o.endpoints.iter().any(|e| e.role().as_str() == "evidence"))
        {
            errors.push(ImpactDiagnostic {
                code: "invalid_relation".into(),
                object_id: Some(o.object_id.clone()),
                message: "来源关系需要来源、目标和原文证据端点".into(),
            });
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn span(id: &str, payload: Value) -> MapEditObject {
        MapEditObject {
            object_id: id.into(),
            revision: format!("sha256:{}", "1".repeat(64)),
            kind: "SourceSpan".into(),
            lifecycle: "active".into(),
            payload: json!({ "payload_kind": "span", "payload": payload }),
            endpoints: vec![],
        }
    }

    /// Source registration and its withdrawal validate the whole live graph
    /// with this function; an ink evidence span must not fail it.
    #[test]
    fn ink_spans_have_a_physical_locator() {
        let ink = json!({
            "markdown": null, "page": null,
            "ink": { "snapshot_artifact": "mathscape:artifact/sha256/aa", "page_id": "p", "stroke_ids": ["s"] }
        });
        assert!(has_ink_locator(&ink));
        let map = BTreeMap::from([("ink".to_owned(), span("ink", ink))]);
        assert!(validate_live_references(&map).is_empty());
        for broken in [
            json!({ "markdown": null, "page": null }),
            json!({ "markdown": null, "page": null, "ink": { "snapshot_artifact": "a", "stroke_ids": [] } }),
            json!({ "markdown": null, "page": null, "ink": { "snapshot_artifact": "", "stroke_ids": ["s"] } }),
        ] {
            assert!(!has_ink_locator(&broken));
            let map = BTreeMap::from([("x".to_owned(), span("x", broken))]);
            let errors = validate_live_references(&map);
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].code, "invalid_locator");
            assert_eq!(errors[0].object_id.as_deref(), Some("x"));
        }
    }
}
