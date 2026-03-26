use async_graphql::{ComplexObject, Context, Error, Object, Result as GqlResult};

use chrono::Utc;
use scryer_application::TitleHistoryFilter;
use scryer_domain::{PolicyInput, TitleHistoryEventType};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{
    from_activity_event, from_backup_info, from_calendar_episode, from_collection, from_disk_space,
    from_download_client_config, from_download_queue_item, from_episode, from_health_check_result,
    from_indexer_config, from_media_rename_plan, from_pending_release, from_provider_type,
    from_release_decision, from_system_health, from_title, from_title_history_page,
    from_title_history_record, from_title_media_file, from_title_release_blocklist_entry,
    from_user, from_wanted_item,
};
use crate::settings_graph::{
    load_download_client_routing, load_indexer_routing, load_library_paths_payload,
    load_media_settings_payload, load_quality_profile_settings_payload,
    load_service_settings_payload,
};
use crate::types::*;

fn from_subtitle_settings(
    settings: scryer_application::SubtitleSettings,
) -> SubtitleSettingsPayload {
    SubtitleSettingsPayload {
        enabled: settings.enabled,
        has_open_subtitles_api_key: settings.open_subtitles_api_key.is_some(),
        open_subtitles_username: settings.open_subtitles_username.unwrap_or_default(),
        has_open_subtitles_password: settings.open_subtitles_password.is_some(),
        languages: settings
            .languages
            .into_iter()
            .map(|language| SubtitleLanguagePreferencePayload {
                code: language.code,
                hearing_impaired: language.hearing_impaired,
                forced: language.forced,
            })
            .collect(),
        auto_download_on_import: settings.auto_download_on_import,
        minimum_score_series: settings.minimum_score_series,
        minimum_score_movie: settings.minimum_score_movie,
        search_interval_hours: settings.search_interval_hours,
        include_ai_translated: settings.include_ai_translated,
        include_machine_translated: settings.include_machine_translated,
        sync_enabled: settings.sync_enabled,
        sync_threshold_series: settings.sync_threshold_series,
        sync_threshold_movie: settings.sync_threshold_movie,
        sync_max_offset_seconds: settings.sync_max_offset_seconds,
    }
}

fn from_acquisition_settings(
    settings: scryer_application::AcquisitionSettings,
) -> AcquisitionSettingsPayload {
    AcquisitionSettingsPayload {
        enabled: settings.enabled,
        upgrade_cooldown_hours: settings.upgrade_cooldown_hours,
        same_tier_min_delta: settings.same_tier_min_delta,
        cross_tier_min_delta: settings.cross_tier_min_delta,
        forced_upgrade_delta_bypass: settings.forced_upgrade_delta_bypass,
        poll_interval_seconds: settings.poll_interval_seconds,
        sync_interval_seconds: settings.sync_interval_seconds,
        batch_size: settings.batch_size,
    }
}

fn from_delay_profile(profile: scryer_application::DelayProfile) -> DelayProfilePayload {
    DelayProfilePayload {
        id: profile.id,
        name: profile.name,
        usenet_delay_minutes: profile.usenet_delay_minutes as i32,
        torrent_delay_minutes: profile.torrent_delay_minutes as i32,
        preferred_protocol: DelayProfilePreferredProtocolValue::from_application(
            profile.preferred_protocol,
        ),
        min_age_minutes: profile.min_age_minutes as i32,
        bypass_score_threshold: profile.bypass_score_threshold,
        applies_to_facets: profile
            .applies_to_facets
            .into_iter()
            .filter_map(|facet| MediaFacetValue::parse(&facet))
            .collect(),
        tags: profile.tags,
        priority: profile.priority,
        enabled: profile.enabled,
    }
}

#[derive(Copy, Clone)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        query: Option<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parsed_facet = facet.map(MediaFacetValue::into_domain);
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

    async fn media_rename_preview(
        &self,
        ctx: &Context<'_>,
        input: MediaRenamePreviewInput,
    ) -> GqlResult<MediaRenamePlanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let _ = input.dry_run;
        let facet = input.facet.into_domain();
        let plan = if let Some(title_id) = input.title_id {
            app.preview_rename_for_title(&actor, &title_id, facet)
                .await
                .map_err(to_gql_error)?
        } else {
            app.preview_rename_for_facet(&actor, facet)
                .await
                .map_err(to_gql_error)?
        };

        Ok(from_media_rename_plan(plan))
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

    async fn wanted_item(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(item)
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
                    facet: input.facet.into_domain(),
                    has_existing_file: input.has_existing_file,
                    candidate_quality: input.candidate_quality,
                    requested_mode: scryer_domain::RequestedMode::parse(&input.requested_mode)
                        .ok_or_else(|| Error::new("invalid requestedMode for policyPreview"))?,
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

    async fn search_releases(
        &self,
        ctx: &Context<'_>,
        input: SearchReleasesInput,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let SearchReleasesInput {
            query,
            title_id,
            season,
            episode,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            absolute_episode,
            limit,
        } = input;

        let safe_limit = limit.unwrap_or(50).clamp(1, 200) as usize;
        let results = match (query, title_id, season, episode) {
            (Some(query), None, Some(season), Some(episode)) => app
                .search_indexers_episode(
                    &actor,
                    query,
                    season,
                    episode,
                    imdb_id,
                    tvdb_id,
                    anidb_id,
                    category,
                    absolute_episode.map(|value| value as u32),
                )
                .await
                .map_err(to_gql_error)?,
            (Some(query), None, None, None) => app
                .search_indexers(&actor, query, imdb_id, tvdb_id, anidb_id, category)
                .await
                .map_err(to_gql_error)?,
            (None, Some(title_id), Some(season), Some(episode)) => app
                .search_indexers_for_episode(&actor, title_id, season, episode)
                .await
                .map_err(to_gql_error)?,
            (None, Some(title_id), None, None) => app
                .search_indexers_for_title(&actor, title_id)
                .await
                .map_err(to_gql_error)?,
            (Some(_), Some(_), _, _) => {
                return Err(Error::new(
                    "searchReleases accepts either query or titleId, not both",
                ));
            }
            (_, _, Some(_), None) | (_, _, None, Some(_)) => {
                return Err(Error::new(
                    "episode searches require both season and episode",
                ));
            }
            _ => {
                return Err(Error::new(
                    "searchReleases requires either query or titleId",
                ));
            }
        };

        Ok(results
            .into_iter()
            .take(safe_limit)
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

    async fn subtitle_settings(&self, ctx: &Context<'_>) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_subtitle_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_subtitle_settings(settings))
    }

    async fn acquisition_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AcquisitionSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_acquisition_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_acquisition_settings(settings))
    }

    async fn delay_profiles(&self, ctx: &Context<'_>) -> GqlResult<Vec<DelayProfilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let profiles = app.get_delay_profiles(&actor).await.map_err(to_gql_error)?;
        Ok(profiles.into_iter().map(from_delay_profile).collect())
    }

    async fn media_settings(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_media_settings_payload(&app, &db, scope).await
    }

    async fn library_paths(&self, ctx: &Context<'_>) -> GqlResult<LibraryPathsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_library_paths_payload(&db).await
    }

    async fn service_settings(&self, ctx: &Context<'_>) -> GqlResult<ServiceSettingsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_service_settings_payload(&db).await
    }

    async fn quality_profile_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_quality_profile_settings_payload(&app, &db).await
    }

    async fn download_client_routing(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_download_client_routing(&db, scope).await
    }

    async fn indexer_routing(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        load_indexer_routing(&db, scope).await
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
        facet: MediaFacetValue,
    ) -> GqlResult<Vec<RootFolderPayload>> {
        let app = app_from_ctx(ctx)?;
        let media_facet = facet.into_domain();
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

    async fn recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<RecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let all = app
            .list_recycled_items(&actor)
            .await
            .map_err(to_gql_error)?;
        let total_count = all.len() as i32;
        let limit = limit.clamp(1, 500) as usize;
        let offset = offset.max(0) as usize;
        let items = all
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|entry| {
                let file_name = std::path::Path::new(&entry.manifest.original_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                RecycledItemPayload {
                    id: entry.entry_id,
                    original_path: entry.manifest.original_path,
                    file_name,
                    size_bytes: entry.manifest.size_bytes as i64,
                    title_id: entry.manifest.title_id,
                    reason: entry.manifest.reason,
                    recycled_at: entry.manifest.recycled_at,
                    media_root: entry.media_root,
                }
            })
            .collect();
        Ok(RecycledItemsPayload { items, total_count })
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
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let releases = app.list_pending_releases().await.map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
    }

    async fn pending_release(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let release = app
            .get_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_pending_release);
        Ok(release)
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
        status: Option<WantedStatusValue>,
        media_type: Option<WantedMediaTypeValue>,
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
                status.map(WantedStatusValue::as_str),
                media_type.map(WantedMediaTypeValue::as_str),
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

#[ComplexObject]
impl TitlePayload {
    async fn collections(&self, ctx: &Context<'_>) -> GqlResult<Vec<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collections = app
            .list_collections(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(collections.into_iter().map(from_collection).collect())
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let files = app
            .services
            .media_files
            .list_media_files_for_title(&self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(files.into_iter().map(from_title_media_file).collect())
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<String>,
    ) -> GqlResult<Vec<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let (items, _) = app
            .list_wanted_items(status.as_deref(), None, Some(&self.id), 500, 0)
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_wanted_item).collect())
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(None, Some(&self.id), limit)
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    async fn download_queue_items(
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
        Ok(items
            .into_iter()
            .filter(|item| item.title_id.as_deref() == Some(self.id.as_str()))
            .map(from_download_queue_item)
            .collect())
    }
}

#[ComplexObject]
impl CollectionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episodes(&self, ctx: &Context<'_>) -> GqlResult<Vec<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_episodes(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_episode).collect())
    }
}

#[ComplexObject]
impl EpisodePayload {
    async fn parent_title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let wanted_item = app
            .services
            .wanted_items
            .get_wanted_item_for_title(&self.title_id, Some(&self.id))
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(wanted_item)
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let files = app
            .services
            .media_files
            .list_media_files_for_title(&self.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(files
            .into_iter()
            .filter(|file| file.episode_id.as_deref() == Some(self.id.as_str()))
            .map(from_title_media_file)
            .collect())
    }
}

#[ComplexObject]
impl TitleMediaFilePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }
}

#[ComplexObject]
impl WantedItemPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(Some(&self.id), None, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    async fn pending_releases(&self, ctx: &Context<'_>) -> GqlResult<Vec<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let releases = app
            .list_pending_releases_for_wanted_item(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
    }
}

#[ComplexObject]
impl ReleaseDecisionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, &self.wanted_item_id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(item)
    }
}

#[ComplexObject]
impl DownloadQueueItemPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let Some(title_id) = self.title_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let title = app
            .services
            .titles
            .get_by_id(title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }
}

#[ComplexObject]
impl PendingReleasePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let title = app
            .services
            .titles
            .get_by_id(&self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let wanted_item = app
            .services
            .wanted_items
            .get_wanted_item_by_id(&self.wanted_item_id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(wanted_item)
    }
}
