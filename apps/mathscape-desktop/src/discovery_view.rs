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
    stage: DesktopIssueStage,
    owner: Option<String>,
    kind: DiscoveryIssueKind,
    observation: String,
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
    report: usize,
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
                    stage: DesktopIssueStage::UnitDiscovery,
                    owner: proposal.unit.clone(),
                    kind: proposal.kind,
                    observation: bounded(&proposal.observation, 1_000),
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
                        stage: DesktopIssueStage::SourceFacts,
                        owner: Some(task.owner().to_owned()),
                        kind: issue.kind,
                        observation: bounded(&issue.observation, 1_000),
                        context_request: issue.context_request.clone(),
                        requires_unit_revision: issue.requires_unit_revision,
                        evidence: evidence.iter().map(DesktopSourceAnchorView::from).collect(),
                    },
                ));
            }
        }
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
        let diagnostics = workflow
            .diagnostics()
            .iter()
            .enumerate()
            .flat_map(|(report, value)| {
                let checked = value.diagnostics.iter().map(move |diagnostic| {
                    DesktopDiscoveryDiagnosticView {
                        report: report + 1,
                        code: diagnostic.code.clone(),
                        path: diagnostic.path.clone(),
                        message: diagnostic.message.clone(),
                        checked: true,
                    }
                });
                let not_checked =
                    value
                        .not_checked
                        .iter()
                        .map(move |path| DesktopDiscoveryDiagnosticView {
                            report: report + 1,
                            code: "not_checked".into(),
                            path: path.clone(),
                            message: "上游验证失败，因此此项未继续检查。".into(),
                            checked: false,
                        });
                checked.chain(not_checked)
            })
            .collect();
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
