//! Shared physical → canonical alias table for folded canonical markdown.
//!
//! One PDF × its current lineage is one external 「规范解析正文」. Every
//! physical markdown snapshot a registered lineage collapses (head and every
//! paged coverage segment) aliases to that lineage's `canonical_id`. The map
//! fold, Desktop search collapse, locate and cite all read this one table so
//! they agree on which node a physical id stands for.
//!
//! A snapshot claimed by several lineages is a conflict: it is not aliased
//! (it stays its own node / hit) and the claimants carry a diagnostic.
//!
//! When one PDF has several lineages (another profile or Marker policy), the
//! *current* one keeps the plain 「… · 规范解析正文」 title; the others are
//! demoted to 「… · 解析正文（旧版 …）」 so no two siblings share a title.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{CanonicalMarkdownView, DEFAULT_PARSER_PROFILE, StableId};

/// Title suffix of the current lineage's node / search hit.
pub const CANONICAL_MARKDOWN_TITLE: &str = "规范解析正文";

#[derive(Clone, Debug, Default)]
pub struct CanonicalAliasTable {
    aliases: BTreeMap<StableId, StableId>,
    conflicts: BTreeMap<StableId, BTreeSet<StableId>>,
    members: BTreeMap<StableId, Vec<StableId>>,
    current: BTreeSet<StableId>,
}

impl CanonicalAliasTable {
    /// Builds the table from registered views. Views without a head do not
    /// fold anything (there is nothing to open).
    pub fn from_views(views: &[CanonicalMarkdownView]) -> Self {
        let mut claims: BTreeMap<StableId, BTreeSet<StableId>> = BTreeMap::new();
        for view in views.iter().filter(|view| view.head_artifact_id.is_some()) {
            for id in view.collapsed_markdown_ids() {
                claims
                    .entry(id)
                    .or_default()
                    .insert(view.canonical_id.clone());
            }
        }
        let mut table = Self::default();
        for (artifact, canonical_ids) in claims {
            if canonical_ids.len() == 1 {
                let canonical = canonical_ids.into_iter().next().expect("one claim");
                table
                    .members
                    .entry(canonical.clone())
                    .or_default()
                    .push(artifact.clone());
                table.aliases.insert(artifact, canonical);
            } else {
                table.conflicts.insert(artifact, canonical_ids);
            }
        }
        let mut by_pdf: BTreeMap<&StableId, Vec<&CanonicalMarkdownView>> = BTreeMap::new();
        for view in views.iter().filter(|view| view.head_artifact_id.is_some()) {
            by_pdf
                .entry(&view.source_pdf_artifact_id)
                .or_default()
                .push(view);
        }
        for lineages in by_pdf.into_values() {
            if let Some(best) = lineages
                .into_iter()
                .max_by(|left, right| lineage_order(left, right))
            {
                table.current.insert(best.canonical_id.clone());
            }
        }
        table
    }

    /// Canonical id a physical markdown snapshot folds into.
    pub fn canonical_for(&self, artifact: &StableId) -> Option<&StableId> {
        self.aliases.get(artifact)
    }

    pub fn aliases(&self) -> &BTreeMap<StableId, StableId> {
        &self.aliases
    }

    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty() && self.conflicts.is_empty()
    }

    /// Every physical snapshot folded into `canonical` (sorted).
    pub fn members(&self, canonical: &StableId) -> &[StableId] {
        self.members.get(canonical).map_or(&[], Vec::as_slice)
    }

    /// Shared snapshots `canonical` claimed but could not fold.
    pub fn conflict_count(&self, canonical: &StableId) -> usize {
        self.conflicts
            .values()
            .filter(|claimants| claimants.contains(canonical))
            .count()
    }

    pub fn conflicts(&self) -> &BTreeMap<StableId, BTreeSet<StableId>> {
        &self.conflicts
    }

    /// Whether `canonical` is its PDF's current lineage.
    pub fn is_current(&self, canonical: &StableId) -> bool {
        self.current.contains(canonical)
    }

    /// 「{pdf} · 规范解析正文」 for the current lineage, otherwise a demoted
    /// 「{pdf} · 解析正文（旧版 · profile · policy）」.
    pub fn title(&self, view: &CanonicalMarkdownView, pdf_label: &str) -> String {
        if self.is_current(&view.canonical_id) {
            format!("{pdf_label} · {CANONICAL_MARKDOWN_TITLE}")
        } else {
            format!(
                "{pdf_label} · 解析正文（旧版 · {} · {}）",
                short_profile(&view.parser_profile),
                view.parser_policy_version
            )
        }
    }
}

fn short_profile(profile: &str) -> &str {
    profile.strip_prefix("mathscape.marker.").unwrap_or(profile)
}

/// Current-lineage order (greater wins): the default profile, then the newest
/// Marker policy (numeric-aware), then more covered pages, then id.
fn lineage_order(left: &CanonicalMarkdownView, right: &CanonicalMarkdownView) -> Ordering {
    let default = |view: &CanonicalMarkdownView| view.parser_profile == DEFAULT_PARSER_PROFILE;
    let pages = |view: &CanonicalMarkdownView| {
        view.coverage(None)
            .covered
            .iter()
            .map(|range| range.page_count())
            .fold(0_u32, u32::saturating_add)
    };
    default(left)
        .cmp(&default(right))
        .then_with(|| natural_cmp(&left.parser_policy_version, &right.parser_policy_version))
        .then_with(|| pages(left).cmp(&pages(right)))
        .then_with(|| right.canonical_id.cmp(&left.canonical_id))
}

/// Compares digit runs numerically (`marker-1.10.0` > `marker-1.9.2`).
fn natural_cmp(left: &str, right: &str) -> Ordering {
    fn chunks(value: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let bytes = value.as_bytes();
        for index in 1..=bytes.len() {
            if index == bytes.len()
                || bytes[index].is_ascii_digit() != bytes[index - 1].is_ascii_digit()
            {
                out.push((bytes[start].is_ascii_digit(), &value[start..index]));
                start = index;
            }
        }
        out
    }
    if left == right {
        return Ordering::Equal;
    }
    let (a, b) = (chunks(left), chunks(right));
    for ((a_digit, a), (b_digit, b)) in a.iter().zip(b.iter()) {
        let order = if *a_digit && *b_digit {
            let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
            a.len().cmp(&b.len()).then_with(|| a.cmp(b))
        } else {
            a.cmp(b)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathscape_domain::{CanonicalSegment, InclusivePageRange};

    fn id(value: &str) -> StableId {
        StableId::parse(value).unwrap()
    }

    pub(crate) fn view(
        canonical: &str,
        pdf: &str,
        profile: &str,
        policy: &str,
        markdown: &[&str],
    ) -> CanonicalMarkdownView {
        CanonicalMarkdownView {
            canonical_id: id(canonical),
            source_pdf_artifact_id: id(pdf),
            parser_profile: profile.into(),
            parser_policy_version: policy.into(),
            head_artifact_id: markdown.last().map(|value| id(value)),
            head_revision: None,
            total_pages: Some(10),
            segments: markdown
                .iter()
                .enumerate()
                .map(|(index, value)| CanonicalSegment {
                    conversion_id: id(&format!("mathscape:conversion/c{index}-{}", value.len())),
                    markdown_artifact_id: id(value),
                    pages: Some(
                        InclusivePageRange::new(index as u32 + 1, index as u32 + 1).unwrap(),
                    ),
                    finished_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn every_coverage_snapshot_aliases_to_its_canonical() {
        let views = [view(
            "mathscape:canonical-markdown/a",
            "mathscape:artifact/pdf",
            DEFAULT_PARSER_PROFILE,
            "marker-1.0.0",
            &[
                "mathscape:artifact/md1",
                "mathscape:artifact/md2",
                "mathscape:artifact/md3",
            ],
        )];
        let table = CanonicalAliasTable::from_views(&views);
        for md in ["md1", "md2", "md3"] {
            assert_eq!(
                table.canonical_for(&id(&format!("mathscape:artifact/{md}"))),
                Some(&id("mathscape:canonical-markdown/a"))
            );
        }
        assert_eq!(
            table.members(&id("mathscape:canonical-markdown/a")).len(),
            3
        );
        assert!(table.is_current(&id("mathscape:canonical-markdown/a")));
    }

    #[test]
    fn other_lineages_are_demoted_with_profile_and_policy() {
        let views = [
            view(
                "mathscape:canonical-markdown/old",
                "mathscape:artifact/pdf",
                DEFAULT_PARSER_PROFILE,
                "marker-1.9.2",
                &["mathscape:artifact/old"],
            ),
            view(
                "mathscape:canonical-markdown/new",
                "mathscape:artifact/pdf",
                DEFAULT_PARSER_PROFILE,
                "marker-1.10.0",
                &["mathscape:artifact/new"],
            ),
            view(
                "mathscape:canonical-markdown/legacy",
                "mathscape:artifact/pdf",
                "mathscape.marker.unspecified",
                "marker-9.0.0",
                &["mathscape:artifact/legacy"],
            ),
        ];
        let table = CanonicalAliasTable::from_views(&views);
        assert!(table.is_current(&id("mathscape:canonical-markdown/new")));
        assert_eq!(
            table.title(&views[1], "book.pdf"),
            "book.pdf · 规范解析正文"
        );
        assert_eq!(
            table.title(&views[0], "book.pdf"),
            "book.pdf · 解析正文（旧版 · math-ocr.v1 · marker-1.9.2）"
        );
        assert_eq!(
            table.title(&views[2], "book.pdf"),
            "book.pdf · 解析正文（旧版 · unspecified · marker-9.0.0）"
        );
    }

    #[test]
    fn natural_policy_order() {
        assert_eq!(
            natural_cmp("marker-1.10.0", "marker-1.9.2"),
            Ordering::Greater
        );
        assert_eq!(natural_cmp("marker-2", "marker-10"), Ordering::Less);
        assert_eq!(natural_cmp("unversioned", "unversioned"), Ordering::Equal);
    }
}
