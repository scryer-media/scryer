use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration as StdDuration;

use scryer_domain::{MediaFacet, NewTitle};

use chrono::Utc;
use tracing::warn;

use super::*;
use crate::library::library::{
    PlannedTitleScanFile, PlannedTitleScanRecord, file_source_snapshot_from_path,
    finalize_title_scan_file,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

const MAX_PENDING_IMPORTS_PAGE_SIZE: i64 = 200;

fn build_pending_import_search_attempt(
    attempt: &LibraryScanUnmatchedSearchAttempt,
) -> PendingImportSearchAttempt {
    let top_results = attempt.top_results.clone();
    let top_results_summary = if top_results.is_empty() {
        "no results".to_string()
    } else {
        top_results.join(" | ")
    };

    PendingImportSearchAttempt {
        query: attempt.query.clone(),
        result_count: attempt.result_count,
        top_results,
        summary: format!(
            "{} result{}: {}",
            attempt.result_count,
            if attempt.result_count == 1 { "" } else { "s" },
            top_results_summary
        ),
    }
}

fn pending_import_movie_entry_path(item: &LibraryScanUnmatchedItem) -> PathBuf {
    let item_path = stored_path_to_path_buf(item.item_path.trim());
    let scan_root = stored_path_to_path_buf(item.scan_root.trim());

    if let Ok(relative) = item_path.strip_prefix(&scan_root)
        && let Some(first_component) = relative.components().next()
    {
        return scan_root.join(first_component.as_os_str());
    }

    item_path
}

fn pending_import_folder_path(item: &LibraryScanUnmatchedItem) -> Option<String> {
    match item.facet {
        MediaFacet::Movie => {
            let entry_path = pending_import_movie_entry_path(item);
            let entry_path = path_to_stored_string(&entry_path).trim().to_string();
            if entry_path.is_empty() || entry_path == item.item_path {
                None
            } else {
                Some(entry_path)
            }
        }
        MediaFacet::Series | MediaFacet::Anime => Some(item.item_path.clone()),
    }
}

fn pending_import_item_from_unmatched(item: LibraryScanUnmatchedItem) -> PendingImportItem {
    let folder_path = pending_import_folder_path(&item);
    let search_attempts = item
        .search_attempts
        .iter()
        .map(build_pending_import_search_attempt)
        .collect();

    PendingImportItem {
        id: item.id,
        library_id: item.library_id,
        library_slug: None,
        facet: item.facet,
        status: item.status,
        title_id: item.title_id,
        title_name: None,
        title_slug: None,
        display_name: item.display_name,
        path: item.item_path,
        folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason_class: PendingImportReasonClass::from_reason_code(&item.reason_code),
        reason: item.reason_code,
        search_attempts,
        size_bytes: item.size_bytes,
        created_at: item.created_at,
    }
}

/// Fill in `size_bytes` for page rows the scanner never recorded a size for.
///
/// Only rows that read back as `None` are stat'ed, and only the current page
/// (capped at [`MAX_PENDING_IMPORTS_PAGE_SIZE`]), so this stays bounded. The
/// resolved value is deliberately not written back: the store column records
/// what the scanner observed, and a read-path backfill would silently rewrite
/// scan history.
async fn hydrate_pending_import_sizes(items: &mut [PendingImportItem]) {
    for item in items.iter_mut().filter(|item| item.size_bytes.is_none()) {
        let path = stored_path_to_path_buf(item.path.trim());
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        item.size_bytes = i64::try_from(metadata.len()).ok();
    }
}

async fn build_pending_import_library_file(
    item: &LibraryScanUnmatchedItem,
) -> AppResult<LibraryFile> {
    let item_path = item.item_path.trim();
    if item_path.is_empty() {
        return Err(AppError::Validation(
            "pending import path is missing or invalid".into(),
        ));
    }

    let path = stored_path_to_path_buf(item_path);
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        AppError::Validation(format!("pending import file is unavailable: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(AppError::Validation(
            "pending import path is not a file".into(),
        ));
    }

    let display_name = if item.display_name.trim().is_empty() {
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(item_path)
            .to_string()
    } else {
        item.display_name.clone()
    };

    Ok(LibraryFile {
        path: path_to_stored_string(&path).trim().to_string(),
        display_name,
        nfo_path: None,
        size_bytes: Some(metadata.len() as i64),
        source_signature_scheme: None,
        source_signature_value: None,
    })
}

async fn list_pending_import_title_episodes(
    app: &AppUseCase,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let mut episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(title_id)
        .await?;
    episodes.sort_by(|left, right| {
        let left_season = left
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let right_season = right
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let left_episode = left
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let right_episode = right
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        left_season
            .cmp(&right_season)
            .then(left_episode.cmp(&right_episode))
            .then(left.id.cmp(&right.id))
    });
    Ok(episodes)
}

fn pending_import_parse_raw_name(item: &LibraryScanUnmatchedItem) -> String {
    stored_path_to_path_buf(item.item_path.trim())
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| item.display_name.clone())
}

fn pending_import_suggested_episode_ids(
    parsed: &ParsedReleaseMetadata,
    available_episodes: &[Episode],
) -> Vec<String> {
    let Some(episode) = parsed.episode.as_ref() else {
        return Vec::new();
    };

    let mut suggested = Vec::new();

    if !episode.episode_numbers.is_empty() {
        let season_number = episode.season.unwrap_or(1).to_string();
        for episode_number in &episode.episode_numbers {
            let episode_number = episode_number.to_string();
            if let Some(matched) = available_episodes.iter().find(|candidate| {
                candidate.season_number.as_deref() == Some(season_number.as_str())
                    && candidate.episode_number.as_deref() == Some(episode_number.as_str())
            }) {
                suggested.push(matched.id.clone());
            }
        }
    }

    if suggested.is_empty()
        && let Some(absolute_episode) = episode.absolute_episode
    {
        let absolute_episode = absolute_episode.to_string();
        if let Some(matched) = available_episodes.iter().find(|candidate| {
            candidate.absolute_number.as_deref() == Some(absolute_episode.as_str())
        }) {
            suggested.push(matched.id.clone());
        }
    }

    if suggested.is_empty() && !episode.special_absolute_episode_numbers.is_empty() {
        for absolute_episode in &episode.special_absolute_episode_numbers {
            let absolute_episode = absolute_episode.to_string();
            if let Some(matched) = available_episodes.iter().find(|candidate| {
                candidate.absolute_number.as_deref() == Some(absolute_episode.as_str())
            }) {
                suggested.push(matched.id.clone());
            }
        }
    }

    if suggested.is_empty()
        && let Some(air_date) = episode.air_date
    {
        let air_date = air_date.to_string();
        suggested.extend(
            available_episodes
                .iter()
                .filter(|candidate| candidate.air_date.as_deref() == Some(air_date.as_str()))
                .map(|candidate| candidate.id.clone()),
        );
    }

    if suggested.is_empty() && episode.full_season {
        let season_number = episode.season.unwrap_or(1).to_string();
        suggested.extend(
            available_episodes
                .iter()
                .filter(|candidate| {
                    candidate.season_number.as_deref() == Some(season_number.as_str())
                })
                .map(|candidate| candidate.id.clone()),
        );
    }

    let mut deduped = Vec::with_capacity(suggested.len());
    let mut seen = HashSet::new();
    for episode_id in suggested {
        if seen.insert(episode_id.clone()) {
            deduped.push(episode_id);
        }
    }
    deduped
}

fn library_scan_summary_has_pending_import_success(summary: &LibraryScanSummary) -> bool {
    summary.imported > 0 || summary.matched > 0
}

fn pending_import_item_requires_action(item: &LibraryScanUnmatchedItem) -> bool {
    item.reason_code
        == crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
        || !(item.facet == MediaFacet::Movie && item.title_id.is_some())
}

fn reject_folder_ownership_conflict_resolution(item: &LibraryScanUnmatchedItem) -> AppResult<()> {
    if item.reason_code
        == crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
    {
        return Err(AppError::Validation(
            "folder ownership conflicts cannot be bound or adopted".into(),
        ));
    }
    Ok(())
}

struct PendingImportResolutionGuard {
    pending_import_id: String,
    locks: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl Drop for PendingImportResolutionGuard {
    fn drop(&mut self) {
        if let Ok(mut locks) = self.locks.lock() {
            locks.remove(&self.pending_import_id);
        }
    }
}

impl AppUseCase {
    fn acquire_pending_import_resolution_guard(
        &self,
        pending_import_id: &str,
    ) -> AppResult<PendingImportResolutionGuard> {
        let mut locks = self
            .pending_import_resolution_locks
            .lock()
            .map_err(|_| AppError::Repository("pending import resolution lock poisoned".into()))?;
        if !locks.insert(pending_import_id.to_string()) {
            return Err(AppError::Validation(format!(
                "pending import {pending_import_id} is already being resolved"
            )));
        }

        Ok(PendingImportResolutionGuard {
            pending_import_id: pending_import_id.to_string(),
            locks: self.pending_import_resolution_locks.clone(),
        })
    }

    pub async fn pending_import_counts(&self, actor: &User) -> AppResult<PendingImportCounts> {
        let manageable = self
            .authorized_library_ids(
                actor,
                None,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let repository = self.services.library.library_scan_unmatched_items.clone();
        let mut movie = 0;
        let mut series = 0;
        let mut anime = 0;
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let items = repository
                .list_library_scan_unmatched_items(
                    Some(facet.clone()),
                    None,
                    Some(PendingImportStatus::Pending),
                    i64::MAX,
                    0,
                )
                .await?;
            let count = items
                .into_iter()
                .filter(|item| {
                    manageable.contains(&item.library_id)
                        && pending_import_item_requires_action(item)
                })
                .count() as i64;
            match facet {
                MediaFacet::Movie => movie = count,
                MediaFacet::Series => series = count,
                MediaFacet::Anime => anime = count,
            }
        }

        Ok(PendingImportCounts {
            movie,
            series,
            anime,
        })
    }

    pub async fn pending_imports(
        &self,
        actor: &User,
        facet: MediaFacet,
        library_ids: Option<Vec<String>>,
        status: PendingImportStatus,
        limit: i64,
        offset: i64,
    ) -> AppResult<PendingImportConnection> {
        let limit = limit.clamp(0, MAX_PENDING_IMPORTS_PAGE_SIZE);
        let offset = offset.max(0);
        let manageable = self
            .authorized_library_ids(
                actor,
                Some(facet.clone()),
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = library_ids
            .unwrap_or_default()
            .into_iter()
            .map(|library_id| library_id.trim().to_string())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        let filtered = self
            .services
            .library
            .library_scan_unmatched_items
            .list_library_scan_unmatched_items(Some(facet), None, Some(status), i64::MAX, 0)
            .await?
            .into_iter()
            .filter(|item| {
                manageable.contains(&item.library_id)
                    && (requested_library_ids.is_empty()
                        || requested_library_ids.contains(&item.library_id))
                    && pending_import_item_requires_action(item)
            })
            .collect::<Vec<_>>();
        let total = filtered.len() as i64;
        let mut items = filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(pending_import_item_from_unmatched)
            .collect::<Vec<_>>();
        self.hydrate_pending_import_known_titles(&mut items).await?;
        hydrate_pending_import_sizes(&mut items).await;

        Ok(PendingImportConnection { total, items })
    }

    async fn hydrate_pending_import_known_titles(
        &self,
        items: &mut [PendingImportItem],
    ) -> AppResult<()> {
        let title_ids = items
            .iter()
            .filter_map(|item| item.title_id.as_deref())
            .map(str::trim)
            .filter(|title_id| !title_id.is_empty())
            .collect::<HashSet<_>>();
        if title_ids.is_empty() {
            return Ok(());
        }

        let mut known_titles = HashMap::with_capacity(title_ids.len());
        for title_id in title_ids {
            if let Some(title) = self.services.catalog.titles.get_by_id(title_id).await? {
                known_titles.insert(title_id.to_string(), (title.name, title.slug));
            }
        }

        for item in items.iter_mut() {
            let Some(title_id) = item
                .title_id
                .as_deref()
                .map(str::trim)
                .filter(|title_id| !title_id.is_empty())
            else {
                continue;
            };

            if let Some((title_name, title_slug)) = known_titles.get(title_id) {
                item.title_name = Some(title_name.clone());
                item.title_slug = title_slug.clone();
            }
        }

        Ok(())
    }

    pub async fn ignore_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
    ) -> AppResult<IgnorePendingImportResult> {
        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let mut item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        self.require_library_permission(
            actor,
            &item.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        if item.status != PendingImportStatus::Ignored {
            item.status = PendingImportStatus::Ignored;
            item.updated_at = Utc::now().to_rfc3339();
            self.services
                .library
                .library_scan_unmatched_items
                .upsert_library_scan_unmatched_item(&item)
                .await?;
        }

        Ok(IgnorePendingImportResult {
            id: item.id,
            status: item.status,
        })
    }

    pub async fn resolve_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
        mut request: NewTitle,
    ) -> AppResult<ResolvePendingImportResult> {
        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        self.require_library_permission(
            actor,
            &item.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        reject_folder_ownership_conflict_resolution(&item)?;
        if item.title_id.is_some() {
            return Err(AppError::Validation(
                "pending import requires explicit episode binding".into(),
            ));
        }

        request.facet = item.facet.clone();
        request.monitored = false;
        request.tags.clear();
        request.root_folder_id = None;
        request.min_availability = None;

        let (target_identity_source, target_identity_value) = match item.facet {
            MediaFacet::Movie => request
                .external_ids
                .iter()
                .find_map(|external_id| {
                    let source = if external_id.source.eq_ignore_ascii_case("smg") {
                        Some("smg")
                    } else if external_id.source.eq_ignore_ascii_case("tvdb") {
                        Some("tvdb")
                    } else if external_id.source.eq_ignore_ascii_case("tmdb") {
                        Some("tmdb")
                    } else if external_id.source.eq_ignore_ascii_case("imdb") {
                        Some("imdb")
                    } else {
                        None
                    }?;
                    let value = external_id.value.trim();
                    (!value.is_empty()).then(|| (source, value.to_string()))
                })
                .ok_or_else(|| AppError::Validation("a title identity is required".into()))?,
            MediaFacet::Series | MediaFacet::Anime => {
                let target_tvdb_id = request
                    .external_ids
                    .iter()
                    .find(|external_id| {
                        external_id.source.eq_ignore_ascii_case("tvdb")
                            && !external_id.value.trim().is_empty()
                    })
                    .map(|external_id| external_id.value.trim().to_string())
                    .ok_or_else(|| AppError::Validation("tvdb id is required".into()))?;
                ("tvdb", target_tvdb_id)
            }
        };

        if self
            .services
            .catalog
            .titles
            .find_by_external_id_in_library_and_facet(
                &item.library_id,
                item.facet.clone(),
                target_identity_source,
                &target_identity_value,
            )
            .await?
            .is_some()
        {
            return Err(AppError::Validation(
                "title already exists in this library".into(),
            ));
        }

        let outcome = self
            .add_title_and_bind_pending_import_with_outcome_in_library(
                actor,
                request,
                item.library_id.clone(),
                pending_import_id,
            )
            .await?;

        if outcome.reused_existing_title {
            return Err(AppError::Validation(
                "title already exists in this library".into(),
            ));
        }

        Ok(ResolvePendingImportResult {
            title: outcome.title,
            created: true,
            library_scan: None,
            metadata_hydration_state: outcome.metadata_hydration_state,
        })
    }

    pub async fn pending_import_title_search(
        &self,
        actor: &User,
        pending_import_id: &str,
        query: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }

        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        self.require_library_permission(
            actor,
            &item.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let limit = limit.clamp(1, 100);
        let search_limit = limit.saturating_mul(3).clamp(limit, 100);
        let results = self
            .services
            .library
            .metadata_gateway
            .search_tvdb_rich(query, item.facet.as_str(), search_limit, language, year)
            .await?;

        let mut seen_tvdb_ids = HashSet::new();
        let tvdb_ids = results
            .iter()
            .map(|result| result.tvdb_id.trim())
            .filter(|tvdb_id| !tvdb_id.is_empty())
            .filter(|tvdb_id| seen_tvdb_ids.insert((*tvdb_id).to_string()))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let existing_tvdb_ids = self
            .services
            .catalog
            .titles
            .list_existing_external_ids_in_library_and_facet(
                &item.library_id,
                item.facet.clone(),
                "tvdb",
                &tvdb_ids,
            )
            .await?;

        let mut filtered = Vec::with_capacity(limit as usize);
        for result in results {
            let tvdb_id = result.tvdb_id.trim();
            if !tvdb_id.is_empty() && existing_tvdb_ids.contains(tvdb_id) {
                continue;
            }

            filtered.push(result);
            if filtered.len() >= limit as usize {
                break;
            }
        }

        Ok(filtered)
    }

    pub async fn preview_title_bound_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
    ) -> AppResult<PendingImportBindingPreview> {
        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        self.require_library_permission(
            actor,
            &item.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        reject_folder_ownership_conflict_resolution(&item)?;
        let title_id = item.title_id.as_deref().ok_or_else(|| {
            AppError::Validation("pending import does not have a known title".into())
        })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

        let available_episodes = list_pending_import_title_episodes(self, &title.id).await?;
        let parse_raw_name = pending_import_parse_raw_name(&item);
        let parse_context =
            crate::build_release_parse_context_for_title(&title, &available_episodes, None);
        let parsed =
            crate::parse_release_metadata_for_target(parse_raw_name.as_str(), &parse_context);
        let suggested_episode_ids =
            pending_import_suggested_episode_ids(&parsed, &available_episodes);
        let file = build_pending_import_library_file(&item).await?;

        Ok(PendingImportBindingPreview {
            title,
            file: PendingImportBindingFilePreview {
                file_path: file.path.clone(),
                file_name: file.display_name.clone(),
                size_bytes: file.size_bytes.unwrap_or_default(),
                parsed_season: parsed.episode.as_ref().and_then(|episode| episode.season),
                parsed_episodes: parsed
                    .episode
                    .as_ref()
                    .map(|episode| episode.episode_numbers.clone())
                    .unwrap_or_default(),
                parsed_absolute_numbers: parsed
                    .episode
                    .as_ref()
                    .map(|episode| {
                        let mut absolute_numbers = episode.special_absolute_episode_numbers.clone();
                        if let Some(value) = episode.absolute_episode {
                            absolute_numbers.push(value);
                        }
                        absolute_numbers
                    })
                    .unwrap_or_default(),
                suggested_episode_ids,
            },
            available_episodes,
        })
    }

    pub async fn bind_title_bound_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
        collection_id: Option<&str>,
        episode_ids: &[String],
    ) -> AppResult<ResolvePendingImportResult> {
        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        self.require_library_permission(
            actor,
            &item.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        reject_folder_ownership_conflict_resolution(&item)?;
        let title_id = item.title_id.as_deref().ok_or_else(|| {
            AppError::Validation("pending import does not have a known title".into())
        })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        let available_episodes = list_pending_import_title_episodes(self, &title.id).await?;

        let target_episodes = if let Some(collection_id) = collection_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let episodes = available_episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .cloned()
                .collect::<Vec<_>>();
            if episodes.is_empty() {
                return Err(AppError::Validation(format!(
                    "collection {collection_id} does not belong to title {}",
                    title.id
                )));
            }
            episodes
        } else {
            let requested_ids = episode_ids
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect::<HashSet<_>>();
            if requested_ids.is_empty() {
                return Err(AppError::Validation(
                    "at least one episode must be selected".into(),
                ));
            }
            let episodes = available_episodes
                .iter()
                .filter(|episode| requested_ids.contains(episode.id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if episodes.len() != requested_ids.len() {
                return Err(AppError::Validation(
                    "one or more selected episodes do not belong to the target title".into(),
                ));
            }
            episodes
        };

        let file = build_pending_import_library_file(&item).await?;
        let parse_raw_name = pending_import_parse_raw_name(&item);
        let parse_context =
            crate::build_release_parse_context_for_title(&title, &available_episodes, None);
        let parsed =
            crate::parse_release_metadata_for_target(parse_raw_name.as_str(), &parse_context);
        let snapshot = file_source_snapshot_from_path(&stored_path_to_path_buf(&file.path)).await?;
        let analysis_outcome = match self
            .services
            .library
            .media_analyzer
            .analyze_file(stored_path_to_path_buf(&file.path))
            .await
        {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "failed to analyze title-bound pending import file"
                );
                None
            }
        };

        let mut episode_links = HashSet::new();
        let mut summary = LibraryScanSummary::default();
        let mut db_elapsed = StdDuration::ZERO;
        let mut external_subtitle_cache =
            crate::subtitles::ExternalSubtitleDirectoryCache::default();
        finalize_title_scan_file(
            self,
            &title,
            PlannedTitleScanFile {
                file,
                parsed,
                target_episodes,
                series_movie_link_id: None,
                snapshot,
                record: PlannedTitleScanRecord::New,
            },
            analysis_outcome,
            LibraryScanMode::Full,
            &mut episode_links,
            &mut summary,
            &mut db_elapsed,
            &mut external_subtitle_cache,
        )
        .await;

        if !library_scan_summary_has_pending_import_success(&summary) {
            return Err(AppError::Validation(
                "failed to bind pending import file to selected episodes".into(),
            ));
        }

        self.services
            .library
            .library_scan_unmatched_items
            .delete_library_scan_unmatched_item(
                &item.library_id,
                item.facet.clone(),
                &item.item_path,
            )
            .await?;

        let refreshed_title = self
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await?
            .unwrap_or(title);

        Ok(ResolvePendingImportResult {
            title: refreshed_title,
            created: false,
            library_scan: Some(summary),
            metadata_hydration_state: AddTitleHydrationState::NotRequired,
        })
    }
}
