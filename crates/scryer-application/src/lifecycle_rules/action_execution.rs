//! Scheduled execution of due lifecycle candidates.
//!
//! # What this module may do
//!
//! For rules that are enabled, in observe mode, and explicitly armed — and only
//! while the matching instance effect gate is on — run the step sequence built
//! from each candidate's decided actions once its grace has elapsed, through the same
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
//! leaves the candidate [`LifecycleCandidateState::Blocked`], re-checked on
//! the next pass. Nothing in this module deletes a file outside
//! [`AppUseCase::delete_title_by_policy`], whose fingerprint check is the same
//! one the human deletion dialog uses.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, Utc};
use scryer_domain::{
    AppPermission, LifecycleActionJobReceipt, LifecycleActionRun, LifecycleActionRunStatus,
    LifecycleActionStepRun, LifecycleCandidate, LifecycleCandidateState, LifecycleRuleState, Title,
    User,
};
use scryer_rules::lifecycle::{
    LifecycleOutcome, LifecyclePolicy, LifecycleRulesEngine, LifecycleRulesEvaluator,
};
use serde::Serialize;
use tracing::warn;

use crate::lifecycle_rules::action_catalog::{
    LifecycleRiskClass, LifecycleSubjectKind as ActionSubjectKind,
};
use crate::lifecycle_rules::action_sequence::{
    ACTION_SEQUENCE_KIND, LifecycleActionSequence, LifecycleActionStepKind,
};
use crate::lifecycle_rules::declared_actions::LifecycleDeclaredActions;
use crate::lifecycle_rules::evaluation::LifecycleEvaluationTrigger;
use crate::lifecycle_rules::facts::{
    LifecycleLibraryRef, LifecycleTitleClaims, LifecycleTitlePeople, LifecycleTitleWatch,
    build_subject_input, build_subject_input_with_completed_retention_episodes,
};
use crate::lifecycle_rules::safety::{LifecycleActivityCheck, LifecyclePlaybackHold};
use crate::lifecycle_rules::sequence_execution::SequenceCandidateExecutionContext;
use crate::lifecycle_rules::sequence_history::lifecycle_sequence_history_steps;
use crate::lifecycle_rules::service::LifecycleRuleSetDetail;
use crate::ports::LifecycleActionStepCandidateKey;
use crate::{AppError, AppResult, AppUseCase};

/// Due candidates read per page while a pass walks one rule. The walk goes on
/// page after page until the rule's own maximum is reached or nothing due is
/// left; the page only bounds how many rows are held at once.
pub(super) const LIFECYCLE_DUE_CANDIDATE_PAGE_SIZE: usize = 200;

/// High-risk failures in one pass that stop further high-risk work that pass —
/// a circuit breaker against a systematically failing deletion path.
pub const LIFECYCLE_HIGH_RISK_FAILURE_BREAKER: usize = 3;

/// The wire names of the high-risk step kinds.
pub(super) fn high_risk_step_kinds() -> Vec<&'static str> {
    LifecycleActionStepKind::ALL
        .iter()
        .copied()
        .filter(|kind| kind.descriptor().risk_class == LifecycleRiskClass::High)
        .map(LifecycleActionStepKind::as_wire_str)
        .collect()
}

/// How far back another rule's monitoring change keeps this rule from
/// undoing it. Two rules that disagree about a title could otherwise flip it
/// on every cycle; within this window the second one holds.
pub const LIFECYCLE_MONITORING_CONFLICT_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

/// The most (rule, title, change) rows one monitoring-conflict scan reads. A
/// scan that reaches it cannot rule a conflict out, so it holds instead.
const LIFECYCLE_MONITORING_CONFLICT_SCAN_LIMIT: usize = 5_000;

/// Which way a rule, or one candidate's steps, can turn a title's
/// monitoring: on, or off (unmonitoring, or deleting its files).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MonitoringSides {
    monitors: bool,
    unmonitors: bool,
}

impl MonitoringSides {
    /// Judged by each head's base action, so a season or episode head counts
    /// the same as its title head: monitoring part of a show opposes
    /// unmonitoring or deleting any part of it.
    fn declared(declared: &LifecycleDeclaredActions) -> Self {
        use scryer_rules::lifecycle_actions::LifecycleActionHead as Head;
        Self {
            monitors: declared
                .heads
                .iter()
                .any(|head| head.base() == Head::Monitor),
            unmonitors: declared.heads.iter().any(|head| {
                matches!(
                    head.base(),
                    Head::Unmonitor | Head::DeleteFiles | Head::DeleteTitle
                )
            }),
        }
    }

    fn of_sequence(sequence: &LifecycleActionSequence) -> Self {
        Self {
            monitors: sequence
                .steps
                .iter()
                .any(|step| step.kind == LifecycleActionStepKind::Monitor),
            unmonitors: sequence.steps.iter().any(|step| {
                matches!(
                    step.kind,
                    LifecycleActionStepKind::Unmonitor
                        | LifecycleActionStepKind::DeleteFiles
                        | LifecycleActionStepKind::DeleteTitleAndFiles
                )
            }),
        }
    }

    fn opposes(&self, other: &Self) -> bool {
        (self.monitors && other.unmonitors) || (self.unmonitors && other.monitors)
    }
}

/// Other rules' recent monitoring changes, grouped by title once per rule
/// pass: each title's (rule, direction) pairs.
#[derive(Debug, Default)]
pub(super) struct MonitoringActsByTitle(HashMap<String, Vec<(String, MonitoringSides)>>);

impl MonitoringActsByTitle {
    fn from_acts(acts: Vec<crate::LifecycleStepAct>) -> Self {
        let monitor = LifecycleActionStepKind::Monitor.as_wire_str();
        let mut by_title: HashMap<String, Vec<(String, MonitoringSides)>> = HashMap::new();
        for act in acts {
            let sides = MonitoringSides {
                monitors: act.step_kind == monitor,
                unmonitors: act.step_kind != monitor,
            };
            by_title
                .entry(act.title_id)
                .or_default()
                .push((act.rule_set_id, sides));
        }
        Self(by_title)
    }
}

/// The hold for a candidate whose steps would undo another rule's recent
/// monitoring change on the same title, if any. `None` acts is a picture
/// that could not be completed, and holds.
fn monitoring_conflict(
    acts: &Option<MonitoringActsByTitle>,
    candidate: &LifecycleCandidate,
    sequence: &LifecycleActionSequence,
) -> Option<&'static str> {
    let own = MonitoringSides::of_sequence(sequence);
    if !own.monitors && !own.unmonitors {
        return None;
    }
    let Some(acts) = acts else {
        return Some(execution_reason::UNKNOWN_AT_EXECUTION);
    };
    acts.0
        .get(&candidate.title_id)
        .is_some_and(|title_acts| {
            title_acts.iter().any(|(rule_set_id, theirs)| {
                *rule_set_id != candidate.rule_set_id && own.opposes(theirs)
            })
        })
        .then_some(execution_reason::MONITORING_CONFLICT)
}

/// Failed attempts before a candidate is marked terminally failed.
pub const LIFECYCLE_MAX_ACTION_ATTEMPTS: i64 = 3;

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
    pub(super) fn lifecycle_lease_stale_before(
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
/// [`LifecycleCandidateRepository::lease_candidate_for_execution`]'s own stale
/// arm, which is the only writer allowed to decide that a lease is abandoned.
///
/// [`LifecycleCandidateRepository::lease_candidate_for_execution`]: crate::ports::LifecycleCandidateRepository::lease_candidate_for_execution
pub(super) const EXECUTOR_DUE_EXPECTED_STATES: &[LifecycleCandidateState] = &[
    LifecycleCandidateState::Observing,
    LifecycleCandidateState::PendingAction,
    LifecycleCandidateState::Blocked,
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
    /// Reading the subject's items back from the media server found an item
    /// a counted account has made a favourite since the stored favourites
    /// were read, for a rule that reads a favourite fact.
    pub const WATCH_RECHECK_NEWER_FAVORITE: &str = "watch_recheck_newer_favorite";
    /// The title has a grab, download, or import in flight.
    pub const ACTIVE_ACQUISITION: &str = "active_acquisition";
    /// Acquisition activity could not be ruled out.
    pub const ACQUISITION_UNKNOWN: &str = "acquisition_unknown";
    /// A storage-pressure deletion owns the conservative global filesystem
    /// guard. This is a hold because another worker will release it.
    pub const STORAGE_DELETION_IN_PROGRESS: &str = "storage_deletion_in_progress";
    /// The candidate's subject is not a level this build can act at.
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
        WATCH_RECHECK_NEWER_FAVORITE,
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

/// Report of one action pass, the acting half of a rule's own fire.
#[derive(Debug, Default, Serialize)]
pub struct LifecycleActionHandlingReport {
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
pub struct LifecycleActionRunView {
    pub subject_label: String,
    pub run: LifecycleActionRun,
    pub title_name: String,
    /// The steps this run's candidate runs, built from what was decided for
    /// it. `None` when the candidate is gone or its steps cannot be built.
    pub action_sequence: Option<LifecycleActionSequence>,
    /// Ordered step progress. The bounded step rows and durable receipts are
    /// loaded page-wise, never as one history query per run.
    pub sequence_steps: Vec<LifecycleActionStepProgressView>,
}

/// One ordered v2 operation and its latest durable evidence.
#[derive(Clone, Debug)]
pub struct LifecycleActionStepProgressView {
    pub step: crate::lifecycle_rules::LifecycleActionStep,
    pub run: Option<LifecycleActionStepRun>,
    pub receipts: Vec<LifecycleActionJobReceipt>,
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
pub(super) struct LifecycleSequencePassBudget {
    remaining_rule_steps: usize,
    remaining_high_risk_steps: usize,
    dispatched_steps: usize,
    failed_high_risk_steps: usize,
    last_dispatch_was_high_risk: bool,
    last_dispatch_failure_recorded: bool,
}

impl LifecycleSequencePassBudget {
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
    Proceed(Box<Title>, Box<scryer_rules::lifecycle::LifecycleInput>),
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
pub(super) struct LifecycleExecutionContext<'a> {
    pub(super) detail: &'a LifecycleRuleSetDetail,
    /// The steps this candidate runs, built from what was decided for it.
    pub(super) sequence: &'a LifecycleActionSequence,
    pub(super) candidate: &'a LifecycleCandidate,
    pub(super) title: &'a Title,
    pub(super) input: &'a scryer_rules::lifecycle::LifecycleInput,
    pub(super) run: &'a mut LifecycleActionRun,
    pub(super) evaluator: &'a mut LifecycleRulesEvaluator,
    pub(super) libraries: &'a HashMap<String, LifecycleLibraryRef>,
    pub(super) tag_conflicts: &'a Option<HashSet<String>>,
    /// The configured roots, read once for this unit of work however many
    /// safety checks it runs.
    pub(super) storage_roots: &'a super::storage::LifecycleStorageRootsOnce,
}

// ── Rule state ──────────────────────────────────────────────────────────────

/// The state an operator sees. A stored Enabled rule reads as Enabled only
/// while it can act exactly as enabling it would allow: it declares an action,
/// has not been edited since, and holds a deletion confirmation for the
/// revision in force exactly when its actions can delete. Anything else lets
/// nothing execute, so it reads as Dry run.
pub fn lifecycle_rule_state(detail: &LifecycleRuleSetDetail) -> LifecycleRuleState {
    let rule = &detail.rule_set;
    let declared = &detail.declared_actions;
    match rule.state {
        LifecycleRuleState::Disabled => LifecycleRuleState::Disabled,
        LifecycleRuleState::Enabled
            if !rule.edited_since_enabled
                && !declared.heads.is_empty()
                && declared.destructive == rule.destructive_confirmed() =>
        {
            LifecycleRuleState::Enabled
        }
        LifecycleRuleState::DryRun | LifecycleRuleState::Enabled => LifecycleRuleState::DryRun,
    }
}

impl AppUseCase {
    /// Move one rule to Disabled, Dry run, or Enabled.
    ///
    /// Disabled is never evaluated; Dry run is evaluated but never acts.
    /// Enabled may act with exactly the effects the rule's own actions can
    /// have; execution still requires the matching instance gate as well.
    /// Enabling a rule that can delete files is the
    /// elevated, count-acknowledged step: it additionally requires
    /// system-settings authority, a confirmation of the reviewed revision and
    /// library scope, and the current number of non-terminal candidates.
    /// Enabling any other rule needs no confirmation.
    ///
    /// The confirmation never outlives the revision it was given for: what an
    /// operator acknowledged is one source's blast radius, so appending a
    /// revision that can delete drops an Enabled rule back to Dry run and this
    /// call has to be repeated against the new source (see
    /// [`AppUseCase::update_lifecycle_rule_source`]).
    pub async fn set_lifecycle_rule_state(
        &self,
        actor: &User,
        rule_set_id: &str,
        state: LifecycleRuleState,
        confirmation: Option<scryer_domain::LifecycleRuleArmingConfirmation>,
        acknowledged_candidate_count: Option<i64>,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let rule_set = self.require_lifecycle_rule_set(rule_set_id).await?;
        let mut detail = self.load_lifecycle_rule_detail(rule_set).await?;
        let enabling = state == LifecycleRuleState::Enabled;
        // Enabling a rule whose actions can delete is the confirmed step.
        let destructive = enabling && detail.declared_actions.destructive;
        if enabling && let Some(reason) = detail.load_refusal {
            return Err(super::service::stored_actions_refused(reason));
        }
        if enabling && !destructive && detail.declared_actions.heads.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_HAS_NO_EFFECT,
                "this rule's action never changes anything, so it can only run as a dry run",
            ));
        }

        if destructive {
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
                    crate::error_reasons::lifecycle::ARM_CONFIRMATION_STALE,
                    "the rule configuration must be reviewed again before arming deletion; reopen the confirmation",
                ));
            }
            let current = self
                .count_active_lifecycle_candidates(&detail.rule_set.id)
                .await?;
            if acknowledged_candidate_count != Some(current) {
                // The count is a fact beside the translated refusal sentence.
                return Err(AppError::validation_refused_with_count(
                    crate::error_reasons::lifecycle::ARM_COUNT_MISMATCH,
                    format!(
                        "destructive arming requires acknowledging the current candidate count ({current})"
                    ),
                    current,
                ));
            }
        }

        // Only the destructive enable stores a confirmation; the store checks
        // it again under the row lock and writes its revision with the state.
        let destructive_confirmation = confirmation.as_ref().filter(|_| destructive);
        let now = Utc::now();
        let updated = self
            .services
            .customization
            .lifecycle_rule_sets
            .update_rule_set_state(&detail.rule_set.id, state, destructive_confirmation, now)
            .await?;
        if !updated {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_CHANGED_WHILE_CONFIRMING,
                "the rule changed while confirming; review it again before arming deletion",
            ));
        }

        // A rule is scheduled while it is not disabled, so only crossing that
        // line changes what the scheduler holds.
        let schedule_changed =
            detail.rule_set.evaluates() != (state != LifecycleRuleState::Disabled);
        // Mirrors the single store write.
        detail.rule_set.state = state;
        detail.rule_set.edited_since_enabled = false;
        detail.rule_set.destructive_confirmed_revision =
            destructive_confirmation.map(|reviewed| reviewed.revision_number);
        detail.rule_set.updated_at = now;
        if schedule_changed {
            self.reload_lifecycle_rule_schedule().await;
        }
        Ok(detail)
    }

    /// Non-terminal candidates of one rule — the number destructive arming
    /// makes the operator acknowledge.
    pub(crate) async fn count_active_lifecycle_candidates(
        &self,
        rule_set_id: &str,
    ) -> AppResult<i64> {
        Ok(self
            .services
            .customization
            .lifecycle_evaluation
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
    pub(super) async fn lifecycle_sequence_projection(
        &self,
        candidates: &[LifecycleCandidate],
    ) -> AppResult<(
        HashMap<String, LifecycleActionSequence>,
        HashMap<String, Vec<LifecycleActionStepProgressView>>,
    )> {
        let decides_profile = candidates.iter().any(|candidate| {
            super::declared_actions::decided_actions_from_json(
                candidate.decided_actions_json.as_deref(),
            )
            .is_some_and(|actions| actions.set_quality_profile.is_some())
        });
        let quality_profile_ids = if decides_profile {
            self.lifecycle_quality_profile_ids_by_name().await?
        } else {
            HashMap::new()
        };
        let sequences: HashMap<String, LifecycleActionSequence> = candidates
            .iter()
            .filter_map(|candidate| {
                declared_candidate_sequence(candidate, &quality_profile_ids)
                    .ok()
                    .map(|sequence| (candidate.id.clone(), sequence))
            })
            .collect();
        let keys: Vec<LifecycleActionStepCandidateKey> = candidates
            .iter()
            .filter(|candidate| sequences.contains_key(&candidate.id))
            .map(|candidate| LifecycleActionStepCandidateKey {
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
            .lifecycle_evaluation
            .list_action_steps_for_candidates(&keys)
            .await?;
        let step_keys: Vec<_> = runs.iter().map(|run| run.key.clone()).collect();
        let receipts = self
            .services
            .customization
            .lifecycle_evaluation
            .list_action_job_receipts_for_steps(&step_keys)
            .await?;
        let mut runs_by_candidate: HashMap<String, HashMap<String, LifecycleActionStepRun>> =
            HashMap::new();
        for run in runs {
            runs_by_candidate
                .entry(run.key.candidate_id.clone())
                .or_default()
                .insert(run.key.step_id.clone(), run);
        }
        let mut receipts_by_step: HashMap<_, Vec<LifecycleActionJobReceipt>> = HashMap::new();
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
                    LifecycleActionStepProgressView {
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

    pub async fn list_lifecycle_action_runs(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<LifecycleActionRunView>> {
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
            .lifecycle_evaluation
            .list_non_sequence_action_runs(rule_set_id, candidate_id, limit)
            .await?;
        runs.extend(
            self.services
                .customization
                .lifecycle_evaluation
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
            .lifecycle_title_names(runs.iter().map(|run| run.title_id.clone()))
            .await?;
        let summaries = self
            .lifecycle_subject_summaries(
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
            .lifecycle_evaluation
            .get_candidates_by_ids(&candidate_ids)
            .await?;
        let (sequences, sequence_steps) = self.lifecycle_sequence_projection(&candidates).await?;
        Ok(runs
            .into_iter()
            .map(|run| {
                let title_name =
                    crate::lifecycle_rules::evaluation::lifecycle_title_name(&names, &run.title_id);
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
                    lifecycle_sequence_history_steps(&run),
                ) {
                    (Some(sequence), Some(snapshot))
                        if run.status != LifecycleActionRunStatus::Running =>
                    {
                        Some(
                            sequence
                                .steps
                                .iter()
                                .map(|step| LifecycleActionStepProgressView {
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
                LifecycleActionRunView {
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

    /// Manual trigger for one rule: one fire of the rule now, exactly as its
    /// schedule runs it and under the same claim. The rule is evaluated first,
    /// so a candidate the evaluation opens that is already due is acted on in
    /// the same run; then its due candidates are acted on. Rules act only
    /// through their own fires, so a trigger that names no rule is refused.
    pub async fn run_lifecycle_action_handler_now(
        &self,
        actor: &User,
        rule_set_id: Option<String>,
    ) -> AppResult<LifecycleEvaluationTrigger> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let Some(rule_set_id) = rule_set_id else {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::ACTION_RUN_RULE_REQUIRED,
                "each lifecycle rule acts on its own schedule; name the rule to act for it now",
            ));
        };
        let rule_set = self.require_lifecycle_rule_set(&rule_set_id).await?;
        let detail = self.load_lifecycle_rule_detail(rule_set).await?;
        if let Some(reason) = detail.refusal() {
            return Err(super::service::stored_actions_refused(reason));
        }
        let rule_set = detail.rule_set;

        let gates = self.load_lifecycle_gates().await?;
        if !gates.reversible_effects_enabled && !gates.destructive_effects_enabled {
            return Ok(LifecycleEvaluationTrigger {
                started: false,
                message: Some(
                    "both instance effect gates are off, so no actions can execute".to_string(),
                ),
            });
        }

        // The same claim a scheduled fire takes: a rule never acts beside
        // itself, so a manual run and a fire together still act on at most
        // the rule's maximum each, one after the other.
        let Some(claim) = self.claim_lifecycle_rule_run(&rule_set_id) else {
            return Ok(LifecycleEvaluationTrigger {
                started: false,
                message: Some(format!(
                    "lifecycle rule {} is already running",
                    rule_set.name
                )),
            });
        };

        let app = self.clone();
        // Tracked, so a restore drain waits for the run instead of swapping
        // the database under it.
        self.restore_coordinator().spawn_task(async move {
            let _claim = claim;
            // The same fire a schedule runs: evaluate, then act on what is due.
            if let Err(error) = app.run_scheduled_lifecycle_rule(&rule_set_id).await {
                warn!(rule_set_id = %rule_set_id, error = %error, "manual lifecycle rule action run failed");
            }
        });

        Ok(LifecycleEvaluationTrigger {
            started: true,
            message: Some(format!(
                "Lifecycle rule {} is evaluating, then acting on its due titles",
                rule_set.name
            )),
        })
    }
}

// ── The handler ─────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Act on the due candidates of every armed rule in one pass, as the
    /// action tests drive them. Production acts only per rule.
    #[cfg(test)]
    pub(crate) async fn run_every_rule_action_pass_for_tests(
        &self,
    ) -> AppResult<LifecycleActionHandlingReport> {
        self.run_lifecycle_action_handling(None).await
    }

    /// Act on the due candidates of every armed rule, or of one named rule.
    ///
    /// In production a rule's candidates are acted on only by that rule's own
    /// fire, on its schedule or by its manual trigger, so a rule's most titles
    /// per run bounds everything it does between two of its fires. Abandoned
    /// leases are reclaimed by the owning rule's next fire, which selects them
    /// with its due work.
    ///
    /// Scoped to one rule, the other rules are still resolved when the named
    /// one declares tag actions, because the tag-conflict scan has to see what
    /// they are about to write; only the named rule acts.
    pub(crate) async fn run_lifecycle_action_handling(
        &self,
        only_rule_set_id: Option<&str>,
    ) -> AppResult<LifecycleActionHandlingReport> {
        let mut report = LifecycleActionHandlingReport::default();
        let gates = self.load_lifecycle_gates().await?;
        if !gates.reversible_effects_enabled && !gates.destructive_effects_enabled {
            return Ok(report);
        }
        report.gates_enabled = true;

        let mut rule_sets = self
            .services
            .customization
            .lifecycle_rule_sets
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

        let libraries = self.lifecycle_library_refs().await?;
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
                    |(detail, _, _): &(LifecycleRuleSetDetail, EligibleRuleAction, bool)| {
                        detail.rule_set.id.as_str() == only_rule_set_id.unwrap_or_default()
                            && (!detail.declared_actions.add_tags.is_empty()
                                || !detail.declared_actions.remove_tags.is_empty())
                    },
                );
                if !target_writes_tags {
                    break;
                }
            }
            if rule_set.state != LifecycleRuleState::Enabled {
                continue;
            }
            let detail = match self.load_lifecycle_rule_detail(rule_set).await {
                Ok(detail) => detail,
                Err(error) => {
                    warn!(error = %error, "skipping lifecycle rule with unusable revision");
                    continue;
                }
            };
            if let Some(reason) = detail.load_refusal {
                // Its open candidates stay where they are and never run; the
                // rule surfaces the refusal until its source is saved again.
                warn!(
                    rule_set_id = detail.rule_set.id.as_str(),
                    reason, "skipping lifecycle rule whose revision cannot be acted on"
                );
                continue;
            }
            let declared = &detail.declared_actions;
            if declared.heads.is_empty() {
                continue;
            }
            let high_risk = declared.destructive;
            let eligible = lifecycle_rule_state(&detail) == LifecycleRuleState::Enabled
                && (!high_risk || gates.destructive_effects_enabled)
                && (!declared.defines_reversible_action() || gates.reversible_effects_enabled);
            if !eligible {
                continue;
            }
            if let Some(reason) = detail.action_refusal {
                // Saving and arming refuse this shape; a row that reached it
                // anyway acts on nothing rather than on everything, and the
                // rule shows the refusal until a maximum is set.
                warn!(
                    rule_set_id = detail.rule_set.id.as_str(),
                    reason, "skipping lifecycle rule whose actions are refused"
                );
                continue;
            }
            let quality_profile_ids = if declared.quality_profiles.is_empty() {
                HashMap::new()
            } else {
                match self.lifecycle_quality_profile_ids_by_name().await {
                    Ok(ids) => ids,
                    Err(error) => {
                        warn!(error = %error, "skipping lifecycle rule whose quality profiles could not be read");
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

        // A candidate an earlier pass held waits for its rule's next
        // evaluation: when each eligible rule's latest one began, read once
        // for all of them. A rule with no recorded evaluation skips nothing.
        let held_after: HashMap<String, chrono::DateTime<Utc>> = if eligible_rules.is_empty() {
            HashMap::new()
        } else {
            let ids: Vec<String> = eligible_rules
                .iter()
                .map(|(detail, _, _)| detail.rule_set.id.clone())
                .collect();
            self.services
                .customization
                .lifecycle_evaluation
                .rule_schedule_anchors(&ids)
                .await?
                .into_iter()
                .filter_map(|(id, anchor)| anchor.last_run_at.map(|started| (id, started)))
                .collect()
        };

        // Titles two rules would tag in opposite directions this pass. Computed
        // before any of them acts, so both sides hold rather than the later one
        // silently undoing the earlier one.
        let tag_conflicts = self
            .lifecycle_tag_patch_conflicts(&eligible_rules, now)
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
                None => match self.claim_lifecycle_rule_run(&detail.rule_set.id) {
                    Some(claim) => Some(claim),
                    None => continue,
                },
            };
            // One compile and evaluator per rule; every candidate's fresh
            // re-evaluation reuses the evaluator sequentially.
            let engine = match LifecycleRulesEngine::build(&[LifecyclePolicy {
                id: detail.rule_set.id.clone(),
                name: detail.rule_set.name.clone(),
                rego_source: detail.revision.rego_source.clone(),
            }]) {
                Ok(engine) => engine,
                Err(error) => {
                    warn!(error = %error, "skipping lifecycle rule whose matcher no longer compiles");
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
            //
            // A candidate held after the rule's latest evaluation began is
            // left out until the next evaluation: holding it again would
            // repeat the same reads and writes for the same answer. The
            // evaluation that starts each scheduled fire releases every such
            // hold, so a hold that clears by itself is seen again on the next
            // fire and nothing stays held unexamined past it.
            let held_after_evaluation = held_after.get(&detail.rule_set.id).copied();
            let maximum = detail
                .rule_set
                .max_titles_per_run
                .map(|max| usize::try_from(max).unwrap_or(usize::MAX));
            let stale_before = self.lifecycle_lease_stale_before(now);
            // Titles, not candidates: a rule acting at several levels can
            // have a title, a season and an episode of one show due together.
            let mut titles_acted_on: HashSet<String> = HashSet::new();
            let mut cursor: Option<crate::LifecycleDueCursor> = None;
            // What other rules recently did to monitoring, read once for this
            // rule, only once something is due, and only when another
            // eligible rule declares the opposite.
            let mut monitoring_acts: Option<Option<MonitoringActsByTitle>> = None;
            'walk: loop {
                let due = self
                    .services
                    .customization
                    .lifecycle_evaluation
                    .list_due_candidates(
                        &detail.rule_set.id,
                        now,
                        stale_before,
                        held_after_evaluation,
                        cursor.as_ref(),
                        LIFECYCLE_DUE_CANDIDATE_PAGE_SIZE,
                    )
                    .await?;
                let Some(last) = due.last() else {
                    break;
                };
                cursor = Some(crate::LifecycleDueCursor::after(last));
                let last_page = due.len() < LIFECYCLE_DUE_CANDIDATE_PAGE_SIZE;
                if monitoring_acts.is_none() {
                    monitoring_acts = Some(
                        self.lifecycle_monitoring_acts(&detail, &monitoring_sides, now)
                            .await,
                    );
                }
                let monitoring_acts = monitoring_acts
                    .as_ref()
                    .expect("monitoring acts are read before the first candidate");

                for candidate in due {
                    if maximum.is_some_and(|max| {
                        titles_acted_on.len() >= max
                            && !titles_acted_on.contains(&candidate.title_id)
                    }) {
                        // The rest stay due, untouched, for the next fire.
                        report.rules_stopped_at_maximum += 1;
                        break 'walk;
                    }
                    // A restore or shutdown drain waits for this pass. Candidates
                    // not started yet stay due for the next one.
                    if self.restore_coordinator().shutdown_workers.is_cancelled() {
                        break 'walk;
                    }
                    if high_risk && high_risk_failures >= LIFECYCLE_HIGH_RISK_FAILURE_BREAKER {
                        break 'walk;
                    }
                    report.candidates_considered += 1;
                    let candidate_title_id = candidate.title_id.clone();
                    let mut sequence_budget = Some({
                        LifecycleSequencePassBudget::new(
                            usize::MAX,
                            if high_risk_failures >= LIFECYCLE_HIGH_RISK_FAILURE_BREAKER {
                                0
                            } else {
                                usize::MAX
                            },
                        )
                    });
                    // A candidate runs only the steps built from what was decided
                    // for it; one without decided actions never runs.
                    // The latest evaluation found a conflict between this
                    // subject and one that encloses it or that it encloses:
                    // it holds before anything is built or run.
                    let built = if candidate.actions_conflict.is_some() {
                        Err(execution_reason::ACTIONS_CONFLICT)
                    } else {
                        declared_candidate_sequence(&candidate, &action.quality_profile_ids)
                    };
                    let outcome = match built {
                        // Another rule recently did the opposite to this
                        // title's monitoring: hold before any step runs.
                        Ok(sequence)
                            if let Some(code) =
                                monitoring_conflict(monitoring_acts, &candidate, &sequence) =>
                        {
                            if candidate.state == LifecycleCandidateState::Blocked
                                && candidate.state_reason == code
                            {
                                // Already held for it; nothing is written.
                                Ok(CandidateOutcome::Held)
                            } else {
                                self.hold_unbuilt_lifecycle_candidate(&candidate, code)
                                    .await
                            }
                        }
                        Ok(sequence) => {
                            let storage_roots = super::storage::LifecycleStorageRootsOnce::new();
                            let budget = sequence_budget
                                .as_mut()
                                .expect("sequence actions always have a step budget");
                            if self
                                .lifecycle_sequence_hold_is_unchanged(
                                    &candidate,
                                    SequenceCandidateExecutionContext {
                                        detail: &detail,
                                        sequence: &sequence,
                                        evaluator: &mut evaluator,
                                        libraries: &libraries,
                                        tag_conflicts: &tag_conflicts,
                                        storage_roots: &storage_roots,
                                        budget,
                                    },
                                )
                                .await
                            {
                                // Already held for it; nothing is written.
                                Ok(CandidateOutcome::Held)
                            } else {
                                self.execute_one_lifecycle_sequence_candidate(
                                    candidate,
                                    SequenceCandidateExecutionContext {
                                        detail: &detail,
                                        sequence: &sequence,
                                        evaluator: &mut evaluator,
                                        libraries: &libraries,
                                        tag_conflicts: &tag_conflicts,
                                        storage_roots: &storage_roots,
                                        budget,
                                    },
                                )
                                .await
                            }
                        }
                        Err(reason)
                            if candidate.state == LifecycleCandidateState::Blocked
                                && candidate.state_reason == reason =>
                        {
                            // Already held for it; nothing is written.
                            Ok(CandidateOutcome::Held)
                        }
                        Err(reason) => {
                            self.hold_unbuilt_lifecycle_candidate(&candidate, reason)
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
                            titles_acted_on.insert(candidate_title_id);
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
                            warn!(error = %error, "lifecycle action handling failed for one candidate");
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
    async fn lifecycle_tag_patch_conflicts(
        &self,
        eligible_rules: &[(LifecycleRuleSetDetail, EligibleRuleAction, bool)],
        now: chrono::DateTime<Utc>,
    ) -> Option<HashSet<String>> {
        let mut added: HashMap<String, HashSet<String>> = HashMap::new();
        let mut removed: HashMap<String, HashSet<String>> = HashMap::new();

        for (detail, _, _) in eligible_rules {
            let declared = &detail.declared_actions;
            if declared.add_tags.is_empty() && declared.remove_tags.is_empty() {
                continue;
            }
            let stale_before = self.lifecycle_lease_stale_before(now);
            let mut cursor: Option<crate::LifecycleDueCursor> = None;
            loop {
                let due = match self
                    .services
                    .customization
                    .lifecycle_evaluation
                    .list_due_candidates(
                        &detail.rule_set.id,
                        now,
                        stale_before,
                        // Held rows included: a candidate held earlier and
                        // waiting for its rule's next evaluation still means
                        // to patch, so the other side must not act around it.
                        None,
                        cursor.as_ref(),
                        LIFECYCLE_DUE_CANDIDATE_PAGE_SIZE,
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
                cursor = Some(crate::LifecycleDueCursor::after(last));
                let last_page = due.len() < LIFECYCLE_DUE_CANDIDATE_PAGE_SIZE;
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
    /// [`LIFECYCLE_MONITORING_CONFLICT_WINDOW_SECONDS`], for one rule about
    /// to run, grouped by title. One bounded read, made only when another
    /// eligible rule declares the opposite direction; otherwise nothing is
    /// read. `None` is a picture Scryer could not complete: a failed read, or
    /// one that hit its bound, which holds this rule's monitoring work for the
    /// pass.
    async fn lifecycle_monitoring_acts(
        &self,
        detail: &LifecycleRuleSetDetail,
        monitoring_sides: &[(String, MonitoringSides)],
        now: chrono::DateTime<Utc>,
    ) -> Option<MonitoringActsByTitle> {
        let own = MonitoringSides::declared(&detail.declared_actions);
        let opposed = monitoring_sides
            .iter()
            .any(|(rule_set_id, other)| *rule_set_id != detail.rule_set.id && own.opposes(other));
        if !opposed {
            return Some(MonitoringActsByTitle::default());
        }
        let kinds = [
            LifecycleActionStepKind::Monitor.as_wire_str(),
            LifecycleActionStepKind::Unmonitor.as_wire_str(),
            LifecycleActionStepKind::DeleteFiles.as_wire_str(),
            LifecycleActionStepKind::DeleteTitleAndFiles.as_wire_str(),
        ];
        match self
            .services
            .customization
            .lifecycle_evaluation
            .list_step_acts_since(
                &kinds,
                now - chrono::Duration::seconds(LIFECYCLE_MONITORING_CONFLICT_WINDOW_SECONDS),
                LIFECYCLE_MONITORING_CONFLICT_SCAN_LIMIT,
            )
            .await
        {
            Ok(acts) if acts.len() < LIFECYCLE_MONITORING_CONFLICT_SCAN_LIMIT => {
                Some(MonitoringActsByTitle::from_acts(acts))
            }
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

    /// The ordered pre-execution rechecks. Order
    /// matters: eligibility first so a disarmed rule never even re-evaluates,
    /// the matcher before the environment so "stopped matching" reads as a
    /// cancel rather than a hold, and the environment holds last.
    pub(super) async fn lifecycle_execution_safety_checks(
        &self,
        detail: &LifecycleRuleSetDetail,
        sequence: &LifecycleActionSequence,
        evaluator: &mut LifecycleRulesEvaluator,
        candidate: &LifecycleCandidate,
        libraries: &HashMap<String, LifecycleLibraryRef>,
        tag_conflicts: &Option<HashSet<String>>,
        storage_roots: &super::storage::LifecycleStorageRootsOnce,
    ) -> SafetyDecision {
        // (1) Re-read the rule and the gates: both can move between selection
        // and execution, and a lowered gate must stop a leased worker before
        // its first external change.
        let fresh = match self
            .require_lifecycle_rule_set(&candidate.rule_set_id)
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
        let gates = match self.load_lifecycle_gates().await {
            Ok(gates) => gates,
            Err(_) => return SafetyDecision::Hold(execution_reason::RULE_NOT_ELIGIBLE),
        };
        if detail.refusal().is_some()
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
        // The candidate's subject is the level its actions were decided at;
        // each decided action must be one the revision defines at that level.
        let Some(level) = super::evaluation::subject_level(&candidate.subject_kind) else {
            return SafetyDecision::Hold(execution_reason::ACTION_NOT_FULLY_SUPPORTED);
        };
        if decided
            .heads()
            .iter()
            .any(|head| !detail.declared_actions.defines_at(*head, level))
        {
            return SafetyDecision::Hold(execution_reason::ACTION_NOT_DECLARED);
        }
        let high_risk = sequence
            .steps
            .iter()
            .any(|step| step.descriptor().risk_class == LifecycleRiskClass::High);
        let sequence_requires_reversible = sequence.requires_reversible_gate();
        // A step that deletes runs only under the confirmation the operator
        // gave for exactly this candidate's revision, which must still be the
        // one in force.
        let still_eligible = fresh.state == LifecycleRuleState::Enabled
            && fresh.current_revision_number == candidate.revision_number
            && if high_risk {
                gates.destructive_effects_enabled
                    && fresh.destructive_confirmed()
                    && fresh.destructive_confirmed_revision == Some(candidate.revision_number)
                    && !fresh.edited_since_enabled
                    && (!sequence_requires_reversible || gates.reversible_effects_enabled)
            } else {
                gates.reversible_effects_enabled
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
            .lifecycle_evaluation
            .list_exclusions(Some(&candidate.rule_set_id))
            .await
        {
            Ok(exclusions) => exclusions,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
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
                .lifecycle_sequence_moved_title(candidate, sequence, &title)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    return SafetyDecision::Cancel(
                        crate::lifecycle_rules::evaluation::candidate_reason::OUT_OF_SCOPE,
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
                &crate::location::ownership_guard::LIFECYCLE_ACTION_ENTRY,
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
            .lifecycle_claims_for_titles(std::slice::from_ref(&title))
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
            .lifecycle_files_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(files) => files,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let library = libraries
            .get(&title.library_id)
            .cloned()
            .unwrap_or_else(|| LifecycleLibraryRef {
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
            .lifecycle_series_movies_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(series_movies) => series_movies,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        // A people lookup that fails holds, like every other unresolvable
        // signal on this path: re-evaluating on a fact snapshot Scryer could
        // not fully assemble is how an action fires on evidence it never had.
        let requesters_by_title = match self
            .lifecycle_requesters_for_titles(std::slice::from_ref(&title))
            .await
        {
            Ok(requesters) => requesters,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let usernames = match self.lifecycle_usernames_by_id().await {
            Ok(usernames) => usernames,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        // Watch signals are resolved on the same fail-closed terms: a gate or
        // signal read that errors holds, and a gate that is merely closed makes
        // every watch fact unknown, which holds any rule that reads one.
        let watch_context = match self.lifecycle_watch_context().await {
            Ok(context) => context,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let signals_by_title = match self
            .lifecycle_watch_signals_for_titles(&watch_context, std::slice::from_ref(&title))
            .await
        {
            Ok(signals) => signals,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let (watch_context, audience_by_title) = self
            .lifecycle_audience_plays_for_titles(&watch_context, std::slice::from_ref(&title))
            .await;
        let people = LifecycleTitlePeople {
            requester_user_ids: requesters_by_title.get(&title.id).map(Vec::as_slice),
            usernames: &usernames,
        };
        let watch = LifecycleTitleWatch {
            context: &watch_context,
            signals: signals_by_title.get(&title.id).map(Vec::as_slice),
            audience: audience_by_title.get(&title.id).map(Vec::as_slice),
        };
        let claims = LifecycleTitleClaims {
            claims: claims_by_title
                .as_ref()
                .and_then(|claims| claims.get(&title.id))
                .map(Vec::as_slice),
            store_readable: claims_by_title.is_some(),
        };
        let subject = match self
            .lifecycle_resolve_subject(
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
            .find(|step| step.kind == LifecycleActionStepKind::DeleteFiles)
            .map(|step| step.id.as_str());
        let deletion_checkpoint = if let Some(step_id) = sequence_delete_files_step_id {
            match self
                .lifecycle_deletion_checkpoint_for_step(candidate, Some(step_id))
                .await
            {
                Ok(checkpoint) => checkpoint,
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        } else {
            None
        };
        let completed_retention_episode_ids = deletion_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.completed_retention_episode_ids());
        let series_movies = series_movies_by_title
            .get(&title.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let subject_input = build_subject_input_with_completed_retention_episodes(
            recheck_time,
            &title,
            &library,
            &subject,
            people,
            watch,
            claims,
            series_movies,
            completed_retention_episode_ids.as_ref(),
        );
        // A season or episode is evaluated beside its title's own facts,
        // read now like everything else here.
        let title_subject = if subject.kind == scryer_domain::LifecycleRuleSubjectKind::Title {
            None
        } else {
            match self
                .lifecycle_resolve_subject("title", &title.id, &title, files)
                .await
            {
                Ok(title_subject) => Some(title_subject),
                Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
            }
        };
        let storage_roots = match storage_roots
            .get_or_try_init(|| self.lifecycle_storage_roots_context())
            .await
        {
            Ok(roots) => roots,
            Err(_) => return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION),
        };
        let mut capacities = super::storage::LifecycleStorageCapacityMemo::default();
        // Re-read like everything else here: a title that joined a list since
        // the pass must not be removed on the pass's stale answer. A store
        // that cannot be read leaves the list facts unknown, which holds any
        // rule that reads them.
        let lists_by_title = self
            .lifecycle_list_facts_loading(
                std::slice::from_ref(&title),
                super::list_facts::LifecycleFactLoads::for_rule(
                    &detail.revision.rego_source,
                    &detail.rule_set.id,
                ),
                &watch_context,
                recheck_time,
            )
            .await;
        // The capacity figures must be fresh for this lease. Baseline restore
        // below deliberately excludes storage facts, so a resumed deletion
        // cannot match against the capacity it observed before deleting.
        let mut title_input = title_subject.as_ref().map(|title_subject| {
            build_subject_input(
                recheck_time,
                &title,
                &library,
                title_subject,
                people,
                watch,
                claims,
                series_movies,
            )
        });
        let mut subject_input = subject_input;
        for (input, files) in [
            (Some(&mut subject_input), subject.files.as_slice()),
            (
                title_input.as_mut(),
                title_subject
                    .as_ref()
                    .map(|title_subject| title_subject.files.as_slice())
                    .unwrap_or_default(),
            ),
        ] {
            let Some(input) = input else { continue };
            super::list_facts::apply_list_facts(input, &lists_by_title, &title.id);
            super::list_facts::apply_watchlist_and_metadata_facts(
                input,
                &lists_by_title,
                &title.id,
                people,
            );
            self.populate_lifecycle_storage_roots(
                &mut input.facts,
                files,
                &storage_roots,
                &mut capacities,
                super::storage::LifecycleStorageRead::Refresh,
            )
            .await;
        }
        // A season or episode whose decided heads include one that conflicts
        // with a title head the rule declares is checked against what the
        // rule decides for its title now, from the title input built above.
        // The stored heads gate the copy: the recheck below holds before the
        // title is evaluated unless the fresh decision equals the stored one.
        // Any other subject keeps no copy and evaluates nothing more.
        let candidate_level = super::evaluation::subject_level(&candidate.subject_kind)
            .filter(|level| *level != scryer_rules::lifecycle::LifecycleSubjectKind::Title);
        let cross_level_title_input = match (title_input.as_ref(), candidate_level) {
            (Some(title_input), Some(level))
                if super::declared_actions::decided_actions_from_json(
                    candidate.decided_actions_json.as_deref(),
                )
                .is_some_and(|stored| {
                    super::cross_level::heads_conflict_with_declared(
                        &super::cross_level::heads_at_level(&stored, level),
                        &detail.declared_actions,
                        scryer_rules::lifecycle::LifecycleSubjectKind::Title,
                    )
                }) =>
            {
                Some((title_input.clone(), level))
            }
            _ => None,
        };
        let mut input = match title_input {
            Some(title_input) => {
                super::facts::nest_subject_input(title_input, subject_input, &subject)
            }
            None => subject_input,
        };
        if super::storage::rule_reads_storage(&detail.revision.rego_source)
            && !matches!(
                input.subject_facts().storage_roots,
                scryer_rules::lifecycle::Observation::Known { .. }
            )
        {
            return SafetyDecision::Hold(execution_reason::UNKNOWN_AT_EXECUTION);
        }
        {
            let completed_steps = match self
                .services
                .customization
                .lifecycle_evaluation
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
            if let Err(reason) = self
                .restore_sequence_own_effects(
                    candidate,
                    &title,
                    &sequence,
                    &completed_steps,
                    deletion_checkpoint.as_ref(),
                    &mut input,
                    SequenceOwnEffects::All,
                    OwnMonitoringEvidence::Catalog,
                )
                .await
            {
                return SafetyDecision::Hold(reason);
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
            Some(LifecycleOutcome::Match) => {
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
                // The title now deciding an action that conflicts with this
                // season's or episode's holds it, as the evaluator's marker
                // would. A title the rule does not match, or cannot decide,
                // adds nothing here: the marker the latest evaluation wrote
                // already covers a conflict it saw.
                if let (Some((title_input, level)), Some(decision)) =
                    (cross_level_title_input, decision.as_ref())
                    && super::cross_level::heads_conflict_with_declared(
                        &super::cross_level::heads_at_level(&decision.actions, level),
                        &detail.declared_actions,
                        scryer_rules::lifecycle::LifecycleSubjectKind::Title,
                    )
                {
                    let title_decision = evaluator.evaluate(&title_input).ok().and_then(|result| {
                        result
                            .records
                            .into_iter()
                            .next()
                            .map(|record| record.decision)
                    });
                    if let Some(title_decision) = title_decision
                        && title_decision.outcome == LifecycleOutcome::Match
                        && super::cross_level::first_conflict(
                            &super::cross_level::heads_at_level(&decision.actions, level),
                            &super::cross_level::heads_at_level(
                                &title_decision.actions,
                                scryer_rules::lifecycle::LifecycleSubjectKind::Title,
                            ),
                        )
                        .is_some()
                    {
                        return SafetyDecision::Hold(execution_reason::ACTIONS_CONFLICT);
                    }
                }
            }
            Some(LifecycleOutcome::NoMatch) => {
                return SafetyDecision::Cancel(execution_reason::NO_MATCH_AT_EXECUTION);
            }
            Some(LifecycleOutcome::Unknown) | None => {
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
            let episode_ids = (subject.kind != scryer_domain::LifecycleRuleSubjectKind::Title)
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
                    .lifecycle_stored_watch_signals_for_titles(std::slice::from_ref(&title))
                    .await
                {
                    Ok(mut stored) => stored.remove(&title.id).unwrap_or_default(),
                    Err(_) => {
                        return SafetyDecision::Hold(execution_reason::WATCH_RECHECK_UNAVAILABLE);
                    }
                }
            };
            // A rule that reads a favourite fact also compares each item's
            // live favourite flags, carried in the same answers, with the
            // favourites stored for the title.
            let favorites = if super::watch_recheck::rule_reads_favorite_facts(
                &detail.revision.rego_source,
                &candidate.rule_set_id,
            ) {
                match self
                    .services
                    .integrations
                    .media_server_signals
                    .audience_favorites_for_titles(std::slice::from_ref(&title.id))
                    .await
                {
                    Ok(mut stored) => Some(stored.remove(&title.id).unwrap_or_default()),
                    Err(_) => {
                        return SafetyDecision::Hold(execution_reason::WATCH_RECHECK_UNAVAILABLE);
                    }
                }
            } else {
                None
            };
            let recheck = self
                .lifecycle_watch_recheck(
                    &watch_context,
                    &baseline,
                    audience_by_title
                        .get(&title.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    episode_ids.as_ref(),
                    entities.as_deref(),
                    favorites.as_deref(),
                )
                .await;
            if let Some(reason) = recheck.hold_reason() {
                return SafetyDecision::Hold(reason);
            }
        }

        // (4) Playback holds every action kind, unknown included.
        match self.lifecycle_playback_hold().await {
            Ok(LifecyclePlaybackHold::Clear) => {}
            Ok(LifecyclePlaybackHold::Hold { .. }) => {
                return SafetyDecision::Hold(execution_reason::PLAYBACK_HOLD);
            }
            Ok(LifecyclePlaybackHold::Unknown { .. }) | Err(_) => {
                return SafetyDecision::Hold(execution_reason::PLAYBACK_UNKNOWN);
            }
        }

        // (5) Acquisition activity, unknown included.
        match self.title_has_active_acquisition(&candidate.title_id).await {
            Ok(LifecycleActivityCheck::Clear) => {}
            Ok(LifecycleActivityCheck::Active) => {
                return SafetyDecision::Hold(execution_reason::ACTIVE_ACQUISITION);
            }
            Ok(LifecycleActivityCheck::Unknown { .. }) | Err(_) => {
                return SafetyDecision::Hold(execution_reason::ACQUISITION_UNKNOWN);
            }
        }

        SafetyDecision::Proceed(Box::new(title), Box::new(input))
    }
}

/// Which of a sequence's own effects [`AppUseCase::restore_sequence_own_effects`]
/// puts back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SequenceOwnEffects {
    /// Everything a completed step can change: monitoring, files, library,
    /// tags, and quality profile. The executor's recheck before a step.
    All,
    /// Monitoring and file facts only: what a delete and its unmonitor
    /// prerequisite change. Evaluation of a candidate whose delete started.
    MonitoringAndFiles,
}

/// Where [`AppUseCase::restore_sequence_own_effects`] confirms that a
/// completed monitoring step's own change still holds. Neither reads a media
/// file or probes the filesystem.
#[derive(Clone, Copy)]
pub(super) enum OwnMonitoringEvidence<'a> {
    /// Read the title, its seasons and its episodes from the catalog now.
    Catalog,
    /// The candidate's subject as this evaluation pass resolved it, with the
    /// pass's title row: no read at all.
    Pass(&'a super::subjects::LifecycleSubject),
}

impl AppUseCase {
    /// Put back onto `input` what this candidate's own completed steps
    /// changed, so the rule is judged on the subject as it was before its own
    /// work rather than on that work half done.
    ///
    /// File counts and sizes come back only for the deletion journal's own
    /// completed targets, and only while the live figures equal the baseline
    /// minus exactly those targets: an import or an outside removal leaves
    /// them live. Capacity figures are never restored.
    ///
    /// Sequence baselines are intentionally narrow. A completed Unmonitor
    /// may keep only its own monitored fact stable for later steps, and
    /// only after the persisted postcondition still holds. An external
    /// re-monitor is live evidence, so it is never hidden by a baseline.
    ///
    /// `completed_steps` is the candidate's step rows for its current match
    /// generation and revision. An `Err` is the hold reason.
    pub(super) async fn restore_sequence_own_effects(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
        sequence: &LifecycleActionSequence,
        completed_steps: &[LifecycleActionStepRun],
        deletion_checkpoint: Option<&super::scoped_deletion::ScopedDeletionCheckpoint>,
        input: &mut scryer_rules::lifecycle::LifecycleInput,
        effects: SequenceOwnEffects,
        evidence: OwnMonitoringEvidence<'_>,
    ) -> Result<(), &'static str> {
        if let Some(checkpoint) = deletion_checkpoint {
            checkpoint.restore_sequence_owned_file_deltas(input);
        }
        let sequence_hash = match sequence.content_hash() {
            Ok(hash) => hash,
            Err(_) => return Err(execution_reason::RULE_NOT_ELIGIBLE),
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
                return Err(execution_reason::RULE_NOT_ELIGIBLE);
            }
            let before = serde_json::from_str::<serde_json::Value>(&run.before_state_json)
                .map_err(|_| ())
                .ok();
            let provenance = serde_json::from_str::<serde_json::Value>(&run.provenance_json)
                .map_err(|_| ())
                .ok();
            match step.kind {
                crate::lifecycle_rules::LifecycleActionStepKind::Unmonitor => {
                    let crate::lifecycle_rules::LifecycleActionStepParameters::Unmonitor {
                        include_descendants,
                    } = &step.parameters
                    else {
                        return Err(execution_reason::RULE_NOT_ELIGIBLE);
                    };
                    let monitoring_changed = provenance
                        .as_ref()
                        .and_then(|value| value.get("monitoring_changed"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(true);
                    // A step that changed nothing has nothing to put back.
                    if !monitoring_changed {
                        continue;
                    }
                    match self
                        .lifecycle_own_unmonitoring_holds(
                            candidate,
                            title,
                            *include_descendants,
                            evidence,
                        )
                        .await
                    {
                        Ok(true) => {
                            if let Some(before_monitored) = before
                                .as_ref()
                                .and_then(|value| value.get("facts_monitored"))
                                .and_then(serde_json::Value::as_bool)
                            {
                                input.subject_facts_mut().monitored =
                                    scryer_rules::lifecycle::Observation::known(before_monitored);
                            }
                            if let Some(before_count) = before
                                .as_ref()
                                .and_then(|value| value.get("facts_monitored_episode_count"))
                                .and_then(serde_json::Value::as_i64)
                            {
                                input.subject_facts_mut().monitored_episode_count =
                                    scryer_rules::lifecycle::Observation::known(before_count);
                            }
                        }
                        // A later external re-monitor is live input for
                        // the matcher. Do not restore our earlier value
                        // or manufacture a no-match here: DeleteFiles
                        // independently rechecks this destructive
                        // prerequisite immediately before every unlink.
                        Ok(false) => {}
                        Err(_) => {
                            return Err(execution_reason::UNKNOWN_AT_EXECUTION);
                        }
                    }
                }
                crate::lifecycle_rules::LifecycleActionStepKind::Monitor => {
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
                        .lifecycle_own_monitoring_holds(candidate, title, evidence)
                        .await
                    {
                        Ok(true) => {
                            if let Some(before_monitored) = before
                                .as_ref()
                                .and_then(|value| value.get("facts_monitored"))
                                .and_then(serde_json::Value::as_bool)
                            {
                                input.subject_facts_mut().monitored =
                                    scryer_rules::lifecycle::Observation::known(before_monitored);
                            }
                        }
                        Ok(false) => {}
                        Err(_) => {
                            return Err(execution_reason::UNKNOWN_AT_EXECUTION);
                        }
                    }
                }
                crate::lifecycle_rules::LifecycleActionStepKind::MoveToLibrary
                | crate::lifecycle_rules::LifecycleActionStepKind::MoveToRoot
                    if effects == SequenceOwnEffects::All =>
                {
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
                        input.library = scryer_rules::lifecycle::LifecycleLibraryDoc {
                            id: id.to_string(),
                            name: name.to_string(),
                        };
                    }
                }
                crate::lifecycle_rules::LifecycleActionStepKind::AddTags
                | crate::lifecycle_rules::LifecycleActionStepKind::RemoveTags
                    if effects == SequenceOwnEffects::All =>
                {
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
                    if let scryer_rules::lifecycle::Observation::Known { value, .. } =
                        &mut input.facts.tags
                    {
                        for tag in changed_tags {
                            match step.kind {
                                crate::lifecycle_rules::LifecycleActionStepKind::AddTags
                                    if value.contains(&tag) =>
                                {
                                    value.retain(|current| current != &tag)
                                }
                                crate::lifecycle_rules::LifecycleActionStepKind::RemoveTags
                                    if !value.contains(&tag) =>
                                {
                                    value.push(tag)
                                }
                                _ => {}
                            }
                        }
                    }
                }
                crate::lifecycle_rules::LifecycleActionStepKind::ChangeQualityProfile
                    if effects == SequenceOwnEffects::All =>
                {
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
                            scryer_rules::lifecycle::Observation::Known { value, .. }
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
                                    scryer_rules::lifecycle::Observation::known(
                                        previous.to_string(),
                                    );
                            }
                            None => {
                                input.facts.quality_profile_id =
                                    scryer_rules::lifecycle::Observation::absent();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// A delete a candidate's own sequence already began: the sequence, its step
/// rows, and the deletion journal once the delete itself has started.
pub(crate) struct StartedSequenceDelete {
    pub(super) sequence: LifecycleActionSequence,
    pub(super) steps: Vec<LifecycleActionStepRun>,
    pub(super) checkpoint: Option<super::scoped_deletion::ScopedDeletionCheckpoint>,
}

#[cfg(test)]
impl StartedSequenceDelete {
    /// The sequence's step ids, the step rows read, and whether a journal
    /// was read, for comparing two reads of the same started delete.
    pub(crate) fn comparable(&self) -> (Vec<String>, Vec<LifecycleActionStepRun>, bool) {
        (
            self.sequence
                .steps
                .iter()
                .map(|step| step.id.clone())
                .collect(),
            self.steps.clone(),
            self.checkpoint.is_some(),
        )
    }
}

/// The sequence a started delete belongs to and the two steps that say it
/// has begun.
struct StartedDeletePlan {
    sequence: LifecycleActionSequence,
    delete_step_id: String,
    unmonitor_step_id: Option<String>,
}

/// Candidates per batched step-row read: the store's page bound, three
/// binds each, well inside both dialects' bind limits.
const STARTED_DELETE_STEP_READ_PAGE: usize = 100;

/// What one rule pass reads once for every candidate's started delete: the
/// quality profile ids, loaded the first time a candidate needs them, and
/// the step rows read ahead for the candidates of the current title chunk,
/// each beside the candidate row it was read for.
#[derive(Default)]
pub(crate) struct StartedDeleteReads {
    quality_profile_ids: Option<HashMap<String, String>>,
    steps: HashMap<String, (LifecycleCandidate, Vec<LifecycleActionStepRun>)>,
}

#[cfg(test)]
impl StartedDeleteReads {
    /// How many candidates have step rows read ahead and not yet used.
    pub(crate) fn read_ahead_len(&self) -> usize {
        self.steps.len()
    }
}

impl AppUseCase {
    /// Whether a completed Unmonitor step's change still holds: the subject
    /// and every episode under it are unmonitored, and for a title with
    /// `include_descendants` every season and episode too. Answers exactly
    /// as [`Self::lifecycle_unmonitoring_is_confirmed`], without reading a
    /// media file or probing one.
    async fn lifecycle_own_unmonitoring_holds(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
        include_descendants: bool,
        evidence: OwnMonitoringEvidence<'_>,
    ) -> AppResult<bool> {
        let title_kind = scryer_domain::LifecycleRuleSubjectKind::Title.as_storage_str();
        if candidate.title_id != title.id
            || (candidate.subject_kind == title_kind && candidate.subject_id != title.id)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::CANDIDATE_TITLE_MISMATCH,
                "lifecycle unmonitoring candidate identity does not match its title",
            ));
        }
        if candidate.subject_kind != title_kind {
            let resolved;
            let subject = match evidence {
                OwnMonitoringEvidence::Pass(subject) => pass_subject(candidate, subject)?,
                OwnMonitoringEvidence::Catalog => {
                    resolved = self
                        .lifecycle_resolve_subject_monitoring(
                            &candidate.subject_kind,
                            &candidate.subject_id,
                            title,
                        )
                        .await?;
                    &resolved
                }
            };
            return Ok(
                !subject.monitored && subject.episodes.iter().all(|episode| !episode.monitored)
            );
        }
        match evidence {
            OwnMonitoringEvidence::Pass(subject) => {
                let subject = pass_subject(candidate, subject)?;
                if title.monitored || !include_descendants {
                    return Ok(!title.monitored);
                }
                let (collections, episodes) = subject
                    .inventory
                    .as_deref()
                    .map(super::subjects::LifecycleTitleEpisodeInventory::collections_and_episodes)
                    .unwrap_or((&[], &[]));
                Ok(descendants_unmonitored(&title.id, collections, episodes))
            }
            OwnMonitoringEvidence::Catalog => {
                let Some(fresh_title) = self.services.catalog.titles.get_by_id(&title.id).await?
                else {
                    return Err(AppError::not_found_reason(
                        crate::error_reasons::catalog::TITLE_NOT_FOUND,
                        format!("title {}", title.id),
                    ));
                };
                if fresh_title.monitored || !include_descendants {
                    return Ok(!fresh_title.monitored);
                }
                let collections = self
                    .services
                    .catalog
                    .shows
                    .list_collections_for_title(&title.id)
                    .await?;
                let episodes = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await?;
                Ok(descendants_unmonitored(&title.id, &collections, &episodes))
            }
        }
    }

    /// Whether a completed Monitor step's change still holds: the subject is
    /// monitored. Answers exactly as [`Self::lifecycle_monitoring_is_confirmed`],
    /// without reading a media file or probing one.
    async fn lifecycle_own_monitoring_holds(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
        evidence: OwnMonitoringEvidence<'_>,
    ) -> AppResult<bool> {
        let title_kind = candidate.subject_kind
            == scryer_domain::LifecycleRuleSubjectKind::Title.as_storage_str();
        match evidence {
            OwnMonitoringEvidence::Pass(subject) => {
                let subject = pass_subject(candidate, subject)?;
                Ok(if title_kind {
                    title.monitored
                } else {
                    subject.monitored
                })
            }
            OwnMonitoringEvidence::Catalog if title_kind => {
                let Some(fresh) = self.services.catalog.titles.get_by_id(&title.id).await? else {
                    return Err(AppError::not_found_reason(
                        crate::error_reasons::catalog::TITLE_NOT_FOUND,
                        format!("title {}", title.id),
                    ));
                };
                Ok(fresh.monitored)
            }
            OwnMonitoringEvidence::Catalog => Ok(self
                .lifecycle_resolve_subject_monitoring(
                    &candidate.subject_kind,
                    &candidate.subject_id,
                    title,
                )
                .await?
                .monitored),
        }
    }

    /// The sequence a candidate's started delete would belong to, with its
    /// DeleteFiles and Unmonitor step ids. `None` for a candidate that is
    /// observing or executing, whose decided actions delete no files, or
    /// whose sequence cannot be built. Reads the quality profile ids the
    /// first time the pass meets a candidate that needs them.
    async fn lifecycle_started_delete_plan(
        &self,
        candidate: &LifecycleCandidate,
        reads: &mut StartedDeleteReads,
    ) -> AppResult<Option<StartedDeletePlan>> {
        if candidate.action_kind != ACTION_SEQUENCE_KIND
            || matches!(
                candidate.state,
                LifecycleCandidateState::Observing | LifecycleCandidateState::Executing
            )
        {
            return Ok(None);
        }
        let Some(actions) = super::declared_actions::decided_actions_from_json(
            candidate.decided_actions_json.as_deref(),
        ) else {
            return Ok(None);
        };
        if !actions.delete_files || actions.delete_title {
            return Ok(None);
        }
        let needs_profiles = actions.set_quality_profile.is_some();
        if needs_profiles && reads.quality_profile_ids.is_none() {
            reads.quality_profile_ids = Some(self.lifecycle_quality_profile_ids_by_name().await?);
        }
        let no_profiles = HashMap::new();
        let quality_profile_ids = match reads.quality_profile_ids.as_ref() {
            Some(ids) if needs_profiles => ids,
            _ => &no_profiles,
        };
        let Ok(sequence) = declared_candidate_sequence(candidate, quality_profile_ids) else {
            return Ok(None);
        };
        let step_of = |kind: LifecycleActionStepKind| {
            sequence
                .steps
                .iter()
                .find(|step| step.kind == kind)
                .map(|step| step.id.clone())
        };
        let Some(delete_step_id) = step_of(LifecycleActionStepKind::DeleteFiles) else {
            return Ok(None);
        };
        let unmonitor_step_id = step_of(LifecycleActionStepKind::Unmonitor);
        Ok(Some(StartedDeletePlan {
            sequence,
            delete_step_id,
            unmonitor_step_id,
        }))
    }

    /// Read ahead, in batched statements, the step rows of every candidate
    /// among `candidates` whose started delete
    /// [`Self::lifecycle_started_sequence_delete`] would read. A failed read
    /// is not an answer: the candidates it covered read their own rows later,
    /// exactly as without the read-ahead.
    pub(crate) async fn lifecycle_read_ahead_started_delete_steps(
        &self,
        candidates: Vec<LifecycleCandidate>,
        reads: &mut StartedDeleteReads,
    ) {
        reads.steps.clear();
        let mut wanted = Vec::new();
        for candidate in candidates {
            if let Ok(Some(_)) = self.lifecycle_started_delete_plan(&candidate, reads).await {
                wanted.push(candidate);
            }
        }
        for page in wanted.chunks(STARTED_DELETE_STEP_READ_PAGE) {
            let keys: Vec<LifecycleActionStepCandidateKey> = page
                .iter()
                .map(|candidate| LifecycleActionStepCandidateKey {
                    candidate_id: candidate.id.clone(),
                    match_generation: candidate.match_generation,
                    revision_number: candidate.revision_number,
                })
                .collect();
            let rows = match self
                .services
                .customization
                .lifecycle_evaluation
                .list_action_steps_for_candidates(&keys)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "could not read lifecycle step rows ahead; each candidate reads its own"
                    );
                    continue;
                }
            };
            let mut by_candidate: HashMap<String, Vec<LifecycleActionStepRun>> = HashMap::new();
            for row in rows {
                by_candidate
                    .entry(row.key.candidate_id.clone())
                    .or_default()
                    .push(row);
            }
            for candidate in page {
                let steps = by_candidate.remove(&candidate.id).unwrap_or_default();
                reads
                    .steps
                    .insert(candidate.id.clone(), (candidate.clone(), steps));
            }
        }
    }

    /// The delete this candidate's sequence already began, if any: its
    /// Unmonitor prerequisite succeeded, or its DeleteFiles step left
    /// `pending`. Reads nothing for a candidate that is observing or
    /// executing, or whose decided actions delete no files. Any other
    /// candidate, a due or pending one included even when the executor has
    /// never leased it, has its step rows read: from the pass's read-ahead
    /// when that read saw this exact candidate row, otherwise on its own.
    /// The deletion journal is read only when the DeleteFiles step itself
    /// has begun. An `Err` is a picture that could not be completed.
    pub(crate) async fn lifecycle_started_sequence_delete(
        &self,
        candidate: &LifecycleCandidate,
        reads: &mut StartedDeleteReads,
    ) -> AppResult<Option<StartedSequenceDelete>> {
        let Some(StartedDeletePlan {
            sequence,
            delete_step_id,
            unmonitor_step_id,
        }) = self.lifecycle_started_delete_plan(candidate, reads).await?
        else {
            return Ok(None);
        };
        // The executor writes step rows only under a lease, and taking one
        // rewrites the candidate's state and `updated_at`. Rows read ahead
        // for a candidate row identical to this one are therefore current.
        let steps = match reads.steps.remove(&candidate.id) {
            Some((read_with, steps)) if read_with == *candidate => steps,
            _ => {
                self.services
                    .customization
                    .lifecycle_evaluation
                    .list_action_steps(
                        &candidate.id,
                        candidate.match_generation,
                        candidate.revision_number,
                    )
                    .await?
            }
        };
        let delete_started = steps.iter().any(|run| {
            run.key.step_id == delete_step_id
                && run.state != scryer_domain::LifecycleActionStepState::Pending
        });
        let unmonitored = steps.iter().any(|run| {
            Some(&run.key.step_id) == unmonitor_step_id.as_ref()
                && run.state.is_completion_success()
        });
        if !delete_started && !unmonitored {
            return Ok(None);
        }
        let checkpoint = if delete_started {
            self.lifecycle_deletion_checkpoint_for_step(candidate, Some(&delete_step_id))
                .await?
        } else {
            None
        };
        Ok(Some(StartedSequenceDelete {
            sequence,
            steps,
            checkpoint,
        }))
    }
}

/// The pass's subject, refused when it is not the candidate's.
fn pass_subject<'a>(
    candidate: &LifecycleCandidate,
    subject: &'a super::subjects::LifecycleSubject,
) -> AppResult<&'a super::subjects::LifecycleSubject> {
    if subject.id != candidate.subject_id || subject.kind.as_storage_str() != candidate.subject_kind
    {
        return Err(AppError::Repository(
            "lifecycle pass subject does not match its candidate".into(),
        ));
    }
    Ok(subject)
}

/// Every season and episode of the title is unmonitored.
fn descendants_unmonitored(
    title_id: &str,
    collections: &[scryer_domain::Collection],
    episodes: &[scryer_domain::Episode],
) -> bool {
    collections
        .iter()
        .all(|collection| collection.title_id == title_id && !collection.monitored)
        && episodes
            .iter()
            .all(|episode| episode.title_id == title_id && !episode.monitored)
}

/// The sequence a declared rule runs for one candidate, built from the
/// actions stored on it. An `Err` is the hold reason.
pub(crate) fn declared_candidate_sequence(
    candidate: &LifecycleCandidate,
    quality_profile_ids: &HashMap<String, String>,
) -> Result<LifecycleActionSequence, &'static str> {
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
    // A cross-level conflict marker is checked by the executor, not here, so
    // a marked candidate still shows its sequence and step progress.
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
    pub(crate) async fn lifecycle_quality_profile_ids_by_name(
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
    async fn hold_unbuilt_lifecycle_candidate(
        &self,
        candidate: &LifecycleCandidate,
        reason: &'static str,
    ) -> AppResult<CandidateOutcome> {
        let candidates = &self.services.customization.lifecycle_evaluation;
        let now = Utc::now();
        if candidate.state != LifecycleCandidateState::Executing
            && candidate.state != LifecycleCandidateState::Due
            && !candidates
                .transition_candidate_state(
                    &candidate.id,
                    LifecycleCandidateState::Due,
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
                self.lifecycle_lease_stale_before(now),
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
                LifecycleCandidateState::Blocked,
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

#[cfg(test)]
mod monitoring_conflict_lookup_tests {
    use super::*;
    use crate::lifecycle_rules::{LifecycleActionStep, LifecycleActionStepParameters};

    fn candidate(rule_set_id: &str, title_id: &str) -> LifecycleCandidate {
        let now = Utc::now();
        LifecycleCandidate {
            id: format!("{rule_set_id}-{title_id}"),
            rule_set_id: rule_set_id.to_string(),
            revision_number: 1,
            source_content_hash: "hash".to_string(),
            title_id: title_id.to_string(),
            library_id: "library-a".to_string(),
            facet: "movie".to_string(),
            subject_kind: "title".to_string(),
            subject_id: title_id.to_string(),
            match_generation: 1,
            state: LifecycleCandidateState::Due,
            state_reason: "due".to_string(),
            reason_codes: vec![],
            action_kind: ACTION_SEQUENCE_KIND.to_string(),
            decided_actions_json: None,
            actions_conflict: None,
            grace_days: 0,
            first_matched_at: now,
            last_matched_at: now,
            due_at: now,
            last_evaluated_at: now,
            held_since: None,
            action_attempts: 0,
            created_at: now,
            updated_at: now,
        }
    }

    fn sequence(kind: LifecycleActionStepKind) -> LifecycleActionSequence {
        let parameters = match kind {
            LifecycleActionStepKind::Unmonitor => LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
            _ => LifecycleActionStepParameters::None,
        };
        LifecycleActionSequence::new(vec![LifecycleActionStep {
            id: "only".to_string(),
            kind,
            parameters,
        }])
    }

    fn act(
        rule_set_id: &str,
        title_id: &str,
        kind: LifecycleActionStepKind,
    ) -> crate::LifecycleStepAct {
        crate::LifecycleStepAct {
            rule_set_id: rule_set_id.to_string(),
            title_id: title_id.to_string(),
            step_kind: kind.as_wire_str().to_string(),
        }
    }

    /// The linear scan the grouped lookup replaced.
    fn scanned(
        acts: &[crate::LifecycleStepAct],
        candidate: &LifecycleCandidate,
        sequence: &LifecycleActionSequence,
    ) -> Option<&'static str> {
        let own = MonitoringSides::of_sequence(sequence);
        if !own.monitors && !own.unmonitors {
            return None;
        }
        let monitor = LifecycleActionStepKind::Monitor.as_wire_str();
        acts.iter()
            .filter(|act| {
                act.title_id == candidate.title_id && act.rule_set_id != candidate.rule_set_id
            })
            .any(|act| {
                own.opposes(&MonitoringSides {
                    monitors: act.step_kind == monitor,
                    unmonitors: act.step_kind != monitor,
                })
            })
            .then_some(execution_reason::MONITORING_CONFLICT)
    }

    #[test]
    fn the_grouped_lookup_answers_as_the_linear_scan() {
        use LifecycleActionStepKind as Kind;
        let acts = vec![
            act("rule-b", "title-1", Kind::Monitor),
            act("rule-a", "title-1", Kind::Unmonitor),
            act("rule-b", "title-2", Kind::DeleteFiles),
            act("rule-a", "title-3", Kind::Monitor),
            act("rule-c", "title-3", Kind::DeleteTitleAndFiles),
        ];
        let grouped = Some(MonitoringActsByTitle::from_acts(acts.clone()));
        let mut conflicts = 0;
        for rule in ["rule-a", "rule-b", "rule-c"] {
            for title in ["title-1", "title-2", "title-3", "title-4"] {
                for kind in [
                    Kind::Monitor,
                    Kind::Unmonitor,
                    Kind::DeleteFiles,
                    Kind::DeleteTitleAndFiles,
                    Kind::Search,
                ] {
                    let candidate = candidate(rule, title);
                    let sequence = sequence(kind);
                    let expected = scanned(&acts, &candidate, &sequence);
                    conflicts += usize::from(expected.is_some());
                    assert_eq!(
                        monitoring_conflict(&grouped, &candidate, &sequence),
                        expected,
                        "{rule} {title} {kind:?}"
                    );
                }
            }
        }
        assert!(conflicts > 0, "the grid includes conflicts");
    }

    #[test]
    fn an_incomplete_picture_holds_only_monitoring_work() {
        let candidate = candidate("rule-a", "title-1");
        assert_eq!(
            monitoring_conflict(
                &None,
                &candidate,
                &sequence(LifecycleActionStepKind::Unmonitor)
            ),
            Some(execution_reason::UNKNOWN_AT_EXECUTION)
        );
        assert_eq!(
            monitoring_conflict(
                &None,
                &candidate,
                &sequence(LifecycleActionStepKind::Search)
            ),
            None
        );
    }
}
