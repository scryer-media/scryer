use super::*;
use crate::library_scan_titles::find_existing_movie_title_index_for_metadata_match;
use crate::library_scan_unmatched::{
    IgnoredLibraryScanItemArgs, LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
    LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER, build_title_bound_unmatched_scan_item,
    persist_ignored_library_scan_item, persist_library_scan_unmatched_item,
    series_unmatched_display_name,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

fn normalize_title_folder_path(path: Option<String>) -> Option<String> {
    path.filter(|value| !value.is_empty())
}

#[expect(
    clippy::too_many_arguments,
    reason = "folder ownership conflicts retain the complete pending-import scan context"
)]
async fn persist_title_folder_ownership_conflict(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: Option<&str>,
    title: &Title,
    item_path: &str,
    display_name: &str,
    query: &str,
    year_hint: Option<u32>,
) -> AppResult<LibraryScanUnmatchedItem> {
    let item = build_title_bound_unmatched_scan_item(
        facet,
        library_id,
        &title.id,
        session_id,
        library_path,
        item_path,
        display_name,
        query,
        year_hint,
        LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER,
        // Folder-ownership conflicts are recorded against a folder, not a file.
        None,
    );
    persist_library_scan_unmatched_item(app, &item).await?;
    Ok(item)
}

async fn claim_series_candidate_folder(
    app: &AppUseCase,
    title: &mut Title,
    candidate: &PreparedSeriesLibraryScanCandidate,
) -> AppResult<()> {
    crate::folder_ownership::claim_title_folder_if_missing(app, title, &candidate.folder_path).await
}

async fn prepare_series_title_for_candidate(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: Option<&str>,
    title: &mut Title,
    candidate: &PreparedSeriesLibraryScanCandidate,
) -> AppResult<Option<LibraryScanUnmatchedItem>> {
    if crate::folder_ownership::title_owns_another_folder(title, &candidate.folder_path) {
        crate::folder_ownership::unlink_title_media_in_folder(app, title, &candidate.folder_path)
            .await?;
        let item_path = candidate.item_path();
        let item_path = item_path.trim();
        let display_name = series_unmatched_display_name(candidate);
        let query = candidate.query.trim();
        let query = if query.is_empty() {
            display_name.as_str()
        } else {
            query
        };
        return persist_title_folder_ownership_conflict(
            app,
            facet,
            library_id,
            library_path,
            session_id,
            title,
            item_path,
            &display_name,
            query,
            candidate.year_hint,
        )
        .await
        .map(Some);
    }

    claim_series_candidate_folder(app, title, candidate).await?;
    Ok(None)
}

async fn persist_movie_title_folder_ownership_conflict(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: Option<&str>,
    title: &Title,
    candidate: &PreparedMovieLibraryScanCandidate,
) -> AppResult<LibraryScanUnmatchedItem> {
    let display_name = candidate.file.display_name.trim();
    let display_name = if display_name.is_empty() {
        candidate.file.path.as_str()
    } else {
        display_name
    };
    let query = candidate.query.trim();
    let query = if query.is_empty() {
        display_name
    } else {
        query
    };
    if let Some(folder_path) = scanned_movie_entry_folder_path(
        Path::new(library_path),
        &candidate.file.path,
        candidate.representative_is_directory,
    ) {
        crate::folder_ownership::unlink_title_media_in_folder(
            app,
            title,
            &stored_path_to_path_buf(&folder_path),
        )
        .await?;
    }
    persist_title_folder_ownership_conflict(
        app,
        facet,
        library_id,
        library_path,
        session_id,
        title,
        &candidate.file.path,
        display_name,
        query,
        candidate.year_hint,
    )
    .await
}

fn movie_scan_folder_conflicts(
    canonical_folder_path: Option<&str>,
    scan_folder_path: Option<&str>,
) -> bool {
    let canonical_folder_path =
        normalize_title_folder_path(canonical_folder_path.map(ToString::to_string));
    let scan_folder_path = normalize_title_folder_path(scan_folder_path.map(ToString::to_string));
    matches!(
        (canonical_folder_path, scan_folder_path),
        (Some(canonical), Some(scan))
            if !crate::stored_paths::folder_paths_match(&canonical, &scan)
    )
}

fn scanned_movie_entry_folder_path(
    scan_root: &Path,
    representative_path: &str,
    representative_is_directory: bool,
) -> Option<String> {
    let representative_path = representative_path.trim();
    if representative_path.is_empty() {
        return None;
    }

    let item_path = stored_path_to_path_buf(representative_path);
    if let Ok(relative) = item_path.strip_prefix(scan_root)
        && let Some(first_component) = relative.components().next()
    {
        let entry_path = scan_root.join(first_component.as_os_str());
        let entry_path = path_to_stored_string(&entry_path).trim().to_string();
        if entry_path.is_empty() {
            return None;
        }
        if entry_path == representative_path {
            return representative_is_directory.then_some(entry_path);
        }
        return Some(entry_path);
    }

    let parent = path_to_stored_string(item_path.parent()?)
        .trim()
        .to_string();
    if parent.is_empty() || parent == path_to_stored_string(scan_root) {
        None
    } else {
        Some(parent)
    }
}

async fn sync_movie_title_folder_path_for_scan(
    app: &AppUseCase,
    title: &mut Title,
    scan_root: &Path,
    representative_path: &str,
    representative_is_directory: bool,
) -> AppResult<(Option<String>, Option<String>)> {
    let scan_folder_path = scanned_movie_entry_folder_path(
        scan_root,
        representative_path,
        representative_is_directory,
    );
    let current_folder_path = normalize_title_folder_path(title.folder_path.clone());

    if current_folder_path.is_some() {
        return Ok((current_folder_path, scan_folder_path));
    }

    let Some(folder_path) = scan_folder_path.as_deref() else {
        return Ok((None, scan_folder_path));
    };

    crate::folder_ownership::claim_title_folder_if_missing(
        app,
        title,
        &stored_path_to_path_buf(folder_path),
    )
    .await?;

    Ok((
        normalize_title_folder_path(title.folder_path.clone()),
        scan_folder_path,
    ))
}

fn sync_existing_title_folder_path_in_memory(existing_titles: &mut [Title], title: &Title) {
    if let Some(existing) = existing_titles
        .iter_mut()
        .find(|existing| existing.id == title.id)
    {
        existing.folder_path = title.folder_path.clone();
    }
}

fn find_existing_movie_title_index(
    candidate: &PreparedMovieLibraryScanCandidate,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(identity_hint) = candidate
        .identity_hint
        .as_ref()
        .filter(|hint| hint.is_external_import_hint())
    {
        if let Some(tmdb_id) = identity_hint.tmdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tmdb_id.get(tmdb_id)
        {
            return Some(index);
        }
        if let Some(imdb_id) = identity_hint.imdb_id.as_deref()
            && let Some(&index) = existing_titles_by_imdb_id.get(imdb_id)
        {
            return Some(index);
        }
        if let Some(tvdb_id) = identity_hint.tvdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
        {
            return Some(index);
        }
        return None;
    }

    if let Some(tvdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_imdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.imdb_id.as_deref())
        .and_then(crate::normalize::normalize_imdb_id)
        && let Some(&index) = existing_titles_by_imdb_id.get(&nfo_imdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_tmdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tmdb_id.as_deref())
        .map(str::to_string)
        && let Some(&index) = existing_titles_by_tmdb_id.get(&nfo_tmdb_id)
    {
        return Some(index);
    }

    if let Some(parsed_imdb_id) = candidate
        .parsed_release
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
        && let Some(&index) = existing_titles_by_imdb_id.get(&parsed_imdb_id)
    {
        return Some(index);
    }

    if let Some(parsed_tmdb_id) = candidate.parsed_release.tmdb_id.as_deref()
        && let Some(&index) = existing_titles_by_tmdb_id.get(parsed_tmdb_id)
    {
        return Some(index);
    }

    candidate.query_variants.iter().find_map(|query_variant| {
        let normalized = crate::title_matching::canonical_lookup_key(query_variant);
        pick_same_name_title_index(
            existing_titles_by_name.get(&normalized)?,
            existing_titles,
            candidate.year_hint,
            |title| title_folder_contains_stored_path(title, &candidate.file.path),
        )
    })
}

/// Among the same-name titles under one name key, the one a scanned path
/// belongs to: it must be year-compatible, and when several are, the title
/// whose owned folder already contains the path wins — a title owns exactly one
/// folder, so binding to a same-name sibling could only end as "already owns
/// another folder". Otherwise the first compatible title.
fn pick_same_name_title_index(
    indexes: &[usize],
    existing_titles: &[Title],
    year_hint: Option<u32>,
    owns_scanned_path: impl Fn(&Title) -> bool,
) -> Option<usize> {
    let mut first_compatible = None;
    for &index in indexes {
        let Some(title) = existing_titles.get(index) else {
            continue;
        };
        if !title_year_compatible(title, year_hint) {
            continue;
        }
        if owns_scanned_path(title) {
            return Some(index);
        }
        first_compatible.get_or_insert(index);
    }
    first_compatible
}

fn title_folder_contains_stored_path(title: &Title, path: &str) -> bool {
    normalize_title_folder_path(title.folder_path.clone())
        .is_some_and(|folder| crate::stored_paths::stored_path_is_within_folder(&folder, path))
}

fn find_existing_series_title_index(
    candidate: &PreparedSeriesLibraryScanCandidate,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(identity_hint) = candidate
        .identity_hint
        .as_ref()
        .filter(|hint| hint.is_external_import_hint())
    {
        if let Some(tvdb_id) = identity_hint.tvdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
        {
            return Some(index);
        }
        if let Some(imdb_id) = identity_hint.imdb_id.as_deref()
            && let Some(&index) = existing_titles_by_imdb_id.get(imdb_id)
        {
            return Some(index);
        }
        if let Some(tmdb_id) = identity_hint.tmdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tmdb_id.get(tmdb_id)
        {
            return Some(index);
        }
        return None;
    }

    if let Some(tvdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_imdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.imdb_id.as_deref())
        .and_then(crate::normalize::normalize_imdb_id)
        && let Some(&index) = existing_titles_by_imdb_id.get(&nfo_imdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_tmdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tmdb_id.as_deref())
        .map(str::to_string)
        && let Some(&index) = existing_titles_by_tmdb_id.get(&nfo_tmdb_id)
    {
        return Some(index);
    }

    candidate
        .title_match_candidates
        .iter()
        .find_map(|name_key| {
            pick_same_name_title_index(
                existing_titles_by_name.get(name_key)?,
                existing_titles,
                candidate.year_hint,
                |title| crate::folder_ownership::title_owns_folder(title, &candidate.folder_path),
            )
        })
}

async fn load_existing_title_for_media_file_path(
    app: &AppUseCase,
    file_path: &str,
) -> AppResult<Option<Title>> {
    let Some(existing_media_file) = app
        .services
        .library
        .media_files
        .get_media_file_by_path(file_path)
        .await?
    else {
        return Ok(None);
    };

    app.services
        .catalog
        .titles
        .get_by_id(&existing_media_file.title_id)
        .await
}

enum MovieCandidateResolution {
    Ready(Box<Title>),
    Skipped,
    Unresolved(Box<PreparedMovieLibraryScanCandidate>),
}

enum MovieMetadataResolution {
    Ready(Title),
    ReadyCreated { index: usize, title: Title },
    CreateFailed(AppError),
    Unmatched,
}

async fn create_title_without_hydration_for_library_scan(
    app: &AppUseCase,
    actor: &User,
    library_id: &str,
    request: NewTitle,
) -> AppResult<CreateTitleOutcome> {
    app.create_title_without_hydration_after_library_authorization(
        actor,
        request,
        library_id.to_string(),
    )
    .await
}

pub(super) fn movie_title_work(
    title: Title,
    pre_scanned_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    cleanup: LibraryScanMovieCleanupContext,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Movie(cleanup),
        scope: LibraryScanTitleWorkScope::ScopedFiles(pre_scanned_files),
        mode,
        created_in_scan,
    }
}

fn movie_cleanup_context(
    canonical_folder_path: Option<String>,
    scan_folder_path: Option<String>,
) -> LibraryScanMovieCleanupContext {
    LibraryScanMovieCleanupContext {
        canonical_folder_path,
        scan_folder_path,
        ..Default::default()
    }
}

fn merge_default_movie_title_work(
    executor: &mut dyn LibraryScanTitleWorkQueue,
    title: Title,
    discovered_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    cleanup: LibraryScanMovieCleanupContext,
    created_in_scan: bool,
) -> bool {
    executor.enqueue(movie_title_work(
        title,
        discovered_files,
        mode,
        cleanup,
        created_in_scan,
    ))
}

fn merge_movie_refresh_title_work(
    executor: &mut dyn LibraryScanTitleWorkQueue,
    title: Title,
    scope: LibraryScanTitleWorkScope,
    mode: LibraryScanTitleWalkMode,
    cleanup: LibraryScanMovieCleanupContext,
    created_in_scan: bool,
) -> bool {
    executor.enqueue(LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Movie(cleanup),
        scope,
        mode,
        created_in_scan,
    })
}

fn movie_refresh_title_work_scope(
    scan_folder_path: Option<&str>,
    discovered_files: Vec<LibraryFile>,
) -> LibraryScanTitleWorkScope {
    if scan_folder_path.is_some() && discovered_files.is_empty() {
        LibraryScanTitleWorkScope::FullFolder
    } else {
        LibraryScanTitleWorkScope::ScopedFiles(discovered_files)
    }
}

pub(super) fn episodic_title_work(
    title: Title,
    pre_scanned_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Episodic,
        scope: LibraryScanTitleWorkScope::ScopedFiles(pre_scanned_files),
        mode,
        created_in_scan,
    }
}

fn deferred_episodic_title_work(
    title: Title,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Episodic,
        scope: LibraryScanTitleWorkScope::FullFolder,
        mode,
        created_in_scan,
    }
}

pub(super) async fn scan_episodic_title_directory_for_progress_metrics(
    library_scanner: Arc<dyn LibraryScanner>,
    folder_path: &Path,
) -> AppResult<LibraryDirectoryScanResult> {
    library_scanner
        .scan_directory_for_progress_with_metrics(path_to_stored_string(folder_path).as_str())
        .await
}

async fn merge_series_title_work_for_index(
    app: &AppUseCase,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut [Title],
    index: usize,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> AppResult<()> {
    crate::folder_ownership::claim_title_folder_if_missing(
        app,
        &mut existing_titles[index],
        folder_path,
    )
    .await?;
    executor.enqueue(deferred_episodic_title_work(
        existing_titles[index].clone(),
        mode,
        created_in_scan,
    ));
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "series title insertion updates shared indexes and executor state together"
)]
async fn append_series_title_and_merge_work(
    app: &AppUseCase,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> AppResult<usize> {
    let index = append_series_title(
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
        title,
    );
    merge_series_title_work_for_index(
        app,
        executor,
        existing_titles,
        index,
        folder_path,
        mode,
        created_in_scan,
    )
    .await?;
    Ok(index)
}

async fn resolve_movie_scan_candidate(
    candidate: PreparedMovieLibraryScanCandidate,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) -> AppResult<MovieCandidateResolution> {
    if let Some(index) = find_existing_movie_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        return Ok(MovieCandidateResolution::Ready(Box::new(
            existing_titles[index].clone(),
        )));
    }

    if !candidate.metadata_lookup_attempted {
        return Ok(MovieCandidateResolution::Skipped);
    }

    Ok(MovieCandidateResolution::Unresolved(Box::new(candidate)))
}

#[expect(
    clippy::too_many_arguments,
    reason = "metadata matches update the same in-memory title indexes and creation context together"
)]
async fn resolve_movie_metadata_match(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_smg_id: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) -> AppResult<MovieMetadataResolution> {
    let selected_metadata =
        select_movie_metadata_from_batch_results(candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        return Ok(MovieMetadataResolution::Unmatched);
    };

    if let Some(index) = find_existing_movie_title_index_for_metadata_match(
        &selected,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_smg_id,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        return Ok(MovieMetadataResolution::Ready(
            existing_titles[index].clone(),
        ));
    }

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            let created_title = created.title;
            let index = append_movie_title(
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_smg_id,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created_title.clone(),
            );
            Ok(if was_created {
                MovieMetadataResolution::ReadyCreated {
                    index,
                    title: created_title,
                }
            } else {
                MovieMetadataResolution::Ready(created_title)
            })
        }
        Err(error) => Ok(MovieMetadataResolution::CreateFailed(error)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie full-scan processing coordinates shared scan state across a single candidate"
)]
pub(super) async fn process_movie_full_scan_candidate(
    app: &AppUseCase,
    _actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    _unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<Option<PreparedMovieLibraryScanCandidate>> {
    let discovered_files = candidate.discovered_files.clone();
    let item_path = normalize_library_scan_item_path(&candidate.file.path);
    let representative_path = candidate.file.path.clone();
    let representative_is_directory = candidate.representative_is_directory;
    let scan_root = Path::new(library_path);

    if let Some(mut title) =
        load_existing_title_for_media_file_path(app, &candidate.file.path).await?
    {
        let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
            app,
            &mut title,
            scan_root,
            &representative_path,
            representative_is_directory,
        )
        .await?;
        sync_existing_title_folder_path_in_memory(existing_titles, &title);
        if movie_scan_folder_conflicts(
            canonical_folder_path.as_deref(),
            scan_folder_path.as_deref(),
        ) {
            let conflict = persist_movie_title_folder_ownership_conflict(
                app,
                facet,
                library_id,
                library_path,
                Some(session_id),
                &title,
                &candidate,
            )
            .await?;
            _unmatched_items.push(conflict);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            return Ok(None);
        }
        let queued = merge_default_movie_title_work(
            executor,
            title,
            discovered_files,
            LibraryScanTitleWalkMode::Full,
            movie_cleanup_context(canonical_folder_path, scan_folder_path),
            false,
        );
        if queued {
            summary.matched += 1;
        }
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    let owned_folder_path = scanned_movie_entry_folder_path(
        scan_root,
        &representative_path,
        representative_is_directory,
    );
    if let Some((index, owned_folder_path)) = owned_folder_path.as_deref().and_then(|folder_path| {
        existing_titles
            .iter()
            .position(|title| {
                title.folder_path.as_deref().is_some_and(|owned| {
                    crate::stored_paths::folder_paths_match(owned, folder_path)
                })
            })
            .map(|index| (index, folder_path.to_string()))
    }) {
        let title = existing_titles[index].clone();
        let queued = merge_default_movie_title_work(
            executor,
            title,
            discovered_files,
            LibraryScanTitleWalkMode::Full,
            movie_cleanup_context(Some(owned_folder_path.clone()), Some(owned_folder_path)),
            false,
        );
        if queued {
            summary.matched += 1;
        }
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if !candidate.metadata_lookup_attempted {
        let display_name = candidate.file.display_name.trim();
        let display_name = if display_name.is_empty() {
            representative_path.as_str()
        } else {
            display_name
        };
        let query = candidate.query.trim();
        let query = if query.is_empty() {
            display_name
        } else {
            query
        };
        if let Err(error) = persist_ignored_library_scan_item(
            app,
            facet,
            library_id,
            IgnoredLibraryScanItemArgs {
                title_id: None,
                session_id: Some(session_id),
                library_path,
                item_path: &item_path,
                display_name,
                query,
                year_hint: candidate.year_hint,
                reason_code: LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
                error_message: None,
                size_bytes: candidate.file.size_bytes,
            },
        )
        .await
        {
            warn!(
                item_path = %item_path,
                error = %error,
                "failed to persist unusable movie scan evidence"
            );
        }
        summary.skipped += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    let pending_candidate = candidate.clone();
    match resolve_movie_scan_candidate(
        candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieCandidateResolution::Ready(title) => {
            let mut title = *title;
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &representative_path,
                representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if movie_scan_folder_conflicts(
                canonical_folder_path.as_deref(),
                scan_folder_path.as_deref(),
            ) {
                let conflict = persist_movie_title_folder_ownership_conflict(
                    app,
                    facet,
                    library_id,
                    library_path,
                    Some(session_id),
                    &title,
                    &pending_candidate,
                )
                .await?;
                _unmatched_items.push(conflict);
                summary.unmatched += 1;
                coordinator.mark_title_match_completed(1).await;
                return Ok(None);
            }
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::Skipped => {
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "series full-scan processing coordinates shared scan state across a single candidate"
)]
pub(super) async fn process_series_full_scan_candidate(
    app: &AppUseCase,
    _actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    summary: &mut LibraryScanSummary,
    _unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    let item_path = candidate.item_path().trim().to_string();
    if candidate.folder_name.as_deref().is_none() {
        if let Err(error) = persist_ignored_library_scan_item(
            app,
            facet,
            library_id,
            IgnoredLibraryScanItemArgs {
                title_id: None,
                session_id: Some(session_id),
                library_path,
                item_path: &item_path,
                display_name: &item_path,
                query: &item_path,
                year_hint: candidate.year_hint,
                reason_code: LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
                error_message: None,
                // Series candidates are folder-shaped; no single file size.
                size_bytes: None,
            },
        )
        .await
        {
            warn!(
                item_path = %item_path,
                error = %error,
                "failed to persist unusable series scan evidence"
            );
        }
        summary.skipped += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    let candidate_folder = candidate.folder_path.clone();
    if let Some(index) = existing_titles
        .iter()
        .position(|title| crate::folder_ownership::title_owns_folder(title, &candidate_folder))
    {
        merge_series_title_work_for_index(
            app,
            executor,
            existing_titles,
            index,
            &candidate_folder,
            LibraryScanTitleWalkMode::Full,
            false,
        )
        .await?;
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        if let Some(conflict) = prepare_series_title_for_candidate(
            app,
            facet,
            library_id,
            library_path,
            Some(session_id),
            &mut existing_titles[index],
            &candidate,
        )
        .await?
        {
            summary.unmatched += 1;
            _unmatched_items.push(conflict);
            coordinator.mark_title_match_completed(1).await;
            return Ok(None);
        }
        merge_series_title_work_for_index(
            app,
            executor,
            existing_titles,
            index,
            &candidate_folder,
            LibraryScanTitleWalkMode::Full,
            false,
        )
        .await?;
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if !candidate.metadata_lookup_attempted {
        let display_name = candidate
            .folder_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(item_path.as_str());
        let query = candidate.query.trim();
        let query = if query.is_empty() {
            display_name
        } else {
            query
        };
        if let Err(error) = persist_ignored_library_scan_item(
            app,
            facet,
            library_id,
            IgnoredLibraryScanItemArgs {
                title_id: None,
                session_id: Some(session_id),
                library_path,
                item_path: &item_path,
                display_name,
                query,
                year_hint: candidate.year_hint,
                reason_code: LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
                error_message: None,
                // Series candidates are folder-shaped; no single file size.
                size_bytes: None,
            },
        )
        .await
        {
            warn!(
                item_path = %item_path,
                error = %error,
                "failed to persist unusable series scan evidence"
            );
        }
        summary.skipped += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    Ok(Some(candidate))
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved movie scan candidates update shared scan state, indexes, and reporting together"
)]
pub(super) async fn process_resolved_movie_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_smg_id: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let discovered_files = candidate.discovered_files.clone();
    let scan_root = Path::new(library_path);
    match resolve_movie_metadata_match(
        app,
        actor,
        facet,
        library_id,
        &candidate,
        batch_search_results,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_smg_id,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieMetadataResolution::Ready(mut title) => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &candidate.file.path,
                candidate.representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if movie_scan_folder_conflicts(
                canonical_folder_path.as_deref(),
                scan_folder_path.as_deref(),
            ) {
                let conflict = persist_movie_title_folder_ownership_conflict(
                    app,
                    facet,
                    library_id,
                    library_path,
                    Some(session_id),
                    &title,
                    &candidate,
                )
                .await?;
                unmatched_items.push(conflict);
                summary.unmatched += 1;
                coordinator.mark_title_match_completed(1).await;
                return Ok(());
            }
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { mut title, .. } => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &candidate.file.path,
                candidate.representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                true,
            );
            if queued {
                summary.imported += 1;
                summary.matched += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::CreateFailed(error) => {
            warn!(
                file = %candidate.file.path,
                query = %candidate.query,
                error = %error,
                "movie scan: failed to create title from search result"
            );
            let unmatched_item = build_movie_unmatched_scan_item(
                facet,
                library_id,
                session_id,
                library_path,
                &candidate,
                batch_search_results,
                Some("title_create_from_search_failed"),
                Some(error.to_string()),
            );
            persist_library_scan_unmatched_item(app, &unmatched_item).await?;
            unmatched_items.push(unmatched_item);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::Unmatched => {
            let unmatched_item = build_movie_unmatched_scan_item(
                facet,
                library_id,
                session_id,
                library_path,
                &candidate,
                batch_search_results,
                None,
                None,
            );
            persist_library_scan_unmatched_item(app, &unmatched_item).await?;
            unmatched_items.push(unmatched_item);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved series scan candidates update shared scan state, indexes, and reporting together"
)]
pub(super) async fn process_resolved_series_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let item_path = candidate.item_path().trim().to_string();
    let Some(folder_name) = candidate.folder_name.as_deref() else {
        if let Err(error) = persist_ignored_library_scan_item(
            app,
            facet,
            library_id,
            IgnoredLibraryScanItemArgs {
                title_id: None,
                session_id: Some(session_id),
                library_path,
                item_path: &item_path,
                display_name: &item_path,
                query: &item_path,
                year_hint: candidate.year_hint,
                reason_code: LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
                error_message: None,
                // Series candidates are folder-shaped; no single file size.
                size_bytes: None,
            },
        )
        .await
        {
            warn!(
                item_path = %item_path,
                error = %error,
                "failed to persist unusable resolved series scan evidence"
            );
        }
        summary.skipped += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    };

    let selected_metadata =
        select_series_metadata_from_batch_results(&candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        debug!(
            folder = %folder_name,
            query = %candidate.query,
            "series scan: no metadata match"
        );
        let unmatched_item = build_series_unmatched_scan_item(
            facet,
            library_id,
            session_id,
            library_path,
            &candidate,
            batch_search_results,
            None,
            None,
        );
        persist_library_scan_unmatched_item(app, &unmatched_item).await?;
        unmatched_items.push(unmatched_item);
        summary.unmatched += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    };

    if let Some(index) = find_existing_title_index_for_metadata_match(
        &selected,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        if let Some(conflict) = prepare_series_title_for_candidate(
            app,
            facet,
            library_id,
            library_path,
            Some(session_id),
            &mut existing_titles[index],
            &candidate,
        )
        .await?
        {
            unmatched_items.push(conflict);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            return Ok(());
        }
        merge_series_title_work_for_index(
            app,
            executor,
            existing_titles,
            index,
            &candidate.folder_path,
            LibraryScanTitleWalkMode::Full,
            false,
        )
        .await?;
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    }

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            let mut created_title = created.title;
            if let Some(conflict) = prepare_series_title_for_candidate(
                app,
                facet,
                library_id,
                library_path,
                Some(session_id),
                &mut created_title,
                &candidate,
            )
            .await?
            {
                unmatched_items.push(conflict);
                summary.unmatched += 1;
                coordinator.mark_title_match_completed(1).await;
                return Ok(());
            }
            let candidate_folder = candidate.folder_path.clone();
            append_series_title_and_merge_work(
                app,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created_title,
                &candidate_folder,
                LibraryScanTitleWalkMode::Full,
                was_created,
            )
            .await?;
            if was_created {
                summary.imported += 1;
            }
            summary.matched += 1;
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
        }
        Err(error) => {
            warn!(
                folder = %folder_name,
                tvdb_id = %selected.tvdb_id,
                error = %error,
                "series scan: failed to create title from search"
            );
            let unmatched_item = build_series_unmatched_scan_item(
                facet,
                library_id,
                session_id,
                library_path,
                &candidate,
                batch_search_results,
                Some("title_create_from_search_failed"),
                Some(error.to_string()),
            );
            persist_library_scan_unmatched_item(app, &unmatched_item).await?;
            unmatched_items.push(unmatched_item);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
        }
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "series refresh matching carries scan identity, indexing, and work queue state"
)]
async fn refresh_existing_series_title_match(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    title: &mut Title,
    index: usize,
    candidate: &PreparedSeriesLibraryScanCandidate,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    if prepare_series_title_for_candidate(
        app,
        facet,
        library_id,
        library_path,
        Some(session_id),
        title,
        candidate,
    )
    .await?
    .is_some()
    {
        summary.unmatched += 1;
        return Ok(());
    }
    update_series_title_folder_path_index(existing_titles_by_folder_path, title, index);
    maybe_probe_existing_series_title_for_background_refresh(
        app,
        title,
        &candidate.folder_path,
        executor,
        summary,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "series refresh candidates need shared title indexes and executor state in one step"
)]
pub(super) async fn process_series_refresh_candidate(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    candidate: PreparedSeriesLibraryScanCandidate,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    if candidate.folder_name.as_deref().is_none() {
        summary.skipped += 1;
        return Ok(None);
    }

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        refresh_existing_series_title_match(
            app,
            facet,
            library_id,
            library_path,
            session_id,
            &mut existing_titles[index],
            index,
            &candidate,
            existing_titles_by_folder_path,
            executor,
            summary,
        )
        .await?;
        return Ok(None);
    }

    if candidate.query.trim().is_empty() {
        summary.skipped += 1;
        return Ok(None);
    }

    Ok(Some(candidate))
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved series refresh candidates update indexes and background work in one place"
)]
pub(super) async fn process_resolved_series_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let Some(folder_name) = candidate.folder_name.as_deref() else {
        summary.skipped += 1;
        return Ok(());
    };

    let selected_metadata =
        select_series_metadata_from_batch_results(&candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        summary.unmatched += 1;
        return Ok(());
    };

    if let Some(index) = find_existing_title_index_for_metadata_match(
        &selected,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        refresh_existing_series_title_match(
            app,
            facet,
            library_id,
            library_path,
            session_id,
            &mut existing_titles[index],
            index,
            &candidate,
            existing_titles_by_folder_path,
            executor,
            summary,
        )
        .await?;
        return Ok(());
    }

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            let mut created_title = created.title;
            if prepare_series_title_for_candidate(
                app,
                facet,
                library_id,
                library_path,
                Some(session_id),
                &mut created_title,
                &candidate,
            )
            .await?
            .is_some()
            {
                summary.unmatched += 1;
                return Ok(());
            }
            let index = append_series_title_and_merge_work(
                app,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created_title,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Additive,
                was_created,
            )
            .await?;
            update_series_title_folder_path_index(
                existing_titles_by_folder_path,
                &existing_titles[index],
                index,
            );
            summary.matched += 1;
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

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie refresh candidates need shared indexes, probe paths, and executor state together"
)]
pub(super) async fn process_movie_refresh_candidate(
    app: &AppUseCase,
    _actor: &User,
    library_id: &str,
    candidate: PreparedMovieLibraryScanCandidate,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    root: &Path,
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedMovieLibraryScanCandidate>> {
    let representative_path = candidate.file.path.clone();
    let representative_is_directory = candidate.representative_is_directory;
    let discovered_files = candidate.discovered_files.clone();
    let pending_candidate = candidate.clone();

    match resolve_movie_scan_candidate(
        candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieCandidateResolution::Ready(title) => {
            let mut title = *title;
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                root,
                &representative_path,
                representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if movie_scan_folder_conflicts(
                canonical_folder_path.as_deref(),
                scan_folder_path.as_deref(),
            ) {
                persist_movie_title_folder_ownership_conflict(
                    app,
                    &MediaFacet::Movie,
                    library_id,
                    &path_to_stored_string(root),
                    None,
                    &title,
                    &pending_candidate,
                )
                .await?;
                summary.unmatched += 1;
                return Ok(None);
            }
            if let Some(index) = existing_titles
                .iter()
                .position(|existing| existing.id == title.id)
            {
                update_movie_probe_path_index(
                    existing_titles_by_probe_path,
                    root,
                    &representative_path,
                    index,
                );
            }
            let scope =
                movie_refresh_title_work_scope(scan_folder_path.as_deref(), discovered_files);
            let queued = merge_movie_refresh_title_work(
                executor,
                title,
                scope,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            }
            Ok(None)
        }
        MovieCandidateResolution::Skipped => Ok(None),
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved movie refresh candidates update indexes and background work in one place"
)]
pub(super) async fn process_resolved_movie_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    library_id: &str,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_smg_id: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    root: &Path,
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let representative_path = candidate.file.path.clone();
    let representative_is_directory = candidate.representative_is_directory;
    let discovered_files = candidate.discovered_files.clone();
    match resolve_movie_metadata_match(
        app,
        actor,
        &MediaFacet::Movie,
        library_id,
        &candidate,
        batch_search_results,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_smg_id,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieMetadataResolution::Ready(mut title) => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                root,
                &representative_path,
                representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if movie_scan_folder_conflicts(
                canonical_folder_path.as_deref(),
                scan_folder_path.as_deref(),
            ) {
                persist_movie_title_folder_ownership_conflict(
                    app,
                    &MediaFacet::Movie,
                    library_id,
                    &path_to_stored_string(root),
                    None,
                    &title,
                    &candidate,
                )
                .await?;
                summary.unmatched += 1;
                return Ok(());
            }
            if let Some(index) = existing_titles
                .iter()
                .position(|existing| existing.id == title.id)
            {
                update_movie_probe_path_index(
                    existing_titles_by_probe_path,
                    root,
                    &representative_path,
                    index,
                );
            }
            let scope =
                movie_refresh_title_work_scope(scan_folder_path.as_deref(), discovered_files);
            let queued = merge_movie_refresh_title_work(
                executor,
                title,
                scope,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            }
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { index, mut title } => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                root,
                &representative_path,
                representative_is_directory,
            )
            .await?;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            update_movie_probe_path_index(
                existing_titles_by_probe_path,
                root,
                &representative_path,
                index,
            );
            let scope =
                movie_refresh_title_work_scope(scan_folder_path.as_deref(), discovered_files);
            let queued = merge_movie_refresh_title_work(
                executor,
                title,
                scope,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                true,
            );
            if queued {
                summary.imported += 1;
                summary.matched += 1;
            }
            Ok(())
        }
        MovieMetadataResolution::CreateFailed(error) => {
            warn!(
                path = %representative_path,
                error = %error,
                "background movie refresh: failed to create title"
            );
            summary.unmatched += 1;
            Ok(())
        }
        MovieMetadataResolution::Unmatched => {
            summary.unmatched += 1;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::MediaFacet;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CountingLibraryScanner {
        metrics_calls: Arc<Mutex<Vec<String>>>,
        progress_calls: Arc<Mutex<Vec<String>>>,
    }

    impl CountingLibraryScanner {
        fn metrics_call_count(&self) -> usize {
            self.metrics_calls.lock().unwrap().len()
        }

        fn progress_call_count(&self) -> usize {
            self.progress_calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl LibraryScanner for CountingLibraryScanner {
        async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
            panic!("unused in test")
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

        async fn scan_directory_with_metrics(
            &self,
            root: &str,
        ) -> AppResult<LibraryDirectoryScanResult> {
            self.metrics_calls.lock().unwrap().push(root.to_string());
            Ok(LibraryDirectoryScanResult {
                files: vec![build_library_file(&format!("{root}/Episode.mkv"))],
                walk_ms: 1,
                stat_ms: 1,
                elapsed_ms: 2,
            })
        }

        async fn scan_directory_for_progress_with_metrics(
            &self,
            root: &str,
        ) -> AppResult<LibraryDirectoryScanResult> {
            self.progress_calls.lock().unwrap().push(root.to_string());
            Ok(LibraryDirectoryScanResult {
                files: vec![build_library_file(&format!("{root}/Episode.mkv"))],
                walk_ms: 1,
                stat_ms: 0,
                elapsed_ms: 1,
            })
        }
    }

    #[derive(Default)]
    struct CapturingTitleWorkQueue {
        queued: Vec<LibraryScanTitleWork>,
    }

    impl LibraryScanTitleWorkQueue for CapturingTitleWorkQueue {
        fn enqueue(&mut self, work: LibraryScanTitleWork) -> bool {
            self.queued.push(work);
            true
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

    fn build_movie_title(id: &str) -> Title {
        let mut title = build_series_title(id);
        title.name = "Test Movie".to_string();
        title.facet = MediaFacet::Movie;
        title.catalog_sort_key = "test movie".to_string();
        title
    }

    fn build_series_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            library_id: "library".to_string(),
            name: "Test Series".to_string(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: Vec::new(),
            root_folder_id: "root".to_string(),
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: "test series".to_string(),
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
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn series_candidate_with_nfo_ids(
        nfo_meta: crate::nfo::NfoMetadata,
    ) -> PreparedSeriesLibraryScanCandidate {
        PreparedSeriesLibraryScanCandidate {
            folder_path: std::path::PathBuf::from("/library/Show"),
            folder_name: Some("Show".to_string()),
            nfo_meta: Some(nfo_meta),
            identity_hint: None,
            query: String::new(),
            year_hint: None,
            search_candidates: Vec::new(),
            title_match_candidates: Vec::new(),
            metadata_lookup_attempted: true,
        }
    }

    #[test]
    fn find_existing_series_title_index_resolves_via_imdb_and_tmdb() {
        let mut imdb_title = build_series_title("series-imdb");
        imdb_title.external_ids = vec![scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: "tt2222222".to_string(),
        }];
        let mut tmdb_title = build_series_title("series-tmdb");
        tmdb_title.external_ids = vec![scryer_domain::ExternalId {
            source: "tmdb".to_string(),
            value: "55555".to_string(),
        }];
        let existing_titles = vec![imdb_title, tmdb_title];
        let (by_name, by_tvdb, by_imdb, by_tmdb) = build_series_title_indexes(&existing_titles);

        // A re-scanned series whose tvdb isn't locally indexed still resolves via
        // its NFO imdb/tmdb id, mirroring the movie scan (no SMG round-trip).
        let imdb_candidate = series_candidate_with_nfo_ids(crate::nfo::NfoMetadata {
            imdb_id: Some("tt2222222".to_string()),
            ..Default::default()
        });
        assert_eq!(
            find_existing_series_title_index(
                &imdb_candidate,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0)
        );

        let tmdb_candidate = series_candidate_with_nfo_ids(crate::nfo::NfoMetadata {
            tmdb_id: Some("55555".to_string()),
            ..Default::default()
        });
        assert_eq!(
            find_existing_series_title_index(
                &tmdb_candidate,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(1)
        );
    }

    fn build_movie_title_named(id: &str, name: &str, year: Option<i32>) -> Title {
        let mut title = build_movie_title(id);
        title.name = name.to_string();
        title.year = year;
        title
    }

    fn movie_candidate(query: &str, year_hint: Option<u32>) -> PreparedMovieLibraryScanCandidate {
        PreparedMovieLibraryScanCandidate {
            file: LibraryFile {
                path: format!("/library/{query}/{query}.mkv"),
                display_name: query.to_string(),
                nfo_path: None,
                size_bytes: None,
                source_signature_scheme: None,
                source_signature_value: None,
            },
            representative_is_directory: false,
            discovered_files: Vec::new(),
            parsed_release: crate::ParsedReleaseMetadata::default(),
            nfo_meta: None,
            identity_hint: None,
            query: query.to_string(),
            year_hint,
            query_variants: vec![query.to_string()],
            search_candidates: Vec::new(),
            metadata_lookup_attempted: true,
        }
    }

    fn series_candidate(query: &str, year_hint: Option<u32>) -> PreparedSeriesLibraryScanCandidate {
        PreparedSeriesLibraryScanCandidate {
            folder_path: std::path::PathBuf::from(format!("/library/{query}")),
            folder_name: Some(query.to_string()),
            nfo_meta: None,
            identity_hint: None,
            query: query.to_string(),
            year_hint,
            search_candidates: Vec::new(),
            title_match_candidates: crate::library_scan_metadata::build_title_match_candidates(&[
                query.to_string(),
            ]),
            metadata_lookup_attempted: true,
        }
    }

    #[test]
    fn same_name_titles_stay_distinct_by_year_in_the_scan_index() {
        // A remake and its original share a name (#148). Every by-name lookup
        // must see both and pick by year, and a folder year the catalog does
        // not have must resolve to nothing rather than to the other year's
        // title.
        let existing_titles = vec![
            build_movie_title_named("namesake-1990", "Namesake Film", Some(1990)),
            build_movie_title_named("namesake-2012", "Namesake Film", Some(2012)),
        ];
        let (by_name, _by_smg, by_tvdb, by_imdb, by_tmdb) =
            build_movie_title_indexes(&existing_titles);
        assert_eq!(
            by_name
                .get(&crate::title_matching::canonical_lookup_key(
                    "Namesake Film"
                ))
                .map(Vec::len),
            Some(2),
            "both same-name titles are indexed"
        );

        for (year, expected) in [(1990, Some(0)), (2012, Some(1)), (2024, None)] {
            assert_eq!(
                find_existing_movie_title_index(
                    &movie_candidate("Namesake Film", Some(year)),
                    &existing_titles,
                    &by_name,
                    &by_tvdb,
                    &by_imdb,
                    &by_tmdb,
                ),
                expected,
                "Namesake Film ({year})"
            );
        }
        // No year on the folder: any same-name title is acceptable (first wins).
        assert_eq!(
            find_existing_movie_title_index(
                &movie_candidate("Namesake Film", None),
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0)
        );
    }

    #[test]
    fn same_name_lookup_prefers_the_title_that_owns_the_scanned_folder() {
        // Both same-name titles accept a yearless folder; the one whose owned
        // folder already contains the scanned path must win, or a new file
        // dropped into a title's own folder binds to its sibling and is refused
        // as "already owns another folder". Year still rules the ownership
        // signal out when they disagree.
        let mut original = build_movie_title_named("namesake-1990", "Namesake Film", Some(1990));
        original.folder_path = Some("/library/Namesake Film".to_string());
        let mut remake = build_movie_title_named("namesake-2012", "Namesake Film", Some(2012));
        remake.folder_path = Some("/library/Namesake Film (2012)".to_string());
        // Ordered so the plain "first compatible" answer is the wrong one.
        let existing_titles = vec![remake, original];
        let (by_name, _by_smg, by_tvdb, by_imdb, by_tmdb) =
            build_movie_title_indexes(&existing_titles);

        let mut in_original_folder = movie_candidate("Namesake Film", None);
        in_original_folder.file.path = "/library/Namesake Film/Namesake.Film.1080p.mkv".to_string();
        assert_eq!(
            find_existing_movie_title_index(
                &in_original_folder,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(1),
            "the folder owner beats the first same-name title"
        );

        let mut in_neither_folder = movie_candidate("Namesake Film", None);
        in_neither_folder.file.path =
            "/library/Namesake Film - Extended/Namesake.Film.mkv".to_string();
        assert_eq!(
            find_existing_movie_title_index(
                &in_neither_folder,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0),
            "no owner among the candidates: first compatible"
        );

        let mut wrong_year_in_original_folder = movie_candidate("Namesake Film", Some(2012));
        wrong_year_in_original_folder.file.path =
            "/library/Namesake Film/Namesake.Film.2012.mkv".to_string();
        assert_eq!(
            find_existing_movie_title_index(
                &wrong_year_in_original_folder,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0),
            "ownership never overrides a year mismatch"
        );

        // Series: the candidate is the folder itself.
        let mut original_show = build_series_title("show-2004");
        original_show.name = "Namesake Show".to_string();
        original_show.year = Some(2004);
        original_show.folder_path = Some("/library/Namesake Show".to_string());
        let mut revival_show = build_series_title("show-2021");
        revival_show.name = "Namesake Show".to_string();
        revival_show.year = Some(2021);
        revival_show.folder_path = Some("/library/Namesake Show (2021)".to_string());
        let existing_series = vec![revival_show, original_show];
        let (by_name, by_tvdb, by_imdb, by_tmdb) = build_series_title_indexes(&existing_series);
        assert_eq!(
            find_existing_series_title_index(
                &series_candidate("Namesake Show", None),
                &existing_series,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(1),
            "the series that owns the scanned folder wins"
        );
        assert_eq!(
            find_existing_series_title_index(
                &series_candidate("Namesake Show", Some(2021)),
                &existing_series,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0),
            "year rules the folder owner out"
        );
    }

    #[test]
    fn metadata_match_maps_onto_an_existing_title_only_when_the_year_agrees() {
        // A "Remade Film (1994)" folder while the catalog holds only the 2019
        // remake: the metadata match for 1994 must NOT be absorbed by the 2019
        // title (which then refuses the folder as "already owns another
        // folder"); it is a new title. The same-year match and the canonical-id
        // match still map.
        let mut remake_2019 = build_movie_title_named("remade-2019", "Remade Film", Some(2019));
        remake_2019.external_ids = vec![scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: "tvdb-2019".to_string(),
        }];
        let existing_titles = vec![remake_2019];
        let (by_name, _by_smg, by_tvdb, _, _) = build_movie_title_indexes(&existing_titles);
        let selected = |tvdb_id: &str, year: Option<i32>| MetadataSearchItem {
            tvdb_id: tvdb_id.to_string(),
            smg_id: None,
            primary_source: None,
            external_ids: vec![],
            name: "Remade Film".to_string(),
            year,
            auto_match_safe: true,
            auto_match_signals: Vec::new(),
        };

        assert_eq!(
            find_existing_title_index_for_metadata_match(
                &selected("tvdb-1994", Some(1994)),
                &existing_titles,
                &by_name,
                &by_tvdb,
            ),
            None,
            "a different year is a different film"
        );
        assert_eq!(
            find_existing_title_index_for_metadata_match(
                &selected("tvdb-unknown", Some(2019)),
                &existing_titles,
                &by_name,
                &by_tvdb,
            ),
            Some(0),
            "same name, same year"
        );
        assert_eq!(
            find_existing_title_index_for_metadata_match(
                &selected("tvdb-2019", Some(1994)),
                &existing_titles,
                &by_name,
                &by_tvdb,
            ),
            Some(0),
            "the canonical id always wins"
        );
        assert_eq!(
            find_existing_title_index_for_metadata_match(
                &selected("tvdb-unknown", None),
                &existing_titles,
                &by_name,
                &by_tvdb,
            ),
            Some(0),
            "an unknown year cannot contradict"
        );
    }

    #[test]
    fn movie_metadata_match_prefers_smg_id_before_tvdb_id() {
        let mut smg_title = build_movie_title_named("smg-title", "SMG Title", Some(2020));
        smg_title.external_ids = vec![scryer_domain::ExternalId {
            source: "smg".to_string(),
            value: "901".to_string(),
        }];
        let mut tvdb_title = build_movie_title_named("tvdb-title", "TVDB Title", Some(2020));
        tvdb_title.external_ids = vec![scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: "movie-902".to_string(),
        }];
        let existing_titles = vec![smg_title, tvdb_title];
        let (by_name, by_smg, by_tvdb, by_imdb, by_tmdb) =
            build_movie_title_indexes(&existing_titles);
        let selected = MetadataSearchItem {
            tvdb_id: "movie-902".to_string(),
            smg_id: Some(901),
            primary_source: Some("tmdb".to_string()),
            external_ids: vec![],
            name: "Different Metadata Name".to_string(),
            year: Some(2020),
            auto_match_safe: true,
            auto_match_signals: Vec::new(),
        };

        assert_eq!(
            find_existing_movie_title_index_for_metadata_match(
                &selected,
                &existing_titles,
                &by_name,
                &by_smg,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0)
        );
    }

    #[test]
    fn deferred_episodic_title_work_requests_full_file_walk() {
        let work = deferred_episodic_title_work(
            build_series_title("title-1"),
            LibraryScanTitleWalkMode::Full,
            false,
        );

        assert!(work.requires_folder_enumeration());
    }

    #[test]
    fn movie_refresh_directory_without_pre_scanned_files_queues_full_folder_work() {
        let mut queue = CapturingTitleWorkQueue::default();

        assert!(merge_movie_refresh_title_work(
            &mut queue,
            build_movie_title("title-1"),
            movie_refresh_title_work_scope(Some("/library/Movie"), Vec::new()),
            LibraryScanTitleWalkMode::Additive,
            movie_cleanup_context(None, Some("/library/Movie".to_string())),
            false,
        ));

        assert_eq!(queue.queued.len(), 1);
        assert!(matches!(
            queue.queued[0].scope,
            LibraryScanTitleWorkScope::FullFolder
        ));
    }

    #[test]
    fn merge_library_scan_title_work_preserves_full_walk_requirement() {
        let title = build_series_title("title-1");
        let mut merged_work = HashMap::new();

        assert!(merge_library_scan_title_work(
            &mut merged_work,
            episodic_title_work(
                title.clone(),
                vec![build_library_file("/library/Show/loose.mkv")],
                LibraryScanTitleWalkMode::Full,
                false,
            ),
        ));
        assert_eq!(
            merged_work
                .get("title-1")
                .and_then(LibraryScanTitleWork::discovered_files)
                .map(Vec::len),
            Some(1),
        );

        assert!(merge_library_scan_title_work(
            &mut merged_work,
            deferred_episodic_title_work(title.clone(), LibraryScanTitleWalkMode::Full, false),
        ));
        assert!(
            merged_work
                .get("title-1")
                .expect("merged title work")
                .requires_folder_enumeration()
        );

        assert!(merge_library_scan_title_work(
            &mut merged_work,
            episodic_title_work(
                title,
                vec![build_library_file("/library/Show/another.mkv")],
                LibraryScanTitleWalkMode::Full,
                false,
            ),
        ));
        assert!(
            merged_work
                .get("title-1")
                .expect("merged title work")
                .requires_folder_enumeration()
        );
    }

    #[tokio::test]
    async fn scan_episodic_title_directory_for_progress_metrics_uses_progress_scan_path() {
        let scanner = CountingLibraryScanner::default();
        let folder_path = Path::new("/library/Show");

        let result = scan_episodic_title_directory_for_progress_metrics(
            Arc::new(scanner.clone()),
            folder_path,
        )
        .await
        .expect("scan episodic title directory");

        assert_eq!(scanner.progress_call_count(), 1);
        assert_eq!(scanner.metrics_call_count(), 0);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "/library/Show/Episode.mkv");
        assert!(result.files[0].source_signature_scheme.is_none());
        assert!(result.files[0].source_signature_value.is_none());
    }
}
