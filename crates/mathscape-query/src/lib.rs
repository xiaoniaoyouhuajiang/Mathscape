//! Read models and stable query contracts for UI, CLI, and orchestration.

mod artifact_graph;
mod source_graph;

pub use artifact_graph::{
    ArtifactGraphProjectionError, ArtifactGraphProjector, project_artifact_catalog,
};
pub use source_graph::{SourceGraphProjectionError, SourceGraphProjector, project_source_map};
