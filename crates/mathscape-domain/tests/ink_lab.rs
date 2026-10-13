use mathscape_domain::{InkLabDocument, InkLabOperation, InkLabProjection, InkSnapshotManifest};
use serde_json::Value;

const TRACE: &str = include_str!("../../../testdata/ink/p0/trace.json");
const EXPECTED: &str = include_str!("../../../testdata/ink/p0/projection.json");
const INVALID: &str = include_str!("../../../testdata/ink/p0/invalid-cases.json");

#[test]
fn immutable_snapshot_and_page_selection_match_the_shared_fixture() {
    let document: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    let manifest = InkSnapshotManifest::from_document(&document, 2).unwrap();
    let fixture: InkSnapshotManifest =
        serde_json::from_str(include_str!("../../../testdata/ink/p2/snapshot.json")).unwrap();
    assert_eq!(manifest, fixture);
    fixture
        .validate_selection("page-a", &["stroke-a".into()])
        .unwrap();
    assert!(
        fixture
            .validate_selection("page-a", &["stroke-b".into()])
            .is_err()
    );
    let mut bad = fixture;
    bad.visible_stroke_ids.clear();
    assert!(bad.validate().is_err());
}

#[test]
fn legacy_fragment_summary_is_compatible_but_new_snapshots_include_fragments() {
    let old: InkSnapshotManifest = serde_json::from_str(include_str!(
        "../../../testdata/ink/p2/legacy-fragments.json"
    ))
    .unwrap();
    old.validate_selection("page-a", &["fragment-a".into()])
        .unwrap();
    let new = InkSnapshotManifest::from_document(&old.document, 2).unwrap();
    assert_eq!(new.version, "mathscape-ink-snapshot-2");
    assert_eq!(new.visible_stroke_ids, vec!["fragment-a"]);
    let mut invalid = new.clone();
    invalid.visible_stroke_ids.clear();
    assert!(invalid.validate().is_err());
    let mut bad_document = new.document;
    if let InkLabOperation::PartialErase { fragments, .. } = &mut bad_document.operations[1] {
        fragments[0].points[0].pressure = Some(2.0);
    }
    assert!(bad_document.validate().is_err());
}

#[test]
fn interchange_replays_edits_and_preserves_cancelled_and_interrupted_samples() {
    let document: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    assert_eq!(
        document.validate().unwrap(),
        serde_json::from_str::<InkLabProjection>(EXPECTED).unwrap()
    );
    let roundtrip: InkLabDocument =
        serde_json::from_str(&serde_json::to_string(&document).unwrap()).unwrap();
    assert_eq!(roundtrip, document);
    assert_eq!(roundtrip.validate().unwrap().point_count, 7);
    assert!(
        matches!(&document.operations[1], InkLabOperation::Stroke { stroke, .. } if stroke.points[0].pressure.is_none())
    );
}

#[test]
fn rust_rejects_the_shared_invalid_interchange_cases() {
    let cases: Vec<Value> = serde_json::from_str(INVALID).unwrap();
    for case in cases {
        let mut value: Value = serde_json::from_str(TRACE).unwrap();
        for change in case["changes"].as_array().unwrap() {
            let path = change["path"].as_str().unwrap();
            if let Some(slot) = value.pointer_mut(path) {
                *slot = change["value"].clone();
            } else {
                let (parent, key) = path.rsplit_once('/').unwrap();
                value
                    .pointer_mut(parent)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(key.to_owned(), change["value"].clone());
            }
        }
        for path in case["removes"].as_array().unwrap() {
            let (parent, key) = path.as_str().unwrap().rsplit_once('/').unwrap();
            value
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
        let accepted = serde_json::from_value::<InkLabDocument>(value)
            .is_ok_and(|document| document.validate().is_ok());
        assert!(!accepted, "accepted invalid case: {}", case["name"]);
    }
}

#[test]
fn schema_is_generated_from_the_rust_contract() {
    let current = serde_json::to_value(schemars::schema_for!(InkLabDocument)).unwrap();
    let saved: Value =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/schema.json")).unwrap();
    assert_eq!(current, saved);
    let point = &current["$defs"]["InkLabPoint"];
    for field in ["pressure", "tilt_x_deg", "tilt_y_deg"] {
        assert!(
            point["required"]
                .as_array()
                .unwrap()
                .contains(&Value::String(field.to_owned()))
        );
        assert!(
            point["properties"][field]["type"]
                .as_array()
                .unwrap()
                .contains(&Value::String("null".to_owned()))
        );
    }
}
