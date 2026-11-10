//! Deterministic scripted backend for Desktop UI / trace wiring and tests.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{DateTime, Duration, FixedOffset};
use futures::{future::BoxFuture, stream::BoxStream};

use crate::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, CallId, CapabilityProbe, EventPayload,
    LoadThread, Prompt, RoundId, SessionSpec, ThreadBinding, ThreadContinuity, ToolResultStatus,
    ToolStatus, TurnId, TurnOutcome,
    event::{Cancel, CancelRequester, ToolCall, ToolResult, TurnEnd, TurnStart, Usage},
};

const BASE_TS: &str = "2026-10-27T15:00:00+08:00";

fn base_ts() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(BASE_TS).expect("valid constant timestamp")
}

struct Builder<'a> {
    round_id: &'a RoundId,
    backend: AgentBackend,
    events: Vec<AgentEvent>,
}

impl Builder<'_> {
    fn push(&mut self, turn: Option<&TurnId>, payload: EventPayload) {
        let seq = self.events.len() as u64;
        self.events.push(AgentEvent {
            round_id: self.round_id.clone(),
            turn_id: turn.cloned(),
            seq,
            ts: base_ts() + Duration::milliseconds(seq as i64 * 250),
            backend: self.backend,
            payload,
        });
    }
}

fn start(input_ref: &str) -> EventPayload {
    EventPayload::TurnStart(TurnStart {
        input_ref: input_ref.into(),
        raw: None,
    })
}

fn end(outcome: TurnOutcome, final_text_ref: Option<&str>) -> EventPayload {
    EventPayload::TurnEnd(TurnEnd {
        outcome,
        final_text_ref: final_text_ref.map(str::to_owned),
        raw: None,
    })
}

/// A valid completed round: one turn calling `workspace.search` then
/// `material.read` (truncated), followed by usage.
pub fn scripted_round(round_id: &RoundId, backend: AgentBackend) -> Vec<AgentEvent> {
    scripted_round_with_input(round_id, backend, "sha256:fake-input")
}

fn scripted_round_with_input(
    round_id: &RoundId,
    backend: AgentBackend,
    input_ref: &str,
) -> Vec<AgentEvent> {
    let turn = TurnId::parse("turn-1").expect("valid constant id");
    let mut b = Builder {
        round_id,
        backend,
        events: Vec::new(),
    };
    b.push(Some(&turn), start(input_ref));
    for (index, (tool, args, status)) in [
        ("workspace.search", "query=紧集", ToolStatus::Ok),
        (
            "material.read",
            "locator=md_byte_range[0,40000)",
            ToolStatus::Truncated,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let call_id = CallId::parse(format!("call-{}", index + 1)).expect("valid id");
        b.push(
            Some(&turn),
            EventPayload::ToolCall(ToolCall {
                call_id: call_id.clone(),
                tool: tool.into(),
                server: "mathscape".into(),
                args_summary: args.into(),
                raw: None,
            }),
        );
        b.push(
            Some(&turn),
            EventPayload::ToolResult(ToolResult {
                call_id,
                status: ToolResultStatus::from(status),
                duration_ms: 12,
                output_hash: Some(format!("sha256:fake-output-{}", index + 1)),
                raw: None,
            }),
        );
    }
    b.push(
        Some(&turn),
        EventPayload::Usage(Usage {
            input_tokens: 1200,
            output_tokens: 300,
            cached_input_tokens: Some(800),
            cost_usd: None,
            raw: None,
        }),
    );
    b.push(
        Some(&turn),
        end(TurnOutcome::Completed, Some("sha256:fake-final")),
    );
    b.events
}

/// A valid cancelled round: start, cancel request, ack, `turn_end(cancelled)`.
fn cancelled_round(round_id: &RoundId, backend: AgentBackend, input_ref: &str) -> Vec<AgentEvent> {
    let turn = TurnId::parse("turn-1").expect("valid constant id");
    let mut b = Builder {
        round_id,
        backend,
        events: Vec::new(),
    };
    b.push(Some(&turn), start(input_ref));
    for ack in [false, true] {
        b.push(
            Some(&turn),
            EventPayload::Cancel(Cancel {
                requested_by: CancelRequester::User,
                ack,
                raw: None,
            }),
        );
    }
    b.push(Some(&turn), end(TurnOutcome::Cancelled, None));
    b.events
}

struct FakeRound {
    input_ref: String,
    events: Vec<AgentEvent>,
}

/// Scripted [`AgentExecutor`]. `start_round` registers a round whose stream
/// replays [`scripted_round`]; `cancel` before reading swaps it for a
/// cancelled script. Requires no external capability.
pub struct FakeBackend {
    backend: AgentBackend,
    next_round: AtomicU64,
    next_thread: AtomicU64,
    rounds: Mutex<HashMap<RoundId, FakeRound>>,
    /// session key → known thread ids
    sessions: Mutex<HashMap<String, Vec<String>>>,
}

impl FakeBackend {
    /// Fakes `backend`'s identity on emitted events (use `Custom` by default).
    pub fn new(backend: AgentBackend) -> Self {
        Self {
            backend,
            next_round: AtomicU64::new(1),
            next_thread: AtomicU64::new(1),
            rounds: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new(AgentBackend::Custom)
    }
}

impl AgentExecutor for FakeBackend {
    fn backend(&self) -> AgentBackend {
        self.backend
    }

    fn open_session(&self, spec: SessionSpec) -> BoxFuture<'_, Result<(), AgentError>> {
        Box::pin(async move {
            if spec.backend != self.backend {
                return Err(AgentError::BackendMismatch {
                    served: self.backend,
                    requested: spec.backend,
                });
            }
            self.sessions
                .lock()
                .expect("fake sessions lock")
                .entry(spec.session_key)
                .or_default();
            Ok(())
        })
    }

    fn new_thread(&self, session_key: &str) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            let mut sessions = self.sessions.lock().expect("fake sessions lock");
            let threads = sessions
                .get_mut(&key)
                .ok_or_else(|| AgentError::UnknownSession(key.clone()))?;
            let n = self.next_thread.fetch_add(1, Ordering::Relaxed);
            let thread_id = format!("fake-thread-{n}");
            threads.push(thread_id.clone());
            Ok(ThreadBinding {
                thread_id,
                continuity: ThreadContinuity::Started,
                state_path: None,
            })
        })
    }

    fn load_thread(
        &self,
        session_key: &str,
        load: LoadThread,
    ) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            let mut sessions = self.sessions.lock().expect("fake sessions lock");
            let threads = sessions
                .get_mut(&key)
                .ok_or_else(|| AgentError::UnknownSession(key.clone()))?;
            let continuity = if threads.contains(&load.thread_id) {
                ThreadContinuity::Live
            } else {
                threads.push(load.thread_id.clone());
                ThreadContinuity::Resumed
            };
            Ok(ThreadBinding {
                thread_id: load.thread_id,
                continuity,
                state_path: None,
            })
        })
    }

    fn prompt(
        &self,
        session_key: &str,
        prompt: Prompt,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            if !self
                .sessions
                .lock()
                .expect("fake sessions lock")
                .get(&key)
                .is_some_and(|threads| threads.contains(&prompt.thread_id))
            {
                return Err(AgentError::UnknownSession(key));
            }
            let n = self.next_round.fetch_add(1, Ordering::Relaxed);
            let round_id = RoundId::parse(format!("fake-round-{n}")).expect("valid id");
            let events = scripted_round_with_input(&round_id, self.backend, &prompt.text);
            self.rounds.lock().expect("fake rounds lock").insert(
                round_id.clone(),
                FakeRound {
                    input_ref: prompt.text,
                    events,
                },
            );
            Ok(round_id)
        })
    }

    fn close_session(&self, session_key: &str) -> BoxFuture<'_, Result<(), AgentError>> {
        self.sessions
            .lock()
            .expect("fake sessions lock")
            .remove(session_key);
        Box::pin(async move { Ok(()) })
    }

    fn events(&self, round_id: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError> {
        let rounds = self.rounds.lock().expect("fake rounds lock");
        let round = rounds
            .get(round_id)
            .ok_or_else(|| AgentError::UnknownRound(round_id.clone()))?;
        Ok(Box::pin(futures::stream::iter(round.events.clone())))
    }

    fn cancel(&self, round_id: &RoundId) -> BoxFuture<'_, Result<(), AgentError>> {
        let round_id = round_id.clone();
        Box::pin(async move {
            let mut rounds = self.rounds.lock().expect("fake rounds lock");
            let round = rounds
                .get_mut(&round_id)
                .ok_or_else(|| AgentError::UnknownRound(round_id.clone()))?;
            round.events = cancelled_round(&round_id, self.backend, &round.input_ref);
            Ok(())
        })
    }

    fn probe(&self) -> Vec<CapabilityProbe> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::{StreamExt, executor::block_on};

    use super::*;
    use crate::{read_jsonl, validate_round, write_jsonl};

    fn request(backend: AgentBackend) -> crate::StartRoundRequest {
        crate::StartRoundRequest {
            backend,
            workspace: PathBuf::from("/tmp/ws"),
            input_ref: "sha256:abc".into(),
            tools_allow: vec!["workspace.search".into(), "material.read".into()],
            harness: crate::HarnessContext::default(),
        }
    }

    #[test]
    fn fake_backend_streams_valid_round() {
        let fake: Box<dyn AgentExecutor> = Box::new(FakeBackend::new(AgentBackend::Codex));
        let round = block_on(fake.start_round(request(AgentBackend::Codex))).unwrap();
        let events: Vec<_> = block_on(fake.events(&round).unwrap().collect());
        assert_eq!(validate_round(&events), vec![]);
        assert!(
            events
                .iter()
                .all(|e| e.round_id == round && e.backend == AgentBackend::Codex)
        );
        let types: Vec<_> = events.iter().map(AgentEvent::type_name).collect();
        assert_eq!(
            types,
            [
                "turn_start",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "usage",
                "turn_end"
            ]
        );
        let EventPayload::TurnStart(start) = &events[0].payload else {
            panic!("first event must be turn_start");
        };
        assert_eq!(start.input_ref, "sha256:abc");

        // Trace persistence round trip.
        let mut buffer = Vec::new();
        write_jsonl(&mut buffer, &events).unwrap();
        assert_eq!(read_jsonl(buffer.as_slice()).unwrap(), events);

        let second = block_on(fake.start_round(request(AgentBackend::Codex))).unwrap();
        assert_ne!(round, second);
        assert!(fake.probe().is_empty());
    }

    #[test]
    fn fake_backend_cancel_produces_valid_cancelled_round() {
        let fake = FakeBackend::default();
        let round = block_on(fake.start_round(request(AgentBackend::Custom))).unwrap();
        block_on(fake.cancel(&round)).unwrap();
        let events: Vec<_> = block_on(fake.events(&round).unwrap().collect());
        assert_eq!(validate_round(&events), vec![]);
        let EventPayload::TurnEnd(end) = &events.last().unwrap().payload else {
            panic!("last event must be turn_end");
        };
        assert_eq!(end.outcome, TurnOutcome::Cancelled);
    }

    #[test]
    fn fake_backend_errors() {
        let fake = FakeBackend::default();
        let unknown = RoundId::parse("nope").unwrap();
        assert!(matches!(
            fake.events(&unknown),
            Err(AgentError::UnknownRound(_))
        ));
        assert!(matches!(
            block_on(fake.cancel(&unknown)),
            Err(AgentError::UnknownRound(_))
        ));
        assert_eq!(
            block_on(fake.start_round(request(AgentBackend::Hmz))),
            Err(AgentError::BackendMismatch {
                served: AgentBackend::Custom,
                requested: AgentBackend::Hmz
            })
        );
    }

    #[test]
    fn scripted_timestamps_carry_offset_and_increase() {
        let events = scripted_round(&RoundId::parse("r").unwrap(), AgentBackend::Custom);
        assert!(
            events
                .windows(2)
                .all(|w| w[0].ts < w[1].ts && w[0].seq < w[1].seq)
        );
        let line = crate::to_jsonl_line(&events[0]).unwrap();
        assert!(
            line.contains("\"ts\":\"2026-10-27T15:00:00+08:00\""),
            "{line}"
        );
    }
}
