//! Workspace handle shared by the material tools.
//!
//! The workspace manifest and catalog are validated once at startup; every
//! tool call then opens a fresh catalog connection, like the desktop
//! commands do, so results reflect concurrent edits from the desktop app.

use std::path::{Path, PathBuf};

use mathscape_agent::{CommandRunner, SystemCommandRunner, ToolError, ToolErrorCode};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError, WorkspaceRoot};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};

use crate::tools::EnsurePolicy;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceOpenError {
    #[error("无法打开工作区：{0}")]
    Workspace(#[from] ArtifactPoolError),
    #[error("无法打开工作区数据库：{0}")]
    Catalog(#[from] SqliteStoreError),
}

pub struct McpWorkspace {
    root: WorkspaceRoot,
    round: Option<String>,
    capability_runner: Box<dyn CommandRunner>,
    ensure_policy: EnsurePolicy,
}

impl McpWorkspace {
    /// Opens an existing, prepared Mathscape workspace. Never creates or
    /// migrates anything beyond what [`WorkspaceRoot::open`] already does.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceOpenError> {
        let root = WorkspaceRoot::open(path)?;
        SqliteArtifactStore::open(&root.database_path())?;
        Ok(Self {
            root,
            round: None,
            capability_runner: Box::new(SystemCommandRunner::default()),
            ensure_policy: EnsurePolicy::default(),
        })
    }

    /// `--round <token>`: Phase1 keeps it as an opaque placeholder; read-only
    /// tools do not validate it.
    pub fn with_round(mut self, round: Option<String>) -> Self {
        self.round = round;
        self
    }

    /// Replaces the environment seam used for CapabilityProbe (tests).
    pub fn with_capability_runner(mut self, runner: Box<dyn CommandRunner>) -> Self {
        self.capability_runner = runner;
        self
    }

    /// Replaces ensure budgets, locks and waits (tests, desktop overrides).
    pub fn with_ensure_policy(mut self, policy: EnsurePolicy) -> Self {
        self.ensure_policy = policy;
        self
    }

    pub(crate) fn ensure_policy(&self) -> EnsurePolicy {
        self.ensure_policy
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.root.database_path()
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn round(&self) -> Option<&str> {
        self.round.as_deref()
    }

    pub(crate) fn capability_runner(&self) -> &dyn CommandRunner {
        self.capability_runner.as_ref()
    }

    pub(crate) fn store(&self) -> Result<SqliteArtifactStore, ToolError> {
        SqliteArtifactStore::open(&self.root.database_path()).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to open workspace catalog: {error}"),
            )
        })
    }

    pub(crate) fn pool(&self) -> Result<ArtifactPool, ToolError> {
        ArtifactPool::open(self.root.path()).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to open artifact pool: {error}"),
            )
        })
    }
}
