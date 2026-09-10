//! Single-occurrence lookup with request- and proposal-bound candidate selection.
use crate::discovery_validation::{AuthorizedRegion, authorize_discovery_input};
use crate::selector_resolver::locator_matches;
use crate::{ResolvedDiscoverySelector, SelectorResolver};
use mathscape_domain::{
    ContentHash, DiscoveryInput, DiscoveryQuoteAnchor, DiscoverySelector, TextByteRange,
    TextOccurrence,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceErrorCode {
    InvalidInput,
    UnknownBlock,
    EmptyText,
    QuoteNotFound,
    AmbiguousQuote,
    InvalidOccurrence,
    UnauthorizedRange,
    InvalidRange,
    StaleCandidate,
}

#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct OccurrenceError {
    pub code: OccurrenceErrorCode,
    pub message: String,
}

fn error(code: OccurrenceErrorCode, message: impl Into<String>) -> OccurrenceError {
    OccurrenceError {
        code,
        message: message.into(),
    }
}

#[derive(Clone, Debug)]
pub struct LocatedOccurrence {
    original: TextOccurrence,
    selector: ResolvedDiscoverySelector,
    normalized: bool,
    precision_review: bool,
}
impl LocatedOccurrence {
    pub fn original(&self) -> &TextOccurrence {
        &self.original
    }
    pub fn selector(&self) -> &ResolvedDiscoverySelector {
        &self.selector
    }
    pub fn normalized(&self) -> bool {
        self.normalized
    }
    pub fn precision_review(&self) -> bool {
        self.precision_review
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct OccurrenceCandidate {
    pub candidate_ref: String,
    pub text: String,
    pub prefix: String,
    pub suffix: String,
}

/// Private bindings are not deserializable from a model response.
#[derive(Clone, Debug)]
pub struct OccurrenceCandidates {
    scope_fingerprint: ContentHash,
    base_fingerprint: ContentHash,
    path: String,
    candidates: Vec<OccurrenceCandidate>,
    locations: Vec<LocatedOccurrence>,
    has_more: bool,
}
impl OccurrenceCandidates {
    pub fn candidates(&self) -> &[OccurrenceCandidate] {
        &self.candidates
    }
    pub fn has_more(&self) -> bool {
        self.has_more
    }
}

pub struct OccurrenceResolver {
    region: AuthorizedRegion,
    resolver: SelectorResolver,
    allowed: Vec<TextByteRange>,
    scope_fingerprint: ContentHash,
}

impl OccurrenceResolver {
    pub fn new(
        input: &DiscoveryInput,
        mut allowed: Vec<TextByteRange>,
    ) -> Result<Self, OccurrenceError> {
        let mut diagnostics = Vec::new();
        let region = authorize_discovery_input(input, &mut diagnostics)
            .ok_or_else(|| error(OccurrenceErrorCode::InvalidInput, "invalid DiscoveryInput"))?;
        allowed.sort_by_key(|r| (r.start, r.end));
        if allowed.is_empty()
            || allowed
                .iter()
                .any(|r| r.start >= r.end || region.text_in(r).is_none())
            || allowed.windows(2).any(|p| p[0].end > p[1].start)
        {
            return Err(error(
                OccurrenceErrorCode::UnauthorizedRange,
                "invalid allowed ranges",
            ));
        }
        let scope_fingerprint = crate::unit_discovery::fingerprint(&(
            &input.fingerprint,
            &allowed,
            crate::unit_discovery::new_request_ref("loc"),
        ));
        Ok(Self {
            resolver: SelectorResolver::from_input(input),
            region,
            allowed,
            scope_fingerprint,
        })
    }

    fn authorized(&self, range: &TextByteRange) -> bool {
        let mut cursor = range.start;
        for r in &self.allowed {
            if r.end <= cursor {
                continue;
            }
            if r.start > cursor {
                return false;
            }
            cursor = r.end;
            if cursor >= range.end {
                return true;
            }
        }
        false
    }

    fn block(&self, handle: &str) -> Result<(&str, &str), OccurrenceError> {
        let binding = self
            .resolver
            .bindings()
            .iter()
            .find(|b| b.handle() == handle)
            .ok_or_else(|| error(OccurrenceErrorCode::UnknownBlock, handle))?;
        Ok((binding.block_id(), binding.text()))
    }

    fn materialize(
        &self,
        original: &TextOccurrence,
        id: &str,
        text: &str,
        span: (usize, usize),
        normalized: bool,
    ) -> Result<LocatedOccurrence, OccurrenceError> {
        let quote = &text[span.0..span.1];
        let (matches, _) = locator_matches(text, quote);
        let occurrence = matches
            .iter()
            .position(|s| s.0 == span.0)
            .and_then(|i| u32::try_from(i + 1).ok())
            .ok_or_else(|| {
                error(
                    OccurrenceErrorCode::InvalidOccurrence,
                    "occurrence overflow",
                )
            })?;
        let anchor = DiscoveryQuoteAnchor {
            quote: quote.into(),
            occurrence: Some(occurrence),
            prefix: None,
            suffix: None,
        };
        let selector = self
            .region
            .resolve(&DiscoverySelector::QuoteRange {
                first: id.into(),
                start: anchor.clone(),
                last: id.into(),
                end: anchor,
            })
            .map_err(|m| error(OccurrenceErrorCode::InvalidRange, m))?;
        if !self.authorized(selector.range()) {
            return Err(error(
                OccurrenceErrorCode::UnauthorizedRange,
                "outside task evidence",
            ));
        }
        Ok(LocatedOccurrence {
            original: original.clone(),
            selector,
            normalized,
            precision_review: false,
        })
    }

    pub fn resolve(&self, anchor: &TextOccurrence) -> Result<LocatedOccurrence, OccurrenceError> {
        match anchor {
            TextOccurrence::TextOccurrence {
                block,
                text,
                occurrence,
            } => {
                if text.trim().is_empty() {
                    return Err(error(OccurrenceErrorCode::EmptyText, "text is blank"));
                }
                let (id, source) = self.block(block)?;
                let (matches, normalized) = locator_matches(source, text);
                let selected = match occurrence {
                    Some(0) => {
                        return Err(error(
                            OccurrenceErrorCode::InvalidOccurrence,
                            "occurrence starts at 1",
                        ));
                    }
                    Some(n) => matches.get((*n - 1) as usize).copied().ok_or_else(|| {
                        error(
                            OccurrenceErrorCode::InvalidOccurrence,
                            "occurrence out of range",
                        )
                    })?,
                    None => match matches.as_slice() {
                        [one] => *one,
                        [] => {
                            return Err(error(
                                OccurrenceErrorCode::QuoteNotFound,
                                "text not found",
                            ));
                        }
                        _ => {
                            return Err(error(
                                OccurrenceErrorCode::AmbiguousQuote,
                                format!("{} matches", matches.len()),
                            ));
                        }
                    },
                };
                self.materialize(anchor, id, source, selected, normalized)
            }
            TextOccurrence::RangeOccurrence { selector } => {
                let general = DiscoverySelector::from(selector.clone());
                if let DiscoverySelector::QuoteRange { first, last, .. } = &general {
                    self.block(first)?;
                    self.block(last)?;
                    if first == last {
                        return Err(error(
                            OccurrenceErrorCode::InvalidRange,
                            "use text_occurrence within one block",
                        ));
                    }
                }
                let (canonical, rewrites) = self
                    .resolver
                    .resolve_selector(general, "anchor")
                    .map_err(|e| error(OccurrenceErrorCode::InvalidRange, e.to_string()))?;
                let selector = self
                    .region
                    .resolve(&canonical)
                    .map_err(|m| error(OccurrenceErrorCode::InvalidRange, m))?;
                if !self.authorized(selector.range()) {
                    return Err(error(
                        OccurrenceErrorCode::UnauthorizedRange,
                        "range crosses task boundary",
                    ));
                }
                let normalized = rewrites
                    .iter()
                    .any(|r| r.kind() == crate::SelectorRewriteKind::SafeQuoteNormalization);
                Ok(LocatedOccurrence {
                    original: anchor.clone(),
                    selector,
                    normalized,
                    precision_review: true,
                })
            }
        }
    }

    /// Candidate IDs are scoped to this opaque set, proposal revision and field.
    pub fn candidates(
        &self,
        anchor: &TextOccurrence,
        base_fingerprint: &ContentHash,
        path: &str,
    ) -> Result<OccurrenceCandidates, OccurrenceError> {
        let TextOccurrence::TextOccurrence { block, text, .. } = anchor else {
            return Err(error(
                OccurrenceErrorCode::InvalidRange,
                "candidate selection requires a single text occurrence",
            ));
        };
        if text.trim().is_empty() {
            return Err(error(OccurrenceErrorCode::EmptyText, "text is blank"));
        }
        let (id, source) = self.block(block)?;
        let (matches, normalized) = locator_matches(source, text);
        let mut candidates = Vec::new();
        let mut locations = Vec::new();
        let mut has_more = false;
        for span in matches {
            let location = match self.materialize(anchor, id, source, span, normalized) {
                Ok(location) => location,
                Err(e) if e.code == OccurrenceErrorCode::UnauthorizedRange => continue,
                Err(e) => return Err(e),
            };
            if candidates.len() == 16 {
                has_more = true;
                break;
            }
            candidates.push(OccurrenceCandidate {
                candidate_ref: format!("c{}", candidates.len() + 1),
                text: source[span.0..span.1].into(),
                prefix: source[..span.0]
                    .chars()
                    .rev()
                    .take(48)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
                suffix: source[span.1..].chars().take(48).collect(),
            });
            locations.push(location);
        }
        if candidates.is_empty() {
            return Err(error(
                OccurrenceErrorCode::QuoteNotFound,
                "no authorized candidates",
            ));
        }
        Ok(OccurrenceCandidates {
            scope_fingerprint: self.scope_fingerprint.clone(),
            base_fingerprint: base_fingerprint.clone(),
            path: path.into(),
            candidates,
            locations,
            has_more,
        })
    }

    pub fn choose_candidate(
        &self,
        set: &OccurrenceCandidates,
        base_fingerprint: &ContentHash,
        path: &str,
        candidate_ref: &str,
    ) -> Result<LocatedOccurrence, OccurrenceError> {
        if self.scope_fingerprint != set.scope_fingerprint
            || base_fingerprint != &set.base_fingerprint
            || path != set.path
        {
            return Err(error(
                OccurrenceErrorCode::StaleCandidate,
                "candidate set belongs to another request, proposal or field",
            ));
        }
        let i = set
            .candidates
            .iter()
            .position(|c| c.candidate_ref == candidate_ref)
            .ok_or_else(|| error(OccurrenceErrorCode::StaleCandidate, "unknown candidate"))?;
        let location = &set.locations[i];
        let verified = self
            .region
            .resolve(location.selector.selector())
            .map_err(|m| error(OccurrenceErrorCode::InvalidRange, m))?;
        if !self.authorized(verified.range()) {
            return Err(error(
                OccurrenceErrorCode::UnauthorizedRange,
                "candidate is outside evidence",
            ));
        }
        Ok(location.clone())
    }
}
