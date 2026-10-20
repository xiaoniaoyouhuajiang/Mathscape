//! N3b: problem card for the currently authorized note's primary link.
//! Kept within §10: only the active note's primary-link excerpt is exposed.

use mathscape_store_sqlite::{InkLinkTarget, InkNoteLink};
use serde::Serialize;
use serde_json::Value;

/// Cap for the card body (Unicode scalar values). Truncation adds an indicator.
pub const PROBLEM_CARD_BODY_MAX: usize = 4000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCardStatus {
    Ok,
    Stale,
    Missing,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCardBodyFormat {
    Markdown,
    Text,
}

#[derive(Clone, Debug, Serialize)]
pub struct InkProblemCard {
    pub link_id: String,
    pub role: String,
    pub target_kind: String,
    pub target_id: String,
    pub target_label: String,
    pub body: String,
    pub body_format: ProblemCardBodyFormat,
    pub body_truncated: bool,
    pub status: ProblemCardStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl InkProblemCard {
    pub fn wire_push(document_id: &str, card: &Self) -> Value {
        serde_json::json!({
            "type": "problem_card",
            "document_id": document_id,
            "card": card,
        })
    }

    pub fn wire_clear(document_id: &str) -> Value {
        serde_json::json!({
            "type": "problem_card_clear",
            "document_id": document_id,
        })
    }
}

/// Truncate to `max` chars; returns `(text, truncated)`.
pub fn truncate_body(text: &str, max: usize) -> (String, bool) {
    let count = text.chars().count();
    if count <= max {
        return (text.to_owned(), false);
    }
    let truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    (format!("{truncated}…"), true)
}

/// Preserve paragraphs, emphasis and math delimiters for the shared renderer.
pub fn markdown_body_from_source(source: &str) -> String {
    source.trim().to_owned()
}

pub fn role_wire(link: &InkNoteLink) -> String {
    link.role.as_str().into()
}

pub fn finalize_card(
    link: &InkNoteLink,
    body: String,
    status: ProblemCardStatus,
    content_hash: Option<String>,
) -> InkProblemCard {
    let (body, body_truncated) = truncate_body(&body, PROBLEM_CARD_BODY_MAX);
    InkProblemCard {
        link_id: link.link_id.clone(),
        role: role_wire(link),
        target_kind: link.target_kind.clone(),
        target_id: link.target_id.clone(),
        target_label: link.target_label.clone(),
        body,
        body_format: if matches!(&link.target, InkLinkTarget::InkNote { .. }) {
            ProblemCardBodyFormat::Text
        } else {
            ProblemCardBodyFormat::Markdown
        },
        body_truncated,
        status,
        content_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_bodies_with_an_ellipsis() {
        let long: String = "题".repeat(PROBLEM_CARD_BODY_MAX + 10);
        let (text, truncated) = truncate_body(&long, PROBLEM_CARD_BODY_MAX);
        assert!(truncated);
        assert_eq!(text.chars().count(), PROBLEM_CARD_BODY_MAX);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn markdown_body_keeps_latex_source() {
        let source = "## 习题\n\n求 $\\int_0^1 x^2\\,dx$。\n";
        let body = markdown_body_from_source(source);
        assert!(body.contains("\\int_0^1"));
        assert_eq!(body, source.trim());
    }
    #[test]
    fn wire_messages_name_the_document() {
        let card = InkProblemCard {
            link_id: "l1".into(),
            role: "attempt".into(),
            target_kind: "ink_note".into(),
            target_id: "doc-b".into(),
            target_label: "目标".into(),
            body: "关联手稿：目标".into(),
            body_format: ProblemCardBodyFormat::Text,
            body_truncated: false,
            status: ProblemCardStatus::Ok,
            content_hash: None,
        };
        let push = InkProblemCard::wire_push("doc-a", &card);
        assert_eq!(push["type"], "problem_card");
        assert_eq!(push["document_id"], "doc-a");
        assert_eq!(push["card"]["body"], "关联手稿：目标");
        let clear = InkProblemCard::wire_clear("doc-other");
        assert_eq!(clear["type"], "problem_card_clear");
        assert_eq!(clear["document_id"], "doc-other");
    }
}
