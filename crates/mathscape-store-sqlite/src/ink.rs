use mathscape_domain::{InkLabDocument, InkLabDocumentMetadata, InkLabOperation};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{SqliteStoreError, table_has_column};

pub const INK_STORAGE_VERSION: &str = "mathscape-ink-storage-1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredInkDocument {
    pub document: InkLabDocument,
    pub head_seq: u64,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredInkDocumentSummary {
    pub document_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub head_seq: u64,
    pub session_count: u64,
    pub active_session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredInkSession {
    pub session_id: String,
    pub document_id: String,
    pub writer_id: String,
    pub writer_epoch: u64,
    pub status: String,
    pub device: Value,
    pub started_at: String,
    pub last_seen_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkOperationReceipt {
    pub request_id: String,
    pub document_id: String,
    pub command_id: String,
    pub seq: u64,
    pub operation_fingerprint: String,
    pub durable: bool,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Clone, Debug, Serialize)]
struct RequestFingerprint<'a> {
    document_id: &'a str,
    session_id: &'a str,
    writer_epoch: u64,
    expected_head: u64,
    metadata: &'a InkLabDocumentMetadata,
    operation: &'a InkLabOperation,
}

trait InkQueryConnection {
    fn connection(&self) -> &Connection;
}

impl InkQueryConnection for Connection {
    fn connection(&self) -> &Connection {
        self
    }
}

impl<'a> InkQueryConnection for Transaction<'a> {
    fn connection(&self) -> &Connection {
        self
    }
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 13",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_p1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 13 is absent or has an unexpected name",
        ));
    }
    for (table, columns) in [
        (
            "ink_documents",
            [
                "document_id",
                "version",
                "title",
                "created_at",
                "exported_at",
                "pages_json",
                "capture_segments_json",
                "head_seq",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "ink_sessions",
            [
                "session_id",
                "document_id",
                "writer_id",
                "writer_epoch",
                "status",
                "device_json",
                "started_at",
                "last_seen_at",
            ]
            .as_slice(),
        ),
        (
            "ink_operations",
            [
                "document_id",
                "seq",
                "command_id",
                "operation_json",
                "operation_fingerprint",
                "recorded_at",
            ]
            .as_slice(),
        ),
        (
            "ink_writer_state",
            [
                "document_id",
                "writer_epoch",
                "active_session_id",
                "writer_id",
                "head_seq",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "ink_requests",
            [
                "request_id",
                "document_id",
                "request_fingerprint",
                "document_seq",
                "response_json",
                "created_at",
            ]
            .as_slice(),
        ),
    ] {
        if !schema_object_exists(connection, "table", table)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "table",
                name: table,
            });
        }
        for column in columns {
            if !table_has_column(connection, table, column)? {
                return Err(SqliteStoreError::MissingSchemaObject {
                    object_kind: "column",
                    name: column,
                });
            }
        }
    }
    for index in [
        "ink_sessions_by_document",
        "ink_operations_by_command",
        "ink_requests_by_document",
    ] {
        if !schema_object_exists(connection, "index", index)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "index",
                name: index,
            });
        }
    }
    Ok(())
}

pub(crate) fn create_document(
    connection: &mut Connection,
    document: &InkLabDocument,
    session_id: &str,
    writer_id: &str,
    device: &Value,
) -> Result<StoredInkSession, SqliteStoreError> {
    validate_identifier(document.document_id.as_str(), "document_id")?;
    validate_identifier(session_id, "session_id")?;
    validate_identifier(writer_id, "writer_id")?;
    document
        .validate()
        .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
    if !document.operations.is_empty() {
        return Err(SqliteStoreError::InkInvariant(
            "a new P1 document must not already contain operations".to_owned(),
        ));
    }
    let pages = serde_json::to_string(&document.pages)?;
    let segments = serde_json::to_string(&document.capture_segments)?;
    let device_json = serde_json::to_string(device)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if transaction
        .query_row(
            "SELECT 1 FROM ink_documents WHERE document_id = ?1",
            [document.document_id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Err(SqliteStoreError::InkInvariant(
            "document_id is already registered".to_owned(),
        ));
    }
    let now = document.created_at.clone();
    transaction.execute(
        "INSERT INTO ink_documents
         (document_id, version, title, created_at, exported_at, pages_json,
          capture_segments_json, head_seq, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?4)",
        params![
            document.document_id,
            document.version,
            document.title,
            document.created_at,
            document.exported_at,
            pages,
            segments,
        ],
    )?;
    transaction.execute(
        "INSERT INTO ink_sessions
         (session_id, document_id, writer_id, writer_epoch, status, device_json,
          started_at, last_seen_at)
         VALUES (?1, ?2, ?3, 1, 'active', ?4, ?5, ?5)",
        params![
            session_id,
            document.document_id,
            writer_id,
            device_json,
            document.created_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO ink_writer_state
         (document_id, writer_epoch, active_session_id, writer_id, head_seq, updated_at)
         VALUES (?1, 1, ?2, ?3, 0, ?4)",
        params![document.document_id, session_id, writer_id, now],
    )?;
    transaction.commit()?;
    read_session(connection, session_id)?.ok_or_else(|| {
        SqliteStoreError::InkInvariant("new session disappeared after commit".to_owned())
    })
}

pub(crate) fn start_session(
    connection: &mut Connection,
    document_id: &str,
    session_id: &str,
    writer_id: &str,
    device: &Value,
) -> Result<StoredInkSession, SqliteStoreError> {
    validate_identifier(document_id, "document_id")?;
    validate_identifier(session_id, "session_id")?;
    validate_identifier(writer_id, "writer_id")?;
    let device_json = serde_json::to_string(device)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    transaction
        .query_row(
            "SELECT 1 FROM ink_documents WHERE document_id = ?1",
            [document_id],
            |_| Ok(()),
        )
        .optional()?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.to_owned()))?;
    if let Some(existing) = transaction
        .query_row(
            "SELECT session_id, document_id, writer_id, writer_epoch, status, device_json,
                    started_at, last_seen_at
             FROM ink_sessions WHERE session_id = ?1",
            [session_id],
            session_from_row,
        )
        .optional()?
    {
        if existing.document_id == document_id && existing.writer_id == writer_id {
            if existing.status == "active" {
                transaction.commit()?;
                return Ok(existing);
            }
            let (epoch, head_seq): (i64, i64) = transaction.query_row(
                "SELECT writer_epoch, head_seq FROM ink_writer_state WHERE document_id = ?1",
                [document_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let now = sqlite_now(&transaction)?;
            transaction.execute(
                "UPDATE ink_sessions SET status = 'active', device_json = ?2, last_seen_at = ?3
                 WHERE session_id = ?1",
                params![session_id, device_json, now],
            )?;
            transaction.execute(
                "UPDATE ink_writer_state
                 SET active_session_id = ?2, writer_id = ?3, head_seq = ?4, updated_at = ?5
                 WHERE document_id = ?1",
                params![document_id, session_id, writer_id, head_seq, now],
            )?;
            transaction.commit()?;
            let resumed = read_session(connection, session_id)?.ok_or_else(|| {
                SqliteStoreError::InkInvariant(
                    "resumed session disappeared after commit".to_owned(),
                )
            })?;
            debug_assert_eq!(resumed.writer_epoch as i64, epoch);
            return Ok(resumed);
        }
        return Err(SqliteStoreError::InkInvariant(
            "session_id is already registered for a different writer or document".to_owned(),
        ));
    }
    let active: Option<(String, String)> = transaction
        .query_row(
            "SELECT session_id, writer_id FROM ink_sessions
             WHERE document_id = ?1 AND status = 'active'
             ORDER BY last_seen_at DESC LIMIT 1",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((active_session, active_writer)) = active {
        return Err(SqliteStoreError::InkWriterConflict {
            document_id: document_id.to_owned(),
            session_id: active_session,
            writer_id: active_writer,
        });
    }
    let (previous_epoch, head_seq): (i64, i64) = transaction.query_row(
        "SELECT writer_epoch, head_seq FROM ink_writer_state WHERE document_id = ?1",
        [document_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let epoch = checked_next_i64(previous_epoch, "writer_epoch")?;
    let now = sqlite_now(&transaction)?;
    transaction.execute(
        "INSERT INTO ink_sessions
         (session_id, document_id, writer_id, writer_epoch, status, device_json,
          started_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?6)",
        params![session_id, document_id, writer_id, epoch, device_json, now],
    )?;
    transaction.execute(
        "UPDATE ink_writer_state
         SET writer_epoch = ?2, active_session_id = ?3, writer_id = ?4,
             head_seq = ?5, updated_at = ?6
         WHERE document_id = ?1",
        params![document_id, epoch, session_id, writer_id, head_seq, now],
    )?;
    transaction.commit()?;
    read_session(connection, session_id)?.ok_or_else(|| {
        SqliteStoreError::InkInvariant("new session disappeared after commit".to_owned())
    })
}

pub(crate) fn close_session(
    connection: &mut Connection,
    session_id: &str,
) -> Result<(), SqliteStoreError> {
    validate_identifier(session_id, "session_id")?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let document_id: Option<String> = transaction
        .query_row(
            "SELECT document_id FROM ink_sessions WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(document_id) = document_id else {
        return Err(SqliteStoreError::InkSessionNotFound(session_id.to_owned()));
    };
    let now = sqlite_now(&transaction)?;
    transaction.execute(
        "UPDATE ink_sessions SET status = 'closed', last_seen_at = ?2 WHERE session_id = ?1",
        params![session_id, now],
    )?;
    transaction.execute(
        "UPDATE ink_writer_state SET active_session_id = NULL, writer_id = NULL, updated_at = ?2
         WHERE document_id = ?1 AND active_session_id = ?3",
        params![document_id, now, session_id],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn append_operation(
    connection: &mut Connection,
    request_id: &str,
    document_id: &str,
    session_id: &str,
    writer_epoch: u64,
    expected_head: u64,
    metadata: &InkLabDocumentMetadata,
    operation: &InkLabOperation,
) -> Result<InkOperationReceipt, SqliteStoreError> {
    validate_identifier(request_id, "request_id")?;
    validate_identifier(document_id, "document_id")?;
    validate_identifier(session_id, "session_id")?;
    let fingerprint = request_fingerprint(
        document_id,
        session_id,
        writer_epoch,
        expected_head,
        metadata,
        operation,
    )?;
    let operation_json = serde_json::to_string(operation)?;
    let operation_fingerprint = sha256_hex(operation_json.as_bytes());
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    if let Some((stored_fingerprint, response_json)) = transaction
        .query_row(
            "SELECT request_fingerprint, response_json FROM ink_requests WHERE request_id = ?1",
            [request_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if stored_fingerprint != fingerprint {
            return Err(SqliteStoreError::InkRequestConflict(request_id.to_owned()));
        }
        let mut receipt: InkOperationReceipt = serde_json::from_str(&response_json)?;
        receipt.replayed = true;
        transaction.commit()?;
        return Ok(receipt);
    }

    let (actual_head, actual_epoch, active_session): (i64, i64, Option<String>) = transaction
        .query_row(
            "SELECT head_seq, writer_epoch, active_session_id
             FROM ink_writer_state WHERE document_id = ?1",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.to_owned()))?;
    if actual_epoch != sqlite_u64(writer_epoch, "writer_epoch")?
        || active_session.as_deref() != Some(session_id)
    {
        return Err(SqliteStoreError::InkWriterEpochConflict {
            document_id: document_id.to_owned(),
            expected: writer_epoch,
            actual: rust_u64(actual_epoch, "writer_epoch")?,
        });
    }
    let actual_head = rust_u64(actual_head, "ink head")?;
    if actual_head != expected_head {
        return Err(SqliteStoreError::InkHeadConflict {
            expected: expected_head,
            actual: actual_head,
        });
    }
    let expected_seq = checked_next_u64(actual_head, "ink sequence")?;
    if u64::from(operation.seq()) != expected_seq {
        return Err(SqliteStoreError::InkSequenceConflict {
            expected: expected_seq,
            actual: u64::from(operation.seq()),
        });
    }
    if transaction
        .query_row(
            "SELECT 1 FROM ink_operations WHERE document_id = ?1 AND command_id = ?2",
            params![document_id, operation.command_id()],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Err(SqliteStoreError::InkInvariant(
            "command_id is already registered for this document".to_owned(),
        ));
    }

    if metadata.document_id != document_id {
        return Err(SqliteStoreError::InkInvariant(
            "operation metadata refers to a different document".to_owned(),
        ));
    }
    let mut document = load_document(&transaction, document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.to_owned()))?;
    if metadata.version != document.document.version
        || metadata.created_at != document.document.created_at
    {
        return Err(SqliteStoreError::InkInvariant(
            "document version and created_at are immutable".to_owned(),
        ));
    }
    document.document.version = metadata.version.clone();
    document.document.title = metadata.title.clone();
    document.document.created_at = metadata.created_at.clone();
    document.document.exported_at = metadata.exported_at.clone();
    document.document.pages = metadata.pages.clone();
    document.document.capture_segments = metadata.capture_segments.clone();
    document.document.operations.push(operation.clone());
    document
        .document
        .validate()
        .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
    let pages_json = serde_json::to_string(&metadata.pages)?;
    let segments_json = serde_json::to_string(&metadata.capture_segments)?;

    let now = sqlite_now(&transaction)?;
    transaction.execute(
        "INSERT INTO ink_operations
         (document_id, seq, command_id, operation_json, operation_fingerprint, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            document_id,
            sqlite_u64(expected_seq, "ink sequence")?,
            operation.command_id(),
            operation_json,
            operation_fingerprint,
            now,
        ],
    )?;
    transaction.execute(
        "UPDATE ink_documents SET version = ?2, title = ?3, created_at = ?4,
         exported_at = ?5, pages_json = ?6, capture_segments_json = ?7,
         head_seq = ?8, updated_at = ?9 WHERE document_id = ?1",
        params![
            document_id,
            metadata.version,
            metadata.title,
            metadata.created_at,
            metadata.exported_at,
            pages_json,
            segments_json,
            sqlite_u64(expected_seq, "ink sequence")?,
            now,
        ],
    )?;
    transaction.execute(
        "UPDATE ink_writer_state SET head_seq = ?2, updated_at = ?3 WHERE document_id = ?1",
        params![document_id, sqlite_u64(expected_seq, "ink sequence")?, now],
    )?;
    transaction.execute(
        "UPDATE ink_sessions SET last_seen_at = ?2 WHERE session_id = ?1",
        params![session_id, now],
    )?;
    let receipt = InkOperationReceipt {
        request_id: request_id.to_owned(),
        document_id: document_id.to_owned(),
        command_id: operation.command_id().to_owned(),
        seq: expected_seq,
        operation_fingerprint,
        durable: true,
        replayed: false,
    };
    transaction.execute(
        "INSERT INTO ink_requests
         (request_id, document_id, request_fingerprint, document_seq, response_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            request_id,
            document_id,
            fingerprint,
            sqlite_u64(expected_seq, "ink sequence")?,
            serde_json::to_string(&receipt)?,
            now,
        ],
    )?;
    transaction.commit()?;
    Ok(receipt)
}

pub(crate) fn open_document(
    connection: &Connection,
    document_id: &str,
) -> Result<Option<StoredInkDocument>, SqliteStoreError> {
    load_document(connection, document_id)
}

pub(crate) fn list_documents(
    connection: &Connection,
) -> Result<Vec<StoredInkDocumentSummary>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT d.document_id, d.title, d.created_at, d.updated_at, d.head_seq,
                (SELECT COUNT(*) FROM ink_sessions s WHERE s.document_id = d.document_id),
                (SELECT s.session_id FROM ink_sessions s
                 WHERE s.document_id = d.document_id AND s.status = 'active'
                 ORDER BY s.last_seen_at DESC, s.session_id DESC LIMIT 1)
         FROM ink_documents d
         ORDER BY d.updated_at DESC, d.document_id DESC",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(
                document_id,
                title,
                created_at,
                updated_at,
                head_seq,
                session_count,
                active_session_id,
            )| {
                Ok(StoredInkDocumentSummary {
                    document_id,
                    title,
                    created_at,
                    updated_at,
                    head_seq: rust_u64(head_seq, "ink head")?,
                    session_count: rust_u64(session_count, "ink session count")?,
                    active_session_id,
                })
            },
        )
        .collect()
}

pub(crate) fn read_session(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<StoredInkSession>, SqliteStoreError> {
    connection
        .query_row(
            "SELECT session_id, document_id, writer_id, writer_epoch, status, device_json,
                    started_at, last_seen_at
             FROM ink_sessions WHERE session_id = ?1",
            [session_id],
            session_from_row,
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn list_sessions(
    connection: &Connection,
    document_id: &str,
) -> Result<Vec<StoredInkSession>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT session_id, document_id, writer_id, writer_epoch, status, device_json,
                started_at, last_seen_at
         FROM ink_sessions WHERE document_id = ?1
         ORDER BY last_seen_at DESC, session_id DESC",
    )?;
    statement
        .query_map([document_id], session_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn load_document<C: InkQueryConnection>(
    connection: &C,
    document_id: &str,
) -> Result<Option<StoredInkDocument>, SqliteStoreError> {
    let connection = connection.connection();
    let Some((
        version,
        title,
        created_at,
        exported_at,
        pages_json,
        segments_json,
        head_seq,
        updated_at,
    )) = connection
        .query_row(
            "SELECT version, title, created_at, exported_at, pages_json,
                    capture_segments_json, head_seq, updated_at
             FROM ink_documents WHERE document_id = ?1",
            [document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT operation_json FROM ink_operations
         WHERE document_id = ?1 ORDER BY seq ASC",
    )?;
    let operations = statement
        .query_map([document_id], |row| row.get::<_, String>(0))?
        .map(|value| value.map_err(SqliteStoreError::from))
        .map(|value| {
            value.and_then(|json| serde_json::from_str(&json).map_err(SqliteStoreError::from))
        })
        .collect::<Result<Vec<InkLabOperation>, _>>()?;
    let document = InkLabDocument {
        version,
        document_id: document_id.to_owned(),
        title,
        created_at,
        exported_at,
        pages: serde_json::from_str(&pages_json)?,
        capture_segments: serde_json::from_str(&segments_json)?,
        operations,
    };
    document
        .validate()
        .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
    Ok(Some(StoredInkDocument {
        document,
        head_seq: rust_u64(head_seq, "ink head")?,
        updated_at,
    }))
}

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredInkSession> {
    let device_json: String = row.get(5)?;
    Ok(StoredInkSession {
        session_id: row.get(0)?,
        document_id: row.get(1)?,
        writer_id: row.get(2)?,
        writer_epoch: rust_u64(row.get(3)?, "writer_epoch").map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        status: row.get(4)?,
        device: serde_json::from_str(&device_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        started_at: row.get(6)?,
        last_seen_at: row.get(7)?,
    })
}

fn request_fingerprint(
    document_id: &str,
    session_id: &str,
    writer_epoch: u64,
    expected_head: u64,
    metadata: &InkLabDocumentMetadata,
    operation: &InkLabOperation,
) -> Result<String, SqliteStoreError> {
    let encoded = serde_json::to_vec(&RequestFingerprint {
        document_id,
        session_id,
        writer_epoch,
        expected_head,
        metadata,
        operation,
    })?;
    Ok(sha256_hex(&encoded))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), SqliteStoreError> {
    if value.trim().is_empty() || value.chars().count() > 200 || value.chars().any(char::is_control)
    {
        return Err(SqliteStoreError::InkInvariant(format!(
            "{field} is invalid"
        )));
    }
    Ok(())
}

fn sqlite_now<C: InkQueryConnection>(connection: &C) -> Result<String, SqliteStoreError> {
    Ok(connection.connection().query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [],
        |row| row.get(0),
    )?)
}

fn checked_next_i64(value: i64, field: &'static str) -> Result<i64, SqliteStoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| SqliteStoreError::InkInvariant(format!("{field} overflowed")))
}

fn checked_next_u64(value: u64, field: &'static str) -> Result<u64, SqliteStoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| SqliteStoreError::InkInvariant(format!("{field} overflowed")))
}

fn sqlite_u64(value: u64, field: &'static str) -> Result<i64, SqliteStoreError> {
    i64::try_from(value).map_err(|_| SqliteStoreError::IntegerRange { field, value })
}

fn rust_u64(value: i64, field: &'static str) -> Result<u64, SqliteStoreError> {
    u64::try_from(value).map_err(|_| SqliteStoreError::InkInvariant(format!("{field} is negative")))
}

fn schema_object_exists(
    connection: &Connection,
    object_kind: &str,
    name: &str,
) -> Result<bool, SqliteStoreError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        params![object_kind, name],
        |row| row.get(0),
    )?;
    Ok(count == 1)
}
