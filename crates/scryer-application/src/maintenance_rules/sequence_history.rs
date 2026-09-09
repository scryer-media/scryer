//! Durable, sequence-level action history.
//!
//! A schema-2 sequence is one operator-visible execution even when its steps
//! use no legacy action-run journal. Delete-files keeps its scoped deletion
//! journal for recovery, while this module records one summary row and copies
//! that journal's per-file outcome into the summary at completion.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use scryer_domain::{
    Id, LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MaintenanceActionJobReceipt, MaintenanceActionStepRun,
};
use serde::{Deserialize, Serialize};

use super::action_sequence::{ACTION_SEQUENCE_KIND, MaintenanceActionSequence};
use crate::{AppResult, AppUseCase};

const SEQUENCE_HISTORY_SCHEMA_VERSION: i32 = 1;
const SEQUENCE_HISTORY_SCAN_LIMIT: usize = 200;

/// Terminal state to persist for one operator-visible schema-2 sequence run.
/// Step rows remain the authority for resumability; this only creates one
/// durable history summary for the whole ordered execution.
#[derive(Clone, Debug)]
pub(super) enum MaintenanceSequenceHistoryOutcome {
    Completed { completed_step_ids: Vec<String> },
    Held(String),
    RetryingFailed,
    Failed { step_id: String },
    Canceled(String),
    Excluded,
    Error(String),
}

#[derive(Deserialize, Serialize)]
struct SequenceHistoryDetail {
    schema_version: i32,
    maintenance_sequence_history: bool,
    sequence_content_hash: String,
    /// The exact `updated_at` value written by the candidate lease CAS. It
    /// makes each reclaim a distinct history row and keeps a stale worker from
    /// finalizing its replacement's summary.
    execution_lease_updated_at: DateTime<Utc>,
    outcome: String,
    completed_step_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hold_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failing_step_id: Option<String>,
    steps: Vec<MaintenanceSequenceHistoryStepSnapshot>,
    /// The scoped-delete action row remains the recovery authority. Copy its
    /// outcome here so history has one sequence row with exact per-file
    /// evidence instead of presenting that internal journal as another action.
    deletion_outcomes: Vec<SequenceDeletionOutcome>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct MaintenanceSequenceHistoryStepSnapshot {
    pub(super) run: MaintenanceActionStepRun,
    pub(super) receipts: Vec<MaintenanceActionJobReceipt>,
}

#[derive(Deserialize, Serialize)]
struct SequenceDeletionOutcome {
    action_run_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hold_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The policy deletion journal includes every authorized file and its
    /// outcome. It is embedded only in the parent sequence history summary.
    detail: String,
}

/// True only for a v2 sequence's operator-visible action-run summary. A
/// DeleteFiles step may also own an internal `LifecycleActionRun` for scoped
/// crash recovery, but that row has no marker and must not render as a second
/// sequence in the action history.
pub(super) fn is_maintenance_sequence_history_run(run: &LifecycleActionRun) -> bool {
    run.action_kind == ACTION_SEQUENCE_KIND
        && serde_json::from_str::<serde_json::Value>(&run.detail)
            .ok()
            .and_then(|detail| {
                detail
                    .get("maintenance_sequence_history")
                    .and_then(serde_json::Value::as_bool)
            })
            == Some(true)
}

/// Reads the immutable per-step snapshot stored when a sequence history row
/// finished. It keeps an earlier held attempt from rendering later retry state.
pub(super) fn maintenance_sequence_history_steps(
    run: &LifecycleActionRun,
) -> Option<HashMap<String, MaintenanceSequenceHistoryStepSnapshot>> {
    if !is_maintenance_sequence_history_run(run) {
        return None;
    }
    serde_json::from_str::<SequenceHistoryDetail>(&run.detail)
        .ok()
        .map(|detail| {
            detail
                .steps
                .into_iter()
                .map(|step| (step.run.key.step_id.clone(), step))
                .collect()
        })
}

fn sequence_history_content_hash(run: &LifecycleActionRun) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&run.detail)
        .ok()
        .and_then(|detail| {
            detail
                .get("sequence_content_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

fn sequence_history_lease_updated_at(run: &LifecycleActionRun) -> Option<DateTime<Utc>> {
    serde_json::from_str::<serde_json::Value>(&run.detail)
        .ok()
        .and_then(|detail| {
            detail
                .get("execution_lease_updated_at")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
        })
}

fn scoped_deletion_action_run_id(run: &MaintenanceActionStepRun) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&run.target_identity_json)
        .ok()
        .and_then(|target| {
            target
                .get("scoped_deletion_action_run_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

impl AppUseCase {
    /// Creates the single action-history row for one claimed sequence attempt.
    /// The caller supplies the exact candidate-lease timestamp it just wrote;
    /// a lease reclaim creates a separate row instead of letting an old worker
    /// write the newer worker's history.
    pub(crate) async fn begin_maintenance_sequence_history(
        &self,
        candidate: &LifecycleCandidate,
        sequence: &MaintenanceActionSequence,
        execution_lease_updated_at: DateTime<Utc>,
    ) -> AppResult<LifecycleActionRun> {
        let sequence_content_hash = sequence
            .content_hash()
            .map_err(|error| crate::AppError::Validation(error.to_string()))?;
        let existing_runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_sequence_history_action_runs(
                None,
                Some(&candidate.id),
                SEQUENCE_HISTORY_SCAN_LIMIT,
            )
            .await?;
        if let Some(existing) = existing_runs.iter().find(|run| {
            run.match_generation == candidate.match_generation
                && run.revision_number == candidate.revision_number
                && run.status == LifecycleActionRunStatus::Running
                && is_maintenance_sequence_history_run(run)
                && sequence_history_content_hash(run).as_deref()
                    == Some(sequence_content_hash.as_str())
                && sequence_history_lease_updated_at(run) == Some(execution_lease_updated_at)
        }) {
            return Ok(existing.clone());
        }

        // A prior worker can crash after it has acquired the candidate lease
        // and created its summary. Owning a newer lease proves that worker can
        // no longer transition the candidate, so finish its summary as an
        // interruption before starting a distinct operator-visible attempt.
        // This changes history evidence only; it does not reserve or consume
        // another candidate action attempt.
        let interrupted_runs: Vec<_> = existing_runs
            .iter()
            .filter(|run| {
                run.match_generation == candidate.match_generation
                    && run.revision_number == candidate.revision_number
                    && run.status == LifecycleActionRunStatus::Running
                    && is_maintenance_sequence_history_run(run)
                    && sequence_history_lease_updated_at(run) != Some(execution_lease_updated_at)
            })
            .cloned()
            .collect();
        for mut interrupted in interrupted_runs {
            let Some(interrupted_lease_updated_at) =
                sequence_history_lease_updated_at(&interrupted)
            else {
                continue;
            };
            self.finish_maintenance_sequence_history(
                &mut interrupted,
                candidate,
                sequence,
                interrupted_lease_updated_at,
                MaintenanceSequenceHistoryOutcome::Held("interrupted".to_string()),
            )
            .await?;
        }

        let attempt = existing_runs
            .iter()
            .filter(|run| {
                run.match_generation == candidate.match_generation
                    && run.revision_number == candidate.revision_number
                    && is_maintenance_sequence_history_run(run)
            })
            .map(|run| run.attempt)
            .max()
            .unwrap_or_default()
            + 1;
        let now = Utc::now();
        let run_id = Id::new().0;
        let run = LifecycleActionRun {
            id: run_id.clone(),
            candidate_id: candidate.id.clone(),
            rule_set_id: candidate.rule_set_id.clone(),
            revision_number: candidate.revision_number,
            title_id: candidate.title_id.clone(),
            subject_kind: candidate.subject_kind.clone(),
            subject_id: candidate.subject_id.clone(),
            action_kind: ACTION_SEQUENCE_KIND.to_string(),
            match_generation: candidate.match_generation,
            idempotency_key: format!(
                "maintenance-sequence-history:{}:{}:{}:{}:{}",
                candidate.id,
                candidate.match_generation,
                candidate.revision_number,
                execution_lease_updated_at
                    .timestamp_nanos_opt()
                    .unwrap_or_default(),
                run_id,
            ),
            attempt,
            status: LifecycleActionRunStatus::Running,
            hold_reason: None,
            error: None,
            detail: serde_json::to_string(&SequenceHistoryDetail {
                schema_version: SEQUENCE_HISTORY_SCHEMA_VERSION,
                maintenance_sequence_history: true,
                sequence_content_hash,
                execution_lease_updated_at,
                outcome: "running".to_string(),
                completed_step_ids: Vec::new(),
                hold_reason: None,
                error: None,
                failing_step_id: None,
                steps: Vec::new(),
                deletion_outcomes: Vec::new(),
            })
            .map_err(|error| crate::AppError::Repository(error.to_string()))?,
            started_at: now,
            finished_at: None,
            created_at: now,
        };
        self.services
            .customization
            .maintenance_evaluation
            .start_action_run(&run)
            .await?;
        Ok(run)
    }

    /// Finishes a sequence history row after the caller has won its candidate
    /// lease CAS and all durable step state has been written. The embedded
    /// snapshot makes the sequence result readable even after the candidate
    /// later moves to a new generation.
    pub(super) async fn finish_maintenance_sequence_history(
        &self,
        run: &mut LifecycleActionRun,
        candidate: &LifecycleCandidate,
        sequence: &MaintenanceActionSequence,
        execution_lease_updated_at: DateTime<Utc>,
        outcome: MaintenanceSequenceHistoryOutcome,
    ) -> AppResult<()> {
        if !is_maintenance_sequence_history_run(run)
            || sequence_history_lease_updated_at(run) != Some(execution_lease_updated_at)
        {
            return Err(crate::AppError::Repository(
                "sequence history row does not belong to this execution lease".to_string(),
            ));
        }
        let step_runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await?;
        let steps_by_id: HashMap<_, _> = step_runs
            .iter()
            .map(|step_run| (step_run.key.step_id.as_str(), step_run))
            .collect();
        let step_keys: Vec<_> = step_runs
            .iter()
            .map(|step_run| step_run.key.clone())
            .collect();
        let receipts = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_job_receipts_for_steps(&step_keys)
            .await?;
        let mut receipts_by_step: HashMap<_, Vec<MaintenanceActionJobReceipt>> = HashMap::new();
        for receipt in receipts {
            receipts_by_step
                .entry(receipt.key.clone())
                .or_default()
                .push(receipt);
        }
        let steps: Vec<_> = sequence
            .steps
            .iter()
            .filter_map(|step| {
                steps_by_id.get(step.id.as_str()).map(|run| {
                    MaintenanceSequenceHistoryStepSnapshot {
                        run: (*run).clone(),
                        receipts: receipts_by_step.get(&run.key).cloned().unwrap_or_default(),
                    }
                })
            })
            .collect();
        let deletion_run_ids: HashSet<String> = step_runs
            .iter()
            .filter_map(scoped_deletion_action_run_id)
            .collect();
        let deletion_runs = if deletion_run_ids.is_empty() {
            Vec::new()
        } else {
            self.services
                .customization
                .maintenance_evaluation
                .list_action_runs(None, Some(&candidate.id), Some(SEQUENCE_HISTORY_SCAN_LIMIT))
                .await?
                .into_iter()
                .filter(|deletion_run| deletion_run_ids.contains(&deletion_run.id))
                .map(|deletion_run| SequenceDeletionOutcome {
                    action_run_id: deletion_run.id,
                    status: deletion_run.status.as_storage_str().to_string(),
                    hold_reason: deletion_run.hold_reason,
                    error: deletion_run.error,
                    detail: deletion_run.detail,
                })
                .collect()
        };
        let sequence_content_hash = sequence
            .content_hash()
            .map_err(|error| crate::AppError::Validation(error.to_string()))?;
        let (outcome, status, hold_reason, mut error, failing_step_id, completed_step_ids) =
            sequence_history_outcome_fields(outcome);
        if error.is_none() && matches!(status, LifecycleActionRunStatus::Failed) {
            error = failing_step_id
                .as_deref()
                .and_then(|step_id| {
                    steps
                        .iter()
                        .find(|step| step.run.key.step_id == step_id)
                        .and_then(|step| step.run.error.clone())
                })
                .or_else(|| steps.iter().rev().find_map(|step| step.run.error.clone()))
                .or_else(|| Some("sequence step failed".to_string()));
        }
        run.status = status;
        run.hold_reason = hold_reason.clone();
        run.error = error.clone();
        run.detail = serde_json::to_string(&SequenceHistoryDetail {
            schema_version: SEQUENCE_HISTORY_SCHEMA_VERSION,
            maintenance_sequence_history: true,
            sequence_content_hash,
            execution_lease_updated_at,
            outcome,
            completed_step_ids,
            hold_reason,
            error,
            failing_step_id,
            steps,
            deletion_outcomes: deletion_runs,
        })
        .map_err(|error| crate::AppError::Repository(error.to_string()))?;
        run.finished_at = Some(Utc::now());
        self.services
            .customization
            .maintenance_evaluation
            .finish_action_run(run)
            .await
    }
}

fn sequence_history_outcome_fields(
    outcome: MaintenanceSequenceHistoryOutcome,
) -> (
    String,
    LifecycleActionRunStatus,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<String>,
) {
    match outcome {
        MaintenanceSequenceHistoryOutcome::Completed { completed_step_ids } => (
            "succeeded".to_string(),
            LifecycleActionRunStatus::Succeeded,
            None,
            None,
            None,
            completed_step_ids,
        ),
        MaintenanceSequenceHistoryOutcome::Held(reason) => (
            "held".to_string(),
            LifecycleActionRunStatus::Held,
            Some(reason),
            None,
            None,
            Vec::new(),
        ),
        MaintenanceSequenceHistoryOutcome::RetryingFailed => (
            "retrying_failed".to_string(),
            LifecycleActionRunStatus::Failed,
            None,
            None,
            None,
            Vec::new(),
        ),
        MaintenanceSequenceHistoryOutcome::Failed { step_id } => (
            "failed".to_string(),
            LifecycleActionRunStatus::Failed,
            None,
            None,
            Some(step_id),
            Vec::new(),
        ),
        MaintenanceSequenceHistoryOutcome::Canceled(reason) => (
            "canceled".to_string(),
            LifecycleActionRunStatus::Held,
            Some(reason),
            None,
            None,
            Vec::new(),
        ),
        MaintenanceSequenceHistoryOutcome::Excluded => (
            "excluded".to_string(),
            LifecycleActionRunStatus::Held,
            Some(crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED.to_string()),
            None,
            None,
            Vec::new(),
        ),
        MaintenanceSequenceHistoryOutcome::Error(error) => (
            "failed".to_string(),
            LifecycleActionRunStatus::Failed,
            None,
            Some(error),
            None,
            Vec::new(),
        ),
    }
}
