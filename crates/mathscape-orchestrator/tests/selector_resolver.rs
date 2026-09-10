use mathscape_domain::{
    DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoveryInput, DiscoveryProposal,
    DiscoverySelectionRequest, SourceSelector, StableId,
};
use mathscape_orchestrator::{
    DISCOVERY_MODEL_INPUT_VERSION, DiscoveryModelInputSegment, SelectorResolutionCode,
    SelectorResolver, SelectorRewriteKind, anchor_markdown, build_discovery_input,
    resolve_selection,
};
use serde_json::json;

fn input(source: &str) -> DiscoveryInput {
    let document = anchor_markdown(
        StableId::parse("test:selector-resolver").unwrap(),
        None,
        source.into(),
    )
    .unwrap();
    let selection = resolve_selection(
        &document,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: document.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    build_discovery_input(&document, &selection, Default::default()).unwrap()
}

fn proposal(input: &DiscoveryInput, selector: serde_json::Value) -> DiscoveryProposal {
    serde_json::from_value(json!({
        "version": "mathscape-discovery-proposal-3",
        "input_fingerprint": input.fingerprint,
        "units": [{
            "temp_id": "u1", "unit_kind": "narrative", "source_labels": [],
            "source_title": null, "display_title": null, "selectors": [selector.clone()],
            "parent": null, "completeness": "complete"
        }],
        "relations": [], "references": [], "issues": [],
        "coverage": [{"selector": selector, "disposition": {"status": "processed"}}]
    }))
    .unwrap()
}

#[test]
fn model_projection_replaces_stable_identifiers_and_offsets_with_local_handles() {
    let input = input("第一段。\n\n第二段。\n");
    let resolver = SelectorResolver::from_input(&input);
    let projected = resolver.model_input(&input);

    assert_eq!(projected.version, DISCOVERY_MODEL_INPUT_VERSION);
    assert_eq!(projected.input_fingerprint, input.fingerprint);
    assert_eq!(resolver.bindings().len(), 2);
    assert_eq!(resolver.bindings()[0].handle(), "b1");
    assert_eq!(resolver.bindings()[1].handle(), "b2");
    assert!(
        resolver.bindings()[0]
            .block_id()
            .starts_with("block:sha256:")
    );
    assert!(matches!(
        &projected.requested.segments[0],
        DiscoveryModelInputSegment::Block { block_ref, .. } if block_ref == "b1"
    ));
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(!encoded.contains("block:sha256:"));
    assert!(!encoded.contains("physical_range"));
    assert!(!encoded.contains("source_range"));
}

#[test]
fn resolver_binds_every_selector_back_to_the_stable_block_id() {
    let input = input("一段完整原文。\n");
    let resolver = SelectorResolver::from_input(&input);
    let proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    let resolved = resolver.resolve(proposal).unwrap();
    let block_id = resolver.bindings()[0].block_id();

    assert!(
        resolved.proposal().units[0]
            .selectors
            .iter()
            .all(|selector| serde_json::to_string(selector).unwrap().contains(block_id))
    );
    assert_eq!(
        resolved
            .rewrites()
            .iter()
            .filter(|rewrite| rewrite.kind() == SelectorRewriteKind::BlockHandle)
            .count(),
        4
    );
}

#[test]
fn resolver_binds_source_label_anchors_and_preserves_plural_labels() {
    let input = input("公式 (20) 与公式 (21)。\n");
    let resolver = SelectorResolver::from_input(&input);
    let mut proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    proposal.units[0].source_labels = serde_json::from_value(json!([
        {
            "surface_form":"(20)",
            "selector":{"kind":"quote_range", "first":"b1",
                "start":{"quote":"(20)","occurrence":null,"prefix":null,"suffix":null},
                "last":"b1",
                "end":{"quote":"(20)","occurrence":null,"prefix":null,"suffix":null}}
        },
        {
            "surface_form":"(21)",
            "selector":{"kind":"quote_range", "first":"b1",
                "start":{"quote":"(21)","occurrence":null,"prefix":null,"suffix":null},
                "last":"b1",
                "end":{"quote":"(21)","occurrence":null,"prefix":null,"suffix":null}}
        }
    ]))
    .unwrap();

    let resolved = resolver.resolve(proposal).unwrap();
    let labels = &resolved.proposal().units[0].source_labels;
    assert_eq!(labels.len(), 2);
    let encoded = serde_json::to_value(labels).unwrap();
    assert_eq!(
        encoded[0]["selector"]["first"],
        resolver.bindings()[0].block_id()
    );
    assert_eq!(encoded[0]["selector"]["start"]["occurrence"], 1);
    assert_eq!(encoded[1]["surface_form"], "(21)");
}

#[test]
fn visible_canonical_block_ids_remain_replayable_without_rewrites() {
    let input = input("一段完整原文。\n");
    let resolver = SelectorResolver::from_input(&input);
    let block_id = resolver.bindings()[0].block_id().to_owned();
    let proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":block_id, "last":block_id}),
    );

    let resolved = resolver.resolve(proposal).unwrap();
    assert!(resolved.rewrites().is_empty());
}

#[test]
fn exact_ambiguity_is_reported_instead_of_guessing_an_occurrence() {
    let input = input("可得式(26)；从式(26)又得到下一式。\n");
    let resolver = SelectorResolver::from_input(&input);
    let mut proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    proposal.references.push(
        serde_json::from_value(json!({
            "source":"u1",
            "selector":{"kind":"quote_range", "first":"b1",
                "start":{"quote":"式(26)","occurrence":null,"prefix":null,"suffix":null},
                "last":"b1",
                "end":{"quote":"式(26)","occurrence":null,"prefix":null,"suffix":null}},
            "surface_form":"式(26)", "target_label":"26", "target":null
        }))
        .unwrap(),
    );

    let report = resolver.resolve(proposal).unwrap_err();
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code(),
        SelectorResolutionCode::AmbiguousQuote
    );
    assert_eq!(
        report.diagnostics()[0].path(),
        "proposal.references[0].selector"
    );
}

#[test]
fn safe_normalization_removes_only_presentation_noise_and_returns_exact_source_text() {
    let source = "关系式 $$z + \\bar{z} = 2\\operatorname{Re} z, \\quad z - \\bar{z} = 2\\operatorname{Im} z;$$\n";
    let input = input(source);
    let resolver = SelectorResolver::from_input(&input);
    let mut proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    proposal.issues.push(
        serde_json::from_value(json!({
            "kind":"suspected_conversion_error", "unit":"u1",
            "evidence":[{"kind":"quote_range", "first":"b1",
                "start":{"quote":"$z - \\bar{z} = 2\\operatorname{Im} z$","occurrence":null,"prefix":null,"suffix":null},
                "last":"b1",
                "end":{"quote":"$z - \\bar{z} = 2\\operatorname{Im} z$","occurrence":null,"prefix":null,"suffix":null}}],
            "observation":"疑似缺失因子", "context_request":null
        }))
        .unwrap(),
    );

    let resolved = resolver.resolve(proposal).unwrap();
    let selector = &resolved.proposal().issues[0].evidence[0];
    let encoded = serde_json::to_value(selector).unwrap();
    assert_eq!(
        encoded["start"]["quote"],
        "z - \\bar{z} = 2\\operatorname{Im} z"
    );
    assert_eq!(encoded["start"]["occurrence"], 1);
    assert!(
        resolved
            .rewrites()
            .iter()
            .any(|rewrite| rewrite.kind() == SelectorRewriteKind::SafeQuoteNormalization)
    );
    assert!(
        resolved
            .rewrites()
            .iter()
            .any(|rewrite| rewrite.kind() == SelectorRewriteKind::OccurrenceBinding)
    );
}

#[test]
fn normalization_does_not_treat_mathematical_operators_as_equivalent() {
    let input = input("条件为 $x ≤ 1$。\n");
    let resolver = SelectorResolver::from_input(&input);
    let mut proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    proposal.issues.push(
        serde_json::from_value(json!({
            "kind":"suspected_conversion_error", "unit":"u1",
            "evidence":[{"kind":"quote_range", "first":"b1",
                "start":{"quote":"$x < 1$","occurrence":null,"prefix":null,"suffix":null},
                "last":"b1",
                "end":{"quote":"$x < 1$","occurrence":null,"prefix":null,"suffix":null}}],
            "observation":"不等号可能不同", "context_request":null
        }))
        .unwrap(),
    );

    let report = resolver.resolve(proposal).unwrap_err();
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == SelectorResolutionCode::QuoteNotFound)
    );
}

#[test]
fn occurrence_and_context_disambiguate_without_expanding_the_selected_quote() {
    let input = input("可得式(26)；从式(26)易推出式(27)。\n");
    let resolver = SelectorResolver::from_input(&input);
    let mut proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"b1", "last":"b1"}),
    );
    proposal.references.push(
        serde_json::from_value(json!({
            "source":"u1",
            "selector":{"kind":"quote_range", "first":"b1",
                "start":{"quote":"式(26)","occurrence":2,"prefix":"从","suffix":"易推出"},
                "last":"b1",
                "end":{"quote":"式(26)","occurrence":2,"prefix":"从","suffix":"易推出"}},
            "surface_form":"式(26)", "target_label":"26", "target":null
        }))
        .unwrap(),
    );

    let resolved = resolver.resolve(proposal).unwrap();
    let selector = serde_json::to_value(&resolved.proposal().references[0].selector).unwrap();
    assert_eq!(selector["start"]["quote"], "式(26)");
    assert_eq!(selector["start"]["occurrence"], 2);
    assert_eq!(selector["start"]["prefix"], "从");
    assert_eq!(selector["start"]["suffix"], "易推出");
}

#[test]
fn zero_or_out_of_range_occurrence_is_rejected() {
    for occurrence in [0, 3] {
        let input = input("式(26)与式(26)。\n");
        let resolver = SelectorResolver::from_input(&input);
        let mut proposal = proposal(
            &input,
            json!({"kind":"block_range", "first":"b1", "last":"b1"}),
        );
        proposal.references.push(
            serde_json::from_value(json!({
                "source":"u1",
                "selector":{"kind":"quote_range", "first":"b1",
                    "start":{"quote":"式(26)","occurrence":occurrence,"prefix":null,"suffix":null},
                    "last":"b1",
                    "end":{"quote":"式(26)","occurrence":occurrence,"prefix":null,"suffix":null}},
                "surface_form":"式(26)", "target_label":"26", "target":null
            }))
            .unwrap(),
        );
        let report = resolver.resolve(proposal).unwrap_err();
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == SelectorResolutionCode::InvalidDisambiguator
        }));
    }
}

#[test]
fn unknown_handles_are_a_resolution_error_before_source_validation() {
    let input = input("正文。\n");
    let resolver = SelectorResolver::from_input(&input);
    let proposal = proposal(
        &input,
        json!({"kind":"block_range", "first":"invented", "last":"b1"}),
    );

    let report = resolver.resolve(proposal).unwrap_err();
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == SelectorResolutionCode::UnknownBlockRef)
    );
}
