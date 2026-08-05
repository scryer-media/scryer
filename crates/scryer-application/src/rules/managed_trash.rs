use scryer_domain::MediaFacet;
use scryer_release_parser::TRASH_GUIDES_SOURCE_REVISION;

use crate::quality_profile::BLOCK_SCORE;
use crate::release_group_db::{
    TRASH_FACT_SCORES, TRASH_LANGUAGE_RULES, TRASH_SCORE_SET_VETO_MAGNITUDES, TrashLanguage,
    TrashLanguageRule,
};
use crate::trash_scores::normalize_trash_score;

pub(crate) const MANAGED_TRASH_KEY_PREFIX: &str = "trash-guides:locale:";
const MANAGED_TRASH_REGISTRY_VERSION: &str = "managed-trash-registry-v2";
const DEFAULT_SCORE_SET: &str = "default";

/// The upstream score sets a pack derives from: the movies/series set that
/// names the pack, and the anime set it pairs with.
#[derive(Debug, Clone, Copy)]
struct PackScoreSets {
    primary: &'static str,
    anime: &'static str,
    /// The language-rule stems this pack applies — its guide's curated list.
    ///
    /// Membership and pricing are different questions. An upstream profile
    /// includes a curated list of custom formats and its score set only prices
    /// them, so "the score resolves" must not imply "the pack applies it": the
    /// `default` fallback would hand every locale pack the base guide's
    /// English-centric vetoes, and a French pack that vetoes a French-audio-only
    /// release (`trash.lang.not_english`) is wrong on its face. A stem listed
    /// here can still price to 0 under this pack's sets and drop out.
    language_stems: &'static [&'static str],
}

/// The French guide's language rules. `language-original-plus-french` is listed
/// for completeness but is not distilled (its MULTi title spec is outside the
/// language-condition shape), so it never emits.
const FRENCH_LANGUAGE_STEMS: &[&str] = &[
    "language-not-french",
    "language-not-original",
    "language-original-plus-french",
];

/// The German guide's language rules; `language-not-original` prices to 0 under
/// the `german` set and drops out, which is upstream's intent.
const GERMAN_LANGUAGE_STEMS: &[&str] = &[
    "language-not-original",
    "not-german-or-english",
    "not-german-japanese-or-english",
    "not-german-japanese-korean-chinese-or-english",
];

const FRENCH_VF_SETS: PackScoreSets = PackScoreSets {
    primary: "french-multi-vf",
    anime: "french-anime-multi",
    language_stems: FRENCH_LANGUAGE_STEMS,
};
const FRENCH_VO_SETS: PackScoreSets = PackScoreSets {
    primary: "french-multi-vo",
    anime: "french-anime-multi",
    language_stems: FRENCH_LANGUAGE_STEMS,
};
const FRENCH_VOSTFR_SETS: PackScoreSets = PackScoreSets {
    primary: "french-vostfr",
    anime: "french-anime-vostfr",
    language_stems: FRENCH_LANGUAGE_STEMS,
};
const GERMAN_SETS: PackScoreSets = PackScoreSets {
    primary: "german",
    anime: "german-anime",
    language_stems: GERMAN_LANGUAGE_STEMS,
};
const ASIAN_SETS: PackScoreSets = PackScoreSets {
    primary: DEFAULT_SCORE_SET,
    anime: DEFAULT_SCORE_SET,
    // Upstream publishes no Asian-guide language rules; the base guide's
    // English-centric vetoes must not leak in through the `default` set.
    language_stems: &[],
};

pub(crate) struct ManagedTrashRulePack {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) applied_facets: &'static [MediaFacet],
    pub(crate) source: fn(Option<&[String]>) -> String,
}

impl ManagedTrashRulePack {
    pub(crate) fn source(&self, tag_filter: Option<&[String]>) -> String {
        (self.source)(tag_filter)
    }
}

pub(crate) fn managed_trash_rule_packs() -> &'static [ManagedTrashRulePack] {
    static PACKS: [ManagedTrashRulePack; 5] = [
        ManagedTrashRulePack {
            key: "trash-guides:locale:french-vf",
            name: "TRaSH Guides French (MULTi VF)",
            description: "Managed TRaSH Guides score-only locale pack for French dubbed audio intent.",
            applied_facets: &[],
            source: french_vf_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:french-vo",
            name: "TRaSH Guides French (MULTi VO)",
            description: "Managed TRaSH Guides score-only locale pack for original audio from MULTi French releases.",
            applied_facets: &[],
            source: french_vo_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:french-vostfr",
            name: "TRaSH Guides French (VOSTFR)",
            description: "Managed TRaSH Guides score-only locale pack for original audio with French subtitles.",
            applied_facets: &[],
            source: french_vostfr_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:german",
            name: "TRaSH Guides German Locale",
            description: "Managed TRaSH Guides score-only locale pack for German audio intent.",
            applied_facets: &[],
            source: german_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:asian",
            name: "TRaSH Guides Asian Locale",
            description: "Managed TRaSH Guides score-only locale pack for the locale:asian tag.",
            applied_facets: &[],
            source: asian_source,
        },
    ];

    &PACKS
}

/// Apps are tried in the order that owns the fact's facet, so a format scored
/// in only one app still resolves.
fn app_preference(code: &str) -> [&'static str; 2] {
    if code.contains("anime") {
        ["sonarr", "radarr"]
    } else {
        ["radarr", "sonarr"]
    }
}

/// Raw upstream score for a fact code under a pack's score sets.
///
/// Resolution mirrors upstream's own: the pack's set, then its anime set, then
/// `default` for the formats a locale set does not restate. Several upstream
/// formats collapse into one Scryer fact — a tier list ships per source context
/// — so the smallest magnitude wins, because the collapsed fact only carries
/// the weakest of those upstream claims.
fn upstream_score(sets: PackScoreSets, code: &str) -> Option<i64> {
    for score_set in [sets.primary, sets.anime, DEFAULT_SCORE_SET] {
        for app in app_preference(code) {
            let smallest = TRASH_FACT_SCORES
                .iter()
                .filter(|entry| {
                    entry.code == code && entry.score_set == score_set && entry.app == app
                })
                .map(|entry| entry.score)
                .min_by_key(|score| (score.abs(), *score));
            if smallest.is_some() {
                return smallest;
            }
        }
    }

    None
}

/// The cutoff the proportional band divides by is the pack's own score set,
/// because that is the quality profile the pack represents.
fn veto_magnitude(score_set: &str) -> i64 {
    TRASH_SCORE_SET_VETO_MAGNITUDES
        .iter()
        .find(|(name, _)| *name == score_set)
        .map_or(i64::from(BLOCK_SCORE).abs(), |(_, magnitude)| *magnitude)
}

/// A pack's score for a fact code, with the comment that records where it came
/// from. `native` applies only to codes upstream publishes no score for at all,
/// so a hand-picked number is always visible as such in the generated policy.
fn pack_score(sets: PackScoreSets, code: &str, native: i32) -> (i32, &'static str) {
    match upstream_score(sets, code) {
        Some(score) => (
            normalize_trash_score(score, veto_magnitude(sets.primary)),
            "",
        ),
        None => (native, "# scryer-native score\n"),
    }
}

// ---------------------------------------------------------------------------
// Language rules
// ---------------------------------------------------------------------------

/// The rego identifier a language rule code maps to. Codes are already
/// `[a-z_.]`, so the dots are all that need replacing.
fn language_rule_ident(code: &str) -> String {
    code.replace('.', "_")
}

/// The language rules a pack's score sets actually value, with the score each
/// resolves to.
///
/// Resolution is `pack_score`'s: the pack's own set, then its anime set, then
/// `default`. A rule the chain scores at zero is omitted rather than emitted as
/// a no-op, because zero is upstream's way of switching a format *off* for that
/// intent — `language-not-french` is a veto under `french-multi-vf` and exactly
/// zero under `french-multi-vo` and `french-vostfr`.
///
/// The `guide-only` rows never resolve, because `app_preference` names only the
/// two real apps. That is deliberate: upstream publishes those five formats
/// outside both app trees precisely because no shipped profile includes them,
/// and they contradict one another by design — `language-not-dutch` vetoes every
/// release without Dutch audio while `language-german-and-original` vetoes
/// German dual audio — so applying them together would refuse almost everything.
/// They are captured in `TRASH_LANGUAGE_RULES` for reference; what a pack
/// *applies* follows the per-app trees, which is where upstream ships policy.
fn pack_language_rules(sets: PackScoreSets) -> Vec<(&'static TrashLanguageRule, i32)> {
    let mut seen = Vec::<&str>::new();
    let mut selected = Vec::new();
    for rule in TRASH_LANGUAGE_RULES {
        // Membership first: only stems on this pack's curated list apply (see
        // `PackScoreSets::language_stems`); pricing below may still drop them.
        if !sets.language_stems.contains(&rule.stem) {
            continue;
        }
        if seen.contains(&rule.code) {
            // One row per app, and the generator fails the sync when the apps
            // disagree about a code's conditions, so the first row speaks for
            // all of them.
            continue;
        }
        seen.push(rule.code);
        let Some(upstream) = upstream_score(sets, rule.code) else {
            continue;
        };
        let score = normalize_trash_score(upstream, veto_magnitude(sets.primary));
        if score == 0 {
            continue;
        }
        selected.push((rule, score));
    }
    selected
}

/// One condition, as the rego expression that answers it.
///
/// `Original` reads `context.inferred_original_audio_language` rather than
/// `context.original_language`. Both are published, but
/// `inferred_original_audio_language` *is* `original_language` verbatim whenever
/// the metadata supplies it, with country inference and an eng/jpn default
/// behind it, so it is the field that always answers "what language was this
/// title made in". `original_language` is null for every title whose metadata
/// lacks it, which would make each Original clause silently inert on exactly the
/// titles it exists to judge.
fn language_condition_expression(language: TrashLanguage, negate: bool) -> String {
    let expression = match language {
        TrashLanguage::Named(code) => format!("has_audio_language(\"{code}\")"),
        TrashLanguage::Original => "has_original_audio_language".to_string(),
    };
    if negate {
        format!("not {expression}")
    } else {
        expression
    }
}

/// The rego for one language rule: a predicate carrying upstream's match
/// semantics, and the gated score entry that consumes it.
///
/// `SpecificationMatchesGroup.DidMatch` is "no required specification may fail,
/// and at least one specification must match". With at least one required
/// condition the second clause is already implied, so the predicate is their
/// conjunction and the optional conditions — which never gate — drop out. With
/// no required condition at all the rule reduces to the second clause, which is
/// a disjunction, so each condition gets its own body.
fn language_rule_source(rule: &TrashLanguageRule, score: i32) -> String {
    let ident = language_rule_ident(rule.code);
    let required = rule
        .conditions
        .iter()
        .filter(|condition| condition.required)
        .collect::<Vec<_>>();

    let predicate = if required.is_empty() {
        rule.conditions
            .iter()
            .map(|condition| {
                format!(
                    "{ident} if {{\n    {}\n}}",
                    language_condition_expression(condition.language, condition.negate)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        let body = required
            .iter()
            .map(|condition| {
                format!(
                    "    {}",
                    language_condition_expression(condition.language, condition.negate)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{ident} if {{\n{body}\n}}")
    };

    format!(
        "# {} ({})\n{predicate}\n\nscore_entry[\"{ident}\"] := {score} if {{\n    locale_intent\n    {ident}\n}}",
        rule.stem, rule.app,
    )
}

/// Every language clause a pack emits, plus the helpers they share.
fn language_rules_source(sets: PackScoreSets) -> String {
    let rules = pack_language_rules(sets);
    if rules.is_empty() {
        return String::new();
    }

    let helpers = r#"has_audio_language(value) if {
    some language in input.release.languages_audio
    lower(language) == value
}

has_original_audio_language if {
    some language in input.release.languages_audio
    lower(language) == lower(input.context.inferred_original_audio_language)
}"#;

    let clauses = rules
        .iter()
        .map(|(rule, score)| language_rule_source(rule, *score))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!("{helpers}\n\n{clauses}")
}

/// The `locale_intent` gate every score rule in a pack hangs off.
///
/// An enabled pack with no tag filter applies wherever its facts
/// match, which is how upstream ships these formats, so the gate is open. A tag
/// filter narrows the pack back down — it is the pack's own locale intent plus
/// the filter's tags, so adopting a pack's historical tag on upgrade leaves the
/// gate behaving exactly as it did before it became opt-in.
fn locale_intent_rules(tag_filter: Option<&[String]>, own_intent: &str) -> String {
    let Some(tags) = tag_filter.filter(|tags| !tags.is_empty()) else {
        return "locale_intent := true".to_string();
    };

    let tags = tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();
    let rendered = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
    let filter_rule = format!("locale_intent if {{\n    has_any_tag({rendered})\n}}");

    if own_intent.is_empty() {
        filter_rule
    } else {
        format!("{own_intent}\n\n{filter_rule}")
    }
}

fn source(
    sets: PackScoreSets,
    tag_filter: Option<&[String]>,
    own_locale_intent: &str,
    intent: &str,
    fact_prefix: &str,
    include_scene: bool,
    locale_rules: &str,
) -> String {
    let (tier_1, tier_1_note) = pack_score(sets, &format!("{fact_prefix}.group.tier1"), 120);
    let (tier_2, tier_2_note) = pack_score(sets, &format!("{fact_prefix}.group.tier2"), 60);
    let (tier_3, tier_3_note) = pack_score(sets, &format!("{fact_prefix}.group.tier3"), 20);
    let (lq, lq_note) = pack_score(sets, &format!("{fact_prefix}.lq"), -150);
    let scene_rule = include_scene.then(|| {
        let (scene, scene_note) = pack_score(sets, &format!("{fact_prefix}.scene"), -40);
        format!(
            r#"{scene_note}score_entry["trash_scene"] := {scene} if {{
    locale_intent
    has_fact("{fact_prefix}.scene")
}}"#
        )
    });
    format!(
        r#"# MANAGED_TRASH_REGISTRY_VERSION={MANAGED_TRASH_REGISTRY_VERSION}
# TRASH_GUIDES_SOURCE_REVISION={TRASH_GUIDES_SOURCE_REVISION}
# TRASH_SCORE_SET={primary}/{anime}
# This managed score-only policy is regenerated from the compiled locale-pack registry.
# Scores are normalized TRaSH Guides scores unless marked scryer-native.

has_any_tag(values) if {{
    some tag in input.context.tags
    some value in values
    lower(tag) == value
}}

{locale_intent}

{intent}

has_fact(value) if {{
    some fact in input.release.guide_facts
    lower(fact) == value
}}

{tier_1_note}score_entry["trash_tier_1"] := {tier_1} if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier1")
}}

{tier_2_note}score_entry["trash_tier_2"] := {tier_2} if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier2")
}}

{tier_3_note}score_entry["trash_tier_3"] := {tier_3} if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier3")
}}

{lq_note}score_entry["trash_lq"] := {lq} if {{
    locale_intent
    has_fact("{fact_prefix}.lq")
}}

{scene_rule}

{locale_rules}

{language_rules}
"#,
        primary = sets.primary,
        anime = sets.anime,
        locale_intent = locale_intent_rules(tag_filter, own_locale_intent),
        scene_rule = scene_rule.as_deref().unwrap_or_default(),
        language_rules = language_rules_source(sets),
    )
}

fn french_source(sets: PackScoreSets, tag_filter: Option<&[String]>) -> String {
    let (vostfr, vostfr_note) = pack_score(sets, "trash.locale.french.marker.vostfr", -100);
    // The regional pairs are Scryer's own preference: upstream detects VFF/VFQ
    // but publishes no score for them, because which French a user wants is the
    // pack choice itself.
    let (fr_fr_reference, fr_fr_reference_note) =
        pack_score(sets, "trash.locale.french.marker.vff", 40);
    let (fr_fr_quebec, fr_fr_quebec_note) = pack_score(sets, "trash.locale.french.marker.vfq", -20);
    let (fr_ca_reference, fr_ca_reference_note) =
        pack_score(sets, "trash.locale.french.marker.vff", -20);
    let (fr_ca_quebec, fr_ca_quebec_note) = pack_score(sets, "trash.locale.french.marker.vfq", 40);

    source(
        sets,
        tag_filter,
        r#"locale_intent if {
    has_required_audio(["fr", "fra", "fre", "french", "fr-fr", "fr-ca"])
}

locale_intent if {
    has_any_tag(["locale:fr", "locale:fr-fr", "locale:fr-ca"])
}"#,
        &format!(
            r#"has_required_audio(values) if {{
    some language in input.profile.required_audio_languages
    some value in values
    lower(language) == value
}}

fr_fr_intent if {{
    has_required_audio(["fr-fr"])
}}

fr_fr_intent if {{
    has_any_tag(["locale:fr-fr"])
}}

fr_ca_intent if {{
    has_required_audio(["fr-ca"])
}}

fr_ca_intent if {{
    has_any_tag(["locale:fr-ca"])
}}

{vostfr_note}score_entry["trash_french_vostfr"] := {vostfr} if {{
    locale_intent
    has_fact("trash.locale.french.marker.vostfr")
}}"#
        ),
        "trash.locale.french",
        true,
        &format!(
            r#"regional_reference if {{
    has_fact("trash.locale.french.marker.vff")
}}

regional_reference if {{
    has_fact("trash.locale.french.marker.vfi")
}}

regional_reference if {{
    has_fact("trash.locale.french.marker.vof")
}}

regional_quebec if {{
    has_fact("trash.locale.french.marker.vfq")
}}

regional_quebec if {{
    has_fact("trash.locale.french.marker.vq")
}}

regional_quebec if {{
    has_fact("trash.locale.french.marker.voq")
}}

{fr_fr_reference_note}score_entry["trash_french_fr_fr_reference"] := {fr_fr_reference} if {{
    fr_fr_intent
    regional_reference
}}

{fr_fr_quebec_note}score_entry["trash_french_fr_fr_quebec"] := {fr_fr_quebec} if {{
    fr_fr_intent
    regional_quebec
}}

{fr_ca_reference_note}score_entry["trash_french_fr_ca_reference"] := {fr_ca_reference} if {{
    fr_ca_intent
    regional_reference
}}

{fr_ca_quebec_note}score_entry["trash_french_fr_ca_quebec"] := {fr_ca_quebec} if {{
    fr_ca_intent
    regional_quebec
}}"#
        ),
    )
}

fn french_vf_source(tag_filter: Option<&[String]>) -> String {
    french_source(FRENCH_VF_SETS, tag_filter)
}

fn french_vo_source(tag_filter: Option<&[String]>) -> String {
    french_source(FRENCH_VO_SETS, tag_filter)
}

fn french_vostfr_source(tag_filter: Option<&[String]>) -> String {
    french_source(FRENCH_VOSTFR_SETS, tag_filter)
}

fn german_source(tag_filter: Option<&[String]>) -> String {
    let (subbed, subbed_note) = pack_score(GERMAN_SETS, "trash.locale.german.marker.subbed", -100);

    source(
        GERMAN_SETS,
        tag_filter,
        r#"locale_intent if {
    has_required_audio(["de", "deu", "ger", "german", "de-de"])
}

locale_intent if {
    has_any_tag(["locale:de", "locale:de-de"])
}"#,
        r#"has_required_audio(values) if {
    some language in input.profile.required_audio_languages
    some value in values
    lower(language) == value
}"#,
        "trash.locale.german",
        true,
        &format!(
            r#"{subbed_note}score_entry["trash_german_subbed"] := {subbed} if {{
    locale_intent
    has_fact("trash.locale.german.marker.subbed")
}}"#
        ),
    )
}

fn asian_source(tag_filter: Option<&[String]>) -> String {
    source(
        ASIAN_SETS,
        tag_filter,
        r#"locale_intent if {
    has_any_tag(["locale:asian"])
}"#,
        "",
        "trash.locale.asian",
        false,
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_group_db::TrashLanguageCondition;
    #[test]
    fn registry_has_stable_versioned_locale_keys() {
        let keys = managed_trash_rule_packs()
            .iter()
            .map(|pack| pack.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "trash-guides:locale:french-vf",
                "trash-guides:locale:french-vo",
                "trash-guides:locale:french-vostfr",
                "trash-guides:locale:german",
                "trash-guides:locale:asian",
            ]
        );
        assert!(
            managed_trash_rule_packs()[0]
                .source(None)
                .contains("MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v2")
        );
    }

    #[test]
    fn managed_packs_only_reference_locale_scene_facts_that_are_generated() {
        assert!(french_vf_source(None).contains("trash.locale.french.scene"));
        assert!(german_source(None).contains("trash.locale.german.scene"));
        assert!(!asian_source(None).contains("trash.locale.asian.scene"));
    }

    /// No filter means the pack applies wherever its facts match,
    /// so the gate is open and the pack's own locale branches drop out.
    #[test]
    fn an_unfiltered_pack_emits_an_open_gate() {
        for source in [
            french_vf_source(None),
            german_source(None),
            asian_source(None),
        ] {
            assert!(source.contains("locale_intent := true"), "{source}");
            assert!(!source.contains("locale_intent if {"), "{source}");
            // Every score rule still hangs off the gate, unchanged in shape.
            assert!(
                source.contains("score_entry[\"trash_tier_1\"]")
                    && source.contains("    locale_intent\n"),
                "{source}"
            );
        }
    }

    /// A filter narrows the pack back down. The pack's own locale intent is kept
    /// alongside it, so adopting a historical tag on upgrade leaves an existing
    /// install evaluating exactly the gate it evaluated before.
    #[test]
    fn a_filtered_pack_keeps_its_own_intent_and_adds_the_filter_tags() {
        let filter = ["Locale:French".to_string(), "locale:fr-classic".to_string()];
        let french = french_vf_source(Some(&filter));
        assert!(!french.contains("locale_intent := true"), "{french}");
        assert!(
            french.contains(
                r#"has_required_audio(["fr", "fra", "fre", "french", "fr-fr", "fr-ca"])"#
            ),
            "{french}"
        );
        assert!(
            french.contains(r#"has_any_tag(["locale:fr", "locale:fr-fr", "locale:fr-ca"])"#),
            "{french}"
        );
        assert!(
            french.contains(r#"has_any_tag(["locale:french","locale:fr-classic"])"#),
            "{french}"
        );

        // An empty filter is not a filter.
        assert_eq!(french_vf_source(Some(&[])), french_vf_source(None));

        let asian = asian_source(Some(&["locale:asian".to_string()]));
        assert!(
            asian.contains(r#"has_any_tag(["locale:asian"])"#),
            "{asian}"
        );
    }

    #[test]
    fn pack_scores_resolve_through_the_upstream_score_sets() {
        // FR tiers are scored in the pack's own set; the smallest of the
        // per-source-context entries wins.
        assert_eq!(
            upstream_score(FRENCH_VF_SETS, "trash.locale.french.group.tier1"),
            Some(1700)
        );
        // Radarr publishes no FR tier 3, so the Sonarr entry resolves it.
        assert_eq!(
            upstream_score(FRENCH_VF_SETS, "trash.locale.french.group.tier3"),
            Some(1600)
        );
        // The German family is scored only under `default`, which the fallback
        // reaches while the divisor stays the pack's own cutoff.
        assert_eq!(
            upstream_score(GERMAN_SETS, "trash.locale.german.group.tier1"),
            Some(2100)
        );
        assert_eq!(veto_magnitude("german"), 35_000);
        assert_eq!(veto_magnitude(DEFAULT_SCORE_SET), 10_000);
        // Regional markers ship unscored, so the pack keeps its native value.
        assert_eq!(
            upstream_score(FRENCH_VF_SETS, "trash.locale.french.marker.vff"),
            None
        );
    }

    #[test]
    fn locale_packs_emit_normalized_upstream_scores() {
        let french = french_vf_source(None);
        assert!(
            french.contains(r#"score_entry["trash_tier_1"] := 245 if"#),
            "{french}"
        );
        assert!(
            french.contains(r#"score_entry["trash_tier_2"] := 240 if"#),
            "{french}"
        );
        assert!(
            french.contains(r#"score_entry["trash_tier_3"] := 236 if"#),
            "{french}"
        );
        assert!(
            french.contains(r#"score_entry["trash_scene"] := 227 if"#),
            "{french}"
        );
        assert!(
            french.contains(r#"score_entry["trash_lq"] := -10000 if"#),
            "{french}"
        );
        assert!(
            !french.contains(r#"score_entry["trash_tier_1"] := 120"#),
            "{french}"
        );

        // VOSTFR intent neutralizes the French dub tiers and rewards the subbed
        // marker instead, which is exactly what its score set encodes.
        let vostfr = french_vostfr_source(None);
        assert!(
            vostfr.contains(r#"score_entry["trash_tier_1"] := 0 if"#),
            "{vostfr}"
        );
        assert!(
            vostfr.contains(r#"score_entry["trash_french_vostfr"] := 181 if"#),
            "{vostfr}"
        );

        let german = german_source(None);
        assert!(
            german.contains(r#"score_entry["trash_tier_1"] := 151 if"#),
            "{german}"
        );
        assert!(
            german.contains(r#"score_entry["trash_german_subbed"] := 329 if"#),
            "{german}"
        );

        let asian = asian_source(None);
        assert!(
            asian.contains(r#"score_entry["trash_tier_3"] := 100 if"#),
            "{asian}"
        );
    }

    #[test]
    fn scryer_native_scores_are_marked_and_upstream_scores_are_not() {
        let french = french_vf_source(None);
        assert!(
            french.contains(
                "# scryer-native score\nscore_entry[\"trash_french_fr_fr_reference\"] := 40 if"
            ),
            "{french}"
        );
        assert!(
            french.contains(
                "# scryer-native score\nscore_entry[\"trash_french_fr_ca_quebec\"] := 40 if"
            ),
            "{french}"
        );
        assert!(
            !french.contains("# scryer-native score\nscore_entry[\"trash_tier_1\"]"),
            "{french}"
        );
    }

    #[test]
    fn french_variants_declare_their_own_score_sets() {
        assert!(
            french_vf_source(None).contains("# TRASH_SCORE_SET=french-multi-vf/french-anime-multi")
        );
        assert!(
            french_vo_source(None).contains("# TRASH_SCORE_SET=french-multi-vo/french-anime-multi")
        );
        assert!(
            french_vostfr_source(None)
                .contains("# TRASH_SCORE_SET=french-vostfr/french-anime-vostfr")
        );
        assert!(german_source(None).contains("# TRASH_SCORE_SET=german/german-anime"));
        assert!(asian_source(None).contains("# TRASH_SCORE_SET=default/default"));
    }

    // -----------------------------------------------------------------------
    // Language rules
    // -----------------------------------------------------------------------

    fn language_entries(source: &str) -> Vec<(String, i32)> {
        let mut entries = source
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix(r#"score_entry["trash_lang_"#)?;
                let (code, rest) = rest.split_once("\"] := ")?;
                let (score, _) = rest.split_once(" if {")?;
                Some((code.to_string(), score.parse::<i32>().ok()?))
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn pack_source_for(key: &str) -> String {
        managed_trash_rule_packs()
            .iter()
            .find(|pack| pack.key == key)
            .unwrap_or_else(|| panic!("{key} should be a registered pack"))
            .source(None)
    }

    /// Upstream's score sets disagree about the language formats on purpose, and
    /// the disagreement is the whole point of shipping three French packs:
    /// `language-not-french` vetoes under MULTi.VF and is exactly zero under
    /// MULTi.VO and VOSTFR, while `language-not-original` is the mirror image.
    #[test]
    fn each_pack_emits_the_language_clauses_its_score_sets_value() {
        let expected: &[(&str, &[(&str, i32)])] = &[
            (
                "trash-guides:locale:french-vf",
                &[("not_french", BLOCK_SCORE)],
            ),
            (
                "trash-guides:locale:french-vo",
                &[("not_original", BLOCK_SCORE)],
            ),
            (
                "trash-guides:locale:french-vostfr",
                &[("not_original", BLOCK_SCORE)],
            ),
            (
                // The German family's vetoes are -35000 upstream. They normalize
                // to the veto sentinel rather than being scaled, which is what
                // keeps them inside the managed-entry contract.
                "trash-guides:locale:german",
                &[
                    ("not_german_japanese_korean_chinese_or_english", BLOCK_SCORE),
                    ("not_german_japanese_or_english", BLOCK_SCORE),
                    ("not_german_or_english", BLOCK_SCORE),
                ],
            ),
            // No Asian-guide language rules exist upstream, and the base guide's
            // English-centric vetoes must not leak in through `default`.
            ("trash-guides:locale:asian", &[]),
        ];

        for (key, clauses) in expected {
            let source = pack_source_for(key);
            let mut expected = clauses
                .iter()
                .map(|(code, score)| ((*code).to_string(), *score))
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(language_entries(&source), expected, "{key}");
        }

        // The French split, stated directly.
        assert!(
            pack_source_for("trash-guides:locale:french-vf")
                .contains(r#"score_entry["trash_lang_not_french"] := -10000"#)
        );
        for key in [
            "trash-guides:locale:french-vo",
            "trash-guides:locale:french-vostfr",
        ] {
            assert!(
                !pack_source_for(key).contains("trash_lang_not_french"),
                "{key} scores language-not-french at zero, so it must not emit the clause"
            );
        }
        // `language-not-original` is a veto under `default` and zero under both
        // `french-multi-vf` and `german`.
        assert!(
            !pack_source_for("trash-guides:locale:french-vf").contains("trash_lang_not_original")
        );
        assert!(!pack_source_for("trash-guides:locale:german").contains("trash_lang_not_original"));
        // Membership is the guide's curated list, not score resolvability: the
        // base guide's English veto belongs to no locale pack.
        for pack in managed_trash_rule_packs() {
            assert!(
                !pack.source(None).contains("trash_lang_not_english"),
                "{} must not inherit the base guide's not-english veto",
                pack.key
            );
        }
    }

    /// The five `guide-only` formats are captured but never applied: upstream
    /// publishes them outside both app trees because no shipped profile includes
    /// them, and they contradict each other by design.
    #[test]
    fn guide_only_language_rules_are_captured_but_never_applied() {
        let guide_only = TRASH_LANGUAGE_RULES
            .iter()
            .filter(|rule| rule.app == "guide-only")
            .map(|rule| rule.code)
            .collect::<Vec<_>>();
        assert_eq!(
            guide_only,
            vec![
                "trash.lang.german_and_original",
                "trash.lang.not_dutch",
                "trash.lang.not_original_or_german",
                "trash.lang.prefer_dutch",
                "trash.lang.prefer_german",
            ]
        );

        for code in &guide_only {
            for sets in [
                FRENCH_VF_SETS,
                FRENCH_VO_SETS,
                FRENCH_VOSTFR_SETS,
                GERMAN_SETS,
                ASIAN_SETS,
            ] {
                assert_eq!(upstream_score(sets, code), None, "{code}");
            }
        }

        for pack in managed_trash_rule_packs() {
            let source = pack.source(None);
            for code in &guide_only {
                assert!(
                    !source.contains(&language_rule_ident(code)),
                    "{} must not apply {code}",
                    pack.key
                );
            }
        }
    }

    /// Every language clause hangs off the same gate as the score rules, so a
    /// filtered pack narrows them exactly as it narrows everything else.
    #[test]
    fn language_clauses_are_gated_on_locale_intent() {
        let filter = ["locale:french".to_string()];
        let source = french_vf_source(Some(&filter));
        assert!(!source.contains("locale_intent := true"), "{source}");

        for line in source.lines() {
            if let Some(rest) = line.strip_prefix(r#"score_entry["trash_lang_"#) {
                let entry = rest.split_once("\"]").unwrap().0;
                assert!(
                    source.contains(&format!(
                        "score_entry[\"trash_lang_{entry}\"] := -10000 if {{\n    locale_intent\n"
                    )),
                    "trash_lang_{entry} must be gated on locale_intent"
                );
            }
        }
    }

    /// `Original` is answered by `inferred_original_audio_language` rather than
    /// `original_language`: the two agree whenever the metadata knows the
    /// answer, and only the inferred field is populated when it does not, so
    /// reading the nullable one would make the clause inert on exactly the
    /// titles it exists to judge.
    #[test]
    fn the_original_condition_reads_the_inferred_original_audio_language() {
        let source = french_vostfr_source(None);
        assert!(
            source.contains(
                "has_original_audio_language if {\n    some language in input.release.languages_audio\n    lower(language) == lower(input.context.inferred_original_audio_language)\n}"
            ),
            "{source}"
        );
        assert!(
            source.contains("trash_lang_not_original if {\n    not has_original_audio_language\n}"),
            "{source}"
        );
        assert!(
            !source.contains("input.context.original_language"),
            "{source}"
        );
    }

    /// Upstream's `DidMatch` is "no required specification may fail, and at
    /// least one must match", so an all-required format is a conjunction and an
    /// all-optional one is a disjunction.
    #[test]
    fn did_match_semantics_survive_into_the_rego() {
        let conjunction = TRASH_LANGUAGE_RULES
            .iter()
            .find(|rule| rule.code == "trash.lang.not_german_or_english" && rule.app == "radarr")
            .unwrap();
        assert!(
            conjunction
                .conditions
                .iter()
                .all(|condition| condition.required)
        );
        assert!(
            language_rule_source(conjunction, BLOCK_SCORE).contains(
                "trash_lang_not_german_or_english if {\n    not has_audio_language(\"deu\")\n    not has_audio_language(\"eng\")\n}"
            ),
            "{}",
            language_rule_source(conjunction, BLOCK_SCORE)
        );

        // A hypothetical all-optional multi-condition format becomes one body
        // per condition, which is rego's disjunction.
        let disjunction = TrashLanguageRule {
            code: "trash.lang.prefer_nordic",
            app: "guide-only",
            stem: "language-prefer-nordic",
            conditions: &[
                TrashLanguageCondition {
                    language: TrashLanguage::Named("dan"),
                    negate: false,
                    required: false,
                },
                TrashLanguageCondition {
                    language: TrashLanguage::Named("swe"),
                    negate: false,
                    required: false,
                },
            ],
        };
        let rendered = language_rule_source(&disjunction, 10);
        assert!(
            rendered.contains(
                "trash_lang_prefer_nordic if {\n    has_audio_language(\"dan\")\n}\n\ntrash_lang_prefer_nordic if {\n    has_audio_language(\"swe\")\n}"
            ),
            "{rendered}"
        );

        // Optional conditions never gate, so they drop out once something is
        // required.
        let mixed = TrashLanguageRule {
            code: "trash.lang.mixed",
            app: "guide-only",
            stem: "language-mixed",
            conditions: &[
                TrashLanguageCondition {
                    language: TrashLanguage::Named("deu"),
                    negate: false,
                    required: true,
                },
                TrashLanguageCondition {
                    language: TrashLanguage::Named("swe"),
                    negate: false,
                    required: false,
                },
            ],
        };
        let rendered = language_rule_source(&mixed, 10);
        assert!(
            rendered.contains("has_audio_language(\"deu\")"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("has_audio_language(\"swe\")"),
            "{rendered}"
        );
    }

    /// Every emitted language score has to satisfy the managed-entry contract:
    /// exactly the veto sentinel, or inside the ranking band.
    #[test]
    fn language_scores_stay_inside_the_managed_entry_contract() {
        for pack in managed_trash_rule_packs() {
            for (code, score) in language_entries(&pack.source(None)) {
                assert!(
                    score == BLOCK_SCORE
                        || (scryer_rules::MANAGED_POLICY_MIN_SCORE
                            ..=scryer_rules::MANAGED_POLICY_MAX_SCORE)
                            .contains(&score),
                    "{}/{code} scored {score}",
                    pack.key
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rego evaluation, through the real engine, on invented titles only.
    // -----------------------------------------------------------------------

    fn fake_profile(required_audio_languages: &[&str]) -> crate::QualityProfile {
        crate::QualityProfile {
            id: "fake-profile".to_string(),
            name: "Fake Profile".to_string(),
            criteria: crate::QualityProfileCriteria {
                quality_tiers: vec!["1080P".to_string()],
                archival_quality: None,
                allow_unknown_quality: false,
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: required_audio_languages
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                scoring_persona: crate::ScoringPersona::Balanced,
                scoring_overrides: crate::ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                facet_persona_overrides: std::collections::HashMap::new(),
            },
        }
    }

    /// An invented release, run through the real rule-input builder so the
    /// language codes the clauses compare against are the ones production
    /// publishes rather than hand-written strings.
    fn fake_release_input(
        indexer_languages: &[&str],
        original_language: Option<&str>,
    ) -> scryer_rules::UserRuleInput {
        let indexer_languages = indexer_languages
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        crate::rules::user_rule_input::build_rule_input(
            &crate::parse_release_metadata("Faux.Harbour.Lights.2024.1080p.WEB-DL.H.264-FAKEGRP"),
            &fake_profile(&[]),
            &crate::QualityProfileDecision {
                release_score: 1000,
                scoring_log: vec![],
                allowed: true,
                block_codes: vec![],
                preference_score: 1000,
            },
            crate::rules::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: Some(4_000_000_000),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&indexer_languages),
            },
            crate::rules::user_rule_input::RuleContextInfo {
                title_id: Some("fake-title"),
                library_name: Some("Movies"),
                category: Some("movie"),
                original_language,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: Some(101),
                is_filler: false,
            },
            None,
        )
    }

    fn evaluate_pack(key: &str, input: &scryer_rules::UserRuleInput) -> Vec<(String, i32)> {
        let pack = managed_trash_rule_packs()
            .iter()
            .find(|pack| pack.key == key)
            .unwrap();
        let id = key.replace([':', '-'], "_");
        let policy = scryer_rules::UserPolicy {
            id: id.clone(),
            name: pack.name.to_string(),
            rego_source: scryer_rules::rewrite_package_declaration(&pack.source(None), &id),
            origin: scryer_rules::PolicyOrigin::System,
            applied_facets: vec![],
        };
        let result = scryer_rules::UserRulesEngine::build(&[policy])
            .expect("managed pack should compile")
            .evaluator()
            .evaluate(input, "movie")
            .expect("evaluation should succeed");
        assert!(result.errors.is_empty(), "{key}: {:?}", result.errors);
        let mut entries = result
            .entries
            .iter()
            .map(|entry| (entry.code.clone(), entry.delta))
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    /// The French packs' whole reason for existing: MULTi.VF wants French
    /// audio, so an original-audio-only release is refused; VOSTFR wants the
    /// original audio, so the same release is fine.
    #[test]
    fn french_vf_vetoes_an_original_audio_release_that_vostfr_accepts() {
        let original_audio_only = fake_release_input(&["English"], Some("en-US"));
        assert_eq!(original_audio_only.release.languages_audio, vec!["eng"]);
        assert_eq!(
            original_audio_only.context.inferred_original_audio_language,
            "eng"
        );

        assert_eq!(
            evaluate_pack("trash-guides:locale:french-vf", &original_audio_only),
            vec![("trash_lang_not_french".to_string(), BLOCK_SCORE)]
        );
        assert_eq!(
            evaluate_pack("trash-guides:locale:french-vostfr", &original_audio_only),
            Vec::<(String, i32)>::new()
        );
    }

    /// The mirror image, which isolates the `Original` condition: a release
    /// carrying French *and* English audio for a Japanese-original title passes
    /// MULTi.VF and is refused by VOSTFR, because VOSTFR is the pack that scores
    /// `language-not-original` as a veto.
    #[test]
    fn vostfr_vetoes_a_release_missing_the_titles_original_audio() {
        let dubbed = fake_release_input(&["French", "English"], Some("ja-JP"));
        assert_eq!(dubbed.release.languages_audio, vec!["fra", "eng"]);
        assert_eq!(dubbed.context.inferred_original_audio_language, "jpn");

        assert_eq!(
            evaluate_pack("trash-guides:locale:french-vf", &dubbed),
            Vec::<(String, i32)>::new()
        );
        assert_eq!(
            evaluate_pack("trash-guides:locale:french-vostfr", &dubbed),
            vec![("trash_lang_not_original".to_string(), BLOCK_SCORE)]
        );
    }

    /// The German family's -35000 vetoes reach the engine as the sentinel, and
    /// they are satisfied by German audio rather than by English alone.
    #[test]
    fn the_german_pack_refuses_a_release_in_neither_german_nor_english() {
        let italian = fake_release_input(&["Italian"], Some("it-IT"));
        assert_eq!(italian.release.languages_audio, vec!["ita"]);

        let entries = evaluate_pack("trash-guides:locale:german", &italian);
        assert!(
            entries.contains(&("trash_lang_not_german_or_english".to_string(), BLOCK_SCORE)),
            "{entries:?}"
        );

        let german = fake_release_input(&["German"], Some("de-DE"));
        assert_eq!(german.release.languages_audio, vec!["deu"]);
        assert!(
            !evaluate_pack("trash-guides:locale:german", &german)
                .iter()
                .any(|(code, _)| code == "trash_lang_not_german_or_english"),
            "German audio satisfies the German-or-English requirement"
        );
    }
}
