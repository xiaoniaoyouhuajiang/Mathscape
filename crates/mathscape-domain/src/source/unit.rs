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
    Ambiguous,
    Broken,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourceEditionBinding {
    Unresolved,
    Bound { edition: StableId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DialectScopeBinding {
    Uncontextualized,
    Bound { scope: StableId },
}

/// Exact or safely normalized source text with its own evidence location.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocatedSourceText {
    text: String,
    source_span: StableId,
    normalized: bool,
    precision_review: bool,
}

impl LocatedSourceText {
    pub fn new(
        text: String,
        source_span: StableId,
        normalized: bool,
        precision_review: bool,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&text, "located_source_text.text")?;
        Ok(Self {
            text,
            source_span,
            normalized,
            precision_review,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn source_span(&self) -> &StableId {
        &self.source_span
    }

    pub fn normalized(&self) -> bool {
        self.normalized
    }

    pub fn precision_review(&self) -> bool {
        self.precision_review
    }
}

/// A source-local citation such as “see Definition 4.1.2”.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalReference {
    surface_form: String,
    source_span: StableId,
    target_local_label: Option<String>,
    candidate_targets: Vec<StableId>,
    status: LocalReferenceStatus,
    resolution_reason: String,
    origins: Vec<String>,
}

impl LocalReference {
    pub fn new(
        surface_form: String,
        source_span: StableId,
        target_local_label: Option<String>,
        candidate_targets: Vec<StableId>,
        status: LocalReferenceStatus,
        resolution_reason: String,
        origins: Vec<String>,
    ) -> Result<Self, DomainError> {
        ensure_non_blank(&surface_form, "local_reference.surface_form")?;
        ensure_optional_non_blank(
            target_local_label.as_deref(),
            "local_reference.target_local_label",
        )?;
        ensure_non_blank(&resolution_reason, "local_reference.resolution_reason")?;
        ensure_unique_refs(&candidate_targets, "local_reference.candidate_targets")?;
        if status == LocalReferenceStatus::Resolved && candidate_targets.len() != 1 {
            return Err(DomainError::InconsistentState {
                field: "local_reference.candidate_targets",
            });
        }
        if status == LocalReferenceStatus::Unresolved && !candidate_targets.is_empty() {
            return Err(DomainError::InconsistentState {
                field: "local_reference.unresolved_candidates",
            });
        }
        if origins.iter().any(|origin| origin.trim().is_empty()) {
            return Err(DomainError::EmptyValue {
                field: "local_reference.origins",
            });
        }
        Ok(Self {
            surface_form,
            source_span,
            target_local_label,
            candidate_targets,
            status,
            resolution_reason,
            origins,
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

    pub fn candidate_targets(&self) -> &[StableId] {
        &self.candidate_targets
    }

    pub fn status(&self) -> LocalReferenceStatus {
        self.status
    }

    pub fn resolution_reason(&self) -> &str {
        &self.resolution_reason
    }

    pub fn origins(&self) -> &[String] {
        &self.origins
    }
}

/// Minimal mathematical speech act tied to one edition and one or more spans.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceSemanticUnit {
    edition_binding: SourceEditionBinding,
    kind: SourceUnitKind,
    display_title: Option<String>,
    source_title: Option<LocatedSourceText>,
    source_labels: Vec<LocatedSourceText>,
    parent_unit: Option<StableId>,
    source_spans: Vec<StableId>,
    dialect_binding: DialectScopeBinding,
    resulting_dialect_scope: Option<StableId>,
    surface_content_artifact: Option<StableId>,
    local_references: Vec<LocalReference>,
    completeness: UnitCompleteness,
    facts_pending: bool,
    trust: TrustRecord,
}

impl SourceSemanticUnit {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        edition_binding: SourceEditionBinding,
        kind: SourceUnitKind,
        display_title: Option<String>,
        source_title: Option<LocatedSourceText>,
        source_labels: Vec<LocatedSourceText>,
        parent_unit: Option<StableId>,
        source_spans: Vec<StableId>,
        dialect_binding: DialectScopeBinding,
        resulting_dialect_scope: Option<StableId>,
        surface_content_artifact: Option<StableId>,
        local_references: Vec<LocalReference>,
        completeness: UnitCompleteness,
        facts_pending: bool,
        trust: TrustRecord,
    ) -> Result<Self, DomainError> {
        ensure_optional_non_blank(display_title.as_deref(), "source_unit.display_title")?;
        ensure_non_empty_unique_refs(&source_spans, "source_unit.source_spans")?;

        for fact in source_title.iter().chain(source_labels.iter()) {
            if !source_spans.contains(fact.source_span()) {
                return Err(DomainError::ArtifactMismatch {
                    field: "source_unit.located_text.source_span",
                });
            }
        }
        for reference in &local_references {
            if !source_spans.contains(reference.source_span()) {
                return Err(DomainError::ArtifactMismatch {
                    field: "local_reference.source_span",
                });
            }
        }

        Ok(Self {
            edition_binding,
            kind,
            display_title,
            source_title,
            source_labels,
            parent_unit,
            source_spans,
            dialect_binding,
            resulting_dialect_scope,
            surface_content_artifact,
            local_references,
            completeness,
            facts_pending,
            trust,
        })
    }

    pub fn edition_binding(&self) -> &SourceEditionBinding {
        &self.edition_binding
    }

    pub fn kind(&self) -> SourceUnitKind {
        self.kind
    }

    pub fn display_title(&self) -> Option<&str> {
        self.display_title.as_deref()
    }

    pub fn source_title(&self) -> Option<&LocatedSourceText> {
        self.source_title.as_ref()
    }

    pub fn source_labels(&self) -> &[LocatedSourceText] {
        &self.source_labels
    }

    pub fn parent_unit(&self) -> Option<&StableId> {
        self.parent_unit.as_ref()
    }

    pub fn source_spans(&self) -> &[StableId] {
        &self.source_spans
    }

    pub fn dialect_binding(&self) -> &DialectScopeBinding {
        &self.dialect_binding
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

    pub fn facts_pending(&self) -> bool {
        self.facts_pending
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
            SourceEditionBinding::Bound {
                edition: StableId::parse(
                    "emir-fixture:edition/analysis-principles-methods/local-pdf",
                )
                .unwrap(),
            },
            SourceUnitKind::Definition,
            Some("有限数列极限".to_owned()),
            None,
            vec![LocatedSourceText::new("4.1.1(1)".into(), span.clone(), false, false).unwrap()],
            Some(StableId::parse("emir-fixture:unit/source-a-section-4-1").unwrap()),
            vec![span],
            DialectScopeBinding::Bound {
                scope: StableId::parse("emir-fixture:scope/source-a/root").unwrap(),
            },
            Some(StableId::parse("emir-fixture:scope/source-a/finite-limit").unwrap()),
            None,
            vec![],
            UnitCompleteness::Complete,
            false,
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
            vec![],
            LocalReferenceStatus::Resolved,
            "unique local declaration".into(),
            vec!["t1:r1".into()],
        )
        .unwrap_err();
        assert_eq!(
            error,
            DomainError::InconsistentState {
                field: "local_reference.candidate_targets"
            }
        );
    }

    #[test]
    fn source_unit_can_be_registered_before_contextualization() {
        let span = StableId::parse("mathscape:source-span/markdown-only").unwrap();
        let unit = SourceSemanticUnit::new(
            SourceEditionBinding::Unresolved,
            SourceUnitKind::Statement,
            Some("闭包的基本性质".into()),
            None,
            vec![],
            None,
            vec![span],
            DialectScopeBinding::Uncontextualized,
            None,
            Some(StableId::parse("mathscape:artifact/markdown").unwrap()),
            vec![],
            UnitCompleteness::RightTruncated,
            true,
            trust(),
        )
        .unwrap();
        assert_eq!(unit.edition_binding(), &SourceEditionBinding::Unresolved);
        assert_eq!(
            unit.dialect_binding(),
            &DialectScopeBinding::Uncontextualized
        );
        assert!(unit.facts_pending());
    }
}
