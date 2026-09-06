use std::collections::{HashMap, HashSet};

use mathscape_domain::{EndpointRole, EntityKind, LifecycleStatus, StableId};
use petgraph::{Directed, Graph, algo::is_cyclic_directed};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const GRAPH_PROJECTION_SCHEMA_VERSION: &str = "mathscape.graph-projection.v1";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProjectionLens {
    Source,
    LocalSemantic,
    GlobalSemantic,
    Alignment,
    Rdtx,
    Formalization,
    Provenance,
    Landscape,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ViewLayer {
    Source,
    LocalSemantic,
    GlobalSemantic,
    Alignment,
    Rdtx,
    Formalization,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidenceDirection {
    IntoRelation,
    OutOfRelation,
    Undirected,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectionGroupId(String);

impl ProjectionGroupId {
    pub fn parse(value: impl Into<String>) -> Result<Self, GraphProjectionError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(GraphProjectionError::InvalidGroupId(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProjectionGroupId {
    type Error = GraphProjectionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ProjectionGroupId> for String {
    fn from(value: ProjectionGroupId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionGroup {
    id: ProjectionGroupId,
    label: String,
    layer: ViewLayer,
    parent: Option<ProjectionGroupId>,
}

impl ProjectionGroup {
    pub fn new(
        id: ProjectionGroupId,
        label: impl Into<String>,
        layer: ViewLayer,
        parent: Option<ProjectionGroupId>,
    ) -> Result<Self, GraphProjectionError> {
        Ok(Self {
            id,
            label: non_empty(label, "group.label")?,
            layer,
            parent,
        })
    }

    pub fn id(&self) -> &ProjectionGroupId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn layer(&self) -> ViewLayer {
        self.layer
    }

    pub fn parent(&self) -> Option<&ProjectionGroupId> {
        self.parent.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionNode {
    entity_id: StableId,
    kind: EntityKind,
    layer: ViewLayer,
    label: String,
    summary: Option<String>,
    lifecycle: Option<LifecycleStatus>,
    group: Option<ProjectionGroupId>,
    boundary: bool,
}

impl ProjectionNode {
    pub fn new(
        entity_id: StableId,
        kind: EntityKind,
        layer: ViewLayer,
        label: impl Into<String>,
    ) -> Result<Self, GraphProjectionError> {
        Ok(Self {
            entity_id,
            kind,
            layer,
            label: non_empty(label, "node.label")?,
            summary: None,
            lifecycle: None,
            group: None,
            boundary: false,
        })
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Result<Self, GraphProjectionError> {
        self.summary = Some(non_empty(summary, "node.summary")?);
        Ok(self)
    }

    pub const fn lifecycle(mut self, lifecycle: LifecycleStatus) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn group(mut self, group: ProjectionGroupId) -> Self {
        self.group = Some(group);
        self
    }

    pub const fn boundary(mut self, boundary: bool) -> Self {
        self.boundary = boundary;
        self
    }

    pub fn entity_id(&self) -> &StableId {
        &self.entity_id
    }

    pub const fn kind(&self) -> EntityKind {
        self.kind
    }

    pub const fn layer(&self) -> ViewLayer {
        self.layer
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn summary_ref(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub const fn lifecycle_status(&self) -> Option<LifecycleStatus> {
        self.lifecycle
    }

    pub fn group_ref(&self) -> Option<&ProjectionGroupId> {
        self.group.as_ref()
    }

    pub const fn is_boundary(&self) -> bool {
        self.boundary
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionEndpoint {
    node: StableId,
    role: EndpointRole,
    ordinal: Option<u32>,
    direction: IncidenceDirection,
}

impl ProjectionEndpoint {
    pub fn new(
        node: StableId,
        role: EndpointRole,
        ordinal: Option<u32>,
        direction: IncidenceDirection,
    ) -> Self {
        Self {
            node,
            role,
            ordinal,
            direction,
        }
    }

    pub fn node(&self) -> &StableId {
        &self.node
    }

    pub fn role(&self) -> &EndpointRole {
        &self.role
    }

    pub const fn ordinal(&self) -> Option<u32> {
        self.ordinal
    }

    pub const fn direction(&self) -> IncidenceDirection {
        self.direction
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionRelation {
    relation_id: StableId,
    kind: EntityKind,
    layer: ViewLayer,
    label: String,
    summary: Option<String>,
    lifecycle: Option<LifecycleStatus>,
    group: Option<ProjectionGroupId>,
    boundary: bool,
    endpoints: Vec<ProjectionEndpoint>,
}

impl ProjectionRelation {
    pub fn new(
        relation_id: StableId,
        kind: EntityKind,
        layer: ViewLayer,
        label: impl Into<String>,
        mut endpoints: Vec<ProjectionEndpoint>,
    ) -> Result<Self, GraphProjectionError> {
        if endpoints.is_empty() {
            return Err(GraphProjectionError::RelationWithoutEndpoints(relation_id));
        }
        validate_endpoint_slots(&relation_id, &endpoints)?;
        endpoints.sort_unstable_by(|left, right| {
            left.role()
                .cmp(right.role())
                .then_with(|| left.ordinal().cmp(&right.ordinal()))
                .then_with(|| left.node().cmp(right.node()))
        });
        Ok(Self {
            relation_id,
            kind,
            layer,
            label: non_empty(label, "relation.label")?,
            summary: None,
            lifecycle: None,
            group: None,
            boundary: false,
            endpoints,
        })
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Result<Self, GraphProjectionError> {
        self.summary = Some(non_empty(summary, "relation.summary")?);
        Ok(self)
    }

    pub const fn lifecycle(mut self, lifecycle: LifecycleStatus) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn group(mut self, group: ProjectionGroupId) -> Self {
        self.group = Some(group);
        self
    }

    pub const fn boundary(mut self, boundary: bool) -> Self {
        self.boundary = boundary;
        self
    }

    pub fn relation_id(&self) -> &StableId {
        &self.relation_id
    }

    pub const fn kind(&self) -> EntityKind {
        self.kind
    }

    pub const fn layer(&self) -> ViewLayer {
        self.layer
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn summary_ref(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub const fn lifecycle_status(&self) -> Option<LifecycleStatus> {
        self.lifecycle
    }

    pub fn group_ref(&self) -> Option<&ProjectionGroupId> {
        self.group.as_ref()
    }

    pub const fn is_boundary(&self) -> bool {
        self.boundary
    }

    pub fn endpoints(&self) -> &[ProjectionEndpoint] {
        &self.endpoints
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphProjection {
    schema_version: String,
    lens: ProjectionLens,
    groups: Vec<ProjectionGroup>,
    nodes: Vec<ProjectionNode>,
    relations: Vec<ProjectionRelation>,
}

impl GraphProjection {
    pub fn new(
        lens: ProjectionLens,
        mut groups: Vec<ProjectionGroup>,
        mut nodes: Vec<ProjectionNode>,
        mut relations: Vec<ProjectionRelation>,
    ) -> Result<Self, GraphProjectionError> {
        validate_groups(&groups)?;
        let group_ids = groups
            .iter()
            .map(|group| group.id().clone())
            .collect::<HashSet<_>>();
        let mut entity_ids = HashSet::new();
        for node in &nodes {
            if !entity_ids.insert(node.entity_id().clone()) {
                return Err(GraphProjectionError::DuplicateEntity(
                    node.entity_id().clone(),
                ));
            }
            validate_group_reference(node.group_ref(), &group_ids)?;
        }

        let mut relation_ids = HashSet::new();
        for relation in &relations {
            if entity_ids.contains(relation.relation_id()) {
                return Err(GraphProjectionError::IdentityUsedAsEntityAndRelation(
                    relation.relation_id().clone(),
                ));
            }
            if !relation_ids.insert(relation.relation_id().clone()) {
                return Err(GraphProjectionError::DuplicateRelation(
                    relation.relation_id().clone(),
                ));
            }
            validate_group_reference(relation.group_ref(), &group_ids)?;
            for endpoint in relation.endpoints() {
                if !entity_ids.contains(endpoint.node()) {
                    return Err(GraphProjectionError::UnknownEndpoint {
                        relation: relation.relation_id().clone(),
                        entity: endpoint.node().clone(),
                    });
                }
            }
        }

        groups.sort_unstable_by(|left, right| left.id().cmp(right.id()));
        nodes.sort_unstable_by(|left, right| left.entity_id().cmp(right.entity_id()));
        relations.sort_unstable_by(|left, right| left.relation_id().cmp(right.relation_id()));
        Ok(Self {
            schema_version: GRAPH_PROJECTION_SCHEMA_VERSION.to_owned(),
            lens,
            groups,
            nodes,
            relations,
        })
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn lens(&self) -> ProjectionLens {
        self.lens
    }

    pub fn groups(&self) -> &[ProjectionGroup] {
        &self.groups
    }

    pub fn nodes(&self) -> &[ProjectionNode] {
        &self.nodes
    }

    pub fn relations(&self) -> &[ProjectionRelation] {
        &self.relations
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum GraphProjectionError {
    #[error("{field} cannot be empty")]
    EmptyText { field: &'static str },
    #[error("projection group ID is invalid: {0:?}")]
    InvalidGroupId(String),
    #[error("projection contains duplicate group {0:?}")]
    DuplicateGroup(ProjectionGroupId),
    #[error("projection group {child:?} refers to unknown parent {parent:?}")]
    UnknownParentGroup {
        child: ProjectionGroupId,
        parent: ProjectionGroupId,
    },
    #[error("projection group hierarchy contains a cycle")]
    GroupCycle,
    #[error("projection element refers to unknown group {0:?}")]
    UnknownGroup(ProjectionGroupId),
    #[error("projection contains duplicate entity {0}")]
    DuplicateEntity(StableId),
    #[error("projection contains duplicate relation {0}")]
    DuplicateRelation(StableId),
    #[error("stable ID {0} is used by both an entity and relation")]
    IdentityUsedAsEntityAndRelation(StableId),
    #[error("projection relation {0} has no endpoints")]
    RelationWithoutEndpoints(StableId),
    #[error("projection relation {relation} refers to missing entity {entity}")]
    UnknownEndpoint {
        relation: StableId,
        entity: StableId,
    },
    #[error("relation {relation} repeats role {role:?} without explicit ordinals")]
    MissingEndpointOrdinal { relation: StableId, role: String },
    #[error("relation {relation} repeats endpoint slot {role:?}[{ordinal:?}]")]
    DuplicateEndpointSlot {
        relation: StableId,
        role: String,
        ordinal: Option<u32>,
    },
}

fn non_empty(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, GraphProjectionError> {
    let value = value.into();
    if value.trim().is_empty() {
        return Err(GraphProjectionError::EmptyText { field });
    }
    Ok(value)
}

fn validate_group_reference(
    group: Option<&ProjectionGroupId>,
    group_ids: &HashSet<ProjectionGroupId>,
) -> Result<(), GraphProjectionError> {
    if let Some(group) = group
        && !group_ids.contains(group)
    {
        return Err(GraphProjectionError::UnknownGroup(group.clone()));
    }
    Ok(())
}

fn validate_groups(groups: &[ProjectionGroup]) -> Result<(), GraphProjectionError> {
    let mut indices = HashMap::new();
    let mut graph = Graph::<(), (), Directed>::new();
    for group in groups {
        if indices.contains_key(group.id()) {
            return Err(GraphProjectionError::DuplicateGroup(group.id().clone()));
        }
        indices.insert(group.id().clone(), graph.add_node(()));
    }
    for group in groups {
        if let Some(parent) = group.parent() {
            let Some(parent_index) = indices.get(parent) else {
                return Err(GraphProjectionError::UnknownParentGroup {
                    child: group.id().clone(),
                    parent: parent.clone(),
                });
            };
            graph.add_edge(indices[group.id()], *parent_index, ());
        }
    }
    if is_cyclic_directed(&graph) {
        return Err(GraphProjectionError::GroupCycle);
    }
    Ok(())
}

fn validate_endpoint_slots(
    relation: &StableId,
    endpoints: &[ProjectionEndpoint],
) -> Result<(), GraphProjectionError> {
    let mut endpoints_by_role: HashMap<&str, Vec<Option<u32>>> = HashMap::new();
    for endpoint in endpoints {
        endpoints_by_role
            .entry(endpoint.role().as_str())
            .or_default()
            .push(endpoint.ordinal());
    }
    for (role, ordinals) in endpoints_by_role {
        if ordinals.len() > 1 && ordinals.iter().any(Option::is_none) {
            return Err(GraphProjectionError::MissingEndpointOrdinal {
                relation: relation.clone(),
                role: role.to_owned(),
            });
        }
        let mut occupied = HashSet::new();
        for ordinal in ordinals {
            if !occupied.insert(ordinal) {
                return Err(GraphProjectionError::DuplicateEndpointSlot {
                    relation: relation.clone(),
                    role: role.to_owned(),
                    ordinal,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mathscape_domain::{EndpointRole, EntityKind, StableId};

    use super::*;

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    fn node(value: &str) -> ProjectionNode {
        ProjectionNode::new(
            id(value),
            EntityKind::SemanticEntity,
            ViewLayer::GlobalSemantic,
            value,
        )
        .unwrap()
    }

    #[test]
    fn rejects_missing_endpoints_and_ambiguous_repeated_roles() {
        let missing = ProjectionRelation::new(
            id("mathscape:inference/missing"),
            EntityKind::Inference,
            ViewLayer::GlobalSemantic,
            "missing",
            vec![],
        )
        .unwrap_err();
        assert!(matches!(
            missing,
            GraphProjectionError::RelationWithoutEndpoints(_)
        ));

        let repeated = ProjectionRelation::new(
            id("mathscape:inference/repeated"),
            EntityKind::Inference,
            ViewLayer::GlobalSemantic,
            "repeated",
            vec![
                ProjectionEndpoint::new(
                    id("mathscape:semantic/a"),
                    EndpointRole::parse("premise").unwrap(),
                    None,
                    IncidenceDirection::IntoRelation,
                ),
                ProjectionEndpoint::new(
                    id("mathscape:semantic/b"),
                    EndpointRole::parse("premise").unwrap(),
                    None,
                    IncidenceDirection::IntoRelation,
                ),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            repeated,
            GraphProjectionError::MissingEndpointOrdinal { .. }
        ));
    }

    #[test]
    fn rejects_unknown_endpoint_and_group_cycles() {
        let relation = ProjectionRelation::new(
            id("mathscape:alignment/example"),
            EntityKind::Alignment,
            ViewLayer::Alignment,
            "alignment",
            vec![ProjectionEndpoint::new(
                id("mathscape:semantic/missing"),
                EndpointRole::parse("candidate").unwrap(),
                None,
                IncidenceDirection::Undirected,
            )],
        )
        .unwrap();
        let error = GraphProjection::new(
            ProjectionLens::Alignment,
            vec![],
            vec![node("mathscape:semantic/existing")],
            vec![relation],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            GraphProjectionError::UnknownEndpoint { .. }
        ));

        let first = ProjectionGroupId::parse("first").unwrap();
        let second = ProjectionGroupId::parse("second").unwrap();
        let groups = vec![
            ProjectionGroup::new(
                first.clone(),
                "First",
                ViewLayer::Source,
                Some(second.clone()),
            )
            .unwrap(),
            ProjectionGroup::new(second, "Second", ViewLayer::Source, Some(first)).unwrap(),
        ];
        assert_eq!(
            GraphProjection::new(ProjectionLens::Source, groups, vec![], vec![]).unwrap_err(),
            GraphProjectionError::GroupCycle
        );
    }

    #[test]
    fn canonicalizes_collection_order_for_stable_serialization() {
        let projection = GraphProjection::new(
            ProjectionLens::Landscape,
            vec![],
            vec![node("mathscape:semantic/z"), node("mathscape:semantic/a")],
            vec![],
        )
        .unwrap();
        assert_eq!(
            projection
                .nodes()
                .iter()
                .map(|node| node.entity_id().as_str())
                .collect::<Vec<_>>(),
            vec!["mathscape:semantic/a", "mathscape:semantic/z"]
        );
    }
}
