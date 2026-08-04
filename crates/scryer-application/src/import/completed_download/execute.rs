use super::lookup::{
    find_completed_download, remap_completed_download_for_client, with_tracked_metadata,
};
use super::result_state::apply_import_result_with_completed;
use super::*;

pub(crate) fn mark_importing(td: &mut TrackedDownload) {
    td.state = TrackedDownloadState::Importing;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages = vec![IMPORT_RUNNING_MESSAGE.to_string()];
}

pub async fn import(app: &AppUseCase, actor: &User, td: &mut TrackedDownload) -> bool {
    import_inner(app, actor, td, None).await
}

pub(crate) async fn import_with_lookup(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
    completed_lookup: &CompletedDownloadLookup,
) -> bool {
    import_inner(app, actor, td, Some(completed_lookup)).await
}

async fn import_inner(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
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

    mark_importing(td);
    crate::tracked_downloads::publish_runtime_tracked_download_snapshot(app, td).await;

    let completed = match resolve_completed_download_origin_for_import(
        app,
        &completed,
        Some(&td.client_item),
    )
    .await
    {
        Ok(ResolvedCompletedDownloadOriginForImport::Ready(completed)) => completed,
        Ok(ResolvedCompletedDownloadOriginForImport::NoScryerOrigin) => completed,
        Ok(ResolvedCompletedDownloadOriginForImport::Conflict { reason, detail }) => {
            block_completed_download_origin_conflict_for_manual_review(
                app, &completed, reason, &detail,
            )
            .await;
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![format!(
                "Download origin scope conflicts with the matched submission ({reason}). Manual confirmation required before import."
            )];
            return false;
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                error = %error,
                "import: completed download origin resolution failed, will retry"
            );
            td.state = TrackedDownloadState::ImportPending;
            return false;
        }
    };

    tracing::info!(
        id = %td.id,
        dest_dir = %completed.dest_dir,
        title_id = ?td.title_id,
        "import: starting import from completed download"
    );

    let success_before = total_successful_artifacts(app, td).await;
    td.import_attempted = true;

    let import_actor = actor_for_tracked_download_import(app, actor, td).await;
    match import_completed_download(app, &import_actor, &completed).await {
        Ok(result) => {
            let success_after = total_successful_artifacts(app, td).await;
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
            )
            .await
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                error = %error,
                dest_dir = %completed.dest_dir,
                "import: pipeline returned error"
            );
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![format!("Import failed: {error}")];
            false
        }
    }
}

async fn actor_for_tracked_download_import(
    app: &AppUseCase,
    fallback_actor: &User,
    td: &TrackedDownload,
) -> User {
    let source_identity = DownloadSourceIdentity::new(
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
        return Some(prepare_completed_download_for_tracked_import(app, td, completed).await);
    }

    let completed = find_completed_download(app, td, None).await;
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
async fn total_successful_artifacts(app: &AppUseCase, td: &TrackedDownload) -> u64 {
    let source_identity = DownloadSourceIdentity::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    let imported = app
        .services
        .workflow
        .import_artifacts
        .count_by_result_for_source_identity(&source_identity, "imported")
        .await
        .unwrap_or(0);
    let already_present = app
        .services
        .workflow
        .import_artifacts
        .count_by_result_for_source_identity(&source_identity, "already_present")
        .await
        .unwrap_or(0);
    imported + already_present
}
