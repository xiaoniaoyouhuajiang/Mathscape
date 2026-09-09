use mathscape_llm::{
    CapabilitySupport, CredentialRef, GenaiBackend, GenaiBackendOptions, LLM_PROFILE_VERSION,
    LlmBackend, LlmCapabilities, LlmErrorKind, LlmGenerationOptions, LlmMessage, LlmProfile,
    LlmRequest, LlmResponseFormat, LlmRole, LlmTask, LlmToolChoice, MemoryCredentialResolver,
    WireProtocol,
};
use secrecy::SecretString;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path},
};

fn request() -> LlmRequest {
    LlmRequest {
        task: LlmTask {
            kind: "source_discovery".to_string(),
            version: "1".to_string(),
            input_fingerprint: None,
        },
        messages: vec![
            LlmMessage::text(LlmRole::System, "Return only valid JSON."),
            LlmMessage::text(LlmRole::User, "Extract the selected source."),
        ],
        tools: Vec::new(),
        tool_choice: LlmToolChoice::Auto,
        response_format: LlmResponseFormat::JsonSchema {
            name: "discovery".to_string(),
            description: None,
            schema: json!({
                "$defs": {
                    "BooleanAnswer": {
                        "type": "boolean",
                        "description": "Reusable boolean answer"
                    }
                },
                "type": "object",
                "properties": {
                    "ok": {
                        "$ref": "#/$defs/BooleanAnswer",
                        "description": "Whether discovery succeeded"
                    }
                },
                "required": ["ok"],
                "additionalProperties": false
            }),
        },
        options: LlmGenerationOptions {
            temperature: Some(0.0),
            max_output_tokens: Some(256),
            capture_raw_response: true,
            ..LlmGenerationOptions::default()
        },
    }
}

fn profile(server: &MockServer) -> LlmProfile {
    LlmProfile {
        display_name: None,
        version: LLM_PROFILE_VERSION.to_string(),
        id: "relay-test".to_string(),
        provider: "custom-relay".to_string(),
        wire_protocol: WireProtocol::OpenAiChatCompletions,
        model: "relay-model".to_string(),
        endpoint: Some(Url::parse(&format!("{}/v1/", server.uri())).unwrap()),
        credential: Some(CredentialRef::new("relay-test-key").unwrap()),
        capabilities: LlmCapabilities {
            structured_output: CapabilitySupport::Supported,
            usage: CapabilitySupport::Supported,
            ..LlmCapabilities::default()
        },
    }
}

fn credentials() -> Arc<MemoryCredentialResolver> {
    let resolver = Arc::new(MemoryCredentialResolver::default());
    resolver
        .insert(
            CredentialRef::new("relay-test-key").unwrap(),
            SecretString::from("fixture-secret".to_string()),
        )
        .unwrap();
    resolver
}

#[tokio::test]
async fn custom_openai_compatible_endpoint_receives_structured_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer fixture-secret"))
        .and(body_partial_json(json!({
            "model": "relay-model",
            "temperature": 0.0,
            "max_tokens": 256
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-fixture",
            "object": "chat.completion",
            "created": 1,
            "model": "relay-model-v2",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "{\"ok\":true}"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 5,
                "total_tokens": 16
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions {
            timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    let response = backend.execute(request()).await.unwrap();

    assert_eq!(response.text().as_deref(), Some(r#"{"ok":true}"#));
    assert_eq!(response.target.provider_model, "relay-model-v2");
    assert_eq!(response.usage.input_tokens, Some(11));
    assert_eq!(response.usage.total_tokens, Some(16));
    assert!(response.raw_response.is_some());

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = received[0].body_json().unwrap();
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["name"], "discovery");
    let wire_schema = &body["response_format"]["json_schema"]["schema"];
    assert_eq!(
        wire_schema["properties"]["ok"]["$ref"],
        "#/$defs/BooleanAnswer"
    );
    assert!(wire_schema["properties"]["ok"].get("description").is_none());
    assert_eq!(
        wire_schema["$defs"]["BooleanAnswer"]["description"],
        "Reusable boolean answer"
    );
}

#[tokio::test]
async fn custom_openai_compatible_stream_collects_structured_json_and_usage() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"relay-model-v2\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"{\\\"ok\\\":\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"relay-model-v2\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"true}\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"chatcmpl-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"relay-model-v2\",\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":5,\"total_tokens\":16}}\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer fixture-secret"))
        .and(body_partial_json(json!({
            "model": "relay-model",
            "stream": true,
            "stream_options": {"include_usage": true}
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions {
            timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let backend = backend.with_stream_observer(Arc::new(move |kind, text| {
        captured
            .lock()
            .unwrap()
            .push((kind.to_string(), text.to_string()));
    }));
    let response = backend.execute_stream(request()).await.unwrap();
    let events = events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _)| kind == "text")
            .map(|(_, text)| text.as_str())
            .collect::<String>(),
        r#"{"ok":true}"#
    );
    assert_eq!(events.last().unwrap().0, "validating");

    assert_eq!(response.text().as_deref(), Some(r#"{"ok":true}"#));
    assert_eq!(response.usage.input_tokens, Some(11));
    assert_eq!(response.usage.output_tokens, Some(5));
    assert_eq!(response.usage.total_tokens, Some(16));
    assert!(matches!(
        response.finish_reason,
        Some(mathscape_llm::LlmFinishReason::Completed { .. })
    ));
    assert!(response.raw_response.is_none());

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = received[0].body_json().unwrap();
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["name"], "discovery");
}

#[tokio::test]
async fn streaming_gateway_status_is_preserved_as_a_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": true})))
        .respond_with(ResponseTemplate::new(524).set_body_string("gateway timeout page"))
        .mount(&server)
        .await;
    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions::default(),
    )
    .unwrap();

    let error = backend.execute_stream(request()).await.unwrap_err();
    assert_eq!(
        error,
        mathscape_llm::LlmError::Provider {
            status: Some(524),
            message: "upstream gateway timed out while waiting for the model response".to_string(),
        }
    );
}

#[tokio::test]
async fn rate_limit_is_classified_and_retry_after_is_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "3")
                .set_body_json(json!({"error": {"message": "slow down"}})),
        )
        .mount(&server)
        .await;
    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions::default(),
    )
    .unwrap();

    let error = backend.execute(request()).await.unwrap_err();
    assert_eq!(error.kind(), LlmErrorKind::RateLimited);
    assert!(matches!(
        error,
        mathscape_llm::LlmError::RateLimited {
            retry_after_ms: Some(3000)
        }
    ));
}

#[tokio::test]
async fn structured_provider_error_fields_are_safely_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "message": "Invalid schema for response_format 'discovery'",
                "type": "invalid_request_error",
                "param": "text.format.schema",
                "code": "invalid_json_schema",
                "private_diagnostics": "must not reach the UI"
            },
            "request_echo": "must not reach the UI"
        })))
        .mount(&server)
        .await;
    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions::default(),
    )
    .unwrap();

    let error = backend.execute(request()).await.unwrap_err();
    let mathscape_llm::LlmError::Provider {
        status: Some(400),
        message,
    } = error
    else {
        panic!("expected an HTTP 400 provider error")
    };
    assert!(message.contains("code=invalid_json_schema"));
    assert!(message.contains("param=text.format.schema"));
    assert!(message.contains("message=Invalid schema"));
    assert!(!message.contains("private_diagnostics"));
    assert!(!message.contains("request_echo"));
    assert!(!message.contains("must not reach the UI"));
}

#[tokio::test]
async fn unstructured_provider_error_body_keeps_the_safe_fallback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(502).set_body_string("upstream secret-shaped noise"))
        .mount(&server)
        .await;
    let backend = GenaiBackend::new(
        profile(&server),
        credentials(),
        GenaiBackendOptions::default(),
    )
    .unwrap();

    let error = backend.execute(request()).await.unwrap_err();
    assert_eq!(
        error,
        mathscape_llm::LlmError::Provider {
            status: Some(502),
            message: "provider rejected the request".to_string(),
        }
    );
}
