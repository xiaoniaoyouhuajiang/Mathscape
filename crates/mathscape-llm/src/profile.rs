use serde::{Deserialize, Serialize};
use url::Url;

use crate::CredentialRef;

pub const LLM_PROFILE_VERSION: &str = "mathscape-llm-profile-1";

/// Provider identity is descriptive; `wire_protocol` is authoritative for I/O.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmProfile {
    pub version: String,
    pub id: String,
    pub provider: String,
    pub wire_protocol: WireProtocol,
    pub model: String,
    /// Base URL including the provider API prefix, normally ending in `/`.
    /// `None` selects the adapter's official endpoint.
    pub endpoint: Option<Url>,
    /// A handle into an external credential store, never the credential itself.
    pub credential: Option<CredentialRef>,
    #[serde(default)]
    pub capabilities: LlmCapabilities,
}

impl LlmProfile {
    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        if self.version != LLM_PROFILE_VERSION {
            return Err(ProfileValidationError::Version(self.version.clone()));
        }
        validate_identifier("profile id", &self.id)?;
        validate_identifier("provider", &self.provider)?;
        if self.model.trim().is_empty() {
            return Err(ProfileValidationError::InvalidField {
                field: "model",
                message: "must not be blank".to_string(),
            });
        }
        if self.model.chars().any(char::is_control) {
            return Err(ProfileValidationError::InvalidField {
                field: "model",
                message: "must not contain control characters".to_string(),
            });
        }
        if let Some(endpoint) = &self.endpoint {
            if !matches!(endpoint.scheme(), "http" | "https") {
                return Err(ProfileValidationError::InvalidField {
                    field: "endpoint",
                    message: "only http and https endpoints are supported".to_string(),
                });
            }
            if !endpoint.username().is_empty() || endpoint.password().is_some() {
                return Err(ProfileValidationError::InvalidField {
                    field: "endpoint",
                    message: "credentials must not be embedded in the URL".to_string(),
                });
            }
            if endpoint.query().is_some() || endpoint.fragment().is_some() {
                return Err(ProfileValidationError::InvalidField {
                    field: "endpoint",
                    message: "query strings and fragments are not allowed".to_string(),
                });
            }
            if !endpoint.path().ends_with('/') {
                return Err(ProfileValidationError::InvalidField {
                    field: "endpoint",
                    message: "base URL path must end with '/'".to_string(),
                });
            }
        } else if !known_provider(&self.provider) {
            return Err(ProfileValidationError::InvalidField {
                field: "endpoint",
                message: "an unknown provider requires an explicit endpoint".to_string(),
            });
        }
        if let Some(reference) = &self.credential {
            crate::CredentialRef::new(reference.as_str()).map_err(|error| {
                ProfileValidationError::InvalidField {
                    field: "credential",
                    message: error.to_string(),
                }
            })?;
        }
        Ok(())
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ProfileValidationError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !value.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(ProfileValidationError::InvalidField {
            field,
            message: "must be 1..=64 ASCII letters, digits, '.', '-' or '_', starting with a letter or digit"
                .to_string(),
        });
    }
    Ok(())
}

fn known_provider(provider: &str) -> bool {
    matches!(
        provider,
        "openai"
            | "anthropic"
            | "gemini"
            | "deepseek"
            | "groq"
            | "openrouter"
            | "moonshot"
            | "aliyun"
            | "zai"
            | "xai"
            | "together"
            | "fireworks"
            | "aihubmix"
            | "ollama"
            | "minimax"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireProtocol {
    OpenAiChatCompletions,
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
    OllamaNative,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmCapabilities {
    #[serde(default)]
    pub structured_output: CapabilitySupport,
    #[serde(default)]
    pub tools: CapabilitySupport,
    #[serde(default)]
    pub streaming: CapabilitySupport,
    #[serde(default)]
    pub reasoning: CapabilitySupport,
    #[serde(default)]
    pub usage: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProfileValidationError {
    #[error("unsupported LLM profile version: {0}")]
    Version(String),
    #[error("invalid LLM profile field '{field}': {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
}
