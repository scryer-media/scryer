use async_graphql::{Context, Error, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::*;
use crate::utils::parse_download_source_kind;

#[derive(Default)]
pub(crate) struct DownloadMutations;

#[Object]
impl DownloadMutations {
    async fn queue_existing_title_download(
        &self,
        ctx: &Context<'_>,
        input: QueueDownloadInput,
    ) -> GqlResult<String> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.queue_existing_title_download(
            &actor,
            &input.title_id,
            input.source_hint,
            parse_download_source_kind(input.source_kind),
            input.source_title,
        )
        .await
        .map_err(to_gql_error)
    }

    async fn queue_manual_import(
        &self,
        ctx: &Context<'_>,
        input: QueueManualImportInput,
    ) -> GqlResult<String> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.queue_manual_import(
            &actor,
            input.title_id,
            input.client_type,
            input.download_client_item_id,
        )
        .await
        .map_err(to_gql_error)
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
            decision: import_result.decision.as_str().to_string(),
            skip_reason: import_result.skip_reason.map(|r| r.as_str().to_string()),
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
            decision: result.decision.as_str().to_string(),
            skip_reason: result.skip_reason.map(|r| r.as_str().to_string()),
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
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.ignore_tracked_download(&actor, &input.client_type, &input.download_client_item_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn mark_tracked_download_failed(
        &self,
        ctx: &Context<'_>,
        input: MarkTrackedDownloadFailedInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.mark_tracked_download_failed(&actor, &input.client_type, &input.download_client_item_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn retry_tracked_download_import(
        &self,
        ctx: &Context<'_>,
        input: RetryTrackedDownloadImportInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.retry_tracked_download_import(
            &actor,
            &input.client_type,
            &input.download_client_item_id,
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
    }

    async fn assign_tracked_download_title(
        &self,
        ctx: &Context<'_>,
        input: AssignTrackedDownloadTitleInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.assign_tracked_download_title(
            &actor,
            &input.client_type,
            &input.download_client_item_id,
            &input.title_id,
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
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
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.pause_download_queue_item(&actor, &input.download_client_item_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn resume_download(
        &self,
        ctx: &Context<'_>,
        input: ResumeDownloadInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.resume_download_queue_item(&actor, &input.download_client_item_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn delete_download(
        &self,
        ctx: &Context<'_>,
        input: DeleteDownloadInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_download_queue_item(&actor, &input.download_client_item_id, input.is_history)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }
}
