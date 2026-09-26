//! Release-language tags checked against annotated, anonymized release shapes.

use scryer_release_parser::{ReleaseParseContext, best_parse_for_target};
use serde_json::Value;

#[test]
fn ukrainian_language_tag_shapes() {
    let corpus: Value =
        serde_json::from_str(include_str!("corpus/languages/ukrainian.json")).expect("corpus JSON");
    assert_eq!(corpus["schema_version"], 1);
    let mut failures = Vec::new();
    for case in corpus["cases"].as_array().expect("cases") {
        let expected = case["expected"].as_object().expect("expected object");
        let evidence = case["evidence"].as_object().expect("evidence object");
        assert!(
            expected.keys().all(|field| evidence.contains_key(field)),
            "{}: every expected field needs evidence",
            case["id"]
        );
        let target: ReleaseParseContext =
            serde_json::from_value(case["context"].clone()).expect("context");
        let parsed = best_parse_for_target(case["release"].as_str().expect("release"), &target);
        let actual = serde_json::to_value(&parsed).expect("serializable parse");
        for (field, value) in expected {
            if actual.get(field) != Some(value) {
                failures.push(format!(
                    "{} {field}: expected {value}, actual {:?}",
                    case["id"],
                    actual.get(field)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
