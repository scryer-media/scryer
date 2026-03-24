use chrono::{Datelike, NaiveDate, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedSpecialKind {
    Special,
    OVA,
    OVD,
    NCOP,
    NCED,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParsedEpisodeReleaseType {
    SingleEpisode,
    MultiEpisode,
    SeasonPack,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedEpisodeMetadata {
    pub season: Option<u32>,
    pub episode_numbers: Vec<u32>,
    pub absolute_episode: Option<u32>,
    pub air_date: Option<NaiveDate>,
    pub daily_part: Option<u32>,
    pub absolute_episode_numbers: Vec<u32>,
    pub special_absolute_episode_numbers: Vec<u32>,
    pub full_season: bool,
    pub is_partial_season: bool,
    pub is_multi_season: bool,
    pub season_part: Option<u32>,
    pub is_season_extra: bool,
    pub is_split_episode: bool,
    pub is_mini_series: bool,
    pub special_kind: Option<ParsedSpecialKind>,
    pub release_type: ParsedEpisodeReleaseType,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedReleaseMetadata {
    pub raw_title: String,
    pub normalized_title: String,
    pub normalized_title_variants: Vec<String>,
    pub release_group: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<u32>,
    pub year: Option<u32>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub audio_codecs: Vec<String>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub streaming_service: Option<String>,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub episode: Option<ParsedEpisodeMetadata>,
    pub parser_version: &'static str,
    pub parse_confidence: f32,
    pub missing_fields: Vec<String>,
    pub parse_hints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAudio {
    codec: &'static str,
    channels: Option<String>,
}

const RELEASE_PARSER_VERSION: &str = "2026.02.7";
const LANG_AUDIO_MARKERS: &[&str] = &[
    "AUDIO",
    "DUB",
    "DUBS",
    "DUBBED",
    "DUAL",
    "DUAL-AUDIO",
    "DUALAUDIO",
    "DUALDUB",
    "DUALDUBBED",
    "DUB-LANG",
];

fn is_reasonable_episode_number(value: u32) -> bool {
    (1..=2000).contains(&value)
}

fn is_reasonable_episode_series(value: &str) -> bool {
    is_reasonable_episode_number(value.parse::<u32>().ok().unwrap_or(0))
}

const LANG_SUBTITLE_MARKERS: &[&str] = &[
    "SUB",
    "SUBS",
    "SUBBED",
    "SUBTITLE",
    "SUBTITLES",
    "VOST",
    "VOSTFR",
    "CC",
];

fn dedupe_keep_order(mut values: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for value in values.drain(..) {
        let mut is_duplicate = false;
        for existing in &out {
            if existing.eq_ignore_ascii_case(&value) {
                is_duplicate = true;
                break;
            }
        }

        if !is_duplicate {
            out.push(value);
        }
    }

    out
}

pub fn normalize_language_token(token: &str) -> Option<&'static str> {
    match token {
        "EN" | "ENG" | "ENGLISH" => Some("eng"),
        "EN-GB" => Some("eng"),
        "JA" | "JP" | "JPN" | "JAP" | "JAPANESE" => Some("jpn"),
        "FR" | "FRA" | "FRE" | "FRENCH" | "TRUEFRENCH" | "VF" | "VF2" | "VFF" | "VFQ" => {
            Some("fra")
        }
        "DE" | "DEU" | "GER" | "GERMAN" | "SWISSGERMAN" => Some("deu"),
        "ES" | "SPA" | "ESP" | "SPANISH" | "ESPANOL" | "ESPAÑOL" | "CASTELLANO" => {
            Some("spa")
        }
        "IT" | "ITA" | "ITALIAN" => Some("ita"),
        "RU" | "RUS" | "RUSSIAN" => Some("rus"),
        "PT" | "POR" | "PORTUGUESE" => Some("por"),
        "PTBR" | "POR-BR" | "PT-BR" | "BRAZILIAN" | "DUBLADO" => Some("por"),
        "LATINO" | "LAT" => Some("spa"),
        "PL" | "POL" | "POLISH" | "PLLEK" | "LEKPL" | "PLDUB" | "DUBPL" => Some("pol"),
        "FI" | "FIN" | "FINNISH" => Some("fin"),
        "HU" | "HUN" | "HUNGARIAN" => Some("hun"),
        "HE" | "HEB" | "HEBREW" => Some("heb"),
        "ZH" | "ZHO" | "CHI" | "CHINESE" | "CHS" | "CHT" | "BIG5" | "GB" => Some("zho"),
        "KO" | "KOR" | "KOREAN" => Some("kor"),
        "KORSUB" | "KORSUBS" => Some("kor"),
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

fn parse_named_season_token(token: &str) -> Option<u32> {
    if let Some(rest) = token.strip_prefix("SEASON") {
        if rest.is_empty() {
            return None;
        }
        let rest = rest.trim_start_matches(['-', '.', '_', ':']);
        let (season, rest) = parse_leading_digits(rest)?;
        if rest
            .trim_matches(|ch: char| ch == '-' || ch == '.' || ch == '_' || ch == ':')
            .is_empty()
        {
            return Some(season);
        }
    }
    None
}

fn parse_named_episode_token(token: &str) -> Vec<u32> {
    for prefix in ["EPISODE", "EP", "E"] {
        if let Some(stripped) = token.strip_prefix(prefix) {
            if stripped.is_empty() {
                return Vec::new();
            }
            return parse_episode_fragment(stripped);
        }
    }

    Vec::new()
}

fn parse_named_episode_anchor_token(token: &str) -> bool {
    matches!(token, "E" | "EP" | "EPISODE" | "EPISODES" | "CHAPTER")
}

fn dedupe_u32(values: Vec<u32>) -> Vec<u32> {
    let mut seen = Vec::<u32>::new();

    for value in values {
        if seen.iter().all(|existing| *existing != value) {
            seen.push(value);
        }
    }

    seen
}

fn parse_episode_fragment(fragment: &str) -> Vec<u32> {
    let mut episodes = Vec::new();
    let bytes = fragment.as_bytes();
    let mut idx = 0usize;
    let em_dash = "—".as_bytes();

    while idx < bytes.len() {
        while idx < bytes.len() && !bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }

        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }

        let left = match fragment[start..idx].parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };

        // Skip anime version suffix (V2-V9) — not a second episode number.
        if idx < bytes.len() && bytes[idx] == b'V' {
            let ver_start = idx + 1;
            if ver_start < bytes.len() && bytes[ver_start].is_ascii_digit() {
                let ver_end = ver_start + 1;
                let at_boundary = ver_end >= bytes.len() || !bytes[ver_end].is_ascii_digit();
                if at_boundary {
                    let ver = bytes[ver_start] - b'0';
                    if (2..=9).contains(&ver) {
                        episodes.push(left);
                        idx = ver_end;
                        continue;
                    }
                }
            }
        }

        let mut has_range = false;
        while idx < bytes.len() {
            if bytes[idx] == b'-' || bytes[idx] == b'~' {
                has_range = true;
                idx += 1;
                break;
            }

            if idx + em_dash.len() <= bytes.len() && &bytes[idx..idx + em_dash.len()] == em_dash {
                has_range = true;
                idx += em_dash.len();
                break;
            }

            if !bytes[idx].is_ascii_digit() {
                idx += 1;
            } else {
                break;
            }
        }

        if has_range {
            while idx < bytes.len() && !bytes[idx].is_ascii_digit() {
                idx += 1;
            }

            let right_start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }

            if let Ok(right) = fragment[right_start..idx].parse::<u32>() {
                if left <= right {
                    for value in left..=right {
                        episodes.push(value);
                    }
                } else {
                    episodes.push(left);
                    episodes.push(right);
                }
            } else {
                episodes.push(left);
            }

            continue;
        }

        episodes.push(left);
    }

    dedupe_u32(episodes)
}

fn parse_leading_digits(text: &str) -> Option<(u32, &str)> {
    let mut idx = 0usize;
    let bytes = text.as_bytes();

    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 {
        return None;
    }

    let value = text[..idx].parse::<u32>().ok()?;
    let rest = &text[idx..];
    Some((value, rest))
}

fn parse_series_only_season(token: &str) -> Option<u32> {
    if !token.starts_with('S') || token.len() <= 1 {
        return None;
    }

    let (_, tail) = token.split_at(1);
    let (season, rest) = parse_leading_digits(tail)?;
    if rest
        .trim_matches(|ch: char| ch == '-' || ch == '.' || ch == '_' || ch == ':')
        .is_empty()
    {
        return Some(season);
    }

    None
}

fn parse_numeric_token(token: &str) -> Option<u32> {
    if is_digit_str(token) {
        return token.parse::<u32>().ok();
    }

    None
}

fn parse_pending_episode_token(token: &str) -> Vec<u32> {
    if token.starts_with("EP") || token.starts_with('E') {
        let episode_token = if let Some(stripped) = token.strip_prefix("EP") {
            stripped
        } else {
            token.strip_prefix('E').unwrap_or(token)
        };
        if episode_token.is_empty() {
            return Vec::new();
        }
        return parse_episode_fragment(episode_token);
    }

    if is_digit_str(token) {
        return parse_episode_fragment(token);
    }

    Vec::new()
}

fn is_language_token(token: &str) -> bool {
    normalize_language_token(token).is_some() || token == "VOSTFR"
}

fn parse_language_token_with_affixes(token: &str) -> Option<&'static str> {
    if let Some(code) = normalize_language_token(token) {
        return Some(code);
    }

    const AFFIXES: &[&str] = &[
        "DUB",
        "DUBBED",
        "DUBS",
        "SUB",
        "SUBS",
        "SUBBED",
        "SUBTITLE",
        "SUBTITLES",
        "AUDIO",
        "CC",
        "FORCED",
    ];

    for affix in AFFIXES {
        if token.starts_with(affix) && token.len() > affix.len() {
            let tail = &token[affix.len()..];
            if let Some(code) = normalize_language_token(tail) {
                return Some(code);
            }
        }

        if token.ends_with(affix) && token.len() > affix.len() {
            let head = &token[..token.len() - affix.len()];
            if let Some(code) = normalize_language_token(head) {
                return Some(code);
            }
        }
    }

    None
}

fn parse_language_hint(token: &str) -> Option<&'static str> {
    if token == "VOSTFR" {
        return Some("fre");
    }

    if token == "KORSUB" || token == "KORSUBS" {
        return Some("kor");
    }

    if token.ends_with("SUB") || token.ends_with("SUBS") || token.contains("VOST") {
        return None;
    }

    parse_language_token_with_affixes(token).or_else(|| normalize_language_token(token))
}

impl ParsedReleaseMetadata {
    pub fn score(&self) -> u32 {
        let mut score = 0u32;

        match self.quality.as_deref() {
            Some("2160p") => score += 4000,
            Some("1080p") => score += 3000,
            Some("720p") => score += 2300,
            Some("576p") | Some("480p") => score += 1600,
            Some(_) => score += 1200,
            None => {}
        }

        if let Some(source) = self.source.as_deref() {
            if source.eq_ignore_ascii_case("WEB-DL") {
                score += 1000;
            } else if source.eq_ignore_ascii_case("WEBRip") {
                score += 850;
            } else if source.eq_ignore_ascii_case("RAWHD") || source.eq_ignore_ascii_case("HDTV") {
                score += 550;
            } else if source.eq_ignore_ascii_case("BluRay") || source.eq_ignore_ascii_case("UHD") {
                score += 850;
            } else if source.eq_ignore_ascii_case("BRDISK") {
                score += 900;
            } else if matches!(
                source.to_ascii_uppercase().as_str(),
                "CAM" | "TELESYNC" | "TELECINE" | "DVDSCR" | "WORKPRINT" | "REGIONAL"
            ) {
                score += 50;
            }
        }

        if self.video_codec.as_deref().is_some() {
            score += 300;
        }

        if self.audio.is_some() {
            score += 200;
        }

        if self.is_dolby_vision {
            score += 250;
        }

        if self.is_dual_audio {
            score += 100;
        }

        if self.is_remux {
            score += 180;
        }

        if self.is_bd_disk {
            score += 150;
        }

        if self.is_proper_upload {
            score += 80;
        }

        if self.is_ai_enhanced {
            score += 100;
        }

        score
    }
}

impl ParsedEpisodeMetadata {
    pub fn first_episode(&self) -> Option<u32> {
        self.episode_numbers
            .first()
            .copied()
            .or_else(|| self.absolute_episode_numbers.first().copied())
            .or_else(|| self.special_absolute_episode_numbers.first().copied())
    }
}

fn finalize_episode_metadata(mut metadata: ParsedEpisodeMetadata) -> ParsedEpisodeMetadata {
    metadata.absolute_episode_numbers = dedupe_u32(metadata.absolute_episode_numbers);
    metadata.special_absolute_episode_numbers = dedupe_u32(metadata.special_absolute_episode_numbers);
    metadata.episode_numbers = dedupe_u32(metadata.episode_numbers);

    if metadata.absolute_episode.is_none() {
        metadata.absolute_episode = metadata.absolute_episode_numbers.first().copied();
    }

    metadata.release_type = if metadata.full_season
        || metadata.is_partial_season
        || metadata.is_multi_season
        || metadata.is_season_extra
    {
        ParsedEpisodeReleaseType::SeasonPack
    } else if metadata.episode_numbers.len() > 1 || metadata.absolute_episode_numbers.len() > 1 {
        ParsedEpisodeReleaseType::MultiEpisode
    } else if !metadata.episode_numbers.is_empty()
        || !metadata.absolute_episode_numbers.is_empty()
        || metadata.air_date.is_some()
        || metadata.special_absolute_episode_numbers.len() == 1
    {
        ParsedEpisodeReleaseType::SingleEpisode
    } else {
        ParsedEpisodeReleaseType::Unknown
    };

    metadata
}

fn new_episode_metadata(
    season: Option<u32>,
    episode_numbers: Vec<u32>,
    absolute_episode_numbers: Vec<u32>,
    raw: Option<String>,
) -> ParsedEpisodeMetadata {
    finalize_episode_metadata(ParsedEpisodeMetadata {
        season,
        episode_numbers,
        absolute_episode: absolute_episode_numbers.first().copied(),
        absolute_episode_numbers,
        raw,
        ..ParsedEpisodeMetadata::default()
    })
}

fn new_absolute_episode_metadata(
    absolute_episode_numbers: Vec<u32>,
    raw: Option<String>,
) -> ParsedEpisodeMetadata {
    new_episode_metadata(None, Vec::new(), absolute_episode_numbers, raw)
}

fn new_season_pack_metadata(
    season: u32,
    raw: Option<String>,
    full_season: bool,
    is_partial_season: bool,
    is_multi_season: bool,
    season_part: Option<u32>,
    is_season_extra: bool,
) -> ParsedEpisodeMetadata {
    finalize_episode_metadata(ParsedEpisodeMetadata {
        season: Some(season),
        full_season,
        is_partial_season,
        is_multi_season,
        season_part,
        is_season_extra,
        raw,
        ..ParsedEpisodeMetadata::default()
    })
}

fn new_daily_episode_metadata(
    air_date: NaiveDate,
    daily_part: Option<u32>,
    raw: Option<String>,
) -> ParsedEpisodeMetadata {
    finalize_episode_metadata(ParsedEpisodeMetadata {
        air_date: Some(air_date),
        daily_part,
        raw,
        ..ParsedEpisodeMetadata::default()
    })
}

fn is_digit_str(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn is_hex_token(value: &str) -> bool {
    value.len() >= 7 && value.len() <= 10 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_known_torrent_suffix(value: &str) -> bool {
    matches!(
        value,
        "ETTV" | "RARTV" | "RARBG" | "CTTV" | "PUBLICHD" | "EZTV"
    )
}

fn is_website_like(value: &str) -> bool {
    let trimmed = value
        .trim()
        .trim_matches(&['[', ']', '(', ')', '-', ' '] as &[_]);
    if trimmed.is_empty() {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper == "NARUTO-KUN.HU" {
        return false;
    }

    if is_known_torrent_suffix(upper.as_str()) {
        return true;
    }

    let Some((head, tail)) = trimmed.rsplit_once('.') else {
        return false;
    };

    if head.is_empty() {
        return false;
    }

    let valid_tail = tail.len() >= 2
        && tail.len() <= 6
        && tail.chars().all(|character| character.is_ascii_alphabetic());
    let valid_head = head
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'));

    valid_tail && valid_head
}

fn strip_known_extension(value: &str) -> &str {
    let trimmed = value.trim();
    let Some((head, tail)) = trimmed.rsplit_once('.') else {
        return trimmed;
    };

    let ext = tail.to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "mkv"
            | "mp4"
            | "avi"
            | "ts"
            | "m2ts"
            | "mov"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "flv"
    ) {
        head
    } else {
        trimmed
    }
}

fn is_reversed_title_token(token: &str) -> bool {
    if matches!(token, "P027" | "P0801") {
        return true;
    }

    let bytes = token.as_bytes();
    if bytes.len() < 5 || bytes.len() > 7 {
        return false;
    }

    let digit_count = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if !(2..=3).contains(&digit_count) || digit_count + 2 >= bytes.len() {
        return false;
    }

    if bytes.get(digit_count) != Some(&b'E') {
        return false;
    }

    let tail = &token[digit_count + 1..];
    let tail = tail.strip_prefix('-').unwrap_or(tail);
    if tail.len() < 3 || !tail.ends_with('S') {
        return false;
    }

    let season_digits = &tail[..tail.len() - 1];
    season_digits.len() == 2 && season_digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn maybe_reverse_title(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let base = strip_known_extension(trimmed);
    let extension = &trimmed[base.len()..];
    let tokens = base
        .split(|character: char| {
            character.is_ascii_whitespace()
                || matches!(character, '.' | '_' | '-' | '[' | ']' | '(' | ')')
        })
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_uppercase())
        .collect::<Vec<_>>();

    if !tokens.iter().any(|token| is_reversed_title_token(token)) {
        return trimmed.to_string();
    }

    let reversed_base = base.chars().rev().collect::<String>();
    format!("{reversed_base}{extension}")
}

fn trim_release_separators(value: &str) -> &str {
    value.trim_start_matches([' ', '-', '.', '_'])
}

fn strip_leading_website_prefix(mut value: String) -> String {
    loop {
        let trimmed = value.trim_start().to_string();
        if trimmed.is_empty() {
            return trimmed;
        }

        let mut stripped = None::<String>;

        if let Some(rest) = trimmed.strip_prefix('[')
            && let Some(close) = rest.find(']')
        {
            let candidate = &rest[..close];
            if is_website_like(candidate) {
                stripped = Some(trim_release_separators(&rest[close + 1..]).to_string());
            }
        }

        if stripped.is_none()
            && let Some(rest) = trimmed.strip_prefix('(')
            && let Some(close) = rest.find(')')
        {
            let candidate = &rest[..close];
            if is_website_like(candidate) {
                stripped = Some(trim_release_separators(&rest[close + 1..]).to_string());
            }
        }

        if stripped.is_none()
            && let Some((prefix, rest)) = trimmed.split_once(" - ")
            && is_website_like(prefix)
        {
            stripped = Some(rest.trim_start().to_string());
        }

        if stripped.is_none()
            && let Some((prefix, rest)) = trimmed.split_once(' ')
            && is_website_like(prefix)
        {
            stripped = Some(rest.trim_start().to_string());
        }

        let Some(next) = stripped else {
            return trimmed;
        };

        if next == trimmed {
            return trimmed;
        }

        value = next;
    }
}

fn strip_trailing_website_suffix(mut value: String) -> String {
    loop {
        let trimmed = value.trim_end().to_string();
        if trimmed.is_empty() {
            return trimmed;
        }

        let mut stripped = None::<String>;

        if let Some(open) = trimmed.rfind('[')
            && trimmed.ends_with(']')
        {
            let candidate = &trimmed[open + 1..trimmed.len() - 1];
            let preserve_as_release_group = canonical_release_group_candidate(candidate).is_some();
            if !preserve_as_release_group
                && (is_website_like(candidate)
                    || is_known_torrent_suffix(&candidate.to_ascii_uppercase()))
            {
                stripped = Some(trimmed[..open].trim_end().to_string());
            }
        }

        let Some(next) = stripped else {
            return trimmed;
        };

        if next == trimmed {
            return trimmed;
        }

        value = next;
    }
}

fn sanitize_release_title(raw_title: &str) -> String {
    let replaced = raw_title
        .replace('【', "[")
        .replace('】', "]")
        .replace('／', "/");
    let reversed = maybe_reverse_title(&replaced);
    let without_prefix = strip_leading_website_prefix(reversed);
    strip_trailing_website_suffix(without_prefix)
}

fn normalize_connector_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0usize;

    while index < tokens.len() {
        let current = tokens[index].as_str();
        let next = tokens.get(index + 1).map(|value| value.as_str());
        let third = tokens.get(index + 2).map(|value| value.as_str());

        if current == "A" && next == Some("K") && third == Some("A") {
            out.push("AKA".to_string());
            index += 3;
            continue;
        }

        out.push(tokens[index].clone());
        index += 1;
    }

    out
}

fn split_title(raw_title: &str) -> Vec<String> {
    let sanitized = sanitize_release_title(raw_title);
    let mut normalized = String::with_capacity(sanitized.len());

    for ch in sanitized.chars() {
        match ch {
            '[' | ']' | '(' | ')' | '{' | '}' | '_' => normalized.push(' '),
            _ => normalized.push(ch.to_ascii_uppercase()),
        }
    }

    let tokens = normalized
        .split_whitespace()
        .flat_map(split_release_token)
        .collect();

    normalize_connector_tokens(tokens)
}

fn parse_year(token: &str) -> Option<u32> {
    if token.len() != 4 || !is_digit_str(token) {
        return None;
    }

    let year = token.parse::<u32>().ok()?;
    (1900..=2100).contains(&year).then_some(year)
}

fn parse_quality(token: &str) -> Option<&'static str> {
    if token == "UHD" || token == "4K" {
        return Some("2160p");
    }
    if token == "8K" || token.contains("4320") {
        return Some("4320p");
    }

    if token.contains("2160") {
        Some("2160p")
    } else if token.contains("1440") {
        Some("1440p")
    } else if token.contains("1080") {
        Some("1080p")
    } else if token.contains("720") {
        Some("720p")
    } else if token.contains("576") {
        Some("576p")
    } else if token.contains("480") {
        Some("480p")
    } else {
        None
    }
}

struct SourceResult {
    source: &'static str,
    service: Option<&'static str>,
}

fn parse_source(token: &str, next: Option<&str>) -> Option<SourceResult> {
    let upper = token;

    // Streaming service tokens → WEB-DL with identified service
    let service = match upper {
        "AMZN" | "AMAZON" => Some("Amazon"),
        "NF" | "NETFLIX" => Some("Netflix"),
        "ATVP" | "APTV" => Some("Apple TV+"),
        "DSNP" | "DNSP" => Some("Disney+"),
        "HMAX" | "HBO" => Some("HBO Max"),
        "PMTP" | "PARAMOUNT" => Some("Paramount+"),
        "PCOK" | "PEACOCK" => Some("Peacock"),
        "HULU" => Some("Hulu"),
        "CR" => Some("Crunchyroll"),
        "FUNI" | "FUNIMATION" => Some("Funimation"),
        "HIDIVE" => Some("HIDIVE"),
        "STAN" => Some("Stan"),
        "IT" | "ITUNES" => Some("iTunes"),
        "BILI" => Some("Bilibili"),
        "HOTSTAR" => Some("Hotstar"),
        "BBC" | "BBCI" | "IPLAYER" => Some("BBC iPlayer"),
        "YOUTUBE" => Some("YouTube"),
        "ROKU" => Some("Roku"),
        "CRAV" => Some("Crave"),
        _ => None,
    };

    if let Some(svc) = service {
        return Some(SourceResult {
            source: "WEB-DL",
            service: Some(svc),
        });
    }

    if upper == "WEB" && next.is_some_and(|next| next == "DL") {
        return Some(SourceResult {
            source: "WEB-DL",
            service: None,
        });
    }

    if upper == "WEB" && next.is_some_and(|next| next == "RIP") {
        return Some(SourceResult {
            source: "WEBRip",
            service: None,
        });
    }

    if (upper == "BD" && next.is_some_and(|next| next == "ISO"))
        || (upper == "BR" && next.is_some_and(|next| next == "DISK"))
    {
        return Some(SourceResult {
            source: "BRDISK",
            service: None,
        });
    }

    match upper {
        "WEBRIP" | "WEB-RIP" | "WEBMUX" | "WEBCAP" => Some(SourceResult {
            source: "WEBRip",
            service: None,
        }),
        "WEB" | "WEBDL" | "WEB-DL" | "WEBHLS" | "WEBD" => Some(SourceResult {
            source: "WEB-DL",
            service: None,
        }),
        "BRDISK" | "BDISO" | "BD25" | "BD50" | "BD66" | "BD100" | "BDMV" => Some(SourceResult {
            source: "BRDISK",
            service: None,
        }),
        "BLURAY" if next == Some("RAY") => Some(SourceResult {
            source: "BluRay",
            service: None,
        }),
        "BLURAY" | "BLURAYRIP" | "BLU" | "BD" | "BDRIP" | "BRRIP" | "BR" | "UHD" => {
            Some(SourceResult {
                source: "BluRay",
                service: None,
            })
        }
        "HDTV" | "HDTVRIP" => Some(SourceResult {
            source: "HDTV",
            service: None,
        }),
        "RAWHD" => Some(SourceResult {
            source: "RAWHD",
            service: None,
        }),
        "CAM" | "CAMRIP" | "HDCAM" | "HQCAM" | "NEWCAM" => Some(SourceResult {
            source: "CAM",
            service: None,
        }),
        "TS" | "TSRIP" | "TELESYNC" | "TELESYNCH" | "HDTS" => Some(SourceResult {
            source: "TELESYNC",
            service: None,
        }),
        "TC" | "TELECINE" => Some(SourceResult {
            source: "TELECINE",
            service: None,
        }),
        "SCR" | "SCREENER" | "DVDSCR" | "DVDSCREENER" => Some(SourceResult {
            source: "DVDSCR",
            service: None,
        }),
        "WP" | "WORKPRINT" => Some(SourceResult {
            source: "WORKPRINT",
            service: None,
        }),
        "REGIONAL" => Some(SourceResult {
            source: "REGIONAL",
            service: None,
        }),
        "DVDRIP" | "DVD" | "DVD5" | "DVD9" | "DVDR" | "MDVDR" => Some(SourceResult {
            source: "DVD",
            service: None,
        }),
        _ if upper.len() == 2
            && upper.starts_with('R')
            && upper[1..].chars().all(|character| character.is_ascii_digit()) =>
        {
            Some(SourceResult {
                source: "REGIONAL",
                service: None,
            })
        }
        _ => None,
    }
}

fn is_hash_like(token: &str) -> bool {
    if !(6..=16).contains(&token.len()) {
        return false;
    }

    token.chars().all(|character| character.is_ascii_hexdigit())
}

fn squash_release_group_candidate(candidate: &str) -> String {
    candidate
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn canonical_release_group_candidate(candidate: &str) -> Option<&'static str> {
    match squash_release_group_candidate(candidate).as_str() {
        "QXR" => Some("QxR"),
        "TIGOLE" => Some("Tigole"),
        "YIFY" => Some("YIFY"),
        "YTS" => Some("YTS"),
        "YTSMX" => Some("YTS.MX"),
        "YTSLT" => Some("YTS.LT"),
        "YTSAG" => Some("YTS.AG"),
        "KRALIMARKO" => Some("KRaLiMaRKo"),
        "HQMUX" => Some("HQMUX"),
        "DATALASS" => Some("DataLass"),
        "BENTHEMEN" => Some("BEN THE MEN"),
        "EMLHDTEAM" => Some("Eml HDTeam"),
        "ZR" => Some("-ZR-"),
        _ => None,
    }
}

fn normalize_release_group_candidate(candidate: &str) -> Option<String> {
    let collapsed = candidate
        .split_whitespace()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let token = collapsed.trim_matches(&[' ', '-', '_', '.', '(', ')', '[', ']', '{', '}'] as &[_]);

    if token.is_empty() {
        return None;
    }

    if token.len() < 2 || token.len() > 40 {
        return None;
    }

    let upper = token.to_ascii_uppercase();

    if let Some(canonical) = canonical_release_group_candidate(token) {
        return Some(canonical.to_string());
    }

    if token.split_whitespace().count() > 3 {
        return None;
    }

    if parse_year(&upper).is_some()
        || parse_quality(&upper).is_some()
        || is_hash_like(&upper)
        || parse_language_hint(&upper).is_some()
    {
        return None;
    }

    if is_hex_token(&upper)
        || parse_source(&upper, None).is_some()
        || parse_video(&upper).0.is_some()
        || parse_audio(&upper, None).is_some()
        || parse_channels(&upper).is_some()
        || parse_episode_token(&upper).is_some()
    {
        return None;
    }

    if !token
        .chars()
        .any(|character| character.is_ascii_alphabetic() || character == '-')
    {
        return None;
    }

    Some(token.to_string())
}

fn extract_delimited_sections(raw_title: &str, open: char, close: char) -> Vec<String> {
    let mut sections = Vec::new();
    let mut start = None::<usize>;
    let mut depth = 0usize;

    for (index, character) in raw_title.char_indices() {
        if character == open {
            if depth == 0 {
                start = Some(index + open.len_utf8());
            }
            depth += 1;
            continue;
        }

        if character == close && depth > 0 {
            if depth == 1 {
                let section_start = start.take();
                if let Some(section_start) = section_start {
                    if index >= section_start {
                        sections.push(raw_title[section_start..index].to_string());
                    } else {
                        sections.push(String::new());
                    }
                }
            }
            depth -= 1;
        }
    }

    sections
}

fn extract_release_group_from_delimiters(raw_title: &str) -> Option<String> {
    let mut last_match = None::<String>;

    for (open, close) in [('[', ']'), ('(', ')'), ('{', '}')] {
        for candidate in extract_delimited_sections(raw_title, open, close) {
            if let Some(normalized) = normalize_release_group_candidate(&candidate) {
                last_match = Some(normalized);
            }
        }
    }

    last_match
}

fn extract_leading_release_group(raw_title: &str) -> Option<String> {
    let trimmed = raw_title.trim_start();

    for (open, close) in [('[', ']'), ('(', ')'), ('{', '}')] {
        if let Some(rest) = trimmed.strip_prefix(open)
            && let Some(close_index) = rest.find(close)
        {
            let candidate = &rest[..close_index];
            if let Some(normalized) = normalize_release_group_candidate(candidate) {
                return Some(normalized);
            }
        }
    }

    None
}

fn is_release_group_token(token: &str) -> bool {
    let normalized = match normalize_release_group_candidate(token) {
        Some(value) => value,
        None => return false,
    };
    let upper = normalized.to_ascii_uppercase();

    !matches!(
        upper.as_str(),
        "REPACK"
            | "PROPER"
            | "REMUX"
            | "BD25"
            | "BD50"
            | "BDMV"
            | "BDRIP"
            | "RIP"
            | "DV"
            | "HDR"
            | "HDR10"
            | "HDR10PLUS"
            | "HDR10P"
            | "HDRVIVID"
            | "X264"
            | "X265"
            | "H.264"
            | "H.265"
            | "HEVC"
            | "AV1"
            | "VP9"
            | "VP8"
            | "AAC"
            | "FLAC"
            | "OPUS"
            | "ATMOS"
            | "DD"
            | "DDP"
            | "AC3"
            | "DTS"
            | "EAC3"
            | "TRUEHD"
            | "PCM"
    )
}

fn extract_release_group_from_tokens(tokens: &[String]) -> Option<String> {
    for token in tokens.iter().rev() {
        if is_release_group_token(token) {
            return normalize_release_group_candidate(token);
        }

        let upper = token.to_ascii_uppercase();
        if parse_year(&upper).is_some()
            || parse_quality(&upper).is_some()
            || parse_source(&upper, None).is_some()
            || parse_video(&upper).0.is_some()
            || parse_audio(&upper, None).is_some()
            || parse_channels(&upper).is_some()
            || parse_language_hint(&upper).is_some()
            || parse_episode_token(&upper).is_some()
            || is_release_suffix_token(&upper)
            || is_repost_suffix_token(&upper)
            || is_hash_like(&upper)
        {
            return None;
        }
    }

    None
}

fn is_release_suffix_token(token: &str) -> bool {
    matches!(
        token,
        "REPACK" | "PROPER" | "RERIP" | "READNFO" | "REAL" | "INTERNAL" | "LIMITED" | "EXTENDED"
    )
}

fn is_repost_suffix_token(token: &str) -> bool {
    if matches!(
        token,
        "RP"
            | "NZBGEEK"
            | "OBFUSCATED"
            | "SCRAMBLED"
            | "SAMPLE"
            | "PRE"
            | "POSTBOT"
            | "XPOST"
            | "WHITEREV"
            | "BUYMORE"
            | "ASREQUESTED"
            | "ALTERNATIVETOREQUESTED"
            | "GEROV"
            | "Z0IDS3N"
            | "CHAMELE0N"
            | "4P"
            | "4PLANET"
            | "ALTEZACHEN"
            | "REPACKPOST"
    ) {
        return true;
    }

    token == "1"
        || token.starts_with("RAKUV")
        || token.starts_with("POST")
        || token.ends_with("FINHEL")
}

fn strip_repost_suffixes(candidate: &str) -> String {
    let mut parts = candidate
        .split('-')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    while let Some(last) = parts.last() {
        let upper = last.to_ascii_uppercase();
        if is_repost_suffix_token(upper.as_str()) {
            parts.pop();
            continue;
        }
        break;
    }

    parts.join("-")
}

fn extract_release_group_from_raw_suffix(raw_title: &str) -> Option<String> {
    let trimmed = raw_title.trim();
    if trimmed.is_empty() {
        return None;
    }

    for (index, _) in trimmed.rmatch_indices('-') {
        let tail = trimmed.get(index + 1..)?.trim();
        if tail.is_empty() {
            continue;
        }

        // Skip dashes that are part of known compound source/audio tokens
        // (e.g. "WEB-DL", "DTS-HD", "E-AC-3", "DUAL-AUDIO").
        let before_dash = trimmed.get(..index).unwrap_or_default();
        let prefix = before_dash
            .rsplit(['.', ' ', '-', '_', '[', '('])
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let compound = format!("{prefix}-{}", tail.split(['.', ' ', '-', '_', '[', '('])
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase());
        if should_preserve_hyphen_token(&compound) {
            continue;
        }

        let mut end = tail.len();

        if let Some(bracket_index) = tail.find('[') {
            end = end.min(bracket_index);
        }

        if let Some(dot_index) = tail.find('.') {
            let suffix = tail
                .get(dot_index + 1..)
                .and_then(|value| value.split(['.', '[']).next())
                .unwrap_or_default()
                .trim()
                .to_ascii_uppercase();
            if is_release_suffix_token(suffix.as_str()) {
                end = end.min(dot_index);
            }
        }

        let candidate = tail.get(..end).unwrap_or_default().trim();
        let candidate = strip_repost_suffixes(candidate);
        if let Some(group) = normalize_release_group_candidate(&candidate) {
            return Some(group);
        }
    }

    None
}

fn extract_release_group(raw_title: &str, tokens: &[String]) -> Option<String> {
    if let Some(group) = extract_leading_release_group(raw_title) {
        return Some(group);
    }

    if let Some(group) = extract_release_group_from_delimiters(raw_title) {
        return Some(group);
    }

    if let Some(group) = extract_release_group_from_raw_suffix(raw_title) {
        return Some(group);
    }

    let has_release_signal = tokens.iter().any(|token| {
        parse_year(token).is_some()
            || parse_quality(token).is_some()
            || parse_source(token, None).is_some()
            || parse_video(token).0.is_some()
            || parse_audio(token, None).is_some()
            || token == "PROPER"
            || token == "REPACK"
            || token == "REMUX"
            || token == "BD25"
            || token == "BD50"
            || token == "BDMV"
            || token == "BDRIP"
    });

    if !has_release_signal {
        return None;
    }

    extract_release_group_from_tokens(tokens)
}

fn parse_video(token: &str) -> (Option<String>, Option<String>) {
    let video_encoding = if token.contains("X264") {
        Some("x264".to_string())
    } else if token.contains("X265") {
        Some("x265".to_string())
    } else {
        None
    };

    let codec = if token.contains("H.265")
        || token == "H265"
        || token.contains("HEVC")
        || token == "X265"
        || token == "X.265"
    {
        Some("H.265")
    } else if token.contains("H.264")
        || token == "H264"
        || token == "AVC"
        || token == "X264"
        || token == "X.264"
    {
        Some("H.264")
    } else if token == "AV1" {
        Some("AV1")
    } else if token == "VP9" {
        Some("VP9")
    } else if token == "XVID" {
        Some("XVID")
    } else {
        None
    };

    (codec.map(str::to_string), video_encoding)
}

fn parse_channels(value: &str) -> Option<String> {
    let upper = value.to_ascii_uppercase();
    if (upper.ends_with("CH") || upper.ends_with("CHS"))
        && !upper.ends_with("ARCH")
        && !upper.ends_with("CHIP")
    {
        let trimmed = upper.trim_end_matches("CHS").trim_end_matches("CH");
        if is_digit_str(trimmed) && !trimmed.is_empty() {
            return Some(format!("{trimmed}.0"));
        }
    }

    if upper.ends_with("CHANNELS") {
        let trimmed = upper.trim_end_matches("CHANNELS");
        if is_digit_str(trimmed) && !trimmed.is_empty() {
            return Some(format!("{trimmed}.0"));
        }
    }

    fn parse_channel_pair(value: &str) -> Option<String> {
        let parts: Vec<&str> = value.split('.').collect();
        if parts.len() == 2
            && parts[0].len() <= 2
            && parts[1].len() <= 2
            && parts[0].bytes().all(|b| b.is_ascii_digit())
            && parts[1].bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("{}.{}", parts[0], parts[1]));
        }

        if parts.len() == 3
            && parts[0].len() <= 2
            && parts[1].len() <= 2
            && parts[2].len() <= 2
            && parts[0].bytes().all(|b| b.is_ascii_digit())
            && parts[1].bytes().all(|b| b.is_ascii_digit())
            && parts[2].bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("{}.{}", parts[0], parts[1]));
        }

        None
    }

    if upper.is_empty() {
        return None;
    }

    let value = upper.as_str();
    let bytes = value.as_bytes();
    let mut idx = 0usize;

    while idx < bytes.len() {
        if !bytes[idx].is_ascii_digit() {
            idx += 1;
            continue;
        }

        let start = idx;
        while idx < bytes.len() {
            let ch = bytes[idx];
            if ch.is_ascii_digit() || ch == b'.' {
                idx += 1;
                continue;
            }
            break;
        }

        let segment = &value[start..idx];
        if let Some(channels) = parse_channel_pair(segment) {
            return Some(channels);
        }

        idx += 1;
    }

    None
}

fn is_preserved_dotted_token(token: &str) -> bool {
    if matches!(
        token,
        "H.264" | "H.265" | "X.264" | "X.265" | "H264" | "X265"
    ) {
        return true;
    }

    if token.starts_with("AC-") || token.starts_with("DD+") || token.starts_with("DDP") {
        return true;
    }

    token
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && token.contains('.')
        && token.split('.').count() >= 2
}

fn looks_like_episode_hyphen_token(token: &str) -> bool {
    if token.starts_with('S')
        && token
            .chars()
            .nth(1)
            .is_some_and(|character| character.is_ascii_digit())
    {
        return true;
    }

    if let Some((left, right)) = token.split_once('X') {
        return !left.is_empty()
            && left.chars().all(|character| character.is_ascii_digit())
            && right.chars().any(|character| character.is_ascii_digit());
    }

    // Preserve bare digit ranges like "1122-1133", "0001-0782", "01-07"
    // These are episode ranges common in fansub/AnimeTosho titles.
    if let Some((left, right)) = token.split_once('-')
        && !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|c| c.is_ascii_digit())
        && right.chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }

    // Preserve "E795-E940" style ranges
    if token.starts_with('E')
        && token.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
        && token.contains('-')
    {
        return true;
    }

    false
}

fn should_preserve_hyphen_token(token: &str) -> bool {
    matches!(token, "WEB-DL" | "DUAL-AUDIO")
        || token.starts_with("AC-3")
        || token.starts_with("E-AC-3")
        || token.starts_with("DTS-")
        || looks_like_episode_hyphen_token(token)
}

fn split_hyphenated_token(token: &str) -> Vec<String> {
    if !token.contains('-') || should_preserve_hyphen_token(token) {
        return vec![token.to_string()];
    }

    if let Some(season_marker_index) = token.find("-S") {
        let season_start = season_marker_index + 1;
        if token
            .chars()
            .nth(season_start + 1)
            .is_some_and(|character| character.is_ascii_digit())
        {
            let mut out = token
                .get(..season_marker_index)
                .unwrap_or_default()
                .split('-')
                .filter(|value| !value.is_empty())
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>();
            // Extract just the S\d+E\d+ portion; any trailing text after
            // a dash (e.g. episode title like "-TRUST" in "S02E21-TRUST")
            // becomes separate tokens.
            let season_tail = &token[season_start..];
            if let Some(ep_dash) = season_tail[1..].find('-') {
                let ep_end = ep_dash + 1;
                let ep_token = &season_tail[..ep_end];
                // Only split if the part after the dash is NOT a continuation
                // of the episode pattern (e.g. "S02E21-E22" should stay together)
                let after = &season_tail[ep_end + 1..];
                if after.starts_with('E') || after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    out.push(season_tail.to_string());
                } else {
                    out.push(ep_token.to_string());
                    out.extend(
                        after
                            .split('-')
                            .filter(|v| !v.is_empty())
                            .map(str::to_string),
                    );
                }
            } else {
                out.push(season_tail.to_string());
            }
            return out;
        }
    }

    token
        .split('-')
        .filter(|value| !value.is_empty())
        .map(std::string::ToString::to_string)
        .collect()
}

fn split_release_token(token: &str) -> Vec<String> {
    if token.is_empty() {
        return Vec::new();
    }

    let token = token.trim_matches('.');
    if token.is_empty() {
        return Vec::new();
    }

    if !token.contains('.') {
        return split_hyphenated_token(token);
    }

    if is_preserved_dotted_token(token) {
        return vec![token.to_string()];
    }

    let mut raw_parts = Vec::new();
    for part in token.split('.') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        raw_parts.push(part.to_string());
    }

    let mut parts = Vec::new();
    let mut index = 0usize;
    while index < raw_parts.len() {
        let current = raw_parts[index].as_str();
        let next = raw_parts.get(index + 1).map(|value| value.as_str());
        let third = raw_parts.get(index + 2).map(|value| value.as_str());

        if let Some(next_value) = next {
            if (current == "H" || current == "X") && (next_value == "264" || next_value == "265") {
                parts.push(format!("{current}.{next_value}"));
                index += 2;
                continue;
            }

            if (current == "H" || current == "X")
                && (next_value.starts_with("264-") || next_value.starts_with("265-"))
            {
                let mut pieces = next_value.splitn(2, '-');
                let numeric = pieces.next().unwrap_or_default();
                let tail = pieces.next().unwrap_or_default();
                if !numeric.is_empty() {
                    parts.push(format!("{current}.{numeric}"));
                }
                if !tail.is_empty() {
                    parts.push(tail.to_string());
                }
                index += 2;
                continue;
            }

            let is_audio_codec = current.starts_with("DDP")
                || current.starts_with("DD+")
                || current.starts_with("DD")
                || current.starts_with("AAC")
                || current.starts_with("AC-3")
                || current == "AC3";

            if is_audio_codec && is_digit_str(next_value) {
                // Try three-token merge for channel specs like AAC.2.0 or AC3.5.1
                if let Some(third_value) = third {
                    if is_digit_str(third_value) {
                        parts.push(format!("{current}.{next_value}.{third_value}"));
                        index += 3;
                        continue;
                    }
                    // Handle "digit-GROUP" suffix: AAC.2.0-DBTV → "AAC.2.0" + "DBTV"
                    if let Some(hyphen_idx) = third_value.find('-') {
                        let digit_part = &third_value[..hyphen_idx];
                        let tail = &third_value[hyphen_idx + 1..];
                        if !digit_part.is_empty() && is_digit_str(digit_part) {
                            parts.push(format!("{current}.{next_value}.{digit_part}"));
                            if !tail.is_empty() {
                                parts.push(tail.to_string());
                            }
                            index += 3;
                            continue;
                        }
                    }
                }
                parts.push(format!("{current}.{next_value}"));
                index += 2;
                continue;
            }

            // Handle "digit-GROUP" in next token: DDP5.1-GROUP → "DDP5.1" + "GROUP"
            if is_audio_codec && let Some(hyphen_idx) = next_value.find('-') {
                let digit_part = &next_value[..hyphen_idx];
                let tail = &next_value[hyphen_idx + 1..];
                if !digit_part.is_empty() && is_digit_str(digit_part) {
                    parts.push(format!("{current}.{digit_part}"));
                    if !tail.is_empty() {
                        parts.push(tail.to_string());
                    }
                    index += 2;
                    continue;
                }
            }
        }

        parts.extend(split_hyphenated_token(current));
        index += 1;
    }

    if parts.is_empty() {
        vec![token.to_string()]
    } else {
        parts
    }
}

#[derive(Clone, Copy)]
enum LanguageScope {
    Auto,
    Audio,
    Subtitle,
}

fn token_has_prefix(token: &str, candidates: &[&str]) -> bool {
    candidates.contains(&token)
}

fn has_language_context_token(token: &str) -> Option<LanguageScope> {
    if token.starts_with("SUB") || token.starts_with("VOST") || token.contains("SUBS") {
        return Some(LanguageScope::Subtitle);
    }

    if token_has_prefix(token, LANG_AUDIO_MARKERS) {
        Some(LanguageScope::Audio)
    } else if token_has_prefix(token, LANG_SUBTITLE_MARKERS) || token.starts_with("MULTI") {
        Some(LanguageScope::Subtitle)
    } else {
        None
    }
}

fn parse_audio(raw_token: &str, next: Option<&str>) -> Option<ParsedAudio> {
    let token = raw_token.trim().trim_start_matches('+');
    if token.is_empty() {
        return None;
    }

    // Dolby Digital Plus (DDP / DD+ / EAC3) — must check before plain DD
    if token.starts_with("DDP") || token.starts_with("DD+") {
        let suffix = token.trim_start_matches("DDP").trim_start_matches("DD+");
        let channels = parse_channels(suffix).or_else(|| next.and_then(parse_channels));
        return Some(ParsedAudio {
            codec: "DDP",
            channels,
        });
    }

    if token.starts_with("AC3") || token.starts_with("AC-3") {
        let suffix = token.trim_start_matches("AC-3").trim_start_matches("AC3");
        return Some(ParsedAudio {
            codec: "AC3",
            channels: parse_channels(suffix)
                .or_else(|| parse_channels(token))
                .or_else(|| next.and_then(parse_channels)),
        });
    }

    // Dolby Digital (DD) — plain DD without + or P suffix.
    // Bare "DD" with no channel info is too ambiguous (common in titles like "DD Returns"),
    // so require either suffix digits or channel info from the next token.
    if token.starts_with("DD") {
        let suffix = token.trim_start_matches("DD");
        let channels = parse_channels(suffix).or_else(|| next.and_then(parse_channels));
        if !suffix.is_empty() || channels.is_some() {
            return Some(ParsedAudio {
                codec: "DD",
                channels,
            });
        }
    }

    if token.starts_with("EAC3") || token.starts_with("E-AC-3") || token.starts_with("EAC") {
        return Some(ParsedAudio {
            codec: "EAC3",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DTS-X") || token.starts_with("DTSX") {
        return Some(ParsedAudio {
            codec: "DTSX",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DTS-MA") || token.starts_with("DTSMA") {
        return Some(ParsedAudio {
            codec: "DTSMA",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DTS-HD") || token.starts_with("DTSHD") {
        return Some(ParsedAudio {
            codec: "DTSHD",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DTS") {
        return Some(ParsedAudio {
            codec: "DTS",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("TRUEHD") {
        return Some(ParsedAudio {
            codec: "TRUEHD",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("FLAC") {
        return Some(ParsedAudio {
            codec: "FLAC",
            channels: None,
        });
    }

    if token.starts_with("OPUS") {
        return Some(ParsedAudio {
            codec: "OPUS",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("AAC") {
        return Some(ParsedAudio {
            codec: "AAC",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("MP3") {
        return Some(ParsedAudio {
            codec: "MP3",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token == "LPCM" || token.starts_with("PCM") {
        return Some(ParsedAudio {
            codec: "PCM",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    None
}

fn parse_fps(raw_title: &str) -> Option<f32> {
    let upper = raw_title.to_ascii_uppercase();
    for chunk in upper.split(|c: char| {
        c.is_ascii_whitespace()
            || c == '['
            || c == ']'
            || c == '('
            || c == ')'
            || c == '{'
            || c == '}'
    }) {
        let chunk = chunk.trim_matches(&['.', '-', '_', ' '] as &[_]);
        if chunk.is_empty() {
            continue;
        }

        let chunk = chunk.trim_end_matches("FPS");
        if let Ok(fps) = chunk.parse::<f32>()
            && (10.0..=300.0).contains(&fps)
        {
            return Some(fps);
        }
    }

    let compact = upper.replace(['[', ']', '(', ')', '{', '}'], " ");
    let parts: Vec<_> = compact.split_whitespace().collect();
    for chunk in parts {
        if let Some(prefix) = chunk.strip_suffix("FPS") {
            let prefix = prefix.trim();
            if let Ok(fps) = prefix.parse::<f32>()
                && (10.0..=300.0).contains(&fps)
            {
                return Some(fps);
            }
        }
    }

    // Dot-separated titles: split on dots and hyphens to find "60FPS" or "60fps" tokens
    for chunk in upper.split(['.', '-', '_']) {
        if let Some(prefix) = chunk.strip_suffix("FPS")
            && let Ok(fps) = prefix.parse::<f32>()
            && (10.0..=300.0).contains(&fps)
        {
            return Some(fps);
        }
    }

    None
}

fn title_case_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    let mut chars = lower.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    out.extend(chars);
    out
}

fn parse_edition_at(tokens: &[String], index: usize) -> Option<(String, usize)> {
    let token = tokens.get(index)?.as_str();
    let next = tokens.get(index + 1).map(|value| value.as_str());
    let third = tokens.get(index + 2).map(|value| value.as_str());
    let fourth = tokens.get(index + 3).map(|value| value.as_str());

    match token {
        "IMAX" if next == Some("ENHANCED") => Some(("IMAX Enhanced".to_string(), 2)),
        "IMAX" => Some(("IMAX".to_string(), 1)),
        "EXTENDED" => {
            if next == Some("THEATRICAL") && third == Some("VERSION") && fourth == Some("IMAX") {
                Some(("Extended Theatrical Version IMAX".to_string(), 4))
            } else if next == Some("CUT") {
                Some(("Extended Cut".to_string(), 2))
            } else {
                Some(("Extended".to_string(), 1))
            }
        }
        "UNRATED" => Some(("Unrated".to_string(), 1)),
        "THEATRICAL" => Some(("Theatrical".to_string(), 1)),
        "CRITERION" => Some(("Criterion".to_string(), 1)),
        "REMASTERED" | "REMASTER" => Some(("Remaster".to_string(), 1)),
        "HYBRID" => Some(("Hybrid".to_string(), 1)),
        "RESTORED" => Some(("Restored".to_string(), 1)),
        "DESPECIALIZED" => Some(("Despecialized".to_string(), 1)),
        "OPEN" if next == Some("MATTE") => Some(("Open Matte".to_string(), 2)),
        "FAN" if next == Some("EDIT") => Some(("Fan Edit".to_string(), 2)),
        "FINAL" if next == Some("CUT") => Some(("Final Cut".to_string(), 2)),
        "ASSEMBLY" if next == Some("CUT") => Some(("Assembly Cut".to_string(), 2)),
        "DIRECTORS" | "DIRECTOR" if next == Some("CUT") => {
            Some(("Director's Cut".to_string(), 2))
        }
        "SPECIAL" if next == Some("EDITION") && third == Some("REMASTERED") => {
            Some(("Special Edition Remastered".to_string(), 3))
        }
        "SPECIAL" if next == Some("EDITION") && third == Some("FAN") && fourth == Some("EDIT") => {
            Some(("Special Edition Fan Edit".to_string(), 4))
        }
        "SPECIAL" if next == Some("EDITION") => Some(("Special Edition".to_string(), 2)),
        "2IN1" | "3IN1" | "4IN1" => Some((token.to_string(), 1)),
        "ULTIMATE" if matches!(next, Some("HUNTER") | Some("REKALL")) && third == Some("EDITION") => {
            Some((format!("Ultimate {} Edition", title_case_token(next.unwrap_or_default())), 3))
        }
        "DIAMOND" | "SIGNATURE" | "IMPERIAL" | "HUNTER" | "REKALL"
            if next == Some("EDITION") =>
        {
            Some((format!("{} Edition", title_case_token(token)), 2))
        }
        "THE" if next == Some("IMPERIAL") && third == Some("EDITION") => {
            Some(("Imperial Edition".to_string(), 3))
        }
        value if value.ends_with("TH")
            && value[..value.len() - 2]
                .chars()
                .all(|character| character.is_ascii_digit())
            && next == Some("ANNIVERSARY") =>
        {
            let label = if third == Some("EDITION") {
                format!("{value} Anniversary Edition")
            } else {
                format!("{value} Anniversary")
            };
            Some((label, if third == Some("EDITION") { 3 } else { 2 }))
        }
        _ => None,
    }
}

fn is_noise_token(token: &str) -> bool {
    if token.len() <= 1 {
        return token != "/";
    }

    if is_hex_token(token) || parse_year(token).is_some() || parse_quality(token).is_some() {
        return true;
    }

    if token == "IMDB"
        || token == "TMDB"
        || token == "TMDBID"
        || token
            .strip_prefix("TT")
            .is_some_and(|value| value.chars().all(|character| character.is_ascii_digit()))
    {
        return true;
    }

    if (token.contains('.') || token.contains('-') || token.bytes().any(|byte| byte.is_ascii_digit()))
        && (parse_source(token, None).is_some()
            || parse_video(token).0.is_some()
            || parse_audio(token, None).is_some()
            || parse_channels(token).is_some()
            || parse_episode_token(token).is_some())
    {
        return true;
    }

    matches!(
        token,
        "MULTI"
            | "DUAL"
            | "DD"
            | "DDP"
            | "AC3"
            | "EAC3"
            | "TRUEHD"
            | "ATMOS"
            | "DTS"
            | "H264"
            | "H265"
            | "X264"
            | "X265"
            | "H.264"
            | "H.265"
            | "HEVC"
            | "AV1"
            | "VP9"
            | "VP8"
            | "AAC"
            | "FLAC"
            | "OPUS"
            | "WEB"
            | "DL"
            | "RIP"
            | "DV"
            | "HDR"
            | "HDR10"
            | "HDR10PLUS"
            | "HDR10P"
            | "HDRVIVID"
            | "SEASON"
            | "SAISON"
            | "STAGIONE"
            | "TEMPORADA"
            | "EPISODE"
            | "EXTRAS"
            | "SUBPACK"
            | "COMPLETE"
            | "BATCH"
            | "PACK"
            | "PART"
            | "VOL"
            | "VOLUME"
            | "OVA"
            | "OVD"
            | "NCOP"
            | "NCED"
            | "SPECIAL"
            | "GROUP"
            | "REMUX"
            | "AMZN"
            | "NF"
            | "BILI"
            | "ATVP"
            | "HULU"
            | "BD"
            | "BLURAY"
            | "BLURAYRIP"
            | "BDRIP"
            | "BRRIP"
            | "BR"
            | "BD25"
            | "BD50"
            | "BDMV"
            | "BDISO"
            | "BRDISK"
            | "PROPER"
            | "REPACK"
            | "EXTENDED"
            | "LIMITED"
            | "HDRIP"
            | "HDTV"
            | "CR"
            | "WEBDL"
            | "WEBRIP"
            | "WEBMUX"
            | "HDCAM"
            | "CAM"
            | "TELESYNC"
            | "TS"
            | "TELECINE"
            | "TC"
            | "DVDSCR"
            | "SCREENER"
            | "WORKPRINT"
            | "REGIONAL"
            | "RAWHD"
            | "AI"
            | "ENHANCED"
            | "AIENHANCED"
            | "RIFE"
            | "HFR"
            | "PCM"
            | "EDITION"
            | "VERSION"
            | "FINAL"
            | "ASSEMBLY"
            | "MATTE"
            | "ANNIVERSARY"
            | "RESTORED"
            | "DESPECIALIZED"
            | "KORSUB"
            | "KORSUBS"
    ) || is_language_token(token)
}

fn is_title_connector_token(token: &str) -> bool {
    token == "AKA" || token == "/"
}

fn collect_normalized_title_tokens(
    tokens: &[String],
    episode: &Option<ParsedEpisodeMetadata>,
    release_group: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let episode_raw_tokens = episode
        .as_ref()
        .and_then(|ep| ep.raw.as_ref())
        .map(|raw| split_title(raw))
        .unwrap_or_default();
    let release_group_tokens = release_group
        .map(split_title)
        .unwrap_or_default();

    for token in tokens {
        if is_noise_token(token) {
            continue;
        }

        if episode_raw_tokens.iter().any(|raw| raw == token) {
            continue;
        }

        if release_group_tokens.iter().any(|group| group == token) {
            continue;
        }

        if is_title_connector_token(token)
            || token.chars().any(|character| character.is_alphabetic())
            || token.chars().all(|character| character.is_ascii_digit())
        {
            out.push(token.to_string());
        }
    }

    out
}

fn normalize_title_tokens(
    tokens: &[String],
    episode: &Option<ParsedEpisodeMetadata>,
    release_group: Option<&str>,
) -> String {
    collect_normalized_title_tokens(tokens, episode, release_group)
        .into_iter()
        .filter(|token| token != "/")
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_normalized_title_variants(title_tokens: &[String], normalized_title: &str) -> Vec<String> {
    let mut variants = Vec::new();

    if !normalized_title.is_empty() {
        variants.push(normalized_title.to_string());
    }

    for connector in ["AKA", "/"] {
        if !title_tokens.iter().any(|token| token == connector) {
            continue;
        }

        let mut current = Vec::new();
        let mut segments = Vec::<Vec<String>>::new();

        for token in title_tokens {
            if token == connector {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                continue;
            }

            current.push(token.clone());
        }

        if !current.is_empty() {
            segments.push(current);
        }

        for segment in segments {
            let normalized = segment.join(" ");
            if !normalized.is_empty() {
                variants.push(normalized);
            }
        }
    }

    dedupe_keep_order(variants)
}

fn parse_imdb_id_from_tokens(tokens: &[String]) -> Option<String> {
    for (index, token) in tokens.iter().enumerate() {
        if let Some(rest) = token.strip_prefix("TT")
            && (6..=12).contains(&rest.len())
            && rest.chars().all(|character| character.is_ascii_digit())
        {
            return Some(format!("tt{rest}"));
        }

        if token == "IMDB"
            && let Some(next) = tokens.get(index + 1)
            && let Some(rest) = next.strip_prefix("TT")
            && (6..=12).contains(&rest.len())
            && rest.chars().all(|character| character.is_ascii_digit())
        {
            return Some(format!("tt{rest}"));
        }
    }

    None
}

fn parse_tmdb_id_from_tokens(tokens: &[String]) -> Option<u32> {
    for (index, token) in tokens.iter().enumerate() {
        if let Some(rest) = token.strip_prefix("TMDBID") {
            let digits = rest.trim_start_matches(['-', '_']);
            if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
                return digits.parse::<u32>().ok();
            }
        }

        if let Some(rest) = token.strip_prefix("TMDB") {
            let digits = rest.trim_start_matches(['-', '_']);
            if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
                return digits.parse::<u32>().ok();
            }
        }

        if matches!(token.as_str(), "TMDB" | "TMDBID")
            && let Some(next) = tokens.get(index + 1)
            && next.chars().all(|character| character.is_ascii_digit())
        {
            return next.parse::<u32>().ok();
        }
    }

    None
}

/// Returns (season, episodes, anime_version).
fn parse_episode_token(token: &str) -> Option<(Option<u32>, Vec<u32>, Option<u32>)> {
    if token.len() < 3 {
        return None;
    }

    if token.starts_with('S') {
        let (_, tail) = token.split_at(1);
        let (season, rest) = parse_leading_digits(tail)?;
        let rest = rest.trim_start_matches(['-', '.', '_', ':']);
        if rest.is_empty() {
            return None;
        }

        let episode_fragment = if let Some(stripped) = rest.strip_prefix("EP") {
            stripped
        } else if let Some(stripped) = rest.strip_prefix('E') {
            stripped
        } else {
            rest
        };

        let episodes = parse_episode_fragment(episode_fragment);
        if !episodes.is_empty() {
            let version = extract_trailing_version(episode_fragment);
            return Some((Some(season), episodes, version));
        }
    }

    if let Some((left, right)) = token.split_once('X')
        && let Ok(season) = left.parse::<u32>()
    {
        let episodes = parse_episode_fragment(right);
        if !episodes.is_empty() {
            let version = extract_trailing_version(right);
            return Some((Some(season), episodes, version));
        }
    }

    None
}

/// Extract a trailing anime version suffix (V2-V9) from a fragment like "01V2" or "01-03V2".
fn extract_trailing_version(fragment: &str) -> Option<u32> {
    let bytes = fragment.as_bytes();
    let len = bytes.len();
    if len >= 2 && bytes[len - 2] == b'V' {
        let ver = bytes[len - 1] - b'0';
        if (2..=9).contains(&ver) {
            return Some(ver as u32);
        }
    }
    None
}

fn parse_short_month_token(token: &str) -> Option<u32> {
    match token {
        "JAN" => Some(1),
        "FEB" => Some(2),
        "MAR" => Some(3),
        "APR" => Some(4),
        "MAY" => Some(5),
        "JUN" => Some(6),
        "JUL" => Some(7),
        "AUG" => Some(8),
        "SEP" => Some(9),
        "OCT" => Some(10),
        "NOV" => Some(11),
        "DEC" => Some(12),
        _ => None,
    }
}

fn parse_day_token(token: &str) -> Option<u32> {
    let trimmed = token
        .trim_end_matches("ST")
        .trim_end_matches("ND")
        .trim_end_matches("RD")
        .trim_end_matches("TH");
    let day = trimmed.parse::<u32>().ok()?;
    (1..=31).contains(&day).then_some(day)
}

fn build_air_date(year: u32, month: u32, day: u32) -> Option<NaiveDate> {
    let date = NaiveDate::from_ymd_opt(year as i32, month, day)?;
    let today = Utc::now().date_naive();
    let min_date = NaiveDate::from_ymd_opt(1951, 1, 1)?;
    let max_date = today.succ_opt().unwrap_or(today);
    (date >= min_date && date <= max_date).then_some(date)
}

fn parse_year_first_air_date(left: &str, middle: &str, right: &str) -> Option<NaiveDate> {
    let year = parse_year(left)?;
    let second = middle.parse::<u32>().ok()?;
    let third = right.parse::<u32>().ok()?;

    if second > 12 {
        build_air_date(year, third, second)
    } else {
        build_air_date(year, second, third)
    }
}

fn parse_year_last_air_date(left: &str, middle: &str, right: &str) -> Option<NaiveDate> {
    let year = parse_year(right)?;
    let first = left.parse::<u32>().ok()?;
    let second = middle.parse::<u32>().ok()?;

    if first > 12 && second <= 12 {
        build_air_date(year, second, first)
    } else if second > 12 && first <= 12 {
        build_air_date(year, first, second)
    } else {
        None
    }
}

fn parse_short_month_air_date(first: &str, second: &str, third: &str) -> Option<NaiveDate> {
    let day = parse_day_token(first)?;
    let month = parse_short_month_token(second)?;
    let year = parse_year(third)?;
    build_air_date(year, month, day)
}

fn parse_six_digit_air_date(token: &str) -> Option<NaiveDate> {
    if token.len() != 6 || !is_digit_str(token) {
        return None;
    }

    let year_short = token[..2].parse::<u32>().ok()?;
    let month = token[2..4].parse::<u32>().ok()?;
    let day = token[4..6].parse::<u32>().ok()?;
    let current_year_short = (Utc::now().year() % 100) as u32;
    let year = if year_short <= current_year_short + 1 {
        2000 + year_short
    } else {
        1900 + year_short
    };

    build_air_date(year, month, day)
}

fn parse_daily_part(tokens: &[String]) -> Option<u32> {
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token.as_str(), "PART" | "PT")
            && let Some(next) = tokens.get(index + 1)
            && let Some(part) = parse_numeric_token(next)
        {
            return Some(part);
        }

        for prefix in ["PART", "PT"] {
            if let Some(rest) = token.strip_prefix(prefix)
                && let Some(part) = parse_numeric_token(rest)
            {
                return Some(part);
            }
        }
    }

    None
}

fn parse_daily_episode(tokens: &[String]) -> Option<ParsedEpisodeMetadata> {
    for index in 0..tokens.len() {
        if let Some(token) = tokens.get(index)
            && let Some(date) = parse_six_digit_air_date(token)
        {
            return Some(new_daily_episode_metadata(
                date,
                parse_daily_part(tokens),
                Some(date.format("%Y-%m-%d").to_string()),
            ));
        }

        if index + 2 >= tokens.len() {
            continue;
        }

        let first = tokens[index].as_str();
        let second = tokens[index + 1].as_str();
        let third = tokens[index + 2].as_str();

        if let Some(date) = parse_year_first_air_date(first, second, third)
            .or_else(|| parse_year_last_air_date(first, second, third))
            .or_else(|| parse_short_month_air_date(first, second, third))
        {
            return Some(new_daily_episode_metadata(
                date,
                parse_daily_part(tokens),
                Some(date.format("%Y-%m-%d").to_string()),
            ));
        }
    }

    None
}

fn parse_special_kind_token(token: &str) -> Option<ParsedSpecialKind> {
    match token {
        "SPECIAL" => Some(ParsedSpecialKind::Special),
        "OVA" => Some(ParsedSpecialKind::OVA),
        "OVD" => Some(ParsedSpecialKind::OVD),
        "NCOP" => Some(ParsedSpecialKind::NCOP),
        "NCED" => Some(ParsedSpecialKind::NCED),
        _ => None,
    }
}

fn parse_localized_season_token(token: &str) -> Option<u32> {
    parse_named_season_token(token).or_else(|| {
        for prefix in ["SAISON", "STAGIONE", "TEMPORADA"] {
            if let Some(rest) = token.strip_prefix(prefix) {
                let rest = rest.trim_start_matches(['-', '.', '_', ':']);
                if let Some((season, tail)) = parse_leading_digits(rest)
                    && tail.trim_matches(['-', '.', '_', ':']).is_empty()
                {
                    return Some(season);
                }
            }
        }
        None
    })
}

fn parse_season_designator(token: &str) -> Option<u32> {
    parse_series_only_season(token).or_else(|| parse_localized_season_token(token))
}

fn parse_season_range_token(token: &str) -> Option<(u32, u32, String)> {
    if token.contains('E') || !token.contains('-') {
        return None;
    }

    let (left, right) = token.split_once('-')?;
    let first = parse_season_designator(left)?;
    if left.starts_with('S')
        && parse_season_designator(right).is_none()
        && left[1..].len() == 1
        && right.len() == 2
    {
        return None;
    }
    let last = parse_season_designator(right).or_else(|| parse_numeric_token(right))?;
    (last >= first).then(|| (first, last, token.to_string()))
}

fn parse_pack_part_token(token: &str) -> Option<(u32, String)> {
    for prefix in ["PART", "VOL", "VOLUME", "P", "PT"] {
        if let Some(rest) = token.strip_prefix(prefix)
            && let Some(part) = parse_numeric_token(rest)
        {
            return Some((part, token.to_string()));
        }
    }

    None
}

fn tokens_have_explicit_episode_pattern(tokens: &[String]) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        if parse_season_range_token(token).is_some() {
            continue;
        }

        if parse_episode_token(token).is_some() || !parse_named_episode_token(token).is_empty() {
            return true;
        }

        if parse_named_episode_anchor_token(token)
            && let Some(next) = tokens.get(index + 1)
            && (!parse_named_episode_token(next).is_empty() || !parse_pending_episode_token(next).is_empty())
        {
            return true;
        }
    }

    false
}

fn parse_season_pack(tokens: &[String]) -> Option<ParsedEpisodeMetadata> {
    let has_explicit_episodes = tokens_have_explicit_episode_pattern(tokens);
    let has_pack_signal = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "COMPLETE" | "BATCH" | "PACK" | "EXTRAS" | "SUBPACK"
        )
    });

    for token in tokens {
        if let Some((first, last, raw)) = parse_season_range_token(token) {
            return Some(new_season_pack_metadata(
                first,
                Some(raw),
                true,
                false,
                last > first,
                None,
                false,
            ));
        }
    }

    for index in 0..tokens.len() {
        let token = tokens[index].as_str();
        let next = tokens.get(index + 1).map(|value| value.as_str());

        let (season, raw, value_index) = if let Some(season) = parse_season_designator(token) {
            (season, token.to_string(), index)
        } else if matches!(token, "SEASON" | "SAISON" | "STAGIONE" | "TEMPORADA")
            && let Some(next) = next
            && let Some(season) = parse_numeric_token(next)
        {
            (season, format!("{token} {next}"), index + 1)
        } else {
            continue;
        };

        let mut season_part = None::<u32>;
        let mut is_season_extra = false;
        let mut is_multi_season = false;

        for offset in 0..=4 {
            let idx = value_index + offset;
            let Some(candidate) = tokens.get(idx) else {
                break;
            };

            if matches!(candidate.as_str(), "EXTRAS" | "SUBPACK") {
                is_season_extra = true;
            }

            if season_part.is_none() {
                if let Some((part, _)) = parse_pack_part_token(candidate) {
                    season_part = Some(part);
                } else if matches!(candidate.as_str(), "PART" | "VOL" | "VOLUME" | "PT")
                    && let Some(next) = tokens.get(idx + 1)
                    && let Some(part) = parse_numeric_token(next)
                {
                    season_part = Some(part);
                }
            }
        }

        if let Some(candidate) = tokens.get(value_index + 1) {
            if let Some(other_season) =
                parse_season_designator(candidate).or_else(|| parse_numeric_token(candidate))
                && other_season > season
                && parse_year(candidate).is_none()
            {
                is_multi_season = true;
            }
        }

        if !is_multi_season
            && let (Some(separator), Some(candidate)) =
                (tokens.get(value_index + 1), tokens.get(value_index + 2))
            && separator == "-"
            && let Some(other_season) =
                parse_season_designator(candidate).or_else(|| parse_numeric_token(candidate))
            && other_season > season
            && parse_year(candidate).is_none()
        {
            is_multi_season = true;
        }

        if has_explicit_episodes && !has_pack_signal && !is_season_extra && season_part.is_none() {
            continue;
        }

        let is_partial_season = season_part.is_some();
        return Some(new_season_pack_metadata(
            season,
            Some(raw),
            !is_partial_season,
            is_partial_season,
            is_multi_season,
            season_part,
            is_season_extra,
        ));
    }

    None
}

fn parse_mini_series_episode(tokens: &[String]) -> Option<ParsedEpisodeMetadata> {
    for (index, token) in tokens.iter().enumerate() {
        if let Some((part, raw)) = parse_pack_part_token(token) {
            return Some(finalize_episode_metadata(ParsedEpisodeMetadata {
                season: Some(1),
                episode_numbers: vec![part],
                is_mini_series: true,
                raw: Some(raw),
                ..ParsedEpisodeMetadata::default()
            }));
        }

        if matches!(token.as_str(), "PART" | "PT")
            && let Some(next) = tokens.get(index + 1)
            && let Some(part) = parse_numeric_token(next)
        {
            return Some(finalize_episode_metadata(ParsedEpisodeMetadata {
                season: Some(1),
                episode_numbers: vec![part],
                is_mini_series: true,
                raw: Some(format!("{token} {next}")),
                ..ParsedEpisodeMetadata::default()
            }));
        }

        if token.starts_with('E')
            && token.len() > 1
            && token[1..].bytes().all(|byte| byte.is_ascii_digit())
            && let Some(episode) = parse_numeric_token(&token[1..])
        {
            return Some(finalize_episode_metadata(ParsedEpisodeMetadata {
                season: Some(1),
                episode_numbers: vec![episode],
                is_mini_series: true,
                raw: Some(token.to_string()),
                ..ParsedEpisodeMetadata::default()
            }));
        }
    }

    None
}

fn merge_daily_context(
    base: Option<ParsedEpisodeMetadata>,
    tokens: &[String],
) -> Option<ParsedEpisodeMetadata> {
    let daily = parse_daily_episode(tokens)?;

    match base {
        Some(mut metadata) => {
            metadata.air_date = daily.air_date;
            metadata.daily_part = daily.daily_part;
            if metadata.raw.is_none() {
                metadata.raw = daily.raw;
            }
            Some(finalize_episode_metadata(metadata))
        }
        None => Some(daily),
    }
}

fn apply_special_context(
    base: Option<ParsedEpisodeMetadata>,
    tokens: &[String],
) -> Option<ParsedEpisodeMetadata> {
    let special = tokens
        .iter()
        .find_map(|token| parse_special_kind_token(token).map(|kind| (kind, token.clone())));

    match (base, special) {
        (Some(mut metadata), Some((kind, raw))) => {
            metadata.season = Some(0);
            metadata.special_kind = Some(kind);
            if metadata.special_absolute_episode_numbers.is_empty() {
                if !metadata.episode_numbers.is_empty() {
                    metadata.special_absolute_episode_numbers = metadata.episode_numbers.clone();
                } else if !metadata.absolute_episode_numbers.is_empty() {
                    metadata.special_absolute_episode_numbers = metadata.absolute_episode_numbers.clone();
                    metadata.episode_numbers = metadata.absolute_episode_numbers.clone();
                    metadata.absolute_episode_numbers.clear();
                    metadata.absolute_episode = None;
                }
            }
            if metadata.raw.is_none() {
                metadata.raw = Some(raw);
            }
            Some(finalize_episode_metadata(metadata))
        }
        (Some(mut metadata), None) => {
            if metadata.season == Some(0) && metadata.special_kind.is_none() {
                metadata.special_kind = Some(ParsedSpecialKind::Special);
                if metadata.special_absolute_episode_numbers.is_empty() && !metadata.episode_numbers.is_empty() {
                    metadata.special_absolute_episode_numbers = metadata.episode_numbers.clone();
                }
            }
            Some(finalize_episode_metadata(metadata))
        }
        (None, Some((kind, raw))) => Some(finalize_episode_metadata(ParsedEpisodeMetadata {
            season: Some(0),
            special_kind: Some(kind),
            raw: Some(raw),
            ..ParsedEpisodeMetadata::default()
        })),
        (None, None) => None,
    }
}

fn parse_series_episode_core(tokens: &[String]) -> Option<ParsedEpisodeMetadata> {
    let mut pending_season: Option<u32> = None;
    let mut pending_season_raw: Option<String> = None;
    let mut pending_episode_anchor: bool = false;
    let mut pending_episode_anchor_raw: Option<String> = None;
    let mut pending_absolute: Option<(u32, String)> = None;
    let mut skip_next_as_season_value = false;
    for (idx, token) in tokens.iter().enumerate() {
        let next = tokens.get(idx + 1).map(|value| value.as_str());

        if parse_season_range_token(token).is_some() {
            continue;
        }

        if let Some((season, episodes, _)) = parse_episode_token(token)
            && episodes
                .iter()
                .all(|value| is_reasonable_episode_number(*value))
        {
            let raw = if token.starts_with('S') && token.contains('-') && !token.contains('E') {
                token.replace('-', " ")
            } else if token.contains('X')
                && token.chars().any(|character| character.is_ascii_digit())
            {
                token.replace('X', "x")
            } else {
                token.clone()
            };

            return Some(new_episode_metadata(season, episodes, Vec::new(), Some(raw)));
        }

        if skip_next_as_season_value {
            // This token was consumed as a standalone season value like:
            // "SEASON 2 EPISODE 03". Do not let it be interpreted as episode 2.
            skip_next_as_season_value = false;
            continue;
        }

        if let Some(season) = pending_season {
            if parse_named_episode_anchor_token(token) {
                pending_episode_anchor = true;
                pending_episode_anchor_raw = Some(token.to_string());
                continue;
            }

            if pending_episode_anchor {
                let _ = pending_episode_anchor;
                let episodes = parse_named_episode_token(token);
                let episodes = if episodes.is_empty() {
                    parse_pending_episode_token(token)
                } else {
                    episodes
                };

                if !episodes.is_empty()
                    && episodes
                        .iter()
                        .all(|value| is_reasonable_episode_number(*value))
                {
                    let raw = pending_season_raw.as_ref().map_or_else(
                        || format!("S{season} {token}"),
                        |season_token| {
                            if let Some(anchor) = pending_episode_anchor_raw.as_ref() {
                                format!("{season_token} {anchor} {token}")
                            } else {
                                format!("{season_token} {token}")
                            }
                        },
                    );

                    return Some(new_episode_metadata(Some(season), episodes, Vec::new(), Some(raw)));
                }
            }

            let episodes = parse_named_episode_token(token);
            if !episodes.is_empty()
                && episodes
                    .iter()
                    .all(|value| is_reasonable_episode_number(*value))
            {
                let raw = pending_season_raw.as_ref().map_or_else(
                    || format!("S{season} {token}"),
                    |season_token| {
                        if let Some(anchor) = pending_episode_anchor_raw.as_ref() {
                            format!("{season_token} {anchor} {token}")
                        } else {
                            format!("{season_token} {token}")
                        }
                    },
                );

                return Some(new_episode_metadata(Some(season), episodes, Vec::new(), Some(raw)));
            }

            let delayed_episodes = parse_pending_episode_token(token);
            if !delayed_episodes.is_empty()
                && delayed_episodes
                    .iter()
                    .all(|value| is_reasonable_episode_number(*value))
            {
                let raw = pending_season_raw.as_ref().map_or_else(
                    || format!("S{season} {token}"),
                    |season_token| {
                        if let Some(anchor) = pending_episode_anchor_raw.as_ref() {
                            format!("{season_token} {anchor} {token}")
                        } else {
                            format!("{season_token} {token}")
                        }
                    },
                );

                if parse_year(token).is_some()
                    || parse_quality(token).is_some()
                    || parse_source(token, next).is_some()
                    || parse_video(token).0.is_some()
                    || parse_audio(token, next).is_some()
                    || is_noise_token(token)
                    || matches!(token.as_str(), "COMPLETE" | "BATCH" | "PACK")
                {
                    continue;
                }

                return Some(new_episode_metadata(
                    Some(season),
                    delayed_episodes,
                    Vec::new(),
                    Some(raw),
                ));
            }

            pending_episode_anchor = false;
            pending_episode_anchor_raw = None;
            continue;
        }

        if token == "SEASON" || token == "S" {
            if let Some(next) = next
                && let Some(season) = parse_numeric_token(next)
            {
                pending_season = Some(season);
                pending_season_raw = Some(format!("{token} {next}"));
                skip_next_as_season_value = true;
            }

            continue;
        }

        if parse_named_episode_anchor_token(token) {
            if pending_season.is_some() {
                pending_episode_anchor = true;
                pending_episode_anchor_raw = Some(token.to_string());
            }
            continue;
        }

        if let Some(season) = parse_named_season_token(token) {
            pending_season = Some(season);
            pending_season_raw = Some(token.clone());
            pending_episode_anchor = false;
            pending_episode_anchor_raw = None;
            continue;
        }

        if let Some(season) = parse_series_only_season(token) {
            pending_season = Some(season);
            pending_season_raw = Some(token.clone());
            pending_episode_anchor = false;
            pending_episode_anchor_raw = None;

            if let Some(next) = next {
                let episodes = parse_named_episode_token(next);
                if !episodes.is_empty()
                    && episodes
                        .iter()
                        .all(|value| is_reasonable_episode_number(*value))
                {
                    let raw = format!("{token} {next}");
                    return Some(new_episode_metadata(
                        Some(season),
                        episodes,
                        Vec::new(),
                        Some(raw),
                    ));
                }

                let episodes = parse_pending_episode_token(next);
                if !episodes.is_empty()
                    && episodes
                        .iter()
                        .all(|value| is_reasonable_episode_number(*value))
                    && is_reasonable_episode_series(next)
                {
                    return Some(new_episode_metadata(
                        Some(season),
                        episodes,
                        Vec::new(),
                        Some(format!("{token} {next}")),
                    ));
                }
            }

            continue;
        }

        // Bare digit-range tokens like "1122-1133", "0001-0782" (fansub episode ranges)
        if idx > 0
            && token.contains('-')
            && let Some((left_str, right_str)) = token.split_once('-')
            && !left_str.is_empty()
            && !right_str.is_empty()
            && left_str.chars().all(|c| c.is_ascii_digit())
            && right_str.chars().all(|c| c.is_ascii_digit())
            && let Ok(left_val) = left_str.parse::<u32>()
            && let Ok(right_val) = right_str.parse::<u32>()
            && left_val <= right_val
            && is_reasonable_episode_number(left_val)
            && is_reasonable_episode_number(right_val)
        {
            let episodes: Vec<u32> = (left_val..=right_val).collect();
            return Some(new_episode_metadata(
                None,
                episodes.clone(),
                episodes,
                Some(token.to_string()),
            ));
        }

        // "E795-E940" style ranges
        if idx > 0 && token.starts_with('E') && token.contains('-') {
            let frag = &token[1..]; // strip leading E
            let episodes = parse_episode_fragment(frag);
            if !episodes.is_empty()
                && episodes
                    .iter()
                    .all(|value| is_reasonable_episode_number(*value))
            {
                return Some(new_episode_metadata(
                    None,
                    episodes.clone(),
                    episodes,
                    Some(token.to_string()),
                ));
            }
        }

        // Bare single absolute episode number, optionally with anime version suffix (e.g. "1155V2")
        {
            let prev = idx.checked_sub(1).and_then(|prev_idx| tokens.get(prev_idx));
            let (digit_part, _version) = if let Some(ver) = parse_anime_version(token) {
                (&token[..token.len() - 2], Some(ver))
            } else {
                (token.as_str(), None)
            };
            let next_is_numeric = next.is_some_and(|value| is_digit_str(value));
            let prev_is_numeric = prev.is_some_and(|value| is_digit_str(value));
            let surrounded_by_media_tokens = prev.is_some_and(|value| {
                parse_audio(value, next).is_some()
                    || parse_video(value).0.is_some()
                    || parse_source(value, next).is_some()
            }) || next.is_some_and(|value| {
                parse_audio(value, None).is_some()
                    || parse_video(value).0.is_some()
                    || matches!(value, "BIT" | "CH" | "CHS" | "FPS")
            });
            if is_digit_str(digit_part)
                && idx > 0
                && parse_quality(digit_part).is_none()
                && is_reasonable_episode_number(digit_part.parse::<u32>().ok()?)
                && (digit_part.len() <= 3
                    || (digit_part.len() == 4 && parse_year(digit_part).is_none()))
                && !next_is_numeric
                && !prev_is_numeric
                && !surrounded_by_media_tokens
                && let Ok(episode) = digit_part.parse::<u32>()
            {
                pending_absolute = Some((episode, token.to_string()));
            }
        }

        // Tilde range: "01 ~ 07" → tokens are ["01", "~", "07"]
        if (token == "~" || token == "—")
            && let Some(prev_abs) = pending_absolute.take()
            && let Some(next_token) = next
            && is_digit_str(next_token)
            && let Ok(right_val) = next_token.parse::<u32>()
            && prev_abs.0 <= right_val
            && is_reasonable_episode_number(right_val)
        {
            let episodes: Vec<u32> = (prev_abs.0..=right_val).collect();
            return Some(new_episode_metadata(
                None,
                episodes.clone(),
                episodes,
                Some(format!("{} ~ {}", prev_abs.1, next_token)),
            ));
        }
    }

    if let Some(season) = pending_season {
        return Some(new_season_pack_metadata(
            season,
            pending_season_raw,
            true,
            false,
            false,
            None,
            false,
        ));
    }

    pending_absolute.and_then(|(episode, raw)| {
        (episode > 0).then(|| new_absolute_episode_metadata(vec![episode], Some(raw)))
    })
}

pub fn parse_series_episode(raw_title: &str) -> Option<ParsedEpisodeMetadata> {
    let tokens = split_title(raw_title);
    if tokens.is_empty() {
        return None;
    }

    let mut parsed = parse_season_pack(&tokens)
        .or_else(|| parse_series_episode_core(&tokens))
        .or_else(|| parse_mini_series_episode(&tokens));
    if let Some(with_daily) = merge_daily_context(parsed.clone(), &tokens) {
        parsed = Some(with_daily);
    }
    parsed = apply_special_context(parsed, &tokens);
    parsed.map(finalize_episode_metadata)
}

/// Parse anime version suffix (e.g. "V2", "01V2", "05V3").
/// Returns the version number if found (2-9).
fn parse_anime_version(token: &str) -> Option<u32> {
    // Pure "V2", "V3" etc.
    if token.len() >= 2
        && token.starts_with('V')
        && let Ok(ver) = token[1..].parse::<u32>()
        && (2..=9).contains(&ver)
    {
        return Some(ver);
    }
    // "01V2", "05V3" — digits followed by V and a single digit
    if let Some(pos) = token.find('V')
        && pos > 0
        && token[..pos].chars().all(|c| c.is_ascii_digit())
        && let Ok(ver) = token[pos + 1..].parse::<u32>()
        && (2..=9).contains(&ver)
    {
        return Some(ver);
    }
    None
}

pub fn parse_release_metadata(raw_title: &str) -> ParsedReleaseMetadata {
    let cleaned_title = sanitize_release_title(raw_title);
    let tokens = split_title(&cleaned_title);
    let mut parsed = ParsedReleaseMetadata {
        raw_title: raw_title.to_string(),
        normalized_title: String::new(),
        normalized_title_variants: Vec::new(),
        release_group: extract_release_group(&cleaned_title, &tokens),
        languages_audio: Vec::new(),
        languages_subtitles: Vec::new(),
        imdb_id: parse_imdb_id_from_tokens(&tokens),
        tmdb_id: parse_tmdb_id_from_tokens(&tokens),
        is_atmos: false,
        year: None,
        quality: None,
        source: None,
        video_codec: None,
        video_encoding: None,
        audio: None,
        audio_codecs: Vec::new(),
        audio_channels: None,
        is_dual_audio: false,
        is_dolby_vision: false,
        detected_hdr: false,
        is_hdr10plus: false,
        is_hlg: false,
        fps: parse_fps(&cleaned_title),
        is_proper_upload: false,
        is_repack: false,
        is_remux: false,
        is_bd_disk: false,
        is_ai_enhanced: false,
        is_hardcoded_subs: false,
        streaming_service: None,
        edition: None,
        anime_version: None,
        episode: None,
        parser_version: RELEASE_PARSER_VERSION,
        parse_confidence: 0.0,
        missing_fields: Vec::new(),
        parse_hints: Vec::new(),
    };

    let mut language_context = LanguageScope::Auto;
    let mut default_dual_applied = false;
    let mut explicit_language_seen = false;

    let mut i = 0usize;
    while i < tokens.len() {
        let token = tokens[i].as_str();
        let next = tokens.get(i + 1).map(|next| next.as_str());

        if token == "PROPER" || token == "REPACK" {
            parsed.is_proper_upload = true;
            if token == "REPACK" {
                parsed.is_repack = true;
            }
            i += 1;
            continue;
        }

        // Anime version detection (v2, v3, etc. — also handles "01V2" style)
        if let Some(ver) = parse_anime_version(token) {
            parsed.anime_version = Some(ver);
            parsed.is_proper_upload = true;
            i += 1;
            continue;
        }

        if token == "KORSUB" || token == "KORSUBS" {
            parsed.is_hardcoded_subs = true;
            if parsed
                .languages_subtitles
                .iter()
                .all(|language| !language.eq_ignore_ascii_case("kor"))
            {
                parsed.languages_subtitles.push("kor".to_string());
            }
            language_context = LanguageScope::Subtitle;
            i += 1;
            continue;
        }

        // Hardcoded subtitles
        if token == "HC" || token == "HARDCODED" || token == "HARDSUBBED" || token == "HARDSUB" {
            parsed.is_hardcoded_subs = true;
            i += 1;
            continue;
        }

        // Edition detection
        if parsed.edition.is_none()
            && let Some((edition, consumed)) = parse_edition_at(&tokens, i)
        {
            parsed.edition = Some(edition);
            i += consumed;
            continue;
        }

        if token == "REMUX" {
            parsed.is_remux = true;
            i += 1;
            continue;
        }

        if matches!(token, "BD25" | "BD50" | "BDMV" | "BDISO" | "BRDISK") {
            parsed.is_bd_disk = true;
            if parsed.source.is_none() {
                parsed.source = Some("BRDISK".to_string());
            }
            i += 1;
            continue;
        }

        // COMPLETE BLURAY / COMPLETE UHD BLURAY = full disc
        if token == "COMPLETE" {
            let next2 = tokens.get(i + 2).map(|t| t.as_str());
            if matches!(next, Some("BLURAY") | Some("BLU"))
                || (next == Some("UHD") && matches!(next2, Some("BLURAY") | Some("BLU")))
            {
                parsed.is_bd_disk = true;
                if parsed.source.is_none() {
                    parsed.source = Some("BRDISK".to_string());
                }
            }
        }

        if token == "AI" && next == Some("ENHANCED") {
            parsed.is_ai_enhanced = true;
            i += 2;
            continue;
        }
        if token == "AIENHANCED" || token == "RIFE" {
            parsed.is_ai_enhanced = true;
            i += 1;
            continue;
        }

        if token == "DUAL" || token == "DUALAUDIO" || token == "DUAL-AUDIO" {
            parsed.is_dual_audio = true;
            language_context = LanguageScope::Audio;
            if parsed.languages_audio.is_empty() && !explicit_language_seen {
                parsed.languages_audio = vec!["eng".to_string(), "jpn".to_string()];
                default_dual_applied = true;
            }
            i += 1;
            continue;
        }

        if token == "ATMOS" || token == "ATMOSPHERE" {
            parsed.is_atmos = true;
            i += 1;
            continue;
        }

        if token == "VOSTFR" {
            language_context = LanguageScope::Subtitle;
            explicit_language_seen = true;
            if parsed.languages_subtitles.is_empty()
                || !parsed
                    .languages_subtitles
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case("fre"))
            {
                parsed.languages_subtitles.push("fre".to_string());
            }
            parsed
                .parse_hints
                .push("language_context=subtitle,vostfr".to_string());
            i += 1;
            continue;
        }

        if let Some(scope) = has_language_context_token(token) {
            language_context = scope;
            let scope_name = match scope {
                LanguageScope::Auto => "auto",
                LanguageScope::Audio => "audio",
                LanguageScope::Subtitle => "subtitle",
            };
            parsed
                .parse_hints
                .push(format!("language_context={scope_name}"));
            i += 1;
            continue;
        }

        if let Some(language) = parse_language_hint(token) {
            if default_dual_applied && !explicit_language_seen {
                parsed.languages_audio.clear();
                default_dual_applied = false;
            }

            explicit_language_seen = true;
            match language_context {
                LanguageScope::Subtitle => parsed.languages_subtitles.push(language.to_string()),
                LanguageScope::Audio | LanguageScope::Auto => {
                    parsed.languages_audio.push(language.to_string())
                }
            }
            i += 1;
            continue;
        }

        language_context = LanguageScope::Auto;

        if token == "DOVI" || (token == "DOLBY" && next == Some("VISION")) || token == "DV" {
            parsed.is_dolby_vision = true;
            parsed.detected_hdr = true;
            i += 1;
            if token == "DOLBY" {
                i += 1;
            }
            continue;
        }

        if token == "HDR"
            || token == "HDR10"
            || token == "HDR10PLUS"
            || token == "HDR10+"
            || token == "HDR10P"
            || token == "HDRVIVID"
            || token == "HLG"
        {
            parsed.detected_hdr = true;
            if token == "HDR10PLUS" || token == "HDR10+" || token == "HDR10P" {
                parsed.is_hdr10plus = true;
            }
            if token == "HLG" {
                parsed.is_hlg = true;
            }
            i += 1;
            continue;
        }

        if let Some(year) = parse_year(token) {
            // Prefer the latest year token to avoid treating numeric movie titles
            // (e.g. "2048.2019...") as the release year.
            parsed.year = Some(year);
        }

        if parsed.quality.is_none()
            && let Some(quality) = parse_quality(token)
        {
            parsed.quality = Some(quality.to_string());
        }

        if parsed.source.is_none()
            && let Some(result) = parse_source(token, next)
        {
            parsed.source = Some(result.source.to_string());
            if let Some(service) = result.service {
                parsed.streaming_service = Some(service.to_string());
            }
        }

        if parsed.video_codec.is_none() {
            let (codec, encoding) = parse_video(token);
            if codec.is_some() {
                parsed.video_codec = codec;
                parsed.video_encoding = encoding;
            }
        } else if parsed.video_encoding.is_none() {
            // Codec already set (e.g., HEVC → H.265) but encoding not yet captured.
            // Check if this token is an encoding indicator like x265.
            let (_, encoding) = parse_video(token);
            if encoding.is_some() {
                parsed.video_encoding = encoding;
            }
        }

        if let Some(audio) = parse_audio(token, next) {
            let codec_value = audio.codec.to_string();
            if parsed.audio.is_none() {
                parsed.audio = Some(codec_value.clone());
            }
            parsed.audio_codecs.push(codec_value);

            if parsed.audio_channels.is_none()
                && let Some(channels) = audio.channels.as_ref()
            {
                parsed.audio_channels = Some(channels.to_string());
            }
            if audio.channels.is_none()
                && matches!(
                    audio.codec,
                    "DDP"
                        | "DD"
                        | "AAC"
                        | "AC3"
                        | "DTS"
                        | "DTSHD"
                        | "DTSMA"
                        | "DTSX"
                        | "TRUEHD"
                        | "EAC3"
                        | "PCM"
                )
            {
                // Some feeds separate channel information into subsequent tokens
                // (for example "DDP.ATMOS.5.1").
                for offset in 1..=3 {
                    if i + offset >= tokens.len() {
                        break;
                    }

                    if let Some(channels) = parse_channels(tokens[i + offset].as_str()) {
                        parsed.audio_channels = Some(channels);
                        break;
                    }

                    if i + offset + 1 < tokens.len() {
                        let left = tokens[i + offset].as_str();
                        let right = tokens[i + offset + 1].as_str();
                        if is_digit_str(left) && is_digit_str(right) {
                            parsed.audio_channels = Some(format!("{left}.{right}"));
                            break;
                        }
                    }
                }
            }
        }

        i += 1;
    }

    parsed.episode = parse_series_episode(&cleaned_title);

    // Detect anime version embedded in the episode token (e.g. S05E01V2).
    if parsed.anime_version.is_none()
        && let Some(ref raw) = parsed.episode.as_ref().and_then(|ep| ep.raw.clone())
    {
        let upper = raw.to_ascii_uppercase();
        if let Some(ver) = extract_trailing_version(&upper) {
            parsed.anime_version = Some(ver);
            parsed.is_proper_upload = true;
        }
    }
    let title_tokens =
        collect_normalized_title_tokens(&tokens, &parsed.episode, parsed.release_group.as_deref());
    parsed.normalized_title =
        normalize_title_tokens(&tokens, &parsed.episode, parsed.release_group.as_deref());
    if parsed.normalized_title.is_empty() {
        parsed.normalized_title = tokens
            .iter()
            .filter(|token| !is_noise_token(token))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
    }
    parsed.normalized_title_variants =
        build_normalized_title_variants(&title_tokens, &parsed.normalized_title);
    if parsed.normalized_title_variants.is_empty() && !parsed.normalized_title.is_empty() {
        parsed
            .normalized_title_variants
            .push(parsed.normalized_title.clone());
    }

    parsed.languages_audio = dedupe_keep_order(parsed.languages_audio);
    parsed.languages_subtitles = dedupe_keep_order(parsed.languages_subtitles);
    parsed.audio_codecs = dedupe_keep_order(parsed.audio_codecs);

    if parsed.languages_audio.is_empty()
        && default_dual_applied
        && parsed.languages_subtitles.is_empty()
    {
        parsed.languages_audio = vec!["eng".to_string(), "jpn".to_string()];
    }

    let mut confidence = 0.35f32;
    if parsed.quality.is_some() {
        confidence += 0.16;
    } else {
        parsed.missing_fields.push("quality".to_string());
    }

    if parsed.source.is_some() {
        confidence += 0.12;
    } else {
        parsed.missing_fields.push("source".to_string());
    }

    if parsed.video_codec.is_some() {
        confidence += 0.12;
    } else {
        parsed.missing_fields.push("video_codec".to_string());
    }

    if parsed.audio.is_some() {
        confidence += 0.10;
    } else {
        parsed.missing_fields.push("audio".to_string());
    }

    if parsed.year.is_some() {
        confidence += 0.05;
    } else {
        parsed.missing_fields.push("year".to_string());
    }

    if parsed.fps.is_some() {
        confidence += 0.05;
    }

    // FPS above 60 is almost certainly AI frame interpolation (RIFE etc.)
    if let Some(fps) = parsed.fps
        && fps > 60.0
    {
        parsed.is_ai_enhanced = true;
    }

    if parsed.is_ai_enhanced {
        confidence += 0.05;
    }

    parsed.parse_confidence = confidence.min(1.0);

    parsed
}

#[cfg(test)]
#[path = "release_parser_tests.rs"]
mod release_parser_tests;
