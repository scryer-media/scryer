pub fn normalize_subtitle_language_code(code: &str) -> Option<String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return None;
    }

    let upper = trimmed.replace('_', "-").to_ascii_uppercase();
    let normalized = match upper.as_str() {
        "ALB" | "SQ" | "SQI" => "sqi",
        "ARA" | "AR" => "ara",
        "ARM" | "HY" | "HYE" => "hye",
        "BAQ" | "EU" | "EUS" => "eus",
        "BEN" | "BN" => "ben",
        "BOS" | "BS" => "bos",
        "BUL" | "BG" | "BGAUDIO" | "BG-AUDIO" => "bul",
        "BUR" | "MY" | "MYA" => "mya",
        "CAT" | "CA" => "cat",
        "CHI" | "ZH" | "ZHO" | "ZH-CN" | "CHS" | "SC" | "ZHS" | "HANS" | "GB" => "zho",
        "CHT" | "TC" | "ZHT" | "HANT" | "BIG5" | "ZH-TW" => "zht",
        "CES" | "CS" | "CZE" => "ces",
        "DAN" | "DA" | "DK" => "dan",
        "DE" | "DEU" | "GER" | "GERMAN" => "deu",
        "DUT" | "NL" | "NLD" => "nld",
        "EA" | "ES-MX" => "ea",
        "EL" | "ELL" | "GRE" => "ell",
        "EN" | "ENG" | "EN-GB" | "EN-US" => "eng",
        "ES" | "SPA" | "ESP" => "spa",
        "EST" | "ET" => "est",
        "FA" | "FAS" | "PER" => "fas",
        "FI" | "FIN" => "fin",
        "FRA" | "FR" | "FRE" | "VF" | "VF2" | "VFF" | "VFQ" => "fra",
        "GEO" | "KA" | "KAT" => "kat",
        "HE" | "HEB" | "IW" => "heb",
        "HI" | "HIN" => "hin",
        "HR" | "HRV" => "hrv",
        "HU" | "HUN" => "hun",
        "ICE" | "IS" | "ISL" => "isl",
        "ID" | "IND" => "ind",
        "IT" | "ITA" => "ita",
        "JA" | "JPN" | "JP" => "jpn",
        "KO" | "KOR" | "KORSUB" | "KORSUBS" => "kor",
        "LAV" | "LV" => "lav",
        "LIT" | "LT" => "lit",
        "MAC" | "MK" | "MKD" => "mkd",
        "MAY" | "MS" | "MSA" => "msa",
        "NOR" | "NB" | "NN" | "NO" => "nor",
        "PL" | "POL" => "pol",
        "POB" | "PB" | "PT-BR" => "pob",
        "POR" | "PT" | "PT-PT" => "por",
        "RO" | "RON" | "RUM" | "RODUBBED" => "ron",
        "RU" | "RUS" => "rus",
        "SCC" | "SR" | "SRP" => "srp",
        "SIN" | "SI" => "sin",
        "SK" | "SLK" | "SLO" => "slk",
        "SLV" | "SL" => "slv",
        "SV" | "SWE" => "swe",
        "TH" | "THA" => "tha",
        "TR" | "TUR" => "tur",
        "UK" | "UKR" => "ukr",
        "UR" | "URD" => "urd",
        "VI" | "VIE" => "vie",
        _ if upper.len() == 3 && upper.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
            return Some(upper.to_ascii_lowercase());
        }
        _ => return None,
    };

    Some(normalized.to_string())
}

pub fn same_subtitle_language(a: &str, b: &str) -> bool {
    match (
        normalize_subtitle_language_code(a),
        normalize_subtitle_language_code(b),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

pub fn to_opensubtitles_language(code: &str) -> Option<String> {
    let normalized = normalize_subtitle_language_code(code)?;
    Some(match normalized.as_str() {
        "ea" => "es-MX".to_string(),
        "pob" => "pt-BR".to_string(),
        "por" => "pt-PT".to_string(),
        "zho" => "zh-CN".to_string(),
        "zht" => "zh-TW".to_string(),
        other => other.to_string(),
    })
}

pub fn from_opensubtitles_language(code: &str) -> Option<String> {
    match code.trim().replace('_', "-").as_str() {
        "pt-PT" => Some("por".to_string()),
        "zh-CN" => Some("zho".to_string()),
        "zh-TW" => Some("zht".to_string()),
        "es-MX" => Some("ea".to_string()),
        other => normalize_subtitle_language_code(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_legacy_and_provider_codes() {
        assert_eq!(
            normalize_subtitle_language_code("fre").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_subtitle_language_code("ger").as_deref(),
            Some("deu")
        );
        assert_eq!(
            normalize_subtitle_language_code("pt-PT").as_deref(),
            Some("por")
        );
        assert_eq!(
            normalize_subtitle_language_code("pt-BR").as_deref(),
            Some("pob")
        );
        assert_eq!(
            normalize_subtitle_language_code("zh-CN").as_deref(),
            Some("zho")
        );
        assert_eq!(
            normalize_subtitle_language_code("zh-TW").as_deref(),
            Some("zht")
        );
        assert_eq!(
            normalize_subtitle_language_code("es-MX").as_deref(),
            Some("ea")
        );
    }

    #[test]
    fn maps_to_and_from_opensubtitles_codes() {
        assert_eq!(to_opensubtitles_language("por").as_deref(), Some("pt-PT"));
        assert_eq!(to_opensubtitles_language("pob").as_deref(), Some("pt-BR"));
        assert_eq!(to_opensubtitles_language("zho").as_deref(), Some("zh-CN"));
        assert_eq!(to_opensubtitles_language("zht").as_deref(), Some("zh-TW"));
        assert_eq!(to_opensubtitles_language("ea").as_deref(), Some("es-MX"));
        assert_eq!(from_opensubtitles_language("pt-PT").as_deref(), Some("por"));
        assert_eq!(from_opensubtitles_language("pt-BR").as_deref(), Some("pob"));
        assert_eq!(from_opensubtitles_language("zh-CN").as_deref(), Some("zho"));
        assert_eq!(from_opensubtitles_language("zh-TW").as_deref(), Some("zht"));
        assert_eq!(from_opensubtitles_language("es-MX").as_deref(), Some("ea"));
    }

    #[test]
    fn compares_language_aliases() {
        assert!(same_subtitle_language("fre", "fra"));
        assert!(same_subtitle_language("pt-PT", "por"));
        assert!(same_subtitle_language("zh-CN", "zho"));
        assert!(!same_subtitle_language("por", "pob"));
    }
}
