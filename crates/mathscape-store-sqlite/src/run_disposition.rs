//! Append-only user decisions, with a small current-state pointer and CAS.
use crate::{
    DiscoveryCheckpointStatus, DiscoveryRunCheckpoint, DiscoveryRunDispositionReceipt,
    SqliteArtifactStore, SqliteStoreError, discovery,
};
use mathscape_domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
fn invalid(message: &str) -> SqliteStoreError {
    SqliteStoreError::DiscoveryInvariant(message.into())
}

fn read_event(
    c: &Connection,
    request: &StableId,
) -> Result<Option<(DiscoveryRunDispositionReceipt, ContentHash)>, SqliteStoreError> {
    c.query_row("SELECT run_id,disposition,actor,reason,decided_at,previous_request_id,result_fingerprint FROM discovery_run_disposition_events WHERE request_id=?1",[request.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?))).optional()?.map(|(run,state,actor,reason,time,previous,fp)|Ok((DiscoveryRunDispositionReceipt{run_id:StableId::parse(run)?,disposition:state,request_id:request.clone(),actor:ActorRef::parse(actor)?,reason,decided_at:Timestamp::parse(time)?,previous_request_id:previous.map(StableId::parse).transpose()?},ContentHash::parse(fp)?))).transpose()
}
pub(crate) fn current_disposition(
    c: &Connection,
    run: &StableId,
) -> Result<Option<DiscoveryRunDispositionReceipt>, SqliteStoreError> {
    let request = c
        .query_row(
            "SELECT request_id FROM discovery_run_dispositions WHERE run_id=?1",
            [run.as_str()],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    request
        .map(|id| {
            read_event(c, &StableId::parse(id)?)?
                .map(|(r, _)| r)
                .ok_or_else(|| invalid("disposition_event_missing"))
        })
        .transpose()
}
fn replay(
    c: &Connection,
    request: &DiscoveryDispositionRequest,
    target: &str,
    actor: &ActorRef,
) -> Result<Option<DiscoveryRunDispositionReceipt>, SqliteStoreError> {
    if let Some((receipt, fp)) = read_event(c, &request.request_id)? {
        if receipt.run_id != request.run_id
            || receipt.disposition != target
            || receipt.actor != *actor
            || receipt.reason != request.reason
            || receipt.previous_request_id != request.expected_disposition_request_id
            || fp != request.expected_result_fingerprint
        {
            return Err(invalid("idempotency_conflict: request_id 已用于其他决定"));
        }
        return Ok(Some(receipt));
    }
    Ok(None)
}
pub(crate) fn change_disposition(
    c: &mut Connection,
    request: &DiscoveryDispositionRequest,
    target: &str,
    actor: &ActorRef,
    time: &Timestamp,
    validated_checkpoint: Option<&DiscoveryRunCheckpoint>,
) -> Result<DiscoveryRunDispositionReceipt, SqliteStoreError> {
    if request.reason.trim().is_empty() {
        return Err(invalid("变更原因不能为空"));
    }
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // A successful old request remains replayable even after a later cycle.
    if let Some(receipt) = replay(&tx, request, target, actor)? {
        return Ok(receipt);
    }
    let current = current_disposition(&tx, &request.run_id)?;
    if current.as_ref().map(|r| &r.request_id) != request.expected_disposition_request_id.as_ref() {
        return Err(invalid("disposition_conflict: 用户决定已变化，请刷新记录"));
    }
    let registered:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM source_registration_commits c JOIN discovery_acceptance_decisions a ON a.decision_id=c.decision_id WHERE a.run_id=?1)",[request.run_id.as_str()],|r|r.get(0))?;
    if registered {
        return Err(invalid(
            "registered_run: 已有登记，不能放弃或恢复未登记结果",
        ));
    }
    let stored = discovery::read_discovery_checkpoint(&tx, &request.run_id)?
        .ok_or_else(|| invalid("discovery run does not exist"))?;
    if stored.checkpoint.result_fingerprint.as_ref() != Some(&request.expected_result_fingerprint) {
        return Err(invalid("result_conflict: 内容发现结果已变化"));
    }
    if matches!(
        stored.checkpoint.status,
        DiscoveryCheckpointStatus::Running | DiscoveryCheckpointStatus::AwaitingPatchReview
    ) {
        return Err(invalid("运行仍在进行或等待修订审核"));
    }
    if current.as_ref().is_some_and(|r| r.disposition == target) {
        return Ok(current.unwrap());
    }
    if target == "restored" {
        if !current
            .as_ref()
            .is_some_and(|r| r.disposition == "discarded")
        {
            return Err(invalid("not_discarded: 当前结果未放弃"));
        }
        if validated_checkpoint != Some(&stored.checkpoint) {
            return Err(invalid("checkpoint_conflict: 检查点已变化，请重新验证"));
        }
    }
    let receipt = DiscoveryRunDispositionReceipt {
        run_id: request.run_id.clone(),
        disposition: target.into(),
        request_id: request.request_id.clone(),
        actor: actor.clone(),
        reason: request.reason.clone(),
        decided_at: time.clone(),
        previous_request_id: request.expected_disposition_request_id.clone(),
    };
    tx.execute("INSERT INTO discovery_run_disposition_events(request_id,run_id,disposition,previous_request_id,result_fingerprint,actor,reason,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![request.request_id.as_str(),request.run_id.as_str(),target,request.expected_disposition_request_id.as_ref().map(StableId::as_str),request.expected_result_fingerprint.as_str(),actor.as_str(),request.reason,time.as_str()])?;
    tx.execute("INSERT INTO discovery_run_dispositions(run_id,disposition,reason,actor,request_id,decided_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(run_id) DO UPDATE SET disposition=excluded.disposition,reason=excluded.reason,actor=excluded.actor,request_id=excluded.request_id,decided_at=excluded.decided_at",params![request.run_id.as_str(),target,request.reason,actor.as_str(),request.request_id.as_str(),time.as_str()])?;
    tx.commit()?;
    Ok(receipt)
}
impl SqliteArtifactStore {
    pub fn discovery_run_disposition(
        &self,
        run: &StableId,
    ) -> Result<Option<DiscoveryRunDispositionReceipt>, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let receipt = current_disposition(&tx, run)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn discovery_disposition_history(
        &self,
        run: &StableId,
    ) -> Result<Vec<DiscoveryRunDispositionReceipt>, SqliteStoreError> {
        let mut q=self.connection.prepare("SELECT request_id FROM discovery_run_disposition_events WHERE run_id=?1 ORDER BY rowid")?;
        let ids = q
            .query_map([run.as_str()], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                read_event(&self.connection, &StableId::parse(id)?)?
                    .map(|(r, _)| r)
                    .ok_or_else(|| invalid("disposition_event_missing"))
            })
            .collect()
    }
    pub fn replay_discovery_restore(
        &self,
        request: &DiscoveryDispositionRequest,
        actor: &ActorRef,
    ) -> Result<Option<DiscoveryRunDispositionReceipt>, SqliteStoreError> {
        replay(&self.connection, request, "restored", actor)
    }
    pub fn discard_discovery_run_checked(
        &mut self,
        request: &DiscoveryDispositionRequest,
        actor: &ActorRef,
        time: &Timestamp,
    ) -> Result<DiscoveryRunDispositionReceipt, SqliteStoreError> {
        change_disposition(
            &mut self.connection,
            request,
            "discarded",
            actor,
            time,
            None,
        )
    }
    pub fn restore_discovery_review(
        &mut self,
        request: &DiscoveryDispositionRequest,
        actor: &ActorRef,
        time: &Timestamp,
        validated_checkpoint: &DiscoveryRunCheckpoint,
    ) -> Result<DiscoveryRunDispositionReceipt, SqliteStoreError> {
        change_disposition(
            &mut self.connection,
            request,
            "restored",
            actor,
            time,
            Some(validated_checkpoint),
        )
    }
}
