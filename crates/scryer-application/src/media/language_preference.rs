//! Preferred and optional audio languages: ranking, never gating.
//!
//! Required languages (`audio.required_languages`) reject a release that lacks
//! any of them. The two lists here only order releases that are otherwise
//! eligible:
//!
//! - **preferred** is an ordered list. A release's *preferred rank* is the
//!   position of the first preferred language it carries; a release carrying
//!   none ranks after every one that does.
//! - **optional** languages are counted. More is better, none is acceptable.
//!
//! Where each comparison sits relative to the quality tier and the revision is
//! the quality profile's decision pipeline, not a property of the languages:
//! see [`crate::decision_pipeline`].
//!
//! Empty lists give every release the same [`AudioLanguageMatch`], so every ordering
//! and admission comparison degrades to the behaviour without preferences.

use crate::audio_requirements::{
    normalize_required_audio_languages, resolve_required_audio_requirements,
    resolved_track_languages,
};

/// The resolved preferences for one title: concrete, normalized language codes
/// with the `original` selector already expanded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguagePreferences {
    pub preferred: Vec<String>,
    pub optional: Vec<String>,
    /// Whether a file still missing optional languages keeps its title wanted.
    pub optional_keep_searching: bool,
}

/// Preferences that rank nothing, for contexts built by hand in tests.
#[cfg(test)]
pub(crate) static NO_LANGUAGE_PREFERENCES: AudioLanguagePreferences = AudioLanguagePreferences {
    preferred: Vec::new(),
    optional: Vec::new(),
    optional_keep_searching: false,
};

#[cfg(test)]
impl AudioLanguageRank {
    /// A rank with no preferences behind it, for tests that only compare ranks.
    /// It is wanted whenever a better preferred language exists.
    pub(crate) fn for_test(preferred_rank: u16, optional_hits: u16) -> Self {
        Self {
            matched: AudioLanguageMatch {
                preferred_rank,
                optional_hits,
            },
            wanted: preferred_rank > 0,
            preferred_wanted: preferred_rank > 0,
            ..Default::default()
        }
    }
}

/// Where one release or file stands against [`AudioLanguagePreferences`].
///
/// Both fields are oriented for comparison: a lower `preferred_rank` is better,
/// a higher `optional_hits` is better.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguageMatch {
    pub preferred_rank: u16,
    pub optional_hits: u16,
}

/// A [`AudioLanguageMatch`] together with what the preferences still want.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguageRank {
    pub matched: AudioLanguageMatch,
    /// Length of the preferred list; `matched.preferred_rank` equals it when
    /// none of the preferred languages is present. Display only, never ranked.
    pub preferred_count: u16,
    /// The preferences still want something this release or file lacks; see
    /// [`AudioLanguagePreferences::is_satisfied`]. A file with this set still
    /// reaches its cutoff, but a release in a better language can pass it.
    pub wanted: bool,
    /// Part of `wanted`: a preferred language exists that this release or
    /// file does not carry at the top rank.
    pub preferred_wanted: bool,
    /// Part of `wanted`: the preferences keep searching for optional languages
    /// and this release or file lacks some of them.
    pub optional_wanted: bool,
}

impl AudioLanguageRank {
    pub(crate) fn new(preferences: &AudioLanguagePreferences, matched: AudioLanguageMatch) -> Self {
        Self {
            matched,
            preferred_count: saturating_u16(preferences.preferred.len()),
            wanted: !preferences.is_satisfied(matched),
            preferred_wanted: preferences.preferred_wanted(matched),
            optional_wanted: preferences.optional_wanted(matched),
        }
    }
}

impl AudioLanguagePreferences {
    /// Normalize configured lists, expanding `original` for the title.
    ///
    /// A language listed as preferred is dropped from the optional list so it
    /// cannot be counted twice.
    pub(crate) fn resolve(
        preferred: &[String],
        optional: &[String],
        optional_keep_searching: bool,
        title_context: &crate::audio_requirements::TitleAudioLanguageContext,
    ) -> Self {
        let preferred = resolve_required_audio_requirements(preferred, title_context);
        let optional = resolve_required_audio_requirements(optional, title_context)
            .into_iter()
            .filter(|language| !preferred.contains(language))
            .collect();
        Self {
            preferred,
            optional,
            optional_keep_searching,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.preferred.is_empty() && self.optional.is_empty()
    }

    /// Rank a set of audio languages. Codes are normalized before matching, so
    /// `de`, `ger` and `deu` are the same language.
    pub fn evaluate(&self, languages: &[String]) -> AudioLanguageMatch {
        if self.is_empty() {
            return AudioLanguageMatch::default();
        }
        let languages = normalize_required_audio_languages(languages.iter().cloned());
        let preferred_rank = self
            .preferred
            .iter()
            .position(|language| languages.contains(language))
            .unwrap_or(self.preferred.len());
        let optional_hits = self
            .optional
            .iter()
            .filter(|language| languages.contains(language))
            .count();
        AudioLanguageMatch {
            preferred_rank: saturating_u16(preferred_rank),
            optional_hits: saturating_u16(optional_hits),
        }
    }

    /// Whether a file with this match has nothing left to gain from another
    /// search on language grounds.
    ///
    /// A missing preferred language always keeps searching. Missing optional
    /// languages only do when [`Self::optional_keep_searching`] is set; without
    /// it, an RSS or manual result that carries more of them still upgrades the
    /// file, but none is sought.
    pub fn is_satisfied(&self, matched: AudioLanguageMatch) -> bool {
        !self.preferred_wanted(matched) && !self.optional_wanted(matched)
    }

    fn preferred_wanted(&self, matched: AudioLanguageMatch) -> bool {
        !self.preferred.is_empty() && matched.preferred_rank > 0
    }

    fn optional_wanted(&self, matched: AudioLanguageMatch) -> bool {
        self.optional_keep_searching && usize::from(matched.optional_hits) < self.optional.len()
    }
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// The audio languages a probed file carries, for ranking.
///
/// Per-track tags win, then track titles. When any track stays unresolved, or
/// the file reports no tracks at all, the release-name and indexer hints fill
/// in, the same way the required-audio gate treats an untagged file.
pub(crate) fn file_audio_languages(
    audio_streams: &[crate::AudioStreamDetail],
    tagged_languages: &[String],
    release_hints: &[String],
) -> Vec<String> {
    let mut resolved = Vec::new();
    let mut has_unresolved_track = audio_streams.is_empty();
    for stream in audio_streams {
        let languages = resolved_track_languages(stream);
        if languages.is_empty() {
            has_unresolved_track = true;
        }
        for code in languages {
            if !resolved.contains(&code) {
                resolved.push(code);
            }
        }
    }
    if audio_streams.is_empty() {
        for code in normalize_required_audio_languages(tagged_languages.iter().cloned()) {
            if !resolved.contains(&code) {
                resolved.push(code);
            }
        }
        has_unresolved_track = resolved.is_empty();
    }
    if has_unresolved_track {
        for hint in normalize_required_audio_languages(release_hints.iter().cloned()) {
            if !resolved.contains(&hint) {
                resolved.push(hint);
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(preferred: &[&str], optional: &[&str]) -> AudioLanguagePreferences {
        AudioLanguagePreferences {
            preferred: preferred.iter().map(|code| code.to_string()).collect(),
            optional: optional.iter().map(|code| code.to_string()).collect(),
            optional_keep_searching: false,
        }
    }

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| code.to_string()).collect()
    }

    fn stream(language: Option<&str>, name: Option<&str>) -> crate::AudioStreamDetail {
        crate::AudioStreamDetail {
            codec: None,
            profile: None,
            channels: None,
            language: language.map(str::to_string),
            name: name.map(str::to_string),
            bitrate_kbps: None,
        }
    }

    #[test]
    fn empty_preferences_rank_every_release_the_same() {
        let empty = AudioLanguagePreferences::default();
        assert_eq!(
            empty.evaluate(&langs(&["deu"])),
            AudioLanguageMatch::default()
        );
        assert_eq!(empty.evaluate(&[]), AudioLanguageMatch::default());
        assert!(empty.is_satisfied(AudioLanguageMatch::default()));
    }

    #[test]
    fn preferred_rank_is_the_first_listed_language_present() {
        let prefs = prefs(&["deu", "eng"], &[]);
        assert_eq!(prefs.evaluate(&langs(&["eng", "deu"])).preferred_rank, 0);
        assert_eq!(prefs.evaluate(&langs(&["eng"])).preferred_rank, 1);
        assert_eq!(prefs.evaluate(&langs(&["fra"])).preferred_rank, 2);
        assert_eq!(prefs.evaluate(&[]).preferred_rank, 2);
    }

    #[test]
    fn the_rank_carries_the_preferred_list_length_for_display() {
        let prefs = prefs(&["deu", "eng"], &[]);
        let english = AudioLanguageRank::new(&prefs, prefs.evaluate(&langs(&["eng"])));
        assert_eq!(english.preferred_count, 2);
        assert!(english.matched.preferred_rank < english.preferred_count);
        let french = AudioLanguageRank::new(&prefs, prefs.evaluate(&langs(&["fra"])));
        assert_eq!(french.matched.preferred_rank, french.preferred_count);
        let none = AudioLanguagePreferences::default();
        assert_eq!(
            AudioLanguageRank::new(&none, none.evaluate(&langs(&["fra"]))).preferred_count,
            0
        );
    }

    #[test]
    fn language_codes_are_normalized_before_matching() {
        let prefs = prefs(&["deu"], &["eng"]);
        let matched = prefs.evaluate(&langs(&["ger", "en"]));
        assert_eq!(matched.preferred_rank, 0);
        assert_eq!(matched.optional_hits, 1);
    }

    #[test]
    fn optional_languages_are_counted() {
        let prefs = prefs(&["deu"], &["eng", "jpn"]);
        assert_eq!(prefs.evaluate(&langs(&["deu"])).optional_hits, 0);
        assert_eq!(prefs.evaluate(&langs(&["deu", "eng"])).optional_hits, 1);
        assert_eq!(prefs.evaluate(&langs(&["jpn", "eng"])).optional_hits, 2);
    }

    #[test]
    fn a_preferred_language_is_not_counted_again_as_optional() {
        let context =
            crate::title_audio_language_context(Some("en"), Some("US"), Some("movie"), &[]);
        let prefs = AudioLanguagePreferences::resolve(
            &langs(&["deu"]),
            &langs(&["ger", "eng"]),
            false,
            &context,
        );
        assert_eq!(prefs.optional, langs(&["eng"]));
    }

    #[test]
    fn original_expands_to_the_titles_original_language() {
        let context =
            crate::title_audio_language_context(Some("ja"), Some("JP"), Some("anime"), &[]);
        let prefs = AudioLanguagePreferences::resolve(
            &langs(&["deu"]),
            &langs(&["original"]),
            false,
            &context,
        );
        assert_eq!(prefs.optional, langs(&["jpn"]));
    }

    #[test]
    fn missing_preferred_language_keeps_searching() {
        let prefs = prefs(&["deu"], &["eng"]);
        assert!(!prefs.is_satisfied(prefs.evaluate(&langs(&["eng"]))));
        assert!(prefs.is_satisfied(prefs.evaluate(&langs(&["deu"]))));
    }

    #[test]
    fn missing_optional_languages_keep_searching_only_when_configured() {
        let mut prefs = prefs(&["deu"], &["eng"]);
        let german_only = prefs.evaluate(&langs(&["deu"]));
        assert!(prefs.is_satisfied(german_only));
        prefs.optional_keep_searching = true;
        assert!(!prefs.is_satisfied(german_only));
        assert!(prefs.is_satisfied(prefs.evaluate(&langs(&["deu", "eng"]))));
    }

    #[test]
    fn a_language_keeps_a_file_wanted_only_while_its_gate_runs() {
        let mut prefs = prefs(&["deu"], &["eng"]);
        prefs.optional_keep_searching = true;
        let english_rank = AudioLanguageRank::new(&prefs, prefs.evaluate(&langs(&["eng"])));
        let german_rank = AudioLanguageRank::new(&prefs, prefs.evaluate(&langs(&["deu"])));
        let held = |language| crate::decision_pipeline::SideFacts {
            language,
            ..Default::default()
        };
        let (english_only, german_only) = (held(english_rank), held(german_rank));
        // The language steps are opt-in, so the order that runs both is a
        // chosen one: tier and revision first, then the languages.
        let both_languages = crate::DecisionPipeline::try_from_gates([
            crate::DecisionGate::QualityTier,
            crate::DecisionGate::Revision,
            crate::DecisionGate::PreferredLanguage,
            crate::DecisionGate::OptionalLanguages,
        ])
        .unwrap();
        assert!(both_languages.still_wants(&english_only));
        assert!(both_languages.still_wants(&german_only));
        // The default order runs neither language step, so it wants nothing.
        let default_order = crate::DecisionPipeline::default_order();
        assert!(!default_order.still_wants(&english_only));
        assert!(!default_order.still_wants(&german_only));
        let without_preferred = both_languages.without(crate::DecisionGate::PreferredLanguage);
        assert!(!without_preferred.still_wants(&english_only));
        assert!(without_preferred.still_wants(&german_only));
        let without_optional = both_languages.without(crate::DecisionGate::OptionalLanguages);
        assert!(without_optional.still_wants(&english_only));
        assert!(!without_optional.still_wants(&german_only));
        assert!(!crate::DecisionPipeline::score_only().still_wants(&english_only));
        assert!(english_rank.wanted && german_rank.wanted);
    }

    #[test]
    fn file_languages_prefer_track_tags_over_release_hints() {
        let streams = [stream(Some("eng"), None)];
        assert_eq!(
            file_audio_languages(&streams, &[], &langs(&["deu"])),
            langs(&["eng"])
        );
    }

    #[test]
    fn untagged_tracks_fall_back_to_release_hints() {
        let streams = [stream(Some("und"), None), stream(Some("eng"), None)];
        assert_eq!(
            file_audio_languages(&streams, &[], &langs(&["deu"])),
            langs(&["eng", "deu"])
        );
    }

    #[test]
    fn track_titles_resolve_untagged_streams() {
        let streams = [stream(None, Some("German 5.1"))];
        assert_eq!(file_audio_languages(&streams, &[], &[]), langs(&["deu"]));
    }

    #[test]
    fn files_without_stream_detail_use_tagged_languages_then_hints() {
        assert_eq!(
            file_audio_languages(&[], &langs(&["ger"]), &langs(&["eng"])),
            langs(&["deu"])
        );
        assert_eq!(
            file_audio_languages(&[], &[], &langs(&["eng"])),
            langs(&["eng"])
        );
    }
}
