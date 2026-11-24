//! Persistence for Phase2 `user.ask` pending questions (`pending_questions`, migration 0027).
//!
//! Questions are scoped to `investigation_id` (`sandbox:<session_id>` in the
//! sandbox), written non-blockingly by `user.ask` (`state = 'pending'`), and
//! answered or cancelled exclusively by Desktop user commands.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SqliteStoreError;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingQuestionAnswer {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_option: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_snapshot_ref: Option<String>,
    pub answered_by: String,
    pub answered_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingQuestionRecord {
    pub question_id: String,
    pub investigation_id: String,
    pub round_id: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_ref: Option<Value>,
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub ink_requested: bool,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<PendingQuestionAnswer>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePendingQuestionInput {
    pub question_id: String,
    pub investigation_id: String,
    pub round_id: String,
    pub origin: String,
    pub harness_ref: Option<Value>,
    pub question: String,
    pub options: Option<Vec<String>>,
    pub ink_requested: bool,
    pub created_at: String,
}

const SELECT_QUESTION_COLUMNS: &str = "
    SELECT question_id, investigation_id, round_id, origin, harness_ref_json,
           question, options_json, ink_requested, state, answer_json,
           created_at, updated_at
    FROM pending_questions
";

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pending_questions'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(SqliteStoreError::MissingSchemaObject {
            object_kind: "table",
            name: "pending_questions",
        });
    }
    connection.prepare(&format!("{SELECT_QUESTION_COLUMNS} LIMIT 0"))?;
    Ok(())
}

fn row_to_question(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingQuestionRecord> {
    let question_id: String = row.get(0)?;
    let investigation_id: String = row.get(1)?;
    let round_id: String = row.get(2)?;
    let origin: String = row.get(3)?;
    let harness_ref_raw: Option<String> = row.get(4)?;
    let question: String = row.get(5)?;
    let options_raw: Option<String> = row.get(6)?;
    let ink_requested_int: i64 = row.get(7)?;
    let state: String = row.get(8)?;
    let answer_raw: Option<String> = row.get(9)?;
    let created_at: String = row.get(10)?;
    let updated_at: String = row.get(11)?;

    let harness_ref = harness_ref_raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let options = options_raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let answer = answer_raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?;

    Ok(PendingQuestionRecord {
        question_id,
        investigation_id,
        round_id,
        origin,
        harness_ref,
        question,
        options,
        ink_requested: ink_requested_int != 0,
        state,
        answer,
        created_at,
        updated_at,
    })
}

pub(crate) fn create_pending_question(
    connection: &mut Connection,
    input: &CreatePendingQuestionInput,
) -> Result<PendingQuestionRecord, SqliteStoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<PendingQuestionRecord> = tx
        .query_row(
            &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1"),
            [&input.question_id],
            row_to_question,
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing.investigation_id == input.investigation_id
            && existing.round_id == input.round_id
            && existing.question == input.question
            && existing.options == input.options
            && existing.ink_requested == input.ink_requested
        {
            return Ok(existing);
        }
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "question_id_conflict: {} 已绑定其他追问内容",
            input.question_id
        )));
    }
    let other_in_round: Option<String> = tx
        .query_row(
            "SELECT question_id FROM pending_questions WHERE round_id = ?1 LIMIT 1",
            [&input.round_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(other) = other_in_round {
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "awaiting_user: 本轮已发出追问 {other}，请先结束本轮等待用户回答"
        )));
    }
    let harness_ref_json = input
        .harness_ref
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let options_json = input
        .options
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    tx.execute(
        "INSERT INTO pending_questions(
             question_id, investigation_id, round_id, origin, harness_ref_json,
             question, options_json, ink_requested, state, answer_json,
             created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, ?9, ?9)",
        params![
            input.question_id,
            input.investigation_id,
            input.round_id,
            input.origin,
            harness_ref_json,
            input.question,
            options_json,
            i64::from(input.ink_requested),
            input.created_at,
        ],
    )?;
    let created = tx.query_row(
        &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1"),
        [&input.question_id],
        row_to_question,
    )?;
    tx.commit()?;
    Ok(created)
}

pub(crate) fn round_has_pending_question(
    connection: &Connection,
    round_id: &str,
) -> Result<bool, SqliteStoreError> {
    let found: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pending_questions WHERE round_id = ?1 LIMIT 1",
            [round_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub(crate) fn list_pending_questions(
    connection: &Connection,
    investigation_id: &str,
    pending_only: bool,
) -> Result<Vec<PendingQuestionRecord>, SqliteStoreError> {
    let sql = if pending_only {
        format!(
            "{SELECT_QUESTION_COLUMNS}
             WHERE investigation_id = ?1 AND state = 'pending'
             ORDER BY created_at ASC, rowid ASC"
        )
    } else {
        format!(
            "{SELECT_QUESTION_COLUMNS}
             WHERE investigation_id = ?1
             ORDER BY created_at ASC, rowid ASC"
        )
    };
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt.query_map([investigation_id], row_to_question)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub(crate) fn answer_pending_question(
    connection: &mut Connection,
    investigation_id: &str,
    question_id: &str,
    answer: &PendingQuestionAnswer,
) -> Result<PendingQuestionRecord, SqliteStoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<PendingQuestionRecord> = tx
        .query_row(
            &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1 AND investigation_id = ?2"),
            params![question_id, investigation_id],
            row_to_question,
        )
        .optional()?;
    let Some(existing) = existing else {
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "question_not_found: 未找到待答问题 {question_id}"
        )));
    };
    if existing.state != "pending" {
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "question_not_pending: 问题 {question_id} 当前状态为 {}，无法重复回答",
            existing.state
        )));
    }
    if let Some(selected) = &answer.selected_option
        && let Some(options) = &existing.options
        && !options.iter().any(|opt| opt == selected)
    {
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "invalid_selected_option: 选项 {selected:?} 不在原问题候选列表中"
        )));
    }
    let answer_json = serde_json::to_string(answer)?;
    tx.execute(
        "UPDATE pending_questions
         SET state = 'answered', answer_json = ?1, updated_at = ?2
         WHERE question_id = ?3 AND investigation_id = ?4",
        params![
            answer_json,
            answer.answered_at,
            question_id,
            investigation_id
        ],
    )?;
    let updated = tx.query_row(
        &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1"),
        [question_id],
        row_to_question,
    )?;
    tx.commit()?;
    Ok(updated)
}

pub(crate) fn cancel_pending_question(
    connection: &mut Connection,
    investigation_id: &str,
    question_id: &str,
    updated_at: &str,
) -> Result<PendingQuestionRecord, SqliteStoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<PendingQuestionRecord> = tx
        .query_row(
            &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1 AND investigation_id = ?2"),
            params![question_id, investigation_id],
            row_to_question,
        )
        .optional()?;
    let Some(existing) = existing else {
        return Err(SqliteStoreError::PendingQuestionConflict(format!(
            "question_not_found: 未找到待答问题 {question_id}"
        )));
    };
    if existing.state != "pending" {
        return Ok(existing);
    }
    tx.execute(
        "UPDATE pending_questions
         SET state = 'cancelled', updated_at = ?1
         WHERE question_id = ?2 AND investigation_id = ?3",
        params![updated_at, question_id, investigation_id],
    )?;
    let updated = tx.query_row(
        &format!("{SELECT_QUESTION_COLUMNS} WHERE question_id = ?1"),
        [question_id],
        row_to_question,
    )?;
    tx.commit()?;
    Ok(updated)
}
