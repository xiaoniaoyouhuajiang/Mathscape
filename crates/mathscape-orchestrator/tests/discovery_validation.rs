use mathscape_domain::{
    AnchoredMarkdown, DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoveryInput,
    DiscoveryProposal, DiscoverySelectionRequest, SourceSelector, StableId,
};
use mathscape_orchestrator::{
    DiscoveryValidationCode, DiscoveryValidationLimits, ValidatedProposalStatus, anchor_markdown,
    build_discovery_input, resolve_selection, validate_discovery_proposal,
};
use serde_json::{Value, json};

const SOURCE: &str = "习题 1：设 $a_n=a$，用定义证明 $a_n$ 收敛于 $a$。\n\n解答：任取 $\\epsilon>0$，取 $N=1$。当 $n>N$ 时，$|a_n-a|=0<\\epsilon$。因此收敛于 $a$。\n";

fn document(source: &str) -> AnchoredMarkdown {
    anchor_markdown(
        StableId::parse("test:proposal-validation").unwrap(),
        Some(StableId::parse("test:conversion").unwrap()),
        source.into(),
    )
    .unwrap()
}

fn input_for(doc: &AnchoredMarkdown, selector: SourceSelector) -> DiscoveryInput {
    let selection = resolve_selection(
        doc,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: doc.snapshot.clone(),
            selector,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    build_discovery_input(doc, &selection, Default::default()).unwrap()
}

fn valid_case() -> (AnchoredMarkdown, DiscoveryInput, Value) {
    let doc = document(SOURCE);
    assert_eq!(doc.blocks.len(), 2);
    let input = input_for(&doc, SourceSelector::WholeDocument);
    let source = include_str!("../../mathscape-domain/tests/fixtures/discovery-proposal.json")
        .replace("b-problem", &doc.blocks[0].id)
        .replace("b-solution", &doc.blocks[1].id);
    let mut proposal: Value = serde_json::from_str(&source).unwrap();
    proposal["input_fingerprint"] = serde_json::to_value(&input.fingerprint).unwrap();
    (doc, input, proposal)
}

fn validate(
    input: &DiscoveryInput,
    value: Value,
) -> Result<
    mathscape_orchestrator::ValidatedDiscoveryProposal,
    mathscape_orchestrator::DiscoveryValidationReport,
> {
    let proposal: DiscoveryProposal = serde_json::from_value(value).unwrap();
    validate_discovery_proposal(input, proposal, DiscoveryValidationLimits::default())
}

fn has(
    report: &mathscape_orchestrator::DiscoveryValidationReport,
    code: DiscoveryValidationCode,
) -> bool {
    report
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == code)
}

#[test]
fn validates_and_normalizes_the_minimal_proposal() {
    let (_, input, proposal) = valid_case();
    let validated = validate(&input, proposal).unwrap();
    assert_eq!(validated.input_fingerprint(), &input.fingerprint);
    assert_eq!(validated.status(), ValidatedProposalStatus::Complete);
    assert_eq!(validated.units().len(), 2);
    assert_eq!(validated.relations().len(), 1);
    assert_eq!(validated.coverage().len(), 1);
    assert!(validated.references().is_empty());
    assert!(validated.issues().is_empty());
    assert_eq!(
        validated.units()[0].selectors()[0].text(),
        &SOURCE[..SOURCE.find("\n\n").unwrap() + 1]
    );
    let evidence = validated.relations()[0].evidence()[0].text();
    assert!(evidence.starts_with("解答："));
    assert!(evidence.ends_with("因此收敛于 $a$。"));
}

#[test]
fn rejects_tampered_inputs_and_wrong_proposal_fingerprints() {
    let (_, input, mut proposal) = valid_case();
    proposal["input_fingerprint"] = json!(format!("sha256:{}", "2".repeat(64)));
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::FingerprintMismatch));

    let (_, mut input, proposal) = valid_case();
    if let mathscape_domain::DiscoveryInputSegment::Block { text, .. } =
        &mut input.requested.segments[0]
    {
        text.replace_range(.."习题".len(), "题目");
    }
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InputIntegrity));
}

#[test]
fn selectors_are_confined_to_visible_requested_blocks() {
    let (_, input, mut proposal) = valid_case();
    proposal["units"][0]["selectors"][0]["first"] = json!("block:outside");
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidSelector));

    let doc = document("prefix START middle END suffix\n");
    let block = doc.blocks[0].id.clone();
    let input = input_for(
        &doc,
        SourceSelector::QuoteRange {
            first: block.clone(),
            start_quote: "START".into(),
            last: block.clone(),
            end_quote: "END".into(),
        },
    );
    let proposal: DiscoveryProposal = serde_json::from_value(json!({
        "version": "mathscape-discovery-proposal-1",
        "input_fingerprint": input.fingerprint,
        "units": [{"temp_id": "u", "unit_kind": "narrative", "source_label": null,
            "source_title": null, "display_title": null,
            "selectors": [{"kind": "block_range", "first": block, "last": block}],
            "parent": null, "completeness": "complete"}],
        "relations": [], "references": [], "issues": [],
        "coverage": [{"selector": {"kind": "block_range", "first": block, "last": block},
            "disposition": {"status": "processed"}}]
    }))
    .unwrap();
    let report = validate_discovery_proposal(&input, proposal, Default::default()).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidSelector));
}

#[test]
fn validates_graph_closure_cycles_relation_types_and_evidence_ownership() {
    let (_, input, mut proposal) = valid_case();
    proposal["units"][0]["parent"] = json!("u-solution");
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::ParentCycle));

    let (_, input, mut proposal) = valid_case();
    proposal["relations"][0]["source"] = json!("u-problem");
    proposal["relations"][0]["target"] = json!("u-solution");
    proposal["relations"][0]["evidence"] = proposal["units"][0]["selectors"].clone();
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidRelation));

    let (_, input, mut proposal) = valid_case();
    proposal["relations"][0]["evidence"] = proposal["units"][0]["selectors"].clone();
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidEvidence));

    let (_, input, mut proposal) = valid_case();
    proposal["references"] = json!([{
        "source": "u-solution", "selector": proposal["units"][1]["selectors"][0],
        "surface_form": "原文中不存在", "target_label": "1", "target": "u-problem"
    }]);
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidEvidence));
}

#[test]
fn overlap_requires_nesting_or_an_explains_relation() {
    let (_, input, mut proposal) = valid_case();
    proposal["units"][0]["selectors"] = json!([proposal["coverage"][0]["selector"].clone()]);
    proposal["units"][1]["parent"] = Value::Null;
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::OverlappingUnits));

    let (_, input, mut proposal) = valid_case();
    proposal["units"][0]["selectors"] = json!([proposal["coverage"][0]["selector"].clone()]);
    // Existing parent relation explains the overlap.
    assert!(validate(&input, proposal).is_ok());
}

#[test]
fn coverage_must_span_non_whitespace_and_units_must_be_processed() {
    let (_, input, mut proposal) = valid_case();
    proposal["coverage"][0]["selector"] = proposal["units"][0]["selectors"][0].clone();
    let report = validate(&input, proposal).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::CoverageGap));
    assert!(has(&report, DiscoveryValidationCode::InvalidCoverage));

    let (_, input, mut proposal) = valid_case();
    proposal["coverage"] = json!([
        {"selector": proposal["units"][0]["selectors"][0], "disposition": {"status": "processed"}},
        {"selector": proposal["units"][1]["selectors"][0], "disposition": {"status": "unprocessed", "reason": "预算不足"}}
    ]);
    proposal["units"].as_array_mut().unwrap().truncate(1);
    proposal["relations"] = json!([]);
    let validated = validate(&input, proposal).unwrap();
    assert_eq!(validated.status(), ValidatedProposalStatus::Partial);
}

#[test]
fn completeness_requires_a_matching_issue() {
    let (_, input, mut proposal) = valid_case();
    proposal["units"][1]["completeness"] = json!("left_truncated");
    let report = validate(&input, proposal.clone()).unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::InvalidCompleteness));

    proposal["issues"] = json!([{
        "kind": "left_truncated", "unit": "u-solution",
        "evidence": proposal["units"][1]["selectors"],
        "observation": "解答从选区左边界开始，前文可能缺失",
        "context_request": {"kind": "before"}
    }]);
    assert!(validate(&input, proposal).is_ok());
}

#[test]
fn enforces_collection_selector_and_serialized_size_limits() {
    let (_, input, proposal) = valid_case();
    let decoded: DiscoveryProposal = serde_json::from_value(proposal.clone()).unwrap();
    let report = validate_discovery_proposal(
        &input,
        decoded,
        DiscoveryValidationLimits::new(1, 100, 100_000).unwrap(),
    )
    .unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::LimitExceeded));

    let decoded: DiscoveryProposal = serde_json::from_value(proposal).unwrap();
    let report = validate_discovery_proposal(
        &input,
        decoded,
        DiscoveryValidationLimits::new(100, 100, 10).unwrap(),
    )
    .unwrap_err();
    assert!(has(&report, DiscoveryValidationCode::LimitExceeded));
    assert!(DiscoveryValidationLimits::new(0, 1, 1).is_none());
}
