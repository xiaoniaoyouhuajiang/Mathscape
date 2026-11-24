//! `user.ask`: non-blocking Phase2 clarification question tool (`docs/phase2-write-contract.md` §4.1).
//!
//! Persists a `pending` row in `pending_questions` bound to the active
//! `(investigation_id, round_id)` and immediately returns `next: "end_turn"` so
//! the Agent ends its turn without holding a live RPC or write slot. Within the
//! same round, a second distinct `user.ask` or any subsequent `propose_changes`
//! is rejected with `forbidden` (`reason: "awaiting_user"`).

use std::collections::BTreeSet;

use chrono::Utc;
use mathscape_agent::{ToolError, ToolErrorCode};
use mathscape_store_sqlite::{CreatePendingQuestionInput, SqliteStoreError};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::USER_ASK;
use crate::McpWorkspace;

pub const MAX_QUESTION_CHARS: usize = 4_000;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 8;
pub const MAX_OPTION_CHARS: usize = 240;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    question: String,
    #[serde(default)]
    options: Option<Vec<String>>,
    #[serde(default)]
    ink_requested: bool,
}

pub(super) fn definition() -> Value {
    json!({
        "name": USER_ASK,
        "description": "Ask the user a focused clarification question and pause this turn without blocking. Use this instead of guessing when a source passage has an unresolved referent or chapter ambiguity, when choosing between keeping an item at the source layer vs promoting a shared SemanticEntity (e.g. problem_schema), when cleanup would retire or reconnect existing objects beyond the user's stated scope, or when a scanned page or handwritten symbol cannot be verified from material.read plus pdf.page_image / ink.page_image. Returns {status:\"ok\", question_id, state:\"pending\", next:\"end_turn\"} immediately: stop calling write tools and end your turn so the user can answer; calling user.ask again with another question or calling propose_changes in the same round returns forbidden (awaiting_user). Provide 2–8 concise mutually-exclusive options when the choice is discrete; set ink_requested true only when you need the user to attach an ink snapshot. Requires a bound round token.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_QUESTION_CHARS,
                    "description": "Markdown/LaTeX clarification question shown to the user."
                },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_OPTION_CHARS
                    },
                    "minItems": MIN_OPTIONS,
                    "maxItems": MAX_OPTIONS,
                    "description": "Optional discrete choices presented as clickable options (user may still add free-text notes)."
                },
                "ink_requested": {
                    "type": "boolean",
                    "description": "Set true when asking the user to attach or select a handwritten ink snapshot."
                }
            },
            "required": ["question"],
            "additionalProperties": false
        }
    })
}

pub(super) fn annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: AskArgs = super::parse_args(arguments)?;
    let round = super::propose::required_round_for(workspace, USER_ASK)?;
    let question = validate_question(&args.question)?;
    let options = validate_options(args.options.as_deref())?;

    let mut store = workspace.store()?;
    let investigation_id = super::propose::bind_investigation(workspace, &mut store, &round)?;
    let question_id = deterministic_question_id(
        &investigation_id,
        &round,
        &question,
        options.as_deref(),
        args.ink_requested,
    );

    let created = store
        .create_pending_question(&CreatePendingQuestionInput {
            question_id,
            investigation_id,
            round_id: round,
            origin: "mcp_user_ask".to_owned(),
            harness_ref: None,
            question,
            options,
            ink_requested: args.ink_requested,
            created_at: Utc::now().to_rfc3339(),
        })
        .map_err(|error| match error {
            SqliteStoreError::PendingQuestionConflict(message)
                if message.starts_with("awaiting_user:") =>
            {
                ToolError::new(ToolErrorCode::Forbidden, message)
                    .with_details(json!({ "reason": "awaiting_user" }))
            }
            SqliteStoreError::PendingQuestionConflict(message) => {
                ToolError::new(ToolErrorCode::InvalidArg, message)
                    .with_details(json!({ "reason": "idempotency_conflict" }))
            }
            other => ToolError::new(ToolErrorCode::Internal, format!("sqlite failure: {other}")),
        })?;

    Ok(json!({
        "status": "ok",
        "question_id": created.question_id,
        "state": created.state,
        "next": "end_turn",
    }))
}

fn validate_question(raw: &str) -> Result<String, ToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_QUESTION_CHARS
        || trimmed
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "question must be non-empty, at most {MAX_QUESTION_CHARS} characters, and free of control characters"
            ),
        ));
    }
    Ok(trimmed.to_owned())
}

fn validate_options(raw: Option<&[String]>) -> Result<Option<Vec<String>>, ToolError> {
    let Some(items) = raw else {
        return Ok(None);
    };
    if items.len() < MIN_OPTIONS || items.len() > MAX_OPTIONS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("options must contain between {MIN_OPTIONS} and {MAX_OPTIONS} items"),
        ));
    }
    let mut normalized = Vec::with_capacity(items.len());
    let mut seen = BTreeSet::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty()
            || trimmed.chars().count() > MAX_OPTION_CHARS
            || trimmed.chars().any(char::is_control)
        {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                format!(
                    "each option must be non-empty, at most {MAX_OPTION_CHARS} characters, and single-line"
                ),
            ));
        }
        if !seen.insert(trimmed.to_owned()) {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "options must be distinct",
            ));
        }
        normalized.push(trimmed.to_owned());
    }
    Ok(Some(normalized))
}

fn deterministic_question_id(
    investigation_id: &str,
    round_id: &str,
    question: &str,
    options: Option<&[String]>,
    ink_requested: bool,
) -> String {
    let bytes = serde_json::to_vec(&json!([
        "mathscape-user-ask-1",
        investigation_id,
        round_id,
        question,
        options,
        ink_requested,
    ]))
    .expect("question fingerprint serializes");
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("mathscape:question/{hex}")
}
