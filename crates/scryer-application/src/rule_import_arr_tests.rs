use super::*;
use crate::external_import::{ArrProfileFormatScore, ArrQualityProfileFormatScores};

fn title_format(id: i64, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "specifications": [{
            "name": "Sample",
            "implementation": "ReleaseTitleSpecification",
            "required": true,
            "negate": false,
            "fields": [{ "name": "value", "value": "\\bsample\\b" }],
        }],
    })
}

fn year_format(id: i64, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "specifications": [{
            "name": "Year",
            "implementation": "YearSpecification",
            "required": true,
            "negate": false,
            "fields": [
                { "name": "min", "value": 2000 },
                { "name": "max", "value": 2010 },
            ],
        }],
    })
}

fn profile(id: i64, name: &str, scores: &[(i64, i64)]) -> ArrQualityProfileFormatScores {
    ArrQualityProfileFormatScores {
        id,
        name: name.to_string(),
        format_items: scores
            .iter()
            .map(|(format, score)| ArrProfileFormatScore {
                format: *format,
                name: format!("format {format}"),
                score: *score,
            })
            .collect(),
    }
}

fn fetched(formats: Vec<Value>, profiles: Vec<ArrQualityProfileFormatScores>) -> FetchedArrRules {
    FetchedArrRules {
        app_name: "Radarr".into(),
        version: "6.0.4.10291".into(),
        custom_formats: formats,
        quality_profiles: profiles,
    }
}

fn scryer_profiles(names: &[&str]) -> RuleImportPlanContext {
    RuleImportPlanContext {
        quality_profile_names: names.iter().map(|name| name.to_string()).collect(),
    }
}

fn plan(fetched: &FetchedArrRules, context: &RuleImportPlanContext) -> Vec<PlannedRuleImportItem> {
    plan_arr_rule_import(ArrSourceApplication::Radarr, fetched, context).unwrap()
}

fn draft(item: &PlannedRuleImportItem) -> &ArrScoringRuleDraft {
    match &item.translation {
        PlannedRuleImportTranslation::ArrScoringRule(PlannedArrScoringRule::Save(draft)) => draft,
        other => panic!("not saved: {other:?}"),
    }
}

fn failed(item: &PlannedRuleImportItem) -> (&'static str, &[RuleImportFact]) {
    match &item.translation {
        PlannedRuleImportTranslation::ArrScoringRule(PlannedArrScoringRule::Failed {
            reason,
            facts,
        }) => (reason, facts),
        other => panic!("not failed: {other:?}"),
    }
}

fn assert_valid(rego_source: &str) {
    let id = "arr_import_test";
    let rewritten = scryer_rules::rewrite_package_declaration(rego_source, id);
    let validation = scryer_rules::validation::validate_user_rule(&rewritten, id).unwrap();
    assert!(validation.valid, "{validation:?}\n{rego_source}");
}

#[test]
fn reason_codes_are_distinct_and_never_reuse_a_refusal() {
    let distinct = reasons::ALL
        .iter()
        .chain(refusal::ALL)
        .collect::<HashSet<_>>();
    assert_eq!(distinct.len(), reasons::ALL.len() + refusal::ALL.len());
    for reason in reasons::ALL {
        assert!(reason.starts_with("RULE_IMPORT_"), "{reason}");
        assert_eq!(reason.to_ascii_uppercase(), *reason);
    }
}

#[test]
fn agreeing_profiles_save_a_literal_score() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile B", &[(1, 25)]),
        ],
    );
    let planned = plan(&fetched, &RuleImportPlanContext::default());
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].key, "customformat:1");
    let draft = draft(&planned[0]);
    assert_eq!(draft.name, "(Radarr) Sample Format Alpha");
    assert_eq!(draft.description, "Imported from Radarr 6.0.4.10291.");
    assert_eq!(draft.applied_facets, [MediaFacet::Movie]);
    assert_eq!(draft.state, RuleImportItemState::Saved);
    assert_eq!(draft.reason, None);
    assert!(draft.facts.is_empty());
    assert!(
        draft.rego_source.contains(":= 25 if {"),
        "{}",
        draft.rego_source
    );
    assert_valid(&draft.rego_source);
}

#[test]
fn one_profile_is_a_literal_score_and_sonarr_applies_to_series_and_anime() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![profile(1, "Sample Profile A", &[(1, -40)])],
    );
    let planned = plan_arr_rule_import(
        ArrSourceApplication::Sonarr,
        &fetched,
        &RuleImportPlanContext::default(),
    )
    .unwrap();
    let draft = draft(&planned[0]);
    assert_eq!(draft.name, "(Sonarr) Sample Format Alpha");
    assert_eq!(
        draft.applied_facets,
        [MediaFacet::Series, MediaFacet::Anime]
    );
    assert!(draft.rego_source.contains(":= -40 if {"));
}

#[test]
fn unscored_formats_still_import_with_an_informational_fact() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 0)]),
            profile(2, "Sample Profile B", &[]),
        ],
    );
    let planned = plan(&fetched, &RuleImportPlanContext::default());
    let draft = draft(&planned[0]);
    assert_eq!(draft.state, RuleImportItemState::Saved);
    assert_eq!(draft.reason, None);
    assert_eq!(draft.facts[0].code, reasons::FORMAT_UNSCORED);
    assert!(draft.rego_source.contains(":= 0 if {"));
    assert_valid(&draft.rego_source);
}

#[test]
fn disagreeing_profiles_that_all_match_score_by_profile_name() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile B", &[(1, 0)]),
            profile(3, "Sample Profile C", &[]),
            profile(4, "Sample Profile D", &[(1, -5)]),
        ],
    );
    let context = scryer_profiles(&[
        "Sample Profile A",
        "Sample Profile B",
        "Sample Profile C",
        "Sample Profile D",
        "Scryer Only",
    ]);
    let planned = plan(&fetched, &context);
    let draft = draft(&planned[0]);
    assert_eq!(draft.name, "(Radarr) Sample Format Alpha");
    assert_eq!(draft.state, RuleImportItemState::Saved);
    assert_eq!(draft.reason, None);
    // Zero scores are left out so those profiles yield no entry.
    assert!(
        draft
            .rego_source
            .contains("\"Sample Profile A\": 25,\n  \"Sample Profile D\": -5\n}"),
        "{}",
        draft.rego_source
    );
    assert!(draft.rego_source.contains("[input.profile.name] if {"));
    assert_eq!(draft.facts[0].code, reasons::PROFILE_SCORES_APPLIED);
    assert_eq!(
        draft.facts[0].details.as_ref().unwrap()["scores"],
        json!([
            { "profile": "Sample Profile A", "score": 25 },
            { "profile": "Sample Profile B", "score": 0 },
            { "profile": "Sample Profile C", "score": 0 },
            { "profile": "Sample Profile D", "score": -5 },
        ])
    );
    assert_valid(&draft.rego_source);
}

#[test]
fn profile_names_match_exactly() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile B", &[(1, 10)]),
        ],
    );
    // A case difference is not a match.
    let context = scryer_profiles(&["sample profile a", "Sample Profile B"]);
    let planned = plan(&fetched, &context);
    let draft = draft(&planned[0]);
    assert_eq!(draft.state, RuleImportItemState::SavedNeedsReview);
    assert_eq!(
        draft.facts[0].details.as_ref().unwrap()["unmatchedProfiles"],
        json!(["Sample Profile A"])
    );
}

#[test]
fn profile_names_match_once_trimmed_and_the_table_uses_the_scryer_spelling() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, " Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile B", &[(1, 10)]),
        ],
    );
    let context = scryer_profiles(&["  Sample Profile A ", "Sample Profile B"]);
    let planned = plan(&fetched, &context);
    let saved = draft(&planned[0]);
    assert_eq!(saved.state, RuleImportItemState::Saved);
    assert!(
        saved
            .rego_source
            .contains("\"  Sample Profile A \": 25,\n  \"Sample Profile B\": 10\n}"),
        "{}",
        saved.rego_source
    );
    // The report names profiles as the source does.
    assert_eq!(
        saved.facts[0].details.as_ref().unwrap()["scores"][0]["profile"],
        "Sample Profile A"
    );
    assert_valid(&saved.rego_source);

    // Two Scryer profiles that trim to one name match neither.
    let ambiguous =
        scryer_profiles(&["Sample Profile A ", " Sample Profile A", "Sample Profile B"]);
    let planned = plan(&fetched, &ambiguous);
    assert_eq!(
        draft(&planned[0]).facts[0].details.as_ref().unwrap()["unmatchedProfiles"],
        json!(["Sample Profile A"])
    );
}

#[test]
fn unmatched_disagreeing_profiles_save_a_review_rule_scoring_zero() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile B", &[(1, -10)]),
            profile(3, "Sample Profile C", &[]),
        ],
    );
    let planned = plan(&fetched, &scryer_profiles(&["Sample Profile A"]));
    let draft = draft(&planned[0]);
    assert_eq!(draft.name, "(Radarr) REVIEW - Sample Format Alpha");
    assert_eq!(draft.state, RuleImportItemState::SavedNeedsReview);
    assert_eq!(draft.reason, Some(reasons::PROFILE_SCORES_UNMATCHED));
    let details = draft.facts[0].details.as_ref().unwrap();
    assert_eq!(draft.facts[0].code, reasons::PROFILE_SCORES_UNMATCHED);
    assert_eq!(
        details["unmatchedProfiles"],
        json!(["Sample Profile B", "Sample Profile C"])
    );
    assert_eq!(details["duplicateProfiles"], json!([]));
    assert_eq!(
        details["scores"],
        json!([
            { "profile": "Sample Profile A", "score": 25 },
            { "profile": "Sample Profile B", "score": -10 },
            { "profile": "Sample Profile C", "score": 0 },
        ])
    );
    let active: Vec<&str> = draft
        .rego_source
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert!(active.iter().any(|line| line.ends_with(":= 0 if {")));
    assert!(!active.iter().any(|line| line.contains("_scores")));
    assert!(
        draft.rego_source.contains("#   \"Sample Profile B\": -10,"),
        "{}",
        draft.rego_source
    );
    assert_valid(&draft.rego_source);
}

#[test]
fn a_repeated_profile_name_with_another_score_needs_review() {
    let fetched = fetched(
        vec![title_format(1, "Sample Format Alpha")],
        vec![
            profile(1, "Sample Profile A", &[(1, 25)]),
            profile(2, "Sample Profile A", &[(1, 5)]),
        ],
    );
    let planned = plan(&fetched, &scryer_profiles(&["Sample Profile A"]));
    let draft = draft(&planned[0]);
    assert_eq!(draft.state, RuleImportItemState::SavedNeedsReview);
    let details = draft.facts[0].details.as_ref().unwrap();
    // The profile exists in Scryer; the source repeating its name is the problem.
    assert_eq!(details["unmatchedProfiles"], json!([]));
    assert_eq!(details["duplicateProfiles"], json!(["Sample Profile A"]));
}

#[test]
fn untranslatable_unreadable_and_out_of_range_formats_save_nothing() {
    let fetched = fetched(
        vec![
            year_format(1, "Sample Format Beta"),
            json!(["not a format"]),
            title_format(3, "Sample Format Gamma"),
            title_format(4, "Sample Format Alpha"),
        ],
        vec![profile(
            1,
            "Sample Profile A",
            &[(3, i64::from(i32::MAX) + 1)],
        )],
    );
    let planned = plan(&fetched, &RuleImportPlanContext::default());
    assert_eq!(planned.len(), 4);

    let (reason, facts) = failed(&planned[0]);
    assert_eq!(reason, reasons::FORMAT_UNTRANSLATABLE);
    assert_eq!(planned[0].source_name, "Sample Format Beta");
    assert!(!facts.is_empty());
    let details = facts[0].details.as_ref().unwrap();
    assert_eq!(details["severity"], "ERROR");
    assert!(details.get("location").is_some());

    let (reason, facts) = failed(&planned[1]);
    assert_eq!(reason, reasons::FORMAT_UNREADABLE);
    assert_eq!(planned[1].key, "customformat:#1");
    assert_eq!(facts[0].code, diagnostic_codes::FORMAT_INVALID);

    let (reason, facts) = failed(&planned[2]);
    assert_eq!(reason, reasons::FORMAT_UNTRANSLATABLE);
    assert_eq!(facts[0].code, diagnostic_codes::SCORE_INVALID);

    assert_eq!(draft(&planned[3]).name, "(Radarr) Sample Format Alpha");
}

#[test]
fn too_many_formats_are_refused() {
    let fetched = fetched(vec![json!({}); MAX_ITEMS + 1], Vec::new());
    let error = plan_arr_rule_import(ArrSourceApplication::Radarr, &fetched, &Default::default())
        .unwrap_err();
    assert_eq!(error.reason(), Some(refusal::TOO_MANY_ITEMS));
}

#[test]
fn hostile_names_and_versions_stay_inert() {
    let hostile = "Evil\"\n} allow := true # \\ \r\t\u{1}end";
    let mut fetched = fetched(
        vec![title_format(1, hostile)],
        vec![
            profile(1, hostile, &[(1, 25)]),
            profile(2, "Sample Profile A", &[(1, 5)]),
        ],
    );
    fetched.version = "6.0\n} deny := true #".into();
    for context in [
        RuleImportPlanContext::default(),
        scryer_profiles(&[hostile, "Sample Profile A"]),
    ] {
        let planned = plan(&fetched, &context);
        let draft = draft(&planned[0]);
        assert_valid(&draft.rego_source);
        for line in draft.rego_source.lines() {
            assert!(!line.starts_with("} "), "{line}\n{}", draft.rego_source);
        }
    }
}

#[test]
fn control_characters_from_the_source_never_reach_names_descriptions_or_source() {
    let mut fetched = fetched(
        vec![title_format(1, "Synthetic\u{0}Format\u{2028}Name\u{7f} ")],
        vec![profile(1, "Sample Profile A", &[(1, 25)])],
    );
    fetched.version = "6.0\u{0}\u{2029}# early\u{1b}end".into();
    let planned = plan(&fetched, &RuleImportPlanContext::default());
    let draft = draft(&planned[0]);
    assert_eq!(draft.name, "(Radarr) Synthetic Format Name");
    assert_eq!(planned[0].source_name, "Synthetic Format Name");
    assert_eq!(draft.description, "Imported from Radarr 6.0 # early end.");
    let hostile = |text: &str| {
        text.chars()
            .any(|c| (c.is_control() && c != '\n') || c == '\u{2028}' || c == '\u{2029}')
    };
    for text in [&draft.name, &draft.description, &draft.rego_source] {
        assert!(!hostile(text), "{text:?}");
    }
    assert!(draft.claimed_names.iter().all(|name| !hostile(name)));
    assert_valid(&draft.rego_source);
}

#[test]
fn a_format_is_imported_under_either_of_its_names() {
    let fetched = fetched(
        vec![
            title_format(1, "Sample Format Alpha"),
            title_format(2, "Sample Format Beta"),
        ],
        vec![
            profile(1, "Sample Profile A", &[(1, 25), (2, 25)]),
            profile(2, "Sample Profile Unmatched", &[(1, 5), (2, 5)]),
        ],
    );
    // The profiles disagree and Scryer has neither: both formats plan for review.
    let review = plan(&fetched, &RuleImportPlanContext::default());
    assert_eq!(
        draft(&review[0]).name,
        "(Radarr) REVIEW - Sample Format Alpha"
    );
    // Profiles now match: both formats plan under their plain names.
    let matched = plan(
        &fetched,
        &scryer_profiles(&["Sample Profile A", "Sample Profile Unmatched"]),
    );
    assert_eq!(draft(&matched[0]).name, "(Radarr) Sample Format Alpha");

    for (planned, existing, expected) in [
        // An earlier run saved the plain name; this one would save for review.
        (
            review,
            "(Radarr) Sample Format Alpha",
            "(Radarr) Sample Format Alpha",
        ),
        // An earlier run saved for review; this one would save the plain name.
        (
            matched,
            "(Radarr) REVIEW - Sample Format Alpha",
            "(Radarr) REVIEW - Sample Format Alpha",
        ),
    ] {
        let mut context = RuleImportSaveContext {
            existing_names: HashSet::from([existing.to_string()]),
            ..Default::default()
        };
        let outcomes: Vec<RuleImportSaveOutcome> = planned
            .into_iter()
            .enumerate()
            .map(|(index, item)| match item.translation {
                PlannedRuleImportTranslation::ArrScoringRule(rule) => {
                    stage_arr_scoring_rule(&mut context, index, rule)
                }
            })
            .collect();
        assert_eq!(outcomes[0].state, RuleImportItemState::SkippedExists);
        assert_eq!(outcomes[0].saved_name.as_deref(), Some(expected));
        assert_eq!(outcomes[1].state, RuleImportItemState::Pending);
        assert_eq!(context.staged_rule_sets.len(), 1);
        let staged = &context.staged_rule_sets[0];
        assert_eq!(staged.claimed_names.len(), 2);
        assert!(staged.claimed_names.contains(&staged.rule_set.name));
        assert!(
            staged
                .claimed_names
                .iter()
                .any(|name| name.contains("REVIEW - Sample Format Beta"))
        );
        assert!(
            staged
                .claimed_names
                .contains(&"(Radarr) Sample Format Beta".to_string())
        );
    }
}

#[test]
fn staging_skips_existing_and_repeated_names_and_saves_disabled() {
    let fetched = fetched(
        vec![
            title_format(1, "Sample Format Alpha"),
            title_format(2, "Sample Format Beta"),
            title_format(3, "Sample Format Beta"),
            year_format(4, "Sample Format Gamma"),
        ],
        vec![profile(1, "Sample Profile A", &[(1, 25), (2, 5), (3, 5)])],
    );
    let mut context = RuleImportSaveContext {
        existing_names: HashSet::from(["(Radarr) Sample Format Alpha".to_string()]),
        ..Default::default()
    };
    let outcomes: Vec<RuleImportSaveOutcome> = plan(&fetched, &RuleImportPlanContext::default())
        .into_iter()
        .enumerate()
        .map(|(index, item)| match item.translation {
            PlannedRuleImportTranslation::ArrScoringRule(rule) => {
                stage_arr_scoring_rule(&mut context, index, rule)
            }
        })
        .collect();
    let states: Vec<_> = outcomes.iter().map(|outcome| outcome.state).collect();
    assert_eq!(
        states,
        [
            RuleImportItemState::SkippedExists,
            RuleImportItemState::Pending,
            RuleImportItemState::SkippedExists,
            RuleImportItemState::Failed,
        ]
    );
    assert_eq!(outcomes[0].reason, Some(refusal::ALREADY_EXISTS));
    assert_eq!(outcomes[2].reason, Some(refusal::ALREADY_EXISTS));
    assert_eq!(outcomes[3].reason, Some(reasons::FORMAT_UNTRANSLATABLE));
    assert_eq!(context.staged_rule_sets.len(), 1);
    let staged = &context.staged_rule_sets[0];
    assert_eq!(staged.item_index, 1);
    assert_eq!(staged.state, RuleImportItemState::Saved);
    assert_eq!(staged.rule_set.name, "(Radarr) Sample Format Beta");
    assert!(!staged.rule_set.enabled);
}
