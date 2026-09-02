mod mutation;

use async_graphql::{Context, ID, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_interface_core::{
    AuthRuntimeStateSnapshot, actor_from_ctx, api_key_management_actor_from_ctx, app_from_ctx,
    auth_runtime_from_ctx, default_persist_session_from_ctx, to_gql_error,
};
use scryer_interface_media::mappers::{
    from_download_client_config_with_fields, from_download_client_routing_entry,
    from_indexer_config_with_fields, from_indexer_proxy_config, from_indexer_routing_entry,
    from_jellyfin_server_user, from_library_paths_settings, from_media_server_connection,
    from_media_server_user_group, from_media_settings, from_quality_profile_settings,
    from_seeding_profile, from_service_settings, from_subtitle_provider_config,
    from_user_with_auth_factor_status,
};
use scryer_interface_media::types::*;

pub use mutation::SettingsMutations;

#[derive(Default)]
pub struct SettingsQueries;

fn parse_required_datetime(value: &str, field: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|error| panic!("invalid {field} timestamp: {error}"))
}

fn parse_optional_datetime(value: Option<String>, field: &str) -> Option<DateTime<Utc>> {
    value.map(|value| parse_required_datetime(&value, field))
}

fn from_subtitle_settings(
    settings: scryer_application::SubtitleSettings,
) -> SubtitleSettingsPayload {
    SubtitleSettingsPayload {
        enabled: settings.enabled,
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

fn from_recycle_bin_settings(
    settings: scryer_application::RecycleBinSettings,
) -> RecycleBinSettingsPayload {
    RecycleBinSettingsPayload {
        enabled: settings.enabled,
    }
}

fn from_plugin_auto_update_settings(
    settings: scryer_application::PluginAutoUpdateSettings,
) -> PluginAutoUpdateSettingsPayload {
    PluginAutoUpdateSettingsPayload {
        enabled: settings.enabled,
    }
}

fn from_oauth_connected_app(
    app: scryer_application::OAuthConnectedAppSummary,
) -> OAuthConnectedAppPayload {
    OAuthConnectedAppPayload {
        grant_id: app.grant_id.into(),
        client_id: app.client_id,
        client_name: app.client_name,
        authorized_at: app.authorized_at,
        last_used_at: app.last_used_at,
    }
}

fn from_api_key(key: scryer_application::ApiKeySummary, owner_username: &str) -> ApiKeyPayload {
    ApiKeyPayload {
        id: key.id.into(),
        actor: format!("api ({}) obo {owner_username}", key.label),
        label: key.label,
        expires_at: key.expires_at,
        revoked_at: key.revoked_at,
        last_used_at: key.last_used_at,
        created_at: key.created_at,
        provisioning_source: key.provisioning_source.as_str().to_owned(),
    }
}

fn from_oauth_client_registration(
    client: scryer_application::OAuthClientInfo,
) -> OAuthClientRegistrationPayload {
    OAuthClientRegistrationPayload {
        client_id: client.client_id,
        display_name: client.name,
        redirect_uris: client.redirect_uris,
        enabled: client.enabled,
        source: match client.source {
            scryer_application::OAuthClientSource::Managed => OAuthClientSourceValue::Managed,
            scryer_application::OAuthClientSource::Custom => OAuthClientSourceValue::Custom,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_auth_runtime_settings_maps_clean_connections() {
        let payload = from_external_auth_runtime_settings(
            scryer_application::ExternalAuthRuntimeSettings {
                login_providers: vec![scryer_domain::ExternalAccountProvider::Jellyfin],
                linking_providers: vec![scryer_domain::ExternalAccountProvider::Plex],
                connections: vec![
                    scryer_application::ExternalAuthRuntimeConnection {
                        id: "jellyfin-main".to_string(),
                        provider: scryer_domain::ExternalAccountProvider::Jellyfin,
                        display_name: "Main Jellyfin".to_string(),
                        login_enabled: true,
                        linking_enabled: false,
                        emby_connect_enabled: false,
                    },
                    scryer_application::ExternalAuthRuntimeConnection {
                        id: "plex-main".to_string(),
                        provider: scryer_domain::ExternalAccountProvider::Plex,
                        display_name: "Main Plex".to_string(),
                        login_enabled: false,
                        linking_enabled: true,
                        emby_connect_enabled: false,
                    },
                ],
            },
            true,
        );

        assert!(matches!(
            payload.login_providers.as_slice(),
            [ExternalAccountProviderValue::Jellyfin]
        ));
        assert!(matches!(
            payload.linking_providers.as_slice(),
            [ExternalAccountProviderValue::Plex]
        ));
        assert_eq!(payload.connections.len(), 2);
        assert_eq!(payload.connections[0].id, "jellyfin-main");
        assert!(matches!(
            payload.connections[0].provider,
            ExternalAccountProviderValue::Jellyfin
        ));
        assert_eq!(payload.connections[0].display_name, "Main Jellyfin");
        assert!(payload.connections[0].login_enabled);
        assert!(!payload.connections[0].linking_enabled);
    }

    #[test]
    fn external_auth_runtime_settings_hides_login_when_form_login_disabled() {
        let payload = from_external_auth_runtime_settings(
            scryer_application::ExternalAuthRuntimeSettings {
                login_providers: vec![scryer_domain::ExternalAccountProvider::Jellyfin],
                linking_providers: vec![scryer_domain::ExternalAccountProvider::Plex],
                connections: vec![scryer_application::ExternalAuthRuntimeConnection {
                    id: "jellyfin-main".to_string(),
                    provider: scryer_domain::ExternalAccountProvider::Jellyfin,
                    display_name: "Main Jellyfin".to_string(),
                    login_enabled: true,
                    linking_enabled: true,
                    emby_connect_enabled: false,
                }],
            },
            false,
        );

        assert!(payload.login_providers.is_empty());
        assert!(matches!(
            payload.linking_providers.as_slice(),
            [ExternalAccountProviderValue::Plex]
        ));
        assert_eq!(payload.connections.len(), 1);
        assert!(!payload.connections[0].login_enabled);
        assert!(payload.connections[0].linking_enabled);
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
        long_tail_backfill_max_scopes_per_cycle: settings.long_tail_backfill_max_scopes_per_cycle,
        long_tail_reconverge_days: settings.long_tail_reconverge_days,
    }
}

fn from_general_settings(settings: scryer_application::GeneralSettings) -> GeneralSettingsPayload {
    GeneralSettingsPayload {
        keep_history_forever: settings.keep_history_forever,
        history_retention_days: settings.history_retention_days,
        image_cache_max_size_mb: settings.image_cache_max_size_mb,
        effective_image_cache_max_size_bytes: Long::from_u64_saturating(
            settings.effective_image_cache_max_size_bytes,
        ),
        effective_image_cache_max_size_mb: settings.effective_image_cache_max_size_mb,
        image_cache_max_size_env_override_active: settings.image_cache_max_size_env_override_active,
        plugin_http_ca_bundle_pem: settings.plugin_http_ca_bundle_pem,
        plugin_http_trusted_certificates: settings
            .plugin_http_trusted_certificates
            .into_iter()
            .map(|certificate| PluginHttpTrustedCertificatePayload {
                fingerprint_sha256: certificate.fingerprint_sha256,
                pem: certificate.pem,
            })
            .collect(),
    }
}

fn from_auto_backup_settings(
    settings: scryer_application::AutoBackupSettings,
) -> AutoBackupSettingsPayload {
    AutoBackupSettingsPayload {
        enabled: settings.enabled,
        daily_time_local: settings.daily_time_local,
        auto_backup_key_present: settings.auto_backup_key_present,
        auto_backup_disabled_missing_key_notice: settings.auto_backup_disabled_missing_key_notice,
        next_run_at: parse_optional_datetime(settings.next_run_at, "auto backup next_run_at"),
    }
}

fn from_backup_settings(settings: scryer_application::BackupSettings) -> BackupSettingsPayload {
    BackupSettingsPayload {
        custom_backup_path: settings.custom_backup_path,
        default_backup_path: settings.default_backup_path,
        effective_backup_path: settings.effective_backup_path,
    }
}

fn from_security_settings(
    settings: scryer_application::SecuritySettings,
    auth_runtime: &AuthRuntimeStateSnapshot,
) -> SecuritySettingsPayload {
    SecuritySettingsPayload {
        form_login_enabled: settings.form_login_enabled,
        password_min_length: settings.password_min_length,
        skip_login_for_local_ips: settings.skip_login_for_local_ips,
        api_keys_restrict_to_system_settings_users: settings
            .api_keys_restrict_to_system_settings_users,
        mfa_require_config_step_up: settings.mfa_require_config_step_up,
        mfa_require_password_login: settings.mfa_require_password_login,
        mfa_require_jellyfin_login: settings.totp_require_jellyfin_login,
        mfa_require_emby_login: settings.totp_require_emby_login,
        totp_require_jellyfin_login: settings.totp_require_jellyfin_login,
        totp_require_emby_login: settings.totp_require_emby_login,
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        env_override_active: auth_runtime.env_override_active,
        env_override_description: auth_runtime.env_override_description.clone(),
    }
}

fn from_ui_theme(theme: scryer_application::UiTheme) -> UiThemeValue {
    match theme {
        scryer_application::UiTheme::Light => UiThemeValue::Light,
        scryer_application::UiTheme::Dark => UiThemeValue::Dark,
        scryer_application::UiTheme::Pride => UiThemeValue::Pride,
        scryer_application::UiTheme::System => UiThemeValue::System,
    }
}

fn to_app_ui_theme(theme: UiThemeValue) -> scryer_application::UiTheme {
    match theme {
        UiThemeValue::Light => scryer_application::UiTheme::Light,
        UiThemeValue::Dark => scryer_application::UiTheme::Dark,
        UiThemeValue::Pride => scryer_application::UiTheme::Pride,
        UiThemeValue::System => scryer_application::UiTheme::System,
    }
}

fn from_ui_date_time_format(format: scryer_application::UiDateTimeFormat) -> UiDateTimeFormatValue {
    match format {
        scryer_application::UiDateTimeFormat::Locale => UiDateTimeFormatValue::Locale,
        scryer_application::UiDateTimeFormat::Iso24h => UiDateTimeFormatValue::Iso24h,
    }
}

fn to_app_ui_date_time_format(
    format: UiDateTimeFormatValue,
) -> scryer_application::UiDateTimeFormat {
    match format {
        UiDateTimeFormatValue::Locale => scryer_application::UiDateTimeFormat::Locale,
        UiDateTimeFormatValue::Iso24h => scryer_application::UiDateTimeFormat::Iso24h,
    }
}

fn from_ui_density(density: scryer_application::UiDensity) -> UiDensityValue {
    match density {
        scryer_application::UiDensity::Compact => UiDensityValue::Compact,
        scryer_application::UiDensity::Comfortable => UiDensityValue::Comfortable,
    }
}

fn to_app_ui_density(density: UiDensityValue) -> scryer_application::UiDensity {
    match density {
        UiDensityValue::Compact => scryer_application::UiDensity::Compact,
        UiDensityValue::Comfortable => scryer_application::UiDensity::Comfortable,
    }
}

fn from_ui_sidebar_mode(mode: scryer_application::UiSidebarMode) -> UiSidebarModeValue {
    match mode {
        scryer_application::UiSidebarMode::Collapsed => UiSidebarModeValue::Collapsed,
        scryer_application::UiSidebarMode::Expanded => UiSidebarModeValue::Expanded,
    }
}

fn to_app_ui_sidebar_mode(mode: UiSidebarModeValue) -> scryer_application::UiSidebarMode {
    match mode {
        UiSidebarModeValue::Collapsed => scryer_application::UiSidebarMode::Collapsed,
        UiSidebarModeValue::Expanded => scryer_application::UiSidebarMode::Expanded,
    }
}

fn from_ui_default_landing_view(
    view: scryer_application::UiDefaultLandingView,
) -> UiDefaultLandingViewValue {
    match view {
        scryer_application::UiDefaultLandingView::Movies => UiDefaultLandingViewValue::Movies,
        scryer_application::UiDefaultLandingView::Series => UiDefaultLandingViewValue::Series,
        scryer_application::UiDefaultLandingView::Anime => UiDefaultLandingViewValue::Anime,
        scryer_application::UiDefaultLandingView::Activity => UiDefaultLandingViewValue::Activity,
        scryer_application::UiDefaultLandingView::Calendar => UiDefaultLandingViewValue::Calendar,
        scryer_application::UiDefaultLandingView::Wanted => UiDefaultLandingViewValue::Wanted,
        scryer_application::UiDefaultLandingView::History => UiDefaultLandingViewValue::History,
        scryer_application::UiDefaultLandingView::Settings => UiDefaultLandingViewValue::Settings,
        scryer_application::UiDefaultLandingView::System => UiDefaultLandingViewValue::System,
    }
}

fn to_app_ui_default_landing_view(
    view: UiDefaultLandingViewValue,
) -> scryer_application::UiDefaultLandingView {
    match view {
        UiDefaultLandingViewValue::Movies => scryer_application::UiDefaultLandingView::Movies,
        UiDefaultLandingViewValue::Series => scryer_application::UiDefaultLandingView::Series,
        UiDefaultLandingViewValue::Anime => scryer_application::UiDefaultLandingView::Anime,
        UiDefaultLandingViewValue::Activity => scryer_application::UiDefaultLandingView::Activity,
        UiDefaultLandingViewValue::Calendar => scryer_application::UiDefaultLandingView::Calendar,
        UiDefaultLandingViewValue::Wanted => scryer_application::UiDefaultLandingView::Wanted,
        UiDefaultLandingViewValue::History => scryer_application::UiDefaultLandingView::History,
        UiDefaultLandingViewValue::Settings => scryer_application::UiDefaultLandingView::Settings,
        UiDefaultLandingViewValue::System => scryer_application::UiDefaultLandingView::System,
    }
}

fn from_ui_settings_facet(facet: scryer_application::UiSettingsFacet) -> UiSettingsFacetValue {
    match facet {
        scryer_application::UiSettingsFacet::Movies => UiSettingsFacetValue::Movies,
        scryer_application::UiSettingsFacet::Series => UiSettingsFacetValue::Series,
        scryer_application::UiSettingsFacet::Anime => UiSettingsFacetValue::Anime,
    }
}

fn to_app_ui_settings_facet(facet: UiSettingsFacetValue) -> scryer_application::UiSettingsFacet {
    match facet {
        UiSettingsFacetValue::Movies => scryer_application::UiSettingsFacet::Movies,
        UiSettingsFacetValue::Series => scryer_application::UiSettingsFacet::Series,
        UiSettingsFacetValue::Anime => scryer_application::UiSettingsFacet::Anime,
    }
}

fn from_ui_table_view_mode(mode: scryer_application::UiTableViewMode) -> UiTableViewModeValue {
    match mode {
        scryer_application::UiTableViewMode::Compact => UiTableViewModeValue::Compact,
        scryer_application::UiTableViewMode::PosterTable => UiTableViewModeValue::PosterTable,
    }
}

fn to_app_ui_table_view_mode(mode: UiTableViewModeValue) -> scryer_application::UiTableViewMode {
    match mode {
        UiTableViewModeValue::Compact => scryer_application::UiTableViewMode::Compact,
        UiTableViewModeValue::PosterTable => scryer_application::UiTableViewMode::PosterTable,
    }
}

pub(crate) fn from_ui_settings(settings: scryer_application::UiSettings) -> UiSettingsPayload {
    UiSettingsPayload {
        theme: from_ui_theme(settings.theme),
        date_time_format: from_ui_date_time_format(settings.date_time_format),
        highlight_color: settings.highlight_color,
        secondary_color: settings.secondary_color,
        high_contrast_mode: settings.high_contrast_mode,
        reduce_motion: settings.reduce_motion,
        hide_sponsor_button: settings.hide_sponsor_button,
        density: from_ui_density(settings.density),
        sidebar_mode: from_ui_sidebar_mode(settings.sidebar_mode),
        default_landing_view: from_ui_default_landing_view(settings.default_landing_view),
        table_columns: settings
            .table_columns
            .into_iter()
            .map(|column| UiTableColumnSettingPayload {
                facet: from_ui_settings_facet(column.facet),
                table_view_mode: from_ui_table_view_mode(column.table_view_mode),
                column_id: column.column_id,
                column_order: column.column_order,
                visible: column.visible,
            })
            .collect(),
    }
}

pub(crate) fn ui_settings_update_from_input(
    input: SetMyUiSettingsInput,
    current_date_time_format: scryer_application::UiDateTimeFormat,
) -> scryer_application::UiSettingsUpdate {
    scryer_application::UiSettingsUpdate {
        theme: to_app_ui_theme(input.theme),
        date_time_format: input
            .date_time_format
            .map(to_app_ui_date_time_format)
            .unwrap_or(current_date_time_format),
        highlight_color: input.highlight_color,
        secondary_color: input.secondary_color,
        high_contrast_mode: input.high_contrast_mode,
        reduce_motion: input.reduce_motion,
        hide_sponsor_button: input.hide_sponsor_button,
        density: to_app_ui_density(input.density),
        sidebar_mode: to_app_ui_sidebar_mode(input.sidebar_mode),
        default_landing_view: to_app_ui_default_landing_view(input.default_landing_view),
        table_columns: input
            .table_columns
            .into_iter()
            .map(|column| scryer_application::UiTableColumnSetting {
                facet: to_app_ui_settings_facet(column.facet),
                table_view_mode: to_app_ui_table_view_mode(column.table_view_mode),
                column_id: column.column_id,
                column_order: column.column_order,
                visible: column.visible,
            })
            .collect(),
    }
}

fn from_external_auth_runtime_settings(
    settings: scryer_application::ExternalAuthRuntimeSettings,
    effective_form_login_enabled: bool,
) -> ExternalAuthRuntimeSettingsPayload {
    ExternalAuthRuntimeSettingsPayload {
        login_providers: if effective_form_login_enabled {
            settings
                .login_providers
                .into_iter()
                .map(ExternalAccountProviderValue::from_domain)
                .collect()
        } else {
            Vec::new()
        },
        linking_providers: settings
            .linking_providers
            .into_iter()
            .map(ExternalAccountProviderValue::from_domain)
            .collect(),
        connections: settings
            .connections
            .into_iter()
            .map(|connection| ExternalAuthRuntimeConnectionPayload {
                id: connection.id.into(),
                provider: ExternalAccountProviderValue::from_domain(connection.provider),
                display_name: connection.display_name,
                login_enabled: effective_form_login_enabled && connection.login_enabled,
                linking_enabled: connection.linking_enabled,
                emby_connect_enabled: connection.emby_connect_enabled,
            })
            .collect(),
    }
}

fn from_auth_runtime_state(
    auth_runtime: &AuthRuntimeStateSnapshot,
    security_settings: scryer_application::SecuritySettings,
    default_persist_session: bool,
) -> AuthRuntimeStatePayload {
    AuthRuntimeStatePayload {
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        skip_login_for_local_ips: auth_runtime.skip_login_for_local_ips,
        passkey_enabled: auth_runtime.passkey_enabled,
        default_persist_session,
        env_override_active: auth_runtime.env_override_active,
        mfa_require_password_login: auth_runtime.effective_form_login_enabled
            && security_settings.mfa_require_password_login,
        mfa_require_config_step_up: auth_runtime.effective_form_login_enabled
            && security_settings.mfa_require_config_step_up,
        mfa_require_jellyfin_login: auth_runtime.effective_form_login_enabled
            && security_settings.totp_require_jellyfin_login,
        mfa_require_emby_login: auth_runtime.effective_form_login_enabled
            && security_settings.totp_require_emby_login,
        totp_require_jellyfin_login: auth_runtime.effective_form_login_enabled
            && security_settings.totp_require_jellyfin_login,
        totp_require_emby_login: auth_runtime.effective_form_login_enabled
            && security_settings.totp_require_emby_login,
    }
}

fn from_passkey_summary(summary: scryer_application::PasskeySummary) -> PasskeySummaryPayload {
    PasskeySummaryPayload {
        id: summary.id.into(),
        friendly_name: summary.friendly_name,
        created_at: parse_required_datetime(&summary.created_at, "passkey created_at"),
        last_used_at: parse_optional_datetime(summary.last_used_at, "passkey last_used_at"),
    }
}

fn from_totp_status(status: scryer_application::TotpStatus) -> TotpStatusPayload {
    TotpStatusPayload {
        enabled: status.enabled,
        created_at: parse_optional_datetime(status.created_at, "TOTP created_at"),
        last_used_at: parse_optional_datetime(status.last_used_at, "TOTP last_used_at"),
        recovery_codes_remaining: status.recovery_codes_remaining,
    }
}

fn from_delay_profile(profile: scryer_application::DelayProfile) -> DelayProfilePayload {
    DelayProfilePayload {
        id: profile.id.into(),
        name: profile.name,
        usenet_delay_minutes: profile.usenet_delay_minutes as i32,
        torrent_delay_minutes: profile.torrent_delay_minutes as i32,
        enable_usenet: profile.enable_usenet,
        enable_torrent: profile.enable_torrent,
        preferred_protocol: DelayProfilePreferredProtocolValue::from_application(
            profile.preferred_protocol,
        ),
        min_age_minutes: profile.min_age_minutes as i32,
        bypass_score_threshold: profile.bypass_score_threshold,
        bypass_if_highest_quality: profile.bypass_if_highest_quality,
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

#[allow(clippy::too_many_arguments)]
#[Object]
impl SettingsQueries {
    /// Returns the current subtitle settings available to the authenticated actor.
    async fn subtitle_settings(&self, ctx: &Context<'_>) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_subtitle_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_subtitle_settings(settings))
    }

    /// Returns acquisition thresholds and polling settings for the authenticated actor.
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

    /// Returns system settings, effective image-cache limits, and trusted plugin certificates.
    async fn general_settings(&self, ctx: &Context<'_>) -> GqlResult<GeneralSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_general_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_general_settings(settings))
    }

    /// Returns whether the recycle bin is enabled for the authenticated actor.
    async fn recycle_bin_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<RecycleBinSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_recycle_bin_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_recycle_bin_settings(settings))
    }

    /// Returns whether the scheduled plugin catalog refresh installs official patch updates automatically.
    async fn plugin_auto_update_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<PluginAutoUpdateSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_plugin_auto_update_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_auto_update_settings(settings))
    }

    /// Returns auto-backup scheduling metadata without exposing the backup key.
    async fn auto_backup_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AutoBackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_auto_backup_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_auto_backup_settings(settings))
    }

    /// Returns configured, default, and effective backup paths.
    async fn backup_settings(&self, ctx: &Context<'_>) -> GqlResult<BackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_backup_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_backup_settings(settings))
    }

    /// Returns the current UI settings for the authenticated actor.
    async fn my_ui_settings(&self, ctx: &Context<'_>) -> GqlResult<UiSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app.get_my_ui_settings(&actor).await.map_err(to_gql_error)?;
        Ok(from_ui_settings(settings))
    }

    /// Returns saved security settings together with effective and environment-overridden login state.
    async fn security_settings(&self, ctx: &Context<'_>) -> GqlResult<SecuritySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let settings = app
            .get_security_settings(&actor)
            .await
            .map_err(to_gql_error)?;

        Ok(from_security_settings(settings, &auth_runtime.snapshot()))
    }

    /// Lists managed and administrator-created OAuth applications.
    async fn oauth_client_registrations(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<OAuthClientRegistrationPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_oauth_client_registrations(&actor)
            .await
            .map(|clients| {
                clients
                    .into_iter()
                    .map(from_oauth_client_registration)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Resolves the display name for an OAuth authorization request after validating its callback URL.
    async fn oauth_authorization_client(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "OAuth client identifier from the authorization request.")]
        client_id: String,
        #[graphql(desc = "Redirect URI from the authorization request.")] redirect_uri: String,
        #[graphql(desc = "Requested OAuth scopes, validated and canonicalized before display.")]
        scope: Option<String>,
    ) -> GqlResult<OAuthAuthorizationClientPayload> {
        let app = app_from_ctx(ctx)?;
        let form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let scope =
            app.effective_oauth_authorization_scope(scope.as_deref(), form_login_enabled)?;
        app.validate_oauth_redirect_uri(&client_id, &redirect_uri)
            .await
            .map(|client| OAuthAuthorizationClientPayload {
                client_id: client.client_id,
                display_name: client.name,
                scope,
            })
            .map_err(to_gql_error)
    }

    /// Returns effective external-auth providers and connections, hiding login capability when form login is disabled.
    async fn external_auth_runtime_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ExternalAuthRuntimeSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let effective_form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        app.get_external_auth_runtime_settings()
            .await
            .map(|settings| {
                from_external_auth_runtime_settings(settings, effective_form_login_enabled)
            })
            .map_err(to_gql_error)
    }

    /// Lists media-server connections, optionally restricted to one provider.
    async fn media_server_connections(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Optional provider filter; omit or pass null to list every connection.")]
        provider: Option<MediaServerProviderValue>,
    ) -> GqlResult<Vec<MediaServerConnectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_media_server_connections(
            &actor,
            provider.map(MediaServerProviderValue::into_domain),
        )
        .await
        .map(|connections| {
            connections
                .into_iter()
                .map(from_media_server_connection)
                .collect()
        })
        .map_err(to_gql_error)
    }

    /// Lists users from the media-server connection identified by `connection_id`.
    async fn jellyfin_server_users(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the Jellyfin media-server connection to query.")] connection_id: ID,
        #[graphql(desc = "Optional user-name search; omit or pass null for no text filter.")]
        search: Option<String>,
    ) -> GqlResult<Vec<JellyfinServerUserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_jellyfin_server_users(&actor, connection_id.as_ref(), search.as_deref())
            .await
            .map(|users| users.into_iter().map(from_jellyfin_server_user).collect())
            .map_err(to_gql_error)
    }

    /// Lists grouped media-server users, optionally filtered by search text.
    async fn media_server_users(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Optional user search; omit or pass null to return all groups.")]
        search: Option<String>,
    ) -> GqlResult<Vec<MediaServerUserGroupPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_media_server_users(&actor, search.as_deref())
            .await
            .map(|groups| {
                groups
                    .into_iter()
                    .map(from_media_server_user_group)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Returns effective authentication and MFA requirements after runtime overrides are applied.
    async fn auth_runtime_state(&self, ctx: &Context<'_>) -> GqlResult<AuthRuntimeStatePayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let security_settings = app.security_settings().await.map_err(to_gql_error)?;
        Ok(from_auth_runtime_state(
            &auth_runtime.snapshot(),
            security_settings,
            default_persist_session_from_ctx(ctx),
        ))
    }

    /// Lists the authenticated actor's passkey summaries without credential material.
    async fn my_passkeys(&self, ctx: &Context<'_>) -> GqlResult<Vec<PasskeySummaryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.list_my_passkeys(&actor, auth_runtime.snapshot().effective_form_login_enabled)
            .await
            .map(|passkeys| passkeys.into_iter().map(from_passkey_summary).collect())
            .map_err(to_gql_error)
    }

    /// Returns TOTP status and recovery-code count without exposing the shared secret or codes.
    async fn my_totp(&self, ctx: &Context<'_>) -> GqlResult<TotpStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.totp_status(&actor)
            .await
            .map(from_totp_status)
            .map_err(to_gql_error)
    }

    /// Lists OAuth grants authorized by the authenticated actor without client secrets.
    async fn my_oauth_apps(&self, ctx: &Context<'_>) -> GqlResult<Vec<OAuthConnectedAppPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_oauth_connected_apps(&actor)
            .await
            .map(|apps| apps.into_iter().map(from_oauth_connected_app).collect())
            .map_err(to_gql_error)
    }

    /// Lists the interactive actor's API keys without returning their secrets.
    async fn my_api_keys(&self, ctx: &Context<'_>) -> GqlResult<Vec<ApiKeyPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = api_key_management_actor_from_ctx(ctx)?;
        let owner_username = actor.username.clone();
        app.list_api_keys(&actor)
            .await
            .map(|keys| {
                keys.into_iter()
                    .map(|key| from_api_key(key, &owner_username))
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Whether the interactive actor may create API keys under the active security policy.
    async fn can_create_my_api_keys(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = api_key_management_actor_from_ctx(ctx)?;
        app.can_create_api_key(&actor).await.map_err(to_gql_error)
    }

    /// Lists delay profiles with minute-based delays and their media-facet assignments.
    async fn delay_profiles(&self, ctx: &Context<'_>) -> GqlResult<Vec<DelayProfilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let profiles = app.get_delay_profiles(&actor).await.map_err(to_gql_error)?;
        Ok(profiles.into_iter().map(from_delay_profile).collect())
    }

    /// Returns media settings for the requested content scope.
    async fn media_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Content scope whose media settings should be returned.")]
        scope: ContentScopeValue,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_media_settings(&actor, scope.into_media_facet())
            .await
            .map(|settings| from_media_settings(scope, settings))
            .map_err(to_gql_error)
    }

    /// Returns the configured movie, series, and anime library paths.
    async fn library_paths(&self, ctx: &Context<'_>) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_library_paths(&actor)
            .await
            .map(from_library_paths_settings)
            .map_err(to_gql_error)
    }

    /// Returns service TLS configuration and related settings.
    async fn service_settings(&self, ctx: &Context<'_>) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_service_settings(&actor)
            .await
            .map(from_service_settings)
            .map_err(to_gql_error)
    }

    /// Returns quality profiles and the current global and facet selections.
    async fn quality_profile_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_quality_profile_settings(&actor)
            .await
            .map(from_quality_profile_settings)
            .map_err(to_gql_error)
    }

    /// Returns download-client routing entries for a content scope.
    async fn download_client_routing(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Content scope whose download-client routing should be returned.")]
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_download_client_routing(&actor, scope.as_scope_id())
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(from_download_client_routing_entry)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Returns indexer routing entries for a content scope.
    async fn indexer_routing(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Content scope whose indexer routing should be returned.")]
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_indexer_routing(&actor, scope.as_scope_id())
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(from_indexer_routing_entry)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Lists indexer configurations and current query statistics, optionally filtered by provider type.
    async fn indexers(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional provider-type filter; omit or pass null to list every indexer."
        )]
        provider_type: Option<String>,
    ) -> GqlResult<Vec<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_indexer_configs(&actor, provider_type)
            .await
            .map_err(to_gql_error)?;
        let stats = app
            .indexer_query_stats(&actor)
            .await
            .map_err(to_gql_error)?;
        let mut payloads = Vec::with_capacity(configs.len());
        for config in configs {
            let config_fields = app
                .indexer_config_fields_for_provider_type(&config.provider_type)
                .unwrap_or_default();
            payloads.push(from_indexer_config_with_fields(config, &config_fields));
        }
        for payload in &mut payloads {
            if let Some(s) = stats.iter().find(|s| s.indexer_id == payload.id.as_ref()) {
                payload.last_query_at =
                    parse_optional_datetime(s.last_query_at.clone(), "indexer stats last_query_at");
            }
        }
        Ok(payloads)
    }

    /// Lists configured torrent seeding profiles.
    async fn seeding_profiles(&self, ctx: &Context<'_>) -> GqlResult<Vec<SeedingProfilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .list_seeding_profiles(&actor)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(from_seeding_profile)
            .collect())
    }

    /// Returns the seeding profile applied when nothing more specific matches.
    async fn default_seeding_profile(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<DefaultSeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(DefaultSeedingProfilePayload {
            seeding_profile_id: app
                .get_default_seeding_profile_id(&actor)
                .await
                .map_err(to_gql_error)?
                .map(Into::into),
            minimum_seeders_floor: app
                .get_minimum_seeders_floor(&actor)
                .await
                .map_err(to_gql_error)?,
        })
    }

    /// Returns compatible download clients and indexers for routing configuration.
    async fn indexer_download_client_mapping_catalog(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<IndexerDownloadClientMappingCatalogPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let catalog = app
            .get_indexer_download_client_mapping_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(IndexerDownloadClientMappingCatalogPayload {
            clients: catalog
                .clients
                .into_iter()
                .map(|client| IndexerDownloadClientMappingClientPayload {
                    id: client.id.into(),
                    name: client.name,
                    client_type: client.client_type,
                    is_enabled: client.is_enabled,
                    health_status: client.health_status,
                })
                .collect(),
            indexers: catalog
                .indexers
                .into_iter()
                .map(|indexer| IndexerDownloadClientMappingIndexerPayload {
                    id: indexer.id.into(),
                    name: indexer.name,
                    download_client_id: indexer.download_client_id.map(Into::into),
                    protocol_families: indexer.protocol_families,
                    supports_mapping: indexer.supports_mapping,
                    compatible_client_ids: indexer
                        .compatible_client_ids
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                })
                .collect(),
            provider_compatibility: catalog
                .provider_compatibility
                .into_iter()
                .map(
                    |provider| IndexerDownloadClientProviderCompatibilityPayload {
                        provider_type: provider.provider_type,
                        protocol_families: provider.protocol_families,
                        supports_mapping: provider.supports_mapping,
                        compatible_client_ids: provider
                            .compatible_client_ids
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    },
                )
                .collect(),
        })
    }

    /// Lists configured indexer proxy settings.
    async fn indexer_proxy_configs(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<IndexerProxyConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_indexer_proxy_configs(&actor)
            .await
            .map(|configs| configs.into_iter().map(from_indexer_proxy_config).collect())
            .map_err(to_gql_error)
    }

    /// Lists root folders for a media facet.
    async fn root_folders(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media facet whose root folders should be returned.")]
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

    /// Lists download-client configurations with provider field metadata and redacted secrets.
    async fn download_client_configs(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<DownloadClientConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(to_gql_error)?;
        let field_map = app
            .available_download_client_provider_types()
            .into_iter()
            .map(|(provider_type, _, fields, _)| (provider_type, fields))
            .collect::<std::collections::HashMap<_, _>>();
        Ok(configs
            .into_iter()
            .map(|config| {
                let fields = field_map
                    .get(config.client_type.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                from_download_client_config_with_fields(config, fields)
            })
            .collect())
    }

    /// Lists subtitle-provider configurations, optionally filtered by provider type.
    async fn subtitle_provider_configs(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional provider-type filter; matching is case-insensitive and null returns all providers."
        )]
        provider_type: Option<String>,
    ) -> GqlResult<Vec<SubtitleProviderConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_subtitle_provider_configs(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(configs
            .into_iter()
            .filter(|config| {
                provider_type.as_ref().is_none_or(|provider_type| {
                    config.provider_type.eq_ignore_ascii_case(provider_type)
                })
            })
            .map(|config| {
                let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
                from_subtitle_provider_config(config, &config_fields)
            })
            .collect())
    }

    /// Lists users with authorization and authentication-factor status, excluding secret values.
    async fn users(&self, ctx: &Context<'_>) -> GqlResult<Vec<UserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let users = app.list_users(&actor).await.map_err(to_gql_error)?;
        let mut payloads = Vec::with_capacity(users.len());
        for user in users {
            let user = app
                .attach_user_authorization(user)
                .await
                .map_err(to_gql_error)?;
            let auth_factor_status = app
                .user_auth_factor_status(&user.id)
                .await
                .map_err(to_gql_error)?;
            payloads.push(from_user_with_auth_factor_status(user, auth_factor_status));
        }
        Ok(payloads)
    }
}
