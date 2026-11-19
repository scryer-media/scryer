use super::inspection::{ImportedFormat, Specification};
use super::rego::{Rule, print_module};
use super::specifications::compile_leaf;
use super::*;
use serde_json::{Value, json};

#[test]
fn diagnostic_codes_are_distinct() {
    let distinct = codes::ALL.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(distinct.len(), codes::ALL.len());
}

fn translate(source: ArrSourceApplication, json: &Value) -> ArrCustomFormatTranslation {
    translate_arr_custom_formats(source, &json.to_string(), &HashMap::new())
        .expect("paste is readable")
}

/// Reviews one format, then translates it with `score`.
fn translate_scored(specifications: Value, score: i32) -> ArrCustomFormatTranslation {
    let paste =
        json!({ "id": "format-id", "name": "Import test", "specifications": specifications })
            .to_string();
    let review =
        translate_arr_custom_formats(ArrSourceApplication::Radarr, &paste, &HashMap::new())
            .expect("paste is readable");
    assert_eq!(
        review.formats.len(),
        1,
        "{:?}",
        review.inspection_diagnostics
    );
    let scores = HashMap::from([(review.formats[0].id.clone(), score)]);
    translate_arr_custom_formats(ArrSourceApplication::Radarr, &paste, &scores)
        .expect("paste is readable")
}

fn title_spec(name: &str, value: &str, required: bool, negate: bool) -> Value {
    json!({
        "name": name,
        "implementation": "ReleaseTitleSpecification",
        "required": required,
        "negate": negate,
        "fields": { "value": value },
    })
}

/// The body of the first rule with this head, up to its closing brace.
fn rule_body<'a>(source: &'a str, head: &str) -> &'a str {
    let start = source
        .find(&format!("{head} if {{\n"))
        .expect("rule exists")
        + head.len()
        + 5;
    let end = source[start..].find("\n}").expect("rule closes");
    &source[start..start + end]
}

// ── Inspection ──────────────────────────────────────────────────────────────

#[test]
fn inspection_accepts_an_export_array_and_preserves_score_choices() {
    let paste = json!([{
        "trash_id": "one",
        "name": "Imported format",
        "trash_scores": { "default": 100, "anime": -10_000 },
        "specifications": [title_spec("Title", "test", true, false)],
    }]);
    let inspected = inspection::inspect(&paste.to_string(), ArrSourceApplication::Sonarr).unwrap();
    let format = &inspected.formats[0];
    assert_eq!(
        format.suggested_scores,
        [
            ("default".to_string(), 100.0),
            ("anime".to_string(), -10_000.0)
        ]
    );
    assert_eq!(
        format.specifications[0].fields,
        [("value".to_string(), json!("test"))]
    );
}

#[test]
fn inspection_refuses_unreadable_pastes_and_reports_invalid_formats() {
    let source = ArrSourceApplication::Radarr;
    assert_eq!(
        translate_arr_custom_formats(source, "{ nope", &HashMap::new()),
        Err(ArrImportRefusal::JsonInvalid)
    );
    assert_eq!(
        translate_arr_custom_formats(source, "[]", &HashMap::new()),
        Err(ArrImportRefusal::FormatsEmpty)
    );
    // A paste with no readable format is not a refusal: the review lists why.
    let result = translate(source, &json!({ "name": "missing specs" }));
    assert!(result.formats.is_empty());
    assert_eq!(
        result.inspection_diagnostics[0].code,
        codes::FORMAT_SPECIFICATIONS_MISSING
    );
    assert_eq!(
        result.inspection_diagnostics[0].severity,
        ArrDiagnosticSeverity::Error
    );
}

#[test]
fn inspection_accepts_the_field_array_export_shape() {
    let paste = json!({
        "id": 12,
        "name": "Array fields",
        "specifications": [{
            "name": "Title",
            "implementation": "ReleaseTitleSpecification",
            "fields": [{ "name": "value", "value": "word" }],
        }],
    });
    let inspected = inspection::inspect(&paste.to_string(), ArrSourceApplication::Radarr).unwrap();
    assert_eq!(inspected.formats[0].id, "$:12");
    assert_eq!(
        inspected.formats[0].specifications[0].field("value"),
        Some(&json!("word"))
    );
}

#[test]
fn invalid_optional_specifications_remain_errors_on_their_whole_format() {
    let result = translate(
        ArrSourceApplication::Radarr,
        &json!({
            "name": "Partial",
            "specifications": [
                { "implementation": "ReleaseTitleSpecification", "fields": { "value": "valid" } },
                {
                    "implementation": "ReleaseTitleSpecification",
                    "required": "false",
                    "fields": { "value": "invalid" },
                },
            ],
        }),
    );
    assert_eq!(result.formats.len(), 1);
    let format = &result.formats[0];
    assert_eq!(
        format.inspection_diagnostics[0].severity,
        ArrDiagnosticSeverity::Error
    );
    assert_eq!(
        result.inspection_diagnostics[0].format_id.as_deref(),
        Some(format.id.as_str())
    );
}

#[test]
fn duplicate_export_ids_remain_distinct_within_a_batch() {
    let format = json!({
        "id": 1,
        "name": "Same",
        "specifications": [{ "implementation": "SourceSpecification", "fields": { "value": 7 } }],
    });
    let result = translate(ArrSourceApplication::Radarr, &json!([format, format]));
    assert_ne!(result.formats[0].id, result.formats[1].id);
}

#[test]
fn conflicting_scores_warn_without_disabling_the_format() {
    let result = translate(
        ArrSourceApplication::Radarr,
        &json!({
            "name": "Scores",
            "score": 20,
            "trash_scores": { "default": 10 },
            "specifications": [{ "implementation": "SourceSpecification", "fields": { "value": 7 } }],
        }),
    );
    let format = &result.formats[0];
    let conflict = &result.inspection_diagnostics[0];
    assert_eq!(conflict.code, codes::SCORE_CONFLICT);
    assert_eq!(conflict.severity, ArrDiagnosticSeverity::Warning);
    assert_eq!(conflict.format_id.as_deref(), Some(format.id.as_str()));
    let mut scores: Vec<f64> = format
        .suggested_scores
        .iter()
        .map(|(_, score)| *score)
        .collect();
    scores.sort_by(f64::total_cmp);
    assert_eq!(scores, [10.0, 20.0]);
}

#[test]
fn inspection_bounds_pasted_utf8_bytes_and_batch_size() {
    let source = ArrSourceApplication::Sonarr;
    let none = HashMap::new();
    let pad = |format: String| {
        let padding = MAX_PASTED_JSON_BYTES - format.len();
        format + &" ".repeat(padding)
    };
    let format = json!({
        "name": "Format",
        "specifications": [{ "implementation": "SourceSpecification", "fields": { "value": 7 } }],
    })
    .to_string();
    // Multi-byte characters count as UTF-8 bytes, not characters.
    for at_limit in [pad(format.clone()), pad(format.replace("Format", "é"))] {
        assert!(translate_arr_custom_formats(source, &at_limit, &none).is_ok());
        assert_eq!(
            translate_arr_custom_formats(source, &(at_limit + " "), &none),
            Err(ArrImportRefusal::JsonTooLarge)
        );
    }
    let too_many = Value::Array(vec![json!({}); MAX_FORMATS + 1]).to_string();
    assert_eq!(
        translate_arr_custom_formats(source, &too_many, &none),
        Err(ArrImportRefusal::FormatsTooMany)
    );
}

#[test]
fn refusals_carry_their_reason_and_limit() {
    use crate::error_reasons::arr_custom_formats as reasons;
    let error = ArrImportRefusal::FormatsTooMany.into_app_error();
    assert_eq!(error.validation_reason(), Some(reasons::FORMATS_TOO_MANY));
    let AppError::ValidationRefused { facts, .. } = ArrImportRefusal::JsonTooLarge.into_app_error()
    else {
        panic!("refusal is a coded validation error");
    };
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].key, reasons::MAX_BYTES_FACT);
    assert_eq!(
        facts[0].value,
        crate::RefusalFactValue::Text(MAX_PASTED_JSON_BYTES.to_string())
    );
    for refusal in [
        ArrImportRefusal::JsonInvalid,
        ArrImportRefusal::FormatsEmpty,
    ] {
        assert!(reasons::ALL.contains(&refusal.reason()));
    }
}

// ── Translation ─────────────────────────────────────────────────────────────

#[test]
fn translation_honors_a_negated_required_specification() {
    let result = translate_scored(
        json!([
            title_spec("wanted", "wanted", true, false),
            title_spec("excluded", "excluded", true, true)
        ]),
        42,
    );
    assert_eq!(result.formats[0].status, ArrFormatStatus::Translated);
    let rego = &result.rego_source;
    assert!(rego.starts_with("import rego.v1\n"), "{rego}");
    assert!(
        rego.contains(r#"regex.match("(?i)wanted", regex.replace(input.release.raw_title"#),
        "{rego}"
    );
    assert!(rego.contains("not _arr_0_spec_1"), "{rego}");
    assert!(
        rego.contains(r#"score_entry["import_test_1"] := 42"#),
        "{rego}"
    );
}

#[test]
fn required_specifications_alone_gate_a_group() {
    let result = translate_scored(
        json!([
            title_spec("required", "required", true, false),
            title_spec("optional", "optional", false, false)
        ]),
        1,
    );
    let group = rule_body(&result.rego_source, "_arr_0_group_0");
    assert!(group.contains("_arr_0_spec_0"), "{group}");
    assert!(!group.contains("_arr_0_spec_1"), "{group}");
}

#[test]
fn all_optional_groups_are_a_disjunction() {
    let result = translate_scored(
        json!([
            title_spec("first", "first", false, false),
            title_spec("second", "second", false, false)
        ]),
        1,
    );
    let rego = &result.rego_source;
    assert!(
        rego.contains("_arr_0_group_0 if {\n  _arr_0_spec_0\n}"),
        "{rego}"
    );
    assert!(
        rego.contains("_arr_0_group_0 if {\n  _arr_0_spec_1\n}"),
        "{rego}"
    );
}

#[test]
fn one_incompatible_optional_condition_disables_the_whole_format() {
    let result = translate_scored(
        json!([
            title_spec("valid", "valid", true, false),
            title_spec("unsupported", r"(foo)\1", false, false)
        ]),
        7,
    );
    let format = &result.formats[0];
    assert_eq!(format.status, ArrFormatStatus::Disabled);
    assert_eq!(
        format.diagnostics[0].code,
        codes::REGEX_BACKREFERENCE_UNSUPPORTED
    );
    assert_eq!(result.diagnostics, format.diagnostics);
    assert!(
        result
            .rego_source
            .contains("score_entry[\"import_test_1\"] := 7 if {\n  false\n}"),
        "{}",
        result.rego_source
    );
}

#[test]
fn positional_lookarounds_use_source_position_relations() {
    let result = translate_scored(
        json!([title_spec("position", "foo(?!bar)", true, false)]),
        7,
    );
    assert_eq!(result.formats[0].status, ArrFormatStatus::Translated);
    let rego = &result.rego_source;
    assert!(rego.contains("numbers.range"), "{rego}");
    // Regorus never satisfies a negated wildcard lookup, so the negation
    // applies to a function call instead.
    assert!(
        rego.contains("not _arr_0_spec_0_position_2_anchored(_arr_0_spec_0_position_2_at)"),
        "{rego}"
    );
    assert!(!rego.contains("not _arr_0_spec_0_position_1[["), "{rego}");
    assert!(!rego.contains("foo(?!bar)"), "{rego}");
}

#[test]
fn source_facets_follow_the_pasted_application() {
    let spec = json!([{ "implementation": "SourceSpecification", "fields": { "value": 7 } }]);
    let movie = translate(
        ArrSourceApplication::Radarr,
        &json!({ "name": "M", "specifications": spec }),
    );
    assert_eq!(movie.applied_facets, ["movie"]);
    let series = translate(
        ArrSourceApplication::Sonarr,
        &json!({ "name": "S", "specifications": spec }),
    );
    assert_eq!(series.applied_facets, ["series", "anime"]);
}

// ── Specifications ──────────────────────────────────────────────────────────

struct Leaf {
    source: String,
    diagnostics: Vec<ArrDiagnostic>,
    alternatives: usize,
    handles_negation: bool,
}

fn compile(
    implementation: &str,
    fields: Value,
    source: ArrSourceApplication,
    negate: bool,
) -> Leaf {
    let fields = match fields {
        Value::Object(map) => map.into_iter().collect(),
        _ => unreachable!("fields are an object"),
    };
    let spec = Specification {
        implementation: implementation.to_string(),
        name: "condition".to_string(),
        negate,
        required: true,
        fields,
        index: 0,
    };
    let format = ImportedFormat {
        id: "format".to_string(),
        name: "Format".to_string(),
        source,
        specifications: vec![],
        suggested_scores: vec![],
        description: None,
        inspection_diagnostics: vec![],
    };
    let compiled = compile_leaf(&spec, &format).expect("not a regex specification");
    let rules: Vec<Rule> = std::iter::once(compiled.body.clone())
        .chain(compiled.alternatives.iter().cloned())
        .map(|body| Rule {
            name: "matches".to_string(),
            args: vec![],
            key: None,
            value: None,
            body,
        })
        .collect();
    Leaf {
        source: print_module(&rules),
        diagnostics: compiled.diagnostics,
        alternatives: compiled.alternatives.len(),
        handles_negation: compiled.handles_negation,
    }
}

fn radarr(implementation: &str, fields: Value) -> Leaf {
    compile(implementation, fields, ArrSourceApplication::Radarr, false)
}

fn sonarr(implementation: &str, fields: Value) -> Leaf {
    compile(implementation, fields, ArrSourceApplication::Sonarr, false)
}

fn first_severity(leaf: &Leaf) -> Option<ArrDiagnosticSeverity> {
    leaf.diagnostics
        .first()
        .map(|diagnostic| diagnostic.severity)
}

const ERROR: Option<ArrDiagnosticSeverity> = Some(ArrDiagnosticSeverity::Error);

#[test]
fn source_ids_are_app_specific_and_retain_remux_distinctions() {
    assert!(
        sonarr("SourceSpecification", json!({ "value": 3 }))
            .source
            .contains("WEB-DL")
    );
    assert!(
        radarr("SourceSpecification", json!({ "value": 3 }))
            .source
            .contains("TELECINE")
    );
    assert!(
        sonarr("SourceSpecification", json!({ "value": 6 }))
            .source
            .contains("is_remux == false")
    );
    assert!(
        sonarr("SourceSpecification", json!({ "value": 7 }))
            .source
            .contains("is_remux == true")
    );
    assert_eq!(
        first_severity(&sonarr("SourceSpecification", json!({ "value": 2 }))),
        ERROR
    );
}

#[test]
fn resolution_accepts_progressive_and_interlaced_tokens_at_the_same_height() {
    let leaf = radarr("ResolutionSpecification", json!({ "value": 1080 }));
    assert!(
        leaf.source.contains("1080p") && leaf.source.contains("1080i"),
        "{}",
        leaf.source
    );
    assert!(
        radarr("ResolutionSpecification", json!({ "value": 0 }))
            .source
            .contains("quality == null")
    );
    assert_eq!(
        first_severity(&radarr("ResolutionSpecification", json!({ "value": 4320 }))),
        ERROR
    );
}

#[test]
fn indexer_freeleech_uses_an_explicit_flag_or_the_volume_factor() {
    let leaf = radarr("IndexerFlagSpecification", json!({ "value": 1 }));
    assert_eq!(leaf.alternatives, 1);
    assert!(leaf.source.contains("freeleech"), "{}", leaf.source);
    assert!(
        leaf.source.contains("downloadvolumefactor"),
        "{}",
        leaf.source
    );
    for unrelated in ["password", "proper", "repack"] {
        assert!(!leaf.source.contains(unrelated), "{}", leaf.source);
    }
    assert!(
        sonarr("IndexerFlagSpecification", json!({ "value": 8 }))
            .source
            .contains("internal")
    );
    assert_eq!(
        first_severity(&radarr("IndexerFlagSpecification", json!({ "value": 8 }))),
        ERROR
    );
}

#[test]
fn language_exceptions_keep_the_null_guard_and_use_original_metadata() {
    let leaf = compile(
        "LanguageSpecification",
        json!({ "value": -2, "exceptLanguage": true }),
        ArrSourceApplication::Sonarr,
        true,
    );
    assert!(leaf.handles_negation);
    for expected in [
        "is_array(input.release.languages_audio)",
        "not scryer.lang_matches",
        "input.context.original_language",
        "== 0",
    ] {
        assert!(
            leaf.source.contains(expected),
            "{expected}: {}",
            leaf.source
        );
    }
    assert!(
        !leaf.source.contains("inferred_original"),
        "{}",
        leaf.source
    );
    for value in [19, 30, 37, 999] {
        assert_eq!(
            first_severity(&radarr("LanguageSpecification", json!({ "value": value }))),
            ERROR
        );
    }
}

#[test]
fn size_converts_gib_with_exclusive_lower_and_inclusive_upper_bounds() {
    let leaf = radarr("SizeSpecification", json!({ "min": 1, "max": 2 }));
    assert!(
        leaf.source.contains("size_bytes > 1073741824"),
        "{}",
        leaf.source
    );
    assert!(
        leaf.source.contains("size_bytes <= 2147483648"),
        "{}",
        leaf.source
    );
    // .NET rounds midpoint values to even, including half-byte boundaries.
    let gib = 1024f64.powi(3);
    let midpoints = radarr(
        "SizeSpecification",
        json!({ "min": 2.5 / gib, "max": 3.5 / gib }),
    );
    assert!(
        midpoints.source.contains("size_bytes > 2\n"),
        "{}",
        midpoints.source
    );
    assert!(
        midpoints.source.contains("size_bytes <= 4\n"),
        "{}",
        midpoints.source
    );
    assert_eq!(
        first_severity(&radarr(
            "SizeSpecification",
            json!({ "min": 0, "max": f64::MAX })
        )),
        ERROR
    );
}

#[test]
fn year_is_disabled_because_release_year_cannot_reproduce_the_metadata_fallback() {
    let leaf = radarr("YearSpecification", json!({ "min": 2000, "max": 2020 }));
    assert_eq!(leaf.diagnostics[0].code, codes::YEAR_UNMAPPED);
    assert_eq!(first_severity(&leaf), ERROR);
}

#[test]
fn unrecognized_fields_and_conditions_cannot_silently_weaken_formats() {
    let leaf = radarr(
        "SourceSpecification",
        json!({ "value": 7, "futureOption": true }),
    );
    assert_eq!(leaf.diagnostics[0].code, codes::FIELDS_UNSUPPORTED);
    let leaf = radarr("FutureSpecification", json!({ "value": 1 }));
    assert_eq!(leaf.diagnostics[0].code, codes::IMPLEMENTATION_UNSUPPORTED);
}

// ── Regex semantics through Regorus ─────────────────────────────────────────

/// Scores `title` with the translated rule for `pattern` and reports whether
/// the format matched.
struct TitleRule {
    engine: scryer_rules::UserRulesEngine,
    input: scryer_rules::UserRuleInput,
    rego_source: String,
}

impl TitleRule {
    fn new(pattern: &str) -> Self {
        let result = translate_scored(json!([title_spec("title", pattern, true, false)]), 1);
        assert_eq!(result.formats[0].diagnostics, [], "{pattern}");
        assert_eq!(
            result.formats[0].status,
            ArrFormatStatus::Translated,
            "{pattern}"
        );
        let id = "arr_regex_truth".to_string();
        let policy = scryer_rules::UserPolicy {
            rego_source: scryer_rules::rewrite_package_declaration(&result.rego_source, &id),
            id,
            name: "Arr regex truth".to_string(),
            origin: scryer_rules::PolicyOrigin::User,
            applied_facets: vec![],
        };
        let engine = scryer_rules::UserRulesEngine::build_with_baseline_rules(
            &[policy],
            &std::collections::HashSet::new(),
        )
        .unwrap_or_else(|error| panic!("{pattern}: {error}\n{}", result.rego_source));
        Self {
            engine,
            input: synthetic_input(),
            rego_source: result.rego_source,
        }
    }

    fn matches(&mut self, title: &str) -> bool {
        self.input.release.raw_title = title.to_string();
        let result = self
            .engine
            .evaluator()
            .evaluate(&self.input, "movie")
            .unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        !result.entries.is_empty()
    }
}

fn synthetic_input() -> scryer_rules::UserRuleInput {
    use crate::quality_profile::QualityProfileDecision;
    use crate::rules::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};
    let profile = crate::QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["1080P"],"allow_upgrades":true}}"#,
    )
    .unwrap();
    let decision = QualityProfileDecision {
        release_score: 0,
        scoring_log: vec![],
        allowed: true,
        block_codes: vec![],
        preference_score: 0,
        tier_index: None,
        size_fit_penalty: 0,
        revision_preference: 0,
        language: Default::default(),
        decision_pipeline: Default::default(),
        decided_by: None,
    };
    build_rule_input(
        &crate::parse_release_metadata("Synthetic.Title.2024.1080p.WEB-DL"),
        &profile,
        &decision,
        ReleaseRuntimeInfo {
            protocol: None,
            size_bytes: None,
            age_days: None,
            thumbs_up: None,
            thumbs_down: None,
            is_password_protected: None,
            extra: None,
            indexer_languages: None,
        },
        RuleContextInfo {
            title_id: None,
            library_name: None,
            category: Some("movie"),
            original_language: Some("en"),
            original_country: None,
            title_tags: &[],
            has_existing_file: false,
            existing_score: None,
            search_mode: "canonical",
            runtime_minutes: None,
            coverage_total_runtime_minutes: None,
            coverage_member_runtime_minutes: None,
            coverage_member_count: None,
            is_filler: false,
        },
        None,
    )
}

/// Expected results come from the JavaScript reference engine with the `iu`
/// flags, where `$` reads as .NET's non-multiline end: a final LF is allowed,
/// a CR before it is not.
#[test]
fn translated_regexes_match_reference_regex_behavior() {
    let cases: &[(&str, &[(&str, bool)])] = &[
        ("foo(?!bar)", &[("foobar", false), ("foo", true)]),
        (
            r"foo(?=bar\b)",
            &[
                ("foobar", true),
                ("foobarX", false),
                ("foobar!", true),
                ("xxfoobar!", true),
                ("foobarX foobar!", true),
            ],
        ),
        (
            "foo(?!bar|baz)",
            &[
                ("foobar", false),
                ("foobaz", false),
                ("fooqux", true),
                ("foobar fooqux", true),
                ("FOOqux", true),
            ],
        ),
        (
            "(?:^foo)(?=bar)",
            &[
                ("foobar", true),
                ("xfoobar", false),
                ("foobar\n", true),
                ("x\nfoobar", false),
            ],
        ),
        (
            "(?<=foo)bar",
            &[
                ("foobar", true),
                ("xxfoobar", true),
                ("barfoo", false),
                ("bar", false),
            ],
        ),
        (
            "(?<!foo|xx)bar",
            &[
                ("foobar", false),
                ("xxbar", false),
                ("xbar", true),
                ("bar", true),
                ("foobar bar", true),
            ],
        ),
        (
            "(?<!foo.*)bar",
            &[
                ("bar", true),
                ("fooXXbar", false),
                ("foo\nbar", true),
                ("xxbar", true),
            ],
        ),
        (
            r"\bfoo(?=x)",
            &[
                ("foox", true),
                ("afoox", false),
                ("!foox", true),
                ("😀foox", true),
            ],
        ),
        (
            r"\Bfoo(?=x)",
            &[
                ("foox", false),
                ("afoox", true),
                ("!foox", false),
                ("😀foox", false),
            ],
        ),
        (
            "foo(?=x$)",
            &[
                ("foox", true),
                ("foox\n", true),
                ("foox\r\n", false),
                ("fooxX", false),
            ],
        ),
        (
            "foo(?=|bar)",
            &[("foo", true), ("foobar", true), ("fooz", true)],
        ),
        (
            "foo(?!|bar)",
            &[("foo", false), ("foobar", false), ("fooz", false)],
        ),
        (
            "a+(?=ab)",
            &[("aab", true), ("aaab", true), ("ab", false), ("baab", true)],
        ),
        (
            "(?:a|aa)(?=ab)",
            &[("aab", true), ("aaab", true), ("ab", false), ("baab", true)],
        ),
        (
            "foo(?=b(?!az))",
            &[
                ("foobar", true),
                ("foobaz", false),
                ("foob", true),
                ("foox", false),
            ],
        ),
        (
            "(?:x(?!y)|z)w",
            &[
                ("xw", true),
                ("xyw", false),
                ("zw", true),
                ("xxw", true),
                ("w", false),
            ],
        ),
        (
            "(?<!a(?!b))c",
            &[
                ("ac", false),
                ("c", true),
                ("abc", true),
                ("bc", true),
                ("aac", false),
            ],
        ),
        (
            "(?:(?=foo)foo|bar)(?=x)",
            &[
                ("foox", true),
                ("barx", true),
                ("foobar", false),
                ("xbarx", true),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for (pattern, titles) in cases {
        let mut rule = TitleRule::new(pattern);
        for (title, expected) in *titles {
            if rule.matches(title) != *expected {
                mismatches.push(format!(
                    "{pattern} on {title:?}: expected {expected}\n{}",
                    rule.rego_source
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn unsupported_regex_modes_and_capture_state_disable_the_format() {
    for pattern in [
        "(?s)foo", "(?m)foo", "(?x)foo", "(?-i)foo", "(?>foo)", "foo++", r"(foo)\1", r"\Gfoo",
        "(?=foo)*", "foo\u{1}",
    ] {
        let result = translate_scored(json!([title_spec("title", pattern, false, false)]), 1);
        let format = &result.formats[0];
        assert_eq!(format.status, ArrFormatStatus::Disabled, "{pattern}");
        assert_eq!(
            format.diagnostics[0].severity,
            ArrDiagnosticSeverity::Error,
            "{pattern}"
        );
    }
}

#[test]
fn regex_nesting_is_bounded_before_any_pass_recurses_through_it() {
    let nested_groups = |depth: usize| format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
    let depth = super::onig::MAX_NESTING_DEPTH;

    let allowed = translate_scored(
        json!([title_spec("deep", &nested_groups(depth), true, false)]),
        1,
    );
    assert_eq!(
        allowed.formats[0].diagnostics,
        vec![],
        "the deepest allowed nesting translates"
    );

    for pattern in [
        nested_groups(depth + 1),
        nested_groups(4_000),
        "(".repeat(8_000),
        format!("{}{}", "[a".repeat(2_000), "]".repeat(2_000)),
        format!("a{}", "{1}".repeat(2_000)),
    ] {
        let refused = translate_scored(json!([title_spec("deep", &pattern, true, false)]), 1);
        let format = &refused.formats[0];
        assert_eq!(format.status, ArrFormatStatus::Disabled);
        assert_eq!(format.diagnostics[0].code, codes::REGEX_PARSE_FAILED);
        assert!(
            format.diagnostics[0].message.contains("nests deeper"),
            "{}",
            format.diagnostics[0].message
        );
    }
}

// ── One format per module ───────────────────────────────────────────────────

const SINGLE_HEAD: &str = r#"import rego.v1

_arr_0_spec_0 if {
  regex.match("(?i)\\bsample\\b", regex.replace(input.release.raw_title, "\\s*(?:[<>?*|])", ""))
}
_arr_0_group_0 if {
  _arr_0_spec_0
}
_arr_0_matches if {
  _arr_0_group_0
}
"#;

fn sample_format() -> Value {
    json!({
        "id": 7,
        "name": "Sample Format Alpha",
        "specifications": [title_spec("Sample", "\\bsample\\b", true, false)],
    })
}

fn sample_table() -> Vec<(String, i32)> {
    vec![
        ("Sample Profile A".to_string(), 25),
        ("Sample Profile B".to_string(), -10),
    ]
}

fn translate_one(raw: &Value, scoring: &ArrFormatScoring) -> ArrSingleFormatTranslation {
    translate_arr_custom_format(
        ArrSourceApplication::Radarr,
        raw,
        scoring,
        &["Imported from Radarr 6.0.4. Saved disabled; enable it once reviewed.".to_string()],
        &["REVIEW: check it.".to_string()],
    )
}

/// Builds an engine over one module, failing on any validator finding.
fn single_engine(rego_source: &str) -> scryer_rules::UserRulesEngine {
    let id = "arr_single_format".to_string();
    let rewritten = scryer_rules::rewrite_package_declaration(rego_source, &id);
    let validation = scryer_rules::validation::validate_user_rule(&rewritten, &id).unwrap();
    assert!(validation.valid, "{validation:?}\n{rego_source}");
    let policy = scryer_rules::UserPolicy {
        rego_source: rewritten,
        id,
        name: "Arr single format".to_string(),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec![],
    };
    scryer_rules::UserRulesEngine::build_with_baseline_rules(
        &[policy],
        &std::collections::HashSet::new(),
    )
    .unwrap_or_else(|error| panic!("{error}\n{rego_source}"))
}

/// The score entries one module yields for a matching release under `profile`.
fn scores_under(engine: &scryer_rules::UserRulesEngine, profile: &str) -> Vec<i32> {
    let mut input = synthetic_input();
    input.release.raw_title = "Synthetic.Sample.2024.1080p.WEB-DL".to_string();
    input.profile.name = profile.to_string();
    let result = engine.evaluator().evaluate(&input, "movie").unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.entries.iter().map(|entry| entry.delta).collect()
}

#[test]
fn a_literal_score_keeps_the_paste_form() {
    let result = translate_one(&sample_format(), &ArrFormatScoring::Literal(25));
    assert_eq!(result.status, Some(ArrFormatStatus::Translated));
    assert_eq!(result.name.as_deref(), Some("Sample Format Alpha"));
    let expected = format!(
        "{SINGLE_HEAD}score_entry[\"sample_format_alpha_1\"] := 25 if {{\n  _arr_0_matches\n}}\n\n\
         # Imported from Radarr 6.0.4. Saved disabled; enable it once reviewed.\n"
    );
    assert_eq!(result.rego_source.as_deref(), Some(expected.as_str()));
    let engine = single_engine(&expected);
    assert_eq!(scores_under(&engine, "Sample Profile A"), [25]);
    assert_eq!(scores_under(&engine, "Any Other Profile"), [25]);
}

#[test]
fn a_profile_table_scores_by_profile_name_and_skips_unlisted_profiles() {
    let result = translate_one(
        &sample_format(),
        &ArrFormatScoring::ProfileTable(sample_table()),
    );
    let expected = format!(
        "{SINGLE_HEAD}_arr_0_scores := {{\n  \"Sample Profile A\": 25,\n  \"Sample Profile B\": -10\n}}\n\
         score_entry[\"sample_format_alpha_1\"] := _arr_0_scores[input.profile.name] if {{\n  _arr_0_matches\n}}\n\n\
         # Imported from Radarr 6.0.4. Saved disabled; enable it once reviewed.\n"
    );
    assert_eq!(result.rego_source.as_deref(), Some(expected.as_str()));
    let engine = single_engine(&expected);
    assert_eq!(scores_under(&engine, "Sample Profile A"), [25]);
    assert_eq!(scores_under(&engine, "Sample Profile B"), [-10]);
    // Not a 0 entry: an unlisted profile yields no entry at all.
    assert_eq!(scores_under(&engine, "Sample Profile C"), Vec::<i32>::new());
    assert_eq!(scores_under(&engine, "sample profile a"), Vec::<i32>::new());
}

#[test]
fn a_review_table_scores_zero_and_keeps_the_table_commented_out() {
    let result = translate_one(
        &sample_format(),
        &ArrFormatScoring::ReviewTable(sample_table()),
    );
    let expected = format!(
        "{SINGLE_HEAD}score_entry[\"sample_format_alpha_1\"] := 0 if {{\n  _arr_0_matches\n}}\n\n\
         # Imported from Radarr 6.0.4. Saved disabled; enable it once reviewed.\n\n\
         # REVIEW: check it.\n#\n\
         # _arr_0_scores := {{\n#   \"Sample Profile A\": 25,\n#   \"Sample Profile B\": -10\n# }}\n\
         # score_entry[\"sample_format_alpha_1\"] := _arr_0_scores[input.profile.name] if {{\n\
         #   _arr_0_matches\n# }}\n"
    );
    assert_eq!(result.rego_source.as_deref(), Some(expected.as_str()));
    let engine = single_engine(&expected);
    assert_eq!(scores_under(&engine, "Sample Profile A"), [0]);
    assert_eq!(scores_under(&engine, "Sample Profile C"), [0]);
}

#[test]
fn the_single_format_body_matches_the_paste_translation() {
    let raw = sample_format();
    let single = translate_one(&raw, &ArrFormatScoring::Literal(25))
        .rego_source
        .unwrap();
    let pasted = translate_scored(raw["specifications"].clone(), 25).rego_source;
    let rule = |source: &str| {
        source
            .lines()
            .filter(|line| !line.starts_with('#') && !line.contains("score_entry"))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(rule(&single), rule(&pasted));
}

#[test]
fn hostile_names_stay_inert_in_strings_and_comments() {
    let hostile = "Evil\"\n} allow := true # \\ \r\t\u{7}\u{2028}end";
    let raw = json!({
        "id": 9,
        "name": hostile,
        "specifications": [title_spec(hostile, "\\bsample\\b", true, false)],
    });
    let table = vec![
        (hostile.to_string(), 5),
        ("Sample Profile A".to_string(), 1),
    ];
    let notes = [format!("Imported from Radarr {hostile}.")];
    for scoring in [
        ArrFormatScoring::Literal(3),
        ArrFormatScoring::ProfileTable(table.clone()),
        ArrFormatScoring::ReviewTable(table.clone()),
    ] {
        let result = translate_arr_custom_format(
            ArrSourceApplication::Radarr,
            &raw,
            &scoring,
            &notes,
            &notes,
        );
        let source = result.rego_source.expect("translates");
        // The name reaches Rego only inside strings and whole-line comments.
        for line in source.lines() {
            let line = line.trim_start();
            assert!(
                line.starts_with('#') || !line.starts_with("} allow"),
                "{line}\n{source}"
            );
            if !line.starts_with('#') && line.contains("allow :=") {
                assert!(line.contains(r#""Evil\"\n} allow"#), "{line}\n{source}");
            }
        }
        let engine = single_engine(&source);
        match scoring {
            ArrFormatScoring::Literal(_) => assert_eq!(scores_under(&engine, hostile), [3]),
            ArrFormatScoring::ProfileTable(_) => {
                assert_eq!(scores_under(&engine, hostile), [5]);
                assert_eq!(scores_under(&engine, "Sample Profile A"), [1]);
            }
            ArrFormatScoring::ReviewTable(_) => assert_eq!(scores_under(&engine, hostile), [0]),
        }
    }
}

#[test]
fn unreadable_and_disabled_formats_produce_no_module() {
    let unreadable = translate_one(&json!("not an object"), &ArrFormatScoring::Literal(1));
    assert_eq!(unreadable.status, None);
    assert_eq!(unreadable.rego_source, None);
    assert_eq!(unreadable.diagnostics[0].code, codes::FORMAT_INVALID);

    let disabled = translate_one(
        &json!({
            "id": 3,
            "name": "Sample Format Beta",
            "specifications": [{
                "name": "Year",
                "implementation": "YearSpecification",
                "required": true,
                "negate": false,
                "fields": { "min": 2000, "max": 2010 },
            }],
        }),
        &ArrFormatScoring::Literal(1),
    );
    assert_eq!(disabled.status, Some(ArrFormatStatus::Disabled));
    assert_eq!(disabled.rego_source, None);
    assert!(!disabled.diagnostics.is_empty());
}
