use futures::StreamExt;
use genai::{
    Client, ModelIden,
    adapter::AdapterKind,
    chat::{
        ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, ChatStreamEvent, ContentPart,
        JsonSpec, MessageContent, ReasoningEffort, StopReason, StreamEnd, Tool, ToolCall,
        ToolChoice, ToolResponse,
    },
    resolver::{AuthData, Endpoint},
};
use secrecy::ExposeSecret;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    CapabilitySupport, CredentialResolver, LlmBackend, LlmError, LlmFinishReason, LlmFuture,
    LlmGenerationOptions, LlmMessage, LlmMessagePart, LlmProfile, LlmReasoningEffort, LlmRequest,
    LlmResolvedTarget, LlmResponse, LlmResponseFormat, LlmResponsePart, LlmRole, LlmToolChoice,
    LlmUsage, WireProtocol,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenaiBackendOptions {
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl Default for GenaiBackendOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

/// `genai` transport adapter. It performs one model call and never retries;
/// retry and repair budgets belong to the orchestrator.
pub struct GenaiBackend {
    profile: LlmProfile,
    client: Client,
    observer: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
}

impl GenaiBackend {
    pub fn new(
        profile: LlmProfile,
        credentials: Arc<dyn CredentialResolver>,
        options: GenaiBackendOptions,
    ) -> Result<Self, LlmError> {
        profile
            .validate()
            .map_err(|error| LlmError::Configuration(error.to_string()))?;
        if options.timeout.is_zero() || options.connect_timeout.is_zero() {
            return Err(LlmError::Configuration(
                "genai timeouts must be greater than zero".to_string(),
            ));
        }
        let adapter_kind = adapter_kind_for(&profile)?;
        let credential = profile.credential.clone();
        let auth_resolver = move |_model: ModelIden| {
            let auth = match &credential {
                Some(reference) => credentials
                    .resolve(reference)
                    .map(|secret| AuthData::from_single(secret.expose_secret().to_owned()))
                    .map_err(|error| {
                        genai::resolver::Error::Custom(format!("mathscape-credential:{error}"))
                    })?,
                None => AuthData::None,
            };
            Ok(Some(auth))
        };

        let web_config = genai::WebConfig::default()
            .with_timeout(options.timeout)
            .with_connect_timeout(options.connect_timeout);
        let mut builder = Client::builder()
            .with_adapter_kind(adapter_kind)
            .with_auth_resolver_fn(auth_resolver)
            .with_web_config(web_config);
        if let Some(endpoint) = profile.endpoint.clone() {
            builder =
                builder.with_service_target_resolver_fn(move |mut target: genai::ServiceTarget| {
                    target.endpoint = Endpoint::from_owned(endpoint.as_str().to_owned());
                    Ok(target)
                });
        }
        Ok(Self {
            profile,
            client: builder.build(),
            observer: None,
        })
    }

    /// Receives only provider-exposed text/reasoning deltas; no synthetic thinking.
    pub fn with_stream_observer(mut self, observer: Arc<dyn Fn(&str, &str) + Send + Sync>) -> Self {
        self.observer = Some(observer);
        self
    }

    async fn execute_inner(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        request
            .validate()
            .map_err(|error| LlmError::InvalidRequest(error.to_string()))?;
        check_declared_capabilities(&self.profile, &request)?;

        let chat_request = to_genai_request(&request)?;
        let options = to_genai_options(&request, &self.profile);
        let started = Instant::now();
        let response = self
            .client
            .exec_chat(self.profile.model.as_str(), chat_request, Some(&options))
            .await
            .map_err(map_genai_error)?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        from_genai_response(&self.profile, response, elapsed_ms)
    }

    async fn execute_stream_inner(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        request
            .validate()
            .map_err(|error| LlmError::InvalidRequest(error.to_string()))?;
        check_declared_capabilities(&self.profile, &request)?;
        if self.profile.capabilities.streaming == CapabilitySupport::Unsupported {
            return Err(LlmError::UnsupportedCapability("streaming".to_string()));
        }

        let chat_request = to_genai_request(&request)?;
        let options = to_genai_options(&request, &self.profile)
            .with_capture_content(true)
            .with_capture_reasoning_content(true)
            .with_capture_tool_calls(true)
            .with_capture_usage(should_capture_stream_usage(&self.profile));
        let started = Instant::now();
        let response = self
            .client
            .exec_chat_stream(self.profile.model.as_str(), chat_request, Some(&options))
            .await
            .map_err(map_genai_error)?;
        let model_iden = response.model_iden;
        let mut stream = response.stream;
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut streamed_tool_calls = Vec::new();
        let mut stream_end = None;

        while let Some(event) = stream.next().await {
            match event.map_err(map_genai_error)? {
                ChatStreamEvent::Start => {}
                ChatStreamEvent::Chunk(chunk) => {
                    if let Some(observer) = &self.observer {
                        observer("text", &chunk.content);
                    }
                    text.push_str(&chunk.content);
                }
                ChatStreamEvent::ReasoningChunk(chunk) => {
                    if let Some(observer) = &self.observer {
                        observer("reasoning", &chunk.content);
                    }
                    reasoning.push_str(&chunk.content);
                }
                ChatStreamEvent::ThoughtSignatureChunk(_) => {
                    // Provider continuation data is retained on captured tool
                    // calls at StreamEnd. Discovery does not issue tools.
                }
                ChatStreamEvent::ToolCallChunk(chunk) => {
                    upsert_streamed_tool_call(&mut streamed_tool_calls, chunk.tool_call);
                }
                ChatStreamEvent::End(end) => {
                    if let Some(observer) = &self.observer {
                        observer("validating", "");
                    }
                    stream_end = Some(end);
                    break;
                }
            }
        }

        if stream_end.is_none() {
            return Err(LlmError::Transport(
                "stream ended without a terminal event".into(),
            ));
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        from_genai_stream(
            &self.profile,
            model_iden,
            text,
            reasoning,
            streamed_tool_calls,
            stream_end,
            elapsed_ms,
        )
    }
}

fn should_capture_stream_usage(profile: &LlmProfile) -> bool {
    if profile.capabilities.usage == CapabilitySupport::Unsupported {
        return false;
    }
    // OpenAI Chat Completions requires `stream_options.include_usage` for
    // streamed usage. Compatibility relays often support `stream: true` but
    // reject this optional extension; only opt in when their profile declares
    // usage support explicitly. Other adapters capture usage from their native
    // event stream without this OpenAI-only request field.
    !matches!(profile.wire_protocol, WireProtocol::OpenAiChatCompletions)
        || profile.provider != "custom-relay"
        || profile.capabilities.usage == CapabilitySupport::Supported
}

impl LlmBackend for GenaiBackend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }

    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(self.execute_inner(request))
    }

    fn execute_stream<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(self.execute_stream_inner(request))
    }
}

fn adapter_kind_for(profile: &LlmProfile) -> Result<AdapterKind, LlmError> {
    let official = match (profile.provider.as_str(), profile.wire_protocol) {
        ("openai", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::OpenAI),
        ("openai", WireProtocol::OpenAiResponses) => Some(AdapterKind::OpenAIResp),
        ("anthropic", WireProtocol::AnthropicMessages) => Some(AdapterKind::Anthropic),
        ("gemini", WireProtocol::GeminiGenerateContent) => Some(AdapterKind::Gemini),
        ("ollama", WireProtocol::OllamaNative) => Some(AdapterKind::Ollama),
        ("deepseek", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::DeepSeek),
        ("groq", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Groq),
        ("openrouter", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::OpenRouter),
        ("moonshot", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Moonshot),
        ("aliyun", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Aliyun),
        ("zai", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Zai),
        ("xai", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Xai),
        ("together", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Together),
        ("fireworks", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Fireworks),
        ("aihubmix", WireProtocol::OpenAiChatCompletions) => Some(AdapterKind::Aihubmix),
        ("minimax", WireProtocol::AnthropicMessages) => Some(AdapterKind::MiniMax),
        _ => None,
    };
    if let Some(adapter) = official {
        return Ok(adapter);
    }
    if profile.endpoint.is_none() {
        return Err(LlmError::Configuration(format!(
            "provider '{}' does not have an official endpoint for protocol {:?}",
            profile.provider, profile.wire_protocol
        )));
    }
    match profile.wire_protocol {
        WireProtocol::OpenAiChatCompletions => Ok(AdapterKind::OpenAI),
        WireProtocol::OpenAiResponses => Ok(AdapterKind::OpenAIResp),
        WireProtocol::AnthropicMessages => Ok(AdapterKind::Anthropic),
        WireProtocol::GeminiGenerateContent => Ok(AdapterKind::Gemini),
        WireProtocol::OllamaNative => Ok(AdapterKind::Ollama),
    }
}

fn check_declared_capabilities(profile: &LlmProfile, request: &LlmRequest) -> Result<(), LlmError> {
    let structured = matches!(
        request.response_format,
        LlmResponseFormat::JsonObject | LlmResponseFormat::JsonSchema { .. }
    );
    if structured && profile.capabilities.structured_output == CapabilitySupport::Unsupported {
        return Err(LlmError::UnsupportedCapability(
            "structured_output".to_string(),
        ));
    }
    if !request.tools.is_empty() && profile.capabilities.tools == CapabilitySupport::Unsupported {
        return Err(LlmError::UnsupportedCapability("tools".to_string()));
    }
    if request.options.reasoning_effort.is_some()
        && profile.capabilities.reasoning == CapabilitySupport::Unsupported
    {
        return Err(LlmError::UnsupportedCapability("reasoning".to_string()));
    }
    Ok(())
}

fn to_genai_request(request: &LlmRequest) -> Result<ChatRequest, LlmError> {
    let messages = request
        .messages
        .iter()
        .map(to_genai_message)
        .collect::<Result<Vec<_>, _>>()?;
    let mut chat_request = ChatRequest::new(messages);
    if !request.tools.is_empty() {
        chat_request = chat_request.with_tools(request.tools.iter().map(|tool| {
            Tool::new(tool.name.clone())
                .with_description(tool.description.clone())
                .with_schema(tool.input_schema.clone())
                .with_strict(tool.strict)
        }));
    }
    Ok(chat_request)
}

fn to_genai_message(message: &LlmMessage) -> Result<ChatMessage, LlmError> {
    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            LlmMessagePart::Text { text } => Ok(ContentPart::Text(text.clone())),
            LlmMessagePart::ToolCall {
                call_id,
                name,
                arguments,
                continuation_tokens,
            } => Ok(ContentPart::ToolCall(ToolCall {
                call_id: call_id.clone(),
                fn_name: name.clone(),
                fn_arguments: arguments.clone(),
                thought_signatures: (!continuation_tokens.is_empty())
                    .then(|| continuation_tokens.clone()),
            })),
            LlmMessagePart::ToolResult {
                call_id,
                name,
                content,
            } => {
                let content = match content {
                    serde_json::Value::String(text) => text.clone(),
                    value => serde_json::to_string(value).map_err(|error| {
                        LlmError::InvalidRequest(format!(
                            "tool result is not serializable: {error}"
                        ))
                    })?,
                };
                Ok(ContentPart::ToolResponse(ToolResponse {
                    call_id: call_id.clone(),
                    fn_name: name.clone(),
                    content,
                }))
            }
        })
        .collect::<Result<Vec<_>, LlmError>>()?;
    let content = MessageContent::from_parts(parts);
    Ok(match message.role {
        LlmRole::System => ChatMessage::system(content),
        LlmRole::User => ChatMessage::user(content),
        LlmRole::Assistant => ChatMessage::assistant(content),
        LlmRole::Tool => ChatMessage::tool(content),
    })
}

fn to_genai_options(request: &LlmRequest, profile: &LlmProfile) -> ChatOptions {
    let LlmGenerationOptions {
        temperature,
        top_p,
        max_output_tokens,
        stop_sequences,
        seed,
        reasoning_effort,
        capture_raw_response,
    } = &request.options;
    let mut options = ChatOptions::default().with_capture_raw_body(*capture_raw_response);
    if let Some(value) = temperature {
        options = options.with_temperature(*value);
    }
    if let Some(value) = top_p {
        options = options.with_top_p(*value);
    }
    if let Some(value) = max_output_tokens {
        options = options.with_max_tokens(*value);
    }
    if !stop_sequences.is_empty() {
        options = options.with_stop_sequences(stop_sequences.clone());
    }
    if let Some(value) = seed {
        options = options.with_seed(*value);
    }
    if let Some(value) = reasoning_effort {
        options = options.with_reasoning_effort(to_genai_reasoning(*value));
    }
    if !request.tools.is_empty() {
        options = options.with_tool_choice(match &request.tool_choice {
            LlmToolChoice::Auto => ToolChoice::Auto,
            LlmToolChoice::None => ToolChoice::None,
            LlmToolChoice::Required => ToolChoice::Required,
            LlmToolChoice::Tool { name } => ToolChoice::tool(name.clone()),
        });
    }
    match &request.response_format {
        LlmResponseFormat::Text => options,
        LlmResponseFormat::JsonObject => options.with_response_format(ChatResponseFormat::JsonMode),
        LlmResponseFormat::JsonSchema {
            name,
            description,
            schema,
        } => {
            // DeepSeek's Chat Completions endpoint currently supports JSON
            // mode (`json_object`), but rejects OpenAI's structured-output
            // (`json_schema`) request shape.  The canonical schema remains
            // in the prompt and is still enforced by the local deserializer
            // and deterministic validators, so this is a transport-level
            // compatibility downgrade only.
            if json_schema_requires_json_mode_fallback(profile) {
                options.with_response_format(ChatResponseFormat::JsonMode)
            } else {
                let schema = lower_strict_schema_for_wire_protocol(schema, profile.wire_protocol);
                let mut spec = JsonSpec::new(name.clone(), schema);
                if let Some(description) = description {
                    spec = spec.with_description(description.clone());
                }
                options.with_response_format(spec)
            }
        }
    }
}

fn json_schema_requires_json_mode_fallback(profile: &LlmProfile) -> bool {
    profile.provider == "deepseek" && profile.wire_protocol == WireProtocol::OpenAiChatCompletions
}

/// Lowers the canonical schema to the portable subset accepted by strict
/// OpenAI-compatible response formats. Draft 2020-12 permits annotation
/// siblings next to `$ref`, but OpenAI-compatible strict decoders can reject
/// them. Assertions are deliberately retained; only non-validating metadata
/// is removed. The canonical schema owned by the caller is never mutated.
fn lower_strict_schema_for_wire_protocol(
    schema: &serde_json::Value,
    wire_protocol: WireProtocol,
) -> serde_json::Value {
    let mut schema = schema.clone();
    if matches!(
        wire_protocol,
        WireProtocol::OpenAiChatCompletions | WireProtocol::OpenAiResponses
    ) {
        strip_ref_annotation_siblings(&mut schema);
    }
    schema
}

fn strip_ref_annotation_siblings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("$ref") {
                for annotation in [
                    "title",
                    "description",
                    "$comment",
                    "default",
                    "examples",
                    "deprecated",
                    "readOnly",
                    "writeOnly",
                ] {
                    object.remove(annotation);
                }
            }
            for child in object.values_mut() {
                strip_ref_annotation_siblings(child);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                strip_ref_annotation_siblings(child);
            }
        }
        _ => {}
    }
}

fn to_genai_reasoning(value: LlmReasoningEffort) -> ReasoningEffort {
    match value {
        LlmReasoningEffort::None => ReasoningEffort::None,
        LlmReasoningEffort::Minimal => ReasoningEffort::Minimal,
        LlmReasoningEffort::Low => ReasoningEffort::Low,
        LlmReasoningEffort::Medium => ReasoningEffort::Medium,
        LlmReasoningEffort::High => ReasoningEffort::High,
        LlmReasoningEffort::XHigh => ReasoningEffort::XHigh,
        LlmReasoningEffort::Max => ReasoningEffort::Max,
        LlmReasoningEffort::Budget(value) => ReasoningEffort::Budget(value),
    }
}

fn from_genai_response(
    profile: &LlmProfile,
    response: genai::chat::ChatResponse,
    elapsed_ms: u64,
) -> Result<LlmResponse, LlmError> {
    let mut parts = Vec::new();
    let mut additional_reasoning = Vec::new();
    for part in response.content.clone() {
        match part {
            ContentPart::Text(text) => parts.push(LlmResponsePart::Text { text }),
            ContentPart::ToolCall(call) => parts.push(LlmResponsePart::ToolCall {
                call_id: call.call_id,
                name: call.fn_name,
                arguments: call.fn_arguments,
                continuation_tokens: call.thought_signatures.unwrap_or_default(),
            }),
            ContentPart::ReasoningContent(reasoning) => additional_reasoning.push(reasoning),
            ContentPart::ThoughtSignature(_) => {
                return Err(LlmError::InvalidResponse(
                    "provider returned an unassociated continuation token".to_string(),
                ));
            }
            ContentPart::Binary(_) | ContentPart::ToolResponse(_) | ContentPart::Custom(_) => {
                return Err(LlmError::InvalidResponse(
                    "provider returned an unsupported response content part".to_string(),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(LlmError::InvalidResponse(
            "provider returned neither text nor tool calls".to_string(),
        ));
    }
    let reasoning = match (response.reasoning_content, additional_reasoning.is_empty()) {
        (Some(reasoning), true) => Some(reasoning),
        (Some(reasoning), false) => Some(
            std::iter::once(reasoning)
                .chain(additional_reasoning)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        (None, false) => Some(additional_reasoning.join("\n")),
        (None, true) => None,
    };
    let target = LlmResolvedTarget {
        profile_id: profile.id.clone(),
        provider: profile.provider.clone(),
        wire_protocol: profile.wire_protocol,
        requested_model: profile.model.clone(),
        resolved_model: response.model_iden.model_name.to_string(),
        provider_model: response.provider_model_iden.model_name.to_string(),
    };
    Ok(LlmResponse {
        target,
        parts,
        reasoning,
        finish_reason: response.stop_reason.map(map_stop_reason),
        usage: map_usage(&response.usage),
        response_id: response.response_id,
        elapsed_ms,
        raw_response: response.captured_raw_body,
    })
}

fn from_genai_stream(
    profile: &LlmProfile,
    model_iden: ModelIden,
    mut text: String,
    mut reasoning: String,
    mut streamed_tool_calls: Vec<ToolCall>,
    stream_end: Option<StreamEnd>,
    elapsed_ms: u64,
) -> Result<LlmResponse, LlmError> {
    let mut usage = LlmUsage::default();
    let mut finish_reason = None;
    let mut response_id = None;

    if let Some(end) = stream_end {
        if let Some(end_usage) = end.captured_usage.as_ref() {
            usage = map_usage(end_usage);
        }
        finish_reason = end.captured_stop_reason.map(map_stop_reason);
        response_id = end.captured_response_id;
        if reasoning.is_empty()
            && let Some(end_reasoning) = end.captured_reasoning_content
        {
            reasoning = end_reasoning;
        }
        if let Some(content) = end.captured_content {
            let mut captured_text = String::new();
            let mut captured_tool_calls = Vec::new();
            for part in content.into_parts() {
                match part {
                    ContentPart::Text(part) => captured_text.push_str(&part),
                    ContentPart::ToolCall(call) => captured_tool_calls.push(call),
                    ContentPart::ThoughtSignature(_)
                    | ContentPart::ReasoningContent(_)
                    | ContentPart::Binary(_)
                    | ContentPart::ToolResponse(_)
                    | ContentPart::Custom(_) => {}
                }
            }
            if text.is_empty() {
                text = captured_text;
            }
            if !captured_tool_calls.is_empty() {
                streamed_tool_calls = captured_tool_calls;
            }
        }
    }

    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(LlmResponsePart::Text { text });
    }
    parts.extend(streamed_tool_calls.into_iter().map(|call| {
        let ToolCall {
            call_id,
            fn_name,
            fn_arguments,
            thought_signatures,
        } = call;
        LlmResponsePart::ToolCall {
            call_id,
            name: fn_name,
            arguments: normalize_streamed_arguments(fn_arguments),
            continuation_tokens: thought_signatures.unwrap_or_default(),
        }
    }));
    if parts.is_empty() {
        return Err(LlmError::InvalidResponse(
            "provider stream returned neither text nor tool calls".to_string(),
        ));
    }

    let provider_model = model_iden.model_name.to_string();
    Ok(LlmResponse {
        target: LlmResolvedTarget {
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            wire_protocol: profile.wire_protocol,
            requested_model: profile.model.clone(),
            resolved_model: provider_model.clone(),
            provider_model,
        },
        parts,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        finish_reason,
        usage,
        response_id,
        elapsed_ms,
        // Raw SSE frames can contain provider-private diagnostics and are not
        // retained. The normalized final response is sufficient for Discovery.
        raw_response: None,
    })
}

fn upsert_streamed_tool_call(tool_calls: &mut Vec<ToolCall>, call: ToolCall) {
    if let Some(existing) = tool_calls
        .iter_mut()
        .find(|existing| existing.call_id == call.call_id)
    {
        *existing = call;
    } else {
        tool_calls.push(call);
    }
}

fn normalize_streamed_arguments(arguments: serde_json::Value) -> serde_json::Value {
    match arguments {
        serde_json::Value::String(value) => {
            serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
        }
        value => value,
    }
}

fn map_stop_reason(reason: StopReason) -> LlmFinishReason {
    let raw = reason.raw().to_string();
    match reason {
        StopReason::Completed(_) => LlmFinishReason::Completed { raw },
        StopReason::MaxTokens(_) => LlmFinishReason::MaxTokens { raw },
        StopReason::ToolCall(_) => LlmFinishReason::ToolCall { raw },
        StopReason::ContentFilter(_) => LlmFinishReason::ContentFilter { raw },
        StopReason::StopSequence(_) => LlmFinishReason::StopSequence { raw },
        StopReason::Other(_) => LlmFinishReason::Other { raw },
    }
}

fn map_usage(usage: &genai::chat::Usage) -> LlmUsage {
    LlmUsage {
        input_tokens: nonnegative(usage.prompt_tokens),
        output_tokens: nonnegative(usage.completion_tokens),
        total_tokens: nonnegative(usage.total_tokens),
        cached_input_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| nonnegative(details.cached_tokens)),
        reasoning_tokens: usage
            .completion_tokens_details
            .as_ref()
            .and_then(|details| nonnegative(details.reasoning_tokens)),
    }
}

fn nonnegative(value: Option<i32>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

fn map_genai_error(error: genai::Error) -> LlmError {
    match error {
        genai::Error::RequiresApiKey { .. }
        | genai::Error::NoAuthResolver { .. }
        | genai::Error::NoAuthData { .. } => {
            LlmError::Authentication("provider requires an API key".to_string())
        }
        genai::Error::Resolver { resolver_error, .. } => match resolver_error {
            genai::resolver::Error::Custom(message) => {
                match message.strip_prefix("mathscape-credential:") {
                    Some(message) => LlmError::Credential(crate::CredentialError::Unavailable(
                        message.to_string(),
                    )),
                    None => LlmError::Configuration(message),
                }
            }
            other => LlmError::Configuration(other.to_string()),
        },
        genai::Error::WebAdapterCall { webc_error, .. }
        | genai::Error::WebModelCall { webc_error, .. } => map_web_error(webc_error),
        genai::Error::HttpError { status, .. } => map_status(status.as_u16(), None, None),
        genai::Error::AdapterNotSupported { feature, .. } => {
            LlmError::UnsupportedCapability(feature)
        }
        genai::Error::ChatResponseGeneration { cause, .. } => LlmError::InvalidResponse(cause),
        genai::Error::NoChatResponse { .. }
        | genai::Error::InvalidJsonResponseElement { .. }
        | genai::Error::ChatResponse { .. }
        | genai::Error::StreamParse { .. }
        | genai::Error::SerdeJson(_) => LlmError::InvalidResponse(error.to_string()),
        genai::Error::WebStream { error, .. } => map_stream_error(error),
        genai::Error::ChatReqHasNoMessages { .. }
        | genai::Error::LastChatMessageIsNotUser { .. }
        | genai::Error::MessageRoleNotSupported { .. }
        | genai::Error::MessageContentTypeNotSupported { .. }
        | genai::Error::JsonModeWithoutInstruction
        | genai::Error::VerbosityParsing { .. }
        | genai::Error::ReasoningParsingError { .. }
        | genai::Error::ServiceTierParsing { .. }
        | genai::Error::PromptCacheRetentionParsing { .. } => {
            LlmError::InvalidRequest(error.to_string())
        }
        _ => LlmError::Provider {
            status: None,
            message: error.to_string(),
        },
    }
}

fn map_stream_error(error: genai::BoxError) -> LlmError {
    match error.downcast::<genai::Error>() {
        Ok(error) => map_genai_error(*error),
        Err(error) => LlmError::Transport(error.to_string()),
    }
}

fn map_web_error(error: genai::webc::Error) -> LlmError {
    match error {
        genai::webc::Error::ResponseFailedStatus {
            status,
            body,
            headers,
        } => {
            let retry_after_ms = headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .and_then(|seconds| seconds.checked_mul(1000));
            map_status(
                status.as_u16(),
                retry_after_ms,
                provider_error_summary(&body),
            )
        }
        genai::webc::Error::Reqwest(error) if error.is_timeout() => LlmError::Timeout,
        genai::webc::Error::Reqwest(error) => LlmError::Transport(error.to_string()),
        genai::webc::Error::ResponseFailedNotJson { .. }
        | genai::webc::Error::ResponseFailedInvalidJson { .. }
        | genai::webc::Error::JsonValueExt(_) => LlmError::InvalidResponse(error.to_string()),
    }
}

fn map_status(
    status: u16,
    retry_after_ms: Option<u64>,
    provider_message: Option<String>,
) -> LlmError {
    match status {
        401 | 403 => LlmError::Authentication(
            provider_message.unwrap_or_else(|| format!("provider returned HTTP {status}")),
        ),
        429 => LlmError::RateLimited { retry_after_ms },
        524 => LlmError::Provider {
            status: Some(status),
            message: provider_message.unwrap_or_else(|| {
                "upstream gateway timed out while waiting for the model response".to_string()
            }),
        },
        _ => LlmError::Provider {
            status: Some(status),
            message: provider_message
                .unwrap_or_else(|| "provider rejected the request".to_string()),
        },
    }
}

const PROVIDER_MESSAGE_MAX_CHARS: usize = 1024;
const PROVIDER_FIELD_MAX_CHARS: usize = 128;

/// Extracts only conventional scalar error fields. Arbitrary response bodies
/// are not surfaced because gateways may include request echoes or diagnostics
/// that do not belong in UI messages and logs.
fn provider_error_summary(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = value.get("error").unwrap_or(&value);
    if let Some(message) = error.as_str() {
        return bounded_provider_text(message, PROVIDER_MESSAGE_MAX_CHARS);
    }
    let object = error.as_object()?;
    let mut fields = Vec::new();
    for (name, limit) in [
        ("code", PROVIDER_FIELD_MAX_CHARS),
        ("param", PROVIDER_FIELD_MAX_CHARS),
        ("message", PROVIDER_MESSAGE_MAX_CHARS),
    ] {
        let Some(value) = object.get(name).and_then(provider_scalar) else {
            continue;
        };
        let Some(value) = bounded_provider_text(&value, limit) else {
            continue;
        };
        fields.push(format!("{name}={value}"));
    }
    (!fields.is_empty()).then(|| fields.join(", "))
}

fn provider_scalar(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn bounded_provider_text(value: &str, limit: usize) -> Option<String> {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().count() <= limit {
        return Some(normalized.to_string());
    }
    let mut bounded = normalized.chars().take(limit).collect::<String>();
    bounded.push('…');
    Some(bounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_protocol_schema_is_not_lowered() {
        let schema = json!({
            "$defs": {"Value": {"type": "string"}},
            "type": "object",
            "properties": {
                "value": {
                    "$ref": "#/$defs/Value",
                    "description": "Provider-visible field guidance"
                }
            }
        });

        for protocol in [
            WireProtocol::AnthropicMessages,
            WireProtocol::GeminiGenerateContent,
            WireProtocol::OllamaNative,
        ] {
            assert_eq!(
                lower_strict_schema_for_wire_protocol(&schema, protocol),
                schema
            );
        }
    }

    #[test]
    fn provider_error_summary_is_bounded_and_single_line() {
        let long_message = format!(
            "first line\n{}",
            "x".repeat(PROVIDER_MESSAGE_MAX_CHARS + 50)
        );
        let body = json!({"error": {"message": long_message}}).to_string();
        let summary = provider_error_summary(&body).unwrap();

        assert!(!summary.contains('\n'));
        assert!(summary.ends_with('…'));
        assert_eq!(
            summary.chars().count(),
            "message=".chars().count() + PROVIDER_MESSAGE_MAX_CHARS + 1
        );
    }
}
