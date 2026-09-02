//! Operation runner: drives an operation through its states, checkpointing per
//! title, stopping at safe cancel points, and resuming across process restarts
//! (FR-030–033, FR-089, FR-092).
//!
//! Execution order per operation (FR-031): validate → fingerprinted preview →
//! explicit confirmation → move one title at a time → verify each copy at the
//! configured depth → apply destination permissions → flip catalog ownership →
//! recycle or preserve redundant sources → remove only empty source directories
//! → finalize root or source-title removal.
//!
//! The preview and confirmation halves live in [`crate::location::preview`]; the
//! runner picks up at a confirmed, persisted operation and owns everything from
//! there.
//!
//! # What is a seam and why
//!
//! The runner is workflow-agnostic on purpose — one state machine behind all six
//! operation types (D5). Everything workflow- or filesystem-specific enters
//! through a trait:
//!
//! | Seam | Supplied by |
//! |---|---|
//! | [`TitleFileMover`] | T014's verified copier (`location::verify`), adapted per workflow |
//! | [`TitleAdmissionCheck`] | the workflow's staleness / blocked-title rules (FR-089, FR-086) |
//! | [`TitleReconciler`] | the workflow's catalog flip and source cleanup (FR-031) |
//!
//! # What stops a run, and what does not
//!
//! Three things end a run early, and they are deliberately different:
//!
//! - **A cancel** stops at the next safe point and settles nothing that has not
//!   finished. There are two such points per title — the boundary before it, and
//!   the boundary before each of its files — because a single title can be a
//!   whole evening's copying and a cancel that only lands between titles is not
//!   a cancel the user can feel (FR-092).
//! - **A stale plan** stops the whole operation: the plan no longer describes
//!   reality, so continuing would carry out instructions nobody confirmed
//!   (FR-089).
//! - **A title failing** stops *that title*. The rest of the plan is
//!   independent work the user asked for, and one unreachable disk should not
//!   cost them the other forty titles. The operation still ends `Failed`, with
//!   a detail saying how many titles failed of how many, and each failure's
//!   reason on its own checkpoint.
//!
//! Before a title is failed, a transient I/O error gets a bounded retry
//! ([`FileMoveRetry`]) — from a clean partial, so a half-written attempt is
//! never resumed. A verification mismatch never is: that is evidence about the
//! bytes, not a hiccup (FR-044).
//!
//! # Progress while one file takes hours
//!
//! Byte counters that only move when a file finishes leave a one-file 80 GB
//! title looking frozen — and leave `updated_at` looking abandoned, which is
//! what a staleness heuristic reads. So the runner pulses every
//! [`PROGRESS_PULSE_INTERVAL`] while a file is in flight: the operation row's
//! bytes and `updated_at`, the title checkpoint's bytes, and whatever
//! [`OperationProgressObserver`] is mirroring it. In-flight bytes are reported,
//! never settled: every write that settles a title carries proven bytes only,
//! which is what a resume reads back.
//!
//! # The stale-vs-resumable rule (FR-089)
//!
//! Staleness is asked **per title, before that title starts**, and only for
//! titles the operation has not processed yet. The runner hands the check the
//! destination paths it has already verified for that title, so the check can
//! tell its own interrupted work apart from a foreign change: expected partial
//! destination state is resumable, a changed catalog input or a changed
//! not-yet-processed source is stale. [`crate::location::preview::PlanInputChange`]
//! is the shared vocabulary for that decision; the runner never hardcodes one
//! workflow's rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::location::classify::TitleLocationClass;
use crate::location::model::{
    FileVerificationRecord, LocationOperation, LocationOperationCounters, LocationOperationState,
    TitleCheckpoint, TitleCheckpointPlacement, TitleCheckpointState, VerificationDepth,
};
use crate::location::ownership_guard::OwnedEntity;
use crate::location::verify::{CopyProgress, FileVerificationIdentity, VerifiedFile};
use crate::ports::{
    LocationOperationProgress, LocationOperationRepository, LocationOwnershipOutcome,
};
use crate::stored_paths::path_to_stored_string;
use crate::{AppError, AppResult};

/// Where a resumed operation picks up. Titles at or before `last_settled_sequence`
/// are never reprocessed, so verified work is never repeated (FR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeCursor {
    pub operation_id: String,
    /// Highest checkpoint sequence whose title reached a settled state.
    pub last_settled_sequence: i64,
}

/// Why a running operation stopped short of completion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The user cancelled; the runner stops at the next safe title checkpoint.
    UserCanceled,
    /// Inputs the plan had not yet processed changed underneath the operation,
    /// so the plan is stale and a new preview is required (FR-089). Expected
    /// partial destination state from an interrupted copy is *not* stale.
    StalePlan,
    /// A verification, filesystem, or catalog error stopped the run.
    Error,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserCanceled => "user_canceled",
            Self::StalePlan => "stale_plan",
            Self::Error => "error",
        }
    }
}

/// One file of planned work inside a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// Tracked media file, or `None` for a companion asset.
    pub media_file_id: Option<String>,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub size_bytes: u64,
}

impl PlannedFile {
    /// The destination in the form verification records store it, which is what
    /// resume compares against.
    pub fn stored_destination(&self) -> String {
        path_to_stored_string(&self.destination_path)
    }
}

/// What the confirmed plan already decided this title's outcome would be, for
/// the FR-091 counters Activity shows.
///
/// These are plan facts, not run facts: the collision engine decided at preview
/// time which files are proven duplicates and which have to be renamed, and the
/// title either carries out those decisions or does not finish at all. Counting
/// them off the plan is what lets a resumed run report the same totals as the
/// run it resumed, without a per-file counter table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TitleOutcomeCounts {
    /// Files or companion assets recycled as proven duplicates (FR-073).
    pub dedups: i64,
    /// Files or companion assets renamed to avoid a collision (FR-074/075).
    pub renames: i64,
}

/// One title of planned work. Titles are the ordering, cancel, and resume
/// granularity of the whole subsystem (FR-031, FR-092).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTitle {
    pub title_id: String,
    /// Position in the confirmed plan; the runner walks ascending.
    pub sequence: i64,
    pub classification: Option<TitleLocationClass>,
    pub placement: TitleCheckpointPlacement,
    pub files: Vec<PlannedFile>,
    /// Dedup and rename decisions this title carries out, counted only once the
    /// title actually finishes.
    pub outcomes: TitleOutcomeCounts,
}

impl PlannedTitle {
    pub fn bytes_total(&self) -> i64 {
        self.files.iter().fold(0_i64, |total, file| {
            total.saturating_add(file.size_bytes as i64)
        })
    }
}

/// Counts the selection's classification fixed before the operation started, and
/// which no amount of running changes (FR-015, FR-091).
///
/// Titles classified as no-ops or as needing a decision never reach
/// [`OperationWorkPlan::titles`] — there is nothing for the runner to do with
/// them — but Activity still has to report them, so the plan carries their
/// counts rather than losing them at the plan boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassifiedTitleBaseline {
    /// Titles the preview classified as already at their destination.
    pub no_ops: i64,
    /// Titles the preview could not resolve into work: an outstanding user
    /// decision (FR-016, FR-086) or an incompatible destination (FR-017).
    pub unresolved: i64,
}

/// The confirmed plan as the runner walks it: the workflow-specific planner
/// (T031 and friends) reduces its own plan to this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationWorkPlan {
    pub titles: Vec<PlannedTitle>,
    /// What classification decided before any title was queued.
    pub baseline: ClassifiedTitleBaseline,
    /// Titles the operation must own even though they carry no instructions
    /// (FR-084). Two cases: a root change owns every title assigned to the root
    /// (a catalog-only or blocked title produces no work to walk), and any
    /// workflow that merges owns the *destination* title it merges into — that
    /// row is rewritten by Groups 1–5 and must not be deleted, renamed, or moved
    /// out from under the merge.
    pub additional_owned_titles: Vec<String>,
    /// Roots the operation must own beyond the operation row's own
    /// `source_root_id`/`destination_root_id` (FR-084).
    ///
    /// The operation row carries a single source root, and it is `None` the
    /// moment a selection spans more than one — which is exactly the bulk move
    /// and cross-library transfer case where *several* source roots are being
    /// read from and pruned. Every root any planned title leaves or arrives on
    /// is listed here, so a root reconfiguration or a scan of any of them is
    /// refused for the operation's duration.
    pub additional_owned_roots: Vec<String>,
}

impl OperationWorkPlan {
    pub fn new(titles: Vec<PlannedTitle>) -> Self {
        let mut titles = titles;
        titles.sort_by_key(|title| title.sequence);
        Self {
            titles,
            baseline: ClassifiedTitleBaseline::default(),
            additional_owned_titles: Vec::new(),
            additional_owned_roots: Vec::new(),
        }
    }

    /// Claim these titles for the operation's duration alongside the ones in
    /// the plan (FR-084).
    pub fn with_additional_owned_titles(mut self, title_ids: Vec<String>) -> Self {
        self.additional_owned_titles = title_ids;
        self
    }

    /// Claim these roots for the operation's duration alongside the operation
    /// row's own source and destination roots (FR-084).
    pub fn with_additional_owned_roots(mut self, root_ids: Vec<String>) -> Self {
        self.additional_owned_roots = root_ids;
        self
    }

    /// Carry the selection's classification counts into the run, so the no-op
    /// and unresolved titles the plan dropped still reach Activity.
    pub fn with_baseline(mut self, baseline: ClassifiedTitleBaseline) -> Self {
        self.baseline = baseline;
        self
    }

    pub fn files_total(&self) -> i64 {
        self.titles
            .iter()
            .map(|title| title.files.len() as i64)
            .sum()
    }

    pub fn bytes_total(&self) -> i64 {
        self.titles.iter().fold(0_i64, |total, title| {
            total.saturating_add(title.bytes_total())
        })
    }
}

/// One file's move + verification, as the runner asks for it.
#[derive(Debug, Clone, Copy)]
pub struct FileMoveRequest<'a> {
    pub operation_id: &'a str,
    pub title: &'a PlannedTitle,
    pub file: &'a PlannedFile,
    /// The operation's configured depth (FR-042). The mover may only ever apply
    /// the quick floor as a recorded fallback, never a silent downgrade.
    pub depth: VerificationDepth,
    /// Where the mover reports bytes as it writes them, so a single multi-hour
    /// file is not an unmoving progress bar (FR-091). A mover that copies
    /// nothing — the same-filesystem rename — reports nothing.
    pub progress: &'a CopyProgress,
}

/// Moves and proves one file.
///
/// The move and the proof are one seam because they are one streaming pass (D2):
/// the CRC that a full read-back is compared against only exists because the
/// copy computed it. T014's `VerifiedCopier` is the production implementation.
#[async_trait]
pub trait TitleFileMover: Send + Sync {
    async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile>;
}

/// Whether a title may still be processed as planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleAdmission {
    /// Nothing relevant changed; run the title.
    Proceed,
    /// A catalog input or an unprocessed item changed underneath the plan: the
    /// operation stops and the user must re-preview (FR-089).
    Stale(String),
    /// The title cannot enter the operation right now — an active download or
    /// import, an unresolved classification, unmapped merge records (FR-086,
    /// FR-016). The operation continues with the remaining titles.
    Blocked(String),
    /// The title needs no work: a no-op, or one the user removed from the
    /// selection after confirmation.
    Skip(String),
}

/// What the admission check gets to look at.
#[derive(Debug, Clone, Copy)]
pub struct TitleAdmissionContext<'a> {
    pub operation: &'a LocationOperation,
    pub title: &'a PlannedTitle,
    /// Destination paths this operation has already verified for this title.
    ///
    /// This is the FR-089 carve-out made concrete: content at these paths is the
    /// operation's own finished work, and content partially written at planned
    /// destinations is its own interrupted work. Neither is evidence that the
    /// plan went stale.
    pub verified_destinations: &'a BTreeSet<String>,
}

/// The workflow's staleness and blocked-title rules.
///
/// Asked once per title, immediately before that title starts, and never for a
/// title that already settled — the scope FR-089 defines.
#[async_trait]
pub trait TitleAdmissionCheck: Send + Sync {
    async fn admit_title(&self, context: TitleAdmissionContext<'_>) -> AppResult<TitleAdmission>;
}

/// An admission check that admits everything: the right default for workflows
/// with no catalog inputs to go stale (catalog-only reassignment).
#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysAdmit;

#[async_trait]
impl TitleAdmissionCheck for AlwaysAdmit {
    async fn admit_title(&self, _context: TitleAdmissionContext<'_>) -> AppResult<TitleAdmission> {
        Ok(TitleAdmission::Proceed)
    }
}

/// What a title's catalog or cleanup step wants the user to see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleStepOutcome {
    /// Warnings that make the title — and the operation — finish with warnings
    /// rather than silently (C3): preserve-instead-of-recycle, collision
    /// renames, hardlink notes.
    pub warnings: Vec<String>,
}

impl TitleStepOutcome {
    pub fn clean() -> Self {
        Self::default()
    }

    pub fn warned(warning: impl Into<String>) -> Self {
        Self {
            warnings: vec![warning.into()],
        }
    }
}

/// The workflow's catalog and cleanup work for one title.
///
/// Both steps run only after every planned file for the title is verified — the
/// FR-031 ordering the runner enforces, not something an implementation has to
/// remember.
#[async_trait]
pub trait TitleReconciler: Send + Sync {
    /// Flip catalog ownership, apply merges, resolve roles (FR-031).
    async fn reconcile_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome>;

    /// Recycle or preserve redundant sources and remove empty source
    /// directories (FR-031, FR-044).
    async fn clean_up_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        let _ = (operation, title);
        Ok(TitleStepOutcome::clean())
    }
}

/// How often a running operation writes a progress pulse while a single file is
/// still being copied and proven.
///
/// Wall clock, not chunks: a 1 MiB copy chunk is milliseconds on a local disk
/// and this must not turn a move into a write storm. Five seconds is fast
/// enough that the Activity bar visibly advances and that `updated_at` stays
/// obviously fresh, and slow enough to be free against any real copy.
pub const PROGRESS_PULSE_INTERVAL: Duration = Duration::from_secs(5);

/// How a file move is retried when the filesystem answers with something
/// transient.
///
/// A verification mismatch is deliberately *not* covered: a mismatch is
/// evidence about the bytes, not a hiccup, and retrying it would turn a caught
/// corruption into a loop (FR-044, C4). Only an I/O-shaped error
/// ([`AppError::Repository`]) — a dropped network mount, a momentary ENOSPC, a
/// blocked descriptor — is worth a second look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMoveRetry {
    /// Total attempts, including the first. `1` disables retrying.
    pub attempts: u32,
    /// Delay before the second attempt; doubled for each attempt after that.
    pub base_delay: Duration,
}

impl Default for FileMoveRetry {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_millis(500),
        }
    }
}

impl FileMoveRetry {
    /// No retrying at all, for a caller that wants a failure to surface
    /// immediately.
    pub fn none() -> Self {
        Self {
            attempts: 1,
            base_delay: Duration::ZERO,
        }
    }

    fn delay_before(&self, next_attempt: u32) -> Duration {
        let doublings = next_attempt.saturating_sub(2).min(16);
        self.base_delay
            .saturating_mul(2_u32.saturating_pow(doublings))
    }
}

/// Whether an error is the transient, I/O-shaped kind a retry can help with.
fn move_error_is_transient(error: &AppError) -> bool {
    matches!(error, AppError::Repository(_))
}

/// A running operation's progress, as somebody outside the operation store
/// wants to mirror it.
#[derive(Debug, Clone, Copy)]
pub struct OperationProgressSnapshot<'a> {
    pub operation_id: &'a str,
    pub state: LocationOperationState,
    pub counters: LocationOperationCounters,
}

/// Mirrors a running operation's progress somewhere else — Activity's job run,
/// in production (FR-091).
///
/// Called on the throttled pulse, never per chunk and never per file, and its
/// failures are the observer's own problem: a move must not stop because a
/// progress mirror could not be written.
#[async_trait]
pub trait OperationProgressObserver: Send + Sync {
    async fn observe(&self, snapshot: OperationProgressSnapshot<'_>);
}

/// How one run of the operation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRunOutcome {
    pub operation_id: String,
    pub state: LocationOperationState,
    /// Absent when the operation ran to completion.
    pub stop_reason: Option<StopReason>,
    pub counters: LocationOperationCounters,
    pub cursor: ResumeCursor,
    /// Warnings gathered across the titles this run processed.
    pub warnings: Vec<String>,
    pub detail: Option<String>,
}

impl OperationRunOutcome {
    pub fn completed(&self) -> bool {
        matches!(
            self.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        )
    }
}

/// Records verification rows through the operation store, so T014's copier can
/// persist without knowing about repositories.
pub struct StoreVerificationRecorder<'a> {
    store: &'a dyn LocationOperationRepository,
}

impl<'a> StoreVerificationRecorder<'a> {
    pub fn new(store: &'a dyn LocationOperationRepository) -> Self {
        Self { store }
    }
}

#[async_trait]
impl crate::location::verify::FileVerificationRecorder for StoreVerificationRecorder<'_> {
    async fn record_file_verification(&self, record: FileVerificationRecord) -> AppResult<()> {
        self.store.record_location_file_verification(&record).await
    }
}

/// One operation-scoped act that has to happen after the last title and before
/// the operation is reported finished.
///
/// # Why this is a seam and not a step in `run_title`
///
/// Almost everything a location operation does is per-title, which is why the
/// runner is written per-title. A **root change** has exactly one thing that is
/// not: retiring the source location and flipping the root's configured path,
/// which FR-087 says may only happen once every title's recycling has completed.
///
/// Doing that after `run` returned would be the obvious shortcut and it is
/// wrong: `run` writes the terminal state as its last act, so the operation
/// would be readable as `completed` — to Activity, to a watching client, to a
/// test — while the root still pointed at the retired path. It would also run
/// after the ownership claims were released (FR-084), leaving the flip
/// unprotected. Both problems disappear when the epilogue is part of the run.
#[async_trait]
pub trait OperationEpilogue: Send + Sync {
    /// Called once, after every title has settled cleanly and before the
    /// terminal state is written. Never called for a run that failed, was
    /// canceled, or stopped for resume — those have not finished the work the
    /// epilogue depends on, and the terminal run that eventually follows will
    /// call it instead.
    ///
    /// Returned warnings join the operation's own, which is what turns a
    /// `Completed` into a `CompletedWithWarnings`. An `Err` fails the
    /// operation: the epilogue is work the user asked for, not bookkeeping.
    async fn finish_operation(&self, operation: &LocationOperation) -> AppResult<Vec<String>>;
}

/// The operation runner (D5).
pub struct LocationOperationRunner<'a> {
    store: &'a dyn LocationOperationRepository,
    mover: &'a dyn TitleFileMover,
    admission: &'a dyn TitleAdmissionCheck,
    reconciler: &'a dyn TitleReconciler,
    registry: Option<&'a crate::location::ownership_guard::LocationOwnershipRegistry>,
    observer: Option<&'a dyn OperationProgressObserver>,
    epilogue: Option<&'a dyn OperationEpilogue>,
    pulse_interval: Duration,
    retry: FileMoveRetry,
}

impl<'a> LocationOperationRunner<'a> {
    pub fn new(
        store: &'a dyn LocationOperationRepository,
        mover: &'a dyn TitleFileMover,
        admission: &'a dyn TitleAdmissionCheck,
        reconciler: &'a dyn TitleReconciler,
    ) -> Self {
        Self {
            store,
            mover,
            admission,
            reconciler,
            registry: None,
            observer: None,
            epilogue: None,
            pulse_interval: PROGRESS_PULSE_INTERVAL,
            retry: FileMoveRetry::default(),
        }
    }

    /// Bind the operation-scoped act that runs after the last title (FR-087).
    pub fn with_epilogue(mut self, epilogue: &'a dyn OperationEpilogue) -> Self {
        self.epilogue = Some(epilogue);
        self
    }

    /// Mirrors this run's claims into the in-process guard registry (D7), so a
    /// same-process scan or import is refused without a query.
    pub fn with_ownership_registry(
        mut self,
        registry: &'a crate::location::ownership_guard::LocationOwnershipRegistry,
    ) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Mirror each progress pulse somewhere outside the operation store —
    /// Activity's job run (FR-091).
    pub fn with_progress_observer(mut self, observer: &'a dyn OperationProgressObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Override the pulse cadence. Production keeps
    /// [`PROGRESS_PULSE_INTERVAL`]; a test that wants to watch a pulse land
    /// shrinks it.
    pub fn with_progress_pulse_interval(mut self, interval: Duration) -> Self {
        self.pulse_interval = interval;
        self
    }

    /// Override the per-file retry policy.
    pub fn with_file_move_retry(mut self, retry: FileMoveRetry) -> Self {
        self.retry = retry;
        self
    }

    /// Operations a restart should pick back up (FR-033). Non-terminal
    /// operations only; each still holds its ownership claims.
    pub async fn resumable_operations(
        store: &dyn LocationOperationRepository,
    ) -> AppResult<Vec<LocationOperation>> {
        store.list_active_location_operations().await
    }

    /// Runs — or resumes — one operation to a terminal state or a safe stop.
    ///
    /// Resuming is not a separate path: the runner always reads the persisted
    /// checkpoints first and skips what already settled, so a fresh start is
    /// just the case where there is nothing to skip (FR-033, FR-092).
    pub async fn run(
        &self,
        operation_id: &str,
        plan: &OperationWorkPlan,
    ) -> AppResult<OperationRunOutcome> {
        let operation = self
            .store
            .get_location_operation(operation_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("location operation {operation_id}")))?;

        if operation.state.is_terminal() {
            let checkpoints = self.load_checkpoints(operation_id).await?;
            let progress = RunProgress::resume(plan, &checkpoints);
            return Ok(OperationRunOutcome {
                operation_id: operation_id.to_string(),
                state: operation.state,
                stop_reason: None,
                counters: progress.counters(plan),
                cursor: progress.cursor(operation_id),
                warnings: Vec::new(),
                detail: operation.detail.clone(),
            });
        }

        let checkpoints = self.load_checkpoints(operation_id).await?;
        let mut progress = RunProgress::resume(plan, &checkpoints);

        // Preparing: ownership before any byte moves, so nothing else can touch
        // the titles or roots this operation is about to change (FR-084).
        self.write_progress(
            &operation,
            LocationOperationState::Preparing,
            &progress,
            plan,
            None,
            true,
        )
        .await?;

        let entities = owned_entities(&operation, plan);
        match self
            .store
            .claim_location_operation_ownership(operation_id, &entities)
            .await?
        {
            LocationOwnershipOutcome::Claimed => {
                if let Some(registry) = self.registry {
                    registry.claim_all(operation_id, &entities);
                }
            }
            LocationOwnershipOutcome::Conflict(conflicts) => {
                let detail = describe_ownership_conflicts(&conflicts);
                return self
                    .finish(
                        &operation,
                        plan,
                        &progress,
                        LocationOperationState::Failed,
                        Some(StopReason::Error),
                        Some(detail),
                    )
                    .await;
            }
        }

        // Failures are contained to the title that hit them: the remaining
        // titles are independent work the user asked for, and abandoning them
        // because one title's disk went away would cost a whole re-preview to
        // recover. The operation still ends Failed, and terminally so — a
        // retry is a fresh preview, which converges because completed titles
        // classify as no-ops and verified copies dedup against their persisted
        // hashes.
        let mut failures: Vec<String> = Vec::new();

        for title in &plan.titles {
            if progress.is_settled(&title.title_id) {
                continue;
            }

            // Safe cancel point: the boundary between titles, before any of this
            // title's content is touched (FR-092).
            if self
                .store
                .location_operation_cancel_requested(operation_id)
                .await?
            {
                return self
                    .finish(
                        &operation,
                        plan,
                        &progress,
                        LocationOperationState::Canceled,
                        Some(StopReason::UserCanceled),
                        Some(
                            "canceled at a title checkpoint; completed titles are unchanged"
                                .to_string(),
                        ),
                    )
                    .await;
            }

            let verified_destinations = self
                .store
                .verified_destination_paths(operation_id, &title.title_id)
                .await?;

            let admission = self
                .admission
                .admit_title(TitleAdmissionContext {
                    operation: &operation,
                    title,
                    verified_destinations: &verified_destinations,
                })
                .await?;

            match admission {
                TitleAdmission::Proceed => {}
                TitleAdmission::Stale(reason) => {
                    return self
                        .finish(
                            &operation,
                            plan,
                            &progress,
                            LocationOperationState::Failed,
                            Some(StopReason::StalePlan),
                            Some(reason),
                        )
                        .await;
                }
                TitleAdmission::Blocked(reason) => {
                    self.settle_title(
                        &operation,
                        title,
                        TitleCheckpointState::Blocked,
                        Some(reason.clone()),
                        &mut progress,
                        plan,
                    )
                    .await?;
                    progress.warnings.push(reason);
                    continue;
                }
                TitleAdmission::Skip(reason) => {
                    self.settle_title(
                        &operation,
                        title,
                        TitleCheckpointState::Skipped,
                        Some(reason),
                        &mut progress,
                        plan,
                    )
                    .await?;
                    continue;
                }
            }

            match self
                .run_title(
                    &operation,
                    title,
                    &verified_destinations,
                    &mut progress,
                    plan,
                )
                .await
            {
                Ok(TitleRunOutcome::Finished(warnings)) => {
                    let state = if warnings.is_empty() {
                        TitleCheckpointState::Completed
                    } else {
                        TitleCheckpointState::CompletedWithWarnings
                    };
                    let detail = (!warnings.is_empty()).then(|| warnings.join("; "));
                    self.settle_title(&operation, title, state, detail, &mut progress, plan)
                        .await?;
                    progress.warnings.extend(warnings);
                }
                // A cancel landed between two of this title's files. The title
                // is deliberately left unsettled — its finished files are
                // recorded and will dedup on a later run, but the title itself
                // has not done what the plan asked (FR-092).
                Ok(TitleRunOutcome::Canceled) => {
                    return self
                        .finish(
                            &operation,
                            plan,
                            &progress,
                            LocationOperationState::Canceled,
                            Some(StopReason::UserCanceled),
                            Some(format!(
                                "canceled while title {} was moving; its verified files are recorded and completed titles are unchanged",
                                title.title_id
                            )),
                        )
                        .await;
                }
                Err(error) => {
                    let detail = error.to_string();
                    tracing::warn!(
                        operation_id = %operation.id,
                        title_id = %title.title_id,
                        error = %error,
                        "a title failed; the operation continues with the remaining titles"
                    );
                    self.settle_title(
                        &operation,
                        title,
                        TitleCheckpointState::Failed,
                        Some(detail.clone()),
                        &mut progress,
                        plan,
                    )
                    .await?;
                    failures.push(format!("{}: {detail}", title.title_id));
                }
            }
        }

        if !failures.is_empty() {
            return self
                .finish(
                    &operation,
                    plan,
                    &progress,
                    LocationOperationState::Failed,
                    Some(StopReason::Error),
                    Some(describe_title_failures(&failures, plan.titles.len())),
                )
                .await;
        }

        // The one act that belongs to the operation rather than to a title, for
        // the workflows that have one — a root change retiring its source
        // location and flipping the root's path (FR-087). It runs here, inside
        // the run, so the operation is never readable as finished before it is,
        // and so it still holds its ownership claims while it works (FR-084).
        if let Some(epilogue) = self.epilogue {
            match epilogue.finish_operation(&operation).await {
                Ok(warnings) => progress.warnings.extend(warnings),
                Err(error) => {
                    return self
                        .finish(
                            &operation,
                            plan,
                            &progress,
                            LocationOperationState::Failed,
                            Some(StopReason::Error),
                            Some(error.to_string()),
                        )
                        .await;
                }
            }
        }

        // A cancel that arrives after the last title still lands as a completed
        // operation: there is no work left to stop.
        let final_state = if progress.warnings.is_empty() {
            LocationOperationState::Completed
        } else {
            LocationOperationState::CompletedWithWarnings
        };
        let detail = (!progress.warnings.is_empty()).then(|| progress.warnings.join("; "));
        self.finish(&operation, plan, &progress, final_state, None, detail)
            .await
    }

    /// One title, in FR-031 order: move → verify → reconcile → clean up.
    async fn run_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        verified_destinations: &BTreeSet<String>,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) -> AppResult<TitleRunOutcome> {
        let mut warnings = Vec::new();

        self.write_title_checkpoint(
            operation,
            title,
            TitleCheckpointState::Moving,
            None,
            progress,
        )
        .await?;
        self.write_progress(
            operation,
            LocationOperationState::Moving,
            progress,
            plan,
            None,
            false,
        )
        .await?;

        for file in &title.files {
            let stored_destination = file.stored_destination();
            if verified_destinations.contains(&stored_destination) {
                // Already proven by an earlier run: never copied or verified
                // twice (FR-092).
                progress.note_file_done(&title.title_id, file);
                continue;
            }

            // Second safe cancel point: between two files of one title. A move
            // of one 80 GB title would otherwise ignore a cancel for hours.
            // Nothing is settled here — the title has not finished its work —
            // so a later run picks it up and dedups the files already proven.
            if self
                .store
                .location_operation_cancel_requested(&operation.id)
                .await?
            {
                return Ok(TitleRunOutcome::Canceled);
            }

            let verified = self
                .move_file_with_retry(operation, title, file, progress, plan)
                .await?;

            if verified.depth.fell_back {
                progress.verification_fallbacks += 1;
                if let Some(detail) = verified.detail.clone() {
                    warnings.push(detail);
                }
            }

            let outcome = verified.outcome;
            let detail = verified.detail.clone();

            let record = verified.into_record(
                FileVerificationIdentity {
                    operation_id: &operation.id,
                    title_id: &title.title_id,
                    media_file_id: file.media_file_id.as_deref(),
                },
                Utc::now(),
            );
            self.store
                .record_location_file_verification(&record)
                .await?;

            if !outcome.permits_source_removal() {
                return Err(AppError::Validation(format!(
                    "verification of {} failed ({}){}",
                    file.destination_path.display(),
                    outcome.as_str(),
                    detail
                        .map(|detail| format!(": {detail}"))
                        .unwrap_or_default()
                )));
            }

            progress.note_file_done(&title.title_id, file);
            self.write_title_checkpoint(
                operation,
                title,
                TitleCheckpointState::Moving,
                None,
                progress,
            )
            .await?;
        }

        // Verifying is the completeness gate, not a second pass: every planned
        // file must have a verified destination before any catalog row moves
        // (FR-031, FR-044).
        self.write_title_checkpoint(
            operation,
            title,
            TitleCheckpointState::Verifying,
            None,
            progress,
        )
        .await?;
        self.write_progress(
            operation,
            LocationOperationState::Verifying,
            progress,
            plan,
            None,
            false,
        )
        .await?;
        let verified_now = self
            .store
            .verified_destination_paths(&operation.id, &title.title_id)
            .await?;
        for file in &title.files {
            if !verified_now.contains(&file.stored_destination()) {
                return Err(AppError::Validation(format!(
                    "{} has no verification record, so the catalog must not be updated for title {}",
                    file.destination_path.display(),
                    title.title_id
                )));
            }
        }

        self.write_title_checkpoint(
            operation,
            title,
            TitleCheckpointState::Reconciling,
            None,
            progress,
        )
        .await?;
        self.write_progress(
            operation,
            LocationOperationState::Reconciling,
            progress,
            plan,
            None,
            false,
        )
        .await?;
        warnings.extend(
            self.reconciler
                .reconcile_title(operation, title)
                .await?
                .warnings,
        );

        self.write_title_checkpoint(
            operation,
            title,
            TitleCheckpointState::CleaningUp,
            None,
            progress,
        )
        .await?;
        self.write_progress(
            operation,
            LocationOperationState::CleaningUp,
            progress,
            plan,
            None,
            false,
        )
        .await?;
        warnings.extend(
            self.reconciler
                .clean_up_title(operation, title)
                .await?
                .warnings,
        );

        Ok(TitleRunOutcome::Finished(warnings))
    }

    /// Move one file, retrying the transient failures and pulsing progress
    /// while it runs.
    async fn move_file_with_retry(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        file: &PlannedFile,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) -> AppResult<VerifiedFile> {
        let copied = Arc::new(AtomicU64::new(0));
        let sink = {
            let copied = copied.clone();
            CopyProgress::from_fn(move |bytes| {
                copied.fetch_add(bytes, Ordering::Relaxed);
            })
        };

        let mut attempt = 1_u32;
        let result = loop {
            match self
                .move_file_once(operation, title, file, &sink, &copied, progress, plan)
                .await
            {
                Ok(verified) => break Ok(verified),
                Err(error) if attempt < self.retry.attempts && move_error_is_transient(&error) => {
                    tracing::warn!(
                        operation_id = %operation.id,
                        title_id = %title.title_id,
                        destination = %file.destination_path.display(),
                        attempt,
                        error = %error,
                        "a file move failed on a transient error; retrying from a clean partial"
                    );
                    tokio::time::sleep(self.retry.delay_before(attempt + 1)).await;
                    attempt += 1;
                }
                Err(error) => break Err(error),
            }
        };

        progress.clear_in_flight();
        result
    }

    /// One attempt at one file, with the pulse running alongside it.
    #[allow(clippy::too_many_arguments)]
    async fn move_file_once(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        file: &PlannedFile,
        sink: &CopyProgress,
        copied: &Arc<AtomicU64>,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) -> AppResult<VerifiedFile> {
        // A retried attempt starts from a clean partial, so the bytes the
        // abandoned one wrote are not progress any more.
        copied.store(0, Ordering::Relaxed);
        progress.clear_in_flight();

        let move_file = self.mover.move_file(FileMoveRequest {
            operation_id: &operation.id,
            title,
            file,
            depth: operation.verification_depth,
            progress: sink,
        });
        tokio::pin!(move_file);

        loop {
            tokio::select! {
                result = &mut move_file => return result,
                () = tokio::time::sleep(self.pulse_interval) => {
                    self.pulse(operation, title, copied, progress, plan).await;
                }
            }
        }
    }

    /// One throttled progress write while a file is still in flight.
    ///
    /// Three things move: the operation row's `bytes_processed` and its
    /// `updated_at` (which is what tells a watcher this run is alive rather
    /// than abandoned), the title's checkpoint bytes, and whatever mirror the
    /// observer keeps. All three are best effort — a move must not fail
    /// because a progress write did.
    async fn pulse(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        copied: &Arc<AtomicU64>,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) {
        progress.set_in_flight(&title.title_id, copied.load(Ordering::Relaxed));

        if let Err(error) = self
            .write_title_checkpoint(
                operation,
                title,
                TitleCheckpointState::Moving,
                None,
                progress,
            )
            .await
        {
            tracing::warn!(
                operation_id = %operation.id,
                title_id = %title.title_id,
                %error,
                "could not write a progress pulse to the title checkpoint"
            );
        }
        if let Err(error) = self
            .write_progress(
                operation,
                LocationOperationState::Moving,
                progress,
                plan,
                None,
                false,
            )
            .await
        {
            tracing::warn!(
                operation_id = %operation.id,
                %error,
                "could not write a progress pulse to the operation row"
            );
        }
        self.observe(operation, LocationOperationState::Moving, progress, plan)
            .await;
    }

    /// Hand the observer a snapshot, at most once per pulse interval.
    ///
    /// Called from the pulse (which has already waited that long) and from each
    /// title boundary. The throttle is what makes the second caller safe: a
    /// plan of ten thousand small titles must not write ten thousand job-run
    /// rows, and a plan of one enormous file must still show movement.
    async fn observe(
        &self,
        operation: &LocationOperation,
        state: LocationOperationState,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) {
        let Some(observer) = self.observer else {
            return;
        };
        let now = std::time::Instant::now();
        if let Some(last) = progress.last_observed
            && now.duration_since(last) < self.pulse_interval
        {
            return;
        }
        progress.last_observed = Some(now);
        observer
            .observe(OperationProgressSnapshot {
                operation_id: &operation.id,
                state,
                counters: progress.counters(plan),
            })
            .await;
    }

    async fn settle_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        state: TitleCheckpointState,
        detail: Option<String>,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
    ) -> AppResult<()> {
        progress.settle(title, state);
        self.write_title_checkpoint(operation, title, state, detail, progress)
            .await?;
        let operation_state = operation_state_for(state);
        self.write_progress(operation, operation_state, progress, plan, None, false)
            .await?;
        // A settled title is progress the jobs list should show even when no
        // single file ran long enough to pulse.
        self.observe(operation, operation_state, progress, plan)
            .await;
        Ok(())
    }

    async fn write_title_checkpoint(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        state: TitleCheckpointState,
        detail: Option<String>,
        progress: &RunProgress,
    ) -> AppResult<()> {
        let now = Utc::now();
        let files_verified = progress.files_done(&title.title_id);
        let bytes_verified = progress.bytes_done(&title.title_id);
        self.store
            .upsert_location_title_checkpoint(&TitleCheckpoint {
                operation_id: operation.id.clone(),
                title_id: title.title_id.clone(),
                sequence: title.sequence,
                state,
                classification: title.classification,
                placement: title.placement.clone(),
                files_total: title.files.len() as i64,
                files_verified,
                bytes_total: title.bytes_total(),
                bytes_verified,
                detail,
                started_at: Some(now),
                updated_at: now,
                completed_at: state.is_settled().then_some(now),
            })
            .await
    }

    async fn write_progress(
        &self,
        operation: &LocationOperation,
        state: LocationOperationState,
        progress: &RunProgress,
        plan: &OperationWorkPlan,
        detail: Option<String>,
        started: bool,
    ) -> AppResult<()> {
        let clear_detail = detail.is_none();
        self.store
            .update_location_operation_progress(&LocationOperationProgress {
                operation_id: operation.id.clone(),
                state,
                counters: progress.counters(plan),
                verification_fallback_count: progress.verification_fallbacks,
                detail,
                clear_detail,
                started_at: started.then(Utc::now),
                completed_at: state.is_terminal().then(Utc::now),
            })
            .await
    }

    async fn finish(
        &self,
        operation: &LocationOperation,
        plan: &OperationWorkPlan,
        progress: &RunProgress,
        state: LocationOperationState,
        stop_reason: Option<StopReason>,
        detail: Option<String>,
    ) -> AppResult<OperationRunOutcome> {
        self.write_progress(operation, state, progress, plan, detail.clone(), false)
            .await?;
        // A terminal operation holds no ownership: the guard must not keep
        // refusing scans and imports for work that has stopped (FR-084).
        self.store
            .release_location_operation_ownership(&operation.id)
            .await?;
        if let Some(registry) = self.registry {
            registry.release_operation(&operation.id);
        }

        Ok(OperationRunOutcome {
            operation_id: operation.id.clone(),
            state,
            stop_reason,
            counters: progress.counters(plan),
            cursor: progress.cursor(&operation.id),
            warnings: progress.warnings.clone(),
            detail,
        })
    }

    async fn load_checkpoints(
        &self,
        operation_id: &str,
    ) -> AppResult<BTreeMap<String, TitleCheckpoint>> {
        Ok(self
            .store
            .list_location_title_checkpoints(operation_id)
            .await?
            .into_iter()
            .map(|checkpoint| (checkpoint.title_id.clone(), checkpoint))
            .collect())
    }
}

/// How one title's run ended, as far as the loop that drives titles cares.
enum TitleRunOutcome {
    /// The title did everything the plan asked, with these warnings.
    Finished(Vec<String>),
    /// A cancel arrived between two of the title's files. The title is left
    /// unsettled on purpose.
    Canceled,
}

/// How many titles failed, of how many, and why — capped so one bad mount
/// cannot write a novel into the operation row.
fn describe_title_failures(failures: &[String], titles_total: usize) -> String {
    const NAMED: usize = 5;
    let mut detail = format!(
        "{} of {titles_total} title(s) failed: {}",
        failures.len(),
        failures
            .iter()
            .take(NAMED)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    if failures.len() > NAMED {
        detail.push_str(&format!(
            "; and {} more (see each title's checkpoint)",
            failures.len() - NAMED
        ));
    }
    detail
}

/// The operation-level state that mirrors a settled title's checkpoint state.
fn operation_state_for(state: TitleCheckpointState) -> LocationOperationState {
    match state {
        TitleCheckpointState::Moving => LocationOperationState::Moving,
        TitleCheckpointState::Verifying => LocationOperationState::Verifying,
        TitleCheckpointState::Reconciling => LocationOperationState::Reconciling,
        TitleCheckpointState::CleaningUp => LocationOperationState::CleaningUp,
        // Settled titles do not end the operation: the runner keeps going, so
        // the operation stays in its working phase until the last title.
        TitleCheckpointState::Pending
        | TitleCheckpointState::Completed
        | TitleCheckpointState::CompletedWithWarnings
        | TitleCheckpointState::Skipped
        | TitleCheckpointState::Blocked
        | TitleCheckpointState::Failed => LocationOperationState::Moving,
    }
}

/// Every entity the operation owns for its duration (FR-084, D7): each title in
/// the plan, each title the plan names without instructions (root-scoped
/// assignments and merge targets), and every root either side of the work.
///
/// # Why the operation row's two root ids are not enough
///
/// `LocationOperation::source_root_id` is `None` whenever the selection spans
/// more than one root — the one case where several source roots are genuinely
/// being read from and pruned. Claiming only the operation row's ids would
/// leave every one of them open to a scan or a root reconfiguration mid-move.
/// [`OperationWorkPlan::additional_owned_roots`] carries the per-title roots the
/// instruction set actually names, so the claim set is the union.
pub fn owned_entities(operation: &LocationOperation, plan: &OperationWorkPlan) -> Vec<OwnedEntity> {
    let mut entities: Vec<OwnedEntity> = plan
        .titles
        .iter()
        .map(|title| OwnedEntity::Title(title.title_id.clone()))
        .chain(
            plan.additional_owned_titles
                .iter()
                .map(|title_id| OwnedEntity::Title(title_id.clone())),
        )
        .collect();
    for root_id in [
        operation.source_root_id.as_ref(),
        operation.destination_root_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    .chain(plan.additional_owned_roots.iter())
    {
        entities.push(OwnedEntity::Root(root_id.clone()));
    }
    entities
        .sort_by(|left, right| (left.kind_str(), left.id()).cmp(&(right.kind_str(), right.id())));
    entities.dedup();
    entities
}

fn describe_ownership_conflicts(
    conflicts: &[crate::location::ownership_guard::OwnershipConflict],
) -> String {
    let held = conflicts
        .iter()
        .map(|conflict| {
            format!(
                "{} {} (operation {})",
                conflict.entity.kind_str(),
                conflict.entity.id(),
                conflict.operation_id
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("another location operation already owns {held}")
}

/// In-run bookkeeping: what has settled, what has been proven, and the counters
/// that go with it. Rebuilt from persisted checkpoints on every run, so a resume
/// starts from durable facts rather than from memory.
#[derive(Debug, Default)]
struct RunProgress {
    settled: BTreeMap<String, TitleCheckpointState>,
    files_done: BTreeMap<String, i64>,
    bytes_done: BTreeMap<String, i64>,
    last_settled_sequence: i64,
    verification_fallbacks: i64,
    warnings: Vec<String>,
    /// Titles this run gave up on.
    ///
    /// Deliberately not part of `settled`: [`TitleCheckpointState::is_settled`]
    /// answers "never reprocess this", and a failed title is one a *later*
    /// operation is allowed to try again (FR-092). This run is nonetheless done
    /// with it, and the counters have to report it as neither processed nor
    /// pending.
    failed: BTreeSet<String>,
    /// When the progress observer was last handed a snapshot, so the two things
    /// that trigger one — the in-file pulse and a settled title — share a
    /// single cadence.
    last_observed: Option<std::time::Instant>,
    /// Bytes of the file currently being copied, and the title it belongs to.
    ///
    /// Transient and never persisted as a settled fact: it is added to the
    /// reported counters while the file is in flight so a bar moves inside a
    /// single large file, and cleared the moment that file resolves. Every
    /// write that settles a title therefore reports proven bytes only, which
    /// is what a resume reads back.
    in_flight: Option<(String, i64)>,
}

impl RunProgress {
    fn resume(plan: &OperationWorkPlan, checkpoints: &BTreeMap<String, TitleCheckpoint>) -> Self {
        let mut progress = Self::default();
        for title in &plan.titles {
            let Some(checkpoint) = checkpoints.get(&title.title_id) else {
                continue;
            };
            if checkpoint.state.is_settled() {
                progress
                    .settled
                    .insert(title.title_id.clone(), checkpoint.state);
                progress
                    .files_done
                    .insert(title.title_id.clone(), checkpoint.files_verified);
                progress
                    .bytes_done
                    .insert(title.title_id.clone(), checkpoint.bytes_verified);
                progress.last_settled_sequence = progress.last_settled_sequence.max(title.sequence);
            }
        }
        progress
    }

    fn is_settled(&self, title_id: &str) -> bool {
        self.settled.contains_key(title_id)
    }

    fn settle(&mut self, title: &PlannedTitle, state: TitleCheckpointState) {
        if state == TitleCheckpointState::Failed {
            self.failed.insert(title.title_id.clone());
            return;
        }
        if state.is_settled() {
            self.settled.insert(title.title_id.clone(), state);
            self.last_settled_sequence = self.last_settled_sequence.max(title.sequence);
        }
    }

    fn note_file_done(&mut self, title_id: &str, file: &PlannedFile) {
        *self.files_done.entry(title_id.to_string()).or_insert(0) += 1;
        let bytes = self.bytes_done.entry(title_id.to_string()).or_insert(0);
        *bytes = bytes.saturating_add(file.size_bytes as i64);
    }

    fn set_in_flight(&mut self, title_id: &str, bytes: u64) {
        self.in_flight = Some((title_id.to_string(), bytes.min(i64::MAX as u64) as i64));
    }

    fn clear_in_flight(&mut self) {
        self.in_flight = None;
    }

    fn in_flight_bytes(&self) -> i64 {
        self.in_flight
            .as_ref()
            .map(|(_, bytes)| *bytes)
            .unwrap_or(0)
    }

    fn files_done(&self, title_id: &str) -> i64 {
        self.files_done.get(title_id).copied().unwrap_or(0)
    }

    fn bytes_done(&self, title_id: &str) -> i64 {
        let proven = self.bytes_done.get(title_id).copied().unwrap_or(0);
        match &self.in_flight {
            Some((in_flight_title, bytes)) if in_flight_title == title_id => {
                proven.saturating_add(*bytes)
            }
            _ => proven,
        }
    }

    fn cursor(&self, operation_id: &str) -> ResumeCursor {
        ResumeCursor {
            operation_id: operation_id.to_string(),
            last_settled_sequence: self.last_settled_sequence,
        }
    }

    /// The FR-091 counters, recomputed from the confirmed plan and the settled
    /// checkpoints every time they are written.
    ///
    /// Recomputed rather than incremented on purpose: a resume rebuilds
    /// `settled` from the persisted checkpoints, so the same title never
    /// contributes twice however many times the operation is interrupted.
    /// Outcome counts are taken only from titles that actually *finished* —
    /// a plan's dedup and rename decisions are not outcomes until the title
    /// carrying them settles.
    fn counters(&self, plan: &OperationWorkPlan) -> LocationOperationCounters {
        let titles_blocked = self
            .settled
            .values()
            .filter(|state| matches!(state, TitleCheckpointState::Blocked))
            .count() as i64;
        let skipped = self
            .settled
            .values()
            .filter(|state| matches!(state, TitleCheckpointState::Skipped))
            .count() as i64;
        // A failed title did not do what the plan asked, so it is not
        // "processed". It lands in `unresolved` beside the blocked ones: both
        // are titles that still need the user, and each one's checkpoint
        // carries the reason.
        let titles_failed = self.failed.len() as i64;

        let mut merges = 0_i64;
        let mut dedups = 0_i64;
        let mut renames = 0_i64;
        for title in &plan.titles {
            if !matches!(
                self.settled.get(&title.title_id),
                Some(TitleCheckpointState::Completed)
                    | Some(TitleCheckpointState::CompletedWithWarnings)
            ) {
                continue;
            }
            if title.placement.merged_into_title_id.is_some() {
                merges += 1;
            }
            dedups += title.outcomes.dedups;
            renames += title.outcomes.renames;
        }

        LocationOperationCounters {
            titles_total: plan.titles.len() as i64,
            titles_processed: self.settled.len() as i64 - titles_blocked,
            titles_blocked,
            files_total: plan.files_total(),
            files_processed: self.files_done.values().sum(),
            bytes_total: plan.bytes_total(),
            bytes_processed: self
                .bytes_done
                .values()
                .sum::<i64>()
                .saturating_add(self.in_flight_bytes()),
            merges,
            dedups,
            renames,
            // A title the preview called a no-op and one the runner skipped are
            // the same fact to the user: nothing had to change.
            no_ops: plan.baseline.no_ops + skipped,
            // Likewise a title classification could not resolve, one the runner
            // could not admit, and one that failed all still need the user.
            unresolved: plan.baseline.unresolved + titles_blocked + titles_failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use crate::location::model::{
        AppliedVerificationDepth, FileVerificationOutcome, LocationExecutionMode,
        LocationOperationType,
    };
    use crate::location::ownership_guard::{GuardedAction, OwnershipConflict};
    use crate::ports::LocationOwnershipClaim;

    /// An in-memory stand-in for the 0206 tables, plus a transition log so the
    /// state machine itself can be asserted rather than only its end state.
    #[derive(Default)]
    struct FakeStore {
        inner: Mutex<FakeStoreState>,
    }

    #[derive(Default)]
    struct FakeStoreState {
        operations: BTreeMap<String, LocationOperation>,
        plans: BTreeMap<String, String>,
        checkpoints: BTreeMap<(String, String), TitleCheckpoint>,
        verifications: Vec<FileVerificationRecord>,
        ownership: BTreeMap<(String, String), String>,
        cancel_requested: BTreeSet<String>,
        operation_states: Vec<LocationOperationState>,
        checkpoint_states: Vec<(String, TitleCheckpointState)>,
        /// Every `bytes_verified` a checkpoint write carried, so an in-flight
        /// pulse is observable rather than only its end state.
        checkpoint_byte_writes: Vec<i64>,
        released_operations: Vec<String>,
    }

    impl FakeStore {
        fn with_operation(operation: LocationOperation) -> Self {
            let store = Self::default();
            store
                .inner
                .lock()
                .expect("lock")
                .operations
                .insert(operation.id.clone(), operation);
            store
        }

        fn claim_for(&self, entity: &OwnedEntity, operation_id: &str) {
            self.inner.lock().expect("lock").ownership.insert(
                (entity.kind_str().to_string(), entity.id().to_string()),
                operation_id.to_string(),
            );
        }

        fn request_cancel(&self, operation_id: &str) {
            self.inner
                .lock()
                .expect("lock")
                .cancel_requested
                .insert(operation_id.to_string());
        }

        fn seed_checkpoint(&self, checkpoint: TitleCheckpoint) {
            self.inner.lock().expect("lock").checkpoints.insert(
                (checkpoint.operation_id.clone(), checkpoint.title_id.clone()),
                checkpoint,
            );
        }

        fn seed_verification(&self, record: FileVerificationRecord) {
            self.inner.lock().expect("lock").verifications.push(record);
        }

        fn operation_states(&self) -> Vec<LocationOperationState> {
            self.inner.lock().expect("lock").operation_states.clone()
        }

        fn checkpoint_states(&self, title_id: &str) -> Vec<TitleCheckpointState> {
            self.inner
                .lock()
                .expect("lock")
                .checkpoint_states
                .iter()
                .filter(|(id, _)| id == title_id)
                .map(|(_, state)| *state)
                .collect()
        }

        fn checkpoint(&self, operation_id: &str, title_id: &str) -> Option<TitleCheckpoint> {
            self.inner
                .lock()
                .expect("lock")
                .checkpoints
                .get(&(operation_id.to_string(), title_id.to_string()))
                .cloned()
        }

        fn released(&self) -> Vec<String> {
            self.inner.lock().expect("lock").released_operations.clone()
        }

        fn open_claims(&self) -> usize {
            self.inner.lock().expect("lock").ownership.len()
        }
    }

    #[async_trait]
    impl LocationOperationRepository for FakeStore {
        async fn create_location_operation(
            &self,
            operation: &LocationOperation,
            plan_json: Option<&str>,
        ) -> AppResult<()> {
            let mut state = self.inner.lock().expect("lock");
            state
                .operations
                .insert(operation.id.clone(), operation.clone());
            if let Some(plan_json) = plan_json {
                state
                    .plans
                    .insert(operation.id.clone(), plan_json.to_string());
            }
            Ok(())
        }

        async fn get_location_operation(
            &self,
            operation_id: &str,
        ) -> AppResult<Option<LocationOperation>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .operations
                .get(operation_id)
                .cloned())
        }

        async fn get_location_operation_plan_json(
            &self,
            operation_id: &str,
        ) -> AppResult<Option<String>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .plans
                .get(operation_id)
                .cloned())
        }

        async fn list_active_location_operations(&self) -> AppResult<Vec<LocationOperation>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .operations
                .values()
                .filter(|operation| operation.state.is_active())
                .cloned()
                .collect())
        }

        async fn update_location_operation_progress(
            &self,
            progress: &LocationOperationProgress,
        ) -> AppResult<()> {
            let mut state = self.inner.lock().expect("lock");
            state.operation_states.push(progress.state);
            if let Some(operation) = state.operations.get_mut(&progress.operation_id) {
                operation.state = progress.state;
                operation.counters = progress.counters;
                operation.verification_fallback_count = progress.verification_fallback_count;
                if progress.detail.is_some() || progress.clear_detail {
                    operation.detail = progress.detail.clone();
                }
                if operation.started_at.is_none() {
                    operation.started_at = progress.started_at;
                }
                if progress.completed_at.is_some() {
                    operation.completed_at = progress.completed_at;
                }
            }
            Ok(())
        }

        async fn set_location_operation_job_run(
            &self,
            operation_id: &str,
            job_run_id: &str,
        ) -> AppResult<()> {
            let mut state = self.inner.lock().expect("lock");
            if let Some(operation) = state.operations.get_mut(operation_id) {
                operation.job_run_id = Some(job_run_id.to_string());
            }
            Ok(())
        }

        async fn request_location_operation_cancel(&self, operation_id: &str) -> AppResult<bool> {
            let mut state = self.inner.lock().expect("lock");
            let terminal = state
                .operations
                .get(operation_id)
                .map(|operation| operation.state.is_terminal())
                .unwrap_or(true);
            if terminal {
                return Ok(false);
            }
            state.cancel_requested.insert(operation_id.to_string());
            Ok(true)
        }

        async fn location_operation_cancel_requested(&self, operation_id: &str) -> AppResult<bool> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .cancel_requested
                .contains(operation_id))
        }

        async fn upsert_location_title_checkpoint(
            &self,
            checkpoint: &TitleCheckpoint,
        ) -> AppResult<()> {
            let mut state = self.inner.lock().expect("lock");
            state
                .checkpoint_states
                .push((checkpoint.title_id.clone(), checkpoint.state));
            state.checkpoint_byte_writes.push(checkpoint.bytes_verified);
            state.checkpoints.insert(
                (checkpoint.operation_id.clone(), checkpoint.title_id.clone()),
                checkpoint.clone(),
            );
            Ok(())
        }

        async fn list_location_title_checkpoints(
            &self,
            operation_id: &str,
        ) -> AppResult<Vec<TitleCheckpoint>> {
            let state = self.inner.lock().expect("lock");
            let mut checkpoints: Vec<TitleCheckpoint> = state
                .checkpoints
                .iter()
                .filter(|((id, _), _)| id == operation_id)
                .map(|(_, checkpoint)| checkpoint.clone())
                .collect();
            checkpoints.sort_by_key(|checkpoint| checkpoint.sequence);
            Ok(checkpoints)
        }

        async fn record_location_file_verification(
            &self,
            record: &FileVerificationRecord,
        ) -> AppResult<()> {
            let mut state = self.inner.lock().expect("lock");
            state.verifications.retain(|existing| {
                existing.operation_id != record.operation_id
                    || existing.destination_path != record.destination_path
            });
            state.verifications.push(record.clone());
            Ok(())
        }

        async fn list_location_file_verifications(
            &self,
            operation_id: &str,
            title_id: Option<&str>,
        ) -> AppResult<Vec<FileVerificationRecord>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .verifications
                .iter()
                .filter(|record| record.operation_id == operation_id)
                .filter(|record| title_id.is_none_or(|title| record.title_id == title))
                .cloned()
                .collect())
        }

        async fn verified_destination_paths(
            &self,
            operation_id: &str,
            title_id: &str,
        ) -> AppResult<BTreeSet<String>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .verifications
                .iter()
                .filter(|record| record.operation_id == operation_id)
                .filter(|record| record.title_id == title_id)
                .filter(|record| record.outcome.permits_source_removal())
                .map(|record| record.destination_path.clone())
                .collect())
        }

        async fn claim_location_operation_ownership(
            &self,
            operation_id: &str,
            entities: &[OwnedEntity],
        ) -> AppResult<LocationOwnershipOutcome> {
            let mut state = self.inner.lock().expect("lock");
            let conflicts: Vec<OwnershipConflict> = entities
                .iter()
                .filter_map(|entity| {
                    let key = (entity.kind_str().to_string(), entity.id().to_string());
                    state
                        .ownership
                        .get(&key)
                        .filter(|holder| holder.as_str() != operation_id)
                        .map(|holder| OwnershipConflict {
                            operation_id: holder.clone(),
                            entity: entity.clone(),
                            action: GuardedAction::LocationOperation,
                        })
                })
                .collect();
            if !conflicts.is_empty() {
                return Ok(LocationOwnershipOutcome::Conflict(conflicts));
            }
            for entity in entities {
                state.ownership.insert(
                    (entity.kind_str().to_string(), entity.id().to_string()),
                    operation_id.to_string(),
                );
            }
            Ok(LocationOwnershipOutcome::Claimed)
        }

        async fn release_location_operation_ownership(&self, operation_id: &str) -> AppResult<u64> {
            let mut state = self.inner.lock().expect("lock");
            state.released_operations.push(operation_id.to_string());
            let before = state.ownership.len();
            state
                .ownership
                .retain(|_, holder| holder.as_str() != operation_id);
            Ok((before - state.ownership.len()) as u64)
        }

        async fn location_ownership_holder(
            &self,
            entity: &OwnedEntity,
        ) -> AppResult<Option<String>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .ownership
                .get(&(entity.kind_str().to_string(), entity.id().to_string()))
                .cloned())
        }

        async fn list_location_ownership_claims(&self) -> AppResult<Vec<LocationOwnershipClaim>> {
            Ok(self
                .inner
                .lock()
                .expect("lock")
                .ownership
                .iter()
                .map(|((kind, id), operation_id)| LocationOwnershipClaim {
                    operation_id: operation_id.clone(),
                    entity: if kind == "root" {
                        OwnedEntity::Root(id.clone())
                    } else {
                        OwnedEntity::Title(id.clone())
                    },
                    acquired_at: Utc::now(),
                })
                .collect())
        }
    }

    /// A mover that records every file it was asked to move and returns whatever
    /// outcome the test scripted for that destination.
    #[derive(Default)]
    struct FakeMover {
        moved: Mutex<Vec<String>>,
        failures: BTreeMap<String, FileVerificationOutcome>,
        fallbacks: BTreeSet<String>,
    }

    impl FakeMover {
        fn moved(&self) -> Vec<String> {
            self.moved.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl TitleFileMover for FakeMover {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            let destination = request.file.stored_destination();
            self.moved.lock().expect("lock").push(destination.clone());
            let outcome = self
                .failures
                .get(&destination)
                .copied()
                .unwrap_or(FileVerificationOutcome::Verified);
            let fell_back = self.fallbacks.contains(&destination);
            Ok(VerifiedFile {
                source_path: request.file.source_path.clone(),
                destination_path: request.file.destination_path.clone(),
                hashes: None,
                depth: if fell_back {
                    AppliedVerificationDepth::quick_fallback()
                } else {
                    AppliedVerificationDepth::exact(request.depth)
                },
                outcome,
                detail: fell_back.then(|| "a cache-bypassed read-back could not run".to_string()),
            })
        }
    }

    /// An admission check driven by a per-title script.
    struct ScriptedAdmission {
        answers: BTreeMap<String, TitleAdmission>,
        /// Titles whose verified destinations the check asserts it can see.
        seen_verified: Mutex<Vec<(String, usize)>>,
    }

    impl ScriptedAdmission {
        fn new(answers: &[(&str, TitleAdmission)]) -> Self {
            Self {
                answers: answers
                    .iter()
                    .map(|(title, admission)| ((*title).to_string(), admission.clone()))
                    .collect(),
                seen_verified: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TitleAdmissionCheck for ScriptedAdmission {
        async fn admit_title(
            &self,
            context: TitleAdmissionContext<'_>,
        ) -> AppResult<TitleAdmission> {
            self.seen_verified.lock().expect("lock").push((
                context.title.title_id.clone(),
                context.verified_destinations.len(),
            ));
            Ok(self
                .answers
                .get(&context.title.title_id)
                .cloned()
                .unwrap_or(TitleAdmission::Proceed))
        }
    }

    #[derive(Default)]
    struct RecordingReconciler {
        reconciled: Mutex<Vec<String>>,
        cleaned: Mutex<Vec<String>>,
        warnings: BTreeMap<String, String>,
    }

    #[async_trait]
    impl TitleReconciler for RecordingReconciler {
        async fn reconcile_title(
            &self,
            _operation: &LocationOperation,
            title: &PlannedTitle,
        ) -> AppResult<TitleStepOutcome> {
            self.reconciled
                .lock()
                .expect("lock")
                .push(title.title_id.clone());
            Ok(match self.warnings.get(&title.title_id) {
                Some(warning) => TitleStepOutcome::warned(warning.clone()),
                None => TitleStepOutcome::clean(),
            })
        }

        async fn clean_up_title(
            &self,
            _operation: &LocationOperation,
            title: &PlannedTitle,
        ) -> AppResult<TitleStepOutcome> {
            self.cleaned
                .lock()
                .expect("lock")
                .push(title.title_id.clone());
            Ok(TitleStepOutcome::clean())
        }
    }

    fn operation() -> LocationOperation {
        let now = Utc::now();
        LocationOperation {
            id: "op-1".to_string(),
            operation_type: LocationOperationType::RootMove,
            mode: LocationExecutionMode::MoveWithScryer,
            state: LocationOperationState::Queued,
            initiated_by_user_id: Some("user-1".to_string()),
            source_library_id: Some("library-1".to_string()),
            destination_library_id: Some("library-1".to_string()),
            source_root_id: Some("root-1".to_string()),
            destination_root_id: Some("root-2".to_string()),
            plan_fingerprint: "fingerprint".to_string(),
            verification_depth: VerificationDepth::Full,
            verification_fallback_count: 0,
            counters: LocationOperationCounters::default(),
            detail: None,
            job_run_id: None,
            workflow_operation_id: None,
            cancel_requested: false,
            cancel_requested_at: None,
            confirmed_at: Some(now),
            started_at: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    fn planned_title(title_id: &str, sequence: i64, files: usize) -> PlannedTitle {
        PlannedTitle {
            title_id: title_id.to_string(),
            sequence,
            classification: Some(TitleLocationClass::RootMove),
            placement: TitleCheckpointPlacement {
                source_root_id: Some("root-1".to_string()),
                destination_root_id: Some("root-2".to_string()),
                ..TitleCheckpointPlacement::default()
            },
            files: (0..files)
                .map(|index| PlannedFile {
                    media_file_id: Some(format!("{title_id}-file-{index}")),
                    source_path: PathBuf::from(format!("/source/{title_id}/{index}.mkv")),
                    destination_path: PathBuf::from(format!("/destination/{title_id}/{index}.mkv")),
                    size_bytes: 100,
                })
                .collect(),
            outcomes: TitleOutcomeCounts::default(),
        }
    }

    fn two_title_plan() -> OperationWorkPlan {
        OperationWorkPlan::new(vec![
            planned_title("title-1", 1, 2),
            planned_title("title-2", 2, 1),
        ])
    }

    /// The same two titles, but carrying the outcome facts a real plan would:
    /// title-1 merges into a destination title and dedups a file, title-2
    /// renames one around a collision.
    fn two_title_plan_with_outcomes() -> OperationWorkPlan {
        let mut first = planned_title("title-1", 1, 2);
        first.placement.merged_into_title_id = Some("title-99".to_string());
        first.outcomes = TitleOutcomeCounts {
            dedups: 1,
            renames: 0,
        };
        let mut second = planned_title("title-2", 2, 1);
        second.outcomes = TitleOutcomeCounts {
            dedups: 0,
            renames: 2,
        };
        OperationWorkPlan::new(vec![first, second]).with_baseline(ClassifiedTitleBaseline {
            no_ops: 3,
            unresolved: 0,
        })
    }

    fn verified_record(title_id: &str, destination: &str) -> FileVerificationRecord {
        FileVerificationRecord {
            operation_id: "op-1".to_string(),
            title_id: title_id.to_string(),
            media_file_id: None,
            source_path: format!("/source/{title_id}"),
            destination_path: destination.to_string(),
            hashes: None,
            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
            outcome: FileVerificationOutcome::Verified,
            detail: None,
            verified_at: Utc::now(),
        }
    }

    /// An epilogue that records what the operation row said while it ran, so
    /// the ordering contract can be asserted rather than assumed.
    struct RecordingEpilogue<'a> {
        store: &'a FakeStore,
        /// The operation states persisted at the moment the epilogue ran.
        states_when_called: std::sync::Mutex<Vec<LocationOperationState>>,
        warnings: Vec<String>,
        fail_with: Option<String>,
    }

    impl<'a> RecordingEpilogue<'a> {
        fn new(store: &'a FakeStore) -> Self {
            Self {
                store,
                states_when_called: std::sync::Mutex::new(Vec::new()),
                warnings: Vec::new(),
                fail_with: None,
            }
        }

        fn warning(mut self, warning: &str) -> Self {
            self.warnings.push(warning.to_string());
            self
        }

        fn failing(mut self, error: &str) -> Self {
            self.fail_with = Some(error.to_string());
            self
        }
    }

    #[async_trait]
    impl OperationEpilogue for RecordingEpilogue<'_> {
        async fn finish_operation(&self, _operation: &LocationOperation) -> AppResult<Vec<String>> {
            *self.states_when_called.lock().expect("lock") = self.store.operation_states();
            match self.fail_with.as_deref() {
                Some(error) => Err(AppError::Repository(error.to_string())),
                None => Ok(self.warnings.clone()),
            }
        }
    }

    /// FR-087: an operation with an epilogue is never readable as finished
    /// before the epilogue has run.
    ///
    /// A root change retires its source location and flips the root's path
    /// here. Doing that after `run` returned would leave a window in which
    /// Activity, a watching client, and a resume all see `completed` while the
    /// root still points at the retired path.
    #[tokio::test]
    async fn an_epilogue_runs_before_the_terminal_state_and_its_warnings_are_the_operations() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let epilogue = RecordingEpilogue::new(&store).warning("the old location was kept");

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .with_epilogue(&epilogue)
            .run("op-1", &two_title_plan())
            .await
            .expect("the run should succeed");

        let seen = epilogue.states_when_called.lock().expect("lock").clone();
        assert!(
            !seen.iter().any(|state| state.is_terminal()),
            "the operation was already terminal when the epilogue ran: {seen:?}"
        );
        assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|warning| warning == "the old location was kept")
        );
    }

    /// The epilogue is work the user asked for, not bookkeeping: a root whose
    /// path could not be flipped is a failed root change, however many bytes
    /// arrived safely.
    #[tokio::test]
    async fn an_epilogue_that_fails_fails_the_operation() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let epilogue = RecordingEpilogue::new(&store).failing("the root path could not be updated");

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .with_epilogue(&epilogue)
            .run("op-1", &two_title_plan())
            .await
            .expect("a failed epilogue is a failed operation, not an error out of `run`");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(outcome.stop_reason, Some(StopReason::Error));
        assert!(
            outcome
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("the root path could not be updated")),
            "detail was {:?}",
            outcome.detail
        );
    }

    #[tokio::test]
    async fn a_clean_run_walks_every_title_through_the_whole_state_machine() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should succeed");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert_eq!(outcome.stop_reason, None);
        assert_eq!(outcome.counters.titles_total, 2);
        assert_eq!(outcome.counters.titles_processed, 2);
        assert_eq!(outcome.counters.files_total, 3);
        assert_eq!(outcome.counters.files_processed, 3);
        assert_eq!(outcome.counters.bytes_processed, 300);
        assert_eq!(outcome.cursor.last_settled_sequence, 2);

        assert_eq!(
            store.checkpoint_states("title-1"),
            vec![
                TitleCheckpointState::Moving,
                TitleCheckpointState::Moving,
                TitleCheckpointState::Moving,
                TitleCheckpointState::Verifying,
                TitleCheckpointState::Reconciling,
                TitleCheckpointState::CleaningUp,
                TitleCheckpointState::Completed,
            ],
            "a title moves, is proven complete, reconciles, cleans up, then settles"
        );

        let operation_states = store.operation_states();
        assert_eq!(
            operation_states.first(),
            Some(&LocationOperationState::Preparing)
        );
        assert_eq!(
            operation_states.last(),
            Some(&LocationOperationState::Completed)
        );
        for state in [
            LocationOperationState::Moving,
            LocationOperationState::Verifying,
            LocationOperationState::Reconciling,
            LocationOperationState::CleaningUp,
        ] {
            assert!(
                operation_states.contains(&state),
                "the operation should pass through {}",
                state.as_str()
            );
        }

        assert_eq!(
            reconciler.reconciled.lock().expect("lock").clone(),
            vec!["title-1".to_string(), "title-2".to_string()],
            "titles are processed one at a time, in plan order"
        );
        assert_eq!(store.released(), vec!["op-1".to_string()]);
        assert_eq!(store.open_claims(), 0, "a settled operation owns nothing");
    }

    #[tokio::test]
    async fn a_cancel_stops_at_the_next_title_boundary_and_leaves_finished_titles_alone() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        // A cancel that arrives before the run starts is honored at the first
        // boundary: nothing moves at all.
        store.request_cancel("op-1");
        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a cancelled run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Canceled);
        assert_eq!(outcome.stop_reason, Some(StopReason::UserCanceled));
        assert!(mover.moved().is_empty());
        assert_eq!(store.open_claims(), 0);

        // Now cancel after the first title has already finished.
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let cancelling = CancelAfterFirstTitle {
            store: &store,
            operation_id: "op-1",
        };
        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &cancelling)
            .run("op-1", &plan)
            .await
            .expect("a cancelled run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Canceled);
        assert_eq!(outcome.counters.titles_processed, 1);
        assert_eq!(
            mover.moved(),
            vec![
                "/destination/title-1/0.mkv".to_string(),
                "/destination/title-1/1.mkv".to_string(),
            ],
            "the second title is never touched"
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("title-1 should have a checkpoint")
                .state,
            TitleCheckpointState::Completed,
            "the completed title stays consistent and visible"
        );
        assert!(
            store.checkpoint("op-1", "title-2").is_none(),
            "the untouched title has no checkpoint at all"
        );
    }

    /// Requests the cancel as soon as the first title's catalog work runs, so the
    /// runner meets it at the next title boundary.
    struct CancelAfterFirstTitle<'a> {
        store: &'a FakeStore,
        operation_id: &'a str,
    }

    #[async_trait]
    impl TitleReconciler for CancelAfterFirstTitle<'_> {
        async fn reconcile_title(
            &self,
            _operation: &LocationOperation,
            _title: &PlannedTitle,
        ) -> AppResult<TitleStepOutcome> {
            self.store.request_cancel(self.operation_id);
            Ok(TitleStepOutcome::clean())
        }
    }

    #[tokio::test]
    async fn a_resume_skips_settled_titles_and_already_verified_files() {
        let mut resumed = operation();
        resumed.state = LocationOperationState::Moving;
        let store = FakeStore::with_operation(resumed);
        let plan = two_title_plan();

        // title-1 settled in an earlier run.
        let now = Utc::now();
        store.seed_checkpoint(TitleCheckpoint {
            operation_id: "op-1".to_string(),
            title_id: "title-1".to_string(),
            sequence: 1,
            state: TitleCheckpointState::Completed,
            classification: Some(TitleLocationClass::RootMove),
            placement: TitleCheckpointPlacement::default(),
            files_total: 2,
            files_verified: 2,
            bytes_total: 200,
            bytes_verified: 200,
            detail: None,
            started_at: Some(now),
            updated_at: now,
            completed_at: Some(now),
        });
        store.seed_verification(verified_record("title-1", "/destination/title-1/0.mkv"));
        store.seed_verification(verified_record("title-1", "/destination/title-1/1.mkv"));
        // One of title-2's files was already proven before the interruption.
        store.seed_verification(verified_record("title-2", "/destination/title-2/0.mkv"));

        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the resume should succeed");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert!(
            mover.moved().is_empty(),
            "verified work is never repeated on resume (FR-092), moved: {:?}",
            mover.moved()
        );
        assert_eq!(
            reconciler.reconciled.lock().expect("lock").clone(),
            vec!["title-2".to_string()],
            "the settled title is not reprocessed"
        );
        assert_eq!(outcome.counters.titles_processed, 2);
        assert_eq!(outcome.counters.files_processed, 3);
    }

    #[tokio::test]
    async fn an_expected_partial_is_resumable_but_a_changed_input_is_stale() {
        // The admission check is handed this operation's own verified
        // destinations, which is what lets a workflow tell its own interrupted
        // work apart from a foreign change (FR-089).
        let mut resumed = operation();
        resumed.state = LocationOperationState::Moving;
        let store = FakeStore::with_operation(resumed);
        store.seed_verification(verified_record("title-1", "/destination/title-1/0.mkv"));

        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();
        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a partially copied title resumes");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert_eq!(
            mover.moved(),
            vec![
                "/destination/title-1/1.mkv".to_string(),
                "/destination/title-2/0.mkv".to_string(),
            ],
            "only the unproven files are copied again"
        );
        let seen = admission.seen_verified.lock().expect("lock").clone();
        assert_eq!(
            seen,
            vec![("title-1".to_string(), 1), ("title-2".to_string(), 0)],
            "the check sees exactly the destinations this operation already proved"
        );

        // A foreign change to an unprocessed input stops the operation instead.
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let stale = ScriptedAdmission::new(&[(
            "title-2",
            TitleAdmission::Stale("the title's files changed on disk".to_string()),
        )]);
        let outcome = LocationOperationRunner::new(&store, &mover, &stale, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a stale plan still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(outcome.stop_reason, Some(StopReason::StalePlan));
        assert_eq!(
            outcome.detail.as_deref(),
            Some("the title's files changed on disk")
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the first title should have settled")
                .state,
            TitleCheckpointState::Completed,
            "titles finished before the staleness was noticed stay consistent"
        );
        assert_eq!(store.open_claims(), 0);
    }

    #[tokio::test]
    async fn a_blocked_title_does_not_stop_the_rest_of_the_operation() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[
            (
                "title-1",
                TitleAdmission::Blocked("an import is running for this title".to_string()),
            ),
            ("title-2", TitleAdmission::Proceed),
        ]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should finish");

        assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
        assert_eq!(outcome.counters.titles_blocked, 1);
        assert_eq!(outcome.counters.titles_processed, 1);
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the blocked title should have a checkpoint")
                .state,
            TitleCheckpointState::Blocked
        );
        assert_eq!(
            mover.moved(),
            vec!["/destination/title-2/0.mkv".to_string()]
        );
    }

    #[tokio::test]
    async fn the_outcome_counters_report_what_the_plan_decided_once_titles_settle() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan_with_outcomes();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should succeed");

        assert_eq!(outcome.counters.merges, 1, "one title merged (US7)");
        assert_eq!(outcome.counters.dedups, 1);
        assert_eq!(outcome.counters.renames, 2);
        assert_eq!(
            outcome.counters.no_ops, 3,
            "the titles classification called no-ops are still reported"
        );
        assert_eq!(outcome.counters.unresolved, 0);

        // The same counts reach the persisted row, which is what Activity reads.
        let persisted = store
            .inner
            .lock()
            .expect("lock")
            .operations
            .get("op-1")
            .expect("the operation should exist")
            .counters;
        assert_eq!(persisted.merges, 1);
        assert_eq!(persisted.dedups, 1);
        assert_eq!(persisted.renames, 2);
        assert_eq!(persisted.no_ops, 3);
    }

    #[tokio::test]
    async fn a_title_that_never_finishes_contributes_no_outcome_counts() {
        // A dedup or rename the plan decided is not an outcome until the title
        // carrying it settles; a blocked title contributes to `unresolved`
        // instead.
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[(
            "title-2",
            TitleAdmission::Blocked("an import is running for this title".to_string()),
        )]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan_with_outcomes();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should finish");

        assert_eq!(outcome.counters.merges, 1);
        assert_eq!(outcome.counters.dedups, 1);
        assert_eq!(
            outcome.counters.renames, 0,
            "the blocked title's renames never happened"
        );
        assert_eq!(outcome.counters.unresolved, 1);
    }

    #[tokio::test]
    async fn a_resume_reports_the_outcome_counters_once_not_twice() {
        let mut resumed = operation();
        resumed.state = LocationOperationState::Moving;
        let store = FakeStore::with_operation(resumed);
        let plan = two_title_plan_with_outcomes();

        let now = Utc::now();
        store.seed_checkpoint(TitleCheckpoint {
            operation_id: "op-1".to_string(),
            title_id: "title-1".to_string(),
            sequence: 1,
            state: TitleCheckpointState::Completed,
            classification: Some(TitleLocationClass::RootMove),
            placement: TitleCheckpointPlacement::default(),
            files_total: 2,
            files_verified: 2,
            bytes_total: 200,
            bytes_verified: 200,
            detail: None,
            started_at: Some(now),
            updated_at: now,
            completed_at: Some(now),
        });
        store.seed_verification(verified_record("title-1", "/destination/title-1/0.mkv"));
        store.seed_verification(verified_record("title-1", "/destination/title-1/1.mkv"));

        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the resume should succeed");

        assert_eq!(
            outcome.counters.dedups, 1,
            "the title that settled in the earlier run is counted exactly once"
        );
        assert_eq!(outcome.counters.merges, 1);
        assert_eq!(outcome.counters.renames, 2);
    }

    #[tokio::test]
    async fn a_completed_with_warnings_title_writes_its_note_to_the_checkpoint() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler {
            warnings: [(
                "title-1".to_string(),
                "one companion asset was renamed".to_string(),
            )]
            .into_iter()
            .collect(),
            ..RecordingReconciler::default()
        };
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should finish");

        assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
        let checkpoint = store
            .checkpoint("op-1", "title-1")
            .expect("the warned title should have a checkpoint");
        assert_eq!(
            checkpoint.state,
            TitleCheckpointState::CompletedWithWarnings
        );
        assert_eq!(
            checkpoint.detail.as_deref(),
            Some("one companion asset was renamed"),
            "the warning note travels on the checkpoint, not on a failure column"
        );
    }

    #[tokio::test]
    async fn a_skipped_title_settles_without_work() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[(
            "title-1",
            TitleAdmission::Skip("the title already lives at the destination".to_string()),
        )]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("the run should finish");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert_eq!(outcome.counters.no_ops, 1);
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the skipped title should have a checkpoint")
                .state,
            TitleCheckpointState::Skipped
        );
    }

    #[tokio::test]
    async fn a_failed_verification_fails_its_own_title_and_the_rest_still_run() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover {
            failures: [(
                "/destination/title-1/1.mkv".to_string(),
                FileVerificationOutcome::Mismatch,
            )]
            .into_iter()
            .collect(),
            ..FakeMover::default()
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a failed run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(outcome.stop_reason, Some(StopReason::Error));
        assert_eq!(
            reconciler.reconciled.lock().expect("lock").clone(),
            vec!["title-2".to_string()],
            "the catalog is never updated for the title whose content did not verify, and the \
             titles after it are still attempted"
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the failing title should have a checkpoint")
                .state,
            TitleCheckpointState::Failed
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-2")
                .expect("the following title should have been attempted")
                .state,
            TitleCheckpointState::Completed,
            "one title's failure is not the whole operation's"
        );

        // The counters stay honest: the failed title is not "processed", and it
        // is reported as still needing the user.
        assert_eq!(outcome.counters.titles_total, 2);
        assert_eq!(outcome.counters.titles_processed, 1);
        assert_eq!(outcome.counters.unresolved, 1);
        assert!(
            outcome
                .detail
                .as_deref()
                .is_some_and(|detail| detail.starts_with("1 of 2 title(s) failed")
                    && detail.contains("title-1")),
            "the operation says how many titles failed, of how many: {:?}",
            outcome.detail
        );
        assert_eq!(store.open_claims(), 0);
    }

    /// Failure containment ends at the operation, not at the title: an
    /// operation with a failure is Failed, whatever else finished.
    #[tokio::test]
    async fn every_title_failing_still_reports_one_operation_level_failure() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover {
            failures: [
                (
                    "/destination/title-1/0.mkv".to_string(),
                    FileVerificationOutcome::Mismatch,
                ),
                (
                    "/destination/title-2/0.mkv".to_string(),
                    FileVerificationOutcome::Mismatch,
                ),
            ]
            .into_iter()
            .collect(),
            ..FakeMover::default()
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a failed run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(outcome.counters.titles_processed, 0);
        assert_eq!(outcome.counters.unresolved, 2);
        assert!(
            outcome
                .detail
                .as_deref()
                .is_some_and(|detail| detail.starts_with("2 of 2 title(s) failed")),
            "{:?}",
            outcome.detail
        );
        assert!(reconciler.reconciled.lock().expect("lock").is_empty());
    }

    /// FR-092: a cancel that arrives between two files of one title stops the
    /// run there. The title stays unsettled — it did not do what the plan asked
    /// — while the files it already proved keep their records, so a later run
    /// dedups them instead of copying them again.
    #[tokio::test]
    async fn a_cancel_between_two_files_leaves_the_title_unsettled() {
        let store = FakeStore::with_operation(operation());
        let mover = CancelAfterFirstFile {
            store: &store,
            operation_id: "op-1",
            moved: Mutex::new(Vec::new()),
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a cancelled run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Canceled);
        assert_eq!(outcome.stop_reason, Some(StopReason::UserCanceled));
        assert_eq!(
            mover.moved.lock().expect("lock").clone(),
            vec!["/destination/title-1/0.mkv".to_string()],
            "the cancel lands before the title's second file, not after it"
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the interrupted title has a checkpoint")
                .state,
            TitleCheckpointState::Moving,
            "an interrupted title is left unsettled so a later run finishes it"
        );
        assert!(
            reconciler.reconciled.lock().expect("lock").is_empty(),
            "the catalog never moves for a title that did not finish"
        );
        assert_eq!(
            store.inner.lock().expect("lock").verifications.len(),
            1,
            "the file that was proven before the cancel keeps its record"
        );
        assert_eq!(store.open_claims(), 0);
    }

    /// Requests the cancel as the first file of the first title is moved, so
    /// the runner meets it at the boundary *inside* that title.
    struct CancelAfterFirstFile<'a> {
        store: &'a FakeStore,
        operation_id: &'a str,
        moved: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl TitleFileMover for CancelAfterFirstFile<'_> {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            self.moved
                .lock()
                .expect("lock")
                .push(request.file.stored_destination());
            self.store.request_cancel(self.operation_id);
            Ok(VerifiedFile {
                source_path: request.file.source_path.clone(),
                destination_path: request.file.destination_path.clone(),
                hashes: None,
                depth: AppliedVerificationDepth::exact(request.depth),
                outcome: FileVerificationOutcome::Verified,
                detail: None,
            })
        }
    }

    /// A transient I/O failure is a hiccup, not a verdict: the file is moved
    /// again from a clean partial and the title finishes normally.
    #[tokio::test]
    async fn a_transient_copy_error_is_retried_and_the_title_still_completes() {
        let store = FakeStore::with_operation(operation());
        let mover = FailOnceMover {
            attempts: Mutex::new(BTreeMap::new()),
            fail_once: "/destination/title-1/1.mkv".to_string(),
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .with_file_move_retry(FileMoveRetry {
                attempts: 3,
                base_delay: Duration::ZERO,
            })
            .run("op-1", &plan)
            .await
            .expect("the run should succeed");

        assert_eq!(
            outcome.state,
            LocationOperationState::Completed,
            "a retried transient error is not a warning the user has to read"
        );
        assert!(outcome.warnings.is_empty());
        assert_eq!(outcome.counters.files_processed, 3);
        assert_eq!(
            mover
                .attempts
                .lock()
                .expect("lock")
                .get("/destination/title-1/1.mkv")
                .copied(),
            Some(2),
            "the file was attempted exactly twice"
        );
    }

    /// A verification mismatch is evidence about the bytes, so it is never
    /// retried — the file is moved once and the title fails.
    #[tokio::test]
    async fn a_verification_mismatch_is_never_retried() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover {
            failures: [(
                "/destination/title-1/0.mkv".to_string(),
                FileVerificationOutcome::Mismatch,
            )]
            .into_iter()
            .collect(),
            ..FakeMover::default()
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .with_file_move_retry(FileMoveRetry {
                attempts: 3,
                base_delay: Duration::ZERO,
            })
            .run("op-1", &plan)
            .await
            .expect("a failed run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(
            mover
                .moved()
                .iter()
                .filter(|destination| *destination == "/destination/title-1/0.mkv")
                .count(),
            1,
            "a mismatch is a fact about the copy, not a transient failure"
        );
    }

    /// Fails one destination on its first attempt with an I/O-shaped error and
    /// succeeds on every attempt after that.
    struct FailOnceMover {
        attempts: Mutex<BTreeMap<String, u32>>,
        fail_once: String,
    }

    #[async_trait]
    impl TitleFileMover for FailOnceMover {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            let destination = request.file.stored_destination();
            let attempt = {
                let mut attempts = self.attempts.lock().expect("lock");
                let attempt = attempts.entry(destination.clone()).or_insert(0);
                *attempt += 1;
                *attempt
            };
            if destination == self.fail_once && attempt == 1 {
                return Err(AppError::Repository(
                    "the destination volume dropped mid-copy".to_string(),
                ));
            }
            Ok(VerifiedFile {
                source_path: request.file.source_path.clone(),
                destination_path: request.file.destination_path.clone(),
                hashes: None,
                depth: AppliedVerificationDepth::exact(request.depth),
                outcome: FileVerificationOutcome::Verified,
                detail: None,
            })
        }
    }

    /// FR-091: a single long file must not look frozen. The pulse writes the
    /// bytes copied so far onto the operation row and its checkpoint, and
    /// mirrors them to whoever is observing, without waiting for the file to
    /// finish.
    #[tokio::test]
    async fn a_slow_file_pulses_its_progress_before_it_finishes() {
        let store = FakeStore::with_operation(operation());
        let mover = SlowMover {
            report_bytes: 60,
            settle_after: Duration::from_millis(120),
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let observer = RecordingObserver::default();
        let plan = OperationWorkPlan::new(vec![planned_title("title-1", 1, 1)]);

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .with_progress_pulse_interval(Duration::from_millis(10))
            .with_progress_observer(&observer)
            .run("op-1", &plan)
            .await
            .expect("the run should succeed");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert!(
            observer
                .snapshots
                .lock()
                .expect("lock")
                .iter()
                .any(|(state, counters)| *state == LocationOperationState::Moving
                    && counters.bytes_processed == 60),
            "a pulse should report the in-flight bytes: {:?}",
            observer.snapshots.lock().expect("lock")
        );
        assert!(
            store
                .inner
                .lock()
                .expect("lock")
                .checkpoint_byte_writes
                .contains(&60),
            "the title checkpoint carries the in-flight bytes too"
        );

        // Once the file resolves, the persisted counters are proven bytes only:
        // an in-flight estimate never survives into a settled fact.
        assert_eq!(outcome.counters.bytes_processed, 100);
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("checkpoint")
                .bytes_verified,
            100
        );
    }

    /// Reports part of a file's bytes, then takes long enough for the pulse to
    /// notice before it returns.
    struct SlowMover {
        report_bytes: u64,
        settle_after: Duration,
    }

    #[async_trait]
    impl TitleFileMover for SlowMover {
        async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
            request.progress.advance(self.report_bytes);
            tokio::time::sleep(self.settle_after).await;
            Ok(VerifiedFile {
                source_path: request.file.source_path.clone(),
                destination_path: request.file.destination_path.clone(),
                hashes: None,
                depth: AppliedVerificationDepth::exact(request.depth),
                outcome: FileVerificationOutcome::Verified,
                detail: None,
            })
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        snapshots: Mutex<Vec<(LocationOperationState, LocationOperationCounters)>>,
    }

    #[async_trait]
    impl OperationProgressObserver for RecordingObserver {
        async fn observe(&self, snapshot: OperationProgressSnapshot<'_>) {
            self.snapshots
                .lock()
                .expect("lock")
                .push((snapshot.state, snapshot.counters));
        }
    }

    #[tokio::test]
    async fn a_quick_floor_fallback_is_counted_and_warned_about() {
        let store = FakeStore::with_operation(operation());
        let mover = FakeMover {
            fallbacks: ["/destination/title-1/0.mkv".to_string()]
                .into_iter()
                .collect(),
            ..FakeMover::default()
        };
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a fallback is not a failure");

        assert_eq!(outcome.state, LocationOperationState::CompletedWithWarnings);
        assert_eq!(
            store
                .inner
                .lock()
                .expect("lock")
                .operations
                .get("op-1")
                .expect("the operation should exist")
                .verification_fallback_count,
            1
        );
    }

    #[tokio::test]
    async fn an_operation_that_cannot_own_its_entities_fails_before_anything_moves() {
        let store = FakeStore::with_operation(operation());
        store.claim_for(&OwnedEntity::Title("title-2".to_string()), "op-other");

        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();
        let plan = two_title_plan();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &plan)
            .await
            .expect("a conflicting run still returns an outcome");

        assert_eq!(outcome.state, LocationOperationState::Failed);
        assert_eq!(outcome.stop_reason, Some(StopReason::Error));
        assert!(
            outcome
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("op-other")
        );
        assert!(mover.moved().is_empty());
    }

    #[tokio::test]
    async fn a_terminal_operation_is_never_run_again() {
        let mut finished = operation();
        finished.state = LocationOperationState::Completed;
        let store = FakeStore::with_operation(finished);
        let mover = FakeMover::default();
        let admission = ScriptedAdmission::new(&[]);
        let reconciler = RecordingReconciler::default();

        let outcome = LocationOperationRunner::new(&store, &mover, &admission, &reconciler)
            .run("op-1", &two_title_plan())
            .await
            .expect("a terminal operation reports its state");

        assert_eq!(outcome.state, LocationOperationState::Completed);
        assert!(mover.moved().is_empty());
        assert!(store.operation_states().is_empty());
    }

    #[test]
    fn an_operation_owns_its_titles_and_both_roots() {
        let entities = owned_entities(&operation(), &two_title_plan());
        assert_eq!(
            entities,
            vec![
                OwnedEntity::Root("root-1".to_string()),
                OwnedEntity::Root("root-2".to_string()),
                OwnedEntity::Title("title-1".to_string()),
                OwnedEntity::Title("title-2".to_string()),
            ]
        );
    }
}
