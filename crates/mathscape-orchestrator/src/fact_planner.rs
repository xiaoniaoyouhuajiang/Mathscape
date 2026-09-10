//! Full-selection reading context with explicit, disjoint extraction ownership.
use crate::discovery_input::discovery_input_fingerprint;
use crate::discovery_validation::{authorize_discovery_input, is_ancestor};
use crate::unit_discovery::{fingerprint, new_request_ref};
use crate::{
    FACT_SYSTEM_PROMPT, OccurrenceResolver, PreparedUnitDiscoveryRequest,
    ResolvedDiscoverySelector, SelectorResolver, StageValidationReport, ValidatedUnitSet,
    fact_output_schema, fact_prompt_manifest,
};
use mathscape_domain::*;
use mathscape_llm::*;
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct FactScope {
    pub owner: String,
    pub entire: Vec<TextByteRange>,
    pub locator: Option<Arc<OccurrenceResolver>>,
}

#[derive(Clone)]
pub struct PreparedFactRequest {
    pub(crate) input: DiscoveryInput,
    pub(crate) model: SourceFactInput,
    pub(crate) scopes: HashMap<String, FactScope>,
    pub(crate) unit_set_fingerprint: ContentHash,
    pub(crate) source_fingerprint: ContentHash,
    pub(crate) request: LlmRequest,
    pub(crate) prompt_fingerprint: ContentHash,
    pub(crate) batch_fingerprint: ContentHash,
}
impl PreparedFactRequest {
    pub fn model_input(&self) -> &SourceFactInput {
        &self.model
    }
    pub fn request(&self) -> &LlmRequest {
        &self.request
    }
    pub fn batch_fingerprint(&self) -> &ContentHash {
        &self.batch_fingerprint
    }
    pub fn unit_set_fingerprint(&self) -> &ContentHash {
        &self.unit_set_fingerprint
    }
    pub fn source_input(&self) -> &DiscoveryInput {
        &self.input
    }
    pub(crate) fn pending_only(&self, completed: &std::collections::HashSet<String>) -> Self {
        let mut pending = self.clone();
        pending
            .model
            .tasks
            .retain(|t| !completed.contains(&t.task_ref));
        pending.scopes.retain(|id, _| !completed.contains(id));
        pending.model.request_ref = new_request_ref("f");
        pending.request.messages[1] = LlmMessage::text(
            LlmRole::User,
            serde_json::to_string(&pending.model).expect("model serializes"),
        );
        pending.request.task.input_fingerprint =
            Some(fingerprint(&(&pending.batch_fingerprint, &pending.model)));
        pending
    }
    pub(crate) fn range(
        &self,
        selector: &DiscoverySelector,
        task: &str,
    ) -> Result<ResolvedDiscoverySelector, String> {
        let resolver = SelectorResolver::from_input(&self.input);
        let (canonical, _) = resolver
            .resolve_selector(selector.clone(), "evidence")
            .map_err(|e| e.to_string())?;
        let mut errors = vec![];
        let region = authorize_discovery_input(&self.input, &mut errors).ok_or("invalid input")?;
        let located = region.resolve(&canonical)?;
        if !covers(
            &self.scopes.get(task).ok_or("unknown task")?.entire,
            located.range(),
        ) {
            return Err("evidence outside owner".into());
        }
        Ok(located)
    }
}

pub(crate) fn covers(ranges: &[TextByteRange], target: &TextByteRange) -> bool {
    let mut cursor = target.start;
    for r in ranges {
        if r.end <= cursor {
            continue;
        }
        if r.start > cursor {
            return false;
        }
        cursor = r.end;
        if cursor >= target.end {
            return true;
        }
    }
    false
}

fn subtract(mut ranges: Vec<TextByteRange>, cut: &TextByteRange) -> Vec<TextByteRange> {
    let mut result = vec![];
    for r in ranges.drain(..) {
        if cut.end <= r.start || cut.start >= r.end {
            result.push(r);
            continue;
        }
        if r.start < cut.start {
            result.push(TextByteRange {
                start: r.start,
                end: cut.start,
            });
        }
        if cut.end < r.end {
            result.push(TextByteRange {
                start: cut.end,
                end: r.end,
            });
        }
    }
    result
}

pub fn plan_source_facts(
    prepared: &PreparedUnitDiscoveryRequest,
    units: &ValidatedUnitSet,
    max_tasks_per_batch: usize,
) -> Result<Vec<PreparedFactRequest>, StageValidationReport> {
    let mut report = StageValidationReport::default();
    if max_tasks_per_batch == 0 {
        report.push("invalid_limits", "planner", "batch size must be positive");
        return Err(report);
    }
    prepared.revalidate_checkpoint(&units.checkpoint(), Default::default())?;
    let original = prepared.input();
    let mut cuts = BTreeSet::new();
    for u in units.units() {
        for s in u.selectors() {
            cuts.insert(s.range().start);
            cuts.insert(s.range().end);
        }
    }
    // Split physical blocks only where a validated unit boundary requires it.
    // All resulting fragments remain in source order; no reading text is removed.
    let mut input = original.clone();
    input.requested.segments.clear();
    let mut fragments = vec![];
    for segment in &original.requested.segments {
        match segment {
            DiscoveryInputSegment::Gap { .. } => input.requested.segments.push(segment.clone()),
            DiscoveryInputSegment::Block {
                source_range,
                text,
                structural_kind,
                ..
            } => {
                let mut edges = vec![source_range.start];
                edges.extend(
                    cuts.range((source_range.start + 1)..source_range.end)
                        .copied(),
                );
                edges.push(source_range.end);
                for pair in edges.windows(2) {
                    let range = TextByteRange {
                        start: pair[0],
                        end: pair[1],
                    };
                    let text = text[(range.start - source_range.start) as usize
                        ..(range.end - source_range.start) as usize]
                        .to_owned();
                    let id = format!("fragment:{}", fingerprint(&(&original.fingerprint, &range)));
                    fragments.push(range.clone());
                    input.requested.segments.push(DiscoveryInputSegment::Block {
                        block_id: id,
                        structural_kind: structural_kind.clone(),
                        physical_range: range.clone(),
                        source_range: range,
                        text,
                    });
                }
            }
        }
    }
    input.fingerprint = discovery_input_fingerprint(&input);
    let mut blocks: Vec<StageBlock> = vec![];
    let mut leading_gap = String::new();
    for s in &input.requested.segments {
        match s {
            DiscoveryInputSegment::Gap { text, .. } => {
                if let Some(b) = blocks.last_mut() {
                    b.following_gap.push_str(text)
                } else {
                    leading_gap.push_str(text)
                }
            }
            DiscoveryInputSegment::Block {
                structural_kind,
                text,
                ..
            } => blocks.push(StageBlock {
                block_ref: format!("b{}", blocks.len() + 1),
                structural_kind: structural_kind.clone(),
                clipped_start: false,
                clipped_end: false,
                text: text.clone(),
                following_gap: String::new(),
            }),
        }
    }
    let mut directory = vec![];
    let mut tasks = vec![];
    let mut scopes = HashMap::new();
    let parents: HashMap<&str, &str> = units
        .units()
        .iter()
        .filter_map(|u| {
            u.proposal()
                .parent
                .as_deref()
                .map(|p| (u.proposal().unit_ref.as_str(), p))
        })
        .collect();
    for (index, u) in units.units().iter().enumerate() {
        let entire: Vec<_> = u.selectors().iter().map(|s| s.range().clone()).collect();
        let mut owned = entire.clone();
        for child in units.units() {
            if is_ancestor(&parents, &u.proposal().unit_ref, &child.proposal().unit_ref)
                && child.selectors().iter().all(|s| covers(&entire, s.range()))
            {
                for r in child.selectors() {
                    owned = subtract(owned, r.range());
                }
            }
        }
        let selectors = entire
            .iter()
            .map(|r| {
                let indexes: Vec<_> = fragments
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.start >= r.start && f.end <= r.end)
                    .map(|(i, _)| i)
                    .collect();
                DiscoverySelector::BlockRange {
                    first: format!("b{}", indexes[0] + 1),
                    last: format!("b{}", indexes[indexes.len() - 1] + 1),
                }
            })
            .collect();
        directory.push(FrozenUnitEntry {
            unit_ref: u.proposal().unit_ref.clone(),
            unit_kind: u.proposal().unit_kind,
            display_title: u.proposal().display_title.clone(),
            parent: u.proposal().parent.clone(),
            selectors,
        });
        let task_ref = format!("t{}", index + 1);
        let mut evidence = vec![];
        let mut context = vec![];
        for (i, f) in fragments.iter().enumerate() {
            let block_ref = format!("b{}", i + 1);
            if covers(&owned, f) {
                evidence.push(block_ref);
            } else {
                context.push(FactContextBlock {
                    block_ref,
                    within_owner: covers(&entire, f),
                });
            }
        }
        let locator = if owned.is_empty() {
            None
        } else {
            Some(Arc::new(
                OccurrenceResolver::new(&input, owned.clone()).map_err(|e| {
                    report.push("invalid_scope", &task_ref, e.to_string());
                    report.clone()
                })?,
            ))
        };
        tasks.push(FactTask {
            task_ref: task_ref.clone(),
            owner_unit_ref: u.proposal().unit_ref.clone(),
            evidence_block_refs: evidence,
            context_block_refs: context,
        });
        scopes.insert(
            task_ref,
            FactScope {
                owner: u.proposal().unit_ref.clone(),
                entire,
                locator,
            },
        );
    }
    let manifest = fact_prompt_manifest();
    let schema = fact_output_schema();
    let prompt_fingerprint = fingerprint(&(&manifest, FACT_SYSTEM_PROMPT, &schema));
    let mut batches = vec![];
    for chunk in tasks.chunks(max_tasks_per_batch) {
        let model = SourceFactInput {
            version: SourceFactInputVersion::V2,
            request_ref: new_request_ref("f"),
            leading_gap: leading_gap.clone(),
            boundary: StageBoundary {
                at_document_start: original.boundary.at_document_start,
                at_document_end: original.boundary.at_document_end,
            },
            blocks: blocks.clone(),
            units: directory.clone(),
            tasks: chunk.to_vec(),
        };
        let batch_fingerprint = fingerprint(&(
            units.fingerprint(),
            &input.fingerprint,
            chunk,
            &prompt_fingerprint,
        ));
        let request = LlmRequest {
            task: LlmTask {
                kind: manifest.task_kind.clone(),
                version: manifest.prompt_revision.clone(),
                input_fingerprint: Some(fingerprint(&(&batch_fingerprint, &model))),
            },
            messages: vec![
                LlmMessage::text(
                    LlmRole::System,
                    format!("{}\n\nJSON Schema:\n{}", FACT_SYSTEM_PROMPT, schema),
                ),
                LlmMessage::text(
                    LlmRole::User,
                    serde_json::to_string(&model).expect("model serializes"),
                ),
            ],
            tools: vec![],
            tool_choice: LlmToolChoice::None,
            response_format: LlmResponseFormat::JsonSchema {
                name: manifest.schema_name.clone(),
                description: None,
                schema: schema.clone(),
            },
            options: LlmGenerationOptions {
                max_output_tokens: Some(manifest.max_output_tokens),
                ..Default::default()
            },
        };
        batches.push(PreparedFactRequest {
            input: input.clone(),
            model,
            scopes: chunk
                .iter()
                .map(|t| (t.task_ref.clone(), scopes[&t.task_ref].clone()))
                .collect(),
            unit_set_fingerprint: units.fingerprint().clone(),
            source_fingerprint: original.fingerprint.clone(),
            request,
            prompt_fingerprint: prompt_fingerprint.clone(),
            batch_fingerprint,
        });
    }
    Ok(batches)
}
