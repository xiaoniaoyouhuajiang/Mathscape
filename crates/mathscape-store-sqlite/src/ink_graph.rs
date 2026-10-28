//! Read-time ink rows for the MCP graph tools (Phase1d-a).
//!
//! `read_ink_graph` reads, in ONE read transaction, every note head, every
//! ink snapshot (the full `ink_snapshots` table, named or not) and every
//! active note link. The graph tools derive `has_snapshot` / `note_link`
//! edges from these rows at read time; nothing is persisted.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::{InkNoteLink, SqliteArtifactStore, SqliteStoreError, ink_link};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InkGraphRows {
    pub notes: Vec<InkGraphNote>,
    pub snapshots: Vec<InkGraphSnapshot>,
    /// Active links (`state='active'`) of every note, trashed notes included.
    pub links: Vec<InkNoteLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkGraphNote {
    pub document_id: String,
    pub title: String,
    pub trashed: bool,
    pub first_page_id: Option<String>,
    pub page_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkGraphSnapshot {
    pub snapshot_id: String,
    pub artifact_id: String,
    pub document_id: String,
    pub through_seq: u64,
    pub label: Option<String>,
    pub note_trashed: bool,
    pub first_page_id: Option<String>,
    pub page_count: u64,
}

fn notes(connection: &Connection) -> Result<Vec<InkGraphNote>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT document_id,title,lifecycle,
                json_extract(pages_json,'$[0].id'),
                COALESCE(json_array_length(pages_json),0)
         FROM ink_documents ORDER BY document_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(InkGraphNote {
            document_id: row.get(0)?,
            title: row.get(1)?,
            trashed: row.get::<_, String>(2)? == "trashed",
            first_page_id: row.get(3)?,
            page_count: u64::try_from(row.get::<_, i64>(4)?).unwrap_or_default(),
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn snapshots(connection: &Connection) -> Result<Vec<InkGraphSnapshot>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT s.snapshot_id,s.artifact_id,s.document_id,s.through_seq,s.label,d.lifecycle,
                json_extract(s.manifest_json,'$.document.pages[0].id'),
                COALESCE(json_array_length(s.manifest_json,'$.document.pages'),0)
         FROM ink_snapshots s JOIN ink_documents d ON d.document_id=s.document_id
         ORDER BY s.document_id, s.through_seq, s.snapshot_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(InkGraphSnapshot {
            snapshot_id: row.get(0)?,
            artifact_id: row.get(1)?,
            document_id: row.get(2)?,
            through_seq: u64::from(row.get::<_, u32>(3)?),
            label: row.get(4)?,
            note_trashed: row.get::<_, String>(5)? == "trashed",
            first_page_id: row.get(6)?,
            page_count: u64::try_from(row.get::<_, i64>(7)?).unwrap_or_default(),
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

impl SqliteArtifactStore {
    /// Note heads, all snapshots and all active note links, read in one
    /// transaction (Phase1d-a graph edges).
    pub fn read_ink_graph(&self) -> Result<InkGraphRows, SqliteStoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let rows = InkGraphRows {
            notes: notes(&transaction)?,
            snapshots: snapshots(&transaction)?,
            links: ink_link::all_active_links(&transaction)?,
        };
        transaction.commit()?;
        Ok(rows)
    }
}
