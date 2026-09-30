//! Stage A: prepare, propose, resolve, validate and freeze. No graph writes.
use crate::discovery_producer::{collect_text, reject_finish_reason};
use crate::discovery_prompt::output_schema_for;
use crate::discovery_validation::{authorize_discovery_input, is_ancestor, valid_temp_id};
use crate::{
    DiscoveryInputError, DiscoveryInputLimits, DiscoveryProducerLimits, DiscoveryRunSummary,
    DiscoveryValidationLimits, PAGE_MARKER_STRUCTURAL_KIND, ResolvedDiscoverySelector,
    SelectorResolver, SelectorRewrite, build_discovery_input, is_page_marker_text,
};
use mathscape_domain::*;
use mathscape_llm::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub const UNIT_VALIDATOR_REVISION: &str = "unit-validator-1";
pub const STAGED_LOCATOR_REVISION: &str = "staged-locator-1";
pub const UNIT_SYSTEM_PROMPT: &str = include_str!("../../../prompts/unit-discovery/v1/system.md");
pub const FACT_SYSTEM_PROMPT: &str = include_str!("../../../prompts/source-facts/v1/system.md");

pub(crate) fn fingerprint<T: Serialize>(value: &T) -> ContentHash {
    let bytes = serde_json::to_vec(value).expect("fingerprint input serializes");
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes))).expect("valid hash")
}

pub(crate) fn new_request_ref(prefix: &str) -> String {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}{time:x}-{:x}",
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagePromptManifest {
    pub task_kind: String,
    pub pipeline_version: String,
    pub input_version: String,
    pub output_version: String,
    pub prompt_revision: String,
    pub schema_name: String,
    pub max_output_tokens: u32,
    pub max_response_bytes: usize,
}
pub fn unit_prompt_manifest() -> StagePromptManifest {
    serde_json::from_str(include_str!(
        "../../../prompts/unit-discovery/v1/manifest.json"
    ))
    .expect("manifest")
}
pub fn fact_prompt_manifest() -> StagePromptManifest {
    serde_json::from_str(include_str!(
        "../../../prompts/source-facts/v1/manifest.json"
    ))
    .expect("manifest")
}
pub fn unit_output_schema() -> Value {
    output_schema_for::<UnitDiscoveryProposal>()
}
pub fn fact_output_schema() -> Value {
    output_schema_for::<SourceFactProposal>()
}
pub fn repair_output_schema() -> Value {
    output_schema_for::<DiscoveryRepairResponse>()
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct StageDiagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
}
#[derive(Clone, Debug, Default, Error, Serialize, Deserialize)]
#[error("staged discovery validation failed")]
pub struct StageValidationReport {
    pub diagnostics: Vec<StageDiagnostic>,
    pub not_checked: Vec<String>,
}
impl StageValidationReport {
    pub(crate) fn push(&mut self, code: &str, path: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(StageDiagnostic {
            code: code.into(),
            path: path.into(),
            message: message.into(),
        });
    }

    /// Records a deterministic, non-blocking quality warning. Warnings stay
    /// in the same audit stream as validation diagnostics, but never force a
    /// repair or make the task invalid.
    pub(crate) fn warn(&mut self, code: &str, path: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(StageDiagnostic {
            code: format!("warning_{code}"),
            path: path.into(),
            message: message.into(),
        });
    }

    pub fn is_valid(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.starts_with("warning_"))
    }
}

#[derive(Clone, Debug)]
pub struct PreparedUnitDiscoveryRequest {
    input: DiscoveryInput,
    model_input: UnitDiscoveryInput,
    resolver: SelectorResolver,
    request: LlmRequest,
    manifest: StagePromptManifest,
    prompt_fingerprint: ContentHash,
    request_fingerprint: ContentHash,
}
impl PreparedUnitDiscoveryRequest {
    pub fn input(&self) -> &DiscoveryInput {
        &self.input
    }
    pub fn model_input(&self) -> &UnitDiscoveryInput {
        &self.model_input
    }
    pub fn request(&self) -> &LlmRequest {
        &self.request
    }
    pub fn manifest(&self) -> &StagePromptManifest {
        &self.manifest
    }
    pub fn prompt_fingerprint(&self) -> &ContentHash {
        &self.prompt_fingerprint
    }
    pub fn request_fingerprint(&self) -> &ContentHash {
        &self.request_fingerprint
    }
    pub fn validate(
        &self,
        proposal: UnitDiscoveryProposal,
        limits: DiscoveryValidationLimits,
    ) -> Result<ValidatedUnitSet, StageValidationReport> {
        validate_units(self, proposal, limits)
    }

    /// Rebind a canonical checkpoint to this transient request and prove that
    /// it still describes the same source and frozen UnitSet.
    pub fn revalidate_checkpoint(
        &self,
        checkpoint: &UnitDiscoveryCheckpoint,
        limits: DiscoveryValidationLimits,
    ) -> Result<ValidatedUnitSet, StageValidationReport> {
        let mut report = StageValidationReport::default();
        if checkpoint.input_fingerprint != self.input.fingerprint {
            report.push(
                "input_mismatch",
                "checkpoint.input_fingerprint",
                "checkpoint belongs to another DiscoveryInput",
            );
            return Err(report);
        }
        let mut proposal = checkpoint.proposal.clone();
        proposal.request_ref = self.model_input.request_ref.clone();
        let units = self.validate(proposal, limits)?;
        if units.fingerprint != checkpoint.unit_set_fingerprint {
            report.push(
                "unit_set_mismatch",
                "checkpoint.unit_set_fingerprint",
                "checkpoint does not reproduce the recorded UnitSet",
            );
            return Err(report);
        }
        Ok(units)
    }
}

pub fn build_unit_discovery_request(
    document: &AnchoredMarkdown,
    selection: &ResolvedDiscoverySelection,
    limits: DiscoveryInputLimits,
) -> Result<PreparedUnitDiscoveryRequest, DiscoveryInputError> {
    let input = build_discovery_input(document, selection, limits)?;
    Ok(prepare_unit_discovery_request_from_input(input))
}

/// Rebuilds the transient prompt and selector machinery around a persisted,
/// freshly rebound DiscoveryInput. Calling this function does not validate a
/// UnitSet; callers must use `revalidate_checkpoint` afterwards.
pub fn prepare_unit_discovery_request_from_input(
    input: DiscoveryInput,
) -> PreparedUnitDiscoveryRequest {
    let resolver = SelectorResolver::from_input(&input);
    let mut blocks: Vec<StageBlock> = Vec::new();
    let mut leading_gap = String::new();
    for segment in &input.requested.segments {
        match segment {
            DiscoveryInputSegment::Block {
                structural_kind,
                text,
                ..
            } => blocks.push(StageBlock {
                block_ref: format!("b{}", blocks.len() + 1),
                structural_kind: if is_page_marker_text(text) {
                    PAGE_MARKER_STRUCTURAL_KIND.into()
                } else {
                    structural_kind.clone()
                },
                clipped_start: segment.clipped_start(),
                clipped_end: segment.clipped_end(),
                text: text.clone(),
                following_gap: String::new(),
            }),
            DiscoveryInputSegment::Gap { text, .. } => match blocks.last_mut() {
                Some(block) => block.following_gap.push_str(text),
                None => leading_gap.push_str(text),
            },
        }
    }
    let model_input = UnitDiscoveryInput {
        version: UnitDiscoveryInputVersion::V1,
        request_ref: new_request_ref("a"),
        leading_gap,
        blocks,
        boundary: StageBoundary {
            at_document_start: input.boundary.at_document_start,
            at_document_end: input.boundary.at_document_end,
        },
    };
    let manifest = unit_prompt_manifest();
    assert_eq!(manifest.task_kind, "unit-discovery");
    assert_eq!(manifest.pipeline_version, DISCOVERY_PIPELINE_VERSION);
    assert_eq!(manifest.input_version, UNIT_DISCOVERY_INPUT_VERSION);
    assert_eq!(manifest.output_version, UNIT_DISCOVERY_VERSION);
    let schema = unit_output_schema();
    let prompt_fingerprint = fingerprint(&(&manifest, UNIT_SYSTEM_PROMPT, &schema));
    let request_fingerprint = fingerprint(&(&input.fingerprint, &model_input, &prompt_fingerprint));
    let request = LlmRequest {
        task: LlmTask {
            kind: manifest.task_kind.clone(),
            version: manifest.prompt_revision.clone(),
            input_fingerprint: Some(request_fingerprint.clone()),
        },
        messages: vec![
            LlmMessage::text(
                LlmRole::System,
                format!("{}\n\nJSON Schema:\n{}", UNIT_SYSTEM_PROMPT, schema),
            ),
            LlmMessage::text(
                LlmRole::User,
                serde_json::to_string(&model_input).expect("input serializes"),
            ),
        ],
        tools: vec![],
        tool_choice: LlmToolChoice::None,
        response_format: LlmResponseFormat::JsonSchema {
            name: manifest.schema_name.clone(),
            description: None,
            schema,
        },
        options: LlmGenerationOptions {
            max_output_tokens: Some(manifest.max_output_tokens),
            ..Default::default()
        },
    };
    PreparedUnitDiscoveryRequest {
        input,
        model_input,
        resolver,
        request,
        manifest,
        prompt_fingerprint,
        request_fingerprint,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrozenDiscoveryUnit {
    proposal: UnitProposal,
    selectors: Vec<ResolvedDiscoverySelector>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrozenDiscoveryIssue {
    proposal: DiscoveryIssueProposal,
    evidence: Vec<ResolvedDiscoverySelector>,
}
impl FrozenDiscoveryIssue {
    pub fn proposal(&self) -> &DiscoveryIssueProposal {
        &self.proposal
    }
    pub fn evidence(&self) -> &[ResolvedDiscoverySelector] {
        &self.evidence
    }
}
impl FrozenDiscoveryUnit {
    pub fn proposal(&self) -> &UnitProposal {
        &self.proposal
    }
    pub fn selectors(&self) -> &[ResolvedDiscoverySelector] {
        &self.selectors
    }
}

/// Only the stage validator constructs this value. No Deserialize or public setters.
#[derive(Clone, Debug)]
pub struct ValidatedUnitSet {
    fingerprint: ContentHash,
    input_fingerprint: ContentHash,
    units: Vec<FrozenDiscoveryUnit>,
    issues: Vec<FrozenDiscoveryIssue>,
    proposal: UnitDiscoveryProposal,
    rewrites: Vec<SelectorRewrite>,
    partial: bool,
}
impl ValidatedUnitSet {
    pub fn fingerprint(&self) -> &ContentHash {
        &self.fingerprint
    }
    pub fn input_fingerprint(&self) -> &ContentHash {
        &self.input_fingerprint
    }
    pub fn units(&self) -> &[FrozenDiscoveryUnit] {
        &self.units
    }
    pub fn issues(&self) -> &[FrozenDiscoveryIssue] {
        &self.issues
    }
    pub fn proposal(&self) -> &UnitDiscoveryProposal {
        &self.proposal
    }
    pub fn rewrites(&self) -> &[SelectorRewrite] {
        &self.rewrites
    }
    pub fn is_partial(&self) -> bool {
        self.partial
    }
    pub fn checkpoint(&self) -> UnitDiscoveryCheckpoint {
        UnitDiscoveryCheckpoint {
            input_fingerprint: self.input_fingerprint.clone(),
            unit_set_fingerprint: self.fingerprint.clone(),
            proposal: self.proposal.clone(),
            issue_registry: canonical_discovery_issues(self),
        }
    }
}

#[derive(Clone, Debug)]
struct RebindBlock {
    handle: String,
    range: TextByteRange,
    text: String,
}

fn rebind_blocks(input: &DiscoveryInput) -> Vec<RebindBlock> {
    input
        .requested
        .segments
        .iter()
        .filter_map(|segment| match segment {
            DiscoveryInputSegment::Block {
                source_range, text, ..
            } => Some((source_range.clone(), text.clone())),
            DiscoveryInputSegment::Gap { .. } => None,
        })
        .enumerate()
        .map(|(index, (range, text))| RebindBlock {
            handle: format!("b{}", index + 1),
            range,
            text,
        })
        .collect()
}

fn quote_anchor_for(
    block: &RebindBlock,
    start: usize,
    end: usize,
) -> Result<DiscoveryQuoteAnchor, String> {
    if start >= end || end > block.text.len() {
        return Err("rebind endpoint is outside its block".into());
    }
    let quote = block
        .text
        .get(start..end)
        .ok_or_else(|| "rebind endpoint is not on a UTF-8 boundary".to_owned())?;
    let occurrence = crate::markdown_patch::exact_occurrences(&block.text, quote)
        .iter()
        .position(|(a, _)| *a == start)
        .and_then(|index| u32::try_from(index + 1).ok());
    Ok(DiscoveryQuoteAnchor {
        quote: quote.to_owned(),
        occurrence: (occurrence != Some(1)).then_some(occurrence.unwrap_or(1)),
        prefix: None,
        suffix: None,
    })
}

fn endpoint_block<'a>(
    blocks: &'a [RebindBlock],
    range: &TextByteRange,
) -> Result<(&'a RebindBlock, usize, &'a RebindBlock, usize), String> {
    let start = blocks
        .iter()
        .find(|block| block.range.start <= range.start && range.start < block.range.end)
        .ok_or_else(|| "rebind range start is not inside a block".to_owned())?;
    let end = blocks
        .iter()
        .find(|block| block.range.start < range.end && range.end <= block.range.end)
        .ok_or_else(|| "rebind range end is not inside a block".to_owned())?;
    let start_offset = usize::try_from(range.start - start.range.start)
        .map_err(|_| "rebind start offset overflows".to_owned())?;
    let end_offset = usize::try_from(range.end - end.range.start)
        .map_err(|_| "rebind end offset overflows".to_owned())?;
    Ok((start, start_offset, end, end_offset))
}

fn selector_for_rebound_range(
    input: &DiscoveryInput,
    range: &TextByteRange,
) -> Result<DiscoverySelector, String> {
    let blocks = rebind_blocks(input);
    let (start_block, start_offset, end_block, end_offset) = endpoint_block(&blocks, range)?;
    // Quotes are at most 64 bytes, widened to the nearest UTF-8 boundary.
    let mut start_end = (start_offset + 64).min(start_block.text.len());
    while !start_block.text.is_char_boundary(start_end) {
        start_end += 1;
    }
    let start_quote = quote_anchor_for(start_block, start_offset, start_end)?;
    let mut end_start = end_offset.saturating_sub(64);
    while !end_block.text.is_char_boundary(end_start) {
        end_start -= 1;
    }
    let end_quote = quote_anchor_for(end_block, end_start, end_offset)?;
    Ok(DiscoverySelector::QuoteRange {
        first: start_block.handle.clone(),
        start: start_quote,
        last: end_block.handle.clone(),
        end: end_quote,
    })
}

fn resolve_existing_selector_range(
    prepared: &PreparedUnitDiscoveryRequest,
    selector: &DiscoverySelector,
) -> Result<TextByteRange, String> {
    let (canonical, _) = prepared
        .resolver
        .resolve_selector(selector.clone(), "rebind")
        .map_err(|error| error.to_string())?;
    let mut diagnostics = Vec::new();
    let region = authorize_discovery_input(&prepared.input, &mut diagnostics)
        .ok_or_else(|| "existing selector input is invalid".to_owned())?;
    region
        .resolve(&canonical)
        .map(|resolved| resolved.range().clone())
        .map_err(|error| error.to_owned())
}

fn map_rebound_ranges(
    ranges: &[TextByteRange],
    offset_map: &crate::markdown_patch::OffsetMap,
) -> Result<Vec<TextByteRange>, usize> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, range)| offset_map.map_range(range).ok_or(index))
        .collect()
}

/// Rebinds a validated A UnitSet onto a patched DiscoveryInput. All selectors
/// are regenerated from mapped byte ranges and passed through the normal A
/// validator again; this function never trusts a shifted quote by itself.
pub fn rebind_unit_set_after_patch(
    prepared: &PreparedUnitDiscoveryRequest,
    units: &ValidatedUnitSet,
    new_input: DiscoveryInput,
    offset_map: &crate::markdown_patch::OffsetMap,
) -> Result<(PreparedUnitDiscoveryRequest, ValidatedUnitSet), StageValidationReport> {
    let new_prepared = prepare_unit_discovery_request_from_input(new_input);
    let mut report = StageValidationReport::default();
    let mut mapped_unit_ranges: HashMap<String, Vec<TextByteRange>> = HashMap::new();
    let mut proposal = units.proposal.clone();
    proposal.request_ref = new_prepared.model_input.request_ref.clone();
    for (index, unit) in proposal.units.iter_mut().enumerate() {
        let Some(frozen) = units.units.get(index) else {
            report.push(
                "rebind_unit_mismatch",
                "units",
                "unit checkpoint is inconsistent",
            );
            continue;
        };
        let old_ranges: Vec<_> = frozen
            .selectors
            .iter()
            .map(|selector| selector.range().clone())
            .collect();
        let ranges = match map_rebound_ranges(&old_ranges, offset_map) {
            Ok(ranges) => ranges,
            Err(failed) => {
                report.push(
                    "rebind_ambiguous_range",
                    format!("units[{index}].selectors[{failed}]"),
                    "range endpoint falls inside a replaced span",
                );
                continue;
            }
        };
        mapped_unit_ranges.insert(unit.unit_ref.clone(), ranges.clone());
        unit.selectors = ranges
            .iter()
            .filter_map(|range| selector_for_rebound_range(&new_prepared.input, range).ok())
            .collect();
        if unit.selectors.len() != ranges.len() {
            report.push(
                "rebind_selector_failed",
                format!("units[{index}].selectors"),
                "could not construct a unique canonical selector",
            );
        }
    }
    for (index, issue) in proposal.issues.iter_mut().enumerate() {
        let Some(frozen) = units.issues.get(index) else {
            report.push(
                "rebind_issue_mismatch",
                "issues",
                "issue checkpoint is inconsistent",
            );
            continue;
        };
        let old_ranges: Vec<_> = frozen
            .evidence
            .iter()
            .map(|selector| selector.range().clone())
            .collect();
        let ranges = match map_rebound_ranges(&old_ranges, offset_map) {
            Ok(ranges) => ranges,
            Err(failed) => {
                report.push(
                    "rebind_ambiguous_range",
                    format!("issues[{index}].evidence[{failed}]"),
                    "range endpoint falls inside a replaced span",
                );
                continue;
            }
        };
        issue.evidence = ranges
            .iter()
            .filter_map(|range| selector_for_rebound_range(&new_prepared.input, range).ok())
            .collect();
        if issue.evidence.len() != ranges.len() {
            report.push(
                "rebind_selector_failed",
                format!("issues[{index}].evidence"),
                "could not construct a unique issue evidence selector",
            );
        }
    }
    for (index, coverage) in proposal.coverage.iter_mut().enumerate() {
        let range = match resolve_existing_selector_range(prepared, &coverage.selector)
            .ok()
            .and_then(|range| offset_map.map_range(&range))
        {
            Some(range) => range,
            None => {
                report.push(
                    "rebind_coverage_failed",
                    format!("coverage[{index}].selector"),
                    "coverage range cannot be mapped to the patched input",
                );
                continue;
            }
        };
        match selector_for_rebound_range(&new_prepared.input, &range) {
            Ok(selector) => coverage.selector = selector,
            Err(error) => report.push(
                "rebind_selector_failed",
                format!("coverage[{index}].selector"),
                error,
            ),
        }
    }
    if !report.is_valid() {
        return Err(report);
    }
    match new_prepared.validate(proposal, Default::default()) {
        Ok(rebound) => Ok((new_prepared, rebound)),
        Err(validation) => Err(validation),
    }
}

/// Assigns canonical, system-owned issue references after A validation. The
/// initial implementation preserves validated proposal order; the persisted
/// checkpoint is the authority across repair/replay.
pub fn canonical_discovery_issues(units: &ValidatedUnitSet) -> Vec<CanonicalDiscoveryIssue> {
    units
        .issues
        .iter()
        .enumerate()
        .map(|(index, issue)| CanonicalDiscoveryIssue {
            issue_ref: DiscoveryIssueRef::parse(format!("a-i{}", index + 1))
                .expect("canonical issue ref is valid"),
            kind: issue.proposal.kind,
            unit_ref: issue.proposal.unit.clone(),
            evidence: issue
                .evidence
                .iter()
                .map(|selector| selector.range().clone())
                .collect(),
            observation: issue.proposal.observation.clone(),
        })
        .collect()
}

/// Returns the persisted issue registry when available. This keeps issue
/// references stable across a repair/replay; legacy v1 checkpoints are
/// upgraded lazily from their proposal order.
pub fn canonical_discovery_issues_for_checkpoint(
    checkpoint: &UnitDiscoveryCheckpoint,
) -> Vec<CanonicalDiscoveryIssue> {
    if !checkpoint.issue_registry.is_empty() {
        return checkpoint.issue_registry.clone();
    }
    // Legacy checkpoints did not persist resolved evidence. Returning an
    // empty evidence set deliberately makes the patch resolver fail closed;
    // callers can only patch such a run after rebuilding a validated UnitSet.
    checkpoint
        .proposal
        .issues
        .iter()
        .enumerate()
        .map(|(index, issue)| CanonicalDiscoveryIssue {
            issue_ref: DiscoveryIssueRef::parse(format!("a-i{}", index + 1))
                .expect("canonical issue ref is valid"),
            kind: issue.kind,
            unit_ref: issue.unit.clone(),
            evidence: Vec::new(),
            observation: issue.observation.clone(),
        })
        .collect()
}

fn inside(outer: &[ResolvedDiscoverySelector], item: &ResolvedDiscoverySelector) -> bool {
    outer
        .iter()
        .any(|r| r.range().start <= item.range().start && r.range().end >= item.range().end)
}

pub fn preflight_units(proposal: &UnitDiscoveryProposal) -> StageValidationReport {
    let mut report = StageValidationReport::default();
    let mut ids = HashSet::new();
    for (i, unit) in proposal.units.iter().enumerate() {
        if !valid_temp_id(&unit.unit_ref) {
            report.push(
                "invalid_value",
                format!("units[{i}].ref"),
                "invalid unit ref",
            );
        }
        if !ids.insert(unit.unit_ref.as_str()) {
            report.push("duplicate_id", format!("units[{i}].ref"), "duplicate unit");
        }
        if unit
            .display_title
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        {
            report.push(
                "invalid_value",
                format!("units[{i}].display_title"),
                "blank title",
            );
        }
    }
    let parents: HashMap<&str, &str> = proposal
        .units
        .iter()
        .filter_map(|u| u.parent.as_deref().map(|p| (u.unit_ref.as_str(), p)))
        .collect();
    for (i, unit) in proposal.units.iter().enumerate() {
        if let Some(p) = unit.parent.as_deref() {
            if p == unit.unit_ref || !ids.contains(p) {
                report.push(
                    "unknown_unit",
                    format!("units[{i}].parent"),
                    "parent must be another declared unit",
                );
            }
            let mut visited = HashSet::new();
            let mut current = unit.unit_ref.as_str();
            while let Some(next) = parents.get(current) {
                if !visited.insert(current) {
                    report.push("parent_cycle", format!("units[{i}].parent"), "parent cycle");
                    break;
                }
                current = next;
            }
        }
    }
    for (i, issue) in proposal.issues.iter().enumerate() {
        if issue.observation.trim().is_empty() {
            report.push(
                "invalid_value",
                format!("issues[{i}].observation"),
                "blank observation",
            );
        }
        if issue.unit.as_deref().is_some_and(|u| !ids.contains(u)) {
            report.push(
                "unknown_unit",
                format!("issues[{i}].unit"),
                "unknown issue unit",
            );
        }
        if issue.unit.is_none() && issue.evidence.is_empty() {
            report.push(
                "invalid_evidence",
                format!("issues[{i}]"),
                "issue has no unit or evidence",
            );
        }
        if let Some(DiscoveryContextRequest::LocalReference { target_label }) =
            &issue.context_request
            && target_label.trim().is_empty()
        {
            report.push(
                "invalid_value",
                format!("issues[{i}].context_request"),
                "blank target label",
            );
        }
    }
    report
}

fn validate_units(
    prepared: &PreparedUnitDiscoveryRequest,
    mut proposal: UnitDiscoveryProposal,
    limits: DiscoveryValidationLimits,
) -> Result<ValidatedUnitSet, StageValidationReport> {
    let mut report = StageValidationReport::default();
    let count = proposal
        .units
        .iter()
        .map(|u| u.selectors.len())
        .sum::<usize>()
        + proposal
            .issues
            .iter()
            .map(|i| i.evidence.len())
            .sum::<usize>()
        + proposal.coverage.len();
    if [
        proposal.units.len(),
        proposal.issues.len(),
        proposal.coverage.len(),
    ]
    .iter()
    .any(|n| *n > limits.max_items_per_collection)
        || count > limits.max_total_selectors
        || serde_json::to_vec(&proposal).map_or(usize::MAX, |s| s.len())
            > limits.max_serialized_bytes
    {
        report.push(
            "limit_exceeded",
            "proposal",
            "stage proposal exceeds limits",
        );
        return Err(report);
    }
    report = preflight_units(&proposal);
    if proposal.request_ref != prepared.model_input.request_ref {
        report.push(
            "request_mismatch",
            "request_ref",
            "proposal belongs to another request",
        );
    }
    let mut input_errors = Vec::new();
    let region = authorize_discovery_input(&prepared.input, &mut input_errors);
    for d in input_errors {
        report.push("input_integrity", d.path(), d.message());
    }
    let Some(region) = region else {
        return Err(report);
    };
    let mut rewrites = Vec::new();
    let mut locate = |selectors: &mut [DiscoverySelector],
                      path: &str,
                      report: &mut StageValidationReport|
     -> Option<Vec<ResolvedDiscoverySelector>> {
        let mut resolved = Vec::new();
        let mut failed = false;
        for (i, selector) in selectors.iter_mut().enumerate() {
            let field = format!("{path}[{i}]");
            match prepared.resolver.resolve_selector(selector.clone(), &field) {
                Ok((canonical, changes)) => match region.resolve(&canonical) {
                    Ok(r) => {
                        resolved.push(r);
                        *selector = canonical;
                        rewrites.extend(changes);
                    }
                    Err(m) => {
                        report.push("invalid_selector", &field, m);
                        failed = true;
                    }
                },
                Err(e) => {
                    for d in e.diagnostics() {
                        report.push("selector_resolution", d.path(), d.message());
                    }
                    failed = true;
                }
            }
        }
        if failed {
            report
                .not_checked
                .push(format!("{path}: dependent range checks"));
            return None;
        }
        resolved.sort_by_key(|r| (r.range().start, r.range().end));
        if resolved
            .windows(2)
            .any(|p| p[0].range().end > p[1].range().start)
        {
            report.push(
                "invalid_selector",
                path,
                "overlapping selectors within an object",
            );
        }
        Some(resolved)
    };
    let mut ranges = Vec::new();
    for (i, unit) in proposal.units.iter_mut().enumerate() {
        if unit.selectors.is_empty() {
            report.push(
                "invalid_selector",
                format!("units[{i}].selectors"),
                "unit needs a range",
            );
        }
        ranges.push(locate(
            &mut unit.selectors,
            &format!("units[{i}].selectors"),
            &mut report,
        ));
    }
    let parents: HashMap<&str, &str> = proposal
        .units
        .iter()
        .filter_map(|u| u.parent.as_deref().map(|p| (u.unit_ref.as_str(), p)))
        .collect();
    for a in 0..proposal.units.len() {
        for b in a + 1..proposal.units.len() {
            let (Some(ra), Some(rb)) = (&ranges[a], &ranges[b]) else {
                continue;
            };
            let overlap = ra.iter().any(|x| {
                rb.iter()
                    .any(|y| x.range().start < y.range().end && y.range().start < x.range().end)
            });
            if !overlap {
                continue;
            }
            let a_parent = is_ancestor(
                &parents,
                &proposal.units[a].unit_ref,
                &proposal.units[b].unit_ref,
            );
            let b_parent = is_ancestor(
                &parents,
                &proposal.units[b].unit_ref,
                &proposal.units[a].unit_ref,
            );
            let a_contains = rb.iter().all(|r| inside(ra, r));
            let b_contains = ra.iter().all(|r| inside(rb, r));
            if !((a_parent && a_contains && !b_contains) || (b_parent && b_contains && !a_contains))
            {
                report.push(
                    "overlapping_units",
                    format!("units[{a}],units[{b}]"),
                    "overlap requires strict physical containment and an ancestor relationship",
                );
            }
        }
    }
    let mut issue_ranges = Vec::with_capacity(proposal.issues.len());
    for (i, issue) in proposal.issues.iter_mut().enumerate() {
        let evidence = locate(
            &mut issue.evidence,
            &format!("issues[{i}].evidence"),
            &mut report,
        );
        if let Some(evidence) = &evidence
            && let Some(unit) = issue
                .unit
                .as_deref()
                .and_then(|id| proposal.units.iter().position(|u| u.unit_ref == id))
        {
            if let Some(owner) = &ranges[unit] {
                if evidence.iter().any(|r| !inside(owner, r)) {
                    report.push(
                        "invalid_evidence",
                        format!("issues[{i}].evidence"),
                        "evidence outside owner",
                    );
                }
            } else {
                report
                    .not_checked
                    .push(format!("issues[{i}]: evidence ownership"));
            }
        }
        issue_ranges.push(evidence);
    }
    for (i, unit) in proposal.units.iter().enumerate() {
        let kinds: BTreeSet<_> = proposal
            .issues
            .iter()
            .filter(|x| x.unit.as_deref() == Some(&unit.unit_ref))
            .map(|x| x.kind)
            .collect();
        let valid = match unit.completeness {
            UnitCompleteness::Complete => true,
            UnitCompleteness::LeftTruncated => kinds.contains(&DiscoveryIssueKind::LeftTruncated),
            UnitCompleteness::RightTruncated => kinds.contains(&DiscoveryIssueKind::RightTruncated),
            UnitCompleteness::InternallyMissing => {
                kinds.contains(&DiscoveryIssueKind::InternallyMissing)
            }
            UnitCompleteness::Uncertain => !kinds.is_empty(),
        };
        let reverse = kinds.iter().all(|k| match k {
            DiscoveryIssueKind::LeftTruncated => matches!(
                unit.completeness,
                UnitCompleteness::LeftTruncated | UnitCompleteness::Uncertain
            ),
            DiscoveryIssueKind::RightTruncated => matches!(
                unit.completeness,
                UnitCompleteness::RightTruncated | UnitCompleteness::Uncertain
            ),
            DiscoveryIssueKind::InternallyMissing => matches!(
                unit.completeness,
                UnitCompleteness::InternallyMissing | UnitCompleteness::Uncertain
            ),
            _ => true,
        });
        if !valid || !reverse {
            report.push(
                "invalid_completeness",
                format!("units[{i}].completeness"),
                "completeness and issues disagree",
            );
        }
    }
    let mut coverage = Vec::new();
    let mut coverage_valid = true;
    if proposal.coverage.is_empty() {
        report.push("invalid_coverage", "coverage", "coverage is required");
    }
    for (i, item) in proposal.coverage.iter_mut().enumerate() {
        match &item.disposition {
            DiscoveryCoverageDisposition::Skipped { reason }
            | DiscoveryCoverageDisposition::Unprocessed { reason }
                if reason.trim().is_empty() =>
            {
                report.push("invalid_value", format!("coverage[{i}]"), "missing reason")
            }
            _ => (),
        }
        match locate(
            std::slice::from_mut(&mut item.selector),
            &format!("coverage[{i}].selector"),
            &mut report,
        ) {
            Some(mut r) => coverage.push((
                r.remove(0),
                matches!(item.disposition, DiscoveryCoverageDisposition::Processed {}),
            )),
            None => coverage_valid = false,
        }
    }
    if coverage_valid {
        coverage.sort_by_key(|(r, _)| (r.range().start, r.range().end));
        let mut cursor = region.range.start;
        for (r, _) in &coverage {
            if r.range().start < cursor {
                report.push("invalid_coverage", "coverage", "overlapping coverage");
            }
            if r.range().start > cursor
                && region
                    .text_in(&TextByteRange {
                        start: cursor,
                        end: r.range().start,
                    })
                    .is_some_and(|s| !s.trim().is_empty())
            {
                report.push("coverage_gap", "coverage", "unaccounted source text");
            }
            cursor = cursor.max(r.range().end);
        }
        if cursor < region.range.end
            && region
                .text_in(&TextByteRange {
                    start: cursor,
                    end: region.range.end,
                })
                .is_some_and(|s| !s.trim().is_empty())
        {
            report.push("coverage_gap", "coverage", "unaccounted trailing text");
        }
        for (i, unit_ranges) in ranges.iter().enumerate() {
            if let Some(unit_ranges) = unit_ranges
                && unit_ranges.iter().any(|r| {
                    !coverage.iter().any(|(c, processed)| {
                        *processed
                            && c.range().start <= r.range().start
                            && c.range().end >= r.range().end
                    })
                })
            {
                report.push(
                    "invalid_coverage",
                    format!("units[{i}].selectors"),
                    "unit is not inside processed coverage",
                );
            }
        }
    } else {
        report
            .not_checked
            .push("coverage: gaps and unit membership".into());
    }
    if !report.is_valid() {
        return Err(report);
    }
    let units: Vec<_> = proposal
        .units
        .iter()
        .cloned()
        .zip(ranges)
        .map(|(proposal, selectors)| FrozenDiscoveryUnit {
            proposal,
            selectors: selectors.expect("all ranges passed"),
        })
        .collect();
    let issues = proposal
        .issues
        .iter()
        .cloned()
        .zip(issue_ranges)
        .map(|(proposal, evidence)| FrozenDiscoveryIssue {
            proposal,
            evidence: evidence.expect("all issue ranges passed"),
        })
        .collect();
    // Canonical occurrence indices and formal block IDs make equivalent locators
    // converge. Request tokens deliberately do not enter the frozen identity.
    let identities: Vec<_> = units
        .iter()
        .map(|u| {
            (
                &u.proposal.unit_ref,
                u.proposal.unit_kind,
                &u.proposal.display_title,
                &u.proposal.parent,
                u.proposal.completeness,
                u.selectors
                    .iter()
                    .map(|s| (s.range(), s.block_ids()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let issue_identity: Vec<_> = proposal
        .issues
        .iter()
        .map(|issue| {
            (
                issue.kind,
                &issue.unit,
                &issue.observation,
                &issue.context_request,
                issue
                    .evidence
                    .iter()
                    .map(|s| region.resolve(s).expect("validated issue").range().clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let fingerprint = fingerprint(&(
        UNIT_VALIDATOR_REVISION,
        STAGED_LOCATOR_REVISION,
        &prepared.input.fingerprint,
        &proposal.version,
        identities,
        issue_identity,
        proposal
            .coverage
            .iter()
            .map(|c| {
                (
                    region
                        .resolve(&c.selector)
                        .expect("validated coverage")
                        .range()
                        .clone(),
                    &c.disposition,
                )
            })
            .collect::<Vec<_>>(),
    ));
    let partial = proposal.coverage.iter().any(|c| {
        matches!(
            c.disposition,
            DiscoveryCoverageDisposition::Unprocessed { .. }
        )
    });
    Ok(ValidatedUnitSet {
        fingerprint,
        input_fingerprint: prepared.input.fingerprint.clone(),
        units,
        issues,
        proposal,
        rewrites,
        partial,
    })
}

#[derive(Debug)]
pub enum UnitProductionOutcome {
    Valid {
        units: ValidatedUnitSet,
        run: DiscoveryRunSummary,
    },
    InvalidProposal {
        proposal: Option<UnitDiscoveryProposal>,
        message: String,
        report: Option<StageValidationReport>,
        run: DiscoveryRunSummary,
    },
    Failed {
        error: LlmError,
        run: DiscoveryRunSummary,
    },
}

pub struct UnitDiscoveryProducer<'a> {
    backend: &'a dyn LlmBackend,
    limits: DiscoveryProducerLimits,
}
impl<'a> UnitDiscoveryProducer<'a> {
    pub fn new(backend: &'a dyn LlmBackend) -> Self {
        Self {
            backend,
            limits: DiscoveryProducerLimits::default(),
        }
    }
    pub fn with_limits(backend: &'a dyn LlmBackend, limits: DiscoveryProducerLimits) -> Self {
        Self { backend, limits }
    }
    pub async fn produce(&self, prepared: PreparedUnitDiscoveryRequest) -> UnitProductionOutcome {
        let profile = self.backend.profile();
        let mut run = DiscoveryRunSummary {
            task_kind: prepared.manifest.task_kind.clone(),
            task_version: prepared.manifest.prompt_revision.clone(),
            input_fingerprint: prepared.request_fingerprint.clone(),
            prompt_fingerprint: prepared.prompt_fingerprint.clone(),
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            requested_model: profile.model.clone(),
            response: None,
        };
        if let Err(e) = prepared.request.validate() {
            return UnitProductionOutcome::Failed {
                error: LlmError::InvalidRequest(e.to_string()),
                run,
            };
        }
        let response = match self.backend.execute_stream(prepared.request.clone()).await {
            Ok(response) => response,
            Err(error) => return UnitProductionOutcome::Failed { error, run },
        };
        run.observe(&response);
        // A missing provider finish reason does not prove that SSE ended early;
        // transport adapters need a separate terminal-event signal for that.
        let failure = reject_finish_reason(response.finish_reason.as_ref());
        if let Some(e) = failure {
            return UnitProductionOutcome::InvalidProposal {
                proposal: None,
                message: e.to_string(),
                report: None,
                run,
            };
        }
        let decoded = (|| -> Result<UnitDiscoveryProposal, String> {
            let text = collect_text(
                &response,
                self.limits
                    .max_response_bytes()
                    .min(prepared.manifest.max_response_bytes),
            )
            .map_err(|e| e.to_string())?;
            let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let proposal: UnitDiscoveryProposal =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            if serde_json::to_value(&proposal).expect("serializes") != value {
                return Err("noncanonical JSON: nullable fields must be explicit".into());
            }
            Ok(proposal)
        })();
        let proposal = match decoded {
            Ok(p) => p,
            Err(message) => {
                return UnitProductionOutcome::InvalidProposal {
                    proposal: None,
                    message,
                    report: None,
                    run,
                };
            }
        };
        match prepared.validate(proposal.clone(), self.limits.validation()) {
            Ok(units) => UnitProductionOutcome::Valid { units, run },
            Err(report) => UnitProductionOutcome::InvalidProposal {
                proposal: Some(proposal),
                message: report.to_string(),
                report: Some(report),
                run,
            },
        }
    }
}
