//! Scheduled execution of due maintenance candidates (RFC 137 sections 8, 9.6,
//! 9.8, 9.10, 10; tracks D2/D3, title scope).
//!
//! # What this module may do
//!
//! For rules that are enabled, in observe mode, and explicitly armed — and only
//! while the matching instance effect gate is on — execute the rule's
//! configured action on candidates whose grace has elapsed, through the same
//! application use cases a human operator would drive: monitoring setters, the
//! title metadata/profile path, the acquisition search job, and the
//! preview-fingerprinted deletion workflow.
//!
//! # Fail-closed shape
//!
//! Every ambiguity defers. A candidate is re-evaluated against fresh facts
//! immediately before its action: a subject that no longer matches cancels
//! instead of acting, and one that cannot be decided holds. Live playback
//! anywhere, an unreachable media server, and active or unattributable
//! acquisition work all hold. A hold writes an action-run row saying why and
//! leaves the candidate [`MaintenanceCandidateState::Blocked`], re-checked on
//! the next pass. Nothing in this module deletes a file outside
//! [`AppUseCase::delete_title_by_policy`], whose fingerprint check is the same
//! one the human deletion dialog uses.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use scryer_domain::{
    AppPermission, Id, LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MaintenanceCandidateState, MaintenanceEffectArming, MaintenanceEvaluationMode, Title, User,
};
use scryer_rules::maintenance::{MaintenanceOutcome, MaintenancePolicy, MaintenanceRulesEngine};
use serde::Serialize;
use tracing::warn;

use crate::jobs::{JobKey, JobTriggerSource};
use crate::maintenance_rules::action_catalog::{
    MaintenanceActionKind, MaintenanceActionParameters, MaintenanceRiskClass,
    MaintenanceTimingMode, descriptor_for,
};
use crate::maintenance_rules::evaluation::MaintenanceEvaluationTrigger;
use crate::maintenance_rules::facts::{
    MaintenanceLibraryRef, QUALITY_PROFILE_TAG_PREFIX, build_title_input,
};
use crate::maintenance_rules::safety::{MaintenanceActivityCheck, MaintenancePlaybackHold};
use crate::maintenance_rules::service::MaintenanceRuleSetDetail;
use crate::{AppError, AppResult, AppUseCase, PolicyDeleteAuthorization, WantedKind};

/// Executed actions per rule per handler pass. Over-cap candidates simply stay
/// due for the next pass (RFC 9.10's per-rule/per-run cap).
pub const MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN: usize = 10;

/// High-risk executions per handler pass across every rule.
pub const MAINTENANCE_MAX_HIGH_RISK_ACTIONS_PER_RUN: usize = 10;

/// High-risk failures in one pass that stop further high-risk work that pass —
/// a circuit breaker against a systematically failing deletion path.
pub const MAINTENANCE_HIGH_RISK_FAILURE_BREAKER: usize = 3;

/// Failed attempts before a candidate is marked terminally failed.
pub const MAINTENANCE_MAX_ACTION_ATTEMPTS: i64 = 3;

/// An `executing` lease older than this is considered abandoned by a crashed
/// run and may be re-leased.
const EXECUTION_LEASE_STALE_AFTER_MINUTES: i64 = 60;

/// Stable `hold_reason` / `state_reason` values the executor writes. Part of
/// the operator-visible contract, like `candidate_reason` on the evaluator.
pub mod execution_reason {
    /// The rule was disarmed, disabled, left observe mode, or its gate went
    /// down between selection and execution.
    pub const RULE_NOT_ELIGIBLE: &str = "rule_not_eligible";
    /// Fresh re-evaluation at execution time no longer matches.
    pub const NO_MATCH_AT_EXECUTION: &str = "no_match_at_execution";
    /// Fresh re-evaluation could not decide, or errored.
    pub const UNKNOWN_AT_EXECUTION: &str = "unknown_at_execution";
    /// A playback session is active on a configured media server.
    pub const PLAYBACK_HOLD: &str = "playback_hold";
    /// A configured media server could not be asked about playback.
    pub const PLAYBACK_UNKNOWN: &str = "playback_unknown";
    /// The title has a grab, download, or import in flight.
    pub const ACTIVE_ACQUISITION: &str = "active_acquisition";
    /// Acquisition activity could not be ruled out.
    pub const ACQUISITION_UNKNOWN: &str = "acquisition_unknown";
    /// The action needs an application seam that does not exist yet
    /// (`unmonitor_title_delete_all_files`'s title-preserving file deletion).
    pub const ACTION_NOT_FULLY_SUPPORTED: &str = "action_not_fully_supported";
    /// The subject no longer exists in the catalog.
    pub const TITLE_MISSING: &str = "title_missing";
    /// The action completed.
    pub const ACTION_SUCCEEDED: &str = "action_succeeded";
    /// The postcondition already held; nothing was mutated.
    pub const ALREADY_SATISFIED: &str = "already_satisfied";
    /// A transient failure was recorded; the candidate is due again.
    pub const RETRY_PENDING: &str = "retry_pending";
    /// The attempt budget is exhausted or the failure was terminal.
    pub const ACTION_FAILED: &str = "action_failed";
}

/// Job report for [`JobKey::LifecycleActionHandling`], serialized onto the run.
#[derive(Debug, Default, Serialize)]
pub struct MaintenanceActionHandlingReport {
    /// False when both effect gates were off and the pass did nothing.
    pub gates_enabled: bool,
    pub rules_considered: usize,
    pub rules_eligible: usize,
    pub candidates_considered: usize,
    pub executed: usize,
    pub already_satisfied: usize,
    pub held: usize,
    pub canceled: usize,
    pub failed: usize,
    pub lease_lost: usize,
}

/// One action run with its title's display name resolved for rendering.
#[derive(Clone, Debug)]
pub struct MaintenanceActionRunView {
    pub run: LifecycleActionRun,
    pub title_name: String,
}

/// Outcome of one candidate's pass, folded into the report.
enum CandidateOutcome {
    Executed,
    AlreadySatisfied,
    Held,
    Canceled,
    Failed,
    LeaseLost,
}

/// What the pre-execution safety rechecks decided.
enum SafetyDecision {
    /// Act on this title, which every check has just seen fresh.
    Proceed(Box<Title>),
    /// Refuse and block the candidate with this reason.
    Hold(&'static str),
    /// The subject stopped matching (or vanished); cancel the candidate.
    Cancel(&'static str),
    /// An exclusion now covers the subject.
    Excluded,
}

/// What an action implementation reports back.
enum ActionResult {
    Executed { detail: serde_json::Value },
    AlreadySatisfied { detail: serde_json::Value },
}

// ── Arming ──────────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Set how far one rule's effects are armed.
    ///
    /// Destructive arming is the elevated, count-acknowledged step of RFC 9.10:
    /// it applies only to rules whose action is high-risk, additionally
    /// requires system-settings authority (RFC 15), and demands the caller
    /// acknowledge the number of candidates the arming would expose. Arming is
    /// deliberately independent of the instance gates — execution requires
    /// both.
    pub async fn set_maintenance_rule_arming(
        &self,
        actor: &User,
        rule_set_id: &str,
        arming: MaintenanceEffectArming,
        acknowledged_candidate_count: Option<i64>,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        if arming == MaintenanceEffectArming::Destructive {
            self.require_app_permission(actor, AppPermission::ManageSystemSettings)
                .await?;
        }

        let rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        let detail = self.load_maintenance_rule_detail(rule_set).await?;

        if arming == MaintenanceEffectArming::Destructive {
            let descriptor = descriptor_for(detail.action_spec.kind);
            if descriptor.risk_class != MaintenanceRiskClass::High {
                return Err(AppError::Validation(format!(
                    "destructive arming applies only to rules whose action can delete files; \
                     this rule's action is {:?} risk",
                    descriptor.risk_class
                )));
            }
            let current = self
                .count_active_maintenance_candidates(&detail.rule_set.id)
                .await?;
            if acknowledged_candidate_count != Some(current) {
                // The web client parses the count out of this exact message to
                // re-present the confirmation; keep the shape stable.
                return Err(AppError::Validation(format!(
                    "destructive arming requires acknowledging the current candidate count ({current})"
                )));
            }
        }

        let now = Utc::now();
        self.services
            .customization
            .maintenance_rule_sets
            .update_rule_set_arming(&detail.rule_set.id, arming, now)
            .await?;

        let mut detail = detail;
        detail.rule_set.effect_arming = arming;
        detail.rule_set.updated_at = now;
        Ok(detail)
    }

    /// Non-terminal candidates of one rule — the number destructive arming
    /// makes the operator acknowledge.
    pub(crate) async fn count_active_maintenance_candidates(
        &self,
        rule_set_id: &str,
    ) -> AppResult<i64> {
        Ok(self
            .services
            .customization
            .maintenance_evaluation
            .count_candidates_by_state(rule_set_id)
            .await?
            .into_iter()
            .filter(|(state, _)| !state.is_terminal())
            .map(|(_, count)| count)
            .sum())
    }

    pub async fn list_maintenance_action_runs(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<MaintenanceActionRunView>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let limit = limit.unwrap_or(50).clamp(1, 200);
        let runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_runs(rule_set_id, candidate_id, Some(limit))
            .await?;
        let names = self
            .maintenance_title_names(runs.iter().map(|run| run.title_id.clone()))
            .await?;
        Ok(runs
            .into_iter()
            .map(|run| {
                let title_name = crate::maintenance_rules::evaluation::maintenance_title_name(
                    &names,
                    &run.title_id,
                );
                MaintenanceActionRunView { run, title_name }
            })
            .collect())
    }

    /// Manual trigger for the action handler, mirroring
    /// [`AppUseCase::run_maintenance_evaluation_now`].
    pub async fn run_maintenance_action_handler_now(
        &self,
        actor: &User,
    ) -> AppResult<MaintenanceEvaluationTrigger> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let gates = self.load_maintenance_gates().await?;
        if !gates.reversible_effects_enabled && !gates.destructive_effects_enabled {
            return Ok(MaintenanceEvaluationTrigger {
                started: false,
                message: Some(
                    "both instance effect gates are off, so no actions can execute".to_string(),
                ),
            });
        }

        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::LifecycleActionHandling)
            .await
        {
            return Ok(MaintenanceEvaluationTrigger {
                started: false,
                message: Some("an action-handling run is already in progress".to_string()),
            });
        }

        let app = self.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::LifecycleActionHandling, JobTriggerSource::Manual)
                .await
            {
                warn!(error = %error, "manual maintenance action-handling run failed");
            }
        });

        Ok(MaintenanceEvaluationTrigger {
            started: true,
            message: Some("Maintenance action handling started".to_string()),
        })
    }
}

// ── The handler ─────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Job body for [`JobKey::LifecycleActionHandling`].
    pub(crate) async fn run_lifecycle_action_handling_job(
        &self,
    ) -> AppResult<MaintenanceActionHandlingReport> {
        let mut report = MaintenanceActionHandlingReport::default();
        let gates = self.load_maintenance_gates().await?;
        if !gates.reversible_effects_enabled && !gates.destructive_effects_enabled {
            return Ok(report);
        }
        report.gates_enabled = true;

        let rule_sets = self
            .services
            .customization
            .maintenance_rule_sets
            .list_rule_sets()
            .await?;
        report.rules_considered = rule_sets.len();

        let libraries = self.maintenance_library_refs().await?;
        let now = Utc::now();
        let mut high_risk_executed = 0usize;
        let mut high_risk_failures = 0usize;

        for rule_set in rule_sets {
            if !rule_set.enabled
                || rule_set.evaluation_mode != MaintenanceEvaluationMode::Observe
                || rule_set.effect_arming == MaintenanceEffectArming::None
            {
                continue;
            }
            let detail = match self.load_maintenance_rule_detail(rule_set).await {
                Ok(detail) => detail,
                Err(error) => {
                    warn!(error = %error, "skipping maintenance rule with unusable revision");
                    continue;
                }
            };
            let Some(kind) = MaintenanceActionKind::parse_wire_str(&detail.revision_action_kind())
            else {
                warn!(
                    rule_set_id = detail.rule_set.id.as_str(),
                    "skipping maintenance rule with unrecognized action kind"
                );
                continue;
            };
            let descriptor = descriptor_for(kind);
            if descriptor.timing_mode == MaintenanceTimingMode::MembershipTracking {
                // do_nothing tracks membership; nothing ever becomes due.
                continue;
            }
            let high_risk = descriptor.risk_class == MaintenanceRiskClass::High;
            let eligible = if high_risk {
                gates.destructive_effects_enabled
                    && detail.rule_set.effect_arming == MaintenanceEffectArming::Destructive
            } else {
                gates.reversible_effects_enabled
                    && detail.rule_set.effect_arming != MaintenanceEffectArming::None
            };
            if !eligible {
                continue;
            }
            report.rules_eligible += 1;

            // One compile per rule; every candidate's fresh re-evaluation
            // shares it.
            let engine = match MaintenanceRulesEngine::build(&[MaintenancePolicy {
                id: detail.rule_set.id.clone(),
                name: detail.rule_set.name.clone(),
                rego_source: detail.revision.rego_source.clone(),
            }]) {
                Ok(engine) => engine,
                Err(error) => {
                    warn!(error = %error, "skipping maintenance rule whose matcher no longer compiles");
                    continue;
                }
            };

            let due = self
                .services
                .customization
                .maintenance_evaluation
                .list_due_candidates(
                    &detail.rule_set.id,
                    now,
                    MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN * 2,
                )
                .await?;

            let mut executed_this_rule = 0usize;
            for candidate in due {
                if executed_this_rule >= MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN {
                    break;
                }
                if high_risk
                    && (high_risk_executed >= MAINTENANCE_MAX_HIGH_RISK_ACTIONS_PER_RUN
                        || high_risk_failures >= MAINTENANCE_HIGH_RISK_FAILURE_BREAKER)
                {
                    break;
                }
                report.candidates_considered += 1;
                let outcome = self
                    .execute_one_maintenance_candidate(
                        &detail, kind, &engine, candidate, &libraries,
                    )
                    .await;
                match outcome {
                    Ok(CandidateOutcome::Executed) => {
                        report.executed += 1;
                        executed_this_rule += 1;
                        if high_risk {
                            high_risk_executed += 1;
                        }
                    }
                    Ok(CandidateOutcome::AlreadySatisfied) => {
                        report.already_satisfied += 1;
                        executed_this_rule += 1;
                    }
                    Ok(CandidateOutcome::Held) => report.held += 1,
                    Ok(CandidateOutcome::Canceled) => report.canceled += 1,
                    Ok(CandidateOutcome::Failed) => {
                        report.failed += 1;
                        executed_this_rule += 1;
                        if high_risk {
                            high_risk_failures += 1;
                        }
                    }
                    Ok(CandidateOutcome::LeaseLost) => report.lease_lost += 1,
                    Err(error) => {
                        // A bookkeeping failure for one candidate is bounded to
                        // that candidate; the pass keeps going.
                        report.failed += 1;
                        warn!(error = %error, "maintenance action handling failed for one candidate");
                    }
                }
            }
        }

        Ok(report)
    }

    /// Lease, recheck, and act on one candidate.
    async fn execute_one_maintenance_candidate(
        &self,
        detail: &MaintenanceRuleSetDetail,
        kind: MaintenanceActionKind,
        engine: &MaintenanceRulesEngine,
        candidate: LifecycleCandidate,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
    ) -> AppResult<CandidateOutcome> {
        let now = Utc::now();
        let candidates = &self.services.customization.maintenance_evaluation;

        // Selection bookkeeping, then the atomic lease. Only the lease decides
        // ownership; the Due transition is what makes it contestable.
        if candidate.state != MaintenanceCandidateState::Due {
            candidates
                .transition_candidate_state(
                    &candidate.id,
                    MaintenanceCandidateState::Due,
                    "due",
                    now,
                )
                .await?;
        }
        let stale_before = now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES);
        if !candidates
            .lease_candidate_for_execution(&candidate.id, stale_before, now)
            .await?
        {
            return Ok(CandidateOutcome::LeaseLost);
        }

        let attempt = candidate.action_attempts + 1;
        let execution_key = maintenance_action_idempotency_key(detail, &candidate);
        let run_id = Id::new().0;
        let mut run = LifecycleActionRun {
            id: run_id.clone(),
            candidate_id: candidate.id.clone(),
            rule_set_id: candidate.rule_set_id.clone(),
            revision_number: candidate.revision_number,
            title_id: candidate.title_id.clone(),
            action_kind: candidate.action_kind.clone(),
            match_generation: candidate.match_generation,
            // Filled in below: holds carry a per-row key so repeated re-checks
            // never collide, while the mutation path claims the execution key.
            idempotency_key: String::new(),
            attempt,
            status: LifecycleActionRunStatus::Running,
            hold_reason: None,
            error: None,
            detail: "{}".to_string(),
            started_at: now,
            finished_at: None,
            created_at: now,
        };

        let decision = self
            .maintenance_execution_safety_checks(engine, &candidate, libraries)
            .await;

        // Idempotency protects mutations, not evidence: a hold re-checks every
        // pass and each refusal is its own appended row, so hold rows key on
        // their own id. Only the path that is about to mutate claims the
        // execution key, whose (key, attempt) uniqueness is what makes a
        // crashed or concurrent duplicate attempt detectable.
        if matches!(decision, SafetyDecision::Proceed(_)) {
            run.idempotency_key = execution_key;
            if let Err(error) = candidates.start_action_run(&run).await {
                candidates
                    .transition_candidate_state(
                        &candidate.id,
                        MaintenanceCandidateState::Due,
                        "duplicate_attempt_detected",
                        Utc::now(),
                    )
                    .await?;
                warn!(error = %error, "duplicate maintenance action attempt refused");
                return Ok(CandidateOutcome::LeaseLost);
            }
        } else {
            run.idempotency_key = format!("hold:{run_id}");
            candidates.start_action_run(&run).await?;
        }

        let outcome = match decision {
            SafetyDecision::Hold(reason) => {
                self.finish_candidate_hold(&mut run, &candidate, reason)
                    .await?;
                CandidateOutcome::Held
            }
            SafetyDecision::Cancel(reason) => {
                run.status = LifecycleActionRunStatus::Held;
                run.hold_reason = Some(reason.to_string());
                run.finished_at = Some(Utc::now());
                candidates.finish_action_run(&run).await?;
                candidates
                    .transition_candidate_state(
                        &candidate.id,
                        MaintenanceCandidateState::Canceled,
                        reason,
                        Utc::now(),
                    )
                    .await?;
                CandidateOutcome::Canceled
            }
            SafetyDecision::Excluded => {
                run.status = LifecycleActionRunStatus::Held;
                run.hold_reason = Some(
                    crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED.to_string(),
                );
                run.finished_at = Some(Utc::now());
                candidates.finish_action_run(&run).await?;
                candidates
                    .transition_candidate_state(
                        &candidate.id,
                        MaintenanceCandidateState::Excluded,
                        crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED,
                        Utc::now(),
                    )
                    .await?;
                CandidateOutcome::Canceled
            }
            SafetyDecision::Proceed(title) => {
                match self
                    .execute_maintenance_action(detail, kind, &candidate, &title)
                    .await
                {
                    Ok(ActionResult::Executed { detail: evidence }) => {
                        run.status = LifecycleActionRunStatus::Succeeded;
                        run.detail = evidence.to_string();
                        run.finished_at = Some(Utc::now());
                        candidates.finish_action_run(&run).await?;
                        candidates
                            .transition_candidate_state(
                                &candidate.id,
                                MaintenanceCandidateState::Succeeded,
                                execution_reason::ACTION_SUCCEEDED,
                                Utc::now(),
                            )
                            .await?;
                        CandidateOutcome::Executed
                    }
                    Ok(ActionResult::AlreadySatisfied { detail: evidence }) => {
                        run.status = LifecycleActionRunStatus::AlreadySatisfied;
                        run.detail = evidence.to_string();
                        run.finished_at = Some(Utc::now());
                        candidates.finish_action_run(&run).await?;
                        candidates
                            .transition_candidate_state(
                                &candidate.id,
                                MaintenanceCandidateState::Succeeded,
                                execution_reason::ALREADY_SATISFIED,
                                Utc::now(),
                            )
                            .await?;
                        CandidateOutcome::AlreadySatisfied
                    }
                    Err(error) => {
                        let terminal = matches!(
                            error,
                            AppError::Validation(_)
                                | AppError::Unauthorized(_)
                                | AppError::NotFound(_)
                        ) || attempt >= MAINTENANCE_MAX_ACTION_ATTEMPTS;
                        run.status = LifecycleActionRunStatus::Failed;
                        run.error = Some(error.to_string());
                        run.finished_at = Some(Utc::now());
                        candidates.finish_action_run(&run).await?;
                        candidates
                            .record_candidate_attempts(&candidate.id, attempt, Utc::now())
                            .await?;
                        candidates
                            .transition_candidate_state(
                                &candidate.id,
                                if terminal {
                                    MaintenanceCandidateState::Failed
                                } else {
                                    MaintenanceCandidateState::Due
                                },
                                if terminal {
                                    execution_reason::ACTION_FAILED
                                } else {
                                    execution_reason::RETRY_PENDING
                                },
                                Utc::now(),
                            )
                            .await?;
                        CandidateOutcome::Failed
                    }
                }
            }
        };

        Ok(outcome)
    }

    async fn finish_candidate_hold(
        &self,
        run: &mut LifecycleActionRun,
        candidate: &LifecycleCandidate,
        reason: &'static str,
    ) -> AppResult<()> {
        let candidates = &self.services.customization.maintenance_evaluation;
        run.status = LifecycleActionRunStatus::Held;
        run.hold_reason = Some(reason.to_string());
        run.finished_at = Some(Utc::now());
        candidates.finish_action_run(run).await?;
        candidates
            .transition_candidate_state(
                &candidate.id,
                MaintenanceCandidateState::Blocked,
                reason,
                Utc::now(),
            )
            .await
    }

    /// The ordered pre-execution rechecks (RFC 8 step 3, RFC 9.10). Order
    /// matters: eligibility first so a disarmed rule never even re-evaluates,
    /// the matcher before the environment so "stopped matching" reads as a
    /// cancel rather than a hold, and the environment holds last.
    async fn maintenance_execution_safety_checks(
        &self,
        engine: &MaintenanceRulesEngine,
        candidate: &LifecycleCandidate,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
    ) -> SafetyDecision {
        // (1) Re-read the rule and the gates: both can move between selection
        // and execution, and a lowered gate must stop a leased worker before
        // its first external change (RFC section 8).
        let fresh = match self
            .require_maintenance_rule_set(&candidate.rule_set_id)
            .await
        {
            Ok(rule_set) => rule_set,
            Err(_) => return SafetyDecision::Cancel(execution_reason::RULE_NOT_ELIGIBLE),
        };
        let gates = match self.load_maintenance_gates().await {
            Ok(gates) => gates,
            Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
        };
        let Some(kind) = MaintenanceActionKind::parse_wire_str(&candidate.action_kind) else {
            return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE);
        };
        let high_risk = descriptor_for(kind).risk_class == MaintenanceRiskClass::High;
        let still_eligible = fresh.enabled
            && fresh.evaluation_mode == MaintenanceEvaluationMode::Observe
            && fresh.current_revision_number == candidate.revision_number
            && if high_risk {
                gates.destructive_effects_enabled
                    && fresh.effect_arming == MaintenanceEffectArming::Destructive
            } else {
                gates.reversible_effects_enabled
                    && fresh.effect_arming != MaintenanceEffectArming::None
            };
        if !still_eligible {
            return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE);
        }

        // (2) Exclusions recheck.
        match self
            .services
            .customization
            .maintenance_evaluation
            .list_exclusions(Some(&candidate.rule_set_id))
            .await
        {
            Ok(exclusions)
                if exclusions
                    .iter()
                    .any(|exclusion| exclusion.title_id == candidate.title_id) =>
            {
                return SafetyDecision::Excluded;
            }
            Ok(_) => {}
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        }

        // (3) Fresh matcher re-evaluation on current facts.
        let title = match self
            .services
            .catalog
            .titles
            .get_by_id(&candidate.title_id)
            .await
        {
            Ok(Some(title)) => title,
            Ok(None) => return SafetyDecision::Cancel(execution_reason::TITLE_MISSING),
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let files_by_title = match self
            .maintenance_files_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(files) => files,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let library = libraries
            .get(&title.library_id)
            .cloned()
            .unwrap_or_else(|| MaintenanceLibraryRef {
                id: title.library_id.clone(),
                name: String::new(),
            });
        let files = files_by_title
            .get(&title.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let input = build_title_input(Utc::now(), &title, &library, files);
        let outcome = engine
            .evaluator()
            .evaluate(&input)
            .ok()
            .and_then(|result| result.records.first().map(|record| record.decision.outcome));
        match outcome {
            Some(MaintenanceOutcome::Match) => {}
            Some(MaintenanceOutcome::NoMatch) => {
                return SafetyDecision::Cancel(execution_reason::NO_MATCH_AT_EXECUTION);
            }
            Some(MaintenanceOutcome::Unknown) | None => {
                return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
            }
        }

        // (4) Playback holds every action kind (RFC 9.10), unknown included.
        match self.maintenance_playback_hold().await {
            Ok(MaintenancePlaybackHold::Clear) => {}
            Ok(MaintenancePlaybackHold::Hold { .. }) => {
                return SafetyDecision::Hold(execution_reason::PLAYBACK_HOLD);
            }
            Ok(MaintenancePlaybackHold::Unknown { .. }) | Err(_) => {
                return SafetyDecision::Hold(execution_reason::PLAYBACK_UNKNOWN);
            }
        }

        // (5) Acquisition activity, unknown included.
        match self.title_has_active_acquisition(&candidate.title_id).await {
            Ok(MaintenanceActivityCheck::Clear) => {}
            Ok(MaintenanceActivityCheck::Active) => {
                return SafetyDecision::Hold(execution_reason::ACTIVE_ACQUISITION);
            }
            Ok(MaintenanceActivityCheck::Unknown { .. }) | Err(_) => {
                return SafetyDecision::Hold(execution_reason::ACQUISITION_UNKNOWN);
            }
        }

        SafetyDecision::Proceed(Box::new(title))
    }

    /// Perform the action through existing use cases, as the system actor.
    async fn execute_maintenance_action(
        &self,
        detail: &MaintenanceRuleSetDetail,
        kind: MaintenanceActionKind,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<ActionResult> {
        let actor = User::system_execution_actor();
        match kind {
            MaintenanceActionKind::DoNothing => {
                // Unreachable: MembershipTracking is filtered at selection.
                Ok(ActionResult::AlreadySatisfied {
                    detail: serde_json::json!({}),
                })
            }
            MaintenanceActionKind::UnmonitorScopeKeepFiles => {
                if !title.monitored {
                    return Ok(ActionResult::AlreadySatisfied {
                        detail: serde_json::json!({ "monitored": false }),
                    });
                }
                self.set_title_monitored(&actor, &title.id, false).await?;
                Ok(ActionResult::Executed {
                    detail: serde_json::json!({ "unmonitored": true }),
                })
            }
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
                self.execute_profile_change_and_search(detail, candidate, title, &actor)
                    .await
            }
            MaintenanceActionKind::DeleteTitleAndFiles => {
                let preview = self.preview_delete_title_files(&actor, &title.id).await?;
                let authorization = PolicyDeleteAuthorization {
                    rule_set_id: candidate.rule_set_id.clone(),
                    candidate_id: candidate.id.clone(),
                    revision_number: candidate.revision_number,
                };
                self.delete_title_by_policy(
                    &actor,
                    &title.id,
                    &preview.fingerprint,
                    &authorization,
                )
                .await?;
                Ok(ActionResult::Executed {
                    detail: serde_json::json!({
                        "deleted_title": true,
                        "preview_fingerprint": preview.fingerprint,
                        "media_count": preview.media_count,
                        "total_file_count": preview.total_file_count,
                    }),
                })
            }
            MaintenanceActionKind::UnmonitorTitleDeleteAllFiles => {
                // The unmonitor half is safe and runs; the title-preserving
                // bulk file deletion has no existing application seam, and this
                // module does not invent deletion paths. The candidate blocks
                // with a stable reason instead (flagged as an MVP gap).
                if title.monitored {
                    self.set_title_monitored(&actor, &title.id, false).await?;
                }
                Err(AppError::Validation(format!(
                    "{}: title-preserving file deletion is not implemented yet; \
                     the title was unmonitored and the candidate holds",
                    execution_reason::ACTION_NOT_FULLY_SUPPORTED
                )))
            }
            MaintenanceActionKind::UnmonitorShowDeleteExistingFiles
            | MaintenanceActionKind::UnmonitorScopeDeleteFiles
            | MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty
            | MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
                // Season/episode scopes cannot be authored for title rules;
                // a stored candidate carrying one is a contract violation.
                Err(AppError::Validation(format!(
                    "action {:?} is not executable for a title-scoped rule",
                    kind
                )))
            }
        }
    }

    /// RFC 9.3's combined profile workflow: change the profile only when it
    /// differs, then run the normal search once.
    ///
    /// Retry postcondition: when a prior attempt exists and the profile already
    /// equals the target, the profile step is treated as done and only the
    /// search is re-run — a first-attempt already-target subject instead
    /// reports `already_satisfied` with no search (the RFC's "searches only
    /// when the profile actually changes").
    async fn execute_profile_change_and_search(
        &self,
        detail: &MaintenanceRuleSetDetail,
        candidate: &LifecycleCandidate,
        title: &Title,
        actor: &User,
    ) -> AppResult<ActionResult> {
        let MaintenanceActionParameters::ChangeQualityProfile {
            target_quality_profile_id,
        } = &detail.action_spec.parameters
        else {
            return Err(AppError::Validation(
                "quality-profile action has no target profile".to_string(),
            ));
        };
        let target = target_quality_profile_id.trim();

        let profiles = self.load_quality_profile_settings().await?;
        if !profiles
            .profiles
            .iter()
            .any(|profile| crate::settings::runtime::quality_profile_ids_equal(&profile.id, target))
        {
            return Err(AppError::Validation(format!(
                "target quality profile '{target}' does not exist"
            )));
        }

        let current = title
            .tags
            .iter()
            .find_map(|tag| tag.strip_prefix(QUALITY_PROFILE_TAG_PREFIX))
            .map(str::trim);
        let already_target = current.is_some_and(|current| {
            crate::settings::runtime::quality_profile_ids_equal(current, target)
        });

        if already_target && candidate.action_attempts == 0 {
            return Ok(ActionResult::AlreadySatisfied {
                detail: serde_json::json!({ "quality_profile_id": target }),
            });
        }

        if !already_target {
            let mut tags: Vec<String> = title
                .tags
                .iter()
                .filter(|tag| !tag.starts_with(QUALITY_PROFILE_TAG_PREFIX))
                .cloned()
                .collect();
            tags.push(format!("{QUALITY_PROFILE_TAG_PREFIX}{target}"));
            self.update_title_metadata(actor, &title.id, None, None, Some(tags))
                .await?;
        }

        // The normal search path: cutoff upgrade when a file exists to
        // upgrade, missing otherwise.
        let has_file = !self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?
            .is_empty();
        self.start_acquisition_search_job(
            actor,
            crate::AcquisitionSearchRequest {
                wanted_kind: if has_file {
                    WantedKind::CutoffUpgrade
                } else {
                    WantedKind::Missing
                },
                facet: None,
                library_ids: Vec::new(),
                title_id: Some(title.id.clone()),
                season_number: None,
                wanted_item_id: None,
            },
        )
        .await?;

        Ok(ActionResult::Executed {
            detail: serde_json::json!({
                "quality_profile_id": target,
                "profile_changed": !already_target,
                "searched": true,
            }),
        })
    }
}

/// Stable across retries of one candidate generation and action; `attempt`
/// distinguishes retries (RFC 9.8's attempt key: action schema, revision,
/// match generation, subject, parameter hash — all of which are pinned by the
/// candidate id + generation + the revision's serialized action spec).
fn maintenance_action_idempotency_key(
    detail: &MaintenanceRuleSetDetail,
    candidate: &LifecycleCandidate,
) -> String {
    let params_hash = scryer_rules::runtime::content_hash(&detail.revision.action_spec_json);
    format!(
        "{}:{}:{}:{}",
        candidate.id,
        candidate.match_generation,
        candidate.action_kind,
        &params_hash[..16]
    )
}

impl MaintenanceRuleSetDetail {
    /// The action kind string the revision in force stores.
    fn revision_action_kind(&self) -> String {
        self.action_spec.kind.as_wire_str().to_string()
    }
}
