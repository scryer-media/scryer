use super::*;
use crate::library_discovery::count_series_loose_root_files;
use crate::library_scan_helpers::require_directory_library_path;
use crate::library_scan_metadata::{
    BatchMetadataSearchKey, MovieCandidateEvidence, execute_batch_metadata_searches,
    prepare_movie_candidate_evidence, prepare_series_library_scan_candidate,
    split_ready_metadata_candidates,
};
use crate::library_scan_unmatched::{
    IgnoredLibraryScanItemArgs, LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
    persist_ignored_library_scan_item,
};
use crate::stored_paths::path_to_stored_string;
use std::collections::VecDeque;

use super::scan_title_scan::{LibraryScanMediaWorkReservation, title_requires_scan_hydration};

/// Concurrent sidecar/evidence reads per scan root. Evidence is cheap (one
/// readdir plus at most two sidecar reads), so this can run much wider than
/// the recursive inventory walks without hammering SMB mounts.
const LIBRARY_SCAN_EVIDENCE_CONCURRENCY: usize = 32;
/// SMG match batches allowed in flight at once for a scan root.
const LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES: usize = 2;
/// Timer flush for the match batcher: a partial batch is dispatched this long
/// after its first candidate arrived.
const LIBRARY_SCAN_MATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
/// Rendezvous storage high-water mark. Once this many file paths are parked
/// waiting for match decisions, the inventory phase pauses until storage
/// drains. Evidence emission is never paused.
const LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER: usize = 100_000;
/// Discovery-to-match channel capacity (at least two SMG batches).
const LIBRARY_SCAN_MATCH_INPUT_QUEUE_CAPACITY: usize = 2 * LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
/// Cap on candidates parked in the match worker waiting for SMG results
/// before the worker stops pulling intake and lets channel backpressure hold.
const LIBRARY_SCAN_MATCH_PENDING_HIGH_WATER: usize = 4 * LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
/// Resolved candidates can perform storage/progress work. Keep this burst
/// small so finalization cannot starve SMG dispatch and completion handling.
const LIBRARY_SCAN_MATCH_RESOLUTION_BURST_SIZE: usize = 4;
/// Hydration runs downstream of matching in bulk batches so a fresh episodic
/// library does not degrade into one SMG metadata call per title.
const LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES: usize = 2;
const LIBRARY_SCAN_DIAGNOSTIC_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL: usize = 20;

pub(super) type ScanCandidateKey = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LibraryScanPipelineKind {
    Movie,
    Series,
}

pub(super) enum ScanPipelineCandidate {
    Movie(Box<PreparedMovieLibraryScanCandidate>),
    Series(Box<PreparedSeriesLibraryScanCandidate>),
}

impl ScanPipelineCandidate {
    fn batch_search_keys(&self) -> AppResult<Vec<BatchMetadataSearchKey>> {
        match self {
            Self::Movie(candidate) => movie_candidate_batch_search_keys(candidate),
            Self::Series(candidate) => series_candidate_batch_search_keys(candidate),
        }
    }

    fn diagnostic_name(&self) -> String {
        match self {
            Self::Movie(candidate) => {
                if !candidate.query.trim().is_empty() {
                    candidate.query.trim().to_string()
                } else {
                    Path::new(&candidate.file.path)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or(candidate.file.path.as_str())
                        .to_string()
                }
            }
            Self::Series(candidate) => {
                if !candidate.query.trim().is_empty() {
                    candidate.query.trim().to_string()
                } else if let Some(folder_name) = candidate.folder_name.as_deref() {
                    folder_name.to_string()
                } else {
                    candidate
                        .folder_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| candidate.folder_path.to_string_lossy().into_owned())
                }
            }
        }
    }
}

/// Events emitted by the root enumerator and candidate jobs.
enum ScanCandidateJobEvent {
    Candidate {
        key: ScanCandidateKey,
        candidate: ScanPipelineCandidate,
        inline_inventory: Option<Vec<LibraryFile>>,
        inventory_cancel: CancellationToken,
    },
    EvidenceFailed {
        item_path: String,
        error: AppError,
    },
    EvidenceDone {
        metrics: CandidateJobMetrics,
    },
    DiscoveryFailed {
        error: AppError,
    },
}

/// Events emitted by recursive inventory/count walks.
enum ScanInventoryJobEvent {
    Inventory {
        key: ScanCandidateKey,
        files: Vec<LibraryFile>,
    },
    InventoryFailed {
        key: ScanCandidateKey,
        item_path: String,
        error: AppError,
    },
    InventoryCanceled {
        key: ScanCandidateKey,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct CandidateJobMetrics {
    candidates_emitted: usize,
    skipped: usize,
    failed: usize,
    inline_inventory_emitted: usize,
    inventory_walks_queued: usize,
    inventory_walks_started: usize,
}

/// Events emitted by the SMG match worker.
enum ScanMatchWorkerEvent {
    Matched {
        key: ScanCandidateKey,
        work: Box<LibraryScanTitleWork>,
    },
    Terminal {
        key: ScanCandidateKey,
    },
    Done(Box<ScanMatchWorkerReport>),
}

struct ScanMatchWorkerReport {
    summary: LibraryScanSummary,
    unmatched_items: Vec<LibraryScanUnmatchedItem>,
    seen_paths: HashSet<String>,
    stats: MetadataLookupBatchStats,
}

/// Staging queue handed to the shared candidate-processing functions. The
/// match worker inspects the staged work after each call to decide whether
/// the candidate matched (work present) or reached a non-media terminal.
struct PipelineTitleWorkSink {
    staged: Option<LibraryScanTitleWork>,
}

impl LibraryScanTitleWorkQueue for PipelineTitleWorkSink {
    fn enqueue(&mut self, work: LibraryScanTitleWork) -> bool {
        self.staged = Some(work);
        true
    }
}

enum CandidateMatchState {
    Pending,
    MatchedAwaitingInventory(Box<LibraryScanTitleWork>),
    Dispatched,
    Terminal,
}

enum CandidateInventoryState {
    Pending,
    Ready(Vec<LibraryFile>),
    Consumed,
    Failed,
    Canceled,
}

fn candidate_match_state_name(state: &CandidateMatchState) -> &'static str {
    match state {
        CandidateMatchState::Pending => "pending",
        CandidateMatchState::MatchedAwaitingInventory(_) => "matched_awaiting_inventory",
        CandidateMatchState::Dispatched => "dispatched",
        CandidateMatchState::Terminal => "terminal",
    }
}

fn candidate_inventory_state_name(state: &CandidateInventoryState) -> &'static str {
    match state {
        CandidateInventoryState::Pending => "pending",
        CandidateInventoryState::Ready(_) => "ready",
        CandidateInventoryState::Consumed => "consumed",
        CandidateInventoryState::Failed => "failed",
        CandidateInventoryState::Canceled => "canceled",
    }
}

struct CandidateRuntime {
    item_path: String,
    match_state: CandidateMatchState,
    inventory: CandidateInventoryState,
    inventory_cancel: CancellationToken,
}

impl CandidateRuntime {
    fn inventory_terminal(&self) -> bool {
        !matches!(self.inventory, CandidateInventoryState::Pending)
    }
}

pub(super) struct LibraryScanPipelineRequest<'a> {
    pub(super) app: &'a AppUseCase,
    pub(super) actor: &'a User,
    pub(super) facet: &'a MediaFacet,
    pub(super) library_id: &'a str,
    pub(super) library_path: &'a str,
    pub(super) session_id: &'a str,
    pub(super) mark_discovery_complete_on_drain: bool,
    pub(super) cancel_token: Option<CancellationToken>,
    pub(super) scan_hints: Option<LibraryScanHintSet>,
    pub(super) kind: LibraryScanPipelineKind,
}

pub(super) async fn run_library_scan_pipeline(
    request: LibraryScanPipelineRequest<'_>,
) -> AppResult<LibraryScanSummary> {
    let LibraryScanPipelineRequest {
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        mark_discovery_complete_on_drain,
        cancel_token,
        scan_hints,
        kind,
    } = request;

    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    require_directory_library_path(library_path)?;

    let (candidate_events_tx, mut candidate_events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (inventory_events_tx, mut inventory_events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (match_input_tx, match_input_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_MATCH_INPUT_QUEUE_CAPACITY);
    let (match_events_tx, mut match_events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (storage_watch_tx, storage_watch_rx) = tokio::sync::watch::channel(0usize);

    let jobs_handle = spawn_candidate_jobs(CandidateJobContext {
        app: app.clone(),
        session_id: session_id.to_string(),
        library_path: library_path.to_string(),
        kind,
        scan_hints,
        mark_discovery_complete_on_drain,
        cancel_token: cancel_token.clone(),
        candidate_events: candidate_events_tx,
        inventory_events: inventory_events_tx,
        storage_watch: storage_watch_rx,
    })?;

    let worker_handle = tokio::spawn(run_scan_match_worker(
        ScanMatchWorkerContext {
            app: app.clone(),
            actor: actor.clone(),
            facet: facet.clone(),
            library_id: library_id.to_string(),
            library_path: library_path.to_string(),
            session_id: session_id.to_string(),
            metadata_language: app.metadata_language().await,
            kind,
        },
        match_input_rx,
        match_events_tx,
        cancel_token.clone(),
    ));

    let pool_policy = LibraryScanMediaAnalysisPolicy::full_scan_pipeline(
        app,
        session_id,
        facet,
        cancel_token.clone(),
    )
    .await;
    let mut pool = LibraryScanMediaAnalysisPool::for_policy(app, actor, pool_policy).await?;
    let analysis_profile = pool.analysis_profile();
    debug!(
        path = %library_path,
        facet = facet.as_str(),
        title_group_concurrency = analysis_profile.title_group_concurrency,
        file_analysis_concurrency_per_title = analysis_profile.file_analysis_concurrency_per_title,
        global_analysis_concurrency = crate::GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
        "library scan media analysis profile selected"
    );

    let mut summary = LibraryScanSummary::default();
    let mut candidates: HashMap<ScanCandidateKey, CandidateRuntime> = HashMap::new();
    let mut early_inventory: HashMap<ScanCandidateKey, CandidateInventoryState> = HashMap::new();
    let mut forward_queue: VecDeque<(ScanCandidateKey, ScanPipelineCandidate)> = VecDeque::new();
    let mut match_input_tx = Some(match_input_tx);
    let mut evidence_done = false;
    let mut inventory_done = false;
    let mut match_done = false;
    let mut worker_report: Option<Box<ScanMatchWorkerReport>> = None;
    let mut stored_inventory_paths = 0usize;
    let mut file_total_marked = false;
    let mut media_file_total_counted = 0usize;
    let mut discovery_error: Option<AppError> = None;
    // Duplicate candidates resolving to already-covered title work are
    // deduplicated by the analysis pool; they correct matched totals but are
    // not user-visible skipped imports.
    let mut media_dedup_skips = 0usize;
    let mut candidate_events_seen = 0usize;
    let mut inventory_events_seen = 0usize;
    let mut match_events_seen = 0usize;
    let mut match_events_matched = 0usize;
    let mut match_events_terminal = 0usize;
    let mut hydration_batches_completed = 0usize;
    let mut last_diagnostic_heartbeat = Instant::now();

    let mut hydration = ScanHydrationBatcher::new(app.clone(), cancel_token.clone());

    loop {
        if evidence_done && match_done && inventory_done && forward_queue.is_empty() {
            break;
        }
        if discovery_error.is_some() {
            break;
        }

        // Close the match worker input once all evidence has been forwarded;
        // recursive inventory/count walks must not hold title matching open.
        if evidence_done && forward_queue.is_empty() {
            match_input_tx = None;
        }

        let hydration_deadline = hydration.deadline_instant();
        let diagnostic_deadline = tokio::time::Instant::from_std(
            last_diagnostic_heartbeat + LIBRARY_SCAN_DIAGNOSTIC_HEARTBEAT_INTERVAL,
        );
        tokio::select! {
            event = candidate_events_rx.recv(), if !evidence_done => {
                match event {
                    Some(ScanCandidateJobEvent::EvidenceDone { metrics }) => {
                        candidate_events_seen = candidate_events_seen.saturating_add(1);
                        evidence_done = true;
                        debug!(
                            path = %library_path,
                            facet = facet.as_str(),
                            candidates = metrics.candidates_emitted,
                            skipped = metrics.skipped,
                            failed = metrics.failed,
                            inline_inventory = metrics.inline_inventory_emitted,
                            inventory_walks_queued = metrics.inventory_walks_queued,
                            inventory_walks_started = metrics.inventory_walks_started,
                            elapsed_ms = elapsed_ms_u64(started_at),
                            "library scan evidence phase completed"
                        );
                    }
                    Some(event) => {
                        candidate_events_seen = candidate_events_seen.saturating_add(1);
                        if let Err(error) = handle_candidate_job_event(CandidateEventContext {
                            app,
                            facet,
                            library_id,
                            library_path,
                            session_id,
                            coordinator: &coordinator,
                            summary: &mut summary,
                            candidates: &mut candidates,
                            early_inventory: &mut early_inventory,
                            forward_queue: &mut forward_queue,
                            discovery_error: &mut discovery_error,
                        }, event).await {
                            discovery_error = Some(error);
                        }
                    }
                    None => {
                        evidence_done = true;
                    }
                }
            }
            event = inventory_events_rx.recv(), if !inventory_done => {
                match event {
                    Some(event) => {
                        inventory_events_seen = inventory_events_seen.saturating_add(1);
                        handle_inventory_job_event(InventoryEventContext {
                            coordinator: &coordinator,
                            candidates: &mut candidates,
                            early_inventory: &mut early_inventory,
                            hydration: &mut hydration,
                            pool: &mut pool,
                            media_dedup_skips: &mut media_dedup_skips,
                            media_file_total_counted: &mut media_file_total_counted,
                            stored_inventory_paths: &mut stored_inventory_paths,
                            storage_watch: &storage_watch_tx,
                        }, event).await?;
                    }
                    None => {
                        inventory_done = true;
                    }
                }
            }
            permit = async {
                match match_input_tx.as_ref() {
                    Some(tx) => tx.reserve().await.ok(),
                    None => None,
                }
            }, if !forward_queue.is_empty() && match_input_tx.is_some() => {
                match permit {
                    Some(permit) => {
                        if let Some(entry) = forward_queue.pop_front() {
                            permit.send(entry);
                        }
                    }
                    None => {
                        // The worker is gone; drain the queue so the scan can
                        // settle instead of spinning.
                        forward_queue.clear();
                    }
                }
            }
            event = match_events_rx.recv(), if !match_done => {
                match event {
                    Some(ScanMatchWorkerEvent::Matched { key, work }) => {
                        match_events_seen = match_events_seen.saturating_add(1);
                        match_events_matched = match_events_matched.saturating_add(1);
                        handle_match_decision(
                            &coordinator,
                            &mut candidates,
                            &mut hydration,
                            &mut pool,
                            &mut media_dedup_skips,
                            &mut media_file_total_counted,
                            &mut stored_inventory_paths,
                            &storage_watch_tx,
                            key,
                            Some(*work),
                        ).await?;
                    }
                    Some(ScanMatchWorkerEvent::Terminal { key }) => {
                        match_events_seen = match_events_seen.saturating_add(1);
                        match_events_terminal = match_events_terminal.saturating_add(1);
                        handle_match_decision(
                            &coordinator,
                            &mut candidates,
                            &mut hydration,
                            &mut pool,
                            &mut media_dedup_skips,
                            &mut media_file_total_counted,
                            &mut stored_inventory_paths,
                            &storage_watch_tx,
                            key,
                            None,
                        ).await?;
                    }
                    Some(ScanMatchWorkerEvent::Done(report)) => {
                        match_events_seen = match_events_seen.saturating_add(1);
                        debug!(
                            path = %library_path,
                            facet = facet.as_str(),
                            scanned = report.summary.scanned,
                            matched = report.summary.matched,
                            unmatched = report.summary.unmatched,
                            skipped = report.summary.skipped,
                            metadata_lookups = report.stats.logical_lookups,
                            metadata_lookup_requests_executed = report.stats.executed_requests,
                            elapsed_ms = elapsed_ms_u64(started_at),
                            "library scan match phase completed"
                        );
                        worker_report = Some(report);
                        match_done = true;
                    }
                    None => {
                        match_done = true;
                    }
                }
            }
            hydrated = hydration.join_next(), if hydration.has_in_flight() => {
                hydration_batches_completed = hydration_batches_completed.saturating_add(1);
                let batch = hydrated?;
                debug!(
                    path = %library_path,
                    facet = facet.as_str(),
                    hydrated = batch.ready.len(),
                    failed = batch.failed.len(),
                    hydration_pending = hydration.pending.len(),
                    hydration_in_flight = hydration.in_flight.len(),
                    hydration_batches_completed,
                    "library scan hydration chunk settled"
                );
                commit_hydration_batch(&mut pool, batch).await?;
            }
            _ = async {
                match hydration_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            }, if hydration_deadline.is_some() => {
                hydration.flush_due();
            }
            _ = tokio::time::sleep_until(diagnostic_deadline) => {}
            _ = async {
                match cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            }, if cancel_token.is_some() => {
                break;
            }
        }

        hydration.maybe_flush();
        pool.pump().await?;

        if last_diagnostic_heartbeat.elapsed() >= LIBRARY_SCAN_DIAGNOSTIC_HEARTBEAT_INTERVAL {
            let (
                candidate_pending,
                candidate_awaiting_inventory,
                candidate_dispatched,
                candidate_terminal,
            ) = candidate_runtime_state_counts(&candidates);
            let pool_diagnostics = pool.diagnostics();
            debug!(
                path = %library_path,
                facet = facet.as_str(),
                elapsed_ms = elapsed_ms_u64(started_at),
                evidence_done,
                inventory_done,
                match_done,
                forward_queue = forward_queue.len(),
                candidates = candidates.len(),
                candidate_pending,
                candidate_awaiting_inventory,
                candidate_dispatched,
                candidate_terminal,
                candidate_events = candidate_events_seen,
                inventory_events = inventory_events_seen,
                match_events = match_events_seen,
                match_events_matched,
                match_events_terminal,
                stored_inventory_paths,
                hydration_pending = hydration.pending.len(),
                hydration_in_flight = hydration.in_flight.len(),
                hydration_flush_requested = hydration.flush_requested,
                hydration_batches_completed,
                media_file_total_counted,
                media_reserved = pool_diagnostics.reserved,
                media_pending_full = pool_diagnostics.pending_full,
                media_pending_scoped = pool_diagnostics.pending_scoped,
                media_analysis_ready = pool_diagnostics.analysis_ready,
                media_in_flight = pool_diagnostics.in_flight,
                media_completed = pool_diagnostics.completed,
                media_walk_tasks = pool_diagnostics.walk_tasks,
                media_input_closed = pool_diagnostics.input_closed,
                media_file_total_known_marked = pool_diagnostics.file_total_known_marked,
                media_dedup_skips,
                "library scan pipeline diagnostic heartbeat"
            );
            last_diagnostic_heartbeat = Instant::now();
        }

        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &hydration,
            file_total_marked: &mut file_total_marked,
            media_file_total_counted,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;
    }

    drop(match_input_tx);

    if let Some(error) = discovery_error {
        // Root discovery failed: settle workers and fail the scan without a
        // success progress latch.
        candidate_events_rx.close();
        inventory_events_rx.close();
        match_events_rx.close();
        jobs_handle.abort();
        worker_handle.abort();
        let _ = jobs_handle.await;
        let _ = worker_handle.await;
        hydration.abort_and_drain().await;
        pool.drain_for_failure().await?;
        return Err(error);
    }

    // Drain any candidate/inventory/match events that raced with loop exit.
    while let Some(event) = candidate_events_rx.recv().await {
        match event {
            ScanCandidateJobEvent::EvidenceDone { .. } => {}
            event => {
                if let Err(error) = handle_candidate_job_event(
                    CandidateEventContext {
                        app,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                        coordinator: &coordinator,
                        summary: &mut summary,
                        candidates: &mut candidates,
                        early_inventory: &mut early_inventory,
                        forward_queue: &mut forward_queue,
                        discovery_error: &mut discovery_error,
                    },
                    event,
                )
                .await
                {
                    discovery_error = Some(error);
                }
            }
        }
    }
    while let Some(event) = inventory_events_rx.recv().await {
        handle_inventory_job_event(
            InventoryEventContext {
                coordinator: &coordinator,
                candidates: &mut candidates,
                early_inventory: &mut early_inventory,
                hydration: &mut hydration,
                pool: &mut pool,
                media_dedup_skips: &mut media_dedup_skips,
                media_file_total_counted: &mut media_file_total_counted,
                stored_inventory_paths: &mut stored_inventory_paths,
                storage_watch: &storage_watch_tx,
            },
            event,
        )
        .await?;
    }

    if let Some(error) = discovery_error {
        match_events_rx.close();
        jobs_handle.abort();
        worker_handle.abort();
        let _ = jobs_handle.await;
        let _ = worker_handle.await;
        hydration.abort_and_drain().await;
        pool.drain_for_failure().await?;
        return Err(error);
    }

    // The worker's final report may not have been consumed if the loop broke
    // on cancellation; drain match events so the summary is not lost.
    while !match_done {
        match match_events_rx.recv().await {
            Some(ScanMatchWorkerEvent::Matched { key, work }) => {
                handle_match_decision(
                    &coordinator,
                    &mut candidates,
                    &mut hydration,
                    &mut pool,
                    &mut media_dedup_skips,
                    &mut media_file_total_counted,
                    &mut stored_inventory_paths,
                    &storage_watch_tx,
                    key,
                    Some(*work),
                )
                .await?;
            }
            Some(ScanMatchWorkerEvent::Terminal { key }) => {
                handle_match_decision(
                    &coordinator,
                    &mut candidates,
                    &mut hydration,
                    &mut pool,
                    &mut media_dedup_skips,
                    &mut media_file_total_counted,
                    &mut stored_inventory_paths,
                    &storage_watch_tx,
                    key,
                    None,
                )
                .await?;
            }
            Some(ScanMatchWorkerEvent::Done(report)) => {
                debug!(
                    path = %library_path,
                    facet = facet.as_str(),
                    scanned = report.summary.scanned,
                    matched = report.summary.matched,
                    unmatched = report.summary.unmatched,
                    skipped = report.summary.skipped,
                    metadata_lookups = report.stats.logical_lookups,
                    metadata_lookup_requests_executed = report.stats.executed_requests,
                    elapsed_ms = elapsed_ms_u64(started_at),
                    "library scan match phase completed"
                );
                worker_report = Some(report);
                match_done = true;
            }
            None => break,
        }
    }

    // Channel closure is not proof that either task completed successfully: a
    // panic drops its senders too. Verify both tasks before committing the
    // final hydration batch, and explicitly settle analysis work on failure.
    let candidate_result = jobs_handle
        .await
        .map_err(|error| {
            AppError::Repository(format!("library scan candidate producer panicked: {error}"))
        })
        .and_then(|result| result);
    let worker_result = worker_handle
        .await
        .map_err(|error| {
            AppError::Repository(format!("library scan match worker panicked: {error}"))
        })
        .and_then(|result| result);
    if let Err(error) = candidate_result.and(worker_result) {
        hydration.abort_and_drain().await;
        pool.drain_for_failure().await?;
        return Err(error);
    }

    let canceled = library_scan_cancel_requested(cancel_token.as_ref());
    if !canceled {
        let match_decisions = match_events_matched.saturating_add(match_events_terminal);
        if match_decisions != candidates.len() {
            hydration.abort_and_drain().await;
            pool.drain_for_failure().await?;
            return Err(AppError::Repository(format!(
                "library scan match worker completed with {match_decisions} decisions for {} candidates",
                candidates.len()
            )));
        }
        let Some(report) = worker_report.as_ref() else {
            hydration.abort_and_drain().await;
            pool.drain_for_failure().await?;
            return Err(AppError::Repository(
                "library scan match worker completed without a final report".to_string(),
            ));
        };
        if report.summary.scanned != candidates.len() {
            let reported_scanned = report.summary.scanned;
            hydration.abort_and_drain().await;
            pool.drain_for_failure().await?;
            return Err(AppError::Repository(format!(
                "library scan match worker reported {reported_scanned} scanned candidates after receiving {}",
                candidates.len()
            )));
        }
    }

    if canceled {
        hydration.abort_and_drain().await;
    } else {
        drain_hydration_into_media(StreamingHydrationDrainContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &mut hydration,
            file_total_marked: &mut file_total_marked,
            media_file_total_counted,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;
    }

    if let Some(report) = worker_report.take() {
        summary.absorb(&report.summary);
        if media_dedup_skips > 0 {
            summary.matched = summary.matched.saturating_sub(media_dedup_skips);
        }
        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &hydration,
            file_total_marked: &mut file_total_marked,
            media_file_total_counted,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;
        if !canceled && !file_total_marked {
            log_file_total_latch_blocked(&candidates, media_file_total_counted, started_at);
        }

        pool.close_input();
        summary.absorb(&pool.finish().await?);
        debug!(
            path = %library_path,
            facet = facet.as_str(),
            imported = summary.imported,
            skipped = summary.skipped,
            elapsed_ms = elapsed_ms_u64(started_at),
            "library scan analysis phase completed"
        );

        if !canceled {
            let mut seen_paths = report.seen_paths;
            for runtime in candidates.values() {
                let trimmed = runtime.item_path.trim();
                if !trimmed.is_empty() {
                    seen_paths.insert(normalize_library_scan_item_path(trimmed));
                }
            }
            reconcile_library_scan_unmatched_items(app, facet, library_path, &seen_paths).await?;
            coordinator.publish_progress().await;
        }

        debug!(
            path = %library_path,
            facet = facet.as_str(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups = report.stats.logical_lookups,
            metadata_lookup_requests_executed = report.stats.executed_requests,
            metadata_lookup_requests_coalesced = report.stats.coalesced_requests,
            match_batch_size = LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE,
            match_in_flight_batches = LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES,
            elapsed_ms = elapsed_ms_u64(started_at),
            "{} library scan completed",
            facet.as_str()
        );

        if !report.unmatched_items.is_empty() {
            debug!(
                count = report.unmatched_items.len(),
                facet = facet.as_str(),
                "{} library scan unmatched items follow",
                facet.as_str()
            );
            for unmatched in &report.unmatched_items {
                debug!(
                    path = %unmatched.item_path,
                    display_name = %unmatched.display_name,
                    query = %unmatched.query,
                    year_hint = ?unmatched.year_hint,
                    reason = %unmatched.reason_code,
                    error_message = ?unmatched.error_message,
                    search_attempts = %format_library_scan_unmatched_search_attempts(&unmatched.search_attempts),
                    "{} library scan unmatched item",
                    facet.as_str()
                );
            }
        }
    } else {
        // Worker never reported (cancellation before drain); settle the pool.
        pool.close_input();
        summary.absorb(&pool.finish().await?);
    }

    Ok(summary)
}

fn matched_inventory_totals_ready(
    candidates: &HashMap<ScanCandidateKey, CandidateRuntime>,
) -> bool {
    candidates
        .values()
        .all(|runtime| match runtime.match_state {
            CandidateMatchState::Pending | CandidateMatchState::MatchedAwaitingInventory(_) => {
                false
            }
            CandidateMatchState::Dispatched | CandidateMatchState::Terminal => {
                runtime.inventory_terminal()
            }
        })
}

fn log_file_total_latch_blocked(
    candidates: &HashMap<ScanCandidateKey, CandidateRuntime>,
    media_file_total_counted: usize,
    started_at: Instant,
) {
    let (candidate_pending, candidate_awaiting_inventory, candidate_dispatched, candidate_terminal) =
        candidate_runtime_state_counts(candidates);
    let blockers = candidates
        .iter()
        .filter_map(|(key, runtime)| {
            let blocked = match runtime.match_state {
                CandidateMatchState::Pending | CandidateMatchState::MatchedAwaitingInventory(_) => {
                    true
                }
                CandidateMatchState::Dispatched | CandidateMatchState::Terminal => {
                    !runtime.inventory_terminal()
                }
            };
            blocked.then(|| {
                format!(
                    "{key}:{}:{}:{}",
                    runtime.item_path,
                    candidate_match_state_name(&runtime.match_state),
                    candidate_inventory_state_name(&runtime.inventory)
                )
            })
        })
        .take(5)
        .collect::<Vec<_>>()
        .join(" | ");
    warn!(
        candidates = candidates.len(),
        candidate_pending,
        candidate_awaiting_inventory,
        candidate_dispatched,
        candidate_terminal,
        media_file_total_counted,
        blockers = %blockers,
        elapsed_ms = elapsed_ms_u64(started_at),
        "library scan file total latch blocked after drain"
    );
}

fn candidate_runtime_state_counts(
    candidates: &HashMap<ScanCandidateKey, CandidateRuntime>,
) -> (usize, usize, usize, usize) {
    let mut pending = 0usize;
    let mut awaiting_inventory = 0usize;
    let mut dispatched = 0usize;
    let mut terminal = 0usize;
    for runtime in candidates.values() {
        match runtime.match_state {
            CandidateMatchState::Pending => pending = pending.saturating_add(1),
            CandidateMatchState::MatchedAwaitingInventory(_) => {
                awaiting_inventory = awaiting_inventory.saturating_add(1);
            }
            CandidateMatchState::Dispatched => dispatched = dispatched.saturating_add(1),
            CandidateMatchState::Terminal => terminal = terminal.saturating_add(1),
        }
    }
    (pending, awaiting_inventory, dispatched, terminal)
}

struct CandidateEventContext<'a> {
    app: &'a AppUseCase,
    facet: &'a MediaFacet,
    library_id: &'a str,
    library_path: &'a str,
    session_id: &'a str,
    coordinator: &'a LibraryScanCoordinator,
    summary: &'a mut LibraryScanSummary,
    candidates: &'a mut HashMap<ScanCandidateKey, CandidateRuntime>,
    early_inventory: &'a mut HashMap<ScanCandidateKey, CandidateInventoryState>,
    forward_queue: &'a mut VecDeque<(ScanCandidateKey, ScanPipelineCandidate)>,
    discovery_error: &'a mut Option<AppError>,
}

async fn handle_candidate_job_event(
    ctx: CandidateEventContext<'_>,
    event: ScanCandidateJobEvent,
) -> AppResult<()> {
    match event {
        ScanCandidateJobEvent::Candidate {
            key,
            candidate,
            inline_inventory,
            inventory_cancel,
        } => {
            let item_path = match &candidate {
                ScanPipelineCandidate::Movie(movie) => {
                    normalize_library_scan_item_path(&movie.file.path)
                }
                ScanPipelineCandidate::Series(series) => series.item_path().trim().to_string(),
            };
            let inventory = ctx
                .early_inventory
                .remove(&key)
                .or_else(|| inline_inventory.map(CandidateInventoryState::Ready))
                .unwrap_or(CandidateInventoryState::Pending);
            ctx.candidates.insert(
                key,
                CandidateRuntime {
                    item_path,
                    match_state: CandidateMatchState::Pending,
                    inventory,
                    inventory_cancel,
                },
            );
            ctx.forward_queue.push_back((key, candidate));
        }
        ScanCandidateJobEvent::EvidenceFailed { item_path, error } => {
            warn!(
                item_path = %item_path,
                error = %error,
                "library scan candidate evidence failed"
            );
            let display_name = std::path::Path::new(&item_path)
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| item_path.clone());
            if let Err(persist_error) = persist_ignored_library_scan_item(
                ctx.app,
                ctx.facet,
                ctx.library_id,
                IgnoredLibraryScanItemArgs {
                    title_id: None,
                    session_id: Some(ctx.session_id),
                    library_path: ctx.library_path,
                    item_path: &item_path,
                    display_name: &display_name,
                    query: &display_name,
                    year_hint: None,
                    reason_code: LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE,
                    error_message: Some(error.to_string()),
                    // Evidence failed before any file metadata was gathered.
                    size_bytes: None,
                },
            )
            .await
            {
                warn!(
                    item_path = %item_path,
                    error = %persist_error,
                    "failed to persist ignored library scan evidence failure"
                );
            }
            ctx.summary.scanned += 1;
            ctx.summary.skipped += 1;
            ctx.coordinator.mark_title_match_completed(1).await;
            ctx.coordinator.publish_progress().await;
        }
        ScanCandidateJobEvent::EvidenceDone { .. } => {
            // The coordinator consumes this lifecycle event directly.
        }
        ScanCandidateJobEvent::DiscoveryFailed { error } => {
            *ctx.discovery_error = Some(error);
        }
    }
    Ok(())
}

struct InventoryEventContext<'a> {
    coordinator: &'a LibraryScanCoordinator,
    candidates: &'a mut HashMap<ScanCandidateKey, CandidateRuntime>,
    early_inventory: &'a mut HashMap<ScanCandidateKey, CandidateInventoryState>,
    hydration: &'a mut ScanHydrationBatcher,
    pool: &'a mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &'a mut usize,
    media_file_total_counted: &'a mut usize,
    stored_inventory_paths: &'a mut usize,
    storage_watch: &'a tokio::sync::watch::Sender<usize>,
}

async fn handle_inventory_job_event(
    ctx: InventoryEventContext<'_>,
    event: ScanInventoryJobEvent,
) -> AppResult<()> {
    match event {
        ScanInventoryJobEvent::Inventory { key, files } => {
            handle_inventory_ready(
                ctx.coordinator,
                ctx.candidates,
                ctx.hydration,
                ctx.pool,
                ctx.media_dedup_skips,
                ctx.media_file_total_counted,
                ctx.stored_inventory_paths,
                ctx.storage_watch,
                ctx.early_inventory,
                key,
                files,
            )
            .await?;
        }
        ScanInventoryJobEvent::InventoryFailed {
            key,
            item_path,
            error,
        } => {
            warn!(
                item_path = %item_path,
                error = %error,
                "library scan candidate inventory failed"
            );
            if let Some(runtime) = ctx.candidates.get_mut(&key) {
                runtime.inventory = CandidateInventoryState::Failed;
                if let CandidateMatchState::MatchedAwaitingInventory(_) =
                    std::mem::replace(&mut runtime.match_state, CandidateMatchState::Terminal)
                {
                    // Matched but inventory failed: no media analysis for it.
                }
            } else {
                ctx.early_inventory
                    .insert(key, CandidateInventoryState::Failed);
            }
        }
        ScanInventoryJobEvent::InventoryCanceled { key } => {
            if let Some(runtime) = ctx.candidates.get_mut(&key) {
                runtime.inventory = CandidateInventoryState::Canceled;
                if matches!(
                    runtime.match_state,
                    CandidateMatchState::MatchedAwaitingInventory(_)
                ) {
                    runtime.match_state = CandidateMatchState::Terminal;
                    // Matched but inventory was canceled: no media analysis for it.
                }
            } else {
                ctx.early_inventory
                    .insert(key, CandidateInventoryState::Canceled);
            }
        }
    }
    Ok(())
}

struct TotalKnownLatchContext<'a> {
    coordinator: &'a LibraryScanCoordinator,
    pool: &'a mut LibraryScanMediaAnalysisPool,
    candidates: &'a HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &'a ScanHydrationBatcher,
    file_total_marked: &'a mut bool,
    media_file_total_counted: usize,
    match_done: bool,
    cancel_token: Option<&'a CancellationToken>,
    started_at: Instant,
    library_path: &'a str,
    facet: &'a MediaFacet,
}

async fn try_mark_file_total_known(ctx: TotalKnownLatchContext<'_>) -> AppResult<()> {
    if *ctx.file_total_marked
        || !ctx.match_done
        || library_scan_cancel_requested(ctx.cancel_token)
        || !matched_inventory_totals_ready(ctx.candidates)
    {
        return Ok(());
    }

    ctx.pool.pump().await?;
    if !matched_inventory_totals_ready(ctx.candidates) {
        return Ok(());
    }

    ctx.coordinator.mark_file_total_known().await;
    ctx.coordinator.publish_progress().await;
    *ctx.file_total_marked = true;
    let diagnostics = ctx.pool.diagnostics();
    debug!(
        path = %ctx.library_path,
        facet = ctx.facet.as_str(),
        file_total = ctx.media_file_total_counted,
        hydration_pending = ctx.hydration.pending.len(),
        hydration_in_flight = ctx.hydration.in_flight.len(),
        media_reserved = diagnostics.reserved,
        media_analysis_ready = diagnostics.analysis_ready,
        media_in_flight = diagnostics.in_flight,
        elapsed_ms = elapsed_ms_u64(ctx.started_at),
        "library scan file totals known"
    );
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "rendezvous updates shared pipeline state for one candidate in one place"
)]
async fn handle_inventory_ready(
    coordinator: &LibraryScanCoordinator,
    candidates: &mut HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    media_file_total_counted: &mut usize,
    stored_inventory_paths: &mut usize,
    storage_watch: &tokio::sync::watch::Sender<usize>,
    early_inventory: &mut HashMap<ScanCandidateKey, CandidateInventoryState>,
    key: ScanCandidateKey,
    files: Vec<LibraryFile>,
) -> AppResult<()> {
    let Some(runtime) = candidates.get_mut(&key) else {
        *stored_inventory_paths = stored_inventory_paths.saturating_add(files.len());
        let _ = storage_watch.send(*stored_inventory_paths);
        early_inventory.insert(key, CandidateInventoryState::Ready(files));
        return Ok(());
    };
    if (key as usize).is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL) {
        debug!(
            key,
            files = files.len(),
            stored_inventory_paths = *stored_inventory_paths,
            match_state = candidate_match_state_name(&runtime.match_state),
            inventory_state = candidate_inventory_state_name(&runtime.inventory),
            "library scan inventory ready diagnostic"
        );
    }

    match &mut runtime.match_state {
        CandidateMatchState::MatchedAwaitingInventory(_) => {
            let CandidateMatchState::MatchedAwaitingInventory(work) =
                std::mem::replace(&mut runtime.match_state, CandidateMatchState::Dispatched)
            else {
                unreachable!("checked variant above");
            };
            runtime.inventory = CandidateInventoryState::Consumed;
            dispatch_media_work(
                coordinator,
                hydration,
                pool,
                media_dedup_skips,
                media_file_total_counted,
                *work,
                files,
            )
            .await?;
        }
        CandidateMatchState::Pending => {
            *stored_inventory_paths = stored_inventory_paths.saturating_add(files.len());
            let _ = storage_watch.send(*stored_inventory_paths);
            runtime.inventory = CandidateInventoryState::Ready(files);
        }
        CandidateMatchState::Dispatched | CandidateMatchState::Terminal => {
            // Unmatched/duplicate inventory: discard immediately.
            runtime.inventory = CandidateInventoryState::Consumed;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "match decisions update shared pipeline state for one candidate in one place"
)]
async fn handle_match_decision(
    coordinator: &LibraryScanCoordinator,
    candidates: &mut HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    media_file_total_counted: &mut usize,
    stored_inventory_paths: &mut usize,
    storage_watch: &tokio::sync::watch::Sender<usize>,
    key: ScanCandidateKey,
    matched_work: Option<LibraryScanTitleWork>,
) -> AppResult<()> {
    let started_at = Instant::now();
    let Some(runtime) = candidates.get_mut(&key) else {
        return Ok(());
    };
    let initial_match_state = candidate_match_state_name(&runtime.match_state);
    let initial_inventory_state = candidate_inventory_state_name(&runtime.inventory);
    let matched = matched_work.is_some();
    let matched_title_name = matched_work.as_ref().map(|work| work.title.name.clone());
    let mut file_count = 0usize;
    let outcome: &'static str;

    match matched_work {
        Some(work) => {
            match std::mem::replace(&mut runtime.inventory, CandidateInventoryState::Consumed) {
                CandidateInventoryState::Ready(files) => {
                    file_count = files.len();
                    *stored_inventory_paths = stored_inventory_paths.saturating_sub(files.len());
                    let _ = storage_watch.send(*stored_inventory_paths);
                    runtime.match_state = CandidateMatchState::Dispatched;
                    outcome = "dispatched_inventory_ready";
                    dispatch_media_work(
                        coordinator,
                        hydration,
                        pool,
                        media_dedup_skips,
                        media_file_total_counted,
                        work,
                        files,
                    )
                    .await?;
                }
                CandidateInventoryState::Pending => {
                    runtime.inventory = CandidateInventoryState::Pending;
                    runtime.match_state =
                        CandidateMatchState::MatchedAwaitingInventory(Box::new(work));
                    outcome = "matched_awaiting_inventory";
                }
                CandidateInventoryState::Failed => {
                    runtime.inventory = CandidateInventoryState::Failed;
                    runtime.match_state = CandidateMatchState::Terminal;
                    outcome = "inventory_failed";
                    warn!(
                        item_path = %runtime.item_path,
                        "matched candidate has failed media inventory; skipping analysis"
                    );
                }
                CandidateInventoryState::Canceled | CandidateInventoryState::Consumed => {
                    runtime.match_state = CandidateMatchState::Terminal;
                    outcome = "inventory_unavailable";
                }
            }
        }
        None => {
            // Unmatched/failed/skipped: cancel any in-flight inventory walk
            // and discard stored file lists at decision time.
            runtime.inventory_cancel.cancel();
            if let CandidateInventoryState::Ready(files) =
                std::mem::replace(&mut runtime.inventory, CandidateInventoryState::Consumed)
            {
                file_count = files.len();
                *stored_inventory_paths = stored_inventory_paths.saturating_sub(files.len());
                let _ = storage_watch.send(*stored_inventory_paths);
            } else if matches!(runtime.inventory, CandidateInventoryState::Consumed) {
                // preserved state
            }
            runtime.match_state = CandidateMatchState::Terminal;
            outcome = "unmatched_or_terminal";
        }
    }
    let elapsed_ms = elapsed_ms_u64(started_at);
    if elapsed_ms >= 500 || (key as usize).is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL) {
        debug!(
            key,
            title_name = matched_title_name.as_deref(),
            matched,
            file_count,
            outcome,
            initial_match_state,
            initial_inventory_state,
            final_match_state = candidate_match_state_name(&runtime.match_state),
            final_inventory_state = candidate_inventory_state_name(&runtime.inventory),
            stored_inventory_paths = *stored_inventory_paths,
            hydration_pending = hydration.pending.len(),
            hydration_in_flight = hydration.in_flight.len(),
            media_dedup_skips = *media_dedup_skips,
            elapsed_ms,
            "library scan coordinator match decision diagnostic"
        );
    }
    Ok(())
}

async fn commit_hydration_batch(
    pool: &mut LibraryScanMediaAnalysisPool,
    batch: ScanHydrationBatchResult,
) -> AppResult<()> {
    for (reservation, reason) in batch.failed {
        pool.fail_reserved(reservation, &reason).await;
    }
    for reservation in batch.ready {
        pool.commit_reserved(reservation);
    }
    pool.pump().await
}

struct StreamingHydrationDrainContext<'a> {
    coordinator: &'a LibraryScanCoordinator,
    pool: &'a mut LibraryScanMediaAnalysisPool,
    candidates: &'a HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &'a mut ScanHydrationBatcher,
    file_total_marked: &'a mut bool,
    media_file_total_counted: usize,
    match_done: bool,
    cancel_token: Option<&'a CancellationToken>,
    started_at: Instant,
    library_path: &'a str,
    facet: &'a MediaFacet,
}

async fn drain_hydration_into_media(ctx: StreamingHydrationDrainContext<'_>) -> AppResult<()> {
    try_mark_file_total_known(TotalKnownLatchContext {
        coordinator: ctx.coordinator,
        pool: ctx.pool,
        candidates: ctx.candidates,
        hydration: ctx.hydration,
        file_total_marked: ctx.file_total_marked,
        media_file_total_counted: ctx.media_file_total_counted,
        match_done: ctx.match_done,
        cancel_token: ctx.cancel_token,
        started_at: ctx.started_at,
        library_path: ctx.library_path,
        facet: ctx.facet,
    })
    .await?;

    ctx.hydration.flush_due();
    ctx.hydration.maybe_flush();
    while ctx.hydration.has_pending_or_in_flight() {
        let batch = ctx.hydration.join_next().await?;
        let batch_len = batch.ready.len().saturating_add(batch.failed.len());
        commit_hydration_batch(ctx.pool, batch).await?;
        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: ctx.coordinator,
            pool: ctx.pool,
            candidates: ctx.candidates,
            hydration: ctx.hydration,
            file_total_marked: ctx.file_total_marked,
            media_file_total_counted: ctx.media_file_total_counted,
            match_done: ctx.match_done,
            cancel_token: ctx.cancel_token,
            started_at: ctx.started_at,
            library_path: ctx.library_path,
            facet: ctx.facet,
        })
        .await?;

        let diagnostics = ctx.pool.diagnostics();
        debug!(
            path = %ctx.library_path,
            facet = ctx.facet.as_str(),
            batch_len,
            hydration_pending = ctx.hydration.pending.len(),
            hydration_in_flight = ctx.hydration.in_flight.len(),
            media_analysis_ready = diagnostics.analysis_ready,
            media_in_flight = diagnostics.in_flight,
            elapsed_ms = elapsed_ms_u64(ctx.started_at),
            "library scan final hydration chunk committed"
        );

        ctx.hydration.flush_due();
        ctx.hydration.maybe_flush();
    }
    Ok(())
}

async fn dispatch_media_work(
    coordinator: &LibraryScanCoordinator,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    media_file_total_counted: &mut usize,
    mut work: LibraryScanTitleWork,
    files: Vec<LibraryFile>,
) -> AppResult<()> {
    let started_at = Instant::now();
    let title_id = work.title.id.clone();
    let title_name = work.title.name.clone();
    let input_file_count = files.len();
    if matches!(work.scope, LibraryScanTitleWorkScope::FullFolder)
        || work
            .discovered_files()
            .is_some_and(|existing| existing.is_empty())
    {
        work.scope = LibraryScanTitleWorkScope::PreEnumeratedFullFolder(files);
    }

    let Some(reservation) = pool.reserve_work(work) else {
        if input_file_count > 0 {
            *media_dedup_skips = media_dedup_skips.saturating_add(1);
        }
        let diagnostics = pool.diagnostics();
        debug!(
            title_id = %title_id,
            title_name = %title_name,
            input_file_count,
            media_dedup_skips = *media_dedup_skips,
            media_reserved = diagnostics.reserved,
            media_analysis_ready = diagnostics.analysis_ready,
            media_in_flight = diagnostics.in_flight,
            elapsed_ms = elapsed_ms_u64(started_at),
            "library scan media work reservation skipped"
        );
        return Ok(());
    };

    let counted_files = reservation.file_count();
    if counted_files > 0 {
        *media_file_total_counted = (*media_file_total_counted).saturating_add(counted_files);
        coordinator.add_file_total(counted_files).await;
        coordinator.publish_progress().await;
    }

    match hydration.submit(reservation).await? {
        ScanHydrationSubmission::Queued => {
            let elapsed_ms = elapsed_ms_u64(started_at);
            if elapsed_ms >= 250 {
                debug!(
                    title_id = %title_id,
                    title_name = %title_name,
                    input_file_count,
                    counted_files,
                    hydration_pending = hydration.pending.len(),
                    hydration_in_flight = hydration.in_flight.len(),
                    elapsed_ms,
                    "library scan media work queued for hydration"
                );
            }
            Ok(())
        }
        ScanHydrationSubmission::Ready(reservation) => {
            pool.commit_reserved(*reservation);
            pool.pump().await?;
            coordinator.publish_progress().await;
            let diagnostics = pool.diagnostics();
            let enqueue_elapsed_ms = elapsed_ms_u64(started_at);
            if enqueue_elapsed_ms >= 250 {
                debug!(
                    title_id = %title_id,
                    title_name = %title_name,
                    input_file_count,
                    counted_files,
                    media_reserved = diagnostics.reserved,
                    media_analysis_ready = diagnostics.analysis_ready,
                    media_in_flight = diagnostics.in_flight,
                    media_walk_tasks = diagnostics.walk_tasks,
                    elapsed_ms = enqueue_elapsed_ms,
                    "library scan media work committed without hydration"
                );
            }
            Ok(())
        }
    }?;

    let pump_elapsed_ms = elapsed_ms_u64(started_at);
    if pump_elapsed_ms >= 500 {
        let diagnostics = pool.diagnostics();
        debug!(
            title_id = %title_id,
            title_name = %title_name,
            input_file_count,
            counted_files,
            media_reserved = diagnostics.reserved,
            media_pending_full = diagnostics.pending_full,
            media_pending_scoped = diagnostics.pending_scoped,
            media_analysis_ready = diagnostics.analysis_ready,
            media_in_flight = diagnostics.in_flight,
            media_walk_tasks = diagnostics.walk_tasks,
            media_completed = diagnostics.completed,
            elapsed_ms = pump_elapsed_ms,
            "library scan media work pump diagnostic"
        );
    }
    Ok(())
}

/// Batches titles that need SMG hydration before media analysis so hydration
/// stays in bulk requests and off the candidate-to-match critical path.
struct ScanHydrationBatchResult {
    ready: Vec<LibraryScanMediaWorkReservation>,
    failed: Vec<(LibraryScanMediaWorkReservation, String)>,
}

enum ScanHydrationSubmission {
    Queued,
    Ready(Box<LibraryScanMediaWorkReservation>),
}

struct ScanHydrationBatcher {
    app: AppUseCase,
    cancel_token: Option<CancellationToken>,
    pending: VecDeque<LibraryScanMediaWorkReservation>,
    first_pending_at: Option<Instant>,
    flush_requested: bool,
    in_flight: tokio::task::JoinSet<AppResult<ScanHydrationBatchResult>>,
}

impl ScanHydrationBatcher {
    fn new(app: AppUseCase, cancel_token: Option<CancellationToken>) -> Self {
        Self {
            app,
            cancel_token,
            pending: VecDeque::new(),
            first_pending_at: None,
            flush_requested: false,
            in_flight: tokio::task::JoinSet::new(),
        }
    }

    async fn submit(
        &mut self,
        reservation: LibraryScanMediaWorkReservation,
    ) -> AppResult<ScanHydrationSubmission> {
        let metadata_language = self
            .app
            .resolve_metadata_language_for_title(&reservation.work.title)
            .await;
        if title_requires_scan_hydration(&self.app, &reservation.work.title, &metadata_language)
            .await?
        {
            if self.pending.is_empty() {
                self.first_pending_at = Some(Instant::now());
            }
            self.pending.push_back(reservation);
            if self.pending.len() >= crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE {
                self.flush_requested = true;
            }
            Ok(ScanHydrationSubmission::Queued)
        } else {
            Ok(ScanHydrationSubmission::Ready(Box::new(reservation)))
        }
    }

    fn deadline_instant(&self) -> Option<tokio::time::Instant> {
        if self.pending.is_empty()
            || self.in_flight.len() >= LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES
        {
            // A full in-flight window wakes via join_next; arming the timer
            // too would spin on an already-expired deadline.
            return None;
        }
        self.first_pending_at
            .map(|first| tokio::time::Instant::from_std(first) + LIBRARY_SCAN_MATCH_FLUSH_INTERVAL)
    }

    fn flush_due(&mut self) {
        self.flush_requested = true;
    }

    fn maybe_flush(&mut self) {
        while self.flush_requested
            && !self.pending.is_empty()
            && self.in_flight.len() < LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES
        {
            let batch_len = self
                .pending
                .len()
                .min(crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE);
            let batch = self.pending.drain(..batch_len).collect::<Vec<_>>();
            if self.pending.is_empty() {
                self.flush_requested = false;
                self.first_pending_at = None;
            } else {
                self.flush_requested = true;
                self.first_pending_at = Some(Instant::now());
            }
            debug!(
                batch_len = batch.len(),
                pending = self.pending.len(),
                in_flight = self.in_flight.len(),
                "library scan hydration chunk dispatched"
            );
            let app = self.app.clone();
            let cancel_token = self.cancel_token.clone();
            self.in_flight.spawn(async move {
                hydrate_library_scan_title_works(&app, batch, cancel_token.as_ref()).await
            });
        }
    }

    fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    fn has_pending_or_in_flight(&self) -> bool {
        !self.pending.is_empty() || !self.in_flight.is_empty()
    }

    async fn join_next(&mut self) -> AppResult<ScanHydrationBatchResult> {
        match self.in_flight.join_next().await {
            Some(Ok(result)) => result,
            Some(Err(error)) if error.is_cancelled() => Ok(ScanHydrationBatchResult {
                ready: Vec::new(),
                failed: Vec::new(),
            }),
            Some(Err(error)) => Err(AppError::Repository(error.to_string())),
            None => Ok(ScanHydrationBatchResult {
                ready: Vec::new(),
                failed: Vec::new(),
            }),
        }
    }

    fn abort(&mut self) {
        self.pending.clear();
        self.in_flight.abort_all();
    }

    async fn abort_and_drain(&mut self) {
        self.abort();
        while self.in_flight.join_next().await.is_some() {}
    }
}

async fn hydrate_library_scan_title_works(
    app: &AppUseCase,
    reservations: Vec<LibraryScanMediaWorkReservation>,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<ScanHydrationBatchResult> {
    let started_at = Instant::now();
    let targets = reservations
        .iter()
        .map(|reservation| crate::catalog_workflow::HydrationTarget {
            title: reservation.work.title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::LibraryScanFull,
        })
        .collect::<Vec<_>>();

    let outcome = app
        .hydrate_titles_bulk_cancellable(targets, cancel_token)
        .await?;

    let mut hydrated_by_id: HashMap<String, Title> = outcome.hydrated_titles.into_iter().collect();
    let failed: HashMap<String, String> = outcome.failed_titles.into_iter().collect();

    let mut ready = Vec::with_capacity(reservations.len());
    let mut failed_reservations = Vec::new();
    for mut reservation in reservations {
        if let Some(reason) = failed.get(&reservation.work.title.id) {
            warn!(
                title_id = %reservation.work.title.id,
                reason = %reason,
                "library scan title hydration failed"
            );
            failed_reservations.push((reservation, reason.clone()));
            continue;
        }
        if let Some(hydrated) = hydrated_by_id.remove(&reservation.work.title.id) {
            reservation.work.title = hydrated;
        }
        ready.push(reservation);
    }
    debug!(
        batch_len = ready.len().saturating_add(failed_reservations.len()),
        hydrated = ready.len(),
        failed = failed_reservations.len(),
        elapsed_ms = elapsed_ms_u64(started_at),
        "library scan hydration chunk completed"
    );
    Ok(ScanHydrationBatchResult {
        ready,
        failed: failed_reservations,
    })
}

struct CandidateJobContext {
    app: AppUseCase,
    session_id: String,
    library_path: String,
    kind: LibraryScanPipelineKind,
    scan_hints: Option<LibraryScanHintSet>,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    candidate_events: tokio::sync::mpsc::UnboundedSender<ScanCandidateJobEvent>,
    inventory_events: tokio::sync::mpsc::UnboundedSender<ScanInventoryJobEvent>,
    storage_watch: tokio::sync::watch::Receiver<usize>,
}

fn spawn_candidate_jobs(
    ctx: CandidateJobContext,
) -> AppResult<tokio::task::JoinHandle<AppResult<()>>> {
    Ok(tokio::spawn(async move {
        let result = match ctx.kind {
            LibraryScanPipelineKind::Movie => run_movie_candidate_jobs(&ctx).await,
            LibraryScanPipelineKind::Series => run_series_candidate_jobs(&ctx).await,
        };
        if let Err(error) = &result {
            let _ = ctx
                .candidate_events
                .send(ScanCandidateJobEvent::DiscoveryFailed {
                    error: AppError::Repository(format!(
                        "library scan candidate producer failed: {error}"
                    )),
                });
        }
        result
    }))
}

enum EvidenceJobOutput {
    Candidate {
        key: ScanCandidateKey,
        candidate: ScanPipelineCandidate,
        inline_inventory: Option<Vec<LibraryFile>>,
        inventory_target: Option<PathBuf>,
    },
    Failed {
        item_path: String,
        error: AppError,
    },
}

struct CandidateJobRunner<'a> {
    ctx: &'a CandidateJobContext,
    next_key: ScanCandidateKey,
    evidence_set: tokio::task::JoinSet<EvidenceJobOutput>,
    inventory_set: tokio::task::JoinSet<()>,
    inventory_queue: VecDeque<(ScanCandidateKey, PathBuf, CancellationToken)>,
    cancel_tokens: HashMap<ScanCandidateKey, CancellationToken>,
    metrics: CandidateJobMetrics,
}

impl<'a> CandidateJobRunner<'a> {
    fn new(ctx: &'a CandidateJobContext) -> Self {
        Self {
            ctx,
            next_key: 0,
            evidence_set: tokio::task::JoinSet::new(),
            inventory_set: tokio::task::JoinSet::new(),
            inventory_queue: VecDeque::new(),
            cancel_tokens: HashMap::new(),
            metrics: CandidateJobMetrics::default(),
        }
    }

    fn allocate_key(&mut self) -> ScanCandidateKey {
        let key = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        key
    }

    async fn forward_evidence_output(&mut self, output: EvidenceJobOutput) -> bool {
        match output {
            EvidenceJobOutput::Candidate {
                key,
                candidate,
                inline_inventory,
                inventory_target,
            } => {
                let inventory_cancel = self.cancel_tokens.entry(key).or_default().clone();
                let has_inline_inventory = inline_inventory.is_some();
                if self
                    .ctx
                    .candidate_events
                    .send(ScanCandidateJobEvent::Candidate {
                        key,
                        candidate,
                        inline_inventory,
                        inventory_cancel: inventory_cancel.clone(),
                    })
                    .is_err()
                {
                    return false;
                }
                self.metrics.candidates_emitted = self.metrics.candidates_emitted.saturating_add(1);
                if self
                    .metrics
                    .candidates_emitted
                    .is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL)
                {
                    debug!(
                        kind = ?self.ctx.kind,
                        candidates_emitted = self.metrics.candidates_emitted,
                        skipped = self.metrics.skipped,
                        failed = self.metrics.failed,
                        inline_inventory = self.metrics.inline_inventory_emitted,
                        inventory_walks_queued = self.metrics.inventory_walks_queued,
                        inventory_walks_started = self.metrics.inventory_walks_started,
                        evidence_in_flight = self.evidence_set.len(),
                        inventory_in_flight = self.inventory_set.len(),
                        inventory_queue = self.inventory_queue.len(),
                        "library scan candidate producer diagnostic"
                    );
                }
                if has_inline_inventory {
                    self.metrics.inline_inventory_emitted =
                        self.metrics.inline_inventory_emitted.saturating_add(1);
                } else if let Some(target) = inventory_target {
                    self.metrics.inventory_walks_queued =
                        self.metrics.inventory_walks_queued.saturating_add(1);
                    self.inventory_queue
                        .push_back((key, target, inventory_cancel));
                }
                true
            }
            EvidenceJobOutput::Failed { item_path, error } => {
                self.metrics.failed = self.metrics.failed.saturating_add(1);
                self.ctx
                    .candidate_events
                    .send(ScanCandidateJobEvent::EvidenceFailed { item_path, error })
                    .is_ok()
            }
        }
    }

    /// Launch queued inventory walks. In the evidence loop this is
    /// non-blocking (`block = false`): if no walk permit is free or the
    /// rendezvous high-water gate is engaged, queued inventory simply waits —
    /// evidence emission is never paused for inventory. `settle` passes
    /// `block = true` once no evidence work remains.
    async fn launch_pending_inventory(&mut self, block: bool) {
        while !self.inventory_queue.is_empty() {
            if library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
                self.inventory_queue.clear();
                return;
            }

            // Rendezvous storage high-water gate: pause the inventory phase
            // (never evidence) until stored file lists drain.
            let mut storage = self.ctx.storage_watch.clone();
            if !block && *storage.borrow() >= LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER {
                return;
            }
            while *storage.borrow() >= LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER {
                if storage.changed().await.is_err() {
                    return;
                }
            }

            let semaphore = self
                .ctx
                .app
                .runtime
                .library
                .library_scan_title_walk_limit
                .clone();
            let permit = if block {
                tokio::select! {
                    permit = semaphore.acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => return,
                    },
                    _ = async {
                        match self.ctx.cancel_token.as_ref() {
                            Some(token) => token.cancelled().await,
                            None => std::future::pending::<()>().await,
                        }
                    }, if self.ctx.cancel_token.is_some() => {
                        self.inventory_queue.clear();
                        return;
                    }
                }
            } else {
                match semaphore.try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            };
            let Some((key, target, cancel)) = self.inventory_queue.pop_front() else {
                return;
            };
            self.metrics.inventory_walks_started =
                self.metrics.inventory_walks_started.saturating_add(1);
            let events = self.ctx.inventory_events.clone();
            let app = self.ctx.app.clone();
            let kind = self.ctx.kind;
            let scan_cancel = self.ctx.cancel_token.clone();
            self.inventory_set.spawn(async move {
                let _permit = permit;
                if cancel.is_cancelled() || library_scan_cancel_requested(scan_cancel.as_ref()) {
                    let _ = events.send(ScanInventoryJobEvent::InventoryCanceled { key });
                    return;
                }

                let target_str = path_to_stored_string(&target);
                let result = match kind {
                    LibraryScanPipelineKind::Movie => {
                        app.services
                            .library
                            .library_scanner
                            .scan_directory(target_str.as_str())
                            .await
                    }
                    LibraryScanPipelineKind::Series => app
                        .services
                        .library
                        .library_scanner
                        .scan_directory_for_progress_with_metrics(target_str.as_str())
                        .await
                        .map(|scan| scan.files),
                };

                let event = match result {
                    Ok(_) if cancel.is_cancelled() => {
                        ScanInventoryJobEvent::InventoryCanceled { key }
                    }
                    Ok(mut files) => {
                        files.sort_by(|left, right| left.path.cmp(&right.path));
                        ScanInventoryJobEvent::Inventory { key, files }
                    }
                    Err(error) => ScanInventoryJobEvent::InventoryFailed {
                        key,
                        item_path: target_str,
                        error,
                    },
                };
                let _ = events.send(event);
            });
        }
    }

    async fn drain_evidence(&mut self) -> AppResult<bool> {
        while !self.evidence_set.is_empty() {
            if library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
                self.evidence_set.abort_all();
                return Ok(false);
            }
            self.launch_pending_inventory(false).await;
            match self.evidence_set.join_next().await {
                Some(Ok(output)) => {
                    if !self.forward_evidence_output(output).await {
                        return Ok(false);
                    }
                }
                Some(Err(error))
                    if error.is_cancelled()
                        && library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) =>
                {
                    return Ok(false);
                }
                Some(Err(error)) => {
                    return Err(AppError::Repository(format!(
                        "library scan evidence task failed: {error}"
                    )));
                }
                None => break,
            }
        }
        Ok(true)
    }

    fn send_evidence_done(&self) -> bool {
        self.ctx
            .candidate_events
            .send(ScanCandidateJobEvent::EvidenceDone {
                metrics: self.metrics,
            })
            .is_ok()
    }

    async fn settle(mut self) -> AppResult<()> {
        let mut task_error = None;
        while !library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
            self.launch_pending_inventory(true).await;
            if self.evidence_set.is_empty()
                && self.inventory_set.is_empty()
                && self.inventory_queue.is_empty()
            {
                break;
            }

            let cancelled = async {
                match self.ctx.cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = cancelled, if self.ctx.cancel_token.is_some() => {
                    break;
                }
                output = self.evidence_set.join_next(), if !self.evidence_set.is_empty() => {
                    match output {
                        Some(Ok(output)) => {
                            if !self.forward_evidence_output(output).await {
                                break;
                            }
                        }
                        Some(Err(error))
                            if error.is_cancelled()
                                && library_scan_cancel_requested(
                                    self.ctx.cancel_token.as_ref(),
                                ) => {}
                        Some(Err(error)) => {
                            task_error = Some(AppError::Repository(format!(
                                "library scan evidence task failed while settling: {error}"
                            )));
                            break;
                        }
                        None => {}
                    }
                }
                output = self.inventory_set.join_next(), if !self.inventory_set.is_empty() => {
                    match output {
                        Some(Ok(_)) | None => {}
                        Some(Err(error))
                            if error.is_cancelled()
                                && library_scan_cancel_requested(
                                    self.ctx.cancel_token.as_ref(),
                                ) => {}
                        Some(Err(error)) => {
                            task_error = Some(AppError::Repository(format!(
                                "library scan inventory task failed: {error}"
                            )));
                            break;
                        }
                    }
                }
            }
        }

        // Abort-and-join both pools; a no-op when they drained normally.
        // Joining (not just aborting) is required so cancellation cannot
        // leave tasks parked in the sets.
        self.inventory_queue.clear();
        self.evidence_set.abort_all();
        while let Some(output) = self.evidence_set.join_next().await {
            if task_error.is_none()
                && let Err(error) = output
                && !error.is_cancelled()
            {
                task_error = Some(AppError::Repository(format!(
                    "library scan evidence task panicked while draining: {error}"
                )));
            }
        }
        self.inventory_set.abort_all();
        while let Some(output) = self.inventory_set.join_next().await {
            if task_error.is_none()
                && let Err(error) = output
                && !error.is_cancelled()
            {
                task_error = Some(AppError::Repository(format!(
                    "library scan inventory task panicked while draining: {error}"
                )));
            }
        }

        match task_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

async fn run_movie_candidate_jobs(ctx: &CandidateJobContext) -> AppResult<()> {
    let root = require_directory_library_path(&ctx.library_path)?.to_path_buf();
    let discovered_entries =
        stream_movie_top_level_entries_batched(&root, LIBRARY_SCAN_MOVIE_BATCH_SIZE).await?;
    let mut queued_entries = spawn_library_discovery_queue(
        ctx.app.clone(),
        ctx.session_id.clone(),
        discovered_entries,
        false,
        ctx.mark_discovery_complete_on_drain,
        ctx.cancel_token.clone(),
    );

    let mut runner = CandidateJobRunner::new(ctx);
    let mut pending_entries: VecDeque<MovieTopLevelEntry> = VecDeque::new();
    let mut discovery_closed = false;

    loop {
        if library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
            pending_entries.clear();
            runner.evidence_set.abort_all();
            runner.inventory_queue.clear();
            runner.inventory_set.abort_all();
            break;
        }

        while runner.evidence_set.len() < LIBRARY_SCAN_EVIDENCE_CONCURRENCY {
            let Some(entry) = pending_entries.pop_front() else {
                break;
            };
            let key = runner.allocate_key();
            let scanner = ctx.app.services.library.library_scanner.clone();
            let library_path = ctx.library_path.clone();
            let scan_hints = ctx.scan_hints.clone();
            runner.evidence_set.spawn(async move {
                movie_evidence_job(scanner, entry, library_path, scan_hints, key).await
            });
        }

        runner.launch_pending_inventory(false).await;

        if discovery_closed && pending_entries.is_empty() && runner.evidence_set.is_empty() {
            break;
        }

        tokio::select! {
            maybe_batch = queued_entries.recv(), if !discovery_closed => {
                match maybe_batch {
                    Some(Ok(batch)) => {
                        let batch_len = batch.len();
                        pending_entries.extend(batch);
                        debug!(
                            kind = ?ctx.kind,
                            batch_len,
                            pending_entries = pending_entries.len(),
                            evidence_in_flight = runner.evidence_set.len(),
                            inventory_in_flight = runner.inventory_set.len(),
                            inventory_queue = runner.inventory_queue.len(),
                            "library scan candidate producer received discovery batch"
                        );
                    }
                    Some(Err(error)) => return Err(error),
                    None => discovery_closed = true,
                }
            }
            Some(output) = runner.evidence_set.join_next(), if !runner.evidence_set.is_empty() => {
                match output {
                    Ok(output) => {
                        if !runner.forward_evidence_output(output).await {
                            return Ok(());
                        }
                    }
                    Err(error) if error.is_cancelled()
                        && library_scan_cancel_requested(ctx.cancel_token.as_ref()) =>
                    {
                        return Ok(());
                    }
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "library scan movie evidence task panicked: {error}"
                        )));
                    }
                }
            }
        }
    }

    if !runner.drain_evidence().await? || !runner.send_evidence_done() {
        return Ok(());
    }
    runner.settle().await?;
    Ok(())
}

async fn movie_evidence_job(
    scanner: Arc<dyn LibraryScanner>,
    entry: MovieTopLevelEntry,
    library_path: String,
    scan_hints: Option<LibraryScanHintSet>,
    key: ScanCandidateKey,
) -> EvidenceJobOutput {
    let item_path = path_to_stored_string(&entry.path);
    let is_dir = entry.is_dir;
    let entry_path = entry.path.clone();
    match prepare_movie_candidate_evidence(scanner, entry, library_path, scan_hints.as_ref()).await
    {
        Ok(MovieCandidateEvidence::Candidate {
            candidate,
            inline_inventory,
        }) => EvidenceJobOutput::Candidate {
            key,
            candidate: ScanPipelineCandidate::Movie(candidate),
            inventory_target: (inline_inventory.is_none() && is_dir).then_some(entry_path),
            inline_inventory,
        },
        Err(error) => EvidenceJobOutput::Failed { item_path, error },
    }
}

async fn run_series_candidate_jobs(ctx: &CandidateJobContext) -> AppResult<()> {
    let root = require_directory_library_path(&ctx.library_path)?.to_path_buf();
    let discovered_folders =
        stream_child_directories_batched(&root, LIBRARY_SCAN_SERIES_BATCH_SIZE).await?;
    let mut queued_folders = spawn_library_discovery_queue(
        ctx.app.clone(),
        ctx.session_id.clone(),
        discovered_folders,
        false,
        false,
        ctx.cancel_token.clone(),
    );

    let coordinator = LibraryScanCoordinator::new(ctx.app.clone(), ctx.session_id.clone());
    let mut runner = CandidateJobRunner::new(ctx);
    let mut pending_folders: VecDeque<PathBuf> = VecDeque::new();
    let mut discovery_closed = false;

    loop {
        if library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
            pending_folders.clear();
            runner.evidence_set.abort_all();
            runner.inventory_queue.clear();
            runner.inventory_set.abort_all();
            break;
        }

        while runner.evidence_set.len() < LIBRARY_SCAN_EVIDENCE_CONCURRENCY {
            let Some(folder) = pending_folders.pop_front() else {
                break;
            };
            let key = runner.allocate_key();
            let scan_hints = ctx.scan_hints.clone();
            runner
                .evidence_set
                .spawn(async move { series_evidence_job(folder, scan_hints, key).await });
        }

        runner.launch_pending_inventory(false).await;

        if discovery_closed && pending_folders.is_empty() && runner.evidence_set.is_empty() {
            break;
        }

        tokio::select! {
            maybe_batch = queued_folders.recv(), if !discovery_closed => {
                match maybe_batch {
                    Some(Ok(batch)) => {
                        let batch_len = batch.len();
                        pending_folders.extend(batch);
                        debug!(
                            kind = ?ctx.kind,
                            batch_len,
                            pending_folders = pending_folders.len(),
                            evidence_in_flight = runner.evidence_set.len(),
                            inventory_in_flight = runner.inventory_set.len(),
                            inventory_queue = runner.inventory_queue.len(),
                            "library scan candidate producer received discovery batch"
                        );
                    }
                    Some(Err(error)) => return Err(error),
                    None => discovery_closed = true,
                }
            }
            Some(output) = runner.evidence_set.join_next(), if !runner.evidence_set.is_empty() => {
                match output {
                    Ok(output) => {
                        if !runner.forward_evidence_output(output).await {
                            return Ok(());
                        }
                    }
                    Err(error) if error.is_cancelled()
                        && library_scan_cancel_requested(ctx.cancel_token.as_ref()) =>
                    {
                        return Ok(());
                    }
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "library scan series evidence task panicked: {error}"
                        )));
                    }
                }
            }
        }
    }

    // A title owns a directory, never the library root. Loose root-level media
    // remains untouched and outside the catalog until the operator organizes it.
    if !library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
        let loose_root_file_count = count_series_loose_root_files(&root).await?;
        if loose_root_file_count > 0 {
            warn!(
                root = %root.display(),
                files = loose_root_file_count,
                "skipping loose media files in library root"
            );
        }
    }

    // DiscoveryDone: the root-level pass (folders plus loose files) is
    // complete; the title-match total is now deterministic even though
    // evidence and inventory jobs may still be running.
    if ctx.mark_discovery_complete_on_drain
        && !library_scan_cancel_requested(ctx.cancel_token.as_ref())
    {
        coordinator.mark_discovery_complete(false).await;
        coordinator.publish_progress().await;
    }

    if !runner.drain_evidence().await? || !runner.send_evidence_done() {
        return Ok(());
    }
    runner.settle().await?;
    Ok(())
}

async fn series_evidence_job(
    folder: PathBuf,
    scan_hints: Option<LibraryScanHintSet>,
    key: ScanCandidateKey,
) -> EvidenceJobOutput {
    let item_path = path_to_stored_string(&folder);
    match prepare_series_library_scan_candidate(folder.clone(), scan_hints.as_ref()).await {
        Ok(candidate) => EvidenceJobOutput::Candidate {
            key,
            candidate: ScanPipelineCandidate::Series(Box::new(candidate)),
            inline_inventory: None,
            inventory_target: Some(folder),
        },
        Err(error) => EvidenceJobOutput::Failed { item_path, error },
    }
}

struct ScanMatchWorkerContext {
    app: AppUseCase,
    actor: User,
    facet: MediaFacet,
    library_id: String,
    library_path: String,
    session_id: String,
    metadata_language: String,
    kind: LibraryScanPipelineKind,
}

struct QueuedMatchCandidate {
    key: ScanCandidateKey,
    candidate: ScanPipelineCandidate,
    queued_at: Instant,
}

struct ScanMatchWorkerState {
    existing_titles: Vec<Title>,
    existing_titles_by_name: TitleNameIndex,
    existing_titles_by_smg_id: HashMap<String, usize>,
    existing_titles_by_tvdb_id: HashMap<String, usize>,
    existing_titles_by_imdb_id: HashMap<String, usize>,
    existing_titles_by_tmdb_id: HashMap<String, usize>,
    search_results: MetadataSearchResults,
    accounted_search_keys: HashSet<BatchMetadataSearchKey>,
    report: ScanMatchWorkerReport,
}

async fn run_scan_match_worker(
    ctx: ScanMatchWorkerContext,
    mut input: tokio::sync::mpsc::Receiver<(ScanCandidateKey, ScanPipelineCandidate)>,
    events: tokio::sync::mpsc::UnboundedSender<ScanMatchWorkerEvent>,
    cancel_token: Option<CancellationToken>,
) -> AppResult<()> {
    let worker_started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(ctx.app.clone(), ctx.session_id.clone());

    let library_ids = vec![ctx.library_id.clone()];
    let existing_titles = ctx
        .app
        .services
        .catalog
        .titles
        .list_for_libraries(Some(ctx.facet.clone()), &library_ids, None)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "library scan match worker failed to load existing titles: {error}"
            ))
        })?;
    let (by_name, by_smg, by_tvdb, by_imdb, by_tmdb) = match ctx.kind {
        LibraryScanPipelineKind::Movie => build_movie_title_indexes(&existing_titles),
        LibraryScanPipelineKind::Series => {
            let (by_name, by_tvdb, by_imdb, by_tmdb) = build_series_title_indexes(&existing_titles);
            (by_name, HashMap::new(), by_tvdb, by_imdb, by_tmdb)
        }
    };
    let mut state = ScanMatchWorkerState {
        existing_titles,
        existing_titles_by_name: by_name,
        existing_titles_by_smg_id: by_smg,
        existing_titles_by_tvdb_id: by_tvdb,
        existing_titles_by_imdb_id: by_imdb,
        existing_titles_by_tmdb_id: by_tmdb,
        search_results: MetadataSearchResults::new(),
        accounted_search_keys: HashSet::new(),
        report: ScanMatchWorkerReport {
            summary: LibraryScanSummary::default(),
            unmatched_items: Vec::new(),
            seen_paths: HashSet::new(),
            stats: MetadataLookupBatchStats::default(),
        },
    };

    let mut pending: Vec<QueuedMatchCandidate> = Vec::new();
    let mut ready_resolution: VecDeque<QueuedMatchCandidate> = VecDeque::new();
    let mut in_flight_keys: HashSet<BatchMetadataSearchKey> = HashSet::new();
    let mut search_set: tokio::task::JoinSet<(
        Vec<BatchMetadataSearchKey>,
        AppResult<MetadataSearchResults>,
    )> = tokio::task::JoinSet::new();
    let mut intake_open = true;
    // Set when every pending candidate is waiting on an in-flight key, so the
    // expired flush timer does not busy-loop until new state arrives.
    let mut flush_blocked = false;
    let mut candidates_intaken = 0usize;
    let mut metadata_batches_started = 0usize;
    let mut metadata_batches_finished = 0usize;

    loop {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            break;
        }
        if !intake_open
            && pending.is_empty()
            && ready_resolution.is_empty()
            && search_set.is_empty()
        {
            break;
        }

        let flush_deadline = if flush_blocked {
            None
        } else {
            pending.first().map(|queued| {
                tokio::time::Instant::from_std(queued.queued_at) + LIBRARY_SCAN_MATCH_FLUSH_INTERVAL
            })
        };

        tokio::select! {
            biased;
            _ = async {
                match cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            }, if cancel_token.is_some() => {
                break;
            }
            Some(joined) = search_set.join_next(), if !search_set.is_empty() => {
                let (chunk, result) = match joined {
                    Ok(entry) => entry,
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "library scan metadata batch task panicked: {error}"
                        )));
                    }
                };
                metadata_batches_finished = metadata_batches_finished.saturating_add(1);
                for key in &chunk {
                    in_flight_keys.remove(key);
                }
                flush_blocked = false;
                debug!(
                    facet = ctx.facet.as_str(),
                    batch_keys = chunk.len(),
                    exact_id_keys = chunk.iter().filter(|key| key.has_external_id()).count(),
                    fuzzy_keys = chunk.iter().filter(|key| !key.has_external_id()).count(),
                    pending = pending.len(),
                    in_flight_keys = in_flight_keys.len(),
                    search_tasks = search_set.len(),
                    metadata_batches_started,
                    metadata_batches_finished,
                    elapsed_ms = elapsed_ms_u64(worker_started_at),
                    "library scan match worker metadata batch completed"
                );
                match result {
                    Ok(results) => {
                        state.search_results.extend(results);
                        stage_ready_candidates(
                            &state.search_results,
                            &mut pending,
                            &mut ready_resolution,
                        )?;
                        debug!(
                            facet = ctx.facet.as_str(),
                            pending = pending.len(),
                            ready_resolution = ready_resolution.len(),
                            search_tasks = search_set.len(),
                            in_flight_keys = in_flight_keys.len(),
                            elapsed_ms = elapsed_ms_u64(worker_started_at),
                            "library scan match worker staged ready candidates"
                        );
                    }
                    Err(error) => {
                        // SMG batch failure: terminal metadata failure for every
                        // candidate keyed into that chunk; the scan continues.
                        fail_candidates_for_chunk(
                            &ctx,
                            &coordinator,
                            &mut state,
                            &mut pending,
                            &events,
                            &chunk,
                            &error,
                        )
                        .await?;
                    }
                }
            }
            maybe_candidate = input.recv(), if intake_open
                && pending.len() < LIBRARY_SCAN_MATCH_PENDING_HIGH_WATER => {
                match maybe_candidate {
                    Some((key, candidate)) => {
                        intake_candidate(
                            &ctx,
                            &coordinator,
                            &mut state,
                            &mut pending,
                            &events,
                            key,
                            candidate,
                        )
                        .await?;
                        candidates_intaken = candidates_intaken.saturating_add(1);
                        if candidates_intaken.is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL)
                        {
                            debug!(
                                facet = ctx.facet.as_str(),
                                candidates_intaken,
                                pending = pending.len(),
                                ready_resolution = ready_resolution.len(),
                                intake_open,
                                search_tasks = search_set.len(),
                                in_flight_keys = in_flight_keys.len(),
                                executed_requests = state.report.stats.executed_requests,
                                logical_lookups = state.report.stats.logical_lookups,
                                scanned = state.report.summary.scanned,
                                matched = state.report.summary.matched,
                                unmatched = state.report.summary.unmatched,
                                "library scan match worker intake diagnostic"
                            );
                        }
                        flush_blocked = false;
                        if pending.len() == 1 {
                            coordinator.publish_progress().await;
                        }
                    }
                    None => {
                        intake_open = false;
                        flush_blocked = false;
                    }
                }
            }
            _ = async {
                match flush_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            }, if flush_deadline.is_some() && search_set.len() < LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES => {
                // Timer flush below.
            }
            _ = std::future::ready(()), if !ready_resolution.is_empty() => {}
        }

        // Flush policy: full batch, timer expiry, or closed intake.
        while search_set.len() < LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES && !pending.is_empty() {
            if !state.search_results.is_empty() {
                stage_ready_candidates(&state.search_results, &mut pending, &mut ready_resolution)?;
            }
            if pending.is_empty() {
                break;
            }
            let timer_expired = pending.first().is_some_and(|queued| {
                queued.queued_at.elapsed() >= LIBRARY_SCAN_MATCH_FLUSH_INTERVAL
            });
            let size_ready = pending.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
            if !size_ready && !timer_expired && intake_open {
                break;
            }
            let flush_reason = if size_ready {
                "size"
            } else if timer_expired {
                "timer"
            } else {
                "intake_closed"
            };

            let chunk =
                next_pipeline_search_chunk(&state.search_results, &in_flight_keys, &pending)?;
            if chunk.is_empty() {
                // Everything pending is waiting on an in-flight key; disarm
                // the timer until results or new candidates arrive.
                flush_blocked = true;
                break;
            }
            for key in &chunk {
                in_flight_keys.insert(key.clone());
            }
            for key in &chunk {
                if state.accounted_search_keys.insert(key.clone()) {
                    state.report.stats.executed_requests =
                        state.report.stats.executed_requests.saturating_add(1);
                }
            }
            metadata_batches_started = metadata_batches_started.saturating_add(1);
            debug!(
                facet = ctx.facet.as_str(),
                batch_keys = chunk.len(),
                exact_id_keys = chunk.iter().filter(|key| key.has_external_id()).count(),
                fuzzy_keys = chunk.iter().filter(|key| !key.has_external_id()).count(),
                pending = pending.len(),
                ready_resolution = ready_resolution.len(),
                search_tasks = search_set.len(),
                in_flight_keys = in_flight_keys.len(),
                metadata_batches_started,
                metadata_batches_finished,
                flush_reason,
                elapsed_ms = elapsed_ms_u64(worker_started_at),
                "library scan match worker dispatching metadata batch"
            );
            let gateway = ctx.app.services.library.metadata_gateway.clone();
            let language = ctx.metadata_language.clone();
            let batch_cancel = cancel_token.clone();
            search_set.spawn(async move {
                let result = execute_batch_metadata_searches(
                    gateway,
                    chunk.clone(),
                    &language,
                    batch_cancel.as_ref(),
                )
                .await;
                (chunk, result)
            });
        }

        if !ready_resolution.is_empty() {
            resolve_ready_candidate_burst(
                &ctx,
                &coordinator,
                &mut state,
                &mut ready_resolution,
                &events,
                LIBRARY_SCAN_MATCH_RESOLUTION_BURST_SIZE,
            )
            .await?;
        }
    }

    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        coordinator.mark_metadata_total_known().await;
        coordinator.publish_progress().await;
    }

    let report = std::mem::replace(
        &mut state.report,
        ScanMatchWorkerReport {
            summary: LibraryScanSummary::default(),
            unmatched_items: Vec::new(),
            seen_paths: HashSet::new(),
            stats: MetadataLookupBatchStats::default(),
        },
    );
    events
        .send(ScanMatchWorkerEvent::Done(Box::new(report)))
        .map_err(|_| {
            AppError::Repository(
                "library scan match event receiver closed before final report".to_string(),
            )
        })?;
    Ok(())
}

async fn intake_candidate(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    pending: &mut Vec<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::UnboundedSender<ScanMatchWorkerEvent>,
    key: ScanCandidateKey,
    candidate: ScanPipelineCandidate,
) -> AppResult<()> {
    let candidate_started_at = Instant::now();
    state.report.summary.scanned += 1;
    let candidate_kind = match &candidate {
        ScanPipelineCandidate::Movie(_) => "movie",
        ScanPipelineCandidate::Series(_) => "series",
    };
    let candidate_name = candidate.diagnostic_name();
    let item_path = match &candidate {
        ScanPipelineCandidate::Movie(movie) => normalize_library_scan_item_path(&movie.file.path),
        ScanPipelineCandidate::Series(series) => series.item_path().trim().to_string(),
    };
    if !item_path.is_empty() {
        state.report.seen_paths.insert(item_path);
    }

    let mut sink = PipelineTitleWorkSink { staged: None };
    let unresolved = match candidate {
        ScanPipelineCandidate::Movie(movie) => process_movie_full_scan_candidate(
            &ctx.app,
            &ctx.actor,
            &ctx.facet,
            &ctx.library_id,
            &ctx.library_path,
            &ctx.session_id,
            coordinator,
            *movie,
            &mut sink,
            &mut state.existing_titles,
            &mut state.existing_titles_by_name,
            &mut state.existing_titles_by_tvdb_id,
            &mut state.existing_titles_by_imdb_id,
            &mut state.existing_titles_by_tmdb_id,
            &mut state.report.summary,
            &mut state.report.unmatched_items,
        )
        .await
        .map(|candidate| candidate.map(|c| ScanPipelineCandidate::Movie(Box::new(c)))),
        ScanPipelineCandidate::Series(series) => process_series_full_scan_candidate(
            &ctx.app,
            &ctx.actor,
            &ctx.facet,
            &ctx.library_id,
            &ctx.library_path,
            &ctx.session_id,
            coordinator,
            *series,
            &mut state.existing_titles,
            &mut state.existing_titles_by_name,
            &mut state.existing_titles_by_tvdb_id,
            &mut state.existing_titles_by_imdb_id,
            &mut state.existing_titles_by_tmdb_id,
            &mut sink,
            &mut state.report.summary,
            &mut state.report.unmatched_items,
        )
        .await
        .map(|candidate| candidate.map(|c| ScanPipelineCandidate::Series(Box::new(c)))),
    };

    let unresolved = match unresolved {
        Ok(unresolved) => unresolved,
        Err(error) => {
            warn!(error = %error, "library scan candidate processing failed");
            state.report.summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            if candidate_started_at.elapsed() >= LIBRARY_SCAN_DIAGNOSTIC_HEARTBEAT_INTERVAL {
                debug!(
                    facet = ctx.facet.as_str(),
                    key,
                    candidate_name = %candidate_name,
                    elapsed_ms = elapsed_ms_u64(candidate_started_at),
                    outcome = "error",
                    "library scan match worker slow candidate intake"
                );
            }
            return send_terminal(events, &mut sink, key);
        }
    };

    match unresolved {
        Some(candidate) => {
            // Register the SMG lookup for metadata progress before queueing.
            let mut search_key_count = 0usize;
            let mut exact_id_key_count = 0usize;
            let keys = candidate.batch_search_keys()?;
            if !keys.is_empty() {
                search_key_count = keys.len();
                exact_id_key_count = keys.iter().filter(|key| key.has_external_id()).count();
                state.report.stats.logical_lookups =
                    state.report.stats.logical_lookups.saturating_add(1);
                coordinator.add_metadata_total(1).await;
            }
            pending.push(QueuedMatchCandidate {
                key,
                candidate,
                queued_at: Instant::now(),
            });
            let elapsed_ms = elapsed_ms_u64(candidate_started_at);
            if elapsed_ms >= 500
                || (key as usize).is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL)
            {
                debug!(
                    facet = ctx.facet.as_str(),
                    key,
                    candidate_kind,
                    candidate_name = %candidate_name,
                    search_key_count,
                    exact_id_key_count,
                    pending = pending.len(),
                    elapsed_ms,
                    outcome = "queued_metadata",
                    "library scan match worker candidate intake diagnostic"
                );
            }
            Ok(())
        }
        None => {
            let elapsed_ms = elapsed_ms_u64(candidate_started_at);
            if elapsed_ms >= 500
                || (key as usize).is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL)
            {
                debug!(
                    facet = ctx.facet.as_str(),
                    key,
                    candidate_kind,
                    candidate_name = %candidate_name,
                    produced_work = sink.staged.is_some(),
                    elapsed_ms,
                    outcome = "terminal",
                    "library scan match worker candidate intake diagnostic"
                );
            }
            send_terminal(events, &mut sink, key)
        }
    }
}

fn send_terminal(
    events: &tokio::sync::mpsc::UnboundedSender<ScanMatchWorkerEvent>,
    sink: &mut PipelineTitleWorkSink,
    key: ScanCandidateKey,
) -> AppResult<()> {
    let event = match sink.staged.take() {
        Some(work) => ScanMatchWorkerEvent::Matched {
            key,
            work: Box::new(work),
        },
        None => ScanMatchWorkerEvent::Terminal { key },
    };
    events.send(event).map_err(|_| {
        AppError::Repository(
            "library scan match event receiver closed before candidate completion".to_string(),
        )
    })
}

fn next_pipeline_search_chunk(
    search_results: &MetadataSearchResults,
    in_flight_keys: &HashSet<BatchMetadataSearchKey>,
    pending: &[QueuedMatchCandidate],
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    let mut chunk = Vec::new();
    let mut seen = HashSet::new();

    for queued in pending {
        if chunk.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE {
            break;
        }
        let mut selected = None;
        for key in queued.candidate.batch_search_keys()? {
            // Resolved keys fall through to the candidate's next variant.
            if search_results.contains_key(&key) {
                continue;
            }
            // A key already dispatched in a previous in-flight batch means
            // the candidate waits for that result instead of eagerly
            // dispatching its fallback variants (no double-dispatch).
            if in_flight_keys.contains(&key) {
                break;
            }
            // A key another candidate already claimed within this chunk is
            // covered by this same batch; fall through to the next variant
            // so same-named siblings keep their distinguishing queries.
            if seen.contains(&key) {
                continue;
            }
            selected = Some(key);
            break;
        }
        if let Some(key) = selected {
            seen.insert(key.clone());
            chunk.push(key);
        }
    }

    Ok(chunk)
}

fn stage_ready_candidates(
    search_results: &MetadataSearchResults,
    pending: &mut Vec<QueuedMatchCandidate>,
    ready_resolution: &mut VecDeque<QueuedMatchCandidate>,
) -> AppResult<()> {
    let queued = std::mem::take(pending);
    let (ready, still_pending) = split_ready_metadata_candidates(
        queued,
        search_results,
        |queued: &QueuedMatchCandidate| queued.candidate.batch_search_keys(),
    )?;
    *pending = still_pending;
    ready_resolution.extend(ready);
    Ok(())
}

async fn resolve_ready_candidate_burst(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    ready_resolution: &mut VecDeque<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::UnboundedSender<ScanMatchWorkerEvent>,
    limit: usize,
) -> AppResult<()> {
    let ready_count = ready_resolution.len();
    let resolve_started_at = Instant::now();
    let mut resolved_count = 0usize;
    let mut matched_events = 0usize;
    let mut terminal_events = 0usize;
    let mut slow_resolved_count = 0usize;
    let mut max_candidate_elapsed_ms = 0u64;
    let mut max_candidate_key: Option<ScanCandidateKey> = None;
    let mut max_candidate_name: Option<String> = None;

    while resolved_count < limit {
        let Some(queued) = ready_resolution.pop_front() else {
            break;
        };
        let QueuedMatchCandidate { key, candidate, .. } = queued;
        let candidate_started_at = Instant::now();
        let candidate_kind = match &candidate {
            ScanPipelineCandidate::Movie(_) => "movie",
            ScanPipelineCandidate::Series(_) => "series",
        };
        let candidate_name = candidate.diagnostic_name();
        let mut sink = PipelineTitleWorkSink { staged: None };
        let result = match candidate {
            ScanPipelineCandidate::Movie(movie) => {
                process_resolved_movie_full_scan_candidate(
                    &ctx.app,
                    &ctx.actor,
                    &ctx.facet,
                    &ctx.library_id,
                    &ctx.library_path,
                    &ctx.session_id,
                    coordinator,
                    *movie,
                    &state.search_results,
                    &mut sink,
                    &mut state.existing_titles,
                    &mut state.existing_titles_by_name,
                    &mut state.existing_titles_by_smg_id,
                    &mut state.existing_titles_by_tvdb_id,
                    &mut state.existing_titles_by_imdb_id,
                    &mut state.existing_titles_by_tmdb_id,
                    &mut state.report.summary,
                    &mut state.report.unmatched_items,
                )
                .await
            }
            ScanPipelineCandidate::Series(series) => {
                process_resolved_series_full_scan_candidate(
                    &ctx.app,
                    &ctx.actor,
                    &ctx.facet,
                    &ctx.library_id,
                    &ctx.library_path,
                    &ctx.session_id,
                    coordinator,
                    *series,
                    &state.search_results,
                    &mut sink,
                    &mut state.existing_titles,
                    &mut state.existing_titles_by_name,
                    &mut state.existing_titles_by_tvdb_id,
                    &mut state.existing_titles_by_imdb_id,
                    &mut state.existing_titles_by_tmdb_id,
                    &mut state.report.summary,
                    &mut state.report.unmatched_items,
                )
                .await
            }
        };
        let candidate_elapsed_ms = elapsed_ms_u64(candidate_started_at);
        if candidate_elapsed_ms > max_candidate_elapsed_ms {
            max_candidate_elapsed_ms = candidate_elapsed_ms;
            max_candidate_key = Some(key);
            max_candidate_name = Some(candidate_name.clone());
        }
        if let Err(error) = result {
            warn!(error = %error, "library scan resolved candidate processing failed");
            state.report.summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
        }
        coordinator.mark_metadata_completed(1).await;
        let produced_work = sink.staged.is_some();
        if produced_work {
            matched_events = matched_events.saturating_add(1);
        } else {
            terminal_events = terminal_events.saturating_add(1);
        }
        send_terminal(events, &mut sink, key)?;
        resolved_count = resolved_count.saturating_add(1);
        if candidate_elapsed_ms >= 1000 {
            slow_resolved_count = slow_resolved_count.saturating_add(1);
            debug!(
                facet = ctx.facet.as_str(),
                key,
                candidate_kind,
                candidate_name = %candidate_name,
                produced_work,
                elapsed_ms = candidate_elapsed_ms,
                resolved_count,
                ready_count,
                remaining_ready = ready_resolution.len(),
                scanned = state.report.summary.scanned,
                matched = state.report.summary.matched,
                unmatched = state.report.summary.unmatched,
                "library scan match worker slow resolved candidate"
            );
        } else if resolved_count.is_multiple_of(LIBRARY_SCAN_DIAGNOSTIC_ITEM_INTERVAL) {
            debug!(
                facet = ctx.facet.as_str(),
                resolved_count,
                ready_count,
                remaining_ready = ready_resolution.len(),
                matched_events,
                terminal_events,
                max_candidate_elapsed_ms,
                max_candidate_key,
                max_candidate_name = max_candidate_name.as_deref(),
                elapsed_ms = elapsed_ms_u64(resolve_started_at),
                "library scan match worker resolve progress diagnostic"
            );
        }
    }
    coordinator.publish_progress().await;
    if ready_count > 0 {
        debug!(
            facet = ctx.facet.as_str(),
            resolved_count,
            initial_ready_count = ready_count,
            remaining_ready = ready_resolution.len(),
            matched_events,
            terminal_events,
            slow_resolved_count,
            max_candidate_elapsed_ms,
            max_candidate_key,
            max_candidate_name = max_candidate_name.as_deref(),
            elapsed_ms = elapsed_ms_u64(resolve_started_at),
            scanned = state.report.summary.scanned,
            matched = state.report.summary.matched,
            unmatched = state.report.summary.unmatched,
            "library scan match worker resolved candidate burst"
        );
    }
    Ok(())
}

async fn fail_candidates_for_chunk(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    pending: &mut Vec<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::UnboundedSender<ScanMatchWorkerEvent>,
    chunk: &[BatchMetadataSearchKey],
    error: &AppError,
) -> AppResult<()> {
    warn!(
        error = %error,
        chunk_size = chunk.len(),
        "library scan SMG match batch failed; failing affected candidates"
    );
    let chunk_keys: HashSet<&BatchMetadataSearchKey> = chunk.iter().collect();
    let queued = std::mem::take(pending);
    let mut failed = Vec::new();

    for queued_candidate in queued {
        let affected = queued_candidate
            .candidate
            .batch_search_keys()
            .map(|keys| keys.iter().any(|key| chunk_keys.contains(key)))
            .unwrap_or(true);
        if affected {
            failed.push(queued_candidate);
        } else {
            pending.push(queued_candidate);
        }
    }

    for queued_candidate in failed {
        let QueuedMatchCandidate { key, candidate, .. } = queued_candidate;
        let unmatched_item = match &candidate {
            ScanPipelineCandidate::Movie(movie) => build_movie_unmatched_scan_item(
                &ctx.facet,
                &ctx.library_id,
                &ctx.session_id,
                &ctx.library_path,
                movie,
                &state.search_results,
                Some("metadata_search_failed"),
                Some(error.to_string()),
            ),
            ScanPipelineCandidate::Series(series) => build_series_unmatched_scan_item(
                &ctx.facet,
                &ctx.library_id,
                &ctx.session_id,
                &ctx.library_path,
                series,
                &state.search_results,
                Some("metadata_search_failed"),
                Some(error.to_string()),
            ),
        };
        if let Err(persist_error) =
            persist_library_scan_unmatched_item(&ctx.app, &unmatched_item).await
        {
            warn!(
                error = %persist_error,
                "failed to persist unmatched item for failed SMG batch"
            );
        }
        state.report.unmatched_items.push(unmatched_item);
        state.report.summary.unmatched += 1;
        coordinator.mark_title_match_completed(1).await;
        coordinator.mark_metadata_failed(1).await;
        events
            .send(ScanMatchWorkerEvent::Terminal { key })
            .map_err(|_| {
                AppError::Repository(
                    "library scan match event receiver closed while reporting metadata failure"
                        .to_string(),
                )
            })?;
    }
    coordinator.publish_progress().await;
    Ok(())
}
