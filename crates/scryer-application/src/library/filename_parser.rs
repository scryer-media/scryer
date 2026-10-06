use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;
use scryer_domain::{MediaFacet, MovieEntity, SeriesMovieLink, VIDEO_EXTENSIONS};
use unicode_normalization::UnicodeNormalization;

use super::*;
use crate::helpers::{
    has_usable_release_title_signal, normalize_release_title_signal, parse_usable_release_title,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryFilenameParseMode {
    TitleOnly,
    TitleScan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryFilenameFallbackPolicy {
    Never,
    WhenNeeded,
    NeedReleaseMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryFilenameParseStrategy {
    SimplePath,
    ExistingRecord,
    ReleaseParserFallback,
    Unparseable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LibraryTitleWalk {
    pub(crate) title: Option<String>,
    pub(crate) year: Option<u32>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
}

impl LibraryTitleWalk {
    pub(crate) fn has_external_ids(&self) -> bool {
        self.imdb_id.is_some() || self.tmdb_id.is_some() || self.tvdb_id.is_some()
    }

    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.year.is_none()
            && self.imdb_id.is_none()
            && self.tmdb_id.is_none()
            && self.tvdb_id.is_none()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LibraryQueryEvidence {
    pub(crate) queries: Vec<String>,
    pub(crate) year: Option<u32>,
    /// Year derived from the containing folder alone, independent of the
    /// `year` selection order above. Scans use it to retry a metadata lookup
    /// when the filename year and the folder year disagree.
    pub(crate) folder_year: Option<u32>,
    pub(crate) file_walk: Option<LibraryTitleWalk>,
    pub(crate) folder_walk: Option<LibraryTitleWalk>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LibraryFilenameExistingRecord<'a> {
    pub(crate) episode_id: Option<&'a str>,
    pub(crate) snapshot_matches: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryFilenameParseInput<'a> {
    pub(crate) path: &'a Path,
    pub(crate) display_name: Option<&'a str>,
    pub(crate) library_root: Option<&'a Path>,
    pub(crate) title: Option<&'a Title>,
    pub(crate) facet: Option<&'a MediaFacet>,
    pub(crate) collections: &'a [Collection],
    pub(crate) series_movie_links: &'a [SeriesMovieLink],
    pub(crate) episodes: &'a [Episode],
    pub(crate) existing_record: Option<LibraryFilenameExistingRecord<'a>>,
    pub(crate) mode: LibraryFilenameParseMode,
    pub(crate) fallback_policy: LibraryFilenameFallbackPolicy,
    /// Community (per-cour) anime numbering for this title, when the catalog
    /// stores one. `None` for every non-anime title and for anime whose
    /// community numbering already matches the catalog's.
    pub(crate) anime_numbering_bridge: Option<&'a scryer_domain::AnimeNumberingBridge>,
}

impl<'a> LibraryFilenameParseInput<'a> {
    pub(crate) fn title_only(path: &'a Path, library_root: Option<&'a Path>) -> Self {
        Self {
            path,
            display_name: None,
            library_root,
            title: None,
            facet: None,
            collections: &[],
            series_movie_links: &[],
            episodes: &[],
            existing_record: None,
            mode: LibraryFilenameParseMode::TitleOnly,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
            anime_numbering_bridge: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LibraryFilenameSeriesMovieTarget {
    pub(crate) series_movie_link_id: String,
    pub(crate) movie: MovieEntity,
    pub(crate) linked_episode: Option<Episode>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LibraryFilenameTarget {
    TitleOnly,
    Episodes {
        episode_identity: crate::ParsedEpisodeMetadata,
        episodes: Vec<Episode>,
        /// Every episode key the filename names resolved, each to a single
        /// catalog episode. A partial range or a key the catalog holds twice
        /// still places the file, but never confidently.
        exact: bool,
    },
    SeriesMovie(Box<LibraryFilenameSeriesMovieTarget>),
    Unmatched {
        reason: &'static str,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryFilenameParse {
    pub(crate) query_evidence: LibraryQueryEvidence,
    pub(crate) parsed_release: crate::ParsedReleaseMetadata,
    pub(crate) episode_identity: Option<crate::ParsedEpisodeMetadata>,
    pub(crate) target: LibraryFilenameTarget,
    pub(crate) strategy: LibraryFilenameParseStrategy,
    pub(crate) release_fallback_used: bool,
}

impl LibraryFilenameParse {
    pub(crate) fn target_episodes(&self) -> Vec<Episode> {
        match &self.target {
            LibraryFilenameTarget::Episodes { episodes, .. } => episodes.clone(),
            LibraryFilenameTarget::SeriesMovie(target) => {
                target.linked_episode.iter().cloned().collect()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn target_series_movie_link_id(&self) -> Option<&str> {
        match &self.target {
            LibraryFilenameTarget::SeriesMovie(target) => {
                Some(target.series_movie_link_id.as_str())
            }
            _ => None,
        }
    }

    pub(crate) fn unmatched_reason(&self) -> Option<&'static str> {
        match &self.target {
            LibraryFilenameTarget::Unmatched { reason } => Some(reason),
            _ => None,
        }
    }

    /// The episodes this parse places the file on when it is confident enough
    /// to overrule the links a scan stored earlier.
    ///
    /// Only a fresh filename parse qualifies: an `ExistingRecord` parse echoes
    /// the stored link back, and title-only strategies never resolve episodes.
    /// Every refusal stays a refusal (anime numbering that is ambiguous or an
    /// unresolved pack, ambiguous or linked series movies, failed lookups), and
    /// so does a parse that only names a season, part of a season, several
    /// seasons, season extras or a whole series, because a single file
    /// resolved from such a token is a guess. An explicit episode range
    /// (`S01E01E02`, `- 01-02`) names its episodes and qualifies; that is how
    /// multi-episode files are written. A special (OVA, movie, extra)
    /// qualifies only when it resolved to
    /// season-zero episodes; the numbering of a special named anywhere else is
    /// not trustworthy enough to move a link. The resolution must also be
    /// exact: a range the catalog only partly holds, a key two catalog
    /// episodes share, or an air date several episodes share without a part
    /// number never qualifies.
    pub(crate) fn confident_episode_target(&self) -> Option<&[Episode]> {
        if self.strategy != LibraryFilenameParseStrategy::ReleaseParserFallback {
            return None;
        }
        let LibraryFilenameTarget::Episodes {
            episode_identity,
            episodes,
            exact,
        } = &self.target
        else {
            return None;
        };
        if !exact {
            return None;
        }
        let names_concrete_episodes = !episode_identity.episode_numbers.is_empty()
            || !episode_identity.absolute_episode_numbers.is_empty()
            || episode_identity.absolute_episode.is_some()
            || !episode_identity.special_absolute_episode_numbers.is_empty()
            || episode_identity.air_date.is_some();
        let names_a_pack = episode_identity.full_season
            || episode_identity.is_partial_season
            || episode_identity.is_multi_season
            || episode_identity.is_series_pack
            || episode_identity.is_season_extra
            || episode_identity.release_type == crate::ParsedEpisodeReleaseType::SeasonPack;
        let special_outside_season_zero = episode_identity.special_kind.is_some()
            && !episodes
                .iter()
                .all(|episode| episode.season_number.as_deref() == Some("0"));
        (names_concrete_episodes
            && !names_a_pack
            && !special_outside_season_zero
            && !episodes.is_empty())
        .then_some(episodes.as_slice())
    }
}

/// Catalog lookups every filename parse of one title derives before it looks
/// at the filename: the release parse context and the episode lookup (with the
/// title's absolute scale). A scan hands one index to every parse of a title so
/// they are built at most once per title instead of once per file. Built lazily
/// on the first parse that needs them.
///
/// Only share an index between parses whose title, facet, collections, series
/// movie links and episodes are the same.
#[derive(Default)]
pub(crate) struct LibraryFilenameTitleIndex {
    prepared: std::sync::OnceLock<PreparedLibraryFilenameTitleIndex>,
}

struct PreparedLibraryFilenameTitleIndex {
    release_context: Option<crate::ReleaseParseContext>,
    episode_lookup: EpisodeLookup,
}

impl LibraryFilenameTitleIndex {
    fn prepared(
        &self,
        input: &LibraryFilenameParseInput<'_>,
    ) -> &PreparedLibraryFilenameTitleIndex {
        self.prepared
            .get_or_init(|| PreparedLibraryFilenameTitleIndex {
                release_context: build_release_parse_context_for_library_filename(input),
                episode_lookup: build_episode_lookup(input.collections, input.episodes),
            })
    }
}

struct QueryEvidenceBuild {
    evidence: LibraryQueryEvidence,
    parsed_release: Option<crate::ParsedReleaseMetadata>,
}

pub(crate) fn parse_library_filename(
    input: &LibraryFilenameParseInput<'_>,
) -> LibraryFilenameParse {
    parse_library_filename_with_index(input, None)
}

/// [`parse_library_filename`], reusing `title_index` for the title's catalog
/// lookups instead of building them for this one parse.
pub(crate) fn parse_library_filename_with_index(
    input: &LibraryFilenameParseInput<'_>,
    title_index: Option<&LibraryFilenameTitleIndex>,
) -> LibraryFilenameParse {
    let allow_title_release_fallback = input.mode == LibraryFilenameParseMode::TitleOnly
        && input.fallback_policy != LibraryFilenameFallbackPolicy::Never;
    let query_build =
        build_library_query_evidence(input.path, input.library_root, allow_title_release_fallback);
    let raw_name = filename_parse_raw_name(input.path, input.display_name);
    let mut parsed_release = query_build
        .parsed_release
        .unwrap_or_else(|| synthesize_release_metadata(&raw_name, input, None));
    let mut release_fallback_used = parsed_release.parser_version != "library_filename_parser";

    if input.mode == LibraryFilenameParseMode::TitleOnly {
        let strategy = if query_build.evidence.queries.is_empty() {
            LibraryFilenameParseStrategy::Unparseable
        } else if release_fallback_used {
            LibraryFilenameParseStrategy::ReleaseParserFallback
        } else {
            LibraryFilenameParseStrategy::SimplePath
        };
        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release,
            episode_identity: None,
            target: LibraryFilenameTarget::TitleOnly,
            strategy,
            release_fallback_used,
        };
    }

    if let Some(existing) = input.existing_record
        && existing.snapshot_matches
        && let Some(episode_id) = existing.episode_id
        && let Some(episode) = input
            .episodes
            .iter()
            .find(|episode| episode.id == episode_id)
    {
        let episode_identity = parsed_episode_metadata_from_episode(episode);
        parsed_release =
            synthesize_release_metadata(&raw_name, input, Some(episode_identity.clone()));
        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release,
            episode_identity: Some(episode_identity.clone()),
            target: LibraryFilenameTarget::Episodes {
                episode_identity,
                episodes: vec![episode.clone()],
                exact: true,
            },
            strategy: LibraryFilenameParseStrategy::ExistingRecord,
            release_fallback_used: false,
        };
    }

    let mut fallback = parse_release_fallback(input, &raw_name, title_index);
    release_fallback_used = true;
    // A library file may be named in the community's per-cour numbering while
    // the catalog follows TVDB's official order. Translate before resolving so
    // the file lands on the episode it actually holds; without a bridge this
    // leaves the parse untouched.
    if let Some(title) = input.title {
        let numbering = crate::anime_numbering::translate_release_numbering(
            input.anime_numbering_bridge,
            title,
            input.episodes,
            &mut fallback,
            None,
        );
        if numbering.is_ambiguous()
            || matches!(
                numbering,
                crate::anime_numbering::NumberingResolution::UnresolvedPack
            )
        {
            return LibraryFilenameParse {
                query_evidence: query_build.evidence,
                parsed_release: fallback,
                episode_identity: None,
                target: LibraryFilenameTarget::Unmatched {
                    reason: if numbering.is_ambiguous() {
                        "anime_numbering_ambiguous"
                    } else {
                        "unresolved_pack_scope"
                    },
                },
                strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
                release_fallback_used,
            };
        }
    }
    if matches!(
        match_series_movie_filename(input.series_movie_links, &raw_name, fallback.year),
        SeriesMovieFilenameMatch::Ambiguous
    ) {
        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release: fallback,
            episode_identity: None,
            target: LibraryFilenameTarget::Unmatched {
                reason: "series_movie_ambiguous",
            },
            strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
            release_fallback_used,
        };
    }
    let fallback_episode = fallback.episode.clone();
    if fallback_episode.is_some()
        && !raw_name_has_explicit_episode_marker(&raw_name)
        && let Some(series_movie) = resolve_series_movie_from_name(input, &raw_name, fallback.year)
    {
        let episode_identity = series_movie
            .linked_episode
            .as_ref()
            .map(parsed_episode_metadata_from_episode);
        fallback.episode = episode_identity.clone();
        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release: fallback,
            episode_identity,
            target: LibraryFilenameTarget::SeriesMovie(Box::new(series_movie)),
            strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
            release_fallback_used,
        };
    }
    if let Some(episode_identity) = fallback_episode.clone() {
        if let Some(series_movie) =
            resolve_series_movie_from_episode_identity(input, &episode_identity)
        {
            return LibraryFilenameParse {
                query_evidence: query_build.evidence,
                parsed_release: fallback,
                episode_identity: Some(episode_identity),
                target: LibraryFilenameTarget::SeriesMovie(Box::new(series_movie)),
                strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
                release_fallback_used,
            };
        }

        let season_str = episode_identity.season.unwrap_or(1).to_string();
        let owned_lookup;
        let lookup = match title_index {
            Some(title_index) => &title_index.prepared(input).episode_lookup,
            None => {
                owned_lookup = build_episode_lookup(input.collections, input.episodes);
                &owned_lookup
            }
        };
        let ResolvedEpisodes { episodes, exact } =
            resolve_episodes_from_identity_with_season(&episode_identity, &season_str, lookup);
        if !episodes.is_empty() {
            return LibraryFilenameParse {
                query_evidence: query_build.evidence,
                parsed_release: fallback,
                episode_identity: Some(episode_identity.clone()),
                target: LibraryFilenameTarget::Episodes {
                    episode_identity,
                    episodes,
                    exact,
                },
                strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
                release_fallback_used,
            };
        }

        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release: fallback,
            episode_identity: Some(episode_identity),
            target: LibraryFilenameTarget::Unmatched {
                reason: "episode_lookup_failed",
            },
            strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
            release_fallback_used,
        };
    }

    if let Some(series_movie) = resolve_series_movie_from_name(input, &raw_name, fallback.year) {
        let episode_identity = series_movie
            .linked_episode
            .as_ref()
            .map(parsed_episode_metadata_from_episode);
        if fallback.episode.is_none() {
            fallback.episode = episode_identity.clone();
        }
        return LibraryFilenameParse {
            query_evidence: query_build.evidence,
            parsed_release: fallback,
            episode_identity,
            target: LibraryFilenameTarget::SeriesMovie(Box::new(series_movie)),
            strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
            release_fallback_used,
        };
    }

    LibraryFilenameParse {
        query_evidence: query_build.evidence,
        parsed_release: fallback,
        episode_identity: None,
        target: LibraryFilenameTarget::Unmatched {
            reason: "episode_identity_missing",
        },
        strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
        release_fallback_used,
    }
}

pub(crate) fn library_title_walk(raw: &str) -> Option<LibraryTitleWalk> {
    let (without_ids, mut walk) = extract_library_title_ids(raw);
    let normalized = normalize_library_title_text(&without_ids);

    if let Some((title, year)) = parse_simple_library_title_year(normalized.as_str()) {
        walk.title = Some(title);
        walk.year = Some(year);
    } else if walk.has_external_ids() {
        walk.title = fallback_title_from_id_text(normalized.as_str());
    }

    (!walk.is_empty()).then_some(walk)
}

pub(crate) fn normalize_folder_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_space = false;
    for ch in name.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

pub(crate) fn strip_year_suffix(folder: &str) -> (String, Option<u32>) {
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(close_pos) = folder.rfind(close)
            && let Some(open_pos) = folder[..close_pos].rfind(open)
            && let Ok(year) = folder[open_pos + 1..close_pos].trim().parse::<u32>()
            && (1888..=2100).contains(&year)
        {
            let title = folder[..open_pos].trim_end().to_string();
            if !title.is_empty() {
                return (title, Some(year));
            }
        }
    }

    (folder.to_string(), None)
}

fn build_library_query_evidence(
    path: &Path,
    library_root: Option<&Path>,
    allow_release_fallback: bool,
) -> QueryEvidenceBuild {
    let root = library_root.map(Path::to_path_buf);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file_walk = library_title_walk(stem.as_str());
    let parsed = allow_release_fallback
        .then(|| normalize_release_title_signal(crate::parse_release_metadata(stem.as_str())));
    let parsed_has_usable_title_signal =
        parsed.as_ref().is_some_and(has_usable_release_title_signal);
    let parsed_queries = parsed
        .as_ref()
        .filter(|_| parsed_has_usable_title_signal)
        .map(|parsed| {
            if parsed.normalized_title_variants.is_empty() {
                vec![parsed.normalized_title.clone()]
            } else {
                parsed.normalized_title_variants.clone()
            }
        })
        .unwrap_or_default();

    let mut queries = Vec::new();
    let mut seen_normalized = HashSet::new();
    let mut folder_year = None;
    let mut folder_queries = Vec::new();
    let mut folder_walk = None;
    let mut raw_folder_query = None;

    if let Some(parent) = path.parent()
        && root.as_deref() != Some(parent)
        && let Some(folder_name) = parent
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty())
    {
        let clean = normalize_folder_name(&folder_name);
        folder_walk = library_title_walk(folder_name.as_str());
        let (clean_title, clean_year) = strip_year_suffix(&clean);
        let parsed_folder = allow_release_fallback
            .then(|| parse_usable_release_title(&folder_name))
            .flatten();
        if let Some(parsed_folder) = parsed_folder {
            let looks_human_named = !folder_name.contains('.') && !folder_name.contains('_');
            let has_release_decoration = parsed_folder
                .release_group
                .as_ref()
                .is_some_and(|group| !group.trim().is_empty())
                || parsed_folder.quality.is_some()
                || parsed_folder.source.is_some()
                || parsed_folder.video_codec.is_some()
                || parsed_folder.video_encoding.is_some()
                || parsed_folder.audio.is_some()
                || !parsed_folder.audio_codecs.is_empty()
                || parsed_folder.audio_channels.is_some()
                || parsed_folder.streaming_service.is_some()
                || parsed_folder.edition.is_some()
                || parsed_folder.is_proper_upload
                || parsed_folder.is_repack
                || parsed_folder.is_remux
                || parsed_folder.is_bd_disk
                || parsed_folder.is_dual_audio
                || parsed_folder.episode.is_some();
            let raw_folder_title = parsed_folder
                .year
                .and_then(|year| u32::try_from(year).ok())
                .map(|year| strip_trailing_plain_year_token(&clean_title, year))
                .unwrap_or_else(|| clean_title.clone());
            if !clean_title.trim().is_empty()
                && !has_release_decoration
                && (clean_year.is_some() || parsed_folder.year.is_some() || looks_human_named)
            {
                raw_folder_query = Some(raw_folder_title);
            }
            let parsed_folder_queries = if parsed_folder.normalized_title_variants.is_empty() {
                vec![parsed_folder.normalized_title.clone()]
            } else {
                parsed_folder.normalized_title_variants.clone()
            };
            folder_queries.extend(parsed_folder_queries);
            folder_year = parsed_folder.year.and_then(|year| u32::try_from(year).ok());
            if folder_year.is_none() {
                folder_year = clean_year;
            }
        } else if !clean_title.trim().is_empty() {
            folder_queries.push(clean_title);
            folder_year = clean_year;
        }
    }

    if let Some(title) = file_walk.as_ref().and_then(|walk| walk.title.clone()) {
        push_unique_query(&mut queries, &mut seen_normalized, title);
    }

    if !parsed_has_usable_title_signal {
        for folder_query in folder_queries.iter().cloned() {
            push_unique_query(&mut queries, &mut seen_normalized, folder_query);
        }
    }

    for query in parsed_queries {
        if let Some(reduced) = part_reduced_query(query.as_str()) {
            push_unique_query(&mut queries, &mut seen_normalized, reduced);
        } else {
            push_unique_query(&mut queries, &mut seen_normalized, query);
        }
    }

    if parsed_has_usable_title_signal {
        for folder_query in folder_queries {
            push_unique_query(&mut queries, &mut seen_normalized, folder_query);
        }
    }

    if let Some(title) = folder_walk.as_ref().and_then(|walk| walk.title.clone()) {
        push_unique_query(&mut queries, &mut seen_normalized, title);
    }

    if let Some(raw_folder_query) = raw_folder_query {
        push_unique_literal_query(&mut queries, raw_folder_query);
    }

    let folder_derived_year = folder_walk
        .as_ref()
        .and_then(|walk| walk.year)
        .or(folder_year);

    let year = file_walk
        .as_ref()
        .and_then(|walk| walk.year)
        .or_else(|| {
            parsed_has_usable_title_signal
                .then_some(parsed.as_ref().and_then(|parsed| parsed.year))
                .flatten()
                .and_then(|year| u32::try_from(year).ok())
        })
        .or(folder_derived_year);

    QueryEvidenceBuild {
        evidence: LibraryQueryEvidence {
            queries,
            year,
            folder_year: folder_derived_year,
            file_walk,
            folder_walk,
        },
        parsed_release: parsed,
    }
}

fn filename_parse_raw_name(path: &Path, display_name: Option<&str>) -> String {
    let raw_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .or(display_name)
        .unwrap_or_default()
        .trim();
    strip_generated_restore_suffix(raw_name).to_string()
}

fn strip_generated_restore_suffix(raw_name: &str) -> &str {
    if let Some(prefix) = raw_name.strip_suffix("-restored") {
        return prefix.trim_end();
    }
    if let Some((prefix, suffix)) = raw_name.rsplit_once("-restored-")
        && !suffix.is_empty()
        && suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return prefix.trim_end();
    }
    raw_name
}

/// Episodes one filename identity resolves to, and whether that resolution is
/// exact: every lane that contributed episodes resolved each key the filename
/// names, and each of those keys named exactly one catalog episode.
struct ResolvedEpisodes {
    episodes: Vec<Episode>,
    exact: bool,
}

/// How one resolution lane went: the distinct keys the filename named, how
/// many of them the catalog resolved, and whether any resolved key was shared
/// by more than one catalog episode.
#[derive(Default)]
struct LaneOutcome {
    named: usize,
    resolved: usize,
    ambiguous: bool,
}

impl LaneOutcome {
    /// A lane that contributed nothing leaves exactness to the lanes that
    /// did; one that contributed must have resolved every key it named, each
    /// to a single episode.
    fn keeps_resolution_exact(&self) -> bool {
        self.resolved == 0 || (self.resolved == self.named && !self.ambiguous)
    }
}

fn distinct_numbers(numbers: &[u32]) -> Vec<u32> {
    let mut seen = HashSet::new();
    numbers
        .iter()
        .copied()
        .filter(|number| seen.insert(*number))
        .collect()
}

fn resolve_episodes_from_identity_with_season(
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
    lookup: &EpisodeLookup,
) -> ResolvedEpisodes {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    let mut lanes = Vec::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let mut lane = LaneOutcome {
            named: 1,
            ..LaneOutcome::default()
        };
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        if let Some(matches) = lookup.by_air_date.get(&air_date_str) {
            if let Some(part) = ep_meta.daily_part {
                let part_index = part.saturating_sub(1) as usize;
                if let Some(episode) = matches.get(part_index) {
                    lane.resolved = 1;
                    if seen.insert(episode.id.clone()) {
                        resolved.push(episode.clone());
                    }
                }
            } else if !matches.is_empty() {
                // A date shared by several episodes, with no part to pick
                // one, names all of them only by guess.
                lane.resolved = 1;
                lane.ambiguous = matches.len() > 1;
                for episode in matches {
                    if seen.insert(episode.id.clone()) {
                        resolved.push(episode.clone());
                    }
                }
            }
        }
        lanes.push(lane);
    }

    let episode_numbers = distinct_numbers(&ep_meta.episode_numbers);
    let mut episode_lane = LaneOutcome {
        named: episode_numbers.len(),
        ..LaneOutcome::default()
    };
    for episode_number in &episode_numbers {
        let key = (target_season.clone(), episode_number.to_string());
        if let Some(episode) = lookup.by_collection_episode.get(&key) {
            episode_lane.resolved += 1;
            episode_lane.ambiguous |= lookup.ambiguous_collection_episodes.contains(&key);
            if seen.insert(episode.id.clone()) {
                resolved.push(episode.clone());
            }
        }
    }
    lanes.push(episode_lane);

    if resolved.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        && let Some(collection_episodes) = lookup.by_collection_index.get(&target_season)
    {
        for episode in collection_episodes {
            if episode.season_number.as_deref() == Some(target_season.as_str())
                && seen.insert(episode.id.clone())
            {
                resolved.push(episode.clone());
            }
        }
    }

    if resolved.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        let special_numbers = distinct_numbers(&ep_meta.special_absolute_episode_numbers);
        let mut special_lane = LaneOutcome {
            named: special_numbers.len(),
            ..LaneOutcome::default()
        };
        for special_number in &special_numbers {
            let key = ("0".to_string(), special_number.to_string());
            if let Some(episode) = lookup.by_collection_episode.get(&key) {
                special_lane.resolved += 1;
                special_lane.ambiguous |= lookup.ambiguous_collection_episodes.contains(&key);
                if seen.insert(episode.id.clone()) {
                    resolved.push(episode.clone());
                }
            }
        }
        lanes.push(special_lane);
    }

    if resolved.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            distinct_numbers(&ep_meta.absolute_episode_numbers)
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            episode_numbers.clone()
        };
        let mut absolute_lane = LaneOutcome {
            named: absolute_numbers.len(),
            ..LaneOutcome::default()
        };

        for absolute_number in absolute_numbers {
            let key = absolute_number.to_string();
            if let Some(episode) = lookup.by_absolute_number.get(&key) {
                absolute_lane.resolved += 1;
                absolute_lane.ambiguous |= lookup.ambiguous_absolute_numbers.contains(&key);
                if seen.insert(episode.id.clone()) {
                    resolved.push(episode.clone());
                }
            }
        }
        lanes.push(absolute_lane);
    }

    ResolvedEpisodes {
        exact: lanes.iter().all(LaneOutcome::keeps_resolution_exact),
        episodes: resolved,
    }
}

#[derive(Default)]
struct EpisodeLookup {
    by_air_date: HashMap<String, Vec<Episode>>,
    by_collection_episode: HashMap<(String, String), Episode>,
    by_absolute_number: HashMap<String, Episode>,
    by_collection_index: HashMap<String, Vec<Episode>>,
    /// Keys of `by_collection_episode` that more than one catalog episode
    /// claims. The lookup still resolves them (first one wins) so matching
    /// keeps working, but a resolution through one is never exact.
    ambiguous_collection_episodes: HashSet<(String, String)>,
    /// Keys of `by_absolute_number` that more than one catalog episode claims.
    ambiguous_absolute_numbers: HashSet<String>,
}

impl EpisodeLookup {
    fn insert_collection_episode(&mut self, key: (String, String), episode: &Episode) {
        match self.by_collection_episode.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                if entry.get().id != episode.id {
                    self.ambiguous_collection_episodes
                        .insert(entry.key().clone());
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(episode.clone());
            }
        }
    }

    fn insert_absolute_number(&mut self, key: String, episode: &Episode) {
        match self.by_absolute_number.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                if entry.get().id != episode.id {
                    self.ambiguous_absolute_numbers.insert(entry.key().clone());
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(episode.clone());
            }
        }
    }
}

fn build_episode_lookup(collections: &[Collection], episodes: &[Episode]) -> EpisodeLookup {
    let collection_indexes = collections
        .iter()
        .map(|collection| (collection.id.clone(), collection.collection_index.clone()))
        .collect::<HashMap<_, _>>();

    // One absolute scale for the whole title; see `AbsoluteScale`.
    let scale = scryer_domain::AbsoluteScale::for_catalog(episodes);
    let mut lookup = EpisodeLookup::default();
    for episode in episodes {
        if let Some(air_date) = episode.air_date.as_ref() {
            lookup
                .by_air_date
                .entry(air_date.clone())
                .or_default()
                .push(episode.clone());
        }

        if let (Some(season_number), Some(episode_number)) = (
            episode.season_number.as_ref(),
            episode.episode_number.as_ref(),
        ) {
            if let Some(collection_id) = episode.collection_id.as_ref()
                && let Some(collection_index) = collection_indexes.get(collection_id)
            {
                lookup.insert_collection_episode(
                    (collection_index.clone(), episode_number.clone()),
                    episode,
                );
            } else {
                lookup.insert_collection_episode(
                    (season_number.clone(), episode_number.clone()),
                    episode,
                );
            }
        }

        if let Some(absolute_number) = scale.episode_absolute(episode) {
            lookup.insert_absolute_number(absolute_number.to_string(), episode);
        }

        if let Some(collection_id) = episode.collection_id.as_ref()
            && let Some(collection_index) = collection_indexes.get(collection_id)
        {
            lookup
                .by_collection_index
                .entry(collection_index.clone())
                .or_default()
                .push(episode.clone());
        }
    }

    for episodes in lookup.by_air_date.values_mut() {
        episodes.sort_by_key(|episode| {
            episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }
    for episodes in lookup.by_collection_index.values_mut() {
        episodes.sort_by_key(|episode| {
            episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }

    lookup
}

fn resolve_series_movie_from_name(
    input: &LibraryFilenameParseInput<'_>,
    raw_name: &str,
    filename_year: Option<i32>,
) -> Option<LibraryFilenameSeriesMovieTarget> {
    match match_series_movie_filename(input.series_movie_links, raw_name, filename_year) {
        SeriesMovieFilenameMatch::Unique(link) => {
            Some(build_series_movie_target(link, input.episodes))
        }
        SeriesMovieFilenameMatch::NoMatch | SeriesMovieFilenameMatch::Ambiguous => None,
    }
}

pub(crate) enum SeriesMovieFilenameMatch<'a> {
    NoMatch,
    Unique(&'a SeriesMovieLink),
    Ambiguous,
}

/// Match a full linked movie title without turning a title number into an episode.
/// Explicit episode coordinates and ambiguous movie names remain authoritative.
pub(crate) fn match_series_movie_filename<'a>(
    links: &'a [SeriesMovieLink],
    raw_name: &str,
    filename_year: Option<i32>,
) -> SeriesMovieFilenameMatch<'a> {
    let raw_key = library_name_match_key(raw_name);
    if raw_key.is_empty() || raw_name_has_explicit_episode_marker(raw_name) {
        return SeriesMovieFilenameMatch::NoMatch;
    }
    let mut matches = links.iter().filter(|link| {
        if let (Some(filename_year), Some(movie_year)) = (filename_year, link.movie.year)
            && filename_year != movie_year
        {
            return false;
        }
        series_movie_match_keys(link)
            .into_iter()
            .any(|key| normalized_key_contains_phrase(&raw_key, &key))
    });
    match (matches.next(), matches.next()) {
        (Some(link), None) => SeriesMovieFilenameMatch::Unique(link),
        (Some(_), Some(_)) => SeriesMovieFilenameMatch::Ambiguous,
        _ => SeriesMovieFilenameMatch::NoMatch,
    }
}

fn resolve_series_movie_from_episode_identity(
    input: &LibraryFilenameParseInput<'_>,
    ep_meta: &crate::ParsedEpisodeMetadata,
) -> Option<LibraryFilenameSeriesMovieTarget> {
    let season = ep_meta.season?;
    if season != 0 {
        return None;
    }
    let episode = ep_meta.episode_numbers.first().copied()?;
    let episode = episode.to_string();
    input
        .series_movie_links
        .iter()
        .find(|link| {
            link.linked_episode_id
                .as_deref()
                .and_then(|episode_id| input.episodes.iter().find(|ep| ep.id == episode_id))
                .is_some_and(|candidate| {
                    candidate.season_number.as_deref() == Some("0")
                        && candidate.episode_number.as_deref() == Some(episode.as_str())
                })
        })
        .map(|link| build_series_movie_target(link, input.episodes))
}

fn build_series_movie_target(
    link: &SeriesMovieLink,
    episodes: &[Episode],
) -> LibraryFilenameSeriesMovieTarget {
    let linked_episode = link
        .linked_episode_id
        .as_deref()
        .and_then(|episode_id| episodes.iter().find(|candidate| candidate.id == episode_id))
        .cloned();

    LibraryFilenameSeriesMovieTarget {
        series_movie_link_id: link.id.clone(),
        movie: link.movie.clone(),
        linked_episode,
    }
}

fn series_movie_match_keys(link: &SeriesMovieLink) -> Vec<String> {
    [
        Some(link.movie.title.as_str()),
        link.movie.sort_title.as_deref(),
        link.movie.slug.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(library_name_match_key)
    .filter(|key| !key.is_empty())
    .collect()
}

fn normalized_key_contains_phrase(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let haystack = format!(" {haystack} ");
    let needle = format!(" {needle} ");
    haystack.contains(&needle)
}

fn raw_name_has_explicit_episode_marker(raw_name: &str) -> bool {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"(?i)(^|[^a-z0-9])s\d{1,2}e\d{1,3}(e\d{1,3})?([^a-z0-9]|$)",
                r"(?i)(^|[^a-z0-9])\d{1,2}x\d{1,3}([^a-z0-9]|$)",
                r"(?i)(^|[^a-z0-9])s\d{1,2}[-_. ]+\d{1,3}([^a-z0-9]|$)",
                r"(?i)(^|[^a-z0-9])season[-_. ]*\d{1,2}[-_. ]*(episode|ep)[-_. ]*\d{1,3}([^a-z0-9]|$)",
            ]
            .into_iter()
            .map(|pattern| Regex::new(pattern).expect("valid explicit episode marker regex"))
            .collect()
        })
        .iter()
        .any(|pattern| pattern.is_match(raw_name))
}

fn library_name_match_key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_release_fallback(
    input: &LibraryFilenameParseInput<'_>,
    raw_name: &str,
    title_index: Option<&LibraryFilenameTitleIndex>,
) -> crate::ParsedReleaseMetadata {
    let mut parsed = parse_release_fallback_name(input, raw_name, title_index);
    if parsed_release_has_title_scan_episode_identity(&parsed, input.facet)
        || !input.mode.eq(&LibraryFilenameParseMode::TitleScan)
    {
        return parsed;
    }

    let Some(parent_name) = input
        .path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty())
    else {
        return parsed;
    };

    let parent_release = parse_release_fallback_name(input, &parent_name, title_index);
    let Some(parent_episode) = parent_release.episode.as_ref() else {
        return parsed;
    };
    if parent_episode.full_season
        || !parsed_release_has_title_scan_episode_identity(&parent_release, input.facet)
        || !immediate_parent_has_single_video_file(input.path)
    {
        return parsed;
    }

    fill_missing_release_metadata(&mut parsed, &parent_release, input.facet);
    parsed
}

fn parse_release_fallback_name(
    input: &LibraryFilenameParseInput<'_>,
    raw_name: &str,
    title_index: Option<&LibraryFilenameTitleIndex>,
) -> crate::ParsedReleaseMetadata {
    if let Some(title_index) = title_index {
        return match title_index.prepared(input).release_context.as_ref() {
            Some(context) => crate::parse_release_metadata_for_target(raw_name, context),
            None => crate::parse_release_metadata(raw_name),
        };
    }
    if let Some(context) = build_release_parse_context_for_library_filename(input) {
        crate::parse_release_metadata_for_target(raw_name, &context)
    } else {
        crate::parse_release_metadata(raw_name)
    }
}

fn build_release_parse_context_for_library_filename(
    input: &LibraryFilenameParseInput<'_>,
) -> Option<crate::ReleaseParseContext> {
    let title = input.title?;
    let facet_hint = input.facet.unwrap_or(&title.facet).as_str().to_string();
    let mut context = crate::build_release_parse_context_for_title(
        title,
        input.episodes,
        Some(facet_hint.as_str()),
    );

    for link in input.series_movie_links {
        let Some(linked_episode) = link.linked_episode_id.as_deref().and_then(|episode_id| {
            input
                .episodes
                .iter()
                .find(|episode| episode.id == episode_id)
        }) else {
            continue;
        };
        let season = linked_episode
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok());
        let episode = linked_episode
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok());
        let mut title_aliases = series_movie_aliases(link);
        title_aliases.sort();
        title_aliases.dedup();
        context
            .episodes
            .push(crate::release_parser::ContextEpisode {
                season,
                episode,
                absolute_number: None,
                air_date: None,
                title: Some(link.movie.title.clone()),
                title_aliases,
            });
    }

    Some(context)
}

fn series_movie_aliases(link: &SeriesMovieLink) -> Vec<String> {
    [
        Some(link.movie.title.clone()),
        link.movie.sort_title.clone(),
        link.movie.slug.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .collect()
}

fn parsed_release_has_title_scan_episode_identity(
    parsed: &crate::ParsedReleaseMetadata,
    facet: Option<&MediaFacet>,
) -> bool {
    matches!(
        parsed.episode.as_ref(),
        Some(ep)
            if !ep.episode_numbers.is_empty()
                || ep.air_date.is_some()
                || !ep.special_absolute_episode_numbers.is_empty()
                || (facet == Some(&MediaFacet::Anime)
                    && (ep.absolute_episode.is_some()
                        || !ep.absolute_episode_numbers.is_empty()))
    )
}

fn immediate_parent_has_single_video_file(source_path: &Path) -> bool {
    let Some(parent) = source_path.parent() else {
        return false;
    };

    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    let mut video_count = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            return false;
        };
        if !file_type.is_file() {
            continue;
        }

        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if VIDEO_EXTENSIONS
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(extension))
        {
            video_count += 1;
            if video_count > 1 {
                return false;
            }
        }
    }

    video_count == 1
}

fn fill_missing_release_metadata(
    target: &mut crate::ParsedReleaseMetadata,
    fallback: &crate::ParsedReleaseMetadata,
    facet: Option<&MediaFacet>,
) {
    if !parsed_release_has_title_scan_episode_identity(target, facet) && fallback.episode.is_some()
    {
        target.episode = fallback.episode.clone();
    }
    if target.imdb_id.is_none() {
        target.imdb_id = fallback.imdb_id.clone();
    }
    if target.tmdb_id.is_none() {
        target.tmdb_id = fallback.tmdb_id.clone();
    }
    if target.year.is_none() {
        target.year = fallback.year;
    }
    if target.quality.is_none() {
        target.quality = fallback.quality.clone();
    }
    if target.source.is_none() {
        target.source = fallback.source;
    }
    if target.video_codec.is_none() {
        target.video_codec = fallback.video_codec;
    }
    if target.video_encoding.is_none() {
        target.video_encoding = fallback.video_encoding.clone();
    }
    if target.audio.is_none() {
        target.audio = fallback.audio;
    }
    if target.audio_channels.is_none() {
        target.audio_channels = fallback.audio_channels.clone();
    }
    if target.release_group.is_none() {
        target.release_group = fallback.release_group.clone();
    }
    if target.streaming_service.is_none() {
        target.streaming_service = fallback.streaming_service;
    }
    if target.edition.is_none() {
        target.edition = fallback.edition.clone();
    }
    if target.normalized_title.trim().is_empty() && !fallback.normalized_title.trim().is_empty() {
        target.normalized_title = fallback.normalized_title.clone();
    }
    if target.normalized_title_variants.is_empty() && !fallback.normalized_title_variants.is_empty()
    {
        target.normalized_title_variants = fallback.normalized_title_variants.clone();
    }
}

fn synthesize_release_metadata(
    raw_name: &str,
    input: &LibraryFilenameParseInput<'_>,
    episode_identity: Option<crate::ParsedEpisodeMetadata>,
) -> crate::ParsedReleaseMetadata {
    let mut parsed = crate::ParsedReleaseMetadata::empty(raw_name, "library_filename_parser");
    parsed.raw_title = raw_name.to_string();
    if let Some(title) = input.title {
        parsed.normalized_title = title.name.clone();
        parsed.year = title.year;
        parsed.imdb_id = title
            .external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case("imdb"))
            .map(|external_id| external_id.value.clone());
        parsed.tmdb_id = title
            .external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case("tmdb"))
            .map(|external_id| external_id.value.clone());
        parsed.tvdb_id = title
            .external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case("tvdb"))
            .map(|external_id| external_id.value.clone());
    } else if let Some(walk) = library_title_walk(raw_name) {
        parsed.normalized_title = walk.title.unwrap_or_default();
        parsed.year = walk.year.and_then(|year| i32::try_from(year).ok());
        parsed.imdb_id = walk.imdb_id;
        parsed.tmdb_id = walk.tmdb_id;
        parsed.tvdb_id = walk.tvdb_id;
    }
    parsed.episode = episode_identity;
    parsed
}

/// The parse a stored episode stands for. It carries the episode's *raw*
/// absolute number, the one every renderer writes: an identity names its
/// catalog episode by season and episode, which lookups read before any
/// absolute, so the matching scale never needs to travel with it.
fn parsed_episode_metadata_from_episode(episode: &Episode) -> crate::ParsedEpisodeMetadata {
    let absolute = episode
        .absolute_number
        .as_deref()
        .and_then(|value| value.trim().parse::<u32>().ok());
    crate::ParsedEpisodeMetadata {
        season: episode
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        episode_numbers: episode
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .into_iter()
            .collect(),
        absolute_episode: absolute,
        absolute_episode_numbers: absolute.into_iter().collect(),
        air_date: episode
            .air_date
            .as_deref()
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()),
        release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
        raw: episode.episode_label.clone(),
        ..Default::default()
    }
}

fn extract_library_title_ids(raw: &str) -> (String, LibraryTitleWalk) {
    let mut walk = LibraryTitleWalk::default();

    for captures in library_id_token_regex().captures_iter(raw) {
        if walk.imdb_id.is_none()
            && let Some(value) = captures.name("imdb")
        {
            walk.imdb_id = crate::normalize::normalize_imdb_id(value.as_str());
        }
        if walk.tmdb_id.is_none()
            && let Some(value) = captures.name("tmdb")
        {
            walk.tmdb_id = crate::normalize::normalize_numeric_id(value.as_str());
        }
        if walk.tvdb_id.is_none()
            && let Some(value) = captures.name("tvdb")
        {
            walk.tvdb_id = crate::normalize::normalize_numeric_id(value.as_str());
        }
    }

    let without_ids = library_id_token_regex().replace_all(raw, " ").to_string();
    (without_ids, walk)
}

fn library_id_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (?:[\[\{\(]\s*)?
            (?:
                imdb(?:id)?\s*(?:://|:|-|=)\s*\(?(?P<imdb>tt[0-9]{5,})\)?
              | tmdb(?:id)?\s*(?:://|:|-|=)\s*\(?(?P<tmdb>[0-9]+)\)?
              | tvdb(?:id)?\s*(?:://|:|-|=)\s*\(?(?P<tvdb>[0-9]+)\)?
            )
            (?:\s*[\]\}\)])?
            ",
        )
        .expect("valid library id token regex")
    })
}

fn parse_simple_library_title_year(value: &str) -> Option<(String, u32)> {
    let captures = simple_library_title_year_regex().captures(value)?;
    let title = clean_library_title_candidate(captures.name("title")?.as_str())?;
    let year = captures.name("year")?.as_str().parse::<u32>().ok()?;
    (1888..=2100).contains(&year).then_some((title, year))
}

fn simple_library_title_year_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            ^\s*
            (?P<title>.+?)
            \s*[\(\[]\s*
            (?P<year>[0-9]{4})
            \s*[\)\]]
            (?:\s+.*)?
            \s*$
            ",
        )
        .expect("valid simple library title regex")
    })
}

fn fallback_title_from_id_text(value: &str) -> Option<String> {
    let title = clean_library_title_candidate(value)?;
    let normalized = title.to_ascii_uppercase();
    if matches!(
        normalized.as_str(),
        "MOVIE" | "VIDEO" | "FILE" | "DOWNLOAD" | "UNKNOWN"
    ) {
        return None;
    }
    if !title.chars().any(|ch| ch.is_alphabetic()) {
        return None;
    }
    Some(title)
}

fn clean_library_title_candidate(value: &str) -> Option<String> {
    let normalized = normalize_library_title_text(value);
    let trimmed = normalized
        .trim()
        .trim_matches(|ch: char| matches!(ch, '-' | '.' | '_' | '[' | ']' | '(' | ')' | '{' | '}'))
        .trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn normalize_library_title_text(value: &str) -> String {
    let separated = value
        .chars()
        .map(|ch| if matches!(ch, '.' | '_') { ' ' } else { ch })
        .collect::<String>();
    normalize_folder_name(separated.as_str())
}

fn strip_trailing_plain_year_token(folder: &str, year: u32) -> String {
    let suffix = year.to_string();
    if let Some(prefix) = folder.strip_suffix(&suffix) {
        let trimmed = prefix.trim_end();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    folder.to_string()
}

fn push_unique_query(
    queries: &mut Vec<String>,
    seen_normalized: &mut HashSet<String>,
    query: String,
) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }

    let normalized = crate::app_usecase_rss::normalize_for_matching(trimmed);
    if normalized.is_empty() || !seen_normalized.insert(normalized) {
        return;
    }

    queries.push(trimmed.to_string());
}

fn push_unique_literal_query(queries: &mut Vec<String>, query: String) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }

    let normalized = trimmed
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.trim().is_empty() {
        return;
    }

    if queries.iter().any(|existing| {
        existing
            .trim()
            .nfkc()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            == normalized
    }) {
        return;
    }

    queries.push(trimmed.to_string());
}

fn part_reduced_query(query: &str) -> Option<String> {
    let tokens = query.split_whitespace().collect::<Vec<_>>();
    if !tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case("part"))
    {
        return None;
    }
    let reduced = tokens
        .into_iter()
        .filter(|token| !token.eq_ignore_ascii_case("part"))
        .collect::<Vec<_>>();
    (reduced.len() >= 2).then(|| reduced.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{EpisodeType, ExternalId};

    #[test]
    fn filename_parse_raw_name_strips_generated_restore_suffix() {
        assert_eq!(
            filename_parse_raw_name(Path::new("/library/Movie.2024-restored.mkv"), None),
            "Movie.2024"
        );
        assert_eq!(
            filename_parse_raw_name(Path::new("/library/Movie.2024-restored-2.mkv"), None),
            "Movie.2024"
        );
        assert_eq!(
            filename_parse_raw_name(Path::new("/library/Movie.2024-restored-cut.mkv"), None),
            "Movie.2024-restored-cut"
        );
    }

    fn title(name: &str, facet: MediaFacet) -> Title {
        Title {
            id: "title-1".into(),
            name: name.into(),
            facet: facet.clone(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![ExternalId::new("tvdb", "12345")],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
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
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            movie_release_dates: None,
            folder_path: None,
        }
    }

    fn episode(id: &str, season: &str, number: &str) -> Episode {
        Episode {
            id: id.into(),
            title_id: "title-1".into(),
            collection_id: None,
            episode_type: EpisodeType::Standard,
            episode_number: Some(number.into()),
            season_number: Some(season.into()),
            episode_label: Some(format!("S{season:0>2}E{number:0>2}")),
            title: Some(format!("Episode {number}")),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            contiguous_absolute_number: None,
            overview: None,
            tvdb_id: None,
            tmdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn series_movie_link(name: &str, linked_episode_id: Option<&str>) -> SeriesMovieLink {
        let now = Utc::now();
        SeriesMovieLink {
            id: format!(
                "series-movie-{}",
                name.to_ascii_lowercase().replace(' ', "-")
            ),
            series_title_id: "title-1".into(),
            movie: MovieEntity {
                id: format!("movie-{}", name.to_ascii_lowercase().replace(' ', "-")),
                title: name.into(),
                sort_title: Some(name.into()),
                slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
                year: Some(2024),
                overview: None,
                poster_url: None,
                background_url: None,
                language: Some("eng".into()),
                runtime_minutes: Some(90),
                content_status: Some("released".into()),
                studio: None,
                digital_release_date: None,
                imdb_id: None,
                tvdb_id: Some("movie-1".into()),
                tmdb_id: None,
                mal_id: None,
                anidb_id: None,
                ratings: None,
                credits: None,
                created_at: now,
                updated_at: now,
            },
            placement: None,
            narrative_order: None,
            after_season: None,
            before_season: None,
            linked_episode_id: linked_episode_id.map(str::to_string),
            association_confidence: None,
            continuity_status: None,
            movie_form: Some("movie".into()),
            confidence: None,
            signal_summary: None,
            source: Some("test".into()),
            monitoring_override: None,
            metadata_active: true,
            monitored: true,
            legacy_collection_id: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn title_walk_extracts_simple_title_year_and_ids() {
        let walk = library_title_walk("Some Show (2024) [tvdbid=12345]").expect("title walk");

        assert_eq!(walk.title.as_deref(), Some("Some Show"));
        assert_eq!(walk.year, Some(2024));
        assert_eq!(walk.tvdb_id.as_deref(), Some("12345"));
    }

    #[test]
    fn title_only_release_style_uses_fallback() {
        let parse = parse_library_filename(&LibraryFilenameParseInput::title_only(
            Path::new("/library/Example.Movie.2024.MAX.WEB-DL.2160p-GRP.mkv"),
            Some(Path::new("/library")),
        ));

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(
            parse.query_evidence.queries.first().map(String::as_str),
            Some("EXAMPLE MOVIE")
        );
    }

    #[test]
    fn title_only_plain_dotted_episode_uses_release_parser_evidence() {
        let parse = parse_library_filename(&LibraryFilenameParseInput::title_only(
            Path::new("/library/Example.Show.S01E01.mkv"),
            Some(Path::new("/library")),
        ));

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(
            parse.query_evidence.queries.first().map(String::as_str),
            Some("EXAMPLE SHOW")
        );
    }

    /// Community (per-cour) anime numbering on disk. TVDB carries one official
    /// season; the community carries four cours of 14 / 12 / 10 / 24, so
    /// community S04E20 is official S01E56. Names are invented.
    fn anime_numbering_bridge_fixture() -> scryer_domain::AnimeNumberingBridge {
        anime_numbering_bridge_fixture_with_titles(&[
            "Lantern Verge Cour 1",
            "Lantern Verge Cour 2",
            "Lantern Verge Cour 3",
            "Lantern Verge Cour 4",
        ])
    }

    fn anime_numbering_bridge_fixture_with_titles(
        titles: &[&str; 4],
    ) -> scryer_domain::AnimeNumberingBridge {
        let mut seasons = Vec::new();
        let mut tvdb_start = 1;
        for (offset, length) in [14, 12, 10, 24].iter().enumerate() {
            let index = i32::try_from(offset).expect("small index") + 1;
            seasons.push(scryer_domain::AnimeCommunitySeason {
                index,
                anidb_id: None,
                anilist_id: None,
                mal_id: None,
                titles: vec![titles[offset].to_string()],
                ranges: vec![scryer_domain::AnimeCommunitySeasonRange {
                    community_episode_start: 1,
                    community_episode_end: Some(*length),
                    tvdb_season: 1,
                    tvdb_episode_start: tvdb_start,
                    tvdb_episode_end: Some(tvdb_start + length - 1),
                }],
                absolute_start: Some(tvdb_start),
                contiguous_absolute_start: None,
                episode_count: Some(*length),
            });
            tvdb_start += length;
        }
        scryer_domain::AnimeNumberingBridge {
            source: Default::default(),
            generated_on: "2026-08-30".to_string(),
            corroborating_order: None,
            seasons,
        }
    }

    /// Per-season episode counts of a long-running multi-cour anime, shaped
    /// like Pokémon's TVDB record (S00 specials plus S01-S20).
    const LONG_RUNNING_ANIME_SEASON_LENGTHS: [u32; 21] = [
        20, 82, 52, 28, 52, 65, 40, 52, 52, 47, 52, 52, 52, 34, 84, 58, 93, 47, 146, 136, 150,
    ];

    /// Episodes for every season of that record, keyed `ep-<season>-<number>`,
    /// in the same order a catalog would hold them.
    fn long_running_anime_episodes() -> Vec<Episode> {
        let mut episodes = Vec::new();
        for (season, length) in LONG_RUNNING_ANIME_SEASON_LENGTHS.iter().enumerate() {
            for number in 1..=*length {
                episodes.push(episode(
                    &format!("ep-{season}-{number}"),
                    &season.to_string(),
                    &number.to_string(),
                ));
            }
        }
        episodes
    }

    /// Sonarr writes anime episodes with the absolute number in its own
    /// hyphen-delimited slot. Every one of those files names exactly one
    /// episode, and the scan must place it there — reading the absolute number
    /// as the far end of an episode range turns a single file into a
    /// season-sized pack that resolves to nothing and is dropped without a
    /// `media_files` row (issue: 1,880 anime files lost on a 111k-file scan).
    #[test]
    fn title_scan_places_sonarr_absolute_numbered_anime_files_on_their_own_episode() {
        let title = title("Pokémon", MediaFacet::Anime);
        let episodes = long_running_anime_episodes();

        // (season, episode, absolute) triples taken from the real library.
        // Season 1 survived the defect because there the absolute number equals
        // the episode number, so the bogus range was one episode wide.
        for (season, number, absolute) in [
            (1_u32, 1_u32, 1_u32),
            (2, 1, 83),
            (2, 36, 118),
            (3, 1, 135),
            (10, 1, 473),
            (20, 150, 1373),
        ] {
            let path = format!(
                "/library/Pokémon (1997) {{tvdb-76703}}/Season {season:02}/Pokémon (1997) - S{season:02}E{number:02} - {absolute:03} - Episode Name [WEBDL-1080p].mkv"
            );
            let path = Path::new(&path);
            let input = LibraryFilenameParseInput {
                path,
                display_name: None,
                library_root: Some(Path::new("/library")),
                title: Some(&title),
                facet: Some(&title.facet),
                collections: &[],
                series_movie_links: &[],
                episodes: &episodes,
                existing_record: None,
                anime_numbering_bridge: None,
                mode: LibraryFilenameParseMode::TitleScan,
                fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
            };

            let parse = parse_library_filename(&input);

            assert_eq!(
                parse.unmatched_reason(),
                None,
                "S{season:02}E{number:02} - {absolute:03} was refused"
            );
            assert_eq!(
                parse
                    .target_episodes()
                    .iter()
                    .map(|episode| episode.id.as_str())
                    .collect::<Vec<_>>(),
                vec![format!("ep-{season}-{number}").as_str()],
                "S{season:02}E{number:02} - {absolute:03} resolved to the wrong episodes"
            );
        }
    }

    #[test]
    fn title_scan_maps_a_community_numbered_anime_file_onto_the_official_episode() {
        let title = title("Lantern Verge", MediaFacet::Anime);
        let episodes: Vec<Episode> = (1..=60)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string()))
            .collect();
        let bridge = anime_numbering_bridge_fixture();
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Lantern Verge/Season 01/Lantern Verge - S04E20.mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: Some(&bridge),
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-56"]
        );
    }

    /// Re:ZERO-shaped bridge: TVDB keeps every cour in season 1, and the third
    /// community season starts at story episode 51 on the contiguous scale.
    ///
    /// SMG reads a community season's contiguous start off its anchor
    /// episode's own contiguous number, so a season whose anchor SMG has not
    /// placed yet (the anchor lies past `contiguous_through`) carries no
    /// contiguous start at all. `newest_cour_open` leaves the third season's
    /// range without an end, as for a cour that is still airing.
    fn re_zero_bridge(
        contiguous_through: u32,
        newest_cour_open: bool,
    ) -> scryer_domain::AnimeNumberingBridge {
        let season = |index: i32, start: i32, length: i32, raw_start: i32, open: bool| {
            scryer_domain::AnimeCommunitySeason {
                index,
                anidb_id: None,
                anilist_id: None,
                mal_id: None,
                titles: vec![format!("Re:ZERO Season {index}")],
                ranges: vec![scryer_domain::AnimeCommunitySeasonRange {
                    community_episode_start: 1,
                    community_episode_end: (!open).then_some(length),
                    tvdb_season: 1,
                    tvdb_episode_start: start,
                    tvdb_episode_end: (!open).then_some(start + length - 1),
                }],
                absolute_start: Some(raw_start),
                contiguous_absolute_start: u32::try_from(start)
                    .is_ok_and(|anchor| anchor <= contiguous_through)
                    .then_some(start),
                episode_count: (!open).then_some(length),
            }
        };
        scryer_domain::AnimeNumberingBridge {
            source: Default::default(),
            generated_on: "2026-09-25".to_string(),
            corroborating_order: None,
            seasons: vec![
                season(1, 1, 25, 1, false),
                season(2, 26, 25, 27, false),
                season(3, 51, 16, 53, newest_cour_open),
            ],
        }
    }

    /// Official S01E01-E66 where two specials sit in TVDB's absolute order (at
    /// raw 13 and raw 40), so from story episode 38 on the raw absolute runs
    /// two ahead of the contiguous one. `contiguous_through` marks the last
    /// episode SMG has placed on the contiguous scale.
    fn re_zero_episodes(contiguous_through: u32) -> Vec<Episode> {
        let mut episodes: Vec<Episode> = (1..=66_u32)
            .map(|number| {
                let raw = match number {
                    1..=12 => number,
                    13..=38 => number + 1,
                    _ => number + 2,
                };
                let mut episode = episode(&format!("ep-{number}"), "1", &number.to_string());
                episode.absolute_number = Some(raw.to_string());
                episode.contiguous_absolute_number = (number <= contiguous_through)
                    .then(|| i32::try_from(number).expect("small episode number"));
                episode
            })
            .collect();
        for (number, raw) in [(1_u32, 13_u32), (2, 40)] {
            let mut special = episode(&format!("sp-{number}"), "0", &number.to_string());
            special.absolute_number = Some(raw.to_string());
            episodes.push(special);
        }
        episodes
    }

    fn scan_re_zero(
        episodes: &[Episode],
        bridge: &scryer_domain::AnimeNumberingBridge,
        file_name: &str,
    ) -> LibraryFilenameParse {
        let title = title("Re:ZERO", MediaFacet::Anime);
        let path = format!("/library/Re ZERO/Season 01/{file_name}");
        let input = LibraryFilenameParseInput {
            path: Path::new(&path),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes,
            existing_record: None,
            anime_numbering_bridge: Some(bridge),
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };
        parse_library_filename(&input)
    }

    fn target_ids(parse: &LibraryFilenameParse) -> Vec<String> {
        parse
            .target_episodes()
            .iter()
            .map(|episode| episode.id.clone())
            .collect()
    }

    #[test]
    fn title_scan_files_an_absolute_numbered_file_on_the_contiguous_scale() {
        // Raw absolute 51 is story episode 49; the contiguous 51 is S01E51.
        let episodes = re_zero_episodes(66);
        let bridge = re_zero_bridge(66, false);

        assert_eq!(
            target_ids(&scan_re_zero(
                &episodes,
                &bridge,
                "[Group] Re:ZERO - 51.mkv"
            )),
            vec!["ep-51".to_string()]
        );
    }

    #[test]
    fn title_scan_leaves_an_absolute_unmatched_when_its_cour_anchor_is_unplaced() {
        // SMG has not placed the newest cour on the contiguous scale yet. Its
        // anchor is exactly the unplaced episode, so SMG serves no contiguous
        // start for that cour and nothing can place `- 51`. The file stays
        // unmatched rather than falling back to raw 51, which is S01E49.
        let episodes = re_zero_episodes(50);
        let bridge = re_zero_bridge(50, false);
        assert_eq!(bridge.seasons[2].contiguous_absolute_start, None);

        let parse = scan_re_zero(&episodes, &bridge, "[Group] Re:ZERO - 51.mkv");

        assert!(target_ids(&parse).is_empty());
        assert!(
            matches!(
                parse.target,
                LibraryFilenameTarget::Unmatched {
                    reason: "episode_lookup_failed"
                }
            ),
            "unexpected target: {:?}",
            parse.target
        );
    }

    #[test]
    fn title_scan_files_an_unplaced_absolute_inside_an_open_cour_with_a_placed_anchor() {
        // SMG placed the airing cour's anchor (S01E51) and its first episodes
        // but not the newest ones; the cour's range is still open. The
        // contiguous start carries `- 60` onto S01E60.
        let episodes = re_zero_episodes(58);
        let bridge = re_zero_bridge(58, true);
        assert_eq!(bridge.seasons[2].contiguous_absolute_start, Some(51));

        assert_eq!(
            target_ids(&scan_re_zero(
                &episodes,
                &bridge,
                "[Group] Re:ZERO - 60.mkv"
            )),
            vec!["ep-60".to_string()]
        );
    }

    #[test]
    fn resolved_community_numbering_renders_the_catalog_raw_absolute_on_import_and_rename() {
        // `Season 3 - 01` is S01E51, whose raw absolute is 53 while the title
        // matches on the contiguous 51. Matching reads the contiguous scale;
        // every renderer writes the raw number, so the import's parse path
        // and a later rename produce the same file name.
        let episodes = re_zero_episodes(66);
        let bridge = re_zero_bridge(66, false);
        let parse = scan_re_zero(&episodes, &bridge, "[Group] Re:ZERO Season 3 - 01.mkv");

        assert_eq!(target_ids(&parse), vec!["ep-51".to_string()]);
        let resolved = parse.target_episodes()[0].clone();
        assert_eq!(resolved.absolute_number.as_deref(), Some("53"));
        let parsed_episode = parse
            .parsed_release
            .episode
            .as_ref()
            .expect("the scan keeps the resolved episode parse");
        assert_eq!(parsed_episode.season, Some(1));
        assert_eq!(parsed_episode.episode_numbers, vec![51]);
        assert_eq!(
            parsed_episode.absolute_episode,
            Some(53),
            "the resolved parse carries the raw rendering absolute"
        );

        let title = title("Re:ZERO", MediaFacet::Anime);
        let template = "{title} - S{season_order:2}E{episode:2} ({absolute_episode:3}) - {episode_title}.{ext}";
        let absolute = crate::import_workflow::import_absolute_episode_token(
            Some(&resolved),
            parsed_episode.absolute_episode,
        );
        assert_eq!(absolute.as_deref(), Some("53"));
        let imported = crate::import_workflow::episode_import_dest_path(
            &title,
            true,
            &parse.parsed_release,
            None,
            "mkv",
            Path::new("/downloads/[Group] Re:ZERO Season 3 - 01.mkv"),
            Path::new("/library/Re ZERO"),
            true,
            template,
            "Season {season:2}",
            "Specials",
            1,
            "51",
            absolute.as_deref(),
            resolved.title.as_deref(),
            None,
        );
        let imported_name = imported
            .file_name()
            .and_then(|name| name.to_str())
            .expect("import renders a file name")
            .to_string();
        assert!(
            imported_name.contains("(053)"),
            "import rendered {imported_name}"
        );

        let media_file = crate::TitleMediaFile {
            id: "file-51".to_string(),
            title_id: title.id.clone(),
            episode_id: Some(resolved.id.clone()),
            file_path: imported.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut planning = crate::library_rename::RenamePlanningState::default();
        let items = crate::library_rename::build_series_rename_plan_items_from_media_files(
            &title,
            true,
            Vec::new(),
            episodes.clone(),
            vec![media_file],
            "/library",
            "{title}",
            "Season {season:2}",
            "Specials",
            template,
            &crate::library_rename::RenameMissingMetadataPolicy::default(),
            &mut planning,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].normalized_filename.as_deref(),
            Some(imported_name.as_str()),
            "rename must render the imported file's own name"
        );
    }

    #[test]
    fn title_scan_fuzzily_anchors_a_cour_title_without_inventing_a_season() {
        let title = title(
            "Rantan Kyoukai Monogatari Honzuki no Gekokujou",
            MediaFacet::Anime,
        );
        let episodes: Vec<Episode> = (1..=60)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string()))
            .collect();
        let bridge = anime_numbering_bridge_fixture_with_titles(&[
            "Rantan Kyoukai Monogatari Hajimari no Akatsuki",
            "Rantan Kyoukai Monogatari Madoromi no Kaze",
            "Rantan Kyoukai Monogatari Shirogane no Hane",
            "Rantan Kyokai Monogatari Saigo no Gassho o Utau Toki no Hikari to Kage no Uta",
        ]);
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Rantan Kyoukai Monogatari Honzuki no Gekokujou/Season 01/Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta - 05.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: Some(&bridge),
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parsed_filename = crate::parse_release_metadata(
            "Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta - 05",
        );
        assert_eq!(
            parsed_filename
                .episode
                .as_ref()
                .and_then(|episode| episode.season),
            None,
            "the parser must preserve the filename's missing season"
        );
        let parse = parse_library_filename(&input);

        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-41"]
        );

        let explicitly_seasoned_input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Rantan Kyoukai Monogatari Honzuki no Gekokujou/Season 01/Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta - S01E05.mkv",
            ),
            ..input
        };
        let explicit_parse = parse_library_filename(&explicitly_seasoned_input);
        assert_eq!(
            explicit_parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-5"],
            "an explicit official season must not be title-anchored to a cour"
        );
    }

    #[test]
    fn title_scan_leaves_a_community_numbered_file_alone_without_a_bridge() {
        let title = title("Lantern Verge", MediaFacet::Anime);
        let episodes: Vec<Episode> = (1..=60)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string()))
            .collect();
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Lantern Verge/Season 01/Lantern Verge - S04E20.mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        // Season 4 does not exist in the catalog, so the file matches nothing —
        // exactly the behaviour this feature is fixing, preserved without a
        // bridge.
        assert!(parse.target_episodes().is_empty());
    }

    #[test]
    fn title_scan_release_parser_resolves_standard_episode() {
        let title = title("Example Show", MediaFacet::Series);
        let episodes = vec![episode("ep-2-3", "2", "3")];
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Example Show/Season 02/Example Show - S02E03.mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(
            parse
                .episode_identity
                .as_ref()
                .and_then(|episode| episode.season),
            Some(2)
        );
        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-2-3"]
        );
    }

    #[test]
    fn title_scan_release_parser_preempts_name_only_series_movie_fallback() {
        let title = title("Example Animated Saga", MediaFacet::Anime);
        let episodes = vec![episode("ep-1-1", "1", "1"), episode("ep-0-1", "0", "1")];
        let series_movie_links = vec![series_movie_link(
            "Synthetic Bridge Feature",
            Some("ep-0-1"),
        )];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Example Animated Saga/Season 01/Example Animated Saga Synthetic Bridge Feature - S01E01.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &series_movie_links,
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert_eq!(
            parse.episode_identity.as_ref().and_then(|ep| ep.season),
            Some(1)
        );
        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-1-1"]
        );
        assert_eq!(parse.target_series_movie_link_id(), None);
    }

    #[test]
    fn title_scan_prefers_unlinked_series_movie_title_over_weak_episode_identity() {
        let title = title("Cipher-Pass", MediaFacet::Anime);
        let episodes = vec![episode("ep-1-3", "1", "3")];
        let series_movie_links = vec![series_movie_link(
            "Cipher-Pass: Keepers of the Signal - Case.3 In the Harbor Beyond Is ____",
            None,
        )];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Cipher-Pass (2024)/Cipher-Pass.Keepers.of.the.Signal.Case.3.In.the.Harbor.Beyond.Is.2024.720p.WEB-DL.AV1.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &series_movie_links,
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert_eq!(parse.target_episodes(), Vec::<Episode>::new());
        assert_eq!(parse.episode_identity, None);
        assert_eq!(parse.parsed_release.episode, None);
        assert_eq!(
            parse.target_series_movie_link_id(),
            Some(series_movie_links[0].id.as_str())
        );
    }

    #[test]
    fn title_scan_release_parser_resolves_x_episode_filename() {
        let title = title("Example Show", MediaFacet::Series);
        let episodes = vec![episode("ep-1-12", "1", "12")];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Example Show/Season 01/Example Show - 01x12 - Finale WEBDL-1080p.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-1-12"]
        );
    }

    #[test]
    fn name_only_series_movie_resolves_as_final_fallback() {
        let title = title("Example Animated Saga", MediaFacet::Anime);
        let episodes = vec![episode("ep-0-1", "0", "1")];
        let series_movie_links = vec![series_movie_link("Example Bonus Feature", Some("ep-0-1"))];
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Example Animated Saga/Example Bonus Feature.mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: None,
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &series_movie_links,
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-0-1"]
        );
        assert_eq!(
            parse.target_series_movie_link_id(),
            Some(series_movie_links[0].id.as_str())
        );
    }

    #[test]
    fn name_only_series_movie_rejects_placement_only_match() {
        let title = title("Example Animated Saga", MediaFacet::Anime);
        let episodes = vec![episode("ep-0-1", "0", "1")];
        let mut link = series_movie_link("Example Bonus Feature", Some("ep-0-1"));
        link.placement = Some("Special".into());
        let series_movie_links = vec![link];
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Example Animated Saga/Special.mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: None,
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &series_movie_links,
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(parse.target_series_movie_link_id(), None);
    }

    #[test]
    fn name_only_series_movie_rejects_mismatched_movie_year() {
        let title = title("Example Animated Saga", MediaFacet::Anime);
        let episodes = vec![episode("ep-0-1", "0", "1")];
        let series_movie_links = vec![series_movie_link("Example Bonus Feature", Some("ep-0-1"))];
        let input = LibraryFilenameParseInput {
            path: Path::new("/library/Example Animated Saga/Example Bonus Feature (2023).mkv"),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: None,
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &series_movie_links,
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(parse.target_series_movie_link_id(), None);
    }

    #[test]
    fn title_scan_release_parser_resolves_episode_without_provenance_tokens() {
        let title = title("Example Show", MediaFacet::Series);
        let episodes = vec![episode("ep-2-3", "2", "3")];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Example Show/Season 02/Example Show - S02E03 - The Episode.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(parse.parsed_release.quality, None);
        assert_eq!(parse.parsed_release.source, None);
        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-2-3"]
        );
    }

    #[test]
    fn title_scan_release_parser_preserves_quality_source_provenance() {
        let title = title("Example Show", MediaFacet::Series);
        let episodes = vec![episode("ep-2-3", "2", "3")];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Example Show/Season 02/Example Show - S02E03 - The Episode 1080p WEB-DL-GROUP.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(
            parse.parsed_release.source,
            Some(crate::ReleaseSource::WebDl)
        );
        assert_eq!(parse.parsed_release.release_group.as_deref(), Some("GROUP"));
        assert_eq!(parse.target_episodes()[0].id, "ep-2-3");
    }

    #[test]
    fn title_scan_release_parser_preserves_remux_provenance() {
        let title = title("Example Show", MediaFacet::Series);
        let episodes = vec![episode("ep-2-3", "2", "3")];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/Example Show/Season 02/Example Show - S02E03 - The Episode 2160p BluRay Remux-GRP.mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert!(parse.parsed_release.is_remux);
        assert_eq!(
            crate::release_parser::parsed_release_source_type(&parse.parsed_release).as_deref(),
            Some("Remux")
        );
        assert_eq!(parse.parsed_release.release_group.as_deref(), Some("GRP"));
    }

    #[test]
    fn numeric_title_stays_target_aware_with_provenance_fallback() {
        let title = title("13", MediaFacet::Series);
        let episodes = vec![episode("ep-2-1", "2", "1")];
        let input = LibraryFilenameParseInput {
            path: Path::new(
                "/library/13 (2024)/Season 02/13 (2024) - S02E01 - Day 2 800 A.M. 900 A.M. [WEBDL-1080p].mkv",
            ),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        };

        let parse = parse_library_filename(&input);

        assert_eq!(
            parse.strategy,
            LibraryFilenameParseStrategy::ReleaseParserFallback
        );
        assert!(parse.release_fallback_used);
        assert_eq!(parse.parsed_release.normalized_title, "13");
        assert_eq!(parse.parsed_release.quality.as_deref(), Some("1080p"));
        assert_eq!(
            parse.parsed_release.source,
            Some(crate::ReleaseSource::WebDl)
        );
        assert_eq!(parse.target_episodes()[0].id, "ep-2-1");
    }

    /// Seasons of an anime catalog, as `(episode count, first absolute
    /// number)` pairs, with TVDB's own episode titles.
    fn anime_catalog(
        seasons: &[(u32, u32)],
        episode_title: impl Fn(u32, u32, u32) -> String,
    ) -> Vec<Episode> {
        let mut episodes = Vec::new();
        for (index, (length, absolute_start)) in seasons.iter().enumerate() {
            let season = u32::try_from(index).expect("small index") + 1;
            for number in 1..=*length {
                let absolute = absolute_start + number - 1;
                let mut entry = episode(
                    &format!("ep-{season}-{number}"),
                    &season.to_string(),
                    &number.to_string(),
                );
                entry.absolute_number = Some(absolute.to_string());
                entry.title = Some(episode_title(season, number, absolute));
                episodes.push(entry);
            }
        }
        episodes
    }

    /// Sonarr writes `Show (Year) - SxxEyy - NNN - Episode Title`, and the
    /// parenthesized premiere year is a name qualifier, never a coordinate.
    /// Reading it as an anime absolute episode number threw away both real
    /// coordinates — the `SxxEyy` token and the absolute-number slot — so the
    /// file resolved to nothing, took the unmatched branch and never got a
    /// `media_files` row (issue: 474 anime files lost on a fresh 38,077-file
    /// scan, 471 of them one long-running series).
    ///
    /// The bogus reading only outscored the real one when the trailing episode
    /// title also matched the catalog, which is why it hit the files whose
    /// TVDB title is literally `Episode <absolute>`.
    #[test]
    fn title_scan_never_reads_a_parenthesized_series_year_as_an_absolute_episode() {
        // Per-season `(episode count, first absolute number)` taken from the
        // TVDB records of the four affected series.
        let shin_chan_seasons: Vec<(u32, u32)> = {
            let mut seasons = Vec::new();
            let mut absolute = 1;
            for length in [
                52_u32, 52, 49, 18, 42, 45, 42, 43, 39, 40, 35, 31, 28, 36, 35, 29, 37, 35, 33, 34,
                35, 28, 29, 34, 36, 34, 35, 39, 52, 50, 51, 51, 51, 52, 34,
            ] {
                seasons.push((length, absolute));
                absolute += length;
            }
            seasons
        };

        struct SonarrAnimeFile<'a> {
            title_name: &'a str,
            seasons: &'a [(u32, u32)],
            title_dir: &'a str,
            display_name: &'a str,
            expected_episode: &'a str,
        }

        let cases = [
            SonarrAnimeFile {
                title_name: "Shin Chan",
                seasons: shin_chan_seasons.as_slice(),
                title_dir: "Shin Chan (1992) {tvdb-79654}",
                display_name: "Shin Chan (1992) - S06E28 - 241 - Episode 241 [WEBDL-1080p]",
                expected_episode: "ep-6-28",
            },
            SonarrAnimeFile {
                title_name: "Shin Chan",
                seasons: shin_chan_seasons.as_slice(),
                title_dir: "Shin Chan (1992) {tvdb-79654}",
                display_name: "Shin Chan (1992) - S07E42 - 300 - Episode 300 [WEBDL-1080p]",
                expected_episode: "ep-7-42",
            },
            SonarrAnimeFile {
                title_name: "Shin Chan",
                seasons: shin_chan_seasons.as_slice(),
                title_dir: "Shin Chan (1992) {tvdb-79654}",
                display_name: "Shin Chan (1992) - S08E01 - 301 - Episode 301 [WEBDL-1080p]",
                expected_episode: "ep-8-1",
            },
            SonarrAnimeFile {
                title_name: "Digimon: Digital Monsters",
                seasons: &[(54, 2), (50, 57)],
                title_dir: "Digimon - Digital Monsters (1999) {tvdb-72241}",
                display_name: "Digimon - Digital Monsters (1999) - S01E11 - 012 - The Dancing Digimon [WEBDL-1080p]",
                expected_episode: "ep-1-11",
            },
            SonarrAnimeFile {
                title_name: "Fist of the North Star",
                seasons: &[(22, 1), (35, 23), (25, 58), (27, 83), (13, 110), (30, 123)],
                title_dir: "Fist of the North Star (1984) {tvdb-79156}",
                display_name: "Fist of the North Star (1984) - S06E28 - 150 - The Final Chapter - The Last Three Episodes! Here is the 2,000 Year-old History [WEBDL-1080p]",
                expected_episode: "ep-6-28",
            },
            SonarrAnimeFile {
                title_name: "Doraemon (2005)",
                seasons: &[(32, 1), (42, 33), (36, 75), (44, 111)],
                title_dir: "Doraemon (2005) (2005) {tvdb-281405}",
                display_name: "Doraemon (2005) (2005) - S04E11 - 121 - Special Effects Ultra Dora-Man + Doraemon - Nobita's New Great Adventure into th [WEBDL-1080p]",
                expected_episode: "ep-4-11",
            },
        ];

        for case in cases {
            let SonarrAnimeFile {
                title_name,
                seasons,
                title_dir,
                display_name,
                expected_episode,
            } = case;
            let title = title(title_name, MediaFacet::Anime);
            let episodes = anime_catalog(seasons, |_, _, absolute| format!("Episode {absolute}"));
            let path = format!("/library/{title_dir}/Season 01/{display_name}.mkv");
            let path = Path::new(&path);
            let input = LibraryFilenameParseInput {
                path,
                display_name: None,
                library_root: Some(Path::new("/library")),
                title: Some(&title),
                facet: Some(&title.facet),
                collections: &[],
                series_movie_links: &[],
                episodes: &episodes,
                existing_record: None,
                anime_numbering_bridge: None,
                mode: LibraryFilenameParseMode::TitleScan,
                fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
            };

            let parse = parse_library_filename(&input);

            assert_eq!(parse.unmatched_reason(), None, "{display_name} was refused");
            assert_eq!(
                parse
                    .target_episodes()
                    .iter()
                    .map(|episode| episode.id.as_str())
                    .collect::<Vec<_>>(),
                vec![expected_episode],
                "{display_name} resolved to the wrong episodes"
            );
        }
    }

    /// Three TVDB seasons of twelve, each carrying its absolute number.
    fn seasonal_catalog_with_absolutes() -> Vec<Episode> {
        let mut episodes = Vec::new();
        for absolute in 1..=36_u32 {
            let season = absolute.div_ceil(12);
            let number = (absolute - 1) % 12 + 1;
            let mut row = episode(
                &format!("ep-{season}-{number}"),
                &season.to_string(),
                &number.to_string(),
            );
            row.absolute_number = Some(absolute.to_string());
            episodes.push(row);
        }
        episodes
    }

    /// Community cours that coincide with the three TVDB seasons. The first
    /// cour answers to the bare franchise name as well as its own subtitle,
    /// which is how the anime metadata providers catalogue a first season.
    fn seasonal_bridge(cour_titles: [&[&str]; 3]) -> scryer_domain::AnimeNumberingBridge {
        let seasons = cour_titles
            .iter()
            .enumerate()
            .map(|(offset, titles)| {
                let index = i32::try_from(offset).expect("small index") + 1;
                scryer_domain::AnimeCommunitySeason {
                    index,
                    anidb_id: None,
                    anilist_id: None,
                    mal_id: None,
                    titles: titles.iter().map(|title| (*title).to_string()).collect(),
                    ranges: vec![scryer_domain::AnimeCommunitySeasonRange {
                        community_episode_start: 1,
                        community_episode_end: Some(12),
                        tvdb_season: index,
                        tvdb_episode_start: 1,
                        tvdb_episode_end: Some(12),
                    }],
                    absolute_start: Some((index - 1) * 12 + 1),
                    contiguous_absolute_start: None,
                    episode_count: Some(12),
                }
            })
            .collect();
        scryer_domain::AnimeNumberingBridge {
            source: Default::default(),
            generated_on: "2026-09-26".to_string(),
            corroborating_order: None,
            seasons,
        }
    }

    fn scan_one(
        title: &Title,
        episodes: &[Episode],
        bridge: Option<&scryer_domain::AnimeNumberingBridge>,
        path: &str,
    ) -> Vec<String> {
        let input = LibraryFilenameParseInput {
            path: Path::new(path),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes,
            existing_record: None,
            anime_numbering_bridge: bridge,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        };
        parse_library_filename(&input)
            .target_episodes()
            .into_iter()
            .map(|episode| episode.id)
            .collect()
    }

    /// The catalog's name for the series differs from the first cour's title
    /// only by punctuation (`Mein*Star` against `Mein Star`), and the files
    /// name the bare franchise. That name is the series, not its first cour,
    /// so each file keeps the season its own `SxxEyy` states.
    #[test]
    fn title_scan_keeps_the_stated_season_when_the_franchise_name_matches_the_first_cour() {
        let title = title("[Lantern Verge] - [Mein*Star]", MediaFacet::Anime);
        let episodes = seasonal_catalog_with_absolutes();
        let bridge = seasonal_bridge([
            &["Lantern Verge: Mein Star", "Lantern Verge"],
            &["Lantern Verge 2nd Season"],
            &["Lantern Verge 3rd Season"],
        ]);

        for (season, absolute) in [(1_u32, 1_u32), (2, 13), (3, 25)] {
            let path = format!(
                "/library/Lantern Verge (2023)/Season {season:02}/Lantern Verge (2023) - S{season:02}E01 - {absolute:03} - Ember Tide [WEBDL-1080p][JA][x265 10bit]-Cindergroup.mkv"
            );
            assert_eq!(
                scan_one(&title, &episodes, Some(&bridge), &path),
                vec![format!("ep-{season}-1")],
                "{path}"
            );
        }
    }

    /// `[EAC3 2.0]` describes the audio track. With the catalog's absolute
    /// numbers in the parse context its `2` used to read as absolute episode
    /// 2 and outrank the file's own `S02E06 - 018`.
    #[test]
    fn title_scan_never_reads_an_audio_channel_layout_as_the_episode() {
        let episodes = seasonal_catalog_with_absolutes();
        for facet in [MediaFacet::Series, MediaFacet::Anime] {
            let title = title("Cinder Atlas", facet.clone());
            for audio in ["EAC3 2.0", "EAC3 5.1", "AAC 7.1"] {
                let path = format!(
                    "/library/Cinder Atlas (2024)/Season 02/Cinder Atlas (2024) - S02E06 - 018 - Ember Tide [WEBDL-1080p][{audio}][JA][x265 10bit]-Cindergroup.mkv"
                );
                assert_eq!(
                    scan_one(&title, &episodes, None, &path),
                    vec!["ep-2-6".to_string()],
                    "{facet:?} {path}"
                );
            }
        }
    }

    fn scan_parse_with_existing(
        title: &Title,
        episodes: &[Episode],
        path: &str,
        existing_episode_id: Option<&str>,
    ) -> LibraryFilenameParse {
        parse_library_filename(&LibraryFilenameParseInput {
            path: Path::new(path),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes,
            existing_record: existing_episode_id.map(|episode_id| LibraryFilenameExistingRecord {
                episode_id: Some(episode_id),
                snapshot_matches: true,
            }),
            anime_numbering_bridge: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        })
    }

    fn confident_ids(parse: &LibraryFilenameParse) -> Option<Vec<String>> {
        parse
            .confident_episode_target()
            .map(|episodes| episodes.iter().map(|episode| episode.id.clone()).collect())
    }

    #[test]
    fn a_fresh_single_episode_parse_is_confident_enough_to_relink() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let episodes = vec![episode("ep-1-1", "1", "1"), episode("ep-1-2", "1", "2")];

        let parse = scan_parse_with_existing(
            &title,
            &episodes,
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E02.mkv",
            None,
        );

        assert_eq!(confident_ids(&parse), Some(vec!["ep-1-2".to_string()]));
    }

    #[test]
    fn a_stored_link_echoed_back_is_never_confident() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let episodes = vec![episode("ep-1-1", "1", "1"), episode("ep-1-2", "1", "2")];

        let parse = scan_parse_with_existing(
            &title,
            &episodes,
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E02.mkv",
            Some("ep-1-1"),
        );

        assert_eq!(parse.strategy, LibraryFilenameParseStrategy::ExistingRecord);
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn a_parse_without_an_episode_identity_is_never_confident() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let episodes = vec![episode("ep-1-1", "1", "1")];

        for path in [
            "/library/Quillmoor Heights/Quillmoor Heights - Bonus Reel.mkv",
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E09.mkv",
        ] {
            let parse = scan_parse_with_existing(&title, &episodes, path, None);
            assert!(parse.unmatched_reason().is_some(), "{path}");
            assert_eq!(confident_ids(&parse), None, "{path}");
        }
    }

    fn scan_lantern_verge(
        bridge: &scryer_domain::AnimeNumberingBridge,
        file_name: &str,
    ) -> LibraryFilenameParse {
        let title = title("Lantern Verge", MediaFacet::Anime);
        let episodes: Vec<Episode> = (1..=60)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string()))
            .collect();
        let path = format!("/library/Lantern Verge/{file_name}");
        parse_library_filename(&LibraryFilenameParseInput {
            path: Path::new(&path),
            display_name: None,
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            anime_numbering_bridge: Some(bridge),
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
        })
    }

    #[test]
    fn an_ambiguous_community_numbering_is_never_confident() {
        // Two community entries both answer to season 2, landing on different
        // official episodes, so nothing ranks one reading above the other.
        let mut bridge = anime_numbering_bridge_fixture();
        bridge.seasons[2].index = 2;

        let parse = scan_lantern_verge(&bridge, "Season 02/Lantern Verge - S02E03.mkv");

        assert_eq!(parse.unmatched_reason(), Some("anime_numbering_ambiguous"));
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn an_unresolved_community_pack_is_never_confident() {
        // The third cour's length is unknown, so a whole-cour file cannot be
        // bounded on the official numbering.
        let mut bridge = anime_numbering_bridge_fixture();
        bridge.seasons[2].episode_count = None;

        let parse = scan_lantern_verge(&bridge, "Season 03/Lantern Verge - S03.mkv");

        assert_eq!(parse.unmatched_reason(), Some("unresolved_pack_scope"));
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn a_community_numbered_single_episode_is_confident_on_its_official_episode() {
        let parse = scan_lantern_verge(
            &anime_numbering_bridge_fixture(),
            "Season 01/Lantern Verge - S04E20.mkv",
        );

        assert_eq!(confident_ids(&parse), Some(vec!["ep-56".to_string()]));
    }

    fn resolved_parse(
        identity: crate::ParsedEpisodeMetadata,
        episodes: Vec<Episode>,
    ) -> LibraryFilenameParse {
        LibraryFilenameParse {
            query_evidence: LibraryQueryEvidence::default(),
            parsed_release: crate::ParsedReleaseMetadata::empty("Quillmoor Heights", "test"),
            episode_identity: Some(identity.clone()),
            target: LibraryFilenameTarget::Episodes {
                episode_identity: identity,
                episodes,
                exact: true,
            },
            strategy: LibraryFilenameParseStrategy::ReleaseParserFallback,
            release_fallback_used: true,
        }
    }

    #[test]
    fn a_pack_or_partial_season_resolved_onto_one_file_is_never_confident() {
        let single = || crate::ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: vec![1],
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };
        for (label, identity) in [
            (
                "full season",
                crate::ParsedEpisodeMetadata {
                    full_season: true,
                    release_type: crate::ParsedEpisodeReleaseType::SeasonPack,
                    ..single()
                },
            ),
            (
                "partial season",
                crate::ParsedEpisodeMetadata {
                    is_partial_season: true,
                    ..single()
                },
            ),
            (
                "several seasons",
                crate::ParsedEpisodeMetadata {
                    is_multi_season: true,
                    ..single()
                },
            ),
        ] {
            let parse = resolved_parse(identity, vec![episode("ep-1-1", "1", "1")]);
            assert_eq!(confident_ids(&parse), None, "{label}");
        }
    }

    #[test]
    fn an_explicit_episode_range_in_one_file_is_confident() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let episodes = vec![
            episode("ep-1-1", "1", "1"),
            episode("ep-1-2", "1", "2"),
            episode("ep-1-3", "1", "3"),
        ];

        for path in [
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E01E02 - Tide.mkv",
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E01-E02.mkv",
        ] {
            let parse = scan_parse_with_existing(&title, &episodes, path, None);
            assert_eq!(
                confident_ids(&parse),
                Some(vec!["ep-1-1".to_string(), "ep-1-2".to_string()]),
                "{path}"
            );
        }
    }

    #[test]
    fn a_range_the_catalog_only_partly_holds_is_never_confident() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let episodes = vec![episode("ep-1-11", "1", "11"), episode("ep-1-12", "1", "12")];

        let parse = scan_parse_with_existing(
            &title,
            &episodes,
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E11-E13.mkv",
            None,
        );

        assert_eq!(
            parse
                .target_episodes()
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ep-1-11", "ep-1-12"],
            "the file still lands on the episodes the catalog holds"
        );
        assert_eq!(confident_ids(&parse), None);

        let mut complete = episodes.clone();
        complete.push(episode("ep-1-13", "1", "13"));
        let parse = scan_parse_with_existing(
            &title,
            &complete,
            "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E11-E13.mkv",
            None,
        );
        assert_eq!(
            confident_ids(&parse),
            Some(vec![
                "ep-1-11".to_string(),
                "ep-1-12".to_string(),
                "ep-1-13".to_string()
            ])
        );
    }

    #[test]
    fn an_episode_key_the_catalog_holds_twice_is_never_confident() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let path = "/library/Quillmoor Heights/Season 01/Quillmoor Heights - S01E02.mkv";
        let episodes = vec![
            episode("ep-1-1", "1", "1"),
            episode("ep-1-2", "1", "2"),
            episode("ep-1-2-renumbered", "1", "2"),
        ];

        let parse = scan_parse_with_existing(&title, &episodes, path, None);

        assert!(!parse.target_episodes().is_empty());
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn an_absolute_number_the_catalog_holds_twice_is_never_confident() {
        let title = title("Lantern Verge", MediaFacet::Anime);
        let path = "/library/Lantern Verge/Lantern Verge - 05.mkv";
        let with_absolute = |id: &str, season: &str, number: &str| Episode {
            absolute_number: Some("5".to_string()),
            ..episode(id, season, number)
        };

        let unique = vec![with_absolute("ep-2-1", "2", "1")];
        let parse = scan_parse_with_existing(&title, &unique, path, None);
        assert_eq!(confident_ids(&parse), Some(vec!["ep-2-1".to_string()]));

        let duplicated = vec![
            with_absolute("ep-2-1", "2", "1"),
            with_absolute("ep-2-2", "2", "2"),
        ];
        let parse = scan_parse_with_existing(&title, &duplicated, path, None);
        assert!(!parse.target_episodes().is_empty());
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn an_air_date_several_episodes_share_is_never_confident_without_a_part() {
        let title = title("Quillmoor Heights", MediaFacet::Series);
        let path = "/library/Quillmoor Heights/Quillmoor Heights - 2024-03-05.mkv";
        let aired = |id: &str, number: &str| Episode {
            air_date: Some("2024-03-05".to_string()),
            ..episode(id, "2024", number)
        };

        let single = vec![aired("ep-daily-1", "1")];
        let parse = scan_parse_with_existing(&title, &single, path, None);
        assert_eq!(confident_ids(&parse), Some(vec!["ep-daily-1".to_string()]));

        let shared = vec![aired("ep-daily-1", "1"), aired("ep-daily-2", "2")];
        let parse = scan_parse_with_existing(&title, &shared, path, None);
        assert_eq!(parse.target_episodes().len(), 2);
        assert_eq!(confident_ids(&parse), None);
    }

    #[test]
    fn a_special_is_confident_only_when_it_resolves_into_season_zero() {
        let special = crate::ParsedEpisodeMetadata {
            season: Some(0),
            episode_numbers: vec![2],
            special_kind: Some(crate::ParsedSpecialKind::Ova),
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };

        let outside = resolved_parse(special.clone(), vec![episode("ep-1-2", "1", "2")]);
        assert_eq!(confident_ids(&outside), None);

        let inside = resolved_parse(special, vec![episode("sp-2", "0", "2")]);
        assert_eq!(confident_ids(&inside), Some(vec!["sp-2".to_string()]));
    }

    #[test]
    fn a_shared_title_index_parses_every_file_the_way_a_standalone_parse_does() {
        let title = title("Lantern Verge", MediaFacet::Anime);
        let episodes: Vec<Episode> = (1..=60)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string()))
            .collect();
        let bridge = anime_numbering_bridge_fixture();
        let index = LibraryFilenameTitleIndex::default();
        for file_name in [
            "Season 01/Lantern Verge - S04E20.mkv",
            "Season 01/Lantern Verge - S01E03.mkv",
            "Season 01/Lantern Verge - S01E02E03.mkv",
            "Lantern Verge - Harbor Bonus Reel.mkv",
        ] {
            let path = format!("/library/Lantern Verge/{file_name}");
            let input = LibraryFilenameParseInput {
                path: Path::new(&path),
                display_name: None,
                library_root: Some(Path::new("/library")),
                title: Some(&title),
                facet: Some(&title.facet),
                collections: &[],
                series_movie_links: &[],
                episodes: &episodes,
                existing_record: None,
                anime_numbering_bridge: Some(&bridge),
                mode: LibraryFilenameParseMode::TitleScan,
                fallback_policy: LibraryFilenameFallbackPolicy::WhenNeeded,
            };

            let standalone = parse_library_filename(&input);
            let shared = parse_library_filename_with_index(&input, Some(&index));

            assert_eq!(target_ids(&shared), target_ids(&standalone), "{file_name}");
            assert_eq!(shared.target, standalone.target, "{file_name}");
            assert_eq!(
                shared.parsed_release.episode, standalone.parsed_release.episode,
                "{file_name}"
            );
        }
    }
}
