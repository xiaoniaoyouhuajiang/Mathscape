//! Deterministic page provenance from Marker-paginated Markdown.

use mathscape_domain::{
    ContentHash, MARKDOWN_PDF_BLOCK_MAP_VERSION, MarkdownPdfBlockEntry, MarkdownPdfBlockMap,
    MarkdownPdfMappingConfidence, StableId,
};
use sha2::{Digest, Sha256};

/// Builds a page map from Marker 2.x's `--paginate_output` separators. The
/// parser deliberately returns `None` for unpaginated or malformed output;
/// callers must then expose Markdown-only provenance rather than guessing.
pub fn build_markdown_pdf_block_map(
    markdown_artifact_id: StableId,
    conversion_id: StableId,
    pdf_artifact_id: StableId,
    source: String,
) -> Result<Option<MarkdownPdfBlockMap>, String> {
    let document = crate::anchor_markdown(
        markdown_artifact_id.clone(),
        Some(conversion_id.clone()),
        source,
    )
    .map_err(|error| error.to_string())?;
    let markers = page_markers(&document.source);
    if markers.is_empty() {
        return Ok(None);
    }

    let intervals = markers
        .iter()
        .enumerate()
        .map(|(index, marker)| {
            let start = if index == 0 { 0 } else { marker.content_start };
            let end = markers
                .get(index + 1)
                .map(|next| next.marker_start)
                .unwrap_or(document.source.len());
            (start, end, marker.page_id)
        })
        .collect::<Vec<_>>();
    let entries = document
        .blocks
        .iter()
        .map(|block| {
            let pages = intervals
                .iter()
                .filter(|(start, end, _)| {
                    block.range.end > *start as u64 && block.range.start < *end as u64
                })
                .map(|(_, _, page)| *page)
                .collect::<Vec<_>>();
            let confidence = match pages.len() {
                0 => MarkdownPdfMappingConfidence::Unmapped,
                1 => MarkdownPdfMappingConfidence::Exact,
                _ => MarkdownPdfMappingConfidence::Candidate,
            };
            let start = usize::try_from(block.range.start).map_err(|_| "block start overflow")?;
            let end = usize::try_from(block.range.end).map_err(|_| "block end overflow")?;
            let text = document
                .source
                .get(start..end)
                .ok_or("block range is not valid UTF-8")?;
            Ok(MarkdownPdfBlockEntry {
                block_id: block.id.clone(),
                markdown_range: block.range.clone(),
                pdf_page_indices: pages,
                confidence,
                text_fingerprint: text_fingerprint(text),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Some(MarkdownPdfBlockMap {
        version: MARKDOWN_PDF_BLOCK_MAP_VERSION.into(),
        conversion_id,
        markdown_artifact_id,
        pdf_artifact_id,
        markdown_content_hash: document.snapshot.content_hash,
        marker_page_ids: markers.iter().map(|marker| marker.page_id).collect(),
        entries,
    }))
}

#[derive(Clone, Copy, Debug)]
struct PageMarker {
    marker_start: usize,
    content_start: usize,
    page_id: u32,
}

fn page_markers(source: &str) -> Vec<PageMarker> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('{') {
        let start = cursor + relative;
        let mut digit_end = start + 1;
        while digit_end < bytes.len() && bytes[digit_end].is_ascii_digit() {
            digit_end += 1;
        }
        if digit_end == start + 1 || digit_end >= bytes.len() || bytes[digit_end] != b'}' {
            cursor = start + 1;
            continue;
        }
        let separator_start = digit_end + 1;
        let mut separator_end = separator_start;
        while separator_end < bytes.len() && bytes[separator_end] == b'-' {
            separator_end += 1;
        }
        if separator_end - separator_start < 16 {
            cursor = start + 1;
            continue;
        }
        let content_start = if bytes.get(separator_end) == Some(&b'\r')
            && bytes.get(separator_end + 1) == Some(&b'\n')
        {
            separator_end + 2
        } else if bytes.get(separator_end) == Some(&b'\n') {
            separator_end + 1
        } else {
            cursor = start + 1;
            continue;
        };
        let page_id = source[start + 1..digit_end].parse::<u32>().ok();
        if let Some(page_id) = page_id {
            result.push(PageMarker {
                marker_start: start,
                content_start,
                page_id,
            });
            cursor = content_start;
        } else {
            cursor = start + 1;
        }
    }
    result
}

fn text_fingerprint(text: &str) -> ContentHash {
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(text.as_bytes())))
        .expect("SHA-256 always produces a valid content hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_paginated_blocks_to_zero_based_marker_pages() {
        let source = "{30}----------------\n\n第一段\n\n{31}----------------\n\n第二段\n";
        let map = build_markdown_pdf_block_map(
            StableId::parse("mathscape:artifact/markdown").unwrap(),
            StableId::parse("mathscape:conversion/marker").unwrap(),
            StableId::parse("mathscape:artifact/pdf").unwrap(),
            source.into(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(map.marker_page_ids, [30, 31]);
        assert!(
            map.entries
                .iter()
                .any(|entry| entry.pdf_page_indices == [30])
        );
        assert!(
            map.entries
                .iter()
                .any(|entry| entry.pdf_page_indices == [31])
        );
    }

    #[test]
    fn unpaginated_markdown_has_no_map() {
        assert!(
            build_markdown_pdf_block_map(
                StableId::parse("mathscape:artifact/markdown").unwrap(),
                StableId::parse("mathscape:conversion/marker").unwrap(),
                StableId::parse("mathscape:artifact/pdf").unwrap(),
                "普通 Markdown".into(),
            )
            .unwrap()
            .is_none()
        );
    }
}
