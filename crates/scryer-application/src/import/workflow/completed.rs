/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .subtitle_settings()
            .await
            .ok()
            .map(|settings| settings.auto_download_on_import)
            .unwrap_or(false);
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}

async fn analyze_and_persist_imported_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    file_path: &std::path::Path,
) {
    let acceptance = match app
        .services
        .library
        .media_analyzer
        .analyze_file(file_path.to_path_buf())
        .await
    {
        Ok(crate::MediaAnalysisOutcome::Valid(analysis)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: Some(*analysis),
                scan_error: None,
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Ok(crate::MediaAnalysisOutcome::Invalid(error)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id,
                file_id = %media_file_id,
                file_path = %file_path.display(),
                "failed to analyze imported media file"
            );
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
    };

    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        media_file_id,
        &acceptance,
    )
    .await;
}

fn completed_download_identity(completed: &CompletedDownload) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    )
}
async fn completed_import_purpose(
    app: &AppUseCase,
    completed: &CompletedDownload,
) -> crate::DownloadSubmissionPurpose {
    let identity = completed_download_identity(completed);
    if let Ok(Some(submission)) = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(&identity)
        .await
    {
        return submission.purpose;
    }

    extract_parameter(&completed.parameters, "*scryer_import_purpose")
        .as_deref()
        .map(crate::DownloadSubmissionPurpose::from_label)
        .unwrap_or_default()
}
fn additional_import_dest_path(
    canonical_dest_path: &Path,
    parsed: &ParsedReleaseMetadata,
) -> PathBuf {
    let parent = canonical_dest_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = canonical_dest_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("additional");
    let extension = canonical_dest_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let raw_label = parsed
        .edition
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(parsed.raw_title.as_str());
    let sanitized_label = sanitize_filesystem_component(raw_label)
        .trim()
        .chars()
        .take(48)
        .collect::<String>();
    let label = if sanitized_label.is_empty() {
        "additional".to_string()
    } else {
        sanitized_label
    };
    let hash = blake3::hash(parsed.raw_title.as_bytes()).to_hex();
    let hash = &hash.as_str()[..8];
    let base_name = sanitize_filesystem_component(&format!("{stem} - {label} {hash}.{extension}"));
    let mut candidate = parent.join(&base_name);
    if !candidate.exists() {
        return candidate;
    }

    for suffix in 2..=999 {
        let name =
            sanitize_filesystem_component(&format!("{stem} - {label} {hash} ({suffix}).{extension}"));
        candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(sanitize_filesystem_component(&format!(
        "{stem} - {label} {hash} {}.{extension}",
        Id::new().0
    )))
}
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";
const SCRYER_FACET_PARAM: &str = "*scryer_facet";
const SCRYER_COLLECTION_ID_PARAM: &str = "*scryer_collection_id";
const SCRYER_SERIES_MOVIE_LINK_ID_PARAM: &str = "*scryer_series_movie_link_id";
const COMPLETED_ORIGIN_SCOPE_CONFLICT: &str = "origin_scope_conflict";

#[derive(Clone, Debug)]
enum CompletedDownloadOriginResolution {
    Ready(CompletedDownload),
    Conflict {
        reason: &'static str,
        detail: String,
    },
    NoScryerOrigin,
}

fn resolve_completed_download_origin(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> CompletedDownloadOriginResolution {
    match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched)
            if submission_has_scryer_origin(&matched.submission) =>
        {
            match reconciled_scryer_origin_parameters(
                &completed.parameters,
                &matched.submission,
            ) {
                Ok(parameters) => {
                    let mut resolved = completed.clone();
                    resolved.parameters = parameters;
                    CompletedDownloadOriginResolution::Ready(resolved)
                }
                Err(detail) => CompletedDownloadOriginResolution::Conflict {
                    reason: COMPLETED_ORIGIN_SCOPE_CONFLICT,
                    detail,
                },
            }
        }
        _ if has_scryer_origin(&completed.parameters) => {
            CompletedDownloadOriginResolution::Ready(completed.clone())
        }
        _ => CompletedDownloadOriginResolution::NoScryerOrigin,
    }
}

fn reconciled_scryer_origin_parameters(
    parameters: &[(String, String)],
    submission: &DownloadSubmission,
) -> Result<Vec<(String, String)>, String> {
    let mut reconciled = parameters.to_vec();
    fill_missing_or_compatible_parameter(
        &mut reconciled,
        SCRYER_TITLE_ID_PARAM,
        &submission.title_id,
        "title id",
    )?;
    fill_missing_or_compatible_parameter(
        &mut reconciled,
        SCRYER_FACET_PARAM,
        &submission.facet,
        "facet",
    )?;
    reconcile_submission_scope_parameters(&mut reconciled, &submission.scope)?;
    Ok(reconciled)
}

fn reconcile_submission_scope_parameters(
    parameters: &mut Vec<(String, String)>,
    scope: &SubmissionScope,
) -> Result<(), String> {
    match scope {
        SubmissionScope::Collection { collection_id } => {
            reject_existing_scope_parameter(
                parameters,
                SCRYER_SERIES_MOVIE_LINK_ID_PARAM,
                "series movie link id",
                "collection",
            )?;
            fill_missing_or_compatible_parameter(
                parameters,
                SCRYER_COLLECTION_ID_PARAM,
                collection_id,
                "collection id",
            )
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => fill_missing_or_compatible_parameter(
            parameters,
            SCRYER_SERIES_MOVIE_LINK_ID_PARAM,
            series_movie_link_id,
            "series movie link id",
        ),
        SubmissionScope::Episode { .. }
        | SubmissionScope::EpisodeSet { .. }
        | SubmissionScope::Title
        | SubmissionScope::Orphan => {
            reject_existing_scope_parameter(
                parameters,
                SCRYER_COLLECTION_ID_PARAM,
                "collection id",
                "non-collection",
            )?;
            reject_existing_scope_parameter(
                parameters,
                SCRYER_SERIES_MOVIE_LINK_ID_PARAM,
                "series movie link id",
                "non-series-movie",
            )
        }
    }
}

fn fill_missing_or_compatible_parameter(
    parameters: &mut Vec<(String, String)>,
    key: &str,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let expected = expected.trim();
    if expected.is_empty() {
        return Ok(());
    }

    if let Some(existing) = non_empty_parameter_value(parameters, key)
        && existing != expected
    {
        return Err(format!(
            "completed download carried {label} {existing:?}, but matched submission expected {expected:?}"
        ));
    }

    insert_missing_or_empty_parameter(parameters, key, expected.to_string());
    Ok(())
}

fn reject_existing_scope_parameter(
    parameters: &[(String, String)],
    key: &str,
    label: &str,
    expected_scope: &str,
) -> Result<(), String> {
    if let Some(existing) = non_empty_parameter_value(parameters, key) {
        return Err(format!(
            "completed download carried {label} {existing:?}, but matched submission expected {expected_scope} scope"
        ));
    }
    Ok(())
}

fn non_empty_parameter_value(parameters: &[(String, String)], key: &str) -> Option<String> {
    parameters
        .iter()
        .find(|(candidate_key, _)| candidate_key == key)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn insert_missing_or_empty_parameter(
    parameters: &mut Vec<(String, String)>,
    key: &str,
    value: String,
) {
    if let Some((_, existing_value)) = parameters.iter_mut().find(|(name, _)| name == key) {
        if existing_value.trim().is_empty() {
            *existing_value = value;
        }
    } else {
        parameters.push((key.to_string(), value));
    }
}
async fn persist_completed_download_tracked_state(
    app: &AppUseCase,
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
    state: TrackedDownloadState,
) {
    if !state.is_terminal() {
        return;
    }
    let state_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => {
            submission_source_identity(&matched.submission)
        }
        _ => completed_download_identity(completed),
    };

    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(&state_identity, state.as_str())
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            tracked_state_client_item_id = state_identity.item_id.as_str(),
            state = state.as_str(),
            "failed to persist completed download terminal state"
        );
    }

    let observed_identity = completed_download_observed_identity(completed);
    let download_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => matched
            .identity
            .clone()
            .filter(|identity| !download_submission_identity_is_empty(identity))
            .or_else(|| {
                (!download_submission_identity_is_empty(&observed_identity))
                    .then_some(observed_identity.clone())
            }),
        CompletedDownloadSubmissionResolution::MissingDownloadId { identity } => {
            Some(identity.clone())
        }
        _ => (!download_submission_identity_is_empty(&observed_identity))
            .then_some(observed_identity.clone()),
    };

    if let Some(download_identity) = download_identity
        && let Err(error) = app
            .services
            .workflow
            .download_submissions
            .record_identity_tracked_state(
                &download_identity,
                Some(&completed_download_identity(completed)),
                state.as_str(),
                None,
                None,
            )
            .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            state = state.as_str(),
            "failed to persist durable completed download terminal state"
        );
    }
}
async fn terminal_download_item_is_still_visible(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
) -> bool {
    let lookup = if is_history {
        app.services
            .integrations
            .download_client
            .list_history()
            .await
    } else {
        app.services.integrations.download_client.list_queue().await
    };

    match lookup {
        Ok(items) => items.iter().any(|item| {
            item.download_client_item_id == download_client_item_id
                && item.client_type.eq_ignore_ascii_case(client_type)
                && (client_id.is_empty() || item.client_id.trim() == client_id)
        }),
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                "failed to confirm download item visibility after delete error"
            );
            true
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDownloadCleanupOutcome {
    NotConfigured,
    Removed,
    AlreadyGone,
    RetryableFailure,
}
pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
    )
}
async fn cleanup_routing_scope_for_title_id(
    app: &AppUseCase,
    title_id: Option<&str>,
) -> (Option<String>, Option<MediaFacet>) {
    let Some(title_id) = title_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => (Some(title.library_id), Some(title.facet)),
        Ok(None) | Err(_) => (None, None),
    }
}
pub(crate) async fn reconcile_terminal_download_cleanup_for_completed(
    app: &AppUseCase,
    completed: &CompletedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let title_id = extract_parameter(&completed.parameters, "*scryer_title_id").unwrap_or_default();
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, Some(title_id.as_str())).await;
    let facet = resolved_facet.or_else(|| facet_for_completed_download(completed));
    reconcile_terminal_download_cleanup(
        app,
        &completed.client_id,
        &completed.client_type,
        &completed.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
    )
    .await
}
pub(crate) async fn reconcile_terminal_download_cleanup_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, tracked.title_id.as_deref()).await;
    let facet = resolved_facet.or_else(|| facet_from_tracked_label(tracked.facet.as_deref()));
    reconcile_terminal_download_cleanup(
        app,
        &tracked.client_id,
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
    )
    .await
}
fn media_file_score(file: &crate::TitleMediaFile) -> i32 {
    file.acquisition_score.unwrap_or(0)
}
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "active-download marker",
        "still being unpacked",
        "still_unpacking",
        "source changed",
        "locked",
        "temporarily",
        "not found or inaccessible",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
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
        .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
            title_tags: &title.tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: title.imdb_id.as_deref(),
            tvdb_id,
            category_hint: Some(category_hint),
        })
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
const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024;
fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}
