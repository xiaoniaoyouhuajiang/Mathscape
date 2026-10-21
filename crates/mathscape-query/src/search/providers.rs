//! Built-in N3d search providers. Each maps light corpus rows to candidates;
//! see the module docs of `search` for adding semantic-layer providers.

use mathscape_store_sqlite::{SearchCorpus, source_unit_kind_name};

use super::{SearchCandidate, SearchKind, SearchProvider, SearchTarget};
use crate::artifact_graph::{product_label, readable_filename};

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
/// snapshots have their own provider.
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
        corpus
            .artifacts
            .iter()
            .filter(|artifact| {
                searchable_media(&artifact.media_type) && !retired(&artifact.lifecycle)
            })
            .map(|artifact| {
                let kind = product_label(&artifact.media_type);
                let readable =
                    readable_filename(artifact.original_filename.as_deref(), &artifact.media_type);
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
            })
            .collect()
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
