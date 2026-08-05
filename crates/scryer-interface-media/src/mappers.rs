use crate::types::*;
use async_graphql::ID;
use chrono::{DateTime, Utc};
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    ActivityEvent, AppUseCase, BackupInfo, CatalogDiscoveryGroup, CatalogDiscoveryGroupKind,
    CatalogDiscoveryQuery, CatalogDiscoveryResult, CatalogDiscoverySurface, DeletePreview,
    DiscoveryFacetRecord, DiscoveryHomeFilterOptions, DiscoveryHomeFilters, DiscoveryHomeQuery,
    DiscoveryHomeResult, DiscoveryItemDetailQuery, DiscoveryItemRecord, DiscoveryItemsQuery,
    DiscoveryItemsResult, DiscoverySectionResult, DiscoverySyncStateRecord, DiscoverySyncStatus,
    DownloadClientRoutingSettingsEntry, FacetScoringPersonaSelection, IgnorePendingImportResult,
    ImageProxyKind, IndexerProxyTestResult, IndexerRoutingSettingsEntry, IndexerSearchResult,
    JobDefinition, JobRun, LibraryPathsSettings, LibraryScanSummary, LibrarySettings,
    ManualPluginPreview, MediaRequestCounts, MediaSettings, ParsedEpisodeMetadata,
    ParsedReleaseMetadata, PendingImportConnection, PendingImportCounts, PendingImportItem,
    PendingImportSearchAttempt, PendingRelease, PluginCatalogStatus, QualityProfile,
    QualityProfileCriteria, QualityProfileDecision, QualityProfileSelection,
    QualityProfileSettings, RegistryPlugin, RenameApplyItemResult, RenameApplyResult, RenamePlan,
    RenamePlanItem, ResolvePendingImportResult, RssSyncReport, ScoringEntry, ScoringSource,
    ServiceSettings, SmgScryerUpdateNotice, SmgVersionCompatibilityNotice, SubmissionScope,
    SystemHealth, TitleHistoryPage, TitleRatingSummary, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    CalendarEpisode, Collection, ConfigFieldDef, ConfigFieldType, DomainEvent,
    DownloadClientConfig, DownloadQueueItem, Episode, IndexerConfig, IndexerProxyConfig, Library,
    MediaFacet, MediaRequest, PluginInstallation, PluginSupportTier, RuleSet,
    SubtitleProviderConfig, Title, TitleHistoryRecord, User,
};
use scryer_rules;
use serde_json::Value;

pub fn parse_iso_date(value: Option<String>) -> Option<Date> {
    value.and_then(|value| Date::parse_iso(&value).ok())
}

fn parse_date(value: Option<String>) -> Option<Date> {
    parse_iso_date(value)
}

fn parse_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid {field} timestamp: {error}"))
}

fn parse_required_datetime(value: &str, field: &str) -> DateTime<Utc> {
    parse_datetime(value, field).expect("Scryer-owned timestamp should be RFC3339")
}

fn parse_optional_datetime(value: Option<String>, field: &str) -> Option<DateTime<Utc>> {
    value.and_then(|value| parse_datetime(&value, field).ok())
}

fn provider_config_field_payload_options(
    field: &ConfigFieldDef,
) -> Vec<PluginConfigFieldOptionPayload> {
    field
        .options
        .iter()
        .map(|option| PluginConfigFieldOptionPayload {
            value: option.value.clone(),
            label: option.label.clone(),
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

fn support_tier_label(value: PluginSupportTier) -> String {
    match value {
        PluginSupportTier::Official => "official".to_string(),
        PluginSupportTier::VerifiedCommunity => "verified_community".to_string(),
        PluginSupportTier::Unverified => "unverified".to_string(),
    }
}

fn import_facet_from_payload(payload: &Value) -> Option<MediaFacetValue> {
    let parameters = payload.get("parameters")?.as_array()?;
    for parameter in parameters {
        let (key, value) = match parameter {
            Value::Array(values) => (
                values.first().and_then(Value::as_str),
                values.get(1).and_then(Value::as_str),
            ),
            Value::Object(_) => (
                parameter.get("key").and_then(Value::as_str),
                parameter.get("value").and_then(Value::as_str),
            ),
            _ => (None, None),
        };
        let Some(key) = key else {
            continue;
        };
        if key != "*scryer_facet" {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        return match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(MediaFacetValue::Movie),
            "series" => Some(MediaFacetValue::Series),
            "anime" => Some(MediaFacetValue::Anime),
            _ => None,
        };
    }
    None
}

fn path_basename(path: &str) -> Option<String> {
    let path = stored_path_to_path_buf(path.trim());
    let display = path.to_string_lossy();
    let trimmed = display.trim().trim_end_matches(std::path::MAIN_SEPARATOR);
    if trimmed.is_empty() {
        return None;
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn looks_like_weaver_job_id(title: &str, source_ref: &str) -> bool {
    let trimmed = title.trim();
    !trimmed.is_empty()
        && (trimmed == source_ref
            || (trimmed.len() >= 4 && trimmed.chars().all(|ch| ch.is_ascii_digit())))
}

fn import_source_title_from_payload(
    payload: &Value,
    source_system: &str,
    source_ref: &str,
    source_path: Option<&str>,
) -> Option<String> {
    let payload_title = payload
        .get("source_title")
        .and_then(Value::as_str)
        .or_else(|| payload.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string);

    let fallback_path_title = source_path.and_then(path_basename).or_else(|| {
        payload
            .get("dest_dir")
            .and_then(Value::as_str)
            .and_then(path_basename)
    });

    if source_system.eq_ignore_ascii_case("weaver")
        && payload_title
            .as_deref()
            .is_some_and(|title| looks_like_weaver_job_id(title, source_ref))
    {
        return fallback_path_title.or(payload_title);
    }

    payload_title.or(fallback_path_title)
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

pub fn from_delete_preview(preview: DeletePreview) -> DeletePreviewPayload {
    DeletePreviewPayload {
        fingerprint: preview.fingerprint,
        total_file_count: preview.total_file_count,
        media_count: preview.media_count,
        subtitle_count: preview.subtitle_count,
        image_count: preview.image_count,
        other_count: preview.other_count,
        directory_count: preview.directory_count,
        requires_typed_confirmation: preview.requires_typed_confirmation,
        typed_confirmation_prompt: preview.typed_confirmation_prompt,
        target_label: preview.target_label,
        sample_paths: preview.sample_paths,
    }
}

pub fn from_delete_titles_preview(
    preview: scryer_application::DeleteTitlesPreview,
) -> DeleteTitlesPreviewPayload {
    let failed_count = preview
        .items
        .iter()
        .filter(|item| item.error.is_some())
        .count() as i32;
    DeleteTitlesPreviewPayload {
        preview: from_delete_preview(preview.preview),
        items: preview
            .items
            .into_iter()
            .map(|item| DeleteTitlePreviewResultPayload {
                title_id: item.title_id.into(),
                preview: item.preview.map(from_delete_preview),
                error: item.error,
            })
            .collect(),
        failed_count,
    }
}

pub fn from_search_result(result: IndexerSearchResult) -> IndexerSearchResultPayload {
    let seeders = result
        .extra
        .get("seeders")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let peers = result
        .extra
        .get("peers")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let info_hash = result
        .extra
        .get("info_hash")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let freeleech = result.extra.get("freeleech").and_then(|v| v.as_bool());
    let download_volume_factor = result
        .extra
        .get("downloadvolumefactor")
        .and_then(|v| v.as_f64());

    IndexerSearchResultPayload {
        source: result.source,
        title: result.title,
        link: result.link,
        download_url: result.download_url,
        source_kind: result
            .source_kind
            .map(DownloadSourceKindValue::from_application),
        size_bytes: result.size_bytes.map(Long::from),
        published_at: parse_optional_datetime(result.published_at, "indexer search published_at"),
        thumbs_up: result.thumbs_up,
        thumbs_down: result.thumbs_down,
        parsed_release: result.parsed_release_metadata.map(from_parsed_release),
        quality_profile_decision: result
            .quality_profile_decision
            .map(from_quality_profile_decision),
        seeders,
        peers,
        info_hash,
        freeleech,
        download_volume_factor,
        candidate_token: result.candidate_token,
        queue_scope: result.queue_scope.map(from_submission_scope),
        auto_eligible: result.auto_eligible,
        auto_decision_code: result.auto_decision_code,
        auto_decision_summary: result.auto_decision_summary,
    }
}

pub fn from_submission_scope(scope: SubmissionScope) -> QueueDownloadScopePayload {
    match scope {
        SubmissionScope::Episode { episode_id } => {
            QueueDownloadScopePayload::episode(episode_id.into())
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            QueueDownloadScopePayload::EpisodeSet(EpisodeSetScopePayload {
                episode_ids: episode_ids.into_iter().map(Into::into).collect(),
            })
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => QueueDownloadScopePayload::SeriesMovie(SeriesMovieScopePayload {
            series_movie_link_id: series_movie_link_id.into(),
        }),
        SubmissionScope::Collection { collection_id } => {
            QueueDownloadScopePayload::Collection(CollectionScopePayload {
                collection_id: collection_id.into(),
            })
        }
        SubmissionScope::Title => {
            QueueDownloadScopePayload::Title(TitleScopePayload { whole_title: true })
        }
        SubmissionScope::Orphan => {
            QueueDownloadScopePayload::Orphan(OrphanScopePayload { orphaned: true })
        }
    }
}

pub fn from_title_release_blocklist_entry(
    entry: TitleReleaseBlocklistEntry,
) -> TitleReleaseBlocklistEntryPayload {
    TitleReleaseBlocklistEntryPayload {
        id: entry.id.into(),
        source_hint: entry.source_hint,
        source_title: entry.source_title,
        error_message: entry.error_message,
        attempted_at: parse_required_datetime(
            &entry.attempted_at,
            "title release blocklist attempted_at",
        ),
        episode_ids: entry.episode_ids.into_iter().map(Into::into).collect(),
    }
}

pub fn from_quality_profile_decision(
    decision: QualityProfileDecision,
) -> QualityProfileDecisionPayload {
    QualityProfileDecisionPayload {
        allowed: decision.allowed,
        block_codes: decision.block_codes,
        release_score: decision.release_score,
        preference_score: decision.preference_score,
        scoring_log: decision
            .scoring_log
            .into_iter()
            .map(|e: ScoringEntry| {
                let (source, rule_set_name) = match e.source {
                    ScoringSource::Builtin => ("builtin".to_string(), None),
                    ScoringSource::UserRule { id, name } => (format!("user:{id}"), Some(name)),
                    ScoringSource::SystemRule { id, name } => (format!("system:{id}"), Some(name)),
                };
                ScoringEntryPayload {
                    code: e.code,
                    delta: e.delta,
                    source,
                    rule_set_name,
                }
            })
            .collect(),
    }
}

pub fn from_parsed_release(result: ParsedReleaseMetadata) -> ParsedReleasePayload {
    ParsedReleasePayload {
        raw_title: result.raw_title,
        normalized_title: result.normalized_title,
        release_group: result.release_group,
        quality: result.quality,
        source: result.source.map(|source| source.to_string()),
        video_codec: result.video_codec.map(|codec| codec.to_string()),
        video_encoding: result.video_encoding,
        audio: result.audio.map(|codec| codec.to_string()),
        is_dual_audio: result.is_dual_audio,
        is_atmos: result.is_atmos,
        is_dolby_vision: result.is_dolby_vision,
        detected_hdr: result.detected_hdr,
        is_proper_upload: result.is_proper_upload,
        is_remux: result.is_remux,
        is_bd_disk: result.is_bd_disk,
        is_ai_enhanced: result.is_ai_enhanced,
        parse_confidence: result.parse_confidence,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
    }
}

pub fn from_parsed_episode(episode: ParsedEpisodeMetadata) -> ParsedEpisodePayload {
    ParsedEpisodePayload {
        season: episode.season.map(|value| value as i32),
        episode_numbers: episode
            .episode_numbers
            .into_iter()
            .map(|value| value as i32)
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
        indexer_proxy_config_id: config.indexer_proxy_config_id.map(Into::into),
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
        last_error_at: config.last_error_at,
        last_query_at: None,
        config: provider_config_values_from_json_with_fields(config_json.as_deref(), config_fields),
        created_at: config.created_at,
        updated_at: config.updated_at,
    }
}

pub fn from_indexer_proxy_config(config: IndexerProxyConfig) -> IndexerProxyConfigPayload {
    IndexerProxyConfigPayload {
        id: config.id.into(),
        name: config.name,
        provider_type: config.provider_type.as_str().to_string(),
        protocol: config.protocol.as_str().to_string(),
        base_url: config.base_url,
        request_timeout_seconds: i32::try_from(config.request_timeout_seconds).unwrap_or(i32::MAX),
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

pub fn from_indexer_proxy_test_result(
    result: IndexerProxyTestResult,
) -> IndexerProxyTestResultPayload {
    IndexerProxyTestResultPayload {
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
                    })
                    .collect(),
                help_text: f.help_text,
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

fn stored_secret_keys_from_config_json(raw: &str, config_fields: &[ConfigFieldDef]) -> Vec<String> {
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

pub fn from_download_queue_item(item: DownloadQueueItem) -> DownloadQueueItemPayload {
    let display_state = DownloadDisplayStateValue::from_application(
        scryer_application::derive_download_queue_display_state(&item),
    );
    DownloadQueueItemPayload {
        id: item.id.into(),
        title_id: item.title_id.map(Into::into),
        episode_id: item.episode_id.map(Into::into),
        title_name: item.title_name,
        facet: item.facet.as_deref().and_then(MediaFacetValue::parse),
        is_scryer_origin: item.is_scryer_origin,
        source_provider: item.source_provider,
        tracked_state: item
            .tracked_state
            .map(TrackedDownloadStateValue::from_domain),
        tracked_status: item
            .tracked_status
            .map(TrackedDownloadStatusValue::from_domain),
        tracked_status_messages: item.tracked_status_messages,
        tracked_match_type: item
            .tracked_match_type
            .map(TitleMatchTypeValue::from_domain),
        client_id: item.client_id.into(),
        client_name: item.client_name,
        client_type: item.client_type,
        state: DownloadQueueStateValue::from_domain(item.state),
        display_state,
        progress_percent: i32::from(item.progress_percent),
        import_transfer_phase: item.import_transfer_phase.map(Into::into),
        import_transfer_bytes: item.import_transfer_bytes.map(Long::from),
        import_transfer_total_bytes: item.import_transfer_total_bytes.map(Long::from),
        import_transfer_started_at: parse_optional_datetime(
            item.import_transfer_started_at,
            "download queue import_transfer_started_at",
        ),
        import_transfer_updated_at: parse_optional_datetime(
            item.import_transfer_updated_at,
            "download queue import_transfer_updated_at",
        ),
        size_bytes: item.size_bytes.map(Long::from),
        remaining_seconds: item
            .remaining_seconds
            .and_then(|value| i32::try_from(value).ok()),
        queued_at: parse_optional_datetime(item.queued_at, "download queue queued_at"),
        last_updated_at: parse_optional_datetime(
            item.last_updated_at,
            "download queue last_updated_at",
        ),
        attention_required: item.attention_required,
        attention_reason: item.attention_reason,
        download_client_item_id: item.download_client_item_id,
        download_id: item.download_id,
        import_status: item.import_status.map(ImportStatusValue::from_domain),
        import_error_code: item
            .import_error_code
            .map(ImportErrorCodeValue::from_domain),
        import_error_message: item.import_error_message,
        imported_at: parse_optional_datetime(item.imported_at, "download queue imported_at"),
        delete_status: item
            .delete_status
            .map(DownloadQueueDeleteStatusValue::from_domain),
        delete_error_message: item.delete_error_message,
    }
}

fn extract_tag_string(tags: &[String], prefix: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        tag.strip_prefix(prefix).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    })
}

fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
}

pub fn from_title(app: &AppUseCase, title: Title) -> TitlePayload {
    let quality_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:");
    let monitor_type = extract_tag_string(&title.tags, "scryer:monitor-type:")
        .as_deref()
        .and_then(MonitorTypeValue::from_tag_value);
    let use_season_folders = extract_tag_string(&title.tags, "scryer:season-folder:")
        .map(|value| !value.eq_ignore_ascii_case("disabled"));
    let monitor_specials = extract_tag_bool(&title.tags, "scryer:monitor-specials:");
    let inter_season_movies = extract_tag_bool(&title.tags, "scryer:inter-season-movies:");
    let filler_policy = extract_tag_string(&title.tags, "scryer:filler-policy:")
        .as_deref()
        .and_then(FillerPolicyValue::from_app_str);
    let recap_policy = extract_tag_string(&title.tags, "scryer:recap-policy:")
        .as_deref()
        .and_then(RecapPolicyValue::from_app_str);
    let poster_upstream = title.poster_source_url.as_deref().or(title
        .poster_url
        .as_deref()
        .filter(|url| url.starts_with("http")));
    let background_upstream = title.background_source_url.as_deref().or(title
        .background_url
        .as_deref()
        .filter(|url| url.starts_with("http")));
    let poster_url = app.media_image_url(
        poster_upstream,
        Some("title"),
        Some(&title.id),
        ImageProxyKind::Poster,
        "w250",
    );
    let background_url = app.media_image_url(
        background_upstream,
        Some("title"),
        Some(&title.id),
        ImageProxyKind::Fanart,
        "w1280",
    );

    TitlePayload {
        id: title.id.into(),
        library_id: title.library_id.into(),
        name: title.name,
        facet: MediaFacetValue::from_domain(title.facet),
        monitored: title.monitored,
        tags: title.tags,
        external_ids: title
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        created_at: title.created_at,
        year: title.year,
        overview: title.overview,
        poster_url: poster_url.clone(),
        poster_source_url: poster_url,
        background_url: background_url.clone(),
        background_source_url: background_url,
        sort_title: title.sort_title,
        slug: title.slug,
        imdb_id: title.imdb_id,
        runtime_minutes: title.runtime_minutes,
        popularity: title.popularity,
        canonical_tags: title
            .canonical_tags
            .into_iter()
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        content_status: title.content_status,
        language: title.language,
        first_aired: parse_date(title.first_aired),
        network: title.network,
        studio: title.studio,
        country: title.country,
        aliases: title.aliases,
        metadata_language: title.metadata_language,
        metadata_fetched_at: title.metadata_fetched_at,
        quality_profile_id: quality_profile_id.map(Into::into),
        root_folder_id: title.root_folder_id.into(),
        monitor_type,
        use_season_folders,
        monitor_specials,
        inter_season_movies,
        filler_policy,
        recap_policy,
    }
}

pub fn from_title_rating_summary(ratings: TitleRatingSummary) -> TitleRatingPayload {
    TitleRatingPayload {
        rating: ratings.rating,
        rating_sources: ratings.rating_sources,
        external_ratings: ratings
            .external_ratings
            .into_iter()
            .map(|rating| TitleExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
    }
}

pub fn from_media_request(app: &AppUseCase, request: MediaRequest) -> MediaRequestPayload {
    let owner_id = request.id.to_string();
    let poster_url = app.media_image_url(
        request.poster_url.as_deref(),
        Some("media_request"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MediaRequestPayload {
        id: request.id.into(),
        library_id: request.library_id.into(),
        facet: MediaFacetValue::from_domain(request.facet),
        status: MediaRequestStatusValue::from_domain(request.status),
        identity_fingerprint: request.identity_fingerprint,
        title: request.title,
        sort_title: request.sort_title,
        slug: request.slug,
        poster_url,
        year: request.year,
        overview: request.overview,
        runtime_minutes: request.runtime_minutes,
        language: request.language,
        content_status: request.content_status,
        requested_quality_profile_id: request.requested_quality_profile_id.map(Into::into),
        requested_quality_profile_name: request.requested_quality_profile_name,
        requested_monitor_type: request
            .requested_monitor_type
            .as_deref()
            .and_then(monitor_type_value_from_normalized),
        resolved_by_user_id: request.resolved_by_user_id.map(Into::into),
        resolved_at: request.resolved_at,
        created_title_id: request.created_title_id.map(Into::into),
        approved_quality_profile_id: request.approved_quality_profile_id.map(Into::into),
        approved_quality_profile_name: request.approved_quality_profile_name,
        external_ids: request
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        requesters: request
            .requesters
            .into_iter()
            .map(|requester| MediaRequestRequesterPayload {
                user_id: requester.user_id.into(),
                username: requester.username,
                avatar_url: requester.avatar_url,
                requested_at: requester.requested_at,
            })
            .collect(),
        created_by_user_id: request.created_by_user_id.into(),
        created_at: request.created_at,
        updated_at: request.updated_at,
    }
}

pub fn from_library(library: Library) -> LibraryPayload {
    let is_bootstrap_default_root_set =
        scryer_application::is_bootstrap_default_library_root_set(&library);
    LibraryPayload {
        id: library.id.into(),
        facet: MediaFacetValue::from_domain(library.facet),
        name: library.name,
        slug: library.slug,
        is_default: library.is_default,
        is_bootstrap_default_root_set,
        roots: library
            .roots
            .into_iter()
            .map(|root| LibraryRootPayload {
                id: root.id.into(),
                path: root.path,
                is_default: root.is_default,
            })
            .collect(),
    }
}

pub fn from_library_scan_summary(summary: LibraryScanSummary) -> LibraryScanSummaryPayload {
    LibraryScanSummaryPayload {
        scanned: summary.scanned as i32,
        matched: summary.matched as i32,
        imported: summary.imported as i32,
        skipped: summary.skipped as i32,
        unmatched: summary.unmatched as i32,
    }
}

pub fn from_pending_import_counts(counts: PendingImportCounts) -> PendingImportCountsPayload {
    PendingImportCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

pub fn from_media_request_counts(counts: MediaRequestCounts) -> MediaRequestCountsPayload {
    MediaRequestCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

fn from_pending_import_search_attempt(
    attempt: PendingImportSearchAttempt,
) -> PendingImportSearchAttemptPayload {
    PendingImportSearchAttemptPayload {
        query: attempt.query,
        result_count: attempt.result_count as i32,
        top_results: attempt.top_results,
        summary: attempt.summary,
    }
}

pub fn from_pending_import_item(item: PendingImportItem) -> PendingImportItemPayload {
    PendingImportItemPayload {
        id: item.id.into(),
        library_id: item.library_id.into(),
        facet: MediaFacetValue::from_domain(item.facet),
        status: PendingImportStatusValue::from_application(item.status),
        title_id: item.title_id.map(Into::into),
        title_name: item.title_name,
        title_slug: item.title_slug,
        display_name: item.display_name,
        path: item.path,
        folder_path: item.folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason: item.reason,
        search_attempts: item
            .search_attempts
            .into_iter()
            .map(from_pending_import_search_attempt)
            .collect(),
    }
}

pub fn from_pending_import_connection(
    connection: PendingImportConnection,
    offset: i64,
) -> PendingImportConnectionPayload {
    let items: Vec<_> = connection
        .items
        .into_iter()
        .map(from_pending_import_item)
        .collect();
    let has_more = offset.saturating_add(items.len() as i64) < connection.total;
    PendingImportConnectionPayload {
        items,
        total_count: connection.total as i32,
        has_more,
    }
}

pub fn from_resolve_pending_import_result(
    app: &AppUseCase,
    result: ResolvePendingImportResult,
) -> ResolvePendingImportPayload {
    ResolvePendingImportPayload {
        title: from_title(app, result.title),
        created: result.created,
        library_scan: result.library_scan.map(from_library_scan_summary),
        metadata_hydration_state: AddTitleHydrationStateValue::from_application(
            result.metadata_hydration_state,
        ),
    }
}

pub fn from_ignore_pending_import_result(
    result: IgnorePendingImportResult,
) -> IgnorePendingImportPayload {
    IgnorePendingImportPayload {
        id: async_graphql::ID::from(result.id),
        status: PendingImportStatusValue::from_application(result.status),
    }
}

pub fn from_cancel_library_scan_result(
    result: scryer_application::CancelLibraryScanResult,
) -> CancelLibraryScanPayload {
    CancelLibraryScanPayload {
        session_id: async_graphql::ID::from(result.session_id),
        accepted: result.accepted,
    }
}

pub fn from_library_scan_phase_progress(
    progress: scryer_application::LibraryScanPhaseProgress,
) -> LibraryScanPhaseProgressPayload {
    LibraryScanPhaseProgressPayload {
        total: progress.total as i32,
        completed: progress.completed as i32,
        failed: progress.failed as i32,
    }
}

pub fn from_library_scan_session(
    session: scryer_application::LibraryScanSession,
) -> LibraryScanProgressPayload {
    LibraryScanProgressPayload {
        session_id: session.session_id.into(),
        facet: MediaFacetValue::from_domain(session.facet),
        library_id: session.library_id.map(Into::into),
        mode: LibraryScanModeValue::from_application(session.mode),
        status: LibraryScanStatusValue::from_application(session.status),
        started_at: session.started_at,
        updated_at: session.updated_at,
        found_titles: session.found_titles as i32,
        title_match_total_known: session.title_match_total_known,
        title_match_progress: from_library_scan_phase_progress(session.title_match_progress),
        hydration_total_known: session.metadata_total_known,
        hydration_progress: from_library_scan_phase_progress(session.metadata_progress),
        media_analysis_total_known: session.file_total_known,
        media_analysis_progress: from_library_scan_phase_progress(session.file_progress),
        summary: session.summary.map(from_library_scan_summary),
    }
}

pub fn from_job_definition(definition: JobDefinition) -> JobDefinitionPayload {
    JobDefinitionPayload {
        key: JobKeyValue::from_application(definition.key),
        display_name: definition.display_name,
        description: definition.description,
        category: JobCategoryValue::from_application(definition.category),
        section: JobSectionValue::from_application(definition.section),
        manual_trigger_allowed: definition.manual_trigger_allowed,
        uses_library_scan_progress: definition.uses_library_scan_progress,
        schedule: JobScheduleInfoPayload {
            kind: JobScheduleKindValue::from_application(definition.schedule.kind),
            description: definition.schedule.description,
            interval_seconds: definition
                .schedule
                .interval_seconds
                .map(|value| value as i32),
            initial_delay_seconds: definition
                .schedule
                .initial_delay_seconds
                .map(|value| value as i32),
            next_run_at: definition.schedule.next_run_at,
        },
    }
}

pub fn from_job_run(run: JobRun) -> JobRunPayload {
    JobRunPayload {
        id: run.id.into(),
        job_key: JobKeyValue::from_application(run.job_key),
        display_name: run.display_name,
        category: JobCategoryValue::from_application(run.category),
        section: JobSectionValue::from_application(run.section),
        status: JobRunStatusValue::from_application(run.status),
        trigger_source: JobTriggerSourceValue::from_application(run.trigger_source),
        started_at: run.started_at,
        completed_at: run.completed_at,
        summary_json: run.summary_json.map(json_string_to_value),
        summary_text: run.summary_text,
        error_text: run.error_text,
        progress_json: run.progress_json.map(json_string_to_value),
        library_scan_progress: run.library_scan_progress.map(from_library_scan_session),
    }
}

pub fn from_discovery_sync_status(status: DiscoverySyncStatus) -> DiscoverySyncStatusPayload {
    DiscoverySyncStatusPayload {
        state: from_discovery_sync_state(status.state),
        pending_context_change_count: Long(status.pending_context_change_count),
    }
}

pub fn from_discovery_sync_state(state: DiscoverySyncStateRecord) -> DiscoverySyncStatePayload {
    DiscoverySyncStatePayload {
        last_success_generation_id: state.last_success_generation_id.map(ID::from),
        last_public_feed_generation_id: state.last_public_feed_generation_id.map(ID::from),
        last_context_snapshot_completed_at: state.last_context_snapshot_completed_at,
        last_incremental_reload_completed_at: state.last_incremental_reload_completed_at,
        last_public_feed_completed_at: state.last_public_feed_completed_at,
        next_context_snapshot_eligible_at: state.next_context_snapshot_eligible_at,
        next_incremental_reload_eligible_at: state.next_incremental_reload_eligible_at,
        next_public_feed_eligible_at: state.next_public_feed_eligible_at,
        updated_at: state.updated_at,
    }
}

const DISCOVERY_HOME_MAX_SECTION_LIMIT: i32 = 100;

fn discovery_home_canonical_tag_keys(
    field_name: &str,
    keys: Option<Vec<String>>,
) -> scryer_application::AppResult<Vec<String>> {
    let keys = keys.unwrap_or_default();
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err(scryer_application::AppError::Validation(format!(
            "discovery home {field_name} entries must not be blank"
        )));
    }
    Ok(keys)
}

pub fn discovery_home_query_from_input(
    input: Option<DiscoveryHomeInput>,
) -> scryer_application::AppResult<DiscoveryHomeQuery> {
    let input = input.unwrap_or_default();
    let filters = input.filters.unwrap_or_default();
    let limit_per_section = match input.limit_per_section {
        Some(value) if !(1..=DISCOVERY_HOME_MAX_SECTION_LIMIT).contains(&value) => {
            return Err(scryer_application::AppError::Validation(format!(
                "discovery home limitPerSection must be between 1 and {DISCOVERY_HOME_MAX_SECTION_LIMIT}"
            )));
        }
        Some(value) => value as usize,
        None => 25,
    };
    let minimum_rating = filters.minimum_rating;
    if minimum_rating.is_some_and(|value| !value.is_finite() || !(0.0..=10.0).contains(&value)) {
        return Err(scryer_application::AppError::Validation(
            "discovery home minimumRating must be a finite value between 0 and 10".to_owned(),
        ));
    }
    if let (Some(minimum_year), Some(maximum_year)) = (filters.minimum_year, filters.maximum_year)
        && minimum_year > maximum_year
    {
        return Err(scryer_application::AppError::Validation(
            "discovery home minimumYear must not exceed maximumYear".to_owned(),
        ));
    }
    let genre_tag_keys = discovery_home_canonical_tag_keys("genreTagKeys", filters.genre_tag_keys)?;
    let theme_tag_keys = discovery_home_canonical_tag_keys("themeTagKeys", filters.theme_tag_keys)?;

    Ok(DiscoveryHomeQuery {
        include_public: input.include_public.unwrap_or(true),
        include_personalized: input.include_personalized.unwrap_or(true),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        limit_per_section,
        filters: DiscoveryHomeFilters {
            content_types: filters
                .content_types
                .unwrap_or_default()
                .into_iter()
                .map(MediaFacetValue::as_scope_id)
                .map(str::to_owned)
                .collect(),
            genre_tag_keys,
            theme_tag_keys,
            studio_slugs: filters.studio_slugs.unwrap_or_default(),
            minimum_year: filters.minimum_year,
            maximum_year: filters.maximum_year,
            minimum_rating,
        },
    })
}

pub fn discovery_home_filter_options_query_from_input(
    input: Option<DiscoveryHomeFilterOptionsInput>,
) -> DiscoveryHomeQuery {
    let input = input.unwrap_or_default();
    DiscoveryHomeQuery {
        include_public: input.include_public.unwrap_or(true),
        include_personalized: input.include_personalized.unwrap_or(true),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        ..DiscoveryHomeQuery::default()
    }
}

pub fn discovery_items_query_from_input(input: Option<DiscoveryItemsInput>) -> DiscoveryItemsQuery {
    let input = input.unwrap_or_default();
    DiscoveryItemsQuery {
        query: input.query,
        target_keys: Vec::new(),
        target_kinds: input.target_kinds.unwrap_or_default(),
        sources: input.sources.unwrap_or_default(),
        relation_types: input.relation_types.unwrap_or_default(),
        relation_subtypes: input.relation_subtypes.unwrap_or_default(),
        genres: input.genres.unwrap_or_default(),
        status_tags: input.status_tags.unwrap_or_default(),
        facet_terms: input.facet_terms.unwrap_or_default(),
        include_owned: input.include_owned.unwrap_or(false),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        include_public: input.include_public.unwrap_or(false),
        limit: input.limit.map(|value| value.max(1) as usize).unwrap_or(50),
        offset: input.offset.map(|value| value.max(0) as usize).unwrap_or(0),
    }
}

pub fn discovery_item_detail_query_from_input(
    input: DiscoveryItemDetailInput,
) -> DiscoveryItemDetailQuery {
    DiscoveryItemDetailQuery {
        target_key: input.target_key,
        include_unresolved: input.include_unresolved.unwrap_or(true),
    }
}

pub fn catalog_discovery_query_from_input(input: CatalogDiscoveryInput) -> CatalogDiscoveryQuery {
    CatalogDiscoveryQuery {
        facet: input.facet.into_domain(),
        library_ids: input
            .library_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        include_unresolved: input.include_unresolved.unwrap_or(true),
        limit_per_group: input
            .limit_per_group
            .map(|value| value.max(1) as usize)
            .unwrap_or(12),
        max_groups: input
            .max_groups
            .map(|value| value.max(1) as usize)
            .unwrap_or(6),
    }
}

pub fn from_discovery_home(app: &AppUseCase, result: DiscoveryHomeResult) -> DiscoveryHomePayload {
    DiscoveryHomePayload {
        status: from_discovery_sync_status(result.status),
        hero_item: result.hero_item.map(|item| from_discovery_item(app, item)),
        public_sections: result
            .public_sections
            .into_iter()
            .map(|section| from_discovery_section(app, section))
            .collect(),
        personalized_sections: result
            .personalized_sections
            .into_iter()
            .map(|section| from_discovery_section(app, section))
            .collect(),
        complete_collection: result
            .complete_collection
            .map(|section| from_discovery_section(app, section)),
        facets: result
            .facets
            .into_iter()
            .map(from_discovery_facet)
            .collect(),
        can_view_personalized: result.can_view_personalized,
    }
}

pub fn from_discovery_home_cards(
    app: &AppUseCase,
    result: DiscoveryHomeResult,
) -> scryer_application::AppResult<DiscoveryHomeCardsPayload> {
    let hero_item = result
        .hero_item
        .map(|item| from_discovery_home_hero(app, item))
        .transpose()?;
    let public_sections = result
        .public_sections
        .into_iter()
        .map(|section| from_discovery_home_section(app, section))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let personalized_sections = result
        .personalized_sections
        .into_iter()
        .map(|section| from_discovery_home_section(app, section))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let complete_collection = result
        .complete_collection
        .map(|section| from_discovery_home_section(app, section))
        .transpose()?;

    Ok(DiscoveryHomeCardsPayload {
        status: from_discovery_sync_status(result.status),
        hero_item,
        public_sections,
        personalized_sections,
        complete_collection,
        can_view_personalized: result.can_view_personalized,
    })
}

pub fn from_discovery_home_filter_options(
    options: DiscoveryHomeFilterOptions,
) -> DiscoveryHomeFilterOptionsPayload {
    DiscoveryHomeFilterOptionsPayload {
        genres: options
            .genres
            .into_iter()
            .map(|option| CanonicalTagFilterOptionPayload {
                key: option.key,
                name: option.name,
            })
            .collect(),
        themes: options
            .themes
            .into_iter()
            .map(|option| CanonicalTagFilterOptionPayload {
                key: option.key,
                name: option.name,
            })
            .collect(),
        studio_slugs: options.studio_slugs,
    }
}

pub fn from_discovery_items_result(
    app: &AppUseCase,
    result: DiscoveryItemsResult,
) -> DiscoveryItemsPayload {
    DiscoveryItemsPayload {
        items: result
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
        total_count: Long(result.total_count),
        can_view_personalized: result.can_view_personalized,
    }
}

pub fn from_catalog_discovery(
    app: &AppUseCase,
    result: CatalogDiscoveryResult,
) -> CatalogDiscoveryPayload {
    CatalogDiscoveryPayload {
        can_view_personalized: result.can_view_personalized,
        groups: result
            .groups
            .into_iter()
            .map(|group| from_catalog_discovery_group(app, group))
            .collect(),
    }
}

fn from_catalog_discovery_group(
    app: &AppUseCase,
    group: CatalogDiscoveryGroup,
) -> CatalogDiscoveryGroupPayload {
    CatalogDiscoveryGroupPayload {
        id: group.id,
        kind: from_catalog_discovery_group_kind(group.kind),
        surface: from_catalog_discovery_surface(group.surface),
        label_value: group.label_value,
        total_count: Long(group.total_count),
        items: group
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
    }
}

fn from_catalog_discovery_group_kind(
    kind: CatalogDiscoveryGroupKind,
) -> CatalogDiscoveryGroupKindValue {
    match kind {
        CatalogDiscoveryGroupKind::PublicTop => CatalogDiscoveryGroupKindValue::PublicTop,
        CatalogDiscoveryGroupKind::PublicSection => CatalogDiscoveryGroupKindValue::PublicSection,
        CatalogDiscoveryGroupKind::GenreAffinity => CatalogDiscoveryGroupKindValue::GenreAffinity,
        CatalogDiscoveryGroupKind::ThemeAffinity => CatalogDiscoveryGroupKindValue::ThemeAffinity,
        CatalogDiscoveryGroupKind::Acclaimed => CatalogDiscoveryGroupKindValue::Acclaimed,
        CatalogDiscoveryGroupKind::CompleteCollection => {
            CatalogDiscoveryGroupKindValue::CompleteCollection
        }
        CatalogDiscoveryGroupKind::Fallback => CatalogDiscoveryGroupKindValue::Fallback,
    }
}

fn from_catalog_discovery_surface(
    surface: CatalogDiscoverySurface,
) -> CatalogDiscoverySurfaceValue {
    match surface {
        CatalogDiscoverySurface::Public => CatalogDiscoverySurfaceValue::Public,
        CatalogDiscoverySurface::Personalized => CatalogDiscoverySurfaceValue::Personalized,
    }
}

pub fn from_discovery_section(
    app: &AppUseCase,
    section: DiscoverySectionResult,
) -> DiscoverySectionPayload {
    DiscoverySectionPayload {
        section_id: section.section_id,
        section_type: section.section_type,
        title: section.title,
        surface: section.surface,
        total_count: Long(section.total_count),
        items: section
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
    }
}

fn discovery_home_media_facet(
    value: &str,
    field_name: &str,
) -> scryer_application::AppResult<MediaFacetValue> {
    MediaFacetValue::parse(value).ok_or_else(|| {
        scryer_application::AppError::Validation(format!(
            "discovery home {field_name} must be a supported media facet: {value}"
        ))
    })
}

fn from_discovery_home_section(
    app: &AppUseCase,
    section: DiscoverySectionResult,
) -> scryer_application::AppResult<DiscoveryHomeSectionPayload> {
    let surface = discovery_surface_value(&section.surface)?;
    let items = section
        .items
        .into_iter()
        .map(|item| from_discovery_home_card(app, item))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;

    Ok(DiscoveryHomeSectionPayload {
        section_id: section.section_id,
        section_type: section.section_type,
        title: section.title,
        surface,
        total_count: Long(section.total_count),
        items,
    })
}

fn discovery_surface_value(value: &str) -> scryer_application::AppResult<DiscoverySurfaceValue> {
    let surface = match value {
        "public" => DiscoverySurfaceValue::Public,
        "personalized" => DiscoverySurfaceValue::Personalized,
        "mixed" => DiscoverySurfaceValue::Mixed,
        value => {
            return Err(scryer_application::AppError::Validation(format!(
                "discovery home section has an unsupported surface: {value}"
            )));
        }
    };
    Ok(surface)
}

fn from_discovery_home_card(
    app: &AppUseCase,
    item: DiscoveryItemRecord,
) -> scryer_application::AppResult<DiscoveryHomeCardPayload> {
    let target_kind = discovery_home_media_facet(&item.target_kind, "card targetKind")?;
    let content_type = discovery_home_media_facet(
        item.content_type
            .as_deref()
            .unwrap_or(item.target_kind.as_str()),
        "card contentType",
    )?;

    let (poster_url, _) = discovery_image_urls(app, &item);
    Ok(DiscoveryHomeCardPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind,
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        content_type,
        is_adult: item.is_adult,
        owned_in_input: item.owned_in_input,
    })
}

fn from_discovery_home_hero(
    app: &AppUseCase,
    item: DiscoveryItemRecord,
) -> scryer_application::AppResult<DiscoveryHomeHeroPayload> {
    let target_kind = discovery_home_media_facet(&item.target_kind, "hero targetKind")?;
    let content_type = discovery_home_media_facet(
        item.content_type
            .as_deref()
            .unwrap_or(item.target_kind.as_str()),
        "hero contentType",
    )?;

    let (poster_url, background_url) = discovery_image_urls(app, &item);
    Ok(DiscoveryHomeHeroPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind,
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        background_url,
        overview: item.overview,
        content_type,
        is_adult: item.is_adult,
        rating: item.rating,
        rating_sources: item.rating_sources,
        external_ratings: item
            .external_ratings
            .into_iter()
            .map(|rating| DiscoveryExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
        genre_tags: item
            .canonical_tags
            .into_iter()
            .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        matched_subject_count: item.matched_subject_count,
        owned_in_input: item.owned_in_input,
    })
}

fn discovery_image_urls(
    app: &AppUseCase,
    item: &DiscoveryItemRecord,
) -> (Option<String>, Option<String>) {
    let (owner_type, owner_id) = item
        .resolved_title_id
        .as_deref()
        .map(|title_id| ("title", title_id))
        .unwrap_or(("discovery", item.id.as_str()));
    let poster_source =
        preferred_discovery_poster_source(item.poster_path.as_deref(), item.poster_url.as_deref());
    let poster = app.media_image_url(
        poster_source.as_deref(),
        Some(owner_type),
        Some(owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    let background = app.media_image_url(
        item.background_url.as_deref(),
        Some(owner_type),
        Some(owner_id),
        ImageProxyKind::Fanart,
        "w1280",
    );
    (poster, background)
}

fn preferred_discovery_poster_source(
    poster_path: Option<&str>,
    poster_url: Option<&str>,
) -> Option<String> {
    let tmdb_source = poster_path.and_then(|value| {
        let value = value.trim();
        if value.starts_with("https://image.tmdb.org/")
            || value.starts_with("http://image.tmdb.org/")
        {
            Some(value.to_string())
        } else if value.starts_with('/') && !value.starts_with("//") {
            Some(format!("https://image.tmdb.org/t/p/original{value}"))
        } else {
            None
        }
    });

    tmdb_source.or_else(|| {
        poster_url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

pub fn from_discovery_item(app: &AppUseCase, item: DiscoveryItemRecord) -> DiscoveryItemPayload {
    let (poster_url, background_url) = discovery_image_urls(app, &item);
    DiscoveryItemPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind: item.target_kind,
        resolved: item.resolved,
        resolved_title_id: item.resolved_title_id.map(ID::from),
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        background_url,
        overview: item.overview,
        content_type: item.content_type,
        canonical_tags: item
            .canonical_tags
            .into_iter()
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        is_adult: item.is_adult,
        content_ratings: item
            .content_ratings
            .into_iter()
            .map(|rating| DiscoveryContentRatingPayload {
                country: rating.country,
                certifications: rating
                    .certifications
                    .into_iter()
                    .map(|certification| DiscoveryContentCertificationPayload {
                        value: certification.value,
                        source: certification.source,
                        release_type: certification.release_type,
                    })
                    .collect(),
                age_rating: rating.age_rating,
                age_rating_source: rating.age_rating_source,
            })
            .collect(),
        rating: item.rating,
        rating_sources: item.rating_sources,
        external_ratings: item
            .external_ratings
            .into_iter()
            .map(|rating| DiscoveryExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
        external_ids: item
            .external_ids
            .into_iter()
            .map(|external_id| DiscoveryExternalIdPayload {
                source: external_id.source,
                kind: external_id.kind,
                id: external_id.id,
                key: external_id.key,
            })
            .collect(),
        status_tags: item.status_tags,
        source_tags: item
            .source_tags
            .into_iter()
            .flat_map(|tag| {
                tag.name
                    .into_iter()
                    .chain(tag.category)
                    .chain(tag.values)
                    .collect::<Vec<_>>()
            })
            .collect(),
        sources: item.sources,
        best_source: item.best_source,
        relation_types: item.relation_types,
        relation_subtypes: item.relation_subtypes,
        source_count: item.source_count,
        edge_count: item.edge_count,
        relation_count: item.relation_count,
        source_subject_count: item.source_subject_count,
        rank_score: item.rank_score,
        matched_subject_titles: item.matched_subject_titles,
        matched_subject_count: item.matched_subject_count,
        tmdb_collection_id: item.tmdb_collection_id,
        tmdb_collection_name: item.tmdb_collection_name,
        owned_in_input: item.owned_in_input,
        studio_slug: item.studio_slug,
        person_ids: item.person_ids,
        facet_terms: item.facet_terms,
        context_terms: item.context_terms,
    }
}

pub fn from_discovery_facet(facet: DiscoveryFacetRecord) -> DiscoveryFacetPayload {
    DiscoveryFacetPayload {
        name: facet.facet_name,
        value: facet.facet_value,
        smg_count: facet.smg_count.map(Long),
        local_count: facet.local_count.map(Long),
    }
}

pub fn from_media_rename_plan(plan: RenamePlan) -> MediaRenamePlanPayload {
    MediaRenamePlanPayload {
        facet: MediaFacetValue::from_domain(plan.facet),
        title_id: plan.title_id.map(Into::into),
        template: plan.template,
        collision_policy: RenameCollisionPolicyValue::from_app_str(plan.collision_policy.as_str())
            .unwrap_or(RenameCollisionPolicyValue::Skip),
        missing_metadata_policy: RenameMissingMetadataPolicyValue::from_app_str(
            plan.missing_metadata_policy.as_str(),
        )
        .unwrap_or(RenameMissingMetadataPolicyValue::FallbackTitle),
        fingerprint: plan.fingerprint,
        total: plan.total as i32,
        renamable: plan.renamable as i32,
        noop: plan.noop as i32,
        conflicts: plan.conflicts as i32,
        errors: plan.errors as i32,
        items: plan
            .items
            .into_iter()
            .map(from_media_rename_plan_item)
            .collect(),
    }
}

fn from_media_rename_plan_item(item: RenamePlanItem) -> MediaRenamePlanItemPayload {
    MediaRenamePlanItemPayload {
        collection_id: item.collection_id.map(Into::into),
        series_movie_link_ids: item
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        normalized_filename: item.normalized_filename,
        collision: item.collision,
        reason_code: item.reason_code,
        write_action: item.write_action.as_str().to_string(),
        source_size_bytes: item.source_size_bytes.map(Long::from_u64_saturating),
        source_mtime_unix_ms: item.source_mtime_unix_ms.map(Long::from),
    }
}

pub fn from_media_rename_apply(result: RenameApplyResult) -> MediaRenameApplyPayload {
    MediaRenameApplyPayload {
        plan_fingerprint: result.plan_fingerprint,
        total: result.total as i32,
        applied: result.applied as i32,
        skipped: result.skipped as i32,
        failed: result.failed as i32,
        items: result
            .items
            .into_iter()
            .map(from_media_rename_apply_item)
            .collect(),
    }
}

fn from_media_rename_apply_item(item: RenameApplyItemResult) -> MediaRenameApplyItemPayload {
    MediaRenameApplyItemPayload {
        collection_id: item.collection_id.map(Into::into),
        series_movie_link_ids: item
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        final_path: item.final_path,
        write_action: item.write_action.as_str().to_string(),
        status: item.status.as_str().to_string(),
        reason_code: item.reason_code,
        error_message: item.error_message,
    }
}

pub fn from_collection(collection: Collection) -> CollectionPayload {
    CollectionPayload {
        id: collection.id.into(),
        title_id: collection.title_id.into(),
        collection_type: collection.collection_type.into(),
        collection_index: collection.collection_index,
        label: collection.label,
        ordered_path: collection.ordered_path,
        narrative_order: collection.narrative_order,
        first_episode_number: collection.first_episode_number,
        last_episode_number: collection.last_episode_number,
        monitored: collection.monitored,
        created_at: collection.created_at,
    }
}

pub fn from_movie_entity(
    app: &AppUseCase,
    movie: scryer_domain::MovieEntity,
) -> MovieEntityPayload {
    let owner_id = movie.id.to_string();
    let poster_url = app.media_image_url(
        movie.poster_url.as_deref(),
        Some("movie"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MovieEntityPayload {
        id: movie.id.into(),
        title: movie.title,
        slug: movie.slug,
        year: movie.year,
        overview: movie.overview,
        poster_url,
        runtime_minutes: movie.runtime_minutes,
        content_status: movie.content_status,
        imdb_id: movie.imdb_id,
        tvdb_id: movie.tvdb_id,
        tmdb_id: movie.tmdb_id,
        mal_id: movie.mal_id,
        anidb_id: movie.anidb_id,
    }
}

pub fn from_series_movie_link(
    app: &AppUseCase,
    link: scryer_domain::SeriesMovieLink,
) -> SeriesMovieLinkPayload {
    SeriesMovieLinkPayload {
        id: link.id.into(),
        movie: from_movie_entity(app, link.movie),
        narrative_order: link.narrative_order,
        after_season: link.after_season,
        before_season: link.before_season,
        linked_episode_id: link.linked_episode_id.map(Into::into),
        continuity_status: link.continuity_status,
        movie_form: link.movie_form,
        signal_summary: link.signal_summary,
        monitored: link.monitored,
    }
}

pub fn from_episode(app: &AppUseCase, episode: Episode) -> EpisodePayload {
    let image_url = app.media_image_url(
        episode.image_url.as_deref(),
        Some("episode"),
        Some(&episode.id),
        ImageProxyKind::EpisodeStill,
        "original",
    );
    EpisodePayload {
        id: episode.id.into(),
        title_id: episode.title_id.into(),
        collection_id: episode.collection_id.map(Into::into),
        episode_type: episode.episode_type.into(),
        episode_number: episode.episode_number,
        season_number: episode.season_number,
        episode_label: episode.episode_label,
        title: episode.title,
        overview: episode.overview,
        air_date: parse_date(episode.air_date),
        duration_seconds: episode.duration_seconds,
        has_multi_audio: episode.has_multi_audio,
        has_subtitle: episode.has_subtitle,
        is_filler: episode.is_filler,
        is_recap: episode.is_recap,
        absolute_number: episode.absolute_number,
        image_url,
        monitored: episode.monitored,
        created_at: episode.created_at,
    }
}

pub fn from_calendar_episode(ep: CalendarEpisode) -> CalendarEpisodePayload {
    CalendarEpisodePayload {
        id: ep.id.into(),
        title_id: ep.title_id.into(),
        library_id: ep.library_id.into(),
        library_name: ep.library_name,
        library_slug: ep.library_slug,
        title_name: ep.title_name,
        title_slug: ep.title_slug,
        title_facet: ep.title_facet,
        season_number: ep.season_number,
        episode_number: ep.episode_number,
        episode_title: ep.episode_title,
        air_date: parse_date(ep.air_date),
        monitored: ep.monitored,
    }
}

pub fn from_title_media_file(file: scryer_application::TitleMediaFile) -> TitleMediaFilePayload {
    TitleMediaFilePayload {
        id: file.id.into(),
        title_id: file.title_id.into(),
        episode_id: file.episode_id.map(Into::into),
        series_movie_link_ids: file
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        file_path: file.file_path,
        size_bytes: Long::from(file.size_bytes),
        role: file.role.as_str().to_string(),
        quality_label: file.quality_label,
        scan_status: file.scan_status,
        created_at: parse_required_datetime(&file.created_at, "title media file created_at"),
        video_codec: file.video_codec.map(|codec| codec.to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        video_bitrate_kbps: file.video_bitrate_kbps,
        video_bit_depth: file.video_bit_depth,
        video_hdr_format: file.video_hdr_format,
        video_frame_rate: file.video_frame_rate,
        video_profile: file.video_profile,
        audio_codec: file.audio_codec,
        audio_channels: file.audio_channels,
        audio_bitrate_kbps: file.audio_bitrate_kbps,
        audio_languages: file.audio_languages,
        audio_streams: file
            .audio_streams
            .into_iter()
            .map(|s| crate::types::AudioStreamDetailPayload {
                codec: s.codec,
                channels: s.channels,
                language: s.language,
                bitrate_kbps: s.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: file.subtitle_languages,
        subtitle_codecs: file.subtitle_codecs,
        subtitle_streams: file
            .subtitle_streams
            .into_iter()
            .map(|s| crate::types::SubtitleStreamDetailPayload {
                codec: s.codec,
                language: s.language,
                name: s.name,
                forced: s.forced,
                default: s.default,
            })
            .collect(),
        has_multiaudio: file.has_multiaudio,
        duration_seconds: file.duration_seconds,
        num_chapters: file.num_chapters,
        container_format: file.container_format,
        scene_name: file.scene_name,
        release_group: file.release_group,
        source_type: file.source_type,
        resolution: file.resolution,
        video_codec_parsed: file.video_codec_parsed.map(|codec| codec.to_string()),
        audio_codec_parsed: file.audio_codec_parsed,
        acquisition_score: file.acquisition_score,
        scoring_log: file.scoring_log,
        indexer_source: file.indexer_source,
        grabbed_release_title: file.grabbed_release_title,
        grabbed_at: parse_optional_datetime(file.grabbed_at, "title media file grabbed_at"),
        edition: file.edition,
        original_file_path: file.original_file_path,
        release_hash: file.release_hash,
    }
}

pub fn from_user(user: User) -> UserPayload {
    from_user_with_auth_factor_status(user, scryer_application::UserAuthFactorStatus::default())
}

pub fn from_user_with_auth_factor_status(
    user: User,
    auth_factor_status: scryer_application::UserAuthFactorStatus,
) -> UserPayload {
    let login_status = user.login_status();
    let User {
        id,
        username,
        password_hash,
        account_kind,
        authorization,
        ..
    } = user;

    let app_permissions = authorization
        .app
        .to_permissions()
        .into_iter()
        .map(AppPermissionValue::from_domain)
        .collect();
    let mut library_permissions = authorization
        .libraries
        .into_iter()
        .map(
            |(library_id, permissions)| UserLibraryPermissionGrantPayload {
                library_id: library_id.into(),
                permissions: permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
    library_permissions
        .sort_by(|left, right| left.library_id.as_str().cmp(right.library_id.as_str()));

    UserPayload {
        id: id.into(),
        is_default_admin: username.eq_ignore_ascii_case("admin"),
        username,
        login_enabled: login_status.is_enabled(),
        has_password: password_hash.is_some(),
        has_mfa: auth_factor_status.has_mfa,
        has_passkey: auth_factor_status.has_passkey,
        account_kind: UserAccountKindValue::from_domain(account_kind),
        app_permissions,
        library_permissions,
    }
}

pub fn from_linked_account(account: scryer_domain::UserExternalAccount) -> LinkedAccountPayload {
    LinkedAccountPayload {
        id: account.id.into(),
        user_id: account.user_id.into(),
        provider: ExternalAccountProviderValue::from_domain(account.provider),
        connection_id: account.connection_id.into(),
        external_user_id: account.external_user_id,
        username: account.username,
        display_name: account.display_name,
        avatar_url: account.avatar_url,
        status: ExternalAccountStatusValue::from_domain(account.status),
        verified_at: account.verified_at,
        last_login_at: account.last_login_at,
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

pub fn from_media_server_connection(
    connection: scryer_domain::MediaServerConnection,
) -> MediaServerConnectionPayload {
    let api_key_present = connection.api_key_present();
    let machine_id_present = connection.machine_id.is_some();
    MediaServerConnectionPayload {
        id: connection.id.into(),
        provider: MediaServerProviderValue::from_domain(connection.provider),
        display_name: connection.display_name,
        base_url: connection.base_url,
        enabled: connection.enabled,
        login_enabled: connection.login_enabled,
        linking_enabled: connection.linking_enabled,
        auto_add_enabled: connection.auto_add_enabled,
        default_app_permissions: connection
            .default_app_permissions
            .to_permissions()
            .into_iter()
            .map(AppPermissionValue::from_domain)
            .collect(),
        default_library_grants: connection
            .default_library_grants
            .into_iter()
            .map(|grant| MediaServerDefaultLibraryGrantPayload {
                library_id: grant.library_id.into(),
                permissions: grant
                    .permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            })
            .collect(),
        machine_id_present,
        api_key_present,
        path_mappings: connection
            .path_mappings
            .into_iter()
            .map(|mapping| MediaServerPathMappingPayload {
                source_path: mapping.source_path,
                destination_path: mapping.destination_path,
            })
            .collect(),
        created_at: connection.created_at,
        updated_at: connection.updated_at,
    }
}

pub fn from_jellyfin_server_user(
    user: scryer_application::JellyfinServerUser,
) -> JellyfinServerUserPayload {
    JellyfinServerUserPayload {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
    }
}

pub fn from_media_server_user(user: scryer_application::MediaServerUser) -> MediaServerUserPayload {
    MediaServerUserPayload {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
    }
}

pub fn from_media_server_user_group(
    group: scryer_application::MediaServerUserGroup,
) -> MediaServerUserGroupPayload {
    MediaServerUserGroupPayload {
        connection_id: group.connection_id.into(),
        connection_name: group.connection_name,
        provider: ExternalAccountProviderValue::from_domain(group.provider),
        status: match group.status {
            scryer_application::MediaServerUserGroupStatus::Ready => {
                MediaServerUserGroupStatusValue::Ready
            }
            scryer_application::MediaServerUserGroupStatus::MissingCredentials => {
                MediaServerUserGroupStatusValue::MissingCredentials
            }
            scryer_application::MediaServerUserGroupStatus::Error => {
                MediaServerUserGroupStatusValue::Error
            }
        },
        error_message: group.error_message,
        users: group
            .users
            .into_iter()
            .map(from_media_server_user)
            .collect(),
    }
}

pub fn from_plex_server_discovery(
    server: scryer_application::PlexServerDiscovery,
) -> PlexServerDiscoveryPayload {
    PlexServerDiscoveryPayload {
        id: server.id,
        name: server.name,
    }
}

pub fn from_activity_event(event: ActivityEvent) -> ActivityEventPayload {
    ActivityEventPayload {
        id: event.id.into(),
        kind: ActivityKindValue::from_application(event.kind),
        severity: ActivitySeverityValue::from_application(event.severity),
        channels: event
            .channels
            .into_iter()
            .map(ActivityChannelValue::from_application)
            .collect(),
        actor_kind: event.actor_kind.into(),
        actor_user_id: event.actor_user_id.map(Into::into),
        actor_display_name: event.actor_display_name,
        title_id: event.title_id.map(Into::into),
        facet: event.facet.as_deref().and_then(MediaFacetValue::parse),
        message: event.message,
        occurred_at: event.occurred_at,
    }
}

pub fn from_import_record(record: scryer_domain::ImportRecord) -> ImportRecordPayload {
    // Deserialize result_json to extract structured fields
    let (error_message, decision, skip_reason, title_id, source_path, dest_path) =
        if let Some(ref result_json) = record.result_json {
            if let Ok(result) = serde_json::from_str::<scryer_domain::ImportResult>(result_json) {
                (
                    result.error_message,
                    Some(ImportDecisionValue::from_domain(result.decision)),
                    result.skip_reason.map(ImportSkipReasonValue::from_domain),
                    result.title_id,
                    Some(result.source_path),
                    result.dest_path,
                )
            } else {
                (None, None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None, None)
        };

    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok();
    let source_title = payload.as_ref().and_then(|payload| {
        import_source_title_from_payload(
            payload,
            &record.source_system,
            &record.source_ref,
            source_path.as_deref(),
        )
    });
    let facet = payload.as_ref().and_then(import_facet_from_payload);

    ImportRecordPayload {
        id: record.id.into(),
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_title,
        facet,
        import_type: ImportTypeValue::from_domain(record.import_type),
        status: ImportStatusValue::from_domain(record.status),
        error_message,
        decision,
        skip_reason,
        title_id: title_id.map(Into::into),
        source_path,
        dest_path,
        started_at: parse_optional_datetime(record.started_at, "import history started_at"),
        finished_at: parse_optional_datetime(record.finished_at, "import history finished_at"),
        created_at: parse_required_datetime(&record.created_at, "import history created_at"),
    }
}

pub fn from_wanted_item(
    item: scryer_application::AcquisitionScopeState,
) -> scryer_application::AppResult<WantedItemPayload> {
    let source_provider = source_provider_from_grabbed_release(item.grabbed_release.as_deref());
    Ok(WantedItemPayload {
        id: item.id.into(),
        title_id: item.title_id.into(),
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: item.title_facet,
        library_id: item.library_id.map(Into::into),
        library_name: item.library_name,
        library_slug: item.library_slug,
        episode_id: item.episode_id.map(Into::into),
        collection_id: item.collection_id.map(Into::into),
        season_number: item.season_number,
        episode_number: item.episode_number,
        media_type: WantedMediaTypeValue::parse(&item.media_type).ok_or_else(|| {
            scryer_application::AppError::Validation(format!(
                "invalid wanted item media_type '{}'",
                item.media_type
            ))
        })?,
        last_search_at: parse_optional_datetime(item.last_search_at, "wanted item last_search_at"),
        status: WantedStatusValue::from_application(item.status),
        grabbed_release: item.grabbed_release,
        source_provider,
        current_score: item.current_score,
        latest_release_decision: item
            .latest_release_decision
            .map(from_release_decision)
            .transpose()?,
        mismatch_recovery_eligible: item.mismatch_recovery_eligible,
        // Relation-field state rows (`title.wantedItems`, `episode.wantedItem`) are
        // not the convergence display surface: the derived Missing /
        // Upgrades views carry live convergence; here we present a neutral default
        // so the shared payload stays well-typed off a bare state row.
        convergence_state: ConvergenceStateValue::Queued,
        indexers_covered: 0,
        indexers_routed: 0,
        recency_lane: RecencyLaneValue::Cold,
        created_at: parse_datetime(&item.created_at, "wanted item created_at")
            .map_err(scryer_application::AppError::Validation)?,
        updated_at: parse_datetime(&item.updated_at, "wanted item updated_at")
            .map_err(scryer_application::AppError::Validation)?,
    })
}

/// Map a derived Missing/Upgrades view row onto the shared `WantedItemPayload`
///. The payload `id` is the scope identity: the state-row id when a
/// state row exists, else the convergence scope key — so a derived target with no
/// state row is still addressable (pause/resume and the interactive search job both
/// accept a scope key). Convergence progress and the recency lane come from the
/// batched per-page derivation.
pub fn from_wanted_scope_view(
    view: scryer_application::WantedScopeView,
) -> scryer_application::AppResult<WantedItemPayload> {
    let id = view
        .state
        .as_ref()
        .map(|state| state.id.clone())
        .unwrap_or_else(|| view.scope_key.clone());
    let state = view.state;
    let grabbed_release = state
        .as_ref()
        .and_then(|state| state.grabbed_release.clone());
    let source_provider = source_provider_from_grabbed_release(grabbed_release.as_deref());
    Ok(WantedItemPayload {
        id: id.into(),
        title_id: view.title_id.into(),
        title_name: view.title_name,
        title_slug: view.title_slug,
        title_facet: Some(view.facet.as_str().to_string()),
        library_id: Some(view.library_id.into()),
        library_name: view.library_name,
        library_slug: view.library_slug,
        episode_id: view.episode_id.map(Into::into),
        collection_id: view.collection_id.map(Into::into),
        season_number: view.season_number,
        episode_number: view.episode_number,
        media_type: WantedMediaTypeValue::parse(&view.media_type).ok_or_else(|| {
            scryer_application::AppError::Validation(format!(
                "invalid wanted view media_type '{}'",
                view.media_type
            ))
        })?,
        last_search_at: state.as_ref().and_then(|state| {
            parse_optional_datetime(state.last_search_at.clone(), "wanted view last_search_at")
        }),
        status: state
            .as_ref()
            .map(|state| WantedStatusValue::from_application(state.status))
            .unwrap_or(WantedStatusValue::Wanted),
        grabbed_release,
        source_provider,
        current_score: state.as_ref().and_then(|state| state.current_score),
        latest_release_decision: state
            .as_ref()
            .and_then(|state| state.latest_release_decision.clone())
            .map(from_release_decision)
            .transpose()?,
        mismatch_recovery_eligible: state
            .as_ref()
            .is_some_and(|state| state.mismatch_recovery_eligible),
        convergence_state: convergence_state_value(view.convergence.state),
        indexers_covered: view.convergence.indexers_covered,
        indexers_routed: view.convergence.indexers_routed,
        recency_lane: if view.is_hot {
            RecencyLaneValue::Hot
        } else {
            RecencyLaneValue::Cold
        },
        created_at: state
            .as_ref()
            .and_then(|state| parse_datetime(&state.created_at, "wanted view created_at").ok())
            .unwrap_or_else(chrono::Utc::now),
        updated_at: state
            .as_ref()
            .and_then(|state| parse_datetime(&state.updated_at, "wanted view updated_at").ok())
            .unwrap_or_else(chrono::Utc::now),
    })
}

fn source_provider_from_grabbed_release(grabbed_release: Option<&str>) -> Option<String> {
    let grabbed_release = serde_json::from_str::<Value>(grabbed_release?).ok()?;
    let source_provider = grabbed_release
        .get("source_provider")
        .or_else(|| grabbed_release.get("indexer"))
        .and_then(Value::as_str)?;
    let source_provider = source_provider.trim();
    (!source_provider.is_empty()
        && !source_provider.contains([':', '/', '\\', '?', '#', '@', '|', '=']))
    .then(|| source_provider.to_string())
}

fn convergence_state_value(
    state: scryer_application::WantedConvergenceState,
) -> ConvergenceStateValue {
    match state {
        scryer_application::WantedConvergenceState::Queued => ConvergenceStateValue::Queued,
        scryer_application::WantedConvergenceState::Searching => ConvergenceStateValue::Searching,
        scryer_application::WantedConvergenceState::Converged => ConvergenceStateValue::Converged,
        scryer_application::WantedConvergenceState::Deferred => ConvergenceStateValue::Deferred,
    }
}

pub fn from_release_decision(
    decision: scryer_application::ReleaseDecision,
) -> scryer_application::AppResult<ReleaseDecisionPayload> {
    Ok(ReleaseDecisionPayload {
        id: decision.id.into(),
        wanted_item_id: decision.wanted_item_id.into(),
        title_id: decision.title_id.into(),
        release_title: decision.release_title,
        release_url: decision.release_url,
        release_size_bytes: decision.release_size_bytes.map(Long::from),
        decision_code: decision.decision_code,
        candidate_score: decision.candidate_score,
        current_score: decision.current_score,
        score_delta: decision.score_delta,
        explanation_json: decision.explanation_json.map(json_string_to_value),
        created_at: parse_datetime(&decision.created_at, "release decision created_at")
            .map_err(scryer_application::AppError::Validation)?,
    })
}

pub fn from_decision_code_count(
    item: scryer_application::DecisionCodeCount,
) -> DecisionCodeCountPayload {
    DecisionCodeCountPayload {
        code: item.code,
        count: item.count,
    }
}

pub fn from_wanted_status_count(
    item: scryer_application::WantedStatusCount,
) -> WantedStatusCountPayload {
    WantedStatusCountPayload {
        status: scryer_application::AcquisitionScopeStatus::parse(&item.status)
            .map(WantedStatusValue::from_application)
            .unwrap_or(WantedStatusValue::Wanted),
        count: item.count,
    }
}

pub fn from_pending_release_status_count(
    item: scryer_application::PendingReleaseStatusCount,
) -> PendingReleaseStatusCountPayload {
    PendingReleaseStatusCountPayload {
        status: scryer_application::PendingReleaseStatus::parse(&item.status)
            .map(PendingReleaseStatusValue::from_application)
            .unwrap_or(PendingReleaseStatusValue::Waiting),
        count: item.count,
    }
}

pub fn from_title_acquisition_diagnostics(
    value: scryer_application::TitleAcquisitionDiagnostics,
) -> scryer_application::AppResult<TitleAcquisitionDiagnosticsPayload> {
    Ok(TitleAcquisitionDiagnosticsPayload {
        recent_decisions: value
            .recent_decisions
            .into_iter()
            .map(from_release_decision)
            .collect::<scryer_application::AppResult<Vec<_>>>()?,
        decision_counts: value
            .decision_counts
            .into_iter()
            .map(from_decision_code_count)
            .collect(),
        wanted_status_counts: value
            .wanted_status_counts
            .into_iter()
            .map(from_wanted_status_count)
            .collect(),
        pending_release_counts: value
            .pending_release_counts
            .into_iter()
            .map(from_pending_release_status_count)
            .collect(),
        mismatch_recovery_eligible_count: value.mismatch_recovery_eligible_count,
        latest_decision_at: parse_optional_datetime(
            value.latest_decision_at,
            "title acquisition latest_decision_at",
        ),
        latest_wanted_search_at: parse_optional_datetime(
            value.latest_wanted_search_at,
            "title acquisition latest_wanted_search_at",
        ),
    })
}

pub fn from_runtime_path_style(
    style: scryer_application::RuntimePathStyle,
) -> RuntimePathStyleValue {
    match style {
        scryer_application::RuntimePathStyle::Unix => RuntimePathStyleValue::Unix,
        scryer_application::RuntimePathStyle::Windows => RuntimePathStyleValue::Windows,
    }
}

pub fn from_system_health(health: SystemHealth) -> SystemHealthPayload {
    SystemHealthPayload {
        service_ready: health.service_ready,
        db_path: health.db_path,
        datastore_engine: health.datastore_engine,
        datastore_migration_key: health.datastore_migration_key,
        runtime_path_style: from_runtime_path_style(health.runtime_path_style),
        total_titles: health.total_titles as i32,
        monitored_titles: health.monitored_titles as i32,
        total_users: health.total_users as i32,
        titles_movie: health.titles_movie as i32,
        titles_series: health.titles_series as i32,
        titles_anime: health.titles_anime as i32,
        titles_other: health.titles_other as i32,
        recent_events: health.recent_events as i32,
        recent_event_preview: health.recent_event_preview,
        db_migration_version: health.db_migration_version,
        indexer_stats: health
            .indexer_stats
            .into_iter()
            .map(|s| IndexerQueryStatsPayload {
                indexer_id: s.indexer_id.into(),
                indexer_name: s.indexer_name,
                queries_last_24h: s.queries_last_24h as i32,
                successful_last_24h: s.successful_last_24h as i32,
                failed_last_24h: s.failed_last_24h as i32,
                last_query_at: parse_optional_datetime(
                    s.last_query_at,
                    "indexer stats last_query_at",
                ),
                api_current: s.api_current.map(|v| v as i32),
                api_max: s.api_max.map(|v| v as i32),
                grab_current: s.grab_current.map(|v| v as i32),
                grab_max: s.grab_max.map(|v| v as i32),
            })
            .collect(),
    }
}

pub fn from_smg_version_compatibility_notice(
    notice: SmgVersionCompatibilityNotice,
) -> SmgVersionCompatibilityNoticePayload {
    SmgVersionCompatibilityNoticePayload {
        status: notice.status,
        minimum_version: notice.minimum_version,
        your_version: notice.your_version,
        message: notice.message,
        upgrade_deadline: notice.upgrade_deadline,
    }
}

pub fn from_smg_scryer_update_notice(
    notice: SmgScryerUpdateNotice,
) -> SmgScryerUpdateNoticePayload {
    SmgScryerUpdateNoticePayload {
        available: notice.available,
        current_version: notice.current_version,
        latest_version: notice.latest_version,
        latest_tag: notice.latest_tag,
        release_url: notice.release_url,
        published_at: parse_optional_datetime(notice.published_at, "SMG notice published_at"),
        checked_at: parse_required_datetime(&notice.checked_at, "SMG notice checked_at"),
    }
}

pub fn from_rule_set(rs: RuleSet) -> RuleSetPayload {
    RuleSetPayload {
        id: rs.id.into(),
        name: rs.name,
        description: rs.description,
        rego_source: scryer_rules::strip_editor_source(&rs.rego_source),
        enabled: rs.enabled,
        priority: rs.priority,
        applied_facets: rs
            .applied_facets
            .iter()
            .map(|f| format!("{:?}", f).to_lowercase())
            .collect(),
        is_managed: rs.is_managed,
        managed_key: rs.managed_key,
        managed_tag_filter: rs.managed_tag_filter,
        created_at: rs.created_at,
        updated_at: rs.updated_at,
    }
}

pub fn from_registry_plugin(p: RegistryPlugin) -> RegistryPluginPayload {
    RegistryPluginPayload {
        id: p.id.into(),
        name: p.name,
        description: p.description,
        version: p.version,
        latest_version: p.latest_version,
        plugin_type: p.plugin_type,
        provider_type: p.provider_type,
        author: p.author,
        official: p.official,
        publisher: p.publisher,
        support_tier: support_tier_label(p.support_tier),
        status: p.status,
        docs_url: p.docs_url,
        source_repo: p.source_repo,
        builtin: p.builtin,
        source_url: p.source_url,
        source_kind: p.source_kind,
        blocked_reason: p.blocked_reason,
        bytes: p.bytes.map(Long::from_u64_saturating),
        is_installed: p.is_installed,
        is_enabled: p.is_enabled,
        installed_version: p.installed_version,
        update_available: p.update_available,
        install_in_progress: p.install_in_progress,
        default_base_url: p.default_base_url,
    }
}

pub fn from_plugin_install_progress(
    snapshot: scryer_application::PluginInstallProgressSnapshot,
) -> PluginInstallProgressPayload {
    PluginInstallProgressPayload {
        plugin_id: snapshot.plugin_id.into(),
        operation_kind: match snapshot.operation_kind {
            scryer_application::PluginInstallOperationKind::Install => {
                PluginInstallOperationKindValue::Install
            }
            scryer_application::PluginInstallOperationKind::Upgrade => {
                PluginInstallOperationKindValue::Upgrade
            }
        },
        state: match snapshot.state {
            scryer_application::PluginInstallState::Downloading => {
                PluginInstallStateValue::Downloading
            }
            scryer_application::PluginInstallState::Verifying => PluginInstallStateValue::Verifying,
            scryer_application::PluginInstallState::Installing => {
                PluginInstallStateValue::Installing
            }
            scryer_application::PluginInstallState::Succeeded => PluginInstallStateValue::Succeeded,
            scryer_application::PluginInstallState::Failed => PluginInstallStateValue::Failed,
        },
        label: snapshot.label,
        step_index: snapshot.step_index,
        step_count: snapshot.step_count,
        message: snapshot.message,
        error: snapshot.error,
    }
}

pub fn from_external_import_monitor_warmup_progress(
    snapshot: scryer_application::ExternalImportMonitorWarmupProgressSnapshot,
) -> ExternalImportMonitorWarmupProgressPayload {
    let map_phase_progress =
        |progress: scryer_application::ExternalImportMonitorWarmupPhaseProgress| {
            LibraryScanPhaseProgressPayload {
                total: progress.total,
                completed: progress.completed,
                failed: progress.failed,
            }
        };

    ExternalImportMonitorWarmupProgressPayload {
        session_id: snapshot.session_id.into(),
        status: match snapshot.status {
            scryer_application::ExternalImportMonitorWarmupStatus::Queued => {
                ExternalImportMonitorWarmupStatusValue::Queued
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Running => {
                ExternalImportMonitorWarmupStatusValue::Running
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Completed => {
                ExternalImportMonitorWarmupStatusValue::Completed
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Canceled => {
                ExternalImportMonitorWarmupStatusValue::Canceled
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Failed => {
                ExternalImportMonitorWarmupStatusValue::Failed
            }
        },
        phase: match snapshot.phase {
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingIndexers => {
                ExternalImportMonitorWarmupPhaseValue::LoadingIndexers
            }
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingMovies => {
                ExternalImportMonitorWarmupPhaseValue::LoadingMovies
            }
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingSeries => {
                ExternalImportMonitorWarmupPhaseValue::LoadingSeries
            }
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingEpisodes => {
                ExternalImportMonitorWarmupPhaseValue::LoadingEpisodes
            }
            scryer_application::ExternalImportMonitorWarmupPhase::BuildingSnapshot => {
                ExternalImportMonitorWarmupPhaseValue::BuildingSnapshot
            }
            scryer_application::ExternalImportMonitorWarmupPhase::Ready => {
                ExternalImportMonitorWarmupPhaseValue::Ready
            }
        },
        started_at: parse_required_datetime(
            &snapshot.started_at,
            "external import warmup started_at",
        ),
        updated_at: parse_required_datetime(
            &snapshot.updated_at,
            "external import warmup updated_at",
        ),
        overall_total_known: snapshot.overall_total_known,
        overall_progress: map_phase_progress(snapshot.overall_progress),
        movies_total_known: snapshot.movies_total_known,
        movies_progress: map_phase_progress(snapshot.movies_progress),
        series_total_known: snapshot.series_total_known,
        series_progress: map_phase_progress(snapshot.series_progress),
        episode_fetch_total_known: snapshot.episode_fetch_total_known,
        episode_fetch_expected_total: snapshot.episode_fetch_expected_total,
        episode_fetch_expected_monitored_total: snapshot.episode_fetch_expected_monitored_total,
        episode_fetch_progress: map_phase_progress(snapshot.episode_fetch_progress),
        snapshot_build_total_known: snapshot.snapshot_build_total_known,
        snapshot_build_progress: map_phase_progress(snapshot.snapshot_build_progress),
        matched_movie_count: snapshot.matched_movie_count,
        matched_series_count: snapshot.matched_series_count,
        unmatched_movie_count: snapshot.unmatched_movie_count,
        unmatched_series_count: snapshot.unmatched_series_count,
        ambiguous_movie_count: snapshot.ambiguous_movie_count,
        ambiguous_series_count: snapshot.ambiguous_series_count,
        error_message: snapshot.error_message,
    }
}

pub fn from_notification_channel(
    ch: scryer_domain::NotificationChannelConfig,
) -> NotificationChannelPayload {
    from_notification_channel_with_fields(ch, &[])
}

pub fn from_notification_channel_with_fields(
    ch: scryer_domain::NotificationChannelConfig,
    config_fields: &[ConfigFieldDef],
) -> NotificationChannelPayload {
    let stored_secret_keys = stored_secret_keys_from_config_json(&ch.config_json, config_fields);
    NotificationChannelPayload {
        id: ch.id.into(),
        name: ch.name,
        channel_type: ch.channel_type.as_str().to_string(),
        config: provider_config_values_from_json_with_fields(Some(&ch.config_json), config_fields),
        stored_secret_keys,
        media_server_connection_id: ch.media_server_connection_id.map(Into::into),
        is_enabled: ch.is_enabled,
        created_at: ch.created_at,
        updated_at: ch.updated_at,
    }
}

pub fn from_notification_subscription(
    sub: scryer_domain::NotificationSubscription,
) -> NotificationSubscriptionPayload {
    NotificationSubscriptionPayload {
        id: sub.id.into(),
        channel_id: sub.channel_id.map(Into::into),
        target_kind: sub.target_kind.as_str().to_string(),
        target_id: sub.target_id.into(),
        event_type: sub.event_type.as_str().to_string(),
        scope: sub.scope,
        scope_id: sub.scope_id,
        is_enabled: sub.is_enabled,
        created_at: sub.created_at,
        updated_at: sub.updated_at,
    }
}

pub fn from_notification_target(
    target: scryer_domain::NotificationTarget,
) -> NotificationTargetPayload {
    NotificationTargetPayload {
        id: target.id.into(),
        target_kind: target.target_kind.as_str().to_string(),
        name: target.name,
        provider_type: target.provider_type,
        media_server_provider: target
            .media_server_provider
            .map(MediaServerProviderValue::from_domain),
        media_server_connection_id: target.media_server_connection_id.map(Into::into),
        is_enabled: target.is_enabled,
    }
}

pub fn from_domain_event(event: DomainEvent) -> DomainEventEnvelopePayload {
    let (stream_kind, stream_id) = match event.stream {
        scryer_domain::DomainEventStream::Global => (StreamKindValue::Global, None),
        scryer_domain::DomainEventStream::Title { title_id } => {
            (StreamKindValue::Title, Some(title_id))
        }
        scryer_domain::DomainEventStream::LibraryScan { session_id } => {
            (StreamKindValue::LibraryScan, Some(session_id))
        }
        scryer_domain::DomainEventStream::JobRun { run_id } => {
            (StreamKindValue::JobRun, Some(run_id))
        }
        scryer_domain::DomainEventStream::DownloadQueueItem { item_id } => {
            (StreamKindValue::DownloadQueueItem, Some(item_id))
        }
    };

    DomainEventEnvelopePayload {
        sequence: Long::from(event.sequence),
        event_id: event.event_id.into(),
        occurred_at: event.occurred_at,
        actor_kind: event.actor_kind.into(),
        actor_user_id: event.actor_user_id.map(Into::into),
        actor_display_name: event.actor_display_name,
        title_id: event.title_id.map(Into::into),
        facet: event.facet.map(MediaFacetValue::from_domain),
        event_type: DomainEventTypeValue::from_domain(event.payload.event_type()),
        stream_kind,
        stream_id: stream_id.map(Into::into),
        payload_json: async_graphql::Json(
            serde_json::to_value(event.payload).unwrap_or(serde_json::Value::Null),
        ),
    }
}

pub fn from_plugin_installation(inst: PluginInstallation) -> PluginInstallationPayload {
    PluginInstallationPayload {
        id: inst.id.into(),
        plugin_id: inst.plugin_id.into(),
        name: inst.name,
        description: inst.description,
        version: inst.version,
        sdk_version: inst.sdk_version,
        sdk_constraint: inst.sdk_constraint,
        plugin_type: inst.plugin_type,
        provider_type: inst.provider_type,
        is_enabled: inst.is_enabled,
        is_builtin: inst.is_builtin,
        source_kind: match inst.source_kind {
            scryer_domain::PluginSourceKind::Bundled => "bundled".to_string(),
            scryer_domain::PluginSourceKind::Downloaded => "downloaded".to_string(),
            scryer_domain::PluginSourceKind::Community => "community".to_string(),
            scryer_domain::PluginSourceKind::Manual => "manual".to_string(),
        },
        source_url: inst.source_url,
        publisher: inst.publisher,
        support_tier: support_tier_label(inst.support_tier),
        docs_url: inst.docs_url,
        source_repo: inst.source_repo,
        manifest_url: inst.manifest_url,
        wasm_digest: inst.wasm_digest,
        artifact_digest: inst.artifact_digest,
        installed_at: inst.installed_at,
        updated_at: inst.updated_at,
    }
}

pub fn from_plugin_catalog_status(status: PluginCatalogStatus) -> PluginCatalogStatusPayload {
    PluginCatalogStatusPayload {
        refresh_state: CatalogRefreshStateValue::from_app_str(&status.refresh_state),
        github_available: status.github_available,
        last_checked_at: parse_optional_datetime(
            status.last_checked_at,
            "plugin catalog last_checked_at",
        ),
        outage_message: status.outage_message,
        blocked_actions: status.blocked_actions,
        restore_warnings: status.restore_warnings,
        last_error: status.last_error,
    }
}

pub fn from_manual_plugin_preview(preview: ManualPluginPreview) -> ManualPluginPreviewPayload {
    ManualPluginPreviewPayload {
        github_repo_url: preview.github_repo_url,
        plugin: from_registry_plugin(preview.plugin),
    }
}

pub fn from_backup_info(info: BackupInfo) -> Result<BackupInfoPayload, String> {
    Ok(BackupInfoPayload {
        filename: info.filename,
        size_bytes: Long::from_u64_saturating(info.size_bytes),
        created_at: parse_datetime(&info.created_at, "backup created_at")?,
        format_version: info.format_version,
        source_engine: info.source_engine,
        source_migration_key: info.source_migration_key,
        encrypted: info.encrypted,
        row_counts: info
            .row_counts
            .into_iter()
            .map(|(table, row_count)| BackupRowCountPayload {
                table,
                row_count: Long::from_u64_saturating(row_count),
            })
            .collect(),
        trigger: info.trigger.as_str().to_string(),
        status: info.status.as_str().to_string(),
        error_message: info.error_message,
    })
}

pub fn from_rss_sync_report(report: RssSyncReport) -> RssSyncReportPayload {
    RssSyncReportPayload {
        releases_fetched: report.releases_fetched as i32,
        releases_matched: report.releases_matched as i32,
        releases_grabbed: report.releases_grabbed as i32,
        releases_held: report.releases_held as i32,
    }
}

pub fn from_pending_release(pr: PendingRelease) -> PendingReleasePayload {
    PendingReleasePayload {
        id: pr.id.into(),
        wanted_item_id: pr.wanted_item_id.into(),
        title_id: pr.title_id.into(),
        release_title: pr.release_title,
        release_url: pr.release_url,
        release_size_bytes: pr.release_size_bytes.map(Long::from),
        release_score: pr.release_score,
        scoring_log_json: pr.scoring_log_json.map(json_string_to_value),
        indexer_source: pr.indexer_source,
        added_at: parse_required_datetime(&pr.added_at, "pending release added_at"),
        delay_until: parse_required_datetime(&pr.delay_until, "pending release delay_until"),
        status: PendingReleaseStatusValue::from_application(pr.status),
    }
}

pub fn from_pp_script(s: scryer_domain::PostProcessingScript) -> PostProcessingScriptPayload {
    PostProcessingScriptPayload {
        id: s.id.into(),
        name: s.name,
        description: s.description,
        script_type: s.script_type.as_str().to_string(),
        script_content: s.script_content,
        applied_facets: s.applied_facets,
        execution_mode: s.execution_mode.into(),
        timeout_secs: s.timeout_secs as i32,
        priority: s.priority,
        enabled: s.enabled,
        debug: s.debug,
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

pub fn from_pp_script_run(
    r: scryer_domain::PostProcessingScriptRun,
) -> PostProcessingScriptRunPayload {
    PostProcessingScriptRunPayload {
        id: r.id.into(),
        script_id: r.script_id.into(),
        script_name: r.script_name,
        title_id: r.title_id.map(Into::into),
        title_name: r.title_name,
        facet: r.facet.as_deref().and_then(MediaFacetValue::parse),
        file_path: r.file_path,
        status: r.status.as_str().to_string(),
        exit_code: r.exit_code,
        stdout_tail: r.stdout_tail,
        stderr_tail: r.stderr_tail,
        duration_ms: r.duration_ms.map(|v| v as i32),
        started_at: parse_required_datetime(&r.started_at, "post-processing script run started_at"),
        completed_at: parse_optional_datetime(
            r.completed_at,
            "post-processing script run completed_at",
        ),
    }
}

pub fn from_title_history_record(
    record: TitleHistoryRecord,
) -> scryer_application::AppResult<TitleHistoryEventPayload> {
    Ok(TitleHistoryEventPayload {
        id: record.id.into(),
        title_id: record.title_id.into(),
        title_name: record.title_name,
        facet: record.facet.map(MediaFacetValue::from_domain),
        episode_id: record.episode_id.map(Into::into),
        episode_ids: record.episode_ids.into_iter().map(Into::into).collect(),
        collection_id: record.collection_id.map(Into::into),
        event_type: record.event_type.as_str().to_string(),
        actor_kind: record.actor_kind.map(Into::into),
        actor_user_id: record.actor_user_id.map(Into::into),
        actor_display_name: record.actor_display_name,
        source_title: record.source_title,
        display_title: record.display_title,
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_provider: record.source_hint.clone(),
        source_hint: record.source_hint,
        quality: record.quality,
        download_id: record.download_id,
        client_id: record.client_id.map(Into::into),
        client_name: record.client_name,
        import_id: record.import_id.map(Into::into),
        skip_reason: record.skip_reason,
        retry_requires_password: record.retry_requires_password,
        failure_reason: record.failure_reason,
        blocklist_reason: record.blocklist_reason,
        source_path: record.source_path,
        dest_path: record.dest_path,
        data_json: record.data_json.map(json_string_to_value),
        occurred_at: parse_datetime(&record.occurred_at, "title history occurred_at")
            .map_err(scryer_application::AppError::Validation)?,
        created_at: parse_datetime(&record.created_at, "title history created_at")
            .map_err(scryer_application::AppError::Validation)?,
    })
}

pub fn from_title_history_page(
    page: TitleHistoryPage,
    offset: usize,
) -> scryer_application::AppResult<TitleHistoryPagePayload> {
    let items = page
        .records
        .into_iter()
        .map(from_title_history_record)
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let has_more = (offset.saturating_add(items.len()) as i64) < page.total_count;
    Ok(TitleHistoryPagePayload {
        items,
        total_count: page.total_count,
        has_more,
    })
}

/// Media-request storage keeps the flattened, lowercase normalization of the
/// monitor type ("futureepisodes"); the API exposes the typed enum.
fn monitor_type_value_from_normalized(value: &str) -> Option<MonitorTypeValue> {
    match value {
        "monitored" => Some(MonitorTypeValue::Monitored),
        "unmonitored" => Some(MonitorTypeValue::Unmonitored),
        "futureepisodes" => Some(MonitorTypeValue::FutureEpisodes),
        "missingandfutureepisodes" => Some(MonitorTypeValue::MissingAndFutureEpisodes),
        "allepisodes" => Some(MonitorTypeValue::AllEpisodes),
        "none" => Some(MonitorTypeValue::NoneSelected),
        _ => None,
    }
}

/// Boundary conversion for JSON persisted as text in the application layer:
/// the wire carries real JSON, never a string-encoded document.
pub fn json_string_to_value(raw: String) -> async_graphql::Json<serde_json::Value> {
    async_graphql::Json(serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::{
        discovery_home_query_from_input, discovery_surface_value, from_import_record,
        from_title_history_record, from_wanted_item, preferred_discovery_poster_source,
        provider_config_values_from_json_with_fields, provider_config_values_to_json,
    };
    use crate::types::{
        BoolConfigValuePayload, DiscoveryHomeFiltersInput, DiscoveryHomeInput,
        DiscoverySurfaceValue, FloatConfigValuePayload, IntConfigValuePayload, MediaFacetValue,
        PluginConfigFieldTypeValue, ProviderConfigFieldValue, ProviderConfigValueInput,
        SecretConfigValuePayload,
    };
    use scryer_application::{AcquisitionScopeState, AcquisitionScopeStatus};
    use scryer_domain::{
        CompletedDownload, ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource, ImportRecord,
        ImportStatus, ImportType, TitleHistoryEventType, TitleHistoryRecord,
    };
    use serde_json::{Value, json};

    fn config_field(key: &str, label: &str, field_type: ConfigFieldType) -> ConfigFieldDef {
        ConfigFieldDef {
            key: key.to_string(),
            label: label.to_string(),
            field_type,
            required: field_type == ConfigFieldType::Password,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: Vec::new(),
            help_text: None,
        }
    }

    fn config_input(key: &str) -> ProviderConfigValueInput {
        ProviderConfigValueInput {
            key: key.to_string(),
            string_value: None,
            bool_value: None,
            int_value: None,
            float_value: None,
            secret_value: None,
            clear_secret: None,
        }
    }

    #[test]
    fn discovery_posters_prefer_tmdb_paths_over_tvdb_urls() {
        assert_eq!(
            preferred_discovery_poster_source(
                Some("/poster.jpg"),
                Some("https://artworks.thetvdb.com/banners/poster.jpg"),
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/original/poster.jpg")
        );
    }

    #[test]
    fn discovery_posters_retain_the_supplied_url_without_a_tmdb_path() {
        assert_eq!(
            preferred_discovery_poster_source(
                Some("not-a-tmdb-path"),
                Some("https://artworks.thetvdb.com/banners/poster.jpg"),
            )
            .as_deref(),
            Some("https://artworks.thetvdb.com/banners/poster.jpg")
        );
    }

    fn wanted_item_fixture() -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: Some("Example".to_string()),
            title_slug: Some("example".to_string()),
            title_facet: Some("movie".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2026-06-19T00:00:00Z".to_string(),
            updated_at: "2026-06-19T00:00:00Z".to_string(),
        }
    }

    fn title_history_record_fixture() -> TitleHistoryRecord {
        TitleHistoryRecord {
            id: "history-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: Some("Example".to_string()),
            facet: None,
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: None,
            event_type: TitleHistoryEventType::Grabbed,
            actor_kind: None,
            actor_user_id: None,
            actor_display_name: None,
            source_title: None,
            display_title: None,
            source_system: None,
            source_ref: None,
            source_hint: None,
            quality: None,
            download_id: None,
            client_id: None,
            client_name: None,
            import_id: None,
            skip_reason: None,
            retry_requires_password: false,
            failure_reason: None,
            blocklist_reason: None,
            source_path: None,
            dest_path: None,
            data_json: None,
            occurred_at: "2026-06-19T00:00:00Z".to_string(),
            created_at: "2026-06-19T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn provider_config_projection_redacts_secret_values() {
        let fields = vec![
            config_field("api_key", "API key", ConfigFieldType::Password),
            config_field("base_url", "Base URL", ConfigFieldType::String),
        ];
        let values = provider_config_values_from_json_with_fields(
            Some(
                r#"{
                  "api_key": "super-secret",
                  "username": "operator",
                  "base_url": "https://example.test",
                  "enabled": true,
                  "retries": 3,
                  "ratio": 1.5,
                  "metadata": {"nested": true}
                }"#,
            ),
            &fields,
        );

        let field = |key: &str| {
            values
                .iter()
                .find(|value| value.key == key)
                .unwrap_or_else(|| panic!("{key} should be present"))
        };

        let api_key = field("api_key");
        assert_eq!(api_key.label.as_deref(), Some("API key"));
        assert!(matches!(
            api_key.field_type,
            Some(PluginConfigFieldTypeValue::Password)
        ));
        assert!(api_key.required);
        assert!(matches!(
            api_key.value,
            Some(ProviderConfigFieldValue::Secret(SecretConfigValuePayload {
                stored: true
            }))
        ));
        let username = field("username");
        assert!(matches!(
            username.value,
            Some(ProviderConfigFieldValue::Secret(SecretConfigValuePayload {
                stored: true
            }))
        ));

        let base_url = field("base_url");
        match &base_url.value {
            Some(ProviderConfigFieldValue::String(payload)) => {
                assert_eq!(payload.value, "https://example.test");
            }
            _ => panic!("base_url should be a string value"),
        }
        assert!(matches!(
            field("enabled").value,
            Some(ProviderConfigFieldValue::Bool(BoolConfigValuePayload {
                value: true
            }))
        ));
        assert!(matches!(
            field("retries").value,
            Some(ProviderConfigFieldValue::Int(IntConfigValuePayload {
                value: 3
            }))
        ));
        assert!(matches!(
            field("ratio").value,
            Some(ProviderConfigFieldValue::Float(FloatConfigValuePayload { value })) if value == 1.5
        ));
        let metadata = field("metadata");
        assert!(metadata.value.is_none());
    }

    #[test]
    fn provider_config_inputs_use_secret_value_and_clear_secret() {
        let mut secret = config_input("api_key");
        secret.secret_value = Some("replacement-secret".to_string());
        let mut cleared = config_input("optional_password");
        cleared.clear_secret = Some(true);
        let mut base_url = config_input("base_url");
        base_url.string_value = Some("https://example.test".to_string());

        let raw = provider_config_values_to_json(vec![secret, cleared, base_url])
            .expect("typed config values should serialize");
        let value: Value = serde_json::from_str(&raw).expect("config should be valid json");

        assert_eq!(value["api_key"], json!("replacement-secret"));
        assert_eq!(value["optional_password"], Value::Null);
        assert_eq!(value["base_url"], json!("https://example.test"));
    }

    #[test]
    fn provider_config_inputs_reject_ambiguous_secret_values() {
        let mut value = config_input("api_key");
        value.secret_value = Some("replacement-secret".to_string());
        value.string_value = Some("projected-secret".to_string());

        let err = provider_config_values_to_json(vec![value])
            .expect_err("ambiguous config values should be rejected");
        assert!(
            err.to_string()
                .contains("config value 'api_key' must provide exactly one value"),
            "{err}"
        );
    }

    #[test]
    fn wanted_item_mapper_rejects_invalid_persisted_media_type() {
        let mut item = wanted_item_fixture();
        item.media_type = "mystery".to_string();

        let err = match from_wanted_item(item) {
            Ok(_) => panic!("invalid media type should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("invalid wanted item media_type 'mystery'"),
            "{err}"
        );
    }

    #[test]
    fn title_history_mapper_rejects_invalid_persisted_timestamps() {
        let mut record = title_history_record_fixture();
        record.occurred_at = "not-a-date".to_string();

        let err = match from_title_history_record(record) {
            Ok(_) => panic!("invalid timestamp should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("invalid title history occurred_at timestamp"),
            "{err}"
        );
    }

    #[test]
    fn from_import_record_uses_release_folder_for_numeric_weaver_job_name() {
        let payload = CompletedDownload {
            client_type: "weaver".to_string(),
            client_id: String::new(),
            download_client_item_id: "10495".to_string(),
            download_id: None,
            name: "10495".to_string(),
            dest_dir: "/downloads/Example.Show.S01E01.1080p.WEB-DL".to_string(),
            category: Some("anime".to_string()),
            size_bytes: None,
            completed_at: None,
            parameters: vec![("*scryer_facet".to_string(), "anime".to_string())],
        };
        let record = ImportRecord {
            id: "import-1".to_string(),
            source_client_id: None,
            source_system: "weaver".to_string(),
            source_ref: "10495".to_string(),
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            import_type: ImportType::SeriesDownload,
            status: ImportStatus::Completed,
            payload_json: serde_json::to_string(&payload).expect("serialize completed download"),
            result_json: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-04-27T20:17:00Z".to_string(),
            updated_at: "2026-04-27T20:17:00Z".to_string(),
        };

        let mapped = from_import_record(record);
        assert_eq!(
            mapped.source_title.as_deref(),
            Some("Example.Show.S01E01.1080p.WEB-DL")
        );
        assert!(matches!(mapped.facet, Some(MediaFacetValue::Anime)));
    }

    #[test]
    fn discovery_home_filters_require_valid_ranges() {
        let invalid_rating = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                minimum_rating: Some(10.1),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("ratings above ten must be rejected");
        assert!(invalid_rating.to_string().contains("minimumRating"));

        let invalid_years = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                minimum_year: Some(2025),
                maximum_year: Some(2024),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("inverted years must be rejected");
        assert!(invalid_years.to_string().contains("minimumYear"));

        let invalid_limit = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            limit_per_section: Some(0),
            ..Default::default()
        }))
        .expect_err("zero card limits must be rejected");
        assert!(invalid_limit.to_string().contains("limitPerSection"));

        let blank_tag_key = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                genre_tag_keys: Some(vec!["  ".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("blank canonical tag keys must be rejected");
        assert!(blank_tag_key.to_string().contains("genreTagKeys"));
    }

    #[test]
    fn discovery_home_filters_map_media_enums_and_canonical_tag_keys() {
        let query = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                content_types: Some(vec![MediaFacetValue::Anime, MediaFacetValue::Movie]),
                genre_tag_keys: Some(vec!["Canonical:Genre:Drama".to_string()]),
                theme_tag_keys: Some(vec!["canonical:theme:found-family".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect("semantic filters should map");

        assert_eq!(query.filters.content_types, ["anime", "movie"]);
        assert_eq!(query.filters.genre_tag_keys, ["Canonical:Genre:Drama"]);
        assert_eq!(
            query.filters.theme_tag_keys,
            ["canonical:theme:found-family"]
        );
    }

    #[test]
    fn discovery_home_section_maps_mixed_surface() {
        let surface =
            discovery_surface_value("mixed").expect("mixed is a supported discovery home surface");

        assert!(matches!(surface, DiscoverySurfaceValue::Mixed));
    }

    #[test]
    fn discovery_home_section_rejects_unknown_surface_without_panicking() {
        let error = discovery_surface_value("unknown")
            .err()
            .expect("unknown surfaces must be returned as validation errors");

        assert!(error.to_string().contains("unsupported surface"));
    }

    #[test]
    fn discovery_home_rejects_unknown_media_facets_without_panicking() {
        let error = super::discovery_home_media_facet("documentary", "card targetKind")
            .err()
            .expect("unsupported media facets must be returned as validation errors");

        assert!(error.to_string().contains("card targetKind"));
    }
}
