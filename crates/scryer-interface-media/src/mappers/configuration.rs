use super::*;

fn provider_config_field_payload_options(
    field: &ConfigFieldDef,
) -> Vec<PluginConfigFieldOptionPayload> {
    field
        .options
        .iter()
        .map(|option| PluginConfigFieldOptionPayload {
            value: option.value.clone(),
            label: option.label.clone(),
            config_overrides: option
                .config_overrides
                .iter()
                .map(|(key, value)| PluginConfigFieldOverridePayload {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect(),
        })
        .collect()
}

fn provider_config_value_payload(
    key: String,
    value: Option<&Value>,
    field: Option<&ConfigFieldDef>,
) -> ProviderConfigValuePayload {
    let field_is_secret = field.is_some_and(|field| field.field_type == ConfigFieldType::Password)
        || looks_like_secret_config_key(&key);
    let secret_stored = field_is_secret
        && value.is_some_and(|value| match value {
            Value::String(value) => !value.trim().is_empty(),
            Value::Null => false,
            _ => true,
        });

    let typed_value = if field_is_secret {
        Some(ProviderConfigFieldValue::Secret(SecretConfigValuePayload {
            stored: secret_stored,
        }))
    } else {
        match value {
            Some(Value::Bool(value)) => {
                Some(ProviderConfigFieldValue::Bool(BoolConfigValuePayload {
                    value: *value,
                }))
            }
            Some(Value::Number(value)) => {
                let int_value = value.as_i64().or_else(|| {
                    value
                        .as_u64()
                        .and_then(|unsigned| i64::try_from(unsigned).ok())
                });
                match int_value {
                    Some(value) => Some(ProviderConfigFieldValue::Int(IntConfigValuePayload {
                        value,
                    })),
                    None => value.as_f64().map(|value| {
                        ProviderConfigFieldValue::Float(FloatConfigValuePayload { value })
                    }),
                }
            }
            Some(Value::String(value)) => {
                Some(ProviderConfigFieldValue::String(StringConfigValuePayload {
                    value: value.clone(),
                }))
            }
            _ => None,
        }
    };

    ProviderConfigValuePayload {
        key,
        label: field.map(|field| field.label.clone()),
        field_type: field.map(|field| PluginConfigFieldTypeValue::from_domain(field.field_type)),
        required: field.is_some_and(|field| field.required),
        default_value: field.and_then(|field| field.default_value.clone()),
        value_source: field
            .map(|field| PluginConfigValueSourceValue::from_domain(field.value_source)),
        role: field.and_then(|field| field.role.map(PluginConfigFieldRoleValue::from_domain)),
        host_binding: field.and_then(|field| {
            field
                .host_binding
                .map(|binding| binding.as_str().to_string())
        }),
        options: field
            .map(provider_config_field_payload_options)
            .unwrap_or_default(),
        help_text: field.and_then(|field| field.help_text.clone()),
        value: typed_value,
    }
}

pub fn provider_config_values_from_json(raw: Option<&str>) -> Vec<ProviderConfigValuePayload> {
    provider_config_values_from_json_with_fields(raw, &[])
}

pub fn provider_config_values_from_json_with_fields(
    raw: Option<&str>,
    fields: &[ConfigFieldDef],
) -> Vec<ProviderConfigValuePayload> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };

    let field_by_key = fields
        .iter()
        .map(|field| (field.key.as_str(), field))
        .collect::<std::collections::HashMap<_, _>>();
    let mut values = fields
        .iter()
        .filter(|field| object.contains_key(&field.key))
        .map(|field| {
            provider_config_value_payload(field.key.clone(), object.get(&field.key), Some(field))
        })
        .collect::<Vec<_>>();
    let mut unknown_values = object
        .iter()
        .filter(|(key, _)| !field_by_key.contains_key(key.as_str()))
        .map(|(key, value)| provider_config_value_payload(key.clone(), Some(value), None))
        .collect::<Vec<_>>();
    unknown_values.sort_by(|left, right| left.key.cmp(&right.key));
    values.extend(unknown_values);
    values
}

pub fn provider_config_values_to_json(
    values: Vec<ProviderConfigValueInput>,
) -> scryer_application::AppResult<String> {
    let mut object = serde_json::Map::new();

    for value in values {
        let key = value.key.trim();
        if key.is_empty() {
            return Err(scryer_application::AppError::Validation(
                "config value key is required".to_string(),
            ));
        }

        let provided_count = [
            value.string_value.is_some(),
            value.bool_value.is_some(),
            value.int_value.is_some(),
            value.float_value.is_some(),
            value.secret_value.is_some(),
            value.clear_secret.unwrap_or(false),
        ]
        .into_iter()
        .filter(|provided| *provided)
        .count();

        if provided_count == 0 {
            continue;
        }
        if provided_count > 1 {
            return Err(scryer_application::AppError::Validation(format!(
                "config value '{key}' must provide exactly one value"
            )));
        }

        let json_value = if value.clear_secret.unwrap_or(false) {
            Value::Null
        } else if let Some(raw) = value.secret_value {
            Value::String(raw)
        } else if let Some(raw) = value.string_value {
            Value::String(raw)
        } else if let Some(raw) = value.bool_value {
            Value::Bool(raw)
        } else if let Some(raw) = value.int_value {
            Value::Number(raw.into())
        } else {
            serde_json::Number::from_f64(value.float_value.unwrap_or_default())
                .map(Value::Number)
                .ok_or_else(|| {
                    scryer_application::AppError::Validation(format!(
                        "config value '{key}' has an invalid float value"
                    ))
                })?
        };

        object.insert(key.to_string(), json_value);
    }

    Ok(Value::Object(object).to_string())
}

pub(super) fn support_tier_label(value: PluginSupportTier) -> String {
    match value {
        PluginSupportTier::Official => "official".to_string(),
        PluginSupportTier::VerifiedCommunity => "verified_community".to_string(),
        PluginSupportTier::Unverified => "unverified".to_string(),
    }
}

pub fn from_scoring_overrides(
    overrides: scryer_application::ScoringOverrides,
) -> ScoringOverridesPayload {
    ScoringOverridesPayload {
        allow_x265_non4k: overrides.allow_x265_non4k,
        block_dv_without_fallback: overrides.block_dv_without_fallback,
        prefer_compact_encodes: overrides.prefer_compact_encodes,
        prefer_lossless_audio: overrides.prefer_lossless_audio,
        block_upscaled: overrides.block_upscaled,
    }
}

pub fn from_quality_profile_criteria(
    criteria: QualityProfileCriteria,
) -> QualityProfileCriteriaPayload {
    QualityProfileCriteriaPayload {
        quality_tiers: criteria.quality_tiers,
        archival_quality: criteria.archival_quality,
        allow_unknown_quality: criteria.allow_unknown_quality,
        source_allowlist: criteria
            .source_allowlist
            .into_iter()
            .map(|source| source.to_string())
            .collect(),
        source_blocklist: criteria
            .source_blocklist
            .into_iter()
            .map(|source| source.to_string())
            .collect(),
        video_codec_allowlist: criteria
            .video_codec_allowlist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        video_codec_blocklist: criteria
            .video_codec_blocklist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        audio_codec_allowlist: criteria
            .audio_codec_allowlist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        audio_codec_blocklist: criteria
            .audio_codec_blocklist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        dolby_vision_allowed: criteria.dolby_vision_allowed,
        detected_hdr_allowed: criteria.detected_hdr_allowed,
        prefer_remux: criteria.prefer_remux,
        allow_bd_disk: criteria.allow_bd_disk,
        allow_upgrades: criteria.allow_upgrades,
        scoring_overrides: from_scoring_overrides(criteria.scoring_overrides),
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
        cutoff_score: criteria.cutoff_score,
    }
}

pub fn from_quality_profile(profile: QualityProfile) -> QualityProfilePayload {
    QualityProfilePayload {
        id: profile.id.into(),
        name: profile.name,
        criteria: from_quality_profile_criteria(profile.criteria),
    }
}

pub fn from_library_paths_settings(settings: LibraryPathsSettings) -> LibraryPathsPayload {
    LibraryPathsPayload {
        movie_path: settings.movie_path,
        series_path: settings.series_path,
        anime_path: settings.anime_path,
    }
}

pub fn from_service_settings(settings: ServiceSettings) -> ServiceSettingsPayload {
    ServiceSettingsPayload {
        tls_cert_path: settings.tls_cert_path,
        tls_key_path: settings.tls_key_path,
    }
}

pub fn from_download_client_routing_entry(
    entry: DownloadClientRoutingSettingsEntry,
) -> DownloadClientRoutingEntryPayload {
    DownloadClientRoutingEntryPayload {
        client_id: entry.client_id.into(),
        enabled: entry.enabled,
        category: entry.category,
        recent_queue_priority: entry.recent_queue_priority,
        older_queue_priority: entry.older_queue_priority,
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
        seeding_profile_id: entry.seeding_profile_id.map(Into::into),
    }
}

pub fn season_pack_seed_mode_value(mode: SeasonPackSeedMode) -> SeasonPackSeedModeValue {
    match mode {
        SeasonPackSeedMode::Inherit => SeasonPackSeedModeValue::Inherit,
        SeasonPackSeedMode::Override => SeasonPackSeedModeValue::Override,
    }
}

pub fn season_pack_seed_mode_from_value(value: SeasonPackSeedModeValue) -> SeasonPackSeedMode {
    match value {
        SeasonPackSeedModeValue::Inherit => SeasonPackSeedMode::Inherit,
        SeasonPackSeedModeValue::Override => SeasonPackSeedMode::Override,
    }
}

pub fn seed_goal_met_action_value(action: SeedGoalMetAction) -> SeedGoalMetActionValue {
    match action {
        SeedGoalMetAction::RemoveEntry => SeedGoalMetActionValue::RemoveEntry,
        SeedGoalMetAction::StopSeeding => SeedGoalMetActionValue::StopSeeding,
        SeedGoalMetAction::Keep => SeedGoalMetActionValue::Keep,
    }
}

pub fn seed_goal_met_action_from_value(value: SeedGoalMetActionValue) -> SeedGoalMetAction {
    match value {
        SeedGoalMetActionValue::RemoveEntry => SeedGoalMetAction::RemoveEntry,
        SeedGoalMetActionValue::StopSeeding => SeedGoalMetAction::StopSeeding,
        SeedGoalMetActionValue::Keep => SeedGoalMetAction::Keep,
    }
}

pub fn post_import_tracking_value(mode: PostImportTracking) -> PostImportTrackingValue {
    match mode {
        PostImportTracking::Park => PostImportTrackingValue::Park,
        PostImportTracking::HandOff => PostImportTrackingValue::HandOff,
    }
}

pub fn post_import_tracking_from_value(value: PostImportTrackingValue) -> PostImportTracking {
    match value {
        PostImportTrackingValue::Park => PostImportTracking::Park,
        PostImportTrackingValue::HandOff => PostImportTracking::HandOff,
    }
}

pub fn from_seeding_profile(profile: SeedingProfile) -> SeedingProfilePayload {
    SeedingProfilePayload {
        id: profile.id.into(),
        name: profile.name,
        ratio: profile.ratio,
        seed_time_minutes: profile.seed_time_minutes,
        season_pack_mode: season_pack_seed_mode_value(profile.season_pack_mode),
        season_pack_ratio: profile.season_pack_ratio,
        season_pack_seed_time_minutes: profile.season_pack_seed_time_minutes,
        honor_tracker_minimums: profile.honor_tracker_minimums,
        goal_met_action: seed_goal_met_action_value(profile.goal_met_action),
        never_remove: profile.never_remove,
        minimum_seeders: profile.minimum_seeders,
        post_import_tracking: post_import_tracking_value(profile.post_import_tracking),
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    }
}

pub fn from_indexer_routing_entry(
    entry: IndexerRoutingSettingsEntry,
) -> IndexerRoutingEntryPayload {
    IndexerRoutingEntryPayload {
        indexer_id: entry.indexer_id.into(),
        enabled: entry.enabled,
        categories: entry.categories,
        priority: entry.priority,
    }
}

pub fn from_library_settings(settings: LibrarySettings) -> LibrarySettingsPayload {
    LibrarySettingsPayload {
        required_audio_languages_override: settings.required_audio_languages_override,
        required_audio_languages: settings.required_audio_languages,
        metadata_language_override: settings.metadata_language_override,
        metadata_language: settings.metadata_language,
        use_season_folders_override: settings.use_season_folders_override,
        use_season_folders: settings.use_season_folders,
        quality_profile_id_override: settings.quality_profile_id_override.map(Into::into),
        quality_profile_id: settings.quality_profile_id.into(),
        request_quality_profile_ids_override: settings
            .request_quality_profile_ids_override
            .map(|ids| ids.into_iter().map(Into::into).collect()),
        request_quality_profile_ids: settings
            .request_quality_profile_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        request_quality_profile_default_id: settings.request_quality_profile_default_id.into(),
        scoring_persona_override: settings
            .scoring_persona_override
            .map(ScoringPersonaValue::from_application),
        scoring_persona: ScoringPersonaValue::from_application(settings.scoring_persona),
        filler_policy_override: settings
            .filler_policy_override
            .as_deref()
            .and_then(FillerPolicyValue::from_app_str),
        filler_policy: settings
            .filler_policy
            .as_deref()
            .and_then(FillerPolicyValue::from_app_str),
        recap_policy_override: settings
            .recap_policy_override
            .as_deref()
            .and_then(RecapPolicyValue::from_app_str),
        recap_policy: settings
            .recap_policy
            .as_deref()
            .and_then(RecapPolicyValue::from_app_str),
        monitor_specials_override: settings.monitor_specials_override,
        monitor_specials: settings.monitor_specials,
        inter_season_movies_override: settings.inter_season_movies_override,
        inter_season_movies: settings.inter_season_movies,
        monitor_filler_movies_override: settings.monitor_filler_movies_override,
        monitor_filler_movies: settings.monitor_filler_movies,
        nfo_write_on_import_override: settings.nfo_write_on_import_override,
        nfo_write_on_import: settings.nfo_write_on_import,
        plexmatch_write_on_import_override: settings.plexmatch_write_on_import_override,
        plexmatch_write_on_import: settings.plexmatch_write_on_import,
        import_mode_override: settings.import_mode_override.map(Into::into),
        import_mode: settings.import_mode.into(),
        set_permissions_linux_override: settings.set_permissions_linux_override,
        set_permissions_linux: settings.set_permissions_linux,
        file_chmod_override: settings.file_chmod_override,
        file_chmod: settings.file_chmod,
        folder_chmod_override: settings.folder_chmod_override,
        folder_chmod: settings.folder_chmod,
        chown_group_override: settings.chown_group_override,
        chown_group: settings.chown_group,
        indexer_routing_override: settings.indexer_routing_override.map(|entries| {
            entries
                .into_iter()
                .map(from_indexer_routing_entry)
                .collect()
        }),
        download_client_routing_override: settings.download_client_routing_override.map(
            |entries| {
                entries
                    .into_iter()
                    .map(from_download_client_routing_entry)
                    .collect()
            },
        ),
    }
}

fn from_quality_scope(facet: MediaFacet) -> ContentScopeValue {
    match facet {
        MediaFacet::Movie => ContentScopeValue::Movie,
        MediaFacet::Series => ContentScopeValue::Series,
        MediaFacet::Anime => ContentScopeValue::Anime,
    }
}

fn from_quality_profile_selection(
    selection: QualityProfileSelection,
) -> QualityProfileSelectionPayload {
    QualityProfileSelectionPayload {
        scope: from_quality_scope(selection.facet),
        override_profile_id: selection.override_profile_id.map(Into::into),
        effective_profile_id: selection.effective_profile_id.into(),
        inherits_global: selection.inherits_global,
    }
}

fn from_facet_scoring_persona_selection(
    selection: FacetScoringPersonaSelection,
) -> FacetScoringPersonaSelectionPayload {
    FacetScoringPersonaSelectionPayload {
        scope: from_quality_scope(selection.facet),
        override_persona: selection
            .override_persona
            .map(ScoringPersonaValue::from_application),
        effective_persona: ScoringPersonaValue::from_application(selection.effective_persona),
        inherits_global: selection.inherits_global,
    }
}

pub fn from_media_settings(
    scope: ContentScopeValue,
    settings: MediaSettings,
) -> MediaSettingsPayload {
    MediaSettingsPayload {
        scope,
        library_path: settings.library_path,
        root_folders: settings
            .root_folders
            .into_iter()
            .map(|entry| RootFolderPayload {
                path: entry.path,
                is_default: entry.is_default,
            })
            .collect(),
        required_audio_languages: settings.required_audio_languages,
        use_season_folders: settings.use_season_folders,
        folder_template: settings.folder_template,
        season_folder_template: settings.season_folder_template,
        specials_folder_template: settings.specials_folder_template,
        rename_enabled: settings.rename_enabled,
        rename_template: settings.rename_template,
        rename_collision_policy: RenameCollisionPolicyValue::from_app_str(
            &settings.rename_collision_policy,
        )
        .unwrap_or(RenameCollisionPolicyValue::Skip),
        rename_missing_metadata_policy: RenameMissingMetadataPolicyValue::from_app_str(
            &settings.rename_missing_metadata_policy,
        )
        // Match the application-layer default (DEFAULT_MISSING_METADATA_POLICY):
        // an unparseable stored value must not flip the semantic to Skip.
        .unwrap_or(RenameMissingMetadataPolicyValue::FallbackTitle),
        filler_policy: settings
            .filler_policy
            .as_deref()
            .and_then(FillerPolicyValue::from_app_str),
        recap_policy: settings
            .recap_policy
            .as_deref()
            .and_then(RecapPolicyValue::from_app_str),
        monitor_specials: settings.monitor_specials,
        inter_season_movies: settings.inter_season_movies,
        monitor_filler_movies: settings.monitor_filler_movies,
        nfo_write_on_import: settings.nfo_write_on_import,
        plexmatch_write_on_import: settings.plexmatch_write_on_import,
        import_mode: settings.import_mode.into(),
        set_permissions_linux: settings.set_permissions_linux,
        file_chmod: settings.file_chmod,
        folder_chmod: settings.folder_chmod,
        chown_group: settings.chown_group,
    }
}

pub fn from_quality_profile_settings(
    settings: QualityProfileSettings,
) -> QualityProfileSettingsPayload {
    QualityProfileSettingsPayload {
        profiles: settings
            .profiles
            .into_iter()
            .map(from_quality_profile)
            .collect(),
        global_profile_id: settings.global_profile_id.into(),
        global_scoring_persona: ScoringPersonaValue::from_application(
            settings.global_scoring_persona,
        ),
        category_selections: settings
            .category_selections
            .into_iter()
            .map(from_quality_profile_selection)
            .collect(),
        category_persona_selections: settings
            .category_persona_selections
            .into_iter()
            .map(from_facet_scoring_persona_selection)
            .collect(),
    }
}

pub fn from_indexer_config_with_fields(
    config: IndexerConfig,
    config_fields: &[ConfigFieldDef],
) -> IndexerConfigPayload {
    let is_managed = config.managed_parent_config_id.is_some();
    let managed_parent_config_id = config.managed_parent_config_id.clone();
    let supports_managed_children_sync = config.provider_type.eq_ignore_ascii_case("prowlarr");
    // Computed before `config` is picked apart below. Goals only: the dropdown
    // says "Managed by Prowlarr" about *seed goals*, so a child carrying nothing
    // but an imported `appMinimumSeeders` must not claim them.
    let has_prowlarr_seed_criteria =
        scryer_application::prowlarr_managed_goal_profile(&config).is_some();
    // The admission half of the same split, read back verbatim: the resolver
    // answers admission from this value at the Prowlarr tier, so the row has to
    // be able to say so instead of reading "Inherit default".
    let prowlarr_minimum_seeders = scryer_application::prowlarr_managed_minimum_seeders(&config);
    let (config_json, stored_secret_keys) =
        redact_indexer_config_json(config.config_json, config_fields);
    let has_api_key = stored_secret_keys.iter().any(|key| key == "api_key")
        || config
            .api_key_encrypted
            .as_ref()
            .is_some_and(|value| !value.is_empty());
    IndexerConfigPayload {
        id: config.id.into(),
        name: config.name,
        provider_type: config.provider_type,
        base_url: config.base_url,
        proxy_config_id: config.proxy_config_id.map(Into::into),
        download_client_id: config.download_client_id.map(Into::into),
        seeding_profile_id: config.seeding_profile_id.map(Into::into),
        has_prowlarr_seed_criteria,
        prowlarr_minimum_seeders,
        has_api_key,
        is_managed,
        managed_parent_config_id: managed_parent_config_id.map(Into::into),
        supports_managed_children_sync,
        stored_secret_keys,
        rate_limit_seconds: config.rate_limit_seconds,
        rate_limit_burst: config.rate_limit_burst,
        disabled_until: config.disabled_until,
        is_enabled: config.is_enabled,
        enable_interactive_search: config.enable_interactive_search,
        enable_auto_search: config.enable_auto_search,
        last_health_status: config.last_health_status,
        last_error_message: config.last_error_message,
        last_error_at: config.last_error_at,
        last_query_at: None,
        config: provider_config_values_from_json_with_fields(config_json.as_deref(), config_fields),
        created_at: config.created_at,
        updated_at: config.updated_at,
    }
}

pub fn from_proxy_config(config: ProxyConfig) -> ProxyConfigPayload {
    // Credentials are write-only, exactly like `IndexerConfig.api_key_encrypted`:
    // the payload says whether one is stored, never what it is.
    let has_credentials = [
        config.username_encrypted.as_deref(),
        config.password_encrypted.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !value.is_empty());
    ProxyConfigPayload {
        id: config.id.into(),
        name: config.name,
        provider_type: config.provider_type.as_str().to_string(),
        protocol: config
            .protocol
            .map(|protocol| protocol.as_str().to_string()),
        base_url: config.base_url,
        request_timeout_seconds: i32::try_from(config.request_timeout_seconds).unwrap_or(i32::MAX),
        has_credentials,
        remote_dns: config.remote_dns,
        // Same write-only treatment as the credentials. The host key is the
        // exception: it is public, and an operator has to be able to read it to
        // compare it with their server.
        has_private_key: config
            .private_key_encrypted
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
        // Both WireGuard public keys are shown in full, for the same reason
        // the host key is: the peer's is what the operator pasted out of their
        // server, ours is what they must paste back into it, and masking
        // either would hide the only two values that make a key mistake
        // diagnosable. The preshared key *is* a secret and gets the write-only
        // treatment.
        peer_public_key: config.peer_public_key,
        has_preshared_key: config
            .preshared_key_encrypted
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
        tunnel_public_key: config.tunnel_public_key,
        tunnel_addresses: config.tunnel_addresses,
        tunnel_dns_servers: config.tunnel_dns_servers,
        tunnel_mtu: config.tunnel_mtu.map(i32::from),
        tunnel_keepalive_seconds: config.tunnel_keepalive_seconds.map(i32::from),
        host_key_fingerprint: config.host_key_fingerprint,
        host_key_pinned_at: config.host_key_pinned_at,
        is_enabled: config.is_enabled,
        last_health_status: config
            .last_health_status
            .map(|status| status.as_str().to_string()),
        last_error_message: config.last_error_message,
        last_error_at: config.last_error_at,
        created_at: config.created_at,
        updated_at: config.updated_at,
    }
}

pub fn from_proxy_test_result(result: ProxyTestResult) -> ProxyTestResultPayload {
    ProxyTestResultPayload {
        ok: result.ok,
        status: result.status.as_str().to_string(),
        message: result.message,
        duration_ms: result
            .duration_ms
            .map(|value| value.min(i32::MAX as u64) as i32),
    }
}

pub fn from_indexer_config_sync_result(
    result: scryer_application::IndexerConfigSyncResult,
) -> IndexerConfigSyncPayload {
    IndexerConfigSyncPayload {
        parent_config_id: result.parent_config_id.into(),
        created_ids: result.created_ids.into_iter().map(Into::into).collect(),
        updated_ids: result.updated_ids.into_iter().map(Into::into).collect(),
        deleted_ids: result.deleted_ids.into_iter().map(Into::into).collect(),
    }
}

fn redact_indexer_config_json(
    config_json: Option<String>,
    config_fields: &[ConfigFieldDef],
) -> (Option<String>, Vec<String>) {
    let Some(raw) = config_json else {
        return (None, Vec::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return (Some(raw), Vec::new());
    };
    let Some(object) = value.as_object() else {
        return (Some(raw), Vec::new());
    };

    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut stored_secret_keys = object
        .iter()
        .filter_map(|(key, value)| {
            let is_secret =
                configured_secret_keys.contains(key.as_str()) || indexer_config_key_is_secret(key);
            if !is_secret {
                return None;
            }
            match value {
                serde_json::Value::String(value) if !value.trim().is_empty() => Some(key.clone()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    stored_secret_keys.sort();
    stored_secret_keys.dedup();

    (Some(raw), stored_secret_keys)
}

fn indexer_config_key_is_secret(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    normalized.contains("apikey")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.ends_with("token")
}

#[expect(
    clippy::too_many_arguments,
    reason = "provider type payload is assembled from discrete application fields"
)]
pub fn from_provider_type(
    provider_type: String,
    name: String,
    config_fields: Vec<scryer_domain::ConfigFieldDef>,
    default_base_url: Option<String>,
    available_host_bindings: Vec<String>,
    recommended_facets: Vec<String>,
    supported_events: Vec<String>,
    supports_test: bool,
) -> ProviderTypePayload {
    ProviderTypePayload {
        provider_type,
        name,
        default_base_url,
        available_host_bindings,
        recommended_facets: recommended_facets
            .into_iter()
            .filter_map(|facet| MediaFacetValue::parse(&facet))
            .collect(),
        supported_events,
        supports_test,
        config_fields: config_fields
            .into_iter()
            .map(|f| PluginConfigFieldPayload {
                key: f.key,
                label: f.label,
                field_type: PluginConfigFieldTypeValue::from_domain(f.field_type),
                required: f.required,
                default_value: f.default_value,
                value_source: PluginConfigValueSourceValue::from_domain(f.value_source),
                role: f.role.map(PluginConfigFieldRoleValue::from_domain),
                host_binding: f.host_binding.map(|binding| binding.as_str().to_string()),
                options: f
                    .options
                    .into_iter()
                    .map(|o| PluginConfigFieldOptionPayload {
                        value: o.value,
                        label: o.label,
                        config_overrides: o
                            .config_overrides
                            .into_iter()
                            .map(|(key, value)| PluginConfigFieldOverridePayload { key, value })
                            .collect(),
                    })
                    .collect(),
                help_text: f.help_text,
                visible_when: f.visible_when.map(PluginFieldConditionPayload::from_domain),
                required_when: f
                    .required_when
                    .map(PluginFieldConditionPayload::from_domain),
                advanced: f.advanced,
            })
            .collect(),
    }
}

pub fn from_download_client_config(config: DownloadClientConfig) -> DownloadClientConfigPayload {
    from_download_client_config_with_fields(config, &[])
}

pub fn from_download_client_config_with_fields(
    config: DownloadClientConfig,
    config_fields: &[ConfigFieldDef],
) -> DownloadClientConfigPayload {
    let base_url =
        scryer_application::resolve_download_client_base_url_from_config_json(&config.config_json);
    let stored_secret_keys =
        stored_secret_keys_from_config_json(&config.config_json, config_fields);
    DownloadClientConfigPayload {
        id: config.id.into(),
        name: config.name,
        client_type: config.client_type,
        base_url,
        config: provider_config_values_from_json_with_fields(
            Some(&config.config_json),
            config_fields,
        ),
        stored_secret_keys,
        is_enabled: config.is_enabled,
        status: config.status.as_str().to_string(),
        last_error: config.last_error,
        last_seen_at: config.last_seen_at,
        proxy_config_id: config.proxy_config_id.map(Into::into),
        created_at: config.created_at,
        updated_at: config.updated_at,
    }
}

pub fn from_subtitle_provider_config(
    config: SubtitleProviderConfig,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> SubtitleProviderConfigPayload {
    let secret_keys = config_fields
        .iter()
        .filter(|field| matches!(field.field_type, scryer_domain::ConfigFieldType::Password))
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();

    let has_config = serde_json::from_str::<Value>(&config.config_json)
        .ok()
        .is_some_and(|value| match value {
            Value::Object(map) => !map.is_empty(),
            Value::Null => false,
            _ => true,
        });

    let stored_secret_keys = serde_json::from_str::<Value>(&config.config_json)
        .ok()
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    let is_secret =
                        secret_keys.contains(key.as_str()) || looks_like_secret_config_key(key);
                    if is_secret
                        && !value.is_null()
                        && value.as_str().is_none_or(|value| !value.trim().is_empty())
                    {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    SubtitleProviderConfigPayload {
        id: config.id.into(),
        name: config.name,
        provider_type: config.provider_type,
        has_config,
        stored_secret_keys,
        enabled_facets: config
            .enabled_facets
            .iter()
            .filter_map(|facet| MediaFacetValue::parse(facet))
            .collect(),
        is_enabled: config.is_enabled,
        last_health_status: config.last_health_status,
        last_error: config.last_error,
        last_error_at: config.last_error_at,
        disabled_until: config.disabled_until,
        created_at: config.created_at,
        updated_at: config.updated_at,
    }
}

fn looks_like_secret_config_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "username"
        || normalized == "user_name"
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

pub(super) fn stored_secret_keys_from_config_json(
    raw: &str,
    config_fields: &[ConfigFieldDef],
) -> Vec<String> {
    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut keys = object
        .iter()
        .filter_map(|(key, value)| {
            let is_secret =
                configured_secret_keys.contains(key.as_str()) || looks_like_secret_config_key(key);
            let has_value = match value {
                Value::String(value) => !value.trim().is_empty(),
                Value::Null => false,
                _ => true,
            };
            (is_secret && has_value).then(|| key.clone())
        })
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn config_option_payload_preserves_preset_overrides() {
        let field = ConfigFieldDef {
            key: "profile_id".to_string(),
            label: "Known provider".to_string(),
            field_type: ConfigFieldType::Select,
            required: false,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: vec![scryer_domain::ConfigFieldOption {
                value: "preset".to_string(),
                label: "Preset".to_string(),
                config_overrides: std::collections::BTreeMap::from([
                    ("api_path".to_string(), "/api".to_string()),
                    (
                        "base_url".to_string(),
                        "https://api.example.test".to_string(),
                    ),
                ]),
            }],
            help_text: None,
            ..Default::default()
        };

        let options = provider_config_field_payload_options(&field);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].config_overrides.len(), 2);
        assert_eq!(options[0].config_overrides[0].key, "api_path");
        assert_eq!(options[0].config_overrides[0].value, "/api");
        assert_eq!(options[0].config_overrides[1].key, "base_url");
    }
}
