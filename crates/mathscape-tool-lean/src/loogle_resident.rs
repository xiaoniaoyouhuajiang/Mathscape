//! Resident Loogle (`loogle --json --interactive`) on top of
//! [`mathscape_resident::Supervisor`].
//!
//! One instance per app, keyed by [`LoogleResidentKey`]: profile fingerprint
//! digest, module, index file and the Loogle executable's SHA-256. A key change
//! stops the old process and starts a new one, so at most one 5.5GB index is
//! ever loaded.
//!
//! Loogle is executed directly (not through `lake env`, which would keep an
//! extra ~0.6GB lake process alive) with `LEAN_PATH` taken once from
//! `lake env printenv LEAN_PATH` and cached per profile fingerprint. The index
//! is always opened with `--index-mode read`: a missing or stale index is a
//! startup failure reported as [`LeanLoogleError::ResidentUnavailable`]; the
//! resident path never rebuilds it. `--max-results` is fixed at startup
//! ([`RESIDENT_MAX_RESULTS`]) and each reply is truncated to the request, with
//! `total` still Loogle's `count`, so `truncated` matches the one-shot path.
//!
//! The protocol is serial (one query line in, one JSON line out); callers keep
//! the instance behind a `Mutex`, which is the queue.

use std::{
    collections::HashMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use mathscape_resident::{
    AcquireError, Clock, Concurrency, EventSink, ProcessSpec, ReadLineError, ResidentPolicy,
    ResidentState, Supervisor, SystemClock,
};
use serde::{Deserialize, Serialize};

use crate::{
    LeanCancellation, LeanLoogleError, LeanRunStatus, LeanRunner, LeanRunnerOptions,
    LoogleSearchResult, ManagedLeanProfile,
    loogle::{
        LoogleIndexMode, build_search_result, ensure_managed_index, fingerprint_file,
        hash_executable, parse_rejection, parse_response,
    },
    runner::LeanCommand,
};

/// Fixed `--max-results` of the resident process (the MCP maximum is 100).
pub const RESIDENT_MAX_RESULTS: usize = 200;
/// The line `loogle --interactive` prints once the index is loaded.
pub const LOOGLE_READY_LINE: &str = "Loogle is ready.";
/// Instance name used in lifecycle events.
pub const LOOGLE_INSTANCE_NAME: &str = "loogle";

/// Everything needed to run Loogle for one managed profile. The MCP server
/// resolves it from the profile; the desktop host re-validates it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoogleTarget {
    pub profile_fingerprint: String,
    pub project_root: PathBuf,
    pub lake_executable: PathBuf,
    pub loogle_executable: PathBuf,
    pub index_file: PathBuf,
    pub managed_cache_root: PathBuf,
}

impl LoogleTarget {
    pub fn for_profile(
        profile: &ManagedLeanProfile,
        loogle_executable: impl Into<PathBuf>,
        index_file: impl Into<PathBuf>,
    ) -> Result<Self, LeanLoogleError> {
        if !profile.probe().is_ready() {
            return Err(LeanLoogleError::CapabilityUnavailable {
                reason: "Lean profile is not ready",
            });
        }
        let lake = profile.probe().lake_executable.clone().ok_or(
            LeanLoogleError::CapabilityUnavailable {
                reason: "Lake executable is unavailable",
            },
        )?;
        let cache_root = profile
            .fingerprint()
            .cache_root
            .as_deref()
            .map(PathBuf::from)
            .ok_or(LeanLoogleError::CapabilityUnavailable {
                reason: "managed profile cache is unavailable",
            })?;
        let target = Self {
            profile_fingerprint: profile.fingerprint().digest.clone(),
            project_root: profile.metadata().project_root.clone(),
            lake_executable: lake,
            loogle_executable: loogle_executable.into(),
            index_file: index_file.into(),
            managed_cache_root: cache_root,
        };
        target.validate()?;
        Ok(target)
    }

    /// Absolute paths, existing files, and the index inside the managed cache.
    pub fn validate(&self) -> Result<(), LeanLoogleError> {
        if self.profile_fingerprint.trim().is_empty() {
            return Err(LeanLoogleError::InvalidQuery(
                "profile fingerprint must not be blank".to_owned(),
            ));
        }
        for path in [
            &self.project_root,
            &self.lake_executable,
            &self.loogle_executable,
            &self.index_file,
            &self.managed_cache_root,
        ] {
            if !path.is_absolute() {
                return Err(access(path, "path must be absolute"));
            }
        }
        if !self.project_root.is_dir() {
            return Err(access(
                &self.project_root,
                "Lean project is not a directory",
            ));
        }
        for file in [&self.lake_executable, &self.loogle_executable] {
            if !file.is_file() {
                return Err(access(file, "executable is not a regular file"));
            }
        }
        ensure_managed_index(&self.index_file, Some(&self.managed_cache_root))?;
        if !self.index_file.is_file() {
            return Err(LeanLoogleError::ResidentUnavailable {
                reason: format!(
                    "managed Loogle index is missing at {}",
                    self.index_file.display()
                ),
            });
        }
        Ok(())
    }
}

fn access(path: &Path, message: &'static str) -> LeanLoogleError {
    LeanLoogleError::Access {
        path: path.to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
    }
}

/// Identity of the resident process. Startup-only Loogle options are part of
/// it, so a different module or index needs a different process.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct LoogleResidentKey {
    pub profile_fingerprint: String,
    pub module: String,
    pub index_file: PathBuf,
    pub executable_sha256: String,
}

/// A resident answer, with the fingerprints the MCP result reports.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResidentLoogleResult {
    pub result: LoogleSearchResult,
    pub index_fingerprint: String,
}

/// Outcome of one resident query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResidentLoogleOutcome {
    Done(ResidentLoogleResult),
    /// Resident mode is off or has fallen back: run the one-shot path.
    Fallback {
        reason: String,
    },
}

/// The lifecycle policy approved for Loogle: start 90s, query 30s, idle 15
/// minutes, three consecutive failures, serial, empty-line farewell.
pub fn loogle_resident_policy() -> ResidentPolicy {
    ResidentPolicy {
        startup_timeout: Duration::from_secs(90),
        request_timeout: Duration::from_secs(30),
        idle_timeout: Some(Duration::from_secs(15 * 60)),
        max_consecutive_failures: 3,
        backoff_initial: Duration::from_millis(500),
        backoff_max: Duration::from_secs(10),
        shutdown_grace: Duration::from_secs(2),
        concurrency: Concurrency::Serial,
        farewell: Some(String::new()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    inode: u64,
}

impl FileStamp {
    fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        };
        #[cfg(not(unix))]
        let inode = 0;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            inode,
        })
    }
}

/// File digests cached by (path, size, mtime, inode): the resident path does
/// not re-hash the 376MB index or the Loogle binary on every query.
#[derive(Debug, Default)]
struct DigestCache {
    entries: HashMap<PathBuf, (FileStamp, String)>,
}

impl DigestCache {
    fn digest(
        &mut self,
        path: &Path,
        hash: fn(&Path) -> std::io::Result<String>,
    ) -> std::io::Result<String> {
        let stamp = FileStamp::of(path)?;
        if let Some((cached, digest)) = self.entries.get(path)
            && *cached == stamp
        {
            return Ok(digest.clone());
        }
        let digest = hash(path)?;
        self.entries
            .insert(path.to_owned(), (stamp, digest.clone()));
        Ok(digest)
    }
}

/// The resident Loogle instance.
pub struct ResidentLoogle {
    supervisor: Supervisor<LoogleResidentKey>,
    digests: DigestCache,
    lean_paths: HashMap<String, OsString>,
}

impl std::fmt::Debug for ResidentLoogle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentLoogle")
            .field("supervisor", &self.supervisor)
            .finish_non_exhaustive()
    }
}

impl ResidentLoogle {
    pub fn new(sink: EventSink) -> Self {
        Self::with_policy(loogle_resident_policy(), Arc::new(SystemClock), sink)
    }

    pub fn with_policy(policy: ResidentPolicy, clock: Arc<dyn Clock>, sink: EventSink) -> Self {
        Self {
            supervisor: Supervisor::new(LOOGLE_INSTANCE_NAME, policy, clock, sink),
            digests: DigestCache::default(),
            lean_paths: HashMap::new(),
        }
    }

    pub const fn state(&self) -> ResidentState {
        self.supervisor.state()
    }

    pub fn pid(&self) -> Option<u32> {
        self.supervisor.pid()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.supervisor.set_enabled(enabled);
    }

    pub fn reap_idle(&mut self) -> bool {
        self.supervisor.reap_idle()
    }

    pub fn shutdown(&mut self) {
        self.supervisor.shutdown();
    }

    /// Runs one query on the resident process, starting it if needed.
    pub fn search(
        &mut self,
        target: &LoogleTarget,
        query: &str,
        module: &str,
        max_results: usize,
    ) -> Result<ResidentLoogleOutcome, LeanLoogleError> {
        validate_resident_query(query)?;
        if !(1..=RESIDENT_MAX_RESULTS).contains(&max_results) {
            return Err(LeanLoogleError::InvalidQuery(format!(
                "max_results must be between 1 and {RESIDENT_MAX_RESULTS}"
            )));
        }
        match self.supervisor.state() {
            ResidentState::Disabled => {
                return Ok(ResidentLoogleOutcome::Fallback {
                    reason: "disabled".to_owned(),
                });
            }
            ResidentState::Failed => {
                return Ok(ResidentLoogleOutcome::Fallback {
                    reason: format!(
                        "fell back after repeated failures: {}",
                        self.supervisor.fallback_reason().unwrap_or_default()
                    ),
                });
            }
            _ => {}
        }
        target.validate()?;
        let executable_sha256 = self
            .digests
            .digest(&target.loogle_executable, hash_executable)
            .map_err(|source| LeanLoogleError::Access {
                path: target.loogle_executable.clone(),
                source,
            })?;
        let index_fingerprint = self
            .digests
            .digest(&target.index_file, fingerprint_file)
            .map_err(|source| LeanLoogleError::Access {
                path: target.index_file.clone(),
                source,
            })?;
        let key = LoogleResidentKey {
            profile_fingerprint: target.profile_fingerprint.clone(),
            module: module.to_owned(),
            index_file: target.index_file.clone(),
            executable_sha256: executable_sha256.clone(),
        };
        let request_timeout = self.supervisor.policy().request_timeout;
        let lean_paths = &mut self.lean_paths;
        let child = match self.supervisor.acquire(
            &key,
            || {
                let lean_path = cached_lean_path(lean_paths, target)?;
                Ok(resident_spec(target, module, lean_path))
            },
            |child, timeout| {
                child
                    .read_until(timeout, |line| line.trim() == LOOGLE_READY_LINE)
                    .map_err(|error| match error {
                        ReadLineError::Timeout => {
                            format!("no `{LOOGLE_READY_LINE}` within {}s", timeout.as_secs())
                        }
                        other => format!("before `{LOOGLE_READY_LINE}`: {other}"),
                    })
            },
        ) {
            Ok(child) => child,
            Err(AcquireError::Disabled) => {
                return Ok(ResidentLoogleOutcome::Fallback {
                    reason: "disabled".to_owned(),
                });
            }
            Err(AcquireError::Fallback { reason }) => {
                return Ok(ResidentLoogleOutcome::Fallback {
                    reason: format!("fell back after repeated failures: {reason}"),
                });
            }
            Err(AcquireError::Startup { reason, stderr }) => {
                let mut reason = reason;
                if !stderr.trim().is_empty() {
                    reason.push_str("; stderr: ");
                    reason.push_str(tail(&stderr, 1024));
                }
                return Err(LeanLoogleError::ResidentUnavailable { reason });
            }
        };

        if let Err(error) = child.write_line(query) {
            let stderr = child.stderr_tail();
            self.supervisor
                .report_failure(format!("failed to write query: {error}"));
            return Err(LeanLoogleError::RunFailed {
                status: "resident_write_failed".to_owned(),
                stderr,
            });
        }
        let line = match child.read_line(request_timeout) {
            Ok(line) => line,
            Err(ReadLineError::Timeout) => {
                self.supervisor.report_failure(format!(
                    "query timed out after {}s",
                    request_timeout.as_secs()
                ));
                return Err(LeanLoogleError::TimedOut {
                    stage: "query".to_owned(),
                    seconds: request_timeout.as_secs(),
                });
            }
            Err(error) => {
                // Give the stderr reader a moment to catch the crash message.
                std::thread::sleep(Duration::from_millis(50));
                let stderr = child.stderr_tail();
                self.supervisor.report_failure(format!("no reply: {error}"));
                return Err(LeanLoogleError::RunFailed {
                    status: format!("resident_{}", reply_status(&error)),
                    stderr,
                });
            }
        };
        if let Some(rejected) = parse_rejection(line.as_bytes()) {
            self.supervisor.report_success();
            return Err(rejected);
        }
        let response = match parse_response(line.as_bytes()) {
            Ok(response) => response,
            Err(error) => {
                // The line stream may be out of step: restart the process.
                self.supervisor
                    .report_failure(format!("invalid reply: {error}"));
                return Err(error);
            }
        };
        self.supervisor.report_success();
        Ok(ResidentLoogleOutcome::Done(ResidentLoogleResult {
            result: build_search_result(
                response,
                &target.profile_fingerprint,
                &executable_sha256,
                max_results,
            ),
            index_fingerprint,
        }))
    }
}

/// The interactive protocol is line-based: a newline or carriage return in a
/// query would split it into two queries.
pub fn validate_resident_query(query: &str) -> Result<(), LeanLoogleError> {
    if query.trim().is_empty() || query.contains(['\n', '\r']) {
        return Err(LeanLoogleError::InvalidQuery(
            "query must be one non-empty line".to_owned(),
        ));
    }
    Ok(())
}

fn reply_status(error: &ReadLineError) -> &'static str {
    match error {
        ReadLineError::Timeout => "timeout",
        ReadLineError::Closed => "exited",
        ReadLineError::Io(_) => "read_failed",
    }
}

fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn resident_spec(target: &LoogleTarget, module: &str, lean_path: OsString) -> ProcessSpec {
    ProcessSpec::new(&target.loogle_executable)
        .arg("--json")
        .arg("--interactive")
        .arg("--index-mode")
        .arg(LoogleIndexMode::Read.as_str())
        .arg("--module")
        .arg(module)
        .arg("--max-results")
        .arg(RESIDENT_MAX_RESULTS.to_string())
        .arg("--index-file")
        .arg(target.index_file.as_os_str())
        .env("LEAN_PATH", lean_path)
        .cwd(&target.project_root)
}

fn cached_lean_path(
    cache: &mut HashMap<String, OsString>,
    target: &LoogleTarget,
) -> Result<OsString, String> {
    if let Some(path) = cache.get(&target.profile_fingerprint) {
        return Ok(path.clone());
    }
    let path = lake_lean_path(target)?;
    cache.insert(target.profile_fingerprint.clone(), path.clone());
    Ok(path)
}

/// `lake env printenv LEAN_PATH` in the profile project. Read-only: it does not
/// build, update or rewrite the user's Lean dependencies.
fn lake_lean_path(target: &LoogleTarget) -> Result<OsString, String> {
    let options = LeanRunnerOptions::new(Duration::from_secs(60))
        .map_err(|error| error.to_string())?
        .max_output_bytes(1024 * 1024)
        .map_err(|error| error.to_string())?;
    let command = LeanCommand::new(&target.lake_executable, &target.project_root).args([
        "env",
        "printenv",
        "LEAN_PATH",
    ]);
    let run = LeanRunner::new(options)
        .run(&command, &LeanCancellation::new())
        .map_err(|error| format!("lake env printenv LEAN_PATH failed: {error}"))?;
    if run.status() != LeanRunStatus::Succeeded || run.stdout().is_truncated() {
        return Err(format!(
            "lake env printenv LEAN_PATH failed ({:?}): {}",
            run.status(),
            tail(&run.stderr().lossy(), 1024)
        ));
    }
    let path = run.stdout().lossy().trim().to_owned();
    if path.is_empty() {
        return Err("lake env printenv LEAN_PATH printed nothing".to_owned());
    }
    Ok(OsString::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_and_carriage_return_are_rejected() {
        for query in ["a\nb", "a\rb", "", "  "] {
            assert!(matches!(
                validate_resident_query(query),
                Err(LeanLoogleError::InvalidQuery(_))
            ));
        }
        validate_resident_query("Nat.add_comm").unwrap();
        validate_resident_query("\"klDiv\"").unwrap();
    }

    #[test]
    fn policy_matches_the_approved_numbers() {
        let policy = loogle_resident_policy();
        assert_eq!(policy.startup_timeout, Duration::from_secs(90));
        assert_eq!(policy.request_timeout, Duration::from_secs(30));
        assert_eq!(policy.idle_timeout, Some(Duration::from_secs(900)));
        assert_eq!(policy.max_consecutive_failures, 3);
        assert_eq!(policy.concurrency, Concurrency::Serial);
    }

    #[test]
    fn tail_respects_char_boundaries() {
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("ééé", 3), "é");
    }
}
