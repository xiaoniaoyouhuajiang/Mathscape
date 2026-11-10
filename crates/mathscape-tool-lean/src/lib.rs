//! Controlled Lean/Lake capability discovery and execution.
//!
//! The managed profile, declaration export, local Loogle search and bounded
//! `lean.check` core live here. MCP registration and OS-level sandboxing remain
//! outside this crate; callers must still apply the Mathscape approval policy.

mod check;
mod config;
mod declaration;
mod discovery;
mod error;
mod export;
mod fingerprint;
mod loogle;
#[cfg(unix)]
mod loogle_ipc;
mod loogle_resident;
mod probe;
mod profile;
mod receipt;
mod runner;

pub use check::{
    LEAN_CHECK_CONTRACT_VERSION, LEAN_CHECK_DEFAULT_OUTPUT_BYTES, LEAN_CHECK_DEFAULT_TIMEOUT_MS,
    LEAN_CHECK_MAX_OUTPUT_BYTES, LEAN_CHECK_MAX_SOURCE_BYTES, LEAN_CHECK_MAX_TIMEOUT_MS,
    LEAN_CHECK_MIN_OUTPUT_BYTES, LEAN_CHECK_MIN_TIMEOUT_MS, LeanCheckRequest, LeanCheckResult,
    LeanCheckStatus, LeanChecker, LeanDiagnostic, LeanDiagnosticSeverity, LeanEvidenceRef,
    LeanProfileRef,
};
pub use config::{
    ELAN_EXECUTABLE_ENV, LAKE_EXECUTABLE_ENV, LEAN_CACHE_ENV, LEAN_EXECUTABLE_ENV,
    LEAN_PROJECT_ENV, LeanProjectConfig,
};
pub use declaration::{DECLARATION_SCHEMA_VERSION, DeclarationCandidate, DeclarationKind};
pub use discovery::{
    DiscoveredLeanProfile, DiscoveredLeanProfileRoot, LeanProfileRootSource, WORKSPACE_PROFILE_DIR,
    discover_profile_root, list_profiles,
};
pub use error::{
    LeanCheckError, LeanConfigError, LeanExportError, LeanFingerprintError, LeanLoogleError,
    LeanProbeError, LeanProfileError, LeanReceiptError, LeanRunnerError,
};
pub use export::{DeclarationExportRequest, DeclarationExportResult, DeclarationExporter};
pub use fingerprint::{
    LEAN_FINGERPRINT_ALGORITHM, LEAN_FINGERPRINT_SCHEMA_VERSION, LeanEnvironmentFingerprint,
    LeanFingerprintFile, LeanFingerprintStatus, fingerprint_environment,
};
pub use loogle::{
    LoogleAdapter, LoogleIndexMode, LoogleSearchRequest, LoogleSearchResult,
    MANAGED_LOOGLE_INDEX_RELATIVE_PATH, MANAGED_LOOGLE_RELATIVE_PATH, fingerprint_file,
};
#[cfg(unix)]
pub use loogle_ipc::{
    LOOGLE_IPC_VERSION, LOOGLE_RESIDENT_ENV, LOOGLE_SOCKET_ENV, LoogleBackendMode, LoogleIpcError,
    LoogleIpcRequest, LoogleIpcResponse, LoogleResidentHost, default_socket_path,
    request_over_socket, resident_forced_off,
};
pub use loogle_resident::{
    LOOGLE_INSTANCE_NAME, LOOGLE_READY_LINE, LoogleResidentKey, LoogleTarget, RESIDENT_MAX_RESULTS,
    ResidentLoogle, ResidentLoogleOutcome, ResidentLoogleResult, loogle_resident_policy,
    validate_resident_query,
};
pub use probe::{LeanEnvironmentProbe, LeanEnvironmentState};
pub use profile::{
    DEFAULT_LEAN_TOOLCHAIN, DEFAULT_MATHLIB_REFERENCE, DEFAULT_PROFILE_ID, LEAN_PROFILE_ROOT_ENV,
    LEAN_PROFILE_SCHEMA_VERSION, LeanProfileState, LeanProfileStore, ManagedLeanProfile,
    ManagedLeanProfileMetadata, ManagedLeanProfileSpec,
};
pub use receipt::{
    LEAN_RECEIPT_SCHEMA_VERSION, LEAN_RUNNER_SCHEMA_VERSION, LeanCheckReceipt, LeanCheckService,
    LeanReceiptStore, LeanRunnerReceipt,
};
pub use runner::{
    CapturedOutput, LeanCancellation, LeanCommand, LeanRun, LeanRunStatus, LeanRunner,
    LeanRunnerOptions,
};
