//! Lifecycle rules that declare their actions as Rego heads: what the engine
//! decodes, what the static scan reads off the source, and the `days_since`
//! helper the host puts in every rule's package.

use crate::diagnostic_codes;
use crate::lifecycle::{
    LifecycleEvalResult, LifecycleInput, LifecycleOutcome, LifecyclePolicy, LifecycleRulesEngine,
    Observation, rewrite_package_declaration, synthetic_lifecycle_input,
};
use crate::lifecycle_actions::{
    LifecycleActionHead, LifecycleActionLiteral, LifecycleActionScan, LifecycleActions,
    MAX_NOTIFY_LEN, scan_lifecycle_action_heads,
};
use crate::validation::{ValidationResult, validate_lifecycle_rule};
use chrono::{DateTime, Utc};

fn policy(id: &str, body: &str) -> LifecyclePolicy {
    LifecyclePolicy {
        id: id.to_string(),
        name: format!("rule {id}"),
        rego_source: rewrite_package_declaration(body, id),
    }
}

fn evaluate_against(body: &str, input: LifecycleInput) -> LifecycleEvalResult {
    let engine =
        LifecycleRulesEngine::build(&[policy("fixture_rule", body)]).expect("rule compiles");
    engine
        .evaluator()
        .evaluate(&input)
        .expect("evaluation should succeed")
}

fn evaluate(body: &str) -> LifecycleEvalResult {
    evaluate_against(body, synthetic_lifecycle_input())
}

/// The actions of the one record a well-formed rule produces.
fn actions_of(result: &LifecycleEvalResult) -> &LifecycleActions {
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.records.len(), 1);
    &result.records[0].decision.actions
}

fn validate(body: &str) -> ValidationResult {
    let source = rewrite_package_declaration(body, "fixture_rule");
    validate_lifecycle_rule(&source, "fixture_rule").expect("validation should not fail outright")
}

/// A complete source with nothing rewritten, so line numbers in a test are the
/// line numbers in the string.
fn source(body: &str) -> String {
    format!("package scryer.maintenance.user.fixture_rule\nimport rego.v1\n{body}")
}

fn literal(value: &str, line: usize, column: usize) -> LifecycleActionLiteral {
    LifecycleActionLiteral {
        value: value.to_string(),
        line,
        column,
    }
}

/// The synthetic run happens at 2023-11-14T22:13:20Z.
fn input_added_at(added_at: &str) -> LifecycleInput {
    let mut input = synthetic_lifecycle_input();
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
         remove_tags := [\"new\"] if input.facts.has_file\n\
         monitor if input.facts.has_file\n\
         fetch_subtitles if input.facts.has_file\n\
         move_to_library := \"Fixture Archive\" if input.facts.has_file\n\
         move_to_root := \"/fixture/archive\" if input.facts.has_file\n\
         notify := \"Fixture archived\" if input.facts.has_file\n",
    );

    let actions = actions_of(&result);
    assert_eq!(
        actions,
        &LifecycleActions {
            unmonitor: true,
            delete_files: true,
            delete_title: true,
            search: true,
            set_quality_profile: Some("Fixture HD".to_string()),
            add_tags: vec!["stale".to_string(), "fixture tag".to_string()],
            remove_tags: vec!["new".to_string()],
            monitor: true,
            fetch_subtitles: true,
            move_to_library: Some("Fixture Archive".to_string()),
            move_to_root: Some("/fixture/archive".to_string()),
            notify: Some("Fixture archived".to_string()),
        }
    );
    assert!(!actions.is_empty());
    assert!(actions.defines_destructive_action());
    assert_eq!(actions.heads(), LifecycleActionHead::ALL.to_vec());
}

#[test]
fn each_boolean_head_is_decoded_on_its_own() {
    for head in [
        LifecycleActionHead::Unmonitor,
        LifecycleActionHead::DeleteFiles,
        LifecycleActionHead::DeleteTitle,
        LifecycleActionHead::Search,
        LifecycleActionHead::Monitor,
        LifecycleActionHead::FetchSubtitles,
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

/// The wrapper's default for every head decodes to no action. A source that
/// defines no head at all never gets this far: it is refused at load.
#[test]
fn undefined_heads_decode_to_no_actions() {
    let defaults: String = LifecycleActionHead::ALL
        .iter()
        .map(|head| format!("\"{}\": {}", head.rule_name(), head.wrapper_default()))
        .collect::<Vec<_>>()
        .join(", ");
    let value = regorus::Value::from_json_str(&format!("{{{defaults}}}")).expect("json");

    let actions = &crate::lifecycle_actions::decode_actions(&value).expect("decodes");
    assert_eq!(actions, &LifecycleActions::default());
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

/// The outcome follows the action heads: a subject matches when at least
/// one head fired for it, and a rule named `match` is an ordinary helper.
#[test]
fn a_fired_action_head_is_what_makes_a_match() {
    let fired = evaluate("unmonitor if input.facts.monitored\n");
    assert!(actions_of(&fired).unmonitor);
    assert_eq!(fired.records[0].decision.outcome, LifecycleOutcome::Match);

    let none_fired = evaluate("match := true\nunmonitor if not match\n");
    assert!(actions_of(&none_fired).is_empty());
    assert_eq!(
        none_fired.records[0].decision.outcome,
        LifecycleOutcome::NoMatch
    );

    let held = evaluate("unknown := true\nunmonitor := true\n");
    assert_eq!(held.records[0].decision.outcome, LifecycleOutcome::Unknown);
}

/// A flag of the wrong type gets past the scan, which reads only value heads,
/// and fails when its decision is decoded. A value head that is not a literal
/// the action accepts never runs: the engine refuses the source at load with
/// the scan's code.
#[test]
fn a_head_of_the_wrong_type_is_an_error_not_a_coerced_action() {
    let malformed = diagnostic_codes::RULE_OUTPUT_INVALID;
    let not_literal = diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL;
    let invalid = diagnostic_codes::RULE_ACTION_VALUE_INVALID;
    for (body, code) in [
        ("unmonitor := \"yes\"\n", malformed),
        ("delete_files := [true]\n", malformed),
        ("delete_title := 1\n", malformed),
        ("search := null\n", malformed),
        ("set_quality_profile := 7\n", not_literal),
        ("set_quality_profile := true\n", not_literal),
        ("set_quality_profile := [\"Fixture HD\"]\n", not_literal),
        ("set_quality_profile := \"\"\n", invalid),
        ("add_tags := [\"stale\", 1]\n", not_literal),
        ("add_tags := \"stale\"\n", not_literal),
        ("add_tags := {\"stale\"}\n", not_literal),
        ("add_tags contains \"stale\"\n", not_literal),
        ("remove_tags := [null]\n", not_literal),
        ("remove_tags := true\n", not_literal),
        ("monitor := \"yes\"\n", malformed),
        ("fetch_subtitles := 1\n", malformed),
        ("move_to_library := 7\n", not_literal),
        ("move_to_library := [\"Fixture Archive\"]\n", not_literal),
        ("move_to_library := \" \"\n", invalid),
        ("move_to_root := true\n", not_literal),
        ("move_to_root := \"\"\n", invalid),
        ("notify := input.facts.title\n", not_literal),
        ("notify := \"\"\n", invalid),
    ] {
        let result = evaluate(&format!("unknown := false\n{body}"));

        assert!(result.records.is_empty(), "{body}: {:?}", result.records);
        assert_eq!(result.errors.len(), 1, "{body}");
        assert_eq!(result.errors[0].code, code, "{body}");
    }
}

#[test]
fn a_notify_message_longer_than_the_bound_is_refused() {
    let at_bound = "n".repeat(MAX_NOTIFY_LEN);
    let fits = evaluate(&format!("notify := \"{at_bound}\"\n"));
    assert_eq!(actions_of(&fits).notify.as_deref(), Some(at_bound.as_str()));

    let over = evaluate(&format!("notify := \"{at_bound}n\"\n"));
    assert!(over.records.is_empty(), "{:?}", over.records);
    assert_eq!(over.errors.len(), 1);
    assert_eq!(
        over.errors[0].code,
        diagnostic_codes::RULE_ACTION_VALUE_INVALID
    );
}

/// Heads that cannot be applied together are found in execution order, so a
/// decision with any such pair can be held as a whole.
#[test]
fn conflicting_heads_are_reported_as_a_pair() {
    let decided = |body: &str| actions_of(&evaluate(body)).clone();
    for (body, pair) in [
        (
            "monitor := true\nunmonitor := true\n",
            (LifecycleActionHead::Monitor, LifecycleActionHead::Unmonitor),
        ),
        (
            "monitor := true\ndelete_title := true\n",
            (
                LifecycleActionHead::Monitor,
                LifecycleActionHead::DeleteTitle,
            ),
        ),
        (
            "move_to_library := \"Fixture Archive\"\nmove_to_root := \"/fixture/archive\"\n",
            (
                LifecycleActionHead::MoveToLibrary,
                LifecycleActionHead::MoveToRoot,
            ),
        ),
        (
            "move_to_root := \"/fixture/archive\"\ndelete_files := true\n",
            (
                LifecycleActionHead::MoveToRoot,
                LifecycleActionHead::DeleteFiles,
            ),
        ),
    ] {
        assert_eq!(decided(body).conflict(), Some(pair), "{body}");
    }
    for body in [
        "monitor := true\nsearch := true\nnotify := \"Fixture\"\n",
        "move_to_library := \"Fixture Archive\"\nunmonitor := true\n",
        "unmonitor := true\ndelete_files := true\n",
    ] {
        assert_eq!(decided(body).conflict(), None, "{body}");
    }
}

/// Decisions stored before the newer heads existed still read, and a decision
/// that uses none of them is written back in the same form.
#[test]
fn decided_actions_json_without_the_newer_heads_round_trips_unchanged() {
    let stored = r#"{"unmonitor":true,"delete_files":false,"delete_title":false,"search":false,"set_quality_profile":null,"add_tags":["stale"],"remove_tags":[]}"#;
    let actions: LifecycleActions = serde_json::from_str(stored).expect("old form reads");
    assert!(actions.unmonitor);
    assert!(!actions.monitor);
    assert_eq!(actions.move_to_library, None);
    assert_eq!(serde_json::to_string(&actions).expect("serializes"), stored);

    let newer = LifecycleActions {
        monitor: true,
        move_to_root: Some("/fixture/archive".to_string()),
        notify: Some("Fixture".to_string()),
        ..LifecycleActions::default()
    };
    let json = serde_json::to_string(&newer).expect("serializes");
    assert_eq!(
        serde_json::from_str::<LifecycleActions>(&json).expect("reads back"),
        newer
    );
}

#[test]
fn the_scan_collects_move_and_notify_literals_with_their_positions() {
    let scan = scan_lifecycle_action_heads(
        &source(
            "move_to_library := \"Fixture Archive\" if input.facts.has_file\n\
             move_to_root := \"/fixture/archive\" if not input.facts.has_file\n\
             notify := \"Fixture moved\" if input.facts.has_file\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.libraries, vec![literal("Fixture Archive", 3, 20)]);
    assert_eq!(scan.roots, vec![literal("/fixture/archive", 4, 17)]);
    assert_eq!(scan.notifications, vec![literal("Fixture moved", 5, 11)]);
    assert!(scan.remove_tags.is_empty());
    assert!(!scan.defines_destructive_action());
}

#[test]
fn tags_keep_the_bounds_every_policy_tag_has() {
    let result = evaluate("add_tags := [\"scryer:managed\"]\n");

    assert!(result.records.is_empty(), "{:?}", result.records);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result.errors[0].code,
        diagnostic_codes::RULE_ACTION_VALUE_INVALID
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
        LifecycleOutcome::Unknown
    );
    assert!(actions_of(&result).is_empty());
}

#[test]
fn a_rule_the_host_holds_gets_no_actions() {
    let mut input = synthetic_lifecycle_input();
    input.facts.active_downloads = Observation::unknown("not_yet_collected");

    let result = evaluate_against("delete_title if not input.facts.active_downloads\n", input);

    assert_eq!(
        result.records[0].decision.outcome,
        LifecycleOutcome::Unknown
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
    let mut stale = synthetic_lifecycle_input();
    stale.facts.last_watched_by_anyone_at = Observation::known("2023-01-01T00:00:00Z".to_string());
    let result = evaluate_against(ARCHIVE_RULE, stale);
    assert_eq!(
        actions_of(&result),
        &LifecycleActions {
            unmonitor: true,
            add_tags: vec!["archived".to_string()],
            ..LifecycleActions::default()
        }
    );

    let mut recent = synthetic_lifecycle_input();
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

    let mut many_files = synthetic_lifecycle_input();
    many_files.facts.file_count = Observation::known(9);
    assert_eq!(
        actions_of(&evaluate_against(BODY, many_files))
            .set_quality_profile
            .as_deref(),
        Some("Fixture UHD")
    );

    let mut no_file = synthetic_lifecycle_input();
    no_file.facts.has_file = Observation::known(false);
    assert_eq!(
        actions_of(&evaluate_against(BODY, no_file)).set_quality_profile,
        None
    );
}

// ── days_since ──────────────────────────────────────────────────────────────

/// Reports the number of days the helper returned as a reason, so a test can
/// read the exact value back. An action value must be a literal, so a reason
/// carries it.
const DAYS_PROBE: &str = "unmonitor := true\n\
                          reasons contains sprintf(\"days %d\", [days_since(input.facts.added_at)])\n";

fn probed_days(input: LifecycleInput) -> Vec<String> {
    let result = evaluate_against(DAYS_PROBE, input);
    assert!(actions_of(&result).unmonitor);
    result.records[0].decision.reason_codes.clone()
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

    let engine =
        LifecycleRulesEngine::build(&[policy("fixture_rule", DAYS_PROBE)]).expect("rule compiles");
    assert!(
        !engine.reads_clock(),
        "the helper must not make a rule look like it reads the wall clock"
    );
}

#[test]
fn days_since_of_a_missing_fact_is_undefined_so_the_condition_is_false() {
    let mut input = synthetic_lifecycle_input();
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
    let engine = LifecycleRulesEngine::build(&[policy(
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

    let mut input = synthetic_lifecycle_input();
    input.facts.added_at = Observation::unknown("source_unavailable");
    let result = engine.evaluator().evaluate(&input).expect("evaluates");

    assert_eq!(
        result.records[0].decision.outcome,
        LifecycleOutcome::Unknown
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

/// A stored source that defines its own `days_since` still loads, because
/// the host leaves its helper out rather than conflict with it, but it never
/// decides: the engine refuses it the way validation does.
#[test]
fn an_author_definition_of_days_since_is_refused_at_save_and_at_load() {
    // A function of the same shape.
    let function = "days_since(timestamp) := 7 if timestamp != \"\"\n\
                    unmonitor if days_since(input.facts.added_at) == 7\n";
    // A function of another arity, which could not coexist with the host's.
    let other_arity = "days_since(from, to) := 3 if from != to\n\
                       unmonitor if days_since(input.facts.added_at, \"fixture\") == 3\n";
    // A plain value under the same name.
    let value = "days_since := 5\nunmonitor if days_since == 5\n";

    for body in [function, other_arity, value] {
        let result = evaluate(body);
        assert!(result.records.is_empty(), "{body}: {:?}", result.records);
        assert_eq!(
            result.errors[0].code,
            diagnostic_codes::RULE_HOST_HELPER_REDEFINED,
            "{body}"
        );
    }

    // Validation refuses every redefinition, pointing at the definition.
    for body in [function, other_arity, value] {
        let validation = validate(&format!("unmonitor := true\n{body}"));
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
    let validation = validate("unmonitor if days_since(input.facts.added_at) > 30\n");

    assert!(validation.valid, "{:?}", validation.errors);
    assert!(validation.warnings.is_empty(), "{:?}", validation.warnings);
}

// ── Static head scan ────────────────────────────────────────────────────────

#[test]
fn the_scan_reports_the_action_heads_a_source_defines() {
    let scan = scan_lifecycle_action_heads(
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
            LifecycleActionHead::Unmonitor,
            LifecycleActionHead::DeleteFiles,
            LifecycleActionHead::AddTags,
        ]
    );
    assert!(
        scan.defined_heads
            .contains(&LifecycleActionHead::DeleteFiles)
    );
    assert!(
        !scan
            .defined_heads
            .contains(&LifecycleActionHead::DeleteTitle)
    );
    assert!(scan.defines_destructive_action());

    let harmless = scan_lifecycle_action_heads(
        &source("unmonitor := true\nsearch := true\n"),
        "fixture_rule",
    )
    .expect("source parses");
    assert!(!harmless.defines_destructive_action());
}

#[test]
fn the_scan_collects_every_literal_value_with_its_position() {
    let scan = scan_lifecycle_action_heads(
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
            LifecycleActionHead::SetQualityProfile,
            LifecycleActionHead::AddTags,
            LifecycleActionHead::RemoveTags,
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
    let scan: LifecycleActionScan = scan_lifecycle_action_heads(
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
    ] {
        let scan =
            scan_lifecycle_action_heads(&source(body), "fixture_rule").expect("source parses");

        assert_eq!(scan.errors.len(), 1, "{body}: {:?}", scan.errors);
        let error = &scan.errors[0];
        assert_eq!(
            error.code,
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            "{body}"
        );
        assert_eq!((error.line, error.column), (line, column), "{body}");
        // Only the literal first branch of the `else` case contributes a
        // literal; a refused definition contributes nothing.
        let expected_profiles = if line == 3 && column == 69 {
            vec![literal("Fixture HD", 3, 24)]
        } else {
            Vec::new()
        };
        assert_eq!(scan.quality_profiles, expected_profiles, "{body}");
        assert!(scan.add_tags.is_empty(), "{body}: {:?}", scan.add_tags);
        assert!(
            scan.remove_tags.is_empty(),
            "{body}: {:?}",
            scan.remove_tags
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
            scan_lifecycle_action_heads(&source(body), "fixture_rule").expect("source parses");

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
fn a_tag_head_with_more_entries_than_a_decision_allows_is_refused_at_save() {
    let tags: Vec<String> = (0..=crate::policy::MAX_TAGS)
        .map(|index| format!("\"tag{index}\""))
        .collect();
    for head in ["add_tags", "remove_tags"] {
        let body = format!("{head} := [{}]\n", tags.join(", "));
        let scan =
            scan_lifecycle_action_heads(&source(&body), "fixture_rule").expect("source parses");
        assert_eq!(scan.errors.len(), 1, "{head}: {:?}", scan.errors);
        assert_eq!(
            scan.errors[0].code,
            diagnostic_codes::RULE_ACTION_VALUE_INVALID
        );
        assert_eq!(
            (scan.errors[0].line, scan.errors[0].column),
            (3, head.len() + 5),
            "{head}"
        );

        let at_limit = format!(
            "{head} := [{}]\n",
            tags[..crate::policy::MAX_TAGS].join(", ")
        );
        let scan =
            scan_lifecycle_action_heads(&source(&at_limit), "fixture_rule").expect("source parses");
        assert!(scan.errors.is_empty(), "{head}: {:?}", scan.errors);
    }
}

#[test]
fn a_rule_name_nothing_uses_is_a_warning() {
    let scan = scan_lifecycle_action_heads(
        &source(
            "unknown := false\n\
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
    let scan = scan_lifecycle_action_heads(
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
             search if by_path\n\
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
fn with_may_not_replace_input_data_or_a_helper_name() {
    let scan = scan_lifecycle_action_heads(
        &source(
            "old if input.facts.file_count > 5\n\
             unmonitor if old with input.facts.file_count as 9\n\
             search if old with data.scryer.maintenance.user.fixture_rule.old as true\n\
             delete_files if days_since(input.facts.added_at) > 1 with time.now_ns as 0\n\
             add_tags := [\"a\"] if count([x | x := 1; old with input as {}]) > 0\n\
             remove_tags := [\"b\"] if every x in [1] { old with floor as 1 }\n\
             set_quality_profile := \"HD\" if search with old as true\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    let refused: Vec<(&str, &str, usize)> = scan
        .errors
        .iter()
        .map(|error| (error.code, error.rule.as_str(), error.line))
        .collect();
    assert_eq!(
        refused,
        vec![
            (diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED, "input", 4),
            (diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED, "data", 5),
            (diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED, "time", 6),
            (diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED, "input", 7),
            (diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED, "floor", 8),
        ],
        "a rule's own helper may still be replaced"
    );
}

#[test]
fn a_helper_used_through_an_import_alias_does_not_warn() {
    let scan = scan_lifecycle_action_heads(
        &source(
            "import data.scryer.maintenance.user.fixture_rule as me\n\
             import data.scryer.maintenance.user.fixture_rule.stale as is_stale\n\
             old if input.facts.file_count > 5\n\
             stale if input.facts.file_count > 9\n\
             unused_helper := 1\n\
             unmonitor if me.old\n\
             search if is_stale\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    let warned: Vec<(&str, &str)> = scan
        .warnings
        .iter()
        .map(|warning| (warning.code, warning.rule.as_str()))
        .collect();
    assert_eq!(
        warned,
        vec![(diagnostic_codes::RULE_UNREFERENCED_RULE, "unused_helper")],
        "only the helper nothing reaches is reported"
    );
}

#[test]
fn the_helper_example_scans_clean() {
    let scan =
        scan_lifecycle_action_heads(&source(ARCHIVE_RULE), "fixture_rule").expect("source parses");

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    assert_eq!(
        scan.defined_heads.iter().copied().collect::<Vec<_>>(),
        vec![LifecycleActionHead::Unmonitor, LifecycleActionHead::AddTags]
    );
    assert_eq!(scan.add_tags, vec![literal("archived", 7, 14)]);
}

#[test]
fn a_source_that_does_not_parse_has_no_scan() {
    assert!(scan_lifecycle_action_heads(&source("unmonitor if {\n"), "fixture_rule").is_err());
}

// ── Validation ──────────────────────────────────────────────────────────────

#[test]
fn validation_refuses_a_computed_action_value_with_its_position() {
    let validation = validate("unmonitor := true\nadd_tags := input.facts.tags\n");

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
        "unmonitor := true\nadd_tags := [\"scryer:managed\"] if input.subject.facet == \"never\"\n",
    );

    assert!(!validation.valid);
    assert_eq!(
        validation.diagnostics[0].code,
        diagnostic_codes::RULE_ACTION_VALUE_INVALID
    );
}

#[test]
fn validation_reports_an_unused_name_without_refusing_the_source() {
    let validation = validate("unmonitor := true\ndelet_title if input.facts.has_file\n");

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
    let validation = validate(&format!("search if archive\n{ARCHIVE_RULE}"));

    assert!(validation.valid, "{:?}", validation.errors);
    assert!(validation.warnings.is_empty(), "{:?}", validation.warnings);
}

#[test]
fn validation_refuses_a_wrong_typed_head_the_dry_run_reaches() {
    let validation = validate("unmonitor := true\ndelete_title := 1\n");

    assert!(!validation.valid);
    assert_eq!(
        validation.diagnostics[0].code,
        diagnostic_codes::RULE_OUTPUT_INVALID
    );
}

// ── Sources the scan refuses, at save and at load ───────────────────────────

/// The first scan error for a body, as (code, line, column).
fn first_scan_error(body: &str) -> (&'static str, usize, usize) {
    let scan = scan_lifecycle_action_heads(&source(body), "fixture_rule").expect("source parses");
    let error = scan
        .errors
        .first()
        .unwrap_or_else(|| panic!("{body}: no scan error"));
    (error.code, error.line, error.column)
}

/// Regorus evaluates a function of no arguments into the package document, so
/// `delete_title() if …` would delete while the scan saw no head. Any function
/// under an action name is refused, whatever its arity.
#[test]
fn an_action_head_written_as_a_function_is_refused_at_any_arity() {
    for body in [
        "delete_title() if input.facts.has_file\n",
        "unmonitor() := true\n",
        "add_tags() := [\"stale\"]\n",
        "add_tags(extra) := [\"stale\"] if extra\n",
        "search(x) if x\n",
    ] {
        assert_eq!(
            first_scan_error(body),
            (diagnostic_codes::RULE_ACTION_HEAD_FUNCTION, 3, 1),
            "{body}"
        );
    }

    let scan = scan_lifecycle_action_heads(
        &source("delete_title() if input.facts.has_file\n"),
        "fixture_rule",
    )
    .expect("source parses");
    assert!(!scan.defines_destructive_action());
    let result = evaluate("delete_title() if input.facts.has_file\n");
    assert!(result.records.is_empty(), "{:?}", result.records);
    assert_eq!(
        result.errors[0].code,
        diagnostic_codes::RULE_ACTION_HEAD_FUNCTION
    );
}

/// A stored source the scan refuses is never evaluated: the engine reports it
/// in error with the scan's code instead of applying the heads it can read.
#[test]
fn the_engine_refuses_a_source_the_scan_refuses() {
    for (body, code) in [
        (
            "unmonitor if input.facts.has_file\nadd_tags := input.facts.tags\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
        ),
        (
            "unmonitor if input.facts.has_file\ndays_since(ts) := 0 if ts\n",
            diagnostic_codes::RULE_HOST_HELPER_REDEFINED,
        ),
        (
            "unmonitor if data.scryer.maintenance.user.other_rule.match\n",
            diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE,
        ),
        (
            "match if input.facts.has_file\n",
            diagnostic_codes::RULE_ACTION_REQUIRED,
        ),
    ] {
        let result = evaluate(body);

        assert!(result.records.is_empty(), "{body}: {:?}", result.records);
        assert_eq!(result.errors.len(), 1, "{body}");
        assert_eq!(result.errors[0].code, code, "{body}");
    }

    let clean = evaluate("unmonitor if input.facts.has_file\n");
    assert!(clean.errors.is_empty(), "{:?}", clean.errors);
}

/// Another rule's document, or `data` as a whole, is outside the unknown-fact
/// hold: the host never sees which facts it was built from.
#[test]
fn a_read_of_data_outside_the_rule_package_is_refused_with_its_position() {
    for (body, line, column) in [
        (
            "unmonitor if data.scryer.maintenance.user.other_rule.match\n",
            3,
            14,
        ),
        ("unmonitor if data.scryer.maintenance\n", 3, 14),
        ("unmonitor if count(data) > 0\n", 3, 20),
        ("unmonitor if data[\"scryer\"]\n", 3, 14),
        ("unmonitor if {\n\tsome x in data.scryer\n\tx\n}\n", 4, 12),
    ] {
        assert_eq!(
            first_scan_error(body),
            (diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE, line, column),
            "{body}"
        );
    }

    let scan = scan_lifecycle_action_heads(
        &format!(
            "package scryer.maintenance.user.fixture_rule\nimport rego.v1\n\
             import data.scryer.maintenance.user.other_rule\n\
             unmonitor if other_rule.match\n"
        ),
        "fixture_rule",
    )
    .expect("source parses");
    assert_eq!(
        scan.errors
            .iter()
            .map(|error| (error.code, error.line, error.column))
            .collect::<Vec<_>>(),
        vec![(diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE, 3, 8)]
    );

    let own = scan_lifecycle_action_heads(
        &source(
            "archive if input.facts.has_file\n\
             unmonitor if data.scryer.maintenance.user.fixture_rule.archive\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");
    assert!(own.errors.is_empty(), "{:?}", own.errors);
}

/// Regorus resolves a call in the package before the builtins, and an import
/// alias shadows a builtin namespace, so each name `days_since` calls is as
/// reserved as `days_since` itself.
#[test]
fn a_name_the_day_count_helper_calls_cannot_be_redefined() {
    for (body, line, column) in [
        (
            "unmonitor if input.facts.has_file\nfloor(x) := 0 if x\n",
            4,
            1,
        ),
        (
            "unmonitor if input.facts.has_file\nis_string(x) if x\n",
            4,
            1,
        ),
        (
            "unmonitor if input.facts.has_file\ntime.parse_rfc3339_ns(x) := 0 if x\n",
            4,
            1,
        ),
        ("unmonitor if input.facts.has_file\ndays_since := 3\n", 4, 1),
    ] {
        assert_eq!(
            first_scan_error(body),
            (diagnostic_codes::RULE_HOST_HELPER_REDEFINED, line, column),
            "{body}"
        );
    }

    let aliased = scan_lifecycle_action_heads(
        "package scryer.maintenance.user.fixture_rule\nimport rego.v1\n\
         import input.facts as time\n\
         unmonitor if input.facts.has_file\n",
        "fixture_rule",
    )
    .expect("source parses");
    assert_eq!(
        aliased
            .errors
            .iter()
            .map(|error| (error.code, error.rule.as_str(), error.line, error.column))
            .collect::<Vec<_>>(),
        vec![(diagnostic_codes::RULE_HOST_HELPER_REDEFINED, "time", 3, 23)]
    );

    // Shadowing would otherwise change what the helper returns.
    let result = evaluate(
        "floor(x) := 1000 if x\n\
         unmonitor if days_since(\"2000-01-01T00:00:00Z\") == 1000\n",
    );
    assert!(result.records.is_empty(), "{:?}", result.records);
    assert_eq!(
        result.errors[0].code,
        diagnostic_codes::RULE_HOST_HELPER_REDEFINED
    );
}

#[test]
fn more_tags_than_a_decision_may_carry_are_refused_by_the_scan() {
    let tags = (0..17)
        .map(|index| format!("\"tag{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        first_scan_error(&format!("add_tags := [{tags}]\n")),
        (diagnostic_codes::RULE_ACTION_VALUE_INVALID, 3, 13)
    );

    let sixteen = (0..16)
        .map(|index| format!("\"tag{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let scan = scan_lifecycle_action_heads(
        &source(&format!("remove_tags := [{sixteen}]\n")),
        "fixture_rule",
    )
    .expect("source parses");
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.remove_tags.len(), 16);
}

#[test]
fn diagnostics_about_a_named_rule_carry_the_name_in_path() {
    let warned = validate("unmonitor := true\ndelet_title if input.facts.has_file\n");
    assert_eq!(warned.warnings[0].path.as_deref(), Some("delet_title"));

    let refused = validate("unmonitor := true\nfloor(x) := 0 if x\n");
    assert_eq!(
        refused.diagnostics[0].code,
        diagnostic_codes::RULE_HOST_HELPER_REDEFINED
    );
    assert_eq!(refused.diagnostics[0].path.as_deref(), Some("floor"));

    // Action-value codes name their head in the code's own meaning.
    let literal = validate("unmonitor := true\nadd_tags := input.facts.tags\n");
    assert_eq!(literal.diagnostics[0].path, None);
}

/// Regorus counts a tab as four columns and a token by its bytes. Every
/// position a client sees counts characters, as an editor does.
#[test]
fn positions_count_characters_on_tab_indented_and_non_ascii_lines() {
    let scan = scan_lifecycle_action_heads(
        &source(
            "add_tags := [\"stale\"] if {\n\
             \tinput.facts.has_file\n\
             }\n\
             remove_tags := [\"x\"] if {\n\
             \t\tinput.subject.name == \"Café\"\n\
             } else := [\"y\"] if {\n\
             \tinput.facts.monitored\n\
             }\n\
             unmonitor if {\n\
             \tdata.scryer.other\n\
             }\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");
    assert_eq!(scan.add_tags, vec![literal("stale", 3, 14)]);
    assert_eq!(
        scan.remove_tags,
        vec![literal("x", 6, 17), literal("y", 8, 12)]
    );
    assert_eq!(
        scan.errors
            .iter()
            .map(|error| (error.code, error.line, error.column))
            .collect::<Vec<_>>(),
        vec![(diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE, 12, 2)]
    );

    let after_non_ascii = scan_lifecycle_action_heads(
        &source(
            "unmonitor if input.subject.name == \"é\"\nadd_tags := [\"é\", \"stale\"] if true\n",
        ),
        "fixture_rule",
    )
    .expect("source parses");
    assert_eq!(after_non_ascii.add_tags, vec![literal("stale", 4, 19)]);

    // A compile error regorus reports on a tab-indented line.
    let validation = validate("unmonitor if {\n\t\tinput.facts.has_file ==\n}\n");
    assert_eq!(
        validation.diagnostics[0].code,
        diagnostic_codes::RULE_COMPILATION_ERROR
    );
    let column = validation.diagnostics[0]
        .column
        .expect("compile error column");
    let line = validation.diagnostics[0].line.expect("compile error line");
    let source = rewrite_package_declaration(
        "unmonitor if {\n\t\tinput.facts.has_file ==\n}\n",
        "fixture_rule",
    );
    let text: Vec<char> = source.split('\n').nth(line - 1).unwrap().chars().collect();
    assert!(
        column <= text.len() + 1,
        "{column} is past line {line}: {text:?}"
    );
}
