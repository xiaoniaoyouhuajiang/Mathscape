//! Deterministic physical Markdown anchors. These do not infer mathematical units.
use std::ops::Range;

use mathscape_domain::{
    AnchoredMarkdown, ContentHash, DISCOVERY_SELECTION_VERSION, DiscoverySelectionRequest,
    MARKDOWN_ANCHOR_SCHEME, MarkdownBlock, MarkdownSnapshot, ResolvedDiscoverySelection,
    SourceSelector, StableId, TextByteRange,
};
use pulldown_cmark::{Event, Options, Parser, Tag};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_MARKDOWN_BYTES: usize = 16 * 1024 * 1024;
const MAX_BLOCKS: usize = 20_000;
/// Structural kind used for Marker pagination metadata. The bytes remain part
/// of the canonical Markdown snapshot, but this block is not mathematical
/// content and must not be used as semantic evidence.
pub const PAGE_MARKER_STRUCTURAL_KIND: &str = "page_marker";

#[derive(Debug, Error)]
pub enum AnchorError {
    #[error("Markdown 超过 16 MiB，当前块级选区暂不支持")]
    TooLarge,
    #[error("Markdown 块数超过 20000，当前选区暂不支持")]
    TooManyBlocks,
    #[error("输入快照或选区协议已改变，请重新打开文件")]
    StaleSnapshot,
    #[error("选区引用了不存在或其他文件的文本块")]
    UnknownBlock,
    #[error("终点必须位于起点之后，且选区不能只有空白")]
    InvalidRange,
    #[error("原文片段不存在或不唯一，请提供更精确的锚点")]
    AmbiguousQuote,
}

fn hash(bytes: &[u8]) -> ContentHash {
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("SHA-256 always produces a valid content hash")
}

/// Returns the zero-based Marker page id when `text` is exactly a pagination
/// separator such as `{21}------------------------------------------------`.
///
/// This deliberately accepts surrounding Markdown whitespace because parser
/// ranges include the blank lines around a top-level paragraph. It does not
/// recognize a separator embedded in ordinary prose or code.
pub fn page_marker_id(text: &str) -> Option<u32> {
    let marker = text.trim();
    let close = marker.find('}')?;
    if !marker.starts_with('{') || close <= 1 {
        return None;
    }
    let page = marker[1..close].parse::<u32>().ok()?;
    let dashes = &marker[close + 1..];
    (dashes.len() >= 16 && dashes.bytes().all(|byte| byte == b'-')).then_some(page)
}

pub fn is_page_marker_text(text: &str) -> bool {
    page_marker_id(text).is_some()
}

/// Top-level CommonMark containers are atomic (including lists/tables/code).
/// Parser ranges are merged with TeX display fences so blank lines and Markdown
/// metacharacters inside them cannot introduce spurious selectable blocks.
pub fn anchor_markdown(
    artifact_id: StableId,
    conversion_id: Option<StableId>,
    source: String,
) -> Result<AnchoredMarkdown, AnchorError> {
    if source.len() > MAX_MARKDOWN_BYTES {
        return Err(AnchorError::TooLarge);
    }
    let content_hash = hash(source.as_bytes());
    let identity = serde_json::to_vec(&(
        &artifact_id,
        &conversion_id,
        &content_hash,
        MARKDOWN_ANCHOR_SCHEME,
    ))
    .expect("simple tuple serialization");
    let snapshot = MarkdownSnapshot {
        artifact_id,
        conversion_id,
        content_hash,
        anchor_scheme: MARKDOWN_ANCHOR_SCHEME.into(),
        fingerprint: hash(&identity),
    };
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut spans: Vec<(Range<usize>, String)> = Vec::new();
    let mut code_ranges = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut kind = String::new();
    for (event, range) in Parser::new_ext(&source, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::CodeBlock(_)) {
                    code_ranges.push(range.clone());
                }
                if depth == 0 {
                    start = range.start;
                    kind = match tag {
                        Tag::Heading { .. } => "heading",
                        Tag::List(_) => "list",
                        Tag::Table(_) => "table",
                        Tag::BlockQuote(_) => "blockquote",
                        Tag::CodeBlock(_) => "code",
                        _ => "text",
                    }
                    .into();
                }
                depth += 1;
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    spans.push((start..range.end, kind.clone()));
                }
            }
            _ if depth == 0 && !range.is_empty() => spans.push((range, "text".into())),
            _ => {}
        }
    }
    // Merge display math ranges with any overlapping parser containers. This
    // deliberately keeps a surrounding list/paragraph whole, never cuts it.
    let mut offset = 0;
    let mut pending: Option<(usize, &str)> = None;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        let trimmed = line.trim();
        let body = line.trim_start_matches(' ');
        let indent = line.len() - body.len();
        if let Some((begin, close)) = pending {
            if trimmed.ends_with(close) {
                spans.push((begin..end, "math".into()));
                pending = None;
            }
        } else if indent < 4 && !code_ranges.iter().any(|r| r.contains(&offset)) {
            let pair = if body.starts_with("$$") {
                Some(("$$", "$$"))
            } else if body.starts_with("\\[") {
                Some(("\\[", "\\]"))
            } else {
                None
            };
            if let Some((open, close)) = pair {
                if body[open.len()..].contains(close) {
                    spans.push((offset..end, "math".into()));
                } else {
                    pending = Some((offset, close));
                }
            }
        }
        offset = end;
    }
    if let Some((begin, _)) = pending {
        spans.push((begin..source.len(), "math_unclosed".into()));
    }
    spans.sort_by_key(|(r, _)| (r.start, r.end));
    let mut merged: Vec<(Range<usize>, String)> = Vec::new();
    for (range, kind) in spans {
        if let Some((last, last_kind)) = merged.last_mut()
            && range.start < last.end
        {
            last.end = last.end.max(range.end);
            if kind.starts_with("math") {
                *last_kind = kind;
            }
            continue;
        }
        merged.push((range, kind));
    }
    // Non-rendering syntax (e.g. reference definitions) remains selectable too.
    let mut ranges = Vec::new();
    let mut cursor = 0;
    for (range, kind) in merged {
        if !source[cursor..range.start].trim().is_empty() {
            ranges.push((cursor..range.start, "source".into()));
        }
        cursor = range.end;
        ranges.push((range, kind));
    }
    if !source[cursor..].trim().is_empty() {
        ranges.push((cursor..source.len(), "source".into()));
    }
    if ranges.len() > MAX_BLOCKS {
        return Err(AnchorError::TooManyBlocks);
    }
    let blocks = ranges
        .into_iter()
        .map(|(range, kind)| {
            let kind = source
                .get(range.clone())
                .filter(|text| kind == "text" && is_page_marker_text(text))
                .map_or(kind, |_| PAGE_MARKER_STRUCTURAL_KIND.into());
            let identity = serde_json::to_vec(&(
                &snapshot.artifact_id,
                &snapshot.content_hash,
                MARKDOWN_ANCHOR_SCHEME,
                range.start,
                range.end,
            ))
            .expect("simple tuple");
            MarkdownBlock {
                id: format!("block:{}", hash(&identity)),
                range: TextByteRange {
                    start: range.start as u64,
                    end: range.end as u64,
                },
                kind,
            }
        })
        .collect();
    Ok(AnchoredMarkdown {
        snapshot,
        source,
        blocks,
    })
}

pub fn resolve_selection(
    document: &AnchoredMarkdown,
    request: DiscoverySelectionRequest,
) -> Result<ResolvedDiscoverySelection, AnchorError> {
    if request.version != DISCOVERY_SELECTION_VERSION || request.snapshot != document.snapshot {
        return Err(AnchorError::StaleSnapshot);
    }
    let find = |id: &str| {
        document
            .blocks
            .iter()
            .position(|b| b.id == id)
            .ok_or(AnchorError::UnknownBlock)
    };
    let quote_position = |block: usize, quote: &str| -> Result<usize, AnchorError> {
        if quote.is_empty() {
            return Err(AnchorError::AmbiguousQuote);
        }
        let range = &document.blocks[block].range;
        let text = &document.source[range.start as usize..range.end as usize];
        let first = text.find(quote).ok_or(AnchorError::AmbiguousQuote)?;
        // Look one Unicode scalar past the start, not past the entire quote,
        // so overlapping matches such as "aa" in "aaa" are ambiguous too.
        let next = first
            + text[first..]
                .chars()
                .next()
                .expect("nonempty quote")
                .len_utf8();
        if text[next..].contains(quote) {
            return Err(AnchorError::AmbiguousQuote);
        }
        Ok(range.start as usize + first)
    };
    let (start, end) = match &request.selector {
        SourceSelector::WholeDocument => (0, document.source.len()),
        SourceSelector::BlockRange { first, last } => {
            let (a, b) = (find(first)?, find(last)?);
            if a > b {
                return Err(AnchorError::InvalidRange);
            }
            (
                document.blocks[a].range.start as usize,
                document.blocks[b].range.end as usize,
            )
        }
        SourceSelector::QuoteRange {
            first,
            start_quote,
            last,
            end_quote,
        } => {
            let (a, b) = (find(first)?, find(last)?);
            if a > b {
                return Err(AnchorError::InvalidRange);
            }
            let start = quote_position(a, start_quote)?;
            let end_start = quote_position(b, end_quote)?;
            if end_start < start || end_start + end_quote.len() < start + start_quote.len() {
                return Err(AnchorError::InvalidRange);
            }
            (start, end_start + end_quote.len())
        }
    };
    let raw_text = document
        .source
        .get(start..end)
        .filter(|s| !s.trim().is_empty())
        .ok_or(AnchorError::InvalidRange)?
        .to_owned();
    let range = TextByteRange {
        start: start as u64,
        end: end as u64,
    };
    let block_ids = document
        .blocks
        .iter()
        .filter(|b| b.range.end > range.start && b.range.start < range.end)
        .map(|b| b.id.clone())
        .collect();
    // UI policy does not alter the mathematical input identity.
    let input_fingerprint =
        hash(&serde_json::to_vec(&(&document.snapshot.fingerprint, &range)).expect("simple tuple"));
    Ok(ResolvedDiscoverySelection {
        request,
        range,
        block_ids,
        raw_text,
        input_fingerprint,
    })
}
