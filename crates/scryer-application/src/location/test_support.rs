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
        Ok(self
            .state
            .lock()
            .expect("lock")
            .cancel_requested
            .contains(operation_id))
    }

    async fn upsert_location_title_checkpoint(
        &self,
        checkpoint: &TitleCheckpoint,
    ) -> AppResult<()> {
        self.state.lock().expect("lock").checkpoints.insert(
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
