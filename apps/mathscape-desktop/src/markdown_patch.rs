use std::collections::{BTreeMap, BTreeSet, HashSet};

use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    ActorRef, AnchoredMarkdown, ContentHash, DiscoveryInput, EntityKind, LifecycleStatus,
    MarkdownPatchApplyReceipt, MarkdownPatchDecision, MarkdownPatchDecisionKind, MarkdownPatchSet,
    MarkdownPatchStatus, MarkdownPdfBlockMap, SourcePayload, StableId, UnitDiscoveryCheckpoint,
};
use mathscape_llm::{GenaiBackend, GenaiBackendOptions};
use mathscape_orchestrator::markdown_patch::resolve_patch_proposals;
use mathscape_orchestrator::markdown_patch::{
    anchor_patched_markdown, apply_patch_set, evaluate_patch_safety, markdown_patch_artifact_id,
    markdown_patch_content_hash, markdown_patch_conversion_id, promote_applied_patch,
    rewrite_patch_set_replacements, select_patch_set,
};
use mathscape_orchestrator::{
    MarkdownPatchProducer, MarkdownPatchProductionOutcome, MarkdownPdfMapInheritance,
    PreparedUnitDiscoveryRequest, ValidatedUnitSet, build_markdown_patch_request_with_issues,
    canonical_discovery_issues, canonical_discovery_issues_for_checkpoint,
    prepare_unit_discovery_request_from_input, rebind_discovery_input,
    rebind_discovery_input_after_patch,
};
use mathscape_store_sqlite::{ArtifactCatalogSnapshot, SqliteArtifactStore};
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;

use crate::backlink::conversion_has_pdf_lineage;
use crate::{DesktopService, DesktopState};

/// Patch decisions and drafts record their actor as a `StableId`, which needs
/// a URI-like scheme unlike the free-form `ActorRef`.
const DESKTOP_PATCH_ACTOR: &str = "mathscape:actor/desktop-user";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMarkdownPatchRequest {
    pub run_id: String,
    pub profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopMarkdownPatchReview {
    pub run_id: String,
    pub patch_set_id: Option<String>,
    pub input_fingerprint: ContentHash,
    pub status: String,
    pub patch_set: Option<MarkdownPatchSet>,
    pub provider_model: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewMarkdownPatchRequest {
    pub run_id: String,
    pub patch_set_id: String,
    #[serde(default)]
    pub replacements: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopMarkdownPatchPreview {
    pub run_id: String,
    pub patch_set_id: String,
    pub patch_set_fingerprint: ContentHash,
    pub original: String,
    pub revised: String,
    pub safety: mathscape_domain::PatchSafetyReport,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitMarkdownPatchDecisionRequest {
    pub run_id: String,
    pub patch_set_id: String,
    pub expected_patch_fingerprint: ContentHash,
    #[serde(default)]
    pub accepted_patch_refs: Vec<String>,
    #[serde(default)]
    pub replacements: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum DesktopMarkdownPatchError {
    #[error("Markdown patch run does not exist")]
    MissingRun,
    #[error("stored discovery checkpoint is invalid: {0}")]
    Checkpoint(String),
    #[error("Markdown patch input is stale: {0}")]
    Input(String),
    #[error("Markdown patch provider failed: {0}")]
    Provider(String),
    #[error("Markdown patch response was rejected: {0}")]
    Rejected(String),
    #[error("Markdown patch validation failed: {0}")]
    Validation(String),
    #[error(transparent)]
    Store(#[from] mathscape_store_sqlite::SqliteStoreError),
    #[error(transparent)]
    Artifact(#[from] mathscape_artifact::ArtifactPoolError),
    #[error(transparent)]
    Promotion(#[from] mathscape_orchestrator::markdown_patch::MarkdownPatchPromotionError),
    #[error(transparent)]
    Domain(#[from] mathscape_domain::DomainError),
    #[error(transparent)]
    Settings(#[from] crate::llm_settings::LlmSettingsError),
    #[error(transparent)]
    WorkspaceSettings(#[from] crate::workspace_settings::WorkspaceSettingsError),
    #[error(transparent)]
    Backend(#[from] mathscape_llm::LlmError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn checkpoint_value<T: serde::de::DeserializeOwned>(
    snapshot: &serde_json::Value,
    field: &'static str,
) -> Result<T, DesktopMarkdownPatchError> {
    let value = snapshot
        .get(field)
        .cloned()
        .ok_or_else(|| DesktopMarkdownPatchError::Checkpoint(format!("missing {field}")))?;
    serde_json::from_value(value).map_err(Into::into)
}

fn fingerprint_id(
    kind: &str,
    fingerprint: &ContentHash,
) -> Result<StableId, mathscape_domain::DomainError> {
    let digest = fingerprint
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(fingerprint.as_str());
    StableId::parse(format!("mathscape:{kind}/{digest}"))
}

pub(crate) fn patch_set_id(
    patch_set: &MarkdownPatchSet,
) -> Result<StableId, mathscape_domain::DomainError> {
    fingerprint_id("markdown-patch-set", &patch_set.fingerprint)
}

fn patch_set_from_store(
    store: &SqliteArtifactStore,
    patch_set_id: &StableId,
) -> Result<MarkdownPatchSet, DesktopMarkdownPatchError> {
    let value = store
        .markdown_patch_set_json(patch_set_id)?
        .ok_or_else(|| DesktopMarkdownPatchError::Checkpoint("patch set not found".into()))?;
    let patch_set: MarkdownPatchSet = serde_json::from_value(value)?;
    let drafts = store.markdown_patch_drafts(patch_set_id)?;
    rewrite_patch_set_replacements(&patch_set, &drafts)
        .map_err(DesktopMarkdownPatchError::Validation)
}

fn read_json_artifact(
    pool: &ArtifactPool,
    catalog: &ArtifactCatalogSnapshot,
    artifact_id: &StableId,
) -> Result<serde_json::Value, DesktopMarkdownPatchError> {
    let record = catalog
        .artifacts()
        .iter()
        .find(|record| record.entity().header().stable_id() == artifact_id)
        .ok_or_else(|| {
            DesktopMarkdownPatchError::Checkpoint(format!("页映射 Artifact {} 不存在", artifact_id))
        })?;
    let byte_size = record.entity().payload().byte_size();
    let mut bytes = Vec::with_capacity(
        usize::try_from(byte_size)
            .map_err(|_| DesktopMarkdownPatchError::Checkpoint("页映射过大".into()))?,
    );
    let mut offset = 0_u64;
    while offset < byte_size {
        let length = (byte_size - offset).min(4 * 1024 * 1024) as u32;
        let chunk = pool.read_bytes(artifact_id, offset, length)?;
        if chunk.len() != length as usize {
            return Err(DesktopMarkdownPatchError::Checkpoint(
                "页映射 Artifact 长度已改变".into(),
            ));
        }
        bytes.extend(chunk);
        offset += u64::from(length);
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| DesktopMarkdownPatchError::Checkpoint(format!("页映射损坏：{error}")))
}

fn markdown_pdf_map_inheritance(
    catalog: &ArtifactCatalogSnapshot,
    pool: &ArtifactPool,
    markdown_artifact_id: &StableId,
    requested_conversion_id: Option<&StableId>,
) -> Result<Option<MarkdownPdfMapInheritance>, DesktopMarkdownPatchError> {
    let conversion = catalog
        .conversions()
        .iter()
        .filter(|conversion| {
            requested_conversion_id.is_none_or(|id| conversion.header().stable_id() == id)
        })
        .find(|conversion| {
            conversion.endpoints().iter().any(|endpoint| {
                endpoint.role().as_str() == "output" && endpoint.entity() == markdown_artifact_id
            })
        });
    let Some(conversion) = conversion else {
        return Ok(None);
    };
    let SourcePayload::ConversionRun(run) = conversion.payload() else {
        return Ok(None);
    };
    let Some(map_id) = run
        .parameters()
        .additional_options()
        .get(mathscape_orchestrator::MARKDOWN_PDF_MAP_ARTIFACT_KEY)
        .and_then(|value| StableId::parse(value).ok())
    else {
        return Ok(None);
    };
    let value = read_json_artifact(pool, catalog, &map_id)?;
    let map: MarkdownPdfBlockMap = serde_json::from_value(value)?;
    if map.conversion_id != *conversion.header().stable_id()
        || map.markdown_artifact_id != *markdown_artifact_id
        || !conversion_has_pdf_lineage(
            catalog,
            conversion.header().stable_id(),
            markdown_artifact_id,
            &map.pdf_artifact_id,
            &mut BTreeSet::new(),
        )
    {
        return Err(DesktopMarkdownPatchError::Checkpoint(
            "基础 Markdown 的 PDF 页映射与转换身份不一致".into(),
        ));
    }
    Ok(Some(MarkdownPdfMapInheritance {
        base_conversion_id: conversion.header().stable_id().clone(),
        pdf_artifact_id: map.pdf_artifact_id,
        marker_page_ids: map.marker_page_ids,
    }))
}

fn replay_patch_receipt(
    patch_set: &MarkdownPatchSet,
    application: mathscape_store_sqlite::StoredPatchApplication,
) -> Result<MarkdownPatchApplyReceipt, DesktopMarkdownPatchError> {
    let status = match application.status.as_str() {
        "applied" => MarkdownPatchStatus::Applied,
        "rejected" => MarkdownPatchStatus::Rejected,
        "failed" => MarkdownPatchStatus::Failed,
        "stale" => MarkdownPatchStatus::Stale,
        _ => {
            return Err(DesktopMarkdownPatchError::Checkpoint(
                "stored Markdown patch application has an invalid status".into(),
            ));
        }
    };
    Ok(MarkdownPatchApplyReceipt {
        version: mathscape_domain::MARKDOWN_PATCH_RECEIPT_VERSION.into(),
        run_id: patch_set.run_id.clone(),
        patch_set_fingerprint: patch_set.fingerprint.clone(),
        base_artifact_id: patch_set.base_snapshot.artifact_id.clone(),
        patched_artifact_id: application
            .patched_artifact_id
            .map(StableId::parse)
            .transpose()?,
        conversion_id: application.conversion_id.map(StableId::parse).transpose()?,
        old_input_fingerprint: ContentHash::parse(application.old_input_fingerprint)?,
        new_input_fingerprint: application
            .new_input_fingerprint
            .map(ContentHash::parse)
            .transpose()?,
        safety: serde_json::from_str(&application.safety_json)?,
        status,
    })
}

struct PatchContext {
    source: DiscoveryInput,
    document: AnchoredMarkdown,
    prepared: PreparedUnitDiscoveryRequest,
    units: ValidatedUnitSet,
}

fn payload_mentions_artifact(value: &serde_json::Value, artifact_id: &StableId) -> bool {
    match value {
        serde_json::Value::String(value) => value == artifact_id.as_str(),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| payload_mentions_artifact(value, artifact_id)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| payload_mentions_artifact(value, artifact_id)),
        _ => false,
    }
}

fn registered_source_artifact(
    store: &SqliteArtifactStore,
    artifact_id: &StableId,
) -> Result<bool, DesktopMarkdownPatchError> {
    let snapshot = store.source_map_snapshot()?;
    Ok(snapshot.objects.iter().any(|object| {
        matches!(
            object.kind,
            EntityKind::SourceSpan
                | EntityKind::SourceSemanticUnit
                | EntityKind::SourceUnitRelation
        ) && object.lifecycle == LifecycleStatus::Active
            && payload_mentions_artifact(&object.payload, artifact_id)
    }))
}

impl DesktopService {
    pub fn get_markdown_patch_review(
        &self,
        run_id_text: String,
    ) -> Result<DesktopMarkdownPatchReview, DesktopMarkdownPatchError> {
        let run_id = StableId::parse(run_id_text.clone())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let checkpoint = store
            .discovery_checkpoint(&run_id)?
            .ok_or(DesktopMarkdownPatchError::MissingRun)?;
        let source: DiscoveryInput = checkpoint_value(&checkpoint.checkpoint.snapshot, "source")?;
        let Some((patch_set_id, _value, status)) =
            store.latest_markdown_patch_set_json_for_run(&run_id)?
        else {
            return Ok(DesktopMarkdownPatchReview {
                run_id: run_id_text,
                patch_set_id: None,
                input_fingerprint: source.fingerprint,
                status: "no_patch_set".into(),
                patch_set: None,
                provider_model: None,
                message: None,
            });
        };
        let patch_set = patch_set_from_store(&store, &patch_set_id)?;
        Ok(DesktopMarkdownPatchReview {
            run_id: run_id_text,
            patch_set_id: Some(patch_set_id.to_string()),
            input_fingerprint: source.fingerprint,
            status,
            patch_set: Some(patch_set),
            provider_model: None,
            message: None,
        })
    }

    /// Review for the newest discovery run over `artifact_id` that is still
    /// paused for Markdown patch review. A recorded decision whose resume has
    /// not happened yet is returned too, so the reader can offer to continue.
    pub fn get_pending_markdown_patch_review(
        &self,
        artifact_id: String,
    ) -> Result<Option<DesktopMarkdownPatchReview>, DesktopMarkdownPatchError> {
        let artifact_id = StableId::parse(artifact_id)?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let Some(run_id) = store.latest_discovery_run_awaiting_patch_review(&artifact_id)? else {
            return Ok(None);
        };
        drop(store);
        self.get_markdown_patch_review(run_id.to_string()).map(Some)
    }

    fn load_patch_context(
        &self,
        run_id: &StableId,
    ) -> Result<PatchContext, DesktopMarkdownPatchError> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let stored = store
            .discovery_checkpoint(run_id)?
            .ok_or(DesktopMarkdownPatchError::MissingRun)?;
        let source: DiscoveryInput = checkpoint_value(&stored.checkpoint.snapshot, "source")?;
        let unit_checkpoint: UnitDiscoveryCheckpoint =
            checkpoint_value(&stored.checkpoint.snapshot, "units")?;
        let document = self
            .markdown_document(
                source.snapshot.artifact_id.as_str(),
                source.snapshot.conversion_id.as_ref().map(StableId::as_str),
            )
            .map_err(DesktopMarkdownPatchError::Input)?;
        rebind_discovery_input(&document, &source, Default::default())
            .map_err(|error| DesktopMarkdownPatchError::Input(error.to_string()))?;
        let prepared = prepare_unit_discovery_request_from_input(source.clone());
        let units = prepared
            .revalidate_checkpoint(&unit_checkpoint, Default::default())
            .map_err(|report| DesktopMarkdownPatchError::Input(format!("{report:?}")))?;
        Ok(PatchContext {
            source,
            document,
            prepared,
            units,
        })
    }

    fn load_patch_set(
        &self,
        patch_set_id: &StableId,
    ) -> Result<MarkdownPatchSet, DesktopMarkdownPatchError> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        patch_set_from_store(&store, patch_set_id)
    }

    fn patch_preview_internal(
        &self,
        request: &PreviewMarkdownPatchRequest,
    ) -> Result<DesktopMarkdownPatchPreview, DesktopMarkdownPatchError> {
        let run_id = StableId::parse(request.run_id.clone())?;
        let patch_set_id = StableId::parse(request.patch_set_id.clone())?;
        let context = self.load_patch_context(&run_id)?;
        let patch_set = self.load_patch_set(&patch_set_id)?;
        if patch_set.run_id != run_id {
            return Err(DesktopMarkdownPatchError::Checkpoint(
                "patch set belongs to another discovery run".into(),
            ));
        }
        let patch_set = rewrite_patch_set_replacements(&patch_set, &request.replacements)
            .map_err(DesktopMarkdownPatchError::Validation)?;
        let applied = apply_patch_set(&context.document.source, &patch_set)
            .map_err(|error| DesktopMarkdownPatchError::Validation(error.to_string()))?;
        let patched_document = anchor_patched_markdown(
            context.source.snapshot.artifact_id.clone(),
            context.source.snapshot.conversion_id.clone(),
            &applied,
        )
        .map_err(|error| DesktopMarkdownPatchError::Input(error.to_string()))?;
        let unit_ranges = context
            .units
            .units()
            .iter()
            .map(|unit| {
                (
                    unit.proposal().unit_ref.clone(),
                    unit.selectors()
                        .iter()
                        .map(|selector| selector.range().clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let safety = evaluate_patch_safety(
            &context.document,
            &patched_document,
            &patch_set,
            &unit_ranges,
        );
        let original = context
            .source
            .requested
            .segments
            .iter()
            .map(mathscape_domain::DiscoveryInputSegment::text)
            .collect::<String>();
        Ok(DesktopMarkdownPatchPreview {
            run_id: request.run_id.clone(),
            patch_set_id: request.patch_set_id.clone(),
            patch_set_fingerprint: patch_set.fingerprint,
            original,
            revised: applied.source,
            safety,
        })
    }

    pub fn preview_markdown_patch(
        &self,
        request: PreviewMarkdownPatchRequest,
    ) -> Result<DesktopMarkdownPatchPreview, DesktopMarkdownPatchError> {
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let patch_set_id = StableId::parse(request.patch_set_id.clone())?;
        let preview = self.patch_preview_internal(&request)?;
        if !request.replacements.is_empty() {
            let actor = StableId::parse(DESKTOP_PATCH_ACTOR)?;
            let now = crate::discovery::timestamp_now()?;
            for (patch_ref, replacement) in &request.replacements {
                store.save_markdown_patch_draft(
                    &patch_set_id,
                    patch_ref,
                    replacement,
                    1,
                    &actor,
                    &now,
                )?;
            }
        }
        Ok(preview)
    }

    pub fn submit_markdown_patch_decision(
        &self,
        request: SubmitMarkdownPatchDecisionRequest,
    ) -> Result<MarkdownPatchApplyReceipt, DesktopMarkdownPatchError> {
        let run_id = StableId::parse(request.run_id.clone())?;
        let original_patch_set_id = StableId::parse(request.patch_set_id.clone())?;
        let context = self.load_patch_context(&run_id)?;
        let original = self.load_patch_set(&original_patch_set_id)?;
        if original.base_input_fingerprint != context.source.fingerprint
            || original.base_snapshot != context.source.snapshot
        {
            return Err(DesktopMarkdownPatchError::Input(
                "patch 的 base Markdown 快照已漂移，请重新运行内容发现".into(),
            ));
        }
        if original.fingerprint != request.expected_patch_fingerprint {
            return Err(DesktopMarkdownPatchError::Input(
                "patch review is stale; reload the latest patch set".into(),
            ));
        }
        let rewritten = rewrite_patch_set_replacements(&original, &request.replacements)
            .map_err(DesktopMarkdownPatchError::Validation)?;
        let actor = StableId::parse(DESKTOP_PATCH_ACTOR)?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let now = crate::discovery::timestamp_now()?;
        match store.markdown_patch_set_status(&original_patch_set_id)? {
            Some(MarkdownPatchStatus::Proposed) => {}
            Some(_) => {
                if let Some(application) =
                    store.latest_markdown_patch_application(&original_patch_set_id)?
                {
                    return replay_patch_receipt(&original, application);
                }
                return Err(DesktopMarkdownPatchError::Validation(
                    "该 Markdown patch set 已经完成审核决定，不能在同一次 run 中再次提交".into(),
                ));
            }
            None => {
                return Err(DesktopMarkdownPatchError::Checkpoint(
                    "patch set not found".into(),
                ));
            }
        }
        // Persist user edits before attempting the decision so a failed
        // submit or a full rejection can be resumed without losing the draft.
        for (patch_ref, replacement) in &request.replacements {
            store.save_markdown_patch_draft(
                &original_patch_set_id,
                patch_ref,
                replacement,
                1,
                &actor,
                &now,
            )?;
        }
        let accepted: HashSet<_> = request.accepted_patch_refs.iter().cloned().collect();
        if accepted.is_empty() {
            if let Some(application) =
                store.latest_markdown_patch_application(&original_patch_set_id)?
            {
                return replay_patch_receipt(&original, application);
            }
            let mut no_patch = rewritten.clone();
            no_patch.patches.clear();
            let safety =
                evaluate_patch_safety(&context.document, &context.document, &no_patch, &[]);
            let decision_id = fingerprint_id("markdown-patch-decision", &rewritten.fingerprint)?;
            let decision = MarkdownPatchDecision {
                decision_id: decision_id.clone(),
                run_id: run_id.clone(),
                patch_set_fingerprint: rewritten.fingerprint.clone(),
                kind: MarkdownPatchDecisionKind::Reject,
                accepted_patch_refs: vec![],
                actor: actor.clone(),
            };
            store.update_markdown_patch_status(
                &original_patch_set_id,
                MarkdownPatchStatus::Rejected,
                None,
                None,
                &now,
            )?;
            store.record_markdown_patch_decision(&decision, &now, &original_patch_set_id)?;
            let receipt = MarkdownPatchApplyReceipt {
                version: mathscape_domain::MARKDOWN_PATCH_RECEIPT_VERSION.into(),
                run_id,
                patch_set_fingerprint: rewritten.fingerprint.clone(),
                base_artifact_id: rewritten.base_snapshot.artifact_id.clone(),
                patched_artifact_id: None,
                conversion_id: None,
                old_input_fingerprint: rewritten.base_input_fingerprint.clone(),
                new_input_fingerprint: None,
                safety,
                status: MarkdownPatchStatus::Rejected,
            };
            let application_id =
                fingerprint_id("markdown-patch-application", &rewritten.fingerprint)?;
            store.record_markdown_patch_application(
                &application_id,
                &original_patch_set_id,
                &receipt,
                None,
                &now,
            )?;
            return Ok(receipt);
        }
        let selected = select_patch_set(&rewritten, &accepted)
            .map_err(DesktopMarkdownPatchError::Validation)?;
        let selected_patch_set_id = patch_set_id(&selected)?;
        if let Some(application) =
            store.latest_markdown_patch_application(&selected_patch_set_id)?
        {
            return replay_patch_receipt(&selected, application);
        }
        if registered_source_artifact(&store, &selected.base_snapshot.artifact_id)? {
            return Err(DesktopMarkdownPatchError::Validation(
                "该 Markdown 已经有活动来源对象，v1 不允许在原来源登记上直接修订；请从该材料重新发起新的发现运行".into(),
            ));
        }
        let applied = apply_patch_set(&context.document.source, &selected)
            .map_err(|error| DesktopMarkdownPatchError::Validation(error.to_string()))?;
        let content_hash = markdown_patch_content_hash(applied.source.as_bytes());
        let patched_artifact_id = markdown_patch_artifact_id(&content_hash)?;
        let conversion_id = markdown_patch_conversion_id(&selected)?;
        let patched_document = anchor_patched_markdown(
            patched_artifact_id.clone(),
            Some(conversion_id.clone()),
            &applied,
        )
        .map_err(|error| DesktopMarkdownPatchError::Input(error.to_string()))?;
        let unit_ranges = context
            .units
            .units()
            .iter()
            .map(|unit| {
                (
                    unit.proposal().unit_ref.clone(),
                    unit.selectors()
                        .iter()
                        .map(|selector| selector.range().clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut safety = evaluate_patch_safety(
            &context.document,
            &patched_document,
            &selected,
            &unit_ranges,
        );
        if safety.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "block_structure_changed" | "page_marker_changed"
            )
        }) {
            return Err(DesktopMarkdownPatchError::Validation(
                "所选修订会改变 Markdown 块结构或 PDF 分页标记，v1 不接受此类修订".into(),
            ));
        }
        let new_input = rebind_discovery_input_after_patch(
            &patched_document,
            &context.source,
            &applied.offset_map,
            Default::default(),
        )
        .ok();
        if matches!(safety.status, mathscape_domain::PatchSafetyStatus::SafeForB) {
            if let Some(input) = &new_input {
                if let Err(report) = mathscape_orchestrator::rebind_unit_set_after_patch(
                    &context.prepared,
                    &context.units,
                    input.clone(),
                    &applied.offset_map,
                ) {
                    safety.status = mathscape_domain::PatchSafetyStatus::RequiresARebuild;
                    safety
                        .diagnostics
                        .push(mathscape_domain::PatchSafetyDiagnostic {
                            code: "unit_set_rebind_failed".into(),
                            path: "unit_set".into(),
                            message: format!("{report:?}"),
                        });
                }
            } else {
                safety.status = mathscape_domain::PatchSafetyStatus::RequiresARebuild;
                safety
                    .diagnostics
                    .push(mathscape_domain::PatchSafetyDiagnostic {
                        code: "input_rebind_failed".into(),
                        path: "input".into(),
                        message: "patched selection could not be rebound".into(),
                    });
            }
        }
        let new_input_fingerprint = new_input.as_ref().map(|input| input.fingerprint.clone());
        let landscape = store.landscape_snapshot()?;
        let base = landscape
            .artifacts()
            .artifacts()
            .iter()
            .find(|record| {
                record.entity().header().stable_id() == &selected.base_snapshot.artifact_id
            })
            .ok_or_else(|| {
                DesktopMarkdownPatchError::Checkpoint("base artifact not found".into())
            })?;
        let mut pool = ArtifactPool::open(&self.workspace_root)?;
        let pdf_inheritance = markdown_pdf_map_inheritance(
            landscape.artifacts(),
            &pool,
            &selected.base_snapshot.artifact_id,
            selected.base_snapshot.conversion_id.as_ref(),
        )?;
        let promotion = promote_applied_patch(
            &mut pool,
            &selected.base_snapshot.artifact_id,
            base.entity().payload(),
            &selected,
            &applied,
            pdf_inheritance.as_ref(),
            safety,
            new_input_fingerprint,
            ActorRef::parse("mathscape-desktop-user")?,
            now.clone(),
        )?;
        store.save_markdown_patch_set(
            &selected_patch_set_id,
            &selected,
            MarkdownPatchStatus::Applied,
            promotion.receipt.patched_artifact_id.as_ref(),
            promotion.receipt.conversion_id.as_ref(),
            &now,
        )?;
        if selected_patch_set_id != original_patch_set_id {
            // A partial decision creates a separate applied patch set. Mark
            // the original proposal consumed so rejected operations cannot be
            // submitted later in the same discovery run.
            store.update_markdown_patch_status(
                &original_patch_set_id,
                MarkdownPatchStatus::Accepted,
                None,
                None,
                &now,
            )?;
        }
        let decision_id = fingerprint_id("markdown-patch-decision", &selected.fingerprint)?;
        let decision = MarkdownPatchDecision {
            decision_id,
            run_id: run_id.clone(),
            patch_set_fingerprint: selected.fingerprint.clone(),
            kind: MarkdownPatchDecisionKind::Accept,
            accepted_patch_refs: selected
                .patches
                .iter()
                .map(|patch| patch.patch_ref.clone())
                .collect(),
            actor,
        };
        store.record_markdown_patch_decision(&decision, &now, &selected_patch_set_id)?;
        let application_id = fingerprint_id("markdown-patch-application", &selected.fingerprint)?;
        store.record_markdown_patch_application(
            &application_id,
            &selected_patch_set_id,
            &promotion.receipt,
            None,
            &now,
        )?;
        Ok(promotion.receipt)
    }

    pub async fn run_markdown_patch(
        &self,
        request: RunMarkdownPatchRequest,
    ) -> Result<DesktopMarkdownPatchReview, DesktopMarkdownPatchError> {
        let run_id = StableId::parse(request.run_id.clone())?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let stored = store
            .discovery_checkpoint(&run_id)?
            .ok_or(DesktopMarkdownPatchError::MissingRun)?;
        let source: DiscoveryInput = checkpoint_value(&stored.checkpoint.snapshot, "source")?;
        // P is intentionally one-shot per discovery run.  Re-opening a
        // review returns the persisted candidate instead of spending another
        // model call; after a decision the run must start a new patch branch.
        if let Some((existing_id, _value, status)) =
            store.latest_markdown_patch_set_json_for_run(&run_id)?
        {
            let patch_set = patch_set_from_store(&store, &existing_id)?;
            return Ok(DesktopMarkdownPatchReview {
                run_id: request.run_id,
                patch_set_id: Some(existing_id.to_string()),
                input_fingerprint: source.fingerprint,
                status,
                patch_set: Some(patch_set),
                provider_model: None,
                message: Some("本次 discovery run 已经完成唯一一次 Markdown patch round".into()),
            });
        }
        let unit_checkpoint: UnitDiscoveryCheckpoint =
            checkpoint_value(&stored.checkpoint.snapshot, "units")?;
        let document = self
            .markdown_document(
                source.snapshot.artifact_id.as_str(),
                source.snapshot.conversion_id.as_ref().map(StableId::as_str),
            )
            .map_err(DesktopMarkdownPatchError::Input)?;
        rebind_discovery_input(&document, &source, Default::default())
            .map_err(|error| DesktopMarkdownPatchError::Input(error.to_string()))?;
        let prepared = prepare_unit_discovery_request_from_input(source.clone());
        let units = prepared
            .revalidate_checkpoint(&unit_checkpoint, Default::default())
            .map_err(|report| DesktopMarkdownPatchError::Input(format!("{report:?}")))?;
        let freshly_resolved = canonical_discovery_issues(&units);
        let persisted = canonical_discovery_issues_for_checkpoint(&unit_checkpoint);
        let issues = if unit_checkpoint.issue_registry.is_empty() {
            freshly_resolved
        } else if persisted.len() == freshly_resolved.len()
            && persisted
                .iter()
                .zip(&freshly_resolved)
                .all(|(stored, current)| {
                    stored.kind == current.kind
                        && stored.unit_ref == current.unit_ref
                        && stored.evidence == current.evidence
                        && stored.observation == current.observation
                })
        {
            persisted
        } else {
            return Err(DesktopMarkdownPatchError::Checkpoint(
                "持久化的 A 阶段 issue registry 与当前 UnitSet 不一致".into(),
            ));
        };
        let patch_request =
            build_markdown_patch_request_with_issues(&prepared, &units, issues.clone());
        if patch_request.input().issues.is_empty() {
            return Ok(DesktopMarkdownPatchReview {
                run_id: request.run_id,
                patch_set_id: None,
                input_fingerprint: source.fingerprint,
                status: "no_patchable_issue".into(),
                patch_set: None,
                provider_model: None,
                message: Some("当前 UnitSet 没有可修订的转换异常".into()),
            });
        }
        let profile = self.selected_llm_profile(request.profile_id.as_deref())?;
        let settings = self.workspace_settings()?;
        let policy =
            settings.discovery_policy(mathscape_domain::DiscoveryAdmission::ReviewBeforePublish);
        let backend = GenaiBackend::new(
            profile.clone(),
            self.credentials.clone(),
            GenaiBackendOptions {
                timeout: policy.request_timeout,
                connect_timeout: policy.connect_timeout,
            },
        )?;
        let outcome = MarkdownPatchProducer::new(&backend)
            .produce(patch_request.clone())
            .await;
        let response = match outcome {
            MarkdownPatchProductionOutcome::Valid { response, .. } => response,
            MarkdownPatchProductionOutcome::Invalid { rejection, .. } => {
                return Err(DesktopMarkdownPatchError::Rejected(rejection.to_string()));
            }
            MarkdownPatchProductionOutcome::Failed { error, .. } => {
                return Err(DesktopMarkdownPatchError::Provider(error.to_string()));
            }
        };
        if response.request_ref != patch_request.input().request_ref {
            return Err(DesktopMarkdownPatchError::Rejected(
                "patch response belongs to another request".into(),
            ));
        }
        let patch_set =
            match resolve_patch_proposals(&source, run_id.clone(), &issues, &response.patches) {
                Ok(patch_set) => patch_set,
                Err(report) => {
                    return Err(DesktopMarkdownPatchError::Validation(
                        report
                            .diagnostics()
                            .iter()
                            .map(|diagnostic| {
                                format!(
                                    "{} · {}: {}",
                                    diagnostic.path, diagnostic.code, diagnostic.message
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
            };
        if patch_set.patches.is_empty() {
            let patch_set_id = patch_set_id(&patch_set)?;
            let now = crate::discovery::timestamp_now()?;
            store.save_markdown_patch_set(
                &patch_set_id,
                &patch_set,
                mathscape_domain::MarkdownPatchStatus::Rejected,
                None,
                None,
                &now,
            )?;
            return Ok(DesktopMarkdownPatchReview {
                run_id: request.run_id,
                patch_set_id: Some(patch_set_id.to_string()),
                input_fingerprint: source.fingerprint,
                status: "no_patch_proposed".into(),
                patch_set: Some(patch_set),
                provider_model: Some(profile.model),
                message: Some("模型没有提出可安全验证的文本修订".into()),
            });
        }
        let patch_set_id = patch_set_id(&patch_set)?;
        let now = crate::discovery::timestamp_now()?;
        store.save_markdown_patch_set(
            &patch_set_id,
            &patch_set,
            mathscape_domain::MarkdownPatchStatus::Proposed,
            None,
            None,
            &now,
        )?;
        Ok(DesktopMarkdownPatchReview {
            run_id: request.run_id,
            patch_set_id: Some(patch_set_id.to_string()),
            input_fingerprint: source.fingerprint,
            status: "awaiting_patch_review".into(),
            patch_set: Some(patch_set),
            provider_model: Some(profile.model),
            message: None,
        })
    }
}

#[tauri::command]
pub async fn run_markdown_patch(
    state: State<'_, DesktopState>,
    request: RunMarkdownPatchRequest,
) -> Result<DesktopMarkdownPatchReview, String> {
    let service = state.service();
    service
        .run_markdown_patch(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_markdown_patch_review(
    state: State<'_, DesktopState>,
    run_id: String,
) -> Result<DesktopMarkdownPatchReview, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.get_markdown_patch_review(run_id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_pending_markdown_patch_review(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<Option<DesktopMarkdownPatchReview>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        service.get_pending_markdown_patch_review(artifact_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_markdown_patch(
    state: State<'_, DesktopState>,
    request: PreviewMarkdownPatchRequest,
) -> Result<DesktopMarkdownPatchPreview, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.preview_markdown_patch(request))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn submit_markdown_patch_decision(
    state: State<'_, DesktopState>,
    request: SubmitMarkdownPatchDecisionRequest,
) -> Result<MarkdownPatchApplyReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.submit_markdown_patch_decision(request))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}
