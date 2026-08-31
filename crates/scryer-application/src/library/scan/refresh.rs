use super::*;
use crate::library_scan_helpers::require_directory_library_path;
use crate::stored_paths::path_to_stored_string;

async fn title_ready_for_background_refresh(app: &AppUseCase, title: &Title) -> AppResult<bool> {
    let metadata_language = app.resolve_metadata_language_for_title(title).await;
    if title.metadata_fetched_at.is_none()
        || title.metadata_language.as_deref() != Some(metadata_language.as_str())
    {
        return Ok(false);
    }

    let Some(handler) = app.facet_registry.get(&title.facet) else {
        return Ok(false);
    };
    if !handler.has_episodes() {
        return Ok(true);
    }

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await?;
    Ok(!episodes.is_empty())
}

fn movie_refresh_entry_to_library_file(entry: &MovieTopLevelEntry) -> LibraryFile {
    LibraryFile {
        path: path_to_stored_string(&entry.path),
        display_name: entry
            .path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        nfo_path: matching_movie_nfo_path(&entry.path),
        size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
    }
}

fn movie_refresh_entry_contains_path(entry: &MovieTopLevelEntry, path: &str) -> bool {
    let entry_path = path_to_stored_string(&entry.path);
    if entry.is_dir {
        path.starts_with(format!("{entry_path}/").as_str()) || path == entry_path
    } else {
        path == entry_path
    }
}

pub(super) async fn maybe_probe_existing_series_title_for_background_refresh(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    if !title_ready_for_background_refresh(app, title).await? {
        summary.skipped += 1;
        return Ok(());
    }

    let probe_outcome =
        run_background_refresh_probe_with_delta(app, &title.id, folder_path, async {
            let file_scan = app
                .services
                .library
                .library_scanner
                .scan_directory_for_progress_with_metrics(
                    path_to_stored_string(folder_path).as_str(),
                )
                .await?;
            let discovered_paths = file_scan
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let existing_paths = app
                .services
                .library
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
            executor.enqueue(super::scan_candidates::episodic_title_work(
                title.clone(),
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                false,
            ));
            summary.matched += 1;
        }
    }

    Ok(())
}

async fn maybe_probe_existing_movie_title_for_background_refresh(
    app: &AppUseCase,
    title: &Title,
    collections: &[Collection],
    entry: &MovieTopLevelEntry,
    executor: &mut dyn LibraryScanTitleWorkQueue,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    if !title_ready_for_background_refresh(app, title).await? {
        summary.skipped += 1;
        return Ok(());
    }

    let probe_outcome =
        run_background_refresh_probe_with_delta(app, &title.id, &entry.path, async {
            let discovered_files = if entry.is_dir {
                app.services
                    .library
                    .library_scanner
                    .scan_directory_for_progress_with_metrics(
                        path_to_stored_string(&entry.path).as_str(),
                    )
                    .await?
                    .files
            } else {
                vec![movie_refresh_entry_to_library_file(entry)]
            };

            let discovered_paths = discovered_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let existing_paths = collections
                .iter()
                .filter_map(|collection| collection.ordered_path.clone())
                .filter(|path| movie_refresh_entry_contains_path(entry, path))
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
            let scan_folder_path = Some(path_to_stored_string(&entry.path));
            let mut cleanup = LibraryScanMovieCleanupContext {
                canonical_folder_path: title
                    .folder_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| scan_folder_path.clone()),
                scan_folder_path,
                ..Default::default()
            };
            for collection in collections {
                let Some(ordered_path) = collection.ordered_path.as_deref() else {
                    continue;
                };
                if movie_refresh_entry_contains_path(entry, ordered_path)
                    && !discovered_paths.contains(ordered_path)
                {
                    cleanup.stale_collection_ids.push(collection.id.clone());
                }
            }

            executor.enqueue(super::scan_candidates::movie_title_work(
                title.clone(),
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                cleanup,
                false,
            ));
            summary.matched += 1;
        }
    }

    Ok(())
}

async fn load_titles_for_background_refresh(
    app: &AppUseCase,
    facet: MediaFacet,
    library_ids: &[String],
) -> AppResult<Vec<Title>> {
    let mut existing_titles = app
        .services
        .catalog
        .titles
        .list_for_libraries(Some(facet.clone()), library_ids, None)
        .await?;
    let forced_metadata_refresh_ids = app
        .services
        .catalog
        .titles
        .list_title_ids_with_metadata_hydration_due(Some(facet), library_ids)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    super::scan_metadata_refresh::refresh_titles_metadata_for_scan_policy(
        app,
        &mut existing_titles,
        &forced_metadata_refresh_ids,
        super::scan_metadata_refresh::LibraryScanMetadataRefreshMode::BackgroundRefresh,
    )
    .await?;
    super::scan_metadata_refresh::queue_title_recommendations_for_background_refresh(
        app,
        &existing_titles,
    )
    .await;
    Ok(existing_titles)
}

pub(super) async fn background_refresh_series(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;

    let folders = list_child_directories(root).await?;
    coordinator
        .register_discovery_batch(folders.len(), false)
        .await;
    coordinator.publish_progress().await;

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let pool_policy =
        LibraryScanMediaAnalysisPolicy::background_refresh(app, session_id, None).await;
    let mut executor = LibraryScanMediaAnalysisPool::for_policy(app, actor, pool_policy).await?;
    let metadata_language = app.metadata_language().await;

    let library_ids = vec![library_id.to_string()];
    let mut existing_titles =
        load_titles_for_background_refresh(app, facet.clone(), &library_ids).await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_series_title_indexes(&existing_titles);
    let mut existing_titles_by_folder_path = build_series_title_folder_path_index(&existing_titles);

    let mut unknown_folders = Vec::new();
    for folder in folders {
        summary.scanned += 1;
        let folder_key = path_to_stored_string(&folder);
        let owner_index = crate::stored_paths::folder_path_identity_key(&folder_key)
            .and_then(|key| existing_titles_by_folder_path.get(&key).copied());
        if let Some(index) = owner_index {
            let title = &mut existing_titles[index];
            maybe_probe_existing_series_title_for_background_refresh(
                app,
                title,
                &folder,
                &mut executor,
                &mut summary,
            )
            .await?;
        } else {
            unknown_folders.push(folder);
        }
        coordinator.mark_title_match_completed(1).await;
        executor.pump().await?;
    }

    for folder_batch in unknown_folders.chunks(LIBRARY_SCAN_SERIES_BATCH_SIZE) {
        let prepared_candidates =
            prepare_series_library_scan_candidates(folder_batch, None).await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_candidates {
            let candidate = process_series_refresh_candidate(
                app,
                facet,
                library_id,
                library_path,
                session_id,
                candidate,
                &mut executor,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut existing_titles_by_folder_path,
                &mut summary,
            )
            .await?;
            if let Some(candidate) = candidate {
                unresolved_candidates.push(candidate);
            } else {
                coordinator.mark_title_match_completed(1).await;
            }
        }
        executor.pump().await?;

        let (ready_candidate_batches, batch_search_results) = resolve_refresh_metadata_batches(
            app.services.library.metadata_gateway.clone(),
            &metadata_language,
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_series_metadata_batch_stats,
            series_candidate_batch_search_keys,
            "background series metadata search chunk unexpectedly empty",
            None,
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_series_refresh_candidate(
                    app,
                    actor,
                    facet,
                    library_id,
                    library_path,
                    session_id,
                    candidate,
                    &batch_search_results,
                    &mut executor,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_tvdb_id,
                    &mut existing_titles_by_imdb_id,
                    &mut existing_titles_by_tmdb_id,
                    &mut existing_titles_by_folder_path,
                    &mut summary,
                )
                .await?;
                coordinator.mark_title_match_completed(1).await;
            }

            coordinator.publish_progress().await;
            executor.pump().await?;
        }
    }

    executor.close_input();
    summary.absorb(&executor.finish().await?);
    coordinator.publish_progress().await;

    debug!(
        path = %library_path,
        facet = facet.as_str(),
        scanned = summary.scanned,
        imported = summary.imported,
        matched = summary.matched,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        elapsed_ms = elapsed_ms_u64(started_at),
        "background library refresh completed"
    );

    Ok(summary)
}

pub(super) async fn background_refresh_movies(
    app: &AppUseCase,
    actor: &User,
    library_id: &str,
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;

    let entries = list_movie_top_level_entries(root).await?;
    coordinator
        .register_discovery_batch(entries.len(), false)
        .await;
    coordinator.publish_progress().await;

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let pool_policy =
        LibraryScanMediaAnalysisPolicy::background_refresh(app, session_id, None).await;
    let mut executor = LibraryScanMediaAnalysisPool::for_policy(app, actor, pool_policy).await?;
    let library_ids = vec![library_id.to_string()];
    let mut existing_titles =
        load_titles_for_background_refresh(app, MediaFacet::Movie, &library_ids).await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_smg_id,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_movie_title_indexes(&existing_titles);
    let existing_title_ids = existing_titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let collections_by_title = app
        .services
        .catalog
        .shows
        .list_collections_for_titles(&existing_title_ids)
        .await
        .unwrap_or_default();

    let mut existing_titles_by_probe_path =
        build_movie_probe_path_indexes(root, &existing_titles, &collections_by_title);

    let mut unknown_entries = Vec::new();
    let metadata_language = app.metadata_language().await;
    for entry in entries {
        summary.scanned += 1;
        let owner_index =
            crate::stored_paths::folder_path_identity_key(&path_to_stored_string(&entry.path))
                .and_then(|key| existing_titles_by_probe_path.get(&key).copied());
        if let Some(index) = owner_index {
            let title = &existing_titles[index];
            let collections = collections_by_title
                .get(&title.id)
                .cloned()
                .unwrap_or_default();
            maybe_probe_existing_movie_title_for_background_refresh(
                app,
                title,
                &collections,
                &entry,
                &mut executor,
                &mut summary,
            )
            .await?;
        } else {
            unknown_entries.push(entry);
        }
        coordinator.mark_title_match_completed(1).await;
        executor.pump().await?;
    }

    for entry_chunk in unknown_entries.chunks(LIBRARY_SCAN_MOVIE_BATCH_SIZE) {
        let prepared_entries = prepare_movie_library_scan_entries(
            app.services.library.library_scanner.clone(),
            entry_chunk,
            library_path,
            None,
        )
        .await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_entries {
            let candidate = process_movie_refresh_candidate(
                app,
                actor,
                library_id,
                candidate,
                &mut executor,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                root,
                &mut existing_titles_by_probe_path,
                &mut summary,
            )
            .await?;
            if let Some(candidate) = candidate {
                unresolved_candidates.push(candidate);
            } else {
                coordinator.mark_title_match_completed(1).await;
            }
        }
        executor.pump().await?;

        let (ready_candidate_batches, batch_search_results) = resolve_refresh_metadata_batches(
            app.services.library.metadata_gateway.clone(),
            &metadata_language,
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_movie_metadata_batch_stats,
            movie_candidate_batch_search_keys,
            "background movie metadata search chunk unexpectedly empty",
            None,
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_movie_refresh_candidate(
                    app,
                    actor,
                    library_id,
                    candidate,
                    &batch_search_results,
                    &mut executor,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_smg_id,
                    &mut existing_titles_by_tvdb_id,
                    &mut existing_titles_by_imdb_id,
                    &mut existing_titles_by_tmdb_id,
                    root,
                    &mut existing_titles_by_probe_path,
                    &mut summary,
                )
                .await?;
                coordinator.mark_title_match_completed(1).await;
            }

            coordinator.publish_progress().await;
            executor.pump().await?;
        }
    }

    executor.close_input();
    summary.absorb(&executor.finish().await?);
    coordinator.publish_progress().await;

    debug!(
        path = %library_path,
        scanned = summary.scanned,
        imported = summary.imported,
        matched = summary.matched,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        elapsed_ms = elapsed_ms_u64(started_at),
        "background movie refresh completed"
    );

    Ok(summary)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::list_child_directories;

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
}
