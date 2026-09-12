//! Acceptance and deterministic planning for source registration. A plan is
//! still pre-commit data: it contains no `TrustRecord` and mutates no map.

use std::collections::{BTreeSet, HashMap};

use mathscape_domain::{
    ActorRef, ContentHash, DiscoveryRelationKind, DiscoveryUnitKind, StableId, TextByteRange,
    Timestamp, UnitCompleteness,
};
use serde::Serialize;
use thiserror::Error;

use crate::{
    DiscoveryResult, ReferenceResolutionStatus, ResolvedDiscoverySelector, ValidatedUnitSet,
    fingerprint,
};

pub const SOURCE_REGISTRATION_PLAN_VERSION: &str = "mathscape-source-registration-plan-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceAuthority {
    HumanReview,
    AutomaticPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptanceSelection {
    AllValidated,
    ExplicitUnits { unit_refs: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryAcceptanceDecision {
    pub decision_id: StableId,
    pub discovery_run_id: StableId,
    pub actor: ActorRef,
    pub decided_at: Timestamp,
    pub authority: AcceptanceAuthority,
    pub expected_result_fingerprint: ContentHash,
    pub selection: AcceptanceSelection,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedSourceSpan {
    pub stable_id: StableId,
    pub markdown_artifact_id: StableId,
    pub range: TextByteRange,
    pub block_ids: Vec<String>,
    pub exact_text_hash: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedLocatedText {
    pub text: String,
    pub span_id: StableId,
    pub normalized: bool,
    pub precision_review: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedSourceReference {
    pub owner: StableId,
    pub mention: PlannedLocatedText,
    pub status: ReferenceResolutionStatus,
    pub candidate_targets: Vec<StableId>,
    pub unregistered_candidate_refs: Vec<String>,
    pub reason: String,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedSourceRelation {
    pub stable_id: StableId,
    pub source: StableId,
    pub target: StableId,
    pub relation: DiscoveryRelationKind,
    pub evidence_spans: Vec<StableId>,
    pub bases: Vec<String>,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceRegistrationPlan {
    pub version: &'static str,
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

pub fn discovery_result_fingerprint(result: &DiscoveryResult) -> ContentHash {
    fingerprint(result)
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
    #[error("could not construct a stable registration identity: {0}")]
    InvalidIdentity(String),
}

#[derive(Serialize)]
struct RegistrationPlanBody<'a> {
    version: &'static str,
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
        let source_spans = unit
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
            .collect();
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
        version: SOURCE_REGISTRATION_PLAN_VERSION,
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
