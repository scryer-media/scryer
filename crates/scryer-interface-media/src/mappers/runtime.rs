use super::configuration::{stored_secret_keys_from_config_json, support_tier_label};
use super::*;

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
                grabs_last_24h: s.grabs_last_24h as i32,
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

pub fn from_application_upgrade_status(
    assessment: scryer_application::application_upgrade::InstallationAssessment,
    current_version: String,
    update_notice: Option<scryer_application::SmgScryerUpdateNotice>,
    active_run: Option<scryer_application::JobRun>,
    latest_run: Option<scryer_application::JobRun>,
) -> ApplicationUpgradeStatusPayload {
    let (update_version, update_tag, update_available) = match update_notice {
        Some(notice) => (
            Some(notice.latest_version),
            Some(notice.latest_tag),
            notice.available,
        ),
        None => (None, None, false),
    };

    ApplicationUpgradeStatusPayload {
        current_version,
        update_version,
        update_tag,
        update_available,
        installation_kind: match assessment.kind {
            scryer_application::application_upgrade::InstallationKind::Portable => {
                ApplicationInstallationKindValue::Portable
            }
            scryer_application::application_upgrade::InstallationKind::DirectMsi => {
                ApplicationInstallationKindValue::DirectMsi
            }
            scryer_application::application_upgrade::InstallationKind::Docker => {
                ApplicationInstallationKindValue::Docker
            }
            scryer_application::application_upgrade::InstallationKind::Homebrew => {
                ApplicationInstallationKindValue::Homebrew
            }
            scryer_application::application_upgrade::InstallationKind::Winget => {
                ApplicationInstallationKindValue::Winget
            }
            scryer_application::application_upgrade::InstallationKind::WindowsSupervised => {
                ApplicationInstallationKindValue::WindowsSupervised
            }
            scryer_application::application_upgrade::InstallationKind::Disabled => {
                ApplicationInstallationKindValue::Disabled
            }
            scryer_application::application_upgrade::InstallationKind::Unsupported => {
                ApplicationInstallationKindValue::Unsupported
            }
        },
        management_owner: match assessment.owner {
            scryer_application::application_upgrade::ManagementOwner::InApp => {
                ApplicationUpgradeOwnerValue::InApp
            }
            scryer_application::application_upgrade::ManagementOwner::Operator => {
                ApplicationUpgradeOwnerValue::Operator
            }
        },
        eligible: assessment.eligible,
        eligibility_reason: assessment.reason.as_str().to_string(),
        active_run: active_run.map(from_job_run),
        latest_run: latest_run.map(from_job_run),
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
        indexer_id: pr.indexer_id.map(ID),
        published_at: parse_optional_datetime(pr.published_at, "pending release published_at"),
        seeders: pr.seeders,
        added_at: parse_required_datetime(&pr.added_at, "pending release added_at"),
        delay_until: parse_required_datetime(&pr.delay_until, "pending release delay_until"),
        last_decision_code: pr.last_decision_code,
        role: PendingReleaseRoleValue::from_application(pr.role),
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
        poster_url: record.poster_url,
        library_id: record.library_id.map(Into::into),
        facet: record.facet.map(MediaFacetValue::from_domain),
        size_bytes: record.size_bytes.map(Long::from),
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
pub(super) fn monitor_type_value_from_normalized(value: &str) -> Option<MonitorTypeValue> {
    match value {
        "monitored" => Some(MonitorTypeValue::Monitored),
        "unmonitored" => Some(MonitorTypeValue::Unmonitored),
        "futureepisodes" => Some(MonitorTypeValue::FutureEpisodes),
        "missingandfutureepisodes" => Some(MonitorTypeValue::MissingAndFutureEpisodes),
        "allepisodes" => Some(MonitorTypeValue::AllEpisodes),
        "advanced" => Some(MonitorTypeValue::Advanced),
        "none" => Some(MonitorTypeValue::NoneSelected),
        _ => None,
    }
}

/// Boundary conversion for JSON persisted as text in the application layer:
/// the wire carries real JSON, never a string-encoded document.
pub fn json_string_to_value(raw: String) -> async_graphql::Json<serde_json::Value> {
    async_graphql::Json(serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null))
}
