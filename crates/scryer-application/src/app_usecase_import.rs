use crate::{
    ActivityChannel, ActivityKind, ActivitySeverity, AppError, AppResult, AppUseCase,
    ImportArtifact, WantedCompleteTransition,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_release_metadata, render_rename_template, require,
};
use chrono::Utc;
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DownloadQueueItem, DownloadQueueState,
    Entitlement, EventType, Id, ImportDecision, ImportResult, ImportSkipReason, ImportStatus,
    ImportType, MediaFacet, NotificationEventType, Title, User, is_video_file,
};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .read_setting_string_value("subtitles.auto_download_on_import", None)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("true");
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}

/// Retry a previously failed import, optionally with an archive password.
pub async fn retry_failed_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    password: Option<&str>,
) -> AppResult<ImportResult> {
    crate::require(actor, &Entitlement::ManageTitle)?;

    let record = app
        .services
        .imports
        .get_import_by_id(import_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("import {import_id}")))?;

    if record.status != ImportStatus::Failed {
        return Err(AppError::Validation(format!(
            "import {} has status '{}', only failed imports can be retried",
            import_id,
            record.status.as_str()
        )));
    }

    let completed: CompletedDownload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Repository(format!("failed to deserialize import payload: {e}")))?;

    // Reset to processing
    app.services
        .update_import_status_and_notify(import_id, ImportStatus::Processing, None)
        .await?;

    let started_at = Utc::now();
    match run_import(app, actor, import_id, &completed, started_at, password).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
                Some(ImportSkipReason::PasswordRequired)
            } else {
                None
            };
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Failed,
                skip_reason,
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
                .update_import_status_and_notify(import_id, ImportStatus::Failed, result_json)
                .await;
            Ok(result)
        }
    }
}

const SERIES_PATH_KEY: &str = "series.path";
const RENAME_TEMPLATE_SERIES_GLOBAL_KEY: &str = "rename.template.series.global";

/// Called from the download queue poller on every tick (currently 2 seconds).
/// Filters completed items, checks dedup, fetches CompletedDownload data, and triggers import.
///
/// Returns the set of `download_client_item_id`s that were actually processed
/// (imported, already-imported, or permanently non-importable). Callers should
/// only suppress future retries for these IDs — items skipped due to transient
/// conditions (e.g. no matching CompletedDownload yet, empty dest_dir) are NOT
/// included so they can be retried on the next snapshot.
pub async fn try_import_completed_downloads(
    app: &AppUseCase,
    actor: &User,
    items: &[DownloadQueueItem],
) -> HashSet<String> {
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
            item.import_status.is_none() || item.import_status == Some(ImportStatus::Failed)
        })
        .collect();

    if completed_items.is_empty() {
        return HashSet::new();
    }

    let mut processed_ids: HashSet<String> = HashSet::new();

    tracing::info!(
        count = completed_items.len(),
        items = %completed_items.iter().map(|i| format!("{}({})", i.title_name, i.download_client_item_id)).collect::<Vec<_>>().join(", "),
        "import: found completed items to evaluate"
    );

    // Fetch completed downloads from the download client (single RPC call)
    let completed_downloads = match app
        .services
        .download_client
        .list_completed_downloads()
        .await
    {
        Ok(downloads) => {
            tracing::debug!(
                count = downloads.len(),
                ids = %downloads.iter().map(|d| d.download_client_item_id.as_str()).collect::<Vec<_>>().join(", "),
                "import: fetched completed downloads from client"
            );
            downloads
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to fetch completed downloads for import");
            return HashSet::new();
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
            Ok(true) => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: skipping already-imported download"
                );
                processed_ids.insert(source_ref.clone());
                continue;
            }
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
            None => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: no matching CompletedDownload from client history (item may still be processing or status != Completed)"
                );
                continue;
            }
        };

        // Skip if dest_dir is empty
        if completed.dest_dir.is_empty() {
            tracing::info!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: skipping download with empty dest_dir"
            );
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
                Ok(Some(submission)) if submission_has_scryer_origin(&submission) => {
                    let mut patched = completed.clone();
                    patched.parameters = vec![
                        ("*scryer_title_id".to_string(), submission.title_id),
                        ("*scryer_facet".to_string(), submission.facet),
                    ];
                    if let Some(coll_id) = submission.collection_id {
                        patched
                            .parameters
                            .push(("*scryer_collection_id".to_string(), coll_id));
                    }
                    patched
                }
                Ok(Some(_)) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: ignoring stub download_submissions row without scryer origin metadata"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                Ok(None) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: no scryer origin — not in parameters or download_submissions table"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                Err(error) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        error = %error,
                        "import: download_submissions lookup failed"
                    );
                    continue;
                }
            }
        };

        let facet_label = extract_parameter(&completed.parameters, "*scryer_facet")
            .unwrap_or_else(|| "unknown".to_string());
        tracing::info!(
            source_ref = %source_ref,
            title = %item.title_name,
            dest_dir = %completed.dest_dir,
            facet = %facet_label,
            "import: triggering import for completed download"
        );
        processed_ids.insert(source_ref.clone());
        let import_start = std::time::Instant::now();
        match import_completed_download(app, actor, &completed).await {
            Ok(result) => {
                if matches!(
                    result.decision,
                    ImportDecision::Failed | ImportDecision::Rejected
                ) {
                    tracing::warn!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import failed for {}",
                        completed.name
                    );
                } else if matches!(result.decision, ImportDecision::Unmatched) {
                    tracing::debug!(
                        decision = ?result.decision,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import unmatched for {}",
                        completed.name
                    );
                } else {
                    tracing::info!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        dest_path = ?result.dest_path,
                        "import completed for {}",
                        completed.name
                    );
                }
                let completed_facet = facet_for_completed_download(&completed);
                let should_remove_completed = if matches!(result.decision, ImportDecision::Imported)
                {
                    match completed_facet.as_ref() {
                        Some(facet) => {
                            app.should_remove_completed_download(facet, &completed.client_id)
                                .await
                        }
                        None => false,
                    }
                } else {
                    false
                };
                let should_remove_failed = if matches!(
                    result.decision,
                    ImportDecision::Failed | ImportDecision::Rejected
                ) {
                    match completed_facet.as_ref() {
                        Some(facet) => {
                            app.should_remove_failed_download(facet, &completed.client_id)
                                .await
                        }
                        None => false,
                    }
                } else {
                    false
                };
                metrics::counter!("scryer_imports_total", "decision" => result.decision.as_str(), "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
                if should_remove_completed {
                    remove_download_history_item(app, &completed, "completed").await;
                } else if should_remove_failed {
                    remove_download_history_item(app, &completed, "failed").await;
                }
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

    processed_ids
}

fn facet_for_completed_download(completed: &CompletedDownload) -> Option<MediaFacet> {
    match extract_parameter(&completed.parameters, "*scryer_facet")
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("tv") | Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}

async fn remove_download_history_item(
    app: &AppUseCase,
    completed: &CompletedDownload,
    outcome_label: &str,
) {
    if let Err(error) = app
        .services
        .download_client
        .delete_queue_item(&completed.download_client_item_id, true)
        .await
    {
        tracing::warn!(
            client_id = completed.client_id.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            outcome = outcome_label,
            error = %error,
            "failed to delete completed download from client history"
        );
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
            ImportType::TvDownload
        } else {
            ImportType::MovieDownload
        }
    };
    let import_id = app
        .services
        .imports
        .queue_import_request(
            completed.client_type.clone(),
            source_ref.clone(),
            import_type.as_str().to_string(),
            serde_json::to_string(completed).unwrap_or_default(),
        )
        .await?;

    // If the source directory no longer exists, the files were already moved
    // by a previous import (possibly under a different source_ref). Mark as
    // skipped so the poller never retries this entry.
    let source_path = std::path::Path::new(&completed.dest_dir);
    if !source_path.exists() {
        tracing::debug!(
            source_ref,
            dest_dir = %completed.dest_dir,
            "import: source directory no longer exists, no files to import"
        );
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: None,
            source_path: completed.dest_dir.clone(),
            dest_path: None,
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        let _ = app
            .services
            .update_import_status_and_notify(&import_id, ImportStatus::Skipped, result_json)
            .await;
        return Ok(result);
    }

    // Mark as processing
    app.services
        .update_import_status_and_notify(&import_id, ImportStatus::Processing, None)
        .await?;

    // From here on, any error must update the import record to "failed" rather than
    // propagating via `?`. Otherwise the record stays "processing" indefinitely.
    match run_import(app, actor, &import_id, completed, started_at, None).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
                Some(ImportSkipReason::PasswordRequired)
            } else {
                None
            };
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Failed,
                skip_reason,
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
                .update_import_status_and_notify(&import_id, ImportStatus::Failed, result_json)
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
    archive_password: Option<&str>,
) -> AppResult<ImportResult> {
    // 2. TITLE MATCHING
    let mut title = None;
    let parsed_completed_name = parse_release_metadata(&completed.name);
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

    if title.is_none() && parsed_completed_name.episode.is_none() {
        let titles = app.services.titles.list(None, None).await?;
        title = find_monitored_movie_title_from_release(&titles, &parsed_completed_name);
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
                .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
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
        MediaFacet::Movie | MediaFacet::Series | MediaFacet::Anime
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
            .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    // 3. FIND VIDEO FILES (extract archives first if needed)
    let dest_dir = Path::new(&completed.dest_dir);
    let is_series = matches!(title.facet, MediaFacet::Series | MediaFacet::Anime);
    let extracted_dir =
        crate::archive_extractor::extract_archives_if_needed(dest_dir, archive_password).await?;
    let effective_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
    let video_files = find_video_files(effective_dir, is_series)?;

    if video_files.is_empty() {
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
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
            .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    // Check if this is an interstitial movie import (anime franchise movie → Season 00)
    let interstitial_collection_id =
        extract_parameter(&completed.parameters, "*scryer_collection_id");

    // Branch on facet: movies import the single largest file, series import all episode files
    let result = if let Some(ref coll_id) = interstitial_collection_id {
        import_interstitial_movie_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
            coll_id,
        )
        .await
    } else if is_series {
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
    };

    // Clean up extracted archive directory if we created one
    if let Some(ref dir) = extracted_dir {
        crate::archive_extractor::cleanup_extracted_dir(dir).await;
    }

    result
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
    let full_folder_path = PathBuf::from(&media_root).join(&title_folder);

    // Persist the folder path on the title so delete can find it deterministically.
    if title.folder_path.is_none() {
        let _ = app
            .services
            .titles
            .set_folder_path(&title.id, &full_folder_path.to_string_lossy())
            .await;
    }

    let dest_path = full_folder_path.join(&rendered_filename);

    // Pre-import checks
    let existing_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    let quality_profile = resolve_import_quality_profile(app, title).await;
    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: &source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &parsed,
        existing_files: &existing_files,
    };
    let verdict = crate::import_checks::run_import_checks(&check_ctx);
    if let crate::import_checks::ImportVerdict::Reject { reason, code } = verdict {
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            &source_video,
            "movie",
            "rejected",
            Some(code),
            None,
            &[],
        )
        .await;
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
            .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    // Upgrade check: if there are existing files, score and compare
    if !existing_files.is_empty() {
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            &quality_profile,
            &parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            title.runtime_minutes,
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;

        // Find the best existing file by acquisition_score
        if let Some(existing_file) = existing_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            if new_score > old_score {
                let media_root_opt = crate::recycle_bin::media_root_for_title(app, title).await;
                let recycle_config =
                    crate::recycle_bin::resolve_recycle_config(app, media_root_opt.as_deref())
                        .await;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &parsed,
                    &quality_profile,
                    completed,
                    new_score,
                    old_score,
                    &[],
                    false,
                    &recycle_config,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
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
                        mark_wanted_completed(app, &title.id, None, None).await;
                        let result_json = serde_json::to_string(&result).ok();
                        app.services
                            .update_import_status_and_notify(
                                import_id,
                                ImportStatus::Completed,
                                result_json,
                            )
                            .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection
                                .skip_reason
                                .as_ref()
                                .map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.services
                            .update_import_status_and_notify(
                                import_id,
                                ImportStatus::Skipped,
                                result_json,
                            )
                            .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "upgrade failed, falling through to normal import"
                        );
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

    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    match crate::post_download_gate::evaluate_imported_file_gate(
        app,
        title,
        &parsed,
        &quality_profile,
        &dest_path,
        file_result.size_bytes as i64,
        !existing_files.is_empty(),
        existing_score,
        false,
    )
    .await
    {
        crate::post_download_gate::ImportedFileGateDecision::Rejected(rejection) => {
            crate::post_download_gate::reject_imported_file(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                &dest_path,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "already_present",
                rejection
                    .skip_reason
                    .as_ref()
                    .map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_path: source_video.to_string_lossy().to_string(),
                dest_path: Some(dest_path.to_string_lossy().to_string()),
                file_size_bytes: Some(file_result.size_bytes as i64),
                link_type: Some(file_result.strategy),
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.services
                .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            Ok(result)
        }
        crate::post_download_gate::ImportedFileGateDecision::Accepted(accepted) => {
            // Write NFO sidecar (non-fatal, opt-in)
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

            // Compute acquisition score with mediainfo rescore
            let acq_score = crate::post_download_gate::compute_acquisition_score(
                &parsed,
                &accepted,
                &quality_profile,
                title,
                file_result.size_bytes as i64,
                !existing_files.is_empty(),
            );

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
                acquisition_score: Some(acq_score),
                ..Default::default()
            };
            let imported_media_file_id = match app
                .services
                .media_files
                .insert_media_file(&media_file_input)
                .await
            {
                Ok(file_id) => {
                    crate::post_download_gate::persist_media_analysis_result(
                        &app.services.media_files,
                        &file_id,
                        &accepted,
                    )
                    .await;
                    maybe_trigger_subtitle_search(app, &title.id, &file_id);
                    Some(file_id)
                }
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

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "imported",
                None,
                imported_media_file_id.as_deref(),
                &[],
            )
            .await;

            // Create collection record (so the movie overview UI can show the file)
            let collection = Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: parsed.quality.clone(),
                ordered_path: Some(dest_path.to_string_lossy().to_string()),
                narrative_order: None,
                first_episode_number: None,
                last_episode_number: None,
                interstitial_movie: None,
                specials_movies: vec![],
                interstitial_season_episode: None,
                monitored: true,
                created_at: Utc::now(),
            };
            if let Err(err) = app.services.shows.create_collection(collection).await {
                tracing::warn!(
                    error = %err,
                    title_id = %title.id,
                    "failed to create collection record"
                );
            }

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
                .update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
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
            {
                let mut meta = HashMap::new();
                meta.insert("title_name".to_string(), serde_json::json!(title.name));
                if let Some(ref poster) = title.poster_url {
                    meta.insert("poster_url".to_string(), serde_json::json!(poster));
                }
                let envelope = crate::activity::NotificationEnvelope {
                    event_type: NotificationEventType::Download,
                    title: format!("Downloaded: {}", title.name),
                    body: event_message.clone(),
                    facet: Some(format!("{:?}", title.facet).to_lowercase()),
                    metadata: meta,
                };
                app.services
                    .record_activity_event_with_notification(
                        Some(actor.id.clone()),
                        Some(title.id.clone()),
                        None,
                        ActivityKind::MovieDownloaded,
                        event_message,
                        ActivitySeverity::Success,
                        vec![ActivityChannel::WebUi],
                        envelope,
                    )
                    .await?;
            }

            Ok(result)
        }
    }
}

// ---------------------------------------------------------------------------
// Interstitial movie import: anime franchise movie → Season 00 of the series
// ---------------------------------------------------------------------------

async fn import_interstitial_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
    collection_id: &str,
) -> AppResult<ImportResult> {
    // Load the interstitial collection
    let collection = match app
        .services
        .shows
        .get_collection_by_id(collection_id)
        .await?
    {
        Some(c) => c,
        None => {
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                source_path: completed.dest_dir.clone(),
                dest_path: None,
                file_size_bytes: None,
                link_type: None,
                error_message: Some(format!("interstitial collection {collection_id} not found")),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.services
                .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    let movie = match collection.interstitial_movie.as_ref() {
        Some(m) => m,
        None => {
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                source_path: completed.dest_dir.clone(),
                dest_path: None,
                file_size_bytes: None,
                link_type: None,
                error_message: Some("interstitial collection has no movie metadata".to_string()),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.services
                .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let (media_root, _rename_template) = resolve_import_paths(app, title).await?;

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

    // Build Season 00 filename using the TVDB episode number from the collection
    let season_episode = collection
        .interstitial_season_episode
        .as_deref()
        .unwrap_or("S00E01");
    let rendered_filename = format!(
        "{} - {} - {}.{}",
        title.name, season_episode, movie.name, ext
    );

    // Build destination: <media_root>/<title folder>/Season 00/<filename>
    let year_str = title.year.map(|y| format!(" ({})", y)).unwrap_or_default();
    let title_folder = format!("{}{}", title.name, year_str);
    let dest_path = PathBuf::from(&media_root)
        .join(&title_folder)
        .join("Season 00")
        .join(&rendered_filename);

    // Pre-import checks (same as movie import)
    let existing_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    // Filter to files in this collection's Season 00 path
    let collection_files: Vec<_> = existing_files
        .iter()
        .filter(|f| {
            collection
                .ordered_path
                .as_deref()
                .is_some_and(|p| f.file_path == p)
        })
        .cloned()
        .collect();
    let quality_profile = resolve_import_quality_profile(app, title).await;

    // Upgrade check: if there's an existing file for this interstitial, score and compare
    if !collection_files.is_empty() {
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            &quality_profile,
            &parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            Some(movie.runtime_minutes),
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;

        if let Some(existing_file) = collection_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            if new_score > old_score {
                let media_root_opt = crate::recycle_bin::media_root_for_title(app, title).await;
                let recycle_config =
                    crate::recycle_bin::resolve_recycle_config(app, media_root_opt.as_deref())
                        .await;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &parsed,
                    &quality_profile,
                    completed,
                    new_score,
                    old_score,
                    &[],
                    false,
                    &recycle_config,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
                        tracing::info!(
                            title = %title.name,
                            movie = %movie.name,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "interstitial movie file upgraded"
                        );
                        mark_wanted_completed_for_collection(app, &title.id, collection_id).await;
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
                        let result_json = serde_json::to_string(&result).ok();
                        app.services
                            .update_import_status_and_notify(
                                import_id,
                                ImportStatus::Completed,
                                result_json,
                            )
                            .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection
                                .skip_reason
                                .as_ref()
                                .map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.services
                            .update_import_status_and_notify(
                                import_id,
                                ImportStatus::Skipped,
                                result_json,
                            )
                            .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "interstitial upgrade failed, falling through to normal import"
                        );
                    }
                }
            } else {
                // New file is not better — skip
                persist_file_import_artifact(
                    app,
                    import_id,
                    completed,
                    title.id.as_str(),
                    &source_video,
                    "movie",
                    "already_present",
                    Some("existing_better_or_equal"),
                    None,
                    &[],
                )
                .await;
                let result = ImportResult {
                    import_id: import_id.to_string(),
                    decision: ImportDecision::Skipped,
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    title_id: Some(title.id.clone()),
                    source_path: source_video.to_string_lossy().to_string(),
                    dest_path: Some(dest_path.to_string_lossy().to_string()),
                    file_size_bytes: Some(source_size),
                    link_type: None,
                    error_message: Some(format!(
                        "new score {new_score} not better than existing {old_score}"
                    )),
                    started_at,
                    completed_at: Utc::now(),
                };
                let result_json = serde_json::to_string(&result).ok();
                app.services
                    .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                    .await?;
                return Ok(result);
            }
        }
    }

    // Ensure Season 00 directory exists
    if let Some(parent) = dest_path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(error = %err, path = %parent.display(), "failed to create Season 00 directory");
    }

    // Import file (hardlink or copy)
    let file_result = app
        .services
        .file_importer
        .import_file(&source_video, &dest_path)
        .await?;

    // Post-download gate (quality profile check)
    match crate::post_download_gate::evaluate_imported_file_gate(
        app,
        title,
        &parsed,
        &quality_profile,
        &dest_path,
        file_result.size_bytes as i64,
        !collection_files.is_empty(),
        collection_files
            .iter()
            .max_by_key(|f| f.acquisition_score.unwrap_or(0))
            .and_then(|f| f.acquisition_score),
        false,
    )
    .await
    {
        crate::post_download_gate::ImportedFileGateDecision::Rejected(rejection) => {
            crate::post_download_gate::reject_imported_file(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                &dest_path,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "already_present",
                rejection
                    .skip_reason
                    .as_ref()
                    .map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_path: source_video.to_string_lossy().to_string(),
                dest_path: Some(dest_path.to_string_lossy().to_string()),
                file_size_bytes: Some(file_result.size_bytes as i64),
                link_type: Some(file_result.strategy),
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.services
                .update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
        crate::post_download_gate::ImportedFileGateDecision::Accepted(accepted) => {
            let acq_score = crate::post_download_gate::compute_acquisition_score(
                &parsed,
                &accepted,
                &quality_profile,
                title,
                file_result.size_bytes as i64,
                !collection_files.is_empty(),
            );

            // Persist media analysis from the gate
            let imported_media_file_id = if let Ok(file_id) = app
                .services
                .media_files
                .insert_media_file(&crate::InsertMediaFileInput {
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
                    acquisition_score: Some(acq_score),
                    ..Default::default()
                })
                .await
            {
                crate::post_download_gate::persist_media_analysis_result(
                    &app.services.media_files,
                    &file_id,
                    &accepted,
                )
                .await;
                maybe_trigger_subtitle_search(app, &title.id, &file_id);
                Some(file_id)
            } else {
                None
            };

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "imported",
                None,
                imported_media_file_id.as_deref(),
                &[],
            )
            .await;
        }
    }

    // Update the interstitial collection with the file path
    if let Err(err) = app
        .services
        .shows
        .update_collection(
            collection_id,
            None,
            None,
            None,
            Some(dest_path.to_string_lossy().to_string()),
            None,
            None,
            None,
        )
        .await
    {
        tracing::warn!(
            error = %err,
            collection_id = collection_id,
            "failed to update interstitial collection ordered_path"
        );
    }

    // Write Jellyfin-compatible NFO with airsbefore_season
    let nfo_enabled = app
        .read_setting_string_value("nfo.write_on_import.anime", None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = crate::nfo::render_interstitial_movie_nfo(
            movie,
            season_episode,
            &collection.collection_index,
        );
        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %err,
                path = %nfo_path.display(),
                "failed to write interstitial movie NFO sidecar"
            );
        }
    }

    // Mark wanted item as completed (by collection_id)
    mark_wanted_completed_for_collection(app, &title.id, collection_id).await;

    // Spawn post-processing
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
        .update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    // Emit activity event
    let event_message = format!(
        "Imported interstitial movie '{}' ({}) for '{}'",
        movie.name, season_episode, title.name
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
    {
        let mut meta = HashMap::new();
        meta.insert("title_name".to_string(), serde_json::json!(title.name));
        meta.insert("movie_name".to_string(), serde_json::json!(movie.name));
        if let Some(ref poster) = title.poster_url {
            meta.insert("poster_url".to_string(), serde_json::json!(poster));
        }
        let envelope = crate::activity::NotificationEnvelope {
            event_type: NotificationEventType::Download,
            title: format!("Downloaded: {} - {}", title.name, movie.name),
            body: event_message.clone(),
            facet: Some("anime".to_string()),
            metadata: meta,
        };
        app.services
            .record_activity_event_with_notification(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                None,
                ActivityKind::MovieDownloaded,
                event_message,
                ActivitySeverity::Success,
                vec![ActivityChannel::WebUi],
                envelope,
            )
            .await?;
    }

    Ok(result)
}

/// Mark a wanted item as completed by collection_id (for interstitial movies).
async fn mark_wanted_completed_for_collection(
    app: &AppUseCase,
    title_id: &str,
    collection_id: &str,
) {
    // Find the wanted item by iterating (since we don't have a direct lookup by collection_id)
    match app
        .services
        .wanted_items
        .list_wanted_items(
            Some("wanted"),
            Some("interstitial_movie"),
            Some(title_id),
            100,
            0,
        )
        .await
    {
        Ok(items) => {
            for item in items {
                if item.collection_id.as_deref() == Some(collection_id) {
                    let now = Utc::now().to_rfc3339();
                    let _ = app
                        .services
                        .wanted_items
                        .transition_wanted_to_completed(&WantedCompleteTransition {
                            id: item.id.clone(),
                            last_search_at: Some(now),
                            search_count: item.search_count,
                            current_score: item.current_score,
                            grabbed_release: item.grabbed_release.clone(),
                        })
                        .await;
                    return;
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = title_id,
                collection_id = collection_id,
                "failed to look up wanted item for interstitial movie"
            );
        }
    }
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
    let full_folder_path = PathBuf::from(&media_root).join(&title_folder);

    if title.folder_path.is_none() {
        let _ = app
            .services
            .titles
            .set_folder_path(&title.id, &full_folder_path.to_string_lossy())
            .await;
    }

    let quality_profile = resolve_import_quality_profile(app, title).await;

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

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut rejected_count: usize = 0;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejection_skip_reason: Option<ImportSkipReason> = None;

    for source_video in video_files {
        match import_single_episode_file(
            app,
            actor,
            title,
            import_id,
            &media_root,
            &rename_template,
            &title_folder,
            completed,
            source_video,
            &quality_profile,
            nfo_enabled,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported) => imported_count += 1,
            Ok(EpisodeImportOutcome::Skipped) => skipped_count += 1,
            Ok(EpisodeImportOutcome::Rejected {
                message,
                skip_reason,
            }) => {
                rejected_count += 1;
                last_error = Some(message);
                last_rejection_skip_reason = skip_reason;
            }
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

    if imported_count > 0 {
        write_series_sidecars(app, title, &media_root, &title_folder, nfo_enabled).await;
    }

    let (decision, status, skip_reason) = if imported_count > 0 {
        (ImportDecision::Imported, ImportStatus::Completed, None)
    } else if failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if rejected_count > 0 {
        (
            ImportDecision::Rejected,
            ImportStatus::Failed,
            last_rejection_skip_reason,
        )
    } else {
        // All files skipped (no parseable episode info, already imported, etc.)
        // — this is a permanent condition, not worth retrying.
        (ImportDecision::Skipped, ImportStatus::Skipped, None)
    };

    let error_message = if failed_count > 0 || skipped_count > 0 || rejected_count > 0 {
        Some(format!(
            "{imported_count} imported, {skipped_count} skipped, {rejected_count} rejected, {failed_count} failed{}",
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
        skip_reason,
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
        .update_import_status_and_notify(import_id, status, result_json)
        .await?;

    if imported_count > 0 {
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
        {
            let mut meta = HashMap::new();
            meta.insert("title_name".to_string(), serde_json::json!(title.name));
            if let Some(ref poster) = title.poster_url {
                meta.insert("poster_url".to_string(), serde_json::json!(poster));
            }
            let envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::ImportComplete,
                title: format!("Import complete: {}", title.name),
                body: event_message.clone(),
                facet: Some(format!("{:?}", title.facet).to_lowercase()),
                metadata: meta,
            };
            app.services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(title.id.clone()),
                    None,
                    ActivityKind::SeriesEpisodeImported,
                    event_message,
                    ActivitySeverity::Success,
                    vec![ActivityChannel::WebUi],
                    envelope,
                )
                .await?;
        }
    }

    Ok(result)
}

enum EpisodeImportOutcome {
    Imported,
    Skipped,
    Rejected {
        message: String,
        skip_reason: Option<ImportSkipReason>,
    },
}

/// Import a single episode video file: parse, gate, import, and link.
async fn import_single_episode_file(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    media_root: &str,
    rename_template: &str,
    title_folder: &str,
    completed: &CompletedDownload,
    source_video: &Path,
    quality_profile: &crate::QualityProfile,
    nfo_enabled: bool,
) -> AppResult<EpisodeImportOutcome> {
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
        Some(ep) if ep.air_date.is_some() => ep,
        Some(ep) if ep.release_type == crate::ParsedEpisodeReleaseType::SeasonPack => ep,
        _ => {
            tracing::debug!(
                file = %source_video.display(),
                "skipping file with no parseable episode info"
            );
            return Ok(EpisodeImportOutcome::Skipped);
        }
    };

    let season = ep_meta.season.unwrap_or(1);
    let season_str = season.to_string();

    // Resolve target episodes early so we can enrich rename tokens with DB
    // metadata (e.g. absolute_number from TVDB).
    let target_episodes = resolve_target_episodes(app, title, ep_meta, &season_str).await;
    let target_episode_ids: Vec<String> = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    let is_filler = target_episodes.iter().any(|episode| episode.is_filler);

    // Build rename tokens and destination path
    let ep_num_str = ep_meta
        .episode_numbers
        .first()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let abs_str = ep_meta.absolute_episode.map(|n| n.to_string()).or_else(|| {
        target_episodes
            .first()
            .and_then(|ep| ep.absolute_number.clone())
    });
    let episode_title = target_episodes.first().and_then(|ep| ep.title.as_deref());
    let dest_path = episode_import_dest_path(
        title,
        &parsed,
        &ext,
        media_root,
        title_folder,
        rename_template,
        season as u32,
        &ep_num_str,
        abs_str.as_deref(),
        episode_title,
        None,
    );

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
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source_video,
            "episode",
            "rejected",
            Some(code),
            None,
            &target_episodes,
        )
        .await;
        return Ok(EpisodeImportOutcome::Skipped);
    }

    // Upgrade check for episodes: find existing file for same dest path
    if !existing_files.is_empty() {
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            quality_profile,
            &parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            title.runtime_minutes,
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;
        let dest_str = dest_path.to_string_lossy();

        // Find an existing file at the same dest path (or matching episode)
        if let Some(existing_file) = existing_files
            .iter()
            .find(|file| file.file_path == dest_str.as_ref())
        {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            if new_score > old_score {
                let recycle_config =
                    crate::recycle_bin::resolve_recycle_config(app, Some(media_root)).await;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    source_video,
                    &dest_path,
                    &parsed,
                    quality_profile,
                    completed,
                    new_score,
                    old_score,
                    &target_episode_ids,
                    is_filler,
                    &recycle_config,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            source_video,
                            "episode",
                            "imported",
                            Some("upgrade"),
                            None,
                            &target_episodes,
                        )
                        .await;
                        tracing::info!(
                            title = %title.name,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "episode file upgraded"
                        );
                        for episode_id in &target_episode_ids {
                            mark_wanted_completed(app, &title.id, Some(episode_id), None).await;
                        }
                        return Ok(EpisodeImportOutcome::Imported);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            source_video,
                            "episode",
                            "already_present",
                            rejection
                                .skip_reason
                                .as_ref()
                                .map(ImportSkipReason::as_str),
                            None,
                            &target_episodes,
                        )
                        .await;
                        return Ok(EpisodeImportOutcome::Rejected {
                            message: rejection.message,
                            skip_reason: rejection.skip_reason,
                        });
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "episode upgrade failed, falling through to normal import"
                        );
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

    let existing_dest_path = dest_path.to_string_lossy().to_string();
    let existing_score = existing_files
        .iter()
        .find(|file| file.file_path == existing_dest_path.as_str())
        .and_then(|file| file.acquisition_score);
    let gate_result = crate::post_download_gate::evaluate_imported_file_gate(
        app,
        title,
        &parsed,
        quality_profile,
        &dest_path,
        file_result.size_bytes as i64,
        existing_files
            .iter()
            .any(|file| file.file_path == existing_dest_path.as_str()),
        existing_score,
        is_filler,
    )
    .await;

    let accepted = match gate_result {
        crate::post_download_gate::ImportedFileGateDecision::Rejected(rejection) => {
            crate::post_download_gate::reject_imported_file(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                &dest_path,
                &target_episode_ids,
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "already_present",
                rejection
                    .skip_reason
                    .as_ref()
                    .map(ImportSkipReason::as_str),
                None,
                &target_episodes,
            )
            .await;
            return Ok(EpisodeImportOutcome::Rejected {
                message: rejection.message,
                skip_reason: rejection.skip_reason,
            });
        }
        crate::post_download_gate::ImportedFileGateDecision::Accepted(accepted) => accepted,
    };

    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        if let Some(episode) = target_episodes.first() {
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

    let has_existing = existing_files
        .iter()
        .any(|file| file.file_path == existing_dest_path.as_str());
    let acq_score = crate::post_download_gate::compute_acquisition_score(
        &parsed,
        &accepted,
        quality_profile,
        title,
        file_result.size_bytes as i64,
        has_existing,
    );

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
        acquisition_score: Some(acq_score),
        ..Default::default()
    };
    let media_file_id = app
        .services
        .media_files
        .insert_media_file(&media_file_input)
        .await?;
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.media_files,
        &media_file_id,
        &accepted,
    )
    .await;
    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        source_video,
        "episode",
        "imported",
        None,
        Some(media_file_id.as_str()),
        &target_episodes,
    )
    .await;
    maybe_trigger_subtitle_search(app, &title.id, &media_file_id);

    for episode in &target_episodes {
        if let Err(err) = app
            .services
            .media_files
            .link_file_to_episode(&media_file_id, &episode.id)
            .await
        {
            tracing::warn!(error = %err, episode_id = %episode.id, "failed to link file to episode");
        }
        mark_wanted_completed(app, &title.id, Some(&episode.id), None).await;
    }

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
        season: Some(season),
        episode: ep_meta.episode_numbers.first().copied(),
        quality: parsed.quality.clone(),
    });

    Ok(EpisodeImportOutcome::Imported)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve media root path and rename template for a title's facet.
pub(crate) async fn resolve_import_paths(
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

/// Compute the destination path for an episode import using the canonical
/// token set: base tokens from parsed release metadata, overridden by the
/// explicit episode values supplied by the caller.
///
/// `ep_num_str` may be empty to leave `{episode}` blank (anime absolute-only
/// files where no per-season episode number is known).
/// `quality_override` replaces the filename-parsed quality token when the
/// caller supplies an explicit label (e.g. manual import).
pub(crate) fn episode_import_dest_path(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
    media_root: &str,
    title_folder: &str,
    rename_template: &str,
    season_num: u32,
    ep_num_str: &str,
    absolute_number: Option<&str>,
    episode_title: Option<&str>,
    quality_override: Option<&str>,
) -> PathBuf {
    let mut tokens = build_rename_tokens(title, parsed, ext);
    tokens.insert("season".to_string(), season_num.to_string());
    tokens.insert("season_order".to_string(), season_num.to_string());
    tokens.insert("episode".to_string(), ep_num_str.to_string());
    tokens.insert(
        "absolute_episode".to_string(),
        absolute_number.unwrap_or("").to_string(),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title.unwrap_or("").to_string(),
    );
    if let Some(q) = quality_override {
        tokens.insert("quality".to_string(), q.to_string());
    }
    let rendered = render_rename_template(rename_template, &tokens);
    if use_season_folders(title) {
        let season_folder = format!("Season {:02}", season_num);
        PathBuf::from(media_root)
            .join(title_folder)
            .join(&season_folder)
            .join(&rendered)
    } else {
        PathBuf::from(media_root).join(title_folder).join(&rendered)
    }
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
pub(crate) async fn mark_wanted_completed(
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
                .transition_wanted_to_completed(&WantedCompleteTransition {
                    id: wanted.id.clone(),
                    last_search_at: Some(now_str),
                    search_count: wanted.search_count,
                    current_score: score,
                    grabbed_release: wanted.grabbed_release.clone(),
                })
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

async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::QualityProfile {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    match app
        .resolve_quality_profile(
            &title.tags,
            title.imdb_id.as_deref(),
            tvdb_id,
            Some(category_hint),
        )
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                "failed to resolve quality profile, using default"
            );
            crate::default_quality_profile_for_search()
        }
    }
}

pub(crate) async fn resolve_target_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
) -> Vec<scryer_domain::Episode> {
    let mut episodes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target_season = if ep_meta.special_kind.is_some() || ep_meta.season == Some(0) {
        "0".to_string()
    } else {
        season_str.to_string()
    };

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        match app.services.shows.list_collections_for_title(&title.id).await {
            Ok(collections) => {
                let mut matches = Vec::new();
                for collection in collections {
                    match app.services.shows.list_episodes_for_collection(&collection.id).await {
                        Ok(collection_episodes) => {
                            matches.extend(collection_episodes.into_iter().filter(|episode| {
                                episode.title_id == title.id
                                    && episode.air_date.as_deref() == Some(air_date_str.as_str())
                            }));
                        }
                        Err(err) => tracing::warn!(error = %err, "daily episode lookup failed during import"),
                    }
                }

                matches.sort_by_key(|episode| {
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });

                if let Some(part) = ep_meta.daily_part {
                    let part_index = part.saturating_sub(1) as usize;
                    if let Some(episode) = matches.into_iter().nth(part_index)
                        && seen.insert(episode.id.clone())
                    {
                        episodes.push(episode);
                    }
                } else {
                    for episode in matches {
                        if seen.insert(episode.id.clone()) {
                            episodes.push(episode);
                        }
                    }
                }
            }
            Err(err) => tracing::warn!(error = %err, "daily collection lookup failed during import"),
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let episode_str = episode_number.to_string();
        match app
            .services
            .shows
            .find_episode_by_title_and_numbers(&title.id, &target_season, &episode_str)
            .await
        {
            Ok(Some(episode)) => {
                if seen.insert(episode.id.clone()) {
                    episodes.push(episode);
                }
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id,
                    season = %season_str,
                    episode = %episode_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => tracing::warn!(error = %err, "episode lookup failed during import"),
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        match app.services.shows.list_collections_for_title(&title.id).await {
            Ok(collections) => {
                for collection in collections
                    .into_iter()
                    .filter(|collection| collection.collection_index == target_season)
                {
                    match app.services.shows.list_episodes_for_collection(&collection.id).await {
                        Ok(collection_episodes) => {
                            let mut collection_episodes: Vec<_> = collection_episodes
                                .into_iter()
                                .filter(|episode| {
                                    episode.title_id == title.id
                                        && episode.season_number.as_deref()
                                            == Some(target_season.as_str())
                                })
                                .collect();
                            collection_episodes.sort_by_key(|episode| {
                                episode
                                    .episode_number
                                    .as_deref()
                                    .and_then(|value| value.parse::<u32>().ok())
                                    .unwrap_or(u32::MAX)
                            });
                            for episode in collection_episodes {
                                if seen.insert(episode.id.clone()) {
                                    episodes.push(episode);
                                }
                            }
                        }
                        Err(err) => tracing::warn!(error = %err, "season episode lookup failed during import"),
                    }
                }
            }
            Err(err) => tracing::warn!(error = %err, "season collection lookup failed during import"),
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let episode_str = special_number.to_string();
            match app
                .services
                .shows
                .find_episode_by_title_and_numbers(&title.id, "0", &episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        special = %episode_str,
                        "no matching special episode found during import"
                    );
                }
                Err(err) => tracing::warn!(error = %err, "special episode lookup failed during import"),
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            let absolute_episode_str = absolute_number.to_string();
            match app
                .services
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &absolute_episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        absolute = absolute_number,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import")
                }
            }
        }
    }

    episodes
}

async fn write_series_sidecars(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    media_root: &str,
    title_folder: &str,
    nfo_enabled: bool,
) {
    if nfo_enabled {
        let tvshow_nfo_path = PathBuf::from(media_root)
            .join(title_folder)
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
        let plexmatch_path = PathBuf::from(media_root)
            .join(title_folder)
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

/// Returns true if the download was submitted by scryer (has *scryer_title_id parameter).
fn has_scryer_origin(params: &[(String, String)]) -> bool {
    params.iter().any(|(k, _)| k == "*scryer_title_id")
}

fn submission_has_scryer_origin(submission: &crate::DownloadSubmission) -> bool {
    !submission.title_id.trim().is_empty()
}

fn extract_parameter(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

async fn persist_file_import_artifact(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    title_id: &str,
    source_path: &Path,
    media_kind: &str,
    result: &str,
    reason_code: Option<&str>,
    imported_media_file_id: Option<&str>,
    episodes: &[scryer_domain::Episode],
) {
    let relative_path = source_path
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .filter(|path| !path.is_empty());
    let normalized_file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| source_path.to_string_lossy().to_ascii_lowercase());

    let episode_rows: Vec<(Option<String>, Option<i32>, Option<i32>)> = if episodes.is_empty() {
        vec![(None, None, None)]
    } else {
        episodes
            .iter()
            .map(|episode| {
                (
                    Some(episode.id.clone()),
                    episode.season_number.as_deref().and_then(|value| value.parse().ok()),
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                )
            })
            .collect()
    };

    for (episode_id, season_number, episode_number) in episode_rows {
        let artifact = ImportArtifact {
            id: Id::new().0,
            source_system: completed.client_type.clone(),
            source_ref: completed.download_client_item_id.clone(),
            import_id: Some(import_id.to_string()),
            relative_path: relative_path.clone(),
            normalized_file_name: normalized_file_name.clone(),
            media_kind: media_kind.to_string(),
            title_id: Some(title_id.to_string()),
            episode_id,
            season_number,
            episode_number,
            result: result.to_string(),
            reason_code: reason_code.map(str::to_string),
            imported_media_file_id: imported_media_file_id.map(str::to_string),
            created_at: Utc::now(),
        };
        if let Err(error) = app.services.import_artifacts.insert_artifact(artifact).await {
            tracing::warn!(
                error = %error,
                import_id,
                source_ref = %completed.download_client_item_id,
                file = %source_path.display(),
                "failed to persist import artifact"
            );
        }
    }
}

fn normalized_release_title_candidates(parsed: &crate::ParsedReleaseMetadata) -> Vec<String> {
    let raw_candidates = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    raw_candidates
        .into_iter()
        .map(|title| crate::app_usecase_rss::normalize_for_matching(&title))
        .filter(|title| !title.is_empty())
        .fold(Vec::<String>::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

fn title_matches_normalized_candidate(title: &Title, candidate: &str) -> bool {
    if crate::app_usecase_rss::normalize_for_matching(&title.name) == candidate {
        return true;
    }

    title
        .aliases
        .iter()
        .any(|alias| crate::app_usecase_rss::normalize_for_matching(alias) == candidate)
}

fn find_movie_title_by_external_ids<'a>(
    titles: &[&'a Title],
    parsed: &crate::ParsedReleaseMetadata,
) -> Option<&'a Title> {
    if let Some(parsed_imdb_id) = parsed.imdb_id.as_deref().and_then(normalize_imdb_id) {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("imdb")
                        && normalize_imdb_id(&external_id.value).as_deref()
                            == Some(parsed_imdb_id.as_str())
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    if let Some(parsed_tmdb_id) = parsed.tmdb_id.map(|id| id.to_string()) {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("tmdb")
                        && external_id.value.trim() == parsed_tmdb_id
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    None
}

fn find_movie_title_by_name<'a>(
    titles: &[&'a Title],
    parsed: &crate::ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let candidates = normalized_release_title_candidates(parsed);
    if candidates.is_empty() {
        return None;
    }

    let mut year_matches = Vec::<&Title>::new();
    let mut any_matches = Vec::<&Title>::new();

    for candidate in candidates {
        for title in titles {
            if !title_matches_normalized_candidate(title, &candidate) {
                continue;
            }

            if !any_matches.iter().any(|existing| existing.id == title.id) {
                any_matches.push(*title);
            }

            if let Some(year) = parsed.year
                && title.year.map(|value| value as u32) == Some(year)
                && !year_matches.iter().any(|existing| existing.id == title.id)
            {
                year_matches.push(*title);
            }
        }
    }

    if year_matches.len() == 1 {
        return year_matches.into_iter().next();
    }

    if any_matches.len() == 1 {
        return any_matches.into_iter().next();
    }

    year_matches
        .into_iter()
        .next()
        .or_else(|| any_matches.into_iter().next())
}

fn find_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &crate::ParsedReleaseMetadata,
) -> Option<Title> {
    let monitored_movies = titles
        .iter()
        .filter(|title| title.monitored && title.facet == MediaFacet::Movie)
        .collect::<Vec<_>>();

    find_movie_title_by_external_ids(&monitored_movies, parsed)
        .or_else(|| find_movie_title_by_name(&monitored_movies, parsed))
        .cloned()
}

fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw_imdb_id)
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
            if suggested_episode_id.is_none()
                && let Some(abs) = ep_meta.absolute_episode
            {
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
        let ep_num_str = episode.episode_number.clone().unwrap_or_default();

        let dest_path = episode_import_dest_path(
            &title,
            &parsed,
            ext,
            &media_root,
            &title_folder,
            &rename_template,
            season_num,
            &ep_num_str,
            episode.absolute_number.as_deref(),
            episode.title.as_deref(),
            mapping.quality.as_deref(),
        );

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
                    maybe_trigger_subtitle_search(app, &title.id, &mf_id);
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
    {
        let mut meta = HashMap::new();
        meta.insert("title_name".to_string(), serde_json::json!(title.name));
        if let Some(ref poster) = title.poster_url {
            meta.insert("poster_url".to_string(), serde_json::json!(poster));
        }
        let envelope = crate::activity::NotificationEnvelope {
            event_type: NotificationEventType::ImportComplete,
            title: format!("Import complete: {}", title.name),
            body: event_message.clone(),
            facet: Some(format!("{:?}", title.facet).to_lowercase()),
            metadata: meta,
        };
        app.services
            .record_activity_event_with_notification(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                None,
                ActivityKind::SeriesEpisodeImported,
                event_message,
                ActivitySeverity::Success,
                vec![ActivityChannel::WebUi],
                envelope,
            )
            .await?;
    }

    Ok(results)
}

#[cfg(test)]
#[path = "app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
