use crate::discovery_producer::{collect_text, reject_finish_reason};
use crate::unit_discovery::fingerprint;
use crate::{
    DiscoveryRunSummary, LocatedOccurrence, OccurrenceCandidates, OccurrenceErrorCode,
    PreparedFactRequest, ResolvedDiscoverySelector, StageValidationReport, ValidatedUnitSet,
    preflight_source_facts,
};
use mathscape_domain::*;
use mathscape_llm::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Serialize)]
pub struct LocatedFact {
    pub fact_ref: String,
    pub location: ResolvedDiscoverySelector,
    pub normalized: bool,
    pub precision_review: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct LocatedReference {
    pub fact: LocatedFact,
    pub target_hint: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct LocatedRelation {
    pub fact_ref: String,
    pub target: String,
    pub relation: DiscoveryRelationKind,
    pub evidence: Vec<ResolvedDiscoverySelector>,
    pub basis: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ValidatedFactTask {
    pub(crate) task_ref: String,
    pub(crate) owner: String,
    pub(crate) unit_set_fingerprint: ContentHash,
    pub(crate) batch_fingerprint: ContentHash,
    pub(crate) original: FactTaskProposal,
    pub(crate) labels: Vec<LocatedFact>,
    pub(crate) title: Option<LocatedFact>,
    pub(crate) references: Vec<LocatedReference>,
    pub(crate) relations: Vec<LocatedRelation>,
    pub(crate) issues: Vec<(FactIssue, Vec<ResolvedDiscoverySelector>)>,
}
impl ValidatedFactTask {
    pub fn task_ref(&self) -> &str {
        &self.task_ref
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn labels(&self) -> &[LocatedFact] {
        &self.labels
    }
    pub fn title(&self) -> Option<&LocatedFact> {
        self.title.as_ref()
    }
    pub fn references(&self) -> &[LocatedReference] {
        &self.references
    }
    pub fn relations(&self) -> &[LocatedRelation] {
        &self.relations
    }
    pub fn issues(&self) -> &[(FactIssue, Vec<ResolvedDiscoverySelector>)] {
        &self.issues
    }
    pub fn requires_unit_revision(&self) -> bool {
        self.original
            .issues
            .iter()
            .any(|i| i.requires_unit_revision)
    }
    pub fn processed(&self) -> bool {
        matches!(self.original.disposition, FactDisposition::Processed {})
    }
}

pub struct FactCandidateChoice {
    pub(crate) task_ref: String,
    pub(crate) path: String,
    pub(crate) set: OccurrenceCandidates,
}
pub struct FactBatchValidation {
    pub tasks: Vec<ValidatedFactTask>,
    pub report: StageValidationReport,
    pub(crate) choices: Vec<FactCandidateChoice>,
}

fn located(id: &str, value: LocatedOccurrence) -> LocatedFact {
    LocatedFact {
        fact_ref: id.into(),
        location: value.selector().clone(),
        normalized: value.normalized(),
        precision_review: value.precision_review(),
    }
}

impl PreparedFactRequest {
    pub fn validate(&self, proposal: &SourceFactProposal) -> FactBatchValidation {
        let mut result = FactBatchValidation {
            tasks: vec![],
            report: StageValidationReport::default(),
            choices: vec![],
        };
        let item_count: usize = proposal
            .tasks
            .iter()
            .map(|t| {
                t.source_labels.len()
                    + t.references.len()
                    + t.relations.len()
                    + t.issues.len()
                    + usize::from(t.source_title.is_some())
            })
            .sum();
        let evidence_count: usize = proposal
            .tasks
            .iter()
            .map(|t| {
                t.relations.iter().map(|r| r.evidence.len()).sum::<usize>()
                    + t.issues.iter().map(|i| i.evidence.len()).sum::<usize>()
            })
            .sum();
        if item_count + evidence_count > 4096
            || proposal.tasks.iter().any(|t| {
                [
                    t.source_labels.len(),
                    t.references.len(),
                    t.relations.len(),
                    t.issues.len(),
                ]
                .iter()
                .any(|n| *n > 1024)
            })
            || proposal.tasks.len() > 1024
            || serde_json::to_vec(proposal).map_or(usize::MAX, |b| b.len()) > 2 * 1024 * 1024
        {
            result
                .report
                .push("limit_exceeded", "proposal", "facts exceed limits");
            return result;
        }
        // Envelope errors invalidate the batch; individual task errors do not discard siblings.
        let seen: HashSet<_> = proposal.tasks.iter().map(|t| t.task_ref.as_str()).collect();
        if proposal.request_ref != self.model.request_ref
            || seen.len() != proposal.tasks.len()
            || seen.iter().any(|t| !self.scopes.contains_key(*t))
        {
            result.report.push(
                "invalid_envelope",
                "proposal",
                "wrong request, duplicate or unknown task",
            );
            return result;
        }
        let base = fingerprint(proposal);
        for assigned in &self.model.tasks {
            let Some(task) = proposal
                .tasks
                .iter()
                .find(|t| t.task_ref == assigned.task_ref)
            else {
                result.report.push(
                    "missing_task",
                    format!("tasks.{}", assigned.task_ref),
                    "task omitted",
                );
                continue;
            };
            let mut input = self.model.clone();
            input.tasks = vec![assigned.clone()];
            let mut report = preflight_source_facts(
                &input,
                &SourceFactProposal {
                    version: proposal.version,
                    request_ref: proposal.request_ref.clone(),
                    tasks: vec![task.clone()],
                },
            );
            report.not_checked.clear();
            let scope = &self.scopes[&task.task_ref];
            let prefix = format!("tasks.{}", task.task_ref);
            let mut choices = vec![];
            let mut resolve = |id: &str,
                               anchor: &TextOccurrence,
                               path: String,
                               report: &mut StageValidationReport|
             -> Option<LocatedFact> {
                let Some(locator) = &scope.locator else {
                    report.push(
                        "unauthorized_range",
                        path,
                        "owner has no direct extraction region",
                    );
                    return None;
                };
                match locator.resolve(anchor) {
                    Ok(value) => Some(located(id, value)),
                    Err(error) => {
                        if error.code == OccurrenceErrorCode::AmbiguousQuote
                            && let Ok(set) = locator.candidates(anchor, &base, &path)
                        {
                            choices.push(FactCandidateChoice {
                                task_ref: task.task_ref.clone(),
                                path: path.clone(),
                                set,
                            });
                        }
                        let code = serde_json::to_value(error.code)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned();
                        report.push(&code, path, error.message);
                        None
                    }
                }
            };
            let labels = task
                .source_labels
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    resolve(
                        &format!("l{}", i + 1),
                        &f.anchor,
                        format!("{prefix}.source_labels[{i}].anchor"),
                        &mut report,
                    )
                })
                .collect();
            let title = task
                .source_title
                .as_ref()
                .and_then(|a| resolve("title", a, format!("{prefix}.source_title"), &mut report));
            let references = task
                .references
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    if f.target_hint.is_some() {
                        return resolve(
                            &format!("r{}", i + 1),
                            &f.anchor,
                            format!("{prefix}.references[{i}].anchor"),
                            &mut report,
                        )
                        .map(|fact| LocatedReference {
                            fact,
                            target_hint: f.target_hint.clone(),
                        });
                    }
                    let path = format!("{prefix}.references[{i}].anchor");
                    let Some(locator) = &scope.locator else {
                        report.push(
                            "unauthorized_range",
                            &path,
                            "owner has no direct extraction region",
                        );
                        return None;
                    };
                    match locator.resolve(&f.anchor) {
                        Ok(value) => Some(LocatedReference {
                            fact: located(&format!("r{}", i + 1), value),
                            target_hint: f.target_hint.clone(),
                        }),
                        Err(error) if error.code == OccurrenceErrorCode::AmbiguousQuote => {
                            // A repeated external citation is useful as an
                            // audit warning, but choosing one occurrence is
                            // not worth blocking the complete task. Do not
                            // create a potentially false source edge.
                            report.warn(
                                "ambiguous_reference",
                                path,
                                format!(
                                    "{}; reference retained as unresolved mention",
                                    error.message
                                ),
                            );
                            None
                        }
                        Err(error) => {
                            let code = serde_json::to_value(error.code)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned();
                            report.push(&code, path, error.message);
                            None
                        }
                    }
                })
                .collect();
            let mut relations = vec![];
            let mut relation_keys = HashSet::new();
            for (i, f) in task.relations.iter().enumerate() {
                if !relation_keys.insert((format!("{:?}", f.relation), &f.target)) {
                    report.push(
                        "duplicate_relation",
                        format!("{prefix}.relations[{i}]"),
                        "duplicate relationship",
                    );
                }
                let mut evidence = vec![];
                for s in &f.evidence {
                    match self.range(s, &task.task_ref) {
                        Ok(r) => evidence.push(r),
                        Err(e) => report.push(
                            "invalid_evidence",
                            format!("{prefix}.relations[{i}].evidence"),
                            e,
                        ),
                    }
                }
                relations.push(LocatedRelation {
                    fact_ref: format!("rel{}", i + 1),
                    target: f.target.clone(),
                    relation: f.relation,
                    evidence,
                    basis: f.basis.clone(),
                });
            }
            let mut issues = vec![];
            for (i, issue) in task.issues.iter().enumerate() {
                if issue.observation.trim().is_empty() {
                    report.push(
                        "invalid_value",
                        format!("{prefix}.issues[{i}]"),
                        "blank observation",
                    );
                }
                if let Some(DiscoveryContextRequest::LocalReference { target_label }) =
                    &issue.context_request
                    && target_label.trim().is_empty()
                {
                    report.push(
                        "invalid_value",
                        format!("{prefix}.issues[{i}]"),
                        "blank context target",
                    );
                }
                let mut evidence = vec![];
                for s in &issue.evidence {
                    match self.range(s, &task.task_ref) {
                        Ok(r) => evidence.push(r),
                        Err(e) => report.push(
                            "invalid_evidence",
                            format!("{prefix}.issues[{i}].evidence"),
                            e,
                        ),
                    }
                }
                issues.push((issue.clone(), evidence));
            }
            for d in &mut report.diagnostics {
                if d.path.starts_with("tasks[0]") {
                    d.path = d.path.replacen("tasks[0]", &prefix, 1);
                }
            }
            if report.is_valid() {
                result.report.diagnostics.extend(
                    report
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.code.starts_with("warning_"))
                        .cloned(),
                );
                result.tasks.push(ValidatedFactTask {
                    task_ref: task.task_ref.clone(),
                    owner: scope.owner.clone(),
                    unit_set_fingerprint: self.unit_set_fingerprint.clone(),
                    batch_fingerprint: self.batch_fingerprint.clone(),
                    original: task.clone(),
                    labels,
                    title,
                    references,
                    relations,
                    issues,
                });
            } else {
                result.report.diagnostics.extend(report.diagnostics);
                result.choices.extend(choices);
            }
        }
        result
    }
}

pub(crate) fn decode_response<T: DeserializeOwned + Serialize>(
    response: &LlmResponse,
    limit: usize,
) -> Result<T, String> {
    if let Some(e) = reject_finish_reason(response.finish_reason.as_ref()) {
        return Err(e.to_string());
    }
    let text = collect_text(response, limit).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let parsed: T = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&parsed).map_err(|e| e.to_string())? != value {
        return Err("noncanonical JSON".into());
    }
    Ok(parsed)
}

pub enum FactProductionOutcome {
    Received {
        proposal: SourceFactProposal,
        validation: FactBatchValidation,
        run: DiscoveryRunSummary,
    },
    InvalidResponse {
        message: String,
        run: DiscoveryRunSummary,
    },
    Failed {
        error: LlmError,
        run: DiscoveryRunSummary,
    },
}
pub(crate) fn pending_run(
    backend: &dyn LlmBackend,
    request: &LlmRequest,
    prompt: &ContentHash,
) -> DiscoveryRunSummary {
    let p = backend.profile();
    DiscoveryRunSummary {
        task_kind: request.task.kind.clone(),
        task_version: request.task.version.clone(),
        input_fingerprint: request
            .task
            .input_fingerprint
            .clone()
            .expect("bound request"),
        prompt_fingerprint: prompt.clone(),
        profile_id: p.id.clone(),
        provider: p.provider.clone(),
        requested_model: p.model.clone(),
        response: None,
    }
}
pub struct SourceFactProducer<'a> {
    backend: &'a dyn LlmBackend,
}
impl<'a> SourceFactProducer<'a> {
    pub fn new(backend: &'a dyn LlmBackend) -> Self {
        Self { backend }
    }
    pub async fn produce(&self, prepared: &PreparedFactRequest) -> FactProductionOutcome {
        let mut run = pending_run(
            self.backend,
            &prepared.request,
            &prepared.prompt_fingerprint,
        );
        match self.backend.execute_stream(prepared.request.clone()).await {
            Err(error) => FactProductionOutcome::Failed { error, run },
            Ok(response) => {
                run.observe(&response);
                match decode_response::<SourceFactProposal>(&response, 2 * 1024 * 1024) {
                    Ok(proposal) => {
                        let validation = prepared.validate(&proposal);
                        FactProductionOutcome::Received {
                            proposal,
                            validation,
                            run,
                        }
                    }
                    Err(message) => FactProductionOutcome::InvalidResponse { message, run },
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceResolutionStatus {
    Unresolved,
    Candidate,
    ResolvedLocal,
    Ambiguous,
}
#[derive(Clone, Debug, Serialize)]
pub struct ReferenceResolution {
    pub status: ReferenceResolutionStatus,
    pub candidates: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct AssembledReference {
    pub owner: String,
    pub location: ResolvedDiscoverySelector,
    pub origins: Vec<String>,
    pub resolution: ReferenceResolution,
}
#[derive(Clone, Debug, Serialize)]
pub struct AssembledRelation {
    pub source: String,
    pub target: String,
    pub relation: DiscoveryRelationKind,
    pub evidence: Vec<ResolvedDiscoverySelector>,
    pub origins: Vec<String>,
    pub bases: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct DiscoveryResult {
    pub(crate) version: DiscoveryResultVersion,
    pub(crate) units: UnitDiscoveryCheckpoint,
    pub(crate) tasks: Vec<ValidatedFactTask>,
    pub(crate) references: Vec<AssembledReference>,
    pub(crate) relations: Vec<AssembledRelation>,
    pub(crate) pending_tasks: Vec<String>,
    pub(crate) pending_task_owners: BTreeMap<String, String>,
    pub(crate) complete: bool,
}
impl DiscoveryResult {
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn units(&self) -> &UnitDiscoveryCheckpoint {
        &self.units
    }
    pub fn tasks(&self) -> &[ValidatedFactTask] {
        &self.tasks
    }
    pub fn pending_tasks(&self) -> &[String] {
        &self.pending_tasks
    }
    pub fn pending_task_owner(&self, task_ref: &str) -> Option<&str> {
        self.pending_task_owners.get(task_ref).map(String::as_str)
    }
    pub fn references(&self) -> &[AssembledReference] {
        &self.references
    }
    pub fn relations(&self) -> &[AssembledRelation] {
        &self.relations
    }
}

fn label_key(text: &str) -> Option<(String, Option<DiscoveryUnitKind>)> {
    let mut t = text.trim();
    let mut kind = None;
    for (prefix, k) in [
        ("定义", Some(DiscoveryUnitKind::Definition)),
        ("定理", Some(DiscoveryUnitKind::Statement)),
        ("命题", Some(DiscoveryUnitKind::Statement)),
        ("式", None),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim();
            kind = k;
            break;
        }
    }
    let bare = t
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(t);
    if bare.is_empty()
        || !bare.chars().next().unwrap().is_ascii_digit()
        || !bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
    {
        return None;
    }
    Some((t.into(), kind))
}
fn section(units: &ValidatedUnitSet, id: &str) -> Option<String> {
    let mut current = Some(id);
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            return None;
        }
        let u = units.units().iter().find(|u| u.proposal().unit_ref == id)?;
        if u.proposal().unit_kind == DiscoveryUnitKind::Section {
            return Some(id.into());
        }
        current = u.proposal().parent.as_deref();
    }
    None
}

fn relation_kind_order(kind: DiscoveryRelationKind) -> u8 {
    match kind {
        DiscoveryRelationKind::Proves => 0,
        DiscoveryRelationKind::Solves => 1,
        DiscoveryRelationKind::Hints => 2,
        DiscoveryRelationKind::Explains => 3,
        DiscoveryRelationKind::Cites => 4,
        DiscoveryRelationKind::AppliesMethod => 5,
    }
}

pub fn assemble_discovery(
    units: &ValidatedUnitSet,
    batches: &[PreparedFactRequest],
    mut tasks: Vec<ValidatedFactTask>,
) -> Result<DiscoveryResult, String> {
    if batches.iter().any(|b| {
        b.unit_set_fingerprint != *units.fingerprint()
            || b.source_fingerprint != *units.input_fingerprint()
    }) {
        return Err("stale fact plan".into());
    }
    let mut seen = HashSet::new();
    for t in &tasks {
        if t.unit_set_fingerprint != *units.fingerprint()
            || !seen.insert(&t.task_ref)
            || !batches.iter().any(|b| {
                b.batch_fingerprint == t.batch_fingerprint && b.scopes.contains_key(&t.task_ref)
            })
        {
            return Err("stale or duplicate fact task".into());
        }
    }

    // Fact responses can arrive from independent batches, and a replay may
    // reconstruct them in a different arrival order.  The task collection has
    // no semantic order of its own, so bind it to the planner's stable order
    // (which follows the frozen unit order) before deriving any aggregates.
    // Do not sort the arrays inside a task: source labels, references, issues,
    // relation evidence, and other occurrence lists retain their source/model
    // order because that order can carry mathematical meaning.
    let task_order: HashMap<String, usize> = batches
        .iter()
        .flat_map(|batch| batch.model.tasks.iter())
        .enumerate()
        .map(|(index, task)| (task.task_ref.clone(), index))
        .collect();
    tasks.sort_by_key(|task| {
        task_order
            .get(&task.task_ref)
            .copied()
            .expect("validated fact task has a planned order")
    });
    let pending_tasks: Vec<_> = batches
        .iter()
        .flat_map(|b| &b.model.tasks)
        .filter(|plan| {
            !tasks.iter().any(|t| {
                t.task_ref == plan.task_ref && t.processed() && !t.requires_unit_revision()
            })
        })
        .map(|t| t.task_ref.clone())
        .collect();
    let pending_task_owners = batches
        .iter()
        .flat_map(|batch| &batch.model.tasks)
        .filter(|task| pending_tasks.contains(&task.task_ref))
        .map(|task| (task.task_ref.clone(), task.owner_unit_ref.clone()))
        .collect();
    let mut references: Vec<AssembledReference> = vec![];
    let mut relations: Vec<AssembledRelation> = vec![];
    for task in &tasks {
        for r in &task.references {
            let mut candidates = vec![];
            let mut scoped = vec![];
            let mut declarations = HashSet::new();
            let key = label_key(r.fact.location.text());
            if let Some((key, kind)) = &key {
                for target in &tasks {
                    let u = units
                        .units()
                        .iter()
                        .find(|u| u.proposal().unit_ref == target.owner)
                        .expect("validated owner");
                    if kind.is_some_and(|k| u.proposal().unit_kind != k) {
                        continue;
                    }
                    let matching: Vec<_> = target
                        .labels
                        .iter()
                        .filter(|l| label_key(l.location.text()).is_some_and(|(k, _)| k == *key))
                        .collect();
                    if !matching.is_empty() {
                        for label in matching {
                            declarations.insert((
                                target.owner.clone(),
                                label.location.range().start,
                                label.location.range().end,
                            ));
                        }
                        candidates.push(target.owner.clone());
                        if let Some(s) = section(units, &task.owner)
                            && section(units, &target.owner) == Some(s)
                        {
                            scoped.push(target.owner.clone());
                        }
                    }
                }
            }
            candidates.sort();
            candidates.dedup();
            scoped.sort();
            scoped.dedup();
            let (status, reason) = if scoped.len() == 1
                && scoped[0] != task.owner
                && candidates.len() == 1
                && declarations.len() == 1
            {
                (
                    ReferenceResolutionStatus::ResolvedLocal,
                    "unique declaration in explicit section scope",
                )
            } else if candidates.len() > 1 || declarations.len() > 1 {
                (
                    ReferenceResolutionStatus::Ambiguous,
                    "multiple declaration candidates",
                )
            } else if candidates.first() == Some(&task.owner) {
                (
                    ReferenceResolutionStatus::Unresolved,
                    "internal unit reference",
                )
            } else if !candidates.is_empty() {
                (
                    ReferenceResolutionStatus::Candidate,
                    "numbering scope not established",
                )
            } else {
                (
                    ReferenceResolutionStatus::Unresolved,
                    if key.is_some() {
                        "no local declaration"
                    } else {
                        "unsupported or composite reference"
                    },
                )
            };
            if let Some(h) = &r.target_hint
                && !candidates.contains(h)
            {
                candidates.push(h.clone());
            }
            // Candidate references form a set.  Keep their value independent
            // of the order in which local declarations and hints were seen.
            candidates.sort();
            candidates.dedup();
            let status =
                if status == ReferenceResolutionStatus::ResolvedLocal && candidates.len() > 1 {
                    ReferenceResolutionStatus::Ambiguous
                } else {
                    if status == ReferenceResolutionStatus::Unresolved && r.target_hint.is_some() {
                        ReferenceResolutionStatus::Candidate
                    } else {
                        status
                    }
                };
            let origin = format!("{}:{}", task.task_ref, r.fact.fact_ref);
            if let Some(existing) = references
                .iter_mut()
                .find(|x| x.owner == task.owner && x.location.range() == r.fact.location.range())
            {
                existing.origins.push(origin);
                if existing.resolution.candidates != candidates
                    || existing.resolution.status != status
                {
                    existing.resolution.candidates.extend(candidates);
                    existing.resolution.candidates.sort();
                    existing.resolution.candidates.dedup();
                    existing.resolution.status = ReferenceResolutionStatus::Ambiguous;
                    existing.resolution.reason =
                        "conflicting interpretations of the same occurrence".into();
                }
            } else {
                references.push(AssembledReference {
                    owner: task.owner.clone(),
                    location: r.fact.location.clone(),
                    origins: vec![origin],
                    resolution: ReferenceResolution {
                        status,
                        candidates,
                        reason: reason.into(),
                    },
                });
            }
        }
        for r in &task.relations {
            merge_relation(
                &mut relations,
                &task.owner,
                &r.target,
                r.relation,
                &r.evidence,
                format!("{}:{}", task.task_ref, r.fact_ref),
                r.basis.clone(),
            );
        }
    }
    for r in &references {
        if r.resolution.status == ReferenceResolutionStatus::ResolvedLocal {
            merge_relation(
                &mut relations,
                &r.owner,
                &r.resolution.candidates[0],
                DiscoveryRelationKind::Cites,
                std::slice::from_ref(&r.location),
                r.origins.join(","),
                "由唯一解析的局部来源引用生成。".into(),
            );
        }
    }

    // These are projections over the ordered task stream, not proof traces:
    // their outer order is presentation/serialization order.  Canonicalize
    // it using source-unit order and source offsets, while leaving each
    // occurrence/evidence vector untouched.
    let unit_order: HashMap<String, usize> = units
        .units()
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.proposal().unit_ref.clone(), index))
        .collect();
    for reference in &mut references {
        reference.origins.sort();
        reference.origins.dedup();
        reference.resolution.candidates.sort();
        reference.resolution.candidates.dedup();
    }
    references.sort_by(|left, right| {
        unit_order
            .get(&left.owner)
            .copied()
            .unwrap_or(usize::MAX)
            .cmp(&unit_order.get(&right.owner).copied().unwrap_or(usize::MAX))
            .then_with(|| {
                left.location
                    .range()
                    .start
                    .cmp(&right.location.range().start)
            })
            .then_with(|| left.location.range().end.cmp(&right.location.range().end))
    });
    for relation in &mut relations {
        relation.origins.sort();
        relation.origins.dedup();
        relation.bases.sort();
        relation.bases.dedup();
    }
    relations.sort_by(|left, right| {
        unit_order
            .get(&left.source)
            .copied()
            .unwrap_or(usize::MAX)
            .cmp(&unit_order.get(&right.source).copied().unwrap_or(usize::MAX))
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| {
                relation_kind_order(left.relation).cmp(&relation_kind_order(right.relation))
            })
    });
    Ok(DiscoveryResult {
        version: DiscoveryResultVersion::V1,
        complete: !units.is_partial() && pending_tasks.is_empty(),
        units: units.checkpoint(),
        tasks,
        references,
        relations,
        pending_tasks,
        pending_task_owners,
    })
}
fn merge_relation(
    out: &mut Vec<AssembledRelation>,
    source: &str,
    target: &str,
    relation: DiscoveryRelationKind,
    evidence: &[ResolvedDiscoverySelector],
    origin: String,
    basis: String,
) {
    if source == target {
        return;
    }
    if let Some(r) = out
        .iter_mut()
        .find(|r| r.source == source && r.target == target && r.relation == relation)
    {
        r.origins.push(origin);
        if !r.bases.contains(&basis) {
            r.bases.push(basis);
        }
        for e in evidence {
            if !r.evidence.iter().any(|x| x.range() == e.range()) {
                r.evidence.push(e.clone());
            }
        }
    } else {
        out.push(AssembledRelation {
            source: source.into(),
            target: target.into(),
            relation,
            evidence: evidence.to_vec(),
            origins: vec![origin],
            bases: vec![basis],
        });
    }
}
