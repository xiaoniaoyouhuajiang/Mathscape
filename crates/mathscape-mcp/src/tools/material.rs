//! Shared catalog lookup and managed-path checks for material tools.
//!
//! Path checks stay inside [`ArtifactPool::managed_path`]. This module only
//! classifies the pool error into the tool envelope: a missing managed copy
//! is business `not_found`, an escaping path or symbolic link is call-error
//! `forbidden`, and other catalog or I/O failures are `internal`.

use std::path::PathBuf;

use mathscape_agent::{ToolError, ToolErrorCode};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError};
use mathscape_domain::StableId;

use crate::McpWorkspace;

pub(crate) const MARKDOWN_MEDIA_TYPE: &str = "text/markdown";
pub(crate) const PDF_MEDIA_TYPE: &str = "application/pdf";

pub(crate) struct Material {
    pub id: StableId,
    pub media_type: String,
    pub byte_size: u64,
    pub revision: String,
}

pub(crate) fn find_material(
    workspace: &McpWorkspace,
    id: &StableId,
) -> Result<Option<Material>, ToolError> {
    let catalog = workspace
        .store()?
        .read_catalog_snapshot()
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to read artifact catalog: {error}"),
            )
        })?;
    Ok(catalog
        .artifacts()
        .iter()
        .find(|record| record.entity().header().stable_id() == id)
        .map(|record| {
            let header = record.entity().header();
            let payload = record.entity().payload();
            Material {
                id: id.clone(),
                media_type: payload.media_type().as_str().to_owned(),
                byte_size: payload.byte_size(),
                revision: header.revision_id().to_string(),
            }
        }))
}

pub(crate) enum ManagedPathFailure {
    NotFound,
    Forbidden,
    Internal,
}

pub(crate) fn managed_path_failure(error: &ArtifactPoolError) -> ManagedPathFailure {
    match error {
        ArtifactPoolError::MissingManagedLocation(_) => ManagedPathFailure::NotFound,
        ArtifactPoolError::Io { source, .. } => {
            if source.kind() == std::io::ErrorKind::NotFound {
                ManagedPathFailure::NotFound
            } else {
                ManagedPathFailure::Internal
            }
        }
        ArtifactPoolError::UnsafeManagedPath(_) => ManagedPathFailure::Forbidden,
        ArtifactPoolError::InvalidReadRange
        | ArtifactPoolError::InvalidWorkspaceManifest { .. }
        | ArtifactPoolError::SourceNotFile(_)
        | ArtifactPoolError::InvalidPoolUri(_)
        | ArtifactPoolError::DigestMismatch { .. }
        | ArtifactPoolError::ManagedObjectConflict { .. }
        | ArtifactPoolError::Domain(_)
        | ArtifactPoolError::Store(_) => ManagedPathFailure::Internal,
    }
}

pub(crate) fn open_managed_path(
    pool: &ArtifactPool,
    id: &StableId,
) -> Result<PathBuf, ArtifactPoolError> {
    pool.managed_path(id)
}
