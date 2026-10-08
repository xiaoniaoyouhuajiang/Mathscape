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
            if p["markdown"].is_null() && p["page"].is_null() {
                errors.push(ImpactDiagnostic {
                    code: "invalid_locator".into(),
                    object_id: Some(o.object_id.clone()),
                    message: "原文证据缺少物理定位".into(),
                });
            }
        } else if o.kind == "SourceUnitRelation" {
            if o.endpoints
                .iter()
                .filter(|e| e.role().as_str() == "source")
                .count()
                != 1
                || o.endpoints
                    .iter()
                    .filter(|e| e.role().as_str() == "target")
                    .count()
                    != 1
                || !o.endpoints.iter().any(|e| e.role().as_str() == "evidence")
            {
                errors.push(ImpactDiagnostic {
                    code: "invalid_relation".into(),
                    object_id: Some(o.object_id.clone()),
                    message: "来源关系需要来源、目标和原文证据端点".into(),
                });
            }
        }
    }
    errors
}
