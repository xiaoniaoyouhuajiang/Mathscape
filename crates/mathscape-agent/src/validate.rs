//! Structural validation of one round's event sequence (contract §3 rules).

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AgentEvent, CallId, EventPayload, RoundId, TurnId, TurnOutcome,
    event::{Cancel, ErrorEvent},
};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ValidationIssue {
    #[error("event seq {seq} belongs to round {actual}, expected {expected}")]
    MixedRound {
        seq: u64,
        expected: RoundId,
        actual: RoundId,
    },
    #[error("seq {seq} does not increase after {previous}")]
    NonMonotonicSeq { seq: u64, previous: u64 },
    #[error("{event_type} at seq {seq} has no turn_id")]
    MissingTurnId { seq: u64, event_type: &'static str },
    #[error("turn {turn_id} started twice (seq {seq})")]
    DuplicateTurnStart { seq: u64, turn_id: TurnId },
    #[error("turn_end for turn {turn_id} without an open turn_start (seq {seq})")]
    UnmatchedTurnEnd { seq: u64, turn_id: TurnId },
    #[error("turn {turn_id} never ended")]
    UnclosedTurn { turn_id: TurnId },
    #[error("tool_call {call_id} issued twice (seq {seq})")]
    DuplicateToolCall { seq: u64, call_id: CallId },
    #[error("tool_result {call_id} without a matching tool_call (seq {seq})")]
    UnmatchedToolResult { seq: u64, call_id: CallId },
    #[error("tool_call {call_id} never got a tool_result")]
    UnansweredToolCall { call_id: CallId },
    #[error("fatal error at seq {error_seq} not followed by turn_end(failed)")]
    FatalWithoutFailedTurnEnd { error_seq: u64 },
    #[error("cancel ack at seq {seq} without a prior cancel request")]
    CancelAckWithoutRequest { seq: u64 },
}

/// Checks a full round trace and returns every issue found (empty = valid).
///
/// Rules: single `round_id`; strictly increasing `seq`; `turn_start` /
/// `turn_end` carry `turn_id` and pair up; `tool_call` / `tool_result` pair by
/// `call_id`; a fatal `error` is followed by `turn_end(failed)` (for its
/// turn, when it names one) before any other `turn_end`; `cancel(ack=true)`
/// follows a `cancel(ack=false)`.
pub fn validate_round(events: &[AgentEvent]) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let Some(first) = events.first() else {
        return issues;
    };
    let mut previous_seq: Option<u64> = None;
    let mut open_turns: BTreeSet<TurnId> = BTreeSet::new();
    let mut seen_turns: BTreeSet<TurnId> = BTreeSet::new();
    let mut open_calls: BTreeMap<CallId, u64> = BTreeMap::new();
    let mut seen_calls: BTreeSet<CallId> = BTreeSet::new();
    // (error seq, turn the error named)
    let mut pending_fatal: Option<(u64, Option<TurnId>)> = None;
    let mut cancel_requested = false;

    for event in events {
        let seq = event.seq;
        if event.round_id != first.round_id {
            issues.push(ValidationIssue::MixedRound {
                seq,
                expected: first.round_id.clone(),
                actual: event.round_id.clone(),
            });
        }
        if let Some(previous) = previous_seq
            && seq <= previous
        {
            issues.push(ValidationIssue::NonMonotonicSeq { seq, previous });
        }
        previous_seq = Some(seq);

        match &event.payload {
            EventPayload::TurnStart(_) => match &event.turn_id {
                None => issues.push(ValidationIssue::MissingTurnId {
                    seq,
                    event_type: "turn_start",
                }),
                Some(turn_id) => {
                    if !seen_turns.insert(turn_id.clone()) {
                        issues.push(ValidationIssue::DuplicateTurnStart {
                            seq,
                            turn_id: turn_id.clone(),
                        });
                    }
                    open_turns.insert(turn_id.clone());
                }
            },
            EventPayload::TurnEnd(end) => {
                if let Some((error_seq, error_turn)) = pending_fatal.take() {
                    let same_turn = error_turn.is_none() || error_turn == event.turn_id;
                    if end.outcome != TurnOutcome::Failed || !same_turn {
                        issues.push(ValidationIssue::FatalWithoutFailedTurnEnd { error_seq });
                    }
                }
                match &event.turn_id {
                    None => issues.push(ValidationIssue::MissingTurnId {
                        seq,
                        event_type: "turn_end",
                    }),
                    Some(turn_id) => {
                        if !open_turns.remove(turn_id) {
                            issues.push(ValidationIssue::UnmatchedTurnEnd {
                                seq,
                                turn_id: turn_id.clone(),
                            });
                        }
                    }
                }
            }
            EventPayload::ToolCall(call) => {
                if !seen_calls.insert(call.call_id.clone()) {
                    issues.push(ValidationIssue::DuplicateToolCall {
                        seq,
                        call_id: call.call_id.clone(),
                    });
                }
                open_calls.insert(call.call_id.clone(), seq);
            }
            EventPayload::ToolResult(result) => {
                if open_calls.remove(&result.call_id).is_none() {
                    issues.push(ValidationIssue::UnmatchedToolResult {
                        seq,
                        call_id: result.call_id.clone(),
                    });
                }
            }
            EventPayload::Usage(_) => {}
            EventPayload::Cancel(Cancel { ack, .. }) => {
                if *ack {
                    if !cancel_requested {
                        issues.push(ValidationIssue::CancelAckWithoutRequest { seq });
                    }
                    cancel_requested = false;
                } else {
                    cancel_requested = true;
                }
            }
            EventPayload::Error(ErrorEvent { fatal, .. }) => {
                if *fatal && pending_fatal.is_none() {
                    pending_fatal = Some((seq, event.turn_id.clone()));
                }
            }
        }
    }

    if let Some((error_seq, _)) = pending_fatal {
        issues.push(ValidationIssue::FatalWithoutFailedTurnEnd { error_seq });
    }
    issues.extend(
        open_turns
            .into_iter()
            .map(|turn_id| ValidationIssue::UnclosedTurn { turn_id }),
    );
    let mut unanswered: Vec<_> = open_calls.into_iter().collect();
    unanswered.sort_by_key(|(_, seq)| *seq);
    issues.extend(
        unanswered
            .into_iter()
            .map(|(call_id, _)| ValidationIssue::UnansweredToolCall { call_id }),
    );
    issues
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;
    use crate::{
        AgentBackend, ErrorCode, ToolResultStatus,
        event::{CancelRequester, ToolCall, ToolResult, TurnEnd, TurnStart},
        scripted_round,
    };

    fn event(seq: u64, turn: Option<&str>, payload: EventPayload) -> AgentEvent {
        AgentEvent {
            round_id: RoundId::parse("r1").unwrap(),
            turn_id: turn.map(|t| TurnId::parse(t).unwrap()),
            seq,
            ts: DateTime::parse_from_rfc3339("2026-10-27T15:00:00+08:00").unwrap(),
            backend: AgentBackend::Custom,
            payload,
        }
    }
    fn start() -> EventPayload {
        EventPayload::TurnStart(TurnStart {
            input_ref: "sha256:in".into(),
            raw: None,
        })
    }
    fn end(outcome: TurnOutcome) -> EventPayload {
        EventPayload::TurnEnd(TurnEnd {
            outcome,
            final_text_ref: None,
            raw: None,
        })
    }
    fn call(id: &str) -> EventPayload {
        EventPayload::ToolCall(ToolCall {
            call_id: CallId::parse(id).unwrap(),
            tool: "workspace.search".into(),
            server: "mathscape".into(),
            args_summary: String::new(),
            raw: None,
        })
    }
    fn result(id: &str) -> EventPayload {
        EventPayload::ToolResult(ToolResult {
            call_id: CallId::parse(id).unwrap(),
            status: ToolResultStatus::Ok,
            duration_ms: 1,
            output_hash: None,
            raw: None,
        })
    }
    fn error(fatal: bool) -> EventPayload {
        EventPayload::Error(ErrorEvent {
            code: ErrorCode::Protocol,
            message: "x".into(),
            fatal,
            raw: None,
        })
    }
    fn cancel(ack: bool) -> EventPayload {
        EventPayload::Cancel(Cancel {
            requested_by: CancelRequester::User,
            ack,
            raw: None,
        })
    }

    #[test]
    fn scripted_round_is_valid() {
        for backend in AgentBackend::ALL {
            let events = scripted_round(&RoundId::parse("r").unwrap(), backend);
            assert_eq!(validate_round(&events), vec![]);
        }
        assert_eq!(validate_round(&[]), vec![]);
    }

    #[test]
    fn turn_pairing() {
        let issues = validate_round(&[
            event(0, Some("t1"), start()),
            event(1, Some("t2"), end(TurnOutcome::Completed)),
            event(2, None, start()),
        ]);
        assert_eq!(
            issues,
            vec![
                ValidationIssue::UnmatchedTurnEnd {
                    seq: 1,
                    turn_id: TurnId::parse("t2").unwrap()
                },
                ValidationIssue::MissingTurnId {
                    seq: 2,
                    event_type: "turn_start"
                },
                ValidationIssue::UnclosedTurn {
                    turn_id: TurnId::parse("t1").unwrap()
                },
            ]
        );
        let issues = validate_round(&[
            event(0, Some("t1"), start()),
            event(1, Some("t1"), end(TurnOutcome::Completed)),
            event(2, Some("t1"), start()),
            event(3, Some("t1"), end(TurnOutcome::Completed)),
        ]);
        assert_eq!(
            issues,
            vec![ValidationIssue::DuplicateTurnStart {
                seq: 2,
                turn_id: TurnId::parse("t1").unwrap()
            }]
        );
    }

    #[test]
    fn tool_pairing_by_call_id() {
        let issues = validate_round(&[
            event(0, Some("t"), start()),
            event(1, Some("t"), call("a")),
            event(2, Some("t"), call("b")),
            event(3, Some("t"), result("b")),
            event(4, Some("t"), result("zzz")),
            event(5, Some("t"), call("b")),
            event(6, Some("t"), result("b")),
            event(7, Some("t"), end(TurnOutcome::Completed)),
        ]);
        assert_eq!(
            issues,
            vec![
                ValidationIssue::UnmatchedToolResult {
                    seq: 4,
                    call_id: CallId::parse("zzz").unwrap()
                },
                ValidationIssue::DuplicateToolCall {
                    seq: 5,
                    call_id: CallId::parse("b").unwrap()
                },
                ValidationIssue::UnansweredToolCall {
                    call_id: CallId::parse("a").unwrap()
                },
            ]
        );
    }

    #[test]
    fn seq_and_round_checks() {
        let mut other = event(1, None, cancel(false));
        other.round_id = RoundId::parse("r2").unwrap();
        let issues = validate_round(&[event(5, None, cancel(false)), other]);
        assert!(issues.contains(&ValidationIssue::NonMonotonicSeq {
            seq: 1,
            previous: 5
        }));
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ValidationIssue::MixedRound { seq: 1, .. }))
        );
    }

    #[test]
    fn fatal_error_requires_failed_turn_end() {
        let ok = validate_round(&[
            event(0, Some("t"), start()),
            event(1, Some("t"), error(true)),
            event(2, Some("t"), end(TurnOutcome::Failed)),
        ]);
        assert_eq!(ok, vec![]);
        let bad = validate_round(&[
            event(0, Some("t"), start()),
            event(1, Some("t"), error(true)),
            event(2, Some("t"), end(TurnOutcome::Completed)),
        ]);
        assert_eq!(
            bad,
            vec![ValidationIssue::FatalWithoutFailedTurnEnd { error_seq: 1 }]
        );
        let dangling = validate_round(&[event(0, None, error(true))]);
        assert_eq!(
            dangling,
            vec![ValidationIssue::FatalWithoutFailedTurnEnd { error_seq: 0 }]
        );
        assert_eq!(validate_round(&[event(0, None, error(false))]), vec![]);
    }

    #[test]
    fn cancel_ack_needs_request() {
        assert_eq!(
            validate_round(&[event(0, None, cancel(true))]),
            vec![ValidationIssue::CancelAckWithoutRequest { seq: 0 }]
        );
        assert_eq!(
            validate_round(&[event(0, None, cancel(false)), event(1, None, cancel(true))]),
            vec![]
        );
    }
}
