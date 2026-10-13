//! User-selected Ink evidence; this workflow creates no discovery run or
//! mathematical interpretation. The caller rebinds verified managed bytes.
use mathscape_domain::{
    ActorRef, EntityEnvelope, EntityHeader, EntityKind, InkLocator, InkSnapshotManifest,
    LayoutKind, LifecycleStatus, MapOperation, MapPatch, RegisterInkEvidenceRequest, RevisionId,
    SchemaVersion, SourcePayload, SourceSpan, StableId, Timestamp,
};
use sha2::{Digest, Sha256};

pub fn plan_ink_evidence(
    manifest: &InkSnapshotManifest,
    request: &RegisterInkEvidenceRequest,
    actor: &ActorRef,
    time: Timestamp,
) -> Result<MapPatch<SourcePayload>, String> {
    let request = request.canonicalized().map_err(|error| error.to_string())?;
    manifest
        .validate_selection(&request.page_id, &request.stroke_ids)
        .map_err(|error| error.to_string())?;
    let locator = InkLocator::new(
        request.expected_artifact_id,
        request.page_id,
        request.stroke_ids,
        request.selection_mode,
    )
    .map_err(|error| error.to_string())?;
    let span =
        SourceSpan::new_ink(LayoutKind::Other, locator).map_err(|error| error.to_string())?;
    let payload = SourcePayload::Span(span);
    let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let header = EntityHeader::new(
        StableId::parse(format!("mathscape:ink-evidence/{digest}"))
            .map_err(|error| error.to_string())?,
        RevisionId::parse(format!("sha256:{digest}")).map_err(|error| error.to_string())?,
        EntityKind::SourceSpan,
        SchemaVersion::parse("mathscape-ink-evidence-1").map_err(|error| error.to_string())?,
        LifecycleStatus::Active,
        time,
        actor.clone(),
        None,
    );
    let entity = EntityEnvelope::new(header, payload).map_err(|error| error.to_string())?;
    MapPatch::new(
        None,
        actor.clone(),
        None,
        Some("登记手稿快照选区为来源证据".into()),
        vec![MapOperation::CreateEntity { entity }],
    )
    .map_err(|error| error.to_string())
}
