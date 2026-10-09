use mathscape_domain::{InkLabDocument, InkLabOperation, InkLabProjection};
use serde_json::Value;

const TRACE: &str = include_str!("../../../testdata/ink/p0/trace.json");
const EXPECTED: &str = include_str!("../../../testdata/ink/p0/projection.json");
const INVALID: &str = include_str!("../../../testdata/ink/p0/invalid-cases.json");

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
