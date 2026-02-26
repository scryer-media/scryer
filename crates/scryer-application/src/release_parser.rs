#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEpisodeMetadata {
    pub season: Option<u32>,
    pub episode_numbers: Vec<u32>,
    pub absolute_episode: Option<u32>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedReleaseMetadata {
    pub raw_title: String,
    pub normalized_title: String,
    pub release_group: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
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
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
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

fn normalize_language_token(token: &str) -> Option<&'static str> {
    match token {
        "EN" | "ENG" | "ENGLISH" => Some("eng"),
        "EN-GB" => Some("eng"),
        "JP" | "JPN" | "JAP" | "JAPANESE" => Some("jpn"),
        "FR" | "FRA" | "FRE" | "FRENCH" => Some("fra"),
        "DE" | "DEU" | "GER" | "GERMAN" => Some("deu"),
        "ES" | "SPA" | "ESP" | "SPANISH" => Some("spa"),
        "IT" | "ITA" | "ITALIAN" => Some("ita"),
        "RU" | "RUS" | "RUSSIAN" => Some("rus"),
        "PT" | "POR" | "PORTUGUESE" => Some("por"),
        "PTBR" | "POR-BR" | "PT-BR" => Some("por"),
        "LATINO" | "LAT" => Some("spa"),
        "PL" | "POL" | "POLISH" => Some("pol"),
        "FI" | "FIN" | "FINNISH" => Some("fin"),
        "HU" | "HUN" | "HUNGARIAN" => Some("hun"),
        "HE" | "HEB" | "HEBREW" => Some("heb"),
        "ZH" | "ZHO" | "CHI" | "CHINESE" => Some("zho"),
        "KO" | "KOR" | "KOREAN" => Some("kor"),
        "SV" | "SWE" | "SWEDISH" => Some("swe"),
        "NO" | "NOR" | "NORWEGIAN" => Some("nor"),
        "DA" | "DAN" | "DANISH" => Some("dan"),
        "NL" | "NLD" | "DUTCH" => Some("nld"),
        "CS" | "CES" | "CZECH" => Some("ces"),
        "TR" | "TUR" | "TURKISH" => Some("tur"),
        "AR" | "ARA" => Some("ara"),
        _ => None,
    }
}

#[allow(dead_code)]
fn is_episode_anchor_token(token: &str) -> bool {
    matches!(token, "E" | "EP" | "EPISODE" | "EPISODES" | "CHAPTER")
}

fn parse_named_season_token(token: &str) -> Option<u32> {
    if let Some(rest) = token.strip_prefix("SEASON") {
        if rest.is_empty() {
            return None;
        }
        let rest =
            rest.trim_start_matches(['-', '.', '_', ':']);
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
            } else if source.eq_ignore_ascii_case("BluRay") || source.eq_ignore_ascii_case("UHD") {
                score += 850;
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

        score
    }
}

impl ParsedEpisodeMetadata {
    pub fn first_episode(&self) -> Option<u32> {
        self.episode_numbers.first().copied()
    }
}

fn is_digit_str(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn is_hex_token(value: &str) -> bool {
    value.len() >= 7 && value.len() <= 10 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn split_title(raw_title: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(raw_title.len());

    for ch in raw_title.chars() {
        match ch {
            '[' | ']' | '(' | ')' | '{' | '}' | '_' => normalized.push(' '),
            _ => normalized.push(ch.to_ascii_uppercase()),
        }
    }

    normalized
        .split_whitespace()
        .flat_map(split_release_token)
        .collect()
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

fn parse_source(token: &str, next: Option<&str>) -> Option<&'static str> {
    let upper = token;
    if upper == "AMZN" || upper == "NF" || upper == "BILI" || upper == "ATVP" || upper == "HULU" {
        return Some("WEB-DL");
    }
    if upper == "AMAZON" || upper == "NETFLIX" || upper == "HOTSTAR" || upper == "ITUNES" {
        return Some("WEB-DL");
    }

    if upper == "WEB" && next.is_some_and(|next| next == "DL") {
        return Some("WEB-DL");
    }

    match upper {
        "WEB" | "WEBDL" | "WEB-DL" | "WEBRIP" | "WEBHLS" => Some("WEB-DL"),
        "WEBD" => Some("WEB-DL"),
        "BLURAY" if next == Some("RAY") => Some("BluRay"),
        "BLURAY" | "BLU" | "BD" | "UHD" => Some("BluRay"),
        "HDTV" | "HDTVRIP" => Some("HDTV"),
        "DVDRIP" | "DVD" => Some("DVD"),
        "CR" => Some("WEB-DL"),
        "BBC" => Some("WEB-DL"),
        "ITUNES" => Some("WEB-DL"),
        _ => None,
    }
}

fn is_hash_like(token: &str) -> bool {
    if !(6..=16).contains(&token.len()) {
        return false;
    }

    token.chars().all(|character| character.is_ascii_hexdigit())
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
    for (open, close) in [
        ('[', ']'),
        ('(', ')'),
        ('{', '}'),
    ] {
        for candidate in extract_delimited_sections(raw_title, open, close) {
            if let Some(normalized) = normalize_release_group_candidate(&candidate) {
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
            | "BDRIP"
            | "RIP"
            | "DV"
            | "HDR"
            | "HDR10"
            | "HDR10PLUS"
            | "HDR10P"
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
            | "DDP"
            | "AC3"
            | "DTS"
            | "EAC3"
            | "TRUEHD"
    )
}

fn extract_release_group_from_tokens(tokens: &[String]) -> Option<String> {
    for token in tokens.iter().rev() {
        if is_release_group_token(token) {
            return Some(token.to_string());
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
        if let Some(group) = normalize_release_group_candidate(candidate) {
            return Some(group);
        }
    }

    None
}

fn extract_release_group(raw_title: &str, tokens: &[String]) -> Option<String> {
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
            out.push(token[season_start..].to_string());
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
            if (current == "H" || current == "X")
                && (next_value == "264" || next_value == "265")
            {
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

            if (current.starts_with("DDP")
                || current.starts_with("DD+")
                || current.starts_with("AAC")
                || current.starts_with("AC-3")
                || current == "AC3")
                && is_digit_str(next_value)
            {
                if current.starts_with("AC-3") || current == "AC3" {
                    if let Some(third_value) = third {
                        if is_digit_str(third_value) {
                            parts.push(format!("{current}.{next_value}.{third_value}"));
                            index += 3;
                            continue;
                        }
                    }
                } else {
                    parts.push(format!("{current}.{next_value}"));
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

    if token.starts_with("DDP") || token.starts_with("DD+") || token == "DD" || token == "DD5" {
        let suffix = token
            .trim_start_matches("DDP")
            .trim_start_matches("DD+")
            .trim_start_matches("DD");
        let channels = parse_channels(suffix).or_else(|| next.and_then(parse_channels));
        return Some(ParsedAudio {
            codec: "DDP",
            channels,
        });
    }

    if token.starts_with("AC3") || token.starts_with("AC-3") {
        let suffix = token
            .trim_start_matches("AC-3")
            .trim_start_matches("AC3");
        return Some(ParsedAudio {
            codec: "AC3",
            channels: parse_channels(suffix)
                .or_else(|| parse_channels(token))
                .or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DD") && parse_channels(token).is_some() {
        return Some(ParsedAudio {
            codec: "DDP",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("EAC3") || token.starts_with("E-AC-3") || token.starts_with("EAC") {
        return Some(ParsedAudio {
            codec: "EAC3",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }

    if token.starts_with("DTS-HD")
        || token.starts_with("DTSHD")
        || token.starts_with("DTS-MA")
        || token.starts_with("DTSMA")
    {
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

    if token == "LPCM" {
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
        if let Ok(fps) = chunk.parse::<f32>() {
            if (10.0..=300.0).contains(&fps) {
                return Some(fps);
            }
        }
    }

    let compact = upper.replace(['[', ']', '(', ')', '{', '}'], " ");
    let parts: Vec<_> = compact.split_whitespace().collect();
    for chunk in parts {
        if let Some(prefix) = chunk.strip_suffix("FPS") {
            let prefix = prefix.trim();
            if let Ok(fps) = prefix.parse::<f32>() {
                if (10.0..=300.0).contains(&fps) {
                    return Some(fps);
                }
            }
        }
    }

    None
}

fn is_noise_token(token: &str) -> bool {
    if token.len() <= 1 {
        return true;
    }

    if is_hex_token(token) || parse_year(token).is_some() || parse_quality(token).is_some() {
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
            | "SEASON"
            | "EPISODE"
            | "GROUP"
            | "REMUX"
            | "AMZN"
            | "NF"
            | "BILI"
            | "ATVP"
            | "HULU"
            | "BD"
            | "BLURAY"
            | "BDRIP"
            | "BD25"
            | "BD50"
            | "PROPER"
            | "REPACK"
            | "EXTENDED"
            | "LIMITED"
            | "HDRIP"
            | "CR"
            | "WEBDL"
            | "WEBRIP"
            | "HDCAM"
            | "CAM"
    ) || is_language_token(token)
}

fn normalize_title_tokens(tokens: &[String], episode: &Option<ParsedEpisodeMetadata>) -> String {
    let mut out = Vec::new();
    let episode_raw = episode
        .as_ref()
        .and_then(|ep| ep.raw.as_ref())
        .map(|raw| raw.to_ascii_uppercase());

    for token in tokens {
        if is_noise_token(token) {
            continue;
        }

        if let Some(raw) = &episode_raw {
            if raw == token {
                continue;
            }
        }

        if token.chars().all(|c| c.is_ascii_alphabetic())
            || token.chars().any(|c| c.is_ascii_alphabetic())
        {
            out.push(token.to_string());
        }
    }

    out.join(" ")
}

fn parse_episode_token(token: &str) -> Option<(Option<u32>, Vec<u32>)> {
    if token.len() < 3 {
        return None;
    }

    if token.starts_with('S') {
        let (_, tail) = token.split_at(1);
        let (season, rest) = parse_leading_digits(tail)?;
        let rest =
            rest.trim_start_matches(['-', '.', '_', ':']);
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
            return Some((Some(season), episodes));
        }
    }

    if let Some((left, right)) = token.split_once('X') {
        if let Ok(season) = left.parse::<u32>() {
            let episodes = parse_episode_fragment(right);
            if !episodes.is_empty() {
                return Some((Some(season), episodes));
            }
        }
    }

    None
}

pub fn parse_series_episode(raw_title: &str) -> Option<ParsedEpisodeMetadata> {
    let tokens = split_title(raw_title);
    if tokens.is_empty() {
        return None;
    }

    let mut pending_season: Option<u32> = None;
    let mut pending_season_raw: Option<String> = None;
    let mut pending_episode_anchor: bool = false;
    let mut pending_episode_anchor_raw: Option<String> = None;
    let mut pending_absolute: Option<(u32, String)> = None;
    let mut skip_next_as_season_value = false;
    for (idx, token) in tokens.iter().enumerate() {
        let next = tokens.get(idx + 1).map(|value| value.as_str());

        if let Some((season, episodes)) = parse_episode_token(token) {
            if episodes
                .iter()
                .all(|value| is_reasonable_episode_number(*value))
            {
                let raw = if token.starts_with('S') && token.contains('-') && !token.contains('E')
                {
                    token.replace('-', " ")
                } else if token.contains('X')
                    && token.chars().any(|character| character.is_ascii_digit())
                {
                    token.replace('X', "x")
                } else {
                    token.clone()
                };

                return Some(ParsedEpisodeMetadata {
                    season,
                    episode_numbers: episodes,
                    absolute_episode: None,
                    raw: Some(raw),
                });
            }
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

                    return Some(ParsedEpisodeMetadata {
                        season: Some(season),
                        episode_numbers: episodes,
                        absolute_episode: None,
                        raw: Some(raw),
                    });
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

                return Some(ParsedEpisodeMetadata {
                    season: Some(season),
                    episode_numbers: episodes,
                    absolute_episode: None,
                    raw: Some(raw),
                });
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

                return Some(ParsedEpisodeMetadata {
                    season: Some(season),
                    episode_numbers: delayed_episodes,
                    absolute_episode: None,
                    raw: Some(raw),
                });
            }

            pending_season = None;
            pending_season_raw = None;
            pending_episode_anchor = false;
            pending_episode_anchor_raw = None;
        }

        if token == "SEASON" || token == "S" {
            if let Some(next) = next {
                if let Some(season) = parse_numeric_token(next) {
                    pending_season = Some(season);
                    pending_season_raw = Some(format!("{token} {next}"));
                    skip_next_as_season_value = true;
                }
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
                    return Some(ParsedEpisodeMetadata {
                        season: Some(season),
                        episode_numbers: episodes,
                        absolute_episode: None,
                        raw: Some(raw),
                    });
                }

                let episodes = parse_pending_episode_token(next);
                if !episodes.is_empty()
                    && episodes
                        .iter()
                        .all(|value| is_reasonable_episode_number(*value))
                    && is_reasonable_episode_series(next)
                {
                    return Some(ParsedEpisodeMetadata {
                        season: Some(season),
                        episode_numbers: episodes,
                        absolute_episode: None,
                        raw: Some(format!("{token} {next}")),
                    });
                }
            }

            continue;
        }

        if is_digit_str(token) && idx > 0 && parse_quality(token).is_none()
            && is_reasonable_episode_number(token.parse::<u32>().ok()?)
                && (token.len() <= 3 || (token.len() == 4 && parse_year(token).is_none()))
            {
                if let Ok(episode) = token.parse::<u32>() {
                    pending_absolute = Some((episode, token.to_string()));
                }
            }
    }

    pending_absolute.and_then(|(episode, raw)| {
        (episode > 0).then(|| ParsedEpisodeMetadata {
            season: None,
            episode_numbers: Vec::new(),
            absolute_episode: Some(episode),
            raw: Some(raw),
        })
    })
}

pub fn parse_release_metadata(raw_title: &str) -> ParsedReleaseMetadata {
    let tokens = split_title(raw_title);
    let mut parsed = ParsedReleaseMetadata {
        raw_title: raw_title.to_string(),
        normalized_title: String::new(),
        release_group: extract_release_group(raw_title, &tokens),
        languages_audio: Vec::new(),
        languages_subtitles: Vec::new(),
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
        fps: parse_fps(raw_title),
        is_proper_upload: false,
        is_remux: false,
        is_bd_disk: false,
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
            i += 1;
            continue;
        }

        if token == "REMUX" {
            parsed.is_remux = true;
            i += 1;
            continue;
        }

        if token == "BD25" || token == "BD50" || token == "BDRIP" {
            parsed.is_bd_disk = true;
            i += 1;
            continue;
        }

        if token == "DUAL" || token == "DUALAUDIO" {
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
            || token == "HLG"
        {
            parsed.detected_hdr = true;
            i += 1;
            continue;
        }

        if let Some(year) = parse_year(token) {
            // Prefer the latest year token to avoid treating numeric movie titles
            // (e.g. "1917.2019...") as the release year.
            parsed.year = Some(year);
        }

        if parsed.quality.is_none() {
            if let Some(quality) = parse_quality(token) {
                parsed.quality = Some(quality.to_string());
            }
        }

        if parsed.source.is_none() {
            if let Some(source) = parse_source(token, next) {
                parsed.source = Some(source.to_string());
            }
        }

        if parsed.video_codec.is_none() {
            let (codec, encoding) = parse_video(token);
            if codec.is_some() {
                parsed.video_codec = codec;
                parsed.video_encoding = encoding;
            }
        }

        if let Some(audio) = parse_audio(token, next) {
            let codec_value = audio.codec.to_string();
            if parsed.audio.is_none() {
                parsed.audio = Some(codec_value.clone());
            }
            parsed.audio_codecs.push(codec_value);

            if parsed.audio_channels.is_none() {
                if let Some(channels) = audio.channels.as_ref() {
                    parsed.audio_channels = Some(channels.to_string());
                }
            }
            if audio.channels.is_none()
                && matches!(audio.codec, "DDP" | "AAC" | "AC3" | "DTS" | "DTSHD")
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

    parsed.episode = parse_series_episode(raw_title);
    parsed.normalized_title = normalize_title_tokens(&tokens, &parsed.episode);
    if parsed.normalized_title.is_empty() {
        parsed.normalized_title = tokens
            .iter()
            .filter(|token| !is_noise_token(token))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
    }

    parsed.languages_audio = dedupe_keep_order(parsed.languages_audio);
    parsed.languages_subtitles = dedupe_keep_order(parsed.languages_subtitles);
    parsed.audio_codecs = dedupe_keep_order(parsed.audio_codecs);

    if parsed.languages_audio.is_empty()
        && default_dual_applied && parsed.languages_subtitles.is_empty() {
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

    parsed.parse_confidence = confidence.min(1.0);

    parsed
}

#[cfg(test)]
#[path = "release_parser_tests.rs"]
mod release_parser_tests;
