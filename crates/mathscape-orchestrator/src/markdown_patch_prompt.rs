//! Provider-neutral request construction for the independent Markdown Patch
//! producer. The request contains only issue-local block context and a small
//! frozen UnitSet directory.

use mathscape_domain::{
    CanonicalDiscoveryIssue, ContentHash, DiscoveryInput, DiscoveryInputSegment,
    MARKDOWN_PATCH_INPUT_VERSION, MarkdownPatchBlock, MarkdownPatchInput, MarkdownPatchUnitContext,
};
use mathscape_llm::{
    LlmGenerationOptions, LlmMessage, LlmRequest, LlmResponseFormat, LlmRole, LlmTask,
    LlmToolChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::discovery_prompt::output_schema_for;
use crate::{PreparedUnitDiscoveryRequest, ValidatedUnitSet};

pub const MARKDOWN_PATCH_SYSTEM_PROMPT: &str =
    include_str!("../../../prompts/markdown-patch/v1/system.md");
pub const MARKDOWN_PATCH_PROMPT_MANIFEST: &str =
    include_str!("../../../prompts/markdown-patch/v1/manifest.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchPromptManifest {
    pub id: String,
    pub version: String,
    pub input_version: String,
    pub output_version: String,
    pub schema_name: String,
    pub max_output_tokens: u32,
    pub max_response_bytes: usize,
}

pub fn markdown_patch_prompt_manifest() -> MarkdownPatchPromptManifest {
    serde_json::from_str(MARKDOWN_PATCH_PROMPT_MANIFEST).expect("patch manifest is valid")
}

pub fn markdown_patch_output_schema() -> Value {
    let mut schema = output_schema_for::<mathscape_domain::MarkdownPatchResponse>();
    // The shared TextOccurrence DTO also supports a cross-block range for
    // Stage B. P v1 deliberately narrows its wire contract to one physical
    // block; the deterministic resolver remains the final authority.
    if let Some(branches) = schema
        .get_mut("$defs")
        .and_then(Value::as_object_mut)
        .and_then(|defs| defs.get_mut("TextOccurrence"))
        .and_then(Value::as_object_mut)
        .and_then(|occurrence| occurrence.get_mut("anyOf"))
        .and_then(Value::as_array_mut)
    {
        branches.retain(|branch| {
            branch
                .pointer("/properties/kind/const")
                .and_then(Value::as_str)
                == Some("text_occurrence")
        });
    }
    schema
}

#[derive(Clone, Debug)]
pub struct PreparedMarkdownPatchRequest {
    input: MarkdownPatchInput,
    request: LlmRequest,
    manifest: MarkdownPatchPromptManifest,
    prompt_fingerprint: ContentHash,
}

impl PreparedMarkdownPatchRequest {
    pub fn input(&self) -> &MarkdownPatchInput {
        &self.input
    }

    pub fn request(&self) -> &LlmRequest {
        &self.request
    }

    pub fn manifest(&self) -> &MarkdownPatchPromptManifest {
        &self.manifest
    }

    pub fn prompt_fingerprint(&self) -> &ContentHash {
        &self.prompt_fingerprint
    }
}

fn fingerprint<T: Serialize>(value: &T) -> ContentHash {
    ContentHash::parse(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("patch prompt serializes"))
    ))
    .expect("valid sha256")
}

fn block_context(
    input: &DiscoveryInput,
    issues: &[CanonicalDiscoveryIssue],
) -> Vec<MarkdownPatchBlock> {
    let blocks: Vec<_> = input
        .requested
        .segments
        .iter()
        .filter_map(|segment| match segment {
            DiscoveryInputSegment::Block {
                structural_kind,
                source_range,
                text,
                ..
            } => Some((structural_kind.clone(), source_range.clone(), text.clone())),
            DiscoveryInputSegment::Gap { .. } => None,
        })
        .collect();
    let mut selected = std::collections::BTreeSet::new();
    for issue in issues {
        for (index, (_, range, _)) in blocks.iter().enumerate() {
            if issue
                .evidence
                .iter()
                .any(|evidence| evidence.end > range.start && range.end > evidence.start)
            {
                let from = index.saturating_sub(mathscape_domain::MAX_PATCH_CONTEXT_BLOCKS_BEFORE);
                let to = (index + mathscape_domain::MAX_PATCH_CONTEXT_BLOCKS_AFTER + 1)
                    .min(blocks.len());
                selected.extend(from..to);
            }
        }
    }
    selected
        .into_iter()
        .map(|index| {
            let (structural_kind, _, text) = &blocks[index];
            MarkdownPatchBlock {
                block_ref: format!("b{}", index + 1),
                structural_kind: structural_kind.clone(),
                text: text.clone(),
            }
        })
        .collect()
}

pub fn build_markdown_patch_request(
    prepared: &PreparedUnitDiscoveryRequest,
    units: &ValidatedUnitSet,
) -> PreparedMarkdownPatchRequest {
    let issues = crate::canonical_discovery_issues(units);
    build_markdown_patch_request_with_issues(prepared, units, issues)
}

pub fn build_markdown_patch_request_with_issues(
    prepared: &PreparedUnitDiscoveryRequest,
    units: &ValidatedUnitSet,
    issues: Vec<CanonicalDiscoveryIssue>,
) -> PreparedMarkdownPatchRequest {
    let input = MarkdownPatchInput {
        version: MARKDOWN_PATCH_INPUT_VERSION.into(),
        request_ref: crate::new_request_ref("p"),
        base_snapshot: prepared.input().snapshot.clone(),
        input_fingerprint: prepared.input().fingerprint.clone(),
        blocks: block_context(prepared.input(), &issues),
        units: units
            .units()
            .iter()
            .map(|unit| MarkdownPatchUnitContext {
                unit_ref: unit.proposal().unit_ref.clone(),
                unit_kind: serde_json::to_value(unit.proposal().unit_kind)
                    .expect("unit kind serializes")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned(),
                parent: unit.proposal().parent.clone(),
                ranges: unit
                    .selectors()
                    .iter()
                    .map(|selector| selector.range().clone())
                    .collect(),
            })
            .collect(),
        issues,
    };
    let manifest = markdown_patch_prompt_manifest();
    let schema = markdown_patch_output_schema();
    let prompt_fingerprint = fingerprint(&(MARKDOWN_PATCH_SYSTEM_PROMPT, &manifest, &schema));
    let system = format!(
        "{}\n\n## JSON Schema\n{}",
        MARKDOWN_PATCH_SYSTEM_PROMPT,
        serde_json::to_string(&schema).expect("patch schema serializes")
    );
    let request = LlmRequest {
        task: LlmTask {
            kind: manifest.id.clone(),
            version: manifest.version.clone(),
            input_fingerprint: Some(input.input_fingerprint.clone()),
        },
        messages: vec![
            LlmMessage::text(LlmRole::System, system),
            LlmMessage::text(
                LlmRole::User,
                serde_json::to_string(&input).expect("patch input serializes"),
            ),
        ],
        tools: vec![],
        tool_choice: LlmToolChoice::None,
        response_format: LlmResponseFormat::JsonSchema {
            name: manifest.schema_name.clone(),
            description: Some("A bounded Markdown correction proposal".into()),
            schema,
        },
        options: LlmGenerationOptions {
            max_output_tokens: Some(manifest.max_output_tokens),
            ..Default::default()
        },
    };
    PreparedMarkdownPatchRequest {
        input,
        request,
        manifest,
        prompt_fingerprint,
    }
}
