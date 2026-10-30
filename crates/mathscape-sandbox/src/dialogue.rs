//! S1 dialogue storage inside the session isolation layer
//! (docs/sandbox-flow.md §3.3, §3.4, §4).
//!
//! Everything here writes **only** under `sessions/<id>/` — the transcript
//! (`transcript.jsonl`, user / agent / system lines) and the raw
//! `agent_events v0` stream (`events.jsonl`). Discard therefore removes them
//! with the rest of the session; nothing touches the checkpoint.
//!
//! Every append first checks that the session is live, so a discarded
//! session can never be "revived" by a late write from an agent round.

use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Result, SESSION_FILE, SandboxError, SandboxHome, SessionView, now_rfc3339, truncate_chars,
    write_json_atomic,
};

pub const EVENTS_FILE: &str = "events.jsonl";
/// Upper bound of one transcript message (user input or agent reply).
pub const MAX_MESSAGE_CHARS: usize = 20_000;

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

    /// Append one transcript line; the session must be live.
    pub fn append_transcript(
        &self,
        session_id: &str,
        role: TranscriptRole,
        text: &str,
        round_id: Option<&str>,
        outcome: Option<&str>,
    ) -> Result<TranscriptEntry> {
        self.session(session_id)?;
        let entry = TranscriptEntry {
            ts: now_rfc3339(),
            role,
            text: truncate_chars(text, MAX_MESSAGE_CHARS),
            round_id: round_id.map(str::to_owned),
            outcome: outcome.map(str::to_owned),
        };
        append_json_line(&self.transcript_path(session_id), &entry)?;
        Ok(entry)
    }

    pub fn read_transcript(&self, session_id: &str) -> Result<Vec<TranscriptEntry>> {
        self.session(session_id)?;
        read_json_lines(&self.transcript_path(session_id))
    }

    /// Append one raw `agent_events v0` event (already serialized as JSON).
    pub fn append_event(&self, session_id: &str, event: &Value) -> Result<()> {
        self.session(session_id)?;
        append_json_line(&self.events_path(session_id), event)
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
pub fn build_dialogue_prompt(history: &[TranscriptEntry], message: &str) -> String {
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
