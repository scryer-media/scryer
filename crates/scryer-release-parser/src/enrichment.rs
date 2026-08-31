use crate::lex::Token;
use crate::model::{
    AudioCodec, ExternalIdSource, MetadataEnrichment, ParsedExternalId, ParsedReleaseMetadata,
    ReleaseParseCandidate, ReleaseSource, VideoCodec,
};
use crate::trash_guides;

#[derive(Clone, Copy)]
enum LanguageScope {
    Auto,
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAudio {
    codec: &'static str,
    channels: Option<String>,
}

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

const DUB_ONLY_MARKERS: &[&str] = &["DUB", "DUBS", "DUBBED"];

pub(crate) fn enrich_candidate(
    tokens: &[Token],
    candidate: &ReleaseParseCandidate,
    raw_input: &str,
) -> MetadataEnrichment {
    let scoped_tokens = metadata_tokens_for_candidate(tokens, candidate);
    let normalized_tokens = scoped_tokens
        .iter()
        .map(|token| token.normalized.clone())
        .collect::<Vec<_>>();

    let mut enrichment = MetadataEnrichment::default();
    let mut language_context = LanguageScope::Auto;
    let mut saw_bare_hdr = false;

    if let Some(tmdb_id) = parse_tmdb_id_from_tokens(&normalized_tokens) {
        enrichment.tmdb_id = Some(tmdb_id.to_string());
        enrichment.external_ids.push(ParsedExternalId {
            source: ExternalIdSource::Tmdb,
            value: tmdb_id.to_string(),
        });
    }

    let mut index = 0usize;
    while index < normalized_tokens.len() {
        let token = normalized_tokens[index].as_str();
        let next = normalized_tokens.get(index + 1).map(String::as_str);

        if matches!(token, "PROPER" | "REPACK") {
            enrichment.is_proper_upload = true;
            if token == "REPACK" {
                enrichment.is_repack = true;
            }
            index += 1;
            continue;
        }

        if let Some(version) = parse_anime_version(token) {
            enrichment.anime_version = Some(version);
            enrichment.is_proper_upload = true;
            index += 1;
            continue;
        }

        if matches!(token, "KORSUB" | "KORSUBS") {
            enrichment.is_hardcoded_subs = true;
            push_unique(&mut enrichment.languages_subtitles, "kor");
            language_context = LanguageScope::Subtitle;
            index += 1;
            continue;
        }

        if matches!(token, "HC" | "HARDCODED" | "HARDSUBBED" | "HARDSUB") {
            enrichment.is_hardcoded_subs = true;
            index += 1;
            continue;
        }

        if enrichment.edition.is_none()
            && let Some((edition, consumed)) = parse_edition_at(&normalized_tokens, index)
        {
            enrichment.edition = Some(edition);
            index += consumed;
            continue;
        }

        if matches!(
            token,
            "BD25" | "BD50" | "BD66" | "BD100" | "BDMV" | "BDISO" | "BRDISK"
        ) {
            enrichment.is_bd_disk = true;
            index += 1;
            continue;
        }

        if token == "COMPLETE"
            && (next.is_some_and(|value| matches!(value, "BLURAY" | "BLU"))
                || (next == Some("UHD")
                    && normalized_tokens
                        .get(index + 2)
                        .is_some_and(|value| matches!(value.as_str(), "BLURAY" | "BLU"))))
        {
            enrichment.is_bd_disk = true;
            index += 1;
            continue;
        }

        if token == "AI" && next == Some("ENHANCED") {
            enrichment.is_ai_enhanced = true;
            index += 2;
            continue;
        }

        if matches!(token, "AIENHANCED" | "RIFE") {
            enrichment.is_ai_enhanced = true;
            index += 1;
            continue;
        }

        if matches!(token, "DUAL" | "DUALAUDIO" | "DUAL-AUDIO") {
            enrichment.is_dual_audio = true;
            language_context = LanguageScope::Audio;
            index += 1;
            continue;
        }

        if is_10bit_token(token) {
            enrichment.is_10bit = true;
            index += 1;
            continue;
        }
        if token == "10" && next.is_some_and(|value| matches!(value, "BIT" | "BITS")) {
            enrichment.is_10bit = true;
            index += 2;
            continue;
        }

        if token == "UNCENSORED" {
            enrichment.is_uncensored = true;
            index += 1;
            continue;
        }

        if DUB_ONLY_MARKERS.contains(&token) {
            enrichment.is_dubs_only = true;
            // Keep scanning this token so markers like "DUB ENG" establish
            // audio-language context for the following language token.
        }

        if token == "ATMOS" {
            enrichment.is_atmos = true;
            index += 1;
            continue;
        }

        if token == "VOSTFR" {
            language_context = LanguageScope::Subtitle;
            push_unique(&mut enrichment.languages_subtitles, "fra");
            enrichment
                .parse_hints
                .push("enrichment:subtitle_vostfr".to_string());
            index += 1;
            continue;
        }

        if let Some(scope) = has_language_context_token(token) {
            language_context = scope;
            enrichment.parse_hints.push(format!(
                "enrichment:language_context={}",
                match scope {
                    LanguageScope::Auto => "auto",
                    LanguageScope::Audio => "audio",
                    LanguageScope::Subtitle => "subtitle",
                }
            ));
            index += 1;
            continue;
        }

        if let Some(language) = parse_language_hint(token) {
            // The context scan only sees markers *before* a language token
            // ("SUB ENG"), but the dominant anime order is language-first:
            // "[ENG-Sub]", "Eng Subs". A language read under no context whose
            // next token is a subtitle marker names the subtitle track, not an
            // audio track — claiming English audio here sends the release
            // through grab scoring as dubbed, and import's real audio probe
            // then terminally rejects every copy.
            let scope = match language_context {
                LanguageScope::Auto
                    if next.is_some_and(|value| LANG_SUBTITLE_MARKERS.contains(&value)) =>
                {
                    LanguageScope::Subtitle
                }
                scope => scope,
            };
            match scope {
                LanguageScope::Subtitle => {
                    push_unique(&mut enrichment.languages_subtitles, language)
                }
                LanguageScope::Audio | LanguageScope::Auto => {
                    push_unique(&mut enrichment.languages_audio, language)
                }
            }
            index += 1;
            continue;
        }

        language_context = LanguageScope::Auto;

        if matches!(token, "DOVI" | "DV") || (token == "DOLBY" && next == Some("VISION")) {
            // Dolby Vision alone (profile 5) has no fallback layer; an
            // explicit HDR10/HDR10+ token is what marks a non-DV-compatible
            // base. The quality gates rely on this distinction.
            enrichment.is_dolby_vision = true;
            enrichment.detected_hdr = true;
            index += usize::from(token == "DOLBY") + 1;
            continue;
        }

        if matches!(
            token,
            "HDR" | "HDR10" | "HDR10PLUS" | "HDR10+" | "HDR10P" | "HDRVIVID" | "HLG"
        ) {
            enrichment.detected_hdr = true;
            if token == "HDR" {
                saw_bare_hdr = true;
            }
            if token == "HDR10" {
                enrichment.has_hdr_fallback = true;
            }
            if matches!(token, "HDR10PLUS" | "HDR10+" | "HDR10P") {
                enrichment.is_hdr10plus = true;
                enrichment.has_hdr_fallback = true;
            }
            if token == "HLG" {
                enrichment.is_hlg = true;
            }
            index += 1;
            continue;
        }

        let (video_codec, video_encoding) = parse_video(token);
        if enrichment.video_codec.is_none() {
            enrichment.video_codec =
                video_codec.or_else(|| parse_split_video_codec(token, next).map(str::to_string));
        }
        if enrichment.video_encoding.is_none() {
            enrichment.video_encoding = video_encoding
                .or_else(|| parse_split_video_encoding(token, next).map(str::to_string));
        }

        if let Some((audio, consumed)) = parse_split_audio_at(&normalized_tokens, index) {
            record_audio(&mut enrichment, &audio);
            index += consumed;
            continue;
        }

        if let Some(audio) = parse_audio(token, next) {
            record_audio(&mut enrichment, &audio);
            fill_following_audio_channels(&mut enrichment, &normalized_tokens, index, audio.codec);
        }

        index += 1;
    }

    if let Some(version) = candidate
        .projected
        .episode
        .as_ref()
        .and_then(|episode| episode.raw.as_deref())
        .and_then(extract_trailing_version)
    {
        enrichment.anime_version.get_or_insert(version);
        enrichment.is_proper_upload = true;
    }

    // "DV HDR" / "DoVi HDR" advertises a Dolby Vision release with an HDR10
    // base layer, in either token order; a bare HDR token without DV carries
    // no fallback meaning.
    if saw_bare_hdr && enrichment.is_dolby_vision {
        enrichment.has_hdr_fallback = true;
    }

    enrichment.fps = parse_fps(raw_input);
    if enrichment.fps.is_some_and(|value| value > 120.0) {
        enrichment.is_ai_enhanced = true;
    }

    let trash_guide_signals = trash_guides::detect_token_signals(&normalized_tokens);
    enrichment.is_ai_enhanced |= trash_guide_signals.ai_enhanced;
    enrichment.is_proper_upload |= trash_guide_signals.proper;
    enrichment.is_repack |= trash_guide_signals.repack;
    enrichment.is_hardcoded_subs |= trash_guide_signals.hardcoded_subs;

    enrichment.languages_audio = dedupe_keep_order(enrichment.languages_audio);
    enrichment.languages_subtitles = dedupe_keep_order(enrichment.languages_subtitles);
    enrichment.audio_codecs = dedupe_keep_order(enrichment.audio_codecs);

    if enrichment.is_dual_audio || enrichment.languages_audio.len() > 1 {
        enrichment.is_dubs_only = false;
    } else if trash_guide_signals.dubs_only {
        enrichment.is_dubs_only = true;
    }

    enrichment.normalized_source = normalize_source_for_service(
        candidate
            .projected
            .source
            .as_ref()
            .map(ReleaseSource::as_str),
        candidate
            .projected
            .streaming_service
            .as_ref()
            .map(crate::model::StreamingService::as_str),
    );
    if enrichment.normalized_source.is_some() {
        enrichment
            .parse_hints
            .push("normalize:service_webrip_to_webdl".to_string());
    }

    enrichment
}

pub(crate) fn project_final_metadata(
    mut projected: ParsedReleaseMetadata,
    enrichment: &MetadataEnrichment,
) -> ParsedReleaseMetadata {
    projected.languages_audio = enrichment.languages_audio.clone();
    projected.languages_subtitles = enrichment.languages_subtitles.clone();

    for external_id in &enrichment.external_ids {
        if !projected.external_ids.iter().any(|existing| {
            existing.source == external_id.source
                && existing.value.eq_ignore_ascii_case(&external_id.value)
        }) {
            projected.external_ids.push(external_id.clone());
        }
    }

    if projected.tmdb_id.is_none() {
        projected.tmdb_id = enrichment.tmdb_id.clone();
    }

    if projected.video_encoding.is_none() {
        projected.video_encoding = enrichment.video_encoding.clone();
    }
    if projected.video_codec.is_none() {
        projected.video_codec = enrichment
            .video_codec
            .as_deref()
            .and_then(VideoCodec::parse);
    }
    if enrichment.audio.is_some() {
        projected.audio = enrichment.audio.as_deref().and_then(AudioCodec::parse);
    }
    if !enrichment.audio_codecs.is_empty() {
        projected.audio_codecs = enrichment
            .audio_codecs
            .iter()
            .filter_map(|codec| AudioCodec::parse(codec))
            .collect();
    } else if let Some(audio) = projected.audio {
        projected.audio_codecs = vec![audio];
    }
    if enrichment.audio_channels.is_some() {
        projected.audio_channels = enrichment.audio_channels.clone();
    }

    projected.is_dual_audio = enrichment.is_dual_audio;
    projected.is_atmos = enrichment.is_atmos;
    projected.is_dolby_vision = enrichment.is_dolby_vision;
    projected.detected_hdr = enrichment.detected_hdr;
    projected.has_hdr_fallback = enrichment.has_hdr_fallback;
    projected.is_hdr10plus = enrichment.is_hdr10plus;
    projected.is_hlg = enrichment.is_hlg;
    projected.is_10bit = enrichment.is_10bit;
    projected.fps = enrichment.fps;
    projected.is_proper_upload = enrichment.is_proper_upload;
    projected.is_repack = enrichment.is_repack;
    projected.is_bd_disk = enrichment.is_bd_disk;
    projected.is_ai_enhanced = enrichment.is_ai_enhanced;
    projected.is_hardcoded_subs = enrichment.is_hardcoded_subs;
    projected.is_uncensored = enrichment.is_uncensored;
    projected.is_dubs_only = enrichment.is_dubs_only;
    if enrichment.anime_version.is_some() {
        projected.anime_version = enrichment.anime_version;
    }

    if projected.is_remux
        && projected
            .edition
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("remux"))
    {
        projected.edition = None;
    }
    if projected.is_repack
        && projected
            .edition
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("repack"))
    {
        projected.edition = None;
    }
    if projected.is_uncensored
        && projected
            .edition
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("uncensored"))
    {
        projected.edition = None;
    }
    if enrichment.edition.is_some() {
        projected.edition = enrichment.edition.clone();
    }

    if let Some(normalized_source) = enrichment.normalized_source.clone() {
        projected.source = ReleaseSource::parse(&normalized_source);
    }

    for hint in &enrichment.parse_hints {
        if !projected
            .parse_hints
            .iter()
            .any(|existing| existing == hint)
        {
            projected.parse_hints.push(hint.clone());
        }
    }

    projected.missing_fields = collect_missing_fields(&projected);
    projected
}

pub(crate) fn collect_missing_fields(projected: &ParsedReleaseMetadata) -> Vec<String> {
    let mut missing_fields = Vec::new();
    if projected.quality.is_none() {
        missing_fields.push("quality".to_string());
    }
    if projected.source.is_none() {
        missing_fields.push("source".to_string());
    }
    if projected.video_codec.is_none() {
        missing_fields.push("video_codec".to_string());
    }
    if projected.audio.is_none() {
        missing_fields.push("audio".to_string());
    }
    if projected.year.is_none() {
        missing_fields.push("year".to_string());
    }
    missing_fields
}

fn metadata_tokens_for_candidate<'a>(
    tokens: &'a [Token],
    candidate: &ReleaseParseCandidate,
) -> Vec<&'a Token> {
    let first_title_token = candidate
        .zones
        .title_zones
        .iter()
        .map(|range| range.start_token)
        .min()
        .unwrap_or(0);
    let metadata_start = candidate
        .zones
        .metadata_zone
        .map(|range| range.start_token)
        .unwrap_or(tokens.len());
    let mut scoped_indices = Vec::new();
    scoped_indices.extend((0..first_title_token).filter(|index| {
        tokens
            .get(*index)
            .is_some_and(|token| is_prefix_metadata_token(token.normalized.as_str()))
    }));
    scoped_indices.extend((0..metadata_start).filter(|index| {
        !token_in_ranges(*index, candidate.zones.title_zones.as_slice())
            && tokens
                .get(*index)
                .is_some_and(|token| is_gap_metadata_token(token.normalized.as_str()))
    }));

    let Some(metadata_zone) = candidate.zones.metadata_zone else {
        scoped_indices.sort_unstable();
        scoped_indices.dedup();
        return scoped_indices
            .into_iter()
            .filter_map(|index| tokens.get(index))
            .collect();
    };
    scoped_indices.extend(
        (metadata_zone.start_token..metadata_zone.end_token).filter(|index| {
            !candidate
                .zones
                .release_group_span
                .is_some_and(|range| (*index >= range.start_token) && (*index < range.end_token))
        }),
    );
    scoped_indices.sort_unstable();
    scoped_indices.dedup();
    scoped_indices
        .into_iter()
        .filter_map(|index| tokens.get(index))
        .collect()
}

fn is_prefix_metadata_token(token: &str) -> bool {
    matches!(
        token,
        "BDISO" | "BDMV" | "BD25" | "BD50" | "BD66" | "BD100" | "BRDISK"
    )
}

fn is_gap_metadata_token(token: &str) -> bool {
    is_prefix_metadata_token(token)
        || matches!(
            token,
            "DUB"
                | "DUBBED"
                | "DUBS"
                | "DUAL"
                | "DUALAUDIO"
                | "ENGLISH"
                | "MULTI"
                | "MULTIAUDIO"
                | "MULTISUB"
                | "MULTISUBS"
                | "SUB"
                | "SUBS"
        )
}

fn token_in_ranges(index: usize, ranges: &[crate::model::TokenRange]) -> bool {
    ranges
        .iter()
        .any(|range| index >= range.start_token && index < range.end_token)
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

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn dedupe_keep_order(values: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        if !output
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
        {
            output.push(value);
        }
    }
    output
}

fn normalize_language_token(token: &str) -> Option<&'static str> {
    match token {
        "EN" | "ENG" | "ENGLISH" | "EN-GB" => Some("eng"),
        "JA" | "JP" | "JPN" | "JAP" | "JAPANESE" => Some("jpn"),
        "FR" | "FRA" | "FRE" | "FRENCH" | "TRUEFRENCH" | "VF" | "VF2" | "VFF" | "VFQ" => {
            Some("fra")
        }
        "DE" | "DEU" | "GER" | "GERMAN" | "SWISSGERMAN" => Some("deu"),
        "ES" | "SPA" | "ESP" | "SPANISH" | "ESPANOL" | "CASTELLANO" => Some("spa"),
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

fn parse_language_token_with_affixes(token: &str) -> Option<&'static str> {
    if let Some(language) = normalize_language_token(token) {
        return Some(language);
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
            if let Some(language) = normalize_language_token(tail) {
                return Some(language);
            }
        }
        if token.ends_with(affix) && token.len() > affix.len() {
            let head = &token[..token.len() - affix.len()];
            if let Some(language) = normalize_language_token(head) {
                return Some(language);
            }
        }
    }
    None
}

fn parse_language_hint(token: &str) -> Option<&'static str> {
    if token == "VOSTFR" {
        return Some("fra");
    }
    if token.ends_with("SUB") || token.ends_with("SUBS") || token.contains("VOST") {
        return None;
    }
    parse_language_token_with_affixes(token).or_else(|| normalize_language_token(token))
}

fn has_language_context_token(token: &str) -> Option<LanguageScope> {
    if token.starts_with("SUB") || token.starts_with("VOST") || token.contains("SUBS") {
        return Some(LanguageScope::Subtitle);
    }
    if LANG_AUDIO_MARKERS.contains(&token) {
        Some(LanguageScope::Audio)
    } else if LANG_SUBTITLE_MARKERS.contains(&token) || token.starts_with("MULTI") {
        Some(LanguageScope::Subtitle)
    } else {
        None
    }
}

fn parse_video(token: &str) -> (Option<String>, Option<String>) {
    let video_encoding = if token.contains("X264") {
        Some("x264".to_string())
    } else if token.contains("X265") {
        Some("x265".to_string())
    } else {
        None
    };
    let codec = if token.contains("H266") || token.contains("VVC") {
        Some("VVC".to_string())
    } else if token.contains("H265") || token.contains("HEVC") || token == "X265" {
        Some("H.265".to_string())
    } else if token.contains("H264") || token.contains("AVC") || token == "X264" {
        Some("H.264".to_string())
    } else if token == "AV1" {
        Some("AV1".to_string())
    } else if token == "VP9" {
        Some("VP9".to_string())
    } else if token == "VC1" {
        Some("VC1".to_string())
    } else if token == "MPEG2" {
        Some("MPEG-2".to_string())
    } else if token == "XVID" {
        Some("XVID".to_string())
    } else if token == "DIVX" {
        Some("DIVX".to_string())
    } else {
        None
    };
    (codec, video_encoding)
}

fn is_10bit_token(token: &str) -> bool {
    matches!(token, "10BIT" | "10BITS" | "HI10" | "HI10P")
        || token.ends_with("10BIT")
        || token.ends_with("10BITS")
}

fn parse_split_video_codec(token: &str, next: Option<&str>) -> Option<&'static str> {
    match (token, next) {
        ("H" | "X", Some("265")) => Some("H.265"),
        ("H" | "X", Some("264")) => Some("H.264"),
        ("H", Some("266")) => Some("VVC"),
        ("VC", Some("1")) => Some("VC1"),
        ("MPEG", Some("2")) => Some("MPEG-2"),
        _ => None,
    }
}

fn parse_split_video_encoding(token: &str, next: Option<&str>) -> Option<&'static str> {
    match (token, next) {
        ("X", Some("265")) => Some("x265"),
        ("X", Some("264")) => Some("x264"),
        _ => None,
    }
}

fn parse_channels(value: &str) -> Option<String> {
    let upper = value.to_ascii_uppercase();
    if matches!(upper.as_str(), "20" | "51" | "71") {
        return match upper.as_str() {
            "20" => Some("2.0".to_string()),
            "51" => Some("5.1".to_string()),
            "71" => Some("7.1".to_string()),
            _ => None,
        };
    }
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
    let segments = upper.split('.').collect::<Vec<_>>();
    if segments.len() >= 2
        && segments.iter().take(2).all(|segment| {
            !segment.is_empty()
                && segment.len() <= 2
                && segment.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Some(format!("{}.{}", segments[0], segments[1]));
    }
    None
}

fn record_audio(enrichment: &mut MetadataEnrichment, audio: &ParsedAudio) {
    if enrichment.audio.is_none() {
        enrichment.audio = Some(audio.codec.to_string());
    }
    push_unique(&mut enrichment.audio_codecs, audio.codec);
    if enrichment.audio_channels.is_none() {
        enrichment.audio_channels = audio.channels.clone();
    }
}

fn fill_following_audio_channels(
    enrichment: &mut MetadataEnrichment,
    tokens: &[String],
    index: usize,
    codec: &str,
) {
    if enrichment.audio_channels.is_some()
        || !matches!(
            codec,
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
                | "OPUS"
                | "VORBIS"
                | "MP3"
        )
    {
        return;
    }

    for offset in 1..=3 {
        let Some(candidate_token) = tokens.get(index + offset) else {
            break;
        };
        if let Some(channels) = parse_channels(candidate_token) {
            enrichment.audio_channels = Some(channels);
            break;
        }
        if let Some(left) = tokens.get(index + offset)
            && let Some(right) = tokens.get(index + offset + 1)
            && is_digit_str(left)
            && is_digit_str(right)
        {
            enrichment.audio_channels = Some(format!("{left}.{right}"));
            break;
        }
    }
}

fn parse_split_audio_at(tokens: &[String], index: usize) -> Option<(ParsedAudio, usize)> {
    let token = tokens.get(index)?.as_str();
    let next = tokens.get(index + 1).map(String::as_str);
    let third = tokens.get(index + 2).map(String::as_str);

    if token == "DTS" && next == Some("HD") {
        let (codec, channel_start, base_consumed) = if third == Some("MA") {
            ("DTSMA", index + 3, 3)
        } else if third.is_some_and(|value| value.starts_with("MA")) {
            ("DTSMA", index + 2, 3)
        } else {
            ("DTSHD", index + 2, 2)
        };
        let (channels, channel_consumed) = parse_channels_from_tokens(tokens, channel_start);
        return Some((
            ParsedAudio { codec, channels },
            base_consumed + channel_consumed,
        ));
    }

    if token == "DTS" && next == Some("X") {
        let (channels, channel_consumed) = parse_channels_from_tokens(tokens, index + 2);
        return Some((
            ParsedAudio {
                codec: "DTSX",
                channels,
            },
            2 + channel_consumed,
        ));
    }

    if token == "DTS" && next.is_some_and(|value| value.starts_with("HD")) {
        let (channels, channel_consumed) = parse_channels_from_tokens(tokens, index + 1);
        return Some((
            ParsedAudio {
                codec: "DTSHD",
                channels,
            },
            2 + channel_consumed.saturating_sub(1),
        ));
    }

    if token == "AC" && next == Some("3") {
        return Some((
            ParsedAudio {
                codec: "AC3",
                channels: None,
            },
            2,
        ));
    }

    if token == "E" && next == Some("AC") && third == Some("3") {
        let (channels, channel_consumed) = parse_channels_from_tokens(tokens, index + 3);
        return Some((
            ParsedAudio {
                codec: "EAC3",
                channels,
            },
            3 + channel_consumed,
        ));
    }

    if (token == "EAC" || token == "EC") && next == Some("3") {
        let (channels, channel_consumed) = parse_channels_from_tokens(tokens, index + 2);
        return Some((
            ParsedAudio {
                codec: "EAC3",
                channels,
            },
            2 + channel_consumed,
        ));
    }

    None
}

fn parse_channels_from_tokens(tokens: &[String], index: usize) -> (Option<String>, usize) {
    let Some(token) = tokens.get(index) else {
        return (None, 0);
    };
    if let Some(channels) = parse_channels(token) {
        return (Some(channels), 1);
    }
    if let Some(left) = tokens.get(index)
        && let Some(right) = tokens.get(index + 1)
        && is_digit_str(left)
        && is_digit_str(right)
    {
        return (Some(format!("{left}.{right}")), 2);
    }
    (None, 0)
}

fn parse_audio(raw_token: &str, next: Option<&str>) -> Option<ParsedAudio> {
    let token = raw_token.trim().trim_start_matches('+');
    if token.is_empty() {
        return None;
    }
    if token.starts_with("DDP") || token.starts_with("DD+") {
        let suffix = token.trim_start_matches("DDP").trim_start_matches("DD+");
        return Some(ParsedAudio {
            codec: "DDP",
            channels: parse_channels(suffix)
                .or_else(|| parse_channels(token))
                .or_else(|| channel_pair_from_suffix(suffix, next))
                .or_else(|| next.and_then(parse_channels)),
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
    if token.starts_with("DD") {
        let suffix = token.trim_start_matches("DD");
        let channels = parse_channels(suffix).or_else(|| next.and_then(parse_channels));
        if token == "DD" || !suffix.is_empty() || channels.is_some() {
            return Some(ParsedAudio {
                codec: "DD",
                channels: channels.or_else(|| channel_pair_from_suffix(suffix, next)),
            });
        }
    }
    if token.starts_with("EAC3") || token.starts_with("EC3") || token.starts_with("E-AC-3") {
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
            channels: parse_channels(token)
                .or_else(|| channel_pair_from_codec_prefix("AAC", token, next))
                .or_else(|| next.and_then(parse_channels)),
        });
    }
    if token.starts_with("MP3") {
        return Some(ParsedAudio {
            codec: "MP3",
            channels: parse_channels(token).or_else(|| next.and_then(parse_channels)),
        });
    }
    if token.starts_with("VORBIS") {
        return Some(ParsedAudio {
            codec: "VORBIS",
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

fn channel_pair_from_suffix(suffix: &str, next: Option<&str>) -> Option<String> {
    match suffix {
        "20" => Some("2.0".to_string()),
        "51" => Some("5.1".to_string()),
        "71" => Some("7.1".to_string()),
        _ => (suffix.len() == 1
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
            && next.is_some_and(is_digit_str))
        .then(|| format!("{}.{}", suffix, next.unwrap_or_default())),
    }
}

fn channel_pair_from_codec_prefix(prefix: &str, token: &str, next: Option<&str>) -> Option<String> {
    let suffix = token.strip_prefix(prefix)?;
    channel_pair_from_suffix(suffix, next)
}

fn parse_fps(raw_title: &str) -> Option<f32> {
    // A number standing apart from the FPS word must be a standard frame rate;
    // otherwise ordinary numerals get claimed ("Top.10.FPS.Games" is not
    // 10fps). Fused forms ("60FPS") are explicit enough to accept any
    // plausible rate.
    const SEPARATED_FRAME_RATES: &[f32] = &[
        23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 72.0, 90.0, 100.0, 120.0, 144.0,
        165.0, 240.0, 300.0,
    ];
    let upper = raw_title.to_ascii_uppercase();
    let mut previous = None::<String>;
    let mut before_previous = None::<String>;
    for part in upper.split(|character: char| {
        character.is_ascii_whitespace()
            || matches!(
                character,
                '[' | ']' | '(' | ')' | '{' | '}' | '.' | '_' | '-'
            )
    }) {
        if part.is_empty() {
            continue;
        }
        if part == "FPS" {
            // Decimal rates split apart on the dot ("23.976" -> "23", "976"),
            // so try the rejoined pair before the single value.
            let rejoined = match (before_previous.as_deref(), previous.as_deref()) {
                (Some(int_part), Some(frac_part)) => {
                    format!("{int_part}.{frac_part}").parse::<f32>().ok()
                }
                _ => None,
            };
            let single = previous
                .as_deref()
                .and_then(|value| value.parse::<f32>().ok());
            for candidate in [rejoined, single].into_iter().flatten() {
                if SEPARATED_FRAME_RATES
                    .iter()
                    .any(|rate| (rate - candidate).abs() < 0.001)
                {
                    return Some(candidate);
                }
            }
            before_previous = None;
            previous = None;
            continue;
        }
        if let Some(value) = part.strip_suffix("FPS") {
            if let Ok(fps) = value.parse::<f32>()
                && (10.0..=300.0).contains(&fps)
            {
                return Some(fps);
            }
            before_previous = None;
            previous = None;
            continue;
        }
        if part.bytes().all(|byte| byte.is_ascii_digit()) {
            before_previous = previous.take();
            previous = Some(part.to_string());
        } else {
            before_previous = None;
            previous = None;
        }
    }
    None
}

fn parse_edition_at(tokens: &[String], index: usize) -> Option<(String, usize)> {
    let token = tokens.get(index)?.as_str();
    let next = tokens.get(index + 1).map(String::as_str);
    let third = tokens.get(index + 2).map(String::as_str);
    let fourth = tokens.get(index + 3).map(String::as_str);

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
        "DIRECTORS" | "DIRECTOR" if next == Some("CUT") => Some(("Director's Cut".to_string(), 2)),
        "SPECIAL" if next == Some("EDITION") && third == Some("REMASTERED") => {
            Some(("Special Edition Remastered".to_string(), 3))
        }
        "SPECIAL" if next == Some("EDITION") => Some(("Special Edition".to_string(), 2)),
        _ => None,
    }
}

fn parse_anime_version(token: &str) -> Option<u32> {
    if token.len() >= 2
        && token.starts_with('V')
        && let Ok(version) = token[1..].parse::<u32>()
        && (2..=9).contains(&version)
    {
        return Some(version);
    }
    if let Some(position) = token.find('V')
        && position > 0
        && token[..position]
            .chars()
            .all(|character| character.is_ascii_digit())
        && let Ok(version) = token[position + 1..].parse::<u32>()
        && (2..=9).contains(&version)
    {
        return Some(version);
    }
    None
}

fn extract_trailing_version(fragment: &str) -> Option<u32> {
    let upper = fragment.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let length = bytes.len();
    if length >= 2
        && bytes[length - 2] == b'V'
        && let digit @ b'2'..=b'9' = bytes[length - 1]
    {
        return Some(u32::from(digit - b'0'));
    }
    None
}

pub(crate) fn normalize_source_for_service(
    source: Option<&str>,
    service: Option<&str>,
) -> Option<String> {
    let source = source?;
    let service = service?.to_ascii_uppercase();
    if source.eq_ignore_ascii_case("WEBRip")
        && matches!(
            service.as_str(),
            "AMZN" | "AMAZON" | "CR" | "CRUNCHYROLL" | "DSNP" | "DISNEY+" | "NF" | "NETFLIX"
        )
    {
        return Some("WEB-DL".to_string());
    }
    None
}

fn is_digit_str(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}
