//! Bounded, in-memory orchestration. Each provider request is independent.
use crate::source_facts::{decode_response, pending_run};
use crate::unit_discovery::{fingerprint, new_request_ref};
use crate::*;
use mathscape_domain::*;
use mathscape_llm::*;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::time::{Instant, timeout_at};

#[derive(Clone, Debug)]
pub struct DiscoveryWorkflowLimits {
    pub timeout: Duration,
    pub max_calls: usize,
    pub max_tasks_per_batch: usize,
    pub max_batches: usize,
    pub max_total_tokens: Option<u64>,
}
impl Default for DiscoveryWorkflowLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            max_calls: 32,
            max_tasks_per_batch: 16,
            max_batches: 16,
            max_total_tokens: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Running,
    Completed,
    Partial,
    NeedsAttention,
    Failed,
    Cancelled,
}

/// Observable lifecycle events for application shells. Raw provider deltas stay
/// on the LLM backend observer; `AttemptStarted` binds those deltas to a stage.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiscoveryWorkflowEvent {
    WorkflowStarted,
    AttemptStarted {
        attempt: usize,
        stage: String,
        batch_index: Option<usize>,
        batch_total: Option<usize>,
        repair: bool,
    },
    AttemptFinished {
        attempt: usize,
        stage: String,
        status: WorkflowStatus,
    },
    ValidationStarted {
        stage: String,
        batch_index: Option<usize>,
        batch_total: Option<usize>,
    },
    UnitCheckpointCreated {
        units: usize,
        partial: bool,
    },
    FactPlanCreated {
        batches: usize,
        tasks: usize,
    },
    FactCheckpointUpdated {
        completed_tasks: usize,
        total_tasks: usize,
    },
    AssemblyStarted,
    WorkflowFinished {
        status: WorkflowStatus,
    },
}

pub type DiscoveryWorkflowObserver = Arc<dyn Fn(DiscoveryWorkflowEvent) + Send + Sync>;

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowAttempt {
    pub stage: String,
    pub batch: Option<ContentHash>,
    pub repair: bool,
    pub request: LlmRequest,
    pub raw_text: Option<String>,
    pub run: DiscoveryRunSummary,
    pub status: WorkflowStatus,
    pub error: Option<String>,
}

/// Exportable audit/checkpoint view, not a deserializable trusted workflow.
#[derive(Clone, Debug, Serialize)]
pub struct DiscoveryWorkflowSnapshot {
    pub status: WorkflowStatus,
    pub source: Option<DiscoveryInput>,
    pub units: Option<UnitDiscoveryCheckpoint>,
    pub result: Option<DiscoveryResult>,
    pub attempts: Vec<WorkflowAttempt>,
    pub diagnostics: Vec<StageValidationReport>,
    pub events: Vec<String>,
}

pub struct DiscoveryWorkflow<'a> {
    backend: &'a dyn LlmBackend,
    limits: DiscoveryWorkflowLimits,
    attempts: Vec<WorkflowAttempt>,
    units: Option<ValidatedUnitSet>,
    batches: Vec<PreparedFactRequest>,
    facts: HashMap<String, ValidatedFactTask>,
    repairs: HashSet<String>,
    diagnostics: Vec<StageValidationReport>,
    status: WorkflowStatus,
    events: Vec<String>,
    source_input: Option<DiscoveryInput>,
    observer: Option<DiscoveryWorkflowObserver>,
}
impl<'a> DiscoveryWorkflow<'a> {
    pub fn new(backend: &'a dyn LlmBackend, limits: DiscoveryWorkflowLimits) -> Self {
        Self {
            backend,
            limits,
            attempts: vec![],
            units: None,
            batches: vec![],
            facts: HashMap::new(),
            repairs: HashSet::new(),
            diagnostics: vec![],
            status: WorkflowStatus::Running,
            events: vec![],
            source_input: None,
            observer: None,
        }
    }
    pub fn with_observer(mut self, observer: DiscoveryWorkflowObserver) -> Self {
        self.observer = Some(observer);
        self
    }
    fn emit(&self, event: DiscoveryWorkflowEvent) {
        if let Some(observer) = &self.observer {
            observer(event);
        }
    }
    fn finish(&mut self, status: WorkflowStatus) -> WorkflowStatus {
        self.status = status.clone();
        self.emit(DiscoveryWorkflowEvent::WorkflowFinished {
            status: status.clone(),
        });
        status
    }
    fn fact_progress(&self) -> (usize, usize) {
        let total = self
            .batches
            .iter()
            .flat_map(|batch| &batch.model.tasks)
            .count();
        let completed = self
            .facts
            .values()
            .filter(|task| task.processed() && !task.requires_unit_revision())
            .count();
        (completed, total)
    }
    pub fn attempts(&self) -> &[WorkflowAttempt] {
        &self.attempts
    }
    pub fn units(&self) -> Option<&ValidatedUnitSet> {
        self.units.as_ref()
    }
    pub fn diagnostics(&self) -> &[StageValidationReport] {
        &self.diagnostics
    }
    pub fn status(&self) -> &WorkflowStatus {
        &self.status
    }
    pub fn events(&self) -> &[String] {
        &self.events
    }
    pub fn source_input(&self) -> Option<&DiscoveryInput> {
        self.source_input.as_ref()
    }
    pub fn planned_task_owner(&self, task_ref: &str) -> Option<&str> {
        self.batches
            .iter()
            .flat_map(|batch| &batch.model.tasks)
            .find(|task| task.task_ref == task_ref)
            .map(|task| task.owner_unit_ref.as_str())
    }
    pub fn snapshot(&self) -> DiscoveryWorkflowSnapshot {
        DiscoveryWorkflowSnapshot {
            status: self.status.clone(),
            source: self.source_input.clone(),
            units: self.units.as_ref().map(ValidatedUnitSet::checkpoint),
            result: self.result(),
            attempts: self.attempts.clone(),
            diagnostics: self.diagnostics.clone(),
            events: self.events.clone(),
        }
    }
    pub fn result(&self) -> Option<DiscoveryResult> {
        let units = self.units.as_ref()?;
        // A completed but unplanned state must never look like a complete pipeline.
        if self.batches.is_empty() && !units.units().is_empty() {
            return None;
        }
        let mut facts: Vec<_> = self.facts.values().cloned().collect();
        facts.sort_by(|a, b| a.task_ref.cmp(&b.task_ref));
        assemble_discovery(units, &self.batches, facts).ok()
    }
    pub fn cancel(&mut self) {
        for a in &mut self.attempts {
            if a.status == WorkflowStatus::Running {
                a.status = WorkflowStatus::Cancelled;
            }
        }
        self.finish(WorkflowStatus::Cancelled);
    }

    async fn call<T: DeserializeOwned + Serialize>(
        &mut self,
        request: LlmRequest,
        prompt: ContentHash,
        batch: Option<ContentHash>,
        batch_position: Option<(usize, usize)>,
        repair: bool,
        deadline: Instant,
    ) -> Result<T, String> {
        if self.attempts.len() >= self.limits.max_calls {
            return Err("workflow call budget exhausted".into());
        }
        if Instant::now() >= deadline {
            return Err("workflow deadline exceeded".into());
        }
        if let Some(limit) = self.limits.max_total_tokens {
            let usages: Vec<_> = self
                .attempts
                .iter()
                .filter_map(|a| a.run.response.as_ref())
                .map(|r| r.usage.total_tokens)
                .collect();
            if usages.iter().any(Option::is_none) {
                return Err("cannot enforce token budget: provider usage unavailable".into());
            }
            if usages.into_iter().flatten().fold(0u64, u64::saturating_add) >= limit {
                return Err("workflow token budget exhausted".into());
            }
        }
        let run = pending_run(self.backend, &request, &prompt);
        let i = self.attempts.len();
        self.events.push(format!(
            "{}{}",
            request.task.kind,
            if repair { " repair" } else { "" }
        ));
        self.attempts.push(WorkflowAttempt {
            stage: request.task.kind.clone(),
            batch,
            repair,
            request: request.clone(),
            raw_text: None,
            run,
            status: WorkflowStatus::Running,
            error: None,
        });
        self.emit(DiscoveryWorkflowEvent::AttemptStarted {
            attempt: i + 1,
            stage: self.attempts[i].stage.clone(),
            batch_index: batch_position.map(|position| position.0),
            batch_total: batch_position.map(|position| position.1),
            repair,
        });
        let response = match timeout_at(deadline, self.backend.execute_stream(request)).await {
            Ok(Ok(r)) => r,
            failure => {
                let message = match failure {
                    Ok(Err(e)) => e.to_string(),
                    _ => "workflow deadline exceeded".into(),
                };
                self.attempts[i].status = WorkflowStatus::Failed;
                self.attempts[i].error = Some(message.clone());
                self.emit(DiscoveryWorkflowEvent::AttemptFinished {
                    attempt: i + 1,
                    stage: self.attempts[i].stage.clone(),
                    status: WorkflowStatus::Failed,
                });
                return Err(message);
            }
        };
        self.attempts[i].run.observe(&response);
        self.attempts[i].raw_text = response
            .text()
            .map(|s| s.chars().take(2 * 1024 * 1024).collect());
        match decode_response::<T>(&response, 2 * 1024 * 1024) {
            Ok(value) => {
                self.attempts[i].status = WorkflowStatus::Completed;
                self.emit(DiscoveryWorkflowEvent::AttemptFinished {
                    attempt: i + 1,
                    stage: self.attempts[i].stage.clone(),
                    status: WorkflowStatus::Completed,
                });
                Ok(value)
            }
            Err(e) => {
                self.attempts[i].status = WorkflowStatus::NeedsAttention;
                self.attempts[i].error = Some(e.clone());
                self.emit(DiscoveryWorkflowEvent::AttemptFinished {
                    attempt: i + 1,
                    stage: self.attempts[i].stage.clone(),
                    status: WorkflowStatus::NeedsAttention,
                });
                Err(e)
            }
        }
    }

    /// Calling again explicitly resumes the same in-memory run; successful tasks
    /// and the frozen A result are retained. Budgets count all attempts.
    pub async fn run(&mut self, prepared: PreparedUnitDiscoveryRequest) -> WorkflowStatus {
        self.emit(DiscoveryWorkflowEvent::WorkflowStarted);
        if self.limits.max_tasks_per_batch == 0
            || self.limits.max_calls == 0
            || self.limits.max_batches == 0
            || self.limits.timeout.is_zero()
        {
            return self.finish(WorkflowStatus::Failed);
        }
        for a in &mut self.attempts {
            if a.status == WorkflowStatus::Running {
                a.status = WorkflowStatus::Cancelled;
            }
        }
        let deadline = Instant::now() + self.limits.timeout;
        if self
            .source_input
            .as_ref()
            .is_some_and(|s| s.fingerprint != prepared.input().fingerprint)
        {
            self.events
                .push("source changed; create a separate workflow".into());
            return self.finish(WorkflowStatus::NeedsAttention);
        }
        self.source_input = Some(prepared.input().clone());
        self.status = WorkflowStatus::Running;
        if self
            .facts
            .values()
            .any(ValidatedFactTask::requires_unit_revision)
        {
            return self.finish(WorkflowStatus::NeedsAttention);
        }
        // An explicit resume retries pending work, not already validated facts.
        self.facts.retain(|_, task| task.processed());
        if let Some(units) = &self.units {
            if prepared
                .revalidate_checkpoint(&units.checkpoint(), Default::default())
                .is_err()
            {
                self.events
                    .push("source or UnitSet changed; start a new workflow".into());
                return self.finish(WorkflowStatus::NeedsAttention);
            }
        } else {
            let raw = match self
                .call::<UnitDiscoveryProposal>(
                    prepared.request().clone(),
                    prepared.prompt_fingerprint().clone(),
                    None,
                    None,
                    false,
                    deadline,
                )
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    self.events.push(e);
                    return self.finish(WorkflowStatus::Failed);
                }
            };
            self.events.push("validate units".into());
            self.emit(DiscoveryWorkflowEvent::ValidationStarted {
                stage: "unit-discovery".into(),
                batch_index: None,
                batch_total: None,
            });
            match prepared.validate(raw.clone(), Default::default()) {
                Ok(units) => self.units = Some(units),
                Err(report) => {
                    self.diagnostics.push(report.clone());
                    if !self.repairs.insert("units".into()) {
                        return self.finish(WorkflowStatus::NeedsAttention);
                    }
                    let repair_ref = new_request_ref("repair");
                    let input = DiscoveryRepairInput {
                        version: DiscoveryRepairVersion::V1,
                        repair_ref: repair_ref.clone(),
                        scope: RepairScope::Units {},
                        blocks: prepared.model_input().blocks.clone(),
                        issues: repair_issues(&report),
                    };
                    let (request, prompt) = repair_request(
                        &input,
                        &raw,
                        &prepared.model_input(),
                        prepared.request_fingerprint(),
                    );
                    let response = match self
                        .call::<DiscoveryRepairResponse>(
                            request, prompt, None, None, true, deadline,
                        )
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            self.events.push(e);
                            return self.finish(WorkflowStatus::NeedsAttention);
                        }
                    };
                    if response.repair_ref != repair_ref || response.actions.len() != 1 {
                        return self.finish(WorkflowStatus::NeedsAttention);
                    }
                    let Some(DiscoveryRepairAction::ReplaceUnits { proposal }) =
                        response.actions.into_iter().next()
                    else {
                        return self.finish(WorkflowStatus::NeedsAttention);
                    };
                    match prepared.validate(*proposal, Default::default()) {
                        Ok(u) => self.units = Some(u),
                        Err(r) => {
                            self.diagnostics.push(r);
                            return self.finish(WorkflowStatus::NeedsAttention);
                        }
                    }
                }
            }
        }
        let units = self.units.as_ref().expect("validated units");
        self.emit(DiscoveryWorkflowEvent::UnitCheckpointCreated {
            units: units.units().len(),
            partial: units.is_partial(),
        });
        if self.batches.is_empty() {
            match plan_source_facts(
                &prepared,
                self.units.as_ref().unwrap(),
                self.limits.max_tasks_per_batch,
            ) {
                Ok(batches) => {
                    if batches.len() > self.limits.max_batches {
                        self.events.push("batch budget exceeded".into());
                        return self.finish(WorkflowStatus::Partial);
                    }
                    self.batches = batches;
                    self.emit(DiscoveryWorkflowEvent::FactPlanCreated {
                        batches: self.batches.len(),
                        tasks: self
                            .batches
                            .iter()
                            .flat_map(|batch| &batch.model.tasks)
                            .count(),
                    });
                }
                Err(r) => {
                    self.diagnostics.push(r);
                    return self.finish(WorkflowStatus::NeedsAttention);
                }
            }
        }
        let batch_total = self.batches.len();
        for (batch_offset, batch) in self.batches.clone().into_iter().enumerate() {
            let batch_position = Some((batch_offset + 1, batch_total));
            if batch
                .model
                .tasks
                .iter()
                .all(|t| self.facts.contains_key(&t.task_ref))
            {
                continue;
            }
            let batch = batch.pending_only(&self.facts.keys().cloned().collect());
            if Instant::now() >= deadline {
                self.events.push("deadline reached between batches".into());
                break;
            }
            let raw = match self
                .call::<SourceFactProposal>(
                    batch.request.clone(),
                    batch.prompt_fingerprint.clone(),
                    Some(batch.batch_fingerprint.clone()),
                    batch_position,
                    false,
                    deadline,
                )
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    self.events.push(e);
                    break;
                }
            };
            self.events.push("validate source facts".into());
            self.emit(DiscoveryWorkflowEvent::ValidationStarted {
                stage: "source-facts".into(),
                batch_index: batch_position.map(|position| position.0),
                batch_total: batch_position.map(|position| position.1),
            });
            let validation = batch.validate(&raw);
            for task in &validation.tasks {
                self.facts.insert(task.task_ref.clone(), task.clone());
            }
            if validation
                .tasks
                .iter()
                .any(ValidatedFactTask::requires_unit_revision)
            {
                self.events.push("requires_unit_revision".into());
                return self.finish(WorkflowStatus::NeedsAttention);
            }
            let (completed_tasks, total_tasks) = self.fact_progress();
            self.emit(DiscoveryWorkflowEvent::FactCheckpointUpdated {
                completed_tasks,
                total_tasks,
            });
            if validation.report.is_valid() {
                continue;
            }
            self.diagnostics.push(validation.report.clone());
            let repair_key = batch.batch_fingerprint.to_string();
            if !self.repairs.insert(repair_key) {
                continue;
            }
            let failed: Vec<_> = batch
                .model
                .tasks
                .iter()
                .filter(|t| !self.facts.contains_key(&t.task_ref))
                .map(|t| t.task_ref.clone())
                .collect();
            let repair_ref = new_request_ref("repair");
            let mut input = DiscoveryRepairInput {
                version: DiscoveryRepairVersion::V1,
                repair_ref: repair_ref.clone(),
                scope: RepairScope::Facts {
                    task_refs: failed.clone(),
                },
                blocks: batch.model.blocks.clone(),
                issues: repair_issues(&validation.report),
            };
            for issue in &mut input.issues {
                if let Some(choice) = validation.choices.iter().find(|c| c.path == issue.path) {
                    issue.candidates = choice
                        .set
                        .candidates()
                        .iter()
                        .map(|c| RepairCandidate {
                            candidate_ref: c.candidate_ref.clone(),
                            text: c.text.clone(),
                            prefix: c.prefix.clone(),
                            suffix: c.suffix.clone(),
                        })
                        .collect();
                    issue.has_more = choice.set.has_more();
                }
            }
            let (request, prompt) =
                repair_request(&input, &raw, &batch.model, &batch.batch_fingerprint);
            let response = match self
                .call::<DiscoveryRepairResponse>(
                    request,
                    prompt,
                    Some(batch.batch_fingerprint.clone()),
                    batch_position,
                    true,
                    deadline,
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    self.events.push(e);
                    break;
                }
            };
            match apply_fact_repair(&batch, &raw, &input, &validation, &response, &failed) {
                Ok(repaired) => {
                    let validated = batch.validate(&repaired);
                    for t in validated.tasks {
                        if failed.contains(&t.task_ref) {
                            self.facts.insert(t.task_ref.clone(), t);
                        }
                    }
                    if !validated.report.is_valid() {
                        self.diagnostics.push(validated.report);
                    }
                }
                Err(e) => {
                    let mut r = StageValidationReport::default();
                    r.push("invalid_repair", "repair", e);
                    self.diagnostics.push(r);
                }
            }
            if self
                .facts
                .values()
                .any(ValidatedFactTask::requires_unit_revision)
            {
                return self.finish(WorkflowStatus::NeedsAttention);
            }
            let (completed_tasks, total_tasks) = self.fact_progress();
            self.emit(DiscoveryWorkflowEvent::FactCheckpointUpdated {
                completed_tasks,
                total_tasks,
            });
        }
        self.events.push("assemble discovery".into());
        self.emit(DiscoveryWorkflowEvent::AssemblyStarted);
        let status = match self.result() {
            Some(r) if r.complete() => WorkflowStatus::Completed,
            Some(_) => WorkflowStatus::Partial,
            None => WorkflowStatus::NeedsAttention,
        };
        self.finish(status)
    }
}

fn repair_issues(report: &StageValidationReport) -> Vec<RepairIssue> {
    report
        .diagnostics
        .iter()
        .enumerate()
        .map(|(i, d)| RepairIssue {
            issue_ref: format!("i{}", i + 1),
            path: d.path.clone(),
            code: d.code.clone(),
            message: d.message.clone(),
            candidates: vec![],
            has_more: false,
        })
        .collect()
}

fn repair_request(
    input: &DiscoveryRepairInput,
    original: &impl Serialize,
    context: &impl Serialize,
    base: &ContentHash,
) -> (LlmRequest, ContentHash) {
    let schema = repair_output_schema();
    let system = include_str!("../../../prompts/discovery-repair/v1/system.md");
    let manifest: StagePromptManifest = serde_json::from_str(include_str!(
        "../../../prompts/discovery-repair/v1/manifest.json"
    ))
    .expect("repair manifest");
    let prompt = fingerprint(&(&manifest, system, &schema));
    let user = serde_json::json!({"repair":input,"original":original,"context":context});
    let request = LlmRequest {
        task: LlmTask {
            kind: manifest.task_kind,
            version: manifest.prompt_revision,
            input_fingerprint: Some(fingerprint(&(base, &user, &prompt))),
        },
        messages: vec![
            LlmMessage::text(LlmRole::System, format!("{system}\nJSON Schema:\n{schema}")),
            LlmMessage::text(LlmRole::User, user.to_string()),
        ],
        tools: vec![],
        tool_choice: LlmToolChoice::None,
        response_format: LlmResponseFormat::JsonSchema {
            name: manifest.schema_name,
            description: None,
            schema,
        },
        options: LlmGenerationOptions {
            max_output_tokens: Some(manifest.max_output_tokens),
            ..Default::default()
        },
    };
    (request, prompt)
}

fn apply_fact_repair(
    batch: &PreparedFactRequest,
    raw: &SourceFactProposal,
    input: &DiscoveryRepairInput,
    validation: &FactBatchValidation,
    response: &DiscoveryRepairResponse,
    failed: &[String],
) -> Result<SourceFactProposal, String> {
    if response.repair_ref != input.repair_ref {
        return Err("stale repair response".into());
    }
    let mut result = raw.clone();
    let mut touched = HashSet::new();
    let mut replaced = HashSet::new();
    let mut selected = HashSet::new();
    let mut deferred = HashSet::new();
    let base = fingerprint(raw);
    for action in &response.actions {
        match action {
            DiscoveryRepairAction::ReplaceTask { task_ref, proposal } => {
                if !failed.contains(task_ref)
                    || proposal.task_ref != *task_ref
                    || touched.contains(task_ref)
                    || deferred.contains(task_ref)
                    || !replaced.insert(task_ref.clone())
                {
                    return Err("conflicting or unauthorized task replacement".into());
                }
                if let Some(t) = result.tasks.iter_mut().find(|t| t.task_ref == *task_ref) {
                    *t = (**proposal).clone();
                } else {
                    result.tasks.push((**proposal).clone());
                }
            }
            DiscoveryRepairAction::ChooseCandidate {
                issue_ref,
                candidate_ref,
            } => {
                if !selected.insert(issue_ref) {
                    return Err("duplicate candidate decision".into());
                }
                let issue = input
                    .issues
                    .iter()
                    .find(|i| i.issue_ref == *issue_ref)
                    .ok_or("unknown repair issue")?;
                let choice = validation
                    .choices
                    .iter()
                    .find(|c| c.path == issue.path)
                    .ok_or("issue has no selectable candidates")?;
                if !failed.contains(&choice.task_ref)
                    || replaced.contains(&choice.task_ref)
                    || deferred.contains(&choice.task_ref)
                {
                    return Err("conflicting candidate decision".into());
                }
                touched.insert(choice.task_ref.clone());
                let locator = batch.scopes[&choice.task_ref]
                    .locator
                    .as_ref()
                    .ok_or("no extraction scope")?;
                let location = locator
                    .choose_candidate(&choice.set, &base, &choice.path, candidate_ref)
                    .map_err(|e| e.to_string())?;
                let DiscoverySelector::QuoteRange { first, start, .. } =
                    location.selector().selector()
                else {
                    return Err("unexpected candidate shape".into());
                };
                let resolver = SelectorResolver::from_input(&batch.input);
                let block = resolver
                    .bindings()
                    .iter()
                    .find(|b| b.block_id() == first)
                    .ok_or("candidate block missing")?
                    .handle()
                    .to_owned();
                let anchor = TextOccurrence::TextOccurrence {
                    block,
                    text: start.quote.clone(),
                    occurrence: start.occurrence,
                };
                let task = result
                    .tasks
                    .iter_mut()
                    .find(|t| t.task_ref == choice.task_ref)
                    .ok_or("missing task")?;
                let suffix = choice
                    .path
                    .strip_prefix(&format!("tasks.{}.", choice.task_ref))
                    .ok_or("invalid candidate path")?;
                if suffix == "source_title" {
                    task.source_title = Some(anchor);
                } else {
                    let (field, tail) = suffix.split_once('[').ok_or("invalid field")?;
                    let index: usize = tail
                        .split(']')
                        .next()
                        .unwrap()
                        .parse()
                        .map_err(|_| "invalid index")?;
                    match field {
                        "source_labels" => {
                            task.source_labels
                                .get_mut(index)
                                .ok_or("missing label")?
                                .anchor = anchor
                        }
                        "references" => {
                            task.references
                                .get_mut(index)
                                .ok_or("missing reference")?
                                .anchor = anchor
                        }
                        _ => return Err("unsupported candidate field".into()),
                    }
                }
            }
            DiscoveryRepairAction::Defer { issue_ref, reason } => {
                if reason.trim().is_empty()
                    || !input.issues.iter().any(|i| i.issue_ref == *issue_ref)
                    || !selected.insert(issue_ref)
                {
                    return Err("invalid defer".into());
                }
                let issue = input
                    .issues
                    .iter()
                    .find(|i| i.issue_ref == *issue_ref)
                    .expect("checked issue");
                let task = issue
                    .path
                    .strip_prefix("tasks.")
                    .and_then(|s| s.split('.').next())
                    .ok_or("batch-level issue deferred")?;
                if replaced.contains(task) || touched.contains(task) {
                    return Err("defer conflicts with task repair".into());
                }
                deferred.insert(task.to_owned());
            }
            _ => return Err("wrong-stage repair action".into()),
        }
    }
    Ok(result)
}
