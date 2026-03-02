use async_graphql::{
    futures_util::stream::{self, unfold, BoxStream},
    Context, Subscription,
};
use scryer_domain::DownloadQueueState;
use tokio::sync::broadcast::error::RecvError;

use crate::context::{actor_from_ctx, app_from_ctx};
use crate::mappers::from_activity_event;
use crate::mappers::from_download_queue_item;
use crate::types::{ActivityEventPayload, DownloadQueueItemPayload};
use crate::context::LogBuffer;

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

        tracing::debug!("activity_events: subscription started for user {}", actor.id);

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

        tracing::debug!("download_queue sub: subscription started for user {}", actor.id);

        let include_all_activity = include_all_activity.unwrap_or(false);
        let include_history_only = include_history_only.unwrap_or(false);

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(items) => {
                        let mut items = if include_all_activity {
                            items
                        } else if include_history_only {
                            items
                                .into_iter()
                                .filter(|item| {
                                    matches!(
                                        item.state,
                                        DownloadQueueState::Completed | DownloadQueueState::Failed
                                        | DownloadQueueState::ImportPending
                                    )
                                })
                                .collect()
                        } else {
                            items
                                .into_iter()
                                .filter(|item| item.is_scryer_origin)
                                .collect()
                        };

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
                        tracing::debug!("download_queue sub: receiver lagged, skipped {n} messages");
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

        tracing::debug!("service_log_lines: subscription started for user {}", actor.id);

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
}

fn parse_sort_value_desc(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}
