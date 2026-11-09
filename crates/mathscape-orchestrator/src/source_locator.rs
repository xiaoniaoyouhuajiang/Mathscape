//! Locator, identity and hash rules shared by content discovery and Phase2
//! `propose_changes`.
//!
//! Everything here used to live in `selector_resolver`, `discovery_validation`
//! and `source_registration`. It is collected in one module so both write
//! paths derive the same SourceSpan identity, compare quotes with the same
//! TeX-aware normalization and record the same text hash for one range.

use mathscape_domain::{ContentHash, StableId, TextByteRange};

use crate::unit_discovery::fingerprint;

/// Temporary identifier rule (content discovery `temp_id`, proposal
/// `local_ref`): at most 64 bytes, first byte ASCII alphanumeric, the rest
/// ASCII alphanumeric or `_`, `-`, `.`.
pub fn valid_temp_id(value: &str) -> bool {
    value.len() <= 64
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// `mathscape:<kind>/<hex>` from a sha256 fingerprint.
pub fn stable_hash_id(kind: &str, fingerprint: &ContentHash) -> Result<StableId, String> {
    StableId::parse(format!(
        "mathscape:{kind}/{}",
        fingerprint
            .as_str()
            .strip_prefix("sha256:")
            .unwrap_or(fingerprint.as_str())
    ))
    .map_err(|error| error.to_string())
}

/// Deterministic SourceSpan identity for one immutable Markdown snapshot and
/// one half-open byte range. Contains no quote text, granularity or other
/// presentation metadata, so every write path agrees on the id.
pub fn markdown_span_id(artifact_id: &StableId, range: &TextByteRange) -> StableId {
    let identity = fingerprint(&("markdown-source-span-1", artifact_id, range));
    stable_hash_id("source-span", &identity).expect("hash-derived StableId is valid")
}

/// Text hash stored in `SourceSpan::normalized_text_hash`.
///
/// The value is `sha256` over the **JSON serialization of the exact range
/// text** (a JSON string literal, quotes and escapes included), i.e. the
/// orchestrator's general `fingerprint(&text)`. Content discovery has always
/// stored this value; Phase2 proposals reuse it instead of introducing a
/// second algorithm, so existing registered spans keep their revisions. The
/// field name says "normalized" for historical reasons only: no TeX or
/// whitespace normalization is applied before hashing.
pub fn source_text_hash(text: &str) -> ContentHash {
    fingerprint(&text)
}

/// Whether `quote` denotes exactly `range_text` (the bytes already cut from
/// the snapshot at the proposed `[start, end)`): exact equality first, then
/// equality after TeX-aware normalization of both sides. This never searches
/// a larger text, so an identical quote elsewhere in the document cannot make
/// a wrong range pass.
pub fn quote_matches_range(range_text: &str, quote: &str) -> bool {
    if range_text == quote {
        return true;
    }
    let normalize = |text: &str| {
        normalized_locator_string(strip_outer_math_delimiters(text))
            .trim()
            .to_owned()
    };
    let wanted = normalize(quote);
    !wanted.is_empty() && normalize(range_text) == wanted
}

/// Normalized text of `text` (whitespace runs and TeX spacing commands
/// collapse to one space).
pub fn normalized_locator_string(text: &str) -> String {
    normalize_locator_text(text, false)
        .chars
        .into_iter()
        .collect()
}

/// Shared matching policy for v3 quotes and staged single occurrences:
/// searches `quote` in `text`, exact first and normalized as a fallback.
pub fn locator_matches(text: &str, quote: &str) -> (Vec<(usize, usize)>, bool) {
    let exact = overlapping_matches(text, quote)
        .into_iter()
        .map(|start| (start, start + quote.len()))
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return (exact, false);
    }
    let source = normalize_locator_text(text, true);
    let wanted = normalize_locator_text(strip_outer_math_delimiters(quote), false);
    (normalized_matches(&source, &wanted.chars), true)
}

pub(crate) fn overlapping_matches(text: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut cursor = 0;
    while cursor <= text.len().saturating_sub(needle.len()) {
        let Some(relative) = text[cursor..].find(needle) else {
            break;
        };
        let start = cursor + relative;
        starts.push(start);
        cursor = start
            + text[start..]
                .chars()
                .next()
                .expect("a nonempty match starts on a character")
                .len_utf8();
    }
    starts
}

pub(crate) struct NormalizedLocatorText {
    pub(crate) chars: Vec<char>,
    source_spans: Vec<(usize, usize)>,
}

pub(crate) fn normalize_locator_text(text: &str, track_source: bool) -> NormalizedLocatorText {
    let mut chars = Vec::new();
    let mut source_spans = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let tail = &text[index..];
        if let Some(len) = tex_spacing_prefix(tail) {
            push_normalized_space(&mut chars, &mut source_spans, index, index + len);
            index += len;
            continue;
        }
        let ch = tail
            .chars()
            .next()
            .expect("index remains on a character boundary");
        let end = index + ch.len_utf8();
        if ch.is_whitespace() {
            let mut whitespace_end = end;
            while whitespace_end < text.len() {
                let next = text[whitespace_end..]
                    .chars()
                    .next()
                    .expect("whitespace scan remains in bounds");
                if !next.is_whitespace() {
                    break;
                }
                whitespace_end += next.len_utf8();
            }
            push_normalized_space(&mut chars, &mut source_spans, index, whitespace_end);
            index = whitespace_end;
            continue;
        }
        chars.push(ch);
        source_spans.push((index, end));
        index = end;
    }
    if !track_source {
        source_spans.clear();
    }
    NormalizedLocatorText {
        chars,
        source_spans,
    }
}

pub(crate) fn strip_outer_math_delimiters(text: &str) -> &str {
    let text = text.trim();
    if text.len() >= 4 && text.starts_with("$$") && text.ends_with("$$") {
        &text[2..text.len() - 2]
    } else if text.len() >= 2 && text.starts_with('$') && text.ends_with('$') {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn tex_spacing_prefix(text: &str) -> Option<usize> {
    ["\\qquad", "\\quad", "\\,", "\\:", "\\;", "\\!", "\\ "]
        .into_iter()
        .find_map(|command| text.starts_with(command).then_some(command.len()))
}

fn push_normalized_space(
    chars: &mut Vec<char>,
    source_spans: &mut Vec<(usize, usize)>,
    start: usize,
    end: usize,
) {
    if chars.last() == Some(&' ') {
        if let Some(span) = source_spans.last_mut() {
            span.1 = end;
        }
    } else {
        chars.push(' ');
        source_spans.push((start, end));
    }
}

fn normalized_matches(source: &NormalizedLocatorText, wanted: &[char]) -> Vec<(usize, usize)> {
    if wanted.is_empty() || wanted.len() > source.chars.len() {
        return Vec::new();
    }
    source
        .chars
        .windows(wanted.len())
        .enumerate()
        .filter(|(_, window)| *window == wanted)
        .map(|(index, _)| {
            (
                source.source_spans[index].0,
                source.source_spans[index + wanted.len() - 1].1,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_must_denote_the_cut_range_not_any_occurrence() {
        let text = "Let $G$ be a group. Let $G$ be a group.";
        assert!(quote_matches_range(&text[0..19], "Let $G$ be a group."));
        assert!(quote_matches_range(&text[4..7], "G"));
        assert!(quote_matches_range("a  \\quad b", "a b"));
        assert!(!quote_matches_range(&text[0..18], "Let $G$ be a group."));
        assert!(!quote_matches_range(&text[1..19], "Let $G$ be a group."));
        assert!(!quote_matches_range("abc", "   "));
    }

    #[test]
    fn source_text_hash_is_the_json_fingerprint_of_the_text() {
        use sha2::{Digest, Sha256};
        let expected = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec("a\"b").unwrap())
        );
        assert_eq!(source_text_hash("a\"b").as_str(), expected);
    }
}
