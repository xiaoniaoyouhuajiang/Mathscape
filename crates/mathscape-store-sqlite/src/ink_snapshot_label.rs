//! N3d named snapshots (migration v20).
//!
//! A snapshot's display name lives in `ink_snapshots.label` /
//! `label_revision`, outside the hashed manifest and the snapshot Artifact
//! bytes, so renaming never changes `content_hash`, `snapshot_id` or any
//! integrity check. Renames are optimistic (`expected_label_revision`) and
//! idempotent per `request_id`.

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ink::{StoredInkSnapshotSummary, non_negative};
use crate::ink_note::{InkLifecycle, digest, identifier, now_ms, read_note};
use crate::{SqliteArtifactStore, SqliteStoreError, table_has_column};

/// Maximum snapshot name length in Unicode scalars (not bytes).
pub const INK_SNAPSHOT_LABEL_MAX_CHARS: usize = 40;

const SNAPSHOT_LABEL_MIGRATION: &str =
    include_str!("../../../migrations/0020_ink_snapshot_labels_n3d.sql");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkSnapshotLabelRequest {
    pub request_id: String,
    pub snapshot_id: String,
    /// Trimmed; empty clears the name.
    pub label: String,
    pub expected_label_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkSnapshotLabelReceipt {
    pub request_id: String,
    pub snapshot: StoredInkSnapshotSummary,
    #[serde(default)]
    pub replayed: bool,
}

/// Product rules for a snapshot name. `Ok(None)` clears the name.
pub fn normalize_snapshot_label(input: &str) -> Result<Option<String>, SqliteStoreError> {
    let label = input.trim();
    if label.is_empty() {
        return Ok(None);
    }
    if label.chars().any(char::is_control) {
        return Err(SqliteStoreError::InkSnapshotLabelInvalid(
            "快照名称不能包含控制字符".into(),
        ));
    }
    if label.chars().count() > INK_SNAPSHOT_LABEL_MAX_CHARS {
        return Err(SqliteStoreError::InkSnapshotLabelInvalid(format!(
            "快照名称最多 {INK_SNAPSHOT_LABEL_MAX_CHARS} 个字符"
        )));
    }
    Ok(Some(label.into()))
}

/// v19 → v20: snapshot label columns and the rename request table.
pub(crate) fn migrate_v20(connection: &Connection) -> Result<(), SqliteStoreError> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(SNAPSHOT_LABEL_MIGRATION)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name: Option<String> = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=20",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_snapshot_labels_n3d") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 20 is absent or has an unexpected name",
        ));
    }
    for column in ["label", "label_revision"] {
        if !table_has_column(connection, "ink_snapshots", column)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "column",
                name: column,
            });
        }
    }
    connection.prepare(
        "SELECT request_id,snapshot_id,request_fingerprint,response_json,created_at
         FROM ink_snapshot_label_requests LIMIT 0",
    )?;
    Ok(())
}

pub(crate) fn read_summary(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Option<StoredInkSnapshotSummary>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT snapshot_id,request_id,document_id,through_seq,artifact_id,content_hash,
                    byte_size,created_at,label,label_revision
             FROM ink_snapshots WHERE snapshot_id=?1",
            [snapshot_id],
            |row| {
                Ok(StoredInkSnapshotSummary {
                    snapshot_id: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    through_seq: non_negative(row.get(3)?, 3)?,
                    artifact_id: row.get(4)?,
                    content_hash: row.get(5)?,
                    byte_size: non_negative(row.get(6)?, 6)?,
                    created_at: row.get(7)?,
                    label: row.get(8)?,
                    label_revision: non_negative(row.get(9)?, 9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn rename_in(
    tx: &Transaction<'_>,
    request: &InkSnapshotLabelRequest,
    now: i64,
) -> Result<InkSnapshotLabelReceipt, SqliteStoreError> {
    identifier(&request.request_id, "snapshot_label.request_id")?;
    identifier(&request.snapshot_id, "snapshot_label.snapshot_id")?;
    let fingerprint = digest(&(
        "snapshot_label",
        &request.snapshot_id,
        request.label.trim(),
        request.expected_label_revision,
    ))?;
    let stored: Option<(String, String)> = tx
        .query_row(
            "SELECT request_fingerprint,response_json FROM ink_snapshot_label_requests WHERE request_id=?1",
            [&request.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((stored_fingerprint, response)) = stored {
        if stored_fingerprint != fingerprint {
            return Err(SqliteStoreError::InkRequestConflict(
                request.request_id.clone(),
            ));
        }
        let mut value: Value = serde_json::from_str(&response)?;
        value["replayed"] = Value::Bool(true);
        return Ok(serde_json::from_value(value)?);
    }
    let label = normalize_snapshot_label(&request.label)?;
    let current = read_summary(tx, &request.snapshot_id)?
        .ok_or_else(|| SqliteStoreError::InkSnapshotNotFound(request.snapshot_id.clone()))?;
    // A trashed note is read-only, including the names of its snapshots.
    let note = read_note(tx, &current.document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(current.document_id.clone()))?;
    if note.lifecycle == InkLifecycle::Trashed {
        return Err(SqliteStoreError::InkDocumentTrashed(current.document_id));
    }
    if current.label_revision != request.expected_label_revision {
        return Err(SqliteStoreError::InkSnapshotLabelConflict {
            snapshot_id: current.snapshot_id,
            current_label: current.label,
            current_revision: current.label_revision,
        });
    }
    if label != current.label {
        // Only the label columns change; manifest, Artifact and hash do not.
        tx.execute(
            "UPDATE ink_snapshots SET label=?2,label_revision=label_revision+1 WHERE snapshot_id=?1",
            params![request.snapshot_id, label],
        )?;
    }
    let receipt = InkSnapshotLabelReceipt {
        request_id: request.request_id.clone(),
        snapshot: read_summary(tx, &request.snapshot_id)?
            .ok_or_else(|| SqliteStoreError::InkSnapshotNotFound(request.snapshot_id.clone()))?,
        replayed: false,
    };
    tx.execute(
        "INSERT INTO ink_snapshot_label_requests(request_id,snapshot_id,request_fingerprint,response_json,created_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            request.request_id,
            request.snapshot_id,
            fingerprint,
            serde_json::to_string(&receipt)?,
            now
        ],
    )?;
    Ok(receipt)
}

impl SqliteArtifactStore {
    /// Sets or clears a snapshot's display name (N3d). Never touches the
    /// snapshot bytes, manifest or content hash.
    pub fn rename_ink_snapshot(
        &mut self,
        request: &InkSnapshotLabelRequest,
    ) -> Result<InkSnapshotLabelReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = rename_in(&tx, request, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn ink_snapshot_summary(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<StoredInkSnapshotSummary>, SqliteStoreError> {
        read_summary(&self.connection, snapshot_id)
    }
}
