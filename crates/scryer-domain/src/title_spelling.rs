//! In-memory title comparisons. These are deliberately independent of persisted
//! catalog ordering: sorting may remove articles, identity matching must not.

use icu_collator::{
    Collator, CollatorBorrowed,
    options::{CollatorOptions, Strength},
};
use icu_locale::Locale;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellingEquivalence {
    Exact,
    Locale(&'static str),
    Different,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TitleScript {
    Latin,
    Cyrillic,
    Cjk,
    Other,
}

pub fn title_script(value: &str) -> TitleScript {
    let mut script = None;
    for ch in value.chars().filter(|ch| ch.is_alphabetic()) {
        let current = match ch as u32 {
            0x41..=0x7a | 0xc0..=0x24f | 0x1e00..=0x1eff => TitleScript::Latin,
            0x400..=0x52f => TitleScript::Cyrillic,
            0x1100..=0x11ff
            | 0x2e80..=0x2fff
            | 0x3040..=0x30ff
            | 0x3100..=0x318f
            | 0x31a0..=0x31ff
            | 0x3400..=0x9fff
            | 0xa960..=0xa97f
            | 0xac00..=0xd7ff
            | 0xf900..=0xfaff
            | 0x20000..=0x323af => TitleScript::Cjk,
            _ => TitleScript::Other,
        };
        script = Some(match (script, current) {
            (None, next) => next,
            (Some(previous), next) if previous == next => next,
            (Some(TitleScript::Latin), TitleScript::Cjk)
            | (Some(TitleScript::Cjk), TitleScript::Latin) => TitleScript::Cjk,
            _ => TitleScript::Other,
        });
    }
    script.unwrap_or(TitleScript::Other)
}

/// Preserve diacritics and native letters; normalize only representation and
/// word separators. Remaining combining marks belong to the preceding letter.
pub fn normalize_title_spelling(value: &str) -> String {
    let mut result = String::new();
    for ch in value.nfkc().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || is_combining_mark(ch) {
            result.push(ch);
        } else if (ch.is_whitespace()
            || matches!(
                ch,
                '.' | ','
                    | ':'
                    | ';'
                    | '-'
                    | '_'
                    | '/'
                    | '\\'
                    | '&'
                    | '+'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '\''
                    | '"'
                    | '!'
                    | '?'
                    | '~'
                    | '’'
                    | '‘'
                    | '“'
                    | '”'
                    | '–'
                    | '—'
                    | '−'
                    | '・'
                    | '。'
                    | '、'
            ))
            && !result.ends_with(' ')
            && !result.is_empty()
        {
            result.push(' ');
        }
    }
    result.trim().to_string()
}

type MatchCollator = Arc<CollatorBorrowed<'static>>;
static COLLATOR_CACHE: LazyLock<Mutex<HashMap<&'static str, MatchCollator>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn collator(tag: &'static str) -> Option<MatchCollator> {
    if let Some(cached) = COLLATOR_CACHE.lock().ok()?.get(tag) {
        return Some(cached.clone());
    }
    let mut options = CollatorOptions::default();
    options.strength = Some(match tag {
        "ru" => Strength::Secondary,
        "ja" | "ko" | "zh" => Strength::Tertiary,
        _ => Strength::Primary,
    });
    let locale: Locale = tag.parse().ok()?;
    let value = Arc::new(Collator::try_new(locale.into(), options).ok()?);
    COLLATOR_CACHE.lock().ok()?.insert(tag, value.clone());
    Some(value)
}

fn profile(language: Option<&str>, script: TitleScript) -> Option<&'static str> {
    let language = language
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    let root = language.split('-').next().unwrap_or("");
    match root {
        "en" | "eng" => Some("en"),
        "de" | "deu" | "ger" => Some("de"),
        "fr" | "fra" | "fre" => Some("fr"),
        "es" | "spa" => Some("es"),
        "it" | "ita" => Some("it"),
        "pt" | "por" | "pob" => Some("pt"),
        "ru" | "rus" => Some("ru"),
        "ja" | "jpn" => Some("ja"),
        "ko" | "kor" => Some("ko"),
        "zh" | "zho" | "chi" => Some("zh"),
        _ if script == TitleScript::Latin => Some("und"),
        _ => None,
    }
}

/// Profiles used for a catalog spelling. Discovery and proof must use the
/// same collations, including the German phonebook expansion fallback.
pub fn title_spelling_profiles(value: &str, language: Option<&str>) -> Vec<&'static str> {
    let script = title_script(value);
    let Some(tag) = profile(language, script) else {
        return Vec::new();
    };
    if script == TitleScript::Other
        || (script == TitleScript::Cyrillic && tag != "ru")
        || (script == TitleScript::Cjk && !matches!(tag, "ja" | "ko" | "zh"))
    {
        return Vec::new();
    }
    let mut profiles = vec![tag];
    if script == TitleScript::Latin && (tag == "de" || value.contains(['ä', 'ö', 'ü'])) {
        profiles.push("de-u-co-phonebk");
    }
    profiles
}

/// Ephemeral lookup key for spelling discovery. Never persist these keys or
/// use catalog-sort keys, whose article handling has different semantics.
pub fn title_spelling_key(value: &str, profile: &'static str) -> Option<Vec<u8>> {
    let mut key = Vec::new();
    collator(profile)?.write_sort_key_to(value, &mut key).ok()?;
    Some(key)
}

/// `None` means no supported profile/data; callers must not turn that failure
/// into an unguarded fuzzy match. Inputs are complete normalized title strings.
pub fn compare_title_spelling(
    left: &str,
    right: &str,
    language: Option<&str>,
) -> Option<SpellingEquivalence> {
    if left == right {
        return Some(SpellingEquivalence::Exact);
    }
    let script = title_script(right);
    if title_script(left) != script || script == TitleScript::Other {
        return None;
    }
    let profiles = title_spelling_profiles(right, language);
    if profiles.is_empty() {
        return None;
    }
    for tag in profiles {
        if collator(tag)?.compare(left, right).is_eq() {
            return Some(SpellingEquivalence::Locale(tag));
        }
    }
    Some(SpellingEquivalence::Different)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multilingual_equivalents_and_distinctions() {
        for (language, left, right) in [
            ("en", "Ｔｈｅ　Harbor", "the harbor"),
            ("de", "Die zwei Paepste", "Die zwei Päpste"),
            ("de", "Goetter ueber der Strasse", "Götter über der Straße"),
            ("fr", "Le coeur de Chloe", "Le cœur de Chloé"),
            ("es", "El ultimo dia", "El último día"),
            ("it", "L’amore in città", "L'amore in citta"),
            ("pt", "Coracao de acucar", "Coração de açúcar"),
            ("ru", "Маи\u{306}скии\u{306} вечер", "Майский вечер"),
            ("zh", "流浪地球２", "流浪地球2"),
            ("ja", "ｶﾞﾗｽの城", "ガラスの城"),
            ("ko", "한글", "한글"),
        ] {
            let result = compare_title_spelling(
                &normalize_title_spelling(left),
                &normalize_title_spelling(right),
                Some(language),
            );
            assert!(
                matches!(
                    result,
                    Some(SpellingEquivalence::Exact | SpellingEquivalence::Locale(_))
                ),
                "{language}: {left} / {right}: {result:?}"
            );
        }
        for (language, left, right) in [
            ("es", "ano", "año"),
            ("ru", "маи", "май"),
            ("ja", "かく", "がく"),
            ("ja", "つき", "っき"),
            ("ko", "달", "탈"),
            ("zh", "大地", "天地"),
        ] {
            assert_eq!(
                compare_title_spelling(left, right, Some(language)),
                Some(SpellingEquivalence::Different),
                "{language}: {left} / {right}"
            );
        }
        assert_eq!(compare_title_spelling("harbor", "hаrbor", Some("en")), None);
        assert_eq!(compare_title_spelling("かく", "がく", Some("ru")), None);
        assert_eq!(compare_title_spelling("маи", "май", Some("ja")), None);
    }
}
