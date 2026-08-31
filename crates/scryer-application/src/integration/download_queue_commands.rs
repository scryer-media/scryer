use crate::AppUseCase;
use crate::polling_worker::PollingWorker;

const DOWNLOAD_DELETE_POLLER_INTERVAL_SECONDS: u64 = 2;
const DOWNLOAD_DELETE_STALE_RECOVERY_SECONDS: i64 = 120;

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

            let result = if let Some(client_id) = command.client_id.as_deref() {
                app.services
                    .integrations
                    .download_client
                    .delete_queue_item_for_client_id(
                        client_id,
                        &command.download_client_item_id,
                        command.is_history,
                        false,
                    )
                    .await
            } else {
                app.services
                    .integrations
                    .download_client
                    .delete_queue_item_for_client(
                        &command.client_type,
                        &command.download_client_item_id,
                        command.is_history,
                        false,
                    )
                    .await
            };

            if let Err(error) = result {
                tracing::warn!(
                    client_id = ?command.client_id,
                    client_type = %command.client_type,
                    download_client_item_id = %command.download_client_item_id,
                    error = %error,
                    "download client delete failed; completing local queue deletion"
                );
            }

            let actor = command
                .requested_by_user_id
                .clone()
                .map(crate::domain_events::DomainEventActor::user_id)
                .unwrap_or_else(crate::domain_events::DomainEventActor::system);
            let source_identity = crate::ClientJobLocator::new(
                command.client_id.as_deref(),
                &command.client_type,
                &command.download_client_item_id,
            );
            match crate::integration::workflow::finalize_scryer_download_ignored_for_download(
                &app,
                actor,
                command.canonical_download_id.as_ref(),
                source_identity.clone(),
            )
            .await
            {
                Ok(crate::integration::workflow::FinalizeIgnoredOutcome::Finalized)
                | Ok(crate::integration::workflow::FinalizeIgnoredOutcome::NoSubmission) => {}
                Ok(crate::integration::workflow::FinalizeIgnoredOutcome::PreservedTerminal(
                    state,
                )) => {
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
            let local_delete_succeeded = match app
                .services
                .workflow
                .download_submissions
                .delete_by_client_item_id(&source_identity)
                .await
            {
                Ok(()) => true,
                Err(error) => {
                    worker.warn_error("delete_download_submission", &error);
                    false
                }
            };
            if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref()
                && let Err(error) = handle
                    .forget(crate::tracked_downloads::tracked_download_id(
                        command.client_id.as_deref(),
                        &command.client_type,
                        &command.download_client_item_id,
                    ))
                    .await
            {
                worker.warn_error("forget_tracked_download", &error);
            }
            app.runtime
                .acquisition
                .download_queue_snapshot
                .stage_remove(
                    command.client_id.as_deref(),
                    &command.client_type,
                    &command.download_client_item_id,
                )
                .await;
            if !local_delete_succeeded {
                continue;
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
            if let Some(canonical_download_id) = command.canonical_download_id.as_ref()
                && let Err(error) = app
                    .services
                    .workflow
                    .download_registry
                    .end_binding(canonical_download_id)
                    .await
            {
                worker.warn_error("end_delete_command_download_binding", &error);
            }
        }
    }
}
