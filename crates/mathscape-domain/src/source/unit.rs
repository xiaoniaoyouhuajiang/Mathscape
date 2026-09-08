use serde::{Deserialize, Serialize};

use crate::{
    DomainError, EndpointRole, EntityHeader, EntityKind, EntityPayload, PayloadShape,
    RelationEndpoint, RelationRecord, StableId, TrustRecord,
};

use super::{
    SourcePayload, SourceRelation, ensure_non_blank, ensure_non_empty_unique_refs,
    ensure_optional_non_blank, ensure_unique_refs,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceUnitKind {
    Section,
    Definition,
    Statement,
    Proof,
    ProofStep,
    Problem,
    Hint,
    Solution,
    Example,
    Remark,
    Method,
    Notation,
    Narrative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitCompleteness {
    Complete,
    LeftTruncated,
    RightTruncated,
    InternallyMissing,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceStatus {
    Unresolved,
    Candidate,
    Resolved,
    Broken,
}

/// A source-local citation such as “see Definition 4.1.2”.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalReference {
    surface_form: String,
    source_span: StableId,
    target_local_label: Option<String>,
    resolved_target: Option<StableId>,
    status: LocalReferenceStatus,
}

impl LocalReference {
    pub fn new(
        surface_form: String,
        source_span: StableId,
        target_local_label: Option<String>,
        resolved_target: Option<StableId>,
        status: LocalReferenceStatus,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&surface_form, "local_reference.surface_form")?;
        ensure_optional_non_blank(
            target_local_label.as_deref(),
            "local_reference.target_local_label",
        )?;
        if status == LocalReferenceStatus::Resolved && resolved_target.is_none() {
            return Err(DomainError::InconsistentState {
                field: "local_reference.resolved_target",
            });
        }
        Ok(Self {
            surface_form,
            source_span,
            target_local_label,
            resolved_target,
            status,
        })
    }

    pub fn source_span(&self) -> &StableId {
        &self.source_span
    }

    pub fn surface_form(&self) -> &str {
        &self.surface_form
    }

    pub fn target_local_label(&self) -> Option<&str> {
        self.target_local_label.as_deref()
    }

    pub fn resolved_target(&self) -> Option<&StableId> {
        self.resolved_target.as_ref()
    }

    pub fn status(&self) -> LocalReferenceStatus {
        self.status
    }
}

/// Minimal mathematical speech act tied to one edition and one or more spans.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceSemanticUnit {
    source_edition: StableId,
    kind: SourceUnitKind,
    local_label: Option<String>,
    local_title: Option<String>,
    parent_unit: Option<StableId>,
    source_spans: Vec<StableId>,
    dialect_scope: StableId,
    resulting_dialect_scope: Option<StableId>,
    surface_content_artifact: Option<StableId>,
    local_references: Vec<LocalReference>,
    completeness: UnitCompleteness,
    trust: TrustRecord,
}

impl SourceSemanticUnit {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_edition: StableId,
        kind: SourceUnitKind,
        local_label: Option<String>,
        local_title: Option<String>,
        parent_unit: Option<StableId>,
        source_spans: Vec<StableId>,
        dialect_scope: StableId,
        resulting_dialect_scope: Option<StableId>,
        surface_content_artifact: Option<StableId>,
        local_references: Vec<LocalReference>,
        completeness: UnitCompleteness,
        trust: TrustRecord,
    ) -> Result<Self, DomainError> {
        ensure_optional_non_blank(local_label.as_deref(), "source_unit.local_label")?;
        ensure_optional_non_blank(local_title.as_deref(), "source_unit.local_title")?;
        ensure_non_empty_unique_refs(&source_spans, "source_unit.source_spans")?;

        for reference in &local_references {
            if !source_spans.contains(reference.source_span()) {
                return Err(DomainError::ArtifactMismatch {
                    field: "local_reference.source_span",
                });
            }
        }

        Ok(Self {
            source_edition,
            kind,
            local_label,
            local_title,
            parent_unit,
            source_spans,
            dialect_scope,
            resulting_dialect_scope,
            surface_content_artifact,
            local_references,
            completeness,
            trust,
        })
    }

    pub fn source_edition(&self) -> &StableId {
        &self.source_edition
    }

    pub fn kind(&self) -> SourceUnitKind {
        self.kind
    }

    pub fn local_label(&self) -> Option<&str> {
        self.local_label.as_deref()
    }

    pub fn local_title(&self) -> Option<&str> {
        self.local_title.as_deref()
    }

    pub fn parent_unit(&self) -> Option<&StableId> {
        self.parent_unit.as_ref()
    }

    pub fn source_spans(&self) -> &[StableId] {
        &self.source_spans
    }

    pub fn dialect_scope(&self) -> &StableId {
        &self.dialect_scope
    }

    pub fn resulting_dialect_scope(&self) -> Option<&StableId> {
        self.resulting_dialect_scope.as_ref()
    }

    pub fn surface_content_artifact(&self) -> Option<&StableId> {
        self.surface_content_artifact.as_ref()
    }

    pub fn local_references(&self) -> &[LocalReference] {
        &self.local_references
    }

    pub fn completeness(&self) -> UnitCompleteness {
        self.completeness
    }

    pub fn trust(&self) -> &TrustRecord {
        &self.trust
    }
}

impl EntityPayload for SourceSemanticUnit {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceSemanticUnit
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceUnitRelationKind {
    States,
    Proves,
    HasPart,
    Poses,
    Hints,
    Solves,
    Explains,
    AppliesMethod,
    Cites,
}

/// Payload of a first-class relation between two source semantic units.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceUnitRelation {
    relation: SourceUnitRelationKind,
    trust: TrustRecord,
}

impl SourceUnitRelation {
    pub fn new(relation: SourceUnitRelationKind, trust: TrustRecord) -> Self {
        Self { relation, trust }
    }

    pub fn relation(&self) -> SourceUnitRelationKind {
        self.relation
    }

    pub fn trust(&self) -> &TrustRecord {
        &self.trust
    }

    pub fn into_record(
        self,
        header: EntityHeader,
        source: StableId,
        target: StableId,
        evidence_spans: Vec<StableId>,
    ) -> Result<SourceRelation, DomainError> {
        if source == target {
            return Err(DomainError::DuplicateReference {
                field: "source_unit_relation.source_and_target",
                stable_id: source.to_string(),
            });
        }
        if evidence_spans.is_empty() {
            return Err(DomainError::MissingEvidence);
        }
        ensure_unique_refs(&evidence_spans, "source_unit_relation.evidence_spans")?;

        let mut endpoints = vec![
            RelationEndpoint::new(EndpointRole::parse("source")?, source, None),
            RelationEndpoint::new(EndpointRole::parse("target")?, target, None),
        ];
        let evidence_role = EndpointRole::parse("evidence")?;
        for (ordinal, span) in evidence_spans.into_iter().enumerate() {
            let ordinal = u32::try_from(ordinal).map_err(|_| DomainError::TooManyItems {
                field: "source_unit_relation.evidence_spans",
                maximum: u64::from(u32::MAX),
            })?;
            endpoints.push(RelationEndpoint::new(
                evidence_role.clone(),
                span,
                Some(ordinal),
            ));
        }

        RelationRecord::new(header, endpoints, SourcePayload::UnitRelation(self))
    }
}

impl EntityPayload for SourceUnitRelation {
    fn entity_kind(&self) -> EntityKind {
        EntityKind::SourceUnitRelation
    }

    fn payload_shape(&self) -> PayloadShape {
        PayloadShape::Relation
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ActorRef, AlignmentStatus, Confidence, CurationStatus, LifecycleStatus, ParseStatus,
        ProofStatus, ProvenanceStatus, RevisionId, SchemaVersion, SegmentationStatus, Timestamp,
        TypingStatus,
    };

    use super::*;

    fn trust() -> TrustRecord {
        TrustRecord::new(
            ProvenanceStatus::Exact,
            ParseStatus::Reviewed,
            SegmentationStatus::Checked,
            TypingStatus::Unchecked,
            ProofStatus::None,
            AlignmentStatus::Unmatched,
            CurationStatus::Accepted,
            Confidence::new(0.95).unwrap(),
            vec![],
            vec![ActorRef::parse("codex-manual-curation").unwrap()],
        )
    }

    fn header(kind: EntityKind, id: &str) -> EntityHeader {
        EntityHeader::new(
            StableId::parse(id).unwrap(),
            RevisionId::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            kind,
            SchemaVersion::parse("emir-0.1.1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-09-01T00:00:00+08:00").unwrap(),
            ActorRef::parse("codex-manual-curation").unwrap(),
            None,
        )
    }

    #[test]
    fn fixture_style_semantic_unit_is_valid() {
        let span = StableId::parse("emir-fixture:span/source-a-definition").unwrap();
        let unit = SourceSemanticUnit::new(
            StableId::parse("emir-fixture:edition/analysis-principles-methods/local-pdf").unwrap(),
            SourceUnitKind::Definition,
            Some("4.1.1(1)".to_owned()),
            Some("有限数列极限".to_owned()),
            Some(StableId::parse("emir-fixture:unit/source-a-section-4-1").unwrap()),
            vec![span],
            StableId::parse("emir-fixture:scope/source-a/root").unwrap(),
            Some(StableId::parse("emir-fixture:scope/source-a/finite-limit").unwrap()),
            None,
            vec![],
            UnitCompleteness::Complete,
            trust(),
        )
        .unwrap();

        assert_eq!(unit.kind(), SourceUnitKind::Definition);
        assert_eq!(unit.source_spans().len(), 1);
    }

    #[test]
    fn source_relation_normalizes_evidence_as_role_labelled_endpoints() {
        let relation = SourceUnitRelation::new(SourceUnitRelationKind::Proves, trust())
            .into_record(
                header(
                    EntityKind::SourceUnitRelation,
                    "emir-fixture:relation/cesaro-proof-proves-problem",
                ),
                StableId::parse("emir-fixture:unit/source-b-cesaro-proof").unwrap(),
                StableId::parse("emir-fixture:unit/source-b-cesaro-problem").unwrap(),
                vec![StableId::parse("emir-fixture:span/source-b-cesaro-proof").unwrap()],
            )
            .unwrap();

        assert_eq!(relation.endpoints().len(), 3);
        assert_eq!(relation.endpoints()[2].role().as_str(), "evidence");
    }

    #[test]
    fn source_relation_requires_evidence() {
        let error = SourceUnitRelation::new(SourceUnitRelationKind::Proves, trust())
            .into_record(
                header(
                    EntityKind::SourceUnitRelation,
                    "mathscape:relation/proof-without-evidence",
                ),
                StableId::parse("mathscape:unit/proof").unwrap(),
                StableId::parse("mathscape:unit/statement").unwrap(),
                vec![],
            )
            .unwrap_err();
        assert_eq!(error, DomainError::MissingEvidence);
    }

    #[test]
    fn resolved_local_reference_requires_a_target() {
        let error = LocalReference::new(
            "参看定义 4.1.2".to_owned(),
            StableId::parse("mathscape:span/reference").unwrap(),
            Some("4.1.2".to_owned()),
            None,
            LocalReferenceStatus::Resolved,
        )
        .unwrap_err();
        assert_eq!(
            error,
            DomainError::InconsistentState {
                field: "local_reference.resolved_target"
            }
        );
    }
}
