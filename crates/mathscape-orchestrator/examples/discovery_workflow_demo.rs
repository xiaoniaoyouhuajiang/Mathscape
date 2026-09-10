//! Offline replay: binds recorded fixture replies to fresh request tokens.
use mathscape_domain::*;
use mathscape_llm::*;
use mathscape_orchestrator::*;
use serde_json::Value;

struct FixtureBackend {
    profile: LlmProfile,
}
impl LlmBackend for FixtureBackend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }
    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        self.execute_stream(request)
    }
    fn execute_stream<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move {
            let input_text = request.messages[1]
                .parts
                .iter()
                .filter_map(|p| match p {
                    LlmMessagePart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            let input: Value = serde_json::from_str(&input_text).expect("prepared input");
            let fixture = match request.task.kind.as_str() {
                "unit-discovery" => {
                    include_str!("../../mathscape-domain/tests/fixtures/staged-units.json")
                }
                "source-facts" => {
                    include_str!("../../mathscape-domain/tests/fixtures/staged-facts.json")
                }
                _ => return Err(LlmError::Exhausted),
            };
            let mut value: Value = serde_json::from_str(fixture).expect("fixture");
            value["request_ref"] = input["request_ref"].clone();
            Ok(LlmResponse {
                target: LlmResolvedTarget {
                    profile_id: "offline".into(),
                    provider: "fixture".into(),
                    wire_protocol: WireProtocol::OpenAiChatCompletions,
                    requested_model: "fixture".into(),
                    resolved_model: "fixture".into(),
                    provider_model: "fixture".into(),
                },
                parts: vec![LlmResponsePart::Text {
                    text: value.to_string(),
                }],
                reasoning: None,
                finish_reason: Some(LlmFinishReason::Completed {
                    raw: "fixture".into(),
                }),
                usage: Default::default(),
                response_id: None,
                elapsed_ms: 0,
                raw_response: None,
            })
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../mathscape-domain/tests/fixtures/staged-source.md");
    let document = anchor_markdown(StableId::parse("fixture:staged-demo")?, None, source.into())?;
    let selection = resolve_selection(
        &document,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: document.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )?;
    let request = build_unit_discovery_request(&document, &selection, Default::default())?;
    let backend = FixtureBackend {
        profile: LlmProfile {
            version: LLM_PROFILE_VERSION.into(),
            id: "offline".into(),
            display_name: None,
            provider: "fixture".into(),
            wire_protocol: WireProtocol::OpenAiChatCompletions,
            model: "fixture".into(),
            endpoint: None,
            credential: None,
            capabilities: Default::default(),
        },
    };
    let mut workflow = DiscoveryWorkflow::new(&backend, Default::default());
    let status = workflow.run(request).await;
    println!("{}", serde_json::to_string_pretty(&workflow.snapshot())?);
    if status != WorkflowStatus::Completed {
        return Err("offline workflow did not complete".into());
    }
    Ok(())
}
