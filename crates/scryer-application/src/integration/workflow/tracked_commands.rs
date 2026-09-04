const TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET: Duration = Duration::from_millis(25);
const TRACKED_DOWNLOAD_FAILED_WORKER_LIMIT: usize = 4;
const DOWNLOAD_QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(10);
const ABSENT_BINDING_RECONCILE_BATCH_SIZE: usize = 200;
const ABSENT_BINDING_RECONCILE_RECENCY_FLOOR: chrono::Duration = chrono::Duration::minutes(10);
const REMOVED_FROM_DOWNLOAD_CLIENT_REASON: &str = "removed from download client";

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
    previous_items_by_projection:
        HashMap<DownloadQueueProjectionSource, HashMap<String, DownloadQueueItem>>,
    tracked_work_in_flight: HashSet<String>,
    tracked_work_drain: TrackedDownloadWorkDrain,
}

enum TrackedDownloadBackgroundWorkEvent {
    Finished(TrackedDownloadBackgroundWorkResult),
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DownloadQueueProjectionSource {
    Poller,
    AuthoritativeBridge {
        client_type: String,
        client_id: Option<String>,
    },
}

enum TrackedDownloadSnapshotProjection {
    Publish {
        source: DownloadQueueProjectionSource,
    },
    UpsertOnly,
}

enum TrackedDownloadSnapshotDispatch {
    AllTrackable,
    Seen {
        completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    },
}

async fn publish_download_queue_source_projection(
    app: &AppUseCase,
    runtime: &mut TrackedDownloadRuntimeState,
    source: DownloadQueueProjectionSource,
    items: &[DownloadQueueItem],
) {
    let authoritative_refresh = matches!(&source, DownloadQueueProjectionSource::Poller);
    let next_items = items
        .iter()
        .cloned()
        .map(|item| (download_queue_projection_key(&item), item))
        .collect::<HashMap<_, _>>();
    runtime
        .previous_items_by_projection
        .insert(source, next_items);
    let reconciled = overlay_tracked_download_activity_items(
        &runtime.tracker,
        runtime
            .previous_items_by_projection
            .values()
            .flat_map(|projection| projection.values().cloned())
            .collect(),
    );
    if authoritative_refresh {
        app.runtime
            .acquisition
            .download_queue_snapshot
            .stage_success(reconciled)
            .await;
    } else {
        app.runtime
            .acquisition
            .download_queue_snapshot
            .stage_partial_success(reconciled)
            .await;
    }
}

fn remove_ended_bridge_projections(
    runtime: &mut TrackedDownloadRuntimeState,
    active_bridged_client_types: &[String],
    static_excluded_client_types: &[String],
) {
    let active = active_bridged_client_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let permanently_external = static_excluded_client_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    runtime.previous_items_by_projection.retain(|source, _| {
        let DownloadQueueProjectionSource::AuthoritativeBridge { client_type, .. } = source else {
            return true;
        };
        active.contains(client_type) || permanently_external.contains(client_type)
    });
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
    // Same rule the tracker itself uses when a refresh arrives without one: a
    // known observation is better than none, and a present one always wins.
    // Without this a held torrent whose live row came back framed as a
    // completed download would show no seeding progress even though the gate
    // is acting on an observation it has.
    if item.seeding.is_none() && tracked.client_item.seeding.is_some() {
        item.seeding.clone_from(&tracked.client_item.seeding);
    }
}
fn apply_tracked_download_activity_projection(
    item: &mut DownloadQueueItem,
    tracked: &TrackedDownloadQueueMetadata,
) {
    apply_tracked_download_queue_metadata(item, tracked);
    if item.client_id.trim().is_empty() && !tracked.client_id.trim().is_empty() {
        item.client_id.clone_from(&tracked.client_id);
    }
    if item.client_type.trim().is_empty() && !tracked.client_type.trim().is_empty() {
        item.client_type.clone_from(&tracked.client_type);
    }
    match tracked.state {
        // `ImportedSeeding` projects exactly like `Imported`: the files are in
        // the library. It differs only in that the client entry is still
        // there, seeding, and has not been released yet.
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => {
            // A settled import reads as finished — unless the client is
            // reporting a live problem with an entry it is still holding. A
            // torrent that errors while seeding out its goal has to keep its
            // warning and its message instead of being repainted healthy;
            // `import_status` stays `Completed` either way, so nothing
            // re-imports it and the seeding gate keeps whatever hold it has.
            if item.state != DownloadQueueState::Warning {
                item.state = DownloadQueueState::Completed;
            }
            item.progress_percent = 100;
            item.remaining_seconds = Some(0);
            item.import_status = Some(ImportStatus::Completed);
            if item.imported_at.is_none() {
                item.imported_at = item.last_updated_at.clone();
            }
        }
        TrackedDownloadState::Failed => {
            item.state = DownloadQueueState::Failed;
            item.progress_percent = 100;
            item.remaining_seconds = Some(0);
            item.attention_required = true;
            if item.import_status.is_none() {
                item.import_status = Some(ImportStatus::Failed);
            }
        }
        TrackedDownloadState::ImportPending => {
            item.state = DownloadQueueState::ImportPending;
            item.progress_percent = 100;
            item.remaining_seconds = Some(0);
        }
        TrackedDownloadState::Importing => {
            item.state = DownloadQueueState::Completed;
            item.progress_percent = 100;
            item.remaining_seconds = Some(0);
            item.import_status = Some(match item.import_status {
                Some(ImportStatus::Processing) => ImportStatus::Processing,
                _ => ImportStatus::Running,
            });
        }
        TrackedDownloadState::ImportBlocked => {
            item.state = DownloadQueueState::Completed;
            item.progress_percent = 100;
            item.remaining_seconds = Some(0);
            item.attention_required = true;
            // The block is authoritative over any *finished* import record
            // (a stale Failed/Skipped/Completed must not repaint the row), but
            // a manual import the operator just queued or that is copying
            // right now is live state the row has to show: keeping it is what
            // turns the display into the active import state, greys the
            // actions, and lets the transfer phase render. Dropping it left
            // blocked rows fully interactive while a manual import was in
            // flight.
            if !matches!(
                item.import_status,
                Some(ImportStatus::Pending | ImportStatus::Running | ImportStatus::Processing)
            ) {
                item.import_status = None;
            }
        }
        TrackedDownloadState::Downloading
        | TrackedDownloadState::FailedPending
        | TrackedDownloadState::Ignored => {}
    }
}
fn tracked_download_queue_snapshot(item: &TrackedDownload) -> TrackedDownloadQueueMetadata {
    TrackedDownloadQueueMetadata::from(item)
}
fn tracked_download_activity_queue_item(item: &TrackedDownload) -> DownloadQueueItem {
    let tracked = tracked_download_queue_snapshot(item);
    let mut item = tracked.client_item.clone();
    apply_tracked_download_activity_projection(&mut item, &tracked);
    item
}
fn overlay_tracked_download_activity_items(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    items: Vec<DownloadQueueItem>,
) -> Vec<DownloadQueueItem> {
    let mut items = dedupe_download_queue_items(items);
    let mut positions = items
        .iter()
        .enumerate()
        .map(|(index, item)| (download_queue_projection_key(item), index))
        .collect::<HashMap<_, _>>();

    for tracked in tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.is_trackable)
    {
        let metadata = tracked_download_queue_snapshot(tracked);
        let mut projected = metadata.client_item.clone();
        apply_tracked_download_activity_projection(&mut projected, &metadata);
        let key = download_queue_projection_key(&projected);
        if let Some(index) = positions.get(&key).copied() {
            apply_tracked_download_activity_projection(&mut items[index], &metadata);
            continue;
        }

        let Some(item) = synthetic_tracked_snapshot_queue_item(&metadata, None) else {
            continue;
        };
        positions.insert(download_queue_projection_key(&item), items.len());
        items.push(item);
    }

    items
}
async fn publish_runtime_tracked_download_and_activity_item(
    app: &AppUseCase,
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    item: Option<DownloadQueueItem>,
) {
    publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
    if let Some(item) = item {
        app.runtime
            .acquisition
            .download_queue_snapshot
            .stage_upserts(vec![item])
            .await;
    }
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
        let source_identity =
            ClientJobLocator::new(client_id, client_type, download_client_item_id);
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
    source_identity: ClientJobLocator,
) -> AppResult<FinalizeIgnoredOutcome> {
    finalize_scryer_download_ignored_for_download(app, actor, None, source_identity).await
}

pub(crate) async fn finalize_scryer_download_ignored_for_download(
    app: &AppUseCase,
    actor: crate::domain_events::DomainEventActor,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    source_identity: ClientJobLocator,
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
        .find_by_client_item_id_for_download(canonical_download_id, &source_identity)
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

    // The durable identity-state row is keyed by the canonical download id, and
    // since 0184 its legacy wire token is optional — so a token-less (plugin)
    // item gets a row just like a token-bearing one. The wire token is only
    // carried along as a compatibility column when the legacy tuple still
    // resolves one; its absence must not skip the durable write, or the item
    // re-enters processing on the first see after a restart.
    //
    // The store resolves the canonical id itself when the caller has none (from
    // the locator's active binding) and reports `None` without writing when
    // nothing is resolvable — for such a submission the legacy
    // `download_submissions` state read above stays the durability and
    // idempotency guard, exactly as it was before download-id identity rows.
    let submission_identity = submission_repository
        .get_submission_identity(&source_identity)
        .await?
        .unwrap_or_default();
    let previous = submission_repository
        .upsert_identity_tracked_state_for_download_returning_previous(
            crate::IdentityTrackedStateTarget {
                canonical_download_id,
                identity: &submission_identity,
                source_identity: Some(&source_identity),
            },
            ignored,
            &preserved_states,
            None,
            None,
        )
        .await?;
    let mut identity_already_ignored = false;
    match previous.as_deref() {
        Some(state) if preserved_states.contains(&state) => {
            return Ok(FinalizeIgnoredOutcome::PreservedTerminal(state.to_string()));
        }
        Some(state) if state == ignored => identity_already_ignored = true,
        _ => {}
    }

    submission_repository
        .update_tracked_state(&source_identity, ignored)
        .await?;

    if identity_already_ignored {
        // Healed the submission row for an identity that was already ignored;
        // the audit event was emitted when the identity row transitioned.
        return Ok(FinalizeIgnoredOutcome::Finalized);
    }

    reopen_scopes_released_by_ignored_submission(app, &submission).await;

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

/// The scope rows an operator remove or ignore releases: rows this submission
/// covers that still say `grabbed` for *this* release. A row grabbed for a
/// different release belongs to that other download and is left alone; a row
/// whose recorded grab cannot be read is released, because holding it would
/// hold it forever now that the claim has no expiry of its own.
pub(crate) fn scope_rows_released_by_ignored_submission<'a>(
    submission: &crate::DownloadSubmission,
    rows: &'a [crate::AcquisitionScopeState],
    episodes: &[scryer_domain::Episode],
) -> Vec<&'a crate::AcquisitionScopeState> {
    let ignored_key = submission
        .source_title
        .as_deref()
        .and_then(crate::admission::release_key);
    rows.iter()
        .filter(|row| {
            if row.status != crate::AcquisitionScopeStatus::Grabbed {
                return false;
            }
            let episode_collection_id = row.episode_id.as_ref().and_then(|episode_id| {
                episodes
                    .iter()
                    .find(|episode| &episode.id == episode_id)
                    .and_then(|episode| episode.collection_id.as_deref())
            });
            if !crate::acquisition_workflow::submission_blocks_wanted_item(
                submission,
                row,
                episode_collection_id,
            ) {
                return false;
            }
            let recorded_key = crate::quality::canonical_context::grabbed_release_record(row)
                .and_then(|record| crate::admission::release_key(&record.title));
            match (ignored_key, recorded_key) {
                (Some(ignored), Some(recorded)) => ignored == recorded,
                _ => true,
            }
        })
        .collect()
}

/// An operator remove or ignore ends a download without an outcome, so the
/// scopes it was claiming must stop saying `grabbed` for it. The scope row's
/// claim never expires on its own; this transition is its release valve. The
/// scope re-opens with its coverage pruned so it is searched again instead of
/// waiting on a download nobody will finish.
async fn reopen_scopes_released_by_ignored_submission(
    app: &AppUseCase,
    submission: &crate::DownloadSubmission,
) {
    let rows = match app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&submission.title_id))
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %submission.title_id,
                download_id = %submission.download_id,
                "failed to load scope rows while releasing an ignored download's claims"
            );
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let episodes = if rows.iter().any(|row| row.episode_id.is_some()) {
        app.services
            .catalog
            .shows
            .list_episodes_for_title(&submission.title_id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    for row in scope_rows_released_by_ignored_submission(submission, &rows, &episodes) {
        app.reopen_wanted_scope_for_acquisition(
            row,
            crate::acquisition::convergence::CoverageReopen::All,
        )
        .await;
        tracing::info!(
            wanted_item_id = %row.id,
            title_id = %submission.title_id,
            download_id = %submission.download_id,
            release = ?submission.source_title,
            "re-opened scope after an operator removed or ignored its download"
        );
    }
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: title.facet.as_str().to_string(),
            download_client_id: client_id.map(str::to_string),
            download_client_type: client_type.to_string(),
            download_client_item_id: download_client_item_id.to_string(),
            // Operator-driven, so there is no announced size.
            release_size_bytes: None,
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            // Filled from the existing row by the assignment command: the
            // grab-time indexer release name survives a reassignment.
            source_title: None,
            info_hash: None,
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

/// Whether the 24 h client-warning timeout may fail this download.
///
/// **A torrent warning is never auto-failed** — Sonarr parity, and the reason
/// is hit-and-run: a completed torrent can sit in `Warning` on a recoverable
/// client condition (disk, permissions, a tracker hiccup), and manufacturing a
/// `Failed` here removes its client entry through a path the seeding gate
/// never sees — bypassing the private rail for exactly the torrents that have
/// no profile to protect them. The gate holds on *unknown*; a timeout that
/// fails on unknown is the opposite rule, and 8 of 13 clients cannot report
/// privateness at all. The warning stays visible for the operator, who can
/// resolve or replace the download; Sonarr's `FailedDownloadService` acts only
/// on `Failed`/`IsEncrypted` and lets warnings persist the same way.
///
/// Usenet downloads keep the timeout. Anything that is not a warned,
/// Scryer-origin download is irrelevant and reports `true` (the tracker then
/// ignores it on its own checks).
pub(crate) fn warning_timeout_applies(
    app: &AppUseCase,
    td: &crate::tracked_downloads::TrackedDownload,
) -> bool {
    if td.client_item.state != scryer_domain::DownloadQueueState::Warning
        || !td.client_item.is_scryer_origin
    {
        return true;
    }
    !crate::seeding_gate::client_type_is_torrent(app, &td.client_type)
}

#[expect(
    clippy::too_many_arguments,
    reason = "snapshot processing owns tracker, dispatch, projection, and source-specific pruning"
)]
async fn process_tracked_download_snapshot(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    mut items: Vec<DownloadQueueItem>,
    completed_download_lookup: Option<crate::completed_download_handler::CompletedDownloadLookup>,
    prune: TrackedDownloadSnapshotPrune,
    projection: TrackedDownloadSnapshotProjection,
    dispatch: TrackedDownloadSnapshotDispatch,
    emit_metrics: bool,
    excluded_client_type_refs: &[&str],
    authoritative_client_ids: Option<&HashSet<String>>,
    snapshot_label: &'static str,
) {
    let cycle_started_at = Instant::now();

    enrich_download_queue_items_from_submissions(app, &mut items).await;
    if let TrackedDownloadSnapshotProjection::Publish { source } = &projection {
        // Poller items already carry import-record state (the poller loads
        // them through `enrich_download_queue_items`). Bridged clients (Weaver)
        // do not: their rows arrive straight from the subscription, so a manual
        // import the operator queued against a blocked download stayed
        // invisible and the row stayed fully interactive. Overlay the same
        // import/delete state here so every source renders the same way.
        if matches!(
            source,
            DownloadQueueProjectionSource::AuthoritativeBridge { .. }
        ) {
            enrich_queue_item_import_states(app, &mut items).await;
        }
        publish_download_queue_source_projection(app, runtime, source.clone(), &items).await;
    }

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

        let warning_timeout_applies = match runtime.tracker.find(&id) {
            Some(td) => warning_timeout_applies(app, td),
            None => true,
        };
        if runtime
            .tracker
            .fail_persistent_warning(&id, Utc::now(), warning_timeout_applies)
        {
            tracing::info!(
                id = %id,
                "tracked: client warning persisted for 24h; queueing failed-download handling"
            );
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

    let reconcile_restart_ghosts = matches!(
        prune,
        TrackedDownloadSnapshotPrune::GlobalExcludingClientTypes
    );
    let unavailable_sources = match prune {
        TrackedDownloadSnapshotPrune::GlobalExcludingClientTypes => runtime
            .tracker
            .update_trackable_excluding_client_types_for_authoritative_clients(
                &seen_ids,
                excluded_client_type_refs,
                authoritative_client_ids,
            ),
        TrackedDownloadSnapshotPrune::Scope(scope) => runtime
            .tracker
            .update_trackable_for_scope_for_authoritative_clients(
                &seen_ids,
                &scope,
                authoritative_client_ids,
            ),
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

        reconcile_authoritatively_absent_source(app, &mut runtime.tracker, &source_identity).await;
    }

    if reconcile_restart_ghosts && let Some(authoritative_client_ids) = authoritative_client_ids {
        reconcile_restart_ghost_bindings(
            app,
            &mut runtime.tracker,
            &items,
            excluded_client_type_refs,
            authoritative_client_ids,
        )
        .await;
    }

    reconcile_terminal_tracked_downloads(app, &mut runtime.tracker).await;
    publish_runtime_tracked_download_snapshot_cache(app, &runtime.tracker).await;

    // Phase 2: Dispatch — import pending and failed items.
    let mut published_after_dispatch = false;
    {
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
                                && match td.state {
                                    TrackedDownloadState::FailedPending => true,
                                    TrackedDownloadState::ImportPending => {
                                        !td.waiting_for_completed_history
                                            && completed_lookup.matches_tracked_download(td)
                                    }
                                    _ => false,
                                }
                        })
                    })
                    .cloned()
                    .collect();
                runtime.tracked_work_drain =
                    TrackedDownloadWorkDrain::new(trackable_ids, completed_lookup);
            }
        }

        while runtime.tracked_work_drain.has_pending()
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

    // The runtime tracker owns import workflow state. Overlay it after checks
    // and dispatch so the cache receives the latest state, including
    // source-missing rows that still require operator action.
    items = overlay_tracked_download_activity_items(&runtime.tracker, items);

    if emit_metrics {
        // Emit download queue gauge by state.
        let mut counts = [0u64; 10];
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
                scryer_domain::DownloadQueueState::Warning => counts[9] += 1,
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
            "warning",
        ];
        for (label, &count) in labels.iter().zip(&counts) {
            metrics::gauge!(crate::services::DOWNLOAD_QUEUE_ITEMS, "state" => *label).set(count as f64);
        }
    }

    app.runtime
        .acquisition
        .download_queue_snapshot
        .stage_upserts(items.clone())
        .await;

    metrics::histogram!("scryer_download_queue_refresh_duration_seconds")
        .record(cycle_started_at.elapsed().as_secs_f64());

    tracing::debug!(
        elapsed_ms = cycle_started_at.elapsed().as_millis() as u64,
        item_count = items.len(),
        tracked_count = runtime.tracker.get_all().len(),
        active_workers = runtime.tracked_work_in_flight.len(),
        snapshot = snapshot_label,
        "download queue poller cycle completed"
    );
}

async fn reconcile_restart_ghost_bindings(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    items: &[DownloadQueueItem],
    excluded_client_type_refs: &[&str],
    authoritative_client_ids: &HashSet<String>,
) {
    let enabled_clients = match app.enabled_download_clients_by_priority().await {
        Ok(clients) => clients,
        Err(error) => {
            tracing::warn!(error = %error, "skipping absent-binding reconciliation without client configuration");
            return;
        }
    };
    let observed_before = chrono::Utc::now() - ABSENT_BINDING_RECONCILE_RECENCY_FLOOR;
    let mut remaining = ABSENT_BINDING_RECONCILE_BATCH_SIZE;

    for client in enabled_clients {
        if remaining == 0 {
            break;
        }
        if crate::tracked_downloads::tracked_client_type_is_excluded(
            &client.client_type,
            excluded_client_type_refs,
        ) {
            continue;
        }
        if !authoritative_client_ids.contains(&client.id) {
            continue;
        }

        let bindings = match app
            .services
            .workflow
            .download_registry
            .list_active_bindings_for_client_before(
                &client.id,
                &client.client_type,
                observed_before,
                remaining,
            )
            .await
        {
            Ok(bindings) => bindings,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    client_id = %client.id,
                    client_type = %client.client_type,
                    "failed to list active bindings for absent-download reconciliation"
                );
                continue;
            }
        };
        remaining = remaining.saturating_sub(bindings.len());

        for binding in bindings {
            let Some(native_item_id) = binding.native_item_id else {
                continue;
            };
            let observed = items.iter().any(|item| {
                item.client_id == client.id
                    && item.client_type.eq_ignore_ascii_case(&client.client_type)
                    && item.download_client_item_id == native_item_id
            });
            if observed {
                continue;
            }

            let source_identity = crate::ClientJobLocator::new(
                Some(client.id.as_str()),
                &client.client_type,
                native_item_id,
            );
            reconcile_authoritatively_absent_source(app, tracker, &source_identity).await;
        }
    }
}

pub(crate) async fn reconcile_authoritatively_absent_source(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    source_identity: &crate::ClientJobLocator,
) {
    let binding = match app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(source_identity)
        .await
    {
        Ok(Some(binding)) => binding,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id = ?source_identity.client_id,
                client_type = %source_identity.client_type,
                item_id = %source_identity.item_id,
                "failed to resolve active binding for unavailable download"
            );
            return;
        }
    };

    match authoritatively_absent_download_disposition(app, tracker, source_identity, &binding).await
    {
        AuthoritativelyAbsentDownloadDisposition::Preserve => return,
        AuthoritativelyAbsentDownloadDisposition::Fail => {
            fail_authoritatively_absent_download(
                app,
                tracker,
                source_identity,
                binding.download_id,
            )
            .await;
        }
        AuthoritativelyAbsentDownloadDisposition::Terminal => {}
    }

    if let Err(error) = app
        .services
        .workflow
        .download_registry
        .end_binding(&binding.download_id)
        .await
    {
        tracing::warn!(
            error = %error,
            download_id = %binding.download_id,
            client_id = ?source_identity.client_id,
            client_type = %source_identity.client_type,
            item_id = %source_identity.item_id,
            "failed to end binding for unavailable download"
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthoritativelyAbsentDownloadDisposition {
    Preserve,
    Terminal,
    Fail,
}

async fn authoritatively_absent_download_disposition(
    app: &AppUseCase,
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    source_identity: &crate::ClientJobLocator,
    binding: &crate::DownloadClientBindingRecord,
) -> AuthoritativelyAbsentDownloadDisposition {
    if let Some(id) = tracker
        .cached_id_for_source_identity_for_download(Some(&binding.download_id), source_identity)
        && let Some(tracked) = tracker.find(&id)
    {
        if crate::tracked_downloads::TrackedDownloadService::should_preserve_tracking(tracked.state)
        {
            return AuthoritativelyAbsentDownloadDisposition::Preserve;
        }
        if tracked.state.is_terminal() {
            return AuthoritativelyAbsentDownloadDisposition::Terminal;
        }
    }

    let durable_disposition = match app
        .services
        .workflow
        .download_submissions
        .get_identity_tracked_state_for_download(
            Some(&binding.download_id),
            &crate::DownloadSubmissionIdentity::default(),
            Some(source_identity),
        )
        .await
    {
        Ok(Some(state)) => match scryer_domain::TrackedDownloadState::from_str_opt(&state) {
            Some(state)
                if crate::tracked_downloads::TrackedDownloadService::should_preserve_tracking(
                    state,
                ) =>
            {
                AuthoritativelyAbsentDownloadDisposition::Preserve
            }
            Some(state) if state.is_terminal() => {
                AuthoritativelyAbsentDownloadDisposition::Terminal
            }
            Some(_) | None => AuthoritativelyAbsentDownloadDisposition::Fail,
        },
        Ok(None) => AuthoritativelyAbsentDownloadDisposition::Fail,
        Err(error) => {
            tracing::warn!(
                error = %error,
                download_id = %binding.download_id,
                "could not load durable unavailable download state; preserving until it can be read"
            );
            return AuthoritativelyAbsentDownloadDisposition::Preserve;
        }
    };
    if durable_disposition != AuthoritativelyAbsentDownloadDisposition::Fail {
        return durable_disposition;
    }

    match app
        .services
        .workflow
        .download_registry
        .load_download(&binding.download_id)
        .await
    {
        Ok(Some(download)) if download.terminal_at.is_some() => {
            AuthoritativelyAbsentDownloadDisposition::Terminal
        }
        Ok(Some(_)) => AuthoritativelyAbsentDownloadDisposition::Fail,
        Ok(None) => {
            tracing::warn!(
                download_id = %binding.download_id,
                "active download binding has no canonical download row; ending binding without replacing terminal state"
            );
            AuthoritativelyAbsentDownloadDisposition::Terminal
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                download_id = %binding.download_id,
                "could not determine unavailable download terminal state; ending binding without replacing terminal state"
            );
            AuthoritativelyAbsentDownloadDisposition::Terminal
        }
    }
}

async fn fail_authoritatively_absent_download(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    source_identity: &crate::ClientJobLocator,
    download_id: scryer_domain::download_identity::DownloadId,
) {
    let tracked_id =
        tracker.cached_id_for_source_identity_for_download(Some(&download_id), source_identity);

    if let Some(id) = tracked_id.as_deref() {
        if let Some(tracked) = tracker.find_mut(id) {
            tracked.state = scryer_domain::TrackedDownloadState::FailedPending;
            tracked.status = scryer_domain::TrackedDownloadStatus::Error;
            tracked.status_messages = vec![REMOVED_FROM_DOWNLOAD_CLIENT_REASON.to_string()];
            tracked.client_item.attention_reason =
                Some(REMOVED_FROM_DOWNLOAD_CLIENT_REASON.to_string());
        }
        if let Some(tracked) = tracker.find_mut(id) {
            crate::failed_download_handler::process_failed(app, tracked).await;
        }
        if let Some(tracked) = tracker.find_mut(id) {
            tracked.state = scryer_domain::TrackedDownloadState::Failed;
            tracked.status = scryer_domain::TrackedDownloadStatus::Error;
            if tracked.status_messages.is_empty() {
                tracked
                    .status_messages
                    .push(REMOVED_FROM_DOWNLOAD_CLIENT_REASON.to_string());
            }
        }
        if let Some(tracked) = tracker.find(id) {
            crate::tracked_downloads::persist_tracked_download_state_marker(
                app,
                tracked,
                scryer_domain::TrackedDownloadState::Failed,
                Some(REMOVED_FROM_DOWNLOAD_CLIENT_REASON),
                None,
            )
            .await;
        }
        return;
    }

    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(
            source_identity,
            scryer_domain::TrackedDownloadState::Failed.as_str(),
        )
        .await
    {
        tracing::warn!(
            error = %error,
            download_id = %download_id,
            "failed to record legacy tracked state for unavailable download"
        );
        return;
    }

    // Since 0184 the durable row is keyed by the canonical download id; the
    // wire identity is only a compatibility column, so a token-less item still
    // gets its failed marker.
    let identity = match app
        .services
        .workflow
        .download_submissions
        .get_submission_identity(source_identity)
        .await
    {
        Ok(identity) => identity.unwrap_or_default(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                download_id = %download_id,
                "failed to load submission identity for unavailable download"
            );
            return;
        }
    };
    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .record_identity_tracked_state_for_download(
            Some(&download_id),
            &identity,
            Some(source_identity),
            scryer_domain::TrackedDownloadState::Failed.as_str(),
            Some(REMOVED_FROM_DOWNLOAD_CLIENT_REASON),
            None,
        )
        .await
    {
        tracing::warn!(
            error = %error,
            download_id = %download_id,
            "failed to record canonical tracked state for unavailable download"
        );
    }
}

fn tracked_download_snapshot_projection_key(
    scope: &crate::tracked_downloads::TrackedDownloadSnapshotScope,
) -> Option<DownloadQueueProjectionSource> {
    match scope {
        crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id,
            client_type,
        } => Some(DownloadQueueProjectionSource::AuthoritativeBridge {
            client_type: client_type.trim().to_ascii_lowercase(),
            client_id: client_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        }),
        crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta => None,
    }
}

async fn process_external_tracked_download_snapshot_update(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    update: crate::tracked_downloads::TrackedDownloadSnapshotUpdate,
    excluded_client_type_refs: &[&str],
) {
    let crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
        scope,
        items,
        completed_downloads,
        actor_id: _,
    } = update;
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
    let authoritative_client_ids = match &prune {
        TrackedDownloadSnapshotPrune::Scope(
            crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
                client_id: Some(client_id),
                ..
            },
        ) => Some(HashSet::from([client_id.clone()])),
        _ => None,
    };
    let projection = match projection_key {
        Some(source) => TrackedDownloadSnapshotProjection::Publish { source },
        None => TrackedDownloadSnapshotProjection::UpsertOnly,
    };
    let emit_metrics = matches!(
        &projection,
        TrackedDownloadSnapshotProjection::Publish { .. }
    );
    let dispatch = TrackedDownloadSnapshotDispatch::Seen {
        completed_lookup: completed_lookup.clone(),
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
        authoritative_client_ids.as_ref(),
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
        tokio::sync::mpsc::unbounded_channel::<TrackedDownloadBackgroundWorkEvent>();

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
                if let Some(event) = maybe_result {
                    match event {
                        TrackedDownloadBackgroundWorkEvent::Finished(result) => {
                            handle_tracked_download_background_work_result(
                                &app,
                                &mut runtime.tracker,
                                &mut runtime.tracked_work_in_flight,
                                result,
                            )
                            .await;
                            let mut dispatched = false;
                            while try_dispatch_next_tracked_download_background_work(
                                &app,
                                &actor,
                                &mut runtime.tracker,
                                &mut runtime.tracked_work_in_flight,
                                &tracked_work_result_tx,
                                &mut runtime.tracked_work_drain,
                            ) {
                                dispatched = true;
                            }
                            if dispatched {
                                publish_runtime_tracked_download_snapshot_cache(&app, &runtime.tracker).await;
                            }
                        }
                    }
                }
            }
            _ = interval.tick() => {
                let active_bridged_client_types = bridged_client_types.snapshot();
                remove_ended_bridge_projections(
                    &mut runtime,
                    &active_bridged_client_types,
                    &static_excluded_client_types,
                );
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
                let client_refresh_started_at = Instant::now();
                let (snapshot, collection_error) = match app
                    .collect_download_snapshot_items_excluding_client_types(
                        true,
                        true,
                        false,
                        &excluded_client_type_refs,
                    )
                    .await
                {
                    Ok(snapshot) => (snapshot, None),
                    Err(error) => {
                        tracing::warn!(error = %error, "download queue poll collected no client snapshot");
                        (
                            crate::ports::DownloadClientSnapshotOutcome::default(),
                            Some(error.to_string()),
                        )
                    }
                };
                if !snapshot.any_client_read_succeeded {
                    let error = collection_error.unwrap_or_else(|| {
                        "all included download clients failed their queue/activity reads".to_string()
                    });
                    app.runtime
                        .acquisition
                        .download_queue_snapshot
                        .mark_refresh_failed(error)
                        .await;
                }
                // Freshness is republished every tick, viewer or not: this is
                // the age of the snapshot currently being served, sampled at a
                // known cadence instead of whenever someone opens the queue.
                metrics::gauge!(crate::services::DOWNLOAD_QUEUE_SNAPSHOT_AGE_SECONDS).set(
                    app.runtime
                        .acquisition
                        .download_queue_snapshot
                        .snapshot()
                        .await
                        .updated_at
                        .map(|updated_at| {
                            chrono::Utc::now()
                                .signed_duration_since(updated_at)
                                .num_milliseconds()
                                .max(0) as f64
                                / 1_000.0
                        })
                        .unwrap_or(0.0),
                );
                let crate::ports::DownloadClientSnapshotOutcome {
                    items,
                    authoritative_client_ids,
                    ..
                } = snapshot;
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
                        source: DownloadQueueProjectionSource::Poller,
                    },
                    TrackedDownloadSnapshotDispatch::AllTrackable,
                    true,
                    &excluded_client_type_refs,
                    Some(&authoritative_client_ids),
                    "poller",
                )
                .await;
                // Whole-tick wall time. Per-client refresh timing is labelled
                // and emitted by the router seam
                // (`scryer_download_client_refresh_duration_seconds`).
                metrics::histogram!(crate::services::DOWNLOAD_QUEUE_POLL_CYCLE_DURATION_SECONDS)
                    .record(client_refresh_started_at.elapsed().as_secs_f64());
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

fn resolve_tracked_command_id_for_download(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    requested_id: &str,
) -> String {
    let legacy = tracker.resolve_cached_id(requested_id);
    let canonical = canonical_download_id.and_then(|canonical_download_id| {
        tracker.cached_id_for_canonical_download_id(canonical_download_id)
    });
    if let (Some(canonical), Some(legacy)) = (&canonical, &legacy)
        && canonical != legacy
    {
        tracing::warn!(
            target: "download_identity_resolver",
            canonical_download_id = ?canonical_download_id,
            canonical_tracked_id = %canonical,
            legacy_tracked_id = %legacy,
            "manual import canonical and legacy tracked download lookups disagreed; using legacy tracked download"
        );
    }
    legacy
        .or(canonical)
        .unwrap_or_else(|| requested_id.to_string())
}
pub(crate) async fn assign_tracked_download_title_command(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    requested_id: String,
    title: scryer_domain::Title,
    mut submission: DownloadSubmission,
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

    // An assignment is an operator's explicit identity for the download and is
    // recorded like a grab (the store reads any titled row back as a Scryer
    // submission anyway). It names the requested scope, and it must not throw
    // away the grab-time indexer release name — that is still the best
    // release evidence for parsing and scoring, whatever title it lands in.
    let source_identity = ClientJobLocator::from_submission(&submission);
    if submission
        .source_title
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
        && let Some(existing) = app
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&source_identity)
            .await?
    {
        submission.source_title = existing
            .source_title
            .filter(|value| !value.trim().is_empty());
    }
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
    let activity_item = tracked_download_activity_queue_item(tracked);
    publish_runtime_tracked_download_and_activity_item(app, tracker, Some(activity_item)).await;
    Ok(())
}
async fn handle_tracked_download_command(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    tracked_work_result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    command: crate::tracked_downloads::TrackedDownloadCommand,
) {
    use crate::tracked_downloads::TrackedDownloadCommand;
    use scryer_domain::{TrackedDownloadState, TrackedDownloadStatus};

    match command {
        TrackedDownloadCommand::ReconcileManualImport {
            id,
            canonical_download_id,
            files_imported_this_pass,
            expected_mapping_count,
            reply,
        } => {
            let requested_id = id;
            let id = resolve_tracked_command_id_for_download(
                tracker,
                canonical_download_id.as_ref(),
                &requested_id,
            );
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let mut activity_item = None;
            let result = async {
                let tracked = tracker.find(&id).cloned().ok_or_else(|| {
                    AppError::NotFound(format!("tracked download {requested_id}"))
                })?;
                if !crate::completed_download_handler::verify_manual_import(
                    app,
                    &tracked,
                    files_imported_this_pass,
                    expected_mapping_count,
                )
                .await?
                {
                    return Ok(false);
                }

                let td = tracker
                    .find_mut(&id)
                    .expect("serialized tracked download disappeared after import verification");
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                activity_item = Some(tracked_download_activity_queue_item(td));
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Imported)
                    .await;
                Ok(true)
            }
            .await;
            if matches!(result, Ok(true)) {
                publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item)
                    .await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkImported {
            id,
            canonical_download_id,
            reply,
        } => {
            let requested_id = id;
            let id = resolve_tracked_command_id_for_download(
                tracker,
                canonical_download_id.as_ref(),
                &requested_id,
            );
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let mut activity_item = None;
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                activity_item = Some(tracked_download_activity_queue_item(td));
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
                publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item)
                    .await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkImportedIfAwaitingImport {
            source_identity,
            canonical_download_id,
            record_completed_at,
            reply,
        } => {
            use crate::tracked_downloads::{
                ManualImportRecoveryOutcome, ManualImportRecoveryVerdict,
                manual_import_recovery_verdict,
            };

            let Some(id) = tracker.cached_id_for_source_identity_for_download(
                canonical_download_id.as_ref(),
                &source_identity,
            ) else {
                let _ = reply.send(Ok(ManualImportRecoveryOutcome::Untracked));
                return;
            };
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Ok(ManualImportRecoveryOutcome::Busy));
                return;
            }
            let Some(tracked) = tracker.find_mut(&id) else {
                let _ = reply.send(Ok(ManualImportRecoveryOutcome::Untracked));
                return;
            };
            // A stale record must never terminalize a fresh download that
            // merely reuses the item id (see `manual_import_recovery_verdict`),
            // and an already-imported download is reported as unchanged so the
            // caller does not keep acting on it every tick.
            if manual_import_recovery_verdict(tracked, record_completed_at)
                != ManualImportRecoveryVerdict::MarkImported
            {
                let _ = reply.send(Ok(ManualImportRecoveryOutcome::Unchanged));
                return;
            }

            tracked.state = TrackedDownloadState::Imported;
            tracked.status = TrackedDownloadStatus::Ok;
            tracked.status_messages.clear();
            let activity_item = Some(tracked_download_activity_queue_item(tracked));
            tracker
                .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                .await;
            finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Imported)
                .await;
            publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item).await;
            let _ = reply.send(Ok(ManualImportRecoveryOutcome::Marked));
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
            let source_identity = tracker.find(&id).and_then(tracked_download_source_identity);
            let mut activity_item = None;
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
                        FinalizeIgnoredOutcome::Finalized
                        | FinalizeIgnoredOutcome::NoSubmission => {}
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
                activity_item = Some(tracked_download_activity_queue_item(td));
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Ignored)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Ignored)
                    .await;
                Ok(())
            }
            .await;
            if result.is_ok() {
                publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item)
                    .await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Forget { id, reply } => {
            let id = resolve_tracked_command_id(tracker, &id);
            tracker.stop_tracking(&id);
            publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            let _ = reply.send(Ok(()));
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
            let has_grabbed_submission = match tracker.find(&id) {
                Some(td) => {
                    crate::failed_download_handler::tracked_download_has_grabbed_submission(app, td)
                        .await
                }
                None => false,
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
                let activity_item = tracker.find(&id).map(tracked_download_activity_queue_item);
                publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item)
                    .await;
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
                let activity_item = tracker.find(&id).map(tracked_download_activity_queue_item);
                publish_runtime_tracked_download_and_activity_item(app, tracker, activity_item)
                    .await;
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
        TrackedDownloadCommand::CompletedSource { identity, reply } => {
            let _ = reply.send(tracker.completed_source_for_identity(&identity));
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
            if td.import_retry_deferred(chrono::Utc::now()) {
                return None;
            }
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
                if td.state == TrackedDownloadState::ImportPending && !td.import_retry_deferred(now)
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
        && !td.import_retry_deferred(now)
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
    let retry_items = excluded_completed_history_retry_items(
        tracker,
        tracked_work_in_flight,
        excluded_client_types,
    );
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
            if td.waiting_for_completed_history && td.state == TrackedDownloadState::ImportPending {
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
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
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
            && (td.state.is_import_settled()
                || td.state == TrackedDownloadState::Importing
                || (td.state == TrackedDownloadState::ImportBlocked && td.import_attempted))
        {
            continue;
        }
        if let Some(state) =
            crate::completed_download_handler::queue_item_identity_tracked_state(app, &item).await
            && (state.is_import_settled() || state == TrackedDownloadState::ImportBlocked)
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
    candidates.retain(|item| match completed_lookup.completed_at_for_item(item) {
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
    });
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
        TrackedDownloadSnapshotProjection::UpsertOnly,
        TrackedDownloadSnapshotDispatch::Seen { completed_lookup },
        false,
        excluded_client_type_refs,
        None,
        "history-reconcile",
    )
    .await;
}

async fn try_dispatch_excluded_completed_history_retry(
    app: &AppUseCase,
    actor: &User,
    runtime: &mut TrackedDownloadRuntimeState,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    excluded_client_types: &[&str],
) {
    if excluded_client_types.is_empty() || runtime.tracked_work_drain.has_pending() {
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

    let mut dispatched = false;
    while try_dispatch_next_tracked_download_background_work(
        app,
        actor,
        &mut runtime.tracker,
        &mut runtime.tracked_work_in_flight,
        result_tx,
        &mut runtime.tracked_work_drain,
    ) {
        dispatched = true;
    }
    if dispatched || revalidated {
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
    let mut remaining = drain.pending_ids.len();
    while remaining > 0 {
        remaining -= 1;
        let id = drain.pending_ids.pop_front()?;
        if drain.attempted_ids.contains(&id) {
            continue;
        }
        if tracked_work_in_flight.contains(&id) {
            continue;
        }
        if let Some((kind, tracked)) =
            prepare_tracked_download_background_work_dispatch(tracker, &id)
        {
            if kind == TrackedDownloadBackgroundWorkKind::Failed
                && failed_tracked_download_work_count(tracker, tracked_work_in_flight)
                    >= TRACKED_DOWNLOAD_FAILED_WORKER_LIMIT
            {
                drain.pending_ids.push_back(id);
                continue;
            }
            drain.attempted_ids.insert(id.clone());
            return Some((id, kind, tracked));
        }
        drain.attempted_ids.insert(id);
    }

    None
}

fn failed_tracked_download_work_count(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
) -> usize {
    tracked_work_in_flight
        .iter()
        .filter(|id| {
            tracker
                .find(id)
                .is_some_and(|tracked| tracked.state == TrackedDownloadState::FailedPending)
        })
        .count()
}

fn try_dispatch_next_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    drain: &mut TrackedDownloadWorkDrain,
) -> bool {
    let Some((id, kind, mut tracked)) = prepare_next_tracked_download_background_work_dispatch(
        tracker,
        tracked_work_in_flight,
        drain,
    ) else {
        return false;
    };

    let preparation_permit = if kind == TrackedDownloadBackgroundWorkKind::Import {
        let Some(permit) = app
            .runtime
            .imports
            .execution_coordinator
            .try_acquire_preparation()
        else {
            drain.attempted_ids.remove(&id);
            drain.pending_ids.push_front(id);
            return false;
        };
        if let Some(live) = tracker.find_mut(&id) {
            crate::completed_download_handler::mark_importing(live);
            tracked = live.clone();
        }
        Some(permit)
    } else {
        None
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
        preparation_permit,
    );
    true
}

fn try_dispatch_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    id: &str,
    completed_lookup: &crate::completed_download_handler::CompletedDownloadLookup,
) -> bool {
    if tracked_work_in_flight.contains(id) {
        return false;
    }

    let Some((kind, mut tracked)) = prepare_tracked_download_background_work_dispatch(tracker, id)
    else {
        return false;
    };
    if kind == TrackedDownloadBackgroundWorkKind::Failed
        && failed_tracked_download_work_count(tracker, tracked_work_in_flight)
            >= TRACKED_DOWNLOAD_FAILED_WORKER_LIMIT
    {
        return false;
    }

    let preparation_permit = if kind == TrackedDownloadBackgroundWorkKind::Import {
        let Some(permit) = app
            .runtime
            .imports
            .execution_coordinator
            .try_acquire_preparation()
        else {
            return false;
        };
        if let Some(live) = tracker.find_mut(id) {
            crate::completed_download_handler::mark_importing(live);
            tracked = live.clone();
        }
        Some(permit)
    } else {
        None
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
        preparation_permit,
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
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    id: &str,
    kind: TrackedDownloadBackgroundWorkKind,
    tracked: TrackedDownload,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) {
    tracing::info!(
        id = %id,
        work = kind.as_str(),
        active_workers = tracked_work_in_flight.len() + 1,
        failed_worker_limit = TRACKED_DOWNLOAD_FAILED_WORKER_LIMIT,
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
        preparation_permit,
    );
}
fn dispatch_tracked_download_background_work(
    app: AppUseCase,
    actor: User,
    tracked: crate::tracked_downloads::TrackedDownload,
    kind: TrackedDownloadBackgroundWorkKind,
    result_tx: tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkEvent>,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    preparation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) {
    tokio::spawn(async move {
        let started_at = Instant::now();
        let tracked_id = tracked.id.clone();
        let worker = std::panic::AssertUnwindSafe(async move {
            let mut tracked = tracked;

            match kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    let preparation_permit = preparation_permit
                        .expect("import dispatch requires a preparation permit");
                    let _ = crate::completed_download_handler::import_with_lookup_and_preparation_permit(
                        &app,
                        &actor,
                        &mut tracked,
                        &completed_lookup,
                        preparation_permit,
                    )
                    .await;
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    crate::failed_download_handler::process_failed(&app, &mut tracked).await;
                }
            }

            tracked
        })
        .catch_unwind()
        .await;

        let outcome = match worker {
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
            Err(_) => {
                let message = format!(
                    "tracked {} worker panicked before completion",
                    kind.as_str()
                );
                tracing::error!(
                    id = %tracked_id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "tracked: background work crashed"
                );
                Err(message)
            }
        };
        let elapsed = started_at.elapsed();
        if result_tx
            .send(TrackedDownloadBackgroundWorkEvent::Finished(
                TrackedDownloadBackgroundWorkResult {
                    id: tracked_id,
                    kind,
                    outcome,
                    elapsed,
                },
            ))
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
    let activity_item = tracked_download_activity_queue_item(tracked);

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
        crate::tracked_downloads::persist_import_blocked_state_marker(
            app,
            td,
            crate::tracked_downloads::ImportBlockedReason::AfterImport,
            td.status_messages.first().map(String::as_str),
        )
        .await;
    }

    publish_runtime_tracked_download_and_activity_item(app, tracker, Some(activity_item)).await;
}
/// Why a terminal settle is running.
///
/// A settled row that is still present in the client is re-created and
/// re-offered to the gate on **every** poll — that re-offering is what
/// eventually releases a parked torrent, but it also means an unguarded
/// lifecycle event would be recorded once per tick forever. Lifecycle history
/// therefore belongs to the transition, not to the re-offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalSettleTrigger {
    /// The row just reached this terminal state (an import finished, or an
    /// operator marked/ignored it).
    Transition,
    /// The reconcile tick re-offering an already-settled row.
    Reconcile,
}

pub(crate) async fn finalize_tracked_terminal_state(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
    state: TrackedDownloadState,
) {
    finalize_tracked_terminal_state_with(
        app,
        tracker,
        id,
        state,
        TerminalSettleTrigger::Transition,
        None,
    )
    .await;
}

/// As `finalize_tracked_terminal_state`, reusing the reconcile tick's shared
/// reads. One-off callers (commands, recovery) pass `None` and take the
/// per-row path.
pub(crate) async fn finalize_tracked_terminal_state_with(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
    state: TrackedDownloadState,
    trigger: TerminalSettleTrigger,
    cache: Option<&crate::import::import::TerminalCleanupTickCache>,
) {
    let Some(td) = tracker.find(id) else {
        return;
    };

    // The settled download must stop conflicting new submissions for its
    // scope: the submission guard's 30s caches still hold it as accepted and
    // the cached client snapshot predates this transition, which would turn an
    // upgrade queued in that window into a phantom non-replaceable conflict.
    // Only the transition invalidates — reconcile re-offers a held row every
    // poll and would otherwise empty the caches on every tick.
    if trigger == TerminalSettleTrigger::Transition
        && let Some(title_id) = td.title_id.as_deref()
    {
        app.runtime
            .acquisition
            .download_submission_guards
            .forget_settled_download(title_id);
    }

    let cleanup = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        app, td, state, cache,
    )
    .await;

    if cleanup.outcome == crate::import::import::TerminalDownloadCleanupOutcome::HeldForSeeding {
        if state == TrackedDownloadState::Failed
            && tracker
                .find(id)
                .is_some_and(|tracked| tracked.burned_by_import_gate)
        {
            // Keep the burned release visibly failed while its torrent remains
            // under the same seeding obligation as an imported download; it
            // deliberately records no seeding history while it is held.
            const HELD_BURNED_TORRENT_MESSAGE: &str = "Kept in the download client until its seeding goal is met; the entry and its data are removed then.";
            if let Some(tracked) = tracker.find_mut(id)
                && !tracked
                    .status_messages
                    .iter()
                    .any(|message| message == HELD_BURNED_TORRENT_MESSAGE)
            {
                tracked
                    .status_messages
                    .push(HELD_BURNED_TORRENT_MESSAGE.to_string());
            }
            return;
        }
        // Only the transition *into* the hold is history. A held torrent is
        // re-offered to the gate on every poll, and one event per tick would
        // bury the feed under the same fact.
        if state != TrackedDownloadState::ImportedSeeding
            && let Some(report) = cleanup.seeding
            && let Some(td) = tracker.find(id)
        {
            record_seeding_started_event(app, td, report).await;
        }
        park_tracked_download_in_imported_seeding(app, tracker, id).await;
        return;
    }

    if crate::import::import::terminal_download_cleanup_is_complete(cleanup.outcome) {
        // Closes the retention window this row's `seeding_started` opened. A
        // torrent that was never held has no window to close, so it gets no
        // seeding history — with one exception: a post-import handoff is a
        // one-shot fact ("Scryer stopped managing this torrent") that has to be
        // recorded even though nothing was ever retained. It is recorded only
        // on the transition, because a handed-off entry stays in the client and
        // is re-offered to the gate on every subsequent poll.
        let records_seeding_history = state == TrackedDownloadState::ImportedSeeding
            || (cleanup.outcome
                == crate::import::import::TerminalDownloadCleanupOutcome::HandedOff
                && trigger == TerminalSettleTrigger::Transition);
        if records_seeding_history && let Some(td) = tracker.find(id) {
            record_seeding_completed_event(app, td, cleanup.seeding).await;
        }
        // A held torrent that has now discharged its obligation graduates to
        // the real terminal state before it stops being tracked, so restart
        // recovery reads `imported`, not `imported_seeding`.
        if state == TrackedDownloadState::ImportedSeeding {
            promote_imported_seeding_to_imported(app, tracker, id).await;
        }
        tracker.stop_tracking(id);
    } else if let Some(td) = tracker.find_mut(id) {
        td.completed_source = None;
    }
}

/// The title and provider label both seeding events carry.
async fn seeding_event_context(
    app: &AppUseCase,
    tracked: &TrackedDownload,
) -> (Option<scryer_domain::Title>, Option<String>) {
    let title = match tracked.title_id.as_deref() {
        Some(title_id) => app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let source_provider =
        crate::integration::workflow::source_provider_label(tracked.indexer.as_deref(), None);
    (title, source_provider)
}

/// Record that a torrent was imported and its client entry is being retained
/// while it seeds — the event that opens the retention window.
async fn record_seeding_started_event(
    app: &AppUseCase,
    tracked: &TrackedDownload,
    report: crate::import::import::SeedingGateReport,
) {
    let (title, source_provider) = seeding_event_context(app, tracked).await;
    let payload =
        scryer_domain::DomainEventPayload::SeedingStarted(scryer_domain::SeedingStartedEventData {
            title: title
                .as_ref()
                .map(crate::domain_events::title_context_snapshot),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            client_id: (!tracked.client_id.trim().is_empty()).then(|| tracked.client_id.clone()),
            client_type: Some(tracked.client_type.clone()),
            source_provider,
            source_title: tracked.source_title.clone(),
            reason: report.reason.to_string(),
            seed_ratio: report.seed_ratio,
            seed_time_seconds: report.seed_time_seconds,
        });
    append_seeding_event(app, title.as_ref(), payload).await;
}

/// Record that the seeding obligation was discharged and what the gate did with
/// the client entry — the event that closes the retention window.
///
/// The report is absent when the gate never ran for this settle (the operator
/// turned `remove_completed` off while the torrent was parked); the window
/// still closes, with nothing removed.
async fn record_seeding_completed_event(
    app: &AppUseCase,
    tracked: &TrackedDownload,
    report: Option<crate::import::import::SeedingGateReport>,
) {
    let (title, source_provider) = seeding_event_context(app, tracked).await;
    let action = report
        .and_then(|report| report.action)
        .unwrap_or(crate::import::import::SeedingReleaseAction::Kept);
    let payload = scryer_domain::DomainEventPayload::SeedingCompleted(
        scryer_domain::SeedingCompletedEventData {
            title: title
                .as_ref()
                .map(crate::domain_events::title_context_snapshot),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            client_id: (!tracked.client_id.trim().is_empty()).then(|| tracked.client_id.clone()),
            client_type: Some(tracked.client_type.clone()),
            source_provider,
            source_title: tracked.source_title.clone(),
            action: action.as_str().to_string(),
            reason: report
                .map(|report| report.reason.to_string())
                .unwrap_or_else(|| "removal_not_configured".to_string()),
            seed_ratio: report.and_then(|report| report.seed_ratio),
            seed_time_seconds: report.and_then(|report| report.seed_time_seconds),
        },
    );
    append_seeding_event(app, title.as_ref(), payload).await;
}

/// A failed append must never disturb the seeding lifecycle: the events are a
/// record of what happened, not part of the decision.
async fn append_seeding_event(
    app: &AppUseCase,
    title: Option<&scryer_domain::Title>,
    payload: scryer_domain::DomainEventPayload,
) {
    // The poller acts on its own; there is no operator behind a seeding
    // transition, the same way there is none behind an automatic import.
    let actor = crate::DomainEventActor::system();
    let event = match title {
        Some(title) => crate::domain_events::new_title_domain_event(actor, title, payload),
        None => crate::domain_events::new_global_domain_event(actor, payload),
    };
    if let Err(error) = app.append_domain_event(event).await {
        tracing::warn!(error = %error, "failed to record a seeding history event");
    }
}

/// Park an imported-but-still-seeding torrent. The row stays in the tracker
/// (and therefore in the queue) and re-enters the gate on the next poll.
async fn park_tracked_download_in_imported_seeding(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
) {
    let Some(td) = tracker.find_mut(id) else {
        return;
    };
    if td.state == TrackedDownloadState::ImportedSeeding {
        // Already parked; re-persisting on every poll would be a write per
        // tick per held torrent for no new information.
        return;
    }
    td.state = TrackedDownloadState::ImportedSeeding;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    let snapshot = td.clone();
    tracing::info!(
        id = %snapshot.id,
        client_id = snapshot.client_id.as_str(),
        client_type = snapshot.client_type.as_str(),
        "tracked: imported, holding the client entry until the seeding goal is met"
    );
    crate::tracked_downloads::persist_tracked_download_state_marker(
        app,
        &snapshot,
        TrackedDownloadState::ImportedSeeding,
        Some("imported_seeding"),
        None,
    )
    .await;
}

async fn promote_imported_seeding_to_imported(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
) {
    let Some(td) = tracker.find_mut(id) else {
        return;
    };
    td.state = TrackedDownloadState::Imported;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    let snapshot = td.clone();
    tracker
        .persist_terminal_state(app, &snapshot.id, TrackedDownloadState::Imported)
        .await;
}
async fn reconcile_terminal_tracked_downloads(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    reconcile_duplicate_terminal_source_states(tracker);

    // `ImportedSeeding` is not terminal, but it has to be re-offered to the
    // gate on every poll — that re-evaluation is what eventually releases the
    // torrent once its goal is met.
    let settled: Vec<&TrackedDownload> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.state.is_import_settled())
        .collect();
    if settled.is_empty() {
        return;
    }

    let terminal_ids: Vec<(String, TrackedDownloadState)> = settled
        .iter()
        .map(|tracked| (tracked.id.clone(), tracked.state))
        .collect();
    // Every settled row is re-offered to the gate below, and each one used to
    // pay for its own seed-goal query, title lookup and routing-entry read.
    // One batched prefetch up front, then memoized reads for the rest — held
    // torrents are the common case, so this is a per-tick cost that would
    // otherwise scale with the seeding backlog.
    let identities: Vec<crate::ClientJobLocator> = settled
        .iter()
        .filter_map(|tracked| tracked_download_source_identity(tracked))
        .collect();
    let cache = crate::import::import::TerminalCleanupTickCache::prefetch(app, &identities).await;

    for (id, state) in terminal_ids {
        finalize_tracked_terminal_state_with(
            app,
            tracker,
            &id,
            state,
            TerminalSettleTrigger::Reconcile,
            Some(&cache),
        )
        .await;
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

    // `is_import_settled` rather than `is_terminal`: a row already parked in
    // `ImportedSeeding` has been through the gate, and flipping it back to a
    // sibling's `Imported` would only send it straight back through on the
    // next poll.
    let updates: Vec<(String, crate::ClientJobLocator, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| !tracked.state.is_import_settled())
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

fn tracked_download_source_identity(tracked: &TrackedDownload) -> Option<crate::ClientJobLocator> {
    let client_type = tracked.client_type.trim();
    let item_id = tracked.client_item.download_client_item_id.trim();
    if client_type.is_empty() || item_id.is_empty() {
        return None;
    }
    Some(crate::ClientJobLocator::new(
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
    use super::{
        DownloadQueueProjectionSource, TrackedDownloadRuntimeState,
        effective_excluded_client_types, remove_ended_bridge_projections,
    };
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

    #[test]
    fn ended_bridge_projection_is_removed_before_fallback_poll_reconciliation() {
        let mut runtime = TrackedDownloadRuntimeState::new();
        runtime.previous_items_by_projection.insert(
            DownloadQueueProjectionSource::AuthoritativeBridge {
                client_type: "weaver".to_string(),
                client_id: Some("bridge-1".to_string()),
            },
            Default::default(),
        );
        runtime.previous_items_by_projection.insert(
            DownloadQueueProjectionSource::AuthoritativeBridge {
                client_type: "sabnzbd".to_string(),
                client_id: Some("external-1".to_string()),
            },
            Default::default(),
        );

        remove_ended_bridge_projections(&mut runtime, &[], &["sabnzbd".to_string()]);

        assert!(!runtime.previous_items_by_projection.keys().any(|source| {
            matches!(
                source,
                DownloadQueueProjectionSource::AuthoritativeBridge { client_type, .. }
                    if client_type == "weaver"
            )
        }));
        assert!(runtime.previous_items_by_projection.keys().any(|source| {
            matches!(
                source,
                DownloadQueueProjectionSource::AuthoritativeBridge { client_type, .. }
                    if client_type == "sabnzbd"
            )
        }));
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

#[cfg(test)]
mod ignored_submission_scope_release_tests {
    use super::scope_rows_released_by_ignored_submission;
    use crate::{AcquisitionScopeState, AcquisitionScopeStatus, DownloadSubmission, SubmissionScope};

    const SPHD: &str = "Desert.Warrior.2025.1080p.BluRay.DD+5.1.x264-SPHD";
    const REMUX: &str = "Desert.Warrior.2025.1080p.BluRay.REMUX.AVC.DTS-HD.MA.5.1-GRP";

    fn ignored_submission(source_title: Option<&str>, scope: SubmissionScope) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: Default::default(),
            facet: "movie".to_string(),
            download_client_id: Some("sab".to_string()),
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "nzo_1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: source_title.map(str::to_string),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope,
        }
    }

    fn row(
        id: &str,
        status: AcquisitionScopeStatus,
        grabbed_release: Option<&str>,
        episode_id: Option<&str>,
    ) -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: episode_id.map(str::to_string),
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: if episode_id.is_some() { "episode" } else { "movie" }.to_string(),
            last_search_at: None,
            status,
            grabbed_release: grabbed_release.map(str::to_string),
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2026-08-29T14:00:00Z".to_string(),
            updated_at: "2026-08-29T14:42:00Z".to_string(),
        }
    }

    fn grabbed(title: &str) -> String {
        format!(r#"{{"title":"{title}","score":400,"grabbed_at":"2026-08-29T14:42:00Z"}}"#)
    }

    fn released_ids(
        submission: &DownloadSubmission,
        rows: &[AcquisitionScopeState],
    ) -> Vec<String> {
        scope_rows_released_by_ignored_submission(submission, rows, &[])
            .into_iter()
            .map(|row| row.id.clone())
            .collect()
    }

    /// The scope row's claim has no clock, so an operator remove or ignore is
    /// what releases it: the row grabbed for this release goes back to wanted.
    /// Case and separator differences between the submission's release name
    /// and the row's recorded grab fold away.
    #[test]
    fn an_ignored_download_releases_the_row_grabbed_for_its_release() {
        let submission = ignored_submission(
            Some("desert warrior 2025 1080p bluray dd+5 1 x264-sphd"),
            SubmissionScope::Title,
        );
        let rows = vec![row(
            "scope-1",
            AcquisitionScopeStatus::Grabbed,
            Some(&grabbed(SPHD)),
            None,
        )];
        assert_eq!(released_ids(&submission, &rows), vec!["scope-1".to_string()]);
    }

    /// A row grabbed for a *different* release belongs to that other download
    /// (a replacement grabbed after this one) and must stay claimed.
    #[test]
    fn a_row_grabbed_for_another_release_stays_claimed() {
        let submission = ignored_submission(Some(SPHD), SubmissionScope::Title);
        let rows = vec![row(
            "scope-1",
            AcquisitionScopeStatus::Grabbed,
            Some(&grabbed(REMUX)),
            None,
        )];
        assert!(released_ids(&submission, &rows).is_empty());
    }

    /// Only a `grabbed` row is released: a completed scope already has its
    /// file, a wanted one has nothing to release.
    #[test]
    fn only_grabbed_rows_are_released() {
        let submission = ignored_submission(Some(SPHD), SubmissionScope::Title);
        for status in [
            AcquisitionScopeStatus::Wanted,
            AcquisitionScopeStatus::Paused,
            AcquisitionScopeStatus::Completed,
        ] {
            let rows = vec![row("scope-1", status, Some(&grabbed(SPHD)), None)];
            assert!(released_ids(&submission, &rows).is_empty(), "{status:?}");
        }
    }

    /// When the release cannot be compared (no recorded grab, or a submission
    /// with no release name) the row is released rather than held forever.
    #[test]
    fn an_unreadable_comparison_releases_rather_than_holds() {
        let rows = vec![row("scope-1", AcquisitionScopeStatus::Grabbed, None, None)];
        let submission = ignored_submission(Some(SPHD), SubmissionScope::Title);
        assert_eq!(released_ids(&submission, &rows), vec!["scope-1".to_string()]);

        let rows = vec![row(
            "scope-1",
            AcquisitionScopeStatus::Grabbed,
            Some(&grabbed(REMUX)),
            None,
        )];
        let submission = ignored_submission(None, SubmissionScope::Title);
        assert_eq!(released_ids(&submission, &rows), vec!["scope-1".to_string()]);
    }

    /// Scope membership still applies: an episode download releases its own
    /// episode's row and leaves its siblings alone.
    #[test]
    fn an_episode_download_releases_only_its_own_episode_row() {
        let submission = ignored_submission(
            Some("Show.S01E01.1080p.WEB-DL-GRP"),
            SubmissionScope::Episode {
                episode_id: "ep-1".to_string(),
            },
        );
        let rows = vec![
            row(
                "scope-ep-1",
                AcquisitionScopeStatus::Grabbed,
                Some(&grabbed("Show.S01E01.1080p.WEB-DL-GRP")),
                Some("ep-1"),
            ),
            row(
                "scope-ep-2",
                AcquisitionScopeStatus::Grabbed,
                Some(&grabbed("Show.S01E02.1080p.WEB-DL-GRP")),
                Some("ep-2"),
            ),
        ];
        assert_eq!(released_ids(&submission, &rows), vec!["scope-ep-1".to_string()]);
    }
}
