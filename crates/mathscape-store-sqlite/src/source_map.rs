use mathscape_domain::{
    CommitId, ContentHash, EntityKind, MapOperation, MapPatch, RevisionId, SourcePayload, StableId,
    Timestamp,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{SqliteStoreError, lifecycle};

pub const SOURCE_REGISTRATION_RECEIPT_VERSION: &str = "mathscape-source-registration-receipt-2";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapObjectOutcome {
    Created,
    Reused,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceRegistrationReceipt {
    pub version: String,
    pub decision_id: StableId,
    pub plan_fingerprint: ContentHash,
    pub commit_id: CommitId,
    pub created_objects: Vec<StableId>,
    pub reused_objects: Vec<StableId>,
    #[serde(default)]
    pub reactivated_objects: Vec<StableId>,
    #[serde(default)]
    pub revised_objects: Vec<StableId>,
    pub span_ids: Vec<StableId>,
    pub unit_ids: Vec<StableId>,
    pub relation_ids: Vec<StableId>,
    pub committed_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredMapObject {
    pub stable_id: StableId,
    pub revision_id: RevisionId,
    pub kind: EntityKind,
    pub relation: bool,
    pub payload: Value,
    pub lifecycle: mathscape_domain::LifecycleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredMapEndpoint {
    pub role: String,
    pub ordinal: Option<u32>,
    pub entity_id: StableId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceMapSnapshot {
    pub objects: Vec<StoredMapObject>,
    pub relation_endpoints: std::collections::BTreeMap<StableId, Vec<StoredMapEndpoint>>,
}

pub(super) fn commit_source_registration(
    transaction: &Transaction<'_>,
    decision_id: &StableId,
    plan_fingerprint: &ContentHash,
    patch: &MapPatch<SourcePayload>,
    committed_at: Timestamp,
) -> Result<SourceRegistrationReceipt, SqliteStoreError> {
    patch.validate()?;
    super::registration::ensure_not_discarded(transaction, decision_id.as_str())?;
    let accepted_plan = transaction
        .query_row(
            "SELECT plan_fingerprint FROM discovery_acceptance_decisions WHERE decision_id = ?1",
            [decision_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if accepted_plan.as_deref() != Some(plan_fingerprint.as_str()) {
        return Err(SqliteStoreError::SourceRegistrationInvariant(
            "registration does not match an accepted plan".into(),
        ));
    }
    if let Some((stored_plan, receipt)) = transaction
        .query_row(
            "SELECT plan_fingerprint, receipt_json FROM source_registration_commits
             WHERE decision_id = ?1",
            [decision_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if stored_plan != plan_fingerprint.as_str() {
            return Err(SqliteStoreError::SourceRegistrationInvariant(
                "decision is already registered under another plan".into(),
            ));
        }
        return serde_json::from_str(&receipt).map_err(Into::into);
    }

    let commit_id = CommitId::from_revision(RevisionId::parse(plan_fingerprint.as_str())?);
    let mut created_objects = Vec::new();
    let mut reused_objects = Vec::new();
    let mut span_ids = Vec::new();
    let mut unit_ids = Vec::new();
    let mut relation_ids = Vec::new();
    let mut persisted = Vec::new();

    // Entities are installed first so parent, evidence and relation endpoints
    // can be checked against the complete candidate batch.
    for operation in patch.operations() {
        match operation {
            MapOperation::CreateEntity { entity } => {
                let kind = entity.header().kind();
                if !matches!(
                    entity.payload(),
                    SourcePayload::Span(_) | SourcePayload::SemanticUnit(_)
                ) {
                    return Err(SqliteStoreError::SourceRegistrationInvariant(
                        "source registration patch contains a non-registerable entity".into(),
                    ));
                }
                let outcome =
                    persist_object(transaction, entity.header(), entity.payload(), false, &[])?;
                record_outcome(
                    entity.header().stable_id(),
                    outcome,
                    &mut created_objects,
                    &mut reused_objects,
                );
                match kind {
                    EntityKind::SourceSpan => span_ids.push(entity.header().stable_id().clone()),
                    EntityKind::SourceSemanticUnit => {
                        unit_ids.push(entity.header().stable_id().clone())
                    }
                    _ => unreachable!("payload variant checked above"),
                }
                persisted.push((
                    entity.header().stable_id().clone(),
                    current_revision(transaction, entity.header().stable_id())?,
                    outcome,
                ));
            }
            MapOperation::CreateRelation { .. } => {}
            MapOperation::ReviseEntity { .. } | MapOperation::ReviseRelation { .. } => {
                return Err(SqliteStoreError::SourceRegistrationInvariant(
                    "source registration v1 accepts create operations only".into(),
                ));
            }
        }
    }

    validate_source_entities(transaction, patch)?;

    for operation in patch.operations() {
        let MapOperation::CreateRelation { relation } = operation else {
            continue;
        };
        if !matches!(relation.payload(), SourcePayload::UnitRelation(_)) {
            return Err(SqliteStoreError::SourceRegistrationInvariant(
                "source registration patch contains a non-source relation".into(),
            ));
        }
        let outcome = persist_object(
            transaction,
            relation.header(),
            relation.payload(),
            true,
            relation.endpoints(),
        )?;
        record_outcome(
            relation.header().stable_id(),
            outcome,
            &mut created_objects,
            &mut reused_objects,
        );
        relation_ids.push(relation.header().stable_id().clone());
        persisted.push((
            relation.header().stable_id().clone(),
            current_revision(transaction, relation.header().stable_id())?,
            outcome,
        ));
    }

    transaction.execute(
        "INSERT INTO map_commits (commit_id, author, generation_run, message, committed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            commit_id.as_str(),
            patch.author().as_str(),
            patch.generation_run().map(StableId::as_str),
            patch.message(),
            committed_at.as_str(),
        ],
    )?;
    for (index, (object_id, revision_id, outcome)) in persisted.iter().enumerate() {
        transaction.execute(
            "INSERT INTO map_commit_objects
             (commit_id, ordinal, object_id, revision_id, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                commit_id.as_str(),
                i64::try_from(index + 1).expect("patch ordinal fits i64"),
                object_id.as_str(),
                revision_id.as_str(),
                outcome_text(*outcome),
            ],
        )?;
    }

    let receipt = SourceRegistrationReceipt {
        version: SOURCE_REGISTRATION_RECEIPT_VERSION.into(),
        decision_id: decision_id.clone(),
        plan_fingerprint: plan_fingerprint.clone(),
        commit_id: commit_id.clone(),
        created_objects,
        reused_objects,
        reactivated_objects: vec![],
        revised_objects: vec![],
        span_ids,
        unit_ids,
        relation_ids,
        committed_at: committed_at.clone(),
    };
    transaction.execute(
        "INSERT INTO source_registration_commits
         (decision_id, plan_fingerprint, commit_id, receipt_json, committed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            decision_id.as_str(),
            plan_fingerprint.as_str(),
            commit_id.as_str(),
            serde_json::to_string(&receipt)?,
            committed_at.as_str(),
        ],
    )?;
    Ok(receipt)
}

fn persist_object(
    transaction: &Transaction<'_>,
    header: &mathscape_domain::EntityHeader,
    payload: &SourcePayload,
    relation: bool,
    endpoints: &[mathscape_domain::RelationEndpoint],
) -> Result<MapObjectOutcome, SqliteStoreError> {
    let shape = if relation { "relation" } else { "entity" };
    let kind = kind_text(header.kind())?;
    let payload_json = serde_json::to_string(payload)?;
    let existing = transaction
        .query_row(
            "SELECT object_shape, entity_kind, current_revision FROM map_objects
             WHERE object_id = ?1",
            [header.stable_id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let outcome = if let Some((stored_shape, stored_kind, stored_revision)) = existing {
        if stored_shape != shape || stored_kind != kind {
            return Err(SqliteStoreError::SourceRegistrationInvariant(format!(
                "stable ID {} conflicts with an existing map object",
                header.stable_id()
            )));
        }
        let (stored_payload, stored_lifecycle) = transaction.query_row(
            "SELECT payload_json,lifecycle FROM map_object_revisions
             WHERE object_id = ?1 AND revision_id = ?2",
            params![header.stable_id().as_str(), stored_revision],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        if matches!(
            stored_lifecycle.as_str(),
            "deprecated" | "rejected" | "superseded"
        ) {
            return Err(SqliteStoreError::SourceRegistrationInvariant(
                "registration_review_required: 重新激活需要登记变更审核".into(),
            ));
        }
        if serde_json::from_str::<Value>(&stored_payload)?
            != serde_json::from_str::<Value>(&payload_json)?
            || !same_endpoints(transaction, header, endpoints, &stored_revision)?
        {
            return Err(SqliteStoreError::SourceRegistrationInvariant(format!(
                "revision {} has conflicting source content",
                header.revision_id()
            )));
        }
        MapObjectOutcome::Reused
    } else {
        transaction.execute(
            "INSERT INTO map_objects (object_id, object_shape, entity_kind, current_revision)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                header.stable_id().as_str(),
                shape,
                kind,
                header.revision_id().as_str(),
            ],
        )?;
        transaction.execute(
            "INSERT INTO map_object_revisions
             (object_id, revision_id, schema_version, lifecycle, created_at,
              created_by, generation_run, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                header.stable_id().as_str(),
                header.revision_id().as_str(),
                header.schema_version().as_str(),
                lifecycle(header.lifecycle())?,
                header.created_at().as_str(),
                header.created_by().as_str(),
                header.generation_run().map(StableId::as_str),
                payload_json,
            ],
        )?;
        for endpoint in endpoints {
            insert_endpoint(transaction, header, endpoint)?;
        }
        MapObjectOutcome::Created
    };
    Ok(outcome)
}

fn same_endpoints(
    transaction: &Transaction<'_>,
    header: &mathscape_domain::EntityHeader,
    expected: &[mathscape_domain::RelationEndpoint],
    revision: &str,
) -> Result<bool, SqliteStoreError> {
    let mut statement = transaction.prepare(
        "SELECT role, ordinal, entity_id FROM map_relation_endpoints
         WHERE relation_id = ?1 AND revision_id = ?2
         ORDER BY role, ordinal_key",
    )?;
    let stored = statement
        .query_map(params![header.stable_id().as_str(), revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<u32>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected = expected
        .iter()
        .map(|endpoint| {
            (
                endpoint.role().as_str().to_owned(),
                endpoint.ordinal(),
                endpoint.entity().as_str().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    Ok(stored == expected)
}

fn insert_endpoint(
    transaction: &Transaction<'_>,
    header: &mathscape_domain::EntityHeader,
    endpoint: &mathscape_domain::RelationEndpoint,
) -> Result<(), SqliteStoreError> {
    transaction.execute(
        "INSERT INTO map_relation_endpoints
         (relation_id, revision_id, role, ordinal_key, ordinal, entity_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            header.stable_id().as_str(),
            header.revision_id().as_str(),
            endpoint.role().as_str(),
            endpoint.ordinal().map(i64::from).unwrap_or(-1),
            endpoint.ordinal().map(i64::from),
            endpoint.entity().as_str(),
        ],
    )?;
    Ok(())
}

fn validate_source_entities(
    transaction: &Transaction<'_>,
    patch: &MapPatch<SourcePayload>,
) -> Result<(), SqliteStoreError> {
    for operation in patch.operations() {
        let MapOperation::CreateEntity { entity } = operation else {
            continue;
        };
        match entity.payload() {
            SourcePayload::Span(span) => {
                if let Some(markdown) = span.markdown() {
                    require_artifact(transaction, markdown.artifact())?;
                }
                if let Some(page) = span.page() {
                    require_artifact(transaction, page.artifact())?;
                }
            }
            SourcePayload::SemanticUnit(unit) => {
                if let Some(artifact) = unit.surface_content_artifact() {
                    require_artifact(transaction, artifact)?;
                }
                for span in unit.source_spans() {
                    require_map_kind(transaction, span, EntityKind::SourceSpan)?;
                }
                if let Some(parent) = unit.parent_unit() {
                    require_map_kind(transaction, parent, EntityKind::SourceSemanticUnit)?;
                }
                for reference in unit.local_references() {
                    require_map_kind(transaction, reference.source_span(), EntityKind::SourceSpan)?;
                    for target in reference.candidate_targets() {
                        require_map_kind(transaction, target, EntityKind::SourceSemanticUnit)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn require_artifact(
    transaction: &Transaction<'_>,
    artifact: &StableId,
) -> Result<(), SqliteStoreError> {
    if transaction
        .query_row(
            "SELECT 1 FROM artifacts WHERE artifact_id = ?1",
            [artifact.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_none()
    {
        return Err(SqliteStoreError::SourceRegistrationInvariant(format!(
            "source locator refers to unknown Artifact {artifact}"
        )));
    }
    Ok(())
}

fn require_map_kind(
    transaction: &Transaction<'_>,
    object: &StableId,
    expected: EntityKind,
) -> Result<(), SqliteStoreError> {
    let expected = kind_text(expected)?;
    let actual = transaction
        .query_row(
            "SELECT o.entity_kind FROM map_objects o JOIN map_object_revisions r ON r.object_id=o.object_id AND r.revision_id=o.current_revision WHERE o.object_id = ?1 AND r.lifecycle NOT IN ('deprecated','superseded','rejected')",
            [object.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if actual.as_deref() != Some(expected.as_str()) {
        return Err(SqliteStoreError::SourceRegistrationInvariant(format!(
            "map reference {object} does not name the required {expected}"
        )));
    }
    Ok(())
}
fn current_revision(
    transaction: &Transaction<'_>,
    id: &StableId,
) -> Result<RevisionId, SqliteStoreError> {
    let revision: String = transaction.query_row(
        "SELECT current_revision FROM map_objects WHERE object_id=?1",
        [id.as_str()],
        |r| r.get(0),
    )?;
    Ok(RevisionId::parse(revision)?)
}

fn record_outcome(
    object: &StableId,
    outcome: MapObjectOutcome,
    created: &mut Vec<StableId>,
    reused: &mut Vec<StableId>,
) {
    match outcome {
        MapObjectOutcome::Created => created.push(object.clone()),
        MapObjectOutcome::Reused => reused.push(object.clone()),
    }
}

fn outcome_text(outcome: MapObjectOutcome) -> &'static str {
    match outcome {
        MapObjectOutcome::Created => "created",
        MapObjectOutcome::Reused => "reused",
    }
}

fn kind_text(kind: EntityKind) -> Result<String, SqliteStoreError> {
    serde_json::to_value(kind)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| SqliteStoreError::SourceRegistrationInvariant("invalid EntityKind".into()))
}

pub(super) fn source_registration_receipt(
    connection: &rusqlite::Connection,
    decision_id: &StableId,
) -> Result<Option<SourceRegistrationReceipt>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT receipt_json FROM source_registration_commits WHERE decision_id = ?1",
            [decision_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|json| serde_json::from_str(&json).map_err(Into::into))
        .transpose()
}

pub(super) fn map_objects(
    connection: &rusqlite::Connection,
) -> Result<Vec<StoredMapObject>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT o.object_id, o.current_revision, o.entity_kind, o.object_shape, r.lifecycle, r.payload_json
         FROM map_objects o JOIN map_object_revisions r
           ON r.object_id = o.object_id AND r.revision_id = o.current_revision
         ORDER BY o.entity_kind, o.object_id",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .map(|row| {
            let (stable_id, revision_id, kind, shape, lifecycle, payload) = row?;
            Ok(StoredMapObject {
                stable_id: StableId::parse(stable_id)?,
                revision_id: RevisionId::parse(revision_id)?,
                kind: serde_json::from_value(Value::String(kind))?,
                relation: shape == "relation",
                payload: serde_json::from_str(&payload)?,
                lifecycle: crate::parse_lifecycle(&lifecycle)?,
            })
        })
        .collect()
}

pub(super) fn source_map_snapshot(
    connection: &rusqlite::Connection,
) -> Result<SourceMapSnapshot, SqliteStoreError> {
    let objects = map_objects(connection)?;
    let mut endpoints = std::collections::BTreeMap::new();
    let mut statement = connection.prepare(
        "SELECT relation_id, role, ordinal, entity_id
         FROM map_relation_endpoints e
         JOIN map_objects o ON o.object_id = e.relation_id
         WHERE e.revision_id = o.current_revision
         ORDER BY relation_id, role, ordinal_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<u32>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (relation, role, ordinal, entity_id) = row?;
        endpoints
            .entry(StableId::parse(relation)?)
            .or_insert_with(Vec::new)
            .push(StoredMapEndpoint {
                role,
                ordinal,
                entity_id: StableId::parse(entity_id)?,
            });
    }
    Ok(SourceMapSnapshot {
        objects,
        relation_endpoints: endpoints,
    })
}
