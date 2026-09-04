use std::{io, path::PathBuf};

/// A deterministic failure while constructing a Marker request.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MarkerRequestError {
    #[error("{field} path must not be empty")]
    EmptyPath { field: &'static str },
    #[error("{field} path must be absolute: {path:?}")]
    RelativePath { field: &'static str, path: PathBuf },
    #[error("Marker source PDF and output root must be different paths")]
    ConflictingPaths,
}

/// Operational failure before or while controlling a Marker child process.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MarkerExecutionError {
    #[error(transparent)]
    InvalidRequest(#[from] MarkerRequestError),
    #[error("invalid execution option {field}: {reason}")]
    InvalidOption {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{field} is not a regular file: {path:?}")]
    NotAFile { field: &'static str, path: PathBuf },
    #[error("{field} is not executable: {path:?}")]
    NotExecutable { field: &'static str, path: PathBuf },
    #[error("{field} is not a directory: {path:?}")]
    NotADirectory { field: &'static str, path: PathBuf },
    #[error("failed to access {field} at {path:?}: {source}")]
    Access {
        field: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to prepare Marker staging path {path:?}: {source}")]
    Staging {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to spawn {program:?}; diagnostics remain at {staging_root:?}: {source}")]
    Spawn {
        program: PathBuf,
        staging_root: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "failed while waiting for {program:?}; diagnostics remain at {staging_root:?}: {source}"
    )]
    Wait {
        program: PathBuf,
        staging_root: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Marker version probe failed with status {exit_code:?}: {stderr}")]
    ProbeFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("could not parse Marker version from {0:?}")]
    InvalidVersion(String),
    #[error("Marker major version {actual} is unsupported; expected {expected}")]
    UnsupportedVersion { expected: u64, actual: u64 },
}

pub(crate) fn validate_absolute_path(
    path: PathBuf,
    field: &'static str,
) -> Result<PathBuf, MarkerRequestError> {
    if path.as_os_str().is_empty() {
        return Err(MarkerRequestError::EmptyPath { field });
    }
    if !path.is_absolute() {
        return Err(MarkerRequestError::RelativePath { field, path });
    }
    Ok(path)
}
