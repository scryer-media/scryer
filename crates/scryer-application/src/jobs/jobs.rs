use super::*;
use crate::discovery::{
    DiscoveryContextDefaults, DiscoveryLibraryContext, build_discovery_library_context,
    coalesce_pending_context_change, incremental_item_records,
    pending_context_change_from_domain_event, pending_context_changes_need_snapshot_reconciliation,
    public_feed_item_records, public_feed_section_records, snapshot_facet_records,
    snapshot_item_records,
};
use crate::domain_events::{DomainEventActor, new_job_run_domain_event};
use crate::event_views::replay_library_scan_state;
use crate::scheduler;
use chrono::{DateTime, Utc};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType,
    JobNextRunUpdatedEventData, JobRunCompletedEventData, JobRunFailedEventData,
    JobRunStartedEventData,
};
use scryer_logging::{ActorContext, LogContext, ResourceContext, WorkflowContext, context_span};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::time::UNIX_EPOCH;
use tokio::sync::broadcast;
use tracing::{Instrument, info, warn};

const BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS: i64 = 6 * 60 * 60;
const BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS: i64 = 15 * 60;
const DISCOVERY_SYNC_INCREMENTAL_CADENCE_SECONDS: i64 = 4 * 60 * 60;
const DISCOVERY_SYNC_LEASE_SECONDS: i64 = 30 * 60;
const DISCOVERY_SYNC_MANUAL_CONTEXT_COOLDOWN_SECONDS: i64 = 15 * 60;
const DISCOVERY_SYNC_DAILY_BACKSTOP_SECONDS: i64 = 24 * 60 * 60;
const DISCOVERY_SYNC_JITTER_WINDOW_SECONDS: i64 = 6 * 60 * 60;
const DISCOVERY_SYNC_ACCELERATED_DELAY_SECONDS: i64 = 60;
const DISCOVERY_SYNC_ACCELERATED_JITTER_WINDOW_SECONDS: i64 = 5 * 60;
const DISCOVERY_SYNC_BOOTSTRAP_JITTER_WINDOW_SECONDS: i64 = 10 * 60;
const DISCOVERY_SYNC_BOOTSTRAP_QUIET_SECONDS: i64 = 10 * 60;
const DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_BATCH_LIMIT: usize = 500;
const DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_MAX_BATCHES: usize = 20;
const DISCOVERY_DIRTY_REASON_TITLE_CHANGE: i64 = 1 << 0;
const DISCOVERY_DIRTY_REASON_SCAN_BOUNDARY: i64 = 1 << 1;
const DISCOVERY_PUBLIC_FEED_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub(crate) const SCHEDULER_INSTANCE_ID_KEY: &str = "scheduler.instance_id";

#[derive(Clone)]
enum JobExecutionPrincipal {
    User(User),
    System,
}

fn job_log_span(run: &JobRunRecord, actor: &User) -> tracing::Span {
    context_span(
        LogContext::workflow(WorkflowContext {
            kind: run.job_key.as_str().to_owned(),
            id: run.id.clone(),
        })
        .with_actor(ActorContext {
            kind: if actor.is_system_execution_actor() {
                "system".to_owned()
            } else {
                "user".to_owned()
            },
            id: Some(actor.id.clone()),
            display_name: Some(actor.username.clone()),
            source: None,
        })
        .with_resource(ResourceContext {
            job_id: Some(run.id.clone()),
            ..ResourceContext::default()
        }),
    )
}

impl JobExecutionPrincipal {
    fn into_actor(self) -> User {
        match self {
            Self::User(actor) => actor,
            Self::System => User::system_execution_actor(),
        }
    }
}

fn apply_library_scan_session_to_job_run(run: &mut JobRun, session: LibraryScanSession) {
    run.library_scan_progress = Some(session.clone());
    run.status = match session.status {
        LibraryScanStatus::Discovering => JobRunStatus::Discovering,
        LibraryScanStatus::Running => JobRunStatus::Running,
        LibraryScanStatus::Completed => JobRunStatus::Completed,
        LibraryScanStatus::Canceled => JobRunStatus::Warning,
        LibraryScanStatus::Warning => JobRunStatus::Warning,
        LibraryScanStatus::Failed => JobRunStatus::Failed,
    };
    if run.status.is_terminal() {
        run.completed_at = Some(session.updated_at);
    }
}

fn is_background_library_refresh_job(job_key: JobKey) -> bool {
    matches!(
        job_key,
        JobKey::BackgroundLibraryRefreshMovies
            | JobKey::BackgroundLibraryRefreshSeries
            | JobKey::BackgroundLibraryRefreshAnime
    )
}

fn library_job_operation_type(job_key: JobKey, library_id: &str) -> String {
    format!("{}:{library_id}", job_key.as_str())
}

fn job_run_library_id(run: &JobRunRecord) -> Option<&str> {
    run.operation_type
        .strip_prefix(run.job_key.as_str())
        .and_then(|value| value.strip_prefix(':'))
        .filter(|value| !value.trim().is_empty())
}

fn background_library_refresh_enabled() -> bool {
    std::env::var("SCRYER_BACKGROUND_LIBRARY_REFRESH")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

fn discovery_jitter_seconds(seed: &str, stream: &str, window_seconds: i64) -> i64 {
    scheduler::stable_jitter_offset(
        seed,
        "discovery_sync",
        stream,
        std::time::Duration::from_secs(window_seconds.max(1) as u64),
    )
    .as_secs() as i64
}

fn discovery_accelerated_at(now: DateTime<Utc>, scheduler_seed: &str) -> DateTime<Utc> {
    now + chrono::Duration::seconds(
        DISCOVERY_SYNC_ACCELERATED_DELAY_SECONDS
            + discovery_jitter_seconds(
                scheduler_seed,
                "first_personalized_snapshot",
                DISCOVERY_SYNC_ACCELERATED_JITTER_WINDOW_SECONDS,
            ),
    )
}

fn discovery_status_is(actual: &str, expected: &str) -> bool {
    actual.trim().eq_ignore_ascii_case(expected)
}

fn discovery_snapshot_status_is_polling(status: &str) -> bool {
    discovery_status_is(status, "ACCEPTED")
        || discovery_status_is(status, "RUNNING")
        || discovery_status_is(status, "BUILDING")
}

fn discovery_snapshot_status_is_terminal(status: &str) -> bool {
    discovery_status_is(status, "FAILED")
        || discovery_status_is(status, "CANCELED")
        || discovery_status_is(status, "EXPIRED")
}

fn discovery_retry_after(now: DateTime<Utc>, retry_after_seconds: i32) -> DateTime<Utc> {
    let seconds = i64::from(retry_after_seconds).clamp(5 * 60, 6 * 60 * 60);
    now + chrono::Duration::seconds(seconds)
}

fn discovery_transient_retry_after(
    state: &mut DiscoverySyncStateRecord,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    state.transient_failure_count = state.transient_failure_count.saturating_add(1);
    let seconds = match state.transient_failure_count {
        0 | 1 => 15 * 60,
        2 => 60 * 60,
        _ => 6 * 60 * 60,
    };
    now + chrono::Duration::seconds(seconds)
}

fn discovery_reset_transient_failure_count(state: &mut DiscoverySyncStateRecord) {
    state.transient_failure_count = 0;
}

fn discovery_schedule_context_snapshot_retry(
    state: &mut DiscoverySyncStateRecord,
    retry_at: DateTime<Utc>,
) {
    state.backoff_until = Some(retry_at);
    state.next_context_snapshot_eligible_at = Some(retry_at);
}

fn discovery_schedule_incremental_retry(
    state: &mut DiscoverySyncStateRecord,
    retry_at: DateTime<Utc>,
) {
    state.backoff_until = Some(retry_at);
    state.next_incremental_reload_eligible_at = Some(retry_at);
}

fn discovery_schedule_public_feed_retry(
    state: &mut DiscoverySyncStateRecord,
    retry_at: DateTime<Utc>,
) {
    state.backoff_until = Some(retry_at);
    state.next_public_feed_eligible_at = Some(retry_at);
}

fn discovery_prefer_earlier_gate(
    gate: &mut Option<DateTime<Utc>>,
    candidate: DateTime<Utc>,
) -> bool {
    if gate.is_none_or(|existing| candidate < existing) {
        *gate = Some(candidate);
        return true;
    }
    false
}

struct DiscoveryNextRunCandidates {
    next_incremental: DateTime<Utc>,
    /// An incremental reload only ever runs against an existing personalized
    /// snapshot (`incremental_due` requires `last_success_generation_id`), so
    /// its cadence bucket is not a reason to wake before the first snapshot
    /// has landed. Waking for it anyway was pure waste — and, because the
    /// bucket is seed-jittered, it could land inside the first-snapshot
    /// window and pre-empt the wake the state actually scheduled.
    incremental_reload_possible: bool,
    next_context: DateTime<Utc>,
    next_public: DateTime<Utc>,
    bootstrap_quiet_until: Option<DateTime<Utc>>,
    backoff_until: Option<DateTime<Utc>>,
    scan_blocked_retry_at: Option<DateTime<Utc>>,
    pending_changes_quiet_at: Option<DateTime<Utc>>,
}

fn discovery_next_run_at(
    now: DateTime<Utc>,
    candidates: DiscoveryNextRunCandidates,
) -> DateTime<Utc> {
    let DiscoveryNextRunCandidates {
        next_incremental,
        incremental_reload_possible,
        next_context,
        next_public,
        bootstrap_quiet_until,
        backoff_until,
        scan_blocked_retry_at,
        pending_changes_quiet_at,
    } = candidates;
    [
        incremental_reload_possible.then_some(next_incremental),
        Some(next_context),
        Some(next_public),
        bootstrap_quiet_until,
        backoff_until,
        scan_blocked_retry_at,
        pending_changes_quiet_at,
    ]
    .into_iter()
    .flatten()
    .filter(|candidate| *candidate >= now)
    .min()
    .unwrap_or(next_incremental)
}

fn discovery_pending_changes_quiet_at(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> Option<DateTime<Utc>> {
    pending_changes
        .iter()
        .map(|change| change.last_seen_at)
        .max()
        .map(|last_seen| {
            last_seen + chrono::Duration::seconds(DISCOVERY_SYNC_BOOTSTRAP_QUIET_SECONDS)
        })
}

fn discovery_pending_changes_are_quiet(
    now: DateTime<Utc>,
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> bool {
    pending_changes
        .iter()
        .map(|change| change.last_seen_at)
        .max()
        .is_none_or(|last_seen| {
            now >= last_seen + chrono::Duration::seconds(DISCOVERY_SYNC_BOOTSTRAP_QUIET_SECONDS)
        })
}

fn mark_discovery_context_dirty(state: &mut DiscoverySyncStateRecord, occurred_at: DateTime<Utc>) {
    state.dirty_since = Some(
        state
            .dirty_since
            .map(|existing| existing.min(occurred_at))
            .unwrap_or(occurred_at),
    );
    state.dirty_reason_mask |= DISCOVERY_DIRTY_REASON_TITLE_CHANGE;
}

fn extend_discovery_bootstrap_quiet_window(
    state: &mut DiscoverySyncStateRecord,
    occurred_at: DateTime<Utc>,
) {
    if state.bootstrap_started_at.is_none() {
        state.bootstrap_started_at = Some(occurred_at);
    }
    let quiet_until = occurred_at
        + chrono::Duration::seconds(
            DISCOVERY_SYNC_BOOTSTRAP_QUIET_SECONDS + state.startup_jitter_seconds,
        );
    state.bootstrap_quiet_until = Some(
        state
            .bootstrap_quiet_until
            .map(|existing| existing.max(quiet_until))
            .unwrap_or(quiet_until),
    );
}

fn discovery_scan_boundary_event(event: &DomainEvent) -> bool {
    matches!(
        &event.payload,
        DomainEventPayload::LibraryScanCompleted(_)
            | DomainEventPayload::LibraryScanCanceled(_)
            | DomainEventPayload::LibraryScanFailed(_)
    )
}

fn discovery_context_dirty_event_types() -> Vec<DomainEventType> {
    vec![
        DomainEventType::TitleAdded,
        DomainEventType::TitleUpdated,
        DomainEventType::TitleRematched,
        DomainEventType::TitleDeleted,
        DomainEventType::LibraryScanCompleted,
        DomainEventType::LibraryScanCanceled,
        DomainEventType::LibraryScanFailed,
    ]
}

fn discovery_scan_projection_event_types() -> Vec<DomainEventType> {
    vec![
        DomainEventType::LibraryScanStarted,
        DomainEventType::LibraryScanProgressed,
        DomainEventType::LibraryScanCompleted,
        DomainEventType::LibraryScanCanceled,
        DomainEventType::LibraryScanFailed,
    ]
}

fn non_empty_discovery_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn next_hash_jittered_bucket(
    now: chrono::DateTime<Utc>,
    jitter_seconds: i64,
) -> chrono::DateTime<Utc> {
    let cadence = DISCOVERY_SYNC_INCREMENTAL_CADENCE_SECONDS;
    let now_system_time =
        UNIX_EPOCH + std::time::Duration::from_secs(now.timestamp().max(0) as u64);
    let delay = scheduler::next_jittered_cycle_delay(
        now_system_time,
        std::time::Duration::from_secs(cadence as u64),
        std::time::Duration::from_secs(jitter_seconds.clamp(0, cadence - 1) as u64),
        std::time::Duration::ZERO,
    );
    let next_seconds = now.timestamp().saturating_add(delay.as_secs() as i64);
    chrono::DateTime::from_timestamp(next_seconds, 0)
        .unwrap_or_else(|| now + chrono::Duration::seconds(cadence))
}

#[derive(Clone, Debug, serde::Serialize)]
struct HealthChecksSummary {
    total: usize,
    errors: usize,
    warnings: usize,
    checks: Vec<HealthCheckSummaryItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HealthCheckSummaryItem {
    source: String,
    status: String,
    message: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CountSummary {
    count: u32,
}

const PLUGIN_CATALOG_REFRESHED_MESSAGE: &str = "Plugin catalog refreshed";

/// Outcome for the plugin catalog refresh job. The message and summary stay
/// exactly as they were whenever automatic plugin updates did nothing, so the
/// default (opt-out) configuration is indistinguishable from before.
fn plugin_registry_refresh_outcome(
    report: crate::plugins::runtime::PluginAutoUpdateReport,
) -> JobExecutionOutcome {
    if !report.did_work() {
        return JobExecutionOutcome::new(Some(PLUGIN_CATALOG_REFRESHED_MESSAGE.to_string()), None);
    }

    let failed_count = report.failed.len() + usize::from(report.error.is_some());
    let mut message = format!(
        "{PLUGIN_CATALOG_REFRESHED_MESSAGE}; auto-updated {} plugin(s)",
        report.updated.len()
    );
    if failed_count > 0 {
        message.push_str(&format!("; {failed_count} failed"));
    }

    let summary_json = serde_json::to_string(&json!({
        "updated": report.updated,
        "failed": report.failed,
        "error": report.error,
    }))
    .ok();

    if report.has_failures() {
        JobExecutionOutcome::warning(Some(message), summary_json)
    } else {
        JobExecutionOutcome::new(Some(message), summary_json)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
struct LibraryScanRunSummary {
    scanned: usize,
    matched: usize,
    imported: usize,
    skipped: usize,
    unmatched: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct RssSyncRunSummary {
    releases_fetched: usize,
    releases_matched: usize,
    releases_grabbed: usize,
    releases_held: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HousekeepingRunSummary {
    orphaned_media_files: u32,
    stale_release_decisions: u32,
    stale_release_attempts: u32,
    stale_indexer_errors: u32,
    stale_history_events: u32,
    stale_history_records: u32,
    staged_nzb_artifacts_pruned: u32,
    recycled_purged: u32,
    recycled_pending_reconciled: u32,
    discovery_pruned_runs: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DiscoverySyncRunSummary {
    state_created: bool,
    trigger_source: String,
    subject_count: usize,
    subject_fingerprint: String,
    subject_context_changed: bool,
    ack_recovery: Option<DiscoveryAckRecoveryRunSummary>,
    context_snapshot: Option<DiscoveryContextSnapshotRunSummary>,
    context_incremental: Option<DiscoveryContextIncrementalRunSummary>,
    public_feed: Option<DiscoveryPublicFeedRunSummary>,
    next_run_at: String,
    next_incremental_reload_eligible_at: String,
    next_context_snapshot_eligible_at: String,
    next_public_feed_eligible_at: String,
    startup_jitter_seconds: i64,
    context_jitter_seconds: i64,
    incremental_reload_jitter_seconds: i64,
    public_feed_jitter_seconds: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DiscoveryAckRecoveryRunSummary {
    attempted: i64,
    acknowledged: i64,
    failed_run_id: Option<String>,
    next_retry_at: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DiscoveryContextSnapshotRunSummary {
    run_id: String,
    committed: bool,
    smg_request_id: Option<String>,
    smg_status: Option<String>,
    page_count: i32,
    item_count: i64,
    facet_count: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DiscoveryContextIncrementalRunSummary {
    run_id: String,
    committed: bool,
    smg_status: Option<String>,
    changed_subject_count: i64,
    affected_target_count: i64,
    item_count: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DiscoveryPublicFeedRunSummary {
    run_id: String,
    committed: bool,
    section_count: i64,
    item_count: i64,
}

#[derive(Clone, Debug)]
struct JobExecutionOutcome {
    summary_text: Option<String>,
    summary_json: Option<String>,
    library_scan_progress: Option<LibraryScanSession>,
    status_override: Option<JobRunStatus>,
    auto_backup_outcome: Option<crate::security::backup::AutoBackupRunOutcome>,
}

impl JobExecutionOutcome {
    fn new(summary_text: Option<String>, summary_json: Option<String>) -> Self {
        Self {
            summary_text,
            summary_json,
            library_scan_progress: None,
            status_override: None,
            auto_backup_outcome: None,
        }
    }

    fn warning(summary_text: Option<String>, summary_json: Option<String>) -> Self {
        Self {
            summary_text,
            summary_json,
            library_scan_progress: None,
            status_override: Some(JobRunStatus::Warning),
            auto_backup_outcome: None,
        }
    }

    fn with_auto_backup_outcome(
        mut self,
        outcome: crate::security::backup::AutoBackupRunOutcome,
    ) -> Self {
        self.auto_backup_outcome = Some(outcome);
        self
    }

    fn from_library_scan(summary: &LibraryScanSummary) -> Self {
        Self::new(
            Some(summary_text_from_library_scan(summary)),
            serde_json::to_string(&LibraryScanRunSummary {
                scanned: summary.scanned,
                matched: summary.matched,
                imported: summary.imported,
                skipped: summary.skipped,
                unmatched: summary.unmatched,
            })
            .ok(),
        )
    }
}

impl AppUseCase {
    /// Boot-time reconciliation of persisted job runs orphaned by a restart.
    ///
    /// Invariant: a persisted running run is only advanced by its in-memory
    /// worker; once the process restarts that worker is gone and the run is
    /// unfinishable. Fail those rows in the store so pollers (the jobs UI, the
    /// GraphQL `acquisitionSearchJob` view, the e2e suite) stop waiting on a
    /// run that can never complete. The in-memory tracker starts empty on a
    /// fresh boot, so there is normally nothing to clear there — but any active
    /// run the tracker still holds that the store just failed is a ghost, so we
    /// push it through `upsert_active_run` (which drops terminal runs from the
    /// active registry) to evict it. Returns the number of runs reconciled.
    pub async fn reconcile_interrupted_job_runs(
        &self,
        excluded_run_ids: &[String],
    ) -> AppResult<u64> {
        let reconciled = self
            .services
            .events
            .job_runs
            .reconcile_interrupted_job_runs(excluded_run_ids)
            .await?;
        if reconciled == 0 {
            return Ok(0);
        }
        warn!(
            reconciled,
            "failed interrupted job runs left running by a previous process"
        );
        // Evict any tracker entry the store no longer considers active.
        for mut run in self.runtime.jobs.job_run_tracker.list_active().await {
            if !run.status.is_terminal() && !excluded_run_ids.contains(&run.id) {
                run.status = JobRunStatus::Failed;
                run.completed_at = Some(Utc::now());
                self.runtime
                    .jobs
                    .job_run_tracker
                    .upsert_active_run(run)
                    .await;
            }
        }
        Ok(reconciled)
    }

    async fn load_active_job_runs_for_listing(&self) -> AppResult<Vec<JobRun>> {
        let tracker_runs = self.runtime.jobs.job_run_tracker.list_active().await;
        if !tracker_runs.is_empty() {
            return Ok(self.attach_active_library_scan_sessions(tracker_runs).await);
        }

        let scan_sessions = self.active_library_scan_sessions_by_id().await;
        let mut runs = self
            .services
            .events
            .job_runs
            .list_active_job_runs()
            .await?
            .into_iter()
            .map(|record| JobRun::from_record(&record, scan_sessions.get(&record.id).cloned()))
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.started_at);
        Ok(runs)
    }

    async fn attach_active_library_scan_sessions(&self, mut runs: Vec<JobRun>) -> Vec<JobRun> {
        let scan_sessions = self.active_library_scan_sessions_by_id().await;
        if scan_sessions.is_empty() {
            return runs;
        }
        for run in &mut runs {
            if let Some(session) = scan_sessions.get(&run.id) {
                apply_library_scan_session_to_job_run(run, session.clone());
            }
        }
        runs
    }

    async fn active_library_scan_sessions_by_id(&self) -> HashMap<String, LibraryScanSession> {
        self.runtime
            .library
            .library_scan_tracker
            .list_active()
            .await
            .into_iter()
            .map(|session| (session.session_id.clone(), session))
            .collect()
    }

    async fn active_library_scan_run_count(&self) -> AppResult<usize> {
        let runs = self.runtime.jobs.job_run_tracker.list_active().await;
        let tracker_scan_count = runs
            .iter()
            .filter(|run| job_key_library_facet(run.job_key).is_some())
            .count();
        if tracker_scan_count > 0 {
            return Ok(tracker_scan_count);
        }

        let runtime_scan_count = self
            .active_library_scan_sessions()
            .await
            .iter()
            .filter(|session| !session.status.is_terminal())
            .count();
        if runtime_scan_count > 0 {
            return Ok(runtime_scan_count);
        }

        let mut events = Vec::new();
        let mut after_sequence = 0i64;
        let event_types = discovery_scan_projection_event_types();
        loop {
            let batch = self
                .services
                .events
                .domain_events
                .list(&DomainEventFilter {
                    after_sequence: Some(after_sequence),
                    event_types: Some(event_types.clone()),
                    limit: DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_BATCH_LIMIT,
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
            if count < DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_BATCH_LIMIT {
                break;
            }
        }

        Ok(replay_library_scan_state(&events)
            .values()
            .filter(|session| !session.status.is_terminal())
            .count())
    }

    pub async fn list_jobs(&self, actor: &User) -> AppResult<Vec<JobDefinition>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let next_runs = self.runtime.jobs.job_run_tracker.all_next_runs().await;
        Ok(crate::jobs::all_job_definitions(&next_runs))
    }

    async fn actor_can_view_interactive_job_run(
        &self,
        actor: &User,
        run: &JobRun,
    ) -> AppResult<bool> {
        if run.actor_user_id.as_deref() != Some(actor.id.as_str()) {
            return Ok(false);
        }
        match run.job_key {
            JobKey::TitleDeletion | JobKey::TitleRename => Ok(true),
            JobKey::MediaFileDeletion | JobKey::RecycleBinRestore | JobKey::RecycleBinPurge => {
                let Some((prefix, remainder)) = run.operation_type.split_once(':') else {
                    return Ok(false);
                };
                let library_ids = match (run.job_key, prefix) {
                    (JobKey::MediaFileDeletion, "media_file_deletion")
                    | (JobKey::RecycleBinRestore, "recycle_bin_restore") => {
                        let Some((library_id, _resource_id)) = remainder.split_once(':') else {
                            // Legacy rows did not persist a library scope and must not
                            // remain visible after the original grant is gone.
                            return Ok(false);
                        };
                        if library_id.is_empty() {
                            return Ok(false);
                        }
                        vec![library_id]
                    }
                    (JobKey::RecycleBinRestore, "recycle_bin_restore_batch")
                    | (JobKey::RecycleBinPurge, "recycle_bin_purge_batch") => {
                        let Some((library_ids, batch_size)) = remainder.rsplit_once(':') else {
                            return Ok(false);
                        };
                        if batch_size
                            .parse::<usize>()
                            .ok()
                            .filter(|size| *size > 0)
                            .is_none()
                        {
                            return Ok(false);
                        }
                        let library_ids = library_ids
                            .split(',')
                            .filter(|library_id| !library_id.is_empty())
                            .collect::<Vec<_>>();
                        if library_ids.is_empty() {
                            return Ok(false);
                        }
                        library_ids
                    }
                    _ => return Ok(false),
                };
                for library_id in library_ids {
                    match self
                        .require_library_permission(
                            actor,
                            library_id,
                            scryer_domain::LibraryPermission::ManageTitles,
                        )
                        .await
                    {
                        Ok(()) => {}
                        Err(AppError::Unauthorized(_)) => return Ok(false),
                        Err(error) => return Err(error),
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn filter_interactive_job_runs_for_actor(
        &self,
        actor: &User,
        runs: Vec<JobRun>,
    ) -> AppResult<Vec<JobRun>> {
        let mut visible = Vec::with_capacity(runs.len());
        for run in runs {
            if self.actor_can_view_interactive_job_run(actor, &run).await? {
                visible.push(run);
            }
        }
        Ok(visible)
    }

    pub async fn active_job_runs(&self, actor: &User) -> AppResult<Vec<JobRun>> {
        let runs = self.load_active_job_runs_for_listing().await?;
        if self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
        {
            return Ok(runs);
        }
        self.filter_interactive_job_runs_for_actor(actor, runs)
            .await
    }

    pub async fn list_job_runs(
        &self,
        actor: &User,
        job_key: JobKey,
        limit: usize,
    ) -> AppResult<Vec<JobRun>> {
        let can_manage_system = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        if !can_manage_system
            && !matches!(
                job_key,
                JobKey::TitleDeletion
                    | JobKey::TitleRename
                    | JobKey::MediaFileDeletion
                    | JobKey::RecycleBinRestore
                    | JobKey::RecycleBinPurge
            )
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ));
        }
        let active_runs = self.runtime.jobs.job_run_tracker.list_active().await;
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = if can_manage_system {
            self.services
                .events
                .job_runs
                .list_job_runs(Some(job_key), limit.max(1))
                .await?
        } else {
            self.services
                .events
                .job_runs
                .list_job_runs_for_actor(Some(job_key), &actor.id, limit.max(1))
                .await?
        };

        let runs = records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect::<Vec<_>>();
        if can_manage_system {
            Ok(runs)
        } else {
            self.filter_interactive_job_runs_for_actor(actor, runs)
                .await
        }
    }

    pub async fn list_recent_job_runs(&self, actor: &User, limit: usize) -> AppResult<Vec<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let active_runs = self.runtime.jobs.job_run_tracker.list_active().await;
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = self
            .services
            .events
            .job_runs
            .list_job_runs(None, limit.max(1))
            .await?;

        Ok(records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    pub async fn discovery_sync_status(&self, actor: &User) -> AppResult<DiscoverySyncStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let recent_runs = self
            .services
            .library
            .discovery
            .list_recent_discovery_sync_runs(10)
            .await?;
        let pending_context_change_count = self
            .services
            .library
            .discovery
            .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?;

        Ok(DiscoverySyncStatus {
            state,
            recent_runs,
            pending_context_change_count,
        })
    }

    pub async fn subscribe_job_run_events(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<JobRun>> {
        let can_manage_system = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let mut receiver = app.runtime.jobs.job_run_tracker.subscribe();
            let initial_runs = match app.active_job_runs(&actor).await {
                Ok(runs) => runs,
                Err(error) => {
                    tracing::warn!("job run subscription initial load failed: {error}");
                    return;
                }
            };
            for run in initial_runs {
                if tx.send(run).is_err() {
                    return;
                }
            }

            loop {
                match receiver.recv().await {
                    Ok(run) => {
                        if !can_manage_system {
                            match app.actor_can_view_interactive_job_run(&actor, &run).await {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(error) => {
                                    tracing::warn!(
                                        "job run subscription authorization failed: {error}"
                                    );
                                    continue;
                                }
                            }
                        }
                        if tx.send(run).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("job run subscription lagged, skipped {n} tracker updates");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub async fn trigger_job(&self, actor: &User, job_key: JobKey) -> AppResult<JobRun> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        if !job_key.manual_trigger_allowed() {
            return Err(AppError::Validation(format!(
                "{} can only run on its configured schedule",
                job_key.display_name()
            )));
        }
        self.ensure_job_can_start(job_key, None).await?;

        let run = self
            .create_job_run_record(
                job_key,
                JobTriggerSource::Manual,
                Some(actor.id.clone()),
                None,
            )
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        let event_actor = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                event_actor.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let log_span = job_log_span(&run, actor);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(
            async move {
                if let Err(error) = app
                    .run_job_run(run, JobExecutionPrincipal::User(actor), event_actor)
                    .await
                {
                    warn!(job_key = job_key.as_str(), error = %error, "manual job trigger failed");
                }
            }
            .instrument(log_span),
        );

        Ok(run_payload)
    }

    pub async fn run_scheduled_job_now(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        if is_background_library_refresh_job(job_key) {
            return self
                .run_scheduled_background_library_refresh_jobs_now(job_key, trigger_source)
                .await;
        }

        self.ensure_job_can_start(job_key, None).await?;
        let run = self
            .create_job_run_record(job_key, trigger_source, None, None)
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload)
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;
        let system_actor = User::system_execution_actor();
        self.run_job_run(
            run.clone(),
            JobExecutionPrincipal::System,
            DomainEventActor::system(),
        )
        .instrument(job_log_span(&run, &system_actor))
        .await
    }

    pub async fn run_scheduled_auto_backup_job_now(
        &self,
        trigger_source: JobTriggerSource,
    ) -> AppResult<crate::security::backup::AutoBackupRunOutcome> {
        let job_key = JobKey::AutoBackup;
        self.ensure_job_can_start(job_key, None).await?;
        let run = self
            .create_job_run_record(job_key, trigger_source, None, None)
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload)
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;
        let system_actor = User::system_execution_actor();
        self.run_job_run_with_auto_backup_outcome(run.clone(), None, DomainEventActor::system())
            .instrument(job_log_span(&run, &system_actor))
            .await?
            .ok_or_else(|| {
                AppError::Repository("auto backup job did not return an outcome".to_string())
            })
    }

    async fn run_scheduled_background_library_refresh_jobs_now(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        let facet = job_key_library_facet(job_key).expect("background refresh facet");
        let libraries = self.services.catalog.libraries.list(Some(facet)).await?;
        if libraries.is_empty() {
            return Err(AppError::Validation(format!(
                "{} has no libraries to refresh",
                job_key.display_name()
            )));
        }

        let mut first_error = None;
        for library in libraries {
            if let Err(error) = self.ensure_job_can_start(job_key, Some(&library.id)).await {
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
                continue;
            }

            let run = self
                .create_job_run_record(
                    job_key,
                    trigger_source,
                    None,
                    Some(library_job_operation_type(job_key, &library.id)),
                )
                .await?;
            let run_payload = JobRun::from_record(&run, None);
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(run_payload)
                .await;
            let _ = self
                .append_domain_event(new_job_run_domain_event(
                    None,
                    run.id.clone(),
                    DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().to_string(),
                        operation_type: run.operation_type.clone(),
                        trigger_source: run.trigger_source.as_str().to_string(),
                    }),
                ))
                .await;

            if let Err(error) = self
                .run_job_run(
                    run,
                    JobExecutionPrincipal::System,
                    DomainEventActor::system(),
                )
                .await
                && first_error.is_none()
            {
                first_error = Some(error.to_string());
            }
        }

        if let Some(error) = first_error {
            Err(AppError::Validation(error))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn schedule_discovery_sync_soon_silent(
        &self,
        reason: &str,
    ) -> AppResult<DateTime<Utc>> {
        let now = self.runtime.environment.now();
        let scheduler_seed = self.discovery_scheduler_seed().await?;
        let next_run_at = discovery_accelerated_at(now, &scheduler_seed);
        let existing = self
            .runtime
            .jobs
            .job_run_tracker
            .next_run_at(JobKey::DiscoverySync)
            .await;
        if existing.is_none_or(|existing| next_run_at < existing) {
            info!(
                reason,
                next_run_at = %next_run_at,
                "accelerating discovery sync"
            );
            self.runtime
                .jobs
                .job_run_tracker
                .set_next_run_at(JobKey::DiscoverySync, next_run_at)
                .await;
            self.runtime.jobs.discovery_sync_wake.notify_waiters();
        } else {
            self.runtime.jobs.discovery_sync_wake.notify_waiters();
        }
        Ok(next_run_at)
    }

    pub async fn set_job_next_run_at(&self, job_key: JobKey, next_run_at: chrono::DateTime<Utc>) {
        self.runtime
            .jobs
            .job_run_tracker
            .set_next_run_at(job_key, next_run_at)
            .await;
        if job_key == JobKey::DiscoverySync {
            self.runtime.jobs.discovery_sync_wake.notify_waiters();
        }
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                job_key.as_str().to_string(),
                DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
                    job_key: job_key.as_str().to_string(),
                    next_run_at: Some(next_run_at.to_rfc3339()),
                }),
            ))
            .await;
    }

    pub(crate) async fn refresh_public_discovery_feed_now(
        &self,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        let now = self.runtime.environment.now();
        let mut state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        state.next_public_feed_eligible_at = Some(now);
        state.updated_at = now;
        self.services
            .library
            .discovery
            .upsert_discovery_sync_state(&state)
            .await?;
        self.set_job_next_run_at(JobKey::DiscoverySync, now).await;
        self.run_scheduled_job_now(JobKey::DiscoverySync, trigger_source)
            .await
    }

    pub async fn clear_job_next_run_at(&self, job_key: JobKey) {
        self.runtime
            .jobs
            .job_run_tracker
            .clear_next_run_at(job_key)
            .await;
        if job_key == JobKey::DiscoverySync {
            self.runtime.jobs.discovery_sync_wake.notify_waiters();
        }
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                job_key.as_str().to_string(),
                DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
                    job_key: job_key.as_str().to_string(),
                    next_run_at: None,
                }),
            ))
            .await;
    }

    async fn ensure_job_can_start(
        &self,
        job_key: JobKey,
        library_id: Option<&str>,
    ) -> AppResult<()> {
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(job_key)
            .await
        {
            return Err(AppError::Validation(format!(
                "{} is already running",
                job_key.display_name()
            )));
        }

        if let Some(facet) = job_key_library_facet(job_key) {
            let library_id = match library_id {
                Some(library_id) => library_id.to_string(),
                None => self
                    .services
                    .catalog
                    .libraries
                    .default_for_facet(facet.clone())
                    .await?
                    .map(|library| library.id)
                    .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet)),
            };
            if self
                .runtime
                .library
                .library_scan_tracker
                .has_conflicting_session(&facet, Some(&library_id))
                .await
            {
                return Err(AppError::Validation(format!(
                    "{} library scan is already running",
                    facet.as_str()
                )));
            }
        }

        Ok(())
    }

    async fn create_job_run_record(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
        actor_user_id: Option<String>,
        operation_type: Option<String>,
    ) -> AppResult<JobRunRecord> {
        let now = Utc::now();
        let initial_status = if job_key.uses_library_scan_progress() {
            JobRunStatus::Discovering
        } else {
            JobRunStatus::Running
        };

        self.services
            .events
            .job_runs
            .create_job_run(&JobRunRecord {
                id: Id::new().0,
                job_key,
                operation_type: operation_type.unwrap_or_else(|| job_key.as_str().to_string()),
                status: initial_status,
                trigger_source,
                actor_user_id,
                progress_json: Some(json!({ "status": initial_status.as_str() }).to_string()),
                summary_json: None,
                summary_text: None,
                error_text: None,
                started_at: now,
                completed_at: None,
                created_at: now,
                updated_at: now,
            })
            .await
    }

    async fn run_job_run(
        &self,
        run: JobRunRecord,
        actor: JobExecutionPrincipal,
        event_actor: DomainEventActor,
    ) -> AppResult<()> {
        self.run_job_run_with_auto_backup_outcome(run, Some(actor.into_actor()), event_actor)
            .await
            .map(|_| ())
    }

    async fn run_job_run_with_auto_backup_outcome(
        &self,
        run: JobRunRecord,
        actor: Option<User>,
        event_actor: DomainEventActor,
    ) -> AppResult<Option<crate::security::backup::AutoBackupRunOutcome>> {
        match self.execute_job_body(&run, actor).await {
            Ok(outcome) => {
                let auto_backup_outcome = outcome.auto_backup_outcome.clone();
                self.finish_job_run(
                    run,
                    event_actor,
                    outcome.summary_text,
                    outcome.summary_json,
                    outcome.library_scan_progress,
                    outcome.status_override,
                )
                .await?;
                Ok(auto_backup_outcome)
            }
            Err(error) => {
                self.fail_job_run(run, event_actor, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    async fn execute_job_body(
        &self,
        run: &JobRunRecord,
        actor: Option<User>,
    ) -> AppResult<JobExecutionOutcome> {
        let job_key = run.job_key;
        let run_id = run.id.as_str();
        let actor = actor.unwrap_or_else(User::system_execution_actor);
        match job_key {
            JobKey::LibraryScanMovies | JobKey::LibraryScanSeries | JobKey::LibraryScanAnime => {
                let facet = job_key_library_facet(job_key).expect("library scan facet");
                let summary = self
                    .scan_library_with_tracking(
                        &actor,
                        facet,
                        Some(run_id.to_string()),
                        LibraryScanMode::Full,
                    )
                    .await?;
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::BackgroundLibraryRefreshMovies
            | JobKey::BackgroundLibraryRefreshSeries
            | JobKey::BackgroundLibraryRefreshAnime => {
                if !background_library_refresh_enabled() {
                    return Err(AppError::Validation(
                        "background library refresh is temporarily disabled".into(),
                    ));
                }
                let summary = if let Some(library_id) = job_run_library_id(run) {
                    self.background_library_refresh_by_id_with_tracking(&actor, library_id, run_id)
                        .await?
                } else {
                    let facet = job_key_library_facet(job_key).expect("background refresh facet");
                    self.background_library_refresh_with_tracking(&actor, facet, run_id)
                        .await?
                };
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::ProwlarrSync => {
                let (synced_count, failures) = self.sync_enabled_prowlarr_indexers(&actor).await?;
                if failures.is_empty() {
                    Ok(JobExecutionOutcome::new(
                        Some(format!("Synced {synced_count} enabled Prowlarr parent(s)")),
                        serde_json::to_string(&CountSummary {
                            count: synced_count,
                        })
                        .ok(),
                    ))
                } else {
                    Ok(JobExecutionOutcome::warning(
                        Some(format!(
                            "Synced {synced_count} enabled Prowlarr parent(s); {} failed",
                            failures.len()
                        )),
                        serde_json::to_string(&json!({
                            "syncedCount": synced_count,
                            "failedCount": failures.len(),
                            "failures": failures,
                        }))
                        .ok(),
                    ))
                }
            }
            JobKey::RssSync => {
                let report = self.run_scheduled_rss_sync().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Fetched {}, matched {}, grabbed {}",
                        report.releases_fetched, report.releases_matched, report.releases_grabbed
                    )),
                    serde_json::to_string(&RssSyncRunSummary {
                        releases_fetched: report.releases_fetched,
                        releases_matched: report.releases_matched,
                        releases_grabbed: report.releases_grabbed,
                        releases_held: report.releases_held,
                    })
                    .ok(),
                ))
            }
            JobKey::SubtitleSearch => Ok(JobExecutionOutcome::new(
                Some(self.run_subtitle_search_job().await?),
                None,
            )),
            JobKey::PluginRegistryRefresh => {
                self.refresh_plugin_catalog_internal().await?;
                Ok(plugin_registry_refresh_outcome(
                    self.run_scheduled_plugin_auto_update().await,
                ))
            }
            JobKey::Housekeeping => {
                let report = self.run_scheduled_housekeeping().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Removed {} orphaned media files and {} stale release decisions",
                        report.orphaned_media_files, report.stale_release_decisions
                    )),
                    serde_json::to_string(&HousekeepingRunSummary {
                        orphaned_media_files: report.orphaned_media_files,
                        stale_release_decisions: report.stale_release_decisions,
                        stale_release_attempts: report.stale_release_attempts,
                        stale_indexer_errors: report.stale_indexer_errors,
                        stale_history_events: report.stale_history_events,
                        stale_history_records: report.stale_history_records,
                        staged_nzb_artifacts_pruned: report.staged_nzb_artifacts_pruned,
                        recycled_purged: report.recycled_purged,
                        recycled_pending_reconciled: report.recycled_pending_reconciled,
                        discovery_pruned_runs: report.discovery_pruned_runs,
                    })
                    .ok(),
                ))
            }
            JobKey::HealthChecks => {
                let results = self.run_health_checks().await;
                *self.runtime.health.results.write().await = results.clone();
                let errors = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Error))
                    .count();
                let warnings = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Warning))
                    .count();
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Completed {} health checks ({} errors, {} warnings)",
                        results.len(),
                        errors,
                        warnings
                    )),
                    serde_json::to_string(&HealthChecksSummary {
                        total: results.len(),
                        errors,
                        warnings,
                        checks: results
                            .iter()
                            .map(|result| HealthCheckSummaryItem {
                                source: result.source.clone(),
                                status: result.status.as_str().to_string(),
                                message: result.message.clone(),
                            })
                            .collect(),
                    })
                    .ok(),
                ))
            }
            JobKey::AutoBackup => {
                let outcome = self.run_auto_backup_job().await?;
                match outcome.clone() {
                    crate::security::backup::AutoBackupRunOutcome::Created {
                        info,
                        pruned_count,
                    } => {
                        let summary_text = Some(format!(
                            "Created {} (encrypted) and pruned {} older automatic backup{}",
                            info.filename,
                            pruned_count,
                            if pruned_count == 1 { "" } else { "s" },
                        ));
                        let summary_json = serde_json::json!({
                            "filename": info.filename,
                            "encrypted": info.encrypted,
                            "prunedCount": pruned_count,
                            "trigger": info.trigger.as_str(),
                        })
                        .to_string();
                        Ok(JobExecutionOutcome::new(summary_text, Some(summary_json))
                            .with_auto_backup_outcome(outcome))
                    }
                    crate::security::backup::AutoBackupRunOutcome::Skipped { reason } => {
                        Ok(JobExecutionOutcome::warning(Some(reason), None)
                            .with_auto_backup_outcome(outcome))
                    }
                }
            }
            JobKey::PendingReleaseProcessing => Ok(JobExecutionOutcome::new(
                Some(
                    "Pending releases are re-evaluated with fresh RSS results during RSS sync"
                        .to_string(),
                ),
                serde_json::to_string(&CountSummary { count: 0 }).ok(),
            )),
            JobKey::StagedNzbPrune => {
                let count = self
                    .services
                    .workflow
                    .staged_nzb_store
                    .prune_staged_nzbs_older_than(Utc::now() - chrono::Duration::hours(1))
                    .await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Pruned {count} staged NZB artifacts")),
                    serde_json::to_string(&CountSummary { count }).ok(),
                ))
            }
            JobKey::MaintenanceRuleEvaluation => {
                let report = self.run_maintenance_rule_evaluation_job().await?;
                let summary_json = serde_json::to_string(&report).ok();
                if !report.gate_enabled {
                    return Ok(JobExecutionOutcome::new(
                        Some(
                            "Maintenance evaluation is disabled by the instance gate; nothing was evaluated"
                                .to_string(),
                        ),
                        summary_json,
                    ));
                }
                let summary_text = format!(
                    "Evaluated {} rule(s) over {} title(s): {} candidate(s) opened, {} canceled, {} superseded, {} held",
                    report.rules_evaluated,
                    report.titles_evaluated,
                    report.candidates_created,
                    report.candidates_canceled,
                    report.candidates_superseded,
                    report.candidates_held,
                );
                if report.rules_failed > 0 {
                    Ok(JobExecutionOutcome::warning(
                        Some(format!(
                            "{summary_text}; {} rule(s) failed and their candidates were held",
                            report.rules_failed
                        )),
                        summary_json,
                    ))
                } else {
                    Ok(JobExecutionOutcome::new(Some(summary_text), summary_json))
                }
            }
            JobKey::LifecycleActionHandling => {
                let report = self.run_lifecycle_action_handling_job().await?;
                let summary_json = serde_json::to_string(&report).ok();
                if !report.gates_enabled {
                    return Ok(JobExecutionOutcome::new(
                        Some(
                            "Both instance effect gates are off; no maintenance actions executed"
                                .to_string(),
                        ),
                        summary_json,
                    ));
                }
                let summary_text = format!(
                    "Considered {} candidate(s) across {} armed rule(s): {} executed, {} already satisfied, {} held, {} canceled, {} failed",
                    report.candidates_considered,
                    report.rules_eligible,
                    report.executed,
                    report.already_satisfied,
                    report.held,
                    report.canceled,
                    report.failed,
                );
                if report.failed > 0 {
                    Ok(JobExecutionOutcome::warning(
                        Some(summary_text),
                        summary_json,
                    ))
                } else {
                    Ok(JobExecutionOutcome::new(Some(summary_text), summary_json))
                }
            }
            JobKey::MediaServerSignalSync => {
                let report = self.run_media_server_signal_sync_job().await?;
                crate::media_server_signals::log_signal_sync_report(&report);
                let summary_json = serde_json::to_string(&report).ok();
                let summary_text = crate::media_server_signals::signal_sync_summary(&report);
                // A failed connection or participant is a warning, not an
                // error: the sweep still stored everything it could read, and
                // the per-connection state row carries the reason.
                if report.connections_failed > 0 || report.participants_failed > 0 {
                    Ok(JobExecutionOutcome::warning(
                        Some(format!(
                            "{summary_text}; {} connection(s) and {} participant(s) failed",
                            report.connections_failed, report.participants_failed
                        )),
                        summary_json,
                    ))
                } else {
                    Ok(JobExecutionOutcome::new(Some(summary_text), summary_json))
                }
            }
            JobKey::FullHashBackfill => {
                let summary = self.run_full_hash_backfill_job().await?;
                Ok(JobExecutionOutcome::new(
                    Some(summary.summary_text()),
                    serde_json::to_string(&summary).ok(),
                ))
            }
            JobKey::DiscoverySync => self.run_discovery_sync_job(run.trigger_source).await,
            JobKey::TitleImageCacheRefresh => {
                let summary = self.run_title_image_cache_refresh().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Refreshed artwork URLs for {} title(s) and {} episode(s); image cache reset",
                        summary.title_urls_updated, summary.episode_urls_updated
                    )),
                    serde_json::to_string(&summary).ok(),
                ))
            }
            JobKey::TitleDeletion => Err(AppError::Validation(
                "title deletion jobs must be started from the title deletion mutation".into(),
            )),
            JobKey::TitleRename => Err(AppError::Validation(
                "title rename jobs must be started from the rename mutation".into(),
            )),
            JobKey::MediaFileDeletion => Err(AppError::Validation(
                "media file deletion jobs must be started from the media file deletion mutation"
                    .into(),
            )),
            JobKey::RecycleBinRestore => Err(AppError::Validation(
                "recycle restore jobs must be started from the recycle restore mutation".into(),
            )),
            JobKey::RecycleBinPurge => Err(AppError::Validation(
                "recycle purge jobs must be started from the recycle bin mutation".into(),
            )),
            JobKey::AcquisitionSearch => Err(AppError::Validation(
                "acquisition search jobs must be started from the acquisition search mutation"
                    .into(),
            )),
            JobKey::ApplicationUpgrade => Err(AppError::Validation(
                "application upgrade jobs must be started from the application upgrade mutation"
                    .into(),
            )),
            JobKey::LocationOperation => Err(AppError::Validation(
                "location operations must be started from the location operation mutation".into(),
            )),
        }
    }

    async fn run_discovery_sync_job(
        &self,
        trigger_source: JobTriggerSource,
    ) -> AppResult<JobExecutionOutcome> {
        let lease_owner_id = format!("discovery-sync-{}", uuid::Uuid::new_v4());
        let acquired_at = self.runtime.environment.now();
        let acquired = self
            .services
            .library
            .discovery
            .try_acquire_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                &lease_owner_id,
                acquired_at + chrono::Duration::seconds(DISCOVERY_SYNC_LEASE_SECONDS),
                acquired_at,
            )
            .await?;
        if !acquired {
            return Ok(JobExecutionOutcome::warning(
                Some("Discovery sync is already running on another worker".to_string()),
                None,
            ));
        }

        let result = self
            .run_discovery_sync_job_with_lease(trigger_source, &lease_owner_id)
            .await;
        let released_at = self.runtime.environment.now();
        let release_result = self
            .services
            .library
            .discovery
            .release_discovery_sync_lease(DISCOVERY_DEFAULT_SCOPE_KEY, &lease_owner_id, released_at)
            .await;
        if let Err(error) = release_result {
            warn!(
                error = %error,
                "failed to release discovery sync lease after job run"
            );
        }
        result
    }

    async fn run_discovery_sync_job_with_lease(
        &self,
        trigger_source: JobTriggerSource,
        lease_owner_id: &str,
    ) -> AppResult<JobExecutionOutcome> {
        let now = self.runtime.environment.now();
        let scheduler_seed = self.discovery_scheduler_seed().await?;
        let titles = self.services.catalog.titles.list(None, None).await?;
        let defaults = DiscoveryContextDefaults {
            region: self.discovery_region().await,
            language: self.metadata_language().await,
            ..DiscoveryContextDefaults::default()
        };
        let library_context = build_discovery_library_context(&titles, defaults.clone());
        let existing_state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?;
        let state_created = existing_state.is_none();
        let mut state = existing_state.unwrap_or_default();
        let subject_context_changed =
            state.last_subject_fingerprint.as_deref() != Some(library_context.fingerprint.as_str());

        state.startup_jitter_seconds = discovery_jitter_seconds(
            &scheduler_seed,
            "startup",
            DISCOVERY_SYNC_BOOTSTRAP_JITTER_WINDOW_SECONDS,
        );
        state.context_jitter_seconds = discovery_jitter_seconds(
            &scheduler_seed,
            "context_snapshot",
            DISCOVERY_SYNC_JITTER_WINDOW_SECONDS,
        );
        state.incremental_reload_jitter_seconds = discovery_jitter_seconds(
            &scheduler_seed,
            "incremental_reload",
            DISCOVERY_SYNC_INCREMENTAL_CADENCE_SECONDS,
        );
        state.public_feed_jitter_seconds = discovery_jitter_seconds(
            &scheduler_seed,
            "public_feed",
            DISCOVERY_SYNC_JITTER_WINDOW_SECONDS,
        );

        let next_incremental =
            next_hash_jittered_bucket(now, state.incremental_reload_jitter_seconds);
        let next_context = now
            + chrono::Duration::seconds(
                DISCOVERY_SYNC_DAILY_BACKSTOP_SECONDS + state.context_jitter_seconds,
            );
        let next_public = now
            + chrono::Duration::seconds(
                DISCOVERY_SYNC_DAILY_BACKSTOP_SECONDS + state.public_feed_jitter_seconds,
            );

        if state.last_success_generation_id.is_some() {
            if state.next_incremental_reload_eligible_at.is_none() {
                state.next_incremental_reload_eligible_at = Some(next_incremental);
                state.updated_at = now;
            }
            if state.next_context_snapshot_eligible_at.is_none() {
                state.next_context_snapshot_eligible_at = Some(next_context);
                state.updated_at = now;
            }
        }
        if state.last_public_feed_generation_id.is_some()
            && state.next_public_feed_eligible_at.is_none()
        {
            state.next_public_feed_eligible_at = Some(next_public);
            state.updated_at = now;
        }

        let ack_recovery = self
            .retry_unacked_discovery_context_snapshot_acks(&mut state, now)
            .await?;

        self.catch_up_discovery_context_dirty_state(&mut state, now)
            .await?;
        let mut pending_changes = self
            .services
            .library
            .discovery
            .list_all_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?;
        let pending_changes_are_quiet = discovery_pending_changes_are_quiet(now, &pending_changes);
        let unchanged_fingerprint_cleanup_due = state.last_success_generation_id.is_some()
            && state.inflight_context_snapshot_run_id.is_none()
            && !subject_context_changed
            && state.dirty_since.is_some()
            && pending_changes_are_quiet;
        let cleanup_sequence = pending_changes
            .iter()
            .filter_map(|change| change.last_seen_sequence)
            .max();
        if unchanged_fingerprint_cleanup_due
            && (pending_changes.is_empty() || cleanup_sequence.is_some())
        {
            if let Some(sequence) = cleanup_sequence {
                self.services
                    .library
                    .discovery
                    .clear_pending_discovery_context_changes_through_sequence(
                        DISCOVERY_DEFAULT_SCOPE_KEY,
                        sequence,
                    )
                    .await?;
            }
            state.dirty_since = None;
            state.dirty_reason_mask = 0;
            state.bootstrap_started_at = None;
            state.bootstrap_quiet_until = None;
            if let Some(sequence) = cleanup_sequence {
                state.last_seen_domain_event_sequence = Some(
                    state
                        .last_seen_domain_event_sequence
                        .unwrap_or_default()
                        .max(sequence),
                );
            }
            state.updated_at = now;
            pending_changes.clear();
        }
        let incremental_changes_need_snapshot_reconciliation =
            pending_context_changes_need_snapshot_reconciliation(&pending_changes);
        let full_snapshot_reconciliation_due = state.last_success_generation_id.is_some()
            && !pending_changes.is_empty()
            && pending_changes_are_quiet
            && incremental_changes_need_snapshot_reconciliation;

        let active_scan_count = self.active_library_scan_run_count().await?;
        let scans_active = active_scan_count > 0;
        let snapshot_backoff_ready = state.backoff_until.is_none_or(|until| now >= until);
        let first_snapshot_pending =
            state.last_success_generation_id.is_none() && !library_context.subjects.is_empty();
        let discovery_sync_tracker_due = trigger_source == JobTriggerSource::ScheduledInterval
            && self
                .runtime
                .jobs
                .job_run_tracker
                .next_run_at(JobKey::DiscoverySync)
                .await
                .is_some_and(|next_run_at| now >= next_run_at);
        let first_snapshot_silent_wake_due = first_snapshot_pending
            && state.next_context_snapshot_eligible_at.is_none()
            && discovery_sync_tracker_due;
        let first_snapshot_bootstrap_quiet_due = first_snapshot_pending
            && state
                .bootstrap_quiet_until
                .is_some_and(|quiet_until| now >= quiet_until);
        let first_snapshot_candidate_at = state
            .bootstrap_quiet_until
            .filter(|quiet_until| *quiet_until > now)
            .map(|quiet_until| quiet_until.max(discovery_accelerated_at(now, &scheduler_seed)))
            .unwrap_or_else(|| discovery_accelerated_at(now, &scheduler_seed));
        if first_snapshot_pending
            && state.inflight_context_snapshot_run_id.is_none()
            && snapshot_backoff_ready
            && !first_snapshot_bootstrap_quiet_due
            && !first_snapshot_silent_wake_due
            && discovery_prefer_earlier_gate(
                &mut state.next_context_snapshot_eligible_at,
                first_snapshot_candidate_at,
            )
        {
            state.updated_at = now;
        }
        let scan_blocked_retry_at = scans_active.then_some(if first_snapshot_pending {
            discovery_accelerated_at(now, &scheduler_seed)
        } else {
            now + chrono::Duration::seconds(DISCOVERY_SYNC_BOOTSTRAP_QUIET_SECONDS)
        });
        let snapshot_resume_due = state.inflight_context_snapshot_run_id.is_some()
            && !scans_active
            && snapshot_backoff_ready;
        let last_context_reload_completed_at = [
            state.last_context_snapshot_completed_at,
            state.last_incremental_reload_completed_at,
        ]
        .into_iter()
        .flatten()
        .max();
        let manual_context_cooldown_active = trigger_source == JobTriggerSource::Manual
            && state.last_success_generation_id.is_some()
            && last_context_reload_completed_at.is_some_and(|completed_at| {
                now < completed_at
                    + chrono::Duration::seconds(DISCOVERY_SYNC_MANUAL_CONTEXT_COOLDOWN_SECONDS)
            });
        let snapshot_can_submit = !library_context.subjects.is_empty()
            && !scans_active
            && state.inflight_context_snapshot_run_id.is_none()
            && !manual_context_cooldown_active
            && snapshot_backoff_ready;
        let context_snapshot_due = if snapshot_resume_due {
            true
        } else if snapshot_can_submit && state.last_success_generation_id.is_none() {
            // First snapshot: let the bootstrap quiet window elapse so library churn
            // settles before the initial SMG submission. An explicit manual trigger
            // skips the settle window (backoff still applies via snapshot_can_submit).
            subject_context_changed
                && (trigger_source == JobTriggerSource::Manual
                    || first_snapshot_bootstrap_quiet_due
                    || (state
                        .next_context_snapshot_eligible_at
                        .is_none_or(|gate| now >= gate)
                        && state
                            .bootstrap_quiet_until
                            .is_none_or(|quiet_until| now >= quiet_until)))
        } else {
            snapshot_can_submit
                && (state
                    .next_context_snapshot_eligible_at
                    .is_some_and(|gate| now >= gate)
                    || full_snapshot_reconciliation_due)
                && state.last_success_generation_id.is_some()
        };

        // Public feed is independent of the personalized pipeline: refresh it
        // immediately on startup (before any snapshot work) so the public rails
        // populate as soon as the app boots; otherwise hold to the daily gate.
        let public_feed_due = trigger_source == JobTriggerSource::Manual
            || trigger_source == JobTriggerSource::ScheduledStartup
            || state.last_public_feed_generation_id.is_none()
            || state
                .next_public_feed_eligible_at
                .is_some_and(|gate| now >= gate);
        let public_feed = if public_feed_due {
            Some(
                self.execute_discovery_public_feed(trigger_source, &defaults, &mut state, now)
                    .await?,
            )
        } else {
            None
        };

        let context_snapshot = if context_snapshot_due {
            Some(
                self.execute_discovery_context_snapshot(
                    trigger_source,
                    &defaults,
                    &library_context,
                    &mut state,
                    lease_owner_id,
                    now,
                )
                .await?,
            )
        } else {
            None
        };

        let incremental_due = context_snapshot.is_none()
            && !scans_active
            && state.last_success_generation_id.is_some()
            && !pending_changes.is_empty()
            && pending_changes_are_quiet
            && !incremental_changes_need_snapshot_reconciliation
            && state
                .next_incremental_reload_eligible_at
                .is_some_and(|gate| now >= gate)
            && state.inflight_context_snapshot_run_id.is_none()
            && !manual_context_cooldown_active
            && state.backoff_until.is_none_or(|until| now >= until);

        let context_incremental = if incremental_due {
            Some(
                self.execute_discovery_context_incremental(
                    trigger_source,
                    &defaults,
                    &library_context,
                    &pending_changes,
                    &mut state,
                    now,
                )
                .await?,
            )
        } else {
            None
        };

        let effective_next_incremental = state
            .next_incremental_reload_eligible_at
            .unwrap_or(next_incremental);
        let effective_next_incremental = if state.last_success_generation_id.is_none() {
            state
                .bootstrap_quiet_until
                .map(|quiet_until| effective_next_incremental.max(quiet_until))
                .unwrap_or(effective_next_incremental)
        } else {
            effective_next_incremental
        };
        let effective_next_context = state
            .next_context_snapshot_eligible_at
            .unwrap_or(next_context);
        let effective_next_public = state.next_public_feed_eligible_at.unwrap_or(next_public);
        let pending_changes_quiet_at = (!pending_changes_are_quiet)
            .then(|| discovery_pending_changes_quiet_at(&pending_changes))
            .flatten();

        let next_run_at = discovery_next_run_at(
            now,
            DiscoveryNextRunCandidates {
                next_incremental: effective_next_incremental,
                incremental_reload_possible: state.last_success_generation_id.is_some(),
                next_context: effective_next_context,
                next_public: effective_next_public,
                bootstrap_quiet_until: state.bootstrap_quiet_until,
                backoff_until: state.backoff_until,
                scan_blocked_retry_at,
                pending_changes_quiet_at,
            },
        );

        self.services
            .library
            .discovery
            .upsert_discovery_sync_state(&state)
            .await?;
        self.set_job_next_run_at(JobKey::DiscoverySync, next_run_at)
            .await;

        let summary = DiscoverySyncRunSummary {
            state_created,
            trigger_source: trigger_source.as_str().to_string(),
            subject_count: library_context.subjects.len(),
            subject_fingerprint: library_context.fingerprint.clone(),
            subject_context_changed,
            ack_recovery,
            context_snapshot,
            context_incremental,
            public_feed,
            next_run_at: next_run_at.to_rfc3339(),
            next_incremental_reload_eligible_at: effective_next_incremental.to_rfc3339(),
            next_context_snapshot_eligible_at: effective_next_context.to_rfc3339(),
            next_public_feed_eligible_at: effective_next_public.to_rfc3339(),
            startup_jitter_seconds: state.startup_jitter_seconds,
            context_jitter_seconds: state.context_jitter_seconds,
            incremental_reload_jitter_seconds: state.incremental_reload_jitter_seconds,
            public_feed_jitter_seconds: state.public_feed_jitter_seconds,
        };
        let summary_json = serde_json::to_string(&summary).ok();

        if trigger_source == JobTriggerSource::Manual
            && summary
                .ack_recovery
                .as_ref()
                .is_none_or(|ack_recovery| ack_recovery.attempted == 0)
            && summary.context_snapshot.is_none()
            && summary.context_incremental.is_none()
            && summary.public_feed.is_none()
        {
            self.record_deferred_discovery_sync_run(
                trigger_source,
                &defaults,
                &state,
                library_context.subjects.len(),
                Some(library_context.fingerprint.clone()),
                now,
                "No discovery sync work is currently eligible",
            )
            .await?;
            return Ok(JobExecutionOutcome::warning(
                Some("Discovery sync deferred; no work is currently eligible".to_string()),
                summary_json,
            ));
        }

        Ok(JobExecutionOutcome::new(
            Some(format!(
                "Discovery sync evaluated {} local subjects; next incremental reload window at {}",
                library_context.subjects.len(),
                effective_next_incremental.to_rfc3339()
            )),
            summary_json,
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "deferred discovery sync records the observed state and scheduling reason explicitly"
    )]
    async fn record_deferred_discovery_sync_run(
        &self,
        trigger_source: JobTriggerSource,
        defaults: &DiscoveryContextDefaults,
        state: &DiscoverySyncStateRecord,
        subject_count: usize,
        subject_fingerprint: Option<String>,
        observed_at: DateTime<Utc>,
        reason: &str,
    ) -> AppResult<()> {
        let run = DiscoverySyncRunRecord {
            id: format!("deferred-{}", uuid::Uuid::new_v4()),
            kind: "deferred".to_string(),
            status: "deferred".to_string(),
            trigger_source: trigger_source.as_str().to_string(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            subject_count: subject_count as i64,
            subject_fingerprint,
            previous_subject_fingerprint: state.last_subject_fingerprint.clone(),
            base_generation_id: state.last_success_generation_id.clone(),
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: None,
            discovery_index_watermark: None,
            page_count: None,
            item_count: None,
            facet_count: None,
            acknowledged_at: None,
            error_text: Some(reason.to_string()),
            started_at: Some(observed_at),
            completed_at: Some(observed_at),
            created_at: observed_at,
            updated_at: observed_at,
        };
        self.services
            .library
            .discovery
            .upsert_discovery_sync_run(&run)
            .await
    }

    async fn catch_up_discovery_context_dirty_state(
        &self,
        state: &mut DiscoverySyncStateRecord,
        now: DateTime<Utc>,
    ) -> AppResult<usize> {
        let start_sequence = state.last_seen_domain_event_sequence.unwrap_or_default();
        let mut after_sequence = start_sequence;
        let mut max_seen_sequence = after_sequence;
        let mut subject_change_count = 0usize;
        let event_types = discovery_context_dirty_event_types();

        for _ in 0..DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_MAX_BATCHES {
            let events = self
                .services
                .events
                .domain_events
                .list(&DomainEventFilter {
                    after_sequence: Some(after_sequence),
                    event_types: Some(event_types.clone()),
                    limit: DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_BATCH_LIMIT,
                    ..DomainEventFilter::default()
                })
                .await?;
            if events.is_empty() {
                break;
            }

            let batch_len = events.len();
            for event in events {
                after_sequence = after_sequence.max(event.sequence);
                max_seen_sequence = max_seen_sequence.max(event.sequence);
                let pending_change =
                    pending_context_change_from_domain_event(DISCOVERY_DEFAULT_SCOPE_KEY, &event)?;
                if let Some(change) = pending_change {
                    subject_change_count += 1;
                    mark_discovery_context_dirty(state, event.occurred_at);
                    if state.last_success_generation_id.is_some() {
                        let existing = self
                            .services
                            .library
                            .discovery
                            .get_pending_discovery_context_change(&change.id)
                            .await?;
                        match coalesce_pending_context_change(existing.as_ref(), change)? {
                            Some(change) => {
                                self.services
                                    .library
                                    .discovery
                                    .upsert_pending_discovery_context_change(&change)
                                    .await?;
                            }
                            None => {
                                if let Some(existing) = existing.as_ref() {
                                    self.services
                                        .library
                                        .discovery
                                        .delete_pending_discovery_context_change(&existing.id)
                                        .await?;
                                }
                            }
                        }
                    } else {
                        extend_discovery_bootstrap_quiet_window(state, event.occurred_at);
                    }
                } else if discovery_scan_boundary_event(&event) && state.dirty_since.is_some() {
                    state.dirty_reason_mask |= DISCOVERY_DIRTY_REASON_SCAN_BOUNDARY;
                }
            }

            if batch_len < DISCOVERY_SYNC_DOMAIN_EVENT_CATCH_UP_BATCH_LIMIT {
                break;
            }
        }

        if max_seen_sequence > start_sequence {
            state.last_seen_domain_event_sequence = Some(
                state
                    .last_seen_domain_event_sequence
                    .unwrap_or_default()
                    .max(max_seen_sequence),
            );
            state.updated_at = now;
        }

        Ok(subject_change_count)
    }

    async fn retry_unacked_discovery_context_snapshot_acks(
        &self,
        state: &mut DiscoverySyncStateRecord,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DiscoveryAckRecoveryRunSummary>> {
        let mut runs = self
            .services
            .library
            .discovery
            .list_unacked_discovery_context_snapshot_runs(10)
            .await?;
        if runs.is_empty() {
            return Ok(None);
        }

        let mut summary = DiscoveryAckRecoveryRunSummary {
            attempted: 0,
            acknowledged: 0,
            failed_run_id: None,
            next_retry_at: None,
        };

        for run in &mut runs {
            let Some(request_id) = run.smg_request_id.clone() else {
                continue;
            };
            summary.attempted += 1;
            match self
                .services
                .library
                .metadata_gateway
                .acknowledge_discovery_context_snapshot(&request_id)
                .await
            {
                Ok(_) => {
                    run.acknowledged_at = Some(now);
                    run.status = "complete".to_string();
                    run.error_text = None;
                    run.updated_at = now;
                    self.services
                        .library
                        .discovery
                        .upsert_discovery_sync_run(run)
                        .await?;
                    discovery_reset_transient_failure_count(state);
                    summary.acknowledged += 1;
                }
                Err(error) => {
                    let failed_at = now;
                    let retry_at = discovery_transient_retry_after(state, failed_at);
                    run.status = "warning".to_string();
                    run.error_text = Some(format!(
                        "local discovery snapshot committed but SMG ack retry failed: {error}"
                    ));
                    run.updated_at = failed_at;
                    self.services
                        .library
                        .discovery
                        .upsert_discovery_sync_run(run)
                        .await?;
                    state.backoff_until = Some(retry_at);
                    state.updated_at = failed_at;
                    summary.failed_run_id = Some(run.id.clone());
                    summary.next_retry_at = Some(retry_at.to_rfc3339());
                    break;
                }
            }
        }

        if summary.attempted == 0 {
            Ok(None)
        } else {
            Ok(Some(summary))
        }
    }

    async fn execute_discovery_public_feed(
        &self,
        trigger_source: JobTriggerSource,
        defaults: &DiscoveryContextDefaults,
        state: &mut DiscoverySyncStateRecord,
        started_at: DateTime<Utc>,
    ) -> AppResult<DiscoveryPublicFeedRunSummary> {
        let run_id = format!("public-feed-{}", uuid::Uuid::new_v4());
        let mut run = DiscoverySyncRunRecord {
            id: run_id.clone(),
            kind: "public_feed".to_string(),
            status: "running".to_string(),
            trigger_source: trigger_source.as_str().to_string(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            subject_count: 0,
            subject_fingerprint: None,
            previous_subject_fingerprint: None,
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: None,
            discovery_index_watermark: None,
            page_count: None,
            item_count: None,
            facet_count: None,
            acknowledged_at: None,
            error_text: None,
            started_at: Some(started_at),
            completed_at: None,
            created_at: started_at,
            updated_at: started_at,
        };
        self.services
            .library
            .discovery
            .upsert_discovery_sync_run(&run)
            .await?;

        let input = defaults.public_feed_input();
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(DISCOVERY_PUBLIC_FEED_REQUEST_TIMEOUT_SECONDS),
            self.services
                .library
                .metadata_gateway
                .discover_public_feed(&input),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                self.fail_discovery_sync_run(&mut run, &error.to_string())
                    .await?;
                let failed_at = self.runtime.environment.now();
                let retry_at = discovery_transient_retry_after(state, failed_at);
                discovery_schedule_public_feed_retry(state, retry_at);
                state.updated_at = failed_at;
                return Ok(DiscoveryPublicFeedRunSummary {
                    run_id,
                    committed: false,
                    section_count: 0,
                    item_count: 0,
                });
            }
            Err(_) => {
                let error = format!(
                    "SMG discovery public feed request timed out after {DISCOVERY_PUBLIC_FEED_REQUEST_TIMEOUT_SECONDS}s"
                );
                self.fail_discovery_sync_run(&mut run, &error).await?;
                let failed_at = self.runtime.environment.now();
                let retry_at = discovery_transient_retry_after(state, failed_at);
                discovery_schedule_public_feed_retry(state, retry_at);
                state.updated_at = failed_at;
                return Ok(DiscoveryPublicFeedRunSummary {
                    run_id,
                    committed: false,
                    section_count: 0,
                    item_count: 0,
                });
            }
        };

        let completed_at = self.runtime.environment.now();
        let sections = public_feed_section_records(&run_id, &result, completed_at)?;
        let items = public_feed_item_records(&run_id, &result, completed_at)?;
        let facet_count = result
            .sections
            .iter()
            .filter(|section| {
                !section
                    .section_type
                    .trim()
                    .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
            })
            .map(|section| section.facets.len() as i64)
            .sum::<i64>();

        run.status = "complete".to_string();
        run.smg_status = Some("COMPLETE".to_string());
        run.page_count = Some(sections.len() as i32);
        run.item_count = Some(items.len() as i64);
        run.facet_count = Some(facet_count);
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;

        state.last_public_feed_generation_id = Some(run_id.clone());
        state.last_public_feed_completed_at = Some(completed_at);
        state.next_public_feed_eligible_at = Some(
            completed_at
                + chrono::Duration::seconds(
                    DISCOVERY_SYNC_DAILY_BACKSTOP_SECONDS + state.public_feed_jitter_seconds,
                ),
        );
        state.updated_at = completed_at;

        let section_count = sections.len() as i64;
        let item_count = items.len() as i64;
        self.services
            .library
            .discovery
            .commit_discovery_public_feed(&DiscoveryPublicFeedCommit {
                state: state.clone(),
                run: run.clone(),
                sections,
                items,
            })
            .await?;

        Ok(DiscoveryPublicFeedRunSummary {
            run_id,
            committed: true,
            section_count,
            item_count,
        })
    }

    async fn renew_discovery_sync_lease(&self, lease_owner_id: &str) -> AppResult<bool> {
        let now = self.runtime.environment.now();
        self.services
            .library
            .discovery
            .renew_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                lease_owner_id,
                now + chrono::Duration::seconds(DISCOVERY_SYNC_LEASE_SECONDS),
                now,
            )
            .await
    }

    async fn execute_discovery_context_snapshot(
        &self,
        trigger_source: JobTriggerSource,
        defaults: &DiscoveryContextDefaults,
        library_context: &DiscoveryLibraryContext,
        state: &mut DiscoverySyncStateRecord,
        lease_owner_id: &str,
        started_at: DateTime<Utc>,
    ) -> AppResult<DiscoveryContextSnapshotRunSummary> {
        let mut resumed_run = None;
        if let Some(run_id) = state.inflight_context_snapshot_run_id.clone() {
            match self
                .services
                .library
                .discovery
                .get_discovery_sync_run(&run_id)
                .await?
            {
                Some(run) if run.smg_request_id.is_some() => {
                    resumed_run = Some(run);
                }
                _ => {
                    state.inflight_context_snapshot_run_id = None;
                    state.inflight_subject_fingerprint = None;
                    state.inflight_domain_event_sequence = None;
                }
            }
        }

        let (run_id, mut run, request_id) = if let Some(mut run) = resumed_run {
            run.status = "running".to_string();
            run.error_text = None;
            run.updated_at = started_at;
            let request_id = run
                .smg_request_id
                .clone()
                .expect("resumed discovery snapshot run has request id");
            (run.id.clone(), run, request_id)
        } else {
            let run_id = format!("context-snapshot-{}", uuid::Uuid::new_v4());
            let mut run = DiscoverySyncRunRecord {
                id: run_id.clone(),
                kind: "context_snapshot".to_string(),
                status: "running".to_string(),
                trigger_source: trigger_source.as_str().to_string(),
                region: defaults.region.clone(),
                language: defaults.language.clone(),
                subject_count: library_context.subjects.len() as i64,
                subject_fingerprint: Some(library_context.fingerprint.clone()),
                previous_subject_fingerprint: state.last_subject_fingerprint.clone(),
                base_generation_id: None,
                changed_subject_count: 0,
                affected_target_count: 0,
                smg_request_id: None,
                smg_status: None,
                discovery_index_watermark: None,
                page_count: None,
                item_count: None,
                facet_count: None,
                acknowledged_at: None,
                error_text: None,
                started_at: Some(started_at),
                completed_at: None,
                created_at: started_at,
                updated_at: started_at,
            };
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;

            let submit_input = library_context.snapshot_submit_input(defaults);
            let submit_result = match self
                .services
                .library
                .metadata_gateway
                .submit_discovery_context_snapshot(&submit_input)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    let failed_at = self.runtime.environment.now();
                    run.status = "deferred".to_string();
                    run.error_text = Some(format!("SMG discovery snapshot submit failed: {error}"));
                    run.completed_at = Some(failed_at);
                    run.updated_at = failed_at;
                    let retry_at = discovery_transient_retry_after(state, failed_at);
                    discovery_schedule_context_snapshot_retry(state, retry_at);
                    state.updated_at = failed_at;
                    self.services
                        .library
                        .discovery
                        .upsert_discovery_sync_run(&run)
                        .await?;
                    return Ok(DiscoveryContextSnapshotRunSummary {
                        run_id,
                        committed: false,
                        smg_request_id: None,
                        smg_status: run.smg_status.clone(),
                        page_count: 0,
                        item_count: 0,
                        facet_count: 0,
                    });
                }
            };

            let now = self.runtime.environment.now();
            run.smg_status = Some(submit_result.status.clone());
            run.smg_request_id = submit_result.request_id.clone();
            run.updated_at = now;

            let Some(request_id) = submit_result.request_id.clone() else {
                run.status = "deferred".to_string();
                run.error_text = Some("SMG discovery snapshot was not accepted".to_string());
                run.completed_at = Some(now);
                let retry_at = discovery_retry_after(now, submit_result.retry_after_seconds);
                discovery_schedule_context_snapshot_retry(state, retry_at);
                state.updated_at = now;
                self.services
                    .library
                    .discovery
                    .upsert_discovery_sync_run(&run)
                    .await?;
                return Ok(DiscoveryContextSnapshotRunSummary {
                    run_id,
                    committed: false,
                    smg_request_id: None,
                    smg_status: run.smg_status.clone(),
                    page_count: 0,
                    item_count: 0,
                    facet_count: 0,
                });
            };

            state.inflight_context_snapshot_run_id = Some(run_id.clone());
            state.inflight_subject_fingerprint = Some(library_context.fingerprint.clone());
            state.inflight_domain_event_sequence = state.last_seen_domain_event_sequence;
            state.updated_at = now;
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;
            (run_id, run, request_id)
        };

        let status_result = match self
            .services
            .library
            .metadata_gateway
            .discovery_context_snapshot_status(&request_id)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let failed_at = self.runtime.environment.now();
                run.status = "deferred".to_string();
                run.error_text = Some(format!("SMG discovery snapshot status failed: {error}"));
                run.completed_at = Some(failed_at);
                run.updated_at = failed_at;
                let retry_at = discovery_transient_retry_after(state, failed_at);
                discovery_schedule_context_snapshot_retry(state, retry_at);
                state.updated_at = failed_at;
                self.services
                    .library
                    .discovery
                    .upsert_discovery_sync_run(&run)
                    .await?;
                return Ok(DiscoveryContextSnapshotRunSummary {
                    run_id,
                    committed: false,
                    smg_request_id: Some(request_id),
                    smg_status: run.smg_status.clone(),
                    page_count: 0,
                    item_count: 0,
                    facet_count: 0,
                });
            }
        };

        let status_checked_at = self.runtime.environment.now();
        run.smg_status = Some(status_result.status.clone());
        run.discovery_index_watermark =
            non_empty_discovery_string(status_result.discovery_index_watermark.as_str());
        run.page_count = Some(status_result.page_count);
        run.item_count = Some(i64::from(status_result.item_count));
        run.facet_count = Some(i64::from(status_result.facet_count));
        run.updated_at = status_checked_at;

        if !discovery_status_is(&status_result.status, "COMPLETE") {
            run.status = if discovery_snapshot_status_is_terminal(&status_result.status) {
                "failed".to_string()
            } else {
                "deferred".to_string()
            };
            run.error_text = Some(format!(
                "SMG discovery snapshot status is {}",
                status_result.status
            ));
            run.completed_at = Some(status_checked_at);
            state.inflight_subject_fingerprint = Some(library_context.fingerprint.clone());
            let retry_at =
                discovery_retry_after(status_checked_at, status_result.retry_after_seconds);
            discovery_schedule_context_snapshot_retry(state, retry_at);
            if discovery_snapshot_status_is_polling(&status_result.status) {
                state.inflight_context_snapshot_run_id = Some(run_id.clone());
                state.inflight_subject_fingerprint = Some(library_context.fingerprint.clone());
            } else {
                state.inflight_context_snapshot_run_id = None;
                state.inflight_subject_fingerprint = None;
                state.inflight_domain_event_sequence = None;
            }
            state.updated_at = status_checked_at;
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;
            return Ok(DiscoveryContextSnapshotRunSummary {
                run_id,
                committed: false,
                smg_request_id: Some(request_id),
                smg_status: run.smg_status.clone(),
                page_count: status_result.page_count,
                item_count: i64::from(status_result.item_count),
                facet_count: i64::from(status_result.facet_count),
            });
        }

        if !self.renew_discovery_sync_lease(lease_owner_id).await? {
            let failed_at = self.runtime.environment.now();
            run.status = "deferred".to_string();
            run.error_text = Some("discovery sync lease was lost before page fetch".to_string());
            run.completed_at = Some(failed_at);
            run.updated_at = failed_at;
            let retry_at = discovery_transient_retry_after(state, failed_at);
            discovery_schedule_context_snapshot_retry(state, retry_at);
            state.updated_at = failed_at;
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;
            return Ok(DiscoveryContextSnapshotRunSummary {
                run_id,
                committed: false,
                smg_request_id: Some(request_id),
                smg_status: run.smg_status.clone(),
                page_count: status_result.page_count,
                item_count: i64::from(status_result.item_count),
                facet_count: i64::from(status_result.facet_count),
            });
        }

        let mut pages = Vec::new();
        for page_number in 1..=status_result.page_count.max(0) {
            let page = match self
                .services
                .library
                .metadata_gateway
                .discovery_context_snapshot_page(&request_id, page_number)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    let failed_at = self.runtime.environment.now();
                    run.status = "deferred".to_string();
                    run.error_text =
                        Some(format!("SMG discovery snapshot page fetch failed: {error}"));
                    run.completed_at = Some(failed_at);
                    run.updated_at = failed_at;
                    state.inflight_context_snapshot_run_id = Some(run_id.clone());
                    state.inflight_subject_fingerprint = Some(library_context.fingerprint.clone());
                    let retry_at = discovery_transient_retry_after(state, failed_at);
                    discovery_schedule_context_snapshot_retry(state, retry_at);
                    state.updated_at = failed_at;
                    self.services
                        .library
                        .discovery
                        .upsert_discovery_sync_run(&run)
                        .await?;
                    return Ok(DiscoveryContextSnapshotRunSummary {
                        run_id,
                        committed: false,
                        smg_request_id: Some(request_id),
                        smg_status: run.smg_status.clone(),
                        page_count: status_result.page_count,
                        item_count: i64::from(status_result.item_count),
                        facet_count: i64::from(status_result.facet_count),
                    });
                }
            };
            pages.push(page);
        }

        if !self.renew_discovery_sync_lease(lease_owner_id).await? {
            let failed_at = self.runtime.environment.now();
            run.status = "deferred".to_string();
            run.error_text =
                Some("discovery sync lease was lost before snapshot commit".to_string());
            run.completed_at = Some(failed_at);
            run.updated_at = failed_at;
            let retry_at = discovery_transient_retry_after(state, failed_at);
            discovery_schedule_context_snapshot_retry(state, retry_at);
            state.updated_at = failed_at;
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;
            return Ok(DiscoveryContextSnapshotRunSummary {
                run_id,
                committed: false,
                smg_request_id: Some(request_id),
                smg_status: run.smg_status.clone(),
                page_count: status_result.page_count,
                item_count: i64::from(status_result.item_count),
                facet_count: i64::from(status_result.facet_count),
            });
        }

        let completed_at = self.runtime.environment.now();
        let snapshot_titles = pages
            .iter()
            .flat_map(|page| page.items.iter().cloned())
            .collect::<Vec<_>>();
        let subject_provenance = library_context.subject_provenance_by_key();
        let items = snapshot_item_records(
            &run_id,
            &run_id,
            &snapshot_titles,
            &subject_provenance,
            completed_at,
        )?;
        let facets = snapshot_facet_records(&run_id, &pages)?;
        let submitted_subjects = library_context.submitted_subject_records(&run_id)?;

        run.status = "complete".to_string();
        run.discovery_index_watermark =
            non_empty_discovery_string(status_result.discovery_index_watermark.as_str());
        run.page_count = Some(status_result.page_count);
        run.item_count = Some(items.len() as i64);
        run.facet_count = Some(facets.len() as i64);
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;

        state.last_success_generation_id = Some(run_id.clone());
        state.last_subject_fingerprint = Some(library_context.fingerprint.clone());
        state.last_context_snapshot_completed_at = Some(completed_at);
        state.next_context_snapshot_eligible_at = Some(
            completed_at
                + chrono::Duration::seconds(
                    DISCOVERY_SYNC_DAILY_BACKSTOP_SECONDS + state.context_jitter_seconds,
                ),
        );
        state.next_incremental_reload_eligible_at = Some(next_hash_jittered_bucket(
            completed_at,
            state.incremental_reload_jitter_seconds,
        ));
        let clear_pending_through_sequence = state.inflight_domain_event_sequence;
        let submitted_fingerprint_still_current = state
            .inflight_subject_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint == library_context.fingerprint);
        let has_newer_dirty_events = match (
            state.last_seen_domain_event_sequence,
            clear_pending_through_sequence,
        ) {
            (Some(last_seen), Some(captured)) => last_seen > captured,
            _ => false,
        };
        if !has_newer_dirty_events && submitted_fingerprint_still_current {
            state.dirty_since = None;
            state.dirty_reason_mask = 0;
        }
        state.bootstrap_started_at = None;
        state.bootstrap_quiet_until = None;
        state.backoff_until = None;
        discovery_reset_transient_failure_count(state);
        state.inflight_context_snapshot_run_id = None;
        state.inflight_subject_fingerprint = None;
        state.inflight_domain_event_sequence = None;
        state.updated_at = completed_at;

        let item_count = items.len() as i64;
        let facet_count = facets.len() as i64;
        self.services
            .library
            .discovery
            .commit_discovery_context_snapshot(&DiscoveryContextSnapshotCommit {
                state: state.clone(),
                run: run.clone(),
                submitted_subjects,
                items,
                facets,
                clear_pending_through_sequence,
            })
            .await?;

        match self
            .services
            .library
            .metadata_gateway
            .acknowledge_discovery_context_snapshot(&request_id)
            .await
        {
            Ok(_) => {
                let acknowledged_at = self.runtime.environment.now();
                run.acknowledged_at = Some(acknowledged_at);
                run.updated_at = acknowledged_at;
                self.services
                    .library
                    .discovery
                    .upsert_discovery_sync_run(&run)
                    .await?;
            }
            Err(error) => {
                let failed_at = self.runtime.environment.now();
                run.status = "warning".to_string();
                run.error_text = Some(format!(
                    "local discovery snapshot committed but SMG ack failed: {error}"
                ));
                run.updated_at = failed_at;
                state.backoff_until = Some(discovery_transient_retry_after(state, failed_at));
                state.updated_at = failed_at;
                self.services
                    .library
                    .discovery
                    .upsert_discovery_sync_run(&run)
                    .await?;
            }
        }

        Ok(DiscoveryContextSnapshotRunSummary {
            run_id,
            committed: true,
            smg_request_id: Some(request_id),
            smg_status: run.smg_status.clone(),
            page_count: status_result.page_count,
            item_count,
            facet_count,
        })
    }

    async fn execute_discovery_context_incremental(
        &self,
        trigger_source: JobTriggerSource,
        defaults: &DiscoveryContextDefaults,
        library_context: &DiscoveryLibraryContext,
        pending_changes: &[DiscoveryPendingContextChangeRecord],
        state: &mut DiscoverySyncStateRecord,
        started_at: DateTime<Utc>,
    ) -> AppResult<DiscoveryContextIncrementalRunSummary> {
        let run_id = format!("context-incremental-{}", uuid::Uuid::new_v4());
        let base_generation_id = state.last_success_generation_id.clone().ok_or_else(|| {
            AppError::Validation("discovery incremental requires an active generation".into())
        })?;
        let previous_fingerprint = state.last_subject_fingerprint.clone().ok_or_else(|| {
            AppError::Validation("discovery incremental requires a previous fingerprint".into())
        })?;
        let covered_sequence = pending_changes
            .iter()
            .filter_map(|change| change.last_seen_sequence)
            .max();
        let input = library_context.incremental_changes_input(
            defaults,
            pending_changes,
            &previous_fingerprint,
        )?;

        let mut run = DiscoverySyncRunRecord {
            id: run_id.clone(),
            kind: "context_incremental".to_string(),
            status: "running".to_string(),
            trigger_source: trigger_source.as_str().to_string(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            subject_count: library_context.subjects.len() as i64,
            subject_fingerprint: Some(library_context.fingerprint.clone()),
            previous_subject_fingerprint: Some(previous_fingerprint),
            base_generation_id: Some(base_generation_id.clone()),
            changed_subject_count: pending_changes.len() as i64,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: None,
            discovery_index_watermark: None,
            page_count: None,
            item_count: None,
            facet_count: None,
            acknowledged_at: None,
            error_text: None,
            started_at: Some(started_at),
            completed_at: None,
            created_at: started_at,
            updated_at: started_at,
        };
        self.services
            .library
            .discovery
            .upsert_discovery_sync_run(&run)
            .await?;

        let result = match self
            .services
            .library
            .metadata_gateway
            .discovery_context_changes(&input)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let failed_at = self.runtime.environment.now();
                run.status = "deferred".to_string();
                run.error_text = Some(format!("SMG discovery incremental request failed: {error}"));
                run.completed_at = Some(failed_at);
                run.updated_at = failed_at;
                let retry_at = discovery_transient_retry_after(state, failed_at);
                discovery_schedule_incremental_retry(state, retry_at);
                state.updated_at = failed_at;
                self.services
                    .library
                    .discovery
                    .upsert_discovery_sync_run(&run)
                    .await?;
                return Ok(DiscoveryContextIncrementalRunSummary {
                    run_id,
                    committed: false,
                    smg_status: run.smg_status.clone(),
                    changed_subject_count: run.changed_subject_count,
                    affected_target_count: run.affected_target_count,
                    item_count: 0,
                });
            }
        };

        let completed_at = self.runtime.environment.now();
        run.smg_status = Some(result.status.clone());
        run.discovery_index_watermark =
            non_empty_discovery_string(&result.discovery_index_watermark);
        run.changed_subject_count = i64::from(result.changed_subject_count);
        run.affected_target_count = result.affected_target_keys.len() as i64;
        run.item_count = Some(result.items.len() as i64);
        run.updated_at = completed_at;

        if !discovery_status_is(&result.status, "COMPLETE") {
            run.status = "deferred".to_string();
            run.error_text = Some(format!(
                "SMG discovery incremental status is {}",
                result.status
            ));
            run.completed_at = Some(completed_at);
            let retry_at = discovery_retry_after(completed_at, result.retry_after_seconds);
            discovery_schedule_incremental_retry(state, retry_at);
            state.updated_at = completed_at;
            self.services
                .library
                .discovery
                .upsert_discovery_sync_run(&run)
                .await?;
            return Ok(DiscoveryContextIncrementalRunSummary {
                run_id,
                committed: false,
                smg_status: run.smg_status.clone(),
                changed_subject_count: run.changed_subject_count,
                affected_target_count: run.affected_target_count,
                item_count: 0,
            });
        }

        let subject_provenance = library_context.subject_provenance_by_key();
        let items = incremental_item_records(
            &run_id,
            &base_generation_id,
            &result.items,
            &subject_provenance,
            completed_at,
        )?;
        let mut tombstone_target_keys = result
            .affected_target_keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        tombstone_target_keys.extend(result.items.iter().map(|item| item.target_key.clone()));
        let tombstone_target_keys = tombstone_target_keys.into_iter().collect::<Vec<_>>();

        run.status = "complete".to_string();
        run.item_count = Some(items.len() as i64);
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;

        state.last_subject_fingerprint = Some(library_context.fingerprint.clone());
        state.last_incremental_reload_completed_at = Some(completed_at);
        state.next_incremental_reload_eligible_at = Some(next_hash_jittered_bucket(
            completed_at,
            state.incremental_reload_jitter_seconds,
        ));
        let has_newer_dirty_events = match (state.last_seen_domain_event_sequence, covered_sequence)
        {
            (Some(last_seen), Some(covered)) => last_seen > covered,
            _ => false,
        };
        if !has_newer_dirty_events {
            state.dirty_since = None;
            state.dirty_reason_mask = 0;
        }
        state.backoff_until = None;
        discovery_reset_transient_failure_count(state);
        if let Some(sequence) = covered_sequence {
            state.last_seen_domain_event_sequence = Some(
                state
                    .last_seen_domain_event_sequence
                    .unwrap_or_default()
                    .max(sequence),
            );
        }
        state.updated_at = completed_at;

        let item_count = items.len() as i64;
        self.services
            .library
            .discovery
            .commit_discovery_context_incremental(&DiscoveryContextIncrementalCommit {
                state: state.clone(),
                run: run.clone(),
                items,
                tombstone_target_keys,
                clear_pending_through_sequence: covered_sequence,
            })
            .await?;

        Ok(DiscoveryContextIncrementalRunSummary {
            run_id,
            committed: true,
            smg_status: run.smg_status.clone(),
            changed_subject_count: run.changed_subject_count,
            affected_target_count: run.affected_target_count,
            item_count,
        })
    }

    async fn fail_discovery_sync_run(
        &self,
        run: &mut DiscoverySyncRunRecord,
        error_text: &str,
    ) -> AppResult<()> {
        let now = self.runtime.environment.now();
        run.status = "failed".to_string();
        run.error_text = Some(error_text.to_string());
        run.completed_at = Some(now);
        run.updated_at = now;
        self.services
            .library
            .discovery
            .upsert_discovery_sync_run(run)
            .await
    }

    /// Stable per-instance seed for schedule jitter. Named for its first
    /// consumer; every scheduled job that wants a deterministic offset uses it.
    pub(crate) async fn discovery_scheduler_seed(&self) -> AppResult<String> {
        if let Some(existing) = self
            .read_setting_string_value(SCHEDULER_INSTANCE_ID_KEY, None)
            .await?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return Ok(existing);
        }

        let seed = uuid::Uuid::new_v4().to_string();
        let value_json = serde_json::to_string(&seed)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                SCHEDULER_INSTANCE_ID_KEY,
                None,
                value_json,
                "system",
                None,
            )
            .await?;
        Ok(seed)
    }

    async fn finish_job_run(
        &self,
        mut run: JobRunRecord,
        event_actor: DomainEventActor,
        summary_text: Option<String>,
        summary_json: Option<String>,
        library_scan_progress: Option<LibraryScanSession>,
        status_override: Option<JobRunStatus>,
    ) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = status_override.unwrap_or_else(|| {
            match library_scan_progress
                .as_ref()
                .map(|session| &session.status)
            {
                Some(LibraryScanStatus::Warning) => JobRunStatus::Warning,
                Some(LibraryScanStatus::Canceled) => JobRunStatus::Warning,
                Some(LibraryScanStatus::Failed) => JobRunStatus::Failed,
                _ => JobRunStatus::Completed,
            }
        });
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.summary_text = summary_text;
        run.summary_json = summary_json;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, library_scan_progress))
            .await;
        let payload = if matches!(run.status, JobRunStatus::Failed) {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: run.id.clone(),
                job_key: run.job_key.as_str().to_string(),
                error_text: run.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: run.id.clone(),
                job_key: run.job_key.as_str().to_string(),
                summary_text: run.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                event_actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    async fn fail_job_run(
        &self,
        mut run: JobRunRecord,
        event_actor: DomainEventActor,
        error_text: String,
    ) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = JobRunStatus::Failed;
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.error_text = Some(error_text.clone());
        run.summary_text = Some(format!("Failed: {error_text}"));
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                event_actor,
                updated.id.clone(),
                DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    error_text: updated.error_text.clone(),
                }),
            ))
            .await;
        Ok(())
    }
}

pub async fn start_background_library_refresh_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    if !background_library_refresh_enabled() {
        info!(
            "background library refresh loop is disabled (SCRYER_BACKGROUND_LIBRARY_REFRESH=false)"
        );
        return;
    }

    let scheduler_seed = match app.discovery_scheduler_seed().await {
        Ok(seed) => seed,
        Err(error) => {
            warn!(
                error = %error,
                "background library refresh scheduler seed unavailable; using ephemeral seed"
            );
            uuid::Uuid::new_v4().to_string()
        }
    };

    for job_key in [
        JobKey::BackgroundLibraryRefreshMovies,
        JobKey::BackgroundLibraryRefreshSeries,
        JobKey::BackgroundLibraryRefreshAnime,
    ] {
        let app = app.clone();
        let token = token.child_token();
        let initial_delay_seconds =
            background_library_refresh_initial_delay_seconds(&scheduler_seed, job_key)
                .expect("background refresh job");
        tokio::spawn(async move {
            run_background_library_refresh_worker(app, token, job_key, initial_delay_seconds).await;
        });
    }

    token.cancelled().await;
}

async fn run_background_library_refresh_worker(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    job_key: JobKey,
    initial_delay_seconds: i64,
) {
    let initial_delay = initial_delay_seconds.max(1) as u64;
    let interval_seconds = job_key
        .interval_seconds()
        .unwrap_or(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS)
        .max(1) as u64;
    let initial_next_run_at = Utc::now() + chrono::Duration::seconds(initial_delay as i64);
    app.set_job_next_run_at(job_key, initial_next_run_at).await;

    tokio::select! {
        _ = token.cancelled() => return,
        _ = tokio::time::sleep(std::time::Duration::from_secs(initial_delay)) => {}
    }

    if let Err(error) = app
        .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledStartup)
        .await
    {
        warn!(job_key = job_key.as_str(), error = %error, "startup background job failed");
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    app.set_job_next_run_at(
        job_key,
        Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
    )
    .await;

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = interval.tick() => {
                app.set_job_next_run_at(
                    job_key,
                    Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
                ).await;
                if let Err(error) = app
                    .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledInterval)
                    .await
                {
                    warn!(job_key = job_key.as_str(), error = %error, "scheduled background job failed");
                }
            }
        }
    }
}

fn background_library_refresh_initial_delay_seconds(
    scheduler_seed: &str,
    job_key: JobKey,
) -> Option<i64> {
    let order = match job_key {
        JobKey::BackgroundLibraryRefreshMovies => 0,
        JobKey::BackgroundLibraryRefreshSeries => 1,
        JobKey::BackgroundLibraryRefreshAnime => 2,
        _ => return None,
    };

    let base = scheduler::stable_jitter_offset(
        scheduler_seed,
        "background_library_refresh",
        "base",
        std::time::Duration::from_secs(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS as u64),
    )
    .as_secs() as i64;
    Some(
        (base + order * BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS)
            .rem_euclid(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS),
    )
}

fn job_key_library_facet(job_key: JobKey) -> Option<MediaFacet> {
    match job_key {
        JobKey::LibraryScanMovies | JobKey::BackgroundLibraryRefreshMovies => {
            Some(MediaFacet::Movie)
        }
        JobKey::LibraryScanSeries | JobKey::BackgroundLibraryRefreshSeries => {
            Some(MediaFacet::Series)
        }
        JobKey::LibraryScanAnime | JobKey::BackgroundLibraryRefreshAnime => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn summary_text_from_library_scan(summary: &LibraryScanSummary) -> String {
    format!(
        "Scanned {}, imported {}, skipped {}, unmatched {}",
        summary.scanned, summary.imported, summary.skipped, summary.unmatched
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::runtime::{
        PluginAutoUpdateFailure, PluginAutoUpdateReport, PluginAutoUpdateUpgrade,
    };
    use chrono::TimeZone;

    #[test]
    fn plugin_catalog_refresh_outcome_is_unchanged_when_automation_does_nothing() {
        let outcome = plugin_registry_refresh_outcome(PluginAutoUpdateReport::default());

        assert_eq!(
            outcome.summary_text.as_deref(),
            Some("Plugin catalog refreshed")
        );
        assert!(outcome.summary_json.is_none());
        assert!(outcome.status_override.is_none());
    }

    #[test]
    fn plugin_catalog_refresh_outcome_reports_applied_updates() {
        let outcome = plugin_registry_refresh_outcome(PluginAutoUpdateReport {
            updated: vec![PluginAutoUpdateUpgrade {
                plugin_id: "alpha".to_string(),
                from_version: "1.2.3".to_string(),
                to_version: "1.2.4".to_string(),
            }],
            ..Default::default()
        });

        assert!(outcome.status_override.is_none());
        let summary_text = outcome.summary_text.expect("summary text");
        assert_eq!(
            summary_text,
            "Plugin catalog refreshed; auto-updated 1 plugin(s)"
        );
        let summary: serde_json::Value =
            serde_json::from_str(&outcome.summary_json.expect("summary json"))
                .expect("summary json parses");
        assert_eq!(summary["updated"][0]["pluginId"], "alpha");
        assert_eq!(summary["updated"][0]["fromVersion"], "1.2.3");
        assert_eq!(summary["updated"][0]["toVersion"], "1.2.4");
        assert!(
            summary["failed"]
                .as_array()
                .expect("failed array")
                .is_empty()
        );
        assert!(summary["error"].is_null());
    }

    #[test]
    fn plugin_catalog_refresh_outcome_warns_on_failures() {
        let outcome = plugin_registry_refresh_outcome(PluginAutoUpdateReport {
            failed: vec![PluginAutoUpdateFailure {
                plugin_id: "alpha".to_string(),
                error: "validation: boom".to_string(),
                rolled_back: false,
                rollback_error: Some("restore failed".to_string()),
            }],
            error: Some("catalog unavailable".to_string()),
            ..Default::default()
        });

        assert_eq!(outcome.status_override, Some(JobRunStatus::Warning));
        assert!(
            outcome
                .summary_text
                .expect("summary text")
                .contains("2 failed")
        );
        let summary: serde_json::Value =
            serde_json::from_str(&outcome.summary_json.expect("summary json"))
                .expect("summary json parses");
        assert_eq!(summary["failed"][0]["pluginId"], "alpha");
        assert_eq!(summary["failed"][0]["error"], "validation: boom");
        assert_eq!(summary["failed"][0]["rolledBack"], false);
        assert_eq!(summary["failed"][0]["rollbackError"], "restore failed");
        assert_eq!(summary["error"], "catalog unavailable");
    }

    #[test]
    fn discovery_next_run_ignores_the_incremental_bucket_before_the_first_snapshot() {
        // Before a personalized snapshot exists there is nothing to reload
        // incrementally, so the seed-jittered incremental bucket must not be
        // the wake reason — otherwise it can pre-empt the first-snapshot gate
        // the state scheduled. Once a snapshot exists it is a normal candidate.
        let now = Utc.timestamp_opt(10_000, 0).unwrap();
        let candidates = |incremental_reload_possible: bool| DiscoveryNextRunCandidates {
            next_incremental: now + chrono::Duration::seconds(37),
            incremental_reload_possible,
            next_context: now + chrono::Duration::seconds(358),
            next_public: now + chrono::Duration::days(1),
            bootstrap_quiet_until: None,
            backoff_until: None,
            scan_blocked_retry_at: None,
            pending_changes_quiet_at: None,
        };

        assert_eq!(
            discovery_next_run_at(now, candidates(false)),
            now + chrono::Duration::seconds(358),
            "first snapshot pending: the context gate wins even though the incremental bucket is earlier"
        );
        assert_eq!(
            discovery_next_run_at(now, candidates(true)),
            now + chrono::Duration::seconds(37),
            "after the first snapshot the incremental bucket is a normal wake candidate"
        );
    }

    #[test]
    fn background_library_refresh_initial_delays_are_staggered_by_facet() {
        let scheduler_seed = "stable-scheduler-seed";
        let movie = background_library_refresh_initial_delay_seconds(
            scheduler_seed,
            JobKey::BackgroundLibraryRefreshMovies,
        )
        .expect("movie delay");
        let series = background_library_refresh_initial_delay_seconds(
            scheduler_seed,
            JobKey::BackgroundLibraryRefreshSeries,
        )
        .expect("series delay");
        let anime = background_library_refresh_initial_delay_seconds(
            scheduler_seed,
            JobKey::BackgroundLibraryRefreshAnime,
        )
        .expect("anime delay");

        assert!((0..BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS).contains(&movie));
        assert!((0..BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS).contains(&series));
        assert!((0..BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS).contains(&anime));
        assert_eq!(
            (series - movie).rem_euclid(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS),
            BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS
        );
        assert_eq!(
            (anime - series).rem_euclid(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS),
            BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS
        );
    }

    #[test]
    fn discovery_sync_job_metadata_matches_dynamic_evaluator_contract() {
        assert_eq!(JobKey::DiscoverySync.as_str(), "discovery_sync");
        assert_eq!(JobKey::parse("discovery_sync"), Some(JobKey::DiscoverySync));
        assert_eq!(
            JobKey::DiscoverySync.schedule_kind(),
            JobScheduleKind::StartupAndInterval
        );
        assert_eq!(
            JobKey::DiscoverySync.schedule_description(),
            "Dynamic discovery evaluator with daily backstop"
        );
        assert_eq!(JobKey::DiscoverySync.interval_seconds(), Some(24 * 3600));
        assert_eq!(JobKey::DiscoverySync.initial_delay_seconds(), Some(30 * 60));
        assert!(ALL_JOB_KEYS.contains(&JobKey::DiscoverySync));
    }

    #[test]
    fn discovery_incremental_bucket_uses_next_jitter_slot_after_offset_passes() {
        let jitter = 2 * 60 * 60;
        let before_jitter = chrono::DateTime::from_timestamp(60 * 60, 0).expect("valid time");
        let after_jitter = chrono::DateTime::from_timestamp(3 * 60 * 60, 0).expect("valid time");

        assert_eq!(
            next_hash_jittered_bucket(before_jitter, jitter).timestamp(),
            jitter
        );
        assert_eq!(
            next_hash_jittered_bucket(after_jitter, jitter).timestamp(),
            DISCOVERY_SYNC_INCREMENTAL_CADENCE_SECONDS + jitter
        );
    }

    #[test]
    fn discovery_jitter_is_stable_and_stream_specific() {
        let first = discovery_jitter_seconds("instance-a", "incremental_reload", 4 * 60 * 60);
        let second = discovery_jitter_seconds("instance-a", "incremental_reload", 4 * 60 * 60);
        let different_stream = discovery_jitter_seconds("instance-a", "public_feed", 4 * 60 * 60);

        assert_eq!(first, second);
        assert_ne!(first, different_stream);
        assert!((0..4 * 60 * 60).contains(&first));
    }
}
