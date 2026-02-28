use regorus::{Engine, Value};

/// Block score constant matching `quality_profile::BLOCK_SCORE`.
const BLOCK_SCORE: i64 = -10_000;

/// Register all Scryer-specific builtins on the engine.
pub(crate) fn register_builtins(engine: &mut Engine) {
    // scryer.block_score() → -10000
    engine
        .add_extension(
            "scryer.block_score".to_string(),
            0,
            Box::new(|_params: Vec<Value>| Ok(Value::from(BLOCK_SCORE))),
        )
        .expect("failed to register scryer.block_score");

    // scryer.size_gib(bytes) → float GiB
    engine
        .add_extension(
            "scryer.size_gib".to_string(),
            1,
            Box::new(|params: Vec<Value>| {
                let bytes = params
                    .first()
                    .and_then(|v| v.as_i64().ok())
                    .unwrap_or(0);
                let gib = (bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                Ok(Value::from(gib))
            }),
        )
        .expect("failed to register scryer.size_gib");

    // scryer.lang_matches(lang_code, pattern) → bool
    // Matches ISO 639-3 codes with common aliases.
    engine
        .add_extension(
            "scryer.lang_matches".to_string(),
            2,
            Box::new(|params: Vec<Value>| {
                let code = params
                    .first()
                    .and_then(|v| v.as_string().ok())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                let pattern = params
                    .get(1)
                    .and_then(|v| v.as_string().ok())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();

                let matches = lang_code_matches(&code, &pattern);
                Ok(Value::from(matches))
            }),
        )
        .expect("failed to register scryer.lang_matches");

    // scryer.normalize_source(raw) → normalized source string
    engine
        .add_extension(
            "scryer.normalize_source".to_string(),
            1,
            Box::new(|params: Vec<Value>| {
                let raw = params
                    .first()
                    .and_then(|v| v.as_string().ok())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let normalized = normalize_source_str(&raw);
                Ok(Value::from(normalized))
            }),
        )
        .expect("failed to register scryer.normalize_source");

    // scryer.normalize_codec(raw) → normalized codec string
    engine
        .add_extension(
            "scryer.normalize_codec".to_string(),
            1,
            Box::new(|params: Vec<Value>| {
                let raw = params
                    .first()
                    .and_then(|v| v.as_string().ok())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let normalized = normalize_codec_str(&raw);
                Ok(Value::from(normalized))
            }),
        )
        .expect("failed to register scryer.normalize_codec");
}

/// Check if a language code matches a pattern, accounting for ISO 639 aliases.
fn lang_code_matches(code: &str, pattern: &str) -> bool {
    if code == pattern {
        return true;
    }

    let code_canonical = canonical_lang(code);
    let pattern_canonical = canonical_lang(pattern);

    code_canonical == pattern_canonical
}

/// Map common language codes/names to a canonical ISO 639-3 form.
fn canonical_lang(input: &str) -> &str {
    match input {
        "en" | "eng" | "english" => "eng",
        "jp" | "ja" | "jpn" | "jap" | "japanese" => "jpn",
        "fr" | "fra" | "fre" | "french" => "fra",
        "de" | "deu" | "ger" | "german" => "deu",
        "es" | "spa" | "esp" | "spanish" | "latino" | "lat" => "spa",
        "it" | "ita" | "italian" => "ita",
        "ru" | "rus" | "russian" => "rus",
        "pt" | "por" | "portuguese" | "ptbr" | "pt-br" => "por",
        "pl" | "pol" | "polish" => "pol",
        "zh" | "zho" | "chi" | "chinese" => "zho",
        "ko" | "kor" | "korean" => "kor",
        "sv" | "swe" | "swedish" => "swe",
        "no" | "nor" | "norwegian" => "nor",
        "da" | "dan" | "danish" => "dan",
        "nl" | "nld" | "dutch" => "nld",
        "fi" | "fin" | "finnish" => "fin",
        "hu" | "hun" | "hungarian" => "hun",
        "he" | "heb" | "hebrew" => "heb",
        other => other,
    }
}

/// Source normalization matching the release parser's logic.
fn normalize_source_str(raw: &str) -> String {
    let upper = raw.to_ascii_uppercase().replace('-', "");
    match upper.as_str() {
        "WEBRIP" => "WEBRIP".to_string(),
        "WEBDL" | "WEB" | "WEB_DL" => "WEB-DL".to_string(),
        "BLURAY" | "BLU" | "BD" | "UHD" => "BLURAY".to_string(),
        "HDTV" => "HDTV".to_string(),
        _ => upper,
    }
}

/// Codec normalization matching the release parser's logic.
fn normalize_codec_str(raw: &str) -> String {
    match raw.to_ascii_uppercase().as_str() {
        "H264" => "H.264".to_string(),
        "H265" | "X265" => "H.265".to_string(),
        "X264" => "H.264".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_matches_same_code() {
        assert!(lang_code_matches("eng", "eng"));
    }

    #[test]
    fn lang_matches_alias_to_canonical() {
        assert!(lang_code_matches("jpn", "ja"));
        assert!(lang_code_matches("jpn", "japanese"));
        assert!(lang_code_matches("ja", "jpn"));
    }

    #[test]
    fn lang_no_match() {
        assert!(!lang_code_matches("eng", "jpn"));
    }

    #[test]
    fn normalize_source_variants() {
        assert_eq!(normalize_source_str("WEB-DL"), "WEB-DL");
        assert_eq!(normalize_source_str("webdl"), "WEB-DL");
        assert_eq!(normalize_source_str("BluRay"), "BLURAY");
        assert_eq!(normalize_source_str("BD"), "BLURAY");
    }

    #[test]
    fn normalize_codec_variants() {
        assert_eq!(normalize_codec_str("H264"), "H.264");
        assert_eq!(normalize_codec_str("x265"), "H.265");
        assert_eq!(normalize_codec_str("AV1"), "AV1");
    }
}
