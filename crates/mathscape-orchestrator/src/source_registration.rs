//! Acceptance and deterministic planning for source registration. A plan is
//! still pre-commit data: it contains no `TrustRecord` and mutates no map.

use std::collections::{BTreeSet, HashMap};

use mathscape_domain::{
    ActorRef, AlignmentStatus, ByteRange, Confidence, ContentHash, CurationStatus,
    DialectScopeBinding, DiscoveryRelationKind, DiscoveryUnitKind, EntityEnvelope, EntityHeader,
    EntityKind, FactTaskProposal, LayoutKind, LifecycleStatus, LocalReference,
    LocalReferenceStatus, LocatedSourceText, MapOperation, MapPatch, MarkdownLocator, ParseStatus,
    ProofStatus, ProvenanceStatus, RevisionId, SchemaVersion, SegmentationStatus,
    SourceEditionBinding, SourceFactProposal, SourceFactVersion, SourcePayload, SourceSemanticUnit,
    SourceSpan, SourceUnitRelation, SpanGranularity, StableId, TextByteRange, Timestamp,
    TrustRecord, TypingStatus, UnitCompleteness, UnitDiscoveryCheckpoint,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DiscoveryResult, PreparedUnitDiscoveryRequest, ReferenceResolutionStatus,
    ResolvedDiscoverySelector, StageValidationReport, ValidatedUnitSet, assemble_discovery,
    fingerprint, plan_source_facts,
};

pub const SOURCE_REGISTRATION_PLAN_VERSION: &str = "mathscape-source-registration-plan-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceAuthority {
    HumanReview,
    AutomaticPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptanceSelection {
    AllValidated,
    ExplicitUnits { unit_refs: Vec<String> },
}

/// Revalidated authority recovered from persisted wire checkpoints. The
/// private fields prevent callers from fabricating validator-owned values.
pub struct RevalidatedDiscovery {
    units: ValidatedUnitSet,
    result: DiscoveryResult,
}

impl RevalidatedDiscovery {
    pub fn units(&self) -> &ValidatedUnitSet {
        &self.units
    }

    pub fn result(&self) -> &DiscoveryResult {
        &self.result
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryCheckpointRevalidationError {
    #[error("stored UnitSet no longer validates: {0}")]
    Units(String),
    #[error("stored fact task {0} is duplicate or was not planned")]
    UnknownFactTask(String),
    #[error("stored fact task {task} no longer validates: {diagnostics}")]
    Fact { task: String, diagnostics: String },
    #[error("stored discovery result can no longer be assembled: {0}")]
    Assembly(String),
}

/// Replays the deterministic validators over persisted untrusted proposals.
/// It makes no LLM request and restores authority only when every persisted
/// fact reproduces under the current pipeline and source input.
pub fn revalidate_discovery_checkpoint(
    prepared: &PreparedUnitDiscoveryRequest,
    unit_checkpoint: &UnitDiscoveryCheckpoint,
    fact_proposals: Vec<FactTaskProposal>,
    max_tasks_per_batch: usize,
) -> Result<RevalidatedDiscovery, DiscoveryCheckpointRevalidationError> {
    let units = prepared
        .revalidate_checkpoint(unit_checkpoint, Default::default())
        .map_err(|report| DiscoveryCheckpointRevalidationError::Units(diagnostics(&report)))?;
    let batches = plan_source_facts(prepared, &units, max_tasks_per_batch)
        .map_err(|report| DiscoveryCheckpointRevalidationError::Units(diagnostics(&report)))?;

    let planned = batches
        .iter()
        .flat_map(|batch| batch.model_input().tasks.iter())
        .map(|task| task.task_ref.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for fact in &fact_proposals {
        if !seen.insert(fact.task_ref.as_str()) || !planned.contains(fact.task_ref.as_str()) {
            return Err(DiscoveryCheckpointRevalidationError::UnknownFactTask(
                fact.task_ref.clone(),
            ));
        }
    }

    let mut validated = Vec::new();
    for batch in &batches {
        for proposal in fact_proposals.iter().filter(|proposal| {
            batch
                .model_input()
                .tasks
                .iter()
                .any(|task| task.task_ref == proposal.task_ref)
        }) {
            let envelope = SourceFactProposal {
                version: SourceFactVersion::V1,
                request_ref: batch.model_input().request_ref.clone(),
                tasks: vec![proposal.clone()],
            };
            let validation = batch.validate(&envelope);
            let Some(task) = validation
                .tasks
                .into_iter()
                .find(|task| task.task_ref() == proposal.task_ref)
            else {
                return Err(DiscoveryCheckpointRevalidationError::Fact {
                    task: proposal.task_ref.clone(),
                    diagnostics: diagnostics(&validation.report),
                });
            };
            validated.push(task);
        }
    }
    let result = assemble_discovery(&units, &batches, validated)
        .map_err(DiscoveryCheckpointRevalidationError::Assembly)?;
    Ok(RevalidatedDiscovery { units, result })
}

fn diagnostics(report: &StageValidationReport) -> String {
    if report.diagnostics.is_empty() {
        return "validation produced no authorized result".into();
    }
    report
        .diagnostics
        .iter()
        .take(4)
        .map(|diagnostic| {
            format!(
                "{} · {}: {}",
                diagnostic.path, diagnostic.code, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryAcceptanceDecision {
    pub decision_id: StableId,
    pub discovery_run_id: StableId,
    pub actor: ActorRef,
    pub decided_at: Timestamp,
    pub authority: AcceptanceAuthority,
    pub expected_result_fingerprint: ContentHash,
    pub selection: AcceptanceSelection,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedSourceSpan {
    pub stable_id: StableId,
    pub markdown_artifact_id: StableId,
    pub range: TextByteRange,
    pub block_ids: Vec<String>,
    pub exact_text_hash: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedLocatedText {
    pub text: String,
    pub span_id: StableId,
    pub normalized: bool,
    pub precision_review: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedSourceUnit {
    pub source_ref: String,
    pub stable_id: StableId,
    pub unit_kind: DiscoveryUnitKind,
    pub display_title: Option<String>,
    pub source_title: Option<PlannedLocatedText>,
    pub source_labels: Vec<PlannedLocatedText>,
    pub parent: Option<StableId>,
    pub source_spans: Vec<StableId>,
    pub completeness: UnitCompleteness,
    pub facts_pending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedSourceReference {
    pub owner: StableId,
    pub mention: PlannedLocatedText,
    pub status: ReferenceResolutionStatus,
    pub candidate_targets: Vec<StableId>,
    pub unregistered_candidate_refs: Vec<String>,
    pub reason: String,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedSourceRelation {
    pub stable_id: StableId,
    pub source: StableId,
    pub target: StableId,
    pub relation: DiscoveryRelationKind,
    pub evidence_spans: Vec<StableId>,
    pub bases: Vec<String>,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceRegistrationPlan {
    pub version: String,
    pub plan_fingerprint: ContentHash,
    pub result_fingerprint: ContentHash,
    pub decision: DiscoveryAcceptanceDecision,
    pub markdown_artifact_id: StableId,
    pub conversion_id: Option<StableId>,
    pub input_fingerprint: ContentHash,
    pub unit_set_fingerprint: ContentHash,
    pub partial: bool,
    pub spans: Vec<PlannedSourceSpan>,
    pub units: Vec<PlannedSourceUnit>,
    pub references: Vec<PlannedSourceReference>,
    pub relations: Vec<PlannedSourceRelation>,
    pub pending_tasks: Vec<String>,
}

#[derive(Debug, Error)]
pub enum SourceMaterializationError {
    #[error("unsupported source registration plan version {0}")]
    UnsupportedVersion(String),
    #[error("planned source object refers to unknown span {0}")]
    UnknownSpan(StableId),
    #[error("planned source reference has no registered owner {0}")]
    UnknownOwner(StableId),
    #[error(transparent)]
    Domain(#[from] mathscape_domain::DomainError),
}

/// Converts an accepted deterministic plan into canonical Source Domain
/// entities. It does not query or mutate storage; create/reuse/conflict is
/// decided atomically by the repository.
pub fn materialize_source_registration(
    plan: &SourceRegistrationPlan,
) -> Result<MapPatch<SourcePayload>, SourceMaterializationError> {
    if plan.version != SOURCE_REGISTRATION_PLAN_VERSION {
        return Err(SourceMaterializationError::UnsupportedVersion(
            plan.version.clone(),
        ));
    }
    let trust = registration_trust(&plan.decision, &[])?;
    let schema = SchemaVersion::parse("mathscape-source-1")?;
    let lifecycle = match plan.decision.authority {
        AcceptanceAuthority::HumanReview => LifecycleStatus::Active,
        AcceptanceAuthority::AutomaticPolicy => LifecycleStatus::Candidate,
    };
    let span_ids = plan
        .spans
        .iter()
        .map(|span| span.stable_id.clone())
        .collect::<BTreeSet<_>>();
    let unit_ids = plan
        .units
        .iter()
        .map(|unit| unit.stable_id.clone())
        .collect::<BTreeSet<_>>();
    let mut operations = Vec::new();

    for planned in &plan.spans {
        let span = SourceSpan::new(
            SpanGranularity::Inline,
            LayoutKind::Other,
            None,
            vec![],
            planned.block_ids.clone(),
            Some(MarkdownLocator::new(
                planned.markdown_artifact_id.clone(),
                ByteRange::new(planned.range.start, planned.range.end)?,
                None,
            )),
            None,
            Some(planned.exact_text_hash.clone()),
        )?;
        let payload = SourcePayload::Span(span);
        let header = source_header(
            planned.stable_id.clone(),
            EntityKind::SourceSpan,
            &payload,
            schema.clone(),
            lifecycle,
            &plan.decision,
        )?;
        operations.push(MapOperation::CreateEntity {
            entity: EntityEnvelope::new(header, payload)?,
        });
    }

    for planned in &plan.units {
        for span in &planned.source_spans {
            if !span_ids.contains(span) {
                return Err(SourceMaterializationError::UnknownSpan(span.clone()));
            }
        }
        if planned
            .parent
            .as_ref()
            .is_some_and(|parent| !unit_ids.contains(parent))
        {
            return Err(SourceMaterializationError::UnknownOwner(
                planned.parent.clone().expect("checked Some"),
            ));
        }
        let source_title = planned
            .source_title
            .as_ref()
            .map(|value| located_source_text(value, &span_ids))
            .transpose()?;
        let source_labels = planned
            .source_labels
            .iter()
            .map(|value| located_source_text(value, &span_ids))
            .collect::<Result<Vec<_>, _>>()?;
        let references = plan
            .references
            .iter()
            .filter(|reference| reference.owner == planned.stable_id)
            .map(|reference| local_reference(reference, &span_ids))
            .collect::<Result<Vec<_>, _>>()?;
        let unit = SourceSemanticUnit::new(
            SourceEditionBinding::Unresolved,
            planned.unit_kind.into(),
            planned.display_title.clone(),
            source_title,
            source_labels,
            planned.parent.clone(),
            planned.source_spans.clone(),
            DialectScopeBinding::Uncontextualized,
            None,
            Some(plan.markdown_artifact_id.clone()),
            references,
            planned.completeness,
            planned.facts_pending,
            trust.clone(),
        )?;
        let payload = SourcePayload::SemanticUnit(unit);
        let header = source_header(
            planned.stable_id.clone(),
            EntityKind::SourceSemanticUnit,
            &payload,
            schema.clone(),
            lifecycle,
            &plan.decision,
        )?;
        operations.push(MapOperation::CreateEntity {
            entity: EntityEnvelope::new(header, payload)?,
        });
    }

    for reference in &plan.references {
        if !unit_ids.contains(&reference.owner) {
            return Err(SourceMaterializationError::UnknownOwner(
                reference.owner.clone(),
            ));
        }
    }
    for planned in &plan.relations {
        if !unit_ids.contains(&planned.source) {
            return Err(SourceMaterializationError::UnknownOwner(
                planned.source.clone(),
            ));
        }
        if !unit_ids.contains(&planned.target) {
            return Err(SourceMaterializationError::UnknownOwner(
                planned.target.clone(),
            ));
        }
        if let Some(span) = planned
            .evidence_spans
            .iter()
            .find(|span| !span_ids.contains(*span))
        {
            return Err(SourceMaterializationError::UnknownSpan(span.clone()));
        }
        let relation_trust = registration_trust(&plan.decision, &planned.evidence_spans)?;
        let relation_payload = SourceUnitRelation::new(planned.relation.into(), relation_trust);
        let revision = revision_for(&(
            &planned.stable_id,
            &planned.source,
            &planned.target,
            &planned.evidence_spans,
            &relation_payload,
        ))?;
        let header = EntityHeader::new(
            planned.stable_id.clone(),
            revision,
            EntityKind::SourceUnitRelation,
            schema.clone(),
            lifecycle,
            plan.decision.decided_at.clone(),
            plan.decision.actor.clone(),
            Some(plan.decision.discovery_run_id.clone()),
        );
        operations.push(MapOperation::CreateRelation {
            relation: relation_payload.into_record(
                header,
                planned.source.clone(),
                planned.target.clone(),
                planned.evidence_spans.clone(),
            )?,
        });
    }

    MapPatch::new(
        None,
        plan.decision.actor.clone(),
        Some(plan.decision.discovery_run_id.clone()),
        Some("Register accepted source discovery".into()),
        operations,
    )
    .map_err(Into::into)
}

fn located_source_text(
    planned: &PlannedLocatedText,
    spans: &BTreeSet<StableId>,
) -> Result<LocatedSourceText, SourceMaterializationError> {
    if !spans.contains(&planned.span_id) {
        return Err(SourceMaterializationError::UnknownSpan(
            planned.span_id.clone(),
        ));
    }
    LocatedSourceText::new(
        planned.text.clone(),
        planned.span_id.clone(),
        planned.normalized,
        planned.precision_review,
    )
    .map_err(Into::into)
}

fn local_reference(
    planned: &PlannedSourceReference,
    spans: &BTreeSet<StableId>,
) -> Result<LocalReference, SourceMaterializationError> {
    if !spans.contains(&planned.mention.span_id) {
        return Err(SourceMaterializationError::UnknownSpan(
            planned.mention.span_id.clone(),
        ));
    }
    let status = match planned.status {
        ReferenceResolutionStatus::Unresolved => LocalReferenceStatus::Unresolved,
        ReferenceResolutionStatus::Candidate => LocalReferenceStatus::Candidate,
        ReferenceResolutionStatus::ResolvedLocal => LocalReferenceStatus::Resolved,
        ReferenceResolutionStatus::Ambiguous => LocalReferenceStatus::Ambiguous,
    };
    let reason = if planned.unregistered_candidate_refs.is_empty() {
        planned.reason.clone()
    } else {
        format!(
            "{}; unregistered local candidates: {}",
            planned.reason,
            planned.unregistered_candidate_refs.join(", ")
        )
    };
    LocalReference::new(
        planned.mention.text.clone(),
        planned.mention.span_id.clone(),
        Some(planned.mention.text.clone()),
        planned.candidate_targets.clone(),
        status,
        reason,
        planned.origins.clone(),
    )
    .map_err(Into::into)
}

fn registration_trust(
    decision: &DiscoveryAcceptanceDecision,
    evidence: &[StableId],
) -> Result<TrustRecord, mathscape_domain::DomainError> {
    let (parse, segmentation, curation, confidence) = match decision.authority {
        AcceptanceAuthority::HumanReview => (
            ParseStatus::Reviewed,
            SegmentationStatus::Checked,
            CurationStatus::Accepted,
            0.95,
        ),
        AcceptanceAuthority::AutomaticPolicy => (
            ParseStatus::Parsed,
            SegmentationStatus::Proposed,
            CurationStatus::Raw,
            0.75,
        ),
    };
    Ok(TrustRecord::new(
        ProvenanceStatus::Exact,
        parse,
        segmentation,
        TypingStatus::Unchecked,
        ProofStatus::None,
        AlignmentStatus::Unmatched,
        curation,
        Confidence::new(confidence)?,
        evidence.to_vec(),
        vec![decision.actor.clone()],
    ))
}

fn source_header(
    stable_id: StableId,
    kind: EntityKind,
    payload: &SourcePayload,
    schema: SchemaVersion,
    lifecycle: LifecycleStatus,
    decision: &DiscoveryAcceptanceDecision,
) -> Result<EntityHeader, mathscape_domain::DomainError> {
    Ok(EntityHeader::new(
        stable_id.clone(),
        revision_for(&(stable_id, payload))?,
        kind,
        schema,
        lifecycle,
        decision.decided_at.clone(),
        decision.actor.clone(),
        Some(decision.discovery_run_id.clone()),
    ))
}

fn revision_for<T: Serialize>(value: &T) -> Result<RevisionId, mathscape_domain::DomainError> {
    RevisionId::parse(fingerprint(value).as_str())
}

pub fn discovery_result_fingerprint(result: &DiscoveryResult) -> ContentHash {
    // Unit/fact request refs are transport nonces and are regenerated during
    // checkpoint revalidation. Batch fingerprints also depend on the planner's
    // batching policy. Bind acceptance to canonical validated facts instead of
    // either transient implementation detail.
    let tasks = result
        .tasks
        .iter()
        .map(|task| {
            (
                &task.task_ref,
                &task.owner,
                &task.original,
                &task.labels,
                &task.title,
                &task.references,
                &task.relations,
                &task.issues,
            )
        })
        .collect::<Vec<_>>();
    fingerprint(&(
        &result.version,
        &result.units.input_fingerprint,
        &result.units.unit_set_fingerprint,
        tasks,
        &result.references,
        &result.relations,
        &result.pending_tasks,
        &result.pending_task_owners,
        result.complete,
    ))
}

/// Idempotent identity for one actor accepting one exact result and scope.
pub fn acceptance_decision_id(
    run_id: &StableId,
    result_fingerprint: &ContentHash,
    actor: &ActorRef,
    selection: &AcceptanceSelection,
) -> Result<StableId, SourceRegistrationPlanError> {
    stable_hash_id(
        "acceptance",
        &fingerprint(&(run_id, result_fingerprint, actor, selection)),
    )
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SourceRegistrationPlanError {
    #[error("acceptance decision refers to stale discovery result")]
    StaleDecision,
    #[error("validated source, UnitSet and result do not have the same input identity")]
    StaleCheckpoint,
    #[error("acceptance selection cannot be empty")]
    EmptySelection,
    #[error("acceptance selection contains duplicate unit {0}")]
    DuplicateUnit(String),
    #[error("acceptance selection contains unknown unit {0}")]
    UnknownUnit(String),
    #[error("accepted unit {unit} requires its parent {parent}")]
    MissingParent { unit: String, parent: String },
    #[error("accepted relation from {owner} requires target {target}")]
    RelationOutsideSelection { owner: String, target: String },
    #[error("accepted unit {0} requires its discovery boundary to be revised")]
    UnitRequiresRevision(String),
    #[error("could not construct a stable registration identity: {0}")]
    InvalidIdentity(String),
}

#[derive(Serialize)]
struct RegistrationPlanBody<'a> {
    version: &'a str,
    result_fingerprint: &'a ContentHash,
    decision: &'a DiscoveryAcceptanceDecision,
    markdown_artifact_id: &'a StableId,
    conversion_id: &'a Option<StableId>,
    input_fingerprint: &'a ContentHash,
    unit_set_fingerprint: &'a ContentHash,
    partial: bool,
    spans: &'a [PlannedSourceSpan],
    units: &'a [PlannedSourceUnit],
    references: &'a [PlannedSourceReference],
    relations: &'a [PlannedSourceRelation],
    pending_tasks: &'a [String],
}

pub fn plan_source_registration(
    source: &mathscape_domain::DiscoveryInput,
    units: &ValidatedUnitSet,
    result: &DiscoveryResult,
    decision: DiscoveryAcceptanceDecision,
) -> Result<SourceRegistrationPlan, SourceRegistrationPlanError> {
    if units.input_fingerprint() != &source.fingerprint
        || result.units().input_fingerprint != source.fingerprint
        || result.units().unit_set_fingerprint != *units.fingerprint()
    {
        return Err(SourceRegistrationPlanError::StaleCheckpoint);
    }
    let result_fingerprint = discovery_result_fingerprint(result);
    if decision.expected_result_fingerprint != result_fingerprint {
        return Err(SourceRegistrationPlanError::StaleDecision);
    }

    let all: BTreeSet<_> = units
        .units()
        .iter()
        .map(|unit| unit.proposal().unit_ref.clone())
        .collect();
    let selected: BTreeSet<String> = match &decision.selection {
        AcceptanceSelection::AllValidated => all.clone(),
        AcceptanceSelection::ExplicitUnits { unit_refs } => {
            if unit_refs.is_empty() {
                return Err(SourceRegistrationPlanError::EmptySelection);
            }
            let mut selected = BTreeSet::new();
            for unit_ref in unit_refs {
                if !selected.insert(unit_ref.clone()) {
                    return Err(SourceRegistrationPlanError::DuplicateUnit(unit_ref.clone()));
                }
                if !all.contains(unit_ref) {
                    return Err(SourceRegistrationPlanError::UnknownUnit(unit_ref.clone()));
                }
            }
            selected
        }
    };
    if selected.is_empty() {
        return Err(SourceRegistrationPlanError::EmptySelection);
    }
    for task in result.tasks() {
        if selected.contains(task.owner()) && task.requires_unit_revision() {
            return Err(SourceRegistrationPlanError::UnitRequiresRevision(
                task.owner().into(),
            ));
        }
    }
    for unit in units.units() {
        let proposal = unit.proposal();
        if selected.contains(&proposal.unit_ref)
            && let Some(parent) = &proposal.parent
            && !selected.contains(parent)
        {
            return Err(SourceRegistrationPlanError::MissingParent {
                unit: proposal.unit_ref.clone(),
                parent: parent.clone(),
            });
        }
    }
    for relation in result.relations() {
        if selected.contains(&relation.source) && !selected.contains(&relation.target) {
            return Err(SourceRegistrationPlanError::RelationOutsideSelection {
                owner: relation.source.clone(),
                target: relation.target.clone(),
            });
        }
    }

    let mut span_by_range = HashMap::<(u64, u64), usize>::new();
    let mut spans = Vec::<PlannedSourceSpan>::new();
    let mut intern_span = |selector: &ResolvedDiscoverySelector| {
        let key = (selector.range().start, selector.range().end);
        if let Some(index) = span_by_range.get(&key).copied() {
            return spans[index].stable_id.clone();
        }
        let identity = fingerprint(&(
            "markdown-source-span-1",
            &source.snapshot.artifact_id,
            selector.range(),
        ));
        let stable_id = stable_hash_id("source-span", &identity)
            .expect("a hash-derived StableId is always valid");
        let exact_text_hash = fingerprint(&selector.text());
        span_by_range.insert(key, spans.len());
        spans.push(PlannedSourceSpan {
            stable_id: stable_id.clone(),
            markdown_artifact_id: source.snapshot.artifact_id.clone(),
            range: selector.range().clone(),
            block_ids: selector.block_ids().to_vec(),
            exact_text_hash,
        });
        stable_id
    };

    let mut stable_units = HashMap::<String, StableId>::new();
    for unit in units
        .units()
        .iter()
        .filter(|unit| selected.contains(&unit.proposal().unit_ref))
    {
        let ranges: Vec<_> = unit.selectors().iter().map(|s| s.range()).collect();
        let identity = fingerprint(&("source-unit-1", &source.snapshot.artifact_id, ranges));
        stable_units.insert(
            unit.proposal().unit_ref.clone(),
            stable_hash_id("source-unit", &identity)?,
        );
    }

    let task_by_owner: HashMap<_, _> = result
        .tasks()
        .iter()
        .filter(|task| selected.contains(task.owner()))
        .map(|task| (task.owner(), task))
        .collect();
    let pending_by_owner: BTreeSet<_> = result
        .pending_tasks()
        .iter()
        .filter_map(|task| result.pending_task_owner(task))
        .filter(|owner| selected.contains(*owner))
        .map(str::to_owned)
        .collect();

    let mut planned_units = Vec::new();
    for unit in units
        .units()
        .iter()
        .filter(|unit| selected.contains(&unit.proposal().unit_ref))
    {
        let proposal = unit.proposal();
        let task = task_by_owner.get(proposal.unit_ref.as_str()).copied();
        let mut source_spans = unit
            .selectors()
            .iter()
            .map(&mut intern_span)
            .collect::<Vec<_>>();
        let located =
            |fact: &crate::LocatedFact,
             intern_span: &mut dyn FnMut(&ResolvedDiscoverySelector) -> StableId| {
                PlannedLocatedText {
                    text: fact.location.text().to_owned(),
                    span_id: intern_span(&fact.location),
                    normalized: fact.normalized,
                    precision_review: fact.precision_review,
                }
            };
        let source_title = task
            .and_then(|task| task.title())
            .map(|fact| located(fact, &mut intern_span));
        let source_labels = task
            .into_iter()
            .flat_map(|task| task.labels())
            .map(|fact| located(fact, &mut intern_span))
            .collect::<Vec<_>>();
        for span_id in source_title
            .iter()
            .map(|title| &title.span_id)
            .chain(source_labels.iter().map(|label| &label.span_id))
        {
            if !source_spans.contains(span_id) {
                source_spans.push(span_id.clone());
            }
        }
        planned_units.push(PlannedSourceUnit {
            source_ref: proposal.unit_ref.clone(),
            stable_id: stable_units[&proposal.unit_ref].clone(),
            unit_kind: proposal.unit_kind,
            display_title: proposal.display_title.clone(),
            source_title,
            source_labels,
            parent: proposal
                .parent
                .as_ref()
                .map(|parent| stable_units[parent].clone()),
            source_spans,
            completeness: proposal.completeness,
            facts_pending: task.is_none() || pending_by_owner.contains(&proposal.unit_ref),
        });
    }

    let mut planned_references = Vec::new();
    for reference in result
        .references()
        .iter()
        .filter(|reference| selected.contains(&reference.owner))
    {
        let mut candidates = Vec::new();
        let mut unregistered = Vec::new();
        for candidate in &reference.resolution.candidates {
            match stable_units.get(candidate) {
                Some(stable_id) => candidates.push(stable_id.clone()),
                None => unregistered.push(candidate.clone()),
            }
        }
        planned_references.push(PlannedSourceReference {
            owner: stable_units[&reference.owner].clone(),
            mention: PlannedLocatedText {
                text: reference.location.text().to_owned(),
                span_id: intern_span(&reference.location),
                normalized: false,
                precision_review: false,
            },
            status: reference.resolution.status.clone(),
            candidate_targets: candidates,
            unregistered_candidate_refs: unregistered,
            reason: reference.resolution.reason.clone(),
            origins: reference.origins.clone(),
        });
    }
    for reference in &planned_references {
        if let Some(unit) = planned_units
            .iter_mut()
            .find(|unit| unit.stable_id == reference.owner)
            && !unit.source_spans.contains(&reference.mention.span_id)
        {
            unit.source_spans.push(reference.mention.span_id.clone());
        }
    }

    let mut planned_relations = Vec::new();
    for relation in result.relations().iter().filter(|relation| {
        selected.contains(&relation.source) && selected.contains(&relation.target)
    }) {
        let evidence_spans = relation
            .evidence
            .iter()
            .map(&mut intern_span)
            .collect::<Vec<_>>();
        let source_id = stable_units[&relation.source].clone();
        let target_id = stable_units[&relation.target].clone();
        let identity = fingerprint(&(
            "source-relation-1",
            &source_id,
            &target_id,
            relation.relation,
            &evidence_spans,
        ));
        planned_relations.push(PlannedSourceRelation {
            stable_id: stable_hash_id("source-relation", &identity)?,
            source: source_id,
            target: target_id,
            relation: relation.relation,
            evidence_spans,
            bases: relation.bases.clone(),
            origins: relation.origins.clone(),
        });
    }

    spans.sort_by_key(|span| (span.range.start, span.range.end));
    planned_units.sort_by(|left, right| left.source_ref.cmp(&right.source_ref));
    let pending_tasks = result
        .pending_tasks()
        .iter()
        .filter(|task| {
            result
                .pending_task_owner(task)
                .is_some_and(|owner| selected.contains(owner))
        })
        .cloned()
        .collect::<Vec<_>>();
    let partial = !result.complete() || selected != all;
    let body = RegistrationPlanBody {
        version: SOURCE_REGISTRATION_PLAN_VERSION,
        result_fingerprint: &result_fingerprint,
        decision: &decision,
        markdown_artifact_id: &source.snapshot.artifact_id,
        conversion_id: &source.snapshot.conversion_id,
        input_fingerprint: &source.fingerprint,
        unit_set_fingerprint: units.fingerprint(),
        partial,
        spans: &spans,
        units: &planned_units,
        references: &planned_references,
        relations: &planned_relations,
        pending_tasks: &pending_tasks,
    };
    let plan_fingerprint = fingerprint(&body);
    Ok(SourceRegistrationPlan {
        version: SOURCE_REGISTRATION_PLAN_VERSION.into(),
        plan_fingerprint,
        result_fingerprint,
        decision,
        markdown_artifact_id: source.snapshot.artifact_id.clone(),
        conversion_id: source.snapshot.conversion_id.clone(),
        input_fingerprint: source.fingerprint.clone(),
        unit_set_fingerprint: units.fingerprint().clone(),
        partial,
        spans,
        units: planned_units,
        references: planned_references,
        relations: planned_relations,
        pending_tasks,
    })
}

fn stable_hash_id(
    kind: &str,
    fingerprint: &ContentHash,
) -> Result<StableId, SourceRegistrationPlanError> {
    StableId::parse(format!(
        "mathscape:{kind}/{}",
        fingerprint
            .as_str()
            .strip_prefix("sha256:")
            .unwrap_or(fingerprint.as_str())
    ))
    .map_err(|error| SourceRegistrationPlanError::InvalidIdentity(error.to_string()))
}
