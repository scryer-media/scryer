use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

use super::*;
use crate::nfo::{looks_like_movie_nfo, parse_nfo};
use tracing::{info, warn};

const METADATA_TYPE_MOVIE: &str = "movie";
const LIBRARY_METADATA_LOOKUP_CONCURRENCY: usize = 4;
const LIBRARY_SCAN_BATCH_SIZE: usize = 128;
const TITLE_SCAN_FILE_BATCH_SIZE: usize = 128;
const TITLE_SCAN_ANALYSIS_CONCURRENCY: usize = 2;
const RADARR_MOVIE_NFO_MAX_BYTES: u64 = 10 * 1024 * 1024;
const RENAME_TEMPLATE_KEY: &str = "rename.template";
const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const DEFAULT_COLLISION_POLICY: RenameCollisionPolicy = RenameCollisionPolicy::Skip;
const DEFAULT_MISSING_METADATA_POLICY: RenameMissingMetadataPolicy =
    RenameMissingMetadataPolicy::FallbackTitle;

fn extract_library_queries(path: &str, library_root: &str) -> (Vec<String>, Option<u32>) {
    // Normalise paths for comparison (strip trailing slash)
    let root = library_root.trim_end_matches('/');

    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(stem);
    let parsed_queries = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    let mut queries = Vec::new();
    let mut seen_normalized = HashSet::new();

    // Preserve the folder name as the canonical movie title for nested-library
    // layouts. We only fall back to parsed release variants when the file sits
    // at the library root. The parsed release year still wins, because it is a
    // better signal when the filename and parent folder disagree.
    let mut folder_year = None;

    // Attempt to get the immediate parent directory of the file.
    if let Some(parent) = Path::new(path).parent() {
        let parent_str = parent.to_string_lossy();
        // Only use parent folder when it is NOT the library root itself
        // (i.e. the file is inside a sub-folder).
        if parent_str.trim_end_matches('/') != root
            && let Some(folder_name) = parent.file_name().and_then(|n| n.to_str())
        {
            let clean = normalize_folder_name(folder_name);
            let (title, year) = strip_year_suffix(&clean);
            if !title.trim().is_empty() {
                push_unique_query(&mut queries, &mut seen_normalized, title);
                folder_year = year;
            }
        }
    }

    if queries.is_empty() {
        for query in parsed_queries {
            push_unique_query(&mut queries, &mut seen_normalized, query);
        }
    }

    (queries, parsed.year.or(folder_year))
}

/// Normalizes a folder name by replacing non-breaking spaces and other Unicode
/// whitespace with regular ASCII spaces, and collapsing runs of whitespace.
fn normalize_folder_name(name: &str) -> String {
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

/// Strips a trailing ` (YYYY)` or ` [YYYY]` year token from a folder name,
/// returning the cleaned title and the parsed year.
fn strip_year_suffix(folder: &str) -> (String, Option<u32>) {
    // Match trailing " (YYYY)" or " [YYYY]"
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

fn normalized_query_title_candidates(queries: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for query in queries {
        let value = crate::app_usecase_rss::normalize_for_matching(query);
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        normalized.push(value);
    }

    normalized
}

async fn list_child_directories(root: &Path) -> AppResult<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let mut entries = tokio::fs::read_dir(root).await.map_err(|err| {
        AppError::Repository(format!(
            "failed to read directory {}: {err}",
            root.display()
        ))
    })?;
    while let Some(entry) = entries.next_entry().await.map_err(|err| {
        AppError::Repository(format!("failed to read entry in {}: {err}", root.display()))
    })? {
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSourceSignature {
    scheme: String,
    value: String,
}

#[derive(Clone, Debug)]
struct FileSourceSnapshot {
    size_bytes: i64,
    signature: Option<FileSourceSignature>,
}

#[derive(Clone, Debug, Default)]
struct TitleEpisodeLookup {
    by_air_date: HashMap<String, Vec<Episode>>,
    by_collection_episode: HashMap<(String, String), Episode>,
    by_absolute_number: HashMap<String, Episode>,
    by_collection_index: HashMap<String, Vec<Episode>>,
}

#[derive(Clone, Debug)]
struct PlannedTitleScanFile {
    file: LibraryFile,
    parsed: crate::ParsedReleaseMetadata,
    target_episodes: Vec<Episode>,
    snapshot: FileSourceSnapshot,
    record: PlannedTitleScanRecord,
}

#[derive(Clone, Debug)]
enum PlannedTitleScanRecord {
    Existing {
        file_id: String,
        should_skip_analysis: bool,
        should_refresh_source_signature: bool,
    },
    New,
}

#[derive(Clone, Debug)]
struct MovieLibraryScanCandidate {
    file: LibraryFile,
    parsed_release: crate::ParsedReleaseMetadata,
    nfo_meta: Option<crate::nfo::NfoMetadata>,
    query: String,
    year_hint: Option<u32>,
    query_variants: Vec<String>,
    selected_metadata: Option<MetadataSearchItem>,
    metadata_lookup_attempted: bool,
}

#[derive(Clone, Debug)]
struct SeriesLibraryScanCandidate {
    folder_path: PathBuf,
    folder_name: Option<String>,
    nfo_meta: Option<crate::nfo::NfoMetadata>,
    query: String,
    selected_metadata: Option<MetadataSearchItem>,
    metadata_lookup_error: Option<String>,
    metadata_lookup_attempted: bool,
}

fn index_movie_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    existing_titles_by_name.insert(normalize_title_key(&title.name), index);
    for alias in &title.aliases {
        existing_titles_by_name.insert(normalize_title_key(alias), index);
    }
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
        {
            existing_titles_by_imdb_id.insert(imdb_id, index);
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            existing_titles_by_tmdb_id.insert(external_id.value.clone(), index);
        }
    }
}

fn index_series_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
) {
    existing_titles_by_name.insert(normalize_title_key(&title.name), index);
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        }
    }
}

async fn read_valid_movie_nfo_metadata(nfo_path: Option<&str>) -> Option<crate::nfo::NfoMetadata> {
    let path = Path::new(nfo_path?).to_path_buf();
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() > RADARR_MOVIE_NFO_MAX_BYTES {
        return None;
    }

    let content = tokio::fs::read_to_string(path).await.ok()?;
    if !looks_like_movie_nfo(&content) {
        return None;
    }

    Some(parse_nfo(&content))
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

async fn preload_movie_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    files: &[LibraryFile],
    library_path: &str,
) -> AppResult<(Vec<MovieLibraryScanCandidate>, usize)> {
    let lookup_limit = Arc::new(tokio::sync::Semaphore::new(
        LIBRARY_METADATA_LOOKUP_CONCURRENCY,
    ));
    let mut lookup_set = tokio::task::JoinSet::new();

    for (index, file) in files.iter().cloned().enumerate() {
        let metadata_gateway = metadata_gateway.clone();
        let lookup_limit = lookup_limit.clone();
        let library_path = library_path.to_string();

        lookup_set.spawn(async move {
            let parsed_release = parse_release_metadata(
                Path::new(&file.path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(file.display_name.as_str()),
            );

            let nfo_meta = read_valid_movie_nfo_metadata(file.nfo_path.as_deref()).await;
            let (query_variants, extracted_year_hint) =
                extract_library_queries(&file.path, &library_path);
            let fallback_query = query_variants.first().cloned().unwrap_or_default();

            let (query, year_hint) = if let Some(ref meta) = nfo_meta {
                let title = meta.title.clone().unwrap_or_else(|| fallback_query.clone());
                let year = meta.year.map(|value| value as u32).or(extracted_year_hint);
                (title, year)
            } else {
                (fallback_query, extracted_year_hint)
            };
            let mut selected_metadata = None;
            let mut metadata_lookup_attempted = false;

            if nfo_meta
                .as_ref()
                .and_then(|meta| meta.tvdb_id.as_deref())
                .is_none()
                && !query.trim().is_empty()
            {
                metadata_lookup_attempted = true;
                let mut search_candidates = Vec::new();
                let mut seen_search_candidates = HashSet::new();
                for value in query_variants
                    .iter()
                    .cloned()
                    .chain(std::iter::once(query.clone()))
                {
                    if value.trim().is_empty() || !seen_search_candidates.insert(value.clone()) {
                        continue;
                    }
                    search_candidates.push(value);
                }
                let normalized_title_candidates =
                    normalized_query_title_candidates(&search_candidates);

                let _permit = lookup_limit
                    .acquire_owned()
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;

                for candidate in search_candidates {
                    let results = metadata_gateway
                        .search_tvdb(&candidate, METADATA_TYPE_MOVIE)
                        .await?;
                    if let Some(best) =
                        select_best_match(&results, year_hint, &normalized_title_candidates)
                    {
                        selected_metadata = Some(best);
                        break;
                    }
                }
            }

            Ok::<_, AppError>((
                index,
                MovieLibraryScanCandidate {
                    file,
                    parsed_release,
                    nfo_meta,
                    query,
                    year_hint,
                    query_variants,
                    selected_metadata,
                    metadata_lookup_attempted,
                },
            ))
        });
    }

    let mut results = vec![None; lookup_set.len()];
    let mut metadata_lookups = 0usize;
    while let Some(result) = lookup_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        if candidate.metadata_lookup_attempted {
            metadata_lookups += 1;
        }
        results[index] = Some(candidate);
    }

    Ok((results.into_iter().flatten().collect(), metadata_lookups))
}

async fn preload_series_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    folders: &[PathBuf],
) -> AppResult<(Vec<SeriesLibraryScanCandidate>, usize)> {
    let lookup_limit = Arc::new(tokio::sync::Semaphore::new(
        LIBRARY_METADATA_LOOKUP_CONCURRENCY,
    ));
    let mut lookup_set = tokio::task::JoinSet::new();

    for (index, folder) in folders.iter().cloned().enumerate() {
        let metadata_gateway = metadata_gateway.clone();
        let lookup_limit = lookup_limit.clone();

        lookup_set.spawn(async move {
            let folder_name = folder
                .file_name()
                .and_then(|name| name.to_str())
                .map(std::string::ToString::to_string);

            let Some(folder_name_value) = folder_name.clone() else {
                return Ok::<_, AppError>((
                    index,
                    SeriesLibraryScanCandidate {
                        folder_path: folder,
                        folder_name: None,
                        nfo_meta: None,
                        query: String::new(),
                        selected_metadata: None,
                        metadata_lookup_error: None,
                        metadata_lookup_attempted: false,
                    },
                ));
            };

            let nfo_meta = read_tvshow_nfo_metadata(folder.clone()).await;
            let clean_name = normalize_folder_name(&folder_name_value);
            let (query, year_hint) = strip_year_suffix(&clean_name);
            let query = query.trim().to_string();

            let mut selected_metadata = None;
            let mut metadata_lookup_error = None;
            let mut metadata_lookup_attempted = false;

            if nfo_meta
                .as_ref()
                .and_then(|meta| meta.tvdb_id.as_deref())
                .is_none()
                && !query.is_empty()
            {
                metadata_lookup_attempted = true;
                let normalized_title_candidates =
                    normalized_query_title_candidates(std::slice::from_ref(&query));
                let _permit = lookup_limit
                    .acquire_owned()
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;

                match metadata_gateway.search_tvdb(&query, "series").await {
                    Ok(results) => {
                        selected_metadata =
                            select_best_match(&results, year_hint, &normalized_title_candidates);
                    }
                    Err(error) => {
                        metadata_lookup_error = Some(error.to_string());
                    }
                }
            }

            Ok::<_, AppError>((
                index,
                SeriesLibraryScanCandidate {
                    folder_path: folder,
                    folder_name,
                    nfo_meta,
                    query,
                    selected_metadata,
                    metadata_lookup_error,
                    metadata_lookup_attempted,
                },
            ))
        });
    }

    let mut results = vec![None; lookup_set.len()];
    let mut metadata_lookups = 0usize;
    while let Some(result) = lookup_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        if candidate.metadata_lookup_attempted {
            metadata_lookups += 1;
        }
        results[index] = Some(candidate);
    }

    Ok((results.into_iter().flatten().collect(), metadata_lookups))
}

async fn ensure_title_folder_path_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) {
    let folder_path = folder_path.to_string_lossy().trim().to_string();
    if folder_path.is_empty()
        || title
            .folder_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return;
    }

    match app
        .services
        .titles
        .set_folder_path(&title.id, folder_path.as_str())
        .await
    {
        Ok(()) => title.folder_path = Some(folder_path),
        Err(error) => warn!(
            error = %error,
            title_id = %title.id,
            folder_path = %folder_path,
            "failed to persist discovered title folder path during library scan"
        ),
    }
}

async fn should_relink_existing_episodic_title(
    app: &AppUseCase,
    title: &Title,
    episode_presence_cache: &mut HashMap<String, bool>,
) -> bool {
    if title.metadata_fetched_at.is_some() {
        return true;
    }

    if let Some(has_episodes) = episode_presence_cache.get(&title.id) {
        return *has_episodes;
    }

    let has_episodes = match app.services.shows.list_episodes_for_title(&title.id).await {
        Ok(episodes) => !episodes.is_empty(),
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to inspect episode records during library scan relink eligibility check"
            );
            false
        }
    };

    episode_presence_cache.insert(title.id.clone(), has_episodes);
    has_episodes
}

fn file_source_signature_from_metadata(
    metadata: &std::fs::Metadata,
) -> Option<FileSourceSignature> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        Some(FileSourceSignature {
            scheme: "windows_last_write_100ns_v1".to_string(),
            value: metadata.last_write_time().to_string(),
        })
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Some(FileSourceSignature {
            scheme: "unix_mtime_nsec_v1".to_string(),
            value: format!("{}:{}", metadata.mtime(), metadata.mtime_nsec()),
        })
    }

    #[cfg(not(any(unix, windows)))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|modified| match modified.duration_since(UNIX_EPOCH) {
                Ok(duration) => Some(FileSourceSignature {
                    scheme: "system_time_nsec_v1".to_string(),
                    value: format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
                }),
                Err(error) => {
                    let duration = error.duration();
                    Some(FileSourceSignature {
                        scheme: "system_time_nsec_v1".to_string(),
                        value: format!("-{}:{}", duration.as_secs(), duration.subsec_nanos()),
                    })
                }
            })
    }
}

fn title_media_file_matches_snapshot(
    media_file: &TitleMediaFile,
    snapshot: &FileSourceSnapshot,
) -> bool {
    if media_file.scan_status != "scanned" || media_file.size_bytes != snapshot.size_bytes {
        return false;
    }

    match (
        &media_file.source_signature_scheme,
        &media_file.source_signature_value,
    ) {
        // Older rows created before source signatures existed can safely reuse
        // the previous analysis on the first post-migration scan when size and
        // scan status still match. The signature gets backfilled separately.
        (None, None) => true,
        (Some(scheme), Some(value)) => snapshot
            .signature
            .as_ref()
            .is_some_and(|signature| signature.scheme == *scheme && signature.value == *value),
        _ => false,
    }
}

fn build_title_episode_lookup(
    collections: &[Collection],
    episodes: &[Episode],
) -> TitleEpisodeLookup {
    let collection_indexes = collections
        .iter()
        .map(|collection| (collection.id.clone(), collection.collection_index.clone()))
        .collect::<HashMap<_, _>>();

    let mut lookup = TitleEpisodeLookup::default();
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

fn resolve_target_episodes_from_lookup(
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
    lookup: &TitleEpisodeLookup,
) -> Vec<Episode> {
    let mut episodes = Vec::new();
    let mut seen = HashSet::new();
    let target_season = if ep_meta.special_kind.is_some() || ep_meta.season == Some(0) {
        "0".to_string()
    } else {
        season_str.to_string()
    };

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        if let Some(matches) = lookup.by_air_date.get(&air_date_str) {
            if let Some(part) = ep_meta.daily_part {
                let part_index = part.saturating_sub(1) as usize;
                if let Some(episode) = matches.get(part_index)
                    && seen.insert(episode.id.clone())
                {
                    episodes.push(episode.clone());
                }
            } else {
                for episode in matches {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode.clone());
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
            episodes.push(episode.clone());
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        && let Some(collection_episodes) = lookup.by_collection_index.get(&target_season)
    {
        for episode in collection_episodes {
            if episode.season_number.as_deref() == Some(target_season.as_str())
                && seen.insert(episode.id.clone())
            {
                episodes.push(episode.clone());
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let key = ("0".to_string(), special_number.to_string());
            if let Some(episode) = lookup.by_collection_episode.get(&key)
                && seen.insert(episode.id.clone())
            {
                episodes.push(episode.clone());
            }
        }
    }

    if episodes.is_empty()
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
                episodes.push(episode.clone());
            }
        }
    }

    episodes
}

const SEASON_FOLDER_TAG_PREFIX: &str = "scryer:season-folder:";

fn set_structured_title_tag(tags: &mut Vec<String>, prefix: &str, value: Option<&str>) {
    tags.retain(|tag| !tag.starts_with(prefix));
    let Some(value) = value else {
        return;
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        return;
    }
    tags.push(format!("{prefix}{normalized}"));
}

fn merge_title_scan_option_tags(mut tags: Vec<String>, use_season_folders: bool) -> Vec<String> {
    set_structured_title_tag(
        &mut tags,
        SEASON_FOLDER_TAG_PREFIX,
        Some(if use_season_folders {
            "enabled"
        } else {
            "disabled"
        }),
    );
    tags
}

fn normalize_layout_component(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut prev_sep = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_whitespace() || matches!(lower, '.' | '_' | '-') {
            if !prev_sep {
                normalized.push(' ');
                prev_sep = true;
            }
        } else {
            normalized.push(lower);
            prev_sep = false;
        }
    }
    normalized.trim().to_string()
}

fn recognize_season_folder_name(name: &str) -> Option<u32> {
    let normalized = normalize_layout_component(name);
    if normalized.is_empty() {
        return None;
    }

    let compact = normalized.replace(' ', "");
    if matches!(compact.as_str(), "specials" | "specialepisodes") {
        return Some(0);
    }

    for prefix in ["season", "series", "s"] {
        let Some(rest) = compact.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        return rest.parse::<u32>().ok();
    }

    None
}

fn infer_target_season_number(target_episodes: &[Episode]) -> Option<u32> {
    let mut seasons = target_episodes
        .iter()
        .map(|episode| episode.season_number.as_deref()?.parse::<u32>().ok())
        .collect::<Option<HashSet<_>>>()?;
    if seasons.len() == 1 {
        seasons.drain().next()
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleScanLayoutObservation {
    Flat,
    SeasonFolder,
    Ambiguous,
}

fn classify_title_scan_layout(
    title_dir: &Path,
    file_path: &Path,
    target_episodes: &[Episode],
) -> TitleScanLayoutObservation {
    let Ok(relative) = file_path.strip_prefix(title_dir) else {
        return TitleScanLayoutObservation::Ambiguous;
    };

    let Some(parent) = relative.parent() else {
        return TitleScanLayoutObservation::Flat;
    };

    let first_component = parent
        .components()
        .find_map(|component| component.as_os_str().to_str())
        .filter(|component| !component.is_empty());

    let Some(first_component) = first_component else {
        return TitleScanLayoutObservation::Flat;
    };

    let Some(folder_season) = recognize_season_folder_name(first_component) else {
        return TitleScanLayoutObservation::Ambiguous;
    };

    match infer_target_season_number(target_episodes) {
        Some(target_season) if target_season == folder_season => {
            TitleScanLayoutObservation::SeasonFolder
        }
        _ => TitleScanLayoutObservation::Ambiguous,
    }
}

#[derive(Default)]
struct TitleScanLayoutSummary {
    saw_flat: bool,
    saw_season_folder: bool,
    ambiguous: bool,
}

impl TitleScanLayoutSummary {
    fn observe(&mut self, observation: TitleScanLayoutObservation) {
        match observation {
            TitleScanLayoutObservation::Flat => self.saw_flat = true,
            TitleScanLayoutObservation::SeasonFolder => self.saw_season_folder = true,
            TitleScanLayoutObservation::Ambiguous => self.ambiguous = true,
        }
    }

    fn inferred_use_season_folders(&self) -> Option<bool> {
        if self.ambiguous || self.saw_flat == self.saw_season_folder {
            None
        } else if self.saw_season_folder {
            Some(true)
        } else if self.saw_flat {
            Some(false)
        } else {
            None
        }
    }
}

impl AppUseCase {
    pub async fn preview_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        require(actor, &Entitlement::ManageTitle)?;

        let title = self
            .services
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;

        if title.facet != facet {
            return Err(AppError::Validation(
                "requested facet does not match title facet".into(),
            ));
        }

        let handler = self.facet_registry.get(&facet).ok_or_else(|| {
            AppError::Validation("rename preview is not supported for this facet".into())
        })?;
        let template = self.read_rename_template(handler.as_ref()).await?;
        let collision_policy = self.read_collision_policy(handler.as_ref()).await?;
        let missing_metadata_policy = self.read_missing_metadata_policy(handler.as_ref()).await?;
        let collections = self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let plan = build_rename_plan_for_facet(
            handler.as_ref(),
            &title,
            collections,
            template,
            collision_policy,
            missing_metadata_policy,
        );

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                EventType::ActionTriggered,
                format!(
                    "rename preview generated (total: {}, renamable: {}, conflicts: {}, errors: {})",
                    plan.total, plan.renamable, plan.conflicts, plan.errors
                ),
            )
            .await?;

        Ok(plan)
    }

    pub async fn preview_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        require(actor, &Entitlement::ManageTitle)?;

        let handler = self.facet_registry.get(&facet).ok_or_else(|| {
            AppError::Validation("rename preview is not supported for this facet".into())
        })?;
        let template = self.read_rename_template(handler.as_ref()).await?;
        let collision_policy = self.read_collision_policy(handler.as_ref()).await?;
        let missing_metadata_policy = self.read_missing_metadata_policy(handler.as_ref()).await?;
        let facet_label = handler.facet_id();

        let mut titles = self.services.titles.list(Some(facet.clone()), None).await?;
        titles.sort_by(|left, right| left.id.cmp(&right.id));

        let mut planned_targets = HashSet::new();
        let mut items = Vec::new();
        for title in titles {
            let mut collections = self
                .services
                .shows
                .list_collections_for_title(&title.id)
                .await?;
            collections.sort_by(|left, right| left.id.cmp(&right.id));

            for collection in collections {
                items.push(handler.build_rename_plan_item(
                    &title,
                    &collection,
                    &template,
                    &collision_policy,
                    &missing_metadata_policy,
                    &mut planned_targets,
                ));
            }
        }

        let plan = build_rename_plan_from_items(
            facet,
            None,
            template,
            collision_policy,
            missing_metadata_policy,
            items,
        );

        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!(
                    "facet rename preview generated for {facet_label} (total: {}, renamable: {}, conflicts: {}, errors: {})",
                    plan.total, plan.renamable, plan.conflicts, plan.errors
                ),
            )
            .await?;

        Ok(plan)
    }

    pub async fn apply_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let preview = self
            .preview_rename_for_title(actor, title_id, facet)
            .await?;
        if preview.fingerprint != plan_fingerprint {
            return Err(AppError::Validation("rename_stale_plan".into()));
        }

        self.apply_rename_plan(actor, Some(title_id), preview).await
    }

    pub async fn apply_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let preview = self.preview_rename_for_facet(actor, facet).await?;
        if preview.fingerprint != plan_fingerprint {
            return Err(AppError::Validation("rename_stale_plan".into()));
        }

        self.apply_rename_plan(actor, None, preview).await
    }

    async fn apply_rename_plan(
        &self,
        actor: &User,
        title_id_hint: Option<&str>,
        preview: RenamePlan,
    ) -> AppResult<RenameApplyResult> {
        self.services
            .record_event(
                Some(actor.id.clone()),
                title_id_hint.map(std::string::ToString::to_string),
                EventType::ActionTriggered,
                format!(
                    "rename apply started (total: {}, renamable: {}, conflicts: {}, errors: {})",
                    preview.total, preview.renamable, preview.conflicts, preview.errors
                ),
            )
            .await?;

        self.services
            .library_renamer
            .validate_targets(&preview)
            .await?;

        let mut item_results = self.services.library_renamer.apply_plan(&preview).await?;
        let mut applied = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for item in &mut item_results {
            match item.status {
                RenameApplyStatus::Applied => {
                    if let (Some(collection_id), Some(final_path)) =
                        (item.collection_id.as_deref(), item.final_path.clone())
                        && let Err(err) = self
                            .services
                            .shows
                            .update_collection(
                                collection_id,
                                None,
                                None,
                                None,
                                Some(final_path),
                                None,
                                None,
                                None,
                            )
                            .await
                    {
                        item.status = RenameApplyStatus::Failed;
                        item.reason_code = "db_update_failed".into();
                        item.error_message = Some(err.to_string());
                        failed += 1;
                        continue;
                    }
                    applied += 1;
                }
                RenameApplyStatus::Skipped => {
                    skipped += 1;
                }
                RenameApplyStatus::Failed => {
                    failed += 1;
                }
            }
        }

        let result = RenameApplyResult {
            plan_fingerprint: preview.fingerprint.clone(),
            total: item_results.len(),
            applied,
            skipped,
            failed,
            items: item_results,
        };

        self.services
            .record_event(
                Some(actor.id.clone()),
                title_id_hint.map(std::string::ToString::to_string),
                EventType::ActionCompleted,
                format!(
                    "rename apply complete (applied: {}, skipped: {}, failed: {})",
                    result.applied, result.skipped, result.failed
                ),
            )
            .await?;

        for item in &result.items {
            let final_path = item
                .final_path
                .as_deref()
                .unwrap_or(item.current_path.as_str());
            self.services
                .record_event(
                    Some(actor.id.clone()),
                    title_id_hint.map(std::string::ToString::to_string),
                    EventType::ActionCompleted,
                    format!(
                        "rename item {} -> {} ({})",
                        item.current_path,
                        final_path,
                        item.status.as_str()
                    ),
                )
                .await?;
        }

        Ok(result)
    }

    pub async fn scan_library(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;

        let path_key = match facet {
            MediaFacet::Movie => "movies.path",
            MediaFacet::Series => "series.path",
            MediaFacet::Anime => "anime.path",
        };

        let Some(library_path) = self
            .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, path_key, None)
            .await?
        else {
            return Err(AppError::Validation(format!(
                "{path_key} is not configured"
            )));
        };

        match facet {
            MediaFacet::Movie => self.scan_library_movies(actor, &facet, &library_path).await,
            MediaFacet::Series | MediaFacet::Anime => {
                self.scan_library_series(actor, &facet, &library_path).await
            }
        }
    }

    /// Movie library scan: each video file is a potential title.
    async fn scan_library_movies(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_path: &str,
    ) -> AppResult<LibraryScanSummary> {
        let started_at = Instant::now();
        let mut file_batches = self
            .services
            .library_scanner
            .scan_library_batched(library_path, LIBRARY_SCAN_BATCH_SIZE)
            .await?;
        let mut existing_titles = self.services.titles.list(Some(facet.clone()), None).await?;
        let mut existing_titles_by_name: HashMap<String, usize> = HashMap::new();
        let mut existing_titles_by_tvdb_id: HashMap<String, usize> = HashMap::new();
        let mut existing_titles_by_imdb_id: HashMap<String, usize> = HashMap::new();
        let mut existing_titles_by_tmdb_id: HashMap<String, usize> = HashMap::new();

        for (index, title) in existing_titles.iter().enumerate() {
            index_movie_title(
                title,
                index,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
            );
        }

        let mut summary = LibraryScanSummary::default();
        let mut metadata_lookups = 0usize;

        while let Some(file_batch) = file_batches.recv().await {
            let files = file_batch?;
            let (candidates, batch_lookups) = preload_movie_library_scan_candidates(
                self.services.metadata_gateway.clone(),
                &files,
                library_path,
            )
            .await?;
            metadata_lookups += batch_lookups;

            for candidate in candidates {
                summary.scanned += 1;
                let file = &candidate.file;
                let parsed_release = &candidate.parsed_release;
                let nfo_meta = candidate.nfo_meta.as_ref();

                if let Some(tvdb_id) = nfo_meta.and_then(|m| m.tvdb_id.as_deref()) {
                    let title = if let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id) {
                        existing_titles[index].clone()
                    } else {
                        let name = nfo_meta
                            .and_then(|m| m.title.clone())
                            .unwrap_or_else(|| candidate.query.clone());

                        let mut external_ids = vec![ExternalId {
                            source: "tvdb".into(),
                            value: tvdb_id.to_string(),
                        }];
                        if let Some(ref imdb) = nfo_meta.and_then(|m| m.imdb_id.clone()) {
                            external_ids.push(ExternalId {
                                source: "imdb".into(),
                                value: imdb.clone(),
                            });
                        }
                        if let Some(ref tmdb) = nfo_meta.and_then(|m| m.tmdb_id.clone()) {
                            external_ids.push(ExternalId {
                                source: "tmdb".into(),
                                value: tmdb.clone(),
                            });
                        }

                        let new_title = NewTitle {
                            name,
                            facet: facet.clone(),
                            monitored: false,
                            tags: vec![],
                            external_ids,
                            min_availability: None,
                            ..Default::default()
                        };

                        let created = self.add_title(actor, new_title).await?;
                        let index = existing_titles.len();
                        existing_titles.push(created.clone());
                        index_movie_title(
                            &created,
                            index,
                            &mut existing_titles_by_name,
                            &mut existing_titles_by_tvdb_id,
                            &mut existing_titles_by_imdb_id,
                            &mut existing_titles_by_tmdb_id,
                        );
                        created
                    };

                    summary.matched += 1;
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    continue;
                }

                let query = candidate.query.clone();
                let year_hint = candidate.year_hint;

                if let Some(parsed_imdb_id) = parsed_release
                    .imdb_id
                    .as_deref()
                    .and_then(crate::normalize::normalize_imdb_id)
                    && let Some(&index) = existing_titles_by_imdb_id.get(&parsed_imdb_id)
                {
                    summary.matched += 1;
                    let title = existing_titles[index].clone();
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    continue;
                }

                if let Some(parsed_tmdb_id) = parsed_release.tmdb_id.map(|id| id.to_string())
                    && let Some(&index) = existing_titles_by_tmdb_id.get(&parsed_tmdb_id)
                {
                    summary.matched += 1;
                    let title = existing_titles[index].clone();
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    continue;
                }

                if let Some(index) = candidate.query_variants.iter().find_map(|query_variant| {
                    let normalized = normalize_title_key(query_variant);
                    existing_titles_by_name
                        .get(&normalized)
                        .copied()
                        .filter(|index| {
                            let title = &existing_titles[*index];
                            year_hint.is_none()
                                || title.year.map(|value| value as u32) == year_hint
                                || title.year.is_none()
                        })
                }) {
                    summary.matched += 1;
                    let title = existing_titles[index].clone();
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    continue;
                }

                if query.is_empty() {
                    summary.skipped += 1;
                    continue;
                }

                let Some(selected) = candidate.selected_metadata.clone() else {
                    summary.unmatched += 1;
                    continue;
                };

                summary.matched += 1;

                let key = normalize_title_key(&selected.name);
                let title = if let Some(index) = existing_titles_by_tvdb_id
                    .get(&selected.tvdb_id)
                    .copied()
                    .or_else(|| existing_titles_by_name.get(&key).copied())
                {
                    existing_titles[index].clone()
                } else {
                    let new_title = NewTitle {
                        name: selected.name.clone(),
                        facet: facet.clone(),
                        monitored: false,
                        tags: vec![],
                        external_ids: vec![ExternalId {
                            source: "tvdb".into(),
                            value: selected.tvdb_id.clone(),
                        }],
                        min_availability: None,
                        ..Default::default()
                    };

                    let title = self.add_title(actor, new_title).await?;
                    let index = existing_titles.len();
                    existing_titles.push(title.clone());
                    index_movie_title(
                        &title,
                        index,
                        &mut existing_titles_by_name,
                        &mut existing_titles_by_tvdb_id,
                        &mut existing_titles_by_imdb_id,
                        &mut existing_titles_by_tmdb_id,
                    );
                    title
                };

                self.track_movie_file_in_collection(&title, file, &mut summary)
                    .await;
            }
        }

        info!(
            path = %library_path,
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups,
            batch_size = LIBRARY_SCAN_BATCH_SIZE,
            worker_concurrency = LIBRARY_METADATA_LOOKUP_CONCURRENCY,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            "movie library scan completed"
        );

        Ok(summary)
    }

    /// Series/anime library scan: each top-level folder is a potential title.
    /// Episode file linking is handled separately by `scan_title_library()` after
    /// hydration populates Episode records.
    async fn scan_library_series(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_path: &str,
    ) -> AppResult<LibraryScanSummary> {
        let started_at = Instant::now();
        let root = Path::new(library_path);
        if !root.is_dir() {
            return Err(AppError::Validation(format!(
                "library path is not a directory: {library_path}"
            )));
        }

        let folders = list_child_directories(root).await?;
        let folders_count = folders.len();

        let mut existing_titles = self.services.titles.list(Some(facet.clone()), None).await?;
        let mut existing_titles_by_name: HashMap<String, usize> = HashMap::new();
        let mut existing_titles_by_tvdb_id: HashMap<String, usize> = HashMap::new();

        for (index, title) in existing_titles.iter().enumerate() {
            index_series_title(
                title,
                index,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
            );
        }

        let mut summary = LibraryScanSummary::default();
        let mut metadata_lookups = 0usize;
        let mut titles_to_relink = HashMap::new();
        let mut episode_presence_cache = HashMap::new();

        for folder_batch in folders.chunks(LIBRARY_SCAN_BATCH_SIZE) {
            let (candidates, batch_lookups) = preload_series_library_scan_candidates(
                self.services.metadata_gateway.clone(),
                folder_batch,
            )
            .await?;
            metadata_lookups += batch_lookups;

            for candidate in candidates {
                summary.scanned += 1;

                let folder_name = match candidate.folder_name.as_deref() {
                    Some(name) => name.to_string(),
                    None => {
                        summary.skipped += 1;
                        continue;
                    }
                };
                let nfo_meta = candidate.nfo_meta.as_ref();

                if let Some(tvdb_id) = nfo_meta.and_then(|m| m.tvdb_id.as_deref()) {
                    if let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id) {
                        let existing = &mut existing_titles[index];
                        ensure_title_folder_path_if_missing(self, existing, &candidate.folder_path)
                            .await;
                        if should_relink_existing_episodic_title(
                            self,
                            existing,
                            &mut episode_presence_cache,
                        )
                        .await
                        {
                            titles_to_relink.insert(existing.id.clone(), existing.clone());
                        }
                        summary.skipped += 1;
                        continue;
                    }

                    let name = nfo_meta
                        .and_then(|m| m.title.clone())
                        .unwrap_or_else(|| folder_name.clone());
                    let name_key = normalize_title_key(&name);
                    if let Some(&index) = existing_titles_by_name.get(&name_key) {
                        let existing = &mut existing_titles[index];
                        ensure_title_folder_path_if_missing(self, existing, &candidate.folder_path)
                            .await;
                        if should_relink_existing_episodic_title(
                            self,
                            existing,
                            &mut episode_presence_cache,
                        )
                        .await
                        {
                            titles_to_relink.insert(existing.id.clone(), existing.clone());
                        }
                        summary.skipped += 1;
                        continue;
                    }

                    let mut external_ids = vec![ExternalId {
                        source: "tvdb".into(),
                        value: tvdb_id.to_string(),
                    }];
                    if let Some(ref imdb) = nfo_meta.and_then(|m| m.imdb_id.clone()) {
                        external_ids.push(ExternalId {
                            source: "imdb".into(),
                            value: imdb.clone(),
                        });
                    }
                    if let Some(ref tmdb) = nfo_meta.and_then(|m| m.tmdb_id.clone()) {
                        external_ids.push(ExternalId {
                            source: "tmdb".into(),
                            value: tmdb.clone(),
                        });
                    }

                    let new_title = NewTitle {
                        name,
                        facet: facet.clone(),
                        monitored: false,
                        tags: vec![],
                        external_ids,
                        min_availability: None,
                        ..Default::default()
                    };

                    match self.add_title(actor, new_title).await {
                        Ok(mut created) => {
                            ensure_title_folder_path_if_missing(
                                self,
                                &mut created,
                                &candidate.folder_path,
                            )
                            .await;
                            let index = existing_titles.len();
                            existing_titles.push(created.clone());
                            index_series_title(
                                &created,
                                index,
                                &mut existing_titles_by_name,
                                &mut existing_titles_by_tvdb_id,
                            );
                            summary.imported += 1;
                        }
                        Err(error) => {
                            warn!(
                                folder = %folder_name,
                                tvdb_id = %tvdb_id,
                                error = %error,
                                "series scan: failed to create title from NFO"
                            );
                            summary.unmatched += 1;
                        }
                    }
                    continue;
                }

                let query = candidate.query.clone();

                if query.is_empty() {
                    summary.skipped += 1;
                    continue;
                }

                let name_key = normalize_title_key(&query);
                if let Some(&index) = existing_titles_by_name.get(&name_key) {
                    let existing = &mut existing_titles[index];
                    ensure_title_folder_path_if_missing(self, existing, &candidate.folder_path)
                        .await;
                    if should_relink_existing_episodic_title(
                        self,
                        existing,
                        &mut episode_presence_cache,
                    )
                    .await
                    {
                        titles_to_relink.insert(existing.id.clone(), existing.clone());
                    }
                    summary.skipped += 1;
                    continue;
                }

                if let Some(error) = candidate.metadata_lookup_error.as_deref() {
                    warn!(
                        folder = %folder_name,
                        query = %query,
                        error = %error,
                        "series scan: metadata search failed"
                    );
                    summary.unmatched += 1;
                    continue;
                }

                let Some(selected) = candidate.selected_metadata.clone() else {
                    info!(
                        folder = %folder_name,
                        query = %query,
                        "series scan: no metadata match"
                    );
                    summary.unmatched += 1;
                    continue;
                };

                if let Some(&index) = existing_titles_by_tvdb_id.get(&selected.tvdb_id) {
                    let existing = &mut existing_titles[index];
                    ensure_title_folder_path_if_missing(self, existing, &candidate.folder_path)
                        .await;
                    if should_relink_existing_episodic_title(
                        self,
                        existing,
                        &mut episode_presence_cache,
                    )
                    .await
                    {
                        titles_to_relink.insert(existing.id.clone(), existing.clone());
                    }
                    summary.skipped += 1;
                    continue;
                }

                let new_title = NewTitle {
                    name: selected.name.clone(),
                    facet: facet.clone(),
                    monitored: false,
                    tags: vec![],
                    external_ids: vec![ExternalId {
                        source: "tvdb".into(),
                        value: selected.tvdb_id.clone(),
                    }],
                    min_availability: None,
                    ..Default::default()
                };

                match self.add_title(actor, new_title).await {
                    Ok(mut created) => {
                        ensure_title_folder_path_if_missing(
                            self,
                            &mut created,
                            &candidate.folder_path,
                        )
                        .await;
                        let index = existing_titles.len();
                        existing_titles.push(created.clone());
                        index_series_title(
                            &created,
                            index,
                            &mut existing_titles_by_name,
                            &mut existing_titles_by_tvdb_id,
                        );
                        summary.imported += 1;
                    }
                    Err(error) => {
                        warn!(
                            folder = %folder_name,
                            tvdb_id = %selected.tvdb_id,
                            error = %error,
                            "series scan: failed to create title from search"
                        );
                        summary.unmatched += 1;
                    }
                }
            }
        }

        // Keep the updated in-memory Title snapshot so the relink pass sees the
        // discovered folder_path without depending on an immediate re-fetch.
        let mut titles_to_relink = titles_to_relink.into_values().collect::<Vec<_>>();
        titles_to_relink.sort_by(|left, right| left.id.cmp(&right.id));
        for title in titles_to_relink {
            let title_id = title.id.clone();
            if let Err(error) = self.scan_title_library_for_title(actor, title).await {
                warn!(
                    error = %error,
                    title_id = %title_id,
                    facet = facet.as_str(),
                    "failed to relink existing episodic title during library scan"
                );
            }
        }

        info!(
            path = %library_path,
            facet = facet.as_str(),
            folders = folders_count,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups,
            batch_size = LIBRARY_SCAN_BATCH_SIZE,
            worker_concurrency = LIBRARY_METADATA_LOOKUP_CONCURRENCY,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            "series library scan completed"
        );

        Ok(summary)
    }

    pub async fn scan_title_library(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;
        let title = self
            .services
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;

        self.scan_title_library_for_title(actor, title).await
    }

    pub(crate) async fn scan_title_library_for_title(
        &self,
        actor: &User,
        title: Title,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;
        let started_at = Instant::now();

        let handler = self.facet_registry.get(&title.facet).ok_or_else(|| {
            AppError::Validation("library scan is not supported for this facet".into())
        })?;
        if !handler.has_episodes() {
            return Err(AppError::Validation(
                "title library scan is only supported for episodic titles".into(),
            ));
        }

        let (media_root, _) = crate::app_usecase_import::resolve_import_paths(self, &title).await?;
        let title_dir = title
            .folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&media_root).join(&title.name));
        let title_dir_str = title_dir.to_string_lossy().to_string();

        // If the title directory was deleted, recreate it and treat as empty.
        if !title_dir.exists() {
            tokio::fs::create_dir_all(&title_dir).await.map_err(|err| {
                AppError::Repository(format!(
                    "failed to recreate title directory {}: {err}",
                    title_dir.display()
                ))
            })?;
        }

        let mut file_batches = self
            .services
            .library_scanner
            .scan_directory_batched(&title_dir_str, TITLE_SCAN_FILE_BATCH_SIZE)
            .await?;

        let existing_files = self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let collections = self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let title_episodes = self
            .services
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episode_lookup = build_title_episode_lookup(&collections, &title_episodes);

        let mut existing_records_by_path: HashMap<String, TitleMediaFile> = HashMap::new();
        let mut episode_links: HashSet<(String, String)> = HashSet::new();

        for file in &existing_files {
            existing_records_by_path
                .entry(file.file_path.clone())
                .or_insert_with(|| file.clone());
            if let Some(episode_id) = file.episode_id.as_ref() {
                episode_links.insert((file.id.clone(), episode_id.clone()));
            }
        }
        let mut remaining_existing_paths = existing_records_by_path
            .keys()
            .cloned()
            .collect::<HashSet<_>>();

        let mut summary = LibraryScanSummary::default();
        let mut layout_summary = TitleScanLayoutSummary::default();
        let analysis_limit = Arc::new(tokio::sync::Semaphore::new(TITLE_SCAN_ANALYSIS_CONCURRENCY));
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;

        while let Some(file_batch) = file_batches.recv().await {
            let files = file_batch?;
            let mut planned_files = Vec::new();

            for file in files {
                remaining_existing_paths.remove(&file.path);
                summary.scanned += 1;

                let source_path = Path::new(&file.path);
                let parsed = parse_release_metadata(
                    source_path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or(file.display_name.as_str()),
                );

                let ep_meta = match parsed.episode.as_ref() {
                    Some(ep) if !ep.episode_numbers.is_empty() => ep,
                    Some(ep) if ep.air_date.is_some() => ep,
                    Some(ep)
                        if ep.absolute_episode.is_some()
                            && title.facet == scryer_domain::MediaFacet::Anime =>
                    {
                        ep
                    }
                    _ => {
                        summary.unmatched += 1;
                        continue;
                    }
                };

                let season_str = ep_meta.season.unwrap_or(1).to_string();
                let target_episodes =
                    resolve_target_episodes_from_lookup(ep_meta, &season_str, &episode_lookup);

                if target_episodes.is_empty() {
                    summary.unmatched += 1;
                    continue;
                }

                let metadata = match tokio::fs::metadata(source_path).await {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file.path,
                            "failed to read file metadata during title scan"
                        );
                        summary.skipped += 1;
                        continue;
                    }
                };

                let snapshot = FileSourceSnapshot {
                    size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                    signature: file_source_signature_from_metadata(&metadata),
                };

                summary.matched += 1;
                let layout_observation =
                    classify_title_scan_layout(&title_dir, source_path, &target_episodes);
                layout_summary.observe(layout_observation);

                let record = if let Some(existing) = existing_records_by_path.get(&file.path) {
                    let desired_scheme = snapshot
                        .signature
                        .as_ref()
                        .map(|value| value.scheme.clone());
                    let desired_value =
                        snapshot.signature.as_ref().map(|value| value.value.clone());
                    PlannedTitleScanRecord::Existing {
                        file_id: existing.id.clone(),
                        should_skip_analysis: title_media_file_matches_snapshot(
                            existing, &snapshot,
                        ),
                        should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                            || existing.source_signature_scheme != desired_scheme
                            || existing.source_signature_value != desired_value
                            || existing.scan_status != "scanned",
                    }
                } else {
                    PlannedTitleScanRecord::New
                };

                planned_files.push(PlannedTitleScanFile {
                    file,
                    parsed,
                    target_episodes,
                    snapshot,
                    record,
                });
            }

            planned_files.sort_by(|left, right| left.file.path.cmp(&right.file.path));

            let mut analysis_set = tokio::task::JoinSet::new();
            for plan in &planned_files {
                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
                    continue;
                }

                analyzed_files += 1;
                let analyzer = self.services.media_analyzer.clone();
                let analysis_limit = analysis_limit.clone();
                let file_path = plan.file.path.clone();
                analysis_set.spawn(async move {
                    let _permit = analysis_limit
                        .acquire_owned()
                        .await
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                    let outcome = analyzer.analyze_file(PathBuf::from(&file_path)).await?;
                    Ok::<(String, MediaAnalysisOutcome), AppError>((file_path, outcome))
                });
            }

            let mut analysis_results = HashMap::new();
            while let Some(result) = analysis_set.join_next().await {
                let (file_path, outcome) =
                    result.map_err(|error| AppError::Repository(error.to_string()))??;
                analysis_results.insert(file_path, outcome);
            }

            for plan in planned_files {
                let source_signature_scheme = plan
                    .snapshot
                    .signature
                    .as_ref()
                    .map(|signature| signature.scheme.clone());
                let source_signature_value = plan
                    .snapshot
                    .signature
                    .as_ref()
                    .map(|signature| signature.value.clone());

                let file_id = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        file_id,
                        should_refresh_source_signature,
                        ..
                    } => {
                        summary.skipped += 1;
                        if *should_refresh_source_signature
                            && let Err(error) = self
                                .services
                                .media_files
                                .update_media_file_source_signature(
                                    file_id,
                                    plan.snapshot.size_bytes,
                                    source_signature_scheme.clone(),
                                    source_signature_value.clone(),
                                )
                                .await
                        {
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_id = %file_id,
                                "failed to refresh media file source signature during title scan"
                            );
                        }
                        file_id.clone()
                    }
                    PlannedTitleScanRecord::New => {
                        let media_file_input = crate::InsertMediaFileInput {
                            title_id: title.id.clone(),
                            file_path: plan.file.path.clone(),
                            size_bytes: plan.snapshot.size_bytes,
                            source_signature_scheme,
                            source_signature_value,
                            quality_label: plan.parsed.quality.clone(),
                            scene_name: Some(plan.parsed.raw_title.clone()),
                            release_group: plan.parsed.release_group.clone(),
                            source_type: plan.parsed.source.clone(),
                            resolution: plan.parsed.quality.clone(),
                            video_codec_parsed: plan.parsed.video_codec.clone(),
                            audio_codec_parsed: plan.parsed.audio.clone(),
                            ..Default::default()
                        };

                        match self
                            .services
                            .media_files
                            .insert_media_file(&media_file_input)
                            .await
                        {
                            Ok(file_id) => {
                                summary.imported += 1;
                                file_id
                            }
                            Err(error) => {
                                warn!(
                                    error = %error,
                                    title_id = %title.id,
                                    file_path = %plan.file.path,
                                    "failed to insert media file during title scan"
                                );
                                summary.skipped += 1;
                                continue;
                            }
                        }
                    }
                };

                for episode in &plan.target_episodes {
                    if episode_links.insert((file_id.clone(), episode.id.clone()))
                        && let Err(error) = self
                            .services
                            .media_files
                            .link_file_to_episode(&file_id, &episode.id)
                            .await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            episode_id = %episode.id,
                            file_id = %file_id,
                            "failed to link scanned file to episode"
                        );
                    }
                    crate::app_usecase_import::mark_wanted_completed(
                        self,
                        &title.id,
                        Some(&episode.id),
                        None,
                    )
                    .await;
                }

                if let Some(outcome) = analysis_results.remove(&plan.file.path) {
                    match outcome {
                        MediaAnalysisOutcome::Valid(analysis) => {
                            if let Err(error) = self
                                .services
                                .media_files
                                .update_media_file_analysis(&file_id, *analysis)
                                .await
                            {
                                warn!(
                                    error = %error,
                                    title_id = %title.id,
                                    file_id = %file_id,
                                    "failed to persist scanned media analysis"
                                );
                            }
                        }
                        MediaAnalysisOutcome::Invalid(error_message) => {
                            if let Err(error) = self
                                .services
                                .media_files
                                .mark_scan_failed(&file_id, &error_message)
                                .await
                            {
                                warn!(
                                    error = %error,
                                    title_id = %title.id,
                                    file_id = %file_id,
                                    "failed to mark scanned media analysis failure"
                                );
                            }
                        }
                    }
                }
            }
        }

        for stale_path in remaining_existing_paths {
            let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                continue;
            };
            if !stale_path.starts_with(title_dir_str.as_str())
                || Path::new(&record.file_path).exists()
            {
                continue;
            }
            if let Err(error) = self
                .services
                .media_files
                .delete_media_file(&record.id)
                .await
            {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %record.file_path,
                    "failed to delete stale media file during title scan"
                );
            }
        }

        if title.folder_path.as_deref() != Some(title_dir_str.as_str()) {
            self.services
                .titles
                .set_folder_path(&title.id, &title_dir_str)
                .await?;
        }

        if let Some(use_season_folders) = layout_summary.inferred_use_season_folders()
            && crate::app_usecase_import::use_season_folders(&title) != use_season_folders
        {
            let tags = merge_title_scan_option_tags(title.tags.clone(), use_season_folders);
            self.update_title_metadata(actor, &title.id, None, None, Some(tags))
                .await?;
        }

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                EventType::ActionCompleted,
                format!(
                    "title scan completed: {} imported, {} skipped, {} unmatched",
                    summary.imported, summary.skipped, summary.unmatched
                ),
            )
            .await?;

        info!(
            title_id = %title.id,
            path = %title_dir.display(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            analyzed_files,
            unchanged_file_skips,
            batch_size = TITLE_SCAN_FILE_BATCH_SIZE,
            worker_concurrency = TITLE_SCAN_ANALYSIS_CONCURRENCY,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            "title library scan completed"
        );

        Ok(summary)
    }

    /// Track a discovered movie file as a collection entry for the given title.
    /// Skips if the file path is already tracked. Increments `summary.imported` or
    /// `summary.skipped` accordingly.
    async fn track_movie_file_in_collection(
        &self,
        title: &Title,
        file: &LibraryFile,
        summary: &mut LibraryScanSummary,
    ) {
        let collections = match self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(c) => c,
            Err(err) => {
                info!(title_id = %title.id, error = %err, "failed to list collections during scan");
                return;
            }
        };

        let already_tracked = collections.iter().any(|collection| {
            collection
                .ordered_path
                .as_deref()
                .is_some_and(|path| path == file.path)
        });

        if already_tracked {
            summary.skipped += 1;
            return;
        }

        let next_collection_index = collections
            .iter()
            .filter_map(|collection| collection.collection_index.parse::<u32>().ok())
            .max()
            .map_or(1, |max| max + 1);

        let file_stem = Path::new(&file.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let parsed = parse_release_metadata(file_stem);
        let quality_label = parsed.quality.as_ref().filter(|q| !q.is_empty()).cloned();

        let collection = Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: next_collection_index.to_string(),
            label: quality_label,
            ordered_path: Some(file.path.clone()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: title.monitored,
            created_at: Utc::now(),
        };

        if let Err(err) = self.services.shows.create_collection(collection).await {
            info!(
                title_id = %title.id,
                path = %file.path,
                error = %err,
                "failed to create collection for library file"
            );
        }

        summary.imported += 1;
    }

    async fn read_rename_template(&self, handler: &dyn crate::FacetHandler) -> AppResult<String> {
        if let Some(scoped) = self
            .read_setting_string_value(RENAME_TEMPLATE_KEY, Some(handler.rename_scope_id()))
            .await?
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(scoped);
        }

        if let Some(global) = self
            .read_setting_string_value(handler.rename_template_key(), None)
            .await?
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(global);
        }

        Ok(handler.default_rename_template().to_string())
    }

    async fn read_collision_policy(
        &self,
        handler: &dyn crate::FacetHandler,
    ) -> AppResult<RenameCollisionPolicy> {
        let scoped = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_KEY, Some(handler.rename_scope_id()))
            .await?;
        if let Some(value) = scoped
            && let Some(policy) = parse_collision_policy(&value)
        {
            return Ok(policy);
        }

        let global = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_GLOBAL_KEY, None)
            .await?;
        if let Some(value) = global
            && let Some(policy) = parse_collision_policy(&value)
        {
            return Ok(policy);
        }

        let global = self
            .read_setting_string_value(handler.collision_policy_key(), None)
            .await?;
        if let Some(value) = global
            && let Some(policy) = parse_collision_policy(&value)
        {
            return Ok(policy);
        }

        Ok(DEFAULT_COLLISION_POLICY)
    }

    async fn read_missing_metadata_policy(
        &self,
        handler: &dyn crate::FacetHandler,
    ) -> AppResult<RenameMissingMetadataPolicy> {
        let scoped = self
            .read_setting_string_value(
                RENAME_MISSING_METADATA_POLICY_KEY,
                Some(handler.rename_scope_id()),
            )
            .await?;
        if let Some(value) = scoped
            && let Some(policy) = parse_missing_metadata_policy(&value)
        {
            return Ok(policy);
        }

        let global = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, None)
            .await?;
        if let Some(value) = global
            && let Some(policy) = parse_missing_metadata_policy(&value)
        {
            return Ok(policy);
        }

        let global = self
            .read_setting_string_value(handler.missing_metadata_policy_key(), None)
            .await?;
        if let Some(value) = global
            && let Some(policy) = parse_missing_metadata_policy(&value)
        {
            return Ok(policy);
        }

        Ok(DEFAULT_MISSING_METADATA_POLICY)
    }
}

#[cfg(test)]
mod scan_layout_tests {
    use super::*;

    fn test_episode(season_number: &str) -> Episode {
        Episode {
            id: "episode-1".into(),
            title_id: "title-1".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some(season_number.into()),
            episode_label: Some("S01E01".into()),
            title: Some("Pilot".into()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn recognize_season_folder_name_accepts_common_variants() {
        assert_eq!(recognize_season_folder_name("Season 01"), Some(1));
        assert_eq!(recognize_season_folder_name("Series_1"), Some(1));
        assert_eq!(recognize_season_folder_name("S01"), Some(1));
        assert_eq!(recognize_season_folder_name("Season-00"), Some(0));
        assert_eq!(recognize_season_folder_name("Special Episodes"), Some(0));
        assert_eq!(recognize_season_folder_name("specials"), Some(0));
        assert_eq!(recognize_season_folder_name("Extras"), None);
    }

    #[test]
    fn classify_title_scan_layout_marks_conflicting_season_folders_ambiguous() {
        let title_dir = PathBuf::from("/library/Example Show");
        let file_path = title_dir.join("Series 02/Example.Show.S01E01.mkv");
        let target_episodes = vec![test_episode("1")];

        assert_eq!(
            classify_title_scan_layout(&title_dir, &file_path, &target_episodes),
            TitleScanLayoutObservation::Ambiguous
        );
    }
}

fn select_best_match(
    results: &[MetadataSearchItem],
    year: Option<u32>,
    normalized_title_candidates: &[String],
) -> Option<MetadataSearchItem> {
    if results.is_empty() {
        return None;
    }

    let exact_title_matches = results
        .iter()
        .filter(|item| {
            let normalized = crate::app_usecase_rss::normalize_for_matching(&item.name);
            !normalized.is_empty()
                && normalized_title_candidates
                    .iter()
                    .any(|candidate| candidate == &normalized)
        })
        .collect::<Vec<_>>();

    if exact_title_matches.is_empty() {
        return None;
    }

    if let Some(year) = year.map(|value| value as i32)
        && let Some(match_item) = exact_title_matches
            .iter()
            .find(|item| item.year == Some(year))
    {
        return Some((*match_item).clone());
    }

    exact_title_matches.into_iter().next().cloned()
}

fn normalize_title_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn parse_collision_policy(raw: &str) -> Option<RenameCollisionPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameCollisionPolicy::Skip),
        "error" => Some(RenameCollisionPolicy::Error),
        "replace_if_better" => Some(RenameCollisionPolicy::ReplaceIfBetter),
        _ => None,
    }
}

fn parse_missing_metadata_policy(raw: &str) -> Option<RenameMissingMetadataPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameMissingMetadataPolicy::Skip),
        "fallback_title" => Some(RenameMissingMetadataPolicy::FallbackTitle),
        _ => None,
    }
}

fn build_rename_plan_for_facet(
    handler: &dyn crate::FacetHandler,
    title: &Title,
    mut collections: Vec<Collection>,
    template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
) -> RenamePlan {
    collections.sort_by(|left, right| left.id.cmp(&right.id));

    let mut planned_targets = HashSet::new();
    let mut items = Vec::with_capacity(collections.len());

    for collection in collections {
        let item = handler.build_rename_plan_item(
            title,
            &collection,
            &template,
            &collision_policy,
            &missing_metadata_policy,
            &mut planned_targets,
        );
        items.push(item);
    }

    build_rename_plan_from_items(
        handler.facet(),
        Some(title.id.clone()),
        template,
        collision_policy,
        missing_metadata_policy,
        items,
    )
}

fn build_rename_plan_from_items(
    facet: MediaFacet,
    title_id: Option<String>,
    template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
    items: Vec<RenamePlanItem>,
) -> RenamePlan {
    let total = items.len();
    let renamable = items
        .iter()
        .filter(|item| {
            matches!(
                item.write_action,
                RenameWriteAction::Move | RenameWriteAction::Replace
            )
        })
        .count();
    let noop = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Noop))
        .count();
    let conflicts = items.iter().filter(|item| item.collision).count();
    let errors = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Error))
        .count();

    let fingerprint = build_rename_plan_fingerprint(
        &items,
        &template,
        &collision_policy,
        &missing_metadata_policy,
    );

    RenamePlan {
        facet,
        title_id,
        template,
        collision_policy,
        missing_metadata_policy,
        fingerprint,
        total,
        renamable,
        noop,
        conflicts,
        errors,
        items,
    }
}

pub(crate) fn build_movie_rename_plan_item(
    title: &Title,
    collection: &Collection,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let Some(current_path) = collection.ordered_path.clone() else {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path: String::new(),
            proposed_path: None,
            normalized_filename: None,
            collision: false,
            reason_code: "no_source_path".into(),
            write_action: RenameWriteAction::Skip,
            source_size_bytes: None,
            source_mtime_unix_ms: None,
        };
    };

    let current_file = Path::new(&current_path);
    let source_metadata = std::fs::metadata(current_file).ok();
    let source_size_bytes = source_metadata.as_ref().map(|meta| meta.len());
    let source_mtime_unix_ms = source_metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());

    if source_metadata.as_ref().is_none_or(|meta| !meta.is_file()) {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: None,
            normalized_filename: None,
            collision: false,
            reason_code: "source_not_file".into(),
            write_action: RenameWriteAction::Error,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    let current_stem = current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let extension = current_file
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let quality = collection
        .label
        .clone()
        .or(parsed.quality.clone())
        .unwrap_or_default();

    let mut tokens = BTreeMap::new();
    tokens.insert("title".to_string(), title_token.clone());
    tokens.insert("year".to_string(), year_token.unwrap_or_default());
    tokens.insert("quality".to_string(), quality);
    tokens.insert(
        "edition".to_string(),
        parsed
            .parse_hints
            .iter()
            .find(|hint| hint.to_ascii_lowercase().contains("edition"))
            .cloned()
            .unwrap_or_default(),
    );
    tokens.insert(
        "source".to_string(),
        parsed.source.clone().unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed.video_codec.clone().unwrap_or_default(),
    );
    tokens.insert(
        "audio_codec".to_string(),
        parsed.audio.clone().unwrap_or_default(),
    );
    tokens.insert(
        "audio_channels".to_string(),
        parsed.audio_channels.clone().unwrap_or_default(),
    );
    tokens.insert(
        "group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert("ext".to_string(), extension.clone());

    let mut rendered = render_rename_template(template, &tokens);
    if rendered.is_empty() {
        if matches!(missing_metadata_policy, RenameMissingMetadataPolicy::Skip) {
            return RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: None,
                normalized_filename: None,
                collision: false,
                reason_code: "missing_metadata".into(),
                write_action: RenameWriteAction::Skip,
                source_size_bytes,
                source_mtime_unix_ms,
            };
        }
        rendered = split_title_and_year_hint(&title.name).0;
    }

    if !extension.is_empty()
        && !rendered
            .to_ascii_lowercase()
            .ends_with(&format!(".{extension}"))
    {
        rendered = format!("{rendered}.{extension}");
    }

    let parent = current_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let proposed_path = parent.join(&rendered);
    let proposed_path_str = proposed_path.to_string_lossy().to_string();

    if proposed_path_str == current_path {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: Some(proposed_path_str),
            normalized_filename: Some(rendered),
            collision: false,
            reason_code: "same_path".into(),
            write_action: RenameWriteAction::Noop,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    if !planned_targets.insert(proposed_path_str.clone()) {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: Some(proposed_path_str),
            normalized_filename: Some(rendered),
            collision: true,
            reason_code: "collision_within_plan".into(),
            write_action: RenameWriteAction::Skip,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    if Path::new(&proposed_path_str).exists() {
        return match collision_policy {
            RenameCollisionPolicy::Skip => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Skip,
                source_size_bytes,
                source_mtime_unix_ms,
            },
            RenameCollisionPolicy::Error => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Error,
                source_size_bytes,
                source_mtime_unix_ms,
            },
            RenameCollisionPolicy::ReplaceIfBetter => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_replace".into(),
                write_action: RenameWriteAction::Replace,
                source_size_bytes,
                source_mtime_unix_ms,
            },
        };
    }

    RenamePlanItem {
        collection_id: Some(collection.id.clone()),
        current_path,
        proposed_path: Some(proposed_path_str),
        normalized_filename: Some(rendered),
        collision: false,
        reason_code: "rename_move".into(),
        write_action: RenameWriteAction::Move,
        source_size_bytes,
        source_mtime_unix_ms,
    }
}

pub(crate) fn build_series_rename_plan_item(
    title: &Title,
    collection: &Collection,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let Some(current_path) = collection.ordered_path.clone() else {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path: String::new(),
            proposed_path: None,
            normalized_filename: None,
            collision: false,
            reason_code: "no_source_path".into(),
            write_action: RenameWriteAction::Skip,
            source_size_bytes: None,
            source_mtime_unix_ms: None,
        };
    };

    let current_file = Path::new(&current_path);
    let source_metadata = std::fs::metadata(current_file).ok();
    let source_size_bytes = source_metadata.as_ref().map(|meta| meta.len());
    let source_mtime_unix_ms = source_metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());

    if source_metadata.as_ref().is_none_or(|meta| !meta.is_file()) {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: None,
            normalized_filename: None,
            collision: false,
            reason_code: "source_not_file".into(),
            write_action: RenameWriteAction::Error,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    let current_stem = current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let extension = current_file
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let quality = collection
        .label
        .clone()
        .or(parsed.quality.clone())
        .unwrap_or_default();

    // For interstitial collections (anime franchise movies in Season 00),
    // use the stored season/episode from interstitial_season_episode and the movie name.
    let (season, episode, episode_title_override) =
        if collection.collection_type == CollectionType::Interstitial {
            if let Some(ref se) = collection.interstitial_season_episode {
                // Parse "S00E03" → season "0", episode "3"
                let (s, e) = se
                    .strip_prefix('S')
                    .and_then(|rest| rest.split_once('E'))
                    .map(|(s, e)| {
                        (
                            s.trim_start_matches('0').to_string(),
                            e.trim_start_matches('0').to_string(),
                        )
                    })
                    .unwrap_or_else(|| ("0".to_string(), "1".to_string()));
                let movie_name = collection
                    .interstitial_movie
                    .as_ref()
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
                (s, e, Some(movie_name))
            } else {
                ("0".to_string(), "1".to_string(), None)
            }
        } else {
            // Season from collection_index, episode from collection's first_episode_number,
            // falling back to parsed release metadata.
            let season = collection.collection_index.clone();
            let episode = collection
                .first_episode_number
                .clone()
                .or_else(|| {
                    parsed
                        .episode
                        .as_ref()
                        .and_then(|ep| ep.episode_numbers.first())
                        .map(|n| n.to_string())
                })
                .unwrap_or_default();
            (season, episode, None)
        };

    let mut tokens = BTreeMap::new();
    tokens.insert("title".to_string(), title_token.clone());
    tokens.insert("year".to_string(), year_token.unwrap_or_default());
    tokens.insert("season".to_string(), season);
    tokens.insert(
        "season_order".to_string(),
        collection
            .narrative_order
            .clone()
            .unwrap_or_else(|| collection.collection_index.clone()),
    );
    tokens.insert("episode".to_string(), episode.clone());
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|ep| ep.absolute_episode)
            .map(|n| format!("{:0>3}", n))
            .unwrap_or_else(|| episode.clone()),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title_override.unwrap_or_default(),
    );
    tokens.insert("quality".to_string(), quality);
    tokens.insert(
        "source".to_string(),
        parsed.source.clone().unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed.video_codec.clone().unwrap_or_default(),
    );
    tokens.insert(
        "audio_codec".to_string(),
        parsed.audio.clone().unwrap_or_default(),
    );
    tokens.insert(
        "audio_channels".to_string(),
        parsed.audio_channels.clone().unwrap_or_default(),
    );
    tokens.insert(
        "group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert("ext".to_string(), extension.clone());

    let mut rendered = render_rename_template(template, &tokens);
    if rendered.is_empty() {
        if matches!(missing_metadata_policy, RenameMissingMetadataPolicy::Skip) {
            return RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: None,
                normalized_filename: None,
                collision: false,
                reason_code: "missing_metadata".into(),
                write_action: RenameWriteAction::Skip,
                source_size_bytes,
                source_mtime_unix_ms,
            };
        }
        rendered = split_title_and_year_hint(&title.name).0;
    }

    if !extension.is_empty()
        && !rendered
            .to_ascii_lowercase()
            .ends_with(&format!(".{extension}"))
    {
        rendered = format!("{rendered}.{extension}");
    }

    let parent = current_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let proposed_path = parent.join(&rendered);
    let proposed_path_str = proposed_path.to_string_lossy().to_string();

    if proposed_path_str == current_path {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: Some(proposed_path_str),
            normalized_filename: Some(rendered),
            collision: false,
            reason_code: "same_path".into(),
            write_action: RenameWriteAction::Noop,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    if !planned_targets.insert(proposed_path_str.clone()) {
        return RenamePlanItem {
            collection_id: Some(collection.id.clone()),
            current_path,
            proposed_path: Some(proposed_path_str),
            normalized_filename: Some(rendered),
            collision: true,
            reason_code: "collision_within_plan".into(),
            write_action: RenameWriteAction::Skip,
            source_size_bytes,
            source_mtime_unix_ms,
        };
    }

    if Path::new(&proposed_path_str).exists() {
        return match collision_policy {
            RenameCollisionPolicy::Skip => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Skip,
                source_size_bytes,
                source_mtime_unix_ms,
            },
            RenameCollisionPolicy::Error => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Error,
                source_size_bytes,
                source_mtime_unix_ms,
            },
            RenameCollisionPolicy::ReplaceIfBetter => RenamePlanItem {
                collection_id: Some(collection.id.clone()),
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_replace".into(),
                write_action: RenameWriteAction::Replace,
                source_size_bytes,
                source_mtime_unix_ms,
            },
        };
    }

    RenamePlanItem {
        collection_id: Some(collection.id.clone()),
        current_path,
        proposed_path: Some(proposed_path_str),
        normalized_filename: Some(rendered),
        collision: false,
        reason_code: "rename_move".into(),
        write_action: RenameWriteAction::Move,
        source_size_bytes,
        source_mtime_unix_ms,
    }
}

fn split_title_and_year_hint(raw_title: &str) -> (String, Option<String>) {
    let trimmed = raw_title.trim();
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(close_pos) = trimmed.rfind(close)
            && let Some(open_pos) = trimmed[..close_pos].rfind(open)
        {
            let candidate = trimmed[open_pos + 1..close_pos].trim();
            if candidate.len() == 4 && candidate.chars().all(|value| value.is_ascii_digit()) {
                let title = trimmed[..open_pos].trim().to_string();
                if !title.is_empty() {
                    return (title, Some(candidate.to_string()));
                }
            }
        }
    }

    (trimmed.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_media_file(
        size_bytes: i64,
        source_signature_scheme: Option<&str>,
        source_signature_value: Option<&str>,
    ) -> TitleMediaFile {
        TitleMediaFile {
            id: "file-1".into(),
            title_id: "title-1".into(),
            episode_id: Some("episode-1".into()),
            file_path: "/library/Show/Season 01/Show.S01E01.mkv".into(),
            size_bytes,
            source_signature_scheme: source_signature_scheme.map(str::to_string),
            source_signature_value: source_signature_value.map(str::to_string),
            quality_label: None,
            scan_status: "scanned".into(),
            created_at: String::new(),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: vec![],
            audio_streams: vec![],
            subtitle_languages: vec![],
            subtitle_codecs: vec![],
            subtitle_streams: vec![],
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: None,
            release_group: None,
            source_type: None,
            resolution: None,
            video_codec_parsed: None,
            audio_codec_parsed: None,
            acquisition_score: None,
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        }
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
    async fn read_valid_movie_nfo_metadata_rejects_tvshow_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<tvshow><title>Bluey</title><tvdbid>81189</tvdbid></tvshow>"#,
        )
        .expect("write nfo");

        assert!(
            read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
                .await
                .is_none()
        );
    }

    #[test]
    fn file_source_signature_uses_platform_scheme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.mkv");
        std::fs::write(&path, b"video").expect("write test file");

        let metadata = std::fs::metadata(&path).expect("metadata");
        let signature = file_source_signature_from_metadata(&metadata).expect("signature");

        #[cfg(unix)]
        assert_eq!(signature.scheme, "unix_mtime_nsec_v1");
        #[cfg(windows)]
        assert_eq!(signature.scheme, "windows_last_write_100ns_v1");
        #[cfg(all(not(unix), not(windows)))]
        assert_eq!(signature.scheme, "system_time_nsec_v1");

        assert!(!signature.value.trim().is_empty());
    }

    #[test]
    fn title_media_file_matches_snapshot_backfills_missing_signatures_without_reanalysis() {
        let media_file = build_test_media_file(1234, None, None);
        let snapshot = FileSourceSnapshot {
            size_bytes: 1234,
            signature: Some(FileSourceSignature {
                scheme: "unix_mtime_nsec_v1".into(),
                value: "1:2".into(),
            }),
        };

        assert!(title_media_file_matches_snapshot(&media_file, &snapshot));
    }

    #[test]
    fn resolve_target_episodes_from_lookup_uses_collection_index_and_preserves_first_duplicate() {
        let collection = Collection {
            id: "collection-2".into(),
            title_id: "title-1".into(),
            collection_type: CollectionType::Season,
            collection_index: "2".into(),
            label: Some("Season 2".into()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".into()),
            last_episode_number: Some("10".into()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let first = Episode {
            id: "episode-a".into(),
            title_id: "title-1".into(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some("1".into()),
            episode_label: Some("S01E01".into()),
            title: Some("First".into()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("101".into()),
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let second = Episode {
            id: "episode-b".into(),
            absolute_number: Some("101".into()),
            ..first.clone()
        };

        let lookup =
            build_title_episode_lookup(std::slice::from_ref(&collection), &[first.clone(), second]);
        let ep_meta = crate::ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![1],
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };

        let episodes = resolve_target_episodes_from_lookup(&ep_meta, "2", &lookup);

        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, first.id);
    }

    #[test]
    fn extract_library_queries_uses_movie_title_variants_for_root_files() {
        let (queries, year) = extract_library_queries(
            "/library/Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.mkv",
            "/library",
        );

        assert_eq!(year, Some(2020));
        assert_eq!(
            queries,
            vec![
                "MON COUSIN AKA MY COUSIN".to_string(),
                "MON COUSIN".to_string(),
                "MY COUSIN".to_string()
            ]
        );
    }

    #[test]
    fn extract_library_queries_prefers_parent_folder_for_nested_movie() {
        let (queries, year) =
            extract_library_queries("/library/My Cousin (2020)/movie.mkv", "/library");

        assert_eq!(queries, vec!["My Cousin".to_string()]);
        assert_eq!(year, Some(2020));
    }

    #[test]
    fn extract_library_queries_prefers_release_year_over_stale_folder_year() {
        let (queries, year) = extract_library_queries(
            "/library/Dune (2020)/Dune.2021.2160p.BluRay.REMUX.HEVC.DTS-HD.MA.TrueHD.7.1.Atmos-FGT.mkv",
            "/library",
        );

        assert_eq!(queries, vec!["Dune".to_string()]);
        assert_eq!(year, Some(2021));
    }

    #[test]
    fn extract_library_queries_keeps_nested_movie_search_grounded_in_folder_title() {
        let (queries, year) = extract_library_queries(
            "/library/Dune (2020)/Dune.Part.Two.2024.2160p.WEB-DL.H265-GRP.mkv",
            "/library",
        );

        assert_eq!(queries, vec!["Dune".to_string()]);
        assert_eq!(year, Some(2024));
    }

    #[test]
    fn select_best_match_prefers_exact_title_and_matching_year() {
        let results = vec![
            MetadataSearchItem {
                tvdb_id: "wrong".into(),
                name: "Dune Drifter".into(),
                year: Some(2020),
            },
            MetadataSearchItem {
                tvdb_id: "right".into(),
                name: "Dune".into(),
                year: Some(2021),
            },
        ];
        let candidates = normalized_query_title_candidates(&["Dune".to_string()]);

        let selected =
            select_best_match(&results, Some(2021), &candidates).expect("exact title match");

        assert_eq!(selected.tvdb_id, "right");
        assert_eq!(selected.name, "Dune");
    }

    #[test]
    fn select_best_match_rejects_non_exact_title_even_with_year_match() {
        let results = vec![MetadataSearchItem {
            tvdb_id: "wrong".into(),
            name: "Dune Drifter".into(),
            year: Some(2020),
        }];
        let candidates = normalized_query_title_candidates(&["Dune".to_string()]);

        assert!(select_best_match(&results, Some(2020), &candidates).is_none());
    }
}
