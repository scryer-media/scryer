use super::*;
use crate::lifecycle_rules::service::{prepare_source, require_max_titles_per_run};
use scryer_rules::lifecycle::{
    LifecycleInput, LifecycleOutcome, LifecyclePolicy, LifecycleRulesEngine, LifecycleSubjectKind,
    LifecycleWatcherDoc, rewrite_package_declaration, synthetic_lifecycle_input,
    synthetic_title_input,
};
use scryer_rules::lifecycle_actions::LifecycleActions;
use scryer_rules::policy::observation::Observation;
use serde_json::{Value, json};
use std::collections::BTreeSet;

// The fixtures keep the exact structure Maintainerr serves, with synthetic
// names. The catalogue is Maintainerr's own property list.
const LIVE_RULES: &str = include_str!("fixtures/live-shape-rules.json");
const LIVE_CONSTANTS: &str = include_str!("fixtures/live-shape-constants.json");
const LIVE_RADARR_PROFILES: &str = include_str!("fixtures/live-shape-radarr-profiles.json");
const LIVE_SONARR_PROFILES: &str = include_str!("fixtures/live-shape-sonarr-profiles.json");
const WEB_RULE_SETS: &str =
    include_str!("../../../../../apps/scryer-web/lib/utils/lifecycle-rule-sets.ts");

// Application and property ids in the fixture catalogue.
const RADARR: i64 = 1;
const SONARR: i64 = 2;
const JELLYFIN: i64 = 6;
const TAUTULLI: i64 = 40;
const SEERR: i64 = 41;

const RADARR_ADD_DATE: (i64, i64) = (RADARR, 0);
const RADARR_TAGS: (i64, i64) = (RADARR, 2);
const RADARR_PROFILE: (i64, i64) = (RADARR, 3);
const RADARR_MONITORED: (i64, i64) = (RADARR, 5);
const RADARR_IN_CINEMAS: (i64, i64) = (RADARR, 6);
const RADARR_FILE_SIZE: (i64, i64) = (RADARR, 7);
const SONARR_MONITORED_NUMBER: (i64, i64) = (SONARR, 8);
const SONARR_MONITORED: (i64, i64) = (SONARR, 9);
const SONARR_SEASON_NUMBER: (i64, i64) = (SONARR, 18);
const JELLYFIN_SEEN_BY: (i64, i64) = (JELLYFIN, 1);
const JELLYFIN_VIEW_COUNT: (i64, i64) = (JELLYFIN, 5);
const JELLYFIN_COLLECTIONS: (i64, i64) = (JELLYFIN, 6);
const TAUTULLI_VIEWS_BY_USER: (i64, i64) = (TAUTULLI, 0);
const TAUTULLI_LAST_VIEWED_BY_USER: (i64, i64) = (TAUTULLI, 1);
const SEERR_ADD_USER: (i64, i64) = (SEERR, 0);

const MIB: i64 = 1_048_576;
/// The synthetic input's evaluation time.
const NOW: i64 = 1_700_000_000;
const DAY: i64 = 86_400;

// Maintainerr's comparator and action numbers, as the wire carries them.
const BIGGER: i64 = 0;
const SMALLER: i64 = 1;
const EQUALS: i64 = 2;
const NOT_EQUALS: i64 = 3;
const CONTAINS: i64 = 4;
const BEFORE: i64 = 5;
const AFTER: i64 = 6;
const IN_LAST: i64 = 7;
const IN_NEXT: i64 = 8;
const NOT_CONTAINS: i64 = 9;
const CONTAINS_PARTIAL: i64 = 10;
const NOT_CONTAINS_PARTIAL: i64 = 11;
const CONTAINS_ALL: i64 = 12;
const NOT_CONTAINS_ALL: i64 = 13;
const COUNT_EQUALS: i64 = 14;
const COUNT_NOT_EQUALS: i64 = 15;
const COUNT_BIGGER: i64 = 16;
const COUNT_SMALLER: i64 = 17;
const EXISTS: i64 = 18;
const NOT_EXISTS: i64 = 19;
const AND: Option<i64> = Some(0);
const OR: Option<i64> = Some(1);

// ── Builders ───────────────────────────────────────────────────────────────

/// What a rule compares its first property with.
#[derive(Clone, Copy)]
enum With<'a> {
    Custom(i64, &'a str),
    Last((i64, i64)),
    Nothing,
}

const fn number(value: &str) -> With<'_> {
    With::Custom(0, value)
}
const fn date(value: &str) -> With<'_> {
    With::Custom(1, value)
}
const fn text(value: &str) -> With<'_> {
    With::Custom(2, value)
}
const fn boolean(value: &str) -> With<'_> {
    With::Custom(3, value)
}

#[derive(Clone)]
struct Rule {
    operator: Option<i64>,
    action: i64,
    first: (i64, i64),
    with: With<'static>,
    section: i64,
    username: Option<&'static str>,
}

fn rule(operator: Option<i64>, first: (i64, i64), action: i64, with: With<'static>) -> Rule {
    Rule {
        operator,
        action,
        first,
        with,
        section: 0,
        username: None,
    }
}

impl Rule {
    fn section(mut self, section: i64) -> Self {
        self.section = section;
        self
    }

    fn user(mut self, username: &'static str) -> Self {
        self.username = Some(username);
        self
    }

    fn dto(&self) -> Value {
        let mut dto = json!({
            "operator": self.operator,
            "action": self.action,
            "firstVal": [self.first.0, self.first.1],
            "section": self.section,
        });
        match self.with {
            With::Custom(rule_type_id, value) => {
                dto["customVal"] = json!({ "ruleTypeId": rule_type_id, "value": value });
            }
            With::Last((app, prop)) => dto["lastVal"] = json!([app, prop]),
            With::Nothing => {}
        }
        if let Some(username) = self.username {
            dto["username"] = json!(username);
        }
        dto
    }
}

/// A rule row as `GET /api/rules` serves it.
fn row(id: i64, group_id: i64, rule: &Rule) -> Value {
    json!({
        "id": id,
        "ruleJson": rule.dto().to_string(),
        "ruleGroupId": group_id,
        "section": rule.section,
        "isActive": true,
    })
}

fn group(id: i64, data_type: &str, arr_action: i64, rules: &[Rule]) -> Value {
    let rows: Vec<Value> = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| row(id * 100 + index as i64, id, rule))
        .collect();
    json!({
        "id": id,
        "name": format!("Sample Rule {id}"),
        "description": "Sample description",
        "libraryId": "sample-library",
        "isActive": true,
        "useRules": true,
        "dataType": data_type,
        "rules": rows,
        "collection": {
            "id": id,
            "title": format!("Sample Rule {id}"),
            "arrAction": arr_action,
            "deleteAfterDays": 30,
            "type": data_type,
            "radarrSettingsId": 1,
            "sonarrSettingsId": 1,
            "radarrQualityProfileId": 4,
            "sonarrQualityProfileId": 4,
            "manualCollection": false,
        },
    })
}

/// The fixture catalogue plus two applications a bare instance does not list.
fn catalogue() -> String {
    let mut constants: Value = serde_json::from_str(LIVE_CONSTANTS).expect("fixture parses");
    let applications = constants["applications"]
        .as_array_mut()
        .expect("fixture lists applications");
    applications.push(json!({
        "id": TAUTULLI,
        "name": "Tautulli",
        "props": [
            { "id": 0, "name": "viewCountByUser", "type": { "key": "0" } },
            { "id": 1, "name": "lastViewedAtByUser", "type": { "key": "1" } },
        ],
    }));
    applications.push(json!({
        "id": SEERR,
        "name": "Seerr",
        "props": [{ "id": 0, "name": "addUser", "type": { "key": "2" } }],
    }));
    constants.to_string()
}

fn context() -> MaintainerrTranslateContext {
    let mut ctx = MaintainerrTranslateContext::default();
    ctx.add_arr_profiles(ArrKind::Radarr, 1, LIVE_RADARR_PROFILES)
        .expect("radarr profiles parse");
    ctx.add_arr_profiles(ArrKind::Sonarr, 1, LIVE_SONARR_PROFILES)
        .expect("sonarr profiles parse");
    ctx.scryer_quality_profile_names = ["HD-1080p".to_string(), "SD".to_string()].into();
    ctx.scryer_usernames = ["sample-user".to_string()].into();
    ctx
}

fn translate_with(
    groups: &[Value],
    ctx: &MaintainerrTranslateContext,
) -> Vec<MaintainerrTranslatedRule> {
    let rules = translate_maintainerr_rule_groups(
        &Value::from(groups.to_vec()).to_string(),
        &catalogue(),
        ctx,
    )
    .expect("the export is readable");
    for rule in &rules {
        for diagnostic in &rule.diagnostics {
            assert!(codes::ALL.contains(&diagnostic.code), "{}", diagnostic.code);
        }
        assert_saves(rule);
    }
    rules
}

fn translate_one(data_type: &str, arr_action: i64, rules: &[Rule]) -> MaintainerrTranslatedRule {
    translate_with(&[group(1, data_type, arr_action, rules)], &context())
        .pop()
        .expect("one group in, one rule out")
}

fn translated(data_type: &str, arr_action: i64, rules: &[Rule]) -> MaintainerrTranslatedRule {
    let rule = translate_one(data_type, arr_action, rules);
    assert_eq!(
        rule.status,
        MaintainerrRuleStatus::Translated,
        "{:?}\n{}",
        rule.diagnostics,
        rule.rego_source
    );
    rule
}

fn errors(rule: &MaintainerrTranslatedRule) -> Vec<&'static str> {
    rule.diagnostics
        .iter()
        .filter(|d| d.severity == MaintainerrDiagnosticSeverity::Error)
        .map(|d| d.code)
        .collect()
}

fn warnings(rule: &MaintainerrTranslatedRule) -> Vec<&'static str> {
    rule.diagnostics
        .iter()
        .filter(|d| d.severity == MaintainerrDiagnosticSeverity::Warning)
        .map(|d| d.code)
        .collect()
}

fn failed_with(data_type: &str, arr_action: i64, rules: &[Rule]) -> Vec<&'static str> {
    let rule = translate_one(data_type, arr_action, rules);
    assert_eq!(
        rule.status,
        MaintainerrRuleStatus::Failed,
        "{}",
        rule.rego_source
    );
    assert_eq!(rule.rego_source, LIFECYCLE_STARTER_SOURCE);
    assert!(
        rule.name.starts_with("(Maintainerr) FAILED - "),
        "{}",
        rule.name
    );
    assert_eq!(rule.max_titles_per_run, None);
    assert!(!rule.reads_person_targeted_fact);
    errors(&rule)
}

/// Every emitted source must save exactly as an operator's would.
fn assert_saves(rule: &MaintainerrTranslatedRule) {
    let prepared = match prepare_source("maintainerr_test", &rule.rego_source, rule.grace_days) {
        Ok(prepared) => prepared,
        Err(error) => panic!(
            "{} does not save: {error:?}\n{}",
            rule.name, rule.rego_source
        ),
    };
    if let Err(error) = require_max_titles_per_run(&prepared.declared, rule.max_titles_per_run) {
        panic!("{} does not save: {error:?}", rule.name);
    }
}

// ── Evaluation ─────────────────────────────────────────────────────────────

fn decide(source: &str, input: &LifecycleInput) -> (LifecycleOutcome, LifecycleActions) {
    let id = "maintainerr_eval";
    let engine = LifecycleRulesEngine::build(&[LifecyclePolicy {
        id: id.to_string(),
        name: id.to_string(),
        rego_source: rewrite_package_declaration(source, id),
    }])
    .expect("the source compiles");
    let result = engine
        .evaluator()
        .evaluate(input)
        .expect("the source evaluates");
    assert!(result.errors.is_empty(), "{:?}\n{source}", result.errors);
    let decision = result
        .records
        .into_iter()
        .next()
        .expect("one record")
        .decision;
    (decision.outcome, decision.actions)
}

fn fires(source: &str, input: &LifecycleInput) -> bool {
    match decide(source, input).0 {
        LifecycleOutcome::Match => true,
        LifecycleOutcome::NoMatch => false,
        LifecycleOutcome::Unknown => panic!("the rule was held\n{source}"),
    }
}

fn rfc3339(seconds: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .expect("in range")
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn movie() -> LifecycleInput {
    synthetic_title_input()
}

fn show() -> LifecycleInput {
    let mut input = synthetic_title_input();
    input.subject.facet = "series".to_string();
    input
}

fn season() -> LifecycleInput {
    let mut input = synthetic_lifecycle_input();
    input.subject.facet = "series".to_string();
    input.subject.kind = LifecycleSubjectKind::Season;
    input.subject.season_number = Some(1);
    input.episode = None;
    input
}

fn episode() -> LifecycleInput {
    let mut input = synthetic_lifecycle_input();
    input.subject.facet = "series".to_string();
    input.subject.kind = LifecycleSubjectKind::Episode;
    input.subject.season_number = Some(1);
    input.season = None;
    input
}

// ── Codes and constants ────────────────────────────────────────────────────

#[test]
fn diagnostic_codes_are_distinct_and_namespaced() {
    let distinct: BTreeSet<_> = codes::ALL.iter().collect();
    assert_eq!(distinct.len(), codes::ALL.len(), "a code is listed twice");
    for code in codes::ALL {
        assert!(code.starts_with("maintainerr."), "{code}");
        assert!(
            code.chars()
                .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
            "{code}"
        );
    }
}

#[test]
fn failed_rules_carry_the_web_editors_starter_source() {
    let marker = "export const LIFECYCLE_STARTER_SOURCE = `";
    let start = WEB_RULE_SETS
        .find(marker)
        .expect("the web defines the starter")
        + marker.len();
    let rest = &WEB_RULE_SETS[start..];
    let bytes = rest.as_bytes();
    let end = (0..bytes.len())
        .find(|&i| bytes[i] == b'`' && (i == 0 || bytes[i - 1] != b'\\'))
        .expect("the starter literal ends");
    let web = rest[..end].replace("\\`", "`");
    assert_eq!(LIFECYCLE_STARTER_SOURCE, web);
}

#[test]
fn refusals_carry_their_reason() {
    let ctx = context();
    let constants = catalogue();
    let refuse = |groups: &str, constants: &str| {
        translate_maintainerr_rule_groups(groups, constants, &ctx).expect_err("refused")
    };
    assert_eq!(
        refuse("not json", &constants),
        MaintainerrImportRefusal::GroupsJsonInvalid
    );
    assert_eq!(
        refuse("{}", &constants),
        MaintainerrImportRefusal::GroupsJsonInvalid
    );
    assert_eq!(
        refuse("[1]", &constants),
        MaintainerrImportRefusal::GroupsJsonInvalid
    );
    assert_eq!(
        refuse("[{}]", &constants),
        MaintainerrImportRefusal::GroupsJsonInvalid
    );
    assert_eq!(
        refuse("[]", "[]"),
        MaintainerrImportRefusal::ConstantsJsonInvalid
    );
    assert_eq!(
        refuse("[]", r#"{"applications":[{"id":1}]}"#),
        MaintainerrImportRefusal::ConstantsJsonInvalid
    );
    let many: Vec<Value> = (0..=MAX_MAINTAINERR_RULE_GROUPS as i64)
        .map(|id| json!({ "id": id }))
        .collect();
    assert_eq!(
        refuse(&Value::from(many).to_string(), &constants),
        MaintainerrImportRefusal::GroupsTooMany
    );
    let huge = " ".repeat(MAX_MAINTAINERR_JSON_BYTES + 1);
    assert_eq!(
        refuse(&huge, &constants),
        MaintainerrImportRefusal::JsonTooLarge
    );

    let mut ctx = MaintainerrTranslateContext::default();
    assert_eq!(
        ctx.add_arr_profiles(ArrKind::Radarr, 1, r#"[{"id":"x"}]"#),
        Err(MaintainerrImportRefusal::ProfilesJsonInvalid)
    );
    for refusal in [
        MaintainerrImportRefusal::JsonTooLarge,
        MaintainerrImportRefusal::GroupsJsonInvalid,
        MaintainerrImportRefusal::ConstantsJsonInvalid,
        MaintainerrImportRefusal::GroupsTooMany,
        MaintainerrImportRefusal::ProfilesJsonInvalid,
    ] {
        assert!(crate::error_reasons::maintainerr_import::ALL.contains(&refusal.reason()));
        assert_eq!(
            refusal.into_app_error().validation_reason(),
            Some(refusal.reason())
        );
    }
}

// ── The live shape ─────────────────────────────────────────────────────────

#[test]
fn the_live_shape_translates_group_by_group() {
    let rules = translate_maintainerr_rule_groups(LIVE_RULES, LIVE_CONSTANTS, &context())
        .expect("the fixture is readable");
    for rule in &rules {
        assert_saves(rule);
    }
    let summary: Vec<(i64, MaintainerrRuleStatus, Vec<&str>)> = rules
        .iter()
        .map(|rule| (rule.source_group_id, rule.status, errors(rule)))
        .collect();
    use MaintainerrRuleStatus::{Failed, Translated};
    assert_eq!(
        summary,
        vec![
            (1, Translated, vec![]),
            (2, Translated, vec![]),
            (3, Translated, vec![]),
            (4, Translated, vec![]),
            (5, Failed, vec![codes::PROPERTY_UNMAPPED]),
            (
                6,
                Failed,
                vec![
                    codes::PROPERTY_COLLECTION_UNSUPPORTED,
                    codes::PROPERTY_COLLECTION_UNSUPPORTED
                ]
            ),
            (7, Translated, vec![]),
            (8, Failed, vec![codes::PROPERTY_UNMAPPED]),
            (
                9,
                Failed,
                vec![codes::PROPERTY_UNMAPPED, codes::PROPERTY_UNMAPPED]
            ),
            (10, Failed, vec![codes::ACTION_UNSUPPORTED]),
        ]
    );
    assert_eq!(rules[0].name, "(Maintainerr) Sample Rule Alpha");
    assert_eq!(rules[0].grace_days, 30);
    assert_eq!(rules[0].max_titles_per_run, Some(10));
    assert_eq!(warnings(&rules[0]), vec![codes::FACT_APPROXIMATE]);
    assert!(!rules[0].source_is_active);
    assert_eq!(
        rules[0].source_library_id.as_deref(),
        Some("sample-library-1")
    );
    assert_eq!(rules[2].max_titles_per_run, None);
    assert_eq!(warnings(&rules[3]), vec![codes::ACTION_APPROXIMATE]);
    assert!(
        rules[4]
            .description
            .starts_with("Could not translate from Maintainerr: ")
    );
    assert!(
        !rules[1]
            .description
            .starts_with("Could not translate from Maintainerr: ")
    );
    assert!(
        rules[1]
            .description
            .ends_with("\n\nSample description Bravo")
    );

    // Unplayed and imported over ninety days ago.
    let mut input = movie();
    input.facts.play_count_total = Observation::known(0);
    input.facts.first_imported_at = Observation::known(rfc3339(NOW - 120 * DAY));
    assert!(fires(&rules[0].rego_source, &input));
    input.facts.first_imported_at = Observation::known(rfc3339(NOW - 30 * DAY));
    assert!(!fires(&rules[0].rego_source, &input));
    input.facts.first_imported_at = Observation::known(rfc3339(NOW - 120 * DAY));
    input.facts.play_count_total = Observation::known(2);
    assert!(!fires(&rules[0].rego_source, &input));

    // Over 20000 MiB and unmonitored.
    let mut input = movie();
    input.facts.total_file_size_bytes = Observation::known(30_000 * MIB);
    input.facts.monitored = Observation::known(false);
    assert!(fires(&rules[2].rego_source, &input));
    input.facts.monitored = Observation::known(true);
    assert!(!fires(&rules[2].rego_source, &input));

    // Tagged keep: a notification, nothing else.
    let mut input = movie();
    input.facts.tags = Observation::known(vec!["Keep".to_string()]);
    let (outcome, actions) = decide(&rules[3].rego_source, &input);
    assert_eq!(outcome, LifecycleOutcome::Match);
    assert_eq!(
        actions.notify.as_deref(),
        Some("Matched the Maintainerr rule group \"Sample Rule Delta is \"quoted\"\".")
    );
    assert!(!actions.defines_destructive_action());
}

/// The real capture, read from a directory outside the repository. Run with
/// `SCRYER_MAINTAINERR_CAPTURE_DIR=<dir>` and `--run-ignored only`.
#[test]
#[ignore = "needs a local Maintainerr capture"]
fn a_local_capture_translates_and_every_source_saves() {
    let dir = std::env::var("SCRYER_MAINTAINERR_CAPTURE_DIR").expect("capture dir is set");
    let read = |name: &str| std::fs::read_to_string(format!("{dir}/{name}")).expect(name);
    let mut ctx = MaintainerrTranslateContext::default();
    ctx.add_arr_profiles(ArrKind::Radarr, 1, &read("radarr-1-profiles.json"))
        .expect("radarr profiles");
    ctx.add_arr_profiles(ArrKind::Sonarr, 1, &read("sonarr-1-profiles.json"))
        .expect("sonarr profiles");
    ctx.scryer_quality_profile_names = ctx.arr_profile_names.values().cloned().collect();
    let users: Vec<String> = serde_json::from_str(&read("users.json")).unwrap_or_default();
    ctx.scryer_usernames = users.into_iter().collect();
    let rules =
        translate_maintainerr_rule_groups(&read("rules.json"), &read("constants.json"), &ctx)
            .expect("the capture is readable");
    for rule in &rules {
        assert_saves(rule);
        let codes: Vec<String> = rule
            .diagnostics
            .iter()
            .map(|d| format!("{:?} {} {}", d.severity, d.code, d.message))
            .collect();
        eprintln!(
            "group {} {:?}: {codes:#?}",
            rule.source_group_id, rule.status
        );
        if rule.status == MaintainerrRuleStatus::Translated {
            eprintln!("{}", rule.rego_source);
        }
    }
}

// ── Actions ────────────────────────────────────────────────────────────────

#[test]
fn every_action_maps_to_existing_heads_at_every_level() {
    let mut ctx = context();
    ctx.scryer_quality_profile_names
        .insert("HD-1080p".to_string());
    // (level, action, heads, destructive); no heads means FAILED.
    let cases: &[(&str, i64, &[&str], bool)] = &[
        ("movie", 0, &["delete_title"], true),
        ("movie", 1, &["unmonitor", "delete_files"], true),
        ("movie", 2, &["delete_title"], true),
        ("movie", 3, &["unmonitor"], false),
        ("movie", 4, &["notify :="], false),
        ("movie", 5, &[], false),
        ("movie", 6, &[], false),
        (
            "movie",
            7,
            &["set_quality_profile := \"HD-1080p\"", "search"],
            false,
        ),
        ("show", 0, &["delete_title"], true),
        ("show", 1, &["unmonitor", "delete_files"], true),
        ("show", 2, &["unmonitor", "delete_files"], true),
        ("show", 3, &["unmonitor"], false),
        ("show", 4, &["notify :="], false),
        ("show", 5, &[], false),
        ("show", 6, &[], false),
        (
            "show",
            7,
            &["set_quality_profile := \"HD-1080p\"", "search"],
            false,
        ),
        (
            "season",
            0,
            &["unmonitor_season", "delete_season_files"],
            true,
        ),
        ("season", 1, &[], false),
        (
            "season",
            2,
            &["unmonitor_season", "delete_season_files"],
            true,
        ),
        ("season", 3, &["unmonitor_season"], false),
        ("season", 4, &["notify_season :="], false),
        ("season", 5, &[], false),
        ("season", 6, &[], false),
        ("season", 7, &[], false),
        (
            "episode",
            0,
            &["unmonitor_episode", "delete_episode_files"],
            true,
        ),
        ("episode", 1, &[], false),
        ("episode", 2, &[], false),
        ("episode", 3, &["unmonitor_episode"], false),
        ("episode", 4, &["notify_episode :="], false),
        ("episode", 5, &[], false),
        ("episode", 6, &[], false),
        ("episode", 7, &[], false),
        ("movie", 8, &[], false),
    ];
    for &(level, action, heads, destructive) in cases {
        let monitored = if level == "movie" {
            RADARR_MONITORED
        } else {
            SONARR_MONITORED
        };
        let rules = [rule(None, monitored, EQUALS, boolean("1"))];
        let result = translate_with(&[group(1, level, action, &rules)], &ctx)
            .pop()
            .expect("one rule");
        if heads.is_empty() {
            assert_eq!(
                result.status,
                MaintainerrRuleStatus::Failed,
                "{level} {action}"
            );
            assert_eq!(
                errors(&result),
                vec![codes::ACTION_UNSUPPORTED],
                "{level} {action}"
            );
            continue;
        }
        assert_eq!(
            result.status,
            MaintainerrRuleStatus::Translated,
            "{level} {action}"
        );
        for head in heads {
            assert!(
                result.rego_source.contains(&format!("\n{head} ")),
                "{level} {action} lacks {head}\n{}",
                result.rego_source
            );
        }
        assert_eq!(
            result.max_titles_per_run,
            destructive.then_some(MAINTAINERR_DESTRUCTIVE_MAX_TITLES_PER_RUN),
            "{level} {action}"
        );
        let input = match level {
            "movie" => movie(),
            "show" => show(),
            "season" => season(),
            _ => episode(),
        };
        let (outcome, actions) = decide(&result.rego_source, &input);
        assert_eq!(outcome, LifecycleOutcome::Match, "{level} {action}");
        if action == 7 {
            assert_eq!(actions.set_quality_profile.as_deref(), Some("HD-1080p"));
            assert!(actions.search);
        }
        // A rule never fires on media of another kind.
        let other = if level == "movie" { show() } else { movie() };
        if matches!(level, "movie" | "show") {
            assert!(!fires(&result.rego_source, &other), "{level} {action}");
        }
    }
}

#[test]
fn quality_profiles_resolve_by_name_or_fail() {
    let rules = [rule(None, RADARR_MONITORED, EQUALS, boolean("1"))];
    let mut unknown_id = group(1, "movie", 7, &rules);
    unknown_id["collection"]["radarrQualityProfileId"] = json!(99);
    let mut no_id = group(2, "movie", 7, &rules);
    no_id["collection"]["radarrQualityProfileId"] = Value::Null;
    let mut not_in_scryer = group(3, "movie", 7, &rules);
    not_in_scryer["collection"]["radarrQualityProfileId"] = json!(5);
    let mut sonarr = group(
        4,
        "show",
        7,
        &[rule(None, SONARR_MONITORED, EQUALS, boolean("1"))],
    );
    sonarr["collection"]["sonarrQualityProfileId"] = json!(2);
    let results = translate_with(&[unknown_id, no_id, not_in_scryer, sonarr], &context());
    assert_eq!(errors(&results[0]), vec![codes::QUALITY_PROFILE_UNRESOLVED]);
    assert_eq!(errors(&results[1]), vec![codes::QUALITY_PROFILE_UNRESOLVED]);
    assert_eq!(
        errors(&results[2]),
        vec![codes::QUALITY_PROFILE_NOT_IN_SCRYER]
    );
    assert_eq!(results[3].status, MaintainerrRuleStatus::Translated);
    assert!(
        results[3]
            .rego_source
            .contains("set_quality_profile := \"SD\" if")
    );
}

#[test]
fn settings_on_the_group_are_read_when_the_collection_has_none() {
    let mut value = group(
        1,
        "movie",
        3,
        &[rule(None, RADARR_MONITORED, EQUALS, boolean("1"))],
    );
    value["collection"]["arrAction"] = Value::Null;
    value["collection"]["deleteAfterDays"] = Value::Null;
    value["arrAction"] = json!(0);
    value["deleteAfterDays"] = json!(12);
    let result = translate_with(&[value], &context())
        .pop()
        .expect("one rule");
    assert!(result.rego_source.contains("\ndelete_title if"));
    assert_eq!(result.grace_days, 12);

    let mut both = group(
        1,
        "movie",
        3,
        &[rule(None, RADARR_MONITORED, EQUALS, boolean("1"))],
    );
    both["arrAction"] = json!(0);
    let result = translate_with(&[both], &context()).pop().expect("one rule");
    assert!(
        result.rego_source.contains("\nunmonitor if"),
        "the collection wins"
    );
}

#[test]
fn grace_days_clamp_and_only_deletions_carry_a_run_limit() {
    for (days, expected) in [
        (json!(null), 0),
        (json!(0), 0),
        (json!(12.9), 12),
        (json!("45"), 45),
        (json!(-3), 0),
        (json!(4000), 3650),
    ] {
        let mut value = group(
            1,
            "movie",
            0,
            &[rule(None, RADARR_MONITORED, EQUALS, boolean("1"))],
        );
        value["collection"]["deleteAfterDays"] = days.clone();
        let result = translate_with(&[value.clone()], &context())
            .pop()
            .expect("one rule");
        assert_eq!(result.grace_days, expected, "{days}");
        assert_eq!(result.max_titles_per_run, Some(10));

        // A failed group keeps its grace period.
        value["useRules"] = json!(false);
        let result = translate_with(&[value], &context())
            .pop()
            .expect("one rule");
        assert_eq!(result.status, MaintainerrRuleStatus::Failed);
        assert_eq!(result.grace_days, expected, "{days}");
    }
    let unmonitor = translated(
        "movie",
        3,
        &[rule(None, RADARR_MONITORED, EQUALS, boolean("1"))],
    );
    assert_eq!(unmonitor.max_titles_per_run, None);
}

// ── Folding ────────────────────────────────────────────────────────────────

/// A: monitored, B: tagged "alpha", C: larger than 100 MiB.
fn abc_input(a: bool, b: bool, c: bool) -> LifecycleInput {
    let mut input = movie();
    input.facts.monitored = Observation::known(a);
    input.facts.tags = Observation::known(if b { vec!["alpha".to_string()] } else { vec![] });
    input.facts.total_file_size_bytes = Observation::known(if c { 200 * MIB } else { 50 * MIB });
    input
}

fn a(operator: Option<i64>) -> Rule {
    rule(operator, RADARR_MONITORED, EQUALS, boolean("1"))
}
fn b(operator: Option<i64>) -> Rule {
    rule(operator, RADARR_TAGS, CONTAINS, text("alpha"))
}
fn c(operator: Option<i64>) -> Rule {
    rule(operator, RADARR_FILE_SIZE, BIGGER, number("100"))
}

fn assert_truth_table(rules: &[Rule], expected: impl Fn(bool, bool, bool) -> bool) {
    let result = translated("movie", 3, rules);
    for bits in 0..8 {
        let (x, y, z) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
        assert_eq!(
            fires(&result.rego_source, &abc_input(x, y, z)),
            expected(x, y, z),
            "A={x} B={y} C={z}\n{}",
            result.rego_source
        );
    }
}

#[test]
fn rules_fold_left_to_right_within_a_section() {
    assert_truth_table(&[a(None), b(OR), c(AND)], |a, b, c| (a || b) && c);
    assert_truth_table(&[a(None), b(AND), c(OR)], |a, b, c| (a && b) || c);
    assert_truth_table(&[a(None), b(AND), c(AND)], |a, b, c| a && b && c);
    assert_truth_table(&[a(None), b(OR), c(OR)], |a, b, c| a || b || c);
    // The first rule's operator is ignored.
    assert_truth_table(&[a(AND), b(OR)], |a, b, _| a || b);
    // Each AND after an OR narrows everything matched so far.
    assert_truth_table(&[a(None), b(OR), c(AND), a(OR), b(AND)], |a, b, c| {
        ((a || b) && c || a) && b
    });
}

#[test]
fn sections_join_by_their_first_rules_operator() {
    assert_truth_table(
        &[a(None), b(OR).section(1), c(AND).section(1)],
        |a, b, c| a || (b && c),
    );
    assert_truth_table(&[a(None), b(OR), c(AND).section(1)], |a, b, c| {
        (a || b) && c
    });
    assert_truth_table(
        &[a(None), b(AND).section(1), c(OR).section(2)],
        |a, b, c| (a && b) || c,
    );
    // A section opened without an operator is OR'ed on.
    assert_truth_table(&[a(None), b(None).section(1)], |a, b, _| a || b);
    // Rules that start in a later section behave as if it were the first.
    assert_truth_table(&[a(None).section(1), b(AND).section(1)], |a, b, _| a && b);
    // Within a later section, the rules fold as they do in the first.
    assert_truth_table(
        &[a(None), b(AND).section(1), c(OR).section(1)],
        |a, b, c| a && (b || c),
    );
}

#[test]
fn rules_run_in_row_id_order_and_inactive_rows_still_count() {
    // Served out of order: row 2 is "AND C", row 1 is "A".
    let mut value = group(1, "movie", 3, &[]);
    value["rules"] = json!([row(2, 1, &c(AND)), row(1, 1, &a(None))]);
    let result = translate_with(&[value], &context())
        .pop()
        .expect("one rule");
    assert!(fires(&result.rego_source, &abc_input(true, false, true)));
    assert!(!fires(&result.rego_source, &abc_input(true, false, false)));
    assert!(!fires(&result.rego_source, &abc_input(false, false, true)));

    // Maintainerr's evaluator does not read a rule row's isActive.
    let mut value = group(1, "movie", 3, &[a(None)]);
    value["rules"][0]["isActive"] = json!(false);
    let result = translate_with(&[value], &context())
        .pop()
        .expect("one rule");
    assert!(fires(&result.rego_source, &abc_input(true, false, false)));

    // A bare rule DTO, without the row around it, reads the same.
    let mut value = group(1, "movie", 3, &[]);
    value["rules"] = json!([a(None).dto()]);
    let result = translate_with(&[value], &context())
        .pop()
        .expect("one rule");
    assert!(fires(&result.rego_source, &abc_input(true, false, false)));
}

// ── Comparators ────────────────────────────────────────────────────────────

fn check(
    first: (i64, i64),
    action: i64,
    with: With<'static>,
    input: &LifecycleInput,
    expected: bool,
) {
    let result = translated("movie", 3, &[rule(None, first, action, with)]);
    assert_eq!(
        fires(&result.rego_source, input),
        expected,
        "action {action}\n{}",
        result.rego_source
    );
}

#[test]
fn number_comparators_match_maintainerr() {
    let mut input = movie();
    // 1500.57 MiB, which Maintainerr rounds to 1501.
    input.facts.total_file_size_bytes = Observation::known(1500 * MIB + 600_000);
    check(RADARR_FILE_SIZE, BIGGER, number("1500"), &input, true);
    check(RADARR_FILE_SIZE, BIGGER, number("1501"), &input, false);
    check(RADARR_FILE_SIZE, SMALLER, number("1501.5"), &input, true);
    check(RADARR_FILE_SIZE, EQUALS, number("1501"), &input, true);
    check(RADARR_FILE_SIZE, NOT_EQUALS, number("1501"), &input, false);
    check(RADARR_FILE_SIZE, NOT_EQUALS, number("7"), &input, true);
    check(RADARR_FILE_SIZE, EXISTS, With::Nothing, &input, true);
    check(RADARR_FILE_SIZE, NOT_EXISTS, With::Nothing, &input, false);
    input.facts.total_file_size_bytes = Observation::absent();
    check(RADARR_FILE_SIZE, EXISTS, With::Nothing, &input, false);
    check(RADARR_FILE_SIZE, NOT_EXISTS, With::Nothing, &input, true);
    // A missing value never matches, negated or not.
    check(RADARR_FILE_SIZE, NOT_EQUALS, number("7"), &input, false);
    check(RADARR_FILE_SIZE, SMALLER, number("7"), &input, false);
}

#[test]
fn boolean_comparators_match_maintainerr() {
    let mut input = movie();
    input.facts.monitored = Observation::known(false);
    check(RADARR_MONITORED, EQUALS, boolean("0"), &input, true);
    check(RADARR_MONITORED, EQUALS, boolean("1"), &input, false);
    check(RADARR_MONITORED, NOT_EQUALS, boolean("1"), &input, true);
    check(RADARR_MONITORED, EXISTS, With::Nothing, &input, true);
    input.facts.monitored = Observation::absent();
    check(RADARR_MONITORED, NOT_EQUALS, boolean("1"), &input, false);
    check(RADARR_MONITORED, NOT_EXISTS, With::Nothing, &input, true);
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, RADARR_MONITORED, EQUALS, boolean("2"))]
        ),
        vec![codes::COMPARATOR_UNSUPPORTED]
    );
}

#[test]
fn date_comparators_match_maintainerr() {
    let mut input = movie();
    input.facts.added_at = Observation::known(rfc3339(NOW - 60 * DAY));
    // A number of seconds on a date property counts from now.
    check(RADARR_ADD_DATE, BEFORE, number("2592000"), &input, true);
    check(RADARR_ADD_DATE, AFTER, number("2592000"), &input, false);
    check(RADARR_ADD_DATE, IN_LAST, number("7776000"), &input, true);
    check(RADARR_ADD_DATE, IN_LAST, number("2592000"), &input, false);
    check(RADARR_ADD_DATE, IN_NEXT, number("2592000"), &input, false);
    // Absolute dates: a day, an offset timestamp, epoch seconds.
    let cutoff = NOW - 30 * DAY;
    let day = chrono::DateTime::<chrono::Utc>::from_timestamp(cutoff, 0)
        .expect("in range")
        .format("%Y-%m-%d")
        .to_string();
    let day: &'static str = Box::leak(day.into_boxed_str());
    let epoch: &'static str = Box::leak(cutoff.to_string().into_boxed_str());
    check(RADARR_ADD_DATE, BEFORE, date(day), &input, true);
    check(RADARR_ADD_DATE, AFTER, date(day), &input, false);
    check(RADARR_ADD_DATE, BEFORE, date(epoch), &input, true);
    check(
        RADARR_ADD_DATE,
        BEFORE,
        date("2000-01-01T00:00:00+02:00"),
        &input,
        false,
    );
    check(
        RADARR_ADD_DATE,
        AFTER,
        date("2000-01-01T00:00:00+02:00"),
        &input,
        true,
    );
    // In the next thirty days.
    input.facts.added_at = Observation::known(rfc3339(NOW + 10 * DAY));
    check(RADARR_ADD_DATE, IN_NEXT, number("2592000"), &input, true);
    check(RADARR_ADD_DATE, IN_LAST, number("2592000"), &input, false);
    // A date-only fact is midnight UTC.
    input.facts.theatrical_release_date = Observation::known("2023-01-01".to_string());
    check(RADARR_IN_CINEMAS, AFTER, date("2022-12-31"), &input, true);
    check(RADARR_IN_CINEMAS, BEFORE, date("2023-01-01"), &input, true);
    check(RADARR_IN_CINEMAS, BEFORE, date("2022-12-31"), &input, false);
    input.facts.added_at = Observation::absent();
    check(RADARR_ADD_DATE, BEFORE, number("2592000"), &input, false);

    // Unfaithful date comparisons fail.
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, RADARR_ADD_DATE, EQUALS, date("2023-01-01"))]
        ),
        vec![codes::COMPARATOR_DATE_EQUALITY_UNSUPPORTED]
    );
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(
                None,
                RADARR_ADD_DATE,
                BEFORE,
                date("2023-01-01T00:00:00")
            )]
        ),
        vec![codes::CUSTOM_VALUE_INVALID]
    );
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, RADARR_ADD_DATE, CONTAINS, number("5"))]
        ),
        vec![codes::COMPARATOR_UNSUPPORTED]
    );
}

#[test]
fn list_comparators_match_maintainerr() {
    let mut input = movie();
    input.facts.tags = Observation::known(vec!["Alpha".to_string(), "beta-two".to_string()]);
    let cases: &[(i64, With<'static>, bool)] = &[
        (CONTAINS, text("ALPHA"), true),
        (CONTAINS, text("beta"), false),
        (CONTAINS, text(r#"["gamma","alpha"]"#), true),
        (NOT_CONTAINS, text("gamma"), true),
        (NOT_CONTAINS, text("alpha"), false),
        (CONTAINS_PARTIAL, text("eta-t"), true),
        (CONTAINS_PARTIAL, text(r#"["zz","lph"]"#), true),
        (NOT_CONTAINS_PARTIAL, text("eta"), false),
        (NOT_CONTAINS_PARTIAL, text("zeta"), true),
        (CONTAINS_ALL, text(r#"["alpha","beta-two"]"#), true),
        (CONTAINS_ALL, text(r#"["alpha","gamma"]"#), false),
        (NOT_CONTAINS_ALL, text(r#"["alpha","gamma"]"#), true),
        (EQUALS, With::Custom(4, r#"["beta-two","alpha"]"#), true),
        (EQUALS, text(r#"["alpha"]"#), false),
        (NOT_EQUALS, text(r#"["alpha"]"#), true),
        (COUNT_EQUALS, number("2"), true),
        (COUNT_NOT_EQUALS, number("2"), false),
        (COUNT_BIGGER, number("1"), true),
        (COUNT_SMALLER, number("2"), false),
        (COUNT_EQUALS, text(r#"["x","y"]"#), true),
        (EXISTS, With::Nothing, true),
        (NOT_EXISTS, With::Nothing, false),
    ];
    for &(action, with, expected) in cases {
        check(RADARR_TAGS, action, with, &input, expected);
    }
    input.facts.tags = Observation::known(Vec::new());
    check(RADARR_TAGS, EXISTS, With::Nothing, &input, false);
    check(RADARR_TAGS, NOT_EXISTS, With::Nothing, &input, true);
    check(RADARR_TAGS, COUNT_EQUALS, number("0"), &input, true);
    check(RADARR_TAGS, CONTAINS, text(r#"["alpha"]"#), &input, false);

    for (action, with) in [
        (BIGGER, number("1")),
        (BEFORE, number("1")),
        (CONTAINS_PARTIAL, text(r#"["", "x"]"#)),
    ] {
        assert_eq!(
            failed_with("movie", 3, &[rule(None, RADARR_TAGS, action, with)]),
            vec![codes::COMPARATOR_UNSUPPORTED],
            "action {action}"
        );
    }
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, RADARR_TAGS, CONTAINS, text("[1, 2]"))]
        ),
        vec![codes::CUSTOM_VALUE_INVALID]
    );
}

#[test]
fn a_second_property_compares_like_a_custom_value() {
    let mut input = movie();
    input.facts.added_at = Observation::known("2022-12-01T00:00:00Z".to_string());
    input.facts.theatrical_release_date = Observation::known("2023-01-01".to_string());
    let earlier = rule(None, RADARR_ADD_DATE, BEFORE, With::Last(RADARR_IN_CINEMAS));
    let later = rule(None, RADARR_ADD_DATE, AFTER, With::Last(RADARR_IN_CINEMAS));
    let earlier = translated("movie", 3, &[earlier]);
    let later = translated("movie", 3, &[later]);
    assert!(fires(&earlier.rego_source, &input));
    assert!(!fires(&later.rego_source, &input));
    input.facts.theatrical_release_date = Observation::absent();
    assert!(!fires(&earlier.rego_source, &input));
    assert!(!fires(&later.rego_source, &input));

    let mut input = movie();
    input.facts.total_file_size_bytes = Observation::known(10 * MIB);
    let same = translated(
        "movie",
        3,
        &[rule(
            None,
            RADARR_FILE_SIZE,
            EQUALS,
            With::Last(RADARR_FILE_SIZE),
        )],
    );
    assert!(fires(&same.rego_source, &input));

    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(
                None,
                RADARR_TAGS,
                CONTAINS,
                With::Last(RADARR_FILE_SIZE)
            )]
        ),
        vec![codes::COMPARATOR_UNSUPPORTED]
    );
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, RADARR_TAGS, CONTAINS, With::Nothing)]
        ),
        vec![codes::RULE_SECOND_VALUE_MISSING]
    );
}

// ── Users ──────────────────────────────────────────────────────────────────

fn watcher(username: &str, last_played_at: Option<i64>, play_count: i64) -> LifecycleWatcherDoc {
    LifecycleWatcherDoc {
        user_id: format!("{username}-id"),
        username: username.to_string(),
        last_played_at: last_played_at.map(rfc3339),
        play_count,
    }
}

#[test]
fn usernames_bind_and_unknown_ones_only_warn() {
    let known = translated(
        "movie",
        3,
        &[rule(None, JELLYFIN_SEEN_BY, CONTAINS, text("Sample-User"))],
    );
    assert_eq!(warnings(&known), vec![codes::FACT_APPROXIMATE]);
    assert!(known.reads_person_targeted_fact);
    let mut input = movie();
    input.facts.watched_by_usernames = Observation::known(vec!["sample-user".to_string()]);
    assert!(fires(&known.rego_source, &input));
    input.facts.watched_by_usernames = Observation::known(vec!["sample-other".to_string()]);
    assert!(!fires(&known.rego_source, &input));

    let unknown = translated(
        "movie",
        3,
        &[rule(
            None,
            JELLYFIN_SEEN_BY,
            CONTAINS_ALL,
            text(r#"["sample-user","sample-other"]"#),
        )],
    );
    assert_eq!(
        warnings(&unknown),
        vec![codes::FACT_APPROXIMATE, codes::USERNAME_UNKNOWN]
    );

    let requested = translated(
        "movie",
        3,
        &[rule(None, SEERR_ADD_USER, EQUALS, text("sample-user"))],
    );
    let mut input = movie();
    input.facts.requested_by_usernames = Observation::known(vec!["Sample-User".to_string()]);
    assert!(fires(&requested.rego_source, &input));

    let monitored = translated("movie", 3, &[a(None)]);
    assert!(!monitored.reads_person_targeted_fact);
}

#[test]
fn per_user_watch_properties_read_that_users_history() {
    let views = rule(None, TAUTULLI_VIEWS_BY_USER, BIGGER, number("2")).user("Sample-User");
    let views = translated("movie", 3, &[views]);
    assert!(views.reads_person_targeted_fact);
    let mut input = movie();
    input.facts.watched_by_users = Observation::known(vec![
        watcher("sample-user", Some(NOW - DAY), 3),
        watcher("sample-other", Some(NOW - DAY), 9),
    ]);
    assert!(fires(&views.rego_source, &input));
    input.facts.watched_by_users = Observation::known(vec![watcher("sample-other", None, 9)]);
    assert!(!fires(&views.rego_source, &input));

    let last = rule(
        None,
        TAUTULLI_LAST_VIEWED_BY_USER,
        BEFORE,
        number("2592000"),
    )
    .user("sample-user");
    let last = translated("movie", 3, &[last]);
    input.facts.watched_by_users =
        Observation::known(vec![watcher("sample-user", Some(NOW - 60 * DAY), 1)]);
    assert!(fires(&last.rego_source, &input));
    input.facts.watched_by_users =
        Observation::known(vec![watcher("sample-user", Some(NOW - DAY), 1)]);
    assert!(!fires(&last.rego_source, &input));
    input.facts.watched_by_users = Observation::known(Vec::new());
    assert!(!fires(&last.rego_source, &input));

    let stranger = rule(None, TAUTULLI_VIEWS_BY_USER, BIGGER, number("2")).user("sample-stranger");
    assert_eq!(
        warnings(&translated("movie", 3, &[stranger])),
        vec![codes::FACT_APPROXIMATE, codes::USERNAME_UNKNOWN]
    );
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, TAUTULLI_VIEWS_BY_USER, BIGGER, number("2"))]
        ),
        vec![codes::RULE_USERNAME_MISSING]
    );
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, TAUTULLI_VIEWS_BY_USER, EXISTS, With::Nothing).user("sample-user")]
        ),
        vec![codes::COMPARATOR_UNSUPPORTED]
    );
}

// ── Failures ───────────────────────────────────────────────────────────────

#[test]
fn anything_without_a_faithful_equivalent_fails_the_group() {
    let unmapped = translate_one(
        "movie",
        3,
        &[a(None), rule(OR, RADARR_PROFILE, EQUALS, text("sd"))],
    );
    assert_eq!(unmapped.status, MaintainerrRuleStatus::Failed);
    assert_eq!(errors(&unmapped), vec![codes::PROPERTY_UNMAPPED]);
    assert_eq!(unmapped.diagnostics[0].rule_index, Some(1));
    assert_eq!(unmapped.name, "(Maintainerr) FAILED - Sample Rule 1");
    assert_eq!(
        unmapped.description,
        "Could not translate from Maintainerr: Radarr \"profile\".\n\nSample description"
    );

    let cases: Vec<(&str, Vec<Rule>, &str)> = vec![
        (
            "movie",
            vec![rule(None, JELLYFIN_COLLECTIONS, BIGGER, number("0"))],
            codes::PROPERTY_COLLECTION_UNSUPPORTED,
        ),
        (
            "movie",
            vec![rule(None, (99, 0), BIGGER, number("0"))],
            codes::APPLICATION_UNKNOWN,
        ),
        (
            "movie",
            vec![rule(None, (RADARR, 99), BIGGER, number("0"))],
            codes::PROPERTY_UNKNOWN,
        ),
        (
            "show",
            vec![rule(None, RADARR_TAGS, CONTAINS, text("x"))],
            codes::PROPERTY_LEVEL_UNSUPPORTED,
        ),
        (
            "show",
            vec![rule(None, JELLYFIN_VIEW_COUNT, BIGGER, number("0"))],
            codes::PROPERTY_LEVEL_UNSUPPORTED,
        ),
        (
            "movie",
            vec![rule(None, SONARR_SEASON_NUMBER, BIGGER, number("0"))],
            codes::PROPERTY_LEVEL_UNSUPPORTED,
        ),
        // Sonarr's deprecated numeric "monitored" is not its boolean one.
        (
            "show",
            vec![rule(None, SONARR_MONITORED_NUMBER, EQUALS, number("1"))],
            codes::PROPERTY_UNMAPPED,
        ),
        (
            "movie",
            vec![rule(None, RADARR_FILE_SIZE, BIGGER, number("abc"))],
            codes::CUSTOM_VALUE_INVALID,
        ),
    ];
    for (level, rules, code) in cases {
        assert_eq!(failed_with(level, 3, &rules), vec![code], "{code}");
    }

    let mut disabled = group(1, "movie", 3, &[a(None)]);
    disabled["useRules"] = json!(false);
    let empty = group(2, "movie", 3, &[]);
    let mut no_type = group(3, "movie", 3, &[a(None)]);
    no_type["dataType"] = Value::Null;
    no_type["collection"]["type"] = json!("music");
    let mut no_action = group(4, "movie", 3, &[a(None)]);
    no_action["collection"]["arrAction"] = Value::Null;
    let mut malformed = group(5, "movie", 3, &[a(None)]);
    malformed["rules"][0]["ruleJson"] = json!("{not json");
    let too_many = group(
        6,
        "movie",
        3,
        &vec![a(OR); MAX_MAINTAINERR_RULES_PER_GROUP + 1],
    );
    let results = translate_with(
        &[disabled, empty, no_type, no_action, malformed, too_many],
        &context(),
    );
    let found: Vec<Vec<&str>> = results.iter().map(errors).collect();
    assert_eq!(
        found,
        vec![
            vec![codes::GROUP_RULES_DISABLED],
            vec![codes::GROUP_RULES_EMPTY],
            vec![codes::GROUP_MEDIA_TYPE_UNSUPPORTED],
            vec![codes::ACTION_MISSING],
            vec![codes::RULE_INVALID],
            vec![codes::GROUP_RULES_TOO_MANY],
        ]
    );
    for result in &results {
        assert_eq!(result.status, MaintainerrRuleStatus::Failed);
        assert_eq!(result.rego_source, LIFECYCLE_STARTER_SOURCE);
    }
}

#[test]
fn embedded_text_is_escaped() {
    let hostile = "Sample \"Rule\" \\ with } and\nnewline\u{2028}end";
    let tag = "a\"b\\c}\nd";
    let mut value = group(1, "movie", 4, &[]);
    value["name"] = json!(hostile);
    value["description"] = json!("Sample\n} description");
    value["rules"] = json!([row(
        1,
        1,
        &rule(
            None,
            RADARR_TAGS,
            CONTAINS,
            text(Box::leak(tag.to_string().into_boxed_str()))
        )
    )]);
    let result = translate_with(&[value], &context())
        .pop()
        .expect("one rule");
    assert_eq!(
        result.status,
        MaintainerrRuleStatus::Translated,
        "{:?}",
        result.diagnostics
    );
    assert_eq!(result.source_group_name, hostile);
    assert_eq!(
        result.name,
        "(Maintainerr) Sample \"Rule\" \\ with } and newline end"
    );
    for line in result
        .rego_source
        .lines()
        .filter(|line| line.starts_with('#'))
    {
        assert!(!line.contains('\u{2028}'), "{line}");
    }
    let mut input = movie();
    input.facts.tags = Observation::known(vec![tag.to_string()]);
    let (outcome, actions) = decide(&result.rego_source, &input);
    assert_eq!(outcome, LifecycleOutcome::Match);
    assert_eq!(
        actions.notify.as_deref(),
        Some("Matched the Maintainerr rule group \"Sample \"Rule\" \\ with } and newline end\".")
    );
    input.facts.tags = Observation::known(vec!["a\"b\\c}".to_string()]);
    assert!(!fires(&result.rego_source, &input));
}

#[test]
fn a_renumbered_catalogue_binds_by_name() {
    let rules = [
        a(None),
        b(AND),
        c(OR),
        rule(OR, JELLYFIN_SEEN_BY, CONTAINS, text("sample-user")),
    ];
    let original = translated("movie", 0, &rules);

    let mut constants: Value = serde_json::from_str(&catalogue()).expect("catalogue parses");
    for application in constants["applications"]
        .as_array_mut()
        .expect("applications")
    {
        let id = application["id"].as_i64().expect("id");
        application["id"] = json!(id + 100);
        for prop in application["props"].as_array_mut().expect("props") {
            let prop_id = prop["id"].as_i64().expect("id");
            prop["id"] = json!(prop_id + 1000);
        }
    }
    let renumbered: Vec<Rule> = rules
        .iter()
        .cloned()
        .map(|mut rule| {
            rule.first = (rule.first.0 + 100, rule.first.1 + 1000);
            rule
        })
        .collect();
    let groups = Value::from(vec![group(1, "movie", 0, &renumbered)]).to_string();
    let moved = translate_maintainerr_rule_groups(&groups, &constants.to_string(), &context())
        .expect("readable")
        .pop()
        .expect("one rule");
    assert_eq!(moved, original);
}

#[test]
fn translation_is_deterministic() {
    let first = translate_maintainerr_rule_groups(LIVE_RULES, LIVE_CONSTANTS, &context())
        .expect("readable");
    let second = translate_maintainerr_rule_groups(LIVE_RULES, LIVE_CONSTANTS, &context())
        .expect("readable");
    assert_eq!(first, second);
}

// ── Bindings to metadata, ratings and file facts ───────────────────────────

const PLEX: i64 = 0;
const PLEX_RATING_IMDB: (i64, i64) = (PLEX, 31);
const PLEX_RATING_TMDB: (i64, i64) = (PLEX, 34);
const PLEX_RATING_IMDB_SHOW: (i64, i64) = (PLEX, 35);
const PLEX_RATING_TMDB_SHOW: (i64, i64) = (PLEX, 38);
const JELLYFIN_RATING_IMDB: (i64, i64) = (JELLYFIN, 44);
const RADARR_RELEASE_DATE: (i64, i64) = (RADARR, 4);
const RADARR_FILE_DATE: (i64, i64) = (RADARR, 10);
const RADARR_IMDB_RATING: (i64, i64) = (RADARR, 16);
const RADARR_IMDB_VOTES: (i64, i64) = (RADARR, 19);
const SONARR_STATUS: (i64, i64) = (SONARR, 6);
const SONARR_ENDED: (i64, i64) = (SONARR, 7);

fn rating(
    source: &str,
    value: Option<f64>,
    votes: Option<i64>,
) -> scryer_rules::lifecycle::LifecycleRatingDoc {
    scryer_rules::lifecycle::LifecycleRatingDoc {
        source: source.to_string(),
        value,
        normalized: value.unwrap_or(0.0),
        votes,
    }
}

fn check_at(
    data_type: &str,
    input: &LifecycleInput,
    first: (i64, i64),
    action: i64,
    with: With<'static>,
    expected: bool,
) {
    let result = translated(data_type, 3, &[rule(None, first, action, with)]);
    assert!(
        warnings(&result).contains(&codes::FACT_APPROXIMATE),
        "{:?}",
        result.diagnostics
    );
    assert_eq!(
        fires(&result.rego_source, input),
        expected,
        "{first:?} action {action}\n{}",
        result.rego_source
    );
}

#[test]
fn imdb_and_tmdb_ratings_read_the_stored_rating_of_that_source() {
    let mut input = movie();
    input.facts.ratings = Observation::known(vec![
        rating("tmdb", Some(6.1), Some(900)),
        rating("IMDb", Some(7.4), Some(12_000)),
    ]);
    for first in [RADARR_IMDB_RATING, PLEX_RATING_IMDB] {
        check_at("movie", &input, first, BIGGER, number("7"), true);
        check_at("movie", &input, first, SMALLER, number("7"), false);
        check_at("movie", &input, first, EXISTS, With::Nothing, true);
    }
    check_at(
        "movie",
        &input,
        RADARR_IMDB_VOTES,
        BIGGER,
        number("10000"),
        true,
    );
    check_at(
        "movie",
        &input,
        RADARR_IMDB_VOTES,
        SMALLER,
        number("10000"),
        false,
    );
    check_at(
        "movie",
        &input,
        PLEX_RATING_TMDB,
        SMALLER,
        number("6.5"),
        true,
    );
    let mut series = show();
    series.facts.ratings = input.facts.ratings.clone();
    check_at(
        "show",
        &series,
        PLEX_RATING_TMDB,
        SMALLER,
        number("6.5"),
        true,
    );

    // A season or an episode reads its show's rating.
    for (data_type, mut narrow) in [("season", season()), ("episode", episode())] {
        narrow.facts.ratings = input.facts.ratings.clone();
        check_at(
            data_type,
            &narrow,
            PLEX_RATING_IMDB_SHOW,
            BIGGER,
            number("7"),
            true,
        );
        check_at(
            data_type,
            &narrow,
            PLEX_RATING_TMDB_SHOW,
            BIGGER,
            number("7"),
            false,
        );
    }

    // No rating from that source, or one without a value, never matches.
    input.facts.ratings = Observation::known(vec![rating("imdb", None, Some(3))]);
    check_at(
        "movie",
        &input,
        RADARR_IMDB_RATING,
        SMALLER,
        number("7"),
        false,
    );
    check_at(
        "movie",
        &input,
        RADARR_IMDB_RATING,
        NOT_EXISTS,
        With::Nothing,
        true,
    );
    check_at(
        "movie",
        &input,
        RADARR_IMDB_VOTES,
        SMALLER,
        number("7"),
        true,
    );
    input.facts.ratings = Observation::known(Vec::new());
    check_at(
        "movie",
        &input,
        PLEX_RATING_IMDB,
        NOT_EQUALS,
        number("7"),
        false,
    );

    // Jellyfin and Emby fill their IMDb rating from the community rating,
    // and an episode's own rating is not stored: neither binds.
    assert_eq!(
        failed_with(
            "movie",
            3,
            &[rule(None, JELLYFIN_RATING_IMDB, BIGGER, number("7"))]
        ),
        vec![codes::PROPERTY_UNMAPPED]
    );
    assert_eq!(
        failed_with(
            "episode",
            3,
            &[rule(None, PLEX_RATING_IMDB, BIGGER, number("7"))]
        ),
        vec![codes::PROPERTY_LEVEL_UNSUPPORTED]
    );
}

#[test]
fn radarr_release_date_is_the_earlier_of_digital_and_physical() {
    let mut input = movie();
    input.facts.digital_release_date = Observation::known("2023-05-02".to_string());
    input.facts.physical_release_date = Observation::known("2023-03-10".to_string());
    let cutoff = "2023-04-01";
    check_at(
        "movie",
        &input,
        RADARR_RELEASE_DATE,
        BEFORE,
        date(cutoff),
        true,
    );
    input.facts.physical_release_date = Observation::absent();
    check_at(
        "movie",
        &input,
        RADARR_RELEASE_DATE,
        BEFORE,
        date(cutoff),
        false,
    );
    check_at(
        "movie",
        &input,
        RADARR_RELEASE_DATE,
        AFTER,
        date(cutoff),
        true,
    );
    input.facts.digital_release_date = Observation::absent();
    check_at(
        "movie",
        &input,
        RADARR_RELEASE_DATE,
        AFTER,
        date(cutoff),
        false,
    );
    check_at(
        "movie",
        &input,
        RADARR_RELEASE_DATE,
        NOT_EXISTS,
        With::Nothing,
        true,
    );
}

#[test]
fn radarr_file_date_is_when_the_newest_file_was_recorded() {
    let mut input = movie();
    let Observation::Known {
        value: mut files, ..
    } = input.facts.files.clone()
    else {
        panic!("the synthetic movie has files");
    };
    files[0].added_at = Some(rfc3339(NOW - 200 * DAY));
    let mut newer = files[0].clone();
    newer.added_at = Some(rfc3339(NOW - 10 * DAY));
    files.push(newer);
    input.facts.files = Observation::known(files);
    check_at(
        "movie",
        &input,
        RADARR_FILE_DATE,
        IN_LAST,
        number("2592000"),
        true,
    );
    check_at(
        "movie",
        &input,
        RADARR_FILE_DATE,
        BEFORE,
        number("8640000"),
        false,
    );
    input.facts.files = Observation::known(Vec::new());
    check_at(
        "movie",
        &input,
        RADARR_FILE_DATE,
        IN_LAST,
        number("2592000"),
        false,
    );
    check_at(
        "movie",
        &input,
        RADARR_FILE_DATE,
        NOT_EXISTS,
        With::Nothing,
        true,
    );
}

#[test]
fn sonarr_status_and_ended_read_the_shows_status() {
    let mut input = show();
    input.facts.series_status = Observation::known("Ended".to_string());
    check_at("show", &input, SONARR_STATUS, EQUALS, text("ended"), true);
    check_at(
        "show",
        &input,
        SONARR_STATUS,
        NOT_EQUALS,
        text("continuing"),
        true,
    );
    check_at("show", &input, SONARR_ENDED, EQUALS, boolean("1"), true);
    check_at("show", &input, SONARR_ENDED, EQUALS, boolean("0"), false);
    input.facts.series_status = Observation::known("continuing".to_string());
    check_at("show", &input, SONARR_ENDED, EQUALS, boolean("0"), true);
    check_at("show", &input, SONARR_STATUS, EQUALS, text("ended"), false);
    // A season reads its show's status.
    let mut narrow = season();
    narrow.facts.series_status = Observation::known("ended".to_string());
    check_at("season", &narrow, SONARR_ENDED, EQUALS, boolean("1"), true);
    // An unknown status never matches either way.
    input.facts.series_status = Observation::absent();
    check_at("show", &input, SONARR_ENDED, EQUALS, boolean("0"), false);
    check_at("show", &input, SONARR_ENDED, EQUALS, boolean("1"), false);
    assert_eq!(
        failed_with(
            "episode",
            3,
            &[rule(None, SONARR_ENDED, EQUALS, boolean("1"))]
        ),
        vec![codes::PROPERTY_LEVEL_UNSUPPORTED]
    );
}
