use mathscape_domain::*;
use mathscape_llm::*;
use mathscape_orchestrator::*;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SOURCE: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-source.md");
const UNITS: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-units.json");
fn prepared(source: &str) -> PreparedUnitDiscoveryRequest {
    let doc = anchor_markdown(StableId::parse("test:m2").unwrap(), None, source.into()).unwrap();
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
fn units(p: &PreparedUnitDiscoveryRequest) -> UnitDiscoveryProposal {
    let mut u: UnitDiscoveryProposal = serde_json::from_str(UNITS).unwrap();
    u.request_ref = p.model_input().request_ref.clone();
    u
}
fn profile() -> LlmProfile {
    LlmProfile {
        version: LLM_PROFILE_VERSION.into(),
        id: "m2".into(),
        display_name: None,
        provider: "openai".into(),
        wire_protocol: WireProtocol::OpenAiChatCompletions,
        model: "scripted".into(),
        endpoint: None,
        credential: None,
        capabilities: Default::default(),
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
            .filter_map(|p| match p {
                LlmMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
    )
    .unwrap()
}
fn facts(input: &SourceFactInput) -> SourceFactProposal {
    let mut f: SourceFactProposal = serde_json::from_str(include_str!(
        "../../mathscape-domain/tests/fixtures/staged-facts.json"
    ))
    .unwrap();
    f.request_ref = input.request_ref.clone();
    f.tasks
        .retain(|t| input.tasks.iter().any(|a| a.task_ref == t.task_ref));
    f
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Clean,
    RepairA,
    RepairB,
    RepairTask,
    Unrepairable,
    TransportBOnce,
    SlowB,
    RequiresRevision,
    WrongStageRepair,
    UnprocessedOnce,
}
struct Backend {
    profile: LlmProfile,
    mode: Mode,
    requests: Mutex<Vec<LlmRequest>>,
    failed: AtomicBool,
}
impl Backend {
    fn new(mode: Mode) -> Self {
        Self {
            profile: profile(),
            mode,
            requests: Mutex::new(vec![]),
            failed: AtomicBool::new(false),
        }
    }
}
impl LlmBackend for Backend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }
    fn execute<'a>(&'a self, r: LlmRequest) -> LlmFuture<'a> {
        self.execute_stream(r)
    }
    fn execute_stream<'a>(&'a self, r: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(r.clone());
            let input = user(&r);
            let output = match r.task.kind.as_str() {
                "unit-discovery" => {
                    let mut u: Value = serde_json::from_str(UNITS).unwrap();
                    u["request_ref"] = input["request_ref"].clone();
                    if self.mode == Mode::RepairA {
                        u["units"][0]["parent"] = json!("u2");
                    }
                    u
                }
                "source-facts" => {
                    if self.mode == Mode::SlowB {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    if self.mode == Mode::TransportBOnce
                        && !self.failed.swap(true, Ordering::SeqCst)
                    {
                        return Err(LlmError::Transport("fixture disconnect".into()));
                    }
                    let input: SourceFactInput = serde_json::from_value(input).unwrap();
                    let mut f = serde_json::to_value(facts(&input)).unwrap();
                    for task in f["tasks"].as_array_mut().unwrap() {
                        if task["task_ref"] == "t2" {
                            if self.mode == Mode::UnprocessedOnce
                                && !self.failed.swap(true, Ordering::SeqCst)
                            {
                                task["disposition"] =
                                    json!({"status":"unprocessed","reason":"fixture budget"});
                                task["references"] = json!([]);
                                task["relations"] = json!([]);
                            }
                            if matches!(
                                self.mode,
                                Mode::RepairB | Mode::Unrepairable | Mode::WrongStageRepair
                            ) {
                                task["references"][0]["anchor"]["occurrence"] = Value::Null;
                            }
                            if self.mode == Mode::RepairTask {
                                task["references"][0]["target_hint"] = json!("u2");
                            }
                            if self.mode == Mode::RequiresRevision {
                                task["issues"] = json!([{"kind":"uncertain_boundary","evidence":[],"observation":"需要修改单元边界","context_request":null,"requires_unit_revision":true}]);
                            }
                        }
                    }
                    f
                }
                "discovery-repair" => {
                    let repair = &input["repair"];
                    let action = if self.mode == Mode::RepairA {
                        let mut p = input["original"].clone();
                        p["units"][0]["parent"] = Value::Null;
                        json!({"kind":"replace_units","proposal":p})
                    } else if self.mode == Mode::RepairTask {
                        let mut task = input["original"]["tasks"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|t| t["task_ref"] == "t2")
                            .unwrap()
                            .clone();
                        task["references"][0]["target_hint"] = Value::Null;
                        json!({"kind":"replace_task","task_ref":"t2","proposal":task})
                    } else if self.mode == Mode::Unrepairable {
                        let task = input["original"]["tasks"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|t| t["task_ref"] == "t2")
                            .unwrap();
                        json!({"kind":"replace_task","task_ref":"t2","proposal":task})
                    } else if self.mode == Mode::WrongStageRepair {
                        json!({"kind":"replace_units","proposal":serde_json::from_str::<Value>(UNITS).unwrap()})
                    } else {
                        let issue = repair["issues"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|i| !i["candidates"].as_array().unwrap().is_empty())
                            .unwrap();
                        json!({"kind":"choose_candidate","issue_ref":issue["issue_ref"],"candidate_ref":"c1"})
                    };
                    json!({"version":"mathscape-discovery-repair-1","repair_ref":repair["repair_ref"],"actions":[action]})
                }
                _ => panic!("unexpected task"),
            };
            Ok(response(output))
        })
    }
}

#[tokio::test]
async fn complete_pipeline_uses_fresh_full_context_and_unresolved_is_valid() {
    let backend = Backend::new(Mode::Clean);
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let mut flow = DiscoveryWorkflow::new(&backend, Default::default())
        .with_observer(Arc::new(move |event| captured.lock().unwrap().push(event)));
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
    let result = flow.result().unwrap();
    assert!(result.complete());
    assert_eq!(result.references().len(), 2);
    assert!(
        result.references().iter().all(
            |r| r.owner == "u2" && r.resolution.status == ReferenceResolutionStatus::Unresolved
        )
    );
    assert_eq!(result.relations().len(), 1);
    assert_eq!(
        result.relations()[0].relation,
        DiscoveryRelationKind::Proves
    );
    let calls = backend.requests.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].messages.len(), 2);
    let b: SourceFactInput = serde_json::from_value(user(&calls[1])).unwrap();
    let reconstructed = b.leading_gap
        + &b.blocks
            .iter()
            .map(|b| b.text.clone() + &b.following_gap)
            .collect::<String>();
    assert_eq!(reconstructed, SOURCE);
    assert!(user(&calls[1]).get("original").is_none());
    drop(calls);
    let events = events.lock().unwrap();
    assert!(matches!(
        events.first(),
        Some(DiscoveryWorkflowEvent::WorkflowStarted)
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        DiscoveryWorkflowEvent::FactPlanCreated {
            batches: 1,
            tasks: 2
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DiscoveryWorkflowEvent::AttemptStarted {
            stage,
            batch_index: Some(1),
            batch_total: Some(1),
            ..
        } if stage == "source-facts"
    )));
    assert!(matches!(
        events.last(),
        Some(DiscoveryWorkflowEvent::WorkflowFinished {
            status: WorkflowStatus::Completed
        })
    ));
}

#[tokio::test]
async fn one_repair_per_stage_and_valid_sibling_is_retained() {
    for mode in [Mode::RepairA, Mode::RepairB, Mode::RepairTask] {
        let backend = Backend::new(mode);
        let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
        assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
        assert_eq!(flow.attempts().len(), 3);
        assert_eq!(flow.attempts().iter().filter(|a| a.repair).count(), 1);
        assert_eq!(flow.result().unwrap().tasks().len(), 2);
        if mode != Mode::RepairA {
            let repair = user(&flow.attempts()[2].request);
            assert_eq!(repair["repair"]["scope"]["task_refs"], json!(["t2"]));
        }
    }
}

#[tokio::test]
async fn failed_repair_does_not_loop_or_discard_successful_facts() {
    for mode in [Mode::Unrepairable, Mode::WrongStageRepair] {
        let backend = Backend::new(mode);
        let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
        assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
        assert_eq!(flow.attempts().len(), 3);
        let result = flow.result().unwrap();
        assert_eq!(result.tasks().len(), 1);
        assert_eq!(result.pending_tasks(), ["t2"]);
    }
}

#[tokio::test]
async fn batching_preserves_entire_source_and_reuses_frozen_a_on_resume() {
    let backend = Backend::new(Mode::TransportBOnce);
    let mut flow = DiscoveryWorkflow::new(
        &backend,
        DiscoveryWorkflowLimits {
            max_tasks_per_batch: 1,
            ..Default::default()
        },
    );
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
    let fingerprint = flow.units().unwrap().fingerprint().clone();
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
    assert_eq!(&fingerprint, flow.units().unwrap().fingerprint());
    let calls = backend.requests.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.task.kind == "unit-discovery")
            .count(),
        1
    );
    for c in calls.iter().filter(|c| c.task.kind == "source-facts") {
        let b: SourceFactInput = serde_json::from_value(user(c)).unwrap();
        assert_eq!(b.tasks.len(), 1);
        assert_eq!(
            b.leading_gap
                + &b.blocks
                    .iter()
                    .map(|b| b.text.clone() + &b.following_gap)
                    .collect::<String>(),
            SOURCE
        );
    }
}

#[tokio::test]
async fn budgets_and_revision_requests_preserve_checkpoints() {
    let backend = Backend::new(Mode::Clean);
    let mut flow = DiscoveryWorkflow::new(
        &backend,
        DiscoveryWorkflowLimits {
            max_calls: 1,
            ..Default::default()
        },
    );
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
    assert!(flow.units().is_some());
    assert_eq!(flow.attempts().len(), 1);
    let backend = Backend::new(Mode::RequiresRevision);
    let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
    assert_eq!(
        flow.run(prepared(SOURCE)).await,
        WorkflowStatus::NeedsAttention
    );
    assert!(flow.units().is_some());
    assert!(!flow.result().unwrap().complete());
}

#[tokio::test]
async fn explicit_resume_requests_only_unprocessed_tasks_and_respects_total_usage_gate() {
    let backend = Backend::new(Mode::UnprocessedOnce);
    let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
    assert_eq!(flow.result().unwrap().pending_tasks(), ["t2"]);
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Completed);
    {
        let calls = backend.requests.lock().unwrap();
        assert_eq!(calls.len(), 3);
        let last = user(&calls[2]);
        assert_eq!(last["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(last["tasks"][0]["task_ref"], "t2");
    }
    let backend = Backend::new(Mode::Clean);
    let mut flow = DiscoveryWorkflow::new(
        &backend,
        DiscoveryWorkflowLimits {
            max_total_tokens: Some(20),
            ..Default::default()
        },
    );
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
    assert_eq!(flow.attempts().len(), 1);
    assert!(flow.events().iter().any(|e| e.contains("token budget")));
}

#[tokio::test]
async fn global_deadline_and_future_cancellation_leave_a_available() {
    let backend = Backend::new(Mode::SlowB);
    let mut flow = DiscoveryWorkflow::new(
        &backend,
        DiscoveryWorkflowLimits {
            timeout: Duration::from_millis(50),
            ..Default::default()
        },
    );
    assert_eq!(flow.run(prepared(SOURCE)).await, WorkflowStatus::Partial);
    assert!(flow.units().is_some());
    assert_eq!(
        flow.attempts().last().unwrap().status,
        WorkflowStatus::Failed
    );
    let mut flow = DiscoveryWorkflow::new(&backend, Default::default());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), flow.run(prepared(SOURCE)))
            .await
            .is_err()
    );
    flow.cancel();
    assert_eq!(flow.status(), &WorkflowStatus::Cancelled);
    assert!(flow.units().is_some());
}

#[test]
fn split_boundaries_keep_utf8_source_order_and_enforce_task_ownership() {
    let p = prepared("命题。证明。\n");
    let mut u = units(&p);
    let q = |text: &str| {
        serde_json::from_value(json!({"kind":"quote_range","first":"b1","last":"b1","start":{"quote":text,"occurrence":null,"prefix":null,"suffix":null},"end":{"quote":text,"occurrence":null,"prefix":null,"suffix":null}})).unwrap()
    };
    u.units[0].selectors = vec![q("命题。")];
    u.units[1].selectors = vec![q("证明。")];
    u.coverage[0].selector = DiscoverySelector::BlockRange {
        first: "b1".into(),
        last: "b1".into(),
    };
    let frozen = p.validate(u, Default::default()).unwrap();
    let batches = plan_source_facts(&p, &frozen, 32).unwrap();
    let b = &batches[0];
    assert_eq!(b.model_input().blocks.len(), 3);
    let input = b.model_input();
    assert_eq!(
        input
            .blocks
            .iter()
            .map(|b| b.text.clone() + &b.following_gap)
            .collect::<String>(),
        "命题。证明。\n"
    );
    assert_eq!(input.tasks[0].evidence_block_refs, ["b1"]);
    assert_eq!(input.tasks[1].evidence_block_refs, ["b2"]);
    let mut f = facts(input);
    f.tasks[0].source_labels[0].anchor = TextOccurrence::TextOccurrence {
        block: "b2".into(),
        text: "证明。".into(),
        occurrence: None,
    };
    f.tasks[1].references.clear();
    f.tasks[1].relations.clear();
    let v = b.validate(&f);
    assert!(
        v.report
            .diagnostics
            .iter()
            .any(|d| d.code == "unauthorized_range")
    );
    assert_eq!(v.tasks.len(), 1);
}

#[test]
fn local_resolution_waits_for_all_labels_and_rejects_stale_unit_sets() {
    let p = prepared("# 本节\n\n定义 2：条件。\n\n由定义 2 得到结论。\n");
    let mut u = units(&p);
    u.units[0].unit_kind = DiscoveryUnitKind::Section;
    u.units[0].selectors = vec![u.coverage[0].selector.clone()];
    u.units[1].unit_kind = DiscoveryUnitKind::Definition;
    u.units[1].selectors = vec![DiscoverySelector::BlockRange {
        first: "b2".into(),
        last: "b2".into(),
    }];
    let mut last = u.units[1].clone();
    last.unit_ref = "u3".into();
    last.unit_kind = DiscoveryUnitKind::Narrative;
    last.selectors = vec![DiscoverySelector::BlockRange {
        first: "b3".into(),
        last: "b3".into(),
    }];
    u.units.push(last);
    let frozen = p.validate(u.clone(), Default::default()).unwrap();
    let batches = plan_source_facts(&p, &frozen, 1).unwrap();
    let mut valid = vec![];
    for b in &batches {
        let id = b.model_input().tasks[0].task_ref.clone();
        let anchor = |block: &str, text: &str| json!({"kind":"text_occurrence","block":block,"text":text,"occurrence":null});
        let f:SourceFactProposal=serde_json::from_value(json!({"version":"mathscape-source-facts-1","request_ref":b.model_input().request_ref,"tasks":[{"task_ref":id,"disposition":{"status":"processed"},"source_labels":if id=="t2"{json!([{"anchor":anchor("b2","2")}])}else{json!([])},"source_title":null,"references":if id=="t3"{json!([{"anchor":anchor("b3","定义 2"),"target_hint":null}])}else{json!([])},"relations":[],"issues":[]}]})).unwrap();
        let v = b.validate(&f);
        assert!(v.report.is_valid(), "{:?}", v.report);
        valid.extend(v.tasks);
    }
    valid.reverse();
    let result = assemble_discovery(&frozen, &batches, valid.clone()).unwrap();
    assert_eq!(
        result.references()[0].resolution.status,
        ReferenceResolutionStatus::ResolvedLocal
    );
    assert_eq!(result.relations()[0].target, "u2");
    u.units[2].display_title = Some("新标题".into());
    let changed = p.validate(u, Default::default()).unwrap();
    assert!(assemble_discovery(&changed, &batches, valid).is_err());
}

#[test]
fn repair_schema_snapshot_matches_runtime_and_closed_contract() {
    let schema = repair_output_schema();
    let snapshot: Value = serde_json::from_str(include_str!(
        "../../../prompts/discovery-repair/v1/output.schema.json"
    ))
    .unwrap();
    assert_eq!(schema, snapshot);
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = json!({"version":"mathscape-discovery-repair-1","repair_ref":"p1","actions":[{"kind":"choose_candidate","issue_ref":"i1","candidate_ref":"c1"}]});
    assert!(validator.is_valid(&value));
    value["actions"][0]["source"] = json!("u1");
    assert!(!validator.is_valid(&value));
}
