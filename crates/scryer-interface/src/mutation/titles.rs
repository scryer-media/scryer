use async_graphql::{Context, ID, MaybeUndefined, Object, Result as GqlResult};
use scryer_application::{
    AppError, AppUseCase, DeleteExecutionConfirmation, DeleteTitlesJobItem, DeleteTitlesJobRequest,
    QueuedReleaseSelection,
};
use scryer_domain::{Library, LibraryPermission, LibraryRoot, MediaFacet, Title, User};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_job_run, from_library_scan_summary, from_title, monitor_selection_from_input,
};
use crate::types::*;
use crate::utils::{
    ResolvedTitleOptionsInput, map_add_input, merge_title_option_tags, normalize_title_tags,
    parse_download_source_kind,
};

#[derive(Default)]
pub(crate) struct TitleMutations;

fn queued_download_payload(
    title: &scryer_domain::Title,
    job_id: String,
    source_title: Option<String>,
    source_kind: Option<scryer_application::DownloadSourceKind>,
) -> QueueDownloadPayload {
    QueueDownloadPayload {
        status: QueueDownloadResultStatusValue::Queued,
        job_id: Some(job_id.into()),
        title_id: title.id.clone().into(),
        title_name: title.name.clone(),
        source_title,
        source_kind: source_kind.map(DownloadSourceKindValue::from_application),
        conflict: None,
    }
}

fn validation_error(message: impl Into<String>) -> async_graphql::Error {
    to_gql_error(AppError::Validation(message.into()))
}

fn normalized_root_folder_id(root_folder_id: &ID) -> GqlResult<&str> {
    let value = root_folder_id.as_ref().trim();
    if value.is_empty() {
        return Err(validation_error("rootFolderId cannot be empty"));
    }
    Ok(value)
}

fn find_library_root<'a>(
    libraries: &'a [Library],
    root_folder_id: &str,
) -> Option<(&'a Library, &'a LibraryRoot)> {
    libraries.iter().find_map(|library| {
        library
            .roots
            .iter()
            .find(|root| root.id == root_folder_id)
            .map(|root| (library, root))
    })
}

fn root_folder_id_override(root: &LibraryRoot) -> Option<Option<String>> {
    Some(Some(root.id.clone()))
}

fn resolved_title_options(
    facet: &MediaFacet,
    options: TitleOptionsInput,
    root_folder_id: Option<Option<String>>,
) -> GqlResult<ResolvedTitleOptionsInput> {
    let TitleOptionsInput {
        quality_profile_id,
        root_folder_id: _,
        monitor_type,
        use_season_folders,
        metadata_language,
        monitor_specials,
        inter_season_movies,
        filler_policy,
        recap_policy,
        monitor_selection,
    } = options;

    let use_season_folders = match use_season_folders {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(value)),
    };
    if *facet == MediaFacet::Movie && use_season_folders.is_some() {
        return Err(validation_error(
            "useSeasonFolders is only valid for series and anime titles",
        ));
    }

    let monitor_selection = match monitor_selection {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(monitor_selection_from_input(value))),
    };
    if *facet == MediaFacet::Movie && monitor_selection.is_some() {
        return Err(validation_error(
            "monitorSelection is only valid for series and anime titles",
        ));
    }
    // ADVANCED is meaningless without picks, and picks are meaningless without
    // ADVANCED, so the pair has to agree whenever either half is supplied.
    let requested_monitor_type = match &monitor_type {
        MaybeUndefined::Value(value) => Some(value.as_tag_value()),
        _ => None,
    };
    if requested_monitor_type == Some("advanced") {
        let has_selection = monitor_selection
            .as_ref()
            .and_then(|value| value.as_ref())
            .is_some_and(|selection| !selection.normalized().is_empty());
        if !has_selection {
            return Err(validation_error(
                "monitorSelection with at least one season or series movie is required when monitorType is ADVANCED",
            ));
        }
    } else if requested_monitor_type.is_some()
        && monitor_selection
            .as_ref()
            .and_then(|value| value.as_ref())
            .is_some()
    {
        return Err(validation_error(
            "monitorSelection is only valid when monitorType is ADVANCED",
        ));
    }

    Ok(ResolvedTitleOptionsInput {
        quality_profile_id: match quality_profile_id {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value.to_string())),
        },
        root_folder_id,
        monitor_type: match monitor_type {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value.as_tag_value().to_string())),
        },
        use_season_folders,
        metadata_language: match metadata_language {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(
                scryer_application::normalize_metadata_language_code(&value).ok_or_else(|| {
                    validation_error(
                        "metadataLanguage must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn",
                    )
                })?,
            )),
        },
        monitor_specials: match monitor_specials {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value)),
        },
        inter_season_movies: match inter_season_movies {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value)),
        },
        filler_policy: match filler_policy {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value.as_app_str().to_string())),
        },
        recap_policy: match recap_policy {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(value) => Some(Some(value.as_app_str().to_string())),
        },
        monitor_selection,
    })
}

async fn manageable_libraries_for_facet(
    app: &AppUseCase,
    actor: &User,
    facet: MediaFacet,
) -> GqlResult<Vec<Library>> {
    app.list_libraries_for_permission(actor, Some(facet), LibraryPermission::ManageTitles)
        .await
        .map_err(to_gql_error)
}

async fn resolve_add_title_options(
    app: &AppUseCase,
    actor: &User,
    facet: MediaFacet,
    library_id: Option<String>,
    options: Option<TitleOptionsInput>,
) -> GqlResult<(Option<String>, Option<ResolvedTitleOptionsInput>)> {
    let Some(options) = options else {
        return Ok((library_id, None));
    };
    let root_folder_id = options.root_folder_id.clone();
    let (library_id, root_folder_id) = match root_folder_id {
        MaybeUndefined::Undefined => (library_id, None),
        MaybeUndefined::Null => (library_id, Some(None)),
        MaybeUndefined::Value(root_folder_id) => {
            let root_folder_id = normalized_root_folder_id(&root_folder_id)?;
            let libraries = manageable_libraries_for_facet(app, actor, facet.clone()).await?;
            let (root_library, root) =
                find_library_root(&libraries, root_folder_id).ok_or_else(|| {
                    validation_error("rootFolderId must reference a configured library root")
                })?;
            if let Some(library_id) = library_id.as_deref()
                && root_library.id != library_id
            {
                return Err(validation_error(
                    "rootFolderId must reference a root on the selected library",
                ));
            }
            (
                library_id.or_else(|| Some(root_library.id.clone())),
                root_folder_id_override(root),
            )
        }
    };

    Ok((
        library_id,
        Some(resolved_title_options(&facet, options, root_folder_id)?),
    ))
}

async fn resolve_update_title_options(
    app: &AppUseCase,
    actor: &User,
    title: &Title,
    facet: &MediaFacet,
    options: TitleOptionsInput,
) -> GqlResult<ResolvedTitleOptionsInput> {
    let root_folder_id = options.root_folder_id.clone();
    let root_folder_id = match root_folder_id {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(root_folder_id) => {
            let root_folder_id = normalized_root_folder_id(&root_folder_id)?;
            let libraries = manageable_libraries_for_facet(app, actor, title.facet.clone()).await?;
            let library = libraries
                .iter()
                .find(|library| library.id == title.library_id)
                .ok_or_else(|| {
                    validation_error("title library is not available for rootFolderId validation")
                })?;
            let root = library
                .roots
                .iter()
                .find(|root| root.id == root_folder_id)
                .ok_or_else(|| {
                    validation_error("rootFolderId must reference a root on the title library")
                })?;
            root_folder_id_override(root)
        }
    };

    resolved_title_options(facet, options, root_folder_id)
}

#[Object]
impl TitleMutations {
    /// Create or reuse a title in an accessible library and return its hydration state.
    async fn add_title(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title metadata, library selection, and optional tri-state title settings."
        )]
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet = input.facet.into_domain();
        let library_id = input.library_id.clone().map(String::from);
        let options = input.options.clone();
        let (library_id, resolved_options) =
            resolve_add_title_options(&app, &actor, facet, library_id, options).await?;
        let metadata_language_override = resolved_options
            .as_ref()
            .and_then(|options| options.metadata_language.clone());
        let options_patch = resolved_options
            .as_ref()
            .map(ResolvedTitleOptionsInput::to_application_patch)
            .unwrap_or_default();
        let request = map_add_input(input, resolved_options)?;
        let result = if let Some(library_id) = library_id {
            app.add_title_with_options_patch_outcome_in_library(
                &actor,
                request,
                library_id,
                options_patch,
            )
            .await
        } else {
            let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
            app.add_title_with_options_patch_outcome_in_library(
                &actor,
                request,
                library_id,
                options_patch,
            )
            .await
        }
        .map_err(to_gql_error)?;
        if let Some(language) = metadata_language_override {
            app.set_title_metadata_language_override(&actor, &result.title.id, language)
                .await
                .map_err(to_gql_error)?;
        }

        Ok(AddTitleResult {
            title: from_title(&app, result.title),
            metadata_hydration_state: AddTitleHydrationStateValue::from_application(
                result.metadata_hydration_state,
            ),
            reused_existing_title: result.reused_existing_title,
            reused_queued_download: false,
            download_job_id: None,
            queued_download: None,
        })
    }

    /// Create or reuse a title and enqueue a download using the supplied source metadata.
    async fn add_title_and_queue_download(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title metadata, library selection, title settings, and optional source release details."
        )]
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let source_hint = input.source_hint.clone();
        let source_kind = parse_download_source_kind(input.source_kind);
        let source_title = input.source_title.clone();
        let facet = input.facet.into_domain();
        let library_id = input.library_id.clone().map(String::from);
        let options = input.options.clone();
        let (library_id, resolved_options) =
            resolve_add_title_options(&app, &actor, facet, library_id, options).await?;
        let metadata_language_override = resolved_options
            .as_ref()
            .and_then(|options| options.metadata_language.clone());
        let options_patch = resolved_options
            .as_ref()
            .map(ResolvedTitleOptionsInput::to_application_patch)
            .unwrap_or_default();
        let request = map_add_input(input, resolved_options)?;
        let queued_release = QueuedReleaseSelection {
            indexer_id: None,
            source_hint,
            source_kind,
            source_title: source_title.clone(),
            source_password: None,
            info_hash_hint: None,
            size_bytes: None,
            seeders: None,
        };
        let result = if let Some(library_id) = library_id {
            app.add_title_and_queue_download_with_options_patch_outcome_in_library(
                &actor,
                request,
                library_id,
                options_patch,
                queued_release,
            )
            .await
        } else {
            let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
            app.add_title_and_queue_download_with_options_patch_outcome_in_library(
                &actor,
                request,
                library_id,
                options_patch,
                queued_release,
            )
            .await
        }
        .map_err(to_gql_error)?;
        if let Some(language) = metadata_language_override {
            app.set_title_metadata_language_override(&actor, &result.title.id, language)
                .await
                .map_err(to_gql_error)?;
        }
        let queued_download = queued_download_payload(
            &result.title,
            result.download_job_id.clone(),
            source_title,
            source_kind,
        );

        Ok(AddTitleResult {
            title: from_title(&app, result.title),
            metadata_hydration_state: AddTitleHydrationStateValue::from_application(
                result.metadata_hydration_state,
            ),
            reused_existing_title: result.reused_existing_title,
            reused_queued_download: result.reused_queued_download,
            download_job_id: Some(result.download_job_id.into()),
            queued_download: Some(queued_download),
        })
    }

    /// Patch title metadata and settings while preserving omitted fields.
    async fn update_title(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity and optional metadata or tri-state settings changes.")]
        input: UpdateTitleInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let UpdateTitleInput {
            title_id,
            name,
            facet,
            tags,
            options,
        } = input;
        let title_id = title_id.to_string();
        let facet = facet.map(MediaFacetValue::into_domain);
        let mut tags = tags.map(normalize_title_tags);
        let mut root_folder_id = None;
        let mut metadata_language_override = None;
        let mut monitor_selection_update = None;

        if let Some(options) = options {
            let title = app
                .get_title_for_management(&actor, &title_id)
                .await
                .map_err(to_gql_error)?
                .ok_or_else(|| to_gql_error(AppError::NotFound(format!("title {title_id}"))))?;
            let base_tags = match tags.take() {
                Some(tags) => tags,
                None => title.tags.clone(),
            };
            let target_facet = facet.as_ref().unwrap_or(&title.facet);
            let resolved_options =
                resolve_update_title_options(&app, &actor, &title, target_facet, options).await?;
            root_folder_id = resolved_options.root_folder_id.clone();
            metadata_language_override = resolved_options.metadata_language.clone();
            // Applied after the tags land: whether the selection is kept or
            // cleared is decided by the monitor type the update just stored.
            monitor_selection_update = Some(resolved_options.monitor_selection.clone());
            tags = Some(merge_title_option_tags(base_tags, resolved_options));
        }

        let title = app
            .update_title_metadata_with_root_folder_id(
                &actor,
                &title_id,
                name,
                facet,
                tags,
                root_folder_id,
            )
            .await
            .map_err(to_gql_error)?;
        if let Some(monitor_selection) = monitor_selection_update {
            app.set_title_monitor_selection(&actor, &title.id, monitor_selection)
                .await
                .map_err(to_gql_error)?;
        }
        if let Some(language) = metadata_language_override {
            app.set_title_metadata_language_override(&actor, &title.id, language)
                .await
                .map_err(to_gql_error)?;
        }
        Ok(from_title(&app, title))
    }

    /// Set the primary media file for a movie title.
    async fn set_primary_movie_file(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Movie title identity and media-file identity to make primary.")]
        input: SetPrimaryMovieFileInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let file_id = input.file_id.to_string();
        let title = app
            .set_primary_movie_file(&actor, &title_id, &file_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(&app, title))
    }

    /// Associate a title with a metadata identity and return any hydration or scan result.
    async fn fix_title_match(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity and metadata identity used for the rematch.")]
        input: FixTitleMatchInput,
    ) -> GqlResult<FixTitleMatchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let result = app
            .fix_title_match(&actor, &title_id, input.tvdb_id.as_deref(), input.smg_id)
            .await
            .map_err(to_gql_error)?;

        Ok(FixTitleMatchPayload {
            title: from_title(&app, result.title),
            hydrated: result.hydrated,
            library_scan: result.library_scan.map(from_library_scan_summary),
            warnings: result.warnings,
        })
    }

    /// Delete one title, optionally removing its files after preview confirmation.
    async fn delete_title(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title identity, disk-deletion choice, and optional preview fingerprint and typed confirmation."
        )]
        input: DeleteTitleInput,
    ) -> GqlResult<DeleteTitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        app.delete_title(
            &actor,
            &title_id,
            input.delete_files_on_disk.unwrap_or(false),
            input
                .preview_fingerprint
                .map(|preview_fingerprint| DeleteExecutionConfirmation {
                    preview_fingerprint,
                    typed_confirmation: input.typed_confirmation,
                }),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(DeleteTitlePayload {
            id: ID::from(title_id),
        })
    }

    /// Accept a background job to delete selected titles and optionally their files.
    async fn delete_titles(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title deletion items, optional disk-deletion choice, and typed confirmation."
        )]
        input: DeleteTitlesInput,
    ) -> GqlResult<DeleteTitlesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .start_delete_titles_job(
                &actor,
                DeleteTitlesJobRequest {
                    items: input
                        .items
                        .into_iter()
                        .map(|item| DeleteTitlesJobItem {
                            title_id: item.title_id.to_string(),
                            preview_fingerprint: item.preview_fingerprint,
                        })
                        .collect(),
                    delete_files_on_disk: input.delete_files_on_disk.unwrap_or(false),
                    typed_confirmation: input.typed_confirmation,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteTitlesPayload {
            job_run: from_job_run(accepted.job_run),
            accepted_title_ids: accepted
                .accepted_title_ids
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// Clear one title release blocklist entry so it can be considered again.
    async fn clear_title_release_blocklist_entry(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title release blocklist entry identity to clear.")] id: ID,
    ) -> GqlResult<ClearTitleReleaseBlocklistEntryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.clear_title_release_blocklist_entry(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ClearTitleReleaseBlocklistEntryPayload { id: ID::from(id) })
    }

    /// Replace a title's monitored state.
    async fn set_title_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity and desired monitored state.")]
        input: SetTitleMonitoredInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let title = app
            .set_title_monitored(&actor, &title_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(&app, title))
    }
}
