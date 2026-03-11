use crate::{
    app_usecase_post_processing::{spawn_post_processing, PostProcessingContext},
    evaluate_against_profile,
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_release_metadata, render_rename_template, require, ActivityChannel, ActivityKind,
    ActivitySeverity, AppError, AppResult, AppUseCase,
};
use chrono::Utc;
use scryer_domain::{
    is_video_file, Collection, CompletedDownload, DownloadQueueItem, DownloadQueueState,
    Entitlement, EventType, Id, ImportDecision, ImportResult, ImportSkipReason, MediaFacet, User,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SERIES_PATH_KEY: &str = "series.path";
const RENAME_TEMPLATE_SERIES_GLOBAL_KEY: &str = "rename.template.series.global";

/// Called from the download queue poller on every tick (currently 2 seconds).
/// Filters completed items, checks dedup, fetches CompletedDownload data, and triggers import.
pub async fn try_import_completed_downloads(
    app: &AppUseCase,
    actor: &User,
    items: &[DownloadQueueItem],
) {
    // TODO: increase to 600 (10 minutes) for production — large NAS copies can take a while
    match app
        .services
        .imports
        .recover_stale_processing_imports(120)
        .await
    {
        Ok(recovered) if recovered > 0 => {
            tracing::warn!(recovered, "recovered stale processing imports → failed");
            let _ = app
                .services
                .record_activity_event(
                    Some(actor.id.clone()),
                    None,
                    ActivityKind::SystemNotice,
                    format!(
                        "{} stale import(s) recovered as failed — check import history",
                        recovered
                    ),
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi],
                )
                .await;
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to recover stale processing imports");
        }
        _ => {}
    }

    let completed_items: Vec<&DownloadQueueItem> = items
        .iter()
        .filter(|item| item.state == DownloadQueueState::Completed)
        .filter(|item| {
            item.import_status.is_none() || item.import_status.as_deref() == Some("failed")
        })
        .collect();

    if completed_items.is_empty() {
        return;
    }

    // Fetch completed downloads from the download client (single RPC call)
    let completed_downloads = match app
        .services
        .download_client
        .list_completed_downloads()
        .await
    {
        Ok(downloads) => downloads,
        Err(error) => {
            tracing::warn!(error = %error, "failed to fetch completed downloads for import");
            return;
        }
    };

    for item in completed_items {
        // Check dedup
        let source_ref = &item.download_client_item_id;
        match app
            .services
            .imports
            .is_already_imported(&item.client_type, source_ref)
            .await
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(error = %error, source_ref = %source_ref, "import dedup check failed");
                continue;
            }
        }

        // Find the matching CompletedDownload
        let completed = match completed_downloads
            .iter()
            .find(|cd| cd.download_client_item_id == item.download_client_item_id)
        {
            Some(cd) => cd,
            None => continue, // Not found in completed downloads (might still be processing)
        };

        // Skip if dest_dir is empty
        if completed.dest_dir.is_empty() {
            continue;
        }

        // Only auto-import downloads that originated from scryer.
        // NZBGet embeds *scryer_title_id via PPParameters. SABnzbd has no
        // equivalent, so we fall back to the download_submissions table which
        // records the (title_id, facet) at grab time.
        let completed = if has_scryer_origin(&completed.parameters) {
            completed.clone()
        } else {
            // Fallback: look up the download_submissions table
            match app
                .services
                .download_submissions
                .find_by_client_item_id(&completed.client_type, &completed.download_client_item_id)
                .await
            {
                Ok(Some(submission)) => {
                    let mut patched = completed.clone();
                    patched.parameters = vec![
                        ("*scryer_title_id".to_string(), submission.title_id),
                        ("*scryer_facet".to_string(), submission.facet),
                    ];
                    patched
                }
                _ => continue, // Not a scryer-originated download
            }
        };

        let facet_label = extract_parameter(&completed.parameters, "*scryer_facet")
            .unwrap_or_else(|| "unknown".to_string());
        let import_start = std::time::Instant::now();
        match import_completed_download(app, actor, &completed).await {
            Ok(result) => {
                tracing::info!(
                    decision = ?result.decision,
                    title_id = ?result.title_id,
                    dest_path = ?result.dest_path,
                    "import completed for {}",
                    completed.name
                );
                metrics::counter!("scryer_imports_total", "decision" => result.decision.as_str(), "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    name = %completed.name,
                    "import failed for completed download"
                );
                metrics::counter!("scryer_imports_total", "decision" => "error", "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
            }
        }
    }
}

pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let started_at = Utc::now();
    let source_ref = &completed.download_client_item_id;

    // 1. DEDUP CHECK
    if app
        .services
        .imports
        .is_already_imported(&completed.client_type, source_ref)
        .await?
    {
        return Ok(ImportResult {
            import_id: String::new(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            title_id: None,
            source_path: completed.dest_dir.clone(),
            dest_path: None,
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            started_at,
            completed_at: Utc::now(),
        });
    }

    // Queue the import request for tracking
    let import_type = {
        let facet_str = extract_parameter(&completed.parameters, "*scryer_facet");
        let is_episode = facet_str
            .as_deref()
            .and_then(|f| app.facet_registry.all().find(|h| h.facet_id() == f))
            .is_some_and(|h| h.has_episodes());
        if is_episode {
            "tv_download"
        } else {
            "movie_download"
        }
    };
    let import_id = app
        .services
        .imports
        .queue_import_request(
            completed.client_type.clone(),
            source_ref.clone(),
            import_type.to_string(),
            serde_json::to_string(completed).unwrap_or_default(),
        )
        .await?;

    // Mark as processing
    app.services
        .imports
        .update_import_status(&import_id, "processing", None)
        .await?;

    // From here on, any error must update the import record to "failed" rather than
    // propagating via `?`. Otherwise the record stays "processing" indefinitely.
    match run_import(app, actor, &import_id, completed, started_at).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: None,
                source_path: completed.dest_dir.clone(),
                dest_path: None,
                file_size_bytes: None,
                link_type: None,
                error_message: Some(error.to_string()),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            let _ = app
                .services
                .imports
                .update_import_status(&import_id, "failed", result_json)
                .await;
            Ok(result)
        }
    }
}

async fn run_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    completed: &CompletedDownload,
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    // 2. TITLE MATCHING
    let mut title = None;
    if let Some(title_id) = extract_parameter(&completed.parameters, "*scryer_title_id") {
        let title_id = title_id.trim();
        if !title_id.is_empty() {
            title = app.services.titles.get_by_id(title_id).await?;
        }
    }

    // fallback to IMDb ID if needed
    if title.is_none() {
        let imdb_id = extract_parameter(&completed.parameters, "*scryer_imdb_id")
            .and_then(|value| normalize_imdb_id(&value));

        title = match imdb_id {
            Some(target_imdb_id) => {
                let titles = app.services.titles.list(None, None).await?;
                let mut matches = titles
                    .into_iter()
                    .filter(|title| {
                        title.external_ids.iter().any(|external_id| {
                            external_id.source.eq_ignore_ascii_case("imdb")
                                && normalize_imdb_id(&external_id.value).as_deref()
                                    == Some(target_imdb_id.as_str())
                        })
                    })
                    .collect::<Vec<_>>();

                if matches.len() == 1 {
                    matches.pop()
                } else {
                    None
                }
            }
            None => None,
        };
    }

    let title = match title {
        Some(t) => t,
        None => {
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Unmatched,
                skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
                title_id: None,
                source_path: completed.dest_dir.clone(),
                dest_path: None,
                file_size_bytes: None,
                link_type: None,
                error_message: Some(format!(
                    "could not match download '{}' to any monitored title",
                    completed.name
                )),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.services
                .imports
                .update_import_status(import_id, "failed", result_json)
                .await?;

            let unmatched_msg = format!(
                "Could not match download '{}' to any monitored title",
                completed.name
            );

            let _ = app
                .services
                .record_event(
                    Some(actor.id.clone()),
                    None,
                    EventType::Error,
                    unmatched_msg.clone(),
                )
                .await;

            app.services
                .record_activity_event(
                    Some(actor.id.clone()),
                    None,
                    ActivityKind::SystemNotice,
                    unmatched_msg,
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi],
                )
                .await?;

            return Ok(result);
        }
    };

    // Validate supported facets
    if !matches!(
        title.facet,
        MediaFacet::Movie | MediaFacet::Tv | MediaFacet::Anime
    ) {
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            title_id: Some(title.id.clone()),
            source_path: completed.dest_dir.clone(),
            dest_path: None,
            file_size_bytes: None,
            link_type: None,
            error_message: Some(format!(
                "title '{}' has unsupported facet '{:?}', skipping import",
                title.name, title.facet
            )),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        app.services
            .imports
            .update_import_status(import_id, "skipped", result_json)
            .await?;
        return Ok(result);
    }

    // 3. FIND VIDEO FILES
    let dest_dir = Path::new(&completed.dest_dir);
    let is_series = matches!(title.facet, MediaFacet::Tv | MediaFacet::Anime);
    let video_files = find_video_files(dest_dir, is_series)?;

    if video_files.is_empty() {
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Failed,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some(title.id.clone()),
            source_path: completed.dest_dir.clone(),
            dest_path: None,
            file_size_bytes: None,
            link_type: None,
            error_message: Some(format!("no video files found in {}", completed.dest_dir)),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        app.services
            .imports
            .update_import_status(import_id, "failed", result_json)
            .await?;
        return Ok(result);
    }

    // Branch on facet: movies import the single largest file, series import all episode files
    if is_series {
        import_series_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
        )
        .await
    } else {
        import_movie_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Movie import: pick largest file, single import
// ---------------------------------------------------------------------------

async fn import_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let (media_root, rename_template) = resolve_import_paths(app, title).await?;

    let parsed = parse_release_metadata(
        source_video
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&completed.name),
    );

    let ext = source_video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv")
        .to_string();

    let tokens = build_rename_tokens(title, &parsed, &ext);
    let rendered_filename = render_rename_template(&rename_template, &tokens);

    let year_str = parsed.year.map(|y| format!(" ({})", y)).unwrap_or_default();
    let title_folder = format!("{}{}", title.name, year_str);

    let dest_path = PathBuf::from(&media_root)
        .join(&title_folder)
        .join(&rendered_filename);

    // Pre-import checks
    let existing_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: &source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &parsed,
        existing_files: &existing_files,
    };
    let verdict = crate::import_checks::run_import_checks(&check_ctx);
    if let crate::import_checks::ImportVerdict::Reject { reason, code } = verdict {
        let skip_reason = Some(match code {
            "duplicate_file" => ImportSkipReason::DuplicateFile,
            "insufficient_disk_space" => ImportSkipReason::DiskFull,
            "invalid_extension" | "sample_file" | "sample_directory" => {
                ImportSkipReason::NoVideoFiles
            }
            _ => ImportSkipReason::PolicyMismatch,
        });
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_path: source_video.to_string_lossy().to_string(),
            dest_path: Some(dest_path.to_string_lossy().to_string()),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        app.services
            .imports
            .update_import_status(import_id, "skipped", result_json)
            .await?;
        return Ok(result);
    }

    // Upgrade check: if there are existing files, score and compare
    if !existing_files.is_empty() {
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.as_str());
        let category_hint = facet_to_category_hint(&title.facet);
        if let Ok(quality_profile) = app
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id,
                Some(category_hint),
            )
            .await
        {
            let persona = quality_profile
                .criteria
                .resolve_persona(Some(category_hint));
            let weights = crate::scoring_weights::build_weights(
                persona,
                &quality_profile.criteria.scoring_overrides,
            );
            let new_decision = evaluate_against_profile(&quality_profile, &parsed, true, &weights);
            let new_score = new_decision.preference_score;

            // Find the best existing file by acquisition_score
            if let Some(existing_file) = existing_files
                .iter()
                .max_by_key(|f| f.acquisition_score.unwrap_or(0))
            {
                let old_score = existing_file.acquisition_score.unwrap_or(0);
                if new_score > old_score {
                    let media_root_opt = crate::recycle_bin::media_root_for_title(app, title).await;
                    let recycle_config =
                        crate::recycle_bin::resolve_recycle_config(app, media_root_opt.as_deref())
                            .await;

                    match crate::upgrade::execute_upgrade(
                        app,
                        &title.name,
                        &title.id,
                        existing_file,
                        &source_video,
                        &dest_path,
                        &parsed,
                        new_score,
                        old_score,
                        &recycle_config,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            let result = ImportResult {
                                import_id: import_id.to_string(),
                                decision: ImportDecision::Imported,
                                skip_reason: None,
                                title_id: Some(title.id.clone()),
                                source_path: source_video.to_string_lossy().to_string(),
                                dest_path: Some(dest_path.to_string_lossy().to_string()),
                                file_size_bytes: Some(source_size),
                                link_type: None,
                                error_message: None,
                                started_at,
                                completed_at: Utc::now(),
                            };
                            tracing::info!(
                                title = %title.name,
                                old_score = outcome.old_score,
                                new_score = outcome.new_score,
                                "movie file upgraded"
                            );
                            let result_json = serde_json::to_string(&result).ok();
                            app.services
                                .imports
                                .update_import_status(import_id, "completed", result_json)
                                .await?;
                            return Ok(result);
                        }
                        Err(err) => {
                            tracing::error!(error = %err, "upgrade failed, falling through to normal import");
                        }
                    }
                }
            }
        }
    }

    // Import file
    let file_result = app
        .services
        .file_importer
        .import_file(&source_video, &dest_path)
        .await?;

    // Spawn post-processing script (non-blocking)
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor_id: Some(actor.id.clone()),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "imdb")
            .map(|e| e.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.clone()),
        season: None,
        episode: None,
        quality: parsed.quality.clone(),
    });

    // Write NFO sidecar (non-fatal, opt-in)
    {
        let nfo_enabled = app
            .read_setting_string_value("nfo.write_on_import.movie", None)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("true");

        if nfo_enabled {
            let nfo_path = dest_path.with_extension("nfo");
            let nfo_content = render_movie_nfo(title);
            if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %nfo_path.display(),
                    "failed to write movie NFO sidecar"
                );
            }
        }
    }

    // Record media file with rich metadata
    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: dest_path.to_string_lossy().to_string(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: parsed.quality.clone(),
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: parsed.source.clone(),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec.clone(),
        audio_codec_parsed: parsed.audio.clone(),
        original_file_path: Some(source_video.to_string_lossy().to_string()),
        ..Default::default()
    };
    let media_file_id = match app
        .services
        .media_files
        .insert_media_file(&media_file_input)
        .await
    {
        Ok(id) => Some(id),
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                dest_path = %dest_path.display(),
                "failed to insert media_files record (import will still succeed)"
            );
            None
        }
    };

    if let Some(ref file_id) = media_file_id {
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.as_str());
        let category_hint = facet_to_category_hint(&title.facet);
        let required_audio_languages = app
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id,
                Some(category_hint),
            )
            .await
            .map(|p| p.criteria.required_audio_languages)
            .unwrap_or_default();
        spawn_media_analysis(
            app,
            file_id.clone(),
            dest_path.clone(),
            title.id.clone(),
            required_audio_languages,
        );
    }

    // Create collection record (so the movie overview UI can show the file)
    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: "movie".to_string(),
        collection_index: "1".to_string(),
        label: parsed.quality.clone(),
        ordered_path: Some(dest_path.to_string_lossy().to_string()),
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        interstitial_movie: None,
        monitored: true,
        created_at: Utc::now(),
    };
    if let Err(err) = app.services.shows.create_collection(collection).await {
        tracing::warn!(error = %err, title_id = %title.id, "failed to create collection record");
    }

    // Reconcile wanted item state
    mark_wanted_completed(app, &title.id, None, None).await;

    // Finalize import record
    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_path: source_video.to_string_lossy().to_string(),
        dest_path: Some(dest_path.to_string_lossy().to_string()),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(file_result.strategy),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.services
        .imports
        .update_import_status(import_id, "completed", result_json)
        .await?;

    // Emit events
    let event_message = format!(
        "Imported '{}' via {} to {}",
        title.name,
        file_result.strategy.as_str(),
        dest_path.display()
    );
    let _ = app
        .services
        .record_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            EventType::ActionCompleted,
            event_message.clone(),
        )
        .await;
    app.services
        .record_activity_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            ActivityKind::MovieDownloaded,
            event_message,
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi],
        )
        .await?;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Series import: process ALL video files, link each to its episode
// ---------------------------------------------------------------------------

async fn import_series_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let (media_root, rename_template) = resolve_import_paths(app, title).await?;
    let title_folder = title.name.clone();

    // Check NFO write setting (non-fatal, opt-in)
    let nfo_key = match title.facet {
        scryer_domain::MediaFacet::Anime => "nfo.write_on_import.anime",
        _ => "nfo.write_on_import.series",
    };
    let nfo_enabled = app
        .read_setting_string_value(nfo_key, None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");

    // Write tvshow.nfo once per series (write-once: skip if already exists)
    if nfo_enabled {
        let tvshow_nfo_path = PathBuf::from(&media_root)
            .join(&title_folder)
            .join("tvshow.nfo");
        if !tvshow_nfo_path.exists() {
            if let Some(parent) = tvshow_nfo_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let nfo_content = render_tvshow_nfo(title);
            if let Err(err) = tokio::fs::write(&tvshow_nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %tvshow_nfo_path.display(),
                    "failed to write tvshow NFO sidecar"
                );
            }
        }
    }

    // Write .plexmatch hint file (non-fatal, opt-in, write-once)
    {
        let plexmatch_key = match title.facet {
            scryer_domain::MediaFacet::Anime => "plexmatch.write_on_import.anime",
            _ => "plexmatch.write_on_import.series",
        };
        let plexmatch_enabled = app
            .read_setting_string_value(plexmatch_key, None)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("true");

        if plexmatch_enabled {
            let plexmatch_path = PathBuf::from(&media_root)
                .join(&title_folder)
                .join(".plexmatch");
            if !plexmatch_path.exists() {
                if let Some(parent) = plexmatch_path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let content = render_plexmatch(title);
                if let Err(err) = tokio::fs::write(&plexmatch_path, content.as_bytes()).await {
                    tracing::warn!(
                        error = %err,
                        path = %plexmatch_path.display(),
                        "failed to write .plexmatch hint file"
                    );
                }
            }
        }
    }

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;

    for source_video in video_files {
        match import_single_episode_file(
            app,
            title,
            &media_root,
            &rename_template,
            &title_folder,
            source_video,
            nfo_enabled,
        )
        .await
        {
            Ok(true) => imported_count += 1,
            Ok(false) => skipped_count += 1, // duplicate or unparseable
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    file = %source_video.display(),
                    title = %title.name,
                    "failed to import episode file"
                );
                last_error = Some(err.to_string());
                failed_count += 1;
            }
        }
    }

    let (decision, status) = if imported_count > 0 {
        (ImportDecision::Imported, "completed")
    } else {
        (ImportDecision::Failed, "failed")
    };

    let error_message = if failed_count > 0 || skipped_count > 0 {
        Some(format!(
            "{imported_count} imported, {skipped_count} skipped, {failed_count} failed{}",
            last_error
                .as_ref()
                .map(|e| format!(". Last error: {e}"))
                .unwrap_or_default()
        ))
    } else {
        None
    };

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        file_size_bytes: None,
        link_type: None,
        error_message,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.services
        .imports
        .update_import_status(import_id, status, result_json)
        .await?;

    // Emit events
    let event_message = format!(
        "Imported {} of {} episode files for '{}'",
        imported_count,
        video_files.len(),
        title.name
    );
    let _ = app
        .services
        .record_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            EventType::ActionCompleted,
            event_message.clone(),
        )
        .await;
    app.services
        .record_activity_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            ActivityKind::SeriesEpisodeImported,
            event_message,
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi],
        )
        .await?;

    Ok(result)
}

/// Import a single episode video file: parse, match, import, link, mark wanted.
/// Returns Ok(true) if imported, Ok(false) if skipped (duplicate or unparseable).
async fn import_single_episode_file(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    media_root: &str,
    rename_template: &str,
    title_folder: &str,
    source_video: &Path,
    nfo_enabled: bool,
) -> AppResult<bool> {
    let source_size = std::fs::metadata(source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let parsed = parse_release_metadata(
        source_video
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(""),
    );

    let ext = source_video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv")
        .to_string();

    // Must have episode info to proceed
    let ep_meta = match parsed.episode.as_ref() {
        Some(ep) if !ep.episode_numbers.is_empty() => ep,
        Some(ep)
            if ep.absolute_episode.is_some() && title.facet == scryer_domain::MediaFacet::Anime =>
        {
            ep
        }
        _ => {
            tracing::debug!(
                file = %source_video.display(),
                "skipping file with no parseable episode info"
            );
            return Ok(false);
        }
    };

    let season = ep_meta.season.unwrap_or(1);
    let season_str = season.to_string();

    // Build rename tokens
    let mut tokens = build_rename_tokens(title, &parsed, &ext);
    tokens.insert("season".to_string(), season_str.clone());
    if let Some(ep_num) = ep_meta.episode_numbers.first() {
        tokens.insert("episode".to_string(), ep_num.to_string());
    }
    if let Some(abs) = ep_meta.absolute_episode {
        tokens.insert("absolute_episode".to_string(), abs.to_string());
    }

    let rendered_filename = render_rename_template(rename_template, &tokens);
    let dest_path = if use_season_folders(title) {
        let season_folder = format!("Season {:02}", season);
        PathBuf::from(media_root)
            .join(title_folder)
            .join(&season_folder)
            .join(&rendered_filename)
    } else {
        PathBuf::from(media_root)
            .join(title_folder)
            .join(&rendered_filename)
    };

    // Pre-import checks
    let existing_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &parsed,
        existing_files: &existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        tracing::debug!(file = %dest_path.display(), %code, %reason, "skipping episode file");
        return Ok(false);
    }

    // Upgrade check for episodes: find existing file for same dest path
    if !existing_files.is_empty() {
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.as_str());
        let category_hint = facet_to_category_hint(&title.facet);
        if let Ok(quality_profile) = app
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id,
                Some(category_hint),
            )
            .await
        {
            let persona = quality_profile
                .criteria
                .resolve_persona(Some(category_hint));
            let weights = crate::scoring_weights::build_weights(
                persona,
                &quality_profile.criteria.scoring_overrides,
            );
            let new_decision = evaluate_against_profile(&quality_profile, &parsed, true, &weights);
            let new_score = new_decision.preference_score;
            let dest_str = dest_path.to_string_lossy();

            // Find an existing file at the same dest path (or matching episode)
            if let Some(existing_file) = existing_files
                .iter()
                .find(|f| f.file_path == dest_str.as_ref())
            {
                let old_score = existing_file.acquisition_score.unwrap_or(0);
                if new_score > old_score {
                    let recycle_config =
                        crate::recycle_bin::resolve_recycle_config(app, Some(media_root)).await;

                    match crate::upgrade::execute_upgrade(
                        app,
                        &title.name,
                        &title.id,
                        existing_file,
                        source_video,
                        &dest_path,
                        &parsed,
                        new_score,
                        old_score,
                        &recycle_config,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            tracing::info!(
                                title = %title.name,
                                old_score = outcome.old_score,
                                new_score = outcome.new_score,
                                "episode file upgraded"
                            );
                            return Ok(true);
                        }
                        Err(err) => {
                            tracing::error!(error = %err, "episode upgrade failed, falling through to normal import");
                        }
                    }
                }
            }
        }
    }

    // Import file (hardlink/copy)
    let file_result = app
        .services
        .file_importer
        .import_file(source_video, &dest_path)
        .await?;

    // Spawn post-processing script (non-blocking)
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor_id: None,
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "imdb")
            .map(|e| e.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.clone()),
        season: Some(season),
        episode: ep_meta.episode_numbers.first().copied(),
        quality: parsed.quality.clone(),
    });

    // Write episode NFO sidecar (non-fatal, opt-in)
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let episode = if let Some(ep_num) = ep_meta.episode_numbers.first() {
            app.services
                .shows
                .find_episode_by_title_and_numbers(&title.id, &season_str, &ep_num.to_string())
                .await
                .ok()
                .flatten()
        } else if let Some(abs) = ep_meta.absolute_episode {
            app.services
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &abs.to_string())
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        if let Some(ref episode) = episode {
            let nfo_content = render_episode_nfo(title, episode);
            if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %nfo_path.display(),
                    "failed to write episode NFO sidecar"
                );
            }
        }
    }

    // Record media file with rich metadata
    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: dest_path.to_string_lossy().to_string(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: parsed.quality.clone(),
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: parsed.source.clone(),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec.clone(),
        audio_codec_parsed: parsed.audio.clone(),
        original_file_path: Some(source_video.to_string_lossy().to_string()),
        ..Default::default()
    };
    let media_file_id = app
        .services
        .media_files
        .insert_media_file(&media_file_input)
        .await?;

    {
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.as_str());
        let category_hint = facet_to_category_hint(&title.facet);
        let required_audio_languages = app
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id,
                Some(category_hint),
            )
            .await
            .map(|p| p.criteria.required_audio_languages)
            .unwrap_or_default();
        spawn_media_analysis(
            app,
            media_file_id.clone(),
            dest_path.clone(),
            title.id.clone(),
            required_audio_languages,
        );
    }

    // Link to ALL matching episodes (supports multi-episode files like S01E01E02)
    let mut linked = false;
    for ep_num in &ep_meta.episode_numbers {
        let ep_str = ep_num.to_string();
        match app
            .services
            .shows
            .find_episode_by_title_and_numbers(&title.id, &season_str, &ep_str)
            .await
        {
            Ok(Some(episode)) => {
                if let Err(err) = app
                    .services
                    .media_files
                    .link_file_to_episode(&media_file_id, &episode.id)
                    .await
                {
                    tracing::warn!(error = %err, episode_id = %episode.id, "failed to link file to episode");
                }
                mark_wanted_completed(app, &title.id, Some(&episode.id), None).await;
                linked = true;
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id, season = %season_str, episode = %ep_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => {
                tracing::warn!(error = %err, "episode lookup failed during import");
            }
        }
    }

    // Anime absolute fallback: if no S##E## match but we have an absolute number
    if !linked {
        if let Some(abs) = ep_meta.absolute_episode {
            let abs_str = abs.to_string();
            match app
                .services
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &abs_str)
                .await
            {
                Ok(Some(episode)) => {
                    if let Err(err) = app
                        .services
                        .media_files
                        .link_file_to_episode(&media_file_id, &episode.id)
                        .await
                    {
                        tracing::warn!(error = %err, episode_id = %episode.id, "failed to link file to episode (absolute)");
                    }
                    mark_wanted_completed(app, &title.id, Some(&episode.id), None).await;
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id, absolute = abs,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import");
                }
            }
        }
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve media root path and rename template for a title's facet.
async fn resolve_import_paths(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> AppResult<(String, String)> {
    let handler = app.facet_registry.get(&title.facet);
    let media_root_key = handler
        .map(|h| h.library_path_key())
        .unwrap_or(SERIES_PATH_KEY);
    let rename_template_key = handler
        .map(|h| h.rename_template_key())
        .unwrap_or(RENAME_TEMPLATE_SERIES_GLOBAL_KEY);
    let media_root_default = handler
        .map(|h| h.default_library_path())
        .unwrap_or("/media/series");
    let rename_template_default = handler
        .map(|h| h.default_rename_template())
        .unwrap_or("{title} - S{season:2}E{episode:2} - {quality}.{ext}");

    let media_root = {
        let default_root = app
            .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, media_root_key, None)
            .await?
            .unwrap_or_else(|| media_root_default.to_string());

        title
            .tags
            .iter()
            .find(|t| t.starts_with("scryer:root-folder:"))
            .map(|t| t.trim_start_matches("scryer:root-folder:").to_string())
            .unwrap_or(default_root)
    };

    let rename_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            rename_template_key,
            None,
        )
        .await?
        .unwrap_or_else(|| rename_template_default.to_string());

    Ok((media_root, rename_template))
}

/// Check whether the title's tags request season-folder organisation.
/// Defaults to `true` (use season folders) when the tag is absent.
pub(crate) fn use_season_folders(title: &scryer_domain::Title) -> bool {
    title
        .tags
        .iter()
        .find(|t| t.starts_with("scryer:season-folder:"))
        .map(|t| {
            !t.trim_start_matches("scryer:season-folder:")
                .eq_ignore_ascii_case("disabled")
        })
        .unwrap_or(true)
}

/// Build the common rename token map from parsed release metadata.
pub(crate) fn build_rename_tokens(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    tokens.insert("title".to_string(), title.name.clone());
    tokens.insert(
        "year".to_string(),
        parsed.year.map(|y| y.to_string()).unwrap_or_default(),
    );
    tokens.insert(
        "quality".to_string(),
        parsed
            .quality
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
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
        "audio".to_string(),
        parsed.audio.clone().unwrap_or_default(),
    );
    tokens.insert(
        "release_group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert(
        "season".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.season)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.episode_numbers.first().copied())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.absolute_episode)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert("episode_title".to_string(), String::new());
    tokens.insert("ext".to_string(), ext.to_string());
    tokens
}

/// Mark a wanted item as completed for a title (and optionally a specific episode).
/// If `imported_score` is provided, it becomes the new `current_score`.
/// If the quality profile allows upgrades, the item re-enters "wanted" status
/// with a recomputed schedule (the 24h cooldown in `evaluate_upgrade` prevents churn).
async fn mark_wanted_completed(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    imported_score: Option<i32>,
) {
    match app
        .services
        .wanted_items
        .get_wanted_item_for_title(title_id, episode_id)
        .await
    {
        Ok(Some(wanted)) => {
            let now = Utc::now();
            let now_str = now.to_rfc3339();
            let score = imported_score.or(wanted.current_score);

            if let Err(err) = app
                .services
                .wanted_items
                .update_wanted_item_status(
                    &wanted.id,
                    "completed",
                    None,
                    Some(&now_str),
                    wanted.search_count,
                    score,
                    wanted.grabbed_release.as_deref(),
                )
                .await
            {
                tracing::warn!(error = %err, title_id = %title_id, "failed to mark wanted item completed");
            }
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(error = %err, title_id = %title_id, "failed to look up wanted item");
        }
    }
}

/// Returns true if the download was submitted by scryer (has *scryer_title_id parameter).
fn has_scryer_origin(params: &[(String, String)]) -> bool {
    params.iter().any(|(k, _)| k == "*scryer_title_id")
}

fn extract_parameter(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    let digits: String = raw_imdb_id.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.is_empty() {
        return None;
    }

    Some(format!("tt{}", digits))
}

/// Recursively find all video files under `dir`, optionally filtering out samples.
///
/// `dir` is usually a directory, but SABnzbd sometimes reports the file path
/// itself as the completed download's `storage` field. If the path has a video
/// extension and cannot be opened as a directory, we treat it as a single-file
/// result.
pub(crate) fn find_video_files(dir: &Path, filter_samples: bool) -> AppResult<Vec<PathBuf>> {
    let mut video_files = Vec::new();
    let mut dirs_to_visit = vec![dir.to_path_buf()];

    while let Some(current_dir) = dirs_to_visit.pop() {
        let entries = match std::fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) if current_dir == dir && is_video_file(dir) => {
                // The top-level path has a video extension but can't be read as
                // a directory — it's a file path, not a directory path.
                tracing::info!(
                    path = %dir.display(),
                    "download path is a video file, not a directory"
                );
                if !filter_samples || !is_sample_file(dir) {
                    video_files.push(dir.to_path_buf());
                }
                return Ok(video_files);
            }
            Err(e) if current_dir == dir => {
                // Top-level directory must be readable.
                return Err(AppError::Repository(format!(
                    "failed to read directory {}: {}",
                    current_dir.display(),
                    e
                )));
            }
            Err(e) => {
                // Subdirectory failures (encoding issues, stale mounts, not-actually-a-dir)
                // should not abort the entire scan.
                tracing::warn!(
                    path = %current_dir.display(),
                    error = %e,
                    "skipping unreadable path during video file scan"
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            // Check video extension first — some filesystem mounts (NFS, CIFS,
            // Docker volumes) incorrectly report files with non-ASCII names as
            // directories, so we must not rely on is_dir() alone.
            if is_video_file(&path) {
                if filter_samples && is_sample_file(&path) {
                    continue;
                }
                video_files.push(path);
            } else if path.is_dir() {
                dirs_to_visit.push(path);
            }
        }
    }

    Ok(video_files)
}

const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024; // 50 MB

pub(crate) fn is_sample_file(path: &Path) -> bool {
    let filename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if filename.contains("sample") {
        return true;
    }

    // Small files in multi-episode directories are almost certainly samples/promos
    std::fs::metadata(path)
        .map(|m| m.len() < SAMPLE_SIZE_THRESHOLD)
        .unwrap_or(false)
}

pub(crate) fn pick_largest_file(files: &[PathBuf]) -> AppResult<PathBuf> {
    files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
        .cloned()
        .ok_or_else(|| AppError::Repository("no files to pick from".to_string()))
}

// ---------------------------------------------------------------------------
// Manual import: preview & execute
// ---------------------------------------------------------------------------

/// A single file in a manual import preview with auto-detected episode info.
pub struct ManualImportFilePreview {
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub quality: Option<String>,
    pub parsed_season: Option<u32>,
    pub parsed_episodes: Vec<u32>,
    pub suggested_episode_id: Option<String>,
    pub suggested_episode_label: Option<String>,
}

/// Result of previewing a manual import: file list + available episodes for matching.
pub struct ManualImportPreview {
    pub files: Vec<ManualImportFilePreview>,
    pub available_episodes: Vec<scryer_domain::Episode>,
}

/// Scan a completed download's directory and attempt to auto-match files to episodes.
pub async fn preview_manual_import(
    app: &AppUseCase,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<ManualImportPreview> {
    // Look up completed download to get dest_dir
    let completed_downloads = app
        .services
        .download_client
        .list_completed_downloads()
        .await?;
    let completed = completed_downloads
        .iter()
        .find(|cd| cd.download_client_item_id == download_client_item_id)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "completed download not found: {}",
                download_client_item_id
            ))
        })?;

    // Scan for video files (recursive, no sample filtering — let user see everything)
    let dest_dir = Path::new(&completed.dest_dir);
    let video_files = find_video_files(dest_dir, false)?;

    // Get all episodes for this title across all seasons
    let collections = app
        .services
        .shows
        .list_collections_for_title(title_id)
        .await?;
    let mut all_episodes = Vec::new();
    for collection in &collections {
        let episodes = app
            .services
            .shows
            .list_episodes_for_collection(&collection.id)
            .await?;
        all_episodes.extend(episodes);
    }

    // For each file, parse and attempt auto-match
    let mut previews = Vec::new();
    for path in &video_files {
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown")
            .to_string();
        let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&file_name);
        let parsed = parse_release_metadata(stem);

        let mut suggested_episode_id = None;
        let mut suggested_episode_label = None;
        let mut parsed_season = None;
        let mut parsed_episodes = Vec::new();

        if let Some(ref ep_meta) = parsed.episode {
            parsed_season = ep_meta.season;
            parsed_episodes = ep_meta.episode_numbers.clone();

            let season_str = ep_meta
                .season
                .map(|s| s.to_string())
                .unwrap_or_else(|| "1".to_string());
            if let Some(ep_num) = ep_meta.episode_numbers.first() {
                let ep_str = ep_num.to_string();
                if let Ok(Some(episode)) = app
                    .services
                    .shows
                    .find_episode_by_title_and_numbers(title_id, &season_str, &ep_str)
                    .await
                {
                    let label = format!(
                        "S{:02}E{:02}{}",
                        ep_meta.season.unwrap_or(1),
                        ep_num,
                        episode
                            .title
                            .as_ref()
                            .map(|t| format!(" - {}", t))
                            .unwrap_or_default()
                    );
                    suggested_episode_id = Some(episode.id.clone());
                    suggested_episode_label = Some(label);
                }
            }

            // Anime absolute fallback
            if suggested_episode_id.is_none() {
                if let Some(abs) = ep_meta.absolute_episode {
                    let abs_str = abs.to_string();
                    if let Ok(Some(episode)) = app
                        .services
                        .shows
                        .find_episode_by_title_and_absolute_number(title_id, &abs_str)
                        .await
                    {
                        let label = format!(
                            "#{}{}",
                            abs,
                            episode
                                .title
                                .as_ref()
                                .map(|t| format!(" - {}", t))
                                .unwrap_or_default()
                        );
                        suggested_episode_id = Some(episode.id.clone());
                        suggested_episode_label = Some(label);
                    }
                }
            }
        }

        previews.push(ManualImportFilePreview {
            file_path: path.to_string_lossy().to_string(),
            file_name,
            size_bytes: size,
            quality: parsed.quality.clone(),
            parsed_season,
            parsed_episodes,
            suggested_episode_id,
            suggested_episode_label,
        });
    }

    Ok(ManualImportPreview {
        files: previews,
        available_episodes: all_episodes,
    })
}

/// A user-specified mapping of one file to one episode.
pub struct ManualImportFileMapping {
    pub file_path: String,
    pub episode_id: String,
    pub quality: Option<String>,
}

/// Per-file result of a manual import execution.
pub struct ManualImportFileResult {
    pub file_path: String,
    pub episode_id: String,
    pub success: bool,
    pub dest_path: Option<String>,
    pub error_message: Option<String>,
}

/// Execute a manual import: import each file with user-specified episode mappings.
pub async fn execute_manual_import(
    app: &AppUseCase,
    actor: &User,
    title_id: &str,
    files: Vec<ManualImportFileMapping>,
) -> AppResult<Vec<ManualImportFileResult>> {
    require(actor, &Entitlement::TriggerActions)?;
    let title = app
        .services
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title not found: {}", title_id)))?;

    let (media_root, rename_template) = resolve_import_paths(app, &title).await?;
    let title_folder = title.name.clone();

    let mut results = Vec::new();

    for mapping in &files {
        let source = Path::new(&mapping.file_path);

        // Validate file exists
        if !source.exists() || !source.is_file() {
            results.push(ManualImportFileResult {
                file_path: mapping.file_path.clone(),
                episode_id: mapping.episode_id.clone(),
                success: false,
                dest_path: None,
                error_message: Some(format!("file not found: {}", mapping.file_path)),
            });
            continue;
        }

        // Look up episode
        let episode = match app
            .services
            .shows
            .get_episode_by_id(&mapping.episode_id)
            .await
        {
            Ok(Some(ep)) => ep,
            Ok(None) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_message: Some(format!("episode not found: {}", mapping.episode_id)),
                });
                continue;
            }
            Err(err) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_message: Some(format!("episode lookup failed: {}", err)),
                });
                continue;
            }
        };

        // Parse release metadata for quality/codec tokens
        let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let parsed = parse_release_metadata(stem);
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("mkv");

        let season_num: u32 = episode
            .season_number
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let ep_num: u32 = episode
            .episode_number
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Build rename tokens using episode metadata (not filename parsing)
        let mut tokens = build_rename_tokens(&title, &parsed, ext);
        tokens.insert("season".to_string(), season_num.to_string());
        tokens.insert("episode".to_string(), ep_num.to_string());
        tokens.insert(
            "episode_title".to_string(),
            episode.title.clone().unwrap_or_default(),
        );
        // Override quality if user specified one
        if let Some(ref q) = mapping.quality {
            tokens.insert("quality".to_string(), q.clone());
        }

        let rendered = render_rename_template(&rename_template, &tokens);
        let dest_path = if use_season_folders(&title) {
            let season_folder = format!("Season {:02}", season_num);
            PathBuf::from(&media_root)
                .join(&title_folder)
                .join(&season_folder)
                .join(&rendered)
        } else {
            PathBuf::from(&media_root)
                .join(&title_folder)
                .join(&rendered)
        };

        // Import file
        match app
            .services
            .file_importer
            .import_file(source, &dest_path)
            .await
        {
            Ok(file_result) => {
                let quality_label = mapping.quality.clone().or_else(|| parsed.quality.clone());

                // Record media file with rich metadata
                let media_file_input = crate::InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: dest_path.to_string_lossy().to_string(),
                    size_bytes: file_result.size_bytes as i64,
                    quality_label: quality_label.clone(),
                    scene_name: Some(parsed.raw_title.clone()),
                    release_group: parsed.release_group.clone(),
                    source_type: parsed.source.clone(),
                    resolution: quality_label,
                    video_codec_parsed: parsed.video_codec.clone(),
                    audio_codec_parsed: parsed.audio.clone(),
                    original_file_path: Some(source.to_string_lossy().to_string()),
                    ..Default::default()
                };
                if let Ok(mf_id) = app
                    .services
                    .media_files
                    .insert_media_file(&media_file_input)
                    .await
                {
                    let _ = app
                        .services
                        .media_files
                        .link_file_to_episode(&mf_id, &episode.id)
                        .await;
                }

                // Mark wanted item completed
                mark_wanted_completed(app, &title.id, Some(&episode.id), None).await;

                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: true,
                    dest_path: Some(dest_path.to_string_lossy().to_string()),
                    error_message: None,
                });
            }
            Err(err) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_message: Some(err.to_string()),
                });
            }
        }
    }

    // Emit summary event
    let success_count = results.iter().filter(|r| r.success).count();
    let event_message = format!(
        "Manual import: {} of {} files imported for '{}'",
        success_count,
        results.len(),
        title.name
    );
    let _ = app
        .services
        .record_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            EventType::ActionCompleted,
            event_message.clone(),
        )
        .await;
    app.services
        .record_activity_event(
            Some(actor.id.clone()),
            Some(title.id.clone()),
            ActivityKind::SeriesEpisodeImported,
            event_message,
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi],
        )
        .await?;

    Ok(results)
}

// ---------------------------------------------------------------------------
// Post-import media analysis (background, non-blocking)
// ---------------------------------------------------------------------------

fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Tv => "tv",
        MediaFacet::Anime => "anime",
        MediaFacet::Other => "other",
    }
}

/// Returns the subset of `required` language codes (uppercase 3-letter ISO) that
/// are absent from `actual` (which may use any case — comparison is case-insensitive).
fn missing_audio_languages<'a>(required: &'a [String], actual: &[String]) -> Vec<&'a str> {
    let actual_upper: std::collections::HashSet<String> =
        actual.iter().map(|l| l.to_ascii_uppercase()).collect();
    required
        .iter()
        .filter(|r| !actual_upper.contains(r.as_str()))
        .map(String::as_str)
        .collect()
}

/// Spawns a background task to analyze an imported media file.
/// Does not block the import response — failures are logged, not propagated.
fn spawn_media_analysis(
    app: &AppUseCase,
    file_id: String,
    path: PathBuf,
    title_id: String,
    required_audio_languages: Vec<String>,
) {
    let media_files = Arc::clone(&app.services.media_files);
    let wanted_items = Arc::clone(&app.services.wanted_items);
    let release_attempts = Arc::clone(&app.services.release_attempts);
    tokio::spawn(async move {
        run_media_analysis(
            media_files,
            wanted_items,
            release_attempts,
            file_id,
            path,
            title_id,
            required_audio_languages,
        )
        .await;
    });
}

pub(crate) async fn run_media_analysis(
    media_files: Arc<dyn crate::MediaFileRepository>,
    wanted_items: Arc<dyn crate::WantedItemRepository>,
    release_attempts: Arc<dyn crate::ReleaseAttemptRepository>,
    file_id: String,
    path: PathBuf,
    title_id: String,
    required_audio_languages: Vec<String>,
) {
    let analysis = match scryer_mediainfo::analyze_file(&path) {
        Ok(a) => a,
        Err(err) => {
            tracing::warn!(error = %err, file_id = %file_id, "media analysis failed");
            let _ = media_files
                .mark_scan_failed(&file_id, &err.to_string())
                .await;
            return;
        }
    };

    if !scryer_mediainfo::is_valid_video(&analysis) {
        tracing::warn!(
            path = %path.display(),
            file_id = %file_id,
            "imported file is not a valid video — deleting and blocklisting"
        );

        // Move to recycle bin (or delete if recycle bin disabled)
        let recycle_config = crate::recycle_bin::config_from_file_path(&path);
        let manifest = crate::recycle_bin::RecycleManifest {
            recycled_at: chrono::Utc::now().to_rfc3339(),
            original_path: path.display().to_string(),
            size_bytes: tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0),
            title_id: Some(title_id.clone()),
            reason: "invalid_file".to_string(),
        };
        if let Err(err) = crate::recycle_bin::recycle_file(&recycle_config, &path, manifest).await {
            tracing::warn!(error = %err, path = %path.display(), "failed to recycle invalid file from disk");
        }

        // Extract the grabbed release title from the wanted item so we can blocklist it
        let release_title = wanted_items
            .get_wanted_item_for_title(&title_id, None)
            .await
            .ok()
            .flatten()
            .and_then(|w| w.grabbed_release)
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .and_then(|v| v["title"].as_str().map(str::to_owned));

        let _ = release_attempts
            .record_release_attempt(
                Some(title_id.clone()),
                None,
                release_title,
                crate::ReleaseDownloadAttemptOutcome::Failed,
                Some("imported file is not a valid video".to_string()),
                None,
            )
            .await;

        // Delete the media_files DB record
        let _ = media_files.delete_media_file(&file_id).await;

        // Reset the wanted item so it re-searches
        if let Ok(Some(item)) = wanted_items
            .get_wanted_item_for_title(&title_id, None)
            .await
        {
            let _ = wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "wanted",
                    None,
                    None,
                    item.search_count,
                    None,
                    None,
                )
                .await;
        }

        return;
    }

    // Language verification: if the quality profile requires specific audio languages,
    // check that the file actually contains them. Missing languages trigger the same
    // delete/blocklist/reset flow as fake-file detection.
    if !required_audio_languages.is_empty() {
        let missing = missing_audio_languages(&required_audio_languages, &analysis.audio_languages);

        if !missing.is_empty() {
            let msg = format!(
                "imported file is missing required audio language(s): {}",
                missing.join(", ")
            );
            tracing::warn!(
                path = %path.display(),
                file_id = %file_id,
                missing = ?missing,
                "{}",
                msg
            );

            let recycle_config = crate::recycle_bin::config_from_file_path(&path);
            let manifest = crate::recycle_bin::RecycleManifest {
                recycled_at: chrono::Utc::now().to_rfc3339(),
                original_path: path.display().to_string(),
                size_bytes: tokio::fs::metadata(&path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0),
                title_id: Some(title_id.clone()),
                reason: "language_mismatch".to_string(),
            };
            if let Err(err) =
                crate::recycle_bin::recycle_file(&recycle_config, &path, manifest).await
            {
                tracing::warn!(error = %err, path = %path.display(), "failed to recycle language-mismatch file from disk");
            }

            let release_title = wanted_items
                .get_wanted_item_for_title(&title_id, None)
                .await
                .ok()
                .flatten()
                .and_then(|w| w.grabbed_release)
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                .and_then(|v| v["title"].as_str().map(str::to_owned));

            let _ = release_attempts
                .record_release_attempt(
                    Some(title_id.clone()),
                    None,
                    release_title,
                    crate::ReleaseDownloadAttemptOutcome::Failed,
                    Some(msg),
                    None,
                )
                .await;

            let _ = media_files.delete_media_file(&file_id).await;

            if let Ok(Some(item)) = wanted_items
                .get_wanted_item_for_title(&title_id, None)
                .await
            {
                let _ = wanted_items
                    .update_wanted_item_status(
                        &item.id,
                        "wanted",
                        None,
                        None,
                        item.search_count,
                        None,
                        None,
                    )
                    .await;
            }

            return;
        }
    }

    // Store analysis results on the media file record
    let dto = crate::MediaFileAnalysis {
        video_codec: analysis.video_codec,
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format,
        video_frame_rate: analysis.video_frame_rate,
        video_profile: analysis.video_profile,
        audio_codec: analysis.audio_codec,
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages: analysis.audio_languages,
        audio_streams: analysis
            .audio_streams
            .into_iter()
            .map(|s| crate::AudioStreamDetail {
                codec: s.codec,
                channels: s.channels,
                language: s.language,
                bitrate_kbps: s.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: analysis.subtitle_languages,
        subtitle_codecs: analysis.subtitle_codecs,
        subtitle_streams: analysis
            .subtitle_streams
            .into_iter()
            .map(|s| crate::SubtitleStreamDetail {
                codec: s.codec,
                language: s.language,
                name: s.name,
                forced: s.forced,
                default: s.default,
            })
            .collect(),
        has_multiaudio: analysis.has_multiaudio,
        duration_seconds: analysis.duration_seconds,
        container_format: analysis.container_format,
        raw_json: analysis.raw_json,
    };

    if let Err(err) = media_files.update_media_file_analysis(&file_id, dto).await {
        tracing::warn!(error = %err, file_id = %file_id, "failed to store media analysis");
        let _ = media_files
            .mark_scan_failed(&file_id, &err.to_string())
            .await;
    }
}

#[cfg(test)]
#[path = "app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
