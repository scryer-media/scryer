//! Maintenance rules that declare their actions as Rego heads: what the engine
//! decodes, what the static scan reads off the source, and the `days_since`
//! helper the host puts in every rule's package.

use crate::diagnostic_codes;
use crate::maintenance::{
    MaintenanceEvalResult, MaintenanceInput, MaintenanceOutcome, MaintenancePolicy,
    MaintenanceRulesEngine, Observation, rewrite_package_declaration, synthetic_maintenance_input,
};
use crate::maintenance_actions::{
    MaintenanceActionHead, MaintenanceActionLiteral, MaintenanceActionScan, MaintenanceActions,
    scan_maintenance_action_heads,
};
use crate::validation::{ValidationResult, validate_maintenance_rule};
use chrono::{DateTime, Utc};

fn policy(id: &str, body: &str) -> MaintenancePolicy {
    MaintenancePolicy {
        id: id.to_string(),
        name: format!("rule {id}"),
        rego_source: rewrite_package_declaration(body, id),
    }
}

fn evaluate_against(body: &str, input: MaintenanceInput) -> MaintenanceEvalResult {
    let engine =
        MaintenanceRulesEngine::build(&[policy("fixture_rule", body)]).expect("rule compiles");
    engine
        .evaluator()
        .evaluate(&input)
        .expect("evaluation should succeed")
}

fn evaluate(body: &str) -> MaintenanceEvalResult {
    evaluate_against(body, synthetic_maintenance_input())
}

/// The actions of the one record a well-formed rule produces.
fn actions_of(result: &MaintenanceEvalResult) -> &MaintenanceActions {
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.records.len(), 1);
    &result.records[0].decision.actions
}

fn validate(body: &str) -> ValidationResult {
    let source = rewrite_package_declaration(body, "fixture_rule");
    validate_maintenance_rule(&source, "fixture_rule").expect("validation should not fail outright")
}

/// A complete source with nothing rewritten, so line numbers in a test are the
/// line numbers in the string.
fn source(body: &str) -> String {
    format!("package scryer.maintenance.user.fixture_rule\nimport rego.v1\n{body}")
}

fn literal(value: &str, line: usize, column: usize) -> MaintenanceActionLiteral {
    MaintenanceActionLiteral {
        value: value.to_string(),
        line,
        column,
    }
}

/// The synthetic run happens at 2023-11-14T22:13:20Z.
fn input_added_at(added_at: &str) -> MaintenanceInput {
    let mut input = synthetic_maintenance_input();
    input.facts.added_at = Observation::known(added_at.to_string());
    input
}

// ── Decoding ────────────────────────────────────────────────────────────────

#[test]
fn every_action_head_decodes_to_its_declared_type() {
    let result = evaluate(
        "unmonitor if input.facts.monitored\n\
         delete_files if input.facts.has_file\n\
         delete_title if input.facts.has_file\n\
         search if input.facts.monitored\n\
         set_quality_profile := \"Fixture HD\" if input.facts.has_file\n\
         add_tags := [\"stale\", \"fixture tag\"] if input.facts.has_file\n\
         remove_tags := [\"new\"] if input.facts.has_file\n",
    );

    let actions = actions_of(&result);
    assert_eq!(
        actions,
        &MaintenanceActions {
            unmonitor: true,
            delete_files: true,
            delete_title: true,
            search: true,
            set_quality_profile: Some("Fixture HD".to_string()),
            add_tags: vec!["stale".to_string(), "fixture tag".to_string()],
            remove_tags: vec!["new".to_string()],
        }
    );
    assert!(!actions.is_empty());
    assert!(actions.defines_destructive_action());
    assert_eq!(actions.heads(), MaintenanceActionHead::ALL.to_vec());
}

#[test]
fn each_boolean_head_is_decoded_on_its_own() {
    for head in [
        MaintenanceActionHead::Unmonitor,
        MaintenanceActionHead::DeleteFiles,
        MaintenanceActionHead::DeleteTitle,
        MaintenanceActionHead::Search,
    ] {
        let result = evaluate(&format!("{} if input.facts.has_file\n", head.rule_name()));
        let actions = actions_of(&result);
        assert_eq!(actions.heads(), vec![head], "{head:?}");
        assert_eq!(
            actions.defines_destructive_action(),
            head.is_destructive(),
            "{head:?}"
        );
    }
}

#[test]
fn undefined_heads_decode_to_no_actions() {
    let result = evaluate("match := true\n");

    let actions = actions_of(&result);
    assert_eq!(actions, &MaintenanceActions::default());
    assert!(actions.is_empty());
    assert!(!actions.defines_destructive_action());
    assert!(actions.heads().is_empty());
}

/// A head whose body does not hold for this subject is off the package
/// document exactly like one that was never written.
#[test]
fn a_head_whose_condition_fails_decodes_to_its_default() {
    let result = evaluate(
        "delete_title if not input.facts.has_file\n\
         set_quality_profile := \"Fixture HD\" if not input.facts.has_file\n\
         add_tags := [\"stale\"] if not input.facts.has_file\n",
    );

    assert!(actions_of(&result).is_empty());
}

/// Action heads leave the outcome alone: it still follows `match` and
/// `unknown` only.
#[test]
fn action_heads_do_not_change_the_outcome() {
    let without_match = evaluate("unmonitor if input.facts.monitored\n");
    assert!(actions_of(&without_match).unmonitor);
    assert_eq!(
        without_match.records[0].decision.outcome,
        MaintenanceOutcome::NoMatch
    );

    let with_match = evaluate("match := true\nunmonitor if input.facts.monitored\n");
    assert!(actions_of(&with_match).unmonitor);
    assert_eq!(
        with_match.records[0].decision.outcome,
        MaintenanceOutcome::Match
    );
}

#[test]
fn a_head_of_the_wrong_type_is_an_error_not_a_coerced_action() {
    for (body, head) in [
        ("unmonitor := \"yes\"\n", "unmonitor"),
        ("delete_files := [true]\n", "delete_files"),
        ("delete_title := 1\n", "delete_title"),
        ("search := null\n", "search"),
        ("set_quality_profile := 7\n", "set_quality_profile"),
        ("set_quality_profile := true\n", "set_quality_profile"),
        (
            "set_quality_profile := [\"Fixture HD\"]\n",
            "set_quality_profile",
        ),
        ("set_quality_profile := \"\"\n", "set_quality_profile"),
        ("add_tags := [\"stale\", 1]\n", "add_tags"),
        ("add_tags := \"stale\"\n", "add_tags"),
        ("add_tags := {\"stale\"}\n", "add_tags"),
        ("add_tags contains \"stale\"\n", "add_tags"),
        ("remove_tags := [null]\n", "remove_tags"),
        ("remove_tags := true\n", "remove_tags"),
    ] {
        let result = evaluate(&format!("match := true\n{body}"));

        assert!(result.records.is_empty(), "{body}: {:?}", result.records);
        assert_eq!(result.errors.len(), 1, "{body}");
        assert!(
            result.errors[0].message.contains(&format!("'{head}'")),
            "{body}: {}",
            result.errors[0].message
        );
    }
}

#[test]
fn tags_keep_the_bounds_every_policy_tag_has() {
    let result = evaluate("add_tags := [\"scryer:managed\"]\n");

    assert!(result.records.is_empty(), "{:?}", result.records);
    assert!(
        result.errors[0].message.contains("reserved"),
        "{}",
        result.errors[0].message
    );
}

#[test]
fn a_rule_that_declares_unknown_gets_no_actions() {
    let result = evaluate(
        "unknown := true\n\
         delete_title := true\n\
         set_quality_profile := \"Fixture HD\"\n\
         add_tags := [\"stale\"]\n",
    );

    assert_eq!(
        result.records[0].decision.outcome,
        MaintenanceOutcome::Unknown
    );
    assert!(actions_of(&result).is_empty());
}

#[test]
fn a_rule_the_host_holds_gets_no_actions() {
    let mut input = synthetic_maintenance_input();
    input.facts.active_downloads = Observation::unknown("not_yet_collected");

    let result = evaluate_against("delete_title if not input.facts.active_downloads\n", input);

    assert_eq!(
        result.records[0].decision.outcome,
        MaintenanceOutcome::Unknown
    );
    assert_eq!(
        result.records[0].decision.reason_codes,
        vec!["not_yet_collected".to_string()]
    );
    assert!(actions_of(&result).is_empty());
}

/// Holding a subject does not excuse a malformed head: the rule is in error
/// either way.
#[test]
fn a_wrong_typed_head_is_an_error_even_when_the_rule_declares_unknown() {
    let result = evaluate("unknown := true\ndelete_title := 1\n");

    assert!(result.records.is_empty(), "{:?}", result.records);
    assert_eq!(result.errors.len(), 1);
}

const ARCHIVE_RULE: &str = "archive if {\n    \
         input.facts.watched_by_anyone\n    \
         days_since(input.facts.last_watched_by_anyone_at) > 90\n\
     }\n\
     add_tags := [\"archived\"] if archive\n\
     unmonitor if archive\n";

#[test]
fn a_helper_rule_is_reused_by_several_action_heads() {
    let mut stale = synthetic_maintenance_input();
    stale.facts.last_watched_by_anyone_at = Observation::known("2023-01-01T00:00:00Z".to_string());
    let result = evaluate_against(ARCHIVE_RULE, stale);
    assert_eq!(
        actions_of(&result),
        &MaintenanceActions {
            unmonitor: true,
            add_tags: vec!["archived".to_string()],
            ..MaintenanceActions::default()
        }
    );

    let mut recent = synthetic_maintenance_input();
    recent.facts.last_watched_by_anyone_at = Observation::known("2023-11-01T00:00:00Z".to_string());
    assert!(actions_of(&evaluate_against(ARCHIVE_RULE, recent)).is_empty());
}

#[test]
fn an_else_chain_on_a_value_head_yields_the_first_branch_that_holds() {
    const BODY: &str = "set_quality_profile := \"Fixture UHD\" if {\n    \
             input.facts.file_count > 5\n\
         } else := \"Fixture HD\" if {\n    \
             input.facts.has_file\n\
         }\n";

    assert_eq!(
        actions_of(&evaluate(BODY)).set_quality_profile.as_deref(),
        Some("Fixture HD")
    );

    let mut many_files = synthetic_maintenance_input();
    many_files.facts.file_count = Observation::known(9);
    assert_eq!(
        actions_of(&evaluate_against(BODY, many_files))
            .set_quality_profile
            .as_deref(),
        Some("Fixture UHD")
    );

    let mut no_file = synthetic_maintenance_input();
    no_file.facts.has_file = Observation::known(false);
    assert_eq!(
        actions_of(&evaluate_against(BODY, no_file)).set_quality_profile,
        None
    );
}

// ── days_since ──────────────────────────────────────────────────────────────

/// Tags the subject with the number of days the helper returned, so a test can
/// read the exact value back.
const DAYS_PROBE: &str = "add_tags := [sprintf(\"days %d\", [days_since(input.facts.added_at)])]\n";

fn probed_days(input: MaintenanceInput) -> Vec<String> {
    actions_of(&evaluate_against(DAYS_PROBE, input))
        .add_tags
        .clone()
}

#[test]
fn days_since_counts_whole_days_up_to_the_evaluation_time() {
    assert_eq!(
        probed_days(input_added_at("2023-11-04T22:13:20Z")),
        vec!["days 10"],
        "exactly ten days"
    );
    assert_eq!(
        probed_days(input_added_at("2023-11-04T22:13:21Z")),
        vec!["days 9"],
        "one second short of ten days is nine whole days"
    );
    assert_eq!(
        probed_days(input_added_at("2023-11-14T22:13:20Z")),
        vec!["days 0"]
    );
    assert_eq!(
        probed_days(input_added_at("2023-11-05T00:13:20+02:00")),
        vec!["days 10"],
        "the offset is part of the timestamp"
    );
}

/// The clock is the run's. Moving it moves the answer, and nothing else does.
#[test]
fn days_since_reads_the_clock_from_the_input() {
    let mut input = input_added_at("2023-11-04T22:13:20Z");
    input.evaluation_time = DateTime::<Utc>::from_timestamp(1_700_000_000 + 5 * 86_400, 0)
        .expect("fixed timestamp is in range");

    assert_eq!(probed_days(input), vec!["days 15"]);

    let engine = MaintenanceRulesEngine::build(&[policy("fixture_rule", DAYS_PROBE)])
        .expect("rule compiles");
    assert!(
        !engine.reads_clock(),
        "the helper must not make a rule look like it reads the wall clock"
    );
}

#[test]
fn days_since_of_a_missing_fact_is_undefined_so_the_condition_is_false() {
    let mut input = synthetic_maintenance_input();
    input.facts.added_at = Observation::absent();

    let result = evaluate_against(
        "unmonitor if days_since(input.facts.added_at) > 1\n\
         search if days_since(input.facts.added_at) <= 1\n",
        input,
    );

    assert!(actions_of(&result).is_empty());
}

/// The fact a rule passes to the helper is still a fact the rule reads, so the
/// host still holds the rule when it could not observe it.
#[test]
fn a_rule_passing_an_unobservable_fact_to_days_since_is_held() {
    let engine = MaintenanceRulesEngine::build(&[policy(
        "fixture_rule",
        "unmonitor if days_since(input.facts.added_at) > 1\n",
    )])
    .expect("rule compiles");
    assert_eq!(
        engine.rules()[0]
            .referenced_facts
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["added_at"],
        "the host module's own reads are not the rule's"
    );

    let mut input = synthetic_maintenance_input();
    input.facts.added_at = Observation::unknown("source_unavailable");
    let result = engine.evaluator().evaluate(&input).expect("evaluates");

    assert_eq!(
        result.records[0].decision.outcome,
        MaintenanceOutcome::Unknown
    );
    assert_eq!(
        result.records[0].decision.reason_codes,
        vec!["source_unavailable".to_string()]
    );
    assert!(actions_of(&result).is_empty());
}

/// A timestamp that is not one is a failure of the rule, never a silent zero.
#[test]
fn days_since_of_a_malformed_timestamp_is_an_error_for_the_rule() {
    let result = evaluate_against(
        "unmonitor if days_since(input.facts.added_at) > 1\n",
        input_added_at("last tuesday"),
    );

    assert!(result.records.is_empty(), "{:?}", result.records);
    assert_eq!(result.errors.len(), 1);
}

#[test]
fn an_author_definition_of_days_since_replaces_the_host_one() {
    // A function of the same shape.
    let function = "days_since(timestamp) := 7 if timestamp != \"\"\n\
                    unmonitor if days_since(input.facts.added_at) == 7\n";
    assert!(actions_of(&evaluate(function)).unmonitor);

    // A function of another arity, which could not coexist with the host's.
    let other_arity = "days_since(from, to) := 3 if from != to\n\
                       unmonitor if days_since(input.facts.added_at, \"fixture\") == 3\n";
    assert!(actions_of(&evaluate(other_arity)).unmonitor);

    // A plain value under the same name.
    let value = "days_since := 5\nunmonitor if days_since == 5\n";
    assert!(actions_of(&evaluate(value)).unmonitor);

    // Validation refuses every redefinition, pointing at the definition.
    for body in [function, other_arity, value] {
        let validation = validate(&format!("match := true\n{body}"));
        assert!(!validation.valid, "{body}");
        let error = validation
            .diagnostics
            .iter()
            .find(|error| error.code == diagnostic_codes::RULE_HOST_HELPER_REDEFINED)
            .unwrap_or_else(|| panic!("{body}: {:?}", validation.diagnostics));
        assert_eq!(error.line, Some(4), "{body}");
        assert_eq!(error.column, Some(1), "{body}");
    }
}

#[test]
fn days_since_works_in_the_validation_dry_run() {
    let validation = validate("match if days_since(input.facts.added_at) > 30\n");

    assert!(validation.valid, "{:?}", validation.errors);
    assert!(validation.warnings.is_empty(), "{:?}", validation.warnings);
}

// ── Static head scan ────────────────────────────────────────────────────────

#[test]
fn the_scan_reports_the_action_heads_a_source_defines() {
    let scan = scan_maintenance_action_heads(
        &source(
            "archive if input.facts.watched_by_anyone\n\
             unmonitor if archive\n\
             delete_files if archive\n\
             add_tags := [\"archived\"] if archive\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    assert_eq!(
        scan.defined_heads.iter().copied().collect::<Vec<_>>(),
        vec![
            MaintenanceActionHead::Unmonitor,
            MaintenanceActionHead::DeleteFiles,
            MaintenanceActionHead::AddTags,
        ]
    );
    assert!(
        scan.defined_heads
            .contains(&MaintenanceActionHead::DeleteFiles)
    );
    assert!(
        !scan
            .defined_heads
            .contains(&MaintenanceActionHead::DeleteTitle)
    );
    assert!(scan.defines_destructive_action());

    let harmless =
        scan_maintenance_action_heads(&source("match := true\nsearch := true\n"), "fixture_rule")
            .expect("source parses");
    assert!(!harmless.defines_destructive_action());
}

#[test]
fn the_scan_collects_every_literal_value_with_its_position() {
    let scan = scan_maintenance_action_heads(
        &source(
            "set_quality_profile := \"Fixture UHD\" if {\n\
             \tinput.facts.file_count > 5\n\
             } else := \"Fixture HD\" if {\n\
             \tinput.facts.has_file\n\
             }\n\
             set_quality_profile := `Fixture SD` if input.facts.file_count == 0\n\
             add_tags := [\"stale\", \"fixture tag\"] if input.facts.has_file\n\
             add_tags := [\"empty\"] if not input.facts.has_file\n\
             remove_tags := []\n\
             default remove_tags := [\"new\"]\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    assert_eq!(
        scan.defined_heads.iter().copied().collect::<Vec<_>>(),
        vec![
            MaintenanceActionHead::SetQualityProfile,
            MaintenanceActionHead::AddTags,
            MaintenanceActionHead::RemoveTags,
        ]
    );
    assert_eq!(
        scan.quality_profiles,
        vec![
            literal("Fixture UHD", 3, 24),
            literal("Fixture HD", 5, 11),
            literal("Fixture SD", 8, 24),
        ]
    );
    assert_eq!(
        scan.add_tags,
        vec![
            literal("stale", 9, 14),
            literal("fixture tag", 9, 23),
            literal("empty", 10, 14),
        ]
    );
    assert_eq!(scan.remove_tags, vec![literal("new", 12, 25)]);
}

#[test]
fn the_scan_result_serializes_to_json() {
    let scan: MaintenanceActionScan = scan_maintenance_action_heads(
        &source("delete_title if input.facts.has_file\nadd_tags := [\"stale\"]\n"),
        "fixture_rule",
    )
    .expect("source parses");

    let json = serde_json::to_value(&scan).expect("serializes");
    assert_eq!(
        json["defined_heads"],
        serde_json::json!(["delete_title", "add_tags"])
    );
    assert_eq!(json["add_tags"][0]["value"], "stale");
    assert_eq!(json["add_tags"][0]["line"], 4);
    assert_eq!(json["add_tags"][0]["column"], 14);
    assert_eq!(json["errors"], serde_json::json!([]));
    assert_eq!(json["warnings"], serde_json::json!([]));
}

#[test]
fn a_value_head_that_is_not_a_plain_literal_is_refused() {
    for (body, line, column) in [
        // A profile read from the subject.
        (
            "set_quality_profile := input.facts.quality_profile_id\n",
            3,
            24,
        ),
        (
            "set_quality_profile := concat(\"-\", [\"Fixture\", \"HD\"])\n",
            3,
            24,
        ),
        ("set_quality_profile := [\"Fixture HD\"]\n", 3, 24),
        ("set_quality_profile := null\n", 3, 24),
        // No value at all, which would yield `true`.
        ("set_quality_profile if input.facts.has_file\n", 3, 1),
        ("add_tags if input.facts.has_file\n", 3, 1),
        // A literal first branch does not excuse a computed second one.
        (
            "set_quality_profile := \"Fixture HD\" if input.facts.has_file else := input.subject.name\n",
            3,
            69,
        ),
        // Tags built per subject.
        ("add_tags := input.facts.tags\n", 3, 13),
        ("add_tags := [input.subject.facet]\n", 3, 13),
        ("add_tags := [\"stale\", 1]\n", 3, 13),
        ("add_tags := \"stale\"\n", 3, 13),
        ("add_tags := {\"stale\"}\n", 3, 13),
        (
            "remove_tags := [tag | some tag in input.facts.tags]\n",
            3,
            16,
        ),
        // A value assembled out of pieces.
        ("add_tags contains \"stale\"\n", 3, 1),
        ("remove_tags[\"stale\"] := true\n", 3, 1),
        ("add_tags(extra) := [\"stale\"] if extra\n", 3, 1),
    ] {
        let scan =
            scan_maintenance_action_heads(&source(body), "fixture_rule").expect("source parses");

        assert_eq!(scan.errors.len(), 1, "{body}: {:?}", scan.errors);
        let error = &scan.errors[0];
        assert_eq!(
            error.code,
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            "{body}"
        );
        assert_eq!((error.line, error.column), (line, column), "{body}");
        assert!(
            scan.quality_profiles.len() + scan.add_tags.len() <= 1,
            "{body}: only a literal branch contributes a literal"
        );
    }
}

#[test]
fn a_literal_the_action_does_not_accept_is_refused() {
    for (body, line, column) in [
        ("set_quality_profile := \"\"\n", 3, 24),
        ("add_tags := [\"stale\", \"scryer:managed\"]\n", 3, 23),
        ("remove_tags := [\"kids/teens\"]\n", 3, 17),
        ("add_tags := [\"\"]\n", 3, 14),
    ] {
        let scan =
            scan_maintenance_action_heads(&source(body), "fixture_rule").expect("source parses");

        assert_eq!(scan.errors.len(), 1, "{body}: {:?}", scan.errors);
        assert_eq!(
            scan.errors[0].code,
            diagnostic_codes::RULE_ACTION_VALUE_INVALID,
            "{body}"
        );
        assert_eq!(
            (scan.errors[0].line, scan.errors[0].column),
            (line, column),
            "{body}"
        );
    }
}

#[test]
fn a_rule_name_nothing_uses_is_a_warning() {
    let scan = scan_maintenance_action_heads(
        &source(
            "match := true\n\
             delet_title if input.facts.has_file\n\
             delet_title if input.facts.monitored\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.defined_heads.is_empty());
    assert_eq!(scan.warnings.len(), 1, "one warning per name");
    assert_eq!(
        scan.warnings[0].code,
        diagnostic_codes::RULE_UNREFERENCED_RULE
    );
    assert_eq!((scan.warnings[0].line, scan.warnings[0].column), (4, 1));
}

#[test]
fn helpers_that_are_used_and_contract_heads_do_not_warn() {
    let scan = scan_maintenance_action_heads(
        &source(
            "day_ns := 86400000000000\n\
             old(timestamp) if time.parse_rfc3339_ns(timestamp) < 10 * day_ns\n\
             limits := {\"files\": 5}\n\
             labels[\"stale\"] := true\n\
             archive if {\n\
             \told(input.facts.added_at)\n\
             \tinput.facts.file_count > limits.files\n\
             \tlabels[\"stale\"]\n\
             }\n\
             by_path if data.scryer.maintenance.user.fixture_rule.archive\n\
             match if by_path\n\
             unknown if input.observations.added_at.status == \"unknown\"\n\
             reasons contains \"archived\" if archive\n\
             unmonitor if archive\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
}

#[test]
fn the_helper_example_scans_clean() {
    let scan = scan_maintenance_action_heads(&source(ARCHIVE_RULE), "fixture_rule")
        .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    assert_eq!(
        scan.defined_heads.iter().copied().collect::<Vec<_>>(),
        vec![
            MaintenanceActionHead::Unmonitor,
            MaintenanceActionHead::AddTags
        ]
    );
    assert_eq!(scan.add_tags, vec![literal("archived", 7, 14)]);
}

#[test]
fn a_source_that_does_not_parse_has_no_scan() {
    assert!(scan_maintenance_action_heads(&source("unmonitor if {\n"), "fixture_rule").is_err());
}

// ── Validation ──────────────────────────────────────────────────────────────

#[test]
fn validation_refuses_a_computed_action_value_with_its_position() {
    let validation = validate("match := true\nadd_tags := input.facts.tags\n");

    assert!(!validation.valid);
    assert_eq!(validation.diagnostics.len(), 1);
    let diagnostic = &validation.diagnostics[0];
    assert_eq!(
        diagnostic.code,
        diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL
    );
    assert!(diagnostic.line.is_some());
    assert_eq!(diagnostic.column, Some(13));
    let position = format!("{}:{}", diagnostic.line.unwrap(), 13);
    assert!(
        !diagnostic.message.contains(&position),
        "the position travels in its own fields: {}",
        diagnostic.message
    );
}

#[test]
fn validation_refuses_an_unacceptable_literal_that_no_dry_run_would_reach() {
    let validation = validate(
        "match := true\nadd_tags := [\"scryer:managed\"] if input.subject.facet == \"never\"\n",
    );

    assert!(!validation.valid);
    assert_eq!(
        validation.diagnostics[0].code,
        diagnostic_codes::RULE_ACTION_VALUE_INVALID
    );
}

#[test]
fn validation_reports_an_unused_name_without_refusing_the_source() {
    let validation = validate("match := true\ndelet_title if input.facts.has_file\n");

    assert!(validation.valid, "{:?}", validation.errors);
    assert!(validation.errors.is_empty());
    assert!(validation.diagnostics.is_empty());
    assert_eq!(validation.warnings.len(), 1);
    assert_eq!(
        validation.warnings[0].code,
        diagnostic_codes::RULE_UNREFERENCED_RULE
    );
    assert!(validation.warnings[0].line.is_some());
    assert_eq!(validation.warnings[0].column, Some(1));
}

#[test]
fn validation_accepts_a_rule_with_action_heads_and_a_shared_helper() {
    let validation = validate(&format!("match if archive\n{ARCHIVE_RULE}"));

    assert!(validation.valid, "{:?}", validation.errors);
    assert!(validation.warnings.is_empty(), "{:?}", validation.warnings);
}

#[test]
fn validation_refuses_a_wrong_typed_head_the_dry_run_reaches() {
    let validation = validate("match := true\ndelete_title := 1\n");

    assert!(!validation.valid);
    assert_eq!(
        validation.diagnostics[0].code,
        diagnostic_codes::RULE_OUTPUT_INVALID
    );
}
