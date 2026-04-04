use crate::AppUseCase;
use scryer_domain::{
    DomainEvent, DomainEventPayload, MediaFileDeletedReason, MediaPathUpdate,
    NotificationEventType, PostProcessingResult, TitleContextSnapshot,
};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const NOTIFICATION_SUBSCRIBER: &str = "notification_dispatcher";
const NOTIFICATION_BATCH_LIMIT: usize = 100;

pub async fn start_notification_dispatcher(app: AppUseCase, cancel: CancellationToken) {
    info!("notification dispatcher started");
    let repo = app.services.domain_events.clone();
    let mut rx = app.services.domain_event_broadcast.subscribe();
    let mut last_sequence = match repo.get_subscriber_offset(NOTIFICATION_SUBSCRIBER).await {
        Ok(sequence) => sequence,
        Err(error) => {
            warn!(error = %error, "failed to load notification subscriber offset; starting at 0");
            0
        }
    };

    loop {
        match dispatch_pending_events(&app, last_sequence).await {
            Ok(sequence) => last_sequence = sequence,
            Err(error) => {
                warn!(error = %error, "notification dispatcher failed to process pending events")
            }
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                info!("notification dispatcher shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(sequence) => {
                        if sequence > last_sequence {
                            last_sequence = sequence.saturating_sub(1);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "notification dispatcher lagged, resyncing from persisted domain events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("domain event broadcast closed, notification dispatcher exiting");
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
    let repo = app.services.domain_events.clone();

    loop {
        let events = repo
            .list_after_sequence(after_sequence, NOTIFICATION_BATCH_LIMIT)
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

async fn dispatch_event(app: &AppUseCase, event: &DomainEvent) {
    let Some(notification) = build_notification(event) else {
        return;
    };

    let sub_repo = match app.notification_subscriptions_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let ch_repo = match app.notification_channels_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let provider = match app.services.notification_provider.as_ref() {
        Some(provider) => provider,
        None => return,
    };

    let event_type = event.payload.event_type();
    debug!(
        event_type = event_type.as_str(),
        title_id = ?event.title_id,
        sequence = event.sequence,
        "dispatching domain-event-backed notification"
    );

    let mut subscriptions = Vec::new();
    for subscription_event_type in subscription_event_types(notification.event_type) {
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

    for subscription in subscriptions {
        if !subscription.is_enabled {
            continue;
        }

        if !matches_scope(
            &subscription.scope,
            subscription.scope_id.as_deref(),
            event.title_id.as_deref(),
            event.facet.as_ref().map(|facet| facet.as_str()),
        ) {
            continue;
        }

        let channel = match ch_repo.get_channel(&subscription.channel_id).await {
            Ok(Some(channel)) if channel.is_enabled => channel,
            _ => continue,
        };

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

        match client
            .send_notification(
                notification.event_type.as_str(),
                &notification.title,
                &notification.body,
                &notification.metadata,
            )
            .await
        {
            Ok(()) => {
                info!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.event_type.as_str(),
                    channel = channel.name.as_str(),
                    "notification dispatched"
                );
            }
            Err(error) => {
                warn!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.event_type.as_str(),
                    channel = channel.name.as_str(),
                    error = %error,
                    "notification dispatch failed"
                );
            }
        }
    }
}

struct BuiltNotification {
    event_type: NotificationEventType,
    title: String,
    body: String,
    metadata: HashMap<String, serde_json::Value>,
}

fn build_notification(event: &DomainEvent) -> Option<BuiltNotification> {
    match &event.payload {
        DomainEventPayload::TitleAdded(data) => Some(BuiltNotification {
            event_type: NotificationEventType::TitleAdded,
            title: format!("Added: {}", data.title.title_name),
            body: format!("Added '{}' to Scryer.", data.title.title_name),
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        DomainEventPayload::TitleDeleted(data) => Some(BuiltNotification {
            event_type: NotificationEventType::TitleDeleted,
            title: format!("Deleted: {}", data.title.title_name),
            body: format!("Deleted '{}' from Scryer.", data.title.title_name),
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        DomainEventPayload::ReleaseGrabbed(data) => Some(BuiltNotification {
            event_type: NotificationEventType::Grab,
            title: format!("Grabbed: {}", data.title.title_name),
            body: data
                .source_title
                .as_ref()
                .map(|source_title| {
                    format!(
                        "Grabbed '{}' for '{}'.",
                        source_title, data.title.title_name
                    )
                })
                .unwrap_or_else(|| format!("Grabbed a release for '{}'.", data.title.title_name)),
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        DomainEventPayload::DownloadFailed(data) => {
            let title = data
                .title
                .as_ref()
                .map(|title| title.title_name.as_str())
                .unwrap_or("Unknown title");
            Some(BuiltNotification {
                event_type: NotificationEventType::Download,
                title: format!("Download failed: {title}"),
                body: data
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "Download failed.".to_string()),
                metadata: data
                    .title
                    .as_ref()
                    .map(|title| lifecycle_metadata(title, &[]))
                    .unwrap_or_default(),
            })
        }
        DomainEventPayload::ImportCompleted(data) => Some(BuiltNotification {
            event_type: NotificationEventType::ImportComplete,
            title: format!("Import complete: {}", data.title.title_name),
            body: format!(
                "Imported {} file{} for '{}'.",
                data.imported_count,
                if data.imported_count == 1 { "" } else { "s" },
                data.title.title_name
            ),
            metadata: lifecycle_metadata(&data.title, &data.media_updates),
        }),
        DomainEventPayload::ImportRejected(data) => {
            let title = data
                .title
                .as_ref()
                .map(|title| title.title_name.as_str())
                .unwrap_or("Unknown title");
            Some(BuiltNotification {
                event_type: NotificationEventType::ImportRejected,
                title: format!("Import rejected: {title}"),
                body: data
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Import was rejected.".to_string()),
                metadata: data
                    .title
                    .as_ref()
                    .map(|title| lifecycle_metadata(title, &[]))
                    .unwrap_or_default(),
            })
        }
        DomainEventPayload::MediaFileUpgraded(data) => Some(BuiltNotification {
            event_type: NotificationEventType::Upgrade,
            title: format!("Upgraded: {}", data.title.title_name),
            body: format!("Upgraded file for '{}'.", data.title.title_name),
            metadata: lifecycle_metadata(&data.title, &data.media_updates),
        }),
        DomainEventPayload::MediaFileRenamed(data) => Some(BuiltNotification {
            event_type: NotificationEventType::Rename,
            title: format!("Renamed: {}", data.title.title_name),
            body: format!(
                "Renamed {} file(s) for '{}'.",
                data.renamed_count, data.title.title_name
            ),
            metadata: lifecycle_metadata(&data.title, &data.media_updates),
        }),
        DomainEventPayload::MediaFileDeleted(data) => {
            let event_type = match data.reason {
                MediaFileDeletedReason::UpgradeCleanup => {
                    NotificationEventType::FileDeletedForUpgrade
                }
                MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
                    NotificationEventType::FileDeleted
                }
            };
            let first_path = data
                .media_updates
                .first()
                .map(|update| update.path.as_str());
            let title = match data.reason {
                MediaFileDeletedReason::UpgradeCleanup => {
                    format!("Deleted for upgrade: {}", data.title.title_name)
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
                MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => format!(
                    "Deleted media file from disk: {}",
                    first_path.unwrap_or("(path unavailable)")
                ),
            };

            Some(BuiltNotification {
                event_type,
                title,
                body,
                metadata: lifecycle_metadata(&data.title, &data.media_updates),
            })
        }
        DomainEventPayload::PostProcessingCompleted(data) => Some(BuiltNotification {
            event_type: NotificationEventType::PostProcessingCompleted,
            title: format!("Post-processing: {}", data.title.title_name),
            body: match data.result {
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
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        DomainEventPayload::SubtitleDownloaded(data) => Some(BuiltNotification {
            event_type: NotificationEventType::SubtitleDownloaded,
            title: format!("Subtitle downloaded: {}", data.title.title_name),
            body: data.language.as_deref().map_or_else(
                || format!("Downloaded subtitle for '{}'.", data.title.title_name),
                |language| {
                    format!(
                        "Downloaded {language} subtitle for '{}'.",
                        data.title.title_name
                    )
                },
            ),
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        DomainEventPayload::SubtitleSearchFailed(data) => Some(BuiltNotification {
            event_type: NotificationEventType::SubtitleSearchFailed,
            title: format!("Subtitle search failed: {}", data.title.title_name),
            body: data.reason.clone().unwrap_or_else(|| {
                format!("Subtitle search failed for '{}'.", data.title.title_name)
            }),
            metadata: lifecycle_metadata(&data.title, &[]),
        }),
        _ => None,
    }
}

fn lifecycle_metadata(
    title: &TitleContextSnapshot,
    updates: &[MediaPathUpdate],
) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        "title_name".to_string(),
        serde_json::json!(title.title_name),
    );
    metadata.insert(
        "title_facet".to_string(),
        serde_json::json!(title.facet.as_str()),
    );
    if let Some(year) = title.year {
        metadata.insert("title_year".to_string(), serde_json::json!(year));
    }
    if let Some(poster_url) = title.poster_url.as_ref() {
        metadata.insert("poster_url".to_string(), serde_json::json!(poster_url));
    }

    let mut external_ids = serde_json::Map::new();
    if let Some(imdb_id) = title.external_ids.imdb_id.as_ref() {
        external_ids.insert("imdb_id".to_string(), serde_json::json!(imdb_id));
    }
    if let Some(tmdb_id) = title.external_ids.tmdb_id.as_ref() {
        external_ids.insert("tmdb_id".to_string(), serde_json::json!(tmdb_id));
    }
    if let Some(tvdb_id) = title.external_ids.tvdb_id.as_ref() {
        external_ids.insert("tvdb_id".to_string(), serde_json::json!(tvdb_id));
    }
    if let Some(anidb_id) = title.external_ids.anidb_id.as_ref() {
        external_ids.insert("anidb_id".to_string(), serde_json::json!(anidb_id));
    }
    if !external_ids.is_empty() {
        metadata.insert(
            "external_ids".to_string(),
            serde_json::Value::Object(external_ids),
        );
    }

    if let Some(first_path) = updates.first().map(|update| update.path.as_str()) {
        metadata.insert("file_path".to_string(), serde_json::json!(first_path));
    }

    if !updates.is_empty() {
        metadata.insert(
            "media_updates".to_string(),
            serde_json::Value::Array(
                updates
                    .iter()
                    .map(|update| {
                        serde_json::json!({
                            "path": update.path,
                            "update_type": update.update_type.as_str(),
                        })
                    })
                    .collect(),
            ),
        );
    }

    metadata
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

fn matches_scope(
    scope: &str,
    scope_id: Option<&str>,
    event_title_id: Option<&str>,
    event_facet: Option<&str>,
) -> bool {
    match scope {
        "global" => true,
        "facet" => match (scope_id, event_facet) {
            (Some(scope_id), Some(facet)) => scope_id == facet,
            _ => false,
        },
        "title" => match (scope_id, event_title_id) {
            (Some(scope_id), Some(title_id)) => scope_id == title_id,
            _ => false,
        },
        _ => false,
    }
}
