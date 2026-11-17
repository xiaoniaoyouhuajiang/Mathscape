//! User-local study progress sidecar (`study_marks` table, migration 0025).
//!
//! Stored outside `map_objects` so marking chapters or units as `unread`,
//! `studying`, or `mastered` never creates map commits or touches `TrustRecord`.

use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::SqliteStoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyStatus {
    Unread,
    Studying,
    Mastered,
}

impl StudyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Studying => "studying",
            Self::Mastered => "mastered",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, SqliteStoreError> {
        match raw {
            "unread" => Ok(Self::Unread),
            "studying" => Ok(Self::Studying),
            "mastered" => Ok(Self::Mastered),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "study_marks.status",
                value: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyMark {
    pub object_id: String,
    pub status: StudyStatus,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyMarkUpdate {
    pub object_id: String,
    pub status: StudyStatus,
    #[serde(default)]
    pub note: Option<String>,
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'study_marks'",
            [],
            |row| row.get(0),
        )
        .ok();
    if exists.is_none() {
        return Err(SqliteStoreError::MissingSchemaObject {
            object_kind: "table",
            name: "study_marks",
        });
    }
    connection.prepare("SELECT object_id, status, updated_at, note FROM study_marks LIMIT 0")?;
    Ok(())
}

pub(crate) fn list_study_marks(
    connection: &Connection,
) -> Result<BTreeMap<String, StudyMark>, SqliteStoreError> {
    let mut stmt = connection.prepare(
        "SELECT object_id, status, updated_at, note
         FROM study_marks
         ORDER BY object_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let object_id: String = row.get(0)?;
        let status_raw: String = row.get(1)?;
        let updated_at: String = row.get(2)?;
        let note: Option<String> = row.get(3)?;
        Ok((object_id, status_raw, updated_at, note))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (object_id, status_raw, updated_at, note) = row?;
        let status = StudyStatus::parse(&status_raw)?;
        out.insert(
            object_id.clone(),
            StudyMark {
                object_id,
                status,
                updated_at,
                note,
            },
        );
    }
    Ok(out)
}

pub(crate) fn set_study_marks_batch(
    connection: &mut Connection,
    updates: &[StudyMarkUpdate],
    updated_at: &str,
) -> Result<Vec<StudyMark>, SqliteStoreError> {
    let tx = connection.transaction()?;
    let mut applied = Vec::with_capacity(updates.len());
    for item in updates {
        let trimmed_id = item.object_id.trim();
        if trimmed_id.is_empty() {
            continue;
        }
        let cleaned_note = item
            .note
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if item.status == StudyStatus::Unread && cleaned_note.is_none() {
            tx.execute(
                "DELETE FROM study_marks WHERE object_id = ?1",
                params![trimmed_id],
            )?;
        } else {
            tx.execute(
                "INSERT INTO study_marks(object_id, status, updated_at, note)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(object_id) DO UPDATE SET
                     status = excluded.status,
                     updated_at = excluded.updated_at,
                     note = excluded.note",
                params![trimmed_id, item.status.as_str(), updated_at, cleaned_note],
            )?;
        }
        applied.push(StudyMark {
            object_id: trimmed_id.to_owned(),
            status: item.status,
            updated_at: updated_at.to_owned(),
            note: cleaned_note,
        });
    }
    tx.commit()?;
    Ok(applied)
}
