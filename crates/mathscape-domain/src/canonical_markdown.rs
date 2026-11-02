//! Logical canonical markdown for one PDF.
//!
//! A canonical id is not an Artifact. Physical Markdown snapshots stay
//! content-addressed and immutable. This module only names the logical view
//! and projects coverage from page ranges that already exist.

use serde::{Deserialize, Serialize};

use crate::{DomainError, PageIndexBase, PageRange, StableId};

/// Identity prefix. The PDF stable id is the final path segment and may itself
/// contain slashes; profile and policy never do.
pub const CANONICAL_MARKDOWN_ID_PREFIX: &str = "mathscape:canonical-markdown/v1";

/// Profile used when a Marker run did not record `marker_profile`.
pub const UNSPECIFIED_PARSER_PROFILE: &str = "mathscape.marker.unspecified";

/// Policy used when a conversion has no tool version.
pub const UNVERSIONED_PARSER_POLICY: &str = "unversioned";

/// Public coverage ranges are closed and 1-based, matching `material.read`'s
/// `pdf_pages`. Marker's `--page_range` and markdown-pdf map page ids are
/// 0-based and must be normalized before they enter the ledger.
pub const CANONICAL_PAGE_INDEX_BASE: PageIndexBase = PageIndexBase::One;

/// `(source PDF, parser profile, parser policy)` decides one canonical lineage.
///
/// A Marker upgrade that changes behavior must use a new profile or policy.
/// It must not overwrite an existing lineage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalMarkdownKey {
    source_pdf_artifact_id: StableId,
    parser_profile: String,
    parser_policy_version: String,
}

impl CanonicalMarkdownKey {
    pub fn new(
        source_pdf_artifact_id: StableId,
        parser_profile: impl Into<String>,
        parser_policy_version: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let parser_profile = parser_profile.into();
        let parser_policy_version = parser_policy_version.into();
        validate_component(&parser_profile, "canonical.parser_profile")?;
        validate_component(&parser_policy_version, "canonical.parser_policy_version")?;
        Ok(Self {
            source_pdf_artifact_id,
            parser_profile,
            parser_policy_version,
        })
    }

    pub fn source_pdf_artifact_id(&self) -> &StableId {
        &self.source_pdf_artifact_id
    }

    pub fn parser_profile(&self) -> &str {
        &self.parser_profile
    }

    pub fn parser_policy_version(&self) -> &str {
        &self.parser_policy_version
    }

    /// Stable logical id. The same key always produces the same id.
    pub fn canonical_id(&self) -> StableId {
        StableId::parse(format!(
            "{CANONICAL_MARKDOWN_ID_PREFIX}/{}/{}/{}",
            self.parser_profile, self.parser_policy_version, self.source_pdf_artifact_id
        ))
        .expect("profile and policy were validated and the PDF id is a stable id")
    }
}

/// `marker-{tool_version}`, or [`UNVERSIONED_PARSER_POLICY`] when absent.
pub fn parser_policy_from_tool_version(tool_version: Option<&str>) -> String {
    match tool_version
        .map(str::trim)
        .filter(|version| !version.is_empty())
    {
        Some(version) => format!("marker-{version}"),
        None => UNVERSIONED_PARSER_POLICY.to_owned(),
    }
}

/// Closed page interval. `from` and `to` are inclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InclusivePageRange {
    pub from: u32,
    pub to: u32,
}

impl InclusivePageRange {
    pub fn new(from: u32, to: u32) -> Result<Self, DomainError> {
        if from == 0 || from > to {
            return Err(DomainError::InvalidRange {
                field: "canonical.page_range",
                start: u64::from(from),
                end: u64::from(to),
            });
        }
        Ok(Self { from, to })
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.from <= other.to && other.from <= self.to
    }

    pub fn page_count(self) -> u32 {
        self.to.saturating_sub(self.from).saturating_add(1)
    }
}

/// Normalizes a conversion [`PageRange`] into the canonical 1-based interval.
pub fn normalize_to_one_based(range: PageRange) -> Result<InclusivePageRange, DomainError> {
    let (from, to) = match range.index_base() {
        PageIndexBase::One => (range.start(), range.end()),
        PageIndexBase::Zero => (
            checked_page(range.start(), "canonical.page_range")?,
            checked_page(range.end(), "canonical.page_range")?,
        ),
    };
    InclusivePageRange::new(from, to)
}

/// Marker page ids are 0-based PDF indexes (`{page}----------------` and
/// `MarkdownPdfBlockMap.marker_page_ids`). Each id becomes one 1-based page,
/// then adjacent pages merge.
pub fn pages_from_marker_page_ids(
    marker_page_ids: &[u32],
) -> Result<Vec<InclusivePageRange>, DomainError> {
    let mut ranges = Vec::with_capacity(marker_page_ids.len());
    for id in marker_page_ids {
        let page = checked_page(*id, "canonical.marker_page_id")?;
        ranges.push(InclusivePageRange::new(page, page)?);
    }
    Ok(merge_page_ranges(ranges))
}

pub fn merge_page_ranges(
    ranges: impl IntoIterator<Item = InclusivePageRange>,
) -> Vec<InclusivePageRange> {
    let mut ranges: Vec<_> = ranges.into_iter().collect();
    ranges.sort_by_key(|range| (range.from, range.to));
    let mut merged: Vec<InclusivePageRange> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.from <= last.to.saturating_add(1)
        {
            last.to = last.to.max(range.to);
            continue;
        }
        merged.push(range);
    }
    merged
}

pub fn missing_pages(total_pages: u32, covered: &[InclusivePageRange]) -> Vec<InclusivePageRange> {
    if total_pages == 0 {
        return Vec::new();
    }
    let covered = merge_page_ranges(covered.iter().copied());
    let mut missing = Vec::new();
    let mut cursor = 1_u32;
    for range in covered {
        if range.from > total_pages {
            break;
        }
        if range.from > cursor {
            missing.push(InclusivePageRange {
                from: cursor,
                to: range.from - 1,
            });
        }
        let next = range.to.saturating_add(1);
        if next > cursor {
            cursor = next;
        }
        if cursor > total_pages {
            return missing;
        }
    }
    if cursor <= total_pages {
        missing.push(InclusivePageRange {
            from: cursor,
            to: total_pages,
        });
    }
    missing
}

pub fn covers_range(covered: &[InclusivePageRange], request: InclusivePageRange) -> bool {
    merge_page_ranges(covered.iter().copied())
        .into_iter()
        .any(|range| range.from <= request.from && request.to <= range.to)
}

/// Coverage of one logical canonical markdown view.
///
/// `covered` is the union of immutable snapshots. `head_covered` is only the
/// current head snapshot. `complete` is true only when that head itself spans
/// every page, because this layer does not concatenate snapshot bytes.
/// `partial` is true when the request is not fully covered or the book is not
/// `complete`, so a successful read of a fragment cannot be mistaken for the
/// full text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownCoverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<u32>,
    pub covered: Vec<InclusivePageRange>,
    pub missing: Vec<InclusivePageRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<InclusivePageRange>,
    pub head_covered: Vec<InclusivePageRange>,
    pub partial: bool,
    pub complete: bool,
    pub page_index_base: PageIndexBase,
}

pub fn markdown_coverage(
    total_pages: Option<u32>,
    covered: &[InclusivePageRange],
    head_covered: &[InclusivePageRange],
    requested: Option<InclusivePageRange>,
) -> MarkdownCoverage {
    let covered = merge_page_ranges(covered.iter().copied());
    let head_covered = merge_page_ranges(head_covered.iter().copied());
    let missing = total_pages.map_or_else(Vec::new, |total| missing_pages(total, &covered));
    let full = |ranges: &[InclusivePageRange]| match total_pages {
        Some(0) => true,
        Some(total) => InclusivePageRange::new(1, total)
            .ok()
            .is_some_and(|span| covers_range(ranges, span)),
        None => false,
    };
    let complete = full(&covered) && full(&head_covered);
    let request_gap = requested.is_some_and(|request| !covers_range(&covered, request));
    MarkdownCoverage {
        total_pages,
        covered,
        missing,
        requested,
        head_covered,
        partial: request_gap || !complete,
        complete,
        page_index_base: CANONICAL_PAGE_INDEX_BASE,
    }
}

/// One immutable Markdown snapshot that contributes pages to a canonical view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalSegment {
    pub conversion_id: StableId,
    pub markdown_artifact_id: StableId,
    /// Absent when the conversion recorded no page range. Those bytes stay
    /// addressable but do not count as covered pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<InclusivePageRange>,
    /// Present on a live projection; the ledger does not need it once a head
    /// has been chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

/// Registry/view row. Not an [`crate::EntityKind`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalMarkdownView {
    pub canonical_id: StableId,
    pub source_pdf_artifact_id: StableId,
    pub parser_profile: String,
    pub parser_policy_version: String,
    pub head_artifact_id: Option<StableId>,
    pub head_revision: Option<String>,
    pub total_pages: Option<u32>,
    pub segments: Vec<CanonicalSegment>,
}

impl CanonicalMarkdownView {
    pub fn from_key(key: &CanonicalMarkdownKey, total_pages: Option<u32>) -> Self {
        Self {
            canonical_id: key.canonical_id(),
            source_pdf_artifact_id: key.source_pdf_artifact_id().clone(),
            parser_profile: key.parser_profile().to_owned(),
            parser_policy_version: key.parser_policy_version().to_owned(),
            head_artifact_id: None,
            head_revision: None,
            total_pages,
            segments: Vec::new(),
        }
    }

    pub fn coverage(&self, requested: Option<InclusivePageRange>) -> MarkdownCoverage {
        let covered: Vec<_> = self
            .segments
            .iter()
            .filter_map(|segment| segment.pages)
            .collect();
        let head = self.head_artifact_id.as_ref();
        let head_covered: Vec<_> = self
            .segments
            .iter()
            .filter(|segment| head.is_some_and(|head| head == &segment.markdown_artifact_id))
            .filter_map(|segment| segment.pages)
            .collect();
        markdown_coverage(self.total_pages, &covered, &head_covered, requested)
    }

    /// Physical markdown artifacts replaced by the single map node: the head,
    /// plus every snapshot that actually recorded a page range.
    pub fn collapsed_markdown_ids(&self) -> Vec<StableId> {
        let mut ids = Vec::new();
        if let Some(head) = &self.head_artifact_id {
            ids.push(head.clone());
        }
        for segment in &self.segments {
            if segment.pages.is_some() {
                ids.push(segment.markdown_artifact_id.clone());
            }
        }
        ids.sort();
        ids.dedup();
        ids
    }
}

fn checked_page(zero_based: u32, field: &'static str) -> Result<u32, DomainError> {
    zero_based.checked_add(1).ok_or(DomainError::InvalidRange {
        field,
        start: u64::from(zero_based),
        end: u64::from(zero_based),
    })
}

fn validate_component(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    if value.chars().any(char::is_control) {
        return Err(DomainError::ControlCharacter { field });
    }
    if value.chars().any(char::is_whitespace) || value.contains('/') {
        return Err(DomainError::InvalidStableId {
            value: value.to_owned(),
            reason: "parser profile and policy cannot contain whitespace or '/'",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf() -> StableId {
        StableId::parse("mathscape:artifact/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()
    }

    fn key(policy: &str) -> CanonicalMarkdownKey {
        CanonicalMarkdownKey::new(pdf(), "mathscape.marker.math-ocr.v1", policy).unwrap()
    }

    #[test]
    fn canonical_id_is_unique_per_pdf_profile_and_policy() {
        let first = key("marker-2.0.0");
        let again = key("marker-2.0.0");
        assert_eq!(first.canonical_id(), again.canonical_id());
        assert_ne!(first.canonical_id(), key("marker-2.1.0").canonical_id());
        let other_profile =
            CanonicalMarkdownKey::new(pdf(), "mathscape.marker.fast-native.v1", "marker-2.0.0")
                .unwrap();
        assert_ne!(first.canonical_id(), other_profile.canonical_id());
        assert!(
            first
                .canonical_id()
                .as_str()
                .starts_with(CANONICAL_MARKDOWN_ID_PREFIX)
        );
        assert!(CanonicalMarkdownKey::new(pdf(), "bad/profile", "marker-2.0.0").is_err());
    }

    #[test]
    fn zero_based_ranges_and_marker_page_ids_become_one_based_coverage() {
        let range =
            normalize_to_one_based(PageRange::new(11, 39, PageIndexBase::Zero).unwrap()).unwrap();
        assert_eq!(range, InclusivePageRange { from: 12, to: 40 });
        let from_ids = pages_from_marker_page_ids(&[11, 13, 12, 40]).unwrap();
        assert_eq!(
            from_ids,
            vec![
                InclusivePageRange { from: 12, to: 14 },
                InclusivePageRange { from: 41, to: 41 },
            ]
        );
        let merged = merge_page_ranges([
            InclusivePageRange { from: 12, to: 40 },
            InclusivePageRange { from: 41, to: 50 },
            InclusivePageRange { from: 10, to: 11 },
        ]);
        assert_eq!(merged, vec![InclusivePageRange { from: 10, to: 50 }]);
    }

    #[test]
    fn partial_means_the_request_or_the_book_is_not_fully_covered() {
        let covered = [InclusivePageRange { from: 12, to: 40 }];
        let fragment = markdown_coverage(
            Some(300),
            &covered,
            &covered,
            Some(InclusivePageRange { from: 12, to: 40 }),
        );
        assert!(fragment.partial);
        assert!(!fragment.complete);
        assert_eq!(fragment.missing.first().unwrap().from, 1);
        assert_eq!(fragment.missing.last().unwrap().to, 300);
        assert_eq!(fragment.page_index_base, PageIndexBase::One);

        let head = [InclusivePageRange { from: 1, to: 2 }];
        let union = [
            InclusivePageRange { from: 1, to: 2 },
            InclusivePageRange { from: 3, to: 4 },
        ];
        let unmerged = markdown_coverage(Some(4), &union, &head, None);
        assert!(
            unmerged.partial,
            "a full union is not complete until the head spans it"
        );
        assert!(!unmerged.complete);
        assert!(unmerged.missing.is_empty());

        let whole = markdown_coverage(
            Some(4),
            &union,
            &union,
            Some(InclusivePageRange { from: 2, to: 3 }),
        );
        assert!(whole.complete);
        assert!(!whole.partial);

        let gap = markdown_coverage(
            Some(4),
            &covered,
            &covered,
            Some(InclusivePageRange { from: 1, to: 2 }),
        );
        assert!(gap.partial);
    }

    #[test]
    fn policy_falls_back_when_the_tool_version_is_missing() {
        assert_eq!(
            parser_policy_from_tool_version(Some("2.0.0")),
            "marker-2.0.0"
        );
        assert_eq!(
            parser_policy_from_tool_version(None),
            UNVERSIONED_PARSER_POLICY
        );
        assert_eq!(
            parser_policy_from_tool_version(Some("  ")),
            UNVERSIONED_PARSER_POLICY
        );
    }
}
