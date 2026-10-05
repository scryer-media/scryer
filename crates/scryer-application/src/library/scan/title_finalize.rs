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

/// The role a movie or series movie file found on disk is recorded with when
/// a scan inserts it. A file whose name marks it as an alternate cut is
/// recorded Additional, so the scan's election never makes it Primary; every
/// other file starts Primary and takes part in the election. Only new rows
/// use this: a file that already has a stored role keeps it.
fn scanned_movie_file_role(parsed: &crate::ParsedReleaseMetadata) -> crate::MediaFileRole {
    if parsed
        .edition
        .as_deref()
        .is_some_and(scryer_release_parser::is_alternate_cut_edition)
    {
        crate::MediaFileRole::Additional
    } else {
        crate::MediaFileRole::Primary
    }
}

/// Whether a scan that found `file_id` for a series movie's linked episode may
/// mark the episode's wanted row completed: the file is a Primary candidate,
/// or the episode already has a Primary. An alternate cut alone leaves the
/// episode missing, and a completed row would hold back a release parked for
/// it. A failed read keeps the old behaviour and completes the row.
async fn series_movie_episode_is_covered(
    app: &AppUseCase,
    title_id: &str,
    episode_id: &str,
    file_id: &str,
) -> bool {
    match app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(title_id, &[episode_id.to_string()])
        .await
    {
        Ok(files) => files.iter().any(|file| {
            (file.media_file.id == file_id && file.title_role == crate::MediaFileRole::Primary)
                || file.primary_episode_ids.iter().any(|id| id == episode_id)
        }),
        Err(_) => true,
    }
}

/// Whether a movie scan that found a file with `file_role` may mark the
/// movie's wanted row completed: the file is Primary, or the movie already has
/// a Primary. An alternate cut alone leaves the movie missing, and a completed
/// row would hold back a release parked for it.
fn movie_is_covered(file_role: crate::MediaFileRole, existing_files: &[TitleMediaFile]) -> bool {
    file_role == crate::MediaFileRole::Primary
        || existing_files
            .iter()
            .any(|file| file.role == crate::MediaFileRole::Primary)
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
    original_file_path: Option<String>,
    new_file_role: crate::MediaFileRole,
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

        // FR-046: the new sampled proof and stale full-hash invalidation form
        // one write. Failure must not make the old full hash appear current.
        if existing.should_refresh_source_signature || existing.should_invalidate_full_hashes {
            let db_started = Instant::now();
            let update_result = app
                .services
                .library
                .media_files
                .refresh_media_file_source_signature(
                    existing.file_id,
                    snapshot.size_bytes,
                    source_signature_scheme.clone(),
                    source_signature_value.clone(),
                    existing.should_invalidate_full_hashes,
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
                summary.skipped += 1;
                return None;
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
        role: new_file_role,
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
        // A scanned file has no grab behind it, so no listing snapshot.
        release_listing_json: None,
        original_file_path,
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
    inspected: crate::media::discs::CataloguedMediaAnalysis,
) -> (Duration, bool) {
    let db_started = Instant::now();
    let files = &app.services.library.media_files;
    let Ok(Some(current)) = files.get_media_file_by_id(file_id).await else {
        return (db_started.elapsed(), false);
    };
    let same_catalogue =
        inspected
            .expected
            .as_ref()
            .map_or(current.analysis_details.revision == 0, |expected| {
                expected.file_path == current.file_path
                    && expected.analysis_details == current.analysis_details
            });
    let same_source =
        crate::media::discs::MediaSourceVersion::read(&stored_path_to_path_buf(&current.file_path))
            .await
            .is_ok_and(|source| source == inspected.source);
    if !same_catalogue || !same_source {
        warn!(
            file_id,
            "discarded media analysis after source or saved metadata changed"
        );
        return (db_started.elapsed(), false);
    }

    let persisted = match inspected.outcome {
        MediaAnalysisOutcome::Valid(mut analysis) => {
            if let Some(disc) = &mut analysis.details.disc {
                let mapping_result = app.validate_disc_episode_mappings(title, disc).await;
                // Mapping validation awaits catalogue reads; the image may have
                // been replaced while those reads were in flight.
                if !crate::media::discs::MediaSourceVersion::read(&stored_path_to_path_buf(
                    &current.file_path,
                ))
                .await
                .is_ok_and(|source| source == inspected.source)
                {
                    return (db_started.elapsed(), false);
                }
                if let Err(error) = mapping_result {
                    analysis.details.report.status = scryer_media_types::ProbeStatus::Incomplete;
                    analysis
                        .details
                        .report
                        .warnings
                        .push(scryer_media_types::ProbeWarning {
                            code: "disc_episode_mapping_review".into(),
                            message: error.to_string(),
                            ..Default::default()
                        });
                    let recorded = files
                        .record_media_analysis_attempt(&current, &analysis)
                        .await
                        .unwrap_or(false);
                    return (db_started.elapsed(), recorded);
                }
            }
            let update_result = app
                .services
                .library
                .media_files
                .update_media_file_analysis_if_unchanged(&current, *analysis)
                .await;
            match update_result {
                Ok(updated) => updated,
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
        MediaAnalysisOutcome::Inconclusive(analysis) => files
            .record_media_analysis_attempt(&current, &analysis)
            .await
            .unwrap_or(false),
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
        MediaAnalysisOutcome::Inconclusive(_) => "incomplete",
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
    analysis_outcome: Option<crate::media::discs::CataloguedMediaAnalysis>,
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
        original_file_path,
    } = plan;

    let existing = match &record {
        PlannedTitleScanRecord::Existing {
            file_id,
            should_skip_analysis,
            should_refresh_source_signature,
            should_invalidate_full_hashes,
            replaced_episode_ids: _,
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

    // Ordinary episode files always start Primary; only a series movie's
    // file is judged by its edition.
    let new_file_role = if series_movie_link_id.is_some() {
        scanned_movie_file_role(&parsed)
    } else {
        crate::MediaFileRole::Primary
    };
    let Some(persisted_file) = persist_or_reuse_scanned_media_file(
        app,
        title,
        &file,
        &parsed,
        &snapshot,
        existing,
        original_file_path,
        new_file_role,
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
    // Disc availability is published atomically from saved playback mappings, never a filename.
    let is_disc_image = scryer_domain::is_disc_image(&destination_path);
    let target_episodes = if is_disc_image {
        Vec::new()
    } else {
        target_episodes
    };
    let series_movie_link_id = if is_disc_image {
        None
    } else {
        series_movie_link_id
    };
    let replaced_episode_ids = match record {
        PlannedTitleScanRecord::Existing {
            replaced_episode_ids,
            ..
        } if !is_disc_image => replaced_episode_ids,
        _ => None,
    };
    // When the replacement fails or is skipped the stored links stay as they
    // were, so the file keeps describing the episodes it is still linked to.
    let mut kept_episode_ids = None;
    if let Some(old_episode_ids) = replaced_episode_ids
        && !target_episodes.is_empty()
    {
        let new_episode_ids = target_episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect::<Vec<_>>();
        let db_started = Instant::now();
        let replace_result = app
            .services
            .library
            .media_files
            .replace_file_episode_links(&persisted_file.file_id, &old_episode_ids, &new_episode_ids)
            .await;
        *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        match replace_result {
            Ok(crate::EpisodeLinkReplacement::Replaced) => {
                tracing::info!(
                    title_id = %title.id,
                    file_id = %persisted_file.file_id,
                    file_path = %file.path,
                    old_episode_ids = ?old_episode_ids,
                    new_episode_ids = ?new_episode_ids,
                    "title scan replaced episode links that contradicted the filename"
                );
                summary.relinked += 1;
                title_updated = true;
                for episode_id in old_episode_ids {
                    episode_links.remove(&(persisted_file.file_id.clone(), episode_id));
                }
                for episode_id in new_episode_ids {
                    episode_links.insert((persisted_file.file_id.clone(), episode_id));
                }
            }
            Ok(crate::EpisodeLinkReplacement::Skipped) => {
                tracing::info!(
                    title_id = %title.id,
                    file_id = %persisted_file.file_id,
                    file_path = %file.path,
                    old_episode_ids = ?old_episode_ids,
                    new_episode_ids = ?new_episode_ids,
                    "title scan left episode links alone: the file changed since the scan read it"
                );
                kept_episode_ids = Some(old_episode_ids);
            }
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %persisted_file.file_id,
                    old_episode_ids = ?old_episode_ids,
                    new_episode_ids = ?new_episode_ids,
                    "failed to replace episode links of scanned file; keeping the stored links"
                );
                kept_episode_ids = Some(old_episode_ids);
            }
        }
    }
    let target_episodes = if kept_episode_ids.is_some() {
        Vec::new()
    } else {
        target_episodes
    };
    let external_subtitle_episode_id =
        match (kept_episode_ids.as_deref(), target_episodes.as_slice()) {
            (Some([episode_id]), _) => Some(episode_id.as_str()),
            (Some(_), _) => None,
            (None, [episode]) => Some(episode.id.as_str()),
            (None, _) => None,
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
        if series_movie_link_id.is_none()
            || series_movie_episode_is_covered(app, &title.id, &episode.id, &persisted_file.file_id)
                .await
        {
            crate::import_workflow::mark_wanted_completed(app, &title.id, Some(&episode.id), false)
                .await;
        }
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
        let analysis_status = scanned_media_analysis_status(&outcome.outcome);
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
                kept_episode_ids.clone().unwrap_or_else(|| {
                    target_episodes
                        .iter()
                        .map(|episode| episode.id.clone())
                        .collect()
                }),
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
#[expect(
    clippy::too_many_arguments,
    reason = "movie-scan finalization coordinates persistence, analysis, and summary accounting together"
)]
pub(super) async fn finalize_movie_scan_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    summary: &mut LibraryScanSummary,
    session_id: Option<&str>,
    title_scan_root: &str,
    cancel_token: Option<&CancellationToken>,
    mode: LibraryScanTitleWalkMode,
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
    let file_role = existing_files
        .iter()
        .find(|item| item.file_path == file.path)
        .map_or_else(
            || scanned_movie_file_role(&parsed),
            |existing| existing.role,
        );
    let movie_covered = movie_is_covered(file_role, &existing_files);

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
        None,
        scanned_movie_file_role(&parsed),
        summary,
        "failed to refresh movie media file source signature during library scan",
        "failed to insert movie media file during library scan",
    )
    .await
    else {
        return;
    };
    if mode == LibraryScanTitleWalkMode::FolderReconciliation
        && persisted_file.should_analyze
        && let Err(error) = app
            .services
            .library
            .media_files
            .update_media_file_analysis(
                &persisted_file.file_id,
                crate::MediaFileAnalysis::default(),
            )
            .await
    {
        warn!(%error, "failed to clear stale movie analysis during folder reconciliation");
        summary.skipped += 1;
        return;
    }
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

    if persisted_file.should_analyze && mode != LibraryScanTitleWalkMode::FolderReconciliation {
        let analysis_outcome = match app
            .analyze_catalogued_media_file(Some(&persisted_file.file_id), file_path.clone())
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                // The file stays catalogued; only its analysis failed. Flag
                // that record the way the episodic walk does so the failure
                // is visible instead of silently missing analysis details.
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "movie media analysis task failed during library scan"
                );
                if let Err(mark_error) = app
                    .services
                    .library
                    .media_files
                    .mark_scan_failed(&persisted_file.file_id, &error.to_string())
                    .await
                {
                    warn!(
                        error = %mark_error,
                        title_id = %title.id,
                        file_path = %file.path,
                        "failed to mark movie media file as scan_failed after a failed analysis"
                    );
                }
                return;
            }
        };
        if library_scan_cancel_requested(cancel_token) {
            return;
        }
        let analysis_status = scanned_media_analysis_status(&analysis_outcome.outcome);
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
            if movie_covered {
                crate::import_workflow::mark_wanted_completed(app, &title.id, None, false).await;
            }
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

    if movie_covered {
        crate::import_workflow::mark_wanted_completed(app, &title.id, None, false).await;
    }
    if persisted_file.title_updated {
        app.emit_title_updated_activity(None, title).await;
    }
}
