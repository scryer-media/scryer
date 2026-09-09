//! The TRaSH scoring verification corpus.
//!
//! The scoring redesign calls for "a before/after ranking corpus per facet and locale rather
//! than unit tests alone", covering the stacked-tie-breaker case from §2 and the
//! token-collision cases from §6a. This is that corpus.
//!
//! Two rules shape every case here:
//!
//! 1. **Assertions are relative, never absolute.** Each case parses invented
//!    release names with the real parser and scores them through the real
//!    bundled Rego pack (including the optional locale templates),
//!    then asserts an ordering. No expected score is written down, so
//!    recalibrating a pack weight does not churn the
//!    corpus — only a *reordering* fails it, which is exactly the regression
//!    worth catching.
//!
//! 2. **Every title is fictional.** Real release *group* names are the data
//!    under test and are used verbatim from the generated tables; real service
//!    tags likewise. The works are invented — Portmere, Glass Harbor, Umibe
//!    Signal, Copper Kettle — including the ones deliberately built around the
//!    dangerous collision tokens, because §6a's collision is with the token, not
//!    with any particular film.

use crate::quality::pack_test_support::{
    score_with_pack_for_category, test_scoring_config_for_category,
};
use crate::quality_profile::{
    BLOCK_SCORE, QualityProfile, QualityProfileCriteria, QualityProfileDecision, ScoringSource,
};
use crate::release_parser::parse_release_metadata;
use crate::rules::builtin_trash;
use crate::rules::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline seams
// ─────────────────────────────────────────────────────────────────────────────

/// The corpus runs one profile for everything, so a reordering is always
/// attributable to the release rather than to profile configuration.
fn corpus_profile() -> QualityProfile {
    QualityProfile {
        id: "adr-0007-corpus".to_string(),
        name: "TRaSH Ranking Corpus".to_string(),
        criteria: QualityProfileCriteria::default(),
    }
}

/// Parse and evaluate the bundled scoring rules with profile requirements.
fn corpus_decision(raw: &str, category: Option<&str>) -> QualityProfileDecision {
    let profile = corpus_profile();
    let weights = test_scoring_config_for_category(
        profile.criteria.resolve_persona(category),
        &profile.criteria.scoring_overrides,
        category,
    );
    let mut decision = score_with_pack_for_category(
        &profile,
        &parse_release_metadata(raw),
        false,
        &weights,
        category,
    );
    crate::quality_profile::apply_min_score_gate(&profile, &mut decision);
    decision
}

fn scoring_log(decision: &QualityProfileDecision) -> Vec<(String, i32)> {
    decision
        .scoring_log
        .iter()
        .map(|entry| (entry.code.clone(), entry.delta))
        .collect()
}

/// Assert a strict ranking, best first. Failure prints both scoring logs,
/// because "which entry moved" is the only useful thing to know here.
#[track_caller]
fn assert_ranked(category: Option<&str>, best_first: &[&str]) {
    let scored = best_first
        .iter()
        .map(|raw| (*raw, corpus_decision(raw, category)))
        .collect::<Vec<_>>();
    for pair in scored.windows(2) {
        let (better, worse) = (&pair[0], &pair[1]);
        assert!(
            better.1.preference_score > worse.1.preference_score,
            "expected `{}` ({}) to outrank `{}` ({})\n  better: {:?}\n  worse:  {:?}",
            better.0,
            better.1.preference_score,
            worse.0,
            worse.1.preference_score,
            scoring_log(&better.1),
            scoring_log(&worse.1),
        );
    }
}

/// One locale-pack corpus case: an invented release plus the title metadata the
/// language rules read.
struct LocaleCase<'a> {
    raw: &'a str,
    category: Option<&'a str>,
    /// Audio languages the indexer reports, which is where a MULTi release's
    /// second track comes from in production.
    indexer_languages: &'a [&'a str],
    original_language: Option<&'a str>,
}

/// Return one optional locale template from the actual bundled pack.
fn bundled_locale_template(template_id: &str) -> crate::RulePackTemplate {
    builtin_trash::verified_pack()
        .expect("bundled TRaSH pack parses")
        .templates
        .into_iter()
        .find(|template| template.id == template_id)
        .unwrap_or_else(|| panic!("unknown bundled locale template {template_id}"))
}

/// Return every optional locale template from the actual bundled pack.
fn bundled_locale_templates() -> Vec<crate::RulePackTemplate> {
    [FRENCH_VF, FRENCH_VO, FRENCH_VOSTFR, GERMAN, ASIAN]
        .into_iter()
        .map(bundled_locale_template)
        .collect()
}

/// Run a case through the same two seams production uses: the builtin decision,
/// then `build_rule_input` into a `UserRulesEngine` carrying the selected
/// bundled locale template.
fn evaluate_locale_pack(
    template_id: &str,
    case: &LocaleCase<'_>,
) -> (Vec<(String, i32)>, QualityProfileDecision) {
    let profile = corpus_profile();
    let category = case.category;
    let parsed = parse_release_metadata(case.raw);
    let weights = test_scoring_config_for_category(
        profile.criteria.resolve_persona(category),
        &profile.criteria.scoring_overrides,
        category,
    );
    let mut decision = score_with_pack_for_category(&profile, &parsed, false, &weights, category);

    let indexer_languages = case
        .indexer_languages
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    let input = build_rule_input(
        &parsed,
        &profile,
        &decision,
        ReleaseRuntimeInfo {
            size_bytes: Some(4_000_000_000),
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            is_password_protected: None,
            extra: None,
            indexer_languages: Some(&indexer_languages),
        },
        RuleContextInfo {
            title_id: Some("corpus-title"),
            library_name: Some("Corpus"),
            category,
            original_language: case.original_language,
            original_country: None,
            title_tags: &[],
            has_existing_file: false,
            existing_score: None,
            search_mode: "auto",
            runtime_minutes: Some(101),
            coverage_total_runtime_minutes: Some(101),
            coverage_member_runtime_minutes: Some(101),
            coverage_member_count: Some(1),
            is_filler: false,
        },
        None,
    );

    let template = bundled_locale_template(template_id);
    let id = template.id.replace('-', "_");
    let policy = scryer_rules::UserPolicy {
        id: id.clone(),
        name: template.title,
        rego_source: scryer_rules::rewrite_package_declaration(&template.rego_source, &id),
        origin: scryer_rules::PolicyOrigin::System,
        applied_facets: vec![],
    };
    let result = scryer_rules::UserRulesEngine::build(&[policy])
        .expect("bundled locale template should compile")
        .evaluator()
        .evaluate(&input, category.unwrap_or("movie"))
        .expect("bundled locale template should evaluate");
    assert!(
        result.errors.is_empty(),
        "{template_id}: {:?}",
        result.errors
    );

    let mut entries = Vec::new();
    for entry in result.entries {
        entries.push((entry.code.clone(), entry.delta));
        decision.log_with_source(
            &entry.code,
            entry.delta,
            ScoringSource::SystemRule {
                id: entry.rule_set_id,
                name: entry.rule_set_name,
            },
        );
    }
    entries.sort();
    crate::quality_profile::apply_min_score_gate(&profile, &mut decision);
    (entries, decision)
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) Release-group tier ladders, one invented title held constant per facet
// ─────────────────────────────────────────────────────────────────────────────

/// Groups are picked from `GROUP_RULES` with the facet + source context the
/// release actually carries: Movie/Web for the movie ladder, Series/Web for the
/// series ladder, Anime/Anime for the anime ladder. `PortmereWorks` is invented
/// and appears in no table, which is what makes it the unknown-group rung.
#[test]
fn movie_group_tier_ladder_ranks_gold_through_unknown_and_banishes_banned() {
    assert_ranked(
        Some("movie"),
        &[
            // FLUX: Movie / Web / Gold
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX",
            // SMURF: Movie / Web / Silver
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-SMURF",
            // BLOOM: Movie / Web / Bronze
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-BLOOM",
            // Invented group: no tier, so the unknown-group penalty applies.
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        ],
    );

    // AROMA: Movie / Any / Banned. A banned group is a veto, so it sits below
    // every rung of the ladder including the unknown one.
    assert_ranks_below_every_allowed_rung(
        Some("movie"),
        "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-AROMA",
        &[
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX",
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-SMURF",
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-BLOOM",
            "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        ],
    );
}

#[test]
fn series_group_tier_ladder_ranks_gold_through_unknown_and_banishes_banned() {
    assert_ranked(
        Some("series"),
        &[
            // FLUX: Series / Web / Gold
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-FLUX",
            // MZABI: Series / Web / Silver
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-MZABI",
            // T4H: Series / Web / Bronze
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-T4H",
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        ],
    );

    // BRiNK: Series / Any / Banned.
    assert_ranks_below_every_allowed_rung(
        Some("series"),
        "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-BRiNK",
        &[
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-FLUX",
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-MZABI",
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-T4H",
            "Umibe.Signal.S02E04.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        ],
    );
}

#[test]
fn anime_group_tier_ladder_ranks_gold_through_unknown_and_banishes_banned() {
    assert_ranked(
        Some("anime"),
        &[
            // Arid: Anime / Anime / Gold
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Arid",
            // Aergia: Anime / Anime / Silver
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Aergia",
            // Afro: Anime / Anime / Bronze
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Afro",
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-PortmereWorks",
        ],
    );

    // ASW: Anime / Anime / Banned.
    assert_ranks_below_every_allowed_rung(
        Some("anime"),
        "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-ASW",
        &[
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Arid",
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Aergia",
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-Afro",
            "Copper.Kettle.S01E05.1080p.WEB-DL.AAC2.0.H.264-PortmereWorks",
        ],
    );
}

#[track_caller]
fn assert_ranks_below_every_allowed_rung(category: Option<&str>, vetoed: &str, rungs: &[&str]) {
    let blocked = corpus_decision(vetoed, category);
    assert!(
        !blocked.allowed,
        "`{vetoed}` should be blocked: {:?}",
        scoring_log(&blocked)
    );
    for rung in rungs {
        let allowed = corpus_decision(rung, category);
        assert!(
            allowed.allowed,
            "`{rung}` should not be blocked: {:?}",
            scoring_log(&allowed)
        );
        assert!(
            blocked.preference_score < allowed.preference_score,
            "vetoed `{vetoed}` ({}) must rank below `{rung}` ({})\n  vetoed: {:?}",
            blocked.preference_score,
            allowed.preference_score,
            scoring_log(&blocked),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) The mapping's documented cost: compression can invert order across the knee
// ─────────────────────────────────────────────────────────────────────────────

/// Every `score_entry` a rendered locale template declares, read straight out of
/// the generated policy so the corpus never restates a score.
fn pack_score_entries(source: &str) -> Vec<(String, i32)> {
    source
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("score_entry[\"")?;
            let (code, rest) = rest.split_once("\"] := ")?;
            let value = rest.strip_suffix(" if {")?;
            Some((code.to_string(), value.parse::<i32>().ok()?))
        })
        .collect()
}

/// The reachability half of §2's accepted cost: the inversion is real in the
/// mapping but not reachable through today's shipped packs, because no pack
/// declares enough sub-knee positives to out-stack its own strongest entry.
///
/// This is the assertion that would fail first if upstream data drifted into
/// the shape the design warns about, which is precisely when `TRASH_CEILING` would
/// need revisiting.
#[test]
fn no_shipped_pack_can_currently_stack_its_way_across_the_knee() {
    // Frozen boundary of the converter's tie-break band, not a host formula.
    const TIE_BREAK_LIMIT: i32 = 100;
    for template in bundled_locale_templates() {
        let entries = pack_score_entries(&template.rego_source);
        assert!(!entries.is_empty(), "{} emitted no scores", template.id);

        let sub_knee_total: i32 = entries
            .iter()
            .filter(|(_, score)| *score > 0 && *score <= TIE_BREAK_LIMIT)
            .map(|(_, score)| *score)
            .sum();
        let strongest = entries
            .iter()
            .map(|(_, score)| *score)
            .max()
            .expect("non-empty");

        assert!(
            sub_knee_total < strongest,
            "{}: every sub-knee positive stacked ({sub_knee_total}) already reaches or passes the pack's strongest entry ({strongest}); the documented knee inversion is now live and TRASH_CEILING needs review\n  {entries:?}",
            template.id,
        );
    }
}

/// Calibration artifact: compression must not reorder a pack's own tiers. The
/// French tiers sit 50 upstream points apart and land 4–5 apart after
/// normalization; the guard is that they still land in the right order.
#[test]
fn normalization_preserves_each_packs_own_tier_ordering() {
    for template in bundled_locale_templates() {
        let entries = pack_score_entries(&template.rego_source);
        let tier = |name: &str| {
            entries
                .iter()
                .find(|(code, _)| code == name)
                .map(|(_, score)| *score)
                .unwrap_or_else(|| panic!("{} is missing {name}", template.id))
        };
        let (tier_1, tier_2, tier_3) = (
            tier("trash_tier_1"),
            tier("trash_tier_2"),
            tier("trash_tier_3"),
        );
        assert!(
            tier_1 >= tier_2 && tier_2 >= tier_3,
            "{}: tiers reordered after normalization: {tier_1} / {tier_2} / {tier_3}",
            template.id,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) Locale packs, through the real rules engine, one scenario per pack
// ─────────────────────────────────────────────────────────────────────────────

const FRENCH_VF: &str = "trash-guides-french-vf";
const FRENCH_VO: &str = "trash-guides-french-vo";
const FRENCH_VOSTFR: &str = "trash-guides-french-vostfr";
const GERMAN: &str = "trash-guides-german";
const ASIAN: &str = "trash-guides-asian";

/// MOONLY is a French Movie/Web tier-1 group upstream and carries no entry in
/// Scryer's native `GROUP_RULES`, so the tier-1 fact is the *only* difference
/// between these two releases — the ordering is the pack's doing.
#[test]
fn french_vf_ranks_a_tiered_multi_release_above_an_untiered_one() {
    let tiered = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.MULTi.1080p.WEB-DL.DDP5.1.H.264-MOONLY",
        category: Some("movie"),
        indexer_languages: &["French", "English"],
        original_language: Some("en-US"),
    };
    let untiered = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.MULTi.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        category: tiered.category,
        indexer_languages: tiered.indexer_languages,
        original_language: tiered.original_language,
    };

    let (tiered_entries, tiered_decision) = evaluate_locale_pack(FRENCH_VF, &tiered);
    let (untiered_entries, untiered_decision) = evaluate_locale_pack(FRENCH_VF, &untiered);

    assert!(
        tiered_entries
            .iter()
            .any(|(code, delta)| code == "trash_tier_1" && *delta > 0),
        "expected a tier-1 score: {tiered_entries:?}"
    );
    assert!(tiered_decision.allowed && untiered_decision.allowed);
    assert!(
        tiered_decision.preference_score > untiered_decision.preference_score,
        "tiered {} vs untiered {}\n  tiered: {tiered_entries:?}\n  untiered: {untiered_entries:?}",
        tiered_decision.preference_score,
        untiered_decision.preference_score,
    );
}

/// An uncompensated language penalty crosses the final score threshold.
#[test]
fn french_vf_vetoes_a_release_with_no_french_audio() {
    let english_only = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        category: Some("movie"),
        indexer_languages: &["English"],
        original_language: Some("en-US"),
    };

    let (entries, decision) = evaluate_locale_pack(FRENCH_VF, &english_only);
    assert!(
        entries.contains(&("trash_lang_not_french".to_string(), BLOCK_SCORE)),
        "{entries:?}"
    );
    assert!(!decision.allowed);
    assert!(
        decision
            .block_codes
            .iter()
            .any(|code| code == "score_at_or_below_block_threshold")
    );
}

/// The MULTi.VO pack wants the *original* audio out of a MULTi release, so a
/// French-dub-only release of a Japanese-original title is refused — the mirror
/// of the VF case above, and the reason the two packs are mutually exclusive.
#[test]
fn french_vo_vetoes_a_dub_that_dropped_the_original_audio() {
    let dub_only = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.MULTi.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        category: Some("movie"),
        indexer_languages: &["French"],
        original_language: Some("ja-JP"),
    };

    let (vo_entries, vo_decision) = evaluate_locale_pack(FRENCH_VO, &dub_only);
    assert!(
        vo_entries.contains(&("trash_lang_not_original".to_string(), BLOCK_SCORE)),
        "{vo_entries:?}"
    );
    assert!(!vo_decision.allowed);

    // The same release is fine for the VF pack, which prices
    // `language-not-original` at zero.
    let (_, vf_decision) = evaluate_locale_pack(FRENCH_VF, &dub_only);
    assert!(vf_decision.allowed);
    assert!(vf_decision.preference_score > vo_decision.preference_score);
}

/// VOSTFR is "original audio, French subtitles": the subbed release survives and
/// scores its marker, while a French dub of the same title is vetoed via
/// `language-not-original`.
#[test]
fn french_vostfr_keeps_the_subbed_release_and_vetoes_the_dub() {
    let subbed = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.SUBFRENCH.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        category: Some("movie"),
        indexer_languages: &["Japanese"],
        original_language: Some("ja-JP"),
    };
    let dubbed = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.FRENCH.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
        category: Some("movie"),
        indexer_languages: &["French"],
        original_language: Some("ja-JP"),
    };

    let (subbed_entries, subbed_decision) = evaluate_locale_pack(FRENCH_VOSTFR, &subbed);
    assert!(
        subbed_decision.allowed,
        "subbed release should survive: {subbed_entries:?}"
    );
    assert!(
        subbed_entries
            .iter()
            .any(|(code, delta)| code == "trash_french_vostfr" && *delta > 0),
        "expected the VOSTFR marker to score: {subbed_entries:?}"
    );

    let (dubbed_entries, dubbed_decision) = evaluate_locale_pack(FRENCH_VOSTFR, &dubbed);
    assert!(
        dubbed_entries.contains(&("trash_lang_not_original".to_string(), BLOCK_SCORE)),
        "{dubbed_entries:?}"
    );
    assert!(!dubbed_decision.allowed);
    assert!(subbed_decision.preference_score > dubbed_decision.preference_score);
}

/// The German family's `not-german-or-english` vetoes are −35000 upstream and
/// arrive as the sentinel, so a release in neither language is refused while a
/// German one is not.
///
/// Both cases run the *same* release name — BUTTERCUP is a German-guide
/// Movie/Web tier-1 group with no native `GROUP_RULES` entry — so the pack is
/// demonstrably active for both and the only variable is the reported audio.
#[test]
fn german_pack_vetoes_a_release_in_neither_german_nor_english() {
    const RAW: &str = "Der.Glaeserne.Hafen.2024.1080p.WEB-DL.DDP5.1.H.264-BUTTERCUP";
    let neither = LocaleCase {
        raw: RAW,
        category: Some("movie"),
        indexer_languages: &["Italian"],
        original_language: Some("it-IT"),
    };
    let german = LocaleCase {
        raw: RAW,
        category: Some("movie"),
        indexer_languages: &["German"],
        original_language: Some("de-DE"),
    };

    let (neither_entries, neither_decision) = evaluate_locale_pack(GERMAN, &neither);
    assert!(
        neither_entries.contains(&("trash_lang_not_german_or_english".to_string(), BLOCK_SCORE)),
        "{neither_entries:?}"
    );
    assert!(!neither_decision.allowed);

    let (german_entries, german_decision) = evaluate_locale_pack(GERMAN, &german);
    // Positive control: the pack ran and its gate is open for this release.
    assert!(
        german_entries
            .iter()
            .any(|(code, delta)| code == "trash_tier_1" && *delta > 0),
        "expected the German tier-1 group to score: {german_entries:?}"
    );
    assert!(
        !german_entries
            .iter()
            .any(|(code, delta)| code.starts_with("trash_lang") && *delta == BLOCK_SCORE),
        "German audio must satisfy the German-or-English requirement: {german_entries:?}"
    );
    assert!(german_decision.allowed);
    assert!(german_decision.preference_score > neither_decision.preference_score);
}

/// Regression pin for the curated language-membership rule: the Asian pack's guide
/// publishes no language formats, so its `language_stems` list is empty and the
/// base guide's English-centric vetoes must not reach it through the `default`
/// score fallback. A CJK-audio release is the case that would expose the leak.
#[test]
fn asian_pack_fires_no_language_veto_for_cjk_audio() {
    for (language, tag) in [
        ("Japanese", "ja-JP"),
        ("Korean", "ko-KR"),
        ("Chinese", "zh-CN"),
    ] {
        let case = LocaleCase {
            raw: "Umibe.Signal.2024.1080p.WEB-DL.AAC2.0.H.264-SHiNE",
            category: Some("movie"),
            indexer_languages: &[language],
            original_language: Some(tag),
        };
        let (entries, decision) = evaluate_locale_pack(ASIAN, &case);
        // Positive control: SHiNE is an Asian-guide tier-3 group, so the pack is
        // demonstrably running with an open gate on this release. A silently
        // inert pack would make the negative assertion below vacuous.
        assert!(
            entries
                .iter()
                .any(|(code, delta)| code == "trash_tier_3" && *delta > 0),
            "{language}: expected the Asian tier-3 group to score: {entries:?}"
        );
        assert!(
            !entries
                .iter()
                .any(|(code, _)| code.starts_with("trash_lang")),
            "the base guide's language vetoes leaked into the Asian pack for {language}: {entries:?}"
        );
        assert!(decision.allowed, "{language}: {entries:?}");
    }

    // The pack's rendered policy carries no language clause at all, which is the
    // membership decision itself rather than a downstream consequence of it.
    let asian = bundled_locale_template(ASIAN);
    assert!(!asian.rego_source.contains("trash_lang"));
}

/// SHiNE is an Asian-guide tier-3 group with no native `GROUP_RULES` entry, so
/// the pack alone separates these two.
#[test]
fn asian_pack_ranks_a_tiered_group_above_an_untiered_one() {
    let tiered = LocaleCase {
        raw: "Umibe.Signal.2024.1080p.WEB-DL.AAC2.0.H.264-SHiNE",
        category: Some("movie"),
        indexer_languages: &["Japanese"],
        original_language: Some("ja-JP"),
    };
    let untiered = LocaleCase {
        raw: "Umibe.Signal.2024.1080p.WEB-DL.AAC2.0.H.264-PortmereWorks",
        category: tiered.category,
        indexer_languages: tiered.indexer_languages,
        original_language: tiered.original_language,
    };

    let (tiered_entries, tiered_decision) = evaluate_locale_pack(ASIAN, &tiered);
    let (_, untiered_decision) = evaluate_locale_pack(ASIAN, &untiered);
    assert!(
        tiered_entries
            .iter()
            .any(|(code, delta)| code == "trash_tier_3" && *delta > 0),
        "{tiered_entries:?}"
    );
    assert!(tiered_decision.preference_score > untiered_decision.preference_score);
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) Service-token collisions
// ─────────────────────────────────────────────────────────────────────────────

fn detected_service(raw: &str) -> Option<String> {
    parse_release_metadata(raw)
        .streaming_service
        .map(|service| service.to_string())
}

/// §6a rule 2: a service format with no required specification gets a
/// WEB-adjacent token, because upstream would otherwise apply it to every WEB
/// release. These tokens are ordinary English words, so the corpus builds
/// invented titles around them.
///
/// The parser crate pins the same policy against a *targeted* parse context;
/// this runs the context-free `parse_release_metadata` entry point instead, and
/// covers `RED`, `FRIDAY` and the positive `IT` case that the targeted tests do
/// not.
#[test]
fn web_adjacent_service_tokens_only_name_a_service_next_to_a_web_marker() {
    let cases: &[(&str, Option<&str>)] = &[
        // NOW — the word, then the tag.
        (
            "Now.You.Return.2024.1080p.BluRay.DTS.5.1.x264-PortmereWorks",
            None,
        ),
        (
            "Copper.Kettle.2024.NOW.WEB-DL.1080p.H.264-PortmereWorks",
            Some("NOW"),
        ),
        // RED — YouTube Premium's tag upstream.
        (
            "Red.Kettle.Morning.2024.1080p.BluRay.DTS.5.1.x264-PortmereWorks",
            None,
        ),
        (
            "Copper.Kettle.2024.RED.WEB-DL.1080p.H.264-PortmereWorks",
            Some("YouTube Premium"),
        ),
        // FRIDAY — friDay Video's tag upstream.
        (
            "Friday.At.Glass.Harbor.2024.1080p.BluRay.DTS.5.1.x264-PortmereWorks",
            None,
        ),
        (
            "Copper.Kettle.2024.FRIDAY.WEB-DL.1080p.H.264-PortmereWorks",
            Some("friDay Video"),
        ),
        // IT — iTunes' tag upstream, and the title word that makes the bare
        // token unusable. The negative case is a WEB-DL, so a policy-free lookup
        // would tag it iTunes on the strength of a title word.
        (
            "It.Rains.In.Portmere.2024.1080p.WEB-DL.DDP5.1.H.264-PortmereWorks",
            None,
        ),
        (
            "Copper.Kettle.2024.IT.WEB-DL.1080p.H.264-PortmereWorks",
            Some("iTunes"),
        ),
    ];

    for (raw, expected) in cases {
        assert_eq!(
            detected_service(raw).as_deref(),
            *expected,
            "service detection drifted for `{raw}`"
        );
    }
}

/// §6a excludes `ma` outright: its upstream pattern relies on a negative
/// lookbehind against `DTS-HD MA`, which is not strippable (rule 3). So a
/// DTS-HD MA audio string ahead of a WEB marker must name no service, and the
/// audio must still parse.
#[test]
fn a_dts_hd_ma_audio_string_before_a_web_marker_names_no_service() {
    for raw in [
        "Glass.Harbor.2024.DTS-HD.MA.1080p.WEB-DL.H.264-FLUX",
        "Glass.Harbor.2024.DTS-HD.MA.5.1.1080p.WEB-DL.H.264-FLUX",
    ] {
        let parsed = parse_release_metadata(raw);
        assert_eq!(
            parsed.streaming_service.map(|service| service.to_string()),
            None,
            "`{raw}` must not resolve a service from the MA audio token"
        );
        assert_eq!(
            parsed.audio.map(|codec| codec.to_string()).as_deref(),
            Some("DTSMA"),
            "`{raw}` lost its audio codec"
        );
    }

    // The channel layout survives the same string.
    let parsed = parse_release_metadata("Glass.Harbor.2024.DTS-HD.MA.5.1.1080p.WEB-DL.H.264-FLUX");
    assert_eq!(parsed.audio_channels.as_deref(), Some("5.1"));
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) Veto dominance
// ─────────────────────────────────────────────────────────────────────────────

/// Nothing outranks a veto. The vetoed releases here are deliberately stacked —
/// 2160p, HDR, Dolby Vision, TrueHD Atmos 7.1, and in one case a gold-tier
/// group — while the allowed release is a bare 720p WEB-DL from an invented
/// group. The positive subtotals confirm the stacking is real before the
/// ordering assertion confirms it does not matter.
#[test]
fn any_vetoed_release_ranks_below_every_allowed_one() {
    let vetoed = [
        // Banned group: the veto rides on the release group alone.
        "Glass.Harbor.2024.2160p.WEB-DL.DV.HDR.TrueHD.Atmos.7.1.H.265-YIFY",
        // Theatrical source: gold group, top tier, still refused.
        "Glass.Harbor.2024.2160p.CAM.TrueHD.Atmos.7.1.H.265-FLUX",
    ];
    let allowed = [
        "Glass.Harbor.2024.720p.WEB-DL.H.264-PortmereWorks",
        "Glass.Harbor.2024.1080p.WEB-DL.DDP5.1.H.264-BLOOM",
    ];

    let positive_subtotal = |decision: &QualityProfileDecision| -> i32 {
        decision
            .scoring_log
            .iter()
            .filter(|entry| entry.delta > 0)
            .map(|entry| entry.delta)
            .sum()
    };

    // The veto must be what demotes these, not arithmetic. Proving that needs at
    // least one vetoed release that still out-stacks an allowed one on raw
    // positives — otherwise the ranking below would be satisfied by the release
    // simply being worse, and the veto could rot unnoticed.
    //
    // It is asserted over the set rather than every pair: quality tier no longer
    // contributes points, so a 2160p CAM no longer carries 3200 points into the
    // comparison and does not out-stack every allowed release any more.
    let mut veto_is_load_bearing = false;

    for raw in vetoed {
        let blocked = corpus_decision(raw, Some("movie"));
        assert!(
            !blocked.allowed,
            "`{raw}` should be blocked: {:?}",
            scoring_log(&blocked)
        );
        for other in allowed {
            let ok = corpus_decision(other, Some("movie"));
            assert!(ok.allowed, "`{other}`: {:?}", scoring_log(&ok));
            if positive_subtotal(&blocked) > positive_subtotal(&ok) {
                veto_is_load_bearing = true;
            }
            assert!(
                blocked.preference_score < ok.preference_score,
                "vetoed `{raw}` ({}) must rank below allowed `{other}` ({})\n  vetoed: {:?}",
                blocked.preference_score,
                ok.preference_score,
                scoring_log(&blocked),
            );
        }
    }

    assert!(
        veto_is_load_bearing,
        "no vetoed release out-stacked an allowed one on positives, so this test \
         would still pass with the veto removed"
    );
}

/// A bundled locale template's veto has the same dominance, which is the veto
/// contract's increase in blast radius made visible: the template is opt-in, and
/// once opted into it can refuse a release the builtin path was happy with.
#[test]
fn a_locale_pack_veto_sinks_a_release_the_builtin_path_allowed() {
    let case = LocaleCase {
        raw: "Le.Phare.De.Portmere.2024.2160p.WEB-DL.DV.HDR.TrueHD.Atmos.7.1.H.265-MOONLY",
        category: Some("movie"),
        indexer_languages: &["English"],
        original_language: Some("en-US"),
    };
    let builtin = corpus_decision(case.raw, case.category);
    assert!(builtin.allowed, "{:?}", scoring_log(&builtin));

    let (entries, decision) = evaluate_locale_pack(FRENCH_VF, &case);
    assert!(
        entries.contains(&("trash_lang_not_french".to_string(), BLOCK_SCORE)),
        "{entries:?}"
    );
    assert!(!decision.allowed);

    let floor = corpus_decision(
        "Glass.Harbor.2024.720p.WEB-DL.H.264-PortmereWorks",
        Some("movie"),
    );
    assert!(decision.preference_score < floor.preference_score);
}
