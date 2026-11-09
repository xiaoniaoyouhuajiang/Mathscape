use std::collections::{BTreeSet, HashMap, HashSet};

use mathscape_domain::{
    ContentHash, DISCOVERY_INPUT_VERSION, DiscoveryContextRequest, DiscoveryCoverageDisposition,
    DiscoveryCoverageProposal, DiscoveryInput, DiscoveryInputRegion, DiscoveryInputSegment,
    DiscoveryIssueKind, DiscoveryIssueProposal, DiscoveryProposal, DiscoveryQuoteAnchor,
    DiscoveryQuoteSelector, DiscoveryReferenceProposal, DiscoveryRelationKind,
    DiscoveryRelationProposal, DiscoverySelector, DiscoverySourceLabel, DiscoveryUnitKind,
    DiscoveryUnitProposal, MARKDOWN_ANCHOR_SCHEME, TextByteRange, UnitCompleteness,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::is_page_marker_text;
use crate::{
    discovery_input::discovery_input_fingerprint,
    selector_resolver::{MAX_DISAMBIGUATOR_CHARACTERS, context_matches},
};

pub const DEFAULT_PROPOSAL_ITEMS_PER_COLLECTION: usize = 1_024;
pub const DEFAULT_PROPOSAL_SELECTORS: usize = 4_096;
pub const DEFAULT_PROPOSAL_SERIALIZED_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryValidationLimits {
    pub(crate) max_items_per_collection: usize,
    pub(crate) max_total_selectors: usize,
    pub(crate) max_serialized_bytes: usize,
}

impl DiscoveryValidationLimits {
    pub fn new(
        max_items_per_collection: usize,
        max_total_selectors: usize,
        max_serialized_bytes: usize,
    ) -> Option<Self> {
        (max_items_per_collection > 0 && max_total_selectors > 0 && max_serialized_bytes > 0)
            .then_some(Self {
                max_items_per_collection,
                max_total_selectors,
                max_serialized_bytes,
            })
    }
}

impl Default for DiscoveryValidationLimits {
    fn default() -> Self {
        Self {
            max_items_per_collection: DEFAULT_PROPOSAL_ITEMS_PER_COLLECTION,
            max_total_selectors: DEFAULT_PROPOSAL_SELECTORS,
            max_serialized_bytes: DEFAULT_PROPOSAL_SERIALIZED_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryValidationCode {
    InputVersion,
    InputIntegrity,
    FingerprintMismatch,
    LimitExceeded,
    InvalidValue,
    DuplicateId,
    UnknownUnit,
    ParentCycle,
    InvalidSelector,
    InvalidRelation,
    InvalidReference,
    InvalidEvidence,
    InvalidCompleteness,
    OverlappingUnits,
    InvalidCoverage,
    CoverageGap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDiagnostic {
    code: DiscoveryValidationCode,
    path: String,
    message: String,
}

impl DiscoveryDiagnostic {
    pub fn code(&self) -> DiscoveryValidationCode {
        self.code
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Error)]
#[error("DiscoveryProposal 未通过确定性验证")]
pub struct DiscoveryValidationReport {
    diagnostics: Vec<DiscoveryDiagnostic>,
}

impl DiscoveryValidationReport {
    pub fn diagnostics(&self) -> &[DiscoveryDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedDiscoverySelector {
    selector: DiscoverySelector,
    range: TextByteRange,
    block_ids: Vec<String>,
    text: String,
}

impl ResolvedDiscoverySelector {
    pub fn selector(&self) -> &DiscoverySelector {
        &self.selector
    }

    pub fn range(&self) -> &TextByteRange {
        &self.range
    }

    pub fn block_ids(&self) -> &[String] {
        &self.block_ids
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// True when the selector contains only a Marker pagination separator.
    /// A selector may still legitimately cross a page marker while selecting
    /// mathematical content spanning two PDF pages.
    pub fn is_page_marker_only(&self) -> bool {
        is_page_marker_text(&self.text)
    }

    /// Returns the block identifiers from the persisted source input that
    /// physically contain this selector. Stage B plans split source blocks
    /// into internal `fragment:*` segments so that task scopes are disjoint;
    /// those identifiers are intentionally not part of the public Markdown
    /// anchor contract. Projection and registration must use this mapping to
    /// expose the original `block:*` identifiers instead.
    pub fn canonical_block_ids(&self, input: &DiscoveryInput) -> Vec<String> {
        input
            .requested
            .segments
            .iter()
            .filter_map(|segment| match segment {
                DiscoveryInputSegment::Block {
                    block_id,
                    source_range,
                    ..
                } if source_range.end > self.range.start && source_range.start < self.range.end => {
                    Some(block_id.clone())
                }
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryUnit {
    proposal: DiscoveryUnitProposal,
    selectors: Vec<ResolvedDiscoverySelector>,
    source_labels: Vec<ValidatedDiscoverySourceLabel>,
}

impl ValidatedDiscoveryUnit {
    pub fn proposal(&self) -> &DiscoveryUnitProposal {
        &self.proposal
    }

    pub fn selectors(&self) -> &[ResolvedDiscoverySelector] {
        &self.selectors
    }

    pub fn source_labels(&self) -> &[ValidatedDiscoverySourceLabel] {
        &self.source_labels
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoverySourceLabel {
    proposal: DiscoverySourceLabel,
    selector: ResolvedDiscoverySelector,
}

impl ValidatedDiscoverySourceLabel {
    pub fn proposal(&self) -> &DiscoverySourceLabel {
        &self.proposal
    }

    pub fn selector(&self) -> &ResolvedDiscoverySelector {
        &self.selector
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryRelation {
    proposal: DiscoveryRelationProposal,
    evidence: Vec<ResolvedDiscoverySelector>,
}

impl ValidatedDiscoveryRelation {
    pub fn proposal(&self) -> &DiscoveryRelationProposal {
        &self.proposal
    }

    pub fn evidence(&self) -> &[ResolvedDiscoverySelector] {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryReference {
    proposal: DiscoveryReferenceProposal,
    selector: ResolvedDiscoverySelector,
}

impl ValidatedDiscoveryReference {
    pub fn proposal(&self) -> &DiscoveryReferenceProposal {
        &self.proposal
    }

    pub fn selector(&self) -> &ResolvedDiscoverySelector {
        &self.selector
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryIssue {
    proposal: DiscoveryIssueProposal,
    evidence: Vec<ResolvedDiscoverySelector>,
}

impl ValidatedDiscoveryIssue {
    pub fn proposal(&self) -> &DiscoveryIssueProposal {
        &self.proposal
    }

    pub fn evidence(&self) -> &[ResolvedDiscoverySelector] {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryCoverage {
    proposal: DiscoveryCoverageProposal,
    selector: ResolvedDiscoverySelector,
}

impl ValidatedDiscoveryCoverage {
    pub fn proposal(&self) -> &DiscoveryCoverageProposal {
        &self.proposal
    }

    pub fn selector(&self) -> &ResolvedDiscoverySelector {
        &self.selector
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatedProposalStatus {
    Complete,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDiscoveryProposal {
    input_fingerprint: ContentHash,
    units: Vec<ValidatedDiscoveryUnit>,
    relations: Vec<ValidatedDiscoveryRelation>,
    references: Vec<ValidatedDiscoveryReference>,
    issues: Vec<ValidatedDiscoveryIssue>,
    coverage: Vec<ValidatedDiscoveryCoverage>,
    status: ValidatedProposalStatus,
}

impl ValidatedDiscoveryProposal {
    pub fn input_fingerprint(&self) -> &ContentHash {
        &self.input_fingerprint
    }

    pub fn units(&self) -> &[ValidatedDiscoveryUnit] {
        &self.units
    }

    pub fn relations(&self) -> &[ValidatedDiscoveryRelation] {
        &self.relations
    }

    pub fn references(&self) -> &[ValidatedDiscoveryReference] {
        &self.references
    }

    pub fn issues(&self) -> &[ValidatedDiscoveryIssue] {
        &self.issues
    }

    pub fn coverage(&self) -> &[ValidatedDiscoveryCoverage] {
        &self.coverage
    }

    pub fn status(&self) -> ValidatedProposalStatus {
        self.status
    }
}

#[derive(Clone)]
struct AuthorizedBlock {
    id: String,
    physical_range: TextByteRange,
    source_range: TextByteRange,
    text: String,
}

fn quote_occurrences(text: &str, quote: &str) -> Vec<usize> {
    if quote.is_empty() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let mut cursor = 0;
    while cursor <= text.len().saturating_sub(quote.len()) {
        let Some(relative) = text[cursor..].find(quote) else {
            break;
        };
        let start = cursor + relative;
        positions.push(start);
        cursor = start
            + text[start..]
                .chars()
                .next()
                .expect("nonempty quote match")
                .len_utf8();
    }
    positions
}

fn quote_position(block: &AuthorizedBlock, anchor: &DiscoveryQuoteAnchor) -> Result<u64, String> {
    if anchor.quote.is_empty() {
        return Err("quote 不能为空".into());
    }
    let positions = quote_occurrences(&block.text, &anchor.quote);
    let position = match anchor.occurrence {
        Some(0) => return Err("occurrence 从 1 开始，不能为 0".into()),
        Some(occurrence) => positions
            .get(usize::try_from(occurrence - 1).unwrap_or(usize::MAX))
            .copied()
            .ok_or_else(|| "occurrence 超出 quote 在指定块中的出现次数".to_owned())?,
        None => match positions.as_slice() {
            [position] => *position,
            [] => return Err("quote 未在指定块的可见文本中出现".into()),
            [_, ..] => return Err("quote 在指定块中不唯一且没有 occurrence".into()),
        },
    };
    for (name, value) in [("prefix", &anchor.prefix), ("suffix", &anchor.suffix)] {
        if let Some(value) = value
            && (value.trim().is_empty() || value.chars().count() > MAX_DISAMBIGUATOR_CHARACTERS)
        {
            return Err(format!(
                "{name} 必须为 1..={MAX_DISAMBIGUATOR_CHARACTERS} 个字符"
            ));
        }
    }
    if !context_matches(
        &block.text,
        (position, position + anchor.quote.len()),
        anchor,
    ) {
        return Err("occurrence 指定位置不满足 prefix/suffix".into());
    }
    Ok(block.source_range.start + position as u64)
}

pub(crate) struct AuthorizedRegion {
    pub(crate) range: TextByteRange,
    text: String,
    blocks: Vec<AuthorizedBlock>,
}

impl AuthorizedRegion {
    pub(crate) fn text_in(&self, range: &TextByteRange) -> Option<&str> {
        if range.start < self.range.start || range.end > self.range.end || range.start > range.end {
            return None;
        }
        let start = usize::try_from(range.start - self.range.start).ok()?;
        let end = usize::try_from(range.end - self.range.start).ok()?;
        self.text.get(start..end)
    }

    pub(crate) fn resolve(
        &self,
        selector: &DiscoverySelector,
    ) -> Result<ResolvedDiscoverySelector, String> {
        let find = |id: &str| {
            self.blocks
                .iter()
                .position(|block| block.id == id)
                .ok_or_else(|| format!("块 {id} 不在 requested 授权区域中"))
        };
        let (start, end) = match selector {
            DiscoverySelector::BlockRange { first, last } => {
                let (first_index, last_index) = (find(first)?, find(last)?);
                if first_index > last_index {
                    return Err("block_range 的终点位于起点之前".into());
                }
                if self.blocks[first_index..=last_index]
                    .iter()
                    .any(|block| block.physical_range != block.source_range)
                {
                    return Err("block_range 包含仅部分可见的物理块，请改用 quote_range".into());
                }
                (
                    self.blocks[first_index].source_range.start,
                    self.blocks[last_index].source_range.end,
                )
            }
            DiscoverySelector::QuoteRange {
                first,
                start: start_anchor,
                last,
                end: end_anchor,
            } => {
                let (first_index, last_index) = (find(first)?, find(last)?);
                if first_index > last_index {
                    return Err("quote_range 的终点块位于起点块之前".into());
                }
                let start = quote_position(&self.blocks[first_index], start_anchor)?;
                let end_start = quote_position(&self.blocks[last_index], end_anchor)?;
                let end = end_start + end_anchor.quote.len() as u64;
                if end_start < start || end < start + start_anchor.quote.len() as u64 {
                    return Err("quote_range 的终点位于起点之前".into());
                }
                (start, end)
            }
        };
        let range = TextByteRange { start, end };
        let text = self
            .text_in(&range)
            .ok_or_else(|| "选择器没有解析到合法 UTF-8 范围".to_owned())?
            .to_owned();
        if text.trim().is_empty() {
            return Err("选择器不能只包含空白".into());
        }
        let block_ids = self
            .blocks
            .iter()
            .filter(|block| block.source_range.end > start && block.source_range.start < end)
            .map(|block| block.id.clone())
            .collect();
        Ok(ResolvedDiscoverySelector {
            selector: selector.clone(),
            range,
            block_ids,
            text,
        })
    }

    fn resolve_quote(
        &self,
        selector: &DiscoveryQuoteSelector,
    ) -> Result<ResolvedDiscoverySelector, String> {
        self.resolve(&DiscoverySelector::from(selector.clone()))
    }
}

fn diagnostic(
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
    code: DiscoveryValidationCode,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(DiscoveryDiagnostic {
        code,
        path: path.into(),
        message: message.into(),
    });
}

fn hash<T: Serialize>(value: &T) -> ContentHash {
    let bytes = serde_json::to_vec(value).expect("validation fingerprint values are serializable");
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("SHA-256 always produces a valid content hash")
}

fn validate_region(
    region: &DiscoveryInputRegion,
    path: &str,
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
) -> Option<AuthorizedRegion> {
    if region.source_range.start >= region.source_range.end || region.segments.is_empty() {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputIntegrity,
            path,
            "输入区域必须为非空半开区间并包含 segment",
        );
        return None;
    }
    let mut cursor = region.source_range.start;
    let mut text = String::new();
    let mut blocks = Vec::new();
    let mut block_ids = HashSet::new();
    for (index, segment) in region.segments.iter().enumerate() {
        let segment_path = format!("{path}.segments[{index}]");
        let (range, segment_text) = match segment {
            DiscoveryInputSegment::Block {
                block_id,
                physical_range,
                source_range,
                text,
                ..
            } => {
                if block_id.trim().is_empty() || !block_ids.insert(block_id.clone()) {
                    diagnostic(
                        diagnostics,
                        DiscoveryValidationCode::InputIntegrity,
                        &segment_path,
                        "block_id 必须非空且在区域内唯一",
                    );
                }
                if physical_range.start > source_range.start
                    || physical_range.end < source_range.end
                    || physical_range.start >= physical_range.end
                {
                    diagnostic(
                        diagnostics,
                        DiscoveryValidationCode::InputIntegrity,
                        &segment_path,
                        "physical_range 必须包含非空 source_range",
                    );
                }
                blocks.push(AuthorizedBlock {
                    id: block_id.clone(),
                    physical_range: physical_range.clone(),
                    source_range: source_range.clone(),
                    text: text.clone(),
                });
                (source_range, text)
            }
            DiscoveryInputSegment::Gap { source_range, text } => (source_range, text),
        };
        let expected_len = range.end.checked_sub(range.start);
        if range.start != cursor
            || range.start >= range.end
            || expected_len != Some(segment_text.len() as u64)
        {
            diagnostic(
                diagnostics,
                DiscoveryValidationCode::InputIntegrity,
                &segment_path,
                "segment 必须连续覆盖区域，且 UTF-8 字节数与范围一致",
            );
        }
        cursor = range.end;
        text.push_str(segment_text);
    }
    if cursor != region.source_range.end
        || text.len() as u64 != region.source_range.end - region.source_range.start
    {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputIntegrity,
            path,
            "segments 未无损覆盖 source_range",
        );
    }
    Some(AuthorizedRegion {
        range: region.source_range.clone(),
        text,
        blocks,
    })
}

fn nonblank(value: &str, path: &str, diagnostics: &mut Vec<DiscoveryDiagnostic>) {
    if value.trim().is_empty() {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InvalidValue,
            path,
            "字符串不能为空或只有空白",
        );
    }
}

pub fn valid_temp_id(value: &str) -> bool {
    value.len() <= 64
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn contains_range(outer: &ResolvedDiscoverySelector, inner: &ResolvedDiscoverySelector) -> bool {
    outer.range.start <= inner.range.start && outer.range.end >= inner.range.end
}

fn unit_contains(unit: &[ResolvedDiscoverySelector], selector: &ResolvedDiscoverySelector) -> bool {
    unit.iter().any(|outer| contains_range(outer, selector))
}

fn resolve_many(
    region: &AuthorizedRegion,
    selectors: &[DiscoverySelector],
    path: &str,
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
) -> Vec<ResolvedDiscoverySelector> {
    let mut resolved = Vec::new();
    for (index, selector) in selectors.iter().enumerate() {
        match region.resolve(selector) {
            Ok(value) if value.is_page_marker_only() => diagnostic(
                diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                format!("{path}[{index}]"),
                "分页标记是非语义元数据，不能作为数学内容、关系或证据的唯一范围",
            ),
            Ok(value) => resolved.push(value),
            Err(message) => diagnostic(
                diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                format!("{path}[{index}]"),
                message,
            ),
        }
    }
    resolved.sort_by_key(|selector| (selector.range.start, selector.range.end));
    for pair in resolved.windows(2) {
        if pair[0].range.end > pair[1].range.start {
            diagnostic(
                diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                path,
                "同一对象的选择器不能重复或重叠",
            );
            break;
        }
    }
    resolved
}

pub(crate) fn relation_types_valid(
    relation: DiscoveryRelationKind,
    source: DiscoveryUnitKind,
    target: DiscoveryUnitKind,
) -> bool {
    match relation {
        DiscoveryRelationKind::Proves => {
            source == DiscoveryUnitKind::Proof && target == DiscoveryUnitKind::Statement
        }
        DiscoveryRelationKind::Solves => {
            matches!(
                source,
                DiscoveryUnitKind::Proof | DiscoveryUnitKind::Solution
            ) && target == DiscoveryUnitKind::Problem
        }
        DiscoveryRelationKind::Hints => {
            source == DiscoveryUnitKind::Hint && target == DiscoveryUnitKind::Problem
        }
        DiscoveryRelationKind::AppliesMethod => target == DiscoveryUnitKind::Method,
        DiscoveryRelationKind::Explains | DiscoveryRelationKind::Cites => true,
    }
}

pub(crate) fn is_ancestor(parent_by_id: &HashMap<&str, &str>, ancestor: &str, child: &str) -> bool {
    let mut current = child;
    let mut visited = HashSet::new();
    while let Some(parent) = parent_by_id.get(current).copied() {
        if parent == ancestor {
            return true;
        }
        if !visited.insert(parent) {
            return false;
        }
        current = parent;
    }
    false
}

fn units_overlap(a: &[ResolvedDiscoverySelector], b: &[ResolvedDiscoverySelector]) -> bool {
    a.iter().any(|left| {
        b.iter()
            .any(|right| left.range.end > right.range.start && right.range.end > left.range.start)
    })
}

pub(crate) fn authorize_discovery_input(
    input: &DiscoveryInput,
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
) -> Option<AuthorizedRegion> {
    let initial_errors = diagnostics.len();
    if input.version != DISCOVERY_INPUT_VERSION {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputVersion,
            "input.version",
            "验证器只接受当前 DiscoveryInput 版本",
        );
    }
    if input.snapshot.anchor_scheme != MARKDOWN_ANCHOR_SCHEME || !input.context.is_empty() {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputIntegrity,
            "input",
            "当前输入必须使用已知 Markdown 锚点方案且 context 为空",
        );
    }
    if input.boundary.at_document_start != (input.requested.source_range.start == 0) {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputIntegrity,
            "input.boundary.at_document_start",
            "文档起点标记与 requested 范围不一致",
        );
    }
    let expected_selection = hash(&(&input.snapshot.fingerprint, &input.requested.source_range));
    if input.selection_fingerprint != expected_selection
        || input.fingerprint != discovery_input_fingerprint(input)
    {
        diagnostic(
            diagnostics,
            DiscoveryValidationCode::InputIntegrity,
            "input.fingerprint",
            "DiscoveryInput 指纹与可见内容不一致",
        );
    }
    let authorized = validate_region(&input.requested, "input.requested", diagnostics);
    if diagnostics.len() != initial_errors {
        return None;
    }
    authorized
}

/// Validates an untrusted producer proposal against the exact requested region.
/// A successful value contains normalized byte ranges but grants no map authority.
pub fn validate_discovery_proposal(
    input: &DiscoveryInput,
    proposal: DiscoveryProposal,
    limits: DiscoveryValidationLimits,
) -> Result<ValidatedDiscoveryProposal, DiscoveryValidationReport> {
    let mut diagnostics = Vec::new();
    let authorized = authorize_discovery_input(input, &mut diagnostics);

    if proposal.input_fingerprint != input.fingerprint {
        diagnostic(
            &mut diagnostics,
            DiscoveryValidationCode::FingerprintMismatch,
            "proposal.input_fingerprint",
            "提案没有回显当前 DiscoveryInput 指纹",
        );
    }
    let collections = [
        ("units", proposal.units.len()),
        ("relations", proposal.relations.len()),
        ("references", proposal.references.len()),
        ("issues", proposal.issues.len()),
        ("coverage", proposal.coverage.len()),
    ];
    for (name, actual) in collections {
        if actual > limits.max_items_per_collection {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::LimitExceeded,
                format!("proposal.{name}"),
                format!(
                    "集合含 {actual} 项，超过限制 {}",
                    limits.max_items_per_collection
                ),
            );
        }
    }
    let selector_count = proposal
        .units
        .iter()
        .map(|unit| unit.selectors.len() + unit.source_labels.len())
        .sum::<usize>()
        + proposal
            .relations
            .iter()
            .map(|relation| relation.evidence.len())
            .sum::<usize>()
        + proposal.references.len()
        + proposal
            .issues
            .iter()
            .map(|issue| issue.evidence.len())
            .sum::<usize>()
        + proposal.coverage.len();
    if selector_count > limits.max_total_selectors {
        diagnostic(
            &mut diagnostics,
            DiscoveryValidationCode::LimitExceeded,
            "proposal",
            format!(
                "提案含 {selector_count} 个选择器，超过限制 {}",
                limits.max_total_selectors
            ),
        );
    }
    let serialized_bytes = serde_json::to_vec(&proposal).map_or(usize::MAX, |bytes| bytes.len());
    if serialized_bytes > limits.max_serialized_bytes {
        diagnostic(
            &mut diagnostics,
            DiscoveryValidationCode::LimitExceeded,
            "proposal",
            format!(
                "提案序列化后为 {serialized_bytes} 字节，超过限制 {}",
                limits.max_serialized_bytes
            ),
        );
    }

    let Some(authorized) = authorized else {
        return Err(DiscoveryValidationReport { diagnostics });
    };
    let mut unit_index = HashMap::new();
    let mut resolved_units = Vec::with_capacity(proposal.units.len());
    let mut resolved_source_labels = Vec::with_capacity(proposal.units.len());
    for (index, unit) in proposal.units.iter().enumerate() {
        let path = format!("proposal.units[{index}]");
        if !valid_temp_id(&unit.temp_id) {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidValue,
                format!("{path}.temp_id"),
                "临时 ID 必须为 1..64 个 ASCII 字母、数字、点、横线或下划线，且以字母或数字开头",
            );
        }
        if unit_index.insert(unit.temp_id.as_str(), index).is_some() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::DuplicateId,
                format!("{path}.temp_id"),
                "临时 ID 在提案中重复",
            );
        }
        for (name, value) in [
            ("source_title", unit.source_title.as_deref()),
            ("display_title", unit.display_title.as_deref()),
        ] {
            if let Some(value) = value {
                nonblank(value, &format!("{path}.{name}"), &mut diagnostics);
            }
        }
        if unit.selectors.is_empty() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                format!("{path}.selectors"),
                "来源单元至少需要一个选择器",
            );
        }
        let resolved = resolve_many(
            &authorized,
            &unit.selectors,
            &format!("{path}.selectors"),
            &mut diagnostics,
        );
        let unit_text = resolved
            .iter()
            .map(|selector| selector.text.as_str())
            .collect::<String>();
        if let Some(value) = unit.source_title.as_deref()
            && !value.trim().is_empty()
            && !unit_text.contains(value)
        {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidEvidence,
                format!("{path}.source_title"),
                "声明为原文的字段没有出现在单元选择器文本中",
            );
        }
        let mut labels = Vec::with_capacity(unit.source_labels.len());
        for (label_index, label) in unit.source_labels.iter().enumerate() {
            let label_path = format!("{path}.source_labels[{label_index}]");
            nonblank(
                &label.surface_form,
                &format!("{label_path}.surface_form"),
                &mut diagnostics,
            );
            match authorized.resolve_quote(&label.selector) {
                Ok(selector) => {
                    if selector.is_page_marker_only() {
                        diagnostic(
                            &mut diagnostics,
                            DiscoveryValidationCode::InvalidSelector,
                            format!("{label_path}.selector"),
                            "分页标记是非语义元数据，不能作为来源标签",
                        );
                        continue;
                    }
                    if !unit_contains(&resolved, &selector) {
                        diagnostic(
                            &mut diagnostics,
                            DiscoveryValidationCode::InvalidEvidence,
                            format!("{label_path}.selector"),
                            "来源标签必须位于所属单元中",
                        );
                    }
                    if !label.surface_form.trim().is_empty() && selector.text != label.surface_form
                    {
                        diagnostic(
                            &mut diagnostics,
                            DiscoveryValidationCode::InvalidEvidence,
                            format!("{label_path}.surface_form"),
                            "来源标签必须与其精确 quote selector 文本完全一致",
                        );
                    }
                    labels.push(selector);
                }
                Err(message) => {
                    diagnostic(
                        &mut diagnostics,
                        DiscoveryValidationCode::InvalidSelector,
                        format!("{label_path}.selector"),
                        message,
                    );
                }
            }
        }
        resolved_source_labels.push(labels);
        resolved_units.push(resolved);
    }

    let mut parent_by_id = HashMap::new();
    for (index, unit) in proposal.units.iter().enumerate() {
        if let Some(parent) = unit.parent.as_deref() {
            if parent == unit.temp_id || !unit_index.contains_key(parent) {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::UnknownUnit,
                    format!("proposal.units[{index}].parent"),
                    "parent 必须引用另一个已声明的临时单元",
                );
            } else {
                parent_by_id.insert(unit.temp_id.as_str(), parent);
            }
        }
    }
    let mut reported_cycles = HashSet::new();
    for unit in &proposal.units {
        let mut current = unit.temp_id.as_str();
        let mut chain = HashSet::new();
        while let Some(parent) = parent_by_id.get(current).copied() {
            if !chain.insert(current) {
                if reported_cycles.insert(current) {
                    diagnostic(
                        &mut diagnostics,
                        DiscoveryValidationCode::ParentCycle,
                        "proposal.units",
                        format!("parent 链在 {current} 形成环"),
                    );
                }
                break;
            }
            current = parent;
        }
    }

    let mut relation_keys = HashSet::new();
    let mut resolved_relations = Vec::with_capacity(proposal.relations.len());
    let mut explains = HashSet::new();
    for (index, relation) in proposal.relations.iter().enumerate() {
        let path = format!("proposal.relations[{index}]");
        nonblank(&relation.basis, &format!("{path}.basis"), &mut diagnostics);
        let source = unit_index.get(relation.source.as_str()).copied();
        let target = unit_index.get(relation.target.as_str()).copied();
        if source.is_none() || target.is_none() || relation.source == relation.target {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::UnknownUnit,
                &path,
                "关系端点必须引用两个不同的已声明单元",
            );
        }
        let key = (
            relation.source.as_str(),
            relation.target.as_str(),
            format!("{:?}", relation.relation),
        );
        if !relation_keys.insert(key) {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidRelation,
                &path,
                "关系三元组重复",
            );
        }
        if let (Some(source), Some(target)) = (source, target)
            && !relation_types_valid(
                relation.relation,
                proposal.units[source].unit_kind,
                proposal.units[target].unit_kind,
            )
        {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidRelation,
                &path,
                "关系方向与端点单元类型不相容",
            );
        }
        if relation.relation == DiscoveryRelationKind::Explains {
            explains.insert((relation.source.as_str(), relation.target.as_str()));
        }
        if relation.evidence.is_empty() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidEvidence,
                format!("{path}.evidence"),
                "来源关系至少需要一段证据",
            );
        }
        let evidence = resolve_many(
            &authorized,
            &relation.evidence,
            &format!("{path}.evidence"),
            &mut diagnostics,
        );
        if let Some(source) = source {
            for item in &evidence {
                if !unit_contains(&resolved_units[source], item) {
                    diagnostic(
                        &mut diagnostics,
                        DiscoveryValidationCode::InvalidEvidence,
                        format!("{path}.evidence"),
                        "关系证据必须位于 source 单元中",
                    );
                }
            }
        }
        resolved_relations.push(evidence);
    }

    for left in 0..proposal.units.len() {
        for right in left + 1..proposal.units.len() {
            let a = proposal.units[left].temp_id.as_str();
            let b = proposal.units[right].temp_id.as_str();
            if units_overlap(&resolved_units[left], &resolved_units[right])
                && !is_ancestor(&parent_by_id, a, b)
                && !is_ancestor(&parent_by_id, b, a)
                && !explains.contains(&(a, b))
                && !explains.contains(&(b, a))
            {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::OverlappingUnits,
                    format!("proposal.units[{left}],proposal.units[{right}]"),
                    "重叠单元必须由 parent 祖先关系或 explains 关系说明",
                );
            }
        }
    }

    let mut resolved_references = Vec::with_capacity(proposal.references.len());
    for (index, reference) in proposal.references.iter().enumerate() {
        let path = format!("proposal.references[{index}]");
        nonblank(
            &reference.surface_form,
            &format!("{path}.surface_form"),
            &mut diagnostics,
        );
        if let Some(label) = reference.target_label.as_deref() {
            nonblank(label, &format!("{path}.target_label"), &mut diagnostics);
        }
        let source = unit_index.get(reference.source.as_str()).copied();
        if source.is_none() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::UnknownUnit,
                format!("{path}.source"),
                "引用者必须是已声明单元",
            );
        }
        if let Some(target) = reference.target.as_deref() {
            if target == reference.source {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::InvalidReference,
                    format!("{path}.target"),
                    "引用目标不能与引用者是同一个单元；没有独立目标时应填 null",
                );
            } else if !unit_index.contains_key(target) {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::UnknownUnit,
                    format!("{path}.target"),
                    "候选引用目标必须是本提案中的单元",
                );
            }
        }
        let selector = match authorized.resolve_quote(&reference.selector) {
            Ok(selector) => selector,
            Err(message) => {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::InvalidSelector,
                    format!("{path}.selector"),
                    message,
                );
                continue;
            }
        };
        if selector.is_page_marker_only() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                format!("{path}.selector"),
                "分页标记是非语义元数据，不能作为引用位置",
            );
            continue;
        }
        if let Some(source) = source
            && !unit_contains(&resolved_units[source], &selector)
        {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidEvidence,
                format!("{path}.selector"),
                "引用出现位置必须位于引用者单元中",
            );
        }
        if !reference.surface_form.trim().is_empty() && selector.text != reference.surface_form {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidEvidence,
                format!("{path}.surface_form"),
                "引用原文必须与其精确 quote selector 文本完全一致",
            );
        }
        resolved_references.push(Some(selector));
    }

    let mut resolved_issues = Vec::with_capacity(proposal.issues.len());
    let mut issue_kinds_by_unit: HashMap<&str, BTreeSet<DiscoveryIssueKind>> = HashMap::new();
    for (index, issue) in proposal.issues.iter().enumerate() {
        let path = format!("proposal.issues[{index}]");
        nonblank(
            &issue.observation,
            &format!("{path}.observation"),
            &mut diagnostics,
        );
        let unit = issue
            .unit
            .as_deref()
            .and_then(|unit| unit_index.get(unit).copied());
        if issue.unit.is_some() && unit.is_none() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::UnknownUnit,
                format!("{path}.unit"),
                "issue.unit 必须引用已声明单元",
            );
        }
        if issue.unit.is_none() && issue.evidence.is_empty() {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidEvidence,
                &path,
                "issue 至少需要受影响单元或证据范围",
            );
        }
        if let Some(unit_id) = issue.unit.as_deref() {
            issue_kinds_by_unit
                .entry(unit_id)
                .or_default()
                .insert(issue.kind);
        }
        if let Some(DiscoveryContextRequest::LocalReference { target_label }) =
            &issue.context_request
        {
            nonblank(
                target_label,
                &format!("{path}.context_request.target_label"),
                &mut diagnostics,
            );
        }
        let evidence = resolve_many(
            &authorized,
            &issue.evidence,
            &format!("{path}.evidence"),
            &mut diagnostics,
        );
        if let Some(unit) = unit {
            for item in &evidence {
                if !unit_contains(&resolved_units[unit], item) {
                    diagnostic(
                        &mut diagnostics,
                        DiscoveryValidationCode::InvalidEvidence,
                        format!("{path}.evidence"),
                        "带 unit 的 issue 证据必须位于该单元中",
                    );
                }
            }
        }
        resolved_issues.push(evidence);
    }
    for (index, unit) in proposal.units.iter().enumerate() {
        let kinds = issue_kinds_by_unit.get(unit.temp_id.as_str());
        let consistent = match unit.completeness {
            UnitCompleteness::Complete => true,
            UnitCompleteness::LeftTruncated => {
                kinds.is_some_and(|set| set.contains(&DiscoveryIssueKind::LeftTruncated))
            }
            UnitCompleteness::RightTruncated => {
                kinds.is_some_and(|set| set.contains(&DiscoveryIssueKind::RightTruncated))
            }
            UnitCompleteness::InternallyMissing => {
                kinds.is_some_and(|set| set.contains(&DiscoveryIssueKind::InternallyMissing))
            }
            UnitCompleteness::Uncertain => kinds.is_some_and(|set| !set.is_empty()),
        };
        if !consistent {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidCompleteness,
                format!("proposal.units[{index}].completeness"),
                "非 complete 状态必须有与该单元一致的 issue",
            );
        }
        if let Some(kinds) = kinds {
            for kind in kinds {
                let valid = match kind {
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
                };
                if !valid {
                    diagnostic(
                        &mut diagnostics,
                        DiscoveryValidationCode::InvalidCompleteness,
                        format!("proposal.units[{index}].completeness"),
                        "截断或缺失 issue 与单元 completeness 不一致",
                    );
                }
            }
        }
    }

    if proposal.coverage.is_empty() {
        diagnostic(
            &mut diagnostics,
            DiscoveryValidationCode::InvalidCoverage,
            "proposal.coverage",
            "非空 requested 区域必须有 coverage",
        );
    }
    let mut resolved_coverage = Vec::with_capacity(proposal.coverage.len());
    for (index, coverage) in proposal.coverage.iter().enumerate() {
        let path = format!("proposal.coverage[{index}]");
        match &coverage.disposition {
            DiscoveryCoverageDisposition::Skipped { reason }
            | DiscoveryCoverageDisposition::Unprocessed { reason } => {
                nonblank(
                    reason,
                    &format!("{path}.disposition.reason"),
                    &mut diagnostics,
                );
            }
            DiscoveryCoverageDisposition::Processed {} => {}
        }
        match authorized.resolve(&coverage.selector) {
            Ok(selector) => resolved_coverage.push((index, selector)),
            Err(message) => diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidSelector,
                format!("{path}.selector"),
                message,
            ),
        }
    }
    resolved_coverage.sort_by_key(|(_, selector)| (selector.range.start, selector.range.end));
    let mut cursor = authorized.range.start;
    for (_, selector) in &resolved_coverage {
        if selector.range.start < cursor {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::InvalidCoverage,
                "proposal.coverage",
                "coverage 范围不能重复或重叠",
            );
        } else if selector.range.start > cursor {
            let gap = TextByteRange {
                start: cursor,
                end: selector.range.start,
            };
            if authorized
                .text_in(&gap)
                .is_some_and(|text| !text.trim().is_empty())
            {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::CoverageGap,
                    "proposal.coverage",
                    format!(
                        "非空白 requested 区间 [{}, {}) 未被覆盖",
                        gap.start, gap.end
                    ),
                );
            }
        }
        cursor = cursor.max(selector.range.end);
    }
    if cursor < authorized.range.end {
        let gap = TextByteRange {
            start: cursor,
            end: authorized.range.end,
        };
        if authorized
            .text_in(&gap)
            .is_some_and(|text| !text.trim().is_empty())
        {
            diagnostic(
                &mut diagnostics,
                DiscoveryValidationCode::CoverageGap,
                "proposal.coverage",
                format!(
                    "非空白 requested 区间 [{}, {}) 未被覆盖",
                    gap.start, gap.end
                ),
            );
        }
    }
    for (unit_index_value, selectors) in resolved_units.iter().enumerate() {
        for selector in selectors {
            let processed = resolved_coverage.iter().any(|(coverage_index, coverage)| {
                matches!(
                    proposal.coverage[*coverage_index].disposition,
                    DiscoveryCoverageDisposition::Processed {}
                ) && contains_range(coverage, selector)
            });
            if !processed {
                diagnostic(
                    &mut diagnostics,
                    DiscoveryValidationCode::InvalidCoverage,
                    format!("proposal.units[{unit_index_value}].selectors"),
                    "来源单元必须落在 processed coverage 中",
                );
            }
        }
    }

    if !diagnostics.is_empty() {
        return Err(DiscoveryValidationReport { diagnostics });
    }

    let units = proposal
        .units
        .iter()
        .cloned()
        .zip(resolved_units)
        .zip(resolved_source_labels)
        .map(|((proposal, selectors), source_labels)| {
            let source_labels = proposal
                .source_labels
                .iter()
                .cloned()
                .zip(source_labels)
                .map(|(proposal, selector)| ValidatedDiscoverySourceLabel { proposal, selector })
                .collect();
            ValidatedDiscoveryUnit {
                proposal,
                selectors,
                source_labels,
            }
        })
        .collect();
    let relations = proposal
        .relations
        .iter()
        .cloned()
        .zip(resolved_relations)
        .map(|(proposal, evidence)| ValidatedDiscoveryRelation { proposal, evidence })
        .collect();
    let references = proposal
        .references
        .iter()
        .cloned()
        .zip(resolved_references)
        .filter_map(|(proposal, selector)| {
            selector.map(|selector| ValidatedDiscoveryReference { proposal, selector })
        })
        .collect();
    let issues = proposal
        .issues
        .iter()
        .cloned()
        .zip(resolved_issues)
        .map(|(proposal, evidence)| ValidatedDiscoveryIssue { proposal, evidence })
        .collect();
    let coverage = resolved_coverage
        .into_iter()
        .map(|(index, selector)| ValidatedDiscoveryCoverage {
            proposal: proposal.coverage[index].clone(),
            selector,
        })
        .collect();
    let status = if proposal.coverage.iter().any(|coverage| {
        matches!(
            coverage.disposition,
            DiscoveryCoverageDisposition::Unprocessed { .. }
        )
    }) {
        ValidatedProposalStatus::Partial
    } else {
        ValidatedProposalStatus::Complete
    };
    Ok(ValidatedDiscoveryProposal {
        input_fingerprint: proposal.input_fingerprint,
        units,
        relations,
        references,
        issues,
        coverage,
        status,
    })
}
