use crate::integration::workflow::{extract_url_origin, source_provider_label};
use crate::library_scan_progress::{
    reduce_library_scan_projection_event, replay_library_scan_projection,
};
use crate::{
    ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity, DownloadQueueItem, JobKey,
    JobRun, JobRunStatus, JobTriggerSource, LibraryScanSession, LibraryScanStatus,
};
use chrono::{DateTime, Utc};
use regex::Regex;
use scryer_domain::{
    ConfigurationChangeAction, DomainEvent, DomainEventPayload, DownloadQueueItemRemovedEventData,
    DownloadQueueItemUpsertedEventData, DownloadQueueState, EventType, HistoryEvent,
    ImportRejectedEventData, ImportSkipReason, ImportStatus, JobNextRunUpdatedEventData,
    MediaFacet, MediaFileDeletedReason, MetadataHydrationState, PostProcessingResult,
    TitleHistoryEventType, TitleHistoryRecord,
};
use serde_json::Value;
use std::{cmp::Ordering, collections::HashMap, sync::OnceLock};

fn default_activity_channels() -> Vec<ActivityChannel> {
    vec![ActivityChannel::WebUi]
}

pub(crate) fn activity_event_from_domain_event(event: &DomainEvent) -> Option<ActivityEvent> {
    let (kind, severity, message) = match &event.payload {
        DomainEventPayload::TitleAdded(data) => (
            ActivityKind::TitleAdded,
            ActivitySeverity::Success,
            format!("Added '{}' to Scryer.", data.title.title_name),
        ),
        DomainEventPayload::TitleUpdated(data) => (
            ActivityKind::TitleUpdated,
            ActivitySeverity::Info,
            format!("Updated '{}'.", data.title.title_name),
        ),
        DomainEventPayload::TitleRematched(data) => (
            ActivityKind::TitleUpdated,
            ActivitySeverity::Info,
            format!(
                "Rematched '{}' to TVDB {}.",
                data.title.title_name, data.new_tvdb_id
            ),
        ),
        DomainEventPayload::TitleDeleted(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!("Deleted '{}' from Scryer.", data.title.title_name),
        ),
        DomainEventPayload::ConfigurationChanged(data) => (
            ActivityKind::SettingSaved,
            ActivitySeverity::Success,
            configuration_changed_message(
                data.resource_type.as_str(),
                data.resource_id.as_deref(),
                data.action,
            ),
        ),
        DomainEventPayload::DiscoverySearchCompleted(data) => (
            ActivityKind::MovieFetched,
            ActivitySeverity::Info,
            discovery_search_completed_message(
                data.search_type.as_str(),
                data.query.as_deref(),
                data.result_count,
            ),
        ),
        DomainEventPayload::MetadataHydrationUpdated(data) => metadata_hydration_activity(
            data.state,
            data.title.title_name.as_str(),
            data.reason.as_deref(),
        ),
        DomainEventPayload::ReleaseGrabbed(data) => (
            ActivityKind::AcquisitionCandidateAccepted,
            ActivitySeverity::Success,
            data.source_title
                .as_ref()
                .map(|source_title| {
                    format!(
                        "Grabbed '{}' for '{}'.",
                        source_title, data.title.title_name
                    )
                })
                .unwrap_or_else(|| format!("Grabbed a release for '{}'.", data.title.title_name)),
        ),
        DomainEventPayload::DownloadFailed(data) => (
            ActivityKind::AcquisitionDownloadFailed,
            ActivitySeverity::Warning,
            data.source_title
                .as_ref()
                .map(|source_title| format!("Download failed for '{}'.", source_title))
                .or_else(|| {
                    data.title
                        .as_ref()
                        .map(|title| format!("Download failed for '{}'.", title.title_name))
                })
                .unwrap_or_else(|| "Download failed.".to_string()),
        ),
        DomainEventPayload::ReleaseBlocklisted(data) => (
            ActivityKind::AcquisitionCandidateRejected,
            ActivitySeverity::Warning,
            data.source_title
                .as_ref()
                .map(|source_title| format!("Blocklisted '{}'.", source_title))
                .or_else(|| {
                    data.title
                        .as_ref()
                        .map(|title| format!("Blocklisted a release for '{}'.", title.title_name))
                })
                .unwrap_or_else(|| "Blocklisted a release.".to_string()),
        ),
        DomainEventPayload::DownloadIgnored(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            data.title
                .as_ref()
                .map(|title| format!("Ignored a download for '{}'.", title.title_name))
                .unwrap_or_else(|| "Ignored a download.".to_string()),
        ),
        DomainEventPayload::ImportCompleted(data) => (
            if data.title.facet == MediaFacet::Movie {
                ActivityKind::MovieDownloaded
            } else {
                ActivityKind::SeriesEpisodeImported
            },
            ActivitySeverity::Success,
            format!(
                "Imported {} file{} for '{}'.",
                data.imported_count,
                if data.imported_count == 1 { "" } else { "s" },
                data.title.title_name
            ),
        ),
        DomainEventPayload::ImportRejected(data) => (
            ActivityKind::ImportRejected,
            ActivitySeverity::Warning,
            import_rejected_message(data),
        ),
        DomainEventPayload::MediaFileImported(data) => (
            if data.title.facet == MediaFacet::Movie {
                ActivityKind::MovieDownloaded
            } else {
                ActivityKind::SeriesEpisodeImported
            },
            ActivitySeverity::Success,
            format!("Imported media file for '{}'.", data.title.title_name),
        ),
        DomainEventPayload::MediaFileAnalyzed(data) => (
            ActivityKind::FileAnalyzed,
            ActivitySeverity::Info,
            format!("Analyzed media file for '{}'.", data.title.title_name),
        ),
        DomainEventPayload::MediaFileRenamed(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!(
                "Renamed {} file(s) for '{}'.",
                data.renamed_count, data.title.title_name
            ),
        ),
        DomainEventPayload::MediaFileDeleted(data) => (
            ActivityKind::SystemNotice,
            if matches!(data.reason, MediaFileDeletedReason::UpgradeCleanup) {
                ActivitySeverity::Info
            } else {
                ActivitySeverity::Warning
            },
            data.media_updates
                .first()
                .map(|update| match data.reason {
                    MediaFileDeletedReason::UpgradeCleanup => {
                        format!("Removed old media file during upgrade: {}", update.path)
                    }
                    MediaFileDeletedReason::RecycleBinPurged => {
                        format!("Permanently deleted recycled media file: {}", update.path)
                    }
                    MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
                        format!("Deleted media file from disk: {}", update.path)
                    }
                })
                .unwrap_or_else(|| format!("Deleted media file for '{}'.", data.title.title_name)),
        ),
        DomainEventPayload::MediaFileUpgraded(data) => (
            ActivityKind::FileUpgraded,
            ActivitySeverity::Success,
            match (data.old_score, data.new_score) {
                (Some(old_score), Some(new_score)) => format!(
                    "Upgraded file for '{}': score {} → {} (delta +{})",
                    data.title.title_name,
                    old_score,
                    new_score,
                    new_score - old_score
                ),
                _ => format!("Upgraded file for '{}'.", data.title.title_name),
            },
        ),
        DomainEventPayload::AcquisitionSearchCompleted(data) => (
            ActivityKind::AcquisitionSearchCompleted,
            ActivitySeverity::Info,
            format!(
                "{} results for '{}'",
                data.result_count, data.title.title_name
            ),
        ),
        DomainEventPayload::AcquisitionCandidateRejected(data) => (
            ActivityKind::AcquisitionCandidateRejected,
            ActivitySeverity::Info,
            format!(
                "{}: '{}' ({})",
                data.reason_code, data.source_title, data.title.title_name
            ),
        ),
        DomainEventPayload::ImportRequested(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            import_requested_message(data.client_type.as_str(), data.source_ref.as_str()),
        ),
        DomainEventPayload::MediaRequestSubmitted(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!("Requested '{}' for catalog review.", data.title_name),
        ),
        DomainEventPayload::MediaRequestUpdated(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!("Updated request for '{}'.", data.title_name),
        ),
        DomainEventPayload::MediaRequestCanceled(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!("Canceled request for '{}'.", data.title_name),
        ),
        DomainEventPayload::ImportRecoveryCompleted(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Warning,
            format!(
                "{} stale import(s) recovered as failed — check import history",
                data.recovered_count
            ),
        ),
        DomainEventPayload::DownloadQueueItemCommandIssued(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!(
                "download {}: {}",
                download_queue_command_label(data.action),
                data.item_id
            ),
        ),
        DomainEventPayload::PostProcessingCompleted(data) => (
            ActivityKind::PostProcessingCompleted,
            post_processing_severity(data.result),
            post_processing_message(
                data.script_name.as_str(),
                data.title.title_name.as_str(),
                data.result,
                data.exit_code,
            ),
        ),
        DomainEventPayload::SubtitleDownloaded(data) => (
            ActivityKind::SubtitleDownloaded,
            ActivitySeverity::Success,
            format!("Downloaded subtitle for '{}'.", data.title.title_name),
        ),
        DomainEventPayload::SubtitleSearchFailed(data) => (
            ActivityKind::SubtitleSearchFailed,
            ActivitySeverity::Warning,
            format!("Subtitle search failed for '{}'.", data.title.title_name),
        ),
        _ => return None,
    };

    let episode_ids = match &event.payload {
        DomainEventPayload::ImportCompleted(data) => data.episode_ids.clone(),
        _ => Vec::new(),
    };

    Some(ActivityEvent {
        id: event.event_id.clone(),
        kind,
        severity,
        channels: default_activity_channels(),
        actor_kind: event.actor_kind,
        actor_user_id: event.actor_user_id.clone(),
        actor_display_name: event.actor_display_name.clone(),
        title_id: event.title_id.clone(),
        facet: event.facet.as_ref().map(|facet| facet.as_str().to_string()),
        episode_ids,
        message,
        occurred_at: event.occurred_at,
    })
}

pub(crate) fn title_history_records_from_domain_event(
    event: &DomainEvent,
) -> Vec<TitleHistoryRecord> {
    let Some(base_record) = title_history_record_from_domain_event(event) else {
        return Vec::new();
    };

    if base_record.episode_ids.is_empty() {
        return vec![base_record];
    }

    base_record
        .episode_ids
        .iter()
        .cloned()
        .map(|episode_id| {
            let mut record = base_record.clone();
            record.episode_id = Some(episode_id);
            record
        })
        .collect()
}

const REDACTED_HISTORY_SECRET: &str = "[redacted]";

fn history_api_key_query_param_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)(?P<prefix>\b(?:api_?key)=)(?P<value>[^&#\s"'<>),\]}]+)"#)
            .expect("history api key regex should compile")
    })
}

fn history_url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\bhttps?://[^\s\"'<>]+"#).expect("history URL regex should compile")
    })
}

fn looks_like_history_secret_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();

    normalized == "apikey" || normalized.ends_with("apikey")
}

fn redact_history_api_keys(raw: &str) -> String {
    history_api_key_query_param_regex()
        .replace_all(raw, format!("${{prefix}}{REDACTED_HISTORY_SECRET}"))
        .into_owned()
}

fn redact_history_urls(raw: &str) -> String {
    history_url_regex()
        .replace_all(raw, |captures: &regex::Captures<'_>| {
            let matched = captures.get(0).expect("URL match").as_str();
            let trimmed = matched.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
            let suffix = &matched[trimmed.len()..];
            format!(
                "{}{}",
                extract_url_origin(trimmed).unwrap_or_else(|| "Origin unavailable".to_string()),
                suffix
            )
        })
        .into_owned()
}

fn sanitize_history_string(value: Option<String>) -> Option<String> {
    value.map(|value| redact_history_api_keys(&redact_history_urls(&value)))
}

fn history_source_provider(value: Option<String>) -> Option<String> {
    source_provider_label(None, value.as_deref())
}

fn sanitize_history_json_value(value: Value) -> Value {
    match value {
        Value::String(value) => {
            Value::String(redact_history_api_keys(&redact_history_urls(&value)))
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(sanitize_history_json_value)
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = match key.as_str() {
                        "source_hint" => match value {
                            Value::String(value) => source_provider_label(None, Some(&value))
                                .map(Value::String)
                                .unwrap_or(Value::Null),
                            _ => Value::Null,
                        },
                        "request_signature" | "source_password" => {
                            Value::String(REDACTED_HISTORY_SECRET.to_string())
                        }
                        _ if looks_like_history_secret_key(&key) => {
                            Value::String(REDACTED_HISTORY_SECRET.to_string())
                        }
                        _ => sanitize_history_json_value(value),
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

fn serialize_title_history_data(payload: &DomainEventPayload) -> Option<String> {
    let value = match payload {
        DomainEventPayload::TitleRematched(data) => serde_json::to_value(data).ok()?,
        _ => serde_json::to_value(payload).ok()?,
    };

    serde_json::to_string(&sanitize_history_json_value(value)).ok()
}

pub(crate) fn title_history_record_from_domain_event(
    event: &DomainEvent,
) -> Option<TitleHistoryRecord> {
    let title_id = event.title_id.clone()?;

    let (
        title_name,
        facet,
        event_type,
        source_title,
        display_title,
        source_system,
        source_ref,
        source_hint,
        quality,
        download_id,
        client_id,
        client_name,
        import_id,
        skip_reason,
        retry_requires_password,
        failure_reason,
        blocklist_reason,
        source_path,
        dest_path,
    ) = match &event.payload {
        DomainEventPayload::ReleaseGrabbed(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            TitleHistoryEventType::Grabbed,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_hint.clone()),
            None,
            None,
            data.source_provider
                .clone()
                .or_else(|| data.source_hint.clone()),
            None,
            data.download_id.clone(),
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::DownloadFailed(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            TitleHistoryEventType::DownloadFailed,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_hint.clone()),
            None,
            None,
            data.source_hint.clone(),
            data.quality.clone(),
            data.download_id.clone(),
            data.client_id.clone(),
            data.client_name.clone(),
            None,
            None,
            false,
            data.reason.clone(),
            None,
            None,
            None,
        ),
        DomainEventPayload::ReleaseBlocklisted(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            TitleHistoryEventType::Blocklisted,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_hint.clone()),
            None,
            None,
            data.source_hint.clone(),
            data.quality.clone(),
            data.download_id.clone(),
            data.client_id.clone(),
            data.client_name.clone(),
            None,
            None,
            false,
            None,
            data.reason.clone(),
            None,
            None,
        ),
        DomainEventPayload::ImportCompleted(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            TitleHistoryEventType::Imported,
            data.source_title
                .clone()
                .or_else(|| data.source_path.clone())
                .or_else(|| {
                    (data.media_updates.len() == 1)
                        .then(|| data.media_updates.first().map(|update| update.path.clone()))
                        .flatten()
                }),
            data.source_title
                .clone()
                .or_else(|| data.source_path.clone())
                .or_else(|| data.dest_path.clone()),
            data.source_system.clone(),
            data.source_ref.clone(),
            None,
            data.quality.clone(),
            None,
            None,
            None,
            data.import_id.clone(),
            None,
            false,
            None,
            None,
            data.source_path.clone(),
            data.dest_path.clone(),
        ),
        DomainEventPayload::ImportRejected(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            match data.status {
                ImportStatus::Failed => TitleHistoryEventType::ImportFailed,
                ImportStatus::Skipped => TitleHistoryEventType::ImportSkipped,
                _ => return None,
            },
            data.source_title
                .clone()
                .or_else(|| data.source_path.clone()),
            data.source_title
                .clone()
                .or_else(|| data.source_path.clone())
                .or_else(|| data.dest_path.clone()),
            data.source_system.clone(),
            data.source_ref.clone(),
            None,
            data.quality.clone(),
            None,
            None,
            None,
            data.import_id.clone(),
            data.skip_reason
                .as_ref()
                .map(|reason| reason.as_str().to_string()),
            data.skip_reason == Some(ImportSkipReason::PasswordRequired),
            data.reason.clone(),
            None,
            data.source_path.clone(),
            data.dest_path.clone(),
        ),
        DomainEventPayload::DownloadIgnored(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            TitleHistoryEventType::DownloadIgnored,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_provider.clone()),
            None,
            None,
            data.source_provider.clone(),
            None,
            Some(data.download_client_item_id.clone()),
            data.client_id.clone(),
            data.client_type.clone(),
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        // The two seeding-retention events reuse the download-lifecycle shape
        // (`DownloadIgnored`): client identity in `download_id`/`client_*`, the
        // release title as the source title. The gate's reason and the action
        // it took ride in `data_json` with the rest of the payload.
        DomainEventPayload::SeedingStarted(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            TitleHistoryEventType::SeedingStarted,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_provider.clone()),
            None,
            None,
            data.source_provider.clone(),
            None,
            Some(data.download_client_item_id.clone()),
            data.client_id.clone(),
            data.client_type.clone(),
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::SeedingCompleted(data) => (
            data.title.as_ref().map(|title| title.title_name.clone()),
            data.title.as_ref().map(|title| title.facet.clone()),
            TitleHistoryEventType::SeedingCompleted,
            data.source_title.clone(),
            data.source_title
                .clone()
                .or_else(|| data.source_provider.clone()),
            None,
            None,
            data.source_provider.clone(),
            None,
            Some(data.download_client_item_id.clone()),
            data.client_id.clone(),
            data.client_type.clone(),
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::MediaFileAnalyzed(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            TitleHistoryEventType::Scanned,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            data.media_updates.first().map(|update| update.path.clone()),
            None,
        ),
        DomainEventPayload::MediaFileDeleted(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            match data.reason {
                MediaFileDeletedReason::UpgradeCleanup => TitleHistoryEventType::FileRecycled,
                _ => TitleHistoryEventType::FileDeleted,
            },
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::MediaFileRenamed(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            TitleHistoryEventType::FileRenamed,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::MediaFileUpgraded(data) => (
            Some(data.title.title_name.clone()),
            Some(data.title.facet.clone()),
            TitleHistoryEventType::FileUpgraded,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::TitleRematched(_) => (
            None,
            event.facet.clone(),
            TitleHistoryEventType::Rematched,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        ),
        DomainEventPayload::TitleUpdated(_) => return None,
        _ => return None,
    };

    let source_title = sanitize_history_string(source_title);
    let display_title = sanitize_history_string(display_title);
    let source_system = sanitize_history_string(source_system);
    let source_ref = sanitize_history_string(source_ref);
    let source_hint = history_source_provider(source_hint);
    let failure_reason = sanitize_history_string(failure_reason);
    let blocklist_reason = sanitize_history_string(blocklist_reason);
    let source_path = sanitize_history_string(source_path);
    let dest_path = sanitize_history_string(dest_path);
    let data_json = serialize_title_history_data(&event.payload);
    let episode_ids = event_episode_ids(event);
    let collection_id = event_collection_id(event);

    Some(TitleHistoryRecord {
        id: event.event_id.clone(),
        title_id,
        title_name,
        // Resolved from the titles lookup during projection hydration; the
        // event payload itself does not carry the owning library, and the
        // poster comes from the live title row rather than the event snapshot.
        poster_url: None,
        library_id: None,
        facet,
        episode_id: None,
        episode_ids,
        collection_id,
        event_type,
        actor_kind: Some(event.actor_kind),
        actor_user_id: event.actor_user_id.clone(),
        actor_display_name: Some(event.actor_display_name.clone()),
        source_title,
        display_title,
        source_system,
        source_ref,
        source_hint,
        quality,
        download_id,
        client_id,
        client_name,
        import_id,
        skip_reason,
        retry_requires_password,
        failure_reason,
        blocklist_reason,
        source_path,
        dest_path,
        size_bytes: title_history_size_bytes(&event.payload),
        data_json,
        occurred_at: event.occurred_at.to_rfc3339(),
        created_at: event.occurred_at.to_rfc3339(),
    })
}

/// Bytes to report for a history row.
///
/// Only import and upgrade events carry a size: an import reports the total
/// bytes it brought in, an upgrade reports the new file's size. Events written
/// before the payloads carried a size read back as `None`.
fn title_history_size_bytes(payload: &DomainEventPayload) -> Option<i64> {
    match payload {
        DomainEventPayload::ImportCompleted(data) => data.size_bytes,
        DomainEventPayload::MediaFileUpgraded(data) => data.size_bytes,
        _ => None,
    }
}

pub(crate) fn history_event_from_domain_event(event: &DomainEvent) -> Option<HistoryEvent> {
    let activity = activity_event_from_domain_event(event)?;
    let event_type = match &event.payload {
        DomainEventPayload::TitleAdded(_) => EventType::TitleAdded,
        DomainEventPayload::TitleUpdated(_) => EventType::TitleUpdated,
        DomainEventPayload::TitleRematched(_) => EventType::TitleUpdated,
        DomainEventPayload::MediaFileUpgraded(_) => EventType::FileUpgraded,
        DomainEventPayload::DownloadFailed(_)
        | DomainEventPayload::ReleaseBlocklisted(_)
        | DomainEventPayload::ImportRejected(_)
        | DomainEventPayload::SubtitleSearchFailed(_) => EventType::Error,
        _ => EventType::ActionCompleted,
    };

    Some(HistoryEvent {
        id: event.event_id.clone(),
        event_type,
        actor_user_id: event.actor_user_id.clone(),
        title_id: event.title_id.clone(),
        message: activity.message,
        occurred_at: event.occurred_at,
    })
}

#[cfg(test)]
fn title_history_records_for_episode_from_domain_events(
    events: &[DomainEvent],
    episode_id: &str,
    limit: usize,
) -> Vec<TitleHistoryRecord> {
    let mut records = events
        .iter()
        .flat_map(title_history_records_from_domain_event)
        .filter(|record| record.episode_id.as_deref() == Some(episode_id))
        .collect::<Vec<_>>();
    records.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    records.truncate(limit);
    records
}

pub fn replay_library_scan_state(events: &[DomainEvent]) -> HashMap<String, LibraryScanSession> {
    replay_library_scan_projection(events)
}

pub fn replay_active_job_runs(events: &[DomainEvent]) -> HashMap<String, JobRun> {
    let mut runs = HashMap::new();
    let mut scans = HashMap::new();
    for event in events {
        apply_library_scan_event(&mut scans, event);
        apply_job_run_event(&mut runs, &scans, event);
    }
    runs
}

pub fn replay_download_queue_state(events: &[DomainEvent]) -> HashMap<String, DownloadQueueItem> {
    let mut items = HashMap::new();
    for event in events {
        apply_download_queue_event(&mut items, event);
    }
    items
}

fn import_rejected_message(data: &ImportRejectedEventData) -> String {
    match data.status {
        ImportStatus::Skipped => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import skipped.".to_string()),
        ImportStatus::Failed => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import failed.".to_string()),
        _ => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import rejected.".to_string()),
    }
}

fn event_episode_ids(event: &DomainEvent) -> Vec<String> {
    let mut ids = Vec::new();
    let iter = match &event.payload {
        DomainEventPayload::ReleaseGrabbed(data) => data.episode_ids.iter(),
        DomainEventPayload::DownloadFailed(data) => data.episode_ids.iter(),
        DomainEventPayload::ReleaseBlocklisted(data) => data.episode_ids.iter(),
        DomainEventPayload::ImportCompleted(data) => data.episode_ids.iter(),
        DomainEventPayload::ImportRejected(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileAnalyzed(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileRenamed(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileDeleted(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileUpgraded(data) => data.episode_ids.iter(),
        _ => return ids,
    };

    for episode_id in iter {
        if !ids.contains(episode_id) {
            ids.push(episode_id.clone());
        }
    }
    ids
}

fn event_collection_id(event: &DomainEvent) -> Option<String> {
    match &event.payload {
        DomainEventPayload::DownloadFailed(data) => data.collection_id.clone(),
        DomainEventPayload::ReleaseBlocklisted(data) => data.collection_id.clone(),
        _ => None,
    }
}

fn configuration_changed_message(
    resource_type: &str,
    resource_id: Option<&str>,
    action: ConfigurationChangeAction,
) -> String {
    let target = resource_id.unwrap_or(resource_type);
    match action {
        ConfigurationChangeAction::Saved => format!("{target} saved"),
        ConfigurationChangeAction::Updated => format!("{target} updated"),
        ConfigurationChangeAction::Deleted => format!("{target} deleted"),
        ConfigurationChangeAction::Reordered => format!("{target} reordered"),
    }
}

fn discovery_search_completed_message(
    search_type: &str,
    query: Option<&str>,
    result_count: i64,
) -> String {
    match query.filter(|value| !value.trim().is_empty()) {
        Some(query) => format!("{search_type} searched: {query} ({result_count} results)"),
        None => format!("{search_type} search completed ({result_count} results)"),
    }
}

fn metadata_hydration_activity(
    state: MetadataHydrationState,
    title_name: &str,
    reason: Option<&str>,
) -> (ActivityKind, ActivitySeverity, String) {
    match state {
        MetadataHydrationState::Started => (
            ActivityKind::MetadataHydrationStarted,
            ActivitySeverity::Info,
            format!("hydrating metadata for {title_name}"),
        ),
        MetadataHydrationState::Completed => (
            ActivityKind::MetadataHydrationCompleted,
            ActivitySeverity::Success,
            format!("metadata hydrated for {title_name}"),
        ),
        MetadataHydrationState::Failed => (
            ActivityKind::MetadataHydrationFailed,
            ActivitySeverity::Warning,
            match reason.filter(|value| !value.trim().is_empty()) {
                Some(reason) => format!("metadata hydration failed for {title_name}: {reason}"),
                None => format!("metadata hydration failed for {title_name}"),
            },
        ),
    }
}

fn import_requested_message(client_type: &str, source_ref: &str) -> String {
    format!("manual import queued for {client_type} ({source_ref})")
}

fn post_processing_severity(result: PostProcessingResult) -> ActivitySeverity {
    match result {
        PostProcessingResult::Succeeded => ActivitySeverity::Success,
        PostProcessingResult::TimedOut | PostProcessingResult::Failed => ActivitySeverity::Warning,
    }
}

fn post_processing_message(
    script_name: &str,
    title_name: &str,
    result: PostProcessingResult,
    exit_code: Option<i32>,
) -> String {
    match result {
        PostProcessingResult::Succeeded => {
            format!("Post-processing '{script_name}' succeeded for '{title_name}'")
        }
        PostProcessingResult::TimedOut => {
            format!("Post-processing '{script_name}' timed out for '{title_name}'")
        }
        PostProcessingResult::Failed => format!(
            "Post-processing '{script_name}' failed (exit {}) for '{title_name}'",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ),
    }
}

fn download_queue_command_label(action: scryer_domain::DownloadQueueCommandAction) -> &'static str {
    match action {
        scryer_domain::DownloadQueueCommandAction::Pause => "paused",
        scryer_domain::DownloadQueueCommandAction::Resume => "resumed",
        scryer_domain::DownloadQueueCommandAction::Delete => "delete queued",
    }
}

fn apply_library_scan_event(
    sessions: &mut HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<LibraryScanSession> {
    reduce_library_scan_projection_event(sessions, event)
}

pub fn apply_library_scan_projection_event(
    sessions: &mut HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<LibraryScanSession> {
    apply_library_scan_event(sessions, event)
}

fn apply_job_run_event(
    runs: &mut HashMap<String, JobRun>,
    scans: &HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<JobRun> {
    fn merge_scan_status(run: &mut JobRun, session: LibraryScanSession) {
        run.library_scan_progress = Some(session.clone());
        run.status = match session.status {
            LibraryScanStatus::Discovering => JobRunStatus::Discovering,
            LibraryScanStatus::Running => JobRunStatus::Running,
            LibraryScanStatus::Completed => JobRunStatus::Completed,
            LibraryScanStatus::Canceled => JobRunStatus::Warning,
            LibraryScanStatus::Warning => JobRunStatus::Warning,
            LibraryScanStatus::Failed => JobRunStatus::Failed,
        };
        if run.status.is_terminal() {
            run.completed_at = Some(session.updated_at);
        }
    }

    match &event.payload {
        DomainEventPayload::JobRunStarted(data) => {
            let job_key = JobKey::parse(&data.job_key)?;
            let run = JobRun {
                id: data.run_id.clone(),
                operation_type: data.operation_type.clone(),
                actor_user_id: event.actor_user_id.clone(),
                job_key,
                display_name: job_key.display_name().to_string(),
                category: job_key.category(),
                section: job_key.section(),
                status: if job_key.uses_library_scan_progress() {
                    JobRunStatus::Discovering
                } else {
                    JobRunStatus::Running
                },
                trigger_source: JobTriggerSource::parse(&data.trigger_source)
                    .unwrap_or(JobTriggerSource::SystemInternal),
                started_at: event.occurred_at,
                completed_at: None,
                summary_json: None,
                summary_text: None,
                error_text: None,
                progress_json: None,
                library_scan_progress: scans.get(&data.run_id).cloned(),
            };
            runs.insert(data.run_id.clone(), run.clone());
            Some(run)
        }
        DomainEventPayload::JobRunCompleted(data) => {
            let mut run = runs.remove(&data.run_id)?;
            run.summary_text = data.summary_text.clone();
            run.completed_at = Some(event.occurred_at);
            run.status = JobRunStatus::Completed;
            Some(run)
        }
        DomainEventPayload::JobRunFailed(data) => {
            let mut run = runs.remove(&data.run_id)?;
            run.error_text = data.error_text.clone();
            run.summary_text = data
                .error_text
                .clone()
                .map(|error| format!("Failed: {error}"));
            run.completed_at = Some(event.occurred_at);
            run.status = JobRunStatus::Failed;
            Some(run)
        }
        DomainEventPayload::LibraryScanStarted(_)
        | DomainEventPayload::LibraryScanProgressed(_)
        | DomainEventPayload::LibraryScanCompleted(_)
        | DomainEventPayload::LibraryScanCanceled(_)
        | DomainEventPayload::LibraryScanFailed(_) => {
            let session_id = match &event.payload {
                DomainEventPayload::LibraryScanStarted(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanProgressed(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanCompleted(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanCanceled(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanFailed(data) => data.session_id.as_str(),
                _ => unreachable!(),
            };
            if let Some(run) = runs.get_mut(session_id) {
                if let Some(scan) = scans.get(session_id).cloned() {
                    merge_scan_status(run, scan);
                    return Some(run.clone());
                }
                let mut projected_scans = HashMap::new();
                if let Some(scan) = run.library_scan_progress.clone() {
                    projected_scans.insert(session_id.to_string(), scan);
                }
                if let Some(scan) =
                    reduce_library_scan_projection_event(&mut projected_scans, event)
                {
                    merge_scan_status(run, scan);
                }
                Some(run.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn apply_job_run_projection_event(
    runs: &mut HashMap<String, JobRun>,
    scans: &HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<JobRun> {
    apply_job_run_event(runs, scans, event)
}

pub fn replay_job_next_runs(events: &[DomainEvent]) -> HashMap<JobKey, DateTime<Utc>> {
    let mut next_runs = HashMap::new();
    for event in events {
        apply_job_next_run_event(&mut next_runs, event);
    }
    next_runs
}

pub fn apply_job_next_run_projection_event(
    next_runs: &mut HashMap<JobKey, DateTime<Utc>>,
    event: &DomainEvent,
) -> Option<(JobKey, Option<DateTime<Utc>>)> {
    apply_job_next_run_event(next_runs, event)
}

fn apply_job_next_run_event(
    next_runs: &mut HashMap<JobKey, DateTime<Utc>>,
    event: &DomainEvent,
) -> Option<(JobKey, Option<DateTime<Utc>>)> {
    let DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
        job_key,
        next_run_at,
    }) = &event.payload
    else {
        return None;
    };

    let job_key = JobKey::parse(job_key)?;
    let next_run_at = next_run_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    match next_run_at {
        Some(next_run_at) => {
            next_runs.insert(job_key, next_run_at);
            Some((job_key, Some(next_run_at)))
        }
        None => {
            next_runs.remove(&job_key);
            Some((job_key, None))
        }
    }
}

fn download_queue_item_key(item: &DownloadQueueItem) -> String {
    if item.client_id.trim().is_empty() {
        return format!("{}::{}", item.client_type, item.download_client_item_id);
    }

    format!("{}::{}", item.client_id, item.download_client_item_id)
}

fn apply_download_queue_event(
    items: &mut HashMap<String, DownloadQueueItem>,
    event: &DomainEvent,
) -> Option<Vec<DownloadQueueItem>> {
    match &event.payload {
        DomainEventPayload::DownloadQueueItemUpserted(upserted) => {
            let DownloadQueueItemUpsertedEventData { item } = upserted.as_ref();
            items.insert(download_queue_item_key(item), item.clone());
            Some(sorted_download_queue_items(items))
        }
        DomainEventPayload::DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData {
            download_client_item_id,
            client_id,
            client_type,
        }) => {
            if let Some(client_id) = client_id.as_ref().filter(|value| !value.trim().is_empty()) {
                items.remove(&format!("{client_id}::{download_client_item_id}"));
            } else if let Some(client_type) = client_type.as_ref() {
                items.remove(&format!("{client_type}::{download_client_item_id}"));
            } else {
                items.retain(|_, item| item.download_client_item_id != *download_client_item_id);
            }
            Some(sorted_download_queue_items(items))
        }
        _ => None,
    }
}

pub fn apply_download_queue_projection_event(
    items: &mut HashMap<String, DownloadQueueItem>,
    event: &DomainEvent,
) -> Option<Vec<DownloadQueueItem>> {
    apply_download_queue_event(items, event)
}

pub fn sorted_download_queue_items(
    items: &HashMap<String, DownloadQueueItem>,
) -> Vec<DownloadQueueItem> {
    let mut values = items.values().cloned().collect::<Vec<_>>();
    sort_download_queue_items(&mut values);
    values
}

pub fn sort_download_queue_items(items: &mut [DownloadQueueItem]) {
    items.sort_by(compare_download_queue_items);
}

pub fn compare_download_queue_items(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
) -> Ordering {
    let left_rank = queue_state_sort_rank(&left.state);
    let right_rank = queue_state_sort_rank(&right.state);
    if left_rank != right_rank {
        return left_rank.cmp(&right_rank);
    }

    match left.state {
        DownloadQueueState::Downloading
        | DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => right
            .progress_percent
            .cmp(&left.progress_percent)
            .then_with(|| left.id.cmp(&right.id)),
        DownloadQueueState::Queued | DownloadQueueState::Paused => {
            compare_queue_sort_values(left.queued_at.as_deref(), right.queued_at.as_deref())
                .then_with(|| left.id.cmp(&right.id))
        }
        _ => compare_queue_sort_values(
            right.last_updated_at.as_deref(),
            left.last_updated_at.as_deref(),
        )
        .then_with(|| left.id.cmp(&right.id)),
    }
}

fn compare_queue_sort_values(left: Option<&str>, right: Option<&str>) -> Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}

fn queue_state_sort_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Downloading
        | DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Paused => 2,
        DownloadQueueState::ImportPending | DownloadQueueState::Completed => 3,
        // Both states want the operator's attention, so they sort together at
        // the end; only their handling differs.
        DownloadQueueState::Warning | DownloadQueueState::Failed => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use scryer_domain::{
        DomainEventStream, DomainExternalIds, DownloadFailedEventData, DownloadQueueCommandAction,
        DownloadQueueItemCommandIssuedEventData, DownloadQueueState, ImportCompletedEventData,
        JobRunStartedEventData, LibraryScanCompletedEventData, LibraryScanProgressedEventData,
        MediaFacet, MediaFileAnalyzedEventData, MediaFileDeletedEventData, MediaFileDeletedReason,
        MediaFileUpgradedEventData, MediaPathUpdate, MediaUpdateType, ReleaseGrabbedEventData,
        TitleContextSnapshot,
    };

    fn title_snapshot(name: &str, facet: MediaFacet) -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: name.to_string(),
            facet,
            external_ids: DomainExternalIds::default(),
            poster_url: None,
            year: Some(2024),
        }
    }

    #[test]
    fn download_activity_payload_freeze_fixtures_are_byte_identical() {
        let title = title_snapshot("Fixture", MediaFacet::Series);
        let fixtures = [
            (
                DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                    title: title.clone(),
                    source_title: Some("Grab.Release".to_string()),
                    source_hint: Some("rss".to_string()),
                    source_provider: None,
                    download_id: Some("download-1".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                }),
                r#"{"type":"release_grabbed","data":{"title":{"title_name":"Fixture","facet":"series","external_ids":{"imdb_id":null,"tmdb_id":null,"tvdb_id":null,"anidb_id":null},"poster_url":null,"year":2024},"source_title":"Grab.Release","source_hint":"rss","source_provider":null,"download_id":"download-1","episode_ids":["episode-1"]}}"#,
                ActivityKind::AcquisitionCandidateAccepted,
                ActivitySeverity::Success,
                "Grabbed 'Grab.Release' for 'Fixture'.",
            ),
            (
                DomainEventPayload::DownloadQueueItemCommandIssued(
                    DownloadQueueItemCommandIssuedEventData {
                        item_id: "queue-1".to_string(),
                        action: DownloadQueueCommandAction::Delete,
                    },
                ),
                r#"{"type":"download_queue_item_command_issued","data":{"item_id":"queue-1","action":"delete"}}"#,
                ActivityKind::SystemNotice,
                ActivitySeverity::Info,
                "download delete queued: queue-1",
            ),
            (
                DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                    title: Some(title.clone()),
                    source_title: Some("Broken.Release".to_string()),
                    source_hint: Some("client".to_string()),
                    download_id: Some("download-1".to_string()),
                    client_id: Some("client-1".to_string()),
                    client_name: Some("Fixture Client".to_string()),
                    client_type: Some("nzbget".to_string()),
                    quality: Some("1080p".to_string()),
                    reason: Some("archive corrupt".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                    collection_id: Some("collection-1".to_string()),
                }),
                r#"{"type":"download_failed","data":{"title":{"title_name":"Fixture","facet":"series","external_ids":{"imdb_id":null,"tmdb_id":null,"tvdb_id":null,"anidb_id":null},"poster_url":null,"year":2024},"source_title":"Broken.Release","source_hint":"client","download_id":"download-1","client_id":"client-1","client_name":"Fixture Client","client_type":"nzbget","quality":"1080p","reason":"archive corrupt","episode_ids":["episode-1"],"collection_id":"collection-1"}}"#,
                ActivityKind::AcquisitionDownloadFailed,
                ActivitySeverity::Warning,
                "Download failed for 'Broken.Release'.",
            ),
            (
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title,
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Fixture.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    import_id: Some("import-1".to_string()),
                    source_system: Some("nzbget".to_string()),
                    source_ref: Some("queue-1".to_string()),
                    source_title: Some("Imported.Release".to_string()),
                    source_path: Some("/downloads/Fixture.mkv".to_string()),
                    dest_path: Some("/library/Fixture.mkv".to_string()),
                    quality: Some("1080p".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                    size_bytes: Some(1024),
                }),
                r#"{"type":"import_completed","data":{"title":{"title_name":"Fixture","facet":"series","external_ids":{"imdb_id":null,"tmdb_id":null,"tvdb_id":null,"anidb_id":null},"poster_url":null,"year":2024},"media_updates":[{"path":"/library/Fixture.mkv","update_type":"created"}],"imported_count":1,"import_id":"import-1","source_system":"nzbget","source_ref":"queue-1","source_title":"Imported.Release","source_path":"/downloads/Fixture.mkv","dest_path":"/library/Fixture.mkv","quality":"1080p","episode_ids":["episode-1"],"size_bytes":1024}}"#,
                ActivityKind::SeriesEpisodeImported,
                ActivitySeverity::Success,
                "Imported 1 file for 'Fixture'.",
            ),
        ];

        for (index, (payload, expected_json, kind, severity, message)) in
            fixtures.into_iter().enumerate()
        {
            assert_eq!(
                serde_json::to_vec(&payload).expect("fixture payload should serialize"),
                expected_json.as_bytes(),
                "fixture {index} payload changed",
            );

            let domain_event = event(index as i64, Utc::now(), payload);
            let activity = activity_event_from_domain_event(&domain_event)
                .expect("fixture should project to activity");
            assert_eq!(activity.kind, kind, "fixture {index} activity kind changed");
            assert_eq!(
                activity.severity, severity,
                "fixture {index} activity severity changed"
            );
            assert_eq!(
                activity.message, message,
                "fixture {index} activity message changed"
            );
            assert_eq!(
                activity.episode_ids,
                match &domain_event.payload {
                    DomainEventPayload::ImportCompleted(data) => data.episode_ids.clone(),
                    _ => Vec::new(),
                },
                "fixture {index} import episode context changed"
            );

            if index != 1 {
                let history = title_history_record_from_domain_event(&domain_event)
                    .expect("title-scoped fixture should project to history");
                assert_eq!(
                    history.data_json.as_deref(),
                    Some(expected_json),
                    "fixture {index} history payload changed"
                );
            }
        }
    }

    fn event(
        sequence: i64,
        occurred_at: DateTime<Utc>,
        payload: DomainEventPayload,
    ) -> DomainEvent {
        DomainEvent {
            sequence,
            event_id: format!("evt-{sequence}"),
            occurred_at,
            actor_kind: scryer_domain::DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some("title-1".to_string()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Global,
            payload,
        }
    }

    #[test]
    fn sanitize_history_json_redacts_api_key_fields_and_query_params() {
        let sanitized = sanitize_history_json_value(serde_json::json!({
            "api_key": "super-secret",
            "nested": {
                "provider_api_key": "also-secret",
                "source_hint": "http://api.nzbgeek.info/api?t=get&id=abc123&apikey=third-secret",
            },
        }));

        assert_eq!(sanitized["api_key"], REDACTED_HISTORY_SECRET);
        assert_eq!(
            sanitized["nested"]["provider_api_key"],
            REDACTED_HISTORY_SECRET
        );
        assert_eq!(sanitized["nested"]["source_hint"], "api.nzbgeek.info");
    }

    #[test]
    fn grabbed_history_prefers_configured_provider_without_discarding_source_url() {
        let source_url = "https://indexer.example/api?t=get&id=release-1";
        let event = event(
            1,
            Utc::now(),
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_snapshot("Example", MediaFacet::Movie),
                source_title: Some("Example.2026.1080p.WEB-DL".to_string()),
                source_hint: Some(source_url.to_string()),
                source_provider: Some("Configured Indexer".to_string()),
                download_id: Some("download-1".to_string()),
                episode_ids: Vec::new(),
            }),
        );

        let history = title_history_record_from_domain_event(&event).expect("history record");
        assert_eq!(history.source_hint.as_deref(), Some("Configured Indexer"));
        let data_json = history.data_json.expect("history event data");
        assert!(data_json.contains("indexer.example"));
    }

    #[test]
    fn manually_grabbed_release_preserves_the_requesting_user_in_history() {
        let mut event = event(
            1,
            Utc::now(),
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_snapshot("Example", MediaFacet::Movie),
                source_title: Some("Example.2026.1080p.WEB-DL".to_string()),
                source_hint: Some("Indexer".to_string()),
                source_provider: Some("Indexer".to_string()),
                download_id: Some("download-1".to_string()),
                episode_ids: Vec::new(),
            }),
        );
        event.actor_kind = scryer_domain::DomainEventActorKind::User;
        event.actor_user_id = Some("user-1".to_string());
        event.actor_display_name = "Manual Grabber".to_string();

        let history = title_history_record_from_domain_event(&event).expect("history record");
        assert_eq!(
            history.actor_kind,
            Some(scryer_domain::DomainEventActorKind::User)
        );
        assert_eq!(history.actor_user_id.as_deref(), Some("user-1"));
        assert_eq!(
            history.actor_display_name.as_deref(),
            Some("Manual Grabber")
        );
    }

    #[test]
    fn title_history_record_redacts_api_keys_before_reaching_ui() {
        let source_hint =
            "http://api.nzbgeek.info/api?t=get&id=abc123&apikey=super-secret".to_string();
        let failure_reason = format!("grab failed while fetching {source_hint}");
        let record = title_history_record_from_domain_event(&event(
            1,
            Utc::now(),
            DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                title: Some(title_snapshot("Planetes", MediaFacet::Anime)),
                source_title: None,
                source_hint: Some(source_hint),
                download_id: None,
                client_id: None,
                client_name: None,
                client_type: None,
                quality: Some("1080p".to_string()),
                reason: Some(failure_reason),
                episode_ids: vec!["ep-1".to_string()],
                collection_id: None,
            }),
        ))
        .expect("download failed event should project to title history");

        let expected_hint = "api.nzbgeek.info";
        assert_eq!(record.source_hint.as_deref(), Some(expected_hint));
        assert_eq!(record.display_title.as_deref(), Some(expected_hint));
        assert_eq!(
            record.failure_reason.as_deref(),
            Some("grab failed while fetching api.nzbgeek.info")
        );

        let data_json = record
            .data_json
            .expect("history payload should be serialized");
        assert!(data_json.contains(expected_hint));
        assert!(!data_json.contains("super-secret"));
        assert!(!data_json.contains("/api?t=get"));
    }

    #[test]
    fn upgrade_recycle_and_purge_project_in_audit_order() {
        let now = Utc::now();
        let snapshot = title_snapshot("Example", MediaFacet::Series);
        let events = [
            event(
                1,
                now,
                DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: snapshot.clone(),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/new.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    episode_ids: vec!["ep-1".to_string()],
                    previous_file_id: Some("old-file".to_string()),
                    current_file_id: Some("new-file".to_string()),
                    old_score: Some(10),
                    new_score: Some(20),
                    size_bytes: Some(2_048),
                }),
            ),
            event(
                2,
                now + Duration::seconds(1),
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: snapshot.clone(),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/old.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("old-file".to_string()),
                    reason: MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: vec!["ep-1".to_string()],
                }),
            ),
            event(
                3,
                now + Duration::seconds(2),
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: snapshot,
                    media_updates: vec![MediaPathUpdate {
                        path: "/recycle/old.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("old-file".to_string()),
                    reason: MediaFileDeletedReason::RecycleBinPurged,
                    episode_ids: Vec::new(),
                }),
            ),
        ];

        let history = events
            .iter()
            .filter_map(title_history_record_from_domain_event)
            .collect::<Vec<_>>();

        assert_eq!(
            history
                .iter()
                .map(|record| record.event_type)
                .collect::<Vec<_>>(),
            vec![
                TitleHistoryEventType::FileUpgraded,
                TitleHistoryEventType::FileRecycled,
                TitleHistoryEventType::FileDeleted,
            ]
        );
        assert_eq!(history[0].episode_ids, vec!["ep-1".to_string()]);
        assert_eq!(history[1].episode_ids, vec!["ep-1".to_string()]);
    }

    fn queue_item(
        id: &str,
        state: DownloadQueueState,
        progress_percent: u8,
        queued_at: i64,
        last_updated_at: i64,
    ) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: Some(queued_at.to_string()),
            last_updated_at: Some(last_updated_at.to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        }
    }

    #[test]
    fn episode_history_returns_most_recent_records_first() {
        let now = Utc::now();
        let events = vec![
            event(
                1,
                now,
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_snapshot("Example", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/old.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    import_id: None,
                    source_system: None,
                    source_ref: None,
                    source_title: Some("Example.S01E01.1080p".to_string()),
                    source_path: Some("/downloads/Example.S01E01.1080p.mkv".to_string()),
                    dest_path: Some("/data/old.mkv".to_string()),
                    quality: Some("1080p".to_string()),
                    episode_ids: vec!["ep-1".to_string()],
                    size_bytes: Some(1_024),
                }),
            ),
            event(
                2,
                now + Duration::seconds(60),
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_snapshot("Example", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/new.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    import_id: None,
                    source_system: None,
                    source_ref: None,
                    source_title: Some("Example.S01E01.2160p".to_string()),
                    source_path: Some("/downloads/Example.S01E01.2160p.mkv".to_string()),
                    dest_path: Some("/data/new.mkv".to_string()),
                    quality: Some("2160p".to_string()),
                    episode_ids: vec!["ep-1".to_string()],
                    size_bytes: Some(4_096),
                }),
            ),
        ];

        let records = title_history_records_for_episode_from_domain_events(&events, "ep-1", 10);

        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].source_title.as_deref(),
            Some("Example.S01E01.2160p")
        );
        assert_eq!(
            records[1].source_title.as_deref(),
            Some("Example.S01E01.1080p")
        );
    }

    #[test]
    fn media_file_analyzed_projects_to_file_analyzed_activity() {
        let domain_event = event(
            1,
            Utc::now(),
            DomainEventPayload::MediaFileAnalyzed(MediaFileAnalyzedEventData {
                title: title_snapshot("Example", MediaFacet::Series),
                media_updates: vec![MediaPathUpdate {
                    path: "/data/episode.mkv".to_string(),
                    update_type: MediaUpdateType::Modified,
                }],
                file_id: "file-1".to_string(),
                analysis_status: "scanned".to_string(),
                episode_ids: vec!["ep-1".to_string()],
            }),
        );
        let activity = activity_event_from_domain_event(&domain_event)
            .expect("media file analyzed should project to activity");

        assert_eq!(activity.kind, ActivityKind::FileAnalyzed);
        assert_eq!(activity.severity, ActivitySeverity::Info);
        assert_eq!(activity.title_id.as_deref(), Some("title-1"));

        let history = title_history_record_from_domain_event(&domain_event)
            .expect("media file analyzed should project to title history");
        assert_eq!(history.event_type, TitleHistoryEventType::Scanned);
        assert_eq!(history.episode_ids, vec!["ep-1".to_string()]);
        assert_eq!(history.source_title.as_deref(), Some("/data/episode.mkv"));
    }

    #[test]
    fn import_rejected_projects_to_title_scoped_activity() {
        let activity = activity_event_from_domain_event(&event(
            1,
            Utc::now(),
            DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: Some(title_snapshot("Example", MediaFacet::Series)),
                status: ImportStatus::Failed,
                import_id: Some("import-1".to_string()),
                source_system: Some("sabnzbd".to_string()),
                source_ref: Some("job-1".to_string()),
                source_title: Some("Example.S01E01.1080p".to_string()),
                source_path: Some("/downloads/Example.S01E01.1080p".to_string()),
                dest_path: None,
                quality: Some("1080p".to_string()),
                reason: Some("Policy mismatch".to_string()),
                skip_reason: None,
                episode_ids: vec!["ep-1".to_string()],
            }),
        ))
        .expect("import rejected should project to activity");

        assert_eq!(activity.kind, ActivityKind::ImportRejected);
        assert_eq!(activity.severity, ActivitySeverity::Warning);
        assert_eq!(activity.title_id.as_deref(), Some("title-1"));
        assert_eq!(activity.facet.as_deref(), Some("series"));
    }

    #[test]
    fn sorted_download_queue_items_matches_query_ordering_contract() {
        let items = HashMap::from([
            (
                "queued".to_string(),
                queue_item("queued", DownloadQueueState::Queued, 0, 10, 10),
            ),
            (
                "failed".to_string(),
                queue_item("failed", DownloadQueueState::Failed, 0, 10, 50),
            ),
            (
                "completed-newer".to_string(),
                queue_item("completed-newer", DownloadQueueState::Completed, 0, 10, 30),
            ),
            (
                "downloading-fast".to_string(),
                queue_item(
                    "downloading-fast",
                    DownloadQueueState::Downloading,
                    80,
                    10,
                    10,
                ),
            ),
            (
                "downloading-slower".to_string(),
                queue_item(
                    "downloading-slower",
                    DownloadQueueState::Downloading,
                    35,
                    10,
                    10,
                ),
            ),
            (
                "completed-older".to_string(),
                queue_item("completed-older", DownloadQueueState::Completed, 0, 10, 20),
            ),
        ]);

        let ordered = sorted_download_queue_items(&items)
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "downloading-fast".to_string(),
                "downloading-slower".to_string(),
                "queued".to_string(),
                "completed-newer".to_string(),
                "completed-older".to_string(),
                "failed".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_library_scan_event_updates_job_run_projection() {
        let now = Utc::now();
        let run_id = "run-1";
        let mut runs = HashMap::new();
        let mut scans = HashMap::new();

        let started = event(
            1,
            now,
            DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                run_id: run_id.to_string(),
                job_key: JobKey::BackgroundLibraryRefreshSeries.as_str().to_string(),
                operation_type: "library_scan".to_string(),
                trigger_source: JobTriggerSource::Manual.as_str().to_string(),
            }),
        );
        let progress = event(
            2,
            now + Duration::seconds(5),
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: run_id.to_string(),
                status: "running".to_string(),
                found_titles: 3,
                title_match_completed: 2,
                title_match_total_known: false,
                titles_completed: 2,
                titles_total: Some(5),
                files_completed: 4,
                files_total: Some(9),
                warning_message: None,
            }),
        );
        let completed = event(
            3,
            now + Duration::seconds(10),
            DomainEventPayload::LibraryScanCompleted(LibraryScanCompletedEventData {
                session_id: run_id.to_string(),
                status: "completed".to_string(),
                found_titles: 5,
                title_match_completed: 5,
                title_match_total_known: true,
                titles_completed: 5,
                titles_total: Some(5),
                files_completed: 9,
                files_total: Some(9),
                summary: None,
                warning_message: None,
            }),
        );

        let run = apply_job_run_projection_event(&mut runs, &scans, &started)
            .expect("job start should create a run");
        assert_eq!(run.status, JobRunStatus::Discovering);

        let _ = apply_library_scan_projection_event(&mut scans, &progress);
        let running = apply_job_run_projection_event(&mut runs, &scans, &progress)
            .expect("progress should update the run");
        assert_eq!(running.status, JobRunStatus::Running);

        let _ = apply_library_scan_projection_event(&mut scans, &completed);
        let terminal = apply_job_run_projection_event(&mut runs, &scans, &completed)
            .expect("terminal scan event should update the run");
        assert_eq!(terminal.status, JobRunStatus::Completed);
        assert!(
            terminal
                .library_scan_progress
                .as_ref()
                .is_some_and(|scan| scan.status == LibraryScanStatus::Completed)
        );
        assert_eq!(terminal.completed_at, Some(completed.occurred_at));
    }

    #[test]
    fn completed_library_scan_event_replays_summary_counts() {
        let now = Utc::now();
        let completed = event(
            1,
            now,
            DomainEventPayload::LibraryScanCompleted(LibraryScanCompletedEventData {
                session_id: "session-1".to_string(),
                status: "completed".to_string(),
                found_titles: 3,
                title_match_completed: 3,
                title_match_total_known: true,
                titles_completed: 2,
                titles_total: Some(2),
                files_completed: 3,
                files_total: Some(3),
                summary: Some(scryer_domain::LibraryScanSummaryEventData {
                    scanned: 3,
                    matched: 2,
                    imported: 2,
                    skipped: 1,
                    unmatched: 0,
                }),
                warning_message: None,
            }),
        );

        let mut scans = HashMap::new();
        let session = apply_library_scan_projection_event(&mut scans, &completed)
            .expect("completed event should project a terminal session");

        assert_eq!(session.status, LibraryScanStatus::Completed);
        assert_eq!(
            session.summary.as_ref().map(|summary| summary.imported),
            Some(2)
        );
        assert_eq!(
            session.summary.as_ref().map(|summary| summary.skipped),
            Some(1)
        );
    }
}
