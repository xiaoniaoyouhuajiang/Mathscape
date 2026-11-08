use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    LeanCancellation, LeanCheckError, LeanCommand, LeanRunStatus, LeanRunner, LeanRunnerOptions,
    ManagedLeanProfile,
};

pub const LEAN_CHECK_CONTRACT_VERSION: &str = "lean_tool_contract v0";
pub const LEAN_CHECK_MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const LEAN_CHECK_DEFAULT_TIMEOUT_MS: u64 = 300_000;
pub const LEAN_CHECK_MIN_TIMEOUT_MS: u64 = 1_000;
pub const LEAN_CHECK_MAX_TIMEOUT_MS: u64 = 900_000;
pub const LEAN_CHECK_DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;
pub const LEAN_CHECK_MIN_OUTPUT_BYTES: usize = 32 * 1024;
pub const LEAN_CHECK_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

const CHECK_DIRECTORY: &str = "check";
static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A source or handwriting object that motivated a Lean check. The checker
/// only records this provenance; it never reads arbitrary paths through it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanEvidenceRef {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<serde_json::Value>,
}

impl LeanEvidenceRef {
    pub fn new(node_id: impl Into<String>) -> Result<Self, LeanCheckError> {
        let node_id = node_id.into();
        if node_id.trim().is_empty() {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.evidence_refs.node_id",
                reason: "must not be blank",
            });
        }
        Ok(Self {
            node_id,
            artifact_id: None,
            locator: None,
        })
    }

    pub fn artifact_id(mut self, artifact_id: impl Into<String>) -> Result<Self, LeanCheckError> {
        let artifact_id = artifact_id.into();
        if artifact_id.trim().is_empty() {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.evidence_refs.artifact_id",
                reason: "must not be blank",
            });
        }
        self.artifact_id = Some(artifact_id);
        Ok(self)
    }

    pub fn locator(mut self, locator: serde_json::Value) -> Self {
        self.locator = Some(locator);
        self
    }
}

/// A complete Lean source file and the bounded process policy for checking it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanCheckRequest {
    pub source_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<LeanEvidenceRef>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_fingerprint: Option<String>,
}

impl LeanCheckRequest {
    pub fn new(source_text: impl Into<String>) -> Result<Self, LeanCheckError> {
        let request = Self {
            source_text: source_text.into(),
            evidence_refs: Vec::new(),
            timeout_ms: LEAN_CHECK_DEFAULT_TIMEOUT_MS,
            max_output_bytes: LEAN_CHECK_DEFAULT_OUTPUT_BYTES,
            profile_fingerprint: None,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn evidence_refs<I>(mut self, refs: I) -> Result<Self, LeanCheckError>
    where
        I: IntoIterator<Item = LeanEvidenceRef>,
    {
        self.evidence_refs = refs.into_iter().collect();
        self.validate()?;
        Ok(self)
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Result<Self, LeanCheckError> {
        self.timeout_ms = timeout_ms;
        self.validate()?;
        Ok(self)
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Result<Self, LeanCheckError> {
        self.max_output_bytes = max_output_bytes;
        self.validate()?;
        Ok(self)
    }

    pub fn profile_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.profile_fingerprint = Some(fingerprint.into());
        self
    }

    pub fn validate(&self) -> Result<(), LeanCheckError> {
        if self.source_text.trim().is_empty() {
            return Err(LeanCheckError::InvalidSource {
                reason: "source_text must not be blank",
            });
        }
        if self.source_text.len() > LEAN_CHECK_MAX_SOURCE_BYTES {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.source_text",
                reason: "must be at most 1 MiB",
            });
        }
        if !(LEAN_CHECK_MIN_TIMEOUT_MS..=LEAN_CHECK_MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.timeout_ms",
                reason: "must be between 1000 and 900000",
            });
        }
        if !(LEAN_CHECK_MIN_OUTPUT_BYTES..=LEAN_CHECK_MAX_OUTPUT_BYTES)
            .contains(&self.max_output_bytes)
        {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.max_output_bytes",
                reason: "must be between 32768 and 16777216",
            });
        }
        if self
            .profile_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint.trim().is_empty())
        {
            return Err(LeanCheckError::InvalidOption {
                field: "lean.check.profile_fingerprint",
                reason: "must not be blank",
            });
        }
        for evidence in &self.evidence_refs {
            if evidence.node_id.trim().is_empty() {
                return Err(LeanCheckError::InvalidOption {
                    field: "lean.check.evidence_refs.node_id",
                    reason: "must not be blank",
                });
            }
            if evidence
                .artifact_id
                .as_deref()
                .is_some_and(|artifact_id| artifact_id.trim().is_empty())
            {
                return Err(LeanCheckError::InvalidOption {
                    field: "lean.check.evidence_refs.artifact_id",
                    reason: "must not be blank",
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanProfileRef {
    pub id: String,
    pub fingerprint: String,
    pub toolchain: String,
    pub mathlib_revision: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanCheckStatus {
    Checked,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanDiagnostic {
    pub severity: LeanDiagnosticSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanCheckResult {
    pub contract_version: String,
    pub status: LeanCheckStatus,
    pub profile: LeanProfileRef,
    pub source_sha256: String,
    pub diagnostics: Vec<LeanDiagnostic>,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub diagnostics_truncated: bool,
}

impl LeanCheckResult {
    pub const fn is_checked(&self) -> bool {
        matches!(self.status, LeanCheckStatus::Checked)
    }
}

#[derive(Clone, Debug, Default)]
pub struct LeanChecker;

impl LeanChecker {
    pub fn new() -> Self {
        Self
    }

    pub fn check(
        &self,
        profile: &ManagedLeanProfile,
        request: &LeanCheckRequest,
        cancellation: &LeanCancellation,
    ) -> Result<LeanCheckResult, LeanCheckError> {
        request.validate()?;
        if let Some(expected) = request.profile_fingerprint.as_deref()
            && expected != profile.fingerprint().digest
        {
            return Err(LeanCheckError::ProfileFingerprintMismatch {
                expected: expected.to_owned(),
                actual: profile.fingerprint().digest.clone(),
            });
        }
        if !profile.probe().is_ready() {
            return Err(LeanCheckError::CapabilityUnavailable {
                reason: format!("Lean profile is not ready: {:?}", profile.probe().state),
            });
        }
        let lake = profile.probe().lake_executable.as_deref().ok_or_else(|| {
            LeanCheckError::CapabilityUnavailable {
                reason: "Lake executable is unavailable".to_owned(),
            }
        })?;
        let cache_root = profile
            .fingerprint()
            .cache_root
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| LeanCheckError::CapabilityUnavailable {
                reason: "managed profile cache root is unavailable".to_owned(),
            })?;
        let scratch = Scratch::create(&cache_root, &request.source_text)?;
        let command = LeanCommand::new(lake.to_owned(), profile.metadata().project_root.clone())
            .args([
                "env".to_owned(),
                "lean".to_owned(),
                scratch.source.to_string_lossy().into_owned(),
            ]);
        let options = LeanRunnerOptions::new(Duration::from_millis(request.timeout_ms))
            .map_err(|_| LeanCheckError::InvalidOption {
                field: "lean.check.timeout_ms",
                reason: "must be greater than zero",
            })?
            .max_output_bytes(request.max_output_bytes)
            .map_err(|_| LeanCheckError::InvalidOption {
                field: "lean.check.max_output_bytes",
                reason: "must be greater than zero",
            })?;
        let run = LeanRunner::new(options).run(&command, cancellation);
        let cleanup = scratch.remove();
        let run = run?;
        cleanup?;

        if run.stdout().is_truncated() || run.stderr().is_truncated() {
            return Err(LeanCheckError::OutputTruncated);
        }

        let mut diagnostics = parse_diagnostics(&run.stderr().lossy());
        diagnostics.extend(parse_diagnostics(&run.stdout().lossy()));
        if diagnostics.is_empty() && run.status() == LeanRunStatus::Failed {
            let output = run.stderr().lossy().trim().to_owned();
            if !output.is_empty() {
                diagnostics.push(LeanDiagnostic {
                    severity: LeanDiagnosticSeverity::Error,
                    message: output,
                    file: None,
                    line: None,
                    column: None,
                    end_line: None,
                    end_column: None,
                });
            }
        }
        let status = match run.status() {
            LeanRunStatus::Succeeded
                if diagnostics
                    .iter()
                    .any(|item| matches!(item.severity, LeanDiagnosticSeverity::Error)) =>
            {
                LeanCheckStatus::Failed
            }
            LeanRunStatus::Succeeded => LeanCheckStatus::Checked,
            LeanRunStatus::Failed => LeanCheckStatus::Failed,
            LeanRunStatus::TimedOut => LeanCheckStatus::TimedOut,
            LeanRunStatus::Cancelled => LeanCheckStatus::Cancelled,
        };
        Ok(LeanCheckResult {
            contract_version: LEAN_CHECK_CONTRACT_VERSION.to_owned(),
            status,
            profile: profile_reference(profile),
            source_sha256: sha256(request.source_text.as_bytes()),
            diagnostics,
            exit_code: run.exit_code(),
            elapsed_ms: run.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            diagnostics_truncated: false,
        })
    }
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

fn default_timeout_ms() -> u64 {
    LEAN_CHECK_DEFAULT_TIMEOUT_MS
}

fn default_output_bytes() -> usize {
    LEAN_CHECK_DEFAULT_OUTPUT_BYTES
}

struct Scratch {
    directory: PathBuf,
    source: PathBuf,
}

impl Scratch {
    fn create(cache_root: &Path, source_text: &str) -> Result<Self, LeanCheckError> {
        let parent = cache_root.join(CHECK_DIRECTORY);
        fs::create_dir_all(&parent).map_err(|source| LeanCheckError::Access {
            path: parent.clone(),
            source,
        })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut directory = None;
        for attempt in 0..100_u32 {
            let candidate = parent.join(format!("run-{pid}-{stamp}-{sequence}-{attempt}"));
            match fs::create_dir(&candidate) {
                Ok(()) => {
                    directory = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(LeanCheckError::Access {
                        path: candidate,
                        source,
                    });
                }
            }
        }
        let directory = directory.ok_or_else(|| LeanCheckError::Access {
            path: parent.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique check scratch directory",
            ),
        })?;
        let source = directory.join("Main.lean");
        if let Err(source_error) = fs::write(&source, source_text) {
            let _ = fs::remove_dir_all(&directory);
            return Err(LeanCheckError::Access {
                path: source,
                source: source_error,
            });
        }
        Ok(Self { directory, source })
    }

    fn remove(self) -> Result<(), LeanCheckError> {
        fs::remove_dir_all(&self.directory).map_err(|source| LeanCheckError::Cleanup {
            path: self.directory,
            source,
        })
    }
}

fn parse_diagnostics(output: &str) -> Vec<LeanDiagnostic> {
    let mut diagnostics = Vec::new();
    for line in output.lines() {
        if let Some(diagnostic) = parse_diagnostic_line(line) {
            diagnostics.push(diagnostic);
        } else if !line.trim().is_empty()
            && let Some(last) = diagnostics.last_mut()
        {
            last.message.push('\n');
            last.message.push_str(line.trim_end());
        }
    }
    diagnostics
}

fn parse_diagnostic_line(line: &str) -> Option<LeanDiagnostic> {
    const SEVERITIES: [(&str, LeanDiagnosticSeverity); 4] = [
        ("error", LeanDiagnosticSeverity::Error),
        ("warning", LeanDiagnosticSeverity::Warning),
        ("information", LeanDiagnosticSeverity::Information),
        ("info", LeanDiagnosticSeverity::Information),
    ];
    for (label, severity) in SEVERITIES {
        let marker = format!(": {label}:");
        let Some(marker_start) = line.find(&marker) else {
            continue;
        };
        let location = &line[..marker_start];
        let message = line[marker_start + marker.len()..].trim().to_owned();
        let (file, line_number, column) = parse_location(location);
        return Some(LeanDiagnostic {
            severity,
            message,
            file,
            line: line_number,
            column,
            end_line: None,
            end_column: None,
        });
    }
    None
}

fn parse_location(location: &str) -> (Option<String>, Option<u32>, Option<u32>) {
    let mut parts = location.rsplitn(3, ':');
    let Some(column) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
        return (None, None, None);
    };
    let Some(line) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
        return (None, None, None);
    };
    let file = parts.next().filter(|value| !value.is_empty());
    (file.map(str::to_owned), Some(line), Some(column))
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
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, thread, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        DEFAULT_LEAN_TOOLCHAIN, DEFAULT_MATHLIB_REFERENCE, DEFAULT_PROFILE_ID,
        LeanEnvironmentProbe, LeanEnvironmentState, LeanFingerprintStatus, LeanProfileState,
        LeanProjectConfig, ManagedLeanProfileMetadata, fingerprint_environment,
    };

    fn executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn profile_fixture(lake_body: &str) -> (TempDir, TempDir, ManagedLeanProfile) {
        let store_root = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        fs::write(
            project.path().join("lean-toolchain"),
            DEFAULT_LEAN_TOOLCHAIN,
        )
        .unwrap();
        fs::write(
            project.path().join("lakefile.toml"),
            "name = \"fixture\"\n[[lean_lib]]\nname = \"Fixture\"\n",
        )
        .unwrap();
        fs::write(
            project.path().join("lake-manifest.json"),
            r#"{"packages":[{"name":"mathlib","inputRev":"v4.33.1","rev":"abc"}]}"#,
        )
        .unwrap();
        fs::create_dir_all(
            project
                .path()
                .join(".lake/packages/mathlib/.lake/build/lib/lean"),
        )
        .unwrap();
        fs::write(
            project
                .path()
                .join(".lake/packages/mathlib/.lake/build/lib/lean/Mathlib.olean"),
            b"fixture",
        )
        .unwrap();
        let bin = project.path().join("bin");
        fs::create_dir(&bin).unwrap();
        executable(
            &bin.join("lean"),
            "#!/bin/sh\nprintf 'Lean (version 4.33.1)\\n'",
        );
        executable(&bin.join("lake"), lake_body);
        let cache_root = store_root.path().join("cache");
        let config = LeanProjectConfig::new(project.path())
            .unwrap()
            .with_lean_executable(bin.join("lean"))
            .unwrap()
            .with_lake_executable(bin.join("lake"))
            .unwrap()
            .with_cache_root(&cache_root)
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert_eq!(probe.state, LeanEnvironmentState::Ready);
        let fingerprint = fingerprint_environment(&config, &probe).unwrap();
        let metadata = ManagedLeanProfileMetadata {
            schema_version: 1,
            id: DEFAULT_PROFILE_ID.to_owned(),
            state: if fingerprint.status == LeanFingerprintStatus::Unverified {
                LeanProfileState::Unverified
            } else {
                LeanProfileState::Ready
            },
            project_root: project.path().to_owned(),
            lean_toolchain: DEFAULT_LEAN_TOOLCHAIN.to_owned(),
            mathlib_reference: DEFAULT_MATHLIB_REFERENCE.to_owned(),
            detected_mathlib_revision: Some(DEFAULT_MATHLIB_REFERENCE.to_owned()),
            fingerprint,
        };
        let profile = crate::ManagedLeanProfile::from_test_parts(metadata, probe);
        (store_root, project, profile)
    }

    #[test]
    fn parses_locations_and_continuations() {
        let diagnostics = parse_diagnostics(
            "/tmp/Main.lean:3:7: error: unknown identifier 'x'\n  additional context\n/tmp/Main.lean:5:1: warning: declaration uses 'sorry'",
        );
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].line, Some(3));
        assert_eq!(diagnostics[0].column, Some(7));
        assert!(diagnostics[0].message.contains("additional context"));
        assert_eq!(diagnostics[1].severity, LeanDiagnosticSeverity::Warning);
    }

    #[test]
    fn validates_request_bounds() {
        assert!(matches!(
            LeanCheckRequest::new("import Mathlib")
                .unwrap()
                .timeout_ms(999),
            Err(LeanCheckError::InvalidOption {
                field: "lean.check.timeout_ms",
                ..
            })
        ));
        assert!(matches!(
            LeanCheckRequest::new(" "),
            Err(LeanCheckError::InvalidSource { .. })
        ));
    }

    #[test]
    fn checks_success_and_removes_scratch() {
        let (_store_root, _project, profile) = profile_fixture(
            r#"#!/bin/sh
if [ "$1" = "env" ] && [ "$2" = "lean" ] && [ "$3" = "--version" ]; then
  printf 'Lean (version 4.33.1)\n'
  exit 0
fi
printf 'Lake version 5\n'
exit 0
"#,
        );
        let request =
            LeanCheckRequest::new("import Mathlib\n\nexample : True := by trivial\n").unwrap();
        let result = LeanChecker::new()
            .check(&profile, &request, &LeanCancellation::new())
            .unwrap();
        assert_eq!(result.status, LeanCheckStatus::Checked);
        assert!(result.diagnostics.is_empty());
        assert!(
            !profile
                .fingerprint()
                .cache_root
                .as_deref()
                .unwrap()
                .contains("run-")
        );
        let check_root = PathBuf::from(profile.fingerprint().cache_root.as_deref().unwrap())
            .join(CHECK_DIRECTORY);
        assert!(fs::read_dir(check_root).map_or(true, |mut entries| entries.next().is_none()));
    }

    #[test]
    fn reports_lean_diagnostics_and_profile_mismatch() {
        let (_store_root, _project, profile) = profile_fixture(
            r#"#!/bin/sh
if [ "$1" = "env" ] && [ "$2" = "lean" ] && [ "$3" = "--version" ]; then
  printf 'Lean (version 4.33.1)\n'
  exit 0
fi
if [ "$1" != "env" ]; then
  printf 'Lake version 5\n'
  exit 0
fi
printf '%s:2:4: error: proof failed\n' "$3" >&2
exit 1
"#,
        );
        let request =
            LeanCheckRequest::new("import Mathlib\n\nexample : True := by sorry\n").unwrap();
        let result = LeanChecker::new()
            .check(&profile, &request, &LeanCancellation::new())
            .unwrap();
        assert_eq!(result.status, LeanCheckStatus::Failed);
        assert_eq!(result.diagnostics[0].line, Some(2));
        let mismatch = request
            .clone()
            .profile_fingerprint("sha256:not-the-profile");
        assert!(matches!(
            LeanChecker::new().check(&profile, &mismatch, &LeanCancellation::new()),
            Err(LeanCheckError::ProfileFingerprintMismatch { .. })
        ));
    }

    #[test]
    fn maps_timeout_and_cancellation() {
        let (_store_root, _project, profile) = profile_fixture(
            r#"#!/bin/sh
if [ "$1" = "env" ] && [ "$2" = "lean" ] && [ "$3" = "--version" ]; then
  printf 'Lean (version 4.33.1)\n'
  exit 0
fi
if [ "$1" != "env" ]; then
  printf 'Lake version 5\n'
  exit 0
fi
trap 'exit 130' INT
while :; do :; done
"#,
        );
        let request = LeanCheckRequest::new("import Mathlib")
            .unwrap()
            .timeout_ms(1_000)
            .unwrap();
        let result = LeanChecker::new()
            .check(&profile, &request, &LeanCancellation::new())
            .unwrap();
        assert_eq!(result.status, LeanCheckStatus::TimedOut);

        let cancellation = LeanCancellation::new();
        let signal = cancellation.clone();
        let profile_copy = profile.clone();
        let request_copy = request.clone();
        let worker =
            thread::spawn(move || LeanChecker::new().check(&profile_copy, &request_copy, &signal));
        thread::sleep(Duration::from_millis(40));
        cancellation.cancel();
        let result = worker.join().unwrap().unwrap();
        assert_eq!(result.status, LeanCheckStatus::Cancelled);
    }
}
