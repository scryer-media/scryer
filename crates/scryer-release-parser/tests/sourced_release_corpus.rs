//! Assertions over anonymized names collected independently of the parser.
//!
//! Run with `SCRYER_PARSER_CORPUS_REPORT_DIR=/tmp/parser-corpus-report` to retain
//! every mismatch as JSON, including when the assertion output is abbreviated.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use scryer_release_parser::{ReleaseParseContext, analyze_release_for_target};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    category: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    source: String,
    source_fingerprint: String,
    shape: String,
    release: String,
    context: ReleaseParseContext,
    expected: Value,
    evidence: BTreeMap<String, String>,
    #[serde(default)]
    accepted_failure: Option<AcceptedFailure>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AcceptedFailure {
    reason: String,
    tradeoff: String,
    mismatches: Vec<AcceptedMismatch>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AcceptedMismatch {
    field: String,
    actual: Value,
    #[serde(default)]
    missing_field: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum CaseStatus {
    Passed,
    AcceptedFailed,
    UnexpectedFailed,
    UnexpectedPass,
    StaleSignature,
}

fn mismatch_signature(mismatches: &[Value]) -> Vec<AcceptedMismatch> {
    mismatches
        .iter()
        .map(|mismatch| AcceptedMismatch {
            field: mismatch["field"]
                .as_str()
                .expect("mismatch field")
                .to_owned(),
            actual: mismatch["actual"].clone(),
            missing_field: mismatch["missing_field"].as_bool().unwrap_or(false),
        })
        .collect()
}

fn classify_case(accepted: Option<&AcceptedFailure>, mismatches: &[Value]) -> CaseStatus {
    match (accepted, mismatches.is_empty()) {
        (None, true) => CaseStatus::Passed,
        (None, false) => CaseStatus::UnexpectedFailed,
        (Some(_), true) => CaseStatus::UnexpectedPass,
        (Some(accepted), false) if accepted.mismatches == mismatch_signature(mismatches) => {
            CaseStatus::AcceptedFailed
        }
        (Some(_), false) => CaseStatus::StaleSignature,
    }
}

fn compare_fields(expected: &Value, actual: &Value, path: &str, mismatches: &mut Vec<Value>) {
    if let Value::Object(fields) = expected {
        for (field, value) in fields {
            if field == "$any" {
                let alternatives = value
                    .as_array()
                    .expect("$any is an array of field assertions");
                assert!(
                    !alternatives.is_empty(),
                    "$any needs at least one alternative"
                );
                if !alternatives.iter().any(|alternative| {
                    let mut failures = Vec::new();
                    compare_fields(alternative, actual, path, &mut failures);
                    failures.is_empty()
                }) {
                    mismatches.push(json!({"field": format!("{path}.$any"), "expected": value, "actual": actual}));
                }
                continue;
            }
            let next_path = if path.is_empty() {
                field.clone()
            } else {
                format!("{path}.{field}")
            };
            match actual.get(field) {
                Some(actual) => compare_fields(value, actual, &next_path, mismatches),
                None => mismatches.push(json!({
                    "field": next_path,
                    "expected": value,
                    "actual": null,
                    "missing_field": true,
                })),
            }
        }
    } else if expected != actual {
        mismatches.push(json!({ "field": path, "expected": expected, "actual": actual }));
    }
}

fn assertion_count(value: &Value) -> usize {
    match value {
        Value::Object(fields) => fields.values().map(assertion_count).sum(),
        _ => 1,
    }
}

fn run_corpus(category: &str, source: &str) {
    let corpus: Corpus = serde_json::from_str(source).expect("valid sourced corpus JSON");
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.category, category);
    assert_eq!(
        corpus.cases.len(),
        500,
        "each facet must contain 500 releases"
    );

    let mut ids = HashSet::new();
    let mut fingerprints = HashSet::new();
    let mut shapes = BTreeMap::<&str, usize>::new();
    let mut accepted_failed = Vec::new();
    let mut unexpected_failed = Vec::new();
    let mut unexpected_pass = Vec::new();
    let mut stale_signature = Vec::new();
    let mut failures = Vec::new();
    let mut checked_fields = 0;
    let mut failed_fields = 0;
    for case in &corpus.cases {
        assert!(ids.insert(&case.id), "duplicate fixture id: {}", case.id);
        assert!(
            fingerprints.insert(&case.source_fingerprint),
            "duplicate source: {}",
            case.id
        );
        assert!(matches!(case.source.as_str(), "srrdb.com" | "nyaa.si"));
        assert_eq!(
            case.source_fingerprint.len(),
            64,
            "{}: SHA-256 fingerprint",
            case.id
        );
        assert!(
            case.source_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert!(
            !case.context.title.name.is_empty(),
            "{}: target title",
            case.id
        );
        assert!(
            !case.evidence.is_empty(),
            "{}: independently annotated evidence",
            case.id
        );
        if let Some(accepted) = &case.accepted_failure {
            assert!(
                !accepted.reason.is_empty(),
                "{}: accepted failure reason",
                case.id
            );
            assert!(
                !accepted.tradeoff.is_empty(),
                "{}: accepted failure tradeoff",
                case.id
            );
            assert!(
                !accepted.mismatches.is_empty(),
                "{}: accepted failure needs a mismatch signature",
                case.id
            );
        }
        let field_count = assertion_count(&case.expected);
        // Sparse names can declare only an episode range and an unnamed season.
        assert!(field_count >= 2, "{}: too few assertions", case.id);
        checked_fields += field_count;
        *shapes.entry(&case.shape).or_default() += 1;

        let analysis = analyze_release_for_target(&case.release, &case.context);
        let actual = analysis
            .best_candidate()
            .map(|candidate| {
                serde_json::to_value(&candidate.projected).expect("serializable parse")
            })
            .unwrap_or(Value::Null);
        let mut mismatches = Vec::new();
        compare_fields(&case.expected, &actual, "", &mut mismatches);
        if !mismatches.is_empty() {
            failed_fields += mismatches.len();
        }
        let status = classify_case(case.accepted_failure.as_ref(), &mismatches);
        let detail = json!({"id": case.id, "shape": case.shape, "release": case.release,
            "mismatches": mismatches, "accepted_failure": case.accepted_failure});
        if status != CaseStatus::Passed && status != CaseStatus::UnexpectedPass {
            failures.push(detail.clone());
        }
        match status {
            CaseStatus::Passed => {}
            CaseStatus::AcceptedFailed => accepted_failed.push(detail),
            CaseStatus::UnexpectedFailed => unexpected_failed.push(detail),
            CaseStatus::UnexpectedPass => unexpected_pass.push(detail),
            CaseStatus::StaleSignature => stale_signature.push(detail),
        }
    }

    let failed_cases = accepted_failed.len() + unexpected_failed.len() + stale_signature.len();
    let passed_cases = corpus.cases.len() - failed_cases;

    let report = json!({
        "category": category,
        "cases": corpus.cases.len(),
        "passed_cases": passed_cases,
        "failed_cases": failed_cases,
        "accepted_failed_cases": accepted_failed.len(),
        "unexpected_failed_cases": unexpected_failed.len(),
        "unexpected_pass_cases": unexpected_pass.len(),
        "stale_signature_cases": stale_signature.len(),
        "checked_fields": checked_fields,
        "failed_fields": failed_fields,
        "shapes": shapes,
        "failures": failures,
        "accepted_failed": accepted_failed,
        "unexpected_failed": unexpected_failed,
        "unexpected_pass": unexpected_pass,
        "stale_signature": stale_signature,
    });
    if let Some(directory) = std::env::var_os("SCRYER_PARSER_CORPUS_REPORT_DIR") {
        let directory = Path::new(&directory);
        std::fs::create_dir_all(directory).expect("create corpus report directory");
        std::fs::write(
            directory.join(format!("{category}.json")),
            serde_json::to_vec_pretty(&report).expect("serializable corpus report"),
        )
        .expect("write corpus report");
    }
    let gate_failures = unexpected_failed
        .iter()
        .chain(unexpected_pass.iter())
        .chain(stale_signature.iter());
    let examples = gate_failures
        .clone()
        .take(20)
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        gate_failures.count() == 0,
        "{category}: {passed_cases}/{} passed; {} accepted failures, {} unexpected failures, {} unexpected passes, and {} stale signatures; {failed_fields}/{checked_fields} fields failed.\n\
         First 20 gate failures:\n{examples}\n\
         Set SCRYER_PARSER_CORPUS_REPORT_DIR to retain the complete report.",
        corpus.cases.len(),
        accepted_failed.len(),
        unexpected_failed.len(),
        unexpected_pass.len(),
        stale_signature.len(),
    );
}

fn accepted_failure(mismatches: Vec<AcceptedMismatch>) -> AcceptedFailure {
    AcceptedFailure {
        reason: "Reviewed parser limitation".to_owned(),
        tradeoff: "Retained as an explicit accuracy failure".to_owned(),
        mismatches,
    }
}
fn mismatch(field: &str, actual: Value, missing_field: bool) -> Value {
    json!({"field": field, "actual": actual, "missing_field": missing_field})
}

#[test]
fn accepted_failure_requires_an_exact_signature() {
    let expected = accepted_failure(vec![AcceptedMismatch {
        field: "episode.source".to_owned(),
        actual: Value::Null,
        missing_field: true,
    }]);
    assert_eq!(
        classify_case(
            Some(&expected),
            &[mismatch("episode.source", Value::Null, true)]
        ),
        CaseStatus::AcceptedFailed
    );
    assert_eq!(
        classify_case(
            Some(&expected),
            &[
                mismatch("episode.source", Value::Null, true),
                mismatch("source", json!("WEB-DL"), false)
            ]
        ),
        CaseStatus::StaleSignature
    );
    assert_eq!(
        classify_case(
            Some(&expected),
            &[mismatch("episode.source", json!("BluRay"), true)]
        ),
        CaseStatus::StaleSignature
    );
    assert_eq!(
        classify_case(
            Some(&expected),
            &[mismatch("episode.source", Value::Null, false)]
        ),
        CaseStatus::StaleSignature
    );
    assert_eq!(
        classify_case(Some(&expected), &[]),
        CaseStatus::UnexpectedPass
    );
}

fn expected_paths(value: &Value, prefix: &str, paths: &mut HashSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                expected_paths(child, &path, paths);
            }
        }
        _ => {
            paths.insert(prefix.to_owned());
        }
    }
}

#[test]
fn reviewed_anime_annotations_match_the_committed_oracle() {
    let corpus: Value =
        serde_json::from_str(include_str!("corpus/sourced/anime.json")).expect("anime corpus JSON");
    let audit: Value = serde_json::from_str(include_str!("corpus/sourced/anime_annotations.json"))
        .expect("anime audit JSON");
    let cases = corpus["cases"].as_array().expect("anime cases");
    let annotations = audit["cases"].as_array().expect("anime annotations");
    assert_eq!(cases.len(), 500);
    assert_eq!(annotations.len(), 500);
    for (index, (case, annotation)) in cases.iter().zip(annotations).enumerate() {
        assert_eq!(case["id"], format!("anime-{:04}", index + 1));
        assert_eq!(annotation["id"], case["id"]);
        assert_eq!(annotation["reviewed"], Value::Bool(true));
        assert_eq!(annotation["source_fingerprint"], case["source_fingerprint"]);
        let mut expected = case["expected"].clone();
        expected
            .as_object_mut()
            .expect("expected object")
            .remove("release_group");
        assert_eq!(annotation["expected"], expected, "{}", case["id"]);
        assert!(
            case["context"]["known_years"]
                .as_array()
                .expect("known years")
                .is_empty()
        );
        assert!(
            case["context"]["episodes"]
                .as_array()
                .expect("episodes")
                .is_empty()
        );
        let mut paths = HashSet::new();
        expected_paths(&case["expected"], "", &mut paths);
        let evidence = case["evidence"].as_object().expect("evidence");
        assert_eq!(paths.len(), evidence.len(), "{}", case["id"]);
        assert!(
            paths.iter().all(|path| evidence.contains_key(path)),
            "{}",
            case["id"]
        );
    }
}

#[test]
fn sourced_fixture_identities_match_the_frozen_baseline() {
    let baseline: Value =
        serde_json::from_str(include_str!("corpus/sourced/baseline.json")).expect("baseline JSON");
    for (category, source) in [
        ("movie", include_str!("corpus/sourced/movie.json")),
        ("series", include_str!("corpus/sourced/series.json")),
        ("anime", include_str!("corpus/sourced/anime.json")),
    ] {
        let corpus: Value = serde_json::from_str(source).expect("corpus JSON");
        let current = corpus["cases"].as_array().expect("corpus cases");
        let frozen = baseline["identities"]
            .as_array()
            .expect("baseline identities")
            .iter()
            .find(|entry| entry["category"] == category)
            .expect("frozen category")["cases"]
            .as_array()
            .expect("frozen cases");
        assert_eq!(current.len(), 500, "{category}: current cases");
        assert_eq!(frozen.len(), 500, "{category}: frozen cases");
        for (case, frozen_case) in current.iter().zip(frozen) {
            assert_eq!(case["id"], frozen_case["id"], "{category}: case id");
            assert_eq!(
                case["source_fingerprint"], frozen_case["source_fingerprint"],
                "{category}: {} fingerprint",
                case["id"]
            );
        }
    }
}

#[test]
fn sourced_movie_release_names() {
    run_corpus("movie", include_str!("corpus/sourced/movie.json"));
}

#[test]
fn sourced_series_release_names() {
    run_corpus("series", include_str!("corpus/sourced/series.json"));
}

#[test]
fn sourced_anime_release_names() {
    run_corpus("anime", include_str!("corpus/sourced/anime.json"));
}
