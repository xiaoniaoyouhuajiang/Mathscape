//! Lean/mathlib MCP tools.
//!
//! Both tools are read-only from the workspace perspective. Lean checks use
//! profile-local scratch files and receipt storage; they never expose a user
//! path or a shell argument to the Agent.

use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_tool_lean::{
    LeanCheckError, LeanCheckRequest, LeanChecker, LeanLoogleError, LeanProfileRef,
    LeanProfileStore, LeanReceiptStore, LoogleAdapter, LoogleSearchRequest, ManagedLeanProfile,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{LEAN_CHECK, MATHLIB_SEARCH, diagnostic};
use crate::McpWorkspace;

const MAX_QUERY_CHARS: usize = 4096;
const DEFAULT_MAX_RESULTS: usize = 50;
const MAX_RESULTS: usize = 100;
const LOOGLE_RELATIVE_PATH: &str = "tools/loogle/.lake/build/bin/loogle";
const LOOGLE_INDEX_RELATIVE_PATH: &str = "loogle/mathlib.loogle-index";

const SEARCH_DESCRIPTION: &str = "Search declarations in the local managed mathlib profile with Loogle. Results are candidates only; validate a proposed proof with lean.check. The profile and index are local and fingerprinted; this tool never contacts the network or accepts executable/index paths.";
const CHECK_DESCRIPTION: &str = "Check one complete Lean source file with the selected managed profile and persist an immutable verification receipt. The source must contain its own imports, declarations and proof; this tool never reads a user path or writes the user Lean project. A checked result proves only the submitted Lean source.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    profile_id: String,
    #[serde(default)]
    profile_fingerprint: Option<String>,
    query: String,
    #[serde(default = "default_module")]
    module: String,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckArgs {
    profile_id: String,
    #[serde(default)]
    profile_fingerprint: Option<String>,
    source_text: String,
    #[serde(default)]
    evidence_refs: Vec<mathscape_tool_lean::LeanEvidenceRef>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_output_bytes: Option<usize>,
}

enum ProfileResolution {
    Ready {
        store: LeanProfileStore,
        profile: Box<ManagedLeanProfile>,
    },
    Capability(Value),
}

pub(super) fn search_definition() -> Value {
    json!({
        "name": MATHLIB_SEARCH,
        "description": SEARCH_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "profile_id": {
                    "type": "string",
                    "description": "Managed Lean profile id.",
                },
                "profile_fingerprint": {
                    "type": "string",
                    "description": "Optional expected profile fingerprint; a mismatch is rejected.",
                },
                "query": {
                    "type": "string",
                    "description": format!("Loogle query, at most {MAX_QUERY_CHARS} Unicode characters. A bare identifier must be an existing fully qualified constant (e.g. InformationTheory.klDiv); quote a fragment (\"klDiv\") to search by name substring. A rejected query returns invalid_arg with Loogle's message and suggestions."),
                },
                "module": {
                    "type": "string",
                    "default": "Mathlib",
                    "description": "Lean module filter.",
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "default": DEFAULT_MAX_RESULTS,
                },
            },
            "required": ["profile_id", "query"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn check_definition() -> Value {
    json!({
        "name": LEAN_CHECK,
        "description": CHECK_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "profile_id": {
                    "type": "string",
                    "description": "Managed Lean profile id.",
                },
                "profile_fingerprint": {
                    "type": "string",
                    "description": "Optional expected profile fingerprint; a mismatch is rejected.",
                },
                "source_text": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 1048576,
                    "description": "Complete Lean source file, including imports.",
                },
                "evidence_refs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "node_id": { "type": "string" },
                            "artifact_id": { "type": "string" },
                            "locator": { "type": "object" },
                        },
                        "required": ["node_id"],
                        "additionalProperties": false,
                    },
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1000,
                    "maximum": 900000,
                    "default": 300000,
                },
                "max_output_bytes": {
                    "type": "integer",
                    "minimum": 32768,
                    "maximum": 16777216,
                    "default": 1048576,
                },
            },
            "required": ["profile_id", "source_text"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn search(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: SearchArgs = super::parse_args(arguments)?;
    validate_profile_id_field(&args.profile_id)?;
    validate_query(&args.query)?;
    validate_module(&args.module)?;
    if !(1..=MAX_RESULTS).contains(&args.max_results) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("max_results must be between 1 and {MAX_RESULTS}"),
        ));
    }
    let resolved = resolve_profile(workspace, &args.profile_id)?;
    let ProfileResolution::Ready { store, profile } = resolved else {
        let ProfileResolution::Capability(value) = resolved else {
            unreachable!();
        };
        return Ok(value);
    };
    validate_expected_fingerprint(args.profile_fingerprint.as_deref(), &profile)?;

    let profile_root = store.profile_root(&args.profile_id).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to resolve Lean profile root: {error}"),
        )
    })?;
    let executable = profile_root.join(LOOGLE_RELATIVE_PATH);
    if !executable.is_file() {
        return Ok(capability(
            &args.profile_id,
            "lean.loogle",
            "loogle_executable_missing",
            format!(
                "managed Loogle executable is missing at {}",
                executable.display()
            ),
        ));
    }
    let index_file = profile
        .fingerprint()
        .cache_root
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Internal,
                "managed profile cache root is unavailable",
            )
        })?
        .join(LOOGLE_INDEX_RELATIVE_PATH);
    if !index_file.is_file() {
        return Ok(capability(
            &args.profile_id,
            "lean.loogle",
            "loogle_index_missing",
            format!(
                "managed Loogle index is missing at {}",
                index_file.display()
            ),
        ));
    }
    let index_fingerprint = hash_file(&index_file).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to fingerprint Loogle index: {error}"),
        )
    })?;
    let request = LoogleSearchRequest::new(args.query)
        .map_err(|error| ToolError::new(ToolErrorCode::InvalidArg, error.to_string()))?
        .module(args.module)
        .max_results(args.max_results)
        .map_err(|error| ToolError::new(ToolErrorCode::InvalidArg, error.to_string()))?
        .index_file(index_file);
    let adapter = LoogleAdapter::new(executable).map_err(map_loogle_constructor_error)?;
    let result = match adapter.search(&profile, &request) {
        Ok(result) => result,
        Err(error) => return map_loogle_error(&args.profile_id, error),
    };
    let status = if result.truncated {
        ToolStatus::Truncated
    } else {
        ToolStatus::Ok
    };
    let mut output = json!({
        "status": status,
        "profile": profile_reference(&profile),
        "backend": result.backend,
        "backend_fingerprint": result.backend_fingerprint,
        "index_fingerprint": index_fingerprint,
        "total": result.total,
        "truncated": result.truncated,
        "candidates": result.candidates,
    });
    if result.candidates.is_empty() {
        output["diagnostics"] = json!([diagnostic(
            "no_matches",
            "Loogle returned no declarations for this query"
        )]);
    }
    Ok(output)
}

pub(super) fn check(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: CheckArgs = super::parse_args(arguments)?;
    validate_profile_id_field(&args.profile_id)?;
    let resolved = resolve_profile(workspace, &args.profile_id)?;
    let ProfileResolution::Ready { profile, .. } = resolved else {
        let ProfileResolution::Capability(value) = resolved else {
            unreachable!();
        };
        return Ok(value);
    };
    validate_expected_fingerprint(args.profile_fingerprint.as_deref(), &profile)?;

    let mut request = match LeanCheckRequest::new(args.source_text) {
        Ok(request) => request,
        Err(error) => return map_check_error(&args.profile_id, error),
    };
    if !args.evidence_refs.is_empty() {
        request = match request.evidence_refs(args.evidence_refs) {
            Ok(request) => request,
            Err(error) => return map_check_error(&args.profile_id, error),
        };
    }
    if let Some(timeout_ms) = args.timeout_ms {
        request = match request.timeout_ms(timeout_ms) {
            Ok(request) => request,
            Err(error) => return map_check_error(&args.profile_id, error),
        };
    }
    if let Some(max_output_bytes) = args.max_output_bytes {
        request = match request.max_output_bytes(max_output_bytes) {
            Ok(request) => request,
            Err(error) => return map_check_error(&args.profile_id, error),
        };
    }
    if let Some(fingerprint) = args.profile_fingerprint {
        request = request.profile_fingerprint(fingerprint);
    }

    let receipt_store = match LeanReceiptStore::for_profile(&profile) {
        Ok(store) => store,
        Err(error) => {
            return Ok(capability(
                &args.profile_id,
                "lean.receipt",
                "receipt_store_unavailable",
                error.to_string(),
            ));
        }
    };
    let result = match LeanChecker::new().check(
        &profile,
        &request,
        &mathscape_tool_lean::LeanCancellation::new(),
    ) {
        Ok(result) => result,
        Err(error) => return map_check_error(&args.profile_id, error),
    };
    let receipt = receipt_store.record(&request, &result).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to persist Lean check receipt: {error}"),
        )
    })?;
    Ok(json!({
        "status": ToolStatus::Ok,
        "check": result,
        "receipt": receipt,
    }))
}

fn resolve_profile(
    workspace: &McpWorkspace,
    profile_id: &str,
) -> Result<ProfileResolution, ToolError> {
    let store = match workspace.lean_profile_store() {
        Ok(store) => store,
        Err(error) => {
            return Ok(ProfileResolution::Capability(capability(
                profile_id,
                "lean.profile",
                "profile_store_unavailable",
                error.to_string(),
            )));
        }
    };
    match store.open(profile_id) {
        Ok(profile) if profile.probe().is_ready() => Ok(ProfileResolution::Ready {
            store,
            profile: Box::new(profile),
        }),
        Ok(profile) => Ok(ProfileResolution::Capability(capability(
            profile_id,
            "lean.profile",
            "profile_not_ready",
            format!("Lean profile is not ready: {:?}", profile.probe().state),
        ))),
        Err(mathscape_tool_lean::LeanProfileError::InvalidId(id)) => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("invalid Lean profile id {id:?}"),
        )),
        Err(error) => Ok(ProfileResolution::Capability(capability(
            profile_id,
            "lean.profile",
            "profile_unavailable",
            error.to_string(),
        ))),
    }
}

fn validate_profile_id_field(profile_id: &str) -> Result<(), ToolError> {
    if profile_id.trim().is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "profile_id must not be blank",
        ));
    }
    Ok(())
}

fn validate_expected_fingerprint(
    expected: Option<&str>,
    profile: &ManagedLeanProfile,
) -> Result<(), ToolError> {
    if let Some(expected) = expected {
        if expected.trim().is_empty() {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "profile_fingerprint must not be blank",
            ));
        }
        if expected != profile.fingerprint().digest {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "profile_fingerprint does not match the selected profile",
            )
            .with_details(json!({
                "expected": expected,
                "actual": profile.fingerprint().digest,
            })));
        }
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), ToolError> {
    if query.trim().is_empty() || query.chars().any(char::is_control) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "query must be non-empty and contain no control characters",
        ));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("query must be at most {MAX_QUERY_CHARS} characters"),
        ));
    }
    Ok(())
}

fn validate_module(module: &str) -> Result<(), ToolError> {
    if module.is_empty()
        || !module.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(index, byte)| {
                    if index == 0 {
                        byte.is_ascii_alphabetic() || byte == b'_'
                    } else {
                        byte.is_ascii_alphanumeric() || byte == b'_'
                    }
                })
        })
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "module must be a non-empty Lean module name",
        ));
    }
    Ok(())
}

fn profile_reference(profile: &ManagedLeanProfile) -> LeanProfileRef {
    LeanProfileRef {
        id: profile.metadata().id.clone(),
        fingerprint: profile.fingerprint().digest.clone(),
        toolchain: profile.metadata().lean_toolchain.clone(),
        mathlib_revision: profile
            .metadata()
            .detected_mathlib_revision
            .clone()
            .unwrap_or_else(|| profile.metadata().mathlib_reference.clone()),
    }
}

fn capability(
    profile_id: &str,
    capability_name: &str,
    code: &str,
    reason: impl Into<String>,
) -> Value {
    let reason = reason.into();
    json!({
        "status": ToolStatus::CapabilityUnavailable,
        "profile_id": profile_id,
        "diagnostics": [diagnostic(code, reason.clone())],
        "details": {
            "capability": capability_name,
            "reason": reason,
        },
    })
}

fn map_check_error(profile_id: &str, error: LeanCheckError) -> Result<Value, ToolError> {
    match error {
        LeanCheckError::CapabilityUnavailable { reason } => Ok(capability(
            profile_id,
            "lean.check",
            "lean_check_unavailable",
            reason,
        )),
        LeanCheckError::InvalidOption { field, reason } => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("{field}: {reason}"),
        )),
        LeanCheckError::InvalidSource { reason } => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("invalid source_text: {reason}"),
        )),
        LeanCheckError::ProfileFingerprintMismatch { expected, actual } => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "profile_fingerprint does not match the selected profile",
        )
        .with_details(json!({ "expected": expected, "actual": actual }))),
        LeanCheckError::OutputTruncated => Err(ToolError::new(
            ToolErrorCode::Internal,
            "Lean check output exceeded the capture limit",
        )),
        LeanCheckError::Runner(error) => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("Lean check runner failed: {error}"),
        )),
        _ => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("Lean check failed: {error}"),
        )),
    }
}

fn map_loogle_constructor_error(error: LeanLoogleError) -> ToolError {
    match error {
        LeanLoogleError::Access { path, source } => ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to access managed Loogle executable {path:?}: {source}"),
        ),
        _ => ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to initialize Loogle: {error}"),
        ),
    }
}

fn map_loogle_error(profile_id: &str, error: LeanLoogleError) -> Result<Value, ToolError> {
    match error {
        LeanLoogleError::CapabilityUnavailable { reason } => Ok(capability(
            profile_id,
            "lean.loogle",
            "loogle_unavailable",
            reason,
        )),
        LeanLoogleError::InvalidQuery(query) => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("invalid Loogle query: {query}"),
        )),
        LeanLoogleError::QueryRejected {
            message,
            suggestions,
        } => {
            let mut text = format!("Loogle rejected the query: {message}");
            if !suggestions.is_empty() {
                text.push_str("; did you mean: ");
                text.push_str(&suggestions.join(", "));
            }
            text.push_str(
                ". Bare identifiers must name an existing constant (use the fully qualified \
                 name), or quote a name fragment like \"klDiv\" for substring search.",
            );
            Err(ToolError::new(ToolErrorCode::InvalidArg, text))
        }
        LeanLoogleError::Access { path, source } => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to access Loogle path {path:?}: {source}"),
        )),
        LeanLoogleError::RunFailed { status, stderr } => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("Loogle failed with {status}: {stderr}"),
        )),
        LeanLoogleError::OutputTruncated => Err(ToolError::new(
            ToolErrorCode::Internal,
            "Loogle output exceeded the capture limit",
        )),
        LeanLoogleError::InvalidJson(error) => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("invalid Loogle JSON: {error}"),
        )),
        LeanLoogleError::Runner(error) => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("Loogle runner failed: {error}"),
        )),
        _ => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("Loogle search failed: {error}"),
        )),
    }
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn default_module() -> String {
    "Mathlib".to_owned()
}

const fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}
