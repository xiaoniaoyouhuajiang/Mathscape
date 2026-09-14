use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{EntityKind, StableId};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionGroup,
    ProjectionGroupId, ProjectionLens, ProjectionNode, ProjectionRelation, ViewLayer,
};
use mathscape_store_sqlite::{
    ArtifactCatalogSnapshot, LandscapeSnapshot, SourceMapSnapshot, StoredMapObject,
};
use sha2::Digest;
use thiserror::Error;

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

#[derive(Clone, Copy)]
pub struct LandscapeProjectionInput<'a> {
    catalog: &'a ArtifactCatalogSnapshot,
    source_map: &'a SourceMapSnapshot,
}

impl<'a> LandscapeProjectionInput<'a> {
    pub fn new(catalog: &'a ArtifactCatalogSnapshot, source_map: &'a SourceMapSnapshot) -> Self {
        Self {
            catalog,
            source_map,
        }
    }

    pub fn from_snapshot(snapshot: &'a LandscapeSnapshot) -> Self {
        Self::new(snapshot.artifacts(), snapshot.sources())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LandscapeProjector;

impl LandscapeProjector {
    pub fn project(
        &self,
        input: LandscapeProjectionInput<'_>,
    ) -> Result<GraphProjection, SourceGraphProjectionError> {
        project_landscape(input)
    }
}

pub fn project_source_map(
    snapshot: &SourceMapSnapshot,
    lens: ProjectionLens,
) -> Result<GraphProjection, SourceGraphProjectionError> {
    project_source_map_internal(snapshot, lens, false)
}

fn project_source_map_internal(
    snapshot: &SourceMapSnapshot,
    lens: ProjectionLens,
    include_evidence: bool,
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

    let group_id = ProjectionGroupId::parse("source-map")?;
    let group = ProjectionGroup::new(group_id.clone(), "来源图", ViewLayer::Source, None)?;
    let mut nodes = Vec::new();
    let source_spans = objects
        .values()
        .filter(|object| object.kind == EntityKind::SourceSpan)
        .collect::<Vec<_>>();
    let render_objects = if include_evidence {
        source_units
            .into_iter()
            .chain(source_spans.iter().copied())
            .collect::<Vec<_>>()
    } else {
        source_units.into_iter().collect::<Vec<_>>()
    };
    for object in render_objects {
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
    if include_evidence {
        let span_ids = source_spans
            .iter()
            .map(|object| object.stable_id.clone())
            .collect::<BTreeSet<_>>();
        for unit in objects
            .values()
            .filter(|object| object.kind == EntityKind::SourceSemanticUnit)
        {
            let payload = object_payload(unit);
            let Some(spans) = payload
                .get("source_spans")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for (ordinal, span) in spans.iter().enumerate() {
                let Some(span) = span.as_str().and_then(|value| StableId::parse(value).ok()) else {
                    continue;
                };
                if !span_ids.contains(&span) {
                    continue;
                }
                let relation_id = synthetic_relation_id("contains-span", &unit.stable_id, &span);
                relations.push(
                    ProjectionRelation::new(
                        relation_id,
                        EntityKind::SourceUnitRelation,
                        ViewLayer::Source,
                        "对应原文",
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
                    .summary("来源单元对应的原文范围")?
                    .group(group_id.clone()),
                );
            }
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
        // Relations whose endpoints are only evidence spans belong to the
        // explicit evidence projection, not the semantic-unit source map.
        if endpoints
            .iter()
            .any(|endpoint| !node_ids.contains(&endpoint.entity_id))
        {
            continue;
        }
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
            .group(group_id.clone())
            .lifecycle(object.lifecycle),
        );
    }
    let groups = if nodes.is_empty() {
        vec![]
    } else {
        vec![group]
    };
    GraphProjection::new(lens, groups, nodes, relations).map_err(Into::into)
}

/// Builds the current cross-view landscape. The two canonical projections
/// remain separately queryable; this view only combines their renderer-neutral
/// nodes and relations so provenance can be followed in one neighborhood.
pub fn project_landscape(
    input: LandscapeProjectionInput<'_>,
) -> Result<GraphProjection, SourceGraphProjectionError> {
    let material =
        super::artifact_graph::project_artifact_catalog(input.catalog, ProjectionLens::Source)
            .map_err(|error| SourceGraphProjectionError::Material(error.to_string()))?;
    let source = project_source_map_internal(input.source_map, ProjectionLens::Source, true)?;
    let mut groups = material.groups().to_vec();
    groups.extend(source.groups().iter().map(|group| {
        // Avoid a group identity collision while preserving the layer boundary.
        ProjectionGroup::new(
            ProjectionGroupId::parse("source-map-landscape").expect("valid group ID"),
            group.label(),
            group.layer(),
            None,
        )
        .expect("valid projection group")
    }));
    let source_group = ProjectionGroupId::parse("source-map-landscape")?;
    let mut nodes = material.nodes().to_vec();
    nodes.extend(source.nodes().iter().map(|node| {
        // ProjectionNode's group is private, so rebuild only the stable public
        // display fields through the existing constructor.
        let mut rebuilt = ProjectionNode::new(
            node.entity_id().clone(),
            node.kind(),
            node.layer(),
            node.label(),
        )
        .expect("source node is already validated")
        .group(source_group.clone())
        .boundary(node.is_boundary());
        if let Some(summary) = node.summary_ref() {
            rebuilt = rebuilt.summary(summary).expect("summary is already valid");
        }
        if let Some(lifecycle) = node.lifecycle_status() {
            rebuilt = rebuilt.lifecycle(lifecycle);
        }
        rebuilt
    }));
    let mut relations = material.relations().to_vec();
    relations.extend(source.relations().iter().map(|relation| {
        let endpoints = relation.endpoints().to_vec();
        let mut rebuilt = ProjectionRelation::new(
            relation.relation_id().clone(),
            relation.kind(),
            relation.layer(),
            relation.label(),
            endpoints,
        )
        .expect("source relation is already validated")
        .group(source_group.clone());
        if let Some(summary) = relation.summary_ref() {
            rebuilt = rebuilt.summary(summary).expect("summary is already valid");
        }
        if let Some(lifecycle) = relation.lifecycle_status() {
            rebuilt = rebuilt.lifecycle(lifecycle);
        }
        rebuilt
    }));
    let material_ids = material
        .nodes()
        .iter()
        .map(|node| node.entity_id().clone())
        .collect::<BTreeSet<_>>();
    let source_ids = source
        .nodes()
        .iter()
        .map(|node| node.entity_id().clone())
        .collect::<BTreeSet<_>>();
    for object in input
        .source_map
        .objects
        .iter()
        .filter(|object| object.kind == EntityKind::SourceSpan && !object.relation)
    {
        let payload = object_payload(object);
        let Some(markdown) = payload
            .get("markdown")
            .and_then(|value| value.get("artifact"))
            .and_then(serde_json::Value::as_str)
            .and_then(|value| StableId::parse(value).ok())
        else {
            continue;
        };
        if !material_ids.contains(&markdown) || !source_ids.contains(&object.stable_id) {
            return Err(SourceGraphProjectionError::UnknownEndpoint {
                relation: object.stable_id.clone(),
                entity: markdown,
            });
        }
        let relation_id = synthetic_relation_id("located-in", &object.stable_id, &markdown);
        relations.push(
            ProjectionRelation::new(
                relation_id,
                EntityKind::Evidence,
                ViewLayer::Source,
                "来源于 Markdown",
                vec![
                    ProjectionEndpoint::new(
                        object.stable_id.clone(),
                        mathscape_domain::EndpointRole::parse("span")?,
                        None,
                        IncidenceDirection::IntoRelation,
                    ),
                    ProjectionEndpoint::new(
                        markdown,
                        mathscape_domain::EndpointRole::parse("artifact")?,
                        None,
                        IncidenceDirection::OutOfRelation,
                    ),
                ],
            )?
            .summary("SourceSpan Markdown 定位")?,
        );
    }
    GraphProjection::new(ProjectionLens::Landscape, groups, nodes, relations).map_err(Into::into)
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
    StableId::parse(format!("mathscape:projection-relation/{prefix}/{digest:x}"))
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
    fn projects_source_units_and_semantic_relations_as_an_incidence_graph() {
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
                    payload: json!({"payload":{"display_title":"命题", "source_spans":[span.to_string()]}}),
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
                        entity_id: unit.clone(),
                    },
                ],
            )]),
        };
        let projection = project_source_map(&snapshot, ProjectionLens::Source).unwrap();
        assert_eq!(projection.nodes().len(), 1);
        assert_eq!(projection.relations().len(), 1);
        assert!(
            projection
                .relations()
                .iter()
                .any(|relation| relation.label() == "证明")
        );
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
    #[error("material graph projection failed: {0}")]
    Material(String),
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
