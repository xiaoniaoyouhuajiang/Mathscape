//! N2 desktop commands for NoteLinks (proposal §6, §14): Markdown block-range
//! resolution, context creation, link management and read-only overlays.

use crate::ink_problem_card::{self, InkProblemCard, ProblemCardStatus};
use mathscape_domain::InkLabDocument;
use mathscape_orchestrator::PAGE_MARKER_STRUCTURAL_KIND;
use mathscape_store_sqlite::{
    InkContextCreateReceipt, InkContextCreateRequest, InkLifecycle, InkLinkOverlay, InkLinkReceipt,
    InkLinkRequest, InkLinkRole, InkLinkTarget, InkLinkTargetStatus, InkLinkedNote,
    InkMarkdownTarget, InkNoteLink, InkSetPrimaryRequest, InkUnlinkRequest, markdown_excerpt,
    markdown_short_label,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ink::{InkCommandError, fail};
use crate::{DesktopService, DesktopState};

/// A Markdown block range as the reader knows it (§14.3).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InkMarkdownTargetRequest {
    pub artifact_id: String,
    pub content_hash: String,
    pub first_block: String,
    pub last_block: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InkLinkTargetRequest {
    SourceObject { object_id: String },
    Markdown(InkMarkdownTargetRequest),
    InkNote { document_id: String },
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateInkNoteFromContextRequest {
    pub request_id: String,
    pub document: InkLabDocument,
    pub session_id: String,
    pub writer_id: String,
    #[serde(default)]
    pub device: Value,
    pub target: InkLinkTargetRequest,
    pub role: InkLinkRole,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkInkNoteRequest {
    pub request_id: String,
    pub document_id: String,
    pub target: InkLinkTargetRequest,
    pub role: InkLinkRole,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub replaces_link_id: Option<String>,
}

/// Links of one Markdown artifact plus the patch-derived hint (§6.4).
#[derive(Clone, Debug, Serialize)]
pub struct InkMarkdownLinks {
    pub artifact_id: String,
    pub links: Vec<InkLinkedNote>,
    /// Active notes linked to the original this file was patched from.
    pub original_note_count: u64,
}

fn invalid(message: impl Into<String>) -> InkCommandError {
    InkCommandError::new("link_target_invalid", message)
}

impl DesktopService {
    /// Re-anchors the Markdown and validates the block range (§6.5): blocks
    /// exist and are ordered, the content hash matches and the range is not
    /// only page markers.
    pub fn resolve_ink_markdown_target(
        &self,
        request: &InkMarkdownTargetRequest,
    ) -> Result<InkMarkdownTarget, InkCommandError> {
        let document = self
            .markdown_for_link(&request.artifact_id)
            .map_err(invalid)?;
        if document.snapshot.content_hash.to_string() != request.content_hash {
            return Err(invalid("Markdown 内容已变化，请重新选择。"));
        }
        let find = |id: &str| document.blocks.iter().position(|block| block.id == id);
        let (Some(first), Some(last)) = (find(&request.first_block), find(&request.last_block))
        else {
            return Err(invalid("所选段落已不存在，请重新选择。"));
        };
        if first > last {
            return Err(invalid("所选段落顺序无效，请重新选择。"));
        }
        let blocks = &document.blocks[first..=last];
        let Some(lead) = blocks
            .iter()
            .find(|block| block.kind != PAGE_MARKER_STRUCTURAL_KIND)
        else {
            return Err(invalid("分页提示不能关联手稿。"));
        };
        let start = blocks[0].range.start;
        let end = blocks[blocks.len() - 1].range.end;
        let text = document
            .source
            .get(start as usize..end as usize)
            .ok_or_else(|| invalid("所选段落范围无效。"))?;
        let lead_text = document
            .source
            .get(lead.range.start as usize..lead.range.end as usize)
            .unwrap_or(text);
        let label = if lead.kind == "heading" {
            markdown_short_label("heading", lead_text)
        } else {
            let from_lead = &document.source[lead.range.start as usize..end as usize];
            markdown_short_label(&lead.kind, from_lead)
        };
        Ok(InkMarkdownTarget {
            artifact_id: request.artifact_id.clone(),
            content_hash: request.content_hash.clone(),
            first_block: request.first_block.clone(),
            last_block: request.last_block.clone(),
            block_ids: blocks.iter().map(|block| block.id.clone()).collect(),
            start,
            end,
            block_hash: format!("{:x}", Sha256::digest(text.as_bytes())),
            excerpt: markdown_excerpt(text),
            label,
        })
    }

    fn resolve_ink_target(
        &self,
        target: &InkLinkTargetRequest,
    ) -> Result<InkLinkTarget, InkCommandError> {
        Ok(match target {
            InkLinkTargetRequest::SourceObject { object_id } => InkLinkTarget::SourceObject {
                object_id: object_id.clone(),
            },
            InkLinkTargetRequest::Markdown(markdown) => {
                InkLinkTarget::Markdown(self.resolve_ink_markdown_target(markdown)?)
            }
            InkLinkTargetRequest::InkNote { document_id } => InkLinkTarget::InkNote {
                document_id: document_id.clone(),
            },
        })
    }

    fn link_store(&self) -> Result<mathscape_store_sqlite::SqliteArtifactStore, InkCommandError> {
        mathscape_store_sqlite::SqliteArtifactStore::open(
            &self.workspace_root().join("mathscape.sqlite3"),
        )
        .map_err(fail)
    }

    fn broadcast_title(&self, receipt: &InkLinkReceipt) {
        if receipt.title_changed && !receipt.replayed {
            self.ink_note_renamed(&receipt.note);
        }
        if !receipt.replayed {
            // Primary add/remove/transfer or any link change that may affect the card.
            self.ink_problem_card_changed(&receipt.note.document_id);
        }
    }

    /// Creates a note with its primary link and title in one transaction.
    pub fn create_ink_note_from_context(
        &self,
        request: CreateInkNoteFromContextRequest,
    ) -> Result<InkContextCreateReceipt, InkCommandError> {
        let target = self.resolve_ink_target(&request.target)?;
        let receipt = self
            .link_store()?
            .create_ink_note_from_context(&InkContextCreateRequest {
                request_id: request.request_id,
                document: request.document,
                session_id: request.session_id,
                writer_id: request.writer_id,
                device: request.device,
                target,
                role: request.role,
                title: request.title,
            })
            .map_err(fail)?;
        // New notes from context always have a primary link.
        self.ink_problem_card_changed(&receipt.note.document_id);
        Ok(receipt)
    }

    pub fn link_ink_note(
        &self,
        request: LinkInkNoteRequest,
    ) -> Result<InkLinkReceipt, InkCommandError> {
        let target = self.resolve_ink_target(&request.target)?;
        let receipt = self
            .link_store()?
            .link_ink_note(&InkLinkRequest {
                request_id: request.request_id,
                document_id: request.document_id,
                target,
                role: request.role,
                primary: request.primary,
                replaces_link_id: request.replaces_link_id,
            })
            .map_err(fail)?;
        self.broadcast_title(&receipt);
        Ok(receipt)
    }

    pub fn unlink_ink_note(
        &self,
        request: InkUnlinkRequest,
    ) -> Result<InkLinkReceipt, InkCommandError> {
        let receipt = self.link_store()?.unlink_ink_note(&request).map_err(fail)?;
        self.broadcast_title(&receipt);
        Ok(receipt)
    }

    pub fn set_ink_note_primary_link(
        &self,
        request: InkSetPrimaryRequest,
    ) -> Result<InkLinkReceipt, InkCommandError> {
        let receipt = self
            .link_store()?
            .set_ink_note_primary_link(&request)
            .map_err(fail)?;
        self.broadcast_title(&receipt);
        Ok(receipt)
    }

    /// N3b: primary-link excerpt for the pad problem card (§10 / decision 10–11).
    pub fn ink_problem_card(
        &self,
        document_id: &str,
    ) -> Result<Option<InkProblemCard>, InkCommandError> {
        let links = self
            .link_store()?
            .ink_note_links(document_id)
            .map_err(fail)?;
        let Some(link) = links.into_iter().find(|link| link.is_primary) else {
            return Ok(None);
        };
        Ok(Some(self.render_problem_card(&link)?))
    }

    fn render_problem_card(&self, link: &InkNoteLink) -> Result<InkProblemCard, InkCommandError> {
        use mathscape_store_sqlite::InkLinkTarget;
        match &link.target {
            InkLinkTarget::Markdown(markdown) => {
                match self.markdown_for_link(&markdown.artifact_id) {
                    Ok(document) => {
                        let live_hash = document.snapshot.content_hash.to_string();
                        if live_hash != markdown.content_hash {
                            return Ok(ink_problem_card::finalize_card(
                                link,
                                markdown.excerpt.clone(),
                                ProblemCardStatus::Stale,
                                Some(live_hash),
                            ));
                        }
                        let find =
                            |id: &str| document.blocks.iter().position(|block| block.id == id);
                        let (Some(first), Some(last)) =
                            (find(&markdown.first_block), find(&markdown.last_block))
                        else {
                            return Ok(ink_problem_card::finalize_card(
                                link,
                                markdown.excerpt.clone(),
                                ProblemCardStatus::Missing,
                                Some(live_hash),
                            ));
                        };
                        if first > last {
                            return Ok(ink_problem_card::finalize_card(
                                link,
                                markdown.excerpt.clone(),
                                ProblemCardStatus::Missing,
                                Some(live_hash),
                            ));
                        }
                        let start = document.blocks[first].range.start as usize;
                        let end = document.blocks[last].range.end as usize;
                        let text = document.source.get(start..end).unwrap_or("");
                        let body = if text.is_empty() {
                            markdown.excerpt.clone()
                        } else {
                            ink_problem_card::markdown_body_from_source(text)
                        };
                        Ok(ink_problem_card::finalize_card(
                            link,
                            body,
                            ProblemCardStatus::Ok,
                            Some(live_hash),
                        ))
                    }
                    Err(_) => Ok(ink_problem_card::finalize_card(
                        link,
                        markdown.excerpt.clone(),
                        ProblemCardStatus::Missing,
                        Some(markdown.content_hash.clone()),
                    )),
                }
            }
            InkLinkTarget::SourceObject { object_id } => {
                self.source_object_problem_card(link, object_id)
            }
            InkLinkTarget::InkNote { document_id } => {
                let note = self.link_store()?.ink_note(document_id).map_err(fail)?;
                match note {
                    Some(note) if note.lifecycle != InkLifecycle::Trashed => {
                        Ok(ink_problem_card::finalize_card(
                            link,
                            format!("关联手稿：{}", note.title),
                            ProblemCardStatus::Unsupported,
                            None,
                        ))
                    }
                    _ => Ok(ink_problem_card::finalize_card(
                        link,
                        String::new(),
                        ProblemCardStatus::Missing,
                        None,
                    )),
                }
            }
        }
    }

    fn source_object_problem_card(
        &self,
        link: &InkNoteLink,
        object_id: &str,
    ) -> Result<InkProblemCard, InkCommandError> {
        use mathscape_artifact::ArtifactPool;
        use mathscape_domain::StableId;
        use std::collections::BTreeSet;

        let unavailable = |status| {
            Ok(ink_problem_card::finalize_card(
                link,
                String::new(),
                status,
                None,
            ))
        };
        if link.target_status != InkLinkTargetStatus::Active {
            return unavailable(ProblemCardStatus::Missing);
        }
        let store = self.link_store()?;
        let Some(payload) = store.map_object_payload(object_id).map_err(fail)? else {
            return unavailable(ProblemCardStatus::Missing);
        };
        let body = payload.get("payload").unwrap_or(&payload);
        // A unit authorizes its source spans, not the entire containing artifact.
        let spans: Vec<String> = match body.get("source_spans").and_then(Value::as_array) {
            Some(spans) => spans
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            None => vec![object_id.to_owned()],
        };
        if spans.is_empty() {
            return unavailable(ProblemCardStatus::Unsupported);
        }
        let catalog = store.read_catalog_snapshot().map_err(fail)?;
        let Ok(pool) = ArtifactPool::open(self.workspace_root()) else {
            return unavailable(ProblemCardStatus::Missing);
        };
        let mut parts = Vec::new();
        let mut chars = 0;
        let mut seen = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for span in spans {
            if chars > ink_problem_card::PROBLEM_CARD_BODY_MAX {
                break;
            }
            let Some(payload) = store.map_object_payload(&span).map_err(fail)? else {
                return unavailable(ProblemCardStatus::Missing);
            };
            let body = payload.get("payload").unwrap_or(&payload);
            let Some(markdown) = body.get("markdown").filter(|value| !value.is_null()) else {
                return unavailable(ProblemCardStatus::Unsupported);
            };
            let (Some(artifact), Some(start), Some(end)) = (
                markdown.get("artifact").and_then(Value::as_str),
                markdown
                    .pointer("/byte_range/start")
                    .and_then(Value::as_u64),
                markdown.pointer("/byte_range/end").and_then(Value::as_u64),
            ) else {
                return unavailable(ProblemCardStatus::Missing);
            };
            if !seen.insert((artifact.to_owned(), start, end)) {
                continue;
            }
            let Ok(id) = StableId::parse(artifact) else {
                return unavailable(ProblemCardStatus::Missing);
            };
            let Some(artifact) = catalog
                .artifacts()
                .iter()
                .find(|a| a.entity().header().stable_id() == &id)
            else {
                return unavailable(ProblemCardStatus::Missing);
            };
            let artifact = artifact.entity().payload();
            if artifact.media_type().as_str() != "text/markdown" {
                return unavailable(ProblemCardStatus::Unsupported);
            }
            if start >= end || end > artifact.byte_size() {
                return unavailable(ProblemCardStatus::Missing);
            }
            // At most 4001 Unicode characters are needed to establish truncation.
            let length = (end - start)
                .min(((ink_problem_card::PROBLEM_CARD_BODY_MAX + 1) * 4) as u64)
                as u32;
            let Ok(bytes) = pool.read_bytes(&id, start, length) else {
                return unavailable(ProblemCardStatus::Missing);
            };
            if bytes.len() != length as usize {
                return unavailable(ProblemCardStatus::Missing);
            }
            let source = match std::str::from_utf8(&bytes) {
                Ok(source) => source,
                Err(error) if error.error_len().is_none() && u64::from(length) < end - start => {
                    std::str::from_utf8(&bytes[..error.valid_up_to()]).map_err(fail)?
                }
                Err(_) => return unavailable(ProblemCardStatus::Missing),
            };
            let text = ink_problem_card::markdown_body_from_source(source);
            if !text.is_empty() {
                chars += text.chars().count() + if parts.is_empty() { 0 } else { 2 };
                parts.push(text);
                hashes.insert(artifact.content_hash().to_string());
            }
        }
        if parts.is_empty() {
            return unavailable(ProblemCardStatus::Missing);
        }
        let hash = if hashes.len() == 1 {
            hashes.into_iter().next()
        } else {
            None
        };
        Ok(ink_problem_card::finalize_card(
            link,
            parts.join("\n\n"),
            ProblemCardStatus::Ok,
            hash,
        ))
    }

    pub fn list_ink_note_links(
        &self,
        document_id: String,
    ) -> Result<Vec<InkNoteLink>, InkCommandError> {
        self.link_store()?
            .ink_note_links(&document_id)
            .map_err(fail)
    }

    pub fn list_ink_links_to_target(
        &self,
        target_kind: String,
        target_id: String,
    ) -> Result<Vec<InkLinkedNote>, InkCommandError> {
        if !matches!(
            target_kind.as_str(),
            "source_object" | "markdown" | "ink_note"
        ) {
            return Err(invalid("未知的关联目标类型。"));
        }
        self.link_store()?
            .ink_links_to_target(&target_kind, &target_id)
            .map_err(fail)
    }

    pub fn ink_markdown_links(
        &self,
        artifact_id: String,
    ) -> Result<InkMarkdownLinks, InkCommandError> {
        let store = self.link_store()?;
        Ok(InkMarkdownLinks {
            links: store
                .ink_links_to_target("markdown", &artifact_id)
                .map_err(fail)?,
            original_note_count: store.ink_markdown_patch_hint(&artifact_id).map_err(fail)?,
            artifact_id,
        })
    }

    pub fn ink_link_overlay(&self) -> Result<InkLinkOverlay, InkCommandError> {
        self.link_store()?.ink_link_overlay().map_err(fail)
    }
}

fn worker(message: String) -> InkCommandError {
    InkCommandError::new("worker_failed", message)
}

macro_rules! blocking {
    ($state:expr, $label:literal, |$service:ident| $body:expr) => {{
        let $service = $state.service.clone();
        tauri::async_runtime::spawn_blocking(move || $body)
            .await
            .map_err(|error| worker(format!(concat!($label, " worker failed: {}"), error)))?
    }};
}

#[tauri::command]
pub async fn create_ink_note_from_context(
    state: State<'_, DesktopState>,
    request: CreateInkNoteFromContextRequest,
) -> Result<InkContextCreateReceipt, InkCommandError> {
    blocking!(state, "Ink context create", |service| service
        .create_ink_note_from_context(request))
}

#[tauri::command]
pub async fn link_ink_note(
    state: State<'_, DesktopState>,
    request: LinkInkNoteRequest,
) -> Result<InkLinkReceipt, InkCommandError> {
    blocking!(state, "Ink link", |service| service.link_ink_note(request))
}

#[tauri::command]
pub async fn unlink_ink_note(
    state: State<'_, DesktopState>,
    request: InkUnlinkRequest,
) -> Result<InkLinkReceipt, InkCommandError> {
    blocking!(state, "Ink unlink", |service| service
        .unlink_ink_note(request))
}

#[tauri::command]
pub async fn set_ink_note_primary_link(
    state: State<'_, DesktopState>,
    request: InkSetPrimaryRequest,
) -> Result<InkLinkReceipt, InkCommandError> {
    blocking!(state, "Ink primary link", |service| service
        .set_ink_note_primary_link(request))
}

#[tauri::command]
pub async fn list_ink_note_links(
    state: State<'_, DesktopState>,
    document_id: String,
) -> Result<Vec<InkNoteLink>, InkCommandError> {
    blocking!(state, "Ink note links", |service| service
        .list_ink_note_links(document_id))
}

#[tauri::command]
pub async fn list_ink_links_to_target(
    state: State<'_, DesktopState>,
    target_kind: String,
    target_id: String,
) -> Result<Vec<InkLinkedNote>, InkCommandError> {
    blocking!(state, "Ink target links", |service| service
        .list_ink_links_to_target(target_kind, target_id))
}

#[tauri::command]
pub async fn get_ink_markdown_links(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<InkMarkdownLinks, InkCommandError> {
    blocking!(state, "Ink Markdown links", |service| service
        .ink_markdown_links(artifact_id))
}

#[tauri::command]
pub async fn get_ink_link_overlay(
    state: State<'_, DesktopState>,
) -> Result<InkLinkOverlay, InkCommandError> {
    blocking!(state, "Ink link overlay", |service| service
        .ink_link_overlay())
}

/// A1 (§7.2): saves unsaved local ink as an importable recovery file before
/// the user discards it. `None` when the user cancels the dialog.
#[tauri::command]
pub async fn export_ink_unsaved_writing(
    app: AppHandle,
    file_name: String,
    contents: String,
) -> Result<Option<String>, InkCommandError> {
    serde_json::from_str::<Value>(&contents)
        .map_err(|_| InkCommandError::new("validation_failed", "导出内容不是有效 JSON。"))?;
    let name: String = file_name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':') && !c.is_control())
        .take(120)
        .collect();
    let selected = app
        .dialog()
        .file()
        .add_filter("Ink JSON recovery", &["json"])
        .set_file_name(if name.is_empty() {
            "Mathscape-未保存笔迹.json".into()
        } else {
            name
        })
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| InkCommandError::new("export_failed", error.to_string()))?;
    std::fs::write(&path, contents)
        .map_err(|error| InkCommandError::new("export_failed", error.to_string()))?;
    Ok(Some(path.to_string_lossy().into()))
}
