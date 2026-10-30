//! S0 sandbox session commands (docs/sandbox-flow.md §3, §5 S0).
//!
//! "Sandbox session" here is the *product* concept: an isolated full copy of
//! a checkpoint workspace that can be discarded or exported. It is unrelated
//! to Codex `SandboxMode` (process permission policy), which S1 will configure
//! separately when a dialogue backend is mounted.
//!
//! S0 is read-only towards the checkpoint (decision T2): no command here writes
//! into a checkpoint root, and the running Desktop workspace is not switched.

use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mathscape_sandbox::{CheckpointRecord, ExportOutcome, SandboxHome, SessionView};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::DesktopState;

/// Override for the sandbox home (defaults to `<app_data>/sandbox`).
pub(crate) const SANDBOX_ROOT_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_SANDBOX_ROOT";

#[derive(Clone)]
pub(crate) struct SandboxState {
    home: Arc<Mutex<SandboxHome>>,
}

impl SandboxState {
    pub(crate) fn open(root: PathBuf) -> Result<Self, mathscape_sandbox::SandboxError> {
        Ok(Self {
            home: Arc::new(Mutex::new(SandboxHome::open(root)?)),
        })
    }

    async fn run<T, F>(&self, label: &str, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&SandboxHome) -> Result<T, mathscape_sandbox::SandboxError> + Send + 'static,
    {
        let home = Arc::clone(&self.home);
        let label = label.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let home = home
                .lock()
                .map_err(|_| "沙箱状态锁已损坏，请重启应用".to_owned())?;
            work(&home).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("{label} worker failed: {error}"))?
    }
}

pub(crate) fn sandbox_root(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match env::var_os(SANDBOX_ROOT_ENVIRONMENT_VARIABLE) {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => Ok(app.path().app_data_dir()?.join("sandbox")),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxOverview {
    pub sandbox_root: PathBuf,
    pub default_export_dir: PathBuf,
    pub current_workspace_root: PathBuf,
    pub checkpoints: Vec<CheckpointRecord>,
    pub sessions: Vec<SessionView>,
}

/// Where a new checkpoint comes from.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CheckpointSource {
    /// The workspace this Desktop instance currently has open.
    CurrentWorkspace,
    /// A folder chosen with the system dialog.
    PickFolder,
    /// An explicit local path.
    Path { path: PathBuf },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterCheckpointRequest {
    pub source: CheckpointSource,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionRequest {
    pub sandbox_session_id: String,
    /// Pick the parent folder with a dialog instead of the default export dir.
    #[serde(default)]
    pub choose_destination: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

#[tauri::command]
pub async fn sandbox_overview(
    sandbox: State<'_, SandboxState>,
    desktop: State<'_, DesktopState>,
) -> Result<SandboxOverview, String> {
    let current_workspace_root = desktop.service.workspace_root().to_path_buf();
    sandbox
        .run("sandbox overview", move |home| {
            Ok(SandboxOverview {
                sandbox_root: home.root().to_path_buf(),
                default_export_dir: home.default_export_dir(),
                current_workspace_root,
                checkpoints: home.list_checkpoints()?,
                sessions: home.list_sessions()?,
            })
        })
        .await
}

#[tauri::command]
pub async fn sandbox_list_checkpoints(
    sandbox: State<'_, SandboxState>,
) -> Result<Vec<CheckpointRecord>, String> {
    sandbox
        .run("checkpoint list", |home| home.list_checkpoints())
        .await
}

/// Returns `None` when the user cancels the folder dialog.
#[tauri::command]
pub async fn sandbox_register_checkpoint(
    app: AppHandle,
    sandbox: State<'_, SandboxState>,
    desktop: State<'_, DesktopState>,
    request: RegisterCheckpointRequest,
) -> Result<Option<CheckpointRecord>, String> {
    let root = match request.source {
        CheckpointSource::CurrentWorkspace => desktop.service.workspace_root().to_path_buf(),
        CheckpointSource::Path { path } => path,
        CheckpointSource::PickFolder => {
            let Some(selected) = app.dialog().file().blocking_pick_folder() else {
                return Ok(None);
            };
            selected
                .into_path()
                .map_err(|error| format!("所选文件夹不是本地路径：{error}"))?
        }
    };
    let label = request.label;
    sandbox
        .run("checkpoint register", move |home| {
            home.register_checkpoint(&root, label.as_deref())
        })
        .await
        .map(Some)
}

#[tauri::command]
pub async fn sandbox_unregister_checkpoint(
    sandbox: State<'_, SandboxState>,
    checkpoint_id: String,
) -> Result<(), String> {
    sandbox
        .run("checkpoint unregister", move |home| {
            home.unregister_checkpoint(&checkpoint_id)
        })
        .await
}

#[tauri::command]
pub async fn sandbox_start_session(
    sandbox: State<'_, SandboxState>,
    checkpoint_id: String,
) -> Result<SessionView, String> {
    sandbox
        .run("sandbox session start", move |home| {
            home.start_session(&checkpoint_id)
        })
        .await
}

#[tauri::command]
pub async fn sandbox_list_sessions(
    sandbox: State<'_, SandboxState>,
) -> Result<Vec<SessionView>, String> {
    sandbox
        .run("sandbox session list", |home| home.list_sessions())
        .await
}

#[tauri::command]
pub async fn sandbox_discard_session(
    sandbox: State<'_, SandboxState>,
    sandbox_session_id: String,
) -> Result<SessionView, String> {
    sandbox
        .run("sandbox session discard", move |home| {
            home.discard_session(&sandbox_session_id)
        })
        .await
}

/// Returns `None` when the user cancels the destination dialog.
#[tauri::command]
pub async fn sandbox_export_session(
    app: AppHandle,
    sandbox: State<'_, SandboxState>,
    request: ExportSessionRequest,
) -> Result<Option<ExportOutcome>, String> {
    let destination = if request.choose_destination {
        let Some(selected) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        Some(
            selected
                .into_path()
                .map_err(|error| format!("所选文件夹不是本地路径：{error}"))?,
        )
    } else {
        None
    };
    let ExportSessionRequest {
        sandbox_session_id,
        notes,
        ..
    } = request;
    sandbox
        .run("sandbox session export", move |home| {
            home.export_session(
                &sandbox_session_id,
                destination.as_deref(),
                notes.as_deref(),
            )
        })
        .await
        .map(Some)
}
