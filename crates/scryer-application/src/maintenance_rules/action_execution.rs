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

use std::collections::{HashMap, HashSet};

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
    MaintenanceLibraryRef, MaintenanceTitlePeople, MaintenanceTitleWatch,
    QUALITY_PROFILE_TAG_PREFIX, build_title_input,
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

/// How many of a candidate's action-run rows a reclaim scans looking for the
/// `running` row an interrupted attempt left behind. Attempts are capped at
/// [`MAINTENANCE_MAX_ACTION_ATTEMPTS`] and holds append rows of their own, so a
/// small bound covers every row that could still be open without reading a
/// candidate's whole history.
const MAINTENANCE_ORPHANED_RUN_SCAN_LIMIT: usize = 20;

/// The states the executor may move a selected candidate into `due` from.
///
/// `executing` is absent on purpose: a stranded lease is reclaimed by
/// [`MaintenanceCandidateRepository::lease_candidate_for_execution`]'s own stale
/// arm, which is the only writer allowed to decide that a lease is abandoned.
///
/// [`MaintenanceCandidateRepository::lease_candidate_for_execution`]: crate::ports::MaintenanceCandidateRepository::lease_candidate_for_execution
const EXECUTOR_DUE_EXPECTED_STATES: &[MaintenanceCandidateState] = &[
    MaintenanceCandidateState::Observing,
    MaintenanceCandidateState::PendingAction,
    MaintenanceCandidateState::Blocked,
];

/// The only state an executor's terminal write may move a candidate out of: the
/// lease it took itself. Zero rows affected means this worker no longer owns the
/// candidate — its lease went stale and was reclaimed while it worked — so the
/// pass records [`CandidateOutcome::LeaseLost`] and writes nothing further
/// rather than stamping a result over the new owner's.
const EXECUTOR_TERMINAL_EXPECTED_STATES: &[MaintenanceCandidateState] =
    &[MaintenanceCandidateState::Executing];

/// The action kinds [`AppUseCase::execute_maintenance_action`] actually
/// dispatches for a title-scoped rule.
///
/// This is the single source of truth for "can a title rule run this action":
/// authoring checks it (`validate_title_scope_action`) and the executor's match
/// refuses everything outside it. Before, the two disagreed —
/// `unmonitor_show_delete_existing_files` is show-subject, so it passed
/// authoring, and then hard-failed at execution three times into a terminal
/// `Failed` candidate. A kind is on this list only when the executor has a real
/// implementation for it; `unmonitor_title_delete_all_files` is on it because it
/// does dispatch (its file-deletion half is a declared MVP gap that reports
/// [`execution_reason::ACTION_NOT_FULLY_SUPPORTED`], not a missing arm).
///
/// The web rule builder mirrors this list —
/// `TITLE_EXECUTOR_UNSUPPORTED_ACTION_KINDS` in
/// `apps/scryer-web/lib/utils/maintenance-rule-sets.ts` — so an operator is
/// never offered an action the backend would refuse to save. Change one, change
/// the other.
pub const EXECUTABLE_TITLE_RULE_ACTIONS: &[MaintenanceActionKind] = &[
    MaintenanceActionKind::DoNothing,
    MaintenanceActionKind::UnmonitorScopeKeepFiles,
    MaintenanceActionKind::DeleteTitleAndFiles,
    MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
    MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
    MaintenanceActionKind::AddTags,
    MaintenanceActionKind::RemoveTags,
];

/// The refusal both the authoring check and the executor's rejection arm speak,
/// so an operator reads the same sentence wherever they hit the boundary.
pub(crate) fn title_rule_action_not_executable(kind: MaintenanceActionKind) -> String {
    format!(
        "this action cannot run for a title-scoped rule yet: {}",
        kind.as_wire_str()
    )
}

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
    /// A location operation owns the title (FR-084); the action retries once
    /// the operation releases it.
    pub const LOCATION_OPERATION_HOLD: &str = "location_operation_hold";
    /// The action completed.
    pub const ACTION_SUCCEEDED: &str = "action_succeeded";
    /// The postcondition already held; nothing was mutated.
    pub const ALREADY_SATISFIED: &str = "already_satisfied";
    /// A transient failure was recorded; the candidate is due again.
    pub const RETRY_PENDING: &str = "retry_pending";
    /// The attempt budget is exhausted or the failure was terminal.
    pub const ACTION_FAILED: &str = "action_failed";
    /// An attempt's worker stopped before it finished; the abandoned lease was
    /// reclaimed and its `running` action-run row closed out.
    pub const LEASE_RECLAIMED: &str = "lease_reclaimed";
    /// A tag the action writes is no longer in the title-tag registry — an
    /// administrator deleted it after the rule was authored. The candidate
    /// holds rather than failing: redefining the tag makes the rule work again,
    /// and writing a label nothing defines would put the catalog in a state the
    /// assignment path itself refuses.
    pub const TAG_NOT_DEFINED: &str = "tag_not_defined";
    /// Another rule in this same pass would write the opposite patch for the
    /// same label on this title. Both candidates hold: whichever ran last would
    /// win, so the outcome would depend on rule order rather than on what the
    /// operator asked for, and that is an authoring mistake to surface rather
    /// than a race to resolve.
    pub const TAG_PATCH_CONFLICT: &str = "tag_patch_conflict";
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
    ///
    /// Arming never outlives the revision it was granted against: what an
    /// operator acknowledged is one matcher's blast radius under one action, so
    /// appending a revision resets the rule to
    /// [`MaintenanceEffectArming::None`] and this call has to be repeated
    /// against the new matcher (see
    /// [`AppUseCase::update_maintenance_rule_matcher`]). Mode changes and
    /// metadata renames leave arming alone, because neither moves the matcher or
    /// the action.
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

        // Every rule that could run this pass is resolved first, in one place.
        // The tag-conflict scan below has to know what *other* rules are about
        // to write before the first of them writes anything, and resolving a
        // rule means reading its current revision — so the alternative is
        // reading every revision twice.
        let mut eligible_rules = Vec::new();
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
            eligible_rules.push((detail, kind));
        }

        // Titles two rules would tag in opposite directions this pass. Computed
        // before any of them acts, so both sides hold rather than the later one
        // silently undoing the earlier one.
        let tag_conflicts = self
            .maintenance_tag_patch_conflicts(&eligible_rules, now)
            .await;

        for (detail, kind) in eligible_rules {
            let descriptor = descriptor_for(kind);
            let high_risk = descriptor.risk_class == MaintenanceRiskClass::High;

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

            // The selection covers abandoned leases as well as due work: a
            // candidate whose worker died mid-run is `executing` with nothing
            // driving it, and only this pass can hand it back to the lease's
            // reclaim arm.
            let due = self
                .services
                .customization
                .maintenance_evaluation
                .list_due_candidates(
                    &detail.rule_set.id,
                    now,
                    now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
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
                        &detail,
                        kind,
                        &engine,
                        candidate,
                        &libraries,
                        &tag_conflicts,
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

    /// Title ids no tag action may write this pass, because two rules disagree
    /// about the same label on them.
    ///
    /// "Disagree" is exact: one rule's `add_tags` and another's `remove_tags`
    /// naming the same label, with both rules holding a due candidate for the
    /// same title. Two rules that add different labels, or that patch different
    /// titles, are not a conflict and both proceed — the bag is a set, and
    /// disjoint edits to a set commute.
    ///
    /// Nothing here resolves the disagreement. Whichever rule ran last would
    /// win, which makes the catalog depend on rule iteration order; the honest
    /// answer is to hold both sides with a reason the operator can act on.
    ///
    /// The scan re-lists due candidates for tag rules only. That is one extra
    /// bounded read per tag rule per pass, and it buys a decision made before
    /// the first write instead of after it.
    async fn maintenance_tag_patch_conflicts(
        &self,
        eligible_rules: &[(MaintenanceRuleSetDetail, MaintenanceActionKind)],
        now: chrono::DateTime<Utc>,
    ) -> HashSet<String> {
        let mut added: HashMap<String, HashSet<String>> = HashMap::new();
        let mut removed: HashMap<String, HashSet<String>> = HashMap::new();

        for (detail, kind) in eligible_rules {
            let labels = detail.action_spec.parameters.tag_labels();
            if labels.is_empty() {
                continue;
            }
            let due = match self
                .services
                .customization
                .maintenance_evaluation
                .list_due_candidates(
                    &detail.rule_set.id,
                    now,
                    now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES),
                    MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN * 2,
                )
                .await
            {
                Ok(due) => due,
                Err(error) => {
                    // An unreadable selection means this rule's contribution to
                    // the conflict picture is unknown. The pass still runs: the
                    // same read fails again in the main loop and holds the
                    // candidates there, with a reason of its own.
                    warn!(error = %error, "could not scan a tag rule for patch conflicts");
                    continue;
                }
            };
            let side = if *kind == MaintenanceActionKind::AddTags {
                &mut added
            } else {
                &mut removed
            };
            for candidate in due {
                side.entry(candidate.title_id)
                    .or_default()
                    .extend(labels.iter().cloned());
            }
        }

        added
            .iter()
            .filter(|(title_id, adds)| {
                removed
                    .get(*title_id)
                    .is_some_and(|removes| adds.iter().any(|label| removes.contains(label)))
            })
            .map(|(title_id, _)| title_id.clone())
            .collect()
    }

    /// Lease, recheck, and act on one candidate.
    async fn execute_one_maintenance_candidate(
        &self,
        detail: &MaintenanceRuleSetDetail,
        kind: MaintenanceActionKind,
        engine: &MaintenanceRulesEngine,
        candidate: LifecycleCandidate,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
        tag_conflicts: &HashSet<String>,
    ) -> AppResult<CandidateOutcome> {
        let now = Utc::now();
        let candidates = &self.services.customization.maintenance_evaluation;

        // A candidate arriving already `executing` is an abandoned lease the
        // selection reclaimed: it goes straight to the lease, whose stale arm is
        // the only writer allowed to decide a lease is dead. Everything else is
        // moved to `due` first — that transition is what makes ownership
        // contestable — and a lost compare-and-set there means another writer
        // reached the row between selection and now.
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
        let stale_before = now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES);
        if !candidates
            .lease_candidate_for_execution(&candidate.id, stale_before, now)
            .await?
        {
            return Ok(CandidateOutcome::LeaseLost);
        }

        if reclaiming {
            // The interrupted attempt left a `running` row that will never be
            // finished by anyone else. Close it out before this attempt starts,
            // so the candidate's history reads as one abandoned attempt followed
            // by a new one rather than two attempts that both appear live.
            self.finalize_orphaned_maintenance_action_runs(&candidate.id)
                .await?;
        }

        // Attempts are counted as reservations (see below), so a candidate that
        // crash-looped through its budget is terminal here rather than being
        // retried forever — the failure path that would normally notice never
        // got to run.
        let attempt = candidate.action_attempts + 1;
        if attempt > MAINTENANCE_MAX_ACTION_ATTEMPTS {
            return Ok(
                if self
                    .finish_maintenance_candidate(
                        &candidate.id,
                        MaintenanceCandidateState::Failed,
                        execution_reason::ACTION_FAILED,
                    )
                    .await?
                {
                    CandidateOutcome::Failed
                } else {
                    CandidateOutcome::LeaseLost
                },
            );
        }
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
            .maintenance_execution_safety_checks(
                detail,
                engine,
                &candidate,
                libraries,
                tag_conflicts,
            )
            .await;

        // Idempotency protects mutations, not evidence: a hold re-checks every
        // pass and each refusal is its own appended row, so hold rows key on
        // their own id. Only the path that is about to mutate claims the
        // execution key, whose (key, attempt) uniqueness is what makes a
        // crashed or concurrent duplicate attempt detectable.
        if matches!(decision, SafetyDecision::Proceed(_)) {
            // The attempt is *reserved* here — durably, and strictly before the
            // run row and any external side effect. A counter only advanced
            // after a handled failure leaves an interrupted attempt invisible:
            // the reclaiming pass would compute the same attempt number, collide
            // with the orphaned `(idempotency_key, attempt)` row, and hand the
            // candidate back to `due` forever. Reserving first also makes the
            // attempt cap bound a crash loop, not just a failure loop.
            candidates
                .record_candidate_attempts(&candidate.id, attempt, Utc::now())
                .await?;
            run.idempotency_key = execution_key;
            if let Err(error) = candidates.start_action_run(&run).await {
                candidates
                    .transition_candidate_state(
                        &candidate.id,
                        MaintenanceCandidateState::Due,
                        "duplicate_attempt_detected",
                        EXECUTOR_TERMINAL_EXPECTED_STATES,
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

        // Every terminal write below is a compare-and-set against this worker's
        // own lease: `false` means the lease went stale and was reclaimed while
        // the action ran, so the pass reports `LeaseLost` and leaves the new
        // owner's row alone. The action-run row is still finished either way —
        // it is this worker's evidence of what it did, and that stays true no
        // matter who owns the candidate now.
        let outcome = match decision {
            SafetyDecision::Hold(reason) => {
                if self
                    .finish_candidate_hold(&mut run, &candidate, reason)
                    .await?
                {
                    CandidateOutcome::Held
                } else {
                    CandidateOutcome::LeaseLost
                }
            }
            SafetyDecision::Cancel(reason) => {
                run.status = LifecycleActionRunStatus::Held;
                run.hold_reason = Some(reason.to_string());
                run.finished_at = Some(Utc::now());
                candidates.finish_action_run(&run).await?;
                if self
                    .finish_maintenance_candidate(
                        &candidate.id,
                        MaintenanceCandidateState::Canceled,
                        reason,
                    )
                    .await?
                {
                    CandidateOutcome::Canceled
                } else {
                    CandidateOutcome::LeaseLost
                }
            }
            SafetyDecision::Excluded => {
                run.status = LifecycleActionRunStatus::Held;
                run.hold_reason = Some(
                    crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED.to_string(),
                );
                run.finished_at = Some(Utc::now());
                candidates.finish_action_run(&run).await?;
                if self
                    .finish_maintenance_candidate(
                        &candidate.id,
                        MaintenanceCandidateState::Excluded,
                        crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED,
                    )
                    .await?
                {
                    CandidateOutcome::Canceled
                } else {
                    CandidateOutcome::LeaseLost
                }
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
                        if self
                            .finish_maintenance_candidate(
                                &candidate.id,
                                MaintenanceCandidateState::Succeeded,
                                execution_reason::ACTION_SUCCEEDED,
                            )
                            .await?
                        {
                            CandidateOutcome::Executed
                        } else {
                            CandidateOutcome::LeaseLost
                        }
                    }
                    Ok(ActionResult::AlreadySatisfied { detail: evidence }) => {
                        run.status = LifecycleActionRunStatus::AlreadySatisfied;
                        run.detail = evidence.to_string();
                        run.finished_at = Some(Utc::now());
                        candidates.finish_action_run(&run).await?;
                        if self
                            .finish_maintenance_candidate(
                                &candidate.id,
                                MaintenanceCandidateState::Succeeded,
                                execution_reason::ALREADY_SATISFIED,
                            )
                            .await?
                        {
                            CandidateOutcome::AlreadySatisfied
                        } else {
                            CandidateOutcome::LeaseLost
                        }
                    }
                    Err(error) => {
                        // The attempt was already reserved before the action
                        // ran, so nothing is counted here — only the verdict on
                        // the budget it consumed.
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
                        if self
                            .finish_maintenance_candidate(
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
                            )
                            .await?
                        {
                            CandidateOutcome::Failed
                        } else {
                            CandidateOutcome::LeaseLost
                        }
                    }
                }
            }
        };

        Ok(outcome)
    }

    /// Returns whether this worker still held the lease when it wrote the hold.
    async fn finish_candidate_hold(
        &self,
        run: &mut LifecycleActionRun,
        candidate: &LifecycleCandidate,
        reason: &'static str,
    ) -> AppResult<bool> {
        let candidates = &self.services.customization.maintenance_evaluation;
        run.status = LifecycleActionRunStatus::Held;
        run.hold_reason = Some(reason.to_string());
        run.finished_at = Some(Utc::now());
        candidates.finish_action_run(run).await?;
        self.finish_maintenance_candidate(&candidate.id, MaintenanceCandidateState::Blocked, reason)
            .await
    }

    /// Write a candidate's post-execution state, but only while this worker
    /// still owns the lease it took. `false` means it does not: another pass
    /// reclaimed the candidate as stale, and this worker must stop writing.
    async fn finish_maintenance_candidate(
        &self,
        candidate_id: &str,
        state: MaintenanceCandidateState,
        reason: &str,
    ) -> AppResult<bool> {
        let moved = self
            .services
            .customization
            .maintenance_evaluation
            .transition_candidate_state(
                candidate_id,
                state,
                reason,
                EXECUTOR_TERMINAL_EXPECTED_STATES,
                Utc::now(),
            )
            .await?;
        if !moved {
            warn!(
                candidate_id,
                "maintenance execution lease was lost before its result could be recorded"
            );
        }
        Ok(moved)
    }

    /// Close out the `running` action-run rows an interrupted attempt left for
    /// this candidate.
    ///
    /// A row stuck at `running` is not evidence of work in flight — the worker
    /// that wrote it is gone — so it is finished as failed with a stable
    /// [`execution_reason::LEASE_RECLAIMED`] message. Only rows already stored
    /// are touched: the reclaiming attempt inserts its own row afterwards, at
    /// the next attempt number, which is what keeps it clear of the orphan's
    /// `(idempotency_key, attempt)` uniqueness.
    async fn finalize_orphaned_maintenance_action_runs(&self, candidate_id: &str) -> AppResult<()> {
        let candidates = &self.services.customization.maintenance_evaluation;
        let orphans: Vec<LifecycleActionRun> = candidates
            .list_action_runs(
                None,
                Some(candidate_id),
                Some(MAINTENANCE_ORPHANED_RUN_SCAN_LIMIT),
            )
            .await?
            .into_iter()
            .filter(|run| run.status == LifecycleActionRunStatus::Running)
            .collect();
        for mut orphan in orphans {
            orphan.status = LifecycleActionRunStatus::Failed;
            orphan.error = Some(format!(
                "{}: the worker holding this attempt stopped before it finished",
                execution_reason::LEASE_RECLAIMED
            ));
            orphan.finished_at = Some(Utc::now());
            candidates.finish_action_run(&orphan).await?;
        }
        Ok(())
    }

    /// The ordered pre-execution rechecks (RFC 8 step 3, RFC 9.10). Order
    /// matters: eligibility first so a disarmed rule never even re-evaluates,
    /// the matcher before the environment so "stopped matching" reads as a
    /// cancel rather than a hold, and the environment holds last.
    async fn maintenance_execution_safety_checks(
        &self,
        detail: &MaintenanceRuleSetDetail,
        engine: &MaintenanceRulesEngine,
        candidate: &LifecycleCandidate,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
        tag_conflicts: &HashSet<String>,
    ) -> SafetyDecision {
        // (1) Re-read the rule and the gates: both can move between selection
        // and execution, and a lowered gate must stop a leased worker before
        // its first external change (RFC section 8).
        let fresh = match self
            .require_maintenance_rule_set(&candidate.rule_set_id)
            .await
        {
            Ok(rule_set) => rule_set,
            // Only a rule that is genuinely gone cancels: the candidate's
            // authorization no longer exists, so there is nothing to re-check
            // later. Every other error is the repository being unreachable,
            // which says nothing about the rule — it holds, like every sibling
            // check on this path.
            Err(AppError::NotFound(_)) => {
                return SafetyDecision::Cancel(execution_reason::RULE_NOT_ELIGIBLE);
            }
            Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
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

        // (1b) Tag actions: the vocabulary has to still exist, and no sibling
        // rule may be about to write the opposite patch on this title.
        //
        // Both checks belong here rather than in the action itself, because a
        // hold from here costs no attempt: these are conditions that a later
        // pass can find changed — an administrator redefines the tag, or the
        // conflicting rule is disarmed — and burning the three-attempt budget
        // on them would turn a fixable authoring problem into a terminally
        // failed candidate.
        let tag_labels = detail.action_spec.parameters.tag_labels();
        if !tag_labels.is_empty() {
            if tag_conflicts.contains(&candidate.title_id) {
                return SafetyDecision::Hold(execution_reason::TAG_PATCH_CONFLICT);
            }
            match self.undefined_title_tag_labels(tag_labels).await {
                Ok(undefined) if !undefined.is_empty() => {
                    return SafetyDecision::Hold(execution_reason::TAG_NOT_DEFINED);
                }
                Ok(_) => {}
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
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
        // The rule's library scope is re-checked against the *fresh* rule and the
        // *fresh* title, for the same reason its arming is: scope is the blast
        // radius an operator acknowledged, and it can be narrowed — or the title
        // moved — between the candidate being opened and this pass. Acting
        // anyway is acting outside the acknowledged scope. An empty scope is
        // instance-wide, which covers every library by definition.
        if !fresh.library_ids.is_empty() && !fresh.library_ids.contains(&title.library_id) {
            return SafetyDecision::Cancel(
                crate::maintenance_rules::evaluation::candidate_reason::OUT_OF_SCOPE,
            );
        }
        // (3b) A location operation that owns the title (FR-084). Destructive
        // work waits for it the way it waits for playback and acquisition
        // activity: the operation releases its claim on every terminal path,
        // so this is a retry, not a verdict. An unreadable claim store holds
        // the same way — acting on a claim Scryer could not read is acting on
        // evidence it never had.
        if high_risk {
            match self
                .location_ownership_denial_for_title(
                    &crate::location::ownership_guard::MAINTENANCE_ACTION_ENTRY,
                    &title.id,
                )
                .await
            {
                Ok(None) => {}
                Ok(Some(_)) => {
                    return SafetyDecision::Hold(execution_reason::LOCATION_OPERATION_HOLD);
                }
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        }
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
        // The same series-movie load the evaluator does, so the re-evaluation
        // at execution time reads the identical fact document.
        let series_movies_by_title = match self
            .maintenance_series_movies_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(series_movies) => series_movies,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        // A people lookup that fails holds, like every other unresolvable
        // signal on this path: re-evaluating on a fact snapshot Scryer could
        // not fully assemble is how an action fires on evidence it never had.
        let requesters_by_title = match self
            .maintenance_requesters_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(requesters) => requesters,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let usernames = match self.maintenance_usernames_by_id().await {
            Ok(usernames) => usernames,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        // Watch signals are resolved on the same fail-closed terms: a gate or
        // signal read that errors holds, and a gate that is merely closed makes
        // every watch fact unknown, which holds any rule that reads one.
        let watch_context = match self.maintenance_watch_context().await {
            Ok(context) => context,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let signals_by_title = match self
            .maintenance_watch_signals_for_titles(&watch_context, std::slice::from_ref(&title))
            .await
        {
            Ok(signals) => signals,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let people = MaintenanceTitlePeople {
            requester_user_ids: requesters_by_title.get(&title.id).map(Vec::as_slice),
            usernames: &usernames,
        };
        let watch = MaintenanceTitleWatch {
            context: &watch_context,
            signals: signals_by_title.get(&title.id).map(Vec::as_slice),
        };
        let input = build_title_input(
            Utc::now(),
            &title,
            &library,
            files,
            people,
            watch,
            series_movies_by_title
                .get(&title.id)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        );
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
            MaintenanceActionKind::AddTags | MaintenanceActionKind::RemoveTags => {
                self.execute_tag_patch(detail, kind, title, &actor).await
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
                // Exactly the complement of `EXECUTABLE_TITLE_RULE_ACTIONS`, and
                // the authoring check now refuses the same set, so a rule
                // carrying one of these can no longer be saved. Rules stored
                // before that check existed still land here, which is why the
                // arm stays: a stored candidate carrying an undispatched action
                // is refused rather than silently reinterpreted. The match is
                // deliberately exhaustive so a new catalog kind is a compile
                // error until it is placed on one side or the other.
                Err(AppError::Validation(title_rule_action_not_executable(kind)))
            }
        }
    }

    /// Apply the rule's tag patch to the subject title.
    ///
    /// One `update_title_tags` call as the system actor, which is the same
    /// application operation the tag picker and the bulk dialog drive: the
    /// registry gate, the normalization, the reserved-namespace guard, the
    /// per-title ceiling, and the in-transaction merge that keeps a concurrent
    /// options save from clobbering the bag are all that call's, not
    /// reimplemented here.
    ///
    /// `already_satisfied` is a real postcondition check rather than a
    /// first-attempt special case: the action is `ensure_state`, so "every
    /// label is already on (or already off) the title" is exactly the state it
    /// exists to ensure, and reporting it as executed would claim a mutation
    /// that did not happen.
    async fn execute_tag_patch(
        &self,
        detail: &MaintenanceRuleSetDetail,
        kind: MaintenanceActionKind,
        title: &Title,
        actor: &User,
    ) -> AppResult<ActionResult> {
        let MaintenanceActionParameters::Tags { tags } = &detail.action_spec.parameters else {
            return Err(AppError::Validation(
                "tag action has no tags configured".to_string(),
            ));
        };
        let adding = kind == MaintenanceActionKind::AddTags;
        let present = |label: &String| title.tags.iter().any(|tag| tag == label);
        let pending: Vec<String> = tags
            .iter()
            .filter(|label| {
                if adding {
                    !present(label)
                } else {
                    present(label)
                }
            })
            .cloned()
            .collect();

        if pending.is_empty() {
            return Ok(ActionResult::AlreadySatisfied {
                detail: serde_json::json!({
                    "tags": tags,
                    "operation": kind.as_wire_str(),
                }),
            });
        }

        let (add, remove): (&[String], &[String]) = if adding {
            (&pending, &[])
        } else {
            (&[], &pending)
        };
        self.update_title_tags(actor, std::slice::from_ref(&title.id), add, remove)
            .await?;

        Ok(ActionResult::Executed {
            detail: serde_json::json!({
                "tags": tags,
                "changed_tags": pending,
                "operation": kind.as_wire_str(),
            }),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance_rules::MaintenanceActionSpec;
    use crate::maintenance_rules::service::validate_title_scope_action;

    /// The kinds spelled out in `execute_maintenance_action`'s rejection arm,
    /// restated here literally. This list and that arm are edited together; the
    /// test below is what makes a one-sided edit fail.
    const EXECUTOR_REJECTION_ARM_KINDS: &[MaintenanceActionKind] = &[
        MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
        MaintenanceActionKind::UnmonitorScopeDeleteFiles,
        MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
        MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty,
    ];

    fn spec_for(kind: MaintenanceActionKind) -> MaintenanceActionSpec {
        match kind {
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
                MaintenanceActionSpec::change_quality_profile("profile-1")
            }
            MaintenanceActionKind::AddTags | MaintenanceActionKind::RemoveTags => {
                MaintenanceActionSpec::tags(kind, vec!["needs review".to_string()])
            }
            _ => MaintenanceActionSpec::new(kind),
        }
    }

    /// Every catalog kind is either dispatched by the title executor or refused
    /// by it, and authoring answers identically for every one of them.
    ///
    /// This is the mechanical link the `unmonitor_show_delete_existing_files`
    /// trap needed: it was savable (show-subject, so the descriptor check passed)
    /// and then unconditionally refused at execution, so a rule using it failed
    /// three attempts into a terminal `Failed` and never recovered.
    #[test]
    fn authoring_and_the_title_executor_agree_on_every_action_kind() {
        for kind in MaintenanceActionKind::ALL.iter().copied() {
            let dispatched = EXECUTABLE_TITLE_RULE_ACTIONS.contains(&kind);
            let refused = EXECUTOR_REJECTION_ARM_KINDS.contains(&kind);
            assert_ne!(
                dispatched, refused,
                "{kind:?} must appear on exactly one of the executor's two sides"
            );

            let authored = validate_title_scope_action(&spec_for(kind)).is_ok();
            assert_eq!(
                authored, dispatched,
                "{kind:?}: authoring must accept exactly what the executor dispatches"
            );
        }

        assert_eq!(
            EXECUTABLE_TITLE_RULE_ACTIONS.len() + EXECUTOR_REJECTION_ARM_KINDS.len(),
            MaintenanceActionKind::ALL.len(),
            "the two sides must together cover the closed catalog exactly once"
        );
    }

    /// The specific regression: the show-subject delete action is savable no
    /// longer, and the refusal says so in words an operator can act on.
    #[test]
    fn a_show_scoped_delete_action_is_refused_at_authoring_time() {
        let refused = validate_title_scope_action(&spec_for(
            MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
        ))
        .expect_err("an action the executor cannot dispatch must not be savable");
        assert!(
            refused
                .to_string()
                .contains("cannot run for a title-scoped rule"),
            "{refused}"
        );
    }
}
