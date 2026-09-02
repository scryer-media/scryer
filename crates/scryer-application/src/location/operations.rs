//! The application-level root-move API: preview, confirm-and-start, cancel, and
//! restart resume (US2, FR-030–033, FR-076, FR-083).
//!
//! These are the use-case methods T034's GraphQL calls. Nothing here knows about
//! GraphQL; everything here knows about the catalog, the settings the operator
//! chose, and the filesystem the plan describes.
//!
//! # The shape of a root move
//!
//! ```text
//! preview_root_move  ─▶ classify every selected title (FR-015)
//!                       calculate destination folders (FR-013)
//!                       walk sources, probe volumes, detect hardlinks
//!                       ─▶ LocationPlan (fingerprinted) + RootMoveExecutionPlan
//!
//! start_root_move    ─▶ re-preview, compare fingerprints (FR-081)
//!                       persist the operation row + the execution plan JSON
//!                       spawn the runner  ─▶ LocationOperationRunner
//!
//! cancel_location_operation ─▶ persists the request; the runner stops at the
//!                              next title checkpoint (FR-092)
//!
//! resume_interrupted_location_operations ─▶ boot hook: every non-terminal
//!                              operation picks up from its checkpoints (FR-033)
//! ```
//!
//! # What a resume refuses, and what it declines
//!
//! Resuming twice is the dangerous mistake: the persisted ownership claim is
//! idempotent for the same operation — which is what lets a resume re-claim
//! what it already holds — so nothing in the store would stop a second runner
//! from walking the same checkpoints. An in-process runner registry is what
//! refuses that, as an error.
//!
//! Everything else a resume cannot do it *declines*, with a reason, leaving the
//! operation exactly as interrupted as it found it: already finished, no stored
//! plan, or a root whose volume is not mounted. That last one is the boot case
//! that matters — spawning into an unmounted share would fail every copy and
//! drive the operation terminally `Failed`, turning a boot-order accident into
//! a permanent one.
//!
//! # Why the preview is rebuilt on start
//!
//! The confirmation the client sends back is a fingerprint, not a plan. Trusting
//! a client-supplied plan would let a caller confirm one thing and execute
//! another, so start rebuilds the preview from current state and only proceeds
//! when the fingerprint still matches (FR-081, C2). That also means the
//! execution plan persisted with the operation is always one Scryer derived
//! itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scryer_domain::{
    DomainEventPayload, JobRunCompletedEventData, JobRunFailedEventData, JobRunStartedEventData,
    LibraryPermission, MediaFacet, Title, User,
};

use crate::domain_events::{DomainEventActor, new_job_run_domain_event};

use crate::location::adoption::{
    AdoptionFileVerifier, DestinationFileFact, TrackedMediaFact, choose_adoption_folder,
    match_title_adoption,
};
use crate::location::classify::{
    DestinationLibraryFacts, DestinationRequest, SelectionClassification, TitleClassificationFacts,
    TitleLocationClass, classify_selection,
};
use crate::location::collisions::{
    CollisionNaming, ContentFacts, DestinationItem, FullHash, PathCaseRule, RecycleAvailability,
};
use crate::location::execution::{
    ImportFilePermissionsApplier, PostMergeWorkRequest, PostMergeWorkScheduler, RootMoveAdmission,
    RootMoveCatalog, RootMoveFileMover, RootMoveReconciler, RecycleBinSourceRecycler,
    TitlePlacementSnapshot,
};
use crate::location::media_server_refresh::{
    LocationMediaServerRefresh, MediaServerRefreshRequest, notify_media_servers_for_operation,
};
use crate::location::merge::engine::{MergePlan, plan_merge};
use crate::location::merge::summary::PostMergeWork;
use crate::location::executor::{LocationOperationRunner, OperationRunOutcome};
use crate::location::hardlinks::detect_hardlinks;
use crate::location::identity::{
    DestinationIdentityOutcome, DestinationTitleCandidate, IdentityRedirects, SourceTitleIdentity,
    detect_destination_titles,
};
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    VerificationDepth,
};
use crate::location::ownership_guard::OwnedEntity;
use crate::location::preview::{
    FreeSpaceEstimate, FreeSpaceRequest, LocationPlan, PlanConfirmationError,
    PlanConfirmationRequest, SystemVolumeProbe, estimate_free_space,
};
use crate::location::root_move::{
    PlannedRootMove, RootMoveExecutionPlan, RootMovePlanRequest, RootMoveTitleDraft, SourceFile,
    build_root_move_plan,
};
use crate::location::transfer_effects::{TitleAssociationFacts, converts_facet};
use crate::location::verify::{VerifiedCopier, same_filesystem};
use crate::services::AppUseCase;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

/// Every location operation verifies at full depth, whatever the operator's
/// verification preference says.
///
/// # Why the preference does not reach here
///
/// The configured depth (`VERIFICATION_DEPTH_KEY`, FR-042) governs the
/// **download-client import copy**: a file Scryer has just acquired, which
/// still exists at the download client if the copy turns out to be wrong. A
/// location operation is the opposite situation. It moves content the user
/// already has — the only copy, in the library, sometimes for years — and the
/// source is recycled once the destination verifies. Proving that destination
/// with the sampled head+tail floor instead of a full read-back is a risk taken
/// with banked data on the user's behalf, and it is not the operator's to take
/// per-installation: a lost library file is a materially worse outcome than a
/// re-downloadable import.
///
/// So every workflow in this subsystem — root move, root change, cross-library
/// transfer, adoption, and consolidation when it lands — plans
/// [`VerificationDepth::Full`] and never reads the preference.
///
/// This is a floor, not a guarantee of the *achieved* depth. When a full
/// read-back cannot run, [`crate::location::verify`] still falls back to the
/// quick floor and records the fallback on the file's verification record and
/// on the operation's counter (FR-043) — that is a capability limit the user is
/// told about, which is a different thing from a preference that quietly asked
/// for less.
pub const LOCATION_OPERATION_VERIFICATION_DEPTH: VerificationDepth = VerificationDepth::Full;

/// What the caller asks to preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootMovePreviewRequest {
    /// The selection, in the order the client submitted it.
    pub title_ids: Vec<String>,
    pub destination: DestinationRequest,
}

/// Everything a preview returns: the fingerprinted plan the user confirms, the
/// grouped classification (FR-015), and the instruction set start would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootMovePreview {
    pub plan: LocationPlan,
    pub classification: SelectionClassification,
    pub warnings: Vec<String>,
    /// Kept out of the client payload; start rebuilds it rather than trusting a
    /// round-trip.
    pub execution: RootMoveExecutionPlan,
}

/// The confirmation a client sends back with the fingerprint it previewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRootMoveRequest {
    pub title_ids: Vec<String>,
    pub destination: DestinationRequest,
    pub confirmation: PlanConfirmationRequest,
}

/// A started operation. Asynchronous by contract: the caller gets an identifier
/// and watches Activity (FR-030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationOperationAccepted {
    pub operation: LocationOperation,
    pub plan: LocationPlan,
}

impl AppUseCase {
    /// Build the shared preview for a root move (FR-012, FR-080–082).
    pub async fn preview_root_move(
        &self,
        actor: &User,
        request: RootMovePreviewRequest,
    ) -> AppResult<RootMovePreview> {
        self.preview_location_change(actor, request, LocationExecutionMode::MoveWithScryer)
            .await
    }

    /// Build the shared preview for **Files are already there** (US3,
    /// FR-050–053).
    ///
    /// Same preview model, same fingerprint, same confirmation rules as a
    /// managed move (FR-051) — the mode is what changes, and the mode is what
    /// the user chose in the move workflow (FR-011).
    pub async fn preview_adoption(
        &self,
        actor: &User,
        request: RootMovePreviewRequest,
    ) -> AppResult<RootMovePreview> {
        self.preview_location_change(actor, request, LocationExecutionMode::FilesAlreadyThere)
            .await
    }

    async fn preview_location_change(
        &self,
        actor: &User,
        request: RootMovePreviewRequest,
        mode: LocationExecutionMode,
    ) -> AppResult<RootMovePreview> {
        let planned = self.plan_root_move(actor, &request, mode).await?;
        Ok(RootMovePreview {
            plan: planned.planned.plan,
            classification: planned.classification,
            warnings: planned.planned.warnings,
            execution: planned.planned.execution,
        })
    }

    /// Confirm a previewed root move and start it (FR-030, FR-081).
    ///
    /// A blocked or unresolved title in the selection stops the start (FR-016);
    /// a fingerprint that no longer matches stops it too, and the caller must
    /// re-preview.
    pub async fn start_root_move(
        &self,
        actor: &User,
        request: StartRootMoveRequest,
    ) -> AppResult<LocationOperationAccepted> {
        self.start_location_change(actor, request, LocationExecutionMode::MoveWithScryer)
            .await
    }

    /// Confirm a previewed adoption and start it (US3, FR-052).
    ///
    /// The refusal a title with unaccounted media produces is the shared one:
    /// its plan carries [`crate::location::preview::PlanItemKind::Blocked`]
    /// items, so `confirm` returns
    /// [`PlanConfirmationError::Blocked`] before anything is persisted.
    pub async fn start_adoption(
        &self,
        actor: &User,
        request: StartRootMoveRequest,
    ) -> AppResult<LocationOperationAccepted> {
        self.start_location_change(actor, request, LocationExecutionMode::FilesAlreadyThere)
            .await
    }

    async fn start_location_change(
        &self,
        actor: &User,
        request: StartRootMoveRequest,
        mode: LocationExecutionMode,
    ) -> AppResult<LocationOperationAccepted> {
        let preview_request = RootMovePreviewRequest {
            title_ids: request.title_ids.clone(),
            destination: request.destination.clone(),
        };
        let planned = self.plan_root_move(actor, &preview_request, mode).await?;
        let plan = planned.planned.plan;

        plan.confirm(&request.confirmation)
            .map_err(confirmation_error)?;

        if planned.planned.execution.titles.is_empty() {
            return Err(AppError::Validation(
                "this selection has nothing to move".to_string(),
            ));
        }

        // Activity's job run is opened before the row it belongs to, so the
        // accepted payload can name it and the client can follow the operation
        // from the jobs list the moment the mutation returns (FR-091).
        let operation_id = scryer_domain::Id::new().0;
        let titles_total = planned.planned.execution.titles.len() as i64;
        let job_run = self
            .open_location_operation_job_run(
                &operation_id,
                titles_total,
                LocationJobRunActor::Confirmed(actor),
            )
            .await?;

        let now = chrono::Utc::now();
        let operation = LocationOperation {
            id: operation_id,
            // The planner decides this: a selection that changes library is a
            // cross-library transfer in Activity, even though both types walk
            // the same runner (FR-091).
            operation_type: plan.header.operation_type,
            mode: plan.header.mode,
            state: LocationOperationState::Queued,
            initiated_by_user_id: Some(actor.id.clone()),
            source_library_id: plan.header.source_library_id.clone(),
            destination_library_id: plan.header.destination_library_id.clone(),
            source_root_id: plan.header.source_root_id.clone(),
            destination_root_id: plan.header.destination_root_id.clone(),
            plan_fingerprint: plan.fingerprint.0.clone(),
            verification_depth: plan.verification.depth,
            verification_fallback_count: 0,
            counters: LocationOperationCounters {
                titles_total,
                files_total: planned
                    .planned
                    .execution
                    .titles
                    .iter()
                    .map(|title| title.files.len() as i64)
                    .sum(),
                bytes_total: planned.planned.execution.moved_bytes() as i64,
                // The two counters the instruction set cannot carry: titles the
                // preview called no-ops or could not resolve produce no work,
                // but Activity still reports them (FR-091). The runner
                // recomputes the row from the same plan on its first progress
                // write, so these never drift from what it will report.
                no_ops: planned.planned.execution.no_op_titles,
                unresolved: planned.planned.execution.unresolved_titles,
                ..LocationOperationCounters::default()
            },
            detail: None,
            // Activity reads the operation through this run (FR-091). The
            // column stays nullable because an operation created before T036 —
            // or one driven by an administrative path with no run — is still a
            // valid row.
            job_run_id: Some(job_run.id.clone()),
            // Location operations are their own subsystem with their own
            // persisted row and checkpoints; nothing reads them through the
            // workflow-operation ledger, so this stays unset.
            workflow_operation_id: None,
            cancel_requested: false,
            cancel_requested_at: None,
            confirmed_at: Some(now),
            started_at: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        let persisted = async {
            let plan_json = serde_json::to_string(&planned.planned.execution)
                .map_err(|error| AppError::Repository(error.to_string()))?;
            self.services
                .library
                .location_operations
                .create_location_operation(&operation, Some(&plan_json))
                .await
        }
        .await;
        if let Err(error) = persisted {
            // The run was opened first so the payload could name it; an
            // operation that never reached the database must not leave a
            // running row in Activity forever.
            self.close_location_operation_job_run(
                &job_run,
                crate::JobRunStatus::Failed,
                "The location operation could not be started.".to_string(),
                Some(error.to_string()),
                None,
            )
            .await;
            return Err(error);
        }

        self.spawn_location_operation(operation.id.clone(), planned.planned.execution);

        Ok(LocationOperationAccepted { operation, plan })
    }

    /// Request cancellation. The runner stops at the next safe title checkpoint
    /// (FR-092); completed titles stay consistent.
    pub async fn cancel_location_operation(
        &self,
        actor: &User,
        operation_id: &str,
    ) -> AppResult<bool> {
        let operation = self
            .location_operation(operation_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("location operation {operation_id}")))?;
        self.require_location_operation_permission(actor, &operation)
            .await?;
        self.services
            .library
            .location_operations
            .request_location_operation_cancel(operation_id)
            .await
    }

    /// Read one operation row.
    pub async fn location_operation(
        &self,
        operation_id: &str,
    ) -> AppResult<Option<LocationOperation>> {
        self.services
            .library
            .location_operations
            .get_location_operation(operation_id)
            .await
    }

    /// Per-title checkpoints in plan order, for the operation's Activity view
    /// (FR-091).
    pub async fn location_operation_checkpoints(
        &self,
        operation_id: &str,
    ) -> AppResult<Vec<crate::location::model::TitleCheckpoint>> {
        self.services
            .library
            .location_operations
            .list_location_title_checkpoints(operation_id)
            .await
    }

    /// Resume one persisted operation from its last verified checkpoint
    /// (FR-033).
    ///
    /// Reports [`LocationResumeDecision::NotResumable`] — never an error, and
    /// never a silent nothing — when the operation is unknown, terminal, stored
    /// without a plan this build can read, or sitting on a volume that is not
    /// mounted right now.
    /// The one case that *is* an error is a second resume of an operation whose
    /// runner is still alive: that is a caller mistake with a real consequence
    /// (two runners over one set of checkpoints), so it is refused rather than
    /// quietly reported as "nothing to do".
    ///
    /// A resumable operation gets a *fresh* Activity job run before the plan is
    /// handed back, and the operation row is repointed at it. Job runs are
    /// per-execution everywhere else in Scryer — the boot reconciler fails every
    /// non-terminal run it finds, so the run an interrupted operation started
    /// under is already `failed` by the time a resume happens, and reopening it
    /// would rewrite a settled Activity row. One run per attempt also keeps the
    /// jobs list honest about how many times a move was picked back up. It is
    /// opened last, after every refusal, so a refused resume leaves no run
    /// behind.
    pub async fn resume_location_operation(
        &self,
        operation_id: &str,
    ) -> AppResult<LocationResumeDecision> {
        let Some(operation) = self.location_operation(operation_id).await? else {
            return Ok(LocationResumeDecision::not_resumable(
                "this operation no longer exists",
            ));
        };
        if operation.state.is_terminal() {
            return Ok(LocationResumeDecision::not_resumable(
                "this operation has already finished, so there is nothing to resume",
            ));
        }
        // A cross-library transfer is a root move that also flips catalog
        // ownership (FR-056), and an adoption is one that proves the destination
        // instead of writing it (FR-050): all three are planned by the same
        // planner, persisted as the same `RootMoveExecutionPlan`, and walked by
        // the same runner, so they resume here. Every *other* type resumes
        // through its own phase and must not be run under these rules.
        //
        // A root change joins them (US4): its instruction set *is* a
        // `RootMoveExecutionPlan` whose two sides carry the same root id, and
        // its root-scoped tail rides on the same JSON and re-runs idempotently
        // (FR-087).
        if !crate::location::root_change_execution::resumes_through_root_move_runner(
            operation.operation_type,
        ) {
            return Ok(LocationResumeDecision::not_resumable(format!(
                "a {} operation does not resume through the root-move runner",
                operation.operation_type.as_str()
            )));
        }
        // A runner that is still alive owns these checkpoints. Re-claiming
        // ownership is idempotent for the same operation, so nothing further
        // down would stop a second runner from walking the same plan.
        if self.runtime.library.location_runners.is_live(operation_id) {
            return Err(AppError::Validation(format!(
                "location operation {operation_id} is still running; wait for it to stop before resuming it"
            )));
        }

        let Some(plan_json) = self
            .services
            .library
            .location_operations
            .get_location_operation_plan_json(operation_id)
            .await?
        else {
            return Ok(LocationResumeDecision::not_resumable(
                "this operation was stored without its plan, so there is nothing to resume",
            ));
        };
        // A stored plan this build cannot read is the same situation for the
        // user as one that was never stored at all: there is nothing to carry
        // on from. Raising a repository error at somebody who pressed "resume"
        // would say "something broke" about a row that is merely unreadable
        // *here* — and the boot hook, which logs errors and moves on, would
        // report it as a failure to read rather than as a move left for the
        // user. The row stays interrupted either way, so a build that can read
        // the plan still resumes it.
        let plan: RootMoveExecutionPlan = match serde_json::from_str(&plan_json) {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!(
                    operation_id = %operation_id,
                    error = %error,
                    "an interrupted location operation's stored plan could not be read; it stays interrupted"
                );
                return Ok(LocationResumeDecision::not_resumable(
                    "this operation's stored plan cannot be read, so there is nothing to resume",
                ));
            }
        };

        // FR-033 is about picking work back up, not about deciding it is over.
        // A root whose volume has not mounted yet — the ordinary shape of a
        // boot — would fail every copy and drive the operation to a terminal
        // Failed, turning a boot-order accident into a permanent one.
        if let Some(unavailable) = unavailable_plan_root(&plan, operation.mode).await {
            return Ok(LocationResumeDecision::not_resumable(format!(
                "{unavailable} is not available right now, so this operation stays interrupted and can be resumed once it is back"
            )));
        }

        // The resumed attempt gets its own run, and the operation points at the
        // latest one. A resume is a continuation of work the user already
        // confirmed, so the run is attributed to that user but triggered by the
        // system — the boot hook has no actor at all, and a resume mutation is
        // asking Scryer to carry on rather than to start something new.
        let job_run = self
            .open_location_operation_job_run(
                operation_id,
                operation.counters.titles_total,
                LocationJobRunActor::Resumed(operation.initiated_by_user_id.clone()),
            )
            .await?;
        self.services
            .library
            .location_operations
            .set_location_operation_job_run(operation_id, &job_run.id)
            .await?;

        Ok(LocationResumeDecision::Resume(Box::new(plan)))
    }

    /// Boot hook, first half: let go of ownership claims whose operation has
    /// already settled (FR-084).
    ///
    /// [`crate::location::executor::LocationOperationRunner::run`] writes the
    /// terminal state and *then* releases ownership, so a process that dies
    /// between those two writes — or a release that fails on its own — leaves a
    /// finished operation still owning its titles and roots. Nothing else ever
    /// lets go of them: a terminal operation is not resumable, so the stale
    /// claims would refuse scans, imports, renames, and every later location
    /// operation over those entities for the life of the installation. A claim
    /// whose operation row is gone entirely is released for the same reason.
    ///
    /// Non-terminal operations keep their claims: those are the ones a resume
    /// is about to re-claim, and re-claiming is idempotent for the same
    /// operation.
    ///
    /// Returns how many operations were released.
    async fn release_settled_location_ownership_claims(&self) -> AppResult<usize> {
        let claims = self.location_ownership_open_claims().await?;
        if claims.is_empty() {
            return Ok(0);
        }
        let mut operation_ids: Vec<String> =
            claims.into_iter().map(|claim| claim.operation_id).collect();
        operation_ids.sort();
        operation_ids.dedup();

        let mut released = 0usize;
        for operation_id in operation_ids {
            let settled = match self.location_operation(&operation_id).await? {
                Some(operation) => operation.state.is_terminal(),
                None => true,
            };
            if !settled {
                continue;
            }
            let entities = self
                .services
                .library
                .location_operations
                .release_location_operation_ownership(&operation_id)
                .await?;
            self.runtime
                .library
                .location_ownership
                .release_operation(&operation_id);
            if entities > 0 {
                tracing::info!(
                    operation_id = %operation_id,
                    entities,
                    "released ownership claims left behind by a location operation that had already finished"
                );
                released += 1;
            }
        }
        Ok(released)
    }

    /// Boot hook: pick every interrupted location operation back up (FR-033).
    ///
    /// Returns how many were resumed. Operations whose plan cannot be read are
    /// left alone and logged rather than failed here, because a startup path
    /// must not decide on its own that a user's half-finished move is over.
    pub async fn resume_interrupted_location_operations(&self) -> AppResult<usize> {
        // Before anything is picked back up: claims held by operations that
        // already finished are nobody's, and no later path releases them
        // (FR-084). A failure here is logged rather than propagated — it must
        // not stop the resumes this hook exists for.
        if let Err(error) = self.release_settled_location_ownership_claims().await {
            tracing::warn!(
                error = %error,
                "could not release the ownership claims left behind by finished location operations"
            );
        }

        let operations = LocationOperationRunner::resumable_operations(
            self.services.library.location_operations.as_ref(),
        )
        .await?;

        let mut resumed = 0usize;
        for operation in operations {
            match self.resume_location_operation(&operation.id).await {
                Ok(LocationResumeDecision::Resume(plan)) => {
                    tracing::info!(
                        operation_id = %operation.id,
                        state = operation.state.as_str(),
                        "resuming an interrupted location operation from its last verified checkpoint"
                    );
                    self.spawn_location_operation(operation.id.clone(), *plan);
                    resumed += 1;
                }
                // Deliberately left interrupted, not failed. The operation
                // stays resumable — by the next boot, or by the user from
                // Activity once whatever is missing is back. Nothing here
                // schedules a retry: a startup path that decided on its own
                // that a user's half-finished move was over would be worse than
                // one that waits to be asked.
                Ok(LocationResumeDecision::NotResumable(reason)) => tracing::warn!(
                    operation_id = %operation.id,
                    operation_type = operation.operation_type.as_str(),
                    reason = %reason,
                    "an interrupted location operation was not resumed and is left for the user"
                ),
                Err(error) => tracing::warn!(
                    operation_id = %operation.id,
                    error = %error,
                    "failed to read an interrupted location operation's plan"
                ),
            }
        }
        Ok(resumed)
    }

    /// Run one root move to a terminal state or a safe stop.
    ///
    /// Public so a test — and, later, a synchronous administrative path — can
    /// drive an operation without a background task.
    ///
    /// Whatever the run settles on is also what closes the operation's Activity
    /// job run (FR-091). The run id is read once, before any work starts, so a
    /// resume that repoints the operation partway through cannot make this call
    /// finalize somebody else's row.
    pub async fn run_root_move(
        &self,
        operation_id: &str,
        plan: &RootMoveExecutionPlan,
    ) -> AppResult<OperationRunOutcome> {
        let row = self.location_operation(operation_id).await.ok().flatten();
        let job_run_id = row
            .as_ref()
            .and_then(|operation| operation.job_run_id.clone());
        // The mode is what decides whether this run copies bytes or proves the
        // ones already there (FR-050). It is read off the persisted row rather
        // than re-derived, so a resumed adoption is still an adoption.
        let mode = row
            .as_ref()
            .map(|operation| operation.mode)
            .unwrap_or(LocationExecutionMode::MoveWithScryer);

        let outcome = self
            .execute_root_move(operation_id, plan, mode, job_run_id.as_deref())
            .await;
        if let Some(job_run_id) = job_run_id {
            self.close_location_operation_job_run_for(&job_run_id, &outcome)
                .await;
        }
        // FR-088, last: the operation has settled and its Activity row is
        // closed, so nothing below can change either. A run that stopped short
        // of terminal notifies nothing — its checkpoints are durable and the
        // terminal run that follows covers them.
        if let Ok(outcome) = outcome.as_ref() {
            notify_media_servers_for_operation(
                self.services.library.location_operations.as_ref(),
                &AppUseCaseMediaServerRefresh { app: self.clone() },
                operation_id,
                outcome.state,
            )
            .await;
        }
        outcome
    }

    async fn execute_root_move(
        &self,
        operation_id: &str,
        plan: &RootMoveExecutionPlan,
        mode: LocationExecutionMode,
        job_run_id: Option<&str>,
    ) -> AppResult<OperationRunOutcome> {
        let adopting = mode == LocationExecutionMode::FilesAlreadyThere;
        let catalog = AppUseCaseRootMoveCatalog { app: self.clone() };
        let recycler = self.root_move_recycler(plan).await;
        let permissions = self.root_move_permissions(plan).await?;
        // US3: adoption copies nothing. The same runner walks the same plan and
        // the same reconciler flips catalog ownership at the same per-title
        // checkpoint; only the per-file step differs, because there is nothing
        // to move and everything to prove (FR-050, FR-053).
        //
        // Both movers are bound here rather than boxed so the runner keeps
        // borrowing a concrete `&dyn TitleFileMover` for the whole run.
        let copy_mover;
        let adoption_mover;
        let mover: &dyn crate::location::executor::TitleFileMover = if adopting {
            adoption_mover = AdoptionFileVerifier::new(plan)
                .with_catalog(Arc::new(AppUseCaseRootMoveCatalog { app: self.clone() }));
            &adoption_mover
        } else {
            // The mover gets its own catalog handle so each verified copy's
            // hashes land on the media file as they are produced (FR-041,
            // migration 0205), instead of the backfill job reading every moved
            // file a second time.
            copy_mover = RootMoveFileMover::new(VerifiedCopier::new(), permissions.clone())
                .with_catalog(Arc::new(AppUseCaseRootMoveCatalog { app: self.clone() }));
            &copy_mover
        };
        let admission = {
            let admission = RootMoveAdmission::new(plan, &catalog);
            if adopting {
                admission.adopting()
            } else {
                admission
            }
        };
        let reconciler = RootMoveReconciler::new(
            plan,
            &catalog,
            self.services.library.location_operations.as_ref(),
            &recycler,
        )
        .with_permissions(permissions)
        // US7: the merge engine and the Group 6 scheduler the executor hands
        // the returned work list to.
        .with_merges(self.services.library.title_merges.as_ref())
        .with_post_merge_work(Arc::new(AppUseCasePostMergeWork { app: self.clone() }));

        // Activity's run mirrors the same pulse the operation row gets, so a
        // long copy shows a moving row in the jobs list instead of "queued"
        // until it finishes (FR-091).
        let observer = job_run_id.map(|job_run_id| LocationJobRunProgress {
            app: self.clone(),
            job_run_id: job_run_id.to_string(),
        });

        // US4: the one act a root change performs that is not about a title —
        // retiring the source location and flipping the root's configured path
        // once every title has recycled (FR-087). Bound to the run rather than
        // performed after it, so the operation is never readable as finished
        // before its root has actually moved.
        let epilogue = plan
            .root_change
            .as_ref()
            .map(|tail| crate::location::root_change_execution::RootChangeEpilogue {
                app: self,
                tail,
            });

        let mut runner = LocationOperationRunner::new(
            self.services.library.location_operations.as_ref(),
            mover,
            &admission,
            &reconciler,
        )
        .with_ownership_registry(&self.runtime.library.location_ownership);
        if let Some(observer) = observer.as_ref() {
            runner = runner.with_progress_observer(observer);
        }
        if let Some(epilogue) = epilogue.as_ref() {
            runner = runner.with_epilogue(epilogue);
        }
        runner.run(operation_id, &plan.to_work_plan()).await
    }

    /// Start the background runner for an operation, unless one is already
    /// alive for it in this process (FR-033).
    pub fn spawn_location_operation(&self, operation_id: String, plan: RootMoveExecutionPlan) {
        let Some(guard) = self.runtime.library.location_runners.begin(&operation_id) else {
            tracing::warn!(
                operation_id = %operation_id,
                "refused to start a second runner for a location operation that is already running"
            );
            return;
        };

        let app = self.clone();
        tokio::spawn(async move {
            // The guard releases the slot when this task ends, however it ends
            // — including a panic — so a crashed runner never leaves an
            // operation permanently unresumable.
            let _guard = guard;
            if let Err(error) = app.run_root_move(&operation_id, &plan).await {
                tracing::error!(
                    operation_id = %operation_id,
                    error = %error,
                    "a location operation stopped with an error"
                );
            }
        });
    }

    // ── Activity job runs (FR-091) ───────────────────────────────────────────
    //
    // Location operations are started from their own mutation and never from
    // the generic job trigger, so they open and close their own runs the way
    // the title-rename and application-upgrade jobs do
    // (`AppUseCase::start_rename_titles_job`,
    // `AppUseCase::start_application_upgrade_job`). Everything Activity needs —
    // the tracker entry, the started/completed/failed domain events — is
    // written here rather than by the jobs engine, which never sees these runs.

    /// Open the run one execution of a location operation reports through.
    pub(super) async fn open_location_operation_job_run(
        &self,
        operation_id: &str,
        titles_total: i64,
        actor: LocationJobRunActor<'_>,
    ) -> AppResult<crate::JobRunRecord> {
        let now = chrono::Utc::now();
        let mut run = crate::JobRunRecord {
            id: scryer_domain::Id::new().0,
            job_key: crate::JobKey::LocationOperation,
            operation_type: format!("location_operation:{operation_id}"),
            status: crate::JobRunStatus::Running,
            trigger_source: actor.trigger_source(),
            actor_user_id: actor.actor_user_id(),
            progress_json: serde_json::to_string(&serde_json::json!({
                "status": crate::JobRunStatus::Running.as_str(),
                "phase": "queued",
                "operationId": operation_id,
                "titlesTotal": titles_total,
                "titlesProcessed": 0,
            }))
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;

        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(crate::JobRun::from_record(&run, None))
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                actor.event_actor(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;
        Ok(run)
    }

    /// Close the run for an execution that has settled, translating the
    /// operation's terminal state into the job vocabulary Activity speaks.
    ///
    /// A cancel is a *warning*, not a failure: the user asked the move to stop
    /// and everything it had finished is intact, which is the same reading a
    /// canceled library scan gets (`JobRun::from_record`). Only an error — or a
    /// run that never returned an outcome at all — is a failure.
    async fn close_location_operation_job_run_for(
        &self,
        job_run_id: &str,
        outcome: &AppResult<OperationRunOutcome>,
    ) {
        let Some(run) = self.location_operation_job_run(job_run_id).await else {
            return;
        };

        let (status, summary_text, error_text, counters) = match outcome {
            Ok(outcome) => {
                let summary = describe_location_operation_outcome(outcome);
                match outcome.state {
                    LocationOperationState::Completed => (
                        crate::JobRunStatus::Completed,
                        summary,
                        None,
                        Some(outcome.counters),
                    ),
                    LocationOperationState::CompletedWithWarnings => (
                        crate::JobRunStatus::Warning,
                        summary,
                        None,
                        Some(outcome.counters),
                    ),
                    LocationOperationState::Canceled => (
                        crate::JobRunStatus::Warning,
                        summary,
                        None,
                        Some(outcome.counters),
                    ),
                    LocationOperationState::Failed => (
                        crate::JobRunStatus::Failed,
                        summary,
                        Some(outcome.detail.clone().unwrap_or_else(|| {
                            "the location operation stopped on an error".to_string()
                        })),
                        Some(outcome.counters),
                    ),
                    // A non-terminal outcome means the runner handed the
                    // operation back for someone else to continue; the run stays
                    // open so the next execution reports on it.
                    _ => return,
                }
            }
            Err(error) => (
                crate::JobRunStatus::Failed,
                "The location operation stopped with an error.".to_string(),
                Some(error.to_string()),
                None,
            ),
        };

        self.close_location_operation_job_run(
            &run,
            status,
            summary_text,
            error_text,
            counters.as_ref(),
        )
        .await;
    }

    /// Mirror one progress pulse onto the operation's Activity run.
    ///
    /// Everything here is best effort and nothing here returns: the jobs list
    /// showing a slightly stale row is a cosmetic problem, and failing a move
    /// over one would not be.
    async fn write_location_operation_job_run_progress(
        &self,
        job_run_id: &str,
        snapshot: &crate::location::executor::OperationProgressSnapshot<'_>,
    ) {
        let Some(mut run) = self.location_operation_job_run(job_run_id).await else {
            return;
        };
        if run.status.is_terminal() {
            // Something already settled this run; a pulse must not reopen it.
            return;
        }

        let counters = &snapshot.counters;
        run.progress_json = serde_json::to_string(&serde_json::json!({
            "status": run.status.as_str(),
            "phase": snapshot.state.as_str(),
            "operationId": snapshot.operation_id,
            "titlesTotal": counters.titles_total,
            "titlesProcessed": counters.titles_processed,
            "filesTotal": counters.files_total,
            "filesProcessed": counters.files_processed,
            "bytesTotal": counters.bytes_total,
            "bytesProcessed": counters.bytes_processed,
        }))
        .ok();
        run.updated_at = chrono::Utc::now();

        match self.services.events.job_runs.update_job_run(&run).await {
            Ok(updated) => {
                self.runtime
                    .jobs
                    .job_run_tracker
                    .upsert_active_run(crate::JobRun::from_record(&updated, None))
                    .await;
            }
            Err(error) => tracing::warn!(
                job_run_id = %job_run_id,
                error = %error,
                "could not mirror a location operation's progress onto its job run"
            ),
        }
    }

    /// The persisted run, when the repository still has it. A missing run is
    /// logged and skipped: a finished move must never fail because Activity
    /// could not be updated.
    async fn location_operation_job_run(&self, job_run_id: &str) -> Option<crate::JobRunRecord> {
        match self.services.events.job_runs.get_job_run(job_run_id).await {
            Ok(Some(run)) => Some(run),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    job_run_id = %job_run_id,
                    error = %error,
                    "could not read the job run for a location operation"
                );
                None
            }
        }
    }

    /// Write a terminal status onto a location-operation run, mirror it into the
    /// tracker, and announce it.
    pub(super) async fn close_location_operation_job_run(
        &self,
        run: &crate::JobRunRecord,
        status: crate::JobRunStatus,
        summary_text: String,
        error_text: Option<String>,
        counters: Option<&LocationOperationCounters>,
    ) {
        let mut run = run.clone();
        if run.status.is_terminal() {
            // Something already settled this run — the boot reconciler, or a
            // second execution. Rewriting a finished Activity row would lose
            // whichever outcome the user already saw.
            return;
        }

        let now = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&serde_json::json!({
            "status": status.as_str(),
            "phase": "completed",
            "titlesTotal": counters.map(|counters| counters.titles_total).unwrap_or_default(),
            "titlesProcessed": counters.map(|counters| counters.titles_processed).unwrap_or_default(),
        }))
        .ok();
        run.summary_json = counters.and_then(|counters| serde_json::to_string(counters).ok());
        run.summary_text = Some(summary_text.clone());
        run.error_text = error_text.clone();
        run.completed_at = Some(now);
        run.updated_at = now;

        let updated = match self.services.events.job_runs.update_job_run(&run).await {
            Ok(updated) => updated,
            Err(error) => {
                tracing::warn!(
                    job_run_id = %run.id,
                    error = %error,
                    "could not finalize the job run for a location operation"
                );
                return;
            }
        };
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(crate::JobRun::from_record(&updated, None))
            .await;

        let payload = if status == crate::JobRunStatus::Failed {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text,
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                summary_text: Some(summary_text),
            })
        };
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                DomainEventActor::system(),
                updated.id.clone(),
                payload,
            ))
            .await;
    }

    /// Recycle-bin configuration per source root, so a redundant source copy
    /// goes to the bin rather than being deleted (FR-073, C4).
    async fn root_move_recycler(&self, plan: &RootMoveExecutionPlan) -> RecycleBinSourceRecycler {
        let mut configs = BTreeMap::new();
        for title in &plan.titles {
            let Some(root_path) = title.source_root_path.as_deref() else {
                continue;
            };
            if configs.contains_key(root_path) {
                continue;
            }
            configs.insert(
                root_path.to_string(),
                self.recycle_bin_config_for_media_root(Some(root_path)).await,
            );
        }
        RecycleBinSourceRecycler::new(configs)
    }

    /// The operator's configured modes for the destination library, applied to
    /// placed content (FR-031).
    async fn root_move_permissions(
        &self,
        plan: &RootMoveExecutionPlan,
    ) -> AppResult<Arc<dyn crate::location::execution::PlacedContentPermissions>> {
        let Some(title) = plan.titles.first() else {
            return Ok(Arc::new(
                crate::location::execution::NoPlacedContentPermissions,
            ));
        };
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&title.destination_library_id)
            .await?;
        let Some(library) = library else {
            return Ok(Arc::new(
                crate::location::execution::NoPlacedContentPermissions,
            ));
        };
        let permissions = self
            .resolve_import_file_permissions(Some(&library.id), &library.facet)
            .await?;
        Ok(Arc::new(ImportFilePermissionsApplier::new(
            self.services.workflow.file_importer.clone(),
            permissions,
        )))
    }

    /// FR-083: the initiating user must hold management permission for the
    /// source library and every destination library involved.
    pub async fn require_location_operation_permission(
        &self,
        actor: &User,
        operation: &LocationOperation,
    ) -> AppResult<()> {
        for library_id in [
            operation.source_library_id.as_deref(),
            operation.destination_library_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            self.require_library_permission(actor, library_id, LibraryPermission::ManageTitles)
                .await?;
        }
        Ok(())
    }
}

// ── Activity job runs ────────────────────────────────────────────────────────

/// Mirrors the runner's throttled progress pulse onto one Activity job run.
struct LocationJobRunProgress {
    app: AppUseCase,
    job_run_id: String,
}

#[async_trait::async_trait]
impl crate::location::executor::OperationProgressObserver for LocationJobRunProgress {
    async fn observe(&self, snapshot: crate::location::executor::OperationProgressSnapshot<'_>) {
        self.app
            .write_location_operation_job_run_progress(&self.job_run_id, &snapshot)
            .await;
    }
}

/// What a resume decided to do (FR-033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationResumeDecision {
    /// Pick the operation back up with this plan.
    ///
    /// Boxed because the execution plan is much larger than the refusal, and an
    /// enum sized by its biggest variant would be paid for on every call.
    Resume(Box<RootMoveExecutionPlan>),
    /// Nothing was started, and why. The operation is left exactly as it was —
    /// still interrupted, still resumable later.
    NotResumable(String),
}

impl LocationResumeDecision {
    fn not_resumable(reason: impl Into<String>) -> Self {
        Self::NotResumable(reason.into())
    }

    /// The plan, when the resume decided to run.
    pub fn plan(self) -> Option<RootMoveExecutionPlan> {
        match self {
            Self::Resume(plan) => Some(*plan),
            Self::NotResumable(_) => None,
        }
    }
}

/// The first root path in `plan` that is not a directory right now, described
/// so the reason a resume gives names it.
///
/// The parent-first stat the full-hash backfill job uses is the same idea: an
/// unmounted share answers "no such directory" immediately, where opening
/// content under it can block on a dead mount.
///
/// An adoption is the one mode that does **not** require its source root: FR-053
/// says a stale or unavailable source mount must not block adoption, and a
/// resume that refused on it would turn that allowance off the moment a process
/// restarted (US3.3, FR-033).
async fn unavailable_plan_root(
    plan: &RootMoveExecutionPlan,
    mode: LocationExecutionMode,
) -> Option<String> {
    let requires_source = mode != LocationExecutionMode::FilesAlreadyThere;
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for title in &plan.titles {
        for (label, root) in [
            (
                "the source root",
                requires_source
                    .then_some(title.source_root_path.as_deref())
                    .flatten(),
            ),
            (
                "the destination root",
                title.destination_root_path.as_deref(),
            ),
        ] {
            let Some(root) = root else {
                continue;
            };
            if !seen.insert(root) {
                continue;
            }
            let path = stored_path_to_path_buf(root);
            let available = tokio::fs::metadata(&path)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !available {
                return Some(format!("{label} {root}"));
            }
        }
    }
    None
}

/// Who a location-operation job run belongs to, and why it was opened.
pub(super) enum LocationJobRunActor<'a> {
    /// A user confirmed a previewed plan: a manual trigger, attributed to them.
    Confirmed(&'a User),
    /// An execution picked back up. The user who confirmed the operation still
    /// owns it, but Scryer — a resume mutation or the boot hook — is what
    /// started this attempt, and the boot hook has no actor to speak of.
    Resumed(Option<String>),
}

impl LocationJobRunActor<'_> {
    fn trigger_source(&self) -> crate::JobTriggerSource {
        match self {
            Self::Confirmed(_) => crate::JobTriggerSource::Manual,
            Self::Resumed(_) => crate::JobTriggerSource::SystemInternal,
        }
    }

    fn actor_user_id(&self) -> Option<String> {
        match self {
            Self::Confirmed(actor) => Some(actor.id.clone()),
            Self::Resumed(user_id) => user_id.clone(),
        }
    }

    fn event_actor(&self) -> DomainEventActor {
        match self {
            Self::Confirmed(actor) => DomainEventActor::from(*actor),
            Self::Resumed(_) => DomainEventActor::system(),
        }
    }
}

/// The one-line summary Activity shows for a settled location operation.
fn describe_location_operation_outcome(outcome: &OperationRunOutcome) -> String {
    let counters = &outcome.counters;
    let mut summary = format!(
        "Moved {} of {} title(s) and {} of {} file(s).",
        counters.titles_processed,
        counters.titles_total,
        counters.files_processed,
        counters.files_total
    );
    if counters.titles_blocked > 0 {
        summary.push_str(&format!(" {} blocked.", counters.titles_blocked));
    }
    if counters.no_ops > 0 {
        summary.push_str(&format!(" {} needed no change.", counters.no_ops));
    }
    if counters.unresolved > 0 {
        summary.push_str(&format!(
            " {} still need a decision.",
            counters.unresolved
        ));
    }
    match outcome.state {
        LocationOperationState::Canceled => {
            summary.push_str(" Canceled at a title checkpoint; finished titles are unchanged.");
        }
        LocationOperationState::Failed => {
            if let Some(detail) = outcome.detail.as_deref() {
                summary.push(' ');
                summary.push_str(detail);
            }
        }
        _ => {}
    }
    summary
}

// ── Planning ─────────────────────────────────────────────────────────────────

struct RootMovePlanning {
    planned: PlannedRootMove,
    classification: SelectionClassification,
}

impl AppUseCase {
    async fn plan_root_move(
        &self,
        actor: &User,
        request: &RootMovePreviewRequest,
        mode: LocationExecutionMode,
    ) -> AppResult<RootMovePlanning> {
        if request.title_ids.is_empty() {
            return Err(AppError::Validation(
                "select at least one title to move".to_string(),
            ));
        }
        if request.destination.is_empty() {
            return Err(AppError::Validation(
                "choose a destination library or root".to_string(),
            ));
        }

        // Load the selection, preserving submission order so the preview and the
        // fingerprint see the selection the user made. A repeated id is one
        // title, not two: counting it twice would double its bytes and give it
        // two checkpoints for the same work.
        let mut selection: Vec<String> = Vec::with_capacity(request.title_ids.len());
        let mut titles: Vec<Title> = Vec::with_capacity(request.title_ids.len());
        for title_id in &request.title_ids {
            if selection.iter().any(|seen| seen == title_id) {
                continue;
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            selection.push(title_id.clone());
            titles.push(title);
        }

        let mut libraries: BTreeMap<String, scryer_domain::Library> = BTreeMap::new();
        for title in &titles {
            if !libraries.contains_key(&title.library_id) {
                let library = self
                    .services
                    .catalog
                    .libraries
                    .get_by_id(&title.library_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("library {}", title.library_id))
                    })?;
                libraries.insert(library.id.clone(), library);
            }
        }

        // The destination library: the requested one, or — when the request only
        // names a root — the single library every selected title already sits in.
        let destination_library = match request.destination.library_id.as_deref() {
            Some(library_id) => self
                .services
                .catalog
                .libraries
                .get_by_id(library_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?,
            None => {
                let mut ids: Vec<&str> =
                    titles.iter().map(|title| title.library_id.as_str()).collect();
                ids.sort_unstable();
                ids.dedup();
                match ids.as_slice() {
                    [only] => libraries
                        .get(*only)
                        .cloned()
                        .ok_or_else(|| AppError::NotFound(format!("library {only}")))?,
                    _ => {
                        return Err(AppError::Validation(
                            "a root move spans one library; choose a destination library for a selection that spans several"
                                .to_string(),
                        ));
                    }
                }
            }
        };

        // FR-083, checked before any filesystem work is planned.
        self.require_library_permission(
            actor,
            &destination_library.id,
            LibraryPermission::ManageTitles,
        )
        .await?;
        for library_id in libraries.keys() {
            self.require_library_permission(actor, library_id, LibraryPermission::ManageTitles)
                .await?;
        }

        let destination_facts = DestinationLibraryFacts {
            library_id: destination_library.id.clone(),
            library_name: destination_library.name.clone(),
            facet: destination_library.facet.clone(),
            root_ids: destination_library
                .roots
                .iter()
                .map(|root| root.id.clone())
                .collect(),
        };

        // FR-055: what the destination library already holds, for the titles
        // that are crossing into it. Costs nothing for a same-library root
        // move — the usual US2 case — and one read of the destination library
        // for a transfer, however many titles are selected.
        let destination_identities = self
            .detect_destination_titles_for(&titles, &destination_library.id)
            .await?;

        // FR-060–FR-062: what each crossing title is linked to and what it
        // contains, so the preview can state a disposition instead of leaving
        // the user to wonder. Read only for titles that actually cross a library
        // boundary — a same-library root move changes none of it.
        let association_facts = self
            .transfer_association_facts(&titles, &destination_library)
            .await?;

        // C2 / FR-066 / FR-071: the merge is decided *at preview time*, by the
        // same `plan_merge` the executor runs. A refusal therefore reaches the
        // user before they confirm anything, and the summary they confirm is
        // the decision that will be carried out rather than a description of
        // one.
        let merge_plans = self
            .plan_destination_merges(&titles, &destination_identities)
            .await?;

        // Classification facts, including the FR-086 blockers.
        let mut facts = Vec::with_capacity(titles.len());
        let mut media_files_by_title: BTreeMap<String, Vec<crate::TitleMediaFile>> = BTreeMap::new();
        for title in &titles {
            let media_files = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await?;
            let library_name = libraries
                .get(&title.library_id)
                .map(|library| library.name.clone())
                .unwrap_or_default();
            let owner = self
                .services
                .library
                .location_operations
                .location_ownership_holder(&OwnedEntity::Title(title.id.clone()))
                .await?;

            let mut fact = TitleClassificationFacts::new(
                title.id.clone(),
                title.facet.clone(),
                title.library_id.clone(),
                title.root_folder_id.clone(),
            )
            .with_name(title.name.clone())
            .with_library_name(library_name)
            .with_tags(title.tags.clone())
            .with_associations(
                association_facts
                    .get(&title.id)
                    .copied()
                    .unwrap_or_default(),
            )
            .with_monitored(title.monitored)
            .with_tracked_files(media_files.len() as i64)
            .with_folder_path(
                title
                    .folder_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string),
            );
            if let Some(detail) = self.active_work_blocking_a_move(title).await? {
                fact = fact.with_active_work(detail);
            }
            if let Some(operation_id) = owner {
                fact = fact.with_owned_by_operation(operation_id);
            }
            if let Some(outcome) = destination_identities.get(&title.id) {
                fact = fact.with_destination_identity(outcome.clone());
            }
            // FR-066: the merge engine refused, so the title needs the user
            // before the job can start — with the blocking records named, not
            // just a count.
            if let Some(plan) = merge_plans.get(&title.id)
                && let Some(reason) = plan.summary.blocked_reason()
            {
                fact = fact.with_unresolved_reason(
                    crate::location::classify::reason_codes::MERGE_RECORDS_UNMAPPED,
                    format!(
                        "\"{}\" cannot merge into {}: {reason}",
                        title.name, plan.destination_title_id
                    ),
                );
            }
            facts.push(fact);
            media_files_by_title.insert(title.id.clone(), media_files);
        }

        let classification =
            classify_selection(&facts, &request.destination, Some(&destination_facts));
        // The counts the plan reports. Draft building below can downgrade a
        // classified title to needs-resolution (a vanished source folder, a
        // destination root with no path), and the counts must follow the drafts
        // or the preview would claim a moving title it produced no work for.
        let mut classification_counts = classification.counts;
        let downgrade_to_needs_resolution =
            |counts: &mut crate::location::classify::ClassificationCounts,
             was: TitleLocationClass| {
                match was {
                    TitleLocationClass::RootMove => counts.root_move -= 1,
                    TitleLocationClass::CrossLibraryTransfer => counts.cross_library_transfer -= 1,
                    _ => {}
                }
                counts.needs_resolution += 1;
            };

        // The destination titles this selection merges into, read once. Their
        // folders are where the merging titles' files land (FR-063: the
        // destination's naming wins, and it already owns a folder).
        let merge_destination_titles = self
            .merge_destination_titles(&destination_identities)
            .await?;

        // Folder-naming policy for the destination library's facet (FR-013).
        let folder_template = self
            .title_folder_template_for(&destination_library.facet)
            .await?;
        let depth = LOCATION_OPERATION_VERIFICATION_DEPTH;
        let probe = SystemVolumeProbe;

        let mut drafts = Vec::with_capacity(titles.len());
        let mut moved_bytes = 0_u64;
        let mut a_source_path: Option<PathBuf> = None;
        let mut a_destination_path: Option<PathBuf> = None;
        let mut source_root_for_recycle: Option<String> = None;

        for title in &titles {
            let classified = classification
                .classification_of(&title.id)
                .expect("every selected title is classified");
            let destination_root_path = destination_library
                .roots
                .iter()
                .find(|root| root.id == classified.destination_root_id)
                .map(|root| PathBuf::from(root.path.clone()));
            let source_root_path = libraries
                .get(&title.library_id)
                .and_then(|library| {
                    library
                        .roots
                        .iter()
                        .find(|root| root.id == title.root_folder_id)
                })
                .map(|root| PathBuf::from(root.path.clone()));
            let source_folder_path = title
                .folder_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(stored_path_to_path_buf);

            let mut draft = RootMoveTitleDraft {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                class: classified.class,
                source_library_id: title.library_id.clone(),
                source_root_id: title.root_folder_id.clone(),
                source_root_path: source_root_path.clone(),
                source_folder_path: source_folder_path.clone(),
                destination_library_id: classified.destination_library_id.clone(),
                destination_root_id: classified.destination_root_id.clone(),
                destination_root_path: destination_root_path.clone(),
                destination_folder_path: None,
                files: Vec::new(),
                source_directories: Vec::new(),
                same_volume: None,
                hardlinks: Vec::new(),
                destination_entries: Vec::new(),
                recycle: RecycleAvailability::Available,
                blocked_reason: classified.reason.clone(),
                destination_identity: classified.destination_identity.clone(),
                facet_conversion: classified.facet_conversion.clone(),
                associations: classified.associations,
                merge_summary: merge_plans
                    .get(&title.id)
                    .map(|plan| plan.summary.clone()),
                adoption: None,
            };

            if !classified.class.moves_files() {
                drafts.push(draft);
                continue;
            }

            let Some(destination_root_path) = destination_root_path else {
                downgrade_to_needs_resolution(&mut classification_counts, draft.class);
                draft.class = TitleLocationClass::NeedsResolution;
                draft.blocked_reason = Some(format!(
                    "destination root {} has no configured path",
                    classified.destination_root_id
                ));
                drafts.push(draft);
                continue;
            };

            // FR-013 for a transfer, FR-063 for a merge.
            //
            // A merge has no destination folder of its own to calculate: the
            // destination title already owns one, and the merged content is
            // that title's content from the moment Groups 1–5 commit. Sending
            // the files anywhere else would leave the surviving title owning a
            // folder that holds half of its media. Routing them into the
            // destination's folder is also what makes FR-073 work here for
            // free: the collision engine reads that folder's entries, and two
            // copies of the same episode dedup against each other instead of
            // landing side by side.
            let merge_destination = classified
                .merge_target_title_id()
                .and_then(|id| merge_destination_titles.get(id));

            // A merge also lands on the *destination title's* root, not on the
            // one the request named, when the two differ. FR-063 gives the
            // destination its placement along with everything else, and a
            // checkpoint recording a root that does not contain the folder
            // would be an audit trail nobody can follow.
            let (destination_root_id, destination_root_path) = match merge_destination {
                Some(destination_title)
                    if destination_title.root_folder_id != classified.destination_root_id =>
                {
                    match destination_library
                        .roots
                        .iter()
                        .find(|root| root.id == destination_title.root_folder_id)
                    {
                        Some(root) => (
                            destination_title.root_folder_id.clone(),
                            PathBuf::from(root.path.clone()),
                        ),
                        None => (
                            classified.destination_root_id.clone(),
                            destination_root_path,
                        ),
                    }
                }
                _ => (
                    classified.destination_root_id.clone(),
                    destination_root_path,
                ),
            };
            draft.destination_root_id = destination_root_id;
            draft.destination_root_path = Some(destination_root_path.clone());

            let destination_folder = match merge_destination {
                Some(destination_title) => merge_destination_folder(
                    destination_title,
                    &destination_root_path,
                    &folder_template,
                ),
                None => crate::library::rename::configured_title_folder_path(
                    &destination_root_path.to_string_lossy(),
                    title,
                    &folder_template,
                    title.year,
                ),
            };
            // US3: nothing is planned to move, because it already did. The
            // destination folder is resolved against what is actually on disk,
            // the tracked media is accounted for against it (FR-050/051), and
            // none of the collision, hardlink, or free-space machinery applies —
            // no bytes are written.
            if mode == LocationExecutionMode::FilesAlreadyThere {
                let adoption_folder = resolve_adoption_folder(
                    destination_folder.clone(),
                    source_folder_path.as_deref(),
                    &destination_root_path,
                )
                .await;
                draft.destination_folder_path = Some(adoption_folder.clone());
                draft.adoption = account_for_adoption(
                    &adoption_folder,
                    source_folder_path.as_deref(),
                    media_files_by_title
                        .get(&title.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                )
                .await;
                if a_source_path.is_none() {
                    a_source_path = source_folder_path.clone().or(source_root_path.clone());
                }
                if a_destination_path.is_none() {
                    a_destination_path = Some(destination_root_path.clone());
                }
                drafts.push(draft);
                continue;
            }

            draft.destination_folder_path = Some(destination_folder.clone());

            let media_files = media_files_by_title
                .get(&title.id)
                .cloned()
                .unwrap_or_default();
            // A source folder that cannot be walked - vanished, unreadable - is
            // that title's problem, not the preview's: erroring here would hide
            // the whole plan behind an opaque failure, and the vanished-folder
            // case is exactly the user who should be offered "Files are already
            // there" (US3). Degrade to a blocked title the preview can name.
            let (files, directories) =
                match collect_source_files(source_folder_path.as_deref(), &media_files).await {
                    Ok(walked) => walked,
                    Err(error) => {
                        downgrade_to_needs_resolution(&mut classification_counts, draft.class);
                        draft.class = TitleLocationClass::NeedsResolution;
                        draft.blocked_reason = Some(format!(
                            "the source folder for \"{}\" could not be read ({}); if its files \
                             were moved by hand, use \"Files are already there\"",
                            title.name, error
                        ));
                        drafts.push(draft);
                        continue;
                    }
                };
            draft.files = files;
            draft.source_directories = directories;

            draft.same_volume = match source_folder_path.as_deref() {
                Some(folder) => Some(same_filesystem(folder, &destination_folder).await),
                None => None,
            };
            draft.hardlinks =
                detect_hardlinks(draft.files.iter().map(|file| file.path.clone()).collect())
                    .await?;
            // FR-073 needs a *proven* hash on both sides, and the only place a
            // destination file's hash exists without reading it again is the
            // catalog. For a merge the destination folder's contents are the
            // destination title's tracked media, so the persisted hashes are
            // right there; without them every identical episode would be
            // renamed beside its twin rather than deduplicated (D4).
            let destination_hashes = match merge_destination {
                Some(destination_title) => {
                    self.persisted_hashes_for_title(&destination_title.id).await?
                }
                None => BTreeMap::new(),
            };
            draft.destination_entries =
                read_destination_entries(&destination_folder, &destination_hashes).await;
            draft.recycle = match source_root_path.as_deref() {
                Some(root) => {
                    let config = self
                        .recycle_bin_config_for_media_root(Some(&root.to_string_lossy()))
                        .await;
                    if !config.enabled {
                        RecycleAvailability::Disabled
                    } else if let Some(error) = config.validation_error.clone() {
                        RecycleAvailability::Unavailable(error)
                    } else {
                        RecycleAvailability::Available
                    }
                }
                None => RecycleAvailability::Unavailable(
                    "the source root has no configured path".to_string(),
                ),
            };

            if draft.same_volume != Some(true) {
                moved_bytes = moved_bytes
                    .saturating_add(draft.files.iter().map(|file| file.size_bytes).sum::<u64>());
            }
            if a_source_path.is_none() {
                a_source_path = source_folder_path.clone().or(source_root_path.clone());
            }
            if a_destination_path.is_none() {
                a_destination_path = Some(destination_root_path.clone());
            }
            if source_root_for_recycle.is_none() {
                source_root_for_recycle =
                    source_root_path.as_deref().map(|path| path.to_string_lossy().to_string());
            }

            drafts.push(draft);
        }

        // FR-080: free space, including the recycle-copy cost when the bin is on
        // another volume.
        let free_space = match (a_source_path.clone(), a_destination_path.clone()) {
            (Some(source), Some(destination)) => {
                let recycle_config = self
                    .recycle_bin_config_for_media_root(source_root_for_recycle.as_deref())
                    .await;
                estimate_free_space(
                    &FreeSpaceRequest {
                        source_path: source,
                        destination_path: destination,
                        moved_bytes,
                        recycled_bytes: moved_bytes,
                        recycle_base_path: recycle_config
                            .enabled
                            .then(|| recycle_config.base_path.clone()),
                    },
                    &probe,
                )
            }
            _ => FreeSpaceEstimate::unknown(),
        };

        let case_rule = PathCaseRule::platform_default();
        let source_library_id = {
            let mut ids: Vec<&str> = titles.iter().map(|title| title.library_id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            match ids.as_slice() {
                [only] => Some((*only).to_string()),
                _ => None,
            }
        };
        let source_root_id = {
            let mut ids: Vec<&str> = titles
                .iter()
                .map(|title| title.root_folder_id.as_str())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            match ids.as_slice() {
                [only] => Some((*only).to_string()),
                _ => None,
            }
        };

        let planned = build_root_move_plan(&RootMovePlanRequest {
            source_library_id,
            destination_library_id: Some(destination_library.id.clone()),
            source_root_id,
            destination_root_id: request.destination.root_id.clone(),
            selection,
            titles: drafts,
            mode,
            classification: classification_counts,
            verification_depth: depth,
            free_space,
            case_rule,
            naming: CollisionNaming::from_source_library(source_library_label(&libraries)),
        });

        Ok(RootMovePlanning {
            planned,
            classification,
        })
    }

    /// Destination-title detection for the selected titles that are crossing
    /// into `destination_library_id` (FR-055).
    ///
    /// # One read, not one per title
    ///
    /// The destination library's titles are read **once** and answered against
    /// for every crossing title, which is what
    /// [`detect_destination_titles`] is shaped for. Asking
    /// `find_by_external_id_in_library_and_facet` per identity would be one
    /// query per `(title, identity)` pair and would still not see the
    /// same-name-without-identity case, which needs the destination's title
    /// text. A selection with no crossing title pays nothing at all: the usual
    /// US2 root move never reaches the read.
    ///
    /// The candidate read is deliberately unfiltered by facet. The
    /// series-anime crossover is a legitimate transfer (FR-057) and a movie-kind
    /// title may live in an episodic library (FR-061), so filtering the
    /// candidates by the source facet would hide exactly the matches that
    /// matter. `detect_destination_titles` applies the compatibility rule
    /// itself, on facts it can see.
    ///
    /// # Redirects
    ///
    /// No redirect edges are supplied. Scryer keeps no redirect ledger: SMG
    /// publishes `from_id → to_id` pairs on a metadata fetch and hydration
    /// rewrites the stored id in place, so the edges exist only for as long as
    /// the fetch that carried them. A preview must not reach the metadata
    /// gateway — it would make an interactive, cached, re-derived-on-confirm
    /// operation depend on a network round-trip — so detection compares the
    /// stored ids on both sides. The consequence is bounded and one-directional:
    /// when a redirect was published and only one side has been hydrated since,
    /// a merge that *could* have been offered is not, and the title transfers
    /// instead. That is a missed merge, never a wrong one.
    async fn detect_destination_titles_for(
        &self,
        titles: &[Title],
        destination_library_id: &str,
    ) -> AppResult<BTreeMap<String, DestinationIdentityOutcome>> {
        let sources: Vec<SourceTitleIdentity> = titles
            .iter()
            .filter(|title| title.library_id != destination_library_id)
            .map(|title| {
                SourceTitleIdentity::new(title.id.clone(), title.facet.clone())
                    .with_name(title.name.clone())
                    .with_external_ids(&title.external_ids)
            })
            .collect();
        if sources.is_empty() {
            return Ok(BTreeMap::new());
        }

        let candidates: Vec<DestinationTitleCandidate> = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, &[destination_library_id.to_string()], None)
            .await?
            .into_iter()
            .map(|title| {
                DestinationTitleCandidate::new(title.id, title.facet)
                    .with_name(title.name)
                    .with_external_ids(&title.external_ids)
            })
            .collect();

        Ok(detect_destination_titles(
            &sources,
            &candidates,
            &IdentityRedirects::new(),
        ))
    }

    /// Plan every merge this selection would perform, at preview time
    /// (C2, FR-066, FR-071).
    ///
    /// # Why this is a preview-time read and not an execution-time one
    ///
    /// [`plan_merge`] is a pure function over a Group 0 snapshot, and it is the
    /// *only* thing that decides whether a merge can run at all. Deferring it
    /// to the title checkpoint would mean the user confirms a plan whose
    /// central question — "can these two titles actually be folded together?" —
    /// has not been asked yet, and an FR-066 refusal would arrive as a failed
    /// title halfway through a move instead of as a blocked row in the preview.
    ///
    /// `current_operation_id` is deliberately `None` here: no operation exists
    /// during a preview, and a *second* operation holding the source title is
    /// exactly the OQ7 hazard the snapshot is asked to report.
    async fn plan_destination_merges(
        &self,
        titles: &[Title],
        identities: &BTreeMap<String, DestinationIdentityOutcome>,
    ) -> AppResult<BTreeMap<String, MergePlan>> {
        let mut plans = BTreeMap::new();
        for title in titles {
            let Some(destination_title_id) = identities
                .get(&title.id)
                .and_then(DestinationIdentityOutcome::merge_target)
            else {
                continue;
            };
            let snapshot = self
                .services
                .library
                .title_merges
                .load_merge_snapshot(&title.id, destination_title_id, None)
                .await?;
            plans.insert(title.id.clone(), plan_merge(&snapshot));
        }
        Ok(plans)
    }

    /// The destination titles a selection merges into, keyed by their id.
    async fn merge_destination_titles(
        &self,
        identities: &BTreeMap<String, DestinationIdentityOutcome>,
    ) -> AppResult<BTreeMap<String, Title>> {
        let mut destinations: BTreeMap<String, Title> = BTreeMap::new();
        for outcome in identities.values() {
            let Some(destination_title_id) = outcome.merge_target() else {
                continue;
            };
            if destinations.contains_key(destination_title_id) {
                continue;
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(destination_title_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("destination title {destination_title_id}"))
                })?;
            destinations.insert(destination_title_id.to_string(), title);
        }
        Ok(destinations)
    }

    /// Persisted full-BLAKE3 state for one title's tracked media, keyed by
    /// stored path, so the collision planner can prove a duplicate without
    /// reading a byte (D4, FR-047).
    async fn persisted_hashes_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<BTreeMap<String, FullHash>> {
        Ok(self
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await?
            .into_iter()
            .map(|file| {
                let hash = FullHash::from_persisted(file.content_hashes.as_ref());
                (file.file_path, hash)
            })
            .collect())
    }

    /// Series-movie links, collections, and episodes for the titles that cross
    /// a library boundary (FR-060–FR-062).
    ///
    /// # Why these three and nothing else
    ///
    /// The schema decides the scope. `series_movie_links.series_title_id`,
    /// `collections.title_id`, and `episodes.title_id`/`collection_id` are keyed
    /// on ids the transfer does not reissue, and none of the three tables has a
    /// library or root column — so a transfer preserves all of it with no
    /// rewrite at all. What the preview needs is therefore a *count*, to say how
    /// much rides along, not a mapping.
    ///
    /// The movie entity at the far end of a link is shared metadata
    /// (`movie_entities` has no owning title and no library), so a linked movie
    /// cannot be orphaned by moving the series that references it — which is why
    /// FR-060's disposition is "move together" rather than a user choice.
    ///
    /// Reads are scoped to crossing episodic titles, and the episode count only
    /// to titles whose facet actually converts, because that is the only case
    /// where the collection statement is emitted.
    async fn transfer_association_facts(
        &self,
        titles: &[Title],
        destination_library: &scryer_domain::Library,
    ) -> AppResult<BTreeMap<String, TitleAssociationFacts>> {
        let crossing: Vec<String> = titles
            .iter()
            .filter(|title| {
                title.library_id != destination_library.id && title.facet != MediaFacet::Movie
            })
            .map(|title| title.id.clone())
            .collect();
        if crossing.is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut facts: BTreeMap<String, TitleAssociationFacts> = crossing
            .iter()
            .map(|title_id| (title_id.clone(), TitleAssociationFacts::default()))
            .collect();

        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_titles(&crossing)
            .await?;
        for (title_id, rows) in collections {
            if let Some(entry) = facts.get_mut(&title_id) {
                entry.collections = rows.len() as i64;
            }
        }

        for link in self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_titles(&crossing)
            .await?
        {
            if let Some(entry) = facts.get_mut(&link.series_title_id) {
                entry.series_movie_links += 1;
            }
        }

        for title in titles
            .iter()
            .filter(|title| converts_facet(&title.facet, &destination_library.facet))
        {
            if let Some(entry) = facts.get_mut(&title.id) {
                entry.episodes = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await?
                    .len() as i64;
            }
        }

        Ok(facts)
    }

    /// The destination library's active folder-naming policy (FR-013).
    async fn title_folder_template_for(&self, facet: &MediaFacet) -> AppResult<String> {
        let raw = self
            .read_setting_string_value(crate::FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
            .await?;
        let default_template = match facet {
            MediaFacet::Movie => crate::DEFAULT_FOLDER_TEMPLATE_MOVIE,
            MediaFacet::Series => crate::DEFAULT_FOLDER_TEMPLATE_SERIES,
            MediaFacet::Anime => crate::DEFAULT_FOLDER_TEMPLATE_ANIME,
        };
        Ok(crate::normalize_title_folder_template_or_default(
            raw,
            default_template,
            facet.as_str(),
        ))
    }

    /// FR-086: an active download or import on the title blocks it from
    /// entering a move.
    ///
    /// The durable tracked-state ledger is what is consulted, not the download
    /// client: a preview must not depend on a network round-trip, and a
    /// submission Scryer has accepted but not yet bound to a client item is
    /// just as much an in-flight claim as one that is downloading.
    pub(super) async fn active_work_blocking_a_move(&self, title: &Title) -> AppResult<Option<String>> {
        let unbound = self
            .services
            .workflow
            .download_submissions
            .list_active_unbound_for_title(&title.id)
            .await?;
        if !unbound.is_empty() {
            return Ok(Some(format!(
                "\"{}\" has {} grab(s) waiting on the download client; the move can start once they finish",
                title.name,
                unbound.len()
            )));
        }

        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if submissions.is_empty() {
            return Ok(None);
        }

        let locators: Vec<crate::contracts::ClientJobLocator> = submissions
            .iter()
            .map(crate::contracts::ClientJobLocator::from_submission)
            .collect();
        let states = self
            .services
            .workflow
            .download_submissions
            .list_identity_tracked_states_for_client_items(&locators)
            .await?;

        let live = states
            .iter()
            .filter_map(|(_, state)| scryer_domain::TrackedDownloadState::from_str_opt(state))
            .filter(|state| {
                // The snapshot is deliberately not consulted (see the doc
                // comment): a submission with no ledger entry is not treated as
                // live, and the executor's admission check plus the ownership
                // guard catch anything that starts afterwards.
                crate::acquisition_workflow::submission_is_queued(Some(*state), false)
            })
            .count();

        if live > 0 {
            return Ok(Some(format!(
                "\"{}\" has {live} download(s) or import(s) in flight; the move can start once they finish",
                title.name
            )));
        }
        Ok(None)
    }
}

/// A label for collision renames: the single source library's name when the
/// selection has one, else a neutral fallback (FR-074).
fn source_library_label(libraries: &BTreeMap<String, scryer_domain::Library>) -> String {
    let mut names: Vec<&str> = libraries.values().map(|library| library.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    match names.as_slice() {
        [only] => (*only).to_string(),
        _ => "another library".to_string(),
    }
}

/// Everything beneath the title's folder, plus the tracked media files that
/// live outside it.
///
/// The walk is what makes companion assets travel with their title (FR-027):
/// the media-file table only knows about tracked media, and a title folder
/// holds subtitles, artwork, NFOs, and trickplay directories that belong to it
/// just as much.
pub(super) async fn collect_source_files(
    source_folder: Option<&Path>,
    media_files: &[crate::TitleMediaFile],
) -> AppResult<(Vec<SourceFile>, Vec<PathBuf>)> {
    let media_by_path: BTreeMap<&str, &crate::TitleMediaFile> = media_files
        .iter()
        .map(|file| (file.file_path.as_str(), file))
        .collect();

    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    if let Some(folder) = source_folder {
        let folder = folder.to_path_buf();
        let walked = tokio::task::spawn_blocking({
            let folder = folder.clone();
            move || {
                crate::library::filesystem_walk::FilesystemWalker::new()
                    .skip_unreadable_subdirectories()
                    .skip_symlinked_directories()
                    .confine_to_root()
                    .walk(&folder)
            }
        })
        .await
        .map_err(|error| AppError::Repository(format!("source walk task panicked: {error}")))??;

        for listing in walked {
            if listing.path != folder {
                directories.push(listing.path.clone());
            }
            for path in listing.files {
                let stored = path_to_stored_string(&path);
                let size_bytes = tokio::fs::symlink_metadata(&path)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let relative = path
                    .strip_prefix(&folder)
                    .ok()
                    .map(std::path::Path::to_path_buf);
                let tracked = media_by_path.get(stored.as_str()).copied();
                seen.insert(stored.clone());
                files.push(SourceFile {
                    media_file_id: tracked.map(|file| file.id.clone()),
                    // The persisted hash, never a fresh read: planning does no
                    // hashing, and an absent hash means "unproven", which the
                    // dedup gate treats as *not* a duplicate (D4).
                    full_blake3: tracked
                        .map(|file| FullHash::from_persisted(file.content_hashes.as_ref()))
                        .unwrap_or(FullHash::Absent),
                    path,
                    relative_path: relative,
                    size_bytes,
                });
            }
        }
    }

    // Tracked media the walk did not see: outside the folder, or the folder is
    // unknown. Leaving it behind would strand a catalog row on the old root.
    for media_file in media_files {
        if seen.contains(&media_file.file_path) {
            continue;
        }
        let path = stored_path_to_path_buf(&media_file.file_path);
        let size_bytes = tokio::fs::symlink_metadata(&path)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or_else(|_| media_file.size_bytes.max(0) as u64);
        files.push(SourceFile {
            media_file_id: Some(media_file.id.clone()),
            full_blake3: FullHash::from_persisted(media_file.content_hashes.as_ref()),
            path,
            relative_path: None,
            size_bytes,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    directories.sort();
    directories.dedup();
    Ok((files, directories))
}

// ── Adoption fact gathering (US3) ────────────────────────────────────────────

/// Which folder an adoption accounts against.
///
/// The pure rule lives in [`crate::location::adoption::choose_adoption_folder`];
/// this is the two `stat`s it needs.
async fn resolve_adoption_folder(
    calculated: PathBuf,
    source_folder: Option<&Path>,
    destination_root: &Path,
) -> PathBuf {
    let calculated_exists = is_existing_directory(&calculated).await;
    let source_named = source_folder
        .and_then(|folder| folder.file_name())
        .map(|name| destination_root.join(name));
    let source_named_exists = match source_named.as_deref() {
        Some(path) => is_existing_directory(path).await,
        None => false,
    };
    choose_adoption_folder(
        calculated,
        calculated_exists,
        source_named,
        source_named_exists,
    )
}

async fn is_existing_directory(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
}

/// Account for one title's tracked media against what the destination folder
/// actually holds (FR-050, FR-051).
///
/// `None` means the folder could not be scanned at all, which the planner
/// reports as a blocked title rather than as "nothing is there".
///
/// # How much proof this reads, and why not more
///
/// FR-050 asks for size, stored identity, the sampled proof, and the persisted
/// full BLAKE3 "where already persisted". A preview is an interactive screen, so
/// the reads are bounded to the ones that can actually decide something:
///
/// - **Size and the stored file signature**: one `stat` per destination file. An
///   external `mv` preserves the mtime, so the signature the catalog already
///   holds is real evidence that costs nothing.
/// - **The sampled head+tail proof**: read for a destination file only when some
///   tracked file of the same size still has a readable source to compare
///   against. With the source gone — the ordinary US3 case — there is nothing to
///   compare a sample to, so reading one would be pure cost.
/// - **The full BLAKE3**: read only when several destination files share a size
///   and a tracked file of that size carries a current persisted hash. That is
///   the one case where hashing turns an ambiguity into an answer; hashing a
///   library to render a preview is not something a preview may do.
///
/// Everything not read stays absent, and absent evidence never resolves
/// anything on its own — the matcher's floor is size plus structural identity,
/// and it refuses rather than guesses (FR-052).
async fn account_for_adoption(
    destination_folder: &Path,
    source_folder: Option<&Path>,
    media_files: &[crate::TitleMediaFile],
) -> Option<crate::location::adoption::TitleAdoptionAccounting> {
    let destination_files = walk_destination_files(destination_folder).await?;

    let mut tracked: Vec<TrackedMediaFact> = Vec::with_capacity(media_files.len());
    for media_file in media_files {
        let path = stored_path_to_path_buf(&media_file.file_path);
        let mut fact = TrackedMediaFact::new(
            media_file.id.clone(),
            media_file.file_path.clone(),
            media_file.size_bytes.max(0) as u64,
        )
        .with_full_blake3(FullHash::from_persisted(media_file.content_hashes.as_ref()))
        .with_signature(stored_file_signature(media_file));
        if let Some(relative) = source_folder
            .and_then(|folder| path.strip_prefix(folder).ok())
            .map(|relative| relative.to_string_lossy().to_string())
        {
            fact = fact.with_relative_path(relative);
        }
        // The source is usually gone — that is the story — so a proof is read
        // only when there is something to read (FR-053).
        if tokio::fs::symlink_metadata(&path).await.is_ok() {
            fact = fact.with_sampled_proof(sampled_proof_of(&path).await);
        }
        tracked.push(fact);
    }

    let mut sizes_with_source_proof: BTreeMap<u64, bool> = BTreeMap::new();
    let mut sizes_with_persisted_hash: BTreeMap<u64, bool> = BTreeMap::new();
    for fact in &tracked {
        if fact.sampled_proof.is_some() {
            sizes_with_source_proof.insert(fact.size_bytes, true);
        }
        if fact.full_blake3.as_known().is_some() {
            sizes_with_persisted_hash.insert(fact.size_bytes, true);
        }
    }
    let mut destination_size_counts: BTreeMap<u64, usize> = BTreeMap::new();
    for file in &destination_files {
        *destination_size_counts.entry(file.size_bytes).or_insert(0) += 1;
    }

    let mut destination = Vec::with_capacity(destination_files.len());
    for file in destination_files {
        let mut fact = DestinationFileFact::new(
            path_to_stored_string(&file.path),
            file.size_bytes,
        )
        .with_signature(file.signature.clone());
        if let Some(relative) = file.relative_path.clone() {
            fact = fact.with_relative_path(relative);
        }
        if sizes_with_source_proof.contains_key(&file.size_bytes) {
            fact = fact.with_sampled_proof(sampled_proof_of(&file.path).await);
        } else if sizes_with_persisted_hash.contains_key(&file.size_bytes)
            && destination_size_counts
                .get(&file.size_bytes)
                .copied()
                .unwrap_or(0)
                > 1
            && let Ok(hashes) =
                crate::location::verify::hash_existing_file(&file.path).await
            {
                fact = fact.with_full_blake3(FullHash::known(hashes.full_blake3));
            }
        destination.push(fact);
    }

    Some(match_title_adoption(&tracked, &destination))
}

/// One file the destination walk found, before it becomes a matcher fact.
struct DestinationScanFile {
    path: PathBuf,
    relative_path: Option<String>,
    size_bytes: u64,
    signature: Option<crate::file_source_signature::FileSourceSignature>,
}

/// Everything beneath the destination folder, at every depth.
///
/// The recursion matters: a title moved by hand keeps its season folders, and a
/// listing one level deep would report every episode missing.
async fn walk_destination_files(folder: &Path) -> Option<Vec<DestinationScanFile>> {
    let root = folder.to_path_buf();
    let walked = tokio::task::spawn_blocking({
        let root = root.clone();
        move || {
            crate::library::filesystem_walk::FilesystemWalker::new()
                .skip_unreadable_subdirectories()
                .skip_symlinked_directories()
                .confine_to_root()
                .walk(&root)
        }
    })
    .await
    .ok()?
    .ok()?;

    let mut files = Vec::new();
    for listing in walked {
        for path in listing.files {
            let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .ok()
                .map(|relative| relative.to_string_lossy().to_string());
            files.push(DestinationScanFile {
                relative_path: relative,
                size_bytes: metadata.len(),
                signature: crate::file_source_signature::file_source_signature_from_metadata(
                    &metadata,
                )
                .ok(),
                path,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Some(files)
}

/// The signature the catalog stored for a media file, when it stored one this
/// build understands. A half-written pair proves nothing and reads back absent.
fn stored_file_signature(
    media_file: &crate::TitleMediaFile,
) -> Option<crate::file_source_signature::FileSourceSignature> {
    match (
        media_file.source_signature_scheme.as_deref(),
        media_file.source_signature_value.as_deref(),
    ) {
        (Some(scheme), Some(value))
            if scheme == crate::file_source_signature::MEDIA_FILE_SOURCE_SIGNATURE_SCHEME =>
        {
            Some(crate::file_source_signature::FileSourceSignature {
                scheme: scheme.to_string(),
                value: value.to_string(),
            })
        }
        _ => None,
    }
}

async fn sampled_proof_of(path: &Path) -> Option<scryer_domain::ImportContentProof> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || crate::fs_integrity::import_content_proof(&path))
        .await
        .ok()?
        .ok()
}

/// What already sits in the destination folder, for the collision planner
/// (FR-072–075). An absent folder simply has nothing in it.
///
/// `known_hashes` maps a stored destination path to the full BLAKE3 the catalog
/// has for it. Anything not in the map keeps [`FullHash::Absent`], which the
/// dedup gate reads as "unproven" and therefore as *not* a duplicate (D4) —
/// the same conservative answer this function gave before hashes were passed at
/// all.
async fn read_destination_entries(
    destination_folder: &Path,
    known_hashes: &BTreeMap<String, FullHash>,
) -> Vec<DestinationItem> {
    let Ok(mut entries) = tokio::fs::read_dir(destination_folder).await else {
        return Vec::new();
    };
    let mut items = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if metadata.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = path_to_stored_string(entry.path());
        let full_hash = known_hashes.get(&path).cloned().unwrap_or(FullHash::Absent);
        items.push(
            DestinationItem::companion(name, metadata.len())
                .with_content(ContentFacts::new(metadata.len()).with_full_hash(full_hash))
                .with_path(path),
        );
    }
    items
}

/// The folder a merging title's content moves into (FR-063).
///
/// The destination title's own folder when it owns one — which is the whole
/// point: its content is already there and stays there. When it owns none (a
/// fileless destination title, which is a perfectly ordinary way to have a
/// wanted-list entry), the destination library's naming policy calculates one
/// from the *destination* title, so the surviving title's folder is named after
/// the surviving title.
fn merge_destination_folder(
    destination_title: &Title,
    destination_root_path: &Path,
    folder_template: &str,
) -> PathBuf {
    destination_title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(stored_path_to_path_buf)
        .unwrap_or_else(|| {
            crate::library::rename::configured_title_folder_path(
                &destination_root_path.to_string_lossy(),
                destination_title,
                folder_template,
                destination_title.year,
            )
        })
}

/// A refused confirmation keeps its prose *and* its code: the client re-previews
/// on a stale plan and unblocks a selection on a blocked one, and it should not
/// have to read a sentence to tell the two apart (FR-016, FR-081).
pub(super) fn confirmation_error(error: PlanConfirmationError) -> AppError {
    let message = match error {
        PlanConfirmationError::Stale => {
            "the preview no longer matches what is on disk or in the catalog; review a fresh preview before confirming"
        }
        PlanConfirmationError::Blocked => {
            "some selected titles still need a decision; resolve or remove them before starting"
        }
        PlanConfirmationError::InsufficientSpace => {
            "the destination does not have enough free space for this move; free space or move fewer titles"
        }
        PlanConfirmationError::TypedConfirmationRequired => {
            "this operation requires typed confirmation"
        }
        PlanConfirmationError::TypedConfirmationMismatch => "the typed confirmation did not match",
    };
    AppError::LocationPlanRefused {
        message: message.to_string(),
        code: error,
    }
}

// ── Catalog adapter ──────────────────────────────────────────────────────────

/// The production [`RootMoveCatalog`]: the real repositories behind the seam the
/// executor path is written against.
struct AppUseCaseRootMoveCatalog {
    app: AppUseCase,
}

#[async_trait::async_trait]
impl RootMoveCatalog for AppUseCaseRootMoveCatalog {
    async fn title_placement(&self, title_id: &str) -> AppResult<Option<TitlePlacementSnapshot>> {
        let Some(title) = self.app.services.catalog.titles.get_by_id(title_id).await? else {
            return Ok(None);
        };
        let media_files = self
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await?;
        Ok(Some(TitlePlacementSnapshot {
            root_folder_id: title.root_folder_id.clone(),
            library_id: title.library_id.clone(),
            folder_path: title
                .folder_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
            media_file_paths: media_files
                .into_iter()
                .map(|file| file.file_path)
                .collect(),
        }))
    }

    async fn set_media_file_path(&self, media_file_id: &str, stored_path: &str) -> AppResult<()> {
        self.app
            .services
            .library
            .media_files
            .update_media_file_path(media_file_id, stored_path)
            .await
    }

    async fn delete_media_file(&self, media_file_id: &str) -> AppResult<()> {
        match self
            .app
            .services
            .library
            .media_files
            .delete_media_file(media_file_id)
            .await
        {
            Ok(()) => Ok(()),
            // Cleanup is re-entered on resume, so a row that has already gone
            // is a completed removal rather than a failure.
            Err(AppError::NotFound(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn set_media_file_content_hashes(
        &self,
        media_file_id: &str,
        hashes: &crate::location::model::PersistedContentHashes,
    ) -> AppResult<()> {
        self.app
            .services
            .library
            .media_files
            .update_media_file_content_hashes(media_file_id, hashes)
            .await
    }

    async fn set_title_folder_path(&self, title_id: &str, stored_path: &str) -> AppResult<()> {
        self.app
            .services
            .catalog
            .titles
            .set_folder_path(title_id, stored_path)
            .await
    }

    async fn set_title_root(&self, title_id: &str, root_folder_id: &str) -> AppResult<()> {
        self.app
            .services
            .catalog
            .titles
            .update_metadata(title_id, None, None, None, Some(root_folder_id.to_string()))
            .await
            .map(|_| ())
    }

    async fn set_title_library_and_root(
        &self,
        title_id: &str,
        library_id: &str,
        root_folder_id: &str,
        converted_facet: Option<&MediaFacet>,
        drop_tag_prefixes: &[String],
    ) -> AppResult<()> {
        self.app
            .services
            .catalog
            .titles
            .transfer_to_library(
                title_id,
                library_id,
                root_folder_id,
                converted_facet.cloned(),
                drop_tag_prefixes,
            )
            .await
    }
}

// ── Group 6 ──────────────────────────────────────────────────────────────────

/// The production [`PostMergeWorkScheduler`]: the merge engine's Group 6 work
/// list, mapped onto the subsystems that already own each cache.
///
/// | Work | Where it goes |
/// |---|---|
/// | `ReindexTitleSearchTerms` | The destination title is re-persisted through `TitleRepository::update_metadata`, which is the write path that rebuilds `title_search_terms`. The merge writes `titles.tags` with raw SQL inside its transaction, so the projection is genuinely stale until this runs. |
/// | `RegenerateRecommendations` | `queue_title_more_like_this_refresh_if_due`, the same queue hydration uses. |
/// | `RecomputeStatistics` | Nothing to invalidate: Scryer keeps no persisted title or library statistics cache — Activity and the dashboard derive their counts on read — so this is satisfied by construction, and it is logged rather than silently skipped. |
/// | `DropSourceIndexerCoverage` | `prune_scope_key_coverage` over every reversible scope key the merge retired: the source title, its episodes, its collections, and its series-movie links. The irreversible `episode_set:b3:` and `series_pack_set:b3:` forms cannot be reconstructed and are left to expire, which is exactly why OQ4 drops coverage uniformly rather than remapping it. |
///
/// Every step is best effort. A failure here leaves a correct catalog with a
/// stale derived cache, which the next natural refresh repairs; failing the
/// title over one would undo a committed merge's checkpoint for no gain.
struct AppUseCasePostMergeWork {
    app: AppUseCase,
}

#[async_trait::async_trait]
impl PostMergeWorkScheduler for AppUseCasePostMergeWork {
    async fn schedule_post_merge_work(&self, request: PostMergeWorkRequest) -> AppResult<()> {
        for work in &request.work {
            match work {
                PostMergeWork::ReindexTitleSearchTerms => {
                    self.reindex_search_terms(&request.destination_title_id)
                        .await;
                }
                PostMergeWork::RegenerateRecommendations => {
                    self.regenerate_recommendations(&request.destination_title_id)
                        .await;
                }
                PostMergeWork::RecomputeStatistics => {
                    tracing::debug!(
                        operation_id = %request.operation_id,
                        destination_title_id = %request.destination_title_id,
                        "merge statistics need no recomputation: Scryer derives title and library counts on read"
                    );
                }
                PostMergeWork::DropSourceIndexerCoverage => {
                    self.drop_source_coverage(&request).await;
                }
            }
        }
        Ok(())
    }
}

impl AppUseCasePostMergeWork {
    async fn reindex_search_terms(&self, destination_title_id: &str) {
        let title = match self
            .app
            .services
            .catalog
            .titles
            .get_by_id(destination_title_id)
            .await
        {
            Ok(Some(title)) => title,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    destination_title_id,
                    error = %error,
                    "could not read the merged title to rebuild its search projection"
                );
                return;
            }
        };
        // Writing the tags back is a no-op to the row and a rebuild to the
        // projection: the merge already wrote the merged array, and this is the
        // repository path that re-derives `title_search_terms` from it.
        if let Err(error) = self
            .app
            .services
            .catalog
            .titles
            .update_metadata(destination_title_id, None, None, Some(title.tags), None)
            .await
        {
            tracing::warn!(
                destination_title_id,
                error = %error,
                "could not rebuild the merged title's search projection; it rebuilds on the title's next write"
            );
        }
    }

    async fn regenerate_recommendations(&self, destination_title_id: &str) {
        let Ok(Some(title)) = self
            .app
            .services
            .catalog
            .titles
            .get_by_id(destination_title_id)
            .await
        else {
            return;
        };
        if let Err(error) = self
            .app
            .queue_title_more_like_this_refresh_if_due(
                &title,
                crate::catalog::workflow::HydrationSource::BackgroundDue,
            )
            .await
        {
            tracing::warn!(
                destination_title_id,
                error = %error,
                "could not queue the merged title's recommendation refresh"
            );
        }
    }

    async fn drop_source_coverage(&self, request: &PostMergeWorkRequest) {
        let mut scope_keys = vec![format!("title:{}", request.source_title_id)];
        scope_keys.extend(
            request
                .retired_episode_ids
                .iter()
                .map(|id| format!("episode:{id}")),
        );
        for collection_id in &request.retired_collection_ids {
            scope_keys.push(format!("collection:{collection_id}"));
            scope_keys.push(format!("series_pack_collection:{collection_id}"));
        }
        scope_keys.extend(
            request
                .retired_series_movie_link_ids
                .iter()
                .map(|id| format!("series_movie:{id}")),
        );
        for scope_key in scope_keys {
            self.app.prune_scope_key_coverage(&scope_key, None).await;
        }
    }
}

// ── FR-088 ───────────────────────────────────────────────────────────────────

/// The production [`LocationMediaServerRefresh`]: hands the changed folders to
/// the media-server subsystem, which owns the connections and their path
/// mappings.
///
/// Thin on purpose — the decision about *which* folders is
/// [`crate::location::media_server_refresh`]'s and the decision about *which
/// servers* is [`crate::media_servers`]'s, and neither belongs in the operation
/// runner.
struct AppUseCaseMediaServerRefresh {
    app: AppUseCase,
}

#[async_trait::async_trait]
impl LocationMediaServerRefresh for AppUseCaseMediaServerRefresh {
    async fn refresh_media_servers(&self, request: MediaServerRefreshRequest) -> AppResult<()> {
        self.app
            .refresh_media_server_folders(&request.operation_id, &request.folders)
            .await
    }
}

/// The execution mode a plan with no file-bearing title takes, exposed so a
/// caller can tell the FR-076 fast path from a managed move without inspecting
/// items.
pub fn is_catalog_only(plan: &LocationPlan) -> bool {
    plan.header.mode == LocationExecutionMode::CatalogOnly
}

/// Verification depth a plan will apply, for a caller that only needs the
/// stamp (FR-043).
pub fn verification_depth(plan: &LocationPlan) -> VerificationDepth {
    plan.verification.depth
}
