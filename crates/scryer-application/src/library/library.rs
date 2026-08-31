use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::domain_events::DomainEventActor;
use crate::library_discovery::{
    BackgroundRefreshProbeOutcome, MovieTopLevelEntry, elapsed_ms_u64, list_child_directories,
    list_movie_top_level_entries, matching_movie_nfo_path, run_background_refresh_probe_with_delta,
    stream_child_directories_batched, stream_movie_top_level_entries_batched,
};
use crate::library_scan::LibraryDirectoryScanResult;
use crate::library_scan_coordinator::LibraryScanCoordinator;
use crate::library_scan_helpers::{
    LibraryScanSessionDropGuard, spawn_library_discovery_queue,
    wait_for_projected_library_scan_session,
};
use crate::library_scan_metadata::{
    LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE, MetadataLookupBatchStats, MetadataSearchResults,
    PreparedMovieLibraryScanCandidate, PreparedSeriesLibraryScanCandidate,
    build_movie_metadata_batch_stats, build_series_metadata_batch_stats,
    movie_candidate_batch_search_keys, prepare_movie_library_scan_entries,
    prepare_series_library_scan_candidates, resolve_refresh_metadata_batches,
    select_movie_metadata_from_batch_results, select_series_metadata_from_batch_results,
    series_candidate_batch_search_keys,
};
use crate::library_scan_titles::{
    TitleNameIndex, append_movie_title, append_series_title, build_movie_probe_path_indexes,
    build_movie_title_indexes, build_new_title_from_metadata_match,
    build_series_title_folder_path_index, build_series_title_indexes,
    find_existing_title_index_for_metadata_match, title_year_compatible,
    update_movie_probe_path_index, update_series_title_folder_path_index,
};
use crate::library_scan_unmatched::{
    build_movie_unmatched_scan_item, build_series_unmatched_scan_item,
    clear_library_scan_unmatched_item, format_library_scan_unmatched_search_attempts,
    normalize_library_scan_item_path, persist_library_scan_unmatched_item,
    reconcile_library_scan_unmatched_items,
};
use crate::settings::settings::{
    effective_scan_roots_from_root_folders, root_folder_entries_from_library_roots,
};
use tracing::{debug, warn};

const LIBRARY_SCAN_MOVIE_BATCH_SIZE: usize = 32;
const LIBRARY_SCAN_SERIES_BATCH_SIZE: usize = 8;
const TITLE_SCAN_FILE_BATCH_SIZE: usize = 128;
#[path = "scan/candidates.rs"]
mod scan_candidates;
#[path = "scan/full.rs"]
mod scan_full;
#[path = "scan/metadata_refresh.rs"]
mod scan_metadata_refresh;
#[path = "scan/pipeline.rs"]
mod scan_pipeline;
#[path = "scan/refresh.rs"]
mod scan_refresh;
#[path = "scan/title_files.rs"]
mod scan_title_files;
#[path = "scan/title_finalize.rs"]
mod scan_title_finalize;
#[path = "scan/title_scan.rs"]
mod scan_title_scan;

use scan_candidates::{
    process_movie_full_scan_candidate, process_movie_refresh_candidate,
    process_resolved_movie_full_scan_candidate, process_resolved_movie_refresh_candidate,
    process_resolved_series_full_scan_candidate, process_resolved_series_refresh_candidate,
    process_series_full_scan_candidate, process_series_refresh_candidate,
    scan_episodic_title_directory_for_progress_metrics,
};
use scan_full::{scan_library_movies, scan_library_series};
use scan_pipeline::{
    LibraryScanPipelineKind, LibraryScanPipelineRequest, run_library_scan_pipeline,
};
use scan_refresh::{
    background_refresh_movies, background_refresh_series,
    maybe_probe_existing_series_title_for_background_refresh,
};
pub(crate) use scan_title_files::{
    FileSourceSnapshot, PlannedTitleScanFile, PlannedTitleScanRecord,
    file_source_snapshot_from_library_file, file_source_snapshot_from_path,
};
use scan_title_files::{
    TitleScanLayoutSummary, classify_title_scan_layout, merge_title_scan_option_tags,
    title_media_file_matches_snapshot,
};
use scan_title_finalize::finalize_movie_scan_file;
pub(crate) use scan_title_finalize::finalize_title_scan_file;
use scan_title_scan::{LibraryScanMediaAnalysisPolicy, LibraryScanMediaAnalysisPool};

/// Destination for matched title work. Implemented by the media-analysis pool
/// (direct dispatch for background refresh and one-off scans) and by the full-scan
/// pipeline's staging sink (rendezvous with candidate inventory).
trait LibraryScanTitleWorkQueue: Send {
    fn enqueue(&mut self, work: LibraryScanTitleWork) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryScanTitleWalkMode {
    Full,
    Additive,
    OneOff,
}

impl LibraryScanTitleWalkMode {
    fn as_file_finalize_mode(self) -> LibraryScanMode {
        match self {
            Self::Additive => LibraryScanMode::Additive,
            Self::Full | Self::OneOff => LibraryScanMode::Full,
        }
    }

    fn allows_existing_additional_role_promotion(self) -> bool {
        matches!(self, Self::OneOff)
    }
}

#[derive(Clone, Debug, Default)]
struct LibraryScanMovieCleanupContext {
    canonical_folder_path: Option<String>,
    scan_folder_path: Option<String>,
    stale_collection_ids: Vec<String>,
}

#[derive(Clone, Debug)]
enum LibraryScanTitleFacetPlan {
    Movie(LibraryScanMovieCleanupContext),
    Episodic,
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryScanTitleWork {
    title: Title,
    facet_plan: LibraryScanTitleFacetPlan,
    scope: LibraryScanTitleWorkScope,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
}

impl LibraryScanTitleWork {
    fn discovered_file_count(&self) -> usize {
        self.scope.discovered_file_count()
    }

    fn discovered_files(&self) -> Option<&Vec<LibraryFile>> {
        self.scope.discovered_files()
    }

    fn discovered_files_mut(&mut self) -> Option<&mut Vec<LibraryFile>> {
        self.scope.discovered_files_mut()
    }

    fn has_full_folder_coverage(&self) -> bool {
        self.scope.has_full_folder_coverage()
    }

    fn requires_folder_enumeration(&self) -> bool {
        matches!(self.scope, LibraryScanTitleWorkScope::FullFolder)
    }
}

#[derive(Clone, Debug)]
enum LibraryScanTitleWorkScope {
    FullFolder,
    PreEnumeratedFullFolder(Vec<LibraryFile>),
    ScopedFiles(Vec<LibraryFile>),
}

impl LibraryScanTitleWorkScope {
    fn discovered_file_count(&self) -> usize {
        self.discovered_files().map(Vec::len).unwrap_or_default()
    }

    fn discovered_files(&self) -> Option<&Vec<LibraryFile>> {
        match self {
            Self::FullFolder => None,
            Self::PreEnumeratedFullFolder(files) | Self::ScopedFiles(files) => Some(files),
        }
    }

    fn discovered_files_mut(&mut self) -> Option<&mut Vec<LibraryFile>> {
        match self {
            Self::FullFolder => None,
            Self::PreEnumeratedFullFolder(files) | Self::ScopedFiles(files) => Some(files),
        }
    }

    fn has_full_folder_coverage(&self) -> bool {
        matches!(self, Self::FullFolder | Self::PreEnumeratedFullFolder(_))
    }

    fn into_discovered_files(self) -> Option<Vec<LibraryFile>> {
        match self {
            Self::FullFolder => None,
            Self::PreEnumeratedFullFolder(files) | Self::ScopedFiles(files) => Some(files),
        }
    }

    fn merge(&mut self, other: Self) {
        let current = std::mem::replace(self, Self::ScopedFiles(Vec::new()));
        *self = match (current, other) {
            (Self::FullFolder, _) | (_, Self::FullFolder) => Self::FullFolder,
            (Self::PreEnumeratedFullFolder(mut existing), Self::PreEnumeratedFullFolder(files))
            | (Self::PreEnumeratedFullFolder(mut existing), Self::ScopedFiles(files)) => {
                append_unique_library_files(&mut existing, files);
                Self::PreEnumeratedFullFolder(existing)
            }
            (Self::ScopedFiles(mut existing), Self::ScopedFiles(files)) => {
                append_unique_library_files(&mut existing, files);
                Self::ScopedFiles(existing)
            }
            (Self::ScopedFiles(mut existing), Self::PreEnumeratedFullFolder(files)) => {
                append_unique_library_files(&mut existing, files);
                Self::PreEnumeratedFullFolder(existing)
            }
        };
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LibraryTitleWalkResult {
    summary: LibraryScanSummary,
}

fn append_unique_library_files(target: &mut Vec<LibraryFile>, files: Vec<LibraryFile>) -> usize {
    let mut added = 0usize;

    for file in files {
        if target.iter().any(|existing| existing.path == file.path) {
            continue;
        }

        target.push(file);
        added = added.saturating_add(1);
    }

    added
}

fn normalized_movie_work_folder(path: Option<&str>) -> Option<String> {
    path.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn movie_work_folders_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (
        normalized_movie_work_folder(left),
        normalized_movie_work_folder(right),
    ) {
        (Some(left), Some(right)) => crate::stored_paths::folder_paths_match(&left, &right),
        (None, None) => true,
        _ => false,
    }
}

fn merge_library_scan_title_work(
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    work: LibraryScanTitleWork,
) -> bool {
    let title_id = work.title.id.clone();
    match workset.get_mut(&title_id) {
        Some(existing) => {
            if let (
                LibraryScanTitleFacetPlan::Movie(existing_cleanup),
                LibraryScanTitleFacetPlan::Movie(new_cleanup),
            ) = (&mut existing.facet_plan, &work.facet_plan)
            {
                let existing_folder_path = existing_cleanup
                    .canonical_folder_path
                    .as_deref()
                    .or(existing_cleanup.scan_folder_path.as_deref());
                if !movie_work_folders_match(
                    existing_folder_path,
                    new_cleanup.scan_folder_path.as_deref(),
                ) {
                    for collection_id in &new_cleanup.stale_collection_ids {
                        if !existing_cleanup
                            .stale_collection_ids
                            .contains(collection_id)
                        {
                            existing_cleanup
                                .stale_collection_ids
                                .push(collection_id.clone());
                        }
                    }
                    return false;
                }

                if existing_cleanup.canonical_folder_path.is_none() {
                    existing_cleanup.canonical_folder_path =
                        new_cleanup.canonical_folder_path.clone();
                }
            }

            existing.scope.merge(work.scope);

            if let (
                LibraryScanTitleFacetPlan::Movie(existing_cleanup),
                LibraryScanTitleFacetPlan::Movie(new_cleanup),
            ) = (&mut existing.facet_plan, work.facet_plan)
            {
                for collection_id in new_cleanup.stale_collection_ids {
                    if !existing_cleanup
                        .stale_collection_ids
                        .contains(&collection_id)
                    {
                        existing_cleanup.stale_collection_ids.push(collection_id);
                    }
                }
            }

            existing.title = work.title;
            existing.mode = work.mode;
            existing.created_in_scan |= work.created_in_scan;
            true
        }
        None => {
            if let LibraryScanTitleFacetPlan::Movie(cleanup) = &work.facet_plan {
                let canonical_folder_path = cleanup
                    .canonical_folder_path
                    .as_deref()
                    .or(cleanup.scan_folder_path.as_deref());
                if !movie_work_folders_match(
                    canonical_folder_path,
                    cleanup.scan_folder_path.as_deref(),
                ) {
                    return false;
                }
            }
            workset.insert(title_id, work);
            true
        }
    }
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
pub(crate) struct TitleScanFinalizeOutcome {
    progress: TitleScanProgressDelta,
    title_updated: bool,
}

#[derive(Clone, Debug)]
enum StartedLibraryScanOutcome {
    Completed(LibraryScanSummary),
    Canceled(LibraryScanSummary),
}

struct StartedLibraryScanRequest {
    actor: User,
    facet: MediaFacet,
    library_id: String,
    library_paths: Vec<String>,
    session_id: String,
    mode: LibraryScanMode,
    scan_hints: Option<LibraryScanHintSet>,
}

#[derive(Clone, Debug)]
struct InvalidLibraryRoot {
    path: String,
    reason: String,
}

pub(crate) fn library_scan_cancel_requested(token: Option<&CancellationToken>) -> bool {
    token.is_some_and(CancellationToken::is_cancelled)
}

async fn flush_title_scan_progress_batch(
    app: &AppUseCase,
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
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    if delta.completed > 0 {
        coordinator.mark_file_completed(delta.completed).await;
    }
    if delta.failed > 0 {
        coordinator.mark_file_failed(delta.failed).await;
    }
    coordinator.publish_progress().await;
}

fn slug_from_library_name(name: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "library".to_string()
    } else {
        slug
    }
}

pub(crate) fn normalize_library_root_drafts(
    mut roots: Vec<LibraryRootDraft>,
) -> AppResult<Vec<LibraryRootDraft>> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    let mut saw_default = false;
    for mut root in roots.drain(..) {
        root.path = scryer_domain::trim_library_root_path(&root.path);
        if root.path.is_empty() {
            continue;
        }
        let key = normalize_library_root_path(&root.path);
        if !seen.insert(key) {
            return Err(AppError::Validation(
                "library roots must be unique within a library".into(),
            ));
        }
        if root.is_default {
            if saw_default {
                root.is_default = false;
            } else {
                saw_default = true;
            }
        }
        normalized.push(root);
    }
    if !saw_default && let Some(first) = normalized.first_mut() {
        first.is_default = true;
    }
    Ok(normalized)
}

fn normalize_library_root_path(path: &str) -> String {
    scryer_domain::normalize_library_root_path(path)
}

fn conflicting_library_names_for_roots(
    libraries: &[Library],
    current_library_id: Option<&str>,
    roots: &[LibraryRootDraft],
) -> HashMap<String, Vec<String>> {
    let mut other_libraries_by_root = HashMap::<String, Vec<String>>::new();
    for library in libraries {
        if current_library_id.is_some_and(|library_id| library.id == library_id) {
            continue;
        }

        for root in &library.roots {
            let normalized_path = normalize_library_root_path(&root.path);
            if normalized_path.is_empty() {
                continue;
            }

            let names = other_libraries_by_root.entry(normalized_path).or_default();
            if !names.contains(&library.name) {
                names.push(library.name.clone());
            }
        }
    }

    let mut conflicts = HashMap::<String, Vec<String>>::new();
    for root in roots {
        let normalized_path = normalize_library_root_path(&root.path);
        if normalized_path.is_empty() {
            continue;
        }
        if let Some(names) = other_libraries_by_root.get(&normalized_path) {
            conflicts.insert(root.path.clone(), names.clone());
        }
    }

    conflicts
}

impl AppUseCase {
    pub(crate) async fn validate_library_root_conflicts(
        &self,
        current_library_id: Option<&str>,
        roots: &[LibraryRootDraft],
    ) -> AppResult<()> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        let conflicts = conflicting_library_names_for_roots(&libraries, current_library_id, roots);
        if let Some((path, library_names)) = conflicts.into_iter().next() {
            let libraries = library_names.join(", ");
            return Err(AppError::Validation(format!(
                "library root '{path}' is already used by {libraries}"
            )));
        }
        Ok(())
    }

    pub(crate) async fn require_library_management_permission(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<()> {
        if self
            .require_library_permission(
                actor,
                library_id,
                scryer_domain::LibraryPermission::ManageLibrary,
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await
    }

    pub async fn external_import_library(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<Library> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))
    }

    pub async fn create_library(
        &self,
        actor: &User,
        facet: MediaFacet,
        name: String,
        roots: Vec<LibraryRootDraft>,
        settings: Option<LibrarySettingsOverrideDraft>,
    ) -> AppResult<Library> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation("library name is required".into()));
        }
        let roots = normalize_library_root_drafts(roots)?;
        if roots.is_empty() {
            return Err(AppError::Validation(
                "libraries require at least one root folder".into(),
            ));
        }
        self.validate_library_root_conflicts(None, &roots).await?;
        let now = Utc::now();
        let library = Library {
            id: Id::new().0,
            facet,
            slug: slug_from_library_name(&name),
            name,
            is_default: false,
            roots: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let library = self
            .services
            .catalog
            .libraries
            .create(library, roots)
            .await?;
        if let Some(settings) = settings {
            self.update_library_settings(actor, &library.id, settings)
                .await?;
        }
        self.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", library.id)))
    }

    pub async fn update_library(
        &self,
        actor: &User,
        library_id: &str,
        name: Option<String>,
        roots: Option<Vec<LibraryRootDraft>>,
        settings: Option<LibrarySettingsOverrideDraft>,
    ) -> AppResult<Library> {
        let existing = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &existing.id)
            .await?;
        let name = name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| existing.name.clone());
        let roots_were_provided = roots.is_some();
        if roots_were_provided {
            // Retiring or repointing a root under a running operation would move
            // the ground out from under it; a name-only edit is harmless
            // (FR-084).
            self.ensure_location_ownership_allows_library_roots(
                &crate::location::ownership_guard::LIBRARY_ROOTS_UPDATE_ENTRY,
                &existing.id,
            )
            .await?;
        }
        let roots = match roots {
            Some(roots) => normalize_library_root_drafts(roots)?,
            None => existing
                .roots
                .iter()
                .map(|root| LibraryRootDraft {
                    path: root.path.clone(),
                    is_default: root.is_default,
                })
                .collect(),
        };
        if roots_were_provided && roots.is_empty() {
            return Err(AppError::Validation(
                "libraries require at least one root folder".into(),
            ));
        }
        let previous_roots = if roots_were_provided {
            let root_folders = root_folder_entries_from_library_roots(&existing.roots);
            Some(effective_scan_roots_from_root_folders(&root_folders))
        } else {
            None
        };
        self.validate_library_root_conflicts(Some(&existing.id), &roots)
            .await?;
        let slug = if existing.is_default {
            scryer_domain::default_library_slug_for_facet(&existing.facet).to_string()
        } else {
            slug_from_library_name(&name)
        };
        let library = self
            .services
            .catalog
            .libraries
            .update(&existing.id, name.clone(), slug, roots)
            .await?;
        if let Some(settings) = settings {
            self.update_library_settings(actor, &library.id, settings)
                .await?;
        }
        if roots_were_provided && library.is_default {
            let root_folders = root_folder_entries_from_library_roots(&library.roots);
            self.mirror_default_library_roots_to_legacy_settings(
                &library.facet,
                &root_folders,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;
        }
        if let Some(previous_roots) = previous_roots {
            let root_folders = root_folder_entries_from_library_roots(&library.roots);
            let current_roots = effective_scan_roots_from_root_folders(&root_folders);
            self.clear_pending_imports_for_removed_roots(
                &library.facet,
                &previous_roots,
                &current_roots,
            )
            .await?;
        }
        self.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", library.id)))
    }

    pub async fn delete_library(&self, actor: &User, library_id: &str) -> AppResult<bool> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;
        if library.is_default {
            return Err(AppError::Validation(
                "default libraries cannot be deleted".into(),
            ));
        }
        // Deleting the library retires every root it configures (FR-084).
        self.ensure_location_ownership_allows_library_roots(
            &crate::location::ownership_guard::LIBRARY_DELETE_ENTRY,
            &library.id,
        )
        .await?;

        for session in self
            .active_library_scan_sessions()
            .await
            .into_iter()
            .filter(|session| session.library_id.as_deref() == Some(library.id.as_str()))
        {
            if let Some(token) = self
                .library_scan_cancellation_token(&session.session_id)
                .await
            {
                token.cancel();
            }
            self.runtime
                .library
                .library_scan_tracker
                .cancel_session(&session.session_id)
                .await;
            self.clear_library_scan_cancellation_token(&session.session_id)
                .await;
        }

        self.services
            .library
            .library_scan_unmatched_items
            .delete_for_library(&library.id)
            .await?;

        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(
                Some(library.facet.clone()),
                std::slice::from_ref(&library.id),
                None,
            )
            .await?;
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        let actor_event = DomainEventActor::from(actor);

        for title in &titles {
            self.purge_title_logical_dependents(title, false, actor_event.clone())
                .await?;
        }

        if !title_ids.is_empty() {
            self.services
                .events
                .domain_events
                .delete_for_title_ids(&title_ids)
                .await?;
            self.services
                .workflow
                .housekeeping
                .delete_history_events_for_title_ids(&title_ids)
                .await?;
            self.services
                .workflow
                .housekeeping
                .delete_download_import_artifacts_for_title_ids(&title_ids)
                .await?;
            self.services
                .workflow
                .housekeeping
                .delete_release_attempts_for_title_ids(&title_ids)
                .await?;
        }

        for title in &titles {
            self.delete_title_row(title, actor_event.clone(), true)
                .await?;
        }

        self.services
            .config
            .settings
            .delete_values_for_scope_id(&library.id)
            .await?;

        let deleted = self
            .services
            .catalog
            .libraries
            .delete_library(&library.id)
            .await?;
        if deleted {
            self.refresh_download_client_category_admission_best_effort()
                .await;
        }
        Ok(deleted)
    }

    pub(crate) async fn ensure_library_scan_cancellation_token(
        &self,
        session_id: &str,
        mode: LibraryScanMode,
    ) -> Option<CancellationToken> {
        if mode != LibraryScanMode::Full {
            return None;
        }

        let mut tokens = self
            .runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await;
        if let Some(existing) = tokens.get(session_id).cloned() {
            return Some(existing);
        }

        let token = CancellationToken::new();
        tokens.insert(session_id.to_string(), token.clone());
        Some(token)
    }

    async fn library_scan_cancellation_token(&self, session_id: &str) -> Option<CancellationToken> {
        self.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .get(session_id)
            .cloned()
    }

    pub(crate) async fn clear_library_scan_cancellation_token(&self, session_id: &str) {
        self.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .remove(session_id);
    }

    pub async fn cancel_library_scan(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<CancelLibraryScanResult> {
        let session = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("library scan session {session_id}")))?;
        if let Some(library_id) = session.library_id.as_deref() {
            self.require_library_management_permission(actor, library_id)
                .await?;
        } else {
            self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
                .await?;
        }

        if session.mode != LibraryScanMode::Full {
            return Err(AppError::Validation(
                "only full library scans can be canceled".into(),
            ));
        }

        let token = self
            .library_scan_cancellation_token(session_id)
            .await
            .ok_or_else(|| AppError::Validation("library scan session is not cancelable".into()))?;
        token.cancel();

        Ok(CancelLibraryScanResult {
            session_id: session_id.to_string(),
            accepted: true,
        })
    }

    pub async fn scan_library(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSummary> {
        self.scan_library_with_tracking(actor, facet, None, LibraryScanMode::Full)
            .await
    }

    pub async fn trigger_library_scan(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSession> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let library_paths = self.read_library_paths_for_scan_facet(&facet).await?;
        let library_id = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
            .map(|library| library.id)
            .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet));
        let (_coordinator, session) = LibraryScanCoordinator::start_for_library(
            self.clone(),
            facet.clone(),
            Some(library_id.clone()),
            LibraryScanMode::Full,
            None,
        )
        .await?;
        self.ensure_library_scan_cancellation_token(&session.session_id, LibraryScanMode::Full)
            .await;
        let mut session_guard =
            LibraryScanSessionDropGuard::new(self.clone(), session.session_id.clone());

        let app = self.clone();
        let actor = actor.clone();
        let session_id = session.session_id.clone();
        tokio::spawn(async move {
            let request = StartedLibraryScanRequest {
                actor,
                facet: facet.clone(),
                library_id: library_id.clone(),
                library_paths: library_paths.clone(),
                session_id: session_id.clone(),
                mode: LibraryScanMode::Full,
                scan_hints: None,
            };
            let result = app.run_started_library_scan_session(&request).await;
            if let Err(error) = result {
                warn!(
                    error = %error,
                    session_id = %session_id,
                    facet = facet.as_str(),
                    "library scan task failed"
                );
                LibraryScanCoordinator::new(app.clone(), session_id.clone())
                    .fail()
                    .await;
            }
        });

        session_guard.disarm();
        Ok(session)
    }

    pub async fn trigger_library_scan_by_id(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<LibraryScanSession> {
        self.trigger_library_scan_by_id_with_hints(actor, library_id, None)
            .await
    }

    pub async fn trigger_library_scan_by_id_with_hints(
        &self,
        actor: &User,
        library_id: &str,
        scan_hints: Option<LibraryScanHintSet>,
    ) -> AppResult<LibraryScanSession> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;
        let library_paths = library
            .roots
            .iter()
            .map(|root| root.path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        if library_paths.is_empty() {
            return Err(AppError::Validation(
                "library roots are not configured".into(),
            ));
        }

        let (_coordinator, session) = LibraryScanCoordinator::start_for_library(
            self.clone(),
            library.facet.clone(),
            Some(library.id.clone()),
            LibraryScanMode::Full,
            None,
        )
        .await?;
        self.ensure_library_scan_cancellation_token(&session.session_id, LibraryScanMode::Full)
            .await;
        let mut session_guard =
            LibraryScanSessionDropGuard::new(self.clone(), session.session_id.clone());

        let app = self.clone();
        let actor = actor.clone();
        let session_id = session.session_id.clone();
        let facet = library.facet.clone();
        let library_id = library.id.clone();
        tokio::spawn(async move {
            let request = StartedLibraryScanRequest {
                actor,
                facet: facet.clone(),
                library_id: library_id.clone(),
                library_paths: library_paths.clone(),
                session_id: session_id.clone(),
                mode: LibraryScanMode::Full,
                scan_hints,
            };
            let result = app.run_started_library_scan_session(&request).await;
            match result {
                Ok(_) => {}
                Err(error) => {
                    warn!(error = %error, session_id = %session_id, "library scan task failed");
                    LibraryScanCoordinator::new(app.clone(), session_id.clone())
                        .fail()
                        .await;
                }
            }
        });

        session_guard.disarm();
        Ok(session)
    }

    pub(crate) async fn scan_library_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id_override: Option<String>,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSummary> {
        let session_library_id = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
            .map(|library| library.id)
            .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet));
        self.require_library_management_permission(actor, &session_library_id)
            .await?;
        let library_paths = self.read_library_paths_for_scan_facet(&facet).await?;
        let (_coordinator, session) = LibraryScanCoordinator::start_for_library(
            self.clone(),
            facet.clone(),
            Some(session_library_id.clone()),
            mode.clone(),
            session_id_override,
        )
        .await?;
        let mut session_guard =
            LibraryScanSessionDropGuard::new(self.clone(), session.session_id.clone());

        self.ensure_library_scan_cancellation_token(&session.session_id, mode.clone())
            .await;
        let request = StartedLibraryScanRequest {
            actor: actor.clone(),
            facet,
            library_id: session_library_id,
            library_paths,
            session_id: session.session_id.clone(),
            mode,
            scan_hints: None,
        };
        let result = self.run_started_library_scan_session(&request).await;

        if result.is_err() {
            LibraryScanCoordinator::new(self.clone(), session.session_id.clone())
                .fail()
                .await;
        }

        session_guard.disarm();

        match result {
            Ok(StartedLibraryScanOutcome::Completed(summary))
            | Ok(StartedLibraryScanOutcome::Canceled(summary)) => {
                let projected_session =
                    wait_for_projected_library_scan_session(self, &session.session_id).await?;

                if projected_session.status == LibraryScanStatus::Failed {
                    return Err(AppError::Repository("library scan failed".into()));
                }

                Ok(projected_session.summary.unwrap_or(summary))
            }
            Err(error) => Err(error),
        }
    }

    async fn run_started_library_scan_session(
        &self,
        request: &StartedLibraryScanRequest,
    ) -> AppResult<StartedLibraryScanOutcome> {
        // Every scan trigger funnels here, so one check covers them all: a scan
        // must not walk a root a location operation is moving files under
        // (FR-084).
        self.ensure_location_ownership_allows_library_roots(
            &crate::location::ownership_guard::LIBRARY_SCAN_ENTRY,
            &request.library_id,
        )
        .await?;
        let cancel_token = self
            .library_scan_cancellation_token(&request.session_id)
            .await;
        let should_apply_import_monitor_snapshot = request.mode == LibraryScanMode::Full;
        let summary = self
            .execute_started_library_scan_session(
                &request.actor,
                &request.facet,
                &request.library_id,
                &request.library_paths,
                &request.session_id,
                request.mode.clone(),
                cancel_token.clone(),
                request.scan_hints.clone(),
            )
            .await?;
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            self.cancel_started_library_scan_session(&request.session_id, &summary)
                .await;
            Ok(StartedLibraryScanOutcome::Canceled(summary))
        } else {
            if should_apply_import_monitor_snapshot
                && let Err(error) = self
                    .apply_pending_external_import_monitor_snapshot_for_library(
                        &request.facet,
                        &request.library_id,
                    )
                    .await
            {
                let warning_message =
                    "Imported Sonarr/Radarr monitored state could not be applied after this scan. Scryer will retry on the next full scan.".to_string();
                let _ = self
                    .runtime
                    .library
                    .library_scan_tracker
                    .set_warning_message(&request.session_id, Some(warning_message))
                    .await;
                warn!(
                    facet = request.facet.as_str(),
                    session_id = %request.session_id,
                    error = %error,
                    "failed to apply pending external import monitoring snapshot after full scan"
                );
            }
            self.finalize_started_library_scan_session(&request.session_id, &summary)
                .await;
            Ok(StartedLibraryScanOutcome::Completed(summary))
        }
    }

    async fn read_library_paths_for_scan_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Vec<String>> {
        let configured_roots = self.root_folders_for_facet(facet).await?;
        let mut roots = Vec::with_capacity(configured_roots.len());
        let mut seen_roots = HashSet::new();

        for root in configured_roots {
            let path = root.path.trim().to_string();
            if path.is_empty() || !seen_roots.insert(path.clone()) {
                continue;
            }
            roots.push(path);
        }

        if roots.is_empty() {
            return Err(AppError::Validation(format!(
                "{} library roots are not configured",
                facet.as_str()
            )));
        }

        Ok(roots)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "library scan sessions carry explicit runtime, permission, and cancellation context"
    )]
    async fn execute_started_library_scan_session(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_id: &str,
        library_paths: &[String],
        session_id: &str,
        mode: LibraryScanMode,
        cancel_token: Option<CancellationToken>,
        scan_hints: Option<LibraryScanHintSet>,
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let mut valid_roots = Vec::new();
        let mut invalid_roots = Vec::new();

        for library_path in library_paths {
            match tokio::fs::metadata(library_path).await {
                Ok(metadata) if metadata.is_dir() => valid_roots.push(library_path.as_str()),
                Ok(_) => invalid_roots.push(InvalidLibraryRoot {
                    path: library_path.clone(),
                    reason: "path exists but is not a directory".to_string(),
                }),
                Err(error) => invalid_roots.push(InvalidLibraryRoot {
                    path: library_path.clone(),
                    reason: error.to_string(),
                }),
            }
        }

        if valid_roots.is_empty() {
            if let Some(invalid_root) = invalid_roots.first() {
                return Err(AppError::Validation(format!(
                    "library path is not a directory: {}",
                    invalid_root.path
                )));
            }

            return Err(AppError::Validation(format!(
                "{} library roots are not configured",
                facet.as_str()
            )));
        }

        let mut summary = LibraryScanSummary::default();

        if !invalid_roots.is_empty() {
            warn!(
                session_id = %session_id,
                facet = facet.as_str(),
                invalid_root_count = invalid_roots.len(),
                valid_root_count = valid_roots.len(),
                "skipping invalid library roots during scan"
            );
            for invalid_root in &invalid_roots {
                warn!(
                    session_id = %session_id,
                    facet = facet.as_str(),
                    library_path = %invalid_root.path,
                    reason = %invalid_root.reason,
                    "skipping invalid library root"
                );
            }

            summary.skipped = summary.skipped.saturating_add(invalid_roots.len());
            coordinator.add_metadata_total(invalid_roots.len()).await;
            coordinator.mark_metadata_failed(invalid_roots.len()).await;
            coordinator.publish_progress().await;
        }

        let valid_root_count = valid_roots.len();

        for (root_index, library_path) in valid_roots.into_iter().enumerate() {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let finalize_discovery_on_drain =
                mode == LibraryScanMode::Full && root_index + 1 == valid_root_count;
            let root_summary = match (mode.clone(), facet) {
                (LibraryScanMode::Full, MediaFacet::Movie) => {
                    scan_library_movies(
                        self,
                        actor,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                        finalize_discovery_on_drain,
                        cancel_token.clone(),
                        scan_hints.as_ref(),
                    )
                    .await?
                }
                (LibraryScanMode::Full, MediaFacet::Series | MediaFacet::Anime) => {
                    // Both Series and Anime route through scan_library_series and
                    // look up hints under LibraryScanHintFacet::Series, which is how
                    // BOTH Sonarr series and Sonarr anime import hints are stamped.
                    // Previously Anime was passed None here, so anime imports lost
                    // their arr identity and fell back to the filesystem parser.
                    scan_library_series(
                        self,
                        actor,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                        finalize_discovery_on_drain,
                        cancel_token.clone(),
                        scan_hints.as_ref(),
                    )
                    .await?
                }
                (LibraryScanMode::Additive, MediaFacet::Movie) => {
                    background_refresh_movies(self, actor, library_id, library_path, session_id)
                        .await?
                }
                (LibraryScanMode::Additive, MediaFacet::Series | MediaFacet::Anime) => {
                    background_refresh_series(
                        self,
                        actor,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                    )
                    .await?
                }
            };
            summary.absorb(&root_summary);
        }

        if mode == LibraryScanMode::Additive || library_scan_cancel_requested(cancel_token.as_ref())
        {
            coordinator.mark_discovery_complete(false).await;
            coordinator.publish_progress().await;
        }

        Ok(summary)
    }

    async fn finalize_started_library_scan_session(
        &self,
        session_id: &str,
        summary: &LibraryScanSummary,
    ) {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        coordinator.set_summary(summary.clone()).await;
        coordinator.publish_progress().await;
        coordinator.maybe_complete().await;
    }

    async fn cancel_started_library_scan_session(
        &self,
        session_id: &str,
        summary: &LibraryScanSummary,
    ) {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        coordinator.set_summary(summary.clone()).await;
        coordinator.cancel().await;
    }

    pub(crate) async fn background_library_refresh_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let library = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("default {} library", facet.as_str())))?;
        self.background_library_refresh_single_library_with_tracking(actor, &library, session_id)
            .await
    }

    pub(crate) async fn background_library_refresh_by_id_with_tracking(
        &self,
        actor: &User,
        library_id: &str,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.background_library_refresh_single_library_with_tracking(actor, &library, session_id)
            .await
    }

    async fn background_library_refresh_single_library_with_tracking(
        &self,
        actor: &User,
        library: &Library,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        self.require_library_management_permission(actor, &library.id)
            .await?;
        let library_paths = library
            .roots
            .iter()
            .map(|root| root.path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        if library_paths.is_empty() {
            return Err(AppError::Validation(format!(
                "{} library roots are not configured",
                library.name
            )));
        }

        let (_coordinator, session) = LibraryScanCoordinator::start_for_library(
            self.clone(),
            library.facet.clone(),
            Some(library.id.clone()),
            LibraryScanMode::Additive,
            Some(session_id.to_string()),
        )
        .await?;

        let result = self
            .execute_started_library_scan_session(
                actor,
                &library.facet,
                &library.id,
                &library_paths,
                &session.session_id,
                LibraryScanMode::Additive,
                None,
                None,
            )
            .await;

        match result {
            Ok(summary) => {
                self.finalize_started_library_scan_session(&session.session_id, &summary)
                    .await;
                Ok(summary)
            }
            Err(error) => {
                LibraryScanCoordinator::new(self.clone(), session.session_id.clone())
                    .fail()
                    .await;
                Err(error)
            }
        }
    }
}
