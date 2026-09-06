use std::{io, path::PathBuf};

use mathscape_domain::ContentHash;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactPoolError {
    #[error(
        "invalid artifact read range: length must be 1..=4 MiB and offset must not exceed file size"
    )]
    InvalidReadRange,
    #[error("{operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workspace manifest at {path} is invalid: {reason}")]
    InvalidWorkspaceManifest { path: PathBuf, reason: &'static str },
    #[error("source is not a regular file: {0}")]
    SourceNotFile(PathBuf),
    #[error("invalid managed Artifact URI {0:?}")]
    InvalidPoolUri(String),
    #[error("managed Artifact path escapes the Workspace or uses a symbolic link: {0}")]
    UnsafeManagedPath(PathBuf),
    #[error(
        "staged content digest {actual} does not match expected digest {expected}; quarantined at {quarantine}"
    )]
    DigestMismatch {
        expected: ContentHash,
        actual: ContentHash,
        quarantine: PathBuf,
    },
    #[error("managed object at {path} conflicts with content identity {expected}")]
    ManagedObjectConflict {
        path: PathBuf,
        expected: ContentHash,
    },
    #[error("artifact {0} has no registered Managed location")]
    MissingManagedLocation(String),
    #[error("domain validation failed: {0}")]
    Domain(#[from] mathscape_domain::DomainError),
    #[error("artifact metadata store failed: {0}")]
    Store(#[from] mathscape_store_sqlite::SqliteStoreError),
}

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: io::Error,
) -> ArtifactPoolError {
    ArtifactPoolError::Io {
        operation,
        path: path.into(),
        source,
    }
}
