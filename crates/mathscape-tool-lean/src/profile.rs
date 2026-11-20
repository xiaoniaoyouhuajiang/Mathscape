use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    LeanEnvironmentFingerprint, LeanEnvironmentProbe, LeanFingerprintStatus, LeanProfileError,
    LeanProjectConfig, error::validate_absolute_path, fingerprint_environment,
    probe::LeanEnvironmentProbe as Probe,
};

pub const LEAN_PROFILE_SCHEMA_VERSION: u32 = 1;
pub const LEAN_PROFILE_ROOT_ENV: &str = "MATHSCAPE_LEAN_PROFILE_ROOT";
pub const DEFAULT_LEAN_TOOLCHAIN: &str = "leanprover/lean4:v4.33.1";
pub const DEFAULT_MATHLIB_REFERENCE: &str = "v4.33.1";
pub const DEFAULT_PROFILE_ID: &str = "lean4-4.33.1-mathlib-4.33.1";

/// The profile state is deliberately separate from the probe state: a tool
/// may exist while the managed profile's pinned revision is wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanProfileState {
    Ready,
    Incomplete,
    Unverified,
    Mismatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLeanProfileSpec {
    pub id: String,
    pub project_root: PathBuf,
    pub lean_toolchain: String,
    pub mathlib_reference: String,
}

impl ManagedLeanProfileSpec {
    pub fn new(
        id: impl Into<String>,
        project_root: impl Into<PathBuf>,
        lean_toolchain: impl Into<String>,
        mathlib_reference: impl Into<String>,
    ) -> Result<Self, LeanProfileError> {
        let id = id.into();
        validate_profile_id(&id)?;
        let project_root =
            validate_absolute_path(project_root.into(), "lean.profile.project_root")?;
        let lean_toolchain = lean_toolchain.into();
        let mathlib_reference = mathlib_reference.into();
        if lean_toolchain.trim().is_empty() || mathlib_reference.trim().is_empty() {
            return Err(LeanProfileError::InvalidId(
                "toolchain and mathlib reference must not be blank".to_owned(),
            ));
        }
        Ok(Self {
            id,
            project_root,
            lean_toolchain,
            mathlib_reference,
        })
    }

    pub fn default_development(project_root: impl Into<PathBuf>) -> Result<Self, LeanProfileError> {
        Self::new(
            DEFAULT_PROFILE_ID,
            project_root,
            DEFAULT_LEAN_TOOLCHAIN,
            DEFAULT_MATHLIB_REFERENCE,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLeanProfileMetadata {
    pub schema_version: u32,
    pub id: String,
    pub state: LeanProfileState,
    pub project_root: PathBuf,
    pub lean_toolchain: String,
    pub mathlib_reference: String,
    pub detected_mathlib_revision: Option<String>,
    pub fingerprint: LeanEnvironmentFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedLeanProfile {
    metadata: ManagedLeanProfileMetadata,
    probe: LeanEnvironmentProbe,
}

impl ManagedLeanProfile {
    pub fn metadata(&self) -> &ManagedLeanProfileMetadata {
        &self.metadata
    }

    pub fn probe(&self) -> &LeanEnvironmentProbe {
        &self.probe
    }

    pub fn fingerprint(&self) -> &LeanEnvironmentFingerprint {
        &self.metadata.fingerprint
    }

    pub fn state(&self) -> LeanProfileState {
        self.metadata.state
    }

    #[cfg(test)]
    pub(crate) fn from_test_parts(
        metadata: ManagedLeanProfileMetadata,
        probe: LeanEnvironmentProbe,
    ) -> Self {
        Self { metadata, probe }
    }
}

/// Filesystem root for profiles managed by Mathscape. The store never runs
/// Lake update or downloads dependencies; those are explicit installation
/// operations performed before [`register_existing`](Self::register_existing).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanProfileStore {
    root: PathBuf,
}

impl LeanProfileStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, LeanProfileError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(LeanProfileError::RelativeRoot(root));
        }
        fs::create_dir_all(&root).map_err(|source| LeanProfileError::Access {
            path: root.clone(),
            source,
        })?;
        ensure_directory(&root)?;
        Ok(Self { root })
    }

    pub fn from_env_or_default() -> Result<Self, LeanProfileError> {
        if let Some(root) = env::var_os(LEAN_PROFILE_ROOT_ENV) {
            return Self::new(PathBuf::from(root));
        }
        let data_root = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .ok_or_else(|| LeanProfileError::Access {
                path: PathBuf::from("<profile root>"),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "set MATHSCAPE_LEAN_PROFILE_ROOT or XDG_DATA_HOME",
                ),
            })?;
        Self::new(data_root.join("mathscape/lean/profiles"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profile_root(&self, id: &str) -> Result<PathBuf, LeanProfileError> {
        validate_profile_id(id)?;
        Ok(self.root.join(id))
    }

    pub fn metadata_path(&self, id: &str) -> Result<PathBuf, LeanProfileError> {
        Ok(self.profile_root(id)?.join("profile.json"))
    }

    /// Registers an already materialized managed project and writes its
    /// metadata atomically. The project is only read; dependency installation
    /// is intentionally outside this method.
    pub fn register_existing(
        &self,
        spec: &ManagedLeanProfileSpec,
    ) -> Result<ManagedLeanProfile, LeanProfileError> {
        let profile_root = self.profile_root(&spec.id)?;
        fs::create_dir_all(&profile_root).map_err(|source| LeanProfileError::Access {
            path: profile_root.clone(),
            source,
        })?;
        ensure_directory(&profile_root)?;

        let mut config = LeanProjectConfig::new(spec.project_root.clone())?;
        config = config.with_cache_root(profile_root.join("cache"))?;
        let probe = Probe::collect(&config)?;
        let fingerprint = fingerprint_environment(&config, &probe)?;
        let actual_toolchain = read_toolchain(&spec.project_root)?;
        let detected_mathlib_revision = read_mathlib_reference(&spec.project_root)?;
        if actual_toolchain != spec.lean_toolchain {
            return Err(LeanProfileError::ToolchainMismatch {
                id: spec.id.clone(),
                expected: spec.lean_toolchain.clone(),
                actual: actual_toolchain,
            });
        }
        if detected_mathlib_revision.as_deref() != Some(spec.mathlib_reference.as_str()) {
            return Err(LeanProfileError::MathlibMismatch {
                id: spec.id.clone(),
                expected: spec.mathlib_reference.clone(),
                actual: detected_mathlib_revision,
            });
        }

        let state = if !probe.is_ready() || fingerprint.status == LeanFingerprintStatus::Incomplete
        {
            LeanProfileState::Incomplete
        } else if fingerprint.status == LeanFingerprintStatus::Unverified {
            LeanProfileState::Unverified
        } else {
            LeanProfileState::Ready
        };
        let metadata = ManagedLeanProfileMetadata {
            schema_version: LEAN_PROFILE_SCHEMA_VERSION,
            id: spec.id.clone(),
            state,
            project_root: spec.project_root.clone(),
            lean_toolchain: spec.lean_toolchain.clone(),
            mathlib_reference: spec.mathlib_reference.clone(),
            detected_mathlib_revision,
            fingerprint,
        };
        write_metadata(&profile_root.join("profile.json"), &metadata)?;
        Ok(ManagedLeanProfile { metadata, probe })
    }

    pub fn load(&self, id: &str) -> Result<ManagedLeanProfileMetadata, LeanProfileError> {
        let path = self.metadata_path(id)?;
        let bytes = fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                LeanProfileError::NotFound {
                    id: id.to_owned(),
                    path: path.clone(),
                }
            } else {
                LeanProfileError::ReadMetadata {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|source| LeanProfileError::InvalidMetadata { path, source })
    }

    /// Re-probes a registered profile and refreshes its metadata before use.
    pub fn open(&self, id: &str) -> Result<ManagedLeanProfile, LeanProfileError> {
        let metadata = self.load(id)?;
        let spec = ManagedLeanProfileSpec::new(
            metadata.id,
            metadata.project_root,
            metadata.lean_toolchain,
            metadata.mathlib_reference,
        )?;
        self.register_existing(&spec)
    }
}

fn validate_profile_id(id: &str) -> Result<(), LeanProfileError> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !id.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(LeanProfileError::InvalidId(id.to_owned()));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), LeanProfileError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| LeanProfileError::Access {
        path: path.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LeanProfileError::Access {
            path: path.to_owned(),
            source: std::io::Error::other("profile path is not a regular directory"),
        });
    }
    Ok(())
}

fn read_toolchain(project_root: &Path) -> Result<String, LeanProfileError> {
    let path = project_root.join("lean-toolchain");
    let content = fs::read_to_string(&path).map_err(|source| LeanProfileError::Access {
        path: path.clone(),
        source,
    })?;
    Ok(content.trim().to_owned())
}

fn read_mathlib_reference(project_root: &Path) -> Result<Option<String>, LeanProfileError> {
    let path = project_root.join("lake-manifest.json");
    let content = fs::read_to_string(&path).map_err(|source| LeanProfileError::Access {
        path: path.clone(),
        source,
    })?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|source| LeanProfileError::InvalidMetadata { path, source })?;
    Ok(find_mathlib_reference(&value).or_else(|| {
        (value.get("name").and_then(Value::as_str) == Some("mathlib"))
            .then(|| {
                fs::read_to_string(project_root.join("lean-toolchain"))
                    .ok()
                    .and_then(|toolchain| toolchain.trim().rsplit(':').next().map(str::to_owned))
            })
            .flatten()
    }))
}

fn find_mathlib_reference(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            if object.get("name").and_then(Value::as_str) == Some("mathlib") {
                return object
                    .get("inputRev")
                    .and_then(Value::as_str)
                    .or_else(|| object.get("rev").and_then(Value::as_str))
                    .map(str::to_owned);
            }
            object.values().find_map(find_mathlib_reference)
        }
        Value::Array(values) => values.iter().find_map(find_mathlib_reference),
        _ => None,
    }
}

fn write_metadata(
    path: &Path,
    metadata: &ManagedLeanProfileMetadata,
) -> Result<(), LeanProfileError> {
    let bytes = serde_json::to_vec_pretty(metadata).map_err(LeanProfileError::SerializeMetadata)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temporary, bytes).map_err(|source| LeanProfileError::WriteMetadata {
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| LeanProfileError::WriteMetadata {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    #[cfg(unix)]
    use tempfile::TempDir;

    use super::*;

    #[cfg(unix)]
    fn executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    fn fixture() -> (TempDir, TempDir, ManagedLeanProfileSpec) {
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
        fs::create_dir_all(project.path().join(".lake/packages/mathlib")).unwrap();
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
        let lean = bin.join("lean");
        let lake = bin.join("lake");
        executable(&lean, "#!/bin/sh\nprintf 'Lean (version 4.33.1)\\n'");
        executable(
            &lake,
            "#!/bin/sh\nif [ \"$1\" = env ]; then printf 'Lean (version 4.33.1)\\n'; else printf 'Lake version 5\\n'; fi",
        );
        let spec = ManagedLeanProfileSpec::new(
            DEFAULT_PROFILE_ID,
            project.path(),
            DEFAULT_LEAN_TOOLCHAIN,
            DEFAULT_MATHLIB_REFERENCE,
        )
        .unwrap();
        let _ = (lean, lake);
        (store_root, project, spec)
    }

    #[test]
    #[cfg(unix)]
    fn registers_and_loads_a_pinned_profile() {
        let (store_root, _project, spec) = fixture();
        let store = LeanProfileStore::new(store_root.path()).unwrap();
        let profile = store.register_existing(&spec).unwrap();
        assert_eq!(profile.metadata().id, DEFAULT_PROFILE_ID);
        assert!(store.load(DEFAULT_PROFILE_ID).is_ok());
    }

    #[test]
    fn rejects_path_traversal_profile_ids() {
        assert!(matches!(
            ManagedLeanProfileSpec::new("../escape", "/tmp/project", "toolchain", "mathlib"),
            Err(LeanProfileError::InvalidId(_))
        ));
    }
}
