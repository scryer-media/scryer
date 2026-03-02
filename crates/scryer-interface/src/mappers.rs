use scryer_application::{
    ActivityChannel, ActivityEvent, IndexerSearchResult, LibraryScanSummary, ParsedEpisodeMetadata,
    ParsedReleaseMetadata, QualityProfileDecision, RenameApplyItemResult, RenameApplyResult,
    RenamePlan, RenamePlanItem, ScoringEntry, ScoringSource, SystemHealth, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    CalendarEpisode, DownloadQueueItem, PolicyOutput, RuleSet,
    Collection, DownloadClientConfig, Episode, HistoryEvent, IndexerConfig, Title, User,
};
use scryer_infrastructure::{SettingsValueRecord, WorkflowOperationRecord};
use std::fs;

use crate::types::*;

pub(crate) fn map_admin_setting(record: SettingsValueRecord) -> AdminSettingsItemPayload {
    let has_override = record.has_override();
    let is_sensitive = record.is_sensitive;

    AdminSettingsItemPayload {
        category: record.category,
        scope: record.scope,
        key_name: record.key_name,
        data_type: record.data_type,
        default_value_json: record.default_value_json,
        effective_value_json: if is_sensitive {
            None
        } else {
            Some(record.effective_value_json)
        },
        value_json: if is_sensitive {
            None
        } else {
            record.value_json
        },
        source: record.source,
        has_override,
        is_sensitive,
        validation_json: record.validation_json,
        scope_id: record.scope_id,
        updated_by_user_id: record.updated_by_user_id,
        created_at: record.created_at,
        updated_at: record.updated_at,
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
    IndexerSearchResultPayload {
        source: result.source,
        title: result.title,
        link: result.link,
        download_url: result.download_url,
        size_bytes: result.size_bytes,
        published_at: result.published_at,
        thumbs_up: result.thumbs_up,
        thumbs_down: result.thumbs_down,
        parsed_release: result.parsed_release_metadata.map(from_parsed_release),
        quality_profile_decision: result
            .quality_profile_decision
            .map(from_quality_profile_decision),
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
            .map(|e: ScoringEntry| ScoringEntryPayload {
                code: e.code,
                delta: e.delta,
                source: match e.source {
                    ScoringSource::Builtin => "builtin".to_string(),
                    ScoringSource::UserRule(id) => format!("user:{id}"),
                },
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
        parser_version: result.parser_version.to_string(),
        parse_confidence: result.parse_confidence,
        missing_fields: result.missing_fields,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
    }
}

pub(crate) fn from_parsed_episode(
    episode: ParsedEpisodeMetadata,
) -> ParsedEpisodePayload {
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
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

pub(crate) fn from_download_client_config(
    config: DownloadClientConfig,
) -> DownloadClientConfigPayload {
    DownloadClientConfigPayload {
        id: config.id,
        name: config.name,
        client_type: config.client_type,
        base_url: config.base_url,
        config_json: config.config_json,
        is_enabled: config.is_enabled,
        status: config.status,
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
        facet: item.facet,
        is_scryer_origin: item.is_scryer_origin,
        client_id: item.client_id,
        client_name: item.client_name,
        client_type: item.client_type,
        state: format!("{:?}", item.state).to_lowercase(),
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
        import_status: item.import_status,
        import_error_message: item.import_error_message,
        imported_at: item.imported_at,
    }
}

pub(crate) fn from_title(title: Title) -> TitlePayload {
    TitlePayload {
        id: title.id,
        name: title.name,
        facet: format!("{:?}", title.facet).to_lowercase(),
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
        quality_tier: None,
        size_bytes: None,
    }
}

pub(crate) fn from_library_scan_summary(
    summary: LibraryScanSummary,
) -> LibraryScanSummaryPayload {
    LibraryScanSummaryPayload {
        scanned: summary.scanned as i32,
        matched: summary.matched as i32,
        imported: summary.imported as i32,
        skipped: summary.skipped as i32,
        unmatched: summary.unmatched as i32,
    }
}

pub(crate) fn from_media_rename_plan(plan: RenamePlan) -> MediaRenamePlanPayload {
    MediaRenamePlanPayload {
        facet: format!("{:?}", plan.facet).to_lowercase(),
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
        items: plan.items.into_iter().map(from_media_rename_plan_item).collect(),
    }
}

fn from_media_rename_plan_item(item: RenamePlanItem) -> MediaRenamePlanItemPayload {
    MediaRenamePlanItemPayload {
        collection_id: item.collection_id,
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
    CollectionPayload {
        id: collection.id,
        title_id: collection.title_id,
        collection_type: collection.collection_type,
        collection_index: collection.collection_index,
        label: collection.label,
        ordered_path: collection.ordered_path,
        narrative_order: collection.narrative_order,
        file_size_bytes,
        first_episode_number: collection.first_episode_number,
        last_episode_number: collection.last_episode_number,
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
        episode_type: episode.episode_type,
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

pub(crate) fn from_title_media_file(file: scryer_application::TitleMediaFile) -> TitleMediaFilePayload {
    TitleMediaFilePayload {
        id: file.id,
        title_id: file.title_id,
        episode_id: file.episode_id,
        file_path: file.file_path,
        size_bytes: file.size_bytes.to_string(),
        quality_label: file.quality_label,
        scan_status: file.scan_status,
        created_at: file.created_at,
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

pub(crate) fn from_event(event: HistoryEvent) -> EventPayload {
    EventPayload {
        id: event.id,
        event_type: format!("{:?}", event.event_type).to_lowercase(),
        actor_user_id: event.actor_user_id,
        title_id: event.title_id,
        message: event.message,
        occurred_at: event.occurred_at.to_rfc3339(),
    }
}

pub(crate) fn from_activity_event(event: ActivityEvent) -> ActivityEventPayload {
    ActivityEventPayload {
        id: event.id,
        kind: event.kind.as_str().to_string(),
        severity: event.severity.as_str().to_string(),
        channels: event
            .channels
            .into_iter()
            .map(|channel: ActivityChannel| channel.as_str().to_string())
            .collect(),
        actor_user_id: event.actor_user_id,
        title_id: event.title_id,
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
                    Some(result.decision.as_str().to_string()),
                    result.skip_reason.map(|r| r.as_str().to_string()),
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
        import_type: record.import_type,
        status: record.status,
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
        media_type: item.media_type,
        search_phase: item.search_phase,
        next_search_at: item.next_search_at,
        last_search_at: item.last_search_at,
        search_count: item.search_count,
        baseline_date: item.baseline_date,
        status: item.status,
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
        rego_source: rs.rego_source,
        enabled: rs.enabled,
        priority: rs.priority,
        applied_facets: rs
            .applied_facets
            .iter()
            .map(|f| format!("{:?}", f).to_lowercase())
            .collect(),
        created_at: rs.created_at.to_rfc3339(),
        updated_at: rs.updated_at.to_rfc3339(),
    }
}
