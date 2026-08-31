use super::lookup::{
    find_completed_download, remap_completed_download_for_client, with_tracked_metadata,
};
use super::result_state::apply_import_result_with_completed;
use super::*;
use crate::AppError;
use crate::import_workflow::import_completed_download_for_tracked;
use scryer_logging::{ActorContext, LogContext, ResourceContext, WorkflowContext, context_span};
use tracing::Instrument;

pub(crate) fn mark_importing(td: &mut TrackedDownload) {
    td.state = TrackedDownloadState::Importing;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages = vec![IMPORT_RUNNING_MESSAGE.to_string()];
}

pub async fn import(app: &AppUseCase, actor: &User, td: &mut TrackedDownload) -> bool {
    let log_span = tracked_download_import_log_span(actor, td);
    import_inner(app, actor, td, None, None)
        .instrument(log_span)
        .await
}

#[cfg(test)]
pub(crate) async fn import_with_lookup(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
    completed_lookup: &CompletedDownloadLookup,
) -> bool {
    let log_span = tracked_download_import_log_span(actor, td);
    import_inner(app, actor, td, Some(completed_lookup), None)
        .instrument(log_span)
        .await
}

pub(crate) async fn import_with_lookup_and_preparation_permit(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
    completed_lookup: &CompletedDownloadLookup,
    preparation_permit: tokio::sync::OwnedSemaphorePermit,
) -> bool {
    let log_span = tracked_download_import_log_span(actor, td);
    import_inner(
        app,
        actor,
        td,
        Some(completed_lookup),
        Some(preparation_permit),
    )
    .instrument(log_span)
    .await
}

fn tracked_download_import_log_span(actor: &User, td: &TrackedDownload) -> tracing::Span {
    context_span(
        LogContext::workflow(WorkflowContext {
            kind: "import".to_owned(),
            id: td.id.clone(),
        })
        .with_actor(ActorContext {
            kind: if actor.is_system_execution_actor() {
                "system".to_owned()
            } else {
                "user".to_owned()
            },
            id: Some(actor.id.clone()),
            display_name: Some(actor.username.clone()),
            source: None,
        })
        .with_resource(ResourceContext {
            title_id: td.title_id.clone(),
            import_id: Some(td.id.clone()),
            download_id: td.client_item.download_id.clone(),
            client_id: Some(td.client_id.clone()),
            ..ResourceContext::default()
        }),
    )
}

async fn import_inner(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> bool {
    if td.state != TrackedDownloadState::ImportPending
        && td.state != TrackedDownloadState::Importing
    {
        return false;
    }

    let Some(completed) = resolve_completed_download_for_import(app, td, completed_lookup).await
    else {
        return false;
    };

    let preparation_permit = match preparation_permit {
        Some(permit) => permit,
        None => {
            app.runtime
                .imports
                .execution_coordinator
                .acquire_preparation()
                .await
        }
    };
    if td.state != TrackedDownloadState::Importing {
        mark_importing(td);
        crate::tracked_downloads::publish_runtime_tracked_download_snapshot(app, td).await;
    }

    let (completed, release_evidence) =
        match resolve_completed_download_origin_for_import(app, &completed, Some(&td.client_item))
            .await
        {
            Ok(ResolvedCompletedDownloadOriginForImport::Ready {
                completed,
                release_evidence,
            }) => (*completed, Some(release_evidence)),
            Ok(ResolvedCompletedDownloadOriginForImport::NoScryerOrigin) => (completed, None),
            Err(error) => {
                tracing::warn!(
                    id = %td.id,
                    item_id = %td.client_item.download_client_item_id,
                    error = %error,
                    "import: completed download origin resolution failed, will retry"
                );
                td.state = TrackedDownloadState::ImportPending;
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages = vec![
                    "Download origin could not be resolved yet; retrying automatically."
                        .to_string(),
                ];
                return false;
            }
        };

    match crate::import_workflow::recent_download_submission_persistence_is_pending(app, &completed)
        .await
    {
        Ok(true) => {
            tracing::info!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                download_id = ?td.client_item.download_id,
                "import: waiting for recent download submission identity to become durable"
            );
            td.import_attempted = false;
            td.state = TrackedDownloadState::ImportPending;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![
                "Download submission identity is still being persisted; retrying automatically."
                    .to_string(),
            ];
            return false;
        }
        Ok(false) => {}
        Err(_) => {}
    }

    tracing::info!(
        id = %td.id,
        dest_dir = %completed.dest_dir,
        title_id = ?td.title_id,
        "import: starting import from completed download"
    );

    let success_before = match total_successful_artifacts(app, td, Some(&completed)).await {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(id = %td.id, error = %error, "import artifact evidence unavailable before import");
            td.schedule_import_execution_retry(Utc::now(), |_, next_retry_at| {
                format!(
                    "Import verification is temporarily unavailable. Retrying at {}.",
                    next_retry_at.to_rfc3339()
                )
            });
            return false;
        }
    };
    td.import_attempted = true;

    let import_actor = actor_for_tracked_download_import(app, actor, td).await;
    let target_title_id = tracked_import_target_title_id(td, release_evidence.as_ref());
    let import = import_completed_download_for_tracked(
        app,
        &import_actor,
        &completed,
        td.canonical_download_id(),
        target_title_id.as_deref(),
        release_evidence.as_ref(),
        preparation_permit,
    )
    .await;
    match import {
        Ok(result) => {
            let success_after = match total_successful_artifacts(app, td, Some(&completed)).await {
                Ok(count) => count,
                Err(error) => {
                    tracing::warn!(id = %td.id, error = %error, "import artifact evidence unavailable after import");
                    td.schedule_import_execution_retry(Utc::now(), |_, next_retry_at| {
                        format!(
                            "Import verification is temporarily unavailable. Retrying at {}.",
                            next_retry_at.to_rfc3339()
                        )
                    });
                    return false;
                }
            };
            let files_imported_this_pass = success_after.saturating_sub(success_before) as usize;
            tracing::info!(
                id = %td.id,
                decision = ?result.decision,
                skip_reason = ?result.skip_reason,
                error_message = ?result.error_message,
                files_imported_this_pass,
                "import: pipeline returned result"
            );
            apply_import_result_with_completed(
                app,
                td,
                result,
                files_imported_this_pass,
                Some(&completed),
                release_evidence.as_ref(),
            )
            .await
        }
        Err(error) => {
            if let AppError::ManualReconciliationRequired(message) = &error {
                tracing::error!(
                    id = %td.id,
                    dest_dir = %completed.dest_dir,
                    error = %message,
                    "import filesystem worker was terminated; routing download to manual reconciliation"
                );
                td.clear_import_execution_retry();
                td.state = TrackedDownloadState::ImportBlocked;
                td.status = TrackedDownloadStatus::Error;
                td.status_messages = vec![message.clone()];
                return false;
            }
            // The pipeline itself erred before it could produce a result
            // (repository/DB failure while queueing or resolving the attempt).
            // That is an execution failure of an approved import, so it gets
            // the same automatic re-attempt as a `Failed` result — Sonarr
            // leaves the item in place and re-processes it on the next
            // refresh; nothing here warrants a sticky manual-review block.
            tracing::warn!(
                id = %td.id,
                error = %error,
                dest_dir = %completed.dest_dir,
                "import: pipeline returned error; scheduling automatic retry"
            );
            td.schedule_import_execution_retry(Utc::now(), |attempts, next_retry_at| {
                format!(
                    "Import failed: {error} Retrying automatically (attempt {attempts}) at {}.",
                    next_retry_at.to_rfc3339()
                )
            });
            false
        }
    }
}

/// The title the import must land in for this tracked download.
///
/// A durable Scryer submission (the only release evidence
/// `resolve_completed_download_origin_for_import` returns) is authoritative:
/// its title drives target resolution and no separate target is passed. For a
/// downloader observation — a parse match the completed-check proved, or an
/// operator assignment — the tracked download's validated title is the target,
/// so the import does not re-derive it from a context-free parse of the
/// release name and land elsewhere.
pub(super) fn tracked_import_target_title_id(
    td: &TrackedDownload,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> Option<String> {
    let tracked_title_id = td
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match release_evidence.and_then(crate::import_workflow::ReleaseEvidence::title_id) {
        Some(submission_title_id) => {
            if let Some(tracked_title_id) = tracked_title_id
                && tracked_title_id != submission_title_id
            {
                tracing::warn!(
                    id = %td.id,
                    item_id = %td.client_item.download_client_item_id,
                    tracked_title_id,
                    submission_title_id,
                    "import: tracked download title disagrees with the durable Scryer submission; importing into the submission title"
                );
            }
            None
        }
        None => tracked_title_id.map(str::to_string),
    }
}

async fn actor_for_tracked_download_import(
    app: &AppUseCase,
    fallback_actor: &User,
    td: &TrackedDownload,
) -> User {
    let source_identity = ClientJobLocator::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    match app
        .services
        .workflow
        .download_submissions
        .get_submission_actor_snapshot(&source_identity)
        .await
    {
        Ok(Some(snapshot)) => actor_with_submission_snapshot(fallback_actor, &snapshot),
        Ok(None) => fallback_actor.clone(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id = td.client_id.as_str(),
                client_type = td.client_type.as_str(),
                download_client_item_id = td.client_item.download_client_item_id.as_str(),
                "failed to load download submission actor snapshot"
            );
            fallback_actor.clone()
        }
    }
}

fn actor_with_submission_snapshot(
    fallback_actor: &User,
    snapshot: &DownloadSubmissionActorSnapshot,
) -> User {
    let mut actor = fallback_actor.clone();
    if let Some(user_id) = snapshot
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        actor.id = user_id.to_string();
    }
    let display_name = snapshot.display_name.trim();
    if !display_name.is_empty() {
        actor.username = display_name.to_string();
    }
    actor
}

pub(super) async fn resolve_completed_download_for_import(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) -> Option<CompletedDownload> {
    if let Some(lookup) = completed_lookup {
        let completed = find_completed_download(app, td, Some(lookup)).await;
        if let Some(completed) = completed.as_ref() {
            td.completed_source = Some(completed.clone());
        }
        if completed.is_none() {
            tracing::debug!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                "import: completed download not found in recent snapshot, will retry"
            );
            td.state = TrackedDownloadState::ImportPending;
        }
        return completed;
    }

    let manual_completed = match app
        .resolve_manual_import_source(
            Some(td.client_id.as_str()),
            Some(td.client_type.as_str()),
            &td.client_item.download_client_item_id,
        )
        .await
    {
        Ok(crate::ManualImportSourceResolution::Eligible { completed }) => completed,
        Ok(crate::ManualImportSourceResolution::NotEligible { message }) => {
            tracing::warn!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                reason = %message,
                "import: source is no longer eligible; routing to failure handling"
            );
            td.state = TrackedDownloadState::FailedPending;
            td.status = TrackedDownloadStatus::Error;
            td.client_item.attention_reason = Some(message.clone());
            td.status_messages = vec![message];
            return None;
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                error = %error,
                "import: could not revalidate source before import"
            );
            td.state = TrackedDownloadState::ImportPending;
            return None;
        }
    };

    if let Some(completed) = manual_completed {
        let completed = prepare_completed_download_for_tracked_import(app, td, *completed).await;
        td.completed_source = Some(completed.clone());
        return Some(completed);
    }

    let completed = find_completed_download(app, td, None).await;
    if let Some(completed) = completed.as_ref() {
        td.completed_source = Some(completed.clone());
    }
    if completed.is_none() {
        tracing::debug!(
            id = %td.id,
            item_id = %td.client_item.download_client_item_id,
            "import: completed download not found in client history, will retry"
        );
        td.state = TrackedDownloadState::ImportPending;
    }
    completed
}

async fn prepare_completed_download_for_tracked_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: CompletedDownload,
) -> CompletedDownload {
    let mut completed = with_tracked_metadata(td, completed);
    remap_completed_download_for_client(app, &mut completed).await;
    completed
}

/// Verify whether a download's import is complete by checking cumulative
/// artifact history across all passes.
///
/// Returns true if all expected files are accounted for (imported or already_present).
async fn total_successful_artifacts(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: Option<&CompletedDownload>,
) -> AppResult<u64> {
    Ok(import_artifacts_for_completed_download(app, td, completed)
        .await?
        .into_iter()
        .filter(|artifact| matches!(artifact.result.as_str(), "imported" | "already_present"))
        .count() as u64)
}
