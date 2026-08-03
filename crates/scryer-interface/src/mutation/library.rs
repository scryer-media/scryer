use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::{AppError, DeleteExecutionConfirmation};
use scryer_domain::AppPermission;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_cancel_library_scan_result, from_ignore_pending_import_result, from_job_run, from_library,
    from_library_scan_session, from_library_scan_summary, from_media_rename_apply,
    from_resolve_pending_import_result,
};
use crate::types::*;
use crate::utils::map_add_input;

static RENAME_IDEMPOTENCY_KEYS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn claim_rename_idempotency_key(scope: &str, key: Option<String>) -> GqlResult<Option<String>> {
    let Some(raw_key) = key else {
        return Ok(None);
    };

    let normalized = raw_key.trim();
    if normalized.is_empty() {
        return Err(to_gql_error(AppError::Validation(
            "idempotencyKey cannot be empty".to_string(),
        )));
    }

    let composite = format!("{scope}:{normalized}");
    let store = &*RENAME_IDEMPOTENCY_KEYS;
    let mut guard = store.lock().map_err(|_| {
        to_gql_error(AppError::Repository(
            "failed to lock rename idempotency key store".to_string(),
        ))
    })?;
    if !guard.insert(composite.clone()) {
        return Err(to_gql_error(AppError::Validation(
            "duplicate idempotencyKey".to_string(),
        )));
    }

    Ok(Some(composite))
}

fn library_settings_draft(
    input: LibrarySettingsInput,
) -> GqlResult<scryer_application::LibrarySettingsOverrideDraft> {
    let import_mode = input.import_mode.map(scryer_domain::ImportMode::from);

    Ok(scryer_application::LibrarySettingsOverrideDraft {
        required_audio_languages: input.required_audio_languages,
        quality_profile_id: input.quality_profile_id.map(String::from),
        request_quality_profile_ids: input
            .request_quality_profile_ids
            .map(|ids| ids.into_iter().map(String::from).collect()),
        scoring_persona: input
            .scoring_persona
            .map(ScoringPersonaValue::into_application),
        filler_policy: input
            .filler_policy
            .map(|policy| policy.as_app_str().to_string()),
        recap_policy: input
            .recap_policy
            .map(|policy| policy.as_app_str().to_string()),
        monitor_specials: input.monitor_specials,
        inter_season_movies: input.inter_season_movies,
        monitor_filler_movies: input.monitor_filler_movies,
        nfo_write_on_import: input.nfo_write_on_import,
        plexmatch_write_on_import: input.plexmatch_write_on_import,
        import_mode,
        set_permissions_linux: input.set_permissions_linux,
        file_chmod: input.file_chmod,
        folder_chmod: input.folder_chmod,
        chown_group: input.chown_group,
        indexer_routing: input.indexer_routing.map(|entries| {
            entries
                .into_iter()
                .map(|entry| scryer_application::IndexerRoutingSettingsEntry {
                    indexer_id: entry.indexer_id.to_string(),
                    enabled: entry.enabled,
                    categories: entry.categories,
                    priority: entry.priority,
                })
                .collect()
        }),
        download_client_routing: input.download_client_routing.map(|entries| {
            entries
                .into_iter()
                .map(
                    |entry| scryer_application::DownloadClientRoutingSettingsEntry {
                        client_id: entry.client_id.to_string(),
                        enabled: entry.enabled,
                        category: entry.category,
                        recent_queue_priority: entry.recent_queue_priority,
                        older_queue_priority: entry.older_queue_priority,
                        remove_completed: entry.remove_completed,
                        remove_failed: entry.remove_failed,
                    },
                )
                .collect()
        }),
    })
}

#[derive(Default)]
pub(crate) struct LibraryMutations;

#[Object]
impl LibraryMutations {
    async fn create_library(
        &self,
        ctx: &Context<'_>,
        input: CreateLibraryInput,
    ) -> GqlResult<LibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let roots = input
            .roots
            .into_iter()
            .map(|root| scryer_application::LibraryRootDraft {
                path: root.path,
                is_default: root.is_default,
            })
            .collect();
        let library = app
            .create_library(
                &actor,
                input.facet.into_domain(),
                input.name,
                roots,
                input.settings.map(library_settings_draft).transpose()?,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_library(library))
    }

    async fn update_library(
        &self,
        ctx: &Context<'_>,
        input: UpdateLibraryInput,
    ) -> GqlResult<LibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let roots = input.roots.map(|roots| {
            roots
                .into_iter()
                .map(|root| scryer_application::LibraryRootDraft {
                    path: root.path,
                    is_default: root.is_default,
                })
                .collect()
        });
        let library = app
            .update_library(
                &actor,
                input.library_id.as_ref(),
                input.name,
                roots,
                input.settings.map(library_settings_draft).transpose()?,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_library(library))
    }

    async fn delete_library(&self, ctx: &Context<'_>, id: ID) -> GqlResult<DeleteLibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.delete_library(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteLibraryPayload { id: ID::from(id) })
    }

    async fn scan_library(
        &self,
        ctx: &Context<'_>,
        input: ScanLibraryInput,
    ) -> GqlResult<LibraryScanProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_id = input.library_id.to_string();
        let scan_hints = match input
            .import_warmup_session_id
            .as_ref()
            .map(|session_id| session_id.as_str())
        {
            Some(session_id) if !session_id.trim().is_empty() => {
                app.external_import_monitor_warmup_scan_hints(&actor, session_id)
                    .await
            }
            _ => None,
        };
        let session = app
            .trigger_library_scan_by_id_with_hints(&actor, &library_id, scan_hints)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_session(session))
    }

    async fn scan_title_library(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
    ) -> GqlResult<LibraryScanSummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = title_id.to_string();
        let summary = app
            .scan_title_library(&actor, &title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_summary(summary))
    }

    async fn cancel_library_scan(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
    ) -> GqlResult<CancelLibraryScanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let session_id = session_id.to_string();
        let result = app
            .cancel_library_scan(&actor, &session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_cancel_library_scan_result(result))
    }

    async fn resolve_pending_import(
        &self,
        ctx: &Context<'_>,
        input: ResolvePendingImportInput,
    ) -> GqlResult<ResolvePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = input.pending_import_id.to_string();
        let mut title_input = input.title;
        title_input.library_id = None;
        title_input.monitored = false;
        title_input.tags.clear();
        title_input.options = None;
        title_input.source_hint = None;
        title_input.source_kind = None;
        title_input.source_title = None;
        title_input.min_availability = None;
        let request = map_add_input(title_input, None)?;
        let result = app
            .resolve_pending_import(&actor, &pending_import_id, request)
            .await
            .map_err(to_gql_error)?;
        Ok(from_resolve_pending_import_result(&app, result))
    }

    async fn bind_pending_import(
        &self,
        ctx: &Context<'_>,
        input: BindPendingImportInput,
    ) -> GqlResult<ResolvePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = input.pending_import_id.to_string();
        let collection_id = input.collection_id.map(String::from);
        let episode_ids = input
            .episode_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let result = app
            .bind_title_bound_pending_import(
                &actor,
                &pending_import_id,
                collection_id.as_deref(),
                &episode_ids,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_resolve_pending_import_result(&app, result))
    }

    async fn ignore_pending_import(
        &self,
        ctx: &Context<'_>,
        pending_import_id: ID,
    ) -> GqlResult<IgnorePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = pending_import_id.to_string();
        let result = app
            .ignore_pending_import(&actor, &pending_import_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_ignore_pending_import_result(result))
    }

    async fn apply_media_rename(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let MediaRenameApplyInput {
            facet,
            title_id,
            fingerprint,
            idempotency_key,
        } = input;
        let title_id = title_id.to_string();
        let facet = facet.into_domain();
        let facet_name = facet.as_str();
        let idempotency_key = claim_rename_idempotency_key("apply_media_rename", idempotency_key)?;

        let result = app
            .apply_rename_for_title(&actor, &title_id, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = app
            .record_rename_apply_audit(
                &actor,
                "rename_apply_title",
                facet_name,
                Some(&title_id),
                idempotency_key.as_deref(),
                &result,
            )
            .await;

        Ok(from_media_rename_apply(result))
    }

    async fn delete_media_file(
        &self,
        ctx: &Context<'_>,
        input: DeleteMediaFileInput,
    ) -> GqlResult<DeleteMediaFilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let file_id = input.file_id.to_string();
        let accepted = app
            .start_delete_media_file_job(
                &actor,
                &file_id,
                input.delete_from_disk.unwrap_or(false),
                input
                    .preview_fingerprint
                    .map(|preview_fingerprint| DeleteExecutionConfirmation {
                        preview_fingerprint,
                        typed_confirmation: input.typed_confirmation,
                    }),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteMediaFilePayload {
            id: ID::from(file_id),
            job_run: from_job_run(accepted.job_run),
        })
    }

    async fn apply_media_rename_bulk(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameBulkApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let MediaRenameBulkApplyInput {
            facet,
            fingerprint,
            idempotency_key,
        } = input;
        let facet = facet.into_domain();
        let facet_name = facet.as_str();
        let idempotency_key =
            claim_rename_idempotency_key("apply_media_rename_bulk", idempotency_key)?;

        let result = app
            .apply_rename_for_facet(&actor, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = app
            .record_rename_apply_audit(
                &actor,
                "rename_apply_facet",
                facet_name,
                None,
                idempotency_key.as_deref(),
                &result,
            )
            .await;

        Ok(from_media_rename_apply(result))
    }

    async fn rehydrate_all_metadata(
        &self,
        ctx: &Context<'_>,
        input: RehydrateAllMetadataInput,
    ) -> GqlResult<RehydrateAllMetadataPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let language = input.language;
        let cleared = app
            .rehydrate_all_metadata(&actor, &language)
            .await
            .map_err(to_gql_error)?;

        tracing::info!(
            language = %language,
            titles_cleared = cleared,
            "metadata rehydration accepted"
        );

        Ok(RehydrateAllMetadataPayload {
            language,
            titles_cleared: i64::try_from(cleared).unwrap_or(i64::MAX),
        })
    }
}
