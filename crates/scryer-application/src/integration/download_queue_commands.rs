use crate::AppUseCase;
use crate::polling_worker::PollingWorker;

const DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS: u64 = 2;
const DOWNLOAD_DELETE_STALE_RECOVERY_SECONDS: i64 = 120;

async fn delete_command_download_id(
    app: &AppUseCase,
    command: &crate::DownloadQueueCommandRecord,
    source_identity: &crate::ClientJobLocator,
) -> Option<scryer_domain::download_identity::DownloadId> {
    if let Some(download_id) = command.canonical_download_id {
        return Some(download_id);
    }

    let command_created_at = chrono::DateTime::parse_from_rfc3339(&command.created_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    let binding = match app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(source_identity)
        .await
    {
        Ok(binding) => binding?,
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id = ?source_identity.client_id,
                client_type = %source_identity.client_type,
                native_item_id = %source_identity.item_id,
                "failed to recover canonical binding for legacy delete command"
            );
            return None;
        }
    };

    crate::download_identity::legacy_binding_predates_delete(&binding, command_created_at)
        .then_some(binding.download_id)
}

async fn delete_command_client_item(
    app: &AppUseCase,
    command: &crate::DownloadQueueCommandRecord,
    is_history: bool,
) -> crate::AppResult<()> {
    let download_client = &app.services.integrations.download_client;
    if let Some(client_id) = command.client_id.as_deref() {
        download_client
            .delete_queue_item_for_client_id(
                client_id,
                &command.download_client_item_id,
                is_history,
                false,
            )
            .await
    } else {
        download_client
            .delete_queue_item_for_client(
                &command.client_type,
                &command.download_client_item_id,
                is_history,
                false,
            )
            .await
    }
}

pub async fn start_background_download_delete_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("download_delete_poller", token);
    tracing::info!(
        interval_seconds = DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS,
        "download delete poller started"
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS,
    ));

    loop {
        if !worker.wait_for_tick(&mut interval).await {
            return;
        }

        match app
            .services
            .workflow
            .download_queue_commands
            .recover_stale_running_delete_commands(DOWNLOAD_DELETE_STALE_RECOVERY_SECONDS)
            .await
        {
            Ok(recovered) if recovered > 0 => {
                worker.warn_recovered("recover_stale_running_delete_commands", recovered);
            }
            Err(error) => {
                worker.warn_error("recover_stale_running_delete_commands", &error);
            }
            _ => {}
        }

        let pending = match app
            .services
            .workflow
            .download_queue_commands
            .list_pending_delete_commands()
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                worker.warn_error("list_pending_delete_commands", &error);
                continue;
            }
        };

        for command in pending {
            if let Err(error) = app
                .services
                .workflow
                .download_queue_commands
                .mark_delete_command_running(&command.id)
                .await
            {
                worker.warn_error("mark_delete_command_running", &error);
                continue;
            }

            execute_delete_command(&app, &worker, &command).await;
        }
    }
}

/// Execute one queued user delete: remove the client entry, finalize the
/// download as ignored, drop the local submission and end its binding.
///
/// Split out of the poller loop so the settle side effects a delete owes the
/// rest of the process can be exercised without driving the timer.
async fn execute_delete_command(
    app: &AppUseCase,
    worker: &PollingWorker,
    command: &crate::DownloadQueueCommandRecord,
) {
    let source_identity = crate::ClientJobLocator::new(
        command.client_id.as_deref(),
        &command.client_type,
        &command.download_client_item_id,
    );
    let canonical_download_id = delete_command_download_id(app, command, &source_identity).await;
    if let Some(download_id) = canonical_download_id.as_ref() {
        match delete_locator_was_rebound(app, &source_identity, download_id).await {
            Ok(false) => {}
            Ok(true) => {
                // A delayed command no longer owns this native client item.
                if let Err(error) = app
                    .services
                    .workflow
                    .download_queue_commands
                    .mark_delete_command_completed(&command.id)
                    .await
                {
                    worker.warn_error("complete_superseded_delete_command", &error);
                }
                return;
            }
            Err(error) => {
                worker.warn_error("check_delete_command_binding", &error);
                return;
            }
        }
    }
    // Read the owning title before the row goes: once the submission is
    // deleted there is nothing left to key the guard caches on, and they
    // hold this download as in flight for up to 30s.
    let deleted_title_id = match app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id_for_download(canonical_download_id.as_ref(), &source_identity)
        .await
    {
        Ok(submission) => submission
            .map(|submission| submission.title_id)
            .filter(|title_id| !title_id.trim().is_empty()),
        Err(error) => {
            worker.warn_error("find_delete_command_submission", &error);
            None
        }
    };

    // A password-retry recovery row is shown even when the job is absent from
    // the native snapshot, so the row's queue/history placement is unknown:
    // the job may still be active (restart before the first poll, accepted but
    // unconfirmed retry) or already in history. Remove it from both.
    let password_retry_fenced = match canonical_download_id.as_ref() {
        Some(download_id) => app
            .services
            .workflow
            .download_registry
            .bindings_have_password_retry(std::slice::from_ref(download_id))
            .await
            .unwrap_or_else(|error| {
                worker.warn_error("check_delete_command_password_retry", &error);
                false
            }),
        None => false,
    };
    let delete_kinds: &[bool] = if password_retry_fenced {
        &[false, true]
    } else {
        std::slice::from_ref(&command.is_history)
    };
    for &is_history in delete_kinds {
        if let Err(error) = delete_command_client_item(app, command, is_history).await {
            tracing::warn!(
                client_id = ?command.client_id,
                client_type = %command.client_type,
                download_client_item_id = %command.download_client_item_id,
                is_history,
                error = %error,
                "download client delete failed; completing local queue deletion"
            );
        }
    }

    let actor = command
        .requested_by_user_id
        .clone()
        .map(crate::domain_events::DomainEventActor::user_id)
        .unwrap_or_else(crate::domain_events::DomainEventActor::system);
    match crate::integration::workflow::finalize_scryer_download_ignored_for_download(
        app,
        actor,
        canonical_download_id.as_ref(),
        source_identity.clone(),
    )
    .await
    {
        Ok(crate::integration::workflow::FinalizeIgnoredOutcome::Finalized) => {}
        Ok(crate::integration::workflow::FinalizeIgnoredOutcome::NoSubmission) => {
            // No submission row, so no title to forget an accepted state
            // for, but the cached client snapshots still list the item this
            // delete is removing, and a snapshot may only answer for a queue
            // nothing has touched.
            app.runtime
                .acquisition
                .download_submission_guards
                .invalidate_client_snapshots();
        }
        Ok(crate::integration::workflow::FinalizeIgnoredOutcome::PreservedTerminal(state)) => {
            tracing::debug!(
                client_type = %command.client_type,
                download_client_item_id = %command.download_client_item_id,
                preserved_state = %state,
                "delete finalization preserved existing terminal download state"
            );
        }
        Err(error) => {
            worker.warn_error("finalize_scryer_download_ignored", &error);
        }
    }
    let submissions = &app.services.workflow.download_submissions;
    let local_delete = match canonical_download_id.as_ref() {
        Some(id) => submissions.delete_by_download_id(id).await,
        None => submissions.delete_by_client_item_id(&source_identity).await,
    };
    let local_delete_succeeded = match local_delete {
        Ok(()) => {
            // The delete ends this item's binding inside the store.
            app.runtime
                .acquisition
                .invalidate_download_registry_observations();
            // A durable retry recovery row may exist only in the Activity
            // read model, so removing it need not change the native snapshot.
            app.invalidate_download_queue_read_model().await;
            // The same settle the terminal transition performs: without
            // it a search for this scope inside the cache window still
            // sees the deleted submission and refuses the new grab as a
            // non-replaceable conflict.
            if let Some(title_id) = deleted_title_id.as_deref() {
                app.runtime
                    .acquisition
                    .download_submission_guards
                    .forget_settled_download(title_id);
            }
            true
        }
        Err(error) => {
            worker.warn_error("delete_download_submission", &error);
            false
        }
    };
    if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
        if let Some(id) = canonical_download_id {
            handle.forget_many(Vec::new(), vec![id]);
        } else if let Err(error) = handle
            .forget(crate::tracked_downloads::tracked_download_id(
                command.client_id.as_deref(),
                &command.client_type,
                &command.download_client_item_id,
            ))
            .await
        {
            worker.warn_error("forget_tracked_download", &error);
        }
    }
    // A new binding may have appeared while the remote request was in flight.
    let remove_snapshot = match canonical_download_id.as_ref() {
        Some(id) => matches!(
            delete_locator_was_rebound(app, &source_identity, id).await,
            Ok(false)
        ),
        None => true,
    };
    if remove_snapshot {
        app.runtime
            .acquisition
            .download_queue_snapshot
            .stage_remove(
                command.client_id.as_deref(),
                &command.client_type,
                &command.download_client_item_id,
            )
            .await;
    }
    if !local_delete_succeeded {
        return;
    }
    if let Err(error) = app
        .services
        .workflow
        .download_queue_commands
        .mark_delete_command_completed(&command.id)
        .await
    {
        worker.warn_error("mark_delete_command_completed", &error);
    }
    if let Some(canonical_download_id) = canonical_download_id.as_ref() {
        match app
            .services
            .workflow
            .download_registry
            .end_binding(canonical_download_id)
            .await
        {
            Ok(()) => app
                .runtime
                .acquisition
                .invalidate_download_registry_observations(),
            Err(error) => worker.warn_error("end_delete_command_download_binding", &error),
        }
    }
}

/// A native client locator can be reused after its original binding ends.
async fn delete_locator_was_rebound(
    app: &AppUseCase,
    locator: &crate::ClientJobLocator,
    download_id: &scryer_domain::download_identity::DownloadId,
) -> crate::AppResult<bool> {
    Ok(app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(locator)
        .await?
        .is_some_and(|binding| binding.download_id != *download_id))
}
