//! In-memory stand-ins for the location subsystem's persistence, shared by the
//! module tests that need a real store rather than a null one.
//!
//! This mirrors migration 0206's behaviour closely enough for the runner's
//! contracts to be asserted end-to-end: checkpoints survive between runs,
//! verification records are idempotent on (operation, destination path) — which
//! is what makes "resume never repeats verified work" testable — and ownership
//! claims are all-or-nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::location::model::{
    FileVerificationRecord, LocationOperation, LocationOperationState, TitleCheckpoint,
};
use crate::location::ownership_guard::{GuardedAction, OwnedEntity, OwnershipConflict};
use crate::ports::{
    LocationOperationProgress, LocationOperationRepository, LocationOwnershipClaim,
    LocationOwnershipOutcome,
};
use crate::AppResult;

#[derive(Default)]
struct State {
    operations: BTreeMap<String, LocationOperation>,
    plans: BTreeMap<String, String>,
    checkpoints: BTreeMap<(String, String), TitleCheckpoint>,
    /// Keyed the way 0206's unique index is: one row per
    /// (operation, destination path).
    verifications: BTreeMap<(String, String), FileVerificationRecord>,
    ownership: BTreeMap<(String, String), String>,
    cancel_requested: BTreeSet<String>,
    /// Cancel checks the runner has made against this store so far.
    cancel_checks: usize,
    /// One-shot fault: the `nth` cancel check fails instead of answering.
    crash_on_cancel_check: Option<usize>,
    /// The cancel check at which a user's cancel lands, for a test that needs
    /// the request to arrive at a known title boundary.
    cancel_at_cancel_check: Option<usize>,
    /// Once a checkpoint write in this state is attempted, every checkpoint
    /// write fails until the fault is cleared.
    fail_checkpoint_writes_from: Option<crate::location::model::TitleCheckpointState>,
    /// Whether that fault has been triggered and is now failing every write.
    checkpoint_writes_failing: bool,
}

/// An in-memory [`LocationOperationRepository`].
#[derive(Default)]
pub(crate) struct InMemoryLocationOperationStore {
    state: Mutex<State>,
}

impl InMemoryLocationOperationStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert_operation(&self, operation: LocationOperation) {
        self.state
            .lock()
            .expect("lock")
            .operations
            .insert(operation.id.clone(), operation);
    }

    pub(crate) fn operation(&self, operation_id: &str) -> Option<LocationOperation> {
        self.state
            .lock()
            .expect("lock")
            .operations
            .get(operation_id)
            .cloned()
    }

    pub(crate) fn checkpoint(&self, operation_id: &str, title_id: &str) -> Option<TitleCheckpoint> {
        self.state
            .lock()
            .expect("lock")
            .checkpoints
            .get(&(operation_id.to_string(), title_id.to_string()))
            .cloned()
    }

    pub(crate) fn verifications(&self) -> Vec<FileVerificationRecord> {
        self.state
            .lock()
            .expect("lock")
            .verifications
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn open_claim_count(&self) -> usize {
        self.state.lock().expect("lock").ownership.len()
    }

    /// Arms a one-shot store failure on the `nth` cancel check the runner makes.
    ///
    /// This is how a test simulates a process that dies mid-operation. The
    /// runner reads the cancel flag once per unprocessed title, at the title
    /// boundary, so arming `2` stops a two-title run after the first title has
    /// settled. The error propagates out of `run` before any terminal state is
    /// written, which is precisely the row a crash leaves behind: non-terminal,
    /// with checkpoints for the titles that did settle and ownership claims
    /// still held (FR-033, FR-084).
    pub(crate) fn crash_on_cancel_check(&self, nth: usize) {
        self.state.lock().expect("lock").crash_on_cancel_check = Some(nth);
    }

    /// Lands a user's cancel on the `nth` cancel check the runner makes.
    ///
    /// A cancel arrives from another task while the runner is working, so a
    /// test that calls `cancel_location_operation` after starting a move is
    /// racing the runner. Arming the boundary instead makes the stop
    /// deterministic without changing what the runner sees: the flag it reads
    /// at a title checkpoint is set, exactly as a persisted cancel request
    /// would leave it (FR-092).
    pub(crate) fn cancel_at_cancel_check(&self, nth: usize) {
        self.state.lock().expect("lock").cancel_at_cancel_check = Some(nth);
    }

    /// Kills the store from the moment a checkpoint reaches `state`.
    ///
    /// This is the crash window a merge needs and a cancel-check fault cannot
    /// reach: `execute_title_merge` is one transaction, so the interesting
    /// interruption is *after* it commits and *before* its checkpoint settles.
    /// The reconciler runs between the `reconciling` and `cleaning_up`
    /// checkpoint writes, so arming `CleaningUp` puts the failure exactly
    /// there. Every later write fails too — including the one that would settle
    /// the title as failed — so the run aborts with the operation left
    /// non-terminal, which is what a process that died looks like.
    pub(crate) fn fail_checkpoint_writes_from(
        &self,
        state: crate::location::model::TitleCheckpointState,
    ) {
        let mut guard = self.state.lock().expect("lock");
        guard.fail_checkpoint_writes_from = Some(state);
        guard.checkpoint_writes_failing = false;
    }

    /// Brings the store back, so the operation can be resumed.
    pub(crate) fn clear_checkpoint_faults(&self) {
        let mut guard = self.state.lock().expect("lock");
        guard.fail_checkpoint_writes_from = None;
        guard.checkpoint_writes_failing = false;
    }
}

/// The ordinal of the cancel check the runner makes at the boundary *before*
/// the `nth` title (1-based) of a plan whose titles each hold `files_per_title`
/// files.
///
/// The runner reads the cancel flag twice over: once at each title boundary and
/// once before each of that title's files, so a cancel can land inside a title
/// as well as between two of them (FR-092). The boundary ordinals are therefore
/// not the title numbers, and a test that wants a title boundary has to say so
/// rather than count.
pub(crate) fn title_boundary_cancel_check(nth: usize, files_per_title: usize) -> usize {
    (nth - 1) * (1 + files_per_title) + 1
}

#[async_trait]
impl LocationOperationRepository for InMemoryLocationOperationStore {
    async fn create_location_operation(
        &self,
        operation: &LocationOperation,
        plan_json: Option<&str>,
    ) -> AppResult<()> {
        let mut state = self.state.lock().expect("lock");
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
            .state
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
            .state
            .lock()
            .expect("lock")
            .plans
            .get(operation_id)
            .cloned())
    }

    async fn list_active_location_operations(&self) -> AppResult<Vec<LocationOperation>> {
        let state = self.state.lock().expect("lock");
        let mut active: Vec<LocationOperation> = state
            .operations
            .values()
            .filter(|operation| operation.state.is_active())
            .cloned()
            .collect();
        active.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        Ok(active)
    }

    async fn update_location_operation_progress(
        &self,
        progress: &LocationOperationProgress,
    ) -> AppResult<()> {
        let mut state = self.state.lock().expect("lock");
        if let Some(operation) = state.operations.get_mut(&progress.operation_id) {
            operation.state = progress.state;
            operation.counters = progress.counters;
            operation.verification_fallback_count = progress.verification_fallback_count;
            // The SQL store stamps `updated_at` on every progress write, and
            // the staleness heuristic that decides whether to offer a resume
            // reads exactly that column, so the in-memory twin has to as well.
            operation.updated_at = chrono::Utc::now();
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
        let mut state = self.state.lock().expect("lock");
        let Some(operation) = state.operations.get_mut(operation_id) else {
            return Err(crate::AppError::NotFound(format!(
                "location operation {operation_id}"
            )));
        };
        operation.job_run_id = Some(job_run_id.to_string());
        operation.updated_at = chrono::Utc::now();
        Ok(())
    }

    async fn request_location_operation_cancel(&self, operation_id: &str) -> AppResult<bool> {
        let mut state = self.state.lock().expect("lock");
        let terminal = state
            .operations
            .get(operation_id)
            .is_some_and(|operation| operation.state.is_terminal());
        if terminal {
            return Ok(false);
        }
        if let Some(operation) = state.operations.get_mut(operation_id) {
            operation.cancel_requested = true;
            operation.cancel_requested_at = Some(chrono::Utc::now());
        }
        Ok(state.cancel_requested.insert(operation_id.to_string()))
    }

    async fn location_operation_cancel_requested(&self, operation_id: &str) -> AppResult<bool> {
        let mut state = self.state.lock().expect("lock");
        state.cancel_checks += 1;
        if state.crash_on_cancel_check == Some(state.cancel_checks) {
            state.crash_on_cancel_check = None;
            return Err(crate::AppError::Repository(
                "the store went away mid-operation".to_string(),
            ));
        }
        if state.cancel_at_cancel_check == Some(state.cancel_checks) {
            state.cancel_at_cancel_check = None;
            state.cancel_requested.insert(operation_id.to_string());
            if let Some(operation) = state.operations.get_mut(operation_id) {
                operation.cancel_requested = true;
                operation.cancel_requested_at = Some(chrono::Utc::now());
            }
        }
        Ok(state.cancel_requested.contains(operation_id))
    }

    async fn upsert_location_title_checkpoint(
        &self,
        checkpoint: &TitleCheckpoint,
    ) -> AppResult<()> {
        let mut state = self.state.lock().expect("lock");
        if state.fail_checkpoint_writes_from == Some(checkpoint.state) {
            state.checkpoint_writes_failing = true;
        }
        if state.checkpoint_writes_failing {
            return Err(crate::AppError::Repository(
                "the store went away before the checkpoint could be written".to_string(),
            ));
        }
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
        let state = self.state.lock().expect("lock");
        let mut checkpoints: Vec<TitleCheckpoint> = state
            .checkpoints
            .values()
            .filter(|checkpoint| checkpoint.operation_id == operation_id)
            .cloned()
            .collect();
        checkpoints.sort_by_key(|checkpoint| checkpoint.sequence);
        Ok(checkpoints)
    }

    async fn record_location_file_verification(
        &self,
        record: &FileVerificationRecord,
    ) -> AppResult<()> {
        self.state.lock().expect("lock").verifications.insert(
            (record.operation_id.clone(), record.destination_path.clone()),
            record.clone(),
        );
        Ok(())
    }

    async fn list_location_file_verifications(
        &self,
        operation_id: &str,
        title_id: Option<&str>,
    ) -> AppResult<Vec<FileVerificationRecord>> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .verifications
            .values()
            .filter(|record| record.operation_id == operation_id)
            .filter(|record| title_id.is_none_or(|title_id| record.title_id == title_id))
            .cloned()
            .collect())
    }

    async fn verified_destination_paths(
        &self,
        operation_id: &str,
        title_id: &str,
    ) -> AppResult<BTreeSet<String>> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .verifications
            .values()
            .filter(|record| record.operation_id == operation_id && record.title_id == title_id)
            .filter(|record| record.outcome.permits_source_removal())
            .map(|record| record.destination_path.clone())
            .collect())
    }

    async fn claim_location_operation_ownership(
        &self,
        operation_id: &str,
        entities: &[OwnedEntity],
    ) -> AppResult<LocationOwnershipOutcome> {
        let mut state = self.state.lock().expect("lock");
        let mut conflicts = Vec::new();
        for entity in entities {
            let key = (entity.kind_str().to_string(), entity.id().to_string());
            if let Some(holder) = state.ownership.get(&key)
                && holder != operation_id
            {
                conflicts.push(OwnershipConflict {
                    entity: entity.clone(),
                    operation_id: holder.clone(),
                    action: GuardedAction::LocationOperation,
                });
            }
        }
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
        let mut state = self.state.lock().expect("lock");
        let before = state.ownership.len();
        state.ownership.retain(|_, holder| holder != operation_id);
        Ok((before - state.ownership.len()) as u64)
    }

    async fn location_ownership_holder(
        &self,
        entity: &OwnedEntity,
    ) -> AppResult<Option<String>> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ownership
            .get(&(entity.kind_str().to_string(), entity.id().to_string()))
            .cloned())
    }

    async fn list_location_ownership_claims(&self) -> AppResult<Vec<LocationOwnershipClaim>> {
        let now = chrono::Utc::now();
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ownership
            .iter()
            .filter_map(|((kind, id), operation_id)| {
                let entity = match kind.as_str() {
                    "title" => OwnedEntity::Title(id.clone()),
                    "root" => OwnedEntity::Root(id.clone()),
                    _ => return None,
                };
                Some(LocationOwnershipClaim {
                    operation_id: operation_id.clone(),
                    entity,
                    acquired_at: now,
                })
            })
            .collect())
    }
}

// ── The US7 merge engine, in memory ──────────────────────────────────────────

/// An in-memory [`TitleMergeRepository`] over the same catalog repositories the
/// use case holds.
///
/// The SQL engine's own semantics — the union rules, the FR-067 gate, the
/// destination-wins collisions, the `domain_events` payload rewrite — are
/// proven against a real schema in `title_merge_store::tests`. What a story
/// test needs from a merge is narrower and is exactly what this reproduces: the
/// snapshot Group 0 would read, and the two catalog facts the pipeline is built
/// on — the source title's media becomes the destination title's, and the
/// source title row is gone.
///
/// The repositories are bound *after* the use case is built, because the
/// builder needs this store before the app it reads from exists.
#[derive(Default)]
pub(crate) struct InMemoryTitleMergeStore {
    catalog: Mutex<Option<MergeCatalogHandles>>,
    /// Snapshot overrides keyed `(source, destination)`, for a test that needs
    /// the FR-066 refusal without seeding a whole episodic catalog.
    snapshots: Mutex<BTreeMap<(String, String), crate::location::merge::engine::MergeCatalogSnapshot>>,
    executed: Mutex<Vec<crate::location::merge::engine::MergePlan>>,
    /// Operation ids Group 0 was asked to exclude from the OQ7 check.
    excluded_operations: Mutex<Vec<Option<String>>>,
}

struct MergeCatalogHandles {
    titles: std::sync::Arc<dyn crate::ports::TitleRepository>,
    media_files: std::sync::Arc<dyn crate::ports::MediaFileRepository>,
}

impl InMemoryTitleMergeStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn bind(
        &self,
        titles: std::sync::Arc<dyn crate::ports::TitleRepository>,
        media_files: std::sync::Arc<dyn crate::ports::MediaFileRepository>,
    ) {
        *self.catalog.lock().expect("lock") = Some(MergeCatalogHandles {
            titles,
            media_files,
        });
    }

    /// Force what Group 0 returns for one pair, so a test can stage an FR-066
    /// block.
    pub(crate) fn stage_snapshot(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
        snapshot: crate::location::merge::engine::MergeCatalogSnapshot,
    ) {
        self.snapshots.lock().expect("lock").insert(
            (source_title_id.to_string(), destination_title_id.to_string()),
            snapshot,
        );
    }

    /// The plans `execute_title_merge` actually ran, in order.
    pub(crate) fn executed(&self) -> Vec<crate::location::merge::engine::MergePlan> {
        self.executed.lock().expect("lock").clone()
    }

    /// The `current_operation_id` each Group 0 read was given (OQ7).
    pub(crate) fn excluded_operations(&self) -> Vec<Option<String>> {
        self.excluded_operations.lock().expect("lock").clone()
    }

    /// The bound repositories, cloned out so no lock guard is held across an
    /// await.
    fn handles(
        &self,
    ) -> (
        std::sync::Arc<dyn crate::ports::TitleRepository>,
        std::sync::Arc<dyn crate::ports::MediaFileRepository>,
    ) {
        let guard = self.catalog.lock().expect("lock");
        let catalog = guard
            .as_ref()
            .expect("the merge store was not bound to a catalog");
        (catalog.titles.clone(), catalog.media_files.clone())
    }
}

#[async_trait]
impl crate::location::merge::engine::TitleMergeRepository for InMemoryTitleMergeStore {
    async fn load_merge_snapshot(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
        current_operation_id: Option<&str>,
    ) -> AppResult<crate::location::merge::engine::MergeCatalogSnapshot> {
        self.excluded_operations
            .lock()
            .expect("lock")
            .push(current_operation_id.map(str::to_string));
        if let Some(staged) = self
            .snapshots
            .lock()
            .expect("lock")
            .get(&(
                source_title_id.to_string(),
                destination_title_id.to_string(),
            ))
            .cloned()
        {
            return Ok(staged);
        }

        let (titles, media_files) = self.handles();

        let source = titles
            .get_by_id(source_title_id)
            .await?
            .ok_or_else(|| crate::AppError::NotFound(format!("title {source_title_id}")))?;
        let destination = titles
            .get_by_id(destination_title_id)
            .await?
            .ok_or_else(|| crate::AppError::NotFound(format!("title {destination_title_id}")))?;
        let source_media = media_files.list_media_files_for_title(source_title_id).await?;

        Ok(crate::location::merge::engine::MergeCatalogSnapshot {
            source_title_id: source_title_id.to_string(),
            destination_title_id: destination_title_id.to_string(),
            destination_title_name: Some(destination.name.clone()),
            source_library_id: Some(source.library_id.clone()),
            destination_library_id: Some(destination.library_id.clone()),
            source_tags: source.tags.clone(),
            destination_tags: destination.tags.clone(),
            source_row_counts: BTreeMap::from([(
                "media_files".to_string(),
                source_media.len() as i64,
            )]),
            ..crate::location::merge::engine::MergeCatalogSnapshot::default()
        })
    }

    async fn execute_title_merge(
        &self,
        plan: &crate::location::merge::engine::MergePlan,
    ) -> AppResult<crate::location::merge::engine::MergeOutcome> {
        let (titles, media_files) = self.handles();

        // Group 1: the source title's media becomes the destination title's.
        // The in-memory media-file repository has no repoint, so the row is
        // re-inserted under the surviving title and the source row removed —
        // the same observable end state the SQL `UPDATE media_files SET
        // title_id` reaches.
        let mut rows_affected = BTreeMap::new();
        let source_media = media_files
            .list_media_files_for_title(&plan.source_title_id)
            .await?;
        for file in &source_media {
            media_files
                .insert_media_file(&crate::InsertMediaFileInput {
                    title_id: plan.destination_title_id.clone(),
                    file_path: file.file_path.clone(),
                    size_bytes: file.size_bytes,
                    role: file.role,
                    ..Default::default()
                })
                .await?;
            media_files.delete_media_file(&file.id).await?;
        }
        rows_affected.insert("1:media_files".to_string(), source_media.len() as u64);

        // Group 3: the merged tag array lands on the destination title.
        titles
            .update_metadata(
                &plan.destination_title_id,
                None,
                None,
                Some(plan.tags.merged_tags.clone()),
                None,
            )
            .await?;

        // Group 5: the source title row goes, and only now (FR-067).
        titles.delete(&plan.source_title_id).await?;
        rows_affected.insert("5:titles".to_string(), 1);

        self.executed.lock().expect("lock").push(plan.clone());
        Ok(crate::location::merge::engine::MergeOutcome {
            source_title_id: plan.source_title_id.clone(),
            destination_title_id: plan.destination_title_id.clone(),
            rows_affected,
            domain_event_payloads_rewritten: 0,
            post_merge_work: plan.summary.post_merge_work.clone(),
        })
    }
}

/// A confirmed, queued operation row for a test to run the runner against.
pub(crate) fn queued_operation(
    operation_id: &str,
    operation_type: crate::location::model::LocationOperationType,
    mode: crate::location::model::LocationExecutionMode,
    depth: crate::location::model::VerificationDepth,
) -> LocationOperation {
    let now = chrono::Utc::now();
    LocationOperation {
        id: operation_id.to_string(),
        operation_type,
        mode,
        state: LocationOperationState::Queued,
        initiated_by_user_id: None,
        source_library_id: None,
        destination_library_id: None,
        source_root_id: None,
        destination_root_id: None,
        plan_fingerprint: "test-fingerprint".to_string(),
        verification_depth: depth,
        verification_fallback_count: 0,
        counters: crate::location::model::LocationOperationCounters::default(),
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
