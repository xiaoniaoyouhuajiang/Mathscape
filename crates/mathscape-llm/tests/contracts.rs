use mathscape_llm::{
    CapabilitySupport, CredentialRef, CredentialResolver, CredentialStore, GenaiBackendOptions,
    LLM_PROFILE_VERSION, LlmBackend, LlmCapabilities, LlmError, LlmErrorKind, LlmFinishReason,
    LlmGenerationOptions, LlmMessage, LlmProfile, LlmRequest, LlmResolvedTarget, LlmResponse,
    LlmResponseFormat, LlmResponsePart, LlmRole, LlmTask, LlmToolChoice, LlmUsage,
    MemoryCredentialResolver, ScriptedLlmBackend, WireProtocol, execute_typed, response_format_for,
};
use schemars::JsonSchema;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use url::Url;

fn profile(endpoint: Option<Url>) -> LlmProfile {
    LlmProfile {
        display_name: None,
        version: LLM_PROFILE_VERSION.to_string(),
        id: "test-profile".to_string(),
        provider: "test-relay".to_string(),
        wire_protocol: WireProtocol::OpenAiChatCompletions,
        model: "test-model".to_string(),
        endpoint,
        credential: None,
        capabilities: LlmCapabilities::default(),
    }
}

fn request(response_format: LlmResponseFormat) -> LlmRequest {
    LlmRequest {
        task: LlmTask {
            kind: "source_discovery".to_string(),
            version: "1".to_string(),
            input_fingerprint: None,
        },
        messages: vec![LlmMessage::text(LlmRole::User, "Return JSON")],
        tools: Vec::new(),
        tool_choice: LlmToolChoice::Auto,
        response_format,
        options: LlmGenerationOptions::default(),
    }
}

fn response(text: &str) -> LlmResponse {
    LlmResponse {
        target: LlmResolvedTarget {
            profile_id: "test-profile".to_string(),
            provider: "test-relay".to_string(),
            wire_protocol: WireProtocol::OpenAiChatCompletions,
            requested_model: "test-model".to_string(),
            resolved_model: "test-model".to_string(),
            provider_model: "test-model-v1".to_string(),
        },
        parts: vec![LlmResponsePart::Text {
            text: text.to_string(),
        }],
        reasoning: None,
        finish_reason: Some(LlmFinishReason::Completed {
            raw: "stop".to_string(),
        }),
        usage: LlmUsage::default(),
        response_id: None,
        elapsed_ms: 1,
        raw_response: None,
    }
}

#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
struct Answer {
    answer: u32,
}

#[test]
fn profile_requires_explicit_endpoint_for_unknown_provider() {
    let error = profile(None).validate().unwrap_err();
    assert!(error.to_string().contains("explicit endpoint"));

    let mut invalid = profile(Some(Url::parse("https://relay.example/v1/").unwrap()));
    invalid.endpoint = Some(Url::parse("https://secret@relay.example/v1/").unwrap());
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("embedded")
    );
}

#[test]
fn credentials_are_referenced_and_debug_redacted() {
    let resolver = MemoryCredentialResolver::default();
    let reference = CredentialRef::new("profile:test").unwrap();
    resolver
        .insert(reference, SecretString::from("top-secret".to_string()))
        .unwrap();
    let debug = format!("{resolver:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains("top-secret"));
    resolver
        .delete(&CredentialRef::new("profile:test").unwrap())
        .unwrap();
    assert!(
        resolver
            .resolve(&CredentialRef::new("profile:test").unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn typed_execution_uses_schema_and_preserves_transport_metadata() {
    let profile = profile(Some(Url::parse("https://relay.example/v1/").unwrap()));
    let backend = ScriptedLlmBackend::new(profile, [Ok(response(r#"{"answer":42}"#))]);
    let format = response_format_for::<Answer>("answer", None).unwrap();
    let result = execute_typed::<Answer>(&backend, request(format))
        .await
        .unwrap();

    assert_eq!(result.value, Answer { answer: 42 });
    assert_eq!(result.response.target.provider_model, "test-model-v1");
    assert_eq!(backend.requests().len(), 1);
}

#[tokio::test]
async fn typed_execution_rejects_text_mode_and_invalid_json() {
    let profile = profile(Some(Url::parse("https://relay.example/v1/").unwrap()));
    let backend = ScriptedLlmBackend::new(profile, [Ok(response("not json"))]);

    let error = execute_typed::<Answer>(&backend, request(LlmResponseFormat::Text))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), LlmErrorKind::InvalidRequest);

    let error = execute_typed::<Answer>(&backend, request(LlmResponseFormat::JsonObject))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), LlmErrorKind::InvalidResponse);
}

#[tokio::test]
async fn scripted_backend_is_object_safe_and_exhaustion_is_explicit() {
    let profile = profile(Some(Url::parse("https://relay.example/v1/").unwrap()));
    let backend: Arc<dyn LlmBackend> = Arc::new(ScriptedLlmBackend::new(profile, []));
    let error = backend
        .execute(request(LlmResponseFormat::Text))
        .await
        .unwrap_err();
    assert_eq!(error, LlmError::Exhausted);
}

#[test]
fn request_validation_rejects_unsupported_and_inconsistent_options() {
    let mut request = request(LlmResponseFormat::JsonSchema {
        name: "bad name".to_string(),
        description: None,
        schema: json!({"type": "object"}),
    });
    assert!(request.validate().is_err());

    request.response_format = LlmResponseFormat::Text;
    request.options.top_p = Some(2.0);
    assert!(request.validate().is_err());

    let _ = GenaiBackendOptions::default();
    let _ = CapabilitySupport::Unknown;
}
