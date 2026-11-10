//! Persistence for Phase2 proposed change sets.
//!
//! These records are intentionally separate from `map_objects` and
//! `map_object_revisions`: a proposal is an auditable candidate and becomes
//! part of the accepted map only through [`SqliteArtifactStore::accept_change_sets`],
//! which writes the patch, the decision rows and the state change in one
//! `BEGIN IMMEDIATE` transaction.

use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{
    ActorRef, CommitReceipt, ImpactPlan, ImpactPlanStatus, MapEditPayload, MapEditSnapshot,
    MapPatch, Timestamp, UserChangeRequest,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::{SqliteArtifactStore, SqliteStoreError};

/// A version and the `(version, state)` list of its change set.
pub type VersionedRecord = (ProposedChangeRecord, Vec<(i64, String)>);

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

impl ProposedChangeRecord {
    pub fn proposal_fingerprint(&self) -> &str {
        self.result
            .get("proposal_fingerprint")
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

/// `dependent@version` references stable ids staged by `depends_on@version`.
/// Both versions are recorded; a dependency never follows a later version of
/// the change set it points to.
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

/// One change set version selected by the user, bound to the proposal
/// fingerprint the user saw.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSetSelectionRef {
    pub change_set_id: String,
    pub version: i64,
    pub proposal_fingerprint: String,
}

/// Everything an acceptance transaction needs besides the planner.
#[derive(Clone, Debug)]
pub struct ChangeSetAcceptance {
    pub investigation_id: String,
    /// Selected versions; for a batch, every selection gets its own decision
    /// row with the same commit id.
    pub selections: Vec<ChangeSetSelectionRef>,
    /// The combined request (topological order) previewed by the user. Its
    /// `base_commit` must equal the current head.
    pub request: UserChangeRequest,
    pub expected_plan: String,
    pub actor: ActorRef,
    pub time: Timestamp,
    pub batch_id: Option<String>,
    pub decided_at: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscardOutcome {
    /// Discarded versions: the selection plus, with cascade, every transitive dependent.
    pub discarded: Vec<(String, i64)>,
    /// Dependents turned `blocked` (without cascade).
    pub blocked: Vec<(String, i64)>,
}

fn conflict(message: impl Into<String>) -> SqliteStoreError {
    SqliteStoreError::ChangeSetConflict(message.into())
}

impl SqliteArtifactStore {
    /// Binds `round_id` to `investigation_id`, or verifies an existing
    /// binding. The first writer wins; a later attempt with another
    /// investigation is refused with `investigation_mismatch`.
    pub fn bind_round_investigation(
        &mut self,
        round_id: &str,
        investigation_id: &str,
        bound_by: &str,
        bound_at: &str,
    ) -> Result<(), SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT investigation_id FROM investigation_rounds WHERE round_id = ?1",
                [round_id],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(existing) if existing == investigation_id => {}
            Some(existing) => {
                return Err(conflict(format!(
                    "investigation_mismatch: round 已绑定到 {existing}，不能改绑 {investigation_id}"
                )));
            }
            None => {
                tx.execute(
                    "INSERT INTO investigation_rounds(round_id, investigation_id, bound_by, bound_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![round_id, investigation_id, bound_by, bound_at],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn round_investigation(&self, round_id: &str) -> Result<Option<String>, SqliteStoreError> {
        self.connection
            .query_row(
                "SELECT investigation_id FROM investigation_rounds WHERE round_id = ?1",
                [round_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

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
    ) -> Result<Option<VersionedRecord>, SqliteStoreError> {
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

    /// Inserts a new version and its dependency rows in one transaction.
    /// Superseding marks the old version `superseded` and turns every
    /// pending dependent of the old version (transitively) `blocked` with
    /// `dependency_superseded`; dependency rows keep the old version number.
    pub fn insert_proposed_change(
        &mut self,
        record: &ProposedChangeRecord,
        supersedes_version: Option<i64>,
        dependencies: &[ChangeSetDependency],
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
                let active_state: String = tx.query_row(
                    "SELECT state FROM proposed_change_set_versions
                     WHERE change_set_id = ?1 AND version = ?2",
                    params![record.change_set_id, active_version],
                    |row| row.get(0),
                )?;
                if investigation_id != record.investigation_id
                    || supersedes_version != Some(active_version)
                    || record.version != active_version + 1
                    || !matches!(active_state.as_str(), "staged" | "blocked")
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
        for dependency in dependencies {
            if dependency.dependent_change_set_id != record.change_set_id
                || dependency.dependent_version != record.version
                || dependency.investigation_id != record.investigation_id
            {
                return Err(SqliteStoreError::InvalidSchema(
                    "dependency row does not belong to the inserted version",
                ));
            }
            tx.execute(
                "INSERT INTO change_set_dependencies
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
        if let Some(old_version) = supersedes_version {
            block_dependents(
                &tx,
                &record.investigation_id,
                &[(record.change_set_id.clone(), old_version)],
                "dependency_superseded",
                &format!(
                    "依赖的 {}@{} 已被 v{} 替换；请用 supersedes 重新提议并引用新版本",
                    record.change_set_id, old_version, record.version
                ),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn dependencies_for(
        &self,
        investigation_id: &str,
        change_set_id: &str,
        version: i64,
    ) -> Result<Vec<ChangeSetDependency>, SqliteStoreError> {
        dependencies_of(&self.connection, investigation_id, change_set_id, version)
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

    /// Active `staged` / `blocked` versions of one investigation.
    pub fn list_pending_change_sets(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<PendingChangeSet>, SqliteStoreError> {
        list_versions(&self.connection, investigation_id, true)
    }

    /// Every version of one investigation in any state, with dependencies.
    pub fn list_change_set_versions(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<PendingChangeSet>, SqliteStoreError> {
        list_versions(&self.connection, investigation_id, false)
    }

    pub fn change_set_decisions(
        &self,
        investigation_id: &str,
    ) -> Result<Vec<ChangeSetDecision>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT decision_id, change_set_id, version, investigation_id, kind, decided_by,
                    plan_fingerprint, commit_id, receipt_json, batch_id, reason, decided_at
             FROM change_set_decisions WHERE investigation_id = ?1
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([investigation_id], |row| {
            let receipt: Option<String> = row.get(8)?;
            Ok(ChangeSetDecision {
                decision_id: row.get(0)?,
                change_set_id: row.get(1)?,
                version: row.get(2)?,
                investigation_id: row.get(3)?,
                kind: row.get(4)?,
                decided_by: row.get(5)?,
                plan_fingerprint: row.get(6)?,
                commit_id: row.get(7)?,
                receipt: receipt.and_then(|text| serde_json::from_str(&text).ok()),
                batch_id: row.get(9)?,
                reason: row.get(10)?,
                decided_at: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Accepts one or several staged versions as one map commit.
    ///
    /// In one `BEGIN IMMEDIATE` transaction: replays an identical request
    /// idempotently; re-checks every selection (active, `staged`, proposal
    /// fingerprint) and its dependencies (accepted or selected); re-reads
    /// the head and requires `request.base_commit` to equal it; re-runs the
    /// planner and requires the previewed plan fingerprint; writes the patch
    /// with the shared `write_patch`; records one decision per selection
    /// (same commit id) and sets each version `accepted`. Any mismatch rolls
    /// everything back.
    pub fn accept_change_sets<F>(
        &mut self,
        acceptance: &ChangeSetAcceptance,
        plan: F,
    ) -> Result<CommitReceipt, SqliteStoreError>
    where
        F: FnOnce(&MapEditSnapshot) -> (ImpactPlan, Option<MapPatch<MapEditPayload>>),
    {
        if acceptance.selections.is_empty() {
            return Err(conflict("empty_selection: 至少选择一个提案"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request = &acceptance.request;
        let request_json = serde_json::to_string(&(request, &acceptance.actor))?;
        let replay = tx
            .query_row(
                "SELECT request_json, plan_fingerprint, receipt_json
                 FROM map_change_requests WHERE request_id = ?1",
                [&request.request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((stored, fingerprint, receipt)) = replay {
            if stored != request_json || fingerprint != acceptance.expected_plan {
                return Err(conflict("idempotency_conflict: request_id 已用于其他内容"));
            }
            return Ok(serde_json::from_str(&receipt)?);
        }
        let selected: BTreeSet<(String, i64)> = acceptance
            .selections
            .iter()
            .map(|s| (s.change_set_id.clone(), s.version))
            .collect();
        let mut records = Vec::new();
        for selection in &acceptance.selections {
            let record = active_record(&tx, &acceptance.investigation_id, selection)?;
            if record.state != "staged" {
                return Err(conflict(format!(
                    "proposal_not_staged: {}@{} 当前为 {}，不能接受",
                    record.change_set_id, record.version, record.state
                )));
            }
            for dependency in dependencies_of(
                &tx,
                &acceptance.investigation_id,
                &record.change_set_id,
                record.version,
            )? {
                let key = (
                    dependency.depends_on_change_set_id.clone(),
                    dependency.depends_on_version,
                );
                if selected.contains(&key) {
                    continue;
                }
                let state = version_state(&tx, &key.0, key.1)?;
                if state.as_deref() != Some("accepted") {
                    return Err(conflict(format!(
                        "dependency_not_accepted: {}@{} 依赖的 {}@{} 尚未接受，也不在同一批次",
                        record.change_set_id, record.version, key.0, key.1
                    )));
                }
            }
            records.push(record);
        }
        let current = crate::map_edit::snapshot(&tx)?;
        if current.base_commit != request.base_commit {
            return Err(conflict("base_commit_conflict: 地图已更新，请重新预览"));
        }
        let (impact, patch) = plan(&current);
        if impact.status != ImpactPlanStatus::Ready {
            return Err(conflict(format!(
                "change_blocked: {}",
                serde_json::to_string(&impact.diagnostics)?
            )));
        }
        if impact.plan_fingerprint != acceptance.expected_plan {
            return Err(conflict("plan_conflict: 影响预览已失效，请重新预览"));
        }
        let patch = patch.ok_or_else(|| conflict("missing_patch: 计划没有可写入的补丁"))?;
        patch.validate()?;
        if patch.author() != &acceptance.actor {
            return Err(conflict("actor_mismatch: 补丁作者与接受者不一致"));
        }
        let receipt = crate::map_edit::write_patch(
            &tx,
            &current,
            &patch,
            request,
            &acceptance.expected_plan,
            &acceptance.actor,
            &acceptance.time,
        )?;
        tx.execute(
            "INSERT INTO map_change_requests(request_id, request_json, plan_fingerprint, commit_id, receipt_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request.request_id,
                request_json,
                acceptance.expected_plan,
                receipt.commit_id,
                serde_json::to_string(&receipt)?
            ],
        )?;
        let receipt_value = serde_json::to_value(&receipt)?;
        for record in records {
            let decision = json!({
                "kind": "accepted",
                "decided_by": acceptance.actor.as_str(),
                "plan_fingerprint": acceptance.expected_plan,
                "commit_id": receipt.commit_id,
                "batch_id": acceptance.batch_id,
                "receipt": receipt_value,
            });
            let mut result = record.result.clone();
            result["status"] = json!("accepted");
            result["decision"] = decision;
            set_state(&tx, &record, "accepted", &result)?;
            insert_decision(
                &tx,
                &ChangeSetDecision {
                    decision_id: decision_id(&record, "accepted", &receipt.commit_id),
                    change_set_id: record.change_set_id.clone(),
                    version: record.version,
                    investigation_id: acceptance.investigation_id.clone(),
                    kind: "accepted".into(),
                    decided_by: acceptance.actor.as_str().to_owned(),
                    plan_fingerprint: Some(acceptance.expected_plan.clone()),
                    commit_id: Some(receipt.commit_id.clone()),
                    receipt: Some(receipt_value.clone()),
                    batch_id: acceptance.batch_id.clone(),
                    reason: None,
                    decided_at: acceptance.decided_at.clone(),
                },
            )?;
        }
        tx.commit()?;
        Ok(receipt)
    }

    /// Discards active `staged` or `blocked` versions. Pending dependents are
    /// discarded too when `cascade` (transitively, each with its own decision
    /// row naming the cause), otherwise they turn `blocked` with
    /// `dependency_discarded` (and their own dependents with
    /// `dependency_blocked`). Never touches the map.
    pub fn discard_change_sets(
        &mut self,
        investigation_id: &str,
        selections: &[ChangeSetSelectionRef],
        cascade: bool,
        decided_by: &str,
        reason: Option<&str>,
        decided_at: &str,
    ) -> Result<DiscardOutcome, SqliteStoreError> {
        if selections.is_empty() {
            return Err(conflict("empty_selection: 至少选择一个提案"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut outcome = DiscardOutcome::default();
        let mut roots = Vec::new();
        for selection in selections {
            let record = active_record(&tx, investigation_id, selection)?;
            if !matches!(record.state.as_str(), "staged" | "blocked") {
                return Err(conflict(format!(
                    "proposal_not_pending: {}@{} 当前为 {}，不能丢弃",
                    record.change_set_id, record.version, record.state
                )));
            }
            discard_record(&tx, &record, decided_by, reason, decided_at)?;
            roots.push((record.change_set_id.clone(), record.version));
            outcome
                .discarded
                .push(roots.last().cloned().expect("pushed"));
        }
        if cascade {
            let mut frontier = roots;
            while let Some(cause) = frontier.pop() {
                for dependent in pending_dependents(&tx, investigation_id, &cause)? {
                    let reason = format!("连带丢弃：依赖的 {}@{} 已丢弃", cause.0, cause.1);
                    discard_record(&tx, &dependent, decided_by, Some(&reason), decided_at)?;
                    let key = (dependent.change_set_id.clone(), dependent.version);
                    outcome.discarded.push(key.clone());
                    frontier.push(key);
                }
            }
        } else {
            let names = roots
                .iter()
                .map(|(id, v)| format!("{id}@{v}"))
                .collect::<Vec<_>>()
                .join("、");
            outcome.blocked = block_dependents(
                &tx,
                investigation_id,
                &roots,
                "dependency_discarded",
                &format!("依赖的 {names} 已被丢弃"),
            )?;
        }
        tx.commit()?;
        Ok(outcome)
    }
}

fn active_record(
    tx: &Transaction<'_>,
    investigation_id: &str,
    selection: &ChangeSetSelectionRef,
) -> Result<ProposedChangeRecord, SqliteStoreError> {
    let record = tx
        .query_row(
            SELECT_PROPOSED_CHANGE_LATEST,
            params![investigation_id, selection.change_set_id],
            row_to_record,
        )
        .optional()?
        .ok_or_else(|| {
            conflict(format!(
                "proposal_not_found: {} 不存在或不属于当前调查",
                selection.change_set_id
            ))
        })?;
    if record.version != selection.version {
        return Err(conflict(format!(
            "proposal_stale: {} 的活动版本是 v{}，不是 v{}；请刷新",
            record.change_set_id, record.version, selection.version
        )));
    }
    if record.proposal_fingerprint() != selection.proposal_fingerprint {
        return Err(conflict(format!(
            "proposal_fingerprint_mismatch: {}@{} 的提案指纹已变化；请刷新",
            record.change_set_id, record.version
        )));
    }
    Ok(record)
}

fn version_state(
    tx: &Transaction<'_>,
    change_set_id: &str,
    version: i64,
) -> Result<Option<String>, SqliteStoreError> {
    tx.query_row(
        "SELECT state FROM proposed_change_set_versions WHERE change_set_id = ?1 AND version = ?2",
        params![change_set_id, version],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn set_state(
    tx: &Transaction<'_>,
    record: &ProposedChangeRecord,
    state: &str,
    result: &Value,
) -> Result<(), SqliteStoreError> {
    tx.execute(
        "UPDATE proposed_change_set_versions SET state = ?3, result_json = ?4
         WHERE change_set_id = ?1 AND version = ?2",
        params![
            record.change_set_id,
            record.version,
            state,
            serde_json::to_string(result)?
        ],
    )?;
    Ok(())
}

fn insert_decision(
    tx: &Transaction<'_>,
    decision: &ChangeSetDecision,
) -> Result<(), SqliteStoreError> {
    tx.execute(
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

fn decision_id(record: &ProposedChangeRecord, kind: &str, salt: &str) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&(
        "change-set-decision-1",
        &record.change_set_id,
        record.version,
        kind,
        salt,
    ))
    .expect("serializable decision identity");
    format!("mathscape:change-set-decision/{:x}", Sha256::digest(bytes))
}

fn discard_record(
    tx: &Transaction<'_>,
    record: &ProposedChangeRecord,
    decided_by: &str,
    reason: Option<&str>,
    decided_at: &str,
) -> Result<(), SqliteStoreError> {
    let mut result = record.result.clone();
    result["status"] = json!("discarded");
    result["decision"] = json!({
        "kind": "discarded",
        "decided_by": decided_by,
        "reason": reason,
    });
    set_state(tx, record, "discarded", &result)?;
    insert_decision(
        tx,
        &ChangeSetDecision {
            decision_id: decision_id(record, "discarded", decided_at),
            change_set_id: record.change_set_id.clone(),
            version: record.version,
            investigation_id: record.investigation_id.clone(),
            kind: "discarded".into(),
            decided_by: decided_by.to_owned(),
            plan_fingerprint: None,
            commit_id: None,
            receipt: None,
            batch_id: None,
            reason: reason.map(str::to_owned),
            decided_at: decided_at.to_owned(),
        },
    )
}

/// Active staged/blocked versions that directly depend on `cause`.
fn pending_dependents(
    tx: &Transaction<'_>,
    investigation_id: &str,
    cause: &(String, i64),
) -> Result<Vec<ProposedChangeRecord>, SqliteStoreError> {
    let mut statement = tx.prepare(
        "SELECT v.change_set_id, v.version, v.investigation_id, v.round_id,
                v.generation, v.request_id, v.request_fingerprint, v.proposal_json,
                v.result_json, v.state, v.base_commit, v.created_at, v.proposed_by
         FROM change_set_dependencies d
         JOIN proposed_change_set_versions v
           ON v.change_set_id = d.dependent_change_set_id AND v.version = d.dependent_version
         JOIN proposed_change_sets s ON s.change_set_id = v.change_set_id
         WHERE d.investigation_id = ?1 AND d.depends_on_change_set_id = ?2
           AND d.depends_on_version = ?3 AND v.version = s.active_version
           AND v.state IN ('staged', 'blocked')
         ORDER BY v.created_at, v.change_set_id",
    )?;
    let rows = statement.query_map(params![investigation_id, cause.0, cause.1], row_to_record)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Turns every pending transitive dependent of `roots` `blocked`. Direct
/// dependents get `code`; deeper ones get `dependency_blocked`. Each gets a
/// validation debt and an error diagnostic; content and fingerprint are kept.
fn block_dependents(
    tx: &Transaction<'_>,
    investigation_id: &str,
    roots: &[(String, i64)],
    code: &str,
    message: &str,
) -> Result<Vec<(String, i64)>, SqliteStoreError> {
    let mut blocked = Vec::new();
    let mut seen = BTreeSet::<(String, i64)>::new();
    let mut frontier: Vec<((String, i64), String, String)> = roots
        .iter()
        .map(|root| (root.clone(), code.to_owned(), message.to_owned()))
        .collect();
    while let Some((cause, code, message)) = frontier.pop() {
        for dependent in pending_dependents(tx, investigation_id, &cause)? {
            let key = (dependent.change_set_id.clone(), dependent.version);
            if !seen.insert(key.clone()) {
                continue;
            }
            let mut result = dependent.result.clone();
            result["status"] = json!("blocked");
            let item = json!({
                "debt_id": format!("validation:{code}:{}@{}", cause.0, cause.1),
                "kind": "validation",
                "code": code,
                "local_ref": Value::Null,
                "target_id": cause.0,
                "message": message,
            });
            match result.get_mut("debts").and_then(Value::as_array_mut) {
                Some(debts) => debts.push(item),
                None => result["debts"] = json!([item]),
            }
            let diagnostic = json!({
                "level": "error", "code": code, "message": message, "local_ref": Value::Null
            });
            match result.get_mut("diagnostics").and_then(Value::as_array_mut) {
                Some(items) => items.push(diagnostic),
                None => result["diagnostics"] = json!([diagnostic]),
            }
            set_state(tx, &dependent, "blocked", &result)?;
            blocked.push(key.clone());
            frontier.push((
                key.clone(),
                "dependency_blocked".to_owned(),
                format!("依赖的 {}@{} 已转为 blocked", key.0, key.1),
            ));
        }
    }
    Ok(blocked)
}

fn dependencies_of(
    connection: &rusqlite::Connection,
    investigation_id: &str,
    change_set_id: &str,
    version: i64,
) -> Result<Vec<ChangeSetDependency>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT dependent_change_set_id, dependent_version,
                depends_on_change_set_id, depends_on_version, investigation_id,
                referenced_ids_json
         FROM change_set_dependencies
         WHERE investigation_id = ?1 AND dependent_change_set_id = ?2
           AND dependent_version = ?3
         ORDER BY depends_on_change_set_id, depends_on_version",
    )?;
    let rows = statement.query_map(params![investigation_id, change_set_id, version], |row| {
        dependency_from_row(row)
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn list_versions(
    connection: &rusqlite::Connection,
    investigation_id: &str,
    pending_only: bool,
) -> Result<Vec<PendingChangeSet>, SqliteStoreError> {
    let sql = if pending_only {
        "SELECT v.change_set_id, v.version, v.investigation_id, v.round_id,
                v.generation, v.request_id, v.request_fingerprint, v.proposal_json,
                v.result_json, v.state, v.base_commit, v.created_at, v.proposed_by
         FROM proposed_change_set_versions v
         JOIN proposed_change_sets s ON s.change_set_id = v.change_set_id
         WHERE v.investigation_id = ?1 AND v.version = s.active_version
           AND v.state IN ('staged', 'blocked')
         ORDER BY v.created_at, v.change_set_id"
    } else {
        "SELECT change_set_id, version, investigation_id, round_id,
                generation, request_id, request_fingerprint, proposal_json,
                result_json, state, base_commit, created_at, proposed_by
         FROM proposed_change_set_versions
         WHERE investigation_id = ?1
         ORDER BY created_at, change_set_id, version"
    };
    let mut statement = connection.prepare(sql)?;
    let records = statement
        .query_map([investigation_id], row_to_record)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_dependent = BTreeMap::<(String, i64), Vec<ChangeSetDependency>>::new();
    let mut statement = connection.prepare(
        "SELECT dependent_change_set_id, dependent_version,
                depends_on_change_set_id, depends_on_version, investigation_id,
                referenced_ids_json
         FROM change_set_dependencies WHERE investigation_id = ?1
         ORDER BY depends_on_change_set_id, depends_on_version",
    )?;
    for dependency in statement.query_map([investigation_id], dependency_from_row)? {
        let dependency = dependency?;
        by_dependent
            .entry((
                dependency.dependent_change_set_id.clone(),
                dependency.dependent_version,
            ))
            .or_default()
            .push(dependency);
    }
    Ok(records
        .into_iter()
        .map(|record| PendingChangeSet {
            dependencies: by_dependent
                .remove(&(record.change_set_id.clone(), record.version))
                .unwrap_or_default(),
            record,
        })
        .collect())
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
