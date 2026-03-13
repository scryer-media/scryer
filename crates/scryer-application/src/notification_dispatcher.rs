use crate::{activity::NotificationEnvelope, ActivityEvent, AppUseCase};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub async fn start_notification_dispatcher(app: AppUseCase, cancel: CancellationToken) {
    info!("notification dispatcher started");
    let mut rx = app.services.activity_event_broadcast.subscribe();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("notification dispatcher shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Some(envelope) = &event.notification {
                            dispatch_event(&app, &event, envelope).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "notification dispatcher lagged, some events may not have been dispatched");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("activity broadcast closed, notification dispatcher exiting");
                        break;
                    }
                }
            }
        }
    }
}

async fn dispatch_event(app: &AppUseCase, event: &ActivityEvent, envelope: &NotificationEnvelope) {
    let event_type_str = envelope.event_type.as_str();
    debug!(event_type = event_type_str, title_id = ?event.title_id, "dispatching notification");

    let sub_repo = match app.notification_subscriptions_repo() {
        Ok(r) => r,
        Err(_) => return,
    };
    let ch_repo = match app.notification_channels_repo() {
        Ok(r) => r,
        Err(_) => return,
    };
    let provider = match app.services.notification_provider.as_ref() {
        Some(p) => p,
        None => return,
    };

    let subscriptions = match sub_repo.list_subscriptions_for_event(event_type_str).await {
        Ok(subs) => subs,
        Err(e) => {
            warn!(error = %e, event_type = event_type_str, "failed to list notification subscriptions");
            return;
        }
    };

    for sub in subscriptions {
        if !sub.is_enabled {
            continue;
        }

        // Scope filtering
        if !matches_scope(
            &sub.scope,
            sub.scope_id.as_deref(),
            event.title_id.as_deref(),
            envelope.facet.as_deref(),
        ) {
            continue;
        }

        let channel = match ch_repo.get_channel(&sub.channel_id).await {
            Ok(Some(ch)) if ch.is_enabled => ch,
            _ => continue,
        };

        let client = match provider.client_for_channel(&channel) {
            Some(c) => c,
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
                event_type_str,
                &envelope.title,
                &envelope.body,
                &envelope.metadata,
            )
            .await
        {
            Ok(()) => {
                info!(
                    event_type = event_type_str,
                    channel = channel.name.as_str(),
                    "notification dispatched"
                );
            }
            Err(e) => {
                warn!(
                    event_type = event_type_str,
                    channel = channel.name.as_str(),
                    error = %e,
                    "notification dispatch failed"
                );
            }
        }
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
            (Some(sid), Some(facet)) => sid == facet,
            _ => false,
        },
        "title" => match (scope_id, event_title_id) {
            (Some(sid), Some(tid)) => sid == tid,
            _ => false,
        },
        _ => true, // unknown scope, allow
    }
}
