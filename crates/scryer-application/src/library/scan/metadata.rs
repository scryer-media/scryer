use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

use crate::library::library::library_scan_cancel_requested;
use crate::library_discovery::{
    LibraryTitleWalk, MovieTopLevelEntry, matching_movie_nfo_path_async, normalize_folder_name,
    strip_year_suffix,
};
use crate::library_filename_parser::{LibraryFilenameParseInput, parse_library_filename};
use crate::library_scan_coordinator::LibraryScanCoordinator;
use crate::nfo::{NfoMetadata, NfoRootKind, detect_nfo_root_kind, parse_nfo, parse_plexmatch};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AppError, AppResult, ExternalIdProvider, LibraryFile, LibraryScanHint, LibraryScanHintFacet,
    LibraryScanHintSet, LibraryScanHintSource, LibraryScanUnmatchedSearchAttempt, LibraryScanner,
    MetadataGateway, MetadataSearchItem, MetadataSearchQuery, await_cancellable_app_result,
};

pub(crate) const METADATA_TYPE_MOVIE: &str = "movie";
pub(crate) const METADATA_TYPE_SERIES: &str = "series";

pub(crate) const LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE: usize = 50;
const MOVIE_ENTRY_PREP_CONCURRENCY: usize = 8;
const RADARR_MOVIE_NFO_MAX_BYTES: u64 = 10 * 1024 * 1024;
const PLEXMATCH_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MetadataIdentitySource {
    ExternalImportRadarr,
    ExternalImportSonarr,
    Nfo,
    Plexmatch,
    Filename,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MetadataIdentityHint {
    pub(crate) source: MetadataIdentitySource,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) year: Option<u32>,
}

impl MetadataIdentityHint {
    pub(crate) fn has_external_ids(&self) -> bool {
        self.imdb_id.is_some() || self.tmdb_id.is_some() || self.tvdb_id.is_some()
    }

    pub(crate) fn is_external_import_hint(&self) -> bool {
        matches!(
            self.source,
            MetadataIdentitySource::ExternalImportRadarr
                | MetadataIdentitySource::ExternalImportSonarr
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BatchMetadataSearchKey {
    type_hint: &'static str,
    query: String,
    year: Option<i32>,
    imdb_id: Option<String>,
    tmdb_id: Option<String>,
    tvdb_id: Option<String>,
}

impl BatchMetadataSearchKey {
    pub(crate) fn new(
        type_hint: &'static str,
        query: &str,
        year: Option<u32>,
        identity_hint: Option<&MetadataIdentityHint>,
    ) -> Option<Self> {
        let trimmed = query.trim();
        if trimmed.is_empty() && !identity_hint.is_some_and(MetadataIdentityHint::has_external_ids)
        {
            return None;
        }

        Some(Self {
            type_hint,
            query: trimmed.to_string(),
            year: year.map(|value| value as i32),
            imdb_id: identity_hint.and_then(|hint| hint.imdb_id.clone()),
            tmdb_id: identity_hint.and_then(|hint| hint.tmdb_id.clone()),
            tvdb_id: identity_hint.and_then(|hint| hint.tvdb_id.clone()),
        })
    }

    pub(crate) fn has_external_id(&self) -> bool {
        self.imdb_id.is_some() || self.tmdb_id.is_some() || self.tvdb_id.is_some()
    }
}

type SharedMetadataSearchItems = Arc<Vec<MetadataSearchItem>>;
pub(crate) type MetadataSearchResults = HashMap<BatchMetadataSearchKey, SharedMetadataSearchItems>;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MetadataLookupBatchStats {
    pub(crate) logical_lookups: usize,
    pub(crate) executed_requests: usize,
    pub(crate) coalesced_requests: usize,
}

impl MetadataLookupBatchStats {
    fn absorb(&mut self, other: Self) {
        self.logical_lookups = self.logical_lookups.saturating_add(other.logical_lookups);
        self.executed_requests = self
            .executed_requests
            .saturating_add(other.executed_requests);
        self.coalesced_requests = self
            .coalesced_requests
            .saturating_add(other.coalesced_requests);
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct MovieLibraryScanCandidate {
    pub(crate) selected_metadata: Option<MetadataSearchItem>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct SeriesLibraryScanCandidate {
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) query: String,
    pub(crate) selected_metadata: Option<MetadataSearchItem>,
    pub(crate) metadata_lookup_error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedMovieLibraryScanCandidate {
    pub(crate) file: LibraryFile,
    pub(crate) representative_is_directory: bool,
    pub(crate) discovered_files: Vec<LibraryFile>,
    pub(crate) parsed_release: crate::ParsedReleaseMetadata,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) identity_hint: Option<MetadataIdentityHint>,
    pub(crate) query: String,
    pub(crate) year_hint: Option<u32>,
    pub(crate) query_variants: Vec<String>,
    pub(crate) search_candidates: Vec<String>,
    pub(crate) metadata_lookup_attempted: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSeriesLibraryScanCandidate {
    pub(crate) folder_path: PathBuf,
    pub(crate) folder_name: Option<String>,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) identity_hint: Option<MetadataIdentityHint>,
    pub(crate) query: String,
    pub(crate) year_hint: Option<u32>,
    pub(crate) search_candidates: Vec<String>,
    pub(crate) title_match_candidates: Vec<String>,
    pub(crate) metadata_lookup_attempted: bool,
}

impl PreparedSeriesLibraryScanCandidate {
    pub(crate) fn item_path(&self) -> String {
        path_to_stored_string(&self.folder_path)
    }
}

pub(crate) async fn read_valid_movie_nfo_metadata(
    nfo_path: Option<&str>,
) -> Option<crate::nfo::NfoMetadata> {
    let path = stored_path_to_path_buf(nfo_path?);
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() > RADARR_MOVIE_NFO_MAX_BYTES {
        return None;
    }

    let content = tokio::fs::read_to_string(path).await.ok()?;
    let root_kind = detect_nfo_root_kind(&content);
    let meta = parse_nfo(&content);
    if root_kind != NfoRootKind::Movie
        && !(root_kind == NfoRootKind::Other && meta.has_external_ids())
    {
        return None;
    }

    Some(meta)
}

async fn read_tvshow_nfo_metadata(folder: PathBuf) -> Option<crate::nfo::NfoMetadata> {
    let path = folder.join("tvshow.nfo");
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    let content = tokio::fs::read_to_string(path).await.ok()?;
    Some(parse_nfo(&content))
}

async fn read_plexmatch_metadata(folder: Option<PathBuf>) -> Option<NfoMetadata> {
    let path = folder?.join(".plexmatch");
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() > PLEXMATCH_MAX_BYTES {
        return None;
    }
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let meta = parse_plexmatch(&content);
    (!meta.is_empty()).then_some(meta)
}

fn normalized_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn metadata_identity_hint_from_nfo(
    source: MetadataIdentitySource,
    meta: &NfoMetadata,
    fallback_year: Option<u32>,
) -> Option<MetadataIdentityHint> {
    let hint = MetadataIdentityHint {
        source,
        imdb_id: meta
            .imdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_imdb_id),
        tmdb_id: meta
            .tmdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id),
        tvdb_id: meta
            .tvdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id),
        title: normalized_non_empty(meta.title.as_deref()),
        year: meta.year.map(|value| value as u32).or(fallback_year),
    };
    (hint.has_external_ids() || hint.title.is_some()).then_some(hint)
}

fn metadata_identity_hint_from_filename(
    parsed: &crate::ParsedReleaseMetadata,
    fallback_query: &str,
    fallback_year: Option<u32>,
) -> Option<MetadataIdentityHint> {
    let title = normalized_non_empty(Some(fallback_query))
        .or_else(|| normalized_non_empty(Some(parsed.normalized_title.as_str())));
    let hint = MetadataIdentityHint {
        source: MetadataIdentitySource::Filename,
        imdb_id: parsed
            .imdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_imdb_id),
        tmdb_id: parsed
            .tmdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id),
        tvdb_id: parsed
            .tvdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id),
        title,
        year: parsed.year.map(|value| value as u32).or(fallback_year),
    };
    (hint.has_external_ids() || hint.title.is_some()).then_some(hint)
}

fn metadata_identity_hint_from_title_walk(
    walk: Option<&LibraryTitleWalk>,
) -> Option<MetadataIdentityHint> {
    let walk = walk?;
    let hint = MetadataIdentityHint {
        source: MetadataIdentitySource::Filename,
        imdb_id: walk.imdb_id.clone(),
        tmdb_id: walk.tmdb_id.clone(),
        tvdb_id: walk.tvdb_id.clone(),
        title: normalized_non_empty(walk.title.as_deref()),
        year: walk.year,
    };
    (hint.has_external_ids() || hint.title.is_some()).then_some(hint)
}

fn metadata_identity_hint_from_library_scan_hint(
    hint: Option<&LibraryScanHint>,
) -> Option<MetadataIdentityHint> {
    let hint = hint?;
    let mut identity_hint = MetadataIdentityHint {
        source: match hint.source {
            LibraryScanHintSource::ExternalImportRadarr => {
                MetadataIdentitySource::ExternalImportRadarr
            }
            LibraryScanHintSource::ExternalImportSonarr => {
                MetadataIdentitySource::ExternalImportSonarr
            }
        },
        imdb_id: None,
        tmdb_id: None,
        tvdb_id: None,
        title: None,
        year: None,
    };

    for id in &hint.ids {
        match id.provider {
            ExternalIdProvider::Imdb => identity_hint.imdb_id = Some(id.value.clone()),
            ExternalIdProvider::Tmdb => identity_hint.tmdb_id = Some(id.value.clone()),
            ExternalIdProvider::Tvdb => identity_hint.tvdb_id = Some(id.value.clone()),
        }
    }

    identity_hint.has_external_ids().then_some(identity_hint)
}

fn external_import_identity_hint_for_scan_path(
    scan_hints: Option<&LibraryScanHintSet>,
    facet: LibraryScanHintFacet,
    leaf_key: Option<&str>,
    full_path_key: Option<&str>,
) -> Option<MetadataIdentityHint> {
    let scan_hint = scan_hints?.hint_for_scan_path(facet, leaf_key?, full_path_key)?;
    metadata_identity_hint_from_library_scan_hint(Some(scan_hint))
}

struct MetadataIdentityHintSelection<'a> {
    library_scan_hint: Option<&'a LibraryScanHint>,
    nfo_meta: Option<&'a NfoMetadata>,
    plexmatch_meta: Option<&'a NfoMetadata>,
    file_walk: Option<&'a LibraryTitleWalk>,
    folder_walk: Option<&'a LibraryTitleWalk>,
    parsed: &'a crate::ParsedReleaseMetadata,
    fallback_query: &'a str,
    fallback_year: Option<u32>,
}

fn select_metadata_identity_hint(
    selection: MetadataIdentityHintSelection<'_>,
) -> Option<MetadataIdentityHint> {
    let MetadataIdentityHintSelection {
        library_scan_hint,
        nfo_meta,
        plexmatch_meta,
        file_walk,
        folder_walk,
        parsed,
        fallback_query,
        fallback_year,
    } = selection;

    metadata_identity_hint_from_library_scan_hint(library_scan_hint)
        .or_else(|| {
            nfo_meta.and_then(|meta| {
                metadata_identity_hint_from_nfo(MetadataIdentitySource::Nfo, meta, fallback_year)
            })
        })
        .or_else(|| {
            plexmatch_meta.and_then(|meta| {
                metadata_identity_hint_from_nfo(
                    MetadataIdentitySource::Plexmatch,
                    meta,
                    fallback_year,
                )
            })
        })
        .or_else(|| metadata_identity_hint_from_title_walk(file_walk))
        .or_else(|| metadata_identity_hint_from_title_walk(folder_walk))
        .or_else(|| metadata_identity_hint_from_filename(parsed, fallback_query, fallback_year))
}

fn push_unique_batch_metadata_search(
    searches: &mut Vec<BatchMetadataSearchKey>,
    seen: &mut HashSet<BatchMetadataSearchKey>,
    type_hint: &'static str,
    query: &str,
    year: Option<u32>,
    identity_hint: Option<&MetadataIdentityHint>,
) {
    let Some(key) = BatchMetadataSearchKey::new(type_hint, query, year, identity_hint) else {
        return;
    };

    if seen.insert(key.clone()) {
        searches.push(key);
    }
}

fn summarize_metadata_search_item(item: &MetadataSearchItem) -> String {
    match item.year {
        Some(year) => format!("{} ({year})", item.name),
        None => item.name.clone(),
    }
}

pub(crate) fn build_library_scan_unmatched_search_attempts(
    type_hint: &'static str,
    search_candidates: &[String],
    year_hint: Option<u32>,
    identity_hint: Option<&MetadataIdentityHint>,
    batch_search_results: &MetadataSearchResults,
) -> Vec<LibraryScanUnmatchedSearchAttempt> {
    search_candidates
        .iter()
        .filter_map(|search_candidate| {
            let key =
                BatchMetadataSearchKey::new(type_hint, search_candidate, year_hint, identity_hint)?;
            let results = batch_search_results
                .get(&key)
                .map_or(&[][..], |items| items.as_slice());

            Some(LibraryScanUnmatchedSearchAttempt {
                query: search_candidate.clone(),
                result_count: results.len(),
                top_results: results
                    .iter()
                    .take(3)
                    .map(summarize_metadata_search_item)
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn library_scan_unmatched_reason_code(
    search_attempts: &[LibraryScanUnmatchedSearchAttempt],
) -> &'static str {
    if search_attempts
        .iter()
        .all(|attempt| attempt.result_count == 0)
    {
        "no_metadata_search_results"
    } else {
        "no_acceptable_metadata_match"
    }
}

pub(crate) async fn execute_batch_metadata_searches(
    metadata_gateway: Arc<dyn MetadataGateway>,
    search_keys: Vec<BatchMetadataSearchKey>,
    metadata_language: &str,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<MetadataSearchResults> {
    if search_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut movie_queries_without_external_id = Vec::new();
    let mut movie_queries_with_external_id = Vec::new();
    let mut series_queries = Vec::new();
    for key in &search_keys {
        let query = MetadataSearchQuery {
            query: key.query.clone(),
            type_hint: key.type_hint.to_string(),
            year: key.year,
            imdb_id: key.imdb_id.clone(),
            tmdb_id: key.tmdb_id.clone(),
            tvdb_id: key.tvdb_id.clone(),
        };
        match key.type_hint {
            METADATA_TYPE_MOVIE if key.has_external_id() => {
                movie_queries_with_external_id.push(query);
            }
            METADATA_TYPE_MOVIE => movie_queries_without_external_id.push(query),
            _ => series_queries.push(query),
        }
    }

    let mut batched_results = HashMap::new();
    for (queries, create_missing) in [
        (movie_queries_without_external_id, false),
        (movie_queries_with_external_id, true),
    ] {
        if queries.is_empty() {
            continue;
        }
        let movie_results = match await_cancellable_app_result(
            cancel_token,
            metadata_gateway.search_titles_batch(
                &queries,
                METADATA_TYPE_MOVIE,
                metadata_language,
                create_missing,
            ),
        )
        .await
        {
            Ok(Some(results)) => results,
            Ok(None) => return Ok(HashMap::new()),
            Err(error) if crate::catalog_workflow::movie_title_queries_not_supported(&error) => {
                let Some(results) = await_cancellable_app_result(
                    cancel_token,
                    metadata_gateway.search_tvdb_batch(&queries, metadata_language),
                )
                .await?
                else {
                    return Ok(HashMap::new());
                };
                results
            }
            Err(error) => return Err(error),
        };
        batched_results.extend(movie_results);
    }

    if !series_queries.is_empty() {
        let Some(series_results) = await_cancellable_app_result(
            cancel_token,
            metadata_gateway.search_tvdb_batch(&series_queries, metadata_language),
        )
        .await?
        else {
            return Ok(HashMap::new());
        };
        batched_results.extend(series_results);
    }

    let mut results = HashMap::new();
    for key in search_keys {
        let result_key = MetadataSearchQuery {
            query: key.query.clone(),
            type_hint: key.type_hint.to_string(),
            year: key.year,
            imdb_id: key.imdb_id.clone(),
            tmdb_id: key.tmdb_id.clone(),
            tvdb_id: key.tvdb_id.clone(),
        };
        let items = batched_results
            .get(&result_key)
            .cloned()
            .unwrap_or_default();
        results.insert(key, Arc::new(items));
    }

    Ok(results)
}

pub(crate) fn build_movie_metadata_batch_stats(
    candidates: &[PreparedMovieLibraryScanCandidate],
) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats) {
    let mut stats = MetadataLookupBatchStats::default();
    let mut total_requested_searches = 0usize;
    let mut batch_searches = Vec::new();
    let mut seen_batch_searches = HashSet::new();

    for candidate in candidates {
        if !candidate.metadata_lookup_attempted {
            continue;
        }

        stats.logical_lookups = stats.logical_lookups.saturating_add(1);
        total_requested_searches =
            total_requested_searches.saturating_add(candidate.search_candidates.len());
        for search_candidate in &candidate.search_candidates {
            push_unique_batch_metadata_search(
                &mut batch_searches,
                &mut seen_batch_searches,
                METADATA_TYPE_MOVIE,
                search_candidate,
                candidate.year_hint,
                candidate.identity_hint.as_ref(),
            );
        }
    }

    stats.executed_requests = batch_searches.len();
    stats.coalesced_requests = total_requested_searches.saturating_sub(stats.executed_requests);
    (batch_searches, stats)
}

pub(crate) fn build_series_metadata_batch_stats(
    candidates: &[PreparedSeriesLibraryScanCandidate],
) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats) {
    let mut stats = MetadataLookupBatchStats::default();
    let mut batch_searches = Vec::new();
    let mut seen_batch_searches = HashSet::new();

    for candidate in candidates {
        if !candidate.metadata_lookup_attempted {
            continue;
        }

        stats.logical_lookups = stats.logical_lookups.saturating_add(1);
        for search_candidate in &candidate.search_candidates {
            push_unique_batch_metadata_search(
                &mut batch_searches,
                &mut seen_batch_searches,
                METADATA_TYPE_SERIES,
                search_candidate,
                candidate.year_hint,
                candidate.identity_hint.as_ref(),
            );
        }
    }

    stats.executed_requests = batch_searches.len();
    stats.coalesced_requests = stats
        .logical_lookups
        .saturating_sub(stats.executed_requests);
    (batch_searches, stats)
}

pub(crate) fn movie_candidate_batch_search_keys(
    candidate: &PreparedMovieLibraryScanCandidate,
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    let mut keys = Vec::with_capacity(candidate.search_candidates.len());

    for search_candidate in &candidate.search_candidates {
        keys.push(
            BatchMetadataSearchKey::new(
                METADATA_TYPE_MOVIE,
                search_candidate,
                candidate.year_hint,
                candidate.identity_hint.as_ref(),
            )
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "movie metadata lookup key unexpectedly missing for query '{}'",
                    search_candidate
                ))
            })?,
        );
    }

    Ok(keys)
}

pub(crate) fn series_candidate_batch_search_keys(
    candidate: &PreparedSeriesLibraryScanCandidate,
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(Vec::new());
    }

    candidate
        .search_candidates
        .iter()
        .map(|search_candidate| {
            BatchMetadataSearchKey::new(
                METADATA_TYPE_SERIES,
                search_candidate,
                candidate.year_hint,
                candidate.identity_hint.as_ref(),
            )
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "series metadata lookup key unexpectedly missing for query '{}'",
                    search_candidate
                ))
            })
        })
        .collect()
}

pub(crate) fn build_title_match_candidates(queries: &[String]) -> Vec<String> {
    let mut title_match_candidates = Vec::new();
    let mut title_match_seen = HashSet::new();

    for query in queries {
        let title_match_key = crate::title_matching::canonical_lookup_key(query);
        if !title_match_key.is_empty() && title_match_seen.insert(title_match_key.clone()) {
            title_match_candidates.push(title_match_key);
        }
    }

    title_match_candidates
}

fn expand_search_candidates(queries: &[String]) -> Vec<String> {
    let mut search_candidates = Vec::new();
    let mut seen = HashSet::new();

    for query in queries {
        for variant in crate::title_matching::search_variants(query) {
            let dedupe_key = variant
                .nfkc()
                .flat_map(char::to_lowercase)
                .collect::<String>();
            if dedupe_key.trim().is_empty() || !seen.insert(dedupe_key) {
                continue;
            }
            search_candidates.push(variant);
        }
    }

    search_candidates
}

pub(crate) fn split_ready_metadata_candidates<T, F>(
    candidates: Vec<T>,
    search_results: &MetadataSearchResults,
    mut candidate_keys: F,
) -> AppResult<(Vec<T>, Vec<T>)>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut ready = Vec::new();
    let mut pending = Vec::new();

    for candidate in candidates {
        let keys = candidate_keys(&candidate)?;
        if metadata_candidate_has_auto_safe_result(&keys, search_results)
            || keys.iter().all(|key| search_results.contains_key(key))
        {
            ready.push(candidate);
        } else {
            pending.push(candidate);
        }
    }

    Ok((ready, pending))
}

fn metadata_candidate_has_auto_safe_result(
    keys: &[BatchMetadataSearchKey],
    search_results: &MetadataSearchResults,
) -> bool {
    keys.iter()
        .filter_map(|key| search_results.get(key))
        .any(|items| items.iter().any(|item| item.auto_match_safe))
}

pub(crate) fn next_metadata_search_chunk<T, F>(
    candidates: &[T],
    search_results: &MetadataSearchResults,
    max_keys: usize,
    mut candidate_keys: F,
) -> AppResult<Vec<BatchMetadataSearchKey>>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut chunk = Vec::new();
    let mut seen = HashSet::new();

    for candidate in candidates {
        if chunk.len() >= max_keys {
            break;
        }

        let Some(key) = candidate_keys(candidate)?
            .into_iter()
            .find(|key| !search_results.contains_key(key) && seen.insert(key.clone()))
        else {
            continue;
        };

        chunk.push(key);
    }

    Ok(chunk)
}

fn count_candidates_with_metadata_lookup<T, F>(
    candidates: &[T],
    mut candidate_keys: F,
) -> AppResult<usize>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut count = 0usize;

    for candidate in candidates {
        if !candidate_keys(candidate)?.is_empty() {
            count = count.saturating_add(1);
        }
    }

    Ok(count)
}

#[expect(
    clippy::too_many_arguments,
    reason = "batched metadata resolution coordinates gateway, progress, and candidate state explicitly"
)]
pub(crate) async fn resolve_refresh_metadata_batches<T, BuildStats, CandidateKeys>(
    metadata_gateway: Arc<dyn MetadataGateway>,
    metadata_language: &str,
    coordinator: &LibraryScanCoordinator,
    unresolved_candidates: Vec<T>,
    metadata_lookup_stats: &mut MetadataLookupBatchStats,
    build_stats: BuildStats,
    candidate_keys: CandidateKeys,
    empty_chunk_message: &'static str,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<(Vec<Vec<T>>, MetadataSearchResults)>
where
    BuildStats: Fn(&[T]) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats),
    CandidateKeys: Fn(&T) -> AppResult<Vec<BatchMetadataSearchKey>> + Copy,
{
    let (_searches, batch_lookup_stats) = build_stats(&unresolved_candidates);
    metadata_lookup_stats.absorb(batch_lookup_stats);

    if unresolved_candidates.is_empty() {
        return Ok((Vec::new(), MetadataSearchResults::new()));
    }

    if batch_lookup_stats.logical_lookups > 0 {
        coordinator
            .add_metadata_total(batch_lookup_stats.logical_lookups)
            .await;
        coordinator.mark_metadata_total_known().await;
    }
    coordinator.publish_progress().await;

    let mut pending_candidates = unresolved_candidates;
    let mut ready_batches = Vec::new();
    let mut batch_search_results = MetadataSearchResults::new();

    while !pending_candidates.is_empty() {
        let (ready_candidates, still_pending) = split_ready_metadata_candidates(
            pending_candidates,
            &batch_search_results,
            candidate_keys,
        )?;
        pending_candidates = still_pending;

        if !ready_candidates.is_empty() {
            let ready_lookup_count =
                count_candidates_with_metadata_lookup(&ready_candidates, candidate_keys)?;
            if ready_lookup_count > 0 {
                coordinator
                    .mark_metadata_completed(ready_lookup_count)
                    .await;
                coordinator.publish_progress().await;
            }
            ready_batches.push(ready_candidates);
            continue;
        }

        if library_scan_cancel_requested(cancel_token) {
            break;
        }

        let search_chunk = next_metadata_search_chunk(
            &pending_candidates,
            &batch_search_results,
            LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE,
            candidate_keys,
        )?;
        if search_chunk.is_empty() {
            return Err(AppError::Repository(empty_chunk_message.into()));
        }

        batch_search_results.extend(
            execute_batch_metadata_searches(
                metadata_gateway.clone(),
                search_chunk,
                metadata_language,
                cancel_token,
            )
            .await?,
        );
    }

    Ok((ready_batches, batch_search_results))
}

#[cfg(test)]
pub(crate) async fn prepare_movie_library_scan_candidates(
    files: &[LibraryFile],
    library_path: &str,
) -> AppResult<Vec<PreparedMovieLibraryScanCandidate>> {
    let mut prepare_set = tokio::task::JoinSet::new();

    for (index, file) in files.iter().cloned().enumerate() {
        let library_path = library_path.to_string();
        prepare_set.spawn(async move {
            Ok::<_, AppError>((
                index,
                prepare_movie_library_scan_candidate(file, library_path).await?,
            ))
        });
    }

    let mut prepared_results = vec![None; prepare_set.len()];
    while let Some(result) = prepare_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        prepared_results[index] = Some(candidate);
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

pub(crate) async fn prepare_series_library_scan_candidates(
    folders: &[PathBuf],
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<Vec<PreparedSeriesLibraryScanCandidate>> {
    let mut prepare_set = tokio::task::JoinSet::new();

    for (index, folder) in folders.iter().cloned().enumerate() {
        let scan_hints = scan_hints.cloned();
        prepare_set.spawn(async move {
            Ok::<_, AppError>((
                index,
                prepare_series_library_scan_candidate(folder, scan_hints.as_ref()).await?,
            ))
        });
    }

    let mut prepared_results = vec![None; prepare_set.len()];
    while let Some(result) = prepare_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        prepared_results[index] = Some(candidate);
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

pub(crate) fn select_movie_metadata_from_batch_results(
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> AppResult<Option<MetadataSearchItem>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(None);
    }

    for search_candidate in &candidate.search_candidates {
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            search_candidate,
            candidate.year_hint,
            candidate.identity_hint.as_ref(),
        )
        .ok_or_else(|| {
            AppError::Repository("movie metadata lookup key unexpectedly missing".to_string())
        })?;
        let results_for_query = batch_search_results.get(&key).ok_or_else(|| {
            AppError::Repository(format!(
                "movie metadata lookup result missing for query '{}'",
                search_candidate
            ))
        })?;

        if let Some(best) = select_safe_batch_match(results_for_query.as_ref()) {
            return Ok(with_metadata_match_fallback_name(
                best,
                movie_candidate_fallback_title(candidate),
            ));
        }
    }

    Ok(None)
}

pub(crate) fn select_series_metadata_from_batch_results(
    candidate: &PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> AppResult<Option<MetadataSearchItem>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(None);
    }

    for search_candidate in &candidate.search_candidates {
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_SERIES,
            search_candidate,
            candidate.year_hint,
            candidate.identity_hint.as_ref(),
        )
        .ok_or_else(|| {
            AppError::Repository("series metadata lookup key unexpectedly missing".to_string())
        })?;
        let results_for_query = batch_search_results.get(&key).ok_or_else(|| {
            AppError::Repository(format!(
                "series metadata lookup result missing for query '{}'",
                search_candidate
            ))
        })?;

        if let Some(best) = select_safe_batch_match(results_for_query.as_ref()) {
            return Ok(with_metadata_match_fallback_name(
                best,
                series_candidate_fallback_title(candidate),
            ));
        }
    }

    Ok(None)
}

fn select_safe_batch_match(results: &[MetadataSearchItem]) -> Option<MetadataSearchItem> {
    results.first().filter(|item| item.auto_match_safe).cloned()
}

fn with_metadata_match_fallback_name(
    mut item: MetadataSearchItem,
    fallback: Option<&str>,
) -> Option<MetadataSearchItem> {
    if item.name.trim().is_empty() {
        item.name = fallback?.trim().to_string();
    }
    (!item.name.trim().is_empty()).then_some(item)
}

fn movie_candidate_fallback_title(candidate: &PreparedMovieLibraryScanCandidate) -> Option<&str> {
    candidate
        .identity_hint
        .as_ref()
        .and_then(|hint| hint.title.as_deref())
        .or_else(|| {
            candidate
                .nfo_meta
                .as_ref()
                .and_then(|nfo| nfo.title.as_deref())
        })
        .or_else(|| non_empty_str(candidate.query.as_str()))
        .or_else(|| non_empty_str(candidate.file.display_name.as_str()))
}

fn series_candidate_fallback_title(candidate: &PreparedSeriesLibraryScanCandidate) -> Option<&str> {
    candidate
        .identity_hint
        .as_ref()
        .and_then(|hint| hint.title.as_deref())
        .or_else(|| {
            candidate
                .nfo_meta
                .as_ref()
                .and_then(|nfo| nfo.title.as_deref())
        })
        .or_else(|| non_empty_str(candidate.query.as_str()))
        .or_else(|| candidate.folder_name.as_deref().and_then(non_empty_str))
}

fn non_empty_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
async fn prepare_movie_library_scan_candidate(
    file: LibraryFile,
    library_path: String,
) -> AppResult<PreparedMovieLibraryScanCandidate> {
    build_prepared_movie_library_scan_candidate(file.clone(), false, vec![file], library_path, None)
        .await
}

pub(crate) async fn prepare_movie_library_scan_entries(
    library_scanner: Arc<dyn LibraryScanner>,
    entries: &[MovieTopLevelEntry],
    library_path: &str,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<Vec<PreparedMovieLibraryScanCandidate>> {
    let mut prepared_results = vec![None; entries.len()];

    for (chunk_index, entry_chunk) in entries.chunks(MOVIE_ENTRY_PREP_CONCURRENCY).enumerate() {
        let mut prepare_set = tokio::task::JoinSet::new();
        let chunk_start = chunk_index * MOVIE_ENTRY_PREP_CONCURRENCY;

        for (offset, entry) in entry_chunk.iter().cloned().enumerate() {
            let index = chunk_start + offset;
            let library_path = library_path.to_string();
            let library_scanner = library_scanner.clone();
            let scan_hints = scan_hints.cloned();
            prepare_set.spawn(async move {
                Ok::<_, AppError>((
                    index,
                    prepare_movie_library_scan_entry(
                        library_scanner,
                        entry,
                        library_path,
                        scan_hints.as_ref(),
                    )
                    .await?,
                ))
            });
        }

        while let Some(result) = prepare_set.join_next().await {
            let (index, candidate) =
                result.map_err(|error| AppError::Repository(error.to_string()))??;
            prepared_results[index] = Some(candidate);
        }
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

async fn prepare_movie_library_scan_entry(
    library_scanner: Arc<dyn LibraryScanner>,
    entry: MovieTopLevelEntry,
    library_path: String,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<PreparedMovieLibraryScanCandidate> {
    match prepare_movie_candidate_evidence(library_scanner, entry, library_path, scan_hints).await?
    {
        MovieCandidateEvidence::Candidate {
            mut candidate,
            inline_inventory,
        } => {
            if let Some(files) = inline_inventory {
                candidate.discovered_files = files;
            }
            Ok(*candidate)
        }
    }
}

/// Evidence-only movie candidate preparation for the streaming scan pipeline.
///
/// Reads only title-level signals: the direct children of the entry (one
/// shallow listing), sidecars (`movie.nfo`, same-stem NFO, `.plexmatch`), and
/// filename/folder-name hints. The recursive inventory walk runs separately;
/// candidates produced here carry an empty `discovered_files` list and the
/// pipeline attaches the real file list at match/inventory rendezvous.
///
/// Folders without direct-child video still produce title evidence from the
/// folder name and sidecars. Recursive inventory runs downstream so empty
/// title folders can be adopted and nested layouts can still count files.
pub(crate) enum MovieCandidateEvidence {
    Candidate {
        candidate: Box<PreparedMovieLibraryScanCandidate>,
        /// Present when evidence gathering already produced the exact
        /// inventory (top-level movie files and root-level movie files).
        inline_inventory: Option<Vec<LibraryFile>>,
    },
}

pub(crate) async fn prepare_movie_candidate_evidence(
    library_scanner: Arc<dyn LibraryScanner>,
    entry: MovieTopLevelEntry,
    library_path: String,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<MovieCandidateEvidence> {
    let entry_path = path_to_stored_string(&entry.path);

    if !entry.is_dir {
        let file = LibraryFile {
            path: entry_path.clone(),
            display_name: entry
                .path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
                .trim()
                .to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        };
        let mut representative = file.clone();
        representative.nfo_path =
            matching_movie_nfo_path_async(&stored_path_to_path_buf(&file.path)).await;
        let candidate = build_prepared_movie_library_scan_candidate(
            representative,
            false,
            Vec::new(),
            library_path,
            scan_hints,
        )
        .await?;
        return Ok(MovieCandidateEvidence::Candidate {
            candidate: Box::new(candidate),
            inline_inventory: Some(vec![file]),
        });
    }

    let mut children = library_scanner
        .scan_directory_children(entry_path.as_str())
        .await?;
    children.sort_by(|left, right| left.path.cmp(&right.path));

    if children.is_empty() {
        let file = build_movie_folder_representative_file(&entry).await;
        let candidate = build_prepared_movie_library_scan_candidate(
            file,
            true,
            Vec::new(),
            library_path,
            scan_hints,
        )
        .await?;
        return Ok(MovieCandidateEvidence::Candidate {
            candidate: Box::new(candidate),
            inline_inventory: None,
        });
    }

    let file = build_movie_entry_representative_file(&entry, &children).await?;
    let candidate = build_prepared_movie_library_scan_candidate(
        file,
        false,
        Vec::new(),
        library_path,
        scan_hints,
    )
    .await?;
    Ok(MovieCandidateEvidence::Candidate {
        candidate: Box::new(candidate),
        inline_inventory: None,
    })
}

async fn build_movie_folder_representative_file(entry: &MovieTopLevelEntry) -> LibraryFile {
    let path = path_to_stored_string(&entry.path);
    let display_name = entry
        .path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone())
        .trim()
        .to_string();
    LibraryFile {
        path: path.clone(),
        display_name,
        nfo_path: directory_movie_nfo_path(&entry.path, &path).await,
        size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
    }
}

async fn build_movie_entry_representative_file(
    entry: &MovieTopLevelEntry,
    discovered_files: &[LibraryFile],
) -> AppResult<LibraryFile> {
    if !entry.is_dir {
        let mut file = discovered_files
            .first()
            .cloned()
            .ok_or_else(|| AppError::Repository("movie entry unexpectedly had no files".into()))?;
        file.nfo_path = matching_movie_nfo_path_async(&stored_path_to_path_buf(&file.path)).await;
        return Ok(file);
    }

    let primary_candidate = detect_primary_movie_entry_file(&entry.path, discovered_files).await?;
    let mut file = if let Some(primary_path) = primary_candidate.as_ref() {
        discovered_files
            .iter()
            .find(|candidate| &candidate.path == primary_path)
            .cloned()
            .unwrap_or_else(|| discovered_files[0].clone())
    } else {
        discovered_files[0].clone()
    };

    file.nfo_path = directory_movie_nfo_path(&entry.path, &file.path).await;
    Ok(file)
}

async fn same_stem_movie_nfo_path(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if tokio::fs::metadata(&same_stem)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&same_stem));
    }

    None
}

async fn directory_movie_nfo_path(entry_path: &Path, file_path: &str) -> Option<String> {
    let file_path_buf = stored_path_to_path_buf(file_path);
    if let Some(nfo_path) = same_stem_movie_nfo_path(&file_path_buf).await {
        return Some(nfo_path);
    }

    // Associate the folder-level movie.nfo with the entry's representative file
    // unconditionally, matching the background-refresh path
    // (matching_movie_nfo_path). The previous `primary_candidate == file_path`
    // gate silently dropped the NFO (and its external ids) whenever the
    // representative fell back to discovered_files[0] or primary detection
    // returned None, leaving titles to a slow, id-less text search.
    let movie_nfo = entry_path.join("movie.nfo");
    if tokio::fs::metadata(&movie_nfo)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&movie_nfo));
    }

    None
}

async fn detect_primary_movie_entry_file(
    entry_path: &Path,
    discovered_files: &[LibraryFile],
) -> AppResult<Option<String>> {
    let immediate_files = discovered_files
        .iter()
        .filter(|file| stored_path_to_path_buf(&file.path).parent() == Some(entry_path))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();

    if immediate_files.len() == 1 {
        let path = immediate_files[0].clone();
        return Ok((!is_sample_video_candidate(&stored_path_to_path_buf(&path))).then_some(path));
    }

    if immediate_files.is_empty() {
        return Ok(None);
    }

    let mut non_sample_videos = Vec::new();
    for path in immediate_files {
        if is_sample_video_candidate(&stored_path_to_path_buf(&path)) {
            continue;
        }
        non_sample_videos.push(path);
    }

    Ok((non_sample_videos.len() == 1).then(|| non_sample_videos[0].clone()))
}

fn is_sample_video_candidate(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();
    stem.contains("sample")
}

async fn build_prepared_movie_library_scan_candidate(
    file: LibraryFile,
    representative_is_directory: bool,
    discovered_files: Vec<LibraryFile>,
    library_path: String,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<PreparedMovieLibraryScanCandidate> {
    let leaf_key = crate::library_scan_file_leaf_key(&file.path);
    let full_path_key = crate::library_scan_file_full_path_key(&file.path);
    if let Some(identity_hint) = external_import_identity_hint_for_scan_path(
        scan_hints,
        LibraryScanHintFacet::Movie,
        leaf_key.as_deref(),
        full_path_key.as_deref(),
    ) {
        return Ok(PreparedMovieLibraryScanCandidate {
            file,
            representative_is_directory,
            discovered_files,
            parsed_release: crate::ParsedReleaseMetadata::default(),
            nfo_meta: None,
            identity_hint: Some(identity_hint),
            query: String::new(),
            year_hint: None,
            query_variants: Vec::new(),
            search_candidates: vec![String::new()],
            metadata_lookup_attempted: true,
        });
    }

    let nfo_meta = read_valid_movie_nfo_metadata(file.nfo_path.as_deref()).await;
    let file_path = stored_path_to_path_buf(&file.path);
    let library_root = stored_path_to_path_buf(&library_path);
    let filename_parse = parse_library_filename(&LibraryFilenameParseInput::title_only(
        file_path.as_path(),
        Some(library_root.as_path()),
    ));
    let parsed_release = filename_parse.parsed_release.clone();
    let query_evidence = filename_parse.query_evidence;
    let query_variants = query_evidence.queries.clone();
    let extracted_year_hint = query_evidence.year;
    let fallback_query = query_variants.first().cloned().unwrap_or_default();
    let local_identity_hint = select_metadata_identity_hint(MetadataIdentityHintSelection {
        library_scan_hint: None,
        nfo_meta: nfo_meta.as_ref(),
        plexmatch_meta: None,
        file_walk: query_evidence.file_walk.as_ref(),
        folder_walk: query_evidence.folder_walk.as_ref(),
        parsed: &parsed_release,
        fallback_query: &fallback_query,
        fallback_year: extracted_year_hint,
    });
    let identity_hint = if local_identity_hint
        .as_ref()
        .is_some_and(MetadataIdentityHint::has_external_ids)
    {
        local_identity_hint
    } else {
        external_import_identity_hint_for_scan_path(
            scan_hints,
            LibraryScanHintFacet::Movie,
            leaf_key.as_deref(),
            full_path_key.as_deref(),
        )
        .or(local_identity_hint)
    };

    let external_import_identity_only = identity_hint
        .as_ref()
        .is_some_and(|hint| hint.is_external_import_hint() && hint.has_external_ids());
    let query = if external_import_identity_only {
        String::new()
    } else {
        identity_hint
            .as_ref()
            .and_then(|hint| hint.title.clone())
            .unwrap_or_else(|| fallback_query.clone())
            .trim()
            .to_string()
    };
    let year_hint = if external_import_identity_only {
        None
    } else {
        identity_hint
            .as_ref()
            .and_then(|hint| hint.year)
            .or(extracted_year_hint)
    };

    let mut search_candidates = Vec::new();
    let has_external_ids = identity_hint
        .as_ref()
        .is_some_and(MetadataIdentityHint::has_external_ids);
    let metadata_lookup_attempted = has_external_ids || !query.trim().is_empty();

    if metadata_lookup_attempted {
        // Lead with an empty-query, id-anchored lookup whenever the hint carries
        // external ids (NFO/plexmatch/arr-import). SMG only resolves by id when
        // the query is empty. The title-text variants follow as fallback, and
        // selection takes the first auto-match-safe hit in this order, so a real
        // id resolves confidently without depending on SMG's text ranking. An
        // arr-import hint stays id-only (its parsed-filename title is noise).
        if has_external_ids {
            search_candidates.push(String::new());
        }
        if !external_import_identity_only {
            let raw_queries = query_variants
                .iter()
                .cloned()
                .chain(std::iter::once(query.clone()))
                .collect::<Vec<_>>();
            search_candidates.extend(expand_search_candidates(&raw_queries));
        }
    }

    Ok(PreparedMovieLibraryScanCandidate {
        file,
        representative_is_directory,
        discovered_files,
        parsed_release,
        nfo_meta,
        identity_hint,
        query,
        year_hint,
        query_variants,
        search_candidates,
        metadata_lookup_attempted,
    })
}

pub(crate) async fn prepare_series_library_scan_candidate(
    folder: PathBuf,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<PreparedSeriesLibraryScanCandidate> {
    let folder_name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());

    let Some(folder_name_value) = folder_name.clone() else {
        return Ok(PreparedSeriesLibraryScanCandidate {
            folder_path: folder,
            folder_name: None,
            nfo_meta: None,
            identity_hint: None,
            query: String::new(),
            year_hint: None,
            search_candidates: Vec::new(),
            title_match_candidates: Vec::new(),
            metadata_lookup_attempted: false,
        });
    };

    let folder_path = folder.to_string_lossy();
    let folder_key = crate::library_scan_folder_leaf_key(folder_path.as_ref());
    let full_path_key = crate::library_scan_folder_full_path_key(folder_path.as_ref());
    if let Some(identity_hint) = external_import_identity_hint_for_scan_path(
        scan_hints,
        LibraryScanHintFacet::Series,
        folder_key.as_deref(),
        full_path_key.as_deref(),
    ) {
        return Ok(PreparedSeriesLibraryScanCandidate {
            folder_path: folder,
            folder_name,
            nfo_meta: None,
            identity_hint: Some(identity_hint),
            query: String::new(),
            year_hint: None,
            search_candidates: vec![String::new()],
            title_match_candidates: Vec::new(),
            metadata_lookup_attempted: true,
        });
    }

    let nfo_meta = read_tvshow_nfo_metadata(folder.clone()).await;
    let plexmatch_meta = read_plexmatch_metadata(Some(folder.clone())).await;
    let clean_name = normalize_folder_name(&folder_name_value);
    let (fallback_query, extracted_year_hint) = strip_year_suffix(&clean_name);
    let filename_parse = parse_library_filename(&LibraryFilenameParseInput::title_only(
        folder.as_path(),
        folder.parent(),
    ));
    let folder_walk = filename_parse.query_evidence.file_walk.clone();
    let fallback_query = folder_walk
        .as_ref()
        .and_then(|walk| walk.title.clone())
        .unwrap_or(fallback_query)
        .trim()
        .to_string();
    let extracted_year_hint = folder_walk
        .as_ref()
        .and_then(|walk| walk.year)
        .or(extracted_year_hint);
    let parsed_release = filename_parse.parsed_release;
    let local_identity_hint = select_metadata_identity_hint(MetadataIdentityHintSelection {
        library_scan_hint: None,
        nfo_meta: nfo_meta.as_ref(),
        plexmatch_meta: plexmatch_meta.as_ref(),
        file_walk: None,
        folder_walk: folder_walk.as_ref(),
        parsed: &parsed_release,
        fallback_query: &fallback_query,
        fallback_year: extracted_year_hint,
    });
    let identity_hint = if local_identity_hint
        .as_ref()
        .is_some_and(MetadataIdentityHint::has_external_ids)
    {
        local_identity_hint
    } else {
        external_import_identity_hint_for_scan_path(
            scan_hints,
            LibraryScanHintFacet::Series,
            folder_key.as_deref(),
            full_path_key.as_deref(),
        )
        .or(local_identity_hint)
    };
    let external_import_identity_only = identity_hint
        .as_ref()
        .is_some_and(|hint| hint.is_external_import_hint() && hint.has_external_ids());
    let query = if external_import_identity_only {
        String::new()
    } else {
        identity_hint
            .as_ref()
            .and_then(|hint| hint.title.clone())
            .unwrap_or_else(|| fallback_query.clone())
            .trim()
            .to_string()
    };
    let year_hint = if external_import_identity_only {
        None
    } else {
        identity_hint
            .as_ref()
            .and_then(|hint| hint.year)
            .or(extracted_year_hint)
    };

    let has_external_ids = identity_hint
        .as_ref()
        .is_some_and(MetadataIdentityHint::has_external_ids);
    let metadata_lookup_attempted = has_external_ids || !query.is_empty();
    let (search_candidates, title_match_candidates) = if metadata_lookup_attempted {
        let raw_queries = if external_import_identity_only {
            vec![String::new()]
        } else {
            vec![query.clone()]
        };
        let mut search_candidates = Vec::new();
        // Lead with an empty-query, id-anchored lookup whenever the hint carries
        // external ids (NFO/plexmatch/arr-import). SMG only resolves by id when
        // the query is empty. Title variants follow as fallback for local hints;
        // arr-import hints stay id-only because their parsed folder title is noise.
        if has_external_ids {
            search_candidates.push(String::new());
        }
        search_candidates.extend(expand_search_candidates(&raw_queries));
        let title_match_candidates = build_title_match_candidates(&raw_queries);
        (search_candidates, title_match_candidates)
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(PreparedSeriesLibraryScanCandidate {
        folder_path: folder,
        folder_name,
        nfo_meta,
        identity_hint,
        query,
        year_hint,
        search_candidates,
        title_match_candidates,
        metadata_lookup_attempted,
    })
}

#[cfg(test)]
pub(crate) async fn preload_movie_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    files: &[LibraryFile],
    library_path: &str,
) -> AppResult<(Vec<MovieLibraryScanCandidate>, MetadataLookupBatchStats)> {
    let prepared_candidates = prepare_movie_library_scan_candidates(files, library_path).await?;
    let (batch_searches, stats) = build_movie_metadata_batch_stats(&prepared_candidates);
    let batch_search_results =
        execute_batch_metadata_searches(metadata_gateway, batch_searches, "eng", None).await?;
    let mut results = Vec::with_capacity(prepared_candidates.len());

    for candidate in prepared_candidates {
        let selected_metadata =
            select_movie_metadata_from_batch_results(&candidate, &batch_search_results)?;

        results.push(MovieLibraryScanCandidate { selected_metadata });
    }

    Ok((results, stats))
}

#[cfg(test)]
pub(crate) async fn preload_series_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    folders: &[PathBuf],
) -> AppResult<(Vec<SeriesLibraryScanCandidate>, MetadataLookupBatchStats)> {
    let prepared_candidates = prepare_series_library_scan_candidates(folders, None).await?;
    let (batch_searches, stats) = build_series_metadata_batch_stats(&prepared_candidates);
    let batch_search_results =
        execute_batch_metadata_searches(metadata_gateway, batch_searches, "eng", None).await;
    let batch_search_error = batch_search_results.as_ref().err().map(ToString::to_string);
    let batch_search_results = batch_search_results.unwrap_or_default();
    let mut results = Vec::with_capacity(prepared_candidates.len());

    for candidate in prepared_candidates {
        let (selected_metadata, metadata_lookup_error) = if candidate.metadata_lookup_attempted {
            if let Some(error) = batch_search_error.as_ref() {
                (None, Some(error.clone()))
            } else {
                (
                    select_series_metadata_from_batch_results(&candidate, &batch_search_results)?,
                    None,
                )
            }
        } else {
            (None, None)
        };

        results.push(SeriesLibraryScanCandidate {
            nfo_meta: candidate.nfo_meta,
            query: candidate.query,
            selected_metadata,
            metadata_lookup_error,
        });
    }

    Ok((results, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_discovery::extract_library_queries;
    use crate::{
        BulkMetadataResult, ExternalIdHint, LibraryFileBatchReceiver, MovieMetadata,
        MultiMetadataSearchResult, RichMetadataSearchItem, SeriesMetadata,
    };
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    type CountingSearchResults =
        Arc<Mutex<HashMap<(String, String), Result<Vec<MetadataSearchItem>, String>>>>;

    #[derive(Clone, Default)]
    struct CountingMetadataGateway {
        search_results: CountingSearchResults,
        search_calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl CountingMetadataGateway {
        fn search_key(type_hint: &str, query: &str) -> (String, String) {
            (type_hint.to_string(), query.trim().to_ascii_uppercase())
        }

        fn set_search_results(
            &self,
            type_hint: &str,
            query: &str,
            results: Vec<MetadataSearchItem>,
        ) {
            self.search_results
                .lock()
                .unwrap()
                .insert(Self::search_key(type_hint, query), Ok(results));
        }

        fn set_search_error(&self, type_hint: &str, query: &str, message: &str) {
            self.search_results
                .lock()
                .unwrap()
                .insert(Self::search_key(type_hint, query), Err(message.to_string()));
        }

        fn search_call_count(&self, type_hint: &str, query: &str) -> usize {
            let normalized_key = Self::search_key(type_hint, query);
            self.search_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|logged_key| *logged_key == &normalized_key)
                .count()
        }
    }

    #[async_trait]
    impl MetadataGateway for CountingMetadataGateway {
        async fn search_tvdb(
            &self,
            query: &str,
            type_hint: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<MetadataSearchItem>> {
            let key = Self::search_key(type_hint, query);
            self.search_calls.lock().unwrap().push(key.clone());
            match self
                .search_results
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| Ok(Vec::new()))
            {
                Ok(results) => Ok(results),
                Err(message) => Err(AppError::Repository(message)),
            }
        }

        async fn search_tvdb_batch(
            &self,
            queries: &[MetadataSearchQuery],
            _language: &str,
        ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
            let mut results = HashMap::new();

            for query in queries {
                let key = Self::search_key(&query.type_hint, &query.query);
                self.search_calls.lock().unwrap().push(key.clone());
                let value = match self
                    .search_results
                    .lock()
                    .unwrap()
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| Ok(Vec::new()))
                {
                    Ok(items) => items,
                    Err(message) => return Err(AppError::Repository(message)),
                };
                results.insert(query.clone(), value);
            }

            Ok(results)
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            panic!("unused in test")
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<MultiMetadataSearchResult> {
            panic!("unused in test")
        }

        async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            panic!("unused in test")
        }

        async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
            panic!("unused in test")
        }

        async fn get_metadata_bulk(
            &self,
            _movie_tvdb_ids: &[i64],
            _series_tvdb_ids: &[i64],
            _language: &str,
        ) -> AppResult<BulkMetadataResult> {
            panic!("unused in test")
        }
    }

    type DelayedScanResponses = Arc<Mutex<HashMap<String, (u64, Vec<LibraryFile>)>>>;

    #[derive(Clone, Default)]
    struct DelayedLibraryScanner {
        responses: DelayedScanResponses,
        scan_library_calls: Arc<std::sync::atomic::AtomicUsize>,
        scan_directory_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl DelayedLibraryScanner {
        fn set_response(&self, root: &str, delay_ms: u64, files: Vec<LibraryFile>) {
            self.responses
                .lock()
                .unwrap()
                .insert(root.to_string(), (delay_ms, files));
        }

        fn scan_library_call_count(&self) -> usize {
            self.scan_library_calls
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn scan_directory_call_count(&self) -> usize {
            self.scan_directory_calls
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[derive(Clone)]
    struct DelayedBatchMetadataGateway {
        delay: Duration,
    }

    impl DelayedBatchMetadataGateway {
        fn new(delay: Duration) -> Self {
            Self { delay }
        }
    }

    #[async_trait]
    impl MetadataGateway for DelayedBatchMetadataGateway {
        async fn search_tvdb(
            &self,
            _query: &str,
            _type_hint: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<MetadataSearchItem>> {
            panic!("unused in test")
        }

        async fn search_tvdb_batch(
            &self,
            queries: &[MetadataSearchQuery],
            _language: &str,
        ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
            tokio::time::sleep(self.delay).await;
            Ok(queries
                .iter()
                .cloned()
                .map(|query| (query, Vec::new()))
                .collect())
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            panic!("unused in test")
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<MultiMetadataSearchResult> {
            panic!("unused in test")
        }

        async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            panic!("unused in test")
        }

        async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
            panic!("unused in test")
        }

        async fn get_metadata_bulk(
            &self,
            _movie_tvdb_ids: &[i64],
            _series_tvdb_ids: &[i64],
            _language: &str,
        ) -> AppResult<BulkMetadataResult> {
            panic!("unused in test")
        }
    }

    #[async_trait]
    impl LibraryScanner for DelayedLibraryScanner {
        async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
            self.scan_library_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.scan_files(root).await
        }

        async fn scan_directory(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
            self.scan_directory_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.scan_files(root).await
        }

        async fn scan_library_batched(
            &self,
            _root: &str,
            _batch_size: usize,
        ) -> AppResult<LibraryFileBatchReceiver> {
            panic!("unused in test")
        }

        async fn scan_directory_batched(
            &self,
            _root: &str,
            _batch_size: usize,
        ) -> AppResult<LibraryFileBatchReceiver> {
            panic!("unused in test")
        }
    }

    impl DelayedLibraryScanner {
        async fn scan_files(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
            let (delay_ms, files) = self
                .responses
                .lock()
                .unwrap()
                .get(root)
                .cloned()
                .unwrap_or_default();
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Ok(files)
        }
    }

    fn build_library_file(path: &str) -> LibraryFile {
        LibraryFile {
            path: path.to_string(),
            display_name: Path::new(path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }
    }

    fn build_prepared_movie_candidate(
        search_candidates: &[&str],
    ) -> PreparedMovieLibraryScanCandidate {
        PreparedMovieLibraryScanCandidate {
            file: build_library_file("/library/Movie/Movie.mkv"),
            representative_is_directory: false,
            discovered_files: vec![build_library_file("/library/Movie/Movie.mkv")],
            parsed_release: crate::ParsedReleaseMetadata::default(),
            nfo_meta: None,
            identity_hint: None,
            query: search_candidates
                .first()
                .copied()
                .unwrap_or_default()
                .to_string(),
            year_hint: None,
            query_variants: search_candidates
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            search_candidates: search_candidates
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            metadata_lookup_attempted: !search_candidates.is_empty(),
        }
    }

    #[test]
    fn select_metadata_identity_hint_prefers_nfo_over_plexmatch_and_filename() {
        let nfo = NfoMetadata {
            imdb_id: Some("tt1234567".into()),
            title: Some("NFO Title".into()),
            year: Some(2022),
            ..Default::default()
        };
        let plexmatch = NfoMetadata {
            imdb_id: Some("tt7654321".into()),
            title: Some("Plexmatch Title".into()),
            year: Some(2021),
            ..Default::default()
        };
        let parsed = crate::ParsedReleaseMetadata {
            imdb_id: Some("tt9999999".into()),
            normalized_title: "Filename Title".into(),
            year: Some(2020),
            ..Default::default()
        };

        let hint = select_metadata_identity_hint(MetadataIdentityHintSelection {
            library_scan_hint: None,
            nfo_meta: Some(&nfo),
            plexmatch_meta: Some(&plexmatch),
            file_walk: None,
            folder_walk: None,
            parsed: &parsed,
            fallback_query: "Filename Title",
            fallback_year: Some(2020),
        })
        .expect("identity hint");

        assert_eq!(hint.source, MetadataIdentitySource::Nfo);
        assert_eq!(hint.imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(hint.title.as_deref(), Some("NFO Title"));
        assert_eq!(hint.year, Some(2022));
    }

    #[test]
    fn select_metadata_identity_hint_falls_through_empty_nfo_to_plexmatch() {
        let empty_nfo = NfoMetadata::default();
        let plexmatch = NfoMetadata {
            tmdb_id: Some("438631".into()),
            title: Some("Plexmatch Title".into()),
            ..Default::default()
        };
        let parsed = crate::ParsedReleaseMetadata {
            normalized_title: "Filename Title".into(),
            ..Default::default()
        };

        let hint = select_metadata_identity_hint(MetadataIdentityHintSelection {
            library_scan_hint: None,
            nfo_meta: Some(&empty_nfo),
            plexmatch_meta: Some(&plexmatch),
            file_walk: None,
            folder_walk: None,
            parsed: &parsed,
            fallback_query: "Filename Title",
            fallback_year: None,
        })
        .expect("identity hint");

        assert_eq!(hint.source, MetadataIdentitySource::Plexmatch);
        assert_eq!(hint.tmdb_id.as_deref(), Some("438631"));
        assert_eq!(hint.title.as_deref(), Some("Plexmatch Title"));
    }

    #[test]
    fn select_metadata_identity_hint_prefers_arr_hint_over_nfo_and_filename() {
        let scan_hint = LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportRadarr,
            facet: LibraryScanHintFacet::Movie,
            path_key: path_to_stored_string(Path::new("/movies/The Lantern Supremacy (2004)")),
            full_path_key: None,
            ids: vec![ExternalIdHint {
                provider: ExternalIdProvider::Tmdb,
                value: "2502".to_string(),
            }],
        };
        let nfo = NfoMetadata {
            tvdb_id: Some("2502".into()),
            title: Some("Pelton".into()),
            ..Default::default()
        };
        let parsed = crate::ParsedReleaseMetadata {
            normalized_title: "Pelton".into(),
            year: Some(1970),
            ..Default::default()
        };

        let hint = select_metadata_identity_hint(MetadataIdentityHintSelection {
            library_scan_hint: Some(&scan_hint),
            nfo_meta: Some(&nfo),
            plexmatch_meta: None,
            file_walk: None,
            folder_walk: None,
            parsed: &parsed,
            fallback_query: "Pelton",
            fallback_year: Some(1970),
        })
        .expect("identity hint");

        assert_eq!(hint.source, MetadataIdentitySource::ExternalImportRadarr);
        assert_eq!(hint.tmdb_id.as_deref(), Some("2502"));
        assert_eq!(hint.tvdb_id, None);
        assert_eq!(hint.title, None);
    }

    #[test]
    fn select_safe_batch_match_trusts_smg_auto_match_safe() {
        let pelton_tvdb_signal = MetadataSearchItem {
            tvdb_id: "2502".to_string(),
            smg_id: None,
            primary_source: None,
            external_ids: vec![],
            name: "Pelton".to_string(),
            year: Some(1970),
            auto_match_safe: true,
            auto_match_signals: vec!["external_id:tvdb".to_string()],
        };

        assert_eq!(
            select_safe_batch_match(&[pelton_tvdb_signal]).map(|item| item.name),
            Some("Pelton".to_string())
        );
    }

    #[tokio::test]
    async fn arr_hint_only_movie_candidate_uses_id_only_batch_search() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nfo_path = tempdir.path().join("movie.nfo");
        std::fs::write(
            &nfo_path,
            r#"<movie><title>Noisy NFO Title</title><year>1901</year><tvdbid>1</tvdbid></movie>"#,
        )
        .expect("write misleading movie nfo");
        let file_path = path_to_stored_string(Path::new("/scryer/Pelton (1970)/Pelton.1970.mkv"));
        let arr_file_path = r"D:\Movies\Pelton (1970)\Pelton.1970.mkv";
        let file = LibraryFile {
            path: file_path.clone(),
            display_name: "Pelton.1970".to_string(),
            nfo_path: Some(path_to_stored_string(&nfo_path)),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        };
        let mut scan_hints = LibraryScanHintSet::new();
        scan_hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportRadarr,
            facet: LibraryScanHintFacet::Movie,
            path_key: crate::library_scan_file_leaf_key(arr_file_path).expect("leaf key"),
            full_path_key: None,
            ids: vec![ExternalIdHint {
                provider: ExternalIdProvider::Tmdb,
                value: "2502".to_string(),
            }],
        });

        let candidate = build_prepared_movie_library_scan_candidate(
            file.clone(),
            false,
            vec![file],
            path_to_stored_string(Path::new("/movies")),
            Some(&scan_hints),
        )
        .await
        .expect("candidate");

        assert!(candidate.metadata_lookup_attempted);
        assert_eq!(candidate.query, "");
        assert_eq!(candidate.year_hint, None);
        assert!(candidate.nfo_meta.is_none());
        assert!(candidate.query_variants.is_empty());
        assert_eq!(candidate.search_candidates, vec![String::new()]);
        assert_eq!(
            candidate
                .identity_hint
                .as_ref()
                .and_then(|hint| hint.tmdb_id.as_deref()),
            Some("2502")
        );

        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("id-only key");
        let mut results = MetadataSearchResults::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "2502".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "The Lantern Supremacy".to_string(),
                year: Some(2004),
                auto_match_safe: true,
                auto_match_signals: vec!["external_id:tmdb".to_string()],
            }]),
        );

        assert_eq!(
            select_movie_metadata_from_batch_results(&candidate, &results)
                .expect("metadata selection")
                .map(|item| item.name),
            Some("The Lantern Supremacy".to_string())
        );
    }

    #[tokio::test]
    async fn arr_series_folder_hint_matches_leaf_folder_across_roots() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let folder = tempdir.path().join("Fathomline (2021)");
        std::fs::create_dir_all(&folder).expect("create folder");
        std::fs::write(
            folder.join("tvshow.nfo"),
            r#"<tvshow><title>Noisy NFO Title</title><year>1901</year><tvdbid>1</tvdbid></tvshow>"#,
        )
        .expect("write misleading tvshow nfo");
        std::fs::write(
            folder.join(".plexmatch"),
            "title: Noisy Plexmatch Title\ntvdbid: 2\n",
        )
        .expect("write misleading plexmatch");
        let mut scan_hints = LibraryScanHintSet::new();
        scan_hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportSonarr,
            facet: LibraryScanHintFacet::Series,
            path_key: crate::library_scan_folder_leaf_key(r"D:\Series\Fathomline (2021)")
                .expect("leaf key"),
            full_path_key: None,
            ids: vec![ExternalIdHint {
                provider: ExternalIdProvider::Tvdb,
                value: "366972".to_string(),
            }],
        });

        let candidate = prepare_series_library_scan_candidate(folder, Some(&scan_hints))
            .await
            .expect("candidate");

        assert_eq!(candidate.query, "");
        assert_eq!(candidate.year_hint, None);
        assert!(candidate.nfo_meta.is_none());
        assert_eq!(candidate.search_candidates, vec![String::new()]);
        assert!(candidate.title_match_candidates.is_empty());
        assert_eq!(
            candidate
                .identity_hint
                .as_ref()
                .and_then(|hint| hint.tvdb_id.as_deref()),
            Some("366972")
        );
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_accepts_url_only_id_sidecar() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nfo_path = tempdir.path().join("movie.nfo");
        std::fs::write(&nfo_path, "https://www.imdb.com/title/tt1234567/").expect("write nfo");

        let meta = read_valid_movie_nfo_metadata(Some(&path_to_stored_string(&nfo_path)))
            .await
            .expect("URL-only NFO should be usable metadata");

        assert_eq!(meta.imdb_id.as_deref(), Some("tt1234567"));
    }

    #[tokio::test]
    async fn prepare_movie_candidate_evidence_uses_empty_folder_sidecar_without_recursive_scan() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().join("movies");
        let folder = root.join("scan-item-folder (2024)");
        std::fs::create_dir_all(&folder).expect("create movie folder");
        let nfo_path = folder.join("movie.nfo");
        std::fs::write(
            &nfo_path,
            r#"<movie><title>Sidecar Item</title><tvdbid>12345</tvdbid></movie>"#,
        )
        .expect("write movie nfo");

        let scanner = DelayedLibraryScanner::default();
        scanner.set_response(path_to_stored_string(&folder).as_str(), 0, Vec::new());

        let evidence = prepare_movie_candidate_evidence(
            Arc::new(scanner.clone()),
            MovieTopLevelEntry {
                path: folder.clone(),
                is_dir: true,
            },
            path_to_stored_string(&root),
            None,
        )
        .await
        .expect("prepare empty folder movie evidence");

        let MovieCandidateEvidence::Candidate {
            candidate,
            inline_inventory,
        } = evidence;
        assert!(inline_inventory.is_none());
        assert_eq!(scanner.scan_directory_call_count(), 1);
        assert_eq!(candidate.file.path, path_to_stored_string(&folder));
        assert_eq!(
            candidate.file.nfo_path.as_deref(),
            Some(path_to_stored_string(&nfo_path).as_str())
        );

        let keys = movie_candidate_batch_search_keys(&candidate).expect("movie search keys");
        assert_eq!(keys.first().map(|key| key.query.as_str()), Some(""));
        assert_eq!(
            keys.first().and_then(|key| key.tvdb_id.as_deref()),
            Some("12345")
        );
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_rejects_tvshow_root() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nfo_path = tempdir.path().join("movie.nfo");
        std::fs::write(
            &nfo_path,
            r#"<tvshow><title>Wrong Root</title><tvdbid>12345</tvdbid></tvshow>"#,
        )
        .expect("write nfo");

        let meta = read_valid_movie_nfo_metadata(Some(&path_to_stored_string(&nfo_path))).await;

        assert!(meta.is_none());
    }

    fn nightfall_tvshow_nfo_fixture() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<tvshow>
  <plot>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</plot>
  <outline>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</outline>
  <lockdata>false</lockdata>
  <dateadded>2026-04-21 04:22:41</dateadded>
  <title>Nightfall!!</title>
  <originaltitle>Nightfall!! Kage no Requiem</originaltitle>
  <trailer>plugin://plugin.video.youtube/play/?video_id=_Iqc-dG8peA</trailer>
  <trailer>plugin://plugin.video.youtube/play/?video_id=Vt4zSf3CfRA</trailer>
  <rating>5</rating>
  <year>2022</year>
  <mpaa>TV-MA</mpaa>
  <collectionnumber>156898</collectionnumber>
  <imdb_id>tt17736234</imdb_id>
  <tmdbid>156898</tmdbid>
  <premiered>1992-08-25</premiered>
  <releasedate>1992-08-25</releasedate>
  <enddate>1993-06-25</enddate>
  <runtime>25</runtime>
  <genre>Anime</genre>
  <genre>magic</genre>
  <genre>stereotypes</genre>
  <genre>super power</genre>
  <genre>violence</genre>
  <studio />
  <studio>Netflix</studio>
  <tag>anime</tag>
  <tag>based on manga</tag>
  <tag>combat</tag>
  <tag>dark fantasy</tag>
  <tag>ecchi</tag>
  <tag>heavy metal</tag>
  <tag>magic</tag>
  <tag>original net animation (ona)</tag>
  <tag>remake</tag>
  <tag>seinen</tag>
  <anidbid>10</anidbid>
  <tvdbid>415677</tvdbid>
  <tvdbslugid>nightfall-2022</tvdbslugid>
  <art>
    <poster>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/poster.jpg</poster>
    <fanart>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/backdrop.jpg</fanart>
  </art>
  <id>415677</id>
  <episodeguide>
    <url cache="415677.xml">http://www.thetvdb.com/api/1D62F2F90030C444/series/415677/all/en.zip</url>
  </episodeguide>
  <season>-1</season>
  <episode>-1</episode>
  <status>Ended</status>
</tvshow>"#
    }

    #[test]
    fn extract_library_queries_uses_movie_title_variants_for_root_files() {
        let (queries, year) = extract_library_queries(
            "/library/Mon.Phare.A.K.A.My.Lighthouse.2020.1080p.BluRay.mkv",
            "/library",
        );

        assert_eq!(year, Some(2020));
        assert_eq!(
            queries,
            vec![
                "MON PHARE AKA MY LIGHTHOUSE".to_string(),
                "MON PHARE".to_string(),
                "MY LIGHTHOUSE".to_string()
            ]
        );
    }

    #[test]
    fn extract_library_queries_prefers_simple_file_title_walk() {
        let (queries, year) = extract_library_queries(
            "/Volumes/Media/Movies/Feranki A Sand Max Saga (2024)/Feranki A Sand Max Saga (2024) Remux-2160p.mkv",
            "/Volumes/Media/Movies",
        );

        assert_eq!(
            queries.first().map(String::as_str),
            Some("Feranki A Sand Max Saga")
        );
        assert!(queries.iter().any(|query| query == "FERANKI A SAND"));
        assert_eq!(year, Some(2024));
    }

    #[test]
    fn extract_library_queries_keeps_release_style_names_on_release_parser_path() {
        let (queries, year) = extract_library_queries(
            "/library/Example.Movie.2024.MAX.WEB-DL.2160p-GRP.mkv",
            "/library",
        );

        assert_eq!(queries.first().map(String::as_str), Some("EXAMPLE MOVIE"));
        assert!(!queries.iter().any(|query| query == "Example Movie"));
        assert_eq!(year, Some(2024));
    }

    #[tokio::test]
    async fn prepare_series_folder_candidate_uses_simple_title_walk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("Fathomline (2021)");
        std::fs::create_dir_all(&folder).expect("create series folder");

        let candidate = prepare_series_library_scan_candidate(folder, None)
            .await
            .expect("prepared candidate");

        assert_eq!(candidate.query, "Fathomline");
        assert_eq!(candidate.year_hint, Some(2021));
        assert_eq!(
            candidate.search_candidates.first().map(String::as_str),
            Some("Fathomline")
        );
    }

    #[tokio::test]
    async fn prepare_series_folder_candidate_uses_title_and_year_hint_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("Rascal!! (2022)");
        std::fs::create_dir_all(&folder).expect("create series folder");

        let candidate = prepare_series_library_scan_candidate(folder, None)
            .await
            .expect("prepared candidate");

        assert_eq!(candidate.query, "Rascal!!");
        assert_eq!(candidate.year_hint, Some(2022));
        assert_eq!(
            candidate.search_candidates.first().map(String::as_str),
            Some("Rascal!!")
        );
        assert!(
            !candidate
                .search_candidates
                .iter()
                .any(|query| query == "Rascal!! (2022)")
        );

        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_SERIES,
            "Rascal!!",
            Some(2022),
            candidate.identity_hint.as_ref(),
        )
        .expect("series key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "415677".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Rascal!! (2022)".into(),
                year: Some(2022),
                auto_match_safe: true,
                auto_match_signals: vec![
                    "exact_title_year_hint".into(),
                    "exact_year".into(),
                    "score_gap_clear".into(),
                ],
            }]),
        );

        let selected = select_series_metadata_from_batch_results(&candidate, &results)
            .expect("series batch selection")
            .expect("safe year-hint match");
        assert_eq!(selected.tvdb_id, "415677");
    }

    #[test]
    fn extract_library_queries_uses_parent_folder_when_filename_is_placeholder() {
        let (queries, year) =
            extract_library_queries("/library/My Lighthouse (2020)/movie.mkv", "/library");

        assert_eq!(queries, vec!["MY LIGHTHOUSE".to_string()]);
        assert_eq!(year, Some(2020));
    }

    #[test]
    fn extract_library_queries_uses_parent_folder_when_filename_is_obfuscated() {
        let (queries, year) = extract_library_queries(
            "/library/Harbor.Pilot.And.The.Silent.Harbors.Part1.2010.720p.BluRay.DTS.x264-LEGION-Obfuscated/aUUKqrO833LbSr7VlByumnR24y7ULADpVJ7K0FTnPhPMqpp0KIIaLSLYXJmyjm.mkv",
            "/library",
        );

        assert_eq!(
            queries,
            vec!["HARBOR PILOT AND THE SILENT HARBORS PART 1".to_string()]
        );
        assert_eq!(year, Some(2010));
    }

    #[test]
    fn extract_library_queries_keeps_raw_parent_folder_title_when_parser_is_lossy() {
        let (queries, year) = extract_library_queries(
            "/library/The Harbor King 1½ (2004)/The Harbor King 1½ (2004) Bluray-1080p.mkv",
            "/library",
        );

        assert_eq!(year, Some(2004));
        assert!(queries.iter().any(|query| query == "The Harbor King 1½"));
    }

    #[test]
    fn extract_library_queries_keeps_raw_human_folder_title_without_explicit_year_suffix() {
        let (queries, year) = extract_library_queries(
            "/library/The Harbor King 1½/The Harbor King 1½ Bluray-1080p.mkv",
            "/library",
        );

        assert_eq!(year, None);
        assert!(queries.iter().any(|query| query == "The Harbor King 1½"));
    }

    #[test]
    fn extract_library_queries_keeps_raw_parent_folder_title_when_context_parse_supplies_year() {
        let (queries, year) = extract_library_queries(
            "/library/The Harbor King 1½ 2004/The Harbor King 1½ Bluray-1080p.mkv",
            "/library",
        );

        assert_eq!(year, Some(2004));
        assert!(queries.iter().any(|query| query == "The Harbor King 1½"));
    }

    #[test]
    fn extract_library_queries_prefers_release_year_over_stale_folder_year() {
        let (queries, year) = extract_library_queries(
            "/library/Glass Harbor (2020)/Glass.Harbor.2021.2160p.BluRay.REMUX.HEVC.DTS-HD.MA.TrueHD.7.1.Atmos-FGT.mkv",
            "/library",
        );

        assert_eq!(queries, vec!["GLASS HARBOR".to_string()]);
        assert_eq!(year, Some(2021));
    }

    #[test]
    fn extract_library_queries_prefers_filename_over_parent_folder_for_nested_movie() {
        let (queries, year) = extract_library_queries(
            "/library/Glass Harbor (2020)/Glass.Harbor.Part.Two.2024.2160p.WEB-DL.H265-GRP.mkv",
            "/library",
        );

        assert_eq!(
            queries,
            vec!["GLASS HARBOR TWO".to_string(), "GLASS HARBOR".to_string()]
        );
        assert_eq!(year, Some(2024));
    }

    #[test]
    fn extract_library_queries_keeps_full_circuit_breakers_crash_the_grid_title() {
        let (queries, year) = extract_library_queries(
            "/library/Circuit Breakers Crash the Grid 2 (2018)/Circuit Breakers Crash the Grid 2.mkv",
            "/library",
        );

        assert_eq!(
            queries,
            vec!["CIRCUIT BREAKERS CRASH THE GRID 2".to_string()]
        );
        assert_eq!(year, Some(2018));
    }

    #[cfg(unix)]
    #[test]
    fn extract_library_queries_uses_lossy_non_utf8_stem() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(
            b"/library/Glass.Harbor.\xFF.2021.2160p.WEB-DL.mkv",
        ));
        let stored_path = path_to_stored_string(path);
        let (queries, year) = extract_library_queries(&stored_path, "/library");

        assert!(!queries.is_empty());
        assert!(queries.iter().any(|query| query.contains("GLASS HARBOR")));
        assert_eq!(year, Some(2021));
    }

    #[test]
    fn build_title_match_candidates_deduplicates_canonical_queries() {
        let raw_candidates = vec![
            "Glass Harbor".to_string(),
            "Glass.Harbor".to_string(),
            "LANTERN, The".to_string(),
        ];

        assert_eq!(
            build_title_match_candidates(&raw_candidates),
            vec!["glass harbor".to_string(), "the lantern".to_string()]
        );
    }

    #[test]
    fn select_movie_metadata_from_batch_results_uses_smg_auto_match_safe_signal() {
        let candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        let key = BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Glass Harbor", None, None)
            .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("safe auto-match");

        assert_eq!(selected.tvdb_id, "movie-1");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_rejects_unsafe_top_result() {
        let candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        let key = BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Glass Harbor", None, None)
            .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: false,
                auto_match_signals: vec!["exact_title".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection");

        assert!(selected.is_none());
    }

    #[test]
    fn select_movie_metadata_from_batch_results_trusts_smg_safe_with_identity_hint() {
        let mut candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Nfo,
            imdb_id: Some("tt1234567".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Glass Harbor".into()),
            year: Some(2021),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "Glass Harbor",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("SMG-safe auto-match");

        assert_eq!(selected.tvdb_id, "movie-1");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_trusts_smg_safe_provider_signal() {
        let mut candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Nfo,
            imdb_id: Some("tt1234567".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Glass Harbor".into()),
            year: Some(2021),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "Glass Harbor",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec![
                    "external_id:tvdb".into(),
                    "exact_title".into(),
                    "exact_year".into(),
                ],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("SMG-safe auto-match");

        assert_eq!(selected.tvdb_id, "movie-1");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_trusts_smg_safe_without_external_signal() {
        let mut candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Filename,
            imdb_id: Some("tt1234567".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Glass Harbor".into()),
            year: Some(2021),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "Glass Harbor",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("SMG-safe auto-match");

        assert_eq!(selected.tvdb_id, "movie-1");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_accepts_external_signal_for_id_hint() {
        let mut candidate = build_prepared_movie_candidate(&["Glass Harbor"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Nfo,
            imdb_id: Some("tt1234567".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Glass Harbor".into()),
            year: Some(2021),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "Glass Harbor",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec!["external_id:imdb".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("ID-backed safe auto-match");

        assert_eq!(selected.tvdb_id, "movie-1");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_trusts_smg_safe_with_conflicting_local_evidence() {
        let mut candidate = build_prepared_movie_candidate(&["The Lantern Supremacy"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Nfo,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: Some("2502".into()),
            title: Some("The Lantern Supremacy".into()),
            year: Some(2004),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "The Lantern Supremacy",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "2502".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Pelton".into(),
                year: Some(1970),
                auto_match_safe: true,
                auto_match_signals: vec!["external_id:tvdb".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("SMG-safe auto-match");

        assert_eq!(selected.tvdb_id, "2502");
    }

    #[test]
    fn select_movie_metadata_from_batch_results_accepts_external_id_with_title_nuance() {
        let mut candidate = build_prepared_movie_candidate(&["Feranki A Sand Kettle Saga"]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Filename,
            imdb_id: Some("tt12037194".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Feranki A Sand Kettle Saga".into()),
            year: Some(2024),
        });
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "Feranki A Sand Kettle Saga",
            None,
            candidate.identity_hint.as_ref(),
        )
        .expect("metadata search key");
        let mut results = HashMap::new();
        results.insert(
            key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "157390".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Feranki: A Sand Kettle Saga".into(),
                year: Some(2024),
                auto_match_safe: true,
                auto_match_signals: vec!["external_id:imdb".into()],
            }]),
        );

        let selected = select_movie_metadata_from_batch_results(&candidate, &results)
            .expect("movie batch selection")
            .expect("SMG-safe auto-match");

        assert_eq!(selected.tvdb_id, "157390");
    }

    #[test]
    fn next_metadata_search_chunk_limits_movie_batch_keys() {
        let candidates = vec![
            build_prepared_movie_candidate(&["Alpha", "Beta"]),
            build_prepared_movie_candidate(&["Gamma"]),
        ];

        let chunk = next_metadata_search_chunk(
            &candidates,
            &HashMap::new(),
            2,
            movie_candidate_batch_search_keys,
        )
        .expect("next metadata search chunk");

        assert_eq!(
            chunk,
            vec![
                BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Alpha", None, None)
                    .expect("alpha key"),
                BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Gamma", None, None)
                    .expect("gamma key"),
            ]
        );
    }

    #[test]
    fn movie_candidate_batch_search_keys_populates_exact_id_fields() {
        let mut candidate = build_prepared_movie_candidate(&[""]);
        candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::ExternalImportRadarr,
            imdb_id: Some("tt0123456".into()),
            tmdb_id: Some("98765".into()),
            tvdb_id: Some("54321".into()),
            title: None,
            year: None,
        });
        candidate.metadata_lookup_attempted = true;

        let keys = movie_candidate_batch_search_keys(&candidate).expect("movie search keys");

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].query, "");
        assert_eq!(keys[0].type_hint, METADATA_TYPE_MOVIE);
        assert_eq!(keys[0].imdb_id.as_deref(), Some("tt0123456"));
        assert_eq!(keys[0].tmdb_id.as_deref(), Some("98765"));
        assert_eq!(keys[0].tvdb_id.as_deref(), Some("54321"));
    }

    #[test]
    fn series_candidate_batch_search_keys_populates_exact_id_fields() {
        let candidate = PreparedSeriesLibraryScanCandidate {
            folder_path: PathBuf::from("/library/Series"),
            folder_name: Some("Series".into()),
            nfo_meta: None,
            identity_hint: Some(MetadataIdentityHint {
                source: MetadataIdentitySource::ExternalImportSonarr,
                imdb_id: Some("tt7654321".into()),
                tmdb_id: Some("12345".into()),
                tvdb_id: Some("67890".into()),
                title: None,
                year: None,
            }),
            query: String::new(),
            year_hint: None,
            search_candidates: vec![String::new()],
            title_match_candidates: Vec::new(),
            metadata_lookup_attempted: true,
        };

        let keys = series_candidate_batch_search_keys(&candidate).expect("series search keys");

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].query, "");
        assert_eq!(keys[0].type_hint, METADATA_TYPE_SERIES);
        assert_eq!(keys[0].imdb_id.as_deref(), Some("tt7654321"));
        assert_eq!(keys[0].tmdb_id.as_deref(), Some("12345"));
        assert_eq!(keys[0].tvdb_id.as_deref(), Some("67890"));
    }

    #[test]
    fn next_metadata_search_chunk_preserves_same_title_with_distinct_ids() {
        let mut alpha_one = build_prepared_movie_candidate(&[""]);
        alpha_one.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::ExternalImportRadarr,
            imdb_id: Some("tt0000001".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: None,
            year: None,
        });
        alpha_one.metadata_lookup_attempted = true;
        let mut alpha_two = build_prepared_movie_candidate(&[""]);
        alpha_two.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::ExternalImportRadarr,
            imdb_id: Some("tt0000002".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: None,
            year: None,
        });
        alpha_two.metadata_lookup_attempted = true;

        let chunk = next_metadata_search_chunk(
            &[alpha_one, alpha_two],
            &HashMap::new(),
            50,
            movie_candidate_batch_search_keys,
        )
        .expect("next metadata search chunk");

        assert_eq!(chunk.len(), 2);
        assert_eq!(chunk[0].imdb_id.as_deref(), Some("tt0000001"));
        assert_eq!(chunk[1].imdb_id.as_deref(), Some("tt0000002"));
    }

    #[test]
    fn split_ready_metadata_candidates_waits_for_all_movie_search_results() {
        let ready_candidate = build_prepared_movie_candidate(&["Alpha", "Beta"]);
        let pending_candidate = build_prepared_movie_candidate(&["Gamma"]);
        let mut search_results = HashMap::new();
        search_results.insert(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Alpha", None, None)
                .expect("alpha key"),
            Arc::new(Vec::new()),
        );
        search_results.insert(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Beta", None, None).expect("beta key"),
            Arc::new(Vec::new()),
        );

        let (ready, pending) = split_ready_metadata_candidates(
            vec![ready_candidate.clone(), pending_candidate.clone()],
            &search_results,
            movie_candidate_batch_search_keys,
        )
        .expect("split ready metadata candidates");

        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].query, ready_candidate.query);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].query, pending_candidate.query);
    }

    #[test]
    fn split_ready_metadata_candidates_waits_when_exact_id_result_is_not_safe() {
        let mut pending_candidate = build_prepared_movie_candidate(&["", "Alpha"]);
        pending_candidate.identity_hint = Some(MetadataIdentityHint {
            source: MetadataIdentitySource::Nfo,
            imdb_id: Some("tt1234567".into()),
            tmdb_id: None,
            tvdb_id: None,
            title: Some("Alpha".into()),
            year: Some(2024),
        });
        pending_candidate.metadata_lookup_attempted = true;
        let exact_key = BatchMetadataSearchKey::new(
            METADATA_TYPE_MOVIE,
            "",
            None,
            pending_candidate.identity_hint.as_ref(),
        )
        .expect("exact id key");
        let mut search_results = HashMap::new();
        search_results.insert(
            exact_key,
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "12345".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Wrong Alpha".into(),
                year: Some(2024),
                auto_match_safe: false,
                auto_match_signals: vec!["external_id_conflict".into()],
            }]),
        );

        let (ready, pending) = split_ready_metadata_candidates(
            vec![pending_candidate],
            &search_results,
            movie_candidate_batch_search_keys,
        )
        .expect("split ready metadata candidates");

        assert!(ready.is_empty());
        assert_eq!(pending.len(), 1);
        let fallback_chunk = next_metadata_search_chunk(
            &pending,
            &search_results,
            50,
            movie_candidate_batch_search_keys,
        )
        .expect("fallback search chunk");
        assert_eq!(fallback_chunk.len(), 1);
        assert_eq!(fallback_chunk[0].query, "Alpha");
    }

    #[test]
    fn split_ready_metadata_candidates_accepts_first_non_empty_movie_result() {
        let ready_candidate = build_prepared_movie_candidate(&["Alpha", "Beta"]);
        let pending_candidate = build_prepared_movie_candidate(&["Gamma"]);
        let mut search_results = HashMap::new();
        search_results.insert(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Alpha", None, None)
                .expect("alpha key"),
            Arc::new(vec![MetadataSearchItem {
                tvdb_id: "12345".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Alpha".into(),
                year: Some(2024),
                auto_match_safe: true,
                auto_match_signals: vec!["external_id:imdb".into()],
            }]),
        );

        let (ready, pending) = split_ready_metadata_candidates(
            vec![ready_candidate.clone(), pending_candidate.clone()],
            &search_results,
            movie_candidate_batch_search_keys,
        )
        .expect("split ready metadata candidates");

        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].query, ready_candidate.query);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].query, pending_candidate.query);
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_accepts_movie_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<movie><title>Test Movie Title</title><tvdbid>123456</tvdbid></movie>"#,
        )
        .expect("write nfo");

        let metadata = read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
            .await
            .expect("movie nfo");
        assert_eq!(metadata.title.as_deref(), Some("Test Movie Title"));
        assert_eq!(metadata.tvdb_id.as_deref(), Some("123456"));
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_accepts_movie_root_with_xml_declaration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<movie>
  <title>Harbor Hound</title>
  <originaltitle>Harbor Hound</originaltitle>
  <sorttitle>Harbor Hound</sorttitle>
  <year>1997</year>
  <imdbid>tt0118570</imdbid>
  <tvdbid>5794</tvdbid>
  <tmdbid>20737</tmdbid>
  <id>tt0118570</id>
  <fileinfo>
    <streamdetails>
      <video>
        <codec>hevc</codec>
        <width>1920</width>
        <height>1080</height>
      </video>
      <audio>
        <codec>aac</codec>
        <language>eng</language>
      </audio>
    </streamdetails>
  </fileinfo>
</movie>%"#,
        )
        .expect("write nfo");

        let metadata = read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
            .await
            .expect("movie nfo");
        assert_eq!(metadata.title.as_deref(), Some("Harbor Hound"));
        assert_eq!(metadata.year, Some(1997));
        assert_eq!(metadata.imdb_id.as_deref(), Some("tt0118570"));
        assert_eq!(metadata.tvdb_id.as_deref(), Some("5794"));
        assert_eq!(metadata.tmdb_id.as_deref(), Some("20737"));
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_rejects_tvshow_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<tvshow><title>Silver Horizon</title><tvdbid>81189</tvdbid></tvshow>"#,
        )
        .expect("write nfo");

        assert!(
            read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_coalesces_duplicate_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(
            METADATA_TYPE_MOVIE,
            "Glass Harbor",
            vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Glass Harbor".into(),
                year: Some(2021),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        );

        let files = vec![
            build_library_file("/library/Glass Harbor (2021)/Glass.Harbor.2021.2160p.BluRay.mkv"),
            build_library_file("/elsewhere/Glass Harbor (2021)/Glass.Harbor.2021.1080p.WEB-DL.mkv"),
        ];

        let (candidates, stats) =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect("movie preload should succeed");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_MOVIE, "Glass Harbor"),
            1
        );
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("movie-1")
        }));
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_reuses_shared_fallback_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(METADATA_TYPE_MOVIE, "MON PHARE AKA MY LIGHTHOUSE", vec![]);
        gateway.set_search_results(METADATA_TYPE_MOVIE, "MON PHARE", vec![]);
        gateway.set_search_results(
            METADATA_TYPE_MOVIE,
            "MY LIGHTHOUSE",
            vec![MetadataSearchItem {
                tvdb_id: "movie-2".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "My Lighthouse".into(),
                year: Some(2020),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        );

        let files = vec![
            build_library_file("/library/Mon.Phare.A.K.A.My.Lighthouse.2020.1080p.BluRay.mkv"),
            build_library_file("/library/My.Lighthouse.2020.720p.WEB-DL.mkv"),
        ];

        let (candidates, stats) =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect("movie preload should succeed");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_MOVIE, "MY LIGHTHOUSE"),
            1
        );
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 3);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("movie-2")
        }));
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_preserves_error_behavior_for_shared_requests() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_error(METADATA_TYPE_MOVIE, "Glass Harbor", "rate limited");

        let files = vec![
            build_library_file("/library/Glass Harbor (2021)/Glass.Harbor.2021.2160p.BluRay.mkv"),
            build_library_file("/elsewhere/Glass Harbor (2021)/Glass.Harbor.2021.1080p.WEB-DL.mkv"),
        ];

        let error =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect_err("movie preload should fail on shared request error");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_MOVIE, "Glass Harbor"),
            1
        );
        assert!(matches!(error, AppError::Repository(message) if message == "rate limited"));
    }

    #[tokio::test]
    async fn preload_series_library_scan_candidates_coalesces_duplicate_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(
            METADATA_TYPE_SERIES,
            "Silver Horizon",
            vec![MetadataSearchItem {
                tvdb_id: "series-1".into(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Silver Horizon".into(),
                year: Some(2018),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        );

        let folders = vec![
            PathBuf::from("/library-a/Silver Horizon (2018)"),
            PathBuf::from("/library-b/Silver Horizon (2018)"),
        ];

        let (candidates, stats) =
            preload_series_library_scan_candidates(Arc::new(gateway.clone()), &folders)
                .await
                .expect("series preload should succeed");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_SERIES, "Silver Horizon"),
            1
        );
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("series-1")
        }));
    }

    #[tokio::test]
    async fn prepare_movie_candidate_ignores_plexmatch_hint() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let folder = tempdir.path().join("The Lantern Supremacy (2004)");
        std::fs::create_dir_all(&folder).expect("create movie dir");
        let movie_path = folder.join("The Lantern Supremacy (2004) Remux-1080p.mkv");
        std::fs::write(&movie_path, b"movie").expect("write movie");
        std::fs::write(
            folder.join(".plexmatch"),
            "title: Pelton\nyear: 1970\ntvdbid: 2502\n",
        )
        .expect("write plexmatch");

        let candidate = prepare_movie_library_scan_candidate(
            LibraryFile {
                path: path_to_stored_string(&movie_path),
                display_name: "The Lantern Supremacy (2004) Remux-1080p".into(),
                nfo_path: None,
                size_bytes: None,
                source_signature_scheme: None,
                source_signature_value: None,
            },
            path_to_stored_string(tempdir.path()),
        )
        .await
        .expect("prepare movie candidate");

        assert_eq!(candidate.query, "The Lantern Supremacy");
        assert_eq!(candidate.year_hint, Some(2004));
        assert!(candidate.identity_hint.as_ref().is_none_or(|hint| {
            hint.tvdb_id.as_deref() != Some("2502")
                && hint.title.as_deref() != Some("Pelton")
                && hint.year != Some(1970)
        }));
    }

    #[tokio::test]
    async fn prepare_series_library_scan_candidates_prefers_tvshow_nfo_identity_for_nightfall_fixture()
     {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let folder = tempdir.path().join("Nightfall!! (2022)");
        std::fs::create_dir_all(&folder).expect("create show dir");
        std::fs::write(folder.join("tvshow.nfo"), nightfall_tvshow_nfo_fixture())
            .expect("write tvshow.nfo");

        let candidates =
            prepare_series_library_scan_candidates(std::slice::from_ref(&folder), None)
                .await
                .expect("prepare series candidates");

        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.query, "Nightfall!!");
        assert_eq!(candidate.year_hint, Some(2022));
        assert_eq!(
            candidate
                .nfo_meta
                .as_ref()
                .and_then(|meta| meta.tvdb_id.as_deref()),
            Some("415677")
        );
        assert!(candidate.metadata_lookup_attempted);
        // The tvshow.nfo carries a tvdb id, so the scan leads with an
        // empty-query, id-anchored lookup (SMG resolves by id only when the
        // query is empty) and keeps the title variants as fallback.
        assert_eq!(
            candidate.search_candidates,
            vec![
                String::new(),
                "Nightfall!!".to_string(),
                "nightfall".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn preload_series_library_scan_candidates_rejects_wrong_year_match_for_nightfall_fixture()
    {
        let gateway = CountingMetadataGateway::default();
        let wrong_year_match = vec![MetadataSearchItem {
            tvdb_id: "wrong-series".into(),
            smg_id: None,
            primary_source: None,
            external_ids: vec![],
            name: "Nightfall".into(),
            year: Some(2009),
            auto_match_safe: false,
            auto_match_signals: vec![],
        }];
        gateway.set_search_results(
            METADATA_TYPE_SERIES,
            "Nightfall!!",
            wrong_year_match.clone(),
        );
        gateway.set_search_results(METADATA_TYPE_SERIES, "nightfall", wrong_year_match);

        let tempdir = tempfile::tempdir().expect("tempdir");
        let folder = tempdir.path().join("Nightfall!! (2022)");
        std::fs::create_dir_all(&folder).expect("create show dir");
        std::fs::write(
            folder.join("tvshow.nfo"),
            r#"<tvshow><title>Nightfall!!</title><year>2022</year></tvshow>"#,
        )
        .expect("write tvshow.nfo");

        let (candidates, stats) =
            preload_series_library_scan_candidates(Arc::new(gateway.clone()), &[folder])
                .await
                .expect("series preload should succeed");

        assert_eq!(stats.logical_lookups, 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].query, "Nightfall!!");
        assert_eq!(
            candidates[0]
                .nfo_meta
                .as_ref()
                .and_then(|meta| meta.year)
                .map(|value| value as u32),
            Some(2022)
        );
        assert!(candidates[0].selected_metadata.is_none());
    }

    #[tokio::test]
    async fn preload_series_library_scan_candidates_preserves_error_behavior_for_shared_requests() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_error(
            METADATA_TYPE_SERIES,
            "Silver Horizon",
            "series rate limited",
        );

        let folders = vec![
            PathBuf::from("/library-a/Silver Horizon (2018)"),
            PathBuf::from("/library-b/Silver Horizon (2018)"),
        ];

        let (candidates, stats) =
            preload_series_library_scan_candidates(Arc::new(gateway.clone()), &folders)
                .await
                .expect("series preload should degrade gracefully");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_SERIES, "Silver Horizon"),
            1
        );
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert!(candidates.iter().all(|candidate| {
            candidate.metadata_lookup_error.as_deref() == Some("repository: series rate limited")
                && candidate.selected_metadata.is_none()
        }));
    }

    #[tokio::test]
    async fn prepare_movie_directory_entry_uses_shallow_evidence_candidate() {
        let scanner = DelayedLibraryScanner::default();
        scanner.set_response(
            "/library/Fast Movie",
            0,
            vec![build_library_file(
                "/library/Fast Movie/Fast.Movie.2024.mkv",
            )],
        );

        let entry = MovieTopLevelEntry {
            path: PathBuf::from("/library/Fast Movie"),
            is_dir: true,
        };

        let prepared = prepare_movie_library_scan_entry(
            Arc::new(scanner.clone()),
            entry,
            "/library".to_string(),
            None,
        )
        .await
        .expect("prepare movie directory entry");

        assert_eq!(prepared.file.display_name, "Fast.Movie.2024");
        assert!(!prepared.representative_is_directory);
        assert!(prepared.discovered_files.is_empty());
        assert_eq!(scanner.scan_directory_call_count(), 1);
        assert_eq!(scanner.scan_library_call_count(), 0);
    }

    #[tokio::test]
    async fn prepare_movie_directory_entry_keeps_empty_folder_candidate() {
        let scanner = DelayedLibraryScanner::default();
        scanner.set_response("/library/Empty Movie (2024)", 0, Vec::new());

        let entry = MovieTopLevelEntry {
            path: PathBuf::from("/library/Empty Movie (2024)"),
            is_dir: true,
        };

        let prepared = prepare_movie_library_scan_entry(
            Arc::new(scanner.clone()),
            entry,
            "/library".to_string(),
            None,
        )
        .await
        .expect("prepare empty movie directory entry");

        assert_eq!(prepared.file.path, "/library/Empty Movie (2024)");
        assert_eq!(prepared.file.display_name, "Empty Movie (2024)");
        assert!(prepared.representative_is_directory);
        assert!(prepared.discovered_files.is_empty());
        assert_eq!(scanner.scan_directory_call_count(), 1);
        assert_eq!(scanner.scan_library_call_count(), 0);
    }

    #[tokio::test]
    async fn execute_batch_metadata_searches_returns_quickly_after_cancel() {
        let gateway = Arc::new(DelayedBatchMetadataGateway::new(Duration::from_millis(500)));
        let cancel_token = CancellationToken::new();
        let search_keys = vec![
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Glass Harbor", None, None)
                .expect("metadata search key"),
        ];

        let cancel_handle = {
            let cancel_token = cancel_token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(25)).await;
                cancel_token.cancel();
            })
        };

        let result = tokio::time::timeout(
            Duration::from_millis(150),
            execute_batch_metadata_searches(gateway, search_keys, "eng", Some(&cancel_token)),
        )
        .await
        .expect("metadata search should stop waiting after cancel")
        .expect("canceled metadata search should not fail");

        cancel_handle.await.expect("cancel trigger task");
        assert!(
            result.is_empty(),
            "canceled metadata search should drop late results"
        );
    }

    #[test]
    fn sample_video_candidate_requires_sample_name_signal() {
        assert!(is_sample_video_candidate(Path::new(
            "/library/Movie/sample-featurette.mkv"
        )));
        assert!(!is_sample_video_candidate(Path::new(
            "/library/Movie/Short.Film.2024.mkv"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn sample_video_candidate_detects_non_utf8_name_signal() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        assert!(is_sample_video_candidate(Path::new(OsStr::from_bytes(
            b"/library/Movie/sample-\xFFfeaturette.mkv"
        ))));
    }

    #[tokio::test]
    async fn detect_primary_movie_entry_file_keeps_small_non_sample_video() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_dir = dir.path().join("Short Film (2024)");
        tokio::fs::create_dir_all(&movie_dir)
            .await
            .expect("movie dir");
        let movie_path = movie_dir.join("Short.Film.2024.mkv");
        tokio::fs::write(&movie_path, b"tiny-but-real")
            .await
            .expect("movie file");

        let discovered_files = vec![LibraryFile {
            path: movie_path.to_string_lossy().to_string(),
            display_name: "Short.Film.2024".to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }];

        let primary = detect_primary_movie_entry_file(&movie_dir, &discovered_files)
            .await
            .expect("primary");

        assert_eq!(
            primary.as_deref(),
            Some(movie_path.to_string_lossy().as_ref())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detect_primary_movie_entry_file_ignores_encoded_non_utf8_sample_video() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let movie_dir = Path::new("/library/Short Film (2024)");
        let sample_path = Path::new(OsStr::from_bytes(
            b"/library/Short Film (2024)/sample-\xFFfeaturette.mkv",
        ));
        let movie_path = Path::new(OsStr::from_bytes(
            b"/library/Short Film (2024)/Short.Film.\xFF2024.mkv",
        ));
        let discovered_files = vec![
            LibraryFile {
                path: path_to_stored_string(sample_path),
                display_name: "sample-\u{FFFD}featurette".to_string(),
                nfo_path: None,
                size_bytes: None,
                source_signature_scheme: None,
                source_signature_value: None,
            },
            LibraryFile {
                path: path_to_stored_string(movie_path),
                display_name: "Short.Film.\u{FFFD}2024".to_string(),
                nfo_path: None,
                size_bytes: None,
                source_signature_scheme: None,
                source_signature_value: None,
            },
        ];

        let primary = detect_primary_movie_entry_file(movie_dir, &discovered_files)
            .await
            .expect("primary");

        assert_eq!(
            primary.as_deref(),
            Some(path_to_stored_string(movie_path).as_str())
        );
    }

    #[tokio::test]
    async fn directory_movie_nfo_path_finds_folder_movie_nfo_without_same_stem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_dir = dir.path().join("Aurelia (1997)");
        tokio::fs::create_dir_all(&movie_dir)
            .await
            .expect("movie dir");
        let movie_path = movie_dir.join("Aurelia (1997) Bluray-1080p.mkv");
        tokio::fs::write(&movie_path, b"movie")
            .await
            .expect("movie file");
        let movie_nfo = movie_dir.join("movie.nfo");
        tokio::fs::write(&movie_nfo, b"<movie><title>Aurelia</title></movie>")
            .await
            .expect("movie nfo");

        // No same-stem `<file>.nfo` exists, so the folder-level movie.nfo must be
        // associated unconditionally (the old primary-candidate gate dropped it).
        let resolved =
            directory_movie_nfo_path(&movie_dir, &path_to_stored_string(&movie_path)).await;

        assert_eq!(
            resolved.as_deref(),
            Some(path_to_stored_string(&movie_nfo).as_str())
        );
    }

    #[tokio::test]
    async fn prepare_movie_candidate_leads_with_id_anchored_lookup_for_nfo_ids() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let folder = tempdir.path().join("Aurelia (1997)");
        std::fs::create_dir_all(&folder).expect("create movie dir");
        let movie_path = folder.join("Aurelia (1997) Bluray-1080p.mkv");
        std::fs::write(&movie_path, b"movie").expect("write movie");
        let movie_nfo = folder.join("movie.nfo");
        std::fs::write(
            &movie_nfo,
            r#"<movie><title>Aurelia</title><year>1997</year><imdbid>tt0118617</imdbid><tvdbid>933</tvdbid><tmdbid>9444</tmdbid></movie>"#,
        )
        .expect("write movie nfo");

        let candidate = prepare_movie_library_scan_candidate(
            LibraryFile {
                path: path_to_stored_string(&movie_path),
                display_name: "Aurelia (1997) Bluray-1080p".into(),
                nfo_path: Some(path_to_stored_string(&movie_nfo)),
                size_bytes: None,
                source_signature_scheme: None,
                source_signature_value: None,
            },
            path_to_stored_string(tempdir.path()),
        )
        .await
        .expect("prepare movie candidate");

        let identity = candidate.identity_hint.as_ref().expect("identity hint");
        assert_eq!(identity.tvdb_id.as_deref(), Some("933"));
        assert_eq!(identity.imdb_id.as_deref(), Some("tt0118617"));
        assert_eq!(identity.tmdb_id.as_deref(), Some("9444"));
        // The NFO ids drive an empty-query, id-anchored lookup first; the title
        // text variants follow as fallback.
        assert_eq!(
            candidate.search_candidates.first().map(String::as_str),
            Some("")
        );
        assert!(
            candidate
                .search_candidates
                .iter()
                .any(|value| !value.trim().is_empty()),
            "title fallback variants should follow the id-anchored lookup: {:?}",
            candidate.search_candidates
        );
    }
}
