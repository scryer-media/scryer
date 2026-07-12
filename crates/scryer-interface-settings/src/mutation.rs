use std::time::Instant;

use async_graphql::{Context, Error, ID, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    AcquisitionSettings as AppAcquisitionSettings, AppError, LoginFailureTimingClass,
    MediaServerConnectionDraft, MediaServerConnectionPatch, QualityProfile, QualityProfileCriteria,
    SecuritySettings as AppSecuritySettings,
    UpdateAutoBackupSettings as AppUpdateAutoBackupSettings,
    UpdateBackupSettings as AppUpdateBackupSettings,
    UpdateGeneralSettings as AppUpdateGeneralSettings,
    UpdateRecycleBinSettings as AppUpdateRecycleBinSettings,
    UpdateSecuritySettings as AppUpdateSecuritySettings,
    UpdateSubtitleSettings as AppUpdateSubtitleSettings,
};

use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, mfa_enrollment_actor_from_ctx,
    mfa_verification_from_ctx, require_config_app_permission, to_gql_error, to_login_gql_error,
    to_login_gql_error_after_timing,
};
use scryer_interface_media::mappers::{
    from_download_client_routing_entry, from_indexer_routing_entry, from_library_paths_settings,
    from_media_server_connection, from_media_settings, from_plex_server_discovery,
    from_quality_profile_settings, from_service_settings, from_user_with_auth_factor_status,
};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct SettingsMutations;

const MEDIA_SERVER_LOGIN_REQUIRES_FORM_LOGIN: &str =
    "Enable form login before enabling media-server login.";

fn parse_import_mode_input(raw: Option<String>) -> GqlResult<Option<scryer_domain::ImportMode>> {
    raw.map(|value| {
        scryer_domain::ImportMode::from_setting(&value).map_err(|message| {
            to_gql_error(AppError::Validation(format!(
                "invalid importMode: {message}"
            )))
        })
    })
    .transpose()
}

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

fn from_general_settings(settings: scryer_application::GeneralSettings) -> GeneralSettingsPayload {
    GeneralSettingsPayload {
        keep_history_forever: settings.keep_history_forever,
        history_retention_days: settings.history_retention_days,
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
    settings: AppSecuritySettings,
    auth_runtime: &scryer_interface_core::AuthRuntimeStateSnapshot,
) -> SecuritySettingsPayload {
    SecuritySettingsPayload {
        form_login_enabled: settings.form_login_enabled,
        password_min_length: settings.password_min_length,
        skip_login_for_local_ips: settings.skip_login_for_local_ips,
        mfa_require_config_step_up: settings.mfa_require_config_step_up,
        mfa_require_password_login: settings.mfa_require_password_login,
        totp_require_jellyfin_login: settings.totp_require_jellyfin_login,
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        env_override_active: auth_runtime.env_override_active,
        env_override_description: auth_runtime.env_override_description.clone(),
    }
}

fn media_server_app_permissions(
    permissions: Option<Vec<AppPermissionValue>>,
) -> scryer_domain::AppPermissionMask {
    scryer_domain::AppPermissionMask::from_permissions(
        permissions
            .unwrap_or_default()
            .into_iter()
            .map(AppPermissionValue::into_domain),
    )
}

fn media_server_library_grants(
    grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
) -> Vec<scryer_domain::MediaServerDefaultLibraryGrant> {
    grants
        .unwrap_or_default()
        .into_iter()
        .map(|grant| scryer_domain::MediaServerDefaultLibraryGrant {
            library_id: grant.library_id.to_string(),
            permissions: scryer_domain::LibraryPermissionMask::from_permissions(
                grant
                    .permissions
                    .into_iter()
                    .map(LibraryPermissionValue::into_domain),
            )
            .normalized_for_storage(),
        })
        .collect()
}

fn media_server_path_mappings(
    mappings: Option<Vec<MediaServerPathMappingInput>>,
) -> Vec<scryer_domain::MediaServerPathMapping> {
    mappings
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, mapping)| scryer_domain::MediaServerPathMapping {
            source_path: mapping.source_path,
            destination_path: mapping.destination_path,
            sort_order: index as i64,
        })
        .collect()
}

fn ensure_media_server_login_allowed(
    requested_login_enabled: Option<bool>,
    effective_form_login_enabled: bool,
) -> Result<(), AppError> {
    if requested_login_enabled.unwrap_or(false) && !effective_form_login_enabled {
        Err(AppError::Validation(
            MEDIA_SERVER_LOGIN_REQUIRES_FORM_LOGIN.to_string(),
        ))
    } else {
        Ok(())
    }
}

fn media_server_draft(input: CreateMediaServerConnectionInput) -> MediaServerConnectionDraft {
    MediaServerConnectionDraft {
        provider: input.provider.into_domain(),
        display_name: input.display_name,
        base_url: input.base_url,
        enabled: input.enabled.unwrap_or(true),
        login_enabled: input.login_enabled.unwrap_or(false),
        linking_enabled: input.linking_enabled.unwrap_or(false),
        auto_add_enabled: input.auto_add_enabled.unwrap_or(false),
        default_app_permissions: media_server_app_permissions(input.default_app_permissions),
        default_library_grants: media_server_library_grants(input.default_library_grants),
        machine_id: input.machine_id,
        plex_auth_token: input.plex_auth_token,
        plex_server_id: input.plex_server_id,
        api_key: input.api_key,
        admin_username: input.admin_username,
        admin_password: input.admin_password,
        path_mappings: media_server_path_mappings(input.path_mappings),
    }
}

fn media_server_patch(input: UpdateMediaServerConnectionInput) -> MediaServerConnectionPatch {
    MediaServerConnectionPatch {
        id: input.id.to_string(),
        provider: input.provider.map(MediaServerProviderValue::into_domain),
        display_name: input.display_name,
        base_url: input.base_url,
        enabled: input.enabled,
        login_enabled: input.login_enabled,
        linking_enabled: input.linking_enabled,
        auto_add_enabled: input.auto_add_enabled,
        default_app_permissions: input
            .default_app_permissions
            .map(|permissions| media_server_app_permissions(Some(permissions))),
        default_library_grants: input
            .default_library_grants
            .map(|grants| media_server_library_grants(Some(grants))),
        machine_id: input.machine_id,
        clear_machine_id: input.clear_machine_id.unwrap_or(false),
        plex_auth_token: input.plex_auth_token,
        plex_server_id: input.plex_server_id,
        api_key: input.api_key,
        clear_api_key: input.clear_api_key.unwrap_or(false),
        admin_username: input.admin_username,
        admin_password: input.admin_password,
        path_mappings: input
            .path_mappings
            .map(|mappings| media_server_path_mappings(Some(mappings))),
    }
}

fn from_delay_profile(profile: scryer_application::DelayProfile) -> DelayProfilePayload {
    DelayProfilePayload {
        id: profile.id.into(),
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

fn from_webauthn_challenge_start(
    challenge: scryer_application::WebauthnChallengeStart,
) -> WebauthnChallengePayload {
    WebauthnChallengePayload {
        challenge_id: challenge.challenge_id.into(),
        options_json: challenge.options_json,
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

fn from_totp_enrollment_start(
    start: scryer_application::TotpEnrollmentStart,
) -> TotpEnrollmentStartPayload {
    TotpEnrollmentStartPayload {
        challenge_id: start.challenge_id.into(),
        otpauth_url: start.otpauth_url,
        secret_base32: start.secret_base32,
        expires_at: parse_required_datetime(&start.expires_at, "TOTP enrollment expires_at"),
    }
}

fn from_totp_enrollment_complete(
    complete: scryer_application::TotpEnrollmentComplete,
) -> TotpEnrollmentCompletePayload {
    TotpEnrollmentCompletePayload {
        status: from_totp_status(complete.status),
        recovery_codes: complete.recovery_codes,
    }
}

async fn login_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    mfa_verified_until: Option<chrono::DateTime<Utc>>,
    mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
) -> Result<LoginPayload, Error> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_access_token_with_mfa(&user, mfa_verified_until, mfa_step_up_verified_until)
        .await
        .map_err(to_gql_error)?;
    let auth_factor_status = app
        .user_auth_factor_status(&user.id)
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(app.token_lifetime());
    Ok(LoginPayload {
        token,
        user: from_user_with_auth_factor_status(user, auth_factor_status),
        expires_at,
        mfa_verified_until,
        mfa_enrollment_required: false,
    })
}

async fn login_mfa_enrollment_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
) -> Result<LoginPayload, Error> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let token = app
        .issue_mfa_enrollment_token(&user)
        .await
        .map_err(to_gql_error)?;
    let auth_factor_status = app
        .user_auth_factor_status(&user.id)
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(app.mfa_enrollment_token_lifetime());
    Ok(LoginPayload {
        token,
        user: from_user_with_auth_factor_status(user, auth_factor_status),
        expires_at,
        mfa_verified_until: None,
        mfa_enrollment_required: true,
    })
}

fn normalize_quality_profile(profile: QualityProfile) -> QualityProfile {
    let normalize_list = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.to_ascii_lowercase()))
            .collect::<Vec<_>>()
    };

    let normalize_quality_tiers = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.clone()))
            .collect::<Vec<_>>()
    };

    let normalize_video_codec_list = |values: Vec<scryer_application::VideoCodec>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|codec| seen.insert(codec.to_string()))
            .collect::<Vec<_>>()
    };
    let normalize_source_list = |values: Vec<scryer_application::ReleaseSource>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|source| seen.insert(source.to_string()))
            .collect::<Vec<_>>()
    };
    let normalize_audio_codec_list = |values: Vec<scryer_application::AudioCodec>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|codec| seen.insert(codec.to_string()))
            .collect::<Vec<_>>()
    };

    let criteria = profile.criteria;
    let mut facet_persona_overrides = std::collections::HashMap::new();
    for (scope, persona) in criteria.facet_persona_overrides {
        if let Some(scope) = ContentScopeValue::parse(&scope) {
            facet_persona_overrides.insert(scope.as_scope_id().to_string(), persona);
        }
    }

    QualityProfile {
        id: profile.id.trim().to_string(),
        name: profile.name.trim().to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: normalize_quality_tiers(criteria.quality_tiers),
            archival_quality: criteria
                .archival_quality
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: normalize_source_list(criteria.source_allowlist),
            source_blocklist: normalize_source_list(criteria.source_blocklist),
            video_codec_allowlist: normalize_video_codec_list(criteria.video_codec_allowlist),
            video_codec_blocklist: normalize_video_codec_list(criteria.video_codec_blocklist),
            audio_codec_allowlist: normalize_audio_codec_list(criteria.audio_codec_allowlist),
            audio_codec_blocklist: normalize_audio_codec_list(criteria.audio_codec_blocklist),
            atmos_preferred: criteria.atmos_preferred,
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: criteria.prefer_dual_audio,
            required_audio_languages: normalize_list(criteria.required_audio_languages),
            scoring_persona: criteria.scoring_persona,
            scoring_overrides: criteria.scoring_overrides,
            cutoff_tier: criteria
                .cutoff_tier
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides,
        },
    }
}

fn quality_profile_from_input(
    input: QualityProfileInput,
    existing: Option<&QualityProfile>,
) -> GqlResult<QualityProfile> {
    let criteria = input.criteria;
    let source_allowlist =
        parse_source_values(criteria.source_allowlist, "criteria.source_allowlist")?;
    let source_blocklist =
        parse_source_values(criteria.source_blocklist, "criteria.source_blocklist")?;
    let video_codec_allowlist = parse_video_codec_values(
        criteria.video_codec_allowlist,
        "criteria.video_codec_allowlist",
    )?;
    let video_codec_blocklist = parse_video_codec_values(
        criteria.video_codec_blocklist,
        "criteria.video_codec_blocklist",
    )?;
    let audio_codec_allowlist = parse_audio_codec_values(
        criteria.audio_codec_allowlist,
        "criteria.audio_codec_allowlist",
    )?;
    let audio_codec_blocklist = parse_audio_codec_values(
        criteria.audio_codec_blocklist,
        "criteria.audio_codec_blocklist",
    )?;

    let profile = normalize_quality_profile(QualityProfile {
        id: input.id.to_string(),
        name: input.name,
        criteria: QualityProfileCriteria {
            quality_tiers: criteria.quality_tiers,
            archival_quality: criteria.archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist,
            source_blocklist,
            video_codec_allowlist,
            video_codec_blocklist,
            audio_codec_allowlist,
            audio_codec_blocklist,
            atmos_preferred: existing
                .map(|profile| profile.criteria.atmos_preferred)
                .unwrap_or(false),
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: false,
            required_audio_languages: Vec::new(),
            scoring_persona: scryer_application::ScoringPersona::Balanced,
            scoring_overrides: criteria.scoring_overrides.into_application(),
            cutoff_tier: criteria.cutoff_tier,
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides: std::collections::HashMap::new(),
        },
    });

    if profile.id.is_empty() {
        return Err(to_gql_error(AppError::Validation(
            "quality profile id is required".to_string(),
        )));
    }
    if profile.name.is_empty() {
        return Err(to_gql_error(AppError::Validation(
            "quality profile name is required".to_string(),
        )));
    }
    if profile.criteria.quality_tiers.is_empty() {
        return Err(to_gql_error(AppError::Validation(
            "quality profile must include at least one quality tier".to_string(),
        )));
    }

    Ok(profile)
}

fn parse_video_codec_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::VideoCodec>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::VideoCodec::parse(trimmed.as_str()).ok_or_else(|| {
                to_gql_error(AppError::Validation(format!(
                    "invalid value {trimmed:?} for {field}"
                )))
            })
        })
        .collect()
}

fn parse_source_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::ReleaseSource>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::ReleaseSource::parse(trimmed.as_str()).ok_or_else(|| {
                to_gql_error(AppError::Validation(format!(
                    "invalid value {trimmed:?} for {field}"
                )))
            })
        })
        .collect()
}

fn parse_audio_codec_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::AudioCodec>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::AudioCodec::parse(trimmed.as_str()).ok_or_else(|| {
                to_gql_error(AppError::Validation(format!(
                    "invalid value {trimmed:?} for {field}"
                )))
            })
        })
        .collect()
}

#[Object]
impl SettingsMutations {
    async fn update_subtitle_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateSubtitleSettingsInput,
    ) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;

        let settings = app
            .update_subtitle_settings(
                &actor,
                AppUpdateSubtitleSettings {
                    enabled: input.enabled,
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
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;

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

    async fn update_general_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateGeneralSettingsInput,
    ) -> GqlResult<GeneralSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;

        let settings = app
            .update_general_settings(
                &actor,
                AppUpdateGeneralSettings {
                    keep_history_forever: input.keep_history_forever,
                    history_retention_days: input.history_retention_days,
                    plugin_http_ca_bundle_pem: input.plugin_http_ca_bundle_pem,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_general_settings(settings))
    }

    async fn clear_title_image_cache(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ClearTitleImageCachePayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let accepted = app
            .clear_title_image_cache(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(ClearTitleImageCachePayload { accepted })
    }

    async fn update_recycle_bin_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateRecycleBinSettingsInput,
    ) -> GqlResult<RecycleBinSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;

        let settings = app
            .update_recycle_bin_settings(
                &actor,
                AppUpdateRecycleBinSettings {
                    enabled: input.enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_recycle_bin_settings(settings))
    }

    async fn update_auto_backup_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateAutoBackupSettingsInput,
    ) -> GqlResult<AutoBackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;

        let settings = app
            .update_auto_backup_settings(
                &actor,
                AppUpdateAutoBackupSettings {
                    enabled: input.enabled,
                    daily_time_local: input.daily_time_local,
                    set_auto_backup_key: input.set_auto_backup_key,
                    clear_auto_backup_key: input.clear_auto_backup_key,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_auto_backup_settings(settings))
    }

    async fn update_backup_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateBackupSettingsInput,
    ) -> GqlResult<BackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;

        let settings = app
            .update_backup_settings(
                &actor,
                AppUpdateBackupSettings {
                    custom_backup_path: input.custom_backup_path,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_backup_settings(settings))
    }

    async fn acknowledge_auto_backup_disabled_missing_key_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AutoBackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let settings = app
            .acknowledge_auto_backup_disabled_missing_key_notice(&actor)
            .await
            .map_err(to_gql_error)?;

        Ok(from_auto_backup_settings(settings))
    }

    async fn update_security_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateSecuritySettingsInput,
    ) -> GqlResult<SecuritySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageUsers).await?;
        let previous_snapshot = auth_runtime.snapshot();

        let settings = app
            .update_security_settings(
                &actor,
                AppUpdateSecuritySettings {
                    form_login_enabled: input.form_login_enabled,
                    password_min_length: input.password_min_length,
                    skip_login_for_local_ips: input.skip_login_for_local_ips,
                    mfa_require_config_step_up: input.mfa_require_config_step_up,
                    mfa_require_password_login: input.mfa_require_password_login,
                    totp_require_jellyfin_login: input.totp_require_jellyfin_login,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let snapshot = auth_runtime.apply_saved_security_settings(
            settings.form_login_enabled,
            settings.skip_login_for_local_ips,
        );
        if !previous_snapshot.effective_form_login_enabled && snapshot.effective_form_login_enabled
        {
            app.revoke_authless_oauth_refresh_grants("form_login_enabled")
                .await
                .map_err(to_gql_error)?;
        }

        Ok(from_security_settings(settings, &snapshot))
    }

    async fn create_media_server_connection(
        &self,
        ctx: &Context<'_>,
        input: CreateMediaServerConnectionInput,
    ) -> GqlResult<MediaServerConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        ensure_media_server_login_allowed(
            input.login_enabled,
            auth_runtime_from_ctx(ctx)
                .snapshot()
                .effective_form_login_enabled,
        )
        .map_err(to_gql_error)?;
        app.create_media_server_connection(&actor, media_server_draft(input))
            .await
            .map(from_media_server_connection)
            .map_err(to_gql_error)
    }

    async fn update_media_server_connection(
        &self,
        ctx: &Context<'_>,
        input: UpdateMediaServerConnectionInput,
    ) -> GqlResult<MediaServerConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        ensure_media_server_login_allowed(
            input.login_enabled,
            auth_runtime_from_ctx(ctx)
                .snapshot()
                .effective_form_login_enabled,
        )
        .map_err(to_gql_error)?;
        app.update_media_server_connection(&actor, media_server_patch(input))
            .await
            .map(from_media_server_connection)
            .map_err(to_gql_error)
    }

    async fn delete_media_server_connection(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteMediaServerConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let id = id.to_string();
        app.delete_media_server_connection(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteMediaServerConnectionPayload {
            id: ID::from(id),
            deleted: true,
        })
    }

    async fn test_media_server_connection(
        &self,
        ctx: &Context<'_>,
        input: TestMediaServerConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let id = input.id.to_string();
        app.test_media_server_connection(&actor, &id, input.plex_auth_token.as_deref())
            .await
            .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: "ok".to_string(),
            message: None,
            retry_after_seconds: None,
        })
    }

    async fn discover_plex_media_servers(
        &self,
        ctx: &Context<'_>,
        plex_auth_token: String,
    ) -> GqlResult<Vec<PlexServerDiscoveryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.discover_plex_media_servers(&actor, &plex_auth_token)
            .await
            .map(|servers| {
                servers
                    .into_iter()
                    .map(from_plex_server_discovery)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    async fn upsert_delay_profile(
        &self,
        ctx: &Context<'_>,
        input: DelayProfileInput,
    ) -> GqlResult<DelayProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;

        let profile = app
            .upsert_delay_profile(
                &actor,
                scryer_application::DelayProfile {
                    id: input.id.to_string(),
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
        id: ID,
    ) -> GqlResult<DelayProfileDeletionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let id = id.to_string();
        let id = app
            .delete_delay_profile(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DelayProfileDeletionPayload { id: ID::from(id) })
    }

    async fn update_media_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateMediaSettingsInput,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let scope = input.scope;
        let import_mode = parse_import_mode_input(input.import_mode)?;
        app.update_media_settings(
            &actor,
            scope.into_media_facet(),
            scryer_application::UpdateMediaSettings {
                library_path: input.library_path,
                root_folders: input.root_folders.map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| scryer_domain::RootFolderEntry {
                            path: entry.path,
                            is_default: entry.is_default,
                        })
                        .collect()
                }),
                required_audio_languages: input.required_audio_languages,
                folder_template: input.folder_template,
                season_folder_template: input.season_folder_template,
                rename_enabled: input.rename_enabled,
                rename_template: input.rename_template,
                rename_collision_policy: input.rename_collision_policy,
                rename_missing_metadata_policy: input.rename_missing_metadata_policy,
                filler_policy: input.filler_policy,
                recap_policy: input.recap_policy,
                monitor_specials: input.monitor_specials,
                inter_season_movies: input.inter_season_movies,
                monitor_filler_movies: input.monitor_filler_movies,
                nfo_write_on_import: input.nfo_write_on_import,
                plexmatch_write_on_import: input.plexmatch_write_on_import,
                import_mode,
            },
        )
        .await
        .map(|settings| from_media_settings(scope, settings))
        .map_err(to_gql_error)
    }

    async fn update_library_paths(
        &self,
        ctx: &Context<'_>,
        input: UpdateLibraryPathsInput,
    ) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        app.update_library_paths(
            &actor,
            scryer_application::UpdateLibraryPaths {
                movie_path: input.movie_path,
                series_path: input.series_path,
                anime_path: input.anime_path,
            },
        )
        .await
        .map(from_library_paths_settings)
        .map_err(to_gql_error)
    }

    async fn update_service_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateServiceSettingsInput,
    ) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.update_service_settings(
            &actor,
            scryer_application::UpdateServiceSettings {
                tls_cert_path: input.tls_cert_path,
                tls_key_path: input.tls_key_path,
            },
        )
        .await
        .map(from_service_settings)
        .map_err(to_gql_error)
    }

    async fn save_quality_profile_settings(
        &self,
        ctx: &Context<'_>,
        input: SaveQualityProfileSettingsInput,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let current = app
            .get_quality_profile_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        let existing_by_id = current
            .profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile))
            .collect::<std::collections::HashMap<_, _>>();

        let profiles = input
            .profiles
            .into_iter()
            .map(|profile| {
                let existing = existing_by_id.get(profile.id.as_ref()).copied();
                quality_profile_from_input(profile, existing)
            })
            .collect::<GqlResult<Vec<_>>>()?;
        app.save_quality_profile_settings(
            &actor,
            scryer_application::SaveQualityProfileSettings {
                profiles,
                replace_existing: input.replace_existing,
                global_profile_id: input.global_profile_id.map(String::from),
                category_selections: input
                    .category_selections
                    .into_iter()
                    .map(
                        |selection| scryer_application::UpdateQualityProfileSelection {
                            facet: selection.scope.into_media_facet(),
                            inherit_global: selection.inherit_global,
                            profile_id: selection.profile_id.map(String::from),
                        },
                    )
                    .collect(),
                global_scoring_persona: input
                    .global_scoring_persona
                    .map(ScoringPersonaValue::into_application),
                category_persona_selections: input
                    .category_persona_selections
                    .into_iter()
                    .map(
                        |selection| scryer_application::UpdateFacetScoringPersonaSelection {
                            facet: selection.scope.into_media_facet(),
                            inherit_global: selection.inherit_global,
                            persona: selection.persona.map(ScoringPersonaValue::into_application),
                        },
                    )
                    .collect(),
            },
        )
        .await
        .map(from_quality_profile_settings)
        .map_err(to_gql_error)
    }

    async fn update_download_client_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientRoutingInput,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let scope = input.scope;
        app.update_download_client_routing(
            &actor,
            scope.as_scope_id(),
            input
                .entries
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
                .collect(),
        )
        .await
        .map(|entries| {
            entries
                .into_iter()
                .map(from_download_client_routing_entry)
                .collect()
        })
        .map_err(to_gql_error)
    }

    async fn update_indexer_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerRoutingInput,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let scope = input.scope;
        app.update_indexer_routing(
            &actor,
            scope.as_scope_id(),
            input
                .entries
                .into_iter()
                .map(|entry| scryer_application::IndexerRoutingSettingsEntry {
                    indexer_id: entry.indexer_id.to_string(),
                    enabled: entry.enabled,
                    categories: entry.categories,
                    priority: entry.priority,
                })
                .collect(),
        )
        .await
        .map(|entries| {
            entries
                .into_iter()
                .map(from_indexer_routing_entry)
                .collect()
        })
        .map_err(to_gql_error)
    }

    async fn delete_quality_profile(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let id = id.to_string();
        app.delete_quality_profile(&actor, &id)
            .await
            .map(from_quality_profile_settings)
            .map_err(to_gql_error)
    }

    async fn webauthn_register_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_start(&actor, auth_runtime.snapshot().effective_form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    async fn webauthn_register_complete(
        &self,
        ctx: &Context<'_>,
        input: WebauthnRegisterCompleteInput,
    ) -> GqlResult<PasskeySummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_complete(
            &actor,
            input.challenge_id.as_ref(),
            &input.response_json,
            input.friendly_name,
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map(from_passkey_summary)
        .map_err(to_gql_error)
    }

    async fn totp_enrollment_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<TotpEnrollmentStartPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let login_enrollment =
            mfa.session_scope == scryer_application::JwtSessionScope::MfaEnrollment;
        let actor = if login_enrollment {
            mfa_enrollment_actor_from_ctx(ctx)?
        } else {
            actor_from_ctx(ctx)?
        };
        let start = if login_enrollment {
            app.start_login_mfa_enrollment(&actor).await
        } else {
            app.totp_enrollment_start(&actor).await
        }
        .map_err(to_gql_error)?;
        Ok(from_totp_enrollment_start(start))
    }

    async fn totp_enrollment_complete(
        &self,
        ctx: &Context<'_>,
        input: TotpEnrollmentCompleteInput,
    ) -> GqlResult<TotpEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.totp_enrollment_complete(&actor, input.challenge_id.as_ref(), &input.code)
            .await
            .map(from_totp_enrollment_complete)
            .map_err(to_gql_error)
    }

    async fn complete_login_mfa_enrollment(
        &self,
        ctx: &Context<'_>,
        input: TotpEnrollmentCompleteInput,
    ) -> GqlResult<LoginMfaEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = mfa_enrollment_actor_from_ctx(ctx)?;
        let complete = app
            .complete_login_mfa_enrollment(&actor, input.challenge_id.as_ref(), &input.code)
            .await
            .map_err(to_gql_error)?;
        let login =
            login_payload_from_user(&app, actor, Some(app.mfa_freshness_verified_until()), None)
                .await?;
        Ok(LoginMfaEnrollmentCompletePayload {
            status: from_totp_status(complete.status),
            recovery_codes: complete.recovery_codes,
            login,
        })
    }

    async fn mfa_verify_step_up(
        &self,
        ctx: &Context<'_>,
        input: TotpVerifyInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mfa_step_up_verified_until = app
            .mfa_verify_step_up(&actor, &input.code)
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(&app, actor, None, Some(mfa_step_up_verified_until)).await
    }

    async fn totp_disable(
        &self,
        ctx: &Context<'_>,
        input: TotpVerifyInput,
    ) -> GqlResult<TotpStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.totp_disable(&actor, &input.code)
            .await
            .map(from_totp_status)
            .map_err(to_gql_error)
    }

    async fn totp_regenerate_recovery_codes(
        &self,
        ctx: &Context<'_>,
        input: TotpVerifyInput,
    ) -> GqlResult<TotpEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.totp_regenerate_recovery_codes(&actor, &input.code)
            .await
            .map(from_totp_enrollment_complete)
            .map_err(to_gql_error)
    }

    async fn webauthn_authenticate_start(
        &self,
        ctx: &Context<'_>,
        username: Option<String>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let form_login_enabled = auth_runtime.snapshot().effective_form_login_enabled;
        let started_at = Instant::now();
        let start = match app
            .webauthn_authenticate_start(username.as_deref(), form_login_enabled)
            .await
        {
            Ok(start) => start,
            Err(err) => {
                if !form_login_enabled {
                    return Err(to_login_gql_error("passkey", err));
                }
                return Err(to_login_gql_error_after_timing(
                    "passkey",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        Ok(from_webauthn_challenge_start(start))
    }

    async fn webauthn_authenticate_complete(
        &self,
        ctx: &Context<'_>,
        input: WebauthnCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let form_login_enabled = auth_runtime.snapshot().effective_form_login_enabled;
        let started_at = Instant::now();
        let user = match app
            .webauthn_authenticate_complete(
                input.challenge_id.as_ref(),
                &input.response_json,
                form_login_enabled,
            )
            .await
        {
            Ok(user) => user,
            Err(err) => {
                if !form_login_enabled {
                    return Err(to_login_gql_error("passkey", err));
                }
                return Err(to_login_gql_error_after_timing(
                    "passkey",
                    LoginFailureTimingClass::FastMasked,
                    started_at,
                    err,
                )
                .await);
            }
        };
        login_payload_from_user(&app, user, None, None).await
    }

    async fn delete_my_passkey(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteMyPasskeyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let id_string = id.to_string();
        app.delete_my_passkey(
            &actor,
            &id_string,
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map_err(to_gql_error)?;
        Ok(DeleteMyPasskeyPayload { id, deleted: true })
    }

    async fn revoke_my_oauth_app(
        &self,
        ctx: &Context<'_>,
        grant_id: ID,
    ) -> GqlResult<RevokeMyOauthAppPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let grant_id_string = grant_id.to_string();
        let revoked = app
            .revoke_oauth_connected_app(&actor, &grant_id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(RevokeMyOauthAppPayload { grant_id, revoked })
    }

    async fn login(&self, ctx: &Context<'_>, input: LoginInput) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = match app
            .authenticate_credentials(&input.username, &input.password)
            .await
        {
            Ok(user) => user,
            Err(err) => return Err(to_login_gql_error("local", err)),
        };
        let effective_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let password_login_mfa_required = effective_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .mfa_require_password_login;
        let mfa_verified_until = if password_login_mfa_required {
            if !app.totp_status(&user).await.map_err(to_gql_error)?.enabled {
                return login_mfa_enrollment_payload_from_user(&app, user).await;
            }
            let code = input.totp_code.as_deref().ok_or_else(|| {
                to_gql_error(scryer_application::AppError::MfaStepUpRequired(
                    "MFA code is required for password login".into(),
                ))
            })?;
            Some(
                app.verify_totp_for_user(&user, code)
                    .await
                    .map_err(to_gql_error)?,
            )
        } else {
            None
        };
        login_payload_from_user(&app, user, mfa_verified_until, None).await
    }

    /// Mark the setup wizard as complete.
    async fn complete_setup(&self, ctx: &Context<'_>) -> GqlResult<CompleteSetupPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let completed = app.complete_setup(&actor).await.map_err(to_gql_error)?;
        Ok(CompleteSetupPayload { completed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_server_login_requires_effective_form_login() {
        let error = ensure_media_server_login_allowed(Some(true), false).unwrap_err();

        assert!(matches!(
            error,
            AppError::Validation(message) if message == MEDIA_SERVER_LOGIN_REQUIRES_FORM_LOGIN
        ));
    }

    #[test]
    fn media_server_login_guard_allows_disabling_while_form_login_disabled() {
        assert!(ensure_media_server_login_allowed(Some(false), false).is_ok());
        assert!(ensure_media_server_login_allowed(None, false).is_ok());
        assert!(ensure_media_server_login_allowed(Some(true), true).is_ok());
    }
}
