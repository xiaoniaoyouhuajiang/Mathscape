//! Persistence for Phase2 proposed change sets.
//!
//! These records are intentionally separate from `map_objects` and
//! `map_object_revisions`: a proposal is an auditable candidate and must not
//! become part of the accepted map until a later Desktop acceptance flow.

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use crate::{SqliteArtifactStore, SqliteStoreError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposedChangeRecord {
    pub change_set_id: String,
    pub version: i64,
    pub investigation_id: String,
    pub round_id: String,
    pub generation: String,
    pub request_id: String,
    pub request_fingerprint: String,
    pub proposal: Value,
    pub result: Value,
    pub state: String,
    pub base_commit: Option<String>,
    pub created_at: String,
    pub proposed_by: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSetDependency {
    pub dependent_change_set_id: String,
    pub dependent_version: i64,
    pub depends_on_change_set_id: String,
    pub depends_on_version: i64,
    pub investigation_id: String,
    pub referenced_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSetDecision {
    pub decision_id: String,
    pub change_set_id: String,
    pub version: i64,
    pub investigation_id: String,
    pub kind: String,
    pub decided_by: String,
    pub plan_fingerprint: Option<String>,
    pub commit_id: Option<String>,
    pub receipt: Option<Value>,
    pub batch_id: Option<String>,
    pub reason: Option<String>,
    pub decided_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingChangeSet {
    pub record: ProposedChangeRecord,
    pub dependencies: Vec<ChangeSetDependency>,
}

impl SqliteArtifactStore {
    pub fn proposed_change_by_request(
        &self,
        investigation_id: &str,
        request_id: &str,
    ) -> Result<Option<ProposedChangeRecord>, SqliteStoreError> {
        self.connection
            .query_row(
                SELECT_PROPOSED_CHANGE,
                params![investigation_id, request_id],
                row_to_record,
            )
            .optional()
            .map_err(SqliteStoreError::from)
    }

    pub fn proposed_change(
        &self,
        investigation_id: &str,
        change_set_id: &str,
        version: Option<i64>,
    ) -> Result<Option<(ProposedChangeRecord, Vec<(i64, String)>)>, SqliteStoreError> {
        let selected = match version {
            Some(version) => self
                .connection
                .query_row(
                    SELECT_PROPOSED_CHANGE_BY_VERSION,
                    params![investigation_id, change_set_id, version],
                    row_to_record,
                )
                .optional()?,
            None => self
                .connection
                .query_row(
                    SELECT_PROPOSED_CHANGE_LATEST,
                    params![investigation_id, change_set_id],
                    row_to_record,
                )
                .optional()?,
        };
        let Some(record) = selected else {
            return Ok(None);
        };
        let mut versions = self.connection.prepare(
            "SELECT version, state
             FROM proposed_change_set_versions
             WHERE investigation_id = ?1 AND change_set_id = ?2
             ORDER BY version",
        )?;
        let versions = versions
            .query_map(params![investigation_id, change_set_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((record, versions)))
    }

    pub fn insert_proposed_change(
        &mut self,
        record: &ProposedChangeRecord,
        supersedes_version: Option<i64>,
    ) -> Result<(), SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, i64)> = tx
            .query_row(
                "SELECT investigation_id, active_version
                 FROM proposed_change_sets
                 WHERE change_set_id = ?1",
                params![record.change_set_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            None => {
                if record.version != 1 || supersedes_version.is_some() {
                    return Err(SqliteStoreError::InvalidSchema(
                        "new proposed change set must start at version 1",
                    ));
                }
                tx.execute(
                    "INSERT INTO proposed_change_sets
                     (change_set_id, investigation_id, active_version, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![
                        record.change_set_id,
                        record.investigation_id,
                        record.version,
                        record.created_at,
                    ],
                )?;
            }
            Some((investigation_id, active_version)) => {
                if investigation_id != record.investigation_id
                    || supersedes_version != Some(active_version)
                    || record.version != active_version + 1
                {
                    return Err(SqliteStoreError::InvalidSchema(
                        "proposed change set version is not the active successor",
                    ));
                }
                tx.execute(
                    "UPDATE proposed_change_sets
                     SET active_version = ?3, updated_at = ?4
                     WHERE change_set_id = ?1 AND investigation_id = ?2",
                    params![
                        record.change_set_id,
                        record.investigation_id,
                        record.version,
                        record.created_at,
                    ],
                )?;
                tx.execute(
                    "UPDATE proposed_change_set_versions
                     SET state = 'superseded'
                     WHERE change_set_id = ?1 AND version = ?2",
                    params![record.change_set_id, active_version],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO proposed_change_set_versions
             (change_set_id, version, investigation_id, round_id, generation,
              request_id, request_fingerprint, proposal_json, result_json,
              state, base_commit, created_at, proposed_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.change_set_id,
                record.version,
                record.investigation_id,
                record.round_id,
                record.generation,
                record.request_id,
                record.request_fingerprint,
                serde_json::to_string(&record.proposal)?,
                serde_json::to_string(&record.result)?,
                record.state,
                record.base_commit,
                record.created_at,
                record.proposed_by,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn insert_change_set_dependencies(
        &mut self,
        dependencies: &[ChangeSetDependency],
    ) -> Result<(), SqliteStoreError> {
        if dependencies.is_empty() {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for dependency in dependencies {
            tx.execute(
                "INSERT OR REPLACE INTO change_set_dependencies
                 (dependent_change_set_id, dependent_version, depends_on_change_set_id,
                  depends_on_version, investigation_id, referenced_ids_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    dependency.dependent_change_set_id,
                    dependency.dependent_version,
                    dependency.depends_on_change_set_id,
                    dependency.depends_on_version,
                    dependency.investigation_id,
                    serde_json::to_string(&dependency.referenced_ids)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Rebind dependents from a superseded version to its successor when the
    /// stable ids they reference remain available. The caller has already
    /// validated the new proposal; callers may then mark missing references
    /// blocked in the dependent result.
    pub fn retarget_dependencies(
        &mut self,
        investigation_id: &str,
        change_set_id: &str,
        old_version: i64,
        new_version: i64,
    ) -> Result<(), SqliteStoreError> {
        self.connection.execute(
            "UPDATE change_set_dependencies
             SET depends_on_version = ?4
             WHERE investigation_id = ?1 AND depends_on_change_set_id = ?2
               AND depends_on_version = ?3",
            params![investigation_id, change_set_id, old_version, new_version],
        )?;
        Ok(())
    }

    pub fn dependencies_for(
        &self,
        investigation_id: &str,
        change_set_id: &str,
        version: i64,
    ) -> Result<Vec<ChangeSetDependency>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT dependent_change_set_id, dependent_version,
                    depends_on_change_set_id, depends_on_version, investigation_id,
                    referenced_ids_json
             FROM change_set_dependencies
             WHERE investigation_id = ?1 AND dependent_change_set_id = ?2
               AND dependent_version = ?3
             ORDER BY depends_on_change_set_id, depends_on_version",
        )?;
        let rows = statement
            .query_map(params![investigation_id, change_set_id, version], |row| {
                dependency_from_row(row)
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn all_dependencies(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<ChangeSetDependency>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT dependent_change_set_id, dependent_version,
                    depends_on_change_set_id, depends_on_version, investigation_id,
                    referenced_ids_json
             FROM change_set_dependencies
             WHERE investigation_id = ?1
             ORDER BY dependent_change_set_id, dependent_version",
        )?;
        let rows = statement.query_map([investigation_id], dependency_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_proposed_state(
        &mut self,
        investigation_id: &str,
        change_set_id: &str,
        version: i64,
        state: &str,
        result: Option<&Value>,
    ) -> Result<(), SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = result {
            tx.execute(
                "UPDATE proposed_change_set_versions SET state = ?4, result_json = ?5
                 WHERE investigation_id = ?1 AND change_set_id = ?2 AND version = ?3",
                params![
                    investigation_id,
                    change_set_id,
                    version,
                    state,
                    serde_json::to_string(result)?,
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE proposed_change_set_versions SET state = ?4
                 WHERE investigation_id = ?1 AND change_set_id = ?2 AND version = ?3",
                params![investigation_id, change_set_id, version, state],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn record_change_set_decision(
        &mut self,
        decision: &ChangeSetDecision,
    ) -> Result<(), SqliteStoreError> {
        self.connection.execute(
            "INSERT INTO change_set_decisions
             (decision_id, change_set_id, version, investigation_id, kind, decided_by,
              plan_fingerprint, commit_id, receipt_json, batch_id, reason, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                decision.decision_id,
                decision.change_set_id,
                decision.version,
                decision.investigation_id,
                decision.kind,
                decision.decided_by,
                decision.plan_fingerprint,
                decision.commit_id,
                decision
                    .receipt
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                decision.batch_id,
                decision.reason,
                decision.decided_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_pending_change_sets(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<PendingChangeSet>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT v.change_set_id, v.version, v.investigation_id, v.round_id,
                    v.generation, v.request_id, v.request_fingerprint, v.proposal_json,
                    v.result_json, v.state, v.base_commit, v.created_at, v.proposed_by
             FROM proposed_change_set_versions v
             JOIN proposed_change_sets s ON s.change_set_id = v.change_set_id
             WHERE v.investigation_id = ?1 AND v.version = s.active_version
               AND v.state IN ('staged', 'blocked')
             ORDER BY v.created_at, v.change_set_id",
        )?;
        let rows = statement.query_map([investigation_id], row_to_record)?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|record| {
                let dependencies =
                    self.dependencies_for(investigation_id, &record.change_set_id, record.version)?;
                Ok(PendingChangeSet {
                    record,
                    dependencies,
                })
            })
            .collect()
    }
}

fn dependency_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeSetDependency> {
    let referenced: String = row.get(5)?;
    Ok(ChangeSetDependency {
        dependent_change_set_id: row.get(0)?,
        dependent_version: row.get(1)?,
        depends_on_change_set_id: row.get(2)?,
        depends_on_version: row.get(3)?,
        investigation_id: row.get(4)?,
        referenced_ids: serde_json::from_str(&referenced).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

const SELECT_PROPOSED_CHANGE: &str =
    "SELECT change_set_id, version, investigation_id, round_id, generation,
            request_id, request_fingerprint, proposal_json, result_json,
            state, base_commit, created_at, proposed_by
     FROM proposed_change_set_versions
     WHERE investigation_id = ?1 AND request_id = ?2";

const SELECT_PROPOSED_CHANGE_BY_VERSION: &str =
    "SELECT change_set_id, version, investigation_id, round_id, generation,
            request_id, request_fingerprint, proposal_json, result_json,
            state, base_commit, created_at, proposed_by
     FROM proposed_change_set_versions
     WHERE investigation_id = ?1 AND change_set_id = ?2 AND version = ?3";

const SELECT_PROPOSED_CHANGE_LATEST: &str =
    "SELECT v.change_set_id, v.version, v.investigation_id, v.round_id, v.generation,
            v.request_id, v.request_fingerprint, v.proposal_json, v.result_json,
            v.state, v.base_commit, v.created_at, v.proposed_by
     FROM proposed_change_set_versions v
     JOIN proposed_change_sets s ON s.change_set_id = v.change_set_id
     WHERE v.investigation_id = ?1 AND v.change_set_id = ?2
       AND v.version = s.active_version";

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProposedChangeRecord> {
    let proposal_json: String = row.get(7)?;
    let result_json: String = row.get(8)?;
    Ok(ProposedChangeRecord {
        change_set_id: row.get(0)?,
        version: row.get(1)?,
        investigation_id: row.get(2)?,
        round_id: row.get(3)?,
        generation: row.get(4)?,
        request_id: row.get(5)?,
        request_fingerprint: row.get(6)?,
        proposal: serde_json::from_str(&proposal_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        result: serde_json::from_str(&result_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        state: row.get(9)?,
        base_commit: row.get(10)?,
        created_at: row.get(11)?,
        proposed_by: row.get(12)?,
    })
}
