use serde::{Deserialize, Serialize};

use crate::{ActorRef, RevisionId, SchemaVersion, StableId, Timestamp};

/// Persistent object kinds currently present in EMIR v0.1.1 and the RDTX view.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[non_exhaustive]
pub enum EntityKind {
    SourceWork,
    SourceEdition,
    SourceArtifact,
    IngestionRequest,
    IngestionResult,
    ConversionRun,
    DocumentIndex,
    RegionCandidate,
    MapBuildUnit,
    BoundaryIssue,
    SourceSpan,
    SourceSemanticUnit,
    LocalSymbol,
    DialectScope,
    RepresentationBinding,
    ProofContext,
    ProofState,
    ProofTransition,
    SourceUnitRelation,
    ExpressionCandidate,
    SemanticEntity,
    Alignment,
    TheoryAdapter,
    Evidence,
    Debt,
    ParseIssue,
    QualityReport,
    RepresentationCore,
    OperationalProfile,
    Inference,
    Transport,
    Macro,
}

/// Governance state of one logical object.
///
/// Lifecycle changes create new revisions; old revisions remain addressable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LifecycleStatus {
    Draft,
    Candidate,
    Active,
    Superseded,
    Rejected,
    Deprecated,
    ParseError,
    SemanticError,
}

/// Common metadata required by every persistent entity or relation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntityHeader {
    stable_id: StableId,
    revision_id: RevisionId,
    kind: EntityKind,
    schema_version: SchemaVersion,
    lifecycle: LifecycleStatus,
    created_at: Timestamp,
    created_by: ActorRef,
    generation_run: Option<StableId>,
}

impl EntityHeader {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stable_id: StableId,
        revision_id: RevisionId,
        kind: EntityKind,
        schema_version: SchemaVersion,
        lifecycle: LifecycleStatus,
        created_at: Timestamp,
        created_by: ActorRef,
        generation_run: Option<StableId>,
    ) -> Self {
        Self {
            stable_id,
            revision_id,
            kind,
            schema_version,
            lifecycle,
            created_at,
            created_by,
            generation_run,
        }
    }

    pub fn stable_id(&self) -> &StableId {
        &self.stable_id
    }

    pub fn revision_id(&self) -> &RevisionId {
        &self.revision_id
    }

    pub fn kind(&self) -> EntityKind {
        self.kind
    }

    pub fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }

    pub fn lifecycle(&self) -> LifecycleStatus {
        self.lifecycle
    }

    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    pub fn created_by(&self) -> &ActorRef {
        &self.created_by
    }

    pub fn generation_run(&self) -> Option<&StableId> {
        self.generation_run.as_ref()
    }
}

/// A typed payload under the common persistent-entity envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EntityEnvelope<P> {
    header: EntityHeader,
    payload: P,
}

/// Reports the persistent kind carried by a payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadShape {
    Entity,
    Relation,
}

pub trait EntityPayload {
    fn entity_kind(&self) -> EntityKind;

    fn payload_shape(&self) -> PayloadShape {
        PayloadShape::Entity
    }
}

impl<P: EntityPayload> EntityEnvelope<P> {
    pub fn new(header: EntityHeader, payload: P) -> Result<Self, crate::DomainError> {
        let actual_shape = payload.payload_shape();
        if actual_shape != PayloadShape::Entity {
            return Err(crate::DomainError::PayloadShapeMismatch {
                expected: PayloadShape::Entity,
                actual: actual_shape,
            });
        }
        let expected = payload.entity_kind();
        let actual = header.kind();
        if expected != actual {
            return Err(crate::DomainError::EntityKindMismatch { expected, actual });
        }
        Ok(Self { header, payload })
    }
}

impl<P> EntityEnvelope<P> {
    pub fn header(&self) -> &EntityHeader {
        &self.header
    }

    pub fn payload(&self) -> &P {
        &self.payload
    }

    pub fn into_parts(self) -> (EntityHeader, P) {
        (self.header, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use crate::{ActorRef, DomainError, RevisionId, SchemaVersion, Timestamp};

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SpanPayload;

    impl EntityPayload for SpanPayload {
        fn entity_kind(&self) -> EntityKind {
            EntityKind::SourceSpan
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RelationPayload;

    impl EntityPayload for RelationPayload {
        fn entity_kind(&self) -> EntityKind {
            EntityKind::ConversionRun
        }

        fn payload_shape(&self) -> PayloadShape {
            PayloadShape::Relation
        }
    }

    fn header(kind: EntityKind, id: &str) -> EntityHeader {
        EntityHeader::new(
            StableId::parse(id).unwrap(),
            RevisionId::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            kind,
            SchemaVersion::parse("mathscape-domain-0.1").unwrap(),
            LifecycleStatus::Candidate,
            Timestamp::parse("2026-09-02T00:00:00+08:00").unwrap(),
            ActorRef::parse("domain-test").unwrap(),
            None,
        )
    }

    #[test]
    fn envelope_rejects_a_payload_under_the_wrong_header_kind() {
        let header = header(EntityKind::SourceArtifact, "mathscape:artifact/example");

        let error = EntityEnvelope::new(header, SpanPayload).unwrap_err();
        assert_eq!(
            error,
            DomainError::EntityKindMismatch {
                expected: EntityKind::SourceSpan,
                actual: EntityKind::SourceArtifact,
            }
        );
    }

    #[test]
    fn entity_envelope_rejects_relation_payloads() {
        let error = EntityEnvelope::new(
            header(EntityKind::ConversionRun, "mathscape:conversion/example"),
            RelationPayload,
        )
        .unwrap_err();
        assert_eq!(
            error,
            DomainError::PayloadShapeMismatch {
                expected: PayloadShape::Entity,
                actual: PayloadShape::Relation,
            }
        );
    }
}
