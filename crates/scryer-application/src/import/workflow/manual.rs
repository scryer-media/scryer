const MANUAL_IMPORT_POLLER_INTERVAL_SECONDS: u64 = 2;
const MANUAL_IMPORT_RECOVERY_BATCH_SIZE: usize = 500;
/// How far back the completed-manual-import recovery sweep looks. Older
/// records have long since terminalized (or their source is gone), and a
/// stale record must not be matched against a fresh download that merely
/// reuses the same client item id.
const MANUAL_IMPORT_RECOVERY_WINDOW_HOURS: i64 = 24;
const MANUAL_IMPORT_SOURCE_UNAVAILABLE: &str = "download is no longer available for manual import";
const MANUAL_MOVIE_NO_PRIMARY_FILE: &str =
    "no primary movie file to import: every mapped video is named as a sample";
// Opaque files have no release-name semantics for the manual candidate UI.
// Keep small samples and sidecars out of that expanded discovery surface while
// leaving known video files (including legitimate short specials) untouched.
const OPAQUE_MANUAL_IMPORT_PROBE_MIN_BYTES: u64 = 16 * 1024 * 1024;
pub async fn start_background_manual_import_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("manual_import_poller", token);
    tracing::info!(
        interval_seconds = MANUAL_IMPORT_POLLER_INTERVAL_SECONDS,
        "manual import poller started"
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        MANUAL_IMPORT_POLLER_INTERVAL_SECONDS,
    ));
    // Completed manual-import records already reconciled against their
    // tracked download in this process; each record is decided once.
    let mut manual_import_recovery_memo: HashMap<String, ManualImportRecoveryMemo> = HashMap::new();
    let mut in_flight: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    match app
        .services
        .workflow
        .imports
        .recover_stale_processing_imports_for_type(
            ImportType::ManualImport,
            IMPORT_STALE_RECOVERY_SECONDS,
        )
        .await
    {
        Ok(recovered) if recovered > 0 => {
            worker.warn_recovered("recover_stale_manual_imports", recovered);
        }
        Err(error) => worker.warn_error("recover_stale_manual_imports", &error),
        _ => {}
    }

    loop {
        if !worker.wait_for_tick(&mut interval).await {
            for (id, task) in in_flight.drain() {
                task.abort();
                let _ = task.await;
                mark_manual_import_reconciliation(
                    &app,
                    &id,
                    "manual import was interrupted during shutdown; inspect source and destination",
                )
                .await;
            }
            return;
        }

        let finished = in_flight
            .iter()
            .filter_map(|(id, task)| task.is_finished().then_some(id.clone()))
            .collect::<Vec<_>>();
        for id in finished {
            if let Some(task) = in_flight.remove(&id)
                && let Err(error) = task.await
            {
                tracing::warn!(import_id = %id, error = %error, "manual import task failed");
                mark_manual_import_reconciliation(
                    &app,
                    &id,
                    "manual import worker ended unexpectedly; inspect source and destination",
                )
                .await;
            }
        }

        recover_completed_manual_imports(&app, &worker, &mut manual_import_recovery_memo).await;

        let pending = match app
            .services
            .workflow
            .imports
            .list_pending_imports_for_type(ImportType::ManualImport)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                worker.warn_error("list_pending_manual_imports", &error);
                continue;
            }
        };

        for record in pending {
            if in_flight.contains_key(&record.id) {
                continue;
            }
            if manual_import_record_requires_reconciliation(&record) {
                continue;
            }
            if !manual_import_retry_is_due(&record, Utc::now()) {
                continue;
            }
            let id = record.id.clone();
            let task_app = app.clone();
            let task = tokio::spawn(async move {
                process_pending_manual_import(task_app, record).await;
            });
            in_flight.insert(id, task);
        }
    }
}

async fn mark_manual_import_reconciliation(app: &AppUseCase, import_id: &str, message: &str) {
    let record = match app
        .services
        .workflow
        .imports
        .get_import_by_id(import_id)
        .await
    {
        Ok(Some(record)) if record.status == ImportStatus::Processing => record,
        Ok(_) => return,
        Err(error) => {
            tracing::error!(import_id, error = %error, "failed to load interrupted manual import");
            return;
        }
    };
    let Ok(payload) = serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
    else {
        tracing::error!(import_id, "interrupted manual import payload is invalid");
        return;
    };
    let file_results = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<ManualImportExecutionResult>(json).ok())
        .map(|result| result.file_results)
        .unwrap_or_default();
    let result_json = manual_import_pending_result_json(
        import_id,
        &payload,
        format!("Manual reconciliation required: {message}"),
        true,
        0,
        None,
        file_results,
    );
    if let Err(error) = app
        .update_import_status_and_notify(import_id, ImportStatus::Pending, result_json)
        .await
    {
        tracing::error!(import_id, error = %error, "failed to preserve interrupted manual import");
    }
}

pub(crate) fn manual_import_record_requires_reconciliation(
    record: &scryer_domain::ImportRecord,
) -> bool {
    let Some(result) = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<ManualImportExecutionResult>(json).ok())
    else {
        return false;
    };
    result.requires_reconciliation
        || result.error_message.is_some_and(|message| {
            message
                .to_ascii_lowercase()
                .starts_with("manual reconciliation required:")
        })
}

fn manual_import_retry_is_due(record: &scryer_domain::ImportRecord, now: DateTime<Utc>) -> bool {
    record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<ManualImportExecutionResult>(json).ok())
        .and_then(|result| result.next_retry_at)
        .is_none_or(|next_retry_at| next_retry_at <= now)
}

async fn process_pending_manual_import(app: AppUseCase, record: scryer_domain::ImportRecord) {
    let payload = match serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json) {
        Ok(payload) => payload,
        Err(error) => {
            let result_json = manual_import_result_json(
                &record.id,
                &ManualImportRequestPayload {
                    requested_by_user_id: None,
                    title_id: None,
                    download_client_item_id: record.source_ref.clone(),
                    client_id: None,
                    client_type: record.source_system.clone(),
                    files: Vec::new(),
                    selection_id: None,
                    release_evidence: None,
                    trusted_source_root: None,
                    archive_workspace_root: None,
                    requested_at: record.created_at.clone(),
                },
                ImportStatus::Failed,
                Some(ImportErrorCode::Unknown),
                Some(format!("invalid manual import payload: {error}")),
                Vec::new(),
            );
            let _ = app
                .update_import_status_and_notify(&record.id, ImportStatus::Failed, result_json)
                .await;
            return;
        }
    };

    let previous_result = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<ManualImportExecutionResult>(json).ok());
    let previous_file_results = previous_result
        .as_ref()
        .map(|result| result.file_results.clone())
        .unwrap_or_default();

    let outcome = match execute_queued_manual_import_with_outcome(&app, &record.id, &payload).await
    {
        Ok(result) => result,
        Err(AppError::ManualReconciliationRequired(message)) => QueuedManualImportOutcome {
            status: ImportStatus::Pending,
            result_json: manual_import_pending_result_json(
                &record.id,
                &payload,
                format!("Manual reconciliation required: {message}"),
                true,
                0,
                None,
                previous_file_results.clone(),
            ),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: Some(payload.files.len()),
            prior_import_proven: false,
        },
        Err(AppError::ImportEvidenceUnavailable(message)) => {
            let retry_attempts = previous_result
                .as_ref()
                .map_or(1, |result| result.retry_attempts.saturating_add(1));
            let next_retry_at = Utc::now() + manual_import_recovery_retry_delay(retry_attempts);
            QueuedManualImportOutcome {
                status: ImportStatus::Pending,
                result_json: manual_import_pending_result_json(
                    &record.id,
                    &payload,
                    format!("Import evidence is temporarily unavailable: {message}"),
                    false,
                    retry_attempts,
                    Some(next_retry_at),
                    previous_file_results,
                ),
                files_imported_this_pass: 0,
                completed: None,
                title_id: payload.title_id.clone(),
                expected_mapping_count: Some(payload.files.len()),
                prior_import_proven: false,
            }
        }
        Err(error) => QueuedManualImportOutcome {
            status: ImportStatus::Failed,
            result_json: manual_import_result_json(
                &record.id,
                &payload,
                ImportStatus::Failed,
                Some(classify_manual_import_error_message(&error.to_string())),
                Some(error.to_string()),
                Vec::new(),
            ),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: None,
            prior_import_proven: false,
        },
    };

    if let Err(error) = app
        .update_import_status_and_notify(&record.id, outcome.status, outcome.result_json.clone())
        .await
    {
        tracing::warn!(import_id = %record.id, error = %error, "failed to finalize manual import request");
        return;
    }

    let has_successful_import = outcome.files_imported_this_pass > 0
        || outcome.status == ImportStatus::Completed
        || outcome.prior_import_proven;
    let terminalized = if has_successful_import {
        let canonical_download_id = match app
            .services
            .workflow
            .imports
            .canonical_download_id_for_import(&record.id)
            .await
        {
            Ok(canonical_download_id) => canonical_download_id,
            Err(error) => {
                tracing::warn!(import_id = %record.id, error = %error, "failed to resolve manual import canonical identity");
                None
            }
        };
        if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
            let tracked_id = crate::tracked_downloads::tracked_download_id(
                payload.client_id.as_deref(),
                &payload.client_type,
                &payload.download_client_item_id,
            );
            let reconciliation = if outcome.prior_import_proven {
                handle
                    .mark_imported_for_download(tracked_id, canonical_download_id)
                    .await
                    .map(|()| true)
            } else {
                handle
                    .reconcile_manual_import_for_download(
                        tracked_id,
                        canonical_download_id,
                        outcome.files_imported_this_pass,
                        outcome.expected_mapping_count,
                    )
                    .await
            };
            match reconciliation {
                Ok(terminalized) => terminalized,
                Err(error) => {
                    tracing::warn!(import_id = %record.id, error = %error, "failed to reconcile manual import");
                    false
                }
            }
        } else {
            false
        }
    } else {
        false
    };

    if terminalized {
        let source_identity = ClientJobLocator::for_import_artifact(
            payload.client_id.as_deref(),
            &payload.client_type,
            &payload.download_client_item_id,
        );
        if let Err(error) = app
            .services
            .workflow
            .imports
            .delete_manual_import_selections_for_source(&source_identity)
            .await
        {
            tracing::warn!(import_id = %record.id, error = %error, "failed to clean up terminal manual import selections");
        }
    }

    maybe_remove_completed_manual_import_download(
        &app,
        outcome.completed.as_ref(),
        outcome.title_id.as_deref(),
        terminalized,
    )
    .await;
}
async fn maybe_remove_completed_manual_import_download(
    app: &AppUseCase,
    completed: Option<&CompletedDownload>,
    title_id: Option<&str>,
    imported: bool,
) {
    if !imported {
        return;
    }

    let Some(completed) = completed else {
        return;
    };

    let (library_id, resolved_facet) = cleanup_routing_scope_for_title_id(app, title_id).await;
    let facet = resolved_facet.or_else(|| facet_for_completed_download(completed));

    let Some(facet) = facet else {
        return;
    };

    // No tracked row on this path, so the client entry is assumed present and
    // the seeding gate decides on the client's own evidence. A torrent still
    // working off its goal is left alone; the tracked poller picks it up and
    // parks it in `ImportedSeeding`.
    let _ = reconcile_terminal_download_cleanup(
        app,
        None,
        &completed.client_id,
        &completed.client_type,
        &completed.download_client_item_id,
        library_id.as_deref(),
        Some(&facet),
        TrackedDownloadState::Imported,
        TerminalFailureOrigin::ClientFailure,
        None,
        true,
        // No tracked row here, so the gate reads the published snapshot.
        None,
        // Outside the reconcile tick: no shared prefetch, per-row reads.
        None,
    )
    .await;
}
// ---------------------------------------------------------------------------
// Manual import: preview & execute
// ---------------------------------------------------------------------------

/// A single file in a manual import preview with auto-detected episode info.
#[derive(Clone, Debug)]
pub struct ManualImportVideoFacts {
    pub container_format: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub duration_seconds: Option<i32>,
}

struct ManualImportFilePreview {
    file_path: String,
    file_name: String,
    size_bytes: i64,
    video_facts: Option<ManualImportVideoFacts>,
    quality: Option<String>,
    parsed_season: Option<u32>,
    parsed_episodes: Vec<u32>,
    suggested_episode_id: Option<String>,
    suggested_episode_label: Option<String>,
    suggested_series_movie_link_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManualImportSeriesMovieTarget {
    pub series_movie_link_id: String,
    pub movie_title: String,
    pub year: Option<i32>,
    pub runtime_minutes: Option<i32>,
}

/// Internal file metadata used to construct a server-owned manual-import selection.
struct ManualImportPreview {
    files: Vec<ManualImportFilePreview>,
}

/// A file selected from a server-owned manual-import selection. Its source path remains internal.
pub struct ManualImportSelectionFilePreview {
    pub candidate_id: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub video_facts: Option<ManualImportVideoFacts>,
    pub quality: Option<String>,
    pub parsed_season: Option<u32>,
    pub parsed_episodes: Vec<u32>,
    pub suggested_episode_id: Option<String>,
    pub suggested_episode_label: Option<String>,
    pub suggested_series_movie_link_id: Option<String>,
}

pub struct ManualImportSelectionPreview {
    pub selection_id: String,
    pub archive_extraction_needed: bool,
    pub files: Vec<ManualImportSelectionFilePreview>,
    pub available_episodes: Vec<scryer_domain::Episode>,
    pub available_series_movies: Vec<ManualImportSeriesMovieTarget>,
}

/// The only client-controlled portion of a manual-import mapping.
#[derive(Clone, Debug)]
pub struct ManualImportCandidateMapping {
    pub candidate_id: String,
    pub episode_id: Option<String>,
    pub series_movie_link_id: Option<String>,
}

async fn manual_import_preview_targets(
    app: &AppUseCase,
    title_id: &str,
) -> AppResult<(
    Vec<scryer_domain::Episode>,
    Vec<ManualImportSeriesMovieTarget>,
)> {
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(title_id)
        .await?;
    let mut all_episodes = Vec::new();
    for collection in &collections {
        let episodes = app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(&collection.id)
            .await?;
        all_episodes.extend(episodes);
    }

    let series_movies = app
        .services
        .catalog
        .shows
        .list_series_movie_links_for_title(title_id)
        .await?
        .into_iter()
        .map(|link| ManualImportSeriesMovieTarget {
            series_movie_link_id: link.id,
            movie_title: link.movie.title,
            year: link.movie.year,
            runtime_minutes: link.movie.runtime_minutes,
        })
        .collect();

    Ok((all_episodes, series_movies))
}

fn manual_import_source_unavailable() -> AppError {
    AppError::NotFound(MANUAL_IMPORT_SOURCE_UNAVAILABLE.to_string())
}

pub(crate) async fn resolve_current_manual_import_source(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<CompletedDownload> {
    let client_id = client_id.trim();
    let client_type = client_type.trim();
    let download_client_item_id = download_client_item_id.trim();
    let authorized = authorize_manual_import_source(
        app,
        actor,
        client_id,
        client_type,
        download_client_item_id,
        title_id,
    )
    .await?;
    resolve_authorized_manual_import_source(app, &authorized.identity).await
}

struct AuthorizedManualImportSource {
    identity: ClientJobLocator,
    title: scryer_domain::Title,
}

async fn authorize_manual_import_source(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<AuthorizedManualImportSource> {
    if client_id.is_empty() || client_type.is_empty() || download_client_item_id.is_empty() {
        return Err(manual_import_source_unavailable());
    }

    let source_identity =
        ClientJobLocator::new(Some(client_id), client_type, download_client_item_id);
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(manual_import_source_unavailable)?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;

    let submission = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(&source_identity)
        .await?;
    if let Some(submission) = submission
        && !matches!(&submission.scope, crate::SubmissionScope::Orphan)
        && (submission.title_id.trim().is_empty() || submission.title_id != title.id)
    {
        return Err(manual_import_source_unavailable());
    }

    Ok(AuthorizedManualImportSource {
        identity: source_identity,
        title,
    })
}

async fn resolve_authorized_manual_import_source(
    app: &AppUseCase,
    identity: &ClientJobLocator,
) -> AppResult<CompletedDownload> {
    if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
        match handle.completed_source(identity.clone()).await {
            Ok(Some(completed)) if completed_download_matches_source(&completed, identity) => {
                if retained_manual_import_source_is_admitted(app, identity, &completed).await? {
                    return Ok(completed);
                }
                return Err(manual_import_source_unavailable());
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    client_id = ?identity.client_id,
                    client_type = %identity.client_type,
                    download_client_item_id = %identity.item_id,
                    "retained manual-import source lookup failed; falling back to live lookup"
                );
            }
        }
    }

    resolve_live_manual_import_source(app, identity).await
}

async fn retained_manual_import_source_is_admitted(
    app: &AppUseCase,
    identity: &ClientJobLocator,
    completed: &CompletedDownload,
) -> AppResult<bool> {
    let has_scryer_submission = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(identity)
        .await?
        .as_ref()
        .is_some_and(crate::import_parameters::submission_has_scryer_origin);
    Ok(app
        .completed_download_admission(has_scryer_submission, completed, None)
        .await
        == crate::services::CompletedDownloadAdmission::Admitted)
}

async fn resolve_live_manual_import_source(
    app: &AppUseCase,
    identity: &ClientJobLocator,
) -> AppResult<CompletedDownload> {
    let completed = match app
        .resolve_manual_import_source(
            identity.client_id.as_deref(),
            Some(identity.client_type.as_str()),
            identity.item_id.as_str(),
        )
        .await?
    {
        crate::ManualImportSourceResolution::Eligible {
            completed: Some(completed),
        } => *completed,
        _ => return Err(manual_import_source_unavailable()),
    };

    if !completed_download_matches_source(&completed, identity) {
        return Err(manual_import_source_unavailable());
    }

    Ok(completed)
}

fn completed_download_matches_source(
    completed: &CompletedDownload,
    identity: &ClientJobLocator,
) -> bool {
    completed
        .client_id
        .eq_ignore_ascii_case(identity.client_id.as_deref().unwrap_or_default())
        && completed
            .client_type
            .eq_ignore_ascii_case(identity.client_type.as_str())
        && completed.download_client_item_id == identity.item_id
}

fn import_record_proves_prior_import(record: &ImportRecord, current_import_id: &str) -> bool {
    if record.id == current_import_id {
        return false;
    }

    let canonical_result = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<scryer_domain::ImportResult>(json).ok());
    match record.status {
        ImportStatus::Completed if record.import_type != ImportType::ManualImport => true,
        ImportStatus::Completed => canonical_result.is_some_and(|result| {
            result.decision == ImportDecision::Imported
                || result.skip_reason == Some(ImportSkipReason::AlreadyImported)
        }),
        ImportStatus::Skipped => canonical_result
            .is_some_and(|result| result.skip_reason == Some(ImportSkipReason::AlreadyImported)),
        _ => false,
    }
}

async fn manual_import_source_was_already_imported(
    app: &AppUseCase,
    source: &AuthorizedManualImportSource,
    current_import_id: &str,
) -> AppResult<bool> {
    if app
        .services
        .workflow
        .download_submissions
        .get_tracked_state(&source.identity)
        .await?
        .as_deref()
        .and_then(TrackedDownloadState::from_str_opt)
        .is_some_and(TrackedDownloadState::counts_as_imported)
    {
        return Ok(true);
    }
    if source.title.facet != MediaFacet::Movie {
        return Ok(false);
    }

    Ok(app
        .services
        .workflow
        .imports
        .list_imports_for_identities(std::slice::from_ref(&source.identity))
        .await?
        .iter()
        .any(|record| import_record_proves_prior_import(record, current_import_id)))
}

fn source_path_canonical(source_path: &Path) -> AppResult<PathBuf> {
    std::fs::canonicalize(source_path).map_err(|err| {
        AppError::Validation(format!(
            "manual import path is not accessible: {} ({err})",
            source_path.display()
        ))
    })
}

fn source_entry_location_under_parent(source_path: &Path) -> AppResult<PathBuf> {
    let parent = source_path.parent().ok_or_else(|| {
        AppError::Validation(format!(
            "manual import file must have a parent directory: {}",
            source_path.display()
        ))
    })?;
    let file_name = source_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "manual import file must have a file name: {}",
            source_path.display()
        ))
    })?;
    let parent = source_path_canonical(parent)?;
    Ok(parent.join(file_name))
}

fn canonical_manual_import_source_under_trusted_root(
    source_path: &Path,
    trusted_root: &Path,
) -> AppResult<PathBuf> {
    let trusted_root_metadata = std::fs::metadata(trusted_root).map_err(|err| {
        AppError::Validation(format!(
            "manual import source root is not accessible: {} ({err})",
            trusted_root.display()
        ))
    })?;
    let canonical_trusted_root = std::fs::canonicalize(trusted_root).map_err(|err| {
        AppError::Validation(format!(
            "manual import source root is not accessible: {} ({err})",
            trusted_root.display()
        ))
    })?;
    let canonical = std::fs::canonicalize(source_path).map_err(|err| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({err})",
            source_path.display()
        ))
    })?;

    if trusted_root_metadata.is_file() {
        if canonical == canonical_trusted_root {
            return Ok(canonical);
        }
        return Err(AppError::Validation(format!(
            "manual import file is outside the trusted source root: {}",
            source_path.display()
        )));
    }
    if !trusted_root_metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "manual import source root is not a file or directory: {}",
            trusted_root.display()
        )));
    }

    let source_entry_location = source_entry_location_under_parent(source_path)?;
    if source_entry_location != canonical_trusted_root
        && !source_entry_location.starts_with(&canonical_trusted_root)
    {
        return Err(AppError::Validation(format!(
            "manual import file path is outside the trusted source root: {}",
            source_path.display()
        )));
    }

    if canonical != canonical_trusted_root && !canonical.starts_with(&canonical_trusted_root) {
        return Err(AppError::Validation(format!(
            "manual import file is outside the trusted source root: {}",
            source_path.display()
        )));
    }

    let metadata = std::fs::metadata(&canonical).map_err(|err| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({err})",
            source_path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::Validation(format!(
            "manual import source is not a file: {}",
            source_path.display()
        )));
    }
    Ok(canonical)
}

pub(crate) fn validate_manual_import_source_under_trusted_root(
    source_path: &Path,
    trusted_root: &Path,
) -> AppResult<()> {
    canonical_manual_import_source_under_trusted_root(source_path, trusted_root).map(drop)
}

#[derive(Clone, Debug)]
pub(crate) struct QualifiedManualImportVideo {
    pub(crate) source_entry_path: PathBuf,
    pub(crate) canonical_path: PathBuf,
    pub(crate) size_bytes: i64,
    pub(crate) video_facts: Option<ManualImportVideoFacts>,
}

pub(crate) async fn qualify_manual_import_video_candidate(
    source_path: &Path,
    trusted_root: &Path,
) -> AppResult<Option<QualifiedManualImportVideo>> {
    let canonical_path =
        canonical_manual_import_source_under_trusted_root(source_path, trusted_root)?;
    let metadata = std::fs::metadata(&canonical_path).map_err(|error| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({error})",
            source_path.display()
        ))
    })?;
    std::fs::File::open(&canonical_path).map_err(|error| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({error})",
            source_path.display()
        ))
    })?;
    let has_known_video_extension = is_video_file(source_path);
    let has_file_extension = source_path.extension().is_some();
    if metadata.len() == 0 {
        return Ok(None);
    }

    if has_known_video_extension {
        return Ok(Some(QualifiedManualImportVideo {
            source_entry_path: source_path.to_path_buf(),
            canonical_path,
            size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            video_facts: None,
        }));
    }

    if has_file_extension || metadata.len() < OPAQUE_MANUAL_IMPORT_PROBE_MIN_BYTES {
        return Ok(None);
    }

    #[cfg(feature = "runtime-media-analysis")]
    let video_facts = {
        let probe_path = canonical_path.clone();
        let analysis = tokio::task::spawn_blocking(move || {
            crate::nice_thread();
            scryer_mediainfo::analyze_file_with_options(
                &probe_path,
                scryer_mediainfo::AnalyzeOptions {
                    profile: scryer_mediainfo::AnalysisProfile::ContentProbe,
                },
            )
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("manual import media probe failed: {error}"))
        })?;

        match analysis {
            Ok(analysis) if scryer_mediainfo::is_valid_video(&analysis) => {
                Some(ManualImportVideoFacts {
                    container_format: analysis.container_format,
                    video_codec: analysis.video_codec,
                    audio_codec: analysis.audio_codec,
                    video_width: analysis.video_width,
                    video_height: analysis.video_height,
                    duration_seconds: analysis.duration_seconds,
                })
            }
            Ok(_) | Err(scryer_mediainfo::MediaInfoError::Parse(_)) => return Ok(None),
            Err(scryer_mediainfo::MediaInfoError::UnsupportedFormat(_)) => return Ok(None),
            Err(scryer_mediainfo::MediaInfoError::Io(error)) => {
                return Err(AppError::Validation(format!(
                    "manual import file is not accessible: {} ({error})",
                    source_path.display()
                )));
            }
        }
    };

    #[cfg(not(feature = "runtime-media-analysis"))]
    return Ok(None);

    #[cfg(feature = "runtime-media-analysis")]
    Ok(Some(QualifiedManualImportVideo {
        source_entry_path: source_path.to_path_buf(),
        canonical_path,
        size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        video_facts,
    }))
}

async fn discover_manual_import_video_candidates(
    trusted_root: &Path,
) -> AppResult<Vec<QualifiedManualImportVideo>> {
    let root_metadata = std::fs::metadata(trusted_root).map_err(|error| {
        AppError::Validation(format!(
            "manual import source root is not accessible: {} ({error})",
            trusted_root.display()
        ))
    })?;
    let paths = if root_metadata.is_file() {
        vec![trusted_root.to_path_buf()]
    } else {
        crate::filesystem_walk::FilesystemWalker::new()
            .skip_unreadable_subdirectories()
            .confine_to_root()
            .walk(trusted_root)?
            .into_iter()
            .flat_map(|entry| entry.files.into_iter())
            .collect()
    };

    let mut candidates: Vec<QualifiedManualImportVideo> = Vec::new();
    let mut candidate_indexes: std::collections::HashMap<PathBuf, usize> =
        std::collections::HashMap::new();
    let mut first_error = None;
    for path in paths {
        match qualify_manual_import_video_candidate(&path, trusted_root).await {
            Ok(Some(candidate)) => {
                if let Some(index) = candidate_indexes.get(&candidate.canonical_path).copied() {
                    let existing = &mut candidates[index];
                    if is_video_file(&candidate.source_entry_path)
                        && !is_video_file(&existing.source_entry_path)
                    {
                        *existing = candidate;
                    }
                } else {
                    candidate_indexes.insert(candidate.canonical_path.clone(), candidates.len());
                    candidates.push(candidate);
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %error,
                    "skipping unavailable manual import candidate"
                );
                first_error.get_or_insert(error);
            }
        }
    }
    if candidates.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    Ok(candidates)
}

/// Scan a completed download's directory and attempt to auto-match files to episodes.
async fn preview_manual_import(
    app: &AppUseCase,
    source_dir: &Path,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
    available_episodes: &[scryer_domain::Episode],
) -> AppResult<ManualImportPreview> {
    let title_id = title.id.as_str();
    let facet = &title.facet;
    // Series/anime: recursive, no sample filtering — the user maps every file
    // explicitly and small specials are legitimate. Movies: a movie import
    // lands exactly one primary file, so sample-named files never become
    // candidates (offering one only invites a mapping that would be recorded
    // as skipped). Name only, never size: the automatic movie path does not
    // size-filter either, and a legitimately small movie must stay importable
    // by hand.
    let mut video_files = discover_manual_import_video_candidates(source_dir).await?;
    if *facet == MediaFacet::Movie {
        video_files.retain(|candidate| !is_sample_named_file(&candidate.source_entry_path));
    }
    let mut grabbed_episode_ids = match release_evidence.scope() {
        Some(SubmissionScope::Episode { episode_id }) => HashSet::from([episode_id.clone()]),
        Some(SubmissionScope::EpisodeSet { episode_ids }) => episode_ids.iter().cloned().collect(),
        Some(SubmissionScope::Collection { collection_id }) => available_episodes
            .iter()
            .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
            .map(|episode| episode.id.clone())
            .collect(),
        Some(
            SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan,
        )
        | None => HashSet::new(),
    };
    // A verified pack vouches for every standard episode in the seasons its
    // release name declares, not only the season (or episode set) the grab was
    // scoped to: a two-season pack grabbed for season 1 imports its season 2
    // members automatically, so Manual Import keeps their suggestions instead
    // of erasing them as "outside the grab".
    let verified_pack = verified_episode_pack(release_evidence, title);
    if let Some(pack) = verified_pack.as_ref() {
        grabbed_episode_ids.extend(
            available_episodes
                .iter()
                .filter(|episode| pack.vouches_for(episode))
                .map(|episode| episode.id.clone()),
        );
    }
    let grabbed_series_movie_link_id = match release_evidence.scope() {
        Some(SubmissionScope::SeriesMovie {
            series_movie_link_id,
        }) => Some(series_movie_link_id.clone()),
        _ => None,
    };
    let grabbed_fallback_path = (grabbed_episode_ids.len() == 1
        || grabbed_series_movie_link_id.is_some())
    .then(|| {
        video_files
            .iter()
            .max_by_key(|candidate| candidate.size_bytes)
            .map(|candidate| candidate.source_entry_path.clone())
    })
    .flatten();
    let expected_pack_episode_ids = match (verified_pack.as_ref(), release_evidence.scope()) {
        (Some(_), Some(scope)) => {
            expected_episode_ids_from_submission_scope(app, title, scope, false).await
        }
        _ => None,
    };

    // For each file, parse and attempt auto-match
    let mut previews = Vec::new();
    for candidate in &video_files {
        let path = &candidate.source_entry_path;
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown")
            .to_string();

        // File parsing is only for user-facing episode suggestions. It must
        // not become release/quality evidence for the later import.
        let parsed = parsed_release_from_file_stem(path);
        // The quality shown is the one the import will score: the release
        // evidence parsed with the title's canonical context, never the file
        // name.
        let quality = release_evidence_quality_for_title(path, release_evidence, title);

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
                    .catalog
                    .shows
                    .find_episode_by_title_and_numbers(title_id, &season_str, &ep_str)
                    .await
                {
                    suggested_episode_id = Some(episode.id.clone());
                    suggested_episode_label = Some(
                        if episode
                            .absolute_number
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                        {
                            manual_import_episode_label(&episode)
                        } else {
                            format!(
                                "S{:02}E{:02}{}",
                                ep_meta.season.unwrap_or(1),
                                ep_num,
                                episode
                                    .title
                                    .as_ref()
                                    .map(|title| format!(" - {title}"))
                                    .unwrap_or_default()
                            )
                        },
                    );
                }
            }

            // Anime absolute fallback
            if suggested_episode_id.is_none()
                && let Some(abs) = ep_meta.absolute_episode
            {
                let abs_str = abs.to_string();
                if let Ok(Some(episode)) = app
                    .services
                    .catalog
                    .shows
                    .find_episode_by_title_and_absolute_number(title_id, &abs_str)
                    .await
                {
                    suggested_episode_id = Some(episode.id.clone());
                    suggested_episode_label = Some(manual_import_episode_label(&episode));
                }
            }
        }

        if suggested_episode_id.is_none()
            && let Some(episode) = reconcile_unresolved_scene_episode_from_scoped_release(
                app,
                title,
                release_evidence,
                path,
                video_files.len() > 1,
            )
            .await?
        {
            suggested_episode_id = Some(episode.id.clone());
            suggested_episode_label = Some(manual_import_episode_label(&episode));
        }

        if suggested_episode_id.is_none()
            && let Some(pack) = verified_pack.as_ref()
            && let ScopedPackMemberReconciliation::Resolved(episode_id) =
                reconcile_unresolved_pack_member_from_expected_scope(
                    title,
                    pack,
                    path,
                    available_episodes,
                    expected_pack_episode_ids.as_ref(),
                )
            && let Some(episode) = available_episodes
                .iter()
                .find(|episode| episode.id == episode_id)
        {
            suggested_episode_id = Some(episode.id.clone());
            suggested_episode_label = Some(manual_import_episode_label(episode));
        }

        let is_grabbed_fallback_path = grabbed_fallback_path
            .as_ref()
            .is_some_and(|fallback| fallback == path);
        let scoped_suggestion = manual_episode_suggestion_for_grabbed_scope(
            suggested_episode_id.clone(),
            &grabbed_episode_ids,
            manual_grabbed_episode_fallback_applies(
                is_grabbed_fallback_path,
                parsed.episode.as_ref(),
            ),
        );
        if scoped_suggestion != suggested_episode_id {
            suggested_episode_label = scoped_suggestion.as_deref().and_then(|episode_id| {
                available_episodes
                    .iter()
                    .find(|episode| episode.id == episode_id)
                    .map(manual_import_episode_label)
            });
            suggested_episode_id = scoped_suggestion;
        }

        previews.push(ManualImportFilePreview {
            file_path: path_to_stored_string(&candidate.canonical_path),
            file_name,
            size_bytes: candidate.size_bytes,
            video_facts: candidate.video_facts.clone(),
            quality,
            parsed_season,
            parsed_episodes,
            suggested_episode_id,
            suggested_episode_label,
            suggested_series_movie_link_id: grabbed_series_movie_link_id.clone().filter(|_| {
                grabbed_fallback_path
                    .as_ref()
                    .is_some_and(|fallback| fallback == path)
            }),
        });
    }

    Ok(ManualImportPreview { files: previews })
}

#[cfg(test)]
pub(crate) async fn preview_manual_import_suggested_episode_ids_for_tests(
    app: &AppUseCase,
    source_dir: &Path,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
    available_episodes: &[scryer_domain::Episode],
) -> AppResult<Vec<Option<String>>> {
    Ok(preview_manual_import(
        app,
        source_dir,
        title,
        release_evidence,
        available_episodes,
    )
    .await?
    .files
    .into_iter()
    .map(|file| file.suggested_episode_id)
    .collect())
}

/// Whether the preview may pre-select the single grabbed episode for a file.
///
/// The grabbed episode is a starting point only for the largest video that
/// carries no episode evidence of its own. A file that positively parses to an
/// episode — inside or outside the grabbed scope — keeps its own parse (and is
/// left to the user when that parse falls outside the scope): "largest file"
/// says nothing about which episode a file that names a different one holds,
/// and pre-selecting the grabbed episode there would silently import the
/// wrong episode under the right number.
fn manual_grabbed_episode_fallback_applies(
    is_largest_video: bool,
    file_episode: Option<&crate::ParsedEpisodeMetadata>,
) -> bool {
    is_largest_video && file_episode.is_none()
}

/// Constrain a file's parsed episode suggestion to the grabbed scope.
///
/// A parsed suggestion inside the scope (or any suggestion when nothing was
/// grabbed) stands. Otherwise the single grabbed episode is offered only when
/// the caller established that the fallback applies to this file —
/// `manual_grabbed_episode_fallback_applies`, i.e. the largest video with no
/// episode parse of its own, so a file that parsed to an episode outside the
/// scope never gets the grabbed episode — and nothing is suggested otherwise.
fn manual_episode_suggestion_for_grabbed_scope(
    parsed_suggestion: Option<String>,
    grabbed_episode_ids: &HashSet<String>,
    grabbed_episode_fallback_applies: bool,
) -> Option<String> {
    if grabbed_episode_ids.is_empty()
        || parsed_suggestion
            .as_ref()
            .is_some_and(|episode_id| grabbed_episode_ids.contains(episode_id))
    {
        return parsed_suggestion;
    }
    if grabbed_episode_fallback_applies && grabbed_episode_ids.len() == 1 {
        return grabbed_episode_ids.iter().next().cloned();
    }
    None
}

fn manual_import_episode_label(episode: &scryer_domain::Episode) -> String {
    if let Some(absolute) = episode
        .absolute_number
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let normalized_number = |value: Option<&str>| {
            let digits = value
                .unwrap_or_default()
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>();
            if digits.is_empty() {
                "??".to_string()
            } else {
                digits
            }
        };
        let season = normalized_number(episode.season_number.as_deref());
        let number = normalized_number(episode.episode_number.as_deref());
        let title = episode
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!(" — {value}"))
            .unwrap_or_default();
        return format!("S{season:0>2}E{number:0>2} · Absolute {absolute}{title}");
    }

    episode.episode_label.clone().unwrap_or_else(|| {
        let season = episode.season_number.as_deref().unwrap_or("1");
        let number = episode.episode_number.as_deref().unwrap_or("?");
        format!(
            "S{season:0>2}E{number:0>2}{}",
            episode
                .title
                .as_ref()
                .map(|title| format!(" - {title}"))
                .unwrap_or_default()
        )
    })
}

fn reusable_manual_archive_workspace(
    selection: &crate::ManualImportSelection,
) -> Option<(PathBuf, String)> {
    let workspace = selection.archive_workspace_root.as_deref()?;
    if selection.trusted_source_root.trim().is_empty() {
        return None;
    }
    let existing_root = std::fs::canonicalize(stored_path_to_path_buf(workspace)).ok()?;
    (existing_root == stored_path_to_path_buf(&selection.trusted_source_root))
        .then(|| (existing_root, workspace.to_string()))
}

/// Creates a durable, server-owned selection for files from a tracked completed download.
/// The caller receives opaque candidate IDs; canonical source paths remain in workflow storage.
pub async fn begin_manual_import_selection(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
    extract_archives: bool,
) -> AppResult<ManualImportSelectionPreview> {
    let client_id = client_id.trim();
    let client_type = client_type.trim().to_ascii_lowercase();
    let source_ref = download_client_item_id.trim();
    let authorized =
        authorize_manual_import_source(app, actor, client_id, &client_type, source_ref, title_id)
            .await?;
    let completed = resolve_authorized_manual_import_source(app, &authorized.identity).await?;
    let canonical_download_id = match crate::download_identity::resolve_observed_client_job(
        app,
        crate::download_identity::observed_completed_job(&completed),
    )
    .await
    {
        crate::download_identity::ObservedClientJobResolution::Resolved(download_id) => {
            Some(download_id)
        }
        crate::download_identity::ObservedClientJobResolution::Conflict
        | crate::download_identity::ObservedClientJobResolution::BindingAlreadyEnded => {
            return Err(AppError::Validation(
                "manual import source has a conflicting canonical download identity".to_string(),
            ));
        }
        crate::download_identity::ObservedClientJobResolution::Unavailable => None,
    };
    let release_evidence =
        resolve_release_evidence_for_completed_download(app, &completed, None).await?;
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let release_evidence_json = serde_json::to_string(&release_evidence)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let download_root = std::fs::canonicalize(&completed.dest_dir)
        .map_err(|_| manual_import_source_unavailable())?;
    let source_identity = ClientJobLocator::new(
        Some(&completed.client_id),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let selection_id = scryer_domain::Id::new().0;
    let prior_selection = app
        .services
        .workflow
        .imports
        .find_manual_import_selection_for_download(
            canonical_download_id.as_ref(),
            &actor.id,
            title_id,
            &source_identity,
        )
        .await?;
    let prior_candidate_ids = prior_selection
        .as_ref()
        .map(|selection| {
            selection
                .candidates
                .iter()
                .map(|candidate| (candidate.canonical_path.clone(), candidate.id.clone()))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let (all_episodes, available_series_movies) =
        manual_import_preview_targets(app, title_id).await?;

    let mut preview_root = download_root.clone();
    let mut trusted_root = download_root;
    let mut archive_workspace_root = None;
    let mut archive_extraction_needed =
        crate::archive_extractor::archive_extraction_would_be_needed(&preview_root)?;

    if archive_extraction_needed && extract_archives {
        let destination =
            archive_extraction_destination_for_title(app, &selection_id, &authorized.title).await?;
        let extracted_root = {
            let _archive_extraction_permit = app
                .runtime
                .imports
                .execution_coordinator
                .acquire_archive_extraction()
                .await;
            crate::archive_extractor::extract_archives_if_needed(
                &preview_root,
                Some(destination),
                None,
                app.services
                    .integrations
                    .archive_extractor_plugin_provider
                    .available()
                    .cloned(),
            )
            .await?
        }
        .ok_or_else(|| {
            AppError::Validation("archive extraction did not produce importable files".to_string())
        })?;
        trusted_root = std::fs::canonicalize(&extracted_root).map_err(|error| {
            AppError::Validation(format!(
                "extracted archive workspace is not accessible: {} ({error})",
                extracted_root.display()
            ))
        })?;
        preview_root = trusted_root.clone();
        archive_workspace_root = Some(path_to_stored_string(&trusted_root));
        archive_extraction_needed = false;
    } else if archive_extraction_needed
        && let Some((existing_root, workspace)) = prior_selection
            .as_ref()
            .and_then(reusable_manual_archive_workspace)
    {
        preview_root = existing_root.clone();
        trusted_root = existing_root;
        archive_workspace_root = Some(workspace.to_string());
        archive_extraction_needed = false;
    }

    if archive_extraction_needed {
        app.services
            .workflow
            .imports
            .replace_manual_import_selection_for_download(
                crate::ManualImportSelection {
                    id: selection_id.clone(),
                    actor_user_id: actor.id.clone(),
                    title_id: title_id.to_string(),
                    source_identity,
                    canonical_download_id: None,
                    release_evidence_json: Some(release_evidence_json),
                    trusted_source_root: path_to_stored_string(&trusted_root),
                    archive_workspace_root: None,
                    candidates: Vec::new(),
                },
                canonical_download_id.as_ref(),
            )
            .await?;
        return Ok(ManualImportSelectionPreview {
            selection_id,
            archive_extraction_needed: true,
            files: Vec::new(),
            available_episodes: all_episodes,
            available_series_movies,
        });
    }

    let mut candidates = Vec::new();
    let mut files = Vec::new();
    let preview = preview_manual_import(
        app,
        &preview_root,
        &authorized.title,
        &release_evidence,
        &all_episodes,
    )
    .await?;
    for file in preview.files {
        let canonical_path = file.file_path.clone();
        let candidate_id = prior_candidate_ids
            .get(&canonical_path)
            .cloned()
            .unwrap_or_else(|| scryer_domain::Id::new().0);
        candidates.push(crate::ManualImportSelectionCandidate {
            id: candidate_id.clone(),
            canonical_path: canonical_path.clone(),
        });
        files.push(ManualImportSelectionFilePreview {
            candidate_id,
            file_name: file.file_name,
            size_bytes: file.size_bytes,
            video_facts: file.video_facts,
            quality: file.quality,
            parsed_season: file.parsed_season,
            parsed_episodes: file.parsed_episodes,
            suggested_episode_id: file.suggested_episode_id,
            suggested_episode_label: file.suggested_episode_label,
            suggested_series_movie_link_id: file.suggested_series_movie_link_id,
        });
    }

    app.services
        .workflow
        .imports
        .replace_manual_import_selection_for_download(
            crate::ManualImportSelection {
                id: selection_id.clone(),
                actor_user_id: actor.id.clone(),
                title_id: title_id.to_string(),
                source_identity,
                canonical_download_id: None,
                release_evidence_json: Some(release_evidence_json),
                trusted_source_root: path_to_stored_string(&trusted_root),
                archive_workspace_root: archive_workspace_root.clone(),
                candidates,
            },
            canonical_download_id.as_ref(),
        )
        .await?;

    if let Some(previous_workspace) = prior_selection
        .as_ref()
        .and_then(|selection| selection.archive_workspace_root.as_deref())
        .filter(|workspace| Some(*workspace) != archive_workspace_root.as_deref())
    {
        crate::archive_extractor::cleanup_extracted_dir(&stored_path_to_path_buf(
            previous_workspace,
        ))
        .await;
    }

    Ok(ManualImportSelectionPreview {
        selection_id,
        archive_extraction_needed: false,
        files,
        available_episodes: all_episodes,
        available_series_movies,
    })
}
/// A user-specified mapping of one file to one manual import target.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileMapping {
    pub file_path: String,
    #[serde(default)]
    pub episode_id: Option<String>,
    #[serde(default)]
    pub series_movie_link_id: Option<String>,
}

#[derive(Clone, Copy)]
enum ManualImportMappingTarget<'a> {
    Episode(&'a str),
    SeriesMovie(&'a str),
    /// The title itself. A standalone movie has exactly one destination, so
    /// there is no sub-target to name.
    Movie,
}

fn normalize_manual_import_target(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Resolve which target a mapping addresses, given the facet of the title the
/// selection belongs to.
///
/// The facet matters because a MOVIE has no sub-target to name: its file maps
/// to the title. Requiring an `episode_id` or `series_movie_link_id`
/// unconditionally made movies unimportable through this path — the UI's
/// one-click action sends neither (there is nothing it could send) and the
/// request was rejected as invalid, so a completed movie awaiting manual
/// import had no action that could complete it.
fn manual_import_mapping_target<'a>(
    mapping: &'a ManualImportFileMapping,
    facet: &MediaFacet,
) -> AppResult<ManualImportMappingTarget<'a>> {
    let episode_id = normalize_manual_import_target(mapping.episode_id.as_deref());
    let series_movie_link_id =
        normalize_manual_import_target(mapping.series_movie_link_id.as_deref());

    match (episode_id, series_movie_link_id) {
        (Some(episode_id), None) => Ok(ManualImportMappingTarget::Episode(episode_id)),
        (None, Some(series_movie_link_id)) => {
            Ok(ManualImportMappingTarget::SeriesMovie(series_movie_link_id))
        }
        (None, None) if matches!(facet, MediaFacet::Movie) => Ok(ManualImportMappingTarget::Movie),
        (None, None) => Err(AppError::Validation(
            "manual import mapping requires episode_id or series_movie_link_id".to_string(),
        )),
        (Some(_), Some(_)) => Err(AppError::Validation(
            "manual import mapping cannot include both episode_id and series_movie_link_id"
                .to_string(),
        )),
    }
}

/// A movie import lands exactly one file. Among the mappings that address the
/// movie itself, the primary is the largest readable video that is not named
/// as a sample; every other movie mapping is a sample, trailer, or featurette
/// that the user mapped along with it and must be recorded as skipped rather
/// than pushed through the movie importer (which would reject it or, worse,
/// treat it as a replacement for the primary it just imported).
///
/// Sample detection is by name only (`is_sample_named_file`): the automatic
/// movie path never size-filters, and manual import — the user's escape hatch —
/// must not be stricter than it, so a legitimately small movie still imports.
///
/// Returns the index into `files` of the primary mapping, or `None` when no
/// mapped movie file is importable (all sample-named, missing, or unreadable).
/// Ties on size resolve to the earliest mapping so the choice is stable.
fn select_manual_movie_primary_index(
    files: &[ManualImportFileMapping],
    facet: &MediaFacet,
    trusted_source_root: &Path,
) -> Option<usize> {
    let mut primary: Option<(usize, u64)> = None;
    for (index, mapping) in files.iter().enumerate() {
        if !matches!(
            manual_import_mapping_target(mapping, facet),
            Ok(ManualImportMappingTarget::Movie)
        ) {
            continue;
        }
        let source = stored_path_to_path_buf(&mapping.file_path);
        if validate_manual_import_source_under_trusted_root(&source, trusted_source_root).is_err()
            || !source.is_file()
            || is_sample_named_file(&source)
        {
            continue;
        }
        let Ok(size) = std::fs::metadata(&source).map(|metadata| metadata.len()) else {
            continue;
        };
        if primary.is_none_or(|(_, primary_size)| size > primary_size) {
            primary = Some((index, size));
        }
    }
    primary.map(|(index, _)| index)
}

pub(crate) fn validate_manual_import_candidate_mapping_targets(
    files: &[ManualImportCandidateMapping],
    facet: &MediaFacet,
) -> AppResult<()> {
    if files.is_empty() {
        return Err(AppError::Validation(
            "at least one manual import candidate is required".to_string(),
        ));
    }
    let mut candidate_ids = std::collections::HashSet::new();
    for mapping in files {
        let candidate_id = mapping.candidate_id.trim();
        if candidate_id.is_empty() {
            return Err(AppError::Validation(
                "manual import candidate id is required".to_string(),
            ));
        }
        if !candidate_ids.insert(candidate_id) {
            return Err(AppError::Validation(
                "manual import candidate ids must be unique".to_string(),
            ));
        }
        let target = ManualImportFileMapping {
            file_path: String::new(),
            episode_id: mapping.episode_id.clone(),
            series_movie_link_id: mapping.series_movie_link_id.clone(),
        };
        manual_import_mapping_target(&target, facet)?;
    }
    Ok(())
}

pub(crate) async fn validate_manual_import_candidate_mapping_scope(
    app: &AppUseCase,
    title_id: &str,
    files: &[ManualImportCandidateMapping],
) -> AppResult<()> {
    for mapping in files {
        validate_manual_import_target_scope(
            app,
            title_id,
            mapping.episode_id.as_deref(),
            mapping.series_movie_link_id.as_deref(),
        )
        .await?;
    }

    Ok(())
}

async fn validate_manual_import_target_scope(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    series_movie_link_id: Option<&str>,
) -> AppResult<()> {
    if let Some(episode_id) = normalize_manual_import_target(episode_id) {
        let episode = app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "manual import episode target is unavailable: {episode_id}"
                ))
            })?;
        if episode.title_id != title_id {
            return Err(AppError::Validation(format!(
                "manual import episode target {episode_id} does not belong to title {title_id}"
            )));
        }
    }

    if let Some(series_movie_link_id) = normalize_manual_import_target(series_movie_link_id) {
        let link = app
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(series_movie_link_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "manual import series movie target is unavailable: {series_movie_link_id}"
                ))
            })?;
        if link.series_title_id != title_id {
            return Err(AppError::Validation(format!(
                "manual import series movie target {series_movie_link_id} does not belong to title {title_id}"
            )));
        }
    }

    Ok(())
}

/// Per-file result of a manual import execution.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileResult {
    pub file_path: String,
    #[serde(default)]
    pub episode_id: Option<String>,
    #[serde(default)]
    pub series_movie_link_id: Option<String>,
    pub success: bool,
    /// The mapping was deliberately not imported (for example a non-primary
    /// movie file such as a sample or featurette). A skipped mapping is neither
    /// a success nor a failure: it does not block the import from completing.
    #[serde(default)]
    pub skipped: bool,
    pub dest_path: Option<String>,
    pub error_code: Option<ImportErrorCode>,
    pub error_message: Option<String>,
}

fn manual_import_file_result(
    mapping: &ManualImportFileMapping,
    success: bool,
    dest_path: Option<String>,
    error_code: Option<ImportErrorCode>,
    error_message: Option<String>,
) -> ManualImportFileResult {
    ManualImportFileResult {
        file_path: mapping.file_path.clone(),
        episode_id: mapping.episode_id.clone(),
        series_movie_link_id: mapping.series_movie_link_id.clone(),
        success,
        skipped: false,
        dest_path,
        error_code,
        error_message,
    }
}

fn manual_import_skipped_file_result(
    mapping: &ManualImportFileMapping,
    message: String,
) -> ManualImportFileResult {
    ManualImportFileResult {
        skipped: true,
        error_message: Some(message),
        ..manual_import_file_result(mapping, false, None, None, None)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportRequestPayload {
    pub requested_by_user_id: Option<String>,
    pub title_id: Option<String>,
    pub download_client_item_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    pub client_type: String,
    #[serde(default)]
    pub files: Vec<ManualImportFileMapping>,
    #[serde(default)]
    pub selection_id: Option<String>,
    /// Persisted at queue time so a manual review of a Scryer grab cannot
    /// degrade to downloader display-name or filename evidence.
    #[serde(default)]
    pub(crate) release_evidence: Option<ReleaseEvidence>,
    /// Server-owned root that the queued candidate paths were validated against.
    #[serde(default)]
    pub(crate) trusted_source_root: Option<String>,
    /// Temporary archive staging root to remove after execution.
    #[serde(default)]
    pub(crate) archive_workspace_root: Option<String>,
    pub requested_at: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportExecutionResult {
    pub import_id: String,
    pub client_type: String,
    pub download_client_item_id: String,
    pub title_id: Option<String>,
    pub status: ImportStatus,
    pub error_code: Option<ImportErrorCode>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub requires_reconciliation: bool,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub file_results: Vec<ManualImportFileResult>,
    pub completed_at: DateTime<Utc>,
}
fn manual_import_error_from_skip_reason(skip_reason: Option<ImportSkipReason>) -> ImportErrorCode {
    match skip_reason {
        Some(ImportSkipReason::DiskFull) => ImportErrorCode::DiskFull,
        Some(ImportSkipReason::PermissionDenied) => ImportErrorCode::PermissionDenied,
        Some(ImportSkipReason::PolicyMismatch) => ImportErrorCode::PolicyMismatch,
        _ => ImportErrorCode::Unknown,
    }
}
fn classify_manual_import_error_message(message: &str) -> ImportErrorCode {
    let normalized = message.trim().to_ascii_lowercase();
    if normalized.contains("file not found") {
        ImportErrorCode::FileNotFound
    } else if normalized.contains("episode not found") {
        ImportErrorCode::EpisodeNotFound
    } else if normalized.contains("episode lookup failed") {
        ImportErrorCode::EpisodeLookupFailed
    } else if normalized.contains("source_job_failed")
        || normalized.contains("source download failed")
        || normalized.contains("source job failed")
    {
        ImportErrorCode::SourceJobFailed
    } else if normalized.contains("permission denied")
        || normalized.contains("operation not permitted")
    {
        ImportErrorCode::PermissionDenied
    } else if normalized.contains("no space left")
        || normalized.contains("disk full")
        || normalized.contains("insufficient disk space")
    {
        ImportErrorCode::DiskFull
    } else if normalized.is_empty() {
        ImportErrorCode::Unknown
    } else {
        // String matching is only a fallback for unexpected error paths.
        // Known manual-import failures should be classified structurally at
        // the point where the skip reason or domain error is produced.
        ImportErrorCode::IoFailed
    }
}
pub(crate) fn manual_import_result_json(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    status: ImportStatus,
    error_code: Option<ImportErrorCode>,
    error_message: Option<String>,
    file_results: Vec<ManualImportFileResult>,
) -> Option<String> {
    serde_json::to_string(&ManualImportExecutionResult {
        import_id: import_id.to_string(),
        client_type: payload.client_type.clone(),
        download_client_item_id: payload.download_client_item_id.clone(),
        title_id: payload.title_id.clone(),
        status,
        error_code,
        error_message,
        requires_reconciliation: false,
        retry_attempts: 0,
        next_retry_at: None,
        file_results,
        completed_at: Utc::now(),
    })
    .ok()
}

fn manual_import_pending_result_json(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    message: String,
    requires_reconciliation: bool,
    retry_attempts: u32,
    next_retry_at: Option<DateTime<Utc>>,
    file_results: Vec<ManualImportFileResult>,
) -> Option<String> {
    serde_json::to_string(&ManualImportExecutionResult {
        import_id: import_id.to_string(),
        client_type: payload.client_type.clone(),
        download_client_item_id: payload.download_client_item_id.clone(),
        title_id: payload.title_id.clone(),
        status: ImportStatus::Pending,
        error_code: Some(classify_manual_import_error_message(&message)),
        error_message: Some(message),
        requires_reconciliation,
        retry_attempts,
        next_retry_at,
        file_results,
        completed_at: Utc::now(),
    })
    .ok()
}

pub(crate) fn manual_import_source_failed_result_json(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    message: String,
) -> Option<String> {
    manual_import_result_json(
        import_id,
        payload,
        ImportStatus::Failed,
        Some(ImportErrorCode::SourceJobFailed),
        Some(message),
        Vec::new(),
    )
}
pub(crate) fn manual_import_request_matches_source(
    payload: &ManualImportRequestPayload,
    client_id: Option<&str>,
    client_type: Option<&str>,
    download_client_item_id: &str,
) -> bool {
    if payload.download_client_item_id != download_client_item_id {
        return false;
    }

    let requested_client_id = client_id.map(str::trim).filter(|value| !value.is_empty());
    let payload_client_id = payload
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let client_id_matches = match (requested_client_id, payload_client_id) {
        (None, None) => true,
        (Some(requested), Some(payload)) => requested.eq_ignore_ascii_case(payload),
        _ => false,
    };
    if !client_id_matches {
        return false;
    }

    let requested_client_type = client_type.map(str::trim).filter(|value| !value.is_empty());
    requested_client_type
        .is_none_or(|client_type| payload.client_type.eq_ignore_ascii_case(client_type))
}
pub(crate) async fn find_active_manual_import_for_source(
    app: &AppUseCase,
    client_id: Option<&str>,
    client_type: &str,
    download_client_item_id: &str,
) -> AppResult<Option<ImportRecord>> {
    let normalized_client_type = client_type.trim().to_lowercase();
    let source_ref = download_client_item_id.trim();
    if normalized_client_type.is_empty() || source_ref.is_empty() {
        return Ok(None);
    }

    let records = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(&[ClientJobLocator::new(
            client_id,
            normalized_client_type.as_str(),
            source_ref,
        )])
        .await?;

    Ok(records.into_iter().find(|record| {
        record.import_type == ImportType::ManualImport
            && record.status.is_active()
            && serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
                .ok()
                .is_some_and(|payload| {
                    manual_import_request_matches_source(
                        &payload,
                        client_id,
                        Some(normalized_client_type.as_str()),
                        source_ref,
                    )
                })
    }))
}
pub(crate) async fn fail_active_manual_import_for_source(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    failure_reason: &str,
) {
    let record = match find_active_manual_import_for_source(
        app,
        Some(tracked.client_id.as_str()),
        tracked.client_type.as_str(),
        tracked.client_item.download_client_item_id.as_str(),
    )
    .await
    {
        Ok(Some(record)) => record,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(
                error = %error,
                item_id = %tracked.client_item.download_client_item_id,
                "failed to inspect manual import request for failed source"
            );
            return;
        }
    };

    let payload = serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
        .unwrap_or_else(|_| ManualImportRequestPayload {
            requested_by_user_id: None,
            title_id: tracked.title_id.clone(),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            client_id: Some(tracked.client_id.clone()).filter(|value| !value.is_empty()),
            client_type: tracked.client_type.clone(),
            files: Vec::new(),
            selection_id: None,
            release_evidence: None,
            trusted_source_root: None,
            archive_workspace_root: None,
            requested_at: record.created_at.clone(),
        });
    let message = format!("source download failed before import: {failure_reason}");
    let result_json = manual_import_source_failed_result_json(&record.id, &payload, message);

    if let Err(error) = app
        .update_import_status_and_notify(&record.id, ImportStatus::Failed, result_json)
        .await
    {
        tracing::warn!(
            error = %error,
            import_id = %record.id,
            item_id = %tracked.client_item.download_client_item_id,
            "failed to terminate manual import request for failed source"
        );
    }
}
/// A manual import is `Completed` when every mapping that was actually
/// attempted succeeded and at least one file landed. Skipped mappings (movie
/// samples/extras beside the primary) are deliberate non-imports and never
/// count against completion; an import that attempted nothing is not complete.
fn manual_import_terminal_status_and_error(
    results: &[ManualImportFileResult],
) -> (ImportStatus, Option<ImportErrorCode>, Option<String>) {
    let attempted = results.iter().filter(|result| !result.skipped);
    let mut succeeded = 0usize;
    let mut first_failure = None;
    for result in attempted {
        if result.success {
            succeeded += 1;
        } else if first_failure.is_none() {
            first_failure = Some(result);
        }
    }

    match first_failure {
        None if succeeded > 0 => (ImportStatus::Completed, None, None),
        None => (
            ImportStatus::Failed,
            Some(ImportErrorCode::Unknown),
            Some("manual import did not import any file".to_string()),
        ),
        Some(failure) => (
            ImportStatus::Failed,
            failure.error_code.or(Some(ImportErrorCode::Unknown)),
            failure.error_message.clone(),
        ),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "manual series-movie import needs source, title, and resolved path context"
)]
async fn execute_manual_series_movie_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title: &scryer_domain::Title,
    completed: Option<&CompletedDownload>,
    release_evidence: &ReleaseEvidence,
    source: &Path,
    mapping: &ManualImportFileMapping,
    series_movie_link_id: &str,
    full_folder_path: &Path,
    season_folder_template: &str,
    specials_folder_template: &str,
    rename_enabled: bool,
) -> AppResult<ManualImportFileResult> {
    let link = match app
        .services
        .catalog
        .shows
        .get_series_movie_link_by_id(series_movie_link_id)
        .await?
    {
        Some(link) if link.series_title_id == title.id => link,
        Some(_) => {
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::Unknown),
                Some(format!(
                    "series movie link {series_movie_link_id} does not belong to title {}",
                    title.id
                )),
            ));
        }
        None => {
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::Unknown),
                Some(format!(
                    "series movie link {series_movie_link_id} not found"
                )),
            ));
        }
    };

    // Same identity the series movie was grabbed under (see
    // `import_series_movie_download`).
    let search_title = crate::acquisition_release_search::series_movie_search_title(title, &link);
    let parsed =
        build_augmented_movie_import_metadata_for_title(source, release_evidence, &search_title);
    let ext = scryer_domain::canonical_video_extension(source)
        .unwrap_or("mkv")
        .to_string();
    let linked_episode = if let Some(linked_episode_id) = link.linked_episode_id.as_deref() {
        app.services
            .catalog
            .shows
            .get_episode_by_id(linked_episode_id)
            .await?
    } else {
        None
    };
    let season_episode = linked_episode
        .as_ref()
        .and_then(|episode| {
            let season = episode.season_number.as_deref()?.parse::<i32>().ok()?;
            let episode_number = episode.episode_number.as_deref()?.parse::<i32>().ok()?;
            Some(format!("S{season:02}E{episode_number:02}"))
        })
        .unwrap_or_else(|| "S00E00".to_string());
    let rendered_filename = if rename_enabled {
        sanitize_filesystem_component(&format!(
            "{} - {} - {}.{}",
            title.name, season_episode, link.movie.title, ext
        ))
    } else {
        preserved_import_filename(source)
    };
    let use_season_folders = app.resolve_use_season_folders(title).await?;
    let dest_path = episodic_import_parent_path(
        title,
        use_season_folders,
        full_folder_path,
        season_folder_template,
        specials_folder_template,
        0,
    )
    .join(rendered_filename);
    persist_title_folder_path_if_missing(app, title, full_folder_path).await?;
    if let Some(parent) = dest_path.parent()
        && let Err(error) = tokio::fs::create_dir_all(parent).await
    {
        return Ok(manual_import_file_result(
            mapping,
            false,
            None,
            Some(classify_manual_import_error_message(&error.to_string())),
            Some(format!(
                "failed to create destination directory {}: {error}",
                parent.display()
            )),
        ));
    }

    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        completed,
    )
    .await?;
    let destination_ownership = ImportDestinationOwnership::series_movie(
        series_movie_link_id,
        link.linked_episode_id.as_deref(),
    );
    let file_result = match import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        &destination_ownership,
        source,
        &dest_path,
        import_mode,
        None,
        completed,
    )
    .await
    {
        Ok(file_result) => file_result,
        Err(error @ AppError::ManualReconciliationRequired(_)) => return Err(error),
        Err(error) => {
            let message = error.to_string();
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(classify_manual_import_error_message(&message)),
                Some(message),
            ));
        }
    };
    let quality_label = parsed.quality.clone();
    let started_at = Utc::now();
    let imported_media_file_id = match file_result
        .insert_or_reuse_media_file(
            app,
            &crate::InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: path_to_stored_string(&dest_path),
                size_bytes: file_result.size_bytes as i64,
                announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
                    file_result.size_bytes as i64,
                    release_evidence.announced_size_bytes(),
                ),
                role: crate::MediaFileRole::Primary,
                quality_label: quality_label.clone(),
                scene_name: Some(parsed.raw_title.clone()),
                release_group: parsed.release_group.clone(),
                source_type: crate::release_parser::parsed_release_source_type(&parsed),
                resolution: quality_label,
                video_codec_parsed: parsed.video_codec,
                audio_codec_parsed: parsed.audio.as_ref().map(ToString::to_string),
                audio_channels_parsed: parsed.audio_channels.clone(),
                original_file_path: Some(path_to_stored_string(source)),
                grabbed_release_title: release_evidence.release_title(Some(source)),
                grabbed_at: Some(started_at.to_rfc3339()),
                edition: parsed.edition.clone(),
                ..Default::default()
            },
        )
        .await
    {
        Ok(persistence) => persistence.media_file_id,
        Err(error) => {
            let message = error.to_string();
            return Ok(manual_import_file_result(
                mapping,
                false,
                Some(path_to_stored_string(&dest_path)),
                Some(classify_manual_import_error_message(&message)),
                Some(message),
            ));
        }
    };

    if let Some(linked_episode_id) = link.linked_episode_id.as_deref() {
        app.services
            .library
            .media_files
            .set_media_file_roles_for_episode(
                &title.id,
                linked_episode_id,
                &imported_media_file_id,
                &[],
            )
            .await?;
    }

    analyze_and_persist_imported_media_file(app, &title.id, &imported_media_file_id, &dest_path)
        .await;
    if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &imported_media_file_id,
        None,
        &dest_path,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            title_id = %title.id,
            file_id = %imported_media_file_id,
            dest_path = %dest_path.display(),
            "failed to reconcile external subtitles after manual series movie import"
        );
    }
    maybe_trigger_subtitle_search(app, &title.id, &imported_media_file_id);
    if let Some(completed) = completed {
        let linked_episode_artifacts = linked_episode.iter().cloned().collect::<Vec<_>>();
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source,
            "movie",
            "imported",
            None,
            Some(imported_media_file_id.as_str()),
            &linked_episode_artifacts,
        )
        .await?;
    }

    if let Err(error) =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path, completed).await
    {
        if matches!(&error, AppError::ManualReconciliationRequired(_)) {
            return Err(error);
        }
        let message = error.to_string();
        return Ok(manual_import_file_result(
            mapping,
            false,
            Some(path_to_stored_string(&dest_path)),
            Some(classify_manual_import_error_message(&message)),
            Some(message),
        ));
    }

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = crate::nfo::render_series_movie_episode_nfo(
            &link.movie,
            &season_episode,
            link.after_season,
        );
        if let Err(error) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %error,
                path = %nfo_path.display(),
                "failed to write manual series movie NFO sidecar"
            );
        }
    }

    mark_wanted_completed_for_series_movie_link(app, &title.id, series_movie_link_id, false).await;
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor: crate::domain_events::DomainEventActor::from(actor),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|external_id| external_id.source == "imdb")
            .map(|external_id| external_id.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|external_id| external_id.source == "tvdb")
            .map(|external_id| external_id.value.clone()),
        season: None,
        episode: None,
        quality: parsed.quality.clone(),
    });

    Ok(manual_import_file_result(
        mapping,
        true,
        Some(path_to_stored_string(&dest_path)),
        None,
        None,
    ))
}

/// Execute a manual import: import each file with user-specified episode mappings.
pub async fn execute_manual_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title_id: &str,
    completed: Option<&CompletedDownload>,
    files: Vec<ManualImportFileMapping>,
    trusted_source_root: Option<PathBuf>,
) -> AppResult<Vec<ManualImportFileResult>> {
    let release_evidence = match completed {
        Some(completed) => {
            resolve_release_evidence_for_completed_download(app, completed, None).await?
        }
        None => ReleaseEvidence::DownloaderObservation { release_name: None },
    };
    execute_manual_import_with_release_evidence(
        app,
        actor,
        import_id,
        title_id,
        completed,
        &release_evidence,
        files,
        trusted_source_root,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "manual execution carries explicit user mappings, trusted root, and durable release evidence"
)]
pub(crate) async fn execute_manual_import_with_release_evidence(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title_id: &str,
    completed: Option<&CompletedDownload>,
    release_evidence: &ReleaseEvidence,
    files: Vec<ManualImportFileMapping>,
    trusted_source_root: Option<PathBuf>,
) -> AppResult<Vec<ManualImportFileResult>> {
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title not found: {}", title_id)))?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;
    // Manual imports do not pass through the completed-download dispatcher, so
    // they carry their own check (FR-084).
    app.ensure_location_ownership_allows_title(
        &crate::location::ownership_guard::MANUAL_IMPORT_ENTRY,
        &title.id,
    )
    .await?;
    for mapping in &files {
        validate_manual_import_target_scope(
            app,
            &title.id,
            mapping.episode_id.as_deref(),
            mapping.series_movie_link_id.as_deref(),
        )
        .await?;
    }
    let trusted_source_root = trusted_source_root
        .as_deref()
        .ok_or_else(|| AppError::Validation("manual import source root is required".to_string()))?;
    let trusted_source_root = std::fs::canonicalize(trusted_source_root).map_err(|error| {
        AppError::Validation(format!(
            "manual import source root is not accessible: {} ({error})",
            trusted_source_root.display()
        ))
    })?;

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    } = resolve_import_paths(app, &title).await?;
    let full_folder_path = effective_title_folder_path(&media_root, &title, &folder_template, None);
    ensure_import_title_folder_available(app, &title, &full_folder_path).await?;
    let quality_profile = resolve_import_quality_profile(app, &title).await?;
    let movie_primary_index = (title.facet == MediaFacet::Movie)
        .then(|| select_manual_movie_primary_index(&files, &title.facet, &trusted_source_root))
        .flatten();

    let mut results = Vec::new();
    // Total bytes across every file this manual import brought in; stays `None`
    // until at least one file reports a size.
    let mut imported_size_bytes: Option<i64> = None;

    for (mapping_index, mapping) in files.iter().enumerate() {
        let source = stored_path_to_path_buf(&mapping.file_path);
        let qualified =
            match qualify_manual_import_video_candidate(&source, &trusted_source_root).await {
                Ok(Some(qualified)) => qualified,
                Ok(None) => {
                    results.push(manual_import_file_result(
                        mapping,
                        false,
                        None,
                        Some(ImportErrorCode::PolicyMismatch),
                        Some("manual import candidate is no longer a valid video".to_string()),
                    ));
                    continue;
                }
                Err(err) => {
                    results.push(manual_import_file_result(
                        mapping,
                        false,
                        None,
                        Some(if !source.exists() {
                            ImportErrorCode::FileNotFound
                        } else {
                            classify_manual_import_error_message(&err.to_string())
                        }),
                        Some(err.to_string()),
                    ));
                    continue;
                }
            };
        let source = qualified.canonical_path;

        let target = match manual_import_mapping_target(mapping, &title.facet) {
            Ok(target) => target,
            Err(error @ AppError::ManualReconciliationRequired(_)) => return Err(error),
            Err(err) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::Unknown),
                    Some(err.to_string()),
                ));
                continue;
            }
        };

        let episode_id = match target {
            ManualImportMappingTarget::Episode(episode_id) => episode_id,
            ManualImportMappingTarget::Movie => {
                // Only the primary movie file is imported; the other mapped
                // videos are samples/extras and are recorded as skipped so they
                // neither reach the movie importer nor block completion.
                match movie_primary_index {
                    Some(primary_index) if primary_index == mapping_index => {}
                    Some(_) => {
                        results.push(manual_import_skipped_file_result(
                            mapping,
                            "skipped: not the primary movie file".to_string(),
                        ));
                        continue;
                    }
                    None => {
                        results.push(manual_import_file_result(
                            mapping,
                            false,
                            None,
                            Some(ImportErrorCode::PolicyMismatch),
                            Some(MANUAL_MOVIE_NO_PRIMARY_FILE.to_string()),
                        ));
                        continue;
                    }
                }
                // Reuse the canonical movie import rather than re-deriving
                // destination and naming here: a manually chosen file must land
                // exactly where the automatic path would have put it, or the
                // same movie ends up named two different ways depending on how
                // it was imported.
                // The canonical movie import derives naming and metadata from
                // the completed download, so it needs one. Every path that can
                // produce a Movie target today resolves the source download
                // first; report rather than unwrap, so a future caller without
                // one gets a message instead of a panic.
                let Some(completed) = completed else {
                    results.push(manual_import_file_result(
                        mapping,
                        false,
                        None,
                        Some(ImportErrorCode::Unknown),
                        Some(
                            "manual movie import requires the completed download context"
                                .to_string(),
                        ),
                    ));
                    continue;
                };
                let result = import_movie_download(
                    app,
                    actor,
                    &title,
                    import_id,
                    completed,
                    release_evidence,
                    // Manual import never asks srrdb: an operator already told
                    // Scryer what this file is.
                    std::slice::from_ref(&ImportVideoFile::physical(source.clone())),
                    Utc::now(),
                    // An operator picked this file. The same bypass the manual
                    // episode path passes: no automatic sample rail, and no
                    // truth-verdict rejection that would blocklist their choice.
                    crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck,
                )
                .await;
                let file_result = match result {
                    Ok(import_result) => {
                        let success = import_result.dest_path.is_some()
                            && import_result.error_message.is_none();
                        manual_import_file_result(
                            mapping,
                            success,
                            import_result.dest_path,
                            (!success).then_some(ImportErrorCode::Unknown),
                            import_result.error_message,
                        )
                    }
                    Err(error @ AppError::ManualReconciliationRequired(_)) => return Err(error),
                    Err(error) => {
                        let message = error.to_string();
                        manual_import_file_result(
                            mapping,
                            false,
                            None,
                            Some(classify_manual_import_error_message(&message)),
                            Some(message),
                        )
                    }
                };
                results.push(file_result);
                continue;
            }
            ManualImportMappingTarget::SeriesMovie(series_movie_link_id) => {
                let result = execute_manual_series_movie_import(
                    app,
                    actor,
                    import_id,
                    &title,
                    completed,
                    release_evidence,
                    &source,
                    mapping,
                    series_movie_link_id,
                    &full_folder_path,
                    &season_folder_template,
                    &specials_folder_template,
                    rename_enabled,
                )
                .await?;
                results.push(result);
                continue;
            }
        };

        // Look up episode
        let episode = match app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await
        {
            Ok(Some(ep)) => ep,
            Ok(None) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::EpisodeNotFound),
                    Some(format!("episode not found: {episode_id}")),
                ));
                continue;
            }
            Err(err) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::EpisodeLookupFailed),
                    Some(format!("episode lookup failed: {}", err)),
                ));
                continue;
            }
        };
        if episode.title_id != title.id {
            results.push(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::EpisodeNotFound),
                Some(format!(
                    "episode {episode_id} does not belong to title {}",
                    title.id
                )),
            ));
            continue;
        }

        // Parse release metadata for quality/codec tokens. The operator's
        // mapping decides the episode; the parse still honours the multi-file
        // rule so the release-type/coverage facts it carries stay honest.
        let parsed = build_augmented_episode_import_metadata_for_title(
            &source,
            release_evidence,
            &title,
            files.len() > 1,
        );

        let season_num: u32 = episode
            .season_number
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let ep_num_str = episode.episode_number.clone().unwrap_or_default();
        match execute_resolved_episode_import(
            app,
            actor,
            &title,
            import_id,
            completed,
            rename_enabled,
            &rename_template,
            &season_folder_template,
            &specials_folder_template,
            &full_folder_path,
            &source,
            &parsed,
            std::slice::from_ref(&episode),
            std::slice::from_ref(&episode),
            season_num,
            &ep_num_str,
            episode.absolute_number.as_deref(),
            episode.title.as_deref(),
            &quality_profile,
            None,
            crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck,
            crate::import_decide::ImportOrigin::OperatorQueued,
            release_evidence.announced_size_bytes(),
            false,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                imported_media_file_id,
                reason_code,
                size_bytes,
                source_cleanup,
                destination_permit: _destination_permit,
                ..
            }) => {
                if let Some(completed) = completed {
                    persist_file_import_artifact(
                        app,
                        import_id,
                        completed,
                        title.id.as_str(),
                        &source,
                        "episode",
                        "imported",
                        reason_code.as_deref(),
                        imported_media_file_id.as_deref(),
                        std::slice::from_ref(&episode),
                    )
                    .await?;
                }
                finalize_deferred_import_source_cleanup(
                    app,
                    source_cleanup.map(|guard| *guard),
                    &crate::stored_paths::stored_path_to_path_buf(&dest_path),
                    completed,
                )
                .await?;
                if let Some(size_bytes) = size_bytes {
                    imported_size_bytes =
                        Some(imported_size_bytes.unwrap_or(0).saturating_add(size_bytes));
                }
                results.push(manual_import_file_result(
                    mapping,
                    true,
                    Some(dest_path),
                    None,
                    None,
                ));
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                reason_code,
                skip_reason,
                ..
            }) => {
                if episode_skip_is_already_present(skip_reason.as_ref()) {
                    // The identical file already sits at the destination (an
                    // earlier import landed it): the operator's mapping is
                    // satisfied, not failed. Record it like the automatic path
                    // so verification counts the unit and the tracked download
                    // finalizes as imported instead of re-offering an import
                    // that can never succeed.
                    if let Some(completed) = completed {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source,
                            "episode",
                            "already_present",
                            reason_code.as_deref(),
                            None,
                            std::slice::from_ref(&episode),
                        )
                        .await?;
                    }
                    results.push(manual_import_file_result(mapping, true, None, None, None));
                    continue;
                }
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(manual_import_error_from_skip_reason(skip_reason.clone())),
                    Some(message),
                ));
            }
            Ok(EpisodeImportOutcome::Ignored { message, .. }) => {
                // Automatic pack planning is never active for an operator's
                // explicit manual mapping. Keep this arm non-successful if a
                // future caller reaches it rather than turning it into a
                // manual ignore action.
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(manual_import_error_from_skip_reason(None)),
                    Some(message),
                ));
            }
            Ok(EpisodeImportOutcome::Rejected { rejection, .. }) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(manual_import_error_from_skip_reason(
                        rejection.skip_reason.clone(),
                    )),
                    Some(rejection.message),
                ));
            }
            Err(error @ AppError::ManualReconciliationRequired(_)) => return Err(error),
            Err(err) => {
                let error_message = err.to_string();
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(classify_manual_import_error_message(&error_message)),
                    Some(error_message),
                ));
            }
        }
    }

    let imported_updates: Vec<NotificationMediaUpdate> = results
        .iter()
        .filter(|result| result.success)
        .filter_map(|result| {
            result
                .dest_path
                .as_ref()
                .map(|path| NotificationMediaUpdate::created(path.clone()))
        })
        .collect();

    let success_count = results.iter().filter(|r| r.success).count();
    let (terminal_status, _, _) = manual_import_terminal_status_and_error(&results);
    if success_count > 0 && terminal_status == ImportStatus::Completed {
        let episode_ids = results
            .iter()
            .filter(|result| result.success)
            .filter_map(|result| result.episode_id.clone())
            .collect::<Vec<_>>();
        app.append_domain_event(new_title_domain_event(
            actor,
            &title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(&title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: success_count as i32,
                import_id: None,
                source_system: completed.map(|download| download.client_type.clone()),
                source_ref: completed.map(|download| download.download_client_item_id.clone()),
                source_title: release_evidence.release_title(
                    files
                        .first()
                        .map(|mapping| Path::new(mapping.file_path.as_str())),
                ),
                source_path: (files.len() == 1).then(|| files[0].file_path.clone()),
                dest_path: results
                    .iter()
                    .find(|result| result.success)
                    .and_then(|result| result.dest_path.clone()),
                quality: None,
                episode_ids,
                size_bytes: imported_size_bytes,
            }),
        ))
        .await?;
    }

    Ok(results)
}

/// Backstop for a manual import whose record reached `Completed` but whose
/// tracked download was never terminalized (crash, dropped reply, restart).
///
/// Bounded three ways: the store query only returns records updated inside
/// the recovery window, `reconciled_import_ids` makes each record a one-time
/// decision per process, and the tracked-download runtime only marks a source
/// the client finished before the record completed and that is waiting on
/// import (a fresh download reusing an old item id is left alone). Busy or
/// not-yet-tracked sources are retried on a later tick without logging.
/// Per-process memory of what the recovery loop already decided about a
/// completed manual-import record, so a 2 s tick does not re-ask the tracked
/// download service the same question for 24 h.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualImportRecoveryMemo {
    /// Marked, already imported, or can never recover — never revisit.
    Settled,
    /// The tracked download was not known (not yet tracked, or its client
    /// history entry is gone); ask again after this instant, on a growing
    /// backoff.
    RetryAfter {
        next_check_at: DateTime<Utc>,
        attempts: u32,
    },
}

/// Backoff for an untracked record: a just-completed download may not be
/// tracked for a tick or two, but one whose history entry is gone will never
/// be, so the cadence grows quickly to the same cap the import retry uses.
fn manual_import_recovery_retry_delay(attempts: u32) -> chrono::Duration {
    crate::tracked_downloads::import_execution_retry_delay(attempts)
}

async fn recover_completed_manual_imports(
    app: &AppUseCase,
    worker: &PollingWorker,
    memo: &mut HashMap<String, ManualImportRecoveryMemo>,
) {
    use crate::tracked_downloads::ManualImportRecoveryOutcome;

    let now = Utc::now();
    let updated_after = now - chrono::Duration::hours(MANUAL_IMPORT_RECOVERY_WINDOW_HOURS);
    let records = match app
        .services
        .workflow
        .imports
        .list_completed_manual_imports(updated_after, MANUAL_IMPORT_RECOVERY_BATCH_SIZE)
        .await
    {
        Ok(records) => records,
        Err(error) => {
            worker.warn_error("list_completed_manual_imports", &error);
            return;
        }
    };
    // Records that aged out of the window never come back; forget them.
    let live_ids = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<HashSet<_>>();
    memo.retain(|id, _| live_ids.contains(id.as_str()));
    let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() else {
        return;
    };

    for record in records {
        let attempts = match memo.get(&record.id) {
            Some(ManualImportRecoveryMemo::Settled) => continue,
            Some(ManualImportRecoveryMemo::RetryAfter {
                next_check_at,
                attempts,
            }) => {
                if *next_check_at > now {
                    continue;
                }
                *attempts
            }
            None => 0,
        };
        let Some(recovery) = completed_manual_import_recovery(&record) else {
            // Partial, malformed, or identity-less: it will never recover.
            memo.insert(record.id, ManualImportRecoveryMemo::Settled);
            continue;
        };
        let source_identity = recovery.source_identity;
        let canonical_download_id = match app
            .services
            .workflow
            .imports
            .canonical_download_id_for_import(&record.id)
            .await
        {
            Ok(canonical_download_id) => canonical_download_id,
            Err(error) => {
                worker.warn_error("recovered_manual_import_canonical_download_id", &error);
                None
            }
        };
        match handle
            .mark_imported_if_awaiting_import_for_download(
                source_identity.clone(),
                canonical_download_id,
                recovery.record_completed_at,
            )
            .await
        {
            Ok(ManualImportRecoveryOutcome::Marked) => {
                memo.insert(record.id, ManualImportRecoveryMemo::Settled);
                if let Err(error) = app
                    .services
                    .workflow
                    .imports
                    .delete_manual_import_selections_for_source(&source_identity)
                    .await
                {
                    worker.warn_error("cleanup_recovered_manual_import_selection", &error);
                }
            }
            Ok(ManualImportRecoveryOutcome::Unchanged) => {
                memo.insert(record.id, ManualImportRecoveryMemo::Settled);
            }
            Ok(ManualImportRecoveryOutcome::Untracked) => {
                let attempts = attempts.saturating_add(1);
                memo.insert(
                    record.id,
                    ManualImportRecoveryMemo::RetryAfter {
                        next_check_at: now + manual_import_recovery_retry_delay(attempts),
                        attempts,
                    },
                );
            }
            // Busy is momentary: ask again next tick.
            Ok(ManualImportRecoveryOutcome::Busy) => {}
            Err(error) => worker.warn_error("recover_completed_manual_import", &error),
        }
    }
}

struct CompletedManualImportRecovery {
    source_identity: ClientJobLocator,
    /// When the record reached `Completed`: `finished_at`, falling back to
    /// `updated_at`. Only a tracked download the client finished before this
    /// may be terminalized on the strength of the record.
    record_completed_at: DateTime<Utc>,
}

fn import_record_completed_at(record: &ImportRecord) -> Option<DateTime<Utc>> {
    record
        .finished_at
        .as_deref()
        .into_iter()
        .chain(std::iter::once(record.updated_at.as_str()))
        .find_map(|value| {
            DateTime::parse_from_rfc3339(value.trim())
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
}

fn completed_manual_import_recovery(
    record: &ImportRecord,
) -> Option<CompletedManualImportRecovery> {
    if record.import_type != ImportType::ManualImport || record.status != ImportStatus::Completed {
        return None;
    }
    let result =
        serde_json::from_str::<ManualImportExecutionResult>(record.result_json.as_deref()?).ok()?;
    // Skipped mappings (movie samples/extras beside the primary) were never
    // attempted; only attempted mappings must have succeeded.
    let attempted = result
        .file_results
        .iter()
        .filter(|file| !file.skipped)
        .collect::<Vec<_>>();
    if result.import_id != record.id
        || result.status != ImportStatus::Completed
        || attempted.is_empty()
        || attempted.iter().any(|file| !file.success)
        || !result
            .client_type
            .eq_ignore_ascii_case(&record.source_system)
        || result.download_client_item_id != record.source_ref
        || record.source_system.trim().is_empty()
        || record.source_ref.trim().is_empty()
    {
        return None;
    }
    let client_id = record
        .source_client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    // Without a usable completion time the record cannot prove it predates the
    // tracked download's completion, so it cannot safely recover anything.
    let record_completed_at = import_record_completed_at(record)?;
    Some(CompletedManualImportRecovery {
        source_identity: ClientJobLocator::new(
            Some(client_id),
            &record.source_system,
            &record.source_ref,
        ),
        record_completed_at,
    })
}

struct QueuedManualImportOutcome {
    status: ImportStatus,
    result_json: Option<String>,
    files_imported_this_pass: usize,
    completed: Option<CompletedDownload>,
    title_id: Option<String>,
    expected_mapping_count: Option<usize>,
    prior_import_proven: bool,
}

impl QueuedManualImportOutcome {
    fn source_unavailable(import_id: &str, payload: &ManualImportRequestPayload) -> Self {
        Self {
            status: ImportStatus::Failed,
            result_json: manual_import_source_failed_result_json(
                import_id,
                payload,
                MANUAL_IMPORT_SOURCE_UNAVAILABLE.to_string(),
            ),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: None,
            prior_import_proven: false,
        }
    }

    fn already_imported(import_id: &str, payload: &ManualImportRequestPayload) -> Self {
        let now = Utc::now();
        let result = scryer_domain::ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            title_id: payload.title_id.clone(),
            source_system: Some(payload.client_type.clone()),
            source_ref: Some(payload.download_client_item_id.clone()),
            source_title: None,
            source_path: String::new(),
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            release_burned: false,
            started_at: now,
            completed_at: now,
        };
        Self {
            status: ImportStatus::Skipped,
            result_json: serde_json::to_string(&result).ok(),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: None,
            prior_import_proven: true,
        }
    }
}

async fn execute_queued_manual_import_with_outcome(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<QueuedManualImportOutcome> {
    let outcome = execute_queued_manual_import_with_outcome_inner(app, import_id, payload).await;
    if let Some(workspace_root) = payload.archive_workspace_root.as_deref() {
        crate::archive_extractor::cleanup_extracted_dir(&stored_path_to_path_buf(workspace_root))
            .await;
    }
    outcome
}

async fn execute_queued_manual_import_with_outcome_inner(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<QueuedManualImportOutcome> {
    let preparation_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_preparation()
        .await;
    let user_id = payload
        .requested_by_user_id
        .as_deref()
        .ok_or_else(|| AppError::Validation("manual import request is missing an actor".into()))?;
    let actor = app
        .services
        .identity
        .users
        .get_by_id(user_id)
        .await?
        .ok_or_else(|| {
            AppError::Validation("manual import request actor no longer exists".into())
        })?;

    let Some(title_id) = payload
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(QueuedManualImportOutcome::source_unavailable(
            import_id, payload,
        ));
    };
    let Some(client_id) = payload
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(QueuedManualImportOutcome::source_unavailable(
            import_id, payload,
        ));
    };
    let client_type = payload.client_type.trim();
    let download_client_item_id = payload.download_client_item_id.trim();
    let authorized_source = match authorize_manual_import_source(
        app,
        &actor,
        client_id,
        client_type,
        download_client_item_id,
        title_id,
    )
    .await
    {
        Ok(source) => source,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    if manual_import_source_was_already_imported(app, &authorized_source, import_id).await? {
        return Ok(QueuedManualImportOutcome::already_imported(
            import_id, payload,
        ));
    }
    let source_identity = authorized_source.identity;
    let completed = match resolve_authorized_manual_import_source(app, &source_identity).await {
        Ok(completed) => completed,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    let release_evidence = match payload.release_evidence.clone() {
        Some(release_evidence) => release_evidence,
        None => resolve_release_evidence_for_completed_download(app, &completed, None).await?,
    };
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let trusted_source_root_result = match payload.trusted_source_root.as_deref() {
        Some(root) => std::fs::canonicalize(stored_path_to_path_buf(root)),
        None => std::fs::canonicalize(&completed.dest_dir),
    };
    let trusted_source_root = match trusted_source_root_result {
        Ok(root) => root,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    if payload.files.is_empty() {
        return Ok(QueuedManualImportOutcome {
            status: ImportStatus::Failed,
            result_json: manual_import_result_json(
                import_id,
                payload,
                ImportStatus::Failed,
                Some(ImportErrorCode::Unknown),
                Some("manual import requires at least one queued file mapping".to_string()),
                Vec::new(),
            ),
            files_imported_this_pass: 0,
            completed: Some(completed),
            title_id: Some(title_id.to_string()),
            expected_mapping_count: None,
            prior_import_proven: false,
        });
    }

    drop(preparation_permit);
    app.update_import_status_and_notify(import_id, ImportStatus::Processing, None)
        .await?;

    let results = execute_manual_import_with_release_evidence(
        app,
        &actor,
        import_id,
        title_id,
        Some(&completed),
        &release_evidence,
        payload.files.clone(),
        Some(trusted_source_root),
    )
    .await?;
    let files_imported_this_pass = results.iter().filter(|result| result.success).count();
    // Verification compares imported files against what the import attempted;
    // skipped mappings (movie samples/extras) were never expected to land.
    let expected_mapping_count = Some(results.iter().filter(|result| !result.skipped).count());
    let (status, error_code, error_message) = manual_import_terminal_status_and_error(&results);

    if status == ImportStatus::Completed
        && let Err(error) = app
            .services
            .workflow
            .imports
            .delete_manual_import_selections_for_source(&source_identity)
            .await
    {
        tracing::warn!(
            error = %error,
            item_id = %source_identity.item_id,
            "failed to clean up terminal manual-import selections"
        );
    }

    let result_json = manual_import_result_json(
        import_id,
        payload,
        status,
        error_code,
        error_message,
        results,
    );

    Ok(QueuedManualImportOutcome {
        status,
        result_json,
        files_imported_this_pass,
        completed: Some(completed),
        title_id: Some(title_id.to_string()),
        expected_mapping_count,
        prior_import_proven: false,
    })
}

pub async fn execute_queued_manual_import(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<(ImportStatus, Option<String>)> {
    let outcome = execute_queued_manual_import_with_outcome(app, import_id, payload).await;
    let outcome = outcome?;
    Ok((outcome.status, outcome.result_json))
}

#[cfg(test)]
mod manual_archive_workspace_tests {
    use super::*;

    fn selection(workspace_root: &Path, trusted_root: &Path) -> crate::ManualImportSelection {
        crate::ManualImportSelection {
            id: "selection-1".to_string(),
            actor_user_id: "user-1".to_string(),
            title_id: "title-1".to_string(),
            source_identity: ClientJobLocator::new(Some("client-1"), "weaver", "item-1"),
            canonical_download_id: None,
            release_evidence_json: None,
            trusted_source_root: path_to_stored_string(trusted_root),
            archive_workspace_root: Some(path_to_stored_string(workspace_root)),
            candidates: Vec::new(),
        }
    }

    #[test]
    fn reuses_archive_workspace_only_when_it_matches_the_trusted_root() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let canonical_workspace =
            std::fs::canonicalize(workspace.path()).expect("canonical workspace root");
        let selection = selection(&canonical_workspace, &canonical_workspace);

        assert_eq!(
            reusable_manual_archive_workspace(&selection),
            Some((
                canonical_workspace.clone(),
                path_to_stored_string(&canonical_workspace),
            ))
        );
    }

    #[test]
    fn refuses_archive_workspace_when_it_differs_from_the_trusted_root() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let trusted = tempfile::tempdir().expect("trusted tempdir");
        let selection = selection(workspace.path(), trusted.path());

        assert!(reusable_manual_archive_workspace(&selection).is_none());
    }
}

#[cfg(test)]
mod manual_preview_suggestion_tests {
    use super::*;

    fn episode_for_label(absolute_number: Option<&str>) -> scryer_domain::Episode {
        scryer_domain::Episode {
            id: "episode-19".to_string(),
            title_id: "title-1".to_string(),
            collection_id: Some("season-1".to_string()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("19".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("provider label is not the import target".to_string()),
            title: Some("Episode Title".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute_number.map(str::to_string),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn file_episode(stem: &str) -> Option<crate::ParsedEpisodeMetadata> {
        parse_release_metadata(stem).episode
    }

    #[test]
    fn manual_episode_label_names_the_absolute_number() {
        assert_eq!(
            manual_import_episode_label(&episode_for_label(Some("19"))),
            "S01E19 · Absolute 19 — Episode Title"
        );

        let mut decorated = episode_for_label(Some("19"));
        decorated.season_number = Some("Season 1".to_string());
        decorated.episode_number = Some("Episode 19".to_string());
        assert_eq!(
            manual_import_episode_label(&decorated),
            "S01E19 · Absolute 19 — Episode Title"
        );

        decorated.season_number = None;
        decorated.episode_number = None;
        assert_eq!(
            manual_import_episode_label(&decorated),
            "S??E?? · Absolute 19 — Episode Title"
        );
    }

    #[test]
    fn manual_episode_label_without_absolute_number_preserves_catalog_label() {
        assert_eq!(
            manual_import_episode_label(&episode_for_label(None)),
            "provider label is not the import target"
        );
    }

    fn suggestion(
        parsed_suggestion: Option<&str>,
        grabbed: &HashSet<String>,
        is_largest_video: bool,
        file_stem: &str,
    ) -> Option<String> {
        let episode = file_episode(file_stem);
        manual_episode_suggestion_for_grabbed_scope(
            parsed_suggestion.map(str::to_string),
            grabbed,
            manual_grabbed_episode_fallback_applies(is_largest_video, episode.as_ref()),
        )
    }

    #[test]
    fn largest_file_that_parses_to_a_different_episode_is_left_unselected() {
        let grabbed = HashSet::from(["episode-3".to_string()]);
        assert!(file_episode("Show.S01E04.720p.WEB-DL").is_some());

        assert_eq!(
            suggestion(Some("episode-4"), &grabbed, true, "Show.S01E04.720p.WEB-DL"),
            None,
            "a positive parse outside the grabbed scope must not be overridden"
        );
    }

    #[test]
    fn largest_file_without_an_episode_parse_starts_from_the_single_grabbed_episode() {
        let grabbed = HashSet::from(["episode-3".to_string()]);
        assert!(file_episode("4f8e2c7a91b6d3e0").is_none());

        assert_eq!(
            suggestion(None, &grabbed, true, "4f8e2c7a91b6d3e0").as_deref(),
            Some("episode-3")
        );
        assert_eq!(
            suggestion(None, &grabbed, false, "4f8e2c7a91b6d3e0"),
            None,
            "only the largest video inherits the grabbed episode"
        );
    }

    #[test]
    fn largest_file_with_an_unresolved_episode_parse_is_left_to_the_user() {
        // Parses to S01E99, which the catalog does not know (no parsed
        // suggestion), but the file still names an episode: do not guess.
        let grabbed = HashSet::from(["episode-3".to_string()]);
        assert!(file_episode("Show.S01E99.720p.WEB-DL").is_some());

        assert_eq!(
            suggestion(None, &grabbed, true, "Show.S01E99.720p.WEB-DL"),
            None
        );
    }

    #[test]
    fn parsed_suggestion_inside_the_grabbed_scope_always_stands() {
        let grabbed = HashSet::from(["episode-3".to_string(), "episode-4".to_string()]);

        assert_eq!(
            suggestion(
                Some("episode-4"),
                &grabbed,
                false,
                "Show.S01E04.720p.WEB-DL"
            )
            .as_deref(),
            Some("episode-4")
        );
        assert_eq!(
            suggestion(Some("episode-8"), &grabbed, true, "Show.S01E08.720p.WEB-DL"),
            None
        );
    }
}

#[cfg(test)]
mod manual_import_recovery_tests {
    use super::*;

    fn completed_manual_import_record(client_type: &str, file_success: bool) -> ImportRecord {
        let id = "manual-import-1";
        let result = ManualImportExecutionResult {
            import_id: id.to_string(),
            client_type: client_type.to_string(),
            download_client_item_id: "download-1".to_string(),
            title_id: Some("title-1".to_string()),
            status: ImportStatus::Completed,
            error_code: None,
            error_message: None,
            requires_reconciliation: false,
            retry_attempts: 0,
            next_retry_at: None,
            file_results: vec![ManualImportFileResult {
                file_path: "/downloads/episode.mkv".to_string(),
                episode_id: Some("episode-1".to_string()),
                series_movie_link_id: None,
                success: file_success,
                skipped: false,
                dest_path: file_success.then(|| "/library/episode.mkv".to_string()),
                error_code: None,
                error_message: None,
            }],
            completed_at: Utc::now(),
        };
        ImportRecord {
            id: id.to_string(),
            source_client_id: Some("client-1".to_string()),
            source_system: client_type.to_string(),
            source_ref: "download-1".to_string(),
            import_type: ImportType::ManualImport,
            status: ImportStatus::Completed,
            payload_json: "{}".to_string(),
            result_json: Some(serde_json::to_string(&result).expect("result JSON")),
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-08-17T00:00:00Z".to_string(),
            updated_at: "2026-08-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn completed_manual_import_recovery_uses_generic_client_identity() {
        for client_type in ["nzbget", "qbittorrent"] {
            let record = completed_manual_import_record(client_type, true);

            let recovery = completed_manual_import_recovery(&record)
                .expect("completed all-success record should recover");
            let identity = recovery.source_identity;

            assert_eq!(identity.client_id.as_deref(), Some("client-1"));
            assert_eq!(identity.client_type, client_type);
            assert_eq!(identity.item_id, "download-1");
        }
    }

    #[test]
    fn reconciliation_pending_manual_import_is_not_automatically_requeued() {
        let mut record = completed_manual_import_record("weaver", true);
        record.status = ImportStatus::Pending;
        let mut result = serde_json::from_str::<ManualImportExecutionResult>(
            record.result_json.as_deref().expect("result JSON"),
        )
        .expect("parse result JSON");
        result.status = ImportStatus::Pending;
        result.error_message =
            Some("Manual reconciliation required: filesystem worker was terminated".to_string());
        record.result_json = Some(serde_json::to_string(&result).expect("result JSON"));

        assert!(manual_import_record_requires_reconciliation(&record));
    }

    #[test]
    fn completed_manual_import_recovery_rejects_partial_failed_or_malformed_records() {
        let failed = completed_manual_import_record("nzbget", false);
        assert!(completed_manual_import_recovery(&failed).is_none());

        let mut empty = completed_manual_import_record("nzbget", true);
        let mut result = serde_json::from_str::<ManualImportExecutionResult>(
            empty.result_json.as_deref().expect("result JSON"),
        )
        .expect("parse result JSON");
        result.file_results.clear();
        empty.result_json = Some(serde_json::to_string(&result).expect("result JSON"));
        assert!(completed_manual_import_recovery(&empty).is_none());

        let mut malformed = completed_manual_import_record("nzbget", true);
        malformed.result_json = Some("not JSON".to_string());
        assert!(completed_manual_import_recovery(&malformed).is_none());

        let mut stale = completed_manual_import_record("nzbget", true);
        stale.source_client_id = Some(" ".to_string());
        assert!(completed_manual_import_recovery(&stale).is_none());
    }

    #[test]
    fn completed_manual_import_recovery_only_requires_the_queued_mappings() {
        let record = completed_manual_import_record("nzbget", true);
        assert!(completed_manual_import_recovery(&record).is_some());
    }

    #[test]
    fn completed_manual_import_recovery_dates_the_record_by_finished_at_then_updated_at() {
        let mut record = completed_manual_import_record("nzbget", true);
        record.updated_at = "2026-08-17T10:05:00+00:00".to_string();
        record.finished_at = Some("2026-08-17T10:00:00Z".to_string());
        assert_eq!(
            completed_manual_import_recovery(&record)
                .expect("recoverable")
                .record_completed_at,
            "2026-08-17T10:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("finished_at")
        );

        record.finished_at = None;
        assert_eq!(
            completed_manual_import_recovery(&record)
                .expect("recoverable")
                .record_completed_at,
            "2026-08-17T10:05:00Z"
                .parse::<DateTime<Utc>>()
                .expect("updated_at")
        );

        // A record that cannot be dated cannot prove it predates the tracked
        // download's completion; it is not recoverable.
        record.finished_at = Some("not a time".to_string());
        record.updated_at = "also not a time".to_string();
        assert!(completed_manual_import_recovery(&record).is_none());
    }

    fn skipped_movie_extra(path: &str) -> ManualImportFileResult {
        ManualImportFileResult {
            file_path: path.to_string(),
            episode_id: None,
            series_movie_link_id: None,
            success: false,
            skipped: true,
            dest_path: None,
            error_code: None,
            error_message: Some("skipped: not the primary movie file".to_string()),
        }
    }

    #[test]
    fn completed_manual_import_recovery_ignores_skipped_movie_extras() {
        let mut record = completed_manual_import_record("qbittorrent", true);
        let mut result = serde_json::from_str::<ManualImportExecutionResult>(
            record.result_json.as_deref().expect("result JSON"),
        )
        .expect("parse result JSON");
        result
            .file_results
            .push(skipped_movie_extra("/downloads/sample.mkv"));
        result
            .file_results
            .push(skipped_movie_extra("/downloads/featurette.mkv"));
        record.result_json = Some(serde_json::to_string(&result).expect("result JSON"));

        assert!(
            completed_manual_import_recovery(&record).is_some(),
            "skipped extras beside a successful primary must not block recovery"
        );

        // Skipped mappings alone prove nothing was imported.
        let mut only_skipped = completed_manual_import_record("qbittorrent", true);
        let mut result = serde_json::from_str::<ManualImportExecutionResult>(
            only_skipped.result_json.as_deref().expect("result JSON"),
        )
        .expect("parse result JSON");
        result.file_results = vec![skipped_movie_extra("/downloads/sample.mkv")];
        only_skipped.result_json = Some(serde_json::to_string(&result).expect("result JSON"));
        assert!(completed_manual_import_recovery(&only_skipped).is_none());
    }

    #[test]
    fn manual_import_file_result_json_without_skipped_field_deserializes_as_attempted() {
        let legacy = serde_json::json!({
            "file_path": "/downloads/episode.mkv",
            "success": true,
            "dest_path": "/library/episode.mkv",
            "error_code": null,
            "error_message": null
        });
        let result: ManualImportFileResult =
            serde_json::from_value(legacy).expect("legacy result JSON should deserialize");
        assert!(!result.skipped);
        assert!(result.success);
    }

    fn attempted(path: &str, success: bool, message: Option<&str>) -> ManualImportFileResult {
        ManualImportFileResult {
            file_path: path.to_string(),
            episode_id: None,
            series_movie_link_id: None,
            success,
            skipped: false,
            dest_path: success.then(|| format!("/library/{path}")),
            error_code: (!success).then_some(ImportErrorCode::PolicyMismatch),
            error_message: message.map(str::to_string),
        }
    }

    #[test]
    fn manual_import_terminal_status_completes_when_every_attempted_mapping_succeeded() {
        let (status, code, message) = manual_import_terminal_status_and_error(&[
            attempted("movie.mkv", true, None),
            skipped_movie_extra("sample.mkv"),
            skipped_movie_extra("featurette.mkv"),
        ]);
        assert_eq!(status, ImportStatus::Completed);
        assert_eq!(code, None);
        assert_eq!(message, None);
    }

    #[test]
    fn manual_import_terminal_status_fails_on_any_attempted_failure_and_on_nothing_imported() {
        let (status, code, message) = manual_import_terminal_status_and_error(&[
            attempted("movie.mkv", true, None),
            attempted("other.mkv", false, Some("no space left on device")),
            skipped_movie_extra("sample.mkv"),
        ]);
        assert_eq!(status, ImportStatus::Failed);
        assert_eq!(code, Some(ImportErrorCode::PolicyMismatch));
        assert_eq!(message.as_deref(), Some("no space left on device"));

        let (status, code, message) =
            manual_import_terminal_status_and_error(&[skipped_movie_extra("sample.mkv")]);
        assert_eq!(status, ImportStatus::Failed);
        assert_eq!(code, Some(ImportErrorCode::Unknown));
        assert_eq!(
            message.as_deref(),
            Some("manual import did not import any file")
        );

        let (status, ..) = manual_import_terminal_status_and_error(&[]);
        assert_eq!(status, ImportStatus::Failed);
    }
}

#[cfg(test)]
mod manual_movie_primary_selection_tests {
    use super::*;

    fn write_video(dir: &Path, name: &str, len: u64) -> ManualImportFileMapping {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).expect("create video");
        file.set_len(len).expect("size video");
        ManualImportFileMapping {
            file_path: path_to_stored_string(&path),
            episode_id: None,
            series_movie_link_id: None,
        }
    }

    const PAST_SAMPLE_THRESHOLD: u64 = 64 * 1024 * 1024;

    #[test]
    fn movie_primary_is_the_largest_non_sample_mapping() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical root");
        // Deliberately not the largest: a movie sample can be big, and a
        // trailer bigger than the film is a data error, not the primary.
        let sample = write_video(
            &root,
            "Movie.2024.1080p-sample.mkv",
            PAST_SAMPLE_THRESHOLD + 2,
        );
        let movie = write_video(&root, "Movie.2024.1080p.mkv", PAST_SAMPLE_THRESHOLD + 1);
        let featurette = write_video(&root, "Making.Of.mkv", 1024);
        let files = vec![sample, movie, featurette];

        assert_eq!(
            select_manual_movie_primary_index(&files, &MediaFacet::Movie, &root),
            Some(1)
        );
    }

    #[test]
    fn movie_primary_prefers_earliest_mapping_on_size_ties_and_ignores_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical root");
        let missing = ManualImportFileMapping {
            file_path: path_to_stored_string(root.join("gone.mkv")),
            episode_id: None,
            series_movie_link_id: None,
        };
        let first = write_video(&root, "Movie.2024.1080p.mkv", PAST_SAMPLE_THRESHOLD);
        let second = write_video(&root, "Movie.2024.1080p.PROPER.mkv", PAST_SAMPLE_THRESHOLD);
        let files = vec![missing, first, second];

        assert_eq!(
            select_manual_movie_primary_index(&files, &MediaFacet::Movie, &root),
            Some(1)
        );
    }

    #[test]
    fn movie_primary_is_none_when_every_mapping_is_named_as_a_sample() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical root");
        let named_sample = write_video(&root, "Movie.2024.sample.mkv", PAST_SAMPLE_THRESHOLD);
        let shouting_sample = write_video(&root, "Movie.2024.SAMPLE.Trailer.mkv", 1024);
        let files = vec![named_sample, shouting_sample];

        assert_eq!(
            select_manual_movie_primary_index(&files, &MediaFacet::Movie, &root),
            None
        );
    }

    #[test]
    fn movie_primary_accepts_a_small_normally_named_movie() {
        // The automatic movie path never size-filters; a short film, old
        // cartoon, or low-bitrate SD file well under the 50 MB heuristic must
        // stay importable by hand.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical root");
        const { assert!(1024 * 1024 < SAMPLE_SIZE_THRESHOLD) };
        let small_movie = write_video(&root, "Short.Film.1998.480p.DVDRip.mkv", 1024 * 1024);
        assert!(is_sample_file(Path::new(&small_movie.file_path)));
        assert!(!is_sample_named_file(Path::new(&small_movie.file_path)));

        assert_eq!(
            select_manual_movie_primary_index(
                std::slice::from_ref(&small_movie),
                &MediaFacet::Movie,
                &root
            ),
            Some(0)
        );

        // A sample-named file beside a bigger main file is still not the primary.
        let sample = write_video(&root, "Short.Film.1998.480p.DVDRip-sample.mkv", 2048);
        let files = vec![sample, small_movie];
        assert_eq!(
            select_manual_movie_primary_index(&files, &MediaFacet::Movie, &root),
            Some(1)
        );
    }

    #[test]
    fn movie_primary_only_considers_mappings_that_address_the_movie() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical root");
        let mut episode_mapping = write_video(&root, "Movie.2024.1080p.mkv", PAST_SAMPLE_THRESHOLD);
        episode_mapping.episode_id = Some("episode-1".to_string());
        let files = vec![episode_mapping];

        assert_eq!(
            select_manual_movie_primary_index(&files, &MediaFacet::Movie, &root),
            None
        );
    }
}

#[cfg(all(test, unix))]
mod manual_source_validation_tests {
    use super::*;

    #[test]
    fn manual_import_source_validation_accepts_symlink_with_path_and_target_under_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        std::fs::create_dir_all(&root).expect("create root");
        let target = root.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = root.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect("symlink inside root should validate");
    }

    #[test]
    fn manual_import_source_validation_rejects_symlink_path_outside_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&outside).expect("create outside");
        let target = root.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = outside.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        let error = validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect_err("symlink path outside root should be rejected");

        assert!(
            error
                .to_string()
                .contains("file path is outside the trusted source root")
        );
    }

    #[test]
    fn manual_import_source_validation_rejects_symlink_target_outside_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&outside).expect("create outside");
        let target = outside.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = root.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        let error = validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect_err("symlink target outside root should be rejected");

        assert!(
            error
                .to_string()
                .contains("file is outside the trusted source root")
        );
    }
}
