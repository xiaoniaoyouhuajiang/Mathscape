//! Typed, rebuildable graph projections and graph algorithms.
//!
//! Persistent domain entities and hyperedges remain authoritative. This crate
//! produces deterministic, renderer-neutral read models and an incidence-graph
//! expansion suitable for binary-edge renderers such as Cytoscape.js.

mod incidence;
mod projection;

pub use incidence::{
    IncidenceEdge, IncidenceGraph, IncidenceNode, IncidenceNodeKind, ProjectionIndex,
    RenderElementId,
};
pub use projection::{
    GRAPH_PROJECTION_SCHEMA_VERSION, GraphProjection, GraphProjectionError, HopClass,
    IncidenceDirection, ProjectionEndpoint, ProjectionGroup, ProjectionGroupId, ProjectionLens,
    ProjectionNode, ProjectionRelation, ProjectionRelationKind, ViewLayer,
};
