use mathscape_domain::{
    DiscoveryProposal, DiscoveryProposalVersion, DiscoveryQuoteSelector, DiscoverySelector,
    SourceUnitKind, SourceUnitRelationKind,
};
use serde_json::{Value, json};

fn example() -> Value {
    serde_json::from_str(include_str!("fixtures/discovery-proposal.json")).unwrap()
}

#[test]
fn proposal_roundtrips_without_gaining_canonical_authority() {
    let value = example();
    let proposal: DiscoveryProposal = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(proposal.version, DiscoveryProposalVersion::V3);
    assert_eq!(
        SourceUnitKind::from(proposal.units[0].unit_kind),
        SourceUnitKind::Problem
    );
    assert_eq!(
        SourceUnitRelationKind::from(proposal.relations[0].relation),
        SourceUnitRelationKind::Solves
    );
    assert_eq!(serde_json::to_value(&proposal).unwrap(), value);
}

#[test]
fn rejects_unknown_authority_fields_at_nested_boundaries() {
    for path in [
        "",
        "/units/0",
        "/relations/0",
        "/coverage/0",
        "/coverage/0/disposition",
    ] {
        let mut value = example();
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("trust".into(), json!({"curation_status": "accepted"}));
        assert!(
            serde_json::from_value::<DiscoveryProposal>(value).is_err(),
            "{path}"
        );
    }
    let mut value = example();
    value["units"][0]["dialect_scope"] = json!("made-up-scope");
    assert!(serde_json::from_value::<DiscoveryProposal>(value).is_err());
}

#[test]
fn refuses_missing_collections_versions_and_out_of_stage_types() {
    for field in ["units", "relations", "references", "issues", "coverage"] {
        let mut value = example();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<DiscoveryProposal>(value).is_err(),
            "{field}"
        );
    }
    for (path, replacement) in [
        ("/version", json!("mathscape-discovery-proposal-99")),
        ("/input_fingerprint", json!("invented-hash")),
        ("/units/0/unit_kind", json!("proof_step")),
        ("/relations/0/relation", json!("has_part")),
        ("/coverage/0/disposition", json!({"status": "unprocessed"})),
    ] {
        let mut value = example();
        *value.pointer_mut(path).unwrap() = replacement;
        assert!(
            serde_json::from_value::<DiscoveryProposal>(value).is_err(),
            "{path}"
        );
    }
    let mut legacy = example();
    legacy["version"] = json!("mathscape-discovery-proposal-2");
    assert!(serde_json::from_value::<DiscoveryProposal>(legacy).is_err());

    let mut missing_labels = example();
    missing_labels["units"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_labels");
    assert!(serde_json::from_value::<DiscoveryProposal>(missing_labels).is_err());
}

#[test]
fn producer_selectors_cannot_request_full_document_or_supply_offsets() {
    for value in [
        json!({"kind": "whole_document"}),
        json!({"kind": "block_range", "first": "b1", "last": "b2", "start": 0, "end": 9999}),
    ] {
        assert!(serde_json::from_value::<DiscoverySelector>(value).is_err());
    }
    let selector: DiscoverySelector = serde_json::from_value(json!({
        "kind": "quote_range", "first": "b1", "last": "b2",
        "start": {"quote": "任取", "occurrence": 2, "prefix": "先", "suffix": null},
        "end": {"quote": "得证", "occurrence": null, "prefix": null, "suffix": "。"}
    }))
    .unwrap();
    assert!(matches!(selector, DiscoverySelector::QuoteRange { .. }));

    assert!(
        serde_json::from_value::<DiscoveryQuoteSelector>(
            json!({"kind": "block_range", "first": "b1", "last": "b1"})
        )
        .is_err()
    );

    let mut proposal = example();
    proposal["references"] = json!([{
        "source": "u-solution",
        "selector": {"kind": "block_range", "first": "b-solution", "last": "b-solution"},
        "surface_form": "解答：", "target_label": null, "target": null
    }]);
    assert!(serde_json::from_value::<DiscoveryProposal>(proposal).is_err());
}

#[test]
fn preserves_unresolved_references_both_cut_edges_and_unprocessed_coverage() {
    let mut value = example();
    let selector = value["units"][1]["selectors"][0].clone();
    let reference_selector = json!({
        "kind": "quote_range", "first": "b-solution",
        "start": {"quote": "解答：", "occurrence": null, "prefix": null, "suffix": null},
        "last": "b-solution",
        "end": {"quote": "解答：", "occurrence": null, "prefix": null, "suffix": null}
    });
    value["references"] = json!([{
        "source": "u-solution", "selector": reference_selector, "surface_form": "解答：",
        "target_label": "1.1", "target": null
    }]);
    value["units"][1]["completeness"] = json!("uncertain");
    value["issues"] = json!([
        {"kind": "left_truncated", "unit": "u-solution", "evidence": [selector],
         "observation": "片段从中间开始", "context_request": {"kind": "before"}},
        {"kind": "right_truncated", "unit": "u-solution", "evidence": [selector],
         "observation": "片段尚未结束", "context_request": {"kind": "after"}}
    ]);
    value["coverage"][0]["disposition"] =
        json!({"status": "unprocessed", "reason": "输出预算不足"});
    let proposal: DiscoveryProposal = serde_json::from_value(value.clone()).unwrap();
    assert!(proposal.references[0].target.is_none());
    assert_eq!(proposal.issues.len(), 2);
    assert_eq!(serde_json::to_value(proposal).unwrap(), value);
    // An external command is not a source context request.
    value["issues"][0]["context_request"] = json!({"kind": "before", "path": "/tmp/other.md"});
    assert!(serde_json::from_value::<DiscoveryProposal>(value).is_err());
}
