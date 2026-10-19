//! N2 NoteLink storage: relations from a note to a source object, a Markdown
//! block range or another note (proposal §4.2, §14). Links are note metadata
//! and never touch `map_objects`, `map_object_revisions` or the source graph.

use mathscape_domain::InkLabDocument;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ink::{self, CreateFingerprint, StoredInkSession};
use crate::ink_note::{
    self, INK_NOTE_TITLE_MAX_CHARS, InkLifecycle, InkNoteState, InkTitleMode, now_ms,
};
use crate::{SqliteArtifactStore, SqliteStoreError};

const MIGRATION_SQL: &str = include_str!("../../../migrations/0018_ink_note_links_n2.sql");
/// Maximum length of a target short name (§5.2).
pub const INK_LINK_LABEL_MAX_CHARS: usize = 24;
const EXCERPT_MAX_CHARS: usize = 200;
const MAX_RANGE_BLOCKS: usize = 2000;
const RETIRED_LIFECYCLES: [&str; 3] = ["superseded", "rejected", "deprecated"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkLinkRole {
    Attempt,
    AltProof,
    Check,
    Study,
    Question,
    Reference,
}

impl InkLinkRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attempt => "attempt",
            Self::AltProof => "alt_proof",
            Self::Check => "check",
            Self::Study => "study",
            Self::Question => "question",
            Self::Reference => "reference",
        }
    }

    /// Product label used in titles and lists (§5.1).
    pub fn label(self) -> &'static str {
        match self {
            Self::Attempt => "尝试",
            Self::AltProof => "另证",
            Self::Check => "验算",
            Self::Study => "学习笔记",
            Self::Question => "疑问",
            Self::Reference => "参考",
        }
    }

    fn parse(value: &str) -> Result<Self, SqliteStoreError> {
        Ok(match value {
            "attempt" => Self::Attempt,
            "alt_proof" => Self::AltProof,
            "check" => Self::Check,
            "study" => Self::Study,
            "question" => Self::Question,
            "reference" => Self::Reference,
            _ => {
                return Err(SqliteStoreError::InkInvariant(format!(
                    "unknown link role {value}"
                )));
            }
        })
    }
}

/// A resolved Markdown block range (§14.3). The desktop re-anchors the
/// Markdown and fills every field; the store checks shape and the artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkMarkdownTarget {
    pub artifact_id: String,
    pub content_hash: String,
    pub first_block: String,
    pub last_block: String,
    /// Every block id from `first_block` to `last_block`, in order.
    pub block_ids: Vec<String>,
    /// Byte range of the blocks in the Markdown source.
    pub start: u64,
    pub end: u64,
    /// SHA-256 of the source bytes in `start..end`.
    pub block_hash: String,
    pub excerpt: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InkLinkTarget {
    SourceObject { object_id: String },
    Markdown(InkMarkdownTarget),
    InkNote { document_id: String },
}

impl InkLinkTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SourceObject { .. } => "source_object",
            Self::Markdown(_) => "markdown",
            Self::InkNote { .. } => "ink_note",
        }
    }
}

/// Whether the link target still exists and is current, computed on read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkLinkTargetStatus {
    Active,
    Retired,
    Trashed,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkNoteLink {
    pub link_id: String,
    pub document_id: String,
    pub target_kind: String,
    pub target_key: String,
    pub target_id: String,
    pub target: InkLinkTarget,
    pub target_revision: Option<String>,
    pub target_label: String,
    pub role: InkLinkRole,
    pub is_primary: bool,
    pub state: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub removed_at: Option<i64>,
    pub target_status: InkLinkTargetStatus,
}

/// A link together with the note it belongs to (lists by target).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkLinkedNote {
    pub note: InkNoteState,
    pub link: InkNoteLink,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkLinkRequest {
    pub request_id: String,
    pub document_id: String,
    pub target: InkLinkTarget,
    pub role: InkLinkRole,
    /// Make this the primary link, replacing any current primary.
    #[serde(default)]
    pub primary: bool,
    /// Role change (D12): removes this link in the same transaction; the new
    /// link inherits its primary flag.
    #[serde(default)]
    pub replaces_link_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkUnlinkRequest {
    pub request_id: String,
    pub document_id: String,
    pub link_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkSetPrimaryRequest {
    pub request_id: String,
    pub document_id: String,
    /// `None` clears the primary link.
    pub link_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkLinkReceipt {
    pub request_id: String,
    pub kind: String,
    pub note: InkNoteState,
    pub link: Option<InkNoteLink>,
    /// The (target, role) link was already active and `link` is that link.
    /// Alone this means nothing changed; see `merged` and `primary_changed`.
    #[serde(default)]
    pub existing: bool,
    /// D12 merge: the replaced link was removed and folded into the existing
    /// link with the requested role.
    #[serde(default)]
    pub merged: bool,
    /// The note's primary link changed in this request.
    #[serde(default)]
    pub primary_changed: bool,
    #[serde(default)]
    pub title_changed: bool,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkContextCreateRequest {
    pub request_id: String,
    pub document: InkLabDocument,
    pub session_id: String,
    pub writer_id: String,
    pub device: Value,
    pub target: InkLinkTarget,
    pub role: InkLinkRole,
    /// Empty or absent: automatic title "用途 · 目标短名".
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkContextCreateReceipt {
    #[serde(flatten)]
    pub session: StoredInkSession,
    pub note: InkNoteState,
    pub link: InkNoteLink,
    pub request_id: String,
    #[serde(default)]
    pub replayed: bool,
}

/// Map overlay row: one active link from an active note (§14.7).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkOverlayLink {
    pub document_id: String,
    pub title: String,
    pub last_used_at: i64,
    pub target_kind: String,
    pub target_id: String,
    pub role: InkLinkRole,
    pub is_primary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkOverlayCount {
    pub target_kind: String,
    pub target_id: String,
    /// Distinct active notes linked to the target.
    pub note_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InkLinkOverlay {
    pub counts: Vec<InkOverlayCount>,
    pub links: Vec<InkOverlayLink>,
}

// ---------------------------------------------------------------- labels

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>().trim().to_owned()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_line_marker(line: &str) -> &str {
    let mut line = line.trim_start();
    loop {
        let before = line;
        line = line.trim_start_matches('#').trim_start();
        line = line.trim_start_matches('>').trim_start();
        for marker in ["- ", "* ", "+ "] {
            if let Some(rest) = line.strip_prefix(marker) {
                line = rest.trim_start();
            }
        }
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && let Some(rest) = line[digits..]
                .strip_prefix(". ")
                .or_else(|| line[digits..].strip_prefix(") "))
        {
            line = rest.trim_start();
        }
        if line == before {
            return line;
        }
    }
}

/// Markdown source to readable text: block markers, emphasis and code
/// fences are removed; LaTeX stays as written.
pub fn normalize_markdown_text(source: &str) -> String {
    let lines: Vec<&str> = source
        .lines()
        .map(strip_line_marker)
        .filter(|line| !line.starts_with("```") && !line.starts_with("~~~"))
        .collect();
    let joined = lines
        .join(" ")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "");
    collapse_whitespace(&joined)
}

const NUMBERED_PREFIXES: [&str; 19] = [
    "定理",
    "引理",
    "命题",
    "推论",
    "定义",
    "例题",
    "习题",
    "练习",
    "问题",
    "例",
    "注",
    "Theorem",
    "Lemma",
    "Proposition",
    "Corollary",
    "Definition",
    "Example",
    "Exercise",
    "Problem",
];

fn numbered_label(text: &str) -> Option<String> {
    for prefix in NUMBERED_PREFIXES {
        let Some(rest) = text.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start();
        let number: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '．'))
            .collect();
        let number = number.trim_end_matches(['.', '-', '．']);
        if number.starts_with(|c: char| c.is_ascii_digit()) {
            return Some(truncate_chars(
                &format!("{prefix} {number}"),
                INK_LINK_LABEL_MAX_CHARS,
            ));
        }
    }
    None
}

/// Short name of a Markdown block range (§5.2): the heading text, a leading
/// "定理/例 + 编号", or the first 24 normalized characters.
pub fn markdown_short_label(first_block_kind: &str, source: &str) -> String {
    let text = normalize_markdown_text(source);
    if first_block_kind != "heading"
        && let Some(label) = numbered_label(&text)
    {
        return label;
    }
    let label = truncate_chars(&text, INK_LINK_LABEL_MAX_CHARS);
    if label.is_empty() {
        "Markdown 片段".into()
    } else {
        label
    }
}

/// Readable excerpt of a Markdown block range.
pub fn markdown_excerpt(source: &str) -> String {
    truncate_chars(&normalize_markdown_text(source), EXCERPT_MAX_CHARS)
}

fn unit_kind_name(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "section" => "章节",
        "definition" => "定义",
        "statement" => "命题",
        "proof" => "证明",
        "proof_step" => "证明步骤",
        "problem" => "题目",
        "hint" => "提示",
        "solution" => "解答",
        "example" => "例",
        "remark" => "注记",
        "method" => "方法",
        "notation" => "记号",
        "narrative" => "叙述",
        _ => return None,
    })
}

fn non_empty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(collapse_whitespace)
        .filter(|text| !text.is_empty())
}

/// Short name of a source object (§14.5): source label → display title →
/// source title → type name.
pub fn source_object_short_label(stored_payload: &Value, entity_kind: &str) -> String {
    let payload = stored_payload.get("payload").unwrap_or(stored_payload);
    let label = non_empty(
        payload
            .get("source_labels")
            .and_then(|labels| labels.get(0))
            .and_then(|label| label.get("text")),
    )
    .or_else(|| non_empty(payload.get("display_title")))
    .or_else(|| {
        non_empty(
            payload
                .get("source_title")
                .and_then(|title| title.get("text")),
        )
    })
    .or_else(|| non_empty(payload.get("local_title")))
    .or_else(|| {
        payload
            .get("kind")
            .and_then(Value::as_str)
            .and_then(unit_kind_name)
            .map(str::to_owned)
    })
    .unwrap_or_else(|| match entity_kind {
        "SourceSpan" => "来源范围".into(),
        "SourceSemanticUnit" => "来源单元".into(),
        _ => "来源对象".into(),
    });
    truncate_chars(&label, INK_LINK_LABEL_MAX_CHARS)
}

/// Automatic title of a note with a primary link: "用途 · 目标短名".
pub fn format_link_title(role: InkLinkRole, label: &str) -> String {
    truncate_chars(
        &format!("{} · {}", role.label(), label),
        INK_NOTE_TITLE_MAX_CHARS,
    )
}

// ---------------------------------------------------------------- schema

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    let name: Option<String> = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=18",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_note_links_n2") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 18 is absent or has an unexpected name",
        ));
    }
    connection.prepare(
        "SELECT link_id,document_id,target_kind,target_key,target_id,target_json,target_revision,
                target_label,role,is_primary,state,created_at,removed_at FROM ink_note_links LIMIT 0",
    )?;
    for index in [
        "ink_note_links_active_unique",
        "ink_note_links_one_primary",
        "ink_note_links_by_target",
        "ink_note_links_by_note",
        "ink_note_requests_by_document",
    ] {
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
    let table: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='ink_note_requests'",
        [],
        |row| row.get(0),
    )?;
    if !table.contains("'set_primary'") {
        return Err(SqliteStoreError::InvalidSchema(
            "ink_note_requests lacks the N2 request kinds",
        ));
    }
    Ok(())
}

/// v17 → v18 in one transaction.
pub(crate) fn migrate_v18(connection: &Connection) -> Result<(), SqliteStoreError> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(MIGRATION_SQL)?;
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------- helpers

fn digest(value: &impl Serialize) -> Result<String, SqliteStoreError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
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

fn link_id_for(request_id: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    format!("link-{}", &hash[..32])
}

fn invalid(message: impl Into<String>) -> SqliteStoreError {
    SqliteStoreError::InkLinkTargetInvalid(message.into())
}

struct ResolvedTarget {
    kind: &'static str,
    key: String,
    id: String,
    json: String,
    revision: Option<String>,
    label: String,
}

/// Lifecycle of a source object's current revision, if it exists.
fn source_object_state(
    connection: &Connection,
    object_id: &str,
) -> Result<Option<(String, String, String, String)>, SqliteStoreError> {
    Ok(connection
        .query_row(
            "SELECT o.entity_kind,o.current_revision,r.lifecycle,r.payload_json
             FROM map_objects o JOIN map_object_revisions r
               ON r.object_id=o.object_id AND r.revision_id=o.current_revision
             WHERE o.object_id=?1 AND o.object_shape='entity'",
            [object_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?)
}

fn resolve_target(
    tx: &Connection,
    document_id: Option<&str>,
    target: &InkLinkTarget,
) -> Result<ResolvedTarget, SqliteStoreError> {
    let json = serde_json::to_string(target)?;
    match target {
        InkLinkTarget::SourceObject { object_id } => {
            identifier(object_id, "target.object_id")?;
            let (kind, revision, lifecycle, payload) = source_object_state(tx, object_id)?
                .ok_or_else(|| invalid("来源对象不存在或无法关联。"))?;
            if RETIRED_LIFECYCLES.contains(&lifecycle.as_str()) {
                return Err(SqliteStoreError::InkLinkTargetRetired(
                    "来源对象已退役，不能新建关联。".into(),
                ));
            }
            let payload: Value = serde_json::from_str(&payload)?;
            Ok(ResolvedTarget {
                kind: "source_object",
                key: format!("src:{object_id}"),
                id: object_id.clone(),
                json,
                revision: Some(revision),
                label: source_object_short_label(&payload, &kind),
            })
        }
        InkLinkTarget::Markdown(markdown) => {
            identifier(&markdown.artifact_id, "target.artifact_id")?;
            identifier(&markdown.content_hash, "target.content_hash")?;
            identifier(&markdown.first_block, "target.first_block")?;
            identifier(&markdown.last_block, "target.last_block")?;
            if markdown.block_ids.is_empty()
                || markdown.block_ids.len() > MAX_RANGE_BLOCKS
                || markdown.block_ids.first() != Some(&markdown.first_block)
                || markdown.block_ids.last() != Some(&markdown.last_block)
                || markdown.start >= markdown.end
                || markdown.block_hash.len() != 64
            {
                return Err(invalid("Markdown 段落范围无效，请重新选择。"));
            }
            for block in &markdown.block_ids {
                identifier(block, "target.block_ids")?;
            }
            let label = collapse_whitespace(&markdown.label);
            if label.is_empty() || label.chars().count() > INK_LINK_LABEL_MAX_CHARS {
                return Err(invalid("Markdown 段落短名无效。"));
            }
            if markdown.excerpt.chars().count() > EXCERPT_MAX_CHARS {
                return Err(invalid("Markdown 段落摘要过长。"));
            }
            let stored_hash: Option<String> = tx
                .query_row(
                    "SELECT content_hash FROM artifacts WHERE artifact_id=?1",
                    [&markdown.artifact_id],
                    |row| row.get(0),
                )
                .optional()?;
            match stored_hash {
                None => return Err(invalid("Markdown 文件不存在。")),
                Some(hash) if hash != markdown.content_hash => {
                    return Err(invalid("Markdown 内容已变化，请重新选择。"));
                }
                Some(_) => {}
            }
            Ok(ResolvedTarget {
                kind: "markdown",
                key: format!(
                    "md:{}:{}:{}",
                    markdown.artifact_id, markdown.first_block, markdown.last_block
                ),
                id: markdown.artifact_id.clone(),
                json,
                revision: Some(markdown.content_hash.clone()),
                label,
            })
        }
        InkLinkTarget::InkNote {
            document_id: target_id,
        } => {
            identifier(target_id, "target.document_id")?;
            if Some(target_id.as_str()) == document_id {
                return Err(invalid("手稿不能关联到自己。"));
            }
            let note =
                ink_note::read_note(tx, target_id)?.ok_or_else(|| invalid("目标手稿不存在。"))?;
            if note.lifecycle == InkLifecycle::Trashed {
                return Err(SqliteStoreError::InkLinkTargetRetired(
                    "目标手稿在回收站中，不能新建关联。".into(),
                ));
            }
            Ok(ResolvedTarget {
                kind: "ink_note",
                key: format!("note:{target_id}"),
                id: target_id.clone(),
                json,
                revision: None,
                label: truncate_chars(&note.title, INK_LINK_LABEL_MAX_CHARS),
            })
        }
    }
}

const LINK_COLUMNS: &str = "link_id,document_id,target_kind,target_key,target_id,target_json,
    target_revision,target_label,role,is_primary,state,created_at,removed_at";

type LinkRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    bool,
    String,
    i64,
    Option<i64>,
);

fn link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LinkRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn target_status(
    connection: &Connection,
    kind: &str,
    id: &str,
) -> Result<InkLinkTargetStatus, SqliteStoreError> {
    Ok(match kind {
        "source_object" => match source_object_state(connection, id)? {
            None => InkLinkTargetStatus::Missing,
            Some((_, _, lifecycle, _)) if RETIRED_LIFECYCLES.contains(&lifecycle.as_str()) => {
                InkLinkTargetStatus::Retired
            }
            Some(_) => InkLinkTargetStatus::Active,
        },
        "markdown" => {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifacts WHERE artifact_id=?1)",
                [id],
                |row| row.get(0),
            )?;
            if exists {
                InkLinkTargetStatus::Active
            } else {
                InkLinkTargetStatus::Missing
            }
        }
        _ => match ink_note::read_note(connection, id)? {
            None => InkLinkTargetStatus::Missing,
            Some(note) if note.lifecycle == InkLifecycle::Trashed => InkLinkTargetStatus::Trashed,
            Some(_) => InkLinkTargetStatus::Active,
        },
    })
}

fn build_link(connection: &Connection, row: LinkRow) -> Result<InkNoteLink, SqliteStoreError> {
    let status = target_status(connection, &row.2, &row.4)?;
    Ok(InkNoteLink {
        link_id: row.0,
        document_id: row.1,
        target_kind: row.2,
        target_key: row.3,
        target_id: row.4,
        target: serde_json::from_str(&row.5)?,
        target_revision: row.6,
        target_label: row.7,
        role: InkLinkRole::parse(&row.8)?,
        is_primary: row.9,
        state: row.10,
        created_at: row.11,
        removed_at: row.12,
        target_status: status,
    })
}

fn read_link(
    connection: &Connection,
    link_id: &str,
) -> Result<Option<InkNoteLink>, SqliteStoreError> {
    let row = connection
        .query_row(
            &format!("SELECT {LINK_COLUMNS} FROM ink_note_links WHERE link_id=?1"),
            [link_id],
            link_row,
        )
        .optional()?;
    row.map(|row| build_link(connection, row)).transpose()
}

fn active_link_of(
    connection: &Connection,
    document_id: &str,
    link_id: &str,
) -> Result<InkNoteLink, SqliteStoreError> {
    match read_link(connection, link_id)? {
        Some(link) if link.document_id == document_id && link.state == "active" => Ok(link),
        _ => Err(SqliteStoreError::InkLinkNotFound(link_id.into())),
    }
}

/// Note links, primary first.
pub(crate) fn links_of_note(
    connection: &Connection,
    document_id: &str,
) -> Result<Vec<InkNoteLink>, SqliteStoreError> {
    let rows = {
        let mut query = connection.prepare(&format!(
            "SELECT {LINK_COLUMNS} FROM ink_note_links WHERE document_id=?1 AND state='active'
             ORDER BY is_primary DESC, created_at, link_id"
        ))?;
        query
            .query_map([document_id], link_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    rows.into_iter()
        .map(|row| build_link(connection, row))
        .collect()
}

/// Automatic title for an auto-mode note: from the primary link if any,
/// otherwise from the creation time.
pub(crate) fn auto_title_for(
    connection: &Connection,
    note: &InkNoteState,
) -> Result<Option<String>, SqliteStoreError> {
    let primary: Option<(String, String)> = connection
        .query_row(
            "SELECT role,target_label FROM ink_note_links
             WHERE document_id=?1 AND state='active' AND is_primary=1",
            [&note.document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match primary {
        Some((role, label)) => Some(format_link_title(InkLinkRole::parse(&role)?, &label)),
        None => ink_note::ink_auto_title(&note.created_at),
    })
}

/// Regenerates the title of an auto-mode note after its primary link
/// changed. Manual titles are never touched. Returns whether it changed.
fn retitle_in(tx: &Connection, document_id: &str) -> Result<bool, SqliteStoreError> {
    let note = ink_note::read_note(tx, document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.into()))?;
    if note.title_mode != InkTitleMode::Auto {
        return Ok(false);
    }
    let Some(title) = auto_title_for(tx, &note)? else {
        return Ok(false);
    };
    if title == note.title {
        return Ok(false);
    }
    tx.execute(
        "UPDATE ink_documents SET title=?2,title_revision=title_revision+1 WHERE document_id=?1",
        params![document_id, title],
    )?;
    Ok(true)
}

/// Number of active links that involve this note in either direction.
pub(crate) fn active_link_count(
    connection: &Connection,
    document_id: &str,
) -> Result<u64, SqliteStoreError> {
    let count: i64 = connection.query_row(
        "SELECT (SELECT count(*) FROM ink_note_links WHERE document_id=?1 AND state='active')
              + (SELECT count(*) FROM ink_note_links WHERE target_kind='ink_note' AND target_id=?1 AND state='active')",
        [document_id],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(count).unwrap_or_default())
}

fn replay<T: serde::de::DeserializeOwned>(
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
    document_id: &str,
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

fn clear_primary(tx: &Connection, document_id: &str) -> Result<(), SqliteStoreError> {
    tx.execute(
        "UPDATE ink_note_links SET is_primary=0 WHERE document_id=?1 AND state='active' AND is_primary=1",
        [document_id],
    )?;
    Ok(())
}

fn remove_link(tx: &Connection, link_id: &str, now: i64) -> Result<(), SqliteStoreError> {
    tx.execute(
        "UPDATE ink_note_links SET state='removed',is_primary=0,removed_at=?2
         WHERE link_id=?1 AND state='active'",
        params![link_id, now],
    )?;
    Ok(())
}

fn insert_link(
    tx: &Connection,
    link_id: &str,
    document_id: &str,
    target: &ResolvedTarget,
    role: InkLinkRole,
    primary: bool,
    now: i64,
) -> Result<(), SqliteStoreError> {
    if primary {
        clear_primary(tx, document_id)?;
    }
    tx.execute(
        "INSERT INTO ink_note_links(link_id,document_id,target_kind,target_key,target_id,target_json,
            target_revision,target_label,role,is_primary,state,created_at,removed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'active',?11,NULL)",
        params![
            link_id,
            document_id,
            target.kind,
            target.key,
            target.id,
            target.json,
            target.revision,
            target.label,
            role.as_str(),
            primary,
            now,
        ],
    )?;
    Ok(())
}

fn find_active(
    tx: &Connection,
    document_id: &str,
    key: &str,
    role: InkLinkRole,
) -> Result<Option<String>, SqliteStoreError> {
    Ok(tx
        .query_row(
            "SELECT link_id FROM ink_note_links
             WHERE document_id=?1 AND target_key=?2 AND role=?3 AND state='active'",
            params![document_id, key, role.as_str()],
            |row| row.get(0),
        )
        .optional()?)
}

fn current_note(tx: &Connection, document_id: &str) -> Result<InkNoteState, SqliteStoreError> {
    ink_note::read_note(tx, document_id)?
        .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document_id.into()))
}

// ---------------------------------------------------------------- writes

fn link_in(
    tx: &Transaction<'_>,
    request: &InkLinkRequest,
    now: i64,
) -> Result<InkLinkReceipt, SqliteStoreError> {
    identifier(&request.request_id, "link.request_id")?;
    identifier(&request.document_id, "link.document_id")?;
    let fingerprint = digest(&("link", request))?;
    if let Some(receipt) = replay(tx, &request.request_id, "link", &fingerprint)? {
        return Ok(receipt);
    }
    ink_note::require_active(tx, &request.document_id)?;
    let replaced = request
        .replaces_link_id
        .as_deref()
        .map(|link_id| active_link_of(tx, &request.document_id, link_id))
        .transpose()?;
    let target = resolve_target(tx, Some(&request.document_id), &request.target)?;
    let mut receipt = InkLinkReceipt {
        request_id: request.request_id.clone(),
        kind: "link".into(),
        note: current_note(tx, &request.document_id)?,
        link: None,
        existing: false,
        merged: false,
        primary_changed: false,
        title_changed: false,
        replayed: false,
    };
    let duplicate = find_active(tx, &request.document_id, &target.key, request.role)?;
    match duplicate {
        Some(existing) => {
            // D12: the (target, role) link already exists. A plain duplicate
            // changes nothing; a role change onto it merges (the replaced link
            // is removed and hands over its primary flag); an explicit
            // "设为主关联" promotes the existing link. All in this transaction.
            receipt.existing = true;
            let existing_link = read_link(tx, &existing)?
                .ok_or_else(|| SqliteStoreError::InkLinkNotFound(existing.clone()))?;
            let mut promote = request.primary && !existing_link.is_primary;
            if let Some(old) = replaced.as_ref().filter(|old| old.link_id != existing) {
                promote |= old.is_primary && !existing_link.is_primary;
                remove_link(tx, &old.link_id, now)?;
                receipt.merged = true;
                receipt.primary_changed |= old.is_primary;
            }
            if promote {
                clear_primary(tx, &request.document_id)?;
                tx.execute(
                    "UPDATE ink_note_links SET is_primary=1 WHERE link_id=?1",
                    [&existing],
                )?;
                receipt.primary_changed = true;
            }
            if receipt.primary_changed {
                receipt.title_changed = retitle_in(tx, &request.document_id)?;
            }
            receipt.link = read_link(tx, &existing)?;
            receipt.note = current_note(tx, &request.document_id)?;
        }
        None => {
            let mut primary = request.primary;
            if let Some(old) = &replaced {
                primary |= old.is_primary;
                remove_link(tx, &old.link_id, now)?;
            }
            let link_id = link_id_for(&request.request_id);
            insert_link(
                tx,
                &link_id,
                &request.document_id,
                &target,
                request.role,
                primary,
                now,
            )?;
            receipt.primary_changed = primary;
            if primary {
                receipt.title_changed = retitle_in(tx, &request.document_id)?;
            }
            receipt.link = read_link(tx, &link_id)?;
            receipt.note = current_note(tx, &request.document_id)?;
        }
    }
    record(
        tx,
        &request.request_id,
        "link",
        &request.document_id,
        &fingerprint,
        &receipt,
        now,
    )?;
    Ok(receipt)
}

fn unlink_in(
    tx: &Transaction<'_>,
    request: &InkUnlinkRequest,
    now: i64,
) -> Result<InkLinkReceipt, SqliteStoreError> {
    identifier(&request.request_id, "unlink.request_id")?;
    identifier(&request.document_id, "unlink.document_id")?;
    identifier(&request.link_id, "unlink.link_id")?;
    let fingerprint = digest(&("unlink", request))?;
    if let Some(receipt) = replay(tx, &request.request_id, "unlink", &fingerprint)? {
        return Ok(receipt);
    }
    ink_note::require_active(tx, &request.document_id)?;
    let link = read_link(tx, &request.link_id)?
        .filter(|link| link.document_id == request.document_id)
        .ok_or_else(|| SqliteStoreError::InkLinkNotFound(request.link_id.clone()))?;
    let mut title_changed = false;
    let primary_changed = link.state == "active" && link.is_primary;
    if link.state == "active" {
        remove_link(tx, &link.link_id, now)?;
        if link.is_primary {
            title_changed = retitle_in(tx, &request.document_id)?;
        }
    }
    let receipt = InkLinkReceipt {
        request_id: request.request_id.clone(),
        kind: "unlink".into(),
        note: current_note(tx, &request.document_id)?,
        link: read_link(tx, &link.link_id)?,
        existing: false,
        merged: false,
        primary_changed,
        title_changed,
        replayed: false,
    };
    record(
        tx,
        &request.request_id,
        "unlink",
        &request.document_id,
        &fingerprint,
        &receipt,
        now,
    )?;
    Ok(receipt)
}

fn set_primary_in(
    tx: &Transaction<'_>,
    request: &InkSetPrimaryRequest,
    now: i64,
) -> Result<InkLinkReceipt, SqliteStoreError> {
    identifier(&request.request_id, "set_primary.request_id")?;
    identifier(&request.document_id, "set_primary.document_id")?;
    let fingerprint = digest(&("set_primary", request))?;
    if let Some(receipt) = replay(tx, &request.request_id, "set_primary", &fingerprint)? {
        return Ok(receipt);
    }
    ink_note::require_active(tx, &request.document_id)?;
    let target = request
        .link_id
        .as_deref()
        .map(|link_id| active_link_of(tx, &request.document_id, link_id))
        .transpose()?;
    let current: Option<String> = tx
        .query_row(
            "SELECT link_id FROM ink_note_links WHERE document_id=?1 AND state='active' AND is_primary=1",
            [&request.document_id],
            |row| row.get(0),
        )
        .optional()?;
    let mut title_changed = false;
    let primary_changed = current.as_deref() != request.link_id.as_deref();
    if primary_changed {
        clear_primary(tx, &request.document_id)?;
        if let Some(link) = &target {
            tx.execute(
                "UPDATE ink_note_links SET is_primary=1 WHERE link_id=?1",
                [&link.link_id],
            )?;
        }
        title_changed = retitle_in(tx, &request.document_id)?;
    }
    let receipt = InkLinkReceipt {
        request_id: request.request_id.clone(),
        kind: "set_primary".into(),
        note: current_note(tx, &request.document_id)?,
        link: match &request.link_id {
            Some(link_id) => read_link(tx, link_id)?,
            None => None,
        },
        existing: false,
        merged: false,
        primary_changed,
        title_changed,
        replayed: false,
    };
    record(
        tx,
        &request.request_id,
        "set_primary",
        &request.document_id,
        &fingerprint,
        &receipt,
        now,
    )?;
    Ok(receipt)
}

#[derive(Serialize)]
struct ContextFingerprint<'a> {
    create: CreateFingerprint<'a>,
    target: &'a InkLinkTarget,
    role: InkLinkRole,
    title: &'a str,
}

fn create_from_context(
    connection: &mut Connection,
    request: &InkContextCreateRequest,
) -> Result<InkContextCreateReceipt, SqliteStoreError> {
    identifier(&request.request_id, "create_from_context.request_id")?;
    ink::validate_new_note(&request.document, &request.session_id, &request.writer_id)?;
    let manual = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty());
    if let Some(title) = manual
        && (title.chars().any(char::is_control) || title.chars().count() > INK_NOTE_TITLE_MAX_CHARS)
    {
        return Err(SqliteStoreError::InkTitleInvalid(format!(
            "标题最多 {INK_NOTE_TITLE_MAX_CHARS} 个字符，且不能包含控制字符"
        )));
    }
    let fingerprint = digest(&(
        "create_from_context",
        ContextFingerprint {
            create: CreateFingerprint {
                document: &request.document,
                session_id: &request.session_id,
                writer_id: &request.writer_id,
                device: &request.device,
                title_mode: if manual.is_some() {
                    InkTitleMode::Manual
                } else {
                    InkTitleMode::Auto
                },
            },
            target: &request.target,
            role: request.role,
            title: manual.unwrap_or_default(),
        },
    ))?;
    let now = now_ms();
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(receipt) = replay(
        &tx,
        &request.request_id,
        "create_from_context",
        &fingerprint,
    )? {
        tx.commit()?;
        return Ok(receipt);
    }
    let target = resolve_target(&tx, None, &request.target)?;
    let (title, mode) = match manual {
        Some(title) => (title.to_owned(), InkTitleMode::Manual),
        None => (
            format_link_title(request.role, &target.label),
            InkTitleMode::Auto,
        ),
    };
    let (session, _) = ink::insert_note_in(
        &tx,
        &request.document,
        &request.session_id,
        &request.writer_id,
        &request.device,
        &title,
        mode,
        now,
    )?;
    let document_id = request.document.document_id.as_str();
    let link_id = link_id_for(&request.request_id);
    insert_link(&tx, &link_id, document_id, &target, request.role, true, now)?;
    let receipt = InkContextCreateReceipt {
        session,
        note: current_note(&tx, document_id)?,
        link: read_link(&tx, &link_id)?
            .ok_or_else(|| SqliteStoreError::InkInvariant("new link disappeared".into()))?,
        request_id: request.request_id.clone(),
        replayed: false,
    };
    record(
        &tx,
        &request.request_id,
        "create_from_context",
        document_id,
        &fingerprint,
        &receipt,
        now,
    )?;
    tx.commit()?;
    Ok(receipt)
}

// ---------------------------------------------------------------- reads

fn links_to_target(
    connection: &Connection,
    target_kind: &str,
    target_id: &str,
) -> Result<Vec<InkLinkedNote>, SqliteStoreError> {
    let rows = {
        let mut query = connection.prepare(&format!(
            "SELECT {} FROM ink_note_links l JOIN ink_documents d ON d.document_id=l.document_id
             WHERE l.target_kind=?1 AND l.target_id=?2 AND l.state='active' AND d.lifecycle='active'
             ORDER BY d.last_used_at DESC, l.created_at, l.link_id",
            LINK_COLUMNS
                .split(',')
                .map(|column| format!("l.{}", column.trim()))
                .collect::<Vec<_>>()
                .join(",")
        ))?;
        query
            .query_map(params![target_kind, target_id], link_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    rows.into_iter()
        .map(|row| {
            let link = build_link(connection, row)?;
            let note = current_note(connection, &link.document_id)?;
            Ok(InkLinkedNote { note, link })
        })
        .collect()
}

fn overlay(connection: &Connection) -> Result<InkLinkOverlay, SqliteStoreError> {
    let links = {
        let mut query = connection.prepare(
            "SELECT l.document_id,d.title,d.last_used_at,l.target_kind,l.target_id,l.role,l.is_primary
             FROM ink_note_links l JOIN ink_documents d ON d.document_id=l.document_id
             WHERE l.state='active' AND d.lifecycle='active' AND l.target_kind IN ('source_object','markdown')
             ORDER BY d.last_used_at DESC, l.document_id, l.link_id",
        )?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, bool>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let links = links
        .into_iter()
        .map(|row| {
            Ok(InkOverlayLink {
                document_id: row.0,
                title: row.1,
                last_used_at: row.2,
                target_kind: row.3,
                target_id: row.4,
                role: InkLinkRole::parse(&row.5)?,
                is_primary: row.6,
            })
        })
        .collect::<Result<Vec<_>, SqliteStoreError>>()?;
    let counts = {
        let mut query = connection.prepare(
            "SELECT l.target_kind,l.target_id,count(DISTINCT l.document_id)
             FROM ink_note_links l JOIN ink_documents d ON d.document_id=l.document_id
             WHERE l.state='active' AND d.lifecycle='active' AND l.target_kind IN ('source_object','markdown')
             GROUP BY l.target_kind,l.target_id ORDER BY l.target_kind,l.target_id",
        )?;
        query
            .query_map([], |row| {
                Ok(InkOverlayCount {
                    target_kind: row.get(0)?,
                    target_id: row.get(1)?,
                    note_count: u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(InkLinkOverlay { counts, links })
}

/// Active notes linked to Markdown artifacts this artifact was patched from
/// (§6.4): a document-level hint only, without any text mapping.
fn patch_hint(connection: &Connection, artifact_id: &str) -> Result<u64, SqliteStoreError> {
    let count: i64 = connection.query_row(
        "WITH RECURSIVE ancestors(artifact_id) AS (
             SELECT base_artifact_id FROM markdown_patch_applications
               WHERE patched_artifact_id=?1 AND status='applied'
             UNION
             SELECT a.base_artifact_id FROM markdown_patch_applications a
               JOIN ancestors ON a.patched_artifact_id=ancestors.artifact_id
               WHERE a.status='applied'
         )
         SELECT count(DISTINCT l.document_id) FROM ink_note_links l
           JOIN ink_documents d ON d.document_id=l.document_id
         WHERE l.target_kind='markdown' AND l.state='active' AND d.lifecycle='active'
           AND l.target_id IN (SELECT artifact_id FROM ancestors WHERE artifact_id<>?1)",
        [artifact_id],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(count).unwrap_or_default())
}

impl SqliteArtifactStore {
    /// Creates a note, its primary link and title in one transaction.
    pub fn create_ink_note_from_context(
        &mut self,
        request: &InkContextCreateRequest,
    ) -> Result<InkContextCreateReceipt, SqliteStoreError> {
        create_from_context(&mut self.connection, request)
    }

    pub fn link_ink_note(
        &mut self,
        request: &InkLinkRequest,
    ) -> Result<InkLinkReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = link_in(&tx, request, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn unlink_ink_note(
        &mut self,
        request: &InkUnlinkRequest,
    ) -> Result<InkLinkReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = unlink_in(&tx, request, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn set_ink_note_primary_link(
        &mut self,
        request: &InkSetPrimaryRequest,
    ) -> Result<InkLinkReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = set_primary_in(&tx, request, now_ms())?;
        tx.commit()?;
        Ok(receipt)
    }

    /// Active links of one note, primary first.
    pub fn ink_note_links(&self, document_id: &str) -> Result<Vec<InkNoteLink>, SqliteStoreError> {
        links_of_note(&self.connection, document_id)
    }

    /// Active links from active notes to one target (`source_object`,
    /// `markdown` artifact or `ink_note`).
    pub fn ink_links_to_target(
        &self,
        target_kind: &str,
        target_id: &str,
    ) -> Result<Vec<InkLinkedNote>, SqliteStoreError> {
        links_to_target(&self.connection, target_kind, target_id)
    }

    pub fn ink_link_overlay(&self) -> Result<InkLinkOverlay, SqliteStoreError> {
        overlay(&self.connection)
    }

    pub fn ink_markdown_patch_hint(&self, artifact_id: &str) -> Result<u64, SqliteStoreError> {
        patch_hint(&self.connection, artifact_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_labels_follow_the_short_name_rules() {
        assert_eq!(
            markdown_short_label("heading", "## 1.2 连续函数"),
            "1.2 连续函数"
        );
        assert_eq!(
            markdown_short_label("text", "**定理 3.2** 设 $f$ 连续"),
            "定理 3.2"
        );
        assert_eq!(markdown_short_label("text", "例4. 求极限"), "例 4");
        assert_eq!(markdown_short_label("text", "Lemma 2.1. Let"), "Lemma 2.1");
        assert_eq!(
            markdown_short_label(
                "list",
                "- 设 $f$ 在 $[a,b]$ 上连续，证明 $f$ 有界并且取到最大值"
            ),
            "设 $f$ 在 $[a,b]$ 上连续，证明 $"
        );
        assert_eq!(markdown_short_label("text", "  "), "Markdown 片段");
    }

    #[test]
    fn source_labels_prefer_located_labels() {
        let payload = serde_json::json!({
            "payload": {
                "kind": "statement",
                "display_title": "介值定理",
                "source_labels": [{ "text": "定理 3.2" }],
            }
        });
        assert_eq!(
            source_object_short_label(&payload, "SourceSemanticUnit"),
            "定理 3.2"
        );
        let payload = serde_json::json!({ "kind": "statement", "display_title": "介值定理" });
        assert_eq!(
            source_object_short_label(&payload, "SourceSemanticUnit"),
            "介值定理"
        );
        let payload = serde_json::json!({ "kind": "proof", "source_title": { "text": "证明" } });
        assert_eq!(
            source_object_short_label(&payload, "SourceSemanticUnit"),
            "证明"
        );
        let payload = serde_json::json!({ "kind": "example" });
        assert_eq!(
            source_object_short_label(&payload, "SourceSemanticUnit"),
            "例"
        );
        assert_eq!(
            source_object_short_label(&serde_json::json!({}), "SourceSpan"),
            "来源范围"
        );
        assert_eq!(
            format_link_title(InkLinkRole::AltProof, "定理 3.2"),
            "另证 · 定理 3.2"
        );
    }
}
