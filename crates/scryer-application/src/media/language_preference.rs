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
//! [`AudioLanguagePriority`] decides whether the preferred rank is compared before
//! the quality tier (`LanguageFirst`) or after tier and revision
//! (`QualityFirst`). Optional hits always come after tier and revision.
//!
//! Empty lists give every release the same [`AudioLanguageMatch`], so every ordering
//! and admission comparison degrades to the behaviour without preferences.

use serde::{Deserialize, Serialize};

use crate::audio_requirements::{
    normalize_required_audio_languages, resolve_required_audio_requirements,
    resolved_track_languages,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioLanguagePriority {
    /// Tier, then revision, then preferred rank, then optional hits.
    #[default]
    QualityFirst,
    /// Preferred rank, then tier, then revision, then optional hits.
    LanguageFirst,
}

impl AudioLanguagePriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QualityFirst => "quality_first",
            Self::LanguageFirst => "language_first",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "quality_first" => Some(Self::QualityFirst),
            "language_first" => Some(Self::LanguageFirst),
            _ => None,
        }
    }
}

/// The resolved preferences for one title: concrete, normalized language codes
/// with the `original` selector already expanded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguagePreferences {
    pub preferred: Vec<String>,
    pub optional: Vec<String>,
    pub priority: AudioLanguagePriority,
    /// Whether a file still missing optional languages keeps its title wanted.
    pub optional_keep_searching: bool,
}

/// Preferences that rank nothing, for contexts built by hand in tests.
#[cfg(test)]
pub(crate) static NO_LANGUAGE_PREFERENCES: AudioLanguagePreferences = AudioLanguagePreferences {
    preferred: Vec::new(),
    optional: Vec::new(),
    priority: AudioLanguagePriority::QualityFirst,
    optional_keep_searching: false,
};

/// Where one release or file stands against [`AudioLanguagePreferences`].
///
/// Both fields are oriented for comparison: a lower `preferred_rank` is better,
/// a higher `optional_hits` is better.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguageMatch {
    pub preferred_rank: u16,
    pub optional_hits: u16,
}

/// A [`AudioLanguageMatch`] together with where its preferred rank is compared.
///
/// Orderings and admission read their language steps from here, so the
/// priority travels with the facts and cannot be applied inconsistently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AudioLanguageRank {
    pub matched: AudioLanguageMatch,
    pub language_first: bool,
}

impl AudioLanguageRank {
    pub(crate) fn new(preferences: &AudioLanguagePreferences, matched: AudioLanguageMatch) -> Self {
        Self {
            matched,
            language_first: preferences.language_first(),
        }
    }

    /// The preferred rank when it outranks the quality tier, else `0`.
    pub(crate) fn before_tier(self) -> u16 {
        if self.language_first {
            self.matched.preferred_rank
        } else {
            0
        }
    }

    /// The preferred rank when it follows tier and revision, else `0`.
    pub(crate) fn after_revision(self) -> u16 {
        if self.language_first {
            0
        } else {
            self.matched.preferred_rank
        }
    }

    /// Optional hits negated, so that less is better like every other step.
    pub(crate) fn negated_optional_hits(self) -> i32 {
        -i32::from(self.matched.optional_hits)
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
        priority: AudioLanguagePriority,
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
            priority,
            optional_keep_searching,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.preferred.is_empty() && self.optional.is_empty()
    }

    pub fn language_first(&self) -> bool {
        self.priority == AudioLanguagePriority::LanguageFirst
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
        if !self.preferred.is_empty() && matched.preferred_rank > 0 {
            return false;
        }
        !(self.optional_keep_searching && usize::from(matched.optional_hits) < self.optional.len())
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
            priority: AudioLanguagePriority::LanguageFirst,
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
            AudioLanguagePriority::LanguageFirst,
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
            AudioLanguagePriority::LanguageFirst,
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
    fn priority_round_trips_through_its_setting_value() {
        for priority in [
            AudioLanguagePriority::QualityFirst,
            AudioLanguagePriority::LanguageFirst,
        ] {
            assert_eq!(
                AudioLanguagePriority::parse(priority.as_str()),
                Some(priority)
            );
        }
        assert_eq!(AudioLanguagePriority::parse("nonsense"), None);
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
