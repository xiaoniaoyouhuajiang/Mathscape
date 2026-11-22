//! Built-in N3d search providers. Each maps light corpus rows to candidates;
//! see the module docs of `search` for adding semantic-layer providers.

use std::collections::{BTreeMap, BTreeSet};

use mathscape_store_sqlite::{
    SearchArtifactRow, SearchCorpus, SearchSemanticEntityRow, source_unit_kind_name,
};

use super::{SearchCandidate, SearchKind, SearchProvider, SearchTarget};
use crate::artifact_graph::{product_label, readable_filename};
use crate::canonical_alias::{CANONICAL_MARKDOWN_TITLE, CanonicalAliasTable};
use crate::semantic_graph::semantic_kind_label;

/// Retired map objects are hidden on the map, so they are not searchable.
fn retired(lifecycle: &str) -> bool {
    matches!(lifecycle, "superseded" | "rejected" | "deprecated")
}

fn unix_text_ms(value: &str) -> i64 {
    value
        .strip_prefix("unix:")
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .map_or(0, |seconds| seconds.saturating_mul(1000))
}

fn push_alias(aliases: &mut Vec<String>, title: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() && value != title && !aliases.iter().any(|alias| alias == value) {
        aliases.push(value.to_owned());
    }
}

/// Notes by title.
pub struct NoteProvider;

impl SearchProvider for NoteProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::Note
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        corpus
            .notes
            .iter()
            .map(|note| SearchCandidate {
                id: note.document_id.clone(),
                title: note.title.clone(),
                aliases: vec![],
                subtitle: Some(
                    if note.trashed {
                        "手稿 · 回收站"
                    } else {
                        "手稿"
                    }
                    .into(),
                ),
                target: SearchTarget::InkNote {
                    document_id: note.document_id.clone(),
                },
                trashed: note.trashed,
                recency: note.last_used_at,
            })
            .collect()
    }
}

/// Named snapshots by name. Unnamed snapshots are not searchable.
pub struct SnapshotProvider;

impl SearchProvider for SnapshotProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::Snapshot
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        corpus
            .snapshots
            .iter()
            .map(|snapshot| SearchCandidate {
                id: snapshot.snapshot_id.clone(),
                title: snapshot.label.clone(),
                aliases: vec![],
                subtitle: Some(format!(
                    "{}{} · 第 {} 步",
                    snapshot.note_title,
                    if snapshot.note_trashed {
                        "（回收站）"
                    } else {
                        ""
                    },
                    snapshot.through_seq
                )),
                target: SearchTarget::InkSnapshot {
                    snapshot_id: snapshot.snapshot_id.clone(),
                    artifact_id: snapshot.artifact_id.clone(),
                    document_id: snapshot.document_id.clone(),
                },
                trashed: snapshot.note_trashed,
                recency: unix_text_ms(&snapshot.created_at),
            })
            .collect()
    }
}

/// Materials (PDF, Markdown, text) by file name. Parsed images, metadata
/// JSON and snapshot artifacts are left out: images and JSON are noise, and
/// snapshots have their own provider. Markdown snapshots folded into a
/// registered canonical lineage collapse into one 「规范解析正文」 hit whose
/// id and target are the canonical id (the PDF keeps its own hit).
pub struct MaterialProvider;

fn searchable_media(media: &str) -> bool {
    !(media.starts_with("image/")
        || media == "application/json"
        || media == "application/vnd.mathscape.ink-snapshot+json")
}

fn stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(stem, _)| stem)
}

impl SearchProvider for MaterialProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::Material
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        // Same alias table as the map fold: every folded snapshot of a
        // registered lineage collapses into ONE 「规范解析正文」 hit.
        let table = CanonicalAliasTable::from_views(&corpus.canonical_markdown);
        let folded: BTreeMap<String, String> = table
            .aliases()
            .iter()
            .map(|(artifact, canonical)| (artifact.to_string(), canonical.to_string()))
            .collect();
        let mut folded_titles: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut out = Vec::new();
        for artifact in corpus.artifacts.iter().filter(|artifact| {
            searchable_media(&artifact.media_type) && !retired(&artifact.lifecycle)
        }) {
            let candidate = material_candidate(artifact);
            if let Some(canonical) = folded.get(&artifact.artifact_id) {
                // Keep the physical names searchable as aliases of the fold.
                let titles = folded_titles.entry(canonical.clone()).or_default();
                titles.push(candidate.title);
                titles.extend(candidate.aliases);
                continue;
            }
            out.push(candidate);
        }
        let pdf_names: BTreeMap<&str, String> = corpus
            .artifacts
            .iter()
            .filter(|artifact| artifact.media_type == "application/pdf")
            .map(|artifact| {
                (
                    artifact.artifact_id.as_str(),
                    readable_filename(artifact.original_filename.as_deref(), &artifact.media_type),
                )
            })
            .collect();
        let mut seen = BTreeSet::new();
        for view in &corpus.canonical_markdown {
            let canonical = view.canonical_id.to_string();
            if view.head_artifact_id.is_none() || !seen.insert(canonical.clone()) {
                continue;
            }
            let pdf_label = pdf_names
                .get(view.source_pdf_artifact_id.as_str())
                .cloned()
                .unwrap_or_else(|| "PDF".to_owned());
            let title = table.title(view, &pdf_label);
            let mut aliases = Vec::new();
            push_alias(&mut aliases, &title, &pdf_label);
            for value in folded_titles.get(&canonical).into_iter().flatten() {
                push_alias(&mut aliases, &title, value);
            }
            let coverage = view.coverage(None);
            let pages = coverage
                .covered
                .iter()
                .map(|range| {
                    if range.from == range.to {
                        format!("{}", range.from)
                    } else {
                        format!("{}–{}", range.from, range.to)
                    }
                })
                .collect::<Vec<_>>();
            let mut subtitle = if table.is_current(&view.canonical_id) {
                CANONICAL_MARKDOWN_TITLE.to_owned()
            } else {
                "解析正文（旧版）".to_owned()
            };
            if !pages.is_empty() {
                subtitle.push_str(&format!(" · 覆盖第 {} 页", pages.join("、")));
            }
            let snapshots = table.members(&view.canonical_id).len();
            if snapshots > 1 {
                subtitle.push_str(&format!(" · 合并 {snapshots} 份快照"));
            }
            subtitle.push_str(&format!(" · 来自 {pdf_label}"));
            out.push(SearchCandidate {
                id: canonical.clone(),
                title,
                aliases,
                subtitle: Some(subtitle),
                // Locate / open the canonical node (reader opens the
                // unified view for a canonical id).
                target: SearchTarget::Artifact {
                    artifact_id: canonical,
                },
                trashed: false,
                recency: 0,
            });
        }
        out
    }
}

fn material_candidate(artifact: &SearchArtifactRow) -> SearchCandidate {
    let kind = product_label(&artifact.media_type);
    let readable = readable_filename(artifact.original_filename.as_deref(), &artifact.media_type);
    let origins: Vec<&str> = artifact
        .origin_filenames
        .iter()
        .map(|name| stem(name))
        .collect();
    // Hash-named conversion outputs read as "<source> · 解析正文".
    let title = if readable == kind && !origins.is_empty() {
        format!("{} · {kind}", origins.join("、"))
    } else {
        readable
    };
    let mut aliases = Vec::new();
    if let Some(filename) = &artifact.original_filename
        && readable_filename(Some(filename), &artifact.media_type) != kind
    {
        push_alias(&mut aliases, &title, filename);
    }
    for origin in &origins {
        push_alias(&mut aliases, &title, origin);
    }
    let subtitle = if origins.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind} · 来自 {}", origins.join("、"))
    };
    SearchCandidate {
        id: artifact.artifact_id.clone(),
        title,
        aliases,
        subtitle: Some(subtitle),
        target: SearchTarget::Artifact {
            artifact_id: artifact.artifact_id.clone(),
        },
        trashed: false,
        recency: 0,
    }
}

/// Source units by display title, source title and source labels (no body).
pub struct SourceUnitProvider;

impl SearchProvider for SourceUnitProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::SourceUnit
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        corpus
            .source_units
            .iter()
            .filter(|unit| !retired(&unit.lifecycle))
            .map(|unit| {
                let kind_name = unit
                    .unit_kind
                    .as_deref()
                    .and_then(source_unit_kind_name)
                    .unwrap_or("来源单元");
                let title = unit
                    .display_title
                    .clone()
                    .or_else(|| unit.source_labels.first().cloned())
                    .or_else(|| unit.source_title.clone())
                    .unwrap_or_else(|| kind_name.to_owned());
                let mut aliases = Vec::new();
                for value in unit
                    .source_labels
                    .iter()
                    .chain(unit.source_title.iter())
                    .chain(unit.display_title.iter())
                {
                    push_alias(&mut aliases, &title, value);
                }
                let subtitle = match unit.source_labels.first() {
                    Some(label) if label != &title => format!("{kind_name} · {label}"),
                    _ => kind_name.to_owned(),
                };
                SearchCandidate {
                    id: unit.object_id.clone(),
                    title,
                    aliases,
                    subtitle: Some(subtitle),
                    target: SearchTarget::SourceObject {
                        object_id: unit.object_id.clone(),
                    },
                    trashed: false,
                    recency: 0,
                }
            })
            .collect()
    }
}

fn semantic_entity_title(row: &SearchSemanticEntityRow) -> String {
    row.display_label
        .clone()
        .or_else(|| row.plain_label.clone())
        .or_else(|| row.preferred_labels.first().cloned())
        .unwrap_or_else(|| {
            semantic_kind_label(row.semantic_kind.as_deref().unwrap_or("")).to_owned()
        })
}

fn semantic_entity_candidate(
    row: &SearchSemanticEntityRow,
    theory_titles: &BTreeMap<&str, String>,
) -> SearchCandidate {
    let kind_name = semantic_kind_label(row.semantic_kind.as_deref().unwrap_or(""));
    let title = semantic_entity_title(row);
    let mut aliases = Vec::new();
    for value in row
        .preferred_labels
        .iter()
        .chain(row.aliases.iter())
        .chain(row.mathlib_decl.iter())
    {
        push_alias(&mut aliases, &title, value);
    }
    let mut subtitle = kind_name.to_owned();
    if let Some(home) = row
        .home_theory_id
        .as_deref()
        .and_then(|id| theory_titles.get(id))
    {
        subtitle.push_str(&format!(" · {home}"));
    }
    if let Some(decl) = &row.mathlib_decl {
        subtitle.push_str(&format!(" · {decl}"));
    }
    SearchCandidate {
        id: row.object_id.clone(),
        title,
        aliases,
        subtitle: Some(subtitle),
        target: SearchTarget::MapNode {
            node_id: row.object_id.clone(),
        },
        trashed: false,
        recency: 0,
    }
}

/// Theories (`semantic_kind = "theory"`) by plain label, preferred labels and aliases.
pub struct TheoryProvider;

impl SearchProvider for TheoryProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::Theory
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        let empty = BTreeMap::new();
        corpus
            .semantic_entities
            .iter()
            .filter(|row| {
                !retired(&row.lifecycle) && row.semantic_kind.as_deref() == Some("theory")
            })
            .map(|row| semantic_entity_candidate(row, &empty))
            .collect()
    }
}

/// Shared objects other than theories (`concept`, `declaration`, `problem_schema`).
pub struct SemanticEntityProvider;

impl SearchProvider for SemanticEntityProvider {
    fn kind(&self) -> SearchKind {
        SearchKind::SemanticEntity
    }
    fn candidates(&self, corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        let theory_titles: BTreeMap<&str, String> = corpus
            .semantic_entities
            .iter()
            .filter(|row| {
                !retired(&row.lifecycle) && row.semantic_kind.as_deref() == Some("theory")
            })
            .map(|row| (row.object_id.as_str(), semantic_entity_title(row)))
            .collect();
        corpus
            .semantic_entities
            .iter()
            .filter(|row| {
                !retired(&row.lifecycle) && row.semantic_kind.as_deref() != Some("theory")
            })
            .map(|row| semantic_entity_candidate(row, &theory_titles))
            .collect()
    }
}
