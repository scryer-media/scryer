use crate::domain_events::DomainEventActor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadQueueBucket {
    Activity,
    Import,
    HistorySuccess,
    HistoryFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClassifiedDownloadQueueItem {
    display_state: DownloadDisplayState,
    bucket: DownloadQueueBucket,
    activity_filter: Option<DownloadActivityFilter>,
    import_filter: Option<DownloadImportFilter>,
    history_filter: Option<DownloadHistoryFilter>,
}
fn push_queue_status_detail(
    values: &mut Vec<String>,
    seen: &mut HashSet<String>,
    raw: Option<&str>,
) {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if seen.insert(value.to_string()) {
        values.push(value.to_string());
    }
}
fn build_download_queue_status_detail(item: &DownloadQueueItem) -> String {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    for message in &item.tracked_status_messages {
        push_queue_status_detail(&mut values, &mut seen, Some(message));
    }
    push_queue_status_detail(&mut values, &mut seen, item.attention_reason.as_deref());
    push_queue_status_detail(&mut values, &mut seen, item.delete_error_message.as_deref());
    push_queue_status_detail(&mut values, &mut seen, item.import_error_message.as_deref());
    values.join("\n")
}
fn base_download_queue_display_state(item: &DownloadQueueItem) -> DownloadDisplayState {
    if item.tracked_state == Some(TrackedDownloadState::Ignored) {
        return DownloadDisplayState::Ignored;
    }

    if item.state == DownloadQueueState::Failed {
        return DownloadDisplayState::Failed;
    }

    match item.import_status {
        Some(ImportStatus::Pending | ImportStatus::Running | ImportStatus::Processing) => {
            return DownloadDisplayState::Importing;
        }
        Some(ImportStatus::Failed | ImportStatus::Skipped)
            if matches!(
                item.tracked_state,
                Some(TrackedDownloadState::ImportBlocked)
            ) || matches!(
                item.state,
                DownloadQueueState::Completed
                    | DownloadQueueState::ImportPending
                    | DownloadQueueState::Failed
            ) =>
        {
            return DownloadDisplayState::ImportFailed;
        }
        _ => {}
    }

    match item.tracked_state {
        Some(TrackedDownloadState::ImportBlocked) => return DownloadDisplayState::ImportBlocked,
        Some(TrackedDownloadState::ImportPending) => return DownloadDisplayState::ImportPending,
        _ => {}
    }

    let failure_reason = build_download_queue_status_detail(item);
    let can_derive_blocked_state = item.tracked_state.is_none()
        && !failure_reason.is_empty()
        && matches!(
            item.state,
            DownloadQueueState::Completed | DownloadQueueState::ImportPending
        )
        && matches!(
            item.import_status,
            Some(ImportStatus::Skipped | ImportStatus::Failed)
        );
    if can_derive_blocked_state {
        return DownloadDisplayState::ImportBlocked;
    }

    match item.state {
        DownloadQueueState::Queued => DownloadDisplayState::Queued,
        DownloadQueueState::Downloading => {
            if is_post_processing_reason(item.attention_reason.as_deref()) {
                DownloadDisplayState::PostProcessing
            } else {
                DownloadDisplayState::Downloading
            }
        }
        DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => DownloadDisplayState::PostProcessing,
        DownloadQueueState::Paused => DownloadDisplayState::Paused,
        DownloadQueueState::Completed => DownloadDisplayState::Completed,
        DownloadQueueState::ImportPending => DownloadDisplayState::ImportPending,
        DownloadQueueState::Failed => DownloadDisplayState::Failed,
    }
}
fn bucket_for_base_display_state(state: DownloadDisplayState) -> DownloadQueueBucket {
    match state {
        DownloadDisplayState::Queued
        | DownloadDisplayState::Downloading
        | DownloadDisplayState::Paused
        | DownloadDisplayState::PostProcessing => DownloadQueueBucket::Activity,
        DownloadDisplayState::Importing
        | DownloadDisplayState::ImportPending
        | DownloadDisplayState::ImportBlocked
        | DownloadDisplayState::ImportFailed => DownloadQueueBucket::Import,
        DownloadDisplayState::Completed | DownloadDisplayState::Ignored => {
            DownloadQueueBucket::HistorySuccess
        }
        DownloadDisplayState::Failed => DownloadQueueBucket::HistoryFailed,
        DownloadDisplayState::Removing | DownloadDisplayState::RemoveFailed => {
            DownloadQueueBucket::HistoryFailed
        }
    }
}
pub fn derive_download_queue_display_state(item: &DownloadQueueItem) -> DownloadDisplayState {
    let base_state = base_download_queue_display_state(item);
    match item.delete_status {
        Some(DownloadQueueDeleteStatus::Queued | DownloadQueueDeleteStatus::Running) => {
            DownloadDisplayState::Removing
        }
        Some(DownloadQueueDeleteStatus::Failed) => DownloadDisplayState::RemoveFailed,
        _ => base_state,
    }
}
fn classify_download_queue_item(item: &DownloadQueueItem) -> ClassifiedDownloadQueueItem {
    let base_state = base_download_queue_display_state(item);
    let base_bucket = bucket_for_base_display_state(base_state);
    let display_state = derive_download_queue_display_state(item);

    let bucket = match (base_bucket, display_state) {
        (DownloadQueueBucket::Import, DownloadDisplayState::RemoveFailed)
        | (DownloadQueueBucket::Activity, DownloadDisplayState::RemoveFailed) => base_bucket,
        (_, DownloadDisplayState::RemoveFailed) => DownloadQueueBucket::HistoryFailed,
        _ => base_bucket,
    };

    let activity_filter = match base_state {
        DownloadDisplayState::Downloading => Some(DownloadActivityFilter::Downloading),
        DownloadDisplayState::Queued => Some(DownloadActivityFilter::Queued),
        DownloadDisplayState::Paused => Some(DownloadActivityFilter::Paused),
        DownloadDisplayState::PostProcessing => Some(DownloadActivityFilter::PostProcessing),
        _ => None,
    };

    let import_filter = match base_state {
        DownloadDisplayState::Importing => Some(DownloadImportFilter::Importing),
        DownloadDisplayState::ImportPending => Some(DownloadImportFilter::Pending),
        DownloadDisplayState::ImportBlocked => Some(DownloadImportFilter::Blocked),
        DownloadDisplayState::ImportFailed => Some(DownloadImportFilter::Failed),
        _ => None,
    };

    let history_filter = match bucket {
        DownloadQueueBucket::HistorySuccess => Some(DownloadHistoryFilter::Success),
        DownloadQueueBucket::HistoryFailed => Some(DownloadHistoryFilter::Failed),
        _ => None,
    };

    ClassifiedDownloadQueueItem {
        display_state,
        bucket,
        activity_filter,
        import_filter,
        history_filter,
    }
}
pub fn matches_download_activity_filter(
    item: &DownloadQueueItem,
    filter: DownloadActivityFilter,
) -> bool {
    let classified = classify_download_queue_item(item);
    if classified.bucket != DownloadQueueBucket::Activity {
        return false;
    }

    match filter {
        DownloadActivityFilter::All => true,
        _ => classified.activity_filter == Some(filter),
    }
}
pub fn matches_download_queue_filter(
    item: &DownloadQueueItem,
    include_history_only: bool,
    include_import_activity: bool,
    activity_filter: DownloadActivityFilter,
) -> bool {
    let classified = classify_download_queue_item(item);

    if include_history_only {
        return matches!(
            classified.bucket,
            DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
        );
    }

    match classified.bucket {
        DownloadQueueBucket::Activity => match activity_filter {
            DownloadActivityFilter::All => true,
            _ => classified.activity_filter == Some(activity_filter),
        },
        DownloadQueueBucket::Import => {
            include_import_activity
                && matches!(
                    classified.import_filter,
                    Some(DownloadImportFilter::Importing | DownloadImportFilter::Pending)
                )
        }
        DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed => false,
    }
}
fn matches_download_import_filter(item: &DownloadQueueItem, filter: DownloadImportFilter) -> bool {
    let classified = classify_download_queue_item(item);
    if classified.bucket != DownloadQueueBucket::Import {
        return false;
    }

    match filter {
        DownloadImportFilter::All => true,
        _ => classified.import_filter == Some(filter),
    }
}
fn matches_download_history_filters(
    item: &DownloadQueueItem,
    filters: Option<&[DownloadHistoryFilter]>,
) -> bool {
    let classified = classify_download_queue_item(item);
    if !matches!(
        classified.bucket,
        DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
    ) {
        return false;
    }

    match filters {
        None => true,
        Some([]) => false,
        Some(filters) if filters.contains(&DownloadHistoryFilter::All) => true,
        Some(filters) => classified
            .history_filter
            .is_some_and(|filter| filters.contains(&filter)),
    }
}
fn download_history_status_rank(item: &DownloadQueueItem) -> u8 {
    match classify_download_queue_item(item).bucket {
        DownloadQueueBucket::HistorySuccess => 0,
        DownloadQueueBucket::HistoryFailed => 1,
        _ => u8::MAX,
    }
}
fn annotate_download_queue_item(
    mut item: DownloadQueueItem,
    primary_client: Option<&DownloadClientConfig>,
) -> DownloadQueueItem {
    if let Some(primary_client) = primary_client {
        if item.client_id.is_empty() {
            item.client_id = primary_client.id.clone();
        }
        if item.client_name.is_empty() {
            item.client_name = primary_client.name.clone();
        }
        if item.client_type.is_empty() {
            item.client_type = primary_client.client_type.clone();
        }
    }
    item.attention_required = matches!(
        classify_download_queue_item(&item).bucket,
        DownloadQueueBucket::Import | DownloadQueueBucket::HistoryFailed
    );
    if item.attention_reason.is_none() {
        item.attention_reason = if item.attention_required {
            Some("requires attention".to_string())
        } else {
            None
        };
    }
    item
}
fn download_queue_projection_key(item: &DownloadQueueItem) -> String {
    if item.client_id.trim().is_empty() {
        return format!("{}::{}", item.client_type, item.download_client_item_id);
    }

    format!("{}::{}", item.client_id, item.download_client_item_id)
}
pub async fn publish_download_queue_snapshot_events(
    app: &AppUseCase,
    actor: impl Into<DomainEventActor>,
    previous_items: &mut HashMap<String, DownloadQueueItem>,
    items: &[DownloadQueueItem],
) {
    let actor = actor.into();
    let mut next_items = HashMap::with_capacity(items.len());
    let mut domain_events = Vec::new();

    for item in items {
        let key = download_queue_projection_key(item);
        let changed = previous_items
            .get(&key)
            .is_none_or(|previous| previous != item);
        if changed {
            domain_events.push(new_download_queue_domain_event(
                actor.clone(),
                key.clone(),
                DomainEventPayload::DownloadQueueItemUpserted(DownloadQueueItemUpsertedEventData {
                    item: item.clone(),
                }),
            ));
        }
        next_items.insert(key, item.clone());
    }

    for (key, previous_item) in previous_items.iter() {
        if !next_items.contains_key(key) {
            domain_events.push(new_download_queue_domain_event(
                actor.clone(),
                key.clone(),
                DomainEventPayload::DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData {
                    download_client_item_id: previous_item.download_client_item_id.clone(),
                    client_id: Some(previous_item.client_id.clone())
                        .filter(|value| !value.trim().is_empty()),
                    client_type: Some(previous_item.client_type.clone()),
                }),
            ));
        }
    }

    *previous_items = next_items;

    if !domain_events.is_empty()
        && let Err(error) = app.append_domain_events(domain_events).await
    {
        tracing::warn!(error = %error, "failed to append download queue domain events");
    }
}

pub async fn publish_download_queue_upsert_events(
    app: &AppUseCase,
    actor: impl Into<DomainEventActor>,
    items: &[DownloadQueueItem],
) {
    let actor = actor.into();
    let domain_events = items
        .iter()
        .map(|item| {
            new_download_queue_domain_event(
                actor.clone(),
                download_queue_projection_key(item),
                DomainEventPayload::DownloadQueueItemUpserted(DownloadQueueItemUpsertedEventData {
                    item: item.clone(),
                }),
            )
        })
        .collect::<Vec<_>>();

    if !domain_events.is_empty()
        && let Err(error) = app.append_domain_events(domain_events).await
    {
        tracing::warn!(error = %error, "failed to append download queue upsert events");
    }
}
