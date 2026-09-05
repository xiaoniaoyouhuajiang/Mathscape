//! Managed, content-addressed Artifact Pool for a portable Mathscape Workspace.

mod error;
mod pool;
mod workspace;

pub use error::ArtifactPoolError;
pub use pool::{
    ArtifactImportOutcome, ArtifactMetadata, ArtifactPool, ArtifactVerification, ManagedArtifact,
    PortabilityReport,
};
pub use workspace::{WORKSPACE_FORMAT_VERSION, WorkspaceRoot};
