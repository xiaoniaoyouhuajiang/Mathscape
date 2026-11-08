use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    LEAN_CHECK_CONTRACT_VERSION, LeanCancellation, LeanCheckRequest, LeanCheckResult,
    LeanCheckStatus, LeanChecker, LeanDiagnostic, LeanEvidenceRef, LeanProfileRef,
    LeanReceiptError, ManagedLeanProfile,
};

pub const LEAN_RECEIPT_SCHEMA_VERSION: &str = "lean_check_receipt v0";
pub const LEAN_RUNNER_SCHEMA_VERSION: &str = "lean_runner v0";

const RECEIPT_DIRECTORY: &str = "lean-receipts";
const SOURCE_DIRECTORY: &str = "sources";
const RECORD_DIRECTORY: &str = "receipts";
const SOURCE_PREFIX: &str = "mathscape:artifact/sha256/";
const RECEIPT_PREFIX: &str = "mathscape:lean-receipt/";
static RECEIPT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The immutable facts about one Lean process invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanRunnerReceipt {
    pub backend: String,
    pub runner_schema: String,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

/// An immutable record of one check against one exact source file and profile.
///
/// A receipt is an evidence record, not a trust-state transition. In
/// particular, a checked result does not claim that a Markdown or handwriting
/// claim was faithfully formalized by the submitted Lean source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanCheckReceipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub request_digest: String,
    pub source_artifact_id: String,
    pub source_sha256: String,
    pub profile: LeanProfileRef,
    pub check_status: LeanCheckStatus,
    pub diagnostics: Vec<LeanDiagnostic>,
    pub runner: LeanRunnerReceipt,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<LeanEvidenceRef>,
    pub created_at: String,
}

impl LeanCheckReceipt {
    pub const fn is_checked(&self) -> bool {
        matches!(self.check_status, LeanCheckStatus::Checked)
    }
}

/// A managed, profile-local receipt store. It stores the exact source bytes
/// alongside receipts so that a hash alone never becomes a false claim of
/// reproducibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanReceiptStore {
    root: PathBuf,
}

impl LeanReceiptStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, LeanReceiptError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(LeanReceiptError::RelativeRoot(root));
        }
        let receipt_root = root.join(RECEIPT_DIRECTORY);
        fs::create_dir_all(receipt_root.join(SOURCE_DIRECTORY)).map_err(|source| {
            LeanReceiptError::Access {
                path: receipt_root.join(SOURCE_DIRECTORY),
                source,
            }
        })?;
        fs::create_dir_all(receipt_root.join(RECORD_DIRECTORY)).map_err(|source| {
            LeanReceiptError::Access {
                path: receipt_root.join(RECORD_DIRECTORY),
                source,
            }
        })?;
        ensure_directory(&receipt_root)?;
        ensure_directory(&receipt_root.join(SOURCE_DIRECTORY))?;
        ensure_directory(&receipt_root.join(RECORD_DIRECTORY))?;
        Ok(Self { root })
    }

    pub fn for_profile(profile: &ManagedLeanProfile) -> Result<Self, LeanReceiptError> {
        let cache_root = profile
            .fingerprint()
            .cache_root
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| LeanReceiptError::CapabilityUnavailable {
                reason: "managed profile cache root is unavailable".to_owned(),
            })?;
        Self::new(cache_root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Persists the source and a receipt for a completed check result.
    /// Failed, timed-out and cancelled checks are valid receipts too.
    pub fn record(
        &self,
        request: &LeanCheckRequest,
        result: &LeanCheckResult,
    ) -> Result<LeanCheckReceipt, LeanReceiptError> {
        request.validate()?;
        if result.contract_version != LEAN_CHECK_CONTRACT_VERSION {
            return Err(LeanReceiptError::InconsistentResult {
                reason: "result contract version is not supported",
            });
        }
        let source_sha256 = sha256(request.source_text.as_bytes());
        if source_sha256 != result.source_sha256 {
            return Err(LeanReceiptError::InconsistentResult {
                reason: "result source hash does not match request source",
            });
        }
        if let Some(expected) = request.profile_fingerprint.as_deref()
            && expected != result.profile.fingerprint
        {
            return Err(LeanReceiptError::InconsistentResult {
                reason: "result profile fingerprint does not match request expectation",
            });
        }

        let source_artifact_id = format!("{SOURCE_PREFIX}{}", digest_suffix(&source_sha256));
        let source_path = self.source_path(&source_artifact_id)?;
        write_immutable(&source_path, request.source_text.as_bytes())?;

        let request_digest = request_digest(request, &result.profile.fingerprint);
        let (created_at, receipt_id) = next_receipt_identity();
        let receipt = LeanCheckReceipt {
            schema_version: LEAN_RECEIPT_SCHEMA_VERSION.to_owned(),
            receipt_id: receipt_id.clone(),
            request_digest,
            source_artifact_id,
            source_sha256,
            profile: result.profile.clone(),
            check_status: result.status,
            diagnostics: result.diagnostics.clone(),
            runner: LeanRunnerReceipt {
                backend: "lean".to_owned(),
                runner_schema: LEAN_RUNNER_SCHEMA_VERSION.to_owned(),
                timeout_ms: request.timeout_ms,
                max_output_bytes: request.max_output_bytes,
                exit_code: result.exit_code,
                elapsed_ms: result.elapsed_ms,
            },
            evidence_refs: request.evidence_refs.clone(),
            created_at,
        };
        let record_path = self.receipt_path(&receipt_id)?;
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|source| {
            LeanReceiptError::InvalidJson {
                path: record_path.clone(),
                source,
            }
        })?;
        write_new(&record_path, &bytes)?;
        Ok(receipt)
    }

    pub fn load(&self, receipt_id: &str) -> Result<LeanCheckReceipt, LeanReceiptError> {
        let path = self.receipt_path(receipt_id)?;
        let bytes = fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                LeanReceiptError::NotFound {
                    receipt_id: receipt_id.to_owned(),
                }
            } else {
                LeanReceiptError::Read {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let receipt: LeanCheckReceipt =
            serde_json::from_slice(&bytes).map_err(|source| LeanReceiptError::InvalidJson {
                path: path.clone(),
                source,
            })?;
        if receipt.receipt_id != receipt_id {
            return Err(LeanReceiptError::Integrity {
                path,
                reason: "receipt id does not match its storage path".to_owned(),
            });
        }
        self.verify(&receipt)?;
        Ok(receipt)
    }

    pub fn verify(&self, receipt: &LeanCheckReceipt) -> Result<(), LeanReceiptError> {
        if receipt.schema_version != LEAN_RECEIPT_SCHEMA_VERSION {
            return Err(LeanReceiptError::Integrity {
                path: self.receipt_path(&receipt.receipt_id)?,
                reason: format!("unsupported receipt schema {:?}", receipt.schema_version),
            });
        }
        let expected_source_id =
            format!("{SOURCE_PREFIX}{}", digest_suffix(&receipt.source_sha256));
        if receipt.source_artifact_id != expected_source_id {
            return Err(LeanReceiptError::Integrity {
                path: self.receipt_path(&receipt.receipt_id)?,
                reason: "source artifact id does not match source hash".to_owned(),
            });
        }
        let expected_request_digest = request_digest_for_values(
            &receipt.source_sha256,
            &receipt.profile.fingerprint,
            receipt.runner.timeout_ms,
            receipt.runner.max_output_bytes,
        );
        if receipt.request_digest != expected_request_digest {
            return Err(LeanReceiptError::Integrity {
                path: self.receipt_path(&receipt.receipt_id)?,
                reason: "request digest does not match receipt facts".to_owned(),
            });
        }
        let source_path = self.source_path(&receipt.source_artifact_id)?;
        let bytes = fs::read(&source_path).map_err(|source| LeanReceiptError::Read {
            path: source_path.clone(),
            source,
        })?;
        let actual = sha256(&bytes);
        if actual != receipt.source_sha256 {
            return Err(LeanReceiptError::Integrity {
                path: source_path,
                reason: format!(
                    "source hash mismatch: expected {:?}, actual {:?}",
                    receipt.source_sha256, actual
                ),
            });
        }
        Ok(())
    }

    pub fn read_source(&self, source_artifact_id: &str) -> Result<Vec<u8>, LeanReceiptError> {
        let path = self.source_path(source_artifact_id)?;
        fs::read(&path).map_err(|source| LeanReceiptError::Read { path, source })
    }

    fn source_path(&self, source_artifact_id: &str) -> Result<PathBuf, LeanReceiptError> {
        let suffix = source_artifact_suffix(source_artifact_id)?;
        Ok(self
            .root
            .join(RECEIPT_DIRECTORY)
            .join(SOURCE_DIRECTORY)
            .join(format!("{suffix}.lean")))
    }

    fn receipt_path(&self, receipt_id: &str) -> Result<PathBuf, LeanReceiptError> {
        let suffix = receipt_id_suffix(receipt_id)?;
        Ok(self
            .root
            .join(RECEIPT_DIRECTORY)
            .join(RECORD_DIRECTORY)
            .join(format!("{suffix}.json")))
    }
}

/// Convenience facade for the common check-then-record operation.
#[derive(Clone, Debug)]
pub struct LeanCheckService {
    checker: LeanChecker,
    receipts: LeanReceiptStore,
}

impl LeanCheckService {
    pub fn new(receipts: LeanReceiptStore) -> Self {
        Self {
            checker: LeanChecker::new(),
            receipts,
        }
    }

    pub fn receipts(&self) -> &LeanReceiptStore {
        &self.receipts
    }

    pub fn check_and_record(
        &self,
        profile: &ManagedLeanProfile,
        request: &LeanCheckRequest,
        cancellation: &LeanCancellation,
    ) -> Result<(LeanCheckResult, LeanCheckReceipt), LeanReceiptError> {
        let result = self.checker.check(profile, request, cancellation)?;
        let receipt = self.receipts.record(request, &result)?;
        Ok((result, receipt))
    }
}

fn ensure_directory(path: &Path) -> Result<(), LeanReceiptError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| LeanReceiptError::Access {
        path: path.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LeanReceiptError::Access {
            path: path.to_owned(),
            source: std::io::Error::other("receipt path is not a regular directory"),
        });
    }
    Ok(())
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), LeanReceiptError> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
        return Err(LeanReceiptError::Integrity {
            path: path.to_owned(),
            reason: "content-addressed source already contains different bytes".to_owned(),
        });
    }
    write_new(path, bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), LeanReceiptError> {
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        RECEIPT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, bytes).map_err(|source| LeanReceiptError::Write {
        path: temporary.clone(),
        source,
    })?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            let existing = fs::read(path).map_err(|read_source| LeanReceiptError::Read {
                path: path.to_owned(),
                source: read_source,
            })?;
            if existing == bytes {
                Ok(())
            } else {
                Err(LeanReceiptError::Integrity {
                    path: path.to_owned(),
                    reason: "immutable receipt path already contains different bytes".to_owned(),
                })
            }
        }
        Err(source) => {
            let _ = fs::remove_file(&temporary);
            Err(LeanReceiptError::Write {
                path: path.to_owned(),
                source,
            })
        }
    }
}

fn source_artifact_suffix(id: &str) -> Result<&str, LeanReceiptError> {
    let suffix = id
        .strip_prefix(SOURCE_PREFIX)
        .filter(|suffix| is_digest_suffix(suffix))
        .ok_or_else(|| LeanReceiptError::InvalidSourceArtifactId(id.to_owned()))?;
    Ok(suffix)
}

fn receipt_id_suffix(id: &str) -> Result<&str, LeanReceiptError> {
    let suffix = id
        .strip_prefix(RECEIPT_PREFIX)
        .filter(|suffix| {
            !suffix.is_empty()
                && suffix.len() <= 160
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .ok_or_else(|| LeanReceiptError::InvalidReceiptId(id.to_owned()))?;
    Ok(suffix)
}

fn is_digest_suffix(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn digest_suffix(value: &str) -> &str {
    value.strip_prefix("sha256:").unwrap_or(value)
}

#[derive(Serialize)]
struct RequestDigestInput<'a> {
    contract_version: &'static str,
    source_sha256: &'a str,
    profile_fingerprint: &'a str,
    timeout_ms: u64,
    max_output_bytes: usize,
}

fn request_digest(request: &LeanCheckRequest, profile_fingerprint: &str) -> String {
    let source_sha256 = sha256(request.source_text.as_bytes());
    request_digest_for_values(
        &source_sha256,
        profile_fingerprint,
        request.timeout_ms,
        request.max_output_bytes,
    )
}

fn request_digest_for_values(
    source_sha256: &str,
    profile_fingerprint: &str,
    timeout_ms: u64,
    max_output_bytes: usize,
) -> String {
    let input = RequestDigestInput {
        contract_version: LEAN_CHECK_CONTRACT_VERSION,
        source_sha256,
        profile_fingerprint,
        timeout_ms,
        max_output_bytes,
    };
    let bytes = serde_json::to_vec(&input).expect("request digest input is serializable");
    sha256(&bytes)
}

fn next_receipt_identity() -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let sequence = RECEIPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = now
        .as_secs()
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::from(now.subsec_nanos()));
    let suffix = format!("{nanos}-{}-{sequence}", std::process::id());
    (format_rfc3339(now), format!("{RECEIPT_PREFIX}{suffix}"))
}

fn format_rfc3339(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:09}Z",
        duration.subsec_nanos()
    )
}

// Howard Hinnant's civil-from-days conversion, expressed without an external
// time crate so receipt serialization stays deterministic and lightweight.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::TempDir;

    use super::*;

    fn request() -> LeanCheckRequest {
        LeanCheckRequest::new("import Mathlib\n\nexample : True := by trivial\n").unwrap()
    }

    fn result(request: &LeanCheckRequest) -> LeanCheckResult {
        LeanCheckResult {
            contract_version: LEAN_CHECK_CONTRACT_VERSION.to_owned(),
            status: LeanCheckStatus::Checked,
            profile: LeanProfileRef {
                id: "lean4-test".to_owned(),
                fingerprint: "sha256:profile".to_owned(),
                toolchain: "leanprover/lean4:v4.33.1".to_owned(),
                mathlib_revision: "v4.33.1".to_owned(),
            },
            source_sha256: sha256(request.source_text.as_bytes()),
            diagnostics: Vec::new(),
            exit_code: Some(0),
            elapsed_ms: 42,
            diagnostics_truncated: false,
        }
    }

    #[test]
    fn records_loads_and_verifies_an_immutable_receipt() {
        let temp = TempDir::new().unwrap();
        let store = LeanReceiptStore::new(temp.path()).unwrap();
        let request = request();
        let receipt = store.record(&request, &result(&request)).unwrap();
        assert!(receipt.is_checked());
        assert!(receipt.receipt_id.starts_with(RECEIPT_PREFIX));
        assert!(receipt.source_artifact_id.starts_with(SOURCE_PREFIX));
        assert_eq!(store.load(&receipt.receipt_id).unwrap(), receipt);
        assert_eq!(
            store.read_source(&receipt.source_artifact_id).unwrap(),
            request.source_text.as_bytes()
        );
    }

    #[test]
    fn request_digest_is_stable_but_receipt_ids_are_unique() {
        let temp = TempDir::new().unwrap();
        let store = LeanReceiptStore::new(temp.path()).unwrap();
        let request = request();
        let first = store.record(&request, &result(&request)).unwrap();
        let second = store.record(&request, &result(&request)).unwrap();
        assert_eq!(first.request_digest, second.request_digest);
        assert_ne!(first.receipt_id, second.receipt_id);
    }

    #[test]
    fn detects_tampered_source_bytes() {
        let temp = TempDir::new().unwrap();
        let store = LeanReceiptStore::new(temp.path()).unwrap();
        let request = request();
        let receipt = store.record(&request, &result(&request)).unwrap();
        let source_path = store
            .source_path(&receipt.source_artifact_id)
            .expect("source path");
        fs::write(source_path, b"tampered").unwrap();
        assert!(matches!(
            store.load(&receipt.receipt_id),
            Err(LeanReceiptError::Integrity { .. })
        ));
    }

    #[test]
    fn rejects_path_traversal_receipt_ids() {
        let temp = TempDir::new().unwrap();
        let store = LeanReceiptStore::new(temp.path()).unwrap();
        assert!(matches!(
            store.load("mathscape:lean-receipt/../escape"),
            Err(LeanReceiptError::InvalidReceiptId(_))
        ));
    }

    #[test]
    fn formats_epoch_and_known_date_as_rfc3339() {
        assert_eq!(
            format_rfc3339(Duration::ZERO),
            "1970-01-01T00:00:00.000000000Z"
        );
        assert_eq!(
            civil_from_days(20_000),
            (2024, 10, 4),
            "civil date conversion should remain stable"
        );
    }
}
