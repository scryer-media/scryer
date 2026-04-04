use super::*;
use crate::domain_events::{
    new_download_queue_domain_event, new_global_domain_event, new_title_domain_event,
    title_context_snapshot,
};
use crate::event_views::{
    activity_event_from_domain_event, history_event_from_domain_event, replay_library_scan_state,
    title_history_records_from_domain_event,
};
use scryer_domain::{
    AcquisitionCandidateRejectedEventData, AcquisitionSearchCompletedEventData,
    ConfigurationChangeAction, ConfigurationChangedEventData, DiscoverySearchCompletedEventData,
    DomainEventPayload, DownloadQueueCommandAction, DownloadQueueItemCommandIssuedEventData,
    ImportRecoveryCompletedEventData, ImportRequestKind, ImportRequestedEventData,
    MetadataHydrationState, MetadataHydrationUpdatedEventData, PostProcessingCompletedEventData,
    PostProcessingResult, SubtitleDownloadedEventData, SubtitleSearchFailedEventData,
    TitleUpdatedEventData,
};

async fn load_all_domain_events(
    app: &AppUseCase,
    mut filter: DomainEventFilter,
) -> AppResult<Vec<DomainEvent>> {
    let mut events = Vec::new();
    let mut after_sequence = 0i64;

    loop {
        filter.after_sequence = Some(after_sequence);
        filter.limit = 500;
        let batch = app.services.domain_events.list(&filter).await?;
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

    Ok(events)
}

async fn load_recent_projected_domain_events<T, F>(
    app: &AppUseCase,
    mut filter: DomainEventFilter,
    target_len: usize,
    mut map: F,
) -> AppResult<Vec<T>>
where
    F: FnMut(&DomainEvent) -> Option<T>,
{
    if target_len == 0 {
        return Ok(Vec::new());
    }

    let mut projected = Vec::new();
    let mut before_sequence = None;

    loop {
        filter.after_sequence = None;
        filter.before_sequence = before_sequence;
        filter.limit = 500;

        let batch = app.services.domain_events.list(&filter).await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            if let Some(item) = map(event) {
                projected.push(item);
                if projected.len() >= target_len {
                    return Ok(projected);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(projected)
}

async fn load_active_library_scan_projection(
    app: &AppUseCase,
) -> AppResult<Vec<LibraryScanSession>> {
    let events = load_all_domain_events(
        app,
        DomainEventFilter {
            event_types: Some(vec![
                DomainEventType::LibraryScanStarted,
                DomainEventType::LibraryScanTitleDiscovered,
                DomainEventType::LibraryScanProgressed,
                DomainEventType::LibraryScanCompleted,
                DomainEventType::LibraryScanFailed,
            ]),
            ..DomainEventFilter::default()
        },
    )
    .await?;
    let mut sessions = replay_library_scan_state(&events)
        .into_values()
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.started_at.cmp(&right.started_at));
    Ok(sessions)
}

const TITLE_HISTORY_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRenamed,
];

fn title_history_record_matches(
    record: &TitleHistoryRecord,
    filter: &TitleHistoryFilter,
    episode_id: Option<&str>,
) -> bool {
    filter
        .event_types
        .as_ref()
        .is_none_or(|event_types| event_types.contains(&record.event_type))
        && filter
            .title_ids
            .as_ref()
            .is_none_or(|title_ids| title_ids.contains(&record.title_id))
        && filter
            .download_id
            .as_ref()
            .is_none_or(|download_id| record.download_id.as_deref() == Some(download_id))
        && episode_id.is_none_or(|expected| record.episode_id.as_deref() == Some(expected))
}

async fn project_title_history_page(
    app: &AppUseCase,
    filter: &TitleHistoryFilter,
    episode_id: Option<&str>,
) -> AppResult<TitleHistoryPage> {
    let mut domain_filter = DomainEventFilter {
        title_id: filter
            .title_ids
            .as_ref()
            .and_then(|title_ids| (title_ids.len() == 1).then(|| title_ids[0].clone())),
        event_types: Some(TITLE_HISTORY_DOMAIN_EVENT_TYPES.to_vec()),
        ..DomainEventFilter::default()
    };
    let limit = filter.limit.max(1);
    let mut before_sequence = None;
    let mut total_count = 0i64;
    let mut records = Vec::new();

    loop {
        domain_filter.after_sequence = None;
        domain_filter.before_sequence = before_sequence;
        domain_filter.limit = 500;

        let batch = app.services.domain_events.list(&domain_filter).await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            for record in title_history_records_from_domain_event(event) {
                if !title_history_record_matches(&record, filter, episode_id) {
                    continue;
                }

                let current_index = total_count as usize;
                total_count += 1;
                if current_index >= filter.offset && records.len() < limit {
                    records.push(record);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(TitleHistoryPage {
        records,
        total_count,
    })
}

async fn project_episode_title_history(
    app: &AppUseCase,
    episode_id: &str,
    limit: usize,
) -> AppResult<Vec<TitleHistoryRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut domain_filter = DomainEventFilter {
        event_types: Some(TITLE_HISTORY_DOMAIN_EVENT_TYPES.to_vec()),
        ..DomainEventFilter::default()
    };
    let mut before_sequence = None;
    let mut records = Vec::new();

    loop {
        domain_filter.after_sequence = None;
        domain_filter.before_sequence = before_sequence;
        domain_filter.limit = 500;

        let batch = app.services.domain_events.list(&domain_filter).await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            for record in title_history_records_from_domain_event(event) {
                if record.episode_id.as_deref() != Some(episode_id) {
                    continue;
                }

                records.push(record);
                if records.len() >= limit {
                    return Ok(records);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(records)
}

impl AppUseCase {
    /// Canonical reactive bus event for title-list/detail refresh. Flows that
    /// change title-visible UI state should emit this instead of open-coding
    /// scan- or workflow-specific refresh signals.
    pub(crate) async fn emit_title_updated_activity(
        &self,
        actor_user_id: Option<String>,
        title: &Title,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                actor_user_id,
                title,
                DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
                    title: title_context_snapshot(title),
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append title updated domain event"
            );
        }
    }

    pub async fn emit_configuration_changed_event(
        &self,
        actor_user_id: Option<String>,
        resource_type: impl Into<String>,
        resource_id: Option<String>,
        action: ConfigurationChangeAction,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_global_domain_event(
                actor_user_id,
                DomainEventPayload::ConfigurationChanged(ConfigurationChangedEventData {
                    resource_type: resource_type.into(),
                    resource_id,
                    action,
                }),
            ))
            .await
        {
            tracing::warn!(error = %error, "failed to append configuration changed domain event");
        }
    }

    pub(crate) async fn emit_discovery_search_completed_event(
        &self,
        actor_user_id: Option<String>,
        search_type: impl Into<String>,
        query: Option<String>,
        result_count: i64,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_global_domain_event(
                actor_user_id,
                DomainEventPayload::DiscoverySearchCompleted(DiscoverySearchCompletedEventData {
                    search_type: search_type.into(),
                    query,
                    result_count,
                }),
            ))
            .await
        {
            tracing::warn!(error = %error, "failed to append discovery search domain event");
        }
    }

    pub(crate) async fn emit_metadata_hydration_updated_event(
        &self,
        title: &Title,
        state: MetadataHydrationState,
        reason: Option<String>,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MetadataHydrationUpdated(MetadataHydrationUpdatedEventData {
                    title: title_context_snapshot(title),
                    state,
                    reason,
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append metadata hydration domain event"
            );
        }
    }

    pub(crate) async fn emit_acquisition_search_completed_event(
        &self,
        actor_user_id: Option<String>,
        title: &Title,
        result_count: i64,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                actor_user_id,
                title,
                DomainEventPayload::AcquisitionSearchCompleted(
                    AcquisitionSearchCompletedEventData {
                        title: title_context_snapshot(title),
                        result_count,
                    },
                ),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append acquisition search domain event"
            );
        }
    }

    pub(crate) async fn emit_acquisition_candidate_rejected_event(
        &self,
        actor_user_id: Option<String>,
        title: &Title,
        source_title: impl Into<String>,
        reason_code: impl Into<String>,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                actor_user_id,
                title,
                DomainEventPayload::AcquisitionCandidateRejected(
                    AcquisitionCandidateRejectedEventData {
                        title: title_context_snapshot(title),
                        source_title: source_title.into(),
                        reason_code: reason_code.into(),
                    },
                ),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append acquisition candidate rejected domain event"
            );
        }
    }

    pub(crate) async fn emit_import_requested_event(
        &self,
        actor_user_id: Option<String>,
        title: Option<&Title>,
        client_type: impl Into<String>,
        source_ref: impl Into<String>,
        request_kind: ImportRequestKind,
    ) {
        let client_type = client_type.into();
        let source_ref = source_ref.into();
        let payload = DomainEventPayload::ImportRequested(ImportRequestedEventData {
            title: title.map(title_context_snapshot),
            client_type: client_type.clone(),
            source_ref: source_ref.clone(),
            request_kind,
        });

        let result = match title {
            Some(title) => {
                self.services
                    .append_domain_event(new_title_domain_event(actor_user_id, title, payload))
                    .await
            }
            None => {
                self.services
                    .append_domain_event(new_global_domain_event(actor_user_id, payload))
                    .await
            }
        };

        if let Err(error) = result {
            tracing::warn!(error = %error, client_type, source_ref, "failed to append import requested domain event");
        }
    }

    pub(crate) async fn emit_import_recovery_completed_event(
        &self,
        actor_user_id: Option<String>,
        recovered_count: i64,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_global_domain_event(
                actor_user_id,
                DomainEventPayload::ImportRecoveryCompleted(ImportRecoveryCompletedEventData {
                    recovered_count,
                }),
            ))
            .await
        {
            tracing::warn!(error = %error, recovered_count, "failed to append import recovery domain event");
        }
    }

    pub(crate) async fn emit_download_queue_item_command_issued_event(
        &self,
        actor_user_id: Option<String>,
        item_id: impl Into<String>,
        action: DownloadQueueCommandAction,
    ) {
        let item_id = item_id.into();
        if let Err(error) = self
            .services
            .append_domain_event(new_download_queue_domain_event(
                actor_user_id,
                item_id.clone(),
                DomainEventPayload::DownloadQueueItemCommandIssued(
                    DownloadQueueItemCommandIssuedEventData {
                        item_id: item_id.clone(),
                        action,
                    },
                ),
            ))
            .await
        {
            tracing::warn!(error = %error, item_id, "failed to append download queue command domain event");
        }
    }

    pub(crate) async fn emit_post_processing_completed_event(
        &self,
        actor_user_id: Option<String>,
        title: &Title,
        script_name: impl Into<String>,
        result: PostProcessingResult,
        exit_code: Option<i32>,
    ) {
        let script_name = script_name.into();
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                actor_user_id,
                title,
                DomainEventPayload::PostProcessingCompleted(PostProcessingCompletedEventData {
                    title: title_context_snapshot(title),
                    script_name: script_name.clone(),
                    result,
                    exit_code,
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                script_name,
                "failed to append post-processing domain event"
            );
        }
    }

    pub(crate) async fn emit_subtitle_downloaded_event(
        &self,
        title: &Title,
        subtitle_path: Option<String>,
        language: Option<String>,
        provider: Option<String>,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::SubtitleDownloaded(SubtitleDownloadedEventData {
                    title: title_context_snapshot(title),
                    subtitle_path,
                    language,
                    provider,
                }),
            ))
            .await
        {
            tracing::warn!(title_id = %title.id, error = %error, "failed to append subtitle downloaded domain event");
        }
    }

    pub(crate) async fn emit_subtitle_search_failed_event(
        &self,
        title: &Title,
        language: Option<String>,
        reason: Option<String>,
    ) {
        if let Err(error) = self
            .services
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::SubtitleSearchFailed(SubtitleSearchFailedEventData {
                    title: title_context_snapshot(title),
                    language,
                    reason,
                }),
            ))
            .await
        {
            tracing::warn!(title_id = %title.id, error = %error, "failed to append subtitle search failed domain event");
        }
    }

    pub async fn evaluate_policy(
        &self,
        actor: &User,
        input: PolicyInput,
    ) -> AppResult<PolicyOutput> {
        require(actor, &Entitlement::ViewHistory)?;

        let mut reason_codes = vec!["default_policy_evaluation".to_string()];
        if input.has_existing_file {
            reason_codes.push("existing_file_present".to_string());
        }

        let score = if input.requested_mode == scryer_domain::RequestedMode::Manual {
            100.0
        } else {
            80.0
        };

        Ok(PolicyOutput {
            decision: true,
            score,
            reason_codes,
            explanation: format!(
                "policy evaluation for title {} in {} mode",
                input.title_id,
                input.requested_mode.as_str()
            ),
            scoring_log: vec![],
        })
    }

    pub async fn recent_events(
        &self,
        actor: &User,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<HistoryEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        let offset = offset.max(0) as usize;
        let limit = limit.max(1) as usize;
        let history = load_recent_projected_domain_events(
            self,
            DomainEventFilter {
                title_id,
                ..DomainEventFilter::default()
            },
            offset.saturating_add(limit),
            history_event_from_domain_event,
        )
        .await?;
        Ok(history.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn recent_activity(
        &self,
        actor: &User,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ActivityEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        let offset = offset.max(0) as usize;
        let limit = limit.max(1) as usize;
        let activities = load_recent_projected_domain_events(
            self,
            DomainEventFilter::default(),
            offset.saturating_add(limit),
            activity_event_from_domain_event,
        )
        .await?;
        Ok(activities.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn list_domain_events(
        &self,
        actor: &User,
        filter: &DomainEventFilter,
    ) -> AppResult<Vec<DomainEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services.domain_events.list(filter).await
    }

    pub async fn list_activity_events_after_sequence(
        &self,
        actor: &User,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<(i64, ActivityEvent)>> {
        require(actor, &Entitlement::ViewHistory)?;
        let events = self
            .services
            .domain_events
            .list(&DomainEventFilter {
                after_sequence: Some(after_sequence),
                limit: limit.max(1),
                ..DomainEventFilter::default()
            })
            .await?;
        Ok(events
            .into_iter()
            .filter_map(|event| {
                activity_event_from_domain_event(&event).map(|activity| (event.sequence, activity))
            })
            .collect())
    }

    pub fn subscribe_activity_events(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<ActivityEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let mut wake_rx = app.services.domain_event_broadcast.subscribe();
            let mut cursor = 0_i64;

            loop {
                match app
                    .list_activity_events_after_sequence(&actor, cursor, 100)
                    .await
                {
                    Ok(events) if !events.is_empty() => {
                        for (sequence, event) in events {
                            cursor = sequence;
                            if tx.send(event).is_err() {
                                return;
                            }
                        }
                        continue;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!("activity subscription replay failed: {error}");
                        break;
                    }
                }

                match wake_rx.recv().await {
                    Ok(sequence) => {
                        if sequence > cursor {
                            cursor = sequence.saturating_sub(1);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("activity subscription lagged, skipped {n} wakeups");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub fn subscribe_domain_event_sequences(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<i64>> {
        require(actor, &Entitlement::ViewHistory)?;
        Ok(self.services.domain_event_broadcast.subscribe())
    }

    pub fn subscribe_import_history(&self, actor: &User) -> AppResult<broadcast::Receiver<()>> {
        require(actor, &Entitlement::ViewHistory)?;
        Ok(self.services.import_history_broadcast.subscribe())
    }

    pub async fn active_library_scans(&self, actor: &User) -> AppResult<Vec<LibraryScanSession>> {
        require(actor, &Entitlement::ViewCatalog)?;
        let sessions = self.services.library_scan_tracker.list_active().await;
        if sessions.is_empty() {
            load_active_library_scan_projection(self).await
        } else {
            Ok(sessions)
        }
    }

    pub fn subscribe_library_scan_progress(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<LibraryScanSession>> {
        require(actor, &Entitlement::ViewCatalog)?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        tokio::spawn(async move {
            let mut receiver = app.services.library_scan_tracker.subscribe();
            let mut initial_sessions = app.services.library_scan_tracker.list_active().await;
            if initial_sessions.is_empty() {
                initial_sessions = match load_active_library_scan_projection(&app).await {
                    Ok(sessions) => sessions,
                    Err(error) => {
                        tracing::warn!("library scan subscription initial load failed: {error}");
                        return;
                    }
                };
            }
            for session in initial_sessions {
                if tx.send(session).is_err() {
                    return;
                }
            }

            loop {
                match receiver.recv().await {
                    Ok(session) => {
                        if tx.send(session).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "library scan subscription lagged, skipped {n} tracker updates"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub fn subscribe_library_scan_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<LibraryScanSession>> {
        self.subscribe_library_scan_progress(actor)
    }

    pub fn subscribe_settings_changed(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<String>>> {
        require(actor, &Entitlement::ViewCatalog)?;
        Ok(self.services.settings_changed_broadcast.subscribe())
    }

    pub fn subscribe_download_queue_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        self.subscribe_download_queue(actor)
    }

    pub fn subscribe_job_run_state(&self, actor: &User) -> AppResult<broadcast::Receiver<JobRun>> {
        self.subscribe_job_run_events(actor)
    }

    pub async fn list_title_history(
        &self,
        actor: &User,
        filter: &TitleHistoryFilter,
    ) -> AppResult<TitleHistoryPage> {
        require(actor, &Entitlement::ViewHistory)?;
        project_title_history_page(self, filter, None).await
    }

    pub async fn list_title_history_for_title(
        &self,
        actor: &User,
        title_id: &str,
        event_types: Option<&[TitleHistoryEventType]>,
        limit: usize,
        offset: usize,
    ) -> AppResult<TitleHistoryPage> {
        require(actor, &Entitlement::ViewHistory)?;
        project_title_history_page(
            self,
            &TitleHistoryFilter {
                event_types: event_types.map(|types| types.to_vec()),
                title_ids: Some(vec![title_id.to_string()]),
                download_id: None,
                limit,
                offset,
            },
            None,
        )
        .await
    }

    pub async fn list_title_history_for_episode(
        &self,
        actor: &User,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        require(actor, &Entitlement::ViewHistory)?;
        project_episode_title_history(self, episode_id, limit).await
    }
}
