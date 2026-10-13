use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, ArtifactCheckResult, InkLabDocument, InkLabDocumentMetadata, InkLabOperation,
    InkSnapshotManifest, MediaType, StableId,
};
use mathscape_store_sqlite::{
    InkOperationReceipt, StoredInkDocument, StoredInkDocumentSummary, StoredInkSession,
    StoredInkSnapshot,
};
use serde::Deserialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::{DesktopService, DesktopState};

#[derive(Clone, Debug, Deserialize)]
pub struct CreateInkDocumentRequest {
    pub document: InkLabDocument,
    pub session_id: String,
    pub writer_id: String,
    #[serde(default)]
    pub device: Value,
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

impl DesktopService {
    pub fn create_ink_snapshot(
        &self,
        request: CreateInkSnapshotRequest,
    ) -> Result<StoredInkSnapshot, String> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        if let Some(existing) = store
            .ink_snapshot_for_request(&request.request_id)
            .map_err(|error| error.to_string())?
        {
            return Ok(existing);
        }
        let stored = store
            .open_ink_document(&request.document_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "手稿不存在或已被移除".to_owned())?;
        if stored.head_seq != request.expected_head {
            return Err(format!(
                "手稿 head 已变化：预期 {}，当前 {}",
                request.expected_head, stored.head_seq
            ));
        }
        let manifest = InkSnapshotManifest::from_document(&stored.document, request.expected_head)
            .map_err(|error| error.to_string())?;
        let manifest_json = serde_json::to_string(&manifest).map_err(|error| error.to_string())?;
        let actor = ActorRef::parse("mathscape-desktop").map_err(|error| error.to_string())?;
        let metadata = ArtifactMetadata::derived(
            MediaType::parse("application/json").map_err(|error| error.to_string())?,
            Some(format!(
                "mathscape-ink-snapshot-{}.json",
                request.document_id
            )),
        )
        .map_err(|error| error.to_string())?;
        let mut pool =
            ArtifactPool::open(&self.workspace_root()).map_err(|error| error.to_string())?;
        let imported = pool
            .import_bytes(manifest_json.as_bytes(), metadata, actor)
            .map_err(|error| error.to_string())?;
        let digest = imported
            .content_hash()
            .as_str()
            .strip_prefix("sha256:")
            .unwrap_or(imported.content_hash().as_str());
        let snapshot_id = StableId::parse(format!("mathscape:ink-snapshot/{digest}"))
            .map_err(|error| error.to_string())?;
        let created_at = format!(
            "unix:{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs()
        );
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
                &created_at,
            )
            .map_err(|error| error.to_string())
    }

    pub fn list_ink_snapshots(
        &self,
        document_id: String,
    ) -> Result<Vec<StoredInkSnapshot>, String> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .list_ink_snapshots(&document_id)
            .map_err(|error| error.to_string())
    }

    pub fn open_ink_snapshot(
        &self,
        snapshot_id: String,
    ) -> Result<Option<StoredInkSnapshot>, String> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        let Some(snapshot) = store
            .open_ink_snapshot(&snapshot_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let snapshot_artifact = StableId::parse(&snapshot.artifact_id)
            .map_err(|error| format!("快照 Artifact 标识无效：{error}"))?;
        let mut pool =
            ArtifactPool::open(&self.workspace_root()).map_err(|error| error.to_string())?;
        let verification = pool
            .verify(&snapshot_artifact)
            .map_err(|error| format!("快照 Artifact 校验失败：{error}"))?;
        if verification.result() != ArtifactCheckResult::Valid
            || verification.observed_hash().map(|hash| hash.as_str())
                != Some(snapshot.content_hash.as_str())
            || verification.observed_size() != Some(snapshot.byte_size)
        {
            return Err(format!(
                "快照 Artifact 完整性校验未通过：{}",
                format!("{:?}", verification.result())
            ));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(snapshot.byte_size).map_err(|_| "快照 Artifact 大小无效".to_owned())?,
        );
        let mut offset = 0_u64;
        while offset < snapshot.byte_size {
            let length = (snapshot.byte_size - offset).min(4 * 1024 * 1024);
            let chunk = pool
                .read_bytes(&snapshot_artifact, offset, u32::try_from(length).unwrap())
                .map_err(|error| format!("读取快照 Artifact 失败：{error}"))?;
            if chunk.len() != usize::try_from(length).unwrap() {
                return Err("快照 Artifact 长度在读取期间发生变化".to_owned());
            }
            bytes.extend_from_slice(&chunk);
            offset += length;
        }
        if bytes != snapshot.manifest_json.as_bytes() {
            return Err("快照索引与 Artifact 内容不一致".to_owned());
        }
        let manifest: InkSnapshotManifest = serde_json::from_slice(&bytes)
            .map_err(|error| format!("快照 manifest 无效：{error}"))?;
        manifest.validate().map_err(|error| error.to_string())?;
        if manifest.document_id != snapshot.document_id
            || u64::from(manifest.through_document_seq) != snapshot.through_seq
        {
            return Err("快照 manifest 与索引绑定不一致".to_owned());
        }
        Ok(Some(snapshot))
    }

    pub fn create_ink_document(
        &self,
        request: CreateInkDocumentRequest,
    ) -> Result<StoredInkSession, String> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .create_ink_document(
                &request.document,
                &request.session_id,
                &request.writer_id,
                &request.device,
            )
            .map_err(|error| error.to_string())
    }

    pub fn start_ink_session(
        &self,
        request: StartInkSessionRequest,
    ) -> Result<StoredInkSession, String> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .start_ink_session(
                &request.document_id,
                &request.session_id,
                &request.writer_id,
                &request.device,
            )
            .map_err(|error| error.to_string())
    }

    pub fn append_ink_operation(
        &self,
        request: AppendInkOperationRequest,
    ) -> Result<InkOperationReceipt, String> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())
    }

    pub fn open_ink_document(
        &self,
        document_id: String,
    ) -> Result<Option<StoredInkDocument>, String> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .open_ink_document(&document_id)
            .map_err(|error| error.to_string())
    }

    pub fn list_ink_documents(&self) -> Result<Vec<StoredInkDocumentSummary>, String> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .list_ink_documents()
            .map_err(|error| error.to_string())
    }

    pub fn list_ink_sessions(&self, document_id: String) -> Result<Vec<StoredInkSession>, String> {
        let store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .list_ink_sessions(&document_id)
            .map_err(|error| error.to_string())
    }

    pub fn close_ink_session(&self, session_id: String) -> Result<(), String> {
        let mut store = mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(|error| error.to_string())?;
        store
            .close_ink_session(&session_id)
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub async fn create_ink_document(
    state: State<'_, DesktopState>,
    request: CreateInkDocumentRequest,
) -> Result<StoredInkSession, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.create_ink_document(request))
        .await
        .map_err(|error| format!("Ink document creation worker failed: {error}"))?
}

#[tauri::command]
pub async fn start_ink_session(
    state: State<'_, DesktopState>,
    request: StartInkSessionRequest,
) -> Result<StoredInkSession, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.start_ink_session(request))
        .await
        .map_err(|error| format!("Ink session worker failed: {error}"))?
}

#[tauri::command]
pub async fn append_ink_operation(
    state: State<'_, DesktopState>,
    request: AppendInkOperationRequest,
) -> Result<InkOperationReceipt, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.append_ink_operation(request))
        .await
        .map_err(|error| format!("Ink operation worker failed: {error}"))?
}

#[tauri::command]
pub async fn create_ink_snapshot(
    state: State<'_, DesktopState>,
    request: CreateInkSnapshotRequest,
) -> Result<StoredInkSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.create_ink_snapshot(request))
        .await
        .map_err(|error| format!("Ink snapshot worker failed: {error}"))?
}

#[tauri::command]
pub async fn list_ink_snapshots(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Vec<StoredInkSnapshot>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_snapshots(document_id))
        .await
        .map_err(|error| format!("Ink snapshot list worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_ink_snapshot(
    state: State<'_, DesktopState>,
    snapshot_id: String,
) -> Result<Option<StoredInkSnapshot>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.open_ink_snapshot(snapshot_id))
        .await
        .map_err(|error| format!("Ink snapshot read worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_ink_document(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Option<StoredInkDocument>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.open_ink_document(document_id))
        .await
        .map_err(|error| format!("Ink document read worker failed: {error}"))?
}

#[tauri::command]
pub async fn list_ink_documents(
    state: State<'_, DesktopState>,
) -> Result<Vec<StoredInkDocumentSummary>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_documents())
        .await
        .map_err(|error| format!("Ink document list worker failed: {error}"))?
}

#[tauri::command]
pub async fn list_ink_sessions(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Vec<StoredInkSession>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.list_ink_sessions(document_id))
        .await
        .map_err(|error| format!("Ink session list worker failed: {error}"))?
}

#[tauri::command]
pub async fn close_ink_session(
    state: State<'_, DesktopState>,
    session_id: String,
) -> Result<(), String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.close_ink_session(session_id))
        .await
        .map_err(|error| format!("Ink session close worker failed: {error}"))?
}
