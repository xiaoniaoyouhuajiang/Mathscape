use std::path::{Path, PathBuf};

use crate::{MarkerRequestError, error::validate_absolute_path};

/// Configured entry point of an external Marker installation.
///
/// Availability and version probing belong to the process-execution slice;
/// construction here only establishes an unambiguous executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerInstallation {
    executable: PathBuf,
    python_executable: PathBuf,
    working_directory: PathBuf,
}

impl MarkerInstallation {
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, MarkerRequestError> {
        let executable = validate_absolute_path(executable.into(), "marker.executable")?;
        let python_executable = executable
            .parent()
            .expect("an absolute executable path has a parent")
            .join("python");
        let working_directory = default_working_directory(&executable);
        Ok(Self {
            executable,
            python_executable,
            working_directory,
        })
    }

    pub fn with_python(
        executable: impl Into<PathBuf>,
        python_executable: impl Into<PathBuf>,
    ) -> Result<Self, MarkerRequestError> {
        let executable = validate_absolute_path(executable.into(), "marker.executable")?;
        Ok(Self {
            working_directory: default_working_directory(&executable),
            executable,
            python_executable: validate_absolute_path(
                python_executable.into(),
                "marker.python_executable",
            )?,
        })
    }

    pub fn with_working_directory(
        mut self,
        working_directory: impl Into<PathBuf>,
    ) -> Result<Self, MarkerRequestError> {
        self.working_directory =
            validate_absolute_path(working_directory.into(), "marker.working_directory")?;
        Ok(self)
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn python_executable(&self) -> &Path {
        &self.python_executable
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
}

fn default_working_directory(executable: &Path) -> PathBuf {
    executable
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| executable.parent().expect("absolute path has a parent"))
        .to_owned()
}
