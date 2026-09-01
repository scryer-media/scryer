//! Persistence for location operations (migration 0206, plan D5/D7).
//!
//! One store behind all six operation types: the operation row, its per-title
//! checkpoints, its per-file verification records, and the ownership registry
//! the concurrency guard reads. Dual-engine through [`SqlRuntime`], following
//! the `workflow_operation_store` precedent.
//!
//! # Column mappings worth knowing
//!
//! - A checkpoint row is written when its title *starts*, so `created_at` is the
//!   title's start instant and `checkpointed_at` is the instant it settled.
//! - One explanation, one column. A checkpoint's detail goes to `blocked_reason`
//!   for a blocked title, to `failure_reason` for a failed one, and to `note`
//!   otherwise (the completed-with-warnings case). A verification's detail goes
//!   to `fallback_reason` when the quick floor was a fallback, to
//!   `failure_reason` when the outcome refuses source removal, and to `detail`
//!   otherwise. Reads take whichever column is populated, so a row written with
//!   any of them NULL still reads back.
//! - The operation row carries the FR-091 outcome counters (`merge_count`,
//!   `dedup_count`, `rename_count`, `no_op_count`, `unresolved_count`) next to
//!   the volume counters, so `LocationOperationCounters` round-trips whole.
//! - `move_crc` is TEXT: the CRC is a `u64` and would not survive a signed
//!   64-bit integer column, so it is stored as its decimal string.

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::location::model::{
    AppliedVerificationDepth, FileVerificationOutcome, FileVerificationRecord, LocationExecutionMode,
    LocationOperation, LocationOperationCounters, LocationOperationState, LocationOperationType,
    MoveCrcAlgorithm, StreamedContentHashes, TitleCheckpoint, TitleCheckpointPlacement,
    TitleCheckpointState, VerificationDepth,
};
use scryer_application::location::classify::TitleLocationClass;
use scryer_application::location::ownership_guard::{GuardedAction, OwnedEntity, OwnershipConflict};
use scryer_application::{
    AppError, AppResult, LocationOperationProgress, LocationOperationRepository,
    LocationOwnershipClaim, LocationOwnershipOutcome,
};
use std::collections::BTreeSet;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const OPERATION_COLUMNS: &str = "id, operation_type, execution_mode, state, initiated_by_user_id,
    source_library_id, source_root_id, destination_library_id, destination_root_id,
    plan_fingerprint, verification_depth, verification_fallback_count,
    title_total, title_completed_count, title_blocked_count,
    file_total, file_completed_count, bytes_total, bytes_completed,
    merge_count, dedup_count, rename_count, no_op_count, unresolved_count,
    job_run_id, workflow_operation_id, cancel_requested, cancel_requested_at,
    failure_reason, confirmed_at, started_at, completed_at, created_at, updated_at";

const CHECKPOINT_COLUMNS: &str = "operation_id, title_id, sequence, state, classification,
    source_library_id, source_root_id, source_folder_path,
    destination_library_id, destination_root_id, destination_folder_path,
    merged_into_title_id, file_total, file_completed_count, bytes_total, bytes_completed,
    blocked_reason, failure_reason, note, checkpointed_at, created_at, updated_at";

const VERIFICATION_COLUMNS: &str = "id, operation_id, title_id, media_file_id, source_path,
    destination_path, size_bytes, requested_depth, applied_depth, fell_back, fallback_reason,
    outcome, move_crc, move_crc_algorithm, full_blake3, failure_reason, detail, verified_at";

/// States an operation can no longer be cancelled or resumed from.
const TERMINAL_STATES: &str = "'completed', 'completed_with_warnings', 'canceled', 'failed'";

#[derive(Clone)]
pub struct LocationOperationStore {
    datastore: StoreDatastore,
}

impl LocationOperationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl LocationOperationRepository for LocationOperationStore {
    async fn create_location_operation(
        &self,
        operation: &LocationOperation,
        plan_json: Option<&str>,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "create_location_operation",
            "INSERT INTO location_operations
             (id, operation_type, execution_mode, state, initiated_by_user_id,
              source_library_id, source_root_id, destination_library_id, destination_root_id,
              plan_fingerprint, plan_json, verification_depth, verification_fallback_count,
              title_total, title_completed_count, title_blocked_count,
              file_total, file_completed_count, bytes_total, bytes_completed,
              merge_count, dedup_count, rename_count, no_op_count, unresolved_count,
              job_run_id, workflow_operation_id, cancel_requested, cancel_requested_at,
              failure_reason, confirmed_at, started_at, completed_at, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
                     {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(operation.id.clone()),
                SqlArg::Text(operation.operation_type.as_str().to_string()),
                SqlArg::Text(operation.mode.as_str().to_string()),
                SqlArg::Text(operation.state.as_str().to_string()),
                SqlArg::OptText(operation.initiated_by_user_id.clone()),
                SqlArg::OptText(operation.source_library_id.clone()),
                SqlArg::OptText(operation.source_root_id.clone()),
                SqlArg::OptText(operation.destination_library_id.clone()),
                SqlArg::OptText(operation.destination_root_id.clone()),
                SqlArg::Text(operation.plan_fingerprint.clone()),
                SqlArg::OptText(plan_json.map(str::to_string)),
                SqlArg::Text(operation.verification_depth.as_str().to_string()),
                SqlArg::I64(operation.verification_fallback_count),
                SqlArg::I64(operation.counters.titles_total),
                SqlArg::I64(operation.counters.titles_processed),
                SqlArg::I64(operation.counters.titles_blocked),
                SqlArg::I64(operation.counters.files_total),
                SqlArg::I64(operation.counters.files_processed),
                SqlArg::I64(operation.counters.bytes_total),
                SqlArg::I64(operation.counters.bytes_processed),
                SqlArg::I64(operation.counters.merges),
                SqlArg::I64(operation.counters.dedups),
                SqlArg::I64(operation.counters.renames),
                SqlArg::I64(operation.counters.no_ops),
                SqlArg::I64(operation.counters.unresolved),
                SqlArg::OptText(operation.job_run_id.clone()),
                SqlArg::OptText(operation.workflow_operation_id.clone()),
                SqlArg::Bool(operation.cancel_requested),
                SqlArg::OptTimestamp(operation.cancel_requested_at),
                SqlArg::OptText(operation.detail.clone()),
                SqlArg::OptTimestamp(operation.confirmed_at),
                SqlArg::OptTimestamp(operation.started_at),
                SqlArg::OptTimestamp(operation.completed_at),
                SqlArg::Timestamp(operation.created_at),
                SqlArg::Timestamp(operation.updated_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_location_operation(
        &self,
        operation_id: &str,
    ) -> AppResult<Option<LocationOperation>> {
        let sql = format!("SELECT {OPERATION_COLUMNS} FROM location_operations WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(operation_id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_operation)
        .transpose()
    }

    async fn get_location_operation_plan_json(
        &self,
        operation_id: &str,
    ) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT plan_json FROM location_operations WHERE id = {}",
            &[SqlArg::Text(operation_id.to_string())],
        )
        .await?;
        match row {
            Some(row) => row.opt_text("plan_json"),
            None => Ok(None),
        }
    }

    async fn list_active_location_operations(&self) -> AppResult<Vec<LocationOperation>> {
        let sql = format!(
            "SELECT {OPERATION_COLUMNS} FROM location_operations
             WHERE state NOT IN ({TERMINAL_STATES})
             ORDER BY created_at ASC, id ASC"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[])
            .await?
            .iter()
            .map(row_to_operation)
            .collect()
    }

    async fn update_location_operation_progress(
        &self,
        progress: &LocationOperationProgress,
    ) -> AppResult<()> {
        // `started_at` is only ever set once: a resume must not rewrite when the
        // operation first began. `completed_at` is only set when a terminal
        // state supplies one.
        let detail_clause = if progress.clear_detail {
            "failure_reason = {}"
        } else {
            "failure_reason = COALESCE({}, failure_reason)"
        };
        let sql = format!(
            "UPDATE location_operations
             SET state = {{}},
                 title_total = {{}},
                 title_completed_count = {{}},
                 title_blocked_count = {{}},
                 file_total = {{}},
                 file_completed_count = {{}},
                 bytes_total = {{}},
                 bytes_completed = {{}},
                 merge_count = {{}},
                 dedup_count = {{}},
                 rename_count = {{}},
                 no_op_count = {{}},
                 unresolved_count = {{}},
                 verification_fallback_count = {{}},
                 {detail_clause},
                 started_at = COALESCE(started_at, {{}}),
                 completed_at = COALESCE({{}}, completed_at),
                 updated_at = {{}}
             WHERE id = {{}}"
        );

        let updated = SqlRuntime::execute_write(
            &self.datastore,
            "update_location_operation_progress",
            &sql,
            vec![
                SqlArg::Text(progress.state.as_str().to_string()),
                SqlArg::I64(progress.counters.titles_total),
                SqlArg::I64(progress.counters.titles_processed),
                SqlArg::I64(progress.counters.titles_blocked),
                SqlArg::I64(progress.counters.files_total),
                SqlArg::I64(progress.counters.files_processed),
                SqlArg::I64(progress.counters.bytes_total),
                SqlArg::I64(progress.counters.bytes_processed),
                SqlArg::I64(progress.counters.merges),
                SqlArg::I64(progress.counters.dedups),
                SqlArg::I64(progress.counters.renames),
                SqlArg::I64(progress.counters.no_ops),
                SqlArg::I64(progress.counters.unresolved),
                SqlArg::I64(progress.verification_fallback_count),
                SqlArg::OptText(progress.detail.clone()),
                SqlArg::OptTimestamp(progress.started_at),
                SqlArg::OptTimestamp(progress.completed_at),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(progress.operation_id.clone()),
            ],
        )
        .await?;

        if updated == 0 {
            return Err(AppError::NotFound(format!(
                "location operation {}",
                progress.operation_id
            )));
        }
        Ok(())
    }

    async fn set_location_operation_job_run(
        &self,
        operation_id: &str,
        job_run_id: &str,
    ) -> AppResult<()> {
        let updated = SqlRuntime::execute_write(
            &self.datastore,
            "set_location_operation_job_run",
            "UPDATE location_operations
             SET job_run_id = {}, updated_at = {}
             WHERE id = {}",
            vec![
                SqlArg::Text(job_run_id.to_string()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(operation_id.to_string()),
            ],
        )
        .await?;

        if updated == 0 {
            return Err(AppError::NotFound(format!(
                "location operation {operation_id}"
            )));
        }
        Ok(())
    }

    async fn request_location_operation_cancel(&self, operation_id: &str) -> AppResult<bool> {
        let sql = format!(
            "UPDATE location_operations
             SET cancel_requested = {{}}, cancel_requested_at = {{}}, updated_at = {{}}
             WHERE id = {{}} AND state NOT IN ({TERMINAL_STATES})"
        );
        let now = Utc::now();
        let updated = SqlRuntime::execute_write(
            &self.datastore,
            "request_location_operation_cancel",
            &sql,
            vec![
                SqlArg::Bool(true),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::Text(operation_id.to_string()),
            ],
        )
        .await?;
        Ok(updated > 0)
    }

    async fn location_operation_cancel_requested(&self, operation_id: &str) -> AppResult<bool> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT cancel_requested FROM location_operations WHERE id = {}",
            &[SqlArg::Text(operation_id.to_string())],
        )
        .await?;
        match row {
            Some(row) => row.bool("cancel_requested"),
            None => Ok(false),
        }
    }

    async fn upsert_location_title_checkpoint(
        &self,
        checkpoint: &TitleCheckpoint,
    ) -> AppResult<()> {
        // One explanation, one column: why the title could not start, why it
        // failed, or what a finished-with-warnings title still has to say.
        let (blocked_reason, failure_reason, note) = match checkpoint.state {
            TitleCheckpointState::Blocked => (checkpoint.detail.clone(), None, None),
            TitleCheckpointState::Failed => (None, checkpoint.detail.clone(), None),
            _ => (None, None, checkpoint.detail.clone()),
        };
        let started_at = checkpoint.started_at.unwrap_or(checkpoint.updated_at);

        SqlRuntime::execute_write(
            &self.datastore,
            "upsert_location_title_checkpoint",
            "INSERT INTO location_operation_title_checkpoints
             (operation_id, title_id, sequence, state, classification,
              source_library_id, source_root_id, source_folder_path,
              destination_library_id, destination_root_id, destination_folder_path,
              merged_into_title_id, file_total, file_completed_count, bytes_total, bytes_completed,
              blocked_reason, failure_reason, note, checkpointed_at, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT(operation_id, title_id) DO UPDATE SET
                sequence = excluded.sequence,
                state = excluded.state,
                classification = excluded.classification,
                source_library_id = excluded.source_library_id,
                source_root_id = excluded.source_root_id,
                source_folder_path = excluded.source_folder_path,
                destination_library_id = excluded.destination_library_id,
                destination_root_id = excluded.destination_root_id,
                destination_folder_path = excluded.destination_folder_path,
                merged_into_title_id = excluded.merged_into_title_id,
                file_total = excluded.file_total,
                file_completed_count = excluded.file_completed_count,
                bytes_total = excluded.bytes_total,
                bytes_completed = excluded.bytes_completed,
                blocked_reason = excluded.blocked_reason,
                failure_reason = excluded.failure_reason,
                note = excluded.note,
                checkpointed_at = excluded.checkpointed_at,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(checkpoint.operation_id.clone()),
                SqlArg::Text(checkpoint.title_id.clone()),
                SqlArg::I64(checkpoint.sequence),
                SqlArg::Text(checkpoint.state.as_str().to_string()),
                SqlArg::OptText(
                    checkpoint
                        .classification
                        .map(|class| class.as_str().to_string()),
                ),
                SqlArg::OptText(checkpoint.placement.source_library_id.clone()),
                SqlArg::OptText(checkpoint.placement.source_root_id.clone()),
                SqlArg::OptText(checkpoint.placement.source_folder_path.clone()),
                SqlArg::OptText(checkpoint.placement.destination_library_id.clone()),
                SqlArg::OptText(checkpoint.placement.destination_root_id.clone()),
                SqlArg::OptText(checkpoint.placement.destination_folder_path.clone()),
                SqlArg::OptText(checkpoint.placement.merged_into_title_id.clone()),
                SqlArg::I64(checkpoint.files_total),
                SqlArg::I64(checkpoint.files_verified),
                SqlArg::I64(checkpoint.bytes_total),
                SqlArg::I64(checkpoint.bytes_verified),
                SqlArg::OptText(blocked_reason),
                SqlArg::OptText(failure_reason),
                SqlArg::OptText(note),
                SqlArg::OptTimestamp(checkpoint.completed_at),
                SqlArg::Timestamp(started_at),
                SqlArg::Timestamp(checkpoint.updated_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_location_title_checkpoints(
        &self,
        operation_id: &str,
    ) -> AppResult<Vec<TitleCheckpoint>> {
        let sql = format!(
            "SELECT {CHECKPOINT_COLUMNS} FROM location_operation_title_checkpoints
             WHERE operation_id = {{}}
             ORDER BY sequence ASC, title_id ASC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(operation_id.to_string())],
        )
        .await?
        .iter()
        .map(row_to_checkpoint)
        .collect()
    }

    async fn record_location_file_verification(
        &self,
        record: &FileVerificationRecord,
    ) -> AppResult<()> {
        let fell_back = record.depth.fell_back;
        // One explanation, one column: why the quick floor was used, why the
        // destination was not accepted, or how a clean verification was proven.
        let (fallback_reason, failure_reason, detail) = if fell_back {
            (record.detail.clone(), None, None)
        } else if record.outcome.permits_source_removal() {
            (None, None, record.detail.clone())
        } else {
            (None, record.detail.clone(), None)
        };
        let hashes = record.hashes.as_ref();

        SqlRuntime::execute_write(
            &self.datastore,
            "record_location_file_verification",
            "INSERT INTO location_operation_verifications
             (id, operation_id, title_id, media_file_id, source_path, destination_path,
              size_bytes, requested_depth, applied_depth, fell_back, fallback_reason,
              outcome, move_crc, move_crc_algorithm, full_blake3, failure_reason, detail,
              verified_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT(operation_id, destination_path) DO UPDATE SET
                title_id = excluded.title_id,
                media_file_id = excluded.media_file_id,
                source_path = excluded.source_path,
                size_bytes = excluded.size_bytes,
                requested_depth = excluded.requested_depth,
                applied_depth = excluded.applied_depth,
                fell_back = excluded.fell_back,
                fallback_reason = excluded.fallback_reason,
                outcome = excluded.outcome,
                move_crc = excluded.move_crc,
                move_crc_algorithm = excluded.move_crc_algorithm,
                full_blake3 = excluded.full_blake3,
                failure_reason = excluded.failure_reason,
                detail = excluded.detail,
                verified_at = excluded.verified_at",
            vec![
                SqlArg::Text(uuid::Uuid::new_v4().to_string()),
                SqlArg::Text(record.operation_id.clone()),
                SqlArg::Text(record.title_id.clone()),
                SqlArg::OptText(record.media_file_id.clone()),
                SqlArg::Text(record.source_path.clone()),
                SqlArg::Text(record.destination_path.clone()),
                SqlArg::I64(hashes.map(|hashes| hashes.size_bytes as i64).unwrap_or(0)),
                SqlArg::Text(record.depth.requested.as_str().to_string()),
                SqlArg::Text(record.depth.applied.as_str().to_string()),
                SqlArg::Bool(fell_back),
                SqlArg::OptText(fallback_reason),
                SqlArg::Text(record.outcome.as_str().to_string()),
                SqlArg::OptText(hashes.map(|hashes| hashes.move_crc.to_string())),
                SqlArg::OptText(hashes.map(|hashes| hashes.crc_algorithm.as_str().to_string())),
                SqlArg::OptText(hashes.map(|hashes| hashes.full_blake3.clone())),
                SqlArg::OptText(failure_reason),
                SqlArg::OptText(detail),
                SqlArg::Timestamp(record.verified_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_location_file_verifications(
        &self,
        operation_id: &str,
        title_id: Option<&str>,
    ) -> AppResult<Vec<FileVerificationRecord>> {
        let mut sql = format!(
            "SELECT {VERIFICATION_COLUMNS} FROM location_operation_verifications
             WHERE operation_id = {{}}"
        );
        let mut args = vec![SqlArg::Text(operation_id.to_string())];
        if let Some(title_id) = title_id {
            sql.push_str(" AND title_id = {}");
            args.push(SqlArg::Text(title_id.to_string()));
        }
        sql.push_str(" ORDER BY verified_at ASC, destination_path ASC");

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_verification)
            .collect()
    }

    async fn verified_destination_paths(
        &self,
        operation_id: &str,
        title_id: &str,
    ) -> AppResult<BTreeSet<String>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT destination_path FROM location_operation_verifications
             WHERE operation_id = {} AND title_id = {} AND outcome = {}",
            &[
                SqlArg::Text(operation_id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(FileVerificationOutcome::Verified.as_str().to_string()),
            ],
        )
        .await?
        .iter()
        .map(|row| row.text("destination_path"))
        .collect()
    }

    async fn claim_location_operation_ownership(
        &self,
        operation_id: &str,
        entities: &[OwnedEntity],
    ) -> AppResult<LocationOwnershipOutcome> {
        if entities.is_empty() {
            return Ok(LocationOwnershipOutcome::Claimed);
        }

        let conflicts = self.conflicting_claims(operation_id, entities).await?;
        if !conflicts.is_empty() {
            return Ok(LocationOwnershipOutcome::Conflict(conflicts));
        }

        let operation = operation_id.to_string();
        let requested: Vec<OwnedEntity> = entities.to_vec();
        let entities: Vec<OwnedEntity> = entities.to_vec();
        let claim = SqlRuntime::run_in_transaction(
            &self.datastore,
            "claim_location_operation_ownership",
            move |tx| {
                let operation = operation.clone();
                let entities = entities.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    for entity in &entities {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO location_operation_owned_entities
                             (operation_id, entity_type, entity_id, ownership_mode, acquired_at, released_at)
                             VALUES ({}, {}, {}, {}, {}, NULL)
                             ON CONFLICT(operation_id, entity_type, entity_id) DO UPDATE SET
                                ownership_mode = excluded.ownership_mode,
                                acquired_at = excluded.acquired_at,
                                released_at = NULL",
                            &[
                                SqlArg::Text(operation.clone()),
                                SqlArg::Text(entity.kind_str().to_string()),
                                SqlArg::Text(entity.id().to_string()),
                                SqlArg::Text("exclusive".to_string()),
                                SqlArg::Timestamp(now),
                            ],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await;

        match claim {
            Ok(()) => Ok(LocationOwnershipOutcome::Claimed),
            // The partial unique index is what makes a lost race fail rather
            // than interleave. Re-read who won so the caller gets the same typed
            // conflict it would have got from the pre-check.
            Err(error) if is_unique_violation(&error) => {
                let conflicts = self.conflicting_claims(operation_id, requested).await?;
                if conflicts.is_empty() {
                    Err(error)
                } else {
                    Ok(LocationOwnershipOutcome::Conflict(conflicts))
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn release_location_operation_ownership(&self, operation_id: &str) -> AppResult<u64> {
        SqlRuntime::execute_write(
            &self.datastore,
            "release_location_operation_ownership",
            "UPDATE location_operation_owned_entities
             SET released_at = {}
             WHERE operation_id = {} AND released_at IS NULL",
            vec![
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(operation_id.to_string()),
            ],
        )
        .await
    }

    async fn location_ownership_holder(&self, entity: &OwnedEntity) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT operation_id FROM location_operation_owned_entities
             WHERE entity_type = {} AND entity_id = {} AND released_at IS NULL",
            &[
                SqlArg::Text(entity.kind_str().to_string()),
                SqlArg::Text(entity.id().to_string()),
            ],
        )
        .await?;
        match row {
            Some(row) => row.text("operation_id").map(Some),
            None => Ok(None),
        }
    }

    async fn list_location_ownership_claims(&self) -> AppResult<Vec<LocationOwnershipClaim>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT operation_id, entity_type, entity_id, acquired_at
             FROM location_operation_owned_entities
             WHERE released_at IS NULL
             ORDER BY acquired_at ASC, entity_type ASC, entity_id ASC",
            &[],
        )
        .await?
        .iter()
        .map(|row| {
            Ok(LocationOwnershipClaim {
                operation_id: row.text("operation_id")?,
                entity: parse_owned_entity(&row.text("entity_type")?, row.text("entity_id")?)?,
                acquired_at: row.timestamp("acquired_at")?,
            })
        })
        .collect()
    }
}

impl LocationOperationStore {
    /// Entities among `entities` that another operation currently owns.
    async fn conflicting_claims(
        &self,
        operation_id: &str,
        entities: impl AsRef<[OwnedEntity]>,
    ) -> AppResult<Vec<OwnershipConflict>> {
        let entities = entities.as_ref();
        if entities.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = vec!["{}"; entities.len()].join(", ");
        let sql = format!(
            "SELECT operation_id, entity_type, entity_id FROM location_operation_owned_entities
             WHERE released_at IS NULL AND operation_id <> {{}} AND entity_id IN ({placeholders})"
        );
        let mut args = vec![SqlArg::Text(operation_id.to_string())];
        args.extend(
            entities
                .iter()
                .map(|entity| SqlArg::Text(entity.id().to_string())),
        );

        let mut conflicts = Vec::new();
        for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
            let entity_type = row.text("entity_type")?;
            let entity_id = row.text("entity_id")?;
            // The id filter can match an entity of another kind; only a
            // (kind, id) match is a real conflict.
            if entities
                .iter()
                .any(|entity| entity.kind_str() == entity_type && entity.id() == entity_id)
            {
                conflicts.push(OwnershipConflict {
                    operation_id: row.text("operation_id")?,
                    entity: parse_owned_entity(&entity_type, entity_id)?,
                    action: GuardedAction::LocationOperation,
                });
            }
        }
        Ok(conflicts)
    }
}

/// Whether a repository error is a unique/primary-key violation.
///
/// The SQL runtime flattens every sqlx failure into `AppError::Repository`, so
/// message matching is the established classification (see the download-identity
/// stores' `unique_violation` helper). Both dialects' stable texts are covered.
fn is_unique_violation(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Repository(message)
            if message.contains("UNIQUE constraint failed")
                || message.contains("duplicate key value violates unique constraint")
    )
}

fn parse_owned_entity(entity_type: &str, entity_id: String) -> AppResult<OwnedEntity> {
    match entity_type {
        "title" => Ok(OwnedEntity::Title(entity_id)),
        "root" => Ok(OwnedEntity::Root(entity_id)),
        other => Err(AppError::Repository(format!(
            "location operation owns an unknown entity type '{other}'"
        ))),
    }
}

fn row_to_operation(row: &SqlRow) -> AppResult<LocationOperation> {
    let operation_type_raw = row.text("operation_type")?;
    let operation_type = LocationOperationType::parse(&operation_type_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "location operation has an unknown type '{operation_type_raw}'"
        ))
    })?;
    let mode_raw = row.text("execution_mode")?;
    let mode = LocationExecutionMode::parse(&mode_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "location operation has an unknown execution mode '{mode_raw}'"
        ))
    })?;
    let state_raw = row.text("state")?;
    let state = LocationOperationState::parse(&state_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "location operation has an unknown state '{state_raw}'"
        ))
    })?;
    let depth_raw = row.text("verification_depth")?;
    let verification_depth = VerificationDepth::from_setting(&depth_raw).map_err(|message| {
        AppError::Repository(format!("location operation {message}"))
    })?;

    Ok(LocationOperation {
        id: row.text("id")?,
        operation_type,
        mode,
        state,
        initiated_by_user_id: row.opt_text("initiated_by_user_id")?,
        source_library_id: row.opt_text("source_library_id")?,
        destination_library_id: row.opt_text("destination_library_id")?,
        source_root_id: row.opt_text("source_root_id")?,
        destination_root_id: row.opt_text("destination_root_id")?,
        plan_fingerprint: row.text("plan_fingerprint")?,
        verification_depth,
        verification_fallback_count: row.i64("verification_fallback_count")?,
        counters: LocationOperationCounters {
            titles_total: row.i64("title_total")?,
            titles_processed: row.i64("title_completed_count")?,
            titles_blocked: row.i64("title_blocked_count")?,
            files_total: row.i64("file_total")?,
            files_processed: row.i64("file_completed_count")?,
            bytes_total: row.i64("bytes_total")?,
            bytes_processed: row.i64("bytes_completed")?,
            merges: row.i64("merge_count")?,
            dedups: row.i64("dedup_count")?,
            renames: row.i64("rename_count")?,
            no_ops: row.i64("no_op_count")?,
            unresolved: row.i64("unresolved_count")?,
        },
        detail: row.opt_text("failure_reason")?,
        job_run_id: row.opt_text("job_run_id")?,
        workflow_operation_id: row.opt_text("workflow_operation_id")?,
        cancel_requested: row.bool("cancel_requested")?,
        cancel_requested_at: row.opt_timestamp("cancel_requested_at")?,
        confirmed_at: row.opt_timestamp("confirmed_at")?,
        started_at: row.opt_timestamp("started_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
        completed_at: row.opt_timestamp("completed_at")?,
    })
}

fn row_to_checkpoint(row: &SqlRow) -> AppResult<TitleCheckpoint> {
    let state_raw = row.text("state")?;
    let state = TitleCheckpointState::parse(&state_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "location operation checkpoint has an unknown state '{state_raw}'"
        ))
    })?;
    let classification = match row.opt_text("classification")? {
        Some(raw) => Some(TitleLocationClass::parse(&raw).ok_or_else(|| {
            AppError::Repository(format!(
                "location operation checkpoint has an unknown classification '{raw}'"
            ))
        })?),
        None => None,
    };
    // Whichever of the three explanation columns this row actually carries; a
    // checkpoint may carry none of them.
    let detail = row
        .opt_text("blocked_reason")?
        .or(row.opt_text("failure_reason")?)
        .or(row.opt_text("note")?);

    Ok(TitleCheckpoint {
        operation_id: row.text("operation_id")?,
        title_id: row.text("title_id")?,
        sequence: row.i64("sequence")?,
        state,
        classification,
        placement: TitleCheckpointPlacement {
            source_library_id: row.opt_text("source_library_id")?,
            source_root_id: row.opt_text("source_root_id")?,
            source_folder_path: row.opt_text("source_folder_path")?,
            destination_library_id: row.opt_text("destination_library_id")?,
            destination_root_id: row.opt_text("destination_root_id")?,
            destination_folder_path: row.opt_text("destination_folder_path")?,
            merged_into_title_id: row.opt_text("merged_into_title_id")?,
        },
        files_total: row.i64("file_total")?,
        files_verified: row.i64("file_completed_count")?,
        bytes_total: row.i64("bytes_total")?,
        bytes_verified: row.i64("bytes_completed")?,
        detail,
        started_at: Some(row.timestamp("created_at")?),
        updated_at: row.timestamp("updated_at")?,
        completed_at: row.opt_timestamp("checkpointed_at")?,
    })
}

fn row_to_verification(row: &SqlRow) -> AppResult<FileVerificationRecord> {
    let requested_raw = row.text("requested_depth")?;
    let requested = VerificationDepth::from_setting(&requested_raw)
        .map_err(|message| AppError::Repository(format!("verification record {message}")))?;
    let applied_raw = row.text("applied_depth")?;
    let applied = VerificationDepth::from_setting(&applied_raw)
        .map_err(|message| AppError::Repository(format!("verification record {message}")))?;
    let outcome_raw = row.text("outcome")?;
    let outcome = FileVerificationOutcome::parse(&outcome_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "verification record has an unknown outcome '{outcome_raw}'"
        ))
    })?;
    let fell_back = row.bool("fell_back")?;

    let hashes = match (
        row.opt_text("move_crc")?,
        row.opt_text("move_crc_algorithm")?,
        row.opt_text("full_blake3")?,
    ) {
        (Some(move_crc), Some(algorithm), Some(full_blake3)) => Some(StreamedContentHashes {
            size_bytes: row.i64("size_bytes")? as u64,
            crc_algorithm: MoveCrcAlgorithm::from_setting(&algorithm)
                .map_err(|message| AppError::Repository(format!("verification record {message}")))?,
            move_crc: move_crc.parse::<u64>().map_err(|error| {
                AppError::Repository(format!("verification record has an unreadable CRC: {error}"))
            })?,
            full_blake3,
        }),
        _ => None,
    };

    Ok(FileVerificationRecord {
        operation_id: row.text("operation_id")?,
        title_id: row.opt_text("title_id")?.unwrap_or_default(),
        media_file_id: row.opt_text("media_file_id")?,
        source_path: row.text("source_path")?,
        destination_path: row.text("destination_path")?,
        hashes,
        depth: AppliedVerificationDepth {
            requested,
            applied,
            fell_back,
        },
        outcome,
        // Whichever of the three explanation columns this row actually carries.
        detail: row
            .opt_text("fallback_reason")?
            .or(row.opt_text("failure_reason")?)
            .or(row.opt_text("detail")?),
        verified_at: row.timestamp("verified_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::DateTime;
    use scryer_application::location::model::MoveCrcAlgorithm;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> LocationOperationStore {
        scryer_infrastructure_datastore::register_spellfix_auto_extension()
            .expect("spellfix extension should register before migrations");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .expect("fresh migrations should apply");
        LocationOperationStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn operation(id: &str, state: LocationOperationState) -> LocationOperation {
        let now = timestamp();
        LocationOperation {
            id: id.to_string(),
            operation_type: LocationOperationType::CrossLibraryTransfer,
            mode: LocationExecutionMode::MoveWithScryer,
            state,
            // No user row exists in this fixture, and 0206's actor reference is
            // nullable precisely because the operation outlives its actor.
            initiated_by_user_id: None,
            source_library_id: Some("library-1".to_string()),
            destination_library_id: Some("library-2".to_string()),
            source_root_id: Some("root-1".to_string()),
            destination_root_id: Some("root-2".to_string()),
            plan_fingerprint: "fingerprint-1".to_string(),
            verification_depth: VerificationDepth::Quick,
            verification_fallback_count: 2,
            counters: LocationOperationCounters {
                titles_total: 4,
                titles_processed: 1,
                titles_blocked: 1,
                files_total: 9,
                files_processed: 3,
                bytes_total: 900,
                bytes_processed: 300,
                merges: 1,
                dedups: 2,
                renames: 3,
                no_ops: 4,
                unresolved: 5,
            },
            detail: Some("one title is blocked".to_string()),
            job_run_id: Some("job-1".to_string()),
            workflow_operation_id: Some("workflow-1".to_string()),
            cancel_requested: false,
            cancel_requested_at: None,
            confirmed_at: Some(now),
            started_at: Some(now),
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    /// Second-resolution instants, so a sqlite text round-trip compares equal.
    fn timestamp() -> DateTime<Utc> {
        DateTime::from_timestamp(1_756_600_000, 0).expect("a valid instant")
    }

    #[tokio::test]
    async fn an_operation_round_trips_through_the_0206_row() {
        let store = test_store().await;
        let operation = operation("op-1", LocationOperationState::Moving);
        store
            .create_location_operation(&operation, Some("{\"titles\":[]}"))
            .await
            .expect("the operation should persist");

        let loaded = store
            .get_location_operation("op-1")
            .await
            .expect("the read should succeed")
            .expect("the operation should exist");
        assert_eq!(loaded, operation);
        // Named explicitly: the outcome counters are the half of FR-091 that
        // used to read back as zero because 0206 had no columns for them.
        assert_eq!(loaded.counters.merges, 1);
        assert_eq!(loaded.counters.dedups, 2);
        assert_eq!(loaded.counters.renames, 3);
        assert_eq!(loaded.counters.no_ops, 4);
        assert_eq!(loaded.counters.unresolved, 5);
        assert_eq!(
            store
                .get_location_operation_plan_json("op-1")
                .await
                .expect("the plan read should succeed")
                .as_deref(),
            Some("{\"titles\":[]}")
        );
    }

    #[tokio::test]
    async fn only_non_terminal_operations_are_resumable() {
        let store = test_store().await;
        for (id, state) in [
            ("op-active", LocationOperationState::Moving),
            ("op-queued", LocationOperationState::Queued),
            ("op-done", LocationOperationState::Completed),
            ("op-failed", LocationOperationState::Failed),
        ] {
            store
                .create_location_operation(&operation(id, state), None)
                .await
                .expect("the operation should persist");
        }

        let active: Vec<String> = store
            .list_active_location_operations()
            .await
            .expect("the read should succeed")
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        assert_eq!(
            active,
            vec!["op-active".to_string(), "op-queued".to_string()]
        );
    }

    #[tokio::test]
    async fn progress_updates_keep_the_first_start_and_land_the_completion() {
        let store = test_store().await;
        let mut queued = operation("op-1", LocationOperationState::Queued);
        queued.started_at = None;
        queued.detail = None;
        store
            .create_location_operation(&queued, None)
            .await
            .expect("the operation should persist");

        let first_start = timestamp();
        store
            .update_location_operation_progress(&LocationOperationProgress {
                operation_id: "op-1".to_string(),
                state: LocationOperationState::Moving,
                counters: LocationOperationCounters {
                    titles_total: 2,
                    files_total: 5,
                    bytes_total: 500,
                    ..LocationOperationCounters::default()
                },
                verification_fallback_count: 0,
                detail: None,
                clear_detail: true,
                started_at: Some(first_start),
                completed_at: None,
            })
            .await
            .expect("the progress write should succeed");

        store
            .update_location_operation_progress(&LocationOperationProgress {
                operation_id: "op-1".to_string(),
                state: LocationOperationState::CompletedWithWarnings,
                counters: LocationOperationCounters {
                    titles_total: 2,
                    titles_processed: 2,
                    titles_blocked: 0,
                    files_total: 5,
                    files_processed: 5,
                    bytes_total: 500,
                    bytes_processed: 500,
                    merges: 1,
                    dedups: 2,
                    renames: 3,
                    no_ops: 4,
                    unresolved: 0,
                },
                verification_fallback_count: 1,
                detail: Some("one file fell back to the quick floor".to_string()),
                clear_detail: false,
                // A later run must not rewrite when the operation began.
                started_at: Some(first_start + chrono::Duration::seconds(60)),
                completed_at: Some(first_start + chrono::Duration::seconds(90)),
            })
            .await
            .expect("the progress write should succeed");

        let loaded = store
            .get_location_operation("op-1")
            .await
            .expect("the read should succeed")
            .expect("the operation should exist");
        assert_eq!(loaded.state, LocationOperationState::CompletedWithWarnings);
        assert_eq!(loaded.started_at, Some(first_start));
        assert_eq!(
            loaded.completed_at,
            Some(first_start + chrono::Duration::seconds(90))
        );
        assert_eq!(loaded.counters.bytes_processed, 500);
        // A progress write owns the outcome counters too: the runner recomputes
        // them from the plan, so the row is replaced, never accumulated.
        assert_eq!(loaded.counters.merges, 1);
        assert_eq!(loaded.counters.dedups, 2);
        assert_eq!(loaded.counters.renames, 3);
        assert_eq!(loaded.counters.no_ops, 4);
        assert_eq!(
            loaded.counters.unresolved, 0,
            "a counter that dropped to zero is written, not left at its old value"
        );
        assert_eq!(loaded.verification_fallback_count, 1);
        assert_eq!(
            loaded.detail.as_deref(),
            Some("one file fell back to the quick floor")
        );

        assert!(
            matches!(
                store
                    .update_location_operation_progress(&LocationOperationProgress {
                        operation_id: "missing".to_string(),
                        state: LocationOperationState::Moving,
                        counters: LocationOperationCounters::default(),
                        verification_fallback_count: 0,
                        detail: None,
                        clear_detail: true,
                        started_at: None,
                        completed_at: None,
                    })
                    .await,
                Err(AppError::NotFound(_))
            ),
            "a progress write for an unknown operation must not silently do nothing"
        );
    }

    #[tokio::test]
    async fn a_cancel_is_recorded_until_the_operation_is_terminal() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        assert!(
            !store
                .location_operation_cancel_requested("op-1")
                .await
                .expect("the read should succeed")
        );
        assert!(
            store
                .request_location_operation_cancel("op-1")
                .await
                .expect("the cancel should succeed")
        );
        assert!(
            store
                .location_operation_cancel_requested("op-1")
                .await
                .expect("the read should succeed")
        );

        store
            .create_location_operation(&operation("op-done", LocationOperationState::Completed), None)
            .await
            .expect("the operation should persist");
        assert!(
            !store
                .request_location_operation_cancel("op-done")
                .await
                .expect("the cancel should succeed"),
            "a finished operation cannot be cancelled"
        );
    }

    /// FR-091: a resumed operation reports through a new Activity run, so the
    /// row has to be repointable after it was created.
    #[tokio::test]
    async fn an_operation_can_be_repointed_at_the_run_for_its_latest_execution() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        store
            .set_location_operation_job_run("op-1", "job-2")
            .await
            .expect("the repoint should succeed");

        let stored = store
            .get_location_operation("op-1")
            .await
            .expect("the read should succeed")
            .expect("the operation should exist");
        assert_eq!(stored.job_run_id.as_deref(), Some("job-2"));

        assert!(
            store
                .set_location_operation_job_run("op-missing", "job-3")
                .await
                .is_err(),
            "an operation that is not there cannot be repointed"
        );
    }

    #[tokio::test]
    async fn a_checkpoint_round_trips_and_upserts_in_place() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let started = timestamp();
        let mut checkpoint = TitleCheckpoint {
            operation_id: "op-1".to_string(),
            title_id: "title-1".to_string(),
            sequence: 3,
            state: TitleCheckpointState::Moving,
            classification: Some(TitleLocationClass::CrossLibraryTransfer),
            placement: TitleCheckpointPlacement {
                source_library_id: Some("library-1".to_string()),
                source_root_id: Some("root-1".to_string()),
                source_folder_path: Some("/media/movies/Arrival (2016)".to_string()),
                destination_library_id: Some("library-2".to_string()),
                destination_root_id: Some("root-2".to_string()),
                destination_folder_path: Some("/archive/movies/Arrival (2016)".to_string()),
                merged_into_title_id: Some("title-99".to_string()),
            },
            files_total: 3,
            files_verified: 1,
            bytes_total: 300,
            bytes_verified: 100,
            detail: None,
            started_at: Some(started),
            updated_at: started,
            completed_at: None,
        };
        store
            .upsert_location_title_checkpoint(&checkpoint)
            .await
            .expect("the checkpoint should persist");

        let settled_at = started + chrono::Duration::seconds(30);
        checkpoint.state = TitleCheckpointState::CompletedWithWarnings;
        checkpoint.files_verified = 3;
        checkpoint.bytes_verified = 300;
        checkpoint.detail = Some("one companion asset was renamed".to_string());
        checkpoint.updated_at = settled_at;
        checkpoint.completed_at = Some(settled_at);
        store
            .upsert_location_title_checkpoint(&checkpoint)
            .await
            .expect("the checkpoint should update in place");

        let loaded = store
            .list_location_title_checkpoints("op-1")
            .await
            .expect("the read should succeed");
        assert_eq!(loaded.len(), 1, "the upsert must not create a second row");
        let loaded = &loaded[0];
        assert_eq!(loaded.state, TitleCheckpointState::CompletedWithWarnings);
        assert_eq!(loaded.files_verified, 3);
        assert_eq!(loaded.completed_at, Some(settled_at));
        assert_eq!(
            loaded.detail.as_deref(),
            Some("one companion asset was renamed")
        );
        assert_eq!(loaded.classification, checkpoint.classification);
        assert_eq!(loaded.placement, checkpoint.placement);
        // The row is created when the title starts, so its creation instant is
        // the title's start.
        assert_eq!(loaded.started_at, Some(started));
    }

    #[tokio::test]
    async fn a_blocked_checkpoint_keeps_its_reason() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let now = timestamp();
        store
            .upsert_location_title_checkpoint(&TitleCheckpoint {
                operation_id: "op-1".to_string(),
                title_id: "title-1".to_string(),
                sequence: 1,
                state: TitleCheckpointState::Blocked,
                classification: None,
                placement: TitleCheckpointPlacement::default(),
                files_total: 0,
                files_verified: 0,
                bytes_total: 0,
                bytes_verified: 0,
                detail: Some("an import is running".to_string()),
                started_at: Some(now),
                updated_at: now,
                completed_at: Some(now),
            })
            .await
            .expect("the checkpoint should persist");

        let loaded = store
            .list_location_title_checkpoints("op-1")
            .await
            .expect("the read should succeed");
        assert_eq!(loaded[0].detail.as_deref(), Some("an import is running"));
        assert_eq!(
            checkpoint_columns(&store, "title-1").await,
            (Some("an import is running".to_string()), None, None),
            "a blocked title's reason belongs in blocked_reason and nowhere else"
        );
    }

    /// `(blocked_reason, failure_reason, note)` exactly as 0206 holds them.
    async fn checkpoint_columns(
        store: &LocationOperationStore,
        title_id: &str,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT blocked_reason, failure_reason, note
               FROM location_operation_title_checkpoints WHERE title_id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
        .expect("the read should succeed")
        .expect("the checkpoint should exist");
        (
            row.opt_text("blocked_reason").expect("column"),
            row.opt_text("failure_reason").expect("column"),
            row.opt_text("note").expect("column"),
        )
    }

    #[tokio::test]
    async fn each_checkpoint_explanation_lands_in_its_own_column() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let now = timestamp();
        let checkpoint = |title_id: &str, state, detail: &str| TitleCheckpoint {
            operation_id: "op-1".to_string(),
            title_id: title_id.to_string(),
            sequence: 1,
            state,
            classification: None,
            placement: TitleCheckpointPlacement::default(),
            files_total: 0,
            files_verified: 0,
            bytes_total: 0,
            bytes_verified: 0,
            detail: Some(detail.to_string()),
            started_at: Some(now),
            updated_at: now,
            completed_at: Some(now),
        };

        for (title_id, state, detail) in [
            (
                "warned",
                TitleCheckpointState::CompletedWithWarnings,
                "one companion asset was renamed",
            ),
            (
                "failed",
                TitleCheckpointState::Failed,
                "the destination filesystem is full",
            ),
        ] {
            store
                .upsert_location_title_checkpoint(&checkpoint(title_id, state, detail))
                .await
                .expect("the checkpoint should persist");
        }

        assert_eq!(
            checkpoint_columns(&store, "warned").await,
            (
                None,
                None,
                Some("one companion asset was renamed".to_string())
            ),
            "a warning note is not a failure and must not squat in failure_reason"
        );
        assert_eq!(
            checkpoint_columns(&store, "failed").await,
            (
                None,
                Some("the destination filesystem is full".to_string()),
                None
            )
        );

        // Whichever column holds it, the model reads one detail back.
        let loaded = store
            .list_location_title_checkpoints("op-1")
            .await
            .expect("the read should succeed");
        let details: Vec<Option<&str>> = loaded
            .iter()
            .map(|checkpoint| checkpoint.detail.as_deref())
            .collect();
        assert_eq!(
            details,
            vec![
                Some("the destination filesystem is full"),
                Some("one companion asset was renamed"),
            ]
        );
    }

    #[tokio::test]
    async fn a_checkpoint_that_settles_clean_leaves_every_explanation_null() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let now = timestamp();
        store
            .upsert_location_title_checkpoint(&TitleCheckpoint {
                operation_id: "op-1".to_string(),
                title_id: "title-1".to_string(),
                sequence: 1,
                state: TitleCheckpointState::Completed,
                classification: None,
                placement: TitleCheckpointPlacement::default(),
                files_total: 1,
                files_verified: 1,
                bytes_total: 100,
                bytes_verified: 100,
                detail: None,
                started_at: Some(now),
                updated_at: now,
                completed_at: Some(now),
            })
            .await
            .expect("the checkpoint should persist");

        assert_eq!(checkpoint_columns(&store, "title-1").await, (None, None, None));
        assert_eq!(
            store
                .list_location_title_checkpoints("op-1")
                .await
                .expect("the read should succeed")[0]
                .detail,
            None,
            "the read stays tolerant of three NULL explanation columns"
        );
    }

    fn verification(destination: &str, outcome: FileVerificationOutcome) -> FileVerificationRecord {
        FileVerificationRecord {
            operation_id: "op-1".to_string(),
            title_id: "title-1".to_string(),
            media_file_id: Some("file-1".to_string()),
            source_path: "/media/movies/Arrival (2016)/Arrival.mkv".to_string(),
            destination_path: destination.to_string(),
            hashes: Some(StreamedContentHashes {
                size_bytes: 18_446_744_073_709_551_000,
                crc_algorithm: MoveCrcAlgorithm::Crc64Nvme,
                move_crc: 18_446_744_073_709_551_615,
                full_blake3: "abc123".to_string(),
            }),
            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
            outcome,
            detail: None,
            verified_at: timestamp(),
        }
    }

    #[tokio::test]
    async fn a_verification_round_trips_including_an_unsigned_crc() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let record = verification("/archive/Arrival.mkv", FileVerificationOutcome::Verified);
        store
            .record_location_file_verification(&record)
            .await
            .expect("the verification should persist");

        let loaded = store
            .list_location_file_verifications("op-1", Some("title-1"))
            .await
            .expect("the read should succeed");
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded[0]
                .hashes
                .as_ref()
                .expect("the hashes should round-trip")
                .move_crc,
            u64::MAX,
            "a CRC above i64::MAX must survive the text column"
        );
        assert_eq!(loaded[0].outcome, FileVerificationOutcome::Verified);
    }

    #[tokio::test]
    async fn a_fallback_reason_survives_and_re_recording_a_file_does_not_duplicate_it() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        let mut record = verification("/archive/Arrival.mkv", FileVerificationOutcome::Verified);
        record.depth = AppliedVerificationDepth::quick_fallback();
        record.detail = Some("a cache-bypassed read-back could not run".to_string());
        store
            .record_location_file_verification(&record)
            .await
            .expect("the verification should persist");
        store
            .record_location_file_verification(&record)
            .await
            .expect("re-recording the same destination should update in place");

        let loaded = store
            .list_location_file_verifications("op-1", None)
            .await
            .expect("the read should succeed");
        assert_eq!(loaded.len(), 1, "the 0206 unique index is honored");
        assert!(loaded[0].depth.fell_back);
        assert_eq!(
            loaded[0].detail.as_deref(),
            Some("a cache-bypassed read-back could not run")
        );
        assert_eq!(
            verification_columns(&store, "/archive/Arrival.mkv").await,
            (
                Some("a cache-bypassed read-back could not run".to_string()),
                None,
                None
            ),
            "a fallback reason stays in the column named for it"
        );
    }

    /// `(fallback_reason, failure_reason, detail)` exactly as 0206 holds them.
    async fn verification_columns(
        store: &LocationOperationStore,
        destination: &str,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT fallback_reason, failure_reason, detail
               FROM location_operation_verifications WHERE destination_path = {}",
            &[SqlArg::Text(destination.to_string())],
        )
        .await
        .expect("the read should succeed")
        .expect("the verification should exist");
        (
            row.opt_text("fallback_reason").expect("column"),
            row.opt_text("failure_reason").expect("column"),
            row.opt_text("detail").expect("column"),
        )
    }

    #[tokio::test]
    async fn each_verification_explanation_lands_in_its_own_column() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        // A destination proven at the depth that was asked for: its note is
        // neither a fallback nor a failure (FR-043).
        let mut proven = verification("/archive/proven.mkv", FileVerificationOutcome::Verified);
        proven.detail = Some("verified (full) against the streaming CRC".to_string());
        // A destination that did not match: a failure, and only a failure.
        let mut mismatched =
            verification("/archive/mismatch.mkv", FileVerificationOutcome::Mismatch);
        mismatched.detail = Some("the read-back CRC did not match".to_string());
        for record in [&proven, &mismatched] {
            store
                .record_location_file_verification(record)
                .await
                .expect("the verification should persist");
        }

        assert_eq!(
            verification_columns(&store, "/archive/proven.mkv").await,
            (
                None,
                None,
                Some("verified (full) against the streaming CRC".to_string())
            ),
            "a clean verification's note must not squat in failure_reason"
        );
        assert_eq!(
            verification_columns(&store, "/archive/mismatch.mkv").await,
            (None, Some("the read-back CRC did not match".to_string()), None)
        );

        let details: Vec<Option<String>> = store
            .list_location_file_verifications("op-1", None)
            .await
            .expect("the read should succeed")
            .into_iter()
            .map(|record| record.detail)
            .collect();
        assert_eq!(
            details,
            vec![
                Some("the read-back CRC did not match".to_string()),
                Some("verified (full) against the streaming CRC".to_string()),
            ],
            "whichever column holds it, one detail reads back"
        );
    }

    #[tokio::test]
    async fn only_verified_destinations_are_offered_to_a_resume() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");

        store
            .record_location_file_verification(&verification(
                "/archive/verified.mkv",
                FileVerificationOutcome::Verified,
            ))
            .await
            .expect("the verification should persist");
        store
            .record_location_file_verification(&verification(
                "/archive/mismatch.mkv",
                FileVerificationOutcome::Mismatch,
            ))
            .await
            .expect("the verification should persist");
        store
            .record_location_file_verification(&verification(
                "/archive/unavailable.mkv",
                FileVerificationOutcome::Unavailable,
            ))
            .await
            .expect("the verification should persist");

        let verified = store
            .verified_destination_paths("op-1", "title-1")
            .await
            .expect("the read should succeed");
        assert_eq!(
            verified,
            BTreeSet::from(["/archive/verified.mkv".to_string()]),
            "a resume may only skip files that actually verified"
        );
    }

    #[tokio::test]
    async fn ownership_is_exclusive_until_it_is_released() {
        let store = test_store().await;
        for id in ["op-1", "op-2"] {
            store
                .create_location_operation(&operation(id, LocationOperationState::Moving), None)
                .await
                .expect("the operation should persist");
        }

        let entities = vec![
            OwnedEntity::Title("title-1".to_string()),
            OwnedEntity::Root("root-1".to_string()),
        ];
        assert_eq!(
            store
                .claim_location_operation_ownership("op-1", &entities)
                .await
                .expect("the claim should succeed"),
            LocationOwnershipOutcome::Claimed
        );
        // Re-claiming after a restart is idempotent for the same operation.
        assert_eq!(
            store
                .claim_location_operation_ownership("op-1", &entities)
                .await
                .expect("the re-claim should succeed"),
            LocationOwnershipOutcome::Claimed
        );

        let conflict = store
            .claim_location_operation_ownership(
                "op-2",
                &[
                    OwnedEntity::Title("title-1".to_string()),
                    OwnedEntity::Title("title-2".to_string()),
                ],
            )
            .await
            .expect("the claim should be answered");
        match conflict {
            LocationOwnershipOutcome::Conflict(conflicts) => {
                assert_eq!(conflicts.len(), 1);
                assert_eq!(conflicts[0].operation_id, "op-1");
                assert_eq!(conflicts[0].entity, OwnedEntity::Title("title-1".to_string()));
                assert_eq!(conflicts[0].action, GuardedAction::LocationOperation);
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert_eq!(
            store
                .location_ownership_holder(&OwnedEntity::Title("title-2".to_string()))
                .await
                .expect("the read should succeed"),
            None,
            "a refused claim must not leave a partial hold behind"
        );

        assert_eq!(
            store
                .location_ownership_holder(&OwnedEntity::Root("root-1".to_string()))
                .await
                .expect("the read should succeed")
                .as_deref(),
            Some("op-1")
        );
        assert_eq!(
            store
                .list_location_ownership_claims()
                .await
                .expect("the read should succeed")
                .len(),
            2
        );

        assert_eq!(
            store
                .release_location_operation_ownership("op-1")
                .await
                .expect("the release should succeed"),
            2
        );
        assert!(
            store
                .list_location_ownership_claims()
                .await
                .expect("the read should succeed")
                .is_empty()
        );
        assert_eq!(
            store
                .claim_location_operation_ownership("op-2", &entities)
                .await
                .expect("the claim should succeed"),
            LocationOwnershipOutcome::Claimed,
            "a released entity is claimable again"
        );
    }

    #[tokio::test]
    async fn a_deleted_operation_takes_its_rows_with_it() {
        let store = test_store().await;
        store
            .create_location_operation(&operation("op-1", LocationOperationState::Moving), None)
            .await
            .expect("the operation should persist");
        store
            .record_location_file_verification(&verification(
                "/archive/verified.mkv",
                FileVerificationOutcome::Verified,
            ))
            .await
            .expect("the verification should persist");

        // The checkpoint and verification tables cascade from the operation, so
        // an operation that is purged leaves no orphan history behind.
        SqlRuntime::execute_write(
            &store.datastore,
            "delete_location_operation",
            "DELETE FROM location_operations WHERE id = {}",
            vec![SqlArg::Text("op-1".to_string())],
        )
        .await
        .expect("the delete should succeed");

        assert!(
            store
                .list_location_file_verifications("op-1", None)
                .await
                .expect("the read should succeed")
                .is_empty()
        );
    }
}
