//! S1 dialogue storage inside the session isolation layer
//! (docs/sandbox-flow.md §3.3, §3.4, §3.5, §4).
//!
//! Everything here writes **only** under `sessions/<id>/` — the transcript
//! (`transcript.jsonl`, user / agent / system lines) and the raw
//! `agent_events v0` stream (`events.jsonl`). Discard therefore removes them
//! with the rest of the session; nothing touches the checkpoint.
//!
//! Every append first checks that the session is live, so a discarded
//! session can never be "revived" by a late write from an agent round.
//!
//! **Dialogue threads** (§3.5): a session may hold several dialogue threads
//! that all share the one isolated copy. A new thread only resets what is
//! replayed into the agent prompt — it never resets workspace state. Exactly
//! one thread is active (the only one that accepts new messages); older
//! threads are read-only history. Discard / export still act on the whole
//! session. Lines written before threads existed carry no `thread_id` and
//! belong to [`FIRST_THREAD_ID`].

use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Result, SESSION_FILE, SandboxError, SandboxHome, SessionRecord, SessionView, now_rfc3339,
    truncate_chars, write_json_atomic,
};

pub const EVENTS_FILE: &str = "events.jsonl";
/// One line per round: the harness context it ran with (skill version and
/// hash, permission grants, Lean profile root). No secrets.
pub const HARNESS_FILE: &str = "harness.jsonl";
/// Per-session directory the harness may write (drafts, object cards, notes).
/// Lives beside `workspace/`, so the workspace copy itself stays read-only to
/// harness tools; discard deletes it with the session.
pub const SCRATCH_DIR: &str = "scratch";
/// Current round token for the session's long-lived MCP process
/// (`mathscape mcp --round-file`). Written before every turn.
pub const CURRENT_ROUND_FILE: &str = "current_round";
/// Harness-owned thread state copies, one subdirectory per backend
/// (`harness/codex/rollout-….jsonl`). A cache: the transcript stays the
/// source of truth.
pub const HARNESS_STATE_DIR: &str = "harness";
/// Upper bound of one transcript message (user input or agent reply).
pub const MAX_MESSAGE_CHARS: usize = 20_000;
/// Implicit thread of every session (and of legacy untagged lines).
pub const FIRST_THREAD_ID: &str = "d1";
const THREAD_ID_PREFIX: &str = "d";
/// Safety bound; a debugging session should never need this many threads.
const MAX_THREADS: usize = 200;

/// One dialogue thread inside a sandbox session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueThread {
    pub thread_id: String,
    pub created_at: String,
    /// The harness thread this dialogue continues (docs/codex-session-persistence.md §3).
    /// `None` until the first turn ran on a persistent harness thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessThreadBinding>,
}

/// Which harness thread a dialogue thread is bound to, and what it was
/// bound with. Compared on the next turn to detect tool / skill updates and
/// a moved workspace.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessThreadBinding {
    /// Backend label, e.g. `codex`.
    pub backend: String,
    /// Harness-native thread id (Codex thread UUID).
    pub thread_id: String,
    /// File name of the rollout copy under `harness/<backend>/`, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout_file: Option<String>,
    /// Session workspace copy path the thread was last run against.
    pub workspace: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_sha256: Option<String>,
    /// When the last turn on this thread ended (RFC3339). The next turn's
    /// 「自上轮以来工作区变化」summary counts changes after this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_at: Option<String>,
}

/// Threads of a session in creation order plus the active one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DialogueThreads {
    pub threads: Vec<DialogueThread>,
    pub active_thread_id: String,
}

/// Threads as recorded, with the implicit first thread filled in for
/// sessions created before threads existed.
pub(crate) fn effective_threads(record: &SessionRecord) -> DialogueThreads {
    let threads = if record.dialogue_threads.is_empty() {
        vec![DialogueThread {
            thread_id: FIRST_THREAD_ID.to_owned(),
            created_at: record.created_at.clone(),
            harness: None,
        }]
    } else {
        record.dialogue_threads.clone()
    };
    let active_thread_id = record
        .active_thread_id
        .clone()
        .filter(|id| threads.iter().any(|t| &t.thread_id == id))
        .unwrap_or_else(|| {
            threads
                .last()
                .map(|t| t.thread_id.clone())
                .unwrap_or_default()
        });
    DialogueThreads {
        threads,
        active_thread_id,
    }
}

/// Who said a transcript line (docs/sandbox-flow.md §4.2 role bars).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    User,
    Agent,
    System,
}

/// One `transcript.jsonl` line: user message or agent-visible reply.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub ts: String,
    pub role: TranscriptRole,
    pub text: String,
    /// `agent_events v0` round this line belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_id: Option<String>,
    /// `turn_end.outcome` for agent / system lines that close a round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Dialogue thread; absent on lines written before threads existed
    /// (read as [`FIRST_THREAD_ID`], see [`TranscriptEntry::thread`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl TranscriptEntry {
    /// Effective thread of this line.
    pub fn thread(&self) -> &str {
        self.thread_id.as_deref().unwrap_or(FIRST_THREAD_ID)
    }
}

impl SandboxHome {
    /// View of one live session; errors if it was discarded or never existed.
    pub fn live_session(&self, session_id: &str) -> Result<SessionView> {
        let record = self.session(session_id)?;
        Ok(self.live_view(&record))
    }

    pub fn events_path(&self, session_id: &str) -> std::path::PathBuf {
        self.session_dir(session_id).join(EVENTS_FILE)
    }

    /// Append one transcript line to the session's **active** thread; the
    /// session must be live.
    pub fn append_transcript(
        &self,
        session_id: &str,
        role: TranscriptRole,
        text: &str,
        round_id: Option<&str>,
        outcome: Option<&str>,
    ) -> Result<TranscriptEntry> {
        let active = effective_threads(&self.session(session_id)?).active_thread_id;
        self.append_thread_transcript(session_id, &active, role, text, round_id, outcome)
    }

    /// Append one transcript line to a named thread. Only the active thread
    /// accepts new lines, so a round that was started before a thread switch
    /// can never leak its reply into the new thread (it is refused instead).
    pub fn append_thread_transcript(
        &self,
        session_id: &str,
        thread_id: &str,
        role: TranscriptRole,
        text: &str,
        round_id: Option<&str>,
        outcome: Option<&str>,
    ) -> Result<TranscriptEntry> {
        let threads = effective_threads(&self.session(session_id)?);
        if threads.active_thread_id != thread_id {
            return Err(SandboxError::InactiveThread {
                thread_id: thread_id.to_owned(),
                active_thread_id: threads.active_thread_id,
            });
        }
        let entry = TranscriptEntry {
            ts: now_rfc3339(),
            role,
            text: truncate_chars(text, MAX_MESSAGE_CHARS),
            round_id: round_id.map(str::to_owned),
            outcome: outcome.map(str::to_owned),
            thread_id: Some(thread_id.to_owned()),
        };
        append_json_line(&self.transcript_path(session_id), &entry)?;
        Ok(entry)
    }

    /// All lines of the session, every thread, in write order.
    pub fn read_transcript(&self, session_id: &str) -> Result<Vec<TranscriptEntry>> {
        self.session(session_id)?;
        read_json_lines(&self.transcript_path(session_id))
    }

    /// Lines of one thread, in write order.
    pub fn read_thread_transcript(
        &self,
        session_id: &str,
        thread_id: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        Ok(self
            .read_transcript(session_id)?
            .into_iter()
            .filter(|entry| entry.thread() == thread_id)
            .collect())
    }

    pub fn dialogue_threads(&self, session_id: &str) -> Result<DialogueThreads> {
        Ok(effective_threads(&self.session(session_id)?))
    }

    /// Open a new dialogue thread on the same isolated copy and make it the
    /// active one. Workspace bytes are untouched; only the prompt history
    /// starts empty. Idempotent while the active thread has no user message
    /// yet (returns it unchanged instead of stacking empty threads).
    ///
    /// Callers must not open a thread while a round is streaming; the core
    /// still refuses late lines for the old thread
    /// ([`SandboxHome::append_thread_transcript`]).
    pub fn start_dialogue_thread(&self, session_id: &str) -> Result<DialogueThreads> {
        let mut record = self.session(session_id)?;
        let current = effective_threads(&record);
        let active_has_messages = self
            .read_thread_transcript(session_id, &current.active_thread_id)?
            .iter()
            .any(|entry| entry.role == TranscriptRole::User);
        if !active_has_messages {
            return Ok(current);
        }
        if current.threads.len() >= MAX_THREADS {
            return Err(SandboxError::Invalid(format!(
                "单个沙箱会话最多 {MAX_THREADS} 条对话"
            )));
        }
        let next = current
            .threads
            .iter()
            .filter_map(|t| {
                t.thread_id
                    .strip_prefix(THREAD_ID_PREFIX)?
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .unwrap_or(1)
            + 1;
        let thread = DialogueThread {
            thread_id: format!("{THREAD_ID_PREFIX}{next}"),
            created_at: now_rfc3339(),
            harness: None,
        };
        record.active_thread_id = Some(thread.thread_id.clone());
        record.dialogue_threads = current.threads;
        record.dialogue_threads.push(thread);
        write_json_atomic(&self.session_dir(session_id).join(SESSION_FILE), &record)?;
        Ok(effective_threads(&record))
    }

    /// Record (or replace) the harness thread a dialogue thread is bound to.
    pub fn set_thread_harness(
        &self,
        session_id: &str,
        thread_id: &str,
        binding: Option<HarnessThreadBinding>,
    ) -> Result<DialogueThreads> {
        let mut record = self.session(session_id)?;
        let mut current = effective_threads(&record);
        let Some(thread) = current
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id == thread_id)
        else {
            return Err(SandboxError::Invalid(format!("未知对话：{thread_id}")));
        };
        thread.harness = binding;
        record.dialogue_threads = current.threads;
        if record.active_thread_id.is_none() {
            record.active_thread_id = Some(current.active_thread_id);
        }
        write_json_atomic(&self.session_dir(session_id).join(SESSION_FILE), &record)?;
        Ok(effective_threads(&record))
    }

    /// `<session>/current_round` (see [`CURRENT_ROUND_FILE`]).
    pub fn current_round_path(&self, session_id: &str) -> std::path::PathBuf {
        self.session_dir(session_id).join(CURRENT_ROUND_FILE)
    }

    /// `<session>/harness/<backend>/`, created on demand.
    pub fn ensure_harness_state_dir(
        &self,
        session_id: &str,
        backend: &str,
    ) -> Result<std::path::PathBuf> {
        self.session(session_id)?;
        if backend.is_empty()
            || !backend
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(SandboxError::Invalid(format!("非法 backend：{backend}")));
        }
        let path = self
            .session_dir(session_id)
            .join(HARNESS_STATE_DIR)
            .join(backend);
        std::fs::create_dir_all(&path).map_err(|e| SandboxError::io(&path, e))?;
        Ok(path)
    }

    /// Append one raw `agent_events v0` event (already serialized as JSON).
    pub fn append_event(&self, session_id: &str, event: &Value) -> Result<()> {
        self.session(session_id)?;
        append_json_line(&self.events_path(session_id), event)
    }

    pub fn harness_path(&self, session_id: &str) -> std::path::PathBuf {
        self.session_dir(session_id).join(HARNESS_FILE)
    }

    pub fn append_harness_context(&self, session_id: &str, context: &Value) -> Result<()> {
        self.session(session_id)?;
        append_json_line(&self.harness_path(session_id), context)
    }

    pub fn read_harness_contexts(&self, session_id: &str) -> Result<Vec<Value>> {
        self.session(session_id)?;
        read_json_lines(&self.harness_path(session_id))
    }

    /// The session's harness-writable scratch dir, created on demand.
    pub fn ensure_session_scratch(&self, session_id: &str) -> Result<std::path::PathBuf> {
        self.session(session_id)?;
        let path = self.session_dir(session_id).join(SCRATCH_DIR);
        std::fs::create_dir_all(&path).map_err(|e| SandboxError::io(&path, e))?;
        Ok(path)
    }

    pub fn read_events(&self, session_id: &str) -> Result<Vec<Value>> {
        self.session(session_id)?;
        read_json_lines(&self.events_path(session_id))
    }

    /// Record which backend / tool allowlist the session dialogue uses, so
    /// exports carry it (`manifest.backend`, `manifest.tool_allowlist`).
    pub fn record_backend(
        &self,
        session_id: &str,
        backend: &str,
        tool_allowlist: &[String],
    ) -> Result<SessionView> {
        let mut record = self.session(session_id)?;
        if record.backend != backend || record.tool_allowlist != tool_allowlist {
            record.backend = backend.to_owned();
            record.tool_allowlist = tool_allowlist.to_vec();
            write_json_atomic(&self.session_dir(session_id).join(SESSION_FILE), &record)?;
        }
        Ok(self.live_view(&record))
    }
}

/// Maximum transcript characters replayed into a new round's prompt.
const PROMPT_HISTORY_CHARS: usize = 12_000;

/// Codex rounds are one turn and `ephemeral` (Phase0 附), so the dialogue
/// shell replays recent transcript lines into each new round's user text.
/// Oldest lines are dropped first once [`PROMPT_HISTORY_CHARS`] is exceeded.
///
/// `history` must already be the active thread only. `continues_session`
/// marks a later thread: the copy may hold state from earlier threads whose
/// conversation is deliberately *not* replayed.
pub fn build_dialogue_prompt(
    history: &[TranscriptEntry],
    message: &str,
    continues_session: bool,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut budget = PROMPT_HISTORY_CHARS;
    for entry in history.iter().rev() {
        let who = match entry.role {
            TranscriptRole::User => "用户",
            TranscriptRole::Agent => "助手",
            TranscriptRole::System => continue,
        };
        let line = format!("[{who}] {}", entry.text.trim());
        let cost = line.chars().count();
        if cost > budget {
            break;
        }
        budget -= cost;
        lines.push(line);
    }
    lines.reverse();
    let mut prompt = String::from(
        "你在 Mathscape 沙箱探索会话里与用户对话。可用 mathscape MCP 只读工具查阅当前（沙箱副本）工作区里的材料、图与手稿；引用对象时尽量给出 object_id。\n",
    );
    if continues_session {
        prompt.push_str(
            "这是本沙箱会话里新开的一条对话：工作区可能保留了更早对话留下的状态，但更早的对话内容不在本轮上下文中；以工具查到的当前状态为准，不要假设之前说过什么。\n",
        );
    }
    if !lines.is_empty() {
        prompt.push_str("\n此前的对话（按时间）：\n");
        for line in &lines {
            prompt.push_str(line);
            prompt.push('\n');
        }
    }
    prompt.push_str("\n用户本轮消息：\n");
    prompt.push_str(message.trim());
    prompt
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut line = serde_json::to_string(value).map_err(|source| SandboxError::Json {
        path: path.to_owned(),
        source,
    })?;
    line.push('\n');
    // No `create_dir_all`: the session dir must already exist (live session).
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| SandboxError::io(path, e))?;
    file.write_all(line.as_bytes())
        .map_err(|e| SandboxError::io(path, e))
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(SandboxError::io(path, e)),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| SandboxError::io(path, e))?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(&line).map_err(|source| SandboxError::Json {
                path: path.to_owned(),
                source,
            })?,
        );
    }
    Ok(out)
}
