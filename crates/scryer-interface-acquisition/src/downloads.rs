use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{
    AppError, AppUseCase, QueueDownloadOutcome, SubmissionConflictPolicy, SubmissionScopeConflict,
};
use scryer_domain::User;

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::{mappers, types::*};

#[derive(Default)]
pub struct DownloadMutations;

async fn queue_item_payload_for_action(
    app: &AppUseCase,
    actor: &User,
    client_id: Option<&str>,
    client_type: Option<&str>,
    download_client_item_id: &str,
) -> GqlResult<Option<DownloadQueueItemPayload>> {
    let item = app
        .find_download_queue_item(actor, client_id, client_type, download_client_item_id)
        .await
        .map_err(to_gql_error)?;
    Ok(item.map(mappers::from_download_queue_item))
}

struct DownloadQueueActionParts {
    kind: DownloadQueueActionKindValue,
    download_client_item_id: String,
    client_id: Option<String>,
    client_type: Option<String>,
    import_id: Option<String>,
    command_id: Option<String>,
    removed: bool,
    queue_item: Option<DownloadQueueItemPayload>,
}

fn download_queue_action_payload(parts: DownloadQueueActionParts) -> DownloadQueueActionPayload {
    DownloadQueueActionPayload {
        kind: parts.kind,
        download_client_item_id: parts.download_client_item_id,
        client_id: parts.client_id.map(Into::into),
        client_type: parts.client_type,
        import_id: parts.import_id.map(Into::into),
        command_id: parts.command_id.map(Into::into),
        removed: parts.removed,
        queue_item: parts.queue_item,
    }
}

pub(crate) fn queue_download_conflict_payload(
    conflict: SubmissionScopeConflict,
) -> QueueDownloadConflictPayload {
    QueueDownloadConflictPayload {
        title_id: conflict.title_id.into(),
        title_name: conflict.title_name,
        download_client_id: conflict.download_client_id.map(Into::into),
        download_client_type: conflict.download_client_type,
        download_client_item_id: conflict.download_client_item_id,
        source_title: conflict.source_title,
        source_kind: conflict
            .source_kind
            .map(DownloadSourceKindValue::from_application),
        scope: mappers::from_submission_scope(conflict.scope),
        state: conflict.state.map(DownloadQueueStateValue::from_domain),
        replaceable: conflict.replaceable,
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
        let QueueDownloadInput {
            title_id,
            candidate_token,
            scope,
            replace_in_progress,
            purpose,
        } = input;
        let title_id = title_id.to_string();
        let outcome = app
            .queue_existing_title_download_from_candidate_token_with_purpose(
                &actor,
                &title_id,
                &candidate_token,
                scope.into_application(),
                SubmissionConflictPolicy::from_replace_flag(replace_in_progress.unwrap_or(false)),
                purpose
                    .map(|value| value.into_application())
                    .unwrap_or_default(),
            )
            .await
            .map_err(to_gql_error)?;
        let title = app
            .get_title_for_download_actions(&actor, &title_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| to_gql_error(AppError::NotFound(format!("title {title_id}"))))?;

        Ok(match outcome {
            QueueDownloadOutcome::Queued(queued) => QueueDownloadPayload {
                status: QueueDownloadResultStatusValue::Queued,
                job_id: Some(queued.job_id.into()),
                title_id: title.id.into(),
                title_name: title.name,
                source_title: queued.queued_release.source_title,
                source_kind: queued
                    .queued_release
                    .source_kind
                    .map(DownloadSourceKindValue::from_application),
                conflict: None,
            },
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadPayload {
                status: QueueDownloadResultStatusValue::Conflict,
                job_id: None,
                title_id: title.id.into(),
                title_name: title.name,
                source_title: None,
                source_kind: None,
                conflict: Some(queue_download_conflict_payload(conflict)),
            },
        })
    }

    async fn queue_best_release(
        &self,
        ctx: &Context<'_>,
        input: QueueBestReleaseInput,
    ) -> GqlResult<QueueDownloadPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let title = app
            .get_title_for_download_actions(&actor, &title_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| to_gql_error(AppError::NotFound(format!("title {title_id}"))))?;
        let outcome = app
            .queue_best_release(
                &actor,
                &title_id,
                input.scope.into_application(),
                SubmissionConflictPolicy::from_replace_flag(
                    input.replace_in_progress.unwrap_or(false),
                ),
            )
            .await
            .map_err(to_gql_error)?;

        Ok(match outcome {
            QueueDownloadOutcome::Queued(queued) => QueueDownloadPayload {
                status: QueueDownloadResultStatusValue::Queued,
                job_id: Some(queued.job_id.into()),
                title_id: title.id.into(),
                title_name: title.name,
                source_title: queued.queued_release.source_title,
                source_kind: queued
                    .queued_release
                    .source_kind
                    .map(DownloadSourceKindValue::from_application),
                conflict: None,
            },
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadPayload {
                status: QueueDownloadResultStatusValue::Conflict,
                job_id: None,
                title_id: title.id.into(),
                title_name: title.name,
                source_title: None,
                source_kind: None,
                conflict: Some(queue_download_conflict_payload(conflict)),
            },
        })
    }

    async fn queue_manual_import(
        &self,
        ctx: &Context<'_>,
        input: QueueManualImportInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection_id = input.selection_id.to_string();
        let import_id = app
            .queue_manual_import_selection(
                &actor,
                selection_id.clone(),
                input
                    .files
                    .into_iter()
                    .map(|file| scryer_application::ManualImportCandidateMapping {
                        candidate_id: file.candidate_id.to_string(),
                        episode_id: file.episode_id.map(String::from),
                        series_movie_link_id: file.series_movie_link_id.map(String::from),
                    })
                    .collect(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::QueuedManualImport,
            download_client_item_id: selection_id,
            client_id: None,
            client_type: None,
            import_id: Some(import_id),
            command_id: None,
            removed: false,
            queue_item: None,
        }))
    }

    async fn begin_manual_import_selection(
        &self,
        ctx: &Context<'_>,
        input: BeginManualImportSelectionInput,
    ) -> GqlResult<ManualImportSelectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = scryer_application::begin_manual_import_selection(
            &app,
            &actor,
            input.client_id.as_ref().map(|id| id.as_str()),
            &input.client_type,
            &input.download_client_item_id,
            input.title_id.as_str(),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(ManualImportSelectionPayload {
            selection_id: selection.selection_id.into(),
            files: selection
                .files
                .into_iter()
                .map(|file| ManualImportFilePreviewPayload {
                    candidate_id: file.candidate_id.into(),
                    file_name: file.file_name,
                    size_bytes: Long::from(file.size_bytes),
                    quality: file.quality,
                    parsed_season: file.parsed_season.map(|value| value as i32),
                    parsed_episodes: file
                        .parsed_episodes
                        .into_iter()
                        .map(|value| value as i32)
                        .collect(),
                    suggested_episode_id: file.suggested_episode_id.map(Into::into),
                    suggested_episode_label: file.suggested_episode_label,
                })
                .collect(),
            available_episodes: selection
                .available_episodes
                .into_iter()
                .map(|episode| mappers::from_episode(&app, episode))
                .collect(),
            available_series_movies: selection
                .available_series_movies
                .into_iter()
                .map(|target| ManualImportSeriesMovieTargetPayload {
                    series_movie_link_id: target.series_movie_link_id,
                    movie_title: target.movie_title,
                    year: target.year,
                    runtime_minutes: target.runtime_minutes,
                })
                .collect(),
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
            input.import_id.as_ref(),
            input.password.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;

        Ok(ImportResultPayload {
            import_id: result.import_id.into(),
            decision: ImportDecisionValue::from_domain(result.decision),
            skip_reason: result.skip_reason.map(ImportSkipReasonValue::from_domain),
            title_id: result.title_id.map(Into::into),
            source_path: result.source_path,
            dest_path: result.dest_path,
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
        let client_id = input.client_id.clone().map(String::from);
        let download_client_item_id = input.download_client_item_id.clone();
        app.ignore_tracked_download(
            &actor,
            client_id.as_deref(),
            &input.client_type,
            &input.download_client_item_id,
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::IgnoredTrackedDownload,
            download_client_item_id,
            client_id,
            client_type: Some(client_type),
            import_id: None,
            command_id: None,
            removed: false,
            queue_item,
        }))
    }

    async fn mark_tracked_download_failed(
        &self,
        ctx: &Context<'_>,
        input: MarkTrackedDownloadFailedInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let client_id = input.client_id.clone().map(String::from);
        let download_client_item_id = input.download_client_item_id.clone();
        app.mark_tracked_download_failed(
            &actor,
            client_id.as_deref(),
            &input.client_type,
            &input.download_client_item_id,
            input.skip_reacquire.unwrap_or(false),
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::MarkedTrackedDownloadFailed,
            download_client_item_id,
            client_id,
            client_type: Some(client_type),
            import_id: None,
            command_id: None,
            removed: false,
            queue_item,
        }))
    }

    async fn assign_tracked_download_title(
        &self,
        ctx: &Context<'_>,
        input: AssignTrackedDownloadTitleInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let AssignTrackedDownloadTitleInput {
            client_id,
            client_type,
            download_client_item_id,
            title_id,
            scope,
        } = input;
        let client_id = client_id.map(String::from);
        let title_id = title_id.to_string();
        app.assign_tracked_download_title(
            &actor,
            client_id.as_deref(),
            &client_type,
            &download_client_item_id,
            &title_id,
            scope.into_application(),
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::AssignedTrackedDownloadTitle,
            download_client_item_id,
            client_id,
            client_type: Some(client_type),
            import_id: None,
            command_id: None,
            removed: false,
            queue_item,
        }))
    }

    async fn pause_download(
        &self,
        ctx: &Context<'_>,
        input: PauseDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        let client_id = input.client_id.clone().map(String::from);
        app.pause_download_queue_item(&actor, client_id.as_deref(), &input.download_client_item_id)
            .await
            .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            None,
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::Paused,
            download_client_item_id,
            client_id,
            client_type: queue_item.as_ref().map(|item| item.client_type.clone()),
            import_id: None,
            command_id: None,
            removed: false,
            queue_item,
        }))
    }

    async fn resume_download(
        &self,
        ctx: &Context<'_>,
        input: ResumeDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let download_client_item_id = input.download_client_item_id.clone();
        let client_id = input.client_id.clone().map(String::from);
        app.resume_download_queue_item(
            &actor,
            client_id.as_deref(),
            &input.download_client_item_id,
        )
        .await
        .map_err(to_gql_error)?;
        let queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            None,
            &download_client_item_id,
        )
        .await?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::Resumed,
            download_client_item_id,
            client_id,
            client_type: queue_item.as_ref().map(|item| item.client_type.clone()),
            import_id: None,
            command_id: None,
            removed: false,
            queue_item,
        }))
    }

    async fn delete_download(
        &self,
        ctx: &Context<'_>,
        input: DeleteDownloadInput,
    ) -> GqlResult<DownloadQueueActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_type = input.client_type.clone();
        let client_id = input.client_id.clone().map(String::from);
        let download_client_item_id = input.download_client_item_id.clone();
        let existing_queue_item = queue_item_payload_for_action(
            &app,
            &actor,
            client_id.as_deref(),
            Some(&client_type),
            &download_client_item_id,
        )
        .await?;
        let command = app
            .delete_download_queue_item(
                &actor,
                client_id.as_deref(),
                &input.client_type,
                &input.download_client_item_id,
                input.is_history,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(download_queue_action_payload(DownloadQueueActionParts {
            kind: DownloadQueueActionKindValue::DeleteQueued,
            download_client_item_id,
            client_id,
            client_type: Some(client_type),
            import_id: None,
            command_id: Some(command.id),
            removed: false,
            queue_item: existing_queue_item,
        }))
    }
}
