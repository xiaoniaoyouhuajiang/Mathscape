//! Unified workspace search (N3d).
//!
//! Layering:
//! - `mathscape-store-sqlite::SqliteArtifactStore::read_search_corpus` reads
//!   lightweight id / name rows for every type in one read transaction.
//! - Each type is a [`SearchProvider`] that turns its corpus rows into
//!   [`SearchCandidate`]s (title, aliases, subtitle, navigation target).
//! - [`normalize`], [`score`] and [`group_hits`] are pure functions shared by
//!   all providers, so they can be reused directly as agent tools.
//! - The desktop `search_workspace` command only forwards to [`search`].
//!
//! # Extension point
//!
//! A new searchable type (semantic-layer `SemanticEntity`, `Alignment`,
//! `ProofState`; later LaTeX body grep or ink-to-text) needs:
//! 1. a light row section in `SearchCorpus`, read in the same transaction;
//! 2. a [`SearchProvider`] registered in [`SearchRegistry::with_defaults`]
//!    (or [`SearchRegistry::register`]);
//! 3. a [`SearchKind`] value with its Chinese group label.
//!
//! [`SearchQuery`] / [`SearchResponse`], the Tauri command and the frontend
//! panel stay unchanged: group labels come from here, and navigation reuses
//! the generic [`SearchTarget::MapNode`] / reader targets.

pub mod normalize;
pub mod providers;
pub mod score;

use mathscape_domain::MarkdownCoverage;
use mathscape_store_sqlite::SearchCorpus;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub use normalize::{NormalizedText, normalize, query_terms};
pub use score::{FieldMatch, MatchKind, TermMatch, score_fields, score_term};

/// Default and maximum hits returned per group.
pub const DEFAULT_GROUP_LIMIT: usize = 8;
pub const MAX_GROUP_LIMIT: usize = 50;
/// Queries longer than this (in chars) are truncated before matching.
pub const MAX_QUERY_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Note,
    SourceUnit,
    Material,
    Snapshot,
}

impl SearchKind {
    /// Chinese group heading shown by the frontend.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Note => "手稿",
            Self::SourceUnit => "来源单元",
            Self::Material => "材料",
            Self::Snapshot => "命名快照",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    /// Include trashed notes and their snapshots.
    #[serde(default)]
    pub include_trashed: bool,
    /// Restrict to these kinds; empty means all.
    #[serde(default)]
    pub kinds: Vec<SearchKind>,
    #[serde(default)]
    pub limit_per_group: Option<usize>,
}

/// Where a result navigates. The frontend handles each variant generically.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchTarget {
    /// Locate a map node (generic; for future semantic-layer kinds).
    MapNode { node_id: String },
    /// A source object: locate its map node; open resolves the containing
    /// material and focus first.
    SourceObject { object_id: String },
    /// A material: locate its map node; open shows it in the reader.
    Artifact { artifact_id: String },
    /// Open the handwriting page on this note.
    InkNote { document_id: String },
    /// Open the snapshot reader.
    InkSnapshot {
        snapshot_id: String,
        artifact_id: String,
        document_id: String,
    },
}

/// What a provider produces for one searchable row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchCandidate {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub subtitle: Option<String>,
    pub target: SearchTarget,
    /// Trashed rows are hidden unless `include_trashed`.
    pub trashed: bool,
    /// Tie-breaker, larger is more recent (unix ms or 0).
    pub recency: i64,
}

/// One searchable type. Providers only map corpus rows to candidates;
/// normalization, scoring and grouping are shared.
pub trait SearchProvider: Send + Sync {
    fn kind(&self) -> SearchKind;
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate>;
}

pub struct SearchRegistry {
    providers: Vec<Box<dyn SearchProvider>>,
}

impl SearchRegistry {
    pub fn empty() -> Self {
        Self { providers: vec![] }
    }

    /// Built-in N3d providers: notes, source units, materials, named snapshots.
    pub fn with_defaults() -> Self {
        let mut registry = Self::empty();
        registry.register(Box::new(providers::NoteProvider));
        registry.register(Box::new(providers::SourceUnitProvider));
        registry.register(Box::new(providers::MaterialProvider));
        registry.register(Box::new(providers::SnapshotProvider));
        registry
    }

    pub fn register(&mut self, provider: Box<dyn SearchProvider>) {
        self.providers.push(provider);
    }

    pub fn kinds(&self) -> Vec<SearchKind> {
        self.providers
            .iter()
            .map(|provider| provider.kind())
            .collect()
    }
}

impl Default for SearchRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// The alias that matched when it beat the title.
    pub matched_alias: Option<String>,
    pub score: u32,
    /// Matched ranges in `title` as `[start, end)` UTF-16 offsets.
    pub highlights: Vec<[u32; 2]>,
    pub trashed: bool,
    pub target: SearchTarget,
    /// Set by MCP when this material belongs to a canonical markdown view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<MarkdownCoverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
    #[serde(skip)]
    pub recency: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchGroup {
    pub kind: SearchKind,
    pub label: String,
    /// All matches in this group; `hits` holds at most the group limit.
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub total: usize,
    pub groups: Vec<SearchGroup>,
}

/// Converts matched source-char positions into merged UTF-16 ranges.
pub fn highlight_ranges(title: &str, positions: &[usize]) -> Vec<[u32; 2]> {
    let mut offsets = Vec::with_capacity(title.chars().count() + 1);
    let mut unit = 0_u32;
    for c in title.chars() {
        offsets.push(unit);
        unit += u32::try_from(c.len_utf16()).unwrap_or(2);
    }
    offsets.push(unit);
    let mut sorted = positions.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<[u32; 2]> = Vec::new();
    for index in sorted
        .into_iter()
        .filter(|&index| index + 1 < offsets.len())
    {
        let (start, end) = (offsets[index], offsets[index + 1]);
        match ranges.last_mut() {
            Some(last) if last[1] == start => last[1] = end,
            _ => ranges.push([start, end]),
        }
    }
    ranges
}

/// Scores one candidate; `None` when it does not match.
pub fn score_candidate(
    kind: SearchKind,
    terms: &[NormalizedText],
    candidate: SearchCandidate,
) -> Option<SearchHit> {
    let fields: Vec<_> = std::iter::once(candidate.title.as_str())
        .chain(candidate.aliases.iter().map(String::as_str))
        .map(normalize)
        .collect();
    let found = score_fields(terms, &fields)?;
    let penalty = if candidate.trashed { 5 } else { 0 };
    Some(SearchHit {
        kind,
        highlights: highlight_ranges(&candidate.title, &found.title_positions),
        matched_alias: found
            .alias
            .and_then(|index| candidate.aliases.get(index).cloned())
            .filter(|_| found.title_positions.is_empty()),
        score: found.score.saturating_sub(penalty),
        id: candidate.id,
        title: candidate.title,
        subtitle: candidate.subtitle,
        trashed: candidate.trashed,
        target: candidate.target,
        canonical_id: None,
        coverage: None,
        partial: None,
        recency: candidate.recency,
    })
}

/// Score desc, then recency desc, shorter title, title, id.
pub fn compare_hits(a: &SearchHit, b: &SearchHit) -> Ordering {
    b.score
        .cmp(&a.score)
        .then(b.recency.cmp(&a.recency))
        .then(a.title.chars().count().cmp(&b.title.chars().count()))
        .then(a.title.cmp(&b.title))
        .then(a.id.cmp(&b.id))
}

/// Sorts each group, keeps `limit` hits, and orders groups by their best hit
/// (ties keep registry order). Empty groups are dropped.
pub fn group_hits(groups: Vec<(SearchKind, Vec<SearchHit>)>, limit: usize) -> Vec<SearchGroup> {
    let mut result: Vec<(usize, SearchGroup)> = groups
        .into_iter()
        .enumerate()
        .filter(|(_, (_, hits))| !hits.is_empty())
        .map(|(order, (kind, mut hits))| {
            hits.sort_by(compare_hits);
            let total = hits.len();
            hits.truncate(limit);
            (
                order,
                SearchGroup {
                    kind,
                    label: kind.label().to_owned(),
                    total,
                    hits,
                },
            )
        })
        .collect();
    result.sort_by(|(a_order, a), (b_order, b)| {
        b.hits[0]
            .score
            .cmp(&a.hits[0].score)
            .then(a_order.cmp(b_order))
    });
    result.into_iter().map(|(_, group)| group).collect()
}

/// Runs a query over a corpus with the given providers.
pub fn search(
    registry: &SearchRegistry,
    corpus: &SearchCorpus,
    query: &SearchQuery,
) -> SearchResponse {
    let text: String = query.text.chars().take(MAX_QUERY_CHARS).collect();
    let terms = query_terms(&text);
    if terms.is_empty() {
        return SearchResponse {
            query: text,
            ..SearchResponse::default()
        };
    }
    let limit = query
        .limit_per_group
        .unwrap_or(DEFAULT_GROUP_LIMIT)
        .clamp(1, MAX_GROUP_LIMIT);
    let groups = registry
        .providers
        .iter()
        .filter(|provider| query.kinds.is_empty() || query.kinds.contains(&provider.kind()))
        .map(|provider| {
            let kind = provider.kind();
            let hits = provider
                .candidates(corpus)
                .into_iter()
                .filter(|candidate| query.include_trashed || !candidate.trashed)
                .filter_map(|candidate| score_candidate(kind, &terms, candidate))
                .collect();
            (kind, hits)
        })
        .collect();
    let groups = group_hits(groups, limit);
    SearchResponse {
        query: text,
        total: groups.iter().map(|group| group.total).sum(),
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_ranges_use_utf16_and_merge() {
        assert_eq!(highlight_ranges("整理稿", &[0, 1]), vec![[0, 2]]);
        assert_eq!(highlight_ranges("a𝔸b", &[1, 2]), vec![[1, 4]]);
        assert_eq!(highlight_ranges("abc", &[0, 2, 9]), vec![[0, 1], [2, 3]]);
    }
}
