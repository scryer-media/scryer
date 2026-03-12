use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::*;
use crate::nfo::parse_nfo;
use tracing::{info, warn};

const METADATA_TYPE_MOVIE: &str = "movie";
const RENAME_TEMPLATE_KEY: &str = "rename.template";
const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const DEFAULT_COLLISION_POLICY: RenameCollisionPolicy = RenameCollisionPolicy::Skip;
const DEFAULT_MISSING_METADATA_POLICY: RenameMissingMetadataPolicy =
    RenameMissingMetadataPolicy::FallbackTitle;

/// Extracts the movie title and year hint from a library file.
///
/// Strategy (in priority order):
/// 1. If the file lives in a sub-folder relative to the library root, use the
///    immediate parent folder name as the title — it is the canonical movie name
///    in any Plex/Kodi-style layout (e.g. `Movies/300/file.mkv` → `"300"`).
///    The year is extracted from the folder name if present (e.g. `"A Quiet Place
///    Day One (2024)"` → year 2024, query `"A Quiet Place Day One"`).
/// 2. Fall back to parsing the file stem when the file is at the root level.
fn extract_library_query(path: &str, library_root: &str) -> (String, Option<u32>) {
    // Normalise paths for comparison (strip trailing slash)
    let root = library_root.trim_end_matches('/');

    // Attempt to get the immediate parent directory of the file.
    if let Some(parent) = Path::new(path).parent() {
        let parent_str = parent.to_string_lossy();
        // Only use parent folder when it is NOT the library root itself
        // (i.e. the file is inside a sub-folder).
        if parent_str.trim_end_matches('/') != root {
            if let Some(folder_name) = parent.file_name().and_then(|n| n.to_str()) {
                let clean = normalize_folder_name(folder_name);
                let (title, year) = strip_year_suffix(&clean);
                if !title.trim().is_empty() {
                    return (title.trim().to_string(), year);
                }
            }
        }
    }

    // Fallback: parse the file stem via the release parser.
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(stem);
    (parsed.normalized_title.clone(), parsed.year)
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
        if let Some(close_pos) = folder.rfind(close) {
            if let Some(open_pos) = folder[..close_pos].rfind(open) {
                let candidate = &folder[open_pos + 1..close_pos];
                if let Ok(year) = candidate.trim().parse::<u32>() {
                    if (1888..=2100).contains(&year) {
                        let title = folder[..open_pos].trim_end().to_string();
                        if !title.is_empty() {
                            return (title, Some(year));
                        }
                    }
                }
            }
        }
    }
    (folder.to_string(), None)
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
                    {
                        if let Err(err) = self
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
            MediaFacet::Tv => "series.path",
            MediaFacet::Anime => "anime.path",
            MediaFacet::Other => "series.path",
        };

        let Some(library_path) = self
            .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, path_key, None)
            .await?
        else {
            return Err(AppError::Validation(format!(
                "{path_key} is not configured"
            )));
        };

        let files = self
            .services
            .library_scanner
            .scan_library(&library_path)
            .await?;
        let existing_titles = self.services.titles.list(Some(facet.clone()), None).await?;
        let mut existing_titles_by_name: HashMap<String, Title> = HashMap::new();
        let mut existing_titles_by_tvdb_id: HashMap<String, Title> = HashMap::new();

        for title in &existing_titles {
            existing_titles_by_name.insert(normalize_title_key(&title.name), title.clone());
            for external_id in &title.external_ids {
                if external_id.source.eq_ignore_ascii_case("tvdb") {
                    existing_titles_by_tvdb_id.insert(external_id.value.clone(), title.clone());
                }
            }
        }

        let mut summary = LibraryScanSummary::default();

        for file in files {
            summary.scanned += 1;

            // Parse companion NFO sidecar if present (non-fatal).
            let nfo_meta = file
                .nfo_path
                .as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|content| parse_nfo(&content));

            // --- Fast path: NFO provides a TVDB ID, skip gateway search ---
            if let Some(tvdb_id) = nfo_meta.as_ref().and_then(|m| m.tvdb_id.as_deref()) {
                let title = if let Some(existing) = existing_titles_by_tvdb_id.get(tvdb_id).cloned()
                {
                    existing
                } else {
                    let (fallback_query, _) = extract_library_query(&file.path, &library_path);
                    let name = nfo_meta
                        .as_ref()
                        .and_then(|m| m.title.clone())
                        .unwrap_or(fallback_query);

                    let mut external_ids = vec![ExternalId {
                        source: "tvdb".into(),
                        value: tvdb_id.to_string(),
                    }];
                    if let Some(ref imdb) = nfo_meta.as_ref().and_then(|m| m.imdb_id.clone()) {
                        external_ids.push(ExternalId {
                            source: "imdb".into(),
                            value: imdb.clone(),
                        });
                    }
                    if let Some(ref tmdb) = nfo_meta.as_ref().and_then(|m| m.tmdb_id.clone()) {
                        external_ids.push(ExternalId {
                            source: "tmdb".into(),
                            value: tmdb.clone(),
                        });
                    }

                    let new_title = NewTitle {
                        name,
                        facet: facet.clone(),
                        monitored: true,
                        tags: vec![],
                        external_ids,
                        min_availability: None,
                        ..Default::default()
                    };

                    let created = self.add_title(actor, new_title).await?;
                    let key = normalize_title_key(&created.name);
                    existing_titles_by_name.insert(key, created.clone());
                    existing_titles_by_tvdb_id.insert(tvdb_id.to_string(), created.clone());
                    created
                };

                summary.matched += 1;
                self.track_movie_file_in_collection(&title, &file, &mut summary)
                    .await;
                continue;
            }

            // --- Normal path: search metadata gateway ---
            // Use NFO title/year if available, otherwise folder/filename heuristics.
            let (query, year_hint) = if let Some(ref meta) = nfo_meta {
                let title_str = meta
                    .title
                    .clone()
                    .unwrap_or_else(|| extract_library_query(&file.path, &library_path).0);
                let year = meta
                    .year
                    .map(|y| y as u32)
                    .or_else(|| extract_library_query(&file.path, &library_path).1);
                (title_str, year)
            } else {
                extract_library_query(&file.path, &library_path)
            };

            if query.is_empty() {
                summary.skipped += 1;
                continue;
            }

            let metadata_type = match facet {
                MediaFacet::Movie => METADATA_TYPE_MOVIE,
                _ => "series",
            };
            let results = self
                .services
                .metadata_gateway
                .search_tvdb(&query, metadata_type)
                .await?;

            let Some(selected) = select_best_match(&results, year_hint) else {
                summary.unmatched += 1;
                continue;
            };

            summary.matched += 1;

            let key = normalize_title_key(&selected.name);
            let title = existing_titles_by_tvdb_id
                .get(&selected.tvdb_id)
                .cloned()
                .or_else(|| existing_titles_by_name.get(&key).cloned());

            let title = if let Some(existing_title) = title {
                existing_title
            } else {
                let new_title = NewTitle {
                    name: selected.name.clone(),
                    facet: facet.clone(),
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![ExternalId {
                        source: "tvdb".into(),
                        value: selected.tvdb_id.clone(),
                    }],
                    min_availability: None,
                    ..Default::default()
                };

                let title = self.add_title(actor, new_title).await?;
                existing_titles_by_name.insert(key, title.clone());
                existing_titles_by_tvdb_id.insert(selected.tvdb_id.clone(), title.clone());
                title
            };

            self.track_movie_file_in_collection(&title, &file, &mut summary)
                .await;
        }

        info!(
            path = %library_path,
            scanned = summary.scanned,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            "library scan completed"
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

        let handler = self.facet_registry.get(&title.facet).ok_or_else(|| {
            AppError::Validation("library scan is not supported for this facet".into())
        })?;
        if !handler.has_episodes() {
            return Err(AppError::Validation(
                "title library scan is only supported for episodic titles".into(),
            ));
        }

        let (media_root, _) = crate::app_usecase_import::resolve_import_paths(self, &title).await?;
        let title_dir = PathBuf::from(&media_root).join(&title.name);
        let title_dir_str = title_dir.to_string_lossy().to_string();

        let files = self
            .services
            .library_scanner
            .scan_directory(&title_dir_str)
            .await?;

        let existing_files = self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();

        let mut file_ids_by_path: HashMap<String, String> = HashMap::new();
        let mut existing_records_by_path: HashMap<String, TitleMediaFile> = HashMap::new();
        let mut episode_links: HashSet<(String, String)> = HashSet::new();

        for file in &existing_files {
            file_ids_by_path
                .entry(file.file_path.clone())
                .or_insert_with(|| file.id.clone());
            existing_records_by_path
                .entry(file.file_path.clone())
                .or_insert_with(|| file.clone());
            if let Some(episode_id) = file.episode_id.as_ref() {
                episode_links.insert((file.id.clone(), episode_id.clone()));
            }
        }

        let scanned_paths: HashSet<String> = files.iter().map(|file| file.path.clone()).collect();
        let stale_paths: Vec<String> = existing_records_by_path
            .iter()
            .filter(|(path, record)| {
                path.starts_with(title_dir_str.as_str())
                    && !scanned_paths.contains(*path)
                    && !Path::new(&record.file_path).exists()
            })
            .map(|(path, _)| path.clone())
            .collect();

        for stale_path in stale_paths {
            if let Some(record) = existing_records_by_path.remove(&stale_path) {
                file_ids_by_path.remove(&stale_path);
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
        }

        let mut summary = LibraryScanSummary::default();

        for file in files {
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
            let target_episodes = crate::app_usecase_import::resolve_target_episodes(
                self,
                &title,
                ep_meta,
                &season_str,
            )
            .await;

            if target_episodes.is_empty() {
                summary.unmatched += 1;
                continue;
            }

            summary.matched += 1;

            let file_id = if let Some(existing_id) = file_ids_by_path.get(&file.path).cloned() {
                summary.skipped += 1;
                existing_id
            } else {
                let size_bytes = std::fs::metadata(source_path)
                    .map(|meta| meta.len() as i64)
                    .unwrap_or(0);
                let media_file_input = crate::InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: file.path.clone(),
                    size_bytes,
                    quality_label: parsed.quality.clone(),
                    scene_name: Some(parsed.raw_title.clone()),
                    release_group: parsed.release_group.clone(),
                    source_type: parsed.source.clone(),
                    resolution: parsed.quality.clone(),
                    video_codec_parsed: parsed.video_codec.clone(),
                    audio_codec_parsed: parsed.audio.clone(),
                    ..Default::default()
                };

                match self
                    .services
                    .media_files
                    .insert_media_file(&media_file_input)
                    .await
                {
                    Ok(file_id) => {
                        file_ids_by_path.insert(file.path.clone(), file_id.clone());
                        summary.imported += 1;
                        file_id
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file.path,
                            "failed to insert media file during title scan"
                        );
                        summary.skipped += 1;
                        continue;
                    }
                }
            };

            for episode in &target_episodes {
                if episode_links.insert((file_id.clone(), episode.id.clone())) {
                    if let Err(error) = self
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
                }
                crate::app_usecase_import::mark_wanted_completed(
                    self,
                    &title.id,
                    Some(&episode.id),
                    None,
                )
                .await;
            }

            match scryer_mediainfo::analyze_file(source_path) {
                Ok(analysis) if scryer_mediainfo::is_valid_video(&analysis) => {
                    if let Err(error) = self
                        .services
                        .media_files
                        .update_media_file_analysis(
                            &file_id,
                            crate::post_download_gate::build_media_file_analysis(&analysis),
                        )
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
                Ok(_) => {
                    if let Err(error) = self
                        .services
                        .media_files
                        .mark_scan_failed(&file_id, "file is not a valid video")
                        .await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_id = %file_id,
                            "failed to mark invalid scanned media file"
                        );
                    }
                }
                Err(error) => {
                    if let Err(mark_error) = self
                        .services
                        .media_files
                        .mark_scan_failed(&file_id, &error.to_string())
                        .await
                    {
                        warn!(
                            error = %mark_error,
                            title_id = %title.id,
                            file_id = %file_id,
                            "failed to mark scanned media analysis failure"
                        );
                    }
                }
            }
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
            collection_type: "movie".to_string(),
            collection_index: next_collection_index.to_string(),
            label: quality_label,
            ordered_path: Some(file.path.clone()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            monitored: true,
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
        if let Some(value) = scoped {
            if let Some(policy) = parse_collision_policy(&value) {
                return Ok(policy);
            }
        }

        let global = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_GLOBAL_KEY, None)
            .await?;
        if let Some(value) = global {
            if let Some(policy) = parse_collision_policy(&value) {
                return Ok(policy);
            }
        }

        let global = self
            .read_setting_string_value(handler.collision_policy_key(), None)
            .await?;
        if let Some(value) = global {
            if let Some(policy) = parse_collision_policy(&value) {
                return Ok(policy);
            }
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
        if let Some(value) = scoped {
            if let Some(policy) = parse_missing_metadata_policy(&value) {
                return Ok(policy);
            }
        }

        let global = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, None)
            .await?;
        if let Some(value) = global {
            if let Some(policy) = parse_missing_metadata_policy(&value) {
                return Ok(policy);
            }
        }

        let global = self
            .read_setting_string_value(handler.missing_metadata_policy_key(), None)
            .await?;
        if let Some(value) = global {
            if let Some(policy) = parse_missing_metadata_policy(&value) {
                return Ok(policy);
            }
        }

        Ok(DEFAULT_MISSING_METADATA_POLICY)
    }
}

fn select_best_match(
    results: &[MetadataSearchItem],
    year: Option<u32>,
) -> Option<MetadataSearchItem> {
    if results.is_empty() {
        return None;
    }

    if let Some(year) = year.map(|value| value as i32) {
        if let Some(match_item) = results.iter().find(|item| item.year == Some(year)) {
            return Some(match_item.clone());
        }
    }

    Some(results[0].clone())
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
    tokens.insert("episode_title".to_string(), String::new());
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
        if let Some(close_pos) = trimmed.rfind(close) {
            if let Some(open_pos) = trimmed[..close_pos].rfind(open) {
                let candidate = trimmed[open_pos + 1..close_pos].trim();
                if candidate.len() == 4 && candidate.chars().all(|value| value.is_ascii_digit()) {
                    let title = trimmed[..open_pos].trim().to_string();
                    if !title.is_empty() {
                        return (title, Some(candidate.to_string()));
                    }
                }
            }
        }
    }

    (trimmed.to_string(), None)
}
