//! Decide Codex app-server approval and elicitation requests for a sandbox round.
//!
//! `approvalPolicy: never` does not mean "auto-approve". Codex rejects MCP
//! calls it thinks need a person (`MCP tool call requires approval, but
//! approval policy is never`), and unannotated tools default to destructive.
//! Sandbox rounds therefore use `on-request`, keep `sandbox: read-only`, and
//! mark the mathscape server `default_tools_approval_mode = approve` with an
//! `enabled_tools` whitelist. This module is the client half of that policy:
//! accept whitelisted read-only Mathscape MCP, decline shell / writes, and
//! auto-decline anything we cannot classify so a turn cannot hang.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::ErrorCode;

/// How long an unclassified approval may sit before we decline it.
pub(crate) const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Shown on the dialogue when a held approval is auto-declined.
pub(crate) const APPROVAL_TIMEOUT_MESSAGE: &str = "等待审批已超时";

/// Wire value for `thread/start` `approvalPolicy`.
///
/// This Codex build's serde enum is kebab-case: `untrusted`, `on-request`,
/// `granular`, `never`. `onRequest` is rejected as an unknown variant.
pub(crate) const SANDBOX_APPROVAL_POLICY: &str = "on-request";

/// The ten Mathscape tools. Duplicated so the adapter can whitelist them
/// without depending on `mathscape-mcp`. All are read-only except that
/// `material.read` may run a bounded Marker ensure that adds an immutable
/// parse-cache snapshot (never rewrites or deletes data).
pub(crate) const MATHSCAPE_READ_ONLY_TOOLS: [&str; 10] = [
    "workspace.search",
    "material.read",
    "material.grep",
    "pdf.outline",
    "graph.get",
    "graph.neighborhood",
    "ink.page_image",
    "pdf.page_image",
    "mathlib.search",
    "lean.check",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApprovalNote {
    /// `denied`, `awaiting`, or `timeout`.
    pub phase: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ApprovalReply {
    pub result: Value,
    pub note: Option<ApprovalNote>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ApprovalPlan {
    /// Answer on this turn of the read loop.
    Respond(ApprovalReply),
    /// Park until [`APPROVAL_TIMEOUT`], then decline with [`APPROVAL_TIMEOUT_MESSAGE`].
    Hold { note: ApprovalNote },
}

/// Tools Codex may register for this round. An empty allow-list means the
/// fixed read-only catalog, not "every tool the server might grow".
pub(crate) fn enabled_tools(allow: &[String]) -> Vec<String> {
    if allow.is_empty() {
        MATHSCAPE_READ_ONLY_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    } else {
        allow.to_vec()
    }
}

pub(crate) fn approval_plan(method: &str, params: &Value, allow: &[String]) -> ApprovalPlan {
    if is_shell_approval(method) {
        return ApprovalPlan::Respond(declined(
            json!({ "decision": "decline" }),
            "审批策略拒绝了 shell 命令（沙箱保持只读）",
        ));
    }
    if is_file_approval(method) {
        return ApprovalPlan::Respond(declined(
            json!({ "decision": "decline" }),
            "审批策略拒绝了文件写入（沙箱保持只读）",
        ));
    }
    if method == "item/permissions/requestApproval" {
        return ApprovalPlan::Respond(declined(
            json!({ "permissions": {} }),
            "审批策略拒绝了额外权限（沙箱保持只读）",
        ));
    }
    if method == "item/tool/call" {
        return ApprovalPlan::Respond(declined(
            json!({
                "contentItems": [{
                    "type": "inputText",
                    "text": "沙箱不执行 Codex 动态工具，请使用 mathscape MCP 的只读工具。"
                }],
                "success": false
            }),
            "沙箱不执行 Codex 动态工具，已拒绝",
        ));
    }
    if is_user_input(method) {
        if let Some(result) = accept_whitelisted_prompt(params, allow) {
            return ApprovalPlan::Respond(ApprovalReply { result, note: None });
        }
        return ApprovalPlan::Respond(declined(
            json!({ "answers": {} }),
            "Codex 请求用户输入，沙箱对话尚未实现，已拒绝",
        ));
    }
    if method == "mcpServer/elicitation/request" || method.contains("elicitation") {
        if elicitation_mode(params) == Some("url") {
            return ApprovalPlan::Respond(declined(
                json!({ "action": "decline", "content": null }),
                "拒绝打开外部链接（沙箱不批准 open-world 请求）",
            ));
        }
        if is_mathscape_context(params, allow) {
            return ApprovalPlan::Respond(ApprovalReply {
                result: json!({ "action": "accept", "content": {} }),
                note: None,
            });
        }
        if has_named_foreign_server(params) || has_non_whitelisted_tool(params, allow) {
            return ApprovalPlan::Respond(declined(
                json!({ "action": "decline", "content": null }),
                "未在白名单的 MCP 请求已拒绝",
            ));
        }
        return hold(method, params);
    }
    if method.ends_with("requestApproval") {
        if is_mathscape_context(params, allow) {
            return ApprovalPlan::Respond(ApprovalReply {
                result: json!({ "decision": "accept" }),
                note: None,
            });
        }
        if has_non_whitelisted_tool(params, allow) {
            return ApprovalPlan::Respond(declined(
                json!({ "decision": "decline" }),
                "未在白名单的工具调用已拒绝",
            ));
        }
        return hold(method, params);
    }
    ApprovalPlan::Respond(ApprovalReply {
        result: json!({}),
        note: None,
    })
}

/// Reply after [`APPROVAL_TIMEOUT`]. The note text is exactly
/// [`APPROVAL_TIMEOUT_MESSAGE`] so the dialogue can show it verbatim.
pub(crate) fn timeout_plan(method: &str) -> ApprovalReply {
    let result = if method.contains("elicitation") {
        json!({ "action": "decline", "content": null })
    } else if is_user_input(method) {
        json!({ "answers": {} })
    } else if method == "item/permissions/requestApproval" {
        json!({ "permissions": {} })
    } else if method == "item/tool/call" {
        json!({
            "contentItems": [{
                "type": "inputText",
                "text": APPROVAL_TIMEOUT_MESSAGE
            }],
            "success": false
        })
    } else if method.ends_with("requestApproval")
        || is_shell_approval(method)
        || is_file_approval(method)
    {
        json!({ "decision": "decline" })
    } else {
        json!({})
    };
    ApprovalReply {
        result,
        note: Some(ApprovalNote {
            phase: "timeout",
            message: APPROVAL_TIMEOUT_MESSAGE.to_owned(),
        }),
    }
}

pub(crate) fn approval_event_raw(
    phase: &str,
    method: &str,
    request_id: &Value,
    params: &Value,
) -> Value {
    json!({
        "approval": {
            "phase": phase,
            "method": method,
            "requestId": request_id,
            "itemId": params.get("itemId").cloned().unwrap_or(Value::Null),
            "summary": approval_summary(method, params),
        }
    })
}

/// Turn Codex's "approval policy is never" failure into dialogue text.
/// The code is `protocol` so the UI does not render a bare `internal`.
pub(crate) fn explain_codex_error(code: ErrorCode, message: &str) -> (ErrorCode, String) {
    let lower = message.to_ascii_lowercase();
    let blocked = lower.contains("approval policy")
        || (lower.contains("requires approval")
            && (lower.contains("mcp") || lower.contains("tool") || lower.contains("never")));
    if blocked {
        return (ErrorCode::Protocol, format!("审批被策略拒绝：{message}"));
    }
    (code, message.to_owned())
}

fn declined(result: Value, message: &str) -> ApprovalReply {
    ApprovalReply {
        result,
        note: Some(ApprovalNote {
            phase: "denied",
            message: message.to_owned(),
        }),
    }
}

fn hold(method: &str, params: &Value) -> ApprovalPlan {
    let summary = approval_summary(method, params);
    let message = if summary.is_empty() {
        "等待审批".to_owned()
    } else {
        format!("等待审批：{summary}")
    };
    ApprovalPlan::Hold {
        note: ApprovalNote {
            phase: "awaiting",
            message,
        },
    }
}

fn is_shell_approval(method: &str) -> bool {
    method == "item/commandExecution/requestApproval"
        || method == "commandExecution/requestApproval"
}

fn is_file_approval(method: &str) -> bool {
    method == "item/fileChange/requestApproval" || method == "fileChange/requestApproval"
}

fn is_user_input(method: &str) -> bool {
    matches!(
        method,
        "item/tool/requestUserInput" | "tool/requestUserInput"
    )
}

fn elicitation_mode(params: &Value) -> Option<&str> {
    params.get("mode").and_then(Value::as_str)
}

fn is_mathscape_context(params: &Value, allow: &[String]) -> bool {
    let server = server_name(params);
    if matches!(server, Some(name) if !name.eq_ignore_ascii_case("mathscape")) {
        return false;
    }
    if refers_to_whitelisted_tool(params, allow) {
        return true;
    }
    server == Some("mathscape") && !has_named_tool(params)
}

fn has_named_foreign_server(params: &Value) -> bool {
    matches!(server_name(params), Some(name) if !name.eq_ignore_ascii_case("mathscape"))
}

fn has_non_whitelisted_tool(params: &Value, allow: &[String]) -> bool {
    let names = tool_names(params);
    !names.is_empty() && names.iter().any(|name| !is_allowed(name, allow))
}

fn refers_to_whitelisted_tool(params: &Value, allow: &[String]) -> bool {
    tool_names(params)
        .iter()
        .any(|name| is_allowed(name, allow))
}

fn has_named_tool(params: &Value) -> bool {
    !tool_names(params).is_empty()
}

fn is_allowed(name: &str, allow: &[String]) -> bool {
    let bare = name
        .strip_prefix("mathscape.")
        .or_else(|| name.strip_prefix("mcp__mathscape__"))
        .unwrap_or(name);
    if allow.iter().any(|tool| tool == name || tool == bare) {
        return true;
    }
    allow.is_empty() && MATHSCAPE_READ_ONLY_TOOLS.contains(&bare)
}

fn server_name(params: &Value) -> Option<&str> {
    ["serverName", "server", "mcpServer"]
        .into_iter()
        .find_map(|key| params.get(key).and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .or_else(|| {
            params
                .pointer("/item/server")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
        })
}

fn tool_names(params: &Value) -> Vec<String> {
    let mut names = Vec::new();
    collect_tool_names(params, &mut names);
    names
}

fn collect_tool_names(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "tool" | "toolName" | "tool_name")
                    && let Some(name) = child.as_str().filter(|name| !name.is_empty())
                {
                    out.push(name.to_owned());
                }
                collect_tool_names(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_tool_names(item, out);
            }
        }
        _ => {}
    }
}

fn approval_summary(method: &str, params: &Value) -> String {
    if let Some(command) = params
        .get("command")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return truncate(command, 120);
    }
    if let Some(tool) = tool_names(params).into_iter().next() {
        return tool;
    }
    if let Some(server) = server_name(params) {
        return server.to_owned();
    }
    if let Some(reason) = params
        .get("reason")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return truncate(reason, 120);
    }
    method.to_owned()
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Accept a `requestUserInput` prompt only when every question has an
/// accept-like option and the payload names a whitelisted Mathscape tool.
fn accept_whitelisted_prompt(params: &Value, allow: &[String]) -> Option<Value> {
    if !refers_to_whitelisted_tool(params, allow) {
        return None;
    }
    let questions = params.get("questions")?.as_array()?;
    let mut answers = Map::new();
    for question in questions {
        let id = question.get("id").and_then(Value::as_str)?;
        let option = accept_option_id(question)?;
        answers.insert(id.to_owned(), json!({ "answers": [option] }));
    }
    if answers.is_empty() {
        return None;
    }
    Some(json!({ "answers": Value::Object(answers) }))
}

fn accept_option_id(question: &Value) -> Option<String> {
    let options = question.get("options").and_then(Value::as_array)?;
    for option in options {
        let label = option.get("label").and_then(Value::as_str).unwrap_or("");
        let id = option
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or(label);
        let blob = format!("{label} {id}").to_ascii_lowercase();
        let accepts = blob.contains("accept")
            || blob.contains("approve")
            || label.contains("允许")
            || label.contains("同意");
        let declines = blob.contains("decline")
            || blob.contains("deny")
            || blob.contains("reject")
            || label.contains("拒绝");
        if accepts && !declines && !id.is_empty() {
            return Some(id.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Vec<String> {
        vec!["workspace.search".into(), "material.read".into()]
    }

    fn note(plan: &ApprovalPlan) -> Option<&ApprovalNote> {
        match plan {
            ApprovalPlan::Respond(reply) => reply.note.as_ref(),
            ApprovalPlan::Hold { note } => Some(note),
        }
    }

    #[test]
    fn shell_and_writes_are_declined_without_waiting() {
        let shell = approval_plan(
            "item/commandExecution/requestApproval",
            &json!({ "command": "rm -rf /tmp/x", "itemId": "cmd_1" }),
            &allow(),
        );
        let ApprovalPlan::Respond(reply) = shell else {
            panic!("shell must not be held");
        };
        assert_eq!(reply.result["decision"], "decline");
        assert_eq!(
            reply.note.as_ref().map(|note| note.message.as_str()),
            Some("审批策略拒绝了 shell 命令（沙箱保持只读）")
        );
        let file = approval_plan(
            "item/fileChange/requestApproval",
            &json!({ "itemId": "f1" }),
            &allow(),
        );
        assert!(matches!(file, ApprovalPlan::Respond(_)));
        assert_eq!(
            note(&file).map(|note| note.message.as_str()),
            Some("审批策略拒绝了文件写入（沙箱保持只读）")
        );
        let perms = approval_plan("item/permissions/requestApproval", &json!({}), &allow());
        let ApprovalPlan::Respond(perms) = perms else {
            panic!("permissions");
        };
        assert_eq!(perms.result["permissions"], json!({}));
    }

    #[test]
    fn mathscape_readonly_elicitation_is_accepted() {
        let plan = approval_plan(
            "mcpServer/elicitation/request",
            &json!({
                "serverName": "mathscape",
                "mode": "form",
                "tool": "workspace.search"
            }),
            &allow(),
        );
        let ApprovalPlan::Respond(reply) = plan else {
            panic!("expected accept");
        };
        assert_eq!(reply.result["action"], "accept");
        assert!(reply.note.is_none());
    }

    #[test]
    fn foreign_server_and_url_elicitation_are_declined() {
        let foreign = approval_plan(
            "mcpServer/elicitation/request",
            &json!({ "serverName": "other", "mode": "form" }),
            &allow(),
        );
        assert_eq!(note(&foreign).map(|note| note.phase), Some("denied"));
        let url = approval_plan(
            "mcpServer/elicitation/request",
            &json!({ "serverName": "mathscape", "mode": "url", "url": "https://example" }),
            &allow(),
        );
        let ApprovalPlan::Respond(reply) = url else {
            panic!("url");
        };
        assert_eq!(reply.result["action"], "decline");
    }

    #[test]
    fn whitelisted_prompt_is_accepted_and_freeform_ask_is_declined() {
        let plan = approval_plan(
            "item/tool/requestUserInput",
            &json!({
                "tool": "mathscape.workspace.search",
                "questions": [{
                    "id": "q1",
                    "options": [
                        { "id": "accept", "label": "Accept" },
                        { "id": "decline", "label": "Decline" }
                    ]
                }]
            }),
            &allow(),
        );
        let ApprovalPlan::Respond(reply) = plan else {
            panic!("prompt");
        };
        assert_eq!(reply.result["answers"]["q1"]["answers"][0], "accept");
        assert!(reply.note.is_none());

        let ask = approval_plan(
            "tool/requestUserInput",
            &json!({ "questions": [{ "id": "q", "question": "which theorem?" }] }),
            &allow(),
        );
        assert_eq!(
            note(&ask).map(|note| note.message.as_str()),
            Some("Codex 请求用户输入，沙箱对话尚未实现，已拒绝")
        );
    }

    #[test]
    fn unknown_approval_is_held_then_times_out_visibly() {
        let plan = approval_plan(
            "item/custom/requestApproval",
            &json!({ "itemId": "x", "reason": "unsure" }),
            &allow(),
        );
        let ApprovalPlan::Hold { note } = plan else {
            panic!("expected hold");
        };
        assert_eq!(note.phase, "awaiting");
        assert!(note.message.starts_with("等待审批"));
        let timed = timeout_plan("item/custom/requestApproval");
        assert_eq!(timed.result["decision"], "decline");
        assert_eq!(
            timed.note.as_ref().map(|note| note.message.as_str()),
            Some(APPROVAL_TIMEOUT_MESSAGE)
        );
        assert_eq!(timed.note.as_ref().map(|note| note.phase), Some("timeout"));
    }

    #[test]
    fn empty_allow_list_still_whitelists_the_read_only_catalog() {
        let tools = enabled_tools(&[]);
        assert_eq!(tools.len(), 10);
        assert!(tools.iter().any(|tool| tool == "workspace.search"));
        assert!(tools.iter().any(|tool| tool == "pdf.page_image"));
        assert!(!tools.iter().any(|tool| tool == "shell"));
        let plan = approval_plan(
            "mcpServer/elicitation/request",
            &json!({ "serverName": "mathscape", "mode": "form", "tool": "pdf.outline" }),
            &[],
        );
        assert!(matches!(
            plan,
            ApprovalPlan::Respond(ApprovalReply { note: None, .. })
        ));
    }

    #[test]
    fn approval_policy_never_is_not_reported_as_internal() {
        let (code, message) = explain_codex_error(
            ErrorCode::Internal,
            "MCP tool call requires approval, but approval policy is never",
        );
        assert_eq!(code, ErrorCode::Protocol);
        assert!(message.starts_with("审批被策略拒绝："));
        assert!(message.contains("approval policy is never"));
        let (same, text) = explain_codex_error(ErrorCode::Auth, "not logged in");
        assert_eq!(same, ErrorCode::Auth);
        assert_eq!(text, "not logged in");
    }
}
