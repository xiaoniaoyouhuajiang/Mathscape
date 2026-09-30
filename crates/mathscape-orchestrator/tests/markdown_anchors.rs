use mathscape_domain::{
    AnchoredMarkdown, DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoverySelectionRequest,
    SourceSelector, StableId,
};
use mathscape_orchestrator::{
    AnchorError, PAGE_MARKER_STRUCTURAL_KIND, anchor_markdown, is_page_marker_text,
    resolve_selection,
};

fn document(text: &str) -> AnchoredMarkdown {
    anchor_markdown(StableId::parse("test:markdown").unwrap(), None, text.into()).unwrap()
}

fn request(doc: &AnchoredMarkdown, selector: SourceSelector) -> DiscoverySelectionRequest {
    DiscoverySelectionRequest {
        version: DISCOVERY_SELECTION_VERSION.into(),
        snapshot: doc.snapshot.clone(),
        selector,
        admission: DiscoveryAdmission::ReviewBeforePublish,
    }
}

fn texts(doc: &AnchoredMarkdown) -> Vec<&str> {
    doc.blocks
        .iter()
        .map(|b| &doc.source[b.range.start as usize..b.range.end as usize])
        .collect()
}

#[test]
fn stable_utf8_ranges_keep_crlf_and_repeated_text_distinct() {
    let source = "\u{feff}# 定义 😀\r\n\r\n重复文字。\r\n\r\n重复文字。\r\n";
    let doc = document(source);
    assert_eq!(doc, document(source));
    assert!(
        doc.blocks
            .windows(2)
            .all(|b| b[0].range.end <= b[1].range.start)
    );
    for block in &doc.blocks {
        assert!(source.is_char_boundary(block.range.start as usize));
        assert!(source.is_char_boundary(block.range.end as usize));
    }
    let repeated: Vec<_> = doc
        .blocks
        .iter()
        .filter(|b| source[b.range.start as usize..b.range.end as usize].contains("重复"))
        .collect();
    assert_eq!(repeated.len(), 2);
    assert_ne!(repeated[0].id, repeated[1].id);
    let all = resolve_selection(&doc, request(&doc, SourceSelector::WholeDocument)).unwrap();
    assert_eq!(all.raw_text, source);
    assert_eq!(all.range.end, source.len() as u64);
}

#[test]
fn containers_and_display_formulas_stay_whole_and_code_is_not_math() {
    let source = "# Title\n\n- 一\n- 二\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n$$\na_1\n\n+a_2\n$$\n\n\\[\nx\n\n+y\n\\]\n\n~~~tex\n$$\n\ncode\n~~~\n\nAfter\n\n![pic][p]\n\n[p]: image.png\n";
    let doc = document(source);
    let chunks = texts(&doc);
    assert!(chunks.iter().any(|s| s.contains("- 一\n- 二")));
    assert!(
        chunks
            .iter()
            .any(|s| s.contains("| a | b |") && s.contains("| 1 | 2 |"))
    );
    assert!(chunks.iter().any(|s| s.contains("$$\na_1\n\n+a_2\n$$")));
    assert!(chunks.iter().any(|s| s.contains("\\[\nx\n\n+y\n\\]")));
    assert!(
        chunks
            .iter()
            .any(|s| s.contains("~~~tex\n$$\n\ncode\n~~~") && !s.contains("After"))
    );
    assert!(chunks.iter().any(|s| s.contains("[p]: image.png")));
}

#[test]
fn marker_pagination_is_a_nonsemantic_block_without_changing_source_ranges() {
    let source = "{21}------------------------------------------------\n\n定义：连续函数。\n\n{22}------------------------------------------------\n\n证明。\n";
    let doc = document(source);
    assert_eq!(doc.blocks.len(), 4);
    let marker_blocks: Vec<_> = doc
        .blocks
        .iter()
        .filter(|block| block.kind == PAGE_MARKER_STRUCTURAL_KIND)
        .collect();
    assert_eq!(marker_blocks.len(), 2);
    assert!(marker_blocks.iter().all(|block| {
        is_page_marker_text(&source[block.range.start as usize..block.range.end as usize])
    }));
    assert!(doc.blocks.iter().any(|block| {
        source[block.range.start as usize..block.range.end as usize].contains("定义：连续函数")
    }));
    assert!(doc.blocks.iter().any(|block| {
        source[block.range.start as usize..block.range.end as usize].contains("证明")
    }));
}

#[test]
fn selectors_reject_foreign_blocks_reversed_endpoints_and_stale_versions() {
    let doc = document("第一段\n\n第二段\n");
    let first = doc.blocks[0].id.clone();
    let last = doc.blocks[1].id.clone();
    let selector = SourceSelector::BlockRange {
        first: first.clone(),
        last: last.clone(),
    };
    let selected = resolve_selection(&doc, request(&doc, selector)).unwrap();
    assert_eq!(selected.raw_text, "第一段\n\n第二段\n");
    assert_eq!(selected.block_ids.len(), 2);
    assert!(matches!(
        resolve_selection(
            &doc,
            request(
                &doc,
                SourceSelector::BlockRange {
                    first: last,
                    last: first
                }
            )
        ),
        Err(AnchorError::InvalidRange)
    ));
    let other = document("different");
    assert!(matches!(
        resolve_selection(
            &doc,
            request(
                &doc,
                SourceSelector::BlockRange {
                    first: other.blocks[0].id.clone(),
                    last: other.blocks[0].id.clone()
                }
            )
        ),
        Err(AnchorError::UnknownBlock)
    ));
    let mut stale = request(&doc, SourceSelector::WholeDocument);
    stale.snapshot.anchor_scheme = "future-version".into();
    assert!(matches!(
        resolve_selection(&doc, stale),
        Err(AnchorError::StaleSnapshot)
    ));
    assert!(matches!(
        resolve_selection(&other, request(&doc, SourceSelector::WholeDocument)),
        Err(AnchorError::StaleSnapshot)
    ));
}

#[test]
fn quotes_use_exact_original_bytes_and_reject_ambiguity_including_overlap() {
    let doc = document("中文 😀 起点 aaa 终点\n\n另段结束\n");
    let make = |start: &str, end: &str, last: usize| SourceSelector::QuoteRange {
        first: doc.blocks[0].id.clone(),
        start_quote: start.into(),
        last: doc.blocks[last].id.clone(),
        end_quote: end.into(),
    };
    let selected = resolve_selection(&doc, request(&doc, make("😀", "结束", 1))).unwrap();
    assert_eq!(selected.raw_text, "😀 起点 aaa 终点\n\n另段结束");
    assert_eq!(selected.range.start, "中文 ".len() as u64);
    for start in ["", "missing", "aa"] {
        assert!(matches!(
            resolve_selection(&doc, request(&doc, make(start, "终点", 0))),
            Err(AnchorError::AmbiguousQuote)
        ));
    }
    assert!(matches!(
        resolve_selection(&doc, request(&doc, make("终点", "起点", 0))),
        Err(AnchorError::InvalidRange)
    ));
    assert!(matches!(
        resolve_selection(&doc, request(&doc, make("起点 aaa 终点", "aaa", 0))),
        Err(AnchorError::InvalidRange)
    ));
}

#[test]
fn policy_is_not_input_identity_and_conversion_context_is() {
    let doc = document("正文");
    let mut auto = request(&doc, SourceSelector::WholeDocument);
    let reviewed = resolve_selection(&doc, auto.clone()).unwrap();
    auto.admission = DiscoveryAdmission::AutomaticCandidate;
    let automatic = resolve_selection(&doc, auto).unwrap();
    assert_eq!(reviewed.input_fingerprint, automatic.input_fingerprint);
    let other = anchor_markdown(
        doc.snapshot.artifact_id.clone(),
        Some(StableId::parse("test:conversion").unwrap()),
        doc.source.clone(),
    )
    .unwrap();
    assert_eq!(doc.blocks, other.blocks);
    assert_ne!(doc.snapshot.fingerprint, other.snapshot.fingerprint);
    assert!(matches!(
        resolve_selection(&other, request(&doc, SourceSelector::WholeDocument)),
        Err(AnchorError::StaleSnapshot)
    ));
}

#[test]
fn empty_whitespace_oversized_and_unclosed_math_have_honest_boundaries() {
    for source in ["", " \r\n\n"] {
        let doc = document(source);
        assert!(doc.blocks.is_empty());
        assert!(matches!(
            resolve_selection(&doc, request(&doc, SourceSelector::WholeDocument)),
            Err(AnchorError::InvalidRange)
        ));
    }
    let doc = document("Before\n\n\\[\nx\n\nmissing close\n");
    assert_eq!(doc.blocks.last().unwrap().kind, "math_unclosed");
    assert!(texts(&doc).last().unwrap().contains("missing close"));
    assert!(matches!(
        anchor_markdown(
            doc.snapshot.artifact_id,
            None,
            "x".repeat(16 * 1024 * 1024 + 1)
        ),
        Err(AnchorError::TooLarge)
    ));
}

#[test]
fn indented_math_and_single_line_formula_with_suffix_do_not_consume_next_paragraph() {
    let doc = document("  \\[\nx\n\n+y\n  \\]\n\n\\[z\\] trailing\n\nNext\n");
    let chunks = texts(&doc);
    assert!(
        chunks
            .iter()
            .any(|s| s.contains("x\n\n+y") && !s.contains("trailing"))
    );
    assert!(
        chunks
            .iter()
            .any(|s| s.contains("trailing") && !s.contains("Next"))
    );
}

#[test]
#[ignore = "set MATHSCAPE_MARKDOWN_FIXTURE to a local converted textbook"]
fn anchors_local_markdown_without_omitting_source_content() {
    let path = std::env::var("MATHSCAPE_MARKDOWN_FIXTURE").expect("fixture path");
    let source = std::fs::read_to_string(path).unwrap();
    let doc = document(&source);
    assert!(!doc.blocks.is_empty());
    let mut cursor = 0;
    for block in &doc.blocks {
        assert!(source[cursor..block.range.start as usize].trim().is_empty());
        cursor = block.range.end as usize;
    }
    assert!(source[cursor..].trim().is_empty());
    assert_eq!(
        resolve_selection(&doc, request(&doc, SourceSelector::WholeDocument))
            .unwrap()
            .raw_text,
        source
    );
}
