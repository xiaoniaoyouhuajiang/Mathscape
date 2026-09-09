use mathscape_domain::{
    DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoverySelectionRequest, SourceSelector,
    StableId,
};
use mathscape_llm::{
    LLM_PROFILE_VERSION, LlmCapabilities, LlmError, LlmFinishReason, LlmProfile, LlmResolvedTarget,
    LlmResponse, LlmResponsePart, LlmUsage, ScriptedLlmBackend, WireProtocol,
};
use mathscape_orchestrator::{
    DiscoveryProducer, DiscoveryProducerLimits, DiscoveryProductionOutcome,
    DiscoveryProposalRejection, DiscoveryValidationLimits, PreparedDiscoveryRequest,
    ValidatedProposalStatus, anchor_markdown, build_discovery_request, resolve_selection,
};
use serde_json::json;

const SAMPLE: &str = include_str!("../../mathscape-domain/tests/fixtures/discovery-proposal.json");
const SOURCE: &str = "习题 1：设 $a_n=a$，用定义证明 $a_n$ 收敛于 $a$。\n\n解答：任取 $\\epsilon>0$，取 $N=1$。当 $n>N$ 时，$|a_n-a|=0<\\epsilon$。因此收敛于 $a$。\n";

fn profile() -> LlmProfile {
    LlmProfile {
        version: LLM_PROFILE_VERSION.into(),
        id: "discovery-test".into(),
        provider: "openai".into(),
        wire_protocol: WireProtocol::OpenAiResponses,
        model: "test-model".into(),
        endpoint: None,
        credential: None,
        capabilities: LlmCapabilities::default(),
    }
}

fn prepared_fixture() -> (PreparedDiscoveryRequest, String) {
    let document = anchor_markdown(
        StableId::parse("test:producer").unwrap(),
        None,
        SOURCE.into(),
    )
    .unwrap();
    let selection = resolve_selection(
        &document,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: document.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    let prepared = build_discovery_request(&document, &selection, Default::default()).unwrap();
    let sample = SAMPLE
        .replace("b-problem", &document.blocks[0].id)
        .replace("b-solution", &document.blocks[1].id);
    let mut value: serde_json::Value = serde_json::from_str(&sample).unwrap();
    value["input_fingerprint"] = json!(prepared.input().fingerprint);
    (prepared, serde_json::to_string(&value).unwrap())
}

fn response(text: &str, finish_reason: Option<LlmFinishReason>) -> LlmResponse {
    LlmResponse {
        target: LlmResolvedTarget {
            profile_id: "discovery-test".into(),
            provider: "openai".into(),
            wire_protocol: WireProtocol::OpenAiResponses,
            requested_model: "test-model".into(),
            resolved_model: "test-model".into(),
            provider_model: "test-model-2026".into(),
        },
        parts: vec![LlmResponsePart::Text { text: text.into() }],
        reasoning: Some("must not enter the producer summary".into()),
        finish_reason,
        usage: LlmUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            ..Default::default()
        },
        response_id: Some("response-1".into()),
        elapsed_ms: 12,
        raw_response: Some(json!({"private": "not retained"})),
    }
}

fn completed() -> Option<LlmFinishReason> {
    Some(LlmFinishReason::Completed { raw: "stop".into() })
}

#[tokio::test]
async fn produces_and_validates_one_proposal_without_hiding_run_facts() {
    let (prepared, text) = prepared_fixture();
    let backend = ScriptedLlmBackend::new(profile(), [Ok(response(&text, completed()))]);

    let outcome = DiscoveryProducer::new(&backend).produce(prepared).await;
    let DiscoveryProductionOutcome::Valid { proposal, run } = outcome else {
        panic!("expected a valid proposal")
    };
    assert_eq!(proposal.units().len(), 2);
    assert_eq!(proposal.status(), ValidatedProposalStatus::Complete);
    assert_eq!(run.profile_id, "discovery-test");
    let observation = run.response.unwrap();
    assert_eq!(observation.response_id.as_deref(), Some("response-1"));
    assert_eq!(observation.usage.total_tokens, Some(150));
    assert_eq!(backend.requests().len(), 1);
    assert_eq!(backend.streaming_request_count(), 1);
}

#[tokio::test]
async fn distinguishes_backend_failure_from_rejected_model_output() {
    let (prepared, _) = prepared_fixture();
    let backend = ScriptedLlmBackend::new(profile(), [Err(LlmError::Timeout)]);
    let outcome = DiscoveryProducer::new(&backend).produce(prepared).await;
    let DiscoveryProductionOutcome::Failed { error, run } = outcome else {
        panic!("expected transport failure")
    };
    assert_eq!(error, LlmError::Timeout);
    assert!(run.response.is_none());

    let (prepared, _) = prepared_fixture();
    let backend = ScriptedLlmBackend::new(profile(), [Ok(response("not json", completed()))]);
    let outcome = DiscoveryProducer::new(&backend).produce(prepared).await;
    assert!(matches!(
        outcome,
        DiscoveryProductionOutcome::InvalidProposal {
            proposal: None,
            rejection: DiscoveryProposalRejection::InvalidJson { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_truncation_tools_and_oversized_text_before_decoding() {
    let (prepared, text) = prepared_fixture();
    let backend = ScriptedLlmBackend::new(
        profile(),
        [Ok(response(
            &text,
            Some(LlmFinishReason::MaxTokens {
                raw: "length".into(),
            }),
        ))],
    );
    assert!(matches!(
        DiscoveryProducer::new(&backend).produce(prepared).await,
        DiscoveryProductionOutcome::InvalidProposal {
            rejection: DiscoveryProposalRejection::MaxOutputTokens,
            ..
        }
    ));

    let (prepared, _) = prepared_fixture();
    let mut tool_response = response("{}", completed());
    tool_response.parts.push(LlmResponsePart::ToolCall {
        call_id: "call-1".into(),
        name: "forbidden".into(),
        arguments: json!({}),
        continuation_tokens: vec![],
    });
    let backend = ScriptedLlmBackend::new(profile(), [Ok(tool_response)]);
    assert!(matches!(
        DiscoveryProducer::new(&backend).produce(prepared).await,
        DiscoveryProductionOutcome::InvalidProposal {
            rejection: DiscoveryProposalRejection::UnexpectedToolCall,
            ..
        }
    ));

    let (prepared, text) = prepared_fixture();
    let backend = ScriptedLlmBackend::new(profile(), [Ok(response(&text, completed()))]);
    let limits = DiscoveryProducerLimits::new(32, DiscoveryValidationLimits::default()).unwrap();
    assert!(matches!(
        DiscoveryProducer::with_limits(&backend, limits)
            .produce(prepared)
            .await,
        DiscoveryProductionOutcome::InvalidProposal {
            rejection: DiscoveryProposalRejection::ResponseTooLarge { limit: 32, .. },
            ..
        }
    ));
}

#[tokio::test]
async fn enforces_canonical_json_then_deterministic_source_validation() {
    let (prepared, text) = prepared_fixture();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["units"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_title");
    let backend = ScriptedLlmBackend::new(
        profile(),
        [Ok(response(
            &serde_json::to_string(&value).unwrap(),
            completed(),
        ))],
    );
    assert!(matches!(
        DiscoveryProducer::new(&backend).produce(prepared).await,
        DiscoveryProductionOutcome::InvalidProposal {
            proposal: Some(_),
            rejection: DiscoveryProposalRejection::NonCanonicalJson,
            ..
        }
    ));

    let (prepared, text) = prepared_fixture();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["input_fingerprint"] =
        json!("sha256:2222222222222222222222222222222222222222222222222222222222222222");
    let backend = ScriptedLlmBackend::new(
        profile(),
        [Ok(response(
            &serde_json::to_string(&value).unwrap(),
            completed(),
        ))],
    );
    let outcome = DiscoveryProducer::new(&backend).produce(prepared).await;
    let DiscoveryProductionOutcome::InvalidProposal {
        proposal: Some(_),
        rejection: DiscoveryProposalRejection::Validation(report),
        ..
    } = outcome
    else {
        panic!("expected deterministic validation rejection")
    };
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|item| { item.path() == "proposal.input_fingerprint" })
    );
}
