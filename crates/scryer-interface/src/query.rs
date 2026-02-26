use async_graphql::{Context, Error, Object, Result as GqlResult};

use chrono::Utc;
use scryer_application::{parse_release_metadata, RenamePlan};
use scryer_domain::PolicyInput;
use serde_json::json;

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{
    from_activity_event, from_collection, from_download_client_config, from_episode,
    from_download_queue_item, from_event, from_indexer_config, from_media_rename_plan,
    from_release_decision, from_system_health, from_title, from_title_media_file,
    from_title_release_blocklist_entry, from_wanted_item, map_admin_setting, from_user,
    file_size_bytes_for_path,
};
use crate::types::*;
use crate::utils::parse_facet;

#[derive(Copy, Clone)]
pub struct QueryRoot;

#[allow(clippy::too_many_arguments)]
#[Object]
impl QueryRoot {
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<String>,
        query: Option<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parsed_facet = parse_facet(facet);
        let titles = app
            .list_titles(&actor, parsed_facet, query)
            .await
            .map_err(to_gql_error)?;

        let title_ids: Vec<String> = titles.iter().map(|t| t.id.clone()).collect();
        let summaries = app
            .list_primary_collection_summaries(&actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        let summary_map: std::collections::HashMap<&str, _> = summaries
            .iter()
            .map(|s| (s.title_id.as_str(), s))
            .collect();

        Ok(titles
            .into_iter()
            .map(|t| {
                let id = t.id.clone();
                let mut payload = from_title(t);
                if let Some(s) = summary_map.get(id.as_str()) {
                    payload.quality_tier = s.label.clone();
                    payload.size_bytes = file_size_bytes_for_path(s.ordered_path.as_deref());
                }
                payload
            })
            .collect())
    }

    async fn title(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn title_collections(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<Vec<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collections = app
            .list_collections(&actor, &title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(collections.into_iter().map(from_collection).collect())
    }

    async fn media_rename_preview(
        &self,
        ctx: &Context<'_>,
        input: MediaRenamePreviewInput,
    ) -> GqlResult<MediaRenamePlanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let _ = input.dry_run;
        let facet = parse_facet(Some(input.facet))
            .ok_or_else(|| Error::new("invalid facet for mediaRenamePreview"))?;
        let plan = if let Some(title_id) = input.title_id {
            app.preview_rename_for_title(&actor, &title_id, facet)
                .await
                .map_err(to_gql_error)?
        } else {
            app.preview_rename_for_facet(&actor, facet)
                .await
                .map_err(to_gql_error)?
        };

        if let Ok(db) = settings_db_from_ctx(ctx) {
            let _ = record_rename_preview_audit(&db, &actor.id, &plan).await;
        }

        Ok(from_media_rename_plan(plan))
    }

    async fn collection_episodes(
        &self,
        ctx: &Context<'_>,
        collection_id: String,
    ) -> GqlResult<Vec<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_episodes(&actor, &collection_id)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_episode).collect())
    }

    async fn title_media_files(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let mut files = app
            .services
            .media_files
            .list_media_files_for_title(&title_id)
            .await
            .map_err(to_gql_error)?;

        // Backfill: link unlinked files to episodes by parsing file paths
        for file in &mut files {
            if file.episode_id.is_some() {
                continue;
            }
            let stem = std::path::Path::new(&file.file_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let parsed = parse_release_metadata(stem);
            if let Some(ref ep_meta) = parsed.episode {
                let season_str = ep_meta
                    .season
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "1".to_string());
                if let Some(ep_num) = ep_meta.episode_numbers.first() {
                    let ep_str = ep_num.to_string();
                    if let Ok(Some(episode)) = app
                        .services
                        .shows
                        .find_episode_by_title_and_numbers(&title_id, &season_str, &ep_str)
                        .await
                    {
                        let _ = app
                            .services
                            .media_files
                            .link_file_to_episode(&file.id, &episode.id)
                            .await;
                        file.episode_id = Some(episode.id);
                    }
                }
            }
        }

        Ok(files.into_iter().map(from_title_media_file).collect())
    }

    async fn collection(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn episode(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }

    async fn policy_preview(
        &self,
        ctx: &Context<'_>,
        input: PolicyInputPayload,
    ) -> GqlResult<PolicyOutputPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let decision = app
            .evaluate_policy(
                &actor,
                PolicyInput {
                    title_id: input.title_id,
                    facet: parse_facet(Some(input.facet)).unwrap_or(scryer_domain::MediaFacet::Other),
                    has_existing_file: input.has_existing_file,
                    candidate_quality: input.candidate_quality,
                    requested_mode: input.requested_mode,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_policy(
            decision,
        ))
    }

    async fn search_indexers(
        &self,
        ctx: &Context<'_>,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(200).clamp(1, 200) as usize;
        let results = app
            .search_indexers(&actor, query, imdb_id, tvdb_id, category, limit)
            .await
            .map_err(to_gql_error)?;

        Ok(results.into_iter().map(crate::mappers::from_search_result).collect())
    }

    async fn search_indexers_episode(
        &self,
        ctx: &Context<'_>,
        title: String,
        season: String,
        episode: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(200).clamp(1, 200) as usize;
        let results = app
            .search_indexers_episode(
                &actor,
                title,
                season,
                episode,
                imdb_id,
                tvdb_id,
                category,
                limit,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(results.into_iter().map(crate::mappers::from_search_result).collect())
    }

    async fn title_events(
        &self,
        ctx: &Context<'_>,
        title_id: Option<String>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<EventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let events = app
            .recent_events(
                &actor,
                title_id,
                limit.unwrap_or(100) as i64,
                offset.unwrap_or(0) as i64,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_event).collect())
    }

    async fn title_release_blocklist(
        &self,
        ctx: &Context<'_>,
        title_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleReleaseBlocklistEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_title_release_blocklist(&actor, &title_id, limit.unwrap_or(100).max(1) as usize)
            .await
            .map_err(to_gql_error)?;
        Ok(items
            .into_iter()
            .map(from_title_release_blocklist_entry)
            .collect())
    }

    async fn activity_events(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<ActivityEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let events = app
            .recent_activity(
                &actor,
                limit.unwrap_or(100) as i64,
                offset.unwrap_or(0) as i64,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_activity_event).collect())
    }

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_download_queue(
                &actor,
                include_all_activity.unwrap_or(false),
                include_history_only.unwrap_or(false),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_download_queue_item).collect())
    }

    async fn admin_settings(
        &self,
        ctx: &Context<'_>,
        scope: Option<String>,
        scope_id: Option<String>,
        category: Option<String>,
    ) -> GqlResult<AdminSettingsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        let scope = scope.unwrap_or_else(|| "system".to_string());
        let category_filter = category.map(|value| value.trim().to_string());

        let records = db
            .list_settings_with_defaults(&scope, scope_id.clone())
            .await
            .map_err(to_gql_error)?;

        let items = records
            .into_iter()
            .filter(|record| {
                category_filter
                    .as_deref()
                    .is_none_or(|target| record.category == target)
            })
            .map(map_admin_setting)
            .collect();

        Ok(AdminSettingsPayload {
            scope,
            scope_id,
            items,
            quality_profiles: None,
        })
    }

    async fn indexers(
        &self,
        ctx: &Context<'_>,
        provider_type: Option<String>,
    ) -> GqlResult<Vec<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_indexer_configs(&actor, provider_type)
            .await
            .map_err(to_gql_error)?;
        Ok(configs.into_iter().map(from_indexer_config).collect())
    }

    async fn indexer(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .get_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_indexer_config);
        Ok(config)
    }

    async fn download_client_configs(
        &self,
        ctx: &Context<'_>,
        client_type: Option<String>,
    ) -> GqlResult<Vec<DownloadClientConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_download_client_configs(&actor, client_type)
            .await
            .map_err(to_gql_error)?;
        Ok(configs.into_iter().map(from_download_client_config).collect())
    }

    async fn download_client_config(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<DownloadClientConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .get_download_client_config(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_download_client_config);
        Ok(config)
    }

    async fn users(&self, ctx: &Context<'_>) -> GqlResult<Vec<UserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let users = app.list_users(&actor).await.map_err(to_gql_error)?;
        Ok(users.into_iter().map(from_user).collect())
    }

    async fn user(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<UserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user = app.get_user(&actor, &id).await.map_err(to_gql_error)?;
        Ok(user.map(from_user))
    }

    async fn system_health(&self, ctx: &Context<'_>) -> GqlResult<SystemHealthPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let health = app.system_health(&actor).await.map_err(to_gql_error)?;
        Ok(from_system_health(health))
    }

    async fn import_history(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<ImportRecordPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewHistory) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let records = app
            .services
            .imports
            .list_imports(limit)
            .await
            .map_err(to_gql_error)?;
        Ok(records
            .into_iter()
            .map(crate::mappers::from_import_record)
            .collect())
    }

    async fn preview_manual_import(
        &self,
        ctx: &Context<'_>,
        download_client_item_id: String,
        title_id: String,
    ) -> GqlResult<ManualImportPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::TriggerActions) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

        let preview = scryer_application::preview_manual_import(
            &app,
            &download_client_item_id,
            &title_id,
        )
        .await
        .map_err(to_gql_error)?;

        Ok(ManualImportPreviewPayload {
            files: preview
                .files
                .into_iter()
                .map(|f| ManualImportFilePreviewPayload {
                    file_path: f.file_path,
                    file_name: f.file_name,
                    size_bytes: f.size_bytes.to_string(),
                    quality: f.quality,
                    parsed_season: f.parsed_season.map(|v| v as i32),
                    parsed_episodes: f
                        .parsed_episodes
                        .into_iter()
                        .map(|v| v as i32)
                        .collect(),
                    suggested_episode_id: f.suggested_episode_id,
                    suggested_episode_label: f.suggested_episode_label,
                })
                .collect(),
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(from_episode)
                .collect(),
        })
    }

    async fn me(&self, ctx: &Context<'_>) -> GqlResult<Option<UserPayload>> {
        match ctx.data_opt::<scryer_domain::User>() {
            Some(user) => Ok(Some(from_user(user.clone()))),
            None => Ok(None),
        }
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
    ) -> GqlResult<WantedItemsListPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let (items, total) = app
            .list_wanted_items(
                status.as_deref(),
                media_type.as_deref(),
                title_id.as_deref(),
                limit,
                offset,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(WantedItemsListPayload {
            items: items.into_iter().map(from_wanted_item).collect(),
            total,
        })
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        wanted_item_id: Option<String>,
        title_id: Option<String>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(
                wanted_item_id.as_deref(),
                title_id.as_deref(),
                limit,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }
}

async fn record_rename_preview_audit(
    db: &scryer_infrastructure::SqliteServices,
    actor_user_id: &str,
    plan: &RenamePlan,
) -> Result<(), scryer_application::AppError> {
    let now = Utc::now().to_rfc3339();
    let fingerprint = plan.fingerprint.clone();
    let progress_json = json!({
        "operation": "rename_preview",
        "facet": format!("{:?}", plan.facet).to_lowercase(),
        "title_id": plan.title_id.clone(),
        "fingerprint": fingerprint.clone(),
        "total": plan.total,
        "renamable": plan.renamable,
        "noop": plan.noop,
        "conflicts": plan.conflicts,
        "errors": plan.errors,
    })
    .to_string();

    let _ = db
        .create_workflow_operation(
            "rename_preview",
            "completed",
            Some(actor_user_id.to_string()),
            Some(progress_json),
            Some(now.clone()),
            Some(now),
        )
        .await?;

    let source_ref = plan
        .title_id
        .as_ref()
        .map(|title_id| format!("title:{title_id}:{fingerprint}"))
        .unwrap_or_else(|| {
            format!(
                "facet:{}:{}",
                format!("{:?}", plan.facet).to_lowercase(),
                fingerprint
            )
        });
    let payload_json = serde_json::to_string(plan)
        .unwrap_or_else(|_| "{\"error\":\"failed_to_serialize_rename_plan\"}".to_string());

    let _ = db
        .create_import_request(
            "scryer_rename".to_string(),
            source_ref,
            "rename_plan_preview".to_string(),
            payload_json,
        )
        .await?;

    Ok(())
}
