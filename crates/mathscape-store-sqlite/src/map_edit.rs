use crate::{SqliteArtifactStore, SqliteStoreError, source_map};
use mathscape_domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

fn invalid(message: impl Into<String>) -> SqliteStoreError {
    SqliteStoreError::SourceRegistrationInvariant(message.into())
}
pub(crate) fn snapshot(c: &Connection) -> Result<MapEditSnapshot, SqliteStoreError> {
    let source = source_map::source_map_snapshot(c)?;
    let base_commit = c
        .query_row(
            "SELECT commit_id FROM map_commits ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let objects = source
        .objects
        .into_iter()
        .map(|o| {
            let endpoints = source
                .relation_endpoints
                .get(&o.stable_id)
                .into_iter()
                .flatten()
                .map(|e| {
                    Ok(RelationEndpoint::new(
                        EndpointRole::parse(&e.role)?,
                        e.entity_id.clone(),
                        e.ordinal,
                    ))
                })
                .collect::<Result<Vec<_>, DomainError>>()?;
            Ok(MapEditObject {
                object_id: o.stable_id.to_string(),
                revision: o.revision_id.to_string(),
                kind: serde_json::to_value(o.kind)?.as_str().unwrap().into(),
                lifecycle: serde_json::to_value(o.lifecycle)?.as_str().unwrap().into(),
                payload: o.payload,
                endpoints,
            })
        })
        .collect::<Result<Vec<_>, SqliteStoreError>>()?;
    Ok(MapEditSnapshot {
        base_commit,
        objects,
        adopted_by: {
            let mut claims = std::collections::BTreeMap::<String, Vec<String>>::new();
            for r in super::registration::adoptions(c)?
                .into_iter()
                .filter(|r| r.summary.state == RegistrationState::Registered)
            {
                for m in r.members {
                    claims
                        .entry(m.object_id)
                        .or_default()
                        .push(r.summary.decision_id.clone());
                }
            }
            claims
        },
    })
}

impl SqliteArtifactStore {
    pub fn map_edit_snapshot(&self) -> Result<MapEditSnapshot, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let result = snapshot(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    /// Plan is rebuilt under the writer lock; storage does not depend on the
    /// orchestrator. Replay is checked before stale-head checks.
    pub fn commit_map_change<F>(
        &mut self,
        request: &UserChangeRequest,
        expected_plan: &str,
        actor: &ActorRef,
        time: Timestamp,
        plan: F,
    ) -> Result<CommitReceipt, SqliteStoreError>
    where
        F: FnOnce(&MapEditSnapshot) -> (ImpactPlan, Option<MapPatch<MapEditPayload>>),
    {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request_json = serde_json::to_string(&(request, actor))?;
        let old=tx.query_row("SELECT request_json,plan_fingerprint,receipt_json FROM map_change_requests WHERE request_id=?1",[&request.request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((stored, fp, receipt)) = old {
            if stored != request_json || fp != expected_plan {
                return Err(invalid("idempotency_conflict: request_id 已用于其他内容"));
            }
            return Ok(serde_json::from_str(&receipt)?);
        }
        let current = snapshot(&tx)?;
        if current.base_commit != request.base_commit {
            return Err(invalid("base_commit_conflict: 地图已更新，请重新预览"));
        }
        let (impact, patch) = plan(&current);
        if impact.status != ImpactPlanStatus::Ready {
            return Err(invalid(format!(
                "change_blocked: {}",
                serde_json::to_string(&impact.diagnostics)?
            )));
        }
        if impact.plan_fingerprint != expected_plan {
            return Err(invalid("plan_conflict: 影响预览已失效"));
        }
        let patch = patch.ok_or_else(|| invalid("missing patch"))?;
        patch.validate()?;
        if patch.author() != actor {
            return Err(invalid("actor mismatch"));
        }
        let receipt = write_patch(&tx, &current, &patch, request, expected_plan, actor, &time)?;
        tx.execute("INSERT INTO map_change_requests(request_id,request_json,plan_fingerprint,commit_id,receipt_json) VALUES(?1,?2,?3,?4,?5)",params![request.request_id,request_json,expected_plan,receipt.commit_id,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn map_object_revision(
        &self,
        object: &str,
        revision: &str,
    ) -> Result<Option<serde_json::Value>, SqliteStoreError> {
        let row=self.connection.query_row("SELECT payload_json,lifecycle,created_at,created_by,generation_run FROM map_object_revisions WHERE object_id=?1 AND revision_id=?2",params![object,revision],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?))).optional()?;
        row.map(|(p,l,t,a,g)|Ok(serde_json::json!({"object_id":object,"revision_id":revision,"payload":serde_json::from_str::<serde_json::Value>(&p)?,"lifecycle":l,"created_at":t,"created_by":a,"generation_run":g}))).transpose()
    }
}

/// Shared writer; caller owns the transaction and its business audit/receipt.
pub(crate) fn write_patch(
    tx: &rusqlite::Transaction<'_>,
    current: &MapEditSnapshot,
    patch: &MapPatch<MapEditPayload>,
    request: &UserChangeRequest,
    expected_plan: &str,
    actor: &ActorRef,
    time: &Timestamp,
) -> Result<CommitReceipt, SqliteStoreError> {
    patch.validate()?;
    if patch.author() != actor {
        return Err(invalid("actor mismatch"));
    }
    let commit_id = expected_plan.to_owned();
    RevisionId::parse(&commit_id)?;
    tx.execute("INSERT INTO map_commits(commit_id,author,generation_run,message,committed_at) VALUES(?1,?2,NULL,?3,?4)",params![commit_id,actor.as_str(),request.reason,time.as_str()])?;
    let mut entries = vec![];
    // Create all identity rows first; relation FK checks can then span the batch.
    for op in patch.operations() {
        let (h, relation, expected) = match op {
            MapOperation::CreateEntity { entity } => (entity.header(), false, None),
            MapOperation::CreateRelation { relation } => (relation.header(), true, None),
            MapOperation::ReviseEntity {
                replacement,
                expected_revision,
                ..
            } => (replacement.header(), false, Some(expected_revision)),
            MapOperation::ReviseRelation {
                replacement,
                expected_revision,
                ..
            } => (replacement.header(), true, Some(expected_revision)),
        };
        let existing = current
            .objects
            .iter()
            .find(|o| o.object_id == h.stable_id().as_str());
        if let Some(expected) = expected {
            if !existing.is_some_and(|o| o.revision == expected.as_str()) {
                return Err(invalid("revision_conflict"));
            }
            if existing.is_some_and(|o| {
                o.kind != serde_json::to_value(h.kind()).unwrap().as_str().unwrap()
                    || (o.kind == "SourceUnitRelation") != relation
            }) {
                return Err(invalid(
                    "object_identity_conflict: revision 不能改变对象类型或形状",
                ));
            }
        } else {
            if existing.is_some() {
                return Err(invalid("object_exists"));
            }
            tx.execute("INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision) VALUES(?1,?2,?3,?4)",params![h.stable_id().as_str(),if relation{"relation"}else{"entity"},serde_json::to_value(h.kind())?.as_str().unwrap(),h.revision_id().as_str()])?;
        }
    }
    for op in patch.operations() {
        let (h, p, endpoints, previous) = match op {
            MapOperation::CreateEntity { entity } => {
                (entity.header(), entity.payload(), &[][..], None)
            }
            MapOperation::CreateRelation { relation } => (
                relation.header(),
                relation.payload(),
                relation.endpoints(),
                None,
            ),
            MapOperation::ReviseEntity {
                replacement,
                expected_revision,
                ..
            } => (
                replacement.header(),
                replacement.payload(),
                &[][..],
                Some(expected_revision),
            ),
            MapOperation::ReviseRelation {
                replacement,
                expected_revision,
                ..
            } => (
                replacement.header(),
                replacement.payload(),
                replacement.endpoints(),
                Some(expected_revision),
            ),
        };
        let life = serde_json::to_value(h.lifecycle())?
            .as_str()
            .unwrap()
            .to_owned();
        let kind = serde_json::to_value(h.kind())?.as_str().unwrap().to_owned();
        if !matches!(
            kind.as_str(),
            "SourceSemanticUnit"
                | "SourceUnitRelation"
                | "SourceSpan"
                | "SemanticEntity"
                | "Alignment"
                | "Debt"
                | "DialectScope"
        ) {
            return Err(invalid("unsupported edit kind"));
        }
        // Keep the original generator as provenance; record human edit actor
        // and time on this revision and the commit separately.
        let generation: Option<String> = if let Some(previous) = previous {
            tx.query_row("SELECT generation_run FROM map_object_revisions WHERE object_id=?1 AND revision_id=?2",params![h.stable_id().as_str(),previous.as_str()],|r|r.get(0))?
        } else {
            None
        };
        tx.execute("INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,created_at,created_by,generation_run,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![h.stable_id().as_str(),h.revision_id().as_str(),h.schema_version().as_str(),life,time.as_str(),actor.as_str(),generation,serde_json::to_string(p)?])?;
        for e in endpoints {
            tx.execute("INSERT INTO map_relation_endpoints(relation_id,revision_id,role,ordinal_key,ordinal,entity_id) VALUES(?1,?2,?3,?4,?5,?6)",params![h.stable_id().as_str(),h.revision_id().as_str(),e.role().as_str(),e.ordinal().map(i64::from).unwrap_or(-1),e.ordinal(),e.entity().as_str()])?;
        }
        tx.execute(
            "UPDATE map_objects SET current_revision=?2 WHERE object_id=?1",
            params![h.stable_id().as_str(), h.revision_id().as_str()],
        )?;
        let action = if life == "deprecated" {
            "retire"
        } else if previous.is_none() {
            "create"
        } else {
            "revise"
        };
        tx.execute("INSERT INTO map_edit_operations(commit_id,object_id,previous_revision,current_revision,action,reason) VALUES(?1,?2,?3,?4,?5,?6)",params![commit_id,h.stable_id().as_str(),previous.map(RevisionId::as_str),h.revision_id().as_str(),action,request.reason])?;
        entries.push(ObjectRevisionReceipt {
            object_id: h.stable_id().to_string(),
            kind,
            previous_revision: previous.map(ToString::to_string),
            current_revision: h.revision_id().to_string(),
            lifecycle: life,
            action: action.into(),
        });
    }
    let receipt = CommitReceipt {
        version: "mathscape-map-commit-receipt-1".into(),
        request_id: request.request_id.clone(),
        plan_fingerprint: expected_plan.into(),
        base_commit: current.base_commit.clone(),
        commit_id: commit_id.clone(),
        committed_at: time.to_string(),
        objects: entries,
    };
    Ok(receipt)
}
