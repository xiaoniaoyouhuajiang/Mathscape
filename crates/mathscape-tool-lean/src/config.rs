use std::{
    env,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{LeanConfigError, error::validate_absolute_path};

/// Environment variables accepted by [`LeanProjectConfig::from_env`].
pub const LEAN_PROJECT_ENV: &str = "MATHSCAPE_LEAN_PROJECT";
pub const LEAN_EXECUTABLE_ENV: &str = "MATHSCAPE_LEAN_EXECUTABLE";
pub const LAKE_EXECUTABLE_ENV: &str = "MATHSCAPE_LAKE_EXECUTABLE";
pub const ELAN_EXECUTABLE_ENV: &str = "MATHSCAPE_ELAN_EXECUTABLE";
pub const LEAN_CACHE_ENV: &str = "MATHSCAPE_LEAN_CACHE";

/// Explicit local Lean project configuration.
///
/// The project and executable paths are intentionally absolute. This keeps a
/// later receipt reproducible when the desktop process changes its cwd and
/// makes it impossible for a tool request to silently target another project.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanProjectConfig {
    project_root: PathBuf,
    lean_executable: Option<PathBuf>,
    lake_executable: Option<PathBuf>,
    elan_executable: Option<PathBuf>,
    cache_root: Option<PathBuf>,
}

impl LeanProjectConfig {
    pub fn new(project_root: impl Into<PathBuf>) -> Result<Self, LeanConfigError> {
        Ok(Self {
            project_root: validate_absolute_path(project_root.into(), "lean.project_root")?,
            lean_executable: None,
            lake_executable: None,
            elan_executable: None,
            cache_root: None,
        })
    }

    /// Reads an optional project configuration from the process environment.
    /// Missing `MATHSCAPE_LEAN_PROJECT` returns `Ok(None)`; no command is run.
    pub fn from_env() -> Result<Option<Self>, LeanConfigError> {
        let Some(project) = env::var_os(LEAN_PROJECT_ENV) else {
            return Ok(None);
        };
        let mut config = Self::new(PathBuf::from(project))?;
        if let Some(path) = env::var_os(LEAN_EXECUTABLE_ENV) {
            config = config.with_lean_executable(path)?;
        }
        if let Some(path) = env::var_os(LAKE_EXECUTABLE_ENV) {
            config = config.with_lake_executable(path)?;
        }
        if let Some(path) = env::var_os(ELAN_EXECUTABLE_ENV) {
            config = config.with_elan_executable(path)?;
        }
        if let Some(path) = env::var_os(LEAN_CACHE_ENV) {
            config = config.with_cache_root(path)?;
        }
        Ok(Some(config))
    }

    pub fn with_lean_executable(
        mut self,
        path: impl Into<PathBuf>,
    ) -> Result<Self, LeanConfigError> {
        self.lean_executable = Some(validate_absolute_path(path.into(), "lean.lean_executable")?);
        Ok(self)
    }

    pub fn with_lake_executable(
        mut self,
        path: impl Into<PathBuf>,
    ) -> Result<Self, LeanConfigError> {
        self.lake_executable = Some(validate_absolute_path(path.into(), "lean.lake_executable")?);
        Ok(self)
    }

    pub fn with_elan_executable(
        mut self,
        path: impl Into<PathBuf>,
    ) -> Result<Self, LeanConfigError> {
        self.elan_executable = Some(validate_absolute_path(path.into(), "lean.elan_executable")?);
        Ok(self)
    }

    pub fn with_cache_root(mut self, path: impl Into<PathBuf>) -> Result<Self, LeanConfigError> {
        self.cache_root = Some(validate_absolute_path(path.into(), "lean.cache_root")?);
        Ok(self)
    }

    /// Checks the project root without touching any project files.
    pub fn validate_project_root(&self) -> Result<(), LeanConfigError> {
        let metadata =
            std::fs::metadata(&self.project_root).map_err(|source| LeanConfigError::Access {
                field: "lean.project_root",
                path: self.project_root.clone(),
                source,
            })?;
        if !metadata.is_dir() {
            return Err(LeanConfigError::NotADirectory {
                field: "lean.project_root",
                path: self.project_root.clone(),
            });
        }
        Ok(())
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
    pub fn lean_executable(&self) -> Option<&Path> {
        self.lean_executable.as_deref()
    }
    pub fn lake_executable(&self) -> Option<&Path> {
        self.lake_executable.as_deref()
    }
    pub fn elan_executable(&self) -> Option<&Path> {
        self.elan_executable.as_deref()
    }
    pub fn cache_root(&self) -> Option<&Path> {
        self.cache_root.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_project_and_tool_paths() {
        assert!(matches!(
            LeanProjectConfig::new("project"),
            Err(LeanConfigError::RelativePath {
                field: "lean.project_root",
                ..
            })
        ));
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(matches!(
            LeanProjectConfig::new(root)
                .unwrap()
                .with_lake_executable("lake"),
            Err(LeanConfigError::RelativePath {
                field: "lean.lake_executable",
                ..
            })
        ));
    }
}
