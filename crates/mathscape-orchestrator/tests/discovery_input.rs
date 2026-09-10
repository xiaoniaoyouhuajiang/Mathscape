use mathscape_domain::{
    AnchoredMarkdown, DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoveryInputSegment,
    DiscoverySelectionRequest, SourceSelector, StableId,
};
use mathscape_orchestrator::{
    DiscoveryInputError, DiscoveryInputLimits, anchor_markdown, build_discovery_input,
    resolve_selection, validate_discovery_proposal,
};

fn document(source: &str) -> AnchoredMarkdown {
    anchor_markdown(
        StableId::parse("test:discovery-input").unwrap(),
        Some(StableId::parse("test:conversion").unwrap()),
        source.into(),
    )
    .unwrap()
}

fn resolve(
    doc: &AnchoredMarkdown,
    selector: SourceSelector,
) -> mathscape_domain::ResolvedDiscoverySelection {
    resolve_selection(
        doc,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: doc.snapshot.clone(),
            selector,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap()
}

fn reconstructed(input: &mathscape_domain::DiscoveryInput) -> String {
    input
        .requested
        .segments
        .iter()
        .map(DiscoveryInputSegment::text)
        .collect()
}

#[test]
fn builds_a_lossless_replayable_input_without_admission_policy() {
    let doc = document("  \n# 定义 😀\n\n第一段。\n\n$$\nx_n \\to A\n$$\n\n尾部\n");
    let selected = resolve(&doc, SourceSelector::WholeDocument);
    let input = build_discovery_input(&doc, &selected, DiscoveryInputLimits::default()).unwrap();

    assert_eq!(reconstructed(&input), doc.source);
    assert_eq!(input.requested.utf8_bytes(), doc.source.len());
    assert_eq!(input.requested.block_count(), doc.blocks.len());
    assert!(input.boundary.at_document_start);
    assert!(input.boundary.at_document_end);
    assert!(
        input
            .requested
            .segments
            .iter()
            .all(|segment| !segment.clipped_start() && !segment.clipped_end())
    );
    assert!(input.context.is_empty());
    let encoded = serde_json::to_string(&input).unwrap();
    assert!(!encoded.contains("review_before_publish"));
    assert!(!encoded.contains("automatic_candidate"));
    assert!(encoded.contains("segment_kind"));
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["version"], "mathscape-discovery-input-2");
    assert!(value.get("utf8_bytes").is_none());
    assert!(value.get("block_count").is_none());
    assert_eq!(value["boundary"].as_object().unwrap().len(), 2);
    for segment in value["requested"]["segments"].as_array().unwrap() {
        assert!(segment.get("clipped_start").is_none());
        assert!(segment.get("clipped_end").is_none());
    }
    let replayed: mathscape_domain::DiscoveryInput = serde_json::from_str(&encoded).unwrap();
    assert_eq!(replayed, input);
}

#[test]
fn block_input_preserves_inter_block_gaps_and_reports_outside_source() {
    let doc = document("Before\n\nOne 中文\n\nTwo\r\n\r\nAfter\n");
    let selection = resolve(
        &doc,
        SourceSelector::BlockRange {
            first: doc.blocks[1].id.clone(),
            last: doc.blocks[2].id.clone(),
        },
    );
    let input = build_discovery_input(&doc, &selection, DiscoveryInputLimits::default()).unwrap();

    assert_eq!(reconstructed(&input), "One 中文\n\nTwo\r\n");
    assert!(!input.boundary.at_document_start);
    assert!(!input.boundary.at_document_end);
    assert!(input.requested.segments.iter().any(|segment| matches!(
        segment,
        DiscoveryInputSegment::Gap { text, .. } if text == "\n"
    )));
}

#[test]
fn quote_input_marks_clipped_physical_blocks() {
    let doc = document("prefix START middle END suffix\n");
    let block = doc.blocks[0].id.clone();
    let selection = resolve(
        &doc,
        SourceSelector::QuoteRange {
            first: block.clone(),
            start_quote: "START".into(),
            last: block,
            end_quote: "END".into(),
        },
    );
    let input = build_discovery_input(&doc, &selection, DiscoveryInputLimits::default()).unwrap();

    assert_eq!(reconstructed(&input), "START middle END");
    assert!(input.requested.segments[0].clipped_start());
    assert!(input.requested.segments[0].clipped_end());
    assert!(matches!(
        &input.requested.segments[0],
        DiscoveryInputSegment::Block {
            text,
            physical_range,
            source_range,
            ..
        } if text == "START middle END" && physical_range != source_range
    ));
}

#[test]
fn input_identity_is_deterministic_and_ignores_admission_but_not_range() {
    let doc = document("One\n\nTwo\n");
    let reviewed = resolve(&doc, SourceSelector::WholeDocument);
    let mut automatic = reviewed.clone();
    automatic.request.admission = DiscoveryAdmission::AutomaticCandidate;
    let one = build_discovery_input(&doc, &reviewed, DiscoveryInputLimits::default()).unwrap();
    let two = build_discovery_input(&doc, &automatic, DiscoveryInputLimits::default()).unwrap();
    assert_eq!(one, two);

    let partial = resolve(
        &doc,
        SourceSelector::BlockRange {
            first: doc.blocks[0].id.clone(),
            last: doc.blocks[0].id.clone(),
        },
    );
    let partial = build_discovery_input(&doc, &partial, DiscoveryInputLimits::default()).unwrap();
    assert_ne!(one.fingerprint, partial.fingerprint);
}

#[test]
fn rejects_limits_instead_of_silently_truncating() {
    assert!(matches!(
        DiscoveryInputLimits::new(0, 1),
        Err(DiscoveryInputError::InvalidLimits)
    ));
    let doc = document("One\n\nTwo\n");
    let selected = resolve(&doc, SourceSelector::WholeDocument);
    assert!(matches!(
        build_discovery_input(&doc, &selected, DiscoveryInputLimits::new(3, 10).unwrap()),
        Err(DiscoveryInputError::ByteLimit { .. })
    ));
    assert!(matches!(
        build_discovery_input(&doc, &selected, DiscoveryInputLimits::new(100, 1).unwrap()),
        Err(DiscoveryInputError::BlockLimit { .. })
    ));
}

#[test]
fn rejects_tampered_resolution_and_malformed_documents() {
    let doc = document("One\n\nTwo\n");
    let mut selected = resolve(&doc, SourceSelector::WholeDocument);
    selected.raw_text.push_str("tampered");
    assert!(matches!(
        build_discovery_input(&doc, &selected, DiscoveryInputLimits::default()),
        Err(DiscoveryInputError::ResolvedSelectionMismatch)
    ));

    let mut malformed = doc.clone();
    malformed.blocks[1].range.start = malformed.blocks[0].range.start;
    let selected = resolve(&doc, SourceSelector::WholeDocument);
    assert!(matches!(
        build_discovery_input(&malformed, &selected, DiscoveryInputLimits::default()),
        Err(DiscoveryInputError::InvalidDocument)
    ));
}

#[test]
fn proposal_example_uses_real_input_anchors_without_model_offsets() {
    let doc = document(
        "习题 1：设 $a_n=a$，用定义证明 $a_n$ 收敛于 $a$。\n\n解答：任取 $\\epsilon>0$，取 $N=1$。当 $n>N$ 时，$|a_n-a|=0<\\epsilon$。因此收敛于 $a$。\n",
    );
    assert_eq!(doc.blocks.len(), 2);
    let selection = resolve(&doc, SourceSelector::WholeDocument);
    let input = build_discovery_input(&doc, &selection, DiscoveryInputLimits::default()).unwrap();
    let example = include_str!("../../mathscape-domain/tests/fixtures/discovery-proposal.json")
        .replace("b-problem", &doc.blocks[0].id)
        .replace("b-solution", &doc.blocks[1].id);
    let mut value: serde_json::Value = serde_json::from_str(&example).unwrap();
    value["input_fingerprint"] = serde_json::to_value(&input.fingerprint).unwrap();
    let proposal: mathscape_domain::DiscoveryProposal = serde_json::from_value(value).unwrap();
    assert_eq!(proposal.input_fingerprint, input.fingerprint);
    let validated = validate_discovery_proposal(&input, proposal, Default::default()).unwrap();
    let evidence = validated.relations()[0].evidence()[0].text();
    assert!(evidence.starts_with("解答："));
    assert!(evidence.ends_with("因此收敛于 $a$。"));
}
