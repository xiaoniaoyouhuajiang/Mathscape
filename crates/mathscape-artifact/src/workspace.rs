use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{ArtifactUri, ContentHash, StableId};

use crate::{ArtifactPoolError, error::io_error};

pub const WORKSPACE_FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "workspace.toml";
const DATABASE: &str = "mathscape.sqlite3";
static WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRoot {
    root: PathBuf,
    workspace_id: StableId,
}

impl WorkspaceRoot {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, ArtifactPoolError> {
        let requested = absolute(path.as_ref())?;
        fs::create_dir_all(&requested)
            .map_err(|source| io_error("create workspace directory", &requested, source))?;
        let root = fs::canonicalize(&requested)
            .map_err(|source| io_error("canonicalize workspace directory", &requested, source))?;
        let workspace_id = initialize_manifest(&root)?;
        let workspace = Self { root, workspace_id };
        workspace.initialize_directories()?;
        Ok(workspace)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArtifactPoolError> {
        let requested = absolute(path.as_ref())?;
        let root = fs::canonicalize(&requested)
            .map_err(|source| io_error("open workspace directory", &requested, source))?;
        let workspace_id = validate_manifest(&root)?;
        let workspace = Self { root, workspace_id };
        workspace.initialize_directories()?;
        Ok(workspace)
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn workspace_id(&self) -> &StableId {
        &self.workspace_id
    }

    pub fn database_path(&self) -> PathBuf {
        self.root.join(DATABASE)
    }

    pub fn objects_path(&self) -> PathBuf {
        self.root.join("objects")
    }

    pub fn staging_path(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn quarantine_path(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    pub fn cache_path(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn object_path(&self, hash: &ContentHash) -> PathBuf {
        let digest = digest(hash);
        self.objects_path()
            .join("sha256")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(digest)
    }

    pub fn pool_uri(&self, hash: &ContentHash) -> Result<ArtifactUri, ArtifactPoolError> {
        ArtifactUri::parse(format!("pool:sha256/{}", digest(hash))).map_err(Into::into)
    }

    pub fn resolve_pool_uri(&self, uri: &ArtifactUri) -> Result<PathBuf, ArtifactPoolError> {
        let Some(value) = uri.as_str().strip_prefix("pool:sha256/") else {
            return Err(ArtifactPoolError::InvalidPoolUri(uri.as_str().to_owned()));
        };
        let hash = ContentHash::parse(format!("sha256:{value}"))?;
        if format!("pool:sha256/{}", digest(&hash)) != uri.as_str() {
            return Err(ArtifactPoolError::InvalidPoolUri(uri.as_str().to_owned()));
        }
        Ok(self.object_path(&hash))
    }

    pub(crate) fn validate_object_parent(&self, parent: &Path) -> Result<(), ArtifactPoolError> {
        let object_root = self.objects_path();
        let objects = fs::canonicalize(&object_root)
            .map_err(|source| io_error("canonicalize managed object root", &object_root, source))?;
        let parent = fs::canonicalize(parent)
            .map_err(|source| io_error("canonicalize managed object directory", parent, source))?;
        if !parent.starts_with(&objects) {
            return Err(ArtifactPoolError::UnsafeManagedPath(parent));
        }
        Ok(())
    }

    fn initialize_directories(&self) -> Result<(), ArtifactPoolError> {
        for path in [
            self.objects_path(),
            self.staging_path(),
            self.quarantine_path(),
            self.cache_path(),
        ] {
            fs::create_dir_all(&path)
                .map_err(|source| io_error("create workspace subdirectory", &path, source))?;
            let metadata = fs::symlink_metadata(&path)
                .map_err(|source| io_error("inspect workspace subdirectory", &path, source))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ArtifactPoolError::UnsafeManagedPath(path));
            }
        }
        Ok(())
    }
}

fn initialize_manifest(root: &Path) -> Result<StableId, ArtifactPoolError> {
    let path = root.join(MANIFEST);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            let workspace_id = new_workspace_id()?;
            let content = format!(
                "format_version = {WORKSPACE_FORMAT_VERSION}\nworkspace_id = \"{}\"\n",
                workspace_id.as_str()
            );
            file.write_all(content.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|source| io_error("write workspace manifest", &path, source))?;
            Ok(workspace_id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => validate_manifest(root),
        Err(source) => Err(io_error("create workspace manifest", path, source)),
    }
}

fn validate_manifest(root: &Path) -> Result<StableId, ArtifactPoolError> {
    let path = root.join(MANIFEST);
    let content = fs::read_to_string(&path)
        .map_err(|source| io_error("read workspace manifest", &path, source))?;
    let mut lines = content.lines();
    if lines.next() != Some("format_version = 1") {
        return Err(ArtifactPoolError::InvalidWorkspaceManifest {
            path,
            reason: "expected format_version = 1",
        });
    }
    let Some(workspace_line) = lines.next() else {
        return Err(ArtifactPoolError::InvalidWorkspaceManifest {
            path,
            reason: "workspace_id is missing",
        });
    };
    if lines.next().is_some() {
        return Err(ArtifactPoolError::InvalidWorkspaceManifest {
            path,
            reason: "unexpected manifest fields",
        });
    }
    let Some(value) = workspace_line
        .strip_prefix("workspace_id = \"")
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err(ArtifactPoolError::InvalidWorkspaceManifest {
            path,
            reason: "workspace_id has invalid syntax",
        });
    };
    StableId::parse(value.to_owned()).map_err(|_| ArtifactPoolError::InvalidWorkspaceManifest {
        path,
        reason: "workspace_id is invalid",
    })
}

fn new_workspace_id() -> Result<StableId, ArtifactPoolError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    StableId::parse(format!(
        "mathscape:workspace/{timestamp}-{}-{sequence}",
        process::id()
    ))
    .map_err(Into::into)
}

fn absolute(path: &Path) -> Result<PathBuf, ArtifactPoolError> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|source| io_error("resolve current directory", path, source))
    }
}

fn digest(hash: &ContentHash) -> &str {
    hash.as_str()
        .strip_prefix("sha256:")
        .expect("ContentHash currently validates the sha256 scheme")
}
