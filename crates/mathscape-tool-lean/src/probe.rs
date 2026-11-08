use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    LeanProbeError, LeanProjectConfig, LeanRunStatus, LeanRunner, LeanRunnerOptions,
    runner::{LeanCancellation, LeanCommand},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanEnvironmentState {
    /// No usable Lean executable was found, either directly or through Lake.
    NoLean,
    /// A Lean executable exists, but no Lake executable was found.
    NoLake,
    /// Lake exists, but its configured project has no detectable mathlib.
    NoMathlib,
    /// The project/toolchain files exist but `lake env lean` could not start.
    ToolchainUnavailable,
    /// Mathlib is present, but its compiled `.olean` entry point is missing.
    BuildUnavailable,
    /// The project has a Lake toolchain and a detectable mathlib package.
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanEnvironmentProbe {
    pub state: LeanEnvironmentState,
    pub project_root: PathBuf,
    pub lean_executable: Option<PathBuf>,
    pub lake_executable: Option<PathBuf>,
    pub elan_executable: Option<PathBuf>,
    pub lean_version: Option<String>,
    pub lake_version: Option<String>,
    pub elan_version: Option<String>,
    pub lake_env_lean_version: Option<String>,
    pub mathlib_root: Option<PathBuf>,
    pub mathlib_build_available: bool,
    pub project_files: Vec<String>,
    pub diagnostics: Vec<String>,
}

impl LeanEnvironmentProbe {
    /// Probes with a short, read-only version command runner.
    pub fn collect(config: &LeanProjectConfig) -> Result<Self, LeanProbeError> {
        let options = LeanRunnerOptions::new(Duration::from_secs(10))
            .map_err(|error| LeanProbeError::Internal(error.to_string()))?;
        Self::collect_with_runner(config, &LeanRunner::new(options))
    }

    /// The runner is injectable so tests can use fixture executables and so a
    /// desktop caller can share its process policy with later checks.
    pub fn collect_with_runner(
        config: &LeanProjectConfig,
        runner: &LeanRunner,
    ) -> Result<Self, LeanProbeError> {
        config.validate_project_root()?;
        let root = config.project_root().to_owned();
        let mut diagnostics = Vec::new();
        let project_files = project_files(&root);
        let lean_executable = resolve_executable(config.lean_executable(), "lean");
        let lake_executable = resolve_executable(config.lake_executable(), "lake");
        let elan_executable = resolve_executable(config.elan_executable(), "elan");
        let lean_version = run_version(runner, lean_executable.as_deref(), &root, &mut diagnostics);
        let lake_version = run_version(runner, lake_executable.as_deref(), &root, &mut diagnostics);
        let elan_version = run_version(runner, elan_executable.as_deref(), &root, &mut diagnostics);

        // `lake env` may create a missing lake-manifest.json as a side effect
        // (and may query the network while doing so). A project without a
        // manifest is incomplete for a reproducible mathlib environment, so
        // probe it without invoking that command.
        let lake_env_lean_version = if root.join("lake-manifest.json").is_file() {
            lake_executable.as_deref().and_then(|lake| {
                run_command_version(
                    runner,
                    lake,
                    &["env", "lean", "--version"],
                    &root,
                    &mut diagnostics,
                )
            })
        } else {
            diagnostics.push(
                "skipped `lake env lean --version`: lake-manifest.json is absent and the probe is read-only"
                    .to_owned(),
            );
            None
        };
        let mathlib_root = detect_mathlib(&root);
        let mathlib_build_available = mathlib_root.as_deref().is_some_and(mathlib_build_exists);
        let lake_ok = lake_executable.is_some() && lake_version.is_some();
        let lean_ok = lean_version.is_some() || lake_env_lean_version.is_some();
        let state = if !lean_ok {
            LeanEnvironmentState::NoLean
        } else if !lake_ok {
            LeanEnvironmentState::NoLake
        } else if lake_env_lean_version.is_none() {
            LeanEnvironmentState::ToolchainUnavailable
        } else if mathlib_root.is_none() {
            LeanEnvironmentState::NoMathlib
        } else if !mathlib_build_available {
            LeanEnvironmentState::BuildUnavailable
        } else {
            LeanEnvironmentState::Ready
        };

        if state == LeanEnvironmentState::NoMathlib {
            diagnostics
                .push("no checked-out .lake/packages/mathlib directory was detected".to_owned());
        } else if state == LeanEnvironmentState::BuildUnavailable {
            diagnostics.push(
                "mathlib source is present, but the compiled Mathlib.olean entry point is missing"
                    .to_owned(),
            );
        }
        Ok(Self {
            state,
            project_root: root,
            lean_executable,
            lake_executable,
            elan_executable,
            lean_version,
            lake_version,
            elan_version,
            lake_env_lean_version,
            mathlib_root,
            mathlib_build_available,
            project_files,
            diagnostics,
        })
    }

    pub const fn is_ready(&self) -> bool {
        matches!(self.state, LeanEnvironmentState::Ready)
    }
}

fn project_files(root: &Path) -> Vec<String> {
    [
        "lean-toolchain",
        "lakefile.lean",
        "lakefile.toml",
        "lake-manifest.json",
    ]
    .into_iter()
    .filter(|name| root.join(name).is_file())
    .map(str::to_owned)
    .collect()
}

fn detect_mathlib(root: &Path) -> Option<PathBuf> {
    let package = root.join(".lake/packages/mathlib");
    if package.is_dir() {
        return Some(package);
    }
    let manifest = root.join("lake-manifest.json");
    if root.join("Mathlib.lean").is_file()
        && fs::read_to_string(manifest)
            .ok()
            .is_some_and(|contents| contents.contains("\"name\": \"mathlib\""))
    {
        return Some(root.to_owned());
    }
    // A manifest entry without the checked-out package is an incomplete
    // environment. In particular, probing must never treat a stale manifest
    // as proof that mathlib can be imported.
    None
}

fn mathlib_build_exists(mathlib_root: &Path) -> bool {
    mathlib_root
        .join(".lake/build/lib/lean/Mathlib.olean")
        .is_file()
}

fn resolve_executable(explicit: Option<&Path>, name: &str) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_owned());
    }
    let path_var = env::var_os("PATH")?;
    for directory in env::split_paths(&path_var) {
        let candidate = directory.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    let home = env::var_os("HOME").map(PathBuf::from)?;
    let candidate = home.join(".elan/bin").join(name);
    is_executable(&candidate).then_some(candidate)
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn run_version(
    runner: &LeanRunner,
    program: Option<&Path>,
    root: &Path,
    diagnostics: &mut Vec<String>,
) -> Option<String> {
    program
        .and_then(|program| run_command_version(runner, program, &["--version"], root, diagnostics))
}

fn run_command_version(
    runner: &LeanRunner,
    program: &Path,
    arguments: &[&str],
    root: &Path,
    diagnostics: &mut Vec<String>,
) -> Option<String> {
    let command = LeanCommand::new(program.to_owned(), root.to_owned())
        .args(arguments.iter().map(|argument| (*argument).to_owned()));
    match runner.run(&command, &LeanCancellation::new()) {
        Ok(run) if run.status() == LeanRunStatus::Succeeded => {
            let output = if run.stdout().bytes().is_empty() {
                run.stderr().lossy()
            } else {
                run.stdout().lossy()
            };
            extract_version(&output)
        }
        Ok(run) => {
            diagnostics.push(format!(
                "{} {:?} exited with {:?}: {}",
                program.display(),
                arguments,
                run.status(),
                run.stderr().lossy()
            ));
            None
        }
        Err(error) => {
            diagnostics.push(format!(
                "{} {:?} could not be run: {error}",
                program.display(),
                arguments
            ));
            None
        }
    }
}

fn extract_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use tempfile::TempDir;

    use super::*;

    fn executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn classifies_a_fixture_project_as_ready() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "package demo").unwrap();
        fs::write(temp.path().join("lean-toolchain"), "v4.20.0").unwrap();
        fs::write(temp.path().join("lake-manifest.json"), "{\"packages\":[]}").unwrap();
        fs::create_dir_all(temp.path().join(".lake/packages/mathlib")).unwrap();
        fs::create_dir_all(
            temp.path()
                .join(".lake/packages/mathlib/.lake/build/lib/lean"),
        )
        .unwrap();
        fs::write(
            temp.path()
                .join(".lake/packages/mathlib/.lake/build/lib/lean/Mathlib.olean"),
            b"fixture",
        )
        .unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let lean = bin.join("lean");
        let lake = bin.join("lake");
        executable(&lean, "#!/bin/sh\nprintf 'Lean 4.20.0\\n'");
        executable(
            &lake,
            "#!/bin/sh\nif [ \"$1\" = env ]; then printf 'Lean 4.20.0\\n'; else printf 'Lake version 5\\n'; fi",
        );
        let config = LeanProjectConfig::new(temp.path())
            .unwrap()
            .with_lean_executable(lean)
            .unwrap()
            .with_lake_executable(lake)
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert_eq!(probe.state, LeanEnvironmentState::Ready);
        assert!(probe.is_ready());
    }

    #[test]
    fn reports_missing_mathlib_without_running_downloads() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "package demo").unwrap();
        fs::write(temp.path().join("lake-manifest.json"), "{\"packages\":[]}").unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let lean = bin.join("lean");
        let lake = bin.join("lake");
        executable(&lean, "#!/bin/sh\nprintf 'Lean 4.20.0\\n'");
        executable(
            &lake,
            "#!/bin/sh\nif [ \"$1\" = env ]; then printf 'Lean 4.20.0\\n'; else printf 'Lake version 5\\n'; fi",
        );
        let config = LeanProjectConfig::new(temp.path())
            .unwrap()
            .with_lean_executable(lean)
            .unwrap()
            .with_lake_executable(lake)
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert_eq!(probe.state, LeanEnvironmentState::NoMathlib);
    }

    #[test]
    fn does_not_invoke_lake_env_when_a_manifest_is_absent() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let lean = bin.join("lean");
        let lake = bin.join("lake");
        let marker = temp.path().join("lake-env-was-run");
        executable(&lean, "#!/bin/sh\nprintf 'Lean 4.20.0\\n'");
        executable(
            &lake,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = env ]; then touch '{}'; exit 0; else printf 'Lake version 5\\n'; fi",
                marker.display()
            ),
        );
        let config = LeanProjectConfig::new(temp.path())
            .unwrap()
            .with_lean_executable(lean)
            .unwrap()
            .with_lake_executable(lake)
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert!(!marker.exists());
        assert!(
            probe
                .diagnostics
                .iter()
                .any(|message| message.contains("read-only"))
        );
    }

    #[test]
    fn classifies_missing_lake_separately_from_a_direct_lean_binary() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let lean = bin.join("lean");
        executable(&lean, "#!/bin/sh\nprintf 'Lean 4.20.0\\n'");
        let config = LeanProjectConfig::new(temp.path())
            .unwrap()
            .with_lean_executable(lean)
            .unwrap()
            .with_lake_executable(temp.path().join("missing-lake"))
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert_eq!(probe.state, LeanEnvironmentState::NoLake);
    }

    #[test]
    fn classifies_missing_lean_when_lake_cannot_enter_a_toolchain() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let lake = bin.join("lake");
        executable(
            &lake,
            "#!/bin/sh\nif [ \"$1\" = env ]; then exit 1; else printf 'Lake version 5\\n'; fi",
        );
        let config = LeanProjectConfig::new(temp.path())
            .unwrap()
            .with_lean_executable(temp.path().join("missing-lean"))
            .unwrap()
            .with_lake_executable(lake)
            .unwrap();
        let probe = LeanEnvironmentProbe::collect(&config).unwrap();
        assert_eq!(probe.state, LeanEnvironmentState::NoLean);
    }
}
