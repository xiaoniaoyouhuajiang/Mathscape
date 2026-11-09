use std::{io, path::PathBuf};

/// Configuration errors are deterministic and can be shown before probing a
/// Lean project.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanConfigError {
    #[error("{field} path must not be empty")]
    EmptyPath { field: &'static str },
    #[error("{field} path must be absolute: {path:?}")]
    RelativePath { field: &'static str, path: PathBuf },
    #[error("{field} is not a directory: {path:?}")]
    NotADirectory { field: &'static str, path: PathBuf },
    #[error("failed to inspect {field} at {path:?}: {source}")]
    Access {
        field: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid {field} value: {value:?}")]
    InvalidValue { field: &'static str, value: String },
}

/// A failure while collecting environment information. Missing tools are
/// represented by [`LeanEnvironmentState`](crate::LeanEnvironmentState), not
/// by this error type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanProbeError {
    #[error(transparent)]
    Config(#[from] LeanConfigError),
    #[error("failed to inspect Lean project path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to compute Lean environment probe: {0}")]
    Internal(String),
}

/// Failure while producing a stable environment fingerprint.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanFingerprintError {
    #[error(transparent)]
    Config(#[from] LeanConfigError),
    #[error("failed to inspect fingerprint path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read fingerprint path {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse Lake manifest {path:?}: {source}")]
    InvalidManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Failure while spawning or controlling a Lean/Lake child process.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanRunnerError {
    #[error("invalid runner option {field}: {reason}")]
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
    #[error("failed to spawn Lean command {program:?}: {source}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed while waiting for Lean command {program:?}: {source}")]
    Wait {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed while capturing output from Lean command {program:?}: {reason}")]
    Capture {
        program: PathBuf,
        reason: &'static str,
    },
    #[error("failed to terminate Lean command {program:?}: {source}")]
    Terminate {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Failure while creating, loading, or validating a Mathscape-managed Lean
/// environment profile.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanProfileError {
    #[error("invalid Lean profile id {0:?}")]
    InvalidId(String),
    #[error("Lean profile path must be absolute: {0:?}")]
    RelativeRoot(PathBuf),
    #[error("Lean profile {id:?} was not found at {path:?}")]
    NotFound { id: String, path: PathBuf },
    #[error("failed to access Lean profile path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read Lean profile metadata {path:?}: {source}")]
    ReadMetadata {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse Lean profile metadata {path:?}: {source}")]
    InvalidMetadata {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize Lean profile metadata: {0}")]
    SerializeMetadata(#[source] serde_json::Error),
    #[error("failed to write Lean profile metadata {path:?}: {source}")]
    WriteMetadata {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Lean profile {id:?} has toolchain {actual:?}; expected {expected:?}")]
    ToolchainMismatch {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("Lean profile {id:?} has mathlib revision {actual:?}; expected {expected:?}")]
    MathlibMismatch {
        id: String,
        expected: String,
        actual: Option<String>,
    },
    #[error(transparent)]
    Config(#[from] LeanConfigError),
    #[error(transparent)]
    Probe(#[from] LeanProbeError),
    #[error(transparent)]
    Fingerprint(#[from] LeanFingerprintError),
}

/// Failure while exporting declaration metadata from a ready Lean profile.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanExportError {
    #[error("invalid export option {field}: {reason}")]
    InvalidOption {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid declaration name {0:?}")]
    InvalidDeclarationName(String),
    #[error("Lean export capability unavailable: {reason}")]
    CapabilityUnavailable { reason: &'static str },
    #[error("failed to access export path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize export request: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("Lean exporter failed with {status}: {stderr}")]
    RunFailed { status: String, stderr: String },
    #[error("Lean exporter output exceeded the capture limit")]
    OutputTruncated,
    #[error("invalid exporter JSON on line {line}: {source}")]
    InvalidJson {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to clean exporter file {path:?}: {source}")]
    Cleanup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Runner(#[from] LeanRunnerError),
}

/// Failure while checking a complete Lean source file in a managed profile.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanCheckError {
    #[error("invalid check option {field}: {reason}")]
    InvalidOption {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid Lean source: {reason}")]
    InvalidSource { reason: &'static str },
    #[error("Lean check capability unavailable: {reason}")]
    CapabilityUnavailable { reason: String },
    #[error("Lean profile fingerprint mismatch: expected {expected:?}, actual {actual:?}")]
    ProfileFingerprintMismatch { expected: String, actual: String },
    #[error("failed to access Lean check path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to clean Lean check path {path:?}: {source}")]
    Cleanup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Lean check output exceeded the capture limit")]
    OutputTruncated,
    #[error(transparent)]
    Runner(#[from] LeanRunnerError),
}

/// Failure while persisting or validating a Lean check receipt.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanReceiptError {
    #[error("Lean receipt root must be absolute: {0:?}")]
    RelativeRoot(PathBuf),
    #[error("invalid Lean receipt id {0:?}")]
    InvalidReceiptId(String),
    #[error("invalid Lean source artifact id {0:?}")]
    InvalidSourceArtifactId(String),
    #[error("Lean receipt capability unavailable: {reason}")]
    CapabilityUnavailable { reason: String },
    #[error("Lean check result is inconsistent: {reason}")]
    InconsistentResult { reason: &'static str },
    #[error("Lean receipt {receipt_id:?} was not found")]
    NotFound { receipt_id: String },
    #[error("failed to access Lean receipt path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read Lean receipt path {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write Lean receipt path {path:?}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid Lean receipt JSON at {path:?}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("Lean receipt integrity check failed at {path:?}: {reason}")]
    Integrity { path: PathBuf, reason: String },
    #[error(transparent)]
    Check(#[from] LeanCheckError),
}

/// Failure while querying a local Loogle binary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeanLoogleError {
    #[error("invalid Loogle query: {0}")]
    InvalidQuery(String),
    /// Loogle ran but rejected the query (for example an unknown bare identifier).
    #[error("Loogle rejected the query: {message}")]
    QueryRejected {
        message: String,
        suggestions: Vec<String>,
    },
    #[error("Loogle capability unavailable: {reason}")]
    CapabilityUnavailable { reason: &'static str },
    #[error("failed to access Loogle path {path:?}: {source}")]
    Access {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Loogle failed with {status}: {stderr}")]
    RunFailed { status: String, stderr: String },
    #[error("Loogle output exceeded the capture limit")]
    OutputTruncated,
    #[error("invalid Loogle JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
    #[error(transparent)]
    Runner(#[from] LeanRunnerError),
}

pub(crate) fn validate_absolute_path(
    path: PathBuf,
    field: &'static str,
) -> Result<PathBuf, LeanConfigError> {
    if path.as_os_str().is_empty() {
        return Err(LeanConfigError::EmptyPath { field });
    }
    if !path.is_absolute() {
        return Err(LeanConfigError::RelativePath { field, path });
    }
    Ok(path)
}
