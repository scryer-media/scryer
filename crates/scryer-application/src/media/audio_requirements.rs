use crate::{ParsedReleaseMetadata, normalize_detected_audio_language_code};

const DEFAULT_NON_ANIME_AUDIO_LANGUAGE: &str = "eng";
const DEFAULT_ANIME_AUDIO_LANGUAGE: &str = "jpn";
pub(crate) const ORIGINAL_AUDIO_LANGUAGE_REQUIREMENT: &str = "original";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TitleAudioLanguageContext {
    pub original_language: Option<String>,
    pub original_country: Option<String>,
    pub inferred_original_audio_language: String,
    pub is_anime: bool,
}

pub(crate) fn normalize_required_audio_languages(
    languages: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for language in languages {
        if let Some(code) = normalize_detected_audio_language_code(&language)
            && !normalized.contains(&code)
        {
            normalized.push(code);
        }
    }
    normalized
}

/// Normalize configured audio requirements without confusing the dynamic
/// `original` selector with an observed language code.
pub(crate) fn normalize_required_audio_requirements(
    requirements: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for requirement in requirements {
        let value = if requirement
            .trim()
            .eq_ignore_ascii_case(ORIGINAL_AUDIO_LANGUAGE_REQUIREMENT)
        {
            Some(ORIGINAL_AUDIO_LANGUAGE_REQUIREMENT.to_string())
        } else {
            normalize_detected_audio_language_code(&requirement)
        };
        if let Some(value) = value
            && !normalized.contains(&value)
        {
            normalized.push(value);
        }
    }
    normalized
}

/// Expand configured requirements into concrete per-title language codes.
/// Multiple requirements retain their existing AND semantics.
pub(crate) fn resolve_required_audio_requirements(
    requirements: &[String],
    title_context: &TitleAudioLanguageContext,
) -> Vec<String> {
    let resolved = requirements.iter().filter_map(|requirement| {
        if requirement.eq_ignore_ascii_case(ORIGINAL_AUDIO_LANGUAGE_REQUIREMENT) {
            Some(title_context.inferred_original_audio_language.clone())
        } else {
            normalize_detected_audio_language_code(requirement)
        }
    });
    normalize_required_audio_languages(resolved)
}

pub(crate) fn normalize_title_country_code(country: &str) -> Option<String> {
    let normalized = country
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if normalized.is_empty() {
        return None;
    }

    let country = match normalized.as_str() {
        "AR" | "ARG" | "ARGENTINA" => "AR",
        "AU" | "AUS" | "AUSTRALIA" => "AU",
        "BE" | "BEL" | "BELGIUM" => "BE",
        "BR" | "BRA" | "BRAZIL" => "BR",
        "CA" | "CAN" | "CANADA" => "CA",
        "CH" | "CHE" | "SWITZERLAND" => "CH",
        "CN" | "CHN" | "CHINA" => "CN",
        "DE" | "DEU" | "GERMANY" => "DE",
        "DK" | "DNK" | "DENMARK" => "DK",
        "ES" | "ESP" | "SPAIN" => "ES",
        "FI" | "FIN" | "FINLAND" => "FI",
        "FR" | "FRA" | "FRANCE" => "FR",
        "GB" | "GBR" | "UK" | "UNITEDKINGDOM" | "GREATBRITAIN" => "GB",
        "HK" | "HKG" | "HONGKONG" => "HK",
        "IE" | "IRL" | "IRELAND" => "IE",
        "IN" | "IND" | "INDIA" => "IN",
        "IT" | "ITA" | "ITALY" => "IT",
        "JP" | "JPN" | "JAPAN" => "JP",
        "KR" | "KOR" | "SOUTHKOREA" => "KR",
        "MX" | "MEX" | "MEXICO" => "MX",
        "NL" | "NLD" | "NETHERLANDS" => "NL",
        "NO" | "NOR" | "NORWAY" => "NO",
        "NZ" | "NZL" | "NEWZEALAND" => "NZ",
        "PH" | "PHL" | "PHILIPPINES" => "PH",
        "PL" | "POL" | "POLAND" => "PL",
        "PT" | "PRT" | "PORTUGAL" => "PT",
        "RU" | "RUS" | "RUSSIA" => "RU",
        "SE" | "SWE" | "SWEDEN" => "SE",
        "TH" | "THA" | "THAILAND" => "TH",
        "TR" | "TUR" | "TURKEY" => "TR",
        "TW" | "TWN" | "TAIWAN" => "TW",
        "US" | "USA" | "UNITEDSTATES" | "UNITEDSTATESOFAMERICA" => "US",
        _ => return None,
    };

    Some(country.to_string())
}

fn inferred_audio_language_for_country(country: &str) -> Option<&'static str> {
    match country {
        "AR" | "ES" | "MX" => Some("spa"),
        "AU" | "GB" | "IE" | "NZ" | "US" => Some("eng"),
        "BR" | "PT" => Some("por"),
        "CN" | "HK" | "TW" => Some("zho"),
        "DE" => Some("deu"),
        "DK" => Some("dan"),
        "FI" => Some("fin"),
        "FR" => Some("fra"),
        "IT" => Some("ita"),
        "JP" => Some("jpn"),
        "KR" => Some("kor"),
        "NL" => Some("nld"),
        "NO" => Some("nor"),
        "PL" => Some("pol"),
        "RU" => Some("rus"),
        "SE" => Some("swe"),
        "TH" => Some("tha"),
        "TR" => Some("tur"),
        _ => None,
    }
}

fn is_anime_context(category: Option<&str>, title_tags: &[String]) -> bool {
    category.is_some_and(|value| value.eq_ignore_ascii_case("anime"))
        || title_tags.iter().any(|tag| tag_marks_anime(tag))
}

/// Whether a title tag marks the title as anime. Matches the bare `anime` tag
/// as well as namespaced variants such as `anime-hd`. The search facet (and the
/// `category` hint derived from it) collapses anime movies and series-movie
/// links to `movie`, so tag-based detection is the fallback signal when the
/// category alone no longer carries the anime origin.
fn tag_marks_anime(tag: &str) -> bool {
    let tag = tag.trim();
    tag.eq_ignore_ascii_case("anime")
        || tag
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("anime-"))
}

pub(crate) fn title_audio_language_context(
    title_language: Option<&str>,
    title_country: Option<&str>,
    category: Option<&str>,
    title_tags: &[String],
) -> TitleAudioLanguageContext {
    let original_language = title_language.and_then(normalize_detected_audio_language_code);
    let original_country = title_country.and_then(normalize_title_country_code);
    let is_anime = is_anime_context(category, title_tags);
    let inferred_original_audio_language = original_language
        .clone()
        .or_else(|| {
            original_country
                .as_deref()
                .and_then(inferred_audio_language_for_country)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if is_anime {
                DEFAULT_ANIME_AUDIO_LANGUAGE.to_string()
            } else {
                DEFAULT_NON_ANIME_AUDIO_LANGUAGE.to_string()
            }
        });

    TitleAudioLanguageContext {
        original_language,
        original_country,
        inferred_original_audio_language,
        is_anime,
    }
}

fn push_language(normalized: &mut Vec<String>, language: &str) {
    if !normalized.iter().any(|existing| existing == language) {
        normalized.push(language.to_string());
    }
}

pub(crate) fn release_audio_language_hints_for_title(
    parsed: &ParsedReleaseMetadata,
    indexer_languages: Option<&[String]>,
    title_language_context: Option<&TitleAudioLanguageContext>,
    infer_unlabeled_audio: bool,
) -> Vec<String> {
    let mut normalized = normalize_required_audio_languages(parsed.languages_audio.clone());

    if let Some(indexer_languages) = indexer_languages {
        for language in indexer_languages {
            if let Some(code) = normalize_detected_audio_language_code(language)
                && !normalized.contains(&code)
            {
                normalized.push(code);
            }
        }
    }

    if parsed.is_dual_audio && normalized.is_empty() {
        // A "dual audio" release ships two audio tracks: English plus the
        // title's original-language audio. Use the title's inferred original
        // language for the second track (which defaults to Japanese for anime),
        // and fall back to the canonical anime pairing when we have no context.
        // This applies to non-anime titles too, so an unlabeled dual-audio
        // release is not falsely treated as having zero audio languages.
        push_language(&mut normalized, "eng");
        let original_language = title_language_context
            .map(|context| context.inferred_original_audio_language.as_str())
            .unwrap_or(DEFAULT_ANIME_AUDIO_LANGUAGE);
        push_language(&mut normalized, original_language);
    }

    if normalized.is_empty()
        && !parsed.is_dual_audio
        && infer_unlabeled_audio
        && let Some(context) = title_language_context
    {
        push_language(&mut normalized, &context.inferred_original_audio_language);
    }

    normalized
}

pub(crate) fn missing_required_audio_languages<'a>(
    required: &'a [String],
    actual: &'a [String],
) -> Vec<String> {
    let actual_languages: Vec<String> = normalize_required_audio_languages(actual.iter().cloned());

    let mut missing = Vec::new();
    for required_language in required {
        let Some(normalized) = normalize_detected_audio_language_code(required_language) else {
            continue;
        };
        if !actual_languages
            .iter()
            .any(|actual_language| actual_language == &normalized)
        {
            missing.push(normalized);
        }
    }

    missing
}

pub(crate) fn required_audio_languages_match(required: &[String], actual: &[String]) -> bool {
    missing_required_audio_languages(required, actual).is_empty()
}

/// Resolve all canonical audio language codes named in a free-text audio track
/// title such as "English 5.1", "Japanese + English", or "Eng+Jpn".
///
/// Only UNAMBIGUOUS signals are accepted: tokens of 3+ characters (full language
/// names or ISO 639 codes), resolved with the strict (passthrough-free) resolver
/// so codec/technical tokens (DTS, AAC, AC3) are not mistaken for languages.
/// Short 2-letter tokens are deliberately ignored: they collide with obscure ISO
/// two-letter codes and common English words (e.g. "VO", "is", "it", "no"), and
/// would otherwise mis-resolve a descriptive title to the wrong language. The
/// title is tokenized on non-alphanumeric boundaries (no whole-string subtag
/// splitting, which would turn "no-audio" into Norwegian). Returns distinct
/// languages in order; empty when nothing maps.
#[cfg(any(test, feature = "runtime-media-analysis"))]
pub(crate) fn resolve_audio_languages_from_track_title(title: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in title.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.len() < 3 {
            continue;
        }
        if let Some(code) = crate::normalize_known_audio_language_code(token)
            && !found.contains(&code)
        {
            found.push(code);
        }
    }
    found
}

/// The resolved language(s) of a single probed audio track: its ISO tag wins;
/// otherwise its title is parsed; otherwise the track is unresolved (empty).
///
/// The tag is resolved with the STRICT resolver too, so a junk/non-ISO language
/// tag (e.g. a codec string mis-stored in the language field) is treated as
/// unresolved rather than a bogus "known" language that could flip an
/// indeterminate result into a false rejection.
#[cfg(any(test, feature = "runtime-media-analysis"))]
fn resolved_track_languages(stream: &crate::AudioStreamDetail) -> Vec<String> {
    if let Some(code) = stream
        .language
        .as_deref()
        .and_then(crate::normalize_known_audio_language_code)
    {
        return vec![code];
    }
    stream
        .name
        .as_deref()
        .map(resolve_audio_languages_from_track_title)
        .unwrap_or_default()
}

/// Verdict for the post-download required-audio-language gate.
#[cfg(any(test, feature = "runtime-media-analysis"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequiredAudioVerdict {
    /// Every required language is present.
    Satisfied,
    /// Required language(s) are provably absent: every audio track's language is
    /// known (via tag or title) and none — nor the release hints — supply them.
    Missing(Vec<String>),
    /// Required language(s) can be neither confirmed present nor proven absent,
    /// because one or more audio tracks carry no usable language signal.
    Indeterminate(Vec<String>),
}

/// Classify whether a probed file satisfies the required audio languages.
///
/// Uses only strong signals — per-track ISO tags, per-track titles, and the
/// release-name/indexer hints already computed for the title. Distinguishes a
/// provable absence (`Missing` → reject) from an indeterminate result
/// (`Indeterminate` → accept + flag) so a correctly-dubbed file whose tracks are
/// untagged ("und") is not falsely rejected.
#[cfg(any(test, feature = "runtime-media-analysis"))]
pub(crate) fn classify_required_audio(
    required: &[String],
    audio_streams: &[crate::AudioStreamDetail],
    release_hints: &[String],
) -> RequiredAudioVerdict {
    let required: Vec<String> = normalize_required_audio_languages(required.iter().cloned());
    if required.is_empty() {
        return RequiredAudioVerdict::Satisfied;
    }

    // A file with no audio tracks carries no usable per-track signal: never
    // reject on it (avoid burying a release on a probe oddity); flag for review.
    if audio_streams.is_empty() {
        return RequiredAudioVerdict::Indeterminate(required);
    }

    let mut resolved: Vec<String> = Vec::new();
    let mut has_unresolved_track = false;
    for stream in audio_streams {
        let langs = resolved_track_languages(stream);
        if langs.is_empty() {
            has_unresolved_track = true;
        }
        for code in langs {
            if !resolved.contains(&code) {
                resolved.push(code);
            }
        }
    }

    // A release-name or indexer label can resolve an otherwise untagged stream,
    // but it cannot override a probe that has already proved every track is a
    // different language.
    if has_unresolved_track {
        for hint in normalize_required_audio_languages(release_hints.iter().cloned()) {
            if !resolved.contains(&hint) {
                resolved.push(hint);
            }
        }
    }

    let missing: Vec<String> = required
        .into_iter()
        .filter(|lang| !resolved.contains(lang))
        .collect();

    if missing.is_empty() {
        RequiredAudioVerdict::Satisfied
    } else if has_unresolved_track {
        RequiredAudioVerdict::Indeterminate(missing)
    } else {
        RequiredAudioVerdict::Missing(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RequiredAudioVerdict, classify_required_audio, missing_required_audio_languages,
        normalize_required_audio_languages, normalize_required_audio_requirements,
        normalize_title_country_code, release_audio_language_hints_for_title,
        required_audio_languages_match, resolve_audio_languages_from_track_title,
        resolve_required_audio_requirements, title_audio_language_context,
    };
    use crate::AudioStreamDetail;
    use crate::normalize_detected_audio_language_code;
    use crate::release_parser::parse_release_metadata;

    fn audio_stream(language: Option<&str>, name: Option<&str>) -> AudioStreamDetail {
        AudioStreamDetail {
            codec: None,
            profile: None,
            channels: None,
            language: language.map(str::to_string),
            name: name.map(str::to_string),
            bitrate_kbps: None,
        }
    }

    #[test]
    fn dual_audio_without_explicit_languages_implies_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn explicit_languages_prevent_dual_audio_fallback() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO ENG 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn indexer_languages_are_merged_with_release_languages() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(
                &parsed,
                Some(&["English".to_string(), "Japanese".to_string()]),
                Some(&context),
                false,
            ),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn required_audio_languages_are_normalized() {
        assert_eq!(
            normalize_required_audio_languages(vec![
                "English".to_string(),
                "eng".to_string(),
                "ja-JP".to_string(),
            ]),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn configured_audio_requirements_preserve_original_selector() {
        assert_eq!(
            normalize_required_audio_requirements(vec![
                "Original".to_string(),
                "original".to_string(),
                "English".to_string(),
            ]),
            vec!["original".to_string(), "eng".to_string()]
        );
    }

    #[test]
    fn original_requirement_resolves_and_keeps_and_semantics() {
        let context = title_audio_language_context(Some("jpn"), None, Some("movie"), &[]);
        assert_eq!(
            resolve_required_audio_requirements(
                &["original".to_string(), "eng".to_string()],
                &context,
            ),
            vec!["jpn".to_string(), "eng".to_string()]
        );
    }

    #[test]
    fn original_requirement_deduplicates_matching_concrete_language() {
        let context = title_audio_language_context(Some("eng"), None, Some("movie"), &[]);
        assert_eq!(
            resolve_required_audio_requirements(
                &["original".to_string(), "eng".to_string()],
                &context,
            ),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn japanese_original_requirement_accepts_unlabeled_original_and_rejects_explicit_english() {
        let context = title_audio_language_context(Some("jpn"), None, Some("anime"), &[]);
        let required = resolve_required_audio_requirements(&["original".to_string()], &context);

        let unlabeled = parse_release_metadata("[Group] Example Title 1080p");
        let unlabeled_audio =
            release_audio_language_hints_for_title(&unlabeled, None, Some(&context), true);
        assert!(required_audio_languages_match(&required, &unlabeled_audio));

        let english = parse_release_metadata("[Group] Example Title ENG 1080p");
        let english_audio =
            release_audio_language_hints_for_title(&english, None, Some(&context), true);
        assert!(!required_audio_languages_match(&required, &english_audio));
    }

    #[test]
    fn japanese_original_and_english_requirement_needs_dual_audio() {
        let context = title_audio_language_context(Some("jpn"), None, Some("anime"), &[]);
        let required = resolve_required_audio_requirements(
            &["original".to_string(), "eng".to_string()],
            &context,
        );

        let dual = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let dual_audio = release_audio_language_hints_for_title(&dual, None, Some(&context), true);
        assert!(required_audio_languages_match(&required, &dual_audio));

        let unlabeled = parse_release_metadata("[Group] Example Title 1080p");
        let unlabeled_audio =
            release_audio_language_hints_for_title(&unlabeled, None, Some(&context), true);
        assert!(!required_audio_languages_match(&required, &unlabeled_audio));
    }

    #[test]
    fn language_aliases_normalize_to_canonical_audio_codes() {
        assert_eq!(
            normalize_detected_audio_language_code("en").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_audio_language_code("English").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fre").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fr-FR").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_detected_audio_language_code("ja-JP").as_deref(),
            Some("jpn")
        );
        assert_eq!(
            normalize_detected_audio_language_code("Ger").as_deref(),
            Some("deu")
        );
        assert_eq!(normalize_detected_audio_language_code("und"), None);
    }

    #[test]
    fn title_countries_normalize_to_uppercase_alpha2_codes() {
        assert_eq!(normalize_title_country_code(" fr ").as_deref(), Some("FR"));
        assert_eq!(normalize_title_country_code("FRA").as_deref(), Some("FR"));
        assert_eq!(
            normalize_title_country_code("France").as_deref(),
            Some("FR")
        );
        assert_eq!(normalize_title_country_code("jp").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("JPN").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("Japan").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("not-a-country"), None);
    }

    #[test]
    fn title_audio_context_prefers_explicit_language() {
        let context = title_audio_language_context(Some("fre"), Some("Japan"), Some("movie"), &[]);

        assert_eq!(context.original_language.as_deref(), Some("fra"));
        assert_eq!(context.original_country.as_deref(), Some("JP"));
        assert_eq!(context.inferred_original_audio_language, "fra");
        assert!(!context.is_anime);
    }

    #[test]
    fn title_audio_context_uses_high_confidence_country_fallback() {
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(context.original_country.as_deref(), Some("FR"));
        assert_eq!(context.inferred_original_audio_language, "fra");
    }

    #[test]
    fn title_audio_context_defaults_unknown_non_anime_to_english() {
        let context = title_audio_language_context(None, Some("Canada"), Some("movie"), &[]);

        assert_eq!(context.original_country.as_deref(), Some("CA"));
        assert_eq!(context.inferred_original_audio_language, "eng");
    }

    #[test]
    fn title_audio_context_defaults_unknown_anime_to_japanese() {
        let context = title_audio_language_context(None, None, Some("anime"), &[]);

        assert_eq!(context.inferred_original_audio_language, "jpn");
        assert!(context.is_anime);
    }

    #[test]
    fn unlabeled_french_origin_release_infers_french_audio() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), true),
            vec!["fra".to_string()]
        );
    }

    #[test]
    fn unlabeled_unknown_non_anime_release_infers_english_audio() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), true),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn unlabeled_release_does_not_infer_audio_when_required_gating_is_disabled() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            Vec::<String>::new()
        );
    }

    #[test]
    fn anime_dual_audio_uses_english_plus_inferred_original_language() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, Some("South Korea"), Some("anime"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "kor".to_string()]
        );
    }

    #[test]
    fn non_anime_dual_audio_infers_english_plus_origin_language() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "fra".to_string()]
        );
    }

    #[test]
    fn non_anime_dual_audio_unknown_origin_implies_english() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn dual_audio_without_title_context_defaults_to_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, None, false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn anime_tagged_movie_dual_audio_infers_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        // Anime movies / series-movie links collapse to the "movie" search
        // category but carry an anime-* tag; detection must still treat them as
        // anime so dual-audio infers eng+jpn rather than just eng.
        let context =
            title_audio_language_context(None, None, Some("movie"), &["anime-hd".to_string()]);
        assert!(context.is_anime);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn anime_tag_variants_drive_anime_detection() {
        assert!(
            title_audio_language_context(None, None, Some("movie"), &["anime-hd".to_string()])
                .is_anime
        );
        assert!(
            title_audio_language_context(None, None, Some("movie"), &["Anime".to_string()])
                .is_anime
        );
        // Substrings that merely start with "anime" but are not the anime marker
        // must not be misdetected.
        assert!(
            !title_audio_language_context(None, None, Some("movie"), &["animation".to_string()])
                .is_anime
        );
    }

    #[test]
    fn subtitle_language_markers_do_not_satisfy_required_audio() {
        let parsed = parse_release_metadata("[Group] Example Title GER SUBS ENG 1080p");
        let context = title_audio_language_context(None, Some("Germany"), Some("series"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), true);

        assert!(actual.contains(&"deu".to_string()));
        assert!(!required_audio_languages_match(
            &["eng".to_string()],
            &actual
        ));
    }

    #[test]
    fn missing_languages_are_reported_in_canonical_form() {
        assert_eq!(
            missing_required_audio_languages(
                &["English".to_string(), "Japanese".to_string()],
                &["eng".to_string()]
            ),
            vec!["jpn".to_string()]
        );
    }

    #[test]
    fn dual_audio_matches_required_english() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(required_audio_languages_match(
            &["eng".to_string()],
            &actual
        ));
    }

    #[test]
    fn dual_audio_matches_required_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(required_audio_languages_match(
            &["jpn".to_string()],
            &actual
        ));
    }

    #[test]
    fn explicit_english_audio_does_not_imply_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO ENG 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(!required_audio_languages_match(
            &["jpn".to_string()],
            &actual
        ));
    }

    #[test]
    fn track_title_resolves_language_tokens_and_ignores_codecs() {
        assert_eq!(
            resolve_audio_languages_from_track_title("English 5.1"),
            vec!["eng".to_string()]
        );
        assert_eq!(
            resolve_audio_languages_from_track_title("Eng DTS-HD MA"),
            vec!["eng".to_string()]
        );
        assert_eq!(
            resolve_audio_languages_from_track_title("Eng+Jpn"),
            vec!["eng".to_string(), "jpn".to_string()]
        );
        // Pure codec / non-language titles must not resolve to a language.
        assert!(resolve_audio_languages_from_track_title("DTS-HD MA").is_empty());
        assert!(resolve_audio_languages_from_track_title("Commentary").is_empty());
        assert!(resolve_audio_languages_from_track_title("").is_empty());
    }

    #[test]
    fn classify_tagged_track_satisfies_requirement() {
        let verdict = classify_required_audio(
            &["eng".to_string()],
            &[audio_stream(Some("eng"), None)],
            &[],
        );
        assert_eq!(verdict, RequiredAudioVerdict::Satisfied);
    }

    #[test]
    fn classify_untagged_track_resolved_by_title_is_satisfied() {
        // English track tagged "und" (so language=None after normalization) but
        // titled "English" must satisfy a required-English profile.
        let verdict = classify_required_audio(
            &["eng".to_string()],
            &[audio_stream(None, Some("English"))],
            &[],
        );
        assert_eq!(verdict, RequiredAudioVerdict::Satisfied);
    }

    #[test]
    fn classify_all_tracks_known_but_missing_is_provable_absence() {
        // Every track is tagged and none is English → provably absent → reject.
        let verdict = classify_required_audio(
            &["eng".to_string()],
            &[audio_stream(Some("jpn"), None)],
            &[],
        );
        assert_eq!(
            verdict,
            RequiredAudioVerdict::Missing(vec!["eng".to_string()])
        );
    }

    #[test]
    fn inferred_unlabeled_original_cannot_override_known_conflicting_track() {
        let context = title_audio_language_context(Some("jpn"), None, Some("movie"), &[]);
        let parsed = parse_release_metadata("Example Title 1080p");
        let inferred = release_audio_language_hints_for_title(&parsed, None, Some(&context), true);
        assert_eq!(inferred, vec!["jpn".to_string()]);

        let verdict = classify_required_audio(
            &["jpn".to_string()],
            &[audio_stream(Some("eng"), None)],
            &inferred,
        );
        assert_eq!(
            verdict,
            RequiredAudioVerdict::Missing(vec!["jpn".to_string()])
        );
    }

    #[test]
    fn classify_untagged_track_is_indeterminate_not_rejected() {
        // One jpn track plus one untagged track: English can be neither confirmed
        // nor proven absent → accept + flag, never a hard reject.
        let verdict = classify_required_audio(
            &["eng".to_string()],
            &[audio_stream(Some("jpn"), None), audio_stream(None, None)],
            &[],
        );
        assert_eq!(
            verdict,
            RequiredAudioVerdict::Indeterminate(vec!["eng".to_string()])
        );
    }

    #[test]
    fn classify_release_hints_resolve_untagged_tracks() {
        // A DUAL release whose file is {jpn-tagged, untagged} satisfies required
        // English via the release hint (the release claims eng).
        let verdict = classify_required_audio(
            &["eng".to_string()],
            &[audio_stream(Some("jpn"), None), audio_stream(None, None)],
            &["eng".to_string()],
        );
        assert_eq!(verdict, RequiredAudioVerdict::Satisfied);
    }

    #[test]
    fn classify_zero_audio_streams_is_indeterminate() {
        let verdict = classify_required_audio(&["eng".to_string()], &[], &[]);
        assert_eq!(
            verdict,
            RequiredAudioVerdict::Indeterminate(vec!["eng".to_string()])
        );
    }

    #[test]
    fn classify_empty_required_is_satisfied() {
        let verdict = classify_required_audio(&[], &[audio_stream(Some("jpn"), None)], &[]);
        assert_eq!(verdict, RequiredAudioVerdict::Satisfied);
    }

    #[test]
    fn track_title_ignores_short_ambiguous_tokens_and_subtags() {
        // 2-letter markers/words must NOT resolve to obscure ISO languages
        // (previously "VO"->vol, "is"->isl, "it"->ita, "no"->nor).
        assert!(resolve_audio_languages_from_track_title("VO 5.1").is_empty());
        assert!(resolve_audio_languages_from_track_title("VO").is_empty());
        assert_eq!(
            resolve_audio_languages_from_track_title("This is the English mix"),
            vec!["eng".to_string()]
        );
        // Hyphenated descriptive names must not subtag-split into a language
        // (previously "no-audio"->nor, "to-be-confirmed"->ton).
        assert!(resolve_audio_languages_from_track_title("no-audio").is_empty());
        assert!(resolve_audio_languages_from_track_title("to-be-confirmed").is_empty());
    }

    #[test]
    fn track_title_lat_resolves_to_spanish_not_latin() {
        assert_eq!(
            resolve_audio_languages_from_track_title("LAT"),
            vec!["spa".to_string()]
        );
    }

    #[test]
    fn classify_junk_tag_is_treated_as_unresolved() {
        // A non-ISO junk tag (e.g. a codec mis-stored in the language field) is
        // unresolved -> Indeterminate, not a bogus known language -> Missing.
        assert_eq!(
            classify_required_audio(
                &["eng".to_string()],
                &[audio_stream(Some("dts"), None)],
                &[],
            ),
            RequiredAudioVerdict::Indeterminate(vec!["eng".to_string()])
        );
    }

    #[test]
    fn classify_two_letter_iso_tag_still_resolves() {
        // A legitimate 2-letter ISO tag resolves via the strict resolver.
        assert_eq!(
            classify_required_audio(&["eng".to_string()], &[audio_stream(Some("en"), None)], &[],),
            RequiredAudioVerdict::Satisfied
        );
    }

    #[test]
    fn classify_tag_beats_conflicting_title() {
        // A jpn-tagged track titled "English" resolves to jpn (tag wins).
        assert_eq!(
            classify_required_audio(
                &["jpn".to_string()],
                &[audio_stream(Some("jpn"), Some("English"))],
                &[],
            ),
            RequiredAudioVerdict::Satisfied
        );
        assert_eq!(
            classify_required_audio(
                &["eng".to_string()],
                &[audio_stream(Some("jpn"), Some("English"))],
                &[],
            ),
            RequiredAudioVerdict::Missing(vec!["eng".to_string()])
        );
    }

    #[test]
    fn classify_multiple_required_languages_partial_coverage() {
        assert_eq!(
            classify_required_audio(
                &["eng".to_string(), "jpn".to_string()],
                &[
                    audio_stream(Some("eng"), None),
                    audio_stream(Some("jpn"), None)
                ],
                &[],
            ),
            RequiredAudioVerdict::Satisfied
        );
        assert_eq!(
            classify_required_audio(
                &["eng".to_string(), "jpn".to_string()],
                &[audio_stream(Some("eng"), None)],
                &[],
            ),
            RequiredAudioVerdict::Missing(vec!["jpn".to_string()])
        );
    }

    #[test]
    fn classify_untagged_with_unhelpful_hint_is_indeterminate() {
        // An unresolved track plus a hint that does NOT cover the requirement
        // stays Indeterminate (never Missing), because the track is unprovable.
        assert_eq!(
            classify_required_audio(
                &["eng".to_string()],
                &[audio_stream(None, None)],
                &["jpn".to_string()],
            ),
            RequiredAudioVerdict::Indeterminate(vec!["eng".to_string()])
        );
    }

    #[test]
    fn classify_unnormalizable_required_collapses_to_satisfied() {
        // A required code that does not normalize (e.g. "und") is dropped; an
        // all-unnormalizable requirement collapses to no requirement -> Satisfied.
        assert_eq!(
            classify_required_audio(
                &["und".to_string()],
                &[audio_stream(Some("jpn"), None)],
                &[],
            ),
            RequiredAudioVerdict::Satisfied
        );
    }
}
