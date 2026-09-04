use std::{path::Path, process::Command};

use crate::{
    MarkerExecutionError, MarkerInstallation, MarkerProfile, MarkerRunner,
    execution::{validate_directory, validate_executable},
};

const VERSION_SCRIPT: &str =
    "import importlib.metadata as m; print(m.version('marker-pdf'), end='')";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerVersion {
    raw: String,
    major: u64,
    minor: u64,
    patch: u64,
}

impl MarkerVersion {
    pub fn parse(raw: impl Into<String>) -> Result<Self, MarkerExecutionError> {
        let raw = raw.into();
        let normalized = raw.trim();
        let release = normalized
            .split_once(['-', '+'])
            .map_or(normalized, |(release, _)| release);
        let mut components = release.split('.');
        let major = parse_component(components.next(), &raw)?;
        let minor = parse_component(components.next(), &raw)?;
        let patch = parse_component(components.next(), &raw)?;
        if components.next().is_some() {
            return Err(MarkerExecutionError::InvalidVersion(raw));
        }
        Ok(Self {
            raw: normalized.to_owned(),
            major,
            minor,
            patch,
        })
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub const fn major(&self) -> u64 {
        self.major
    }

    pub const fn minor(&self) -> u64 {
        self.minor
    }

    pub const fn patch(&self) -> u64 {
        self.patch
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerProbe {
    executable: std::path::PathBuf,
    python_executable: std::path::PathBuf,
    version: MarkerVersion,
}

impl MarkerProbe {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn python_executable(&self) -> &Path {
        &self.python_executable
    }

    pub const fn version(&self) -> &MarkerVersion {
        &self.version
    }
}

impl MarkerRunner {
    /// Verifies the configured files and reads Marker package metadata through
    /// the installation's own Python interpreter.
    pub fn probe(
        &self,
        installation: &MarkerInstallation,
    ) -> Result<MarkerProbe, MarkerExecutionError> {
        validate_executable(installation.executable(), "marker.executable")?;
        validate_executable(installation.python_executable(), "marker.python_executable")?;
        validate_directory(installation.working_directory(), "marker.working_directory")?;

        let output = Command::new(installation.python_executable())
            .args(["-c", VERSION_SCRIPT])
            .current_dir(installation.working_directory())
            .env("PYTHONUNBUFFERED", "1")
            .output()
            .map_err(|source| MarkerExecutionError::Access {
                field: "marker.python_executable",
                path: installation.python_executable().to_owned(),
                source,
            })?;
        if !output.status.success() {
            return Err(MarkerExecutionError::ProbeFailed {
                exit_code: output.status.code(),
                stderr: bounded_lossy(&output.stderr, 16 * 1024),
            });
        }
        let version = MarkerVersion::parse(String::from_utf8_lossy(&output.stdout).into_owned())?;
        if version.major() != MarkerProfile::SUPPORTED_MARKER_MAJOR {
            return Err(MarkerExecutionError::UnsupportedVersion {
                expected: MarkerProfile::SUPPORTED_MARKER_MAJOR,
                actual: version.major(),
            });
        }
        Ok(MarkerProbe {
            executable: installation.executable().to_owned(),
            python_executable: installation.python_executable().to_owned(),
            version,
        })
    }
}

fn parse_component(value: Option<&str>, raw: &str) -> Result<u64, MarkerExecutionError> {
    value
        .and_then(|component| component.parse().ok())
        .ok_or_else(|| MarkerExecutionError::InvalidVersion(raw.to_owned()))
}

fn bounded_lossy(bytes: &[u8], maximum: usize) -> String {
    let end = bytes.len().min(maximum);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marker_release_versions() {
        let version = MarkerVersion::parse("2.0.0\n").unwrap();
        assert_eq!(version.raw(), "2.0.0");
        assert_eq!(
            (version.major(), version.minor(), version.patch()),
            (2, 0, 0)
        );
    }

    #[test]
    fn rejects_incomplete_versions() {
        assert!(matches!(
            MarkerVersion::parse("2.0"),
            Err(MarkerExecutionError::InvalidVersion(_))
        ));
    }
}
