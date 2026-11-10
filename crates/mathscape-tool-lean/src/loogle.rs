use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    DECLARATION_SCHEMA_VERSION, DeclarationCandidate, DeclarationKind, LeanCancellation,
    LeanLoogleError, LeanRunStatus, LeanRunner, LeanRunnerOptions, ManagedLeanProfile,
    runner::LeanCommand,
};

/// Managed Loogle executable, relative to the profile directory.
pub const MANAGED_LOOGLE_RELATIVE_PATH: &str = "tools/loogle/.lake/build/bin/loogle";
/// Managed Loogle index, relative to the profile's fingerprinted cache root.
pub const MANAGED_LOOGLE_INDEX_RELATIVE_PATH: &str = "loogle/mathlib.loogle-index";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoogleIndexMode {
    None,
    Use,
    Read,
    Write,
}

impl LoogleIndexMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Use => "use",
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoogleSearchRequest {
    pub query: String,
    pub module: String,
    pub max_results: usize,
    pub index_mode: LoogleIndexMode,
    pub index_file: Option<PathBuf>,
}

impl LoogleSearchRequest {
    pub fn new(query: impl Into<String>) -> Result<Self, LeanLoogleError> {
        let query = query.into();
        if query.trim().is_empty() || query.chars().any(char::is_control) {
            return Err(LeanLoogleError::InvalidQuery(query));
        }
        Ok(Self {
            query,
            module: "Mathlib".to_owned(),
            max_results: 50,
            index_mode: LoogleIndexMode::None,
            index_file: None,
        })
    }

    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module = module.into();
        self
    }

    pub fn max_results(mut self, maximum: usize) -> Result<Self, LeanLoogleError> {
        if maximum == 0 {
            return Err(LeanLoogleError::InvalidQuery(
                "max_results must be greater than zero".to_owned(),
            ));
        }
        self.max_results = maximum;
        Ok(self)
    }

    pub fn index_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.index_mode = LoogleIndexMode::Use;
        self.index_file = Some(path.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoogleSearchResult {
    pub backend: String,
    pub backend_fingerprint: String,
    pub candidates: Vec<DeclarationCandidate>,
    pub total: usize,
    pub truncated: bool,
    pub profile_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct LoogleAdapter {
    executable: PathBuf,
    backend_fingerprint: String,
    runner: LeanRunner,
}

impl LoogleAdapter {
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, LeanLoogleError> {
        let executable = executable.into();
        if !executable.is_file() {
            return Err(LeanLoogleError::Access {
                path: executable,
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Loogle executable is not a regular file",
                ),
            });
        }
        let backend_fingerprint =
            hash_executable(&executable).map_err(|source| LeanLoogleError::Access {
                path: executable.clone(),
                source,
            })?;
        let options = LeanRunnerOptions::new(Duration::from_secs(5 * 60))
            .map_err(|error| LeanLoogleError::InvalidQuery(error.to_string()))?
            .max_output_bytes(16 * 1024 * 1024)
            .map_err(|error| LeanLoogleError::InvalidQuery(error.to_string()))?;
        Ok(Self {
            executable,
            backend_fingerprint,
            runner: LeanRunner::new(options),
        })
    }

    pub fn search(
        &self,
        profile: &ManagedLeanProfile,
        request: &LoogleSearchRequest,
    ) -> Result<LoogleSearchResult, LeanLoogleError> {
        if !profile.probe().is_ready() {
            return Err(LeanLoogleError::CapabilityUnavailable {
                reason: "Lean profile is not ready",
            });
        }
        let lake = profile.probe().lake_executable.as_deref().ok_or(
            LeanLoogleError::CapabilityUnavailable {
                reason: "Lake executable is unavailable",
            },
        )?;
        let project_root = &profile.metadata().project_root;
        if let Some(index_file) = &request.index_file {
            ensure_managed_index(
                index_file,
                profile.fingerprint().cache_root.as_deref().map(Path::new),
            )?;
        }
        let mut command = LeanCommand::new(lake.to_owned(), project_root.to_owned()).args([
            "env".to_owned(),
            self.executable.to_string_lossy().into_owned(),
            "--json".to_owned(),
            "--index-mode".to_owned(),
            request.index_mode.as_str().to_owned(),
            "--module".to_owned(),
            request.module.clone(),
            "--max-results".to_owned(),
            request.max_results.to_string(),
        ]);
        if let Some(index_file) = &request.index_file {
            command = command
                .arg("--index-file")
                .arg(index_file.to_string_lossy().into_owned());
        }
        command = command.arg(request.query.clone());
        let run = self.runner.run(&command, &LeanCancellation::new())?;
        if run.status() != LeanRunStatus::Succeeded {
            // Loogle may report a rejected query as JSON even on a failing exit.
            if !run.stdout().is_truncated()
                && let Some(rejected) = parse_rejection(run.stdout().bytes())
            {
                return Err(rejected);
            }
            return Err(LeanLoogleError::RunFailed {
                status: format!("{:?}", run.status()),
                stderr: run.stderr().lossy(),
            });
        }
        if run.stdout().is_truncated() {
            return Err(LeanLoogleError::OutputTruncated);
        }
        let response = parse_response(run.stdout().bytes())?;
        Ok(build_search_result(
            response,
            &profile.fingerprint().digest,
            &self.backend_fingerprint,
            usize::MAX,
        ))
    }
}

/// Turns a Loogle reply into the MCP-facing result. `limit` caps the
/// candidates (the resident process runs with a fixed `--max-results`);
/// `total` stays Loogle's `count`, so `truncated` means the same thing on both
/// backends: `count > candidates.len()`.
pub(crate) fn build_search_result(
    response: RawLoogleResponse,
    profile_fingerprint: &str,
    backend_fingerprint: &str,
    limit: usize,
) -> LoogleSearchResult {
    let candidates = response
        .hits
        .into_iter()
        .take(limit)
        .map(|hit| DeclarationCandidate {
            schema_version: DECLARATION_SCHEMA_VERSION,
            profile_fingerprint: profile_fingerprint.to_owned(),
            name: hit.name,
            kind: DeclarationKind::Unknown,
            module: hit.module,
            type_text: hit.type_text,
            docstring: hit.doc,
            source_file: None,
            source_line: None,
            source_column: None,
            dependencies: Vec::new(),
            type_truncated: false,
            metadata_complete: false,
        })
        .collect::<Vec<_>>();
    LoogleSearchResult {
        backend: "loogle".to_owned(),
        backend_fingerprint: backend_fingerprint.to_owned(),
        total: response.count,
        truncated: response.count > candidates.len(),
        candidates,
        profile_fingerprint: profile_fingerprint.to_owned(),
    }
}

/// Checks that `index_file` stays inside the profile's managed cache.
pub(crate) fn ensure_managed_index(
    index_file: &Path,
    managed_root: Option<&Path>,
) -> Result<(), LeanLoogleError> {
    let Some(managed_root) = managed_root else {
        return Err(LeanLoogleError::Access {
            path: index_file.to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "managed profile cache is unavailable",
            ),
        });
    };
    if !index_file.starts_with(managed_root) {
        return Err(LeanLoogleError::Access {
            path: index_file.to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Loogle index must stay inside the managed profile cache",
            ),
        });
    }
    Ok(())
}

/// SHA-256 of a file as `sha256:<hex>` (the MCP `index_fingerprint` format).
pub fn fingerprint_file(path: &Path) -> std::io::Result<String> {
    Ok(format!("sha256:{}", hash_executable(path)?))
}

pub(crate) fn hash_executable(path: &Path) -> std::io::Result<String> {
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
    Ok(hex(&hasher.finalize()))
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

pub(crate) fn parse_response(bytes: &[u8]) -> Result<RawLoogleResponse, LeanLoogleError> {
    if let Some(rejected) = parse_rejection(bytes) {
        return Err(rejected);
    }
    serde_json::from_slice(bytes).map_err(LeanLoogleError::InvalidJson)
}

/// Recognizes Loogle's `{"error": ..., "suggestions": [...]}` reply.
pub(crate) fn parse_rejection(bytes: &[u8]) -> Option<LeanLoogleError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let message = value.get("error")?.as_str()?.trim().to_owned();
    let suggestions = value
        .get("suggestions")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(LeanLoogleError::QueryRejected {
        message,
        suggestions,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLoogleResponse {
    pub(crate) count: usize,
    #[serde(default)]
    #[serde(rename = "header")]
    _header: Option<String>,
    #[serde(default)]
    #[serde(rename = "heartbeats")]
    _heartbeats: Option<u64>,
    /// A query that parses but matches nothing comes back as `count: 0` with
    /// suggestions (measured on Loogle for Mathlib 4.33.1).
    #[serde(default)]
    #[serde(rename = "suggestions")]
    _suggestions: Option<Vec<String>>,
    pub(crate) hits: Vec<RawLoogleHit>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLoogleHit {
    name: String,
    module: String,
    #[serde(rename = "type")]
    type_text: String,
    doc: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_to_mathlib_without_an_index_write() {
        let request = LoogleSearchRequest::new("add_comm").unwrap();
        assert_eq!(request.module, "Mathlib");
        assert_eq!(request.index_mode, LoogleIndexMode::None);
        assert!(request.index_file.is_none());
    }

    #[test]
    fn error_reply_becomes_query_rejected_with_suggestions() {
        let error = parse_response(
            br#"{"error":"Unknown identifier 'klDiv'","header":"","heartbeats":12,"suggestions":["InformationTheory.klDiv"]}"#,
        )
        .unwrap_err();
        match error {
            LeanLoogleError::QueryRejected {
                message,
                suggestions,
            } => {
                assert_eq!(message, "Unknown identifier 'klDiv'");
                assert_eq!(suggestions, vec!["InformationTheory.klDiv".to_owned()]);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_reply_without_suggestions_is_still_rejected() {
        assert!(matches!(
            parse_response(br#"{"error":"Cannot parse query"}"#),
            Err(LeanLoogleError::QueryRejected { suggestions, .. }) if suggestions.is_empty()
        ));
    }

    #[test]
    fn hit_reply_still_parses() {
        let response = parse_response(
            br#"{"count":1,"header":"Found one","heartbeats":3,"hits":[{"name":"add_comm","module":"Mathlib.Algebra.Group.Defs","type":"forall a b, a + b = b + a","doc":null}]}"#,
        )
        .unwrap();
        assert_eq!(response.count, 1);
        assert_eq!(response.hits[0].name, "add_comm");
    }

    #[test]
    fn no_match_reply_with_suggestions_parses_as_empty() {
        let response = parse_response(
            br#"{"count":0,"header":"","heartbeats":1,"hits":[],"suggestions":["Nat.add_comm"]}"#,
        )
        .unwrap();
        assert_eq!(response.count, 0);
        assert!(response.hits.is_empty());
    }

    #[test]
    fn unrelated_json_stays_invalid() {
        assert!(matches!(
            parse_response(br#"{"surprise":true}"#),
            Err(LeanLoogleError::InvalidJson(_))
        ));
    }
}
