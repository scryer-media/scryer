use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use super::*;
use crate::library_scan::source_signature_from_std_metadata;
use crate::nfo::{looks_like_movie_nfo, parse_nfo};
use crate::{
    NotificationEnvelope,
    activity::{NotificationMediaUpdate, build_lifecycle_notification_metadata},
};
use scryer_domain::{NotificationEventType, VIDEO_EXTENSIONS};
use tracing::{info, warn};

const METADATA_TYPE_MOVIE: &str = "movie";
const LIBRARY_METADATA_LOOKUP_CONCURRENCY: usize = 4;
const LIBRARY_SCAN_BATCH_SIZE: usize = 128;
const TITLE_SCAN_FILE_BATCH_SIZE: usize = 128;
const TITLE_PRE_SCAN_CONCURRENCY: usize = 16;
const RADARR_MOVIE_NFO_MAX_BYTES: u64 = 10 * 1024 * 1024;
const LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME: &str = "immediate_children_v1";
const LIBRARY_PROBE_SIGNATURE_FILE_SCHEME: &str = "file_snapshot_v1";
const RENAME_TEMPLATE_KEY: &str = "rename.template";
const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const DEFAULT_COLLISION_POLICY: RenameCollisionPolicy = RenameCollisionPolicy::Skip;
const DEFAULT_MISSING_METADATA_POLICY: RenameMissingMetadataPolicy =
    RenameMissingMetadataPolicy::FallbackTitle;

#[derive(Default)]
struct RenamePersistenceState {
    media_file_updated: bool,
}

struct RenamePersistenceFailure {
    error: AppError,
    state: RenamePersistenceState,
}

struct RenameRollbackOutcome {
    fully_restored: bool,
    detail: String,
}

#[derive(Clone, Debug)]
struct MovieTopLevelEntry {
    path: PathBuf,
    is_dir: bool,
}

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

fn elapsed_ms_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn list_child_directories(root: &Path) -> AppResult<Vec<PathBuf>> {
    crate::filesystem_walk::FilesystemWalker::new().list_child_directories(root)
}

async fn list_movie_top_level_entries(root: &Path) -> AppResult<Vec<MovieTopLevelEntry>> {
    let mut entries = tokio::fs::read_dir(root).await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })?;
    let mut results = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })? {
        let path = entry.path();
        let file_type = entry.file_type().await.map_err(|error| {
            AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if file_type.is_dir() {
            results.push(MovieTopLevelEntry { path, is_dir: true });
            continue;
        }

        if file_type.is_file() && is_allowed_video_path(&path) {
            results.push(MovieTopLevelEntry {
                path,
                is_dir: false,
            });
        }
    }

    results.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(results)
}

fn is_allowed_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.as_str()))
}

fn matching_movie_nfo_path(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if same_stem.is_file() {
        return Some(same_stem.to_string_lossy().to_string());
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if movie_nfo.is_file() {
        return Some(movie_nfo.to_string_lossy().to_string());
    }

    None
}

fn derive_movie_probe_path(
    root: &Path,
    title: &Title,
    collections: &[Collection],
) -> Option<PathBuf> {
    if let Some(folder_path) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(folder_path));
    }

    let mut ordered_paths = collections
        .iter()
        .filter_map(|collection| collection.ordered_path.as_deref())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    ordered_paths.sort();
    ordered_paths.dedup();

    let first = ordered_paths.into_iter().next()?;
    if let Some(parent) = first.parent()
        && parent != root
    {
        return Some(parent.to_path_buf());
    }

    Some(first)
}

async fn compute_library_probe_signature(path: &Path) -> AppResult<(String, String)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || compute_library_probe_signature_blocking(path))
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
}

#[derive(Clone, Debug)]
struct PendingLibraryProbe {
    path: String,
    scheme: String,
    value: String,
    now: chrono::DateTime<Utc>,
    stored_probe: Option<LibraryProbeSignature>,
}

enum BackgroundRefreshProbeOutcome<T> {
    Unchanged,
    Changed(T),
}

async fn begin_background_refresh_probe(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
) -> AppResult<Option<PendingLibraryProbe>> {
    let path_string = path.to_string_lossy().to_string();
    let now = Utc::now();
    let (scheme, value) = compute_library_probe_signature(path).await?;
    let stored_probe = app
        .services
        .library_probe_signatures
        .get_probe_signature(title_id)
        .await?;
    let unchanged = stored_probe.as_ref().is_some_and(|probe| {
        probe.path == path_string
            && probe.probe_signature_scheme.as_deref() == Some(scheme.as_str())
            && probe.probe_signature_value.as_deref() == Some(value.as_str())
    });

    if unchanged {
        app.services
            .library_probe_signatures
            .upsert_probe_signature(&LibraryProbeSignature {
                title_id: title_id.to_string(),
                path: path_string,
                probe_signature_scheme: Some(scheme),
                probe_signature_value: Some(value),
                last_probed_at: Some(now),
                last_changed_at: stored_probe.and_then(|probe| probe.last_changed_at),
            })
            .await?;
        return Ok(None);
    }

    Ok(Some(PendingLibraryProbe {
        path: path_string,
        scheme,
        value,
        now,
        stored_probe,
    }))
}

async fn persist_background_refresh_probe_result(
    app: &AppUseCase,
    title_id: &str,
    probe: PendingLibraryProbe,
    has_delta: bool,
) -> AppResult<()> {
    app.services
        .library_probe_signatures
        .upsert_probe_signature(&LibraryProbeSignature {
            title_id: title_id.to_string(),
            path: probe.path,
            probe_signature_scheme: Some(probe.scheme),
            probe_signature_value: Some(probe.value),
            last_probed_at: Some(probe.now),
            last_changed_at: has_delta
                .then_some(probe.now)
                .or_else(|| probe.stored_probe.and_then(|stored| stored.last_changed_at)),
        })
        .await
}

async fn run_background_refresh_probe_with_delta<T, Fut>(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
    scan_and_diff: Fut,
) -> AppResult<BackgroundRefreshProbeOutcome<T>>
where
    Fut: std::future::Future<Output = AppResult<(T, HashSet<String>, HashSet<String>)>>,
{
    let Some(probe) = begin_background_refresh_probe(app, title_id, path).await? else {
        return Ok(BackgroundRefreshProbeOutcome::Unchanged);
    };

    let (payload, discovered_paths, existing_paths) = scan_and_diff.await?;
    let has_delta = discovered_paths != existing_paths;
    persist_background_refresh_probe_result(app, title_id, probe, has_delta).await?;

    if has_delta {
        Ok(BackgroundRefreshProbeOutcome::Changed(payload))
    } else {
        Ok(BackgroundRefreshProbeOutcome::Unchanged)
    }
}

fn compute_library_probe_signature_blocking(path: PathBuf) -> AppResult<(String, String)> {
    let metadata = std::fs::metadata(&path).map_err(|error| {
        AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
    })?;

    if metadata.is_dir() {
        let mut markers = Vec::new();
        let entries = std::fs::read_dir(&path).map_err(|error| {
            AppError::Repository(format!("failed to read {}: {error}", path.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read entry in {}: {error}",
                    path.display()
                ))
            })?;
            let child_path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect filesystem entry {}: {error}",
                    child_path.display()
                ))
            })?;

            let (kind, child_metadata) = if file_type.is_dir() {
                ("dir", std::fs::metadata(&child_path).ok())
            } else if file_type.is_file() {
                ("file", std::fs::metadata(&child_path).ok())
            } else if file_type.is_symlink() {
                match std::fs::metadata(&child_path) {
                    Ok(metadata) if metadata.is_dir() => ("dir", Some(metadata)),
                    Ok(metadata) if metadata.is_file() => ("file", Some(metadata)),
                    _ => continue,
                }
            } else {
                continue;
            };

            let marker = child_metadata
                .as_ref()
                .map(metadata_probe_marker)
                .unwrap_or_else(|| "unknown".to_string());
            let name = child_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            markers.push(format!("{name}|{kind}|{marker}"));
        }
        markers.sort();
        let payload = markers.join("\n");
        Ok((
            LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME.to_string(),
            sha256_hex(payload),
        ))
    } else {
        let payload = metadata_probe_marker(&metadata);
        Ok((
            LIBRARY_PROBE_SIGNATURE_FILE_SCHEME.to_string(),
            sha256_hex(payload),
        ))
    }
}

fn metadata_probe_marker(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| format!("{}:{}", value.as_secs(), value.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{modified}|{}", metadata.len())
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

#[derive(Clone, Copy, Debug, Default)]
struct TitleScanProgressDelta {
    completed: usize,
    failed: usize,
}

impl TitleScanProgressDelta {
    fn completed(count: usize) -> Self {
        Self {
            completed: count,
            failed: 0,
        }
    }

    fn failed(count: usize) -> Self {
        Self {
            completed: 0,
            failed: count,
        }
    }

    fn total(self) -> usize {
        self.completed.saturating_add(self.failed)
    }

    fn absorb(&mut self, other: Self) {
        self.completed = self.completed.saturating_add(other.completed);
        self.failed = self.failed.saturating_add(other.failed);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TitleScanFinalizeOutcome {
    progress: TitleScanProgressDelta,
    title_updated: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackedTitleRetentionPolicy {
    KeepTrackedForDeferredReconcile,
    SkipIfNotRelinkable,
}

enum TrackedTitleInput {
    SchedulePreScan,
    PreScannedFiles(Vec<LibraryFile>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackedTitleWorkflowOutcome {
    SkipCompleteNow,
    SchedulePreScanAndQueue,
    SchedulePreScanOnly,
    QueueWithExistingFiles,
    AttachOnly,
}

async fn track_title_for_library_scan_session(
    app: &AppUseCase,
    session_id: &str,
    title: &Title,
    pre_scanned_files: Option<Vec<LibraryFile>>,
) -> Option<LibraryScanTitleAttachResult> {
    let result = app
        .services
        .library_scan_tracker
        .attach_title(session_id, &title.id, pre_scanned_files)
        .await?;

    if result.new_title && title.metadata_fetched_at.is_none() {
        let _ = app
            .services
            .library_scan_tracker
            .add_metadata_total(session_id, 1)
            .await;
    }

    Some(result)
}

async fn maybe_complete_library_scan_session(app: &AppUseCase, session_id: &str) {
    let _ = app
        .services
        .library_scan_tracker
        .complete_if_finished(session_id)
        .await;
}

async fn prepare_tracked_episodic_title_for_library_scan_session(
    app: &AppUseCase,
    session_id: &str,
    title: &Title,
    tracked_title_input: TrackedTitleInput,
    episode_presence_cache: &mut HashMap<String, bool>,
    retention_policy: TrackedTitleRetentionPolicy,
) -> Option<TrackedTitleWorkflowOutcome> {
    let has_pre_scanned_files =
        matches!(tracked_title_input, TrackedTitleInput::PreScannedFiles(_));
    let pre_scanned_files = match tracked_title_input {
        TrackedTitleInput::SchedulePreScan => None,
        TrackedTitleInput::PreScannedFiles(files) => Some(files),
    };
    let attach_result =
        track_title_for_library_scan_session(app, session_id, title, pre_scanned_files).await?;

    if should_relink_existing_episodic_title(app, title, episode_presence_cache).await {
        return Some(if has_pre_scanned_files {
            TrackedTitleWorkflowOutcome::QueueWithExistingFiles
        } else {
            TrackedTitleWorkflowOutcome::SchedulePreScanAndQueue
        });
    }

    if matches!(
        retention_policy,
        TrackedTitleRetentionPolicy::KeepTrackedForDeferredReconcile
    ) {
        return Some(if has_pre_scanned_files {
            TrackedTitleWorkflowOutcome::AttachOnly
        } else {
            TrackedTitleWorkflowOutcome::SchedulePreScanOnly
        });
    }

    if attach_result.new_title && title.metadata_fetched_at.is_none() {
        let _ = app
            .services
            .library_scan_tracker
            .increment_metadata_completed(session_id, 1)
            .await;
    }

    if attach_result.added_file_count > 0 {
        let _ = app
            .services
            .library_scan_tracker
            .increment_file_completed(session_id, attach_result.added_file_count)
            .await;
    }
    app.services
        .library_scan_tracker
        .release_title(&title.id)
        .await;
    maybe_complete_library_scan_session(app, session_id).await;
    Some(TrackedTitleWorkflowOutcome::SkipCompleteNow)
}

async fn pre_scan_title_for_library_scan_session(
    library_scanner: Arc<dyn LibraryScanner>,
    library_scan_tracker: LibraryScanTracker,
    session_id: String,
    title_id: String,
    folder_path: PathBuf,
) -> bool {
    if library_scan_tracker
        .session_for_title(&title_id)
        .await
        .as_deref()
        != Some(session_id.as_str())
    {
        return false;
    }

    let started_at = Instant::now();
    let folder_path_display = folder_path.display().to_string();
    let folder_path_string = folder_path.to_string_lossy().to_string();
    let pre_scan = match library_scanner
        .scan_directory_for_progress_with_metrics(&folder_path_string)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            library_scan_tracker
                .mark_title_pre_scan_failed(&title_id)
                .await;
            warn!(
                error = %error,
                title_id = %title_id,
                folder_path = %folder_path_display,
                "failed to pre-scan episodic title folder for library scan progress"
            );
            return false;
        }
    };

    if library_scan_tracker
        .session_for_title(&title_id)
        .await
        .as_deref()
        != Some(session_id.as_str())
    {
        return false;
    }

    let pre_scanned_files = pre_scan.files;
    let files_count = pre_scanned_files.len();
    let attached = library_scan_tracker
        .attach_title(&session_id, &title_id, Some(pre_scanned_files))
        .await
        .is_some();

    info!(
        title_id = %title_id,
        path = %folder_path_display,
        files = files_count,
        walk_ms = pre_scan.walk_ms,
        stat_ms = pre_scan.stat_ms,
        analyze_ms = 0u64,
        db_ms = 0u64,
        elapsed_ms = elapsed_ms_u64(started_at),
        "episodic title pre-scan completed"
    );

    attached
}

async fn schedule_title_pre_scan_for_library_scan_session(
    pre_scan_set: &mut tokio::task::JoinSet<()>,
    library_scanner: Arc<dyn LibraryScanner>,
    library_scan_tracker: LibraryScanTracker,
    post_hydration_title_scan_queue: PostHydrationTitleScanQueue,
    pre_scan_limit: Arc<tokio::sync::Semaphore>,
    session_id: &str,
    title_id: &str,
    folder_path: PathBuf,
    queue_after_pre_scan: bool,
) {
    library_scan_tracker
        .mark_title_pre_scan_started(title_id)
        .await;

    let session_id = session_id.to_string();
    let title_id = title_id.to_string();
    pre_scan_set.spawn(async move {
        let Ok(_permit) = pre_scan_limit.acquire_owned().await else {
            library_scan_tracker
                .mark_title_pre_scan_finished(&title_id)
                .await;
            return;
        };
        let pre_scan_ready = pre_scan_title_for_library_scan_session(
            library_scanner,
            library_scan_tracker,
            session_id,
            title_id.clone(),
            folder_path,
        )
        .await;
        if queue_after_pre_scan
            && pre_scan_ready
            && post_hydration_title_scan_queue
                .enqueue(title_id.clone())
                .await
        {
            info!(title_id = %title_id, "queued tracked episodic title scan");
        }
    });
}

fn should_schedule_title_pre_scan_for_library_scan_session(
    outcome: TrackedTitleWorkflowOutcome,
) -> bool {
    matches!(
        outcome,
        TrackedTitleWorkflowOutcome::SchedulePreScanAndQueue
            | TrackedTitleWorkflowOutcome::SchedulePreScanOnly
    )
}

async fn record_series_title_for_library_scan_session(
    pre_scan_set: &mut tokio::task::JoinSet<()>,
    scheduled_title_ids: &mut HashSet<String>,
    library_scanner: &Arc<dyn LibraryScanner>,
    library_scan_tracker: &LibraryScanTracker,
    post_hydration_title_scan_queue: &PostHydrationTitleScanQueue,
    pre_scan_limit: &Arc<tokio::sync::Semaphore>,
    session_id: &str,
    title: &Title,
    folder_path: &Path,
    queue_after_pre_scan: bool,
) {
    if !scheduled_title_ids.insert(title.id.clone()) {
        return;
    }

    schedule_title_pre_scan_for_library_scan_session(
        pre_scan_set,
        library_scanner.clone(),
        library_scan_tracker.clone(),
        post_hydration_title_scan_queue.clone(),
        pre_scan_limit.clone(),
        session_id,
        &title.id,
        folder_path.to_path_buf(),
        queue_after_pre_scan,
    )
    .await;
}

async fn prepare_series_title_for_full_library_scan(
    app: &AppUseCase,
    pre_scan_set: &mut tokio::task::JoinSet<()>,
    scheduled_title_ids: &mut HashSet<String>,
    queued_titles: &mut HashMap<String, Title>,
    library_scanner: &Arc<dyn LibraryScanner>,
    library_scan_tracker: &LibraryScanTracker,
    pre_scan_limit: &Arc<tokio::sync::Semaphore>,
    session_id: &str,
    title: &mut Title,
    folder_path: &Path,
    episode_presence_cache: &mut HashMap<String, bool>,
    retention_policy: TrackedTitleRetentionPolicy,
) -> Option<TrackedTitleWorkflowOutcome> {
    ensure_title_folder_path_if_missing(app, title, folder_path).await;

    let outcome = prepare_tracked_episodic_title_for_library_scan_session(
        app,
        session_id,
        title,
        TrackedTitleInput::SchedulePreScan,
        episode_presence_cache,
        retention_policy,
    )
    .await?;

    if matches!(outcome, TrackedTitleWorkflowOutcome::QueueWithExistingFiles) {
        queued_titles.insert(title.id.clone(), title.clone());
    }

    if should_schedule_title_pre_scan_for_library_scan_session(outcome) {
        record_series_title_for_library_scan_session(
            pre_scan_set,
            scheduled_title_ids,
            library_scanner,
            library_scan_tracker,
            &app.services.post_hydration_title_scan_queue,
            pre_scan_limit,
            session_id,
            title,
            folder_path,
            matches!(
                outcome,
                TrackedTitleWorkflowOutcome::SchedulePreScanAndQueue
            ),
        )
        .await;
    }

    Some(outcome)
}

async fn flush_title_scan_progress_batch(
    tracker: &LibraryScanTracker,
    session_id: Option<&str>,
    pending_progress: &mut TitleScanProgressDelta,
) {
    let Some(session_id) = session_id else {
        *pending_progress = TitleScanProgressDelta::default();
        return;
    };
    if pending_progress.total() == 0 {
        return;
    }

    let delta = std::mem::take(pending_progress);
    if delta.completed > 0 {
        let _ = tracker
            .increment_file_completed(session_id, delta.completed)
            .await;
    }
    if delta.failed > 0 {
        let _ = tracker
            .increment_file_failed(session_id, delta.failed)
            .await;
    }
}

fn file_source_signature_from_metadata(
    metadata: &std::fs::Metadata,
) -> Option<FileSourceSignature> {
    source_signature_from_std_metadata(metadata)
        .map(|(scheme, value)| FileSourceSignature { scheme, value })
}

fn file_source_snapshot_from_library_file(file: &LibraryFile) -> Option<FileSourceSnapshot> {
    let size_bytes = file.size_bytes?;
    let signature = match (
        file.source_signature_scheme.clone(),
        file.source_signature_value.clone(),
    ) {
        (Some(scheme), Some(value)) => Some(FileSourceSignature { scheme, value }),
        _ => None,
    };

    Some(FileSourceSnapshot {
        size_bytes,
        signature,
    })
}

fn title_media_file_matches_snapshot(
    media_file: &TitleMediaFile,
    snapshot: &FileSourceSnapshot,
) -> bool {
    if media_file.scan_status != "scanned"
        || media_file.size_bytes != snapshot.size_bytes
        || !title_media_file_has_persisted_analysis(media_file)
    {
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

fn title_media_file_has_persisted_analysis(media_file: &TitleMediaFile) -> bool {
    media_file.video_codec.is_some()
        || media_file.video_width.is_some()
        || media_file.video_height.is_some()
        || media_file.video_bitrate_kbps.is_some()
        || media_file.video_bit_depth.is_some()
        || media_file.video_hdr_format.is_some()
        || media_file.video_frame_rate.is_some()
        || media_file.video_profile.is_some()
        || media_file.audio_codec.is_some()
        || media_file.audio_channels.is_some()
        || media_file.audio_bitrate_kbps.is_some()
        || !media_file.audio_languages.is_empty()
        || !media_file.audio_streams.is_empty()
        || !media_file.subtitle_languages.is_empty()
        || !media_file.subtitle_codecs.is_empty()
        || !media_file.subtitle_streams.is_empty()
        || media_file.duration_seconds.is_some()
        || media_file.num_chapters.is_some()
        || media_file.container_format.is_some()
        || media_file.has_multiaudio
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
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

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
        let plan = self
            .build_rename_plan_for_title(
                handler.as_ref(),
                &title,
                template,
                collision_policy,
                missing_metadata_policy,
            )
            .await?;

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
            let mut title_items = self
                .build_rename_plan_items_for_title(
                    &title,
                    handler.as_ref(),
                    &template,
                    &collision_policy,
                    &missing_metadata_policy,
                    &mut planned_targets,
                )
                .await?;
            items.append(&mut title_items);
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
                    if let Some(final_path) = item.final_path.clone()
                        && let Err(failure) =
                            self.persist_rename_item_paths(item, &final_path).await
                    {
                        let rollback = self
                            .rollback_rename_item_after_db_failure(item, &failure.state)
                            .await;

                        item.status = RenameApplyStatus::Failed;
                        item.reason_code = "db_update_failed".into();
                        item.error_message =
                            Some(format!("{}; {}", failure.error, rollback.detail));
                        if rollback.fully_restored {
                            item.final_path = Some(item.current_path.clone());
                        }
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

        self.emit_rename_notifications(actor, &result.items).await;

        Ok(result)
    }

    async fn emit_rename_notifications(&self, actor: &User, items: &[RenameApplyItemResult]) {
        let mut grouped: HashMap<String, (Title, Vec<NotificationMediaUpdate>)> = HashMap::new();

        for item in items {
            if !matches!(item.status, RenameApplyStatus::Applied) {
                continue;
            }

            let Some(final_path) = item.final_path.clone() else {
                continue;
            };

            let title = match self.resolve_title_for_rename_item(item).await {
                Ok(Some(title)) => title,
                Ok(None) => continue,
                Err(error) => {
                    warn!(
                        error = %error,
                        current_path = item.current_path.as_str(),
                        "failed to resolve title for rename notification"
                    );
                    continue;
                }
            };

            let entry = grouped
                .entry(title.id.clone())
                .or_insert_with(|| (title.clone(), Vec::new()));
            entry
                .1
                .push(NotificationMediaUpdate::deleted(item.current_path.clone()));
            entry.1.push(NotificationMediaUpdate::created(final_path));
        }

        for (title_id, (title, updates)) in grouped {
            if updates.is_empty() {
                continue;
            }

            let renamed_files = updates
                .iter()
                .filter(|u| u.update_type == "created")
                .count();
            let metadata = build_lifecycle_notification_metadata(&title, updates);
            let envelope = NotificationEnvelope {
                event_type: NotificationEventType::Rename,
                title: format!("Renamed: {}", title.name),
                body: format!("Renamed {} file(s) for '{}'.", renamed_files, title.name),
                facet: Some(title.facet.as_str().to_string()),
                metadata,
            };

            if let Err(error) = self
                .services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(title_id),
                    None,
                    ActivityKind::SystemNotice,
                    format!("rename completed for '{}'", title.name),
                    ActivitySeverity::Success,
                    vec![ActivityChannel::WebUi],
                    envelope,
                )
                .await
            {
                warn!(
                    error = %error,
                    title = title.name.as_str(),
                    "failed to emit rename notification"
                );
            }
        }
    }

    async fn resolve_title_for_rename_item(
        &self,
        item: &RenameApplyItemResult,
    ) -> AppResult<Option<Title>> {
        let title_id = if let Some(media_file_id) = item.media_file_id.as_deref() {
            self.services
                .media_files
                .get_media_file_by_id(media_file_id)
                .await?
                .map(|file| file.title_id)
        } else if let Some(collection_id) = item.collection_id.as_deref() {
            self.services
                .shows
                .get_collection_by_id(collection_id)
                .await?
                .map(|collection| collection.title_id)
        } else {
            None
        };

        match title_id {
            Some(title_id) => self.services.titles.get_by_id(&title_id).await,
            None => Ok(None),
        }
    }

    async fn persist_rename_item_paths(
        &self,
        item: &RenameApplyItemResult,
        final_path: &str,
    ) -> Result<(), RenamePersistenceFailure> {
        let mut state = RenamePersistenceState::default();

        if let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .media_files
                .update_media_file_path(media_file_id, final_path)
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        } else if item.media_file_id.is_some() {
            state.media_file_updated = true;
        }

        if let Some(collection_id) = item.collection_id.as_deref()
            && let Err(error) = self
                .services
                .shows
                .update_collection(
                    collection_id,
                    None,
                    None,
                    None,
                    Some(final_path.to_string()),
                    None,
                    None,
                    None,
                )
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        }

        Ok(())
    }

    async fn rollback_rename_item_after_db_failure(
        &self,
        item: &RenameApplyItemResult,
        state: &RenamePersistenceState,
    ) -> RenameRollbackOutcome {
        let mut details = Vec::new();
        let mut fully_restored = true;
        let mut filesystem_restored = false;

        match item.write_action {
            RenameWriteAction::Move => match self
                .services
                .library_renamer
                .rollback(std::slice::from_ref(item))
                .await
            {
                Ok(_) => {
                    filesystem_restored = true;
                }
                Err(error) => {
                    fully_restored = false;
                    details.push(format!("filesystem rollback failed: {error}"));
                }
            },
            _ => {
                fully_restored = false;
                details.push("filesystem rollback unavailable for this write action".to_string());
            }
        }

        if filesystem_restored
            && state.media_file_updated
            && let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .media_files
                .update_media_file_path(media_file_id, &item.current_path)
                .await
        {
            fully_restored = false;
            details.push(format!("media file rollback failed: {error}"));
        }

        if details.is_empty() {
            RenameRollbackOutcome {
                fully_restored,
                detail: "rollback succeeded".to_string(),
            }
        } else {
            RenameRollbackOutcome {
                fully_restored,
                detail: format!("rollback failed: {}", details.join("; ")),
            }
        }
    }

    async fn build_rename_plan_for_title(
        &self,
        handler: &dyn crate::FacetHandler,
        title: &Title,
        template: String,
        collision_policy: RenameCollisionPolicy,
        missing_metadata_policy: RenameMissingMetadataPolicy,
    ) -> AppResult<RenamePlan> {
        let mut planned_targets = HashSet::new();
        let items = self
            .build_rename_plan_items_for_title(
                title,
                handler,
                &template,
                &collision_policy,
                &missing_metadata_policy,
                &mut planned_targets,
            )
            .await?;

        Ok(build_rename_plan_from_items(
            handler.facet(),
            Some(title.id.clone()),
            template,
            collision_policy,
            missing_metadata_policy,
            items,
        ))
    }

    async fn build_rename_plan_items_for_title(
        &self,
        title: &Title,
        handler: &dyn crate::FacetHandler,
        template: &str,
        collision_policy: &RenameCollisionPolicy,
        missing_metadata_policy: &RenameMissingMetadataPolicy,
        planned_targets: &mut HashSet<String>,
    ) -> AppResult<Vec<RenamePlanItem>> {
        match title.facet.clone() {
            MediaFacet::Movie => {
                let mut collections = self
                    .services
                    .shows
                    .list_collections_for_title(&title.id)
                    .await?;
                let media_files = self
                    .services
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await?;
                collections.sort_by(|left, right| left.id.cmp(&right.id));
                let media_file_ids_by_path = media_files.into_iter().fold(
                    HashMap::<String, String>::new(),
                    |mut acc, media_file| {
                        acc.entry(media_file.file_path).or_insert(media_file.id);
                        acc
                    },
                );

                let items = collections
                    .into_iter()
                    .map(|collection| {
                        let mut item = handler.build_rename_plan_item(
                            title,
                            &collection,
                            template,
                            collision_policy,
                            missing_metadata_policy,
                            planned_targets,
                        );
                        if let Some(media_file_id) =
                            media_file_ids_by_path.get(item.current_path.as_str())
                        {
                            item.media_file_id = Some(media_file_id.clone());
                        }
                        item
                    })
                    .collect::<Vec<_>>();
                self.normalize_existing_rename_collisions(items).await
            }
            MediaFacet::Series | MediaFacet::Anime => {
                let collections = self
                    .services
                    .shows
                    .list_collections_for_title(&title.id)
                    .await?;
                let episodes = self
                    .services
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await?;
                let media_files = self
                    .services
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await?;

                self.normalize_existing_rename_collisions(
                    build_series_rename_plan_items_from_media_files(
                        title,
                        collections,
                        episodes,
                        media_files,
                        template,
                        collision_policy,
                        missing_metadata_policy,
                        planned_targets,
                    ),
                )
                .await
            }
        }
    }

    async fn normalize_existing_rename_collisions(
        &self,
        items: Vec<RenamePlanItem>,
    ) -> AppResult<Vec<RenamePlanItem>> {
        let mut collection_cache = HashMap::<String, Option<Collection>>::new();
        let mut media_file_cache = HashMap::<String, Option<TitleMediaFile>>::new();
        let mut out = Vec::with_capacity(items.len());

        for mut item in items {
            let Some(proposed_path) = item.proposed_path.clone() else {
                out.push(item);
                continue;
            };

            if proposed_path == item.current_path {
                out.push(item);
                continue;
            }

            let destination_exists_on_disk = Path::new(&proposed_path).exists();

            let tracked_media_file = if let Some(existing) = media_file_cache.get(&proposed_path) {
                existing.clone()
            } else {
                let loaded = self
                    .services
                    .media_files
                    .get_media_file_by_path(&proposed_path)
                    .await?;
                media_file_cache.insert(proposed_path.clone(), loaded.clone());
                loaded
            };
            let tracked_collection = if let Some(existing) = collection_cache.get(&proposed_path) {
                existing.clone()
            } else {
                let loaded = self
                    .services
                    .shows
                    .get_collection_by_ordered_path(&proposed_path)
                    .await?;
                collection_cache.insert(proposed_path.clone(), loaded.clone());
                loaded
            };

            let tracked_media_conflict = tracked_media_file.as_ref().is_some_and(|media_file| {
                item.media_file_id.as_deref() != Some(media_file.id.as_str())
            });
            let tracked_collection_conflict =
                tracked_collection.as_ref().is_some_and(|collection| {
                    item.collection_id.as_deref() != Some(collection.id.as_str())
                });

            if tracked_media_conflict || tracked_collection_conflict {
                item.collision = true;
                item.reason_code = "collision_existing_tracked".into();
                item.write_action = RenameWriteAction::Error;
            } else if !destination_exists_on_disk {
                out.push(item);
                continue;
            } else if matches!(item.write_action, RenameWriteAction::Replace) {
                item.collision = true;
                item.reason_code = "collision_existing".into();
                item.write_action = RenameWriteAction::Error;
            }

            out.push(item);
        }

        Ok(out)
    }

    pub async fn scan_library(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSummary> {
        self.scan_library_with_tracking(actor, facet, None, LibraryScanMode::Full)
            .await
    }

    pub(crate) async fn scan_library_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id_override: Option<String>,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;

        let session = if let Some(session_id) = session_id_override {
            self.services
                .library_scan_tracker
                .start_session_with_id(session_id, facet.clone(), mode)
                .await?
        } else {
            self.services
                .library_scan_tracker
                .start_session(facet.clone())
                .await?
        };

        let result = async {
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

            let summary = match facet {
                MediaFacet::Movie => {
                    self.scan_library_movies(actor, &facet, &library_path, &session.session_id)
                        .await?
                }
                MediaFacet::Series | MediaFacet::Anime => {
                    self.scan_library_series(actor, &facet, &library_path, &session.session_id)
                        .await?
                }
            };

            let _ = self
                .services
                .library_scan_tracker
                .set_summary(&session.session_id, summary.clone())
                .await;
            maybe_complete_library_scan_session(self, &session.session_id).await;
            Ok(summary)
        }
        .await;

        if result.is_err() {
            let _ = self
                .services
                .library_scan_tracker
                .fail_session(&session.session_id)
                .await;
        }

        result
    }

    /// Movie library scan: each video file is a potential title.
    async fn scan_library_movies(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_path: &str,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let started_at = Instant::now();
        let mut discovered_files = self
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

        while let Some(file_chunk) = discovered_files.recv().await {
            let file_chunk = file_chunk?;
            if file_chunk.is_empty() {
                continue;
            }
            let _ = self
                .services
                .library_scan_tracker
                .add_found_titles(session_id, file_chunk.len())
                .await;
            let _ = self
                .services
                .library_scan_tracker
                .add_file_total(session_id, file_chunk.len())
                .await;

            let (candidates, batch_lookups) = preload_movie_library_scan_candidates(
                self.services.metadata_gateway.clone(),
                &file_chunk,
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
                        track_title_for_library_scan_session(self, session_id, &created, None)
                            .await;
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

                    if title.metadata_fetched_at.is_none() {
                        track_title_for_library_scan_session(self, session_id, &title, None).await;
                    }
                    summary.matched += 1;
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
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
                    if title.metadata_fetched_at.is_none() {
                        track_title_for_library_scan_session(self, session_id, &title, None).await;
                    }
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
                        .await;
                    continue;
                }

                if let Some(parsed_tmdb_id) = parsed_release.tmdb_id.map(|id| id.to_string())
                    && let Some(&index) = existing_titles_by_tmdb_id.get(&parsed_tmdb_id)
                {
                    summary.matched += 1;
                    let title = existing_titles[index].clone();
                    if title.metadata_fetched_at.is_none() {
                        track_title_for_library_scan_session(self, session_id, &title, None).await;
                    }
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
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
                    if title.metadata_fetched_at.is_none() {
                        track_title_for_library_scan_session(self, session_id, &title, None).await;
                    }
                    self.track_movie_file_in_collection(&title, file, &mut summary)
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
                        .await;
                    continue;
                }

                if query.is_empty() {
                    summary.skipped += 1;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
                        .await;
                    continue;
                }

                let Some(selected) = candidate.selected_metadata.clone() else {
                    summary.unmatched += 1;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
                        .await;
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
                    track_title_for_library_scan_session(self, session_id, &title, None).await;
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

                if title.metadata_fetched_at.is_none() {
                    track_title_for_library_scan_session(self, session_id, &title, None).await;
                }
                self.track_movie_file_in_collection(&title, file, &mut summary)
                    .await;
                let _ = self
                    .services
                    .library_scan_tracker
                    .increment_file_completed(session_id, 1)
                    .await;
            }
        }

        let _ = self
            .services
            .library_scan_tracker
            .mark_metadata_total_known(session_id)
            .await;
        let _ = self
            .services
            .library_scan_tracker
            .mark_file_total_known_if_resolved(session_id)
            .await;

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
            elapsed_ms = elapsed_ms_u64(started_at),
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
        session_id: &str,
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
        let _ = self
            .services
            .library_scan_tracker
            .set_found_titles(session_id, folders_count)
            .await;

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
        let mut queued_titles = HashMap::new();
        let mut scheduled_title_ids = HashSet::new();
        let mut episode_presence_cache = HashMap::new();
        let library_scan_tracker = self.services.library_scan_tracker.clone();
        let library_scanner = self.services.library_scanner.clone();
        let mut pre_scan_set = tokio::task::JoinSet::new();
        let pre_scan_limit = Arc::new(tokio::sync::Semaphore::new(TITLE_PRE_SCAN_CONCURRENCY));

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
                        prepare_series_title_for_full_library_scan(
                            self,
                            &mut pre_scan_set,
                            &mut scheduled_title_ids,
                            &mut queued_titles,
                            &library_scanner,
                            &library_scan_tracker,
                            &pre_scan_limit,
                            session_id,
                            existing,
                            &candidate.folder_path,
                            &mut episode_presence_cache,
                            TrackedTitleRetentionPolicy::SkipIfNotRelinkable,
                        )
                        .await;
                        summary.skipped += 1;
                        continue;
                    }

                    let name = nfo_meta
                        .and_then(|m| m.title.clone())
                        .unwrap_or_else(|| folder_name.clone());
                    let name_key = normalize_title_key(&name);
                    if let Some(&index) = existing_titles_by_name.get(&name_key) {
                        let existing = &mut existing_titles[index];
                        prepare_series_title_for_full_library_scan(
                            self,
                            &mut pre_scan_set,
                            &mut scheduled_title_ids,
                            &mut queued_titles,
                            &library_scanner,
                            &library_scan_tracker,
                            &pre_scan_limit,
                            session_id,
                            existing,
                            &candidate.folder_path,
                            &mut episode_presence_cache,
                            TrackedTitleRetentionPolicy::SkipIfNotRelinkable,
                        )
                        .await;
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
                            prepare_series_title_for_full_library_scan(
                                self,
                                &mut pre_scan_set,
                                &mut scheduled_title_ids,
                                &mut queued_titles,
                                &library_scanner,
                                &library_scan_tracker,
                                &pre_scan_limit,
                                session_id,
                                &mut created,
                                &candidate.folder_path,
                                &mut episode_presence_cache,
                                TrackedTitleRetentionPolicy::KeepTrackedForDeferredReconcile,
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
                    prepare_series_title_for_full_library_scan(
                        self,
                        &mut pre_scan_set,
                        &mut scheduled_title_ids,
                        &mut queued_titles,
                        &library_scanner,
                        &library_scan_tracker,
                        &pre_scan_limit,
                        session_id,
                        existing,
                        &candidate.folder_path,
                        &mut episode_presence_cache,
                        TrackedTitleRetentionPolicy::SkipIfNotRelinkable,
                    )
                    .await;
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
                    prepare_series_title_for_full_library_scan(
                        self,
                        &mut pre_scan_set,
                        &mut scheduled_title_ids,
                        &mut queued_titles,
                        &library_scanner,
                        &library_scan_tracker,
                        &pre_scan_limit,
                        session_id,
                        existing,
                        &candidate.folder_path,
                        &mut episode_presence_cache,
                        TrackedTitleRetentionPolicy::SkipIfNotRelinkable,
                    )
                    .await;
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
                        prepare_series_title_for_full_library_scan(
                            self,
                            &mut pre_scan_set,
                            &mut scheduled_title_ids,
                            &mut queued_titles,
                            &library_scanner,
                            &library_scan_tracker,
                            &pre_scan_limit,
                            session_id,
                            &mut created,
                            &candidate.folder_path,
                            &mut episode_presence_cache,
                            TrackedTitleRetentionPolicy::KeepTrackedForDeferredReconcile,
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

        let _ = self
            .services
            .library_scan_tracker
            .mark_metadata_total_known(session_id)
            .await;

        let mut title_ids_to_scan = queued_titles.keys().cloned().collect::<Vec<_>>();
        title_ids_to_scan.sort();
        for title_id in title_ids_to_scan {
            if self
                .services
                .post_hydration_title_scan_queue
                .enqueue(title_id.clone())
                .await
            {
                info!(title_id = %title_id, "queued tracked episodic title scan");
            }
        }

        while self
            .services
            .library_scan_tracker
            .has_pending_title_pre_scans_for_session(session_id)
            .await
        {
            let Some(result) = pre_scan_set.join_next().await else {
                break;
            };
            if let Err(error) = result {
                warn!(
                    error = %error,
                    facet = facet.as_str(),
                    "episodic title pre-scan task failed during library scan"
                );
            }
        }

        let _ = self
            .services
            .library_scan_tracker
            .mark_file_total_known_if_resolved(session_id)
            .await;

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
            elapsed_ms = elapsed_ms_u64(started_at),
            "series library scan completed"
        );

        Ok(summary)
    }

    pub(crate) async fn background_library_refresh_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;

        let session = self
            .services
            .library_scan_tracker
            .start_session_with_id(
                session_id.to_string(),
                facet.clone(),
                LibraryScanMode::Additive,
            )
            .await?;

        let result = async {
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

            let summary = match facet {
                MediaFacet::Movie => {
                    self.background_refresh_movies(actor, &library_path, &session.session_id)
                        .await?
                }
                MediaFacet::Series | MediaFacet::Anime => {
                    self.background_refresh_series(
                        actor,
                        &facet,
                        &library_path,
                        &session.session_id,
                    )
                    .await?
                }
            };

            let _ = self
                .services
                .library_scan_tracker
                .set_summary(&session.session_id, summary.clone())
                .await;
            maybe_complete_library_scan_session(self, &session.session_id).await;
            Ok(summary)
        }
        .await;

        if result.is_err() {
            let _ = self
                .services
                .library_scan_tracker
                .fail_session(&session.session_id)
                .await;
        }

        result
    }

    async fn background_refresh_series(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_path: &str,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let started_at = Instant::now();
        let root = Path::new(library_path);
        if !root.is_dir() {
            return Err(AppError::Validation(format!(
                "library path is not a directory: {library_path}"
            )));
        }

        let folders = list_child_directories(root).await?;
        let _ = self
            .services
            .library_scan_tracker
            .set_found_titles(session_id, folders.len())
            .await;

        let mut summary = LibraryScanSummary::default();
        let mut metadata_lookups = 0usize;
        let mut episode_presence_cache = HashMap::new();

        let mut existing_titles = self.services.titles.list(Some(facet.clone()), None).await?;
        let mut existing_titles_by_name = HashMap::new();
        let mut existing_titles_by_tvdb_id = HashMap::new();
        let mut existing_titles_by_folder_path = HashMap::new();
        for (index, title) in existing_titles.iter().enumerate() {
            index_series_title(
                title,
                index,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
            );
            if let Some(folder_path) = title
                .folder_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                existing_titles_by_folder_path.insert(folder_path.to_string(), index);
            }
        }

        let mut unknown_folders = Vec::new();
        for folder in folders {
            summary.scanned += 1;
            let folder_key = folder.to_string_lossy().to_string();
            if let Some(&index) = existing_titles_by_folder_path.get(&folder_key) {
                let title = &mut existing_titles[index];
                self.maybe_probe_existing_series_title_for_background_refresh(
                    session_id,
                    title,
                    &folder,
                    &mut summary,
                    &mut episode_presence_cache,
                )
                .await?;
            } else {
                unknown_folders.push(folder);
            }
        }

        for folder_batch in unknown_folders.chunks(LIBRARY_SCAN_BATCH_SIZE) {
            let (candidates, batch_lookups) = preload_series_library_scan_candidates(
                self.services.metadata_gateway.clone(),
                folder_batch,
            )
            .await?;
            metadata_lookups += batch_lookups;

            for candidate in candidates {
                let folder_name = match candidate.folder_name.as_deref() {
                    Some(value) => value.to_string(),
                    None => {
                        summary.skipped += 1;
                        continue;
                    }
                };

                if let Some(tvdb_id) = candidate
                    .nfo_meta
                    .as_ref()
                    .and_then(|meta| meta.tvdb_id.as_deref())
                    && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
                {
                    let title = &mut existing_titles[index];
                    ensure_title_folder_path_if_missing(self, title, &candidate.folder_path).await;
                    if let Some(folder_path) = title
                        .folder_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        existing_titles_by_folder_path.insert(folder_path.to_string(), index);
                    }
                    self.maybe_probe_existing_series_title_for_background_refresh(
                        session_id,
                        title,
                        &candidate.folder_path,
                        &mut summary,
                        &mut episode_presence_cache,
                    )
                    .await?;
                    continue;
                }

                let query = candidate.query.trim().to_string();
                let name_key = normalize_title_key(&query);
                if let Some(&index) = existing_titles_by_name.get(&name_key) {
                    let title = &mut existing_titles[index];
                    ensure_title_folder_path_if_missing(self, title, &candidate.folder_path).await;
                    if let Some(folder_path) = title
                        .folder_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        existing_titles_by_folder_path.insert(folder_path.to_string(), index);
                    }
                    self.maybe_probe_existing_series_title_for_background_refresh(
                        session_id,
                        title,
                        &candidate.folder_path,
                        &mut summary,
                        &mut episode_presence_cache,
                    )
                    .await?;
                    continue;
                }

                let selected = if let Some(selected) = candidate.selected_metadata.clone() {
                    selected
                } else {
                    summary.unmatched += 1;
                    continue;
                };

                if let Some(&index) = existing_titles_by_tvdb_id.get(&selected.tvdb_id) {
                    let title = &mut existing_titles[index];
                    ensure_title_folder_path_if_missing(self, title, &candidate.folder_path).await;
                    if let Some(folder_path) = title
                        .folder_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        existing_titles_by_folder_path.insert(folder_path.to_string(), index);
                    }
                    self.maybe_probe_existing_series_title_for_background_refresh(
                        session_id,
                        title,
                        &candidate.folder_path,
                        &mut summary,
                        &mut episode_presence_cache,
                    )
                    .await?;
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
                        let file_scan = self
                            .services
                            .library_scanner
                            .scan_directory_for_progress_with_metrics(
                                candidate.folder_path.to_string_lossy().as_ref(),
                            )
                            .await?;
                        match prepare_tracked_episodic_title_for_library_scan_session(
                            self,
                            session_id,
                            &created,
                            TrackedTitleInput::PreScannedFiles(file_scan.files),
                            &mut episode_presence_cache,
                            TrackedTitleRetentionPolicy::KeepTrackedForDeferredReconcile,
                        )
                        .await
                        {
                            Some(TrackedTitleWorkflowOutcome::QueueWithExistingFiles) => {
                                if self
                                    .services
                                    .post_hydration_title_scan_queue
                                    .enqueue(created.id.clone())
                                    .await
                                {
                                    info!(title_id = %created.id, "queued additive episodic title scan");
                                }
                            }
                            Some(TrackedTitleWorkflowOutcome::AttachOnly)
                            | Some(TrackedTitleWorkflowOutcome::SkipCompleteNow)
                            | Some(TrackedTitleWorkflowOutcome::SchedulePreScanAndQueue)
                            | Some(TrackedTitleWorkflowOutcome::SchedulePreScanOnly)
                            | None => {}
                        }
                        let index = existing_titles.len();
                        existing_titles.push(created.clone());
                        index_series_title(
                            &created,
                            index,
                            &mut existing_titles_by_name,
                            &mut existing_titles_by_tvdb_id,
                        );
                        if let Some(folder_path) = created
                            .folder_path
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            existing_titles_by_folder_path.insert(folder_path.to_string(), index);
                        }
                        summary.imported += 1;
                    }
                    Err(error) => {
                        warn!(
                            folder = %folder_name,
                            tvdb_id = %selected.tvdb_id,
                            error = %error,
                            "background series refresh: failed to create title"
                        );
                        summary.unmatched += 1;
                    }
                }
            }
        }

        let _ = self
            .services
            .library_scan_tracker
            .mark_metadata_total_known(session_id)
            .await;
        let _ = self
            .services
            .library_scan_tracker
            .mark_file_total_known(session_id)
            .await;

        info!(
            path = %library_path,
            facet = facet.as_str(),
            scanned = summary.scanned,
            imported = summary.imported,
            matched = summary.matched,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups,
            elapsed_ms = elapsed_ms_u64(started_at),
            "background library refresh completed"
        );

        Ok(summary)
    }

    async fn maybe_probe_existing_series_title_for_background_refresh(
        &self,
        session_id: &str,
        title: &mut Title,
        folder_path: &Path,
        summary: &mut LibraryScanSummary,
        episode_presence_cache: &mut HashMap<String, bool>,
    ) -> AppResult<()> {
        let probe_outcome =
            run_background_refresh_probe_with_delta(self, &title.id, folder_path, async {
                let file_scan = self
                    .services
                    .library_scanner
                    .scan_directory_for_progress_with_metrics(
                        folder_path.to_string_lossy().as_ref(),
                    )
                    .await?;
                let discovered_paths = file_scan
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<HashSet<_>>();
                let existing_paths = self
                    .services
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await?
                    .into_iter()
                    .map(|file| file.file_path)
                    .collect::<HashSet<_>>();
                Ok::<_, AppError>((file_scan.files, discovered_paths, existing_paths))
            })
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "background series refresh: failed to probe existing title {} at {}: {error}",
                    title.id,
                    folder_path.display()
                ))
            })?;

        match probe_outcome {
            BackgroundRefreshProbeOutcome::Unchanged => {
                summary.skipped += 1;
            }
            BackgroundRefreshProbeOutcome::Changed(discovered_files) => {
                match prepare_tracked_episodic_title_for_library_scan_session(
                    self,
                    session_id,
                    title,
                    TrackedTitleInput::PreScannedFiles(discovered_files),
                    episode_presence_cache,
                    TrackedTitleRetentionPolicy::SkipIfNotRelinkable,
                )
                .await
                {
                    Some(TrackedTitleWorkflowOutcome::QueueWithExistingFiles) => {
                        summary.matched += 1;
                        if self
                            .services
                            .post_hydration_title_scan_queue
                            .enqueue(title.id.clone())
                            .await
                        {
                            info!(title_id = %title.id, "queued additive episodic title scan");
                        }
                    }
                    Some(TrackedTitleWorkflowOutcome::SkipCompleteNow) => {
                        summary.skipped += 1;
                    }
                    Some(TrackedTitleWorkflowOutcome::AttachOnly)
                    | Some(TrackedTitleWorkflowOutcome::SchedulePreScanAndQueue)
                    | Some(TrackedTitleWorkflowOutcome::SchedulePreScanOnly)
                    | None => {
                        summary.skipped += 1;
                    }
                }
            }
        }

        Ok(())
    }

    async fn background_refresh_movies(
        &self,
        actor: &User,
        library_path: &str,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let started_at = Instant::now();
        let root = Path::new(library_path);
        if !root.is_dir() {
            return Err(AppError::Validation(format!(
                "library path is not a directory: {library_path}"
            )));
        }

        let entries = list_movie_top_level_entries(root).await?;
        let _ = self
            .services
            .library_scan_tracker
            .set_found_titles(session_id, entries.len())
            .await;

        let mut summary = LibraryScanSummary::default();
        let mut metadata_lookups = 0usize;
        let mut existing_titles = self
            .services
            .titles
            .list(Some(MediaFacet::Movie), None)
            .await?;
        let mut existing_titles_by_name = HashMap::new();
        let mut existing_titles_by_tvdb_id = HashMap::new();
        let mut existing_titles_by_imdb_id = HashMap::new();
        let mut existing_titles_by_tmdb_id = HashMap::new();
        let mut existing_titles_by_probe_path = HashMap::new();
        let existing_title_ids = existing_titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let collections_by_title = self
            .services
            .shows
            .list_collections_for_titles(&existing_title_ids)
            .await
            .unwrap_or_default();

        for (index, title) in existing_titles.iter().enumerate() {
            index_movie_title(
                title,
                index,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
            );
            let collections = collections_by_title
                .get(&title.id)
                .cloned()
                .unwrap_or_default();
            if let Some(probe_path) = derive_movie_probe_path(root, title, &collections) {
                existing_titles_by_probe_path
                    .insert(probe_path.to_string_lossy().to_string(), index);
            }
        }

        let mut unknown_files = Vec::new();
        for entry in entries {
            summary.scanned += 1;
            let entry_key = entry.path.to_string_lossy().to_string();
            if let Some(&index) = existing_titles_by_probe_path.get(&entry_key) {
                let title = &existing_titles[index];
                let collections = collections_by_title
                    .get(&title.id)
                    .cloned()
                    .unwrap_or_default();
                self.maybe_probe_existing_movie_title_for_background_refresh(
                    session_id,
                    title,
                    &collections,
                    &entry,
                    &mut summary,
                )
                .await?;
                continue;
            }

            if entry.is_dir {
                let mut files = self
                    .services
                    .library_scanner
                    .scan_library(entry.path.to_string_lossy().as_ref())
                    .await?;
                unknown_files.append(&mut files);
            } else {
                unknown_files.push(LibraryFile {
                    path: entry.path.to_string_lossy().to_string(),
                    display_name: entry
                        .path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    nfo_path: matching_movie_nfo_path(&entry.path),
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                });
            }
        }

        for file_chunk in unknown_files.chunks(LIBRARY_SCAN_BATCH_SIZE) {
            let (candidates, batch_lookups) = preload_movie_library_scan_candidates(
                self.services.metadata_gateway.clone(),
                file_chunk,
                library_path,
            )
            .await?;
            metadata_lookups += batch_lookups;

            for candidate in candidates {
                let file = &candidate.file;
                let mut resolved_title: Option<Title> = None;

                if let Some(tvdb_id) = candidate
                    .nfo_meta
                    .as_ref()
                    .and_then(|meta| meta.tvdb_id.as_deref())
                    && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
                {
                    resolved_title = Some(existing_titles[index].clone());
                }

                if resolved_title.is_none()
                    && let Some(parsed_imdb_id) = candidate
                        .parsed_release
                        .imdb_id
                        .as_deref()
                        .and_then(crate::normalize::normalize_imdb_id)
                    && let Some(&index) = existing_titles_by_imdb_id.get(&parsed_imdb_id)
                {
                    resolved_title = Some(existing_titles[index].clone());
                }

                if resolved_title.is_none()
                    && let Some(parsed_tmdb_id) = candidate
                        .parsed_release
                        .tmdb_id
                        .map(|value| value.to_string())
                    && let Some(&index) = existing_titles_by_tmdb_id.get(&parsed_tmdb_id)
                {
                    resolved_title = Some(existing_titles[index].clone());
                }

                if resolved_title.is_none()
                    && let Some(index) = candidate.query_variants.iter().find_map(|query_variant| {
                        existing_titles_by_name
                            .get(&normalize_title_key(query_variant))
                            .copied()
                    })
                {
                    resolved_title = Some(existing_titles[index].clone());
                }

                if resolved_title.is_none() {
                    let Some(selected) = candidate.selected_metadata.clone() else {
                        summary.unmatched += 1;
                        continue;
                    };
                    let new_title = NewTitle {
                        name: selected.name.clone(),
                        facet: MediaFacet::Movie,
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
                        Ok(created) => {
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
                            if let Some(parent) = Path::new(&file.path).parent()
                                && parent != root
                            {
                                existing_titles_by_probe_path
                                    .insert(parent.to_string_lossy().to_string(), index);
                            } else {
                                existing_titles_by_probe_path.insert(file.path.clone(), index);
                            }
                            resolved_title = Some(created);
                            summary.imported += 1;
                        }
                        Err(error) => {
                            warn!(
                                path = %file.path,
                                error = %error,
                                "background movie refresh: failed to create title"
                            );
                            summary.unmatched += 1;
                            continue;
                        }
                    }
                }

                let Some(title) = resolved_title else {
                    summary.unmatched += 1;
                    continue;
                };

                if title.metadata_fetched_at.is_none() {
                    track_title_for_library_scan_session(self, session_id, &title, None).await;
                }
                let _ = self
                    .services
                    .library_scan_tracker
                    .add_file_total(session_id, 1)
                    .await;
                self.track_movie_file_in_collection(&title, file, &mut summary)
                    .await;
                let _ = self
                    .services
                    .library_scan_tracker
                    .increment_file_completed(session_id, 1)
                    .await;
                summary.matched += 1;
            }
        }

        let _ = self
            .services
            .library_scan_tracker
            .mark_metadata_total_known(session_id)
            .await;
        let _ = self
            .services
            .library_scan_tracker
            .mark_file_total_known(session_id)
            .await;

        info!(
            path = %library_path,
            scanned = summary.scanned,
            imported = summary.imported,
            matched = summary.matched,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups,
            elapsed_ms = elapsed_ms_u64(started_at),
            "background movie refresh completed"
        );

        Ok(summary)
    }

    async fn maybe_probe_existing_movie_title_for_background_refresh(
        &self,
        session_id: &str,
        title: &Title,
        collections: &[Collection],
        entry: &MovieTopLevelEntry,
        summary: &mut LibraryScanSummary,
    ) -> AppResult<()> {
        let probe_outcome =
            run_background_refresh_probe_with_delta(self, &title.id, &entry.path, async {
                let discovered_files = if entry.is_dir {
                    self.services
                        .library_scanner
                        .scan_directory_for_progress_with_metrics(
                            entry.path.to_string_lossy().as_ref(),
                        )
                        .await?
                        .files
                } else {
                    vec![LibraryFile {
                        path: entry.path.to_string_lossy().to_string(),
                        display_name: entry
                            .path
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .unwrap_or_default()
                            .to_string(),
                        nfo_path: matching_movie_nfo_path(&entry.path),
                        size_bytes: None,
                        source_signature_scheme: None,
                        source_signature_value: None,
                    }]
                };

                let discovered_paths = discovered_files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<HashSet<_>>();
                let existing_paths = collections
                    .iter()
                    .filter_map(|collection| collection.ordered_path.clone())
                    .filter(|path| {
                        if entry.is_dir {
                            path.starts_with(format!("{}/", entry.path.to_string_lossy()).as_str())
                                || path == entry.path.to_string_lossy().as_ref()
                        } else {
                            path == entry.path.to_string_lossy().as_ref()
                        }
                    })
                    .collect::<HashSet<_>>();

                Ok::<_, AppError>((discovered_files, discovered_paths, existing_paths))
            })
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "background movie refresh: failed to probe existing title {} at {}: {error}",
                    title.id,
                    entry.path.display()
                ))
            })?;

        match probe_outcome {
            BackgroundRefreshProbeOutcome::Unchanged => {
                summary.skipped += 1;
            }
            BackgroundRefreshProbeOutcome::Changed(discovered_files) => {
                let discovered_paths = discovered_files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<HashSet<_>>();

                if title.metadata_fetched_at.is_none() {
                    track_title_for_library_scan_session(self, session_id, title, None).await;
                }
                let _ = self
                    .services
                    .library_scan_tracker
                    .add_file_total(session_id, discovered_files.len())
                    .await;
                for file in &discovered_files {
                    self.track_movie_file_in_collection(title, file, summary)
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .increment_file_completed(session_id, 1)
                        .await;
                }

                for collection in collections {
                    let Some(ordered_path) = collection.ordered_path.as_deref() else {
                        continue;
                    };
                    let should_consider = if entry.is_dir {
                        ordered_path
                            .starts_with(format!("{}/", entry.path.to_string_lossy()).as_str())
                            || ordered_path == entry.path.to_string_lossy().as_ref()
                    } else {
                        ordered_path == entry.path.to_string_lossy().as_ref()
                    };
                    if !should_consider || discovered_paths.contains(ordered_path) {
                        continue;
                    }
                    if let Err(error) = self.services.shows.delete_collection(&collection.id).await
                    {
                        warn!(
                            error = %error,
                            collection_id = %collection.id,
                            path = %ordered_path,
                            "background movie refresh: failed to delete stale collection"
                        );
                    }
                }

                summary.matched += 1;
            }
        }

        Ok(())
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

        self.scan_title_library_for_title(actor, title, None, None)
            .await
    }

    pub(crate) async fn scan_title_library_for_title(
        &self,
        actor: &User,
        title: Title,
        session_id: Option<&str>,
        pre_scanned_files: Option<Vec<LibraryFile>>,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;
        let started_at = Instant::now();
        let pre_scanned_file_count = pre_scanned_files.as_ref().map(Vec::len);
        let scan_mode = match session_id {
            Some(value) => self
                .services
                .library_scan_tracker
                .get_session(value)
                .await
                .map(|session| session.mode)
                .unwrap_or(LibraryScanMode::Full),
            None => LibraryScanMode::Full,
        };

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
        info!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            scan_mode = %scan_mode.as_str(),
            title_dir = %title_dir_str,
            pre_scanned_file_count,
            "title scan stage: start"
        );
        let mut walk_elapsed = Duration::ZERO;
        let mut stat_elapsed = Duration::ZERO;
        let mut analyze_elapsed = Duration::ZERO;
        let mut db_elapsed = Duration::ZERO;

        // If the title directory was deleted, recreate it and treat as empty.
        if tokio::fs::metadata(&title_dir).await.is_err() {
            tokio::fs::create_dir_all(&title_dir).await.map_err(|err| {
                AppError::Repository(format!(
                    "failed to recreate title directory {}: {err}",
                    title_dir.display()
                ))
            })?;
        }

        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                if session_id.is_some() {
                    self.services
                        .library_scan_tracker
                        .abandon_title_pre_scan(&title.id)
                        .await;
                }
                let scan_result = self
                    .services
                    .library_scanner
                    .scan_directory_with_metrics(&title_dir_str)
                    .await?;
                walk_elapsed =
                    walk_elapsed.saturating_add(Duration::from_millis(scan_result.walk_ms));
                stat_elapsed =
                    stat_elapsed.saturating_add(Duration::from_millis(scan_result.stat_ms));
                if let Some(session_id) = session_id {
                    let _ = self
                        .services
                        .library_scan_tracker
                        .add_file_total(session_id, scan_result.files.len())
                        .await;
                    let _ = self
                        .services
                        .library_scan_tracker
                        .mark_file_total_known_if_resolved(session_id)
                        .await;
                }
                scan_result.files
            }
        };

        let db_started = Instant::now();
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
        db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        info!(
            title_id = %title.id,
            title_name = %title.name,
            discovered_files = discovered_files.len(),
            existing_files = existing_files.len(),
            collections = collections.len(),
            title_episodes = title_episodes.len(),
            "title scan stage: db state loaded"
        );
        let episode_lookup = build_title_episode_lookup(&collections, &title_episodes);
        info!(
            title_id = %title.id,
            title_name = %title.name,
            "title scan stage: episode lookup built"
        );

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
        let analysis_limit = self.services.library_scan_analysis_limit.clone();
        let library_scan_tracker = self.services.library_scan_tracker.clone();
        let mut pending_progress = TitleScanProgressDelta::default();
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;
        let actor_user_id = Some(actor.id.clone());

        for file_chunk in discovered_files.chunks(TITLE_SCAN_FILE_BATCH_SIZE) {
            let files = file_chunk.to_vec();
            let mut planned_files = Vec::new();
            let mut title_updated_in_batch = false;

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
                        pending_progress.absorb(TitleScanProgressDelta::completed(1));
                        flush_title_scan_progress_batch(
                            &library_scan_tracker,
                            session_id,
                            &mut pending_progress,
                        )
                        .await;
                        continue;
                    }
                };

                let season_str = ep_meta.season.unwrap_or(1).to_string();
                let target_episodes =
                    resolve_target_episodes_from_lookup(ep_meta, &season_str, &episode_lookup);

                if target_episodes.is_empty() {
                    summary.unmatched += 1;
                    pending_progress.absorb(TitleScanProgressDelta::completed(1));
                    flush_title_scan_progress_batch(
                        &library_scan_tracker,
                        session_id,
                        &mut pending_progress,
                    )
                    .await;
                    continue;
                }

                let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(&file)
                {
                    snapshot
                } else {
                    let stat_started = Instant::now();
                    let metadata = match tokio::fs::metadata(source_path).await {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_path = %file.path,
                                "failed to read file metadata during title scan"
                            );
                            summary.skipped += 1;
                            pending_progress.absorb(TitleScanProgressDelta::completed(1));
                            flush_title_scan_progress_batch(
                                &library_scan_tracker,
                                session_id,
                                &mut pending_progress,
                            )
                            .await;
                            continue;
                        }
                    };
                    stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());

                    FileSourceSnapshot {
                        size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                        signature: file_source_signature_from_metadata(&metadata),
                    }
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
            info!(
                title_id = %title.id,
                title_name = %title.name,
                chunk_files = planned_files.len(),
                "title scan stage: chunk planned"
            );
            info!(
                title_id = %title.id,
                title_name = %title.name,
                "title scan stage: analysis phase begin"
            );

            let mut analysis_set = tokio::task::JoinSet::new();
            let mut pending_analysis_plans = HashMap::new();
            for plan in planned_files {
                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
                    let outcome = self
                        .finalize_title_scan_file(
                            &title,
                            plan,
                            None,
                            scan_mode.clone(),
                            &mut episode_links,
                            &mut summary,
                            &mut db_elapsed,
                        )
                        .await;
                    pending_progress.absorb(outcome.progress);
                    title_updated_in_batch |= outcome.title_updated;
                    flush_title_scan_progress_batch(
                        &library_scan_tracker,
                        session_id,
                        &mut pending_progress,
                    )
                    .await;
                    continue;
                }

                analyzed_files += 1;
                let analyzer = self.services.media_analyzer.clone();
                let analysis_limit = analysis_limit.clone();
                let file_path = plan.file.path.clone();
                pending_analysis_plans.insert(file_path.clone(), plan);
                analysis_set.spawn(async move {
                    tracing::info!(file_path = %file_path, "title scan analysis task: start");
                    let _permit = analysis_limit
                        .acquire_owned()
                        .await
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                    let analysis_started = Instant::now();
                    let outcome = analyzer.analyze_file(PathBuf::from(&file_path)).await?;
                    tracing::info!(file_path = %file_path, "title scan analysis task: complete");
                    Ok::<(String, MediaAnalysisOutcome, Duration), AppError>((
                        file_path,
                        outcome,
                        analysis_started.elapsed(),
                    ))
                });
            }
            info!(
                title_id = %title.id,
                title_name = %title.name,
                pending_analysis = pending_analysis_plans.len(),
                "title scan stage: analysis tasks spawned"
            );

            while let Some(result) = analysis_set.join_next().await {
                let (file_path, outcome, analysis_duration) =
                    result.map_err(|error| AppError::Repository(error.to_string()))??;
                analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);
                let Some(plan) = pending_analysis_plans.remove(&file_path) else {
                    warn!(
                        title_id = %title.id,
                        file_path = %file_path,
                        "missing planned title scan file for completed analysis result"
                    );
                    continue;
                };
                info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file begin"
                );
                let outcome = self
                    .finalize_title_scan_file(
                        &title,
                        plan,
                        Some(outcome),
                        scan_mode.clone(),
                        &mut episode_links,
                        &mut summary,
                        &mut db_elapsed,
                    )
                    .await;
                pending_progress.absorb(outcome.progress);
                title_updated_in_batch |= outcome.title_updated;
                flush_title_scan_progress_batch(
                    &library_scan_tracker,
                    session_id,
                    &mut pending_progress,
                )
                .await;
                info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file complete"
                );
            }

            if title_updated_in_batch {
                self.emit_title_updated_activity(actor_user_id.clone(), &title)
                    .await;
            }
        }

        flush_title_scan_progress_batch(&library_scan_tracker, session_id, &mut pending_progress)
            .await;

        let mut title_updated_after_scan = false;
        for stale_path in remaining_existing_paths {
            let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                continue;
            };
            if !stale_path.starts_with(title_dir_str.as_str()) {
                continue;
            }
            if Path::new(&record.file_path).exists() {
                continue;
            }
            let db_started = Instant::now();
            let delete_result = self
                .services
                .media_files
                .delete_media_file(&record.id)
                .await;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            if let Err(error) = delete_result {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %record.file_path,
                    "failed to delete stale media file during title scan"
                );
            } else {
                title_updated_after_scan = true;
            }
        }

        if title.folder_path.as_deref() != Some(title_dir_str.as_str()) {
            let db_started = Instant::now();
            self.services
                .titles
                .set_folder_path(&title.id, &title_dir_str)
                .await?;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            title_updated_after_scan = true;
        }

        if let Some(use_season_folders) = layout_summary.inferred_use_season_folders()
            && crate::app_usecase_import::use_season_folders(&title) != use_season_folders
        {
            let tags = merge_title_scan_option_tags(title.tags.clone(), use_season_folders);
            let db_started = Instant::now();
            self.update_title_metadata(actor, &title.id, None, None, Some(tags))
                .await?;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            title_updated_after_scan = true;
        }

        let db_started = Instant::now();
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
        db_elapsed = db_elapsed.saturating_add(db_started.elapsed());

        if title_updated_after_scan {
            self.emit_title_updated_activity(actor_user_id, &title)
                .await;
        }

        if let Some(session_id) = session_id {
            self.services
                .library_scan_tracker
                .release_title(&title.id)
                .await;
            maybe_complete_library_scan_session(self, session_id).await;
        }

        info!(
            title_id = %title.id,
            path = %title_dir.display(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            walk_ms = u64::try_from(walk_elapsed.as_millis()).unwrap_or(u64::MAX),
            stat_ms = u64::try_from(stat_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyze_ms = u64::try_from(analyze_elapsed.as_millis()).unwrap_or(u64::MAX),
            db_ms = u64::try_from(db_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyzed_files,
            unchanged_file_skips,
            batch_size = TITLE_SCAN_FILE_BATCH_SIZE,
            worker_concurrency = GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
            elapsed_ms = elapsed_ms_u64(started_at),
            "title library scan completed"
        );

        Ok(summary)
    }

    async fn finalize_title_scan_file(
        &self,
        title: &Title,
        plan: PlannedTitleScanFile,
        analysis_outcome: Option<MediaAnalysisOutcome>,
        scan_mode: LibraryScanMode,
        episode_links: &mut HashSet<(String, String)>,
        summary: &mut LibraryScanSummary,
        db_elapsed: &mut Duration,
    ) -> TitleScanFinalizeOutcome {
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
        let mut title_updated = false;

        let file_id = match &plan.record {
            PlannedTitleScanRecord::Existing {
                file_id,
                should_refresh_source_signature,
                ..
            } => {
                summary.skipped += 1;
                if *should_refresh_source_signature {
                    let db_started = Instant::now();
                    let update_result = self
                        .services
                        .media_files
                        .update_media_file_source_signature(
                            file_id,
                            plan.snapshot.size_bytes,
                            source_signature_scheme.clone(),
                            source_signature_value.clone(),
                        )
                        .await;
                    *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    if let Err(error) = update_result {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_id = %file_id,
                            "failed to refresh media file source signature during title scan"
                        );
                    }
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

                let db_started = Instant::now();
                let insert_result = self
                    .services
                    .media_files
                    .insert_media_file(&media_file_input)
                    .await;
                *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());

                match insert_result {
                    Ok(file_id) => {
                        summary.imported += 1;
                        title_updated = true;
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
                        return TitleScanFinalizeOutcome {
                            progress: TitleScanProgressDelta::failed(1),
                            title_updated: false,
                        };
                    }
                }
            }
        };

        let should_link_target_episodes = !matches!(
            (&scan_mode, &plan.record),
            (
                LibraryScanMode::Additive,
                PlannedTitleScanRecord::Existing { .. }
            )
        );

        for episode in &plan.target_episodes {
            if !should_link_target_episodes {
                continue;
            }
            if episode_links.insert((file_id.clone(), episode.id.clone())) {
                title_updated = true;
                let db_started = Instant::now();
                let link_result = self
                    .services
                    .media_files
                    .link_file_to_episode(&file_id, &episode.id)
                    .await;
                *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                if let Err(error) = link_result {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        episode_id = %episode.id,
                        file_id = %file_id,
                        "failed to link scanned file to episode"
                    );
                }
            }
            crate::app_usecase_import::mark_wanted_completed(
                self,
                &title.id,
                Some(&episode.id),
                None,
            )
            .await;
        }

        if let Some(outcome) = analysis_outcome {
            match outcome {
                MediaAnalysisOutcome::Valid(analysis) => {
                    let db_started = Instant::now();
                    let update_result = self
                        .services
                        .media_files
                        .update_media_file_analysis(&file_id, *analysis)
                        .await;
                    *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    if let Err(error) = update_result {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_id = %file_id,
                            "failed to persist scanned media analysis"
                        );
                    }
                }
                MediaAnalysisOutcome::Invalid(error_message) => {
                    let db_started = Instant::now();
                    let mark_result = self
                        .services
                        .media_files
                        .mark_scan_failed(&file_id, &error_message)
                        .await;
                    *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    if let Err(error) = mark_result {
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

        TitleScanFinalizeOutcome {
            progress: TitleScanProgressDelta::completed(1),
            title_updated,
        }
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
        } else {
            self.emit_title_updated_activity(None, title).await;
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

    #[cfg(unix)]
    #[tokio::test]
    async fn list_child_directories_deduplicates_symlinked_show_folders() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("Real Show");
        let link = dir.path().join("Linked Show");
        std::fs::create_dir_all(&target).expect("target dir");
        symlink(&target, &link).expect("symlink");

        let child_dirs = list_child_directories(dir.path())
            .await
            .expect("child dirs");

        assert_eq!(child_dirs, vec![link]);
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

struct GroupedTitleMediaFile {
    file: TitleMediaFile,
    episode_ids: Vec<String>,
}

struct ResolvedSeriesRenameMetadata {
    collection_id: Option<String>,
    season: String,
    season_order: String,
    episode: String,
    absolute_episode: String,
    episode_title: String,
}

fn build_series_rename_plan_items_from_media_files(
    title: &Title,
    mut collections: Vec<Collection>,
    episodes: Vec<Episode>,
    media_files: Vec<TitleMediaFile>,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> Vec<RenamePlanItem> {
    collections.sort_by(|left, right| left.id.cmp(&right.id));

    let collections_by_id = collections
        .iter()
        .cloned()
        .map(|collection| (collection.id.clone(), collection))
        .collect::<HashMap<_, _>>();
    let episodes_by_id = episodes
        .into_iter()
        .map(|episode| (episode.id.clone(), episode))
        .collect::<HashMap<_, _>>();

    let mut grouped_files = group_title_media_files(media_files);
    grouped_files.sort_by(|left, right| {
        left.file
            .file_path
            .cmp(&right.file.file_path)
            .then_with(|| left.file.id.cmp(&right.file.id))
    });

    grouped_files
        .into_iter()
        .map(|source| {
            build_series_media_file_rename_plan_item(
                title,
                &collections,
                &collections_by_id,
                &episodes_by_id,
                source,
                template,
                collision_policy,
                missing_metadata_policy,
                planned_targets,
            )
        })
        .collect()
}

fn group_title_media_files(media_files: Vec<TitleMediaFile>) -> Vec<GroupedTitleMediaFile> {
    let mut grouped: Vec<GroupedTitleMediaFile> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();

    for media_file in media_files {
        if let Some(index) = indexes.get(&media_file.id).copied() {
            if let Some(episode_id) = media_file.episode_id.as_ref()
                && !grouped[index]
                    .episode_ids
                    .iter()
                    .any(|value| value == episode_id)
            {
                grouped[index].episode_ids.push(episode_id.clone());
            }
            continue;
        }

        let episode_ids = media_file
            .episode_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        indexes.insert(media_file.id.clone(), grouped.len());
        grouped.push(GroupedTitleMediaFile {
            file: media_file,
            episode_ids,
        });
    }

    grouped
}

fn build_series_media_file_rename_plan_item(
    title: &Title,
    collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: GroupedTitleMediaFile,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let media_file_id = Some(source.file.id.clone());
    let current_path = source.file.file_path.clone();
    if current_path.trim().is_empty() {
        return RenamePlanItem {
            collection_id: None,
            media_file_id,
            current_path,
            proposed_path: None,
            normalized_filename: None,
            collision: false,
            reason_code: "no_source_path".into(),
            write_action: RenameWriteAction::Skip,
            source_size_bytes: None,
            source_mtime_unix_ms: None,
        };
    }

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
            collection_id: None,
            media_file_id,
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
    let rename_metadata = resolve_series_rename_metadata(
        collections,
        collections_by_id,
        episodes_by_id,
        &source,
        &parsed,
    );
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let extension = current_file
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let quality = source
        .file
        .quality_label
        .clone()
        .or(parsed.quality.clone())
        .unwrap_or_default();

    let mut tokens = BTreeMap::new();
    tokens.insert("title".to_string(), title_token.clone());
    tokens.insert("year".to_string(), year_token.unwrap_or_default());
    tokens.insert("season".to_string(), rename_metadata.season.clone());
    tokens.insert(
        "season_order".to_string(),
        rename_metadata.season_order.clone(),
    );
    tokens.insert("episode".to_string(), rename_metadata.episode.clone());
    tokens.insert(
        "absolute_episode".to_string(),
        rename_metadata.absolute_episode.clone(),
    );
    tokens.insert(
        "episode_title".to_string(),
        rename_metadata.episode_title.clone(),
    );
    tokens.insert("quality".to_string(), quality);
    tokens.insert(
        "source".to_string(),
        source
            .file
            .source_type
            .clone()
            .or(parsed.source.clone())
            .unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        source
            .file
            .video_codec_parsed
            .clone()
            .or(parsed.video_codec.clone())
            .unwrap_or_default(),
    );
    tokens.insert(
        "audio_codec".to_string(),
        source
            .file
            .audio_codec_parsed
            .clone()
            .or(parsed.audio.clone())
            .unwrap_or_default(),
    );
    tokens.insert(
        "audio_channels".to_string(),
        parsed.audio_channels.clone().unwrap_or_default(),
    );
    tokens.insert(
        "group".to_string(),
        source
            .file
            .release_group
            .clone()
            .or(parsed.release_group.clone())
            .unwrap_or_default(),
    );
    tokens.insert("ext".to_string(), extension.clone());

    let mut rendered = render_rename_template(template, &tokens);
    if rendered.is_empty() {
        if matches!(missing_metadata_policy, RenameMissingMetadataPolicy::Skip) {
            return RenamePlanItem {
                collection_id: rename_metadata.collection_id,
                media_file_id,
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
    let collection_id = rename_metadata.collection_id;

    if proposed_path_str == current_path {
        return RenamePlanItem {
            collection_id,
            media_file_id,
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
            collection_id,
            media_file_id,
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
                collection_id,
                media_file_id,
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
                collection_id,
                media_file_id,
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
                collection_id,
                media_file_id,
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Error,
                source_size_bytes,
                source_mtime_unix_ms,
            },
        };
    }

    RenamePlanItem {
        collection_id,
        media_file_id,
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

fn resolve_series_rename_metadata(
    collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: &GroupedTitleMediaFile,
    parsed: &ParsedReleaseMetadata,
) -> ResolvedSeriesRenameMetadata {
    if source.episode_ids.is_empty()
        && let Some(collection) = collections.iter().find(|collection| {
            collection.collection_type == CollectionType::Interstitial
                && collection.ordered_path.as_deref() == Some(source.file.file_path.as_str())
        })
    {
        let (season, episode) =
            parse_interstitial_season_episode(collection.interstitial_season_episode.as_deref())
                .unwrap_or_else(|| ("0".to_string(), "1".to_string()));

        return ResolvedSeriesRenameMetadata {
            collection_id: Some(collection.id.clone()),
            season_order: non_empty_owned(collection.narrative_order.clone())
                .or_else(|| non_empty_string(&collection.collection_index))
                .unwrap_or_else(|| season.clone()),
            absolute_episode: parsed
                .episode
                .as_ref()
                .and_then(|episode_meta| episode_meta.absolute_episode)
                .map(|value| format!("{value:03}"))
                .unwrap_or_else(|| episode.clone()),
            episode_title: collection
                .interstitial_movie
                .as_ref()
                .map(|movie| movie.name.clone())
                .unwrap_or_default(),
            season,
            episode,
        };
    }

    let linked_episodes =
        select_sorted_episodes(&source.episode_ids, episodes_by_id, collections_by_id);
    if let Some(primary_episode) = linked_episodes.first().copied() {
        let collection = primary_episode
            .collection_id
            .as_deref()
            .and_then(|collection_id| collections_by_id.get(collection_id));
        let parsed_episode = parsed.episode.as_ref();
        let season = non_empty_owned(primary_episode.season_number.clone())
            .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
            .or_else(|| {
                parsed_episode
                    .and_then(|value| value.season)
                    .map(|value| value.to_string())
            })
            .unwrap_or_default();
        let episode = format_number_token(collect_episode_numbers(&linked_episodes), 2, false)
            .or_else(|| non_empty_owned(primary_episode.episode_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_episode_token))
            .unwrap_or_default();

        return ResolvedSeriesRenameMetadata {
            collection_id: None,
            season_order: collection
                .and_then(|value| non_empty_owned(value.narrative_order.clone()))
                .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
                .or_else(|| non_empty_owned(primary_episode.season_number.clone()))
                .unwrap_or_else(|| season.clone()),
            absolute_episode: format_number_token(
                collect_absolute_episode_numbers(&linked_episodes),
                3,
                true,
            )
            .or_else(|| normalize_absolute_episode_token(primary_episode.absolute_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_absolute_episode_token))
            .unwrap_or_else(|| episode.clone()),
            episode_title: join_episode_titles(&linked_episodes).unwrap_or_default(),
            season,
            episode,
        };
    }

    let parsed_episode = parsed.episode.as_ref();
    let season = parsed_episode
        .and_then(|value| value.season)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let episode = parsed_episode
        .and_then(parsed_episode_token)
        .unwrap_or_default();

    ResolvedSeriesRenameMetadata {
        collection_id: None,
        season_order: if season.is_empty() {
            String::new()
        } else {
            season.clone()
        },
        absolute_episode: parsed_episode
            .and_then(parsed_absolute_episode_token)
            .unwrap_or_else(|| episode.clone()),
        episode_title: String::new(),
        season,
        episode,
    }
}

fn select_sorted_episodes<'a>(
    episode_ids: &[String],
    episodes_by_id: &'a HashMap<String, Episode>,
    collections_by_id: &HashMap<String, Collection>,
) -> Vec<&'a Episode> {
    let mut episodes = episode_ids
        .iter()
        .filter_map(|episode_id| episodes_by_id.get(episode_id))
        .collect::<Vec<_>>();
    episodes.sort_by_key(|episode| episode_sort_key(episode, collections_by_id));
    episodes
}

fn collect_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.episode_number.as_deref()))
        .collect()
}

fn collect_absolute_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.absolute_number.as_deref()))
        .collect()
}

fn join_episode_titles(episodes: &[&Episode]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut titles = Vec::new();

    for episode in episodes {
        let Some(title) = episode
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let normalized = title.to_ascii_lowercase();
        if seen.insert(normalized) {
            titles.push(title.to_string());
        }
    }

    if titles.is_empty() {
        None
    } else {
        Some(titles.join(" + "))
    }
}

fn format_number_token(mut numbers: Vec<u32>, width: usize, pad_single: bool) -> Option<String> {
    if numbers.is_empty() {
        return None;
    }

    numbers.sort_unstable();
    numbers.dedup();

    if numbers.len() == 1 {
        let value = numbers[0];
        return Some(if pad_single {
            format!("{value:0width$}")
        } else {
            value.to_string()
        });
    }

    Some(
        numbers
            .into_iter()
            .map(|value| format!("{value:0width$}"))
            .collect::<Vec<_>>()
            .join("-"),
    )
}

fn parsed_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.episode_numbers.is_empty() {
        format_number_token(parsed_episode.episode_numbers.clone(), 2, false)
    } else {
        parsed_episode
            .first_episode()
            .map(|value| value.to_string())
    }
}

fn parsed_absolute_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.absolute_episode_numbers.is_empty() {
        format_number_token(parsed_episode.absolute_episode_numbers.clone(), 3, true)
    } else {
        parsed_episode
            .absolute_episode
            .map(|value| format!("{value:03}"))
    }
}

fn episode_sort_key(
    episode: &Episode,
    collections_by_id: &HashMap<String, Collection>,
) -> (u32, u32, u32, u32, String) {
    let collection = episode
        .collection_id
        .as_deref()
        .and_then(|collection_id| collections_by_id.get(collection_id));

    (
        collection
            .and_then(|value| {
                parse_sort_number(
                    value
                        .narrative_order
                        .as_deref()
                        .or(Some(value.collection_index.as_str())),
                )
            })
            .unwrap_or(u32::MAX),
        parse_sort_number(episode.season_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.episode_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.absolute_number.as_deref()).unwrap_or(u32::MAX),
        episode.id.clone(),
    )
}

fn parse_sort_number(value: Option<&str>) -> Option<u32> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn parse_interstitial_season_episode(value: Option<&str>) -> Option<(String, String)> {
    let raw = value?.trim();
    let stripped = raw.strip_prefix('S')?;
    let (season, episode) = stripped.split_once('E')?;
    let season = season.trim_start_matches('0');
    let episode = episode.trim_start_matches('0');
    Some((
        if season.is_empty() {
            "0".to_string()
        } else {
            season.to_string()
        },
        if episode.is_empty() {
            "0".to_string()
        } else {
            episode.to_string()
        },
    ))
}

fn non_empty_owned(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    })
}

fn normalize_absolute_episode_token(value: Option<String>) -> Option<String> {
    non_empty_owned(value).map(|value| match value.parse::<u32>() {
        Ok(number) => format!("{number:03}"),
        Err(_) => value,
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
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
        .filter(|item| matches!(item.write_action, RenameWriteAction::Move))
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
            media_file_id: None,
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
            media_file_id: None,
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
                media_file_id: None,
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
            media_file_id: None,
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
            media_file_id: None,
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
                media_file_id: None,
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
                media_file_id: None,
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
                media_file_id: None,
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Error,
                source_size_bytes,
                source_mtime_unix_ms,
            },
        };
    }

    RenamePlanItem {
        collection_id: Some(collection.id.clone()),
        media_file_id: None,
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
            media_file_id: None,
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
            media_file_id: None,
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
                media_file_id: None,
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
            media_file_id: None,
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
            media_file_id: None,
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
                media_file_id: None,
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
                media_file_id: None,
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
                media_file_id: None,
                current_path,
                proposed_path: Some(proposed_path_str),
                normalized_filename: Some(rendered),
                collision: true,
                reason_code: "collision_existing".into(),
                write_action: RenameWriteAction::Error,
                source_size_bytes,
                source_mtime_unix_ms,
            },
        };
    }

    RenamePlanItem {
        collection_id: Some(collection.id.clone()),
        media_file_id: None,
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
        analyzed: bool,
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
            video_codec: analyzed.then(|| "h264".into()),
            video_width: analyzed.then_some(1920),
            video_height: analyzed.then_some(1080),
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: analyzed.then(|| "aac".into()),
            audio_channels: analyzed.then_some(2),
            audio_bitrate_kbps: None,
            audio_languages: vec![],
            audio_streams: vec![],
            subtitle_languages: vec![],
            subtitle_codecs: vec![],
            subtitle_streams: vec![],
            has_multiaudio: false,
            duration_seconds: analyzed.then_some(1440),
            num_chapters: None,
            container_format: analyzed.then(|| "matroska".into()),
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
        let media_file = build_test_media_file(1234, None, None, true);
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
    fn title_media_file_matches_snapshot_requires_persisted_analysis() {
        let media_file =
            build_test_media_file(1234, Some("unix_mtime_nsec_v1"), Some("1:2"), false);
        let snapshot = FileSourceSnapshot {
            size_bytes: 1234,
            signature: Some(FileSourceSignature {
                scheme: "unix_mtime_nsec_v1".into(),
                value: "1:2".into(),
            }),
        };

        assert!(!title_media_file_matches_snapshot(&media_file, &snapshot));
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
    fn resolve_target_episodes_from_lookup_keeps_explicit_standard_episode_season() {
        let collection = Collection {
            id: "collection-4".into(),
            title_id: "title-1".into(),
            collection_type: CollectionType::Season,
            collection_index: "4".into(),
            label: Some("Season 4".into()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("29".into()),
            last_episode_number: Some("30".into()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode = Episode {
            id: "episode-29".into(),
            title_id: "title-1".into(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("29".into()),
            season_number: Some("4".into()),
            episode_label: Some("S04E29".into()),
            title: Some("The Final Chapters Special 1".into()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let lookup = build_title_episode_lookup(
            std::slice::from_ref(&collection),
            std::slice::from_ref(&episode),
        );
        let ep_meta = crate::ParsedEpisodeMetadata {
            season: Some(4),
            episode_numbers: vec![29],
            special_kind: Some(crate::ParsedSpecialKind::Special),
            special_absolute_episode_numbers: vec![1],
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };

        let episodes = resolve_target_episodes_from_lookup(&ep_meta, "4", &lookup);

        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, episode.id);
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
