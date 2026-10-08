//! Adoption queries and atomic withdrawal/registration. No cascade policy in SQL.
use crate::{
    SourceRegistrationReceipt, SqliteArtifactStore, SqliteStoreError, map_edit, source_map,
};
use mathscape_domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

fn invalid(message: impl Into<String>) -> SqliteStoreError {
    SqliteStoreError::SourceRegistrationInvariant(message.into())
}
pub(crate) fn ensure_not_discarded(c: &Connection, decision: &str) -> Result<(), SqliteStoreError> {
    let discarded: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM discovery_run_dispositions x JOIN discovery_acceptance_decisions a ON a.run_id=x.run_id WHERE a.decision_id=?1)",[decision],|r|r.get(0))?;
    if discarded {
        return Err(invalid("run_discarded: 已放弃的结果不能登记"));
    }
    Ok(())
}
pub(crate) fn adoptions(c: &Connection) -> Result<Vec<RegistrationAdoption>, SqliteStoreError> {
    let mut q=c.prepare("SELECT c.decision_id,c.commit_id,a.run_id,c.committed_at,a.selection_json,c.receipt_json,x.receipt_json,a.plan_json FROM source_registration_commits c JOIN discovery_acceptance_decisions a ON a.decision_id=c.decision_id LEFT JOIN discovery_registration_reversals x ON x.decision_id=c.decision_id ORDER BY c.decision_id")?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = vec![];
    for (decision, commit, run, time, selection, receipt, reversal, plan) in rows {
        let receipt: SourceRegistrationReceipt = serde_json::from_str(&receipt)?;
        if receipt.decision_id.as_str() != decision || receipt.commit_id.as_str() != commit {
            return Err(invalid("registration_identity_conflict"));
        }
        let mut m=c.prepare("SELECT m.object_id,m.revision_id,m.outcome FROM map_commit_objects m JOIN map_object_revisions r ON r.object_id=m.object_id AND r.revision_id=m.revision_id WHERE m.commit_id=?1 ORDER BY m.object_id")?;
        let members = m
            .query_map([&commit], |r| {
                Ok(RegistrationMember {
                    object_id: r.get(0)?,
                    revision_id: r.get(1)?,
                    outcome: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let ids: std::collections::BTreeSet<_> = receipt
            .created_objects
            .iter()
            .chain(&receipt.reused_objects)
            .chain(&receipt.reactivated_objects)
            .chain(&receipt.revised_objects)
            .map(|id| id.to_string())
            .collect();
        if ids.len() != members.len() || members.iter().any(|m| !ids.contains(&m.object_id)) {
            return Err(invalid(
                "registration_members_missing: 登记成员或原 revision 缺失",
            ));
        }
        if receipt.created_objects.len()
            + receipt.reused_objects.len()
            + receipt.reactivated_objects.len()
            + receipt.revised_objects.len()
            != ids.len()
            || members.iter().any(|m| {
                let ids = match m.outcome.as_str() {
                    "created" => &receipt.created_objects,
                    "reused" => &receipt.reused_objects,
                    "reactivated" => &receipt.reactivated_objects,
                    "revised" => &receipt.revised_objects,
                    _ => return true,
                };
                !ids.iter().any(|id| id.as_str() == m.object_id)
            })
        {
            return Err(invalid(
                "registration_outcome_conflict: 回执与成员采纳结果不一致",
            ));
        }
        let selection: serde_json::Value = serde_json::from_str(&selection)?;
        let selected_unit_refs = if selection["kind"] == "all_validated" {
            serde_json::from_str::<serde_json::Value>(&plan)?["units"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u["source_ref"].as_str().map(str::to_owned))
                .collect()
        } else {
            selection["unit_refs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        let reversal_receipt = reversal
            .map(|v| serde_json::from_str::<DiscoveryReversalReceipt>(&v))
            .transpose()?;
        result.push(RegistrationAdoption {
            summary: RegistrationSummary {
                decision_id: decision,
                registration_commit_id: commit,
                run_id: run,
                state: if reversal_receipt.is_some() {
                    RegistrationState::Withdrawn
                } else {
                    RegistrationState::Registered
                },
                registered_at: time,
                selected_unit_refs,
                object_count: members.len() as u32,
                reversal_receipt,
            },
            members,
        });
    }
    Ok(result)
}
fn snapshot(c: &Connection) -> Result<RegistrationSnapshot, SqliteStoreError> {
    Ok(RegistrationSnapshot {
        map: map_edit::snapshot(c)?,
        registrations: adoptions(c)?,
    })
}
impl SqliteArtifactStore {
    pub fn pending_registration_decisions(
        &self,
        run: &str,
    ) -> Result<Vec<String>, SqliteStoreError> {
        let mut q=self.connection.prepare("SELECT a.decision_id FROM discovery_acceptance_decisions a LEFT JOIN source_registration_commits c ON c.decision_id=a.decision_id WHERE a.run_id=?1 AND c.decision_id IS NULL AND NOT EXISTS(SELECT 1 FROM discovery_run_dispositions x WHERE x.run_id=a.run_id) ORDER BY a.decision_id")?;
        Ok(q.query_map([run], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }
    pub fn registration_snapshot(&self) -> Result<RegistrationSnapshot, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let s = snapshot(&tx)?;
        tx.commit()?;
        Ok(s)
    }
    pub fn list_discovery_registrations(
        &self,
        run: &str,
    ) -> Result<Vec<RegistrationSummary>, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let rows = adoptions(&tx)?
            .into_iter()
            .filter(|r| r.summary.run_id == run)
            .map(|r| r.summary)
            .collect();
        tx.commit()?;
        Ok(rows)
    }
    pub fn discovery_registration_reversal(
        &self,
        decision: &str,
    ) -> Result<Option<DiscoveryReversalReceipt>, SqliteStoreError> {
        self.connection
            .query_row(
                "SELECT receipt_json FROM discovery_registration_reversals WHERE decision_id=?1",
                [decision],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn commit_registration_reversal<F>(
        &mut self,
        request: &ReverseDiscoveryRequest,
        expected: &str,
        actor: &ActorRef,
        time: Timestamp,
        planner: F,
    ) -> Result<DiscoveryReversalReceipt, SqliteStoreError>
    where
        F: FnOnce(
            &RegistrationSnapshot,
        ) -> (DiscoveryReversalPlan, Option<MapPatch<MapEditPayload>>),
    {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request_json = serde_json::to_string(&(request, actor))?;
        let old=tx.query_row("SELECT request_json,plan_fingerprint,receipt_json FROM discovery_registration_reversals WHERE request_id=?1",[&request.request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((stored, fp, receipt)) = old {
            if stored != request_json || fp != expected {
                return Err(invalid("idempotency_conflict"));
            }
            return Ok(serde_json::from_str(&receipt)?);
        }
        let already = tx
            .query_row(
                "SELECT receipt_json FROM discovery_registration_reversals WHERE decision_id=?1",
                [&request.decision_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if let Some(receipt) = already {
            return Err(invalid(format!("already_withdrawn: {receipt}")));
        }
        let s = snapshot(&tx)?;
        if s.map.base_commit != request.base_commit {
            return Err(invalid("base_commit_conflict: 地图已更新，请重新计算影响"));
        }
        let (plan, patch) = planner(&s);
        if serde_json::to_value(&plan.request)? != serde_json::to_value(request)? {
            return Err(invalid("planner_request_mismatch"));
        }
        if plan.request.request_id != request.request_id
            || plan.impact.status != ImpactPlanStatus::Ready
        {
            return Err(invalid(format!(
                "reversal_blocked: {}",
                serde_json::to_string(&plan.impact.diagnostics)?
            )));
        }
        if plan.impact.plan_fingerprint != expected {
            return Err(invalid(
                "plan_conflict: 采纳集合或地图已变化，请重新计算影响",
            ));
        }
        let mut retired = vec![];
        let mut preserved = vec![];
        let mut inactive = vec![];
        for i in &plan.impact.direct_changes {
            if i.action == "retire" {
                retired.push(i.object_id.clone());
            } else if i.reason_codes.iter().any(|c| c == "already_inactive") {
                inactive.push(i.object_id.clone());
            } else {
                preserved.push(i.object_id.clone());
            }
        }
        let map_commit = if let Some(p) = patch {
            let u = UserChangeRequest {
                request_id: request.request_id.clone(),
                base_commit: request.base_commit.clone(),
                reason: request.reason.clone(),
                operations: vec![],
            };
            Some(map_edit::write_patch(
                &tx, &s.map, &p, &u, expected, actor, &time,
            )?)
        } else {
            if !retired.is_empty() {
                return Err(invalid("missing retirement patch"));
            }
            None
        };
        let receipt = DiscoveryReversalReceipt {
            version: "mathscape-discovery-reversal-receipt-1".into(),
            request_id: request.request_id.clone(),
            decision_id: request.decision_id.clone(),
            registration_commit_id: request.registration_commit_id.clone(),
            plan_fingerprint: expected.into(),
            state: RegistrationState::Withdrawn,
            decided_at: time.to_string(),
            actor: actor.clone(),
            reason: request.reason.clone(),
            map_commit,
            retired_objects: retired,
            preserved_objects: preserved,
            already_inactive_objects: inactive,
        };
        if let Some(commit) = &receipt.map_commit {
            let actual = commit
                .objects
                .iter()
                .map(|o| o.object_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let expected = receipt
                .retired_objects
                .iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            if actual != expected || commit.objects.iter().any(|o| o.lifecycle != "deprecated") {
                return Err(invalid("retirement_patch_mismatch"));
            }
        }
        tx.execute("INSERT INTO discovery_registration_reversals(request_id,decision_id,registration_commit_id,base_commit,plan_fingerprint,request_json,map_commit_id,receipt_json,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![request.request_id,request.decision_id,request.registration_commit_id,request.base_commit,expected,request_json,receipt.map_commit.as_ref().map(|c|c.commit_id.as_str()),serde_json::to_string(&receipt)?,time.as_str()])?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn commit_bound_source_registration<F>(
        &mut self,
        request: &SourceRegistrationCommitRequest,
        expected: &str,
        actor: &ActorRef,
        time: Timestamp,
        planner: F,
    ) -> Result<SourceRegistrationReceipt, SqliteStoreError>
    where
        F: FnOnce(
            &RegistrationSnapshot,
        ) -> (
            SourceRegistrationCommitPlan,
            Option<MapPatch<MapEditPayload>>,
        ),
    {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut canonical_request = request.clone();
        canonical_request
            .replacement_approvals
            .sort_by(|a, b| a.object_id.cmp(&b.object_id));
        let request_json = serde_json::to_string(&(canonical_request, actor))?;
        if let Some((saved,fp,receipt))=tx.query_row("SELECT request_json,plan_fingerprint,receipt_json FROM source_registration_requests WHERE request_id=?1",[&request.request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()? {
            if saved!=request_json||fp!=expected {return Err(invalid("idempotency_conflict"));}return Ok(serde_json::from_str(&receipt)?);
        }
        ensure_not_discarded(&tx, &request.decision_id)?;
        if let Some(r) =
            source_map::source_registration_receipt(&tx, &StableId::parse(&request.decision_id)?)?
        {
            if r.plan_fingerprint.as_str() != request.acceptance_plan_fingerprint {
                return Err(invalid("acceptance_plan_conflict"));
            }
            return Ok(r);
        }
        let (fp,run): (String,String)=tx.query_row("SELECT plan_fingerprint,run_id FROM discovery_acceptance_decisions WHERE decision_id=?1",[&request.decision_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if fp != request.acceptance_plan_fingerprint {
            return Err(invalid("acceptance_plan_conflict"));
        }
        let s = snapshot(&tx)?;
        if s.map.base_commit != request.base_commit {
            return Err(invalid("base_commit_conflict"));
        }
        let (plan, patch) = planner(&s);
        if serde_json::to_value(&plan.request)? != serde_json::to_value(request)? {
            return Err(invalid("planner_request_mismatch"));
        }
        if plan.impact.status != ImpactPlanStatus::Ready {
            return Err(invalid(format!(
                "registration_blocked: {}",
                serde_json::to_string(&plan.impact.diagnostics)?
            )));
        }
        if plan.impact.plan_fingerprint != expected {
            return Err(invalid("plan_conflict"));
        }
        if let Some(p) = patch {
            let u = UserChangeRequest {
                request_id: request.request_id.clone(),
                base_commit: request.base_commit.clone(),
                reason: "登记已接受的来源发现结果".into(),
                operations: vec![],
            };
            let receipt = map_edit::write_patch(&tx, &s.map, &p, &u, expected, actor, &time)?;
            tx.execute(
                "UPDATE map_commits SET generation_run=?2 WHERE commit_id=?1",
                params![expected, run],
            )?;
            for o in receipt.objects.iter().filter(|o| {
                o.previous_revision.is_none()
                    || plan.objects.iter().any(|change| {
                        change.object_id == o.object_id && !change.changed_fields.is_empty()
                    })
            }) {
                tx.execute("UPDATE map_object_revisions SET generation_run=?3 WHERE object_id=?1 AND revision_id=?2",params![o.object_id,o.current_revision,run])?;
            }
        } else {
            if plan
                .objects
                .iter()
                .any(|o| o.action != RegistrationObjectAction::Reuse)
            {
                return Err(invalid("missing registration patch"));
            }
            tx.execute("INSERT INTO map_commits(commit_id,author,generation_run,message,committed_at) VALUES(?1,?2,?3,'记录来源登记采纳（全部复用）',?4)",params![expected,actor.as_str(),run,time.as_str()])?;
        }
        let mut receipt = SourceRegistrationReceipt {
            version: crate::SOURCE_REGISTRATION_RECEIPT_VERSION.into(),
            decision_id: StableId::parse(&request.decision_id)?,
            plan_fingerprint: ContentHash::parse(&fp)?,
            commit_id: CommitId::from_revision(RevisionId::parse(expected)?),
            created_objects: vec![],
            reused_objects: vec![],
            reactivated_objects: vec![],
            revised_objects: vec![],
            span_ids: vec![],
            unit_ids: vec![],
            relation_ids: vec![],
            committed_at: time.clone(),
        };
        for (n, o) in plan.objects.iter().enumerate() {
            let (kind, rev): (String, String) = tx.query_row(
                "SELECT entity_kind,current_revision FROM map_objects WHERE object_id=?1",
                [&o.object_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let id = StableId::parse(&o.object_id)?;
            match kind.as_str() {
                "SourceSpan" => receipt.span_ids.push(id.clone()),
                "SourceSemanticUnit" => receipt.unit_ids.push(id.clone()),
                "SourceUnitRelation" => receipt.relation_ids.push(id.clone()),
                _ => return Err(invalid("unsupported registration kind")),
            }
            let outcome = match o.action {
                RegistrationObjectAction::Create => {
                    receipt.created_objects.push(id);
                    "created"
                }
                RegistrationObjectAction::Reuse => {
                    receipt.reused_objects.push(id);
                    "reused"
                }
                RegistrationObjectAction::Reactivate => {
                    receipt.reactivated_objects.push(id);
                    "reactivated"
                }
                RegistrationObjectAction::Revise => {
                    receipt.revised_objects.push(id);
                    "revised"
                }
                RegistrationObjectAction::Block => {
                    return Err(invalid("blocked registration member"));
                }
            };
            tx.execute("INSERT INTO map_commit_objects(commit_id,ordinal,object_id,revision_id,outcome) VALUES(?1,?2,?3,?4,?5)",params![expected,n as i64+1,o.object_id,rev,outcome])?;
        }
        tx.execute("INSERT INTO source_registration_commits(decision_id,plan_fingerprint,commit_id,receipt_json,committed_at) VALUES(?1,?2,?3,?4,?5)",params![request.decision_id,fp,expected,serde_json::to_string(&receipt)?,time.as_str()])?;
        tx.execute("INSERT INTO source_registration_requests(request_id,decision_id,request_json,plan_fingerprint,receipt_json) VALUES(?1,?2,?3,?4,?5)",params![request.request_id,request.decision_id,request_json,expected,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Original registration revisions and their original endpoints, never the current head.
    pub fn registration_history(
        &self,
        decision: &str,
    ) -> Result<MapEditSnapshot, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let commit: String = tx.query_row(
            "SELECT commit_id FROM source_registration_commits WHERE decision_id=?1",
            [decision],
            |r| r.get(0),
        )?;
        let members = adoptions(&tx)?
            .into_iter()
            .find(|r| r.summary.decision_id == decision)
            .ok_or_else(|| invalid("registration_not_found"))?
            .members;
        let mut objects = vec![];
        for m in members {
            let (kind,life,payload):(String,String,String)=tx.query_row("SELECT o.entity_kind,r.lifecycle,r.payload_json FROM map_objects o JOIN map_object_revisions r ON r.object_id=o.object_id WHERE o.object_id=?1 AND r.revision_id=?2",params![m.object_id,m.revision_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            let mut q=tx.prepare("SELECT role,ordinal,entity_id FROM map_relation_endpoints WHERE relation_id=?1 AND revision_id=?2 ORDER BY role,ordinal_key")?;
            let endpoints = q
                .query_map(params![m.object_id, m.revision_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<u32>>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .map(|r| {
                    let (role, ordinal, id) = r?;
                    Ok(RelationEndpoint::new(
                        EndpointRole::parse(role)?,
                        StableId::parse(id)?,
                        ordinal,
                    ))
                })
                .collect::<Result<Vec<_>, SqliteStoreError>>()?;
            objects.push(MapEditObject {
                object_id: m.object_id,
                revision: m.revision_id,
                kind,
                lifecycle: life,
                payload: serde_json::from_str(&payload)?,
                endpoints,
            });
        }
        tx.commit()?;
        Ok(MapEditSnapshot {
            base_commit: Some(commit),
            objects,
            adopted_by: Default::default(),
        })
    }
}
