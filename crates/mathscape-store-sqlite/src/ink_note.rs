//! N1 note metadata for Ink documents: title authority, lifecycle (trash and
//! restore) and the idempotency records of note-level writes.
//!
//! The Ink interchange structs and the operation request fingerprint are left
//! untouched on purpose; old outbox entries keep replaying with their original
//! metadata (including a stale title) and are only stripped of title authority.
use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{SqliteArtifactStore, SqliteStoreError, StoredInkSession, table_has_column};

/// Product limit for user-entered titles (D5). Stored titles may be longer
/// (recovery drafts append a suffix) as long as the 200 UTF-16 contract holds.
pub const INK_NOTE_TITLE_MAX_CHARS: usize = 100;
const MIGRATION_SQL: &str = include_str!("../../../migrations/0017_ink_notes_n1.sql");
const LEGACY_DEFAULT_TITLE: &str = "数学手稿";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkTitleMode {
    Auto,
    Manual,
}
impl InkTitleMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
    fn parse(value: &str) -> Result<Self, SqliteStoreError> {
        match value {
            "auto" => Ok(Self::Auto),
            "manual" => Ok(Self::Manual),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "ink_documents.title_mode",
                value: other.into(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkLifecycle {
    Active,
    Trashed,
}
impl InkLifecycle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Trashed => "trashed",
        }
    }
    fn parse(value: &str) -> Result<Self, SqliteStoreError> {
        match value {
            "active" => Ok(Self::Active),
            "trashed" => Ok(Self::Trashed),
            other => Err(SqliteStoreError::InvalidStoredValue {
                field: "ink_documents.lifecycle",
                value: other.into(),
            }),
        }
    }
}

/// Note metadata kept outside `InkLabDocument`, so neither the operation
/// metadata nor the snapshot manifest change shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkNoteState {
    pub document_id: String,
    pub title: String,
    pub title_mode: InkTitleMode,
    pub title_revision: u64,
    pub lifecycle: InkLifecycle,
    pub lifecycle_revision: u64,
    /// Unix milliseconds.
    pub trashed_at: Option<i64>,
    /// Unix milliseconds of the last content edit.
    pub last_used_at: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkRenameRequest {
    pub request_id: String,
    pub document_id: String,
    /// Empty (after trimming) restores the automatic title.
    pub title: String,
    pub expected_title_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkLifecycleRequest {
    pub request_id: String,
    pub document_id: String,
    pub expected_lifecycle_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkCleanBlankRequest {
    pub request_id: String,
    /// Candidates chosen by the client (it excludes the open document and
    /// documents with local pending writes); each is re-checked here.
    pub document_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkNoteReceipt {
    pub request_id: String,
    pub kind: String,
    pub note: InkNoteState,
    #[serde(default)]
    pub replayed: bool,
    /// Trash only: sessions closed and device credentials revoked.
    #[serde(default)]
    pub closed_sessions: u64,
    #[serde(default)]
    pub revoked_devices: u64,
    /// Restore only: a tablet was paired before and must pair again.
    #[serde(default)]
    pub requires_repairing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkCleanSkip {
    pub document_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkCleanBlankReceipt {
    pub request_id: String,
    pub trashed: Vec<InkNoteState>,
    pub skipped: Vec<InkCleanSkip>,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkCreateReceipt {
    #[serde(flatten)]
    pub session: StoredInkSession,
    pub note: InkNoteState,
    pub request_id: Option<String>,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkTrashImpact {
    pub note: InkNoteState,
    pub head_seq: u64,
    pub page_count: u64,
    pub snapshot_count: u64,
    /// Distinct Ink SourceSpans registered from this document's snapshots.
    pub evidence_count: u64,
    pub remote_session: bool,
    pub device_credentials: u64,
    pub pairing_pending: bool,
    /// Active links from or to this note (N2).
    #[serde(default)]
    pub active_link_count: u64,
    /// Matches the §13.6 blank definition, ignoring local client state.
    pub blank: bool,
}

/// Formats the D3 automatic title from a local wall-clock time.
pub fn format_ink_auto_title(local: NaiveDateTime, current_year: i32) -> String {
    if local.year() == current_year {
        format!(
            "手稿 · {}月{}日 {:02}:{:02}",
            local.month(),
            local.day(),
            local.hour(),
            local.minute()
        )
    } else {
        format!(
            "手稿 · {}年{}月{}日 {:02}:{:02}",
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute()
        )
    }
}

/// Parses the ISO and `unix:N[.frac]` forms used by older Ink columns.
pub fn parse_ink_timestamp_ms(value: &str) -> Option<i64> {
    if let Some(unix) = value.strip_prefix("unix:") {
        let (seconds, fraction) = unix.split_once('.').unwrap_or((unix, ""));
        let seconds: i64 = seconds.parse().ok()?;
        let millis: i64 = if fraction.is_empty() {
            0
        } else {
            let digits: String = fraction.chars().take(3).collect();
            if !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            format!("{digits:0<3}").parse().ok()?
        };
        return seconds.checked_mul(1000)?.checked_add(millis);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}

/// Automatic title of a document created at `created_at`, in the local time
/// zone of this computer. `None` when the timestamp cannot be parsed.
pub fn ink_auto_title(created_at: &str) -> Option<String> {
    let millis = parse_ink_timestamp_ms(created_at)?;
    auto_title_ms(millis, now_ms())
}
fn auto_title_ms(created_ms: i64, now_ms: i64) -> Option<String> {
    let created = Local.timestamp_millis_opt(created_ms).earliest()?;
    let now = Local.timestamp_millis_opt(now_ms).earliest()?;
    Some(format_ink_auto_title(created.naive_local(), now.year()))
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| i64::try_from(value.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn digest(value: &impl Serialize) -> Result<String, SqliteStoreError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn unsigned(value: i64, field: &'static str) -> Result<u64, SqliteStoreError> {
    u64::try_from(value).map_err(|_| SqliteStoreError::InkInvariant(format!("{field} is negative")))
}
fn identifier(value: &str, field: &'static str) -> Result<(), SqliteStoreError> {
    if value.trim().is_empty() || value.chars().count() > 200 || value.chars().any(char::is_control)
    {
        return Err(SqliteStoreError::InkInvariant(format!(
            "{field} is invalid"
        )));
    }
    Ok(())
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name: Option<String> = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=17",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_notes_n1") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 17 is absent or has an unexpected name",
        ));
    }
    for column in [
        "title_mode",
        "title_revision",
        "lifecycle",
        "lifecycle_revision",
        "trashed_at",
        "last_used_at",
    ] {
        if !table_has_column(connection, "ink_documents", column)? {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "column",
                name: column,
            });
        }
    }
    connection.prepare("SELECT request_id,kind,document_id,request_fingerprint,response_json,created_at FROM ink_note_requests LIMIT 0")?;
    for index in ["ink_documents_by_last_used", "ink_documents_by_trashed"] {
        let present: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
            [index],
            |row| row.get(0),
        )?;
        if !present {
            return Err(SqliteStoreError::MissingSchemaObject {
                object_kind: "index",
                name: index,
            });
        }
    }
    Ok(())
}

/// v16 → v17: columns, idempotency table and the D4 backfill in one transaction.
pub(crate) fn migrate_v17(connection: &Connection) -> Result<(), SqliteStoreError> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(MIGRATION_SQL)?;
    backfill_v17(&tx, now_ms())?;
    tx.commit()?;
    Ok(())
}

fn backfill_v17(tx: &Transaction<'_>, now: i64) -> Result<(), SqliteStoreError> {
    let rows = {
        let mut query =
            tx.prepare("SELECT document_id,title,created_at,updated_at FROM ink_documents")?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (document_id, title, created_at, updated_at) in rows {
        let created = parse_ink_timestamp_ms(&created_at);
        let last_used = parse_ink_timestamp_ms(&updated_at).or(created).unwrap_or(0);
        // Only the untouched desktop default becomes automatic (D4). Recovery
        // drafts and any other title stay manual.
        let (title, mode) = match (title == LEGACY_DEFAULT_TITLE, created) {
            (true, Some(ms)) => match auto_title_ms(ms, now) {
                Some(auto) => (auto, InkTitleMode::Auto),
                None => (title, InkTitleMode::Manual),
            },
            _ => (title, InkTitleMode::Manual),
        };
        tx.execute(
            "UPDATE ink_documents SET title=?2,title_mode=?3,title_revision=0,lifecycle='active',
             lifecycle_revision=0,trashed_at=NULL,last_used_at=?4 WHERE document_id=?1",
            params![document_id, title, mode.as_str(), last_used],
        )?;
    }
    Ok(())
}

pub(crate) fn read_note(
    connection: &Connection,
    document_id: &str,
) -> Result<Option<InkNoteState>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT title,title_mode,title_revision,lifecycle,lifecycle_revision,trashed_at,
                    last_used_at,created_at
             FROM ink_documents WHERE document_id=?1",
            [document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        Ok(InkNoteState {
            document_id: document_id.into(),
            title: row.0,
            title_mode: InkTitleMode::parse(&row.1)?,
            title_revision: unsigned(row.2, "title_revision")?,
            lifecycle: InkLifecycle::parse(&row.3)?,
            lifecycle_revision: unsigned(row.4, "lifecycle_revision")?,
            trashed_at: row.5,
            last_used_at: row.6,
            created_at: row.7,
        })
    })
    .transpose()
}

/// Rejects every new write, session and pairing for a trashed document.
pub(crate) fn require_active(
    connection: &Connection,
    document_id: &str,
) -> Result<InkNoteState, SqliteStoreError> {
    let note = read_note(connection, document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.into()))?;
    if note.lifecycle == InkLifecycle::Trashed {
        return Err(SqliteStoreError::InkDocumentTrashed(document_id.into()));
    }
    Ok(note)
}

/// Product title rules (D5). `Ok(None)` means "restore the automatic title".
fn normalize_title(input: &str) -> Result<Option<String>, SqliteStoreError> {
    let title = input.trim();
    if title.is_empty() {
        return Ok(None);
    }
    if title.chars().any(char::is_control) {
        return Err(SqliteStoreError::InkTitleInvalid(
            "标题不能包含控制字符".into(),
        ));
    }
    if title.chars().count() > INK_NOTE_TITLE_MAX_CHARS {
        return Err(SqliteStoreError::InkTitleInvalid(format!(
            "标题最多 {INK_NOTE_TITLE_MAX_CHARS} 个字符"
        )));
    }
    Ok(Some(title.into()))
}

fn replay<T: DeserializeOwned>(
    tx: &Connection,
    request_id: &str,
    kind: &str,
    fingerprint: &str,
) -> Result<Option<T>, SqliteStoreError> {
    let stored: Option<(String, String, String)> = tx
        .query_row(
            "SELECT kind,request_fingerprint,response_json FROM ink_note_requests WHERE request_id=?1",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((stored_kind, stored_fingerprint, response)) = stored else {
        return Ok(None);
    };
    if stored_kind != kind || stored_fingerprint != fingerprint {
        return Err(SqliteStoreError::InkRequestConflict(request_id.into()));
    }
    let mut value: Value = serde_json::from_str(&response)?;
    value["replayed"] = Value::Bool(true);
    Ok(Some(serde_json::from_value(value)?))
}

fn record(
    tx: &Connection,
    request_id: &str,
    kind: &str,
    document_id: Option<&str>,
    fingerprint: &str,
    response: &impl Serialize,
    now: i64,
) -> Result<(), SqliteStoreError> {
    tx.execute(
        "INSERT INTO ink_note_requests(request_id,kind,document_id,request_fingerprint,response_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![request_id, kind, document_id, fingerprint, serde_json::to_string(response)?, now],
    )?;
    Ok(())
}

pub(crate) fn create_fingerprint(value: &impl Serialize) -> Result<String, SqliteStoreError> {
    digest(&("create", value))
}
pub(crate) fn replay_create(
    tx: &Connection,
    request_id: &str,
    fingerprint: &str,
) -> Result<Option<InkCreateReceipt>, SqliteStoreError> {
    replay(tx, request_id, "create", fingerprint)
}
pub(crate) fn record_create(
    tx: &Connection,
    receipt: &InkCreateReceipt,
    fingerprint: &str,
    now: i64,
) -> Result<(), SqliteStoreError> {
    let request_id = receipt
        .request_id
        .as_deref()
        .ok_or_else(|| SqliteStoreError::InkInvariant("create receipt has no request".into()))?;
    record(
        tx,
        request_id,
        "create",
        Some(&receipt.note.document_id),
        fingerprint,
        receipt,
        now,
    )
}

pub(crate) fn rename_in(
    tx: &Transaction<'_>,
    request: &InkRenameRequest,
    now: i64,
) -> Result<InkNoteReceipt, SqliteStoreError> {
    identifier(&request.request_id, "rename.request_id")?;
    identifier(&request.document_id, "rename.document_id")?;
    let fingerprint = digest(&(
        "rename",
        &request.document_id,
        request.title.trim(),
        request.expected_title_revision,
    ))?;
    if let Some(receipt) = replay(tx, &request.request_id, "rename", &fingerprint)? {
        return Ok(receipt);
    }
    let note = require_active(tx, &request.document_id)?;
    if note.title_revision != request.expected_title_revision {
        return Err(SqliteStoreError::InkTitleConflict {
            document_id: note.document_id,
            current_title: note.title,
            current_title_mode: note.title_mode.as_str().into(),
            current_revision: note.title_revision,
        });
    }
    let (title, mode) = match normalize_title(&request.title)? {
        Some(title) => (title, InkTitleMode::Manual),
        // Restoring the automatic title follows the primary link (N2).
        None => match crate::ink_link::auto_title_for(tx, &note)? {
            Some(title) => (title, InkTitleMode::Auto),
            None => {
                return Err(SqliteStoreError::InkTitleInvalid(
                    "无法根据创建时间生成自动标题，请输入标题".into(),
                ));
            }
        },
    };
    if title != note.title || mode != note.title_mode {
        // Rename never touches head, updated_at, last_used_at or the writer (D9).
        tx.execute(
            "UPDATE ink_documents SET title=?2,title_mode=?3,title_revision=title_revision+1
             WHERE document_id=?1",
            params![request.document_id, title, mode.as_str()],
        )?;
    }
    let receipt = InkNoteReceipt {
        request_id: request.request_id.clone(),
        kind: "rename".into(),
        note: read_note(tx, &request.document_id)?
            .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(request.document_id.clone()))?,
        replayed: false,
        closed_sessions: 0,
        revoked_devices: 0,
        requires_repairing: false,
    };
    record(
        tx,
        &request.request_id,
        "rename",
        Some(&request.document_id),
        &fingerprint,
        &receipt,
        now,
    )?;
    Ok(receipt)
}

/// Moves a document to the trash and fences every writer in one transaction:
/// sessions close, the epoch advances, credentials are revoked and unclaimed
/// pairings expire. Returns (closed sessions, revoked devices).
fn trash_core(
    tx: &Transaction<'_>,
    document_id: &str,
    now: i64,
) -> Result<(u64, u64), SqliteStoreError> {
    let seconds = now / 1000;
    let text = format!("unix:{seconds}");
    tx.execute(
        "UPDATE ink_documents SET lifecycle='trashed',trashed_at=?2,lifecycle_revision=lifecycle_revision+1
         WHERE document_id=?1",
        params![document_id, now],
    )?;
    let closed = tx.execute(
        "UPDATE ink_sessions SET status='closed',last_seen_at=?2 WHERE document_id=?1 AND status='active'",
        params![document_id, text],
    )?;
    tx.execute(
        "UPDATE ink_writer_state SET writer_epoch=writer_epoch+1,active_session_id=NULL,writer_id=NULL,updated_at=?2
         WHERE document_id=?1",
        params![document_id, text],
    )?;
    let revoked = tx.execute(
        "UPDATE ink_lan_devices SET revoked_at=?2 WHERE document_id=?1 AND revoked_at IS NULL",
        params![document_id, seconds],
    )?;
    tx.execute(
        "UPDATE ink_pairings SET expires_at=?2 WHERE document_id=?1 AND credential_hash IS NULL AND expires_at>?2",
        params![document_id, seconds],
    )?;
    Ok((closed as u64, revoked as u64))
}

pub(crate) fn trash_in(
    tx: &Transaction<'_>,
    request: &InkLifecycleRequest,
    now: i64,
) -> Result<InkNoteReceipt, SqliteStoreError> {
    lifecycle_in(tx, request, InkLifecycle::Trashed, now)
}

fn lifecycle_in(
    tx: &Transaction<'_>,
    request: &InkLifecycleRequest,
    target: InkLifecycle,
    now: i64,
) -> Result<InkNoteReceipt, SqliteStoreError> {
    let kind = match target {
        InkLifecycle::Trashed => "trash",
        InkLifecycle::Active => "restore",
    };
    identifier(&request.request_id, "lifecycle.request_id")?;
    identifier(&request.document_id, "lifecycle.document_id")?;
    let fingerprint = digest(&(
        kind,
        &request.document_id,
        request.expected_lifecycle_revision,
    ))?;
    if let Some(receipt) = replay(tx, &request.request_id, kind, &fingerprint)? {
        return Ok(receipt);
    }
    let note = read_note(tx, &request.document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(request.document_id.clone()))?;
    let mut receipt = InkNoteReceipt {
        request_id: request.request_id.clone(),
        kind: kind.into(),
        note: note.clone(),
        replayed: false,
        closed_sessions: 0,
        revoked_devices: 0,
        requires_repairing: false,
    };
    // Reaching the target state already is an idempotent success (M9).
    if note.lifecycle != target {
        if note.lifecycle_revision != request.expected_lifecycle_revision {
            return Err(SqliteStoreError::InkLifecycleConflict {
                document_id: note.document_id,
                lifecycle: note.lifecycle.as_str().into(),
                current_revision: note.lifecycle_revision,
            });
        }
        match target {
            InkLifecycle::Trashed => {
                let (closed, revoked) = trash_core(tx, &request.document_id, now)?;
                receipt.closed_sessions = closed;
                receipt.revoked_devices = revoked;
            }
            InkLifecycle::Active => {
                // Restore never revives sessions, credentials or the old epoch.
                tx.execute(
                    "UPDATE ink_documents SET lifecycle='active',trashed_at=NULL,lifecycle_revision=lifecycle_revision+1
                     WHERE document_id=?1",
                    [&request.document_id],
                )?;
            }
        }
        receipt.note = read_note(tx, &request.document_id)?
            .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(request.document_id.clone()))?;
    }
    if target == InkLifecycle::Active {
        receipt.requires_repairing = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ink_lan_devices WHERE document_id=?1)",
            [&request.document_id],
            |row| row.get(0),
        )?;
    }
    record(
        tx,
        &request.request_id,
        kind,
        Some(&request.document_id),
        &fingerprint,
        &receipt,
        now,
    )?;
    Ok(receipt)
}

fn impact(
    connection: &Connection,
    document_id: &str,
    now: i64,
) -> Result<Option<InkTrashImpact>, SqliteStoreError> {
    let Some(note) = read_note(connection, document_id)? else {
        return Ok(None);
    };
    let seconds = now / 1000;
    let (head, pages, snapshots, reservations, evidence, remote, devices, pairing): (
        i64,
        i64,
        i64,
        i64,
        i64,
        bool,
        i64,
        bool,
    ) = connection.query_row(
        "SELECT d.head_seq, json_array_length(d.pages_json),
            (SELECT count(*) FROM ink_snapshots s WHERE s.document_id=d.document_id),
            (SELECT count(*) FROM ink_snapshot_requests r WHERE r.document_id=d.document_id AND r.snapshot_id IS NULL),
            (SELECT count(DISTINCT e.span_id) FROM ink_evidence_requests e JOIN ink_snapshots s ON e.snapshot_id=s.snapshot_id WHERE s.document_id=d.document_id),
            EXISTS(SELECT 1 FROM ink_sessions s WHERE s.document_id=d.document_id AND s.status='active' AND s.writer_id<>'desktop-local'),
            (SELECT count(*) FROM ink_lan_devices v WHERE v.document_id=d.document_id AND v.revoked_at IS NULL AND v.expires_at>?2),
            EXISTS(SELECT 1 FROM ink_pairings p WHERE p.document_id=d.document_id AND p.credential_hash IS NULL AND p.expires_at>?2)
         FROM ink_documents d WHERE d.document_id=?1",
        params![document_id, seconds],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    let links = crate::ink_link::active_link_count(connection, document_id)?;
    let blank = note.lifecycle == InkLifecycle::Active
        && head == 0
        && links == 0
        && note.title_mode == InkTitleMode::Auto
        && snapshots == 0
        && reservations == 0
        && !remote
        && devices == 0
        && !pairing;
    Ok(Some(InkTrashImpact {
        note,
        head_seq: unsigned(head, "head_seq")?,
        page_count: unsigned(pages, "page_count")?,
        snapshot_count: unsigned(snapshots + reservations, "snapshot_count")?,
        evidence_count: unsigned(evidence, "evidence_count")?,
        remote_session: remote,
        device_credentials: unsigned(devices, "device_credentials")?,
        pairing_pending: pairing,
        active_link_count: links,
        blank,
    }))
}

fn blank_skip_reason(impact: &InkTrashImpact) -> Option<&'static str> {
    if impact.note.lifecycle == InkLifecycle::Trashed {
        Some("trashed")
    } else if impact.head_seq > 0 {
        Some("has_ink")
    } else if impact.note.title_mode == InkTitleMode::Manual {
        Some("manual_title")
    } else if impact.snapshot_count > 0 {
        Some("has_snapshot")
    } else if impact.remote_session || impact.device_credentials > 0 {
        Some("device_session")
    } else if impact.pairing_pending {
        Some("pairing_pending")
    } else if impact.active_link_count > 0 {
        Some("has_links")
    } else {
        None
    }
}

impl SqliteArtifactStore {
    pub fn ink_note(&self, document_id: &str) -> Result<Option<InkNoteState>, SqliteStoreError> {
        read_note(&self.connection, document_id)
    }

    pub fn ink_trash_impact(
        &self,
        document_id: &str,
    ) -> Result<Option<InkTrashImpact>, SqliteStoreError> {
        impact(&self.connection, document_id, now_ms())
    }

    pub fn rename_ink_note(
        &mut self,
        request: &InkRenameRequest,
    ) -> Result<InkNoteReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = rename_in(&tx, request, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn trash_ink_note(
        &mut self,
        request: &InkLifecycleRequest,
    ) -> Result<InkNoteReceipt, SqliteStoreError> {
        self.trash_ink_note_at(request, now_ms())
    }

    /// `now` is Unix milliseconds; exposed for deterministic tests.
    pub fn trash_ink_note_at(
        &mut self,
        request: &InkLifecycleRequest,
        now: i64,
    ) -> Result<InkNoteReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = trash_in(&tx, request, now)?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn restore_ink_note(
        &mut self,
        request: &InkLifecycleRequest,
    ) -> Result<InkNoteReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = lifecycle_in(&tx, request, InkLifecycle::Active, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    /// Trashes the blank candidates in one transaction and reports the rest.
    pub fn clean_blank_ink_notes(
        &mut self,
        request: &InkCleanBlankRequest,
    ) -> Result<InkCleanBlankReceipt, SqliteStoreError> {
        identifier(&request.request_id, "clean.request_id")?;
        if request.document_ids.len() > 1000 {
            return Err(SqliteStoreError::InkInvariant(
                "too many clean candidates".into(),
            ));
        }
        let mut candidates = request.document_ids.clone();
        candidates.sort();
        candidates.dedup();
        let fingerprint = digest(&("clean_blank", &candidates))?;
        let now = now_ms();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = replay(&tx, &request.request_id, "clean_blank", &fingerprint)? {
            tx.commit()?;
            return Ok(receipt);
        }
        let mut receipt = InkCleanBlankReceipt {
            request_id: request.request_id.clone(),
            trashed: vec![],
            skipped: vec![],
            replayed: false,
        };
        for document_id in candidates {
            identifier(&document_id, "clean.document_id")?;
            let reason = match impact(&tx, &document_id, now)? {
                None => Some("not_found"),
                Some(impact) => blank_skip_reason(&impact),
            };
            if let Some(reason) = reason {
                receipt.skipped.push(InkCleanSkip {
                    document_id,
                    reason: reason.into(),
                });
                continue;
            }
            trash_core(&tx, &document_id, now)?;
            receipt.trashed.push(
                read_note(&tx, &document_id)?
                    .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.clone()))?,
            );
        }
        record(
            &tx,
            &request.request_id,
            "clean_blank",
            None,
            &fingerprint,
            &receipt,
            now,
        )?;
        tx.commit()?;
        Ok(receipt)
    }
}
