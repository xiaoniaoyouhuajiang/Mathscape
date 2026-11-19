//! Append-only block/range Markdown errata overlay sidecar (`markdown_block_errata`
//! and `markdown_errata_unit_reviews`, migration 0026).
//!
//! Physical Markdown Artifact bytes remain immutable. Every active errata owns a
//! disjoint contiguous physical block range `[base_byte_start, base_byte_end)` on
//! one physical snapshot; overlapping re-edits supersede prior active rows in a
//! single transaction so full edit history and instant revert are preserved.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::SqliteStoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownErrataStatus {
    Active,
    Reverted,
    Superseded,
}

impl MarkdownErrataStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Reverted => "reverted",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, SqliteStoreError> {
        match raw {
            "active" => Ok(Self::Active),
            "reverted" => Ok(Self::Reverted),
            "superseded" => Ok(Self::Superseded),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "markdown_block_errata.status",
                value: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownErrataSourceKind {
    Manual,
    DiscoveryPatch,
}

impl MarkdownErrataSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::DiscoveryPatch => "discovery_patch",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, SqliteStoreError> {
        match raw {
            "manual" => Ok(Self::Manual),
            "discovery_patch" => Ok(Self::DiscoveryPatch),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "markdown_block_errata.source_kind",
                value: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrataUnitReviewDecision {
    Acknowledged,
    SentToSandbox,
}

impl ErrataUnitReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acknowledged => "acknowledged",
            Self::SentToSandbox => "sent_to_sandbox",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, SqliteStoreError> {
        match raw {
            "acknowledged" => Ok(Self::Acknowledged),
            "sent_to_sandbox" => Ok(Self::SentToSandbox),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "markdown_errata_unit_reviews.decision",
                value: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownBlockErrata {
    pub errata_id: String,
    pub artifact_id: String,
    pub base_content_hash: String,
    pub first_block_id: String,
    pub last_block_id: String,
    pub covered_block_ids: Vec<String>,
    pub base_byte_start: u64,
    pub base_byte_end: u64,
    pub base_text_hash: String,
    pub original_text: String,
    pub corrected_text: String,
    pub status: MarkdownErrataStatus,
    pub source_kind: MarkdownErrataSourceKind,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_pdf_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_set_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpsertMarkdownErrata {
    pub errata_id: String,
    pub artifact_id: String,
    pub base_content_hash: String,
    pub first_block_id: String,
    pub last_block_id: String,
    pub covered_block_ids: Vec<String>,
    pub base_byte_start: u64,
    pub base_byte_end: u64,
    pub base_text_hash: String,
    pub original_text: String,
    pub corrected_text: String,
    pub source_kind: MarkdownErrataSourceKind,
    pub actor: String,
    pub note: Option<String>,
    pub verified_pdf_page: Option<u32>,
    pub discovery_run_id: Option<String>,
    pub patch_set_id: Option<String>,
    pub now: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertMarkdownErrataOutcome {
    pub errata: MarkdownBlockErrata,
    pub superseded_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrataUnitReviewRecord {
    pub unit_id: String,
    pub errata_id: String,
    pub decision: ErrataUnitReviewDecision,
    pub reviewed_at: String,
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    for table in ["markdown_block_errata", "markdown_errata_unit_reviews"] {
        let exists: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .ok();
        if exists.is_none() {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
    }
    connection.prepare(
        "SELECT errata_id, artifact_id, base_content_hash, first_block_id, last_block_id,
                covered_block_ids_json, base_byte_start, base_byte_end, base_text_hash,
                original_text, corrected_text, status, source_kind, actor, note,
                verified_pdf_page, discovery_run_id, patch_set_id, superseded_by,
                created_at, updated_at
         FROM markdown_block_errata LIMIT 0",
    )?;
    connection.prepare(
        "SELECT unit_id, errata_id, decision, reviewed_at
         FROM markdown_errata_unit_reviews LIMIT 0",
    )?;
    Ok(())
}

const SELECT_COLUMNS: &str = "
    errata_id, artifact_id, base_content_hash, first_block_id, last_block_id,
    covered_block_ids_json, base_byte_start, base_byte_end, base_text_hash,
    original_text, corrected_text, status, source_kind, actor, note,
    verified_pdf_page, discovery_run_id, patch_set_id, superseded_by,
    created_at, updated_at
";

fn map_errata_row(row: &rusqlite::Row<'_>) -> Result<MarkdownBlockErrata, SqliteStoreError> {
    let covered_json: String = row.get(5)?;
    let covered_block_ids: Vec<String> = serde_json::from_str(&covered_json).map_err(|_| {
        SqliteStoreError::InvalidStoredValue {
            field: "markdown_block_errata.covered_block_ids_json",
            value: covered_json,
        }
    })?;
    let start_i64: i64 = row.get(6)?;
    let end_i64: i64 = row.get(7)?;
    let base_byte_start = u64::try_from(start_i64).map_err(|_| SqliteStoreError::IntegerRange {
        field: "markdown_block_errata.base_byte_start",
        value: start_i64 as u64,
    })?;
    let base_byte_end = u64::try_from(end_i64).map_err(|_| SqliteStoreError::IntegerRange {
        field: "markdown_block_errata.base_byte_end",
        value: end_i64 as u64,
    })?;
    let status_raw: String = row.get(11)?;
    let source_kind_raw: String = row.get(12)?;
    let verified_page_i64: Option<i64> = row.get(15)?;
    let verified_pdf_page = match verified_page_i64 {
        Some(v) => Some(u32::try_from(v).map_err(|_| SqliteStoreError::IntegerRange {
            field: "markdown_block_errata.verified_pdf_page",
            value: v as u64,
        })?),
        None => None,
    };
    Ok(MarkdownBlockErrata {
        errata_id: row.get(0)?,
        artifact_id: row.get(1)?,
        base_content_hash: row.get(2)?,
        first_block_id: row.get(3)?,
        last_block_id: row.get(4)?,
        covered_block_ids,
        base_byte_start,
        base_byte_end,
        base_text_hash: row.get(8)?,
        original_text: row.get(9)?,
        corrected_text: row.get(10)?,
        status: MarkdownErrataStatus::parse(&status_raw)?,
        source_kind: MarkdownErrataSourceKind::parse(&source_kind_raw)?,
        actor: row.get(13)?,
        note: row.get(14)?,
        verified_pdf_page,
        discovery_run_id: row.get(16)?,
        patch_set_id: row.get(17)?,
        superseded_by: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

pub(crate) fn list_active_markdown_errata(
    connection: &Connection,
    artifact_id: &str,
) -> Result<Vec<MarkdownBlockErrata>, SqliteStoreError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS}
         FROM markdown_block_errata
         WHERE artifact_id = ?1 AND status = 'active'
         ORDER BY base_byte_start ASC, base_byte_end ASC, errata_id ASC"
    );
    let mut stmt = connection.prepare(&sql)?;
    let mut rows = stmt.query(params![artifact_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_errata_row(row)?);
    }
    Ok(out)
}

pub(crate) fn list_all_active_markdown_errata(
    connection: &Connection,
) -> Result<Vec<MarkdownBlockErrata>, SqliteStoreError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS}
         FROM markdown_block_errata
         WHERE status = 'active'
         ORDER BY artifact_id ASC, base_byte_start ASC, errata_id ASC"
    );
    let mut stmt = connection.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_errata_row(row)?);
    }
    Ok(out)
}

pub(crate) fn list_markdown_errata_history(
    connection: &Connection,
    artifact_id: &str,
    block_id: Option<&str>,
) -> Result<Vec<MarkdownBlockErrata>, SqliteStoreError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS}
         FROM markdown_block_errata
         WHERE artifact_id = ?1
         ORDER BY created_at DESC, errata_id DESC"
    );
    let mut stmt = connection.prepare(&sql)?;
    let mut rows = stmt.query(params![artifact_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let item = map_errata_row(row)?;
        if let Some(target_block) = block_id
            && !item.covered_block_ids.iter().any(|b| b == target_block)
        {
            continue;
        }
        out.push(item);
    }
    Ok(out)
}

pub(crate) fn get_markdown_errata(
    connection: &Connection,
    errata_id: &str,
) -> Result<Option<MarkdownBlockErrata>, SqliteStoreError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS}
         FROM markdown_block_errata
         WHERE errata_id = ?1"
    );
    let mut stmt = connection.prepare(&sql)?;
    let mut rows = stmt.query(params![errata_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(map_errata_row(row)?)),
        None => Ok(None),
    }
}

/// Inserts a new active errata row and atomically marks any overlapping active
/// rows on the same artifact as `superseded`.
///
/// The caller must already have expanded `[base_byte_start, base_byte_end)` to
/// cover the union of all overlapping active rows so no partial overlap remains.
pub(crate) fn upsert_markdown_errata(
    connection: &mut Connection,
    input: UpsertMarkdownErrata,
) -> Result<UpsertMarkdownErrataOutcome, SqliteStoreError> {
    if input.base_byte_end <= input.base_byte_start || input.covered_block_ids.is_empty() {
        return Err(SqliteStoreError::ArtifactInvariant(
            "markdown errata requires a non-empty block range",
        ));
    }
    let start_i64 = i64::try_from(input.base_byte_start).map_err(|_| {
        SqliteStoreError::IntegerRange {
            field: "markdown_block_errata.base_byte_start",
            value: input.base_byte_start,
        }
    })?;
    let end_i64 = i64::try_from(input.base_byte_end).map_err(|_| {
        SqliteStoreError::IntegerRange {
            field: "markdown_block_errata.base_byte_end",
            value: input.base_byte_end,
        }
    })?;
    let covered_json = serde_json::to_string(&input.covered_block_ids).map_err(|_| {
        SqliteStoreError::ArtifactInvariant("failed to serialize covered_block_ids")
    })?;
    let note = input
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let verified_page_i64 = input.verified_pdf_page.map(i64::from);

    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let overlapping: Vec<(String, i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT errata_id, base_byte_start, base_byte_end
             FROM markdown_block_errata
             WHERE artifact_id = ?1
               AND status = 'active'
               AND base_byte_start < ?3
               AND base_byte_end > ?2
             ORDER BY base_byte_start ASC, errata_id ASC",
        )?;
        let rows = stmt.query_map(params![input.artifact_id, start_i64, end_i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        items
    };

    for (_, ov_start, ov_end) in &overlapping {
        if *ov_start < start_i64 || *ov_end > end_i64 {
            return Err(SqliteStoreError::ArtifactInvariant(
                "markdown errata range must cover the full union of overlapping active errata",
            ));
        }
    }

    let superseded_ids: Vec<String> = overlapping.into_iter().map(|(id, _, _)| id).collect();
    for old_id in &superseded_ids {
        tx.execute(
            "UPDATE markdown_block_errata
             SET status = 'superseded', superseded_by = ?2, updated_at = ?3
             WHERE errata_id = ?1",
            params![old_id, input.errata_id, input.now],
        )?;
    }

    tx.execute(
        "INSERT INTO markdown_block_errata(
            errata_id, artifact_id, base_content_hash, first_block_id, last_block_id,
            covered_block_ids_json, base_byte_start, base_byte_end, base_text_hash,
            original_text, corrected_text, status, source_kind, actor, note,
            verified_pdf_page, discovery_run_id, patch_set_id, superseded_by,
            created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9,
            ?10, ?11, 'active', ?12, ?13, ?14,
            ?15, ?16, ?17, NULL,
            ?18, ?18
         )",
        params![
            input.errata_id,
            input.artifact_id,
            input.base_content_hash,
            input.first_block_id,
            input.last_block_id,
            covered_json,
            start_i64,
            end_i64,
            input.base_text_hash,
            input.original_text,
            input.corrected_text,
            input.source_kind.as_str(),
            input.actor,
            note,
            verified_page_i64,
            input.discovery_run_id,
            input.patch_set_id,
            input.now,
        ],
    )?;

    tx.commit()?;

    Ok(UpsertMarkdownErrataOutcome {
        errata: MarkdownBlockErrata {
            errata_id: input.errata_id,
            artifact_id: input.artifact_id,
            base_content_hash: input.base_content_hash,
            first_block_id: input.first_block_id,
            last_block_id: input.last_block_id,
            covered_block_ids: input.covered_block_ids,
            base_byte_start: input.base_byte_start,
            base_byte_end: input.base_byte_end,
            base_text_hash: input.base_text_hash,
            original_text: input.original_text,
            corrected_text: input.corrected_text,
            status: MarkdownErrataStatus::Active,
            source_kind: input.source_kind,
            actor: input.actor,
            note,
            verified_pdf_page: input.verified_pdf_page,
            discovery_run_id: input.discovery_run_id,
            patch_set_id: input.patch_set_id,
            superseded_by: None,
            created_at: input.now.clone(),
            updated_at: input.now,
        },
        superseded_ids,
    })
}

pub(crate) fn revert_markdown_errata(
    connection: &mut Connection,
    errata_id: &str,
    now: &str,
) -> Result<Option<MarkdownBlockErrata>, SqliteStoreError> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing_status: Option<String> = tx
        .query_row(
            "SELECT status FROM markdown_block_errata WHERE errata_id = ?1",
            params![errata_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(status_raw) = existing_status else {
        return Ok(None);
    };
    if status_raw == "active" {
        tx.execute(
            "UPDATE markdown_block_errata
             SET status = 'reverted', updated_at = ?2
             WHERE errata_id = ?1",
            params![errata_id, now],
        )?;
    }
    tx.commit()?;
    get_markdown_errata(connection, errata_id)
}

pub(crate) fn mark_errata_unit_reviewed(
    connection: &mut Connection,
    unit_id: &str,
    errata_id: &str,
    decision: ErrataUnitReviewDecision,
    reviewed_at: &str,
) -> Result<ErrataUnitReviewRecord, SqliteStoreError> {
    let trimmed_unit = unit_id.trim();
    let trimmed_errata = errata_id.trim();
    if trimmed_unit.is_empty() || trimmed_errata.is_empty() {
        return Err(SqliteStoreError::ArtifactInvariant(
            "unit_id and errata_id must not be empty",
        ));
    }
    connection.execute(
        "INSERT INTO markdown_errata_unit_reviews(unit_id, errata_id, decision, reviewed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(unit_id, errata_id) DO UPDATE SET
             decision = excluded.decision,
             reviewed_at = excluded.reviewed_at",
        params![trimmed_unit, trimmed_errata, decision.as_str(), reviewed_at],
    )?;
    Ok(ErrataUnitReviewRecord {
        unit_id: trimmed_unit.to_owned(),
        errata_id: trimmed_errata.to_owned(),
        decision,
        reviewed_at: reviewed_at.to_owned(),
    })
}

pub(crate) fn list_errata_unit_reviews(
    connection: &Connection,
    unit_id: Option<&str>,
) -> Result<Vec<ErrataUnitReviewRecord>, SqliteStoreError> {
    let mut out = Vec::new();
    if let Some(uid) = unit_id {
        let mut stmt = connection.prepare(
            "SELECT unit_id, errata_id, decision, reviewed_at
             FROM markdown_errata_unit_reviews
             WHERE unit_id = ?1
             ORDER BY reviewed_at DESC, errata_id ASC",
        )?;
        let mut rows = stmt.query(params![uid])?;
        while let Some(row) = rows.next()? {
            let decision_raw: String = row.get(2)?;
            out.push(ErrataUnitReviewRecord {
                unit_id: row.get(0)?,
                errata_id: row.get(1)?,
                decision: ErrataUnitReviewDecision::parse(&decision_raw)?,
                reviewed_at: row.get(3)?,
            });
        }
    } else {
        let mut stmt = connection.prepare(
            "SELECT unit_id, errata_id, decision, reviewed_at
             FROM markdown_errata_unit_reviews
             ORDER BY unit_id ASC, errata_id ASC",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let decision_raw: String = row.get(2)?;
            out.push(ErrataUnitReviewRecord {
                unit_id: row.get(0)?,
                errata_id: row.get(1)?,
                decision: ErrataUnitReviewDecision::parse(&decision_raw)?,
                reviewed_at: row.get(3)?,
            });
        }
    }
    Ok(out)
}
