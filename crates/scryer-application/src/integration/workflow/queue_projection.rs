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
    if !item.import_status.is_some_and(ImportStatus::is_active) {
        for message in &item.tracked_status_messages {
            push_queue_status_detail(&mut values, &mut seen, Some(message));
        }
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
        Some(ImportStatus::Pending) => return DownloadDisplayState::ImportPending,
        Some(ImportStatus::Running | ImportStatus::Processing) => {
            return DownloadDisplayState::Importing;
        }
        Some(ImportStatus::Completed) if item.state != DownloadQueueState::Warning => {
            return if item.tracked_state == Some(TrackedDownloadState::ImportedSeeding) {
                DownloadDisplayState::ImportedSeeding
            } else {
                DownloadDisplayState::Completed
            };
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

    if item.tracked_state == Some(TrackedDownloadState::ImportedSeeding)
        && item.state != DownloadQueueState::Warning
    {
        return DownloadDisplayState::ImportedSeeding;
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
        // Warning is checked last on purpose: an import overlay or a tracked
        // block is the more specific answer, and the client's recoverable
        // problem must not preempt it the way `Failed` does above.
        DownloadQueueState::Warning => DownloadDisplayState::Warning,
        DownloadQueueState::Failed => DownloadDisplayState::Failed,
    }
}
fn bucket_for_base_display_state(state: DownloadDisplayState) -> DownloadQueueBucket {
    match state {
        DownloadDisplayState::Queued
        | DownloadDisplayState::Downloading
        | DownloadDisplayState::Paused
        | DownloadDisplayState::PostProcessing
        | DownloadDisplayState::ImportedSeeding
        // A warned download is still live in the client and still recoverable,
        // so it belongs with the activity it is part of, not in history.
        | DownloadDisplayState::Warning => DownloadQueueBucket::Activity,
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
/// How a torrent's seeding obligation reads to an operator looking at the
/// queue.
///
/// Derived from the same gate that decides whether the entry may be removed, so
/// the badge and the reconciler can never disagree: every variant is one of the
/// gate's outcomes, collapsed to what is worth showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadSeedingState {
    /// A torrent with nothing to report yet — still downloading, or a client
    /// that exposes no seeding state at all.
    None,
    /// Seeding towards an obligation that is not discharged.
    Seeding,
    /// The obligation is discharged; the entry is free to be acted on.
    GoalMet,
    /// Observed private with no resolved goals: never auto-removed, because
    /// Scryer has no goal it could prove and a private tracker counts an early
    /// removal as a hit and run.
    HeldPrivate,
    /// The profile says seed forever.
    NeverRemove,
}

/// The seeding state to show for one queue row, or `None` when the row has no
/// torrent seeding information at all (usenet, and clients that report none).
///
/// Reads only what is already on the item: the adapter's observation and the
/// goals the queue enrichment joined from the persisted grab-time resolution.
pub fn derive_download_seeding_state(item: &DownloadQueueItem) -> Option<DownloadSeedingState> {
    let snapshot = item.seeding.as_ref()?;
    let parked_by_the_gate = item.tracked_state == Some(TrackedDownloadState::ImportedSeeding);
    if !snapshot.has_torrent_signal() && !parked_by_the_gate {
        // `can_remove`/`can_move_files` alone are reported by usenet clients
        // too, so they are not evidence that this is a torrent.
        return None;
    }

    // Before the payload is complete there is no seeding to report, and a
    // client's `can_remove: Some(false)` on a half-downloaded torrent means
    // "not finished", not "still seeding".
    let payload_complete = parked_by_the_gate
        || item.progress_percent >= 100
        || matches!(
            item.state,
            DownloadQueueState::Completed | DownloadQueueState::ImportPending
        );
    if !payload_complete {
        return Some(DownloadSeedingState::None);
    }

    let decision = crate::seeding_gate::evaluate_seeding_gate(&crate::seeding_gate::SeedingGateInput {
        is_torrent: true,
        client_type: item.client_type.clone(),
        present_in_client: true,
        observation: crate::seeding_gate::observation_from_queue_item(item),
        goals: Some(crate::PersistedSeedGoals {
            seed_goal_ratio: snapshot.seed_goal_ratio,
            seed_goal_seconds: snapshot.seed_goal_seconds,
            never_remove: snapshot.never_remove,
            ..crate::PersistedSeedGoals::default()
        }),
        now: chrono::Utc::now(),
    });

    use crate::seeding_gate::reason;
    Some(match decision.reason {
        reason::PROFILE_NEVER_REMOVE => DownloadSeedingState::NeverRemove,
        reason::PRIVATE_WITHOUT_GOALS => DownloadSeedingState::HeldPrivate,
        reason::PROFILE_GOAL_UNMET | reason::CLIENT_LIMIT_UNMET => DownloadSeedingState::Seeding,
        reason::PROFILE_GOAL_MET | reason::CLIENT_OBLIGATION_MET => DownloadSeedingState::GoalMet,
        // A client that cannot answer and a profile with nothing to prove:
        // the gate holds, and the row is genuinely still seeding as far as
        // anyone can tell.
        reason::CLIENT_VERDICT_UNKNOWN => DownloadSeedingState::Seeding,
        // Blackhole has no session to report on, and a vanished entry is not
        // in the queue to begin with.
        _ => DownloadSeedingState::None,
    })
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
        DownloadDisplayState::ImportedSeeding => Some(DownloadActivityFilter::Seeding),
        // Every activity state needs its own filter: the queue page asks for an
        // explicit list of them, so a state that belongs to no filter is a
        // state the operator can never see.
        DownloadDisplayState::Warning => Some(DownloadActivityFilter::Warning),
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
        DownloadImportFilter::Attention => matches!(
            classified.import_filter,
            Some(
                DownloadImportFilter::Pending
                    | DownloadImportFilter::Blocked
                    | DownloadImportFilter::Failed
            )
        ),
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
