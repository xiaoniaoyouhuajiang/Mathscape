use mathscape_domain::{
    ContentHash, MarkdownPatchApplyReceipt, MarkdownPatchDecision, MarkdownPatchDecisionKind,
    MarkdownPatchSet, MarkdownPatchStatus, StableId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::SqliteStoreError;

#[derive(Clone, Debug, PartialEq)]
pub struct StoredMarkdownPatchSet {
    pub patch_set: MarkdownPatchSet,
    pub status: MarkdownPatchStatus,
    pub patched_artifact_id: Option<StableId>,
    pub conversion_id: Option<StableId>,
}

pub(super) fn save_patch_set(
    connection: &mut Connection,
    patch_set_id: &StableId,
    patch_set: &MarkdownPatchSet,
    status: MarkdownPatchStatus,
    patched_artifact_id: Option<&StableId>,
    conversion_id: Option<&StableId>,
    now: &Timestamp,
) -> Result<(), SqliteStoreError> {
    if patch_set.version != mathscape_domain::MARKDOWN_PATCH_SET_VERSION {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "unsupported Markdown patch set version".into(),
        ));
    }
    // A patch-set id is replayable identity, not an upsert escape hatch.  Do
    // not let a caller accidentally overwrite a set belonging to another
    // run/base/fingerprint (for example after a stale frontend retry).
    if let Some((stored_run, stored_base, stored_input, stored_fingerprint)) = connection
        .query_row(
            "SELECT run_id, base_artifact_id, base_input_fingerprint, patch_fingerprint
             FROM markdown_patch_sets WHERE patch_set_id = ?1",
            [patch_set_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
    {
        if stored_run != patch_set.run_id.as_str()
            || stored_base != patch_set.base_snapshot.artifact_id.as_str()
            || stored_input != patch_set.base_input_fingerprint.as_str()
            || stored_fingerprint != patch_set.fingerprint.as_str()
        {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "Markdown patch set identity conflicts with the stored record".into(),
            ));
        }
    }
    if let Some(stored_id) = connection
        .query_row(
            "SELECT patch_set_id FROM markdown_patch_sets WHERE patch_fingerprint = ?1",
            [patch_set.fingerprint.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        && stored_id != patch_set_id.as_str()
    {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "patch fingerprint is already bound to another patch set id".into(),
        ));
    }
    let transaction = connection.transaction()?;
    let snapshot = serde_json::to_string(&patch_set.base_snapshot)?;
    let patch_json = serde_json::to_string(patch_set)?;
    transaction.execute(
        "INSERT INTO markdown_patch_sets (
            patch_set_id, run_id, base_artifact_id, base_input_fingerprint,
            patch_fingerprint, status, base_snapshot_json, patch_json,
            patched_artifact_id, conversion_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT(patch_set_id) DO UPDATE SET
            status = excluded.status,
            patch_json = excluded.patch_json,
            patched_artifact_id = excluded.patched_artifact_id,
            conversion_id = excluded.conversion_id,
            updated_at = excluded.updated_at",
        params![
            patch_set_id.as_str(),
            patch_set.run_id.as_str(),
            patch_set.base_snapshot.artifact_id.as_str(),
            patch_set.base_input_fingerprint.as_str(),
            patch_set.fingerprint.as_str(),
            status_string(status),
            snapshot,
            patch_json,
            patched_artifact_id.map(StableId::as_str),
            conversion_id.map(StableId::as_str),
            now.as_str(),
        ],
    )?;
    transaction.execute(
        "DELETE FROM markdown_patch_operations WHERE patch_set_id = ?1",
        [patch_set_id.as_str()],
    )?;
    for (ordinal, operation) in patch_set.patches.iter().enumerate() {
        transaction.execute(
            "INSERT INTO markdown_patch_operations (
                patch_set_id, patch_ref, issue_ref, ordinal, old_start, old_end,
                old_text_hash, model_replacement, draft_replacement, draft_revision,
                draft_actor, draft_updated_at, status, operation_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, NULL, NULL, ?9, ?10)",
            params![
                patch_set_id.as_str(),
                operation.patch_ref,
                operation.issue_ref.as_str(),
                i64::try_from(ordinal).expect("patch ordinal fits i64"),
                i64::try_from(operation.range.start).map_err(|_| {
                    SqliteStoreError::IntegerRange {
                        field: "markdown_patch.old_start",
                        value: operation.range.start,
                    }
                })?,
                i64::try_from(operation.range.end).map_err(|_| SqliteStoreError::IntegerRange {
                    field: "markdown_patch.old_end",
                    value: operation.range.end,
                })?,
                operation.old_text_hash.as_str(),
                operation.replacement,
                status_string(operation.status),
                serde_json::to_string(operation)?,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn save_patch_draft(
    connection: &mut Connection,
    patch_set_id: &StableId,
    patch_ref: &str,
    replacement: &str,
    revision: u64,
    actor: &StableId,
    updated_at: &Timestamp,
) -> Result<(), SqliteStoreError> {
    let current_revision = connection
        .query_row(
            "SELECT draft_revision FROM markdown_patch_operations
             WHERE patch_set_id = ?1 AND patch_ref = ?2",
            params![patch_set_id.as_str(), patch_ref],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| {
            SqliteStoreError::DiscoveryInvariant("unknown Markdown patch draft operation".into())
        })?;
    let current_revision = u64::try_from(current_revision).map_err(|_| {
        SqliteStoreError::DiscoveryInvariant(
            "stored Markdown patch draft revision is invalid".into(),
        )
    })?;
    let revision = revision.max(current_revision.saturating_add(1));
    let changed = connection.execute(
        "UPDATE markdown_patch_operations
         SET draft_replacement = ?3, draft_revision = ?4, draft_actor = ?5,
             draft_updated_at = ?6
         WHERE patch_set_id = ?1 AND patch_ref = ?2",
        params![
            patch_set_id.as_str(),
            patch_ref,
            replacement,
            i64::try_from(revision).map_err(|_| SqliteStoreError::IntegerRange {
                field: "markdown_patch.draft_revision",
                value: revision,
            })?,
            actor.as_str(),
            updated_at.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "unknown Markdown patch draft operation".into(),
        ));
    }
    Ok(())
}

pub(super) fn update_patch_set_status(
    connection: &Connection,
    patch_set_id: &StableId,
    status: MarkdownPatchStatus,
    patched_artifact_id: Option<&StableId>,
    conversion_id: Option<&StableId>,
    updated_at: &Timestamp,
) -> Result<(), SqliteStoreError> {
    let changed = connection.execute(
        "UPDATE markdown_patch_sets
         SET status = ?2, patched_artifact_id = ?3, conversion_id = ?4, updated_at = ?5
         WHERE patch_set_id = ?1",
        params![
            patch_set_id.as_str(),
            status_string(status),
            patched_artifact_id.map(StableId::as_str),
            conversion_id.map(StableId::as_str),
            updated_at.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "unknown Markdown patch set".into(),
        ));
    }
    Ok(())
}

pub(super) fn record_patch_decision(
    connection: &mut Connection,
    decision: &MarkdownPatchDecision,
    created_at: &Timestamp,
    patch_set_id: &StableId,
) -> Result<(), SqliteStoreError> {
    connection.execute(
        "INSERT OR IGNORE INTO markdown_patch_decisions (
            decision_id, patch_set_id, run_id, actor, decision_kind,
            patch_set_fingerprint, accepted_patch_refs_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            decision.decision_id.as_str(),
            patch_set_id.as_str(),
            decision.run_id.as_str(),
            decision.actor.as_str(),
            decision_kind(decision.kind),
            decision.patch_set_fingerprint.as_str(),
            serde_json::to_string(&decision.accepted_patch_refs)?,
            created_at.as_str(),
        ],
    )?;
    Ok(())
}

pub(super) fn record_patch_application(
    connection: &mut Connection,
    application_id: &StableId,
    patch_set_id: &StableId,
    receipt: &MarkdownPatchApplyReceipt,
    error: Option<&str>,
    created_at: &Timestamp,
) -> Result<(), SqliteStoreError> {
    connection.execute(
        "INSERT OR REPLACE INTO markdown_patch_applications (
            application_id, patch_set_id, base_artifact_id, patched_artifact_id,
            conversion_id, old_input_fingerprint, new_input_fingerprint,
            safety_json, status, error, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            application_id.as_str(),
            patch_set_id.as_str(),
            receipt.base_artifact_id.as_str(),
            receipt.patched_artifact_id.as_ref().map(StableId::as_str),
            receipt.conversion_id.as_ref().map(StableId::as_str),
            receipt.old_input_fingerprint.as_str(),
            receipt
                .new_input_fingerprint
                .as_ref()
                .map(ContentHash::as_str),
            serde_json::to_string(&receipt.safety)?,
            status_string(receipt.status),
            error,
            created_at.as_str(),
        ],
    )?;
    Ok(())
}

pub(super) fn patch_set_json(
    connection: &Connection,
    patch_set_id: &StableId,
) -> Result<Option<Value>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT patch_json FROM markdown_patch_sets WHERE patch_set_id = ?1",
            [patch_set_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value).map_err(SqliteStoreError::from))
        .transpose()
}

pub(super) fn patch_drafts(
    connection: &Connection,
    patch_set_id: &StableId,
) -> Result<std::collections::BTreeMap<String, String>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT patch_ref, draft_replacement
         FROM markdown_patch_operations
         WHERE patch_set_id = ?1 AND draft_revision > 0
         ORDER BY ordinal",
    )?;
    statement
        .query_map([patch_set_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|row| row.map_err(SqliteStoreError::from))
        .collect()
}

pub(super) fn latest_patch_application(
    connection: &Connection,
    patch_set_id: &StableId,
) -> Result<Option<StoredPatchApplication>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT application_id, patched_artifact_id, conversion_id,
                    old_input_fingerprint, new_input_fingerprint, safety_json, status
             FROM markdown_patch_applications
             WHERE patch_set_id = ?1
             ORDER BY created_at DESC, application_id DESC
             LIMIT 1",
            [patch_set_id.as_str()],
            |row| {
                Ok(StoredPatchApplication {
                    application_id: row.get(0)?,
                    patched_artifact_id: row.get(1)?,
                    conversion_id: row.get(2)?,
                    old_input_fingerprint: row.get(3)?,
                    new_input_fingerprint: row.get(4)?,
                    safety_json: row.get(5)?,
                    status: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

#[derive(Clone, Debug)]
pub struct StoredPatchApplication {
    pub application_id: String,
    pub patched_artifact_id: Option<String>,
    pub conversion_id: Option<String>,
    pub old_input_fingerprint: String,
    pub new_input_fingerprint: Option<String>,
    pub safety_json: String,
    pub status: String,
}

pub(super) fn latest_patch_set_json_for_run(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Option<(StableId, Value, String)>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT patch_set_id, patch_json, status
             FROM markdown_patch_sets
             WHERE run_id = ?1
             ORDER BY updated_at DESC,
                      CASE status
                        WHEN 'applied' THEN 0
                        WHEN 'rejected' THEN 1
                        WHEN 'accepted' THEN 2
                        ELSE 3
                      END,
                      patch_set_id DESC
             LIMIT 1",
            [run_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    row.map(|(id, json, status)| Ok((StableId::parse(id)?, serde_json::from_str(&json)?, status)))
        .transpose()
}

fn status_string(status: MarkdownPatchStatus) -> &'static str {
    match status {
        MarkdownPatchStatus::Proposed => "proposed",
        MarkdownPatchStatus::Accepted => "accepted",
        MarkdownPatchStatus::Rejected => "rejected",
        MarkdownPatchStatus::Stale => "stale",
        MarkdownPatchStatus::Applied => "applied",
        MarkdownPatchStatus::Failed => "failed",
    }
}

fn decision_kind(kind: MarkdownPatchDecisionKind) -> &'static str {
    match kind {
        MarkdownPatchDecisionKind::Accept => "accept",
        MarkdownPatchDecisionKind::Reject => "reject",
    }
}
