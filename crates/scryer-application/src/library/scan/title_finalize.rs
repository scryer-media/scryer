use super::*;
use crate::library_scan_unmatched::{
    IgnoredLibraryScanItemArgs, LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE,
    persist_ignored_library_scan_item,
};
use crate::stored_paths::stored_path_to_path_buf;

struct ExistingScannedMediaFile<'a> {
    file_id: &'a str,
    should_skip_analysis: bool,
    should_refresh_source_signature: bool,
    /// FR-046: the sampled quick proof changed, so the persisted full hashes
    /// describe bytes that no longer exist.
    should_invalidate_full_hashes: bool,
}

struct PersistedScannedMediaFile {
    file_id: String,
    should_analyze: bool,
    title_updated: bool,
    db_elapsed: Duration,
}

#[expect(
    clippy::too_many_arguments,
    reason = "media-file persistence combines source metadata, cache state, and summary accounting"
)]
async fn persist_or_reuse_scanned_media_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    parsed: &crate::ParsedReleaseMetadata,
    snapshot: &FileSourceSnapshot,
    existing: Option<ExistingScannedMediaFile<'_>>,
    summary: &mut LibraryScanSummary,
    update_error_message: &'static str,
    insert_error_message: &'static str,
) -> Option<PersistedScannedMediaFile> {
    let source_signature_scheme = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.scheme.clone());
    let source_signature_value = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.value.clone());

    if let Some(existing) = existing {
        let mut db_elapsed = Duration::default();

        // FR-046: a changed quick proof throws away the persisted full hashes
        // before anything else, so a crash between here and the signature
        // refresh leaves the file queued for backfill rather than carrying a
        // hash of content it no longer holds. The scan itself never computes a
        // full hash; it only ever clears one.
        if existing.should_invalidate_full_hashes {
            let db_started = Instant::now();
            let invalidated = app
                .services
                .library
                .media_files
                .clear_media_file_content_hashes(existing.file_id)
                .await;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            match invalidated {
                Ok(true) => tracing::info!(
                    title_id = %title.id,
                    file_id = %existing.file_id,
                    "sampled content proof changed; cleared persisted full hashes and re-queued the file for backfill"
                ),
                Ok(false) => {}
                Err(error) => warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %existing.file_id,
                    "failed to invalidate persisted full hashes after a changed content proof"
                ),
            }
        }

        if existing.should_refresh_source_signature {
            let db_started = Instant::now();
            let update_result = app
                .services
                .library
                .media_files
                .update_media_file_source_signature(
                    existing.file_id,
                    snapshot.size_bytes,
                    source_signature_scheme.clone(),
                    source_signature_value.clone(),
                )
                .await;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            if let Err(error) = update_result {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %existing.file_id,
                    "{update_error_message}"
                );
            }
        }

        return Some(PersistedScannedMediaFile {
            file_id: existing.file_id.to_string(),
            should_analyze: !existing.should_skip_analysis,
            title_updated: false,
            db_elapsed,
        });
    }

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: file.path.clone(),
        size_bytes: snapshot.size_bytes,
        role: crate::MediaFileRole::Primary,
        source_signature_scheme,
        source_signature_value,
        quality_label: None,
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: crate::release_parser::parsed_release_source_type(parsed),
        resolution: None,
        video_codec_parsed: None,
        audio_codec_parsed: None,
        audio_channels_parsed: None,
        ..Default::default()
    };

    let db_started = Instant::now();
    let insert_result = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await;
    let db_elapsed = db_started.elapsed();

    match insert_result {
        Ok(file_id) => {
            summary.imported += 1;
            Some(PersistedScannedMediaFile {
                file_id,
                should_analyze: true,
                title_updated: true,
                db_elapsed,
            })
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_path = %file.path,
                "{insert_error_message}"
            );
            summary.skipped += 1;
            None
        }
    }
}

async fn persist_scanned_media_analysis_outcome(
    app: &AppUseCase,
    title: &Title,
    file_id: &str,
    outcome: MediaAnalysisOutcome,
) -> (Duration, bool) {
    let db_started = Instant::now();

    let persisted = match outcome {
        MediaAnalysisOutcome::Valid(analysis) => {
            let update_result = app
                .services
                .library
                .media_files
                .update_media_file_analysis(file_id, *analysis)
                .await;
            match update_result {
                Ok(()) => true,
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_id = %file_id,
                        "failed to persist scanned media analysis"
                    );
                    false
                }
            }
        }
        MediaAnalysisOutcome::Invalid(error_message) => {
            let mark_result = app
                .services
                .library
                .media_files
                .mark_scan_failed(file_id, &error_message)
                .await;
            match mark_result {
                Ok(()) => true,
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_id = %file_id,
                        "failed to mark scanned media analysis failure"
                    );
                    false
                }
            }
        }
    };

    (db_started.elapsed(), persisted)
}

fn scanned_media_analysis_status(outcome: &MediaAnalysisOutcome) -> &'static str {
    match outcome {
        MediaAnalysisOutcome::Valid(_) => "scanned",
        MediaAnalysisOutcome::Invalid(_) => "failed",
    }
}

async fn emit_scanned_media_file_analyzed_event(
    app: &AppUseCase,
    title: &Title,
    file_id: &str,
    file_path: &str,
    analysis_status: &str,
    episode_ids: Vec<String>,
) {
    let event = crate::domain_events::new_title_domain_event(
        None,
        title,
        scryer_domain::DomainEventPayload::MediaFileAnalyzed(
            scryer_domain::MediaFileAnalyzedEventData {
                title: crate::domain_events::title_context_snapshot(title),
                media_updates: vec![crate::domain_events::modified_media_update(file_path)],
                file_id: file_id.to_string(),
                analysis_status: analysis_status.to_string(),
                episode_ids,
            },
        ),
    );

    if let Err(error) = app.append_domain_event(event).await {
        warn!(
            error = %error,
            title_id = %title.id,
            file_id = %file_id,
            "failed to append scanned media file analyzed domain event"
        );
    }
}

async fn ensure_movie_collection_for_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    parsed: &crate::ParsedReleaseMetadata,
    collections: &[Collection],
) -> bool {
    let already_tracked = collections.iter().any(|collection| {
        collection
            .ordered_path
            .as_deref()
            .is_some_and(|path| path == file.path)
    });

    if already_tracked {
        return false;
    }

    let next_collection_index = collections
        .iter()
        .filter_map(|collection| collection.collection_index.parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
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
        monitored: title.monitored,
        created_at: Utc::now(),
    };

    if let Err(err) = app
        .services
        .catalog
        .shows
        .create_collection(collection)
        .await
    {
        debug!(
            title_id = %title.id,
            path = %file.path,
            error = %err,
            "failed to create collection for library file"
        );
        false
    } else {
        true
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "title-scan finalization coordinates persistence, linking, and summary accounting together"
)]
pub(crate) async fn finalize_title_scan_file(
    app: &AppUseCase,
    title: &Title,
    plan: PlannedTitleScanFile,
    analysis_outcome: Option<MediaAnalysisOutcome>,
    _scan_mode: LibraryScanMode,
    episode_links: &mut HashSet<(String, String)>,
    summary: &mut LibraryScanSummary,
    db_elapsed: &mut Duration,
    external_subtitle_cache: &mut crate::subtitles::ExternalSubtitleDirectoryCache,
) -> TitleScanFinalizeOutcome {
    let PlannedTitleScanFile {
        file,
        parsed,
        target_episodes,
        series_movie_link_id,
        snapshot,
        record,
    } = plan;

    let existing = match &record {
        PlannedTitleScanRecord::Existing {
            file_id,
            should_skip_analysis,
            should_refresh_source_signature,
            should_invalidate_full_hashes,
        } => Some(ExistingScannedMediaFile {
            file_id,
            should_skip_analysis: *should_skip_analysis,
            should_refresh_source_signature: *should_refresh_source_signature,
            should_invalidate_full_hashes: *should_invalidate_full_hashes,
        }),
        PlannedTitleScanRecord::New => None,
    };

    let destination_path = stored_path_to_path_buf(&file.path);
    let destination_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_destination(&destination_path)
        .await;

    let Some(persisted_file) = persist_or_reuse_scanned_media_file(
        app,
        title,
        &file,
        &parsed,
        &snapshot,
        existing,
        summary,
        "failed to refresh media file source signature during title scan",
        "failed to insert media file during title scan",
    )
    .await
    else {
        return TitleScanFinalizeOutcome {
            progress: TitleScanProgressDelta::failed(1),
            title_updated: false,
        };
    };
    *db_elapsed = db_elapsed.saturating_add(persisted_file.db_elapsed);

    let mut title_updated = persisted_file.title_updated;
    let external_subtitle_episode_id = match target_episodes.as_slice() {
        [episode] => Some(episode.id.as_str()),
        _ => None,
    };

    for episode in &target_episodes {
        if episode_links.insert((persisted_file.file_id.clone(), episode.id.clone())) {
            title_updated = true;
            let db_started = Instant::now();
            let link_result = app
                .services
                .library
                .media_files
                .link_file_to_episode(&persisted_file.file_id, &episode.id)
                .await;
            *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            if let Err(error) = link_result {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    episode_id = %episode.id,
                    file_id = %persisted_file.file_id,
                    "failed to link scanned file to episode"
                );
            }
        }
        crate::import_workflow::mark_wanted_completed(app, &title.id, Some(&episode.id), false)
            .await;
    }

    if let Some(series_movie_link_id) = series_movie_link_id.as_deref() {
        title_updated = true;
        let db_started = Instant::now();
        let link_result = app
            .services
            .library
            .media_files
            .link_file_to_series_movie(&persisted_file.file_id, series_movie_link_id)
            .await;
        *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        if let Err(error) = link_result {
            warn!(
                error = %error,
                title_id = %title.id,
                series_movie_link_id,
                file_id = %persisted_file.file_id,
                "failed to link scanned file to series movie"
            );
        }
    }
    drop(destination_permit);

    let file_path = destination_path;
    match crate::subtitles::reconcile_external_subtitles_for_media_file_with_cache(
        app,
        &title.id,
        &persisted_file.file_id,
        external_subtitle_episode_id,
        file_path.as_path(),
        external_subtitle_cache,
    )
    .await
    {
        Ok(changed) => {
            if changed {
                title_updated = true;
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_id = %persisted_file.file_id,
                file_path = %file.path,
                "failed to reconcile external subtitles during title scan"
            );
        }
    }

    if let Some(outcome) = analysis_outcome {
        let analysis_status = scanned_media_analysis_status(&outcome);
        let (analysis_db_elapsed, analysis_persisted) =
            persist_scanned_media_analysis_outcome(app, title, &persisted_file.file_id, outcome)
                .await;
        *db_elapsed = db_elapsed.saturating_add(analysis_db_elapsed);
        if analysis_persisted {
            emit_scanned_media_file_analyzed_event(
                app,
                title,
                &persisted_file.file_id,
                &file.path,
                analysis_status,
                target_episodes
                    .iter()
                    .map(|episode| episode.id.clone())
                    .collect(),
            )
            .await;
        }
    }

    TitleScanFinalizeOutcome {
        progress: TitleScanProgressDelta::completed(1),
        title_updated,
    }
}

async fn persist_ignored_movie_scan_file_metadata_error(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    session_id: Option<&str>,
    title_scan_root: &str,
    error_message: String,
) {
    let file_path = stored_path_to_path_buf(&file.path);
    let display_name = file.display_name.trim();
    let display_name = if display_name.is_empty() {
        file_path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.path.clone())
    } else {
        display_name.to_string()
    };
    let fallback_library_path;
    let library_path = if title_scan_root.trim().is_empty() {
        fallback_library_path = file_path
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        fallback_library_path.as_str()
    } else {
        title_scan_root
    };

    if let Err(error) = persist_ignored_library_scan_item(
        app,
        &title.facet,
        &title.library_id,
        IgnoredLibraryScanItemArgs {
            title_id: Some(&title.id),
            session_id,
            library_path,
            item_path: &file.path,
            display_name: &display_name,
            query: &display_name,
            year_hint: title.year.and_then(|year| u32::try_from(year).ok()),
            reason_code: LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE,
            error_message: Some(error_message),
            size_bytes: file.size_bytes,
        },
    )
    .await
    {
        warn!(
            path = %file.path,
            error = %error,
            "failed to persist ignored movie scan file"
        );
    }
}

/// Register a discovered movie file the same way episodic title scans do:
/// persist or reuse a media-file row, run media analysis when needed, and
/// ensure a movie collection points at the file path for overview UI.
pub(super) async fn finalize_movie_scan_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    summary: &mut LibraryScanSummary,
    session_id: Option<&str>,
    title_scan_root: &str,
    cancel_token: Option<&CancellationToken>,
) {
    let file_path = stored_path_to_path_buf(&file.path);
    let file_stem = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(file_stem);

    let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(file) {
        snapshot
    } else {
        match file_source_snapshot_from_path(&file_path).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "failed to read movie file source signature during library scan"
                );
                persist_ignored_movie_scan_file_metadata_error(
                    app,
                    title,
                    file,
                    session_id,
                    title_scan_root,
                    error.to_string(),
                )
                .await;
                summary.skipped += 1;
                return;
            }
        }
    };

    let existing_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_path = %file.path,
                "failed to list media files during movie library scan"
            );
            return;
        }
    };

    let desired_source_signature_scheme = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.scheme.clone());
    let desired_source_signature_value = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.value.clone());
    let existing = existing_files
        .iter()
        .find(|item| item.file_path == file.path)
        .map(|existing| ExistingScannedMediaFile {
            file_id: existing.id.as_str(),
            should_skip_analysis: title_media_file_matches_snapshot(existing, &snapshot),
            should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                || existing.source_signature_scheme != desired_source_signature_scheme.clone()
                || existing.source_signature_value != desired_source_signature_value.clone()
                || existing.scan_status != "scanned",
            should_invalidate_full_hashes: title_media_file_quick_proof_changed(
                existing, &snapshot,
            ),
        });

    let destination_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_destination(&file_path)
        .await;
    let Some(mut persisted_file) = persist_or_reuse_scanned_media_file(
        app,
        title,
        file,
        &parsed,
        &snapshot,
        existing,
        summary,
        "failed to refresh movie media file source signature during library scan",
        "failed to insert movie media file during library scan",
    )
    .await
    else {
        return;
    };
    drop(destination_permit);

    match crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &persisted_file.file_id,
        None,
        file_path.as_path(),
    )
    .await
    {
        Ok(changed) => {
            if changed {
                persisted_file.title_updated = true;
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_id = %persisted_file.file_id,
                file_path = %file.path,
                "failed to reconcile external subtitles during movie scan"
            );
        }
    }

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    if persisted_file.should_analyze {
        let analysis_outcome = match app
            .services
            .library
            .media_analyzer
            .analyze_file(file_path.clone())
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "movie media analysis task failed during library scan"
                );
                MediaAnalysisOutcome::Invalid(error.to_string())
            }
        };
        if library_scan_cancel_requested(cancel_token) {
            return;
        }
        let analysis_status = scanned_media_analysis_status(&analysis_outcome);
        let (_, analysis_persisted) = persist_scanned_media_analysis_outcome(
            app,
            title,
            &persisted_file.file_id,
            analysis_outcome,
        )
        .await;
        if analysis_persisted {
            emit_scanned_media_file_analyzed_event(
                app,
                title,
                &persisted_file.file_id,
                &file.path,
                analysis_status,
                Vec::new(),
            )
            .await;
        }
    }

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    let collections = match app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
    {
        Ok(c) => c,
        Err(err) => {
            warn!(
                title_id = %title.id,
                error = %err,
                "failed to list collections during movie scan"
            );
            crate::import_workflow::mark_wanted_completed(app, &title.id, None, false).await;
            if persisted_file.title_updated {
                app.emit_title_updated_activity(None, title).await;
            }
            return;
        }
    };

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    if ensure_movie_collection_for_file(app, title, file, &parsed, &collections).await {
        persisted_file.title_updated = true;
    }

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    crate::import_workflow::mark_wanted_completed(app, &title.id, None, false).await;
    if persisted_file.title_updated {
        app.emit_title_updated_activity(None, title).await;
    }
}
