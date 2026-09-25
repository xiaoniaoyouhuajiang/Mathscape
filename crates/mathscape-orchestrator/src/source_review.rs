//! Project a validated staged discovery workflow into the reusable source
//! review DTO. This module performs no acceptance or persistence.

use std::collections::{HashMap, HashSet};

use mathscape_domain::{
    SOURCE_REVIEW_PROJECTION_VERSION, SourceReviewAcceptance, SourceReviewAcceptanceScope,
    SourceReviewAcceptanceState, SourceReviewAnchor, SourceReviewAnchorPurpose,
    SourceReviewDiagnostic, SourceReviewDiagnosticSeverity, SourceReviewFactStatus,
    SourceReviewPendingTask, SourceReviewProjection, SourceReviewReference,
    SourceReviewReferenceStatus, SourceReviewRelation, SourceReviewRunStatus, SourceReviewSource,
    SourceReviewStructureRelation, SourceReviewTextFact, SourceReviewUnit,
};

use crate::{
    DiscoveryWorkflow, ResolvedDiscoverySelector, WorkflowStatus, discovery_result_fingerprint,
};

pub fn project_source_review(
    workflow: &DiscoveryWorkflow<'_>,
    run_id: mathscape_domain::StableId,
) -> Option<SourceReviewProjection> {
    let input = workflow.source_input()?;
    let units = workflow.units()?;
    let result = workflow.result()?;
    let result_fingerprint = discovery_result_fingerprint(&result);
    let tasks = result
        .tasks()
        .iter()
        .map(|task| (task.owner().to_owned(), task))
        .collect::<HashMap<_, _>>();
    let unit_refs = units
        .units()
        .iter()
        .map(|unit| unit.proposal().unit_ref.clone())
        .collect::<HashSet<_>>();

    let review_units = units
        .units()
        .iter()
        .map(|unit| {
            let proposal = unit.proposal();
            let task = tasks.get(&proposal.unit_ref).copied();
            let fact_status = match task {
                Some(task) if task.requires_unit_revision() => {
                    SourceReviewFactStatus::RequiresUnitRevision
                }
                Some(task) if task.processed() => SourceReviewFactStatus::Processed,
                _ => SourceReviewFactStatus::Pending,
            };
            let acceptance = if matches!(fact_status, SourceReviewFactStatus::RequiresUnitRevision)
            {
                SourceReviewAcceptanceState::Blocked
            } else {
                SourceReviewAcceptanceState::Selectable
            };
            let anchors = unit
                .selectors()
                .iter()
                .map(|selector| anchor(selector, SourceReviewAnchorPurpose::Unit))
                .collect::<Vec<_>>();
            let source_title = task
                .and_then(|task| task.title())
                .map(|fact| text_fact(fact, SourceReviewAnchorPurpose::SourceTitle));
            let source_labels = task
                .into_iter()
                .flat_map(|task| task.labels())
                .map(|fact| text_fact(fact, SourceReviewAnchorPurpose::SourceLabel))
                .collect();
            SourceReviewUnit {
                unit_ref: proposal.unit_ref.clone(),
                unit_kind: proposal.unit_kind,
                display_title: proposal.display_title.clone(),
                source_title,
                source_labels,
                parent: proposal.parent.clone(),
                completeness: proposal.completeness,
                fact_status,
                acceptance,
                source_excerpt: bounded(
                    &unit
                        .selectors()
                        .iter()
                        .map(ResolvedDiscoverySelector::text)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    480,
                ),
                anchors,
            }
        })
        .collect::<Vec<_>>();

    let references = result
        .references()
        .iter()
        .enumerate()
        .map(|(index, reference)| SourceReviewReference {
            reference_ref: format!("r{}", index + 1),
            owner: reference.owner.clone(),
            text: bounded(reference.location.text(), 240),
            anchor: anchor(&reference.location, SourceReviewAnchorPurpose::Reference),
            status: reference_status(&reference.resolution.status),
            candidates: reference.resolution.candidates.clone(),
            reason: reference.resolution.reason.clone(),
            origins: reference.origins.clone(),
        })
        .collect::<Vec<_>>();

    let structure_relations = units
        .units()
        .iter()
        .filter_map(|unit| {
            unit.proposal()
                .parent
                .as_ref()
                .map(|parent| SourceReviewStructureRelation {
                    relation_ref: format!("part:{}", unit.proposal().unit_ref),
                    parent: parent.clone(),
                    child: unit.proposal().unit_ref.clone(),
                })
        })
        .collect::<Vec<_>>();

    let relations = result
        .relations()
        .iter()
        .enumerate()
        .map(|(index, relation)| SourceReviewRelation {
            relation_ref: format!("rel{}", index + 1),
            source: relation.source.clone(),
            target: relation.target.clone(),
            relation: relation.relation,
            evidence: relation
                .evidence
                .iter()
                .map(|selector| anchor(selector, SourceReviewAnchorPurpose::RelationEvidence))
                .collect(),
            bases: relation.bases.clone(),
            origins: relation.origins.clone(),
        })
        .collect::<Vec<_>>();

    let pending_tasks = result
        .pending_tasks()
        .iter()
        .map(|task_ref| SourceReviewPendingTask {
            task_ref: task_ref.clone(),
            owner: result.pending_task_owner(task_ref).map(str::to_owned),
            reason: Some("来源事实尚未完成".into()),
        })
        .collect::<Vec<_>>();

    // Prior failed attempts remain visible for audit, but a successfully
    // assembled result proves that those diagnostics no longer block the
    // validated current checkpoint.
    let diagnostics = workflow
        .diagnostics()
        .iter()
        .flat_map(|report| report.diagnostics.iter())
        .enumerate()
        .map(|(index, diagnostic)| SourceReviewDiagnostic {
            diagnostic_ref: format!("d{}", index + 1),
            severity: SourceReviewDiagnosticSeverity::Warning,
            stage: "validation".into(),
            code: diagnostic.code.clone(),
            path: diagnostic.path.clone(),
            message: diagnostic.message.clone(),
            checked: true,
            related_units: related_units(&diagnostic.path, &unit_refs),
        })
        .collect::<Vec<_>>();

    let complete = result.complete();
    let blockers = review_units
        .iter()
        .filter(|unit| unit.acceptance == SourceReviewAcceptanceState::Blocked)
        .map(|unit| unit.unit_ref.clone())
        .collect::<Vec<_>>();
    let acceptance_state = if review_units
        .iter()
        .any(|unit| unit.acceptance == SourceReviewAcceptanceState::Selectable)
    {
        SourceReviewAcceptanceState::Selectable
    } else {
        SourceReviewAcceptanceState::Blocked
    };
    Some(SourceReviewProjection {
        version: SOURCE_REVIEW_PROJECTION_VERSION.into(),
        run_id,
        result_fingerprint,
        status: review_status(workflow.status()),
        complete,
        source: SourceReviewSource {
            artifact_id: input.snapshot.artifact_id.clone(),
            conversion_id: input.snapshot.conversion_id.clone(),
            content_hash: input.snapshot.content_hash.clone(),
            input_fingerprint: input.fingerprint.clone(),
        },
        units: review_units,
        references,
        structure_relations,
        relations,
        diagnostics,
        pending_tasks,
        acceptance: SourceReviewAcceptance {
            state: acceptance_state,
            selected_units: vec![],
            required_units: vec![],
            missing_targets: vec![],
            blockers,
            allowed_scopes: vec![SourceReviewAcceptanceScope::AllValidated],
        },
    })
}

fn anchor(
    selector: &ResolvedDiscoverySelector,
    purpose: SourceReviewAnchorPurpose,
) -> SourceReviewAnchor {
    SourceReviewAnchor {
        purpose,
        range: selector.range().clone(),
        block_ids: selector.block_ids().to_vec(),
        excerpt: bounded(selector.text(), 240),
    }
}

fn text_fact(
    fact: &crate::LocatedFact,
    purpose: SourceReviewAnchorPurpose,
) -> SourceReviewTextFact {
    SourceReviewTextFact {
        text: bounded(fact.location.text(), 240),
        anchor: anchor(&fact.location, purpose),
        normalized: fact.normalized,
        precision_review: fact.precision_review,
    }
}

fn reference_status(status: &crate::ReferenceResolutionStatus) -> SourceReviewReferenceStatus {
    match status {
        crate::ReferenceResolutionStatus::Unresolved => SourceReviewReferenceStatus::Unresolved,
        crate::ReferenceResolutionStatus::Candidate => SourceReviewReferenceStatus::Candidate,
        crate::ReferenceResolutionStatus::ResolvedLocal => {
            SourceReviewReferenceStatus::ResolvedLocal
        }
        crate::ReferenceResolutionStatus::Ambiguous => SourceReviewReferenceStatus::Ambiguous,
    }
}

fn review_status(status: &WorkflowStatus) -> SourceReviewRunStatus {
    match status {
        WorkflowStatus::Running => SourceReviewRunStatus::Running,
        WorkflowStatus::AwaitingPatchReview => SourceReviewRunStatus::AwaitingPatchReview,
        WorkflowStatus::Completed => SourceReviewRunStatus::Completed,
        WorkflowStatus::Partial => SourceReviewRunStatus::Partial,
        WorkflowStatus::NeedsAttention => SourceReviewRunStatus::NeedsAttention,
        WorkflowStatus::Failed => SourceReviewRunStatus::Failed,
        WorkflowStatus::Cancelled => SourceReviewRunStatus::Cancelled,
    }
}

fn related_units(path: &str, unit_refs: &HashSet<String>) -> Vec<String> {
    path.split(['.', '[', ']'])
        .find_map(|part| unit_refs.contains(part).then(|| part.to_owned()))
        .into_iter()
        .collect()
}

fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathscape_domain::StableId;

    #[test]
    fn maps_workflow_status_without_promoting_to_registered() {
        assert_eq!(
            review_status(&WorkflowStatus::Completed),
            SourceReviewRunStatus::Completed
        );
        assert_ne!(
            SourceReviewAcceptanceState::Selectable,
            SourceReviewAcceptanceState::Registered
        );
        assert_eq!(
            related_units("tasks.t12.references[0]", &HashSet::from(["u12".into()])),
            Vec::<String>::new()
        );
        let _ = StableId::parse("mathscape:discovery-run/test").unwrap();
    }
}
