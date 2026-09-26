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
    normalize_release_language_token(&upper).map(str::to_string)
}

fn normalize_release_language_token(token: &str) -> Option<&'static str> {
    match token {
        "EN" | "ENG" | "ENGLISH" | "EN-GB" => Some("eng"),
        "JA" | "JP" | "JPN" | "JAP" | "JAPANESE" => Some("jpn"),
        "FR" | "FRA" | "FRE" | "FRENCH" | "TRUEFRENCH" | "VF" | "VF2" | "VFF" | "VFQ" => {
            Some("fra")
        }
        "DE" | "DEU" | "GER" | "GERMAN" | "SWISSGERMAN" => Some("deu"),
        "ES" | "SPA" | "ESP" | "SPANISH" | "ESPANOL" | "ESPAÑOL" | "CASTELLANO" => Some("spa"),
        "IT" | "ITA" | "ITALIAN" => Some("ita"),
        "RU" | "RUS" | "RUSSIAN" => Some("rus"),
        // Bare `UK` is the United Kingdom in release names (`UK.BluRay`).
        "UKR" | "UKRAINIAN" => Some("ukr"),
        "PT" | "POR" | "PORTUGUESE" => Some("por"),
        "PTBR" | "POR-BR" | "PT-BR" | "BRAZILIAN" | "DUBLADO" => Some("por"),
        "LATINO" | "LAT" => Some("spa"),
        "PL" | "POL" | "POLISH" | "PLLEK" | "LEKPL" | "PLDUB" | "DUBPL" => Some("pol"),
        "FI" | "FIN" | "FINNISH" => Some("fin"),
        "HU" | "HUN" | "HUNGARIAN" => Some("hun"),
        "HE" | "HEB" | "HEBREW" => Some("heb"),
        "ZH" | "ZHO" | "CHI" | "CHINESE" | "CHS" | "CHT" | "BIG5" | "GB" => Some("zho"),
        "KO" | "KOR" | "KOREAN" | "KORSUB" | "KORSUBS" => Some("kor"),
        "RO" | "RON" | "RUM" | "ROMANIAN" | "RODUBBED" => Some("ron"),
        "SV" | "SWE" | "SWEDISH" => Some("swe"),
        "NOR" | "NORWEGIAN" => Some("nor"),
        "DA" | "DAN" | "DANISH" => Some("dan"),
        "NL" | "NLD" | "DUTCH" => Some("nld"),
        "CS" | "CES" | "CZECH" => Some("ces"),
        "TR" | "TUR" | "TURKISH" => Some("tur"),
        "BG" | "BUL" | "BULGARIAN" | "BGAUDIO" => Some("bul"),
        "HI" | "HIN" | "HINDI" => Some("hin"),
        "TH" | "THA" | "THAI" => Some("tha"),
        "AR" | "ARA" => Some("ara"),
        "IS" | "ISL" | "ICELANDIC" => Some("isl"),
        "LV" | "LAV" | "LATVIAN" => Some("lav"),
        "LT" | "LIT" | "LITHUANIAN" => Some("lit"),
        "VI" | "VIE" | "VIETNAMESE" => Some("vie"),
        "CA" | "CAT" | "CATALAN" => Some("cat"),
        "KA" | "KAT" | "GEORGIAN" => Some("kat"),
        _ => None,
    }
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

/// Strict language resolution for free-text contexts such as audio track titles.
///
/// Resolves only inputs that map to a *known* language via the ISO tables, the
/// Scryer variants, or the release-token aliases. Unlike
/// [`normalize_detected_audio_language_code`], this deliberately omits the
/// 3-letter passthrough fallback, so codec/technical tokens (e.g. "DTS", "AAC",
/// "AC3") are NOT misread as languages when scanning a track title token by token.
pub fn normalize_known_audio_language_code(code: &str) -> Option<String> {
    let normalized = code.trim().replace('_', "-");
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("und") {
        return None;
    }

    match normalized.to_ascii_lowercase().as_str() {
        "ea" => return Some("spa".to_string()),
        "pob" => return Some("por".to_string()),
        "zht" => return Some("zho".to_string()),
        // "LAT" is the release/scene abbreviation for Latino (Latin-American
        // Spanish), not the dead language Latin. Resolve it before the ISO-exact
        // lookup, which would otherwise map it to "lat" (Latin).
        "lat" => return Some("spa".to_string()),
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
}

/// Normalize a language selected for metadata hydration.
///
/// Metadata providers are deliberately limited to the language choices exposed
/// by Scryer's metadata-language picker, rather than accepting arbitrary ISO
/// or release aliases intended for media-file detection.
pub fn normalize_metadata_language_code(code: &str) -> Option<String> {
    let normalized = code.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "eng" | "spa" | "fra" | "deu" | "ita" | "por" | "kor" | "zho" | "jpn" | "nld"
    )
    .then_some(normalized)
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
        normalize_metadata_language_code,
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
    fn normalizes_only_metadata_picker_languages() {
        for language in [
            "eng", "spa", "fra", "deu", "ita", "por", "kor", "zho", "jpn", "nld",
        ] {
            assert_eq!(
                normalize_metadata_language_code(&language.to_ascii_uppercase()).as_deref(),
                Some(language),
            );
        }

        for language in ["en", "en-US", "rus", "pob", "und", "", "  "] {
            assert_eq!(
                normalize_metadata_language_code(language),
                None,
                "{language}"
            );
        }
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
