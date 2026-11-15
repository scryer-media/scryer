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
    AppPermission, LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MaintenanceActionJobReceipt, MaintenanceActionStepRun, MaintenanceCandidateState,
    MaintenanceEffectArming, MaintenanceEvaluationMode, MaintenanceRuleState, Title, User,
};
use scryer_rules::maintenance::{
    MaintenanceOutcome, MaintenancePolicy, MaintenanceRulesEngine, MaintenanceRulesEvaluator,
};
use serde::Serialize;
use tracing::warn;

use crate::jobs::{JobKey, JobTriggerSource};
use crate::maintenance_rules::action_catalog::{
    MaintenanceRiskClass, MaintenanceSubjectKind as ActionSubjectKind,
};
use crate::maintenance_rules::action_sequence::{
    ACTION_SEQUENCE_KIND, MaintenanceActionSequence, MaintenanceActionStepKind,
};
use crate::maintenance_rules::declared_actions::MaintenanceDeclaredActions;
use crate::maintenance_rules::evaluation::MaintenanceEvaluationTrigger;
use crate::maintenance_rules::facts::{
    MaintenanceLibraryRef, MaintenanceTitleClaims, MaintenanceTitlePeople, MaintenanceTitleWatch,
    build_subject_input_with_completed_retention_episodes,
};
use crate::maintenance_rules::safety::{MaintenanceActivityCheck, MaintenancePlaybackHold};
use crate::maintenance_rules::sequence_execution::SequenceCandidateExecutionContext;
use crate::maintenance_rules::sequence_history::maintenance_sequence_history_steps;
use crate::maintenance_rules::service::MaintenanceRuleSetDetail;
use crate::ports::MaintenanceActionStepCandidateKey;
use crate::{AppError, AppResult, AppUseCase};

/// Due candidates read per page while a pass walks one rule. The walk goes on
/// page after page until the rule's own maximum is reached or nothing due is
/// left; the page only bounds how many rows are held at once.
pub(super) const MAINTENANCE_DUE_CANDIDATE_PAGE_SIZE: usize = 200;

/// High-risk failures in one pass that stop further high-risk work that pass —
/// a circuit breaker against a systematically failing deletion path.
pub const MAINTENANCE_HIGH_RISK_FAILURE_BREAKER: usize = 3;

/// The wire names of the high-risk step kinds.
pub(super) fn high_risk_step_kinds() -> Vec<&'static str> {
    MaintenanceActionStepKind::ALL
        .iter()
        .copied()
        .filter(|kind| kind.descriptor().risk_class == MaintenanceRiskClass::High)
        .map(MaintenanceActionStepKind::as_wire_str)
        .collect()
}

/// How far back another rule's monitoring change keeps this rule from
/// undoing it. Two rules that disagree about a title could otherwise flip it
/// on every cycle; within this window the second one holds.
pub const MAINTENANCE_MONITORING_CONFLICT_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

/// The most (rule, title, change) rows one monitoring-conflict scan reads. A
/// scan that reaches it cannot rule a conflict out, so it holds instead.
const MAINTENANCE_MONITORING_CONFLICT_SCAN_LIMIT: usize = 5_000;

/// Which way a rule, or one candidate's steps, can turn a title's
/// monitoring: on, or off (unmonitoring, or deleting its files).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MonitoringSides {
    monitors: bool,
    unmonitors: bool,
}

impl MonitoringSides {
    fn declared(declared: &MaintenanceDeclaredActions) -> Self {
        use scryer_rules::maintenance_actions::MaintenanceActionHead as Head;
        Self {
            monitors: declared.heads.contains(&Head::Monitor),
            unmonitors: declared.heads.iter().any(|head| {
                matches!(
                    head,
                    Head::Unmonitor | Head::DeleteFiles | Head::DeleteTitle
                )
            }),
        }
    }

    fn of_sequence(sequence: &MaintenanceActionSequence) -> Self {
        Self {
            monitors: sequence
                .steps
                .iter()
                .any(|step| step.kind == MaintenanceActionStepKind::Monitor),
            unmonitors: sequence.steps.iter().any(|step| {
                matches!(
                    step.kind,
                    MaintenanceActionStepKind::Unmonitor
                        | MaintenanceActionStepKind::DeleteFiles
                        | MaintenanceActionStepKind::DeleteTitleAndFiles
                )
            }),
        }
    }

    fn opposes(&self, other: &Self) -> bool {
        (self.monitors && other.unmonitors) || (self.unmonitors && other.monitors)
    }
}

/// The hold for a candidate whose steps would undo another rule's recent
/// monitoring change on the same title, if any.
fn monitoring_conflict(
    acts: &Option<Vec<crate::MaintenanceStepAct>>,
    candidate: &LifecycleCandidate,
    sequence: &MaintenanceActionSequence,
) -> Option<&'static str> {
    let own = MonitoringSides::of_sequence(sequence);
    if !own.monitors && !own.unmonitors {
        return None;
    }
    let Some(acts) = acts else {
        return Some(execution_reason::UNKNOWN_AT_EXECUTION);
    };
    let monitor = MaintenanceActionStepKind::Monitor.as_wire_str();
    acts.iter()
        .filter(|act| {
            act.title_id == candidate.title_id && act.rule_set_id != candidate.rule_set_id
        })
        .any(|act| {
            let theirs = MonitoringSides {
                monitors: act.step_kind == monitor,
                unmonitors: act.step_kind != monitor,
            };
            own.opposes(&theirs)
        })
        .then_some(execution_reason::MONITORING_CONFLICT)
}

/// Failed attempts before a candidate is marked terminally failed.
pub const MAINTENANCE_MAX_ACTION_ATTEMPTS: i64 = 3;

/// An `executing` lease older than this is considered abandoned by a crashed
/// run and may be re-leased.
pub(super) const EXECUTION_LEASE_STALE_AFTER_MINUTES: i64 = 60;

impl AppUseCase {
    /// Leases last touched before this are abandoned: older than
    /// [`EXECUTION_LEASE_STALE_AFTER_MINUTES`], or taken before this process
    /// started. A worker of this process always touches its lease after the
    /// start, so a lease from before it has no worker, and a sequence a
    /// restart interrupted resumes at the rule's first fire instead of an
    /// hour later.
    pub(super) fn maintenance_lease_stale_before(
        &self,
        now: chrono::DateTime<Utc>,
    ) -> chrono::DateTime<Utc> {
        (now - Duration::minutes(EXECUTION_LEASE_STALE_AFTER_MINUTES))
            .max(self.runtime.jobs.process_started_at)
    }
}

/// The states the executor may move a selected candidate into `due` from.
///
/// `executing` is absent on purpose: a stranded lease is reclaimed by
/// [`MaintenanceCandidateRepository::lease_candidate_for_execution`]'s own stale
/// arm, which is the only writer allowed to decide that a lease is abandoned.
///
/// [`MaintenanceCandidateRepository::lease_candidate_for_execution`]: crate::ports::MaintenanceCandidateRepository::lease_candidate_for_execution
pub(super) const EXECUTOR_DUE_EXPECTED_STATES: &[MaintenanceCandidateState] = &[
    MaintenanceCandidateState::Observing,
    MaintenanceCandidateState::PendingAction,
    MaintenanceCandidateState::Blocked,
];

/// Stable `hold_reason` / `state_reason` values the executor writes. Part of
/// the operator-visible contract, like `candidate_reason` on the evaluator.
///
/// These are stored on candidate and action-run rows and read by the web
/// client, and they shipped as lower snake case before the area-prefixed
/// SCREAMING_SNAKE convention existed. A released code is never renamed, so
/// this module keeps that one shape, and its test holds every code to it.
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
    /// A rule that reads a watch fact matched, and reading the subject's
    /// items back from the media server found a play newer than the stored
    /// facts, by an account the watch facts count.
    pub const WATCH_RECHECK_NEWER_PLAY: &str = "watch_recheck_newer_play";
    /// The media server's history names a play of the subject that the
    /// stored facts credit to no account, such as one by an account that is
    /// not linked to a Scryer user.
    pub const WATCH_RECHECK_UNCREDITED_PLAY: &str = "watch_recheck_uncredited_play";
    /// The subject's items could not be read back from the media server
    /// before acting on a watch fact.
    pub const WATCH_RECHECK_UNAVAILABLE: &str = "watch_recheck_unavailable";
    /// A connected media server's library mapping has no item for the
    /// subject (or one of its episodes with a file), so it could not be read
    /// back there and no read was attempted.
    pub const WATCH_RECHECK_UNMAPPED: &str = "watch_recheck_unmapped";
    /// The title has a grab, download, or import in flight.
    pub const ACTIVE_ACQUISITION: &str = "active_acquisition";
    /// Acquisition activity could not be ruled out.
    pub const ACQUISITION_UNKNOWN: &str = "acquisition_unknown";
    /// A storage-pressure deletion owns the conservative global filesystem
    /// guard. This is a hold because another worker will release it.
    pub const STORAGE_DELETION_IN_PROGRESS: &str = "storage_deletion_in_progress";
    /// The action needs an application seam that does not exist yet
    /// (`unmonitor_title_delete_all_files`'s title-preserving file deletion).
    pub const ACTION_NOT_FULLY_SUPPORTED: &str = "action_not_fully_supported";
    /// The subject no longer exists in the catalog.
    pub const TITLE_MISSING: &str = "title_missing";
    /// A location operation owns the title (FR-084); the action retries once
    /// the operation releases it.
    pub const LOCATION_OPERATION_HOLD: &str = "location_operation_hold";
    /// This rule's per-pass action-step allowance was spent by earlier
    /// sequence steps. The candidate remains blocked for the next pass.
    pub const PASS_ACTION_STEP_BUDGET: &str = "pass_action_step_budget";
    /// The shared high-risk allowance was spent by earlier destructive
    /// sequence steps. Reversible predecessors may still run in a later pass.
    pub const PASS_HIGH_RISK_STEP_BUDGET: &str = "pass_high_risk_step_budget";
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
    /// Another rule recently did the opposite to this title's monitoring:
    /// it monitored the title this rule would unmonitor or delete from, or
    /// unmonitored or deleted from the title this rule would monitor. Acting
    /// would undo the other rule's work, and the two would flip the title
    /// every cycle, so whichever rule would act next holds instead.
    pub const MONITORING_CONFLICT: &str = "monitoring_conflict";
    /// A high-risk action on a season or episode subject whose files cannot
    /// all be placed on that subject's episodes.
    pub const AMBIGUOUS_EPISODE_FILE_COVERAGE: &str = "ambiguous_episode_file_coverage";
    /// The quality profile the rule decided names no single configured
    /// profile now: it was renamed or removed after the rule was saved, or two
    /// profiles share the name.
    pub const QUALITY_PROFILE_UNRESOLVED: &str = "quality_profile_unresolved";
    /// What the rule decided for this subject cannot run as one sequence, such
    /// as adding and removing the same tag, or a whole-title action decided
    /// for a season or episode.
    pub const ACTIONS_NOT_RUNNABLE: &str = "actions_not_runnable";
    /// Fresh re-evaluation still matches but decides different actions than
    /// the candidate carries. The next evaluation pass replaces the candidate.
    pub const ACTIONS_CHANGED_AT_EXECUTION: &str = "actions_changed_at_execution";
    /// The candidate carries an action its rule's current revision does not
    /// declare. Nothing outside what the stored source can decide ever runs.
    pub const ACTION_NOT_DECLARED: &str = "action_not_declared";
    /// The candidate's steps include one a storage-root rule cannot run.
    pub const STORAGE_ROOT_ACTION_NOT_ALLOWED: &str = "storage_root_action_not_allowed";
    /// The rule decided actions that cannot be applied together, such as
    /// monitoring and unmonitoring, both moves, or a move and a delete.
    /// Nothing is applied.
    pub const ACTIONS_CONFLICT: &str = "actions_conflict";
    /// The library a move names is not one configured library of the title's
    /// kind now: it was renamed or removed after the rule was saved, or two
    /// libraries share the name.
    pub const MOVE_LIBRARY_UNRESOLVED: &str = "move_library_unresolved";
    /// The library a move names holds another kind of title.
    pub const MOVE_LIBRARY_FACET_MISMATCH: &str = "move_library_facet_mismatch";
    /// The root a move names is not a root of the title's library.
    pub const MOVE_ROOT_NOT_IN_LIBRARY: &str = "move_root_not_in_library";
    /// The move's plan was refused, or it went stale between the preview and
    /// the start. The files stay where they are and the step retries.
    pub const MOVE_REFUSED: &str = "move_refused";

    /// Every code above.
    pub const ALL: &[&str] = &[
        RULE_NOT_ELIGIBLE,
        NO_MATCH_AT_EXECUTION,
        UNKNOWN_AT_EXECUTION,
        PLAYBACK_HOLD,
        PLAYBACK_UNKNOWN,
        WATCH_RECHECK_NEWER_PLAY,
        WATCH_RECHECK_UNCREDITED_PLAY,
        WATCH_RECHECK_UNAVAILABLE,
        WATCH_RECHECK_UNMAPPED,
        ACTIVE_ACQUISITION,
        ACQUISITION_UNKNOWN,
        STORAGE_DELETION_IN_PROGRESS,
        ACTION_NOT_FULLY_SUPPORTED,
        TITLE_MISSING,
        LOCATION_OPERATION_HOLD,
        PASS_ACTION_STEP_BUDGET,
        PASS_HIGH_RISK_STEP_BUDGET,
        ACTION_SUCCEEDED,
        ALREADY_SATISFIED,
        RETRY_PENDING,
        ACTION_FAILED,
        LEASE_RECLAIMED,
        TAG_NOT_DEFINED,
        TAG_PATCH_CONFLICT,
        AMBIGUOUS_EPISODE_FILE_COVERAGE,
        QUALITY_PROFILE_UNRESOLVED,
        ACTIONS_CONFLICT,
        MOVE_LIBRARY_UNRESOLVED,
        MOVE_LIBRARY_FACET_MISMATCH,
        MOVE_ROOT_NOT_IN_LIBRARY,
        MOVE_REFUSED,
        ACTIONS_NOT_RUNNABLE,
        ACTIONS_CHANGED_AT_EXECUTION,
        ACTION_NOT_DECLARED,
        STORAGE_ROOT_ACTION_NOT_ALLOWED,
        MONITORING_CONFLICT,
    ];

    #[cfg(test)]
    mod tests {
        use super::ALL;
        use std::collections::BTreeSet;

        #[test]
        fn reasons_are_distinct_and_keep_the_stored_snake_case_shape() {
            let distinct = ALL.iter().collect::<BTreeSet<_>>();
            assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
            for reason in ALL {
                assert!(!reason.is_empty());
                assert!(
                    reason
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "{reason} is not lower snake case"
                );
                assert!(
                    !reason.starts_with('_') && !reason.ends_with('_') && !reason.contains("__"),
                    "{reason} has a stray underscore"
                );
            }
            assert_eq!(ALL.len(), 36);
        }
    }
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
    /// Rules that reached their own most-titles-per-run this pass. Their
    /// remaining due candidates were left for the next fire, untouched.
    pub rules_stopped_at_maximum: usize,
}

/// One action run with its title's display name resolved for rendering.
#[derive(Clone, Debug)]
pub struct MaintenanceActionRunView {
    pub subject_label: String,
    pub run: LifecycleActionRun,
    pub title_name: String,
    /// The steps this run's candidate runs, built from what was decided for
    /// it. `None` when the candidate is gone or its steps cannot be built.
    pub action_sequence: Option<MaintenanceActionSequence>,
    /// Ordered step progress. The bounded step rows and durable receipts are
    /// loaded page-wise, never as one history query per run.
    pub sequence_steps: Vec<MaintenanceActionStepProgressView>,
}

/// One ordered v2 operation and its latest durable evidence.
#[derive(Clone, Debug)]
pub struct MaintenanceActionStepProgressView {
    pub step: crate::maintenance_rules::MaintenanceActionStep,
    pub run: Option<MaintenanceActionStepRun>,
    pub receipts: Vec<MaintenanceActionJobReceipt>,
}

/// Outcome of one candidate's pass, folded into the report.
pub(super) enum CandidateOutcome {
    Executed,
    Held,
    Canceled,
    Failed,
    LeaseLost,
}

/// One handler pass shares these limits across every v2 candidate of a rule.
///
/// A sequence charges each claimed adapter dispatch, so a long sequence cannot
/// spend an unbounded number of operations while appearing as one candidate.
pub(super) struct MaintenanceSequencePassBudget {
    remaining_rule_steps: usize,
    remaining_high_risk_steps: usize,
    dispatched_steps: usize,
    failed_high_risk_steps: usize,
    last_dispatch_was_high_risk: bool,
    last_dispatch_failure_recorded: bool,
}

impl MaintenanceSequencePassBudget {
    pub(super) fn new(remaining_rule_steps: usize, remaining_high_risk_steps: usize) -> Self {
        Self {
            remaining_rule_steps,
            remaining_high_risk_steps,
            dispatched_steps: 0,
            failed_high_risk_steps: 0,
            last_dispatch_was_high_risk: false,
            last_dispatch_failure_recorded: false,
        }
    }

    pub(super) fn hold_reason_before_dispatch(&self, high_risk: bool) -> Option<&'static str> {
        if self.remaining_rule_steps == 0 {
            Some(execution_reason::PASS_ACTION_STEP_BUDGET)
        } else if high_risk && self.remaining_high_risk_steps == 0 {
            Some(execution_reason::PASS_HIGH_RISK_STEP_BUDGET)
        } else {
            None
        }
    }

    pub(super) fn record_dispatch(&mut self, high_risk: bool) {
        debug_assert!(self.remaining_rule_steps > 0);
        self.remaining_rule_steps -= 1;
        self.dispatched_steps += 1;
        self.last_dispatch_was_high_risk = high_risk;
        self.last_dispatch_failure_recorded = false;
        if high_risk {
            debug_assert!(self.remaining_high_risk_steps > 0);
            self.remaining_high_risk_steps -= 1;
        }
    }

    pub(super) fn record_failed_dispatch(&mut self) {
        if self.last_dispatch_was_high_risk && !self.last_dispatch_failure_recorded {
            self.failed_high_risk_steps += 1;
            self.last_dispatch_failure_recorded = true;
        }
    }

    pub(super) fn dispatched_steps(&self) -> usize {
        self.dispatched_steps
    }

    pub(super) fn failed_high_risk_steps(&self) -> usize {
        self.failed_high_risk_steps
    }
}

/// What an eligible rule needs to run its candidates. Each candidate runs the
/// sequence built from what was decided for it; `quality_profile_ids`
/// resolves every profile name the source can decide, read once per pass.
struct EligibleRuleAction {
    quality_profile_ids: HashMap<String, String>,
}

/// What the pre-execution safety rechecks decided.
pub(super) enum SafetyDecision {
    /// Act on this title, which every check has just seen fresh.
    Proceed(Box<Title>, Box<scryer_rules::maintenance::MaintenanceInput>),
    /// Refuse and block the candidate with this reason.
    Hold(&'static str),
    /// The subject stopped matching (or vanished); cancel the candidate.
    Cancel(&'static str),
    /// An exclusion now covers the subject.
    Excluded,
}

/// What an action implementation reports back.
pub(super) enum ActionResult {
    Executed {
        detail: serde_json::Value,
    },
    /// A storage-pressure matcher stopped matching after one or more files.
    /// The resulting terminal cancellation lets a future match start a new
    /// generation and grace period.
    Canceled {
        reason: &'static str,
        detail: serde_json::Value,
    },
    /// Capacity or another live execution fact became unknown after work had
    /// started. Keep the journal and remaining manifest for a later recheck.
    Held {
        reason: &'static str,
        detail: serde_json::Value,
    },
}

/// Fresh evidence and mutable execution state shared by an action dispatcher
/// and its scoped-deletion implementation. Keeping this as one bundle makes
/// the action boundary explicit without splitting a single lease across
/// loosely related parameters.
pub(super) struct MaintenanceExecutionContext<'a> {
    pub(super) detail: &'a MaintenanceRuleSetDetail,
    /// The steps this candidate runs, built from what was decided for it.
    pub(super) sequence: &'a MaintenanceActionSequence,
    pub(super) candidate: &'a LifecycleCandidate,
    pub(super) title: &'a Title,
    pub(super) input: &'a scryer_rules::maintenance::MaintenanceInput,
    pub(super) run: &'a mut LifecycleActionRun,
    pub(super) evaluator: &'a mut MaintenanceRulesEvaluator,
    pub(super) libraries: &'a HashMap<String, MaintenanceLibraryRef>,
    pub(super) tag_conflicts: &'a Option<HashSet<String>>,
}

// ── Rule state ──────────────────────────────────────────────────────────────

/// The arming an Enabled rule receives: file deletion when any part of its
/// action is high-risk, library updates when it changes anything else, and
/// nothing for an action that only tracks membership.
pub fn derived_effect_arming(declared: &MaintenanceDeclaredActions) -> MaintenanceEffectArming {
    if declared.destructive {
        MaintenanceEffectArming::Destructive
    } else if declared.heads.is_empty() {
        MaintenanceEffectArming::None
    } else {
        MaintenanceEffectArming::Reversible
    }
}

/// The arming a rule keeps across a save that needs no new confirmation: what
/// it held, lowered to what the new heads can arm. A save never raises arming,
/// and a rule whose source no longer deletes is no longer armed for deletes.
pub fn arming_kept_across_edit(
    stored: MaintenanceEffectArming,
    declared: &MaintenanceDeclaredActions,
) -> MaintenanceEffectArming {
    fn rank(arming: MaintenanceEffectArming) -> u8 {
        match arming {
            MaintenanceEffectArming::None => 0,
            MaintenanceEffectArming::Reversible => 1,
            MaintenanceEffectArming::Destructive => 2,
        }
    }
    let derived = derived_effect_arming(declared);
    if rank(derived) < rank(stored) {
        derived
    } else {
        stored
    }
}

/// The state an operator sees. A rule reads as Enabled only while it holds
/// exactly the arming enabling it would grant and has not been edited since,
/// because any other arming lets nothing execute.
pub fn maintenance_rule_state(detail: &MaintenanceRuleSetDetail) -> MaintenanceRuleState {
    let rule = &detail.rule_set;
    if rule.evaluation_mode == MaintenanceEvaluationMode::Disabled {
        MaintenanceRuleState::Disabled
    } else if rule.effect_arming != MaintenanceEffectArming::None
        && !rule.destructive_rearm_required
        && rule.effect_arming == derived_effect_arming(&detail.declared_actions)
    {
        MaintenanceRuleState::Enabled
    } else {
        MaintenanceRuleState::DryRun
    }
}

impl AppUseCase {
    /// Move one rule to Disabled, Dry run, or Enabled.
    ///
    /// Disabled is never evaluated; Dry run is evaluated but unarmed. Enabled
    /// arms exactly the effects the rule's own action can have
    /// ([`derived_effect_arming`]); execution still requires the matching
    /// instance gate as well. Enabling a rule that can delete files is the
    /// elevated, count-acknowledged step: it additionally requires
    /// system-settings authority, a confirmation of the reviewed revision and
    /// library scope, and the current number of non-terminal candidates.
    /// Enabling any other rule needs no confirmation.
    ///
    /// Arming never outlives the revision it was granted against: what an
    /// operator acknowledged is one matcher's blast radius under one action, so
    /// appending a revision drops an Enabled rule back to Dry run and this call
    /// has to be repeated against the new matcher (see
    /// [`AppUseCase::update_maintenance_rule_matcher`]).
    pub async fn set_maintenance_rule_state(
        &self,
        actor: &User,
        rule_set_id: &str,
        state: MaintenanceRuleState,
        confirmation: Option<scryer_domain::MaintenanceRuleArmingConfirmation>,
        acknowledged_candidate_count: Option<i64>,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        let mut detail = self.load_maintenance_rule_detail(rule_set).await?;
        let (mode, arming) = match state {
            MaintenanceRuleState::Disabled => (
                MaintenanceEvaluationMode::Disabled,
                MaintenanceEffectArming::None,
            ),
            MaintenanceRuleState::DryRun => (
                MaintenanceEvaluationMode::Observe,
                MaintenanceEffectArming::None,
            ),
            MaintenanceRuleState::Enabled => (
                MaintenanceEvaluationMode::Observe,
                derived_effect_arming(&detail.declared_actions),
            ),
        };
        if state == MaintenanceRuleState::Enabled
            && let Some(reason) = detail.load_refusal
        {
            return Err(super::service::stored_actions_refused(reason));
        }
        if state == MaintenanceRuleState::Enabled && arming == MaintenanceEffectArming::None {
            return Err(AppError::validation_refused(crate::error_reasons::maintenance::RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN, "this rule's action never changes anything, so it can only run as a dry run"));
        }

        if arming == MaintenanceEffectArming::Destructive {
            self.require_app_permission(actor, AppPermission::ManageSystemSettings)
                .await?;
            super::service::require_max_titles_per_run(
                &detail.declared_actions,
                detail.rule_set.max_titles_per_run,
            )?;
            if !confirmation
                .as_ref()
                .is_some_and(|reviewed| reviewed.matches(&detail.rule_set))
            {
                return Err(AppError::validation_refused(
                    crate::error_reasons::maintenance::RULE_CONFIGURATION_INVALID,
                    "the rule configuration must be reviewed again before arming deletion; reopen the confirmation",
                ));
            }
            let current = self
                .count_active_maintenance_candidates(&detail.rule_set.id)
                .await?;
            if acknowledged_candidate_count != Some(current) {
                // The count is a fact beside the translated refusal sentence.
                return Err(AppError::validation_refused_with_count(
                    crate::error_reasons::maintenance::ARM_COUNT_MISMATCH,
                    format!(
                        "destructive arming requires acknowledging the current candidate count ({current})"
                    ),
                    current,
                ));
            }
        }

        let now = Utc::now();
        let updated = self
            .services
            .customization
            .maintenance_rule_sets
            .update_rule_set_state(
                &detail.rule_set.id,
                mode,
                arming,
                confirmation.as_ref(),
                now,
            )
            .await?;
        if !updated {
            return Err(AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_CHANGED_WHILE_CONFIRMING,
                "the rule changed while confirming; review it again before arming deletion",
            ));
        }

        // A rule is scheduled while it is not disabled, so only crossing that
        // line changes what the scheduler holds.
        let schedule_changed = (detail.rule_set.evaluation_mode
            == MaintenanceEvaluationMode::Disabled)
            != (mode == MaintenanceEvaluationMode::Disabled);
        // Mirrors the single store write.
        detail.rule_set.evaluation_mode = mode;
        detail.rule_set.enabled = mode != MaintenanceEvaluationMode::Disabled;
        detail.rule_set.effect_arming = arming;
        detail.rule_set.destructive_rearm_required = false;
        detail.rule_set.updated_at = now;
        if schedule_changed {
            self.reload_maintenance_rule_schedule().await;
        }
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

    /// The step sequence each candidate runs and its progress, keyed by
    /// candidate id.
    ///
    /// Each sequence is built from what was decided for that candidate, the
    /// same way the executor builds it. Quality profile names are resolved in
    /// one read, and only when some candidate decided a profile.
    pub(super) async fn maintenance_sequence_projection(
        &self,
        candidates: &[LifecycleCandidate],
    ) -> AppResult<(
        HashMap<String, MaintenanceActionSequence>,
        HashMap<String, Vec<MaintenanceActionStepProgressView>>,
    )> {
        let decides_profile = candidates.iter().any(|candidate| {
            super::declared_actions::decided_actions_from_json(
                candidate.decided_actions_json.as_deref(),
            )
            .is_some_and(|actions| actions.set_quality_profile.is_some())
        });
        let quality_profile_ids = if decides_profile {
            self.maintenance_quality_profile_ids_by_name().await?
        } else {
            HashMap::new()
        };
        let sequences: HashMap<String, MaintenanceActionSequence> = candidates
            .iter()
            .filter_map(|candidate| {
                declared_candidate_sequence(candidate, &quality_profile_ids)
                    .ok()
                    .map(|sequence| (candidate.id.clone(), sequence))
            })
            .collect();
        let keys: Vec<MaintenanceActionStepCandidateKey> = candidates
            .iter()
            .filter(|candidate| sequences.contains_key(&candidate.id))
            .map(|candidate| MaintenanceActionStepCandidateKey {
                candidate_id: candidate.id.clone(),
                match_generation: candidate.match_generation,
                revision_number: candidate.revision_number,
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_steps_for_candidates(&keys)
            .await?;
        let step_keys: Vec<_> = runs.iter().map(|run| run.key.clone()).collect();
        let receipts = self
            .services
            .customization
            .maintenance_evaluation
            .list_action_job_receipts_for_steps(&step_keys)
            .await?;
        let mut runs_by_candidate: HashMap<String, HashMap<String, MaintenanceActionStepRun>> =
            HashMap::new();
        for run in runs {
            runs_by_candidate
                .entry(run.key.candidate_id.clone())
                .or_default()
                .insert(run.key.step_id.clone(), run);
        }
        let mut receipts_by_step: HashMap<_, Vec<MaintenanceActionJobReceipt>> = HashMap::new();
        for receipt in receipts {
            receipts_by_step
                .entry(receipt.key.clone())
                .or_default()
                .push(receipt);
        }
        let mut progress = HashMap::new();
        for (candidate_id, sequence) in &sequences {
            let candidate_runs = runs_by_candidate
                .get(candidate_id)
                .cloned()
                .unwrap_or_default();
            let steps = sequence
                .steps
                .iter()
                .map(|step| {
                    let run = candidate_runs.get(&step.id).cloned();
                    let receipts = run
                        .as_ref()
                        .and_then(|run| receipts_by_step.get(&run.key))
                        .cloned()
                        .unwrap_or_default();
                    MaintenanceActionStepProgressView {
                        step: step.clone(),
                        run,
                        receipts,
                    }
                })
                .collect();
            progress.insert(candidate_id.clone(), steps);
        }
        Ok((sequences, progress))
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
        // A DeleteFiles step owns an internal scoped-deletion action run for
        // fingerprints and crash recovery. Filter both streams in the store,
        // before their limits apply, so those child journals cannot push an
        // operator-visible legacy or sequence run off this history page.
        let mut runs = self
            .services
            .customization
            .maintenance_evaluation
            .list_non_sequence_action_runs(rule_set_id, candidate_id, limit)
            .await?;
        runs.extend(
            self.services
                .customization
                .maintenance_evaluation
                .list_sequence_history_action_runs(rule_set_id, candidate_id, limit)
                .await?,
        );
        runs.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        runs.truncate(limit);
        let names = self
            .maintenance_title_names(runs.iter().map(|run| run.title_id.clone()))
            .await?;
        let summaries = self
            .maintenance_subject_summaries(
                runs.iter()
                    .map(|row| (row.subject_kind.clone(), row.title_id.clone())),
            )
            .await?;
        let candidate_ids: Vec<String> = runs
            .iter()
            .map(|run| run.candidate_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let candidates = self
            .services
            .customization
            .maintenance_evaluation
            .get_candidates_by_ids(&candidate_ids)
            .await?;
        let (sequences, sequence_steps) = self.maintenance_sequence_projection(&candidates).await?;
        Ok(runs
            .into_iter()
            .map(|run| {
                let title_name = crate::maintenance_rules::evaluation::maintenance_title_name(
                    &names,
                    &run.title_id,
                );
                let subject_label = serde_json::from_str::<serde_json::Value>(&run.detail)
                    .ok()
                    .and_then(|detail| {
                        detail
                            .get("subject_label")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .or_else(|| {
                        summaries
                            .get(&(run.subject_kind.clone(), run.subject_id.clone()))
                            .map(|summary| summary.0.clone())
                    })
                    .unwrap_or_else(|| run.subject_id.clone());
                let action_sequence = sequences.get(&run.candidate_id).cloned();
                // A completed history row must describe that particular lease,
                // rather than the candidate's later durable step state. Running
                // summaries deliberately retain the live projection.
                let history_steps = match (
                    action_sequence.as_ref(),
                    maintenance_sequence_history_steps(&run),
                ) {
                    (Some(sequence), Some(snapshot))
                        if run.status != LifecycleActionRunStatus::Running =>
                    {
                        Some(
                            sequence
                                .steps
                                .iter()
                                .map(|step| MaintenanceActionStepProgressView {
                                    step: step.clone(),
                                    run: snapshot.get(&step.id).map(|entry| entry.run.clone()),
                                    receipts: snapshot
                                        .get(&step.id)
                                        .map(|entry| entry.receipts.clone())
                                        .unwrap_or_default(),
                                })
                                .collect(),
                        )
                    }
                    _ => None,
                };
                MaintenanceActionRunView {
                    action_sequence,
                    sequence_steps: history_steps.unwrap_or_else(|| {
                        sequence_steps
                            .get(&run.candidate_id)
                            .cloned()
                            .unwrap_or_default()
                    }),
                    run,
                    title_name,
                    subject_label,
                }
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
        // Tracked, so a restore drain waits for the run instead of swapping
        // the database under it.
        self.restore_coordinator().spawn_task(async move {
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
        self.run_lifecycle_action_handling(None).await
    }

    /// Act on the due candidates of every armed rule, or of one named rule.
    ///
    /// Scoped to one rule, the other rules are still resolved when the named
    /// one declares tag actions, because the tag-conflict scan has to see what
    /// they are about to write; only the named rule acts.
    pub(crate) async fn run_lifecycle_action_handling(
        &self,
        only_rule_set_id: Option<&str>,
    ) -> AppResult<MaintenanceActionHandlingReport> {
        let mut report = MaintenanceActionHandlingReport::default();
        let gates = self.load_maintenance_gates().await?;
        if !gates.reversible_effects_enabled && !gates.destructive_effects_enabled {
            return Ok(report);
        }
        report.gates_enabled = true;

        let mut rule_sets = self
            .services
            .customization
            .maintenance_rule_sets
            .list_rule_sets()
            .await?;
        if let Some(only) = only_rule_set_id {
            // The named rule resolves first, so whether the others are needed
            // is known before any of them is read.
            let Some(position) = rule_sets.iter().position(|rule| rule.id == only) else {
                return Ok(report);
            };
            let target = rule_sets.remove(position);
            rule_sets.insert(0, target);
        }
        report.rules_considered = if only_rule_set_id.is_some() {
            1
        } else {
            rule_sets.len()
        };

        let libraries = self.maintenance_library_refs().await?;
        let now = Utc::now();
        let mut high_risk_failures = 0usize;

        // Every rule that could run this pass is resolved first, in one place.
        // The tag-conflict scan below has to know what *other* rules are about
        // to write before the first of them writes anything, and resolving a
        // rule means reading its current revision — so the alternative is
        // reading every revision twice.
        let mut eligible_rules = Vec::new();
        for (index, rule_set) in rule_sets.into_iter().enumerate() {
            if only_rule_set_id.is_some() && index > 0 {
                // Peers matter only to the tag-conflict scan of a target that
                // was itself eligible and writes tags.
                let target_writes_tags = eligible_rules.first().is_some_and(
                    |(detail, _, _): &(MaintenanceRuleSetDetail, EligibleRuleAction, bool)| {
                        detail.rule_set.id.as_str() == only_rule_set_id.unwrap_or_default()
                            && (!detail.declared_actions.add_tags.is_empty()
                                || !detail.declared_actions.remove_tags.is_empty())
                    },
                );
                if !target_writes_tags {
                    break;
                }
            }
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
            if let Some(reason) = detail.load_refusal {
                // Its open candidates stay where they are and never run; the
                // rule surfaces the refusal until its source is saved again.
                warn!(
                    rule_set_id = detail.rule_set.id.as_str(),
                    reason, "skipping maintenance rule whose revision cannot be acted on"
                );
                continue;
            }
            let declared = &detail.declared_actions;
            if declared.heads.is_empty() {
                continue;
            }
            let high_risk = declared.destructive;
            let eligible = maintenance_rule_state(&detail) == MaintenanceRuleState::Enabled
                && (!high_risk || gates.destructive_effects_enabled)
                && (!declared.defines_reversible_action() || gates.reversible_effects_enabled);
            if !eligible {
                continue;
            }
            if high_risk && detail.rule_set.max_titles_per_run.is_none() {
                // Saving and arming refuse this shape; a row that reached it
                // anyway acts on nothing rather than on everything.
                warn!(
                    rule_set_id = detail.rule_set.id.as_str(),
                    "skipping destructive maintenance rule without a most-titles-per-run"
                );
                continue;
            }
            let quality_profile_ids = if declared.quality_profiles.is_empty() {
                HashMap::new()
            } else {
                match self.maintenance_quality_profile_ids_by_name().await {
                    Ok(ids) => ids,
                    Err(error) => {
                        warn!(error = %error, "skipping maintenance rule whose quality profiles could not be read");
                        continue;
                    }
                }
            };
            let action = EligibleRuleAction {
                quality_profile_ids,
            };
            if !eligible {
                continue;
            }
            if only_rule_set_id.is_none_or(|only| only == detail.rule_set.id) {
                report.rules_eligible += 1;
            }
            eligible_rules.push((detail, action, high_risk));
        }

        // Titles two rules would tag in opposite directions this pass. Computed
        // before any of them acts, so both sides hold rather than the later one
        // silently undoing the earlier one.
        let tag_conflicts = self
            .maintenance_tag_patch_conflicts(&eligible_rules, now)
            .await;
        // Which way each eligible rule can turn monitoring, from its
        // declared heads alone.
        let monitoring_sides: Vec<(String, MonitoringSides)> = eligible_rules
            .iter()
            .map(|(detail, _, _)| {
                (
                    detail.rule_set.id.clone(),
                    MonitoringSides::declared(&detail.declared_actions),
                )
            })
            .collect();

        for (detail, action, high_risk) in eligible_rules {
            if only_rule_set_id.is_some_and(|only| only != detail.rule_set.id) {
                continue;
            }
            // A rule never acts beside itself. Scoped, the caller holds the
            // rule's claim; across every rule, a rule whose own run is going
            // is left to it.
            let _claim = match only_rule_set_id {
                Some(_) => None,
                None => match self.claim_maintenance_rule_run(&detail.rule_set.id) {
                    Some(claim) => Some(claim),
                    None => continue,
                },
            };
            // One compile and evaluator per rule; every candidate's fresh
            // re-evaluation reuses the evaluator sequentially.
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
            let mut evaluator = engine.evaluator();

            // The selection covers abandoned leases as well as due work: a
            // candidate whose worker died mid-run is `executing` with nothing
            // driving it, and only this pass can hand it back to the lease's
            // reclaim arm.
            //
            // The walk pages through everything due, in `(due_at, id)` order,
            // until the rule has acted on its own most titles per fire. A
            // candidate this pass rewrites is stamped later than `now`, so it
            // never comes back in a later page of the same walk.
            let maximum = detail
                .rule_set
                .max_titles_per_run
                .map(|max| usize::try_from(max).unwrap_or(usize::MAX));
            let stale_before = self.maintenance_lease_stale_before(now);
            let mut titles_acted_on = 0usize;
            let mut cursor: Option<crate::MaintenanceDueCursor> = None;
            // What other rules recently did to monitoring, read once for this
            // rule, only once something is due, and only when another
            // eligible rule declares the opposite.
            let mut monitoring_acts: Option<Option<Vec<crate::MaintenanceStepAct>>> = None;
            'walk: loop {
                let due = self
                    .services
                    .customization
                    .maintenance_evaluation
                    .list_due_candidates(
                        &detail.rule_set.id,
                        now,
                        stale_before,
                        cursor.as_ref(),
                        MAINTENANCE_DUE_CANDIDATE_PAGE_SIZE,
                    )
                    .await?;
                let Some(last) = due.last() else {
                    break;
                };
                cursor = Some(crate::MaintenanceDueCursor::after(last));
                let last_page = due.len() < MAINTENANCE_DUE_CANDIDATE_PAGE_SIZE;
                if monitoring_acts.is_none() {
                    monitoring_acts = Some(
                        self.maintenance_monitoring_acts(&detail, &monitoring_sides, now)
                            .await,
                    );
                }
                let monitoring_acts = monitoring_acts
                    .as_ref()
                    .expect("monitoring acts are read before the first candidate");

                for candidate in due {
                    if maximum.is_some_and(|max| titles_acted_on >= max) {
                        // The rest stay due, untouched, for the next fire.
                        report.rules_stopped_at_maximum += 1;
                        break 'walk;
                    }
                    // A restore or shutdown drain waits for this pass. Candidates
                    // not started yet stay due for the next one.
                    if self.restore_coordinator().shutdown_workers.is_cancelled() {
                        break 'walk;
                    }
                    if high_risk && high_risk_failures >= MAINTENANCE_HIGH_RISK_FAILURE_BREAKER {
                        break 'walk;
                    }
                    report.candidates_considered += 1;
                    let mut sequence_budget = Some({
                        MaintenanceSequencePassBudget::new(
                            usize::MAX,
                            if high_risk_failures >= MAINTENANCE_HIGH_RISK_FAILURE_BREAKER {
                                0
                            } else {
                                usize::MAX
                            },
                        )
                    });
                    // A candidate runs only the steps built from what was decided
                    // for it; one without decided actions never runs.
                    let outcome = match declared_candidate_sequence(
                        &candidate,
                        &action.quality_profile_ids,
                    ) {
                        // Another rule recently did the opposite to this
                        // title's monitoring: hold before any step runs.
                        Ok(sequence)
                            if let Some(code) =
                                monitoring_conflict(monitoring_acts, &candidate, &sequence) =>
                        {
                            if candidate.state == MaintenanceCandidateState::Blocked
                                && candidate.state_reason == code
                            {
                                // Already held for it; nothing is written.
                                Ok(CandidateOutcome::Held)
                            } else {
                                self.hold_unbuilt_maintenance_candidate(&candidate, code)
                                    .await
                            }
                        }
                        Ok(sequence) => {
                            self.execute_one_maintenance_sequence_candidate(
                                candidate,
                                SequenceCandidateExecutionContext {
                                    detail: &detail,
                                    sequence: &sequence,
                                    evaluator: &mut evaluator,
                                    libraries: &libraries,
                                    tag_conflicts: &tag_conflicts,
                                    budget: sequence_budget
                                        .as_mut()
                                        .expect("sequence actions always have a step budget"),
                                },
                            )
                            .await
                        }
                        Err(reason) => {
                            self.hold_unbuilt_maintenance_candidate(&candidate, reason)
                                .await
                        }
                    };
                    if outcome.is_err() {
                        // The dispatch may have reached its adapter before a
                        // persistence failure escaped. Charge a high-risk last
                        // dispatch conservatively, matching the v1 error path.
                        if let Some(budget) = sequence_budget.as_mut() {
                            budget.record_failed_dispatch();
                        }
                    }
                    if let Some(budget) = sequence_budget.as_ref() {
                        // A title counts once however many of its steps ran,
                        // including one resuming a sequence an earlier fire began.
                        if budget.dispatched_steps() > 0 {
                            titles_acted_on += 1;
                        }
                        high_risk_failures += budget.failed_high_risk_steps();
                    }
                    match outcome {
                        Ok(CandidateOutcome::Executed) => {
                            report.executed += 1;
                        }
                        Ok(CandidateOutcome::Held) => report.held += 1,
                        Ok(CandidateOutcome::Canceled) => report.canceled += 1,
                        Ok(CandidateOutcome::Failed) => {
                            report.failed += 1;
                        }
                        Ok(CandidateOutcome::LeaseLost) => report.lease_lost += 1,
                        Err(error) => {
                            // The side effect may have completed before its result
                            // failed to persist. Charge the attempt conservatively.
                            report.failed += 1;
                            warn!(error = %error, "maintenance action handling failed for one candidate");
                        }
                    }
                }
                if last_page {
                    break;
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
    /// The scan re-lists due candidates for tag rules only, in the same
    /// bounded pages the walk uses. A rule's maximum does not shorten it: a
    /// held candidate lets the walk reach past the first few due, so every
    /// due title of a tag rule is one it could patch this pass. That buys a
    /// decision made before the first write instead of after it.
    async fn maintenance_tag_patch_conflicts(
        &self,
        eligible_rules: &[(MaintenanceRuleSetDetail, EligibleRuleAction, bool)],
        now: chrono::DateTime<Utc>,
    ) -> Option<HashSet<String>> {
        let mut added: HashMap<String, HashSet<String>> = HashMap::new();
        let mut removed: HashMap<String, HashSet<String>> = HashMap::new();

        for (detail, _, _) in eligible_rules {
            let declared = &detail.declared_actions;
            if declared.add_tags.is_empty() && declared.remove_tags.is_empty() {
                continue;
            }
            let stale_before = self.maintenance_lease_stale_before(now);
            let mut cursor: Option<crate::MaintenanceDueCursor> = None;
            loop {
                let due = match self
                    .services
                    .customization
                    .maintenance_evaluation
                    .list_due_candidates(
                        &detail.rule_set.id,
                        now,
                        stale_before,
                        cursor.as_ref(),
                        MAINTENANCE_DUE_CANDIDATE_PAGE_SIZE,
                    )
                    .await
                {
                    Ok(due) => due,
                    Err(error) => {
                        // A later read may recover, but it cannot repair this
                        // pass's incomplete conflict picture. Hold tag effects
                        // without charging an attempt; unrelated effects may run.
                        warn!(error = %error, "could not scan a tag rule for patch conflicts");
                        return None;
                    }
                };
                let Some(last) = due.last() else {
                    break;
                };
                cursor = Some(crate::MaintenanceDueCursor::after(last));
                let last_page = due.len() < MAINTENANCE_DUE_CANDIDATE_PAGE_SIZE;
                for candidate in due {
                    // A candidate patches only what was decided for it; the
                    // source's full tag lists are only the reason to look. One
                    // without decided actions never runs, so it patches nothing.
                    let Some(decided) = super::declared_actions::decided_actions_from_json(
                        candidate.decided_actions_json.as_deref(),
                    ) else {
                        continue;
                    };
                    let (adds, removes) = (&decided.add_tags, &decided.remove_tags);
                    if !adds.is_empty() {
                        added
                            .entry(candidate.title_id.clone())
                            .or_default()
                            .extend(adds.iter().cloned());
                    }
                    if !removes.is_empty() {
                        removed
                            .entry(candidate.title_id)
                            .or_default()
                            .extend(removes.iter().cloned());
                    }
                }
                if last_page {
                    break;
                }
            }
        }

        Some(
            added
                .iter()
                .filter(|(title_id, adds)| {
                    removed
                        .get(*title_id)
                        .is_some_and(|removes| adds.iter().any(|label| removes.contains(label)))
                })
                .map(|(title_id, _)| title_id.clone())
                .collect(),
        )
    }

    /// What other rules did to monitoring within
    /// [`MAINTENANCE_MONITORING_CONFLICT_WINDOW_SECONDS`], for one rule about
    /// to run. One bounded read, made only when another eligible rule
    /// declares the opposite direction; otherwise nothing is read. `None` is
    /// a picture Scryer could not complete: a failed read, or one that hit
    /// its bound, which holds this rule's monitoring work for the pass.
    async fn maintenance_monitoring_acts(
        &self,
        detail: &MaintenanceRuleSetDetail,
        monitoring_sides: &[(String, MonitoringSides)],
        now: chrono::DateTime<Utc>,
    ) -> Option<Vec<crate::MaintenanceStepAct>> {
        let own = MonitoringSides::declared(&detail.declared_actions);
        let opposed = monitoring_sides
            .iter()
            .any(|(rule_set_id, other)| *rule_set_id != detail.rule_set.id && own.opposes(other));
        if !opposed {
            return Some(Vec::new());
        }
        let kinds = [
            MaintenanceActionStepKind::Monitor.as_wire_str(),
            MaintenanceActionStepKind::Unmonitor.as_wire_str(),
            MaintenanceActionStepKind::DeleteFiles.as_wire_str(),
            MaintenanceActionStepKind::DeleteTitleAndFiles.as_wire_str(),
        ];
        match self
            .services
            .customization
            .maintenance_evaluation
            .list_step_acts_since(
                &kinds,
                now - chrono::Duration::seconds(MAINTENANCE_MONITORING_CONFLICT_WINDOW_SECONDS),
                MAINTENANCE_MONITORING_CONFLICT_SCAN_LIMIT,
            )
            .await
        {
            Ok(acts) if acts.len() < MAINTENANCE_MONITORING_CONFLICT_SCAN_LIMIT => Some(acts),
            Ok(_) => {
                warn!(
                    rule_set_id = detail.rule_set.id,
                    "too many recent monitoring changes to rule out a conflict"
                );
                None
            }
            Err(error) => {
                warn!(error = %error, "could not scan recent monitoring changes for conflicts");
                None
            }
        }
    }

    /// The ordered pre-execution rechecks (RFC 8 step 3, RFC 9.10). Order
    /// matters: eligibility first so a disarmed rule never even re-evaluates,
    /// the matcher before the environment so "stopped matching" reads as a
    /// cancel rather than a hold, and the environment holds last.
    pub(super) async fn maintenance_execution_safety_checks(
        &self,
        detail: &MaintenanceRuleSetDetail,
        sequence: &MaintenanceActionSequence,
        evaluator: &mut MaintenanceRulesEvaluator,
        candidate: &LifecycleCandidate,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
        tag_conflicts: &Option<HashSet<String>>,
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
            Err(AppError::NotFound(_) | AppError::NotFoundReason { .. }) => {
                return SafetyDecision::Cancel(execution_reason::RULE_NOT_ELIGIBLE);
            }
            Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
        };
        let gates = match self.load_maintenance_gates().await {
            Ok(gates) => gates,
            Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
        };
        if detail.load_refusal.is_some()
            || candidate.action_kind != ACTION_SEQUENCE_KIND
            || detail.revision.revision_number != candidate.revision_number
        {
            return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE);
        }
        // Only what the stored revision's source can decide ever runs. The
        // steps were built from the candidate's decided actions, which must
        // exist and name only heads that revision declares.
        let Some(decided) = super::declared_actions::decided_actions_from_json(
            candidate.decided_actions_json.as_deref(),
        ) else {
            return SafetyDecision::Hold(execution_reason::ACTIONS_NOT_RUNNABLE);
        };
        if decided
            .heads()
            .iter()
            .any(|head| !detail.declared_actions.defines(*head))
        {
            return SafetyDecision::Hold(execution_reason::ACTION_NOT_DECLARED);
        }
        if detail.revision.storage_root_id.is_some()
            && sequence
                .steps
                .iter()
                .any(|step| !step.descriptor().storage_root_allowed)
        {
            return SafetyDecision::Hold(execution_reason::STORAGE_ROOT_ACTION_NOT_ALLOWED);
        }
        let high_risk = sequence
            .steps
            .iter()
            .any(|step| step.descriptor().risk_class == MaintenanceRiskClass::High);
        let sequence_requires_reversible = sequence.requires_reversible_gate();
        let still_eligible = fresh.enabled
            && fresh.evaluation_mode == MaintenanceEvaluationMode::Observe
            && fresh.current_revision_number == candidate.revision_number
            && if high_risk {
                gates.destructive_effects_enabled
                    && fresh.effect_arming == MaintenanceEffectArming::Destructive
                    && !fresh.destructive_rearm_required
                    && (!sequence_requires_reversible || gates.reversible_effects_enabled)
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
        let tag_labels: Vec<&String> = sequence
            .steps
            .iter()
            .flat_map(|step| step.parameters.tag_labels().iter())
            .collect();
        if !tag_labels.is_empty() {
            let Some(tag_conflicts) = tag_conflicts else {
                return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
            };
            if tag_conflicts.contains(&candidate.title_id) {
                return SafetyDecision::Hold(execution_reason::TAG_PATCH_CONFLICT);
            }
            let tag_labels: Vec<String> = tag_labels.into_iter().cloned().collect();
            match self.undefined_title_tag_labels(&tag_labels).await {
                Ok(undefined) if !undefined.is_empty() => {
                    return SafetyDecision::Hold(execution_reason::TAG_NOT_DEFINED);
                }
                Ok(_) => {}
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        }

        let exclusions = match self
            .services
            .customization
            .maintenance_evaluation
            .list_exclusions(Some(&candidate.rule_set_id))
            .await
        {
            Ok(exclusions) => exclusions,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        if fresh.subject_kind.as_storage_str() != candidate.subject_kind {
            return SafetyDecision::Hold(execution_reason::ACTION_NOT_FULLY_SUPPORTED);
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
        // The one exception is a move this same sequence made: a title the
        // rule itself sent to another library is still the rule's subject for
        // the steps that follow the move.
        if !fresh.library_ids.is_empty() && !fresh.library_ids.contains(&title.library_id) {
            match self
                .maintenance_sequence_moved_title(candidate, sequence, &title)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    return SafetyDecision::Cancel(
                        crate::maintenance_rules::evaluation::candidate_reason::OUT_OF_SCOPE,
                    );
                }
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        }
        let action_subject = match candidate.subject_kind.as_str() {
            "title" if title.facet == scryer_domain::MediaFacet::Movie => ActionSubjectKind::Movie,
            "title" => ActionSubjectKind::Show,
            "season" => ActionSubjectKind::Season,
            "episode" => ActionSubjectKind::Episode,
            _ => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
        };
        if sequence
            .steps
            .iter()
            .any(|step| !step.descriptor().supports_subject(action_subject))
        {
            return SafetyDecision::Hold(execution_reason::ACTION_NOT_FULLY_SUPPORTED);
        }
        // (3b) A location operation that owns the title. Every action waits
        // for it the way destructive work waits for playback and acquisition
        // activity: the operation releases its claim on every terminal path,
        // so this is a retry, not a verdict. An unreadable claim store holds
        // the same way — acting on a claim Scryer could not read is acting on
        // evidence it never had.
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
        // (3c) The title's lifecycle claims, for the fact builder below, so
        // preview, the scheduled pass, and this recheck all decide on one
        // claim snapshot. A lease is only a time to delete: it reaches rules
        // through the request_lease facts and holds nothing by itself.
        let recheck_time = Utc::now();
        let claims_by_title = match self
            .maintenance_claims_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(claims) => Some(claims),
            // A claim store Scryer cannot read is a hold for a high-risk
            // action, and unknown claim facts for everything else.
            Err(_) if high_risk => {
                return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
            }
            Err(_) => None,
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
        let (watch_context, audience_by_title) = self
            .maintenance_audience_plays_for_titles(&watch_context, std::slice::from_ref(&title))
            .await;
        let people = MaintenanceTitlePeople {
            requester_user_ids: requesters_by_title.get(&title.id).map(Vec::as_slice),
            usernames: &usernames,
        };
        let watch = MaintenanceTitleWatch {
            context: &watch_context,
            signals: signals_by_title.get(&title.id).map(Vec::as_slice),
            audience: audience_by_title.get(&title.id).map(Vec::as_slice),
        };
        let claims = MaintenanceTitleClaims {
            claims: claims_by_title
                .as_ref()
                .and_then(|claims| claims.get(&title.id))
                .map(Vec::as_slice),
            store_readable: claims_by_title.is_some(),
        };
        let subject = match self
            .maintenance_resolve_subject(
                &candidate.subject_kind,
                &candidate.subject_id,
                &title,
                files,
            )
            .await
        {
            Ok(subject) => subject,
            Err(AppError::NotFound(_) | AppError::NotFoundReason { .. }) => {
                return SafetyDecision::Cancel(execution_reason::TITLE_MISSING);
            }
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        if subject.excluded_by(&title.id, &exclusions) {
            return SafetyDecision::Excluded;
        }
        if high_risk && candidate.subject_kind != "title" && subject.ambiguous_files {
            return SafetyDecision::Hold(execution_reason::AMBIGUOUS_EPISODE_FILE_COVERAGE);
        }
        let sequence_delete_files_step_id = sequence
            .steps
            .iter()
            .find(|step| step.kind == MaintenanceActionStepKind::DeleteFiles)
            .map(|step| step.id.as_str());
        let deletion_checkpoint = if let Some(step_id) = sequence_delete_files_step_id {
            match self
                .maintenance_deletion_checkpoint_for_step(
                    candidate,
                    detail.revision.storage_root_id.as_deref(),
                    Some(step_id),
                )
                .await
            {
                Ok(checkpoint) => checkpoint,
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        } else {
            None
        };
        // A storage-scoped rule's subject may move after its preview. Even
        // unmonitor-and-keep-files changes the whole subject, so it must not
        // run once no actual owned file remains on the selected root.
        let mut storage_journal_cleanup_only = false;
        if detail.revision.storage_root_id.is_some() {
            let storage_owned_files = if candidate.subject_kind == "title" {
                match self.maintenance_title_deletion_files(&title).await {
                    Ok(files) => files,
                    Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
                }
            } else if subject.ambiguous_files {
                return SafetyDecision::Hold(execution_reason::AMBIGUOUS_EPISODE_FILE_COVERAGE);
            } else {
                subject.files.clone()
            };
            match self
                .maintenance_root_file_summary(
                    &storage_owned_files,
                    detail.revision.storage_root_id.as_deref(),
                )
                .await
            {
                Ok(summary) if matches!(summary.file_count, Some(count) if count > 0) => {}
                Ok(summary) => {
                    let Some(checkpoint) = deletion_checkpoint.as_ref() else {
                        return if matches!(summary.file_count, Some(0)) {
                            SafetyDecision::Cancel(
                                crate::maintenance_rules::evaluation::candidate_reason::OUT_OF_SCOPE,
                            )
                        } else {
                            SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION)
                        };
                    };
                    match self
                        .maintenance_storage_checkpoint_can_resume(
                            checkpoint,
                            &storage_owned_files,
                            detail.revision.storage_root_id.as_deref(),
                        )
                        .await
                    {
                        Ok(true) => match self
                            .maintenance_storage_checkpoint_needs_catalog_cleanup(
                                checkpoint,
                                &storage_owned_files,
                            )
                            .await
                        {
                            Ok(cleanup_only) => storage_journal_cleanup_only = cleanup_only,
                            Err(_) => {
                                return SafetyDecision::Hold(
                                    execution_reason::UNKNOWN_AT_EXECUTION,
                                );
                            }
                        },
                        Ok(false) | Err(_) => {
                            return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
                        }
                    }
                }
                Err(_) => {
                    return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
                }
            }
        }
        let completed_retention_episode_ids = deletion_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.completed_retention_episode_ids());
        let mut input = build_subject_input_with_completed_retention_episodes(
            recheck_time,
            &title,
            &library,
            &subject,
            people,
            watch,
            claims,
            series_movies_by_title
                .get(&title.id)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            completed_retention_episode_ids.as_ref(),
        );
        // Re-read like everything else here: a title that joined a list since
        // the pass must not be removed on the pass's stale answer. A store
        // that cannot be read leaves the list facts unknown, which holds any
        // rule that reads them.
        let lists_by_title = self
            .maintenance_list_facts_loading(
                std::slice::from_ref(&title),
                super::list_facts::MaintenanceFactLoads::for_rule(
                    &detail.revision.rego_source,
                    &detail.rule_set.id,
                ),
                &watch_context,
                recheck_time,
            )
            .await;
        super::list_facts::apply_list_facts(&mut input, &lists_by_title, &title.id);
        super::list_facts::apply_watchlist_and_metadata_facts(
            &mut input,
            &lists_by_title,
            &title.id,
            people,
        );
        // The capacity snapshot must be fresh for this lease. Baseline restore
        // below deliberately excludes storage facts, so a resumed deletion
        // cannot match against the capacity it observed before deleting.
        if self
            .populate_maintenance_storage_facts(
                &mut input,
                detail.revision.storage_root_id.as_deref(),
            )
            .await
            .is_err()
        {
            return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
        }
        if detail.revision.storage_root_id.is_some()
            && !matches!(
                input.facts.storage_available_bytes,
                scryer_rules::maintenance::Observation::Known { .. }
            )
        {
            return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
        }
        if let Some(checkpoint) = deletion_checkpoint.as_ref() {
            checkpoint.restore_sequence_owned_file_deltas(&mut input);
        }
        // Sequence baselines are intentionally narrow. A completed Unmonitor
        // may keep only its own monitored fact stable for later steps, and
        // only after the persisted postcondition still holds. An external
        // re-monitor is live evidence, so it is never hidden by a baseline.
        {
            let completed_steps = match self
                .services
                .customization
                .maintenance_evaluation
                .list_action_steps(
                    &candidate.id,
                    candidate.match_generation,
                    candidate.revision_number,
                )
                .await
            {
                Ok(steps) => steps,
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            };
            let sequence_hash = match sequence.content_hash() {
                Ok(hash) => hash,
                Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
            };
            for step in &sequence.steps {
                let Some(run) = completed_steps
                    .iter()
                    .find(|run| run.key.step_id == step.id)
                else {
                    continue;
                };
                if !run.state.is_completion_success() {
                    continue;
                }
                if !super::sequence_execution::persisted_sequence_step_is_compatible(
                    run,
                    candidate,
                    &sequence_hash,
                    step,
                ) {
                    return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE);
                }
                let before = serde_json::from_str::<serde_json::Value>(&run.before_state_json)
                    .map_err(|_| ())
                    .ok();
                let provenance = serde_json::from_str::<serde_json::Value>(&run.provenance_json)
                    .map_err(|_| ())
                    .ok();
                match step.kind {
                    crate::maintenance_rules::MaintenanceActionStepKind::Unmonitor => {
                        let crate::maintenance_rules::MaintenanceActionStepParameters::Unmonitor {
                            include_descendants,
                        } = &step.parameters
                        else {
                            return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE);
                        };
                        match self
                            .maintenance_unmonitoring_is_confirmed(
                                candidate,
                                &title,
                                *include_descendants,
                            )
                            .await
                        {
                            Ok(true) => {
                                let monitoring_changed = provenance
                                    .as_ref()
                                    .and_then(|value| value.get("monitoring_changed"))
                                    .and_then(serde_json::Value::as_bool)
                                    == Some(true);
                                let before_monitored = before
                                    .as_ref()
                                    .and_then(|value| value.get("facts_monitored"))
                                    .and_then(serde_json::Value::as_bool);
                                if monitoring_changed {
                                    if let Some(before_monitored) = before_monitored {
                                        input.facts.monitored =
                                            scryer_rules::maintenance::Observation::known(
                                                before_monitored,
                                            );
                                    }
                                    if let Some(before_count) = before
                                        .as_ref()
                                        .and_then(|value| {
                                            value.get("facts_monitored_episode_count")
                                        })
                                        .and_then(serde_json::Value::as_i64)
                                    {
                                        input.facts.monitored_episode_count =
                                            scryer_rules::maintenance::Observation::known(
                                                before_count,
                                            );
                                    }
                                }
                            }
                            // A later external re-monitor is live input for
                            // the matcher. Do not restore our earlier value
                            // or manufacture a no-match here: DeleteFiles
                            // independently rechecks this destructive
                            // prerequisite immediately before every unlink.
                            Ok(false) => {}
                            Err(_) => {
                                return SafetyDecision::Hold(
                                    execution_reason::UNKNOWN_AT_EXECUTION,
                                );
                            }
                        }
                    }
                    crate::maintenance_rules::MaintenanceActionStepKind::Monitor => {
                        let monitoring_changed = provenance
                            .as_ref()
                            .and_then(|value| value.get("monitoring_changed"))
                            .and_then(serde_json::Value::as_bool)
                            == Some(true);
                        if !monitoring_changed {
                            continue;
                        }
                        // Same terms as Unmonitor: our own write is hidden
                        // from the matcher only while it still holds. An
                        // external unmonitor since is live input.
                        match self
                            .maintenance_monitoring_is_confirmed(candidate, &title)
                            .await
                        {
                            Ok(true) => {
                                if let Some(before_monitored) = before
                                    .as_ref()
                                    .and_then(|value| value.get("facts_monitored"))
                                    .and_then(serde_json::Value::as_bool)
                                {
                                    input.facts.monitored =
                                        scryer_rules::maintenance::Observation::known(
                                            before_monitored,
                                        );
                                }
                            }
                            Ok(false) => {}
                            Err(_) => {
                                return SafetyDecision::Hold(
                                    execution_reason::UNKNOWN_AT_EXECUTION,
                                );
                            }
                        }
                    }
                    crate::maintenance_rules::MaintenanceActionStepKind::MoveToLibrary
                    | crate::maintenance_rules::MaintenanceActionStepKind::MoveToRoot => {
                        // A title this sequence moved is matched on the
                        // library it was in before the move, while it is
                        // still where the move sent it.
                        let landed = provenance.as_ref().is_some_and(|value| {
                            super::sequence_execution::sequence_move_landed(value, &title)
                        });
                        if landed
                            && let Some(before) = before.as_ref()
                            && let (Some(id), Some(name)) = (
                                before.get("library_id").and_then(serde_json::Value::as_str),
                                before
                                    .get("library_name")
                                    .and_then(serde_json::Value::as_str),
                            )
                        {
                            input.library = scryer_rules::maintenance::MaintenanceLibraryDoc {
                                id: id.to_string(),
                                name: name.to_string(),
                            };
                        }
                    }
                    crate::maintenance_rules::MaintenanceActionStepKind::AddTags
                    | crate::maintenance_rules::MaintenanceActionStepKind::RemoveTags => {
                        let changed_tags = provenance
                            .as_ref()
                            .and_then(|value| value.get("changed_tags"))
                            .and_then(serde_json::Value::as_array)
                            .map(|tags| {
                                tags.iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        if let scryer_rules::maintenance::Observation::Known { value, .. } =
                            &mut input.facts.tags
                        {
                            for tag in changed_tags {
                                match step.kind {
                                    crate::maintenance_rules::MaintenanceActionStepKind::AddTags
                                        if value.contains(&tag) => value.retain(|current| current != &tag),
                                    crate::maintenance_rules::MaintenanceActionStepKind::RemoveTags
                                        if !value.contains(&tag) => value.push(tag),
                                    _ => {}
                                }
                            }
                        }
                    }
                    crate::maintenance_rules::MaintenanceActionStepKind::ChangeQualityProfile => {
                        let profile_changed = provenance
                            .as_ref()
                            .and_then(|value| value.get("profile_changed"))
                            .and_then(serde_json::Value::as_bool)
                            == Some(true);
                        let target = provenance
                            .as_ref()
                            .and_then(|value| value.get("quality_profile_id"))
                            .and_then(serde_json::Value::as_str);
                        let current_matches_target = target.is_some_and(|target| {
                            matches!(
                                &input.facts.quality_profile_id,
                                scryer_rules::maintenance::Observation::Known { value, .. }
                                    if crate::settings::runtime::quality_profile_ids_equal(value, target)
                            )
                        });
                        if profile_changed && current_matches_target {
                            match before
                                .as_ref()
                                .and_then(|value| value.get("quality_profile_id"))
                                .and_then(serde_json::Value::as_str)
                            {
                                Some(previous) => {
                                    input.facts.quality_profile_id =
                                        scryer_rules::maintenance::Observation::known(
                                            previous.to_string(),
                                        );
                                }
                                None => {
                                    input.facts.quality_profile_id =
                                        scryer_rules::maintenance::Observation::absent();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let decision = evaluator.evaluate(&input).ok().and_then(|result| {
            result
                .records
                .into_iter()
                .next()
                .map(|record| record.decision)
        });
        let outcome = decision.as_ref().map(|decision| decision.outcome);
        match outcome {
            Some(MaintenanceOutcome::Match) => {
                // A candidate carries the actions it was decided. Acting on a
                // fresh decision that differs would run steps nobody waited
                // out the grace for, so it holds until the evaluator replaces
                // the candidate.
                if let Some(stored) = candidate.decided_actions_json.as_deref() {
                    let stored = super::declared_actions::decided_actions_from_json(Some(stored));
                    if stored.as_ref() != decision.as_ref().map(|decision| &decision.actions) {
                        return SafetyDecision::Hold(
                            execution_reason::ACTIONS_CHANGED_AT_EXECUTION,
                        );
                    }
                }
            }
            Some(MaintenanceOutcome::NoMatch) if !storage_journal_cleanup_only => {
                return SafetyDecision::Cancel(execution_reason::NO_MATCH_AT_EXECUTION);
            }
            // Every exact journal path is already absent. Continue only far
            // enough to remove its stale media-file row and record progress.
            Some(MaintenanceOutcome::NoMatch) => {}
            Some(MaintenanceOutcome::Unknown) | None => {
                return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
            }
        }

        // (3b) A match that rests on watch facts rests on the last sweep. Read
        // the subject's watched items back from their servers first; a play
        // since then, a play no stored fact credits, or a read that cannot be
        // made holds. A clean read changes nothing it read.
        if super::watch_recheck::rule_reads_watch_facts(
            &detail.revision.rego_source,
            &candidate.rule_set_id,
        ) {
            let episode_ids = (subject.kind != scryer_domain::MaintenanceRuleSubjectKind::Title)
                .then(|| {
                    subject
                        .episodes
                        .iter()
                        .map(|episode| episode.id.clone())
                        .collect::<HashSet<_>>()
                });
            let entities =
                super::watch_recheck::recheck_entities(&title.facet, &title.id, &subject);
            // The comparison baseline is what the sweep stored, whether or
            // not the per-user gate lets those rows be used as facts: with
            // the gate closed the rule ran on the anonymous facts alone, and
            // an old play a swept account already had is not a new one.
            let baseline = if watch_context.freshness == super::facts::WatchSignalFreshness::Fresh {
                signals_by_title.get(&title.id).cloned().unwrap_or_default()
            } else {
                match self
                    .maintenance_stored_watch_signals_for_titles(std::slice::from_ref(&title))
                    .await
                {
                    Ok(mut stored) => stored.remove(&title.id).unwrap_or_default(),
                    Err(_) => {
                        return SafetyDecision::Hold(execution_reason::WATCH_RECHECK_UNAVAILABLE);
                    }
                }
            };
            let recheck = self
                .maintenance_watch_recheck(
                    &watch_context,
                    &baseline,
                    audience_by_title
                        .get(&title.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    episode_ids.as_ref(),
                    entities.as_deref(),
                )
                .await;
            if let Some(reason) = recheck.hold_reason() {
                return SafetyDecision::Hold(reason);
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

        SafetyDecision::Proceed(Box::new(title), Box::new(input))
    }
}

/// The sequence a declared rule runs for one candidate, built from the
/// actions stored on it. An `Err` is the hold reason.
fn declared_candidate_sequence(
    candidate: &LifecycleCandidate,
    quality_profile_ids: &HashMap<String, String>,
) -> Result<MaintenanceActionSequence, &'static str> {
    let Some(actions) = super::declared_actions::decided_actions_from_json(
        candidate.decided_actions_json.as_deref(),
    ) else {
        return Err(execution_reason::ACTIONS_NOT_RUNNABLE);
    };
    let subject = match candidate.subject_kind.as_str() {
        "title" if candidate.facet == scryer_domain::MediaFacet::Movie.as_str() => {
            ActionSubjectKind::Movie
        }
        "title" => ActionSubjectKind::Show,
        "season" => ActionSubjectKind::Season,
        "episode" => ActionSubjectKind::Episode,
        _ => return Err(execution_reason::ACTIONS_NOT_RUNNABLE),
    };
    if actions.conflict().is_some() {
        return Err(execution_reason::ACTIONS_CONFLICT);
    }
    let profile_id = match actions.set_quality_profile.as_deref() {
        Some(name) => match quality_profile_ids.get(name) {
            Some(id) => Some(id.as_str()),
            None => return Err(execution_reason::QUALITY_PROFILE_UNRESOLVED),
        },
        None => None,
    };
    super::declared_actions::build_declared_action_sequence(&actions, subject, profile_id)
        .map_err(|_| execution_reason::ACTIONS_NOT_RUNNABLE)
}

impl AppUseCase {
    /// Configured quality profiles by name. A name two profiles share
    /// resolves to neither, so a rule never picks one of them by accident.
    pub(crate) async fn maintenance_quality_profile_ids_by_name(
        &self,
    ) -> AppResult<HashMap<String, String>> {
        let profiles = self.load_quality_profile_settings().await?.profiles;
        let mut ids: HashMap<String, Option<String>> = HashMap::new();
        for profile in profiles {
            ids.entry(profile.name)
                .and_modify(|id| *id = None)
                .or_insert(Some(profile.id));
        }
        Ok(ids
            .into_iter()
            .filter_map(|(name, id)| id.map(|id| (name, id)))
            .collect())
    }

    /// Block a due candidate whose decided actions cannot become a sequence,
    /// without running anything or charging an attempt. A later pass retries:
    /// the profile may be restored, or the evaluator may replace the
    /// candidate.
    async fn hold_unbuilt_maintenance_candidate(
        &self,
        candidate: &LifecycleCandidate,
        reason: &'static str,
    ) -> AppResult<CandidateOutcome> {
        let candidates = &self.services.customization.maintenance_evaluation;
        let now = Utc::now();
        if candidate.state != MaintenanceCandidateState::Executing
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
                self.maintenance_lease_stale_before(now),
                now,
            )
            .await?
        {
            return Ok(CandidateOutcome::LeaseLost);
        }
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
            Ok(CandidateOutcome::Held)
        } else {
            Ok(CandidateOutcome::LeaseLost)
        }
    }
}
