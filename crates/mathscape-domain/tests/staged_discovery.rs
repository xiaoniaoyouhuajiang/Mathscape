use mathscape_domain::*;
use serde_json::{Value, json};

#[test]
fn stage_outputs_roundtrip_and_do_not_accept_each_others_responsibilities() {
    let units: Value = serde_json::from_str(include_str!("fixtures/staged-units.json")).unwrap();
    let facts: Value = serde_json::from_str(include_str!("fixtures/staged-facts.json")).unwrap();
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<UnitDiscoveryProposal>(units.clone()).unwrap()
        )
        .unwrap(),
        units
    );
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SourceFactProposal>(facts.clone()).unwrap())
            .unwrap(),
        facts
    );
    for field in [
        "source_labels",
        "references",
        "relations",
        "input_fingerprint",
    ] {
        let mut invalid = units.clone();
        invalid[field] = json!([]);
        assert!(serde_json::from_value::<UnitDiscoveryProposal>(invalid).is_err());
    }
    for field in ["source", "owner_unit_ref", "units", "parent"] {
        let mut invalid = facts.clone();
        invalid["tasks"][0][field] = json!("u1");
        assert!(serde_json::from_value::<SourceFactProposal>(invalid).is_err());
    }
    let mut invalid = facts;
    invalid["tasks"][1]["references"][0]["source"] = json!("u1");
    assert!(serde_json::from_value::<SourceFactProposal>(invalid).is_err());
    assert!(serde_json::from_value::<DiscoveryProposal>(units).is_err());
    let legacy: DiscoveryProposal =
        serde_json::from_str(include_str!("fixtures/discovery-proposal.json")).unwrap();
    assert_eq!(legacy.version, DiscoveryProposalVersion::V3);
}

#[test]
fn occurrence_and_repair_are_closed_wire_contracts() {
    let anchor = json!({"kind":"text_occurrence","block":"b3","text":"定义 2","occurrence":2});
    let parsed: TextOccurrence = serde_json::from_value(anchor.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), anchor);
    let mut invalid = anchor;
    invalid["prefix"] = json!("由");
    assert!(serde_json::from_value::<TextOccurrence>(invalid).is_err());
    let repair = json!({"version":"mathscape-discovery-repair-1","repair_ref":"p1", "actions":[
        {"kind":"choose_candidate","issue_ref":"i1","candidate_ref":"c2"},
        {"kind":"defer","issue_ref":"i2","reason":"缺少原文"}
    ]});
    let parsed: DiscoveryRepairResponse = serde_json::from_value(repair.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), repair);

    let repair_input = json!({
        "version":"mathscape-discovery-repair-1", "repair_ref":"p1",
        "scope":{"stage":"facts","task_refs":["t2"]}, "blocks":[],
        "issues":[{"issue_ref":"i1","path":"tasks[1].references[0]",
            "code":"ambiguous_quote","message":"出现两次",
            "candidates":[{"candidate_ref":"c1","text":"定义 2","prefix":"由","suffix":"得出"}],
            "has_more":false}]
    });
    let parsed: DiscoveryRepairInput = serde_json::from_value(repair_input.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), repair_input);
}
