//! The bounded, round-scoped file staging tool.
//!
//! This is deliberately separate from ArtifactPool: staged bytes and records
//! live under workspace/staging/agent-artifacts until a user accepts a
//! proposal that promotes them.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::Utc;
use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{ARTIFACT_STAGE, diagnostic};
use crate::McpWorkspace;

const SCHEMA_VERSION: &str = "artifact_stage v0";
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_NAME_CHARS: usize = 200;
const MAX_REQUEST_ID_CHARS: usize = 128;
const MAX_REFERENCE_ID_CHARS: usize = 256;
const OBJECTS_DIR: &str = "objects/sha256";
const RECORDS_DIR: &str = "records";

const MEDIA_TYPES: [&str; 3] = ["text/markdown", "text/x-lean", "text/plain"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StageArgs {
    request_id: String,
    media_type: String,
    name: String,
    content: String,
    #[serde(default)]
    evidence_refs: Vec<Value>,
    #[serde(default)]
    validation_refs: Vec<ValidationRef>,
    #[serde(default)]
    supersedes: Option<Supersedes>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ValidationRef {
    validator: String,
    receipt_id: String,
    #[serde(default)]
    profile_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Supersedes {
    staged_artifact_id: String,
    content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ValidationSummary {
    validator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receipt_id: Option<String>,
    state: String,
    source_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StageRecord {
    schema_version: String,
    status: String,
    staged_artifact_id: String,
    request_id: String,
    request_fingerprint: String,
    content_hash: String,
    media_type: String,
    name: String,
    byte_size: usize,
    proposed_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    round_id: Option<String>,
    visibility: String,
    created_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence_refs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    validation_refs: Vec<ValidationRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    validation: Vec<ValidationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supersedes: Option<Supersedes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    promoted_artifact_id: Option<String>,
}

pub(super) fn definition() -> Value {
    json!({
        "name": ARTIFACT_STAGE,
        "description": "Stage one UTF-8 Markdown, Lean or plain-text file in the current round's isolated Mathscape staging store. It creates an immutable staged snapshot with provenance and optional validation references; it never writes the ArtifactPool, map or accepted graph. Requires a bound round token.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "request_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_REQUEST_ID_CHARS,
                    "description": "Idempotency key for this staging request.",
                },
                "media_type": {
                    "type": "string",
                    "enum": MEDIA_TYPES,
                },
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_NAME_CHARS,
                    "description": "Logical display filename; never a filesystem path.",
                },
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_CONTENT_BYTES,
                },
                "evidence_refs": {
                    "type": "array",
                    "items": { "type": "object" },
                },
                "validation_refs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "validator": { "type": "string" },
                            "receipt_id": { "type": "string" },
                            "profile_id": { "type": "string" },
                        },
                        "required": ["validator", "receipt_id"],
                        "additionalProperties": false,
                    },
                },
                "supersedes": {
                    "type": "object",
                    "properties": {
                        "staged_artifact_id": { "type": "string" },
                        "content_hash": { "type": "string" },
                    },
                    "required": ["staged_artifact_id", "content_hash"],
                    "additionalProperties": false,
                },
            },
            "required": ["request_id", "media_type", "name", "content"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: StageArgs = super::parse_args(arguments)?;
    let round = workspace
        .round()
        .filter(|round| !round.trim().is_empty())
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Forbidden,
                "artifact.stage requires a bound round token",
            )
            .with_details(json!({ "reason": "round_not_bound" }))
        })?;
    validate_args(&args)?;

    let store = StageStore::new(workspace.path()).map_err(internal)?;
    let request_fingerprint = request_fingerprint(&args)?;
    let round = round.as_str();
    let staged_artifact_id = staged_id(round, &args.request_id);
    if let Some(existing) = store.load_record(&staged_artifact_id).map_err(internal)? {
        if existing.request_fingerprint == request_fingerprint {
            return Ok(output(existing));
        }
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "request_id was already used with different staging content",
        )
        .with_details(json!({ "reason": "idempotency_conflict" })));
    }

    let content_hash = sha256(args.content.as_bytes());
    let mut validation = Vec::new();
    let mut status = "staged".to_owned();
    for reference in &args.validation_refs {
        let summary = validate_reference(workspace, &args.media_type, reference, &content_hash)?;
        if summary.state != "passed" {
            status = "blocked".to_owned();
        }
        validation.push(summary);
    }

    if let Some(supersedes) = &args.supersedes {
        let old = store
            .load_record(&supersedes.staged_artifact_id)
            .map_err(internal)?
            .ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::InvalidArg,
                    "supersedes.staged_artifact_id was not found",
                )
            })?;
        if old.content_hash != supersedes.content_hash
            || old.round_id.as_deref() != Some(round)
            || !matches!(old.status.as_str(), "staged" | "blocked")
        {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "superseded staged artifact is stale or not replaceable",
            )
            .with_details(json!({ "reason": "supersede_conflict" })));
        }
    }

    let record = StageRecord {
        schema_version: SCHEMA_VERSION.to_owned(),
        status,
        staged_artifact_id: staged_artifact_id.clone(),
        request_id: args.request_id.clone(),
        request_fingerprint,
        content_hash: content_hash.clone(),
        media_type: args.media_type.clone(),
        name: args.name.clone(),
        byte_size: args.content.len(),
        proposed_by: "agent:custom".to_owned(),
        round_id: Some(round.to_owned()),
        visibility: "investigation".to_owned(),
        created_at: Utc::now().to_rfc3339(),
        evidence_refs: args.evidence_refs.clone(),
        validation_refs: args.validation_refs.clone(),
        validation,
        supersedes: args.supersedes.clone(),
        promoted_artifact_id: None,
    };
    store
        .write_content(&content_hash, args.content.as_bytes())
        .map_err(internal)?;
    store.write_record(&record).map_err(internal)?;

    if let Some(supersedes) = &args.supersedes
        && let Some(mut old) = store
            .load_record(&supersedes.staged_artifact_id)
            .map_err(internal)?
    {
        old.status = "superseded".to_owned();
        store.write_record(&old).map_err(internal)?;
    }
    Ok(output(record))
}

fn validate_args(args: &StageArgs) -> Result<(), ToolError> {
    if args.request_id.trim().is_empty()
        || args.request_id.chars().count() > MAX_REQUEST_ID_CHARS
        || args.request_id.chars().any(char::is_control)
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "request_id must be non-empty, bounded and contain no control characters",
        ));
    }
    if !MEDIA_TYPES.contains(&args.media_type.as_str()) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "media_type is not supported by artifact.stage v0",
        ));
    }
    if args.name.trim().is_empty()
        || args.name.chars().count() > MAX_NAME_CHARS
        || args.name.chars().any(char::is_control)
        || args.name.contains('/')
        || args.name.contains('\\')
        || args.name.contains("..")
        || Path::new(&args.name).is_absolute()
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "name must be a logical filename, not a path",
        ));
    }
    if args.content.is_empty() || args.content.len() > MAX_CONTENT_BYTES {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "content must be between 1 byte and 1 MiB",
        ));
    }
    for reference in &args.evidence_refs {
        let node_id = reference
            .get("node_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::InvalidArg,
                    "every evidence_ref needs a non-empty node_id",
                )
            })?;
        if node_id.chars().any(char::is_control) {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "evidence_ref node_id contains a control character",
            ));
        }
    }
    for reference in &args.validation_refs {
        if reference.validator.trim().is_empty()
            || reference.validator.chars().count() > MAX_REFERENCE_ID_CHARS
            || reference.validator.chars().any(char::is_control)
            || reference.receipt_id.trim().is_empty()
            || reference.receipt_id.chars().count() > MAX_REFERENCE_ID_CHARS
            || reference.receipt_id.chars().any(char::is_control)
            || reference.profile_id.as_deref().is_some_and(|value| {
                value.trim().is_empty()
                    || value.chars().count() > MAX_REFERENCE_ID_CHARS
                    || value.chars().any(char::is_control)
            })
        {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "validation references must contain bounded, non-empty identifiers",
            ));
        }
    }
    if let Some(supersedes) = &args.supersedes
        && (!is_valid_staged_id(&supersedes.staged_artifact_id)
            || !is_valid_sha256(&supersedes.content_hash))
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "supersedes must contain a staged artifact id and sha256 content hash",
        ));
    }
    Ok(())
}

fn validate_reference(
    workspace: &McpWorkspace,
    media_type: &str,
    reference: &ValidationRef,
    content_hash: &str,
) -> Result<ValidationSummary, ToolError> {
    if reference.validator != "lean.check" {
        return Ok(ValidationSummary {
            validator: reference.validator.clone(),
            receipt_id: Some(reference.receipt_id.clone()),
            state: "not_run".to_owned(),
            source_hash: content_hash.to_owned(),
            profile_fingerprint: None,
            diagnostics: vec![diagnostic(
                "validator_not_available",
                "this validator is not registered for artifact.stage v0",
            )],
        });
    }
    if media_type != "text/x-lean" {
        return Ok(ValidationSummary {
            validator: reference.validator.clone(),
            receipt_id: Some(reference.receipt_id.clone()),
            state: "not_run".to_owned(),
            source_hash: content_hash.to_owned(),
            profile_fingerprint: None,
            diagnostics: vec![diagnostic(
                "validator_media_type_mismatch",
                "lean.check validation requires media_type text/x-lean",
            )],
        });
    }
    let Some(profile_id) = reference.profile_id.as_deref() else {
        return Ok(ValidationSummary {
            validator: reference.validator.clone(),
            receipt_id: Some(reference.receipt_id.clone()),
            state: "not_run".to_owned(),
            source_hash: content_hash.to_owned(),
            profile_fingerprint: None,
            diagnostics: vec![diagnostic(
                "profile_id_required",
                "profile_id is required to verify a lean.check receipt",
            )],
        });
    };
    let store = workspace.lean_profile_store()?;
    let profile = store.open(profile_id).map_err(internal)?;
    let receipt_store =
        mathscape_tool_lean::LeanReceiptStore::for_profile(&profile).map_err(internal)?;
    let receipt = receipt_store
        .load(&reference.receipt_id)
        .map_err(internal)?;
    if receipt.source_sha256 != content_hash {
        return Ok(ValidationSummary {
            validator: reference.validator.clone(),
            receipt_id: Some(reference.receipt_id.clone()),
            state: "failed".to_owned(),
            source_hash: receipt.source_sha256,
            profile_fingerprint: Some(receipt.profile.fingerprint),
            diagnostics: vec![diagnostic(
                "source_hash_mismatch",
                "lean.check receipt does not refer to this staged content",
            )],
        });
    }
    if receipt.profile.fingerprint != profile.fingerprint().digest {
        return Ok(ValidationSummary {
            validator: reference.validator.clone(),
            receipt_id: Some(reference.receipt_id.clone()),
            state: "failed".to_owned(),
            source_hash: receipt.source_sha256,
            profile_fingerprint: Some(receipt.profile.fingerprint),
            diagnostics: vec![diagnostic(
                "profile_fingerprint_mismatch",
                "lean.check receipt was produced by a different profile fingerprint",
            )],
        });
    }
    let state = if receipt.is_checked() {
        "passed"
    } else {
        "failed"
    };
    Ok(ValidationSummary {
        validator: reference.validator.clone(),
        receipt_id: Some(reference.receipt_id.clone()),
        state: state.to_owned(),
        source_hash: receipt.source_sha256,
        profile_fingerprint: Some(receipt.profile.fingerprint),
        diagnostics: serde_json::to_value(receipt.diagnostics)
            .map(|value| value.as_array().cloned().unwrap_or_default())
            .map_err(internal)?,
    })
}

fn output(record: StageRecord) -> Value {
    json!({
        "status": ToolStatus::Ok,
        "stage": record,
    })
}

fn internal(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(ToolErrorCode::Internal, error.to_string())
}

#[derive(Clone, Debug)]
struct StageStore {
    root: PathBuf,
}

impl StageStore {
    fn new(workspace: &Path) -> io::Result<Self> {
        let root = workspace.join("staging/agent-artifacts");
        fs::create_dir_all(root.join(OBJECTS_DIR))?;
        fs::create_dir_all(root.join(RECORDS_DIR))?;
        ensure_directory(&root)?;
        ensure_directory(&root.join(OBJECTS_DIR))?;
        ensure_directory(&root.join(RECORDS_DIR))?;
        Ok(Self { root })
    }

    fn object_path(&self, content_hash: &str) -> io::Result<PathBuf> {
        let suffix = digest_suffix(content_hash)?;
        Ok(self.root.join(OBJECTS_DIR).join(suffix))
    }

    fn record_path(&self, staged_id: &str) -> io::Result<PathBuf> {
        let suffix = staged_id
            .strip_prefix("mathscape:staged-artifact/")
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid staged id"))?;
        Ok(self.root.join(RECORDS_DIR).join(format!("{suffix}.json")))
    }

    fn load_record(&self, staged_id: &str) -> io::Result<Option<StageRecord>> {
        let path = self.record_path(staged_id)?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_content(&self, content_hash: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.object_path(content_hash)?;
        if let Ok(existing) = fs::read(&path) {
            if existing == bytes {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "content hash collision in staging store",
            ));
        }
        atomic_write(&path, bytes)
    }

    fn write_record(&self, record: &StageRecord) -> io::Result<()> {
        let path = self.record_path(&record.staged_artifact_id)?;
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        atomic_write(&path, &bytes)
    }
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::other("staging path is not a regular directory"));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn request_fingerprint(args: &StageArgs) -> Result<String, ToolError> {
    let bytes = serde_json::to_vec(args).map_err(internal)?;
    Ok(sha256(&bytes))
}

fn staged_id(round: &str, request_id: &str) -> String {
    format!(
        "mathscape:staged-artifact/{}",
        digest_bytes(format!("{round}\0{request_id}").as_bytes()).trim_start_matches("sha256:")
    )
}

fn sha256(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        digest_bytes(bytes).trim_start_matches("sha256:")
    )
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn digest_suffix(content_hash: &str) -> io::Result<&str> {
    if !is_valid_sha256(content_hash) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid sha256 content hash",
        ));
    }
    Ok(content_hash.trim_start_matches("sha256:"))
}

fn is_valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|suffix| {
        suffix.len() == 64 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn is_valid_staged_id(value: &str) -> bool {
    value
        .strip_prefix("mathscape:staged-artifact/")
        .is_some_and(|suffix| {
            suffix.len() == 64 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}
