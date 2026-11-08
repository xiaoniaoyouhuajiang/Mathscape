use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{LeanEnvironmentProbe, LeanEnvironmentState, LeanFingerprintError, LeanProjectConfig};

pub const LEAN_FINGERPRINT_ALGORITHM: &str = "sha256";
pub const LEAN_FINGERPRINT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanFingerprintStatus {
    /// The environment is complete enough for cache reuse.
    Verified,
    /// A path/dirty dependency prevents safe cache reuse.
    Unverified,
    /// The project is incomplete or the manifest identity is unavailable.
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanFingerprintFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanEnvironmentFingerprint {
    pub schema_version: u32,
    pub algorithm: String,
    pub digest: String,
    pub status: LeanFingerprintStatus,
    pub project_root: String,
    pub lean_executable: Option<String>,
    pub lake_executable: Option<String>,
    pub elan_executable: Option<String>,
    pub cache_root: Option<String>,
    pub lean_version: Option<String>,
    pub lake_version: Option<String>,
    pub elan_version: Option<String>,
    pub lake_env_lean_version: Option<String>,
    pub mathlib_root: Option<String>,
    pub mathlib_build_available: bool,
    pub files: Vec<LeanFingerprintFile>,
    pub warnings: Vec<String>,
}

impl LeanEnvironmentFingerprint {
    pub fn cache_reusable(&self) -> bool {
        self.status == LeanFingerprintStatus::Verified
    }

    pub fn same_environment(&self, other: &Self) -> bool {
        self.algorithm == other.algorithm && self.digest == other.digest
    }
}

/// Fingerprints the project/toolchain identity without reading or writing
/// source declarations. The digest covers the project root, versions, and
/// contents of the Lake configuration files; source/path dependencies are
/// explicitly marked unverified instead of pretending to be reproducible.
pub fn fingerprint_environment(
    config: &LeanProjectConfig,
    probe: &LeanEnvironmentProbe,
) -> Result<LeanEnvironmentFingerprint, LeanFingerprintError> {
    config.validate_project_root()?;
    let project_root =
        fs::canonicalize(config.project_root()).map_err(|source| LeanFingerprintError::Access {
            path: config.project_root().to_owned(),
            source,
        })?;
    let mut files = Vec::new();
    for relative in [
        "lean-toolchain",
        "lakefile.lean",
        "lakefile.toml",
        "lake-manifest.json",
    ] {
        let path = project_root.join(relative);
        if path.is_file() {
            files.push(fingerprint_file(&project_root, relative, &path)?);
        }
    }

    let manifest = project_root.join("lake-manifest.json");
    let mut warnings = probe.diagnostics.clone();
    let mut path_dependency = false;
    let mut dirty_dependency = false;
    if manifest.is_file() {
        let contents =
            fs::read_to_string(&manifest).map_err(|source| LeanFingerprintError::Read {
                path: manifest.clone(),
                source,
            })?;
        let value: Value = serde_json::from_str(&contents).map_err(|source| {
            LeanFingerprintError::InvalidManifest {
                path: manifest.clone(),
                source,
            }
        })?;
        path_dependency = contains_unverified_dependency(&value, &project_root);
        dirty_dependency = contains_dirty_dependency(&value);
    } else {
        warnings.push("lake-manifest.json is absent; dependency revision is unknown".to_owned());
    }
    if probe
        .mathlib_root
        .as_deref()
        .is_some_and(|path| path.join(".git").exists())
    {
        dirty_dependency = true;
        warnings
            .push("mathlib has a local .git directory; dependency state is unverified".to_owned());
    }
    if path_dependency {
        warnings
            .push("Lake manifest contains a path dependency; cache reuse is disabled".to_owned());
    }
    if dirty_dependency {
        warnings.push(
            "Lake manifest contains a dirty or local dependency; cache reuse is disabled"
                .to_owned(),
        );
    }

    let status = if probe.state != LeanEnvironmentState::Ready {
        LeanFingerprintStatus::Incomplete
    } else if path_dependency || dirty_dependency {
        LeanFingerprintStatus::Unverified
    } else if manifest.is_file() {
        LeanFingerprintStatus::Verified
    } else {
        LeanFingerprintStatus::Incomplete
    };

    let mut fingerprint = LeanEnvironmentFingerprint {
        schema_version: LEAN_FINGERPRINT_SCHEMA_VERSION,
        algorithm: LEAN_FINGERPRINT_ALGORITHM.to_owned(),
        digest: String::new(),
        status,
        project_root: project_root.to_string_lossy().into_owned(),
        lean_executable: probe.lean_executable.as_deref().map(path_string),
        lake_executable: probe.lake_executable.as_deref().map(path_string),
        elan_executable: probe.elan_executable.as_deref().map(path_string),
        cache_root: config.cache_root().map(path_string),
        lean_version: probe.lean_version.clone(),
        lake_version: probe.lake_version.clone(),
        elan_version: probe.elan_version.clone(),
        lake_env_lean_version: probe.lake_env_lean_version.clone(),
        mathlib_root: probe.mathlib_root.as_deref().map(path_string),
        mathlib_build_available: probe.mathlib_build_available,
        files,
        warnings,
    };
    let bytes = serde_json::to_vec(&fingerprint).map_err(|error| LeanFingerprintError::Read {
        path: project_root.clone(),
        source: io::Error::other(error),
    })?;
    fingerprint.digest = format!("sha256:{}", hex(&Sha256::digest(bytes)));
    Ok(fingerprint)
}

fn fingerprint_file(
    root: &Path,
    relative: &str,
    path: &Path,
) -> Result<LeanFingerprintFile, LeanFingerprintError> {
    let metadata = fs::metadata(path).map_err(|source| LeanFingerprintError::Access {
        path: path.to_owned(),
        source,
    })?;
    let sha256 = hash_file(path).map_err(|source| LeanFingerprintError::Read {
        path: path.to_owned(),
        source,
    })?;
    let relative = path
        .strip_prefix(root)
        .ok()
        .and_then(|path| path.to_str())
        .unwrap_or(relative)
        .replace(std::path::MAIN_SEPARATOR, "/");
    Ok(LeanFingerprintFile {
        path: relative,
        size: metadata.len(),
        sha256,
    })
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn contains_unverified_dependency(value: &Value, project_root: &Path) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            if key.eq_ignore_ascii_case("type") && value.as_str() == Some("path") {
                let dir = object.get("dir").and_then(Value::as_str);
                let revision = object.get("source_revision").and_then(Value::as_str);
                return match (dir, revision) {
                    (Some(dir), Some(revision)) if !revision.is_empty() => {
                        !project_root.join(dir).is_dir()
                    }
                    _ => true,
                };
            }
            if key.eq_ignore_ascii_case("path") && value.as_str().is_some() {
                return true;
            }
            contains_unverified_dependency(value, project_root)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_unverified_dependency(value, project_root)),
        _ => false,
    }
}

fn contains_dirty_dependency(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key.eq_ignore_ascii_case("dirty") && value.as_bool() == Some(true))
                || contains_dirty_dependency(value)
        }),
        Value::Array(values) => values.iter().any(contains_dirty_dependency),
        _ => false,
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

    use crate::{LeanEnvironmentProbe, LeanProjectConfig};

    use super::*;

    #[test]
    fn digest_changes_when_toolchain_configuration_changes() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "package demo").unwrap();
        fs::write(temp.path().join("lean-toolchain"), "v4.20.0").unwrap();
        fs::write(
            temp.path().join("lake-manifest.json"),
            r#"{"packages":[{"name":"mathlib","rev":"abc"}]}"#,
        )
        .unwrap();
        fs::create_dir_all(temp.path().join(".lake/packages/mathlib")).unwrap();
        let config = LeanProjectConfig::new(temp.path()).unwrap();
        let probe = LeanEnvironmentProbe {
            state: LeanEnvironmentState::Ready,
            project_root: temp.path().to_owned(),
            lean_executable: Some(PathBuf::from("/bin/lean")),
            lake_executable: Some(PathBuf::from("/bin/lake")),
            elan_executable: None,
            lean_version: Some("Lean 4.20.0".to_owned()),
            lake_version: Some("Lake 5".to_owned()),
            elan_version: None,
            lake_env_lean_version: Some("Lean 4.20.0".to_owned()),
            mathlib_root: Some(temp.path().join(".lake/packages/mathlib")),
            mathlib_build_available: true,
            project_files: vec!["lakefile.lean".to_owned()],
            diagnostics: Vec::new(),
        };
        let first = fingerprint_environment(&config, &probe).unwrap();
        assert!(first.cache_reusable());
        fs::write(temp.path().join("lean-toolchain"), "v4.21.0").unwrap();
        let second = fingerprint_environment(&config, &probe).unwrap();
        assert!(!first.same_environment(&second));
    }

    #[test]
    fn path_dependencies_are_explicitly_unverified() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "package demo").unwrap();
        fs::write(
            temp.path().join("lake-manifest.json"),
            r#"{"packages":[{"name":"mathlib","source":{"path":"../mathlib"}}]}"#,
        )
        .unwrap();
        let config = LeanProjectConfig::new(temp.path()).unwrap();
        let probe = LeanEnvironmentProbe {
            state: LeanEnvironmentState::Ready,
            project_root: temp.path().to_owned(),
            lean_executable: None,
            lake_executable: None,
            elan_executable: None,
            lean_version: None,
            lake_version: None,
            elan_version: None,
            lake_env_lean_version: None,
            mathlib_root: None,
            mathlib_build_available: false,
            project_files: vec!["lakefile.lean".to_owned()],
            diagnostics: Vec::new(),
        };
        let fingerprint = fingerprint_environment(&config, &probe).unwrap();
        assert_eq!(fingerprint.status, LeanFingerprintStatus::Unverified);
        assert!(!fingerprint.cache_reusable());
    }

    #[test]
    fn managed_path_dependencies_with_locked_revisions_are_reusable() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "package demo").unwrap();
        fs::create_dir_all(temp.path().join(".lake/packages/mathlib")).unwrap();
        fs::write(
            temp.path().join("lake-manifest.json"),
            r#"{"packages":[{"name":"mathlib","type":"path","dir":".lake/packages/mathlib","source_revision":"abc"}]}"#,
        )
        .unwrap();
        let config = LeanProjectConfig::new(temp.path()).unwrap();
        let probe = LeanEnvironmentProbe {
            state: LeanEnvironmentState::Ready,
            project_root: temp.path().to_owned(),
            lean_executable: None,
            lake_executable: None,
            elan_executable: None,
            lean_version: None,
            lake_version: None,
            elan_version: None,
            lake_env_lean_version: None,
            mathlib_root: Some(temp.path().join(".lake/packages/mathlib")),
            mathlib_build_available: true,
            project_files: vec!["lake-manifest.json".to_owned()],
            diagnostics: Vec::new(),
        };
        let fingerprint = fingerprint_environment(&config, &probe).unwrap();
        assert_eq!(fingerprint.status, LeanFingerprintStatus::Verified);
        assert!(fingerprint.cache_reusable());
    }
}
