//! CapabilityProbe (contract §4): best-effort, read-only detection of
//! external tools. Probing never installs anything; a missing tool is a
//! normal result, not an error.

use std::{
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// External capability names (snake_case on the wire).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Marker,
    Lean,
    Codex,
    ClaudeCode,
    Hmz,
}

impl Capability {
    pub const ALL: [Self; 5] = [
        Self::Marker,
        Self::Lean,
        Self::Codex,
        Self::ClaudeCode,
        Self::Hmz,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Marker => "marker",
            Self::Lean => "lean",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Hmz => "hmz",
        }
    }

    /// Version requirement, if Mathscape pins one. Marker follows
    /// `mathscape-tool-marker`'s supported major (2.x).
    pub const fn required(self) -> Option<&'static str> {
        match self {
            Self::Marker => Some(">=2.0, <3"),
            _ => None,
        }
    }

    /// Repository-relative install script (`scripts/install/<capability>.sh`).
    pub fn install_hint(self) -> String {
        format!("scripts/install/{}.sh", self.as_str())
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    Missing,
    /// Found, but the version is unknown or does not satisfy `required`.
    Version,
}

/// `{ capability, state, found_version, required, install_hint }`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProbe {
    pub capability: Capability,
    pub state: CapabilityState,
    pub found_version: Option<String>,
    pub required: Option<String>,
    pub install_hint: Option<String>,
}

impl CapabilityProbe {
    pub fn is_available(&self) -> bool {
        self.state == CapabilityState::Available
    }
}

/// Output of a finished probe command.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Environment seam so probing can be tested without real binaries.
pub trait CommandRunner: Send + Sync {
    /// Runs `program args…`. `Err` means it could not be started (e.g. not
    /// installed) or timed out.
    fn run(&self, program: &Path, args: &[&str]) -> io::Result<CommandOutput>;
    fn env_var(&self, key: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
}

/// Real runner: spawns processes with a timeout and null stdin.
#[derive(Clone, Debug)]
pub struct SystemCommandRunner {
    pub timeout: Duration,
}

impl Default for SystemCommandRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
        }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &Path, args: &[&str]) -> io::Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "probe timed out"));
            }
            thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn env_var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// Probes every Phase0 capability.
pub fn probe_all(runner: &dyn CommandRunner) -> Vec<CapabilityProbe> {
    Capability::ALL
        .iter()
        .map(|capability| probe_capability(runner, *capability))
        .collect()
}

pub fn probe_capability(runner: &dyn CommandRunner, capability: Capability) -> CapabilityProbe {
    let found = match capability {
        Capability::Marker => probe_marker(runner),
        Capability::Lean => probe_version_command(runner, "lean", &["--version"]).or_else(|| {
            home_bin(runner, ".elan/bin/lean")
                .and_then(|lean| version_of(runner, &lean, &["--version"]))
        }),
        Capability::Codex => probe_version_command(runner, "codex", &["--version"]),
        Capability::ClaudeCode => probe_version_command(runner, "claude", &["--version"]),
        Capability::Hmz => probe_version_command(runner, "hmz", &["--version"]),
    };
    finish_probe(capability, found)
}

/// Probes one executable with `program --version` and reports `capability`.
///
/// Local process only: no network and no install. Marker keeps the venv lookup
/// in [`probe_capability`]; this is the Codex / Claude / hmz / lean shape, and
/// what [`crate::CodexExecutor`] uses for a configured binary path.
pub fn probe_program(
    runner: &dyn CommandRunner,
    capability: Capability,
    program: &Path,
) -> CapabilityProbe {
    finish_probe(capability, version_of(runner, program, &["--version"]))
}

fn finish_probe(capability: Capability, found: Option<Found>) -> CapabilityProbe {
    let required = capability.required();
    let state = match (&found, required) {
        (None, _) => CapabilityState::Missing,
        (Some(Found { version: None }), Some(_)) => CapabilityState::Version,
        (Some(Found { version: Some(v) }), Some(req)) if !satisfies(v, req) => {
            CapabilityState::Version
        }
        _ => CapabilityState::Available,
    };
    CapabilityProbe {
        capability,
        state,
        found_version: found.and_then(|found| found.version),
        required: required.map(str::to_owned),
        install_hint: Some(capability.install_hint()),
    }
}

struct Found {
    version: Option<String>,
}

fn probe_version_command(
    runner: &dyn CommandRunner,
    program: &str,
    args: &[&str],
) -> Option<Found> {
    version_of(runner, Path::new(program), args)
}

fn version_of(runner: &dyn CommandRunner, program: &Path, args: &[&str]) -> Option<Found> {
    let output = runner.run(program, args).ok()?;
    if !output.success {
        // It started, so it exists; the version is just unknown.
        return Some(Found { version: None });
    }
    Some(Found {
        version: extract_version(&output.stdout).or_else(|| extract_version(&output.stderr)),
    })
}

fn home_bin(runner: &dyn CommandRunner, relative: &str) -> Option<PathBuf> {
    let path = PathBuf::from(runner.env_var("HOME")?).join(relative);
    runner.is_file(&path).then_some(path)
}

const MARKER_VERSION_SCRIPT: &str =
    "import importlib.metadata as m; print(m.version('marker-pdf'), end='')";

/// Marker lives in its own venv; read `marker-pdf` metadata with the venv's
/// Python next to `marker_single`. Candidates, in order:
/// `$MATHSCAPE_MARKER_EXECUTABLE`, `$MATHSCAPE_MARKER_VENV/bin`, the default
/// venv used by `scripts/install/marker.sh`, then `marker_single` on `PATH`.
fn probe_marker(runner: &dyn CommandRunner) -> Option<Found> {
    let dir = marker_bin_dir(runner)?;
    let python = ["python", "python3"]
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| runner.is_file(candidate));
    let version = python.and_then(|python| {
        runner
            .run(&python, &["-c", MARKER_VERSION_SCRIPT])
            .ok()
            .filter(|output| output.success)
            .and_then(|output| extract_version(&output.stdout))
    });
    Some(Found { version })
}

/// The `marker_single` the Marker probe would report, in the same candidate
/// order. Lets a caller run exactly the installation it probed.
pub fn locate_marker_executable(runner: &dyn CommandRunner) -> Option<PathBuf> {
    marker_bin_dir(runner).map(|dir| dir.join("marker_single"))
}

fn marker_bin_dir(runner: &dyn CommandRunner) -> Option<PathBuf> {
    let mut bin_dirs = Vec::new();
    if let Some(executable) = runner.env_var("MATHSCAPE_MARKER_EXECUTABLE")
        && let Some(parent) = Path::new(&executable).parent()
    {
        bin_dirs.push(parent.to_path_buf());
    }
    if let Some(venv) = runner.env_var("MATHSCAPE_MARKER_VENV") {
        bin_dirs.push(PathBuf::from(venv).join("bin"));
    }
    if let Some(venv) = default_marker_venv(runner) {
        bin_dirs.push(venv.join("bin"));
    }
    if let Some(path) = runner.env_var("PATH") {
        bin_dirs.extend(std::env::split_paths(&path));
    }
    bin_dirs
        .into_iter()
        .find(|dir| runner.is_file(&dir.join("marker_single")))
}

/// `$XDG_DATA_HOME/mathscape/marker/venv` or `~/.local/share/mathscape/marker/venv`.
pub(crate) fn default_marker_venv(runner: &dyn CommandRunner) -> Option<PathBuf> {
    let data_home = runner
        .env_var("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            runner
                .env_var("HOME")
                .map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data_home.join("mathscape/marker/venv"))
}

/// First dotted numeric token, e.g. `codex-cli 0.46.0` → `0.46.0`,
/// `Lean (version 4.22.0, …)` → `4.22.0`, `v1.2.3-rc1` → `1.2.3-rc1`.
fn extract_version(text: &str) -> Option<String> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '(' | ')' | '"'))
        .map(|token| token.trim_start_matches('v'))
        .find(|token| {
            let release = token.split(['-', '+']).next().unwrap_or("");
            let parts: Vec<_> = release.split('.').collect();
            parts.len() >= 2
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
}

fn numeric_parts(version: &str) -> Option<Vec<u64>> {
    version
        .trim()
        .split(['-', '+'])
        .next()?
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}

fn compare(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| {
            a.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|ordering| ordering.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Comma-separated comparators: `>=`, `>`, `<=`, `<`, `=`.
fn satisfies(version: &str, requirement: &str) -> bool {
    let Some(found) = numeric_parts(version) else {
        return false;
    };
    requirement
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .all(|comparator| {
            let (op, rest) = ["<=", ">=", "<", ">", "="]
                .iter()
                .find_map(|op| comparator.strip_prefix(op).map(|rest| (*op, rest)))
                .unwrap_or(("=", comparator));
            let Some(bound) = numeric_parts(rest) else {
                return false;
            };
            let ordering = compare(&found, &bound);
            match op {
                "<=" => ordering.is_le(),
                ">=" => ordering.is_ge(),
                "<" => ordering.is_lt(),
                ">" => ordering.is_gt(),
                _ => ordering.is_eq(),
            }
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        commands: HashMap<PathBuf, CommandOutput>,
        env: HashMap<&'static str, OsString>,
        files: Vec<PathBuf>,
    }

    impl FakeRunner {
        fn ok(mut self, program: &str, stdout: &str) -> Self {
            self.commands.insert(
                PathBuf::from(program),
                CommandOutput {
                    success: true,
                    stdout: stdout.into(),
                    stderr: String::new(),
                },
            );
            self
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &Path, _args: &[&str]) -> io::Result<CommandOutput> {
            self.commands
                .get(program)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn env_var(&self, key: &str) -> Option<OsString> {
            self.env.get(key).cloned()
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|f| f == path)
        }
    }

    #[test]
    fn nothing_installed_is_all_missing() {
        let probes = probe_all(&FakeRunner::default());
        assert_eq!(probes.len(), 5);
        assert!(probes.iter().all(|p| p.state == CapabilityState::Missing));
        assert_eq!(
            serde_json::to_value(&probes[0]).unwrap(),
            json!({"capability": "marker", "state": "missing", "found_version": null,
                   "required": ">=2.0, <3", "install_hint": "scripts/install/marker.sh"})
        );
    }

    #[test]
    fn version_commands() {
        let runner = FakeRunner::default()
            .ok("codex", "codex-cli 0.46.0\n")
            .ok("claude", "1.0.88 (Claude Code)\n")
            .ok(
                "lean",
                "Lean (version 4.22.0, x86_64-unknown-linux-gnu, commit abc, Release)\n",
            );
        let codex = probe_capability(&runner, Capability::Codex);
        assert_eq!(codex.state, CapabilityState::Available);
        assert_eq!(codex.found_version.as_deref(), Some("0.46.0"));
        let claude = probe_capability(&runner, Capability::ClaudeCode);
        assert_eq!(claude.found_version.as_deref(), Some("1.0.88"));
        let lean = probe_capability(&runner, Capability::Lean);
        assert_eq!(lean.found_version.as_deref(), Some("4.22.0"));
        assert_eq!(
            probe_capability(&runner, Capability::Hmz).state,
            CapabilityState::Missing
        );
    }

    #[test]
    fn marker_via_default_venv_and_version_gate() {
        let mut runner =
            FakeRunner::default().ok("/h/.local/share/mathscape/marker/venv/bin/python", "2.1.0");
        runner.env.insert("HOME", "/h".into());
        runner.files = vec![
            "/h/.local/share/mathscape/marker/venv/bin/marker_single".into(),
            "/h/.local/share/mathscape/marker/venv/bin/python".into(),
        ];
        let probe = probe_capability(&runner, Capability::Marker);
        assert_eq!(probe.state, CapabilityState::Available);
        assert_eq!(probe.found_version.as_deref(), Some("2.1.0"));

        let runner = FakeRunner {
            commands: HashMap::from([(
                PathBuf::from("/m/bin/python"),
                CommandOutput {
                    success: true,
                    stdout: "1.10.0".into(),
                    stderr: String::new(),
                },
            )]),
            env: HashMap::from([("MATHSCAPE_MARKER_EXECUTABLE", "/m/bin/marker_single".into())]),
            files: vec!["/m/bin/marker_single".into(), "/m/bin/python".into()],
        };
        let probe = probe_capability(&runner, Capability::Marker);
        assert_eq!(probe.state, CapabilityState::Version);
        assert_eq!(probe.found_version.as_deref(), Some("1.10.0"));
    }

    #[test]
    fn marker_without_readable_version_is_version_state() {
        let runner = FakeRunner {
            env: HashMap::from([("MATHSCAPE_MARKER_EXECUTABLE", "/m/bin/marker_single".into())]),
            files: vec!["/m/bin/marker_single".into()],
            ..FakeRunner::default()
        };
        let probe = probe_capability(&runner, Capability::Marker);
        assert_eq!(probe.state, CapabilityState::Version);
        assert_eq!(probe.found_version, None);
    }

    #[test]
    fn requirement_matching() {
        assert!(satisfies("2.0.0", ">=2.0, <3"));
        assert!(satisfies("2.9.1-rc1", ">=2.0, <3"));
        assert!(!satisfies("3.0.0", ">=2.0, <3"));
        assert!(!satisfies("1.9", ">=2.0"));
        assert!(satisfies("1.2", "=1.2.0"));
        assert_eq!(extract_version("v1.2.3\n").as_deref(), Some("1.2.3"));
        assert_eq!(extract_version("no version here"), None);
    }

    #[test]
    fn system_runner_reports_missing_binary() {
        let runner = SystemCommandRunner::default();
        assert!(
            runner
                .run(
                    Path::new("mathscape-definitely-not-installed-xyz"),
                    &["--version"]
                )
                .is_err()
        );
    }
}
