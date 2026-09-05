//! The grab side of the loop: find candidates, order them, decide one.
//!
//! ```text
//! Found → Parsed → Scored(announced) → Ranked → Decided → Grabbed
//! ```
//!
//! Each arrow has exactly one owner, and none of them is here:
//! [`crate::canonical_scoring`] scores, [`crate::acquisition::scoring`] ranks,
//! [`crate::admission`] decides. This module runs the sequence and records the
//! reason code, so a lane cannot acquire its own opinion about what a release
//! is worth.
//!
//! Two consequences of that split are worth stating, because they are what the
//! module is careful about:
//!
//! - **Listing metadata orders, it never scores.** Release age,
//!   indexer priority, seeders, votes and coverage preference decide which
//!   candidate is looked at first and never enter a number that gets persisted
//!   or compared across time — none of them can be reconstructed from a media
//!   row.
//! - **Whatever this lane grabs, import will accept.** The admission call
//!   here is the same function the import gate calls, over the same subject
//!   builder, under a *stricter* policy — so a release that clears the grab
//!   cannot be refused at import on the same facts.

use super::acquisition::{
    collection_download_submission_scope_for_wanted_item,
    direct_download_submission_scope_for_wanted_item,
};
use super::*;
use crate::acquisition_search_queries::{
    anidb_id_from_external_ids, build_movie_search_queries, build_search_queries,
    imdb_id_from_title, mal_id_from_external_ids, movie_text_search_query,
    tmdb_id_from_external_ids, tvdb_id_from_external_ids,
};
use crate::delay_profile::DelayProfile;
use crate::quality::release_parser::ParseDisposition;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// Lookup keys that cannot identify a canonical title on their own. This
/// includes keys shared with another library title and the bare form of an
/// explicitly year-qualified canonical title such as `Tide Chart (2023)`.
#[derive(Clone, Debug, Default)]
pub(crate) struct TitleIdentityAmbiguity {
    pub(crate) shared_lookup_keys: Vec<String>,
}

impl TitleIdentityAmbiguity {
    pub(crate) fn from_shared_keys(shared_lookup_keys: Vec<String>) -> Self {
        Self { shared_lookup_keys }
    }

    fn from_year_qualified_canonical_key(canonical_key: &str, title_year: Option<i32>) -> Self {
        let Some(title_year) = title_year else {
            return Self::default();
        };
        let stripped = crate::import_title_resolution::strip_trailing_year_key(canonical_key);
        let suffix_matches_title_year = canonical_key
            .rsplit_once(' ')
            .is_some_and(|(_, suffix)| suffix == title_year.to_string());
        if suffix_matches_title_year && stripped != canonical_key && !stripped.is_empty() {
            return Self::from_shared_keys(vec![canonical_key.to_string()]);
        }
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        for key in other.shared_lookup_keys {
            if !self.shared_lookup_keys.contains(&key) {
                self.shared_lookup_keys.push(key);
            }
        }
    }

    /// True when an auto candidate must present a positive disambiguator.
    pub(crate) fn requires_disambiguator(&self) -> bool {
        !self.shared_lookup_keys.is_empty()
    }

    /// True when `key` is an alias only this title claims within the library
    /// collision set — the "unique alias hit" disambiguator.
    pub(crate) fn key_is_unique_to_title(&self, key: &str) -> bool {
        !self
            .shared_lookup_keys
            .iter()
            .any(|shared| shared.as_str() == key)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalTitleEvidence {
    pub(crate) lookup_keys: Vec<String>,
    pub(crate) canonical_key: String,
    pub(crate) year: Option<i32>,
    /// The requested episode's air year is direct season-scoped evidence.
    pub(crate) episode_release_years: HashSet<i32>,
    /// Explicit trailing years keyed by the distinct alias that carries them.
    /// A year only relaxes the veto when this exact alias matched.
    pub(crate) alias_release_years: HashMap<String, i32>,
    pub(crate) parse_context: crate::ReleaseParseContext,
    /// Library-local collision data. Defaults to "not ambiguous" so every
    /// existing construction site keeps its behavior; the resolution paths
    /// attach real data through [`CanonicalTitleEvidence::with_ambiguity`].
    pub(crate) ambiguity: TitleIdentityAmbiguity,
}

impl CanonicalTitleEvidence {
    pub(crate) fn with_ambiguity(mut self, ambiguity: TitleIdentityAmbiguity) -> Self {
        self.ambiguity.merge(ambiguity);
        self
    }
}

/// How a parsed release name matched a canonical title, retained so the identity
/// check can tell a shared bare key from a unique alias.
#[derive(Clone, Debug)]
pub(crate) struct TitleEvidenceMatch {
    /// The canonical lookup key that actually matched.
    pub(crate) matched_key: String,
    /// The release carries the title's year.
    pub(crate) year_corroborated: bool,
    /// A one-word alias is too weak to establish identity without an external
    /// id (or the title year, represented by `year_corroborated`).
    pub(crate) requires_external_id: bool,
}

/// A candidate's title match proven from the raw release title.
#[derive(Clone, Debug)]
pub(crate) struct CandidateTitleMatch {
    pub(crate) evidence_match: Option<TitleEvidenceMatch>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedReleaseSearchSubject {
    pub(crate) title_id: String,
    pub(crate) title_tags: Vec<String>,
    pub(crate) title_evidence: CanonicalTitleEvidence,
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: String,
    pub(crate) owner_facet: MediaFacet,
    pub(crate) search_facet: MediaFacet,
    pub(crate) id_search_facet: Option<MediaFacet>,
    pub(crate) newznab_categories: Vec<String>,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) absolute_episode: Option<u32>,
    pub(crate) subject_kind: ReleaseSearchSubjectKind,
    pub(crate) last_search_at: Option<String>,
    pub(crate) submission_scope: SubmissionScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseAutoDecisionCode {
    Eligible,
    ParseAmbiguous,
    ParseUnparseable,
    TitleMismatch,
    EpisodeMismatch,
    EpisodeNotMonitored,
    CategoryMismatch,
    AmbiguousIdentity,
    QualityBlocked,
    NegativeScore,
    UpgradeRejected,
    CutoffReached,
    ProperForOldFile,
    AlreadyActive,
    QueuedBetterOrEqual,
    DbBlocklisted,
    PendingDelay,
    MinimumAge,
    ReleaseAgeUnknown,
    ProtocolDisabled,
    DownloadClientUnavailable,
    RepackGroupMismatch,
    MinimumSeeders,
    PackBelowMissingThreshold,
    SubtitlesOnly,
}

impl ReleaseAutoDecisionCode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "eligible" => Some(Self::Eligible),
            "parse_ambiguous" => Some(Self::ParseAmbiguous),
            "parse_unparseable" => Some(Self::ParseUnparseable),
            "title_mismatch" => Some(Self::TitleMismatch),
            "episode_mismatch" => Some(Self::EpisodeMismatch),
            "episode_not_monitored" => Some(Self::EpisodeNotMonitored),
            // Deliberately the same string the pre-submission gate records on
            // failed attempts, so both category vetoes read alike.
            "category_mismatch" => Some(Self::CategoryMismatch),
            "ambiguous_identity" => Some(Self::AmbiguousIdentity),
            "quality_blocked" => Some(Self::QualityBlocked),
            "negative_score" => Some(Self::NegativeScore),
            "upgrade_rejected" => Some(Self::UpgradeRejected),
            "cutoff_reached" => Some(Self::CutoffReached),
            "proper_for_old_file" => Some(Self::ProperForOldFile),
            "already_active" => Some(Self::AlreadyActive),
            "queued_better_or_equal" => Some(Self::QueuedBetterOrEqual),
            "db_blocklisted" => Some(Self::DbBlocklisted),
            "pending_delay" => Some(Self::PendingDelay),
            "minimum_age" => Some(Self::MinimumAge),
            "release_age_unknown" => Some(Self::ReleaseAgeUnknown),
            "protocol_disabled" => Some(Self::ProtocolDisabled),
            "download_client_unavailable" => Some(Self::DownloadClientUnavailable),
            "repack_group_mismatch" => Some(Self::RepackGroupMismatch),
            "minimum_seeders" => Some(Self::MinimumSeeders),
            "pack_below_missing_threshold" => Some(Self::PackBelowMissingThreshold),
            "subtitles_only" => Some(Self::SubtitlesOnly),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::ParseAmbiguous => "parse_ambiguous",
            Self::ParseUnparseable => "parse_unparseable",
            Self::TitleMismatch => "title_mismatch",
            Self::EpisodeMismatch => "episode_mismatch",
            Self::EpisodeNotMonitored => "episode_not_monitored",
            Self::CategoryMismatch => "category_mismatch",
            Self::AmbiguousIdentity => "ambiguous_identity",
            Self::QualityBlocked => "quality_blocked",
            Self::NegativeScore => "negative_score",
            Self::UpgradeRejected => "upgrade_rejected",
            Self::CutoffReached => "cutoff_reached",
            Self::ProperForOldFile => "proper_for_old_file",
            Self::AlreadyActive => "already_active",
            Self::QueuedBetterOrEqual => "queued_better_or_equal",
            Self::DbBlocklisted => "db_blocklisted",
            Self::PendingDelay => "pending_delay",
            Self::MinimumAge => "minimum_age",
            Self::ReleaseAgeUnknown => "release_age_unknown",
            Self::ProtocolDisabled => "protocol_disabled",
            Self::DownloadClientUnavailable => "download_client_unavailable",
            Self::RepackGroupMismatch => "repack_group_mismatch",
            Self::MinimumSeeders => "minimum_seeders",
            Self::PackBelowMissingThreshold => "pack_below_missing_threshold",
            Self::SubtitlesOnly => "subtitles_only",
        }
    }

    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Eligible => "auto search would grab this release",
            Self::ParseAmbiguous => "release parse is ambiguous and blocks auto-grab",
            Self::ParseUnparseable => "release could not be parsed and blocks auto-grab",
            Self::TitleMismatch => "release title does not match the target title",
            Self::EpisodeMismatch => "release numbering does not match the target episode",
            // Distinct from `EpisodeMismatch` on purpose: "the numbering does
            // not match" and "one of these episodes is unmonitored" call for
            // different operator actions.
            Self::EpisodeNotMonitored => "release covers an episode this library is not monitoring",
            Self::CategoryMismatch => "indexer category contradicts the target title",
            Self::AmbiguousIdentity => {
                "canonical title is ambiguous and no disambiguator was present"
            }
            Self::QualityBlocked => "quality profile blocked this release",
            Self::NegativeScore => {
                "release score is negative after scoring penalties (no longer emitted)"
            }
            Self::UpgradeRejected => "upgrade policy rejected this release",
            Self::CutoffReached => "existing file already meets the configured cutoff",
            Self::ProperForOldFile => {
                "the existing file is too old to be worth replacing with a PROPER"
            }
            Self::AlreadyActive => "release is already active or covered in the queue",
            // Deliberately distinct from `AlreadyActive`: "the same release is
            // downloading" and "a different, better release is downloading" are
            // different operator situations.
            Self::QueuedBetterOrEqual => {
                "a release already downloading for this scope is equal or better"
            }
            Self::DbBlocklisted => "release is blocklisted from prior failures",
            Self::PendingDelay => "release is eligible but held by a delay profile",
            Self::MinimumAge => "release has not reached the configured usenet minimum age",
            Self::ReleaseAgeUnknown => "release age is unavailable for an active age gate",
            Self::ProtocolDisabled => "release protocol is disabled by the delay profile",
            Self::DownloadClientUnavailable => "matching download clients are unavailable",
            Self::RepackGroupMismatch => "repack group does not match the existing file",
            Self::MinimumSeeders => "too few seeders for this indexer's seeding profile",
            Self::PackBelowMissingThreshold => {
                "series pack does not meet the missing-episode threshold"
            }
            Self::SubtitlesOnly => "release carries subtitles only and no video",
        }
    }

    pub(crate) fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }
}

#[derive(Clone)]
pub(crate) struct AutoCandidateEvaluationContext<'a> {
    pub(crate) title: &'a Title,
    pub(crate) subject: &'a ResolvedReleaseSearchSubject,
    /// The primary files occupying this scope, each with a canonical bar.
    /// Replaces a ledger score that could be null, stale, or the grab-time score
    /// of a release that never landed.
    pub(crate) admission: &'a crate::admission::AdmissionSubject,
    pub(crate) last_search_at: Option<&'a str>,
    pub(crate) profile: &'a QualityProfile,
    pub(crate) thresholds: &'a AcquisitionThresholds,
    /// The scope's best incumbent has reached the profile's cutoff — the
    /// candidate-independent half of Sonarr's `QualityCutoffNotMet`. The
    /// candidate-dependent half (a same-tier revision upgrade escapes it) lives
    /// in [`cutoff_refusal`], so there is one cutoff gate rather than a
    /// scope-level short-circuit in every lane.
    pub(crate) incumbent_at_cutoff: bool,
    /// This evaluation is a feed pass rather than an active search, which is the
    /// condition Sonarr's `ProperSpecification` keys on
    /// (`if (information.SearchCriteria != null) return Accept()`). The old-file
    /// guard binds only here.
    pub(crate) is_rss_lane: bool,
    /// Interactive searches bypass protocol delay while retaining hard minimum
    /// age and permanent diagnostics.
    pub(crate) user_invoked: bool,
    /// Publication timestamp of the oldest active pending release overlapping
    /// this scope. Populated only by the RSS merge lane.
    pub(crate) oldest_overlapping_pending_published_at: Option<DateTime<Utc>>,
    pub(crate) now: &'a DateTime<Utc>,
    pub(crate) dl_snapshot: Option<&'a crate::acquisition_workflow::DownloadClientSnapshot>,
    pub(crate) db_blocklist: &'a crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
    pub(crate) existing_files: &'a [TitleMediaFile],
    pub(crate) delay_profiles: &'a [DelayProfile],
    pub(crate) failed_routes: Option<&'a [crate::acquisition_workflow::DownloadRouteKey]>,
    /// Admission threshold per indexer id, resolved once for the batch.
    /// An indexer absent from the map is treated as unrestricted.
    pub(crate) minimum_seeders: &'a HashMap<String, i32>,
    /// Episodes of this title nobody is monitoring, resolved once per scope.
    /// Empty for a title with no episodes.
    pub(crate) unmonitored_episode_ids: &'a HashSet<String>,
}

pub fn release_strategy_kind_for_label(label: &str, is_rss_request: bool) -> ReleaseStrategyKind {
    if is_rss_request {
        return ReleaseStrategyKind::RssFeed;
    }

    if label.starts_with("ids") {
        return ReleaseStrategyKind::IdBacked;
    }

    match label {
        "freetext" | "freetext_alias" => ReleaseStrategyKind::Freetext,
        _ => ReleaseStrategyKind::Fallback,
    }
}

pub(crate) fn canonical_title_lookup_keys(title: &Title) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();

    for candidate in std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
    {
        let normalized = crate::title_matching::canonical_lookup_key(candidate);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            keys.push(normalized);
        }
    }

    keys
}

pub(crate) fn canonical_title_evidence(title: &Title) -> CanonicalTitleEvidence {
    canonical_title_evidence_for_episode(title, None)
}

fn canonical_title_evidence_for_episode(
    title: &Title,
    episode: Option<&Episode>,
) -> CanonicalTitleEvidence {
    let lookup_keys = canonical_title_lookup_keys(title);
    let canonical_key = crate::title_matching::canonical_lookup_key(&title.name);
    let mut alias_release_years = HashMap::new();
    let canonical_shape = crate::import_title_resolution::strip_trailing_year_key(&canonical_key);
    for alias in title
        .aliases
        .iter()
        .map(String::as_str)
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
    {
        let alias_key = crate::title_matching::canonical_lookup_key(alias);
        let alias_shape = crate::import_title_resolution::strip_trailing_year_key(&alias_key);
        let explicit_year = alias_key
            .split_whitespace()
            .next_back()
            .filter(|token| token.len() == 4)
            .and_then(|token| token.parse::<i32>().ok())
            .filter(|year| (1900..=2099).contains(year));
        if alias_shape != alias_key
            && alias_shape != canonical_shape
            && let Some(year) = explicit_year
        {
            alias_release_years.insert(alias_key, year);
        }
    }
    let episode_release_years = episode
        .and_then(|episode| episode.air_date.as_deref())
        .and_then(|air_date| air_date.get(..4))
        .and_then(|year| year.parse::<i32>().ok())
        .into_iter()
        .collect();
    let mut parse_context =
        crate::build_release_parse_context(title, episode, None, Some(title.facet.as_str()));
    if title.year.is_some() {
        let stripped_key = crate::import_title_resolution::strip_trailing_year_key(&canonical_key);
        if stripped_key != canonical_key
            && !stripped_key.is_empty()
            && !parse_context.aliases.iter().any(|alias| {
                crate::title_matching::canonical_lookup_key(&alias.name) == stripped_key
            })
        {
            parse_context
                .aliases
                .push(crate::release_parser::ContextAlias {
                    name: stripped_key.to_string(),
                });
        }
    }

    let ambiguity =
        TitleIdentityAmbiguity::from_year_qualified_canonical_key(&canonical_key, title.year);
    CanonicalTitleEvidence {
        lookup_keys,
        canonical_key,
        year: title.year,
        episode_release_years,
        alias_release_years,
        parse_context,
        ambiguity,
    }
}

pub(crate) fn series_movie_search_title(
    title: &Title,
    link: &scryer_domain::SeriesMovieLink,
) -> Title {
    let movie = &link.movie;
    let mut search_title = title.clone();
    search_title.name = movie.title.clone();
    search_title.facet = MediaFacet::Movie;
    search_title.year = movie.year;
    search_title.imdb_id = movie.imdb_id.clone();
    search_title.runtime_minutes = movie.runtime_minutes;
    search_title.external_ids.retain(|external_id| {
        !matches!(
            external_id.source.trim().to_ascii_lowercase().as_str(),
            "imdb" | "tvdb" | "tmdb" | "anidb" | "mal"
        )
    });
    if let Some(imdb_id) = movie
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: imdb_id,
        });
    }
    if let Some(tvdb_id) = movie
        .tvdb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.clone(),
        });
    }
    if let Some(tmdb_id) = movie
        .tmdb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tmdb".to_string(),
            value: tmdb_id.clone(),
        });
    }
    if let Some(anidb_id) = movie
        .anidb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "anidb".to_string(),
            value: anidb_id.clone(),
        });
    }
    if let Some(mal_id) = movie
        .mal_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "mal".to_string(),
            value: mal_id.clone(),
        });
    }
    search_title.aliases = series_movie_search_aliases(&search_title);
    search_title.tagged_aliases = search_title
        .aliases
        .iter()
        .map(|alias| scryer_domain::TaggedAlias {
            name: alias.clone(),
            language: "und".to_string(),
        })
        .collect();
    search_title
}

fn series_movie_search_aliases(search_title: &Title) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();
    let primary_key = crate::title_matching::canonical_lookup_key(&search_title.name);
    push_series_movie_alias_evidence(&mut aliases, &mut seen, search_title.name.clone());

    for candidate in crate::title_matching::search_variants(&search_title.name) {
        push_series_movie_alias(&mut aliases, &mut seen, &primary_key, candidate);
    }
    let reduced = crate::title_matching::reduced_comparison_key(
        &search_title.name,
        crate::title_matching::TitleMatchProfile::Movie,
    );
    push_series_movie_alias(&mut aliases, &mut seen, &primary_key, reduced);

    let parsed = crate::parse_release_metadata_for_target(
        &search_title.name,
        &crate::build_release_parse_context(search_title, None, None, Some("movie")),
    );
    let mut variants = parsed.normalized_title_variants.clone();
    if !variants
        .iter()
        .any(|title| title.eq_ignore_ascii_case(&parsed.normalized_title))
    {
        variants.push(parsed.normalized_title);
    }
    for variant in variants {
        push_series_movie_alias(&mut aliases, &mut seen, &primary_key, variant);
    }

    aliases
}

fn push_series_movie_alias(
    aliases: &mut Vec<String>,
    seen: &mut HashSet<String>,
    primary_key: &str,
    alias: String,
) {
    let alias = alias.split_whitespace().collect::<Vec<_>>().join(" ");
    if alias.is_empty() {
        return;
    }
    let key = crate::title_matching::canonical_lookup_key(&alias);
    if !key.is_empty() && key != primary_key && seen.insert(key) {
        aliases.push(alias);
    }
}

fn push_series_movie_alias_evidence(
    aliases: &mut Vec<String>,
    seen: &mut HashSet<String>,
    alias: String,
) {
    let alias = alias.split_whitespace().collect::<Vec<_>>().join(" ");
    if alias.is_empty() {
        return;
    }
    let key = crate::title_matching::canonical_lookup_key(&alias);
    if !key.is_empty() && seen.insert(key) {
        aliases.push(alias);
    }
}

fn media_facet_from_str(value: &str) -> Option<MediaFacet> {
    match value.trim().to_ascii_lowercase().as_str() {
        "movie" => Some(MediaFacet::Movie),
        "series" => Some(MediaFacet::Series),
        "anime" => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn owner_facet_for_wanted_item(title: &Title, item: &AcquisitionScopeState) -> MediaFacet {
    item.title_facet
        .as_deref()
        .and_then(media_facet_from_str)
        .unwrap_or_else(|| title.facet.clone())
}

fn series_movie_newznab_categories(owner_facet: &MediaFacet) -> Vec<String> {
    let mut categories = vec!["2000".to_string()];
    if matches!(owner_facet, MediaFacet::Anime) {
        categories.push("5070".to_string());
    }
    categories
}

pub(crate) fn parsed_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    match_parsed_release_to_title_evidence(parsed, evidence).is_some()
}

fn push_anchor_key(keys: &mut Vec<String>, key: &str) {
    let key = key.trim();
    if !key.is_empty() && !keys.iter().any(|existing| existing == key) {
        keys.push(key.to_string());
    }
}

/// Pass 1 of the identity proof: canonical keys the *context-free* parse
/// extracts from a release name, before any target bias is applied.
///
/// A target-biased parse can project the target title out of a longer raw
/// span (`Electric Bloom` projecting the `BLOOM` alias), so bias may only
/// refine an identity that unbiased extraction already supports. Besides the
/// neutral title and its variants, two principled near-miss forms anchor too:
/// a leading known release-group run stripped (`Erai-raws.Title...`), and the
/// halves of an `AKA` dual-titled name.
pub(crate) fn context_free_identity_anchor_keys(raw_title: &str) -> Vec<String> {
    let neutral = crate::parse_release_metadata(raw_title);
    let mut extracted = neutral.normalized_title_variants.clone();
    extracted.push(neutral.normalized_title.clone());

    // Year tokens in the raw name re-attach to the extraction: a boundary
    // heuristic reads `Signal.Runner.2049.2160p` as title `Signal Runner`, but
    // the subject's key is `signal runner 2049`.
    let mut year_tokens = Vec::<String>::new();
    for digits in raw_title.split(|ch: char| !ch.is_ascii_digit()) {
        if digits.len() == 4
            && digits
                .parse::<i32>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
            && !year_tokens.iter().any(|existing| existing == digits)
        {
            year_tokens.push(digits.to_string());
        }
    }

    let mut keys = Vec::<String>::new();
    for title in extracted {
        let key = crate::title_matching::canonical_lookup_key(&title);
        if key.is_empty() {
            continue;
        }
        push_anchor_key(&mut keys, &key);
        for year in &year_tokens {
            if !key.ends_with(year.as_str()) {
                push_anchor_key(&mut keys, &format!("{key} {year}"));
            }
        }

        // `Title AKA Other Title` names both subjects; each half anchors.
        if key.contains(" aka ") {
            for half in key.split(" aka ") {
                push_anchor_key(&mut keys, half);
            }
        }

        // An unbracketed leading group tag reads as title text to a neutral
        // parse. Only a run the release-group database recognizes may be
        // elided — an unknown prefix stays, so containment junk cannot anchor.
        let tokens = key.split_whitespace().collect::<Vec<_>>();
        for dropped in 1..=tokens.len().saturating_sub(1).min(3) {
            let prefix = &tokens[..dropped];
            if [prefix.join("-"), prefix.join(" ")]
                .iter()
                .any(|candidate| crate::release_group_db::is_known_release_group(candidate))
            {
                push_anchor_key(&mut keys, &tokens[dropped..].join(" "));
            }
        }
    }

    keys
}

/// The one key-comparison rule shared by the anchor gate and the contextual
/// confirm loop: a normalized string names the title when it equals a lookup
/// key outright, or equals a key with the title's own year elided.
pub(crate) fn evidence_key_for_normalized(
    evidence: &CanonicalTitleEvidence,
    normalized: &str,
) -> Option<String> {
    evidence
        .lookup_keys
        .iter()
        .find(|key| {
            key.as_str() == normalized
                || evidence
                    .year
                    .is_some_and(|year| key.strip_suffix(&format!(" {year}")) == Some(normalized))
        })
        .cloned()
}

/// Stacked-alias anchor: fansub names often glue two alias forms of the same
/// subject together (`Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01`), so a
/// neutral parse extracts one long title no single key equals. The extraction
/// still anchors when it decomposes *completely* into two or three distinct
/// lookup keys of this title — full coverage, so containment junk (extra words
/// that are no key of the subject) can never satisfy it.
fn extraction_decomposes_into_evidence_keys(
    evidence: &CanonicalTitleEvidence,
    extracted_key: &str,
) -> bool {
    const MAX_STACKED_SEGMENTS: usize = 3;

    fn covers(
        evidence: &CanonicalTitleEvidence,
        tokens: &[&str],
        start: usize,
        used: &mut Vec<String>,
    ) -> bool {
        if start == tokens.len() {
            return used.len() >= 2;
        }
        if used.len() >= MAX_STACKED_SEGMENTS {
            return false;
        }
        for end in start + 1..=tokens.len() {
            let segment = tokens[start..end].join(" ");
            let Some(matched_key) = evidence_key_for_normalized(evidence, &segment) else {
                continue;
            };
            if used.contains(&matched_key) {
                continue;
            }
            used.push(matched_key);
            if covers(evidence, tokens, end, used) {
                return true;
            }
            used.pop();
        }
        false
    }

    let tokens = extracted_key.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty() && covers(evidence, &tokens, 0, &mut Vec::new())
}

/// Matching counterpart of [`parsed_release_matches_title_evidence`] that keeps
/// *which* lookup key matched and whether the release year corroborated it.
/// The identity check needs both: a shared bare key is not evidence for an
/// ambiguous subject, while a unique alias or a year agreement is.
pub(crate) fn match_parsed_release_to_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> Option<TitleEvidenceMatch> {
    let root_or_episode_year = parsed.year.is_some_and(|parsed_year| {
        evidence.year == Some(parsed_year) || evidence.episode_release_years.contains(&parsed_year)
    });
    let mut evidence_match =
        contextual_release_matches_title_evidence(parsed, evidence, root_or_episode_year)?;

    if let (Some(parsed_year), Some(expected_year)) = (parsed.year, evidence.year)
        && parsed_year != expected_year
    {
        let matched_alias_supports_year = evidence
            .alias_release_years
            .get(&evidence_match.matched_key)
            .is_some_and(|year| *year == parsed_year);
        if !evidence.episode_release_years.contains(&parsed_year) && !matched_alias_supports_year {
            return None;
        }
        if matched_alias_supports_year {
            evidence_match.year_corroborated = true;
            evidence_match.requires_external_id = false;
        }
    }

    Some(evidence_match)
}

fn contextual_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
    year_corroborated: bool,
) -> Option<TitleEvidenceMatch> {
    // Pass 1 — the unbiased extraction must name this title before the
    // target-biased parse is allowed to prove anything. Bias can refine an
    // anchored identity (projection, numbering, year); it can never
    // manufacture one.
    let anchored = context_free_identity_anchor_keys(&parsed.raw_title)
        .iter()
        .any(|anchor_key| {
            evidence_key_for_normalized(evidence, anchor_key).is_some()
                || extraction_decomposes_into_evidence_keys(evidence, anchor_key)
        });
    if !anchored {
        return None;
    }

    // Pass 2 — the target-biased parse confirms the anchor and supplies the
    // projection the acquisition pipeline actually consumes.
    let contextual = crate::analyze_release_for_target(&parsed.raw_title, &evidence.parse_context);
    if contextual.is_unparseable() {
        return None;
    }
    let best_candidate = contextual.best_candidate()?;

    let year_corroborated = year_corroborated
        || (best_candidate.projected.year.is_some()
            && evidence.year.is_some()
            && best_candidate.projected.year == evidence.year);

    // The biased parse's pre-projection canonical/alias spans confirm the
    // anchor; each must sit inside a recognized title zone. Full zone
    // accounting is the anchor's job now — `Electric Bloom` already failed
    // pass 1 for `BLOOM`, because its unbiased extraction names no such key.
    best_candidate
        .context_title_matches
        .iter()
        .filter(|context_match| {
            !matches!(
                context_match.kind,
                crate::release_parser::ContextTitleMatchKind::EpisodeTitle
            )
        })
        .filter(|context_match| {
            best_candidate.zones.title_zones.iter().any(|zone| {
                context_match.token_range.start_token >= zone.start_token
                    && context_match.token_range.end_token <= zone.end_token
            })
        })
        .filter_map(|context_match| {
            let normalized = crate::title_matching::canonical_lookup_key(&context_match.normalized);
            let matched_key = evidence_key_for_normalized(evidence, &normalized)?;
            let canonical_shape =
                crate::import_title_resolution::strip_trailing_year_key(&evidence.canonical_key);
            let is_single_word_alias = context_match.kind
                == crate::release_parser::ContextTitleMatchKind::TitleAlias
                && normalized.split_whitespace().count() == 1
                && normalized != evidence.canonical_key
                && normalized != canonical_shape;
            Some(TitleEvidenceMatch {
                matched_key,
                year_corroborated,
                requires_external_id: is_single_word_alias && !year_corroborated,
            })
        })
        .max_by_key(|evidence_match| {
            (
                evidence
                    .ambiguity
                    .key_is_unique_to_title(&evidence_match.matched_key),
                evidence_match.matched_key.len(),
            )
        })
}

#[cfg(test)]
pub(crate) fn candidate_matches_title_subject(
    candidate: &IndexerSearchResult,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    candidate_title_match(candidate, evidence).is_some()
}

/// Matching counterpart of [`candidate_matches_title_subject`] that retains the
/// disambiguator inputs (matched key and year agreement).
pub(crate) fn candidate_title_match(
    candidate: &IndexerSearchResult,
    evidence: &CanonicalTitleEvidence,
) -> Option<CandidateTitleMatch> {
    let parsed_owned;
    let parsed = if let Some(parsed) = candidate.parsed_release_metadata.as_ref() {
        parsed
    } else {
        parsed_owned =
            crate::parse_release_metadata_for_target(&candidate.title, &evidence.parse_context);
        &parsed_owned
    };

    match_parsed_release_to_title_evidence(parsed, evidence).map(|evidence_match| {
        CandidateTitleMatch {
            evidence_match: Some(evidence_match),
        }
    })
}

/// For an identity-ambiguous subject, an automatic candidate must present one
/// positive disambiguator. `external_id_agreement` is computed by
/// [`candidate_external_id_agreement`] from the captured response attributes.
/// An indexer-asserted id suffices alone; a contradicting parsed year has
/// already vetoed the match upstream in
/// [`match_parsed_release_to_title_evidence`], so the year veto still outranks
/// it. Only `Some(true)` satisfies the gate — a disagreement or an absent
/// assertion is simply not a disambiguator, never a veto of its own.
pub(crate) fn candidate_presents_identity_disambiguator(
    evidence: &CanonicalTitleEvidence,
    title_match: &CandidateTitleMatch,
    external_id_agreement: Option<bool>,
) -> bool {
    if let Some(evidence_match) = title_match.evidence_match.as_ref() {
        // The release carries the title's year.
        if evidence_match.year_corroborated {
            return true;
        }
        // The matched key is an alias unique to this title within the library
        // collision set, not the shared bare key.
        if evidence
            .ambiguity
            .key_is_unique_to_title(&evidence_match.matched_key)
        {
            return true;
        }
    }

    // External-id agreement can break the identity tie;
    // `title_validated_upstream` remains diagnostic provenance and cannot.
    external_id_agreement.unwrap_or(false)
}

const EXTERNAL_ID_CONFLICTS_EXTRA_KEY: &str = "scryer_external_id_conflicts";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ExternalIdComparison {
    compared: bool,
    matched: bool,
    conflicts: Vec<ExternalIdConflict>,
}

impl ExternalIdComparison {
    fn agreement(&self) -> Option<bool> {
        if self.matched {
            Some(true)
        } else {
            self.compared.then_some(false)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExternalIdConflict {
    kind: &'static str,
    expected: String,
    actual: String,
}

/// Compare the indexer's response ids against ids Scryer already holds.
///
/// Agreement is positive disambiguating evidence. Conflicts remain advisory:
/// they are retained separately for diagnostics and never override an
/// agreement from another id kind.
pub(crate) fn external_id_agreement(
    response: &IndexerResponseAttributes,
    tvdb_id: Option<&str>,
    tmdb_id: Option<&str>,
    imdb_id: Option<&str>,
) -> Option<bool> {
    external_id_comparison(response, tvdb_id, tmdb_id, imdb_id).agreement()
}

fn numeric_external_id_values(
    response: Option<&str>,
    subject: Option<&str>,
) -> Option<(String, String)> {
    let response = response.map(str::trim).filter(|value| !value.is_empty())?;
    let subject = subject.map(str::trim).filter(|value| !value.is_empty())?;
    Some((response.to_string(), subject.to_string()))
}

fn imdb_external_id_values(
    response: Option<&str>,
    subject: Option<&str>,
) -> Option<(String, String)> {
    let response = crate::normalize::normalize_imdb_id(response?)?;
    let subject = crate::normalize::normalize_imdb_id(subject?)?;
    Some((response, subject))
}

fn external_id_comparison(
    response: &IndexerResponseAttributes,
    tvdb_id: Option<&str>,
    tmdb_id: Option<&str>,
    imdb_id: Option<&str>,
) -> ExternalIdComparison {
    let mut comparison = ExternalIdComparison::default();
    for (kind, values) in [
        (
            "tvdb",
            numeric_external_id_values(response.tvdb_id.as_deref(), tvdb_id),
        ),
        (
            "tmdb",
            numeric_external_id_values(response.tmdb_id.as_deref(), tmdb_id),
        ),
        (
            "imdb",
            imdb_external_id_values(response.imdb_id.as_deref(), imdb_id),
        ),
    ] {
        let Some((actual, expected)) = values else {
            continue;
        };
        comparison.compared = true;
        if actual == expected {
            comparison.matched = true;
        } else {
            comparison.conflicts.push(ExternalIdConflict {
                kind,
                expected,
                actual,
            });
        }
    }
    comparison
}

fn candidate_external_id_comparison(
    candidate: &IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) -> ExternalIdComparison {
    external_id_comparison(
        &candidate.response_attributes,
        subject.tvdb_id.as_deref(),
        subject.tmdb_id.as_deref(),
        subject.imdb_id.as_deref(),
    )
}

fn candidate_external_id_agreement(
    candidate: &IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) -> Option<bool> {
    candidate_external_id_comparison(candidate, subject).agreement()
}

fn annotate_external_id_diagnostics(
    candidate: &mut IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) {
    let comparison = candidate_external_id_comparison(candidate, subject);
    if comparison.conflicts.is_empty() {
        candidate.extra.remove(EXTERNAL_ID_CONFLICTS_EXTRA_KEY);
        return;
    }
    candidate.extra.insert(
        EXTERNAL_ID_CONFLICTS_EXTRA_KEY.to_string(),
        serde_json::Value::Array(
            comparison
                .conflicts
                .into_iter()
                .map(|conflict| {
                    serde_json::json!({
                        "kind": conflict.kind,
                        "expected": conflict.expected,
                        "actual": conflict.actual,
                    })
                })
                .collect(),
        ),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateParseState {
    Parsed,
    Ambiguous,
    Unparseable,
}

fn candidate_parse_state(candidate: &IndexerSearchResult) -> CandidateParseState {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return CandidateParseState::Unparseable;
    };

    if matches!(parsed.disposition, ParseDisposition::Unparseable)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:unparseable" || hint == "parse_status:unparseable")
    {
        return CandidateParseState::Unparseable;
    }
    if parsed.is_ambiguous
        || matches!(parsed.disposition, ParseDisposition::Ambiguous)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:ambiguous" || hint == "parse_status:ambiguous")
    {
        return CandidateParseState::Ambiguous;
    }
    CandidateParseState::Parsed
}

fn normalized_release_identity(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Every name a stored file can be recognised by: the release it was grabbed
/// as, the scene name it carried, and the **stems** of the paths it has lived
/// at.
///
/// Stems, not paths. The path entries are absolute, so an exact comparison
/// against them is essentially always false — which is why the old rule fell
/// back to `contains`, and `contains` is what made this check wrong in the
/// direction that matters: `Show.S01E01.1080p-GRP` is a substring of
/// `/data/TV/Show/Show.S01E01.1080p-GRP.PROPER.mkv`, so the PROPER of a release
/// already on disk reported "already active" and could never be grabbed.
fn release_identities(file: &TitleMediaFile) -> impl Iterator<Item = String> + '_ {
    [
        file.grabbed_release_title.as_deref(),
        file.scene_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(normalized_release_identity)
    .chain(
        [
            file.original_file_path.as_deref(),
            Some(file.file_path.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter_map(|path| {
            std::path::Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(normalized_release_identity)
        }),
    )
}

/// Has this exact release already landed in the scope? The anti-loop guard: a
/// re-grab of a file Scryer already holds is never an upgrade.
///
/// Scoped by the **subject's incumbent file ids** rather than by a scalar
/// `episode_id`. `SubmissionScope::episode_id()` is `Some` only for a single
/// episode, so every pack, batch, title and link scope used to match *any* file
/// of the title; and a multi-episode file's span lives on the link table, not on
/// the row's scalar column. The subject already holds exactly the primary files
/// occupying the scope, whatever its shape, so membership by file id is
/// span-correct by construction.
fn candidate_matches_existing_media_file(
    candidate: &IndexerSearchResult,
    existing_files: &[TitleMediaFile],
    subject: &crate::admission::AdmissionSubject,
) -> bool {
    let release_title = normalized_release_identity(&candidate.title);
    if release_title.is_empty() {
        return false;
    }
    let in_scope: HashSet<&str> = subject
        .incumbents()
        .iter()
        .map(|incumbent| incumbent.file_id.as_str())
        .collect();
    if in_scope.is_empty() {
        return false;
    }

    existing_files
        .iter()
        .filter(|file| in_scope.contains(file.id.as_str()))
        .any(|file| release_identities(file).any(|identity| identity == release_title))
}

pub(crate) fn annotate_auto_decision(
    candidate: &mut IndexerSearchResult,
    code: ReleaseAutoDecisionCode,
) {
    candidate.auto_eligible = Some(code.is_eligible());
    candidate.auto_decision_code = Some(code.as_str().to_string());
    candidate.auto_decision_summary = Some(code.summary().to_string());
}

pub(crate) fn serialize_decision_explanation(candidate: &IndexerSearchResult) -> Option<String> {
    let quality = candidate.quality_profile_decision.as_ref();
    let scoring_log = quality
        .map(|decision| {
            decision
                .scoring_log
                .iter()
                .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let parsed = candidate.parsed_release_metadata.as_ref().map(|parsed| {
        serde_json::json!({
            "raw_title": parsed.raw_title.as_str(),
            "normalized_title": parsed.normalized_title.as_str(),
            "normalized_title_variants": &parsed.normalized_title_variants,
            "year": parsed.year,
            "quality": parsed.quality.as_deref(),
            "source": parsed.source.as_ref().map(|source| format!("{source:?}")),
            "release_group": parsed.release_group.as_deref(),
            "disposition": format!("{:?}", parsed.disposition),
            "parse_family": format!("{:?}", parsed.parse_family),
            "parse_confidence": parsed.parse_confidence,
            "is_ambiguous": parsed.is_ambiguous,
            "parse_hints": &parsed.parse_hints,
        })
    });
    let payload = serde_json::json!({
        "candidate": {
            "source": candidate.source.as_str(),
            "source_kind": candidate.source_kind.map(|kind| kind.as_str()),
            "guid": candidate.guid.as_deref(),
            "download_url_present": candidate.download_url.as_deref().is_some_and(|value| !value.trim().is_empty()),
            "link_present": candidate.link.as_deref().is_some_and(|value| !value.trim().is_empty()),
            "external_id_conflicts": candidate.extra.get(EXTERNAL_ID_CONFLICTS_EXTRA_KEY),
        },
        "auto_decision": {
            "eligible": candidate.auto_eligible,
            "code": candidate.auto_decision_code.as_deref(),
            "summary": candidate.auto_decision_summary.as_deref(),
        },
        "quality_profile_decision": {
            "allowed": quality.map(|decision| decision.allowed),
            "block_codes": quality.map(|decision| &decision.block_codes),
            "release_score": quality.map(|decision| decision.release_score),
            "preference_score": quality.map(|decision| decision.preference_score),
            "scoring_log": scoring_log,
        },
        "parsed": parsed,
    });

    serde_json::to_string(&payload).ok()
}

/// Sonarr-style episode anchoring for numbering-scoped subjects: an episode or
/// season search must not auto-grab a release whose parse carries no episode
/// identity at all (movie-shaped bare-title junk survives the title guard for
/// generic names like "Pals"), or whose numbering contradicts the target.
/// Absent parse data stays permissive — the ambiguous/unparseable handling in
/// `evaluate_auto_candidate` owns that case. Daily (air-date) and season-pack
/// parses carry an episode identity and are only checked for fields both
/// sides actually have, mirroring the strategy-level guard in the search
/// client.
fn candidate_numbering_contradicts_subject(
    candidate: &IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) -> bool {
    if subject.season.is_none() && subject.episode.is_none() && subject.absolute_episode.is_none() {
        return false;
    }
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return false;
    };
    let Some(episode) = parsed.episode.as_ref() else {
        // A release that carries no numbering at all cannot answer a subject
        // that names one. The single exception is a specials-season subject
        // with no episode number of its own: specials are published under
        // names that assert nothing ("{title} OVA", "{title} - {special
        // name}"), so that pairing stays with the coverage resolver rather
        // than being vetoed here.
        return !(subject.season == Some(0)
            && subject.episode.is_none()
            && subject.absolute_episode.is_none());
    };
    crate::acquisition_coverage::parsed_numbering_contradicts_episode(
        subject.season,
        subject.episode,
        subject.absolute_episode,
        episode,
    )
}

/// A candidate's PROPER/REPACK rank, `0` when it could not be parsed.
///
/// An unparsed result already loses on tier and on score; reading it as
/// revision 0 keeps it from *winning* the revision step, which is the only way
/// a missing parse could help it.
fn candidate_revision(candidate: &IndexerSearchResult) -> i32 {
    candidate
        .parsed_release_metadata
        .as_ref()
        .map_or(0, crate::acquisition::scoring::revision_rank)
}

/// Sonarr's `IsRevisionUpgrade`: the same quality already on disk, re-released
/// at a later revision.
///
/// Tier-scoped rather than exact-quality-scoped, because Scryer's tiers are
/// resolution-only until Part 5 — a 1080p WEB-DL PROPER therefore counts as a
/// revision of a 1080p Bluray. Coarser than Sonarr, and the same coarseness the
/// rest of the ladder already has.
///
/// `false` for an unoccupied scope: there is nothing to be a revision *of*.
///
/// Over **any** incumbent of the scope, not only the best one: Sonarr's
/// `ProperSpecification` and `QualityCutoffNotMet` both iterate every covered
/// file, so on a pack or multi-episode scope a PROPER of a weaker member is
/// still a revision upgrade.
pub(crate) fn candidate_is_revision_upgrade(
    candidate: crate::admission::CandidateFacts,
    subject: &crate::admission::AdmissionSubject,
) -> bool {
    subject
        .incumbents()
        .iter()
        .any(|incumbent| revision_upgrade_over(candidate, incumbent))
}

/// Sonarr's `IsRevisionUpgrade` against one file: same tier, later revision.
fn revision_upgrade_over(
    candidate: crate::admission::CandidateFacts,
    incumbent: &crate::admission::Incumbent,
) -> bool {
    candidate.tier_index == incumbent.tier_index && candidate.revision > incumbent.revision
}

/// Has this scope reached its cutoff — **both** halves of it?
///
/// Sonarr's `CutoffNotMet` is `QualityCutoffNotMet || CustomFormatCutoffNotMet`,
/// so a scope is finished only when the quality *and* the format score have both
/// arrived. Checking the quality half alone would defeat the format cutoff:
/// `derive_format_cutoff_targets` would nominate a scope whose bar sits below
/// `cutoff_score`, and every lane would then refuse its candidates
/// `CutoffReached` because the quality was fine.
///
/// An unoccupied scope answers `true` for the score half — there is no bar to
/// fall short of — which is moot in practice, because a scope with no file has
/// no analyzed quality either and the quality half is already `false`.
pub(crate) fn incumbent_at_cutoff(
    quality_cutoff_met: bool,
    subject: &crate::admission::AdmissionSubject,
    cutoff_score: Option<i32>,
) -> bool {
    quality_cutoff_met
        && cutoff_score.is_none_or(|cutoff| {
            subject
                .best_incumbent()
                .is_none_or(|(_, bar)| bar >= cutoff)
        })
}

/// The one cutoff gate.
///
/// Sonarr's `QualityCutoffNotMet` is two halves: the scope's best file has
/// reached the profile cutoff (`incumbent_at_cutoff`, resolved per scope by the
/// lane), **and** the candidate is not a revision upgrade over it. Both halves
/// used to live in different places — a scope-level `if cutoff_reached { return }`
/// in each of the three auto lanes, which is why a PROPER could never reach a
/// scope that had otherwise finished.
///
/// The old-file guard is **cutoff-independent** and keyed on the revision
/// escape: the candidate genuinely is a PROPER over a file Scryer holds, and
/// Scryer is declining it on age alone, whether or not the scope has reached
/// its cutoff — Sonarr's `ProperSpecification` never consults the cutoff. It is
/// a *different* refusal with its own reason code. RSS/pending only — that
/// specification accepts unconditionally when a search produced the candidate.
pub(crate) fn cutoff_refusal(
    candidate: crate::admission::CandidateFacts,
    subject: &crate::admission::AdmissionSubject,
    incumbent_at_cutoff: bool,
    is_rss_lane: bool,
    now: &DateTime<Utc>,
) -> Option<ReleaseAutoDecisionCode> {
    // The revision escape and the old-file guard come first, because both are
    // cutoff-independent: Sonarr's `ProperSpecification` rejects a PROPER for a
    // week-old file whether or not the scope has reached its cutoff, and a
    // PROPER over a below-cutoff file is still the revision upgrade the ladder
    // admits. Gating the age check behind `incumbent_at_cutoff` made it inert
    // for every below-cutoff scope — exactly the scopes that see the most
    // PROPERs.
    if candidate_is_revision_upgrade(candidate, subject) {
        let proper_for_old_file = is_rss_lane
            && subject.incumbents().iter().any(|incumbent| {
                revision_upgrade_over(candidate, incumbent)
                    && crate::acquisition_policy::file_predates_proper_window(
                        Some(incumbent.created_at.as_str()),
                        now,
                    )
            });
        return proper_for_old_file.then_some(ReleaseAutoDecisionCode::ProperForOldFile);
    }
    if !incumbent_at_cutoff {
        return None;
    }
    Some(ReleaseAutoDecisionCode::CutoffReached)
}

/// Apply the shared admission rule to one search candidate.
fn candidate_meets_minimum_seeders(
    candidate: &IndexerSearchResult,
    thresholds: &HashMap<String, i32>,
) -> bool {
    let threshold = candidate
        .indexer_id
        .as_deref()
        .and_then(|indexer_id| thresholds.get(indexer_id))
        .copied()
        .unwrap_or(0);
    crate::acquisition::seed_goals::meets_minimum_seeders(
        candidate.source_kind,
        candidate.indexer_id.as_deref(),
        crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
        threshold,
    )
}

/// Evaluate only the delay-profile portion of the automatic decision using the
/// same current candidate and incumbent facts as the full evaluator. RSS uses
/// the returned exact deadline when persisting a temporary decision.
pub(crate) fn auto_candidate_delay_decision(
    candidate: &IndexerSearchResult,
    context: &AutoCandidateEvaluationContext<'_>,
) -> Option<crate::delay_profile::DelayDecision> {
    automatic_candidate_delay_decision(
        candidate,
        context.title,
        context.admission,
        context.profile,
        context.delay_profiles,
        context.user_invoked,
        context.oldest_overlapping_pending_published_at,
        context.now,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the search lane must reuse the exact delay inputs used by automatic evaluation"
)]
pub(crate) fn automatic_candidate_delay_decision(
    candidate: &IndexerSearchResult,
    title: &Title,
    admission: &crate::admission::AdmissionSubject,
    profile: &QualityProfile,
    delay_profiles: &[DelayProfile],
    user_invoked: bool,
    oldest_overlapping_pending_published_at: Option<DateTime<Utc>>,
    now: &DateTime<Utc>,
) -> Option<crate::delay_profile::DelayDecision> {
    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    let candidate_facts = crate::admission::CandidateFacts::new(
        crate::quality_profile::quality_tier_index(
            &profile.criteria,
            candidate
                .parsed_release_metadata
                .as_ref()
                .and_then(|parsed| parsed.quality.as_deref()),
        ),
        candidate_revision(candidate),
        candidate_score,
    )
    .with_release_title(&candidate.title);
    let delay_context = crate::delay_profile::DelayPolicyContext {
        user_invoked,
        candidate_score,
        preferred_protocol_same_tier_revision: admission.incumbents().iter().any(|incumbent| {
            incumbent.tier_index == candidate_facts.tier_index
                && candidate_facts.revision > incumbent.revision
        }),
        preferred_protocol_highest_quality: candidate_facts.tier_index == Some(0),
        oldest_overlapping_pending_published_at,
    };
    crate::delay_profile::grab_time_delay_decision_with_context(
        delay_profiles,
        &title.tags,
        &title.facet,
        candidate.source_kind,
        candidate
            .published_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at),
        &delay_context,
        now,
    )
}

/// Terminal extensions that mark a release as carrying subtitles and nothing
/// else. `mks` is Matroska's subtitle-only container; `sup` is a PGS bitmap
/// track. Wider than `scryer_domain::SUBTITLE_EXTENSIONS`, which classifies
/// files on disk rather than release names.
const SUBTITLES_ONLY_RELEASE_EXTENSIONS: &[&str] =
    &["mks", "srt", "ass", "ssa", "sub", "idx", "vtt", "sup"];

/// Only a terminal extension counts: `...mks.mkv` is a video release, and
/// "subs" or "ass" inside a release name is just a word.
fn release_title_is_subtitles_only(release_title: &str) -> bool {
    release_title
        .trim_end()
        .rsplit_once('.')
        .is_some_and(|(_, extension)| {
            SUBTITLES_ONLY_RELEASE_EXTENSIONS
                .iter()
                .any(|subtitle_extension| extension.eq_ignore_ascii_case(subtitle_extension))
        })
}

pub(crate) fn evaluate_auto_candidate(
    candidate: &IndexerSearchResult,
    context: &AutoCandidateEvaluationContext<'_>,
) -> ReleaseAutoDecisionCode {
    // Ahead of every other gate: a subtitle-only release carries no video, so
    // nothing the rest of the ladder measures can make it grabbable.
    if release_title_is_subtitles_only(&candidate.title) {
        return ReleaseAutoDecisionCode::SubtitlesOnly;
    }

    let parse_state = candidate_parse_state(candidate);
    let title_match = candidate_title_match(candidate, &context.subject.title_evidence);
    let matches_title = title_match.is_some();
    match parse_state {
        CandidateParseState::Ambiguous if !matches_title => {
            return ReleaseAutoDecisionCode::ParseAmbiguous;
        }
        CandidateParseState::Unparseable if !matches_title => {
            return ReleaseAutoDecisionCode::ParseUnparseable;
        }
        CandidateParseState::Ambiguous
        | CandidateParseState::Unparseable
        | CandidateParseState::Parsed => {}
    }

    let Some(title_match) = title_match else {
        return ReleaseAutoDecisionCode::TitleMismatch;
    };

    if candidate_numbering_contradicts_subject(candidate, context.subject) {
        return ReleaseAutoDecisionCode::EpisodeMismatch;
    }

    // Sonarr's `MonitoredEpisodeSpecification`. A multi-episode release that
    // reaches into an episode nobody is monitoring brings unwanted bytes with
    // the wanted ones, and there is no way to take half a file.
    //
    // `EpisodeSet` only. `SingleEpisode` coverage is already monitored-filtered
    // by target derivation and by the RSS lane. A `Collection` is exempt because
    // a season pack's scope *is* its monitored members; refusing a whole season
    // because one episode is unmonitored would reintroduce the partial-monitoring
    // trap. An operator-started search is exempt the way Sonarr's spec skips
    // the monitored check for user searches: the operator asking for a release
    // outranks the monitoring flags it happens to touch.
    if !context.user_invoked
        && !matches!(
            context.subject.submission_scope,
            SubmissionScope::Collection { .. }
        )
        && let Some(SubmissionScope::EpisodeSet { episode_ids }) = &candidate.coverage_scope
        && episode_ids
            .iter()
            .any(|episode_id| context.unmonitored_episode_ids.contains(episode_id))
    {
        return ReleaseAutoDecisionCode::EpisodeNotMonitored;
    }

    // The indexer filed this release under a category that contradicts the
    // subject. Checked before the ambiguity gate because it is the sharper
    // reason — an explicit contradiction rather than absent evidence — and it
    // is the only category protection torrent/magnet grabs and out-of-band
    // plugin NZB fetches get. The pre-submission gate sees only NZBs Scryer
    // itself downloads before submission.
    // Compare against the facet the subject was SEARCHED as, not the owning
    // title's facet: a series-movie subject is movie-faceted while its owner
    // is a series, and a correctly categorized Movies release must not read
    // as a contradiction.
    if crate::indexer_category::indexer_categories_contradict_facet(
        &candidate.response_attributes.categories,
        &context.subject.search_facet,
    ) {
        return ReleaseAutoDecisionCode::CategoryMismatch;
    }

    // Burned releases report as blocklisted BEFORE the ambiguity gate runs: a
    // release that already failed must never be re-parked for review.
    if crate::app_usecase_discovery::is_release_blocklisted(
        candidate.indexer_id.as_deref(),
        &candidate.title,
        candidate.info_hash(),
        context.db_blocklist,
    ) {
        return ReleaseAutoDecisionCode::DbBlocklisted;
    }

    // Swarm health, checked after the sharper vetoes above so a release that is
    // both mis-categorised and dead still reports the reason an operator can
    // act on. This is admission only: nothing is recorded, so the same release
    // becomes eligible again the moment the swarm recovers.
    if !candidate_meets_minimum_seeders(candidate, context.minimum_seeders) {
        return ReleaseAutoDecisionCode::MinimumSeeders;
    }

    let external_id_agreement = candidate_external_id_agreement(candidate, context.subject);
    if title_match
        .evidence_match
        .as_ref()
        .is_some_and(|evidence_match| evidence_match.requires_external_id)
        && external_id_agreement != Some(true)
    {
        return ReleaseAutoDecisionCode::AmbiguousIdentity;
    }

    // A bare release name is not identity evidence when the subject's canonical
    // title collides with another library title.
    if context
        .subject
        .title_evidence
        .ambiguity
        .requires_disambiguator()
        && !candidate_presents_identity_disambiguator(
            &context.subject.title_evidence,
            &title_match,
            external_id_agreement,
        )
    {
        return ReleaseAutoDecisionCode::AmbiguousIdentity;
    }

    let is_allowed = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.allowed)
        .unwrap_or(false);
    if !is_allowed {
        return ReleaseAutoDecisionCode::QualityBlocked;
    }

    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    // Built before the cutoff gate because that gate needs the candidate's tier
    // and revision, not just the scope's state.
    let candidate_facts = crate::admission::CandidateFacts::new(
        crate::quality_profile::quality_tier_index(
            &context.profile.criteria,
            candidate
                .parsed_release_metadata
                .as_ref()
                .and_then(|parsed| parsed.quality.as_deref()),
        ),
        candidate_revision(candidate),
        candidate_score,
    )
    .with_release_title(&candidate.title);

    if let Some(code) = cutoff_refusal(
        candidate_facts,
        context.admission,
        context.incumbent_at_cutoff,
        context.is_rss_lane,
        context.now,
    ) {
        return code;
    }

    // No hardcoded zero floor. Zero stopped meaning anything once quality tier
    // left the score: every listed tier used to contribute 3200/900/300, so a
    // release had to be genuinely bad to fall below zero, and now a perfectly
    // ordinary one can. The score is a relative, within-tier quantity — the only
    // absolute floor is the profile's own `min_score_to_grab`, applied by
    // `apply_min_score_gate` and carried here as a block, which is Sonarr's
    // opt-in `MinFormatScore` rather than a built-in rule.
    //
    // `NegativeScore` stays in the decision-code enum so historical rows still
    // decode.

    if let Some(dl_snapshot) = context.dl_snapshot
        && dl_snapshot.is_active(&candidate.title)
    {
        return ReleaseAutoDecisionCode::AlreadyActive;
    }

    if candidate_matches_existing_media_file(candidate, context.existing_files, context.admission) {
        return ReleaseAutoDecisionCode::AlreadyActive;
    }

    if let Some(failed_routes) = context.failed_routes
        && let Some(route) = crate::acquisition_workflow::DownloadRouteKey::for_candidate(candidate)
        && failed_routes.contains(&route)
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    // The same gate the import path runs, over the same incumbents. That shared
    // predicate is what stops Scryer queueing a download it would then refuse.
    // Grab additionally applies the persona's churn thresholds, so it is the
    // stricter of the two — safe, because anything it declines is never fetched.
    let policy = crate::admission::AdmissionPolicy {
        allow_upgrades: context.profile.criteria.allow_upgrades,
        min_delta: context.thresholds.same_tier_min_delta,
        // Sonarr reads `UpgradeAllowed ? CutoffFormatScore : MinFormatScore`
        // here; the `else` arm is unreachable in this ladder, because a
        // no-upgrade profile returns `UpgradesDisabled` before either gate
        // consults the cutoff. So this is just the cutoff.
        cutoff_score: context.profile.criteria.cutoff_score,
        manual_override: false,
        // The grab lanes, and only the grab lanes, treat in-flight submissions
        // as pseudo-incumbents.
        applies_to_queue: true,
    };
    let verdict = crate::admission::evaluate_admission(context.admission, candidate_facts, &policy);
    if let Some(rejection) = verdict.rejection() {
        // The decision row records the *bar* the gate compared against; this
        // records which file set it. Two files can share a score, and an
        // operator asking "why did this not grab" needs the row, not the number.
        tracing::debug!(
            release = candidate.title.as_str(),
            reason = ?rejection.reason,
            incumbent_file_id = rejection.incumbent_file_id.as_str(),
            incumbent_file_path = rejection.incumbent_file_path.as_str(),
            "admission refused a grab candidate"
        );
        // One refusal reads as its own thing rather than as a generic upgrade
        // rejection: an operator can act on "something better is already
        // downloading", and cannot act on "upgrade policy said no".
        return match rejection.reason {
            crate::admission::AdmissionRejectionReason::QueuedEqualOrBetter { .. }
            | crate::admission::AdmissionRejectionReason::QueuedSameRelease { .. } => {
                ReleaseAutoDecisionCode::QueuedBetterOrEqual
            }
            _ => ReleaseAutoDecisionCode::UpgradeRejected,
        };
    }

    // Churn guard: a freshly-imported scope is left alone briefly even when a
    // better release shows up. This gates *starting* work, so it is a grab-only
    // concern and deliberately absent from the shared verdict.
    if let Some(incumbent) = context.admission.best_incumbent()
        && crate::acquisition_policy::upgrade_cooldown_is_active(
            crate::acquisition_policy::CooldownCandidate {
                tier_index: candidate_facts.tier_index,
                score: candidate_score,
            },
            incumbent,
            context.last_search_at,
            context.now,
            context.thresholds,
        )
    {
        return ReleaseAutoDecisionCode::UpgradeRejected;
    }

    // After admission on purpose: a same-tier higher-revision candidate now
    // admits above, so this rule runs on exactly the population Sonarr's
    // `RepackSpecification` checks — the repacks that would otherwise be fetched.
    if crate::acquisition_policy::repack_group_mismatch(
        candidate,
        candidate_facts,
        context.admission,
    ) {
        return ReleaseAutoDecisionCode::RepackGroupMismatch;
    }

    if let Some(delay_decision) = auto_candidate_delay_decision(candidate, context)
        && delay_decision.blocks_grab()
    {
        return match delay_decision.reason {
            crate::delay_profile::DelayDecisionReason::Eligible => {
                ReleaseAutoDecisionCode::Eligible
            }
            crate::delay_profile::DelayDecisionReason::PendingDelay => {
                ReleaseAutoDecisionCode::PendingDelay
            }
            crate::delay_profile::DelayDecisionReason::MinimumAge => {
                ReleaseAutoDecisionCode::MinimumAge
            }
            crate::delay_profile::DelayDecisionReason::ReleaseAgeUnknown => {
                ReleaseAutoDecisionCode::ReleaseAgeUnknown
            }
            crate::delay_profile::DelayDecisionReason::ProtocolDisabled => {
                ReleaseAutoDecisionCode::ProtocolDisabled
            }
        };
    }

    ReleaseAutoDecisionCode::Eligible
}

fn active_pending_release_delay_code(
    code: ReleaseAutoDecisionCode,
    user_invoked: bool,
    has_active_pending_overlap: bool,
) -> ReleaseAutoDecisionCode {
    if !user_invoked && has_active_pending_overlap && code == ReleaseAutoDecisionCode::Eligible {
        ReleaseAutoDecisionCode::PendingDelay
    } else {
        code
    }
}

fn preferred_scoped_external_id(ids: &[ScopedExternalId], source: &str) -> Option<String> {
    ids.iter()
        .find(|id| {
            id.source.eq_ignore_ascii_case(source)
                && id
                    .source_scope
                    .as_deref()
                    .is_some_and(|scope| scope.eq_ignore_ascii_case("R"))
                && !id.external_id.trim().is_empty()
        })
        .or_else(|| {
            ids.iter().find(|id| {
                id.source.eq_ignore_ascii_case(source) && !id.external_id.trim().is_empty()
            })
        })
        .map(|id| id.external_id.trim().to_string())
}

impl AppUseCase {
    /// Library-local identity ambiguity for a search subject.
    /// Reads the cached monitored-title matcher, whose normalized-title index is
    /// already built from `canonical_title_lookup_keys`, so a convergence cycle
    /// pays for one index build instead of a query per subject. Falls back to
    /// "not ambiguous" when the index cannot be loaded; the import gate still
    /// catches the mismatch.
    pub(crate) async fn title_identity_ambiguity(&self, title: &Title) -> TitleIdentityAmbiguity {
        match self.monitored_title_matcher().await {
            Ok(matcher) => TitleIdentityAmbiguity::from_shared_keys(
                matcher.shared_lookup_keys(&title.id, &canonical_title_lookup_keys(title)),
            ),
            Err(error) => {
                tracing::debug!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "identity ambiguity: monitored title index unavailable, treating title as unambiguous"
                );
                TitleIdentityAmbiguity::default()
            }
        }
    }

    fn release_search_category_for_facet(&self, facet: &MediaFacet) -> String {
        self.facet_registry
            .get(facet)
            .map(|handler| handler.search_category().to_string())
            .unwrap_or_else(|| match facet {
                MediaFacet::Movie => "movie".to_string(),
                MediaFacet::Series => "series".to_string(),
                MediaFacet::Anime => "anime".to_string(),
            })
    }

    pub(crate) async fn local_scoped_anidb_id_for_episode(
        &self,
        episode: Option<&Episode>,
    ) -> Option<String> {
        let episode = episode?;
        // Prefer season/collection-scoped AniDB mappings, then let callers fall
        // back to the title-level AniDB ID.
        let collection_id = episode.collection_id.as_deref()?;
        self.local_scoped_anidb_id_for_collection(collection_id)
            .await
    }

    async fn local_scoped_anidb_id_for_collection(&self, collection_id: &str) -> Option<String> {
        let collection_ids = self
            .services
            .catalog
            .shows
            .list_collection_external_ids(collection_id)
            .await
            .unwrap_or_default();
        preferred_scoped_external_id(&collection_ids, "anidb")
    }

    pub(crate) async fn release_search_title_for_wanted_item(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
    ) -> Title {
        let search_title = if item.media_type == "series_movie" {
            if let Some(ref link_id) = item.series_movie_link_id
                && let Ok(Some(link)) = self
                    .services
                    .catalog
                    .shows
                    .get_series_movie_link_by_id(link_id)
                    .await
            {
                series_movie_search_title(title, &link)
            } else {
                title.clone()
            }
        } else {
            title.clone()
        };

        if item.media_type == "episode"
            && let Some(anidb_id) = self.local_scoped_anidb_id_for_episode(episode).await
        {
            let mut search_title = search_title;
            search_title.external_ids.retain(|id| {
                !matches!(
                    id.source.trim().to_ascii_lowercase().as_str(),
                    "anidb" | "anidb_id"
                )
            });
            search_title.external_ids.push(scryer_domain::ExternalId {
                source: "anidb".into(),
                value: anidb_id,
            });
            return search_title;
        }

        search_title
    }

    pub(crate) async fn evaluate_search_results_for_subject(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        mut results: Vec<IndexerSearchResult>,
        user_invoked: bool,
    ) -> Vec<IndexerSearchResult> {
        // `DbBlocklisted` reads the per-title blocklist (the single, removable
        // exclusion source), never the failed-attempt history.
        let db_blocklist = self
            .load_title_release_blocklist_signatures(&title.id)
            .await;

        let dl_snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|file| file.role.is_primary())
            .collect::<Vec<_>>();
        let delay_profiles = self.load_delay_profiles().await;
        let now = Utc::now();
        let cutoff_scope = self.cutoff_scope_for(&subject.submission_scope).await;
        let analyzed_cutoff_quality =
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing_files,
                &cutoff_scope,
            );
        let upgrade_context = match self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                title,
                Some(subject.category.as_str()),
                analyzed_cutoff_quality,
            )
            .await
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "auto evaluation: failed to resolve quality profile; leaving candidates unevaluated"
                );
                return results;
            }
        };

        // One catalog read per scope, shared by the unmonitored-episode refusal
        // and by the queued pseudo-incumbents' runtime-basis calculation.
        let (catalog_episodes, catalog_collections) = if title.facet == MediaFacet::Movie {
            (Vec::new(), Vec::new())
        } else {
            (
                self.services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await
                    .unwrap_or_default(),
                self.services
                    .catalog
                    .shows
                    .list_collections_for_title(&title.id)
                    .await
                    .unwrap_or_default(),
            )
        };
        let unmonitored_episode_ids: HashSet<String> = catalog_episodes
            .iter()
            .filter(|episode| !episode.monitored)
            .map(|episode| episode.id.clone())
            .collect();

        let minimum_seeders = self.minimum_seeders_for_candidates(&results).await;
        // What is actually in the way, scored the way the import gate will score
        // it — not the ledger's recollection of a past grab.
        let scoring_context = self
            .resolve_canonical_scoring_context(title, &upgrade_context.profile)
            .await;
        let mut admission = self
            .admission_subject_for_scope(
                title,
                &subject.submission_scope,
                &scoring_context,
                None,
                crate::quality::canonical_context::SubjectIntent::Grab,
            )
            .await;
        // **What is already downloading counts too.** Sonarr's
        // `QueueSpecification` compares a queued release the same way it
        // compares a file on disk, and the convergence lane's old answer — a
        // scope-level "something is in flight, skip" — could not tell an
        // identical re-grab from a genuine upgrade over a slow download.
        //
        // An unobservable queue is the one case that still hard-skips, in the
        // lane; here it means the pseudo-incumbents would be built from a
        // snapshot that reports everything as active, so they are skipped.
        let membership = self
            .scope_membership_for(title, &subject.submission_scope)
            .await;
        let mut queued = Vec::new();
        if !dl_snapshot.queue_listing_failed() {
            let submissions = self
                .services
                .workflow
                .download_submissions
                .list_for_title(&title.id)
                .await
                .unwrap_or_default();
            if !submissions.is_empty() {
                let identities = submissions
                    .iter()
                    .map(crate::contracts::ClientJobLocator::from_submission)
                    .collect::<Vec<_>>();
                let tracked_states = self
                    .services
                    .workflow
                    .download_submissions
                    .list_identity_tracked_states_for_client_items(&identities)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(identity, state)| {
                        scryer_domain::TrackedDownloadState::from_str_opt(&state)
                            .map(|state| (identity, state))
                    })
                    .collect();
                queued = self
                    .queued_releases_for_scope(
                        title,
                        &membership.view(),
                        &scoring_context,
                        &submissions,
                        &tracked_states,
                        &dl_snapshot,
                        &catalog_episodes,
                        &catalog_collections,
                    )
                    .await;
            }
        }
        // The ledger's recorded grab claims the scope even when the client
        // shows nothing for it this pass.
        let queued = self
            .queued_releases_with_grabbed_claims(
                queued,
                title,
                &membership.view(),
                &scoring_context,
                &catalog_episodes,
                &catalog_collections,
            )
            .await;
        admission = admission.with_queued(queued);
        let has_active_pending_overlap = if user_invoked {
            false
        } else {
            let covered_wanted_item_ids = self
                .covered_wanted_item_ids_for_submission_scope(
                    &title.id,
                    &subject.submission_scope,
                    "",
                )
                .await
                .unwrap_or_default()
                .into_iter()
                .collect::<HashSet<_>>();
            !covered_wanted_item_ids.is_empty()
                && self
                    .services
                    .workflow
                    .pending_releases
                    .list_pending_releases_for_title(&title.id)
                    .await
                    .unwrap_or_default()
                    .iter()
                    .any(|release| {
                        matches!(
                            release.status,
                            crate::types::PendingReleaseStatus::Waiting
                                | crate::types::PendingReleaseStatus::Standby
                                | crate::types::PendingReleaseStatus::Processing
                        ) && covered_wanted_item_ids.contains(&release.wanted_item_id)
                    })
        };
        let evaluation_context = AutoCandidateEvaluationContext {
            title,
            subject,
            admission: &admission,
            last_search_at: subject.last_search_at.as_deref(),
            profile: &upgrade_context.profile,
            thresholds: &upgrade_context.thresholds,
            incumbent_at_cutoff: incumbent_at_cutoff(
                upgrade_context.cutoff_reached,
                &admission,
                upgrade_context.profile.criteria.cutoff_score,
            ),
            // Convergence and interactive both land here, and neither is a feed
            // pass: the old-file guard is Sonarr's RSS-only rule.
            is_rss_lane: false,
            user_invoked,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: Some(&dl_snapshot),
            db_blocklist: &db_blocklist,
            existing_files: &existing_files,
            delay_profiles: &delay_profiles,
            failed_routes: None,
            minimum_seeders: &minimum_seeders,
            unmonitored_episode_ids: &unmonitored_episode_ids,
        };

        for candidate in &mut results {
            if candidate.auto_decision_code.as_deref()
                == Some(ReleaseAutoDecisionCode::PackBelowMissingThreshold.as_str())
            {
                continue;
            }
            let code = active_pending_release_delay_code(
                evaluate_auto_candidate(candidate, &evaluation_context),
                user_invoked,
                has_active_pending_overlap,
            );
            annotate_external_id_diagnostics(candidate, evaluation_context.subject);
            annotate_auto_decision(candidate, code);
        }

        results
    }

    pub(crate) async fn resolve_release_search_subject_for_title(
        &self,
        title: &Title,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let tmdb_id = tmdb_id_from_external_ids(&title.external_ids);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let query = if title.facet == MediaFacet::Movie {
            movie_text_search_query(&title.name, title.year)
        } else {
            title.name.trim().to_string()
        };
        if !Self::has_release_search_input(
            &title.facet,
            &query,
            imdb_id.as_deref(),
            tmdb_id.as_deref(),
            tvdb_id.as_deref(),
            anidb_id.as_deref(),
        ) {
            return Err(AppError::Validation(
                "title has no name or external IDs".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(&title.id, None)
            .await
            .ok()
            .flatten();

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title)
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries: vec![query],
            imdb_id,
            tmdb_id,
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            submission_scope: SubmissionScope::Title,
        })
    }

    fn has_release_search_input(
        facet: &MediaFacet,
        query: &str,
        imdb_id: Option<&str>,
        tmdb_id: Option<&str>,
        tvdb_id: Option<&str>,
        anidb_id: Option<&str>,
    ) -> bool {
        !query.is_empty()
            || imdb_id.is_some()
            || tvdb_id.is_some()
            || anidb_id.is_some()
            // TMDB identifies movies, but does not make a series searchable.
            || (*facet == MediaFacet::Movie && tmdb_id.is_some())
    }

    pub(crate) async fn resolve_release_search_subject_for_episode(
        &self,
        title: &Title,
        season: &str,
        episode: &str,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let season = season.trim();
        let episode = episode.trim();
        if season.is_empty() || episode.is_empty() {
            return Err(AppError::Validation(
                "season and episode are required".into(),
            ));
        }

        let season_digits: String = season
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        let episode_digits: String = episode
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        if season_digits.is_empty() || episode_digits.is_empty() {
            return Err(AppError::Validation(
                "season and episode must include numeric values".into(),
            ));
        }

        let season_num = season_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid season value".into()))?;
        let episode_num = episode_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid episode value".into()))?;

        let episode_record = self
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &season_digits, &episode_digits)
            .await?;

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(
                &title.id,
                episode_record.as_ref().map(|episode| episode.id.as_str()),
            )
            .await
            .ok()
            .flatten();

        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let title_anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = self
            .local_scoped_anidb_id_for_episode(episode_record.as_ref())
            .await
            .or(title_anidb_id);

        let absolute_episode = episode_record
            .as_ref()
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.trim().parse::<u32>().ok());

        let category = self.release_search_category_for_facet(&title.facet);

        let mut queries = vec![format!(
            "{} S{:0>2}E{:0>2}",
            title.name.trim(),
            season_num,
            episode_num
        )];
        queries.push(format!("{} S{:0>2}", title.name.trim(), season_num));
        if title.facet == MediaFacet::Anime {
            if let Some(absolute) = absolute_episode {
                queries.insert(0, format!("{} {:0>3}", title.name.trim(), absolute));
            }
            queries.push(title.name.trim().to_string());
        }
        let mut seen = HashSet::new();
        queries.retain(|query| !query.trim().is_empty() && seen.insert(query.to_ascii_lowercase()));

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(title, episode_record.as_ref())
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries,
            imdb_id,
            tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: episode_record
                .as_ref()
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(title.runtime_minutes),
            season: Some(season_num),
            episode: Some(episode_num),
            absolute_episode,
            subject_kind: ReleaseSearchSubjectKind::Episode,
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            submission_scope: episode_record
                .as_ref()
                .map(|episode| SubmissionScope::Episode {
                    episode_id: episode.id.clone(),
                })
                .unwrap_or(SubmissionScope::Title),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_season_pack(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
        season_num: u32,
        runtime_minutes: Option<i32>,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let collection_anidb_id = match episode.and_then(|episode| episode.collection_id.as_deref())
        {
            Some(collection_id) => {
                self.local_scoped_anidb_id_for_collection(collection_id)
                    .await
            }
            None => None,
        };
        let anidb_id = collection_anidb_id.or(anidb_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let mut queries = vec![format!("{} S{:0>2}", title.name.trim(), season_num)];
        queries.retain(|query| !query.trim().is_empty());
        if queries.is_empty() && (imdb_id.is_some() || tvdb_id.is_some() || anidb_id.is_some()) {
            queries.push(String::new());
        }
        if queries.is_empty() {
            return Err(AppError::Validation(
                "season pack search subject has no searchable title or external IDs".into(),
            ));
        }

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title)
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries,
            imdb_id,
            tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes,
            season: Some(season_num),
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Season,
            last_search_at: item.last_search_at.clone(),
            submission_scope: collection_download_submission_scope_for_wanted_item(item, episode),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_series_movie(
        &self,
        title: &Title,
        link: &scryer_domain::SeriesMovieLink,
    ) -> AppResult<(Title, ResolvedReleaseSearchSubject)> {
        let search_title = series_movie_search_title(title, link);
        if search_title.name.trim().is_empty() {
            return Err(AppError::Validation(
                "series movie search subject has no searchable title".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                media_types: vec!["series_movie".into()],
                title_id: Some(title.id.clone()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?
            .into_iter()
            .find(|item| item.series_movie_link_id.as_deref() == Some(link.id.as_str()));

        let imdb_id = search_title
            .imdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_imdb_id);
        let tvdb_id = tvdb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let query_result = build_movie_search_queries(
            &search_title,
            "series_movie",
            self.release_search_category_for_facet(&search_title.facet),
        );
        let mut queries = query_result.queries;
        if queries.is_empty() && imdb_id.is_some() {
            queries.push(String::new());
        }
        let category = self.release_search_category_for_facet(&search_title.facet);

        Ok((
            search_title.clone(),
            ResolvedReleaseSearchSubject {
                title_id: title.id.clone(),
                title_tags: title.tags.clone(),
                title_evidence: canonical_title_evidence(&search_title)
                    .with_ambiguity(self.title_identity_ambiguity(&search_title).await),
                queries,
                imdb_id,
                tmdb_id: tmdb_id_from_external_ids(&search_title.external_ids),
                tvdb_id,
                anidb_id,
                mal_id: mal_id_from_external_ids(&search_title.external_ids),
                category,
                owner_facet: title.facet.clone(),
                search_facet: search_title.facet.clone(),
                id_search_facet: Some(MediaFacet::Movie),
                newznab_categories: series_movie_newznab_categories(&title.facet),
                runtime_minutes: search_title.runtime_minutes,
                season: None,
                episode: None,
                absolute_episode: None,
                subject_kind: ReleaseSearchSubjectKind::Title,
                last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
                submission_scope: SubmissionScope::SeriesMovie {
                    series_movie_link_id: link.id.clone(),
                },
            },
        ))
    }

    pub(crate) async fn resolve_release_search_subject_for_wanted_item(
        &self,
        owner_title: &Title,
        search_title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
    ) -> ResolvedReleaseSearchSubject {
        let query_result = build_search_queries(search_title, item, episode, &self.facet_registry);
        let owner_facet = if item.media_type == "series_movie" {
            owner_title.facet.clone()
        } else {
            owner_facet_for_wanted_item(owner_title, item)
        };
        let absolute_episode = episode
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.parse::<u32>().ok());

        ResolvedReleaseSearchSubject {
            title_id: owner_title.id.clone(),
            title_tags: owner_title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(search_title, episode)
                .with_ambiguity(self.title_identity_ambiguity(search_title).await),
            queries: query_result.queries,
            imdb_id: query_result.imdb_id,
            tmdb_id: query_result.tmdb_id,
            tvdb_id: query_result.tvdb_id,
            anidb_id: query_result.anidb_id,
            mal_id: query_result.mal_id,
            category: query_result.category.clone(),
            owner_facet: owner_facet.clone(),
            search_facet: search_title.facet.clone(),
            id_search_facet: (item.media_type == "series_movie").then_some(MediaFacet::Movie),
            newznab_categories: if item.media_type == "series_movie" {
                series_movie_newznab_categories(&owner_facet)
            } else {
                Vec::new()
            },
            runtime_minutes: episode
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(search_title.runtime_minutes),
            season: query_result.season,
            episode: query_result.episode,
            absolute_episode,
            subject_kind: match item.media_type.as_str() {
                "episode" => ReleaseSearchSubjectKind::Episode,
                _ => ReleaseSearchSubjectKind::Title,
            },
            last_search_at: item.last_search_at.clone(),
            submission_scope: direct_download_submission_scope_for_wanted_item(item, episode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, TaggedAlias, Title};

    #[test]
    fn active_pending_overlap_temporarily_delays_an_automatic_search() {
        assert_eq!(
            active_pending_release_delay_code(ReleaseAutoDecisionCode::Eligible, false, true),
            ReleaseAutoDecisionCode::PendingDelay
        );
    }

    #[test]
    fn interactive_search_skips_the_active_pending_delay_gate() {
        assert_eq!(
            active_pending_release_delay_code(ReleaseAutoDecisionCode::Eligible, true, true),
            ReleaseAutoDecisionCode::Eligible
        );
    }

    #[test]
    fn active_pending_delay_does_not_hide_minimum_age_or_permanent_diagnostics() {
        for code in [
            ReleaseAutoDecisionCode::MinimumAge,
            ReleaseAutoDecisionCode::ProtocolDisabled,
        ] {
            assert_eq!(active_pending_release_delay_code(code, false, true), code);
        }
    }

    fn make_title() -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Nightfall!!".to_string(),
            facet: MediaFacet::Anime,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Anime),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2022),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![TaggedAlias {
                name: "Nightfall Heavy Chorus Dark Lantern".to_string(),
                language: "eng".to_string(),
            }],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn tmdb_only_movie_is_searchable_without_tvdb() {
        let mut title = make_title();
        title.name.clear();
        title.facet = MediaFacet::Movie;
        title.external_ids = vec![
            scryer_domain::ExternalId {
                source: "smg".to_string(),
                value: "101".to_string(),
            },
            scryer_domain::ExternalId {
                source: "tmdb".to_string(),
                value: "603".to_string(),
            },
        ];

        let query_result = build_movie_search_queries(&title, "movie", "movie".to_string());

        assert!(AppUseCase::has_release_search_input(
            &title.facet,
            "",
            None,
            query_result.tmdb_id.as_deref(),
            None,
            None,
        ));
        assert_eq!(query_result.queries, vec![String::new()]);
        assert_eq!(query_result.tmdb_id.as_deref(), Some("603"));
        assert_eq!(query_result.imdb_id, None);
        assert_eq!(query_result.tvdb_id, None);
    }

    #[test]
    fn tmdb_only_id_does_not_make_a_series_searchable() {
        assert!(!AppUseCase::has_release_search_input(
            &MediaFacet::Series,
            "",
            None,
            Some("603"),
            None,
            None,
        ));
    }

    fn make_candidate(
        release_title: &str,
        provenance: Option<ReleaseCandidateProvenance>,
    ) -> IndexerSearchResult {
        IndexerSearchResult {
            indexer_id: None,
            source: "nzbgeek".to_string(),
            title: release_title.to_string(),
            link: None,
            download_url: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
            quality_profile_decision: None,
            extra: Default::default(),
            response_attributes: Default::default(),
            guid: None,
            info_url: None,
            provenance,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    fn make_media_file(release_title: &str, episode_id: Option<&str>) -> TitleMediaFile {
        TitleMediaFile {
            id: "media-file-1".to_string(),
            title_id: "title-1".to_string(),
            episode_id: episode_id.map(str::to_string),
            series_movie_link_ids: Vec::new(),
            role: crate::MediaFileRole::Primary,
            file_path: format!("/data/series/{release_title}.mkv"),
            size_bytes: 1,
            announced_size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("720p".to_string()),
            scan_status: "scanned".to_string(),
            created_at: Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: Some(1280),
            video_height: Some(720),
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            dovi_profile: None,
            dovi_bl_compat_id: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_languages: Vec::new(),
            subtitle_codecs: Vec::new(),
            subtitle_streams: Vec::new(),
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: Some(release_title.to_string()),
            release_group: None,
            source_type: None,
            resolution: Some("720p".to_string()),
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_channels_parsed: None,
            acquisition_score: Some(-15),
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: Some(format!(
                "/nzbget-downloads/completed/{release_title}/{release_title}.mkv"
            )),
            release_hash: None,
        }
    }

    /// A search subject over an explicit episode span.
    fn episode_set_subject(title: &Title, episode_ids: &[&str]) -> ResolvedReleaseSearchSubject {
        ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title),
            queries: vec![title.name.clone()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            last_search_at: None,
            submission_scope: SubmissionScope::EpisodeSet {
                episode_ids: episode_ids.iter().map(|id| (*id).to_string()).collect(),
            },
        }
    }

    /// A scope with nothing in it: the gate then has no bar to enforce, which is
    /// the starting point for most of these cases.
    fn empty_admission() -> crate::admission::AdmissionSubject {
        crate::admission::AdmissionSubject::new(crate::admission::AdmissionScope::Title, [])
    }

    /// A scope already holding one primary file at `score`.
    fn admission_holding(score: i32) -> crate::admission::AdmissionSubject {
        crate::admission::AdmissionSubject::new(
            crate::admission::AdmissionScope::Title,
            [(
                crate::admission::Incumbent {
                    // Tier-neutral on purpose: these cases are about the score
                    // comparison, so the incumbent sits in the same (unlisted)
                    // tier as the candidate and the tier gate is a no-op.
                    tier_index: None,
                    revision: 0,
                    file_id: "file-1".to_string(),
                    file_path: "/data/Movies/Nightfall (2022)/Nightfall.mkv".to_string(),
                    release_group: None,
                    score,
                    covers: Vec::new(),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                },
                true,
            )],
        )
    }

    fn allowed_quality_decision(score: i32) -> QualityProfileDecision {
        QualityProfileDecision {
            release_score: score,
            scoring_log: Vec::new(),
            allowed: true,
            block_codes: Vec::new(),
            preference_score: score,
            tier_index: None,
        }
    }

    #[test]
    fn canonical_title_lookup_keys_include_tagged_aliases() {
        let title = make_title();
        let keys = canonical_title_lookup_keys(&title);

        assert!(keys.iter().any(|key| key == "nightfall"));
        assert!(
            keys.iter()
                .any(|key| key == "nightfall heavy chorus dark lantern")
        );
    }

    #[test]
    fn upstream_validation_cannot_bypass_raw_title_proof() {
        let mut title = make_title();
        title.name = "Amber Circuit".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let candidate = make_candidate(
            "Amber.Circuit.2002.1080p.WEB-DL",
            Some(ReleaseCandidateProvenance {
                search_subject_kind: ReleaseSearchSubjectKind::Episode,
                strategy_kind: ReleaseStrategyKind::IdBacked,
                title_validated_upstream: true,
            }),
        );

        assert!(!candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn text_title_matching_rejects_only_mismatched_parsed_years() {
        let mut title = make_title();
        title.name = "Amber Circuit".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let evidence = canonical_title_evidence(&title);

        let mismatched = crate::parse_release_metadata("Amber.Circuit.2002.1080p.WEB-DL");
        assert_eq!(mismatched.year, Some(2002));
        assert!(!parsed_release_matches_title_evidence(
            &mismatched,
            &evidence
        ));

        let mut matching = mismatched.clone();
        matching.year = Some(2026);
        assert!(parsed_release_matches_title_evidence(&matching, &evidence));

        let mut missing_year = mismatched;
        missing_year.year = None;
        assert!(parsed_release_matches_title_evidence(
            &missing_year,
            &evidence
        ));
    }

    #[test]
    fn automatic_text_candidate_with_mismatched_year_is_not_eligible() {
        let mut title = make_title();
        title.name = "Amber Circuit".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let candidate = make_candidate("Amber.Circuit.2002.1080p.WEB-DL", None);
        let subject = ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(&title),
            queries: vec!["Amber Circuit 2026".to_string()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            last_search_at: None,
            submission_scope: SubmissionScope::Title,
        };
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::TitleMismatch
        );
    }

    /// A torrent candidate from `indexer` reporting `seeders`.
    fn torrent_candidate(
        release_title: &str,
        indexer: &str,
        seeders: Option<i64>,
    ) -> IndexerSearchResult {
        let mut candidate = make_candidate(release_title, None);
        candidate.indexer_id = Some(indexer.to_string());
        candidate.source_kind = Some(DownloadSourceKind::MagnetUri);
        if let Some(seeders) = seeders {
            candidate
                .extra
                .insert("seeders".to_string(), serde_json::json!(seeders));
        }
        candidate
    }

    #[test]
    fn a_dead_torrent_reports_minimum_seeders_without_being_recorded() {
        let mut title = make_title();
        title.name = "Amber Circuit".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2001);
        let subject = ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(&title),
            queries: vec![title.name.clone()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            last_search_at: None,
            submission_scope: SubmissionScope::Title,
        };
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let mut minimum_seeders = HashMap::new();
        minimum_seeders.insert("idx-private".to_string(), 1);
        let no_unmonitored_episodes = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &minimum_seeders,
            unmonitored_episode_ids: &no_unmonitored_episodes,
        };

        let dead = torrent_candidate("Amber.Circuit.2001.1080p.WEB-DL", "idx-private", Some(0));
        assert_eq!(
            evaluate_auto_candidate(&dead, &context),
            ReleaseAutoDecisionCode::MinimumSeeders
        );
        assert!(
            !ReleaseAutoDecisionCode::MinimumSeeders.is_eligible(),
            "auto search must skip it and move to the next candidate"
        );

        // Everything below asserts the gate does not fire rather than asserting
        // eligibility: this fixture carries no quality decision, so evaluation
        // continues past this gate and stops at the quality one. What matters
        // here is that the swarm check let the candidate through.
        for (label, candidate) in [
            // Recovered swarm — nothing about the earlier rejection was
            // recorded, so the same release is judged afresh.
            (
                "recovered",
                torrent_candidate("Amber.Circuit.2001.1080p.WEB-DL", "idx-private", Some(1)),
            ),
            // An indexer with no resolved threshold.
            (
                "unthresholded indexer",
                torrent_candidate("Amber.Circuit.2001.1080p.WEB-DL", "idx-public", Some(0)),
            ),
            // An indexer that reports no seeder count at all.
            (
                "unknown count",
                torrent_candidate("Amber.Circuit.2001.1080p.WEB-DL", "idx-private", None),
            ),
        ] {
            assert_ne!(
                evaluate_auto_candidate(&candidate, &context),
                ReleaseAutoDecisionCode::MinimumSeeders,
                "{label} must not be rejected for seeders"
            );
        }
    }

    fn numbering_scoped_subject(
        title: &Title,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> ResolvedReleaseSearchSubject {
        ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title),
            queries: vec![title.name.clone()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season,
            episode,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Episode,
            last_search_at: None,
            submission_scope: SubmissionScope::Title,
        }
    }

    #[test]
    fn subtitle_containers_are_recognised_only_as_a_terminal_extension() {
        for subtitles_only in [
            "Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.mks",
            "Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.MKS",
            "Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.srt",
            "Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.sup",
        ] {
            assert!(
                release_title_is_subtitles_only(subtitles_only),
                "{subtitles_only} carries subtitles only"
            );
        }

        for content in [
            "Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.mks.mkv",
            "Quiet.Meridian.S01E01.1080p.WEB-DL.[subs included]-GroupTag",
            "Quiet.Meridian.S01E01.ass.kicker.1080p.WEB-DL-GroupTag",
            "Quiet Meridian S01E01 1080p WEB-DL GroupTag",
        ] {
            assert!(
                !release_title_is_subtitles_only(content),
                "{content} is a content release"
            );
        }
    }

    #[test]
    fn a_subtitles_only_candidate_is_never_admissible() {
        let mut title = make_title();
        title.name = "Quiet Meridian".to_string();
        title.facet = MediaFacet::Series;
        title.tagged_aliases = Vec::new();
        let subject = numbering_scoped_subject(&title, Some(1), Some(1));

        let mut subtitles_only =
            make_candidate("Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.mks", None);
        subtitles_only.quality_profile_decision = Some(allowed_quality_decision(2400));
        assert_eq!(
            decision_for(&title, &subject, &subtitles_only),
            ReleaseAutoDecisionCode::SubtitlesOnly
        );

        let mut content = make_candidate("Quiet.Meridian.S01E01.1080p.WEB-DL-GroupTag.mkv", None);
        content.quality_profile_decision = Some(allowed_quality_decision(2400));
        assert_eq!(
            decision_for(&title, &subject, &content),
            ReleaseAutoDecisionCode::Eligible
        );
    }

    #[test]
    fn episode_subject_rejects_candidates_without_episode_identity() {
        // Bare-title junk for a generic name ("Pals") carries neither a
        // contradicting year nor episode numbering — the movie-shaped parse is
        // the only signal, and an episode-scoped subject must refuse it.
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Pals.1080p.BluRay.x264-GRP", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::EpisodeMismatch
        );
    }

    #[test]
    fn episode_subject_rejects_contradicting_season_numbering() {
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Pals.S05E01.1080p.WEB-DL", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::EpisodeMismatch
        );
    }

    fn specials_wanted_item(
        title: &Title,
        season: &str,
        episode_number: &str,
    ) -> (crate::AcquisitionScopeState, Episode) {
        let episode = Episode {
            id: "episode-special".into(),
            title_id: title.id.clone(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Special,
            episode_number: Some(episode_number.into()),
            season_number: Some(season.into()),
            episode_label: None,
            title: Some("Quiet Harbor: Graduation".into()),
            air_date: Some("2024-05-01".into()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let item = crate::AcquisitionScopeState {
            id: "wanted-1".into(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: Some(title.facet.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: episode.season_number.clone(),
            episode_number: episode.episode_number.clone(),
            media_type: "episode".into(),
            last_search_at: None,
            status: crate::AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: String::new(),
            updated_at: String::new(),
        };
        (item, episode)
    }

    /// The source of the regression: the specials season used to be folded into
    /// "no season" here, which is what left the acceptance veto with nothing to
    /// compare a season-1 release against.
    #[test]
    fn a_season_zero_wanted_item_reports_season_zero() {
        let mut title = make_title();
        title.name = "Quiet Harbor".to_string();
        title.facet = MediaFacet::Anime;
        let (item, episode) = specials_wanted_item(&title, "0", "2");

        let result =
            build_search_queries(&title, &item, Some(&episode), &crate::FacetRegistry::new());

        assert_eq!(result.season, Some(0));
        assert_eq!(result.episode, Some(2));
        assert!(
            !result
                .queries
                .iter()
                .any(|query| query.contains("S00E02") || query.contains("S00")),
            "season 0 must not shape an S00 query: {:?}",
            result.queries
        );
    }

    /// A regular season is unchanged by that: it still reports its own number
    /// and still shapes the `SxxEyy` queries.
    #[test]
    fn a_regular_wanted_item_still_reports_its_own_season() {
        let mut title = make_title();
        title.name = "Quiet Harbor".to_string();
        title.facet = MediaFacet::Anime;
        let (item, episode) = specials_wanted_item(&title, "2", "3");

        let result =
            build_search_queries(&title, &item, Some(&episode), &crate::FacetRegistry::new());

        assert_eq!(result.season, Some(2));
        assert_eq!(result.episode, Some(3));
        assert!(
            result.queries.iter().any(|query| query.contains("S02E03")),
            "a regular season still shapes an SxxEyy query: {:?}",
            result.queries
        );
    }

    /// The specials regression. A season-0 wanted item used to reach the
    /// acceptance layer with no expected season at all, so a regular S01E02
    /// release satisfied it on the episode number alone and the film was
    /// imported against the wrong scope.
    #[test]
    fn a_specials_season_subject_rejects_a_regular_season_release() {
        let mut title = make_title();
        title.name = "Quiet Harbor".to_string();
        title.facet = MediaFacet::Anime;
        title.tagged_aliases = Vec::new();
        let subject = numbering_scoped_subject(&title, Some(0), Some(2));

        let mut candidate = make_candidate("Quiet.Harbor.S01E02.1080p.WEB-DL-GroupTag.mkv", None);
        candidate.quality_profile_decision = Some(allowed_quality_decision(2400));

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::EpisodeMismatch
        );
    }

    /// The other half of the same gate. A specials subject that carries no
    /// episode number of its own must not start vetoing the unnumbered release
    /// names specials actually ship under — that pairing stays with the
    /// coverage resolver, exactly as it did before the season became known.
    #[test]
    fn a_specials_subject_without_an_episode_number_leaves_unnumbered_releases_alone() {
        let mut title = make_title();
        title.name = "Quiet Harbor".to_string();
        title.facet = MediaFacet::Anime;
        title.tagged_aliases = Vec::new();
        let subject = numbering_scoped_subject(&title, Some(0), None);

        let unnumbered = make_candidate("Quiet.Harbor.Graduation.1080p.BluRay-GroupTag.mkv", None);
        assert!(
            unnumbered
                .parsed_release_metadata
                .as_ref()
                .is_some_and(|parsed| parsed.episode.is_none()),
            "fixture must model a release that asserts no numbering"
        );
        assert!(
            !candidate_numbering_contradicts_subject(&unnumbered, &subject),
            "a release that asserts no numbering must not be vetoed for a special"
        );

        let regular_season = make_candidate("Quiet.Harbor.S01E02.1080p.WEB-DL-GroupTag.mkv", None);
        assert!(
            candidate_numbering_contradicts_subject(&regular_season, &subject),
            "a season-1 release cannot satisfy a season-0 scope"
        );
    }

    #[test]
    fn episode_subject_accepts_matching_numbering() {
        // A real multi-episode release (Pals.S09E23E24) agrees on season
        // and contains the wanted episode; it must clear the numbering gate
        // and fall through to the quality decision (absent here → blocked).
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Pals.S09E23E24.1080p.BluRay.x264-TENEIGHTY", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn pals_year_bearing_junk_is_vetoed_but_bare_releases_match() {
        // Pins the intent of the unconditional year veto: wrong-property junk
        // that carries its own year (a same-name twin) must never
        // match the 1994 series, while the real scene releases — which are
        // bare-titled — must keep matching.
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let evidence = canonical_title_evidence(&title);

        let mut junk = crate::parse_release_metadata("Pals.S01E01.1080p.WEB-DL");
        junk.year = Some(2002);
        assert!(!parsed_release_matches_title_evidence(&junk, &evidence));

        let legit =
            crate::parse_release_metadata("Pals.S09E23E24.1080p.NF.WEB-DL.DDP5.1.x264-PRAGMA");
        assert_eq!(legit.year, None);
        assert!(parsed_release_matches_title_evidence(&legit, &evidence));
    }

    #[test]
    fn generic_alias_does_not_support_a_conflicting_release_year() {
        let mut title = make_title();
        title.name = "Fixture Sitcom".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        title.aliases = vec!["Fixture Sitcom 2002 Archive".to_string()];
        title.tagged_aliases = vec![TaggedAlias {
            name: "Fixture Sitcom Television Series".to_string(),
            language: "eng".to_string(),
        }];
        let evidence = canonical_title_evidence(&title);

        assert!(evidence.episode_release_years.is_empty());
        assert!(evidence.alias_release_years.is_empty());
        let conflicting = crate::parse_release_metadata(
            "Fixture.Sitcom.2002.S01E03.1080p.WEB-DL.DDP5.1.H.264-GRP",
        );
        assert_eq!(conflicting.year, Some(2002));
        assert!(!parsed_release_matches_title_evidence(
            &conflicting,
            &evidence
        ));
    }

    #[test]
    fn year_bearing_alias_supports_anime_continuation_release_year() {
        let mut title = make_title();
        title.name = "Fixture Anime".to_string();
        title.facet = MediaFacet::Anime;
        title.year = Some(2004);
        title.aliases = vec!["Fixture Anime Continuation (2023)".to_string()];
        title.tagged_aliases.clear();
        let evidence = canonical_title_evidence(&title);

        assert_eq!(
            evidence
                .alias_release_years
                .get("fixture anime continuation 2023"),
            Some(&2023)
        );
        let continuation = crate::parse_release_metadata(
            "Fixture.Anime.Continuation.2023.S17E03.1080p.WEB-DL-GRP",
        );
        assert_eq!(continuation.year, Some(2023));
        assert!(parsed_release_matches_title_evidence(
            &continuation,
            &evidence
        ));
    }

    #[test]
    fn unmatched_year_bearing_alias_does_not_support_the_canonical_title() {
        let mut title = make_title();
        title.name = "Synthetic Root".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(2004);
        title.aliases = vec!["Synthetic Continuation (2023)".to_string()];
        title.tagged_aliases.clear();
        let evidence = canonical_title_evidence(&title);
        let release = crate::parse_release_metadata("Synthetic.Root.2023.S02E03.1080p.WEB-DL-GRP");

        assert!(!parsed_release_matches_title_evidence(&release, &evidence));
    }

    #[test]
    fn requested_episode_air_year_supports_a_continuation_release_year() {
        let mut title = make_title();
        title.name = "Synthetic Continuation".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(2004);
        title.aliases.clear();
        title.tagged_aliases.clear();
        let episode = Episode {
            id: "episode-1".into(),
            title_id: title.id.clone(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".into()),
            season_number: Some("2".into()),
            episode_label: None,
            title: None,
            air_date: Some("2023-07-01".into()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let evidence = canonical_title_evidence_for_episode(&title, Some(&episode));
        let release =
            crate::parse_release_metadata("Synthetic.Continuation.2023.S02E03.1080p.WEB-DL-GRP");

        assert!(evidence.episode_release_years.contains(&2023));
        assert!(parsed_release_matches_title_evidence(&release, &evidence));
    }

    // ── Identity ambiguity and required disambiguators ──────────────────────

    /// The incident pair: a live-action `Tide Chart` (2023, series) and the
    /// anime `Tide Chart` (1999) in the same library, both claiming the bare
    /// canonical key `tide chart`. `aliases` is applied to the live-action title
    /// so a unique-alias hit can be exercised.
    fn tide_chart_library(aliases: Vec<String>) -> (Title, Vec<Title>) {
        let mut live_action = make_title();
        live_action.id = "title-tide-chart-live".to_string();
        live_action.name = "Tide Chart".to_string();
        live_action.facet = MediaFacet::Series;
        live_action.year = Some(2023);
        live_action.aliases = aliases;
        live_action.tagged_aliases = Vec::new();

        let mut anime = make_title();
        anime.id = "title-tide-chart-anime".to_string();
        anime.name = "Tide Chart".to_string();
        anime.facet = MediaFacet::Anime;
        anime.year = Some(1999);
        anime.aliases = Vec::new();
        anime.tagged_aliases = Vec::new();

        let library = vec![live_action.clone(), anime];
        (live_action, library)
    }

    /// Tier 0 ambiguity exactly as the acquisition paths derive it: from the
    /// monitored-title index over the library, with no schema or SMG input.
    fn library_local_ambiguity(subject: &Title, library: &[Title]) -> TitleIdentityAmbiguity {
        let matcher = crate::import_title_resolution::MonitoredTitleMatcher::new(library.to_vec());
        TitleIdentityAmbiguity::from_shared_keys(
            matcher.shared_lookup_keys(&subject.id, &canonical_title_lookup_keys(subject)),
        )
    }

    fn ambiguous_episode_subject(
        title: &Title,
        library: &[Title],
        season: Option<u32>,
        episode: Option<u32>,
    ) -> ResolvedReleaseSearchSubject {
        let mut subject = numbering_scoped_subject(title, season, episode);
        subject.title_evidence = subject
            .title_evidence
            .with_ambiguity(library_local_ambiguity(title, library));
        subject
    }

    fn decision_for(
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        candidate: &IndexerSearchResult,
    ) -> ReleaseAutoDecisionCode {
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title,
            subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };
        evaluate_auto_candidate(candidate, &context)
    }

    #[test]
    fn library_local_collision_flags_shared_bare_key() {
        let (live_action, library) = tide_chart_library(vec!["Tide Chart Live Action".to_string()]);
        let ambiguity = library_local_ambiguity(&live_action, &library);

        assert!(ambiguity.requires_disambiguator());
        assert_eq!(ambiguity.shared_lookup_keys, vec!["tide chart".to_string()]);
        assert!(!ambiguity.key_is_unique_to_title("tide chart"));
        assert!(ambiguity.key_is_unique_to_title("tide chart live action"));
    }

    #[test]
    fn ambiguous_title_rejects_bare_candidate_without_disambiguator() {
        // The driving incident: a bare `Tide.Chart.S02E01` names both library
        // titles equally well, so it is not identity evidence for either.
        let (live_action, library) = tide_chart_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn ambiguous_title_accepts_year_disambiguator() {
        // The release carries the live-action title's year, so it names one of
        // the two colliding titles and clears the identity gate.
        let (live_action, library) = tide_chart_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("Tide.Chart.2023.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_accepts_unique_alias_disambiguator() {
        // The matched key is an alias only the live-action title claims.
        let (live_action, library) = tide_chart_library(vec!["Tide Chart Live Action".to_string()]);
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("Tide.Chart.Live.Action.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_rejects_upstream_provenance_without_release_id() {
        let (live_action, library) = tide_chart_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate(
            "Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP",
            Some(ReleaseCandidateProvenance {
                search_subject_kind: ReleaseSearchSubjectKind::Episode,
                strategy_kind: ReleaseStrategyKind::IdBacked,
                title_validated_upstream: true,
            }),
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    fn year_qualified_tide_chart() -> Title {
        let mut title = make_title();
        title.id = "title-tide-chart-live".to_string();
        title.name = "Tide Chart (2023)".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(2023);
        title.external_ids = vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "392276".to_string(),
            },
            ExternalId {
                source: "tmdb".to_string(),
                value: "111110".to_string(),
            },
        ];
        title
    }

    #[test]
    fn year_qualified_title_is_ambiguous_without_a_local_collider() {
        let live_action = year_qualified_tide_chart();
        let subject = numbering_scoped_subject(&live_action, Some(2), Some(7));
        let candidate = make_candidate("Tide.Chart.S02E07.1080p.WEB-DL.x264-GRP", None);

        assert!(subject.title_evidence.ambiguity.requires_disambiguator());
        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn year_qualified_title_accepts_positive_identity_evidence() {
        let mut live_action = year_qualified_tide_chart();
        live_action.aliases = vec!["Tide Chart Live Action".to_string()];
        let mut subject = numbering_scoped_subject(&live_action, Some(2), Some(7));
        subject.tvdb_id = Some("392276".to_string());
        subject.tmdb_id = Some("111110".to_string());

        let by_year = make_candidate("Tide.Chart.2023.S02E07.1080p.WEB-DL.x264-GRP", None);
        assert_eq!(
            decision_for(&live_action, &subject, &by_year),
            ReleaseAutoDecisionCode::QualityBlocked
        );

        let by_alias = make_candidate("Tide.Chart.Live.Action.S02E07.1080p.WEB-DL.x264-GRP", None);
        assert_eq!(
            decision_for(&live_action, &subject, &by_alias),
            ReleaseAutoDecisionCode::QualityBlocked
        );

        let mut by_id = make_candidate("Tide.Chart.S02E07.1080p.WEB-DL.x264-GRP", None);
        by_id.response_attributes.tvdb_id = Some("392276".to_string());
        assert_eq!(
            decision_for(&live_action, &subject, &by_id),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ordinary_title_year_metadata_does_not_require_release_year() {
        let mut pals = make_title();
        pals.name = "Pals".to_string();
        pals.year = Some(1994);

        assert!(
            !canonical_title_evidence(&pals)
                .ambiguity
                .requires_disambiguator()
        );
    }

    #[test]
    fn conflicting_response_id_is_advisory_when_another_id_agrees() {
        let live_action = year_qualified_tide_chart();
        let mut subject = numbering_scoped_subject(&live_action, Some(2), Some(7));
        subject.tvdb_id = Some("392276".to_string());
        subject.tmdb_id = Some("111110".to_string());
        let mut candidate = make_candidate("Tide.Chart.S02E07.1080p.WEB-DL.x264-GRP", None);
        candidate.response_attributes.tvdb_id = Some("392276".to_string());
        candidate.response_attributes.tmdb_id = Some("999999".to_string());

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked,
            "stale advisory metadata must not veto an otherwise valid title match"
        );
        assert_eq!(
            candidate_external_id_agreement(&candidate, &subject),
            Some(true),
            "the agreeing TVDB id remains positive disambiguating evidence"
        );

        annotate_external_id_diagnostics(&mut candidate, &subject);
        assert_eq!(
            candidate.extra.get(EXTERNAL_ID_CONFLICTS_EXTRA_KEY),
            Some(&serde_json::json!([{
                "kind": "tmdb",
                "expected": "111110",
                "actual": "999999",
            }])),
            "the advisory conflict remains visible for diagnostics"
        );

        candidate.response_attributes.tvdb_id = Some("888888".to_string());
        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity,
            "conflicting ids do not veto the title, but also cannot clear its ambiguity gate"
        );
        assert_eq!(
            candidate_external_id_agreement(&candidate, &subject),
            Some(false),
            "conflicts alone still do not satisfy an ambiguity gate"
        );
    }

    #[test]
    fn conflicting_response_id_does_not_veto_an_unambiguous_title() {
        let mut title = make_title();
        title.external_ids.push(ExternalId {
            source: "tvdb".to_string(),
            value: "425348".to_string(),
        });
        let mut subject = numbering_scoped_subject(&title, Some(1), Some(2));
        subject.tvdb_id = Some("425348".to_string());
        let mut candidate = make_candidate("Nightfall.S01E02.1080p.WEB-DL.x264-GRP", None);
        candidate.response_attributes.tvdb_id = Some("999999".to_string());

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked,
            "advisory indexer metadata must not override a valid unambiguous title match"
        );
        annotate_external_id_diagnostics(&mut candidate, &subject);
        assert!(
            candidate
                .extra
                .contains_key(EXTERNAL_ID_CONFLICTS_EXTRA_KEY)
        );
    }

    #[test]
    fn year_suffixed_title_pair_still_collides_and_bare_release_is_ambiguous() {
        // Adversarial-review regression: `Tide Chart` vs `Tide Chart (2023)` is
        // the commonest real collision shape; byte-equality collision
        // detection missed it, and the with_year matching bridge then
        // laundered the synthesized `tide chart 2023` key into a "unique
        // alias" disambiguator for a bare release.
        let (mut live_action, mut library) = tide_chart_library(Vec::new());
        live_action.name = "Tide Chart (2023)".to_string();
        library[0] = live_action.clone();

        let ambiguity = library_local_ambiguity(&live_action, &library);
        assert!(
            ambiguity.requires_disambiguator(),
            "year-suffixed pair must collide: {ambiguity:?}"
        );

        let mut subject = numbering_scoped_subject(&live_action, Some(2), Some(1));
        subject.title_evidence = subject.title_evidence.with_ambiguity(ambiguity);
        let candidate = make_candidate("Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP", None);
        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity,
            "a bare release must not clear the gate via a synthesized year key"
        );
    }

    #[test]
    fn blocklisted_release_reports_blocklisted_not_ambiguous() {
        // A burned release must never be re-parked for review: DbBlocklisted
        // outranks AmbiguousIdentity in the decision order.
        let (live_action, library) = tide_chart_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP", None);

        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures {
            release_names: HashSet::from([(
                String::new(),
                "tide.chart.s02e01.1080p.web-dl.x264-grp".to_string(),
            )]),
            info_hashes: HashSet::new(),
        };
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &live_action,
            subject: &subject,
            admission: &empty_admission(),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::DbBlocklisted
        );
    }

    #[test]
    fn unambiguous_title_demands_no_disambiguator() {
        // Pals is alone on its canonical key, so a bare scene release keeps
        // clearing the identity gate untouched.
        let mut title = make_title();
        title.id = "title-pals".to_string();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        title.aliases = Vec::new();
        title.tagged_aliases = Vec::new();
        let library = vec![title.clone()];
        let subject = ambiguous_episode_subject(&title, &library, Some(9), Some(23));
        assert!(!subject.title_evidence.ambiguity.requires_disambiguator());

        let candidate = make_candidate("Pals.S09E23E24.1080p.BluRay.x264-TENEIGHTY", None);
        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    // ── Indexer response attributes ─────────────────────────────────────────

    fn series_episode_candidate(
        release_title: &str,
        response_attributes: IndexerResponseAttributes,
    ) -> IndexerSearchResult {
        let mut candidate = make_candidate(release_title, None);
        candidate.response_attributes = response_attributes;
        candidate
    }

    fn response_categories(categories: &[&str]) -> IndexerResponseAttributes {
        IndexerResponseAttributes {
            categories: categories.iter().map(|value| value.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn anime_only_response_category_vetoes_a_series_subject() {
        // The indexer filed this under anime only, and the wanted item is a
        // plain series episode.
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = series_episode_candidate(
            "Pals.S09E23E24.1080p.BluRay.x264-TENEIGHTY",
            response_categories(&["5070"]),
        );

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::CategoryMismatch
        );
    }

    #[test]
    fn dual_categorized_response_clears_the_category_gate() {
        // The set rule: `5000` is a plain-TV assertion the series subject
        // satisfies, so the additional `5070` is not a contradiction.
        let mut title = make_title();
        title.name = "Pals".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = series_episode_candidate(
            "Pals.S09E23E24.1080p.BluRay.x264-TENEIGHTY",
            response_categories(&["5000", "5070"]),
        );

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_accepts_response_id_disambiguator() {
        // The indexer asserts the live-action title's own TVDB id, which
        // suffices on its own for a bare release name.
        let (live_action, library) = tide_chart_library(Vec::new());
        let mut subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        subject.tvdb_id = Some("393199".to_string());
        let candidate = series_episode_candidate(
            "Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP",
            IndexerResponseAttributes {
                tvdb_id: Some("393199".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_without_response_ids_stays_ambiguous() {
        // Same subject, same release name — only the indexer's id assertion is
        // missing, and absence is not a disambiguator.
        let (live_action, library) = tide_chart_library(Vec::new());
        let mut subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        subject.tvdb_id = Some("393199".to_string());
        let candidate = make_candidate("Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn category_contradiction_outranks_identity_ambiguity() {
        // The incident release is both anime-categorized and identity-ambiguous.
        // The category is the sharper, more actionable reason, so it reports
        // first.
        let (live_action, library) = tide_chart_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = series_episode_candidate(
            "Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP",
            response_categories(&["5070"]),
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::CategoryMismatch
        );
    }

    #[test]
    fn external_id_agreement_reports_only_comparable_kinds() {
        let response = IndexerResponseAttributes {
            tvdb_id: Some(" 393199 ".to_string()),
            imdb_id: Some("14688458".to_string()),
            ..Default::default()
        };

        assert_eq!(
            external_id_agreement(&response, Some("393199"), None, None),
            Some(true),
            "a trimmed numeric id agrees with the subject's own"
        );
        assert_eq!(
            external_id_agreement(&response, None, None, Some("tt14688458")),
            Some(true),
            "imdb ids agree once both sides are normalized"
        );
        assert_eq!(
            external_id_agreement(&response, Some("81189"), None, None),
            Some(false),
            "a comparable id that disagrees is a disagreement"
        );
        assert_eq!(
            external_id_agreement(&response, None, Some("140342"), None),
            None,
            "the indexer asserted no tmdb id, so there is nothing to compare"
        );
        assert_eq!(
            external_id_agreement(
                &IndexerResponseAttributes::default(),
                Some("393199"),
                None,
                None
            ),
            None,
            "a response with no ids is not evidence either way"
        );
    }

    #[test]
    fn candidate_matches_title_subject_uses_contextual_alias_parse_when_needed() {
        let mut title = make_title();
        title.name = "Silver Horizon Beyond the Vale".to_string();
        title.year = Some(2023);
        title.aliases = vec!["Sora no Vale".to_string()];
        title.tagged_aliases = vec![TaggedAlias {
            name: "Silver Horizon Beyond the Vale".to_string(),
            language: "eng".to_string(),
        }];

        let candidate = make_candidate(
            "[SubsPlease] Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01.[1080p].[HEVC]",
            None,
        );

        assert!(candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn candidate_parse_state_marks_ambiguous_parse() {
        let mut candidate = make_candidate("Nightfall.S01E01.1080p.WEB-DL", None);
        let parsed = candidate
            .parsed_release_metadata
            .as_mut()
            .expect("candidate has parsed metadata");
        parsed.is_ambiguous = true;
        parsed.disposition = ParseDisposition::Ambiguous;
        parsed.parse_hints.push("v2:ambiguous".to_string());

        assert_eq!(
            candidate_parse_state(&candidate),
            CandidateParseState::Ambiguous
        );
    }

    #[test]
    fn candidate_matches_existing_media_file_for_same_episode_release() {
        let candidate = make_candidate("Nightfall.S01E01.1080p.WEB-DL", None);
        let existing = vec![make_media_file(
            "Nightfall.S01E01.1080p.WEB-DL",
            Some("episode-1"),
        )];
        let occupied = crate::admission::AdmissionSubject::new(
            crate::admission::AdmissionScope::Episodes(vec!["episode-1".to_string()]),
            [(
                crate::admission::Incumbent {
                    tier_index: Some(1),
                    revision: 0,
                    file_id: existing[0].id.clone(),
                    file_path: existing[0].file_path.clone(),
                    release_group: None,
                    score: 900,
                    covers: vec!["episode-1".to_string()],
                    created_at: existing[0].created_at.clone(),
                },
                true,
            )],
        );
        let elsewhere = crate::admission::AdmissionSubject::new(
            crate::admission::AdmissionScope::Episodes(vec!["episode-2".to_string()]),
            [],
        );

        assert!(candidate_matches_existing_media_file(
            &candidate, &existing, &occupied
        ));
        assert!(!candidate_matches_existing_media_file(
            &candidate, &existing, &elsewhere
        ));
    }

    #[test]
    fn analyzed_cutoff_quality_matches_the_current_scope() {
        use crate::acquisition::decision_helpers::{
            CutoffScope, analyzed_cutoff_quality_for_scope,
        };

        let mut title_file = make_media_file("Nightfall.2022.1080p.WEB-DL", None);
        title_file.quality_label = Some("1080p".to_string());
        title_file.acquisition_score = Some(900);
        let episode_file = make_media_file("Nightfall.S01E01.1080p.WEB-DL", Some("episode-1"));
        let existing = vec![title_file, episode_file];

        assert_eq!(
            analyzed_cutoff_quality_for_scope(
                &existing,
                &CutoffScope::Episode("episode-1".to_string()),
            ),
            Some("720p")
        );
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&existing, &CutoffScope::Title),
            Some("1080p")
        );
    }

    /// **M3.** A pack scope has no episode id and no link id, so it used to fall
    /// into the title-scoped branch and match nothing for a series: a season
    /// entirely at cutoff read as "not reached" and could be re-fetched whole.
    ///
    /// The answer for a multi-member scope is the **weakest** member, and `None`
    /// while any member is empty — a season is at cutoff only when all of it is.
    #[test]
    fn a_pack_scope_reports_the_weakest_member_quality() {
        use crate::acquisition::decision_helpers::{
            CutoffScope, analyzed_cutoff_quality_for_scope,
        };

        let mut first = make_media_file("Nightfall.S01E01.1080p.WEB-DL", Some("episode-1"));
        first.quality_label = Some("1080p".to_string());
        // A high stored score on the weakest file used to win the election.
        let mut second = make_media_file("Nightfall.S01E02.720p.WEB-DL", Some("episode-2"));
        second.quality_label = Some("720p".to_string());
        second.acquisition_score = Some(9_000);
        let existing = vec![first, second];

        let members =
            |ids: &[&str]| CutoffScope::Episodes(ids.iter().map(|id| (*id).to_string()).collect());

        assert_eq!(
            analyzed_cutoff_quality_for_scope(&existing, &members(&["episode-1", "episode-2"])),
            Some("720p"),
            "the season is only as good as its worst episode"
        );
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&existing, &members(&["episode-1"])),
            Some("1080p")
        );
        assert_eq!(
            analyzed_cutoff_quality_for_scope(
                &existing,
                &members(&["episode-1", "episode-2", "episode-3"])
            ),
            None,
            "a missing member means the season has not reached any cutoff"
        );
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&existing, &members(&[])),
            None
        );
    }

    /// The cutoff-defining file is elected by **quality**, not by the stored
    /// `acquisition_score` — which is display history on an old scale and, before
    /// vetoes became verdicts, could be −10 000.
    #[test]
    fn the_cutoff_file_is_elected_by_quality_not_by_a_stored_score() {
        use crate::acquisition::decision_helpers::{
            CutoffScope, analyzed_cutoff_quality_for_scope,
        };

        let mut good = make_media_file("Nightfall.S01E01.2160p.WEB-DL", Some("episode-1"));
        good.quality_label = Some("2160p".to_string());
        good.acquisition_score = Some(-10_000);
        let mut poor = make_media_file("Nightfall.S01E01.720p.WEB-DL", Some("episode-1"));
        poor.quality_label = Some("720p".to_string());
        poor.acquisition_score = Some(9_000);
        let existing = vec![good, poor];

        assert_eq!(
            analyzed_cutoff_quality_for_scope(
                &existing,
                &CutoffScope::Episode("episode-1".to_string()),
            ),
            Some("2160p")
        );
    }

    /// A scope that is already occupied refuses a candidate it cannot beat — on
    /// the library's own evidence, not on a number remembered by the ledger row.
    #[test]
    fn an_occupied_scope_refuses_a_candidate_it_cannot_beat() {
        let title = make_title();
        let mut candidate = make_candidate("Nightfall.2022.1080p.WEB-DL", None);
        candidate.quality_profile_decision = Some(allowed_quality_decision(2400));
        let subject = ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(&title),
            queries: vec!["Nightfall".to_string()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            last_search_at: None,
            submission_scope: SubmissionScope::Title,
        };
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            // The premise is "something is already there", which is now a fact
            // about the library rather than a number on the ledger row.
            admission: &admission_holding(1_200),
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &HashSet::new(),
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::Eligible
        );
    }

    // ── The one cutoff gate, and the PROPER escape ──────────────────────────

    mod cutoff {
        use super::*;
        use crate::admission::{AdmissionScope, AdmissionSubject, CandidateFacts, Incumbent};

        fn now() -> DateTime<Utc> {
            DateTime::parse_from_rfc3339("2026-08-21T12:00:00Z")
                .expect("fixture timestamp")
                .with_timezone(&Utc)
        }

        /// A title scope holding one primary file at `score`, tier-neutral so the
        /// score is what decides.
        fn holding_at(score: i32) -> AdmissionSubject {
            AdmissionSubject::new(
                AdmissionScope::Title,
                [(
                    Incumbent {
                        tier_index: None,
                        revision: 0,
                        file_id: "file-1".to_string(),
                        file_path: "/data/Movies/Nightfall (2022)/Nightfall.mkv".to_string(),
                        release_group: None,
                        score,
                        covers: Vec::new(),
                        created_at: "2026-08-20T00:00:00Z".to_string(),
                    },
                    true,
                )],
            )
        }

        /// A plain title search subject for this fixture's title.
        fn title_subject(title: &Title) -> ResolvedReleaseSearchSubject {
            ResolvedReleaseSearchSubject {
                title_id: title.id.clone(),
                title_tags: title.tags.clone(),
                title_evidence: canonical_title_evidence(title),
                queries: vec![title.name.clone()],
                imdb_id: None,
                tmdb_id: None,
                tvdb_id: None,
                anidb_id: None,
                mal_id: None,
                category: title.facet.as_str().to_string(),
                owner_facet: title.facet.clone(),
                search_facet: title.facet.clone(),
                id_search_facet: None,
                newznab_categories: Vec::new(),
                runtime_minutes: title.runtime_minutes,
                season: None,
                episode: None,
                absolute_episode: None,
                subject_kind: ReleaseSearchSubjectKind::Title,
                last_search_at: None,
                submission_scope: SubmissionScope::Title,
            }
        }

        /// One primary file at tier 1, revision 0, imported `days_ago` days back.
        fn holding(revision: i32, days_ago: i64) -> AdmissionSubject {
            AdmissionSubject::new(
                AdmissionScope::Title,
                [(
                    Incumbent {
                        tier_index: Some(1),
                        revision,
                        file_id: "file-1".to_string(),
                        file_path: "/data/Movies/Nightfall (2022)/Nightfall.mkv".to_string(),
                        release_group: None,
                        score: 900,
                        covers: Vec::new(),
                        created_at: (now() - chrono::Duration::days(days_ago)).to_rfc3339(),
                    },
                    true,
                )],
            )
        }

        /// The cutoff has two halves and a scope is finished only when **both**
        /// have arrived, which is Sonarr's
        /// `CutoffNotMet = QualityCutoffNotMet || CustomFormatCutoffNotMet`.
        ///
        /// Gating on the quality alone would defeat the format-score cutoff:
        /// `derive_format_cutoff_targets` nominates exactly the scopes whose
        /// quality is fine and whose bar is below `cutoff_score`, and every lane
        /// would then refuse their candidates `CutoffReached`.
        #[test]
        fn the_cutoff_needs_both_the_quality_and_the_score() {
            // Quality at cutoff, bar 300, cutoff_score 500: not finished.
            let below = holding_at(300);
            assert!(!incumbent_at_cutoff(true, &below, Some(500)));
            // Bar 600: finished.
            let at = holding_at(600);
            assert!(incumbent_at_cutoff(true, &at, Some(500)));
            // No `cutoff_score` configured: the quality half decides alone.
            assert!(incumbent_at_cutoff(true, &below, None));
            assert!(!incumbent_at_cutoff(false, &at, Some(500)));
            // An unoccupied scope has no bar to fall short of; the quality half
            // is what keeps it out.
            let empty = AdmissionSubject::new(AdmissionScope::Title, []);
            assert!(incumbent_at_cutoff(true, &empty, Some(500)));
            assert!(!incumbent_at_cutoff(false, &empty, Some(500)));
        }

        /// …and through the real gate: a same-tier candidate scoring 600 over a
        /// bar of 300 is eligible while `cutoff_score` is 500, and refused once
        /// the bar reaches it.
        #[test]
        fn a_format_cutoff_target_is_still_grabbable() {
            let title = make_title();
            let subject = title_subject(&title);
            let mut candidate = make_candidate("Nightfall.2022.1080p.WEB-DL", None);
            candidate.quality_profile_decision = Some(allowed_quality_decision(600));

            let profile = QualityProfile::default();
            let thresholds = AcquisitionThresholds::default();
            let now = Utc::now();
            let db_blocklist =
                crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
            let no_minimum_seeders = HashMap::new();
            let no_unmonitored = HashSet::new();

            let evaluate = |admission: &AdmissionSubject| {
                let context = AutoCandidateEvaluationContext {
                    title: &title,
                    subject: &subject,
                    admission,
                    last_search_at: None,
                    profile: &profile,
                    thresholds: &thresholds,
                    incumbent_at_cutoff: incumbent_at_cutoff(true, admission, Some(500)),
                    is_rss_lane: true,
                    user_invoked: false,
                    oldest_overlapping_pending_published_at: None,
                    now: &now,
                    dl_snapshot: None,
                    db_blocklist: &db_blocklist,
                    existing_files: &[],
                    delay_profiles: &[],
                    failed_routes: None,
                    minimum_seeders: &no_minimum_seeders,
                    unmonitored_episode_ids: &no_unmonitored,
                };
                evaluate_auto_candidate(&candidate, &context)
            };

            let below = holding_at(300);
            assert_eq!(evaluate(&below), ReleaseAutoDecisionCode::Eligible);
            let at = holding_at(600);
            assert_eq!(
                evaluate(&at),
                ReleaseAutoDecisionCode::CutoffReached,
                "once the bar reaches the score cutoff the scope really is finished"
            );
        }

        /// A scope below cutoff never consults the candidate at all: the gate is
        /// the *scope's* state first.
        #[test]
        fn a_scope_below_cutoff_is_not_gated() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 0, 100),
                    &holding(0, 1),
                    false,
                    true,
                    &now(),
                ),
                None
            );
        }

        /// Sonarr's `QualityCutoffNotMet`: at cutoff, a plain release of the
        /// same quality has nothing to offer.
        #[test]
        fn a_non_revision_candidate_at_cutoff_is_refused() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 0, 9_000),
                    &holding(0, 1),
                    true,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::CutoffReached)
            );
        }

        /// …and the point of the whole change: a PROPER of the file at cutoff
        /// gets through. This is what the three lane-level
        /// `if cutoff_reached { return }` short-circuits made impossible.
        #[test]
        fn a_proper_escapes_the_cutoff_on_the_feed_lane() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 1),
                    true,
                    true,
                    &now(),
                ),
                None
            );
        }

        /// A better *tier* is not a revision upgrade, so it stays refused at
        /// cutoff — Scryer's cutoff means "good enough", and Sonarr's
        /// `IsRevisionUpgrade` is same-quality by construction.
        #[test]
        fn a_better_tier_candidate_is_still_refused_at_cutoff() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(0), 1, 9_000),
                    &holding(0, 1),
                    true,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::CutoffReached)
            );
        }

        /// Sonarr's `ProperSpecification`: a PROPER for a file imported more
        /// than a week ago is declined on age, with its own reason code so an
        /// operator can tell it from a plain cutoff.
        #[test]
        fn a_proper_for_a_week_old_file_is_refused_on_the_feed_lane() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 30),
                    true,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::ProperForOldFile)
            );
        }

        /// …and it is feed-only. `ProperSpecification.cs` accepts unconditionally
        /// when a search produced the candidate, so an operator's search (or the
        /// convergence lane, which annotates through the same path) is not
        /// subject to the age guard.
        #[test]
        fn the_old_file_guard_does_not_bind_an_active_search() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 30),
                    true,
                    false,
                    &now(),
                ),
                None
            );
        }

        /// An unoccupied scope cannot be at cutoff, but if a lane ever says it
        /// is, "no incumbent" must not read as "revision upgrade".
        #[test]
        fn an_unoccupied_scope_has_nothing_to_be_a_revision_of() {
            let empty = AdmissionSubject::new(AdmissionScope::Title, []);
            assert!(!candidate_is_revision_upgrade(
                CandidateFacts::new(Some(1), 2, 100),
                &empty
            ));
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 2, 100),
                    &empty,
                    true,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::CutoffReached)
            );
        }

        /// The window boundary, read the way Sonarr reads it: **midnight** minus
        /// seven days, against the file's import time — not a rolling 168 hours.
        #[test]
        fn the_proper_window_closes_at_utc_midnight_minus_seven_days() {
            use crate::acquisition_policy::file_predates_proper_window;
            let now = now();
            // Midnight − 7 days is 2026-08-14T00:00:00Z.
            assert!(file_predates_proper_window(
                Some("2026-08-13T23:59:59Z"),
                &now
            ));
            assert!(!file_predates_proper_window(
                Some("2026-08-14T00:00:00Z"),
                &now
            ));
            // 7 days and 2 hours ago is *inside* the window, because the
            // boundary is a calendar day rather than an elapsed duration.
            assert!(!file_predates_proper_window(
                Some("2026-08-14T10:00:00Z"),
                &now
            ));
            // An absent or unreadable import time never refuses.
            assert!(!file_predates_proper_window(None, &now));
            assert!(!file_predates_proper_window(Some("whenever"), &now));
        }

        /// **Final review M1.** The old-file guard is cutoff-independent. A
        /// PROPER for a month-old file is declined on the feed lane whether or
        /// not the scope has reached its cutoff — Sonarr's `ProperSpecification`
        /// never looks at the cutoff — and a PROPER for a fresh below-cutoff
        /// file is still the revision upgrade the ladder admits.
        #[test]
        fn the_old_file_guard_binds_below_the_cutoff_too() {
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 30),
                    false,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::ProperForOldFile),
                "a below-cutoff scope holding an old file must still refuse the PROPER on the feed lane"
            );
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 1),
                    false,
                    true,
                    &now(),
                ),
                None,
                "a PROPER over a fresh below-cutoff file is an ordinary revision upgrade"
            );
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &holding(0, 30),
                    false,
                    false,
                    &now(),
                ),
                None,
                "an active search is never subject to the age guard"
            );
        }

        /// The guard reads every incumbent, not only the best: a pack or
        /// multi-episode scope whose weaker member is a month old still refuses
        /// the PROPER that would replace it.
        #[test]
        fn the_old_file_guard_reads_every_incumbent_not_only_the_best() {
            let member = |id: &str, score: i32, days_ago: i64, covers: &str| {
                (
                    Incumbent {
                        tier_index: Some(1),
                        revision: 0,
                        file_id: id.to_string(),
                        file_path: format!("/data/TV/{id}.mkv"),
                        release_group: None,
                        score,
                        covers: vec![covers.to_string()],
                        created_at: (now() - chrono::Duration::days(days_ago)).to_rfc3339(),
                    },
                    true,
                )
            };
            let subject = AdmissionSubject::new(
                AdmissionScope::Episodes(vec!["ep-01".to_string(), "ep-02".to_string()]),
                [
                    member("fresh", 950, 1, "ep-01"),
                    member("old", 900, 30, "ep-02"),
                ],
            );
            assert_eq!(
                cutoff_refusal(
                    CandidateFacts::new(Some(1), 1, 100),
                    &subject,
                    false,
                    true,
                    &now(),
                ),
                Some(ReleaseAutoDecisionCode::ProperForOldFile),
                "the best incumbent is fresh, but the PROPER would also replace the old one"
            );
        }
    }

    /// Sonarr's `MonitoredEpisodeSpecification`: a batch that reaches
    /// into an episode nobody is monitoring brings unwanted bytes with the
    /// wanted ones, and there is no way to take half a file.
    #[test]
    fn a_batch_touching_an_unmonitored_episode_is_refused() {
        let title = make_title();
        let subject = episode_set_subject(&title, &["episode-1", "episode-2"]);
        let mut candidate = make_candidate("Nightfall.S01E01-E02.1080p.WEB-DL", None);
        candidate.quality_profile_decision = Some(allowed_quality_decision(900));
        candidate.coverage_scope = Some(SubmissionScope::EpisodeSet {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
        });

        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let admission = empty_admission();
        let unmonitored: HashSet<String> = ["episode-2".to_string()].into_iter().collect();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &admission,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &unmonitored,
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::EpisodeNotMonitored
        );

        // An operator-started search skips the monitored check the way
        // Sonarr's spec does for user searches: the same batch is fine.
        let context = AutoCandidateEvaluationContext {
            user_invoked: true,
            ..context
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::Eligible
        );

        // Every episode monitored: the same batch is fine.
        let all_monitored = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            user_invoked: false,
            unmonitored_episode_ids: &all_monitored,
            ..context
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::Eligible
        );
    }

    /// A **Collection** scope is exempt. A season pack's scope already *is* its
    /// monitored members, and refusing the whole season because one episode is
    /// unmonitored would reintroduce the partial-monitoring trap.
    #[test]
    fn a_season_pack_is_exempt_from_the_unmonitored_refusal() {
        let title = make_title();
        let mut subject = episode_set_subject(&title, &["episode-1", "episode-2"]);
        subject.submission_scope = SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        };
        let mut candidate = make_candidate("Nightfall.S01.1080p.WEB-DL", None);
        candidate.quality_profile_decision = Some(allowed_quality_decision(900));
        candidate.coverage_scope = Some(SubmissionScope::EpisodeSet {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
        });

        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
        let no_minimum_seeders = HashMap::new();
        let admission = empty_admission();
        let unmonitored: HashSet<String> = ["episode-2".to_string()].into_iter().collect();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            admission: &admission,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            incumbent_at_cutoff: false,
            is_rss_lane: false,
            user_invoked: false,
            oldest_overlapping_pending_published_at: None,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_routes: None,
            minimum_seeders: &no_minimum_seeders,
            unmonitored_episode_ids: &unmonitored,
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::Eligible
        );
    }

    // ── The anti-loop check and unmonitored episodes ─────────────────────────

    mod already_imported {
        use super::*;

        fn subject_holding(file: &TitleMediaFile) -> crate::admission::AdmissionSubject {
            crate::admission::AdmissionSubject::new(
                crate::admission::AdmissionScope::Episodes(vec!["episode-1".to_string()]),
                [(
                    crate::admission::Incumbent {
                        tier_index: Some(1),
                        revision: 0,
                        file_id: file.id.clone(),
                        file_path: file.file_path.clone(),
                        release_group: None,
                        score: 900,
                        covers: vec!["episode-1".to_string()],
                        created_at: file.created_at.clone(),
                    },
                    true,
                )],
            )
        }

        /// Re-grabbing the identical release is never an upgrade, whichever name
        /// the row remembers it by.
        #[test]
        fn the_same_release_is_recognised_by_every_name_the_row_keeps() {
            const RELEASE: &str = "Nightfall.S01E01.1080p.WEB-DL-GRP";
            let release = RELEASE;
            let mutations: [fn(&mut TitleMediaFile); 4] = [
                |file| file.grabbed_release_title = Some(RELEASE.to_string()),
                |file| file.scene_name = Some(RELEASE.to_string()),
                |file| file.file_path = format!("/data/TV/Nightfall/{RELEASE}.mkv"),
                |file| {
                    file.original_file_path = Some(format!("/downloads/{RELEASE}/{RELEASE}.mkv"));
                },
            ];
            for mutate in mutations {
                let mut file = make_media_file("Something.Else", Some("episode-1"));
                file.grabbed_release_title = None;
                file.scene_name = None;
                file.original_file_path = None;
                mutate(&mut file);
                let subject = subject_holding(&file);
                assert!(
                    candidate_matches_existing_media_file(
                        &make_candidate(release, None),
                        std::slice::from_ref(&file),
                        &subject,
                    ),
                    "the identical release was not recognised: {file:?}"
                );
            }
        }

        /// **The defect.** The old rule matched on `contains` against whole
        /// paths, so `Nightfall.S01E01.1080p-GRP` matched a stored path of
        /// `.../Nightfall.S01E01.1080p-GRP.PROPER.mkv` — and the PROPER of a
        /// release already on disk reported "already active" and could never be
        /// grabbed. Comparing file *stems* exactly is what fixes it, and it is
        /// what makes the revision comparison reachable at all.
        #[test]
        fn a_proper_of_a_release_on_disk_is_still_grabbable() {
            let mut file = make_media_file("Nightfall.S01E01.1080p.WEB-DL-GRP", Some("episode-1"));
            file.grabbed_release_title = Some("Nightfall.S01E01.1080p.WEB-DL-GRP".to_string());
            file.scene_name = None;
            file.file_path = "/data/TV/Nightfall/Nightfall.S01E01.1080p.WEB-DL-GRP.mkv".to_string();
            file.original_file_path = None;
            let subject = subject_holding(&file);

            assert!(!candidate_matches_existing_media_file(
                &make_candidate("Nightfall.S01E01.1080p.WEB-DL.PROPER-GRP", None),
                std::slice::from_ref(&file),
                &subject,
            ));
            // …and the release itself is still recognised, so the anti-loop
            // guard has not simply been switched off.
            assert!(candidate_matches_existing_media_file(
                &make_candidate("Nightfall.S01E01.1080p.WEB-DL-GRP", None),
                std::slice::from_ref(&file),
                &subject,
            ));
        }

        /// Membership is by the subject's incumbent ids, so a file that is not
        /// in the scope cannot report a match — which is what the old scalar
        /// `episode_id` filter got wrong for every pack, batch, title and link
        /// scope (it matched *any* file of the title).
        #[test]
        fn a_file_outside_the_scope_is_not_consulted() {
            let release = "Nightfall.S01E01.1080p.WEB-DL-GRP";
            let mut file = make_media_file(release, Some("episode-1"));
            file.grabbed_release_title = Some(release.to_string());
            let empty_subject = crate::admission::AdmissionSubject::new(
                crate::admission::AdmissionScope::Episodes(vec!["episode-1".to_string()]),
                [],
            );

            assert!(!candidate_matches_existing_media_file(
                &make_candidate(release, None),
                std::slice::from_ref(&file),
                &empty_subject,
            ));
        }
    }
}
