//! Offline construction of the versioned discovery request. No model calls or writes.

use mathscape_domain::{
    AnchoredMarkdown, ContentHash, DiscoveryInput, DiscoveryProposal, ResolvedDiscoverySelection,
};
use mathscape_llm::{
    LlmGenerationOptions, LlmMessage, LlmRequest, LlmResponseFormat, LlmRole, LlmTask,
    LlmToolChoice,
};
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{DiscoveryInputError, DiscoveryInputLimits, build_discovery_input};

pub const DISCOVERY_SYSTEM_PROMPT: &str =
    include_str!("../../../prompts/source-discovery/v1/system.md");
pub const DISCOVERY_PROMPT_MANIFEST: &str =
    include_str!("../../../prompts/source-discovery/v1/manifest.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryPromptManifest {
    pub id: String,
    pub version: String,
    pub input_version: String,
    pub proposal_version: String,
    pub schema_name: String,
    pub system_prompt: String,
    pub output_schema: String,
    pub schema_dialect: String,
    pub schema_contract: String,
    pub max_output_tokens: u32,
    /// Producer must enforce this before decoding the returned proposal.
    pub max_response_bytes: usize,
}

pub fn discovery_prompt_manifest() -> DiscoveryPromptManifest {
    serde_json::from_str(DISCOVERY_PROMPT_MANIFEST).expect("embedded discovery manifest is valid")
}

/// Describes canonical producer output: nullable fields are present, all objects
/// closed, and internally tagged enum branches use anyOf with disjoint tags.
/// Domain Deserialize remains backward-compatible with omitted optional fields.
pub fn discovery_output_schema() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<DiscoveryProposal>();
    let mut schema = serde_json::to_value(schema).expect("schema is serializable");
    normalize_tagged_unions(&mut schema);
    schema
}

fn normalize_tagged_unions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            // This is specific to the Discovery DTO's internally tagged enums.
            // Distinct kind/status constants make these branches disjoint.
            if let Some(branches) = object.remove("oneOf") {
                object.insert("anyOf".into(), branches);
            }
            for value in object.values_mut() {
                normalize_tagged_unions(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_tagged_unions(value);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug)]
pub struct PreparedDiscoveryRequest {
    pub input: DiscoveryInput,
    pub request: LlmRequest,
    pub manifest: DiscoveryPromptManifest,
    /// Identity of manifest + system prompt + generated schema, not a complete
    /// run/cache key (model and generation parameters are separate).
    pub prompt_fingerprint: ContentHash,
}

/// Re-resolves the selection against the actual anchored document through the
/// input builder. Source bytes live only in a user message, never interpolated
/// into the system instruction. This function does not invoke an LLM.
pub fn build_discovery_request(
    document: &AnchoredMarkdown,
    selection: &ResolvedDiscoverySelection,
    limits: DiscoveryInputLimits,
) -> Result<PreparedDiscoveryRequest, DiscoveryInputError> {
    let input = build_discovery_input(document, selection, limits)?;
    let manifest = discovery_prompt_manifest();
    let schema = discovery_output_schema();
    let bundle = serde_json::to_vec(&(&manifest, DISCOVERY_SYSTEM_PROMPT, &schema))
        .expect("embedded prompt bundle is serializable");
    let prompt_fingerprint = ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bundle)))
        .expect("SHA-256 always produces a valid hash");
    let system = format!(
        "{}\n\n## 输出 JSON Schema\n{}",
        DISCOVERY_SYSTEM_PROMPT,
        serde_json::to_string(&schema).expect("schema is serializable")
    );
    let request = LlmRequest {
        task: LlmTask {
            kind: manifest.id.clone(),
            version: manifest.version.clone(),
            input_fingerprint: Some(input.fingerprint.clone()),
        },
        messages: vec![
            LlmMessage::text(LlmRole::System, system),
            LlmMessage::text(
                LlmRole::User,
                serde_json::to_string(&input).expect("input is serializable"),
            ),
        ],
        tools: vec![],
        tool_choice: LlmToolChoice::None,
        response_format: LlmResponseFormat::JsonSchema {
            name: manifest.schema_name.clone(),
            description: Some(
                "Source content discovery proposal; not canonical map entities".into(),
            ),
            schema,
        },
        options: LlmGenerationOptions {
            max_output_tokens: Some(manifest.max_output_tokens),
            ..Default::default()
        },
    };
    Ok(PreparedDiscoveryRequest {
        input,
        request,
        manifest,
        prompt_fingerprint,
    })
}
