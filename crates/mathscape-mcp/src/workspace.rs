//! Workspace handle shared by the material tools.
//!
//! The workspace manifest and catalog are validated once at startup; every
//! tool call then opens a fresh catalog connection, like the desktop
//! commands do, so results reflect concurrent edits from the desktop app.

use std::path::{Path, PathBuf};

use mathscape_agent::{CommandRunner, SystemCommandRunner, ToolError, ToolErrorCode};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError, WorkspaceRoot};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use mathscape_tool_lean::{LeanProfileRootSource, LeanProfileStore, discover_profile_root};

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
    lean_profile_store: Option<LeanProfileStore>,
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
            lean_profile_store: None,
        })
    }

    /// `--round <token>`: read-only tools keep it as an opaque placeholder;
    /// write-like staging tools require a non-empty bound token.
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

    /// Overrides the managed Lean profile store used by the Lean MCP tools.
    /// Desktop and tests can use this to bind a workspace to an explicit
    /// profile root without changing process-global environment variables.
    pub fn with_lean_profile_store(mut self, store: LeanProfileStore) -> Self {
        self.lean_profile_store = Some(store);
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

    pub(crate) fn lean_profile_store(&self) -> Result<LeanProfileStore, ToolError> {
        if let Some(store) = &self.lean_profile_store {
            return Ok(store.clone());
        }

        // Same search order the desktop uses for the source layer and for
        // sandbox rounds (`mathscape_tool_lean::discover_profile_root`):
        // explicit environment, workspace-local .mathscape roots, cwd, then
        // the platform data directory. An explicit root keeps the store's
        // create-on-open behaviour; discovered roots already exist.
        match discover_profile_root(Some(self.path())) {
            Some(found) if found.source != LeanProfileRootSource::Environment => {
                LeanProfileStore::new(found.root).map_err(|error| {
                    ToolError::new(
                        ToolErrorCode::Internal,
                        format!("failed to open workspace Lean profile store: {error}"),
                    )
                })
            }
            _ => LeanProfileStore::from_env_or_default().map_err(|error| {
                ToolError::new(
                    ToolErrorCode::Internal,
                    format!("failed to open Lean profile store: {error}"),
                )
            }),
        }
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
