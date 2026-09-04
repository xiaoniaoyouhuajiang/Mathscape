use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{MarkerExecutionError, MarkerRequestError, error::validate_absolute_path};

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Parent directory under which this process may allocate isolated Marker runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerStagingArea {
    root: PathBuf,
}

impl MarkerStagingArea {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, MarkerRequestError> {
        Ok(Self {
            root: validate_absolute_path(root.into(), "marker.staging_area")?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Creates one uniquely owned, initially empty run directory.
    pub fn allocate(&self) -> Result<MarkerStagingDirectory, MarkerExecutionError> {
        fs::create_dir_all(&self.root).map_err(|source| MarkerExecutionError::Staging {
            path: self.root.clone(),
            source,
        })?;
        for _ in 0..32 {
            let sequence = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let name = format!("marker-{timestamp}-{}-{sequence}", process::id());
            let path = self.root.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(MarkerStagingDirectory { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(MarkerExecutionError::Staging { path, source });
                }
            }
        }
        Err(MarkerExecutionError::Staging {
            path: self.root.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique Marker run directory",
            ),
        })
    }
}

/// A directory known to have been created exclusively for one Marker run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerStagingDirectory {
    path: PathBuf,
}

impl MarkerStagingDirectory {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Explicitly removes only this owned run directory.
    pub fn cleanup(self) -> Result<(), MarkerExecutionError> {
        match fs::remove_dir_all(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(MarkerExecutionError::Staging {
                path: self.path,
                source,
            }),
        }
    }
}
