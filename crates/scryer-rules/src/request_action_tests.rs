//! Request rule heads: the static scan, validation, and the runtime decode.

use crate::diagnostic_codes;
use crate::request::{
    RequestPolicy, RequestRulesEngine, RequestVote, rewrite_package_declaration,
    synthetic_request_input,
};
use crate::request_actions::{
    RequestActionHead, RequestDeclaredActions, RequestShaping, scan_request_action_heads,
};
use crate::validation::{ValidationResult, validate_request_rule};

const RULE_ID: &str = "fixture_rule";

fn source(body: &str) -> String {
    rewrite_package_declaration(body, RULE_ID)
}

fn validate(body: &str) -> ValidationResult {
    validate_request_rule(&source(body), RULE_ID).expect("validation runs")
}

fn codes(result: &ValidationResult) -> Vec<&str> {
    result.diagnostics.iter().map(|d| d.code).collect()
}

fn decide(body: &str) -> crate::request::RequestRuleDecision {
    let engine = RequestRulesEngine::build(&[RequestPolicy {
        id: RULE_ID.to_string(),
        name: "fixture".to_string(),
        rego_source: source(body),
    }])
    .expect("policy compiles");
    let mut result = engine
        .evaluator()
        .evaluate(&synthetic_request_input())
        .expect("evaluation runs");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.records.remove(0).decision
}

// ── scan rules ──

#[test]
fn a_head_written_as_a_function_is_refused() {
    for body in [
        "approve(x) if x\nmanual if approve(true)\n",
        "reasons(x) := [x]\napprove if reasons(\"a\")\n",
    ] {
        let scan = scan_request_action_heads(&source(body), RULE_ID).expect("parses");
        assert_eq!(
            scan.errors.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec![diagnostic_codes::RULE_HEAD_DEFINED_AS_FUNCTION],
            "{body}"
        );
        assert_eq!(
            (scan.errors[0].line, scan.errors[0].column),
            (3, 1),
            "{body}"
        );
    }

    // A valued head written as a function keeps its own code.
    let scan =
        scan_request_action_heads(&source("approve := true\nset_monitor(x) := x\n"), RULE_ID)
            .expect("parses");
    assert_eq!(
        scan.errors.iter().map(|e| e.code).collect::<Vec<_>>(),
        vec![diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL]
    );
}

#[test]
fn data_outside_the_rule_package_is_refused() {
    for (body, at) in [
        (
            "approve if data.scryer.request.user.other_rule.approve\n",
            (3, 12),
        ),
        (
            "approve if data.scryer.request.wrapper.fixture_rule.decision.approve\n",
            (3, 12),
        ),
        ("approve if count(data) > 0\n", (3, 18)),
        ("approve if {\n\tsome key\n\tdata[key]\n}\n", (5, 2)),
        (
            "import data.scryer.request.user.other_rule as other\napprove if other.approve\n",
            (3, 8),
        ),
    ] {
        let scan = scan_request_action_heads(&source(body), RULE_ID).expect("parses");
        assert_eq!(
            scan.errors.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec![diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE],
            "{body}"
        );
        assert_eq!((scan.errors[0].line, scan.errors[0].column), at, "{body}");
    }

    let own = scan_request_action_heads(
        &source("helper if input.facts.is_adult\napprove if data.scryer.request.user.fixture_rule.helper\n"),
        RULE_ID,
    )
    .expect("parses");
    assert!(own.errors.is_empty(), "{:?}", own.errors);
}

/// Regorus counts a tab as four columns; an editor counts one character.
#[test]
fn reported_columns_are_character_offsets_after_a_tab() {
    let scan = scan_request_action_heads(
        &source("approve := true\n\tadd_tags := [\"scryer:x\"]\n"),
        RULE_ID,
    )
    .expect("parses");
    assert_eq!(scan.errors.len(), 1, "{:?}", scan.errors);
    assert_eq!(
        scan.errors[0].code,
        diagnostic_codes::RULE_ACTION_VALUE_INVALID
    );
    assert_eq!((scan.errors[0].line, scan.errors[0].column), (4, 15));
}

// ── request heads ──

#[test]
fn a_tags_rule_is_an_unreferenced_helper_like_any_other_name() {
    let result = validate("approve := true\ntags contains \"family\"\n");
    assert!(result.valid, "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
        vec![diagnostic_codes::RULE_UNREFERENCED_RULE]
    );
    assert_eq!(
        (result.warnings[0].line, result.warnings[0].column),
        (Some(4), Some(1))
    );

    let decision = decide("approve := true\ntags contains \"family\"\n");
    assert_eq!(decision.shaping, RequestShaping::default());
}

#[test]
fn a_rule_must_decide_something() {
    for body in [
        "reasons contains \"adult_content\" if input.facts.is_adult\n",
        "notify := \"someone asked\"\n",
        "notify := \"someone asked\"\nreasons contains \"quota\"\n",
    ] {
        let result = validate(body);
        assert!(!result.valid, "{body}");
        assert_eq!(
            codes(&result),
            vec![diagnostic_codes::RULE_VOTE_REQUIRED],
            "{body}"
        );
    }

    for body in [
        "set_lease_days := 7 if input.facts.is_adult\n",
        "set_monitor := \"futureEpisodes\"\n",
        "set_required_audio_languages := []\n",
        "add_tags := [\"family\"]\nnotify := \"tagged\"\n",
    ] {
        let result = validate(body);
        assert!(result.valid, "{body}: {:?}", result.diagnostics);
    }
}

#[test]
fn valued_heads_take_plain_literals_in_range() {
    for (body, code, at) in [
        (
            "set_quality_profile := input.request.quality_profile_name\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            (3, 24),
        ),
        (
            "set_lease_days := input.request.lease_days\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            (3, 19),
        ),
        (
            "set_lease_days := 3651\n",
            diagnostic_codes::RULE_ACTION_VALUE_INVALID,
            (3, 19),
        ),
        (
            "set_lease_days := 1.5\n",
            diagnostic_codes::RULE_ACTION_VALUE_INVALID,
            (3, 19),
        ),
        (
            "set_monitor := \"\"\n",
            diagnostic_codes::RULE_ACTION_VALUE_INVALID,
            (3, 16),
        ),
        (
            "set_preferred_audio_languages := [\"eng\", input.facts.original_language]\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            (3, 34),
        ),
        (
            "set_required_audio_languages := [\"\"]\n",
            diagnostic_codes::RULE_ACTION_VALUE_INVALID,
            (3, 34),
        ),
        (
            "approve := true\nnotify := concat(\" \", [\"for\", input.requester.username])\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            (4, 11),
        ),
        (
            "add_tags contains \"family\"\n",
            diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
            (3, 1),
        ),
    ] {
        let result = validate(body);
        assert!(!result.valid, "{body}");
        assert_eq!(codes(&result), vec![code], "{body}");
        assert_eq!(
            (result.diagnostics[0].line, result.diagnostics[0].column),
            (Some(at.0), Some(at.1)),
            "{body}"
        );
    }

    let too_many = (0..=16)
        .map(|i| format!("\"tag{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let result = validate(&format!("add_tags := [{too_many}]\n"));
    assert_eq!(
        codes(&result),
        vec![diagnostic_codes::RULE_ACTION_VALUE_INVALID]
    );
}

#[test]
fn an_unreferenced_helper_is_a_warning_and_does_not_refuse() {
    let result = validate("approve := true\nset_monitr := \"none\"\n");
    assert!(result.valid, "{:?}", result.diagnostics);
    assert_eq!(
        result.warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
        vec![diagnostic_codes::RULE_UNREFERENCED_RULE]
    );
}

#[test]
fn the_scan_declares_every_literal_a_rule_can_produce() {
    let scan = scan_request_action_heads(
        &source(
            "approve if input.facts.certification_rank <= 2\n\
             set_quality_profile := \"HD\" if input.facts.is_adult else := \"SD\"\n\
             set_lease_days := 0 if input.requester.username == \"alice\"\n\
             set_lease_days := 14 if input.requester.username == \"bob\"\n\
             set_monitor := \"futureEpisodes\"\n\
             set_required_audio_languages := [\"eng\"]\n\
             set_preferred_audio_languages := [\"jpn\", \"eng\"]\n\
             add_tags := [\"family\", \"family\"]\n\
             notify := \"Approved for the family\"\n",
        ),
        RULE_ID,
    )
    .expect("parses");
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(
        scan.declared_actions(),
        RequestDeclaredActions {
            heads: vec![
                RequestActionHead::Approve,
                RequestActionHead::SetQualityProfile,
                RequestActionHead::SetLeaseDays,
                RequestActionHead::SetMonitor,
                RequestActionHead::SetRequiredAudioLanguages,
                RequestActionHead::SetPreferredAudioLanguages,
                RequestActionHead::AddTags,
                RequestActionHead::Notify,
            ],
            quality_profiles: vec!["HD".to_string(), "SD".to_string()],
            lease_days: vec![0, 14],
            monitors: vec!["futureEpisodes".to_string()],
            required_audio_languages: vec!["eng".to_string()],
            preferred_audio_languages: vec!["jpn".to_string(), "eng".to_string()],
            add_tags: vec!["family".to_string()],
            notifications: vec!["Approved for the family".to_string()],
        }
    );
}

// ── runtime decode ──

#[test]
fn a_rule_that_ran_carries_its_shaping_tags_and_notification() {
    let decision = decide(
        "approve := true\n\
         set_quality_profile := \"SD\"\n\
         set_lease_days := 0\n\
         set_monitor := \"none\"\n\
         set_required_audio_languages := []\n\
         set_preferred_audio_languages := [\"jpn\"]\n\
         add_tags := [\"family\"]\n\
         notify := \"shaped\"\n",
    );
    assert_eq!(decision.vote, RequestVote::Approve);
    assert_eq!(
        decision.shaping,
        RequestShaping {
            quality_profile: Some("SD".to_string()),
            lease_days: Some(0),
            monitor: Some("none".to_string()),
            required_audio_languages: Some(Vec::new()),
            preferred_audio_languages: Some(vec!["jpn".to_string()]),
        }
    );
    assert_eq!(decision.tags, vec!["family".to_string()]);
    assert_eq!(decision.notify.as_deref(), Some("shaped"));
}

#[test]
fn a_head_that_did_not_fire_leaves_the_request_alone() {
    let decision = decide(
        "approve := true\nset_lease_days := 7 if input.facts.is_adult\nnotify := \"x\" if input.facts.is_adult\n",
    );
    assert!(decision.shaping.is_empty());
    assert_eq!(decision.notify, None);
}

#[test]
fn a_held_rule_carries_no_shaping() {
    let mut input = synthetic_request_input();
    input.facts.is_adult = crate::request::Observation::unknown("no_adult_flag");
    let engine = RequestRulesEngine::build(&[RequestPolicy {
        id: RULE_ID.to_string(),
        name: "fixture".to_string(),
        rego_source: source("approve := true\nset_lease_days := 7 if input.facts.is_adult\n"),
    }])
    .expect("policy compiles");
    let result = engine
        .evaluator()
        .evaluate(&input)
        .expect("evaluation runs");
    let decision = &result.records[0].decision;
    assert!(decision.held);
    assert!(decision.shaping.is_empty());
    assert_eq!(decision.notify, None);
}

// ── Report heads ────────────────────────────────────────────────────────────

#[test]
fn a_rule_that_mixes_report_and_request_heads_is_refused() {
    for body in [
        "approve := true\nmark_bad := true\n",
        "deny := true\nreject_report := true\n",
        "set_lease_days := 7\nmark_bad := true\n",
        "add_tags := [\"family\"]\nreject_report := true\n",
    ] {
        let result = validate(body);
        assert!(!result.valid, "{body}");
        assert!(
            codes(&result).contains(&diagnostic_codes::RULE_MIXES_REPORT_AND_REQUEST_HEADS),
            "{body}: {:?}",
            result.diagnostics
        );
    }

    for body in [
        "mark_bad := true if input.report.reason == \"corrupted\"\n",
        "reject_report := true if not input.report.has_details\nnotify := \"rejected\"\n",
        "mark_bad := true if input.report.facts.reporter_requested_media\nreasons contains \"own_request\"\n",
        "mark_bad := true\nreject_report := true if input.report.reason == \"other\"\n",
    ] {
        let result = validate(body);
        assert!(result.valid, "{body}: {:?}", result.diagnostics);
    }
}

#[test]
fn report_heads_decode_into_a_report_vote() {
    use crate::request::RequestReportVote;
    assert_eq!(
        decide("mark_bad := true\n").report_vote,
        RequestReportVote::MarkBad
    );
    assert_eq!(
        decide("reject_report := true\n").report_vote,
        RequestReportVote::Reject
    );
    assert_eq!(
        decide("mark_bad := true\nreject_report := true\n").report_vote,
        RequestReportVote::Conflict
    );
    let silent = decide("mark_bad := true if input.report.reason == \"other\"\n");
    assert_eq!(silent.report_vote, RequestReportVote::Abstain);
    assert_eq!(silent.vote, RequestVote::Abstain);
}

#[test]
fn a_report_rule_reading_an_unknown_report_fact_is_held() {
    let mut input = synthetic_request_input();
    input
        .report
        .as_mut()
        .unwrap()
        .facts
        .reporter_requested_media = crate::request::Observation::unknown("lookup_failed");
    let engine = RequestRulesEngine::build(&[RequestPolicy {
        id: RULE_ID.to_string(),
        name: "fixture".to_string(),
        rego_source: source("mark_bad := true if input.report.facts.reporter_requested_media\n"),
    }])
    .expect("policy compiles");
    let result = engine
        .evaluator()
        .evaluate(&input)
        .expect("evaluation runs");
    let decision = &result.records[0].decision;
    assert!(decision.held);
    assert_eq!(
        decision.report_vote,
        crate::request::RequestReportVote::Abstain
    );
}

#[test]
fn report_rules_read_only_the_report_fact_family() {
    let report_family: &[&str] = &["report.reporter_requested_media"];
    let title_family: &[&str] = &["is_adult"];
    let reads = |body: &str| {
        crate::validation::request_rule_may_read_fact_families(
            &source(body),
            RULE_ID,
            &[report_family, title_family],
        )
    };
    assert_eq!(
        reads("mark_bad := true if input.report.facts.reporter_requested_media\n"),
        vec![true, false]
    );
    assert_eq!(
        reads(
            "mark_bad := true if input.report.observations.reporter_requested_media.status == \"unknown\"\n"
        ),
        vec![true, false]
    );
    assert_eq!(
        reads("mark_bad := true if input.report.reason == \"corrupted\"\n"),
        vec![false, false]
    );
}

// ── The head table ──────────────────────────────────────────────────────────

/// The wrapper projects heads in this order, and stored shaping and the
/// GraphQL enum follow it.
#[test]
fn the_heads_keep_their_order() {
    use crate::request_actions::REQUEST_HEADS;
    use RequestActionHead::*;
    assert_eq!(
        RequestActionHead::ALL,
        [
            Approve,
            Deny,
            Manual,
            SetQualityProfile,
            SetLeaseDays,
            SetMonitor,
            SetRequiredAudioLanguages,
            SetPreferredAudioLanguages,
            AddTags,
            Notify,
            MarkBad,
            RejectReport,
        ]
    );
    assert_eq!(
        REQUEST_HEADS.last().map(|spec| spec.rule_name),
        Some("reasons")
    );
}

#[test]
fn every_head_is_the_table_entry_at_its_index() {
    use crate::request_actions::REQUEST_HEADS;
    let actions: Vec<RequestActionHead> = REQUEST_HEADS
        .iter()
        .filter_map(|spec| spec.action)
        .collect();
    assert_eq!(actions, RequestActionHead::ALL.to_vec());
    for head in RequestActionHead::ALL {
        assert_eq!(REQUEST_HEADS[head as usize].action, Some(head));
        assert_eq!(
            RequestActionHead::from_rule_name(head.rule_name()),
            Some(head)
        );
    }
    let mut ids: Vec<&str> = REQUEST_HEADS.iter().map(|spec| spec.id).collect();
    let mut names: Vec<&str> = REQUEST_HEADS.iter().map(|spec| spec.rule_name).collect();
    ids.sort_unstable();
    ids.dedup();
    names.sort_unstable();
    names.dedup();
    assert_eq!(ids.len(), REQUEST_HEADS.len(), "ids are distinct");
    assert_eq!(names.len(), REQUEST_HEADS.len(), "rule names are distinct");
    for spec in &REQUEST_HEADS {
        assert_eq!(
            spec.id,
            spec.rule_name.to_ascii_uppercase(),
            "the id is the rule name in capitals"
        );
    }
}

#[test]
fn the_wrapper_projects_every_table_entry() {
    use crate::request_actions::REQUEST_HEADS;
    let wrapper = crate::request::decision_wrapper_source(RULE_ID);
    for spec in &REQUEST_HEADS {
        assert!(
            wrapper.contains(&format!(
                "\"{name}\": object.get(data.scryer.request.user.{RULE_ID}, \"{name}\", {default})",
                name = spec.rule_name,
                default = spec.wrapper_default
            )),
            "{} missing from the wrapper:\n{wrapper}",
            spec.rule_name
        );
    }
}

#[test]
fn the_shaping_and_declared_consumers_cover_every_valued_head() {
    use crate::request_actions::{
        DeclaredSlot, REQUEST_HEADS, RequestHeadRole, RequestHeadValue, RequestValueKind,
    };
    for spec in &REQUEST_HEADS {
        let Some(head) = spec.action else {
            continue;
        };
        let sample = match spec.value_kind {
            RequestValueKind::Flag => None,
            RequestValueKind::Number => Some(RequestHeadValue::Number(
                spec.sample[0].parse().expect("number sample"),
            )),
            RequestValueKind::Text | RequestValueKind::Choice => {
                Some(RequestHeadValue::Text(spec.sample[0].to_string()))
            }
            RequestValueKind::TextList => Some(RequestHeadValue::List(
                spec.sample.iter().map(|item| item.to_string()).collect(),
            )),
        };
        let mut shaping = RequestShaping::default();
        shaping.set(head, sample.clone());
        assert_eq!(
            shaping.get(head),
            if spec.role == RequestHeadRole::Shaping {
                sample
            } else {
                None
            },
            "{} round-trips through the shaping exactly when it shapes",
            spec.rule_name
        );
        assert_eq!(
            head.is_shaping(),
            spec.role == RequestHeadRole::Shaping,
            "{}",
            spec.rule_name
        );
        let mut declared = RequestDeclaredActions::default();
        let slot = declared.slot_mut(head);
        match (spec.value_kind, slot) {
            (RequestValueKind::Flag, None)
            | (RequestValueKind::Number, Some(DeclaredSlot::Numbers(_)))
            | (
                RequestValueKind::Text | RequestValueKind::Choice | RequestValueKind::TextList,
                Some(DeclaredSlot::Texts(_)),
            ) => {}
            _ => panic!("{} has no declared slot of its kind", spec.rule_name),
        }
    }
}

#[test]
fn every_template_filled_with_its_sample_validates_and_declares_the_head() {
    use crate::request_actions::{BUILDER_HELPER_RULE, REQUEST_HEADS};
    for spec in &REQUEST_HEADS {
        let output = spec.fill_template(BUILDER_HELPER_RULE, spec.sample);
        // Reasons and a notification alone decide nothing, so they are paired
        // with a vote the way the builder's output rules require.
        let companion = if spec.role.decides() {
            String::new()
        } else {
            format!("\napprove if {BUILDER_HELPER_RULE}")
        };
        let body = format!(
            "{output}{companion}\n\n{BUILDER_HELPER_RULE} if {{\n\tinput.facts.certification_rank <= 2\n}}\n"
        );
        let result = validate(&body);
        assert!(
            result.valid,
            "{}:\n{body}\n{:?}",
            spec.rule_name, result.diagnostics
        );
        assert!(
            result.warnings.is_empty(),
            "{}: {:?}",
            spec.rule_name,
            result.warnings
        );

        let scan = scan_request_action_heads(&source(&body), RULE_ID).expect("parses");
        assert!(
            scan.errors.is_empty(),
            "{}: {:?}",
            spec.rule_name,
            scan.errors
        );
        match spec.action {
            Some(head) => {
                let declared = scan.declared_actions();
                assert!(
                    declared.heads.contains(&head),
                    "{} not declared",
                    spec.rule_name
                );
                let mut declared = declared;
                match declared.slot_mut(head) {
                    Some(crate::request_actions::DeclaredSlot::Texts(values)) => {
                        let expected: Vec<String> =
                            spec.sample.iter().map(|item| item.to_string()).collect();
                        assert_eq!(*values, expected, "{}", spec.rule_name);
                    }
                    Some(crate::request_actions::DeclaredSlot::Numbers(values)) => {
                        assert_eq!(
                            *values,
                            vec![spec.sample[0].parse::<i64>().expect("number sample")],
                            "{}",
                            spec.rule_name
                        );
                    }
                    None => {}
                }
            }
            None => assert!(scan.defines_reasons, "{} not declared", spec.rule_name),
        }
    }
}

#[test]
fn the_output_rules_follow_the_validation_rules() {
    use crate::request_actions::request_builder_schema;
    use crate::rule_builder_schema::RuleBuilderOutputRuleKind;
    let schema = request_builder_schema();
    let rule = |name: RuleBuilderOutputRuleKind| {
        schema
            .output_rules
            .iter()
            .find(|rule| rule.rule == name)
            .unwrap_or_else(|| panic!("{name:?} rule"))
            .of
            .clone()
            .expect("the rule names its outputs")
    };
    assert_eq!(
        rule(RuleBuilderOutputRuleKind::AtMostOne),
        vec!["APPROVE", "DENY", "MANUAL"]
    );
    let deciding = rule(RuleBuilderOutputRuleKind::AtLeastOne);
    for output in &schema.outputs {
        let head = RequestActionHead::from_rule_name(output.head);
        let spec = crate::request_actions::head_spec(output.head).expect("table entry");
        let alone = format!(
            "{}\n\nalways := true\n",
            spec.fill_template("always", spec.sample)
        );
        let decides = validate(&alone).valid;
        assert_eq!(
            deciding.contains(&output.id),
            decides,
            "{} alone: {alone}",
            output.head
        );
        assert_eq!(head.is_none(), output.id == "REASONS");
        assert_eq!(
            output.label_key,
            format!("ruleOutput.request.{}", output.head)
        );
        assert_eq!(output.help_key, None, "{}", output.head);
    }
}

/// The characters of a `[...]+` pattern, for checking it against the engine.
fn pattern_class(pattern: &str) -> Vec<char> {
    let inner = pattern
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix("]+"))
        .expect("a single character class");
    let chars: Vec<char> = inner.chars().collect();
    let mut class = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if index + 2 < chars.len() && chars[index + 1] == '-' {
            class.extend(chars[index]..=chars[index + 2]);
            index += 3;
        } else {
            class.push(chars[index]);
            index += 1;
        }
    }
    class
}

#[test]
fn every_item_pattern_matches_the_engine_check() {
    use crate::request_actions::REQUEST_HEADS;
    for spec in &REQUEST_HEADS {
        let (Some(pattern), Some(rule)) = (spec.pattern, spec.item_rule) else {
            continue;
        };
        let class = pattern_class(pattern);
        for byte in 0x20u8..0x7f {
            let c = byte as char;
            // A leading letter keeps a check on the start of the item, such as
            // a reserved prefix, out of the comparison.
            let item = format!("a{c}");
            assert_eq!(
                rule.check(&item).is_ok(),
                class.contains(&c),
                "{}: {item:?} against {pattern}",
                spec.rule_name
            );
        }
        if let Some(max) = spec.max_bytes {
            assert!(rule.check(&"a".repeat(max)).is_ok(), "{}", spec.rule_name);
            assert!(
                rule.check(&"a".repeat(max + 1)).is_err(),
                "{}",
                spec.rule_name
            );
        }
    }
}

#[test]
fn the_builder_schema_copies_are_byte_identical() {
    assert_eq!(
        include_str!("../request-builder-schema.json"),
        include_str!("../../../apps/scryer-web/lib/contracts/request-builder-schema.json"),
        "crates/scryer-rules/request-builder-schema.json and \
         apps/scryer-web/lib/contracts/request-builder-schema.json must stay byte-identical"
    );
}

#[test]
fn the_title_builder_schema_copies_are_byte_identical() {
    assert_eq!(
        include_str!("../lifecycle-builder-schema.json"),
        include_str!("../../../apps/scryer-web/lib/contracts/lifecycle-builder-schema.json"),
        "crates/scryer-rules/lifecycle-builder-schema.json and \
         apps/scryer-web/lib/contracts/lifecycle-builder-schema.json must stay byte-identical"
    );
}
