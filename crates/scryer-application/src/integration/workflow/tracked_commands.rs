const TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET: Duration = Duration::from_millis(25);
const TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT: usize = 1;
const DOWNLOAD_QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Poll cadence for download-queue snapshots.
///
/// Every tick collects queue AND recent history together, so this is also the
/// worst-case detection latency for a completion. There is no longer a separate
/// history cadence to tune — history is not a slower reconciliation pass, it is
/// half of the primary read.
///
/// Defaults to `DOWNLOAD_QUEUE_POLL_INTERVAL`. The
/// `SCRYER_DOWNLOAD_QUEUE_POLL_INTERVAL_SECS` environment variable exists solely so
/// the e2e harness can shrink the interval; production leaves it unset and keeps
/// the default, so production timing is unchanged. Resolved once and cached for the
/// process lifetime rather than re-read on every poll tick.
fn download_queue_poll_interval() -> Duration {
    static CACHED: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let raw = std::env::var("SCRYER_DOWNLOAD_QUEUE_POLL_INTERVAL_SECS").ok();
        parse_poll_secs(raw.as_deref(), DOWNLOAD_QUEUE_POLL_INTERVAL)
    })
}

/// Parse a poll-interval override expressed in whole seconds.
///
/// Returns `default` when `raw` is unset, blank, unparsable, or zero so that a
/// missing or malformed override never alters production timing; the minimum
/// honored override is one second.
fn parse_poll_secs(raw: Option<&str>, default: Duration) -> Duration {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs >= 1)
        .map(Duration::from_secs)
        .unwrap_or(default)
}

/// How far back the bridge-covered history reconciliation sweep will reach.
///
/// The sweep heals completions whose realtime event was missed, which is
/// noticed within minutes; the bound keeps an upgrade from mass-importing a
/// download client's entire retained history on its first cycle. The
/// `SCRYER_DOWNLOAD_QUEUE_RECONCILE_MAX_AGE_HOURS` override exists for the e2e
/// harness and for operators performing a deliberate wider backfill.
const DOWNLOAD_QUEUE_RECONCILE_MAX_AGE_HOURS: i64 = 24;

fn reconcile_history_max_age() -> chrono::Duration {
    static CACHED: std::sync::OnceLock<chrono::Duration> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let hours = std::env::var("SCRYER_DOWNLOAD_QUEUE_RECONCILE_MAX_AGE_HOURS")
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|hours| *hours >= 1)
            .unwrap_or(DOWNLOAD_QUEUE_RECONCILE_MAX_AGE_HOURS);
        chrono::Duration::hours(hours)
    })
}


#[derive(Clone, Debug)]
pub struct DownloadQueuePollerOptions {
    pub interval: Duration,
    /// Client types excluded for the poller's whole lifetime.
    pub excluded_client_types: Vec<String>,
    /// Client types excluded only while a realtime bridge covers them.
    ///
    /// Read fresh on every tick so the bridge supervisor can flip coverage at
    /// runtime — the fix for bridge eligibility being frozen at boot. An
    /// unwired handle is empty and changes nothing.
    pub bridged_client_types: crate::tracked_downloads::BridgedClientTypesHandle,
}

impl Default for DownloadQueuePollerOptions {
    fn default() -> Self {
        Self {
            interval: download_queue_poll_interval(),
            excluded_client_types: Vec::new(),
            bridged_client_types: crate::tracked_downloads::BridgedClientTypesHandle::new(),
        }
    }
}

/// The static and bridge-covered exclusions, merged and deduplicated.
fn effective_excluded_client_types(
    static_excluded: &[String],
    bridged: &crate::tracked_downloads::BridgedClientTypesHandle,
) -> Vec<String> {
    let mut merged = static_excluded.to_vec();
    for client_type in bridged.snapshot() {
        if !merged.contains(&client_type) {
            merged.push(client_type);
        }
    }
    merged
}

struct TrackedDownloadWorkDrain {
    pending_ids: std::collections::VecDeque<String>,
    attempted_ids: HashSet<String>,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
}

impl TrackedDownloadWorkDrain {
    fn empty() -> Self {
        Self {
            pending_ids: std::collections::VecDeque::new(),
            attempted_ids: HashSet::new(),
            completed_lookup: crate::completed_download_handler::CompletedDownloadLookup::default(),
        }
    }

    fn new(
        ids: Vec<String>,
        completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    ) -> Self {
        Self {
            pending_ids: ids.into(),
            attempted_ids: HashSet::new(),
            completed_lookup,
        }
    }

    fn has_pending(&self) -> bool {
        !self.pending_ids.is_empty()
    }
}

struct TrackedDownloadRuntimeState {
    tracker: crate::tracked_downloads::TrackedDownloadService,
    previous_items_by_projection: HashMap<String, HashMap<String, DownloadQueueItem>>,
    tracked_work_in_flight: HashSet<String>,
    tracked_work_drain: TrackedDownloadWorkDrain,
}

impl TrackedDownloadRuntimeState {
    fn new() -> Self {
        Self {
            tracker: crate::tracked_downloads::TrackedDownloadService::new(),
            previous_items_by_projection: HashMap::new(),
            tracked_work_in_flight: HashSet::new(),
            tracked_work_drain: TrackedDownloadWorkDrain::empty(),
        }
    }
}

enum TrackedDownloadSnapshotPrune {
    GlobalExcludingClientTypes,
    Scope(crate::tracked_downloads::TrackedDownloadSnapshotScope),
    None,
}

enum TrackedDownloadSnapshotProjection {
    Publish {
        key: String,
        actor_id: Option<String>,
    },
    UpsertOnly {
        actor_id: Option<String>,
    },
}

enum TrackedDownloadSnapshotDispatch {
    AllTrackable,
    Seen {
        completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    },
    None,
}

fn apply_tracked_download_queue_metadata(
    item: &mut DownloadQueueItem,
    tracked: &TrackedDownloadQueueMetadata,
) {
    item.tracked_state = Some(tracked.state);
    item.tracked_status = Some(tracked.status);
    item.tracked_status_messages
        .clone_from(&tracked.status_messages);
    item.tracked_match_type = Some(tracked.match_type);
    if let Some(source_title) = tracked
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        item.title_name = source_title.to_string();
    }
    if item.title_id.is_none() && tracked.title_id.is_some() {
        item.title_id.clone_from(&tracked.title_id);
    }
    if item.facet.is_none() && tracked.facet.is_some() {
        item.facet.clone_from(&tracked.facet);
    }
}
fn tracked_download_queue_snapshot(item: &TrackedDownload) -> TrackedDownloadQueueMetadata {
    TrackedDownloadQueueMetadata::from(item)
}
impl AppUseCase {
    pub async fn ignore_tracked_download(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let source_identity = DownloadSourceIdentity::new(
            client_id,
            client_type,
            download_client_item_id,
        );
        let tracked_id = crate::tracked_downloads::tracked_download_id(
            client_id,
            client_type,
            download_client_item_id,
        );

        let Some(handle) = self.runtime.acquisition.tracked_download_handle.as_ref() else {
            return match finalize_scryer_download_ignored(
                self,
                crate::domain_events::DomainEventActor::from(actor),
                source_identity,
            )
            .await?
            {
                FinalizeIgnoredOutcome::Finalized => Ok(()),
                FinalizeIgnoredOutcome::PreservedTerminal(state) => {
                    Err(preserved_terminal_ignore_error(&state))
                }
                FinalizeIgnoredOutcome::NoSubmission => Err(AppError::Repository(
                    "tracked download service unavailable".into(),
                )),
            };
        };

        match handle.ignore(tracked_id).await {
            Ok(()) => Ok(()),
            Err(error @ AppError::NotFound(_)) => {
                match finalize_scryer_download_ignored(
                    self,
                    crate::domain_events::DomainEventActor::from(actor),
                    source_identity,
                )
                .await?
                {
                    FinalizeIgnoredOutcome::Finalized => Ok(()),
                    FinalizeIgnoredOutcome::PreservedTerminal(state) => {
                        Err(preserved_terminal_ignore_error(&state))
                    }
                    FinalizeIgnoredOutcome::NoSubmission => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }
}

/// Outcome of durably finalizing a download as ignored.
pub(crate) enum FinalizeIgnoredOutcome {
    /// The download is durably ignored (now, or already was).
    Finalized,
    /// The download already reached a terminal outcome; it was left as-is.
    PreservedTerminal(String),
    /// No scryer submission exists for this identity; nothing durable to write.
    NoSubmission,
}

fn preserved_terminal_ignore_error(state: &str) -> AppError {
    AppError::Validation(format!(
        "download already resolved as {state}; nothing to ignore"
    ))
}

pub(crate) async fn finalize_scryer_download_ignored(
    app: &AppUseCase,
    actor: crate::domain_events::DomainEventActor,
    source_identity: DownloadSourceIdentity,
) -> AppResult<FinalizeIgnoredOutcome> {
    let ignored = scryer_domain::TrackedDownloadState::Ignored.as_str();
    if let Err(error) = app
        .services
        .workflow
        .imports
        .delete_manual_import_selections_for_source(&source_identity)
        .await
    {
        tracing::warn!(
            client_type = %source_identity.client_type,
            download_client_item_id = %source_identity.item_id,
            error = %error,
            "failed to remove terminal manual import source"
        );
    }
    // A download that already reached a terminal outcome keeps it: a later
    // delete of the client entry is cleanup, not a change of outcome.
    let preserved_states = [
        scryer_domain::TrackedDownloadState::Imported.as_str(),
        scryer_domain::TrackedDownloadState::Failed.as_str(),
    ];

    let submission_repository = &app.services.workflow.download_submissions;
    let Some(submission) = submission_repository
        .find_by_client_item_id(&source_identity)
        .await?
    else {
        return Ok(FinalizeIgnoredOutcome::NoSubmission);
    };
    if submission.title_id.trim().is_empty() {
        return Ok(FinalizeIgnoredOutcome::NoSubmission);
    }

    match submission_repository
        .get_tracked_state(&source_identity)
        .await?
        .as_deref()
    {
        Some(state) if state == ignored => return Ok(FinalizeIgnoredOutcome::Finalized),
        Some(state) if preserved_states.contains(&state) => {
            return Ok(FinalizeIgnoredOutcome::PreservedTerminal(state.to_string()));
        }
        _ => {}
    }

    // Legacy submissions predate download-id identity rows; for them the
    // submission-row state above is both the durability and idempotency guard.
    let mut identity_already_ignored = false;
    if let Some(submission_identity) = submission_repository
        .get_submission_identity(&source_identity)
        .await?
    {
        let previous = submission_repository
            .upsert_identity_tracked_state_returning_previous(
                &submission_identity,
                Some(&source_identity),
                ignored,
                &preserved_states,
                None,
                None,
            )
            .await?;
        match previous.as_deref() {
            Some(state) if preserved_states.contains(&state) => {
                return Ok(FinalizeIgnoredOutcome::PreservedTerminal(state.to_string()));
            }
            Some(state) if state == ignored => identity_already_ignored = true,
            _ => {}
        }
    }

    submission_repository
        .update_tracked_state(&source_identity, ignored)
        .await?;

    if identity_already_ignored {
        // Healed the submission row for an identity that was already ignored;
        // the audit event was emitted when the identity row transitioned.
        return Ok(FinalizeIgnoredOutcome::Finalized);
    }

    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&submission.title_id)
        .await?;
    let source_provider = crate::integration::workflow::source_provider_label(
        submission.source_provider_name.as_deref(),
        submission.source_hint.as_deref(),
    );
    let payload = scryer_domain::DomainEventPayload::DownloadIgnored(
        scryer_domain::DownloadIgnoredEventData {
            title: title
                .as_ref()
                .map(crate::domain_events::title_context_snapshot),
            download_client_item_id: source_identity.item_id.clone(),
            client_id: source_identity.client_id.clone(),
            client_type: Some(source_identity.client_type.clone()),
            source_provider,
            source_title: submission.source_title.clone(),
        },
    );
    let event = if let Some(title) = title.as_ref() {
        crate::domain_events::new_title_domain_event(actor, title, payload)
    } else {
        crate::domain_events::new_global_domain_event(actor, payload)
    };
    app.append_domain_event(event).await?;
    Ok(FinalizeIgnoredOutcome::Finalized)
}
impl AppUseCase {
    pub async fn mark_tracked_download_failed(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        skip_reacquire: bool,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .mark_failed(
                crate::tracked_downloads::tracked_download_id(
                    client_id,
                    client_type,
                    download_client_item_id,
                ),
                skip_reacquire,
            )
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn retry_tracked_download_import(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .retry_import(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn assign_tracked_download_title(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        title_id: &str,
        scope: SubmissionScope,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let submission = DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: title.facet.as_str().to_string(),
            download_client_id: client_id.map(str::to_string),
            download_client_type: client_type.to_string(),
            download_client_item_id: download_client_item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(title.name.clone()),
            request_signature: None,
            scope,
        };
        let actor_snapshot = crate::domain_events::DomainEventActor::from(actor)
            .into_download_submission_actor_snapshot();
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .assign_title(
                crate::tracked_downloads::tracked_download_id(
                    client_id,
                    client_type,
                    download_client_item_id,
                ),
                title,
                submission,
                actor_snapshot,
            )
            .await?;
        Ok(())
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "snapshot processing owns tracker, dispatch, projection, and source-specific pruning"
)]
async fn process_tracked_download_snapshot(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    mut items: Vec<DownloadQueueItem>,
    completed_download_lookup: Option<crate::completed_download_handler::CompletedDownloadLookup>,
    prune: TrackedDownloadSnapshotPrune,
    projection: TrackedDownloadSnapshotProjection,
    dispatch: TrackedDownloadSnapshotDispatch,
    emit_metrics: bool,
    excluded_client_type_refs: &[&str],
    snapshot_label: &'static str,
) {
    let cycle_started_at = Instant::now();

    enrich_download_queue_items_from_submissions(app, &mut items).await;

    let mut seen_ids = HashSet::new();
    let mut seen_order = Vec::new();

    // Phase 1: Refresh — track each item and run checks.
    for item in items.iter() {
        let id = tracked_download_id_for_item(item);
        if seen_ids.insert(id.clone()) {
            seen_order.push(id.clone());
        }

        let is_new = runtime.tracker.find(&id).is_none();
        runtime.tracker.track(app, item.clone()).await;

        if let Some(td) = runtime.tracker.find(&id)
            && is_new
        {
            if td.state.is_terminal() || is_history_download_state(&td.client_item.state) {
                tracing::debug!(
                    id = %td.id,
                    state = ?td.state,
                    client_state = ?td.client_item.state,
                    match_type = ?td.match_type,
                    title_id = ?td.title_id,
                    client_title_name = %td.client_item.title_name,
                    "tracked: new background download"
                )
            } else {
                tracing::info!(
                    id = %td.id,
                    state = ?td.state,
                    client_state = ?td.client_item.state,
                    match_type = ?td.match_type,
                    title_id = ?td.title_id,
                    client_title_name = %td.client_item.title_name,
                    "tracked: new download"
                )
            }
        }

        if let Some(td) = runtime.tracker.find_mut(&id)
            && matches!(
                td.state,
                TrackedDownloadState::Downloading
                    | TrackedDownloadState::ImportPending
                    | TrackedDownloadState::ImportBlocked
            )
        {
            let state_before = td.state;
            crate::failed_download_handler::check(td);
            if td.state != TrackedDownloadState::FailedPending {
                crate::completed_download_handler::check_with_lookup(
                    app,
                    td,
                    completed_download_lookup.as_ref(),
                )
                .await;
            }
            if td.state != state_before {
                tracing::info!(
                    id = %id,
                    from = ?state_before,
                    to = ?td.state,
                    "tracked: state transition after check"
                );
            }
        }
    }

    let unavailable_sources = match prune {
        TrackedDownloadSnapshotPrune::GlobalExcludingClientTypes => runtime
            .tracker
            .update_trackable_excluding_client_types(&seen_ids, excluded_client_type_refs),
        TrackedDownloadSnapshotPrune::Scope(scope) => {
            runtime
                .tracker
                .update_trackable_for_scope(&seen_ids, &scope)
        }
        TrackedDownloadSnapshotPrune::None => Vec::new(),
    };

    for source_identity in unavailable_sources {
        if let Err(error) = app
            .services
            .workflow
            .imports
            .delete_manual_import_selections_for_source(&source_identity)
            .await
        {
            tracing::warn!(
                error = %error,
                client_type = %source_identity.client_type,
                item_id = %source_identity.item_id,
                "failed to clean up manual-import selections for unavailable download"
            );
        }
    }

    reconcile_terminal_tracked_downloads(app, &mut runtime.tracker).await;
    publish_runtime_tracked_download_snapshot_cache(app, &runtime.tracker).await;

    // Phase 2: Dispatch — import pending and failed items.
    let mut published_after_dispatch = false;
    if runtime.tracked_work_in_flight.is_empty() {
        if !runtime.tracked_work_drain.has_pending() {
            match dispatch {
                TrackedDownloadSnapshotDispatch::AllTrackable => {
                    let trackable_ids = trackable_ids_excluding_client_types(
                        &runtime.tracker,
                        excluded_client_type_refs,
                    );
                    runtime.tracked_work_drain = build_tracked_download_work_drain(
                        app,
                        &runtime.tracker,
                        &runtime.tracked_work_in_flight,
                        &trackable_ids,
                        excluded_client_type_refs,
                    )
                    .await;
                }
                TrackedDownloadSnapshotDispatch::Seen { completed_lookup } => {
                    let trackable_ids = seen_order
                        .iter()
                        .filter(|id| {
                            runtime.tracker.find(id).is_some_and(|td| {
                                td.is_trackable
                                    && !td.state.is_terminal()
                                    && !td.waiting_for_completed_history
                                    && completed_lookup.matches_tracked_download(td)
                            })
                        })
                        .cloned()
                        .collect();
                    runtime.tracked_work_drain =
                        TrackedDownloadWorkDrain::new(trackable_ids, completed_lookup);
                }
                TrackedDownloadSnapshotDispatch::None => {}
            }
        }

        if runtime.tracked_work_drain.has_pending()
            && try_dispatch_next_tracked_download_background_work(
                app,
                actor,
                &mut runtime.tracker,
                &mut runtime.tracked_work_in_flight,
                result_tx,
                &mut runtime.tracked_work_drain,
            )
        {
            published_after_dispatch = true;
        }
    }

    if published_after_dispatch {
        publish_runtime_tracked_download_snapshot_cache(app, &runtime.tracker).await;
    }

    // Enrich items with tracked state before broadcasting.
    for item in &mut items {
        let id = tracked_download_id_for_item(item);
        if let Some(td) = runtime.tracker.find(&id) {
            let metadata = tracked_download_queue_snapshot(td);
            apply_tracked_download_queue_metadata(item, &metadata);
        }
    }

    if emit_metrics {
        // Emit download queue gauge by state.
        let mut counts = [0u64; 9];
        for item in &items {
            match item.state {
                scryer_domain::DownloadQueueState::Queued => counts[0] += 1,
                scryer_domain::DownloadQueueState::Downloading => counts[1] += 1,
                scryer_domain::DownloadQueueState::Paused => counts[2] += 1,
                scryer_domain::DownloadQueueState::Completed => counts[3] += 1,
                scryer_domain::DownloadQueueState::ImportPending => counts[4] += 1,
                scryer_domain::DownloadQueueState::Failed => counts[5] += 1,
                scryer_domain::DownloadQueueState::Verifying => counts[6] += 1,
                scryer_domain::DownloadQueueState::Repairing => counts[7] += 1,
                scryer_domain::DownloadQueueState::Extracting => counts[8] += 1,
            }
        }
        let labels = [
            "queued",
            "downloading",
            "paused",
            "completed",
            "import_pending",
            "failed",
            "verifying",
            "repairing",
            "extracting",
        ];
        for (label, &count) in labels.iter().zip(&counts) {
            metrics::gauge!("scryer_download_queue_items", "state" => *label).set(count as f64);
        }
    }

    match projection {
        TrackedDownloadSnapshotProjection::Publish { key, actor_id } => {
            let previous_items = runtime
                .previous_items_by_projection
                .entry(key)
                .or_default();
            publish_download_queue_snapshot_events(app, actor_id, previous_items, &items).await;
        }
        TrackedDownloadSnapshotProjection::UpsertOnly { actor_id } => {
            publish_download_queue_upsert_events(app, actor_id, &items).await;
        }
    }

    tracing::debug!(
        elapsed_ms = cycle_started_at.elapsed().as_millis() as u64,
        item_count = items.len(),
        tracked_count = runtime.tracker.get_all().len(),
        active_workers = runtime.tracked_work_in_flight.len(),
        snapshot = snapshot_label,
        "download queue poller cycle completed"
    );
}

fn tracked_download_snapshot_projection_key(
    scope: &crate::tracked_downloads::TrackedDownloadSnapshotScope,
) -> Option<String> {
    match scope {
        crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id,
            client_type,
        } => Some(format!(
            "client:{}:{}",
            client_type.trim().to_ascii_lowercase(),
            client_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("")
        )),
        crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta => None,
    }
}

async fn process_external_tracked_download_snapshot_update(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    update: crate::tracked_downloads::TrackedDownloadSnapshotUpdate,
    excluded_client_type_refs: &[&str],
) {
    let crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
        scope,
        items,
        completed_downloads,
        actor_id,
    } = update;
    let has_completed_downloads = !completed_downloads.is_empty();
    let completed_lookup =
        crate::completed_download_handler::CompletedDownloadLookup::from_recent_downloads(
            completed_downloads,
        );
    let projection_key = tracked_download_snapshot_projection_key(&scope);
    let prune = match scope.clone() {
        crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
            ..
        } => TrackedDownloadSnapshotPrune::Scope(scope),
        crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta => {
            TrackedDownloadSnapshotPrune::None
        }
    };
    let projection = match projection_key {
        Some(key) => TrackedDownloadSnapshotProjection::Publish { key, actor_id },
        None => TrackedDownloadSnapshotProjection::UpsertOnly { actor_id },
    };
    let emit_metrics = matches!(
        &projection,
        TrackedDownloadSnapshotProjection::Publish { .. }
    );
    let dispatch = if has_completed_downloads {
        TrackedDownloadSnapshotDispatch::Seen {
            completed_lookup: completed_lookup.clone(),
        }
    } else {
        TrackedDownloadSnapshotDispatch::None
    };

    process_tracked_download_snapshot(
        app,
        actor,
        runtime,
        result_tx,
        items,
        Some(completed_lookup),
        prune,
        projection,
        dispatch,
        emit_metrics,
        excluded_client_type_refs,
        "external",
    )
    .await;
}

pub async fn start_download_queue_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
) {
    let (_snapshot_tx, snapshot_rx) =
        tokio::sync::mpsc::channel::<crate::tracked_downloads::TrackedDownloadSnapshotUpdate>(1);
    start_download_queue_poller_with_options(
        app,
        token,
        command_rx,
        snapshot_rx,
        DownloadQueuePollerOptions::default(),
    )
    .await;
}

pub async fn start_download_queue_poller_with_options(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    mut command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
    mut snapshot_rx: tokio::sync::mpsc::Receiver<
        crate::tracked_downloads::TrackedDownloadSnapshotUpdate,
    >,
    options: DownloadQueuePollerOptions,
) {
    use crate::tracked_downloads::publish_runtime_tracked_download_snapshot_cache;

    let actor = User::system_execution_actor();

    let mut runtime = TrackedDownloadRuntimeState::new();
    let (tracked_work_result_tx, mut tracked_work_result_rx) =
        tokio::sync::mpsc::unbounded_channel::<TrackedDownloadBackgroundWorkResult>();

    // Exclusions are re-derived at every use instead of once at startup: the
    // bridged set changes at runtime as the bridge supervisor starts and stops
    // realtime coverage for clients.
    let static_excluded_client_types = options.excluded_client_types.clone();
    let bridged_client_types = options.bridged_client_types.clone();
    tracing::info!(
        interval_secs = options.interval.as_secs(),
        excluded_client_types = ?static_excluded_client_types,
        bridged_client_types = ?bridged_client_types.snapshot(),
        "download queue poller started (tracked downloads enabled)"
    );
    let mut interval = tokio::time::interval(options.interval);
    let mut commands_open = true;
    let mut snapshots_open = true;
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("download queue poller shutting down");
                break;
            }
            maybe_command = command_rx.recv(), if commands_open => {
                match maybe_command {
                    Some(command) => {
                        handle_tracked_download_command(
                            &app,
                            &actor,
                            &mut runtime.tracker,
                            &mut runtime.tracked_work_in_flight,
                            &tracked_work_result_tx,
                            command,
                        )
                        .await;
                    }
                    None => {
                        commands_open = false;
                    }
                }
            }
            maybe_snapshot = snapshot_rx.recv(), if snapshots_open => {
                match maybe_snapshot {
                    Some(update) => {
                        let effective_excluded = effective_excluded_client_types(
                            &static_excluded_client_types,
                            &bridged_client_types,
                        );
                        let excluded_client_type_refs = effective_excluded
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>();
                        process_external_tracked_download_snapshot_update(
                            &app,
                            &actor,
                            &mut runtime,
                            &tracked_work_result_tx,
                            update,
                            &excluded_client_type_refs,
                        )
                        .await;
                    }
                    None => {
                        snapshots_open = false;
                    }
                }
            }
            maybe_result = tracked_work_result_rx.recv(), if !runtime.tracked_work_in_flight.is_empty() => {
                if let Some(result) = maybe_result {
                    handle_tracked_download_background_work_result(
                        &app,
                        &mut runtime.tracker,
                        &mut runtime.tracked_work_in_flight,
                        result,
                    )
                    .await;
                    if try_dispatch_next_tracked_download_background_work(
                        &app,
                        &actor,
                        &mut runtime.tracker,
                        &mut runtime.tracked_work_in_flight,
                        &tracked_work_result_tx,
                        &mut runtime.tracked_work_drain,
                    ) {
                        publish_runtime_tracked_download_snapshot_cache(&app, &runtime.tracker).await;
                    }
                }
            }
            _ = interval.tick() => {
                let effective_excluded = effective_excluded_client_types(
                    &static_excluded_client_types,
                    &bridged_client_types,
                );
                let excluded_client_type_refs = effective_excluded
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                // Queue AND recent history, every tick.
                //
                // A download client's queue only shows work IN FLIGHT, so a
                // completion is a queue-ABSENCE event — and absence is not
                // observable. Recent history is where completions actually
                // appear. Sampling history on its own slower window meant most
                // ticks were structurally incapable of seeing a completion:
                // anything that finished between two history reads was
                // stranded, and the faster the client (weaver finishes small
                // jobs in ~200ms; nzbget in ~1s) the more likely that was.
                // Reading both together on one cadence is what Sonarr does,
                // and it makes the tick self-sufficient rather than a
                // best-effort sighting that history occasionally repairs.
                match app
                    .collect_download_snapshot_items_excluding_client_types(
                        true,
                        true,
                        false,
                        &excluded_client_type_refs,
                    )
                    .await
                {
                    Ok(items) => {
                        let completed_download_lookup =
                            crate::completed_download_handler::load_completed_download_lookup_for_items_excluding_client_types(
                                &app,
                                &items,
                                DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
                                &excluded_client_type_refs,
                            )
                            .await;
                        process_tracked_download_snapshot(
                            &app,
                            &actor,
                            &mut runtime,
                            &tracked_work_result_tx,
                            items,
                            completed_download_lookup,
                            TrackedDownloadSnapshotPrune::GlobalExcludingClientTypes,
                            TrackedDownloadSnapshotProjection::Publish {
                                key: "poller".to_string(),
                                actor_id: None,
                            },
                            TrackedDownloadSnapshotDispatch::AllTrackable,
                            true,
                            &excluded_client_type_refs,
                            "poller",
                        ).await;
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "download queue poll failed");
                    }
                }
                reconcile_excluded_client_recent_history(
                    &app,
                    &actor,
                    &mut runtime,
                    &tracked_work_result_tx,
                    &excluded_client_type_refs,
                )
                .await;
                try_dispatch_excluded_completed_history_retry(
                    &app,
                    &actor,
                    &mut runtime,
                    &tracked_work_result_tx,
                    &excluded_client_type_refs,
                )
                .await;
            }
        }
    }
}
fn resolve_tracked_command_id(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    requested_id: &str,
) -> String {
    tracker
        .resolve_cached_id(requested_id)
        .unwrap_or_else(|| requested_id.to_string())
}
pub(crate) async fn assign_tracked_download_title_command(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    requested_id: String,
    title: scryer_domain::Title,
    submission: DownloadSubmission,
    actor_snapshot: crate::DownloadSubmissionActorSnapshot,
) -> AppResult<()> {
    let id = resolve_tracked_command_id(tracker, &requested_id);
    if tracked_work_in_flight.contains(&id) {
        return Err(AppError::Validation(format!(
            "tracked download {requested_id} is busy processing"
        )));
    }
    if tracker.find(&id).is_none() {
        return Err(AppError::NotFound(format!(
            "tracked download {requested_id}"
        )));
    }

    let source_identity = DownloadSourceIdentity::from_submission(&submission);
    app.services
        .workflow
        .download_submissions
        .record_submission(submission)
        .await?;
    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .record_submission_actor_snapshot(&source_identity, actor_snapshot)
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = ?source_identity.client_id,
            client_type = %source_identity.client_type,
            download_client_item_id = %source_identity.item_id,
            "download_submission_actor_snapshot_persistence_failed"
        );
    }

    let tracked = tracker
        .find_mut(&id)
        .expect("serialized tracked download disappeared during title assignment");
    crate::tracked_downloads::assign_title_to_tracked_download(app, tracked, &title).await;
    publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
    Ok(())
}
async fn handle_tracked_download_command(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    tracked_work_result_tx: &tokio::sync::mpsc::UnboundedSender<
        TrackedDownloadBackgroundWorkResult,
    >,
    command: crate::tracked_downloads::TrackedDownloadCommand,
) {
    use crate::tracked_downloads::TrackedDownloadCommand;
    use scryer_domain::{TrackedDownloadState, TrackedDownloadStatus};

    match command {
        TrackedDownloadCommand::MarkImported { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Imported)
                    .await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Ignore { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let source_identity = tracker
                .find(&id)
                .and_then(tracked_download_source_identity);
            let result = async {
                if tracker.find(&id).is_none() {
                    return Err(AppError::NotFound(format!(
                        "tracked download {requested_id}"
                    )));
                }
                if let Some(source_identity) = source_identity {
                    match finalize_scryer_download_ignored(
                        app,
                        crate::domain_events::DomainEventActor::from(actor),
                        source_identity,
                    )
                    .await?
                    {
                        FinalizeIgnoredOutcome::Finalized | FinalizeIgnoredOutcome::NoSubmission => {
                        }
                        FinalizeIgnoredOutcome::PreservedTerminal(state) => {
                            return Err(preserved_terminal_ignore_error(&state));
                        }
                    }
                }
                let td = tracker
                    .find_mut(&id)
                    .expect("serialized tracked download disappeared while ignoring");
                td.state = TrackedDownloadState::Ignored;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Ignored)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Ignored)
                    .await;
                Ok(())
            }
            .await;
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkFailed {
            id,
            skip_reacquire,
            reply,
        } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let failure_identity = tracker.find(&id).and_then(
                crate::failed_download_handler::tracked_download_failure_submission_identity,
            );
            let has_grabbed_submission = if let Some(identity) = failure_identity.as_ref() {
                crate::failed_download_handler::download_submission_exists(app, identity).await
            } else {
                false
            };
            let result = if let Some(td) = tracker.find_mut(&id) {
                if !has_grabbed_submission {
                    crate::failed_download_handler::warn_download_not_grabbed(td);
                    if td.state == TrackedDownloadState::FailedPending {
                        td.state = TrackedDownloadState::Downloading;
                    }
                    td.skip_reacquire_on_failure = false;
                    Ok(())
                } else {
                    td.state = TrackedDownloadState::FailedPending;
                    td.status = TrackedDownloadStatus::Error;
                    td.status_messages.clear();
                    td.skip_reacquire_on_failure = skip_reacquire;
                    let completed_lookup =
                        crate::completed_download_handler::CompletedDownloadLookup::default();
                    let _ = try_dispatch_tracked_download_background_work(
                        app,
                        actor,
                        tracker,
                        tracked_work_in_flight,
                        tracked_work_result_tx,
                        &id,
                        &completed_lookup,
                    );
                    Ok(())
                }
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::RetryImport { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.reset_for_import_retry();
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::AssignTitle {
            id,
            title,
            submission,
            actor_snapshot,
            reply,
        } => {
            let result = assign_tracked_download_title_command(
                app,
                tracker,
                tracked_work_in_flight,
                id,
                *title,
                *submission,
                actor_snapshot,
            )
            .await;
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Snapshot { ids, reply } => {
            let snapshot = ids
                .into_iter()
                .filter_map(|id| {
                    let resolved_id = resolve_tracked_command_id(tracker, &id);
                    tracker
                        .find(&resolved_id)
                        .map(|tracked| (id, tracked_download_queue_snapshot(tracked)))
                })
                .collect();
            let _ = reply.send(snapshot);
        }
    }
}
fn prepare_tracked_download_background_work_dispatch(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
) -> Option<(TrackedDownloadBackgroundWorkKind, TrackedDownload)> {
    let td = tracker.find_mut(id)?;
    match td.state {
        TrackedDownloadState::ImportPending => {
            if td.waiting_for_completed_history {
                return None;
            }
            if td
                .no_video_import_retry
                .as_ref()
                .is_some_and(|retry| retry.next_retry_at > chrono::Utc::now())
            {
                return None;
            }
            crate::completed_download_handler::mark_importing(td);
            Some((TrackedDownloadBackgroundWorkKind::Import, td.clone()))
        }
        TrackedDownloadState::FailedPending => {
            Some((TrackedDownloadBackgroundWorkKind::Failed, td.clone()))
        }
        _ => None,
    }
}

fn trackable_import_work_completed_lookup_items(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    trackable_ids: &[String],
) -> Vec<DownloadQueueItem> {
    let now = chrono::Utc::now();
    trackable_ids
        .iter()
        .filter(|id| !tracked_work_in_flight.contains(*id))
        .filter_map(|id| {
            tracker.find(id).and_then(|td| {
                if td.state == TrackedDownloadState::ImportPending
                    && td
                        .no_video_import_retry
                        .as_ref()
                        .is_none_or(|retry| retry.next_retry_at <= now)
                {
                    Some(td.client_item.clone())
                } else {
                    None
                }
            })
        })
        .collect()
}

fn trackable_ids_excluding_client_types(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    excluded_client_types: &[&str],
) -> Vec<String> {
    tracker
        .get_all()
        .into_iter()
        .filter(|td| td.is_trackable && !td.state.is_terminal())
        .filter(|td| {
            !crate::tracked_downloads::tracked_client_type_is_excluded(
                &td.client_type,
                excluded_client_types,
            )
        })
        .map(|td| td.id.clone())
        .collect()
}

fn excluded_completed_history_retry_items(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    excluded_client_types: &[&str],
) -> Vec<(String, DownloadQueueItem)> {
    tracker
        .get_all()
        .into_iter()
        .filter(|td| td.is_trackable)
        .filter(|td| td.client_item.state == scryer_domain::DownloadQueueState::Completed)
        .filter(|td| excluded_completed_history_retry_candidate(td))
        .filter(|td| !td.import_attempted)
        .filter(|td| !tracked_work_in_flight.contains(&td.id))
        .filter(|td| {
            crate::tracked_downloads::tracked_client_type_is_excluded(
                &td.client_type,
                excluded_client_types,
            )
        })
        .map(|td| (td.id.clone(), td.client_item.clone()))
        .collect()
}

fn excluded_completed_history_retry_candidate(
    td: &crate::tracked_downloads::TrackedDownload,
) -> bool {
    (td.state == TrackedDownloadState::ImportPending && td.waiting_for_completed_history)
        || (td.state == TrackedDownloadState::Downloading && td.path_missing_since.is_some())
}

fn tracked_download_ready_for_retry_dispatch(
    td: &crate::tracked_downloads::TrackedDownload,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    td.is_trackable
        && td.state == TrackedDownloadState::ImportPending
        && !td.waiting_for_completed_history
        && !td.import_attempted
        && td
            .no_video_import_retry
            .as_ref()
            .is_none_or(|retry| retry.next_retry_at <= now)
}

struct TrackedDownloadHistoryRetryDrain {
    drain: TrackedDownloadWorkDrain,
    revalidated: bool,
}

async fn build_excluded_completed_history_retry_drain(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    excluded_client_types: &[&str],
) -> TrackedDownloadHistoryRetryDrain {
    let retry_items =
        excluded_completed_history_retry_items(tracker, tracked_work_in_flight, excluded_client_types);
    if retry_items.is_empty() {
        return TrackedDownloadHistoryRetryDrain {
            drain: TrackedDownloadWorkDrain::empty(),
            revalidated: false,
        };
    }

    let lookup_items = retry_items
        .iter()
        .map(|(_, item)| item.clone())
        .collect::<Vec<_>>();
    let mut completed_lookup =
        crate::completed_download_handler::load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
            app,
            &lookup_items,
            DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
            &[],
        )
        .await
        .unwrap_or_default();
    let mut retry_ids = Vec::new();
    let mut revalidated = false;

    // A long-stuck item can age out of the recent window and would otherwise
    // never match again. This used to be handled by asking the client for each
    // missing row individually — but for nzbget that "targeted" lookup is a
    // full history fetch (its API has no id filter), so a cycle cost one whole
    // history download PER STUCK ITEM, sequentially. Worse, the rows were
    // already in hand: the batch load above fetches the client's history in
    // full and then TRUNCATES to the recent limit, so the per-item calls were
    // re-downloading rows this function had just discarded.
    //
    // The population that triggers it is exactly the population that grows when
    // completions are being missed, so it degraded precisely when it was most
    // needed: slower fetches, feedback timeouts, client backoff, more stranded
    // items. One widened batch refresh, at most once per cycle, replaces all of
    // it — bounded work regardless of how many items are stuck.
    if retry_items.iter().any(|(id, _)| {
        !tracker
            .find(id)
            .is_some_and(|td| completed_lookup.matches_tracked_download(td))
    }) {
        let widened =
            crate::completed_download_handler::load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
                app,
                &lookup_items,
                DOWNLOAD_QUEUE_STUCK_COMPLETED_LOOKUP_LIMIT,
                &[],
            )
            .await;
        if let Some(widened) = widened {
            completed_lookup.merge(widened);
        }
    }

    for (id, item) in retry_items {
        let has_completed_history = tracker
            .find(&id)
            .is_some_and(|td| completed_lookup.matches_tracked_download(td));
        if !has_completed_history {
            tracing::debug!(
                id = %id,
                client_type = %item.client_type,
                "no completed history for stuck download; will retry next cycle"
            );
            continue;
        }

        if let Some(td) = tracker.find_mut(&id) {
            if td.waiting_for_completed_history && td.state == TrackedDownloadState::ImportPending
            {
                td.state = TrackedDownloadState::Downloading;
            }
            crate::completed_download_handler::check_with_lookup(app, td, Some(&completed_lookup))
                .await;
            revalidated = true;
        }

        let now = chrono::Utc::now();
        if tracker.find(&id).is_some_and(|td| {
            tracked_download_ready_for_retry_dispatch(td, now)
                && completed_lookup.matches_tracked_download(td)
        }) {
            retry_ids.push(id);
        }
    }

    TrackedDownloadHistoryRetryDrain {
        drain: TrackedDownloadWorkDrain::new(retry_ids, completed_lookup),
        revalidated,
    }
}

/// Bounded history reconciliation for clients excluded from generic polling
/// because a realtime bridge owns their live queue.
///
/// A bridge can miss terminal events (drops, disconnect gaps, process
/// restarts), and completed items never appear in queue snapshots, so without
/// this sweep a missed completion is permanently invisible. Every
/// recent-history cycle, fetch the client's recent history and feed completed
/// rows that still need handling through the normal snapshot path.
async fn reconcile_excluded_client_recent_history(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    excluded_client_type_refs: &[&str],
) {
    if excluded_client_type_refs.is_empty() {
        return;
    }

    let history_items = match app
        .services
        .integrations
        .download_client
        .list_recent_activity_for_client_types(
            DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT,
            excluded_client_type_refs,
        )
        .await
    {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "excluded-client history reconciliation: failed to list recent history"
            );
            return;
        }
    };

    let mut candidates: Vec<DownloadQueueItem> = Vec::new();
    for item in history_items {
        if item.state != scryer_domain::DownloadQueueState::Completed {
            continue;
        }
        let id = tracked_download_id_for_item(&item);
        if runtime.tracked_work_in_flight.contains(&id) {
            continue;
        }
        if let Some(td) = runtime.tracker.find(&id)
            && (td.state.is_terminal()
                || td.state == TrackedDownloadState::Importing
                || (td.state == TrackedDownloadState::ImportBlocked && td.import_attempted))
        {
            continue;
        }
        if let Some(state) =
            crate::completed_download_handler::queue_item_identity_tracked_state(app, &item).await
            && (state.is_terminal() || state == TrackedDownloadState::ImportBlocked)
        {
            continue;
        }
        candidates.push(item);
    }
    if candidates.is_empty() {
        return;
    }

    let completed_lookup =
        crate::completed_download_handler::load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
            app,
            &candidates,
            DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
            &[],
        )
        .await
        .unwrap_or_default();

    // This sweep exists to heal completions the bridge missed, not to import a
    // client's whole retained history. Without an age bound, the first run
    // after an upgrade would mass-import every completed row the client still
    // keeps. Anything older than the window needs an explicit backfill.
    let cutoff = chrono::Utc::now() - reconcile_history_max_age();
    let mut skipped_as_stale = 0usize;
    let mut skipped_without_timestamp = 0usize;
    candidates.retain(
        |item| match completed_lookup.completed_at_for_item(item) {
            Some(completed_at) if completed_at >= cutoff => true,
            Some(_) => {
                skipped_as_stale += 1;
                false
            }
            // No completion timestamp means the age cannot be established, so
            // the row is left alone rather than assumed recent. Reported
            // separately: unlike a stale row, this one would never age in.
            None => {
                skipped_without_timestamp += 1;
                false
            }
        },
    );
    if skipped_as_stale > 0 || skipped_without_timestamp > 0 {
        tracing::debug!(
            skipped_as_stale,
            skipped_without_timestamp,
            max_age_hours = reconcile_history_max_age().num_hours(),
            "history reconciliation skipped completions outside the reconcile window"
        );
    }
    if candidates.is_empty() {
        return;
    }

    tracing::info!(
        count = candidates.len(),
        "reconciling completed history for subscription-covered clients"
    );
    process_tracked_download_snapshot(
        app,
        actor,
        runtime,
        result_tx,
        candidates,
        Some(completed_lookup.clone()),
        TrackedDownloadSnapshotPrune::None,
        TrackedDownloadSnapshotProjection::UpsertOnly { actor_id: None },
        TrackedDownloadSnapshotDispatch::Seen { completed_lookup },
        false,
        excluded_client_type_refs,
        "history-reconcile",
    )
    .await;
}

async fn try_dispatch_excluded_completed_history_retry(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    excluded_client_types: &[&str],
) {
    if excluded_client_types.is_empty()
        || !runtime.tracked_work_in_flight.is_empty()
        || runtime.tracked_work_drain.has_pending()
    {
        return;
    }

    let retry_drain = build_excluded_completed_history_retry_drain(
        app,
        &mut runtime.tracker,
        &runtime.tracked_work_in_flight,
        excluded_client_types,
    )
    .await;
    let revalidated = retry_drain.revalidated;
    runtime.tracked_work_drain = retry_drain.drain;

    if try_dispatch_next_tracked_download_background_work(
        app,
        actor,
        &mut runtime.tracker,
        &mut runtime.tracked_work_in_flight,
        result_tx,
        &mut runtime.tracked_work_drain,
    ) || revalidated
    {
        publish_runtime_tracked_download_snapshot_cache(app, &runtime.tracker).await;
    }
}

async fn build_tracked_download_work_drain(
    app: &AppUseCase,
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    trackable_ids: &[String],
    excluded_client_type_refs: &[&str],
) -> TrackedDownloadWorkDrain {
    let import_lookup_items = trackable_import_work_completed_lookup_items(
        tracker,
        tracked_work_in_flight,
        trackable_ids,
    );
    let completed_lookup = if !import_lookup_items.is_empty() {
        crate::completed_download_handler::load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
            app,
            &import_lookup_items,
            DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
            excluded_client_type_refs,
        )
        .await
        .unwrap_or_default()
    } else {
        crate::completed_download_handler::CompletedDownloadLookup::default()
    };

    TrackedDownloadWorkDrain::new(trackable_ids.to_vec(), completed_lookup)
}

fn prepare_next_tracked_download_background_work_dispatch(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    drain: &mut TrackedDownloadWorkDrain,
) -> Option<(String, TrackedDownloadBackgroundWorkKind, TrackedDownload)> {
    if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT {
        return None;
    }

    while let Some(id) = drain.pending_ids.pop_front() {
        if !drain.attempted_ids.insert(id.clone()) {
            continue;
        }
        if tracked_work_in_flight.contains(&id) {
            continue;
        }
        if let Some((kind, tracked)) =
            prepare_tracked_download_background_work_dispatch(tracker, &id)
        {
            return Some((id, kind, tracked));
        }
    }

    None
}

fn try_dispatch_next_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    drain: &mut TrackedDownloadWorkDrain,
) -> bool {
    let Some((id, kind, tracked)) = prepare_next_tracked_download_background_work_dispatch(
        tracker,
        tracked_work_in_flight,
        drain,
    ) else {
        return false;
    };

    dispatch_prepared_tracked_download_background_work(
        app,
        actor,
        tracked_work_in_flight,
        result_tx,
        &id,
        kind,
        tracked,
        drain.completed_lookup.clone(),
    );
    true
}

fn try_dispatch_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    id: &str,
    completed_lookup: &crate::completed_download_handler::CompletedDownloadLookup,
) -> bool {
    if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT
        || tracked_work_in_flight.contains(id)
    {
        return false;
    }

    let Some((kind, tracked)) = prepare_tracked_download_background_work_dispatch(tracker, id)
    else {
        return false;
    };

    dispatch_prepared_tracked_download_background_work(
        app,
        actor,
        tracked_work_in_flight,
        result_tx,
        id,
        kind,
        tracked,
        completed_lookup.clone(),
    );
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "dispatch wiring carries state needed by both manual and drain dispatch paths"
)]
fn dispatch_prepared_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    id: &str,
    kind: TrackedDownloadBackgroundWorkKind,
    tracked: TrackedDownload,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
) {
    tracing::info!(
        id = %id,
        work = kind.as_str(),
        active_workers = tracked_work_in_flight.len() + 1,
        worker_limit = TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT,
        "tracked: dispatched background work"
    );
    tracked_work_in_flight.insert(id.to_string());
    dispatch_tracked_download_background_work(
        app.clone(),
        actor.clone(),
        tracked,
        kind,
        result_tx.clone(),
        completed_lookup,
    );
}
fn dispatch_tracked_download_background_work(
    app: AppUseCase,
    actor: User,
    tracked: crate::tracked_downloads::TrackedDownload,
    kind: TrackedDownloadBackgroundWorkKind,
    result_tx: tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
) {
    tokio::spawn(async move {
        let started_at = Instant::now();
        let tracked_id = tracked.id.clone();
        let worker = tokio::spawn(async move {
            let mut tracked = tracked;

            match kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    let _ = crate::completed_download_handler::import_with_lookup(
                        &app,
                        &actor,
                        &mut tracked,
                        &completed_lookup,
                    )
                    .await;
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    crate::failed_download_handler::process_failed(&app, &mut tracked).await;
                }
            }

            tracked
        });

        let outcome = match worker.await {
            Ok(tracked) => {
                tracing::info!(
                    id = %tracked.id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    final_state = tracked.state.as_str(),
                    "tracked: background work completed"
                );
                Ok(tracked)
            }
            Err(error) => {
                let message = format!(
                    "tracked {} worker exited before completion: {}",
                    kind.as_str(),
                    error
                );
                tracing::error!(
                    id = %tracked_id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    error = %error,
                    "tracked: background work crashed"
                );
                Err(message)
            }
        };
        let elapsed = started_at.elapsed();
        if result_tx
            .send(TrackedDownloadBackgroundWorkResult {
                id: tracked_id,
                kind,
                outcome,
                elapsed,
            })
            .is_err()
        {
            tracing::debug!(
                work = kind.as_str(),
                "tracked background work result dropped after poller shutdown"
            );
        }
    });
}
async fn handle_tracked_download_background_work_result(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result: TrackedDownloadBackgroundWorkResult,
) {
    tracked_work_in_flight.remove(&result.id);

    let Some(tracked) = tracker.find_mut(&result.id) else {
        tracing::debug!(
            id = %result.id,
            work = result.kind.as_str(),
            elapsed_ms = result.elapsed.as_millis() as u64,
            "tracked background work finished after tracker entry disappeared"
        );
        return;
    };

    let state = match result.outcome {
        Ok(finished) => {
            merge_tracked_download_background_work_state(tracked, finished);
            tracked.state
        }
        Err(message) => {
            tracked.status = TrackedDownloadStatus::Error;
            tracked.status_messages.clear();
            tracked.status_messages.push(message);
            match result.kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    tracked.import_attempted = true;
                    tracked.state = TrackedDownloadState::ImportBlocked;
                    TrackedDownloadState::ImportBlocked
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    tracked.state = TrackedDownloadState::Failed;
                    TrackedDownloadState::Failed
                }
            }
        }
    };

    if state.is_terminal() {
        tracing::info!(
            id = %result.id,
            state = state.as_str(),
            work = result.kind.as_str(),
            "tracked: persisting worker terminal state"
        );
        let persisted = tracker.persist_terminal_state(app, &result.id, state).await;
        if persisted {
            finalize_tracked_terminal_state(app, tracker, &result.id, state).await;
        }
    } else if state == TrackedDownloadState::ImportBlocked
        && result.kind == TrackedDownloadBackgroundWorkKind::Import
        && let Some(td) = tracker.find(&result.id)
    {
        // A rejected import is an operator decision point; record it durably
        // so restarts don't erase it and reconciliation doesn't re-offer it.
        crate::tracked_downloads::persist_tracked_download_state_marker(
            app,
            td,
            TrackedDownloadState::ImportBlocked,
            Some("import_blocked_after_import"),
            td.status_messages.first().map(String::as_str),
        )
        .await;
    }

    publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
}
async fn finalize_tracked_terminal_state(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
    state: TrackedDownloadState,
) {
    let Some(td) = tracker.find(id) else {
        return;
    };

    let cleanup =
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(app, td, state)
            .await;
    if crate::import::import::terminal_download_cleanup_is_complete(cleanup) {
        tracker.stop_tracking(id);
    }
}
async fn reconcile_terminal_tracked_downloads(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    reconcile_duplicate_terminal_source_states(tracker);

    let terminal_ids: Vec<(String, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.state.is_terminal())
        .map(|tracked| (tracked.id.clone(), tracked.state))
        .collect();

    for (id, state) in terminal_ids {
        finalize_tracked_terminal_state(app, tracker, &id, state).await;
    }
}

fn reconcile_duplicate_terminal_source_states(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    let mut terminal_source_states = HashMap::new();
    for tracked in tracker.get_all() {
        if !tracked.state.is_terminal() {
            continue;
        }
        let Some(source_identity) = tracked_download_source_identity(tracked) else {
            continue;
        };
        let should_replace = terminal_source_states
            .get(&source_identity)
            .is_none_or(|existing| {
                terminal_state_precedence(tracked.state) > terminal_state_precedence(*existing)
            });
        if should_replace {
            terminal_source_states.insert(source_identity, tracked.state);
        }
    }

    if terminal_source_states.is_empty() {
        return;
    }

    let updates: Vec<(String, crate::DownloadSourceIdentity, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| !tracked.state.is_terminal())
        .filter_map(|tracked| {
            let source_identity = tracked_download_source_identity(tracked)?;
            terminal_source_states
                .get(&source_identity)
                .copied()
                .map(|state| (tracked.id.clone(), source_identity, state))
        })
        .collect();

    for (id, source_identity, state) in updates {
        let Some(tracked) = tracker.find_mut(&id) else {
            continue;
        };
        tracing::info!(
            id = %tracked.id,
            client_id = tracked.client_id.as_str(),
            client_type = tracked.client_type.as_str(),
            download_client_item_id = source_identity.item_id.as_str(),
            from = ?tracked.state,
            to = ?state,
            "tracked: reconciling duplicate terminal source state"
        );
        apply_reconciled_terminal_state(tracked, state);
    }
}

fn tracked_download_source_identity(
    tracked: &TrackedDownload,
) -> Option<crate::DownloadSourceIdentity> {
    let client_type = tracked.client_type.trim();
    let item_id = tracked.client_item.download_client_item_id.trim();
    if client_type.is_empty() || item_id.is_empty() {
        return None;
    }
    Some(crate::DownloadSourceIdentity::new(
        Some(tracked.client_id.as_str()),
        client_type,
        item_id,
    ))
}

fn terminal_state_precedence(state: TrackedDownloadState) -> u8 {
    match state {
        TrackedDownloadState::Imported => 3,
        TrackedDownloadState::Failed => 2,
        TrackedDownloadState::Ignored => 1,
        _ => 0,
    }
}

fn apply_reconciled_terminal_state(tracked: &mut TrackedDownload, state: TrackedDownloadState) {
    tracked.state = state;
    match state {
        TrackedDownloadState::Imported => {
            tracked.status = TrackedDownloadStatus::Ok;
            tracked.status_messages.clear();
        }
        TrackedDownloadState::Failed => {
            tracked.status = TrackedDownloadStatus::Error;
        }
        _ => {}
    }
}

#[cfg(test)]
mod bridged_exclusion_tests {
    use super::effective_excluded_client_types;
    use crate::tracked_downloads::BridgedClientTypesHandle;

    #[test]
    fn an_unwired_handle_changes_nothing() {
        let handle = BridgedClientTypesHandle::new();
        let static_excluded = vec!["sabnzbd".to_string()];
        assert_eq!(
            effective_excluded_client_types(&static_excluded, &handle),
            vec!["sabnzbd".to_string()]
        );
        assert_eq!(
            effective_excluded_client_types(&[], &handle),
            Vec::<String>::new()
        );
    }

    #[test]
    fn bridge_coverage_is_visible_on_the_next_read() {
        // The whole point of the handle: coverage set AFTER the poller starts
        // must still take effect — the old Vec was frozen at construction.
        let handle = BridgedClientTypesHandle::new();
        assert_eq!(
            effective_excluded_client_types(&[], &handle),
            Vec::<String>::new()
        );

        handle.set(vec!["weaver".to_string()]);
        assert_eq!(
            effective_excluded_client_types(&[], &handle),
            vec!["weaver".to_string()]
        );

        handle.clear();
        assert_eq!(
            effective_excluded_client_types(&[], &handle),
            Vec::<String>::new()
        );
    }

    #[test]
    fn static_and_bridged_exclusions_merge_without_duplicates() {
        let handle = BridgedClientTypesHandle::new();
        handle.set(vec!["weaver".to_string(), "sabnzbd".to_string()]);
        let static_excluded = vec!["sabnzbd".to_string()];
        assert_eq!(
            effective_excluded_client_types(&static_excluded, &handle),
            vec!["sabnzbd".to_string(), "weaver".to_string()]
        );
    }

    #[test]
    fn clones_of_the_handle_share_one_underlying_set() {
        // The supervisor writes through its clone; the poller reads its own.
        let writer = BridgedClientTypesHandle::new();
        let reader = writer.clone();
        writer.set(vec!["weaver".to_string()]);
        assert_eq!(reader.snapshot(), vec!["weaver".to_string()]);
    }
}


#[cfg(test)]
mod poll_interval_tests {
    use super::parse_poll_secs;
    use std::time::Duration;

    const DEFAULT: Duration = Duration::from_secs(10);

    #[test]
    fn unset_override_falls_back_to_default() {
        assert_eq!(parse_poll_secs(None, DEFAULT), DEFAULT);
    }

    #[test]
    fn blank_override_falls_back_to_default() {
        assert_eq!(parse_poll_secs(Some(""), DEFAULT), DEFAULT);
        assert_eq!(parse_poll_secs(Some("   "), DEFAULT), DEFAULT);
    }

    #[test]
    fn unparsable_override_falls_back_to_default() {
        assert_eq!(parse_poll_secs(Some("abc"), DEFAULT), DEFAULT);
        assert_eq!(parse_poll_secs(Some("1.5"), DEFAULT), DEFAULT);
        assert_eq!(parse_poll_secs(Some("-1"), DEFAULT), DEFAULT);
    }

    #[test]
    fn zero_override_falls_back_to_default() {
        assert_eq!(parse_poll_secs(Some("0"), DEFAULT), DEFAULT);
    }

    #[test]
    fn valid_override_is_honored() {
        assert_eq!(parse_poll_secs(Some("1"), DEFAULT), Duration::from_secs(1));
        assert_eq!(parse_poll_secs(Some("5"), DEFAULT), Duration::from_secs(5));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            parse_poll_secs(Some("  3  "), DEFAULT),
            Duration::from_secs(3)
        );
    }
}
