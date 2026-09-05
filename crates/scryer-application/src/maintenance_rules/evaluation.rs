//! Scheduled dark evaluation of stored maintenance rules (RFC 137 tracks
//! C1/C2, sections 7.5, 8, 10, 11).
//!
//! # What this module may do
//!
//! Evaluate enabled rule sets on a schedule, reconcile durable lifecycle
//! candidates, honour exclusions, and record bounded per-rule evaluation runs.
//!
//! # What it must not do
//!
//! Execute an action of any kind. Nothing here mutates media, monitoring, tags,
//! files, or any external system: the only states it ever writes are
//! [`MaintenanceCandidateState::Observing`], [`MaintenanceCandidateState::Canceled`],
//! and [`MaintenanceCandidateState::Excluded`]. The remaining states belong to
//! the executor wave and are already pinned in the domain enum so that wave
//! needs no migration.
//!
//! # Fail-closed shape
//!
//! Every failure path holds rather than advances. An `unknown` decision, a
//! per-title evaluation error, and a matcher that no longer compiles all leave
//! the existing candidate exactly where it was; only a confirmed `no_match`
//! cancels one. That is the RFC 9.8 rule ("transient rule-evaluation failure
//! preserves membership and holds action") expressed as code.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    AppPermission, Id, LifecycleCandidate, MaintenanceCandidateState, MaintenanceEvaluationMode,
    MaintenanceEvaluationRun, MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion,
    MaintenanceRuleSet, Title, User,
};
use scryer_rules::maintenance::{
    MaintenanceOutcome, MaintenancePolicy, MaintenanceRulesEngine, MaintenanceRulesEvaluator,
    MaintenanceSeriesMovieDoc,
};
use serde::Serialize;
use tracing::{debug, warn};

use crate::jobs::{JobKey, JobTriggerSource};
use crate::maintenance_rules::facts::{
    MaintenanceLibraryRef, MaintenanceTitlePeople, MaintenanceTitleWatch, MaintenanceWatchContext,
    WATCH_SIGNAL_FRESHNESS_HOURS, WatchSignalFreshness, build_title_input, unknown_reason,
};
use crate::maintenance_rules::service::MaintenanceRuleSetDetail;
use crate::media_server_signals::SIGNAL_SYNC_PROVIDERS;
use crate::ports::MaintenanceCandidateQuery;
use crate::settings::keys::{
    MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY, MAINTENANCE_GATE_EVALUATION_KEY,
    MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY, MAINTENANCE_GATE_RESULT_DISPLAY_KEY,
    MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY,
};
use crate::{AppError, AppResult, AppUseCase};

/// How many titles one batched media-file load covers. The evaluator is a
/// background job, not a latency path, so the chunk exists to bound peak memory
/// and query size rather than to go fast.
pub const MAINTENANCE_EVALUATION_TITLE_CHUNK: usize = 200;

/// The candidate states the evaluator is allowed to move a row out of.
///
/// [`MaintenanceCandidateState::Executing`] is deliberately absent: that state
/// means the action handler holds a lease, and the two jobs are independently
/// schedulable. Every evaluator write is a compare-and-set against this set, so
/// an evaluation pass that runs while an action is executing can neither cancel
/// the row out from under the executor nor overwrite the terminal state the
/// executor just wrote.
const EVALUATOR_WRITABLE_STATES: &[MaintenanceCandidateState] = &[
    MaintenanceCandidateState::Observing,
    MaintenanceCandidateState::PendingAction,
    MaintenanceCandidateState::Due,
    MaintenanceCandidateState::Blocked,
];

/// Stable `state_reason` values written by the evaluator. They are part of the
/// operator-visible contract: a UI filtering on "why did this cancel" compares
/// against exactly these strings.
///
/// A repeat match deliberately has no reason of its own: `state_reason` answers
/// "why is the candidate in this state", and a continuing membership has not
/// changed state. It stays [`FIRST_MATCH`] until something actually moves it.
pub mod candidate_reason {
    /// A subject started matching and a candidate was opened.
    pub const FIRST_MATCH: &str = "first_match";
    /// The subject stopped matching.
    pub const NO_MATCH: &str = "no_match";
    /// The rule's matcher changed, so the candidate's revision no longer
    /// describes what the rule means (RFC 7.1).
    pub const REVISION_SUPERSEDED: &str = "revision_superseded";
    /// A global or per-rule exclusion now covers the subject.
    pub const EXCLUDED: &str = "excluded";
    /// The rule's library scope no longer covers the subject — either the scope
    /// was re-pointed at other libraries, or the subject left the catalog.
    pub const OUT_OF_SCOPE: &str = "out_of_scope";
    /// The subject was merged into another title (US7). Its live candidate
    /// closes and follows the surviving title as history; the survivor is
    /// re-evaluated on its own facts at the next pass.
    pub const MERGED_INTO_DESTINATION: &str = "merged_into_destination";
}

// ── Instance gates ──────────────────────────────────────────────────────────

/// The five independent instance-wide gates (RFC 137 section 10).
///
/// All default off, and a missing settings row reads as off, so an instance
/// that has never been configured evaluates nothing. This wave consumes only
/// `evaluation` and `result_display`; the other three are stored for the
/// executor wave so an operator's arming survives that upgrade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceGates {
    pub evaluation_enabled: bool,
    pub result_display_enabled: bool,
    pub presentation_effects_enabled: bool,
    pub reversible_effects_enabled: bool,
    pub destructive_effects_enabled: bool,
}

/// A partial gate update. `None` leaves the stored value alone, so a client can
/// arm one gate without restating the others.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaintenanceGatesUpdate {
    pub evaluation_enabled: Option<bool>,
    pub result_display_enabled: Option<bool>,
    pub presentation_effects_enabled: Option<bool>,
    pub reversible_effects_enabled: Option<bool>,
    pub destructive_effects_enabled: Option<bool>,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A candidate with the two names a reader needs, both batch-resolved.
///
/// `title_name` falls back to the stored `title_id` when the title is gone: a
/// deleted subject must not fail the whole listing, and the id is still the
/// honest answer to "what was this".
#[derive(Clone, Debug)]
pub struct MaintenanceCandidateView {
    pub candidate: LifecycleCandidate,
    pub rule_name: String,
    pub title_name: String,
}

#[derive(Clone, Debug)]
pub struct MaintenanceExclusionView {
    pub exclusion: MaintenanceRuleExclusion,
    pub title_name: String,
}

/// Which candidates a reader wants.
#[derive(Clone, Debug, Default)]
pub struct MaintenanceCandidateFilter {
    pub rule_set_id: Option<String>,
    pub states: Vec<MaintenanceCandidateState>,
    pub library_id: Option<String>,
    /// Shadow is dark by default (RFC C1). Candidates produced by a rule in
    /// shadow mode are returned only when this is set.
    pub include_shadow: bool,
    pub limit: Option<usize>,
}

/// Answer to a manual run request.
#[derive(Clone, Debug)]
pub struct MaintenanceEvaluationTrigger {
    pub started: bool,
    pub message: Option<String>,
}

/// What one whole evaluation pass did, across every rule it considered.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceEvaluationReport {
    /// False when the instance evaluation gate is off, in which case every
    /// other number is zero and nothing at all was written.
    pub gate_enabled: bool,
    pub rules_considered: usize,
    pub rules_evaluated: usize,
    pub rules_failed: usize,
    pub titles_evaluated: i64,
    pub candidates_created: i64,
    pub candidates_matched: i64,
    pub candidates_canceled: i64,
    pub candidates_superseded: i64,
    pub candidates_excluded: i64,
    pub candidates_held: i64,
}

/// Per-rule tallies, kept separate from the report so one rule's failure cannot
/// corrupt another rule's counts.
#[derive(Clone, Copy, Debug, Default)]
struct RuleCounts {
    evaluated: i64,
    matched: i64,
    no_match: i64,
    unknown: i64,
    errors: i64,
    created: i64,
    canceled: i64,
    superseded: i64,
    excluded: i64,
    held: i64,
}

// ── Gates ───────────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Read the gates with no permission check. The job reads them at the start
    /// of every run, which is what makes a gate change take effect on the next
    /// run without a restart.
    pub(crate) async fn load_maintenance_gates(&self) -> AppResult<MaintenanceGates> {
        Ok(MaintenanceGates {
            evaluation_enabled: self
                .maintenance_gate(MAINTENANCE_GATE_EVALUATION_KEY)
                .await?,
            result_display_enabled: self
                .maintenance_gate(MAINTENANCE_GATE_RESULT_DISPLAY_KEY)
                .await?,
            presentation_effects_enabled: self
                .maintenance_gate(MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY)
                .await?,
            reversible_effects_enabled: self
                .maintenance_gate(MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY)
                .await?,
            destructive_effects_enabled: self
                .maintenance_gate(MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY)
                .await?,
        })
    }

    /// A missing or unparseable row is off. Losing the settings table disarms
    /// maintenance; it never arms it.
    async fn maintenance_gate(&self, key: &str) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(key, None)
            .await?
            .unwrap_or(false))
    }

    /// Read the gates. Instance-wide arming is a system setting, so it is
    /// gated like one, not like the catalog-settings authoring surface.
    pub async fn maintenance_instance_gates(&self, actor: &User) -> AppResult<MaintenanceGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.load_maintenance_gates().await
    }

    /// Arm or disarm gates. Omitted fields are left exactly as stored.
    pub async fn set_maintenance_instance_gates(
        &self,
        actor: &User,
        update: MaintenanceGatesUpdate,
    ) -> AppResult<MaintenanceGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;

        for (key, value) in [
            (MAINTENANCE_GATE_EVALUATION_KEY, update.evaluation_enabled),
            (
                MAINTENANCE_GATE_RESULT_DISPLAY_KEY,
                update.result_display_enabled,
            ),
            (
                MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY,
                update.presentation_effects_enabled,
            ),
            (
                MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY,
                update.reversible_effects_enabled,
            ),
            (
                MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY,
                update.destructive_effects_enabled,
            ),
        ] {
            if let Some(value) = value {
                self.upsert_system_setting_json(key, &value, Some(actor.id.clone()))
                    .await?;
            }
        }

        self.load_maintenance_gates().await
    }
}

// ── Rule mode ───────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Move a rule set between evaluation modes.
    ///
    /// `enabled` is derived from the mode rather than accepted separately: an
    /// enabled-but-disabled-mode row would be a state the evaluator has no
    /// reading for. Creation is untouched and still always produces a disabled
    /// rule, so arming is always a deliberate second step.
    pub async fn set_maintenance_rule_evaluation_mode(
        &self,
        actor: &User,
        rule_set_id: &str,
        mode: MaintenanceEvaluationMode,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        let enabled = mode != MaintenanceEvaluationMode::Disabled;
        let now = Utc::now();

        self.services
            .customization
            .maintenance_rule_sets
            .update_rule_set_evaluation_mode(&rule_set.id, mode, enabled, now)
            .await?;

        rule_set.evaluation_mode = mode;
        rule_set.enabled = enabled;
        rule_set.updated_at = now;
        self.load_maintenance_rule_detail(rule_set).await
    }
}

// ── Exclusions ──────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Exclusions, optionally narrowed to those that apply to one rule: that
    /// rule's own rows plus every global row, because both are what actually
    /// stop it acting.
    pub async fn list_maintenance_exclusions(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<MaintenanceExclusionView>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let exclusions = self
            .services
            .customization
            .maintenance_evaluation
            .list_exclusions(rule_set_id)
            .await?;
        let names = self
            .maintenance_title_names(exclusions.iter().map(|row| row.title_id.clone()))
            .await?;

        Ok(exclusions
            .into_iter()
            .map(|exclusion| MaintenanceExclusionView {
                title_name: maintenance_title_name(&names, &exclusion.title_id),
                exclusion,
            })
            .collect())
    }

    /// Exclude a subject globally (`rule_set_id` omitted) or for one rule.
    ///
    /// The row takes effect at the next evaluation, which is also where an
    /// existing candidate for the subject is moved to `Excluded`. Writing that
    /// transition here would mean the mutation reached into candidate state it
    /// does not own.
    pub async fn exclude_maintenance_subject(
        &self,
        actor: &User,
        title_id: &str,
        rule_set_id: Option<String>,
        reason: Option<String>,
    ) -> AppResult<MaintenanceExclusionView> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id} not found")))?;
        if let Some(rule_set_id) = rule_set_id.as_deref() {
            self.require_maintenance_rule_set(rule_set_id).await?;
        }

        let exclusion = MaintenanceRuleExclusion {
            id: Id::new().0,
            rule_set_id,
            title_id: title.id.clone(),
            reason: reason.unwrap_or_default(),
            created_by: Some(actor.id.clone()),
            created_at: Utc::now(),
        };
        self.services
            .customization
            .maintenance_evaluation
            .create_exclusion(&exclusion)
            .await?;

        Ok(MaintenanceExclusionView {
            exclusion,
            title_name: title.name,
        })
    }

    pub async fn remove_maintenance_exclusion(&self, actor: &User, id: &str) -> AppResult<String> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .customization
            .maintenance_evaluation
            .get_exclusion(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("maintenance exclusion {id} not found")))?;
        self.services
            .customization
            .maintenance_evaluation
            .delete_exclusion(id)
            .await?;
        Ok(id.to_string())
    }
}

// ── Candidate and run reads ─────────────────────────────────────────────────

impl AppUseCase {
    /// Candidates, with rule and title names resolved in two batched lookups.
    ///
    /// Two independent things hide rows here, and they are deliberately not the
    /// same thing:
    ///
    /// * a rule in `shadow` mode is dark by definition (RFC C1), so its
    ///   candidates appear only when the caller asks for them explicitly; and
    /// * the instance `result_display` gate hides everything else.
    ///
    /// `include_shadow` overrides the gate as well as the mode filter. That is
    /// deliberate: an operator deciding whether to arm `result_display` has to
    /// be able to see what shadow evaluation actually found first, and the
    /// query already requires catalog-settings management to reach at all.
    pub async fn list_maintenance_candidates(
        &self,
        actor: &User,
        filter: MaintenanceCandidateFilter,
    ) -> AppResult<Vec<MaintenanceCandidateView>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let gates = self.load_maintenance_gates().await?;
        if !gates.result_display_enabled && !filter.include_shadow {
            return Ok(Vec::new());
        }

        let rule_sets = self
            .services
            .customization
            .maintenance_rule_sets
            .list_rule_sets()
            .await?;
        let rules_by_id: HashMap<String, MaintenanceRuleSet> = rule_sets
            .into_iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set))
            .collect();

        let candidates = self
            .services
            .customization
            .maintenance_evaluation
            .list_candidates(&MaintenanceCandidateQuery {
                rule_set_id: filter.rule_set_id.clone(),
                states: filter.states.clone(),
                library_id: filter.library_id.clone(),
                limit: filter.limit,
            })
            .await?;

        let visible: Vec<LifecycleCandidate> = candidates
            .into_iter()
            .filter(|candidate| {
                let Some(rule_set) = rules_by_id.get(&candidate.rule_set_id) else {
                    // The FK cascades, so this is only reachable for a rule
                    // deleted between the two reads. Hiding the row is the
                    // conservative answer.
                    return false;
                };
                filter.include_shadow
                    || rule_set.evaluation_mode != MaintenanceEvaluationMode::Shadow
            })
            .collect();

        let names = self
            .maintenance_title_names(visible.iter().map(|candidate| candidate.title_id.clone()))
            .await?;

        Ok(visible
            .into_iter()
            .map(|candidate| MaintenanceCandidateView {
                rule_name: rules_by_id
                    .get(&candidate.rule_set_id)
                    .map(|rule_set| rule_set.name.clone())
                    .unwrap_or_default(),
                title_name: maintenance_title_name(&names, &candidate.title_id),
                candidate,
            })
            .collect())
    }

    /// Evaluation runs carry counts and timing, never subjects, so they are not
    /// hidden by the result-display gate: they are how an operator sees that
    /// dark evaluation is working at all.
    pub async fn list_maintenance_evaluation_runs(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<MaintenanceEvaluationRun>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .maintenance_evaluation
            .list_evaluation_runs(rule_set_id, limit)
            .await
    }

    pub(crate) async fn maintenance_title_names(
        &self,
        title_ids: impl Iterator<Item = String>,
    ) -> AppResult<HashMap<String, String>> {
        let ids: Vec<String> = title_ids.collect::<HashSet<String>>().into_iter().collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(self
            .services
            .catalog
            .titles
            .get_by_ids(&ids)
            .await?
            .into_iter()
            .map(|title| (title.id, title.name))
            .collect())
    }
}

/// A subject whose title is gone renders as its stored id rather than blank.
pub(crate) fn maintenance_title_name(names: &HashMap<String, String>, title_id: &str) -> String {
    names
        .get(title_id)
        .cloned()
        .unwrap_or_else(|| title_id.to_string())
}

// ── Manual trigger ──────────────────────────────────────────────────────────

impl AppUseCase {
    /// Run evaluation now.
    ///
    /// Unscoped, this goes through the ordinary job seam so the run appears in
    /// the system-jobs surface exactly like the scheduled pass, and returns as
    /// soon as it is accepted. Scoped to one rule, it runs inline instead: the
    /// job seam carries no parameters, and a single rule over its own library
    /// scope is bounded by the same per-rule work the scheduled pass already
    /// does for it.
    pub async fn run_maintenance_evaluation_now(
        &self,
        actor: &User,
        rule_set_id: Option<String>,
    ) -> AppResult<MaintenanceEvaluationTrigger> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        if !self.load_maintenance_gates().await?.evaluation_enabled {
            return Ok(MaintenanceEvaluationTrigger {
                started: false,
                message: Some(
                    "the instance maintenance evaluation gate is off, so nothing was evaluated"
                        .to_string(),
                ),
            });
        }

        if let Some(rule_set_id) = rule_set_id {
            let rule_set = self.require_maintenance_rule_set(&rule_set_id).await?;
            if rule_set.evaluation_mode == MaintenanceEvaluationMode::Disabled {
                return Ok(MaintenanceEvaluationTrigger {
                    started: false,
                    message: Some(format!(
                        "maintenance rule {} is disabled, so it was not evaluated",
                        rule_set.name
                    )),
                });
            }
            let report = self.evaluate_maintenance_rules(Some(&rule_set_id)).await?;
            return Ok(MaintenanceEvaluationTrigger {
                started: true,
                message: Some(format!(
                    "Evaluated {} title(s): {} candidate(s) opened, {} canceled",
                    report.titles_evaluated, report.candidates_created, report.candidates_canceled
                )),
            });
        }

        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::MaintenanceRuleEvaluation)
            .await
        {
            return Ok(MaintenanceEvaluationTrigger {
                started: false,
                message: Some("a maintenance evaluation run is already in progress".to_string()),
            });
        }

        let app = self.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::MaintenanceRuleEvaluation, JobTriggerSource::Manual)
                .await
            {
                warn!(error = %error, "manual maintenance evaluation run failed");
            }
        });

        Ok(MaintenanceEvaluationTrigger {
            started: true,
            message: Some("Maintenance evaluation started".to_string()),
        })
    }
}

// ── The evaluator ───────────────────────────────────────────────────────────

impl AppUseCase {
    /// Job body for [`JobKey::MaintenanceRuleEvaluation`].
    pub(crate) async fn run_maintenance_rule_evaluation_job(
        &self,
    ) -> AppResult<MaintenanceEvaluationReport> {
        self.evaluate_maintenance_rules(None).await
    }

    /// One evaluation pass over every enabled rule, or over one named rule.
    ///
    /// The evaluation time is captured once for the whole pass so every subject
    /// in it sees the same clock (RFC 7.2). A single rule's failure is counted
    /// and logged; it never aborts the pass.
    pub(crate) async fn evaluate_maintenance_rules(
        &self,
        only_rule_set_id: Option<&str>,
    ) -> AppResult<MaintenanceEvaluationReport> {
        let mut report = MaintenanceEvaluationReport::default();

        // Read at run start, not at registration: flipping a gate has to take
        // effect on the next run without a restart.
        if !self.load_maintenance_gates().await?.evaluation_enabled {
            return Ok(report);
        }
        report.gate_enabled = true;

        let evaluation_time = Utc::now();
        let rule_sets = self
            .services
            .customization
            .maintenance_rule_sets
            .list_rule_sets()
            .await?;
        let libraries = self.maintenance_library_refs().await?;

        for rule_set in rule_sets {
            if only_rule_set_id.is_some_and(|id| id != rule_set.id) {
                continue;
            }
            report.rules_considered += 1;

            // A disabled rule is skipped whole: its candidates are left exactly
            // as they are, so flipping a rule off and on again does not destroy
            // the membership and clocks it already established.
            if rule_set.evaluation_mode == MaintenanceEvaluationMode::Disabled {
                continue;
            }

            match self
                .evaluate_one_maintenance_rule(&rule_set, evaluation_time, &libraries)
                .await
            {
                Ok(counts) => {
                    report.rules_evaluated += 1;
                    report.titles_evaluated += counts.evaluated;
                    report.candidates_created += counts.created;
                    report.candidates_matched += counts.matched;
                    report.candidates_canceled += counts.canceled;
                    report.candidates_superseded += counts.superseded;
                    report.candidates_excluded += counts.excluded;
                    report.candidates_held += counts.held;
                }
                Err(error) => {
                    report.rules_failed += 1;
                    warn!(
                        rule_set_id = rule_set.id.as_str(),
                        error = %error,
                        "maintenance rule evaluation failed; its candidates are held"
                    );
                }
            }
        }

        Ok(report)
    }

    /// Evaluate one rule set, recording a run row around the work.
    ///
    /// The run row is inserted before evaluation begins so an interrupted pass
    /// leaves a `running` row rather than no evidence at all.
    async fn evaluate_one_maintenance_rule(
        &self,
        rule_set: &MaintenanceRuleSet,
        evaluation_time: DateTime<Utc>,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
    ) -> AppResult<RuleCounts> {
        let detail = self
            .load_maintenance_rule_detail(rule_set.clone())
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "maintenance rule {} has no usable revision: {error}",
                    rule_set.id
                ))
            })?;

        let mut run = MaintenanceEvaluationRun {
            id: Id::new().0,
            rule_set_id: rule_set.id.clone(),
            revision_number: detail.revision.revision_number,
            matcher_content_hash: detail.revision.matcher_content_hash.clone(),
            started_at: evaluation_time,
            finished_at: None,
            status: MaintenanceEvaluationRunStatus::Running,
            evaluated_count: 0,
            matched_count: 0,
            no_match_count: 0,
            unknown_count: 0,
            error_count: 0,
            canceled_candidates: 0,
            superseded_candidates: 0,
            duration_ms: None,
            error: None,
        };
        self.services
            .customization
            .maintenance_evaluation
            .start_evaluation_run(&run)
            .await?;

        let outcome = self
            .reconcile_maintenance_rule(&detail, evaluation_time, libraries)
            .await;

        let finished_at = Utc::now();
        run.finished_at = Some(finished_at);
        run.duration_ms = Some((finished_at - evaluation_time).num_milliseconds().max(0));
        match &outcome {
            Ok(counts) => {
                run.status = MaintenanceEvaluationRunStatus::Succeeded;
                run.evaluated_count = counts.evaluated;
                run.matched_count = counts.matched;
                run.no_match_count = counts.no_match;
                run.unknown_count = counts.unknown;
                run.error_count = counts.errors;
                run.canceled_candidates = counts.canceled;
                run.superseded_candidates = counts.superseded;
            }
            Err(error) => {
                run.status = MaintenanceEvaluationRunStatus::Failed;
                run.error = Some(error.to_string());
            }
        }
        if let Err(error) = self
            .services
            .customization
            .maintenance_evaluation
            .finish_evaluation_run(&run)
            .await
        {
            warn!(
                rule_set_id = rule_set.id.as_str(),
                error = %error,
                "could not record the maintenance evaluation run"
            );
        }

        outcome
    }

    /// Compile the current revision once, then reconcile every scoped title
    /// against it.
    ///
    /// A compile failure returns before a single candidate is touched. That is
    /// the hold the RFC asks for: a rule that cannot be evaluated must preserve
    /// the membership it already established rather than cancel it.
    async fn reconcile_maintenance_rule(
        &self,
        detail: &MaintenanceRuleSetDetail,
        evaluation_time: DateTime<Utc>,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
    ) -> AppResult<RuleCounts> {
        let rule_set = &detail.rule_set;
        let engine = MaintenanceRulesEngine::build(&[MaintenancePolicy {
            id: rule_set.id.clone(),
            name: rule_set.name.clone(),
            rego_source: detail.revision.rego_source.clone(),
        }])
        .map_err(|error| {
            AppError::Validation(format!(
                "maintenance rule {} failed to compile: {error}",
                rule_set.id
            ))
        })?;
        let mut evaluator = engine.evaluator();

        let excluded = self.maintenance_excluded_title_ids(&rule_set.id).await?;
        let titles = self.maintenance_scoped_titles(rule_set).await?;
        let in_scope: HashSet<String> = titles.iter().map(|title| title.id.clone()).collect();
        // Users are read once for the whole rule, not per chunk: the roster is
        // small and bounded, and one snapshot keeps every subject in the run
        // resolving names against the same view, exactly like the clock.
        let usernames = self.maintenance_usernames_by_id().await?;
        // The watch gate is whole-instance and, like the roster, is read once
        // per rule rather than per chunk.
        let watch_context = self.maintenance_watch_context().await?;

        let mut counts = RuleCounts::default();
        for chunk in titles.chunks(MAINTENANCE_EVALUATION_TITLE_CHUNK) {
            let files_by_title = self.maintenance_files_for_titles(chunk).await?;
            let series_movies_by_title = self.maintenance_series_movies_for_titles(chunk).await?;
            let requesters_by_title = self.maintenance_requesters_for_titles(chunk).await?;
            let signals_by_title = self
                .maintenance_watch_signals_for_titles(&watch_context, chunk)
                .await?;
            for title in chunk {
                let files = files_by_title
                    .get(&title.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let people = MaintenanceTitlePeople {
                    requester_user_ids: requesters_by_title.get(&title.id).map(Vec::as_slice),
                    usernames: &usernames,
                };
                let watch = MaintenanceTitleWatch {
                    context: &watch_context,
                    signals: signals_by_title.get(&title.id).map(Vec::as_slice),
                };
                if let Err(error) = self
                    .reconcile_maintenance_title(
                        detail,
                        &mut evaluator,
                        title,
                        files,
                        people,
                        watch,
                        series_movies_by_title
                            .get(&title.id)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                        libraries,
                        &excluded,
                        evaluation_time,
                        &mut counts,
                    )
                    .await
                {
                    // One subject's write or evaluation failing is bounded to
                    // that subject: the rule keeps going and the count carries
                    // the failure into the run row.
                    counts.errors += 1;
                    warn!(
                        rule_set_id = rule_set.id.as_str(),
                        title_id = title.id.as_str(),
                        error = %error,
                        "maintenance evaluation failed for one subject"
                    );
                }
            }
        }

        self.cancel_out_of_scope_maintenance_candidates(
            &rule_set.id,
            &in_scope,
            evaluation_time,
            &mut counts,
        )
        .await?;

        Ok(counts)
    }

    /// Close the rule's live candidates whose subject is no longer in its
    /// selection.
    ///
    /// Reconciliation above only ever visits titles the rule is *currently*
    /// scoped to, so a candidate whose title fell out of that selection — the
    /// rule was re-scoped away from its library, or the title left the catalog
    /// — is never looked at again: it stays live forever, keeps inflating the
    /// count destructive arming makes an operator acknowledge, and (before the
    /// executor's own scope recheck) was still executable. This is the sweep
    /// that closes it, with a reason that says why.
    ///
    /// The read is bounded by [`EVALUATOR_WRITABLE_STATES`], so a leased
    /// (`executing`) candidate is not even listed, and each cancel is the same
    /// compare-and-set every other evaluator write uses: a row another writer
    /// moved in the meantime is left alone and re-examined next pass. Out-of-
    /// scope cancels are counted with the rule's other cancels — that is what
    /// they are.
    async fn cancel_out_of_scope_maintenance_candidates(
        &self,
        rule_set_id: &str,
        in_scope: &HashSet<String>,
        evaluation_time: DateTime<Utc>,
        counts: &mut RuleCounts,
    ) -> AppResult<()> {
        let candidates = &self.services.customization.maintenance_evaluation;
        let live = candidates
            .list_candidates(&MaintenanceCandidateQuery {
                rule_set_id: Some(rule_set_id.to_string()),
                states: EVALUATOR_WRITABLE_STATES.to_vec(),
                ..Default::default()
            })
            .await?;

        for candidate in live {
            if in_scope.contains(&candidate.title_id) {
                continue;
            }
            if candidates
                .transition_candidate_state(
                    &candidate.id,
                    MaintenanceCandidateState::Canceled,
                    candidate_reason::OUT_OF_SCOPE,
                    EVALUATOR_WRITABLE_STATES,
                    evaluation_time,
                )
                .await?
            {
                counts.canceled += 1;
            }
        }
        Ok(())
    }

    /// Reconcile one subject. Every branch is RFC 7.5 stated literally.
    #[allow(clippy::too_many_arguments)]
    async fn reconcile_maintenance_title(
        &self,
        detail: &MaintenanceRuleSetDetail,
        evaluator: &mut MaintenanceRulesEvaluator,
        title: &Title,
        files: &[crate::types::TitleMediaFile],
        people: MaintenanceTitlePeople<'_>,
        watch: MaintenanceTitleWatch<'_>,
        series_movies: &[MaintenanceSeriesMovieDoc],
        libraries: &HashMap<String, MaintenanceLibraryRef>,
        excluded: &HashSet<String>,
        evaluation_time: DateTime<Utc>,
        counts: &mut RuleCounts,
    ) -> AppResult<()> {
        let rule_set_id = detail.rule_set.id.as_str();
        let candidates = &self.services.customization.maintenance_evaluation;

        let mut active = candidates
            .get_active_candidate(rule_set_id, &title.id)
            .await?;

        // A candidate under an execution lease belongs to the action handler for
        // the length of that lease, so this pass leaves it entirely alone — no
        // advance, no cancel, no hold, not even an evaluated tally. The executor
        // re-evaluates the subject on fresh facts immediately before it acts, so
        // nothing is lost by not deciding here; deciding anyway is how a cancel
        // lands on top of a succeeded row, or a succeeded row on top of a
        // cancel.
        if let Some(candidate) = active.as_ref()
            && candidate.state == MaintenanceCandidateState::Executing
        {
            debug!(
                rule_set_id,
                title_id = title.id.as_str(),
                candidate_id = candidate.id.as_str(),
                "skipping a candidate the action handler holds a lease on"
            );
            return Ok(());
        }

        // Exclusions are honoured from the first shadow evaluation (RFC 11):
        // an excluded subject is never evaluated, and an existing candidate for
        // it becomes terminal rather than lingering as live membership.
        if excluded.contains(&title.id) {
            if let Some(candidate) = active
                && candidates
                    .transition_candidate_state(
                        &candidate.id,
                        MaintenanceCandidateState::Excluded,
                        candidate_reason::EXCLUDED,
                        EVALUATOR_WRITABLE_STATES,
                        evaluation_time,
                    )
                    .await?
            {
                counts.excluded += 1;
            }
            return Ok(());
        }

        // A candidate recorded against an older revision no longer describes
        // what the rule means, so it is closed before this pass decides
        // anything (RFC 7.1). A fresh match then opens a new candidate with a
        // new generation and a new clock.
        if let Some(candidate) = active.as_ref()
            && candidate.revision_number != detail.rule_set.current_revision_number
        {
            if candidates
                .transition_candidate_state(
                    &candidate.id,
                    MaintenanceCandidateState::Canceled,
                    candidate_reason::REVISION_SUPERSEDED,
                    EVALUATOR_WRITABLE_STATES,
                    evaluation_time,
                )
                .await?
            {
                counts.superseded += 1;
            }
            // The row is no longer this pass's to continue either way: it either
            // closed here, or another writer moved it and owns what happens next.
            active = None;
        }

        let library = libraries
            .get(&title.library_id)
            .cloned()
            .unwrap_or_else(|| MaintenanceLibraryRef {
                id: title.library_id.clone(),
                name: String::new(),
            });
        let input = build_title_input(
            evaluation_time,
            title,
            &library,
            files,
            people,
            watch,
            series_movies,
        );
        let evaluation = evaluator.evaluate(&input);
        counts.evaluated += 1;

        let decision = match &evaluation {
            Ok(result) => result.records.first().map(|record| &record.decision),
            Err(_) => None,
        };

        let Some(decision) = decision else {
            // No record means the rule errored for this subject. An error is
            // never rendered as a no-match, so it holds exactly like unknown.
            counts.errors += 1;
            if let Some(candidate) = active {
                candidates
                    .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                    .await?;
                counts.held += 1;
            }
            return Ok(());
        };

        match decision.outcome {
            MaintenanceOutcome::Match => {
                counts.matched += 1;
                match active {
                    Some(candidate) => {
                        // Continuing membership: the clock keeps running from
                        // the original first match, which is why
                        // `first_matched_at` is not a parameter here.
                        candidates
                            .record_candidate_match(
                                &candidate.id,
                                evaluation_time,
                                &decision.reason_codes,
                                evaluation_time,
                            )
                            .await?;
                    }
                    None => {
                        let generation = candidates
                            .max_match_generation(rule_set_id, &title.id)
                            .await?
                            + 1;
                        let grace_days = detail.revision.grace_days.max(0);
                        candidates
                            .create_candidate(&LifecycleCandidate {
                                id: Id::new().0,
                                rule_set_id: rule_set_id.to_string(),
                                revision_number: detail.rule_set.current_revision_number,
                                matcher_content_hash: detail.revision.matcher_content_hash.clone(),
                                title_id: title.id.clone(),
                                library_id: title.library_id.clone(),
                                facet: title.facet.as_str().to_string(),
                                subject_kind: detail
                                    .rule_set
                                    .subject_kind
                                    .as_storage_str()
                                    .to_string(),
                                match_generation: generation,
                                state: MaintenanceCandidateState::Observing,
                                state_reason: candidate_reason::FIRST_MATCH.to_string(),
                                reason_codes: decision.reason_codes.clone(),
                                action_kind: detail.action_spec.kind.as_wire_str().to_string(),
                                grace_days,
                                first_matched_at: evaluation_time,
                                last_matched_at: evaluation_time,
                                // Materialized, so a zero-day grace is due the
                                // instant it matches.
                                due_at: evaluation_time + Duration::days(grace_days),
                                last_evaluated_at: evaluation_time,
                                held_since: None,
                                action_attempts: 0,
                                created_at: evaluation_time,
                                updated_at: evaluation_time,
                            })
                            .await?;
                        counts.created += 1;
                    }
                }
            }
            MaintenanceOutcome::NoMatch => {
                counts.no_match += 1;
                if let Some(candidate) = active
                    && candidates
                        .transition_candidate_state(
                            &candidate.id,
                            MaintenanceCandidateState::Canceled,
                            candidate_reason::NO_MATCH,
                            EVALUATOR_WRITABLE_STATES,
                            evaluation_time,
                        )
                        .await?
                {
                    counts.canceled += 1;
                }
            }
            MaintenanceOutcome::Unknown => {
                // Unknown holds: never advance, never cancel (RFC 7.5).
                counts.unknown += 1;
                if let Some(candidate) = active {
                    candidates
                        .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                        .await?;
                    counts.held += 1;
                }
            }
        }

        Ok(())
    }

    /// Global exclusions plus this rule's own, as a subject lookup set.
    async fn maintenance_excluded_title_ids(
        &self,
        rule_set_id: &str,
    ) -> AppResult<HashSet<String>> {
        Ok(self
            .services
            .customization
            .maintenance_evaluation
            .list_exclusions(Some(rule_set_id))
            .await?
            .into_iter()
            .map(|exclusion| exclusion.title_id)
            .collect())
    }

    /// Every title the rule is scoped to. An empty library scope means the
    /// whole catalog.
    async fn maintenance_scoped_titles(
        &self,
        rule_set: &MaintenanceRuleSet,
    ) -> AppResult<Vec<Title>> {
        if rule_set.library_ids.is_empty() {
            self.services.catalog.titles.list(None, None).await
        } else {
            self.services
                .catalog
                .titles
                .list_for_libraries(None, &rule_set.library_ids, None)
                .await
        }
    }

    /// One batched media-file load per chunk, grouped by title, exactly as
    /// preview does. A per-title query here would make the job's cost scale
    /// with the library.
    pub(crate) async fn maintenance_files_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<crate::types::TitleMediaFile>>> {
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        let mut files_by_title: HashMap<String, Vec<crate::types::TitleMediaFile>> = HashMap::new();
        if title_ids.is_empty() {
            return Ok(files_by_title);
        }
        for file in self
            .services
            .library
            .media_files
            .list_media_files_for_titles(&title_ids)
            .await?
        {
            files_by_title
                .entry(file.title_id.clone())
                .or_default()
                .push(file);
        }
        Ok(files_by_title)
    }

    /// One batched series-movie load per chunk, grouped by series title.
    ///
    /// Two reads, mirroring how `files` is built: every link for the chunk in
    /// one query, then the "which links have a file" answer per show. The
    /// second is per title because that is the shape the port has, and it is
    /// only asked for shows that actually carry links — a movie-only chunk
    /// issues neither query.
    pub(crate) async fn maintenance_series_movies_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<MaintenanceSeriesMovieDoc>>> {
        let series_title_ids: Vec<String> = titles
            .iter()
            .filter(|title| !matches!(title.facet, scryer_domain::MediaFacet::Movie))
            .map(|title| title.id.clone())
            .collect();
        if series_title_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let links = self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_titles(&series_title_ids)
            .await?;
        if links.is_empty() {
            return Ok(HashMap::new());
        }

        let mut linked_titles: Vec<String> = links
            .iter()
            .map(|link| link.series_title_id.clone())
            .collect();
        linked_titles.sort();
        linked_titles.dedup();
        let mut with_files: HashSet<String> = HashSet::new();
        for title_id in &linked_titles {
            with_files.extend(
                self.services
                    .library
                    .media_files
                    .list_series_movie_link_ids_with_files_for_title(title_id)
                    .await?,
            );
        }

        let mut by_title: HashMap<String, Vec<MaintenanceSeriesMovieDoc>> = HashMap::new();
        for link in links {
            by_title
                .entry(link.series_title_id.clone())
                .or_default()
                .push(MaintenanceSeriesMovieDoc {
                    has_file: with_files.contains(&link.id),
                    link_id: link.id,
                    name: link.movie.title,
                    year: link.movie.year,
                    monitored: link.monitored,
                    tags: link
                        .tags
                        .into_iter()
                        .filter(|tag| !crate::is_reserved_title_tag(tag))
                        .collect(),
                });
        }
        Ok(by_title)
    }

    /// One batched media-request load per chunk, keyed by title. A title is a
    /// key only when a request created it, which is what the `requested` fact
    /// reads.
    pub(crate) async fn maintenance_requesters_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<String>>> {
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.services
            .catalog
            .media_requests
            .requester_user_ids_by_title_ids(&title_ids)
            .await
    }

    /// The watch-signal gate and participant roster, read once per rule.
    ///
    /// Both halves are whole-instance answers, so resolving them per title
    /// would be both wasteful and wrong: two subjects in one run must never
    /// disagree about whether the watch picture is fresh, exactly as they never
    /// disagree about the clock.
    ///
    /// The gate is deliberately unanimous. Every *enabled* connection of a
    /// signal-sync provider has to have swept cleanly inside
    /// [`WATCH_SIGNAL_FRESHNESS_HOURS`]; one that has not poisons watch facts
    /// for the whole run, because Scryer would otherwise be reporting a partial
    /// watch picture as a complete one. A disabled connection is skipped: an
    /// operator turning a server off is a decision, not a gap.
    ///
    /// The roster is part of the gate, not just its payload: a clean sweep with
    /// nobody linked observes nobody, so it reports unavailable rather than an
    /// empty — and therefore decisive — watch picture.
    pub(crate) async fn maintenance_watch_context(&self) -> AppResult<MaintenanceWatchContext> {
        let mut connections = Vec::new();
        for provider in SIGNAL_SYNC_PROVIDERS {
            connections.extend(
                self.services
                    .integrations
                    .media_server_connections
                    .list(Some(provider))
                    .await?,
            );
        }
        connections.retain(|connection| connection.enabled);
        if connections.is_empty() {
            return Ok(MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::NO_MEDIA_SERVER_CONNECTION,
                ),
                linked_user_ids: HashSet::new(),
            });
        }

        let states: HashMap<String, DateTime<Utc>> = self
            .services
            .integrations
            .media_server_signals
            .signal_sync_states()
            .await?
            .into_iter()
            .filter_map(|state| Some((state.connection_id, state.last_success_at?)))
            .collect();

        // Never-swept is reported ahead of stale: it is the more specific
        // answer, and the one an operator can actually act on.
        if connections
            .iter()
            .any(|connection| !states.contains_key(&connection.id))
        {
            return Ok(MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::SIGNAL_SYNC_NEVER_SUCCEEDED,
                ),
                linked_user_ids: HashSet::new(),
            });
        }
        let oldest_allowed = Utc::now() - Duration::hours(WATCH_SIGNAL_FRESHNESS_HOURS);
        if connections
            .iter()
            .filter_map(|connection| states.get(&connection.id))
            .any(|last_success_at| *last_success_at < oldest_allowed)
        {
            return Ok(MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(unknown_reason::SIGNALS_STALE),
                linked_user_ids: HashSet::new(),
            });
        }

        // Only resolved once the gate has passed: with watch facts already
        // unknown, the roster would answer a question nothing is going to ask.
        let mut linked_user_ids = HashSet::new();
        for connection in &connections {
            let Some(provider) = connection.provider.external_account_provider() else {
                continue;
            };
            for account in self
                .services
                .identity
                .external_accounts
                .list_verified_by_connection(provider, &connection.id)
                .await?
            {
                linked_user_ids.insert(account.user_id);
            }
        }

        // A fresh sweep over nobody is not a watch picture. With no verified
        // link on any enabled connection, every subject would come back "no
        // plays recorded" — which reads as "nobody watched it" while Scryer is
        // in fact observing nobody at all.
        if linked_user_ids.is_empty() {
            return Ok(MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::NO_LINKED_PARTICIPANTS,
                ),
                linked_user_ids,
            });
        }

        Ok(MaintenanceWatchContext {
            freshness: WatchSignalFreshness::Fresh,
            linked_user_ids,
        })
    }

    /// One batched movie-signal load per chunk, keyed by owning title.
    ///
    /// Only movie-facet titles are asked about, for the same reason only they
    /// get a watch answer: episode signals roll up to nothing this wave can
    /// report, so fetching them would be work whose result is discarded. A
    /// closed gate skips the query entirely — every watch fact is already
    /// unknown, so the rows could not change a decision.
    pub(crate) async fn maintenance_watch_signals_for_titles(
        &self,
        context: &MaintenanceWatchContext,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<scryer_domain::UserMediaSignal>>> {
        if context.freshness != WatchSignalFreshness::Fresh {
            return Ok(HashMap::new());
        }
        let title_ids: Vec<String> = titles
            .iter()
            .filter(|title| matches!(title.facet, scryer_domain::MediaFacet::Movie))
            .map(|title| title.id.clone())
            .collect();
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.services
            .integrations
            .media_server_signals
            .movie_signals_for_titles(&title_ids)
            .await
    }

    /// Username by user id, read once per evaluation run.
    ///
    /// A user that no longer exists is simply missing from the map, which the
    /// facts builder turns into an unknown name rather than a wrong one.
    pub(crate) async fn maintenance_usernames_by_id(&self) -> AppResult<HashMap<String, String>> {
        Ok(self
            .services
            .identity
            .users
            .list_all()
            .await?
            .into_iter()
            .map(|user| (user.id, user.username))
            .collect())
    }
}
