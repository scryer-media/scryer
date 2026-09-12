//! Durable orchestration for v2 maintenance action sequences.
//!
//! This module deliberately sits beside the v1 action handler. A sequence
//! keeps one durable row per stable step id, and never reinterprets the legacy
//! action-run journal as sequence completion evidence.

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Duration, Utc};
use scryer_domain::{
    Id, LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MAINTENANCE_ACTION_JOB_RECEIPT_SCHEMA_VERSION, MaintenanceActionJobReceipt,
    MaintenanceActionJobReceiptState, MaintenanceActionStepKey, MaintenanceActionStepRun,
    MaintenanceActionStepState, MaintenanceCandidateState, MaintenanceSequenceTerminalMembership,
    MaintenanceSequenceTerminalOutcome, Title,
};
use scryer_rules::maintenance::MaintenanceRulesEvaluator;

use super::action_execution::{
    ActionResult, CandidateOutcome, EXECUTION_LEASE_STALE_AFTER_MINUTES,
    EXECUTOR_DUE_EXPECTED_STATES, MaintenanceExecutionContext, MaintenanceSequencePassBudget,
    SafetyDecision, execution_reason,
};
use super::action_sequence::{
    MaintenanceActionCompletionPolicy, MaintenanceActionSequence, MaintenanceActionStep,
    MaintenanceActionStepKind, MaintenanceActionStepParameters,
};
use super::facts::MaintenanceLibraryRef;
use super::sequence_history::MaintenanceSequenceHistoryOutcome;
use super::service::MaintenanceRuleSetDetail;
use crate::ports::MaintenanceActionStepClaim;
#[cfg(test)]
use crate::ports::{MaintenanceActionJobReceiptClaim, MaintenanceActionJobReceiptTransition};
use crate::{AcquisitionSearchRequest, AppError, AppResult, AppUseCase, WantedKind};

/// A sequence stops at the first unfinished step. A hold releases the
/// candidate lease; a failed step remains retryable until its own attempt
/// budget is exhausted.
enum SequenceOutcome {
    Completed,
    Held(&'static str),
    RetryingFailed,
    Failed { step_id: String },
    Canceled(&'static str),
    Excluded,
    LeaseLost,
}

enum SequenceStepResult {
    Finished(MaintenanceActionStepState),
    Held(&'static str),
    Canceled(&'static str),
    Excluded,
}

/// Completion policy belongs to the registered executor adapter, never to the
/// user-authored action payload. Search is accepted once its atomic workflow
/// record exists; a future durable adapter advances only after its receipt is
/// reconciled completed.
/// Internal adapter boundary shared by immediate and durable job operations.
/// It deliberately carries the durable receipt instead of an in-memory job
/// handle, so a resumed worker always reconciles the same persisted boundary.
enum SequenceJobDispatchResult {
    Immediate(SequenceStepResult),
    DurableJob {
        receipt: MaintenanceActionJobReceipt,
        completion_policy: MaintenanceActionCompletionPolicy,
    },
}

#[derive(serde::Deserialize, serde::Serialize)]
struct SequenceStepIntent {
    step: MaintenanceActionStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    search_request: Option<SequenceSearchRequest>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct SequenceSearchRequest {
    wanted_kind: String,
    title_id: String,
}

/// Borrowed inputs shared by every operation in one claimed sequence step.
/// Keeping the context intact also makes the deletion adapter receive exactly
/// the already rechecked title and fact snapshot.
struct SequenceStepExecutionContext<'a> {
    detail: &'a MaintenanceRuleSetDetail,
    sequence: &'a MaintenanceActionSequence,
    step_index: usize,
    completed: &'a HashMap<String, MaintenanceActionStepRun>,
    candidate: &'a LifecycleCandidate,
    title: &'a Title,
    input: &'a scryer_rules::maintenance::MaintenanceInput,
    evaluator: &'a mut MaintenanceRulesEvaluator,
    libraries: &'a HashMap<String, MaintenanceLibraryRef>,
    tag_conflicts: &'a Option<std::collections::HashSet<String>>,
}

struct SequenceStepRunContext<'a> {
    detail: &'a MaintenanceRuleSetDetail,
    candidate: &'a LifecycleCandidate,
    sequence_hash: &'a str,
    title: &'a Title,
}

/// Shared pass inputs for one sequence candidate. Keeping this together makes
/// the step budget part of the same execution boundary as fresh safety facts
/// and title-scoped coordination, rather than a side channel each adapter can
/// accidentally bypass.
pub(super) struct SequenceCandidateExecutionContext<'a> {
    pub(super) detail: &'a MaintenanceRuleSetDetail,
    pub(super) sequence: &'a MaintenanceActionSequence,
    pub(super) evaluator: &'a mut MaintenanceRulesEvaluator,
    pub(super) libraries: &'a HashMap<String, MaintenanceLibraryRef>,
    pub(super) tag_conflicts: &'a Option<std::collections::HashSet<String>>,
    pub(super) budget: &'a mut MaintenanceSequencePassBudget,
}

/// A private completed-job seam used only by sequence executor regressions.
/// Production never registers it; the closed action catalog remains unchanged.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TestCompletedSequenceJobState {
    Waiting,
    Completed,
    Failed,
    Unknown,
}

#[cfg(test)]
pub(crate) type TestCompletedSequenceJobMap =
    HashMap<MaintenanceActionStepKey, TestCompletedSequenceJobState>;

#[cfg(test)]
static TEST_COMPLETED_SEQUENCE_JOBS: OnceLock<Mutex<Option<TestCompletedSequenceJobMap>>> =
    OnceLock::new();
#[cfg(test)]
static TEST_COMPLETED_SEQUENCE_JOB_SERIALIZER: OnceLock<Arc<tokio::sync::Semaphore>> =
    OnceLock::new();

#[cfg(test)]
fn test_completed_sequence_job_slot() -> &'static Mutex<Option<TestCompletedSequenceJobMap>> {
    TEST_COMPLETED_SEQUENCE_JOBS.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
/// Serializes test-only completed-job adapter registration across parallel
/// regression cases. Its state is deliberately outside production services.
pub(crate) struct TestCompletedSequenceJobAdapterGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
impl Drop for TestCompletedSequenceJobAdapterGuard {
    fn drop(&mut self) {
        *test_completed_sequence_job_slot()
            .lock()
            .expect("test completed sequence job adapter lock") = None;
    }
}

#[cfg(test)]
pub(crate) async fn install_test_completed_sequence_job_adapter(
    jobs: TestCompletedSequenceJobMap,
) -> TestCompletedSequenceJobAdapterGuard {
    let permit = TEST_COMPLETED_SEQUENCE_JOB_SERIALIZER
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
        .acquire_owned()
        .await
        .expect("test completed sequence job adapter semaphore");
    *test_completed_sequence_job_slot()
        .lock()
        .expect("test completed sequence job adapter lock") = Some(jobs);
    TestCompletedSequenceJobAdapterGuard { _permit: permit }
}

#[cfg(test)]
pub(crate) fn set_test_completed_sequence_job_state(
    key: &MaintenanceActionStepKey,
    state: TestCompletedSequenceJobState,
) {
    if let Some(jobs) = test_completed_sequence_job_slot()
        .lock()
        .expect("test completed sequence job adapter lock")
        .as_mut()
    {
        jobs.insert(key.clone(), state);
    }
}

#[cfg(test)]
fn test_completed_sequence_job_state(
    key: &MaintenanceActionStepKey,
) -> Option<TestCompletedSequenceJobState> {
    test_completed_sequence_job_slot()
        .lock()
        .expect("test completed sequence job adapter lock")
        .as_ref()
        .and_then(|jobs| jobs.get(key).copied())
}

impl AppUseCase {
    pub(super) async fn execute_one_maintenance_sequence_candidate(
        &self,
        candidate: LifecycleCandidate,
        context: SequenceCandidateExecutionContext<'_>,
    ) -> AppResult<CandidateOutcome> {
        let SequenceCandidateExecutionContext {
            detail,
            sequence,
            evaluator,
            libraries,
            tag_conflicts,
            budget,
        } = context;
        let now = Utc::now();
        let candidates = &self.services.customization.maintenance_evaluation;
        let reclaiming = candidate.state == MaintenanceCandidateState::Executing;
        if !reclaiming
            && candidate.state != MaintenanceCandidateState::Due
            && !candidates
                .transition_candidate_state(
                    &candidate.id,
                    MaintenanceCandidateState::Due,
                    "due",
                    EXECUTOR_DUE_EXPECTED_STATES,
                    now,
                )
                .await?
        {
            return Ok(CandidateOutcome::LeaseLost);
        }
        if !candidates
            .lease_candidate_for_execution(
                &candidate.id,
                now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
                now,
            )
            .await?
        {
            return Ok(CandidateOutcome::LeaseLost);
        }

        let mut history = match self
            .begin_maintenance_sequence_history(&candidate, sequence, now)
            .await
        {
            Ok(history) => history,
            Err(error) => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Blocked,
                        execution_reason::UNKNOWN_AT_EXECUTION,
                        Utc::now(),
                    )
                    .await?
                {
                    return Err(error);
                }
                return Ok(CandidateOutcome::LeaseLost);
            }
        };
        let outcome = match self
            .run_maintenance_action_sequence(
                &candidate,
                SequenceCandidateExecutionContext {
                    detail,
                    sequence,
                    evaluator,
                    libraries,
                    tag_conflicts,
                    budget,
                },
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Blocked,
                        execution_reason::UNKNOWN_AT_EXECUTION,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Error(error.to_string()),
                    )
                    .await?;
                    return Err(error);
                }
                return Ok(CandidateOutcome::LeaseLost);
            }
        };
        match outcome {
            SequenceOutcome::Completed => {
                // Terminal membership is based on the durable step rows, not
                // the in-memory progress map that led us here. A process may
                // have crossed a persistence boundary between a last step and
                // this candidate-level commit.
                let durable_success_ids: HashSet<_> = candidates
                    .list_action_steps(
                        &candidate.id,
                        candidate.match_generation,
                        candidate.revision_number,
                    )
                    .await?
                    .into_iter()
                    .filter(|run| run.state.is_completion_success())
                    .map(|run| run.key.step_id)
                    .collect();
                let completed_step_ids: Vec<String> = sequence
                    .steps
                    .iter()
                    .filter(|step| durable_success_ids.contains(&step.id))
                    .map(|step| step.id.clone())
                    .collect();
                let membership = sequence_terminal_membership(
                    sequence,
                    &candidate,
                    MaintenanceSequenceTerminalOutcome::Succeeded,
                    completed_step_ids.clone(),
                    None,
                )?;
                if candidates
                    .finish_sequence_terminal_membership_and_candidate(
                        &membership,
                        MaintenanceCandidateState::Executing,
                        now,
                        MaintenanceCandidateState::Succeeded,
                        execution_reason::ACTION_SUCCEEDED,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Completed { completed_step_ids },
                    )
                    .await?;
                    Ok(CandidateOutcome::Executed)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::Held(reason) => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Blocked,
                        reason,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Held(reason.to_string()),
                    )
                    .await?;
                    Ok(CandidateOutcome::Held)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::RetryingFailed => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Blocked,
                        execution_reason::RETRY_PENDING,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::RetryingFailed,
                    )
                    .await?;
                    // The candidate remains retryable, while the pass report
                    // still counts this failed step for the high-risk circuit
                    // breaker.
                    Ok(CandidateOutcome::Failed)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::Canceled(reason) => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Canceled,
                        reason,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Canceled(reason.to_string()),
                    )
                    .await?;
                    Ok(CandidateOutcome::Canceled)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::Excluded => {
                if candidates
                    .finish_leased_candidate(
                        &candidate.id,
                        now,
                        MaintenanceCandidateState::Excluded,
                        crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Excluded,
                    )
                    .await?;
                    Ok(CandidateOutcome::Canceled)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::Failed { step_id } => {
                let persisted_step_ids: HashSet<_> = self
                    .services
                    .customization
                    .maintenance_evaluation
                    .list_action_steps(
                        &candidate.id,
                        candidate.match_generation,
                        candidate.revision_number,
                    )
                    .await?
                    .into_iter()
                    .filter(|run| run.state.is_completion_success() || run.key.step_id == step_id)
                    .map(|run| run.key.step_id)
                    .collect();
                let completed_step_ids = sequence
                    .steps
                    .iter()
                    .filter(|step| persisted_step_ids.contains(&step.id))
                    .map(|step| step.id.clone())
                    .collect();
                let membership = sequence_terminal_membership(
                    sequence,
                    &candidate,
                    MaintenanceSequenceTerminalOutcome::Failed,
                    completed_step_ids,
                    Some(step_id.clone()),
                )?;
                if candidates
                    .finish_sequence_terminal_membership_and_candidate(
                        &membership,
                        MaintenanceCandidateState::Executing,
                        now,
                        MaintenanceCandidateState::Failed,
                        execution_reason::ACTION_FAILED,
                        Utc::now(),
                    )
                    .await?
                {
                    self.finish_maintenance_sequence_history(
                        &mut history,
                        &candidate,
                        sequence,
                        now,
                        MaintenanceSequenceHistoryOutcome::Failed { step_id },
                    )
                    .await?;
                    Ok(CandidateOutcome::Failed)
                } else {
                    Ok(CandidateOutcome::LeaseLost)
                }
            }
            SequenceOutcome::LeaseLost => Ok(CandidateOutcome::LeaseLost),
        }
    }

    async fn run_maintenance_action_sequence(
        &self,
        candidate: &LifecycleCandidate,
        context: SequenceCandidateExecutionContext<'_>,
    ) -> AppResult<SequenceOutcome> {
        let SequenceCandidateExecutionContext {
            detail,
            sequence,
            evaluator,
            libraries,
            tag_conflicts,
            budget,
        } = context;
        let sequence_hash = sequence
            .content_hash()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let persisted = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await?;
        let mut completed = HashMap::new();
        for run in persisted {
            let Some(step) = sequence
                .steps
                .iter()
                .find(|step| step.id == run.key.step_id)
            else {
                return Ok(SequenceOutcome::Held(execution_reason::RULE_NOT_ELIGIBLE));
            };
            if !persisted_sequence_step_is_compatible(&run, candidate, &sequence_hash, step) {
                return Ok(SequenceOutcome::Held(execution_reason::RULE_NOT_ELIGIBLE));
            }
            completed.insert(run.key.step_id.clone(), run);
        }

        for (index, step) in sequence.steps.iter().enumerate() {
            if sequence.steps[..index].iter().any(|prior| {
                !completed
                    .get(&prior.id)
                    .is_some_and(|run| run.state.is_completion_success())
            }) {
                return Ok(SequenceOutcome::Held(execution_reason::RULE_NOT_ELIGIBLE));
            }
            if completed
                .get(&step.id)
                .is_some_and(|run| run.state.is_completion_success())
            {
                continue;
            }
            if let Some(exhausted) = completed.get(&step.id)
                && exhausted.state == MaintenanceActionStepState::Failed
                && exhausted.attempt >= super::action_execution::MAINTENANCE_MAX_ACTION_ATTEMPTS
            {
                // A worker may have written the third failed attempt and died
                // before it could write the candidate's terminal membership.
                // Recover that durable fact before claiming an impossible
                // fourth adapter dispatch.
                return Ok(SequenceOutcome::Failed {
                    step_id: step.id.clone(),
                });
            }

            // The terminal deletion can remove its catalog target before a
            // crash reaches `finish_action_step`. Its immutable target record
            // is the only authority for treating that absence as success; a
            // generic missing-title safety result would otherwise erase the
            // exact in-flight recovery path.
            if step.kind == MaintenanceActionStepKind::DeleteTitleAndFiles
                && let Some(existing) = completed.get(&step.id).cloned()
                && existing.state == MaintenanceActionStepState::Running
            {
                match self
                    .services
                    .catalog
                    .titles
                    .get_by_id(&candidate.title_id)
                    .await
                {
                    Ok(None) if terminal_title_absence_is_proven(&existing) => {
                        let lease_id = Id::new().0;
                        match self
                            .services
                            .customization
                            .maintenance_evaluation
                            .claim_action_step(
                                &existing,
                                Utc::now() - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
                                &lease_id,
                                Utc::now(),
                            )
                            .await?
                        {
                            MaintenanceActionStepClaim::Claimed(mut recovered) => {
                                recovered.state = MaintenanceActionStepState::AlreadySatisfied;
                                recovered.finished_at = Some(Utc::now());
                                recovered.updated_at = Utc::now();
                                if !self
                                    .services
                                    .customization
                                    .maintenance_evaluation
                                    .finish_action_step(&recovered, &lease_id)
                                    .await?
                                {
                                    return Ok(SequenceOutcome::LeaseLost);
                                }
                                completed.insert(step.id.clone(), recovered);
                                continue;
                            }
                            MaintenanceActionStepClaim::Completed(existing) => {
                                if !persisted_sequence_step_is_compatible(
                                    &existing,
                                    candidate,
                                    &sequence_hash,
                                    step,
                                ) {
                                    return Ok(SequenceOutcome::Held(
                                        execution_reason::RULE_NOT_ELIGIBLE,
                                    ));
                                }
                                completed.insert(step.id.clone(), existing);
                                continue;
                            }
                            MaintenanceActionStepClaim::Busy => {
                                return Ok(SequenceOutcome::Held("step_busy"));
                            }
                        }
                    }
                    Ok(None) => return Ok(SequenceOutcome::Held(execution_reason::TITLE_MISSING)),
                    Ok(Some(_)) => {}
                    Err(_) => {
                        return Ok(SequenceOutcome::Held(
                            execution_reason::UNKNOWN_AT_EXECUTION,
                        ));
                    }
                }
            }

            // For non-terminal catalog mutations, a running checkpoint whose
            // exact postcondition now holds is recovered without replaying the
            // write. The same applies to a failed or held checkpoint: an
            // adapter may have completed its mutation before persistence of
            // its result failed. Keeping the original provenance is necessary
            // for later sequence baseline attribution and conditional Search.
            // If a postcondition does not hold, only an unchanged exact
            // before-state may safely replay the idempotent mutation.
            if let Some(existing) = completed.get(&step.id).cloned()
                && matches!(
                    existing.state,
                    MaintenanceActionStepState::Running
                        | MaintenanceActionStepState::Failed
                        | MaintenanceActionStepState::Held
                )
                && !matches!(
                    step.kind,
                    MaintenanceActionStepKind::Search
                        | MaintenanceActionStepKind::DeleteFiles
                        | MaintenanceActionStepKind::DeleteTitleAndFiles
                )
            {
                let title = match self
                    .services
                    .catalog
                    .titles
                    .get_by_id(&candidate.title_id)
                    .await
                {
                    Ok(Some(title)) => title,
                    Ok(None) => return Ok(SequenceOutcome::Held(execution_reason::TITLE_MISSING)),
                    Err(_) => {
                        return Ok(SequenceOutcome::Held(
                            execution_reason::UNKNOWN_AT_EXECUTION,
                        ));
                    }
                };
                let postcondition_holds = match self
                    .sequence_step_postcondition_holds(candidate, step, &existing, &title)
                    .await
                {
                    Ok(holds) => holds,
                    Err(_) => {
                        return Ok(SequenceOutcome::Held(
                            execution_reason::UNKNOWN_AT_EXECUTION,
                        ));
                    }
                };
                if postcondition_holds {
                    let lease_id = Id::new().0;
                    let mut recovery_claim = existing.clone();
                    recovery_claim.state = MaintenanceActionStepState::Running;
                    if existing.state == MaintenanceActionStepState::Failed {
                        // The repository requires a new ordinal to recover a
                        // failed attempt. The exhausted case returned above,
                        // so this cannot create an attempt beyond the cap.
                        recovery_claim.attempt += 1;
                    }
                    match self
                        .services
                        .customization
                        .maintenance_evaluation
                        .claim_action_step(
                            &recovery_claim,
                            Utc::now() - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
                            &lease_id,
                            Utc::now(),
                        )
                        .await?
                    {
                        MaintenanceActionStepClaim::Claimed(mut recovered) => {
                            // The checkpoint proves the intended postcondition,
                            // but a crash cannot prove this worker performed the
                            // write rather than an external actor. Preserve that
                            // distinction in the durable outcome.
                            recovered.state = MaintenanceActionStepState::AlreadySatisfied;
                            recovered.finished_at = Some(Utc::now());
                            recovered.updated_at = Utc::now();
                            if !self
                                .services
                                .customization
                                .maintenance_evaluation
                                .finish_action_step(&recovered, &lease_id)
                                .await?
                            {
                                return Ok(SequenceOutcome::LeaseLost);
                            }
                            completed.insert(step.id.clone(), recovered);
                            continue;
                        }
                        MaintenanceActionStepClaim::Completed(existing) => {
                            completed.insert(step.id.clone(), existing);
                            continue;
                        }
                        MaintenanceActionStepClaim::Busy => {
                            return Ok(SequenceOutcome::Held("step_busy"));
                        }
                    }
                }
                if !sequence_step_before_state_holds(candidate, step, &existing, &title) {
                    return Ok(SequenceOutcome::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
            }

            let step_is_high_risk = step.descriptor().risk_class
                == crate::maintenance_rules::MaintenanceRiskClass::High;
            if let Some(reason) = budget.hold_reason_before_dispatch(step_is_high_risk) {
                return Ok(SequenceOutcome::Held(reason));
            }

            let decision = self
                .maintenance_execution_safety_checks(
                    detail,
                    evaluator,
                    candidate,
                    libraries,
                    tag_conflicts,
                )
                .await;
            let (title, input) = match decision {
                SafetyDecision::Proceed(title, input) => (title, input),
                SafetyDecision::Hold(reason) => return Ok(SequenceOutcome::Held(reason)),
                SafetyDecision::Cancel(reason) => return Ok(SequenceOutcome::Canceled(reason)),
                SafetyDecision::Excluded => return Ok(SequenceOutcome::Excluded),
            };

            let lease_id = Id::new().0;
            let key = MaintenanceActionStepKey {
                candidate_id: candidate.id.clone(),
                match_generation: candidate.match_generation,
                revision_number: candidate.revision_number,
                step_id: step.id.clone(),
            };
            let attempt = completed.get(&step.id).map_or(1, |run| {
                if run.state == MaintenanceActionStepState::Failed {
                    run.attempt + 1
                } else {
                    run.attempt
                }
            });
            let mut run = sequence_step_run(
                key,
                SequenceStepRunContext {
                    detail,
                    candidate,
                    sequence_hash: &sequence_hash,
                    title: &title,
                },
                step,
                attempt,
                &lease_id,
            )?;
            let claim = self
                .services
                .customization
                .maintenance_evaluation
                .claim_action_step(
                    &run,
                    Utc::now() - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
                    &lease_id,
                    Utc::now(),
                )
                .await?;
            match claim {
                MaintenanceActionStepClaim::Completed(existing) => {
                    if !persisted_sequence_step_is_compatible(
                        &existing,
                        candidate,
                        &sequence_hash,
                        step,
                    ) {
                        return Ok(SequenceOutcome::Held(execution_reason::RULE_NOT_ELIGIBLE));
                    }
                    completed.insert(existing.key.step_id.clone(), existing);
                    continue;
                }
                MaintenanceActionStepClaim::Busy => return Ok(SequenceOutcome::Held("step_busy")),
                MaintenanceActionStepClaim::Claimed(claimed) => run = claimed,
            }

            let result = if step.kind == MaintenanceActionStepKind::Search {
                budget.record_dispatch(step_is_high_risk);
                self.execute_sequence_step(
                    SequenceStepExecutionContext {
                        detail,
                        sequence,
                        step_index: index,
                        completed: &completed,
                        candidate,
                        title: &title,
                        input: &input,
                        evaluator,
                        libraries,
                        tag_conflicts,
                    },
                    step,
                    &mut run,
                )
                .await
            } else {
                match self
                    .runtime
                    .jobs
                    .interactive_operation_guards
                    .try_acquire(&format!("maintenance-title:{}", candidate.title_id))
                    .await
                {
                    Some(_title_guard) => {
                        // The initial safety pass makes it safe to claim the
                        // durable step. Recheck while the title-wide mutation
                        // guard is held so a legacy or another v2 operation
                        // cannot change its target between observation and
                        // this adapter call.
                        match self
                            .maintenance_execution_safety_checks(
                                detail,
                                evaluator,
                                candidate,
                                libraries,
                                tag_conflicts,
                            )
                            .await
                        {
                            SafetyDecision::Proceed(guarded_title, guarded_input) => {
                                budget.record_dispatch(step_is_high_risk);
                                self.execute_sequence_step(
                                    SequenceStepExecutionContext {
                                        detail,
                                        sequence,
                                        step_index: index,
                                        completed: &completed,
                                        candidate,
                                        title: &guarded_title,
                                        input: &guarded_input,
                                        evaluator,
                                        libraries,
                                        tag_conflicts,
                                    },
                                    step,
                                    &mut run,
                                )
                                .await
                            }
                            SafetyDecision::Hold(reason) => Ok(SequenceStepResult::Held(reason)),
                            SafetyDecision::Cancel(reason) => {
                                Ok(SequenceStepResult::Canceled(reason))
                            }
                            SafetyDecision::Excluded => Ok(SequenceStepResult::Excluded),
                        }
                    }
                    None => Ok(SequenceStepResult::Held(
                        execution_reason::LOCATION_OPERATION_HOLD,
                    )),
                }
            };
            let result = if step.kind == MaintenanceActionStepKind::Search {
                result
            } else {
                result.and_then(|outcome| {
                    self.reconcile_sequence_job_dispatch(
                        SequenceJobDispatchResult::Immediate(outcome),
                        &run,
                        "",
                    )
                })
            };
            match result {
                Ok(SequenceStepResult::Finished(state)) => {
                    run.state = state;
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    if !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .finish_action_step(&run, &lease_id)
                        .await?
                    {
                        return Ok(SequenceOutcome::LeaseLost);
                    }
                    if state.is_completion_success() {
                        completed.insert(step.id.clone(), run);
                        continue;
                    }
                    return Ok(SequenceOutcome::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                Ok(SequenceStepResult::Held(reason)) => {
                    run.state = MaintenanceActionStepState::Held;
                    run.hold_reason = Some(reason.to_string());
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    if !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .finish_action_step(&run, &lease_id)
                        .await?
                    {
                        return Ok(SequenceOutcome::LeaseLost);
                    }
                    return Ok(SequenceOutcome::Held(reason));
                }
                Ok(SequenceStepResult::Canceled(reason)) => {
                    run.state = MaintenanceActionStepState::Held;
                    run.hold_reason = Some(reason.to_string());
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    if !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .finish_action_step(&run, &lease_id)
                        .await?
                    {
                        return Ok(SequenceOutcome::LeaseLost);
                    }
                    return Ok(SequenceOutcome::Canceled(reason));
                }
                Ok(SequenceStepResult::Excluded) => {
                    run.state = MaintenanceActionStepState::Held;
                    run.hold_reason = Some(
                        crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED
                            .to_string(),
                    );
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    if !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .finish_action_step(&run, &lease_id)
                        .await?
                    {
                        return Ok(SequenceOutcome::LeaseLost);
                    }
                    return Ok(SequenceOutcome::Excluded);
                }
                Err(error) => {
                    budget.record_failed_dispatch();
                    run.state = MaintenanceActionStepState::Failed;
                    run.error = Some(error.to_string());
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    if !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .finish_action_step(&run, &lease_id)
                        .await?
                    {
                        return Ok(SequenceOutcome::LeaseLost);
                    }
                    // The per-step budget is intentionally independent from
                    // the legacy candidate budget. Failed evidence survives
                    // each retry under the same stable step key.
                    let terminal_error = matches!(
                        error,
                        AppError::Validation(_) | AppError::Unauthorized(_) | AppError::NotFound(_)
                    );
                    if terminal_error
                        || run.attempt >= super::action_execution::MAINTENANCE_MAX_ACTION_ATTEMPTS
                    {
                        return Ok(SequenceOutcome::Failed {
                            step_id: step.id.clone(),
                        });
                    }
                    return Ok(SequenceOutcome::RetryingFailed);
                }
            }
        }
        Ok(SequenceOutcome::Completed)
    }

    async fn execute_sequence_step(
        &self,
        context: SequenceStepExecutionContext<'_>,
        step: &MaintenanceActionStep,
        run: &mut MaintenanceActionStepRun,
    ) -> AppResult<SequenceStepResult> {
        let SequenceStepExecutionContext {
            detail,
            sequence,
            step_index,
            completed,
            candidate,
            title,
            input,
            evaluator,
            libraries,
            tag_conflicts,
        } = context;
        match (&step.kind, &step.parameters) {
            (
                MaintenanceActionStepKind::Unmonitor,
                MaintenanceActionStepParameters::Unmonitor {
                    include_descendants,
                },
            ) => {
                let before_monitored = match &input.facts.monitored {
                    scryer_rules::maintenance::Observation::Known { value, .. } => Some(*value),
                    scryer_rules::maintenance::Observation::Unknown { .. }
                    | scryer_rules::maintenance::Observation::Absent { .. } => None,
                };
                let before_monitored_episode_count = match &input.facts.monitored_episode_count {
                    scryer_rules::maintenance::Observation::Known { value, .. } => Some(*value),
                    scryer_rules::maintenance::Observation::Unknown { .. }
                    | scryer_rules::maintenance::Observation::Absent { .. } => None,
                };
                let already_satisfied = self
                    .maintenance_unmonitoring_is_confirmed(candidate, title, *include_descendants)
                    .await?;
                let monitoring_changed = !already_satisfied;
                run.provenance_json = sequence_step_provenance(
                    candidate,
                    step,
                    serde_json::json!({
                        "monitoring_changed": monitoring_changed,
                        "include_descendants": include_descendants,
                        "expected_postcondition": "unmonitored",
                    }),
                );
                run.before_state_json = serde_json::json!({
                    "schema_version": 1,
                    "facts_monitored": before_monitored,
                    "facts_monitored_episode_count": before_monitored_episode_count,
                    "monitoring_changed": monitoring_changed,
                })
                .to_string();
                self.checkpoint_sequence_step(run).await?;
                if already_satisfied {
                    return Ok(SequenceStepResult::Finished(
                        MaintenanceActionStepState::AlreadySatisfied,
                    ));
                }
                if candidate.subject_kind == "title" {
                    self.unmonitor_maintenance_title_with_descendants(title, *include_descendants)
                        .await?;
                } else {
                    self.unmonitor_maintenance_child(candidate, title).await?;
                }
                if !self
                    .maintenance_unmonitoring_is_confirmed(candidate, title, *include_descendants)
                    .await?
                {
                    return Err(AppError::Repository(
                        "maintenance Unmonitor postcondition was not confirmed".to_string(),
                    ));
                }
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::Succeeded,
                ))
            }
            (
                MaintenanceActionStepKind::AddTags,
                MaintenanceActionStepParameters::Tags { tags },
            )
            | (
                MaintenanceActionStepKind::RemoveTags,
                MaintenanceActionStepParameters::Tags { tags },
            ) => {
                let adding = step.kind == MaintenanceActionStepKind::AddTags;
                let pending: Vec<String> = tags
                    .iter()
                    .filter(|tag| title.tags.iter().any(|present| present == *tag) != adding)
                    .cloned()
                    .collect();
                if pending.is_empty() {
                    run.provenance_json = sequence_step_provenance(
                        candidate,
                        step,
                        serde_json::json!({"changed_tags": []}),
                    );
                    self.checkpoint_sequence_step(run).await?;
                    return Ok(SequenceStepResult::Finished(
                        MaintenanceActionStepState::AlreadySatisfied,
                    ));
                }
                let (add, remove): (&[String], &[String]) = if adding {
                    (&pending, &[])
                } else {
                    (&[], &pending)
                };
                run.provenance_json = sequence_step_provenance(
                    candidate,
                    step,
                    serde_json::json!({
                        "operation": step.kind.as_wire_str(),
                        "changed_tags": pending,
                        "expected_postcondition": if adding { "tags_present" } else { "tags_absent" },
                    }),
                );
                self.checkpoint_sequence_step(run).await?;
                self.update_title_tags(
                    &scryer_domain::User::system_execution_actor(),
                    std::slice::from_ref(&title.id),
                    add,
                    remove,
                )
                .await?;
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::Succeeded,
                ))
            }
            (
                MaintenanceActionStepKind::ChangeQualityProfile,
                MaintenanceActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id,
                },
            ) => {
                let target = target_quality_profile_id.trim();
                let profiles = self.load_quality_profile_settings().await?;
                if !profiles.profiles.iter().any(|profile| {
                    crate::settings::runtime::quality_profile_ids_equal(&profile.id, target)
                }) {
                    return Err(AppError::Validation(format!(
                        "target quality profile '{target}' does not exist"
                    )));
                }
                let current = title.tags.iter().find_map(|tag| {
                    tag.strip_prefix(super::facts::QUALITY_PROFILE_TAG_PREFIX)
                        .map(str::trim)
                });
                run.before_state_json = serde_json::json!({
                    "schema_version": 1,
                    "quality_profile_id": current,
                    "title_tags": title.tags,
                })
                .to_string();
                if current.is_some_and(|current| {
                    crate::settings::runtime::quality_profile_ids_equal(current, target)
                }) {
                    run.provenance_json = sequence_step_provenance(
                        candidate,
                        step,
                        serde_json::json!({
                            "profile_changed": false,
                            "quality_profile_id": target,
                        }),
                    );
                    self.checkpoint_sequence_step(run).await?;
                    return Ok(SequenceStepResult::Finished(
                        MaintenanceActionStepState::AlreadySatisfied,
                    ));
                }
                let mut tags: Vec<String> = title
                    .tags
                    .iter()
                    .filter(|tag| !tag.starts_with(super::facts::QUALITY_PROFILE_TAG_PREFIX))
                    .cloned()
                    .collect();
                tags.push(format!(
                    "{}{}",
                    super::facts::QUALITY_PROFILE_TAG_PREFIX,
                    target
                ));
                run.provenance_json = sequence_step_provenance(
                    candidate,
                    step,
                    serde_json::json!({
                        "profile_changed": true,
                        "quality_profile_id": target,
                        "expected_postcondition": "quality_profile_changed",
                    }),
                );
                self.checkpoint_sequence_step(run).await?;
                self.update_title_metadata(
                    &scryer_domain::User::system_execution_actor(),
                    &title.id,
                    None,
                    None,
                    Some(tags),
                )
                .await?;
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::Succeeded,
                ))
            }
            (
                MaintenanceActionStepKind::Search,
                MaintenanceActionStepParameters::Search { condition },
            ) => {
                let should_skip = matches!(
                    condition,
                    super::action_sequence::MaintenanceSearchCondition::PreviousProfileChanged
                ) && sequence.steps[..step_index]
                    .last()
                    .filter(|previous| {
                        previous.kind == MaintenanceActionStepKind::ChangeQualityProfile
                    })
                    .and_then(|profile| completed.get(&profile.id))
                    .and_then(|profile| {
                        serde_json::from_str::<serde_json::Value>(&profile.provenance_json).ok()
                    })
                    .and_then(|provenance| {
                        provenance
                            .get("profile_changed")
                            .and_then(serde_json::Value::as_bool)
                    })
                    != Some(true);
                if should_skip {
                    run.provenance_json = sequence_step_provenance(
                        candidate,
                        step,
                        serde_json::json!({"skipped": "previous_profile_unchanged"}),
                    );
                    self.checkpoint_sequence_step(run).await?;
                    return Ok(SequenceStepResult::Finished(
                        MaintenanceActionStepState::Skipped,
                    ));
                }
                self.dispatch_sequence_search(candidate, step, run).await
            }
            (
                MaintenanceActionStepKind::DeleteTitleAndFiles,
                MaintenanceActionStepParameters::None,
            ) => {
                let actor = scryer_domain::User::system_execution_actor();
                let preview = self.preview_delete_title_files(&actor, &title.id).await?;
                run.target_identity_json = sequence_target_with(
                    run,
                    serde_json::json!({
                        "preview_fingerprint": preview.fingerprint,
                    }),
                );
                run.provenance_json = sequence_step_provenance(
                    candidate,
                    step,
                    serde_json::json!({"expected_postcondition": "title_absent"}),
                );
                self.checkpoint_sequence_step(run).await?;
                self.delete_title_by_policy(
                    &actor,
                    &title.id,
                    &preview.fingerprint,
                    &crate::PolicyDeleteAuthorization {
                        rule_set_id: candidate.rule_set_id.clone(),
                        candidate_id: candidate.id.clone(),
                        revision_number: candidate.revision_number,
                    },
                )
                .await?;
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::Succeeded,
                ))
            }
            (MaintenanceActionStepKind::DeleteFiles, MaintenanceActionStepParameters::None) => {
                self.execute_sequence_scoped_deletion(
                    SequenceStepExecutionContext {
                        detail,
                        sequence,
                        step_index,
                        completed,
                        candidate,
                        title,
                        input,
                        evaluator,
                        libraries,
                        tag_conflicts,
                    },
                    step,
                    run,
                )
                .await
            }
            _ => Err(AppError::Validation(
                "maintenance sequence step parameters are invalid".into(),
            )),
        }
    }

    async fn checkpoint_sequence_step(&self, run: &MaintenanceActionStepRun) -> AppResult<()> {
        let lease_id = run.lease_id.as_deref().ok_or_else(|| {
            AppError::Repository("claimed sequence step has no lease identity".to_string())
        })?;
        if self
            .services
            .customization
            .maintenance_evaluation
            .checkpoint_action_step(run, lease_id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Repository(
                "sequence step lease was lost before its mutation".to_string(),
            ))
        }
    }

    async fn sequence_step_postcondition_holds(
        &self,
        candidate: &LifecycleCandidate,
        step: &MaintenanceActionStep,
        run: &MaintenanceActionStepRun,
        title: &Title,
    ) -> AppResult<bool> {
        let provenance = serde_json::from_str::<serde_json::Value>(&run.provenance_json)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        match (&step.kind, &step.parameters) {
            (
                MaintenanceActionStepKind::Unmonitor,
                MaintenanceActionStepParameters::Unmonitor {
                    include_descendants,
                },
            ) => {
                self.maintenance_unmonitoring_is_confirmed(candidate, title, *include_descendants)
                    .await
            }
            (MaintenanceActionStepKind::AddTags, MaintenanceActionStepParameters::Tags { .. }) => {
                Ok(provenance
                    .get("changed_tags")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|tags| {
                        tags.iter()
                            .filter_map(serde_json::Value::as_str)
                            .all(|tag| title.tags.iter().any(|current| current == tag))
                    }))
            }
            (
                MaintenanceActionStepKind::RemoveTags,
                MaintenanceActionStepParameters::Tags { .. },
            ) => Ok(provenance
                .get("changed_tags")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tags| {
                    tags.iter()
                        .filter_map(serde_json::Value::as_str)
                        .all(|tag| !title.tags.iter().any(|current| current == tag))
                })),
            (
                MaintenanceActionStepKind::ChangeQualityProfile,
                MaintenanceActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id,
                },
            ) => Ok(title.tags.iter().any(|tag| {
                tag.strip_prefix(super::facts::QUALITY_PROFILE_TAG_PREFIX)
                    .map(str::trim)
                    .is_some_and(|current| {
                        crate::settings::runtime::quality_profile_ids_equal(
                            current,
                            target_quality_profile_id,
                        )
                    })
            })),
            _ => Ok(false),
        }
    }

    fn reconcile_sequence_job_dispatch(
        &self,
        dispatch: SequenceJobDispatchResult,
        run: &MaintenanceActionStepRun,
        request_hash: &str,
    ) -> AppResult<SequenceStepResult> {
        let (receipt, completion_policy) = match dispatch {
            SequenceJobDispatchResult::Immediate(result) => return Ok(result),
            SequenceJobDispatchResult::DurableJob {
                receipt,
                completion_policy,
            } => (receipt, completion_policy),
        };
        if receipt.validate_schema().is_err()
            || receipt.key != run.key
            || receipt.logical_request_key != run.key.logical_request_key()
            || receipt.request_hash != request_hash
        {
            return Ok(SequenceStepResult::Held(
                execution_reason::UNKNOWN_AT_EXECUTION,
            ));
        }
        match (receipt.state, completion_policy) {
            (MaintenanceActionJobReceiptState::Completed, _)
            | (
                MaintenanceActionJobReceiptState::Accepted,
                MaintenanceActionCompletionPolicy::Accepted,
            ) => Ok(SequenceStepResult::Finished(
                MaintenanceActionStepState::Succeeded,
            )),
            (
                MaintenanceActionJobReceiptState::Accepted,
                MaintenanceActionCompletionPolicy::Completed,
            ) => Ok(SequenceStepResult::Held(
                execution_reason::ACTIVE_ACQUISITION,
            )),
            (MaintenanceActionJobReceiptState::Dispatching, _)
            | (MaintenanceActionJobReceiptState::Unknown, _) => Ok(SequenceStepResult::Held(
                execution_reason::UNKNOWN_AT_EXECUTION,
            )),
            (MaintenanceActionJobReceiptState::Failed, _) => Err(AppError::Repository(
                "maintenance sequence job dispatch failed".to_string(),
            )),
        }
    }

    async fn dispatch_sequence_search(
        &self,
        candidate: &LifecycleCandidate,
        step: &MaintenanceActionStep,
        run: &mut MaintenanceActionStepRun,
    ) -> AppResult<SequenceStepResult> {
        let mut intent = sequence_step_intent(&run.intent_json).ok_or_else(|| {
            AppError::Repository("maintenance Search step intent is unreadable".to_string())
        })?;
        if intent.step != *step {
            return Err(AppError::Repository(
                "maintenance Search step intent does not match its sequence".to_string(),
            ));
        }
        let request_intent = match intent.search_request.clone() {
            Some(request) => request,
            None => {
                let has_file = !self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&candidate.title_id)
                    .await?
                    .is_empty();
                let request = SequenceSearchRequest {
                    wanted_kind: if has_file {
                        WantedKind::CutoffUpgrade.as_str().to_string()
                    } else {
                        WantedKind::Missing.as_str().to_string()
                    },
                    title_id: candidate.title_id.clone(),
                };
                intent.search_request = Some(request.clone());
                run.intent_json = serde_json::to_string(&intent)
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                request
            }
        };
        let wanted_kind = WantedKind::parse(&request_intent.wanted_kind).ok_or_else(|| {
            AppError::Repository(
                "maintenance Search request has an invalid wanted kind".to_string(),
            )
        })?;
        if request_intent.title_id != candidate.title_id {
            return Err(AppError::Repository(
                "maintenance Search request targets a different title".to_string(),
            ));
        }
        let request_hash = blake3::hash(
            serde_json::to_vec(&request_intent)
                .map_err(|error| AppError::Repository(error.to_string()))?
                .as_slice(),
        )
        .to_hex()
        .to_string();
        let receipts = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_job_receipts(&run.key)
            .await?;
        #[cfg(test)]
        if test_completed_sequence_job_state(&run.key).is_some() {
            return self
                .dispatch_test_completed_sequence_job(
                    candidate,
                    step,
                    run,
                    &request_intent,
                    &request_hash,
                    &receipts,
                )
                .await;
        }
        for receipt in &receipts {
            if receipt.validate_schema().is_err()
                || receipt.key != run.key
                || receipt.logical_request_key != run.key.logical_request_key()
            {
                return Ok(SequenceStepResult::Held(
                    execution_reason::UNKNOWN_AT_EXECUTION,
                ));
            }
            if receipt.request_hash != request_hash {
                continue;
            }
            match receipt.state {
                MaintenanceActionJobReceiptState::Accepted
                | MaintenanceActionJobReceiptState::Completed => {
                    run.provenance_json = sequence_step_provenance(
                        candidate,
                        step,
                        serde_json::json!({
                            "request_hash": request_hash,
                            "dispatch_attempt": receipt.dispatch_attempt,
                            "job_run_id": receipt.job_run_id,
                            "accepted": true,
                        }),
                    );
                    self.checkpoint_sequence_step(run).await?;
                    return Ok(SequenceStepResult::Finished(
                        MaintenanceActionStepState::Succeeded,
                    ));
                }
                MaintenanceActionJobReceiptState::Dispatching
                | MaintenanceActionJobReceiptState::Unknown => {
                    return Ok(SequenceStepResult::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                MaintenanceActionJobReceiptState::Failed => {}
            }
        }
        let dispatch_attempt = receipts
            .iter()
            .map(|receipt| receipt.dispatch_attempt)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now();
        let receipt = MaintenanceActionJobReceipt {
            schema_version: MAINTENANCE_ACTION_JOB_RECEIPT_SCHEMA_VERSION,
            key: run.key.clone(),
            dispatch_attempt,
            logical_request_key: run.key.logical_request_key(),
            request_hash: request_hash.clone(),
            job_run_id: Some(Id::new().0),
            state: MaintenanceActionJobReceiptState::Accepted,
            reconciliation_evidence_json: serde_json::json!({
                "schema_version": 1,
                "request": request_intent,
            })
            .to_string(),
            created_at: now,
            updated_at: now,
        };
        run.target_identity_json = sequence_target_with(
            run,
            serde_json::json!({
                "search_request_hash": request_hash,
                "search_title_id": candidate.title_id,
            }),
        );
        run.provenance_json = sequence_step_provenance(
            candidate,
            step,
            serde_json::json!({
                "request_hash": receipt.request_hash,
                "dispatch_attempt": receipt.dispatch_attempt,
                "job_run_id": receipt.job_run_id,
                "dispatch_state": "accepted_intent",
            }),
        );
        self.checkpoint_sequence_step(run).await?;
        let request = AcquisitionSearchRequest {
            automatic: false,
            wanted_kind,
            facet: None,
            library_ids: Vec::new(),
            title_id: Some(candidate.title_id.clone()),
            season_number: None,
            wanted_item_id: None,
        };
        let receipt = match self
            .start_maintenance_acquisition_search_job(
                &scryer_domain::User::system_execution_actor(),
                request,
                receipt,
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(AppError::Validation(message))
                if message.contains("acquisition search job is already running") =>
            {
                return Ok(SequenceStepResult::Held(
                    execution_reason::ACTIVE_ACQUISITION,
                ));
            }
            Err(error) => return Err(error),
        };
        run.provenance_json = sequence_step_provenance(
            candidate,
            step,
            serde_json::json!({
                "request_hash": receipt.request_hash,
                "dispatch_attempt": receipt.dispatch_attempt,
                "job_run_id": receipt.job_run_id,
                "dispatch_state": receipt.state.as_storage_str(),
            }),
        );
        self.reconcile_sequence_job_dispatch(
            SequenceJobDispatchResult::DurableJob {
                receipt,
                completion_policy: MaintenanceActionCompletionPolicy::Accepted,
            },
            run,
            &request_hash,
        )
    }

    #[cfg(test)]
    async fn dispatch_test_completed_sequence_job(
        &self,
        candidate: &LifecycleCandidate,
        step: &MaintenanceActionStep,
        run: &mut MaintenanceActionStepRun,
        request: &SequenceSearchRequest,
        request_hash: &str,
        receipts: &[MaintenanceActionJobReceipt],
    ) -> AppResult<SequenceStepResult> {
        run.target_identity_json = sequence_target_with(
            run,
            serde_json::json!({
                "search_request_hash": request_hash,
                "search_title_id": request.title_id,
            }),
        );
        run.provenance_json = sequence_step_provenance(
            candidate,
            step,
            serde_json::json!({
                "request_hash": request_hash,
                "test_completed_adapter": true,
            }),
        );
        self.checkpoint_sequence_step(run).await?;

        let mut current = receipts
            .iter()
            .filter(|receipt| receipt.request_hash == request_hash)
            .max_by_key(|receipt| receipt.dispatch_attempt)
            .cloned();
        if current
            .as_ref()
            .is_some_and(|receipt| receipt.state == MaintenanceActionJobReceiptState::Failed)
        {
            current = None;
        }
        if current.is_none() {
            let dispatch_attempt = receipts
                .iter()
                .map(|receipt| receipt.dispatch_attempt)
                .max()
                .unwrap_or(0)
                + 1;
            let now = Utc::now();
            let dispatching = MaintenanceActionJobReceipt {
                schema_version: MAINTENANCE_ACTION_JOB_RECEIPT_SCHEMA_VERSION,
                key: run.key.clone(),
                dispatch_attempt,
                logical_request_key: run.key.logical_request_key(),
                request_hash: request_hash.to_string(),
                job_run_id: Some(Id::new().0),
                state: MaintenanceActionJobReceiptState::Dispatching,
                reconciliation_evidence_json: serde_json::json!({
                    "request": request,
                    "test_completed_adapter": true,
                })
                .to_string(),
                created_at: now,
                updated_at: now,
            };
            current = Some(
                match self
                    .services
                    .customization
                    .maintenance_evaluation
                    .claim_action_job_dispatch(&dispatching)
                    .await?
                {
                    MaintenanceActionJobReceiptClaim::Claimed(receipt) => receipt,
                    MaintenanceActionJobReceiptClaim::Existing(receipt) => receipt,
                },
            );
        }
        let receipt = current.expect("a test completed job receipt is always selected");
        if receipt.validate_schema().is_err()
            || receipt.key != run.key
            || receipt.logical_request_key != run.key.logical_request_key()
            || receipt.request_hash != request_hash
        {
            return Ok(SequenceStepResult::Held(
                execution_reason::UNKNOWN_AT_EXECUTION,
            ));
        }
        let Some(state) = test_completed_sequence_job_state(&run.key) else {
            return Ok(SequenceStepResult::Held(
                execution_reason::UNKNOWN_AT_EXECUTION,
            ));
        };
        match (receipt.state, state) {
            (MaintenanceActionJobReceiptState::Completed, _) => self
                .reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                ),
            (
                MaintenanceActionJobReceiptState::Dispatching,
                TestCompletedSequenceJobState::Waiting,
            )
            | (
                MaintenanceActionJobReceiptState::Accepted,
                TestCompletedSequenceJobState::Waiting,
            ) => {
                let accepted = MaintenanceActionJobReceiptTransition {
                    key: run.key.clone(),
                    dispatch_attempt: receipt.dispatch_attempt,
                    expected_states: vec![MaintenanceActionJobReceiptState::Dispatching],
                    next_state: MaintenanceActionJobReceiptState::Accepted,
                    job_run_id: receipt.job_run_id.clone(),
                    reconciliation_evidence_json: serde_json::json!({
                        "test_completed_adapter": "waiting",
                    })
                    .to_string(),
                    updated_at: Utc::now(),
                };
                let mut accepted_receipt = receipt.clone();
                if receipt.state == MaintenanceActionJobReceiptState::Dispatching
                    && !self
                        .services
                        .customization
                        .maintenance_evaluation
                        .transition_action_job_receipt(&accepted)
                        .await?
                {
                    return Ok(SequenceStepResult::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                accepted_receipt.state = MaintenanceActionJobReceiptState::Accepted;
                self.reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt: accepted_receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                )
            }
            (
                state @ (MaintenanceActionJobReceiptState::Dispatching
                | MaintenanceActionJobReceiptState::Accepted
                | MaintenanceActionJobReceiptState::Unknown),
                TestCompletedSequenceJobState::Completed,
            ) => {
                let transition = MaintenanceActionJobReceiptTransition {
                    key: run.key.clone(),
                    dispatch_attempt: receipt.dispatch_attempt,
                    expected_states: vec![state],
                    next_state: MaintenanceActionJobReceiptState::Completed,
                    job_run_id: receipt.job_run_id.clone(),
                    reconciliation_evidence_json: serde_json::json!({
                        "test_completed_adapter": "completed",
                    })
                    .to_string(),
                    updated_at: Utc::now(),
                };
                if !self
                    .services
                    .customization
                    .maintenance_evaluation
                    .transition_action_job_receipt(&transition)
                    .await?
                {
                    return Ok(SequenceStepResult::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                let mut completed_receipt = receipt.clone();
                completed_receipt.state = MaintenanceActionJobReceiptState::Completed;
                self.reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt: completed_receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                )
            }
            (
                state @ (MaintenanceActionJobReceiptState::Dispatching
                | MaintenanceActionJobReceiptState::Accepted
                | MaintenanceActionJobReceiptState::Unknown),
                TestCompletedSequenceJobState::Unknown,
            ) => {
                let transition = MaintenanceActionJobReceiptTransition {
                    key: run.key.clone(),
                    dispatch_attempt: receipt.dispatch_attempt,
                    expected_states: vec![state],
                    next_state: MaintenanceActionJobReceiptState::Unknown,
                    job_run_id: receipt.job_run_id.clone(),
                    reconciliation_evidence_json: serde_json::json!({
                        "test_completed_adapter": "unknown",
                    })
                    .to_string(),
                    updated_at: Utc::now(),
                };
                if !self
                    .services
                    .customization
                    .maintenance_evaluation
                    .transition_action_job_receipt(&transition)
                    .await?
                {
                    return Ok(SequenceStepResult::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                let mut unknown_receipt = receipt.clone();
                unknown_receipt.state = MaintenanceActionJobReceiptState::Unknown;
                self.reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt: unknown_receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                )
            }
            (
                state @ (MaintenanceActionJobReceiptState::Dispatching
                | MaintenanceActionJobReceiptState::Accepted
                | MaintenanceActionJobReceiptState::Unknown),
                TestCompletedSequenceJobState::Failed,
            ) => {
                let transition = MaintenanceActionJobReceiptTransition {
                    key: run.key.clone(),
                    dispatch_attempt: receipt.dispatch_attempt,
                    expected_states: vec![state],
                    next_state: MaintenanceActionJobReceiptState::Failed,
                    job_run_id: receipt.job_run_id.clone(),
                    reconciliation_evidence_json: serde_json::json!({
                        "test_completed_adapter": "failed",
                    })
                    .to_string(),
                    updated_at: Utc::now(),
                };
                if !self
                    .services
                    .customization
                    .maintenance_evaluation
                    .transition_action_job_receipt(&transition)
                    .await?
                {
                    return Ok(SequenceStepResult::Held(
                        execution_reason::UNKNOWN_AT_EXECUTION,
                    ));
                }
                let mut failed_receipt = receipt.clone();
                failed_receipt.state = MaintenanceActionJobReceiptState::Failed;
                self.reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt: failed_receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                )
            }
            (MaintenanceActionJobReceiptState::Unknown, TestCompletedSequenceJobState::Waiting) => {
                self.reconcile_sequence_job_dispatch(
                    SequenceJobDispatchResult::DurableJob {
                        receipt,
                        completion_policy: MaintenanceActionCompletionPolicy::Completed,
                    },
                    run,
                    request_hash,
                )
            }
            (MaintenanceActionJobReceiptState::Failed, _) => self.reconcile_sequence_job_dispatch(
                SequenceJobDispatchResult::DurableJob {
                    receipt,
                    completion_policy: MaintenanceActionCompletionPolicy::Completed,
                },
                run,
                request_hash,
            ),
        }
    }

    async fn execute_sequence_scoped_deletion(
        &self,
        context: SequenceStepExecutionContext<'_>,
        step: &MaintenanceActionStep,
        run: &mut MaintenanceActionStepRun,
    ) -> AppResult<SequenceStepResult> {
        let SequenceStepExecutionContext {
            detail,
            sequence,
            step_index,
            candidate,
            title,
            input,
            evaluator,
            libraries,
            tag_conflicts,
            ..
        } = context;
        let Some(unmonitor) = sequence.steps[..step_index]
            .iter()
            .find(|previous| previous.kind == MaintenanceActionStepKind::Unmonitor)
        else {
            return Err(AppError::Validation(
                "DeleteFiles requires an earlier Unmonitor step".to_string(),
            ));
        };
        let MaintenanceActionStepParameters::Unmonitor {
            include_descendants,
        } = &unmonitor.parameters
        else {
            return Err(AppError::Validation(
                "DeleteFiles has an incompatible Unmonitor prerequisite".to_string(),
            ));
        };
        if !self
            .maintenance_unmonitoring_is_confirmed(candidate, title, *include_descendants)
            .await?
        {
            return Ok(SequenceStepResult::Canceled(
                execution_reason::NO_MATCH_AT_EXECUTION,
            ));
        }

        let saved_checkpoint = self
            .maintenance_deletion_checkpoint_for_step(
                candidate,
                detail.revision.storage_root_id.as_deref(),
                Some(&step.id),
            )
            .await?;
        let mut deletion_run = if saved_checkpoint.is_some() {
            let saved_run_id = serde_json::from_str::<serde_json::Value>(&run.target_identity_json)
                .ok()
                .and_then(|target| {
                    target
                        .get("scoped_deletion_action_run_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| {
                    AppError::Repository(
                        "sequence deletion checkpoint lacks its action-run identity".to_string(),
                    )
                })?;
            let existing = self
                .services
                .customization
                .maintenance_evaluation
                .latest_scoped_deletion_action_run(
                    &candidate.id,
                    candidate.match_generation,
                    &candidate.action_kind,
                )
                .await?
                .ok_or_else(|| {
                    AppError::Repository(
                        "sequence deletion checkpoint action run is missing".to_string(),
                    )
                })?;
            if existing.id != saved_run_id {
                return Err(AppError::Repository(
                    "sequence deletion checkpoint action run identity changed".to_string(),
                ));
            }
            existing
        } else {
            let now = Utc::now();
            let deletion_run_id = Id::new().0;
            let deletion_run = LifecycleActionRun {
                id: deletion_run_id.clone(),
                candidate_id: candidate.id.clone(),
                rule_set_id: candidate.rule_set_id.clone(),
                revision_number: candidate.revision_number,
                title_id: candidate.title_id.clone(),
                subject_kind: candidate.subject_kind.clone(),
                subject_id: candidate.subject_id.clone(),
                action_kind: candidate.action_kind.clone(),
                match_generation: candidate.match_generation,
                idempotency_key: format!(
                    "maintenance-sequence-delete-files:{}:{}:{}:{}:{}:{}",
                    candidate.id,
                    candidate.match_generation,
                    candidate.revision_number,
                    step.id,
                    run.attempt,
                    deletion_run_id,
                ),
                attempt: run.attempt,
                status: LifecycleActionRunStatus::Running,
                hold_reason: None,
                error: None,
                detail: "{}".to_string(),
                started_at: now,
                finished_at: None,
                created_at: now,
            };
            run.target_identity_json = sequence_target_with(
                run,
                serde_json::json!({"scoped_deletion_action_run_id": deletion_run.id}),
            );
            run.provenance_json = sequence_step_provenance(
                candidate,
                step,
                serde_json::json!({"scoped_deletion_step_id": step.id}),
            );
            self.checkpoint_sequence_step(run).await?;
            self.services
                .customization
                .maintenance_evaluation
                .start_action_run(&deletion_run)
                .await?;
            deletion_run
        };
        deletion_run.status = LifecycleActionRunStatus::Running;
        deletion_run.hold_reason = None;
        deletion_run.error = None;
        deletion_run.finished_at = None;
        let action = {
            let mut context = MaintenanceExecutionContext {
                detail,
                candidate,
                title,
                input,
                run: &mut deletion_run,
                evaluator,
                libraries,
                tag_conflicts,
            };
            self.execute_scoped_maintenance_deletion(
                &mut context,
                Some(&step.id),
                false,
                Some(*include_descendants),
            )
            .await
        };
        match action {
            Ok(ActionResult::Executed { detail }) => {
                deletion_run.status = LifecycleActionRunStatus::Succeeded;
                deletion_run.detail = detail.to_string();
                deletion_run.finished_at = Some(Utc::now());
                self.services
                    .customization
                    .maintenance_evaluation
                    .finish_action_run(&deletion_run)
                    .await?;
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::Succeeded,
                ))
            }
            Ok(ActionResult::AlreadySatisfied { detail }) => {
                deletion_run.status = LifecycleActionRunStatus::AlreadySatisfied;
                deletion_run.detail = detail.to_string();
                deletion_run.finished_at = Some(Utc::now());
                self.services
                    .customization
                    .maintenance_evaluation
                    .finish_action_run(&deletion_run)
                    .await?;
                Ok(SequenceStepResult::Finished(
                    MaintenanceActionStepState::AlreadySatisfied,
                ))
            }
            Ok(ActionResult::Held { reason, detail }) => {
                deletion_run.status = LifecycleActionRunStatus::Held;
                deletion_run.hold_reason = Some(reason.to_string());
                deletion_run.detail = detail.to_string();
                deletion_run.finished_at = Some(Utc::now());
                self.services
                    .customization
                    .maintenance_evaluation
                    .finish_action_run(&deletion_run)
                    .await?;
                Ok(SequenceStepResult::Held(reason))
            }
            Ok(ActionResult::Canceled { reason, detail }) => {
                deletion_run.status = LifecycleActionRunStatus::Held;
                deletion_run.hold_reason = Some(reason.to_string());
                deletion_run.detail = detail.to_string();
                deletion_run.finished_at = Some(Utc::now());
                self.services
                    .customization
                    .maintenance_evaluation
                    .finish_action_run(&deletion_run)
                    .await?;
                if reason == crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED {
                    Ok(SequenceStepResult::Excluded)
                } else {
                    Ok(SequenceStepResult::Canceled(reason))
                }
            }
            Err(error) => {
                deletion_run.status = LifecycleActionRunStatus::Failed;
                deletion_run.error = Some(error.to_string());
                deletion_run.finished_at = Some(Utc::now());
                self.services
                    .customization
                    .maintenance_evaluation
                    .finish_action_run(&deletion_run)
                    .await?;
                Err(error)
            }
        }
    }
}

fn sequence_target_with(run: &MaintenanceActionStepRun, detail: serde_json::Value) -> String {
    let mut target = serde_json::from_str::<serde_json::Value>(&run.target_identity_json)
        .unwrap_or_else(|_| serde_json::json!({"schema_version": 1}));
    if let (Some(target), Some(detail)) = (target.as_object_mut(), detail.as_object()) {
        target.extend(detail.clone());
    }
    target.to_string()
}

fn terminal_title_absence_is_proven(run: &MaintenanceActionStepRun) -> bool {
    serde_json::from_str::<serde_json::Value>(&run.target_identity_json)
        .ok()
        .and_then(|target| {
            target
                .get("preview_fingerprint")
                .and_then(serde_json::Value::as_str)
                .filter(|fingerprint| !fingerprint.trim().is_empty())
                .map(|_| ())
        })
        .is_some()
        && serde_json::from_str::<serde_json::Value>(&run.provenance_json)
            .ok()
            .is_some_and(|provenance| {
                provenance
                    .get("expected_postcondition")
                    .and_then(serde_json::Value::as_str)
                    == Some("title_absent")
            })
}

/// A stale `Running` row may be retried only when its durable pre-mutation
/// snapshot still describes the live target. A postcondition proves a completed
/// write; this complementary check proves that replaying an idempotent write is
/// still safe after a crash before the write. Any unreadable or changed state
/// stays held for an operator rather than being overwritten.
fn sequence_step_before_state_holds(
    candidate: &LifecycleCandidate,
    step: &MaintenanceActionStep,
    run: &MaintenanceActionStepRun,
    title: &Title,
) -> bool {
    let Ok(before) = serde_json::from_str::<serde_json::Value>(&run.before_state_json) else {
        return false;
    };
    match (&step.kind, &step.parameters) {
        (
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        ) if candidate.subject_kind == "title"
            && candidate.subject_id == candidate.title_id
            && title.id == candidate.title_id =>
        {
            before
                .get("facts_monitored")
                .and_then(serde_json::Value::as_bool)
                .is_some_and(|monitored| title.monitored == monitored)
        }
        (MaintenanceActionStepKind::AddTags, MaintenanceActionStepParameters::Tags { .. })
        | (MaintenanceActionStepKind::RemoveTags, MaintenanceActionStepParameters::Tags { .. })
        | (
            MaintenanceActionStepKind::ChangeQualityProfile,
            MaintenanceActionStepParameters::ChangeQualityProfile { .. },
        ) => before
            .get("title_tags")
            .and_then(serde_json::Value::as_array)
            .and_then(|tags| {
                tags.iter()
                    .map(serde_json::Value::as_str)
                    .collect::<Option<Vec<_>>>()
            })
            .is_some_and(|tags| {
                title.tags.len() == tags.len()
                    && title
                        .tags
                        .iter()
                        .zip(tags)
                        .all(|(current, saved)| current == saved)
            }),
        _ => false,
    }
}

fn sequence_step_run(
    key: MaintenanceActionStepKey,
    context: SequenceStepRunContext<'_>,
    step: &MaintenanceActionStep,
    attempt: i64,
    lease_id: &str,
) -> AppResult<MaintenanceActionStepRun> {
    let SequenceStepRunContext {
        detail,
        candidate,
        sequence_hash,
        title,
    } = context;
    let now = Utc::now();
    Ok(MaintenanceActionStepRun {
        key,
        rule_set_id: candidate.rule_set_id.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        sequence_content_hash: sequence_hash.to_string(),
        step_kind: step.kind.as_wire_str().to_string(),
        intent_json: serde_json::to_string(&SequenceStepIntent {
            step: step.clone(),
            search_request: None,
        })
        .map_err(|error| AppError::Repository(error.to_string()))?,
        before_state_json: serde_json::json!({
            "schema_version": 1,
            "facts_monitored": title.monitored,
            "monitoring_changed": false,
            "title_tags": title.tags,
        })
        .to_string(),
        target_identity_json: serde_json::json!({
            "schema_version": 1,
            "rule_set_id": detail.rule_set.id,
            "candidate_id": candidate.id,
            "match_generation": candidate.match_generation,
            "revision_number": candidate.revision_number,
            "title_id": title.id,
            "subject_kind": candidate.subject_kind,
            "subject_id": candidate.subject_id,
            "step_id": step.id,
            "step_kind": step.kind.as_wire_str(),
            "sequence_content_hash": sequence_hash,
        })
        .to_string(),
        provenance_json: sequence_step_provenance(candidate, step, serde_json::json!({})),
        state: MaintenanceActionStepState::Running,
        attempt,
        lease_id: Some(lease_id.to_string()),
        lease_expires_at: Some(now + Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES)),
        hold_reason: None,
        error: None,
        created_at: now,
        updated_at: now,
        finished_at: None,
    })
}

fn sequence_step_provenance(
    candidate: &LifecycleCandidate,
    step: &MaintenanceActionStep,
    detail: serde_json::Value,
) -> String {
    let mut provenance = serde_json::json!({
        "schema_version": 1,
        "candidate_id": candidate.id,
        "match_generation": candidate.match_generation,
        "revision_number": candidate.revision_number,
        "rule_set_id": candidate.rule_set_id,
        "title_id": candidate.title_id,
        "subject_kind": candidate.subject_kind,
        "subject_id": candidate.subject_id,
        "step_id": step.id,
        "step_kind": step.kind.as_wire_str(),
    });
    if let (Some(provenance), Some(detail)) = (provenance.as_object_mut(), detail.as_object()) {
        provenance.extend(detail.clone());
    }
    provenance.to_string()
}

pub(super) fn persisted_sequence_step_is_compatible(
    run: &MaintenanceActionStepRun,
    candidate: &LifecycleCandidate,
    sequence_hash: &str,
    step: &MaintenanceActionStep,
) -> bool {
    if run.key.candidate_id != candidate.id
        || run.key.match_generation != candidate.match_generation
        || run.key.revision_number != candidate.revision_number
        || run.key.step_id != step.id
        || run.rule_set_id != candidate.rule_set_id
        || run.title_id != candidate.title_id
        || run.subject_kind != candidate.subject_kind
        || run.subject_id != candidate.subject_id
        || run.sequence_content_hash != sequence_hash
        || run.step_kind != step.kind.as_wire_str()
    {
        return false;
    }
    if sequence_step_intent(&run.intent_json)
        .map(|intent| intent.step)
        .as_ref()
        != Some(step)
    {
        return false;
    }
    let target = match serde_json::from_str::<serde_json::Value>(&run.target_identity_json) {
        Ok(target) => target,
        Err(_) => return false,
    };
    let provenance = match serde_json::from_str::<serde_json::Value>(&run.provenance_json) {
        Ok(provenance) => provenance,
        Err(_) => return false,
    };
    let target_matches =
        |key: &str, value: &str| target.get(key).and_then(serde_json::Value::as_str) == Some(value);
    let provenance_matches = |key: &str, value: &str| {
        provenance.get(key).and_then(serde_json::Value::as_str) == Some(value)
    };
    target
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && target_matches("rule_set_id", &candidate.rule_set_id)
        && target_matches("candidate_id", &candidate.id)
        && target
            .get("match_generation")
            .and_then(serde_json::Value::as_i64)
            == Some(candidate.match_generation)
        && target
            .get("revision_number")
            .and_then(serde_json::Value::as_i64)
            == Some(candidate.revision_number)
        && target_matches("title_id", &candidate.title_id)
        && target_matches("subject_kind", &candidate.subject_kind)
        && target_matches("subject_id", &candidate.subject_id)
        && target_matches("step_id", &step.id)
        && target_matches("step_kind", step.kind.as_wire_str())
        && target_matches("sequence_content_hash", sequence_hash)
        && provenance
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
        && provenance_matches("rule_set_id", &candidate.rule_set_id)
        && provenance_matches("candidate_id", &candidate.id)
        && provenance
            .get("match_generation")
            .and_then(serde_json::Value::as_i64)
            == Some(candidate.match_generation)
        && provenance
            .get("revision_number")
            .and_then(serde_json::Value::as_i64)
            == Some(candidate.revision_number)
        && provenance_matches("title_id", &candidate.title_id)
        && provenance_matches("subject_kind", &candidate.subject_kind)
        && provenance_matches("subject_id", &candidate.subject_id)
        && provenance_matches("step_id", &step.id)
        && provenance_matches("step_kind", step.kind.as_wire_str())
}

fn sequence_step_intent(intent_json: &str) -> Option<SequenceStepIntent> {
    serde_json::from_str(intent_json).ok().or_else(|| {
        serde_json::from_str::<MaintenanceActionStep>(intent_json)
            .ok()
            .map(|step| SequenceStepIntent {
                step,
                search_request: None,
            })
    })
}

fn sequence_terminal_membership(
    sequence: &MaintenanceActionSequence,
    candidate: &LifecycleCandidate,
    outcome: MaintenanceSequenceTerminalOutcome,
    completed_step_ids: Vec<String>,
    terminal_step_id: Option<String>,
) -> AppResult<MaintenanceSequenceTerminalMembership> {
    Ok(MaintenanceSequenceTerminalMembership {
        id: Id::new().0,
        candidate_id: candidate.id.clone(),
        rule_set_id: candidate.rule_set_id.clone(),
        revision_number: candidate.revision_number,
        matcher_content_hash: candidate.matcher_content_hash.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        match_generation: candidate.match_generation,
        sequence_content_hash: sequence
            .content_hash()
            .map_err(|error| AppError::Validation(error.to_string()))?,
        outcome,
        expected_step_ids: sequence.steps.iter().map(|step| step.id.clone()).collect(),
        completed_step_ids,
        terminal_step_id,
        created_at: Utc::now(),
        released_at: None,
    })
}
