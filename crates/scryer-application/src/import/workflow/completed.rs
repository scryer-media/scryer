/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .subtitle_settings()
            .await
            .ok()
            .map(|settings| settings.auto_download_on_import)
            .unwrap_or(false);
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}

async fn analyze_and_persist_imported_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    file_path: &std::path::Path,
) {
    let acceptance = match app
        .services
        .library
        .media_analyzer
        .analyze_file(file_path.to_path_buf())
        .await
    {
        Ok(crate::MediaAnalysisOutcome::Valid(analysis)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: Some(*analysis),
                scan_error: None,
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Ok(crate::MediaAnalysisOutcome::Invalid(error)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id,
                file_id = %media_file_id,
                file_path = %file_path.display(),
                "failed to analyze imported media file"
            );
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
    };

    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        media_file_id,
        &acceptance,
    )
    .await;
}

fn completed_download_identity(completed: &CompletedDownload) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    )
}
fn additional_import_dest_path(
    canonical_dest_path: &Path,
    parsed: &ParsedReleaseMetadata,
) -> PathBuf {
    let parent = canonical_dest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let stem = canonical_dest_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("additional");
    let extension = canonical_dest_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let raw_label = parsed
        .edition
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(parsed.raw_title.as_str());
    let sanitized_label = sanitize_filesystem_component(raw_label)
        .trim()
        .chars()
        .take(48)
        .collect::<String>();
    let label = if sanitized_label.is_empty() {
        "additional".to_string()
    } else {
        sanitized_label
    };
    let hash = blake3::hash(parsed.raw_title.as_bytes()).to_hex();
    let hash = &hash.as_str()[..8];
    let base_name = sanitize_filesystem_component(&format!("{stem} - {label} {hash}.{extension}"));
    let mut candidate = parent.join(&base_name);
    if !candidate.exists() {
        return candidate;
    }

    for suffix in 2..=999 {
        let name = sanitize_filesystem_component(&format!(
            "{stem} - {label} {hash} ({suffix}).{extension}"
        ));
        candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(sanitize_filesystem_component(&format!(
        "{stem} - {label} {hash} {}.{extension}",
        Id::new().0
    )))
}
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";
const SCRYER_FACET_PARAM: &str = "*scryer_facet";
const SCRYER_COLLECTION_ID_PARAM: &str = "*scryer_collection_id";
const SCRYER_SERIES_MOVIE_LINK_ID_PARAM: &str = "*scryer_series_movie_link_id";

/// The pure "stamp" step of provenance resolution: a completed download whose
/// live submission is a Scryer grab carries that grab's identity parameters
/// (authoritative over whatever the client echoed) and its persisted indexer
/// release title as `release_name`. A submission recorded without a release
/// title must not blank a real client-reported name; the completed download
/// keeps it.
fn stamp_scryer_submission_origin(
    completed: &CompletedDownload,
    submission: &DownloadSubmission,
) -> CompletedDownload {
    let mut resolved = completed.clone();
    resolved.parameters = authoritative_scryer_origin_parameters(&completed.parameters, submission);
    resolved.release_name =
        submission_source_title(submission).or_else(|| completed_observed_release_name(completed));
    resolved
}

fn authoritative_scryer_origin_parameters(
    parameters: &[(String, String)],
    submission: &DownloadSubmission,
) -> Vec<(String, String)> {
    let mut resolved = parameters
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                SCRYER_TITLE_ID_PARAM
                    | SCRYER_FACET_PARAM
                    | SCRYER_COLLECTION_ID_PARAM
                    | SCRYER_SERIES_MOVIE_LINK_ID_PARAM
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    if !submission.title_id.trim().is_empty() {
        resolved.push((
            SCRYER_TITLE_ID_PARAM.to_string(),
            submission.title_id.clone(),
        ));
    }
    if !submission.facet.trim().is_empty() {
        resolved.push((SCRYER_FACET_PARAM.to_string(), submission.facet.clone()));
    }
    match &submission.scope {
        SubmissionScope::Collection { collection_id } => {
            resolved.push((
                SCRYER_COLLECTION_ID_PARAM.to_string(),
                collection_id.clone(),
            ));
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => {
            resolved.push((
                SCRYER_SERIES_MOVIE_LINK_ID_PARAM.to_string(),
                series_movie_link_id.clone(),
            ));
        }
        SubmissionScope::Episode { .. }
        | SubmissionScope::EpisodeSet { .. }
        | SubmissionScope::Title
        | SubmissionScope::Orphan => {}
    }
    resolved
}
async fn terminal_download_item_is_still_visible(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
) -> bool {
    let locator =
        crate::ClientJobLocator::new(Some(client_id), client_type, download_client_item_id);
    let lookup = app
        .services
        .integrations
        .download_client
        .observe_download(&locator, 0)
        .await;

    match lookup {
        Ok(crate::DownloadClientObservation::Absent) => false,
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                "failed to confirm download item visibility after delete error"
            );
            true
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDownloadCleanupOutcome {
    NotConfigured,
    Removed,
    AlreadyGone,
    RetryableFailure,
    /// The torrent is imported but has not discharged its seeding obligation.
    /// The tracked download stays visible in `ImportedSeeding` and re-enters
    /// the gate on the next poll.
    HeldForSeeding,
    /// The seeding obligation is discharged but the profile (or the client's
    /// own nature, as with `torrent-blackhole`) says the entry stays. Nothing
    /// further to reconcile.
    SeedingEntryKept,
    /// The profile's post-import tracking is `HandOff`: the download settles
    /// with the client entry untouched and Scryer stops managing the torrent.
    /// Kept distinct from `SeedingEntryKept` so logs, tests and the history
    /// event can tell "the goal was met and the profile keeps the entry" from
    /// "the operator opted out of management".
    HandedOff,
}
/// What the gate actually did with a client entry it released.
///
/// Distinct from `SeedGoalMetAction`, which is the profile's *intent*: a
/// `StopSeeding` profile on a client that cannot pause degrades to `Kept`, and
/// the history has to say what happened, not what was wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedingReleaseAction {
    Removed,
    Paused,
    Kept,
    /// The entry was already gone from the client when the gate looked.
    Vanished,
    /// The entry was left exactly as it is and Scryer stopped managing the
    /// torrent, per the profile's post-import tracking.
    HandedOff,
}

impl SeedingReleaseAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Paused => "paused",
            Self::Kept => "kept",
            Self::Vanished => "vanished",
            Self::HandedOff => "handed_off",
        }
    }
}

/// The seeding gate's verdict for one terminal cleanup, carried back for the
/// seeding history events. Absent when the gate never ran (usenet, or removal
/// disabled), in which case no seeding history is recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeedingGateReport {
    /// The gate's reason constant, verbatim — the same string the queue
    /// projection derives its badge from.
    pub reason: &'static str,
    /// What happened to the entry; `None` while it is still held.
    pub action: Option<SeedingReleaseAction>,
    /// Observed at the moment of the decision, when the client reports it.
    pub seed_ratio: Option<f64>,
    pub seed_time_seconds: Option<i64>,
}

/// A terminal cleanup's outcome plus, when the seeding gate ran, its verdict.
///
/// Compares equal to a bare `TerminalDownloadCleanupOutcome` so every existing
/// call site and assertion reads as before while the seeding detail rides
/// along for the history events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerminalDownloadCleanup {
    pub outcome: TerminalDownloadCleanupOutcome,
    pub seeding: Option<SeedingGateReport>,
    /// What host-side payload deletion achieved, when Scryer managed it.
    /// `None` when the client deleted its own data or no data removal ran.
    pub payload: Option<HostPayloadDisposition>,
}

impl TerminalDownloadCleanup {
    fn bare(outcome: TerminalDownloadCleanupOutcome) -> Self {
        Self {
            outcome,
            seeding: None,
            payload: None,
        }
    }

    fn gated(outcome: TerminalDownloadCleanupOutcome, seeding: SeedingGateReport) -> Self {
        Self {
            outcome,
            seeding: Some(seeding),
            payload: None,
        }
    }
}

impl PartialEq<TerminalDownloadCleanupOutcome> for TerminalDownloadCleanup {
    fn eq(&self, other: &TerminalDownloadCleanupOutcome) -> bool {
        self.outcome == *other
    }
}

pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
            | TerminalDownloadCleanupOutcome::SeedingEntryKept
            | TerminalDownloadCleanupOutcome::HandedOff
    )
}
pub(crate) async fn cleanup_routing_scope_for_title_id(
    app: &AppUseCase,
    title_id: Option<&str>,
) -> (Option<String>, Option<MediaFacet>) {
    let Some(title_id) = title_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => (Some(title.library_id), Some(title.facet)),
        Ok(None) | Err(_) => (None, None),
    }
}

/// Library and facet a terminal cleanup routes by, resolved from its title.
type CleanupRoutingScope = (Option<String>, Option<MediaFacet>);

/// What `should_remove_completed_download` actually depends on: routing scope
/// plus the client key the entry would be removed from.
type RemovalPolicyKey = (Option<String>, MediaFacet, String);

/// The reads every settled tracked row in one reconcile tick would otherwise
/// repeat for itself.
///
/// `reconcile_terminal_tracked_downloads` re-offers *every* settled row to the
/// removal gate on every poll — that re-offering is what eventually releases a
/// held torrent — so the per-row cost is paid once per row per tick, forever,
/// for as long as a torrent is held. Without this each row runs its own
/// seed-goal query, its own title lookup and its own routing-entry read, and
/// rows of the same title (a season pack) or the same client repeat those
/// answers verbatim.
///
/// Deliberately scoped to one tick and never reused across ticks: routing
/// configuration and persisted goals can change between polls, and acting on a
/// stale `remove_completed` or a stale goal is exactly the class of mistake
/// that removes a torrent still under obligation.
pub(crate) struct TerminalCleanupTickCache {
    goals: crate::seeding_gate::SeedGoalBatch,
    routing_scopes: std::sync::Mutex<HashMap<String, CleanupRoutingScope>>,
    remove_completed: std::sync::Mutex<HashMap<RemovalPolicyKey, bool>>,
    /// Reads that actually reached a repository, so a test can pin the hoist
    /// itself rather than only the shape of the key.
    routing_scope_reads: std::sync::atomic::AtomicUsize,
    remove_completed_reads: std::sync::atomic::AtomicUsize,
}

impl TerminalCleanupTickCache {
    /// Prefetch the seed goals for every settled row in this tick in one query.
    /// The memoized caches start empty and fill as rows are reconciled.
    pub(crate) async fn prefetch(app: &AppUseCase, identities: &[ClientJobLocator]) -> Self {
        Self {
            goals: crate::seeding_gate::SeedGoalBatch::prefetch(app, identities).await,
            routing_scopes: std::sync::Mutex::new(HashMap::new()),
            remove_completed: std::sync::Mutex::new(HashMap::new()),
            routing_scope_reads: std::sync::atomic::AtomicUsize::new(0),
            remove_completed_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn goal_batch(&self) -> &crate::seeding_gate::SeedGoalBatch {
        &self.goals
    }

    /// `(routing-scope reads, remove-completed reads)` that missed the memo.
    #[cfg(test)]
    pub(crate) fn memo_reads(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering;
        (
            self.routing_scope_reads.load(Ordering::Relaxed),
            self.remove_completed_reads.load(Ordering::Relaxed),
        )
    }
}

/// `cleanup_routing_scope_for_title_id`, answering from the tick cache when the
/// same title has already been resolved in this tick.
async fn cleanup_routing_scope_for_title_id_cached(
    app: &AppUseCase,
    title_id: Option<&str>,
    cache: Option<&TerminalCleanupTickCache>,
) -> CleanupRoutingScope {
    let Some(cache) = cache else {
        return cleanup_routing_scope_for_title_id(app, title_id).await;
    };
    let Some(key) = title_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return (None, None);
    };

    if let Ok(scopes) = cache.routing_scopes.lock()
        && let Some(hit) = scopes.get(&key)
    {
        return hit.clone();
    }
    cache
        .routing_scope_reads
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let resolved = cleanup_routing_scope_for_title_id(app, Some(key.as_str())).await;
    if let Ok(mut scopes) = cache.routing_scopes.lock() {
        scopes.insert(key, resolved.clone());
    }
    resolved
}

/// `AppUseCase::should_remove_completed_download`, memoized per tick on the
/// `(library_id, facet, routing_key)` tuple it actually depends on.
async fn should_remove_completed_download_cached(
    app: &AppUseCase,
    library_id: Option<&str>,
    facet: &MediaFacet,
    routing_key: &str,
    cache: Option<&TerminalCleanupTickCache>,
) -> bool {
    let Some(cache) = cache else {
        return app
            .should_remove_completed_download(library_id, facet, routing_key)
            .await;
    };
    let key = (
        library_id.map(str::to_string),
        facet.clone(),
        routing_key.to_string(),
    );
    if let Ok(policies) = cache.remove_completed.lock()
        && let Some(hit) = policies.get(&key).copied()
    {
        return hit;
    }
    cache
        .remove_completed_reads
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let resolved = app
        .should_remove_completed_download(library_id, facet, routing_key)
        .await;
    if let Ok(mut policies) = cache.remove_completed.lock() {
        policies.insert(key, resolved);
    }
    resolved
}

async fn terminal_failure_origin_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
) -> TerminalFailureOrigin {
    if tracked.burned_by_import_gate {
        return TerminalFailureOrigin::ImportGate;
    }
    if state != TrackedDownloadState::Failed {
        return TerminalFailureOrigin::ClientFailure;
    }
    if tracked.burned_by_import_gate {
        return TerminalFailureOrigin::ImportGate;
    }

    let identity = crate::tracked_downloads::observed_queue_item_identity(&tracked.client_item);
    if crate::download_submission_identity_is_empty(&identity) {
        return TerminalFailureOrigin::ClientFailure;
    }
    let source_identity = ClientJobLocator::new(
        Some(tracked.client_id.as_str()),
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
    );
    let seeding_gate_failure = app
        .services
        .workflow
        .download_submissions
        .get_identity_tracked_state_reason(&identity, Some(&source_identity))
        .await
        .ok()
        .flatten()
        .is_some_and(|reason| {
            matches!(
                reason.as_str(),
                crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON
                    | crate::tracked_downloads::WARNING_TIMEOUT_TRACKED_STATE_REASON
            )
        });
    if seeding_gate_failure {
        TerminalFailureOrigin::ImportGate
    } else {
        TerminalFailureOrigin::ClientFailure
    }
}

pub(crate) async fn reconcile_terminal_download_cleanup_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
    cache: Option<&TerminalCleanupTickCache>,
) -> TerminalDownloadCleanup {
    match app
        .services
        .workflow
        .download_submissions
        .claim_download_cleanup(&tracked.download_id)
        .await
    {
        Ok(crate::DownloadCleanupClaim::Unmanaged) => {
            reconcile_unclaimed_terminal_cleanup(app, tracked, state, cache, None).await
        }
        Ok(crate::DownloadCleanupClaim::Claimed(record)) => {
            run_claimed_download_cleanup(app, *record, cache)
                .await
                .map(|(_, _, cleanup)| cleanup)
                .unwrap_or_else(|| {
                    TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure)
                })
        }
        Ok(crate::DownloadCleanupClaim::Deferred) => {
            TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure)
        }
        Ok(crate::DownloadCleanupClaim::Settled { outcome }) => {
            TerminalDownloadCleanup::bare(match outcome.as_str() {
                "policy_retained" => TerminalDownloadCleanupOutcome::NotConfigured,
                "handed_off" => TerminalDownloadCleanupOutcome::HandedOff,
                "seeding_entry_kept" => TerminalDownloadCleanupOutcome::SeedingEntryKept,
                "payload_and_entry_removed"
                | "entry_removed"
                | "entry_removed_payload_partially_retained"
                | "entry_removed_payload_unverified" => TerminalDownloadCleanupOutcome::Removed,
                "entry_absent"
                | "payload_removed_entry_absent"
                | "native_removal_recovered"
                | "entry_absent_payload_unverified" => TerminalDownloadCleanupOutcome::AlreadyGone,
                // Nothing left for Scryer to do: the client is gone, the row
                // could never be attributed, or the operator must finish by
                // hand. The scope is released either way.
                "client_removed" | "client_unresolved" | "cleanup_abandoned" => {
                    TerminalDownloadCleanupOutcome::NotConfigured
                }
                _ => TerminalDownloadCleanupOutcome::RetryableFailure,
            })
        }
        Err(error) => {
            tracing::warn!(download_id = %tracked.download_id, error = %error, "failed to claim terminal cleanup");
            TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure)
        }
    }
}

async fn reconcile_unclaimed_terminal_cleanup(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
    cache: Option<&TerminalCleanupTickCache>,
    record: Option<&crate::DownloadCleanupRecord>,
) -> TerminalDownloadCleanup {
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id_cached(app, tracked.title_id.as_deref(), cache).await;
    let facet = resolved_facet.or_else(|| facet_from_tracked_label(tracked.facet.as_deref()));
    let precomputed_should_remove = if state == TrackedDownloadState::Failed
        && !tracked.burned_by_import_gate
    {
        let should_remove = crate::import_workflow::should_remove_terminal_download(
            app,
            &tracked.client_id,
            &tracked.client_type,
            library_id.as_deref(),
            facet.as_ref(),
            state,
            cache,
        )
        .await;
        if !should_remove {
            return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured);
        }
        Some(should_remove)
    } else {
        None
    };
    let failure_origin = terminal_failure_origin_for_tracked(app, tracked, state).await;
    reconcile_terminal_download_cleanup(
        app,
        tracked.canonical_download_id(),
        &tracked.client_id,
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
        failure_origin,
        precomputed_should_remove,
        // The tracker already answers "is this still in the client?": a row
        // absent from the client's snapshot past the grace window is marked
        // untrackable. Reusing that avoids a per-item listing call every tick.
        tracked.is_trackable,
        // The live tracked row was refreshed from the client earlier in this
        // same tick, so its observation is fresher than the published snapshot
        // (which is only republished *after* reconcile runs). Passing it in is
        // what makes each cycle re-evaluate against current ratio/seed time
        // rather than the answer that first parked the row.
        Some(
            crate::seeding_gate::observation_from_queue_item(&tracked.client_item)
                .unwrap_or_default(),
        ),
        cache,
        record,
    )
    .await
}

pub(crate) async fn run_claimed_download_cleanup(
    app: &AppUseCase,
    record: crate::DownloadCleanupRecord,
    cache: Option<&TerminalCleanupTickCache>,
) -> Option<(
    scryer_domain::download_identity::DownloadId,
    Option<crate::tracked_downloads::TrackedDownload>,
    TerminalDownloadCleanup,
)> {
    let locator = crate::ClientJobLocator::new(
        Some(&record.client_id),
        &record.client_type,
        &record.item_id,
    );
    let result = async {
        let persisted = app.services.workflow.download_submissions
            .get_identity_tracked_state_for_download(
                Some(&record.download_id), &crate::DownloadSubmissionIdentity::default(), Some(&locator),
            ).await?;
        if persisted.as_deref().is_some_and(|state| {
            !matches!(state, "imported" | "imported_seeding" | "failed" | "ignored")
        }) {
            // A genuine deferral, not a failure: the job re-entered the import
            // pipeline and the state transition will re-enqueue cleanup once
            // it settles again.
            finish_cleanup_attempt(app, &record, "cleanup_deferred", false, record.history_offset,
                Some("import is no longer settled; cleanup deferred")).await;
            return Ok(None);
        }
        // A cleanup whose client no longer exists (or, for a legacy row, could
        // never be attributed to one client) has nothing left to act on.
        let client_config = if record.client_id.trim().is_empty() {
            None
        } else {
            app.services.integrations.download_client_configs.get_by_id(&record.client_id).await?
        };
        if client_config.is_none() {
            let outcome = if record.client_id.trim().is_empty() { "client_unresolved" } else { "client_removed" };
            if !finish_cleanup_attempt(app, &record, outcome, true, 0,
                Some("download client is no longer configured; nothing to clean up")).await {
                return Ok(None);
            }
            tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                client_type = %record.client_type, item_id = %record.item_id, outcome,
                "terminal download cleanup settled without a client; any client entry or payload is left as-is");
            return Ok(Some((record.download_id, None,
                TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured))));
        }
        let Some(state) = TrackedDownloadState::from_str_opt(&record.tracked_state) else {
            return Err(AppError::Validation("invalid durable cleanup state".into()));
        };
        let title = match record.title_id.as_deref() {
            Some(id) => app.services.catalog.titles.get_by_id(id).await?,
            None => None,
        };
        let library_id = title.as_ref().map(|title| title.library_id.as_str());
        let facet = title.as_ref().map(|title| title.facet.clone())
            .or_else(|| facet_from_tracked_label(record.facet.as_deref()));
        let remove = if state == TrackedDownloadState::Ignored {
            true
        } else {
            let facet = facet.as_ref().ok_or_else(|| AppError::Validation("cleanup has no valid routing attribution".into()))?;
            let policy = app.read_download_client_routing_entry(
                library_id, facet, &record.client_id,
            ).await?.unwrap_or_else(crate::catalog_helpers::default_download_client_routing_entry);
            if state.counts_as_imported() { policy.remove_completed } else { policy.remove_failed }
        };
        if !remove {
            if !finish_cleanup_attempt(app, &record, "policy_retained", true, 0, None).await {
                return Ok(None);
            }
            tracing::info!(download_id = %record.download_id, "terminal download retained by removal policy");
            return Ok(Some((record.download_id, None,
                TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured))));
        }
        let item = match app.services.integrations.download_client
            .observe_download(&locator, record.history_offset).await? {
            crate::DownloadClientObservation::Present(item) => *item,
            crate::DownloadClientObservation::Absent => {
                let checkpoint = record.payload_checkpoint.as_deref()
                    .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok());
                let payload_removed = checkpoint.as_ref()
                    .is_some_and(|checkpoint| checkpoint.get("payload_removed").and_then(|v| v.as_bool()) == Some(true));
                let checkpoint_matches = checkpoint.as_ref().is_some_and(|checkpoint| {
                    checkpoint.get("download_id").and_then(|v| v.as_str()) == Some(record.download_id.to_string().as_str())
                        && checkpoint.get("client_id").and_then(|v| v.as_str()) == Some(record.client_id.as_str())
                        && checkpoint.get("client_type").and_then(|v| v.as_str()) == Some(record.client_type.as_str())
                        && checkpoint.get("item_id").and_then(|v| v.as_str()) == Some(record.item_id.as_str())
                });
                let native_requested = checkpoint_matches && checkpoint.as_ref().is_some_and(|checkpoint|
                    checkpoint.get("native_remove_requested").and_then(|v| v.as_bool()) == Some(true));
                let entry_only_requested = checkpoint_matches
                    && matches!(state, TrackedDownloadState::Failed | TrackedDownloadState::Ignored)
                    && checkpoint.as_ref().is_some_and(|checkpoint|
                        checkpoint.get("entry_only_remove_requested").and_then(|v| v.as_bool()) == Some(true));
                // The entry vanished while a host payload deletion was in
                // flight (the plan and inventory are checkpointed): finish the
                // verified deletion from that checkpoint, then settle.
                let host_payload_in_flight = !payload_removed
                    && checkpoint.as_ref().is_some_and(|checkpoint| checkpoint.get("completed").is_some())
                    && record.client_type.trim() != "weaver"
                    && !plugin_has_native_data_removal(app, &record.client_id).await;
                let payload_removed = if host_payload_in_flight {
                    match remove_host_payload_before_entry_cleanup(
                        app, Some(&record.download_id), &record.client_id, &record.client_type,
                        &record.item_id, Some(&record),
                    ).await {
                        Ok(report) => {
                            if let Some(marker) = report.filesystem_checkpoint.as_deref()
                                && let Err(error) = clear_host_payload_cleanup_checkpoint(marker).await
                            {
                                tracing::warn!(download_id = %record.download_id, error = %error,
                                    "failed to clear payload cleanup checkpoint after entry absence");
                            }
                            true
                        }
                        Err(error) => {
                            tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                                attempts = record.attempts, error = %error,
                                "entry absent; host payload cleanup from checkpoint failed");
                            false
                        }
                    }
                } else {
                    payload_removed
                };
                if !(payload_removed || native_requested || entry_only_requested || state == TrackedDownloadState::Ignored)
                    && record.attempts >= HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL
                {
                    // The entry is gone and Scryer cannot prove what happened to
                    // the payload. Retrying cannot change that; settle so the
                    // scope is released and say so once.
                    if !finish_cleanup_attempt(app, &record, "entry_absent_payload_unverified", true, 0,
                        Some("client entry vanished before Scryer verified payload cleanup; payload may remain on disk")).await {
                        return Ok(None);
                    }
                    tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                        item_id = %record.item_id,
                        "client entry vanished before Scryer verified payload cleanup; payload may remain on disk");
                    return Ok(Some((record.download_id, None,
                        TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::AlreadyGone))));
                }
                if payload_removed || native_requested || entry_only_requested || state == TrackedDownloadState::Ignored {
                    if state == TrackedDownloadState::ImportedSeeding {
                        app.services.workflow.download_submissions.record_identity_tracked_state_for_download(
                            Some(&record.download_id), &crate::DownloadSubmissionIdentity::default(),
                            Some(&locator), "imported", Some("seeding_complete"), None,
                        ).await?;
                    }
                    let outcome = if payload_removed { "payload_removed_entry_absent" }
                        else if native_requested { "native_removal_recovered" } else { "entry_absent" };
                    if !finish_cleanup_attempt(app, &record, outcome, true, 0, None).await {
                        return Ok(None);
                    }
                    tracing::info!(download_id = %record.download_id, client_id = %record.client_id,
                        outcome, "cleanup recovered after authoritative entry absence");
                    return Ok(Some((record.download_id, None,
                        TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::AlreadyGone))));
                }
                return Err(AppError::Validation(
                    "entry absent; payload deletion has not been verified".into()));
            }
            crate::DownloadClientObservation::Unknown { reason, next_history_offset } => {
                finish_cleanup_attempt(app, &record, "observation_unknown", false,
                    next_history_offset, Some(&reason)).await;
                return Ok(None);
            }
        };
        let active = app.services.workflow.download_registry
            .find_active_binding_by_locator(&locator).await?;
        let token = item.download_id.as_deref()
            .and_then(scryer_domain::download_identity::DownloadId::from_wire);
        // Hash-addressed torrent clients report the torrent's internal name,
        // which need not equal the indexer's release title. Compare content
        // identity with the original submission, while still requiring its
        // active binding when no canonical token is available.
        let torrent_hash_matches = if crate::seeding_gate::client_type_is_torrent(app, &record.client_type)
            && let Some(observed) = crate::normalize_torrent_info_hash(Some(&item.download_client_item_id))
        {
            app.services.workflow.download_submissions
                .find_by_canonical_download_id(&record.download_id).await?
                .and_then(|submission| crate::normalize_torrent_info_hash(submission.info_hash.as_deref()))
                .map(|expected| expected == observed)
        } else { None };
        if crate::ClientJobLocator::new(Some(&item.client_id), &item.client_type,
            &item.download_client_item_id) != locator
            || token.is_some_and(|id| id != record.download_id)
            || active.as_ref().is_some_and(|binding| binding.download_id != record.download_id)
            || torrent_hash_matches == Some(false)
            || (token != Some(record.download_id)
                && !(active.as_ref().is_some_and(|binding| binding.download_id == record.download_id)
                    && (torrent_hash_matches == Some(true)
                        || record.source_title.as_deref().is_some_and(|name| name == item.title_name))))
        {
            return Err(AppError::Validation("cleanup locator is reused or identity is ambiguous".into()));
        }
        let mut tracked = crate::tracked_downloads::TrackedDownloadService::build_new_tracked_download(
            app, record.download_id,
            crate::tracked_downloads::tracked_download_id(
                Some(&record.client_id), &record.client_type, &record.item_id,
            ), item,
        ).await;
        tracked.title_id = record.title_id.clone();
        tracked.facet = record.facet.clone();
        tracked.source_title = record.source_title.clone();
        tracked.state = state;
        let cleanup = reconcile_unclaimed_terminal_cleanup(app, &tracked, state, cache, Some(&record)).await;
        let complete = terminal_download_cleanup_is_complete(cleanup.outcome);
        let outcome = match cleanup.outcome {
            TerminalDownloadCleanupOutcome::Removed if state == TrackedDownloadState::Ignored => "entry_removed",
            TerminalDownloadCleanupOutcome::Removed => match cleanup.payload {
                Some(HostPayloadDisposition::PartiallyRetained) => "entry_removed_payload_partially_retained",
                Some(HostPayloadDisposition::Unverified) => "entry_removed_payload_unverified",
                Some(HostPayloadDisposition::Removed) | None => "payload_and_entry_removed",
            },
            TerminalDownloadCleanupOutcome::AlreadyGone => "entry_absent",
            TerminalDownloadCleanupOutcome::NotConfigured => "policy_retained",
            TerminalDownloadCleanupOutcome::HeldForSeeding => "seeding_hold",
            TerminalDownloadCleanupOutcome::SeedingEntryKept => "seeding_entry_kept",
            TerminalDownloadCleanupOutcome::HandedOff => "handed_off",
            TerminalDownloadCleanupOutcome::RetryableFailure => "cleanup_failed",
        };
        let error = (cleanup.outcome == TerminalDownloadCleanupOutcome::RetryableFailure)
            .then_some("required payload or entry operation failed; see client cleanup log");
        if !finish_cleanup_attempt(app, &record, outcome, complete, 0, error).await {
            return Ok(None);
        }
        if complete {
            if matches!(cleanup.payload, Some(HostPayloadDisposition::PartiallyRetained | HostPayloadDisposition::Unverified)) {
                tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                    item_id = %record.item_id, outcome,
                    "terminal download cleanup completed with payload left on disk for the operator");
            } else {
                tracing::info!(download_id = %record.download_id, client_id = %record.client_id,
                    item_id = %record.item_id, outcome, "terminal download cleanup completed");
            }
        }
        Ok(Some((record.download_id, Some(tracked), cleanup)))
    }.await;
    match result {
        Ok(result) => result,
        Err(error) => {
            // Validation errors describe a row that cannot be reconciled as
            // recorded (ambiguous identity, missing attribution). They do not
            // heal with time, so after the retry budget the row settles as
            // abandoned and the scope is released; the client entry stays.
            let abandon = matches!(error, AppError::Validation(_))
                && record.attempts >= HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL;
            if abandon {
                tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                    item_id = %record.item_id, attempts = record.attempts, error = %error,
                    "terminal cleanup abandoned; the client entry is left for the operator");
                finish_cleanup_attempt(app, &record, "cleanup_abandoned", true, 0,
                    Some(&error.to_string())).await;
                return Some((record.download_id, None,
                    TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured)));
            }
            // Keep the log useful: the first few attempts and then one line
            // per hour of backoff, not one every five minutes forever.
            if record.attempts <= HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL
                || record.attempts.is_multiple_of(12) {
                tracing::warn!(download_id = %record.download_id, client_id = %record.client_id,
                    attempts = record.attempts, error = %error, "terminal cleanup remains pending");
            } else {
                tracing::debug!(download_id = %record.download_id, client_id = %record.client_id,
                    attempts = record.attempts, error = %error, "terminal cleanup remains pending");
            }
            finish_cleanup_attempt(
                app,
                &record,
                "cleanup_failed",
                false,
                record.history_offset,
                Some(&error.to_string()),
            )
            .await;
            None
        }
    }
}

async fn finish_cleanup_attempt(
    app: &AppUseCase,
    record: &crate::DownloadCleanupRecord,
    outcome: &str,
    complete: bool,
    offset: usize,
    error: Option<&str>,
) -> bool {
    let delay = (30_i64 * (1_i64 << record.attempts.saturating_sub(1).min(4))).min(300);
    match app
        .services
        .workflow
        .download_submissions
        .finish_download_cleanup(&record.download_id, outcome, complete, delay, offset, error)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(download_id = %record.download_id, error = %error, "failed to persist cleanup outcome");
            false
        }
    }
}
/// Scryer's own transient markers on a *non-`Failed`* import result.
///
/// Execution-phase failures never come through here: they arrive as
/// `ImportDecision::Failed` and are retried by the phase rule in
/// `completed_import_result_is_retryable` regardless of the message (Sonarr's
/// model — no error-string catalogue). This list only recognises the transient
/// conditions Scryer itself reports as a `Skipped`/`Rejected` result after an
/// execution race. Import-check outcomes such as a source still unpacking are
/// represented by `ImportSkipReason`, never their message text.
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    const SCRYER_TRANSIENT_PHRASES: &[&str] =
        &["source changed", "temporarily", "not found or inaccessible"];
    SCRYER_TRANSIENT_PHRASES
        .iter()
        .any(|needle| normalized.contains(needle))
        || contains_word(&normalized, "locked")
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        let before_is_word = haystack[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after_is_word = haystack[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        !before_is_word && !after_is_word
    })
}

async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::AppResult<crate::QualityProfile> {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    // Resolution failures propagate: gating an import against a substitute
    // profile silently applies the wrong quality rules, which is the failure
    // mode the strict resolver exists to prevent. A validation failure (e.g. a
    // dangling profile reference) needs operator action and surfaces as a
    // blocked import; any other failure is treated as transient and worded so
    // `completed_import_error_message_is_retryable` re-attempts it.
    app.resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
        title_tags: &title.tags,
        library_id: Some(title.library_id.as_str()),
        imdb_id: title.imdb_id.as_deref(),
        tvdb_id,
        category_hint: Some(category_hint),
    })
    .await
    .map_err(|error| match error {
        crate::AppError::Validation(_) => error,
        other => crate::AppError::Repository(format!(
            "quality profile resolution temporarily unavailable: {other}"
        )),
    })
}
/// "Not media at all" — a sample, a promo, a zero-length placeholder. Owned by
/// the import pipeline alone: scoring no longer has a floor to keep in step with
/// it, because the smallness it used to veto below is now a penalty read off the
/// size curve like any other band.
const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024;

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}
