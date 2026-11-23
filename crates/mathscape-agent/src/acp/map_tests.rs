use super::map::*;
use crate::{AgentBackend, EventPayload, RoundId, TurnOutcome};
use serde_json::json;
use std::collections::HashMap;

fn mapper() -> Mapper {
    Mapper::new(
        RoundId::parse("test").unwrap(),
        AgentBackend::OpenCode,
        "opencode",
        HashMap::from([("mathscape_some_tool_name".into(), "some.tool_name".into())]),
    )
}

#[test]
fn exact_lookup_and_titleless_terminal_preserve_call_identity() {
    let mut mapper = mapper();
    let calls=mapper.update(&json!({"sessionUpdate":"tool_call","toolCallId":"x","title":"mathscape_some_tool_name","kind":"other","rawInput":{"x":1}}));
    assert!(
        matches!(&calls[0].payload,EventPayload::ToolCall(c) if c.tool=="some.tool_name" && c.server=="mathscape")
    );
    let results=mapper.update(&json!({"sessionUpdate":"tool_call_update","toolCallId":"x","status":"completed","rawOutput":{"output":{"status":"blocked"}}}));
    assert!(
        matches!(&results[0].payload,EventPayload::ToolResult(r) if r.status==crate::ToolResultStatus::Blocked && r.raw.as_ref().unwrap()["rawInput"]["x"]==1)
    );
    assert!(
        mapper
            .update(
                &json!({"sessionUpdate":"tool_call_update","toolCallId":"x","status":"completed"})
            )
            .is_empty()
    );
    let unknown=mapper.update(&json!({"sessionUpdate":"tool_call","toolCallId":"y","title":"mathscape_some_tool_name_extra","kind":"other"}));
    assert!(matches!(&unknown[0].payload,EventPayload::ToolCall(c) if c.server=="opencode"));
}

#[test]
fn messages_empty_ends_unrecognized_stop_and_open_calls() {
    let mut mapper = mapper();
    mapper.update(
        &json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}),
    );
    assert_eq!(mapper.text, "hello");
    assert!(
        matches!(mapper.finish(Some(&json!({"stopReason":"end_turn"})),None).last().unwrap().payload,EventPayload::TurnEnd(ref end) if end.outcome==TurnOutcome::Completed)
    );
    let mut mapper = super::map_tests::mapper();
    assert!(
        matches!(mapper.finish(Some(&json!({"stopReason":"max_tokens"})),None).last().unwrap().payload,EventPayload::TurnEnd(ref end) if end.outcome==TurnOutcome::Failed)
    );
    let mut mapper = super::map_tests::mapper();
    mapper.update(&json!({"sessionUpdate":"tool_call","toolCallId":"unfinished","title":"read","kind":"read"}));
    let events = mapper.finish(None, Some("peer exited".into()));
    assert!(
        matches!(events[0].payload,EventPayload::ToolResult(ref result) if result.status==crate::ToolResultStatus::Error)
    );
    assert!(
        mapper
            .update(&json!({"sessionUpdate":"tool_call"}))
            .is_empty()
    );
}
