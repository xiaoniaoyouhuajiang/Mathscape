//! Sandbox session commands (docs/sandbox-flow.md §3, §5 S0/S1).
//!
//! "Sandbox session" here is the *product* concept: an isolated full copy of
//! a checkpoint workspace that can be discarded or exported. It is unrelated
//! to Codex `SandboxMode` (process permission policy), which S1 will configure
//! separately when a dialogue backend is mounted.
//!
//! Read-only towards the checkpoint (decision T2): no command here writes
//! into a checkpoint root. S1 adds the in-window switch: the window can bind
//! to a live session's isolated copy (after a confirmation in the UI) and
//! back to the workspace it was launched with ("mother"). The switch is not
//! persisted; a restart always opens the mother workspace. Dialogue commands
//! live in `sandbox_dialogue.rs`.

use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mathscape_sandbox::{CheckpointRecord, ExportOutcome, SandboxHome, SessionView};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::sandbox_dialogue::SandboxDialogueState;
use tauri_plugin_dialog::DialogExt;

use crate::DesktopState;

/// Override for the sandbox home (defaults to `<app_data>/sandbox`).
pub(crate) const SANDBOX_ROOT_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_SANDBOX_ROOT";

#[derive(Clone)]
pub(crate) struct SandboxState {
    home: Arc<Mutex<SandboxHome>>,
}

impl SandboxState {
    pub(crate) fn home(&self) -> Arc<Mutex<SandboxHome>> {
        Arc::clone(&self.home)
    }

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
    pub workspace: WorkspaceStatus,
}

/// The sandbox session the window is currently bound to (S1).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InWindowSandbox {
    pub sandbox_session_id: String,
    pub checkpoint_id: String,
    pub workspace_root: PathBuf,
}

/// What the top bar needs: are we in a sandbox, and where is home.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    pub mother_workspace_root: PathBuf,
    pub current_workspace_root: PathBuf,
    pub sandbox: Option<InWindowSandbox>,
}

fn workspace_status(desktop: &DesktopState) -> WorkspaceStatus {
    WorkspaceStatus {
        mother_workspace_root: desktop.mother_root().to_path_buf(),
        current_workspace_root: desktop.service().workspace_root().to_path_buf(),
        sandbox: desktop.in_window_sandbox(),
    }
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
    let workspace = workspace_status(&desktop);
    let current_workspace_root = workspace.current_workspace_root.clone();
    sandbox
        .run("sandbox overview", move |home| {
            Ok(SandboxOverview {
                sandbox_root: home.root().to_path_buf(),
                default_export_dir: home.default_export_dir(),
                current_workspace_root,
                checkpoints: home.list_checkpoints()?,
                sessions: home.list_sessions()?,
                workspace,
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
        CheckpointSource::CurrentWorkspace => desktop.service().workspace_root().to_path_buf(),
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
    desktop: State<'_, DesktopState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
) -> Result<SessionView, String> {
    if desktop
        .in_window_sandbox()
        .is_some_and(|current| current.sandbox_session_id == sandbox_session_id)
    {
        return Err("本窗口正在使用这个沙箱，请先「返回母工作区」再丢弃".into());
    }
    // A still-running round is cancelled first; its late writes are refused
    // by SandboxHome once the directory is gone, so nothing is revived.
    dialogue.cancel_session_round(&sandbox_session_id).await;
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

#[tauri::command]
pub fn sandbox_workspace_status(desktop: State<'_, DesktopState>) -> WorkspaceStatus {
    workspace_status(&desktop)
}

/// Binds this window to a live session's isolated copy. The UI asks for
/// confirmation first and reloads after success; the checkpoint and the
/// mother workspace are not written.
#[tauri::command]
pub async fn sandbox_enter_session(
    sandbox: State<'_, SandboxState>,
    desktop: State<'_, DesktopState>,
    sandbox_session_id: String,
) -> Result<WorkspaceStatus, String> {
    let session = sandbox
        .run("sandbox session enter", move |home| {
            home.live_session(&sandbox_session_id)
        })
        .await?;
    let Some(workspace_root) = session.workspace_root.clone() else {
        return Err("该沙箱没有隔离工作区".into());
    };
    if desktop
        .in_window_sandbox()
        .is_some_and(|current| current.sandbox_session_id == session.sandbox_session_id)
    {
        return Ok(workspace_status(&desktop));
    }
    desktop
        .switch_to(
            workspace_root.clone(),
            Some(InWindowSandbox {
                sandbox_session_id: session.sandbox_session_id,
                checkpoint_id: session.checkpoint_id,
                workspace_root,
            }),
        )
        .await?;
    Ok(workspace_status(&desktop))
}

/// Rebinds this window to the workspace it was launched with.
#[tauri::command]
pub async fn sandbox_return_to_mother(
    desktop: State<'_, DesktopState>,
) -> Result<WorkspaceStatus, String> {
    if desktop.in_window_sandbox().is_some() {
        desktop
            .switch_to(desktop.mother_root().to_path_buf(), None)
            .await?;
    }
    Ok(workspace_status(&desktop))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use mathscape_sandbox::fingerprint_workspace;

    use super::*;
    use crate::{DesktopService, MarkerProfileChoice, ParsePdfRequest};

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn in_window_switch_binds_session_copy_and_returns_to_mother() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = Dir(env::temp_dir().join(format!(
            "mathscape-s1-switch-{}-{nanos}",
            std::process::id()
        )));
        let mother = DesktopService::open_with_credential_store(
            dir.0.join("mother"),
            Arc::new(mathscape_llm::MemoryCredentialResolver::default()),
        )
        .unwrap();
        let mother_root = mother.workspace_root().to_path_buf();
        let before = fingerprint_workspace(&mother_root).unwrap();
        let home = SandboxHome::open(dir.0.join("sandbox")).unwrap();
        let checkpoint = home.register_checkpoint(&mother_root, None).unwrap();
        let session = home.start_session(&checkpoint.checkpoint_id).unwrap();
        let copy = session.workspace_root.clone().unwrap();

        let state = DesktopState::new(mother.clone());
        assert!(state.in_window_sandbox().is_none());
        tauri::async_runtime::block_on(state.switch_to(
            copy.clone(),
            Some(InWindowSandbox {
                sandbox_session_id: session.sandbox_session_id.clone(),
                checkpoint_id: session.checkpoint_id.clone(),
                workspace_root: copy.clone(),
            }),
        ))
        .unwrap();
        let status = workspace_status(&state);
        assert_eq!(status.mother_workspace_root, mother_root);
        assert_eq!(
            std::fs::canonicalize(&status.current_workspace_root).unwrap(),
            std::fs::canonicalize(&copy).unwrap()
        );
        assert_eq!(
            status
                .sandbox
                .as_ref()
                .map(|s| s.sandbox_session_id.as_str()),
            Some(session.sandbox_session_id.as_str())
        );
        // Commands now read the copy; the mother/checkpoint bytes are untouched
        // while the window works in the sandbox.
        state.service().snapshot().unwrap();
        assert!(
            fingerprint_workspace(&mother_root)
                .unwrap()
                .same_content(&before)
        );

        // A busy workspace refuses to switch away.
        *state.service().active_parse.lock().unwrap() = Some(ParsePdfRequest::new(
            "a",
            1,
            1,
            MarkerProfileChoice::FastNativeV1,
        ));
        let refused = tauri::async_runtime::block_on(state.switch_to(mother_root.clone(), None));
        assert!(refused.unwrap_err().contains("PDF 解析"));
        *state.service().active_parse.lock().unwrap() = None;

        tauri::async_runtime::block_on(state.switch_to(mother_root.clone(), None)).unwrap();
        assert!(state.in_window_sandbox().is_none());
        assert_eq!(state.service().workspace_root(), mother_root);
        state.service().snapshot().unwrap();
    }
}
