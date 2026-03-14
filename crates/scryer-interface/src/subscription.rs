use async_graphql::{
    futures_util::stream::{self, unfold, BoxStream},
    Context, Subscription,
};
use scryer_domain::DownloadQueueState;
use std::collections::HashSet;
use tokio::sync::broadcast::error::RecvError;

use crate::context::LogBuffer;
use crate::context::{actor_from_ctx, app_from_ctx};
use crate::mappers::from_activity_event;
use crate::mappers::from_download_queue_item;
use crate::types::{ActivityEventPayload, DownloadQueueItemPayload};

pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    async fn activity_events(&self, ctx: &Context<'_>) -> BoxStream<'static, ActivityEventPayload> {
        let empty_stream =
            || -> BoxStream<'static, ActivityEventPayload> { Box::pin(stream::empty()) };

        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("activity_events: app_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("activity_events: actor_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let receiver = match app.subscribe_activity_events(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("activity_events: subscribe failed: {e}");
                return empty_stream();
            }
        };

        tracing::debug!(
            "activity_events: subscription started for user {}",
            actor.id
        );

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => return Some((from_activity_event(event), receiver)),
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!("activity_events: receiver lagged, skipped {n} messages");
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("activity_events: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        Box::pin(stream)
    }

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
    ) -> BoxStream<'static, Vec<DownloadQueueItemPayload>> {
        let empty_stream =
            || -> BoxStream<'static, Vec<DownloadQueueItemPayload>> { Box::pin(stream::empty()) };

        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("download_queue sub: app_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("download_queue sub: actor_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let receiver = match app.subscribe_download_queue(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("download_queue sub: subscribe failed: {e}");
                return empty_stream();
            }
        };

        tracing::debug!(
            "download_queue sub: subscription started for user {}",
            actor.id
        );

        let include_all_activity = include_all_activity.unwrap_or(false);
        let include_history_only = include_history_only.unwrap_or(false);

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(items) => {
                        let mut items = filter_download_queue_items(
                            items,
                            include_all_activity,
                            include_history_only,
                        );

                        if include_history_only {
                            items.sort_by(|left, right| {
                                parse_sort_value_desc(
                                    right.last_updated_at.as_deref(),
                                    left.last_updated_at.as_deref(),
                                )
                            });
                            items.truncate(50);
                        }

                        let payloads = items.into_iter().map(from_download_queue_item).collect();
                        return Some((payloads, receiver));
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "download_queue sub: receiver lagged, skipped {n} messages"
                        );
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("download_queue sub: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        Box::pin(stream)
    }

    async fn service_log_lines(&self, ctx: &Context<'_>) -> BoxStream<'static, String> {
        let empty_stream = || -> BoxStream<'static, String> { Box::pin(stream::empty()) };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("service_log_lines: actor_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            tracing::warn!("service_log_lines: insufficient entitlements");
            return empty_stream();
        }

        let receiver = match ctx.data_opt::<LogBuffer>() {
            Some(buf) => buf.subscribe(),
            None => {
                tracing::warn!("service_log_lines: no LogBuffer in context");
                return empty_stream();
            }
        };

        tracing::debug!(
            "service_log_lines: subscription started for user {}",
            actor.id
        );

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(line) => return Some((line, receiver)),
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!("service_log_lines: receiver lagged, skipped {n} messages");
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("service_log_lines: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        Box::pin(stream)
    }

    async fn import_history_changed(&self, ctx: &Context<'_>) -> BoxStream<'static, bool> {
        let empty_stream = || -> BoxStream<'static, bool> { Box::pin(stream::empty()) };

        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("import_history_changed: app_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("import_history_changed: actor_from_ctx failed: {e:?}");
                return empty_stream();
            }
        };

        let receiver = match app.subscribe_import_history(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("import_history_changed: subscribe failed: {e}");
                return empty_stream();
            }
        };

        tracing::debug!(
            "import_history_changed: subscription started for user {}",
            actor.id
        );

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(()) => return Some((true, receiver)),
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "import_history_changed: receiver lagged, skipped {n} messages"
                        );
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("import_history_changed: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        Box::pin(stream)
    }
}

fn parse_sort_value_desc(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}

fn filter_download_queue_items(
    items: Vec<scryer_domain::DownloadQueueItem>,
    include_all_activity: bool,
    include_history_only: bool,
) -> Vec<scryer_domain::DownloadQueueItem> {
    dedupe_download_queue_items(items)
        .into_iter()
        .filter(|item| {
            if include_all_activity {
                return true;
            }

            if include_history_only {
                return matches!(
                    item.state,
                    DownloadQueueState::Completed
                        | DownloadQueueState::Failed
                        | DownloadQueueState::ImportPending
                );
            }

            item.is_scryer_origin
                && matches!(
                    item.state,
                    DownloadQueueState::ImportPending
                        | DownloadQueueState::Failed
                        | DownloadQueueState::Downloading
                        | DownloadQueueState::Queued
                        | DownloadQueueState::Paused
                        | DownloadQueueState::Verifying
                        | DownloadQueueState::Repairing
                        | DownloadQueueState::Extracting
                )
        })
        .collect()
}

fn dedupe_download_queue_items(
    items: Vec<scryer_domain::DownloadQueueItem>,
) -> Vec<scryer_domain::DownloadQueueItem> {
    let mut seen = HashSet::with_capacity(items.len());
    let mut deduped = Vec::with_capacity(items.len());

    for item in items {
        let key = if item.client_type.trim().is_empty() && item.download_client_item_id.is_empty() {
            item.id.clone()
        } else {
            format!(
                "{}:{}",
                item.client_type.trim().to_ascii_lowercase(),
                item.download_client_item_id.trim()
            )
        };

        if seen.insert(key) {
            deduped.push(item);
        }
    }

    deduped
}

#[cfg(test)]
mod tests {
    use super::{dedupe_download_queue_items, filter_download_queue_items};
    use chrono::Utc;
    use scryer_domain::{DownloadQueueItem, DownloadQueueState};

    fn item(id: &str, state: DownloadQueueState, is_scryer_origin: bool) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            title_name: "Example".to_string(),
            facet: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent: 100,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: Some(Utc::now().timestamp_millis().to_string()),
            last_updated_at: Some(Utc::now().timestamp_millis().to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            import_status: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin,
        }
    }

    #[test]
    fn dedupe_download_queue_items_keeps_first_instance_for_duplicate_client_job_ids() {
        let items = vec![
            item("job-1", DownloadQueueState::Completed, true),
            item("job-1", DownloadQueueState::Completed, true),
            item("job-2", DownloadQueueState::Failed, true),
        ];

        let deduped = dedupe_download_queue_items(items);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].download_client_item_id, "job-1");
        assert_eq!(deduped[1].download_client_item_id, "job-2");
    }

    #[test]
    fn filter_download_queue_items_hides_completed_entries_from_scryer_only_live_view() {
        let items = vec![
            item("job-1", DownloadQueueState::Completed, true),
            item("job-2", DownloadQueueState::Failed, true),
            item("job-3", DownloadQueueState::Queued, true),
            item("job-4", DownloadQueueState::Queued, false),
        ];

        let filtered = filter_download_queue_items(items, false, false);

        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|item| item.is_scryer_origin));
        assert!(filtered.iter().all(|item| {
            matches!(
                item.state,
                DownloadQueueState::Failed
                    | DownloadQueueState::ImportPending
                    | DownloadQueueState::Downloading
                    | DownloadQueueState::Queued
                    | DownloadQueueState::Paused
            )
        }));
    }

    #[test]
    fn filter_keeps_processing_states_in_scryer_only_view() {
        let items = vec![
            item("job-1", DownloadQueueState::Verifying, true),
            item("job-2", DownloadQueueState::Repairing, true),
            item("job-3", DownloadQueueState::Extracting, true),
            item("job-4", DownloadQueueState::Extracting, false),
        ];

        let filtered = filter_download_queue_items(items, false, false);

        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|item| item.is_scryer_origin));
    }
}
