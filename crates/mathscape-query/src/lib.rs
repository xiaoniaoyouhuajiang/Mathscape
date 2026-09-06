//! Read models and stable query contracts for UI, CLI, and orchestration.

mod artifact_graph;

pub use artifact_graph::{
    ArtifactGraphProjectionError, ArtifactGraphProjector, project_artifact_catalog,
};
