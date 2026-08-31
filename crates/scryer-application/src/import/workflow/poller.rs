pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::RequireSubmission,
        None,
        None,
        None,
    )
    .await
}

pub async fn import_completed_download_for_manual_review(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::AllowUnresolved,
        None,
        None,
        None,
    )
    .await
}

pub(crate) async fn import_completed_download_for_manual_review_with_title_override(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    title_id: &str,
    release_evidence: Option<&ReleaseEvidence>,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::AllowUnresolved,
        Some(title_id),
        release_evidence,
        None,
    )
    .await
}

pub(crate) async fn import_completed_download_for_tracked(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    target_title_id: Option<&str>,
    release_evidence: Option<&ReleaseEvidence>,
    preparation_permit: tokio::sync::OwnedSemaphorePermit,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy_for_download(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::RequireSubmission,
        target_title_id,
        release_evidence,
        canonical_download_id,
        Some(preparation_permit),
    )
    .await
}

/// Who chose the requested target title, which decides how a disagreement with a
/// durable Scryer submission is settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletedImportIdentityPolicy {
    /// Automatic import: the target (if any) is the tracked download's
    /// validated title. A Scryer submission is authoritative over it — the
    /// submission wins and the disagreement is logged, never an error.
    RequireSubmission,
    /// Manual review: the target is an operator's explicit choice. It is
    /// passed through as-is so `resolve_completed_import_target` rejects a
    /// choice outside the durable Scryer submission title.
    AllowUnresolved,
}

async fn import_completed_download_with_identity_policy(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    identity_policy: CompletedImportIdentityPolicy,
    target_title_id: Option<&str>,
    release_evidence: Option<&ReleaseEvidence>,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy_for_download(
        app,
        actor,
        completed,
        identity_policy,
        target_title_id,
        release_evidence,
        None,
        preparation_permit,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "import orchestration keeps canonical ownership and preparation permits explicit"
)]
async fn import_completed_download_with_identity_policy_for_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    identity_policy: CompletedImportIdentityPolicy,
    target_title_id: Option<&str>,
    release_evidence: Option<&ReleaseEvidence>,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> AppResult<ImportResult> {
    let request = match prepare_completed_import_request(
        app,
        completed,
        identity_policy,
        target_title_id,
        release_evidence,
        canonical_download_id,
    )
    .await?
    {
        CompletedImportProgress::Ready(request) => request,
        CompletedImportProgress::Finished(result) => return Ok(result),
    };
    let request = match validate_completed_import_source_and_mark_processing(app, request).await? {
        CompletedImportProgress::Ready(request) => request,
        CompletedImportProgress::Finished(result) => return Ok(result),
    };

    execute_completed_import(app, actor, request, preparation_permit).await
}

struct CompletedImportRequest {
    completed: CompletedDownload,
    release_evidence: ReleaseEvidence,
    target_title_id: Option<String>,
    import_id: String,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompletedImportRequestPayload {
    completed: CompletedDownload,
    release_evidence: ReleaseEvidence,
    /// The title the import must land in when the release evidence carries no
    /// Scryer identity: the tracked download's validated title for automatic
    /// imports, or the operator's chosen title for manual review. Persisted so
    /// a retry after the tracked download is gone still lands in it.
    #[serde(default, alias = "manual_title_id")]
    target_title_id: Option<String>,
}

/// Where the release evidence for an import attempt came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletedImportEvidenceSource {
    /// The caller resolved it itself (tracked auto-import, manual selection).
    Override,
    /// A live `download_submissions` row for the completed download.
    FreshRow,
    /// No row exists any more; the evidence an earlier attempt persisted.
    Persisted,
    /// Nothing durable: the client-reported observation.
    FreshObservation,
}

struct CompletedImportEvidenceInputs<'a> {
    identity_policy: CompletedImportIdentityPolicy,
    /// The live submission lookup, or `None` when it failed transiently.
    fresh_resolution: Option<&'a CompletedDownloadSubmissionResolution>,
    release_evidence_override: Option<&'a ReleaseEvidence>,
    persisted_release_evidence: Option<&'a ReleaseEvidence>,
    persisted_target_title_id: Option<&'a str>,
    requested_target_title_id: Option<&'a str>,
    completed: &'a CompletedDownload,
}

struct SelectedCompletedImportEvidence {
    release_evidence: ReleaseEvidence,
    target_title_id: Option<String>,
    source: CompletedImportEvidenceSource,
}

fn non_empty_title_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Chooses the release evidence and target title for an import attempt.
///
/// A live `download_submissions` row is authoritative over anything an earlier
/// attempt persisted: after an operator reassignment the row is rewritten to
/// name the new title, and a retry that replayed the stale persisted
/// `ScryerSubmission` would import into the old title. Persisted evidence is
/// used only when the row is gone (lost) or the live lookup failed
/// transiently. The persisted target follows the same rule: a live row that
/// names a title supersedes it.
fn select_completed_import_evidence(
    inputs: CompletedImportEvidenceInputs<'_>,
) -> SelectedCompletedImportEvidence {
    let fresh_match = inputs
        .fresh_resolution
        .and_then(|resolution| match resolution {
            CompletedDownloadSubmissionResolution::Matched(matched) => Some(matched.as_ref()),
            _ => None,
        });

    let (release_evidence, source) = if let Some(evidence) = inputs.release_evidence_override {
        (evidence.clone(), CompletedImportEvidenceSource::Override)
    } else if let (Some(resolution), Some(_)) = (inputs.fresh_resolution, fresh_match) {
        (
            release_evidence_for_resolution(inputs.completed, resolution),
            CompletedImportEvidenceSource::FreshRow,
        )
    } else if let Some(persisted) = inputs.persisted_release_evidence {
        (persisted.clone(), CompletedImportEvidenceSource::Persisted)
    } else {
        (
            ReleaseEvidence::from_completed_observation(inputs.completed),
            CompletedImportEvidenceSource::FreshObservation,
        )
    };

    let requested_target_title_id = non_empty_title_id(inputs.requested_target_title_id);
    let target_title_id = match release_evidence.title_id() {
        Some(submission_title_id) => match (inputs.identity_policy, requested_target_title_id) {
            (_, None) => None,
            (CompletedImportIdentityPolicy::AllowUnresolved, Some(requested)) => {
                Some(requested.to_string())
            }
            (CompletedImportIdentityPolicy::RequireSubmission, Some(requested)) => {
                if requested != submission_title_id {
                    tracing::warn!(
                        client_id = %inputs.completed.client_id,
                        client_type = %inputs.completed.client_type,
                        download_client_item_id = %inputs.completed.download_client_item_id,
                        requested_title_id = requested,
                        submission_title_id,
                        "import: requested target title disagrees with the durable Scryer submission; importing into the submission title"
                    );
                }
                None
            }
        },
        None => requested_target_title_id
            .map(str::to_string)
            .or_else(|| {
                // A live titled row without Scryer origin (defensive: a titled
                // orphan seen before it round-trips through the store) still
                // names the operator's choice, which outranks a stale target.
                fresh_match
                    .and_then(|matched| non_empty_title_id(Some(&matched.submission.title_id)))
                    .map(str::to_string)
            })
            .or_else(|| {
                non_empty_title_id(inputs.persisted_target_title_id).map(str::to_string)
            })
            .or_else(|| {
                // Last resort, when both the submission row and the tracked
                // state are gone: `*scryer_title_id` is Scryer's own stamp,
                // written only at Scryer add time (NZBGet PP parameter) or by
                // `with_tracked_metadata`, and 0.18.12 honored it directly.
                // It never outranks a Scryer submission title (handled above)
                // or any of the earlier target sources.
                extract_parameter(&inputs.completed.parameters, SCRYER_TITLE_ID_PARAM)
                    .as_deref()
                    .and_then(|value| non_empty_title_id(Some(value)))
                    .map(str::to_string)
            }),
    };

    SelectedCompletedImportEvidence {
        release_evidence,
        target_title_id,
        source,
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredCompletedImportRequestPayload {
    Current(CompletedImportRequestPayload),
    Legacy(CompletedDownload),
}

/// One provenance resolution for an import attempt, however it was reached
/// (tracked auto-import, poller/manual import, retry of a failed record).
struct ImportProvenanceRequest<'a> {
    identity_policy: CompletedImportIdentityPolicy,
    /// The live queue item when the caller has one (tracked auto-import); it
    /// sharpens the submission lookup by download id.
    queue_item: Option<&'a DownloadQueueItem>,
    requested_target_title_id: Option<&'a str>,
    release_evidence_override: Option<&'a ReleaseEvidence>,
    /// What an earlier attempt persisted for this identity, if any.
    persisted: Option<&'a CompletedImportRequestPayload>,
    /// Retry: a transient live-lookup failure falls back to the persisted
    /// evidence instead of failing the attempt.
    tolerate_lookup_failure: bool,
}

struct ImportProvenance {
    /// The completed download with its Scryer-origin parameters made
    /// authoritative (from the live submission, or the replayed attempt).
    completed: CompletedDownload,
    /// The live submission lookup, ambiguity/missing-id already downgraded to
    /// an observation; `None` only when the lookup failed transiently and the
    /// persisted evidence carried the attempt.
    submission_resolution: Option<CompletedDownloadSubmissionResolution>,
    release_evidence: ReleaseEvidence,
    target_title_id: Option<String>,
}

/// The single resolve → downgrade → select → stamp sequence every import
/// entry point runs; keeping it in one place is what stops the tracked path,
/// the poller path, and the retry path from disagreeing about where a
/// download came from.
async fn resolve_import_provenance(
    app: &AppUseCase,
    mut completed: CompletedDownload,
    request: ImportProvenanceRequest<'_>,
) -> AppResult<ImportProvenance> {
    let submission_resolution =
        match resolve_completed_download_submission(app, &completed, request.queue_item).await {
            Ok(resolution) => Some(downgrade_unresolved_submission_identity(
                &completed, resolution,
            )),
            Err(error) => {
                if !request.tolerate_lookup_failure || request.persisted.is_none() {
                    return Err(error);
                }
                tracing::warn!(
                    client_id = %completed.client_id,
                    client_type = %completed.client_type,
                    download_client_item_id = %completed.download_client_item_id,
                    error = %error,
                    "import: live submission lookup failed; continuing with the persisted release evidence"
                );
                None
            }
        };
    let SelectedCompletedImportEvidence {
        release_evidence,
        target_title_id,
        source: evidence_source,
    } = select_completed_import_evidence(CompletedImportEvidenceInputs {
        identity_policy: request.identity_policy,
        fresh_resolution: submission_resolution.as_ref(),
        release_evidence_override: request.release_evidence_override,
        persisted_release_evidence: request.persisted.map(|payload| &payload.release_evidence),
        persisted_target_title_id: request
            .persisted
            .and_then(|payload| payload.target_title_id.as_deref()),
        requested_target_title_id: request.requested_target_title_id,
        completed: &completed,
    });
    // The Scryer origin parameters follow the evidence: a live Scryer submission
    // stamps its own (authoritative, idempotent with what callers already
    // applied); evidence replayed from an earlier attempt because the row is
    // gone brings that attempt's parameters along.
    if let Some(CompletedDownloadSubmissionResolution::Matched(matched)) =
        submission_resolution.as_ref()
        && submission_has_scryer_origin(&matched.submission)
    {
        completed = stamp_scryer_submission_origin(&completed, &matched.submission);
    } else if let Some(payload) = request.persisted
        && evidence_source == CompletedImportEvidenceSource::Persisted
        && matches!(release_evidence, ReleaseEvidence::ScryerSubmission { .. })
    {
        completed.parameters = payload.completed.parameters.clone();
    }
    Ok(ImportProvenance {
        completed,
        submission_resolution,
        release_evidence,
        target_title_id,
    })
}

/// A completion that cannot resolve a durable Scryer submission is an
/// observation, not another application's property. It remains eligible for
/// the normal import flow using canonical NZB evidence.
fn downgrade_unresolved_submission_identity(
    completed: &CompletedDownload,
    resolution: CompletedDownloadSubmissionResolution,
) -> CompletedDownloadSubmissionResolution {
    match resolution {
        CompletedDownloadSubmissionResolution::AmbiguousDownloadId {
            download_id,
            matches,
        } => {
            tracing::warn!(
                client_id = %completed.client_id,
                client_type = %completed.client_type,
                download_client_item_id = %completed.download_client_item_id,
                download_id,
                matches,
                "import: ambiguous submission identity; importing as a downloader observation"
            );
            CompletedDownloadSubmissionResolution::DownloaderObservation
        }
        CompletedDownloadSubmissionResolution::MissingDownloadId { identity } => {
            tracing::debug!(
                client_id = %completed.client_id,
                client_type = %completed.client_type,
                download_client_item_id = %completed.download_client_item_id,
                download_id = ?identity.download_id,
                "import: no durable submission for the download id; importing as a downloader observation"
            );
            CompletedDownloadSubmissionResolution::DownloaderObservation
        }
        resolution => resolution,
    }
}

enum CompletedImportProgress {
    Ready(CompletedImportRequest),
    Finished(ImportResult),
}

async fn prepare_completed_import_request(
    app: &AppUseCase,
    completed: &CompletedDownload,
    identity_policy: CompletedImportIdentityPolicy,
    target_title_id: Option<&str>,
    release_evidence_override: Option<&ReleaseEvidence>,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
) -> AppResult<CompletedImportProgress> {
    let mut completed = completed.clone();
    remap_completed_download_for_client(app, &mut completed).await;
    let started_at = Utc::now();
    let source_identity = completed_download_identity(&completed);
    let persisted_request = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(std::slice::from_ref(&source_identity))
        .await?
        .into_iter()
        .find_map(|record| {
            serde_json::from_str::<StoredCompletedImportRequestPayload>(&record.payload_json)
                .ok()
                .and_then(|payload| match payload {
                    StoredCompletedImportRequestPayload::Current(payload) => Some(payload),
                    // A 0.18.12 payload carries no evidence to replay.
                    StoredCompletedImportRequestPayload::Legacy(_) => None,
                })
        });
    let ImportProvenance {
        completed,
        submission_resolution,
        release_evidence,
        target_title_id: resolved_target_title_id,
    } = resolve_import_provenance(
        app,
        completed,
        ImportProvenanceRequest {
            identity_policy,
            queue_item: None,
            requested_target_title_id: target_title_id,
            release_evidence_override,
            persisted: persisted_request.as_ref(),
            tolerate_lookup_failure: false,
        },
    )
    .await?;
    let submission_resolution = submission_resolution
        .unwrap_or(CompletedDownloadSubmissionResolution::DownloaderObservation);

    // Queue the import request for tracking
    let import_type = {
        let facet_str = extract_parameter(&completed.parameters, "*scryer_facet");
        let is_episode = facet_str
            .as_deref()
            .and_then(|f| app.facet_registry.all().find(|h| h.facet_id() == f))
            .is_some_and(|h| h.has_episodes());
        if is_episode {
            ImportType::SeriesDownload
        } else {
            ImportType::MovieDownload
        }
    };
    let import_id = app
        .services
        .workflow
        .imports
        .queue_import_request_with_identity_for_download(
            source_identity,
            import_type.as_str().to_string(),
            serde_json::to_string(&CompletedImportRequestPayload {
                completed: completed.clone(),
                release_evidence: release_evidence.clone(),
                target_title_id: resolved_target_title_id.clone(),
            })
            .unwrap_or_default(),
            completed_download_import_identity_for_resolution(&completed, &submission_resolution),
            canonical_download_id,
        )
        .await?;

    Ok(CompletedImportProgress::Ready(CompletedImportRequest {
        completed,
        release_evidence,
        target_title_id: resolved_target_title_id,
        import_id,
        started_at,
    }))
}

async fn validate_completed_import_source_and_mark_processing(
    app: &AppUseCase,
    request: CompletedImportRequest,
) -> AppResult<CompletedImportProgress> {
    // If the source directory no longer exists, the files were already moved
    // by a previous import (possibly under a different source_ref). Mark as
    // skipped so the poller never retries this entry.
    let source_ref = &request.completed.download_client_item_id;
    let source_path = std::path::Path::new(&request.completed.dest_dir);
    if !source_path.exists() {
        tracing::debug!(
            source_ref,
            dest_dir = %request.completed.dest_dir,
            "import: source directory no longer exists, no files to import"
        );
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            release_burned: false,
            ..base_completed_import_result(
                &request.import_id,
                &request.completed,
                &request.release_evidence,
                request.started_at,
            )
        };
        let result_json = serde_json::to_string(&result).ok();
        let _ = app
            .update_import_status_and_notify(&request.import_id, ImportStatus::Skipped, result_json)
            .await;
        return Ok(CompletedImportProgress::Finished(result));
    }

    // Mark as processing
    app.update_import_status_and_notify(&request.import_id, ImportStatus::Processing, None)
        .await?;

    Ok(CompletedImportProgress::Ready(request))
}

async fn execute_completed_import(
    app: &AppUseCase,
    actor: &User,
    request: CompletedImportRequest,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> AppResult<ImportResult> {
    // From here on, any error must update the import record to "failed" rather than
    // propagating via `?`. Otherwise the record stays "processing" indefinitely.
    match Box::pin(run_import(
        app,
        actor,
        &request.import_id,
        &request.completed,
        &request.release_evidence,
        request.target_title_id.as_deref(),
        request.started_at,
        None,
        preparation_permit,
    ))
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => finalize_completed_import_error(app, &request, error).await,
    }
}

async fn finalize_completed_import_error(
    app: &AppUseCase,
    request: &CompletedImportRequest,
    error: AppError,
) -> AppResult<ImportResult> {
    let requires_reconciliation = matches!(&error, AppError::ManualReconciliationRequired(_));
    let cancelled = matches!(&error, AppError::Canceled(_));
    let skip_reason = completed_import_error_skip_reason(&error);
    let result = ImportResult {
        decision: if cancelled {
            ImportDecision::Skipped
        } else {
            ImportDecision::Failed
        },
        skip_reason,
        error_message: Some(if cancelled {
            "Import was cancelled. Use Manual Import to resume it.".to_string()
        } else {
            error.to_string()
        }),
        release_burned: false,
        ..base_completed_import_result(
            &request.import_id,
            &request.completed,
            &request.release_evidence,
            request.started_at,
        )
    };
    let result_json = serde_json::to_string(&result).ok();
    // Decide retryability BEFORE the status write: a `Failed` write emits an
    // `ImportRejected` domain event (history + notifications), which must not
    // fire for an attempt the tracked layer is about to re-run automatically.
    let status = if cancelled {
        ImportStatus::Skipped
    } else {
        completed_import_status_for_result(&result, ImportStatus::Failed)
    };
    let _ = app
        .update_import_status_and_notify(&request.import_id, status, result_json)
        .await;
    if requires_reconciliation {
        return Err(error);
    }
    Ok(result)
}

fn completed_import_error_skip_reason(error: &AppError) -> Option<ImportSkipReason> {
    if crate::archive_extractor::is_password_required_error(error) {
        Some(ImportSkipReason::PasswordRequired)
    } else if matches!(error, AppError::ArchiveExtractionPluginRequired { .. }) {
        Some(ImportSkipReason::ArchiveExtractionPluginRequired)
    } else if crate::archive_extractor::is_timeout_error(error) {
        Some(ImportSkipReason::ArchiveExtractionTimedOut)
    } else {
        None
    }
}

#[cfg(test)]
mod poller_tests {
    use super::*;

    #[test]
    fn missing_archive_extractor_is_not_an_automatic_retry() {
        let error = AppError::archive_extraction_plugin_required(None);

        assert_eq!(
            completed_import_error_skip_reason(&error),
            Some(ImportSkipReason::ArchiveExtractionPluginRequired)
        );
    }
}
