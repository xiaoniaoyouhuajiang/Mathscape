use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{EntityKind, StableId};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionGroup,
    ProjectionGroupId, ProjectionLens, ProjectionNode, ProjectionRelation, ViewLayer,
};
use mathscape_store_sqlite::{SourceMapSnapshot, StoredMapObject};
use thiserror::Error;
use sha2::Digest;

#[derive(Clone, Copy, Debug, Default)]
pub struct SourceGraphProjector;

impl SourceGraphProjector {
    pub fn project(
        &self,
        snapshot: &SourceMapSnapshot,
        lens: ProjectionLens,
    ) -> Result<GraphProjection, SourceGraphProjectionError> {
        project_source_map(snapshot, lens)
    }
}

pub fn project_source_map(
    snapshot: &SourceMapSnapshot,
    lens: ProjectionLens,
) -> Result<GraphProjection, SourceGraphProjectionError> {
    if lens != ProjectionLens::Source {
        return Err(SourceGraphProjectionError::UnsupportedLens(lens));
    }

    let mut objects = BTreeMap::new();
    for object in &snapshot.objects {
        if matches!(
            object.kind,
            EntityKind::SourceSpan
                | EntityKind::SourceSemanticUnit
                | EntityKind::SourceUnitRelation
        ) {
            objects.insert(object.stable_id.clone(), object);
        }
    }
    let source_units = objects
        .values()
        .filter(|object| object.kind == EntityKind::SourceSemanticUnit)
        .collect::<Vec<_>>();
    let source_spans = objects
        .values()
        .filter(|object| object.kind == EntityKind::SourceSpan)
        .collect::<Vec<_>>();

    let group_id = ProjectionGroupId::parse("source-map")?;
    let group = ProjectionGroup::new(group_id.clone(), "来源图", ViewLayer::Source, None)?;
    let mut nodes = Vec::new();
    for object in source_units.into_iter().chain(source_spans) {
        let (label, summary, boundary) = object_label(object);
        let mut node = ProjectionNode::new(
            object.stable_id.clone(),
            object.kind,
            ViewLayer::Source,
            label,
        )?
        .group(group_id.clone())
        .boundary(boundary);
        if let Some(summary) = summary {
            node = node.summary(summary)?;
        }
        nodes.push(node.lifecycle(object.lifecycle));
    }
    let node_ids = nodes
        .iter()
        .map(|node| node.entity_id().clone())
        .collect::<BTreeSet<_>>();
    let mut relations = Vec::new();
    // `source_spans` is a canonical containment field on SourceSemanticUnit.
    // Project it as an explicit structural edge so a span is never rendered
    // as an unrelated neighbour of the unit that owns it.
    let span_ids = source_spans
        .iter()
        .map(|object| object.stable_id.clone())
        .collect::<BTreeSet<_>>();
    for unit in objects
        .values()
        .filter(|object| object.kind == EntityKind::SourceSemanticUnit)
    {
        let payload = object_payload(unit);
        let Some(spans) = payload.get("source_spans").and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for (ordinal, span) in spans.iter().enumerate() {
            let Some(span) = span.as_str().and_then(|value| StableId::parse(value).ok()) else {
                continue;
            };
            if !span_ids.contains(&span) {
                return Err(SourceGraphProjectionError::UnknownEndpoint {
                    relation: unit.stable_id.clone(),
                    entity: span,
                });
            }
            let relation_id = synthetic_relation_id("contains-span", &unit.stable_id, &span);
            relations.push(
                ProjectionRelation::new(
                    relation_id,
                    EntityKind::SourceUnitRelation,
                    ViewLayer::Source,
                    "包含原文范围",
                    vec![
                        ProjectionEndpoint::new(
                            unit.stable_id.clone(),
                            mathscape_domain::EndpointRole::parse("unit")?,
                            None,
                            IncidenceDirection::IntoRelation,
                        ),
                        ProjectionEndpoint::new(
                            span,
                            mathscape_domain::EndpointRole::parse("span")?,
                            Some(u32::try_from(ordinal).unwrap_or(u32::MAX)),
                            IncidenceDirection::OutOfRelation,
                        ),
                    ],
                )?
                .summary("来源单元的 source_spans 字段")?,
            );
        }
    }
    for object in objects
        .values()
        .filter(|object| object.relation && object.kind == EntityKind::SourceUnitRelation)
    {
        let endpoints = snapshot
            .relation_endpoints
            .get(&object.stable_id)
            .ok_or_else(|| {
                SourceGraphProjectionError::MissingEndpoints(object.stable_id.clone())
            })?;
        let projected = endpoints
            .iter()
            .map(|endpoint| {
                if !node_ids.contains(&endpoint.entity_id) {
                    return Err(SourceGraphProjectionError::UnknownEndpoint {
                        relation: object.stable_id.clone(),
                        entity: endpoint.entity_id.clone(),
                    });
                }
                Ok(ProjectionEndpoint::new(
                    endpoint.entity_id.clone(),
                    mathscape_domain::EndpointRole::parse(endpoint.role.clone())?,
                    endpoint.ordinal,
                    direction(&endpoint.role),
                ))
            })
            .collect::<Result<Vec<_>, SourceGraphProjectionError>>()?;
        let label = relation_label(&object.payload);
        let summary = format!("来源关系 · {}", label);
        relations.push(
            ProjectionRelation::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::Source,
                label,
                projected,
            )?
            .summary(summary)?
            .lifecycle(object.lifecycle),
        );
    }
    GraphProjection::new(lens, vec![group], nodes, relations).map_err(Into::into)
}

fn direction(role: &str) -> IncidenceDirection {
    match role {
        "source" => IncidenceDirection::IntoRelation,
        "target" => IncidenceDirection::OutOfRelation,
        _ => IncidenceDirection::Undirected,
    }
}

fn object_label(object: &StoredMapObject) -> (String, Option<String>, bool) {
    let payload = object_payload(object);
    let label = payload
        .get("display_title")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            payload
                .get("local_title")
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| match object.kind {
            EntityKind::SourceSpan => "来源范围".into(),
            EntityKind::SourceSemanticUnit => "来源单元".into(),
            _ => object.stable_id.to_string(),
        });
    let summary = match object.kind {
        EntityKind::SourceSpan => payload
            .get("markdown")
            .and_then(|markdown| markdown.get("byte_range"))
            .map(|range| format!("Markdown 字节范围 {}", range)),
        EntityKind::SourceSemanticUnit => Some(
            payload
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("source_unit")
                .to_owned(),
        ),
        _ => None,
    };
    let boundary = object.kind == EntityKind::SourceSpan;
    (label, summary, boundary)
}

fn object_payload(object: &StoredMapObject) -> &serde_json::Value {
    object.payload.get("payload").unwrap_or(&object.payload)
}

fn synthetic_relation_id(prefix: &str, source: &StableId, target: &StableId) -> StableId {
    let digest = sha2::Sha256::digest(format!("{prefix}:{source}:{target}").as_bytes());
    StableId::parse(format!(
        "mathscape:projection-relation/{prefix}/{digest:x}"
    ))
    .expect("synthetic projection relation IDs are valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathscape_domain::RevisionId;
    use mathscape_store_sqlite::{SourceMapSnapshot, StoredMapEndpoint, StoredMapObject};
    use serde_json::json;

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    #[test]
    fn projects_source_units_spans_and_relations_as_an_incidence_graph() {
        let span = id("mathscape:source-span/a");
        let unit = id("mathscape:source-unit/a");
        let relation = id("mathscape:source-relation/a");
        let snapshot = SourceMapSnapshot {
            objects: vec![
                StoredMapObject {
                    stable_id: span.clone(),
                    revision_id: RevisionId::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                    kind: EntityKind::SourceSpan,
                    relation: false,
                    payload: json!({"payload":{"markdown":{"byte_range":{"start":0,"end":4}}}}),
                    lifecycle: mathscape_domain::LifecycleStatus::Active,
                },
                StoredMapObject {
                    stable_id: unit.clone(),
                    revision_id: RevisionId::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                    kind: EntityKind::SourceSemanticUnit,
                    relation: false,
                    payload: json!({"payload":{"display_title":"命题"}}),
                    lifecycle: mathscape_domain::LifecycleStatus::Active,
                },
                StoredMapObject {
                    stable_id: relation.clone(),
                    revision_id: RevisionId::parse(format!("sha256:{}", "3".repeat(64))).unwrap(),
                    kind: EntityKind::SourceUnitRelation,
                    relation: true,
                    payload: json!({"payload":{"relation":"proves"}}),
                    lifecycle: mathscape_domain::LifecycleStatus::Active,
                },
            ],
            relation_endpoints: BTreeMap::from([(
                relation.clone(),
                vec![
                    StoredMapEndpoint {
                        role: "source".into(),
                        ordinal: None,
                        entity_id: unit.clone(),
                    },
                    StoredMapEndpoint {
                        role: "target".into(),
                        ordinal: None,
                        entity_id: span.clone(),
                    },
                ],
            )]),
        };
        let projection = project_source_map(&snapshot, ProjectionLens::Source).unwrap();
        assert_eq!(projection.nodes().len(), 2);
        assert_eq!(projection.relations().len(), 1);
        assert_eq!(projection.relations()[0].endpoints().len(), 2);
    }
}

fn relation_label(payload: &serde_json::Value) -> String {
    let payload = payload.get("payload").unwrap_or(payload);
    payload
        .get("relation")
        .and_then(serde_json::Value::as_str)
        .map(|value| match value {
            "proves" => "证明",
            "solves" => "解答",
            "hints" => "提示",
            "explains" => "解释",
            "cites" => "引用",
            "applies_method" => "应用方法",
            other => other,
        })
        .unwrap_or("来源关系")
        .to_owned()
}

#[derive(Debug, Error)]
pub enum SourceGraphProjectionError {
    #[error("source map cannot be projected through the {0:?} lens")]
    UnsupportedLens(ProjectionLens),
    #[error("source relation {0} has no current endpoints")]
    MissingEndpoints(StableId),
    #[error("source relation {relation} points to unknown object {entity}")]
    UnknownEndpoint {
        relation: StableId,
        entity: StableId,
    },
    #[error(transparent)]
    Domain(#[from] mathscape_domain::DomainError),
    #[error(transparent)]
    Graph(#[from] GraphProjectionError),
}
