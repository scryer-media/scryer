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

/// The identities a pending import's chosen title is looked up by, tried in
/// this order: SMG title id, TVDB id, TMDB id, and for a movie also its IMDb
/// id. Each is the first id of that source among `external_ids` whose kind
/// fits the facet. Every one is tried because any may be the one the library
/// stored: a series SMG knows only from TMDB has no TVDB id, and a title
/// added before SMG ids were stored has no SMG id. The resolver and the
/// search annotation both use this, so a candidate shown as already in the
/// library is exactly one the resolver finds there.
fn pending_import_target_identities(
    facet: &MediaFacet,
    external_ids: &[ExternalId],
) -> Vec<(&'static str, String)> {
    let sources: &[&'static str] = match facet {
        MediaFacet::Movie => &["smg", "tvdb", "tmdb", "imdb"],
        MediaFacet::Series | MediaFacet::Anime => &["smg", "tvdb", "tmdb"],
    };
    sources
        .iter()
        .filter_map(|source| {
            external_ids
                .iter()
                .find(|external_id| {
                    external_id.source.trim().eq_ignore_ascii_case(source)
                        && !external_id.value.trim().is_empty()
                        && crate::normalize::external_id_kind_fits_facet(external_id, facet)
                })
                .map(|external_id| (*source, external_id.value.trim().to_string()))
        })
        .collect()
}

/// Whether `title` carries `source:value` as an id of `facet`'s kind. The
/// title store matches source and value only; a TMDB id must also be kinded
/// for the facet (or not kinded), so an anime title's `tmdb:movie:N` never
/// stands for series `N`.
fn title_carries_identity(title: &Title, facet: &MediaFacet, source: &str, value: &str) -> bool {
    !source.eq_ignore_ascii_case("tmdb")
        || title.external_ids.iter().any(|external_id| {
            external_id.source.trim().eq_ignore_ascii_case("tmdb")
                && external_id.value.trim() == value
                && crate::normalize::external_id_kind_fits_facet(external_id, facet)
        })
}

/// The ids a title search result names its title by, in the order the web
/// client sends them when that result is chosen: the result's own ids first,
/// then its SMG, TVDB and IMDb ids.
fn search_result_external_ids(result: &RichMetadataSearchItem) -> Vec<ExternalId> {
    let mut external_ids = result.external_ids.clone();
    if let Some(smg_id) = result.smg_id {
        external_ids.push(ExternalId::new("smg", smg_id.to_string()));
    }
    let tvdb_id = result.tvdb_id.trim();
    if !tvdb_id.is_empty() {
        external_ids.push(ExternalId::new("tvdb", tvdb_id));
    }
    if let Some(imdb_id) = result
        .imdb_id
        .as_deref()
        .map(str::trim)
        .filter(|imdb_id| !imdb_id.is_empty())
    {
        external_ids.push(ExternalId::new("imdb", imdb_id));
    }
    external_ids
}

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
        title_folder_path: None,
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

/// Whether this pending import names a directory on disk right now. A path the
/// process cannot stat is reported as "not a directory" so the file-shaped
/// paths keep their existing behaviour when media storage is unavailable.
async fn pending_import_path_is_directory(item: &LibraryScanUnmatchedItem) -> bool {
    let item_path = item.item_path.trim();
    if item_path.is_empty() {
        return false;
    }
    tokio::fs::metadata(stored_path_to_path_buf(item_path))
        .await
        .is_ok_and(|metadata| metadata.is_dir())
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
        // Matched on the title's one absolute scale; see `AbsoluteScale`.
        let scale = scryer_domain::AbsoluteScale::for_catalog(available_episodes);
        if let Some(matched) = available_episodes
            .iter()
            .find(|candidate| scale.episode_absolute(candidate) == Some(absolute_episode))
        {
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

/// Whether this item names a folder-ownership problem a person has to settle:
/// either a scan found a title already owning some other folder, or a user
/// deliberately took a folder away from its owner (FR-007).
fn pending_import_item_is_folder_ownership_item(item: &LibraryScanUnmatchedItem) -> bool {
    item.reason_code
        == crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
        || item.reason_code
            == crate::library_scan_unmatched::LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER
}

fn pending_import_item_requires_action(item: &LibraryScanUnmatchedItem) -> bool {
    pending_import_item_is_folder_ownership_item(item)
        || !(item.facet == MediaFacet::Movie && item.title_id.is_some())
}

fn reject_folder_ownership_conflict_resolution(item: &LibraryScanUnmatchedItem) -> AppResult<()> {
    if pending_import_item_is_folder_ownership_item(item) {
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

    /// Release a title binding that can never be resolved file by file.
    ///
    /// Folder-level series and anime rows attached before folder binding
    /// existed still carry a title id, and every episode-binding call on them
    /// fails because the path is a directory. Dropping the stale binding puts
    /// the row back in the un-attached state so "Search & Match" can attach the
    /// folder again, this time through the folder-scanning path.
    async fn release_stale_directory_title_binding(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<AppError> {
        if item.title_id.is_some() {
            let mut released = item.clone();
            released.title_id = None;
            released.updated_at = Utc::now().to_rfc3339();
            self.services
                .library
                .library_scan_unmatched_items
                .upsert_library_scan_unmatched_item(&released)
                .await?;
        }

        Ok(AppError::Validation(
            "pending import path is a folder, not a file, so it cannot be bound to episodes; \
             its stale title link was cleared, so reload pending imports and attach the folder \
             to a title again"
                .into(),
        ))
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
        let by_library = self.pending_import_counts_by_library().await?;
        let mut counts = PendingImportCounts::default();
        for (library_id, library_counts) in by_library {
            if manageable.contains(&library_id) {
                counts.movie += library_counts.movie;
                counts.series += library_counts.series;
                counts.anime += library_counts.anime;
            }
        }
        Ok(counts)
    }

    /// The same counts, per library and before any actor is considered, so a
    /// caller that answers for many actors — the navigation badges — counts
    /// once and filters in memory.
    pub(crate) async fn pending_import_counts_by_library(
        &self,
    ) -> AppResult<HashMap<String, PendingImportCounts>> {
        let repository = self.services.library.library_scan_unmatched_items.clone();
        let mut by_library: HashMap<String, PendingImportCounts> = HashMap::new();
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
            for item in items
                .into_iter()
                .filter(pending_import_item_requires_action)
            {
                let counts = by_library.entry(item.library_id).or_default();
                match facet {
                    MediaFacet::Movie => counts.movie += 1,
                    MediaFacet::Series => counts.series += 1,
                    MediaFacet::Anime => counts.anime += 1,
                }
            }
        }
        Ok(by_library)
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
        // Keep list requests independent of media storage: unknown scan-time
        // sizes stay unknown rather than blocking the page on filesystem I/O.

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
                known_titles.insert(
                    title_id.to_string(),
                    (title.name, title.slug, title.folder_path),
                );
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

            if let Some((title_name, title_slug, title_folder_path)) = known_titles.get(title_id) {
                item.title_name = Some(title_name.clone());
                item.title_slug = title_slug.clone();
                item.title_folder_path = title_folder_path.clone();
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
        attach_to_existing_title: bool,
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

        let target_identities =
            pending_import_target_identities(&item.facet, &request.external_ids);
        if target_identities.is_empty() {
            return Err(AppError::Validation("a title identity is required".into()));
        }

        let mut existing_match = None;
        for (source, value) in &target_identities {
            existing_match = self
                .services
                .catalog
                .titles
                .find_by_external_id_in_library_and_facet(
                    &item.library_id,
                    item.facet.clone(),
                    source,
                    value,
                )
                .await?
                .filter(|title| title_carries_identity(title, &item.facet, source, value));
            if existing_match.is_some() {
                break;
            }
        }
        if let Some(existing_title) = existing_match {
            if !attach_to_existing_title {
                return Err(AppError::Validation(
                    "title already exists in this library".into(),
                ));
            }

            let (title, library_scan) = self
                .bind_pending_import_to_existing_title(actor, &item, &existing_title)
                .await?;
            return Ok(ResolvePendingImportResult {
                title,
                created: false,
                library_scan,
                metadata_hydration_state: AddTitleHydrationState::NotRequired,
            });
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
            // Someone else created the title between the check above and the
            // insert. The create-and-bind store call leaves the pending import
            // untouched whenever it reuses a title, so bind it here.
            if !attach_to_existing_title {
                return Err(AppError::Validation(
                    "title already exists in this library".into(),
                ));
            }

            let (title, library_scan) = self
                .bind_pending_import_to_existing_title(actor, &item, &outcome.title)
                .await?;
            return Ok(ResolvePendingImportResult {
                title,
                created: false,
                library_scan,
                metadata_hydration_state: outcome.metadata_hydration_state,
            });
        }

        // The create-and-bind store call stamps the new title onto the row.
        // For a folder-level series or anime row that leaves a title bound to
        // a directory, which no episode bind can use, so take the folder route
        // now: the new title claims the folder and each file becomes its own
        // title-bound row for episode binding once metadata arrives.
        if item.facet != MediaFacet::Movie && pending_import_path_is_directory(&item).await {
            let (title, library_scan) = self
                .bind_pending_import_to_existing_title(actor, &item, &outcome.title)
                .await?;
            return Ok(ResolvePendingImportResult {
                title,
                created: true,
                library_scan,
                metadata_hydration_state: outcome.metadata_hydration_state,
            });
        }

        Ok(ResolvePendingImportResult {
            title: outcome.title,
            created: true,
            library_scan: None,
            metadata_hydration_state: outcome.metadata_hydration_state,
        })
    }

    /// Bind an item to a title that already exists. Movies are scanned before
    /// their pending row is deleted so an attach cannot silently lose a file.
    /// A folder-level series or anime row names the series directory itself, so
    /// it is scanned the same way; only file-level series and anime rows keep a
    /// title-bound pending row for episode selection.
    async fn bind_pending_import_to_existing_title(
        &self,
        actor: &User,
        item: &LibraryScanUnmatchedItem,
        title: &Title,
    ) -> AppResult<(Title, Option<LibraryScanSummary>)> {
        if item.facet == MediaFacet::Movie {
            let mut title = title.clone();
            let item_path = stored_path_to_path_buf(item.item_path.trim());
            let metadata = tokio::fs::metadata(&item_path).await.map_err(|error| {
                AppError::Validation(format!("pending import path is unavailable: {error}"))
            })?;
            let discovered_files = if metadata.is_file() {
                vec![build_pending_import_library_file(item).await?]
            } else if metadata.is_dir() {
                self.services
                    .library
                    .library_scanner
                    .scan_library(path_to_stored_string(&item_path).as_str())
                    .await?
            } else {
                return Err(AppError::Validation(
                    "pending import path is not a file or directory".into(),
                ));
            };
            if discovered_files.is_empty() {
                return Err(AppError::Validation(
                    "pending import directory contains no files to attach".into(),
                ));
            }
            let entry_path = pending_import_movie_entry_path(item);
            if metadata.is_dir() || entry_path != item_path {
                refuse_library_root_as_title_folder(self, &entry_path).await?;
                crate::folder_ownership::claim_title_folder_if_missing(
                    self,
                    &mut title,
                    &entry_path,
                )
                .await?;
            } else if title.folder_path.is_some() {
                let parent = item_path.parent().ok_or_else(|| {
                    AppError::Validation("pending import path has no parent folder".into())
                })?;
                crate::folder_ownership::ensure_folder_available_to_title(self, &title, parent)
                    .await?;
            }
            let expected_paths = discovered_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let summary = self
                .scan_title_library_with_discovered_files(actor, title.clone(), discovered_files)
                .await?;
            let attached_paths = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await?
                .into_iter()
                .map(|media_file| media_file.file_path)
                .collect::<HashSet<_>>();
            if !expected_paths.is_subset(&attached_paths) {
                return Err(AppError::Validation(
                    "failed to attach pending import file to selected movie".into(),
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
            return Ok((title, Some(summary)));
        }

        // A folder-level series or anime row points at the series directory, so
        // there is no single file to bind to an episode. Attaching it does what
        // the movie branch does: scan the folder, claim it for the title, and
        // let the normal title scan match episodes. Files the scan cannot place
        // come back as title-bound file-level rows, so "Bind Episodes" still
        // works per file afterwards.
        if pending_import_path_is_directory(item).await {
            let mut title = title.clone();
            let item_path = stored_path_to_path_buf(item.item_path.trim());
            let discovered_files = self
                .services
                .library
                .library_scanner
                .scan_library(path_to_stored_string(&item_path).as_str())
                .await?;
            if discovered_files.is_empty() {
                return Err(AppError::Validation(
                    "pending import directory contains no files to attach".into(),
                ));
            }
            refuse_library_root_as_title_folder(self, &item_path).await?;
            crate::folder_ownership::claim_title_folder_if_missing(self, &mut title, &item_path)
                .await?;
            let summary = self
                .scan_title_library_with_discovered_files(actor, title.clone(), discovered_files)
                .await?;
            // The scan records its own file-level rows under the file paths, so
            // deleting the folder row by path leaves those untouched.
            self.services
                .library
                .library_scan_unmatched_items
                .delete_library_scan_unmatched_item(
                    &item.library_id,
                    item.facet.clone(),
                    &item.item_path,
                )
                .await?;
            return Ok((title, Some(summary)));
        }

        let mut bound = item.clone();
        bound.title_id = Some(title.id.clone());
        bound.status = PendingImportStatus::Pending;
        bound.updated_at = Utc::now().to_rfc3339();
        self.services
            .library
            .library_scan_unmatched_items
            .upsert_library_scan_unmatched_item(&bound)
            .await?;
        Ok((title.clone(), None))
    }

    pub async fn pending_import_title_search(
        &self,
        actor: &User,
        pending_import_id: &str,
        query: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<PendingImportTitleSearchItem>> {
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
        let gateway = &self.services.library.metadata_gateway;
        // The title surface finds every title, including a series SMG knows
        // only from TMDB, which has no TVDB id.
        let results = gateway
            .search_titles(query, item.facet.as_str(), search_limit, language, year)
            .await?;

        // Candidates the library already owns are annotated, not dropped: when
        // the only real candidate is already there, hiding it made the dialog
        // look like a search failure. The UI turns the annotation into an
        // "attach to existing title" action instead. A candidate is annotated
        // exactly when resolving it would find that title, so each is looked
        // up by the same identities, in the same order, as the resolver.
        let identities_by_result = results
            .iter()
            .map(|result| {
                pending_import_target_identities(&item.facet, &search_result_external_ids(result))
            })
            .collect::<Vec<_>>();
        let mut values_by_source: Vec<(&'static str, Vec<String>)> = Vec::new();
        for (source, value) in identities_by_result.iter().flatten() {
            match values_by_source
                .iter_mut()
                .find(|(known, _)| known == source)
            {
                Some((_, values)) => {
                    if !values.contains(value) {
                        values.push(value.clone());
                    }
                }
                None => values_by_source.push((source, vec![value.clone()])),
            }
        }
        let titles = &self.services.catalog.titles;
        let mut existing_title_ids: HashMap<(&'static str, String), String> = HashMap::new();
        for (source, values) in &values_by_source {
            let found = titles
                .map_existing_external_ids_to_title_ids_in_library_and_facet(
                    &item.library_id,
                    item.facet.clone(),
                    source,
                    values,
                )
                .await?;
            existing_title_ids.extend(
                found
                    .into_iter()
                    .map(|(value, title_id)| ((*source, value), title_id)),
            );
        }
        // The batch lookup matches source and value only, so a TMDB match is
        // kept only when the title carries the id in a kind that names this
        // facet, as the resolver requires.
        let tmdb_title_ids = existing_title_ids
            .iter()
            .filter(|((source, _), _)| *source == "tmdb")
            .map(|(_, title_id)| title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !tmdb_title_ids.is_empty() {
            let tmdb_titles = titles
                .get_by_ids(&tmdb_title_ids)
                .await?
                .into_iter()
                .map(|title| (title.id.clone(), title))
                .collect::<HashMap<_, _>>();
            existing_title_ids.retain(|(source, value), title_id| {
                *source != "tmdb"
                    || tmdb_titles.get(title_id).is_some_and(|title| {
                        title_carries_identity(title, &item.facet, source, value)
                    })
            });
        }

        let mut annotated = Vec::with_capacity(limit as usize);
        for (result, identities) in results.into_iter().zip(identities_by_result) {
            let existing_title_id = identities
                .into_iter()
                .find_map(|identity| existing_title_ids.get(&identity).cloned());

            annotated.push(PendingImportTitleSearchItem {
                item: result,
                existing_title_id,
            });
            if annotated.len() >= limit as usize {
                break;
            }
        }

        Ok(annotated)
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
        if pending_import_path_is_directory(&item).await {
            return Err(self.release_stale_directory_title_binding(&item).await?);
        }
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
        if pending_import_path_is_directory(&item).await {
            return Err(self.release_stale_directory_title_binding(&item).await?);
        }
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
            .analyze_catalogued_media_file(None, stored_path_to_path_buf(&file.path))
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
                // The episodes were picked by hand. Recording the source path
                // marks the row as placed on purpose, the way every import
                // does, so a later scan never replaces these links with the
                // ones the filename names.
                original_file_path: Some(item.item_path.clone()),
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

/// A pending import attaches everything in its folder to one title, so that
/// folder cannot be a library root.
async fn refuse_library_root_as_title_folder(
    app: &AppUseCase,
    folder_path: &std::path::Path,
) -> AppResult<()> {
    let folder_path = path_to_stored_string(folder_path);
    if crate::folder_ownership::folder_spans_a_library_root(app, &folder_path).await? {
        return Err(AppError::Validation(format!(
            "{} is a library root, not a title folder",
            crate::stored_paths::stored_path_to_display_string(&folder_path)
        )));
    }
    Ok(())
}
