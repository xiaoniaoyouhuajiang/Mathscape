use std::collections::HashSet;

use mathscape_domain::{
    ActorRef, ContentHash, DiscoveryAdmission, DiscoveryUnitKind, StableId, Timestamp,
    UnitCompleteness,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{SqliteStoreError, sqlite_bool};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryCheckpointStatus {
    Running,
    AwaitingPatchReview,
    Completed,
    Partial,
    NeedsAttention,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryAttemptCheckpoint {
    pub stage: String,
    pub batch_fingerprint: Option<ContentHash>,
    pub repair: bool,
    pub status: DiscoveryCheckpointStatus,
    pub request: Value,
    pub raw_text: Option<String>,
    pub run: Value,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDiagnosticCheckpoint {
    pub code: String,
    pub path: String,
    pub message: String,
    pub checked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryValidationCheckpoint {
    pub report: Value,
    pub diagnostics: Vec<DiscoveryDiagnosticCheckpoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryUnitCheckpointRecord {
    pub unit_ref: String,
    pub parent_ref: Option<String>,
    pub unit_kind: DiscoveryUnitKind,
    pub completeness: UnitCompleteness,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryFactCheckpointRecord {
    pub task_ref: String,
    pub owner_unit_ref: String,
    pub processed: bool,
    pub requires_unit_revision: bool,
    pub payload: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingDiscoveryTask {
    pub task_ref: String,
    pub owner_unit_ref: Option<String>,
}

/// Replayable audit/checkpoint document. Loading this value never restores the
/// validator-only authority of `ValidatedUnitSet` or `ValidatedFactTask`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryRunCheckpoint {
    pub run_id: StableId,
    pub artifact_id: StableId,
    pub conversion_id: Option<StableId>,
    pub pipeline_version: String,
    pub input_fingerprint: ContentHash,
    pub selection_fingerprint: ContentHash,
    pub unit_set_fingerprint: Option<ContentHash>,
    pub result_fingerprint: Option<ContentHash>,
    pub status: DiscoveryCheckpointStatus,
    pub admission: DiscoveryAdmission,
    pub profile_id: String,
    pub snapshot: Value,
    pub attempts: Vec<DiscoveryAttemptCheckpoint>,
    pub reports: Vec<DiscoveryValidationCheckpoint>,
    pub units: Vec<DiscoveryUnitCheckpointRecord>,
    pub facts: Vec<DiscoveryFactCheckpointRecord>,
    pub pending_tasks: Vec<PendingDiscoveryTask>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredDiscoveryRun {
    pub checkpoint: DiscoveryRunCheckpoint,
    pub created_at: String,
    pub updated_at: String,
}

/// Compact, query-oriented metadata used by the Desktop run ledger. The full
/// checkpoint is deliberately not loaded for list views.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredDiscoveryRunSummary {
    pub run_id: StableId,
    pub artifact_id: StableId,
    pub artifact_name: Option<String>,
    pub conversion_id: Option<StableId>,
    pub input_fingerprint: ContentHash,
    pub result_fingerprint: Option<ContentHash>,
    pub pipeline_version: String,
    pub status: DiscoveryCheckpointStatus,
    pub review_status: String,
    pub registrations: Vec<mathscape_domain::RegistrationSummary>,
    pub profile_id: String,
    pub unit_count: usize,
    pub fact_count: usize,
    pub pending_task_count: usize,
    pub unresolved_reference_count: usize,
    pub relation_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryRunDispositionReceipt {
    pub run_id: StableId,
    pub disposition: String,
    pub request_id: StableId,
    pub actor: ActorRef,
    pub reason: String,
    pub decided_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredAcceptanceAuthority {
    HumanReview,
    AutomaticPolicy,
}

/// Immutable accepted-result record plus the pre-commit registration plan.
/// Persisting it records authorization, but still does not commit map nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptedRegistrationPlanRecord {
    pub decision_id: StableId,
    pub run_id: StableId,
    pub actor: ActorRef,
    pub decided_at: Timestamp,
    pub authority: StoredAcceptanceAuthority,
    pub expected_result_fingerprint: ContentHash,
    pub selection: Value,
    pub plan_fingerprint: ContentHash,
    pub plan: Value,
}

impl DiscoveryRunCheckpoint {
    fn validate(&self) -> Result<(), SqliteStoreError> {
        for (field, value) in [
            ("discovery.pipeline_version", self.pipeline_version.as_str()),
            ("discovery.profile_id", self.profile_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(SqliteStoreError::DiscoveryInvariant(format!(
                    "{field} cannot be blank"
                )));
            }
        }
        if !self.snapshot.is_object() {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "discovery snapshot must be a JSON object".into(),
            ));
        }
        let unit_refs: HashSet<_> = self
            .units
            .iter()
            .map(|unit| unit.unit_ref.as_str())
            .collect();
        if unit_refs.len() != self.units.len() {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "discovery unit refs must be unique".into(),
            ));
        }
        if self.status == DiscoveryCheckpointStatus::AwaitingPatchReview
            && (self.units.is_empty()
                || self.result_fingerprint.is_some()
                || !self.facts.is_empty()
                || !self.pending_tasks.is_empty())
        {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "a run awaiting patch review holds a frozen UnitSet and no Stage B state".into(),
            ));
        }
        if !self.units.is_empty() && self.unit_set_fingerprint.is_none() {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "a non-empty unit checkpoint requires its UnitSet fingerprint".into(),
            ));
        }
        for unit in &self.units {
            if unit.unit_ref.trim().is_empty() || !unit.payload.is_object() {
                return Err(SqliteStoreError::DiscoveryInvariant(
                    "discovery unit ref/payload is invalid".into(),
                ));
            }
            if unit
                .parent_ref
                .as_deref()
                .is_some_and(|parent| parent == unit.unit_ref || !unit_refs.contains(parent))
            {
                return Err(SqliteStoreError::DiscoveryInvariant(format!(
                    "unit {} has an invalid parent",
                    unit.unit_ref
                )));
            }
        }
        let task_refs: HashSet<_> = self
            .facts
            .iter()
            .map(|fact| fact.task_ref.as_str())
            .collect();
        if task_refs.len() != self.facts.len() {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "validated fact task refs must be unique".into(),
            ));
        }
        for fact in &self.facts {
            if fact.task_ref.trim().is_empty()
                || !unit_refs.contains(fact.owner_unit_ref.as_str())
                || !fact.payload.is_object()
            {
                return Err(SqliteStoreError::DiscoveryInvariant(format!(
                    "fact task {} has an invalid owner or payload",
                    fact.task_ref
                )));
            }
        }
        let mut pending = HashSet::new();
        for task in &self.pending_tasks {
            if task.task_ref.trim().is_empty()
                || !pending.insert(task.task_ref.as_str())
                || task
                    .owner_unit_ref
                    .as_deref()
                    .is_some_and(|owner| !unit_refs.contains(owner))
            {
                return Err(SqliteStoreError::DiscoveryInvariant(
                    "pending task has an invalid ref or owner".into(),
                ));
            }
        }
        if self.attempts.iter().any(|attempt| {
            attempt.stage.trim().is_empty()
                || !attempt.request.is_object()
                || !attempt.run.is_object()
        }) || self.reports.iter().any(|report| !report.report.is_object())
        {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "attempt or validation report payload is invalid".into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn save_discovery_checkpoint(
    connection: &mut Connection,
    checkpoint: &DiscoveryRunCheckpoint,
) -> Result<(), SqliteStoreError> {
    checkpoint.validate()?;
    let transaction = connection.transaction()?;
    save_discovery_checkpoint_in(&transaction, checkpoint)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn save_discovery_checkpoint_in(
    transaction: &Transaction<'_>,
    checkpoint: &DiscoveryRunCheckpoint,
) -> Result<(), SqliteStoreError> {
    checkpoint.validate()?;
    let existing = transaction
        .query_row(
            "SELECT artifact_id, pipeline_version, input_fingerprint, selection_fingerprint,
                    status
             FROM discovery_runs WHERE run_id = ?1",
            [checkpoint.run_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    if let Some((artifact, pipeline, input, selection, stored_status)) = existing {
        if pipeline != checkpoint.pipeline_version {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "run ID is already bound to another discovery pipeline".into(),
            ));
        }
        // Only a run paused before Stage B may move onto the patched Markdown
        // snapshot; every other state keeps its source identity for life.
        let rebinding = artifact != checkpoint.artifact_id.as_str()
            || input != checkpoint.input_fingerprint.as_str()
            || selection != checkpoint.selection_fingerprint.as_str();
        if rebinding
            && parse_status(&stored_status)? != DiscoveryCheckpointStatus::AwaitingPatchReview
        {
            return Err(SqliteStoreError::DiscoveryInvariant(
                "run ID is already bound to another discovery input".into(),
            ));
        }
    }

    let snapshot = serde_json::to_string(&checkpoint.snapshot)?;
    transaction.execute(
        "INSERT INTO discovery_runs (
            run_id, artifact_id, conversion_id, pipeline_version, input_fingerprint,
            selection_fingerprint, unit_set_fingerprint, result_fingerprint,
            status, admission, profile_id, snapshot_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(run_id) DO UPDATE SET
            artifact_id = excluded.artifact_id,
            conversion_id = excluded.conversion_id,
            input_fingerprint = excluded.input_fingerprint,
            selection_fingerprint = excluded.selection_fingerprint,
            unit_set_fingerprint = excluded.unit_set_fingerprint,
            result_fingerprint = excluded.result_fingerprint,
            status = excluded.status,
            admission = excluded.admission,
            profile_id = excluded.profile_id,
            snapshot_json = excluded.snapshot_json,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![
            checkpoint.run_id.as_str(),
            checkpoint.artifact_id.as_str(),
            checkpoint.conversion_id.as_ref().map(StableId::as_str),
            checkpoint.pipeline_version,
            checkpoint.input_fingerprint.as_str(),
            checkpoint.selection_fingerprint.as_str(),
            checkpoint
                .unit_set_fingerprint
                .as_ref()
                .map(ContentHash::as_str),
            checkpoint
                .result_fingerprint
                .as_ref()
                .map(ContentHash::as_str),
            status(checkpoint.status),
            admission(checkpoint.admission),
            checkpoint.profile_id,
            snapshot,
        ],
    )?;

    for table in [
        "discovery_pending_tasks",
        "discovery_fact_tasks",
        "discovery_units",
        "discovery_validation_reports",
        "discovery_attempts",
    ] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE run_id = ?1"),
            [checkpoint.run_id.as_str()],
        )?;
    }

    for (index, attempt) in checkpoint.attempts.iter().enumerate() {
        transaction.execute(
            "INSERT INTO discovery_attempts (
                run_id, ordinal, stage, batch_fingerprint, repair, status,
                request_json, raw_text, run_json, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                checkpoint.run_id.as_str(),
                i64::try_from(index + 1).expect("attempt ordinal fits i64"),
                attempt.stage,
                attempt.batch_fingerprint.as_ref().map(ContentHash::as_str),
                i64::from(attempt.repair),
                status(attempt.status),
                serde_json::to_string(&attempt.request)?,
                attempt.raw_text,
                serde_json::to_string(&attempt.run)?,
                attempt.error,
            ],
        )?;
    }
    for (report_index, report) in checkpoint.reports.iter().enumerate() {
        let report_ordinal = i64::try_from(report_index + 1).expect("report ordinal fits i64");
        transaction.execute(
            "INSERT INTO discovery_validation_reports (run_id, report_ordinal, report_json)
             VALUES (?1, ?2, ?3)",
            params![
                checkpoint.run_id.as_str(),
                report_ordinal,
                serde_json::to_string(&report.report)?,
            ],
        )?;
        for (diagnostic_index, diagnostic) in report.diagnostics.iter().enumerate() {
            transaction.execute(
                "INSERT INTO discovery_diagnostics (
                    run_id, report_ordinal, diagnostic_ordinal, code, path, message, checked
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    checkpoint.run_id.as_str(),
                    report_ordinal,
                    i64::try_from(diagnostic_index + 1).expect("diagnostic ordinal fits i64"),
                    diagnostic.code,
                    diagnostic.path,
                    diagnostic.message,
                    i64::from(diagnostic.checked),
                ],
            )?;
        }
    }
    let unit_set_fingerprint = checkpoint
        .unit_set_fingerprint
        .as_ref()
        .map(ContentHash::as_str)
        .unwrap_or_default();
    for unit in &checkpoint.units {
        transaction.execute(
            "INSERT INTO discovery_units (
                run_id, unit_ref, parent_ref, unit_kind, completeness,
                unit_set_fingerprint, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                checkpoint.run_id.as_str(),
                unit.unit_ref,
                unit.parent_ref,
                enum_json(unit.unit_kind)?,
                enum_json(unit.completeness)?,
                unit_set_fingerprint,
                serde_json::to_string(&unit.payload)?,
            ],
        )?;
    }
    for fact in &checkpoint.facts {
        transaction.execute(
            "INSERT INTO discovery_fact_tasks (
                run_id, task_ref, owner_unit_ref, processed,
                requires_unit_revision, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                checkpoint.run_id.as_str(),
                fact.task_ref,
                fact.owner_unit_ref,
                i64::from(fact.processed),
                i64::from(fact.requires_unit_revision),
                serde_json::to_string(&fact.payload)?,
            ],
        )?;
    }
    for pending in &checkpoint.pending_tasks {
        transaction.execute(
            "INSERT INTO discovery_pending_tasks (run_id, task_ref, owner_unit_ref)
             VALUES (?1, ?2, ?3)",
            params![
                checkpoint.run_id.as_str(),
                pending.task_ref,
                pending.owner_unit_ref,
            ],
        )?;
    }
    Ok(())
}

pub(super) fn read_discovery_checkpoint(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Option<StoredDiscoveryRun>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT artifact_id, conversion_id, pipeline_version, input_fingerprint,
                    selection_fingerprint, unit_set_fingerprint, result_fingerprint,
                    status, admission, profile_id, snapshot_json, created_at, updated_at
             FROM discovery_runs WHERE run_id = ?1",
            [run_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?;
    let Some((
        artifact_id,
        conversion_id,
        pipeline_version,
        input_fingerprint,
        selection_fingerprint,
        unit_set_fingerprint,
        result_fingerprint,
        stored_status,
        stored_admission,
        profile_id,
        snapshot,
        created_at,
        updated_at,
    )) = row
    else {
        return Ok(None);
    };

    let attempts = read_attempts(connection, run_id)?;
    let reports = read_reports(connection, run_id)?;
    let units = read_units(connection, run_id)?;
    let facts = read_facts(connection, run_id)?;
    let pending_tasks = read_pending(connection, run_id)?;
    let checkpoint = DiscoveryRunCheckpoint {
        run_id: run_id.clone(),
        artifact_id: StableId::parse(artifact_id)?,
        conversion_id: conversion_id.map(StableId::parse).transpose()?,
        pipeline_version,
        input_fingerprint: ContentHash::parse(input_fingerprint)?,
        selection_fingerprint: ContentHash::parse(selection_fingerprint)?,
        unit_set_fingerprint: unit_set_fingerprint.map(ContentHash::parse).transpose()?,
        result_fingerprint: result_fingerprint.map(ContentHash::parse).transpose()?,
        status: parse_status(&stored_status)?,
        admission: parse_admission(&stored_admission)?,
        profile_id,
        snapshot: serde_json::from_str(&snapshot)?,
        attempts,
        reports,
        units,
        facts,
        pending_tasks,
    };
    checkpoint.validate()?;
    Ok(Some(StoredDiscoveryRun {
        checkpoint,
        created_at,
        updated_at,
    }))
}

pub(super) fn list_discovery_run_summaries(
    connection: &Connection,
    artifact_id: Option<&StableId>,
) -> Result<Vec<StoredDiscoveryRunSummary>, SqliteStoreError> {
    let adoptions = super::registration::adoptions(connection)?;
    let mut statement = connection.prepare(
        "SELECT d.run_id, d.artifact_id, ar.original_filename, d.conversion_id, d.input_fingerprint,
                d.result_fingerprint, d.pipeline_version, d.status, d.profile_id,
                d.created_at, d.updated_at,
                (SELECT COUNT(*) FROM discovery_units u WHERE u.run_id = d.run_id),
                (SELECT COUNT(*) FROM discovery_fact_tasks f WHERE f.run_id = d.run_id),
                (SELECT COUNT(*) FROM discovery_pending_tasks p WHERE p.run_id = d.run_id),
                (SELECT COUNT(*) FROM json_each(d.snapshot_json, '$.result.references')
                 WHERE json_extract(value, '$.resolution.status')
                       IN ('unresolved', 'ambiguous', 'candidate')),
                COALESCE(json_array_length(d.snapshot_json, '$.result.relations'), 0),
                EXISTS(SELECT 1 FROM discovery_acceptance_decisions a WHERE a.run_id = d.run_id),
                EXISTS(SELECT 1 FROM source_registration_commits c
                       JOIN discovery_acceptance_decisions a ON a.decision_id = c.decision_id
                       WHERE a.run_id = d.run_id),
                EXISTS(SELECT 1 FROM discovery_run_dispositions x WHERE x.run_id = d.run_id)
         FROM discovery_runs d
         JOIN artifacts a ON a.artifact_id = d.artifact_id
         JOIN artifact_revisions ar ON ar.artifact_id = a.artifact_id
             AND ar.revision_id = a.current_revision
         WHERE (?1 IS NULL OR d.artifact_id = ?1)
         ORDER BY d.updated_at DESC, d.run_id DESC",
    )?;
    let rows = statement.query_map([artifact_id.map(StableId::as_str)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, i64>(13)?,
            row.get::<_, i64>(14)?,
            row.get::<_, i64>(15)?,
            row.get::<_, i64>(16)?,
            row.get::<_, i64>(17)?,
            row.get::<_, i64>(18)?,
        ))
    })?;
    rows.map(|row| {
        let (
            run_id,
            artifact_id,
            artifact_name,
            conversion_id,
            input_fingerprint,
            result_fingerprint,
            pipeline_version,
            status,
            profile_id,
            created_at,
            updated_at,
            unit_count,
            fact_count,
            pending_task_count,
            unresolved_reference_count,
            relation_count,
            accepted,
            registered,
            discarded,
        ) = row?;
        let status = parse_status(&status)?;
        let registrations = adoptions
            .iter()
            .filter(|r| r.summary.run_id == run_id)
            .map(|r| r.summary.clone())
            .collect::<Vec<_>>();
        let review_status = if discarded != 0 {
            "discarded"
        } else if registered != 0
            && registrations
                .iter()
                .all(|r| r.state == mathscape_domain::RegistrationState::Withdrawn)
        {
            "withdrawn"
        } else if registered != 0 {
            "registered"
        } else if accepted != 0 {
            "accepted"
        } else {
            match status {
                DiscoveryCheckpointStatus::Running
                | DiscoveryCheckpointStatus::AwaitingPatchReview => "processing",
                DiscoveryCheckpointStatus::Completed | DiscoveryCheckpointStatus::Partial
                    if result_fingerprint.is_some() =>
                {
                    "reviewable"
                }
                // NeedsAttention is only reviewable when assembly produced a
                // result. A validation/transport stop may use the same
                // workflow status while leaving no result fingerprint; such
                // runs must open the diagnostic view instead.
                DiscoveryCheckpointStatus::NeedsAttention if result_fingerprint.is_some() => {
                    "reviewable"
                }
                _ => "diagnostic",
            }
        };
        Ok(StoredDiscoveryRunSummary {
            run_id: StableId::parse(run_id)?,
            artifact_id: StableId::parse(artifact_id)?,
            artifact_name,
            conversion_id: conversion_id.map(StableId::parse).transpose()?,
            input_fingerprint: ContentHash::parse(input_fingerprint)?,
            result_fingerprint: result_fingerprint.map(ContentHash::parse).transpose()?,
            pipeline_version,
            status,
            review_status: review_status.into(),
            registrations,
            profile_id,
            unit_count: usize::try_from(unit_count).map_err(|_| {
                SqliteStoreError::InvalidStoredValue {
                    field: "discovery_runs.unit_count",
                    value: unit_count.to_string(),
                }
            })?,
            fact_count: usize::try_from(fact_count).map_err(|_| {
                SqliteStoreError::InvalidStoredValue {
                    field: "discovery_runs.fact_count",
                    value: fact_count.to_string(),
                }
            })?,
            pending_task_count: usize::try_from(pending_task_count).map_err(|_| {
                SqliteStoreError::InvalidStoredValue {
                    field: "discovery_runs.pending_task_count",
                    value: pending_task_count.to_string(),
                }
            })?,
            unresolved_reference_count: usize::try_from(unresolved_reference_count).map_err(
                |_| SqliteStoreError::InvalidStoredValue {
                    field: "discovery_runs.unresolved_reference_count",
                    value: unresolved_reference_count.to_string(),
                },
            )?,
            relation_count: usize::try_from(relation_count).map_err(|_| {
                SqliteStoreError::InvalidStoredValue {
                    field: "discovery_runs.relation_count",
                    value: relation_count.to_string(),
                }
            })?,
            created_at,
            updated_at,
        })
    })
    .collect()
}

pub(super) fn discard_discovery_run(
    connection: &mut Connection,
    run_id: &StableId,
    request_id: &StableId,
    expected_result_fingerprint: &ContentHash,
    actor: &ActorRef,
    reason: &str,
    decided_at: &Timestamp,
) -> Result<DiscoveryRunDispositionReceipt, SqliteStoreError> {
    if reason.trim().is_empty() {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "discovery run disposition reason cannot be blank".into(),
        ));
    }
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let row = transaction
        .query_row(
            "SELECT result_fingerprint, status FROM discovery_runs WHERE run_id = ?1",
            [run_id.as_str()],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((stored_result, status)) = row else {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "discovery run does not exist".into(),
        ));
    };
    if stored_result.as_deref() != Some(expected_result_fingerprint.as_str()) {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "discovery result fingerprint is stale".into(),
        ));
    }
    if matches!(
        parse_status(&status)?,
        DiscoveryCheckpointStatus::Running | DiscoveryCheckpointStatus::AwaitingPatchReview
    ) {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "a running or patch-paused discovery run cannot be discarded here".into(),
        ));
    }
    let registered: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM source_registration_commits c JOIN discovery_acceptance_decisions a ON a.decision_id=c.decision_id WHERE a.run_id = ?1)",
        [run_id.as_str()],
        |row| row.get(0),
    )?;
    if registered {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "a registered discovery run must use registration reversal".into(),
        ));
    }
    if let Some(existing) = transaction
        .query_row(
            "SELECT run_id, disposition, reason, actor, request_id, decided_at
             FROM discovery_run_dispositions WHERE run_id = ?1",
            [run_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?
    {
        transaction.commit()?;
        return Ok(DiscoveryRunDispositionReceipt {
            run_id: StableId::parse(existing.0)?,
            disposition: existing.1,
            request_id: StableId::parse(existing.4)?,
            actor: ActorRef::parse(existing.3)?,
            reason: existing.2,
            decided_at: Timestamp::parse(existing.5)?,
        });
    }
    transaction.execute(
        "INSERT INTO discovery_run_dispositions
         (run_id, disposition, reason, actor, request_id, decided_at)
         VALUES (?1, 'discarded', ?2, ?3, ?4, ?5)",
        params![
            run_id.as_str(),
            reason,
            actor.as_str(),
            request_id.as_str(),
            decided_at.as_str(),
        ],
    )?;
    transaction.commit()?;
    Ok(DiscoveryRunDispositionReceipt {
        run_id: run_id.clone(),
        disposition: "discarded".into(),
        request_id: request_id.clone(),
        actor: actor.clone(),
        reason: reason.into(),
        decided_at: decided_at.clone(),
    })
}

pub(super) fn save_registration_plan(
    connection: &mut Connection,
    record: &AcceptedRegistrationPlanRecord,
) -> Result<(), SqliteStoreError> {
    if !record.selection.is_object() || !record.plan.is_object() {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "acceptance selection and registration plan must be JSON objects".into(),
        ));
    }
    let transaction = connection.transaction()?;
    let stored_result = transaction
        .query_row(
            "SELECT result_fingerprint FROM discovery_runs WHERE run_id = ?1",
            [record.run_id.as_str()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    if stored_result.as_ref().and_then(Option::as_deref)
        != Some(record.expected_result_fingerprint.as_str())
    {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "acceptance decision does not match the stored discovery result".into(),
        ));
    }
    let discarded: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM discovery_run_dispositions WHERE run_id = ?1)",
        [record.run_id.as_str()],
        |row| row.get(0),
    )?;
    if discarded {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "a discarded discovery result cannot be accepted".into(),
        ));
    }
    let existing = transaction
        .query_row(
            "SELECT run_id, expected_result_fingerprint, plan_fingerprint, plan_json
             FROM discovery_acceptance_decisions WHERE decision_id = ?1",
            [record.decision_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let plan = serde_json::to_string(&record.plan)?;
    if let Some((run, result, fingerprint, stored_plan)) = existing {
        if run == record.run_id.as_str()
            && result == record.expected_result_fingerprint.as_str()
            && fingerprint == record.plan_fingerprint.as_str()
            && stored_plan == plan
        {
            transaction.commit()?;
            return Ok(());
        }
        return Err(SqliteStoreError::DiscoveryInvariant(
            "decision ID is already bound to another accepted result".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO discovery_acceptance_decisions (
            decision_id, run_id, actor, decided_at, authority,
            expected_result_fingerprint, selection_json, plan_fingerprint, plan_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.decision_id.as_str(),
            record.run_id.as_str(),
            record.actor.as_str(),
            record.decided_at.as_str(),
            acceptance_authority(record.authority),
            record.expected_result_fingerprint.as_str(),
            serde_json::to_string(&record.selection)?,
            record.plan_fingerprint.as_str(),
            plan,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn read_registration_plan(
    connection: &Connection,
    decision_id: &StableId,
) -> Result<Option<AcceptedRegistrationPlanRecord>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT run_id, actor, decided_at, authority, expected_result_fingerprint,
                    selection_json, plan_fingerprint, plan_json
             FROM discovery_acceptance_decisions WHERE decision_id = ?1",
            [decision_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
        .map(
            |(run, actor, decided_at, authority, result, selection, plan_fingerprint, plan)| {
                Ok(AcceptedRegistrationPlanRecord {
                    decision_id: decision_id.clone(),
                    run_id: StableId::parse(run)?,
                    actor: ActorRef::parse(actor)?,
                    decided_at: Timestamp::parse(decided_at)?,
                    authority: parse_acceptance_authority(&authority)?,
                    expected_result_fingerprint: ContentHash::parse(result)?,
                    selection: serde_json::from_str(&selection)?,
                    plan_fingerprint: ContentHash::parse(plan_fingerprint)?,
                    plan: serde_json::from_str(&plan)?,
                })
            },
        )
        .transpose()
}

fn read_attempts(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Vec<DiscoveryAttemptCheckpoint>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT stage, batch_fingerprint, repair, status, request_json, raw_text, run_json, error
         FROM discovery_attempts WHERE run_id = ?1 ORDER BY ordinal",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    rows.map(|row| {
        let (stage, batch, repair, status_value, request, raw_text, run, error) = row?;
        Ok(DiscoveryAttemptCheckpoint {
            stage,
            batch_fingerprint: batch.map(ContentHash::parse).transpose()?,
            repair: sqlite_bool(repair, "discovery_attempts.repair")?,
            status: parse_status(&status_value)?,
            request: serde_json::from_str(&request)?,
            raw_text,
            run: serde_json::from_str(&run)?,
            error,
        })
    })
    .collect()
}

fn read_reports(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Vec<DiscoveryValidationCheckpoint>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT report_ordinal, report_json FROM discovery_validation_reports
         WHERE run_id = ?1 ORDER BY report_ordinal",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut reports = Vec::new();
    for row in rows {
        let (ordinal, report) = row?;
        let mut diagnostics_statement = connection.prepare(
            "SELECT code, path, message, checked FROM discovery_diagnostics
             WHERE run_id = ?1 AND report_ordinal = ?2 ORDER BY diagnostic_ordinal",
        )?;
        let diagnostics = diagnostics_statement
            .query_map(params![run_id.as_str(), ordinal], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .map(|row| {
                let (code, path, message, checked) = row?;
                Ok(DiscoveryDiagnosticCheckpoint {
                    code,
                    path,
                    message,
                    checked: sqlite_bool(checked, "discovery_diagnostics.checked")?,
                })
            })
            .collect::<Result<Vec<_>, SqliteStoreError>>()?;
        reports.push(DiscoveryValidationCheckpoint {
            report: serde_json::from_str(&report)?,
            diagnostics,
        });
    }
    Ok(reports)
}

fn read_units(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Vec<DiscoveryUnitCheckpointRecord>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT unit_ref, parent_ref, unit_kind, completeness, payload_json
         FROM discovery_units WHERE run_id = ?1 ORDER BY unit_ref",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (unit_ref, parent_ref, kind, completeness, payload) = row?;
        Ok(DiscoveryUnitCheckpointRecord {
            unit_ref,
            parent_ref,
            unit_kind: parse_enum(&kind, "discovery_units.unit_kind")?,
            completeness: parse_enum(&completeness, "discovery_units.completeness")?,
            payload: serde_json::from_str(&payload)?,
        })
    })
    .collect()
}

fn read_facts(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Vec<DiscoveryFactCheckpointRecord>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT task_ref, owner_unit_ref, processed, requires_unit_revision, payload_json
         FROM discovery_fact_tasks WHERE run_id = ?1 ORDER BY task_ref",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (task_ref, owner_unit_ref, processed, requires_revision, payload) = row?;
        Ok(DiscoveryFactCheckpointRecord {
            task_ref,
            owner_unit_ref,
            processed: sqlite_bool(processed, "discovery_fact_tasks.processed")?,
            requires_unit_revision: sqlite_bool(
                requires_revision,
                "discovery_fact_tasks.requires_unit_revision",
            )?,
            payload: serde_json::from_str(&payload)?,
        })
    })
    .collect()
}

fn read_pending(
    connection: &Connection,
    run_id: &StableId,
) -> Result<Vec<PendingDiscoveryTask>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT task_ref, owner_unit_ref FROM discovery_pending_tasks
         WHERE run_id = ?1 ORDER BY task_ref",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok(PendingDiscoveryTask {
            task_ref: row.get(0)?,
            owner_unit_ref: row.get(1)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn enum_json<T: Serialize>(value: T) -> Result<String, SqliteStoreError> {
    let Value::String(value) = serde_json::to_value(value)? else {
        return Err(SqliteStoreError::DiscoveryInvariant(
            "expected a string enum representation".into(),
        ));
    };
    Ok(value)
}

fn parse_enum<T: for<'de> Deserialize<'de>>(
    value: &str,
    field: &'static str,
) -> Result<T, SqliteStoreError> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| {
        SqliteStoreError::InvalidStoredValue {
            field,
            value: value.to_owned(),
        }
    })
}

fn status(value: DiscoveryCheckpointStatus) -> &'static str {
    match value {
        DiscoveryCheckpointStatus::Running => "running",
        DiscoveryCheckpointStatus::AwaitingPatchReview => "awaiting_patch_review",
        DiscoveryCheckpointStatus::Completed => "completed",
        DiscoveryCheckpointStatus::Partial => "partial",
        DiscoveryCheckpointStatus::NeedsAttention => "needs_attention",
        DiscoveryCheckpointStatus::Failed => "failed",
        DiscoveryCheckpointStatus::Cancelled => "cancelled",
    }
}

fn parse_status(value: &str) -> Result<DiscoveryCheckpointStatus, SqliteStoreError> {
    match value {
        "running" => Ok(DiscoveryCheckpointStatus::Running),
        "awaiting_patch_review" => Ok(DiscoveryCheckpointStatus::AwaitingPatchReview),
        "completed" => Ok(DiscoveryCheckpointStatus::Completed),
        "partial" => Ok(DiscoveryCheckpointStatus::Partial),
        "needs_attention" => Ok(DiscoveryCheckpointStatus::NeedsAttention),
        "failed" => Ok(DiscoveryCheckpointStatus::Failed),
        "cancelled" => Ok(DiscoveryCheckpointStatus::Cancelled),
        _ => Err(SqliteStoreError::InvalidStoredValue {
            field: "discovery_runs.status",
            value: value.to_owned(),
        }),
    }
}

fn admission(value: DiscoveryAdmission) -> &'static str {
    match value {
        DiscoveryAdmission::ReviewBeforePublish => "review_before_publish",
        DiscoveryAdmission::AutomaticCandidate => "automatic_candidate",
    }
}

fn parse_admission(value: &str) -> Result<DiscoveryAdmission, SqliteStoreError> {
    match value {
        "review_before_publish" => Ok(DiscoveryAdmission::ReviewBeforePublish),
        "automatic_candidate" => Ok(DiscoveryAdmission::AutomaticCandidate),
        _ => Err(SqliteStoreError::InvalidStoredValue {
            field: "discovery_runs.admission",
            value: value.to_owned(),
        }),
    }
}

fn acceptance_authority(value: StoredAcceptanceAuthority) -> &'static str {
    match value {
        StoredAcceptanceAuthority::HumanReview => "human_review",
        StoredAcceptanceAuthority::AutomaticPolicy => "automatic_policy",
    }
}

fn parse_acceptance_authority(value: &str) -> Result<StoredAcceptanceAuthority, SqliteStoreError> {
    match value {
        "human_review" => Ok(StoredAcceptanceAuthority::HumanReview),
        "automatic_policy" => Ok(StoredAcceptanceAuthority::AutomaticPolicy),
        _ => Err(SqliteStoreError::InvalidStoredValue {
            field: "discovery_acceptance_decisions.authority",
            value: value.to_owned(),
        }),
    }
}
