use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use chrono::NaiveDate;
use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

use crate::context::{ContextAlias, ContextEpisode, ContextFacetHint, ReleaseParseContext};
use crate::enrichment::{
    collect_missing_fields, enrich_candidate, normalize_source_for_service, project_final_metadata,
};
use crate::lex::{
    BracketKind, CstNode, LexedRelease, ReleaseCst, SeparatorKind, Token, lex_lossless,
    normalize_token,
};
use crate::model::{
    AudioCodec, CandidateZones, ContextTitleMatch, ContextTitleMatchKind, ExternalIdSource,
    MetadataAst, ParseDisposition, ParseFamily, ParseReason, ParsedEpisodeMetadata,
    ParsedEpisodeReleaseType, ParsedExternalId, ParsedReleaseMetadata, ParsedSpecialKind,
    ReleaseIdentity, ReleaseParseAnalysis, ReleaseParseCandidate, ReleaseSource, StreamingService,
    TitleSegment, TitleSegmentKind, TokenAnnotations, TokenRange, TokenRole, VideoCodec,
};

const BEAM_WIDTH: usize = 24;
const MAX_FINAL_CANDIDATES: usize = 16;
const ALT_ROLE_PRIMARY_DEBT: i32 = 0;
const ALT_ROLE_FIRST_DEBT: i32 = -3;
const ALT_ROLE_SECOND_DEBT: i32 = -6;
const TITLE_WORD_AMBIGUITY_DEBT: i32 = -8;
const MAX_ALIAS_BRANCH_FANOUT: usize = 3;
const ALIAS_AUTOMATON_CACHE_CAPACITY: usize = 64;
/// Version of the parser's score-bearing projection.
///
/// Consumers that persist a score derived from a parse fold this into their
/// provenance, so a parser change invalidates those stored scores instead of
/// letting them be compared against freshly-parsed ones.
pub const SCORING_MODEL_VERSION: u16 = 1;

pub(crate) struct AnalysisInputs<'a> {
    pub(crate) raw_input: &'a str,
    pub(crate) sanitized_input: &'a str,
    pub(crate) sanitize_hints: &'a [String],
    pub(crate) parser_version: &'static str,
    pub(crate) target: &'a ReleaseParseContext,
}

pub(crate) fn analyze_inputs(inputs: AnalysisInputs<'_>) -> ReleaseParseAnalysis {
    let category_hint = match inputs.target.facet_hint {
        ContextFacetHint::Movie => Some("movie"),
        ContextFacetHint::Series => Some("series"),
        ContextFacetHint::Anime => Some("anime"),
        ContextFacetHint::Unknown => None,
    };
    let facts = crate::trash_guides::derive_facts(inputs.raw_input, category_hint);
    let lexed = lex_lossless(inputs.sanitized_input);
    let annotations = annotate_tokens(&lexed.tokens);
    let context_index = build_context_index(inputs.target);
    let alias_oracle = build_alias_oracle(&lexed.tokens, &context_index);
    let mut parse_hints = inputs.sanitize_hints.to_vec();
    parse_hints.extend(lexed.hints.clone());
    parse_hints.extend(alias_oracle.parse_hints.iter().cloned());
    if annotations.iter().any(|annotation| annotation.role_pruned) {
        parse_hints.push("annotation:role_pruned".to_string());
    }

    let mut candidates = score_candidates(
        &lexed,
        &annotations,
        &context_index,
        &alias_oracle,
        inputs.raw_input,
        inputs.parser_version,
    );
    for candidate in &mut candidates {
        let mut candidate_facts = facts.clone();
        candidate_facts.extend(crate::trash_guides::derive_locale_group_facts(
            &candidate.projected,
            category_hint,
        ));
        candidate_facts.extend(crate::trash_guides::derive_structural_facts(
            &candidate.projected,
            category_hint,
        ));
        candidate_facts.sort();
        candidate_facts.dedup();
        candidate.projected.guide_facts = candidate_facts;
        crate::trash_guides::project_safe_facts(&mut candidate.projected);
    }
    let best_candidate_index = candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, candidate)| candidate.raw_score)
        .map(|(index, _)| index);
    let ambiguity_margin = best_candidate_index
        .map(|best_index| semantic_ambiguity_margin(&candidates, best_index))
        .unwrap_or_default();
    let disposition = if candidates.is_empty() {
        ParseDisposition::Unparseable
    } else if ambiguity_margin < 8 {
        ParseDisposition::Ambiguous
    } else {
        ParseDisposition::Parsed
    };
    let is_ambiguous = matches!(disposition, ParseDisposition::Ambiguous);
    // Zones an ambiguous winner may safely treat as title text: the union over
    // every contender close enough to have caused the ambiguity. Tokens outside
    // this union are release metadata under *every* competing interpretation,
    // so extracting structure-independent facts from them cannot leak a title
    // word (issue #170).
    let ambiguous_title_zone_union = (is_ambiguous && best_candidate_index.is_some()).then(|| {
        let best_score = best_candidate_index
            .and_then(|index| candidates.get(index))
            .map(|candidate| candidate.raw_score)
            .unwrap_or_default();
        candidates
            .iter()
            .filter(|candidate| candidate.raw_score >= best_score.saturating_sub(8))
            .flat_map(|candidate| candidate.zones.title_zones.iter().copied())
            .collect::<Vec<_>>()
    });

    if let Some(best_index) = best_candidate_index
        && let Some(best_candidate) = candidates.get_mut(best_index)
    {
        match disposition {
            ParseDisposition::Parsed => {
                let enrichment = enrich_candidate(&lexed.tokens, best_candidate, inputs.raw_input);
                best_candidate.projected =
                    project_final_metadata(best_candidate.projected.clone(), &enrichment);
                best_candidate.enrichment = Some(enrichment);
            }
            ParseDisposition::Ambiguous => {
                // Title, season and episode assignment stay unresolved on an
                // ambiguous winner, but the token-derived facts — languages,
                // codecs, HDR, proper/repack — read the same under every
                // competing interpretation. Enrich over the tokens outside the
                // contenders' combined title zones so a required-language rule
                // can still see `iTALiAN` on a parse whose numbering is
                // ambiguous (issue #170).
                if let Some(title_zone_union) = ambiguous_title_zone_union.as_ref() {
                    let mut scoped = best_candidate.clone();
                    scoped.zones.title_zones = title_zone_union.clone();
                    let enrichment = enrich_candidate(&lexed.tokens, &scoped, inputs.raw_input);
                    best_candidate.projected =
                        project_final_metadata(best_candidate.projected.clone(), &enrichment);
                    best_candidate.enrichment = Some(enrichment);
                }
                if let Some(normalized_source) = normalize_source_for_service(
                    best_candidate
                        .projected
                        .source
                        .as_ref()
                        .map(ReleaseSource::as_str),
                    best_candidate
                        .projected
                        .streaming_service
                        .as_ref()
                        .map(StreamingService::as_str),
                ) {
                    best_candidate.projected.source = ReleaseSource::parse(&normalized_source);
                    best_candidate
                        .projected
                        .parse_hints
                        .push("normalize:service_webrip_to_webdl".to_string());
                }
                best_candidate.projected.missing_fields =
                    collect_missing_fields(&best_candidate.projected);
                best_candidate
                    .projected
                    .parse_hints
                    .push("enrichment:ambiguous_structure_independent".to_string());
            }
            ParseDisposition::Unparseable => {
                if let Some(normalized_source) = normalize_source_for_service(
                    best_candidate
                        .projected
                        .source
                        .as_ref()
                        .map(ReleaseSource::as_str),
                    best_candidate
                        .projected
                        .streaming_service
                        .as_ref()
                        .map(StreamingService::as_str),
                ) {
                    best_candidate.projected.source = ReleaseSource::parse(&normalized_source);
                    best_candidate
                        .projected
                        .parse_hints
                        .push("normalize:service_webrip_to_webdl".to_string());
                }
                best_candidate.projected.missing_fields =
                    collect_missing_fields(&best_candidate.projected);
                best_candidate
                    .projected
                    .parse_hints
                    .push("enrichment_skipped_unparseable".to_string());
            }
        }

        best_candidate.projected.ambiguity_margin = ambiguity_margin;
        best_candidate.projected.is_ambiguous = is_ambiguous;
        best_candidate.projected.disposition = disposition;
        best_candidate.projected.scoring_model_version = SCORING_MODEL_VERSION;
    }

    let analysis_facts = best_candidate_index
        .and_then(|index| candidates.get(index))
        .map(|candidate| candidate.projected.guide_facts.clone())
        .unwrap_or(facts);

    ReleaseParseAnalysis {
        raw_input: inputs.raw_input.to_string(),
        sanitized_input: inputs.sanitized_input.to_string(),
        guide_facts: analysis_facts,
        parse_hints,
        tokens: lexed.tokens,
        annotations,
        cst: lexed.cst,
        candidates,
        best_candidate_index,
        parser_version: inputs.parser_version,
        scoring_model_version: SCORING_MODEL_VERSION,
        ambiguity_margin,
        is_ambiguous,
        disposition,
    }
}

#[derive(Clone, Debug)]
struct AliasEntry {
    tokens: Vec<String>,
    raw: String,
    code: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AliasEvidenceKind {
    CanonicalTitle,
    TitleAlias,
    EpisodeTitle,
}

impl AliasEvidenceKind {
    fn code(self) -> &'static str {
        match self {
            Self::CanonicalTitle => "context:title_canonical_hit",
            Self::TitleAlias => "context:title_alias_hit",
            Self::EpisodeTitle => "context:episode_title_hit",
        }
    }

    fn score_weight(self) -> i32 {
        match self {
            Self::CanonicalTitle => 12,
            Self::TitleAlias => 15,
            Self::EpisodeTitle => 10,
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Self::CanonicalTitle => 0,
            Self::TitleAlias => 1,
            Self::EpisodeTitle => 2,
        }
    }
}

#[derive(Clone, Debug)]
struct AliasPattern {
    text: String,
    kind: AliasEvidenceKind,
    raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AliasHit {
    token_range: TokenRange,
    pattern_id: usize,
    evidence: AliasEvidenceKind,
    score_weight: i32,
}

type AliasHitList = SmallVec<[AliasHit; MAX_ALIAS_BRANCH_FANOUT]>;

#[derive(Clone, Debug, Default)]
struct AliasOracle {
    patterns: Vec<AliasPattern>,
    hits_at: Vec<AliasHitList>,
    parse_hints: Vec<String>,
}

struct AliasAutomatonCacheEntry {
    key: Vec<String>,
    automaton: Arc<AhoCorasick>,
}

#[derive(Clone, Debug, Default)]
struct TokenByteMap {
    start_to_token: Vec<(usize, usize)>,
    end_to_token: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, Default)]
struct ContextIndex {
    facet_hint: ContextFacetHint,
    aliases: Vec<AliasEntry>,
    episode_titles: Vec<AliasEntry>,
    years: Vec<i32>,
    air_dates: Vec<NaiveDate>,
    absolute_numbers: Vec<u32>,
    episodes: Vec<EpisodeContextHint>,
}

#[derive(Clone, Copy, Debug, Default)]
struct EpisodeContextHint {
    season: Option<u32>,
    episode: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum ParsePhase {
    #[default]
    Prefix,
    Title,
    Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnitKind {
    Token,
    BracketGroup(BracketKind),
    HyphenGroup,
    SlashGroup,
    DelimitedRun(SeparatorKind),
}

#[derive(Clone, Debug)]
struct ParseUnit {
    token_range: TokenRange,
    kind: UnitKind,
    raw: String,
    normalized_tokens: Vec<String>,
    has_strong_anchor: bool,
    has_metadata_role: bool,
    has_title_like_token: bool,
    is_grouped: bool,
}

type ParseUnitList = SmallVec<[ParseUnit; 6]>;
type ParseUnitIndex = Vec<ParseUnitList>;
type TitleTokenIndices = SmallVec<[usize; 16]>;
type EvidenceList = SmallVec<[String; 4]>;
type AcceptedAliasHits = SmallVec<[AliasHit; 4]>;

#[derive(Clone, Debug, Default)]
struct RoleCandidate {
    role: TokenRole,
    confidence: u8,
    strong_anchor: bool,
}

#[derive(Clone, Debug)]
struct ParseState {
    cursor: usize,
    phase: ParsePhase,
    family: ParseFamily,
    title_token_indices: TitleTokenIndices,
    title_token_mask: FixedBitSet,
    identity: ReleaseIdentity,
    metadata: MetadataAst,
    metadata_token_mask: FixedBitSet,
    release_group: Option<String>,
    consumed_tokens: FixedBitSet,
    score: i32,
    reasons: Vec<ParseReason>,
    raw_evidence: EvidenceList,
    context_evidence: EvidenceList,
    accepted_alias_hits: AcceptedAliasHits,
}

impl ParseState {
    fn seeded(token_count: usize, family: ParseFamily, score: i32) -> Self {
        Self {
            cursor: 0,
            phase: ParsePhase::Prefix,
            family,
            title_token_indices: SmallVec::new(),
            title_token_mask: FixedBitSet::with_capacity(token_count),
            identity: ReleaseIdentity::Unknown,
            metadata: MetadataAst::default(),
            metadata_token_mask: FixedBitSet::with_capacity(token_count),
            release_group: None,
            consumed_tokens: FixedBitSet::with_capacity(token_count),
            score,
            reasons: Vec::new(),
            raw_evidence: SmallVec::new(),
            context_evidence: SmallVec::new(),
            accepted_alias_hits: SmallVec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BeamKey {
    cursor: usize,
    phase: u8,
    family: u8,
    identity: IdentityShapeKey,
    metadata_mask: u8,
    title_signature: TitleTokenIndices,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum IdentityShapeKey {
    Movie,
    Standard {
        season: Option<u32>,
        episode_numbers: SmallVec<[u32; 4]>,
    },
    Daily {
        air_date: NaiveDate,
        part: Option<u32>,
    },
    Absolute {
        absolute_episode_numbers: SmallVec<[u32; 4]>,
        version: Option<u32>,
        season_hint: Option<u32>,
    },
    SeasonPack {
        seasons: SmallVec<[u32; 4]>,
        is_partial: bool,
        season_part: Option<u32>,
    },
    RangePack {
        season: Option<u32>,
        range_start: u32,
        range_end: u32,
    },
    Special {
        kind: u8,
        season_hint: Option<u32>,
        episode_hint: Option<u32>,
    },
    Unknown,
}

#[derive(Clone, Debug, Default)]
struct CompoundMetadata {
    quality: Option<String>,
    source: Option<&'static str>,
    video_codec: Option<&'static str>,
    audio_codec: Option<&'static str>,
    audio_channels: Option<&'static str>,
}

fn annotate_tokens(tokens: &[Token]) -> Vec<TokenAnnotations> {
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let mut roles = classify_token(token, tokens.get(index + 1));
            roles.sort_by(|left, right| {
                right
                    .confidence
                    .cmp(&left.confidence)
                    .then(left.role.cmp(&right.role))
            });
            let role_pruned = roles.len() > 3;
            roles.truncate(3);
            let primary_role = roles
                .first()
                .map(|role| role.role)
                .unwrap_or(TokenRole::TitleWord);
            let alternate_roles = roles
                .iter()
                .skip(1)
                .map(|role| role.role)
                .collect::<smallvec::SmallVec<[TokenRole; 2]>>();
            let strong_anchor = roles.first().is_some_and(|role| role.strong_anchor);
            let may_be_title_word = !strong_anchor && is_title_like_token(token);
            TokenAnnotations {
                primary_role,
                alternate_roles,
                may_be_title_word,
                role_confidence: roles.first().map(|role| role.confidence).unwrap_or(0),
                role_pruned,
            }
        })
        .collect()
}

fn classify_token(token: &Token, next: Option<&Token>) -> Vec<RoleCandidate> {
    let mut roles = Vec::new();
    let normalized = token.normalized.as_str();
    let compound = detect_compound_metadata(normalized);

    if parse_year(normalized).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::Year,
            confidence: 95,
            strong_anchor: false,
        });
    }

    if parse_resolution_quality_token(normalized).is_some() || compound.quality.is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::Quality,
            confidence: 100,
            strong_anchor: true,
        });
    }
    if matches!(
        normalized,
        "WEB"
            | "WEBDL"
            | "WEBRIP"
            | "BDMV"
            | "BDISO"
            | "BRDISK"
            | "BLURAY"
            | "BD"
            | "BDRIP"
            | "BDRIO"
            | "BRRIP"
            | "BDREMUX"
            | "BLU"
            | "DVD"
            | "HDTV"
            | "CAM"
            | "HQCAM"
            | "CAMRIP"
            | "HDCAM"
            | "TELESYNC"
            | "TELECINE"
            | "DVDSCR"
            | "WORKPRINT"
            | "DVDRIP"
    ) || compound.source.is_some()
    {
        roles.push(RoleCandidate {
            role: TokenRole::Source,
            confidence: 98,
            strong_anchor: true,
        });
    }
    // Detection is the distilled TRaSH table plus the curated supplement, not a
    // hand-maintained list: the generated table is the source of
    // truth. WEB-adjacent aliases need the neighbor, which is why this site --
    // and only this site -- resolves them with context.
    if normalize_streaming_service_with_neighbor(normalized, next).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::StreamingService,
            confidence: 96,
            strong_anchor: false,
        });
    }
    if matches!(
        normalized,
        "H264"
            | "H265"
            | "H266"
            | "X264"
            | "X265"
            | "AVC"
            | "AVC1"
            | "HEVC"
            | "AV1"
            | "VP9"
            | "VVC"
            | "VC1"
            | "MPEG2"
            | "XVID"
            | "DIVX"
    ) || compound.video_codec.is_some()
    {
        roles.push(RoleCandidate {
            role: TokenRole::VideoCodec,
            confidence: 100,
            strong_anchor: true,
        });
    }
    if matches!(
        normalized,
        "AAC"
            | "DDP"
            | "DD"
            | "AC3"
            | "EAC3"
            | "EC3"
            | "TRUEHD"
            | "FLAC"
            | "DTS"
            | "DTSHD"
            | "DTSMA"
            | "DTSX"
            | "ATMOS"
            | "OPUS"
            | "VORBIS"
            | "MP3"
            | "PCM"
            | "LPCM"
    ) || compound.audio_codec.is_some()
    {
        roles.push(RoleCandidate {
            role: TokenRole::AudioCodec,
            confidence: 95,
            strong_anchor: true,
        });
    }
    if matches!(normalized, "20" | "51" | "71" | "2CH")
        || matches!(normalized, "5.1" | "7.1" | "2.0")
        || compound.audio_channels.is_some()
    {
        roles.push(RoleCandidate {
            role: TokenRole::AudioChannels,
            confidence: 88,
            strong_anchor: false,
        });
    }
    if matches!(
        normalized,
        "MULTI" | "MULTISUB" | "MULTISUBS" | "SUB" | "SUBS" | "DUB" | "DUAL"
    ) || is_explicit_language_metadata_token(normalized)
    {
        roles.push(RoleCandidate {
            role: TokenRole::Language,
            confidence: 80,
            strong_anchor: false,
        });
    }
    if matches!(
        normalized,
        "PROPER" | "REPACK" | "REMUX" | "UNCENSORED" | "UNCUT" | "EXTENDED" | "DIRECTORS"
    ) {
        roles.push(RoleCandidate {
            role: TokenRole::Edition,
            confidence: 87,
            strong_anchor: false,
        });
    }
    if is_release_flag_metadata_token(normalized) {
        roles.push(RoleCandidate {
            role: TokenRole::ReleaseFlag,
            confidence: 86,
            strong_anchor: false,
        });
    }
    if matches!(
        normalized,
        "OVA" | "OAD" | "NCOP" | "NCED" | "SPECIAL" | "EXTRA"
    ) {
        roles.push(RoleCandidate {
            role: TokenRole::SpecialMarker,
            confidence: 96,
            strong_anchor: false,
        });
    }
    if parse_version(normalized).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::VersionMarker,
            confidence: 92,
            strong_anchor: false,
        });
    }
    if parse_external_id_token(normalized).is_some() || is_external_id_label(normalized) {
        roles.push(RoleCandidate {
            role: TokenRole::ExternalId,
            confidence: if is_external_id_label(normalized) {
                88
            } else {
                100
            },
            strong_anchor: !is_external_id_label(normalized),
        });
    }
    // Bracketed 8-char tokens are hashes by scene convention; fused air dates
    // ("20260105") only appear unbracketed, so an all-decimal valid date in
    // brackets stays a checksum.
    let fused_date = token.bracket_depth == 0 && parse_fused_daily_date(normalized).is_some();
    if is_checksum(normalized) && !fused_date {
        roles.push(RoleCandidate {
            role: TokenRole::ChecksumOrHash,
            confidence: 100,
            strong_anchor: true,
        });
    }
    if fused_date {
        roles.push(RoleCandidate {
            role: TokenRole::DateMarker,
            confidence: 96,
            strong_anchor: false,
        });
    }
    if parse_standard_episode_token(normalized).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::EpisodeMarker,
            confidence: 100,
            strong_anchor: false,
        });
    }
    if parse_season_token(normalized).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::SeasonMarker,
            confidence: 94,
            strong_anchor: false,
        });
    }
    if parse_numeric_token(normalized).is_some() {
        roles.push(RoleCandidate {
            role: TokenRole::AbsoluteEpisodeMarker,
            confidence: 55,
            strong_anchor: false,
        });
    }
    if matches!(normalized, "COMPLETE" | "PACK" | "BATCH" | "PART") {
        roles.push(RoleCandidate {
            role: TokenRole::PackMarker,
            confidence: 88,
            strong_anchor: false,
        });
    }
    if matches!(normalized, "READNFO" | "RARBG" | "WWW") {
        roles.push(RoleCandidate {
            role: TokenRole::Noise,
            confidence: 85,
            strong_anchor: false,
        });
    }
    if token.separator_before == SeparatorKind::Hyphen && token.raw.len() <= 16 {
        roles.push(RoleCandidate {
            role: TokenRole::ReleaseGroupCandidate,
            confidence: 72,
            strong_anchor: false,
        });
    }
    if roles.is_empty() {
        roles.push(RoleCandidate {
            role: TokenRole::TitleWord,
            confidence: 40,
            strong_anchor: false,
        });
    }
    roles
}

fn is_explicit_language_metadata_token(token: &str) -> bool {
    if is_language_metadata_atom(token) {
        return true;
    }

    const AFFIXES: &[&str] = &[
        "AUDIO",
        "CC",
        "DUB",
        "DUBBED",
        "DUBS",
        "FORCED",
        "SUB",
        "SUBBED",
        "SUBS",
        "SUBTITLE",
        "SUBTITLES",
    ];
    for affix in AFFIXES {
        if let Some(head) = token.strip_suffix(affix)
            && !head.is_empty()
            && is_language_metadata_atom(head)
        {
            return true;
        }
        if let Some(tail) = token.strip_prefix(affix)
            && !tail.is_empty()
            && is_language_metadata_atom(tail)
        {
            return true;
        }
    }

    false
}

fn is_language_metadata_atom(token: &str) -> bool {
    matches!(
        token,
        "AR" | "ARA"
            | "BG"
            | "BUL"
            | "BULGARIAN"
            | "CA"
            | "CAT"
            | "CATALAN"
            | "CES"
            | "CZECH"
            | "DAN"
            | "DANISH"
            | "DEU"
            | "DUTCH"
            | "EN"
            | "ENG"
            | "ENGLISH"
            | "ESP"
            | "FRA"
            | "FRE"
            | "FRENCH"
            | "FR"
            | "GER"
            | "GERMAN"
            | "HEB"
            | "HEBREW"
            | "HIN"
            | "HINDI"
            | "HUN"
            | "HUNGARIAN"
            | "ICELANDIC"
            | "ISL"
            | "ITA"
            | "ITALIAN"
            | "JAP"
            | "JAPANESE"
            | "JP"
            | "JPN"
            | "KAT"
            | "KOR"
            | "KOREAN"
            | "KORSUB"
            | "KORSUBS"
            | "LAV"
            | "LATVIAN"
            | "LIT"
            | "LITHUANIAN"
            | "NLD"
            | "NOR"
            | "NORWEGIAN"
            | "POL"
            | "POLISH"
            | "POR"
            | "PORTUGUESE"
            | "PTBR"
            | "RON"
            | "ROMANIAN"
            | "RUM"
            | "RUS"
            | "RUSSIAN"
            | "SPA"
            | "SPANISH"
            | "SWE"
            | "SWEDISH"
            | "THA"
            | "THAI"
            | "TR"
            | "TRUEFRENCH"
            | "TUR"
            | "TURKISH"
            | "VFF"
            | "VFQ"
            | "VOSTFR"
            | "ZHO"
            | "CHINESE"
    )
}

fn is_release_flag_metadata_token(token: &str) -> bool {
    matches!(
        token,
        "10BIT"
            | "10BITS"
            | "DOVI"
            | "DV"
            | "HDR"
            | "HDR10"
            | "HDR10+"
            | "HDR10P"
            | "HDR10PLUS"
            | "HLG"
            | "HI10"
            | "HI10P"
    ) || token.ends_with("10BIT")
        || token.ends_with("10BITS")
}

fn score_candidates(
    lexed: &LexedRelease,
    annotations: &[TokenAnnotations],
    context_index: &ContextIndex,
    alias_oracle: &AliasOracle,
    raw_input: &str,
    parser_version: &'static str,
) -> Vec<ReleaseParseCandidate> {
    let unit_index = build_parse_unit_index(lexed.tokens.as_slice(), &lexed.cst, annotations);
    let mut beam = seed_beam(context_index, lexed.tokens.len());
    let mut completed = Vec::<ParseState>::new();

    for _ in 0..(lexed.tokens.len().saturating_mul(3).max(1)) {
        if beam.is_empty() {
            break;
        }
        let mut next = Vec::<ParseState>::new();
        let mut progressed = false;
        for state in std::mem::take(&mut beam) {
            if state.cursor >= lexed.tokens.len() {
                completed.push(state);
                continue;
            }
            let expanded = expand_state(
                &state,
                &unit_index,
                lexed.tokens.as_slice(),
                annotations,
                context_index,
                alias_oracle,
            );
            if expanded.is_empty() {
                let mut stalled = state.clone();
                stalled.cursor = lexed.tokens.len();
                completed.push(stalled);
                continue;
            }
            progressed = true;
            next.extend(expanded);
        }
        if !progressed {
            break;
        }
        beam = prune_beam(next);
    }
    completed.extend(beam.into_iter().map(|mut state| {
        state.cursor = lexed.tokens.len();
        state
    }));

    let mut candidates = completed
        .into_iter()
        .map(|mut state| {
            apply_context(
                &mut state,
                lexed.tokens.as_slice(),
                context_index,
                alias_oracle,
            );
            build_candidate(
                state,
                lexed.tokens.as_slice(),
                annotations,
                raw_input,
                parser_version,
                context_index,
                alias_oracle,
            )
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.raw_score));
    candidates.truncate(MAX_FINAL_CANDIDATES);
    candidates
}

/// Identity evidence codes whose season/episode numbering came from the search
/// context rather than the release name. Extend this list when a new identity
/// family starts inferring; every entry is penalized by
/// [`INFERRED_IDENTITY_PENALTY`] and excluded from manufacturing ambiguity
/// against an explicitly parsed rival.
const INFERRED_IDENTITY_EVIDENCE: &[&str] = &["context:season_episode_hint"];
const INFERRED_IDENTITY_PENALTY: i32 = 12;
const INFERRED_IDENTITY_PENALTY_REASON: &str = "identity:inferred_from_context";

fn candidate_identity_is_inferred(candidate: &ReleaseParseCandidate) -> bool {
    candidate
        .reasons
        .iter()
        .any(|reason| reason.code == INFERRED_IDENTITY_PENALTY_REASON)
}

fn semantic_ambiguity_margin(candidates: &[ReleaseParseCandidate], best_index: usize) -> i32 {
    let Some(best_candidate) = candidates.get(best_index) else {
        return 0;
    };
    let best_signature = candidate_ambiguity_signature(best_candidate);
    let best_is_inferred = candidate_identity_is_inferred(best_candidate);
    let second_best = candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != best_index)
        // A context-inferred candidate is a fallback reading, not a competing
        // interpretation of the name; it never renders an explicit winner
        // ambiguous (issue #170). An inferred winner still contends with
        // everything.
        .filter(|(_, candidate)| best_is_inferred || !candidate_identity_is_inferred(candidate))
        .filter(|(_, candidate)| candidate_ambiguity_signature(candidate) != best_signature)
        .map(|(_, candidate)| candidate.raw_score)
        .max();

    second_best.map_or(i32::MAX, |score| {
        best_candidate.raw_score.saturating_sub(score)
    })
}

fn candidate_ambiguity_signature(candidate: &ReleaseParseCandidate) -> String {
    let episode_signature = candidate
        .projected
        .episode
        .as_ref()
        .map(|episode| {
            format!(
                "{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}",
                episode.season,
                episode.episode_numbers,
                episode.absolute_episode,
                episode.absolute_episode_numbers,
                episode.special_absolute_episode_numbers,
                episode.air_date,
                episode.full_season,
                episode.is_partial_season,
                episode.is_multi_season,
                episode.release_type
            )
        })
        .unwrap_or_else(|| "none".to_string());

    format!(
        "{}|{:?}|{:?}|{:?}|{:?}|{}",
        candidate.projected.normalized_title,
        candidate.projected.year,
        candidate.projected.quality,
        candidate.projected.source,
        candidate.projected.video_codec,
        episode_signature
    )
}

fn build_parse_unit_index(
    tokens: &[Token],
    cst: &ReleaseCst,
    annotations: &[TokenAnnotations],
) -> ParseUnitIndex {
    let mut units_by_start = vec![ParseUnitList::new(); tokens.len()];
    for index in 0..tokens.len() {
        push_unit(
            &mut units_by_start,
            build_unit(
                tokens,
                annotations,
                TokenRange::new(index, index + 1),
                UnitKind::Token,
            ),
        );
    }

    for node in &cst.nodes {
        match node {
            CstNode::BracketGroup {
                bracket_kind,
                token_indices,
                ..
            } => {
                if let Some(token_range) = node_span(token_indices) {
                    push_unit(
                        &mut units_by_start,
                        build_unit(
                            tokens,
                            annotations,
                            token_range,
                            UnitKind::BracketGroup(*bracket_kind),
                        ),
                    );
                }
            }
            CstNode::HyphenGroup { token_indices } => {
                if let Some(token_range) = node_span(token_indices)
                    && token_indices.len() <= 4
                {
                    push_unit(
                        &mut units_by_start,
                        build_unit(tokens, annotations, token_range, UnitKind::HyphenGroup),
                    );
                }
            }
            CstNode::SlashGroup { token_indices } => {
                if let Some(token_range) = node_span(token_indices)
                    && token_indices.len() <= 4
                {
                    push_unit(
                        &mut units_by_start,
                        build_unit(tokens, annotations, token_range, UnitKind::SlashGroup),
                    );
                }
            }
            CstNode::DelimitedRun {
                separator,
                token_indices,
            } => {
                if let Some(token_range) = node_span(token_indices)
                    && token_indices.len() <= 5
                    && matches!(
                        separator,
                        SeparatorKind::Dot
                            | SeparatorKind::Underscore
                            | SeparatorKind::Space
                            | SeparatorKind::Hyphen
                            | SeparatorKind::Slash
                    )
                {
                    push_unit(
                        &mut units_by_start,
                        build_unit(
                            tokens,
                            annotations,
                            token_range,
                            UnitKind::DelimitedRun(*separator),
                        ),
                    );
                }
            }
            CstNode::Token { .. } => {}
        }
    }

    for units in &mut units_by_start {
        units.sort_by(|left, right| {
            let left_len = left.token_range.len();
            let right_len = right.token_range.len();
            right_len
                .cmp(&left_len)
                .then(unit_priority(left.kind).cmp(&unit_priority(right.kind)))
        });
        units.dedup_by(|left, right| {
            left.token_range == right.token_range && left.kind == right.kind
        });
    }

    units_by_start
}

fn push_unit(units_by_start: &mut ParseUnitIndex, unit: ParseUnit) {
    if let Some(units) = units_by_start.get_mut(unit.token_range.start_token) {
        units.push(unit);
    }
}

fn node_span(token_indices: &[usize]) -> Option<TokenRange> {
    let start_token = token_indices.iter().min().copied()?;
    let end_token = token_indices.iter().max().map(|value| value + 1)?;
    Some(TokenRange::new(start_token, end_token))
}

fn unit_priority(kind: UnitKind) -> u8 {
    match kind {
        UnitKind::BracketGroup(_) => 0,
        UnitKind::HyphenGroup => 1,
        UnitKind::SlashGroup => 2,
        UnitKind::DelimitedRun(_) => 3,
        UnitKind::Token => 4,
    }
}

fn build_unit(
    tokens: &[Token],
    annotations: &[TokenAnnotations],
    token_range: TokenRange,
    kind: UnitKind,
) -> ParseUnit {
    let raw = token_range
        .indices()
        .into_iter()
        .filter_map(|index| tokens.get(index))
        .map(|token| token.raw.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let normalized_tokens = token_range
        .indices()
        .into_iter()
        .filter_map(|index| tokens.get(index))
        .map(|token| token.normalized.clone())
        .collect::<Vec<_>>();
    let has_strong_anchor = token_range.indices().into_iter().any(|index| {
        annotations
            .get(index)
            .is_some_and(|annotation| is_strong_anchor(annotation.primary_role))
    });
    let has_metadata_role = token_range.indices().into_iter().any(|index| {
        annotations.get(index).is_some_and(|annotation| {
            matches!(
                annotation.primary_role,
                TokenRole::Year
                    | TokenRole::Quality
                    | TokenRole::Source
                    | TokenRole::StreamingService
                    | TokenRole::VideoCodec
                    | TokenRole::AudioCodec
                    | TokenRole::AudioChannels
                    | TokenRole::Language
                    | TokenRole::Edition
                    | TokenRole::ReleaseFlag
                    | TokenRole::ExternalId
            )
        })
    });
    let has_title_like_token = token_range
        .indices()
        .into_iter()
        .any(|index| tokens.get(index).is_some_and(is_title_like_token));
    ParseUnit {
        token_range,
        kind,
        raw,
        normalized_tokens,
        has_strong_anchor,
        has_metadata_role,
        has_title_like_token,
        is_grouped: !matches!(kind, UnitKind::Token),
    }
}

fn seed_beam(context: &ContextIndex, token_count: usize) -> Vec<ParseState> {
    seeded_families(context)
        .into_iter()
        .map(|(family, seed_adjustment)| {
            ParseState::seeded(
                token_count,
                family,
                family_seed_score(family) + seed_adjustment,
            )
        })
        .collect()
}

fn seeded_families(context: &ContextIndex) -> Vec<(ParseFamily, i32)> {
    match context.facet_hint {
        ContextFacetHint::Movie => {
            let mut families = vec![(ParseFamily::Movie, 0)];
            if !context.episodes.is_empty()
                || !context.air_dates.is_empty()
                || !context.absolute_numbers.is_empty()
            {
                families.extend([
                    (ParseFamily::AnimeAbsolute, -8),
                    (ParseFamily::DailyEpisode, -8),
                    (ParseFamily::StandardEpisode, -10),
                    (ParseFamily::EpisodeRangePack, -12),
                    (ParseFamily::Special, -12),
                ]);
            }
            families
        }
        ContextFacetHint::Series => {
            let mut families = vec![
                (ParseFamily::StandardEpisode, 0),
                (ParseFamily::DailyEpisode, 0),
                (ParseFamily::SeasonPack, 0),
                (ParseFamily::EpisodeRangePack, 0),
                (ParseFamily::Special, 0),
            ];
            if !context.absolute_numbers.is_empty() {
                families.push((ParseFamily::AnimeAbsolute, 0));
            }
            families
        }
        ContextFacetHint::Anime => vec![
            (ParseFamily::AnimeAbsolute, 0),
            (ParseFamily::StandardEpisode, 0),
            (ParseFamily::EpisodeRangePack, 0),
            (ParseFamily::SeasonPack, 0),
            (ParseFamily::Special, 0),
        ],
        ContextFacetHint::Unknown => vec![
            (ParseFamily::Movie, 0),
            (ParseFamily::StandardEpisode, 0),
            (ParseFamily::DailyEpisode, 0),
            (ParseFamily::AnimeAbsolute, 0),
            (ParseFamily::SeasonPack, 0),
            (ParseFamily::EpisodeRangePack, 0),
            (ParseFamily::Special, 0),
        ],
    }
}

fn family_seed_score(family: ParseFamily) -> i32 {
    match family {
        ParseFamily::Movie => 10,
        ParseFamily::DailyEpisode => 16,
        ParseFamily::StandardEpisode => 16,
        ParseFamily::AnimeAbsolute => 16,
        ParseFamily::SeasonPack => 14,
        ParseFamily::EpisodeRangePack => 14,
        ParseFamily::Special => 12,
        ParseFamily::Unknown => 0,
    }
}

fn expand_state(
    state: &ParseState,
    units_by_start: &ParseUnitIndex,
    tokens: &[Token],
    annotations: &[TokenAnnotations],
    context: &ContextIndex,
    alias_oracle: &AliasOracle,
) -> Vec<ParseState> {
    let Some(units) = units_by_start.get(state.cursor) else {
        return Vec::new();
    };
    let mut next = Vec::new();
    let protected_context_hits = protected_context_hits_at_cursor(state, alias_oracle, annotations);

    if let Some(hits) = alias_oracle.hits_at.get(state.cursor) {
        for hit in hits {
            if alias_hit_allowed(state, hit, annotations) {
                next.push(branch_alias_hit(state, hit, alias_oracle, tokens));
            }
        }
    }

    if state.phase == ParsePhase::Metadata {
        for hit in &protected_context_hits {
            next.push(branch_context_phrase(state, hit, alias_oracle, tokens));
        }
    }

    for unit in units {
        let unit_overlaps_protected_context =
            unit_overlaps_protected_context(unit, protected_context_hits.as_slice())
                || unit_overlaps_accepted_title_tokens(state, unit);

        if !has_identity(&state.identity)
            && identity_branch_allowed(state)
            && let Some(identity_state) = branch_identity(state, unit, tokens, annotations, context)
        {
            next.push(identity_state);
        }

        if unit_is_title_like(unit, context)
            && state.phase != ParsePhase::Metadata
            && !unit_is_metadata_like(unit)
            && !unit_is_explicit_identity_unit(unit, annotations, context)
            && !unit_is_prefix_release_group_candidate(state, unit, tokens, context)
            && !unit_is_foreign_alt_title_group(unit, context)
        {
            next.push(branch_title(state, unit, context));
        }

        if (unit_is_metadata_like(unit) || state.phase == ParsePhase::Metadata)
            && !unit_overlaps_protected_context
        {
            next.push(branch_metadata(state, unit, tokens, annotations));
        }

        if matches!(state.phase, ParsePhase::Prefix | ParsePhase::Metadata)
            && state.release_group.is_none()
            && unit_can_be_release_group(unit, tokens)
        {
            next.push(branch_release_group(state, unit, tokens));
        }
    }

    if state.phase == ParsePhase::Prefix
        && let Some(token_unit) = units
            .iter()
            .find(|unit| matches!(unit.kind, UnitKind::Token))
        && is_skip_candidate(token_unit)
    {
        let mut skipped = state.clone();
        skipped.cursor += 1;
        skipped.score -= 2;
        skipped
            .reasons
            .push(reason("beam:skip_prefix", -2, Some(token_unit.raw.clone())));
        next.push(skipped);
    }

    next
}

fn identity_branch_allowed(state: &ParseState) -> bool {
    state.phase != ParsePhase::Metadata
        || metadata_allows_late_identity(&state.metadata)
        || matches!(state.family, ParseFamily::EpisodeRangePack)
}

fn metadata_allows_late_identity(metadata: &MetadataAst) -> bool {
    metadata.source.is_none()
        && metadata.quality.is_none()
        && metadata.video_codec.is_none()
        && metadata.audio_codec.is_none()
        && metadata.audio_channels.is_none()
        && metadata.streaming_service.is_none()
        && metadata.edition.is_none()
        && metadata.external_ids.is_empty()
}

fn alias_hit_allowed(state: &ParseState, hit: &AliasHit, annotations: &[TokenAnnotations]) -> bool {
    if state.phase == ParsePhase::Metadata {
        return false;
    }
    if context_hit_is_protected(hit, state, annotations) {
        return true;
    }
    !(hit.token_range.start_token..hit.token_range.end_token).any(|index| {
        annotations.get(index).is_some_and(|annotation| {
            matches!(
                annotation.primary_role,
                TokenRole::Quality
                    | TokenRole::VideoCodec
                    | TokenRole::AudioCodec
                    | TokenRole::AudioChannels
                    | TokenRole::ExternalId
                    | TokenRole::ChecksumOrHash
            )
        })
    })
}

fn protected_context_hits_at_cursor(
    state: &ParseState,
    alias_oracle: &AliasOracle,
    annotations: &[TokenAnnotations],
) -> Vec<AliasHit> {
    alias_oracle
        .hits_at
        .get(state.cursor)
        .into_iter()
        .flat_map(|hits| hits.iter())
        .filter(|hit| context_hit_is_protected(hit, state, annotations))
        .cloned()
        .collect()
}

fn context_hit_is_protected(
    hit: &AliasHit,
    state: &ParseState,
    annotations: &[TokenAnnotations],
) -> bool {
    if hit.token_range.len() > 1 || range_is_accepted_title_span(state, hit.token_range) {
        return true;
    }
    !(hit.token_range.start_token..hit.token_range.end_token).any(|index| {
        annotations
            .get(index)
            .is_some_and(annotation_has_source_quality_or_codec_role)
    })
}

fn annotation_has_source_quality_or_codec_role(annotation: &TokenAnnotations) -> bool {
    source_quality_or_codec_role(annotation.primary_role)
        || annotation
            .alternate_roles
            .iter()
            .any(|role| source_quality_or_codec_role(*role))
}

fn source_quality_or_codec_role(role: TokenRole) -> bool {
    matches!(
        role,
        TokenRole::Quality
            | TokenRole::Source
            | TokenRole::VideoCodec
            | TokenRole::AudioCodec
            | TokenRole::AudioChannels
    )
}

fn range_is_accepted_title_span(state: &ParseState, range: TokenRange) -> bool {
    (range.start_token..range.end_token).all(|index| state.title_token_mask.contains(index))
}

fn unit_overlaps_accepted_title_tokens(state: &ParseState, unit: &ParseUnit) -> bool {
    (unit.token_range.start_token..unit.token_range.end_token)
        .any(|index| state.title_token_mask.contains(index))
}

fn unit_overlaps_protected_context(unit: &ParseUnit, protected_hits: &[AliasHit]) -> bool {
    protected_hits
        .iter()
        .any(|hit| token_ranges_overlap(unit.token_range, hit.token_range))
}

fn token_ranges_overlap(left: TokenRange, right: TokenRange) -> bool {
    left.start_token < right.end_token && right.start_token < left.end_token
}

fn branch_alias_hit(
    state: &ParseState,
    hit: &AliasHit,
    alias_oracle: &AliasOracle,
    tokens: &[Token],
) -> ParseState {
    let mut next = state.clone();
    next.phase = ParsePhase::Title;
    next.cursor = hit.token_range.end_token;
    next.title_token_indices
        .extend(hit.token_range.start_token..hit.token_range.end_token);
    for token_index in hit.token_range.start_token..hit.token_range.end_token {
        next.title_token_mask.insert(token_index);
        next.consumed_tokens.insert(token_index);
    }
    next.accepted_alias_hits.push(hit.clone());
    let detail = alias_oracle
        .patterns
        .get(hit.pattern_id)
        .map(|pattern| pattern.raw.clone())
        .unwrap_or_else(|| {
            render_token_indices(
                tokens,
                &(hit.token_range.start_token..hit.token_range.end_token).collect::<Vec<_>>(),
            )
        });
    let delta = 10 + hit.score_weight;
    next.score += delta;
    next.context_evidence.push(hit.evidence.code().to_string());
    next.reasons
        .push(reason("beam:alias_hit", delta, Some(detail)));
    next
}

fn branch_context_phrase(
    state: &ParseState,
    hit: &AliasHit,
    alias_oracle: &AliasOracle,
    tokens: &[Token],
) -> ParseState {
    let mut next = state.clone();
    next.phase = ParsePhase::Metadata;
    next.cursor = hit.token_range.end_token;
    for token_index in hit.token_range.start_token..hit.token_range.end_token {
        next.consumed_tokens.insert(token_index);
    }
    let detail = alias_oracle
        .patterns
        .get(hit.pattern_id)
        .map(|pattern| pattern.raw.clone())
        .unwrap_or_else(|| {
            render_token_indices(
                tokens,
                &(hit.token_range.start_token..hit.token_range.end_token).collect::<Vec<_>>(),
            )
        });
    let delta = hit.score_weight;
    next.score += delta;
    next.context_evidence.push(hit.evidence.code().to_string());
    next.reasons
        .push(reason(hit.evidence.code(), delta, Some(detail)));
    next
}

fn unit_is_prefix_release_group_candidate(
    state: &ParseState,
    unit: &ParseUnit,
    tokens: &[Token],
    context: &ContextIndex,
) -> bool {
    if state.phase != ParsePhase::Prefix || unit_alias_bonus(unit, context) > 0 {
        return false;
    }
    match unit.kind {
        UnitKind::BracketGroup(_) => unit_can_be_release_group(unit, tokens),
        UnitKind::Token => tokens
            .get(unit.token_range.start_token)
            .is_some_and(|token| token.group_id.is_some() && token.bracket_depth > 0),
        _ => false,
    }
}

fn unit_is_foreign_alt_title_group(unit: &ParseUnit, context: &ContextIndex) -> bool {
    matches!(unit.kind, UnitKind::BracketGroup(BracketKind::Paren))
        && unit_alias_bonus(unit, context) == 0
        && unit
            .raw
            .chars()
            .any(|ch| !ch.is_ascii() && ch.is_alphabetic())
}

fn parse_identity_for_unit(
    family: ParseFamily,
    unit: &ParseUnit,
    tokens: &[Token],
    context: &ContextIndex,
) -> Option<(usize, ReleaseIdentity, usize, i32, &'static str)> {
    parse_identity_at(family, tokens, unit.token_range.start_token, context)
        .map(|(identity, last_token, family_bonus, evidence)| {
            (
                unit.token_range.start_token,
                identity,
                last_token,
                family_bonus,
                evidence,
            )
        })
        .or_else(|| {
            (family == ParseFamily::DailyEpisode
                && unit.token_range.end_token > unit.token_range.start_token + 1)
                .then(|| {
                    ((unit.token_range.start_token + 1)..unit.token_range.end_token).find_map(
                        |index| {
                            parse_identity_at(family, tokens, index, context).map(
                                |(identity, last_token, family_bonus, evidence)| {
                                    (index, identity, last_token, family_bonus, evidence)
                                },
                            )
                        },
                    )
                })?
        })
}

fn branch_identity(
    state: &ParseState,
    unit: &ParseUnit,
    tokens: &[Token],
    annotations: &[TokenAnnotations],
    context: &ContextIndex,
) -> Option<ParseState> {
    let (identity_start, identity, last_token, family_bonus, evidence) =
        parse_identity_for_unit(state.family, unit, tokens, context)?;
    let mut next = state.clone();
    next.phase = ParsePhase::Metadata;
    next.identity = identity;
    next.score += family_bonus;
    next.cursor = last_token + 1;
    next.raw_evidence.push(evidence.to_string());
    next.reasons.push(reason(evidence, family_bonus, None));
    if INFERRED_IDENTITY_EVIDENCE.contains(&evidence) {
        // Identity inferred from the search context, not read out of the name.
        // Explicit release evidence must always outrank inferred numbering —
        // a bare `S01` season pack is what the name *says*, while the episode
        // number here is only what the search *asked for* — so context-only
        // identity carries a confidence penalty large enough that a competing
        // explicit parse clears the ambiguity margin. When inference is the
        // only plausible reading, nothing explicit competes and the penalized
        // candidate still wins (issue #170).
        next.score -= INFERRED_IDENTITY_PENALTY;
        next.reasons.push(reason(
            INFERRED_IDENTITY_PENALTY_REASON,
            -INFERRED_IDENTITY_PENALTY,
            None,
        ));
    }
    for token_index in identity_start..=last_token {
        next.consumed_tokens.insert(token_index);
        if let Some(annotation) = annotations.get(token_index) {
            let expected_role = match next.family {
                ParseFamily::DailyEpisode => TokenRole::DateMarker,
                ParseFamily::SeasonPack => TokenRole::PackMarker,
                ParseFamily::EpisodeRangePack | ParseFamily::AnimeAbsolute => {
                    TokenRole::AbsoluteEpisodeMarker
                }
                ParseFamily::Special => TokenRole::SpecialMarker,
                ParseFamily::StandardEpisode => {
                    if token_index == unit.token_range.start_token {
                        TokenRole::EpisodeMarker
                    } else {
                        TokenRole::SeasonMarker
                    }
                }
                ParseFamily::Movie | ParseFamily::Unknown => TokenRole::TitleWord,
            };
            apply_role_usage_bonus(&mut next, Some(annotation), expected_role);
        }
        if next.metadata.quality.is_none()
            && matches!(next.family, ParseFamily::StandardEpisode)
            && let Some(token) = tokens.get(token_index)
            && let Some(quality) = trailing_quality_suffix(token.normalized.as_str())
        {
            next.metadata.quality = Some(quality);
            next.metadata.quality_span = Some(TokenRange {
                start_token: token_index,
                end_token: token_index + 1,
            });
            record_metadata_token(&mut next, token_index);
            next.reasons
                .push(reason("metadata:fused_episode_quality", 6, None));
        }
    }
    Some(next)
}

fn branch_title(state: &ParseState, unit: &ParseUnit, context: &ContextIndex) -> ParseState {
    let mut next = state.clone();
    next.phase = ParsePhase::Title;
    next.cursor = unit.token_range.end_token;
    for token_index in unit.token_range.start_token..unit.token_range.end_token {
        next.title_token_indices.push(token_index);
        next.title_token_mask.insert(token_index);
    }
    let delta = unit_title_delta(unit) + unit_alias_bonus(unit, context);
    next.score += delta;
    next.reasons
        .push(reason("beam:title_unit", delta, Some(unit.raw.clone())));
    next
}

fn branch_metadata(
    state: &ParseState,
    unit: &ParseUnit,
    tokens: &[Token],
    annotations: &[TokenAnnotations],
) -> ParseState {
    let mut next = state.clone();
    next.phase = ParsePhase::Metadata;
    next.cursor = unit.token_range.end_token;
    consume_unit_metadata(&mut next, unit, tokens, annotations);
    next
}

fn branch_release_group(state: &ParseState, unit: &ParseUnit, tokens: &[Token]) -> ParseState {
    let mut next = state.clone();
    let range = release_group_token_range(unit, tokens);
    next.cursor = range.end_token;
    next.release_group = Some(render_token_range_preserving_separators(tokens, range));
    next.score += 8;
    next.reasons
        .push(reason("beam:release_group", 8, Some(unit.raw.clone())));
    for token_index in range.start_token..range.end_token {
        next.consumed_tokens.insert(token_index);
    }
    next
}

fn prune_beam(states: Vec<ParseState>) -> Vec<ParseState> {
    let mut deduped = BTreeMap::<BeamKey, ParseState>::new();
    for state in states {
        let key = beam_key(&state);
        match deduped.get(&key) {
            Some(existing) if existing.score >= state.score => {}
            _ => {
                deduped.insert(key, state);
            }
        }
    }
    let mut retained = deduped.into_values().collect::<Vec<_>>();
    retained.sort_by_key(|state| std::cmp::Reverse(state.score));
    retained.truncate(BEAM_WIDTH);
    let top_score = retained
        .first()
        .map(|state| state.score)
        .unwrap_or_default();
    retained.retain(|state| state.score >= top_score - 20);
    retained
}

fn beam_key(state: &ParseState) -> BeamKey {
    let metadata_mask = [
        state.metadata.year.is_some(),
        state.metadata.quality.is_some(),
        state.metadata.source.is_some(),
        state.metadata.video_codec.is_some(),
        state.metadata.audio_codec.is_some(),
    ]
    .iter()
    .enumerate()
    .fold(
        0u8,
        |mask, (index, value)| {
            if *value { mask | (1u8 << index) } else { mask }
        },
    );
    BeamKey {
        cursor: state.cursor,
        phase: state.phase as u8,
        family: state.family as u8,
        identity: identity_shape_key(&state.identity),
        metadata_mask,
        title_signature: state.title_token_indices.clone(),
    }
}

fn special_kind_key(kind: ParsedSpecialKind) -> u8 {
    match kind {
        ParsedSpecialKind::Special => 0,
        ParsedSpecialKind::Ova => 1,
        ParsedSpecialKind::Oad => 2,
        ParsedSpecialKind::Ncop => 3,
        ParsedSpecialKind::Nced => 4,
        ParsedSpecialKind::Extra => 5,
    }
}

fn identity_shape_key(identity: &ReleaseIdentity) -> IdentityShapeKey {
    match identity {
        ReleaseIdentity::MovieIdentity => IdentityShapeKey::Movie,
        ReleaseIdentity::StandardEpisodeIdentity {
            season,
            episode_numbers,
        } => IdentityShapeKey::Standard {
            season: *season,
            episode_numbers: episode_numbers.iter().copied().collect(),
        },
        ReleaseIdentity::DailyIdentity { air_date, part } => IdentityShapeKey::Daily {
            air_date: *air_date,
            part: *part,
        },
        ReleaseIdentity::AbsoluteIdentity {
            absolute_episode_numbers,
            version,
            season_hint,
        } => IdentityShapeKey::Absolute {
            absolute_episode_numbers: absolute_episode_numbers.iter().copied().collect(),
            version: *version,
            season_hint: *season_hint,
        },
        ReleaseIdentity::SeasonPackIdentity {
            seasons,
            is_partial,
            season_part,
            ..
        } => IdentityShapeKey::SeasonPack {
            seasons: seasons.iter().copied().collect(),
            is_partial: *is_partial,
            season_part: *season_part,
        },
        ReleaseIdentity::RangePackIdentity {
            season,
            range_start,
            range_end,
        } => IdentityShapeKey::RangePack {
            season: *season,
            range_start: *range_start,
            range_end: *range_end,
        },
        ReleaseIdentity::SpecialIdentity {
            special_kind,
            season_hint,
            episode_hint,
        } => IdentityShapeKey::Special {
            kind: special_kind_key(*special_kind),
            season_hint: *season_hint,
            episode_hint: *episode_hint,
        },
        ReleaseIdentity::Unknown => IdentityShapeKey::Unknown,
    }
}

fn has_identity(identity: &ReleaseIdentity) -> bool {
    !matches!(identity, ReleaseIdentity::Unknown)
}

fn unit_is_title_like(unit: &ParseUnit, context: &ContextIndex) -> bool {
    unit.has_title_like_token
        || unit_alias_bonus(unit, context) > 0
        || context.aliases.iter().any(|alias| {
            alias
                .tokens
                .windows(unit.normalized_tokens.len())
                .any(|window| window == unit.normalized_tokens)
        })
}

fn unit_is_explicit_identity_unit(
    unit: &ParseUnit,
    annotations: &[TokenAnnotations],
    context: &ContextIndex,
) -> bool {
    if unit_alias_bonus(unit, context) > 0 {
        return false;
    }
    (unit.token_range.start_token..unit.token_range.end_token).any(|index| {
        let Some(annotation) = annotations.get(index) else {
            return false;
        };
        matches!(
            annotation.primary_role,
            TokenRole::EpisodeMarker
                | TokenRole::SeasonMarker
                | TokenRole::DateMarker
                | TokenRole::PackMarker
                | TokenRole::SpecialMarker
        ) || matches!(annotation.primary_role, TokenRole::AbsoluteEpisodeMarker)
            && context.facet_hint != ContextFacetHint::Movie
    })
}

fn unit_is_metadata_like(unit: &ParseUnit) -> bool {
    unit.has_strong_anchor
        || unit.has_metadata_role
        || unit
            .normalized_tokens
            .iter()
            .any(|token| is_compound_metadata_like_token(token.as_str()))
}

fn is_compound_metadata_like_token(token: &str) -> bool {
    let compound = detect_compound_metadata(token);
    (token.len() >= 6 || token.chars().any(|ch| ch.is_ascii_digit()))
        && (compound.quality.is_some()
            || compound.source.is_some()
            || compound.video_codec.is_some()
            || compound.audio_codec.is_some()
            || compound.audio_channels.is_some())
}

fn unit_can_be_release_group(unit: &ParseUnit, tokens: &[Token]) -> bool {
    match unit.kind {
        UnitKind::Token => tokens
            .get(unit.token_range.start_token)
            .is_some_and(|token| {
                token.separator_before == SeparatorKind::Hyphen
                    && token.raw.len() <= 24
                    && release_group_part_is_valid(token, false)
                    && !is_compound_metadata_suffix(tokens, unit.token_range.start_token)
            }),
        UnitKind::HyphenGroup => {
            let span_len = unit
                .token_range
                .end_token
                .saturating_sub(unit.token_range.start_token);
            span_len > 0
                && span_len <= 3
                && tokens
                    .get(unit.token_range.start_token)
                    .is_some_and(|token| token.separator_before == SeparatorKind::Hyphen)
                && (unit.token_range.start_token..unit.token_range.end_token).all(|index| {
                    tokens.get(index).is_some_and(|token| {
                        release_group_part_is_valid(token, index > unit.token_range.start_token)
                    })
                })
        }
        UnitKind::BracketGroup(_) => {
            let span_len = unit
                .token_range
                .end_token
                .saturating_sub(unit.token_range.start_token);
            span_len > 0
                && span_len <= 3
                && (unit.token_range.start_token..unit.token_range.end_token).all(|index| {
                    tokens.get(index).is_some_and(|token| {
                        !token.normalized.is_empty()
                            && parse_year(token.normalized.as_str()).is_none()
                            && parse_numeric_token(token.normalized.as_str()).is_none()
                            && !matches!(
                                token.normalized.as_str(),
                                "WEB"
                                    | "WEBDL"
                                    | "WEBRIP"
                                    | "BLURAY"
                                    | "BDRIP"
                                    | "BRRIP"
                                    | "BDREMUX"
                                    | "HDTV"
                                    | "DVDRIP"
                                    | "AAC"
                                    | "DDP"
                                    | "EAC3"
                                    | "EC3"
                                    | "AC3"
                                    | "AVC"
                                    | "HEVC"
                                    | "H264"
                                    | "H265"
                                    | "H266"
                                    | "VVC"
                            )
                    })
                })
        }
        _ => false,
    }
}

fn release_group_token_range(unit: &ParseUnit, tokens: &[Token]) -> TokenRange {
    let mut range = unit.token_range;

    if unit.kind != UnitKind::Token {
        return range;
    }

    while range.end_token < tokens.len()
        && range.end_token.saturating_sub(range.start_token) < 4
        && tokens.get(range.end_token).is_some_and(|token| {
            matches!(
                token.separator_before,
                SeparatorKind::Dot | SeparatorKind::Hyphen | SeparatorKind::Underscore
            ) && release_group_part_is_valid(token, true)
        })
    {
        range.end_token += 1;
    }

    range
}

fn release_group_part_is_valid(token: &Token, continuation: bool) -> bool {
    let normalized = token.normalized.as_str();
    if normalized.is_empty() || token.raw.len() > 24 {
        return false;
    }
    if !continuation
        && !normalized
            .chars()
            .any(|character| character.is_ascii_alphabetic())
    {
        return false;
    }
    if continuation
        && !normalized
            .chars()
            .any(|character| character.is_ascii_alphanumeric())
    {
        return false;
    }
    if parse_year(normalized).is_some() && !continuation {
        return false;
    }
    !matches!(
        normalized,
        "WEB"
            | "DL"
            | "RIP"
            | "WEBDL"
            | "WEBRIP"
            | "BDISO"
            | "BDMV"
            | "BD25"
            | "BD50"
            | "BD66"
            | "BD100"
            | "BRDISK"
            | "BLURAY"
            | "BD"
            | "BDRIP"
            | "BRRIP"
            | "BDREMUX"
            | "HDTV"
            | "DVDRIP"
            | "DVD"
            | "AAC"
            | "DD"
            | "DDP"
            | "EAC3"
            | "AC3"
            | "DTS"
            | "DTSHD"
            | "DTSMA"
            | "DTSX"
            | "TRUEHD"
            | "OPUS"
            | "VORBIS"
            | "MP3"
            | "PCM"
            | "LPCM"
            | "AVC"
            | "HEVC"
            | "H264"
            | "H265"
            | "H266"
            | "X264"
            | "X265"
            | "XVID"
            | "VVC"
            | "VC1"
            | "MPEG2"
            | "MKV"
            | "MP4"
            | "AVI"
            | "SUB"
            | "SUBS"
            | "AUDIO"
            | "DUAL"
            | "DUALAUDIO"
            | "MULTI"
            | "MULTIAUDIO"
            | "MULTISUB"
            | "MULTISUBS"
            | "ENGLISH"
            | "FRENCH"
            | "GERMAN"
            | "ITALIAN"
            | "SPANISH"
            | "TURKCE"
            | "TURKISH"
            | "UNCUT"
            | "UNCENSORED"
            | "HQ"
            | "SET"
            | "BOX"
            | "BATCH"
            | "COMPLETE"
            | "SEASON"
            | "EP"
            | "EPS"
            | "EPISODE"
            | "EPISODES"
            | "PROPER"
            | "REPACK"
            | "REMUX"
    )
}

fn is_compound_metadata_suffix(tokens: &[Token], index: usize) -> bool {
    let Some(token) = tokens.get(index) else {
        return false;
    };
    let Some(previous) = index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
    else {
        return false;
    };
    token.separator_before == SeparatorKind::Hyphen
        && (matches!(
            (previous.normalized.as_str(), token.normalized.as_str()),
            ("WEB", "DL") | ("WEB", "RIP") | ("E", "AC") | ("DUAL", "AUDIO") | ("HD", "MA")
        ) || (previous.normalized == "DTS"
            && (token.normalized == "X" || token.normalized.starts_with("HD"))))
}

fn infer_leading_release_group(tokens: &[Token]) -> Option<String> {
    let first = tokens.first()?;
    let group_id = first.group_id?;
    if first.bracket_depth == 0 {
        return None;
    }

    let group_indices = tokens
        .iter()
        .enumerate()
        .take_while(|(_, token)| token.group_id == Some(group_id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if group_indices.is_empty() || group_indices.len() > 3 {
        return None;
    }

    let start_token = *group_indices.first()?;
    let end_token = group_indices.last().map(|index| index + 1)?;
    if !(start_token..end_token).all(|index| {
        tokens
            .get(index)
            .is_some_and(|token| release_group_part_is_valid(token, index > start_token))
    }) {
        return None;
    }

    Some(render_token_range_preserving_separators(
        tokens,
        TokenRange {
            start_token,
            end_token,
        },
    ))
}

fn infer_release_group_for_candidate(state: &ParseState, tokens: &[Token]) -> Option<String> {
    let title_zones = contiguous_token_ranges(state.title_token_indices.as_slice());
    infer_leading_release_group(tokens)
        .or_else(|| infer_release_group_from_token_suffix(state, tokens))
        .or_else(|| infer_state_release_group(state, tokens, title_zones.as_slice()))
}

fn infer_release_group_from_token_suffix(state: &ParseState, tokens: &[Token]) -> Option<String> {
    let title_zones = contiguous_token_ranges(state.title_token_indices.as_slice());
    for index in (0..tokens.len()).rev() {
        let Some(token) = tokens.get(index) else {
            continue;
        };
        if token.separator_before != SeparatorKind::Hyphen
            || !release_group_part_is_valid(token, false)
            || is_compound_metadata_suffix(tokens, index)
        {
            continue;
        }
        let range = release_group_suffix_range(tokens, index);
        if !release_group_suffix_range_allowed(state, tokens, range, title_zones.as_slice()) {
            continue;
        }
        return Some(render_token_range_preserving_separators(tokens, range));
    }

    infer_terminal_release_group(state, tokens, title_zones.as_slice())
}

fn release_group_suffix_range_allowed(
    state: &ParseState,
    tokens: &[Token],
    range: TokenRange,
    title_zones: &[TokenRange],
) -> bool {
    if range.start_token >= range.end_token {
        return false;
    }
    if title_zones
        .iter()
        .any(|title_range| overlaps_range(*title_range, Some(range)))
    {
        return false;
    }
    if range_is_embedded_in_larger_group(tokens, range) {
        return false;
    }
    if range_is_metadata_marker(tokens, range) {
        return false;
    }
    if range_is_standalone_short_group(tokens, range) {
        return true;
    }
    has_release_group_suffix_context(state, tokens, range.start_token)
}

fn infer_state_release_group(
    state: &ParseState,
    tokens: &[Token],
    title_zones: &[TokenRange],
) -> Option<String> {
    let release_group = state.release_group.as_deref()?;
    let range = find_phrase_span(tokens, release_group)?;
    if title_zones
        .iter()
        .any(|title_range| overlaps_range(*title_range, Some(range)))
    {
        return None;
    }
    if range_is_metadata_marker(tokens, range) || range_is_embedded_in_larger_group(tokens, range) {
        return None;
    }
    Some(release_group.to_string())
}

fn range_is_standalone_short_group(tokens: &[Token], range: TokenRange) -> bool {
    let Some(group_id) = tokens
        .get(range.start_token)
        .and_then(|token| token.group_id)
    else {
        return false;
    };
    let group_indices = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| (token.group_id == Some(group_id)).then_some(index))
        .collect::<Vec<_>>();
    let Some(group_start) = group_indices.first().copied() else {
        return false;
    };
    let Some(group_end) = group_indices.last().map(|index| index + 1) else {
        return false;
    };
    group_start == range.start_token
        && group_end == range.end_token
        && range.end_token.saturating_sub(range.start_token) <= 3
}

fn range_is_embedded_in_larger_group(tokens: &[Token], range: TokenRange) -> bool {
    let Some(group_id) = tokens
        .get(range.start_token)
        .and_then(|token| token.group_id)
    else {
        return false;
    };
    let group_indices = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| (token.group_id == Some(group_id)).then_some(index))
        .collect::<Vec<_>>();
    let Some(group_start) = group_indices.first().copied() else {
        return false;
    };
    let Some(group_end) = group_indices.last().map(|index| index + 1) else {
        return false;
    };
    group_start != range.start_token || group_end != range.end_token
}

fn range_is_metadata_marker(tokens: &[Token], range: TokenRange) -> bool {
    let range_len = range.end_token.saturating_sub(range.start_token);
    (range.start_token..range.end_token).any(|index| {
        tokens.get(index).is_some_and(|token| {
            let normalized = token.normalized.as_str();
            !release_group_part_is_valid(token, index > range.start_token)
                || is_explicit_language_metadata_token(normalized)
                || is_release_flag_metadata_token(normalized)
                || (range_len == 1 && normalize_standalone_streaming_service(normalized).is_some())
                || parse_special_kind(normalized).is_some()
        })
    })
}

fn has_release_group_suffix_context(state: &ParseState, tokens: &[Token], index: usize) -> bool {
    let metadata_before = state
        .metadata
        .token_indices
        .iter()
        .any(|metadata_index| *metadata_index < index);
    if metadata_before {
        return true;
    }

    let start = index.saturating_sub(8);
    tokens[start..index].iter().any(|token| {
        let normalized = token.normalized.as_str();
        let compound = detect_compound_metadata(normalized);
        compound.quality.is_some()
            || compound.source.is_some()
            || compound.video_codec.is_some()
            || compound.audio_codec.is_some()
            || compound.audio_channels.is_some()
            || parse_standard_episode_token(normalized).is_some()
            || parse_season_token(normalized).is_some()
    })
}

fn release_group_suffix_range(tokens: &[Token], index: usize) -> TokenRange {
    let mut start_token = index;
    if let Some(previous_index) = index.checked_sub(1)
        && let Some(previous) = tokens.get(previous_index)
        && (previous.separator_before == SeparatorKind::Hyphen
            || (previous.group_id.is_some()
                && previous.group_id == tokens.get(index).and_then(|token| token.group_id)))
        && previous.raw.len() <= 3
        && release_group_part_is_valid(previous, false)
    {
        start_token = previous_index;
    }

    let unit = ParseUnit {
        token_range: TokenRange::new(start_token, index + 1),
        kind: UnitKind::Token,
        raw: tokens
            .get(start_token)
            .map(|token| token.raw.clone())
            .unwrap_or_default(),
        normalized_tokens: tokens
            .get(start_token)
            .map(|token| vec![token.normalized.clone()])
            .unwrap_or_default(),
        has_strong_anchor: false,
        has_metadata_role: false,
        has_title_like_token: true,
        is_grouped: false,
    };
    release_group_token_range(&unit, tokens)
}

fn infer_terminal_release_group(
    state: &ParseState,
    tokens: &[Token],
    title_zones: &[TokenRange],
) -> Option<String> {
    let index = tokens.len().checked_sub(1)?;
    let token = tokens.get(index)?;
    if !matches!(
        token.separator_before,
        SeparatorKind::Dot | SeparatorKind::Space | SeparatorKind::Underscore
    ) || !release_group_part_is_valid(token, false)
        || !terminal_release_group_context(tokens, index)
    {
        return None;
    }
    let range = TokenRange {
        start_token: index,
        end_token: index + 1,
    };
    if !release_group_suffix_range_allowed(state, tokens, range, title_zones) {
        return None;
    }

    Some(render_token_range_preserving_separators(tokens, range))
}

fn terminal_release_group_context(tokens: &[Token], index: usize) -> bool {
    let start = index.saturating_sub(4);
    tokens[start..index].iter().any(|token| {
        let normalized = token.normalized.as_str();
        let compound = detect_compound_metadata(normalized);
        compound.source.is_some()
            || compound.audio_codec.is_some()
            || compound.audio_channels.is_some()
            || matches!(
                normalized,
                "TURKCE" | "TURKISH" | "ENGLISH" | "FRENCH" | "GERMAN" | "ITALIAN" | "DUAL"
            )
    })
}

fn is_skip_candidate(unit: &ParseUnit) -> bool {
    unit.is_grouped || !unit.has_title_like_token || unit_is_metadata_like(unit)
}

fn unit_title_delta(unit: &ParseUnit) -> i32 {
    match unit.kind {
        UnitKind::BracketGroup(_) => 8,
        UnitKind::HyphenGroup | UnitKind::SlashGroup => 7,
        UnitKind::DelimitedRun(separator) => {
            if matches!(
                separator,
                SeparatorKind::Dot | SeparatorKind::Underscore | SeparatorKind::Space
            ) {
                6
            } else {
                5
            }
        }
        UnitKind::Token => 4,
    }
}

fn unit_alias_bonus(unit: &ParseUnit, context: &ContextIndex) -> i32 {
    context
        .aliases
        .iter()
        .find(|alias| alias.tokens == unit.normalized_tokens)
        .map_or(0, |alias| {
            if alias.code == "context:title_alias_hit" {
                6
            } else {
                4
            }
        })
}

fn parse_identity_at(
    family: ParseFamily,
    tokens: &[Token],
    index: usize,
    context: &ContextIndex,
) -> Option<(ReleaseIdentity, usize, i32, &'static str)> {
    match family {
        ParseFamily::Movie => None,
        ParseFamily::StandardEpisode => {
            if let Some((season, episode_numbers, consumed)) =
                parse_parenthetical_standard_after_absolute_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season,
                        episode_numbers,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    42,
                    "family:standard_episode_with_absolute_prefix",
                ));
            }
            if let Some((season, episode_numbers, last_token)) =
                parse_hyphenated_standard_episode_range_at(tokens, index)
            {
                let score = if episode_numbers.len() > 1 { 32 } else { 28 };
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season,
                        episode_numbers,
                    },
                    last_token,
                    score,
                    "family:standard_episode_range",
                ));
            }
            if let Some((season, episode_numbers)) =
                parse_standard_episode_token(tokens.get(index)?.normalized.as_str())
            {
                let score = if episode_numbers.len() > 1 { 32 } else { 28 };
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season,
                        episode_numbers,
                    },
                    index,
                    score,
                    "family:standard_episode",
                ));
            }
            if let Some((season, episode_numbers, consumed)) =
                parse_split_standard_episode_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season,
                        episode_numbers,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    30,
                    "family:standard_episode_split",
                ));
            }
            if let Some((season, episode_numbers, consumed)) =
                parse_season_keyword_episode_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season,
                        episode_numbers,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    40,
                    "family:standard_episode_season_dash",
                ));
            }
            if let Some(season) = parse_season_token(tokens.get(index)?.normalized.as_str())
                && let Some(episode_number) = context_episode_for_season(context, season)
            {
                return Some((
                    ReleaseIdentity::StandardEpisodeIdentity {
                        season: Some(season),
                        episode_numbers: vec![episode_number],
                    },
                    index,
                    30,
                    "context:season_episode_hint",
                ));
            }
            None
        }
        ParseFamily::DailyEpisode => {
            parse_daily_at(tokens, index).map(|(air_date, consumed, part)| {
                (
                    ReleaseIdentity::DailyIdentity { air_date, part },
                    consumed.iter().max().copied().unwrap_or(index),
                    34,
                    "family:daily",
                )
            })
        }
        ParseFamily::AnimeAbsolute => parse_anime_absolute_at(tokens, index, context).map(
            |(absolute_episode_numbers, version)| {
                (
                    ReleaseIdentity::AbsoluteIdentity {
                        absolute_episode_numbers,
                        version,
                        season_hint: None,
                    },
                    index,
                    30,
                    "family:anime_absolute",
                )
            },
        ),
        ParseFamily::SeasonPack => parse_season_pack_at(tokens, index).map(
            |SeasonPackParse {
                 seasons,
                 consumed,
                 is_partial,
                 season_part,
                 is_series_pack,
             }| {
                let explicit = tokens
                    .get(index)
                    .is_some_and(|token| token.normalized == "SEASON");
                (
                    ReleaseIdentity::SeasonPackIdentity {
                        seasons,
                        is_partial,
                        season_part,
                        is_series_pack,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    if is_series_pack {
                        50
                    } else if explicit {
                        38
                    } else {
                        30
                    },
                    "family:season_pack",
                )
            },
        ),
        ParseFamily::EpisodeRangePack => {
            if let Some(RangePackParse {
                season,
                range_start,
                range_end,
                consumed,
            }) = parse_standard_episode_range_pack_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::RangePackIdentity {
                        season,
                        range_start,
                        range_end,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    44,
                    "family:range_pack_standard_episode",
                ));
            }
            if let Some(RangePackParse {
                season,
                range_start,
                range_end,
                consumed,
            }) = parse_batch_season_range_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::RangePackIdentity {
                        season,
                        range_start,
                        range_end,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    40,
                    "family:range_pack_batch_season",
                ));
            }
            if let Some(RangePackParse {
                season,
                range_start,
                range_end,
                consumed,
            }) = parse_labeled_range_pack_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::RangePackIdentity {
                        season,
                        range_start,
                        range_end,
                    },
                    consumed.iter().max().copied().unwrap_or(index),
                    if season.is_some() { 42 } else { 36 },
                    "family:range_pack_labeled",
                ));
            }
            // A bare range directly after a season marker ("Season 1 - 001-020")
            // is the same claim as the season-scoped episode span; emitting a
            // season-less duplicate here only manufactures ambiguity against
            // the standard-episode interpretation.
            if range_directly_follows_season_marker(tokens, index) {
                return None;
            }
            parse_range_pack_at(tokens, index).map(|range| {
                (
                    ReleaseIdentity::RangePackIdentity {
                        season: range.season,
                        range_start: range.range_start,
                        range_end: range.range_end,
                    },
                    range.consumed.iter().max().copied().unwrap_or(index),
                    30,
                    "family:range_pack",
                )
            })
        }
        ParseFamily::Special => {
            if let Some((special_kind, episode_hint, last_token)) =
                parse_numbered_special_at(tokens, index)
            {
                return Some((
                    ReleaseIdentity::SpecialIdentity {
                        special_kind,
                        season_hint: None,
                        episode_hint: Some(episode_hint),
                    },
                    last_token,
                    44,
                    "family:numbered_special",
                ));
            }
            parse_special_kind(tokens.get(index)?.normalized.as_str()).map(|special_kind| {
                let episode_hint = special_episode_hint_around(tokens, index);
                (
                    ReleaseIdentity::SpecialIdentity {
                        special_kind,
                        season_hint: None,
                        episode_hint,
                    },
                    index,
                    if episode_hint.is_some() { 34 } else { 24 },
                    "family:special",
                )
            })
        }
        ParseFamily::Unknown => None,
    }
}

fn parse_numbered_special_at(
    tokens: &[Token],
    index: usize,
) -> Option<(ParsedSpecialKind, u32, usize)> {
    let episode_hint = parse_numeric_token(tokens.get(index)?.normalized.as_str())?;
    for candidate_index in index + 1..=(index + 3).min(tokens.len().saturating_sub(1)) {
        let candidate = tokens.get(candidate_index)?;
        if !matches!(
            candidate.separator_before,
            SeparatorKind::Hyphen
                | SeparatorKind::Space
                | SeparatorKind::Dot
                | SeparatorKind::OpenBracket
                | SeparatorKind::OpenParen
        ) {
            continue;
        }
        if let Some(special_kind) = parse_special_kind(candidate.normalized.as_str()) {
            return Some((special_kind, episode_hint, candidate_index));
        }
    }
    None
}

fn special_episode_hint_around(tokens: &[Token], index: usize) -> Option<u32> {
    index
        .checked_sub(1)
        .and_then(|previous_index| tokens.get(previous_index))
        .filter(|token| {
            matches!(
                token.separator_after,
                SeparatorKind::Hyphen
                    | SeparatorKind::Space
                    | SeparatorKind::Dot
                    | SeparatorKind::CloseBracket
                    | SeparatorKind::CloseParen
                    | SeparatorKind::Boundary
            ) || tokens
                .get(index)
                .is_some_and(|special| special.separator_before == SeparatorKind::Hyphen)
        })
        .and_then(|token| parse_numeric_token(token.normalized.as_str()))
        .or_else(|| {
            tokens
                .get(index + 1)
                .filter(|token| {
                    matches!(
                        token.separator_before,
                        SeparatorKind::Hyphen | SeparatorKind::Space | SeparatorKind::Dot
                    )
                })
                .and_then(|token| parse_numeric_token(token.normalized.as_str()))
        })
}

fn consume_unit_metadata(
    state: &mut ParseState,
    unit: &ParseUnit,
    tokens: &[Token],
    annotations: &[TokenAnnotations],
) {
    for index in unit.token_range.start_token..unit.token_range.end_token {
        let Some(token) = tokens.get(index) else {
            continue;
        };
        let Some(annotation) = annotations.get(index) else {
            continue;
        };
        let compound = detect_compound_metadata(token.normalized.as_str());

        if state.metadata.year.is_none()
            && let Some(year) = parse_year(token.normalized.as_str())
        {
            state.metadata.year = Some(year);
            state.metadata.year_span = Some(TokenRange {
                start_token: index,
                end_token: index + 1,
            });
            state.score += 12;
            state.reasons.push(reason("metadata:year", 12, None));
            state.consumed_tokens.insert(index);
            record_metadata_token(state, index);
            continue;
        }
        if state.metadata.quality.is_none()
            && let Some(quality) = compound.quality.as_deref()
        {
            state.metadata.quality = Some(quality.to_string());
            state.metadata.quality_span = Some(TokenRange {
                start_token: index,
                end_token: index + 1,
            });
            state.score += 10;
            state.reasons.push(reason("metadata:quality", 10, None));
            state.consumed_tokens.insert(index);
            record_metadata_token(state, index);
        }
        if state.metadata.source.is_none()
            && let Some(source) = compound.source
        {
            state.metadata.source = Some(source.to_string());
            state.metadata.source_span = Some(source_span_for_token(tokens, index));
            state.score += 10;
            state.reasons.push(reason("metadata:source", 10, None));
            mark_metadata_span(
                state,
                state.metadata.source_span.unwrap_or(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                }),
            );
        }
        match annotation.primary_role {
            TokenRole::Quality if state.metadata.quality.is_none() => {
                state.metadata.quality = Some(token.raw.clone());
                state.metadata.quality_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 10;
                state.reasons.push(reason("metadata:quality", 10, None));
                record_metadata_token(state, index);
            }
            TokenRole::Source if state.metadata.source.is_none() => {
                state.metadata.source = Some(coalesce_source(tokens, index));
                state.metadata.source_span = Some(source_span_for_token(tokens, index));
                state.score += 10;
                state.reasons.push(reason("metadata:source", 10, None));
                mark_metadata_span(
                    state,
                    state.metadata.source_span.unwrap_or(TokenRange {
                        start_token: index,
                        end_token: index + 1,
                    }),
                );
            }
            TokenRole::VideoCodec if state.metadata.video_codec.is_none() => {
                state.metadata.video_codec = Some(
                    compound
                        .video_codec
                        .map(str::to_string)
                        .unwrap_or_else(|| token.raw.clone()),
                );
                state.metadata.video_codec_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 8;
                state.reasons.push(reason("metadata:video_codec", 8, None));
                record_metadata_token(state, index);
            }
            TokenRole::AudioCodec if state.metadata.audio_codec.is_none() => {
                state.metadata.audio_codec = Some(
                    compound
                        .audio_codec
                        .map(str::to_string)
                        .unwrap_or_else(|| token.raw.clone()),
                );
                state.metadata.audio_codec_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 8;
                state.reasons.push(reason("metadata:audio_codec", 8, None));
                record_metadata_token(state, index);
            }
            TokenRole::AudioChannels
                if state.metadata.audio_channels.is_none()
                    && audio_channel_has_audio_context(tokens, index) =>
            {
                state.metadata.audio_channels = Some(
                    compound
                        .audio_channels
                        .map(str::to_string)
                        .unwrap_or_else(|| normalize_channels(token.raw.as_str())),
                );
                state.metadata.audio_channels_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 6;
                state
                    .reasons
                    .push(reason("metadata:audio_channels", 6, None));
                record_metadata_token(state, index);
            }
            TokenRole::StreamingService if state.metadata.streaming_service.is_none() => {
                state.metadata.streaming_service = Some(
                    normalize_streaming_service(token.normalized.as_str())
                        .unwrap_or(token.raw.as_str())
                        .to_string(),
                );
                state.metadata.streaming_service_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 4;
                state.reasons.push(reason("metadata:service", 4, None));
                record_metadata_token(state, index);
            }
            TokenRole::Language => {
                state.reasons.push(reason("metadata:language", 1, None));
                record_metadata_token(state, index);
            }
            TokenRole::ReleaseFlag => {
                state.reasons.push(reason("metadata:release_flag", 1, None));
                record_metadata_token(state, index);
            }
            TokenRole::Edition if matches!(token.normalized.as_str(), "PROPER" | "REPACK") => {
                // Revision flags, not editions; enrichment owns the
                // is_proper_upload/is_repack projection. Scores +4 like the
                // edition arm so the flag itself ranks identically (a later
                // real edition may now also be consumed, which is the intent).
                state.score += 4;
                state.reasons.push(reason("metadata:release_flag", 4, None));
                record_metadata_token(state, index);
            }
            TokenRole::Edition
                if state.metadata.edition.is_none()
                    || token.normalized.eq_ignore_ascii_case("REMUX") =>
            {
                state.metadata.edition = Some(token.raw.clone());
                state.metadata.edition_span = Some(TokenRange {
                    start_token: index,
                    end_token: index + 1,
                });
                state.score += 4;
                state.reasons.push(reason("metadata:edition", 4, None));
                record_metadata_token(state, index);
            }
            TokenRole::ExternalId => {
                if let Some((external_id, span)) = parse_external_id_at(tokens, index) {
                    push_unique_external_id(&mut state.metadata.external_ids, external_id);
                    state.metadata.external_id_spans.push(TokenRange {
                        start_token: span.start_token,
                        end_token: span.end_token,
                    });
                    state.score += 8;
                    state.reasons.push(reason("metadata:external_id", 8, None));
                    mark_metadata_span(state, span);
                }
            }
            _ => {}
        }
    }
}

fn record_metadata_token(state: &mut ParseState, index: usize) {
    state.consumed_tokens.insert(index);
    if !state.metadata_token_mask.contains(index) {
        state.metadata_token_mask.insert(index);
        state.metadata.token_indices.push(index);
    }
}

fn mark_metadata_span(state: &mut ParseState, span: TokenRange) {
    for index in span.start_token..span.end_token {
        record_metadata_token(state, index);
    }
}

fn audio_channel_has_audio_context(tokens: &[Token], index: usize) -> bool {
    let start = index.saturating_sub(3);
    let end = (index + 3).min(tokens.len().saturating_sub(1));
    tokens[start..=end]
        .iter()
        .enumerate()
        .any(|(offset, token)| {
            let token_index = start + offset;
            token_index != index && token_has_audio_codec(token.normalized.as_str())
        })
}

fn token_has_audio_codec(token: &str) -> bool {
    matches!(
        token,
        "AAC"
            | "DD"
            | "DDP"
            | "AC3"
            | "EAC3"
            | "TRUEHD"
            | "DTS"
            | "DTSHD"
            | "DTSMA"
            | "DTSX"
            | "FLAC"
            | "OPUS"
            | "MP3"
            | "PCM"
            | "LPCM"
    ) || detect_compound_metadata(token).audio_codec.is_some()
}

fn source_span_for_token(tokens: &[Token], index: usize) -> TokenRange {
    let Some(token) = tokens.get(index) else {
        return TokenRange {
            start_token: index,
            end_token: index + 1,
        };
    };
    if token.normalized == "WEB"
        && tokens.get(index + 1).is_some_and(|candidate| {
            candidate.separator_before == SeparatorKind::Hyphen
                && matches!(candidate.normalized.as_str(), "DL" | "RIP")
        })
    {
        return TokenRange {
            start_token: index,
            end_token: index + 2,
        };
    }
    if token.normalized == "DVD"
        && tokens
            .get(index + 1)
            .is_some_and(|candidate| candidate.normalized == "RIP")
    {
        return TokenRange {
            start_token: index,
            end_token: index + 2,
        };
    }
    TokenRange {
        start_token: index,
        end_token: index + 1,
    }
}

fn build_candidate(
    state: ParseState,
    tokens: &[Token],
    annotations: &[TokenAnnotations],
    raw_input: &str,
    parser_version: &'static str,
    context_index: &ContextIndex,
    alias_oracle: &AliasOracle,
) -> ReleaseParseCandidate {
    let title_segments = title_segments_for_state(
        tokens,
        state.title_token_indices.as_slice(),
        state.accepted_alias_hits.as_slice(),
        alias_oracle,
    );
    let context_title_matches = context_title_matches_for_state(
        tokens,
        state.title_token_indices.as_slice(),
        state.accepted_alias_hits.as_slice(),
        alias_oracle,
    );
    let canonical_context_title = canonical_context_title(context_index);
    let title_context_matched = state.context_evidence.iter().any(|code| {
        matches!(
            code.as_str(),
            "context:title_alias_hit" | "context:title_canonical_hit"
        )
    });
    let fallback_title = fallback_title_from_context(tokens, annotations, context_index);
    let normalized_title = if title_context_matched {
        canonical_context_title
            .clone()
            .or(fallback_title.clone())
            .unwrap_or_else(|| {
                title_segments
                    .iter()
                    .find(|segment| segment.kind == TitleSegmentKind::ObservedPrimary)
                    .map(|segment| segment.normalized.clone())
                    .unwrap_or_else(|| {
                        state
                            .title_token_indices
                            .iter()
                            .filter_map(|index| tokens.get(*index))
                            .map(|token| token.normalized.clone())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
            })
    } else {
        fallback_title.clone().unwrap_or_else(|| {
            title_segments
                .iter()
                .find(|segment| segment.kind == TitleSegmentKind::ObservedPrimary)
                .map(|segment| segment.normalized.clone())
                .unwrap_or_else(|| {
                    state
                        .title_token_indices
                        .iter()
                        .filter_map(|index| tokens.get(*index))
                        .map(|token| token.normalized.clone())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
        })
    };
    let mut normalized_title_variants = title_segments
        .iter()
        .map(|segment| segment.normalized.clone())
        .filter(|title| !title.trim().is_empty())
        .fold(Vec::<String>::new(), |mut acc, title| {
            if !acc
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&title))
            {
                acc.push(title);
            }
            acc
        });
    if title_context_matched
        && let Some(canonical) = canonical_context_title
        && !normalized_title_variants
            .iter()
            .any(|title| title.eq_ignore_ascii_case(&canonical))
    {
        normalized_title_variants.insert(0, canonical);
    }
    if let Some(fallback) = fallback_title
        && !normalized_title_variants
            .iter()
            .any(|title| title.eq_ignore_ascii_case(&fallback))
    {
        normalized_title_variants.push(fallback);
    }
    extend_title_connector_variants(
        &mut normalized_title_variants,
        tokens,
        state.title_token_indices.as_slice(),
    );
    let episode = project_episode(&state.identity, tokens, context_index);
    let external_ids = state.metadata.external_ids.clone();
    let imdb_id = external_ids
        .iter()
        .find(|value| value.source == ExternalIdSource::Imdb)
        .map(|value| value.value.clone());
    let tmdb_id = external_ids
        .iter()
        .find(|value| value.source == ExternalIdSource::Tmdb)
        .map(|value| value.value.clone());
    let tvdb_id = external_ids
        .iter()
        .find(|value| value.source == ExternalIdSource::Tvdb)
        .map(|value| value.value.clone());
    let unconsumed_tokens = tokens
        .iter()
        .enumerate()
        .filter(|(index, _)| !state.consumed_tokens.contains(*index))
        .map(|(_, token)| token.span)
        .collect::<Vec<_>>();
    let release_group = infer_release_group_for_candidate(&state, tokens);
    let raw_score = finalize_score(&state, tokens, annotations);
    let zones = build_candidate_zones(&state, tokens, release_group.as_deref());
    let is_remux = state
        .metadata
        .edition
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("remux"));
    let projected = ParsedReleaseMetadata {
        raw_title: raw_input.to_string(),
        guide_facts: Vec::new(),
        normalized_title,
        normalized_title_variants,
        release_group: release_group.clone(),
        languages_audio: Vec::new(),
        languages_subtitles: Vec::new(),
        external_ids,
        imdb_id,
        tmdb_id,
        tvdb_id,
        year: state.metadata.year,
        quality: state.metadata.quality.clone(),
        source: state
            .metadata
            .source
            .as_deref()
            .and_then(ReleaseSource::parse),
        video_codec: state
            .metadata
            .video_codec
            .as_deref()
            .and_then(VideoCodec::parse),
        video_encoding: None,
        audio: state
            .metadata
            .audio_codec
            .as_deref()
            .and_then(AudioCodec::parse),
        audio_codecs: state
            .metadata
            .audio_codec
            .as_deref()
            .and_then(AudioCodec::parse)
            .into_iter()
            .collect::<Vec<_>>(),
        audio_channels: state.metadata.audio_channels.clone(),
        is_dual_audio: false,
        is_atmos: false,
        is_dolby_vision: false,
        detected_hdr: false,
        has_hdr_fallback: false,
        is_hdr10plus: false,
        is_hlg: false,
        is_10bit: false,
        fps: None,
        is_proper_upload: false,
        is_repack: false,
        is_remux,
        is_bd_disk: false,
        is_ai_enhanced: false,
        is_hardcoded_subs: false,
        is_uncensored: false,
        is_dubs_only: false,
        streaming_service: state
            .metadata
            .streaming_service
            .as_deref()
            .and_then(StreamingService::parse),
        edition: (!is_remux)
            .then(|| state.metadata.edition.as_deref().map(canonical_edition))
            .flatten(),
        anime_version: anime_version_from_identity(&state.identity),
        episode,
        parser_version,
        scoring_model_version: SCORING_MODEL_VERSION,
        parse_confidence: (raw_score.max(0) as f32 / 100.0).clamp(0.0, 1.0),
        ambiguity_margin: 0,
        is_ambiguous: false,
        disposition: ParseDisposition::Parsed,
        parse_family: state.family,
        missing_fields: Vec::new(),
        parse_hints: state
            .reasons
            .iter()
            .map(|reason| reason.code.clone())
            .collect(),
    };

    ReleaseParseCandidate {
        family: state.family,
        title_segments,
        context_title_matches,
        identity: state.identity,
        metadata: state.metadata,
        zones,
        release_group,
        unconsumed_tokens,
        reasons: state.reasons,
        raw_evidence: state.raw_evidence.into_vec(),
        context_evidence: state.context_evidence.into_vec(),
        raw_score,
        enrichment: None,
        projected,
    }
}

fn build_candidate_zones(
    state: &ParseState,
    tokens: &[Token],
    release_group: Option<&str>,
) -> CandidateZones {
    let title_zones = contiguous_token_ranges(state.title_token_indices.as_slice());
    let max_metadata_token = state.metadata.token_indices.iter().max().copied();
    let metadata_zone = state
        .metadata
        .token_indices
        .iter()
        .min()
        .copied()
        .map(|start_token| TokenRange {
            start_token,
            end_token: tokens.len(),
        });
    let release_group_span = state
        .release_group
        .as_deref()
        .or(release_group)
        .and_then(|release_group| find_phrase_span(tokens, release_group))
        .filter(|span| {
            max_metadata_token.is_none_or(|max_token| span.start_token > max_token)
                && !overlaps_range(*span, state.metadata.source_span)
                && !overlaps_range(*span, state.metadata.streaming_service_span)
                && !overlaps_range(*span, state.metadata.video_codec_span)
                && !overlaps_range(*span, state.metadata.audio_codec_span)
                && !overlaps_range(*span, state.metadata.audio_channels_span)
        });
    let metadata_zone = metadata_zone.map(|range| {
        if let Some(group_span) = release_group_span
            && group_span.start_token >= range.start_token
        {
            return TokenRange {
                start_token: range.start_token,
                end_token: group_span.start_token,
            };
        }
        range
    });
    let trailing_zone = release_group_span.and_then(|group_span| {
        (group_span.end_token < tokens.len()).then_some(TokenRange {
            start_token: group_span.end_token,
            end_token: tokens.len(),
        })
    });

    CandidateZones {
        title_zones,
        metadata_zone,
        trailing_zone,
        source_span: state.metadata.source_span,
        service_span: state.metadata.streaming_service_span,
        video_span: state.metadata.video_codec_span,
        audio_span: state
            .metadata
            .audio_codec_span
            .or(state.metadata.audio_channels_span),
        language_span: None,
        edition_span: state.metadata.edition_span,
        release_group_span,
    }
}

fn overlaps_range(left: TokenRange, right: Option<TokenRange>) -> bool {
    let Some(right) = right else {
        return false;
    };
    left.start_token < right.end_token && right.start_token < left.end_token
}

fn contiguous_token_ranges(indices: &[usize]) -> Vec<TokenRange> {
    if indices.is_empty() {
        return Vec::new();
    }
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges = Vec::new();
    let mut start = sorted[0];
    let mut end = start + 1;

    for index in sorted.into_iter().skip(1) {
        if index == end {
            end += 1;
            continue;
        }
        ranges.push(TokenRange {
            start_token: start,
            end_token: end,
        });
        start = index;
        end = index + 1;
    }
    ranges.push(TokenRange {
        start_token: start,
        end_token: end,
    });
    ranges
}

fn find_phrase_span(tokens: &[Token], phrase: &str) -> Option<TokenRange> {
    let phrase_tokens = normalized_phrase(phrase)?;
    if phrase_tokens.is_empty() || tokens.len() < phrase_tokens.len() {
        return None;
    }
    for start in 0..=tokens.len() - phrase_tokens.len() {
        let matches = phrase_tokens.iter().enumerate().all(|(offset, token)| {
            tokens
                .get(start + offset)
                .is_some_and(|candidate| &candidate.normalized == token)
        });
        if matches {
            return Some(TokenRange {
                start_token: start,
                end_token: start + phrase_tokens.len(),
            });
        }
    }
    None
}

fn apply_context(
    state: &mut ParseState,
    tokens: &[Token],
    context: &ContextIndex,
    alias_oracle: &AliasOracle,
) {
    for hit in contextual_hits_within_title_zone(
        state.title_token_indices.as_slice(),
        state.accepted_alias_hits.as_slice(),
        alias_oracle,
    ) {
        let Some(pattern) = alias_oracle.patterns.get(hit.pattern_id) else {
            continue;
        };
        state.score += hit.score_weight;
        state.context_evidence.push(hit.evidence.code().to_string());
        state.reasons.push(reason(
            hit.evidence.code(),
            hit.score_weight,
            Some(pattern.raw.clone()),
        ));
    }
    if state.metadata.year.is_none()
        && let Some(year) = tokens
            .iter()
            .rev()
            .filter_map(|token| parse_year(token.normalized.as_str()))
            .find(|year| contains_sorted(&context.years, year))
    {
        state.metadata.year = Some(year);
        state
            .context_evidence
            .push("context:year_token_hit".to_string());
        state
            .reasons
            .push(reason("context:year_token_hit", 8, Some(year.to_string())));
    }
    if let Some(year) = state.metadata.year
        && contains_sorted(&context.years, &year)
    {
        state.score += 8;
        state
            .context_evidence
            .push("context:year_match".to_string());
        state
            .reasons
            .push(reason("context:year_match", 8, Some(year.to_string())));
    }
    if let ReleaseIdentity::DailyIdentity { air_date, .. } = state.identity
        && contains_sorted(&context.air_dates, &air_date)
    {
        state.score += 8;
        state
            .context_evidence
            .push("context:air_date_hit".to_string());
        state.reasons.push(reason(
            "context:air_date_hit",
            8,
            Some(air_date.to_string()),
        ));
    }
    if let ReleaseIdentity::AbsoluteIdentity {
        absolute_episode_numbers,
        ..
    } = &state.identity
        && absolute_episode_numbers
            .iter()
            .any(|number| contains_sorted(&context.absolute_numbers, number))
    {
        state.score += 10;
        state
            .context_evidence
            .push("context:absolute_mapping_hit".to_string());
        state
            .reasons
            .push(reason("context:absolute_mapping_hit", 10, None));
    }
}

fn title_segments_for_state(
    tokens: &[Token],
    title_indices: &[usize],
    accepted_alias_hits: &[AliasHit],
    alias_oracle: &AliasOracle,
) -> Vec<TitleSegment> {
    if title_indices.is_empty() {
        return Vec::new();
    }
    let default_normalized = normalized_title_from_indices(tokens, title_indices);
    let default_segment = TitleSegment {
        kind: TitleSegmentKind::ObservedPrimary,
        token_start: *title_indices.first().unwrap_or(&0),
        token_end: title_indices.last().copied().unwrap_or(0) + 1,
        raw: render_token_indices(tokens, title_indices),
        normalized: default_normalized,
    };
    let contextual_hits =
        contextual_hits_within_title_zone(title_indices, accepted_alias_hits, alias_oracle);
    let mut matches = accepted_alias_hits
        .iter()
        .chain(contextual_hits.iter())
        .filter_map(|hit| {
            let pattern = alias_oracle.patterns.get(hit.pattern_id)?;
            Some(TitleSegment {
                kind: TitleSegmentKind::ContextMatchedAlias,
                token_start: hit.token_range.start_token,
                token_end: hit.token_range.end_token,
                raw: render_token_indices(
                    tokens,
                    &(hit.token_range.start_token..hit.token_range.end_token).collect::<Vec<_>>(),
                ),
                normalized: pattern.text.clone(),
            })
        })
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return vec![default_segment];
    }

    matches.sort_by(|left, right| {
        let left_len = left.token_end.saturating_sub(left.token_start);
        let right_len = right.token_end.saturating_sub(right.token_start);
        right_len
            .cmp(&left_len)
            .then(left.token_start.cmp(&right.token_start))
    });
    let mut segments = Vec::new();
    let primary = matches.first().cloned().unwrap_or(default_segment.clone());
    segments.push(TitleSegment {
        kind: TitleSegmentKind::ObservedPrimary,
        ..primary
    });
    if !default_segment.normalized.trim().is_empty()
        && !segments.iter().any(|existing| {
            existing
                .normalized
                .eq_ignore_ascii_case(&default_segment.normalized)
        })
    {
        segments.push(TitleSegment {
            kind: TitleSegmentKind::ObservedAlternate,
            ..default_segment.clone()
        });
    }
    for segment in matches.into_iter().skip(1) {
        if !segments.iter().any(|existing| {
            existing
                .normalized
                .eq_ignore_ascii_case(&segment.normalized)
        }) {
            segments.push(TitleSegment {
                kind: TitleSegmentKind::ObservedAlternate,
                ..segment
            });
        }
    }
    segments
}

fn context_title_matches_for_state(
    tokens: &[Token],
    title_indices: &[usize],
    accepted_alias_hits: &[AliasHit],
    alias_oracle: &AliasOracle,
) -> Vec<ContextTitleMatch> {
    let contextual_hits =
        contextual_hits_within_title_zone(title_indices, accepted_alias_hits, alias_oracle);
    let mut matches = accepted_alias_hits
        .iter()
        .chain(contextual_hits.iter())
        .filter_map(|hit| {
            let pattern = alias_oracle.patterns.get(hit.pattern_id)?;
            let kind = match hit.evidence {
                AliasEvidenceKind::CanonicalTitle => ContextTitleMatchKind::CanonicalTitle,
                AliasEvidenceKind::TitleAlias => ContextTitleMatchKind::TitleAlias,
                AliasEvidenceKind::EpisodeTitle => ContextTitleMatchKind::EpisodeTitle,
            };
            Some(ContextTitleMatch {
                kind,
                token_range: hit.token_range,
                raw: render_token_indices(
                    tokens,
                    &(hit.token_range.start_token..hit.token_range.end_token).collect::<Vec<_>>(),
                ),
                normalized: pattern.text.clone(),
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        left.token_range
            .start_token
            .cmp(&right.token_range.start_token)
            .then(right.token_range.len().cmp(&left.token_range.len()))
            .then(left.kind.cmp(&right.kind))
    });
    matches.dedup_by(|left, right| {
        left.kind == right.kind
            && left.token_range == right.token_range
            && left.normalized == right.normalized
    });
    matches
}

fn extend_title_connector_variants(
    variants: &mut Vec<String>,
    tokens: &[Token],
    title_indices: &[usize],
) {
    let Some(full_title) = (!title_indices.is_empty())
        .then(|| normalized_title_from_indices(tokens, title_indices))
        .filter(|title| !title.trim().is_empty())
    else {
        return;
    };
    push_unique_title_variant(variants, full_title.clone());

    for (left, right) in aka_split_positions(tokens, title_indices) {
        push_side_title_variant(variants, tokens, left);
        push_side_title_variant(variants, tokens, right);
    }

    for split_index in slash_split_positions(tokens, title_indices) {
        let left = &title_indices[..split_index];
        let right = &title_indices[split_index..];
        push_side_title_variant(variants, tokens, left);
        push_side_title_variant(variants, tokens, right);
    }

    for split_index in subtitle_split_positions(tokens, title_indices) {
        let left = &title_indices[..split_index];
        let right = &title_indices[split_index..];
        push_side_title_variant(variants, tokens, left);
        push_side_title_variant(variants, tokens, right);
        if let Some(prefix) = leading_connector_segment(tokens, left)
            && prefix.len() < left.len()
        {
            push_combined_title_variant(variants, tokens, prefix, right);
        }
    }

    if let Some(without_part) = remove_part_connector_variant(tokens, title_indices) {
        push_unique_title_variant(variants, without_part);
    }
}

fn push_side_title_variant(variants: &mut Vec<String>, tokens: &[Token], side: &[usize]) {
    let Some(title) = (!side.is_empty())
        .then(|| normalized_title_from_indices(tokens, side))
        .filter(|title| !title.trim().is_empty())
    else {
        return;
    };
    push_unique_title_variant(variants, title);
}

fn push_combined_title_variant(
    variants: &mut Vec<String>,
    tokens: &[Token],
    left: &[usize],
    right: &[usize],
) {
    if left.is_empty() || right.is_empty() {
        return;
    }
    let combined = left.iter().chain(right.iter()).copied().collect::<Vec<_>>();
    push_side_title_variant(variants, tokens, &combined);
}

fn leading_connector_segment<'a>(tokens: &[Token], indices: &'a [usize]) -> Option<&'a [usize]> {
    let split_offset = indices
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(offset, token_index)| {
            tokens
                .get(*token_index)
                .is_some_and(|token| {
                    matches!(
                        token.separator_before,
                        SeparatorKind::Other | SeparatorKind::Slash
                    )
                })
                .then_some(offset)
        })?;
    Some(&indices[..split_offset]).filter(|segment| !segment.is_empty())
}

fn push_unique_title_variant(variants: &mut Vec<String>, title: String) {
    if !variants
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&title))
    {
        variants.push(title);
    }
}

fn normalized_title_from_indices(tokens: &[Token], indices: &[usize]) -> String {
    let mut words = Vec::new();
    let mut index = 0usize;
    while index < indices.len() {
        if is_aka_sequence(tokens, indices, index) {
            words.push("AKA".to_string());
            index += 3;
            continue;
        }
        if let Some(token) = indices
            .get(index)
            .and_then(|token_index| tokens.get(*token_index))
            .filter(|token| !token.normalized.is_empty())
        {
            words.push(token.normalized.clone());
        }
        index += 1;
    }
    words.join(" ")
}

fn is_aka_sequence(tokens: &[Token], indices: &[usize], index: usize) -> bool {
    let Some(a) = indices
        .get(index)
        .and_then(|token_index| tokens.get(*token_index))
    else {
        return false;
    };
    if a.normalized == "AKA" {
        return false;
    }
    let Some(k) = indices
        .get(index + 1)
        .and_then(|token_index| tokens.get(*token_index))
    else {
        return false;
    };
    let Some(second_a) = indices
        .get(index + 2)
        .and_then(|token_index| tokens.get(*token_index))
    else {
        return false;
    };
    a.normalized == "A" && k.normalized == "K" && second_a.normalized == "A"
}

fn aka_split_positions<'a>(
    tokens: &[Token],
    indices: &'a [usize],
) -> Vec<(&'a [usize], &'a [usize])> {
    let mut splits = Vec::new();
    let mut index = 0usize;
    while index < indices.len() {
        if is_aka_sequence(tokens, indices, index) {
            splits.push((&indices[..index], &indices[index + 3..]));
            index += 3;
            continue;
        }
        if indices
            .get(index)
            .and_then(|token_index| tokens.get(*token_index))
            .is_some_and(|token| token.normalized == "AKA")
        {
            splits.push((&indices[..index], &indices[index + 1..]));
        }
        index += 1;
    }
    splits
}

fn slash_split_positions(tokens: &[Token], indices: &[usize]) -> Vec<usize> {
    indices
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(offset, token_index)| {
            tokens
                .get(*token_index)
                .is_some_and(|token| token.separator_before == SeparatorKind::Slash)
                .then_some(offset)
        })
        .collect()
}

fn subtitle_split_positions(tokens: &[Token], indices: &[usize]) -> Vec<usize> {
    indices
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(offset, token_index)| {
            tokens
                .get(*token_index)
                .is_some_and(|token| token.separator_before == SeparatorKind::Other)
                .then_some(offset)
        })
        .collect()
}

fn remove_part_connector_variant(tokens: &[Token], indices: &[usize]) -> Option<String> {
    let filtered = indices
        .iter()
        .copied()
        .filter(|index| {
            tokens
                .get(*index)
                .is_none_or(|token| token.normalized != "PART")
        })
        .collect::<Vec<_>>();
    (filtered.len() < indices.len() && !filtered.is_empty())
        .then(|| normalized_title_from_indices(tokens, &filtered))
}

fn canonical_edition(raw: &str) -> String {
    match raw.to_ascii_uppercase().as_str() {
        "UNCUT" => "Uncut".to_string(),
        "UNCENSORED" => "Uncensored".to_string(),
        "EXTENDED" => "Extended".to_string(),
        "DIRECTORS" | "DIRECTOR" => "Directors".to_string(),
        _ => raw.to_string(),
    }
}

fn anime_version_from_identity(identity: &ReleaseIdentity) -> Option<u32> {
    match identity {
        ReleaseIdentity::AbsoluteIdentity { version, .. } => *version,
        _ => None,
    }
}

fn contextual_hits_within_title_zone(
    title_indices: &[usize],
    accepted_alias_hits: &[AliasHit],
    alias_oracle: &AliasOracle,
) -> Vec<AliasHit> {
    if title_indices.is_empty() {
        return Vec::new();
    }
    let mut title_indices_sorted = title_indices.to_vec();
    title_indices_sorted.sort_unstable();
    title_indices_sorted.dedup();

    let mut ordered_hits = title_indices_sorted
        .iter()
        .filter_map(|token_start| alias_oracle.hits_at.get(*token_start))
        .flat_map(|hits| hits.iter())
        .filter(|hit| {
            (hit.token_range.start_token..hit.token_range.end_token)
                .all(|index| title_indices_sorted.binary_search(&index).is_ok())
                && !accepted_alias_hits.iter().any(|accepted| {
                    accepted.token_range.start_token == hit.token_range.start_token
                        && accepted.token_range.end_token == hit.token_range.end_token
                        && accepted.pattern_id == hit.pattern_id
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    ordered_hits.sort_by(|left, right| {
        left.token_range
            .start_token
            .cmp(&right.token_range.start_token)
            .then(left.evidence.precedence().cmp(&right.evidence.precedence()))
            .then(right.token_range.len().cmp(&left.token_range.len()))
            .then(left.pattern_id.cmp(&right.pattern_id))
    });
    let mut selected = Vec::new();
    for hit in ordered_hits {
        if selected.iter().any(|accepted_hit: &AliasHit| {
            accepted_hit.token_range.start_token <= hit.token_range.start_token
                && accepted_hit.token_range.end_token >= hit.token_range.end_token
        }) {
            continue;
        }
        selected.push(hit);
    }
    selected
}

fn project_episode(
    identity: &ReleaseIdentity,
    tokens: &[Token],
    context: &ContextIndex,
) -> Option<ParsedEpisodeMetadata> {
    match identity {
        ReleaseIdentity::StandardEpisodeIdentity {
            season,
            episode_numbers,
        } => Some(ParsedEpisodeMetadata {
            season: *season,
            season_numbers: season.iter().copied().collect(),
            episode_numbers: episode_numbers.clone(),
            absolute_episode: contextual_absolute_companion(identity, tokens, context)
                .first()
                .copied(),
            absolute_episode_numbers: contextual_absolute_companion(identity, tokens, context),
            special_absolute_episode_numbers: Vec::new(),
            air_date: None,
            daily_part: None,
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_series_pack: false,
            season_part: None,
            is_season_extra: false,
            is_split_episode: episode_numbers.len() > 1,
            is_mini_series: detect_mini_series_tokens(tokens),
            special_kind: None,
            release_type: if episode_numbers.len() > 1 {
                ParsedEpisodeReleaseType::MultiEpisode
            } else {
                ParsedEpisodeReleaseType::SingleEpisode
            },
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::DailyIdentity { air_date, part } => Some(ParsedEpisodeMetadata {
            season: None,
            season_numbers: Vec::new(),
            episode_numbers: Vec::new(),
            absolute_episode: contextual_absolute_companion(identity, tokens, context)
                .first()
                .copied(),
            absolute_episode_numbers: contextual_absolute_companion(identity, tokens, context),
            special_absolute_episode_numbers: Vec::new(),
            air_date: Some(*air_date),
            daily_part: *part,
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_series_pack: false,
            season_part: None,
            is_season_extra: false,
            is_split_episode: false,
            is_mini_series: false,
            special_kind: None,
            release_type: ParsedEpisodeReleaseType::Daily,
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::AbsoluteIdentity {
            absolute_episode_numbers,
            ..
        } => Some(ParsedEpisodeMetadata {
            season: None,
            season_numbers: Vec::new(),
            episode_numbers: Vec::new(),
            absolute_episode: absolute_episode_numbers.first().copied(),
            absolute_episode_numbers: absolute_episode_numbers.clone(),
            special_absolute_episode_numbers: Vec::new(),
            air_date: None,
            daily_part: None,
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_series_pack: false,
            season_part: None,
            is_season_extra: false,
            is_split_episode: false,
            is_mini_series: detect_mini_series_tokens(tokens),
            special_kind: None,
            release_type: if absolute_episode_numbers.len() > 1 {
                ParsedEpisodeReleaseType::RangePack
            } else {
                ParsedEpisodeReleaseType::SingleEpisode
            },
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::SeasonPackIdentity {
            seasons,
            is_partial,
            season_part,
            is_series_pack,
            ..
        } => Some(ParsedEpisodeMetadata {
            season: (seasons.len() == 1).then(|| seasons[0]),
            season_numbers: seasons.clone(),
            episode_numbers: Vec::new(),
            absolute_episode: None,
            absolute_episode_numbers: Vec::new(),
            special_absolute_episode_numbers: Vec::new(),
            air_date: None,
            daily_part: None,
            full_season: !is_partial,
            is_partial_season: *is_partial,
            is_multi_season: seasons.len() > 1,
            is_series_pack: *is_series_pack,
            season_part: *season_part,
            is_season_extra: detect_season_extra_tokens(tokens),
            is_split_episode: false,
            is_mini_series: false,
            special_kind: None,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::RangePackIdentity {
            season,
            range_start,
            range_end,
        } => Some(ParsedEpisodeMetadata {
            season: *season,
            season_numbers: season.iter().copied().collect(),
            episode_numbers: if season.is_some() {
                (*range_start..=*range_end).collect()
            } else {
                Vec::new()
            },
            absolute_episode: season.is_none().then_some(*range_start),
            absolute_episode_numbers: if season.is_none() {
                (*range_start..=*range_end).collect()
            } else {
                Vec::new()
            },
            special_absolute_episode_numbers: Vec::new(),
            air_date: None,
            daily_part: None,
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_series_pack: false,
            season_part: None,
            is_season_extra: false,
            is_split_episode: false,
            is_mini_series: false,
            special_kind: None,
            release_type: ParsedEpisodeReleaseType::RangePack,
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::SpecialIdentity {
            special_kind,
            episode_hint,
            ..
        } => Some(ParsedEpisodeMetadata {
            season: None,
            season_numbers: Vec::new(),
            episode_numbers: Vec::new(),
            absolute_episode: None,
            absolute_episode_numbers: Vec::new(),
            special_absolute_episode_numbers: episode_hint.iter().copied().collect(),
            air_date: None,
            daily_part: None,
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_series_pack: false,
            season_part: None,
            is_season_extra: false,
            is_split_episode: false,
            is_mini_series: false,
            special_kind: Some(*special_kind),
            release_type: ParsedEpisodeReleaseType::SingleEpisode,
            raw: Some(render_identity_raw(identity, tokens)),
        }),
        ReleaseIdentity::MovieIdentity | ReleaseIdentity::Unknown => None,
    }
}

fn detect_season_extra_tokens(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(token.normalized.as_str(), "EXTRAS" | "SUBPACK"))
}

fn detect_mini_series_tokens(tokens: &[Token]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token.normalized.as_str(), "PART" | "PT" | "VOL" | "VOLUME")
            && tokens.get(index + 1).is_some_and(|candidate| {
                parse_numeric_token(candidate.normalized.as_str()).is_some()
            })
    })
}

fn contextual_absolute_companion(
    identity: &ReleaseIdentity,
    tokens: &[Token],
    context: &ContextIndex,
) -> Vec<u32> {
    if context.absolute_numbers.is_empty() {
        return Vec::new();
    }
    if !matches!(
        identity,
        ReleaseIdentity::StandardEpisodeIdentity { .. } | ReleaseIdentity::DailyIdentity { .. }
    ) {
        return Vec::new();
    }

    let candidates = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            parse_numeric_token(token.normalized.as_str()).map(|value| (index, value))
        })
        .filter(|(_, value)| contains_sorted(&context.absolute_numbers, value))
        .filter(|(index, value)| absolute_companion_allowed(tokens, *index, *value))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();

    let mut candidates = candidates;
    sort_and_dedup(&mut candidates);

    if candidates.len() == 1 {
        candidates
    } else {
        Vec::new()
    }
}

fn absolute_companion_allowed(tokens: &[Token], index: usize, value: u32) -> bool {
    if value > 2000 {
        return false;
    }
    let Some(token) = tokens.get(index) else {
        return false;
    };
    let previous = index.checked_sub(1).and_then(|prev| tokens.get(prev));
    let next = tokens.get(index + 1);
    let title_sandwiched = previous.is_some_and(is_title_like_token)
        && next.is_some_and(is_title_like_token)
        && token.separator_before != SeparatorKind::Hyphen
        && next.is_none_or(|candidate| candidate.separator_before != SeparatorKind::Hyphen);

    if title_sandwiched {
        return false;
    }

    token.separator_before == SeparatorKind::Hyphen
        || previous.is_some_and(|candidate| is_episode_label_marker(candidate.normalized.as_str()))
        || next.is_some_and(|candidate| is_episode_label_marker(candidate.normalized.as_str()))
        || tokens
            .iter()
            .skip(index + 1)
            .take(2)
            .any(|candidate| parse_standard_episode_token(candidate.normalized.as_str()).is_some())
}

fn is_episode_label_marker(token: &str) -> bool {
    matches!(
        token,
        "EP" | "EPS" | "EPISODE" | "EPISODES" | "BOL" | "BLM" | "PART" | "PT"
    )
}

fn finalize_score(state: &ParseState, tokens: &[Token], annotations: &[TokenAnnotations]) -> i32 {
    let mut score = state.score;
    if matches!(state.family, ParseFamily::Movie) && state.title_token_indices.is_empty() {
        score -= 25;
    }
    if matches!(
        state.family,
        ParseFamily::StandardEpisode
            | ParseFamily::DailyEpisode
            | ParseFamily::AnimeAbsolute
            | ParseFamily::SeasonPack
            | ParseFamily::EpisodeRangePack
            | ParseFamily::Special
    ) && matches!(state.identity, ReleaseIdentity::Unknown)
    {
        score -= 45;
    }
    for index in &state.title_token_indices {
        if let Some(annotation) = annotations.get(*index)
            && is_strong_anchor(annotation.primary_role)
        {
            score -= 20;
        }
    }
    let unresolved_strong_anchors = annotations
        .iter()
        .enumerate()
        .filter(|(index, annotation)| {
            is_strong_anchor(annotation.primary_role)
                && !state.consumed_tokens.contains(*index)
                && !state.title_token_mask.contains(*index)
        })
        .count();
    score -= (unresolved_strong_anchors.min(6) as i32) * 10;
    score -= tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            !state.consumed_tokens.contains(*index)
                && !state.title_token_mask.contains(*index)
                && !token.raw.trim().is_empty()
        })
        .count() as i32
        * 5;
    score
}

fn contains_sorted<T: Ord>(values: &[T], needle: &T) -> bool {
    values.binary_search(needle).is_ok()
}

fn sort_and_dedup<T: Ord>(values: &mut Vec<T>) {
    values.sort_unstable();
    values.dedup();
}

fn build_context_index(context: &ReleaseParseContext) -> ContextIndex {
    let mut index = ContextIndex {
        facet_hint: context.facet_hint,
        years: context.known_years.to_vec(),
        ..Default::default()
    };

    if let Some(tokens) = normalized_phrase(context.title.name.as_str()) {
        push_alias_tokens(
            &mut index.aliases,
            context.title.name.as_str(),
            tokens,
            "context:title_canonical_hit",
        );
    }

    for alias in &context.aliases {
        push_alias_entry(&mut index.aliases, alias);
    }
    for imdb_id in &context.imdb_ids {
        if let Some(value) = parse_imdb_id(imdb_id.as_str()) {
            index.aliases.push(AliasEntry {
                raw: value.clone(),
                tokens: vec![value],
                code: "context:title_alias_hit",
            });
        }
    }
    for episode in &context.episodes {
        push_episode_entries(&mut index, episode);
    }

    sort_and_dedup(&mut index.years);
    sort_and_dedup(&mut index.air_dates);
    sort_and_dedup(&mut index.absolute_numbers);

    index
}

fn push_alias_entry(entries: &mut Vec<AliasEntry>, alias: &ContextAlias) {
    if let Some(tokens) = normalized_phrase(alias.name.as_str()) {
        push_alias_tokens(
            entries,
            alias.name.as_str(),
            tokens,
            "context:title_alias_hit",
        );
    }
}

fn push_episode_entries(index: &mut ContextIndex, episode: &ContextEpisode) {
    index.episodes.push(EpisodeContextHint {
        season: episode.season,
        episode: episode.episode,
    });
    if let Some(title) = episode.title.as_deref()
        && let Some(tokens) = normalized_phrase(title)
    {
        index.episode_titles.push(AliasEntry {
            raw: title.to_string(),
            tokens,
            code: "context:episode_title_hit",
        });
    }
    for alias in &episode.title_aliases {
        if let Some(tokens) = normalized_phrase(alias) {
            index.episode_titles.push(AliasEntry {
                raw: alias.clone(),
                tokens,
                code: "context:episode_title_hit",
            });
        }
    }
    if let Some(air_date) = episode.air_date {
        index.air_dates.push(air_date);
    }
    if let Some(absolute_number) = episode.absolute_number {
        index.absolute_numbers.push(absolute_number);
    }
}

fn push_alias_tokens(
    entries: &mut Vec<AliasEntry>,
    raw: &str,
    tokens: Vec<String>,
    code: &'static str,
) {
    push_unique_alias_entry(entries, raw, tokens.clone(), code);
    for variant in merged_alias_token_variants(tokens.as_slice()) {
        push_unique_alias_entry(entries, raw, variant, code);
    }
}

fn push_unique_alias_entry(
    entries: &mut Vec<AliasEntry>,
    raw: &str,
    tokens: Vec<String>,
    code: &'static str,
) {
    if tokens.is_empty() {
        return;
    }
    if entries
        .iter()
        .any(|entry| entry.code == code && entry.tokens == tokens)
    {
        return;
    }
    entries.push(AliasEntry {
        raw: raw.to_string(),
        tokens,
        code,
    });
}

fn merged_alias_token_variants(tokens: &[String]) -> Vec<Vec<String>> {
    let mut variants = Vec::new();
    for index in 0..tokens.len().saturating_sub(1) {
        if tokens[index].len() > 2 && tokens[index + 1].len() > 2 {
            continue;
        }
        let mut variant = Vec::with_capacity(tokens.len().saturating_sub(1));
        variant.extend(tokens[..index].iter().cloned());
        variant.push(format!("{}{}", tokens[index], tokens[index + 1]));
        variant.extend(tokens[index + 2..].iter().cloned());
        variants.push(variant);
    }
    variants
}

fn canonical_context_title(context: &ContextIndex) -> Option<String> {
    context
        .aliases
        .iter()
        .find(|alias| alias.code == "context:title_canonical_hit")
        .map(|alias| alias.tokens.join(" "))
}

fn fallback_title_from_context(
    tokens: &[Token],
    annotations: &[TokenAnnotations],
    context: &ContextIndex,
) -> Option<String> {
    let boundary = tokens
        .iter()
        .enumerate()
        .find(|(index, _)| {
            annotations.get(*index).is_some_and(|annotation| {
                matches!(
                    annotation.primary_role,
                    TokenRole::Year
                        | TokenRole::Quality
                        | TokenRole::Source
                        | TokenRole::StreamingService
                        | TokenRole::VideoCodec
                        | TokenRole::AudioCodec
                        | TokenRole::AudioChannels
                        | TokenRole::EpisodeMarker
                        | TokenRole::SeasonMarker
                        | TokenRole::DateMarker
                        | TokenRole::PackMarker
                        | TokenRole::SpecialMarker
                        | TokenRole::ExternalId
                        | TokenRole::ChecksumOrHash
                )
            })
        })
        .map(|(index, _)| index)
        .unwrap_or(tokens.len());
    let mut seen_ungrouped_prefix = false;
    let window = collapse_aka_tokens(
        tokens
            .iter()
            .take(boundary)
            .filter(|token| {
                if !seen_ungrouped_prefix && token.group_id.is_some() {
                    return false;
                }
                if token.group_id.is_none() {
                    seen_ungrouped_prefix = true;
                }
                !(token.group_id.is_some()
                    && token
                        .raw
                        .chars()
                        .any(|ch| !ch.is_ascii() && ch.is_alphabetic()))
            })
            .map(|token| token.normalized.clone())
            .collect::<Vec<_>>(),
    );
    let mut aliases = context.aliases.iter().collect::<Vec<_>>();
    aliases.sort_by_key(|alias| std::cmp::Reverse(alias.tokens.len()));
    aliases
        .into_iter()
        .find(|alias| {
            !alias.tokens.is_empty()
                && alias.tokens.len() <= window.len()
                && window
                    .windows(alias.tokens.len())
                    .any(|candidate| candidate == alias.tokens)
        })
        .map(|alias| alias.tokens.join(" "))
}

fn context_episode_for_season(context: &ContextIndex, season: u32) -> Option<u32> {
    let mut matched = None;
    for episode in &context.episodes {
        if episode.season != Some(season) {
            continue;
        }
        let Some(candidate) = episode.episode else {
            continue;
        };
        match matched {
            Some(existing) if existing != candidate => return None,
            Some(_) => {}
            None => matched = Some(candidate),
        }
    }
    matched
}

fn alias_automaton_for_patterns(
    patterns: &[AliasPattern],
) -> Result<Arc<AhoCorasick>, aho_corasick::BuildError> {
    let key = patterns
        .iter()
        .map(|pattern| pattern.text.clone())
        .collect::<Vec<_>>();
    if let Some(automaton) = cached_alias_automaton(key.as_slice()) {
        return Ok(automaton);
    }

    let automaton = Arc::new(
        AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::Standard)
            .build(key.iter().map(String::as_str))?,
    );
    remember_alias_automaton(key, Arc::clone(&automaton));
    Ok(automaton)
}

fn alias_automaton_cache() -> &'static Mutex<VecDeque<AliasAutomatonCacheEntry>> {
    static CACHE: OnceLock<Mutex<VecDeque<AliasAutomatonCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn cached_alias_automaton(key: &[String]) -> Option<Arc<AhoCorasick>> {
    let mut cache = alias_automaton_cache().lock().ok()?;
    let position = cache.iter().position(|entry| entry.key == key)?;
    let entry = cache.remove(position)?;
    let automaton = Arc::clone(&entry.automaton);
    cache.push_front(entry);
    Some(automaton)
}

fn remember_alias_automaton(key: Vec<String>, automaton: Arc<AhoCorasick>) {
    let Ok(mut cache) = alias_automaton_cache().lock() else {
        return;
    };
    if let Some(position) = cache.iter().position(|entry| entry.key == key) {
        if let Some(entry) = cache.remove(position) {
            cache.push_front(entry);
        }
        return;
    }
    while cache.len() >= ALIAS_AUTOMATON_CACHE_CAPACITY {
        cache.pop_back();
    }
    cache.push_front(AliasAutomatonCacheEntry { key, automaton });
}

fn build_alias_oracle(tokens: &[Token], context: &ContextIndex) -> AliasOracle {
    let mut patterns = Vec::<AliasPattern>::new();
    for alias in &context.aliases {
        if alias.tokens.is_empty() {
            continue;
        }
        let kind = if alias.code == "context:title_canonical_hit" {
            AliasEvidenceKind::CanonicalTitle
        } else {
            AliasEvidenceKind::TitleAlias
        };
        patterns.push(AliasPattern {
            text: alias.tokens.join(" "),
            kind,
            raw: alias.raw.clone(),
        });
    }
    for alias in &context.episode_titles {
        if alias.tokens.is_empty() {
            continue;
        }
        patterns.push(AliasPattern {
            text: alias.tokens.join(" "),
            kind: AliasEvidenceKind::EpisodeTitle,
            raw: alias.raw.clone(),
        });
    }
    if patterns.is_empty() {
        return AliasOracle::default();
    }

    // Standard + overlapping is intentional here: the beam wants every viable
    // alias span and applies its own precedence and nesting filters afterward.
    let automaton = match alias_automaton_for_patterns(&patterns) {
        Ok(automaton) => automaton,
        Err(_) => {
            return AliasOracle {
                patterns,
                hits_at: vec![AliasHitList::new(); tokens.len()],
                parse_hints: vec!["alias_oracle_construction_failed".to_string()],
            };
        }
    };
    let (haystack, token_byte_map) = join_tokens_with_map(tokens);
    let mut hits_at = vec![AliasHitList::new(); tokens.len()];

    for mat in automaton.find_overlapping_iter(&haystack) {
        let Some(token_range) = byte_range_to_token_range(mat.start(), mat.end(), &token_byte_map)
        else {
            continue;
        };
        let pattern_id = mat.pattern().as_usize();
        let pattern = &patterns[pattern_id];
        if let Some(bucket) = hits_at.get_mut(token_range.start_token) {
            bucket.push(AliasHit {
                token_range,
                pattern_id,
                evidence: pattern.kind,
                score_weight: pattern.kind.score_weight(),
            });
        }
    }

    for hits in hits_at.iter_mut().filter(|hits| !hits.is_empty()) {
        hits.sort_by(|left, right| {
            left.evidence
                .precedence()
                .cmp(&right.evidence.precedence())
                .then(right.token_range.len().cmp(&left.token_range.len()))
                .then(left.pattern_id.cmp(&right.pattern_id))
        });
        let mut filtered = AliasHitList::new();
        for hit in hits.iter() {
            if filtered.iter().any(|accepted| {
                accepted.token_range.start_token <= hit.token_range.start_token
                    && accepted.token_range.end_token >= hit.token_range.end_token
            }) {
                continue;
            }
            filtered.push(hit.clone());
            if filtered.len() == MAX_ALIAS_BRANCH_FANOUT {
                break;
            }
        }
        *hits = filtered;
    }

    AliasOracle {
        patterns,
        hits_at,
        parse_hints: Vec::new(),
    }
}

fn join_tokens_with_map(tokens: &[Token]) -> (String, TokenByteMap) {
    let mut haystack = String::new();
    let mut token_byte_map = TokenByteMap::default();
    token_byte_map.start_to_token.reserve(tokens.len());
    token_byte_map.end_to_token.reserve(tokens.len());
    for (token_index, token) in tokens.iter().enumerate() {
        if !haystack.is_empty() {
            haystack.push(' ');
        }
        let start = haystack.len();
        haystack.push_str(token.normalized.as_str());
        let end = haystack.len();
        token_byte_map.start_to_token.push((start, token_index));
        token_byte_map.end_to_token.push((end, token_index));
    }
    (haystack, token_byte_map)
}

fn byte_range_to_token_range(
    start: usize,
    end: usize,
    token_byte_map: &TokenByteMap,
) -> Option<TokenRange> {
    let token_start = token_byte_map
        .start_to_token
        .binary_search_by_key(&start, |(offset, _)| *offset)
        .ok()
        .and_then(|index| token_byte_map.start_to_token.get(index))
        .map(|(_, token_index)| *token_index)?;
    let token_end = token_byte_map
        .end_to_token
        .binary_search_by_key(&end, |(offset, _)| *offset)
        .ok()
        .and_then(|index| token_byte_map.end_to_token.get(index))
        .map(|(_, token_index)| *token_index + 1)?;
    Some(TokenRange::new(token_start, token_end))
}

fn normalized_phrase(raw: &str) -> Option<Vec<String>> {
    let tokens = collapse_aka_tokens(
        raw.split(|ch: char| {
            matches!(
                ch,
                '.' | '_' | ' ' | '-' | '/' | ':' | '[' | ']' | '(' | ')' | '{' | '}'
            )
        })
        .map(normalize_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>(),
    );
    (!tokens.is_empty()).then_some(tokens)
}

fn collapse_aka_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::with_capacity(tokens.len());
    let mut index = 0usize;
    while index < tokens.len() {
        if tokens.get(index).is_some_and(|token| token == "A")
            && tokens.get(index + 1).is_some_and(|token| token == "K")
            && tokens.get(index + 2).is_some_and(|token| token == "A")
        {
            collapsed.push("AKA".to_string());
            index += 3;
            continue;
        }
        if let Some(token) = tokens.get(index) {
            collapsed.push(token.clone());
        }
        index += 1;
    }
    collapsed
}

fn render_token_indices(tokens: &[Token], indices: &[usize]) -> String {
    indices
        .iter()
        .filter_map(|index| tokens.get(*index))
        .map(|token| token.raw.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_token_range_preserving_separators(tokens: &[Token], range: TokenRange) -> String {
    let mut rendered = String::new();
    for index in range.start_token..range.end_token {
        let Some(token) = tokens.get(index) else {
            continue;
        };
        if index > range.start_token {
            rendered.push_str(separator_text(token.separator_before));
        }
        rendered.push_str(token.raw.as_str());
    }
    rendered
}

fn separator_text(separator: SeparatorKind) -> &'static str {
    match separator {
        SeparatorKind::Dot => ".",
        SeparatorKind::Underscore => "_",
        SeparatorKind::Hyphen => "-",
        SeparatorKind::Slash => "/",
        SeparatorKind::Space => " ",
        _ => "",
    }
}

fn render_identity_raw(identity: &ReleaseIdentity, tokens: &[Token]) -> String {
    match identity {
        ReleaseIdentity::StandardEpisodeIdentity {
            season,
            episode_numbers,
        } => format!(
            "S{:02}E{}",
            season.unwrap_or_default(),
            episode_numbers
                .iter()
                .map(|value| format!("{value:02}"))
                .collect::<Vec<_>>()
                .join("E")
        ),
        ReleaseIdentity::DailyIdentity { air_date, part } => match part {
            Some(part) => format!("{air_date} Part {part}"),
            None => air_date.to_string(),
        },
        ReleaseIdentity::AbsoluteIdentity {
            absolute_episode_numbers,
            version,
            ..
        } => {
            let numbers = absolute_episode_numbers
                .iter()
                .map(|value| format!("{value:02}"))
                .collect::<Vec<_>>()
                .join("-");
            match version {
                Some(version) => format!("{numbers}v{version}"),
                None => numbers,
            }
        }
        ReleaseIdentity::SeasonPackIdentity { seasons, .. } => {
            match (seasons.first(), seasons.last()) {
                (Some(first), Some(last)) if first != last => {
                    format!("S{first:02}-S{last:02}")
                }
                (Some(first), _) => format!("S{first:02}"),
                _ => String::new(),
            }
        }
        ReleaseIdentity::RangePackIdentity {
            season,
            range_start,
            range_end,
        } => season.map_or_else(
            || format!("{range_start}-{range_end}"),
            |season| format!("S{season:02}E{range_start:02}-E{range_end:02}"),
        ),
        ReleaseIdentity::SpecialIdentity { special_kind, .. } => match special_kind {
            ParsedSpecialKind::Special => "SPECIAL".to_string(),
            ParsedSpecialKind::Ova => "OVA".to_string(),
            ParsedSpecialKind::Oad => "OAD".to_string(),
            ParsedSpecialKind::Ncop => "NCOP".to_string(),
            ParsedSpecialKind::Nced => "NCED".to_string(),
            ParsedSpecialKind::Extra => "EXTRA".to_string(),
        },
        ReleaseIdentity::MovieIdentity | ReleaseIdentity::Unknown => {
            render_token_indices(tokens, &[])
        }
    }
}

fn reason(code: &str, delta: i32, detail: Option<String>) -> ParseReason {
    ParseReason {
        code: code.to_string(),
        delta,
        detail,
    }
}

fn apply_role_usage_bonus(
    state: &mut ParseState,
    annotation: Option<&TokenAnnotations>,
    expected_role: TokenRole,
) {
    let Some(annotation) = annotation else {
        return;
    };
    let delta = if annotation.primary_role == expected_role {
        ALT_ROLE_PRIMARY_DEBT
    } else if annotation
        .alternate_roles
        .first()
        .is_some_and(|role| *role == expected_role)
    {
        ALT_ROLE_FIRST_DEBT
    } else if annotation
        .alternate_roles
        .get(1)
        .is_some_and(|role| *role == expected_role)
    {
        ALT_ROLE_SECOND_DEBT
    } else if annotation.may_be_title_word {
        TITLE_WORD_AMBIGUITY_DEBT
    } else {
        0
    };
    state.score += delta;
}

fn is_title_like_token(token: &Token) -> bool {
    token.raw.chars().any(|ch| ch.is_alphabetic()) && !token.normalized.is_empty()
}

fn is_strong_anchor(role: TokenRole) -> bool {
    matches!(
        role,
        TokenRole::Quality
            | TokenRole::Source
            | TokenRole::VideoCodec
            | TokenRole::AudioCodec
            | TokenRole::ExternalId
            | TokenRole::ChecksumOrHash
    )
}

fn parse_standard_episode_token(token: &str) -> Option<(Option<u32>, Vec<u32>)> {
    if token.starts_with('S') && token.contains('E') {
        let e_index = token.find('E')?;
        let season = token.get(1..e_index)?.parse::<u32>().ok();
        let suffix = token.get(e_index + 1..)?;
        let episode_numbers = parse_standard_episode_suffix(suffix)
            .or_else(|| parse_fused_standard_episode_suffix(suffix))?;
        return (!episode_numbers.is_empty()).then_some((season, episode_numbers));
    }
    if let Some(suffix) = token.strip_prefix('E') {
        let episode_numbers = parse_standard_episode_suffix(suffix)
            .or_else(|| parse_fused_standard_episode_suffix(suffix))?;
        return (!episode_numbers.is_empty()).then_some((Some(1), episode_numbers));
    }
    if let Some((season, suffix)) = token.split_once('X') {
        if season.is_empty() || suffix.is_empty() {
            return None;
        }
        let season = season.parse::<u32>().ok()?;
        let episode_numbers = parse_standard_episode_suffix(suffix)
            .or_else(|| parse_fused_standard_episode_suffix(suffix))?;
        return (!episode_numbers.is_empty()).then_some((Some(season), episode_numbers));
    }
    None
}

fn has_explicit_single_standard_episode_marker(tokens: &[Token]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            parse_standard_episode_token(&token.normalized),
            Some((Some(_), episode_numbers)) if episode_numbers.len() == 1
        )
    })
}

fn parse_season_keyword_episode_at(
    tokens: &[Token],
    index: usize,
) -> Option<(Option<u32>, Vec<u32>, Vec<usize>)> {
    let token = tokens.get(index)?.normalized.as_str();
    if token != "SEASON" {
        return None;
    }
    let season = tokens.get(index + 1)?.normalized.parse::<u32>().ok()?;
    let episode_token = tokens.get(index + 2)?;
    let episode = if episode_token.separator_before == SeparatorKind::Hyphen {
        parse_numeric_token(episode_token.normalized.as_str())?
    } else {
        dot_split_episode_after_season(tokens, index + 2)?
    };
    if let Some(range_end) = split_episode_range_end(tokens, index + 3, episode) {
        return Some((
            Some(season),
            (episode..=range_end).collect(),
            vec![index, index + 1, index + 2, index + 3],
        ));
    }
    Some((
        Some(season),
        vec![episode],
        vec![index, index + 1, index + 2],
    ))
}

fn parse_standard_episode_suffix(suffix: &str) -> Option<Vec<u32>> {
    if let Some((start, end)) = suffix.split_once('-') {
        let range_start = parse_episode_component(start)?;
        let range_end = parse_episode_component(end)?;
        if range_end < range_start {
            return None;
        }
        return Some((range_start..=range_end).collect());
    }

    let mut episode_numbers = Vec::new();
    for part in suffix.split('E') {
        let parsed = parse_episode_component(part)?;
        episode_numbers.push(parsed);
    }
    (!episode_numbers.is_empty()).then_some(episode_numbers)
}

fn parse_episode_component(component: &str) -> Option<u32> {
    component
        .strip_prefix('E')
        .unwrap_or(component)
        .parse::<u32>()
        .ok()
}

fn parse_fused_standard_episode_suffix(suffix: &str) -> Option<Vec<u32>> {
    let stripped = strip_trailing_quality_suffix(suffix)?;
    parse_standard_episode_suffix(stripped)
}

fn strip_trailing_quality_suffix(value: &str) -> Option<&str> {
    trailing_quality_suffix_start(value).and_then(|split_at| value.get(..split_at))
}

fn trailing_quality_suffix(value: &str) -> Option<String> {
    let split_at = trailing_quality_suffix_start(value)?;
    parse_resolution_quality_token(value.get(split_at..)?)
}

fn trailing_quality_suffix_start(value: &str) -> Option<usize> {
    for suffix_len in [5usize, 4usize] {
        if value.len() <= suffix_len {
            continue;
        }
        let split_at = value.len() - suffix_len;
        let suffix = value.get(split_at..)?;
        if parse_resolution_quality_token(suffix).is_some() {
            return Some(split_at);
        }
    }
    None
}

fn parse_split_standard_episode_at(
    tokens: &[Token],
    index: usize,
) -> Option<(Option<u32>, Vec<u32>, Vec<usize>)> {
    let season = parse_season_token(tokens.get(index)?.normalized.as_str())?;
    let episode_token = tokens.get(index + 1)?;
    let episode = if episode_token.separator_before == SeparatorKind::Hyphen {
        parse_numeric_token(episode_token.normalized.as_str())?
    } else {
        dot_split_episode_after_season(tokens, index + 1)?
    };
    if let Some(range_end) = split_episode_range_end(tokens, index + 2, episode) {
        return Some((
            Some(season),
            (episode..=range_end).collect(),
            vec![index, index + 1, index + 2],
        ));
    }
    Some((Some(season), vec![episode], vec![index, index + 1]))
}

/// Hyphen continuation after a split episode (`S3.01-02`, `Season 1 - 001-020`):
/// a 1-3 digit numeric strictly above the range start extends it to a
/// multi-episode span.
fn split_episode_range_end(tokens: &[Token], index: usize, range_start: u32) -> Option<u32> {
    let token = tokens.get(index)?;
    if token.separator_before != SeparatorKind::Hyphen {
        return None;
    }
    let digits = token.normalized.as_str();
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u32>()
        .ok()
        .filter(|range_end| *range_end > range_start)
}

/// Accept `S<N>.<EE>` (including `S<N>.-.<EE>`, whose separator chain merges
/// to `Dot`) only for zero-padded episode tokens: two digits, or three digits
/// with a leading zero. That shape excludes bare resolutions (`S02.720`),
/// years (`S02.2024`), and dotted numeric chains, so true season packs like
/// `S02.1080p` keep parsing as packs.
fn dot_split_episode_after_season(tokens: &[Token], episode_index: usize) -> Option<u32> {
    let episode_token = tokens.get(episode_index)?;
    if episode_token.separator_before != SeparatorKind::Dot {
        return None;
    }
    let digits = episode_token.normalized.as_str();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let zero_padded = digits.len() == 2 || (digits.len() == 3 && digits.starts_with('0'));
    if !zero_padded {
        return None;
    }
    if tokens.get(episode_index + 1).is_some_and(|next| {
        next.separator_before == SeparatorKind::Dot
            && !next.normalized.is_empty()
            && next.normalized.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return None;
    }
    // Split color-depth markers ("S02.10.bit.x265") are metadata, not episodes.
    if tokens
        .get(episode_index + 1)
        .is_some_and(|next| matches!(next.normalized.as_str(), "BIT" | "BITS"))
    {
        return None;
    }
    digits.parse::<u32>().ok().filter(|episode| *episode > 0)
}

fn parse_parenthetical_standard_after_absolute_at(
    tokens: &[Token],
    index: usize,
) -> Option<(Option<u32>, Vec<u32>, Vec<usize>)> {
    let absolute_token = tokens.get(index)?;
    parse_numeric_token(absolute_token.normalized.as_str())?;

    for candidate_index in (index + 1)..(index + 4).min(tokens.len()) {
        let candidate = tokens.get(candidate_index)?;
        if candidate.bracket_depth == 0 && candidate.separator_before != SeparatorKind::OpenParen {
            continue;
        }
        if let Some((season, episode_numbers)) =
            parse_standard_episode_token(candidate.normalized.as_str())
        {
            return Some((season, episode_numbers, vec![index, candidate_index]));
        }
    }

    None
}

fn parse_hyphenated_standard_episode_range_at(
    tokens: &[Token],
    index: usize,
) -> Option<(Option<u32>, Vec<u32>, usize)> {
    let (season, episode_numbers) =
        parse_standard_episode_token(tokens.get(index)?.normalized.as_str())?;
    if episode_numbers.len() != 1 {
        return Some((season, episode_numbers, index));
    }
    let next = tokens.get(index + 1)?;
    if next.separator_before != SeparatorKind::Hyphen {
        return Some((season, episode_numbers, index));
    }
    let range_end = parse_episode_component(next.normalized.as_str())?;
    let range_start = *episode_numbers.first()?;
    if range_end < range_start {
        return Some((season, episode_numbers, index));
    }
    Some((season, (range_start..=range_end).collect(), index + 1))
}

fn parse_season_token(token: &str) -> Option<u32> {
    // "SEASON" must be tried before 'S': every SEASON-prefixed token also
    // starts with 'S', so the reverse order leaves fused `Season1` unparsed.
    if let Some(value) = token.strip_prefix("SEASON") {
        return value
            .parse::<u32>()
            .ok()
            .filter(|season| (1..=100).contains(season));
    }
    token
        .strip_prefix('S')
        .and_then(|value| value.parse::<u32>().ok())
}

struct SeasonPackParse {
    seasons: Vec<u32>,
    consumed: Vec<usize>,
    is_partial: bool,
    season_part: Option<u32>,
    is_series_pack: bool,
}

fn series_pack(seasons: Vec<u32>, consumed: Vec<usize>) -> Option<SeasonPackParse> {
    let mut seasons = seasons;
    seasons.sort_unstable();
    seasons.dedup();
    (seasons.len() > 1).then_some(SeasonPackParse {
        seasons,
        consumed,
        is_partial: false,
        season_part: None,
        is_series_pack: true,
    })
}

/// Parse the compact no-whitespace season forms that the lexer keeps inside one
/// token, such as `S01+S02+OVAs`.
fn parse_compact_series_seasons(token: &str) -> Option<Vec<u32>> {
    let mut seasons = Vec::new();

    for component in token.split('+') {
        let component_seasons = if let Some(season) = parse_season_token(component) {
            Some(vec![season])
        } else if let Some((start, end)) = component.split_once('-') {
            match (parse_season_token(start), parse_season_token(end)) {
                (Some(start), Some(end)) if end > start => Some((start..=end).collect()),
                _ => None,
            }
        } else {
            None
        };

        let Some(component_seasons) = component_seasons else {
            break;
        };
        seasons.extend(component_seasons);
    }

    series_pack(seasons, vec![0]).map(|pack| pack.seasons)
}

/// Recognize only the empirically observed high-signal series-pack markers.
/// Bare episode batches and a bare `COMPLETE` are deliberately not enough.
fn parse_series_pack_at(tokens: &[Token], index: usize) -> Option<SeasonPackParse> {
    // An explicit `SxxEyy` names one episode, even when its episode title
    // happens to contain a whole-series marker. Keep batches and ranges out of
    // this guard: those still deliberately describe pack-shaped releases.
    if has_explicit_single_standard_episode_marker(tokens) {
        return None;
    }
    let token = tokens.get(index)?.normalized.as_str();

    if token == "COMPLETE" {
        let marker = tokens.get(index + 1).map(|token| token.normalized.as_str());
        let complete_series = marker.is_some_and(|value| value.split('+').next() == Some("SERIES"));
        let complete_tv_series = marker == Some("TV")
            && tokens
                .get(index + 2)
                .is_some_and(|token| token.normalized.split('+').next() == Some("SERIES"));
        if complete_series || complete_tv_series {
            let end = if complete_tv_series { 2 } else { 1 };
            return Some(SeasonPackParse {
                seasons: Vec::new(),
                consumed: (index..=index + end).collect(),
                is_partial: false,
                season_part: None,
                is_series_pack: true,
            });
        }
    }

    if let Some(seasons) = parse_compact_series_seasons(token) {
        return series_pack(seasons, vec![index]);
    }

    let (first, next, second, consumed) = if token == "SEASONS" {
        let first = tokens
            .get(index + 1)
            .and_then(|token| parse_numeric_token(&token.normalized))?;
        let next_index = index + 2;
        let next = tokens.get(next_index)?;
        let second = parse_numeric_token(&next.normalized)?;
        (first, next, second, vec![index, index + 1, next_index])
    } else {
        let first = parse_season_token(token)?;
        let next_index = index + 1;
        let next = tokens.get(next_index)?;
        let second = parse_season_token(&next.normalized)?;
        (first, next, second, vec![index, next_index])
    };
    if !matches!(
        next.separator_before,
        SeparatorKind::Hyphen | SeparatorKind::Other
    ) {
        return None;
    }
    let seasons = if next.separator_before == SeparatorKind::Hyphen && second > first {
        (first..=second).collect()
    } else {
        vec![first, second]
    };
    series_pack(seasons, consumed)
}

struct RangePackParse {
    season: Option<u32>,
    range_start: u32,
    range_end: u32,
    consumed: Vec<usize>,
}

fn parse_season_pack_at(tokens: &[Token], index: usize) -> Option<SeasonPackParse> {
    if let Some(series_pack) = parse_series_pack_at(tokens, index) {
        return Some(series_pack);
    }

    let token = tokens.get(index)?.normalized.as_str();
    if token == "SEASON" {
        if season_scoped_range_after(tokens, index).is_some()
            && !has_unlabeled_parenthetical_range_after(tokens, index + 2)
        {
            return None;
        }
        if (tokens.get(index + 2).is_some_and(|candidate| {
            candidate.separator_before == SeparatorKind::Hyphen
                && parse_numeric_token(candidate.normalized.as_str()).is_some()
        }) || dot_split_episode_after_season(tokens, index + 2).is_some())
            && !has_batch_marker_around(tokens, index)
        {
            return None;
        }
        let season = tokens.get(index + 1)?.normalized.parse::<u32>().ok()?;
        let mut consumed = vec![index, index + 1];
        let part_parse = parse_part_markers_after(tokens, index + 2);
        let (is_partial, season_part) = match part_parse {
            Some(PartMarkerParse {
                parts,
                consumed: part_consumed,
            }) if part_sequence_is_full_season(parts.as_slice()) => {
                if part_marker_scan_can_consume(tokens, index + 2, part_consumed.as_slice()) {
                    consumed.extend(part_consumed);
                }
                (false, None)
            }
            Some(PartMarkerParse {
                parts,
                consumed: part_consumed,
            }) => {
                if part_marker_scan_can_consume(tokens, index + 2, part_consumed.as_slice()) {
                    consumed.extend(part_consumed);
                }
                (true, parts.last().copied())
            }
            None => (false, None),
        };
        return Some(SeasonPackParse {
            seasons: vec![season],
            consumed,
            is_partial,
            season_part,
            is_series_pack: false,
        });
    }
    if let Some(season) = parse_season_token(token) {
        if season_scoped_range_after(tokens, index).is_some()
            && !has_unlabeled_parenthetical_range_after(tokens, index + 1)
        {
            return None;
        }
        if let Some(next) = tokens.get(index + 1)
            && next.separator_before == SeparatorKind::Hyphen
            && let Some(end_season) = parse_season_token(next.normalized.as_str())
            && end_season > season
            && !has_explicit_single_standard_episode_marker(tokens)
        {
            return Some(SeasonPackParse {
                seasons: (season..=end_season).collect(),
                consumed: vec![index, index + 1],
                is_partial: false,
                season_part: None,
                is_series_pack: true,
            });
        }
        if tokens.get(index + 1).is_some_and(|token| {
            token.separator_before == SeparatorKind::Hyphen
                && parse_numeric_token(token.normalized.as_str()).is_some()
        }) || dot_split_episode_after_season(tokens, index + 1).is_some()
        {
            return None;
        }
        if let Some(PartMarkerParse { parts, consumed }) =
            parse_part_markers_after(tokens, index + 1)
        {
            let mut identity_consumed = vec![index];
            if part_marker_scan_can_consume(tokens, index + 1, consumed.as_slice()) {
                identity_consumed.extend(consumed);
            }
            return Some(SeasonPackParse {
                seasons: vec![season],
                consumed: identity_consumed,
                is_partial: !part_sequence_is_full_season(parts.as_slice()),
                season_part: (!part_sequence_is_full_season(parts.as_slice()))
                    .then(|| parts.last().copied())
                    .flatten(),
                is_series_pack: false,
            });
        }
        return Some(SeasonPackParse {
            seasons: vec![season],
            consumed: vec![index],
            is_partial: false,
            season_part: None,
            is_series_pack: false,
        });
    }
    None
}

fn has_unlabeled_parenthetical_range_after(tokens: &[Token], index: usize) -> bool {
    let end = (index + 8).min(tokens.len());
    for start_index in index..end {
        let Some(start_token) = tokens.get(start_index) else {
            continue;
        };
        if start_token.separator_before != SeparatorKind::OpenParen {
            continue;
        }
        let Some(range_start) = parse_numeric_token(start_token.normalized.as_str()) else {
            continue;
        };
        let Some(end_token) = tokens.get(start_index + 1) else {
            continue;
        };
        if end_token.separator_before != SeparatorKind::Hyphen {
            continue;
        }
        let Some(range_end) = parse_numeric_token(end_token.normalized.as_str()) else {
            continue;
        };
        if range_end <= range_start {
            continue;
        }
        if start_index
            .checked_sub(1)
            .and_then(|previous| tokens.get(previous))
            .is_some_and(|token| is_episode_label_marker(token.normalized.as_str()))
        {
            continue;
        }
        return true;
    }
    false
}

fn part_marker_scan_can_consume(tokens: &[Token], scan_start: usize, consumed: &[usize]) -> bool {
    let Some(first_part_token) = consumed.iter().min().copied() else {
        return false;
    };
    !(scan_start..first_part_token).any(|index| {
        tokens
            .get(index)
            .is_some_and(|token| is_late_part_scan_boundary(token.normalized.as_str()))
    })
}

fn is_late_part_scan_boundary(token: &str) -> bool {
    matches!(
        token,
        "WEB"
            | "WEBDL"
            | "WEBRIP"
            | "BLURAY"
            | "BD"
            | "BDRIP"
            | "BRRIP"
            | "BDREMUX"
            | "HDTV"
            | "DVDRIP"
            | "DVD"
            | "AAC"
            | "DD"
            | "DDP"
            | "EAC3"
            | "AC3"
            | "DTS"
            | "DTSHD"
            | "DTSMA"
            | "DTSX"
            | "TRUEHD"
            | "FLAC"
            | "OPUS"
            | "MP3"
            | "PCM"
            | "LPCM"
            | "AVC"
            | "HEVC"
            | "H264"
            | "H265"
            | "H266"
            | "X264"
            | "X265"
            | "XVID"
            | "VVC"
            | "VC1"
            | "MPEG2"
            | "MULTI"
            | "MULTISUB"
            | "DUAL"
            | "DUALAUDIO"
    ) || parse_year(token).is_some()
        || normalize_standalone_streaming_service(token).is_some()
        || is_compound_metadata_like_token(token)
}

struct PartMarkerParse {
    parts: Vec<u32>,
    consumed: Vec<usize>,
}

fn parse_part_markers_after(tokens: &[Token], index: usize) -> Option<PartMarkerParse> {
    let mut cursor = index;
    let end = (index + 24).min(tokens.len());
    let mut parts = Vec::new();
    let mut consumed = Vec::new();

    while cursor < end {
        let Some(token) = tokens.get(cursor) else {
            break;
        };
        let normalized = token.normalized.as_str();
        if normalized.is_empty() || matches!(normalized, "+" | "AND") {
            consumed.push(cursor);
            cursor += 1;
            continue;
        }
        let Some((part, part_consumed)) = parse_part_marker_at(tokens, cursor) else {
            if parts.is_empty() {
                cursor += 1;
                continue;
            }
            break;
        };
        parts.push(part);
        consumed.extend(part_consumed);
        cursor = consumed.last().copied().unwrap_or(cursor) + 1;
    }

    (!parts.is_empty()).then_some(PartMarkerParse { parts, consumed })
}

fn parse_part_marker_at(tokens: &[Token], index: usize) -> Option<(u32, Vec<usize>)> {
    let token = tokens.get(index)?.normalized.as_str();
    if matches!(token, "PART" | "PT") {
        let value = parse_numeric_token(tokens.get(index + 1)?.normalized.as_str())?;
        return Some((value, vec![index, index + 1]));
    }
    if let Some(value) = token
        .strip_prefix("PART")
        .or_else(|| token.strip_prefix("PAR"))
        .or_else(|| token.strip_prefix("PT"))
        .and_then(parse_numeric_token)
    {
        return Some((value, vec![index]));
    }
    None
}

fn part_sequence_is_full_season(parts: &[u32]) -> bool {
    parts.contains(&1) && parts.iter().any(|part| *part > 1)
}

fn season_scoped_range_after(tokens: &[Token], index: usize) -> Option<RangePackParse> {
    let season = parse_season_marker_at(tokens, index)?;
    let end = (index + 18).min(tokens.len());
    for candidate_index in index + 1..end {
        if let Some(mut range) = parse_labeled_range_pack_at(tokens, candidate_index) {
            range.season = Some(season);
            let mut consumed = vec![index];
            if tokens
                .get(index)
                .is_some_and(|token| token.normalized == "SEASON")
            {
                consumed.push(index + 1);
            }
            consumed.extend(range.consumed);
            range.consumed = consumed;
            return Some(range);
        }
    }
    None
}

fn range_directly_follows_season_marker(tokens: &[Token], index: usize) -> bool {
    index
        .checked_sub(1)
        .is_some_and(|previous| parse_season_marker_at(tokens, previous).is_some())
        || index.checked_sub(2).is_some_and(|previous| {
            tokens
                .get(previous)
                .is_some_and(|token| token.normalized == "SEASON")
                && parse_season_marker_at(tokens, previous).is_some()
        })
}

fn parse_season_marker_at(tokens: &[Token], index: usize) -> Option<u32> {
    let token = tokens.get(index)?.normalized.as_str();
    if token == "SEASON" {
        return parse_numeric_token(tokens.get(index + 1)?.normalized.as_str());
    }
    parse_season_token(token)
}

fn parse_daily_at(tokens: &[Token], index: usize) -> Option<(NaiveDate, Vec<usize>, Option<u32>)> {
    let first = tokens.get(index)?;
    if first.bracket_depth == 0
        && let Some(date) = parse_fused_daily_date(first.normalized.as_str())
    {
        return Some(daily_with_part(tokens, date, vec![index], index + 1));
    }
    let second = tokens.get(index + 1)?;
    let third = tokens.get(index + 2)?;
    if !matches!(
        (second.separator_before, third.separator_before),
        (SeparatorKind::Dot, SeparatorKind::Dot) | (SeparatorKind::Hyphen, SeparatorKind::Hyphen)
    ) {
        return None;
    }

    let numeric = |token: &Token| token.normalized.parse::<u32>().ok();
    let year_of = |token: &Token| {
        token
            .normalized
            .parse::<i32>()
            .ok()
            .filter(|year| (1900..=2099).contains(year))
    };
    let month_name = |token: &Token| month_from_name(token.normalized.as_str());

    let date = if let (Some(year), Some(month), Some(day)) =
        (year_of(first), numeric(second), numeric(third))
    {
        NaiveDate::from_ymd_opt(year, month, day)
    } else if let (Some(day), Some(month), Some(year)) =
        (numeric(first), numeric(second), year_of(third))
    {
        NaiveDate::from_ymd_opt(year, month, day)
    } else if let (Some(year), Some(month), Some(day)) =
        (year_of(first), month_name(second), numeric(third))
    {
        NaiveDate::from_ymd_opt(year, month, day)
    } else if let (Some(day), Some(month), Some(year)) =
        (numeric(first), month_name(second), year_of(third))
    {
        NaiveDate::from_ymd_opt(year, month, day)
    } else {
        None
    }?;

    Some(daily_with_part(
        tokens,
        date,
        vec![index, index + 1, index + 2],
        index + 3,
    ))
}

fn daily_with_part(
    tokens: &[Token],
    date: NaiveDate,
    mut consumed: Vec<usize>,
    part_index: usize,
) -> (NaiveDate, Vec<usize>, Option<u32>) {
    let part = tokens
        .get(part_index + 1)
        .filter(|_| {
            tokens
                .get(part_index)
                .is_some_and(|token| token.normalized == "PART")
        })
        .and_then(|token| token.normalized.parse::<u32>().ok());
    if part.is_some() {
        consumed.push(part_index);
        consumed.push(part_index + 1);
    }
    (date, consumed, part)
}

fn parse_fused_daily_date(token: &str) -> Option<NaiveDate> {
    if token.len() != 8 || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = token.get(..4)?.parse::<i32>().ok()?;
    if !(1900..=2099).contains(&year) {
        return None;
    }
    let month = token.get(4..6)?.parse::<u32>().ok()?;
    let day = token.get(6..8)?.parse::<u32>().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn month_from_name(token: &str) -> Option<u32> {
    match token {
        "JAN" | "JANUARY" => Some(1),
        "FEB" | "FEBRUARY" => Some(2),
        "MAR" | "MARCH" => Some(3),
        "APR" | "APRIL" => Some(4),
        "MAY" => Some(5),
        "JUN" | "JUNE" => Some(6),
        "JUL" | "JULY" => Some(7),
        "AUG" | "AUGUST" => Some(8),
        "SEP" | "SEPT" | "SEPTEMBER" => Some(9),
        "OCT" | "OCTOBER" => Some(10),
        "NOV" | "NOVEMBER" => Some(11),
        "DEC" | "DECEMBER" => Some(12),
        _ => None,
    }
}

fn parse_standard_episode_range_pack_at(tokens: &[Token], index: usize) -> Option<RangePackParse> {
    if let Some((season, episode_numbers)) =
        parse_standard_episode_token(tokens.get(index)?.normalized.as_str())
        && episode_numbers.len() > 1
    {
        return Some(RangePackParse {
            season,
            range_start: *episode_numbers.first()?,
            range_end: *episode_numbers.last()?,
            consumed: vec![index],
        });
    }
    if let Some((season, episode_numbers, last_token)) =
        parse_hyphenated_standard_episode_range_at(tokens, index)
        && episode_numbers.len() > 1
    {
        return Some(RangePackParse {
            season,
            range_start: *episode_numbers.first()?,
            range_end: *episode_numbers.last()?,
            consumed: (index..=last_token).collect(),
        });
    }
    None
}

fn parse_range_pack_at(tokens: &[Token], index: usize) -> Option<RangePackParse> {
    let start = parse_numeric_token(tokens.get(index)?.normalized.as_str())?;
    let end = parse_numeric_token(tokens.get(index + 1)?.normalized.as_str())?;
    if is_unlabeled_parenthetical_season_annotation(tokens, index) {
        return None;
    }
    let separator = tokens.get(index + 1)?.separator_before;
    let same_group = tokens.get(index)?.group_id.is_some()
        && tokens.get(index)?.group_id == tokens.get(index + 1)?.group_id;
    if (separator == SeparatorKind::Hyphen || same_group) && end > start {
        return Some(RangePackParse {
            season: None,
            range_start: start,
            range_end: end,
            consumed: vec![index, index + 1],
        });
    }
    None
}

fn is_unlabeled_parenthetical_season_annotation(tokens: &[Token], index: usize) -> bool {
    let Some(start_token) = tokens.get(index) else {
        return false;
    };
    if start_token.separator_before != SeparatorKind::OpenParen {
        return false;
    }
    if index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| is_episode_label_marker(token.normalized.as_str()))
    {
        return false;
    }
    let start = index.saturating_sub(4);
    (start..index).rev().any(|candidate_index| {
        tokens
            .get(candidate_index)
            .is_some_and(|_| parse_season_marker_at(tokens, candidate_index).is_some())
    })
}

fn parse_labeled_range_pack_at(tokens: &[Token], index: usize) -> Option<RangePackParse> {
    let token = tokens.get(index)?.normalized.as_str();
    if !matches!(token, "EP" | "EPS" | "EPISODE" | "EPISODES") {
        return None;
    }
    let mut range = parse_range_pack_at(tokens, index + 1)?;
    range.season = preceding_season_hint(tokens, index);
    range.consumed = [vec![index], range.consumed].concat();
    Some(range)
}

fn parse_batch_season_range_at(tokens: &[Token], index: usize) -> Option<RangePackParse> {
    season_scoped_range_after(tokens, index).filter(|_| has_batch_marker_around(tokens, index))
}

fn preceding_season_hint(tokens: &[Token], index: usize) -> Option<u32> {
    let start = index.saturating_sub(16);
    (start..index).rev().find_map(|candidate_index| {
        let token = tokens.get(candidate_index)?;
        if token.normalized == "SEASON" {
            return parse_numeric_token(tokens.get(candidate_index + 1)?.normalized.as_str());
        }
        parse_season_token(token.normalized.as_str())
    })
}

fn has_batch_marker_around(tokens: &[Token], index: usize) -> bool {
    let start = index.saturating_sub(4);
    let end = (index + 4).min(tokens.len().saturating_sub(1));
    tokens[start..=end]
        .iter()
        .any(|token| matches!(token.normalized.as_str(), "COMPLETE" | "BATCH"))
}

fn parse_anime_absolute_at(
    tokens: &[Token],
    index: usize,
    context: &ContextIndex,
) -> Option<(Vec<u32>, Option<u32>)> {
    if parse_range_pack_at(tokens, index).is_some() {
        return None;
    }
    if tokens
        .get(index)
        .is_some_and(|token| token.separator_before == SeparatorKind::Hyphen)
        && tokens
            .get(index.saturating_sub(1))
            .is_some_and(|token| parse_numeric_token(token.normalized.as_str()).is_some())
        && !previous_numeric_token_is_context_title(tokens, index, context)
    {
        return None;
    }
    if tokens
        .get(index.saturating_sub(1))
        .is_some_and(|token| parse_numeric_token(token.normalized.as_str()).is_some())
        && tokens.get(index).and_then(|token| token.group_id).is_some()
        && tokens.get(index).and_then(|token| token.group_id)
            == tokens
                .get(index.saturating_sub(1))
                .and_then(|token| token.group_id)
    {
        return None;
    }
    if tokens
        .get(index + 1)
        .is_some_and(|token| token.separator_before == SeparatorKind::Hyphen)
        && tokens
            .get(index + 1)
            .is_some_and(|token| parse_numeric_token(token.normalized.as_str()).is_some())
    {
        return None;
    }
    if tokens
        .get(index)
        .is_some_and(|token| token.separator_before == SeparatorKind::Hyphen)
        && tokens
            .get(index.saturating_sub(1))
            .is_some_and(|token| parse_season_token(token.normalized.as_str()).is_some())
    {
        return None;
    }
    if index >= 1
        && tokens
            .get(index - 1)
            .is_some_and(|token| parse_season_token(token.normalized.as_str()).is_some())
        && dot_split_episode_after_season(tokens, index).is_some()
    {
        return None;
    }
    if tokens
        .get(index.saturating_sub(1))
        .is_some_and(|token| token.normalized == "SEASON")
    {
        return None;
    }
    let token = tokens.get(index)?.normalized.as_str();
    if let Some(number) = parse_numeric_token(token) {
        let is_anime_cued = tokens.first().is_some_and(|token| token.group_id.is_some())
            || tokens
                .iter()
                .any(|candidate| candidate.normalized == "ANIME")
            || context.facet_hint == ContextFacetHint::Anime;
        let contextual_match = contains_sorted(&context.absolute_numbers, &number);
        if (is_anime_cued || contextual_match)
            && number <= 2000
            && (contextual_match || absolute_companion_allowed(tokens, index, number))
        {
            return Some((vec![number], None));
        }
    }
    if let Some((number, version)) = parse_versioned_absolute(token) {
        return Some((vec![number], Some(version)));
    }
    None
}

fn previous_numeric_token_is_context_title(
    tokens: &[Token],
    index: usize,
    context: &ContextIndex,
) -> bool {
    let Some(previous_index) = index.checked_sub(1) else {
        return false;
    };
    let Some(previous) = tokens.get(previous_index) else {
        return false;
    };
    if parse_numeric_token(previous.normalized.as_str()).is_none() {
        return false;
    }
    context
        .aliases
        .iter()
        .any(|alias| alias.tokens.len() == 1 && alias.tokens[0] == previous.normalized)
}

fn parse_special_kind(token: &str) -> Option<ParsedSpecialKind> {
    match token {
        "OVA" => Some(ParsedSpecialKind::Ova),
        "OAD" => Some(ParsedSpecialKind::Oad),
        "NCOP" => Some(ParsedSpecialKind::Ncop),
        "NCED" => Some(ParsedSpecialKind::Nced),
        "SPECIAL" => Some(ParsedSpecialKind::Special),
        "EXTRA" => Some(ParsedSpecialKind::Extra),
        _ => None,
    }
}

fn parse_resolution_quality_token(token: &str) -> Option<String> {
    const COMMON_RESOLUTIONS: &[u32] = &[
        240, 360, 480, 540, 576, 720, 864, 900, 960, 1080, 1200, 1440, 1600, 2160, 2880, 4320, 8640,
    ];
    let suffix = token
        .strip_suffix('P')
        .map(|value| (value, 'p'))
        .or_else(|| token.strip_suffix('I').map(|value| (value, 'i')))?;
    let (digits, scan_type) = suffix;
    if !(3..=4).contains(&digits.len()) || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let resolution = digits.parse::<u32>().ok()?;
    COMMON_RESOLUTIONS
        .contains(&resolution)
        .then(|| format!("{resolution}{scan_type}"))
}

fn parse_external_id_at(tokens: &[Token], index: usize) -> Option<(ParsedExternalId, TokenRange)> {
    let token = tokens.get(index)?.normalized.as_str();
    if let Some(external_id) = parse_external_id_token(token) {
        return Some((
            external_id,
            TokenRange {
                start_token: index,
                end_token: index + 1,
            },
        ));
    }

    let source = external_id_source_from_label(token)?;
    let next = tokens.get(index + 1)?;
    let value = external_id_value_for_source(source, next.normalized.as_str())?;
    Some((
        ParsedExternalId { source, value },
        TokenRange {
            start_token: index,
            end_token: index + 2,
        },
    ))
}

fn parse_external_id_token(token: &str) -> Option<ParsedExternalId> {
    if let Some(value) = parse_imdb_id(token) {
        return Some(ParsedExternalId {
            source: ExternalIdSource::Imdb,
            value,
        });
    }
    for source in [ExternalIdSource::Tmdb, ExternalIdSource::Tvdb] {
        let source_label = source.as_str().to_ascii_uppercase();
        for prefix in [source_label.clone(), format!("{source_label}ID")] {
            let Some(value) = token.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()) {
                return Some(ParsedExternalId {
                    source,
                    value: value.to_string(),
                });
            }
        }
    }
    None
}

fn is_external_id_label(token: &str) -> bool {
    external_id_source_from_label(token).is_some()
}

fn external_id_source_from_label(token: &str) -> Option<ExternalIdSource> {
    match token {
        "IMDB" | "IMDBID" => Some(ExternalIdSource::Imdb),
        "TMDB" | "TMDBID" => Some(ExternalIdSource::Tmdb),
        "TVDB" | "TVDBID" => Some(ExternalIdSource::Tvdb),
        _ => None,
    }
}

fn external_id_value_for_source(source: ExternalIdSource, token: &str) -> Option<String> {
    match source {
        ExternalIdSource::Imdb => parse_imdb_id(token).or_else(|| {
            token
                .chars()
                .all(|ch| ch.is_ascii_digit())
                .then(|| format!("tt{token}"))
        }),
        ExternalIdSource::Tmdb | ExternalIdSource::Tvdb => (!token.is_empty()
            && token.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| token.to_string()),
        ExternalIdSource::AniDb
        | ExternalIdSource::AniDbEpisode
        | ExternalIdSource::AniList
        | ExternalIdSource::Mal => None,
    }
}

fn push_unique_external_id(ids: &mut Vec<ParsedExternalId>, id: ParsedExternalId) {
    if !ids.iter().any(|existing| {
        existing.source == id.source && existing.value.eq_ignore_ascii_case(&id.value)
    }) {
        ids.push(id);
    }
}

fn parse_imdb_id(token: &str) -> Option<String> {
    let stripped = token.strip_prefix("TT")?;
    (!stripped.is_empty() && stripped.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("tt{stripped}"))
}

fn parse_year(token: &str) -> Option<i32> {
    let year = token.parse::<i32>().ok()?;
    (1900..=2099).contains(&year).then_some(year)
}

fn parse_numeric_token(token: &str) -> Option<u32> {
    (!token.is_empty() && token.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| token.parse::<u32>().ok())
        .flatten()
}

fn parse_version(token: &str) -> Option<u32> {
    token.strip_prefix('V')?.parse::<u32>().ok()
}

fn parse_versioned_absolute(token: &str) -> Option<(u32, u32)> {
    let (number, version) = token.split_once('V')?;
    Some((number.parse::<u32>().ok()?, version.parse::<u32>().ok()?))
}

fn is_checksum(token: &str) -> bool {
    token.len() == 8 && token.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn normalize_channels(raw: &str) -> String {
    match raw {
        "20" => "2.0".to_string(),
        "51" => "5.1".to_string(),
        "71" => "7.1".to_string(),
        "2CH" => "2.0".to_string(),
        _ => raw.to_string(),
    }
}

fn coalesce_source(tokens: &[Token], index: usize) -> String {
    let Some(token) = tokens.get(index) else {
        return String::new();
    };
    if let Some(next) = tokens.get(index + 1).filter(|candidate| {
        token.normalized == "WEB"
            && candidate.separator_before == SeparatorKind::Hyphen
            && matches!(candidate.normalized.as_str(), "DL" | "RIP")
    }) {
        return format!("WEB-{}", next.raw);
    }
    if token.normalized == "WEB" {
        return "WEB-DL".to_string();
    }
    if token.normalized == "DVD" {
        return "DVD".to_string();
    }
    if let Some(normalized) = normalize_source_token(token.normalized.as_str()) {
        return normalized.to_string();
    }
    token.raw.clone()
}

/// Display name for a token that already holds the streaming-service role.
fn normalize_streaming_service(token: &str) -> Option<&'static str> {
    crate::trash_guides::normalize_streaming_service_alias(token)
}

/// Service detection for callers that see one token and no neighbors.
///
/// Restricted to standalone aliases: a WEB-adjacent alias such as `NOW` or `RED`
/// cannot be resolved without the neighbor, and guessing that it names a service
/// would let a common title word flip a structural predicate.
fn normalize_standalone_streaming_service(token: &str) -> Option<&'static str> {
    crate::trash_guides::normalize_streaming_service_alias_standalone(token)
}

/// Service detection at the role-assignment site, which has the next token.
fn normalize_streaming_service_with_neighbor(
    token: &str,
    next: Option<&Token>,
) -> Option<&'static str> {
    let web_adjacent = next.is_some_and(|next| is_web_marker_token(next.normalized.as_str()));
    crate::trash_guides::normalize_streaming_service_alias_in_context(token, web_adjacent)
}

/// The WEB markers upstream's adjacency patterns accept. Bare `WEB` counts:
/// their shape is `token[ ._-]web[ ._-]?(dl|rip)?`, where the suffix is optional.
fn is_web_marker_token(normalized: &str) -> bool {
    matches!(normalized, "WEB" | "WEBDL" | "WEBRIP")
}

fn detect_compound_metadata(token: &str) -> CompoundMetadata {
    let mut metadata = CompoundMetadata {
        quality: detect_compound_quality(token),
        ..Default::default()
    };

    if contains_any(token, &["WEBRIP", "WEBRI"]) {
        metadata.source = Some("WEBRip");
    } else if token.starts_with("WEB") {
        // Prefix, not substring: "COBWEB"-style title words must not read as
        // WEB-DL sources.
        metadata.source = Some("WEB-DL");
    } else if contains_any(token, &["BDMV", "BDISO", "BRDISK"]) {
        metadata.source = Some("BRDISK");
    } else if contains_any(token, &["BLURAY", "BDREMUX", "BDRIO", "BDRIP", "BRRIP"])
        || matches!(token, "BLU" | "BD")
    {
        metadata.source = Some("BluRay");
    } else if token.contains("HDTV") {
        metadata.source = Some("HDTV");
    } else if token == "CAM"
        || token.starts_with("CAMRIP")
        || token.starts_with("HDCAM")
        || token.starts_with("HQCAM")
    {
        // Bare "CAM" must match exactly: it is a substring of ordinary title
        // words ("BECAME", "CAMERA", "CAMP") that flow through metadata
        // consumption after an episode identity. The longer forms are
        // unambiguous and may be fused with other metadata.
        metadata.source = Some("CAM");
    } else if token.contains("TELESYNC") || token == "TS" {
        metadata.source = Some("TELESYNC");
    } else if token.contains("TELECINE") || token == "TC" {
        metadata.source = Some("TELECINE");
    } else if token.contains("DVDSCR") {
        metadata.source = Some("DVDSCR");
    } else if token.contains("WORKPRINT") {
        metadata.source = Some("WORKPRINT");
    } else if token.contains("DVDRIP") || token == "DVD" {
        metadata.source = Some("DVD");
    }

    if token.contains("H266") || token.contains("VVC") {
        metadata.video_codec = Some("VVC");
    } else if token.contains("X265") || token.contains("H265") || token.contains("HEVC") {
        metadata.video_codec = Some("H.265");
    } else if contains_any(token, &["X264", "H264", "AVC1", "AVC"]) {
        metadata.video_codec = Some("H.264");
    } else if token.contains("AV1") {
        metadata.video_codec = Some("AV1");
    } else if token.contains("VP9") {
        metadata.video_codec = Some("VP9");
    } else if token.contains("VC1") {
        metadata.video_codec = Some("VC1");
    } else if token.contains("MPEG2") {
        metadata.video_codec = Some("MPEG2");
    } else if token.contains("XVID") {
        metadata.video_codec = Some("XVID");
    } else if token.contains("DIVX") {
        metadata.video_codec = Some("DIVX");
    }

    if contains_any(token, &["DDP", "DD+"]) {
        metadata.audio_codec = Some("DDP");
    } else if contains_any(token, &["EAC3", "EC3"]) {
        metadata.audio_codec = Some("EAC3");
    } else if contains_any(token, &["AC3", "AC-3"]) {
        metadata.audio_codec = Some("AC3");
    } else if token.contains("AAC") {
        metadata.audio_codec = Some("AAC");
    } else if token.contains("TRUEHD") {
        metadata.audio_codec = Some("TRUEHD");
    } else if contains_any(token, &["DTSMA", "DTSHDMA"]) {
        metadata.audio_codec = Some("DTSMA");
    } else if contains_any(token, &["DTSX", "DTS-X"]) {
        metadata.audio_codec = Some("DTSX");
    } else if contains_any(token, &["DTSHD", "DTS-HD"]) {
        metadata.audio_codec = Some("DTSHD");
    } else if token.contains("DTS") {
        metadata.audio_codec = Some("DTS");
    } else if token.contains("FLAC") {
        metadata.audio_codec = Some("FLAC");
    } else if token.contains("OPUS") {
        metadata.audio_codec = Some("OPUS");
    } else if token.contains("VORBIS") {
        metadata.audio_codec = Some("VORBIS");
    } else if token.contains("MP3") {
        metadata.audio_codec = Some("MP3");
    } else if token.contains("LPCM") || token.contains("PCM") {
        metadata.audio_codec = Some("PCM");
    }

    // These substring probes are deliberately loose ("2019" contains "20") and
    // are only safe because of two invariants elsewhere: role annotation ranks
    // Year (95) and Quality (100) above AudioChannels (88), so digit-bearing
    // year/resolution tokens never take the channels role, and
    // `consume_unit_metadata` only accepts an AudioChannels role when
    // `audio_channel_has_audio_context` sees an audio codec within 3 tokens.
    // Tighten those guards before loosening anything here.
    if token.contains("71") || token.contains("7CH") || token.contains("8CH") {
        metadata.audio_channels = Some("7.1");
    } else if token.contains("51") || token.contains("6CH") {
        metadata.audio_channels = Some("5.1");
    } else if token.contains("20") || token.contains("2CH") {
        metadata.audio_channels = Some("2.0");
    }

    metadata
}

fn detect_compound_quality(token: &str) -> Option<String> {
    parse_resolution_quality_token(token).or_else(|| {
        [
            8640u32, 4320, 2160, 1440, 1080, 864, 720, 576, 540, 480, 360, 240,
        ]
        .into_iter()
        .find(|resolution| token.contains(&resolution.to_string()))
        .map(|resolution| format!("{resolution}p"))
    })
}

fn contains_any(token: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| token.contains(needle))
}

fn normalize_source_token(token: &str) -> Option<&'static str> {
    match token {
        "WEB" | "WEBDL" => Some("WEB-DL"),
        "WEBRIP" => Some("WEBRip"),
        "BDMV" | "BDISO" | "BRDISK" => Some("BRDISK"),
        "BLURAY" | "BD" | "BDRIP" | "BDRIO" | "BRRIP" | "BDREMUX" | "BLU" => Some("BluRay"),
        "DVD" | "DVDRIP" => Some("DVD"),
        "HDTV" => Some("HDTV"),
        "CAM" | "HQCAM" => Some("CAM"),
        "TELESYNC" | "TS" => Some("TELESYNC"),
        "TELECINE" | "TC" => Some("TELECINE"),
        "DVDSCR" => Some("DVDSCR"),
        "WORKPRINT" => Some("WORKPRINT"),
        _ => None,
    }
}
