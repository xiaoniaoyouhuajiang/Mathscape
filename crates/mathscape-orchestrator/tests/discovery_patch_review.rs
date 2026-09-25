use mathscape_domain::*;
use mathscape_llm::*;
use mathscape_orchestrator::*;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

const SOURCE: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-source.md");
const UNITS: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-units.json");
const FACTS: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-facts.json");

fn document(source: &str) -> AnchoredMarkdown {
    anchor_markdown(StableId::parse("test:m2").unwrap(), None, source.into()).unwrap()
}

fn prepared(source: &str) -> PreparedUnitDiscoveryRequest {
    let doc = document(source);
    let selected = resolve_selection(
        &doc,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: doc.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    build_unit_discovery_request(&doc, &selected, Default::default()).unwrap()
}

fn run_id() -> StableId {
    StableId::parse("mathscape:discovery-run/patch-fixture").unwrap()
}

/// Which exact replacement P proposes for the single A conversion issue.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Patch {
    /// Inside u2 and inside block b2: SafeForB.
    Inner,
    /// Starts at the first byte of u1/b1: touches a unit boundary.
    Boundary,
    /// Splits b2 by inserting a heading.
    Heading,
    /// The target text does not exist.
    Missing,
    /// First P call fails in transport.
    TransportOnce,
}

struct Backend {
    profile: LlmProfile,
    patch: Patch,
    requests: Mutex<Vec<LlmRequest>>,
    failed: AtomicBool,
}

impl Backend {
    fn new(patch: Patch) -> Self {
        Self {
            profile: LlmProfile {
                version: LLM_PROFILE_VERSION.into(),
                id: "m2".into(),
                display_name: None,
                provider: "openai".into(),
                wire_protocol: WireProtocol::OpenAiChatCompletions,
                model: "scripted".into(),
                endpoint: None,
                credential: None,
                capabilities: Default::default(),
            },
            patch,
            requests: Mutex::new(vec![]),
            failed: AtomicBool::new(false),
        }
    }
    fn calls(&self, kind: &str) -> Vec<LlmRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.task.kind == kind)
            .cloned()
            .collect()
    }
    fn issue_block(&self) -> &'static str {
        if self.patch == Patch::Boundary {
            "b1"
        } else {
            "b2"
        }
    }
}

fn response(value: Value) -> LlmResponse {
    LlmResponse {
        target: LlmResolvedTarget {
            profile_id: "m2".into(),
            provider: "openai".into(),
            wire_protocol: WireProtocol::OpenAiChatCompletions,
            requested_model: "scripted".into(),
            resolved_model: "scripted".into(),
            provider_model: "scripted".into(),
        },
        parts: vec![LlmResponsePart::Text {
            text: value.to_string(),
        }],
        reasoning: None,
        finish_reason: Some(LlmFinishReason::Completed { raw: "stop".into() }),
        usage: LlmUsage {
            input_tokens: Some(10),
            output_tokens: Some(10),
            total_tokens: Some(20),
            ..Default::default()
        },
        response_id: None,
        elapsed_ms: 1,
        raw_response: None,
    }
}

fn user(request: &LlmRequest) -> Value {
    serde_json::from_str(
        &request.messages[1]
            .parts
            .iter()
            .filter_map(|part| match part {
                LlmMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
    )
    .unwrap()
}

impl LlmBackend for Backend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }
    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        self.execute_stream(request)
    }
    fn execute_stream<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request.clone());
            let input = user(&request);
            let output = match request.task.kind.as_str() {
                "unit-discovery" => {
                    let mut units: Value = serde_json::from_str(UNITS).unwrap();
                    units["request_ref"] = input["request_ref"].clone();
                    let block = self.issue_block();
                    units["issues"] = json!([{
                        "kind": "suspected_conversion_error",
                        "unit": if block == "b1" { "u1" } else { "u2" },
                        "evidence": [{"kind": "block_range", "first": block, "last": block}],
                        "observation": "fixture conversion damage",
                        "context_request": null
                    }]);
                    units
                }
                "markdown-patch" => {
                    if self.patch == Patch::TransportOnce
                        && !self.failed.swap(true, Ordering::SeqCst)
                    {
                        return Err(LlmError::Transport("fixture disconnect".into()));
                    }
                    let (block, text, replacement) = match self.patch {
                        Patch::Inner | Patch::TransportOnce => ("b2", "N=1", "N=2"),
                        Patch::Boundary => ("b1", "**命题", "**引理"),
                        Patch::Heading => ("b2", "N=1；", "N=1\n\n# 插入标题\n\n"),
                        Patch::Missing => ("b2", "N=9", "N=2"),
                    };
                    json!({
                        "version": MARKDOWN_PATCH_VERSION,
                        "request_ref": input["request_ref"],
                        "patches": [{
                            "patch_ref": "model-1",
                            "issue_ref": input["issues"][0]["issue_ref"],
                            "target": {
                                "kind": "text_occurrence",
                                "block": block,
                                "text": text,
                                "occurrence": null
                            },
                            "replacement": replacement,
                            "rationale": "fixture repair"
                        }]
                    })
                }
                "source-facts" => {
                    let input: SourceFactInput = serde_json::from_value(input).unwrap();
                    let mut facts: SourceFactProposal = serde_json::from_str(FACTS).unwrap();
                    facts.request_ref = input.request_ref.clone();
                    facts
                        .tasks
                        .retain(|task| input.tasks.iter().any(|t| t.task_ref == task.task_ref));
                    serde_json::to_value(facts).unwrap()
                }
                kind => panic!("unexpected task {kind}"),
            };
            Ok(response(output))
        })
    }
}

fn reviewing(backend: &Backend) -> DiscoveryWorkflow<'_> {
    DiscoveryWorkflow::new(backend, Default::default())
        .with_patch_gate(MarkdownPatchGate::ReviewBeforeApply)
        .with_run_id(run_id())
}

fn fact_input(backend: &Backend) -> SourceFactInput {
    let calls = backend.calls("source-facts");
    assert_eq!(calls.len(), 1);
    serde_json::from_value(user(&calls[0])).unwrap()
}

#[tokio::test]
async fn skip_gate_never_calls_p_and_passes_no_warnings() {
    let backend = Backend::new(Patch::Inner);
    let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
    assert!(backend.calls("markdown-patch").is_empty());
    assert!(fact_input(&backend).source_warnings.is_empty());
}

#[tokio::test]
async fn review_gate_pauses_before_b_and_rejection_keeps_the_original_snapshot() {
    let backend = Backend::new(Patch::Inner);
    let mut flow = reviewing(&backend);
    let original = prepared(SOURCE);
    assert_eq!(
        flow.run(original.clone()).await,
        WorkflowStatus::AwaitingPatchReview
    );
    assert!(flow.result().is_none());
    assert!(backend.calls("source-facts").is_empty());
    let pending = flow.pending_patch().unwrap().clone();
    assert_eq!(pending.run_id, run_id());
    assert_eq!(pending.patches.len(), 1);
    assert_eq!(pending.patches[0].patch_ref, "p1");
    assert_eq!(pending.patches[0].issue_ref.as_str(), "a-i1");
    assert_eq!(
        flow.snapshot().patch.pending.as_ref(),
        Some(&pending),
        "the pending set is part of the persisted snapshot"
    );

    // Calling run again while paused neither spends a call nor reaches B.
    assert_eq!(
        flow.run(original.clone()).await,
        WorkflowStatus::AwaitingPatchReview
    );
    assert_eq!(backend.requests.lock().unwrap().len(), 2);

    flow.resolve_patch_review(PatchReviewDecision::RejectAll)
        .unwrap();
    assert!(flow.patch_round_exhausted());
    assert_eq!(flow.run(original).await, WorkflowStatus::Completed);
    let input = fact_input(&backend);
    assert_eq!(input.source_warnings.len(), 1);
    assert_eq!(input.source_warnings[0].issue_ref.as_str(), "a-i1");
    assert_eq!(backend.calls("markdown-patch").len(), 1);
    assert_eq!(
        flow.source_input().unwrap().snapshot.artifact_id.as_str(),
        "test:m2"
    );
}

#[tokio::test]
async fn safe_patch_rebinds_the_frozen_units_and_b_reads_the_patched_snapshot() {
    let backend = Backend::new(Patch::Inner);
    let mut flow = reviewing(&backend);
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::AwaitingPatchReview
    );
    let before = flow.units().unwrap().fingerprint().clone();
    let pending = flow.pending_patch().unwrap().clone();
    let continuation = flow
        .prepare_patch_continuation(&document(SOURCE), &pending)
        .unwrap();
    assert_eq!(continuation.safety().status, PatchSafetyStatus::SafeForB);
    assert!(continuation.rebinds_units());
    assert!(continuation.document().source.contains("取 N=2；"));
    let next = continuation.prepared();
    let patched_artifact = continuation.patched_artifact_id().clone();
    flow.resolve_patch_review(PatchReviewDecision::Apply(Box::new(continuation)))
        .unwrap();
    assert_eq!(flow.run(next.clone()).await, WorkflowStatus::Completed);

    assert_eq!(backend.calls("unit-discovery").len(), 1);
    let units = flow.units().unwrap();
    assert_ne!(units.fingerprint(), &before);
    assert_eq!(units.input_fingerprint(), &next.input().fingerprint);
    assert_eq!(
        flow.source_input().unwrap().snapshot.artifact_id,
        patched_artifact
    );
    let input = fact_input(&backend);
    let text: String = input
        .blocks
        .iter()
        .map(|block| block.text.clone() + &block.following_gap)
        .collect();
    assert!(text.contains("取 N=2；"));
    assert!(
        input.source_warnings.is_empty(),
        "the applied issue is resolved"
    );
    let applied = flow.patch_state().applied.as_ref().unwrap();
    assert_eq!(applied.accepted_patch_refs, ["p1"]);
    assert_eq!(applied.new_input_fingerprint, next.input().fingerprint);
    assert!(flow.result().unwrap().complete());
}

#[tokio::test]
async fn boundary_patch_rebuilds_a_once_and_carries_its_issues_to_b() {
    let backend = Backend::new(Patch::Boundary);
    let mut flow = reviewing(&backend);
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::AwaitingPatchReview
    );
    let pending = flow.pending_patch().unwrap().clone();
    let continuation = flow
        .prepare_patch_continuation(&document(SOURCE), &pending)
        .unwrap();
    assert_eq!(
        continuation.safety().status,
        PatchSafetyStatus::RequiresARebuild
    );
    assert!(!continuation.rebinds_units());
    let next = continuation.prepared();
    flow.resolve_patch_review(PatchReviewDecision::Apply(Box::new(continuation)))
        .unwrap();
    assert!(flow.units().is_none());
    assert_eq!(flow.run(next).await, WorkflowStatus::Completed);

    assert_eq!(backend.calls("unit-discovery").len(), 2);
    assert_eq!(
        backend.calls("markdown-patch").len(),
        1,
        "no second P round"
    );
    let rebuilt = user(&backend.calls("unit-discovery")[1]);
    assert!(
        rebuilt["blocks"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("**引理")
    );
    let input = fact_input(&backend);
    assert_eq!(input.source_warnings.len(), 1);
    assert_eq!(input.source_warnings[0].unit_ref.as_deref(), Some("u1"));
}

#[tokio::test]
async fn structural_patch_is_refused_and_the_run_stays_paused() {
    let backend = Backend::new(Patch::Heading);
    let mut flow = reviewing(&backend);
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::AwaitingPatchReview
    );
    let pending = flow.pending_patch().unwrap().clone();
    assert!(matches!(
        flow.prepare_patch_continuation(&document(SOURCE), &pending),
        Err(PatchReviewError::Selection(_))
    ));
    assert_eq!(flow.status(), &WorkflowStatus::AwaitingPatchReview);
}

#[tokio::test]
async fn continuation_rejects_a_drifted_base_or_foreign_patch() {
    let backend = Backend::new(Patch::Inner);
    let mut flow = reviewing(&backend);
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::AwaitingPatchReview
    );
    let pending = flow.pending_patch().unwrap().clone();
    let drifted = document(&SOURCE.replace("收敛于 c", "收敛于 d"));
    assert!(matches!(
        flow.prepare_patch_continuation(&drifted, &pending),
        Err(PatchReviewError::Stale(_))
    ));
    let mut forged = pending.clone();
    forged.patches[0].range.start += 1;
    assert!(matches!(
        flow.prepare_patch_continuation(&document(SOURCE), &forged),
        Err(PatchReviewError::Selection(_))
    ));
}

#[tokio::test]
async fn unusable_p_answer_spends_the_round_and_b_sees_the_issue() {
    let backend = Backend::new(Patch::Missing);
    let mut flow = reviewing(&backend);
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
    assert!(flow.patch_round_exhausted());
    assert_eq!(fact_input(&backend).source_warnings.len(), 1);
    assert!(
        flow.events()
            .iter()
            .any(|event| event.contains("quote_not_found"))
    );
}

#[tokio::test]
async fn p_transport_failure_needs_attention_without_spending_the_round() {
    let backend = Backend::new(Patch::TransportOnce);
    let mut flow = reviewing(&backend);
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::NeedsAttention
    );
    assert_eq!(flow.patch_state().rounds, 0);
    assert!(backend.calls("source-facts").is_empty());
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::AwaitingPatchReview
    );
    assert_eq!(backend.calls("unit-discovery").len(), 1);
    assert_eq!(backend.calls("markdown-patch").len(), 2);
}

#[tokio::test]
async fn persisted_pause_resumes_in_a_new_workflow_with_its_budget() {
    let backend = Backend::new(Patch::Inner);
    let mut flow = reviewing(&backend);
    let original = prepared(SOURCE);
    assert_eq!(
        flow.run(original.clone()).await,
        WorkflowStatus::AwaitingPatchReview
    );
    // Round-trip everything through JSON, as the desktop checkpoint does.
    let snapshot = serde_json::to_value(flow.snapshot()).unwrap();
    let checkpoint = || PatchReviewCheckpoint {
        units: serde_json::from_value(snapshot["units"].clone()).unwrap(),
        pending: serde_json::from_value(snapshot["patch"]["pending"].clone()).unwrap(),
        attempts: serde_json::from_value(snapshot["attempts"].clone()).unwrap(),
        diagnostics: serde_json::from_value(snapshot["diagnostics"].clone()).unwrap(),
        events: serde_json::from_value(snapshot["events"].clone()).unwrap(),
    };
    let source: DiscoveryInput = serde_json::from_value(snapshot["source"].clone()).unwrap();
    let rebuilt = prepare_unit_discovery_request_from_input(source);

    // A different run id or a skipped gate cannot adopt the paused run.
    assert!(
        DiscoveryWorkflow::new(&backend, Default::default())
            .with_run_id(run_id())
            .continue_from(&rebuilt, checkpoint())
            .is_err()
    );

    let mut resumed = reviewing(&backend)
        .continue_from(&rebuilt, checkpoint())
        .unwrap();
    assert_eq!(resumed.status(), &WorkflowStatus::AwaitingPatchReview);
    assert_eq!(resumed.attempts().len(), 2);
    let pending = resumed.pending_patch().unwrap().clone();
    let continuation = resumed
        .prepare_patch_continuation(&document(SOURCE), &pending)
        .unwrap();
    let next = continuation.prepared();
    resumed
        .resolve_patch_review(PatchReviewDecision::Apply(Box::new(continuation)))
        .unwrap();
    assert_eq!(resumed.run(next).await, WorkflowStatus::Completed);
    assert_eq!(resumed.attempts().len(), 3);
    assert_eq!(backend.calls("unit-discovery").len(), 1);

    // Restored attempts still count: with two calls allowed B has no budget.
    let mut limited = DiscoveryWorkflow::new(
        &backend,
        DiscoveryWorkflowLimits {
            max_calls: 2,
            ..Default::default()
        },
    )
    .with_patch_gate(MarkdownPatchGate::ReviewBeforeApply)
    .with_run_id(run_id())
    .continue_from(&rebuilt, checkpoint())
    .unwrap();
    limited
        .resolve_patch_review(PatchReviewDecision::RejectAll)
        .unwrap();
    assert_eq!(limited.run(rebuilt).await, WorkflowStatus::Partial);
    assert!(
        limited
            .events()
            .iter()
            .any(|event| event.contains("call budget"))
    );
}
