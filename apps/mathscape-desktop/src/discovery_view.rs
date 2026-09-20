//! Stable, read-only Desktop projection of a validated staged discovery run.

use std::collections::{HashMap, HashSet};

use mathscape_domain::{
    ContentHash, DiscoveryContextRequest, DiscoveryIssueKind, DiscoveryRelationKind,
    DiscoveryUnitKind, UnitCompleteness,
};
use mathscape_llm::LlmFinishReason;
use mathscape_orchestrator::{
    DiscoveryWorkflow, LocatedFact, ReferenceResolutionStatus, ResolvedDiscoverySelector,
    WorkflowStatus,
};
use serde::Serialize;

pub const DESKTOP_DISCOVERY_VIEW_VERSION: &str = "mathscape-desktop-discovery-view-1";

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryView {
    version: &'static str,
    status: WorkflowStatus,
    complete: bool,
    source: Option<DesktopDiscoverySourceView>,
    units: Vec<DesktopDiscoveryUnitView>,
    references: Vec<DesktopDiscoveryReferenceView>,
    relations: Vec<DesktopDiscoveryRelationView>,
    issues: Vec<DesktopDiscoveryIssueView>,
    pending_tasks: Vec<DesktopPendingFactTaskView>,
    diagnostics: Vec<DesktopDiscoveryDiagnosticView>,
    attempts: Vec<DesktopDiscoveryAttemptView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoverySourceView {
    artifact_id: String,
    conversion_id: Option<String>,
    content_hash: ContentHash,
    input_fingerprint: ContentHash,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopSourceAnchorView {
    start: u64,
    end: u64,
    block_ids: Vec<String>,
    excerpt: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopLocatedTextView {
    fact_ref: String,
    text: String,
    anchor: DesktopSourceAnchorView,
    normalized: bool,
    precision_review: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFactStatus {
    Processed,
    Pending,
    RequiresUnitRevision,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryUnitView {
    unit_ref: String,
    unit_kind: DiscoveryUnitKind,
    title: String,
    display_title: Option<String>,
    parent: Option<String>,
    completeness: UnitCompleteness,
    fact_status: DesktopFactStatus,
    source_excerpt: String,
    anchors: Vec<DesktopSourceAnchorView>,
    source_labels: Vec<DesktopLocatedTextView>,
    source_title: Option<DesktopLocatedTextView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryReferenceView {
    owner: String,
    text: String,
    anchor: DesktopSourceAnchorView,
    status: ReferenceResolutionStatus,
    candidates: Vec<String>,
    reason: String,
    origins: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryRelationView {
    source: String,
    target: String,
    relation: DiscoveryRelationKind,
    evidence: Vec<DesktopSourceAnchorView>,
    bases: Vec<String>,
    origins: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopIssueStage {
    UnitDiscovery,
    SourceFacts,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryIssueView {
    issue_ref: String,
    issue_refs: Vec<String>,
    stage: DesktopIssueStage,
    stages: Vec<DesktopIssueStage>,
    owner: Option<String>,
    kind: DiscoveryIssueKind,
    observation: String,
    observations: Vec<String>,
    occurrences: usize,
    context_request: Option<DiscoveryContextRequest>,
    requires_unit_revision: bool,
    evidence: Vec<DesktopSourceAnchorView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopPendingFactTaskView {
    task_ref: String,
    owner: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryDiagnosticView {
    /// First report containing this diagnostic, retained for v1 consumers.
    report: usize,
    reports: Vec<usize>,
    occurrences: usize,
    code: String,
    path: String,
    message: String,
    checked: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryAttemptView {
    attempt: usize,
    stage: String,
    repair: bool,
    status: WorkflowStatus,
    error: Option<String>,
    provider_model: Option<String>,
    finish_reason: Option<&'static str>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    elapsed_ms: Option<u64>,
}

impl DesktopDiscoveryView {
    pub fn status(&self) -> &WorkflowStatus {
        &self.status
    }

    pub fn project(workflow: &DiscoveryWorkflow<'_>) -> Self {
        let result = workflow.result();
        let source = workflow
            .source_input()
            .map(|input| DesktopDiscoverySourceView {
                artifact_id: input.snapshot.artifact_id.to_string(),
                conversion_id: input
                    .snapshot
                    .conversion_id
                    .as_ref()
                    .map(ToString::to_string),
                content_hash: input.snapshot.content_hash.clone(),
                input_fingerprint: input.fingerprint.clone(),
            });
        let tasks: HashMap<_, _> = result
            .as_ref()
            .into_iter()
            .flat_map(|result| result.tasks())
            .map(|task| (task.owner(), task))
            .collect();
        let units = workflow
            .units()
            .into_iter()
            .flat_map(|unit_set| unit_set.units())
            .map(|unit| {
                let proposal = unit.proposal();
                let task = tasks.get(proposal.unit_ref.as_str()).copied();
                let source_labels = task
                    .into_iter()
                    .flat_map(|task| task.labels())
                    .map(DesktopLocatedTextView::from)
                    .collect::<Vec<_>>();
                let source_title = task
                    .and_then(|task| task.title())
                    .map(DesktopLocatedTextView::from);
                let title = source_title
                    .as_ref()
                    .map(|title| title.text.clone())
                    .or_else(|| source_labels.first().map(|label| label.text.clone()))
                    .or_else(|| proposal.display_title.clone())
                    .unwrap_or_else(|| proposal.unit_ref.clone());
                let fact_status = match task {
                    Some(task) if task.requires_unit_revision() => {
                        DesktopFactStatus::RequiresUnitRevision
                    }
                    Some(task) if task.processed() => DesktopFactStatus::Processed,
                    _ => DesktopFactStatus::Pending,
                };
                DesktopDiscoveryUnitView {
                    unit_ref: proposal.unit_ref.clone(),
                    unit_kind: proposal.unit_kind,
                    title,
                    display_title: proposal.display_title.clone(),
                    parent: proposal.parent.clone(),
                    completeness: proposal.completeness,
                    fact_status,
                    source_excerpt: bounded(
                        &unit
                            .selectors()
                            .iter()
                            .map(ResolvedDiscoverySelector::text)
                            .collect::<Vec<_>>()
                            .join("\n"),
                        480,
                    ),
                    anchors: unit
                        .selectors()
                        .iter()
                        .map(DesktopSourceAnchorView::from)
                        .collect(),
                    source_labels,
                    source_title,
                }
            })
            .collect();
        let references = result
            .as_ref()
            .into_iter()
            .flat_map(|result| result.references())
            .map(|reference| DesktopDiscoveryReferenceView {
                owner: reference.owner.clone(),
                text: bounded(reference.location.text(), 240),
                anchor: DesktopSourceAnchorView::from(&reference.location),
                status: reference.resolution.status.clone(),
                candidates: reference.resolution.candidates.clone(),
                reason: reference.resolution.reason.clone(),
                origins: reference.origins.clone(),
            })
            .collect();
        let relations = result
            .as_ref()
            .into_iter()
            .flat_map(|result| result.relations())
            .map(|relation| DesktopDiscoveryRelationView {
                source: relation.source.clone(),
                target: relation.target.clone(),
                relation: relation.relation,
                evidence: relation
                    .evidence
                    .iter()
                    .map(DesktopSourceAnchorView::from)
                    .collect(),
                bases: relation.bases.clone(),
                origins: relation.origins.clone(),
            })
            .collect();
        let mut issues = Vec::new();
        if let Some(unit_set) = workflow.units() {
            issues.extend(unit_set.issues().iter().enumerate().map(|(index, issue)| {
                let proposal = issue.proposal();
                DesktopDiscoveryIssueView {
                    issue_ref: format!("a{}", index + 1),
                    issue_refs: vec![format!("a{}", index + 1)],
                    stage: DesktopIssueStage::UnitDiscovery,
                    stages: vec![DesktopIssueStage::UnitDiscovery],
                    owner: proposal.unit.clone(),
                    kind: proposal.kind,
                    observation: bounded(&proposal.observation, 1_000),
                    observations: vec![bounded(&proposal.observation, 1_000)],
                    occurrences: 1,
                    context_request: proposal.context_request.clone(),
                    requires_unit_revision: false,
                    evidence: issue
                        .evidence()
                        .iter()
                        .map(DesktopSourceAnchorView::from)
                        .collect(),
                }
            }));
        }
        if let Some(result) = &result {
            for task in result.tasks() {
                issues.extend(task.issues().iter().enumerate().map(
                    |(index, (issue, evidence))| DesktopDiscoveryIssueView {
                        issue_ref: format!("{}:i{}", task.task_ref(), index + 1),
                        issue_refs: vec![format!("{}:i{}", task.task_ref(), index + 1)],
                        stage: DesktopIssueStage::SourceFacts,
                        stages: vec![DesktopIssueStage::SourceFacts],
                        owner: Some(task.owner().to_owned()),
                        kind: issue.kind,
                        observation: bounded(&issue.observation, 1_000),
                        observations: vec![bounded(&issue.observation, 1_000)],
                        occurrences: 1,
                        context_request: issue.context_request.clone(),
                        requires_unit_revision: issue.requires_unit_revision,
                        evidence: evidence.iter().map(DesktopSourceAnchorView::from).collect(),
                    },
                ));
            }
        }
        issues = aggregate_issues(issues);
        let mut seen_pending = HashSet::new();
        let pending_tasks = result
            .as_ref()
            .into_iter()
            .flat_map(|result| result.pending_tasks())
            .filter(|task| seen_pending.insert((*task).clone()))
            .map(|task| DesktopPendingFactTaskView {
                task_ref: task.clone(),
                owner: workflow.planned_task_owner(task).map(str::to_owned),
            })
            .collect();
        let mut diagnostics = Vec::new();
        let mut diagnostic_indexes = HashMap::new();
        for (report_index, value) in workflow.diagnostics().iter().enumerate() {
            let report = report_index + 1;
            for diagnostic in &value.diagnostics {
                push_diagnostic(
                    &mut diagnostics,
                    &mut diagnostic_indexes,
                    report,
                    diagnostic.code.clone(),
                    diagnostic.path.clone(),
                    diagnostic.message.clone(),
                    true,
                );
            }
            for path in &value.not_checked {
                push_diagnostic(
                    &mut diagnostics,
                    &mut diagnostic_indexes,
                    report,
                    "not_checked".into(),
                    path.clone(),
                    "上游验证失败，因此此项未继续检查。".into(),
                    false,
                );
            }
        }
        let attempts = workflow
            .attempts()
            .iter()
            .enumerate()
            .map(|(index, attempt)| {
                let response = attempt.run.response.as_ref();
                DesktopDiscoveryAttemptView {
                    attempt: index + 1,
                    stage: attempt.stage.clone(),
                    repair: attempt.repair,
                    status: attempt.status.clone(),
                    error: attempt.error.clone(),
                    provider_model: response.map(|response| response.target.provider_model.clone()),
                    finish_reason: response
                        .and_then(|response| response.finish_reason.as_ref())
                        .map(finish_reason),
                    input_tokens: response.and_then(|response| response.usage.input_tokens),
                    output_tokens: response.and_then(|response| response.usage.output_tokens),
                    total_tokens: response.and_then(|response| response.usage.total_tokens),
                    elapsed_ms: response.map(|response| response.elapsed_ms),
                }
            })
            .collect();
        Self {
            version: DESKTOP_DISCOVERY_VIEW_VERSION,
            status: workflow.status().clone(),
            complete: result.as_ref().is_some_and(|result| result.complete()),
            source,
            units,
            references,
            relations,
            issues,
            pending_tasks,
            diagnostics,
            attempts,
        }
    }
}

fn aggregate_issues(mut issues: Vec<DesktopDiscoveryIssueView>) -> Vec<DesktopDiscoveryIssueView> {
    let mut aggregated = Vec::with_capacity(issues.len());
    for issue in issues.drain(..) {
        let Some(existing) = aggregated
            .iter_mut()
            .find(|existing| same_issue(existing, &issue))
        else {
            aggregated.push(issue);
            continue;
        };
        existing.occurrences += issue.occurrences;
        for issue_ref in issue.issue_refs {
            if !existing.issue_refs.contains(&issue_ref) {
                existing.issue_refs.push(issue_ref);
            }
        }
        for stage in issue.stages {
            if !existing.stages.contains(&stage) {
                existing.stages.push(stage);
            }
        }
        for observation in issue.observations {
            if !existing.observations.contains(&observation) {
                existing.observations.push(observation);
            }
        }
        for evidence in issue.evidence {
            if !existing
                .evidence
                .iter()
                .any(|current| current.start == evidence.start && current.end == evidence.end)
            {
                existing.evidence.push(evidence);
            }
        }
        existing.requires_unit_revision |= issue.requires_unit_revision;
        if existing.context_request.is_none() {
            existing.context_request = issue.context_request;
        }
        existing.observation = existing.observations.join("；");
    }
    aggregated
}

fn same_issue(left: &DesktopDiscoveryIssueView, right: &DesktopDiscoveryIssueView) -> bool {
    if left.kind != right.kind || left.owner != right.owner {
        return false;
    }
    if left.evidence.is_empty() || right.evidence.is_empty() {
        return left.evidence.is_empty()
            && right.evidence.is_empty()
            && left.observation == right.observation;
    }
    left.evidence.iter().any(|left| {
        right
            .evidence
            .iter()
            .any(|right| left.start < right.end && right.start < left.end)
    })
}

type DiagnosticKey = (String, String, String, bool);

fn push_diagnostic(
    diagnostics: &mut Vec<DesktopDiscoveryDiagnosticView>,
    indexes: &mut HashMap<DiagnosticKey, usize>,
    report: usize,
    code: String,
    path: String,
    message: String,
    checked: bool,
) {
    let key = (code.clone(), path.clone(), message.clone(), checked);
    if let Some(index) = indexes.get(&key).copied() {
        let existing = &mut diagnostics[index];
        existing.occurrences += 1;
        if !existing.reports.contains(&report) {
            existing.reports.push(report);
        }
        return;
    }
    indexes.insert(key, diagnostics.len());
    diagnostics.push(DesktopDiscoveryDiagnosticView {
        report,
        reports: vec![report],
        occurrences: 1,
        code,
        path,
        message,
        checked,
    });
}

impl From<&ResolvedDiscoverySelector> for DesktopSourceAnchorView {
    fn from(selector: &ResolvedDiscoverySelector) -> Self {
        Self {
            start: selector.range().start,
            end: selector.range().end,
            block_ids: selector.block_ids().to_vec(),
            excerpt: bounded(selector.text(), 240),
        }
    }
}

impl From<&LocatedFact> for DesktopLocatedTextView {
    fn from(fact: &LocatedFact) -> Self {
        Self {
            fact_ref: fact.fact_ref.clone(),
            text: bounded(fact.location.text(), 240),
            anchor: DesktopSourceAnchorView::from(&fact.location),
            normalized: fact.normalized,
            precision_review: fact.precision_review,
        }
    }
}

fn bounded(value: &str, max_characters: usize) -> String {
    if value.chars().count() <= max_characters {
        return value.to_owned();
    }
    let mut result = value.chars().take(max_characters).collect::<String>();
    result.push('…');
    result
}

fn finish_reason(reason: &LlmFinishReason) -> &'static str {
    match reason {
        LlmFinishReason::Completed { .. } => "completed",
        LlmFinishReason::MaxTokens { .. } => "max_tokens",
        LlmFinishReason::ToolCall { .. } => "tool_call",
        LlmFinishReason::ContentFilter { .. } => "content_filter",
        LlmFinishReason::StopSequence { .. } => "stop_sequence",
        LlmFinishReason::Other { .. } => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_diagnostics_are_collapsed_for_presentation() {
        let mut diagnostics = Vec::new();
        let mut indexes = HashMap::new();
        for report in [1, 2] {
            push_diagnostic(
                &mut diagnostics,
                &mut indexes,
                report,
                "invalid_range".into(),
                "tasks.t1.source_title".into(),
                "bad title".into(),
                true,
            );
        }
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].report, 1);
        assert_eq!(diagnostics[0].reports, [1, 2]);
        assert_eq!(diagnostics[0].occurrences, 2);
    }

    #[test]
    fn cross_stage_issues_are_aggregated_by_owner_kind_and_overlapping_evidence() {
        let issue =
            |issue_ref: &str, stage: DesktopIssueStage, start: u64| DesktopDiscoveryIssueView {
                issue_ref: issue_ref.into(),
                issue_refs: vec![issue_ref.into()],
                stage,
                stages: vec![stage],
                owner: Some("u4".into()),
                kind: DiscoveryIssueKind::SuspectedConversionError,
                observation: format!("observation {issue_ref}"),
                observations: vec![format!("observation {issue_ref}")],
                occurrences: 1,
                context_request: None,
                requires_unit_revision: false,
                evidence: vec![DesktopSourceAnchorView {
                    start,
                    end: start + 10,
                    block_ids: vec![],
                    excerpt: "fixture".into(),
                }],
            };
        let aggregated = aggregate_issues(vec![
            issue("a1", DesktopIssueStage::UnitDiscovery, 10),
            issue("t4:i1", DesktopIssueStage::SourceFacts, 15),
        ]);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].occurrences, 2);
        assert_eq!(aggregated[0].issue_refs, ["a1", "t4:i1"]);
        assert_eq!(aggregated[0].stages.len(), 2);
        assert_eq!(aggregated[0].observations.len(), 2);
    }
}
