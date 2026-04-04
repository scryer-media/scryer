use crate::{ParsedReleaseMetadata, normalize_detected_audio_language_code};

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

pub(crate) fn release_audio_language_hints(
    parsed: &ParsedReleaseMetadata,
    indexer_languages: Option<&[String]>,
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

    // Treat bare "DUAL" as English + Japanese only when the release does not
    // already carry explicit audio-language hints from the title or the indexer.
    if parsed.is_dual_audio && normalized.is_empty() {
        for language in ["eng", "jpn"] {
            if !normalized.iter().any(|existing| existing == language) {
                normalized.push(language.to_string());
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{
        missing_required_audio_languages, normalize_required_audio_languages,
        release_audio_language_hints,
    };
    use crate::release_parser::parse_release_metadata;

    #[test]
    fn dual_audio_without_explicit_languages_implies_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        assert_eq!(
            release_audio_language_hints(&parsed, None),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn explicit_languages_prevent_dual_audio_fallback() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO ENG 1080p");
        assert_eq!(
            release_audio_language_hints(&parsed, None),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn indexer_languages_are_merged_with_release_languages() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        assert_eq!(
            release_audio_language_hints(
                &parsed,
                Some(&["English".to_string(), "Japanese".to_string()])
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
    fn missing_languages_are_reported_in_canonical_form() {
        assert_eq!(
            missing_required_audio_languages(
                &["English".to_string(), "Japanese".to_string()],
                &["eng".to_string()]
            ),
            vec!["jpn".to_string()]
        );
    }
}
