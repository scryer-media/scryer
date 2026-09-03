use std::time::Instant;

use async_graphql::{Context, Error, ID, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    AcquisitionSettings as AppAcquisitionSettings, ApiKeyExpiryPreset as AppApiKeyExpiryPreset,
    AppError, CreateApiKey, CreateOAuthClientRegistration, LoginFailureTimingClass,
    LoginVerificationMethod, LoginVerificationRequirement, MediaServerConnectionDraft,
    MediaServerConnectionPatch, QualityProfile, QualityProfileCriteria,
    SecuritySettings as AppSecuritySettings,
    UpdateAutoBackupSettings as AppUpdateAutoBackupSettings,
    UpdateBackupSettings as AppUpdateBackupSettings,
    UpdateGeneralSettings as AppUpdateGeneralSettings, UpdateOAuthClientRegistration,
    UpdatePluginAutoUpdateSettings as AppUpdatePluginAutoUpdateSettings,
    UpdateRecycleBinSettings as AppUpdateRecycleBinSettings,
    UpdateSecuritySettings as AppUpdateSecuritySettings,
    UpdateSubtitleSettings as AppUpdateSubtitleSettings,
};

use super::{
    from_api_key, from_oauth_client_registration, from_plugin_auto_update_settings,
    from_ui_settings, into_oauth_client_kind, ui_settings_update_from_input,
};
use scryer_interface_core::{
    AuthlessDefaultSession, LoginAttemptPrincipal, LoginErrorClassification,
    account_security_actor_from_ctx, actor_from_ctx, api_key_management_actor_from_ctx,
    app_from_ctx, auth_runtime_from_ctx, classify_login_error, default_persist_session_from_ctx,
    interactive_session_actor_from_ctx, login_attempt_limiter_from_ctx,
    login_verification_required_gql_error, mfa_enrollment_actor_from_ctx,
    mfa_verification_from_ctx, password_change_required_actor_from_ctx, persist_session_or_default,
    require_config_app_permission, to_gql_error, to_login_gql_error,
    to_login_gql_error_after_timing, totp_enrollment_actor_from_ctx,
    totp_management_actor_from_ctx,
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

fn parse_import_mode_input(raw: Option<ImportModeValue>) -> Option<scryer_domain::ImportMode> {
    raw.map(Into::into)
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
    settings: AppSecuritySettings,
    auth_runtime: &scryer_interface_core::AuthRuntimeStateSnapshot,
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

fn resolve_mfa_route_requirement(
    generic: Option<bool>,
    legacy: Option<bool>,
    route: &str,
) -> GqlResult<Option<bool>> {
    if let (Some(generic), Some(legacy)) = (generic, legacy)
        && generic != legacy
    {
        return Err(to_gql_error(AppError::Validation(format!(
            "conflicting MFA requirements for {route} login"
        ))));
    }
    Ok(generic.or(legacy))
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
        external_url: input.external_url,
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
        emby_connection_mode: input.emby_connection_mode.map(emby_connection_mode),
        emby_local_setup_method: input.emby_local_setup_method.map(emby_local_setup_method),
        emby_connect_enabled: input.emby_connect_enabled,
        emby_connect_username_or_email: input.emby_connect_username_or_email,
        emby_connect_password: input.emby_connect_password,
        emby_connect_server_id: input.emby_connect_server_id,
        path_mappings: media_server_path_mappings(input.path_mappings),
    }
}

fn media_server_patch(input: UpdateMediaServerConnectionInput) -> MediaServerConnectionPatch {
    MediaServerConnectionPatch {
        id: input.id.to_string(),
        provider: input.provider.map(MediaServerProviderValue::into_domain),
        display_name: input.display_name,
        base_url: input.base_url,
        external_url: input.external_url,
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
        emby_connection_mode: input.emby_connection_mode.map(emby_connection_mode),
        emby_local_setup_method: input.emby_local_setup_method.map(emby_local_setup_method),
        emby_connect_enabled: input.emby_connect_enabled,
        emby_connect_username_or_email: input.emby_connect_username_or_email,
        emby_connect_password: input.emby_connect_password,
        emby_connect_server_id: input.emby_connect_server_id,
        path_mappings: input
            .path_mappings
            .map(|mappings| media_server_path_mappings(Some(mappings))),
    }
}

fn emby_connection_mode(value: EmbyConnectionModeValue) -> scryer_application::EmbyConnectionMode {
    match value {
        EmbyConnectionModeValue::Local => scryer_application::EmbyConnectionMode::Local,
        EmbyConnectionModeValue::Connect => scryer_application::EmbyConnectionMode::Connect,
    }
}

fn emby_local_setup_method(
    value: EmbyLocalSetupMethodValue,
) -> scryer_application::EmbyLocalSetupMethod {
    match value {
        EmbyLocalSetupMethodValue::ApiKey => scryer_application::EmbyLocalSetupMethod::ApiKey,
        EmbyLocalSetupMethodValue::AdminCredentials => {
            scryer_application::EmbyLocalSetupMethod::AdminCredentials
        }
    }
}

fn emby_connect_user_type(
    value: scryer_application::EmbyConnectUserType,
) -> EmbyConnectUserTypeValue {
    match value {
        scryer_application::EmbyConnectUserType::LinkedUser => EmbyConnectUserTypeValue::LinkedUser,
        scryer_application::EmbyConnectUserType::Guest => EmbyConnectUserTypeValue::Guest,
        scryer_application::EmbyConnectUserType::Unknown => EmbyConnectUserTypeValue::Unknown,
    }
}

fn emby_connect_address_status(
    value: scryer_application::EmbyConnectAddressStatus,
) -> EmbyConnectAddressStatusValue {
    match value {
        scryer_application::EmbyConnectAddressStatus::Reachable => {
            EmbyConnectAddressStatusValue::Reachable
        }
        scryer_application::EmbyConnectAddressStatus::Unreachable => {
            EmbyConnectAddressStatusValue::Unreachable
        }
        scryer_application::EmbyConnectAddressStatus::InvalidUrl => {
            EmbyConnectAddressStatusValue::InvalidUrl
        }
        scryer_application::EmbyConnectAddressStatus::ServerIdMismatch => {
            EmbyConnectAddressStatusValue::ServerIdMismatch
        }
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

fn from_webauthn_challenge_start(
    challenge: scryer_application::WebauthnChallengeStart,
) -> WebauthnChallengePayload {
    WebauthnChallengePayload {
        challenge_id: challenge.challenge_id.into(),
        options_json: scryer_interface_media::mappers::json_string_to_value(challenge.options_json),
        expires_at: parse_required_datetime(&challenge.expires_at, "WebAuthn challenge expires_at"),
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
    persist_session: bool,
    expected_auth_session_version: Option<&Option<String>>,
    _password_change_required: bool,
) -> Result<LoginPayload, Error> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let password_change_required = user.password_change_required;
    let token = if password_change_required {
        app.issue_password_change_required_token(
            &user,
            mfa_verified_until,
            persist_session,
            expected_auth_session_version,
        )
        .await
    } else {
        match expected_auth_session_version {
            Some(expected_auth_session_version) => {
                app.issue_access_token_with_mfa_and_persistence_at_auth_session_version(
                    &user,
                    mfa_verified_until,
                    mfa_step_up_verified_until,
                    persist_session,
                    expected_auth_session_version,
                )
                .await
            }
            None => {
                app.issue_access_token_with_mfa_and_persistence(
                    &user,
                    mfa_verified_until,
                    mfa_step_up_verified_until,
                    persist_session,
                )
                .await
            }
        }
    }
    .map_err(to_gql_error)?;
    let auth_factor_status = app
        .user_auth_factor_status(&user.id)
        .await
        .map_err(to_gql_error)?;
    let expires_at = Utc::now()
        + chrono::Duration::seconds(if password_change_required {
            app.mfa_enrollment_token_lifetime()
        } else {
            app.token_lifetime()
        });
    Ok(LoginPayload {
        token,
        user: from_user_with_auth_factor_status(user, auth_factor_status),
        expires_at,
        mfa_verified_until,
        security_action_verified_until: (!password_change_required)
            .then(|| app.security_action_verified_until()),
        mfa_enrollment_required: false,
        password_change_required,
        persist_session,
    })
}

async fn login_mfa_enrollment_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
    persist_session: bool,
    _password_change_required_after_enrollment: bool,
    expected_auth_session_version: &Option<String>,
) -> Result<LoginPayload, Error> {
    let user = app
        .load_user_for_auth_payload(&user)
        .await
        .map_err(to_gql_error)?;
    let password_change_required_after_enrollment = user.password_change_required;
    let token = app
        .issue_mfa_enrollment_token(
            &user,
            persist_session,
            password_change_required_after_enrollment,
            Some(expected_auth_session_version),
        )
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
        security_action_verified_until: None,
        mfa_enrollment_required: true,
        password_change_required: false,
        persist_session,
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
            cutoff_score: criteria.cutoff_score,
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
            // Preserved from the stored profile, like `atmos_preferred` above:
            // the quality-profile editor does not send `cutoffScore` yet (D19
            // says web exposure is a follow-on), so reading it from the input
            // would let any UI save silently clear a value set through the API.
            // TODO(web): surface `cutoffScore` in
            // `settings-quality-profiles-section.tsx` and read it from the input.
            cutoff_score: criteria
                .cutoff_score
                .or_else(|| existing.and_then(|profile| profile.criteria.cutoff_score)),
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
    /// Replaces the authenticated actor's UI settings, retaining the current date-time format when the input omits or nulls it.
    async fn set_my_ui_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Complete UI settings to save; omitted or null date_time_format keeps its current value."
        )]
        input: SetMyUiSettingsInput,
    ) -> GqlResult<UiSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let current = app.get_my_ui_settings(&actor).await.map_err(to_gql_error)?;
        let settings = app
            .set_my_ui_settings(
                &actor,
                ui_settings_update_from_input(input, current.date_time_format),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_ui_settings(settings))
    }

    /// Saves subtitle preferences after checking the catalog-settings permission.
    async fn update_subtitle_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Subtitle settings to validate and save; language flags default to false when omitted."
        )]
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

    /// Saves acquisition thresholds and polling intervals after checking the catalog-settings permission.
    async fn update_acquisition_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Acquisition settings with hour, second, day, count, and score values validated by the application."
        )]
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
                    long_tail_backfill_max_scopes_per_cycle: input
                        .long_tail_backfill_max_scopes_per_cycle,
                    long_tail_reconverge_days: input.long_tail_reconverge_days,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_acquisition_settings(settings))
    }

    /// Saves general system settings after checking the system-settings permission.
    async fn update_general_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "General settings including history retention, image-cache size in MB, and plugin CA bundle text."
        )]
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
                    image_cache_max_size_mb: input.image_cache_max_size_mb,
                    plugin_http_ca_bundle_pem: input.plugin_http_ca_bundle_pem,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_general_settings(settings))
    }

    /// Requests title-image cache clearing and returns the request timestamp after authorization.
    async fn clear_title_image_cache(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ClearTitleImageCachePayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.clear_title_image_cache(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(ClearTitleImageCachePayload {
            requested_at: Utc::now(),
        })
    }

    /// Saves recycle-bin settings after checking the system-settings permission.
    async fn update_recycle_bin_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Recycle-bin settings to save.")] input: UpdateRecycleBinSettingsInput,
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

    /// Saves the automatic official-plugin patch update setting after checking the system-settings permission.
    async fn update_plugin_auto_update_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Whether the scheduled plugin catalog refresh installs official patch updates automatically."
        )]
        input: UpdatePluginAutoUpdateSettingsInput,
    ) -> GqlResult<PluginAutoUpdateSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;

        let settings = app
            .update_plugin_auto_update_settings(
                &actor,
                AppUpdatePluginAutoUpdateSettings {
                    enabled: input.enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_plugin_auto_update_settings(settings))
    }

    /// Saves auto-backup scheduling and key changes after checking the system-settings permission.
    async fn update_auto_backup_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Auto-backup settings; key fields change or clear the stored secret according to their flags."
        )]
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

    /// Saves the custom backup path after checking the system-settings permission.
    async fn update_backup_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Backup path settings to save.")] input: UpdateBackupSettingsInput,
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

    /// Marks the missing auto-backup-key notice as acknowledged and returns refreshed backup status.
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

    /// Saves security and MFA requirements, applies effective login state, and revokes authless OAuth refresh grants when login becomes enabled.
    async fn update_security_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Security settings with password length and MFA requirements validated by the application."
        )]
        input: UpdateSecuritySettingsInput,
    ) -> GqlResult<SecuritySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageUsers).await?;
        if input.api_keys_restrict_to_system_settings_users.is_some() {
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        }
        let previous_snapshot = auth_runtime.snapshot();
        let jellyfin_mfa_requirement = resolve_mfa_route_requirement(
            input.mfa_require_jellyfin_login,
            input.totp_require_jellyfin_login,
            "Jellyfin",
        )?
        .ok_or_else(|| {
            to_gql_error(AppError::Validation(
                "an MFA requirement for Jellyfin login is required".into(),
            ))
        })?;
        let emby_mfa_requirement = resolve_mfa_route_requirement(
            input.mfa_require_emby_login,
            input.totp_require_emby_login,
            "Emby",
        )?;

        let settings = app
            .update_security_settings(
                &actor,
                AppUpdateSecuritySettings {
                    form_login_enabled: input.form_login_enabled,
                    password_min_length: input.password_min_length,
                    skip_login_for_local_ips: input.skip_login_for_local_ips,
                    api_keys_restrict_to_system_settings_users: input
                        .api_keys_restrict_to_system_settings_users,
                    mfa_require_config_step_up: input.mfa_require_config_step_up,
                    mfa_require_password_login: input.mfa_require_password_login,
                    totp_require_jellyfin_login: jellyfin_mfa_requirement,
                    totp_require_emby_login: emby_mfa_requirement,
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

    /// Registers a public OAuth application with exact HTTPS callbacks and mandatory S256 PKCE.
    async fn create_oauth_client_registration(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Display name and exact HTTPS callback allowlist for the application.")]
        input: CreateOAuthClientRegistrationInput,
    ) -> GqlResult<OAuthClientRegistrationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.create_oauth_client_registration(
            &actor,
            CreateOAuthClientRegistration {
                display_name: input.display_name,
                redirect_uris: input.redirect_uris,
                kind: into_oauth_client_kind(input.kind),
            },
        )
        .await
        .map(from_oauth_client_registration)
        .map_err(to_gql_error)
    }

    /// Replaces custom OAuth application metadata and callback allowlist. Disabling revokes all of its grants.
    async fn update_oauth_client_registration(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Immutable client identifier returned during registration.")]
        client_id: String,
        #[graphql(desc = "Replacement public-client configuration.")]
        input: UpdateOAuthClientRegistrationInput,
    ) -> GqlResult<OAuthClientRegistrationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.update_oauth_client_registration(
            &actor,
            &client_id,
            UpdateOAuthClientRegistration {
                display_name: input.display_name,
                redirect_uris: input.redirect_uris,
                enabled: input.enabled,
            },
        )
        .await
        .map(from_oauth_client_registration)
        .map_err(to_gql_error)
    }

    /// Deletes a custom OAuth application and revokes every active grant issued to it.
    async fn delete_oauth_client_registration(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Immutable client identifier returned during registration.")]
        client_id: String,
    ) -> GqlResult<DeleteOAuthClientRegistrationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.delete_oauth_client_registration(&actor, &client_id)
            .await
            .map(|deleted| DeleteOAuthClientRegistrationPayload { client_id, deleted })
            .map_err(to_gql_error)
    }

    /// Creates a media-server connection with defaults for omitted enablement flags and redacted secret fields in the result.
    async fn create_media_server_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Connection details and optional credentials; omitted enabled defaults true while login and linking default false."
        )]
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

    /// Patches a media-server connection by ID, leaving omitted fields unchanged and honoring explicit clear flags for secrets.
    async fn update_media_server_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Connection ID and optional patch fields; omitted fields remain unchanged and clear flags remove stored secrets."
        )]
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

    /// Deletes the media-server connection identified by `id` after authorization.
    async fn delete_media_server_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the media-server connection to delete.")] id: ID,
    ) -> GqlResult<DeleteMediaServerConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        let id = id.to_string();
        app.delete_media_server_connection(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteMediaServerConnectionPayload { id: ID::from(id) })
    }

    /// Validates the selected media-server connection without changing its saved configuration.
    async fn test_media_server_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Connection ID and optional Plex token used only for this validation request."
        )]
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

    /// Discovers Plex servers for the supplied token without saving a connection.
    async fn discover_plex_media_servers(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Plex authentication token used for discovery and not returned by this field."
        )]
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

    /// Discovers Emby Connect servers using credentials without saving a connection.
    async fn discover_emby_connect_servers(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Emby Connect credentials used only for discovery.")]
        input: DiscoverEmbyConnectServersInput,
    ) -> GqlResult<Vec<EmbyConnectServerPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.discover_emby_connect_media_servers(&actor, &input.username_or_email, &input.password)
            .await
            .map(|servers| {
                servers
                    .into_iter()
                    .map(|server| EmbyConnectServerPayload {
                        server_id: server.server_id,
                        name: server.name,
                        user_type: emby_connect_user_type(server.user_type),
                        local_address: server.local_address,
                        remote_address: server.remote_address,
                        local_api_base_url: server.local_api_base_url,
                        remote_api_base_url: server.remote_api_base_url,
                        local_status: emby_connect_address_status(server.local_status),
                        remote_status: emby_connect_address_status(server.remote_status),
                        suggested_base_url: server.suggested_base_url,
                    })
                    .collect()
            })
            .map_err(to_gql_error)
    }

    /// Validates an Emby Connect configuration without changing saved settings.
    async fn test_emby_connect(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Connection ID and Emby Connect credentials used only for this validation request."
        )]
        input: TestEmbyConnectInput,
    ) -> GqlResult<MediaServerConnectionTestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        app.test_emby_connect(
            &actor,
            input.connection_id.as_ref(),
            &input.username_or_email,
            &input.password,
        )
        .await
        .map_err(to_gql_error)?;
        Ok(MediaServerConnectionTestPayload {
            status: "ok".into(),
            message: None,
        })
    }

    /// Creates or updates a delay profile after checking the catalog-settings permission.
    async fn upsert_delay_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Delay profile; delay and age values are expressed in minutes and the ID selects create versus update."
        )]
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
                    enable_usenet: input.enable_usenet.unwrap_or(true),
                    enable_torrent: input.enable_torrent.unwrap_or(true),
                    preferred_protocol: input.preferred_protocol.into_application(),
                    min_age_minutes: input.min_age_minutes as i64,
                    bypass_score_threshold: input.bypass_score_threshold,
                    bypass_if_highest_quality: input.bypass_if_highest_quality.unwrap_or(false),
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

    /// Deletes the delay profile identified by `id` and returns the accepted ID.
    async fn delete_delay_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the delay profile to delete.")] id: ID,
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

    /// Saves media settings for one content scope, with nullable fields passed through as application update semantics.
    async fn update_media_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Content scope and settings update; absent or null optional fields keep their current values, while supplied empty lists are applied as empty."
        )]
        input: UpdateMediaSettingsInput,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        let scope = input.scope;
        let import_mode = parse_import_mode_input(input.import_mode);
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
                use_season_folders: input.use_season_folders,
                folder_template: input.folder_template,
                season_folder_template: input.season_folder_template,
                specials_folder_template: input.specials_folder_template,
                rename_enabled: input.rename_enabled,
                rename_template: input.rename_template,
                rename_collision_policy: input
                    .rename_collision_policy
                    .map(|policy| policy.as_app_str().to_string()),
                rename_missing_metadata_policy: input
                    .rename_missing_metadata_policy
                    .map(|policy| policy.as_app_str().to_string()),
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
            },
        )
        .await
        .map(|settings| from_media_settings(scope, settings))
        .map_err(to_gql_error)
    }

    /// Replace nonblank default library roots for movie, series, and anime facets.
    async fn update_library_paths(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Default facet root paths; blank movie or series values and a null or blank anime value leave that facet unchanged."
        )]
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

    /// Saves service TLS paths after checking the system-settings permission.
    async fn update_service_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "TLS certificate and key paths to save.")]
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

    /// Saves quality profiles and global or facet selections, using profile IDs to match existing profiles.
    async fn save_quality_profile_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Quality profiles and selections; replace_existing controls replacement while profile IDs match existing records."
        )]
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

    /// Replaces download-client routing entries for one content scope after authorization.
    async fn update_download_client_routing(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Content scope and complete routing entry list; an empty list clears entries for that scope."
        )]
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
                        seeding_profile_id: entry.seeding_profile_id.map(|value| value.to_string()),
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

    /// Replaces indexer routing entries for one content scope after authorization.
    async fn update_indexer_routing(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Content scope and complete routing entry list; an empty list clears entries for that scope."
        )]
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

    /// Deletes the quality profile identified by `id` and returns refreshed quality settings.
    async fn delete_quality_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the quality profile to delete.")] id: ID,
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

    /// Starts passkey registration and returns a short-lived challenge without credential material.
    async fn webauthn_register_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = account_security_actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_start(&actor, auth_runtime.snapshot().effective_form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    /// Completes passkey registration using the authorized enrollment challenge.
    async fn webauthn_register_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Challenge ID and WebAuthn response used to complete the pending registration."
        )]
        input: WebauthnRegisterCompleteInput,
    ) -> GqlResult<PasskeySummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = interactive_session_actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_complete(
            &actor,
            input.challenge_id.as_ref(),
            &serde_json::to_string(&input.response_json.0).unwrap_or_default(),
            input.friendly_name,
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map(from_passkey_summary)
        .map_err(to_gql_error)
    }

    /// Starts passkey enrollment for a restricted post-primary MFA-enrollment session.
    async fn webauthn_login_enrollment_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = mfa_enrollment_actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let form_login_enabled = auth_runtime.snapshot().effective_form_login_enabled;
        app.webauthn_login_enrollment_start(&actor, form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    /// Completes passkey enrollment for a restricted post-primary MFA-enrollment session and issues a full session.
    async fn webauthn_login_enrollment_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Passkey registration challenge and browser response for login enrollment."
        )]
        input: WebauthnRegisterCompleteInput,
    ) -> GqlResult<LoginPasskeyEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = mfa_enrollment_actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let form_login_enabled = auth_runtime.snapshot().effective_form_login_enabled;
        let passkey = app
            .webauthn_login_enrollment_complete(
                &actor,
                input.challenge_id.as_ref(),
                &serde_json::to_string(&input.response_json.0).unwrap_or_default(),
                input.friendly_name,
                form_login_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        let login = login_payload_from_user(
            &app,
            actor,
            Some(app.mfa_freshness_verified_until()),
            None,
            mfa.persist_session,
            Some(&mfa.auth_session_version),
            mfa.password_change_required_after_enrollment,
        )
        .await?;
        Ok(LoginPasskeyEnrollmentCompletePayload {
            passkey: from_passkey_summary(passkey),
            login,
        })
    }

    /// Starts TOTP enrollment for the authenticated session or MFA-enrollment session.
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
            totp_enrollment_actor_from_ctx(ctx)?
        };
        let start = if login_enrollment {
            app.start_login_mfa_enrollment(&actor).await
        } else {
            app.totp_enrollment_start(&actor).await
        }
        .map_err(to_gql_error)?;
        Ok(from_totp_enrollment_start(start))
    }

    /// Completes TOTP enrollment with the one-time challenge code and returns status plus recovery codes.
    async fn totp_enrollment_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Enrollment challenge ID and current TOTP code.")]
        input: TotpEnrollmentCompleteInput,
    ) -> GqlResult<TotpEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = totp_management_actor_from_ctx(ctx)?;
        let complete = app
            .totp_enrollment_complete(&actor, input.challenge_id.as_ref(), &input.code)
            .await
            .map_err(to_gql_error)?;
        Ok(from_totp_enrollment_complete(complete))
    }

    /// Completes required login MFA enrollment, returns one-time recovery codes, and issues the authenticated login payload.
    async fn complete_login_mfa_enrollment(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "MFA-enrollment challenge ID and current TOTP code.")]
        input: TotpEnrollmentCompleteInput,
    ) -> GqlResult<LoginMfaEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = mfa_enrollment_actor_from_ctx(ctx)?;
        let complete = app
            .complete_login_mfa_enrollment(&actor, input.challenge_id.as_ref(), &input.code)
            .await
            .map_err(to_gql_error)?;
        let login = login_payload_from_user(
            &app,
            actor,
            Some(app.mfa_freshness_verified_until()),
            None,
            mfa.persist_session,
            Some(&mfa.auth_session_version),
            mfa.password_change_required_after_enrollment,
        )
        .await?;
        Ok(LoginMfaEnrollmentCompletePayload {
            status: from_totp_status(complete.status),
            recovery_codes: complete.recovery_codes,
            login,
        })
    }

    /// Verifies TOTP for a configuration step-up and issues a session with the resulting freshness window.
    async fn mfa_verify_step_up(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Current TOTP code used for step-up verification.")]
        input: TotpVerifyInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = actor_from_ctx(ctx)?;
        let mfa_step_up_verified_until = app
            .mfa_verify_step_up(&actor, &input.code)
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(
            &app,
            actor,
            None,
            Some(mfa_step_up_verified_until),
            mfa.persist_session,
            Some(&mfa.auth_session_version),
            false,
        )
        .await
    }

    /// Verifies the current local password and returns a fresh account-security session.
    async fn account_security_password_verify(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Current local password used to reauthenticate this interactive session."
        )]
        current_password: String,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = interactive_session_actor_from_ctx(ctx)?;
        app.account_security_password_verify(&actor, &current_password)
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(
            &app,
            actor,
            None,
            None,
            mfa.persist_session,
            Some(&mfa.auth_session_version),
            false,
        )
        .await
    }

    /// Starts a passkey assertion that reauthenticates the current interactive session.
    async fn account_security_passkey_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = interactive_session_actor_from_ctx(ctx)?;
        let form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        app.account_security_passkey_start(&actor, form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    /// Completes a passkey reauthentication assertion and returns a fresh account-security session.
    async fn account_security_passkey_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Passkey assertion response for the current account-security challenge.")]
        input: WebauthnCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = interactive_session_actor_from_ctx(ctx)?;
        let form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        app.account_security_passkey_complete(
            &actor,
            input.challenge_id.as_ref(),
            &serde_json::to_string(&input.response_json.0).unwrap_or_default(),
            form_login_enabled,
        )
        .await
        .map_err(to_gql_error)?;
        login_payload_from_user(
            &app,
            actor,
            None,
            None,
            mfa.persist_session,
            Some(&mfa.auth_session_version),
            false,
        )
        .await
    }

    /// Disables TOTP after verifying the current TOTP code.
    async fn totp_disable(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Current TOTP code required to disable TOTP.")] input: TotpVerifyInput,
    ) -> GqlResult<TotpStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = totp_management_actor_from_ctx(ctx)?;
        let expected_auth_session_version = if ctx.data_opt::<AuthlessDefaultSession>().is_some() {
            app.current_actor_auth_session_version(&actor)
                .await
                .map_err(to_gql_error)?
        } else {
            mfa.auth_session_version.clone()
        };
        app.totp_disable(
            &actor,
            &input.code,
            expected_auth_session_version.as_deref(),
        )
        .await
        .map(from_totp_status)
        .map_err(to_gql_error)
    }

    /// Regenerates recovery codes after verifying TOTP; returned codes are one-time secrets.
    async fn totp_regenerate_recovery_codes(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Current TOTP code required to generate a new recovery-code set.")]
        input: TotpVerifyInput,
    ) -> GqlResult<TotpEnrollmentCompletePayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = totp_management_actor_from_ctx(ctx)?;
        let expected_auth_session_version = if ctx.data_opt::<AuthlessDefaultSession>().is_some() {
            app.current_actor_auth_session_version(&actor)
                .await
                .map_err(to_gql_error)?
        } else {
            mfa.auth_session_version.clone()
        };
        let complete = app
            .totp_regenerate_recovery_codes(
                &actor,
                &input.code,
                expected_auth_session_version.as_deref(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_totp_enrollment_complete(complete))
    }

    /// Starts passkey authentication, masking failures when form login is enabled.
    async fn webauthn_authenticate_start(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional compatibility hint that is not used to select or reveal an account; passkeys always start discoverable authentication."
        )]
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

    /// Completes passkey authentication and issues a login payload without exposing the credential response.
    async fn webauthn_authenticate_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Challenge ID and WebAuthn assertion response from the authentication start request."
        )]
        input: WebauthnCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let form_login_enabled = auth_runtime.snapshot().effective_form_login_enabled;
        let started_at = Instant::now();
        let (user, auth_session_version) = match app
            .webauthn_authenticate_complete(
                input.challenge_id.as_ref(),
                &serde_json::to_string(&input.response_json.0).unwrap_or_default(),
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
        let persist_session = persist_session_or_default(
            input.persist_session,
            default_persist_session_from_ctx(ctx),
        );
        login_payload_from_user(
            &app,
            user,
            Some(app.mfa_freshness_verified_until()),
            None,
            persist_session,
            Some(&auth_session_version),
            false,
        )
        .await
    }

    /// Starts the user-bound passkey assertion required after primary credentials succeed.
    async fn login_verification_passkey_start(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Opaque login verification challenge ID returned after primary authentication."
        )]
        challenge_id: ID,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        app.login_verification_passkey_start(challenge_id.as_ref(), form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    /// Completes the passkey assertion required after primary credentials succeed.
    async fn login_verification_passkey_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending login verification and browser assertion response.")]
        input: LoginVerificationPasskeyCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let form_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let (user, verified_until, persist_session, auth_session_version, password_change_required) =
            app.login_verification_passkey_complete(
                input.login_challenge_id.as_ref(),
                input.webauthn_challenge_id.as_ref(),
                &serde_json::to_string(&input.response_json.0).unwrap_or_default(),
                form_login_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(
            &app,
            user,
            Some(verified_until),
            None,
            persist_session,
            Some(&auth_session_version),
            password_change_required,
        )
        .await
    }

    /// Completes the TOTP or recovery-code fallback required after primary credentials succeed.
    async fn login_verification_totp_complete(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Pending login verification challenge and current authenticator or recovery code."
        )]
        input: LoginVerificationTotpCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let (user, verified_until, persist_session, auth_session_version, password_change_required) =
            app.complete_login_verification_totp(input.login_challenge_id.as_ref(), &input.code)
                .await
                .map_err(to_gql_error)?;
        login_payload_from_user(
            &app,
            user,
            Some(verified_until),
            None,
            persist_session,
            Some(&auth_session_version),
            password_change_required,
        )
        .await
    }

    /// Deletes the authenticated actor's passkey identified by `id`.
    async fn delete_my_passkey(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the authenticated actor's passkey to delete.")] id: ID,
    ) -> GqlResult<DeleteMyPasskeyPayload> {
        let app = app_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        let actor = account_security_actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let id_string = id.to_string();
        app.delete_my_passkey(
            &actor,
            &id_string,
            auth_runtime.snapshot().effective_form_login_enabled,
            mfa.auth_session_version.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(DeleteMyPasskeyPayload { id })
    }

    /// Revokes the authenticated actor's OAuth grant identified by `grant_id`.
    async fn revoke_my_oauth_app(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Grant ID of the OAuth app authorization to revoke.")] grant_id: ID,
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

    /// Creates an API key for the interactive actor and returns its secret once.
    async fn create_my_api_key(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Label and expiration policy for the new API key.")]
        input: CreateMyApiKeyInput,
    ) -> GqlResult<CreateMyApiKeyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = api_key_management_actor_from_ctx(ctx)?;
        let mfa = mfa_verification_from_ctx(ctx);
        app.require_api_key_mfa_step_up(&actor, mfa.step_up_verified_until)
            .await
            .map_err(to_gql_error)?;
        let expiry = match input.expiry.unwrap_or(ApiKeyExpiryPresetValue::Days90) {
            ApiKeyExpiryPresetValue::Days30 => AppApiKeyExpiryPreset::Days30,
            ApiKeyExpiryPresetValue::Days90 => AppApiKeyExpiryPreset::Days90,
            ApiKeyExpiryPresetValue::Days365 => AppApiKeyExpiryPreset::Days365,
            ApiKeyExpiryPresetValue::Never => AppApiKeyExpiryPreset::Never,
        };
        let created = app
            .create_api_key(
                &actor,
                CreateApiKey {
                    label: input.label,
                    expiry,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(CreateMyApiKeyPayload {
            api_key: created.raw_key,
            key: from_api_key(created.api_key, &actor.username),
        })
    }

    /// Revokes one user-created API key owned by the interactive actor.
    async fn revoke_my_api_key(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "API-key record ID to revoke.")] id: ID,
    ) -> GqlResult<RevokeMyApiKeyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = api_key_management_actor_from_ctx(ctx)?;
        let id_string = id.to_string();
        let revoked = app
            .revoke_api_key(&actor, &id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(RevokeMyApiKeyPayload { id, revoked })
    }

    /// Authenticates local credentials, optionally verifies required TOTP, and issues a session or MFA-enrollment token.
    async fn login(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Username, password, and optional TOTP code for local authentication.")]
        input: LoginInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let principal = LoginAttemptPrincipal::local(&input.username);
        if let Some(principal) = principal.as_ref()
            && let Some(limiter) = login_attempt_limiter_from_ctx(ctx)
        {
            limiter.check(principal)?;
        }
        let verified = match app
            .authenticate_local_credentials(&input.username, &input.password)
            .await
        {
            Ok(verified) => {
                if let Some(principal) = principal.as_ref()
                    && let Some(limiter) = login_attempt_limiter_from_ctx(ctx)
                {
                    limiter.clear_success(principal);
                }
                verified
            }
            Err(err) => {
                if classify_login_error(&err) == LoginErrorClassification::MaskedPrimaryFailure
                    && let Some(principal) = principal.as_ref()
                    && let Some(limiter) = login_attempt_limiter_from_ctx(ctx)
                {
                    limiter.record_failure(principal);
                }
                return Err(to_login_gql_error("local", err));
            }
        };
        let auth_session_version = verified.auth_session_version;
        let user = verified.user;
        let effective_login_enabled = auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
        let password_login_mfa_required = effective_login_enabled
            && app
                .security_settings()
                .await
                .map_err(to_gql_error)?
                .mfa_require_password_login;
        let persist_session = persist_session_or_default(
            input.persist_session,
            default_persist_session_from_ctx(ctx),
        );
        match app
            .login_verification_requirement(
                &user,
                LoginVerificationMethod::LocalPassword,
                password_login_mfa_required,
                persist_session,
                input.totp_code.as_deref(),
                Some(&auth_session_version),
            )
            .await
            .map_err(to_gql_error)?
        {
            LoginVerificationRequirement::Satisfied(satisfied) => {
                let password_change_required = user.password_change_required;
                login_payload_from_user(
                    &app,
                    user,
                    satisfied.mfa_verified_until,
                    None,
                    persist_session,
                    Some(&satisfied.auth_session_version),
                    password_change_required,
                )
                .await
            }
            LoginVerificationRequirement::EnrollmentRequired {
                auth_session_version,
            } => {
                login_mfa_enrollment_payload_from_user(
                    &app,
                    user.clone(),
                    persist_session,
                    user.password_change_required,
                    &auth_session_version,
                )
                .await
            }
            LoginVerificationRequirement::Challenge(challenge) => {
                Err(login_verification_required_gql_error(
                    &challenge.id,
                    &challenge.expires_at,
                    challenge.allow_passkey,
                    challenge.allow_totp,
                ))
            }
        }
    }

    /// Replaces an administrator-provided temporary password and issues the full session.
    async fn complete_required_password_change(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "New password selected by the authenticated user.")]
        input: CompleteRequiredPasswordChangeInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let claims = mfa_verification_from_ctx(ctx);
        let actor = password_change_required_actor_from_ctx(ctx)?;
        let (user, auth_session_version) = app
            .complete_required_password_change(&actor, input.password, &claims.auth_session_version)
            .await
            .map_err(to_gql_error)?;
        auth_runtime_from_ctx(ctx).invalidate_connections();
        let mfa_verified_until = claims
            .verified_until
            .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0));
        let mfa_step_up_verified_until = claims
            .step_up_verified_until
            .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0));
        login_payload_from_user(
            &app,
            user,
            mfa_verified_until,
            mfa_step_up_verified_until,
            claims.persist_session,
            Some(&auth_session_version),
            false,
        )
        .await
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
