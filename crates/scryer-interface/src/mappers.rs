use scryer_application::{
    ActivityEvent, BackupInfo, DiskSpaceInfo, HealthCheckResult, HousekeepingReport,
    IndexerSearchResult, LibraryScanSummary, ParsedEpisodeMetadata, ParsedReleaseMetadata,
    PendingRelease, QualityProfile, QualityProfileCriteria, QualityProfileDecision, RegistryPlugin,
    RenameApplyItemResult, RenameApplyResult, RenamePlan, RenamePlanItem, RssSyncReport,
    ScoringEntry, ScoringSource, SystemHealth, TitleHistoryPage, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    CalendarEpisode, Collection, DownloadClientConfig, DownloadQueueItem, Episode, IndexerConfig,
    PluginInstallation, PolicyOutput, RuleSet, Title, TitleHistoryRecord, User,
};
use scryer_infrastructure::WorkflowOperationRecord;
use scryer_rules;
use std::fs;

use crate::types::*;

pub(crate) fn from_scoring_overrides(
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

pub(crate) fn from_content_scope(scope: &str) -> Option<ContentScopeValue> {
    ContentScopeValue::parse(scope)
}

pub(crate) fn from_quality_profile_criteria(
    criteria: QualityProfileCriteria,
) -> QualityProfileCriteriaPayload {
    let mut facet_persona_overrides: Vec<FacetScoringPersonaOverridePayload> = criteria
        .facet_persona_overrides
        .into_iter()
        .filter_map(|(scope, persona)| {
            from_content_scope(&scope).map(|scope| FacetScoringPersonaOverridePayload {
                scope,
                persona: ScoringPersonaValue::from_application(persona),
            })
        })
        .collect();
    facet_persona_overrides.sort_by_key(|entry| entry.scope.as_scope_id());

    QualityProfileCriteriaPayload {
        quality_tiers: criteria.quality_tiers,
        archival_quality: criteria.archival_quality,
        allow_unknown_quality: criteria.allow_unknown_quality,
        source_allowlist: criteria.source_allowlist,
        source_blocklist: criteria.source_blocklist,
        video_codec_allowlist: criteria.video_codec_allowlist,
        video_codec_blocklist: criteria.video_codec_blocklist,
        audio_codec_allowlist: criteria.audio_codec_allowlist,
        audio_codec_blocklist: criteria.audio_codec_blocklist,
        atmos_preferred: criteria.atmos_preferred,
        dolby_vision_allowed: criteria.dolby_vision_allowed,
        detected_hdr_allowed: criteria.detected_hdr_allowed,
        prefer_remux: criteria.prefer_remux,
        allow_bd_disk: criteria.allow_bd_disk,
        allow_upgrades: criteria.allow_upgrades,
        prefer_dual_audio: criteria.prefer_dual_audio,
        required_audio_languages: criteria.required_audio_languages,
        scoring_persona: ScoringPersonaValue::from_application(criteria.scoring_persona),
        scoring_overrides: from_scoring_overrides(criteria.scoring_overrides),
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
        facet_persona_overrides,
    }
}

pub(crate) fn from_quality_profile(profile: QualityProfile) -> QualityProfilePayload {
    QualityProfilePayload {
        id: profile.id,
        name: profile.name,
        criteria: from_quality_profile_criteria(profile.criteria),
    }
}

pub(crate) fn from_tvdb_scan_operation(
    operation: WorkflowOperationRecord,
    limit: i64,
    source: String,
) -> TvdbScanOperationPayload {
    TvdbScanOperationPayload {
        id: operation.id,
        operation_type: operation.operation_type,
        status: operation.status,
        actor_user_id: operation.actor_user_id,
        limit,
        source,
        started_at: operation.started_at,
        completed_at: operation.completed_at,
        created_at: operation.created_at,
        updated_at: operation.updated_at,
    }
}

pub(crate) fn from_search_result(result: IndexerSearchResult) -> IndexerSearchResultPayload {
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
        size_bytes: result.size_bytes,
        published_at: result.published_at,
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
    }
}

pub(crate) fn from_title_release_blocklist_entry(
    entry: TitleReleaseBlocklistEntry,
) -> TitleReleaseBlocklistEntryPayload {
    TitleReleaseBlocklistEntryPayload {
        source_hint: entry.source_hint,
        source_title: entry.source_title,
        error_message: entry.error_message,
        attempted_at: entry.attempted_at,
    }
}

pub(crate) fn from_quality_profile_decision(
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

pub(crate) fn from_parsed_release(result: ParsedReleaseMetadata) -> ParsedReleasePayload {
    ParsedReleasePayload {
        raw_title: result.raw_title,
        normalized_title: result.normalized_title,
        release_group: result.release_group,
        languages_audio: result.languages_audio,
        languages_subtitles: result.languages_subtitles,
        year: result.year.map(|value| value as i32),
        quality: result.quality,
        source: result.source,
        video_codec: result.video_codec,
        video_encoding: result.video_encoding,
        audio: result.audio,
        audio_channels: result.audio_channels,
        is_dual_audio: result.is_dual_audio,
        is_atmos: result.is_atmos,
        is_dolby_vision: result.is_dolby_vision,
        detected_hdr: result.detected_hdr,
        fps: result.fps,
        is_proper_upload: result.is_proper_upload,
        is_remux: result.is_remux,
        is_bd_disk: result.is_bd_disk,
        is_ai_enhanced: result.is_ai_enhanced,
        parser_version: result.parser_version.to_string(),
        parse_confidence: result.parse_confidence,
        missing_fields: result.missing_fields,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
    }
}

pub(crate) fn from_parsed_episode(episode: ParsedEpisodeMetadata) -> ParsedEpisodePayload {
    ParsedEpisodePayload {
        season: episode.season.map(|value| value as i32),
        episode_numbers: episode
            .episode_numbers
            .into_iter()
            .map(|value| value as i32)
            .collect(),
        absolute_episode: episode.absolute_episode.map(|value| value as i32),
        raw: episode.raw,
    }
}

pub(crate) fn from_indexer_config(config: IndexerConfig) -> IndexerConfigPayload {
    IndexerConfigPayload {
        id: config.id,
        name: config.name,
        provider_type: config.provider_type,
        base_url: config.base_url,
        has_api_key: config
            .api_key_encrypted
            .as_ref()
            .is_some_and(|value| !value.is_empty()),
        rate_limit_seconds: config.rate_limit_seconds,
        rate_limit_burst: config.rate_limit_burst,
        disabled_until: config.disabled_until.map(|value| value.to_rfc3339()),
        is_enabled: config.is_enabled,
        enable_interactive_search: config.enable_interactive_search,
        enable_auto_search: config.enable_auto_search,
        last_health_status: config.last_health_status,
        last_error_at: config.last_error_at.map(|value| value.to_rfc3339()),
        last_query_at: None,
        config_json: config.config_json,
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_provider_type(
    provider_type: String,
    name: String,
    config_fields: Vec<scryer_domain::ConfigFieldDef>,
    default_base_url: Option<String>,
) -> ProviderTypePayload {
    ProviderTypePayload {
        provider_type,
        name,
        default_base_url,
        config_fields: config_fields
            .into_iter()
            .map(|f| PluginConfigFieldPayload {
                key: f.key,
                label: f.label,
                field_type: f.field_type.as_str().to_string(),
                required: f.required,
                default_value: f.default_value,
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

pub(crate) fn from_download_client_config(
    config: DownloadClientConfig,
) -> DownloadClientConfigPayload {
    let base_url = scryer_infrastructure::resolve_base_url_from_config_json(&config.config_json);
    DownloadClientConfigPayload {
        id: config.id,
        name: config.name,
        client_type: config.client_type,
        base_url,
        config_json: config.config_json,
        is_enabled: config.is_enabled,
        status: config.status.as_str().to_string(),
        last_error: config.last_error,
        last_seen_at: config.last_seen_at.map(|value| value.to_rfc3339()),
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_download_queue_item(item: DownloadQueueItem) -> DownloadQueueItemPayload {
    DownloadQueueItemPayload {
        id: item.id,
        title_id: item.title_id,
        title_name: item.title_name,
        facet: item.facet.as_deref().and_then(MediaFacetValue::parse),
        is_scryer_origin: item.is_scryer_origin,
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
        client_id: item.client_id,
        client_name: item.client_name,
        client_type: item.client_type,
        state: DownloadQueueStateValue::from_domain(item.state),
        progress_percent: i32::from(item.progress_percent),
        size_bytes: item.size_bytes.map(|value| value.to_string()),
        remaining_seconds: item
            .remaining_seconds
            .and_then(|value| i32::try_from(value).ok()),
        queued_at: item.queued_at,
        last_updated_at: item.last_updated_at,
        attention_required: item.attention_required,
        attention_reason: item.attention_reason,
        download_client_item_id: item.download_client_item_id,
        import_status: item.import_status.map(ImportStatusValue::from_domain),
        import_error_message: item.import_error_message,
        imported_at: item.imported_at,
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

pub(crate) fn from_title(title: Title) -> TitlePayload {
    let quality_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:");
    let root_folder_path = extract_tag_string(&title.tags, "scryer:root-folder:");
    let monitor_type = extract_tag_string(&title.tags, "scryer:monitor-type:")
        .as_deref()
        .and_then(MonitorTypeValue::from_tag_value);
    let use_season_folders = extract_tag_string(&title.tags, "scryer:season-folder:")
        .map(|value| !value.eq_ignore_ascii_case("disabled"));
    let monitor_specials = extract_tag_bool(&title.tags, "scryer:monitor-specials:");
    let inter_season_movies = extract_tag_bool(&title.tags, "scryer:inter-season-movies:");
    let filler_policy = extract_tag_string(&title.tags, "scryer:filler-policy:");
    let recap_policy = extract_tag_string(&title.tags, "scryer:recap-policy:");

    TitlePayload {
        id: title.id,
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
        created_by: title.created_by,
        created_at: title.created_at.to_rfc3339(),
        year: title.year,
        overview: title.overview,
        poster_url: title.poster_url,
        poster_source_url: title.poster_source_url,
        banner_url: title.banner_url,
        banner_source_url: title.banner_source_url,
        background_url: title.background_url,
        background_source_url: title.background_source_url,
        sort_title: title.sort_title,
        slug: title.slug,
        imdb_id: title.imdb_id,
        runtime_minutes: title.runtime_minutes,
        genres: title.genres,
        content_status: title.content_status,
        language: title.language,
        first_aired: title.first_aired,
        network: title.network,
        studio: title.studio,
        country: title.country,
        aliases: title.aliases,
        metadata_language: title.metadata_language,
        metadata_fetched_at: title.metadata_fetched_at.map(|dt| dt.to_rfc3339()),
        min_availability: title.min_availability,
        digital_release_date: title.digital_release_date,
        quality_profile_id,
        root_folder_path,
        monitor_type,
        use_season_folders,
        monitor_specials,
        inter_season_movies,
        filler_policy,
        recap_policy,
        quality_tier: None,
        size_bytes: None,
        episodes_owned: None,
        episodes_total: None,
    }
}

pub(crate) fn from_library_scan_summary(summary: LibraryScanSummary) -> LibraryScanSummaryPayload {
    LibraryScanSummaryPayload {
        scanned: summary.scanned as i32,
        matched: summary.matched as i32,
        imported: summary.imported as i32,
        skipped: summary.skipped as i32,
        unmatched: summary.unmatched as i32,
    }
}

pub(crate) fn from_library_scan_phase_progress(
    progress: scryer_application::LibraryScanPhaseProgress,
) -> LibraryScanPhaseProgressPayload {
    LibraryScanPhaseProgressPayload {
        total: progress.total as i32,
        completed: progress.completed as i32,
        failed: progress.failed as i32,
    }
}

pub(crate) fn from_library_scan_session(
    session: scryer_application::LibraryScanSession,
) -> LibraryScanProgressPayload {
    LibraryScanProgressPayload {
        session_id: session.session_id,
        facet: MediaFacetValue::from_domain(session.facet),
        status: LibraryScanStatusValue::from_application(session.status),
        started_at: session.started_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        found_titles: session.found_titles as i32,
        metadata_total_known: session.metadata_total_known,
        file_total_known: session.file_total_known,
        metadata_progress: from_library_scan_phase_progress(session.metadata_progress),
        file_progress: from_library_scan_phase_progress(session.file_progress),
        summary: session.summary.map(from_library_scan_summary),
    }
}

pub(crate) fn from_media_rename_plan(plan: RenamePlan) -> MediaRenamePlanPayload {
    MediaRenamePlanPayload {
        facet: MediaFacetValue::from_domain(plan.facet),
        title_id: plan.title_id,
        template: plan.template,
        collision_policy: plan.collision_policy.as_str().to_string(),
        missing_metadata_policy: plan.missing_metadata_policy.as_str().to_string(),
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
        collection_id: item.collection_id,
        media_file_id: item.media_file_id,
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        normalized_filename: item.normalized_filename,
        collision: item.collision,
        reason_code: item.reason_code,
        write_action: item.write_action.as_str().to_string(),
        source_size_bytes: item.source_size_bytes.map(|value| value.to_string()),
        source_mtime_unix_ms: item.source_mtime_unix_ms.map(|value| value.to_string()),
    }
}

pub(crate) fn from_media_rename_apply(result: RenameApplyResult) -> MediaRenameApplyPayload {
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
        collection_id: item.collection_id,
        media_file_id: item.media_file_id,
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        final_path: item.final_path,
        write_action: item.write_action.as_str().to_string(),
        status: item.status.as_str().to_string(),
        reason_code: item.reason_code,
        error_message: item.error_message,
    }
}

pub(crate) fn from_collection(collection: Collection) -> CollectionPayload {
    let file_size_bytes = file_size_bytes_for_path(collection.ordered_path.as_deref());
    let map_movie =
        |movie: scryer_domain::InterstitialMovieMetadata| InterstitialMovieMetadataPayload {
            tvdb_id: movie.tvdb_id,
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            content_status: movie.content_status,
            overview: movie.overview,
            poster_url: movie.poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            genres: movie.genres,
            studio: movie.studio,
            digital_release_date: movie.digital_release_date,
            association_confidence: movie.association_confidence,
            continuity_status: movie.continuity_status,
            movie_form: movie.movie_form,
            confidence: movie.confidence,
            signal_summary: movie.signal_summary,
            placement: movie.placement,
            movie_tmdb_id: movie.movie_tmdb_id,
            movie_mal_id: movie.movie_mal_id,
        };
    CollectionPayload {
        id: collection.id,
        title_id: collection.title_id,
        collection_type: collection.collection_type.as_str().to_string(),
        collection_index: collection.collection_index,
        label: collection.label,
        ordered_path: collection.ordered_path,
        narrative_order: collection.narrative_order,
        file_size_bytes,
        first_episode_number: collection.first_episode_number,
        last_episode_number: collection.last_episode_number,
        interstitial_movie: collection.interstitial_movie.map(map_movie),
        interstitial_season_episode: collection.interstitial_season_episode,
        specials_movies: collection
            .specials_movies
            .into_iter()
            .map(map_movie)
            .collect(),
        monitored: collection.monitored,
        created_at: collection.created_at.to_rfc3339(),
    }
}

pub(crate) fn file_size_bytes_for_path(ordered_path: Option<&str>) -> Option<i64> {
    let path = ordered_path?;
    fs::metadata(path).ok().and_then(|metadata| {
        if metadata.is_file() {
            Some(metadata.len() as i64)
        } else {
            None
        }
    })
}

pub(crate) fn from_episode(episode: Episode) -> EpisodePayload {
    EpisodePayload {
        id: episode.id,
        title_id: episode.title_id,
        collection_id: episode.collection_id,
        episode_type: episode.episode_type.as_str().to_string(),
        episode_number: episode.episode_number,
        season_number: episode.season_number,
        episode_label: episode.episode_label,
        title: episode.title,
        overview: episode.overview,
        air_date: episode.air_date,
        duration_seconds: episode.duration_seconds,
        has_multi_audio: episode.has_multi_audio,
        has_subtitle: episode.has_subtitle,
        is_filler: episode.is_filler,
        is_recap: episode.is_recap,
        absolute_number: episode.absolute_number,
        tvdb_id: episode.tvdb_id,
        monitored: episode.monitored,
        created_at: episode.created_at.to_rfc3339(),
    }
}

pub(crate) fn from_calendar_episode(ep: CalendarEpisode) -> CalendarEpisodePayload {
    CalendarEpisodePayload {
        id: ep.id,
        title_id: ep.title_id,
        title_name: ep.title_name,
        title_facet: ep.title_facet,
        season_number: ep.season_number,
        episode_number: ep.episode_number,
        episode_title: ep.episode_title,
        air_date: ep.air_date,
        monitored: ep.monitored,
    }
}

pub(crate) fn from_title_media_file(
    file: scryer_application::TitleMediaFile,
) -> TitleMediaFilePayload {
    TitleMediaFilePayload {
        id: file.id,
        title_id: file.title_id,
        episode_id: file.episode_id,
        file_path: file.file_path,
        size_bytes: file.size_bytes.to_string(),
        quality_label: file.quality_label,
        scan_status: file.scan_status,
        created_at: file.created_at,
        video_codec: file.video_codec,
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
        video_codec_parsed: file.video_codec_parsed,
        audio_codec_parsed: file.audio_codec_parsed,
        acquisition_score: file.acquisition_score,
        scoring_log: file.scoring_log,
        indexer_source: file.indexer_source,
        grabbed_release_title: file.grabbed_release_title,
        grabbed_at: file.grabbed_at,
        edition: file.edition,
        original_file_path: file.original_file_path,
        release_hash: file.release_hash,
    }
}

pub(crate) fn from_user(user: User) -> UserPayload {
    UserPayload {
        id: user.id,
        username: user.username,
        entitlements: user
            .entitlements
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
    }
}

pub(crate) fn from_activity_event(event: ActivityEvent) -> ActivityEventPayload {
    ActivityEventPayload {
        id: event.id,
        kind: ActivityKindValue::from_application(event.kind),
        severity: ActivitySeverityValue::from_application(event.severity),
        channels: event
            .channels
            .into_iter()
            .map(ActivityChannelValue::from_application)
            .collect(),
        actor_user_id: event.actor_user_id,
        title_id: event.title_id,
        facet: event.facet.as_deref().and_then(MediaFacetValue::parse),
        message: event.message,
        occurred_at: event.occurred_at.to_rfc3339(),
    }
}

pub(crate) fn from_import_record(record: scryer_domain::ImportRecord) -> ImportRecordPayload {
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

    let source_title = serde_json::from_str::<serde_json::Value>(&record.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .get("source_title")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload.get("name").and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(ToString::to_string)
        });

    ImportRecordPayload {
        id: record.id,
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_title,
        import_type: ImportTypeValue::from_domain(record.import_type),
        status: ImportStatusValue::from_domain(record.status),
        error_message,
        decision,
        skip_reason,
        title_id,
        source_path,
        dest_path,
        started_at: record.started_at,
        finished_at: record.finished_at,
        created_at: record.created_at,
    }
}

pub(crate) fn from_policy(policy: PolicyOutput) -> PolicyOutputPayload {
    PolicyOutputPayload {
        decision: policy.decision,
        score: policy.score,
        reason_codes: policy.reason_codes,
        explanation: policy.explanation,
        scoring_log: policy
            .scoring_log
            .into_iter()
            .map(|e| ScoringEntryPayload {
                code: e.code,
                delta: e.delta,
                source: e.source,
                rule_set_name: None,
            })
            .collect(),
    }
}

pub(crate) fn from_wanted_item(item: scryer_application::WantedItem) -> WantedItemPayload {
    WantedItemPayload {
        id: item.id,
        title_id: item.title_id,
        title_name: item.title_name,
        episode_id: item.episode_id,
        collection_id: item.collection_id,
        media_type: WantedMediaTypeValue::parse(&item.media_type)
            .expect("wanted item media_type should map to GraphQL enum"),
        search_phase: WantedSearchPhaseValue::parse(&item.search_phase)
            .expect("wanted item search_phase should map to GraphQL enum"),
        next_search_at: item.next_search_at,
        last_search_at: item.last_search_at,
        search_count: item.search_count,
        baseline_date: item.baseline_date,
        status: WantedStatusValue::from_application(item.status),
        grabbed_release: item.grabbed_release,
        current_score: item.current_score,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

pub(crate) fn from_release_decision(
    decision: scryer_application::ReleaseDecision,
) -> ReleaseDecisionPayload {
    ReleaseDecisionPayload {
        id: decision.id,
        wanted_item_id: decision.wanted_item_id,
        title_id: decision.title_id,
        release_title: decision.release_title,
        release_url: decision.release_url,
        release_size_bytes: decision.release_size_bytes,
        decision_code: decision.decision_code,
        candidate_score: decision.candidate_score,
        current_score: decision.current_score,
        score_delta: decision.score_delta,
        explanation_json: decision.explanation_json,
        created_at: decision.created_at,
    }
}

pub(crate) fn from_disk_space(info: DiskSpaceInfo) -> DiskSpacePayload {
    DiskSpacePayload {
        path: info.path,
        label: info.label,
        total_bytes: info.total_bytes.to_string(),
        free_bytes: info.free_bytes.to_string(),
        used_bytes: info.used_bytes.to_string(),
    }
}

pub(crate) fn from_system_health(health: SystemHealth) -> SystemHealthPayload {
    SystemHealthPayload {
        service_ready: health.service_ready,
        db_path: health.db_path,
        total_titles: health.total_titles as i32,
        monitored_titles: health.monitored_titles as i32,
        total_users: health.total_users as i32,
        titles_movie: health.titles_movie as i32,
        titles_tv: health.titles_tv as i32,
        titles_anime: health.titles_anime as i32,
        titles_other: health.titles_other as i32,
        recent_events: health.recent_events as i32,
        recent_event_preview: health.recent_event_preview,
        db_migration_version: health.db_migration_version,
        db_pending_migrations: health.db_pending_migrations as i32,
        smg_cert_expires_at: health.smg_cert_expires_at,
        smg_cert_days_remaining: health.smg_cert_days_remaining.map(|d| d as i32),
        indexer_stats: health
            .indexer_stats
            .into_iter()
            .map(|s| IndexerQueryStatsPayload {
                indexer_id: s.indexer_id,
                indexer_name: s.indexer_name,
                queries_last_24h: s.queries_last_24h as i32,
                successful_last_24h: s.successful_last_24h as i32,
                failed_last_24h: s.failed_last_24h as i32,
                last_query_at: s.last_query_at,
                api_current: s.api_current.map(|v| v as i32),
                api_max: s.api_max.map(|v| v as i32),
                grab_current: s.grab_current.map(|v| v as i32),
                grab_max: s.grab_max.map(|v| v as i32),
            })
            .collect(),
    }
}

pub(crate) fn from_rule_set(rs: RuleSet) -> RuleSetPayload {
    RuleSetPayload {
        id: rs.id,
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
        created_at: rs.created_at.to_rfc3339(),
        updated_at: rs.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_registry_plugin(p: RegistryPlugin) -> RegistryPluginPayload {
    RegistryPluginPayload {
        id: p.id,
        name: p.name,
        description: p.description,
        version: p.version,
        plugin_type: p.plugin_type,
        provider_type: p.provider_type,
        author: p.author,
        official: p.official,
        builtin: p.builtin,
        source_url: p.source_url,
        is_installed: p.is_installed,
        is_enabled: p.is_enabled,
        installed_version: p.installed_version,
        update_available: p.update_available,
        default_base_url: p.default_base_url,
    }
}

pub(crate) fn from_notification_channel(
    ch: scryer_domain::NotificationChannelConfig,
) -> NotificationChannelPayload {
    NotificationChannelPayload {
        id: ch.id,
        name: ch.name,
        channel_type: ch.channel_type.as_str().to_string(),
        config_json: ch.config_json,
        is_enabled: ch.is_enabled,
        created_at: ch.created_at.to_rfc3339(),
        updated_at: ch.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_notification_subscription(
    sub: scryer_domain::NotificationSubscription,
) -> NotificationSubscriptionPayload {
    NotificationSubscriptionPayload {
        id: sub.id,
        channel_id: sub.channel_id,
        event_type: sub.event_type.as_str().to_string(),
        scope: sub.scope,
        scope_id: sub.scope_id,
        is_enabled: sub.is_enabled,
        created_at: sub.created_at.to_rfc3339(),
        updated_at: sub.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_plugin_installation(inst: PluginInstallation) -> PluginInstallationPayload {
    PluginInstallationPayload {
        id: inst.id,
        plugin_id: inst.plugin_id,
        name: inst.name,
        description: inst.description,
        version: inst.version,
        plugin_type: inst.plugin_type,
        provider_type: inst.provider_type,
        is_enabled: inst.is_enabled,
        is_builtin: inst.is_builtin,
        source_url: inst.source_url,
        installed_at: inst.installed_at.to_rfc3339(),
        updated_at: inst.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_backup_info(info: BackupInfo) -> BackupInfoPayload {
    BackupInfoPayload {
        filename: info.filename,
        size_bytes: info.size_bytes.to_string(),
        created_at: info.created_at,
    }
}

pub(crate) fn from_health_check_result(result: HealthCheckResult) -> HealthCheckPayload {
    HealthCheckPayload {
        source: result.source,
        status: result.status.as_str().to_string(),
        message: result.message,
    }
}

pub(crate) fn from_rss_sync_report(report: RssSyncReport) -> RssSyncReportPayload {
    RssSyncReportPayload {
        releases_fetched: report.releases_fetched as i32,
        releases_matched: report.releases_matched as i32,
        releases_grabbed: report.releases_grabbed as i32,
        releases_held: report.releases_held as i32,
    }
}

pub(crate) fn from_pending_release(pr: PendingRelease) -> PendingReleasePayload {
    PendingReleasePayload {
        id: pr.id,
        wanted_item_id: pr.wanted_item_id,
        title_id: pr.title_id,
        release_title: pr.release_title,
        release_url: pr.release_url,
        release_size_bytes: pr.release_size_bytes.map(|v| v.to_string()),
        release_score: pr.release_score,
        scoring_log_json: pr.scoring_log_json,
        indexer_source: pr.indexer_source,
        added_at: pr.added_at,
        delay_until: pr.delay_until,
        status: PendingReleaseStatusValue::from_application(pr.status),
    }
}

pub(crate) fn from_pp_script(
    s: scryer_domain::PostProcessingScript,
) -> PostProcessingScriptPayload {
    PostProcessingScriptPayload {
        id: s.id,
        name: s.name,
        description: s.description,
        script_type: s.script_type.as_str().to_string(),
        script_content: s.script_content,
        applied_facets: s.applied_facets,
        execution_mode: s.execution_mode.as_str().to_string(),
        timeout_secs: s.timeout_secs as i32,
        priority: s.priority,
        enabled: s.enabled,
        debug: s.debug,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_pp_script_run(
    r: scryer_domain::PostProcessingScriptRun,
) -> PostProcessingScriptRunPayload {
    PostProcessingScriptRunPayload {
        id: r.id,
        script_id: r.script_id,
        script_name: r.script_name,
        title_id: r.title_id,
        title_name: r.title_name,
        facet: r.facet.as_deref().and_then(MediaFacetValue::parse),
        file_path: r.file_path,
        status: r.status.as_str().to_string(),
        exit_code: r.exit_code,
        stdout_tail: r.stdout_tail,
        stderr_tail: r.stderr_tail,
        duration_ms: r.duration_ms.map(|v| v as i32),
        env_payload_json: r.env_payload_json,
        started_at: r.started_at,
        completed_at: r.completed_at,
    }
}

pub(crate) fn from_housekeeping_report(report: HousekeepingReport) -> HousekeepingReportPayload {
    HousekeepingReportPayload {
        orphaned_media_files: report.orphaned_media_files as i32,
        stale_release_decisions: report.stale_release_decisions as i32,
        stale_release_attempts: report.stale_release_attempts as i32,
        expired_event_outboxes: report.expired_event_outboxes as i32,
        stale_history_events: report.stale_history_events as i32,
        staged_nzb_artifacts_pruned: report.staged_nzb_artifacts_pruned as i32,
        recycled_purged: report.recycled_purged as i32,
        ran_at: report.ran_at,
    }
}

pub(crate) fn from_title_history_record(record: TitleHistoryRecord) -> TitleHistoryEventPayload {
    TitleHistoryEventPayload {
        id: record.id,
        title_id: record.title_id,
        episode_id: record.episode_id,
        collection_id: record.collection_id,
        event_type: record.event_type.as_str().to_string(),
        source_title: record.source_title,
        quality: record.quality,
        download_id: record.download_id,
        data_json: record.data_json,
        occurred_at: record.occurred_at,
        created_at: record.created_at,
    }
}

pub(crate) fn from_title_history_page(page: TitleHistoryPage) -> TitleHistoryPagePayload {
    TitleHistoryPagePayload {
        records: page
            .records
            .into_iter()
            .map(from_title_history_record)
            .collect(),
        total_count: page.total_count,
    }
}
