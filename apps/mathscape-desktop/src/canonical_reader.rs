//! Desktop unified reader projection for a canonical markdown node.
//!
//! 「阅读文件」 on a folded 「… · 规范解析正文」 node opens this composed,
//! read-only view instead of the head snapshot alone. The page plan comes from
//! [`mathscape_domain::plan_canonical_reader`] over the registered coverage
//! ledger; this module only reads the chosen immutable snapshots and returns
//! their text in page order. Nothing is written: no Artifact, no ledger row.
//!
//! `get_artifact_content_info` and `read_artifact_bytes` stay physical-id only.

use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    InclusivePageRange, MarkdownBlock, MarkdownCoverage, ReaderPiece, SkippedReaderSegment,
    StableId, markdown_offset_of_page, plan_canonical_reader,
};
use mathscape_orchestrator::{MAX_MARKDOWN_BYTES, anchor_markdown};
use mathscape_store_sqlite::SqliteArtifactStore;
use serde::Serialize;
use tauri::State;

use crate::{DesktopService, DesktopState};

#[derive(Debug, Serialize)]
pub struct CanonicalReaderProjection {
    pub canonical_id: String,
    pub source_pdf_artifact_id: String,
    pub head_artifact_id: Option<String>,
    pub total_pages: Option<u32>,
    pub coverage: MarkdownCoverage,
    pub pieces: Vec<CanonicalReaderPiece>,
    /// Snapshots whose pages are all shown by earlier segments.
    pub skipped: Vec<SkippedReaderSegment>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalReaderPiece {
    Segment {
        conversion_id: String,
        /// Physical snapshot the text was read from (open it for selection).
        artifact_id: String,
        snapshot_pages: Option<InclusivePageRange>,
        /// Pages this segment adds to the view.
        pages: Option<InclusivePageRange>,
        /// Leading pages that repeat an earlier segment.
        overlap: Option<InclusivePageRange>,
        /// True when the overlap was cut at a Marker page marker; false means
        /// the overlapping pages are shown again.
        overlap_trimmed: bool,
        is_head: bool,
        /// Byte offset in the snapshot where `markdown` starts.
        byte_start: u64,
        /// Immutable content hash of the underlying physical snapshot.
        content_hash: String,
        /// Physical snapshot blocks starting at or after `byte_start` (ranges remain physical snapshot byte offsets).
        blocks: Vec<MarkdownBlock>,
        markdown: String,
    },
    Gap {
        pages: InclusivePageRange,
    },
}

impl DesktopService {
    pub fn canonical_reader_projection(
        &self,
        canonical_id: &str,
    ) -> Result<CanonicalReaderProjection, String> {
        let id = StableId::parse(canonical_id).map_err(|e| e.to_string())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let view = store
            .list_canonical_markdown()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|view| view.canonical_id == id)
            .ok_or("规范解析正文未登记，请刷新地图后重试")?;
        drop(store);
        let plan = plan_canonical_reader(&view);
        let pool = ArtifactPool::open(&self.workspace_root).map_err(|e| e.to_string())?;
        let mut budget = MAX_MARKDOWN_BYTES;
        let mut pieces = Vec::with_capacity(plan.pieces.len());
        for piece in plan.pieces {
            match piece {
                ReaderPiece::Gap { pages } => pieces.push(CanonicalReaderPiece::Gap { pages }),
                ReaderPiece::Segment(segment) => {
                    let source = self.read_markdown_text(&pool, &segment.markdown_artifact_id)?;
                    let start = match (segment.overlap, segment.snapshot_pages, segment.pages) {
                        (Some(_), Some(snapshot), Some(pages)) => {
                            markdown_offset_of_page(&source, snapshot, pages.from)
                        }
                        _ => None,
                    };
                    let byte_start = start.unwrap_or(0);
                    let markdown = source[byte_start..].to_owned();
                    budget = budget
                        .checked_sub(markdown.len())
                        .ok_or("规范解析正文拼接后超过 16 MiB，当前阅读视图暂不加载")?;
                    let anchored = anchor_markdown(
                        segment.markdown_artifact_id.clone(),
                        Some(segment.conversion_id.clone()),
                        source,
                    )
                    .map_err(|e| e.to_string())?;
                    let content_hash = anchored.snapshot.content_hash.to_string();
                    let blocks: Vec<MarkdownBlock> = anchored
                        .blocks
                        .into_iter()
                        .filter(|b| b.range.end > byte_start as u64)
                        .collect();
                    pieces.push(CanonicalReaderPiece::Segment {
                        conversion_id: segment.conversion_id.to_string(),
                        artifact_id: segment.markdown_artifact_id.to_string(),
                        snapshot_pages: segment.snapshot_pages,
                        pages: segment.pages,
                        overlap: segment.overlap,
                        overlap_trimmed: start.is_some(),
                        is_head: segment.is_head,
                        byte_start: byte_start as u64,
                        content_hash,
                        blocks,
                        markdown,
                    });
                }
            }
        }
        Ok(CanonicalReaderProjection {
            canonical_id: view.canonical_id.to_string(),
            source_pdf_artifact_id: view.source_pdf_artifact_id.to_string(),
            head_artifact_id: view.head_artifact_id.as_ref().map(ToString::to_string),
            total_pages: view.total_pages,
            coverage: view.coverage(None),
            pieces,
            skipped: plan.skipped,
        })
    }

    fn read_markdown_text(&self, pool: &ArtifactPool, id: &StableId) -> Result<String, String> {
        let info = self.content_info(id.as_str())?;
        if info.media_type != "text/markdown" {
            return Err(format!("覆盖快照不是 Markdown：{id}"));
        }
        if info.byte_size > MAX_MARKDOWN_BYTES as u64 {
            return Err("Markdown 超过 16 MiB，当前阅读视图暂不加载".into());
        }
        let mut bytes = Vec::with_capacity(info.byte_size as usize);
        while (bytes.len() as u64) < info.byte_size {
            let length = (info.byte_size - bytes.len() as u64).min(4 * 1024 * 1024) as u32;
            let chunk = pool
                .read_bytes(id, bytes.len() as u64, length)
                .map_err(|e| e.to_string())?;
            if chunk.len() != length as usize {
                return Err("文件长度已改变，请检查 Artifact 完整性".into());
            }
            bytes.extend(chunk);
        }
        String::from_utf8(bytes).map_err(|_| format!("Markdown 不是有效 UTF-8：{id}"))
    }
}

#[tauri::command]
pub async fn get_canonical_reader_projection(
    state: State<'_, DesktopState>,
    canonical_id: String,
) -> Result<CanonicalReaderProjection, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.canonical_reader_projection(&canonical_id))
        .await
        .map_err(|e| e.to_string())?
}
