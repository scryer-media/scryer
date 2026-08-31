use async_graphql::{
    Context, ID, Subscription,
    futures_util::{
        StreamExt,
        stream::{self, BoxStream, unfold},
    },
};
use scryer_application::user_facing_domain_event_types;
use scryer_domain::{AppPermission, DomainEvent, DomainEventPayload, DownloadQueueItem};
use std::collections::{HashSet, VecDeque};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use scryer_interface_core::LogBuffer;
use scryer_interface_core::{actor_from_ctx, app_from_ctx, auth_runtime_from_ctx};
use scryer_interface_media::mappers::{
    from_activity_event, from_domain_event, from_download_queue_item,
    from_external_import_monitor_warmup_progress, from_job_run, from_library_scan_session,
    from_plugin_install_progress,
};
use scryer_interface_media::types;
use scryer_interface_media::types::{
    ActiveImportStreamSyncPayload, ActivityEventPayload, DomainEventEnvelopePayload,
    DownloadActivityFilterValue, DownloadQueueItemPayload, DownloadQueueSyncPayload,
    ExternalImportMonitorWarmupProgressPayload, IntoApplication, JobRunPayload,
    LibraryScanProgressPayload, Long, MediaRequestChangedPayload, PluginInstallProgressPayload,
    ProviderCatalogFamilyValue,
};

pub struct SubscriptionRoot;

fn empty_box_stream<T: Send + 'static>() -> BoxStream<'static, T> {
    Box::pin(stream::empty())
}

fn guard_subscription_stream<T: Send + 'static>(
    ctx: &Context<'_>,
    stream: BoxStream<'static, T>,
) -> BoxStream<'static, T> {
    let auth_runtime = auth_runtime_from_ctx(ctx);
    let expected_epoch = auth_runtime.snapshot().epoch;
    let epoch_rx = auth_runtime.subscribe_epoch();

    Box::pin(unfold(
        (stream, epoch_rx, expected_epoch),
        move |(mut stream, mut epoch_rx, expected_epoch)| async move {
            loop {
                tokio::select! {
                    next = stream.next() => {
                        return next.map(|item| (item, (stream, epoch_rx, expected_epoch)));
                    }
                    changed = epoch_rx.changed() => {
                        match changed {
                            Ok(()) if *epoch_rx.borrow() == expected_epoch => continue,
                            Ok(()) | Err(_) => return None,
                        }
                    }
                }
            }
        },
    ))
}

fn media_request_changed_payload(event: DomainEvent) -> Option<MediaRequestChangedPayload> {
    let event_id = event.event_id;
    let event_type = event.payload.event_type();

    match event.payload {
        DomainEventPayload::MediaRequestSubmitted(data)
        | DomainEventPayload::MediaRequestUpdated(data) => Some(MediaRequestChangedPayload {
            event_id: event_id.into(),
            event_type: crate::types::DomainEventTypeValue::from_domain(event_type),
            request_id: data.request_id.into(),
            library_id: data.library_id.into(),
        }),
        DomainEventPayload::MediaRequestApproved(data)
        | DomainEventPayload::MediaRequestRejected(data)
        | DomainEventPayload::MediaRequestCanceled(data) => Some(MediaRequestChangedPayload {
            event_id: event_id.into(),
            event_type: crate::types::DomainEventTypeValue::from_domain(event_type),
            request_id: data.request_id.into(),
            library_id: data.library_id.into(),
        }),
        _ => None,
    }
}

fn provider_catalog_family(value: &str) -> Option<ProviderCatalogFamilyValue> {
    match value {
        "subtitle" => Some(ProviderCatalogFamilyValue::Subtitle),
        "notification" => Some(ProviderCatalogFamilyValue::Notification),
        "indexer" => Some(ProviderCatalogFamilyValue::Indexer),
        "download_client" => Some(ProviderCatalogFamilyValue::DownloadClient),
        "archive_extractor" => Some(ProviderCatalogFamilyValue::ArchiveExtractor),
        _ => None,
    }
}

fn library_scan_state_stream_from_domain_events(
    receiver: tokio::sync::broadcast::Receiver<scryer_application::LibraryScanSession>,
) -> BoxStream<'static, LibraryScanProgressPayload> {
    let stream = unfold(receiver, move |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(session) => {
                    return Some((from_library_scan_session(session), receiver));
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!(
                        "library_scan_state: receiver lagged, skipped {n} projected updates"
                    );
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });

    Box::pin(stream)
}

async fn job_run_state_stream_from_domain_events(
    receiver: tokio::sync::broadcast::Receiver<scryer_application::JobRun>,
    initial_runs: Vec<scryer_application::JobRun>,
) -> BoxStream<'static, JobRunPayload> {
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

fn download_queue_state_stream_from_snapshots(
    receiver: tokio::sync::broadcast::Receiver<Vec<DownloadQueueItem>>,
    include_all_activity: bool,
    include_history_only: bool,
    include_import_activity: bool,
    title_id: Option<String>,
    activity_filter: DownloadActivityFilterValue,
) -> BoxStream<'static, Vec<DownloadQueueItemPayload>> {
    let stream = unfold(
        (receiver, VecDeque::<Vec<DownloadQueueItemPayload>>::new()),
        move |(mut receiver, mut pending)| {
            let title_id = title_id.clone();
            async move {
                loop {
                    if let Some(snapshot) = pending.pop_front() {
                        return Some((snapshot, (receiver, pending)));
                    }

                    match receiver.recv().await {
                        Ok(snapshot) => {
                            let payload = filter_download_queue_items(
                                snapshot,
                                include_all_activity,
                                include_history_only,
                                include_import_activity,
                                title_id.as_deref(),
                                activity_filter,
                            )
                            .into_iter()
                            .map(from_download_queue_item)
                            .collect::<Vec<_>>();
                            pending.push_back(payload);
                        }
                        Err(RecvError::Lagged(n)) => {
                            tracing::debug!(
                                "download_queue_state: receiver lagged, skipped {n} snapshots"
                            );
                        }
                        Err(RecvError::Closed) => return None,
                    }
                }
            }
        },
    );

    Box::pin(stream)
}

fn download_queue_sync_stream(
    receiver: tokio::sync::watch::Receiver<scryer_application::DownloadQueueSync>,
) -> BoxStream<'static, DownloadQueueSyncPayload> {
    Box::pin(unfold(
        (receiver, true),
        |(mut receiver, first)| async move {
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let sync = receiver.borrow_and_update().clone();
            Some((
                DownloadQueueSyncPayload {
                    revision: Long::from_u64_saturating(sync.revision),
                    updated_at: sync.updated_at,
                },
                (receiver, false),
            ))
        },
    ))
}

fn active_import_stream_sync_stream(
    receiver: tokio::sync::watch::Receiver<scryer_application::ActiveImportStreamSync>,
) -> BoxStream<'static, ActiveImportStreamSyncPayload> {
    Box::pin(unfold(
        (receiver, true),
        |(mut receiver, first)| async move {
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let sync = receiver.borrow_and_update().clone();
            Some((
                ActiveImportStreamSyncPayload {
                    revision: Long::from_u64_saturating(sync.revision),
                    updated_at: sync.updated_at,
                },
                (receiver, false),
            ))
        },
    ))
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

        let receiver = match app.subscribe_domain_event_sequences(&actor).await {
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

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn domain_event_feed(
        &self,
        ctx: &Context<'_>,
        after_sequence: Option<Long>,
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

        let receiver = match app.subscribe_domain_event_sequences(&actor).await {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("domain_event_feed: subscribe failed: {error}");
                return empty_box_stream();
            }
        };

        let initial_after = after_sequence.map(|value| value.0).unwrap_or(0);
        let event_types = user_facing_domain_event_types();
        let stream = unfold(
            (receiver, initial_after, VecDeque::<DomainEvent>::new()),
            move |(mut receiver, mut cursor, mut pending)| {
                let app = app.clone();
                let actor = actor.clone();
                let event_types = event_types.clone();
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
                                    event_types: Some(event_types.clone()),
                                    limit: 100,
                                    ..scryer_domain::DomainEventFilter::default()
                                },
                            )
                            .await
                        {
                            Ok(events) if !events.is_empty() => events,
                            Ok(_) => match receiver.recv().await {
                                Ok(sequence) => {
                                    let mut earliest_sequence_after_cursor =
                                        (sequence > cursor).then_some(sequence);
                                    loop {
                                        match receiver.try_recv() {
                                            Ok(next_sequence) => {
                                                if next_sequence > cursor {
                                                    earliest_sequence_after_cursor = Some(
                                                        earliest_sequence_after_cursor
                                                            .map_or(next_sequence, |earliest| {
                                                                earliest.min(next_sequence)
                                                            }),
                                                    );
                                                }
                                            }
                                            Err(TryRecvError::Empty) => break,
                                            Err(TryRecvError::Lagged(n)) => {
                                                tracing::debug!(
                                                    "domain_event_feed: receiver lagged while draining, skipped {n} wakeups"
                                                );
                                                break;
                                            }
                                            Err(TryRecvError::Closed) => return None,
                                        }
                                    }
                                    if let Some(sequence) = earliest_sequence_after_cursor {
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

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn download_queue_sync(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, DownloadQueueSyncPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!("download_queue_sync sub: app_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(error) => {
                tracing::warn!("download_queue_sync sub: actor_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        let receiver = match app.subscribe_download_queue_sync(&actor) {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("download_queue_sync sub: subscribe failed: {error}");
                return empty_box_stream();
            }
        };
        guard_subscription_stream(ctx, download_queue_sync_stream(receiver))
    }

    /// Notify clients when queued or active import streams change.
    async fn active_import_streams_sync(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, ActiveImportStreamSyncPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!("active_import_streams_sync sub: app_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(error) => {
                tracing::warn!("active_import_streams_sync sub: actor_from_ctx failed: {error:?}");
                return empty_box_stream();
            }
        };
        let receiver = match app.subscribe_active_import_streams(&actor).await {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("active_import_streams_sync sub: subscribe failed: {error}");
                return empty_box_stream();
            }
        };
        guard_subscription_stream(ctx, active_import_stream_sync_stream(receiver))
    }

    #[graphql(deprecation = "use downloadQueueSync")]
    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        title_id: Option<ID>,
        activity_filter: Option<DownloadActivityFilterValue>,
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
        tracing::debug!(
            "download_queue sub: subscription started for user {}",
            actor.id
        );

        let receiver = match app.subscribe_download_queue_state(&actor) {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("download_queue sub: subscribe failed: {error}");
                return empty_box_stream();
            }
        };

        guard_subscription_stream(
            ctx,
            download_queue_state_stream_from_snapshots(
                receiver,
                include_all_activity.unwrap_or(false),
                include_history_only.unwrap_or(false),
                include_import_activity.unwrap_or(false),
                title_id.map(String::from),
                activity_filter.unwrap_or(DownloadActivityFilterValue::All),
            ),
        )
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
        let receiver = match app.subscribe_library_scan_progress(&actor).await {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("library_scan_state: subscription setup failed: {error}");
                return empty_box_stream();
            }
        };

        guard_subscription_stream(ctx, library_scan_state_stream_from_domain_events(receiver))
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
        let initial_runs = match app.active_job_runs(&actor).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!("job_run_events: initial load failed: {error}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_job_run_state(&actor).await {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::warn!("job_run_events: subscribe failed: {error}");
                return empty_box_stream();
            }
        };

        guard_subscription_stream(
            ctx,
            job_run_state_stream_from_domain_events(receiver, initial_runs).await,
        )
    }

    async fn service_log_lines(&self, ctx: &Context<'_>) -> BoxStream<'static, String> {
        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("service_log_lines: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        if !actor
            .authorization
            .has_app_permission(AppPermission::ManageSystemSettings)
        {
            tracing::warn!("service_log_lines: insufficient permissions");
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

        guard_subscription_stream(ctx, Box::pin(stream))
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

        let receiver = match app.subscribe_import_history(&actor).await {
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

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn media_requests_changed(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, MediaRequestChangedPayload> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("media_requests_changed: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("media_requests_changed: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        match app.can_access_media_requests(&actor).await {
            Ok(true) => {}
            Ok(false) => return empty_box_stream(),
            Err(e) => {
                tracing::warn!("media_requests_changed: permission check failed: {e}");
                return empty_box_stream();
            }
        }

        let receiver = match app.subscribe_domain_event_sequences(&actor).await {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("media_requests_changed: subscribe failed: {e}");
                return empty_box_stream();
            }
        };

        tracing::debug!(
            "media_requests_changed: subscription started for user {}",
            actor.id
        );

        let stream = unfold(
            (
                receiver,
                Option::<i64>::None,
                VecDeque::<DomainEvent>::new(),
            ),
            move |(mut receiver, mut cursor, mut pending)| {
                let app = app.clone();
                let actor = actor.clone();
                async move {
                    loop {
                        if let Some(event) = pending.pop_front() {
                            cursor = Some(event.sequence);
                            if let Some(payload) = media_request_changed_payload(event) {
                                return Some((payload, (receiver, cursor, pending)));
                            }
                            continue;
                        }

                        let after_sequence = match cursor {
                            Some(cursor) => cursor,
                            None => match receiver.recv().await {
                                Ok(sequence) => {
                                    cursor = Some(sequence.saturating_sub(1));
                                    continue;
                                }
                                Err(RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "media_requests_changed: receiver lagged, skipped {n} wakeups"
                                    );
                                    continue;
                                }
                                Err(RecvError::Closed) => {
                                    tracing::debug!(
                                        "media_requests_changed: broadcast channel closed"
                                    );
                                    return None;
                                }
                            },
                        };

                        let events = match app
                            .list_media_request_lifecycle_events_for_actor(
                                &actor,
                                after_sequence,
                                100,
                            )
                            .await
                        {
                            Ok(events) if !events.is_empty() => events,
                            Ok(_) => match receiver.recv().await {
                                Ok(sequence) => {
                                    if sequence > after_sequence {
                                        cursor = Some(sequence.saturating_sub(1));
                                    }
                                    continue;
                                }
                                Err(RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "media_requests_changed: receiver lagged, skipped {n} wakeups"
                                    );
                                    continue;
                                }
                                Err(RecvError::Closed) => {
                                    tracing::debug!(
                                        "media_requests_changed: broadcast channel closed"
                                    );
                                    return None;
                                }
                            },
                            Err(error) => {
                                tracing::warn!("media_requests_changed: list failed: {error}");
                                return None;
                            }
                        };

                        pending = events.into_iter().collect();
                    }
                }
            },
        );

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn indexers_changed(&self, ctx: &Context<'_>) -> BoxStream<'static, bool> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("indexers_changed: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("indexers_changed: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_indexers_changed(&actor).await {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("indexers_changed: subscribe failed: {e}");
                return empty_box_stream();
            }
        };

        tracing::debug!(
            "indexers_changed: subscription started for user {}",
            actor.id
        );

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(()) => return Some((true, receiver)),
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!("indexers_changed: receiver lagged, skipped {n} messages");
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("indexers_changed: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn provider_catalog_changed(
        &self,
        ctx: &Context<'_>,
    ) -> BoxStream<'static, Vec<ProviderCatalogFamilyValue>> {
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("provider_catalog_changed: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("provider_catalog_changed: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app.subscribe_provider_catalog_changed(&actor).await {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!("provider_catalog_changed: subscribe failed: {e}");
                return empty_box_stream();
            }
        };

        let stream = unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(families) => {
                        let payload = families
                            .into_iter()
                            .filter_map(|family| provider_catalog_family(family.as_str()))
                            .collect::<Vec<_>>();
                        return Some((payload, receiver));
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "provider_catalog_changed: receiver lagged, skipped {n} messages"
                        );
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("provider_catalog_changed: broadcast channel closed");
                        return None;
                    }
                }
            }
        });

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn plugin_install_progress(
        &self,
        ctx: &Context<'_>,
        plugin_id: async_graphql::ID,
    ) -> BoxStream<'static, PluginInstallProgressPayload> {
        let plugin_id = String::from(plugin_id);
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!("plugin_install_progress: app_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!("plugin_install_progress: actor_from_ctx failed: {e:?}");
                return empty_box_stream();
            }
        };

        let receiver = match app
            .subscribe_plugin_install_progress(&actor, &plugin_id)
            .await
        {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!(
                    plugin_id = plugin_id.as_str(),
                    "plugin_install_progress: subscribe failed: {e}"
                );
                return empty_box_stream();
            }
        };

        let stream = unfold(
            (receiver, true),
            move |(mut receiver, emit_initial)| async move {
                if emit_initial {
                    let payload = from_plugin_install_progress(receiver.borrow().clone());
                    return Some((payload, (receiver, false)));
                }

                match receiver.changed().await {
                    Ok(()) => {
                        let payload = from_plugin_install_progress(receiver.borrow().clone());
                        Some((payload, (receiver, false)))
                    }
                    Err(_) => None,
                }
            },
        );

        guard_subscription_stream(ctx, Box::pin(stream))
    }

    async fn external_import_monitor_warmup_progress(
        &self,
        ctx: &Context<'_>,
        session_id: async_graphql::ID,
    ) -> BoxStream<'static, ExternalImportMonitorWarmupProgressPayload> {
        let session_id = String::from(session_id);
        let app = match app_from_ctx(ctx) {
            Ok(app) => app,
            Err(e) => {
                tracing::warn!(
                    "external_import_monitor_warmup_progress: app_from_ctx failed: {e:?}"
                );
                return empty_box_stream();
            }
        };

        let actor = match actor_from_ctx(ctx) {
            Ok(actor) => actor,
            Err(e) => {
                tracing::warn!(
                    "external_import_monitor_warmup_progress: actor_from_ctx failed: {e:?}"
                );
                return empty_box_stream();
            }
        };

        let receiver = match app
            .subscribe_external_import_monitor_warmup_progress(&actor, &session_id)
            .await
        {
            Ok(receiver) => receiver,
            Err(e) => {
                tracing::warn!(
                    session_id = session_id.as_str(),
                    "external_import_monitor_warmup_progress: subscribe failed: {e}"
                );
                return empty_box_stream();
            }
        };

        let stream = unfold(
            (receiver, true),
            move |(mut receiver, emit_initial)| async move {
                if emit_initial {
                    let payload =
                        from_external_import_monitor_warmup_progress(receiver.borrow().clone());
                    return Some((payload, (receiver, false)));
                }

                match receiver.changed().await {
                    Ok(()) => {
                        let payload =
                            from_external_import_monitor_warmup_progress(receiver.borrow().clone());
                        Some((payload, (receiver, false)))
                    }
                    Err(_) => None,
                }
            },
        );

        guard_subscription_stream(ctx, Box::pin(stream))
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

        let receiver = match app.subscribe_settings_changed(&actor).await {
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

        guard_subscription_stream(ctx, Box::pin(stream))
    }
}

fn filter_download_queue_items(
    items: Vec<scryer_domain::DownloadQueueItem>,
    include_all_activity: bool,
    include_history_only: bool,
    include_import_activity: bool,
    title_id: Option<&str>,
    activity_filter: DownloadActivityFilterValue,
) -> Vec<scryer_domain::DownloadQueueItem> {
    dedupe_download_queue_items(items)
        .into_iter()
        .filter(|item| {
            if let Some(title_id) = title_id
                && item.title_id.as_deref() != Some(title_id)
            {
                return false;
            }

            let matches_queue_filter = scryer_application::matches_download_queue_filter(
                item,
                include_history_only,
                include_import_activity,
                activity_filter.into_application(),
            );
            if include_all_activity {
                return matches_queue_filter;
            }

            item.is_scryer_origin && matches_queue_filter
        })
        .collect()
}

fn dedupe_download_queue_items(
    items: Vec<scryer_domain::DownloadQueueItem>,
) -> Vec<scryer_domain::DownloadQueueItem> {
    let mut seen = HashSet::with_capacity(items.len());
    let mut deduped = Vec::with_capacity(items.len());

    for item in items {
        let key = download_queue_item_identity_key(&item);

        if seen.insert(key) {
            deduped.push(item);
        }
    }

    deduped
}

fn download_queue_item_identity_key(item: &scryer_domain::DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    let client_id = item.client_id.trim();
    if client_id.is_empty() {
        format!("{}:{}", item.client_type, item.download_client_item_id)
    } else {
        format!("{}:{}", client_id, item.download_client_item_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{dedupe_download_queue_items, filter_download_queue_items};
    use crate::types::DownloadActivityFilterValue;
    use chrono::Utc;
    use scryer_domain::{DownloadQueueItem, DownloadQueueState};

    fn item(id: &str, state: DownloadQueueState, is_scryer_origin: bool) -> DownloadQueueItem {
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
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: Some(Utc::now().timestamp_millis().to_string()),
            last_updated_at: Some(Utc::now().timestamp_millis().to_string()),
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
            is_scryer_origin,
            source_provider: None,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
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
    fn dedupe_download_queue_items_keeps_same_native_id_from_different_clients() {
        let mut first = item("job-1", DownloadQueueState::Completed, true);
        first.client_id = "client-1".to_string();
        let mut second = item("job-1", DownloadQueueState::Completed, true);
        second.client_id = "client-2".to_string();

        let deduped = dedupe_download_queue_items(vec![first, second]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].client_id, "client-1");
        assert_eq!(deduped[1].client_id, "client-2");
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

        let filtered = filter_download_queue_items(
            items,
            false,
            false,
            false,
            None,
            DownloadActivityFilterValue::All,
        );

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

        let filtered = filter_download_queue_items(
            items,
            false,
            false,
            false,
            None,
            DownloadActivityFilterValue::All,
        );

        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|item| item.is_scryer_origin));
    }

    #[test]
    fn filter_download_queue_items_respects_title_filter() {
        let mut matching = item("job-1", DownloadQueueState::Queued, true);
        matching.title_id = Some("title-1".to_string());
        let mut other = item("job-2", DownloadQueueState::Queued, true);
        other.title_id = Some("title-2".to_string());

        let filtered = filter_download_queue_items(
            vec![matching, other],
            true,
            false,
            false,
            Some("title-1"),
            DownloadActivityFilterValue::All,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title_id.as_deref(), Some("title-1"));
    }

    #[test]
    fn filter_download_queue_items_can_include_import_activity() {
        let item = item("job-1", DownloadQueueState::ImportPending, true);

        let filtered = filter_download_queue_items(
            vec![item],
            false,
            false,
            true,
            None,
            DownloadActivityFilterValue::All,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].state, DownloadQueueState::ImportPending);
    }
}
