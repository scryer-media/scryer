use async_graphql::{
    Context, Subscription,
    futures_util::stream::{self, BoxStream, unfold},
};
use scryer_application::{
    apply_download_queue_projection_event, replay_download_queue_state, sorted_download_queue_items,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventType, DownloadQueueState, Entitlement,
};
use std::collections::{HashSet, VecDeque};
use tokio::sync::broadcast::error::RecvError;

use crate::context::LogBuffer;
use crate::context::{actor_from_ctx, app_from_ctx};
use crate::mappers::{
    from_activity_event, from_domain_event, from_download_queue_item, from_job_run,
    from_library_scan_session,
};
use crate::types::{
    ActivityEventPayload, DomainEventEnvelopePayload, DownloadQueueItemPayload, JobRunPayload,
    LibraryScanProgressPayload,
};

pub struct SubscriptionRoot;

fn empty_box_stream<T: Send + 'static>() -> BoxStream<'static, T> {
    Box::pin(stream::empty())
}

async fn load_domain_events_for_projection(
    app: &scryer_application::AppUseCase,
    event_types: Vec<DomainEventType>,
) -> Result<(Vec<DomainEvent>, i64), scryer_application::AppError> {
    let mut events = Vec::new();
    let mut after_sequence = 0i64;

    loop {
        let batch = app
            .services
            .domain_events
            .list(&DomainEventFilter {
                event_types: Some(event_types.clone()),
                after_sequence: Some(after_sequence),
                limit: 500,
                ..DomainEventFilter::default()
            })
            .await?;
        if batch.is_empty() {
            break;
        }

        after_sequence = batch
            .last()
            .map(|event| event.sequence)
            .unwrap_or(after_sequence);
        let count = batch.len();
        events.extend(batch);
        if count < 500 {
            break;
        }
    }

    Ok((events, after_sequence))
}

async fn library_scan_state_stream_from_domain_events(
    app: scryer_application::AppUseCase,
    initial_sessions: Vec<scryer_application::LibraryScanSession>,
) -> BoxStream<'static, LibraryScanProgressPayload> {
    let receiver = app.services.library_scan_tracker.subscribe();

    let stream = unfold(
        (receiver, VecDeque::from(initial_sessions)),
        move |(mut receiver, mut pending)| async move {
            loop {
                if let Some(session) = pending.pop_front() {
                    return Some((from_library_scan_session(session), (receiver, pending)));
                }

                match receiver.recv().await {
                    Ok(session) => {
                        pending.push_back(session);
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "library_scan_state: receiver lagged, skipped {n} tracker updates"
                        );
                    }
                    Err(RecvError::Closed) => return None,
                }
            }
        },
    );

    Box::pin(stream)
}

async fn job_run_state_stream_from_domain_events(
    app: scryer_application::AppUseCase,
    initial_runs: Vec<scryer_application::JobRun>,
) -> BoxStream<'static, JobRunPayload> {
    let receiver = app.services.job_run_tracker.subscribe();

    let stream = unfold(
        (receiver, VecDeque::from(initial_runs)),
        move |(mut receiver, mut pending)| async move {
            loop {
                if let Some(run) = pending.pop_front() {
                    return Some((from_job_run(run), (receiver, pending)));
                }

                match receiver.recv().await {
                    Ok(run) => {
                        pending.push_back(run);
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "job_run_state: receiver lagged, skipped {n} tracker updates"
                        );
                    }
                    Err(RecvError::Closed) => return None,
                }
            }
        },
    );

    Box::pin(stream)
}

async fn download_queue_state_stream_from_domain_events(
    app: scryer_application::AppUseCase,
    include_all_activity: bool,
    include_history_only: bool,
) -> BoxStream<'static, Vec<DownloadQueueItemPayload>> {
    let receiver = app.services.domain_event_broadcast.subscribe();

    let event_types = vec![
        DomainEventType::DownloadQueueItemUpserted,
        DomainEventType::DownloadQueueItemRemoved,
    ];

    let (events, cursor) = match load_domain_events_for_projection(&app, event_types.clone()).await
    {
        Ok(loaded) => loaded,
        Err(error) => {
            tracing::warn!("download_queue_state: initial load failed: {error}");
            return empty_box_stream();
        }
    };

    let initial_items = replay_download_queue_state(&events);
    let initial_snapshot = filter_download_queue_items(
        sorted_download_queue_items(&initial_items),
        include_all_activity,
        include_history_only,
    )
    .into_iter()
    .map(from_download_queue_item)
    .collect::<Vec<_>>();

    let pending_initial = if initial_snapshot.is_empty() {
        VecDeque::new()
    } else {
        VecDeque::from(vec![initial_snapshot])
    };

    let stream = unfold(
        (receiver, cursor, pending_initial, initial_items),
        move |(mut receiver, mut cursor, mut pending, mut items)| {
            let app = app.clone();
            let event_types = event_types.clone();
            async move {
                loop {
                    if let Some(snapshot) = pending.pop_front() {
                        return Some((snapshot, (receiver, cursor, pending, items)));
                    }

                    let events = match app
                        .services
                        .domain_events
                        .list(&DomainEventFilter {
                            event_types: Some(event_types.clone()),
                            after_sequence: Some(cursor),
                            limit: 100,
                            ..DomainEventFilter::default()
                        })
                        .await
                    {
                        Ok(events) if !events.is_empty() => events,
                        Ok(_) => match receiver.recv().await {
                            Ok(sequence) => {
                                if sequence > cursor {
                                    cursor = sequence.saturating_sub(1);
                                }
                                continue;
                            }
                            Err(RecvError::Lagged(n)) => {
                                tracing::debug!(
                                    "download_queue_state: receiver lagged, skipped {n} wakeups"
                                );
                                continue;
                            }
                            Err(RecvError::Closed) => return None,
                        },
                        Err(error) => {
                            tracing::warn!("download_queue_state: list failed: {error}");
                            return None;
                        }
                    };

                    for event in events {
                        cursor = event.sequence;
                        if let Some(snapshot) =
                            apply_download_queue_projection_event(&mut items, &event)
                        {
                            let payload = filter_download_queue_items(
                                snapshot,
                                include_all_activity,
                                include_history_only,
                            )
                            .into_iter()
                            .map(from_download_queue_item)
                            .collect::<Vec<_>>();
                            pending.push_back(payload);
                        }
                    }
                }
            }
        },
    );

    Box::pin(stream)
}

#[Subscription]
impl SubscriptionRoot {
    async fn activity_events(&self, ctx: &Context<'_>) -> BoxStream<'static, ActivityEventPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("activity_events: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("activity_events: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_domain_event_sequences(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("activity_events: subscribe failed: {e}");
                return empty_box_stream();
            }
        };

        tracing::debug!(
            "activity_events: subscription started for user {}",
            actor.id
        );

        let stream = unfold(
            (receiver, 0_i64, VecDeque::new()),
            move |(mut receiver, mut cursor, mut pending): (
                tokio::sync::broadcast::Receiver<i64>,
                i64,
                VecDeque<(i64, scryer_application::ActivityEvent)>,
            )| {
                let app = app.clone();
                let actor = actor.clone();
                async move {
                    loop {
                        if let Some((sequence, event)) = pending.pop_front() {
                            cursor = sequence;
                            return Some((from_activity_event(event), (receiver, cursor, pending)));
                        }

                        let events = match app
                            .list_activity_events_after_sequence(&actor, cursor, 100)
                            .await
                        {
                            Ok(events) if !events.is_empty() => events,
                            Ok(_) => match receiver.recv().await {
                                Ok(sequence) => {
                                    if sequence > cursor {
                                        cursor = sequence.saturating_sub(1);
                                    }
                                    continue;
                                }
                                Err(RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "activity_events: receiver lagged, skipped {n} wakeups"
                                    );
                                    continue;
                                }
                                Err(RecvError::Closed) => {
                                    tracing::debug!("activity_events: broadcast channel closed");
                                    return None;
                                }
                            },
                            Err(error) => {
                                tracing::warn!("activity_events: list failed: {error}");
                                return None;
                            }
                        };

                        pending = events.into_iter().collect();
                    }
                }
            },
        );

        Box::pin(stream)
    }

    async fn domain_event_feed(
        &self,
        ctx: &Context<'_>,
        after_sequence: Option<i64>,
    ) -> BoxStream<'static, DomainEventEnvelopePayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!("domain_event_feed: app_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(error) => {
                tracing::warn!("domain_event_feed: actor_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_domain_event_sequences(&actor) {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("domain_event_feed: subscribe failed: {error}");
                return empty_box_stream();
            }
        };

        let initial_after = after_sequence.unwrap_or(0);
        let stream = unfold(
            (receiver, initial_after, VecDeque::<DomainEvent>::new()),
            move |(mut receiver, mut cursor, mut pending)| {
                let app = app.clone();
                let actor = actor.clone();
                async move {
                    loop {
                        if let Some(event) = pending.pop_front() {
                            cursor = event.sequence;
                            return Some((from_domain_event(event), (receiver, cursor, pending)));
                        }

                        let events = match app
                            .list_domain_events(
                                &actor,
                                &scryer_domain::DomainEventFilter {
                                    after_sequence: Some(cursor),
                                    limit: 100,
                                    ..scryer_domain::DomainEventFilter::default()
                                },
                            )
                            .await
                        {
                            Ok(events) if !events.is_empty() => events,
                            Ok(_) => match receiver.recv().await {
                                Ok(sequence) => {
                                    if sequence > cursor {
                                        cursor = sequence.saturating_sub(1);
                                    }
                                    continue;
                                }
                                Err(RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "domain_event_feed: receiver lagged, skipped {n} wakeups"
                                    );
                                    continue;
                                }
                                Err(RecvError::Closed) => return None,
                            },
                            Err(error) => {
                                tracing::warn!("domain_event_feed: list failed: {error}");
                                return None;
                            }
                        };

                        if !events.is_empty() {
                            pending = events.into_iter().collect();
                            continue;
                        }
                    }
                }
            },
        );

        Box::pin(stream)
    }

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
    ) -> BoxStream<'static, Vec<DownloadQueueItemPayload>> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("download_queue sub: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("download_queue sub: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            tracing::warn!("download_queue sub: insufficient entitlements");
            return empty_box_stream();
        }

        tracing::debug!(
            "download_queue sub: subscription started for user {}",
            actor.id
        );

        download_queue_state_stream_from_domain_events(
            app,
            include_all_activity.unwrap_or(false),
            include_history_only.unwrap_or(false),
        )
        .await
    }

    async fn download_queue_state(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
    ) -> BoxStream<'static, Vec<DownloadQueueItemPayload>> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("download_queue_state sub: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("download_queue_state sub: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            tracing::warn!("download_queue_state sub: insufficient entitlements");
            return empty_box_stream();
        }

        download_queue_state_stream_from_domain_events(
            app,
            include_all_activity.unwrap_or(false),
            include_history_only.unwrap_or(false),
        )
        .await
    }

    async fn library_scan_progress(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, LibraryScanProgressPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("library_scan_progress: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("library_scan_progress: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ViewCatalog) {
            tracing::warn!("library_scan_progress: insufficient entitlements");
            return empty_box_stream();
        }

        tracing::debug!(
            "library_scan_progress: subscription started for user {}",
            actor.id
        );

        let initial_sessions = match app.active_library_scans(&actor).await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!("library_scan_progress: initial load failed: {error}");
                return empty_box_stream();
            }
        };

        library_scan_state_stream_from_domain_events(app, initial_sessions).await
    }

    async fn library_scan_state(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, LibraryScanProgressPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("library_scan_state: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("library_scan_state: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ViewCatalog) {
            tracing::warn!("library_scan_state: insufficient entitlements");
            return empty_box_stream();
        }

        let initial_sessions = match app.active_library_scans(&actor).await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!("library_scan_state: initial load failed: {error}");
                return empty_box_stream();
            }
        };

        library_scan_state_stream_from_domain_events(app, initial_sessions).await
    }

    async fn job_run_events(&self, ctx: &Context<'_>) -> BoxStream<'static, JobRunPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!("job_run_events: app_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(error) => {
                tracing::warn!("job_run_events: actor_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            tracing::warn!("job_run_events: insufficient entitlements");
            return empty_box_stream();
        }

        let initial_runs = match app.active_job_runs(&actor).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!("job_run_events: initial load failed: {error}");
                return empty_box_stream();
            }
        };

        job_run_state_stream_from_domain_events(app, initial_runs).await
    }

    async fn job_run_state(&self, ctx: &Context<'_>) -> BoxStream<'static, JobRunPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!("job_run_state: app_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(error) => {
                tracing::warn!("job_run_state: actor_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            tracing::warn!("job_run_state: insufficient entitlements");
            return empty_box_stream();
        }

        let initial_runs = match app.active_job_runs(&actor).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!("job_run_state: initial load failed: {error}");
                return empty_box_stream();
            }
        };

        job_run_state_stream_from_domain_events(app, initial_runs).await
    }

    async fn service_log_lines(&self, ctx: &Context<'_>) -> BoxStream<'static, String> {
        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("service_log_lines: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            tracing::warn!("service_log_lines: insufficient entitlements");
            return empty_box_stream();
        }

        let receiver = match ctx.data_opt::<LogBuffer>() {
            Some(buf) => buf.subscribe(),
            None => {
                tracing::warn!("service_log_lines: no LogBuffer in context");
                return empty_box_stream();
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
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("import_history_changed: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("import_history_changed: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_import_history(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("import_history_changed: subscribe failed: {e}");
                return empty_box_stream();
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

    async fn settings_changed(&self, ctx: &Context<'_>) -> BoxStream<'static, Vec<String>> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("settings_changed: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("settings_changed: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_settings_changed(&actor) {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("settings_changed: subscribe failed: {e}");
                return empty_box_stream();
            }
        };

        tracing::debug!(
            "settings_changed: subscription started for user {}",
            actor.id
        );

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(keys) => return Some((keys, receiver)),
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!("settings_changed: receiver lagged, skipped {n} messages");
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("settings_changed: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        Box::pin(stream)
    }
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
                    DownloadQueueState::Downloading
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
        let key = if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
            item.id.clone()
        } else {
            format!("{}:{}", item.client_type, item.download_client_item_id)
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
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
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
            item("job-2b", DownloadQueueState::ImportPending, true),
            item("job-3", DownloadQueueState::Queued, true),
            item("job-4", DownloadQueueState::Queued, false),
        ];

        let filtered = filter_download_queue_items(items, false, false);

        assert_eq!(filtered.len(), 1);
        assert!(filtered.iter().all(|item| item.is_scryer_origin));
        assert!(filtered.iter().all(|item| {
            matches!(
                item.state,
                DownloadQueueState::Downloading
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
