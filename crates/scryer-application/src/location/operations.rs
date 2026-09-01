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

use crate::location::classify::{
    DestinationLibraryFacts, DestinationRequest, SelectionClassification, TitleClassificationFacts,
    TitleLocationClass, classify_selection,
};
use crate::location::collisions::{
    CollisionNaming, ContentFacts, DestinationItem, FullHash, PathCaseRule, RecycleAvailability,
};
use crate::location::execution::{
    ImportFilePermissionsApplier, RootMoveAdmission, RootMoveCatalog, RootMoveFileMover,
    RootMoveReconciler, RecycleBinSourceRecycler, TitlePlacementSnapshot,
};
use crate::location::executor::{LocationOperationRunner, OperationRunOutcome};
use crate::location::hardlinks::detect_hardlinks;
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    LocationOperationType, VerificationDepth,
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
use crate::location::verify::{VerifiedCopier, same_filesystem};
use crate::services::AppUseCase;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

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
        let planned = self.plan_root_move(actor, &request).await?;
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
        let preview_request = RootMovePreviewRequest {
            title_ids: request.title_ids.clone(),
            destination: request.destination.clone(),
        };
        let planned = self.plan_root_move(actor, &preview_request).await?;
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
            operation_type: LocationOperationType::RootMove,
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
    /// (FR-033). Returns `None` when the operation is unknown, terminal, or was
    /// stored without its plan.
    ///
    /// A resumable operation gets a *fresh* Activity job run before the plan is
    /// handed back, and the operation row is repointed at it. Job runs are
    /// per-execution everywhere else in Scryer — the boot reconciler fails every
    /// non-terminal run it finds, so the run an interrupted operation started
    /// under is already `failed` by the time a resume happens, and reopening it
    /// would rewrite a settled Activity row. One run per attempt also keeps the
    /// jobs list honest about how many times a move was picked back up.
    pub async fn resume_location_operation(
        &self,
        operation_id: &str,
    ) -> AppResult<Option<RootMoveExecutionPlan>> {
        let Some(operation) = self.location_operation(operation_id).await? else {
            return Ok(None);
        };
        if operation.state.is_terminal() {
            return Ok(None);
        }
        if operation.operation_type != LocationOperationType::RootMove {
            // Other operation types resume through their own phases; this one
            // must not run them under root-move rules.
            return Ok(None);
        }
        let Some(plan_json) = self
            .services
            .library
            .location_operations
            .get_location_operation_plan_json(operation_id)
            .await?
        else {
            return Ok(None);
        };
        let plan: RootMoveExecutionPlan = serde_json::from_str(&plan_json).map_err(|error| {
            AppError::Repository(format!(
                "location operation {operation_id} has an unreadable plan: {error}"
            ))
        })?;

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

        Ok(Some(plan))
    }

    /// Boot hook: pick every interrupted location operation back up (FR-033).
    ///
    /// Returns how many were resumed. Operations whose plan cannot be read are
    /// left alone and logged rather than failed here, because a startup path
    /// must not decide on its own that a user's half-finished move is over.
    pub async fn resume_interrupted_location_operations(&self) -> AppResult<usize> {
        let operations = LocationOperationRunner::resumable_operations(
            self.services.library.location_operations.as_ref(),
        )
        .await?;

        let mut resumed = 0usize;
        for operation in operations {
            match self.resume_location_operation(&operation.id).await {
                Ok(Some(plan)) => {
                    tracing::info!(
                        operation_id = %operation.id,
                        state = operation.state.as_str(),
                        "resuming an interrupted location operation from its last verified checkpoint"
                    );
                    self.spawn_location_operation(operation.id.clone(), plan);
                    resumed += 1;
                }
                Ok(None) => tracing::warn!(
                    operation_id = %operation.id,
                    operation_type = operation.operation_type.as_str(),
                    "an interrupted location operation could not be resumed and is left for the user"
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
        let job_run_id = self
            .location_operation(operation_id)
            .await
            .ok()
            .flatten()
            .and_then(|operation| operation.job_run_id);

        let outcome = self.execute_root_move(operation_id, plan).await;
        if let Some(job_run_id) = job_run_id {
            self.close_location_operation_job_run_for(&job_run_id, &outcome)
                .await;
        }
        outcome
    }

    async fn execute_root_move(
        &self,
        operation_id: &str,
        plan: &RootMoveExecutionPlan,
    ) -> AppResult<OperationRunOutcome> {
        let catalog = AppUseCaseRootMoveCatalog { app: self.clone() };
        let recycler = self.root_move_recycler(plan).await;
        let permissions = self.root_move_permissions(plan).await?;
        // The mover gets its own catalog handle so each verified copy's hashes
        // land on the media file as they are produced (FR-041, migration 0205),
        // instead of the backfill job reading every moved file a second time.
        let mover = RootMoveFileMover::new(VerifiedCopier::new(), permissions.clone())
            .with_catalog(Arc::new(AppUseCaseRootMoveCatalog { app: self.clone() }));
        let admission = RootMoveAdmission::new(plan, &catalog);
        let reconciler = RootMoveReconciler::new(
            plan,
            &catalog,
            self.services.library.location_operations.as_ref(),
            &recycler,
        )
        .with_permissions(permissions);

        LocationOperationRunner::new(
            self.services.library.location_operations.as_ref(),
            &mover,
            &admission,
            &reconciler,
        )
        .with_ownership_registry(&self.runtime.library.location_ownership)
        .run(operation_id, &plan.to_work_plan())
        .await
    }

    pub fn spawn_location_operation(&self, operation_id: String, plan: RootMoveExecutionPlan) {
        let app = self.clone();
        tokio::spawn(async move {
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
    async fn open_location_operation_job_run(
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
                        Some(outcome.counters.clone()),
                    ),
                    LocationOperationState::CompletedWithWarnings => (
                        crate::JobRunStatus::Warning,
                        summary,
                        None,
                        Some(outcome.counters.clone()),
                    ),
                    LocationOperationState::Canceled => (
                        crate::JobRunStatus::Warning,
                        summary,
                        None,
                        Some(outcome.counters.clone()),
                    ),
                    LocationOperationState::Failed => (
                        crate::JobRunStatus::Failed,
                        summary,
                        Some(outcome.detail.clone().unwrap_or_else(|| {
                            "the location operation stopped on an error".to_string()
                        })),
                        Some(outcome.counters.clone()),
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
    async fn close_location_operation_job_run(
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

/// Who a location-operation job run belongs to, and why it was opened.
enum LocationJobRunActor<'a> {
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
            facts.push(fact);
            media_files_by_title.insert(title.id.clone(), media_files);
        }

        let classification =
            classify_selection(&facts, &request.destination, Some(&destination_facts));

        // Folder-naming policy for the destination library's facet (FR-013).
        let folder_template = self
            .title_folder_template_for(&destination_library.facet)
            .await?;
        let depth = self.resolve_verification_depth().await;
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
            };

            if !classified.class.moves_files() {
                drafts.push(draft);
                continue;
            }

            let Some(destination_root_path) = destination_root_path else {
                draft.class = TitleLocationClass::NeedsResolution;
                draft.blocked_reason = Some(format!(
                    "destination root {} has no configured path",
                    classified.destination_root_id
                ));
                drafts.push(draft);
                continue;
            };

            // FR-013: calculated fresh from the destination policy, which is
            // what repairs a stale folder name.
            let destination_folder = crate::library::rename::configured_title_folder_path(
                &destination_root_path.to_string_lossy(),
                title,
                &folder_template,
                title.year,
            );
            draft.destination_folder_path = Some(destination_folder.clone());

            let media_files = media_files_by_title
                .get(&title.id)
                .cloned()
                .unwrap_or_default();
            let (files, directories) =
                collect_source_files(source_folder_path.as_deref(), &media_files).await?;
            draft.files = files;
            draft.source_directories = directories;

            draft.same_volume = match source_folder_path.as_deref() {
                Some(folder) => Some(same_filesystem(folder, &destination_folder).await),
                None => None,
            };
            draft.hardlinks =
                detect_hardlinks(draft.files.iter().map(|file| file.path.clone()).collect())
                    .await?;
            draft.destination_entries = read_destination_entries(&destination_folder).await;
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
            classification: classification.counts,
            verification_depth: depth,
            free_space,
            case_rule,
            naming: CollisionNaming::from_source_library(&source_library_label(&libraries)),
        });

        Ok(RootMovePlanning {
            planned,
            classification,
        })
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
    async fn active_work_blocking_a_move(&self, title: &Title) -> AppResult<Option<String>> {
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
async fn collect_source_files(
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

/// What already sits in the destination folder, for the collision planner
/// (FR-072–075). An absent folder simply has nothing in it.
async fn read_destination_entries(destination_folder: &Path) -> Vec<DestinationItem> {
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
        let path = path_to_stored_string(&entry.path());
        items.push(
            DestinationItem::companion(name, metadata.len())
                .with_content(ContentFacts::new(metadata.len()))
                .with_path(path),
        );
    }
    items
}

/// A refused confirmation keeps its prose *and* its code: the client re-previews
/// on a stale plan and unblocks a selection on a blocked one, and it should not
/// have to read a sentence to tell the two apart (FR-016, FR-081).
fn confirmation_error(error: PlanConfirmationError) -> AppError {
    let message = match error {
        PlanConfirmationError::Stale => {
            "the preview no longer matches what is on disk or in the catalog; review a fresh preview before confirming"
        }
        PlanConfirmationError::Blocked => {
            "some selected titles still need a decision; resolve or remove them before starting"
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
