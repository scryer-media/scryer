use std::collections::HashMap;
use std::sync::OnceLock;

use crate::media_language_data::ISO6392_LANGUAGE_ENTRIES;
use crate::subtitles::normalize_subtitle_language_code;

#[derive(Debug)]
struct LanguageLookupTables {
    exact_to_canonical: HashMap<String, String>,
}

static LANGUAGE_LOOKUP_TABLES: OnceLock<LanguageLookupTables> = OnceLock::new();

fn language_lookup_tables() -> &'static LanguageLookupTables {
    LANGUAGE_LOOKUP_TABLES.get_or_init(build_language_lookup_tables)
}

fn build_language_lookup_tables() -> LanguageLookupTables {
    let mut exact_to_canonical = HashMap::<String, String>::new();

    for entry in ISO6392_LANGUAGE_ENTRIES {
        let canonical = entry.canonical.trim().to_ascii_lowercase();
        if canonical.is_empty() {
            continue;
        }

        for alias in [
            entry.canonical,
            entry.bibliographic,
            entry.two_letter,
            entry.english_name,
        ] {
            for value in [alias, alias.split(';').next().unwrap_or(alias)] {
                let key = language_lookup_key(value);
                if key.is_empty() {
                    continue;
                }
                exact_to_canonical
                    .entry(key)
                    .or_insert_with(|| canonical.clone());
            }
        }
    }

    LanguageLookupTables { exact_to_canonical }
}

fn language_lookup_key(value: &str) -> String {
    value.trim().replace('_', "-").to_ascii_lowercase()
}

fn normalize_with_primary_subtag<F>(code: &str, normalize: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    normalize(code).or_else(|| {
        code.split_once('-')
            .and_then(|(primary, _)| normalize(primary))
    })
}

fn normalize_release_language_code(code: &str) -> Option<String> {
    let upper = code.trim().replace('_', "-").to_ascii_uppercase();
    scryer_release_parser::normalize_language_token(&upper).map(str::to_string)
}

fn normalize_iso_language_code_exact(code: &str) -> Option<String> {
    language_lookup_tables()
        .exact_to_canonical
        .get(&language_lookup_key(code))
        .cloned()
}

fn normalize_generic_app_language_code(code: &str) -> Option<String> {
    let normalized = code.trim().replace('_', "-");
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("und") {
        return None;
    }

    match normalized.to_ascii_lowercase().as_str() {
        // Scryer subtitle-specific variants should collapse to a generic app-facing code.
        "ea" => return Some("spa".to_string()),
        "pob" => return Some("por".to_string()),
        "zht" => return Some("zho".to_string()),
        _ => {}
    }

    normalize_iso_language_code_exact(&normalized)
        .or_else(|| normalize_release_language_code(&normalized))
        .or_else(|| {
            normalized
                .split_once('-')
                .and_then(|(primary, _)| normalize_iso_language_code_exact(primary))
        })
        .or_else(|| {
            normalized
                .split_once('-')
                .and_then(|(primary, _)| normalize_release_language_code(primary))
        })
        .or_else(|| {
            let primary = normalized.split('-').next().unwrap_or(normalized.as_str());
            if primary.len() == 3 && primary.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                Some(primary.to_ascii_lowercase())
            } else {
                None
            }
        })
}

pub fn normalize_detected_audio_language_code(code: &str) -> Option<String> {
    normalize_generic_app_language_code(code)
}

pub fn normalize_detected_audio_languages<'a>(
    languages: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for language in languages {
        if let Some(code) = normalize_detected_audio_language_code(language)
            && !normalized.contains(&code)
        {
            normalized.push(code);
        }
    }
    normalized
}

pub fn normalize_detected_subtitle_language_code(code: &str) -> Option<String> {
    let normalized = code.trim().replace('_', "-");
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("und") {
        return None;
    }

    normalize_with_primary_subtag(&normalized, normalize_subtitle_language_code)
        .or_else(|| normalize_generic_app_language_code(&normalized))
}

pub fn normalize_detected_subtitle_languages<'a>(
    languages: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for language in languages {
        if let Some(code) = normalize_detected_subtitle_language_code(language)
            && !normalized.contains(&code)
        {
            normalized.push(code);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_detected_audio_language_code, normalize_detected_audio_languages,
        normalize_detected_subtitle_language_code, normalize_detected_subtitle_languages,
    };
    use crate::media_language_data::ISO6392_LANGUAGE_ENTRIES;

    #[test]
    fn normalizes_detected_audio_languages_with_full_iso_coverage() {
        assert_eq!(
            normalize_detected_audio_language_code("en-US").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_audio_language_code("ja-JP").as_deref(),
            Some("jpn")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fre").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_detected_audio_language_code("de-DE").as_deref(),
            Some("deu")
        );
        assert_eq!(
            normalize_detected_audio_language_code("pt-BR").as_deref(),
            Some("por")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fr-CA").as_deref(),
            Some("frc")
        );
        assert_eq!(
            normalize_detected_audio_language_code("tgl").as_deref(),
            Some("tgl")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fil").as_deref(),
            Some("fil")
        );
        assert_eq!(
            normalize_detected_audio_language_code("zxx").as_deref(),
            Some("zxx")
        );
        assert_eq!(normalize_detected_audio_language_code("und"), None);
    }

    #[test]
    fn normalizes_every_iso_audio_alias_to_canonical_code() {
        let lookup_tables = super::language_lookup_tables();

        for entry in ISO6392_LANGUAGE_ENTRIES {
            if entry.canonical == "und" {
                continue;
            }

            for alias in [
                entry.canonical,
                entry.bibliographic,
                entry.english_name,
                entry
                    .english_name
                    .split(';')
                    .next()
                    .unwrap_or(entry.english_name),
            ] {
                if alias.trim().is_empty() {
                    continue;
                }
                assert_eq!(
                    normalize_detected_audio_language_code(alias).as_deref(),
                    Some(entry.canonical),
                    "alias {alias:?} should normalize to {}",
                    entry.canonical
                );
            }

            if !entry.two_letter.trim().is_empty()
                && lookup_tables
                    .exact_to_canonical
                    .get(&super::language_lookup_key(entry.two_letter))
                    .map(String::as_str)
                    == Some(entry.canonical)
            {
                assert_eq!(
                    normalize_detected_audio_language_code(entry.two_letter).as_deref(),
                    Some(entry.canonical),
                    "two-letter alias {:?} should normalize to {}",
                    entry.two_letter,
                    entry.canonical
                );
            }
        }
    }

    #[test]
    fn normalizes_detected_subtitle_languages_with_scryer_overrides() {
        assert_eq!(
            normalize_detected_subtitle_language_code("en-US").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("ja-JP").as_deref(),
            Some("jpn")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("pt-BR").as_deref(),
            Some("pob")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("zh-TW").as_deref(),
            Some("zht")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("ace").as_deref(),
            Some("ace")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("Filipino").as_deref(),
            Some("fil")
        );
        assert_eq!(
            normalize_detected_subtitle_language_code("zxx").as_deref(),
            Some("zxx")
        );
        assert_eq!(normalize_detected_subtitle_language_code("und"), None);
    }

    #[test]
    fn dedupes_normalized_language_lists_in_order() {
        assert_eq!(
            normalize_detected_audio_languages(["eng", "en-US", "jpn", "ja-JP"]),
            vec!["eng".to_string(), "jpn".to_string()]
        );
        assert_eq!(
            normalize_detected_subtitle_languages(["eng", "en-US", "pt-BR", "pob"]),
            vec!["eng".to_string(), "pob".to_string()]
        );
    }
}
