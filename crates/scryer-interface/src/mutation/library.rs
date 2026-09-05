use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::{AppError, DeleteExecutionConfirmation};
use scryer_domain::AppPermission;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_cancel_library_scan_result, from_delete_episode_files_job_accepted,
    from_ignore_pending_import_result, from_job_run, from_library, from_library_scan_session,
    from_library_scan_summary, from_media_rename_apply, from_resolve_pending_import_result,
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
        metadata_language: input.metadata_language,
        use_season_folders: input.use_season_folders,
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
                        seeding_profile_id: entry.seeding_profile_id.map(|value| value.to_string()),
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
    /// Create a media library with roots and optional acquisition or import settings.
    async fn create_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Library facet, name, root paths, and optional settings.")]
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

    /// Patch a library while preserving omitted fields.
    async fn update_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Library identity and optional replacement name, roots, or settings.")]
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

    /// Delete the library configuration identified by the supplied ID.
    async fn delete_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Library identity to delete.")] id: ID,
    ) -> GqlResult<DeleteLibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.delete_library(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteLibraryPayload { id: ID::from(id) })
    }

    /// Start a library scan and return the accepted scan session snapshot.
    async fn scan_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Library identity and optional completed import-warmup session for scan hints."
        )]
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

    /// Scan the library paths associated with one title and return the summary.
    async fn scan_title_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity whose library paths should be scanned.")] title_id: ID,
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

    /// Request cancellation of a library scan and return the cancellation result.
    async fn cancel_library_scan(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Library scan session identity to cancel.")] session_id: ID,
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

    /// Resolve a pending import by creating or selecting its title metadata.
    async fn resolve_pending_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-import identity and title metadata used to resolve it.")]
        input: ResolvePendingImportInput,
    ) -> GqlResult<ResolvePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = input.pending_import_id.to_string();
        let attach_to_existing_title = input.attach_to_existing_title;
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
            .resolve_pending_import(
                &actor,
                &pending_import_id,
                request,
                attach_to_existing_title,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_resolve_pending_import_result(&app, result))
    }

    /// Bind a pending import to an existing collection and optional episodes.
    async fn bind_pending_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Pending-import identity, optional collection identity, and episode identities to bind."
        )]
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

    /// Mark a pending import ignored without importing its files.
    async fn ignore_pending_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-import identity to ignore.")] pending_import_id: ID,
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

    /// Apply a title rename plan after validating its preview fingerprint and optional idempotency key.
    #[graphql(deprecation = "use renameTitles, which runs the work as a job")]
    async fn apply_media_rename(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Facet, title identity, preview fingerprint, and optional idempotency key."
        )]
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

    /// Accept a background job to delete a media file, optionally removing it from disk.
    async fn delete_media_file(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Media-file identity, disk-deletion choice, and optional preview fingerprint and typed confirmation."
        )]
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

    /// Accept a background job deleting every media file linked to the supplied
    /// episodes of one title.
    ///
    /// The aggregate preview from `deleteEpisodeFilesPreview` must still match
    /// when files are removed from disk. Per-file results land on the returned
    /// job run: a file that fails is recorded there while the rest still run.
    async fn delete_episode_files(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title and episode identities, disk-deletion choice, and the aggregate preview fingerprint with optional typed confirmation."
        )]
        input: DeleteEpisodeFilesInput,
    ) -> GqlResult<DeleteEpisodeFilesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let episode_ids = input
            .episode_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let accepted = app
            .start_delete_episode_files_job(
                &actor,
                &title_id,
                &episode_ids,
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
        Ok(from_delete_episode_files_job_accepted(accepted))
    }

    /// Start a background job renaming the files of the given titles.
    async fn rename_titles(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Facet and titles whose files are renamed; the work runs in the background."
        )]
        input: RenameTitlesInput,
    ) -> GqlResult<RenameTitlesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_ids = input
            .title_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let accepted = app
            .start_rename_titles_job(&actor, &title_ids, input.facet.into_domain())
            .await
            .map_err(to_gql_error)?;
        Ok(RenameTitlesPayload {
            job_run: from_job_run(accepted.job_run),
            accepted_title_ids: accepted
                .accepted_title_ids
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// Apply a facet-wide rename plan after validating its preview fingerprint and optional idempotency key.
    #[graphql(deprecation = "use renameTitles, which runs the work as a job")]
    async fn apply_media_rename_bulk(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Facet, preview fingerprint, and optional idempotency key.")]
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

    /// Rehydrate accessible title metadata for one language and report cleared title counts.
    async fn rehydrate_all_metadata(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Metadata language code used for rehydration.")]
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
