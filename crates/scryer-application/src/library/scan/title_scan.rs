use super::*;
use crate::domain_events::DomainEventActor;
use crate::library::movie_scan_scope::MovieScanScope;
use crate::library_filename_parser::{
    LibraryFilenameExistingRecord, LibraryFilenameFallbackPolicy, LibraryFilenameParseInput,
    LibraryFilenameParseMode, parse_library_filename,
};
use crate::library_scan_unmatched::{
    IgnoredLibraryScanItemArgs, LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE,
    build_title_bound_unmatched_scan_item, persist_ignored_library_scan_item,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

fn hydration_source_for_scan_mode(
    mode: LibraryScanMode,
) -> crate::catalog_workflow::HydrationSource {
    match mode {
        LibraryScanMode::Full => crate::catalog_workflow::HydrationSource::LibraryScanFull,
        LibraryScanMode::Additive => crate::catalog_workflow::HydrationSource::LibraryScanAdditive,
    }
}

pub(super) async fn title_requires_scan_hydration(
    app: &AppUseCase,
    title: &Title,
    metadata_language: &str,
) -> AppResult<bool> {
    let hydratable = match title.facet {
        MediaFacet::Movie => crate::catalog_workflow::movie_title_ref(title).is_some(),
        MediaFacet::Series | MediaFacet::Anime => {
            crate::catalog_workflow::extract_tvdb_id(title).is_some()
        }
    };
    if !hydratable {
        return Ok(false);
    }

    if title.metadata_fetched_at.is_none()
        || title.metadata_language.as_deref() != Some(metadata_language)
    {
        return Ok(true);
    }

    let Some(handler) = app.facet_registry.get(&title.facet) else {
        return Ok(false);
    };
    if !handler.has_episodes() {
        return Ok(false);
    }

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await?;
    Ok(episodes.is_empty())
}

async fn discover_episodic_title_files_for_progress(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<Vec<LibraryFile>> {
    let import_paths = crate::import_workflow::resolve_import_paths(app, title).await?;
    let title_dir = crate::effective_title_folder_path(
        &import_paths.media_root,
        title,
        &import_paths.folder_template,
        None,
    );
    if tokio::fs::metadata(&title_dir).await.is_err() {
        tokio::fs::create_dir_all(&title_dir).await.map_err(|err| {
            AppError::Repository(format!(
                "failed to recreate title directory {}: {err}",
                title_dir.display()
            ))
        })?;
    }
    let scan_result = scan_episodic_title_directory_for_progress_metrics(
        app.services.library.library_scanner.clone(),
        &title_dir,
    )
    .await?;
    Ok(scan_result.files)
}

async fn discover_movie_title_files(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<Vec<LibraryFile>> {
    let import_paths = crate::import_workflow::resolve_import_paths(app, title).await?;
    let media_root_path = PathBuf::from(&import_paths.media_root);
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .unwrap_or_default();
    let mut discovered_files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut candidate_paths = Vec::<PathBuf>::new();
    let mut seen_candidate_paths = HashSet::new();

    for collection in collections {
        let Some(ordered_path) = collection.ordered_path else {
            continue;
        };
        let ordered_path_buf = stored_path_to_path_buf(&ordered_path);
        if let Some(parent) = ordered_path_buf.parent()
            && parent != media_root_path.as_path()
            && seen_candidate_paths.insert(path_to_stored_string(parent))
        {
            candidate_paths.push(parent.to_path_buf());
        }
        if !seen_paths.insert(ordered_path.clone()) {
            continue;
        }

        match tokio::fs::metadata(&ordered_path_buf).await {
            Ok(metadata) if metadata.is_file() => {}
            Ok(metadata) if metadata.is_dir() => {
                if seen_candidate_paths.insert(ordered_path.clone()) {
                    candidate_paths.push(ordered_path_buf);
                }
                continue;
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %ordered_path,
                    "failed to inspect tracked movie path during title scan discovery"
                );
                continue;
            }
        }

        discovered_files.push(LibraryFile {
            path: ordered_path.clone(),
            display_name: ordered_path_buf
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: matching_movie_nfo_path(&ordered_path_buf),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });
    }

    if !discovered_files.is_empty() {
        return Ok(discovered_files);
    }

    let default_candidate_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(stored_path_to_path_buf)
        .unwrap_or_else(|| {
            crate::effective_title_folder_path(
                &import_paths.media_root,
                title,
                &import_paths.folder_template,
                None,
            )
        });
    if default_candidate_path != media_root_path
        && seen_candidate_paths.insert(path_to_stored_string(&default_candidate_path))
    {
        candidate_paths.push(default_candidate_path);
    }

    for candidate_path in candidate_paths {
        match tokio::fs::metadata(&candidate_path).await {
            Ok(metadata) if metadata.is_dir() => {
                let files = app
                    .services
                    .library
                    .library_scanner
                    .scan_library(path_to_stored_string(&candidate_path).as_str())
                    .await?;
                for file in files {
                    if seen_paths.insert(file.path.clone()) {
                        discovered_files.push(file);
                    }
                }
                if !discovered_files.is_empty() {
                    return Ok(discovered_files);
                }
            }
            Ok(metadata) if metadata.is_file() => {
                return Ok(vec![LibraryFile {
                    path: path_to_stored_string(&candidate_path),
                    display_name: candidate_path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    nfo_path: matching_movie_nfo_path(&candidate_path),
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                }]);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!(
                error = %error,
                title_id = %title.id,
                path = %candidate_path.display(),
                "failed to inspect movie scan candidate path"
            ),
        }
    }

    Ok(Vec::new())
}

async fn tracked_movie_path_confirmed_missing(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent() else {
                return false;
            };
            tokio::fs::metadata(parent)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        }
        Err(error) => {
            warn!(
                error = %error,
                path = %path.display(),
                "failed to inspect tracked movie path during stale cleanup"
            );
            false
        }
    }
}

fn title_external_id<'a>(title: &'a Title, source: &str) -> Option<&'a str> {
    if source == "imdb"
        && let Some(imdb_id) = title.imdb_id.as_deref()
        && !imdb_id.trim().is_empty()
    {
        return Some(imdb_id.trim());
    }

    title
        .external_ids
        .iter()
        .find(|external_id| {
            external_id.source.eq_ignore_ascii_case(source) && !external_id.value.trim().is_empty()
        })
        .map(|external_id| external_id.value.trim())
}

/// Rank a scanned movie file for the primary role, exactly the way admission
/// would: **tier first, then bar**.
///
/// Scan is not exempt from invariant I1, and it is not exempt from I3 either.
/// The file the scan promotes to primary is the file every later upgrade
/// decision measures against, so it has to be the one
/// [`crate::admission::AdmissionSubject::best_incumbent`] would name. Ranking on
/// the score alone was safe only while the quality tier was worth 3200/900/300
/// *inside* it; with the tier out of the number (D11) a 720p file at +300 sorts
/// above a 2160p one at +100, and the scan would elect the 720p file as the bar
/// — after which the 2160p file on disk reads as an upgrade candidate.
///
/// [`crate::AppUseCase::incumbent_bar`] is the same derivation the grab path
/// uses, so the pair this returns is the pair the gate will see for that file.
/// It also keeps the ranking number free of `BLOCK_SCORE`: a veto is a verdict,
/// never a −10 000 comparison term (I5).
fn rank_movie_media_file_for_primary(
    app: &AppUseCase,
    context: &crate::quality::canonical_context::ResolvedScoringContext,
    file: &TitleMediaFile,
) -> (usize, i32) {
    let bar = app.incumbent_bar(
        file,
        context,
        // A movie is one member: the title's own runtime, filled in by `view`.
        crate::quality_profile::CoverageSizeBasis::default(),
    );
    (crate::admission::tier_sort_key(bar.tier_index), bar.score)
}

/// One candidate for the primary role.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RankedMovieFile {
    /// Profile tier position, ascending; [`crate::admission::tier_sort_key`].
    tier_key: usize,
    /// The file's canonical bar, descending.
    score: i32,
    size_bytes: i64,
    file_path: String,
    file_id: String,
}

impl RankedMovieFile {
    /// Best-first ordering key: tier, then bar, then the long-standing
    /// size / path / id tie-breaks that keep the election deterministic when
    /// two files are genuinely equivalent.
    fn sort_key(
        &self,
    ) -> (
        usize,
        std::cmp::Reverse<i32>,
        std::cmp::Reverse<i64>,
        &str,
        &str,
    ) {
        (
            self.tier_key,
            std::cmp::Reverse(self.score),
            std::cmp::Reverse(self.size_bytes),
            self.file_path.as_str(),
            self.file_id.as_str(),
        )
    }
}

async fn normalize_movie_file_roles_after_scan(
    app: &AppUseCase,
    title: &Title,
    movie_scope: &MovieScanScope,
    newly_imported_file_count: usize,
    allow_existing_additional_role_promotion: bool,
) -> bool {
    let mut media_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files
            .into_iter()
            .filter(|file| movie_scope.file_is_in_scan_scope(&file.file_path))
            .collect::<Vec<_>>(),
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie media files for scan role normalization"
            );
            return false;
        }
    };
    if media_files.is_empty() {
        return false;
    }
    media_files.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.id.cmp(&right.id))
    });

    let primary_files = media_files
        .iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let should_rank_primary = newly_imported_file_count == media_files.len()
        || (primary_files.is_empty() && allow_existing_additional_role_promotion);
    if primary_files.is_empty() && !should_rank_primary {
        return false;
    }
    let selected_primary_id = if should_rank_primary {
        let category = crate::post_download_gate::facet_to_category_hint(&title.facet);
        let profile_lookup = crate::catalog::discovery::QualityProfileLookup {
            title_tags: &title.tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: title_external_id(title, "imdb"),
            tvdb_id: title_external_id(title, "tvdb"),
            category_hint: Some(category),
        };
        let profile = match app.resolve_quality_profile(profile_lookup).await {
            Ok(profile) => profile,
            Err(error) => {
                // Scan is a bulk pass: one title's dangling profile must not
                // leave its files unscanned, so role selection degrades to the
                // canonical built-in default instead of failing the title.
                warn!(
                    error = %error,
                    title_id = %title.id,
                    "failed to resolve quality profile for movie scan role selection; using the built-in default"
                );
                crate::builtin_default_quality_profile()
            }
        };
        let context = app.resolve_canonical_scoring_context(title, &profile).await;

        let mut ranked = Vec::with_capacity(media_files.len());
        for file in &media_files {
            let (tier_key, score) = rank_movie_media_file_for_primary(app, &context, file);
            ranked.push(RankedMovieFile {
                tier_key,
                score,
                size_bytes: file.size_bytes,
                file_path: file.file_path.clone(),
                file_id: file.id.clone(),
            });
        }
        ranked.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        ranked
            .first()
            .map(|ranked| ranked.file_id.clone())
            // `media_files` is non-empty above, so this is unreachable; a
            // rejection beats a panic on a bulk scan pass (D14).
            .unwrap_or_default()
    } else if let [file] = primary_files.as_slice() {
        file.id.clone()
    } else {
        let mut primary_files = primary_files;
        primary_files.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| left.id.cmp(&right.id))
        });
        primary_files[0].id.clone()
    };

    let additional_file_ids = media_files
        .iter()
        .filter(|file| file.id != selected_primary_id)
        .map(|file| file.id.clone())
        .collect::<Vec<_>>();
    let needs_update = media_files.iter().any(|file| {
        if file.id == selected_primary_id {
            !file.role.is_primary()
        } else {
            !file.role.is_additional()
        }
    });
    if !needs_update {
        return false;
    }

    match app
        .services
        .library
        .media_files
        .set_media_file_roles_for_title(&title.id, &selected_primary_id, &additional_file_ids)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                primary_file_id = %selected_primary_id,
                "failed to normalize movie media file roles after scan"
            );
            false
        }
    }
}

fn select_primary_episodic_media_file(
    files: &[&crate::EpisodeScopedMediaFile],
    episode_id: &str,
    allow_existing_additional_role_promotion: bool,
) -> Option<String> {
    let association_primary_files = files
        .iter()
        .copied()
        .filter(|file| {
            file.primary_episode_ids
                .iter()
                .any(|primary_episode_id| primary_episode_id == episode_id)
        })
        .collect::<Vec<_>>();
    if let [file] = association_primary_files.as_slice() {
        return Some(file.media_file.id.clone());
    }

    let mut ranked = if association_primary_files.is_empty() {
        let title_primary_files = files
            .iter()
            .copied()
            .filter(|file| file.title_role.is_primary())
            .collect::<Vec<_>>();
        if title_primary_files.is_empty() {
            if !allow_existing_additional_role_promotion {
                return None;
            }
            files.to_vec()
        } else {
            title_primary_files
        }
    } else {
        association_primary_files
    };
    ranked.sort_by(|left, right| {
        right
            .media_file
            .acquisition_score
            .unwrap_or(0)
            .cmp(&left.media_file.acquisition_score.unwrap_or(0))
            .then_with(|| right.media_file.size_bytes.cmp(&left.media_file.size_bytes))
            .then_with(|| left.media_file.file_path.cmp(&right.media_file.file_path))
            .then_with(|| left.media_file.id.cmp(&right.media_file.id))
    });
    Some(ranked[0].media_file.id.clone())
}

async fn normalize_episodic_file_roles_after_scan(
    app: &AppUseCase,
    title: &Title,
    episode_ids: &HashSet<String>,
    allow_existing_additional_role_promotion: bool,
) -> bool {
    if episode_ids.is_empty() {
        return false;
    }

    let mut episode_ids = episode_ids.iter().cloned().collect::<Vec<_>>();
    episode_ids.sort();

    let scoped_files = match app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(&title.id, &episode_ids)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list episodic media files for scan role normalization"
            );
            return false;
        }
    };
    if scoped_files.is_empty() {
        return false;
    }

    let mut title_updated = false;
    for episode_id in episode_ids {
        let candidates = scoped_files
            .iter()
            .filter(|file| file.episode_ids.iter().any(|id| id == &episode_id))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }

        let Some(selected_primary_id) = select_primary_episodic_media_file(
            &candidates,
            &episode_id,
            allow_existing_additional_role_promotion,
        ) else {
            continue;
        };
        let additional_file_ids = candidates
            .iter()
            .filter(|file| file.media_file.id != selected_primary_id)
            .map(|file| file.media_file.id.clone())
            .collect::<Vec<_>>();
        let needs_update = candidates.iter().any(|file| {
            let is_primary_for_episode = file
                .primary_episode_ids
                .iter()
                .any(|primary_episode_id| primary_episode_id == &episode_id);
            if file.media_file.id == selected_primary_id {
                !is_primary_for_episode
            } else {
                is_primary_for_episode
            }
        });
        if !needs_update {
            continue;
        }

        match app
            .services
            .library
            .media_files
            .set_media_file_roles_for_episode(
                &title.id,
                &episode_id,
                &selected_primary_id,
                &additional_file_ids,
            )
            .await
        {
            Ok(()) => title_updated = true,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    episode_id = %episode_id,
                    primary_file_id = %selected_primary_id,
                    "failed to normalize episodic media file roles after scan"
                );
            }
        }
    }

    title_updated
}

async fn cleanup_missing_movie_title_records(
    app: &AppUseCase,
    title: &Title,
    cleanup: &LibraryScanMovieCleanupContext,
    movie_scope: &MovieScanScope,
) -> bool {
    let mut title_updated = false;

    let media_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(media_files) => media_files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie media files during stale cleanup"
            );
            Vec::new()
        }
    };

    for media_file in media_files {
        if movie_scope.file_is_outside_canonical_folder(&media_file.file_path) {
            if let Err(error) = app
                .delete_media_file_record_with_dependents(&media_file.id)
                .await
            {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    media_file_id = %media_file.id,
                    file_path = %media_file.file_path,
                    "failed to detach out-of-folder movie media file after title scan"
                );
            } else {
                title_updated = true;
            }
            continue;
        }

        let file_path = stored_path_to_path_buf(&media_file.file_path);
        if !tracked_movie_path_confirmed_missing(file_path.as_path()).await {
            continue;
        }
        if let Err(error) = app
            .delete_media_file_record_with_dependents(&media_file.id)
            .await
        {
            warn!(
                error = %error,
                title_id = %title.id,
                media_file_id = %media_file.id,
                file_path = %media_file.file_path,
                "failed to delete stale movie media file after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    let collections = match app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
    {
        Ok(collections) => collections,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie collections during stale cleanup"
            );
            Vec::new()
        }
    };
    let cleanup_ids = cleanup
        .stale_collection_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    for collection in collections {
        let outside_canonical_folder = collection
            .ordered_path
            .as_deref()
            .is_some_and(|path| movie_scope.file_is_outside_canonical_folder(path));
        let missing_by_path = if let Some(path) = collection.ordered_path.as_deref() {
            tracked_movie_path_confirmed_missing(Path::new(path)).await
        } else {
            false
        };
        if !outside_canonical_folder && !missing_by_path && !cleanup_ids.contains(&collection.id) {
            continue;
        }

        if let Err(error) = app
            .services
            .catalog
            .shows
            .delete_collection(&collection.id)
            .await
        {
            warn!(
                error = %error,
                collection_id = %collection.id,
                title_id = %title.id,
                "failed to delete stale movie collection after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    title_updated
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LibraryScanFileTotalMode {
    /// One-off title scans: the walk itself publishes and latches the total.
    MarkKnownAfterThisWalk,
    /// Background refresh scans: the pool aggregates totals per accepted work and
    /// latches `file_total_known` once its input is closed and drained.
    BackgroundRefreshAggregate,
    /// Streaming full scans: the pool aggregates totals per accepted work,
    /// but the pipeline coordinator owns the `file_total_known` latch.
    FullScanAggregate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LibraryScanTitleWorkSummaryMode {
    FullScan,
    OneOff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LibraryScanMediaAnalysisProfile {
    pub title_group_concurrency: usize,
    pub file_analysis_concurrency_per_title: usize,
}

impl LibraryScanMediaAnalysisProfile {
    fn for_facet(facet: &MediaFacet) -> Self {
        match facet {
            MediaFacet::Movie => Self {
                title_group_concurrency: crate::LIBRARY_SCAN_MOVIE_TITLE_ANALYSIS_GROUP_CONCURRENCY,
                file_analysis_concurrency_per_title:
                    crate::LIBRARY_SCAN_MOVIE_FILE_ANALYSIS_CONCURRENCY_PER_WALK,
            },
            MediaFacet::Series | MediaFacet::Anime => Self {
                title_group_concurrency:
                    crate::LIBRARY_SCAN_EPISODIC_TITLE_ANALYSIS_GROUP_CONCURRENCY,
                file_analysis_concurrency_per_title:
                    crate::LIBRARY_SCAN_EPISODIC_FILE_ANALYSIS_CONCURRENCY_PER_WALK,
            },
        }
    }

    fn one_off() -> Self {
        Self {
            title_group_concurrency: 1,
            file_analysis_concurrency_per_title:
                crate::LIBRARY_SCAN_EPISODIC_FILE_ANALYSIS_CONCURRENCY_PER_WALK,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LibraryScanWorkCoverage {
    full_folder: bool,
    scoped_paths: HashSet<String>,
}

impl LibraryScanWorkCoverage {
    fn from_work(work: &LibraryScanTitleWork) -> Self {
        match &work.scope {
            LibraryScanTitleWorkScope::FullFolder => Self {
                full_folder: true,
                scoped_paths: HashSet::new(),
            },
            LibraryScanTitleWorkScope::PreEnumeratedFullFolder(files) => Self {
                full_folder: true,
                scoped_paths: files.iter().map(|file| file.path.clone()).collect(),
            },
            LibraryScanTitleWorkScope::ScopedFiles(files) => Self {
                full_folder: false,
                scoped_paths: files.iter().map(|file| file.path.clone()).collect(),
            },
        }
    }

    fn absorb(&mut self, other: Self) {
        self.full_folder |= other.full_folder;
        self.scoped_paths.extend(other.scoped_paths);
    }
}

fn analysis_ready_scoped_path_already_covered(
    ready: &std::collections::VecDeque<QueuedLibraryScanTitleAnalysisWork>,
    title_id: &str,
    path: &str,
) -> bool {
    ready
        .iter()
        .filter(|queued| queued.work.title.id == title_id)
        .filter_map(|queued| queued.work.discovered_files())
        .any(|files| files.iter().any(|file| file.path == path))
}

fn analysis_ready_full_folder_already_covered(
    ready: &std::collections::VecDeque<QueuedLibraryScanTitleAnalysisWork>,
    title_id: &str,
) -> bool {
    ready
        .iter()
        .any(|queued| queued.work.title.id == title_id && queued.coverage.full_folder)
}

struct LibraryScanTitleWalkTaskOutput {
    title_id: String,
    discovered_file_count: usize,
    coverage: LibraryScanWorkCoverage,
    absorb_walk_summary: bool,
    created_in_scan: bool,
    result: AppResult<LibraryTitleWalkResult>,
}

struct QueuedLibraryScanTitleAnalysisWork {
    work: LibraryScanTitleWork,
    coverage: LibraryScanWorkCoverage,
}

pub(super) struct LibraryScanMediaWorkReservation {
    pub(super) work: LibraryScanTitleWork,
    coverage: LibraryScanWorkCoverage,
    pub(super) file_count: usize,
}

impl LibraryScanMediaWorkReservation {
    pub(super) fn file_count(&self) -> usize {
        self.file_count
    }
}

/// Media-analysis worker pool: the walk/analyze stage of the scan pipeline.
///
/// Full scans feed it matched title work whose file list already rendezvoused
/// with the candidate inventory upstream; refresh and one-off scans may
/// enqueue deferred work, in which case the walk task discovers the folder
/// inline. Per-title hydration runs inside the walk task so it stays off the
/// candidate-to-match critical path (the streaming pipeline pre-hydrates in
/// bulk, making the per-title check a no-op there).
pub(super) struct LibraryScanMediaAnalysisPool {
    app: AppUseCase,
    actor: User,
    session_id: Option<String>,
    coordinator: Option<LibraryScanCoordinator>,
    cancel_token: Option<CancellationToken>,
    metadata_language: String,
    hydration_source: crate::catalog_workflow::HydrationSource,
    file_total_mode: LibraryScanFileTotalMode,
    summary_mode: LibraryScanTitleWorkSummaryMode,
    analysis_profile: LibraryScanMediaAnalysisProfile,
    input_closed: bool,
    reserved: HashMap<String, LibraryScanWorkCoverage>,
    pending_full: HashMap<String, LibraryScanTitleWork>,
    pending_scoped: HashMap<String, LibraryScanTitleWork>,
    analysis_ready: std::collections::VecDeque<QueuedLibraryScanTitleAnalysisWork>,
    in_flight: HashMap<String, LibraryScanWorkCoverage>,
    completed: HashMap<String, LibraryScanWorkCoverage>,
    work_set: tokio::task::JoinSet<LibraryScanTitleWalkTaskOutput>,
    file_total_known_marked: bool,
    summary: LibraryScanSummary,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LibraryScanMediaAnalysisPoolDiagnostics {
    pub input_closed: bool,
    pub reserved: usize,
    pub pending_full: usize,
    pub pending_scoped: usize,
    pub analysis_ready: usize,
    pub in_flight: usize,
    pub completed: usize,
    pub walk_tasks: usize,
    pub file_total_known_marked: bool,
}

pub(super) struct LibraryScanMediaAnalysisPolicy {
    session_id: Option<String>,
    coordinator: Option<LibraryScanCoordinator>,
    cancel_token: Option<CancellationToken>,
    hydration_source: crate::catalog_workflow::HydrationSource,
    file_total_mode: LibraryScanFileTotalMode,
    summary_mode: LibraryScanTitleWorkSummaryMode,
    analysis_profile: LibraryScanMediaAnalysisProfile,
}

impl LibraryScanMediaAnalysisPolicy {
    fn from_scan_session(
        app: &AppUseCase,
        session_id: &str,
        cancel_token: Option<CancellationToken>,
        hydration_source: crate::catalog_workflow::HydrationSource,
        file_total_mode: LibraryScanFileTotalMode,
        analysis_profile: LibraryScanMediaAnalysisProfile,
    ) -> Self {
        Self {
            session_id: Some(session_id.to_string()),
            coordinator: Some(LibraryScanCoordinator::new(
                app.clone(),
                session_id.to_string(),
            )),
            cancel_token,
            hydration_source,
            file_total_mode,
            summary_mode: LibraryScanTitleWorkSummaryMode::FullScan,
            analysis_profile,
        }
    }

    pub(super) async fn full_scan_pipeline(
        app: &AppUseCase,
        session_id: &str,
        facet: &MediaFacet,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        let session = app
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await;
        let hydration_source = session
            .map(|session| hydration_source_for_scan_mode(session.mode))
            .unwrap_or(crate::catalog_workflow::HydrationSource::LibraryScanFull);
        Self::from_scan_session(
            app,
            session_id,
            cancel_token,
            hydration_source,
            LibraryScanFileTotalMode::FullScanAggregate,
            LibraryScanMediaAnalysisProfile::for_facet(facet),
        )
    }

    pub(super) async fn background_refresh(
        app: &AppUseCase,
        session_id: &str,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        let session = app
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await;
        let analysis_profile = session
            .as_ref()
            .map(|session| LibraryScanMediaAnalysisProfile::for_facet(&session.facet))
            .unwrap_or_else(LibraryScanMediaAnalysisProfile::one_off);
        let hydration_source = session
            .map(|session| hydration_source_for_scan_mode(session.mode))
            .unwrap_or(crate::catalog_workflow::HydrationSource::LibraryScanFull);
        Self::from_scan_session(
            app,
            session_id,
            cancel_token,
            hydration_source,
            LibraryScanFileTotalMode::BackgroundRefreshAggregate,
            analysis_profile,
        )
    }

    fn one_off() -> Self {
        Self {
            session_id: None,
            coordinator: None,
            cancel_token: None,
            hydration_source: crate::catalog_workflow::HydrationSource::Interactive,
            file_total_mode: LibraryScanFileTotalMode::MarkKnownAfterThisWalk,
            summary_mode: LibraryScanTitleWorkSummaryMode::OneOff,
            analysis_profile: LibraryScanMediaAnalysisProfile::one_off(),
        }
    }

    pub(super) fn user_title_refresh() -> Self {
        Self::one_off()
    }

    pub(super) fn scoped_media_scan() -> Self {
        Self::one_off()
    }
}

struct LibraryScanTitleWalkOptions<'a> {
    session_id: Option<&'a str>,
    pre_scanned_files: Option<Vec<LibraryFile>>,
    mode: LibraryScanTitleWalkMode,
    cancel_token: Option<CancellationToken>,
    file_total_mode: LibraryScanFileTotalMode,
    full_folder_scan: bool,
    file_analysis_concurrency: usize,
}

impl LibraryScanMediaAnalysisPool {
    pub(super) async fn for_policy(
        app: &AppUseCase,
        actor: &User,
        policy: LibraryScanMediaAnalysisPolicy,
    ) -> AppResult<Self> {
        Ok(Self {
            app: app.clone(),
            actor: actor.clone(),
            session_id: policy.session_id,
            coordinator: policy.coordinator,
            cancel_token: policy.cancel_token,
            metadata_language: app.metadata_language().await,
            hydration_source: policy.hydration_source,
            file_total_mode: policy.file_total_mode,
            summary_mode: policy.summary_mode,
            analysis_profile: policy.analysis_profile,
            input_closed: false,
            reserved: HashMap::new(),
            pending_full: HashMap::new(),
            pending_scoped: HashMap::new(),
            analysis_ready: std::collections::VecDeque::new(),
            in_flight: HashMap::new(),
            completed: HashMap::new(),
            work_set: tokio::task::JoinSet::new(),
            file_total_known_marked: false,
            summary: LibraryScanSummary::default(),
        })
    }

    pub(super) fn analysis_profile(&self) -> LibraryScanMediaAnalysisProfile {
        self.analysis_profile
    }

    pub(super) fn diagnostics(&self) -> LibraryScanMediaAnalysisPoolDiagnostics {
        LibraryScanMediaAnalysisPoolDiagnostics {
            input_closed: self.input_closed,
            reserved: self.reserved.len(),
            pending_full: self.pending_full.len(),
            pending_scoped: self.pending_scoped.len(),
            analysis_ready: self.analysis_ready.len(),
            in_flight: self.in_flight.len(),
            completed: self.completed.len(),
            walk_tasks: self.work_set.len(),
            file_total_known_marked: self.file_total_known_marked,
        }
    }

    fn trim_uncovered_work(&self, work: &mut LibraryScanTitleWork) -> bool {
        let title_id = work.title.id.clone();
        if work.has_full_folder_coverage() && self.full_folder_already_covered(&title_id) {
            return false;
        }

        let has_full_folder_coverage = work.has_full_folder_coverage();
        if let Some(files) = work.discovered_files_mut() {
            files.retain(|file| !self.scoped_path_already_covered(&title_id, &file.path));
            if files.is_empty() && !has_full_folder_coverage {
                return false;
            }
        }

        true
    }

    fn enqueue_work(&mut self, mut work: LibraryScanTitleWork) -> bool {
        if self.input_closed {
            return false;
        }

        if !self.trim_uncovered_work(&mut work) {
            return false;
        }

        if work.requires_folder_enumeration() {
            merge_library_scan_title_work(&mut self.pending_full, work)
        } else {
            merge_library_scan_title_work(&mut self.pending_scoped, work)
        }
    }

    pub(super) fn reserve_work(
        &mut self,
        mut work: LibraryScanTitleWork,
    ) -> Option<LibraryScanMediaWorkReservation> {
        if self.input_closed {
            return None;
        }

        if !self.trim_uncovered_work(&mut work) {
            return None;
        }

        let title_id = work.title.id.clone();
        let coverage = LibraryScanWorkCoverage::from_work(&work);
        let file_count = work.discovered_file_count();
        self.reserved
            .entry(title_id)
            .or_default()
            .absorb(coverage.clone());
        Some(LibraryScanMediaWorkReservation {
            work,
            coverage,
            file_count,
        })
    }

    pub(super) fn commit_reserved(&mut self, reservation: LibraryScanMediaWorkReservation) {
        let LibraryScanMediaWorkReservation { work, coverage, .. } = reservation;
        self.analysis_ready
            .push_back(QueuedLibraryScanTitleAnalysisWork { work, coverage });
    }

    pub(super) async fn fail_reserved(
        &mut self,
        reservation: LibraryScanMediaWorkReservation,
        reason: &str,
    ) {
        warn!(
            title_id = %reservation.work.title.id,
            title_name = %reservation.work.title.name,
            files = reservation.file_count,
            reason = %reason,
            "library scan reserved media work failed before analysis"
        );
        if reservation.file_count > 0
            && let Some(coordinator) = self.coordinator.as_ref()
        {
            coordinator.mark_file_failed(reservation.file_count).await;
            coordinator.publish_progress().await;
        }
    }

    pub(super) async fn pump(&mut self) -> AppResult<()> {
        self.reap_completed_ready().await?;
        if library_scan_cancel_requested(self.cancel_token.as_ref()) {
            return Ok(());
        }
        self.promote_pending().await;
        self.launch_analysis().await?;
        self.mark_file_total_known_if_ready().await
    }

    pub(super) fn close_input(&mut self) {
        self.input_closed = true;
    }

    pub(super) async fn finish(&mut self) -> AppResult<LibraryScanSummary> {
        self.input_closed = true;
        loop {
            self.reap_completed_ready().await?;
            if library_scan_cancel_requested(self.cancel_token.as_ref()) {
                self.drain_cancelled_walks().await?;
                break;
            }

            self.promote_pending().await;
            self.launch_analysis().await?;
            self.mark_file_total_known_if_ready().await?;

            if self.pending_full.is_empty()
                && self.pending_scoped.is_empty()
                && self.analysis_ready.is_empty()
                && self.work_set.is_empty()
            {
                break;
            }

            if !self.work_set.is_empty() {
                self.reap_next_completed_or_cancelled().await?;
            } else {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        Ok(self.summary.clone())
    }

    /// Stop queued work, abort unfinished walks, and absorb any walk output
    /// that completed before the abort landed. Used when the scan fails
    /// terminally (for example root discovery failure) and no success progress
    /// latch may be published.
    pub(super) async fn drain_for_failure(&mut self) -> AppResult<()> {
        self.input_closed = true;
        self.drain_cancelled_walks().await
    }

    async fn drain_cancelled_walks(&mut self) -> AppResult<()> {
        self.pending_full.clear();
        self.pending_scoped.clear();
        self.reserved.clear();
        self.analysis_ready.clear();
        self.work_set.abort_all();
        while let Some(result) = self.work_set.join_next().await {
            match result {
                Ok(output) => self.handle_walk_task_result(Ok(output)).await?,
                Err(error) if error.is_cancelled() => {}
                Err(error) => return Err(AppError::Repository(error.to_string())),
            }
        }
        self.in_flight.clear();
        Ok(())
    }

    fn scoped_path_already_covered(&self, title_id: &str, path: &str) -> bool {
        self.completed
            .get(title_id)
            .is_some_and(|coverage| coverage.scoped_paths.contains(path))
            || self
                .in_flight
                .get(title_id)
                .is_some_and(|coverage| coverage.scoped_paths.contains(path))
            || analysis_ready_scoped_path_already_covered(&self.analysis_ready, title_id, path)
            || self
                .reserved
                .get(title_id)
                .is_some_and(|coverage| coverage.scoped_paths.contains(path))
            || self
                .pending_scoped
                .get(title_id)
                .and_then(LibraryScanTitleWork::discovered_files)
                .is_some_and(|files| files.iter().any(|file| file.path == path))
    }

    fn full_folder_already_covered(&self, title_id: &str) -> bool {
        self.completed
            .get(title_id)
            .is_some_and(|coverage| coverage.full_folder)
            || self
                .in_flight
                .get(title_id)
                .is_some_and(|coverage| coverage.full_folder)
            || analysis_ready_full_folder_already_covered(&self.analysis_ready, title_id)
            || self
                .reserved
                .get(title_id)
                .is_some_and(|coverage| coverage.full_folder)
            || self
                .pending_scoped
                .get(title_id)
                .is_some_and(LibraryScanTitleWork::has_full_folder_coverage)
            || self.pending_full.contains_key(title_id)
    }

    /// Move merged pending work into the analysis queue, publishing file
    /// totals for work that already knows its file list.
    async fn promote_pending(&mut self) {
        let mut works = Vec::new();
        for (_, work) in self.pending_full.drain() {
            works.push(work);
        }
        for (_, work) in self.pending_scoped.drain() {
            works.push(work);
        }
        works.sort_by(|left, right| left.title.id.cmp(&right.title.id));

        for work in works {
            let coverage = LibraryScanWorkCoverage::from_work(&work);
            let discovered_file_count = work.discovered_file_count();
            if discovered_file_count > 0
                && !self.file_total_known_marked
                && self.file_total_mode == LibraryScanFileTotalMode::BackgroundRefreshAggregate
                && let Some(coordinator) = self.coordinator.as_ref()
            {
                coordinator.add_file_total(discovered_file_count).await;
                coordinator.publish_progress().await;
            }
            self.analysis_ready
                .push_back(QueuedLibraryScanTitleAnalysisWork { work, coverage });
        }
    }

    fn pop_analysis_ready_work(&mut self) -> Option<QueuedLibraryScanTitleAnalysisWork> {
        let len = self.analysis_ready.len();
        for _ in 0..len {
            let queued = self.analysis_ready.pop_front()?;
            if self.in_flight.contains_key(&queued.work.title.id) {
                self.analysis_ready.push_back(queued);
            } else {
                return Some(queued);
            }
        }
        None
    }

    async fn launch_analysis(&mut self) -> AppResult<()> {
        while !library_scan_cancel_requested(self.cancel_token.as_ref()) {
            if self.in_flight.len() >= self.analysis_profile.title_group_concurrency {
                debug!(
                    analysis_ready = self.analysis_ready.len(),
                    in_flight = self.in_flight.len(),
                    walk_tasks = self.work_set.len(),
                    title_group_concurrency = self.analysis_profile.title_group_concurrency,
                    "library scan media analysis launch waiting for local profile cap"
                );
                break;
            }
            let Some(queued) = self.pop_analysis_ready_work() else {
                break;
            };
            self.spawn_walk(queued).await?;
        }
        Ok(())
    }

    async fn spawn_walk(&mut self, queued: QueuedLibraryScanTitleAnalysisWork) -> AppResult<()> {
        let QueuedLibraryScanTitleAnalysisWork { work, coverage } = queued;
        let title_id = work.title.id.clone();
        self.in_flight.insert(title_id.clone(), coverage.clone());
        let absorb_walk_summary = self.summary_mode == LibraryScanTitleWorkSummaryMode::OneOff
            || matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
        let full_folder_scan = coverage.full_folder;
        let created_in_scan = work.created_in_scan;
        debug!(
            in_flight = self.in_flight.len(),
            analysis_ready = self.analysis_ready.len(),
            walk_tasks = self.work_set.len(),
            full_folder_scan,
            scoped_files = coverage.scoped_paths.len(),
            "library scan media analysis walk launched"
        );
        let app = self.app.clone();
        let actor = self.actor.clone();
        let session_id = self.session_id.clone();
        let coordinator = self.coordinator.clone();
        let cancel_token = self.cancel_token.clone();
        let file_total_mode = self.file_total_mode;
        let metadata_language = self.metadata_language.clone();
        let hydration_source = self.hydration_source;
        let file_analysis_concurrency = self.analysis_profile.file_analysis_concurrency_per_title;
        self.work_set.spawn(async move {
            let (discovered_file_count, result) = hydrate_enumerate_and_walk_title_work(
                &app,
                &actor,
                work,
                LibraryScanTitleWalkTaskContext {
                    session_id,
                    coordinator,
                    cancel_token,
                    file_total_mode,
                    full_folder_scan,
                    metadata_language,
                    hydration_source,
                    file_analysis_concurrency,
                },
            )
            .await;
            LibraryScanTitleWalkTaskOutput {
                title_id,
                discovered_file_count,
                coverage,
                absorb_walk_summary,
                created_in_scan,
                result,
            }
        });
        Ok(())
    }

    async fn reap_completed_ready(&mut self) -> AppResult<()> {
        while let Some(result) = self.work_set.try_join_next() {
            debug!(
                in_flight = self.in_flight.len(),
                analysis_ready = self.analysis_ready.len(),
                walk_tasks = self.work_set.len(),
                "library scan media analysis walk completed"
            );
            self.handle_walk_task_result(result).await?;
        }
        Ok(())
    }

    async fn reap_next_completed_or_cancelled(&mut self) -> AppResult<()> {
        if self.work_set.is_empty() {
            return Ok(());
        }

        if let Some(cancel_token) = self.cancel_token.as_ref() {
            tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {}
                result = self.work_set.join_next() => {
                    if let Some(result) = result {
                        self.handle_walk_task_result(result).await?;
                    }
                }
            }
            return Ok(());
        }

        if let Some(result) = self.work_set.join_next().await {
            self.handle_walk_task_result(result).await?;
        }
        Ok(())
    }

    async fn mark_file_total_known_if_ready(&mut self) -> AppResult<()> {
        if self.file_total_mode != LibraryScanFileTotalMode::BackgroundRefreshAggregate
            || self.file_total_known_marked
            || !self.input_closed
            || library_scan_cancel_requested(self.cancel_token.as_ref())
            || !self.pending_full.is_empty()
            || !self.pending_scoped.is_empty()
            || !self.analysis_ready.is_empty()
            || !self.work_set.is_empty()
        {
            return Ok(());
        }

        if let Some(coordinator) = self.coordinator.as_ref() {
            coordinator.mark_file_total_known().await;
            coordinator.publish_progress().await;
        }
        self.file_total_known_marked = true;
        Ok(())
    }

    async fn handle_walk_task_result(
        &mut self,
        result: Result<LibraryScanTitleWalkTaskOutput, tokio::task::JoinError>,
    ) -> AppResult<()> {
        let output = result.map_err(|error| AppError::Repository(error.to_string()))?;
        self.in_flight.remove(&output.title_id);
        match output.result {
            Ok(walk_result) => {
                self.completed
                    .entry(output.title_id)
                    .or_default()
                    .absorb(output.coverage);
                if output.absorb_walk_summary {
                    let mut summary = walk_result.summary;
                    if output.created_in_scan {
                        summary.imported = summary.imported.saturating_sub(1);
                    }
                    self.summary.absorb(&summary);
                }
            }
            Err(error) => {
                if self.summary_mode == LibraryScanTitleWorkSummaryMode::OneOff {
                    return Err(error);
                }
                warn!(
                    error = %error,
                    title_id = %output.title_id,
                    "library scan title walk failed"
                );
                if output.discovered_file_count > 0
                    && let Some(coordinator) = self.coordinator.as_ref()
                {
                    coordinator
                        .mark_file_failed(output.discovered_file_count)
                        .await;
                    coordinator.publish_progress().await;
                }
            }
        }
        Ok(())
    }
}

impl LibraryScanTitleWorkQueue for LibraryScanMediaAnalysisPool {
    fn enqueue(&mut self, work: LibraryScanTitleWork) -> bool {
        self.enqueue_work(work)
    }
}

struct LibraryScanTitleWalkTaskContext {
    session_id: Option<String>,
    coordinator: Option<LibraryScanCoordinator>,
    cancel_token: Option<CancellationToken>,
    file_total_mode: LibraryScanFileTotalMode,
    full_folder_scan: bool,
    metadata_language: String,
    hydration_source: crate::catalog_workflow::HydrationSource,
    file_analysis_concurrency: usize,
}

/// One media-analysis task: per-title hydration guard, inline folder
/// discovery for deferred work, then the title walk. Returns the file count
/// known at the point of failure so failed work can be accounted against the
/// published file totals.
async fn hydrate_enumerate_and_walk_title_work(
    app: &AppUseCase,
    actor: &User,
    mut work: LibraryScanTitleWork,
    ctx: LibraryScanTitleWalkTaskContext,
) -> (usize, AppResult<LibraryTitleWalkResult>) {
    let mut known_file_count = work.discovered_file_count();

    match title_requires_scan_hydration(app, &work.title, &ctx.metadata_language).await {
        Ok(true) => {
            let target = crate::catalog_workflow::HydrationTarget {
                title: work.title.clone(),
                requested_tvdb_id: None,
                requested_movie_ref: None,
                sync_wanted_after_completion: false,
                source: ctx.hydration_source,
            };
            match app
                .hydrate_titles_bulk_cancellable(vec![target], ctx.cancel_token.as_ref())
                .await
            {
                Ok(outcome) => {
                    if let Some((_, reason)) = outcome
                        .failed_titles
                        .into_iter()
                        .find(|(title_id, _)| title_id == &work.title.id)
                    {
                        return (known_file_count, Err(AppError::Repository(reason)));
                    }
                    if let Some((_, hydrated)) = outcome
                        .hydrated_titles
                        .into_iter()
                        .find(|(title_id, _)| title_id == &work.title.id)
                    {
                        work.title = hydrated;
                    }
                }
                Err(error) => return (known_file_count, Err(error)),
            }
        }
        Ok(false) => {}
        Err(error) => return (known_file_count, Err(error)),
    }

    if work.requires_folder_enumeration() {
        match enumerate_library_scan_title_work(app, work, ctx.cancel_token.clone()).await {
            Ok(enumerated) => {
                work = enumerated;
                known_file_count = work.discovered_file_count();
                if known_file_count > 0
                    && ctx.file_total_mode == LibraryScanFileTotalMode::BackgroundRefreshAggregate
                    && let Some(coordinator) = ctx.coordinator.as_ref()
                {
                    coordinator.add_file_total(known_file_count).await;
                    coordinator.publish_progress().await;
                }
            }
            Err(error) => return (known_file_count, Err(error)),
        }
    }

    let result = app
        .walk_library_title(
            actor,
            LibraryScanTitleWalkRequest {
                work,
                session_id: ctx.session_id,
                cancel_token: ctx.cancel_token,
                file_total_mode: ctx.file_total_mode,
                full_folder_scan: ctx.full_folder_scan,
                file_analysis_concurrency: ctx.file_analysis_concurrency,
            },
        )
        .await;
    (known_file_count, result)
}

async fn enumerate_library_scan_title_work(
    app: &AppUseCase,
    mut work: LibraryScanTitleWork,
    cancel_token: Option<CancellationToken>,
) -> AppResult<LibraryScanTitleWork> {
    if !work.requires_folder_enumeration() {
        return Ok(work);
    }
    if library_scan_cancel_requested(cancel_token.as_ref()) {
        return Err(AppError::Canceled("library scan canceled".into()));
    }

    let files = match &work.facet_plan {
        LibraryScanTitleFacetPlan::Movie(_) => discover_movie_title_files(app, &work.title).await?,
        LibraryScanTitleFacetPlan::Episodic => {
            discover_episodic_title_files_for_progress(app, &work.title).await?
        }
    };
    work.scope = LibraryScanTitleWorkScope::PreEnumeratedFullFolder(files);
    Ok(work)
}

type TitleScanAnalysisTaskResult =
    AppResult<(PlannedTitleScanFile, MediaAnalysisOutcome, Duration)>;

fn launch_pending_title_scan_analysis_tasks(
    analysis_set: &mut tokio::task::JoinSet<TitleScanAnalysisTaskResult>,
    pending_analysis_plans: &mut std::collections::VecDeque<PlannedTitleScanFile>,
    analyzer: std::sync::Arc<dyn MediaAnalyzer>,
    analysis_limit: std::sync::Arc<tokio::sync::Semaphore>,
    file_analysis_concurrency: usize,
    cancel_token: Option<&CancellationToken>,
) {
    while !library_scan_cancel_requested(cancel_token)
        && analysis_set.len() < file_analysis_concurrency
    {
        let Some(plan) = pending_analysis_plans.pop_front() else {
            break;
        };
        let analyzer = analyzer.clone();
        let analysis_limit = analysis_limit.clone();
        let file_path = plan.file.path.clone();
        analysis_set.spawn(async move {
            tracing::debug!(file_path = %file_path, "title scan analysis task: start");
            let _permit = analysis_limit
                .acquire_owned()
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let analysis_started = Instant::now();
            let outcome = analyzer
                .analyze_file(stored_path_to_path_buf(&file_path))
                .await?;
            tracing::debug!(file_path = %file_path, "title scan analysis task: complete");
            Ok::<(PlannedTitleScanFile, MediaAnalysisOutcome, Duration), AppError>((
                plan,
                outcome,
                analysis_started.elapsed(),
            ))
        });
    }
}

struct TitleScanAnalysisFinalizeContext<'a> {
    app: &'a AppUseCase,
    title: &'a Title,
    scan_mode: &'a LibraryScanMode,
    episode_links: &'a mut HashSet<(String, String)>,
    summary: &'a mut LibraryScanSummary,
    db_elapsed: &'a mut Duration,
    external_subtitle_cache: &'a mut crate::subtitles::ExternalSubtitleDirectoryCache,
    pending_progress: &'a mut TitleScanProgressDelta,
    session_id: Option<&'a str>,
    title_updated_since_emit: &'a mut bool,
    cancel_token: Option<&'a CancellationToken>,
}

async fn finalize_title_scan_analysis_task_result(
    ctx: TitleScanAnalysisFinalizeContext<'_>,
    result: Result<TitleScanAnalysisTaskResult, tokio::task::JoinError>,
) -> AppResult<Duration> {
    let (plan, analysis_outcome, analysis_duration) =
        result.map_err(|error| AppError::Repository(error.to_string()))??;
    if library_scan_cancel_requested(ctx.cancel_token) {
        return Ok(analysis_duration);
    }

    let file_path = plan.file.path.clone();
    debug!(
        title_id = %ctx.title.id,
        title_name = %ctx.title.name,
        file_path = %file_path,
        "title scan stage: finalize file begin"
    );
    let outcome = finalize_title_scan_file(
        ctx.app,
        ctx.title,
        plan,
        Some(analysis_outcome),
        ctx.scan_mode.clone(),
        ctx.episode_links,
        ctx.summary,
        ctx.db_elapsed,
        ctx.external_subtitle_cache,
    )
    .await;
    if outcome.progress.failed == 0
        && let Err(error) = clear_library_scan_unmatched_item(
            ctx.app,
            &ctx.title.facet,
            &ctx.title.library_id,
            &file_path,
        )
        .await
    {
        warn!(
            error = %error,
            title_id = %ctx.title.id,
            file_path = %file_path,
            "failed to clear unmatched title scan item"
        );
    }
    ctx.pending_progress.absorb(outcome.progress);
    *ctx.title_updated_since_emit |= outcome.title_updated;
    flush_title_scan_progress_batch(ctx.app, ctx.session_id, ctx.pending_progress).await;
    debug!(
        title_id = %ctx.title.id,
        title_name = %ctx.title.name,
        file_path = %file_path,
        "title scan stage: finalize file complete"
    );

    Ok(analysis_duration)
}

fn title_scan_facet_plan(title: &Title) -> LibraryScanTitleFacetPlan {
    match title.facet {
        MediaFacet::Movie => {
            LibraryScanTitleFacetPlan::Movie(LibraryScanMovieCleanupContext::default())
        }
        MediaFacet::Series | MediaFacet::Anime => LibraryScanTitleFacetPlan::Episodic,
    }
}

impl AppUseCase {
    async fn execute_title_scan_work_with_policy(
        &self,
        actor: &User,
        work: LibraryScanTitleWork,
        policy: LibraryScanMediaAnalysisPolicy,
    ) -> AppResult<LibraryScanSummary> {
        let mut executor = LibraryScanMediaAnalysisPool::for_policy(self, actor, policy).await?;
        executor.enqueue(work);
        executor.close_input();
        executor.finish().await
    }

    async fn execute_user_title_refresh_work(
        &self,
        actor: &User,
        mut work: LibraryScanTitleWork,
    ) -> AppResult<LibraryScanSummary> {
        self.refresh_title_metadata_for_user_scan(&mut work.title)
            .await?;
        self.execute_title_scan_work_with_policy(
            actor,
            work,
            LibraryScanMediaAnalysisPolicy::user_title_refresh(),
        )
        .await
    }

    async fn refresh_title_metadata_for_user_scan(&self, title: &mut Title) -> AppResult<()> {
        let library_ids = vec![title.library_id.clone()];
        let forced_ids = self
            .services
            .catalog
            .titles
            .list_title_ids_with_metadata_hydration_due(Some(title.facet.clone()), &library_ids)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let mut titles = vec![title.clone()];
        super::scan_metadata_refresh::refresh_titles_metadata_for_scan_policy(
            self,
            &mut titles,
            &forced_ids,
            super::scan_metadata_refresh::LibraryScanMetadataRefreshMode::UserInitiatedTitleRefresh,
        )
        .await?;
        if let Some(refreshed) = titles.pop() {
            *title = refreshed;
        }
        Ok(())
    }

    async fn execute_scoped_media_scan_work(
        &self,
        actor: &User,
        work: LibraryScanTitleWork,
    ) -> AppResult<LibraryScanSummary> {
        self.execute_title_scan_work_with_policy(
            actor,
            work,
            LibraryScanMediaAnalysisPolicy::scoped_media_scan(),
        )
        .await
    }

    pub async fn scan_title_library(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_management_permission(actor, &title.library_id)
            .await?;

        let work = LibraryScanTitleWork {
            facet_plan: title_scan_facet_plan(&title),
            title,
            scope: LibraryScanTitleWorkScope::FullFolder,
            mode: LibraryScanTitleWalkMode::OneOff,
            created_in_scan: false,
        };

        self.execute_user_title_refresh_work(actor, work).await
    }

    pub(crate) async fn scan_title_library_with_discovered_files(
        &self,
        actor: &User,
        title: Title,
        discovered_files: Vec<LibraryFile>,
    ) -> AppResult<LibraryScanSummary> {
        if let Err(error) = self
            .require_library_management_permission(actor, &title.library_id)
            .await
        {
            match error {
                AppError::Unauthorized(_) => {
                    self.require_library_permission(
                        actor,
                        &title.library_id,
                        scryer_domain::LibraryPermission::ManageTitles,
                    )
                    .await?;
                }
                error => return Err(error),
            }
        }

        let work = LibraryScanTitleWork {
            facet_plan: title_scan_facet_plan(&title),
            title,
            scope: LibraryScanTitleWorkScope::ScopedFiles(discovered_files),
            mode: LibraryScanTitleWalkMode::OneOff,
            created_in_scan: false,
        };
        self.execute_scoped_media_scan_work(actor, work).await
    }

    pub(crate) async fn walk_library_title(
        &self,
        actor: &User,
        request: LibraryScanTitleWalkRequest,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkRequest {
            work,
            session_id,
            cancel_token,
            file_total_mode,
            full_folder_scan,
            file_analysis_concurrency,
        } = request;
        let session_id = session_id.as_deref();
        let LibraryScanTitleWork {
            title,
            facet_plan,
            scope,
            mode,
            created_in_scan: _,
        } = work;
        let pre_scanned_files = scope.into_discovered_files();
        match facet_plan {
            LibraryScanTitleFacetPlan::Movie(cleanup) => {
                let options = LibraryScanTitleWalkOptions {
                    session_id,
                    pre_scanned_files,
                    mode,
                    cancel_token,
                    file_total_mode,
                    full_folder_scan,
                    file_analysis_concurrency,
                };
                self.walk_movie_library_title(title, cleanup, options).await
            }
            LibraryScanTitleFacetPlan::Episodic => {
                let options = LibraryScanTitleWalkOptions {
                    session_id,
                    pre_scanned_files,
                    mode,
                    cancel_token,
                    file_total_mode,
                    full_folder_scan,
                    file_analysis_concurrency,
                };
                self.walk_episodic_library_title(actor, title, options)
                    .await
            }
        }
    }

    async fn walk_movie_library_title(
        &self,
        title: Title,
        cleanup: LibraryScanMovieCleanupContext,
        options: LibraryScanTitleWalkOptions<'_>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkOptions {
            session_id,
            pre_scanned_files,
            mode,
            cancel_token,
            file_total_mode,
            full_folder_scan: _,
            file_analysis_concurrency: _,
        } = options;
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let mut summary = LibraryScanSummary::default();
        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                let files = discover_movie_title_files(self, &title).await?;
                if file_total_mode == LibraryScanFileTotalMode::MarkKnownAfterThisWalk
                    && let Some(coordinator) = session_coordinator.as_ref()
                {
                    coordinator.add_file_total(files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                files
            }
        };
        let discovered_file_count = discovered_files.len();
        let movie_scope = MovieScanScope::from_scan_inputs(
            cleanup.canonical_folder_path.as_deref(),
            title.folder_path.as_deref(),
            cleanup.scan_folder_path.as_deref(),
            &discovered_files,
        );

        debug!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            pre_scanned_file_count = discovered_file_count,
            "movie title scan stage: start"
        );

        let title_scan_root = cleanup
            .scan_folder_path
            .as_deref()
            .or(title.folder_path.as_deref())
            .unwrap_or_default();

        for file in &discovered_files {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            finalize_movie_scan_file(
                self,
                &title,
                file,
                &mut summary,
                session_id,
                title_scan_root,
                cancel_token.as_ref(),
            )
            .await;
            if let Some(coordinator) = session_coordinator.as_ref() {
                coordinator.mark_file_completed(1).await;
            }
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
        }

        if !library_scan_cancel_requested(cancel_token.as_ref()) {
            let cleanup_updated =
                cleanup_missing_movie_title_records(self, &title, &cleanup, &movie_scope).await;
            let roles_updated = normalize_movie_file_roles_after_scan(
                self,
                &title,
                &movie_scope,
                summary.imported,
                mode.allows_existing_additional_role_promotion(),
            )
            .await;
            if cleanup_updated || roles_updated {
                self.emit_title_updated_activity(None, &title).await;
            }
        }

        debug!(
            title_id = %title.id,
            title_name = %title.name,
            files = discovered_file_count,
            imported = summary.imported,
            skipped = summary.skipped,
            elapsed_ms = elapsed_ms_u64(started_at),
            "movie title scan completed"
        );
        if let Some(coordinator) = session_coordinator.as_ref() {
            coordinator.publish_progress().await;
        }

        Ok(LibraryTitleWalkResult { summary })
    }

    async fn walk_episodic_library_title(
        &self,
        actor: &User,
        title: Title,
        options: LibraryScanTitleWalkOptions<'_>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkOptions {
            session_id,
            pre_scanned_files,
            mode,
            cancel_token,
            file_total_mode,
            full_folder_scan,
            file_analysis_concurrency,
        } = options;
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let scoped_discovered_files = pre_scanned_files.is_some() && !full_folder_scan;
        let pre_scanned_file_count = pre_scanned_files.as_ref().map(Vec::len);
        let scan_mode = mode.as_file_finalize_mode();

        let handler = self.facet_registry.get(&title.facet).ok_or_else(|| {
            AppError::Validation("library scan is not supported for this facet".into())
        })?;
        if !handler.has_episodes() {
            return Err(AppError::Validation(
                "title library scan is only supported for episodic titles".into(),
            ));
        }

        let import_paths = crate::import_workflow::resolve_import_paths(self, &title).await?;
        let title_dir = crate::effective_title_folder_path(
            &import_paths.media_root,
            &title,
            &import_paths.folder_template,
            None,
        );
        let title_dir_str = path_to_stored_string(&title_dir);
        debug!(
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

        if !scoped_discovered_files && tokio::fs::metadata(&title_dir).await.is_err() {
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
                let scan_result = scan_episodic_title_directory_for_progress_metrics(
                    self.services.library.library_scanner.clone(),
                    &title_dir,
                )
                .await?;
                walk_elapsed =
                    walk_elapsed.saturating_add(Duration::from_millis(scan_result.walk_ms));
                stat_elapsed =
                    stat_elapsed.saturating_add(Duration::from_millis(scan_result.stat_ms));
                if file_total_mode == LibraryScanFileTotalMode::MarkKnownAfterThisWalk
                    && let Some(coordinator) = session_coordinator.as_ref()
                {
                    coordinator.add_file_total(scan_result.files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                scan_result.files
            }
        };
        let db_started = Instant::now();
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let series_movie_links = self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_title(&title.id)
            .await
            .unwrap_or_default();
        let title_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            discovered_files = discovered_files.len(),
            existing_files = existing_files.len(),
            collections = collections.len(),
            series_movie_links = series_movie_links.len(),
            title_episodes = title_episodes.len(),
            "title scan stage: db state loaded"
        );
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            "title scan stage: episode context loaded"
        );

        let mut existing_records_by_path: HashMap<String, TitleMediaFile> = HashMap::new();
        let mut episode_links: HashSet<(String, String)> = HashSet::new();
        let mut role_normalization_episode_ids = HashSet::new();

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
        let mut seen_paths = HashSet::new();
        let analysis_limit = self.runtime.library.library_scan_analysis_limit.clone();
        let mut pending_progress = TitleScanProgressDelta::default();
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;
        let mut external_subtitle_cache =
            crate::subtitles::ExternalSubtitleDirectoryCache::default();
        let actor_event = DomainEventActor::from(actor);
        let analyzer = self.services.library.media_analyzer.clone();
        let mut analysis_set = tokio::task::JoinSet::new();
        let mut pending_analysis_plans = std::collections::VecDeque::new();
        let mut title_updated_since_emit = false;

        'file_chunks: for file_chunk in discovered_files.chunks(TITLE_SCAN_FILE_BATCH_SIZE) {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }

            for file in file_chunk.iter().cloned() {
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    break;
                }
                if !file.path.trim().is_empty() {
                    seen_paths.insert(file.path.clone());
                }
                remaining_existing_paths.remove(&file.path);
                summary.scanned += 1;

                let source_path = stored_path_to_path_buf(&file.path);
                let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(&file)
                {
                    snapshot
                } else {
                    let stat_started = Instant::now();
                    match file_source_snapshot_from_path(&source_path).await {
                        Ok(snapshot) => {
                            stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());
                            snapshot
                        }
                        Err(error) => {
                            stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_path = %file.path,
                                "failed to read file source signature during title scan"
                            );
                            let display_name = file.display_name.trim().to_string();
                            let display_name = if display_name.is_empty() {
                                source_path
                                    .file_name()
                                    .map(|value| value.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| file.path.clone())
                            } else {
                                display_name
                            };
                            if let Err(persist_error) = persist_ignored_library_scan_item(
                                self,
                                &title.facet,
                                &title.library_id,
                                IgnoredLibraryScanItemArgs {
                                    title_id: Some(&title.id),
                                    session_id,
                                    library_path: &title_dir_str,
                                    item_path: &file.path,
                                    display_name: &display_name,
                                    query: &display_name,
                                    year_hint: title.year.and_then(|year| u32::try_from(year).ok()),
                                    reason_code: LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE,
                                    error_message: Some(error.to_string()),
                                    size_bytes: file.size_bytes,
                                },
                            )
                            .await
                            {
                                warn!(
                                    path = %file.path,
                                    error = %persist_error,
                                    "failed to persist ignored library scan file"
                                );
                            }
                            summary.skipped += 1;
                            pending_progress.absorb(TitleScanProgressDelta::completed(1));
                            flush_title_scan_progress_batch(
                                self,
                                session_id,
                                &mut pending_progress,
                            )
                            .await;
                            continue;
                        }
                    }
                };

                let existing = existing_records_by_path.get(&file.path);
                let existing_snapshot_matches = existing
                    .is_some_and(|existing| title_media_file_matches_snapshot(existing, &snapshot));

                let filename_parse = parse_library_filename(&LibraryFilenameParseInput {
                    path: &source_path,
                    display_name: Some(file.display_name.as_str()),
                    library_root: None,
                    title: Some(&title),
                    facet: Some(&title.facet),
                    collections: &collections,
                    series_movie_links: &series_movie_links,
                    episodes: &title_episodes,
                    existing_record: existing.map(|existing| LibraryFilenameExistingRecord {
                        episode_id: existing.episode_id.as_deref(),
                        snapshot_matches: existing_snapshot_matches,
                    }),
                    mode: LibraryFilenameParseMode::TitleScan,
                    fallback_policy: if existing.is_none() {
                        LibraryFilenameFallbackPolicy::NeedReleaseMetadata
                    } else {
                        LibraryFilenameFallbackPolicy::WhenNeeded
                    },
                });
                let target_episodes = filename_parse.target_episodes();
                let series_movie_link_id = filename_parse
                    .target_series_movie_link_id()
                    .map(str::to_string);

                if target_episodes.is_empty() && series_movie_link_id.is_none() {
                    let reason = filename_parse.unmatched_reason().unwrap_or_else(|| {
                        if filename_parse.episode_identity.is_some() {
                            "episode_lookup_failed"
                        } else {
                            "episode_identity_missing"
                        }
                    });
                    debug!(
                        title_id = %title.id,
                        title_name = %title.name,
                        file_path = %file.path,
                        display_name = %file.display_name,
                        title_dir = %title_dir_str,
                        discovered_files = discovered_files.len(),
                        parsed_episode = ?filename_parse.episode_identity,
                        strategy = ?filename_parse.strategy,
                        release_fallback_used = filename_parse.release_fallback_used,
                        reason,
                        "title scan: episode target missing"
                    );
                    let unmatched_item = build_title_bound_unmatched_scan_item(
                        &title.facet,
                        &title.library_id,
                        &title.id,
                        session_id,
                        &title_dir_str,
                        &file.path,
                        &file.display_name,
                        &title.name,
                        title.year.map(|value| value as u32),
                        reason,
                        file.size_bytes,
                    );
                    if let Err(error) =
                        persist_library_scan_unmatched_item(self, &unmatched_item).await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file.path,
                            "failed to persist unmatched title scan item"
                        );
                    }
                    summary.unmatched += 1;
                    pending_progress.absorb(TitleScanProgressDelta::completed(1));
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                summary.matched += 1;
                for episode in &target_episodes {
                    role_normalization_episode_ids.insert(episode.id.clone());
                }
                let configured_folder_name =
                    crate::library::workflow::scan_title_files::infer_target_season_number(
                        &target_episodes,
                    )
                    .map(|season| {
                        crate::render_episode_folder_name(
                            &title,
                            season,
                            &import_paths.season_folder_template,
                            &import_paths.specials_folder_template,
                        )
                    });
                let layout_observation = classify_title_scan_layout(
                    &title_dir,
                    &source_path,
                    &target_episodes,
                    configured_folder_name.as_deref(),
                );
                layout_summary.observe(layout_observation);

                let record = if let Some(existing) = existing {
                    let desired_scheme = snapshot
                        .signature
                        .as_ref()
                        .map(|value| value.scheme.clone());
                    let desired_value =
                        snapshot.signature.as_ref().map(|value| value.value.clone());
                    PlannedTitleScanRecord::Existing {
                        file_id: existing.id.clone(),
                        should_skip_analysis: existing_snapshot_matches,
                        should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                            || existing.source_signature_scheme != desired_scheme
                            || existing.source_signature_value != desired_value
                            || existing.scan_status != "scanned",
                    }
                } else {
                    PlannedTitleScanRecord::New
                };

                let plan = PlannedTitleScanFile {
                    file,
                    parsed: filename_parse.parsed_release,
                    target_episodes,
                    series_movie_link_id,
                    snapshot,
                    record,
                };

                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
                    let file_path = plan.file.path.clone();
                    let outcome = finalize_title_scan_file(
                        self,
                        &title,
                        plan,
                        None,
                        scan_mode.clone(),
                        &mut episode_links,
                        &mut summary,
                        &mut db_elapsed,
                        &mut external_subtitle_cache,
                    )
                    .await;
                    if outcome.progress.failed == 0
                        && let Err(error) = clear_library_scan_unmatched_item(
                            self,
                            &title.facet,
                            &title.library_id,
                            &file_path,
                        )
                        .await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file_path,
                            "failed to clear unmatched title scan item"
                        );
                    }
                    pending_progress.absorb(outcome.progress);
                    title_updated_since_emit |= outcome.title_updated;
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                } else {
                    analyzed_files += 1;
                    pending_analysis_plans.push_back(plan);
                }

                launch_pending_title_scan_analysis_tasks(
                    &mut analysis_set,
                    &mut pending_analysis_plans,
                    analyzer.clone(),
                    analysis_limit.clone(),
                    file_analysis_concurrency,
                    cancel_token.as_ref(),
                );
                while let Some(result) = analysis_set.try_join_next() {
                    let analysis_duration = finalize_title_scan_analysis_task_result(
                        TitleScanAnalysisFinalizeContext {
                            app: self,
                            title: &title,
                            scan_mode: &scan_mode,
                            episode_links: &mut episode_links,
                            summary: &mut summary,
                            db_elapsed: &mut db_elapsed,
                            external_subtitle_cache: &mut external_subtitle_cache,
                            pending_progress: &mut pending_progress,
                            session_id,
                            title_updated_since_emit: &mut title_updated_since_emit,
                            cancel_token: cancel_token.as_ref(),
                        },
                        result,
                    )
                    .await?;
                    analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);
                }
            }

            if title_updated_since_emit {
                self.emit_title_updated_activity(actor_event.clone(), &title)
                    .await;
                title_updated_since_emit = false;
            }

            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break 'file_chunks;
            }
        }

        while !pending_analysis_plans.is_empty() || !analysis_set.is_empty() {
            launch_pending_title_scan_analysis_tasks(
                &mut analysis_set,
                &mut pending_analysis_plans,
                analyzer.clone(),
                analysis_limit.clone(),
                file_analysis_concurrency,
                cancel_token.as_ref(),
            );

            if library_scan_cancel_requested(cancel_token.as_ref()) {
                pending_analysis_plans.clear();
                analysis_set.abort_all();
                break;
            }

            let Some(result) = await_cancellable(cancel_token.as_ref(), analysis_set.join_next())
                .await
                .flatten()
            else {
                pending_analysis_plans.clear();
                analysis_set.abort_all();
                break;
            };
            let analysis_duration = finalize_title_scan_analysis_task_result(
                TitleScanAnalysisFinalizeContext {
                    app: self,
                    title: &title,
                    scan_mode: &scan_mode,
                    episode_links: &mut episode_links,
                    summary: &mut summary,
                    db_elapsed: &mut db_elapsed,
                    external_subtitle_cache: &mut external_subtitle_cache,
                    pending_progress: &mut pending_progress,
                    session_id,
                    title_updated_since_emit: &mut title_updated_since_emit,
                    cancel_token: cancel_token.as_ref(),
                },
                result,
            )
            .await?;
            analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);

            if title_updated_since_emit {
                self.emit_title_updated_activity(actor_event.clone(), &title)
                    .await;
                title_updated_since_emit = false;
            }
        }

        flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;

        if !library_scan_cancel_requested(cancel_token.as_ref()) {
            let mut title_updated_after_scan = false;

            if !scoped_discovered_files {
                reconcile_library_scan_unmatched_items(
                    self,
                    &title.facet,
                    &title_dir_str,
                    &seen_paths,
                )
                .await?;
                for stale_path in remaining_existing_paths {
                    let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                        continue;
                    };
                    if !stale_path.starts_with(title_dir_str.as_str()) {
                        continue;
                    }
                    if stored_path_to_path_buf(&record.file_path).exists() {
                        continue;
                    }
                    let db_started = Instant::now();
                    let delete_result = self
                        .delete_media_file_record_with_dependents(&record.id)
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
                        .catalog
                        .titles
                        .set_folder_path(&title.id, &title_dir_str)
                        .await?;
                    db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    title_updated_after_scan = true;
                }

                if matches!(title.facet, MediaFacet::Series | MediaFacet::Anime)
                    && let Some(use_season_folders) = layout_summary.inferred_use_season_folders()
                    && crate::import_workflow::season_folder_tag_override(&title).is_none()
                    // A scan must not turn a layout observed under a previous
                    // policy into a title override that defeats an explicit
                    // library setting.
                    && self
                        .library_use_season_folders_override(&title.library_id)
                        .await?
                        .is_none()
                    && self.resolve_use_season_folders(&title).await? != use_season_folders
                {
                    let tags = merge_title_scan_option_tags(title.tags.clone(), use_season_folders);
                    let db_started = Instant::now();
                    self.apply_title_metadata_update(actor, &title.id, None, None, Some(tags))
                        .await?;
                    db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                    title_updated_after_scan = true;
                }
            }

            if normalize_episodic_file_roles_after_scan(
                self,
                &title,
                &role_normalization_episode_ids,
                mode.allows_existing_additional_role_promotion(),
            )
            .await
            {
                title_updated_after_scan = true;
            }

            if title_updated_after_scan {
                self.emit_title_updated_activity(actor_event.clone(), &title)
                    .await;
            }
        }

        debug!(
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
            worker_concurrency = file_analysis_concurrency,
            elapsed_ms = elapsed_ms_u64(started_at),
            "title library scan completed"
        );

        Ok(LibraryTitleWalkResult { summary })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryScanTitleWalkRequest {
    pub(crate) work: LibraryScanTitleWork,
    pub(crate) session_id: Option<String>,
    pub(crate) cancel_token: Option<CancellationToken>,
    pub(crate) file_total_mode: LibraryScanFileTotalMode,
    pub(crate) full_folder_scan: bool,
    pub(crate) file_analysis_concurrency: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{Episode, ExternalId, MediaFacet, Title};
    use std::path::Path;

    fn numeric_series_title() -> Title {
        Title {
            id: "title-13".into(),
            name: "13".into(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".into(),
                value: "131313".into(),
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

    fn numeric_series_episode(season: &str, episode: &str) -> Episode {
        Episode {
            id: format!("episode-{season}-{episode}"),
            title_id: "title-13".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(episode.into()),
            season_number: Some(season.into()),
            episode_label: Some(format!("S{season:0>2}E{episode:0>2}")),
            title: Some(format!("Day {season} 800 A.M. 900 A.M.")),
            air_date: None,
            duration_seconds: None,
            image_url: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn ready_queue_test_file(path: &str) -> LibraryFile {
        LibraryFile {
            path: path.into(),
            display_name: path.into(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }
    }

    fn ready_queue_test_work(
        title_id: &str,
        discovered_files: Option<Vec<LibraryFile>>,
    ) -> QueuedLibraryScanTitleAnalysisWork {
        let mut title = numeric_series_title();
        title.id = title_id.into();
        let scope = match discovered_files {
            Some(files) => LibraryScanTitleWorkScope::ScopedFiles(files),
            None => LibraryScanTitleWorkScope::FullFolder,
        };
        let work = LibraryScanTitleWork {
            title,
            facet_plan: LibraryScanTitleFacetPlan::Episodic,
            scope,
            mode: LibraryScanTitleWalkMode::Full,
            created_in_scan: false,
        };
        let coverage = LibraryScanWorkCoverage::from_work(&work);
        QueuedLibraryScanTitleAnalysisWork { work, coverage }
    }

    #[test]
    fn analysis_ready_full_folder_counts_as_existing_full_coverage() {
        let mut ready = std::collections::VecDeque::new();
        ready.push_back(ready_queue_test_work("title-ready", None));
        ready.push_back(ready_queue_test_work(
            "title-other",
            Some(vec![ready_queue_test_file("/library/other.mkv")]),
        ));

        assert!(analysis_ready_full_folder_already_covered(
            &ready,
            "title-ready"
        ));
        assert!(!analysis_ready_full_folder_already_covered(
            &ready,
            "title-other"
        ));
        assert!(!analysis_ready_full_folder_already_covered(
            &ready,
            "title-missing"
        ));
    }

    #[test]
    fn analysis_ready_scoped_file_counts_as_existing_scoped_coverage() {
        let mut ready = std::collections::VecDeque::new();
        ready.push_back(ready_queue_test_work(
            "title-ready",
            Some(vec![
                ready_queue_test_file("/library/show/s01e01.mkv"),
                ready_queue_test_file("/library/show/s01e02.mkv"),
            ]),
        ));

        assert!(analysis_ready_scoped_path_already_covered(
            &ready,
            "title-ready",
            "/library/show/s01e01.mkv"
        ));
        assert!(!analysis_ready_scoped_path_already_covered(
            &ready,
            "title-ready",
            "/library/show/s01e03.mkv"
        ));
        assert!(!analysis_ready_scoped_path_already_covered(
            &ready,
            "title-other",
            "/library/show/s01e01.mkv"
        ));
    }

    #[test]
    fn pre_enumerated_empty_full_folder_counts_as_full_coverage() {
        let mut title = numeric_series_title();
        title.id = "title-empty".into();
        let work = LibraryScanTitleWork {
            title,
            facet_plan: LibraryScanTitleFacetPlan::Episodic,
            scope: LibraryScanTitleWorkScope::PreEnumeratedFullFolder(Vec::new()),
            mode: LibraryScanTitleWalkMode::Full,
            created_in_scan: false,
        };

        let coverage = LibraryScanWorkCoverage::from_work(&work);

        assert!(coverage.full_folder);
        assert!(coverage.scoped_paths.is_empty());
        assert_eq!(work.discovered_file_count(), 0);
    }

    #[test]
    fn scope_merge_preserves_scoped_paths_when_full_inventory_arrives() {
        let mut scope = LibraryScanTitleWorkScope::ScopedFiles(vec![ready_queue_test_file(
            "/library/show/loose.mkv",
        )]);
        scope.merge(LibraryScanTitleWorkScope::PreEnumeratedFullFolder(vec![
            ready_queue_test_file("/library/show/s01e01.mkv"),
        ]));

        assert!(scope.has_full_folder_coverage());
        let files = scope.discovered_files().expect("merged files");
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .any(|file| file.path == "/library/show/loose.mkv")
        );
        assert!(
            files
                .iter()
                .any(|file| file.path == "/library/show/s01e01.mkv")
        );
    }

    #[tokio::test]
    async fn parses_anonymized_numeric_series_season_two_filename_for_title_scan() {
        let title = numeric_series_title();
        let episodes = vec![
            numeric_series_episode("1", "1"),
            numeric_series_episode("2", "1"),
        ];
        let path = Path::new(
            "/library/13 (2024)/Season 02/13 (2024) - S02E01 - Day 2 800 A.M. 900 A.M. [WEBDL-1080p] [EAC3 5.1] [h265].mkv",
        );

        let parsed = parse_library_filename(&LibraryFilenameParseInput {
            path,
            display_name: Some("13 (2024) - S02E01"),
            library_root: Some(Path::new("/library")),
            title: Some(&title),
            facet: Some(&title.facet),
            collections: &[],
            series_movie_links: &[],
            episodes: &episodes,
            existing_record: None,
            mode: LibraryFilenameParseMode::TitleScan,
            fallback_policy: LibraryFilenameFallbackPolicy::NeedReleaseMetadata,
        });
        let episode = parsed
            .parsed_release
            .episode
            .as_ref()
            .expect("episode metadata");

        assert_eq!(parsed.parsed_release.normalized_title, "13");
        assert_eq!(episode.season, Some(2));
        assert_eq!(episode.episode_numbers, vec![1]);
    }

    fn ranked(file_id: &str, tier_key: usize, score: i32) -> RankedMovieFile {
        RankedMovieFile {
            tier_key,
            score,
            size_bytes: 1_000,
            file_path: format!("/movies/{file_id}.mkv"),
            file_id: file_id.to_string(),
        }
    }

    fn elect(mut candidates: Vec<RankedMovieFile>) -> String {
        candidates.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        candidates
            .first()
            .map(|candidate| candidate.file_id.clone())
            .expect("fixture is non-empty")
    }

    /// The scan must elect the file the admission gate would call the bar:
    /// **tier before score**. With the quality tier out of the number (D11) a
    /// 720p file can out-score a 2160p one, and electing it as primary would
    /// make the 2160p file already on disk look like an upgrade candidate.
    #[test]
    fn the_scan_elects_the_better_tier_even_when_it_scores_lower() {
        assert_eq!(
            elect(vec![ranked("uhd", 0, 100), ranked("hd", 2, 300)]),
            "uhd"
        );
    }

    /// A quality the profile does not list sorts last, matching
    /// `admission::tier_cmp` — it is only a candidate at all because some other
    /// gate let it through.
    #[test]
    fn an_untiered_file_never_wins_the_primary_role_over_a_tiered_one() {
        assert_eq!(
            elect(vec![
                ranked("unlisted", crate::admission::tier_sort_key(None), 9_000),
                ranked("listed", 3, -50),
            ]),
            "listed"
        );
    }

    /// Within one tier the bar decides, and the size / path / id tie-breaks keep
    /// the election deterministic below that.
    #[test]
    fn within_a_tier_the_bar_then_the_tie_breaks_decide() {
        assert_eq!(
            elect(vec![ranked("weak", 1, 100), ranked("strong", 1, 400)]),
            "strong"
        );

        let mut bigger = ranked("bigger", 1, 400);
        bigger.size_bytes = 5_000;
        assert_eq!(elect(vec![ranked("smaller", 1, 400), bigger]), "bigger");
    }
}
