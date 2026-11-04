//! Lightweight rows for workspace search (N3d).
//!
//! `read_search_corpus` reads only identifiers, names and the few context
//! fields a result row needs, for every searchable type, inside ONE read
//! transaction so the groups are mutually consistent. It deliberately does not
//! build the landscape / desktop snapshot: no payload bodies, endpoints,
//! locations or manifests are loaded. Scoring lives in `mathscape-query`.
//!
//! Extension point: a new searchable type (for example semantic-layer
//! entities) adds one `Vec<...Row>` section here, read in the same
//! transaction, plus a provider in `mathscape_query::search`.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::{SqliteArtifactStore, SqliteStoreError};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchCorpus {
    pub notes: Vec<SearchNoteRow>,
    pub snapshots: Vec<SearchSnapshotRow>,
    pub artifacts: Vec<SearchArtifactRow>,
    pub source_units: Vec<SearchSourceUnitRow>,
    /// Registered canonical markdown lineages (the same rows the map folds),
    /// so the material provider collapses folded snapshots into one hit.
    #[serde(default)]
    pub canonical_markdown: Vec<mathscape_domain::CanonicalMarkdownView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchNoteRow {
    pub document_id: String,
    pub title: String,
    pub trashed: bool,
    /// Unix milliseconds; 0 when never recorded.
    pub last_used_at: i64,
}

/// Only named snapshots are searchable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchSnapshotRow {
    pub snapshot_id: String,
    pub artifact_id: String,
    pub document_id: String,
    pub through_seq: u64,
    pub label: String,
    pub created_at: String,
    pub note_title: String,
    pub note_trashed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchArtifactRow {
    pub artifact_id: String,
    pub media_type: String,
    pub original_filename: Option<String>,
    pub lifecycle: String,
    /// File names of the inputs of conversions that produced this artifact.
    pub origin_filenames: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchSourceUnitRow {
    pub object_id: String,
    pub lifecycle: String,
    pub unit_kind: Option<String>,
    pub display_title: Option<String>,
    pub source_title: Option<String>,
    pub source_labels: Vec<String>,
}

fn unsigned(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn notes(connection: &Connection) -> Result<Vec<SearchNoteRow>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT document_id,title,lifecycle,last_used_at FROM ink_documents ORDER BY document_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SearchNoteRow {
            document_id: row.get(0)?,
            title: row.get(1)?,
            trashed: row.get::<_, String>(2)? == "trashed",
            last_used_at: row.get(3)?,
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn snapshots(connection: &Connection) -> Result<Vec<SearchSnapshotRow>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT s.snapshot_id,s.artifact_id,s.document_id,s.through_seq,s.label,s.created_at,
                d.title,d.lifecycle
         FROM ink_snapshots s JOIN ink_documents d ON d.document_id=s.document_id
         WHERE s.label IS NOT NULL
         ORDER BY s.snapshot_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SearchSnapshotRow {
            snapshot_id: row.get(0)?,
            artifact_id: row.get(1)?,
            document_id: row.get(2)?,
            through_seq: unsigned(row.get(3)?, 3)?,
            label: row.get(4)?,
            created_at: row.get(5)?,
            note_title: row.get(6)?,
            note_trashed: row.get::<_, String>(7)? == "trashed",
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn artifacts(connection: &Connection) -> Result<Vec<SearchArtifactRow>, SqliteStoreError> {
    let mut origins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut statement = connection.prepare(
        "SELECT DISTINCT o.artifact_id, r.original_filename
         FROM conversion_endpoints o
         JOIN conversion_endpoints i
           ON i.run_id=o.run_id AND i.revision_id=o.revision_id AND i.role='input'
         JOIN artifacts a ON a.artifact_id=i.artifact_id
         JOIN artifact_revisions r ON r.artifact_id=a.artifact_id AND r.revision_id=a.current_revision
         WHERE o.role='output' AND r.original_filename IS NOT NULL
         ORDER BY o.artifact_id, r.original_filename",
    )?;
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))? {
        let (artifact, filename) = row?;
        origins.entry(artifact).or_default().push(filename);
    }
    drop(statement);
    let mut statement = connection.prepare(
        "SELECT a.artifact_id, r.media_type, r.original_filename, r.lifecycle
         FROM artifacts a
         JOIN artifact_revisions r ON r.artifact_id=a.artifact_id AND r.revision_id=a.current_revision
         ORDER BY a.artifact_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (artifact_id, media_type, original_filename, lifecycle) = row?;
        result.push(SearchArtifactRow {
            origin_filenames: origins.remove(&artifact_id).unwrap_or_default(),
            artifact_id,
            media_type,
            original_filename,
            lifecycle,
        });
    }
    Ok(result)
}

fn text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn source_units(connection: &Connection) -> Result<Vec<SearchSourceUnitRow>, SqliteStoreError> {
    // Only the name fields are extracted; bodies stay in SQLite. Stored
    // payloads may wrap the unit in a `payload` envelope.
    let mut statement = connection.prepare(
        "SELECT o.object_id, r.lifecycle,
                COALESCE(json_extract(r.payload_json,'$.payload.kind'), json_extract(r.payload_json,'$.kind')),
                COALESCE(json_extract(r.payload_json,'$.payload.display_title'), json_extract(r.payload_json,'$.display_title')),
                COALESCE(json_extract(r.payload_json,'$.payload.source_title.text'), json_extract(r.payload_json,'$.source_title.text')),
                COALESCE(json_extract(r.payload_json,'$.payload.source_labels'), json_extract(r.payload_json,'$.source_labels'))
         FROM map_objects o
         JOIN map_object_revisions r ON r.object_id=o.object_id AND r.revision_id=o.current_revision
         WHERE o.entity_kind='SourceSemanticUnit' AND o.object_shape='entity'
         ORDER BY o.object_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (object_id, lifecycle, unit_kind, display_title, source_title, labels) = row?;
        let source_labels = match labels {
            Some(json) => serde_json::from_str::<Value>(&json)?
                .as_array()
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|label| label.get("text").and_then(Value::as_str))
                        .map(str::trim)
                        .filter(|label| !label.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            None => vec![],
        };
        result.push(SearchSourceUnitRow {
            object_id,
            lifecycle,
            unit_kind,
            display_title: text(display_title),
            source_title: text(source_title),
            source_labels,
        });
    }
    Ok(result)
}

impl SqliteArtifactStore {
    /// Every searchable row, read in one transaction (N3d).
    pub fn read_search_corpus(&self) -> Result<SearchCorpus, SqliteStoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let corpus = SearchCorpus {
            notes: notes(&transaction)?,
            snapshots: snapshots(&transaction)?,
            artifacts: artifacts(&transaction)?,
            source_units: source_units(&transaction)?,
            canonical_markdown: crate::canonical_markdown::list_canonical_markdown(&transaction)?,
        };
        transaction.commit()?;
        Ok(corpus)
    }
}
