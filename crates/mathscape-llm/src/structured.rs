use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{LlmBackend, LlmError, LlmRequest, LlmResponse, LlmResponseFormat};

#[derive(Clone, Debug, PartialEq)]
pub struct TypedLlmResponse<T> {
    pub value: T,
    pub response: LlmResponse,
}

pub fn response_format_for<T: JsonSchema>(
    name: impl Into<String>,
    description: Option<String>,
) -> Result<LlmResponseFormat, LlmError> {
    let schema = serde_json::to_value(schemars::schema_for!(T)).map_err(|error| {
        LlmError::Configuration(format!("could not serialize response schema: {error}"))
    })?;
    Ok(LlmResponseFormat::JsonSchema {
        name: name.into(),
        description,
        schema,
    })
}

/// Execute a schema-constrained request and deserialize its textual JSON body.
/// Deterministic semantic validation remains the caller's responsibility.
pub async fn execute_typed<T: DeserializeOwned>(
    backend: &dyn LlmBackend,
    request: LlmRequest,
) -> Result<TypedLlmResponse<T>, LlmError> {
    if !matches!(
        request.response_format,
        LlmResponseFormat::JsonObject | LlmResponseFormat::JsonSchema { .. }
    ) {
        return Err(LlmError::InvalidRequest(
            "typed execution requires a JSON response format".to_string(),
        ));
    }
    let response = backend.execute(request).await?;
    let text = response.text().ok_or_else(|| {
        LlmError::InvalidResponse("structured response did not contain text".to_string())
    })?;
    let value = serde_json::from_str(&text).map_err(|error| {
        LlmError::InvalidResponse(format!("structured JSON could not be decoded: {error}"))
    })?;
    Ok(TypedLlmResponse { value, response })
}
