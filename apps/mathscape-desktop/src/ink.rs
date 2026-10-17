use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, ArtifactCheckResult, InkLabDocument, InkLabDocumentMetadata, InkLabOperation,
    InkSnapshotManifest, MediaType, RegisterInkEvidenceRequest, StableId, Timestamp,
};
use mathscape_store_sqlite::{
    InkCleanBlankReceipt, InkCleanBlankRequest, InkCreateReceipt, InkEvidenceReceipt,
    InkLifecycleRequest, InkNoteReceipt, InkNoteState, InkOperationReceipt, InkRenameRequest,
    InkTitleMode, InkTrashImpact, SqliteStoreError, StoredInkDocument, StoredInkDocumentSummary,
    StoredInkSession, StoredInkSnapshot, StoredInkSnapshotSummary,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::{DesktopService, DesktopState};

/// Error returned by Ink commands. The frontend branches on `code`; `message`
/// is user-facing Chinese text and `details` carries conflict data.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InkCommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl InkCommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Value::Null,
        }
    }
    /// Matches either the code or the message, like `str::contains` on the old
    /// string errors.
    pub fn contains(&self, needle: &str) -> bool {
        self.code.contains(needle) || self.message.contains(needle)
    }
}

impl std::fmt::Display for InkCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for InkCommandError {}

impl From<String> for InkCommandError {
    fn from(message: String) -> Self {
        Self::new("request_failed", message)
    }
}

impl From<&str> for InkCommandError {
    fn from(message: &str) -> Self {
        Self::new("request_failed", message)
    }
}

impl From<SqliteStoreError> for InkCommandError {
    fn from(error: SqliteStoreError) -> Self {
        let code = error.ink_code();
        let mut result = Self::new(code, error.to_string());
        match &error {
            SqliteStoreError::InkDocumentTrashed(document_id) => {
                result.message = "手稿已在回收站中，恢复后才能继续编辑。".into();
                result.details = serde_json::json!({ "document_id": document_id });
            }
            SqliteStoreError::InkTitleConflict {
                document_id,
                current_title,
                current_title_mode,
                current_revision,
            } => {
                result.message = "标题已在别处修改，请选择保留哪一个。".into();
                result.details = serde_json::json!({
                    "document_id": document_id,
                    "current_title": current_title,
                    "current_title_mode": current_title_mode,
                    "current_title_revision": current_revision,
                });
            }
            SqliteStoreError::InkLifecycleConflict {
                document_id,
                lifecycle,
                current_revision,
            } => {
                result.message = "手稿状态已在别处变化，请刷新后重试。".into();
                result.details = serde_json::json!({
                    "document_id": document_id,
                    "lifecycle": lifecycle,
                    "current_lifecycle_revision": current_revision,
                });
            }
            SqliteStoreError::InkTitleInvalid(message) => result.message = message.clone(),
            SqliteStoreError::InkWriterEpochConflict {
                document_id,
                expected,
                actual,
            } => {
                result.details = serde_json::json!({
                    "document_id": document_id,
                    "expected_epoch": expected,
                    "actual_epoch": actual,
                });
            }
            _ => {}
        }
        result
    }
}

/// Maps any error into an [`InkCommandError`], keeping store codes.
pub(crate) fn fail<E: std::fmt::Display + 'static>(error: E) -> InkCommandError {
    let any: Box<dyn std::any::Any> = Box::new(error);
    match any.downcast::<SqliteStoreError>() {
        Ok(store) => InkCommandError::from(*store),
        Err(any) => match any.downcast::<InkCommandError>() {
            Ok(command) => *command,
            Err(any) => match any.downcast::<String>() {
                Ok(message) => InkCommandError::from(*message),
                Err(_) => InkCommandError::new("storage_failed", "电脑存储暂时不可用。"),
            },
        },
    }
}

fn snapshot_error(message: String) -> InkCommandError {
    InkCommandError::new("snapshot_invalid", message)
}

fn worker(message: String) -> InkCommandError {
    InkCommandError::new("worker_failed", message)
}

#[derive(Deserialize)]
struct SnapshotTitle {
    document: SnapshotDocumentTitle,
}
#[derive(Deserialize)]
struct SnapshotDocumentTitle {
    title: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateInkDocumentRequest {
    pub document: InkLabDocument,
    pub session_id: String,
    pub writer_id: String,
    #[serde(default)]
    pub device: Value,
    /// Retries reuse the same id, so a lost reply never creates a second note (D12).
    #[serde(default)]
    pub request_id: Option<String>,
    /// `auto` materializes the creation-time title (D3); omitted means manual.
    #[serde(default)]
    pub title_mode: Option<InkTitleMode>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StartInkSessionRequest {
    pub document_id: String,
    pub session_id: String,
    pub writer_id: String,
    #[serde(default)]
    pub device: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppendInkOperationRequest {
    pub request_id: String,
    pub document_id: String,
    pub session_id: String,
    pub writer_epoch: u64,
    pub expected_head: u64,
    pub metadata: InkLabDocumentMetadata,
    pub operation: InkLabOperation,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateInkSnapshotRequest {
    pub request_id: String,
    pub document_id: String,
    pub expected_head: u64,
}

/// Trash confirmation data plus whether a tablet is connected right now.
#[derive(Clone, Debug, Serialize)]
pub struct InkTrashImpactView {
    #[serde(flatten)]
    pub impact: InkTrashImpact,
    pub connected: bool,
}

impl DesktopService {
    fn ink_store(&self) -> Result<mathscape_store_sqlite::SqliteArtifactStore, InkCommandError> {
        mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)
    }

    pub fn ink_note(&self, document_id: String) -> Result<Option<InkNoteState>, InkCommandError> {
        self.ink_store()?.ink_note(&document_id).map_err(fail)
    }

    pub fn ink_trash_impact(
        &self,
        document_id: String,
    ) -> Result<InkTrashImpactView, InkCommandError> {
        let impact = self
            .ink_store()?
            .ink_trash_impact(&document_id)
            .map_err(fail)?
            .ok_or_else(|| InkCommandError::new("document_not_found", "手稿不存在。"))?;
        Ok(InkTrashImpactView {
            impact,
            connected: self.ink_note_connected(&document_id),
        })
    }

    pub fn list_trashed_ink_documents(
        &self,
    ) -> Result<Vec<StoredInkDocumentSummary>, InkCommandError> {
        self.ink_store()?.list_trashed_ink_documents().map_err(fail)
    }

    /// Desktop rename (§5.3). Tablets and the setup page follow the new title.
    pub fn rename_ink_note(
        &self,
        request: InkRenameRequest,
    ) -> Result<InkNoteReceipt, InkCommandError> {
        let receipt = self.ink_store()?.rename_ink_note(&request).map_err(fail)?;
        if !receipt.replayed {
            self.ink_note_renamed(&receipt.note);
        }
        Ok(receipt)
    }

    /// Moves a document to the trash. The store fences writers in one
    /// transaction; afterwards live tablet connections close with
    /// `writer_revoked` + `reason: document_trashed` (D6/D7).
    pub async fn trash_ink_note(
        &self,
        request: InkLifecycleRequest,
    ) -> Result<InkNoteReceipt, InkCommandError> {
        let service = self.clone();
        let receipt = tauri::async_runtime::spawn_blocking(move || {
            service.ink_store()?.trash_ink_note(&request).map_err(fail)
        })
        .await
        .map_err(|error| worker(format!("Ink trash worker failed: {error}")))??;
        self.ink_notes_trashed(std::slice::from_ref(&receipt.note.document_id))
            .await;
        Ok(receipt)
    }

    /// Restores a trashed document. Old sessions and credentials stay dead;
    /// `requires_repairing` tells the user to pair the tablet again.
    pub fn restore_ink_note(
        &self,
        request: InkLifecycleRequest,
    ) -> Result<InkNoteReceipt, InkCommandError> {
        self.ink_store()?.restore_ink_note(&request).map_err(fail)
    }

    pub async fn clean_blank_ink_notes(
        &self,
        request: InkCleanBlankRequest,
    ) -> Result<InkCleanBlankReceipt, InkCommandError> {
        let service = self.clone();
        let receipt = tauri::async_runtime::spawn_blocking(move || {
            service
                .ink_store()?
                .clean_blank_ink_notes(&request)
                .map_err(fail)
        })
        .await
        .map_err(|error| worker(format!("Ink clean worker failed: {error}")))??;
        let trashed: Vec<String> = receipt
            .trashed
            .iter()
            .map(|note| note.document_id.clone())
            .collect();
        self.ink_notes_trashed(&trashed).await;
        Ok(receipt)
    }

    pub fn create_ink_snapshot(
        &self,
        request: CreateInkSnapshotRequest,
    ) -> Result<StoredInkSnapshot, InkCommandError> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        let created_at = format!(
            "unix:{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(fail)?
                .as_secs()
        );
        let reservation = store
            .reserve_ink_snapshot(
                &request.request_id,
                &request.document_id,
                request.expected_head,
                &created_at,
            )
            .map_err(fail)?;
        if let Some(existing) = store
            .ink_snapshot_for_request(&request.request_id)
            .map_err(fail)?
        {
            return Ok(existing);
        }
        let manifest_json = reservation.manifest_json;
        let title: SnapshotTitle = serde_json::from_str(&manifest_json).map_err(fail)?;
        let actor = ActorRef::parse("mathscape-desktop").map_err(fail)?;
        let metadata = ArtifactMetadata::derived(
            MediaType::parse("application/vnd.mathscape.ink-snapshot+json").map_err(fail)?,
            Some(format!(
                "{} · 手稿快照 head {}.json",
                title.document.title, reservation.through_seq
            )),
        )
        .map_err(fail)?;
        let mut pool = ArtifactPool::open(&self.workspace_root()).map_err(fail)?;
        let imported = pool
            .import_bytes(manifest_json.as_bytes(), metadata, actor)
            .map_err(fail)?;
        let digest = imported
            .content_hash()
            .as_str()
            .strip_prefix("sha256:")
            .unwrap_or(imported.content_hash().as_str());
        let snapshot_id =
            StableId::parse(format!("mathscape:ink-snapshot/{digest}")).map_err(fail)?;
        store
            .record_ink_snapshot(
                &request.request_id,
                &request.document_id,
                request.expected_head,
                snapshot_id.as_str(),
                imported.stable_id().as_str(),
                imported.content_hash().as_str(),
                imported.byte_size(),
                &manifest_json,
                &reservation.created_at,
            )
            .map_err(fail)
    }

    pub fn list_ink_snapshots(
        &self,
        document_id: String,
    ) -> Result<Vec<StoredInkSnapshotSummary>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store.list_ink_snapshots(&document_id).map_err(fail)
    }

    pub fn open_ink_snapshot(
        &self,
        snapshot_id: String,
    ) -> Result<Option<StoredInkSnapshot>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        let Some(snapshot) = store.open_ink_snapshot(&snapshot_id).map_err(fail)? else {
            return Ok(None);
        };
        let snapshot_artifact = StableId::parse(&snapshot.artifact_id)
            .map_err(|error| snapshot_error(format!("快照 Artifact 标识无效：{error}")))?;
        let catalog = store.read_catalog_snapshot().map_err(fail)?;
        let artifact = catalog
            .artifacts()
            .iter()
            .find(|record| record.entity().header().stable_id() == &snapshot_artifact)
            .ok_or("快照 Artifact 未登记")?
            .entity()
            .payload();
        if !matches!(
            artifact.media_type().as_str(),
            "application/json" | "application/vnd.mathscape.ink-snapshot+json"
        ) {
            return Err("快照 Artifact 媒体类型无效".into());
        }
        let mut pool = ArtifactPool::open(&self.workspace_root()).map_err(fail)?;
        let verification = pool
            .verify(&snapshot_artifact)
            .map_err(|error| snapshot_error(format!("快照 Artifact 校验失败：{error}")))?;
        if verification.result() != ArtifactCheckResult::Valid
            || verification.observed_hash().map(|hash| hash.as_str())
                != Some(snapshot.content_hash.as_str())
            || verification.observed_size() != Some(snapshot.byte_size)
        {
            return Err(snapshot_error(format!(
                "快照 Artifact 完整性校验未通过：{:?}",
                verification.result()
            )));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(snapshot.byte_size).map_err(|_| "快照 Artifact 大小无效".to_owned())?,
        );
        let mut offset = 0_u64;
        while offset < snapshot.byte_size {
            let length = (snapshot.byte_size - offset).min(4 * 1024 * 1024);
            let chunk = pool
                .read_bytes(&snapshot_artifact, offset, u32::try_from(length).unwrap())
                .map_err(|error| snapshot_error(format!("读取快照 Artifact 失败：{error}")))?;
            if chunk.len() != usize::try_from(length).unwrap() {
                return Err(snapshot_error(
                    "快照 Artifact 长度在读取期间发生变化".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
            offset += length;
        }
        if bytes != snapshot.manifest_json.as_bytes() {
            return Err(snapshot_error("快照索引与 Artifact 内容不一致".to_owned()));
        }
        let manifest: InkSnapshotManifest = serde_json::from_slice(&bytes)
            .map_err(|error| snapshot_error(format!("快照 manifest 无效：{error}")))?;
        manifest.validate().map_err(fail)?;
        if manifest.document_id != snapshot.document_id
            || u64::from(manifest.through_document_seq) != snapshot.through_seq
        {
            return Err(snapshot_error("快照 manifest 与索引绑定不一致".to_owned()));
        }
        Ok(Some(snapshot))
    }

    pub fn open_ink_snapshot_for_artifact(
        &self,
        artifact_id: String,
    ) -> Result<Option<StoredInkSnapshot>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        let Some(snapshot) = store
            .ink_snapshot_for_artifact(&artifact_id)
            .map_err(fail)?
        else {
            return Ok(None);
        };
        self.open_ink_snapshot(snapshot.snapshot_id)
    }

    pub fn register_ink_evidence(
        &self,
        request: RegisterInkEvidenceRequest,
    ) -> Result<InkEvidenceReceipt, InkCommandError> {
        let request = request.canonicalized().map_err(fail)?;
        let snapshot = self
            .open_ink_snapshot(request.snapshot_id.clone())?
            .ok_or("快照不存在")?;
        if snapshot.artifact_id != request.expected_artifact_id.as_str()
            || snapshot.content_hash != request.expected_content_hash.as_str()
        {
            return Err("快照身份已变化，请重新打开".into());
        }
        let actor = ActorRef::parse("mathscape-desktop-user").map_err(fail)?;
        let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(fail)?;
        let now = Timestamp::parse(format!(
            "unix:{}.{:09}",
            duration.as_secs(),
            duration.subsec_nanos()
        ))
        .map_err(fail)?;
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store
            .register_ink_evidence(&request, &actor, now.clone(), |stored| {
                let manifest: InkSnapshotManifest = serde_json::from_str(&stored.manifest_json)
                    .map_err(|error| error.to_string())?;
                mathscape_orchestrator::plan_ink_evidence(&manifest, &request, &actor, now)
            })
            .map_err(fail)
    }

    pub fn list_ink_evidence(
        &self,
        snapshot_id: String,
    ) -> Result<Vec<InkEvidenceReceipt>, InkCommandError> {
        mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?
        .list_ink_evidence(&snapshot_id)
        .map_err(fail)
    }

    pub fn create_ink_document(
        &self,
        request: CreateInkDocumentRequest,
    ) -> Result<InkCreateReceipt, InkCommandError> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store
            .create_ink_note(
                &request.document,
                &request.session_id,
                &request.writer_id,
                &request.device,
                request.title_mode.unwrap_or(InkTitleMode::Manual),
                request.request_id.as_deref(),
            )
            .map_err(fail)
    }

    pub fn start_ink_session(
        &self,
        request: StartInkSessionRequest,
    ) -> Result<StoredInkSession, InkCommandError> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store
            .start_ink_session(
                &request.document_id,
                &request.session_id,
                &request.writer_id,
                &request.device,
            )
            .map_err(fail)
    }

    pub fn append_ink_operation(
        &self,
        request: AppendInkOperationRequest,
    ) -> Result<InkOperationReceipt, InkCommandError> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store
            .append_ink_operation(
                &request.request_id,
                &request.document_id,
                &request.session_id,
                request.writer_epoch,
                request.expected_head,
                &request.metadata,
                &request.operation,
            )
            .map_err(fail)
    }

    pub fn open_ink_document(
        &self,
        document_id: String,
    ) -> Result<Option<StoredInkDocument>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store.open_ink_document(&document_id).map_err(fail)
    }

    pub fn list_ink_documents(&self) -> Result<Vec<StoredInkDocumentSummary>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store.list_ink_documents().map_err(fail)
    }

    pub fn list_ink_sessions(
        &self,
        document_id: String,
    ) -> Result<Vec<StoredInkSession>, InkCommandError> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store.list_ink_sessions(&document_id).map_err(fail)
    }

    pub fn close_ink_session(&self, session_id: String) -> Result<(), InkCommandError> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)?;
        store.close_ink_session(&session_id).map_err(fail)
    }
}

#[tauri::command]
pub async fn create_ink_document(
    state: State<'_, DesktopState>,
    request: CreateInkDocumentRequest,
) -> Result<InkCreateReceipt, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.create_ink_document(request))
        .await
        .map_err(|error| worker(format!("Ink document creation worker failed: {error}")))?
}

#[tauri::command]
pub async fn start_ink_session(
    state: State<'_, DesktopState>,
    request: StartInkSessionRequest,
) -> Result<StoredInkSession, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.start_ink_session(request))
        .await
        .map_err(|error| worker(format!("Ink session worker failed: {error}")))?
}

#[tauri::command]
pub async fn append_ink_operation(
    state: State<'_, DesktopState>,
    request: AppendInkOperationRequest,
) -> Result<InkOperationReceipt, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.append_ink_operation(request))
        .await
        .map_err(|error| worker(format!("Ink operation worker failed: {error}")))?
}

#[tauri::command]
pub async fn create_ink_snapshot(
    state: State<'_, DesktopState>,
    request: CreateInkSnapshotRequest,
) -> Result<StoredInkSnapshot, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.create_ink_snapshot(request))
        .await
        .map_err(|error| worker(format!("Ink snapshot worker failed: {error}")))?
}

#[tauri::command]
pub async fn list_ink_snapshots(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Vec<StoredInkSnapshotSummary>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_snapshots(document_id))
        .await
        .map_err(|error| worker(format!("Ink snapshot list worker failed: {error}")))?
}

#[tauri::command]
pub async fn get_ink_snapshot(
    state: State<'_, DesktopState>,
    snapshot_id: String,
) -> Result<Option<StoredInkSnapshot>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.open_ink_snapshot(snapshot_id))
        .await
        .map_err(|error| worker(format!("Ink snapshot read worker failed: {error}")))?
}

#[tauri::command]
pub(crate) async fn get_ink_snapshot_for_artifact(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<Option<StoredInkSnapshot>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.open_ink_snapshot_for_artifact(artifact_id)
    })
    .await
    .map_err(fail)?
}

#[tauri::command]
pub(crate) async fn register_ink_evidence(
    state: State<'_, DesktopState>,
    request: RegisterInkEvidenceRequest,
) -> Result<InkEvidenceReceipt, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.register_ink_evidence(request))
        .await
        .map_err(fail)?
}

#[tauri::command]
pub(crate) async fn list_ink_evidence(
    state: State<'_, DesktopState>,
    snapshot_id: String,
) -> Result<Vec<InkEvidenceReceipt>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_evidence(snapshot_id))
        .await
        .map_err(fail)?
}

#[tauri::command]
pub async fn get_ink_document(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Option<StoredInkDocument>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.open_ink_document(document_id))
        .await
        .map_err(|error| worker(format!("Ink document read worker failed: {error}")))?
}

#[tauri::command]
pub async fn list_ink_documents(
    state: State<'_, DesktopState>,
) -> Result<Vec<StoredInkDocumentSummary>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_documents())
        .await
        .map_err(|error| worker(format!("Ink document list worker failed: {error}")))?
}

#[tauri::command]
pub async fn list_ink_sessions(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Vec<StoredInkSession>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_sessions(document_id))
        .await
        .map_err(|error| worker(format!("Ink session list worker failed: {error}")))?
}

#[tauri::command]
pub async fn close_ink_session(
    state: State<'_, DesktopState>,
    session_id: String,
) -> Result<(), InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.close_ink_session(session_id))
        .await
        .map_err(|error| worker(format!("Ink session close worker failed: {error}")))?
}

#[tauri::command]
pub async fn get_ink_note(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Option<InkNoteState>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.ink_note(document_id))
        .await
        .map_err(|error| worker(format!("Ink note worker failed: {error}")))?
}

#[tauri::command]
pub async fn get_ink_trash_impact(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<InkTrashImpactView, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.ink_trash_impact(document_id))
        .await
        .map_err(|error| worker(format!("Ink trash impact worker failed: {error}")))?
}

#[tauri::command]
pub async fn list_trashed_ink_documents(
    state: State<'_, DesktopState>,
) -> Result<Vec<StoredInkDocumentSummary>, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_trashed_ink_documents())
        .await
        .map_err(|error| worker(format!("Ink trash list worker failed: {error}")))?
}

#[tauri::command]
pub async fn rename_ink_note(
    state: State<'_, DesktopState>,
    request: InkRenameRequest,
) -> Result<InkNoteReceipt, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.rename_ink_note(request))
        .await
        .map_err(|error| worker(format!("Ink rename worker failed: {error}")))?
}

#[tauri::command]
pub async fn trash_ink_note(
    state: State<'_, DesktopState>,
    request: InkLifecycleRequest,
) -> Result<InkNoteReceipt, InkCommandError> {
    state.service.clone().trash_ink_note(request).await
}

#[tauri::command]
pub async fn restore_ink_note(
    state: State<'_, DesktopState>,
    request: InkLifecycleRequest,
) -> Result<InkNoteReceipt, InkCommandError> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.restore_ink_note(request))
        .await
        .map_err(|error| worker(format!("Ink restore worker failed: {error}")))?
}

#[tauri::command]
pub async fn clean_blank_ink_notes(
    state: State<'_, DesktopState>,
    request: InkCleanBlankRequest,
) -> Result<InkCleanBlankReceipt, InkCommandError> {
    state.service.clone().clean_blank_ink_notes(request).await
}
