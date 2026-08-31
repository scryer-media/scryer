use crate::ports::{NOTIFICATION_REQUEST_SCHEMA_VERSION, NotificationMediaRequestPayload};
use crate::{
    AppUseCase, NotificationActorPayload, NotificationAppPayload, NotificationDownloadPayload,
    NotificationEpisodePayload, NotificationExternalIdsPayload, NotificationFilePayload,
    NotificationImportPayload, NotificationMediaFilePayload, NotificationMediaUpdatePayload,
    NotificationMediaUpdateTypePayload, NotificationPayload, NotificationReleasePayload,
    NotificationSeverityPayload, NotificationTitlePayload,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType, DomainExternalIds,
    DownloadFailedEventData, Episode, ExternalId, ImportCompletedEventData,
    ImportRejectedEventData, MediaFileDeletedEventData, MediaFileDeletedReason,
    MediaFileRenamedEventData, MediaFileUpgradedEventData, MediaPathUpdate,
    MediaRequestResolvedEventData, MediaRequestSubmittedEventData, MediaUpdateType,
    NotificationEventType, NotificationTargetKind, PostProcessingCompletedEventData,
    PostProcessingResult, ReleaseGrabbedEventData, SubtitleDownloadedEventData,
    SubtitleSearchFailedEventData, Title, TitleAddedEventData, TitleContextSnapshot,
    TitleDeletedEventData,
};
use std::collections::{BTreeMap, BTreeSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const NOTIFICATION_SUBSCRIBER: &str = "notification_dispatcher";
const NOTIFICATION_BATCH_LIMIT: usize = 100;

macro_rules! notification_event_mappings {
    ($macro:ident $(, $extra:expr)*) => {
        $macro! {
            $($extra,)*
            title_added => DomainEventPayload::TitleAdded(_) => DomainEventPayload::TitleAdded(data) => DomainEventType::TitleAdded => NotificationEventType::TitleAdded => build_title_added_notification(data),
            title_deleted => DomainEventPayload::TitleDeleted(_) => DomainEventPayload::TitleDeleted(data) => DomainEventType::TitleDeleted => NotificationEventType::TitleDeleted => build_title_deleted_notification(data),
            release_grabbed => DomainEventPayload::ReleaseGrabbed(_) => DomainEventPayload::ReleaseGrabbed(data) => DomainEventType::ReleaseGrabbed => NotificationEventType::Grab => build_release_grabbed_notification(data),
            download_failed => DomainEventPayload::DownloadFailed(_) => DomainEventPayload::DownloadFailed(data) => DomainEventType::DownloadFailed => NotificationEventType::Download => build_download_failed_notification(data),
            import_completed => DomainEventPayload::ImportCompleted(_) => DomainEventPayload::ImportCompleted(data) => DomainEventType::ImportCompleted => NotificationEventType::ImportComplete => build_import_completed_notification(data),
            import_rejected => DomainEventPayload::ImportRejected(_) => DomainEventPayload::ImportRejected(data) => DomainEventType::ImportRejected => NotificationEventType::ImportRejected => build_import_rejected_notification(data),
            media_file_upgraded => DomainEventPayload::MediaFileUpgraded(_) => DomainEventPayload::MediaFileUpgraded(data) => DomainEventType::MediaFileUpgraded => NotificationEventType::Upgrade => build_media_file_upgraded_notification(data),
            media_file_renamed => DomainEventPayload::MediaFileRenamed(_) => DomainEventPayload::MediaFileRenamed(data) => DomainEventType::MediaFileRenamed => NotificationEventType::Rename => build_media_file_renamed_notification(data),
            media_file_deleted_upgrade => DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData { reason: MediaFileDeletedReason::UpgradeCleanup, .. }) => DomainEventPayload::MediaFileDeleted(data @ MediaFileDeletedEventData { reason: MediaFileDeletedReason::UpgradeCleanup, .. }) => DomainEventType::MediaFileDeleted => NotificationEventType::FileDeletedForUpgrade => build_media_file_deleted_notification(data, NotificationEventType::FileDeletedForUpgrade),
            media_file_deleted => DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData { reason: MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk, .. }) => DomainEventPayload::MediaFileDeleted(data @ MediaFileDeletedEventData { reason: MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk, .. }) => DomainEventType::MediaFileDeleted => NotificationEventType::FileDeleted => build_media_file_deleted_notification(data, NotificationEventType::FileDeleted),
            post_processing_completed => DomainEventPayload::PostProcessingCompleted(_) => DomainEventPayload::PostProcessingCompleted(data) => DomainEventType::PostProcessingCompleted => NotificationEventType::PostProcessingCompleted => build_post_processing_completed_notification(data),
            subtitle_downloaded => DomainEventPayload::SubtitleDownloaded(_) => DomainEventPayload::SubtitleDownloaded(data) => DomainEventType::SubtitleDownloaded => NotificationEventType::SubtitleDownloaded => build_subtitle_downloaded_notification(data),
            subtitle_search_failed => DomainEventPayload::SubtitleSearchFailed(_) => DomainEventPayload::SubtitleSearchFailed(data) => DomainEventType::SubtitleSearchFailed => NotificationEventType::SubtitleSearchFailed => build_subtitle_search_failed_notification(data),
            media_request_submitted => DomainEventPayload::MediaRequestSubmitted(_) => DomainEventPayload::MediaRequestSubmitted(data) => DomainEventType::MediaRequestSubmitted => NotificationEventType::MediaRequestSubmitted => build_media_request_submitted_notification(data),
            media_request_approved => DomainEventPayload::MediaRequestApproved(_) => DomainEventPayload::MediaRequestApproved(data) => DomainEventType::MediaRequestApproved => NotificationEventType::MediaRequestApproved => build_media_request_resolved_notification(data, NotificationEventType::MediaRequestApproved),
            media_request_rejected => DomainEventPayload::MediaRequestRejected(_) => DomainEventPayload::MediaRequestRejected(data) => DomainEventType::MediaRequestRejected => NotificationEventType::MediaRequestRejected => build_media_request_resolved_notification(data, NotificationEventType::MediaRequestRejected),
            media_request_canceled => DomainEventPayload::MediaRequestCanceled(_) => DomainEventPayload::MediaRequestCanceled(data) => DomainEventType::MediaRequestCanceled => NotificationEventType::MediaRequestCanceled => build_media_request_resolved_notification(data, NotificationEventType::MediaRequestCanceled),
        }
    };
}

macro_rules! notification_domain_event_type_list {
    ($( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        const NOTIFICATION_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
            $( $domain_event_type, )*
        ];
    };
}

notification_event_mappings!(notification_domain_event_type_list);

macro_rules! notification_supported_event_type_list {
    ($( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        const SUPPORTED_NOTIFICATION_EVENT_TYPES: &[NotificationEventType] = &[
            $( $notification_event_type, )*
        ];
    };
}

notification_event_mappings!(notification_supported_event_type_list);

macro_rules! notification_event_type_match {
    ($payload:expr, $( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        match $payload {
            $( $type_pattern => Some($notification_event_type), )*
            _ => None,
        }
    };
}

macro_rules! notification_build_match {
    ($payload:expr, $( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        match $payload {
            $( $build_pattern => Some($builder), )*
            _ => None,
        }
    };
}

pub async fn start_notification_dispatcher(app: AppUseCase, cancel: CancellationToken) {
    info!("notification dispatcher started");
    let repo = app.services.events.domain_events.clone();
    let mut rx = app.runtime.events.notification_event_broadcast.subscribe();
    let mut last_sequence = match repo.get_subscriber_offset(NOTIFICATION_SUBSCRIBER).await {
        Ok(sequence) => sequence,
        Err(error) => {
            warn!(error = %error, "failed to load notification subscriber offset; starting at 0");
            0
        }
    };
    // Send-side filtering keeps operational bursts from waking this dispatcher, but persisted
    // filtered replay stays authoritative. The broadcast payload is only a high-water hint used
    // to avoid needless catch-up queries when we already processed that range.
    let mut should_poll = true;

    loop {
        if should_poll {
            match dispatch_pending_events(&app, last_sequence).await {
                Ok(sequence) => last_sequence = sequence,
                Err(error) => {
                    warn!(error = %error, "notification dispatcher failed to process pending events")
                }
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("notification dispatcher shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(high_water_sequence) => {
                        should_poll = high_water_sequence > last_sequence;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "notification dispatcher lagged, resyncing from persisted domain events");
                        should_poll = true;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("notification event broadcast closed, notification dispatcher exiting");
                        break;
                    }
                }
            }
        }
    }
}

async fn dispatch_pending_events(
    app: &AppUseCase,
    mut after_sequence: i64,
) -> crate::AppResult<i64> {
    let repo = app.services.events.domain_events.clone();

    loop {
        let events = repo
            .list(&DomainEventFilter {
                event_types: Some(NOTIFICATION_DOMAIN_EVENT_TYPES.to_vec()),
                after_sequence: Some(after_sequence),
                limit: NOTIFICATION_BATCH_LIMIT,
                ..DomainEventFilter::default()
            })
            .await?;
        if events.is_empty() {
            break;
        }

        for event in events {
            dispatch_event(app, &event).await;
            after_sequence = event.sequence;
            repo.set_subscriber_offset(NOTIFICATION_SUBSCRIBER, after_sequence)
                .await?;
        }
    }

    Ok(after_sequence)
}

pub(crate) fn notification_event_type(
    payload: &DomainEventPayload,
) -> Option<NotificationEventType> {
    notification_event_mappings!(notification_event_type_match, payload)
}

pub(crate) fn supported_notification_event_types() -> &'static [NotificationEventType] {
    SUPPORTED_NOTIFICATION_EVENT_TYPES
}

async fn dispatch_event(app: &AppUseCase, event: &DomainEvent) {
    let Some(notification) = build_notification(event) else {
        return;
    };
    let notification = enrich_notification(app, event, notification).await;

    let sub_repo = match app.notification_subscriptions_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let ch_repo = match app.notification_channels_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let Some(provider) = app.services.notifications.notification_provider() else {
        return;
    };

    let event_type = event.payload.event_type();
    debug!(
        event_type = event_type.as_str(),
        title_id = ?event.title_id,
        sequence = event.sequence,
        "dispatching domain-event-backed notification"
    );

    let scope_title_id = notification_scope_title_id(event);
    let scope_facet = notification_scope_facet(event);

    let mut subscriptions = Vec::new();
    for subscription_event_type in subscription_event_types(notification.payload.event_type) {
        match sub_repo
            .list_subscriptions_for_event(subscription_event_type)
            .await
        {
            Ok(mut matching) => subscriptions.append(&mut matching),
            Err(error) => {
                warn!(
                    error = %error,
                    event_type = subscription_event_type.as_str(),
                    "failed to list notification subscriptions"
                );
                return;
            }
        }
    }
    subscriptions.sort_by(|left, right| left.id.cmp(&right.id));
    subscriptions.dedup_by(|left, right| left.id == right.id);
    let mut dispatched_targets = BTreeSet::new();

    for subscription in subscriptions {
        if !subscription.is_enabled {
            continue;
        }

        if !matches_scope(
            &subscription.scope,
            subscription.scope_id.as_deref(),
            scope_title_id,
            scope_facet,
        ) {
            continue;
        }

        if !dispatched_targets.insert((
            subscription.target_kind.as_str(),
            subscription.target_id.clone(),
        )) {
            continue;
        }

        let channel = match subscription.target_kind {
            NotificationTargetKind::PluginChannel => {
                let Some(channel_id) = subscription.channel_id.as_deref() else {
                    warn!(
                        subscription_id = subscription.id.as_str(),
                        target_id = subscription.target_id.as_str(),
                        "plugin notification subscription is missing channel_id"
                    );
                    continue;
                };
                match ch_repo.get_channel(channel_id).await {
                    Ok(Some(channel)) if channel.is_enabled => channel,
                    Ok(_) => continue,
                    Err(error) => {
                        warn!(
                            subscription_id = subscription.id.as_str(),
                            channel_id,
                            error = %error,
                            "failed to load notification channel"
                        );
                        continue;
                    }
                }
            }
            NotificationTargetKind::MediaServerConnection => match app
                .notification_channel_for_media_server_target(&subscription.target_id)
                .await
            {
                Ok(channel) if channel.is_enabled => channel,
                Ok(_) => continue,
                Err(error) => {
                    warn!(
                        subscription_id = subscription.id.as_str(),
                        target_id = subscription.target_id.as_str(),
                        error = %error,
                        "failed to resolve media server notification target"
                    );
                    continue;
                }
            },
        };
        let channel = match app
            .notification_channel_with_resolved_media_server_config(channel)
            .await
        {
            Ok(channel) => channel,
            Err(error) => {
                warn!(
                    target_kind = subscription.target_kind.as_str(),
                    target_id = subscription.target_id.as_str(),
                    error = %error,
                    "failed to resolve notification target configuration"
                );
                continue;
            }
        };

        let supported_events =
            provider.supported_events_for_provider(channel.channel_type.as_str());
        if !supported_events.is_empty()
            && !supported_events.contains(&notification.payload.event_type)
        {
            warn!(
                channel_type = channel.channel_type.as_str(),
                channel_name = channel.name.as_str(),
                event_type = notification.payload.event_type.as_str(),
                "notification plugin no longer supports subscribed event"
            );
            continue;
        }

        let client = match provider.client_for_channel(&channel) {
            Some(client) => client,
            None => {
                warn!(
                    channel_type = channel.channel_type.as_str(),
                    channel_name = channel.name.as_str(),
                    "no notification plugin available for channel type"
                );
                continue;
            }
        };

        match client.send_notification(&notification.payload).await {
            Ok(()) => {
                info!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.payload.event_type.as_str(),
                    channel = channel.name.as_str(),
                    "notification dispatched"
                );
            }
            Err(error) => {
                warn!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.payload.event_type.as_str(),
                    channel = channel.name.as_str(),
                    error = %error,
                    "notification dispatch failed"
                );
            }
        }
    }
}

struct BuiltNotification {
    payload: NotificationPayload,
}

fn build_notification(event: &DomainEvent) -> Option<BuiltNotification> {
    notification_event_mappings!(notification_build_match, &event.payload)
}

fn build_title_added_notification(data: &TitleAddedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::TitleAdded,
            format!("Added: {}", data.title.title_name),
            format!("Added '{}' to Scryer.", data.title.title_name),
            Some(&data.title),
            &[],
            &[],
        ),
    }
}

fn build_title_deleted_notification(data: &TitleDeletedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::TitleDeleted,
            format!("Deleted: {}", data.title.title_name),
            format!("Deleted '{}' from Scryer.", data.title.title_name),
            Some(&data.title),
            &[],
            &[],
        ),
    }
}

fn build_release_grabbed_notification(data: &ReleaseGrabbedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::Grab,
        format!("Grabbed: {}", data.title.title_name),
        data.source_title
            .as_ref()
            .map(|source_title| {
                format!(
                    "Grabbed '{}' for '{}'.",
                    source_title, data.title.title_name
                )
            })
            .unwrap_or_else(|| format!("Grabbed a release for '{}'.", data.title.title_name)),
        Some(&data.title),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        source_hint: data.source_hint.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        download_id: data.download_id.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_download_failed_notification(data: &DownloadFailedEventData) -> BuiltNotification {
    let title = data
        .title
        .as_ref()
        .map(|title| title.title_name.as_str())
        .unwrap_or("Unknown title");
    let mut payload = base_notification_payload(
        NotificationEventType::Download,
        format!("Download failed: {title}"),
        data.reason
            .clone()
            .unwrap_or_else(|| "Download failed.".to_string()),
        data.title.as_ref(),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        source_hint: data.source_hint.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        download_id: data.download_id.clone(),
        client_id: data.client_id.clone(),
        client_name: data.client_name.clone(),
        client_type: data.client_type.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_import_completed_notification(data: &ImportCompletedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::ImportComplete,
        format!("Import complete: {}", data.title.title_name),
        format!(
            "Imported {} file{} for '{}'.",
            data.imported_count,
            if data.imported_count == 1 { "" } else { "s" },
            data.title.title_name
        ),
        Some(&data.title),
        &data.episode_ids,
        &data.media_updates,
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        client_name: data.source_system.clone(),
        ..Default::default()
    });
    payload.import = Some(NotificationImportPayload {
        import_id: data.import_id.clone(),
        source_system: data.source_system.clone(),
        source_ref: data.source_ref.clone(),
        source_title: data.source_title.clone(),
        source_path: data.source_path.clone(),
        dest_path: data.dest_path.clone(),
        imported_count: Some(data.imported_count),
        status: Some("completed".to_string()),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_import_rejected_notification(data: &ImportRejectedEventData) -> BuiltNotification {
    let title = data
        .title
        .as_ref()
        .map(|title| title.title_name.as_str())
        .unwrap_or("Unknown title");
    let mut payload = base_notification_payload(
        NotificationEventType::ImportRejected,
        format!("Import rejected: {title}"),
        data.reason
            .clone()
            .unwrap_or_else(|| "Import was rejected.".to_string()),
        data.title.as_ref(),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.import = Some(NotificationImportPayload {
        import_id: data.import_id.clone(),
        source_system: data.source_system.clone(),
        source_ref: data.source_ref.clone(),
        source_title: data.source_title.clone(),
        source_path: data.source_path.clone(),
        dest_path: data.dest_path.clone(),
        status: Some(data.status.as_str().to_string()),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_media_file_upgraded_notification(data: &MediaFileUpgradedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::Upgrade,
            format!("Upgraded: {}", data.title.title_name),
            format!("Upgraded file for '{}'.", data.title.title_name),
            Some(&data.title),
            &[],
            &data.media_updates,
        ),
    }
}

fn build_media_file_renamed_notification(data: &MediaFileRenamedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::Rename,
            format!("Renamed: {}", data.title.title_name),
            format!(
                "Renamed {} file(s) for '{}'.",
                data.renamed_count, data.title.title_name
            ),
            Some(&data.title),
            &data.episode_ids,
            &data.media_updates,
        ),
    }
}

fn build_media_file_deleted_notification(
    data: &MediaFileDeletedEventData,
    event_type: NotificationEventType,
) -> BuiltNotification {
    let first_path = data
        .media_updates
        .first()
        .map(|update| update.path.as_str());
    let title = match data.reason {
        MediaFileDeletedReason::UpgradeCleanup => {
            format!("Deleted for upgrade: {}", data.title.title_name)
        }
        MediaFileDeletedReason::RecycleBinPurged => {
            format!("Recycle bin purged: {}", data.title.title_name)
        }
        MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
            format!("File deleted: {}", data.title.title_name)
        }
    };
    let body = match data.reason {
        MediaFileDeletedReason::UpgradeCleanup => format!(
            "Removed old media file during upgrade: {}",
            first_path.unwrap_or("(path unavailable)")
        ),
        MediaFileDeletedReason::RecycleBinPurged => format!(
            "Permanently deleted recycled media file: {}",
            first_path.unwrap_or("(path unavailable)")
        ),
        MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
            format!(
                "Deleted media file from disk: {}",
                first_path.unwrap_or("(path unavailable)")
            )
        }
    };

    BuiltNotification {
        payload: base_notification_payload(
            event_type,
            title,
            body,
            Some(&data.title),
            &data.episode_ids,
            &data.media_updates,
        ),
    }
}

fn build_post_processing_completed_notification(
    data: &PostProcessingCompletedEventData,
) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::PostProcessingCompleted,
        format!("Post-processing: {}", data.title.title_name),
        match data.result {
            PostProcessingResult::Succeeded => format!(
                "Post-processing '{}' succeeded for '{}'.",
                data.script_name, data.title.title_name
            ),
            PostProcessingResult::TimedOut => format!(
                "Post-processing '{}' timed out for '{}'.",
                data.script_name, data.title.title_name
            ),
            PostProcessingResult::Failed => format!(
                "Post-processing '{}' failed for '{}'.",
                data.script_name, data.title.title_name
            ),
        },
        Some(&data.title),
        &[],
        &[],
    );
    payload.import = Some(NotificationImportPayload {
        status: Some(
            match data.result {
                PostProcessingResult::Succeeded => "succeeded",
                PostProcessingResult::TimedOut => "timed_out",
                PostProcessingResult::Failed => "failed",
            }
            .to_string(),
        ),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_subtitle_downloaded_notification(data: &SubtitleDownloadedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::SubtitleDownloaded,
        format!("Subtitle downloaded: {}", data.title.title_name),
        data.language.as_deref().map_or_else(
            || format!("Downloaded subtitle for '{}'.", data.title.title_name),
            |language| {
                format!(
                    "Downloaded {language} subtitle for '{}'.",
                    data.title.title_name
                )
            },
        ),
        Some(&data.title),
        &[],
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        provider: data.provider.clone(),
        language: data.language.clone(),
        ..Default::default()
    });
    payload.file = Some(NotificationFilePayload {
        primary_path: data.subtitle_path.clone(),
        media_updates: Vec::new(),
    });
    BuiltNotification { payload }
}

fn build_subtitle_search_failed_notification(
    data: &SubtitleSearchFailedEventData,
) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::SubtitleSearchFailed,
        format!("Subtitle search failed: {}", data.title.title_name),
        data.reason
            .clone()
            .unwrap_or_else(|| format!("Subtitle search failed for '{}'.", data.title.title_name)),
        Some(&data.title),
        &[],
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        language: data.language.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_media_request_submitted_notification(
    data: &MediaRequestSubmittedEventData,
) -> BuiltNotification {
    let title = media_request_submitted_title_context(data);
    let mut payload = base_notification_payload(
        NotificationEventType::MediaRequestSubmitted,
        format!("Media request submitted: {}", data.title_name),
        format!("Submitted media request for '{}'.", data.title_name),
        Some(&title),
        &[],
        &[],
    );
    payload.media_request = Some(NotificationMediaRequestPayload {
        request_id: Some(data.request_id.clone()),
        library_id: Some(data.library_id.clone()),
        status: Some("pending".to_string()),
        facet: Some(data.facet.as_str().to_string()),
        requested_quality_profile_id: data.requested_quality_profile_id.clone(),
        requested_quality_profile_name: data.requested_quality_profile_name.clone(),
        requested_monitor_type: data.requested_monitor_type.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_media_request_resolved_notification(
    data: &MediaRequestResolvedEventData,
    event_type: NotificationEventType,
) -> BuiltNotification {
    let title = media_request_resolved_title_context(data);
    let (status, verb) = match event_type {
        NotificationEventType::MediaRequestApproved => ("approved", "Approved"),
        NotificationEventType::MediaRequestRejected => ("rejected", "Rejected"),
        NotificationEventType::MediaRequestCanceled => ("canceled", "Canceled"),
        _ => ("resolved", "Resolved"),
    };
    let mut payload = base_notification_payload(
        event_type,
        format!("Media request {status}: {}", data.title_name),
        format!("{verb} media request for '{}'.", data.title_name),
        Some(&title),
        &[],
        &[],
    );
    payload.media_request = Some(NotificationMediaRequestPayload {
        request_id: Some(data.request_id.clone()),
        library_id: Some(data.library_id.clone()),
        status: Some(status.to_string()),
        facet: Some(data.facet.as_str().to_string()),
        requested_quality_profile_id: data.requested_quality_profile_id.clone(),
        requested_quality_profile_name: data.requested_quality_profile_name.clone(),
        requested_monitor_type: data.requested_monitor_type.clone(),
        approved_quality_profile_id: data.approved_quality_profile_id.clone(),
        approved_quality_profile_name: data.approved_quality_profile_name.clone(),
        created_title_id: data.created_title_id.clone(),
    });
    BuiltNotification { payload }
}

fn base_notification_payload(
    event_type: NotificationEventType,
    summary_title: String,
    summary_message: String,
    title: Option<&TitleContextSnapshot>,
    episode_ids: &[String],
    updates: &[MediaPathUpdate],
) -> NotificationPayload {
    NotificationPayload {
        schema_version: NOTIFICATION_REQUEST_SCHEMA_VERSION,
        event_type,
        event_id: None,
        occurred_at: None,
        correlation_id: None,
        actor: None,
        severity: None,
        is_test: matches!(event_type, NotificationEventType::Test),
        summary_title,
        summary_message,
        app: NotificationAppPayload {
            name: "Scryer".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        title: title.map(title_payload_from_context),
        episode: episode_payload(episode_ids),
        episodes: Vec::new(),
        release: None,
        download: None,
        import: None,
        health: None,
        file: file_payload(updates),
        media_files: Vec::new(),
        application_update: None,
        manual_interaction: None,
        media_request: None,
    }
}

fn title_payload_from_context(title: &TitleContextSnapshot) -> NotificationTitlePayload {
    NotificationTitlePayload {
        id: None,
        name: title.title_name.clone(),
        facet: title.facet.as_str().to_string(),
        year: title.year,
        slug: None,
        path: None,
        overview: None,
        sort_title: None,
        poster_url: title.poster_url.clone(),
        background_url: None,
        tags: Vec::new(),
        aliases: Vec::new(),
        original_language: None,
        original_country: None,
        external_ids: NotificationExternalIdsPayload {
            tmdb_id: title.external_ids.tmdb_id.clone(),
            imdb_id: title.external_ids.imdb_id.clone(),
            tvdb_id: title.external_ids.tvdb_id.clone(),
            anidb_id: title.external_ids.anidb_id.clone(),
            tvmaze_id: None,
            anilist_ids: Vec::new(),
            mal_ids: Vec::new(),
            kitsu_ids: Vec::new(),
            by_source: external_ids_by_source_from_snapshot(title),
        },
    }
}

fn media_request_submitted_title_context(
    data: &MediaRequestSubmittedEventData,
) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: data.title_name.clone(),
        facet: data.facet.clone(),
        external_ids: media_request_external_ids(&data.external_ids),
        poster_url: data.poster_url.clone(),
        year: data.year,
    }
}

fn media_request_resolved_title_context(
    data: &MediaRequestResolvedEventData,
) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: data.title_name.clone(),
        facet: data.facet.clone(),
        external_ids: media_request_external_ids(&data.external_ids),
        poster_url: None,
        year: None,
    }
}

fn media_request_external_ids(external_ids: &[ExternalId]) -> DomainExternalIds {
    let mut out = DomainExternalIds::default();
    for external_id in external_ids {
        match external_id.source.as_str() {
            "imdb" => out.imdb_id = Some(external_id.value.clone()),
            "tmdb" => out.tmdb_id = Some(external_id.value.clone()),
            "tvdb" => out.tvdb_id = Some(external_id.value.clone()),
            "anidb" => out.anidb_id = Some(external_id.value.clone()),
            _ => {}
        }
    }
    out
}

fn episode_payload(episode_ids: &[String]) -> Option<NotificationEpisodePayload> {
    (!episode_ids.is_empty()).then(|| NotificationEpisodePayload {
        episode_ids: episode_ids.to_vec(),
        display: None,
        ..Default::default()
    })
}

fn file_payload(updates: &[MediaPathUpdate]) -> Option<NotificationFilePayload> {
    if updates.is_empty() {
        return None;
    }

    Some(NotificationFilePayload {
        primary_path: updates.first().map(|update| update.path.clone()),
        media_updates: updates
            .iter()
            .map(|update| NotificationMediaUpdatePayload {
                path: update.path.clone(),
                update_type: match update.update_type {
                    MediaUpdateType::Created => NotificationMediaUpdateTypePayload::Created,
                    MediaUpdateType::Modified => NotificationMediaUpdateTypePayload::Modified,
                    MediaUpdateType::Deleted => NotificationMediaUpdateTypePayload::Deleted,
                },
            })
            .collect(),
    })
}

async fn enrich_notification(
    app: &AppUseCase,
    event: &DomainEvent,
    mut notification: BuiltNotification,
) -> BuiltNotification {
    notification.payload.schema_version = NOTIFICATION_REQUEST_SCHEMA_VERSION;
    notification.payload.event_id = Some(event.event_id.clone());
    notification.payload.occurred_at = Some(event.occurred_at.to_rfc3339());
    notification.payload.correlation_id = event.correlation_id.clone();
    notification.payload.actor =
        event
            .actor_user_id
            .as_ref()
            .map(|user_id| NotificationActorPayload {
                user_id: Some(user_id.clone()),
            });
    notification.payload.severity = Some(notification_severity(notification.payload.event_type));
    notification.payload.is_test =
        matches!(notification.payload.event_type, NotificationEventType::Test);

    notification.payload.title =
        resolve_notification_title(app, event, notification.payload.title.take()).await;
    notification.payload.episodes =
        resolve_notification_episodes(app, notification.payload.episode.as_ref()).await;
    if let Some(summary) = notification.payload.episode.as_mut()
        && summary.display.is_none()
    {
        summary.display = notification
            .payload
            .episodes
            .first()
            .and_then(|episode| episode.display.clone());
    }
    notification.payload.media_files =
        resolve_notification_media_files(app, event, notification.payload.file.as_ref()).await;
    enrich_episode_media_file_associations(app, event, &mut notification.payload).await;
    enrich_release_from_media_files(&mut notification.payload);

    notification
}

fn notification_severity(event_type: NotificationEventType) -> NotificationSeverityPayload {
    match event_type {
        NotificationEventType::Download
        | NotificationEventType::ImportRejected
        | NotificationEventType::SubtitleSearchFailed => NotificationSeverityPayload::Error,
        NotificationEventType::HealthIssue => NotificationSeverityPayload::Warning,
        _ => NotificationSeverityPayload::Info,
    }
}

async fn resolve_notification_title(
    app: &AppUseCase,
    event: &DomainEvent,
    mut fallback: Option<NotificationTitlePayload>,
) -> Option<NotificationTitlePayload> {
    let Some(title_id) = notification_scope_title_id(event) else {
        return fallback;
    };

    if let Some(fallback) = fallback.as_mut()
        && fallback.id.is_none()
    {
        fallback.id = Some(title_id.to_string());
    }

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => Some(title_payload_from_title(&title)),
        Ok(None) => fallback,
        Err(error) => {
            warn!(title_id, error = %error, "failed to load notification title metadata");
            fallback
        }
    }
}

async fn resolve_notification_episodes(
    app: &AppUseCase,
    summary: Option<&NotificationEpisodePayload>,
) -> Vec<NotificationEpisodePayload> {
    let Some(summary) = summary else {
        return Vec::new();
    };
    if summary.episode_ids.is_empty() {
        return Vec::new();
    }

    let mut episodes = Vec::new();
    for episode_id in &summary.episode_ids {
        match app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await
        {
            Ok(Some(episode)) => episodes.push(episode_payload_from_episode(&episode)),
            Ok(None) => {}
            Err(error) => warn!(
                episode_id,
                error = %error,
                "failed to load notification episode metadata"
            ),
        }
    }
    episodes
}

async fn resolve_notification_media_files(
    app: &AppUseCase,
    event: &DomainEvent,
    file_summary: Option<&NotificationFilePayload>,
) -> Vec<NotificationMediaFilePayload> {
    let mut media_files = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for file_id in notification_file_ids(event) {
        match app
            .services
            .library
            .media_files
            .get_media_file_by_id(&file_id)
            .await
        {
            Ok(Some(media_file)) => {
                if seen_paths.insert(media_file.file_path.clone()) {
                    media_files.push(media_file_payload_from_record(&media_file));
                }
            }
            Ok(None) => {}
            Err(error) => warn!(
                file_id,
                error = %error,
                "failed to load notification media file by id"
            ),
        }
    }

    for path in notification_media_update_paths(file_summary) {
        match app
            .services
            .library
            .media_files
            .get_media_file_by_path(&path)
            .await
        {
            Ok(Some(media_file)) => {
                if seen_paths.insert(media_file.file_path.clone()) {
                    media_files.push(media_file_payload_from_record(&media_file));
                }
            }
            Ok(None) => {
                if seen_paths.insert(path.clone()) {
                    media_files.push(NotificationMediaFilePayload {
                        path,
                        ..NotificationMediaFilePayload::default()
                    });
                }
            }
            Err(error) => warn!(
                path,
                error = %error,
                "failed to load notification media file by path"
            ),
        }
    }

    media_files
}

async fn enrich_episode_media_file_associations(
    app: &AppUseCase,
    event: &DomainEvent,
    payload: &mut NotificationPayload,
) {
    let Some(title_id) = event.title_id.as_deref() else {
        return;
    };

    let mut episode_ids = BTreeSet::new();
    if let Some(summary) = payload.episode.as_ref() {
        episode_ids.extend(summary.episode_ids.iter().cloned());
    }
    for episode in &payload.episodes {
        if let Some(id) = episode.id.as_ref() {
            episode_ids.insert(id.clone());
        }
        episode_ids.extend(episode.episode_ids.iter().cloned());
    }
    if episode_ids.is_empty() {
        return;
    }

    let episode_id_list = episode_ids.iter().cloned().collect::<Vec<_>>();
    let scoped_files = match app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(title_id, &episode_id_list)
        .await
    {
        Ok(scoped_files) => scoped_files,
        Err(error) => {
            warn!(
                title_id,
                error = %error,
                "failed to load notification media file episode associations"
            );
            return;
        }
    };

    let mut associations: BTreeMap<String, Option<(String, String)>> = BTreeMap::new();
    for scoped_file in scoped_files {
        let media_file_id = scoped_file.media_file.id.clone();
        let media_file_path = scoped_file.media_file.file_path.clone();
        for episode_id in scoped_file.episode_ids {
            if !episode_ids.contains(&episode_id) {
                continue;
            }

            match associations.get_mut(&episode_id) {
                Some(Some((existing_file_id, _))) if existing_file_id != &media_file_id => {
                    associations.insert(episode_id, None);
                }
                Some(_) => {}
                None => {
                    associations.insert(
                        episode_id,
                        Some((media_file_id.clone(), media_file_path.clone())),
                    );
                }
            }
        }
    }

    if let Some(summary) = payload.episode.as_mut() {
        apply_episode_media_file_association(summary, &associations);
    }
    for episode in &mut payload.episodes {
        apply_episode_media_file_association(episode, &associations);
    }
}

fn apply_episode_media_file_association(
    episode: &mut NotificationEpisodePayload,
    associations: &BTreeMap<String, Option<(String, String)>>,
) {
    let Some((media_file_id, media_file_path)) =
        notification_episode_media_file_association(episode, associations)
    else {
        return;
    };

    episode.media_file_id = Some(media_file_id.clone());
    episode.media_file_path = Some(media_file_path.clone());
}

fn notification_episode_media_file_association<'a>(
    episode: &NotificationEpisodePayload,
    associations: &'a BTreeMap<String, Option<(String, String)>>,
) -> Option<&'a (String, String)> {
    if let Some(id) = episode.id.as_deref() {
        match associations.get(id) {
            Some(Some(association)) => return Some(association),
            Some(None) => return None,
            None => {}
        }
    }

    let mut selected = None;
    for episode_id in &episode.episode_ids {
        match associations.get(episode_id) {
            Some(Some(association)) => {
                if selected.is_some_and(|existing| existing != association) {
                    return None;
                }
                selected = Some(association);
            }
            Some(None) => return None,
            None => {}
        }
    }
    selected
}

#[cfg(test)]
mod media_file_association_tests {
    use super::*;

    fn association(file_id: &str, path: &str) -> Option<(String, String)> {
        Some((file_id.to_string(), path.to_string()))
    }

    #[test]
    fn episode_media_file_association_uses_exact_episode_id() {
        let mut associations = BTreeMap::new();
        associations.insert(
            "episode-1".to_string(),
            association("file-1", "/show/e1.mkv"),
        );
        let mut episode = NotificationEpisodePayload {
            id: Some("episode-1".to_string()),
            episode_ids: vec!["episode-1".to_string()],
            ..NotificationEpisodePayload::default()
        };

        apply_episode_media_file_association(&mut episode, &associations);

        assert_eq!(episode.media_file_id.as_deref(), Some("file-1"));
        assert_eq!(episode.media_file_path.as_deref(), Some("/show/e1.mkv"));
    }

    #[test]
    fn episode_media_file_association_allows_multi_episode_same_file() {
        let mut associations = BTreeMap::new();
        associations.insert(
            "episode-1".to_string(),
            association("file-1", "/show/e1e2.mkv"),
        );
        associations.insert(
            "episode-2".to_string(),
            association("file-1", "/show/e1e2.mkv"),
        );
        let mut episode = NotificationEpisodePayload {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
            ..NotificationEpisodePayload::default()
        };

        apply_episode_media_file_association(&mut episode, &associations);

        assert_eq!(episode.media_file_id.as_deref(), Some("file-1"));
        assert_eq!(episode.media_file_path.as_deref(), Some("/show/e1e2.mkv"));
    }

    #[test]
    fn episode_media_file_association_rejects_ambiguous_files() {
        let mut associations = BTreeMap::new();
        associations.insert(
            "episode-1".to_string(),
            association("file-1", "/show/e1.mkv"),
        );
        associations.insert(
            "episode-2".to_string(),
            association("file-2", "/show/e2.mkv"),
        );
        let mut episode = NotificationEpisodePayload {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
            ..NotificationEpisodePayload::default()
        };

        apply_episode_media_file_association(&mut episode, &associations);

        assert_eq!(episode.media_file_id, None);
        assert_eq!(episode.media_file_path, None);
    }
}

fn enrich_release_from_media_files(payload: &mut NotificationPayload) {
    let Some(release) = payload.release.as_mut() else {
        return;
    };
    let Some(first_media_file) = payload.media_files.first() else {
        return;
    };

    if release.quality.is_none() {
        release.quality = first_media_file.quality.clone();
    }
    if release.release_group.is_none() {
        release.release_group = first_media_file.release_group.clone();
    }
    if release.languages.is_empty() {
        release.languages = first_media_file.audio_languages.clone();
    }
}

fn title_payload_from_title(title: &Title) -> NotificationTitlePayload {
    NotificationTitlePayload {
        id: Some(title.id.clone()),
        name: title.name.clone(),
        facet: title.facet.as_str().to_string(),
        year: title.year,
        slug: title.slug.clone(),
        path: title.folder_path.clone(),
        overview: title.overview.clone(),
        sort_title: title.sort_title.clone(),
        poster_url: title.poster_url.clone(),
        background_url: title.background_url.clone(),
        tags: title.tags.clone(),
        aliases: title_aliases(title),
        original_language: title.language.clone(),
        original_country: title.country.clone(),
        external_ids: external_ids_payload_from_title(title),
    }
}

fn title_aliases(title: &Title) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut aliases = Vec::new();
    for alias in &title.aliases {
        if seen.insert(alias.clone()) {
            aliases.push(alias.clone());
        }
    }
    for alias in &title.tagged_aliases {
        if seen.insert(alias.name.clone()) {
            aliases.push(alias.name.clone());
        }
    }
    aliases
}

fn external_ids_payload_from_title(title: &Title) -> NotificationExternalIdsPayload {
    let mut payload = NotificationExternalIdsPayload::default();
    if let Some(imdb_id) = &title.imdb_id {
        payload.imdb_id = Some(imdb_id.clone());
        payload
            .by_source
            .entry("imdb".to_string())
            .or_default()
            .push(imdb_id.clone());
    }

    for external_id in &title.external_ids {
        push_external_id(&mut payload, external_id);
    }

    payload
}

fn push_external_id(payload: &mut NotificationExternalIdsPayload, external_id: &ExternalId) {
    let source = external_id.source.trim().to_ascii_lowercase();
    if source == "smg" {
        // SMG identifies Scryer's metadata source; third-party provider payloads must not expose it.
        return;
    }
    payload
        .by_source
        .entry(source.clone())
        .or_default()
        .push(external_id.value.clone());

    match source.as_str() {
        "tmdb" if payload.tmdb_id.is_none() => payload.tmdb_id = Some(external_id.value.clone()),
        "imdb" if payload.imdb_id.is_none() => payload.imdb_id = Some(external_id.value.clone()),
        "tvdb" if payload.tvdb_id.is_none() => payload.tvdb_id = Some(external_id.value.clone()),
        "anidb" if payload.anidb_id.is_none() => payload.anidb_id = Some(external_id.value.clone()),
        "tvmaze" if payload.tvmaze_id.is_none() => {
            payload.tvmaze_id = Some(external_id.value.clone())
        }
        "anilist" => payload.anilist_ids.push(external_id.value.clone()),
        "mal" => payload.mal_ids.push(external_id.value.clone()),
        "kitsu" => payload.kitsu_ids.push(external_id.value.clone()),
        _ => {}
    }
}

fn external_ids_by_source_from_snapshot(
    title: &TitleContextSnapshot,
) -> BTreeMap<String, Vec<String>> {
    let mut by_source = BTreeMap::new();
    if let Some(tmdb_id) = &title.external_ids.tmdb_id {
        by_source.insert("tmdb".to_string(), vec![tmdb_id.clone()]);
    }
    if let Some(imdb_id) = &title.external_ids.imdb_id {
        by_source.insert("imdb".to_string(), vec![imdb_id.clone()]);
    }
    if let Some(tvdb_id) = &title.external_ids.tvdb_id {
        by_source.insert("tvdb".to_string(), vec![tvdb_id.clone()]);
    }
    if let Some(anidb_id) = &title.external_ids.anidb_id {
        by_source.insert("anidb".to_string(), vec![anidb_id.clone()]);
    }
    by_source
}

fn episode_payload_from_episode(episode: &Episode) -> NotificationEpisodePayload {
    NotificationEpisodePayload {
        id: Some(episode.id.clone()),
        episode_ids: vec![episode.id.clone()],
        media_file_id: None,
        media_file_path: None,
        display: episode_display(episode),
        collection_id: episode.collection_id.clone(),
        season_number: episode.season_number.clone(),
        episode_number: episode.episode_number.clone(),
        absolute_number: episode.absolute_number.clone(),
        title: episode.title.clone(),
        overview: episode.overview.clone(),
        air_date: episode.air_date.clone(),
        air_date_utc: None,
        episode_type: Some(episode.episode_type.as_str().to_string()),
        finale_type: None,
        tvdb_id: episode.tvdb_id.clone(),
    }
}

fn episode_display(episode: &Episode) -> Option<String> {
    match (&episode.season_number, &episode.episode_number) {
        (Some(season_number), Some(episode_number)) => Some(format!(
            "S{}E{}",
            padded_number(season_number),
            padded_number(episode_number)
        )),
        _ => episode
            .absolute_number
            .as_ref()
            .map(|absolute_number| format!("#{absolute_number}")),
    }
}

fn padded_number(value: &str) -> String {
    value
        .parse::<u32>()
        .map(|parsed| format!("{parsed:02}"))
        .unwrap_or_else(|_| value.to_string())
}

fn notification_file_ids(event: &DomainEvent) -> Vec<String> {
    match &event.payload {
        DomainEventPayload::MediaFileDeleted(data) => {
            data.file_id.iter().cloned().collect::<Vec<_>>()
        }
        DomainEventPayload::MediaFileUpgraded(data) => {
            let mut file_ids = Vec::new();
            if let Some(previous_file_id) = &data.previous_file_id {
                file_ids.push(previous_file_id.clone());
            }
            if let Some(current_file_id) = &data.current_file_id {
                file_ids.push(current_file_id.clone());
            }
            file_ids
        }
        _ => Vec::new(),
    }
}

fn notification_media_update_paths(file_summary: Option<&NotificationFilePayload>) -> Vec<String> {
    let Some(file_summary) = file_summary else {
        return Vec::new();
    };

    file_summary
        .media_updates
        .iter()
        .map(|update| update.path.clone())
        .collect()
}

fn media_file_payload_from_record(
    media_file: &crate::types::TitleMediaFile,
) -> NotificationMediaFilePayload {
    NotificationMediaFilePayload {
        id: Some(media_file.id.clone()),
        path: media_file.file_path.clone(),
        previous_path: media_file.original_file_path.clone(),
        recycle_bin_path: None,
        size_bytes: Some(media_file.size_bytes),
        quality: crate::media::release_labels::quality_from_video_dimensions(
            media_file.video_width,
            media_file.video_height,
        )
        .map(str::to_string)
        .or_else(|| media_file.quality_label.clone())
        .or_else(|| media_file.resolution.clone()),
        release_group: media_file.release_group.clone(),
        scene_name: media_file.scene_name.clone(),
        audio_languages: media_file.audio_languages.clone(),
        subtitle_languages: media_file.subtitle_languages.clone(),
        video_codec: media_file
            .video_codec
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| {
                media_file
                    .video_codec_parsed
                    .as_ref()
                    .map(ToString::to_string)
            }),
        audio_codec: media_file
            .audio_codec
            .clone()
            .or_else(|| media_file.audio_codec_parsed.clone()),
        audio_channels: media_file
            .audio_channels
            .map(|channels| channels.to_string())
            .or_else(|| media_file.audio_channels_parsed.clone()),
        video_width: media_file.video_width,
        video_height: media_file.video_height,
        video_bit_depth: media_file.video_bit_depth,
        video_hdr_format: media_file.video_hdr_format.clone(),
        video_frame_rate: media_file.video_frame_rate.clone(),
        container_format: media_file.container_format.clone(),
        edition: media_file.edition.clone(),
    }
}

fn subscription_event_types(event_type: NotificationEventType) -> Vec<NotificationEventType> {
    match event_type {
        NotificationEventType::FileDeletedForUpgrade => vec![
            NotificationEventType::FileDeletedForUpgrade,
            NotificationEventType::FileDeleted,
        ],
        _ => vec![event_type],
    }
}

fn notification_scope_title_id(event: &DomainEvent) -> Option<&str> {
    event.title_id.as_deref().or(match &event.payload {
        DomainEventPayload::MediaRequestApproved(data) => data.created_title_id.as_deref(),
        _ => None,
    })
}

fn notification_scope_facet(event: &DomainEvent) -> Option<&str> {
    event
        .facet
        .as_ref()
        .map(|facet| facet.as_str())
        .or_else(|| match &event.payload {
            DomainEventPayload::MediaRequestSubmitted(data) => Some(data.facet.as_str()),
            DomainEventPayload::MediaRequestApproved(data)
            | DomainEventPayload::MediaRequestRejected(data)
            | DomainEventPayload::MediaRequestCanceled(data) => Some(data.facet.as_str()),
            _ => None,
        })
}

fn matches_scope(
    scope: &str,
    scope_id: Option<&str>,
    event_title_id: Option<&str>,
    event_facet: Option<&str>,
) -> bool {
    match scope {
        "global" => true,
        "facet" => match (scope_id, event_facet) {
            (Some(scope_id), Some(facet)) => scope_id
                .split(',')
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .any(|candidate| candidate.eq_ignore_ascii_case(facet)),
            _ => false,
        },
        "title" => match (scope_id, event_title_id) {
            (Some(scope_id), Some(title_id)) => scope_id == title_id,
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_events::new_global_domain_event;
    use crate::lib_tests::bootstrap;
    use chrono::Utc;
    use scryer_domain::{
        DomainEventActorKind, DomainExternalIds, DownloadFailedEventData, ImportCompletedEventData,
        ImportRejectedEventData, ImportStatus, LibraryScanProgressedEventData, MediaFacet,
        MediaFileDeletedEventData, MediaFileRenamedEventData, MediaFileUpgradedEventData,
        MediaUpdateType, PostProcessingCompletedEventData, ReleaseGrabbedEventData,
        SubtitleDownloadedEventData, SubtitleSearchFailedEventData, TitleAddedEventData,
        TitleDeletedEventData,
    };

    fn title_context(name: &str, facet: MediaFacet) -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: name.to_string(),
            facet,
            external_ids: DomainExternalIds {
                imdb_id: Some("tt1234567".to_string()),
                tmdb_id: Some("987".to_string()),
                tvdb_id: Some("123".to_string()),
                anidb_id: None,
            },
            poster_url: Some("https://example.invalid/poster.jpg".to_string()),
            year: Some(2024),
        }
    }

    fn media_request_test_external_ids() -> Vec<ExternalId> {
        vec![
            ExternalId {
                source: "imdb".to_string(),
                value: "tt7654321".to_string(),
            },
            ExternalId {
                source: "tvdb".to_string(),
                value: "456".to_string(),
            },
        ]
    }

    #[test]
    fn supported_notification_event_types_include_media_request_lifecycle() {
        let supported = supported_notification_event_types();
        assert!(supported.contains(&NotificationEventType::MediaRequestSubmitted));
        assert!(supported.contains(&NotificationEventType::MediaRequestApproved));
        assert!(supported.contains(&NotificationEventType::MediaRequestRejected));
        assert!(supported.contains(&NotificationEventType::MediaRequestCanceled));
    }

    #[test]
    fn matches_scope_accepts_any_selected_facet_in_csv_scope_id() {
        assert!(matches_scope(
            "facet",
            Some("movie, series"),
            None,
            Some("movie")
        ));
        assert!(matches_scope(
            "facet",
            Some("movie, series"),
            None,
            Some("series")
        ));
        assert!(!matches_scope(
            "facet",
            Some("movie, series"),
            None,
            Some("anime")
        ));
    }

    fn notification_sample_events() -> Vec<DomainEvent> {
        vec![
            DomainEvent {
                sequence: 1,
                event_id: "evt-title-added".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::TitleAdded(TitleAddedEventData {
                    title: title_context("Example Movie", MediaFacet::Movie),
                }),
            },
            DomainEvent {
                sequence: 2,
                event_id: "evt-title-deleted".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                    title: title_context("Deleted Movie", MediaFacet::Movie),
                }),
            },
            DomainEvent {
                sequence: 3,
                event_id: "evt-release-grabbed".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                    title: title_context("Example Show", MediaFacet::Series),
                    source_title: Some("Example.Show.S01E01.1080p".to_string()),
                    source_hint: Some("rss".to_string()),
                    source_provider: Some("rss".to_string()),
                    download_id: Some("grab-1".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                }),
            },
            DomainEvent {
                sequence: 4,
                event_id: "evt-download-failed".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                    title: Some(title_context("Broken Download", MediaFacet::Movie)),
                    source_title: Some("Broken.Download.2024".to_string()),
                    source_hint: Some("manual".to_string()),
                    download_id: None,
                    client_id: None,
                    client_name: None,
                    client_type: None,
                    quality: None,
                    reason: Some("archive corrupt".to_string()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                }),
            },
            DomainEvent {
                sequence: 5,
                event_id: "evt-import-completed".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_context("Imported Show", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Imported Show/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-1".to_string()),
                    source_title: Some("Imported.Show.S01E01.1080p".to_string()),
                    source_path: Some("/downloads/Imported.Show.S01E01.1080p.mkv".to_string()),
                    dest_path: Some("/library/Imported Show/S01E01.mkv".to_string()),
                    quality: Some("1080p".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                    size_bytes: Some(3_221_225_472),
                }),
            },
            DomainEvent {
                sequence: 6,
                event_id: "evt-import-rejected".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: Some(title_context("Rejected Movie", MediaFacet::Movie)),
                    status: ImportStatus::Failed,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-2".to_string()),
                    source_title: Some("Rejected.Movie.1080p".to_string()),
                    source_path: Some("/downloads/rejected.mkv".to_string()),
                    dest_path: None,
                    quality: Some("1080p".to_string()),
                    reason: Some("not parsable".to_string()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            },
            DomainEvent {
                sequence: 7,
                event_id: "evt-media-upgraded".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: title_context("Upgraded Movie", MediaFacet::Movie),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Upgraded Movie/Upgraded Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Modified,
                    }],
                    episode_ids: Vec::new(),
                    previous_file_id: Some("file-old".to_string()),
                    current_file_id: Some("file-new".to_string()),
                    old_score: Some(10),
                    new_score: Some(15),
                    size_bytes: Some(8_589_934_592),
                }),
            },
            DomainEvent {
                sequence: 8,
                event_id: "evt-media-renamed".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                    title: title_context("Renamed Show", MediaFacet::Series),
                    media_updates: vec![
                        MediaPathUpdate {
                            path: "/library/Renamed Show/Old.mkv".to_string(),
                            update_type: MediaUpdateType::Deleted,
                        },
                        MediaPathUpdate {
                            path: "/library/Renamed Show/New.mkv".to_string(),
                            update_type: MediaUpdateType::Created,
                        },
                    ],
                    renamed_count: 1,
                    episode_ids: vec!["episode-1".to_string()],
                }),
            },
            DomainEvent {
                sequence: 9,
                event_id: "evt-media-deleted".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context("Deleted Movie", MediaFacet::Movie),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Deleted Movie/Deleted Movie.old.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-old".to_string()),
                    reason: MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: Vec::new(),
                }),
            },
            DomainEvent {
                sequence: 10,
                event_id: "evt-post-processing".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::PostProcessingCompleted(
                    PostProcessingCompletedEventData {
                        title: title_context("Post Processed Movie", MediaFacet::Movie),
                        script_name: "notify.sh".to_string(),
                        result: PostProcessingResult::Succeeded,
                        exit_code: Some(0),
                    },
                ),
            },
            DomainEvent {
                sequence: 11,
                event_id: "evt-subtitle-downloaded".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::SubtitleDownloaded(SubtitleDownloadedEventData {
                    title: title_context("Subtitle Show", MediaFacet::Series),
                    subtitle_path: Some("/library/Subtitle Show/S01E01.en.srt".to_string()),
                    language: Some("English".to_string()),
                    provider: Some("opensubtitles".to_string()),
                }),
            },
            DomainEvent {
                sequence: 12,
                event_id: "evt-subtitle-search-failed".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::System,
                actor_user_id: None,
                actor_display_name: "System".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::SubtitleSearchFailed(SubtitleSearchFailedEventData {
                    title: title_context("Subtitle Failure", MediaFacet::Series),
                    language: Some("English".to_string()),
                    reason: Some("provider timeout".to_string()),
                }),
            },
            DomainEvent {
                sequence: 13,
                event_id: "evt-media-request-submitted".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some("requester-1".to_string()),
                actor_display_name: "requester-1".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaRequestSubmitted(
                    MediaRequestSubmittedEventData {
                        request_id: "request-1".to_string(),
                        library_id: "library-series".to_string(),
                        facet: MediaFacet::Series,
                        title_name: "Requested Show".to_string(),
                        external_ids: media_request_test_external_ids(),
                        poster_url: Some("https://example.invalid/request.jpg".to_string()),
                        year: Some(2025),
                        requested_quality_profile_id: Some("quality-1".to_string()),
                        requested_quality_profile_name: Some("HD".to_string()),
                        requested_monitor_type: Some("missingAndFutureEpisodes".to_string()),
                    },
                ),
            },
            DomainEvent {
                sequence: 14,
                event_id: "evt-media-request-approved".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some("admin-1".to_string()),
                actor_display_name: "admin-1".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaRequestApproved(MediaRequestResolvedEventData {
                    request_id: "request-1".to_string(),
                    library_id: "library-series".to_string(),
                    facet: MediaFacet::Series,
                    title_name: "Requested Show".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: Some("title-requested-show".to_string()),
                    requested_quality_profile_id: Some("quality-1".to_string()),
                    requested_quality_profile_name: Some("HD".to_string()),
                    requested_monitor_type: Some("missingAndFutureEpisodes".to_string()),
                    approved_quality_profile_id: Some("quality-2".to_string()),
                    approved_quality_profile_name: Some("HD Approved".to_string()),
                }),
            },
            DomainEvent {
                sequence: 15,
                event_id: "evt-media-request-rejected".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some("admin-1".to_string()),
                actor_display_name: "admin-1".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaRequestRejected(MediaRequestResolvedEventData {
                    request_id: "request-2".to_string(),
                    library_id: "library-movie".to_string(),
                    facet: MediaFacet::Movie,
                    title_name: "Rejected Movie".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: None,
                    requested_quality_profile_id: Some("quality-1".to_string()),
                    requested_quality_profile_name: Some("HD".to_string()),
                    requested_monitor_type: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                }),
            },
            DomainEvent {
                sequence: 16,
                event_id: "evt-media-request-canceled".to_string(),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some("requester-1".to_string()),
                actor_display_name: "requester-1".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaRequestCanceled(MediaRequestResolvedEventData {
                    request_id: "request-3".to_string(),
                    library_id: "library-anime".to_string(),
                    facet: MediaFacet::Anime,
                    title_name: "Canceled Anime".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: None,
                    requested_quality_profile_id: Some("quality-1".to_string()),
                    requested_quality_profile_name: Some("HD".to_string()),
                    requested_monitor_type: Some("futureEpisodes".to_string()),
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                }),
            },
        ]
    }

    #[test]
    fn media_request_scope_context_uses_payload_when_event_envelope_is_global() {
        let events = notification_sample_events();
        let media_request_events = &events[12..16];

        assert_eq!(
            notification_scope_facet(&media_request_events[0]),
            Some("series")
        );
        assert_eq!(
            notification_scope_facet(&media_request_events[1]),
            Some("series")
        );
        assert_eq!(
            notification_scope_facet(&media_request_events[2]),
            Some("movie")
        );
        assert_eq!(
            notification_scope_facet(&media_request_events[3]),
            Some("anime")
        );
        assert_eq!(
            notification_scope_title_id(&media_request_events[1]),
            Some("title-requested-show")
        );
    }

    #[test]
    fn notification_external_ids_exclude_smg_source() {
        let mut payload = NotificationExternalIdsPayload::default();
        push_external_id(
            &mut payload,
            &ExternalId {
                source: "smg".to_string(),
                value: "101".to_string(),
            },
        );
        push_external_id(
            &mut payload,
            &ExternalId {
                source: "tmdb".to_string(),
                value: "603".to_string(),
            },
        );

        assert!(!payload.by_source.contains_key("smg"));
        assert_eq!(payload.tmdb_id.as_deref(), Some("603"));
    }

    #[tokio::test]
    async fn dispatch_pending_events_replays_only_notification_events() {
        let (app, _) = bootstrap();
        let operational = new_global_domain_event(
            None,
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "scan-1".to_string(),
                status: "running".to_string(),
                found_titles: 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: 1,
                titles_total: Some(10),
                files_completed: 1,
                files_total: Some(10),
                warning_message: None,
            }),
        );
        let notification = new_global_domain_event(
            None,
            DomainEventPayload::TitleAdded(TitleAddedEventData {
                title: title_context("Replay Fixture", MediaFacet::Movie),
            }),
        );

        app.append_domain_events(vec![operational, notification])
            .await
            .expect("events should append");

        let last_sequence = dispatch_pending_events(&app, 0)
            .await
            .expect("dispatch should replay");

        assert_eq!(last_sequence, 2);
        let offset = app
            .services
            .events
            .domain_events
            .get_subscriber_offset(NOTIFICATION_SUBSCRIBER)
            .await
            .expect("offset should load");
        assert_eq!(offset, 2);
    }

    #[test]
    fn notification_filter_list_matches_buildable_payloads() {
        let supported_events = notification_sample_events();
        let configured_event_types = NOTIFICATION_DOMAIN_EVENT_TYPES
            .iter()
            .map(|event_type| event_type.as_str().to_string())
            .collect::<std::collections::HashSet<_>>();
        let buildable_event_types = supported_events
            .iter()
            .map(|event| {
                let built = build_notification(event)
                    .expect("supported payload should build a notification");
                assert!(
                    notification_event_type(&event.payload) == Some(built.payload.event_type),
                    "notification type helper should mirror notification classification"
                );
                event.payload.event_type().as_str().to_string()
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(buildable_event_types, configured_event_types);

        let unsupported = DomainEvent {
            sequence: 99,
            event_id: "evt-scan".to_string(),
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: None,
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: scryer_domain::DomainEventStream::Global,
            payload: DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "scan-unsupported".to_string(),
                status: "running".to_string(),
                found_titles: 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: 1,
                titles_total: Some(5),
                files_completed: 1,
                files_total: Some(5),
                warning_message: None,
            }),
        };
        assert!(notification_event_type(&unsupported.payload).is_none());
        assert!(build_notification(&unsupported).is_none());
    }

    #[tokio::test]
    async fn media_request_notifications_include_typed_context() {
        let cases = [
            (
                DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
                    request_id: "request-submitted".to_string(),
                    library_id: "library-series".to_string(),
                    facet: MediaFacet::Series,
                    title_name: "Requested Show".to_string(),
                    external_ids: media_request_test_external_ids(),
                    poster_url: Some("https://example.invalid/request.jpg".to_string()),
                    year: Some(2025),
                    requested_quality_profile_id: Some("quality-requested".to_string()),
                    requested_quality_profile_name: Some("Requested HD".to_string()),
                    requested_monitor_type: Some("missingAndFutureEpisodes".to_string()),
                }),
                NotificationEventType::MediaRequestSubmitted,
                "pending",
                None,
            ),
            (
                DomainEventPayload::MediaRequestApproved(MediaRequestResolvedEventData {
                    request_id: "request-approved".to_string(),
                    library_id: "library-movie".to_string(),
                    facet: MediaFacet::Movie,
                    title_name: "Approved Movie".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: Some("title-approved".to_string()),
                    requested_quality_profile_id: Some("quality-requested".to_string()),
                    requested_quality_profile_name: Some("Requested HD".to_string()),
                    requested_monitor_type: None,
                    approved_quality_profile_id: Some("quality-approved".to_string()),
                    approved_quality_profile_name: Some("Approved HD".to_string()),
                }),
                NotificationEventType::MediaRequestApproved,
                "approved",
                Some("title-approved"),
            ),
            (
                DomainEventPayload::MediaRequestRejected(MediaRequestResolvedEventData {
                    request_id: "request-rejected".to_string(),
                    library_id: "library-movie".to_string(),
                    facet: MediaFacet::Movie,
                    title_name: "Rejected Movie".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: None,
                    requested_quality_profile_id: Some("quality-requested".to_string()),
                    requested_quality_profile_name: Some("Requested HD".to_string()),
                    requested_monitor_type: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                }),
                NotificationEventType::MediaRequestRejected,
                "rejected",
                None,
            ),
            (
                DomainEventPayload::MediaRequestCanceled(MediaRequestResolvedEventData {
                    request_id: "request-canceled".to_string(),
                    library_id: "library-anime".to_string(),
                    facet: MediaFacet::Anime,
                    title_name: "Canceled Anime".to_string(),
                    external_ids: media_request_test_external_ids(),
                    created_title_id: None,
                    requested_quality_profile_id: Some("quality-requested".to_string()),
                    requested_quality_profile_name: Some("Requested HD".to_string()),
                    requested_monitor_type: Some("futureEpisodes".to_string()),
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                }),
                NotificationEventType::MediaRequestCanceled,
                "canceled",
                None,
            ),
        ];

        for (payload, expected_event_type, expected_status, expected_created_title_id) in cases {
            let event = DomainEvent {
                sequence: 100,
                event_id: format!("evt-{}", expected_event_type.as_str()),
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some("actor-1".to_string()),
                actor_display_name: "actor-1".to_string(),
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload,
            };

            let built =
                build_notification(&event).expect("media request notification should build");
            let (app, _) = bootstrap();
            let enriched = enrich_notification(&app, &event, built).await;
            assert_eq!(enriched.payload.event_type, expected_event_type);
            assert_eq!(
                enriched
                    .payload
                    .actor
                    .as_ref()
                    .and_then(|actor| actor.user_id.as_deref()),
                Some("actor-1")
            );
            let title = enriched.payload.title.as_ref().expect("title context");
            assert_eq!(title.external_ids.imdb_id.as_deref(), Some("tt7654321"));
            let request = enriched
                .payload
                .media_request
                .as_ref()
                .expect("media request context");
            assert_eq!(request.status.as_deref(), Some(expected_status));
            assert_eq!(
                request.requested_quality_profile_name.as_deref(),
                Some("Requested HD")
            );
            assert_eq!(
                request.created_title_id.as_deref(),
                expected_created_title_id
            );
        }
    }
}
