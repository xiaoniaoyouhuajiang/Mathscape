use std::collections::HashMap;

use mathscape_domain::{EndpointRole, EntityKind, LifecycleStatus, StableId};
use petgraph::{Direction, stable_graph::StableDiGraph};
use serde::Serialize;

use crate::{
    GRAPH_PROJECTION_SCHEMA_VERSION, GraphProjection, IncidenceDirection, ProjectionGroupId,
    ProjectionLens, ViewLayer,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RenderElementId(String);

impl RenderElementId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn entity(entity: &StableId) -> Self {
        Self(format!("entity:{}", entity.as_str()))
    }

    fn relation(relation: &StableId) -> Self {
        Self(format!("relation:{}", relation.as_str()))
    }

    fn group(group: &ProjectionGroupId) -> Self {
        Self(format!("group:{}", group.as_str()))
    }

    fn incidence(relation: &StableId, role: &EndpointRole, ordinal: Option<u32>) -> Self {
        let slot = ordinal.map_or_else(|| "single".to_owned(), |value| value.to_string());
        Self(format!(
            "incidence:{}:{}:{slot}",
            relation.as_str(),
            role.as_str()
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidenceNodeKind {
    Entity,
    Relation,
    Group,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IncidenceNode {
    id: RenderElementId,
    element_kind: IncidenceNodeKind,
    domain_id: Option<StableId>,
    domain_kind: Option<EntityKind>,
    layer: ViewLayer,
    label: String,
    summary: Option<String>,
    lifecycle: Option<LifecycleStatus>,
    parent: Option<RenderElementId>,
    boundary: bool,
}

impl IncidenceNode {
    pub fn id(&self) -> &RenderElementId {
        &self.id
    }

    pub const fn element_kind(&self) -> IncidenceNodeKind {
        self.element_kind
    }

    pub fn domain_id(&self) -> Option<&StableId> {
        self.domain_id.as_ref()
    }

    pub const fn domain_kind(&self) -> Option<EntityKind> {
        self.domain_kind
    }

    pub const fn layer(&self) -> ViewLayer {
        self.layer
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub const fn lifecycle(&self) -> Option<LifecycleStatus> {
        self.lifecycle
    }

    pub fn parent(&self) -> Option<&RenderElementId> {
        self.parent.as_ref()
    }

    pub const fn is_boundary(&self) -> bool {
        self.boundary
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IncidenceEdge {
    id: RenderElementId,
    source: RenderElementId,
    target: RenderElementId,
    relation_id: StableId,
    role: EndpointRole,
    ordinal: Option<u32>,
    direction: IncidenceDirection,
    directed: bool,
}

impl IncidenceEdge {
    pub fn id(&self) -> &RenderElementId {
        &self.id
    }

    pub fn source(&self) -> &RenderElementId {
        &self.source
    }

    pub fn target(&self) -> &RenderElementId {
        &self.target
    }

    pub fn relation_id(&self) -> &StableId {
        &self.relation_id
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

    pub const fn is_directed(&self) -> bool {
        self.directed
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IncidenceGraph {
    schema_version: String,
    lens: ProjectionLens,
    nodes: Vec<IncidenceNode>,
    edges: Vec<IncidenceEdge>,
}

impl IncidenceGraph {
    pub fn project(projection: &GraphProjection) -> Self {
        let mut nodes = Vec::with_capacity(
            projection.groups().len() + projection.nodes().len() + projection.relations().len(),
        );
        for group in projection.groups() {
            nodes.push(IncidenceNode {
                id: RenderElementId::group(group.id()),
                element_kind: IncidenceNodeKind::Group,
                domain_id: None,
                domain_kind: None,
                layer: group.layer(),
                label: group.label().to_owned(),
                summary: None,
                lifecycle: None,
                parent: group.parent().map(RenderElementId::group),
                boundary: false,
            });
        }
        for node in projection.nodes() {
            nodes.push(IncidenceNode {
                id: RenderElementId::entity(node.entity_id()),
                element_kind: IncidenceNodeKind::Entity,
                domain_id: Some(node.entity_id().clone()),
                domain_kind: Some(node.kind()),
                layer: node.layer(),
                label: node.label().to_owned(),
                summary: node.summary_ref().map(str::to_owned),
                lifecycle: node.lifecycle_status(),
                parent: node.group_ref().map(RenderElementId::group),
                boundary: node.is_boundary(),
            });
        }

        let mut edges = Vec::new();
        for relation in projection.relations() {
            let relation_node = RenderElementId::relation(relation.relation_id());
            nodes.push(IncidenceNode {
                id: relation_node.clone(),
                element_kind: IncidenceNodeKind::Relation,
                domain_id: Some(relation.relation_id().clone()),
                domain_kind: Some(relation.kind()),
                layer: relation.layer(),
                label: relation.label().to_owned(),
                summary: relation.summary_ref().map(str::to_owned),
                lifecycle: relation.lifecycle_status(),
                parent: relation.group_ref().map(RenderElementId::group),
                boundary: relation.is_boundary(),
            });
            for endpoint in relation.endpoints() {
                let entity_node = RenderElementId::entity(endpoint.node());
                let (source, target, directed) = match endpoint.direction() {
                    IncidenceDirection::IntoRelation => (entity_node, relation_node.clone(), true),
                    IncidenceDirection::OutOfRelation => (relation_node.clone(), entity_node, true),
                    IncidenceDirection::Undirected => (relation_node.clone(), entity_node, false),
                };
                edges.push(IncidenceEdge {
                    id: RenderElementId::incidence(
                        relation.relation_id(),
                        endpoint.role(),
                        endpoint.ordinal(),
                    ),
                    source,
                    target,
                    relation_id: relation.relation_id().clone(),
                    role: endpoint.role().clone(),
                    ordinal: endpoint.ordinal(),
                    direction: endpoint.direction(),
                    directed,
                });
            }
        }
        nodes.sort_unstable_by(|left, right| left.id().cmp(right.id()));
        edges.sort_unstable_by(|left, right| left.id().cmp(right.id()));
        Self {
            schema_version: GRAPH_PROJECTION_SCHEMA_VERSION.to_owned(),
            lens: projection.lens(),
            nodes,
            edges,
        }
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn lens(&self) -> ProjectionLens {
        self.lens
    }

    pub fn nodes(&self) -> &[IncidenceNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[IncidenceEdge] {
        &self.edges
    }

    pub fn index(&self) -> ProjectionIndex {
        ProjectionIndex::new(self)
    }
}

pub struct ProjectionIndex {
    graph: StableDiGraph<IncidenceNode, IncidenceEdge>,
    nodes: HashMap<RenderElementId, petgraph::stable_graph::NodeIndex>,
}

impl ProjectionIndex {
    fn new(incidence: &IncidenceGraph) -> Self {
        let mut graph = StableDiGraph::new();
        let mut nodes = HashMap::new();
        for node in &incidence.nodes {
            nodes.insert(node.id.clone(), graph.add_node(node.clone()));
        }
        for edge in &incidence.edges {
            graph.add_edge(nodes[&edge.source], nodes[&edge.target], edge.clone());
        }
        Self { graph, nodes }
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn contains(&self, id: &RenderElementId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn neighbors<'a>(
        &'a self,
        id: &RenderElementId,
    ) -> impl Iterator<Item = &'a IncidenceNode> + 'a {
        self.nodes.get(id).into_iter().flat_map(|index| {
            self.graph
                .neighbors_directed(*index, Direction::Outgoing)
                .chain(self.graph.neighbors_directed(*index, Direction::Incoming))
                .map(|neighbor| &self.graph[neighbor])
        })
    }
}

#[cfg(test)]
mod tests {
    use mathscape_domain::{EndpointRole, EntityKind, StableId};

    use super::*;
    use crate::{
        GraphProjection, IncidenceDirection, ProjectionEndpoint, ProjectionLens, ProjectionNode,
        ProjectionRelation, ViewLayer,
    };

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    #[test]
    fn expands_a_hyperedge_into_one_relation_node_and_role_edges() {
        let pdf = id("mathscape:artifact/pdf");
        let markdown = id("mathscape:artifact/markdown");
        let metadata = id("mathscape:artifact/metadata");
        let conversion = id("mathscape:conversion/marker");
        let projection = GraphProjection::new(
            ProjectionLens::Source,
            vec![],
            vec![
                ProjectionNode::new(
                    pdf.clone(),
                    EntityKind::SourceArtifact,
                    ViewLayer::Source,
                    "PDF",
                )
                .unwrap(),
                ProjectionNode::new(
                    markdown.clone(),
                    EntityKind::SourceArtifact,
                    ViewLayer::Source,
                    "Markdown",
                )
                .unwrap(),
                ProjectionNode::new(
                    metadata.clone(),
                    EntityKind::SourceArtifact,
                    ViewLayer::Source,
                    "Metadata",
                )
                .unwrap(),
            ],
            vec![
                ProjectionRelation::new(
                    conversion,
                    EntityKind::ConversionRun,
                    ViewLayer::Source,
                    "Marker conversion",
                    vec![
                        ProjectionEndpoint::new(
                            pdf,
                            EndpointRole::parse("input").unwrap(),
                            None,
                            IncidenceDirection::IntoRelation,
                        ),
                        ProjectionEndpoint::new(
                            markdown,
                            EndpointRole::parse("output").unwrap(),
                            Some(0),
                            IncidenceDirection::OutOfRelation,
                        ),
                        ProjectionEndpoint::new(
                            metadata,
                            EndpointRole::parse("output").unwrap(),
                            Some(1),
                            IncidenceDirection::OutOfRelation,
                        ),
                    ],
                )
                .unwrap(),
            ],
        )
        .unwrap();

        let incidence = IncidenceGraph::project(&projection);
        assert_eq!(incidence.lens(), ProjectionLens::Source);
        assert_eq!(incidence.nodes().len(), 4);
        assert_eq!(incidence.edges().len(), 3);
        assert_eq!(
            incidence
                .nodes()
                .iter()
                .filter(|node| node.element_kind() == IncidenceNodeKind::Relation)
                .count(),
            1
        );
        assert_eq!(incidence.index().node_count(), 4);
        assert_eq!(incidence.index().edge_count(), 3);
        assert!(incidence.edges().iter().any(|edge| {
            edge.role().as_str() == "input"
                && edge.source().as_str().starts_with("entity:")
                && edge.target().as_str().starts_with("relation:")
        }));

        let wire = serde_json::to_value(&incidence).unwrap();
        assert_eq!(
            wire["schema_version"],
            serde_json::json!(GRAPH_PROJECTION_SCHEMA_VERSION)
        );
        assert_eq!(wire["lens"], serde_json::json!("source"));
        assert_eq!(wire["nodes"].as_array().unwrap().len(), 4);
        assert_eq!(wire["edges"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn maps_view_groups_to_compound_node_parents() {
        let group = ProjectionGroupId::parse("source-book").unwrap();
        let artifact = id("mathscape:artifact/grouped");
        let projection = GraphProjection::new(
            ProjectionLens::Source,
            vec![
                crate::ProjectionGroup::new(group.clone(), "Source book", ViewLayer::Source, None)
                    .unwrap(),
            ],
            vec![
                ProjectionNode::new(
                    artifact.clone(),
                    EntityKind::SourceArtifact,
                    ViewLayer::Source,
                    "PDF",
                )
                .unwrap()
                .group(group.clone()),
            ],
            vec![],
        )
        .unwrap();

        let incidence = IncidenceGraph::project(&projection);
        let entity = incidence
            .nodes()
            .iter()
            .find(|node| node.domain_id() == Some(&artifact))
            .unwrap();
        assert_eq!(
            entity.parent().map(RenderElementId::as_str),
            Some("group:source-book")
        );
        assert!(incidence.nodes().iter().any(|node| {
            node.id().as_str() == "group:source-book"
                && node.element_kind() == IncidenceNodeKind::Group
        }));
    }
}
