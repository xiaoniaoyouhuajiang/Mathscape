//! Controlled Lean/Lake capability discovery and execution.
//!
//! The first slice intentionally stops at environment configuration, probing,
//! fingerprinting, and an argv-safe process runner. It does not register MCP
//! tools or claim that Lean execution is an operating-system read-only
//! sandbox; callers must still apply the Mathscape approval policy.

mod config;
mod declaration;
mod error;
mod export;
mod fingerprint;
mod loogle;
mod probe;
mod profile;
mod runner;

pub use config::{
    ELAN_EXECUTABLE_ENV, LAKE_EXECUTABLE_ENV, LEAN_CACHE_ENV, LEAN_EXECUTABLE_ENV,
    LEAN_PROJECT_ENV, LeanProjectConfig,
};
pub use declaration::{DECLARATION_SCHEMA_VERSION, DeclarationCandidate, DeclarationKind};
pub use error::{
    LeanConfigError, LeanExportError, LeanFingerprintError, LeanLoogleError, LeanProbeError,
    LeanProfileError, LeanRunnerError,
};
pub use export::{DeclarationExportRequest, DeclarationExportResult, DeclarationExporter};
pub use fingerprint::{
    LEAN_FINGERPRINT_ALGORITHM, LEAN_FINGERPRINT_SCHEMA_VERSION, LeanEnvironmentFingerprint,
    LeanFingerprintFile, LeanFingerprintStatus, fingerprint_environment,
};
pub use loogle::{LoogleAdapter, LoogleIndexMode, LoogleSearchRequest, LoogleSearchResult};
pub use probe::{LeanEnvironmentProbe, LeanEnvironmentState};
pub use profile::{
    DEFAULT_LEAN_TOOLCHAIN, DEFAULT_MATHLIB_REFERENCE, DEFAULT_PROFILE_ID, LEAN_PROFILE_ROOT_ENV,
    LEAN_PROFILE_SCHEMA_VERSION, LeanProfileState, LeanProfileStore, ManagedLeanProfile,
    ManagedLeanProfileMetadata, ManagedLeanProfileSpec,
};
pub use runner::{
    CapturedOutput, LeanCancellation, LeanCommand, LeanRun, LeanRunStatus, LeanRunner,
    LeanRunnerOptions,
};
