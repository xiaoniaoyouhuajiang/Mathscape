use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::WireProtocol;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmResponse {
    pub target: LlmResolvedTarget,
    pub parts: Vec<LlmResponsePart>,
    pub reasoning: Option<String>,
    pub finish_reason: Option<LlmFinishReason>,
    pub usage: LlmUsage,
    pub response_id: Option<String>,
    pub elapsed_ms: u64,
    pub raw_response: Option<Value>,
}

impl LlmResponse {
    pub fn text(&self) -> Option<String> {
        let texts: Vec<&str> = self
            .parts
            .iter()
            .filter_map(|part| match part {
                LlmResponsePart::Text { text } => Some(text.as_str()),
                LlmResponsePart::ToolCall { .. } => None,
            })
            .collect();
        (!texts.is_empty()).then(|| texts.join(""))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmResolvedTarget {
    pub profile_id: String,
    pub provider: String,
    pub wire_protocol: WireProtocol,
    pub requested_model: String,
    pub resolved_model: String,
    pub provider_model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmResponsePart {
    Text {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        continuation_tokens: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmFinishReason {
    Completed { raw: String },
    MaxTokens { raw: String },
    ToolCall { raw: String },
    ContentFilter { raw: String },
    StopSequence { raw: String },
    Other { raw: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
