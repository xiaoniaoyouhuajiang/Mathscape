use std::collections::{HashMap, HashSet};

use mathscape_domain::{
    ContentHash, DiscoveryInput, DiscoveryInputBoundary, DiscoveryInputRegion,
    DiscoveryInputSegment, DiscoveryProposal, DiscoveryQuoteAnchor, DiscoveryQuoteSelector,
    DiscoverySelector,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DISCOVERY_MODEL_INPUT_VERSION: &str = "mathscape-discovery-model-input-1";

/// Minimal model-facing view of one trusted DiscoveryInput. Stable hashes,
/// byte offsets and artifact identities remain in the orchestrator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryModelInput {
    pub version: String,
    pub source_input_version: String,
    pub input_fingerprint: ContentHash,
    pub requested: DiscoveryModelInputRegion,
    pub context: Vec<DiscoveryModelInputRegion>,
    pub boundary: DiscoveryInputBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryModelInputRegion {
    pub segments: Vec<DiscoveryModelInputSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "segment_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryModelInputSegment {
    Block {
        block_ref: String,
        structural_kind: String,
        clipped_start: bool,
        clipped_end: bool,
        text: String,
    },
    Gap {
        text: String,
    },
}

impl DiscoveryModelInputSegment {
    pub fn block_ref(&self) -> Option<&str> {
        match self {
            Self::Block { block_ref, .. } => Some(block_ref),
            Self::Gap { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectorBinding {
    handle: String,
    block_id: String,
    text: String,
}

impl SelectorBinding {
    pub fn handle(&self) -> &str {
        &self.handle
    }

    pub fn block_id(&self) -> &str {
        &self.block_id
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectorResolutionCode {
    UnknownBlockRef,
    QuoteNotFound,
    AmbiguousQuote,
    InvalidDisambiguator,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectorResolutionDiagnostic {
    code: SelectorResolutionCode,
    path: String,
    message: String,
}

impl SelectorResolutionDiagnostic {
    pub fn code(&self) -> SelectorResolutionCode {
        self.code
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectorRewriteKind {
    BlockHandle,
    SafeQuoteNormalization,
    OccurrenceBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectorRewrite {
    kind: SelectorRewriteKind,
    path: String,
    original: String,
    resolved: String,
}

impl SelectorRewrite {
    pub fn kind(&self) -> SelectorRewriteKind {
        self.kind
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn resolved(&self) -> &str {
        &self.resolved
    }
}

#[derive(Debug, Error)]
#[error("DiscoveryProposal 的来源定位意图无法唯一解析")]
pub struct SelectorResolutionReport {
    diagnostics: Vec<SelectorResolutionDiagnostic>,
}

impl SelectorResolutionReport {
    pub fn diagnostics(&self) -> &[SelectorResolutionDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSelectorProposal {
    proposal: DiscoveryProposal,
    rewrites: Vec<SelectorRewrite>,
}

impl ResolvedSelectorProposal {
    pub fn proposal(&self) -> &DiscoveryProposal {
        &self.proposal
    }

    pub fn into_proposal(self) -> DiscoveryProposal {
        self.proposal
    }

    pub fn rewrites(&self) -> &[SelectorRewrite] {
        &self.rewrites
    }
}

/// Request-local name binding and conservative textual locator resolution.
/// It never searches outside the exact source blocks visible in the input.
#[derive(Clone, Debug)]
pub struct SelectorResolver {
    bindings: Vec<SelectorBinding>,
    by_handle: HashMap<String, usize>,
    by_block_id: HashMap<String, usize>,
}

impl SelectorResolver {
    pub fn from_input(input: &DiscoveryInput) -> Self {
        let mut bindings = Vec::new();
        let mut seen = HashSet::new();
        for region in std::iter::once(&input.requested).chain(input.context.iter()) {
            for segment in &region.segments {
                let DiscoveryInputSegment::Block { block_id, text, .. } = segment else {
                    continue;
                };
                if seen.insert(block_id.clone()) {
                    bindings.push(SelectorBinding {
                        handle: format!("b{}", bindings.len() + 1),
                        block_id: block_id.clone(),
                        text: text.clone(),
                    });
                }
            }
        }
        let by_handle = bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| (binding.handle.clone(), index))
            .collect();
        let by_block_id = bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| (binding.block_id.clone(), index))
            .collect();
        Self {
            bindings,
            by_handle,
            by_block_id,
        }
    }

    pub fn bindings(&self) -> &[SelectorBinding] {
        &self.bindings
    }

    /// Resolve one locator without manufacturing a whole DiscoveryProposal.
    pub fn resolve_selector(
        &self,
        mut selector: DiscoverySelector,
        path: &str,
    ) -> Result<(DiscoverySelector, Vec<SelectorRewrite>), SelectorResolutionReport> {
        let mut diagnostics = Vec::new();
        let mut rewrites = Vec::new();
        self.resolve_one(&mut selector, path, &mut diagnostics, &mut rewrites);
        if diagnostics.is_empty() {
            Ok((selector, rewrites))
        } else {
            Err(SelectorResolutionReport { diagnostics })
        }
    }

    pub fn model_input(&self, input: &DiscoveryInput) -> DiscoveryModelInput {
        DiscoveryModelInput {
            version: DISCOVERY_MODEL_INPUT_VERSION.into(),
            source_input_version: input.version.clone(),
            input_fingerprint: input.fingerprint.clone(),
            requested: self.project_region(&input.requested),
            context: input
                .context
                .iter()
                .map(|region| self.project_region(region))
                .collect(),
            boundary: input.boundary.clone(),
        }
    }

    pub fn resolve(
        &self,
        mut proposal: DiscoveryProposal,
    ) -> Result<ResolvedSelectorProposal, SelectorResolutionReport> {
        let mut diagnostics = Vec::new();
        let mut rewrites = Vec::new();

        for (unit_index, unit) in proposal.units.iter_mut().enumerate() {
            for (label_index, label) in unit.source_labels.iter_mut().enumerate() {
                self.resolve_quote_one(
                    &mut label.selector,
                    &format!("proposal.units[{unit_index}].source_labels[{label_index}].selector"),
                    &mut diagnostics,
                    &mut rewrites,
                );
            }
            self.resolve_many(
                &mut unit.selectors,
                &format!("proposal.units[{unit_index}].selectors"),
                &mut diagnostics,
                &mut rewrites,
            );
        }
        for (relation_index, relation) in proposal.relations.iter_mut().enumerate() {
            self.resolve_many(
                &mut relation.evidence,
                &format!("proposal.relations[{relation_index}].evidence"),
                &mut diagnostics,
                &mut rewrites,
            );
        }
        for (reference_index, reference) in proposal.references.iter_mut().enumerate() {
            self.resolve_quote_one(
                &mut reference.selector,
                &format!("proposal.references[{reference_index}].selector"),
                &mut diagnostics,
                &mut rewrites,
            );
        }
        for (issue_index, issue) in proposal.issues.iter_mut().enumerate() {
            self.resolve_many(
                &mut issue.evidence,
                &format!("proposal.issues[{issue_index}].evidence"),
                &mut diagnostics,
                &mut rewrites,
            );
        }
        for (coverage_index, coverage) in proposal.coverage.iter_mut().enumerate() {
            self.resolve_one(
                &mut coverage.selector,
                &format!("proposal.coverage[{coverage_index}].selector"),
                &mut diagnostics,
                &mut rewrites,
            );
        }

        if diagnostics.is_empty() {
            Ok(ResolvedSelectorProposal { proposal, rewrites })
        } else {
            Err(SelectorResolutionReport { diagnostics })
        }
    }

    fn project_region(&self, region: &DiscoveryInputRegion) -> DiscoveryModelInputRegion {
        DiscoveryModelInputRegion {
            segments: region
                .segments
                .iter()
                .map(|segment| match segment {
                    DiscoveryInputSegment::Block {
                        block_id,
                        structural_kind,
                        physical_range,
                        source_range,
                        text,
                    } => DiscoveryModelInputSegment::Block {
                        block_ref: self
                            .binding_for_id(block_id)
                            .expect("bindings originate from this input")
                            .handle
                            .clone(),
                        structural_kind: structural_kind.clone(),
                        clipped_start: source_range.start > physical_range.start,
                        clipped_end: source_range.end < physical_range.end,
                        text: text.clone(),
                    },
                    DiscoveryInputSegment::Gap { text, .. } => {
                        DiscoveryModelInputSegment::Gap { text: text.clone() }
                    }
                })
                .collect(),
        }
    }

    fn resolve_many(
        &self,
        selectors: &mut [DiscoverySelector],
        path: &str,
        diagnostics: &mut Vec<SelectorResolutionDiagnostic>,
        rewrites: &mut Vec<SelectorRewrite>,
    ) {
        for (index, selector) in selectors.iter_mut().enumerate() {
            self.resolve_one(selector, &format!("{path}[{index}]"), diagnostics, rewrites);
        }
    }

    fn resolve_quote_one(
        &self,
        selector: &mut DiscoveryQuoteSelector,
        path: &str,
        diagnostics: &mut Vec<SelectorResolutionDiagnostic>,
        rewrites: &mut Vec<SelectorRewrite>,
    ) {
        let mut general = DiscoverySelector::from(selector.clone());
        self.resolve_one(&mut general, path, diagnostics, rewrites);
        *selector = DiscoveryQuoteSelector::try_from(general)
            .expect("a quote-only selector remains quote_range during resolution");
    }

    fn resolve_one(
        &self,
        selector: &mut DiscoverySelector,
        path: &str,
        diagnostics: &mut Vec<SelectorResolutionDiagnostic>,
        rewrites: &mut Vec<SelectorRewrite>,
    ) {
        let resolved = match selector {
            DiscoverySelector::BlockRange { first, last } => {
                let first = self.resolve_block_ref(first, &format!("{path}.first"));
                let last = self.resolve_block_ref(last, &format!("{path}.last"));
                match (first, last) {
                    (Ok(first), Ok(last)) => DiscoverySelector::BlockRange {
                        first: first.to_owned(),
                        last: last.to_owned(),
                    },
                    (first, last) => {
                        if let Err(error) = first {
                            diagnostics.push(error);
                        }
                        if let Err(error) = last {
                            diagnostics.push(error);
                        }
                        return;
                    }
                }
            }
            DiscoverySelector::QuoteRange {
                first,
                start,
                last,
                end,
            } => {
                let first_binding = self.resolve_binding(first, &format!("{path}.first"));
                let last_binding = self.resolve_binding(last, &format!("{path}.last"));
                let (first_binding, last_binding) = match (first_binding, last_binding) {
                    (Ok(first), Ok(last)) => (first, last),
                    (first, last) => {
                        if let Err(error) = first {
                            diagnostics.push(error);
                        }
                        if let Err(error) = last {
                            diagnostics.push(error);
                        }
                        return;
                    }
                };

                let same_anchor = first_binding.block_id == last_binding.block_id && start == end;
                if same_anchor {
                    match resolve_anchor(first_binding, start, path) {
                        Ok(resolved_anchor) => DiscoverySelector::QuoteRange {
                            first: first_binding.block_id.clone(),
                            start: resolved_anchor.clone(),
                            last: last_binding.block_id.clone(),
                            end: resolved_anchor,
                        },
                        Err(error) => {
                            diagnostics.push(error);
                            return;
                        }
                    }
                } else {
                    let start = resolve_anchor(first_binding, start, &format!("{path}.start"));
                    let end = resolve_anchor(last_binding, end, &format!("{path}.end"));
                    match (start, end) {
                        (Ok(start), Ok(end)) => DiscoverySelector::QuoteRange {
                            first: first_binding.block_id.clone(),
                            start,
                            last: last_binding.block_id.clone(),
                            end,
                        },
                        (start, end) => {
                            if let Err(error) = start {
                                diagnostics.push(error);
                            }
                            if let Err(error) = end {
                                diagnostics.push(error);
                            }
                            return;
                        }
                    }
                }
            }
        };

        record_selector_rewrites(path, selector, &resolved, rewrites);
        *selector = resolved;
    }

    fn resolve_block_ref<'a>(
        &'a self,
        value: &str,
        path: &str,
    ) -> Result<&'a str, SelectorResolutionDiagnostic> {
        self.resolve_binding(value, path)
            .map(|binding| binding.block_id.as_str())
    }

    fn resolve_binding(
        &self,
        value: &str,
        path: &str,
    ) -> Result<&SelectorBinding, SelectorResolutionDiagnostic> {
        let index = self
            .by_handle
            .get(value)
            .or_else(|| self.by_block_id.get(value))
            .copied();
        index
            .map(|index| &self.bindings[index])
            .ok_or_else(|| SelectorResolutionDiagnostic {
                code: SelectorResolutionCode::UnknownBlockRef,
                path: path.into(),
                message: format!("块引用 {value} 不在本次 DiscoveryInput 的绑定表中"),
            })
    }

    fn binding_for_id(&self, block_id: &str) -> Option<&SelectorBinding> {
        self.by_block_id
            .get(block_id)
            .map(|index| &self.bindings[*index])
    }
}

fn record_selector_rewrites(
    path: &str,
    original: &DiscoverySelector,
    resolved: &DiscoverySelector,
    rewrites: &mut Vec<SelectorRewrite>,
) {
    match (original, resolved) {
        (
            DiscoverySelector::BlockRange {
                first: old_first,
                last: old_last,
            },
            DiscoverySelector::BlockRange {
                first: new_first,
                last: new_last,
            },
        ) => {
            record_rewrite(
                SelectorRewriteKind::BlockHandle,
                &format!("{path}.first"),
                old_first,
                new_first,
                rewrites,
            );
            record_rewrite(
                SelectorRewriteKind::BlockHandle,
                &format!("{path}.last"),
                old_last,
                new_last,
                rewrites,
            );
        }
        (
            DiscoverySelector::QuoteRange {
                first: old_first,
                start: old_start,
                last: old_last,
                end: old_end,
            },
            DiscoverySelector::QuoteRange {
                first: new_first,
                start: new_start,
                last: new_last,
                end: new_end,
            },
        ) => {
            record_rewrite(
                SelectorRewriteKind::BlockHandle,
                &format!("{path}.first"),
                old_first,
                new_first,
                rewrites,
            );
            record_rewrite(
                SelectorRewriteKind::BlockHandle,
                &format!("{path}.last"),
                old_last,
                new_last,
                rewrites,
            );
            record_rewrite(
                SelectorRewriteKind::SafeQuoteNormalization,
                &format!("{path}.start.quote"),
                &old_start.quote,
                &new_start.quote,
                rewrites,
            );
            record_optional_u32_rewrite(
                &format!("{path}.start.occurrence"),
                old_start.occurrence,
                new_start.occurrence,
                rewrites,
            );
            record_optional_u32_rewrite(
                &format!("{path}.end.occurrence"),
                old_end.occurrence,
                new_end.occurrence,
                rewrites,
            );
            record_rewrite(
                SelectorRewriteKind::SafeQuoteNormalization,
                &format!("{path}.end.quote"),
                &old_end.quote,
                &new_end.quote,
                rewrites,
            );
        }
        _ => unreachable!("selector kind is preserved during resolution"),
    }
}

fn record_optional_u32_rewrite(
    path: &str,
    original: Option<u32>,
    resolved: Option<u32>,
    rewrites: &mut Vec<SelectorRewrite>,
) {
    if original != resolved {
        rewrites.push(SelectorRewrite {
            kind: SelectorRewriteKind::OccurrenceBinding,
            path: path.into(),
            original: original.map_or_else(|| "null".into(), |value| value.to_string()),
            resolved: resolved.map_or_else(|| "null".into(), |value| value.to_string()),
        });
    }
}

fn record_rewrite(
    kind: SelectorRewriteKind,
    path: &str,
    original: &str,
    resolved: &str,
    rewrites: &mut Vec<SelectorRewrite>,
) {
    if original != resolved {
        rewrites.push(SelectorRewrite {
            kind,
            path: path.into(),
            original: original.into(),
            resolved: resolved.into(),
        });
    }
}

pub(crate) const MAX_DISAMBIGUATOR_CHARACTERS: usize = 256;

fn resolve_anchor(
    binding: &SelectorBinding,
    anchor: &DiscoveryQuoteAnchor,
    path: &str,
) -> Result<DiscoveryQuoteAnchor, SelectorResolutionDiagnostic> {
    if anchor.quote.is_empty() {
        return Err(SelectorResolutionDiagnostic {
            code: SelectorResolutionCode::QuoteNotFound,
            path: format!("{path}.quote"),
            message: "quote 不能为空".into(),
        });
    }
    if anchor.occurrence == Some(0) {
        return Err(SelectorResolutionDiagnostic {
            code: SelectorResolutionCode::InvalidDisambiguator,
            path: format!("{path}.occurrence"),
            message: "occurrence 从 1 开始，不能为 0".into(),
        });
    }
    for (name, value) in [("prefix", &anchor.prefix), ("suffix", &anchor.suffix)] {
        if let Some(value) = value
            && (value.trim().is_empty() || value.chars().count() > MAX_DISAMBIGUATOR_CHARACTERS)
        {
            return Err(SelectorResolutionDiagnostic {
                code: SelectorResolutionCode::InvalidDisambiguator,
                path: format!("{path}.{name}"),
                message: format!("{name} 必须为 1..={MAX_DISAMBIGUATOR_CHARACTERS} 个字符"),
            });
        }
    }

    let (matches, normalized) = locator_matches(&binding.text, &anchor.quote);
    if matches.is_empty() {
        return Err(SelectorResolutionDiagnostic {
            code: SelectorResolutionCode::QuoteNotFound,
            path: format!("{path}.quote"),
            message: format!("quote 未在 {} 的可见文本中找到", binding.handle),
        });
    }

    let selected = if let Some(occurrence) = anchor.occurrence {
        let index = usize::try_from(occurrence - 1).unwrap_or(usize::MAX);
        let Some(candidate) = matches.get(index).copied() else {
            return Err(SelectorResolutionDiagnostic {
                code: SelectorResolutionCode::InvalidDisambiguator,
                path: format!("{path}.occurrence"),
                message: format!(
                    "occurrence={occurrence} 超出 {} 中的 {} 个候选",
                    binding.handle,
                    matches.len()
                ),
            });
        };
        if !context_matches(&binding.text, candidate, anchor) {
            return Err(SelectorResolutionDiagnostic {
                code: SelectorResolutionCode::InvalidDisambiguator,
                path: path.into(),
                message: "occurrence 指定的候选不满足 prefix/suffix".into(),
            });
        }
        candidate
    } else {
        let filtered = matches
            .iter()
            .copied()
            .filter(|candidate| context_matches(&binding.text, *candidate, anchor))
            .collect::<Vec<_>>();
        match filtered.as_slice() {
            [candidate] => *candidate,
            [] => {
                return Err(SelectorResolutionDiagnostic {
                    code: SelectorResolutionCode::QuoteNotFound,
                    path: path.into(),
                    message: format!("quote 的候选均不满足 {} 中的 prefix/suffix", binding.handle),
                });
            }
            [_, ..] => {
                return Err(SelectorResolutionDiagnostic {
                    code: SelectorResolutionCode::AmbiguousQuote,
                    path: path.into(),
                    message: format!(
                        "quote{}在 {} 中仍有 {} 个候选，需要 occurrence 或更强上下文",
                        if normalized {
                            "经安全规范化后"
                        } else {
                            ""
                        },
                        binding.handle,
                        filtered.len()
                    ),
                });
            }
        }
    };

    let resolved_quote = binding.text[selected.0..selected.1].to_owned();
    let resolved_occurrence = overlapping_matches(&binding.text, &resolved_quote)
        .iter()
        .position(|start| *start == selected.0)
        .and_then(|index| u32::try_from(index + 1).ok())
        .expect("the selected exact source substring is one of its own matches");
    Ok(DiscoveryQuoteAnchor {
        quote: resolved_quote,
        occurrence: Some(resolved_occurrence),
        prefix: anchor.prefix.clone(),
        suffix: anchor.suffix.clone(),
    })
}

pub(crate) fn context_matches(
    text: &str,
    candidate: (usize, usize),
    anchor: &DiscoveryQuoteAnchor,
) -> bool {
    let prefix_matches = anchor.prefix.as_ref().is_none_or(|prefix| {
        let source = normalized_string(&text[..candidate.0]);
        let wanted = normalized_string(prefix);
        source.trim_end().ends_with(wanted.trim())
    });
    let suffix_matches = anchor.suffix.as_ref().is_none_or(|suffix| {
        let source = normalized_string(&text[candidate.1..]);
        let wanted = normalized_string(suffix);
        source.trim_start().starts_with(wanted.trim())
    });
    prefix_matches && suffix_matches
}

fn normalized_string(text: &str) -> String {
    normalize_locator_text(text, false)
        .chars
        .into_iter()
        .collect()
}

/// Shared matching policy for v3 quotes and staged single occurrences.
pub fn locator_matches(text: &str, quote: &str) -> (Vec<(usize, usize)>, bool) {
    let exact = overlapping_matches(text, quote)
        .into_iter()
        .map(|start| (start, start + quote.len()))
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return (exact, false);
    }
    let source = normalize_locator_text(text, true);
    let wanted = normalize_locator_text(strip_outer_math_delimiters(quote), false);
    (normalized_matches(&source, &wanted.chars), true)
}

fn overlapping_matches(text: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut cursor = 0;
    while cursor <= text.len().saturating_sub(needle.len()) {
        let Some(relative) = text[cursor..].find(needle) else {
            break;
        };
        let start = cursor + relative;
        starts.push(start);
        cursor = start
            + text[start..]
                .chars()
                .next()
                .expect("a nonempty match starts on a character")
                .len_utf8();
    }
    starts
}

struct NormalizedLocatorText {
    chars: Vec<char>,
    source_spans: Vec<(usize, usize)>,
}

fn normalize_locator_text(text: &str, track_source: bool) -> NormalizedLocatorText {
    let mut chars = Vec::new();
    let mut source_spans = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let tail = &text[index..];
        if let Some(len) = tex_spacing_prefix(tail) {
            push_normalized_space(&mut chars, &mut source_spans, index, index + len);
            index += len;
            continue;
        }
        let ch = tail
            .chars()
            .next()
            .expect("index remains on a character boundary");
        let end = index + ch.len_utf8();
        if ch.is_whitespace() {
            let mut whitespace_end = end;
            while whitespace_end < text.len() {
                let next = text[whitespace_end..]
                    .chars()
                    .next()
                    .expect("whitespace scan remains in bounds");
                if !next.is_whitespace() {
                    break;
                }
                whitespace_end += next.len_utf8();
            }
            push_normalized_space(&mut chars, &mut source_spans, index, whitespace_end);
            index = whitespace_end;
            continue;
        }
        chars.push(ch);
        source_spans.push((index, end));
        index = end;
    }
    if !track_source {
        source_spans.clear();
    }
    NormalizedLocatorText {
        chars,
        source_spans,
    }
}

fn strip_outer_math_delimiters(text: &str) -> &str {
    let text = text.trim();
    if text.len() >= 4 && text.starts_with("$$") && text.ends_with("$$") {
        &text[2..text.len() - 2]
    } else if text.len() >= 2 && text.starts_with('$') && text.ends_with('$') {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn tex_spacing_prefix(text: &str) -> Option<usize> {
    ["\\qquad", "\\quad", "\\,", "\\:", "\\;", "\\!", "\\ "]
        .into_iter()
        .find_map(|command| text.starts_with(command).then_some(command.len()))
}

fn push_normalized_space(
    chars: &mut Vec<char>,
    source_spans: &mut Vec<(usize, usize)>,
    start: usize,
    end: usize,
) {
    if chars.last() == Some(&' ') {
        if let Some(span) = source_spans.last_mut() {
            span.1 = end;
        }
    } else {
        chars.push(' ');
        source_spans.push((start, end));
    }
}

fn normalized_matches(source: &NormalizedLocatorText, wanted: &[char]) -> Vec<(usize, usize)> {
    if wanted.is_empty() || wanted.len() > source.chars.len() {
        return Vec::new();
    }
    source
        .chars
        .windows(wanted.len())
        .enumerate()
        .filter(|(_, window)| *window == wanted)
        .map(|(index, _)| {
            (
                source.source_spans[index].0,
                source.source_spans[index + wanted.len() - 1].1,
            )
        })
        .collect()
}
