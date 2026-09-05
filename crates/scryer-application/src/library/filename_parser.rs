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
}

struct QueryEvidenceBuild {
    evidence: LibraryQueryEvidence,
    parsed_release: Option<crate::ParsedReleaseMetadata>,
}

pub(crate) fn parse_library_filename(
    input: &LibraryFilenameParseInput<'_>,
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
            },
            strategy: LibraryFilenameParseStrategy::ExistingRecord,
            release_fallback_used: false,
        };
    }

    let mut fallback = parse_release_fallback(input, &raw_name);
    release_fallback_used = true;
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
        let episodes = resolve_episodes_from_identity_with_season(
            &episode_identity,
            &season_str,
            input.collections,
            input.episodes,
        );
        if !episodes.is_empty() {
            return LibraryFilenameParse {
                query_evidence: query_build.evidence,
                parsed_release: fallback,
                episode_identity: Some(episode_identity.clone()),
                target: LibraryFilenameTarget::Episodes {
                    episode_identity,
                    episodes,
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

    let year = file_walk
        .as_ref()
        .and_then(|walk| walk.year)
        .or_else(|| {
            parsed_has_usable_title_signal
                .then_some(parsed.as_ref().and_then(|parsed| parsed.year))
                .flatten()
                .and_then(|year| u32::try_from(year).ok())
        })
        .or_else(|| folder_walk.as_ref().and_then(|walk| walk.year))
        .or(folder_year);

    QueryEvidenceBuild {
        evidence: LibraryQueryEvidence {
            queries,
            year,
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

fn resolve_episodes_from_identity_with_season(
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
    collections: &[Collection],
    episodes: &[Episode],
) -> Vec<Episode> {
    let lookup = build_episode_lookup(collections, episodes);
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        if let Some(matches) = lookup.by_air_date.get(&air_date_str) {
            if let Some(part) = ep_meta.daily_part {
                let part_index = part.saturating_sub(1) as usize;
                if let Some(episode) = matches.get(part_index)
                    && seen.insert(episode.id.clone())
                {
                    resolved.push(episode.clone());
                }
            } else {
                for episode in matches {
                    if seen.insert(episode.id.clone()) {
                        resolved.push(episode.clone());
                    }
                }
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let key = (target_season.clone(), episode_number.to_string());
        if let Some(episode) = lookup.by_collection_episode.get(&key)
            && seen.insert(episode.id.clone())
        {
            resolved.push(episode.clone());
        }
    }

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
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let key = ("0".to_string(), special_number.to_string());
            if let Some(episode) = lookup.by_collection_episode.get(&key)
                && seen.insert(episode.id.clone())
            {
                resolved.push(episode.clone());
            }
        }
    }

    if resolved.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            if let Some(episode) = lookup.by_absolute_number.get(&absolute_number.to_string())
                && seen.insert(episode.id.clone())
            {
                resolved.push(episode.clone());
            }
        }
    }

    resolved
}

#[derive(Default)]
struct EpisodeLookup {
    by_air_date: HashMap<String, Vec<Episode>>,
    by_collection_episode: HashMap<(String, String), Episode>,
    by_absolute_number: HashMap<String, Episode>,
    by_collection_index: HashMap<String, Vec<Episode>>,
}

fn build_episode_lookup(collections: &[Collection], episodes: &[Episode]) -> EpisodeLookup {
    let collection_indexes = collections
        .iter()
        .map(|collection| (collection.id.clone(), collection.collection_index.clone()))
        .collect::<HashMap<_, _>>();

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
                lookup
                    .by_collection_episode
                    .entry((collection_index.clone(), episode_number.clone()))
                    .or_insert_with(|| episode.clone());
            } else {
                lookup
                    .by_collection_episode
                    .entry((season_number.clone(), episode_number.clone()))
                    .or_insert_with(|| episode.clone());
            }
        }

        if let Some(absolute_number) = episode.absolute_number.as_ref() {
            lookup
                .by_absolute_number
                .entry(absolute_number.clone())
                .or_insert_with(|| episode.clone());
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
    let raw_key = library_name_match_key(raw_name);
    if raw_key.is_empty() {
        return None;
    }

    input
        .series_movie_links
        .iter()
        .filter_map(|link| {
            if let (Some(filename_year), Some(movie_year)) = (filename_year, link.movie.year)
                && filename_year != movie_year
            {
                return None;
            }

            let movie_matches = series_movie_match_keys(link)
                .into_iter()
                .any(|key| normalized_key_contains_phrase(&raw_key, &key));
            if !movie_matches {
                return None;
            }

            Some(build_series_movie_target(link, input.episodes))
        })
        .next()
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
) -> crate::ParsedReleaseMetadata {
    let mut parsed = parse_release_fallback_name(input, raw_name);
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

    let parent_release = parse_release_fallback_name(input, &parent_name);
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
) -> crate::ParsedReleaseMetadata {
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

fn parsed_episode_metadata_from_episode(episode: &Episode) -> crate::ParsedEpisodeMetadata {
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
        absolute_episode: episode
            .absolute_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        absolute_episode_numbers: episode
            .absolute_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .into_iter()
            .collect(),
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
            external_ids: vec![ExternalId {
                source: "tvdb".into(),
                value: "12345".into(),
            }],
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
            overview: None,
            tvdb_id: None,
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
}
