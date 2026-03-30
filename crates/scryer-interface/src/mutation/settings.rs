use async_graphql::{Context, Error, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{
    AcquisitionSettings as AppAcquisitionSettings, QUALITY_PROFILE_CATALOG_KEY,
    QUALITY_PROFILE_ID_KEY, QUALITY_PROFILE_INHERIT_VALUE,
    UpdateSubtitleSettings as AppUpdateSubtitleSettings,
};
use scryer_domain::Entitlement;
use serde_json::json;

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{from_tvdb_scan_operation, from_user};
use crate::settings_graph::{
    load_download_client_routing, load_indexer_routing, load_library_paths_payload,
    load_media_settings_payload, load_quality_profile_settings_payload,
    load_service_settings_payload, persist_library_paths, persist_media_settings,
    persist_quality_profile_catalog, persist_service_settings, quality_profile_from_input,
    serialize_download_client_routing, serialize_indexer_routing,
};
use crate::types::*;

#[derive(Default)]
pub(crate) struct SettingsMutations;

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

async fn record_settings_saved(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    message: String,
    changed_keys: Vec<String>,
) {
    let _ = app
        .services
        .record_activity_event(
            Some(actor.id.clone()),
            None,
            None,
            scryer_application::ActivityKind::SettingSaved,
            message,
            scryer_application::ActivitySeverity::Success,
            vec![
                scryer_application::ActivityChannel::Toast,
                scryer_application::ActivityChannel::WebUi,
            ],
        )
        .await;

    let _ = app.services.settings_changed_broadcast.send(changed_keys);
}

#[Object]
impl SettingsMutations {
    async fn update_subtitle_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateSubtitleSettingsInput,
    ) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_subtitle_settings(
                &actor,
                AppUpdateSubtitleSettings {
                    enabled: input.enabled,
                    open_subtitles_api_key: input.open_subtitles_api_key,
                    open_subtitles_username: input.open_subtitles_username,
                    open_subtitles_password: input.open_subtitles_password,
                    languages: input
                        .languages
                        .into_iter()
                        .map(|language| {
                            scryer_application::subtitles::wanted::SubtitleLanguagePref {
                                code: language.code,
                                hearing_impaired: language.hearing_impaired.unwrap_or(false),
                                forced: language.forced.unwrap_or(false),
                            }
                        })
                        .collect(),
                    auto_download_on_import: input.auto_download_on_import,
                    minimum_score_series: input.minimum_score_series,
                    minimum_score_movie: input.minimum_score_movie,
                    search_interval_hours: input.search_interval_hours,
                    include_ai_translated: input.include_ai_translated,
                    include_machine_translated: input.include_machine_translated,
                    sync_enabled: input.sync_enabled,
                    sync_threshold_series: input.sync_threshold_series,
                    sync_threshold_movie: input.sync_threshold_movie,
                    sync_max_offset_seconds: input.sync_max_offset_seconds,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_subtitle_settings(settings))
    }

    async fn update_acquisition_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateAcquisitionSettingsInput,
    ) -> GqlResult<AcquisitionSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_acquisition_settings(
                &actor,
                AppAcquisitionSettings {
                    enabled: input.enabled,
                    upgrade_cooldown_hours: input.upgrade_cooldown_hours,
                    same_tier_min_delta: input.same_tier_min_delta,
                    cross_tier_min_delta: input.cross_tier_min_delta,
                    forced_upgrade_delta_bypass: input.forced_upgrade_delta_bypass,
                    poll_interval_seconds: input.poll_interval_seconds,
                    sync_interval_seconds: input.sync_interval_seconds,
                    batch_size: input.batch_size,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_acquisition_settings(settings))
    }

    async fn upsert_delay_profile(
        &self,
        ctx: &Context<'_>,
        input: DelayProfileInput,
    ) -> GqlResult<DelayProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let profile = app
            .upsert_delay_profile(
                &actor,
                scryer_application::DelayProfile {
                    id: input.id,
                    name: input.name,
                    usenet_delay_minutes: input.usenet_delay_minutes as i64,
                    torrent_delay_minutes: input.torrent_delay_minutes as i64,
                    preferred_protocol: input.preferred_protocol.into_application(),
                    min_age_minutes: input.min_age_minutes as i64,
                    bypass_score_threshold: input.bypass_score_threshold,
                    applies_to_facets: input
                        .applies_to_facets
                        .into_iter()
                        .map(|facet| facet.into_domain().as_str().to_string())
                        .collect(),
                    tags: input.tags,
                    priority: input.priority,
                    enabled: input.enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_delay_profile(profile))
    }

    async fn delete_delay_profile(
        &self,
        ctx: &Context<'_>,
        input: DeleteDelayProfileInput,
    ) -> GqlResult<DelayProfileDeletionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = app
            .delete_delay_profile(&actor, &input.id)
            .await
            .map_err(to_gql_error)?;
        Ok(DelayProfileDeletionPayload { id })
    }

    async fn update_media_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateMediaSettingsInput,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        let scope = input.scope;
        let changed_keys =
            persist_media_settings(&db, scope, input, Some(actor.id.clone())).await?;

        record_settings_saved(
            &app,
            &actor,
            format!("media settings updated for {}", scope.as_scope_id()),
            changed_keys,
        )
        .await;

        load_media_settings_payload(&app, &db, scope).await
    }

    async fn update_library_paths(
        &self,
        ctx: &Context<'_>,
        input: UpdateLibraryPathsInput,
    ) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        let changed_keys = persist_library_paths(&db, &input, Some(actor.id.clone())).await?;

        record_settings_saved(
            &app,
            &actor,
            "library paths updated".to_string(),
            changed_keys,
        )
        .await;

        load_library_paths_payload(&db).await
    }

    async fn update_service_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateServiceSettingsInput,
    ) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        let changed_keys = persist_service_settings(&db, &input, Some(actor.id.clone())).await?;

        record_settings_saved(
            &app,
            &actor,
            "service settings updated".to_string(),
            changed_keys,
        )
        .await;

        load_service_settings_payload(&db).await
    }

    async fn save_quality_profile_settings(
        &self,
        ctx: &Context<'_>,
        input: SaveQualityProfileSettingsInput,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let existing_profiles = app
            .services
            .quality_profiles
            .list_quality_profiles("system", None)
            .await
            .map_err(to_gql_error)?;
        let existing_by_id = existing_profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile))
            .collect::<std::collections::HashMap<_, _>>();

        let profiles = input
            .profiles
            .into_iter()
            .map(|profile| {
                let existing = existing_by_id.get(profile.id.as_str()).copied();
                quality_profile_from_input(profile, existing)
            })
            .collect::<GqlResult<Vec<_>>>()?;

        if !profiles.is_empty() {
            persist_quality_profile_catalog(
                &db,
                &profiles,
                Some(actor.id.clone()),
                input.replace_existing,
            )
            .await?;
        }

        let mut changed_keys = Vec::new();
        if !profiles.is_empty() {
            changed_keys.push(QUALITY_PROFILE_CATALOG_KEY.to_string());
        }

        let current = load_quality_profile_settings_payload(&app, &db).await?;
        let valid_profile_ids: std::collections::HashSet<&str> = current
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect();

        if let Some(global_profile_id) = input.global_profile_id {
            let global_profile_id = global_profile_id.trim();
            if !global_profile_id.is_empty() {
                if !valid_profile_ids.contains(global_profile_id) {
                    return Err(Error::new(format!(
                        "unknown quality profile '{global_profile_id}'"
                    )));
                }
                db.upsert_setting_value(
                    "system",
                    QUALITY_PROFILE_ID_KEY,
                    None,
                    serde_json::to_string(global_profile_id)
                        .map_err(|error| Error::new(error.to_string()))?,
                    "typed_graphql",
                    Some(actor.id.clone()),
                )
                .await
                .map_err(to_gql_error)?;
                changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
            }
        }

        for selection in input.category_selections {
            let scope_id = selection.scope.as_scope_id().to_string();
            let value = if selection.inherit_global {
                QUALITY_PROFILE_INHERIT_VALUE.to_string()
            } else {
                let profile_id = selection
                    .profile_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        Error::new("profile_id is required when inheritGlobal is false")
                    })?;
                if !valid_profile_ids.contains(profile_id) {
                    return Err(Error::new(format!(
                        "unknown quality profile '{profile_id}'"
                    )));
                }
                profile_id.to_string()
            };

            db.upsert_setting_value(
                "system",
                QUALITY_PROFILE_ID_KEY,
                Some(scope_id),
                serde_json::to_string(&value).map_err(|error| Error::new(error.to_string()))?,
                "typed_graphql",
                Some(actor.id.clone()),
            )
            .await
            .map_err(to_gql_error)?;
            if !changed_keys.iter().any(|key| key == QUALITY_PROFILE_ID_KEY) {
                changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
            }
        }

        let _ = app
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                scryer_application::ActivityKind::SettingSaved,
                "quality profile settings updated".to_string(),
                scryer_application::ActivitySeverity::Success,
                vec![
                    scryer_application::ActivityChannel::Toast,
                    scryer_application::ActivityChannel::WebUi,
                ],
            )
            .await;
        if !changed_keys.is_empty() {
            let _ = app.services.settings_changed_broadcast.send(changed_keys);
        }

        load_quality_profile_settings_payload(&app, &db).await
    }

    async fn update_download_client_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientRoutingInput,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let scope = input.scope;
        let payload = serialize_download_client_routing(input.entries)?;
        db.upsert_setting_value(
            "system",
            "download_client.routing",
            Some(scope.as_scope_id().to_string()),
            payload,
            "typed_graphql",
            Some(actor.id.clone()),
        )
        .await
        .map_err(to_gql_error)?;

        let _ = app
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                scryer_application::ActivityKind::SettingSaved,
                format!(
                    "download client routing updated for {}",
                    scope.as_scope_id()
                ),
                scryer_application::ActivitySeverity::Success,
                vec![
                    scryer_application::ActivityChannel::Toast,
                    scryer_application::ActivityChannel::WebUi,
                ],
            )
            .await;
        let _ = app
            .services
            .settings_changed_broadcast
            .send(vec!["download_client.routing".to_string()]);

        load_download_client_routing(&db, scope).await
    }

    async fn update_indexer_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerRoutingInput,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let scope = input.scope;
        let payload = serialize_indexer_routing(input.entries)?;
        db.upsert_setting_value(
            "system",
            "indexer.routing",
            Some(scope.as_scope_id().to_string()),
            payload,
            "typed_graphql",
            Some(actor.id.clone()),
        )
        .await
        .map_err(to_gql_error)?;

        let _ = app
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                scryer_application::ActivityKind::SettingSaved,
                format!("indexer routing updated for {}", scope.as_scope_id()),
                scryer_application::ActivitySeverity::Success,
                vec![
                    scryer_application::ActivityChannel::Toast,
                    scryer_application::ActivityChannel::WebUi,
                ],
            )
            .await;
        let _ = app
            .services
            .settings_changed_broadcast
            .send(vec!["indexer.routing".to_string()]);

        load_indexer_routing(&db, scope).await
    }

    async fn delete_quality_profile(
        &self,
        ctx: &Context<'_>,
        input: DeleteQualityProfileInput,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let profile_id = input.profile_id.trim().to_string();
        if profile_id.is_empty() {
            return Err(Error::new("profile_id is required"));
        }

        // Check if this profile is the global default.
        let global_setting = db
            .get_setting_with_defaults(
                "system".to_string(),
                QUALITY_PROFILE_ID_KEY.to_string(),
                None::<String>,
            )
            .await
            .map_err(to_gql_error)?;
        if let Some(record) = &global_setting {
            let effective: String =
                serde_json::from_str(&record.effective_value_json).unwrap_or_default();
            if effective.trim() == profile_id {
                return Err(Error::new(
                    "cannot delete this profile because it is set as the global default quality profile",
                ));
            }
        }

        // Check if this profile is a category override.
        for scope_id in &["movie", "series", "anime"] {
            let category_setting = db
                .get_setting_with_defaults(
                    "system".to_string(),
                    QUALITY_PROFILE_ID_KEY.to_string(),
                    Some(scope_id.to_string()),
                )
                .await
                .map_err(to_gql_error)?;
            if let Some(record) = &category_setting {
                if record.value_json.is_none() {
                    continue;
                }
                let value: String =
                    serde_json::from_str(record.value_json.as_deref().unwrap_or("\"\""))
                        .unwrap_or_default();
                if value.trim() == profile_id {
                    return Err(Error::new(format!(
                        "cannot delete this profile because it is set as the quality profile override for {scope_id}",
                    )));
                }
            }
        }

        // Delete the profile from the DB.
        db.delete_quality_profile(&profile_id)
            .await
            .map_err(to_gql_error)?;

        let remaining_profiles = app
            .services
            .quality_profiles
            .list_quality_profiles("system", None)
            .await
            .map_err(to_gql_error)?;
        persist_quality_profile_catalog(&db, &remaining_profiles, Some(actor.id.clone()), true)
            .await?;

        let _ = app
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                scryer_application::ActivityKind::SettingSaved,
                format!("quality profile '{profile_id}' deleted"),
                scryer_application::ActivitySeverity::Success,
                vec![
                    scryer_application::ActivityChannel::Toast,
                    scryer_application::ActivityChannel::WebUi,
                ],
            )
            .await;

        let _ = app.services.settings_changed_broadcast.send(vec![
            "quality.profiles".to_string(),
            "quality.profile_id".to_string(),
        ]);

        load_quality_profile_settings_payload(&app, &db).await
    }

    async fn queue_tvdb_movies_scan(
        &self,
        ctx: &Context<'_>,
        input: QueueTvdbMoviesScanInput,
    ) -> GqlResult<TvdbScanOperationPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let limit = if input.limit > 0 {
            input.limit
        } else {
            return Err(Error::new(
                "limit is required and must be greater than zero",
            ));
        };

        let source = input.source.trim();
        if source.is_empty() {
            return Err(Error::new("source is required"));
        }

        let progress_json = json!({
            "type": "tvdb_movies_scan",
            "limit": limit,
            "source": source,
        })
        .to_string();

        let operation = db
            .create_workflow_operation(
                "tvdb_movies_scan",
                "queued",
                Some(actor.id),
                Some(progress_json),
                None,
                None,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_tvdb_scan_operation(
            operation,
            limit,
            source.to_string(),
        ))
    }

    async fn login(&self, ctx: &Context<'_>, input: LoginInput) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .authenticate_credentials(&input.username, &input.password)
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

    /// Issue a JWT for the default admin user without credentials.
    /// Retained for compatibility when authentication is disabled.
    async fn dev_auto_login(&self, ctx: &Context<'_>) -> GqlResult<LoginPayload> {
        let api_ctx = ctx.data_unchecked::<crate::context::ApiContext>();
        if api_ctx.auth_enabled {
            return Err(Error::new("authentication is enabled"));
        }
        let app = &api_ctx.app;
        let user = app
            .find_or_create_default_user()
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

    /// Mark the setup wizard as complete.
    async fn complete_setup(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;
        db.upsert_setting_value(
            "system",
            "setup.complete",
            None,
            "true",
            "setup-wizard",
            Some(actor.id),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }
}
