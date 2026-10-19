//! Recovery from an exported local queue (N2 A1, "导出到文件…").
//!
//! The A1 export keeps the raw outbox items when they cannot be appended to the
//! current document, typically because a newer writer already extended the
//! history (`writer_epoch_conflict`). Each writer's queue was written against
//! a prefix of the stored history (`expected_head`), so it is rebuilt on that
//! exact prefix as a self-contained recovery draft: nothing is merged into the
//! original note and nothing is dropped because the head moved on.
use crate::ink_lan::import_recovery_in;
use crate::{SqliteArtifactStore, SqliteStoreError, StoredInkSession, ink};
use mathscape_domain::{InkLabDocument, InkLabDocumentMetadata, InkLabOperation};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const INK_UNSAVED_OPERATIONS_FORMAT: &str = "mathscape-ink-unsaved-operations";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkUnsavedOperationsExport {
    pub format: String,
    pub version: u32,
    pub document_id: String,
    #[serde(default)]
    pub exported_at: Option<String>,
    pub items: Vec<InkUnsavedOperation>,
}

/// One outbox item as the desktop stored it. `quarantined` and `queued_at`
/// are kept for the user's information and ignored by the rebuild.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkUnsavedOperation {
    pub request_id: String,
    pub document_id: String,
    pub session_id: String,
    pub writer_epoch: u64,
    pub expected_head: u64,
    pub client_seq: u64,
    #[serde(default)]
    pub queued_at: Option<String>,
    pub metadata: InkLabDocumentMetadata,
    pub operation: InkLabOperation,
    #[serde(default)]
    pub quarantined: Option<serde_json::Value>,
}

fn invalid(message: impl Into<String>) -> SqliteStoreError {
    SqliteStoreError::InkValidation(message.into())
}

fn set_seq(operation: &mut InkLabOperation, value: u32) {
    match operation {
        InkLabOperation::Stroke { seq, .. }
        | InkLabOperation::Erase { seq, .. }
        | InkLabOperation::PartialErase { seq, .. }
        | InkLabOperation::Transform { seq, .. }
        | InkLabOperation::Restyle { seq, .. }
        | InkLabOperation::PageAdd { seq, .. }
        | InkLabOperation::Undo { seq, .. }
        | InkLabOperation::Redo { seq, .. } => *seq = value,
    }
}

/// Builds one recovery document per writer queue (session, epoch): the stored
/// history up to the queue's `expected_head`, then the queued operations in
/// client order, renumbered to follow that prefix.
pub(crate) fn rebuild_documents(
    export: &InkUnsavedOperationsExport,
    original: Option<&InkLabDocument>,
    note_title: Option<&str>,
) -> Result<Vec<InkLabDocument>, SqliteStoreError> {
    if export.format != INK_UNSAVED_OPERATIONS_FORMAT || export.version != 1 {
        return Err(invalid("不支持的未保存笔迹导出格式"));
    }
    if export.items.is_empty() {
        return Err(invalid("导出文件里没有未保存的笔迹"));
    }
    let mut queues: BTreeMap<(u64, String), Vec<&InkUnsavedOperation>> = BTreeMap::new();
    for item in &export.items {
        if item.document_id != export.document_id || item.metadata.document_id != export.document_id
        {
            return Err(invalid("导出文件混有其他手稿的笔迹"));
        }
        queues
            .entry((item.writer_epoch, item.session_id.clone()))
            .or_default()
            .push(item);
    }
    let mut documents = Vec::new();
    for items in queues.values_mut() {
        items.sort_by_key(|item| item.client_seq);
        let base = items[0].expected_head;
        let mut operations = match original {
            Some(original) if original.operations.len() as u64 >= base => {
                original.operations[..base as usize].to_vec()
            }
            _ if base == 0 => Vec::new(),
            _ => {
                return Err(invalid(format!(
                    "原手稿缺少这些笔迹所基于的前 {base} 个操作，无法重建恢复草稿"
                )));
            }
        };
        let mut seen = std::collections::HashSet::new();
        for item in items.iter() {
            // A replayed export may list the same request twice.
            if !seen.insert(item.request_id.as_str()) {
                continue;
            }
            let mut operation = item.operation.clone();
            set_seq(&mut operation, operations.len() as u32 + 1);
            operations.push(operation);
        }
        let last = &items[items.len() - 1].metadata;
        let mut pages = last.pages.clone();
        let mut capture_segments = last.capture_segments.clone();
        if let Some(original) = original {
            // Pages and capture segments named by the prefix must stay present.
            for page in &original.pages {
                if !pages.iter().any(|known| known.id == page.id) {
                    pages.push(page.clone());
                }
            }
            for segment in &original.capture_segments {
                if !capture_segments.iter().any(|known| known.id == segment.id) {
                    capture_segments.push(segment.clone());
                }
            }
        }
        documents.push(InkLabDocument {
            version: last.version.clone(),
            document_id: export.document_id.clone(),
            title: note_title.unwrap_or(&last.title).to_owned(),
            created_at: last.created_at.clone(),
            exported_at: last.exported_at.clone(),
            pages,
            capture_segments,
            operations,
        });
    }
    Ok(documents)
}

impl SqliteArtifactStore {
    /// Imports an A1 unsaved-operations export as recovery drafts, one per
    /// writer queue. All drafts and their request receipts are written in one
    /// transaction: either every draft is persisted or none is. Idempotent per
    /// `request` (each draft derives its own recovery request id from it).
    pub fn import_ink_unsaved_operations(
        &mut self,
        export: &InkUnsavedOperationsExport,
        request: &str,
        now: i64,
    ) -> Result<Vec<StoredInkSession>, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = ink::open_document(&tx, &export.document_id)?;
        let documents = rebuild_documents(
            export,
            stored.as_ref().map(|stored| &stored.document),
            stored.as_ref().map(|stored| stored.note.title.as_str()),
        )?;
        // Validate every draft before writing any.
        for document in &documents {
            document
                .validate()
                .map_err(|error| invalid(format!("恢复草稿无效：{error}")))?;
        }
        let mut sessions = Vec::with_capacity(documents.len());
        for (index, document) in documents.iter().enumerate() {
            let derived = if index == 0 {
                request.to_owned()
            } else {
                format!("{:x}", Sha256::digest(format!("{request}:{index}")))[..32].to_owned()
            };
            // Dropping `tx` on error rolls back every draft written so far.
            sessions.push(import_recovery_in(&tx, document, &derived, now)?);
        }
        tx.commit()?;
        Ok(sessions)
    }
}
