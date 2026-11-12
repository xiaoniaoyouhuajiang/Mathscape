use super::*;
use serde_json::json;

fn actor() -> ActorRef {
    ActorRef::parse("agent:codex").unwrap()
}

#[test]
fn trust_axis_is_a_fixed_mapping() {
    use AlignmentReviewStatus as S;
    for (status, axis) in [
        (S::Unmatched, AlignmentStatus::Unmatched),
        (S::RetrievedCandidate, AlignmentStatus::Candidate),
        (S::ContextChecked, AlignmentStatus::Candidate),
        (S::PartiallyAligned, AlignmentStatus::Candidate),
        (S::Ambiguous, AlignmentStatus::Candidate),
        (S::SemanticallyReviewed, AlignmentStatus::Reviewed),
        (S::Accepted, AlignmentStatus::Accepted),
        (S::Disputed, AlignmentStatus::Disputed),
        (S::Incompatible, AlignmentStatus::Disputed),
    ] {
        assert_eq!(status.trust_axis(), axis, "{status:?}");
    }
    assert!(!S::Accepted.agent_writable());
    assert!(!S::SemanticallyReviewed.agent_writable());
    assert!(!S::FormallyRelated.user_reviewable());
    assert!(S::Accepted.user_reviewable());
}

#[test]
fn same_as_is_not_a_relation() {
    assert!(serde_json::from_value::<AlignmentRelation>(json!("same_as")).is_err());
    assert_eq!(
        serde_json::from_value::<AlignmentRelation>(json!("overlaps_with")).unwrap(),
        AlignmentRelation::OverlapsWith
    );
}

#[test]
fn agent_writes_at_most_claimed_paid() {
    assert!(DebtStatus::ClaimedPaid.agent_writable());
    for status in [DebtStatus::Paid, DebtStatus::Waived, DebtStatus::Obsolete] {
        assert!(!status.agent_writable());
    }
}

#[test]
fn stored_payloads_round_trip_in_the_source_envelope() {
    let draft: SemanticEntityDraft = serde_json::from_value(json!({
        "semantic_kind": "declaration",
        "home_theory": "mathscape:semantic-entity/t",
        "preferred_labels": [{ "lang": "zh", "label": "Cassini 恒等式" }],
        "conventions": [{ "key": "sequence_index_origin", "subject": "F", "value": 0,
                          "text": "F₀ = 0", "basis": "mathlib" }],
        "mathlib_baseline": { "decl": "Int.fib_succ_mul_fib_pred_sub_fib_sq",
                              "module": "Mathlib.Data.Int.Fib.Lemmas",
                              "relation_hint": "specializes", "status": "unchecked" },
        "declaration": { "declaration_kind": "theorem",
                         "binders": [{ "name": "n", "type": "ℕ", "role": "universal" }],
                         "statement": { "text": "$F_{n+1}F_{n-1}-F_n^2=(-1)^n$" } }
    }))
    .unwrap();
    let payload = SemanticPayload::SemanticEntity(SemanticEntity {
        draft,
        trust: semantic_trust(AlignmentStatus::Unmatched, &actor()),
    });
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["payload_kind"], "semantic_entity");
    assert_eq!(value["payload"]["semantic_kind"], "declaration");
    assert_eq!(value["payload"]["trust"]["alignment"], "unmatched");
    let back: SemanticPayload = serde_json::from_value(value).unwrap();
    assert_eq!(back, payload);
    assert_eq!(back.entity_kind(), EntityKind::SemanticEntity);
    assert_eq!(
        DomainPayload::Semantic(back).payload_shape(),
        PayloadShape::Entity
    );
}

#[test]
fn drafts_reject_server_fields_and_unknown_keys() {
    let base = json!({
        "semantic_kind": "concept",
        "home_theory": "mathscape:semantic-entity/t",
        "preferred_labels": [{ "lang": "zh", "label": "Fibonacci 数列" }]
    });
    assert!(serde_json::from_value::<SemanticEntityDraft>(base.clone()).is_ok());
    let mut with_trust = base.clone();
    with_trust["trust"] = json!({});
    assert!(serde_json::from_value::<SemanticEntityDraft>(with_trust).is_err());
    let mut kind = base;
    kind["semantic_kind"] = json!("proof");
    assert!(serde_json::from_value::<SemanticEntityDraft>(kind).is_err());
}

#[test]
fn edit_changes_have_no_identity_fields() {
    assert!(
        serde_json::from_value::<AlignmentChanges>(json!({ "target": "mathscape:x/y" })).is_err()
    );
    assert!(
        serde_json::from_value::<SemanticEntityChanges>(json!({ "semantic_kind": "theory" }))
            .is_err()
    );
    assert!(serde_json::from_value::<DebtChanges>(json!({ "attached_to": "x" })).is_err());
    let changes: AlignmentChanges = serde_json::from_value(json!({
        "relation": "overlaps_with",
        "map_label": { "action": "clear" }
    }))
    .unwrap();
    assert_eq!(changes.relation, Some(AlignmentRelation::OverlapsWith));
    assert!(matches!(changes.map_label, crate::FieldChange::Clear));
}
