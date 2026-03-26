use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_application::AppUseCase;
use scryer_domain::User;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::*;
use crate::utils::parse_download_source_kind;

#[derive(Default)]
pub(crate) struct DownloadMutations;

async fn queue_item_payload_for_action(
    app: &AppUseCase,
    actor: &User,
    client_type: Option<&str>,
    download_client_item_id: &str,
) -> GqlResult<Option<DownloadQueueItemPayload>> {
    let item = app
        .find_download_queue_item(actor, client_type, download_client_item_id)
        .await
        .map_err(to_gql_error)?;
    Ok(item.map(crate::mappers::from_download_queue_item))
}

fn download_queue_action_payload(
    kind: DownloadQueueActionKindValue,
    download_client_item_id: impl Into<String>,
    client_type: Option<String>,
    import_id: Option<String>,
    removed: bool,
    queue_item: Option<DownloadQueueItemPayload>,
) -> DownloadQueueActionPayload {
    DownloadQueueActionPayload {
        kind,
        download_client_item_id: download_client_item_id.into(),
        client_type,
        import_id,
        removed,
        queue_item,
    }
}

#[Object]
impl DownloadMutations {
    async fn queue_existing_title_download(
        &self,
        ctx: &Context<'_>,
        input: QueueDownloadInput,
    ) -> GqlResult<QueueDownloadPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let source_kind = parse_download_source_kind(input.release.source_kind);
        let job_id = app
            .queue_existing_title_download(
                &actor,
                &input.title_id,
                input.release.source_hint,
                source_kind,
                input.release.source_title.clone(),
            )
            .await
            .map_err(to_gql_error)?;
        let title = app
            .services
            .titles
            .get_by_id(&input.title_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| Error::new(format!("title not found: {}", input.title_id)))?;

        Ok(QueueDownloadPayload {
            job_id,
            title_id: title.id,
            title_name: title.name,
            source_title: input.release.source_title,
            source_kind: source_kind.map(DownloadSourceKindValue::from_application),
        })
    }

    async fn queue_manual_import(
        &self,
        ctx: &Context<'_>,
        input: QueueManualImportInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        let client_type = input.client_type.clone();
        let import_id = app
            .queue_manual_import(
                &actor,
                input.title_id,
                client_type.clone(),
                download_client_item_id.clone(),
            )
            .await
            .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_type.as_deref(),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::QueuedManualImport,
            download_client_item_id,
            client_type,
            Some(import_id),
            false,
            queue_item,
        ))
    }

    async fn trigger_import(
        &self,
        ctx: &Context<'_>,
        input: TriggerImportInput,
    ) -> GqlResult<ImportResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let completed_downloads = app
            .services
            .download_client
            .list_completed_downloads()
            .await
            .map_err(to_gql_error)?;

        let completed = completed_downloads
            .into_iter()
            .find(|cd| cd.download_client_item_id == input.download_client_item_id)
            .ok_or_else(|| {
                Error::new(format!(
                    "completed download not found: {}",
                    input.download_client_item_id
                ))
            })?;

        let import_result = app
            .trigger_manual_import(&actor, &completed, input.title_id.as_deref())
            .await
            .map_err(to_gql_error)?;

        Ok(ImportResultPayload {
            import_id: import_result.import_id,
            decision: ImportDecisionValue::from_domain(import_result.decision),
            skip_reason: import_result
                .skip_reason
                .map(ImportSkipReasonValue::from_domain),
            title_id: import_result.title_id,
            source_path: import_result.source_path,
            dest_path: import_result.dest_path,
            file_size_bytes: import_result.file_size_bytes.map(|v| v.to_string()),
            link_type: import_result.link_type.map(|s| s.as_str().to_string()),
            error_message: import_result.error_message,
        })
    }

    /// Retry a previously failed import, optionally with an archive password.
    async fn retry_import(
        &self,
        ctx: &Context<'_>,
        input: RetryImportInput,
    ) -> GqlResult<ImportResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let result = scryer_application::retry_failed_import(
            &app,
            &actor,
            &input.import_id,
            input.password.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(ImportResultPayload {
            import_id: result.import_id,
            decision: ImportDecisionValue::from_domain(result.decision),
            skip_reason: result.skip_reason.map(ImportSkipReasonValue::from_domain),
            title_id: result.title_id,
            source_path: result.source_path,
            dest_path: result.dest_path,
            file_size_bytes: result.file_size_bytes.map(|v| v.to_string()),
            link_type: result.link_type.map(|s| s.as_str().to_string()),
            error_message: result.error_message,
        })
    }

    async fn ignore_tracked_download(
        &self,
        ctx: &Context<'_>,
        input: IgnoreTrackedDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let download_client_item_id = input.download_client_item_id.clone();
        app.ignore_tracked_download(&actor, &input.client_type, &input.download_client_item_id)
            .await
            .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::IgnoredTrackedDownload,
            download_client_item_id,
            Some(client_type),
            None,
            false,
            queue_item,
        ))
    }

    async fn mark_tracked_download_failed(
        &self,
        ctx: &Context<'_>,
        input: MarkTrackedDownloadFailedInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let download_client_item_id = input.download_client_item_id.clone();
        app.mark_tracked_download_failed(
            &actor,
            &input.client_type,
            &input.download_client_item_id,
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::MarkedTrackedDownloadFailed,
            download_client_item_id,
            Some(client_type),
            None,
            false,
            queue_item,
        ))
    }

    async fn retry_tracked_download_import(
        &self,
        ctx: &Context<'_>,
        input: RetryTrackedDownloadImportInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let download_client_item_id = input.download_client_item_id.clone();
        app.retry_tracked_download_import(
            &actor,
            &input.client_type,
            &input.download_client_item_id,
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::RetriedTrackedDownloadImport,
            download_client_item_id,
            Some(client_type),
            None,
            false,
            queue_item,
        ))
    }

    async fn assign_tracked_download_title(
        &self,
        ctx: &Context<'_>,
        input: AssignTrackedDownloadTitleInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let download_client_item_id = input.download_client_item_id.clone();
        app.assign_tracked_download_title(
            &actor,
            &input.client_type,
            &input.download_client_item_id,
            &input.title_id,
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::AssignedTrackedDownloadTitle,
            download_client_item_id,
            Some(client_type),
            None,
            false,
            queue_item,
        ))
    }

    async fn execute_manual_import(
        &self,
        ctx: &Context<'_>,
        input: ExecuteManualImportInput,
    ) -> GqlResult<Vec<ManualImportFileResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let mappings = input
            .files
            .into_iter()
            .map(|f| scryer_application::ManualImportFileMapping {
                file_path: f.file_path,
                episode_id: f.episode_id,
                quality: f.quality,
            })
            .collect();

        let results =
            scryer_application::execute_manual_import(&app, &actor, &input.title_id, mappings)
                .await
                .map_err(to_gql_error)?;

        Ok(results
            .into_iter()
            .map(|r| ManualImportFileResultPayload {
                file_path: r.file_path,
                episode_id: r.episode_id,
                success: r.success,
                dest_path: r.dest_path,
                error_message: r.error_message,
            })
            .collect())
    }

    async fn pause_download(
        &self,
        ctx: &Context<'_>,
        input: PauseDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        app.pause_download_queue_item(&actor, &input.download_client_item_id)
            .await
            .map_err(to_gql_error)?;
        let queue_item =
            queue_item_payload_for_action(&app, &actor, None, &download_client_item_id).await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::Paused,
            download_client_item_id,
            queue_item.as_ref().map(|item| item.client_type.clone()),
            None,
            false,
            queue_item,
        ))
    }

    async fn resume_download(
        &self,
        ctx: &Context<'_>,
        input: ResumeDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        app.resume_download_queue_item(&actor, &input.download_client_item_id)
            .await
            .map_err(to_gql_error)?;
        let queue_item =
            queue_item_payload_for_action(&app, &actor, None, &download_client_item_id).await?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::Resumed,
            download_client_item_id,
            queue_item.as_ref().map(|item| item.client_type.clone()),
            None,
            false,
            queue_item,
        ))
    }

    async fn delete_download(
        &self,
        ctx: &Context<'_>,
        input: DeleteDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        let existing_queue_item =
            queue_item_payload_for_action(&app, &actor, None, &download_client_item_id).await?;
        app.delete_download_queue_item(&actor, &input.download_client_item_id, input.is_history)
            .await
            .map_err(to_gql_error)?;

        Ok(download_queue_action_payload(
            DownloadQueueActionKindValue::Deleted,
            download_client_item_id,
            existing_queue_item
                .as_ref()
                .map(|item| item.client_type.clone()),
            None,
            true,
            None,
        ))
    }
}
