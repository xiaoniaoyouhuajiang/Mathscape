use mathscape_domain::{
    ActorRef, InkLocator, InkSnapshotManifest, LifecycleStatus, MapOperation, MapPatch,
    RegisterInkEvidenceRequest, SourcePayload, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MapObjectOutcome, SqliteArtifactStore, SqliteStoreError, StoredInkSnapshot, ink, source_map,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InkEvidenceReceipt {
    pub version: String,
    pub request_id: String,
    pub snapshot_id: String,
    pub artifact_id: String,
    pub span_id: String,
    pub page_id: String,
    pub stroke_ids: Vec<String>,
    pub selection_mode: mathscape_domain::InkSelectionMode,
    pub revision_id: String,
    pub commit_id: String,
    pub outcome: MapObjectOutcome,
    pub committed_at: Timestamp,
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name: Option<String> = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=15",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_evidence_p2") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 15 is absent or has an unexpected name",
        ));
    }
    connection.prepare("SELECT request_id,document_id,through_seq,manifest_json,created_at,snapshot_id FROM ink_snapshot_requests LIMIT 0")?;
    connection.prepare("SELECT request_id,request_fingerprint,snapshot_id,span_id,commit_id,receipt_json FROM ink_evidence_requests LIMIT 0")?;
    Ok(())
}

fn fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn invalid(message: &str) -> SqliteStoreError {
    SqliteStoreError::InkInvariant(message.into())
}

impl SqliteArtifactStore {
    /// The managed Artifact is verified by the application service before this
    /// call. Rebind its immutable index and rebuild a bounded patch under the
    /// writer lock; object, map commit and idempotent receipt commit together.
    pub fn register_ink_evidence<F>(
        &mut self,
        request: &RegisterInkEvidenceRequest,
        actor: &ActorRef,
        time: Timestamp,
        plan: F,
    ) -> Result<InkEvidenceReceipt, SqliteStoreError>
    where
        F: FnOnce(&StoredInkSnapshot) -> Result<MapPatch<SourcePayload>, String>,
    {
        let request = request
            .canonicalized()
            .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
        let request_fingerprint = fingerprint(&serde_json::to_vec(&(&request, actor))?);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String,String)> = tx.query_row("SELECT request_fingerprint,receipt_json FROM ink_evidence_requests WHERE request_id=?1", [&request.request_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((stored, receipt)) = old {
            if stored != request_fingerprint {
                return Err(SqliteStoreError::InkRequestConflict(request.request_id));
            }
            let receipt = serde_json::from_str(&receipt)?;
            tx.commit()?;
            return Ok(receipt);
        }
        let snapshot = ink::read_snapshot(&tx, &request.snapshot_id)?
            .ok_or_else(|| invalid("evidence snapshot does not exist"))?;
        // Snapshots of a trashed document stay readable, but they do not take
        // new evidence until the document is restored (D8).
        crate::ink_note::require_active(&tx, &snapshot.document_id)?;
        if snapshot.artifact_id != request.expected_artifact_id.as_str()
            || snapshot.content_hash != request.expected_content_hash.as_str()
        {
            return Err(invalid("evidence snapshot identity conflict"));
        }
        let manifest: InkSnapshotManifest = serde_json::from_str(&snapshot.manifest_json)?;
        manifest
            .validate_selection(&request.page_id, &request.stroke_ids)
            .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
        if manifest.document_id != snapshot.document_id
            || u64::from(manifest.through_document_seq) != snapshot.through_seq
            || fingerprint(snapshot.manifest_json.as_bytes()) != snapshot.content_hash
        {
            return Err(invalid("evidence manifest does not match its snapshot"));
        }
        let (hash,size,media): (String,i64,String) = tx.query_row(
            "SELECT a.content_hash,a.byte_size,r.media_type FROM artifacts a JOIN artifact_revisions r ON a.artifact_id=r.artifact_id AND a.current_revision=r.revision_id WHERE a.artifact_id=?1",
            [&snapshot.artifact_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        )?;
        if hash != snapshot.content_hash
            || u64::try_from(size).ok() != Some(snapshot.byte_size)
            || snapshot.manifest_json.len() as u64 != snapshot.byte_size
            || !matches!(
                media.as_str(),
                "application/json" | "application/vnd.mathscape.ink-snapshot+json"
            )
        {
            return Err(invalid("evidence Artifact catalog binding is inconsistent"));
        }
        let patch = plan(&snapshot).map_err(SqliteStoreError::InkValidation)?;
        patch.validate()?;
        if patch.author() != actor || patch.generation_run().is_some() {
            return Err(invalid("evidence author or generation_run mismatch"));
        }
        let [MapOperation::CreateEntity { entity }] = patch.operations() else {
            return Err(invalid("Ink evidence accepts exactly one new SourceSpan"));
        };
        let SourcePayload::Span(span) = entity.payload() else {
            return Err(invalid(
                "Ink evidence cannot register a semantic interpretation",
            ));
        };
        let expected_locator = InkLocator::new(
            request.expected_artifact_id.clone(),
            request.page_id.clone(),
            request.stroke_ids.clone(),
            request.selection_mode,
        )?;
        if span.ink() != Some(&expected_locator)
            || span.markdown().is_some()
            || span.page().is_some()
        {
            return Err(invalid("evidence plan changed the authorized selection"));
        }
        let payload_hash = fingerprint(&serde_json::to_vec(entity.payload())?);
        let expected_id = format!("mathscape:ink-evidence/{}", &payload_hash[7..]);
        if entity.header().stable_id().as_str() != expected_id
            || entity.header().revision_id().as_str() != payload_hash
            || entity.header().lifecycle() != LifecycleStatus::Active
            || entity.header().created_by() != actor
            || entity.header().generation_run().is_some()
        {
            return Err(invalid("evidence identity or lifecycle mismatch"));
        }
        let outcome =
            source_map::persist_object(&tx, entity.header(), entity.payload(), false, &[])?;
        let revision_id: String = tx.query_row(
            "SELECT current_revision FROM map_objects WHERE object_id=?1",
            [entity.header().stable_id().as_str()],
            |row| row.get(0),
        )?;
        // Domain-separated from payload revisions and other map workflows.
        let commit_id = fingerprint(&serde_json::to_vec(&(
            "mathscape-ink-evidence-commit-1",
            &request_fingerprint,
        ))?);
        tx.execute("INSERT INTO map_commits(commit_id,author,generation_run,message,committed_at) VALUES(?1,?2,NULL,?3,?4)",params![commit_id,actor.as_str(),patch.message(),time.as_str()])?;
        tx.execute("INSERT INTO map_commit_objects(commit_id,ordinal,object_id,revision_id,outcome) VALUES(?1,1,?2,?3,?4)",params![commit_id,entity.header().stable_id().as_str(),revision_id,match outcome { MapObjectOutcome::Created => "created", MapObjectOutcome::Reused => "reused" }])?;
        let receipt = InkEvidenceReceipt {
            version: "mathscape-ink-evidence-receipt-1".into(),
            request_id: request.request_id.clone(),
            snapshot_id: snapshot.snapshot_id,
            artifact_id: snapshot.artifact_id,
            span_id: entity.header().stable_id().to_string(),
            page_id: request.page_id,
            stroke_ids: request.stroke_ids,
            selection_mode: request.selection_mode,
            revision_id,
            commit_id,
            outcome,
            committed_at: time,
        };
        tx.execute("INSERT INTO ink_evidence_requests(request_id,request_fingerprint,snapshot_id,span_id,commit_id,receipt_json) VALUES(?1,?2,?3,?4,?5,?6)",params![receipt.request_id,request_fingerprint,receipt.snapshot_id,receipt.span_id,receipt.commit_id,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn list_ink_evidence(
        &self,
        snapshot_id: &str,
    ) -> Result<Vec<InkEvidenceReceipt>, SqliteStoreError> {
        let mut statement = self.connection.prepare("SELECT receipt_json FROM ink_evidence_requests WHERE snapshot_id=?1 ORDER BY rowid DESC")?;
        let jsons = statement
            .query_map([snapshot_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen = std::collections::BTreeSet::new();
        let mut receipts = Vec::new();
        for json in jsons {
            let receipt: InkEvidenceReceipt = serde_json::from_str(&json)?;
            if seen.insert(receipt.span_id.clone()) {
                receipts.push(receipt);
            }
        }
        Ok(receipts)
    }
}
