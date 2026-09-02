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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntityEnvelope<P> {
    header: EntityHeader,
    payload: P,
}

impl<P> EntityEnvelope<P> {
    pub fn new(header: EntityHeader, payload: P) -> Self {
        Self { header, payload }
    }

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
