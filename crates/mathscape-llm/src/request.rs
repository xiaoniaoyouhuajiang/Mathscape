use mathscape_domain::ContentHash;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmRequest {
    pub task: LlmTask,
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<LlmToolDefinition>,
    #[serde(default)]
    pub tool_choice: LlmToolChoice,
    #[serde(default)]
    pub response_format: LlmResponseFormat,
    #[serde(default)]
    pub options: LlmGenerationOptions,
}

impl LlmRequest {
    pub fn validate(&self) -> Result<(), RequestValidationError> {
        validate_name("task.kind", &self.task.kind, 96)?;
        validate_name("task.version", &self.task.version, 96)?;
        if self.messages.is_empty() {
            return Err(RequestValidationError::invalid(
                "messages",
                "at least one message is required",
            ));
        }
        for (index, message) in self.messages.iter().enumerate() {
            message.validate(index)?;
        }

        let mut names = BTreeSet::new();
        for tool in &self.tools {
            validate_name("tools.name", &tool.name, 64)?;
            if tool.description.trim().is_empty() {
                return Err(RequestValidationError::invalid(
                    "tools.description",
                    format!("tool '{}' has a blank description", tool.name),
                ));
            }
            require_object_schema("tools.input_schema", &tool.input_schema)?;
            if !names.insert(tool.name.as_str()) {
                return Err(RequestValidationError::invalid(
                    "tools.name",
                    format!("duplicate tool '{}'", tool.name),
                ));
            }
        }
        if let LlmToolChoice::Tool { name } = &self.tool_choice
            && !names.contains(name.as_str())
        {
            return Err(RequestValidationError::invalid(
                "tool_choice",
                format!("unknown tool '{name}'"),
            ));
        }
        if matches!(self.tool_choice, LlmToolChoice::Required) && self.tools.is_empty() {
            return Err(RequestValidationError::invalid(
                "tool_choice",
                "required tool choice needs at least one tool",
            ));
        }
        if let LlmResponseFormat::JsonSchema { name, schema, .. } = &self.response_format {
            validate_schema_name(name)?;
            require_object_schema("response_format.schema", schema)?;
        }
        validate_probability("options.temperature", self.options.temperature, 0.0, 2.0)?;
        validate_probability("options.top_p", self.options.top_p, 0.0, 1.0)?;
        if self.options.max_output_tokens == Some(0) {
            return Err(RequestValidationError::invalid(
                "options.max_output_tokens",
                "must be greater than zero",
            ));
        }
        if matches!(
            self.options.reasoning_effort,
            Some(LlmReasoningEffort::Budget(0))
        ) {
            return Err(RequestValidationError::invalid(
                "options.reasoning_effort",
                "reasoning budget must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmTask {
    pub kind: String,
    pub version: String,
    pub input_fingerprint: Option<ContentHash>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub parts: Vec<LlmMessagePart>,
}

impl LlmMessage {
    pub fn text(role: LlmRole, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![LlmMessagePart::Text { text: text.into() }],
        }
    }

    fn validate(&self, index: usize) -> Result<(), RequestValidationError> {
        if self.parts.is_empty() {
            return Err(RequestValidationError::invalid(
                "messages.parts",
                format!("message {index} has no parts"),
            ));
        }
        for part in &self.parts {
            match (&self.role, part) {
                (
                    LlmRole::System | LlmRole::User | LlmRole::Assistant,
                    LlmMessagePart::Text { text },
                ) => {
                    if text.is_empty() {
                        return Err(RequestValidationError::invalid(
                            "messages.parts.text",
                            format!("message {index} contains empty text"),
                        ));
                    }
                }
                (
                    LlmRole::Assistant,
                    LlmMessagePart::ToolCall {
                        call_id,
                        name,
                        arguments: _,
                        continuation_tokens: _,
                    },
                ) => {
                    validate_name("messages.parts.tool_call.call_id", call_id, 128)?;
                    validate_name("messages.parts.tool_call.name", name, 64)?;
                }
                (
                    LlmRole::Tool,
                    LlmMessagePart::ToolResult {
                        call_id,
                        name,
                        content: _,
                    },
                ) => {
                    validate_name("messages.parts.tool_result.call_id", call_id, 128)?;
                    if let Some(name) = name {
                        validate_name("messages.parts.tool_result.name", name, 64)?;
                    }
                }
                _ => {
                    return Err(RequestValidationError::invalid(
                        "messages.parts",
                        format!("message {index} contains a part incompatible with its role"),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmMessagePart {
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
    ToolResult {
        call_id: String,
        name: Option<String>,
        content: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub strict: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmToolChoice {
    #[default]
    Auto,
    None,
    Required,
    Tool {
        name: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmResponseFormat {
    #[default]
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        description: Option<String>,
        schema: Value,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmGenerationOptions {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    pub seed: Option<u64>,
    pub reasoning_effort: Option<LlmReasoningEffort>,
    #[serde(default)]
    pub capture_raw_response: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Budget(u32),
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid LLM request field '{field}': {message}")]
pub struct RequestValidationError {
    pub field: &'static str,
    pub message: String,
}

impl RequestValidationError {
    fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

fn validate_name(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), RequestValidationError> {
    if value.is_empty()
        || value.len() > max_len
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(RequestValidationError::invalid(
            field,
            format!("must be 1..={max_len} portable identifier characters"),
        ));
    }
    Ok(())
}

fn validate_schema_name(value: &str) -> Result<(), RequestValidationError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(RequestValidationError::invalid(
            "response_format.name",
            "must be 1..=64 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn require_object_schema(
    field: &'static str,
    schema: &Value,
) -> Result<(), RequestValidationError> {
    if !schema.is_object() {
        return Err(RequestValidationError::invalid(
            field,
            "must be a JSON object schema",
        ));
    }
    Ok(())
}

fn validate_probability(
    field: &'static str,
    value: Option<f64>,
    minimum: f64,
    maximum: f64,
) -> Result<(), RequestValidationError> {
    if value.is_some_and(|value| !value.is_finite() || !(minimum..=maximum).contains(&value)) {
        return Err(RequestValidationError::invalid(
            field,
            format!("must be finite and between {minimum} and {maximum}"),
        ));
    }
    Ok(())
}
