use mathscape_domain::*;
use mathscape_orchestrator::{
    map_edit::plan_map_change, registration::*, source_integrity::source_references,
};
use serde_json::json;
use std::collections::BTreeMap;
fn revision(n: char) -> String {
    format!("sha256:{}", n.to_string().repeat(64))
}
fn qualify(id: &str) -> String {
    format!("mathscape:test/{id}")
}
fn normalize(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s)
            if ["span", "parent", "child", "outside"].contains(&s.as_str()) =>
        {
            *s = qualify(s)
        }
        serde_json::Value::Array(a) => {
            for v in a {
                normalize(v)
            }
        }
        serde_json::Value::Object(o) => {
            for v in o.values_mut() {
                normalize(v)
            }
        }
        _ => {}
    }
}
fn object(id: &str, kind: &str, mut p: serde_json::Value) -> MapEditObject {
    normalize(&mut p);
    MapEditObject {
        object_id: qualify(id),
        revision: revision('1'),
        kind: kind.into(),
        lifecycle: "active".into(),
        payload: json!({"payload_kind":if kind=="SourceSpan"{"span"}else{"semantic_unit"},"payload":p}),
        endpoints: vec![],
    }
}
fn adoption(decision: &str, ids: &[(&str, &str)]) -> RegistrationAdoption {
    RegistrationAdoption {
        summary: RegistrationSummary {
            decision_id: qualify(decision),
            registration_commit_id: revision('2'),
            run_id: qualify("run"),
            state: RegistrationState::Registered,
            registered_at: "unix:1".into(),
            selected_unit_refs: vec![],
            object_count: ids.len() as u32,
            reversal_receipt: None,
        },
        members: ids
            .iter()
            .map(|(id, outcome)| RegistrationMember {
                object_id: qualify(id),
                revision_id: revision('1'),
                outcome: (*outcome).into(),
            })
            .collect(),
    }
}
fn setup() -> RegistrationSnapshot {
    RegistrationSnapshot {
        map: MapEditSnapshot {
            base_commit: Some(revision('2')),
            adopted_by: BTreeMap::new(),
            objects: vec![
                object(
                    "span",
                    "SourceSpan",
                    json!({"markdown":{"artifact":"mathscape:test/md","byte_range":{"start":0,"end":3}}}),
                ),
                object(
                    "parent",
                    "SourceSemanticUnit",
                    json!({"source_spans":["span"],"parent_unit":null}),
                ),
                object(
                    "child",
                    "SourceSemanticUnit",
                    json!({"source_spans":["span"],"parent_unit":"parent"}),
                ),
            ],
        },
        registrations: vec![adoption(
            "decision",
            &[
                ("span", "created"),
                ("parent", "created"),
                ("child", "created"),
            ],
        )],
    }
}
fn request() -> ReverseDiscoveryRequest {
    ReverseDiscoveryRequest {
        request_id: qualify("request"),
        decision_id: qualify("decision"),
        registration_commit_id: revision('2'),
        base_commit: Some(revision('2')),
        reason: "整理".into(),
    }
}
fn actor() -> ActorRef {
    ActorRef::parse("test").unwrap()
}
#[test]
fn internal_parent_and_evidence_do_not_block_batch_retirement() {
    let p = plan_registration_reversal(&setup(), &request(), &actor());
    assert_eq!(p.plan.impact.status, ImpactPlanStatus::Ready);
    assert_eq!(p.plan.summary.retire, 3);
    assert_eq!(p.patch.unwrap().operations().len(), 3);
}
#[test]
fn another_registration_adoption_without_graph_edges_protects_dependency_closure() {
    let mut s = setup();
    s.registrations
        .push(adoption("other", &[("child", "reused")]));
    let p = plan_registration_reversal(&s, &request(), &actor());
    assert_eq!(p.plan.impact.status, ImpactPlanStatus::Ready);
    assert_eq!(p.plan.summary.retire, 0);
    assert_eq!(p.plan.summary.preserve, 3);
    assert!(p.patch.is_none());
}
#[test]
fn outside_parent_reference_protects_parent_and_span_but_not_unrelated_child() {
    let mut s = setup();
    s.map.objects.push(object(
        "outside",
        "SourceSemanticUnit",
        json!({"parent_unit":"parent","source_spans":["span"]}),
    ));
    let p = plan_registration_reversal(&s, &request(), &actor());
    assert_eq!(p.plan.impact.status, ImpactPlanStatus::Ready);
    assert_eq!(p.plan.summary.retire, 1);
    assert_eq!(
        p.plan
            .impact
            .direct_changes
            .iter()
            .find(|i| i.object_id == qualify("child"))
            .unwrap()
            .action,
        "retire"
    );
}
#[test]
fn subsequent_modification_protects_necessary_evidence() {
    let mut s = setup();
    s.map
        .objects
        .iter_mut()
        .find(|o| o.object_id == qualify("parent"))
        .unwrap()
        .revision = revision('3');
    let p = plan_registration_reversal(&s, &request(), &actor());
    assert_eq!(p.plan.summary.retire, 1);
    assert_eq!(p.plan.summary.preserve, 2);
}
#[test]
fn inactive_allows_no_patch_and_preserves_current_lifecycle() {
    let mut s = setup();
    for o in &mut s.map.objects {
        o.lifecycle = "rejected".into();
    }
    let p = plan_registration_reversal(&s, &request(), &actor());
    assert_eq!(p.plan.impact.status, ImpactPlanStatus::Ready);
    assert_eq!(p.plan.summary.already_inactive, 3);
    assert!(p.patch.is_none());
}
#[test]
fn collects_all_located_facts_references_and_trust_evidence() {
    let o = object(
        "u",
        "SourceSemanticUnit",
        json!({"source_spans":["s1"],"source_title":{"source_span":"s2"},"source_labels":[{"source_span":"s3"}],"local_references":[{"source_span":"s4","candidate_targets":["v"]}],"trust":{"evidence":["s5"]}}),
    );
    let refs = source_references(&o).unwrap();
    for id in ["s1", "s2", "s3", "s4", "s5", "v"] {
        assert!(refs.iter().any(|(target, _)| target == id));
    }
}
#[test]
fn unordered_snapshot_permutations_have_identical_plan_fingerprint() {
    let mut a = setup();
    let p = plan_registration_reversal(&a, &request(), &actor());
    a.map.objects.reverse();
    a.registrations[0].members.reverse();
    assert_eq!(
        p.plan.impact.plan_fingerprint,
        plan_registration_reversal(&a, &request(), &actor())
            .plan
            .impact
            .plan_fingerprint
    );
}
#[test]
fn manual_span_retirement_blocks_on_live_unit_instead_of_cascading_units() {
    let s = setup();
    let r = UserChangeRequest {
        request_id: qualify("request"),
        base_commit: s.map.base_commit.clone(),
        reason: "证据清理".into(),
        operations: vec![UserChangeOperation::RetireObject {
            object_id: qualify("span"),
            expected_revision: revision('1'),
        }],
    };
    let p = plan_map_change(&s.map, &r, &actor());
    assert_eq!(p.impact.status, ImpactPlanStatus::Blocked);
    assert!(p.patch.is_none());
    assert!(
        p.impact
            .diagnostics
            .iter()
            .any(|d| d.code == "inactive_or_invalid_reference")
    );
}
