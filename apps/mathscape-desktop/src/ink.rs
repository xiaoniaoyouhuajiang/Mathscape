use mathscape_domain::{InkLabDocument, InkLabDocumentMetadata, InkLabOperation};
use mathscape_store_sqlite::{
    InkOperationReceipt, StoredInkDocument, StoredInkDocumentSummary, StoredInkSession,
};
use serde::Deserialize;
use serde_json::Value;
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

impl DesktopService {
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
