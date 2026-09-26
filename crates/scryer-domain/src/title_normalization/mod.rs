//! Language-specific transliteration folds for title matching.
//!
//! A catalog and a release group can each write one foreign name in a
//! different romanization. Each language's rules live in their own file and
//! implement [`LanguageRules`]; [`RULES`] registers them. A catalog name
//! carries a language tag, so it is folded by the one rule set its tag
//! selects ([`romanization_key`]): the persisted title search index and
//! [`compare_title_spelling`](crate::title_spelling::compare_title_spelling)
//! use that. A release name carries no language, so it is folded by every
//! rule set that reads its script ([`release_romanization_keys`]), and the
//! release matchers look each key up.
//!
//! The index persists these keys, so
//! [`title_collation_data_version`](crate::title_spelling::title_collation_data_version)
//! hashes every registered rule set's tag and its probes' folds. A rule change
//! moves the fingerprint and rebuilds the index once.

pub(crate) mod japanese;

use crate::title_spelling::{TitleScript, title_script};

pub use japanese::JAPANESE_ROMANIZATION_TAG;

/// One language's transliteration fold.
///
/// A new language needs three things: a new file in this module holding a
/// unit struct that implements this trait, a `mod` line for that file here,
/// and one entry in [`RULES`]. Nothing else changes: the catalog index, the
/// release matchers and the fingerprint all read [`RULES`]. The file must
/// provide:
///
/// * [`equivalence_tag`](Self::equivalence_tag): a BCP 47 tag, distinct from
///   every other registered rule set's, reported when two spellings agree only
///   under this fold (the Japanese rules report `ja-latn`).
/// * [`applies_to`](Self::applies_to): which catalog language tags select these
///   rules. It must not accept a tag another registered rule set accepts; the
///   first match in [`RULES`] wins.
/// * [`sample_tags`](Self::sample_tags): tags `applies_to` accepts. This
///   module's tests refuse a registry where one rule set accepts another's
///   sample tag.
/// * [`script`](Self::script): the script the fold reads. A name in any other
///   script is never folded.
/// * [`fold`](Self::fold): the comparison key of a name already in
///   [`title_lookup_form`](crate::title_spelling::title_lookup_form). It must
///   be deterministic, and it must only equate spellings its doc comment
///   documents as one key. It may equate distinct names only where its doc
///   comment lists them as known collisions, relying on the caller's
///   competitor check, which proves no other library identity answers to a
///   key before trusting it.
/// * [`probes`](Self::probes): a non-empty list of names such that turning
///   off any one rule of the fold changes at least one probe's key. The index
///   fingerprint hashes these keys, so a rule change that no probe notices
///   would leave installed indexes keyed by the old rules. A rule set with no
///   probes is rejected by this module's tests; the file's own tests prove
///   the rest by turning each rule off in turn and checking that the
///   fingerprint moves.
///
/// The new file also carries its own tests: one positive and one negative per
/// rule, with invented names only.
pub trait LanguageRules: Sync {
    fn equivalence_tag(&self) -> &'static str;
    fn applies_to(&self, language: &str) -> bool;
    fn sample_tags(&self) -> &'static [&'static str];
    fn script(&self) -> TitleScript;
    fn fold(&self, value: &str) -> String;
    fn probes(&self) -> &'static [&'static str];
}

/// Every registered rule set, in selection order.
pub static RULES: &[&dyn LanguageRules] = &[&japanese::JapaneseRomanization];

/// The rule set a catalog language tag selects, if any.
pub fn rules_for(language: Option<&str>) -> Option<&'static dyn LanguageRules> {
    select(RULES, language)
}

fn select<'a>(
    registry: &[&'a dyn LanguageRules],
    language: Option<&str>,
) -> Option<&'a dyn LanguageRules> {
    let language = language?;
    registry
        .iter()
        .copied()
        .find(|rules| rules.applies_to(language))
}

/// Why a registry breaks the [`LanguageRules`] contract, if it does: two rule
/// sets sharing an equivalence tag, a rule set without probes, a rule set
/// refusing its own sample tag, or two rule sets accepting one catalog tag.
#[cfg(test)]
fn contract_violation(registry: &[&dyn LanguageRules]) -> Option<String> {
    let mut tags = std::collections::HashSet::new();
    for (index, rules) in registry.iter().enumerate() {
        let tag = rules.equivalence_tag();
        if !tags.insert(tag) {
            return Some(format!("{tag}: tag registered twice"));
        }
        if rules.probes().is_empty() {
            return Some(format!("{tag}: no probes"));
        }
        if rules.sample_tags().is_empty() {
            return Some(format!("{tag}: no sample tags"));
        }
        for sample in rules.sample_tags() {
            if !rules.applies_to(sample) {
                return Some(format!("{tag}: refuses its own sample tag {sample:?}"));
            }
            if let Some(other) = registry
                .iter()
                .enumerate()
                .find(|(other, others)| *other != index && others.applies_to(sample))
            {
                return Some(format!(
                    "{tag}: sample tag {sample:?} is also accepted by {}",
                    other.1.equivalence_tag()
                ));
            }
        }
    }
    None
}

/// The comparison-only romanization key of a name, or `None` when the language
/// tag selects no rule set or the name is not in that rule set's script.
/// Indexes that need to find every spelling of a name must key on this as well
/// as on the literal and collation keys: romanization variance is not a
/// bounded edit distance, so a Levenshtein-shaped candidate filter can miss it.
pub fn romanization_key(value: &str, language: Option<&str>) -> Option<String> {
    let rules = rules_for(language)?;
    (title_script(value) == rules.script()).then(|| rules.fold(value))
}

/// The romanization keys of a release name, which carries no language tag:
/// one per registered rule set that reads the name's script, without
/// duplicates. With one registered rule set this is at most one key.
pub fn release_romanization_keys(value: &str) -> Vec<String> {
    release_keys(RULES, value)
}

fn release_keys(registry: &[&dyn LanguageRules], value: &str) -> Vec<String> {
    let script = title_script(value);
    let mut keys = Vec::new();
    for rules in registry {
        if rules.script() == script {
            let key = rules.fold(value);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Whether a reported spelling equivalence came from a romanization fold
/// rather than from a collation profile.
pub fn is_romanization_tag(tag: &str) -> bool {
    RULES.iter().any(|rules| rules.equivalence_tag() == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second language, registered the way a sister file would be: a unit
    /// struct implementing the trait and one more entry in the registry.
    /// Its fold reads Latin names too, so release names get a second key.
    struct Sister;

    impl LanguageRules for Sister {
        fn equivalence_tag(&self) -> &'static str {
            "zz-latn"
        }
        fn applies_to(&self, language: &str) -> bool {
            language.eq_ignore_ascii_case("zz")
        }
        fn sample_tags(&self) -> &'static [&'static str] {
            &["zz"]
        }
        fn script(&self) -> TitleScript {
            TitleScript::Latin
        }
        fn fold(&self, value: &str) -> String {
            value.replace('q', "k")
        }
        fn probes(&self) -> &'static [&'static str] {
            &["qara no qumo"]
        }
    }

    static WITH_SISTER: &[&dyn LanguageRules] = &[&japanese::JapaneseRomanization, &Sister];

    #[test]
    fn a_sister_language_needs_a_file_a_mod_line_and_a_registry_entry() {
        assert_eq!(contract_violation(WITH_SISTER), None);
        assert_eq!(
            select(WITH_SISTER, Some("zz")).map(|rules| rules.equivalence_tag()),
            Some("zz-latn")
        );
        assert_eq!(
            select(WITH_SISTER, Some("x-jat")).map(|rules| rules.equivalence_tag()),
            Some(JAPANESE_ROMANIZATION_TAG)
        );
        assert!(select(WITH_SISTER, Some("en")).is_none());
        assert!(select(WITH_SISTER, None).is_none());
        // A release name is folded by both, so either catalog key is reachable.
        assert_eq!(
            release_keys(WITH_SISTER, "qara no qumo"),
            vec!["qaranoqumo".to_string(), "kara no kumo".to_string()]
        );
        // Keys two rule sets agree on are looked up once.
        assert_eq!(
            release_keys(WITH_SISTER, "hoshi kage"),
            vec!["hoshikage".to_string(), "hoshi kage".to_string()]
        );
        assert_eq!(
            release_keys(WITH_SISTER, "hoshikage"),
            vec!["hoshikage".to_string()]
        );
        assert!(release_keys(WITH_SISTER, "ほしかげ").is_empty());
    }

    #[test]
    fn two_rule_sets_may_not_accept_one_catalog_tag() {
        struct Greedy(&'static str);
        impl LanguageRules for Greedy {
            fn equivalence_tag(&self) -> &'static str {
                "zx-latn"
            }
            fn applies_to(&self, language: &str) -> bool {
                language.eq_ignore_ascii_case(self.0) || language == "zx"
            }
            fn sample_tags(&self) -> &'static [&'static str] {
                &["zx"]
            }
            fn script(&self) -> TitleScript {
                TitleScript::Latin
            }
            fn fold(&self, value: &str) -> String {
                value.to_string()
            }
            fn probes(&self) -> &'static [&'static str] {
                &["qara"]
            }
        }
        for claimed in JAPANESE_ROMANIZATION_SAMPLES {
            let greedy = Greedy(claimed);
            let registry: &[&dyn LanguageRules] = &[&japanese::JapaneseRomanization, &greedy];
            assert!(
                contract_violation(registry).is_some_and(|reason| reason.contains(claimed)),
                "{claimed}"
            );
        }
        // And the other way round: Japanese must not accept a sister's tag.
        let sister_claims_ja = Greedy("zz");
        let registry: &[&dyn LanguageRules] = &[&Sister, &sister_claims_ja];
        assert!(contract_violation(registry).is_some());
        let registry: &[&dyn LanguageRules] = &[&japanese::JapaneseRomanization, &Greedy("zq")];
        assert_eq!(contract_violation(registry), None);
    }

    const JAPANESE_ROMANIZATION_SAMPLES: [&str; 4] = ["ja", "jpn", "ja-latn", "x-jat"];

    #[test]
    fn the_japanese_sample_tags_are_the_catalog_tags_it_selects() {
        assert_eq!(
            japanese::JapaneseRomanization.sample_tags(),
            JAPANESE_ROMANIZATION_SAMPLES
        );
    }

    #[test]
    fn every_registered_rule_set_honours_the_contract() {
        assert_eq!(contract_violation(RULES), None);
    }

    #[test]
    fn a_rule_set_without_probes_is_refused() {
        struct Silent;
        impl LanguageRules for Silent {
            fn equivalence_tag(&self) -> &'static str {
                "zy-latn"
            }
            fn applies_to(&self, language: &str) -> bool {
                language == "zy"
            }
            fn sample_tags(&self) -> &'static [&'static str] {
                &["zy"]
            }
            fn script(&self) -> TitleScript {
                TitleScript::Latin
            }
            fn fold(&self, value: &str) -> String {
                value.replace('q', "k")
            }
            fn probes(&self) -> &'static [&'static str] {
                &[]
            }
        }
        let registry: &[&dyn LanguageRules] = &[&japanese::JapaneseRomanization, &Silent];
        assert_eq!(
            contract_violation(registry),
            Some("zy-latn: no probes".to_string())
        );
        let doubled: &[&dyn LanguageRules] = &[&Sister, &Sister];
        assert!(contract_violation(doubled).is_some());
    }

    #[test]
    fn romanization_keys_need_a_selecting_tag_and_the_rule_script() {
        assert_eq!(
            romanization_key("hoshi kage no machi", Some("x-jat")).as_deref(),
            Some("hoshikagenomachi")
        );
        assert_eq!(romanization_key("hoshi kage no machi", Some("en")), None);
        assert_eq!(romanization_key("hoshi kage no machi", None), None);
        assert_eq!(romanization_key("ほしかげ", Some("ja")), None);
        assert_eq!(
            release_romanization_keys("hoshi kage no machi"),
            vec!["hoshikagenomachi".to_string()]
        );
        assert!(release_romanization_keys("ほしかげ").is_empty());
        assert!(is_romanization_tag(JAPANESE_ROMANIZATION_TAG));
        assert!(!is_romanization_tag("ja"));
    }
}
