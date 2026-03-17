use async_graphql::{Context, Error, Object, Result as GqlResult};

use chrono::Utc;
use scryer_application::TitleHistoryFilter;
use scryer_application::{RenamePlan, parse_release_metadata};
use scryer_domain::{MediaFacet, PolicyInput, TitleHistoryEventType};
use serde_json::json;

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{
    from_activity_event, from_backup_info, from_calendar_episode, from_collection, from_disk_space,
    from_download_client_config, from_download_queue_item, from_episode, from_health_check_result,
    from_indexer_config, from_media_rename_plan, from_pending_release, from_provider_type,
    from_release_decision, from_system_health, from_title, from_title_history_page,
    from_title_history_record, from_title_media_file, from_title_release_blocklist_entry,
    from_user, from_wanted_item, map_admin_setting,
};
use crate::types::*;
use crate::utils::parse_facet;

#[derive(Copy, Clone)]
pub struct QueryRoot;

#[expect(clippy::too_many_arguments)]
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
        let media_size_summaries = app
            .list_title_media_size_summaries(&actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        let summary_map: std::collections::HashMap<&str, _> =
            summaries.iter().map(|s| (s.title_id.as_str(), s)).collect();
        let media_size_map: std::collections::HashMap<&str, i64> = media_size_summaries
            .iter()
            .map(|summary| (summary.title_id.as_str(), summary.total_size_bytes))
            .collect();

        Ok(titles
            .into_iter()
            .map(|t| {
                let id = t.id.clone();
                let mut payload = from_title(t);
                if let Some(s) = summary_map.get(id.as_str()) {
                    payload.quality_tier = s.label.clone();
                }
                payload.size_bytes = media_size_map.get(id.as_str()).copied();
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
                    facet: parse_facet(Some(input.facet))
                        .unwrap_or(scryer_domain::MediaFacet::Other),
                    has_existing_file: input.has_existing_file,
                    candidate_quality: input.candidate_quality,
                    requested_mode: input.requested_mode,
                    release_title: None,
                    quality_profile_id: None,
                    category: None,
                    tags: vec![],
                    is_anime: false,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_policy(decision))
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

        Ok(results
            .into_iter()
            .map(crate::mappers::from_search_result)
            .collect())
    }

    async fn search_indexers_episode(
        &self,
        ctx: &Context<'_>,
        title: String,
        season: String,
        episode: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        absolute_episode: Option<i32>,
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
                anidb_id,
                category,
                absolute_episode.map(|v| v as u32),
                limit,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(results
            .into_iter()
            .map(crate::mappers::from_search_result)
            .collect())
    }

    async fn search_indexers_season(
        &self,
        ctx: &Context<'_>,
        title: String,
        season: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(200).clamp(1, 200) as usize;
        let results = app
            .search_indexers_season(&actor, title, season, imdb_id, tvdb_id, category, limit)
            .await
            .map_err(to_gql_error)?;

        Ok(results
            .into_iter()
            .map(crate::mappers::from_search_result)
            .collect())
    }

    async fn title_events(
        &self,
        ctx: &Context<'_>,
        title_id: Option<String>,
        event_types: Option<Vec<String>>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<TitleHistoryEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let parsed_types: Option<Vec<TitleHistoryEventType>> = event_types.map(|types| {
            types
                .iter()
                .filter_map(|s| TitleHistoryEventType::parse(s))
                .collect()
        });

        if let Some(ref tid) = title_id {
            let page = app
                .list_title_history_for_title(
                    &actor,
                    tid,
                    parsed_types.as_deref(),
                    limit.unwrap_or(100).max(1) as usize,
                    offset.unwrap_or(0).max(0) as usize,
                )
                .await
                .map_err(to_gql_error)?;
            Ok(page
                .records
                .into_iter()
                .map(from_title_history_record)
                .collect())
        } else {
            let filter = TitleHistoryFilter {
                event_types: parsed_types,
                title_ids: None,
                download_id: None,
                limit: limit.unwrap_or(100).max(1) as usize,
                offset: offset.unwrap_or(0).max(0) as usize,
            };
            let page = app
                .list_title_history(&actor, &filter)
                .await
                .map_err(to_gql_error)?;
            Ok(page
                .records
                .into_iter()
                .map(from_title_history_record)
                .collect())
        }
    }

    async fn title_history(
        &self,
        ctx: &Context<'_>,
        filter: TitleHistoryFilterInput,
    ) -> GqlResult<TitleHistoryPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let parsed_types = filter.event_types.map(|types| {
            types
                .iter()
                .filter_map(|s| TitleHistoryEventType::parse(s))
                .collect()
        });

        let f = TitleHistoryFilter {
            event_types: parsed_types,
            title_ids: filter.title_ids,
            download_id: filter.download_id,
            limit: filter.limit.unwrap_or(50).max(1) as usize,
            offset: filter.offset.unwrap_or(0).max(0) as usize,
        };

        let page = app
            .list_title_history(&actor, &f)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_history_page(page))
    }

    async fn episode_history(
        &self,
        ctx: &Context<'_>,
        episode_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleHistoryEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let records = app
            .list_title_history_for_episode(
                &actor,
                &episode_id,
                limit.unwrap_or(50).max(1) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(records.into_iter().map(from_title_history_record).collect())
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
        key_names: Option<Vec<String>>,
    ) -> GqlResult<AdminSettingsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        let scope = scope.unwrap_or_else(|| "system".to_string());
        let category_filter = category.map(|value| value.trim().to_string());
        let key_filter = key_names.map(|values| {
            values
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<std::collections::HashSet<_>>()
        });

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
                    && key_filter
                        .as_ref()
                        .is_none_or(|targets| targets.contains(record.key_name.as_str()))
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
        let stats = app.services.indexer_stats.all_stats();
        let mut payloads: Vec<IndexerConfigPayload> =
            configs.into_iter().map(from_indexer_config).collect();
        for payload in &mut payloads {
            if let Some(s) = stats.iter().find(|s| s.indexer_id == payload.id) {
                payload.last_query_at = s.last_query_at.clone();
            }
        }
        Ok(payloads)
    }

    async fn indexer(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mut payload = app
            .get_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_indexer_config);
        if let Some(ref mut p) = payload {
            let stats = app.services.indexer_stats.all_stats();
            if let Some(s) = stats.iter().find(|s| s.indexer_id == p.id) {
                p.last_query_at = s.last_query_at.clone();
            }
        }
        Ok(payload)
    }

    async fn root_folders(
        &self,
        ctx: &Context<'_>,
        facet: String,
    ) -> GqlResult<Vec<RootFolderPayload>> {
        let app = app_from_ctx(ctx)?;
        let media_facet = parse_facet(Some(facet)).unwrap_or(MediaFacet::Movie);
        let entries = app
            .root_folders_for_facet(&media_facet)
            .await
            .map_err(to_gql_error)?;
        Ok(entries
            .into_iter()
            .map(|e| RootFolderPayload {
                path: e.path,
                is_default: e.is_default,
            })
            .collect())
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
        Ok(configs
            .into_iter()
            .map(from_download_client_config)
            .collect())
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

    async fn health_checks(&self, ctx: &Context<'_>) -> GqlResult<Vec<HealthCheckPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let results = app.services.health_check_results.read().await;
        Ok(results
            .iter()
            .cloned()
            .map(from_health_check_result)
            .collect())
    }

    async fn disk_space(&self, ctx: &Context<'_>) -> GqlResult<Vec<DiskSpacePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let info = app.disk_space(&actor).await.map_err(to_gql_error)?;
        Ok(info.into_iter().map(from_disk_space).collect())
    }

    async fn backups(&self, ctx: &Context<'_>) -> GqlResult<Vec<BackupInfoPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let backups = app.list_backups(&actor).await.map_err(to_gql_error)?;
        Ok(backups.into_iter().map(from_backup_info).collect())
    }

    async fn pending_releases(&self, ctx: &Context<'_>) -> GqlResult<Vec<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let releases = app.list_pending_releases().await.map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
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

        let preview =
            scryer_application::preview_manual_import(&app, &download_client_item_id, &title_id)
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
                    parsed_episodes: f.parsed_episodes.into_iter().map(|v| v as i32).collect(),
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
            .list_release_decisions(wanted_item_id.as_deref(), title_id.as_deref(), limit)
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    // ── Rule Sets ──────────────────────────────────────────────────────

    async fn rule_sets(&self, ctx: &Context<'_>) -> GqlResult<Vec<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_sets = app.list_rule_sets(&actor).await.map_err(to_gql_error)?;
        Ok(rule_sets
            .into_iter()
            .map(crate::mappers::from_rule_set)
            .collect())
    }

    async fn rule_set(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set = app.get_rule_set(&actor, &id).await.map_err(to_gql_error)?;
        Ok(rule_set.map(crate::mappers::from_rule_set))
    }

    async fn convenience_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ConvenienceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .get_convenience_settings(&actor)
            .await
            .map_err(to_gql_error)?;

        Ok(ConvenienceSettingsPayload {
            required_audio: settings
                .required_audio
                .into_iter()
                .map(|s| ConvenienceAudioSettingPayload {
                    scope: s.scope,
                    languages: s.languages,
                    rule_set_id: s.rule_set_id,
                })
                .collect(),
            prefer_dual_audio: settings
                .prefer_dual_audio
                .into_iter()
                .map(|s| ConvenienceBoolSettingPayload {
                    scope: s.scope,
                    enabled: s.enabled,
                    rule_set_id: s.rule_set_id,
                })
                .collect(),
        })
    }

    // ── Post-Processing Scripts ──────────────────────────────────────────

    async fn post_processing_scripts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<PostProcessingScriptPayload>> {
        let app = app_from_ctx(ctx)?;

        let scripts = app
            .services
            .pp_scripts
            .list_scripts()
            .await
            .map_err(to_gql_error)?;
        Ok(scripts
            .into_iter()
            .map(crate::mappers::from_pp_script)
            .collect())
    }

    async fn post_processing_script_runs(
        &self,
        ctx: &Context<'_>,
        script_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<PostProcessingScriptRunPayload>> {
        let app = app_from_ctx(ctx)?;

        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let runs = app
            .services
            .pp_scripts
            .list_runs_for_script(&script_id, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(runs
            .into_iter()
            .map(crate::mappers::from_pp_script_run)
            .collect())
    }

    // ── Plugins ──────────────────────────────────────────────────────────

    async fn plugins(&self, ctx: &Context<'_>) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .list_available_plugins(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins
            .into_iter()
            .map(crate::mappers::from_registry_plugin)
            .collect())
    }

    /// List community rule packs from the plugin registry.
    async fn rule_pack_registry(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RulePackRegistryEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let packs = app
            .list_rule_pack_registry(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(packs
            .into_iter()
            .map(|p| RulePackRegistryEntryPayload {
                id: p.id,
                name: p.name,
                description: p.description,
                author: p.author,
                version: p.version,
            })
            .collect())
    }

    /// Fetch templates from a community rule pack by its registry ID.
    async fn rule_pack_templates(
        &self,
        ctx: &Context<'_>,
        pack_id: String,
    ) -> GqlResult<Vec<RulePackTemplatePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let templates = app
            .fetch_rule_pack_templates(&actor, &pack_id)
            .await
            .map_err(to_gql_error)?;
        Ok(templates
            .into_iter()
            .map(|t| RulePackTemplatePayload {
                id: t.id,
                title: t.title,
                description: t.description,
                category: t.category,
                rego_source: t.rego_source,
                applied_facets: t.applied_facets,
            })
            .collect())
    }

    /// Returns all available indexer provider types from loaded plugins,
    /// with their config field schemas for dynamic form rendering.
    async fn indexer_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_indexer_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(pt, name, fields, default_base_url)
            })
            .collect())
    }

    async fn download_client_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_download_client_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(pt, name, fields, default_base_url)
            })
            .collect())
    }

    // ── Metadata Gateway (proxied from SMG) ──────────────────────────────

    async fn search_metadata(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(name = "type")] type_hint: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<Vec<MetadataSearchItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.clamp(1, 100);
        let results = app
            .services
            .metadata_gateway
            .search_tvdb_rich(&query, &type_hint, limit, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(results
            .into_iter()
            .map(|item| MetadataSearchItemPayload {
                tvdb_id: item.tvdb_id,
                name: item.name,
                imdb_id: item.imdb_id,
                slug: item.slug,
                type_hint: item.type_hint,
                year: item.year,
                status: item.status,
                overview: item.overview,
                popularity: item.popularity,
                poster_url: item.poster_url,
                language: item.language,
                runtime_minutes: item.runtime_minutes,
                sort_title: item.sort_title,
            })
            .collect())
    }

    async fn search_metadata_multi(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSearchMultiPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.clamp(1, 100);
        let result = app
            .services
            .metadata_gateway
            .search_tvdb_multi(&query, limit, &language)
            .await
            .map_err(to_gql_error)?;
        let convert = |items: Vec<scryer_application::RichMetadataSearchItem>| {
            items
                .into_iter()
                .map(|item| MetadataSearchItemPayload {
                    tvdb_id: item.tvdb_id,
                    name: item.name,
                    imdb_id: item.imdb_id,
                    slug: item.slug,
                    type_hint: item.type_hint,
                    year: item.year,
                    status: item.status,
                    overview: item.overview,
                    popularity: item.popularity,
                    poster_url: item.poster_url,
                    language: item.language,
                    runtime_minutes: item.runtime_minutes,
                    sort_title: item.sort_title,
                })
                .collect()
        };
        Ok(MetadataSearchMultiPayload {
            movies: convert(result.movies),
            series: convert(result.series),
            anime: convert(result.anime),
        })
    }

    async fn metadata_movie(
        &self,
        ctx: &Context<'_>,
        tvdb_id: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataMoviePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let movie = app
            .services
            .metadata_gateway
            .get_movie(tvdb_id as i64, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataMoviePayload {
            tvdb_id: movie.tvdb_id.to_string(),
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            status: movie.content_status,
            overview: movie.overview,
            poster_url: movie.poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            genres: movie.genres,
            studio: movie.studio,
            tmdb_release_date: movie.tmdb_release_date,
        })
    }

    async fn metadata_series(
        &self,
        ctx: &Context<'_>,
        id: String,
        #[graphql(default = true)] include_episodes: bool,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSeriesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let tvdb_id: i64 = id.parse().map_err(|_| Error::new("invalid tvdb id"))?;
        let series = app
            .services
            .metadata_gateway
            .get_series(tvdb_id, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSeriesPayload {
            tvdb_id: series.tvdb_id.to_string(),
            name: series.name,
            sort_name: series.sort_name,
            slug: series.slug,
            year: series.year,
            status: series.content_status,
            first_aired: series.first_aired,
            overview: series.overview,
            network: series.network,
            runtime_minutes: series.runtime_minutes,
            poster_url: series.poster_url,
            country: series.country,
            genres: series.genres,
            aliases: series.aliases,
            seasons: series
                .seasons
                .into_iter()
                .map(|s| MetadataSeasonPayload {
                    tvdb_id: s.tvdb_id.to_string(),
                    number: s.number,
                    label: s.label,
                    episode_type: s.episode_type,
                })
                .collect(),
            episodes: if include_episodes {
                series
                    .episodes
                    .into_iter()
                    .map(|e| MetadataEpisodePayload {
                        tvdb_id: e.tvdb_id.to_string(),
                        episode_number: e.episode_number,
                        season_number: e.season_number,
                        name: e.name,
                        aired: e.aired,
                        runtime_minutes: e.runtime_minutes,
                        is_filler: e.is_filler,
                    })
                    .collect()
            } else {
                vec![]
            },
        })
    }

    // ── Calendar ──────────────────────────────────────────────────────

    async fn calendar_episodes(
        &self,
        ctx: &Context<'_>,
        start_date: String,
        end_date: String,
    ) -> GqlResult<Vec<CalendarEpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_calendar_episodes(&actor, &start_date, &end_date)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_calendar_episode).collect())
    }

    // ── Notifications ────────────────────────────────────────────────────

    async fn notification_channels(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationChannelPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let channels = app
            .list_notification_channels(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(channels
            .into_iter()
            .map(crate::mappers::from_notification_channel)
            .collect())
    }

    async fn notification_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subs = app
            .list_notification_subscriptions(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(subs
            .into_iter()
            .map(crate::mappers::from_notification_subscription)
            .collect())
    }

    async fn notification_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_notification_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|pt| {
                let name = app
                    .notification_provider_name(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = app.notification_provider_config_fields(&pt);
                from_provider_type(pt, name, fields, None)
            })
            .collect())
    }

    async fn notification_event_types(&self, ctx: &Context<'_>) -> GqlResult<Vec<String>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        Ok(scryer_domain::NotificationEventType::all()
            .iter()
            .map(|e| e.as_str().to_string())
            .collect())
    }

    // ── Service Logs ────────────────────────────────────────────────────

    async fn setup_status(&self, ctx: &Context<'_>) -> GqlResult<SetupStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;

        let setup_complete = match db
            .get_setting_with_defaults("system", "setup.complete", None)
            .await
        {
            Ok(Some(record)) => {
                record.value_json.as_deref().map(|v| v.trim_matches('"')) == Some("true")
            }
            _ => false,
        };

        let has_download_clients = !app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        let has_indexers = !app
            .list_indexer_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        Ok(SetupStatusPayload {
            setup_complete,
            has_download_clients,
            has_indexers,
        })
    }

    async fn browse_path(
        &self,
        ctx: &Context<'_>,
        #[graphql(default_with = "String::from(\"/\")")] path: String,
    ) -> GqlResult<Vec<DirectoryEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let target = std::path::Path::new(&path);
        if !target.is_absolute() {
            return Err(Error::new("path must be absolute"));
        }
        let read_dir = std::fs::read_dir(target)
            .map_err(|e| Error::new(format!("cannot read directory: {e}")))?;
        let mut entries: Vec<DirectoryEntryPayload> = Vec::new();
        for entry in read_dir.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let full_path = entry.path().to_string_lossy().into_owned();
            entries.push(DirectoryEntryPayload {
                name,
                path: full_path,
            });
        }
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(entries)
    }

    async fn service_logs(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 250)] limit: i32,
    ) -> GqlResult<ServiceLogsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let safe_limit = (limit.clamp(1, 2000)) as usize;
        let lines = match ctx.data_opt::<crate::context::LogBuffer>() {
            Some(buf) => buf.snapshot(safe_limit),
            None => vec![],
        };
        let count = lines.len() as i32;
        Ok(ServiceLogsPayload {
            generated_at: Utc::now().to_rfc3339(),
            lines,
            count,
        })
    }

    /// List downloaded subtitles for a title.
    async fn subtitle_downloads(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<Vec<SubtitleDownloadPayload>> {
        let _actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;
        let downloads =
            scryer_infrastructure::queries::subtitle::list_subtitle_downloads_for_title(
                db.pool(),
                &title_id,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(downloads
            .into_iter()
            .map(|d| SubtitleDownloadPayload {
                id: d.id,
                media_file_id: d.media_file_id,
                title_id: d.title_id,
                episode_id: d.episode_id,
                language: d.language,
                provider: d.provider,
                file_path: d.file_path,
                score: d.score,
                hearing_impaired: d.hearing_impaired,
                forced: d.forced,
                ai_translated: d.ai_translated,
                machine_translated: d.machine_translated,
                uploader: d.uploader,
                release_info: d.release_info,
                synced: d.synced,
                downloaded_at: d.downloaded_at,
            })
            .collect())
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
