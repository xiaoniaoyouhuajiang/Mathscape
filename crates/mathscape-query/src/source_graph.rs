use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{CanonicalMarkdownView, EntityKind, StableId};
use mathscape_graph::{
    GraphProjection, GraphProjectionError, IncidenceDirection, ProjectionEndpoint, ProjectionGroup,
    ProjectionGroupId, ProjectionLens, ProjectionNode, ProjectionRelation, ProjectionRelationKind,
    ViewLayer,
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
    ink_snapshot_labels: Option<&'a BTreeMap<String, String>>,
    canonical_markdown: &'a [CanonicalMarkdownView],
}

impl<'a> LandscapeProjectionInput<'a> {
    pub fn new(catalog: &'a ArtifactCatalogSnapshot, source_map: &'a SourceMapSnapshot) -> Self {
        Self {
            catalog,
            source_map,
            ink_snapshot_labels: None,
            canonical_markdown: &[],
        }
    }

    /// Registry rows that collapse physical markdown fragments into one node.
    pub fn with_canonical_markdown(mut self, views: &'a [CanonicalMarkdownView]) -> Self {
        self.canonical_markdown = views;
        self
    }

    /// N3d: snapshot Artifact id → snapshot name, shown on the snapshot
    /// material node and in ink evidence summaries.
    pub fn with_ink_snapshot_labels(mut self, labels: &'a BTreeMap<String, String>) -> Self {
        self.ink_snapshot_labels = Some(labels);
        self
    }

    pub fn from_snapshot(snapshot: &'a LandscapeSnapshot) -> Self {
        Self::new(snapshot.artifacts(), snapshot.sources())
            .with_ink_snapshot_labels(snapshot.ink_snapshot_labels())
            .with_canonical_markdown(snapshot.canonical_markdown())
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
                    .relation_kind(ProjectionRelationKind::ContainsSpan)
                    .summary("来源单元对应的原文范围")?
                    .group(group_id.clone()),
                );
            }
        }
    }
    // A unit's place in the document hierarchy lives in its own payload rather
    // than in a stored relation, so the structural edge only exists if the
    // projection synthesizes it. Without this, focusing a section cannot reach
    // the units it contains.
    for unit in objects
        .values()
        .filter(|object| object.kind == EntityKind::SourceSemanticUnit)
    {
        let Some(parent) = object_payload(unit)
            .get("parent_unit")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| StableId::parse(value).ok())
        else {
            continue;
        };
        if parent == unit.stable_id || !node_ids.contains(&parent) {
            continue;
        }
        let relation_id = synthetic_relation_id("has-part", &parent, &unit.stable_id);
        relations.push(
            ProjectionRelation::new(
                relation_id,
                EntityKind::SourceUnitRelation,
                ViewLayer::Source,
                relation_label(ProjectionRelationKind::HasPart),
                vec![
                    ProjectionEndpoint::new(
                        parent,
                        mathscape_domain::EndpointRole::parse("parent")?,
                        None,
                        IncidenceDirection::IntoRelation,
                    ),
                    ProjectionEndpoint::new(
                        unit.stable_id.clone(),
                        mathscape_domain::EndpointRole::parse("child")?,
                        None,
                        IncidenceDirection::OutOfRelation,
                    ),
                ],
            )?
            .relation_kind(ProjectionRelationKind::HasPart)
            .summary("来源单元的结构层级")?
            .group(group_id.clone()),
        );
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
        let relation_kind = relation_kind_of(&object.payload);
        let label = relation_label(relation_kind);
        let summary = format!("来源关系 · {}", label);
        relations.push(
            ProjectionRelation::new(
                object.stable_id.clone(),
                object.kind,
                ViewLayer::Source,
                label,
                projected,
            )?
            .relation_kind(relation_kind)
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
    let material = super::artifact_graph::project_artifact_catalog_labeled(
        input.catalog,
        ProjectionLens::Source,
        input.ink_snapshot_labels,
        input.canonical_markdown,
    )
    .map_err(|error| SourceGraphProjectionError::Material(error.to_string()))?;
    let canonical_aliases =
        super::artifact_graph::canonical_markdown_aliases(input.canonical_markdown);
    // Ink evidence spans → their snapshot's name, if the snapshot is named.
    let evidence_snapshot_names: BTreeMap<StableId, &str> = input
        .ink_snapshot_labels
        .map(|labels| {
            input
                .source_map
                .objects
                .iter()
                .filter(|object| object.kind == EntityKind::SourceSpan)
                .filter_map(|object| {
                    let artifact = object_payload(object)
                        .pointer("/ink/snapshot_artifact")?
                        .as_str()?;
                    Some((object.stable_id.clone(), labels.get(artifact)?.as_str()))
                })
                .collect()
        })
        .unwrap_or_default();
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
            let summary = match evidence_snapshot_names.get(node.entity_id()) {
                Some(name) => {
                    summary.replacen("手稿快照选区", &format!("手稿快照「{name}」选区"), 1)
                }
                None => summary.to_owned(),
            };
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
        .relation_kind(relation.relation_kind_value())
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
        let locator = payload
            .get("markdown")
            .and_then(|value| value.get("artifact"))
            .and_then(serde_json::Value::as_str)
            .map(|id| (id, "来源于 Markdown", "SourceSpan Markdown 定位"))
            .or_else(|| {
                payload
                    .pointer("/ink/snapshot_artifact")
                    .and_then(serde_json::Value::as_str)
                    .map(|id| (id, "来源于手稿快照", "SourceSpan Ink 笔迹定位"))
            });
        let Some((artifact, label, summary)) = locator.and_then(|(id, label, summary)| {
            StableId::parse(id).ok().map(|artifact| {
                let artifact = canonical_aliases
                    .get(&artifact)
                    .cloned()
                    .unwrap_or(artifact);
                (artifact, label, summary)
            })
        }) else {
            continue;
        };
        if !material_ids.contains(&artifact) || !source_ids.contains(&object.stable_id) {
            return Err(SourceGraphProjectionError::UnknownEndpoint {
                relation: object.stable_id.clone(),
                entity: artifact,
            });
        }
        let relation_id = synthetic_relation_id("located-in", &object.stable_id, &artifact);
        relations.push(
            ProjectionRelation::new(
                relation_id,
                EntityKind::Evidence,
                ViewLayer::Source,
                label,
                vec![
                    ProjectionEndpoint::new(
                        object.stable_id.clone(),
                        mathscape_domain::EndpointRole::parse("span")?,
                        None,
                        IncidenceDirection::IntoRelation,
                    ),
                    ProjectionEndpoint::new(
                        artifact,
                        mathscape_domain::EndpointRole::parse("artifact")?,
                        None,
                        IncidenceDirection::OutOfRelation,
                    ),
                ],
            )?
            .relation_kind(ProjectionRelationKind::LocatedIn)
            .summary(summary)?,
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
            EntityKind::SourceSpan if payload.get("ink").is_some() => "手稿笔迹证据".into(),
            EntityKind::SourceSpan => "来源范围".into(),
            EntityKind::SourceSemanticUnit => "来源单元".into(),
            _ => object.stable_id.to_string(),
        });
    let summary = match object.kind {
        EntityKind::SourceSpan => payload
            .get("markdown")
            .and_then(|markdown| markdown.get("byte_range"))
            .map(|range| format!("Markdown 字节范围 {}", range))
            .or_else(|| {
                payload.get("ink").map(|ink| {
                    format!(
                        "手稿快照选区 · {} 笔",
                        ink.get("stroke_ids")
                            .and_then(serde_json::Value::as_array)
                            .map_or(0, Vec::len)
                    )
                })
            }),
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
        assert!(projection.relations().iter().any(|relation| {
            relation.label() == "证明"
                && relation.relation_kind_value() == ProjectionRelationKind::Proves
        }));
    }

    #[test]
    fn synthesizes_a_structural_relation_from_the_units_parent() {
        let span = id("mathscape:source-span/a");
        let section = id("mathscape:source-unit/section");
        let child = id("mathscape:source-unit/child");
        let unit =
            |stable_id: &StableId, revision: char, payload: serde_json::Value| StoredMapObject {
                stable_id: stable_id.clone(),
                revision_id: RevisionId::parse(format!(
                    "sha256:{}",
                    revision.to_string().repeat(64)
                ))
                .unwrap(),
                kind: EntityKind::SourceSemanticUnit,
                relation: false,
                payload,
                lifecycle: mathscape_domain::LifecycleStatus::Active,
            };
        let snapshot = SourceMapSnapshot {
            objects: vec![
                unit(
                    &section,
                    '1',
                    json!({"payload":{"display_title":"第一节","source_spans":[span.to_string()]}}),
                ),
                unit(
                    &child,
                    '2',
                    json!({"payload":{
                        "display_title":"命题 1.1",
                        "source_spans":[span.to_string()],
                        "parent_unit": section.to_string()
                    }}),
                ),
            ],
            relation_endpoints: BTreeMap::new(),
        };

        let projection = project_source_map(&snapshot, ProjectionLens::Source).unwrap();
        let structural = projection
            .relations()
            .iter()
            .find(|relation| relation.relation_kind_value() == ProjectionRelationKind::HasPart)
            .expect("the unit hierarchy must reach the projection");
        assert_eq!(structural.label(), "包含");
        // The parent has to sit on the into-relation side or focus traversal
        // would walk from a child up into its whole section.
        assert!(structural.endpoints().iter().any(|endpoint| {
            endpoint.node() == &section && endpoint.direction() == IncidenceDirection::IntoRelation
        }));
        assert!(structural.endpoints().iter().any(|endpoint| {
            endpoint.node() == &child && endpoint.direction() == IncidenceDirection::OutOfRelation
        }));
    }

    #[test]
    fn unmapped_relation_payloads_fall_back_to_unknown() {
        assert_eq!(
            relation_kind_of(&json!({"payload":{"relation":"has_part"}})),
            ProjectionRelationKind::HasPart
        );
        // Traversal policies must fail closed, so anything the projector does
        // not recognize stays `unknown` instead of borrowing a neighbour's kind.
        assert_eq!(
            relation_kind_of(&json!({"payload":{"relation":"invented_later"}})),
            ProjectionRelationKind::Unknown
        );
        assert_eq!(
            relation_kind_of(&json!({"payload":{}})),
            ProjectionRelationKind::Unknown
        );
    }
}

fn relation_kind_of(payload: &serde_json::Value) -> ProjectionRelationKind {
    let payload = payload.get("payload").unwrap_or(payload);
    payload
        .get("relation")
        .and_then(serde_json::Value::as_str)
        .map_or(ProjectionRelationKind::Unknown, |value| {
            ProjectionRelationKind::from_domain_relation(value)
        })
}

fn relation_label(kind: ProjectionRelationKind) -> String {
    match kind {
        ProjectionRelationKind::States => "陈述",
        ProjectionRelationKind::Proves => "证明",
        ProjectionRelationKind::HasPart => "包含",
        ProjectionRelationKind::Poses => "提出",
        ProjectionRelationKind::Hints => "提示",
        ProjectionRelationKind::Solves => "解答",
        ProjectionRelationKind::Explains => "解释",
        ProjectionRelationKind::AppliesMethod => "应用方法",
        ProjectionRelationKind::Cites => "引用",
        _ => "来源关系",
    }
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
