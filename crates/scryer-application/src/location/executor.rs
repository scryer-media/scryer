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

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::location::classify::TitleLocationClass;
use crate::location::model::{
    FileVerificationRecord, LocationOperation, LocationOperationCounters, LocationOperationState,
    TitleCheckpoint, TitleCheckpointPlacement, TitleCheckpointState, VerificationDepth,
};
use crate::location::ownership_guard::OwnedEntity;
use crate::location::verify::{FileVerificationIdentity, VerifiedFile};
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
}

impl PlannedTitle {
    pub fn bytes_total(&self) -> i64 {
        self.files
            .iter()
            .fold(0_i64, |total, file| total.saturating_add(file.size_bytes as i64))
    }
}

/// The confirmed plan as the runner walks it: the workflow-specific planner
/// (T031 and friends) reduces its own plan to this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationWorkPlan {
    pub titles: Vec<PlannedTitle>,
}

impl OperationWorkPlan {
    pub fn new(titles: Vec<PlannedTitle>) -> Self {
        let mut titles = titles;
        titles.sort_by_key(|title| title.sequence);
        Self { titles }
    }

    pub fn files_total(&self) -> i64 {
        self.titles
            .iter()
            .map(|title| title.files.len() as i64)
            .sum()
    }

    pub fn bytes_total(&self) -> i64 {
        self.titles
            .iter()
            .fold(0_i64, |total, title| {
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

/// The operation runner (D5).
pub struct LocationOperationRunner<'a> {
    store: &'a dyn LocationOperationRepository,
    mover: &'a dyn TitleFileMover,
    admission: &'a dyn TitleAdmissionCheck,
    reconciler: &'a dyn TitleReconciler,
    registry: Option<&'a crate::location::ownership_guard::LocationOwnershipRegistry>,
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
        }
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
                        Some("canceled at a title checkpoint; completed titles are unchanged".to_string()),
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
                .run_title(&operation, title, &verified_destinations, &mut progress, plan)
                .await
            {
                Ok(warnings) => {
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
                Err(error) => {
                    let detail = error.to_string();
                    self.settle_title(
                        &operation,
                        title,
                        TitleCheckpointState::Failed,
                        Some(detail.clone()),
                        &mut progress,
                        plan,
                    )
                    .await?;
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
    ) -> AppResult<Vec<String>> {
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

            let verified = self
                .mover
                .move_file(FileMoveRequest {
                    operation_id: &operation.id,
                    title,
                    file,
                    depth: operation.verification_depth,
                })
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

        Ok(warnings)
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
        self.write_progress(operation, operation_state_for(state), progress, plan, None, false)
            .await
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
/// the plan plus the roots on either side.
pub fn owned_entities(operation: &LocationOperation, plan: &OperationWorkPlan) -> Vec<OwnedEntity> {
    let mut entities: Vec<OwnedEntity> = plan
        .titles
        .iter()
        .map(|title| OwnedEntity::Title(title.title_id.clone()))
        .collect();
    for root_id in [
        operation.source_root_id.as_ref(),
        operation.destination_root_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        entities.push(OwnedEntity::Root(root_id.clone()));
    }
    entities.sort_by(|left, right| {
        (left.kind_str(), left.id()).cmp(&(right.kind_str(), right.id()))
    });
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
                progress.last_settled_sequence =
                    progress.last_settled_sequence.max(title.sequence);
            }
        }
        progress
    }

    fn is_settled(&self, title_id: &str) -> bool {
        self.settled.contains_key(title_id)
    }

    fn settle(&mut self, title: &PlannedTitle, state: TitleCheckpointState) {
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

    fn files_done(&self, title_id: &str) -> i64 {
        self.files_done.get(title_id).copied().unwrap_or(0)
    }

    fn bytes_done(&self, title_id: &str) -> i64 {
        self.bytes_done.get(title_id).copied().unwrap_or(0)
    }

    fn cursor(&self, operation_id: &str) -> ResumeCursor {
        ResumeCursor {
            operation_id: operation_id.to_string(),
            last_settled_sequence: self.last_settled_sequence,
        }
    }

    fn counters(&self, plan: &OperationWorkPlan) -> LocationOperationCounters {
        let titles_blocked = self
            .settled
            .values()
            .filter(|state| matches!(state, TitleCheckpointState::Blocked))
            .count() as i64;
        let no_ops = self
            .settled
            .values()
            .filter(|state| matches!(state, TitleCheckpointState::Skipped))
            .count() as i64;

        LocationOperationCounters {
            titles_total: plan.titles.len() as i64,
            titles_processed: self.settled.len() as i64 - titles_blocked,
            titles_blocked,
            files_total: plan.files_total(),
            files_processed: self.files_done.values().sum(),
            bytes_total: plan.bytes_total(),
            bytes_processed: self.bytes_done.values().sum(),
            merges: 0,
            dedups: 0,
            renames: 0,
            no_ops,
            unresolved: titles_blocked,
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
            self.inner
                .lock()
                .expect("lock")
                .verifications
                .push(record);
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

        async fn location_operation_cancel_requested(
            &self,
            operation_id: &str,
        ) -> AppResult<bool> {
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

        async fn release_location_operation_ownership(
            &self,
            operation_id: &str,
        ) -> AppResult<u64> {
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
                detail: fell_back
                    .then(|| "a cache-bypassed read-back could not run".to_string()),
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
        }
    }

    fn two_title_plan() -> OperationWorkPlan {
        OperationWorkPlan::new(vec![
            planned_title("title-1", 1, 2),
            planned_title("title-2", 2, 1),
        ])
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
        assert_eq!(operation_states.first(), Some(&LocationOperationState::Preparing));
        assert_eq!(operation_states.last(), Some(&LocationOperationState::Completed));
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
        assert_eq!(mover.moved(), vec!["/destination/title-2/0.mkv".to_string()]);
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
    async fn a_failed_verification_stops_the_operation_before_any_catalog_change() {
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
        assert!(
            reconciler.reconciled.lock().expect("lock").is_empty(),
            "the catalog is never updated for a title whose content did not verify"
        );
        assert_eq!(
            store
                .checkpoint("op-1", "title-1")
                .expect("the failing title should have a checkpoint")
                .state,
            TitleCheckpointState::Failed
        );
        assert!(
            store.checkpoint("op-1", "title-2").is_none(),
            "the operation stops rather than continuing past a failure"
        );
        assert_eq!(store.open_claims(), 0);
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
