//! Scheduled evaluation of stored lifecycle rules.
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
//! [`LifecycleCandidateState::Observing`], [`LifecycleCandidateState::Canceled`],
//! and [`LifecycleCandidateState::Excluded`]. Execution writes the rest.
//!
//! # Fail-closed shape
//!
//! Every failure path holds rather than advances. An `unknown` decision, a
//! per-title evaluation error, and a matcher that no longer compiles all leave
//! the existing candidate exactly where it was; only a confirmed `no_match`
//! cancels one: a transient evaluation failure preserves membership and holds
//! the action.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    AppPermission, Id, LifecycleCandidate, LifecycleCandidateState, LifecycleEvaluationMode,
    LifecycleEvaluationRun, LifecycleEvaluationRunStatus, LifecycleRuleExclusion, LifecycleRuleSet,
    Title, User,
};
use scryer_rules::lifecycle::{
    LifecycleOutcome, LifecyclePolicy, LifecycleRulesEngine, LifecycleRulesEvaluator,
    LifecycleSeriesMovieDoc,
};
use serde::Serialize;
use tracing::{debug, warn};

use crate::jobs::{JobKey, JobTriggerSource};
use crate::lifecycle_rules::action_sequence::{ACTION_SEQUENCE_KIND, LifecycleActionSequence};
use crate::lifecycle_rules::cross_level::{LifecycleDecidedCandidate, LifecycleSubjectDecision};
use crate::lifecycle_rules::facts::{
    LifecycleLibraryRef, LifecycleTitleClaims, LifecycleTitlePeople, LifecycleTitleWatch,
    LifecycleWatchContext, WatchSignalFreshness, build_subject_input,
    build_subject_input_with_completed_retention_episodes, nest_subject_input, unknown_reason,
    watch_signal_sweep_is_fresh,
};
use crate::lifecycle_rules::service::LifecycleRuleSetDetail;
use crate::lifecycle_rules::storage::{
    LifecycleStorageCapacityMemo, LifecycleStorageRead, LifecycleStorageRoots,
};
use crate::lifecycle_rules::subjects::{
    LifecycleListingInventories, LifecyclePassFilePresence, LifecycleTitleEpisodeInventory,
};
use crate::media_server_signals::SIGNAL_SYNC_PROVIDERS;
use crate::ports::LifecycleCandidateQuery;
use crate::settings::keys::{
    LIFECYCLE_GATE_DESTRUCTIVE_EFFECTS_KEY, LIFECYCLE_GATE_EVALUATION_KEY,
    LIFECYCLE_GATE_RESULT_DISPLAY_KEY, LIFECYCLE_GATE_REVERSIBLE_EFFECTS_KEY,
};
use crate::{AppError, AppResult, AppUseCase};

/// How many titles one batched media-file load covers. The evaluator is a
/// background job, not a latency path, so the chunk exists to bound peak memory
/// and query size rather than to go fast.
pub const LIFECYCLE_EVALUATION_TITLE_CHUNK: usize = 200;

/// Per-pass read-through state. Inventories are capped to a title page so a
/// long evaluation cannot retain its entire catalog, while repeated rules can
/// reuse a consistent first-page snapshot. The configured roots are read once
/// per pass and capacity figures are kept by root for this pass only.
/// File presence is kept for the whole pass, by path only, so a second rule
/// past the first page does not probe the share again for the same file.
#[derive(Default)]
struct LifecycleEvaluationBatchCache {
    inventories: HashMap<String, Arc<LifecycleTitleEpisodeInventory>>,
    file_presence: LifecyclePassFilePresence,
    storage: LifecyclePassStorage,
}

/// What a pass reads storage facts with: the configured roots, read once,
/// and each root's capacity, read once.
#[derive(Default)]
pub(crate) struct LifecyclePassStorage {
    roots: Option<LifecycleStorageRoots>,
    capacities: LifecycleStorageCapacityMemo,
}

impl AppUseCase {
    /// Set `facts.storage_roots` for `files`, reading the configured roots
    /// on the pass's first use. A configuration that cannot be read leaves
    /// storage unknown for the rest of the pass.
    pub(crate) async fn lifecycle_pass_storage_facts(
        &self,
        storage: &mut LifecyclePassStorage,
        facts: &mut scryer_rules::lifecycle::LifecycleFactsDoc,
        files: &[crate::types::TitleMediaFile],
        read: LifecycleStorageRead,
    ) {
        if storage.roots.is_none() {
            storage.roots = Some(match self.lifecycle_storage_roots_context().await {
                Ok(roots) => roots,
                Err(error) => {
                    warn!(error = %error, "could not read the configured roots; storage facts are unknown");
                    LifecycleStorageRoots::unavailable()
                }
            });
        }
        let roots = storage.roots.as_ref().expect("roots are read above");
        self.populate_lifecycle_storage_roots(facts, files, roots, &mut storage.capacities, read)
            .await;
    }
}

/// The level a stored candidate's subject kind names.
pub(crate) fn subject_level(
    subject_kind: &str,
) -> Option<scryer_rules::lifecycle::LifecycleSubjectKind> {
    use scryer_rules::lifecycle::LifecycleSubjectKind as Level;
    match subject_kind {
        "title" => Some(Level::Title),
        "season" => Some(Level::Season),
        "episode" => Some(Level::Episode),
        _ => None,
    }
}

/// What this evaluation decided for one subject, kept for pairing it with
/// the title's other subjects. `None` unless `track` is set, the actions are
/// known and the subject kind names a level.
fn tracked_decision(
    track: bool,
    subject: &super::subjects::LifecycleSubject,
    actions: Option<&scryer_rules::lifecycle_actions::LifecycleActions>,
    candidate: Option<&LifecycleCandidate>,
) -> Option<LifecycleSubjectDecision> {
    if !track {
        return None;
    }
    let actions = actions?;
    let level = subject_level(subject.kind.as_storage_str())?;
    Some(LifecycleSubjectDecision {
        level,
        subject_id: subject.id.clone(),
        season_id: match level {
            scryer_rules::lifecycle::LifecycleSubjectKind::Title => None,
            _ => subject
                .collection
                .as_ref()
                .map(|collection| collection.id.clone()),
        },
        heads: super::cross_level::heads_at_level(actions, level),
        candidate: candidate.map(|candidate| LifecycleDecidedCandidate {
            id: candidate.id.clone(),
            actions_conflict: candidate.actions_conflict.clone(),
        }),
    })
}

/// The storage kind of a level.
pub(crate) fn level_scope(
    level: scryer_rules::lifecycle::LifecycleSubjectKind,
) -> scryer_domain::LifecycleRuleSubjectKind {
    use scryer_domain::LifecycleRuleSubjectKind as Scope;
    use scryer_rules::lifecycle::LifecycleSubjectKind as Level;
    match level {
        Level::Title => Scope::Title,
        Level::Season => Scope::Season,
        Level::Episode => Scope::Episode,
    }
}

/// The candidate states the evaluator is allowed to move a row out of.
///
/// [`LifecycleCandidateState::Executing`] is deliberately absent: that state
/// means the action handler holds a lease, and the two jobs are independently
/// schedulable. Every evaluator write is a compare-and-set against this set, so
/// an evaluation pass that runs while an action is executing can neither cancel
/// the row out from under the executor nor overwrite the terminal state the
/// executor just wrote.
const EVALUATOR_WRITABLE_STATES: &[LifecycleCandidateState] = &[
    LifecycleCandidateState::Observing,
    LifecycleCandidateState::PendingAction,
    LifecycleCandidateState::Due,
    LifecycleCandidateState::Blocked,
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
    /// The subject still matches but the rule now decides different actions
    /// for it. The old candidate closes with this reason, and the candidate
    /// that replaces it carries the reason too when it keeps the old grace
    /// clock.
    pub const ACTIONS_CHANGED: &str = "actions_changed";
}

// ── Instance gates ──────────────────────────────────────────────────────────

/// The stored instance-wide gates.
///
/// All default off, and a missing settings row reads as off, so an instance
/// that has never been configured evaluates nothing and acts on nothing.
/// Operators drive them in pairs through [`LifecycleGatesUpdate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LifecycleGates {
    pub evaluation_enabled: bool,
    pub result_display_enabled: bool,
    pub reversible_effects_enabled: bool,
    pub destructive_effects_enabled: bool,
}

impl LifecycleGates {
    /// Lifecycle is enabled when rules are evaluated and their candidates
    /// are shown.
    pub fn enabled(&self) -> bool {
        self.evaluation_enabled && self.result_display_enabled
    }

    /// Actions are paused only when no effect of either class may run.
    pub fn actions_paused(&self) -> bool {
        !self.reversible_effects_enabled && !self.destructive_effects_enabled
    }
}

/// The two operator switches. `None` leaves that switch's gates as stored.
#[derive(Clone, Copy, Debug, Default)]
pub struct LifecycleGatesUpdate {
    /// Writes both the evaluation and the result display gate.
    pub enabled: Option<bool>,
    /// Writes both effect gates: paused closes them, unpaused opens them.
    pub actions_paused: Option<bool>,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A candidate with the two names a reader needs, both batch-resolved.
///
/// `title_name` falls back to the stored `title_id` when the title is gone: a
/// deleted subject must not fail the whole listing, and the id is still the
/// honest answer to "what was this".
#[derive(Clone, Debug)]
pub struct LifecycleCandidateView {
    pub subject_label: String,
    pub file_count: Option<i64>,
    pub total_size_bytes: Option<i64>,
    pub candidate: LifecycleCandidate,
    pub rule_name: String,
    pub title_name: String,
    /// The steps this candidate runs, built from the actions its rule decided
    /// for it. `None` when no decision is recorded or a decided target no
    /// longer resolves; such a candidate cannot run.
    pub action_sequence: Option<LifecycleActionSequence>,
    /// Ordered v2 step evidence for this candidate generation.
    pub sequence_steps:
        Vec<crate::lifecycle_rules::action_execution::LifecycleActionStepProgressView>,
}

#[derive(Clone, Debug)]
pub struct LifecycleExclusionView {
    pub subject_label: String,
    pub exclusion: LifecycleRuleExclusion,
    pub title_name: String,
}

/// Which candidates a reader wants.
#[derive(Clone, Debug, Default)]
pub struct LifecycleCandidateFilter {
    pub rule_set_id: Option<String>,
    pub states: Vec<LifecycleCandidateState>,
    pub library_id: Option<String>,

    pub limit: Option<usize>,
}

/// Answer to a manual run request.
#[derive(Clone, Debug)]
pub struct LifecycleEvaluationTrigger {
    pub started: bool,
    pub message: Option<String>,
}

/// What one whole evaluation pass did, across every rule it considered.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEvaluationReport {
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
    /// Lease bookkeeping (spec 0003 FR-041). Both counts are produced before
    /// the evaluation gate is even read, because a lease's clock is not an
    /// opinion the operator armed: it runs whether or not any rule is allowed
    /// to look at it.
    pub leases_expired: u64,
    pub leases_activated: u64,
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
    pub(crate) async fn load_lifecycle_gates(&self) -> AppResult<LifecycleGates> {
        Ok(LifecycleGates {
            evaluation_enabled: self.lifecycle_gate(LIFECYCLE_GATE_EVALUATION_KEY).await?,
            result_display_enabled: self
                .lifecycle_gate(LIFECYCLE_GATE_RESULT_DISPLAY_KEY)
                .await?,
            reversible_effects_enabled: self
                .lifecycle_gate(LIFECYCLE_GATE_REVERSIBLE_EFFECTS_KEY)
                .await?,
            destructive_effects_enabled: self
                .lifecycle_gate(LIFECYCLE_GATE_DESTRUCTIVE_EFFECTS_KEY)
                .await?,
        })
    }

    /// A missing or unparseable row is off. Losing the settings table disarms
    /// lifecycle; it never arms it.
    async fn lifecycle_gate(&self, key: &str) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(key, None)
            .await?
            .unwrap_or(false))
    }

    /// Read the gates. Instance-wide arming is a system setting, so it is
    /// gated like one, not like the catalog-settings authoring surface.
    pub async fn lifecycle_instance_gates(&self, actor: &User) -> AppResult<LifecycleGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.load_lifecycle_gates().await
    }

    /// Flip either switch. An omitted switch leaves its gates exactly as stored.
    pub async fn set_lifecycle_instance_gates(
        &self,
        actor: &User,
        update: LifecycleGatesUpdate,
    ) -> AppResult<LifecycleGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;

        let enabled = update.enabled.map(|enabled| {
            [
                LIFECYCLE_GATE_EVALUATION_KEY,
                LIFECYCLE_GATE_RESULT_DISPLAY_KEY,
            ]
            .map(|key| (key, enabled))
        });
        let effects = update.actions_paused.map(|paused| {
            [
                LIFECYCLE_GATE_REVERSIBLE_EFFECTS_KEY,
                LIFECYCLE_GATE_DESTRUCTIVE_EFFECTS_KEY,
            ]
            .map(|key| (key, !paused))
        });
        let values: Vec<_> = enabled
            .into_iter()
            .chain(effects)
            .flatten()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        if !values.is_empty() {
            self.services
                .config
                .settings
                .upsert_global_settings_json(
                    crate::SETTINGS_SCOPE_SYSTEM,
                    &values,
                    crate::settings::keys::SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
        }

        self.load_lifecycle_gates().await
    }
}

// ── Exclusions ──────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Exclusions, optionally narrowed to those that apply to one rule: that
    /// rule's own rows plus every global row, because both are what actually
    /// stop it acting.
    pub async fn list_lifecycle_exclusions(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<LifecycleExclusionView>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let exclusions = self
            .services
            .customization
            .lifecycle_evaluation
            .list_exclusions(rule_set_id)
            .await?;
        let names = self
            .lifecycle_title_names(exclusions.iter().map(|row| row.title_id.clone()))
            .await?;

        let summaries = self
            .lifecycle_subject_summaries(exclusions.iter().map(|row| {
                (
                    row.subject_kind.as_storage_str().to_string(),
                    row.title_id.clone(),
                )
            }))
            .await?;
        Ok(exclusions
            .into_iter()
            .map(|exclusion| LifecycleExclusionView {
                subject_label: summaries
                    .get(&(
                        exclusion.subject_kind.as_storage_str().to_string(),
                        exclusion.subject_id.clone(),
                    ))
                    .map(|summary| summary.0.clone())
                    .unwrap_or_else(|| exclusion.subject_id.clone()),
                title_name: lifecycle_title_name(&names, &exclusion.title_id),
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
    pub async fn exclude_lifecycle_subject(
        &self,
        actor: &User,
        title_id: &str,
        rule_set_id: Option<String>,
        reason: Option<String>,
    ) -> AppResult<LifecycleExclusionView> {
        self.exclude_lifecycle_scoped_subject(
            actor,
            title_id,
            scryer_domain::LifecycleRuleSubjectKind::Title,
            title_id,
            rule_set_id,
            reason,
        )
        .await
    }

    pub async fn exclude_lifecycle_scoped_subject(
        &self,
        actor: &User,
        title_id: &str,
        subject_kind: scryer_domain::LifecycleRuleSubjectKind,
        subject_id: &str,
        rule_set_id: Option<String>,
        reason: Option<String>,
    ) -> AppResult<LifecycleExclusionView> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id} not found"),
                )
            })?;
        if let Some(rule_set_id) = rule_set_id.as_deref() {
            self.require_lifecycle_rule_set(rule_set_id).await?;
        }

        let subject = self
            .lifecycle_resolve_subject(subject_kind.as_storage_str(), subject_id, &title, &[])
            .await?;
        let exclusion = LifecycleRuleExclusion {
            id: Id::new().0,
            rule_set_id,
            title_id: title.id.clone(),
            subject_kind,
            subject_id: subject_id.to_string(),
            reason: reason.unwrap_or_default(),
            created_by: Some(actor.id.clone()),
            created_at: Utc::now(),
        };
        self.services
            .customization
            .lifecycle_evaluation
            .create_exclusion(&exclusion)
            .await?;

        Ok(LifecycleExclusionView {
            subject_label: subject.label,
            exclusion,
            title_name: title.name,
        })
    }

    pub async fn remove_lifecycle_exclusion(&self, actor: &User, id: &str) -> AppResult<String> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .customization
            .lifecycle_evaluation
            .get_exclusion(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::lifecycle::EXCLUSION_NOT_FOUND,
                    format!("lifecycle exclusion {id} not found"),
                )
            })?;
        self.services
            .customization
            .lifecycle_evaluation
            .delete_exclusion(id)
            .await?;
        Ok(id.to_string())
    }
}

// ── Candidate and run reads ─────────────────────────────────────────────────

impl AppUseCase {
    /// Candidates, with rule and title names resolved in two batched lookups.
    /// The instance `result_display` gate hides every row.
    pub async fn list_lifecycle_candidates(
        &self,
        actor: &User,
        filter: LifecycleCandidateFilter,
    ) -> AppResult<Vec<LifecycleCandidateView>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let gates = self.load_lifecycle_gates().await?;
        if !gates.result_display_enabled {
            return Ok(Vec::new());
        }

        let rule_sets = self
            .services
            .customization
            .lifecycle_rule_sets
            .list_rule_sets()
            .await?;
        let rules_by_id: HashMap<String, LifecycleRuleSet> = rule_sets
            .into_iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set))
            .collect();

        let candidates = self
            .services
            .customization
            .lifecycle_evaluation
            .list_candidates(&LifecycleCandidateQuery {
                rule_set_id: filter.rule_set_id.clone(),
                states: filter.states.clone(),
                library_id: filter.library_id.clone(),
                limit: filter.limit,
            })
            .await?;

        let visible: Vec<LifecycleCandidate> = candidates
            .into_iter()
            // The FK cascades, so a missing rule is only reachable for one
            // deleted between the two reads. Hiding the row is the
            // conservative answer.
            .filter(|candidate| rules_by_id.contains_key(&candidate.rule_set_id))
            .collect();

        let names = self
            .lifecycle_title_names(visible.iter().map(|candidate| candidate.title_id.clone()))
            .await?;

        let mut inventories = LifecycleListingInventories::default();
        let summaries = self
            .lifecycle_listing_subject_summaries(
                visible
                    .iter()
                    .map(|row| (row.subject_kind.clone(), row.title_id.clone())),
                &mut inventories,
            )
            .await?;
        let (sequences, sequence_steps) = self.lifecycle_sequence_projection(&visible).await?;
        Ok(visible
            .into_iter()
            .map(|candidate| LifecycleCandidateView {
                subject_label: summaries
                    .get(&(candidate.subject_kind.clone(), candidate.subject_id.clone()))
                    .map(|summary| summary.0.clone())
                    .unwrap_or_else(|| candidate.subject_id.clone()),
                file_count: summaries
                    .get(&(candidate.subject_kind.clone(), candidate.subject_id.clone()))
                    .map(|summary| summary.1),
                total_size_bytes: summaries
                    .get(&(candidate.subject_kind.clone(), candidate.subject_id.clone()))
                    .map(|summary| summary.2),
                rule_name: rules_by_id
                    .get(&candidate.rule_set_id)
                    .map(|rule_set| rule_set.name.clone())
                    .unwrap_or_default(),
                title_name: lifecycle_title_name(&names, &candidate.title_id),
                action_sequence: sequences.get(&candidate.id).cloned(),
                sequence_steps: sequence_steps
                    .get(&candidate.id)
                    .cloned()
                    .unwrap_or_default(),
                candidate,
            })
            .collect())
    }

    /// Evaluation runs carry counts and timing, never subjects, so they are not
    /// hidden by the result-display gate: they are how an operator sees that
    /// evaluation is working at all.
    pub async fn list_lifecycle_evaluation_runs(
        &self,
        actor: &User,
        rule_set_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<LifecycleEvaluationRun>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .lifecycle_evaluation
            .list_evaluation_runs(rule_set_id, limit)
            .await
    }

    pub(crate) async fn lifecycle_title_names(
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
pub(crate) fn lifecycle_title_name(names: &HashMap<String, String>, title_id: &str) -> String {
    names
        .get(title_id)
        .cloned()
        .unwrap_or_else(|| title_id.to_string())
}

/// Whether a changed decision deletes something the stored one did not.
/// Deleting files and deleting the title are compared one by one: a decision
/// that moves from deleting files to deleting the title is a new delete and
/// waits out its own grace. A candidate without a stored decision counts as
/// deleting nothing.
fn decision_deletes_more(
    stored: Option<&scryer_rules::lifecycle_actions::LifecycleActions>,
    decided: &scryer_rules::lifecycle_actions::LifecycleActions,
) -> bool {
    let (stored_files, stored_title) = stored.map_or((false, false), |stored| {
        (stored.delete_files, stored.delete_title)
    });
    (decided.delete_files && !stored_files) || (decided.delete_title && !stored_title)
}

/// The high-risk attempts a superseded candidate spent: the most any of its
/// high-risk steps spent, or what it carried itself. A failed or running
/// (possibly dispatched) attempt is spent; a held one is not yet.
fn carried_high_risk_attempts(
    candidate: &LifecycleCandidate,
    steps: &[scryer_domain::LifecycleActionStepRun],
) -> i64 {
    let high_risk = super::action_execution::high_risk_step_kinds();
    steps
        .iter()
        .filter(|step| high_risk.contains(&step.step_kind.as_str()))
        .map(|step| match step.state {
            scryer_domain::LifecycleActionStepState::Failed
            | scryer_domain::LifecycleActionStepState::Running => step.attempt,
            _ => step.attempt - 1,
        })
        .fold(candidate.action_attempts.max(0), i64::max)
}

#[cfg(test)]
mod decision_change_tests {
    use super::decision_deletes_more;
    use scryer_rules::lifecycle_actions::LifecycleActions;

    fn actions(delete_files: bool, delete_title: bool, search: bool) -> LifecycleActions {
        LifecycleActions {
            delete_files,
            delete_title,
            search,
            ..LifecycleActions::default()
        }
    }

    #[test]
    fn each_delete_head_is_compared_on_its_own() {
        let files = actions(true, false, false);
        let title = actions(false, true, false);
        let both = actions(true, true, false);
        let none = actions(false, false, true);

        assert!(decision_deletes_more(Some(&files), &title));
        assert!(decision_deletes_more(Some(&title), &files));
        assert!(decision_deletes_more(Some(&files), &both));
        assert!(decision_deletes_more(Some(&none), &files));
        assert!(decision_deletes_more(None, &title));

        assert!(!decision_deletes_more(Some(&both), &files));
        assert!(!decision_deletes_more(Some(&both), &title));
        assert!(!decision_deletes_more(
            Some(&files),
            &actions(true, false, true)
        ));
        assert!(!decision_deletes_more(Some(&files), &none));
        assert!(!decision_deletes_more(None, &none));
    }
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
    pub async fn run_lifecycle_evaluation_now(
        &self,
        actor: &User,
        rule_set_id: Option<String>,
    ) -> AppResult<LifecycleEvaluationTrigger> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        if !self.load_lifecycle_gates().await?.evaluation_enabled {
            return Ok(LifecycleEvaluationTrigger {
                started: false,
                message: Some(
                    "the instance lifecycle evaluation gate is off, so nothing was evaluated"
                        .to_string(),
                ),
            });
        }

        if let Some(rule_set_id) = rule_set_id {
            let rule_set = self.require_lifecycle_rule_set(&rule_set_id).await?;
            if rule_set.evaluation_mode == LifecycleEvaluationMode::Disabled {
                return Ok(LifecycleEvaluationTrigger {
                    started: false,
                    message: Some(format!(
                        "lifecycle rule {} is disabled, so it was not evaluated",
                        rule_set.name
                    )),
                });
            }
            // The same claim a scheduled fire takes: a rule never evaluates
            // or acts beside itself.
            let Some(_claim) = self.claim_lifecycle_rule_run(&rule_set_id) else {
                return Ok(LifecycleEvaluationTrigger {
                    started: false,
                    message: Some(format!(
                        "lifecycle rule {} is already running",
                        rule_set.name
                    )),
                });
            };
            let report = self.evaluate_lifecycle_rules(Some(&rule_set_id)).await?;
            return Ok(LifecycleEvaluationTrigger {
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
            .has_active_job(JobKey::LifecycleRuleEvaluation)
            .await
        {
            return Ok(LifecycleEvaluationTrigger {
                started: false,
                message: Some("a lifecycle evaluation run is already in progress".to_string()),
            });
        }

        let app = self.clone();
        // Tracked, so a restore drain waits for the run.
        self.restore_coordinator().spawn_task(async move {
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::LifecycleRuleEvaluation, JobTriggerSource::Manual)
                .await
            {
                warn!(error = %error, "manual lifecycle evaluation run failed");
            }
        });

        Ok(LifecycleEvaluationTrigger {
            started: true,
            message: Some("Lifecycle evaluation started".to_string()),
        })
    }
}

// ── The evaluator ───────────────────────────────────────────────────────────

impl AppUseCase {
    /// Job body for [`JobKey::LifecycleRuleEvaluation`].
    ///
    /// Lease bookkeeping runs first and unconditionally. Expiring a lapsed
    /// lease and starting a dormant one are not evaluation — they are the clock
    /// the requester was promised — so they must not be hostage to the
    /// instance's evaluation gate, which an operator turns off to stop *rules*
    /// from deciding things.
    pub(crate) async fn run_lifecycle_rule_evaluation_job(
        &self,
    ) -> AppResult<LifecycleEvaluationReport> {
        let (leases_expired, leases_activated) = self.reconcile_lifecycle_claims(Utc::now()).await;
        let mut report = self.evaluate_lifecycle_rules(None).await?;
        report.leases_expired = leases_expired;
        report.leases_activated = leases_activated;
        Ok(report)
    }

    /// Claim one rule's run for as long as the returned guard lives. `None`
    /// while a scheduled fire, a manual run, or an all-rules pass is working
    /// on that rule.
    pub(crate) fn claim_lifecycle_rule_run(
        &self,
        rule_set_id: &str,
    ) -> Option<crate::scripts::scheduler::LifecycleRuleFireGuard> {
        self.runtime
            .jobs
            .custom_job_scheduler
            .begin_lifecycle_rule_fire(rule_set_id)
    }

    /// The scheduled pass of [`JobKey::LifecycleRuleEvaluation`]. Rules run
    /// on their own schedules, so the instance-wide tick keeps only the lease
    /// clock; an explicit run still evaluates every rule.
    pub(crate) async fn run_lifecycle_lease_reconciliation_job(&self) -> LifecycleEvaluationReport {
        let (leases_expired, leases_activated) = self.reconcile_lifecycle_claims(Utc::now()).await;
        LifecycleEvaluationReport {
            leases_expired,
            leases_activated,
            ..LifecycleEvaluationReport::default()
        }
    }

    /// One fire of one rule, on its schedule or by its manual trigger:
    /// evaluate it, then act on what it decided. The caller holds the rule's
    /// claim. Both halves keep every gate, hold and re-check of the
    /// instance-wide passes; neither touches another rule's candidates.
    pub(crate) async fn run_scheduled_lifecycle_rule(
        &self,
        rule_set_id: &str,
    ) -> AppResult<(
        LifecycleEvaluationReport,
        super::action_execution::LifecycleActionHandlingReport,
    )> {
        let evaluation = self.evaluate_lifecycle_rules(Some(rule_set_id)).await?;
        // A drain that began while this rule was evaluating waits for the
        // fire; it should not also wait for actions nobody started yet.
        if self.restore_coordinator().shutdown_workers.is_cancelled() {
            return Ok((evaluation, Default::default()));
        }
        let actions = self
            .run_lifecycle_action_handling(Some(rule_set_id))
            .await?;
        Ok((evaluation, actions))
    }

    /// One evaluation pass over every enabled rule, or over one named rule.
    ///
    /// The evaluation time is captured once for the whole pass so every subject
    /// in it sees the same clock (RFC 7.2). A single rule's failure is counted
    /// and logged; it never aborts the pass.
    pub(crate) async fn evaluate_lifecycle_rules(
        &self,
        only_rule_set_id: Option<&str>,
    ) -> AppResult<LifecycleEvaluationReport> {
        let mut report = LifecycleEvaluationReport::default();

        // Read at run start, not at registration: flipping a gate has to take
        // effect on the next run without a restart.
        if !self.load_lifecycle_gates().await?.evaluation_enabled {
            return Ok(report);
        }
        report.gate_enabled = true;

        let evaluation_time = Utc::now();
        let rule_sets = self
            .services
            .customization
            .lifecycle_rule_sets
            .list_rule_sets()
            .await?;
        let libraries = self.lifecycle_library_refs().await?;
        let mut batch_cache = LifecycleEvaluationBatchCache::default();

        for rule_set in rule_sets {
            if only_rule_set_id.is_some_and(|id| id != rule_set.id) {
                continue;
            }
            report.rules_considered += 1;

            // A disabled rule is skipped whole: its candidates are left exactly
            // as they are, so flipping a rule off and on again does not destroy
            // the membership and clocks it already established.
            if rule_set.evaluation_mode == LifecycleEvaluationMode::Disabled {
                continue;
            }
            // A rule never evaluates beside itself. Scoped, the caller holds
            // the rule's claim already; across every rule, a rule whose own
            // fire is running is left to that fire.
            let _claim = match only_rule_set_id {
                Some(_) => None,
                None => match self.claim_lifecycle_rule_run(&rule_set.id) {
                    Some(claim) => Some(claim),
                    None => {
                        tracing::info!(
                            rule_set_id = rule_set.id.as_str(),
                            "lifecycle rule is already running; leaving it to that run"
                        );
                        continue;
                    }
                },
            };

            match self
                .evaluate_one_lifecycle_rule(
                    &rule_set,
                    evaluation_time,
                    &libraries,
                    &mut batch_cache,
                )
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
                        "lifecycle rule evaluation failed; its candidates are held"
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
    async fn evaluate_one_lifecycle_rule(
        &self,
        rule_set: &LifecycleRuleSet,
        evaluation_time: DateTime<Utc>,
        libraries: &HashMap<String, LifecycleLibraryRef>,
        batch_cache: &mut LifecycleEvaluationBatchCache,
    ) -> AppResult<RuleCounts> {
        let detail = self
            .load_lifecycle_rule_detail(rule_set.clone())
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "lifecycle rule {} has no usable revision: {error}",
                    rule_set.id
                ))
            })?;
        // A revision that does not declare its actions never matches anything:
        // it is refused before any candidate is written, so its open
        // candidates keep their state and the executor holds them.
        if let Some(reason) = detail.load_refusal {
            return Err(crate::lifecycle_rules::service::stored_actions_refused(
                reason,
            ));
        }

        let mut run = LifecycleEvaluationRun {
            id: Id::new().0,
            rule_set_id: rule_set.id.clone(),
            revision_number: detail.revision.revision_number,
            source_content_hash: detail.revision.source_content_hash.clone(),
            started_at: evaluation_time,
            finished_at: None,
            status: LifecycleEvaluationRunStatus::Running,
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
            .lifecycle_evaluation
            .start_evaluation_run(&run)
            .await?;

        let outcome = self
            .reconcile_lifecycle_rule(&detail, evaluation_time, libraries, batch_cache)
            .await;

        let finished_at = Utc::now();
        run.finished_at = Some(finished_at);
        run.duration_ms = Some((finished_at - evaluation_time).num_milliseconds().max(0));
        match &outcome {
            Ok(counts) => {
                run.status = LifecycleEvaluationRunStatus::Succeeded;
                run.evaluated_count = counts.evaluated;
                run.matched_count = counts.matched;
                run.no_match_count = counts.no_match;
                run.unknown_count = counts.unknown;
                run.error_count = counts.errors;
                run.canceled_candidates = counts.canceled;
                run.superseded_candidates = counts.superseded;
            }
            Err(error) => {
                run.status = LifecycleEvaluationRunStatus::Failed;
                run.error = Some(error.to_string());
            }
        }
        if let Err(error) = self
            .services
            .customization
            .lifecycle_evaluation
            .finish_evaluation_run(&run)
            .await
        {
            warn!(
                rule_set_id = rule_set.id.as_str(),
                error = %error,
                "could not record the lifecycle evaluation run"
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
    async fn reconcile_lifecycle_rule(
        &self,
        detail: &LifecycleRuleSetDetail,
        evaluation_time: DateTime<Utc>,
        libraries: &HashMap<String, LifecycleLibraryRef>,
        batch_cache: &mut LifecycleEvaluationBatchCache,
    ) -> AppResult<RuleCounts> {
        let rule_set = &detail.rule_set;
        let engine = LifecycleRulesEngine::build(&[LifecyclePolicy {
            id: rule_set.id.clone(),
            name: rule_set.name.clone(),
            rego_source: detail.revision.rego_source.clone(),
        }])
        .map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_FAILED_COMPILE,
                format!("lifecycle rule {} failed to compile: {error}", rule_set.id),
            )
        })?;
        let mut evaluator = engine.evaluator();

        let excluded = self
            .services
            .customization
            .lifecycle_evaluation
            .list_exclusions(Some(&rule_set.id))
            .await?;

        let mut in_scope: HashSet<String> = HashSet::new();
        // Users are read once for the whole rule, not per chunk: the roster is
        // small and bounded, and one snapshot keeps every subject in the run
        // resolving names against the same view, exactly like the clock.
        let usernames = self.lifecycle_usernames_by_id().await?;
        // The watch gate is whole-instance and, like the roster, is read once
        // per rule rather than per chunk.
        let watch_context = self.lifecycle_watch_context().await?;
        // The fact groups with a read of their own are read only for a rule
        // that names one of their facts.
        let fact_loads = super::list_facts::LifecycleFactLoads::for_rule(
            &detail.revision.rego_source,
            &rule_set.id,
        );

        let mut counts = RuleCounts::default();
        // The levels the rule acts at, read off the heads it defines. A rule
        // with title heads alone never loads a season or an episode.
        let levels = detail.declared_actions.levels();
        let narrow_levels: Vec<_> = levels
            .iter()
            .copied()
            .filter(|level| *level != scryer_rules::lifecycle::LifecycleSubjectKind::Title)
            .collect();
        let evaluates_titles =
            levels.contains(&scryer_rules::lifecycle::LifecycleSubjectKind::Title);
        // Whether two of the rule's declared heads at different levels can
        // conflict. A rule where none can keeps no decisions and writes no
        // conflict marker.
        let cross_level =
            super::cross_level::cross_level_conflict_possible(&detail.declared_actions);
        // Read once per rule pass for every candidate's started delete.
        let mut started_reads = super::action_execution::StartedDeleteReads::default();
        let mut after_id = None;
        loop {
            // The rule's libraries filter in the page read (none means all).
            let selected = self
                .services
                .catalog
                .titles
                .list_page_after_id_in_libraries(
                    after_id.clone(),
                    &rule_set.library_ids,
                    LIFECYCLE_EVALUATION_TITLE_CHUNK,
                )
                .await?;
            if selected.is_empty() {
                break;
            }
            after_id = selected.last().map(|title| title.id.clone());
            let chunk = selected.as_slice();
            let files_by_title = self.lifecycle_files_for_titles(chunk).await?;
            let series_movies_by_title = self.lifecycle_series_movies_for_titles(chunk).await?;
            let requesters_by_title = self.lifecycle_requesters_for_titles(chunk).await?;
            let signals_by_title = self
                .lifecycle_watch_signals_for_titles(&watch_context, chunk)
                .await?;
            let (chunk_watch_context, audience_by_title) = self
                .lifecycle_audience_plays_for_titles(&watch_context, chunk)
                .await;
            // A claim store that cannot be read poisons the claim facts for the
            // whole chunk rather than failing the rule: the facts then read
            // unknown, and any rule that touches one is held. Failing the pass
            // instead would stop every *other* rule too, including the ones
            // that never mention a lease.
            let claims_by_title = match self.lifecycle_claims_for_titles(chunk).await {
                Ok(claims) => Some(claims),
                Err(error) => {
                    warn!(
                        rule_set_id = rule_set.id.as_str(),
                        error = %error,
                        "could not read lifecycle claims; claim facts are unknown for this chunk"
                    );
                    None
                }
            };
            let lists_by_title = self
                .lifecycle_list_facts_loading(chunk, fact_loads, &watch_context, evaluation_time)
                .await;
            // The chunk's shows the pass has not read yet are read together. A
            // show the batch could not build loads on its own below, as before.
            let uncached: Vec<Title> = chunk
                .iter()
                .filter(|title| !batch_cache.inventories.contains_key(&title.id))
                .cloned()
                .collect();
            let mut chunk_inventories = self
                .lifecycle_load_title_episode_inventories(
                    &uncached,
                    Some(&mut batch_cache.file_presence),
                )
                .await;
            // Every title of the chunk has its subjects built first, so the
            // step rows of the chunk's candidates are read together before
            // any subject is reconciled.
            let mut prepared = Vec::with_capacity(chunk.len());
            for title in chunk {
                let files = files_by_title
                    .get(&title.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let people = LifecycleTitlePeople {
                    requester_user_ids: requesters_by_title.get(&title.id).map(Vec::as_slice),
                    usernames: &usernames,
                };
                let watch = LifecycleTitleWatch {
                    context: &chunk_watch_context,
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
                // A movie has no seasons or episodes: only its title is a
                // subject. A show's subjects come from one episode inventory.
                let mut subjects = Vec::new();
                if title.facet == scryer_domain::MediaFacet::Movie {
                    if evaluates_titles {
                        subjects = self
                            .lifecycle_subjects_for_title(
                                scryer_domain::LifecycleRuleSubjectKind::Title,
                                title,
                                files,
                            )
                            .await?;
                    }
                } else {
                    let inventory = if let Some(inventory) = batch_cache.inventories.get(&title.id)
                    {
                        inventory.clone()
                    } else {
                        let inventory = match chunk_inventories.remove(&title.id) {
                            Some(inventory) => inventory,
                            None => {
                                self.lifecycle_load_title_episode_inventory_for_pass(
                                    title,
                                    &mut batch_cache.file_presence,
                                )
                                .await?
                            }
                        };
                        if batch_cache.inventories.len() < LIFECYCLE_EVALUATION_TITLE_CHUNK {
                            batch_cache
                                .inventories
                                .insert(title.id.clone(), inventory.clone());
                        }
                        inventory
                    };
                    // The title subject is built whenever the rule acts at any
                    // level: a season or episode is evaluated beside it.
                    let needs_title = evaluates_titles || !narrow_levels.is_empty();
                    if needs_title {
                        subjects.extend(self.lifecycle_subjects_from_episode_inventory(
                            scryer_domain::LifecycleRuleSubjectKind::Title,
                            title,
                            files,
                            &inventory,
                        )?);
                    }
                    for level in &narrow_levels {
                        subjects.extend(self.lifecycle_subjects_from_episode_inventory(
                            level_scope(*level),
                            title,
                            files,
                            &inventory,
                        )?);
                    }
                }
                let series_movies = series_movies_by_title
                    .get(&title.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let library = libraries
                    .get(&title.library_id)
                    .cloned()
                    .unwrap_or_else(|| LifecycleLibraryRef {
                        id: title.library_id.clone(),
                        name: String::new(),
                    });
                // The title's own input, beside which its seasons and episodes
                // are evaluated. Built once per title, and only for a rule that
                // acts below the title.
                let mut title_input = None;
                if !narrow_levels.is_empty()
                    && let Some(title_subject) = subjects.iter().find(|subject| {
                        subject.kind == scryer_domain::LifecycleRuleSubjectKind::Title
                    })
                {
                    let mut input = build_subject_input(
                        evaluation_time,
                        title,
                        &library,
                        title_subject,
                        people,
                        watch,
                        claims,
                        series_movies,
                    );
                    super::list_facts::apply_list_facts(&mut input, &lists_by_title, &title.id);
                    super::list_facts::apply_watchlist_and_metadata_facts(
                        &mut input,
                        &lists_by_title,
                        &title.id,
                        people,
                    );
                    self.lifecycle_pass_storage_facts(
                        &mut batch_cache.storage,
                        &mut input.facts,
                        &title_subject.files,
                        LifecycleStorageRead::Sampled(evaluation_time),
                    )
                    .await;
                    title_input = Some(input);
                }
                if !evaluates_titles {
                    subjects.retain(|subject| {
                        subject.kind != scryer_domain::LifecycleRuleSubjectKind::Title
                    });
                }
                prepared.push((
                    title,
                    subjects,
                    title_input,
                    people,
                    watch,
                    claims,
                    series_movies,
                ));
            }
            // Only a rule that deletes media can have a delete under way.
            if detail.declared_actions.destructive {
                let candidates = self
                    .lifecycle_chunk_active_candidates(
                        &rule_set.id,
                        detail.rule_set.current_revision_number,
                        prepared.iter().flat_map(|(_, subjects, ..)| subjects),
                    )
                    .await;
                self.lifecycle_read_ahead_started_delete_steps(candidates, &mut started_reads)
                    .await;
            }
            for (title, subjects, title_input, people, watch, claims, series_movies) in prepared {
                let mut decisions: Vec<LifecycleSubjectDecision> = Vec::new();
                // A subject that failed or lost a write this pass is missing
                // from `decisions`, so its partners cannot be judged free of
                // it: markers may then be set but never cleared.
                let mut incomplete = false;
                for subject in subjects {
                    in_scope.insert(subject.id.clone());
                    match self
                        .reconcile_lifecycle_title(
                            cross_level,
                            detail,
                            &mut evaluator,
                            title,
                            &subject,
                            title_input.as_ref(),
                            people,
                            watch,
                            claims,
                            &lists_by_title,
                            series_movies,
                            libraries,
                            &excluded,
                            evaluation_time,
                            &mut batch_cache.storage,
                            &mut started_reads,
                            &mut counts,
                            &mut incomplete,
                        )
                        .await
                    {
                        Ok(decision) => decisions.extend(decision),
                        Err(error) => {
                            // One subject's write or evaluation failing is
                            // bounded to that subject: the rule keeps going
                            // and the count carries the failure into the run
                            // row.
                            incomplete = true;
                            counts.errors += 1;
                            warn!(
                                rule_set_id = rule_set.id.as_str(),
                                title_id = title.id.as_str(),
                                error = %error,
                                "lifecycle evaluation failed for one subject"
                            );
                        }
                    }
                }
                if cross_level
                    && let Err(error) = self
                        .mark_cross_level_conflicts(&decisions, !incomplete, evaluation_time)
                        .await
                {
                    counts.errors += 1;
                    warn!(
                        rule_set_id = rule_set.id.as_str(),
                        title_id = title.id.as_str(),
                        error = %error,
                        "could not record a lifecycle cross-level conflict"
                    );
                }
            }
        }

        self.cancel_out_of_scope_lifecycle_candidates(
            &rule_set.id,
            &in_scope,
            evaluation_time,
            &mut counts,
        )
        .await?;

        Ok(counts)
    }

    /// The live candidates of the rule's current revision for a chunk's
    /// subjects, one read per subject kind. A failed read is left out: those
    /// candidates then read their own step rows when reconciled.
    async fn lifecycle_chunk_active_candidates<'s>(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subjects: impl Iterator<Item = &'s super::subjects::LifecycleSubject>,
    ) -> Vec<LifecycleCandidate> {
        let mut ids_by_kind: HashMap<&'static str, Vec<String>> = HashMap::new();
        for subject in subjects {
            ids_by_kind
                .entry(subject.kind.as_storage_str())
                .or_default()
                .push(subject.id.clone());
        }
        let mut candidates = Vec::new();
        for (kind, ids) in ids_by_kind {
            match self
                .services
                .customization
                .lifecycle_evaluation
                .list_active_subject_candidates(rule_set_id, kind, &ids)
                .await
            {
                Ok(found) => candidates.extend(
                    found
                        .into_iter()
                        .filter(|candidate| candidate.revision_number == revision_number),
                ),
                Err(error) => debug!(
                    rule_set_id,
                    error = %error,
                    "could not read a chunk's lifecycle candidates ahead; each reads its own"
                ),
            }
        }
        candidates
    }

    /// Record on each of one title's live candidates whether the rule decided
    /// a conflicting action for a subject that encloses it or that it
    /// encloses, from the decisions this evaluation already made.
    ///
    /// The marker only ever holds: the executor blocks a marked candidate with
    /// [`super::execution_reason::ACTIONS_CONFLICT`] before building any step,
    /// and an evaluation that finds the conflict gone clears it, after which
    /// the candidate runs through the full execution recheck as any other. A
    /// row is written only when its marker changes, with the same
    /// compare-and-set every other evaluator write uses, and one title's
    /// changes land together in one transaction. With `may_clear` unset, some
    /// subject of the title is missing from `decisions`, and no marker is
    /// cleared.
    async fn mark_cross_level_conflicts(
        &self,
        decisions: &[LifecycleSubjectDecision],
        may_clear: bool,
        evaluation_time: DateTime<Utc>,
    ) -> AppResult<()> {
        let changes: Vec<(String, Option<scryer_domain::LifecycleActionsConflict>)> =
            super::cross_level::cross_level_markers(decisions)
                .into_iter()
                .filter_map(|(index, desired)| {
                    let candidate = decisions[index].candidate.as_ref()?;
                    let changed =
                        candidate.actions_conflict != desired && (desired.is_some() || may_clear);
                    changed.then(|| (candidate.id.clone(), desired))
                })
                .collect();
        if changes.is_empty() {
            return Ok(());
        }
        self.services
            .customization
            .lifecycle_evaluation
            .set_candidate_actions_conflicts(&changes, EVALUATOR_WRITABLE_STATES, evaluation_time)
            .await
            .map(|_| ())
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
    async fn cancel_out_of_scope_lifecycle_candidates(
        &self,
        rule_set_id: &str,
        in_scope: &HashSet<String>,
        evaluation_time: DateTime<Utc>,
        counts: &mut RuleCounts,
    ) -> AppResult<()> {
        let candidates = &self.services.customization.lifecycle_evaluation;
        let live = candidates
            .list_candidates(&LifecycleCandidateQuery {
                rule_set_id: Some(rule_set_id.to_string()),
                states: EVALUATOR_WRITABLE_STATES.to_vec(),
                ..Default::default()
            })
            .await?;

        for candidate in live {
            if in_scope.contains(&candidate.subject_id) {
                continue;
            }
            if candidates
                .transition_candidate_state(
                    &candidate.id,
                    LifecycleCandidateState::Canceled,
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
    ///
    /// With `track` set, returns what was decided for the subject and its
    /// live candidate, for [`Self::mark_cross_level_conflicts`]. A subject
    /// this pass cannot decide (unknown, an error, or a leased candidate) is
    /// paired on the actions its candidate stored, so a conflict it took part
    /// in keeps holding the other side. `incomplete` is set when the subject
    /// errored or another writer won one of its writes, since its partners
    /// then cannot be judged free of it.
    #[allow(clippy::too_many_arguments)]
    async fn reconcile_lifecycle_title(
        &self,
        track: bool,
        detail: &LifecycleRuleSetDetail,
        evaluator: &mut LifecycleRulesEvaluator,
        title: &Title,
        subject: &super::subjects::LifecycleSubject,
        title_input: Option<&scryer_rules::lifecycle::LifecycleInput>,
        people: LifecycleTitlePeople<'_>,
        watch: LifecycleTitleWatch<'_>,
        claims: LifecycleTitleClaims<'_>,
        lists: &super::list_facts::LifecycleListFacts,
        series_movies: &[LifecycleSeriesMovieDoc],
        libraries: &HashMap<String, LifecycleLibraryRef>,
        excluded: &[LifecycleRuleExclusion],
        evaluation_time: DateTime<Utc>,
        storage: &mut LifecyclePassStorage,
        started_reads: &mut super::action_execution::StartedDeleteReads,
        counts: &mut RuleCounts,
        incomplete: &mut bool,
    ) -> AppResult<Option<LifecycleSubjectDecision>> {
        let rule_set_id = detail.rule_set.id.as_str();
        let candidates = &self.services.customization.lifecycle_evaluation;

        let mut active = candidates
            .get_active_subject_candidate(rule_set_id, subject.kind.as_storage_str(), &subject.id)
            .await?;
        // The pairing a subject gets when this pass cannot decide it: the
        // actions its live candidate stored. With `as_candidate` unset the
        // candidate pairs but is not itself marked.
        let stored_decision = |candidate: &LifecycleCandidate, as_candidate: bool| {
            if !track {
                return None;
            }
            let stored = super::declared_actions::decided_actions_from_json(
                candidate.decided_actions_json.as_deref(),
            );
            tracked_decision(
                track,
                subject,
                stored.as_ref(),
                as_candidate.then_some(candidate),
            )
        };

        // A candidate under an execution lease belongs to the action handler for
        // the length of that lease, so this pass leaves it entirely alone — no
        // advance, no cancel, no hold, not even an evaluated tally. The executor
        // re-evaluates the subject on fresh facts immediately before it acts, so
        // nothing is lost by not deciding here; deciding anyway is how a cancel
        // lands on top of a succeeded row, or a succeeded row on top of a
        // cancel.
        if let Some(candidate) = active.as_ref()
            && candidate.state == LifecycleCandidateState::Executing
        {
            debug!(
                rule_set_id,
                title_id = title.id.as_str(),
                candidate_id = candidate.id.as_str(),
                "skipping a candidate the action handler holds a lease on"
            );
            // A leased row cannot be marked; its stored actions still pair.
            return Ok(stored_decision(candidate, false));
        }

        // Exclusions are honoured from the first evaluation:
        // an excluded subject is never evaluated, and an existing candidate for
        // it becomes terminal rather than lingering as live membership.
        if subject.excluded_by(&title.id, excluded) {
            if let Some(candidate) = active {
                if candidates
                    .transition_candidate_state(
                        &candidate.id,
                        LifecycleCandidateState::Excluded,
                        candidate_reason::EXCLUDED,
                        EVALUATOR_WRITABLE_STATES,
                        evaluation_time,
                    )
                    .await?
                {
                    counts.excluded += 1;
                } else {
                    *incomplete = true;
                }
            }
            return Ok(None);
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
                    LifecycleCandidateState::Canceled,
                    candidate_reason::REVISION_SUPERSEDED,
                    EVALUATOR_WRITABLE_STATES,
                    evaluation_time,
                )
                .await?
            {
                counts.superseded += 1;
            } else {
                *incomplete = true;
            }
            // The row is no longer this pass's to continue either way: it either
            // closed here, or another writer moved it and owns what happens next.
            active = None;
        }

        let library = libraries
            .get(&title.library_id)
            .cloned()
            .unwrap_or_else(|| LifecycleLibraryRef {
                id: title.library_id.clone(),
                name: String::new(),
            });
        // A delete this candidate's own sequence already began changed the
        // subject's monitoring and files itself. The rule is judged on the
        // subject as it was before that work, so a half-finished delete does
        // not cancel itself; any outside change still reads live.
        let started_delete = match active.as_ref() {
            Some(candidate) => match self
                .lifecycle_started_sequence_delete(candidate, started_reads)
                .await
            {
                Ok(started) => started,
                Err(error) => {
                    warn!(
                        rule_set_id,
                        candidate_id = candidate.id.as_str(),
                        error = %error,
                        "could not read a started lifecycle delete; holding its candidate"
                    );
                    counts.errors += 1;
                    *incomplete = true;
                    candidates
                        .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                        .await?;
                    counts.held += 1;
                    return Ok(stored_decision(candidate, true));
                }
            },
            None => None,
        };
        let deletion_checkpoint = started_delete
            .as_ref()
            .and_then(|started| started.checkpoint.as_ref());
        let completed_retention_episode_ids = deletion_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.completed_retention_episode_ids());
        let mut input = build_subject_input_with_completed_retention_episodes(
            evaluation_time,
            title,
            &library,
            subject,
            people,
            watch,
            claims,
            series_movies,
            completed_retention_episode_ids.as_ref(),
        );
        super::list_facts::apply_list_facts(&mut input, lists, &title.id);
        super::list_facts::apply_watchlist_and_metadata_facts(&mut input, lists, &title.id, people);
        // An evaluation pass samples each configured root once. A deletion
        // checkpoint may restore the action's own monitoring/file facts below,
        // but it must never restore or overwrite the capacity figures.
        self.lifecycle_pass_storage_facts(
            storage,
            &mut input.facts,
            &subject.files,
            LifecycleStorageRead::Sampled(evaluation_time),
        )
        .await;
        // A season or episode is evaluated beside its title's own facts.
        if let Some(title_input) = title_input
            && subject.kind != scryer_domain::LifecycleRuleSubjectKind::Title
        {
            input = nest_subject_input(title_input.clone(), input, subject);
        }
        if let (Some(candidate), Some(started)) = (active.as_ref(), started_delete.as_ref())
            && let Err(reason) = self
                .restore_sequence_own_effects(
                    candidate,
                    title,
                    &started.sequence,
                    &started.steps,
                    started.checkpoint.as_ref(),
                    &mut input,
                    super::action_execution::SequenceOwnEffects::MonitoringAndFiles,
                    super::action_execution::OwnMonitoringEvidence::Pass(subject),
                )
                .await
        {
            debug!(
                rule_set_id,
                candidate_id = candidate.id.as_str(),
                reason,
                "could not restore a started lifecycle delete's own effects; holding its candidate"
            );
            counts.held += 1;
            candidates
                .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                .await?;
            return Ok(stored_decision(candidate, true));
        }
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
            *incomplete = true;
            if let Some(candidate) = active {
                candidates
                    .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                    .await?;
                counts.held += 1;
                return Ok(stored_decision(&candidate, true));
            }
            return Ok(None);
        };

        match decision.outcome {
            LifecycleOutcome::Match => {
                counts.matched += 1;
                // What the rule decided for this subject now. A continuing
                // match that decides different actions is a different intent:
                // the old candidate closes and a new one carries the new
                // actions. The grace clock and the attempt count carry over,
                // except when the decision deletes something the stored one
                // did not, which restarts both so every delete waits out the
                // full grace.
                let decided_json = Some(super::declared_actions::decided_actions_to_json(
                    &decision.actions,
                ));
                let mut carried_clock = None;
                let mut carried_attempts = 0;
                if let Some(candidate) = active.as_ref() {
                    let stored = super::declared_actions::decided_actions_from_json(
                        candidate.decided_actions_json.as_deref(),
                    );
                    if stored.as_ref() != Some(&decision.actions) {
                        let newly_destructive =
                            decision_deletes_more(stored.as_ref(), &decision.actions);
                        if candidates
                            .transition_candidate_state(
                                &candidate.id,
                                LifecycleCandidateState::Canceled,
                                candidate_reason::ACTIONS_CHANGED,
                                EVALUATOR_WRITABLE_STATES,
                                evaluation_time,
                            )
                            .await?
                        {
                            counts.superseded += 1;
                            if !newly_destructive {
                                carried_clock =
                                    Some((candidate.first_matched_at, candidate.due_at));
                                // The budget the executor charges lives on
                                // the step rows, keyed by candidate; carry
                                // what the high-risk steps already spent.
                                // Read only on this rare recreate.
                                let steps = candidates
                                    .list_action_steps(
                                        &candidate.id,
                                        candidate.match_generation,
                                        candidate.revision_number,
                                    )
                                    .await?;
                                carried_attempts = carried_high_risk_attempts(candidate, &steps);
                            }
                            active = None;
                        } else {
                            // Another writer owns the row now; the next pass
                            // decides again.
                            *incomplete = true;
                            return Ok(None);
                        }
                    }
                }
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
                        return Ok(tracked_decision(
                            track,
                            subject,
                            Some(&decision.actions),
                            Some(&candidate),
                        ));
                    }
                    None => {
                        // A terminal sequence membership is distinct from an
                        // active candidate: a completed or exhausted sequence
                        // has no live candidate row, but a continuous match
                        // must never manufacture a new generation and reset
                        // its step budgets. Only the confirmed no-match arm
                        // below releases this marker.
                        if let Some(latch) = candidates
                            .get_active_sequence_completion(
                                rule_set_id,
                                detail.rule_set.current_revision_number,
                                subject.kind.as_storage_str(),
                                &subject.id,
                            )
                            .await?
                        {
                            // Declared actions build their steps per subject:
                            // a decision that would now run different steps
                            // is new work, not the finished membership.
                            let same_steps = latch.expected_step_ids
                                == super::declared_actions::declared_step_ids(&decision.actions);
                            if same_steps {
                                // A finished membership has no row to mark,
                                // but what it decided still pairs.
                                return Ok(tracked_decision(
                                    track,
                                    subject,
                                    Some(&decision.actions),
                                    None,
                                ));
                            }
                            candidates
                                .release_sequence_completion_on_confirmed_non_match(
                                    rule_set_id,
                                    detail.rule_set.current_revision_number,
                                    subject.kind.as_storage_str(),
                                    &subject.id,
                                    evaluation_time,
                                )
                                .await?;
                        }
                        let generation = candidates
                            .max_subject_match_generation(
                                rule_set_id,
                                subject.kind.as_storage_str(),
                                &subject.id,
                            )
                            .await?
                            + 1;
                        let grace_days = detail.revision.grace_days.max(0);
                        let (first_matched_at, due_at) = carried_clock.unwrap_or((
                            evaluation_time,
                            // Materialized, so a zero-day grace is due the
                            // instant it matches.
                            evaluation_time + Duration::days(grace_days),
                        ));
                        let created = LifecycleCandidate {
                            id: Id::new().0,
                            rule_set_id: rule_set_id.to_string(),
                            revision_number: detail.rule_set.current_revision_number,
                            source_content_hash: detail.revision.source_content_hash.clone(),
                            title_id: title.id.clone(),
                            subject_id: subject.id.clone(),
                            library_id: title.library_id.clone(),
                            facet: title.facet.as_str().to_string(),
                            subject_kind: subject.kind.as_storage_str().to_string(),
                            match_generation: generation,
                            state: LifecycleCandidateState::Observing,
                            state_reason: if carried_clock.is_some() {
                                candidate_reason::ACTIONS_CHANGED
                            } else {
                                candidate_reason::FIRST_MATCH
                            }
                            .to_string(),
                            reason_codes: decision.reason_codes.clone(),
                            action_kind: ACTION_SEQUENCE_KIND.to_string(),
                            decided_actions_json: decided_json,
                            actions_conflict: None,
                            grace_days,
                            first_matched_at,
                            last_matched_at: evaluation_time,
                            due_at,
                            last_evaluated_at: evaluation_time,
                            held_since: None,
                            action_attempts: carried_attempts,
                            created_at: evaluation_time,
                            updated_at: evaluation_time,
                        };
                        candidates.create_candidate(&created).await?;
                        counts.created += 1;
                        return Ok(tracked_decision(
                            track,
                            subject,
                            Some(&decision.actions),
                            Some(&created),
                        ));
                    }
                }
            }
            LifecycleOutcome::NoMatch => {
                counts.no_match += 1;
                // The latch represents continuous membership, not a permanent
                // ban. A raw confirmed no-match is the only evidence that
                // starts a fresh generation; unknown and hold paths never
                // reach this branch.
                candidates
                    .release_sequence_completion_on_confirmed_non_match(
                        rule_set_id,
                        detail.rule_set.current_revision_number,
                        subject.kind.as_storage_str(),
                        &subject.id,
                        evaluation_time,
                    )
                    .await?;
                if let Some(candidate) = active {
                    if candidates
                        .transition_candidate_state(
                            &candidate.id,
                            LifecycleCandidateState::Canceled,
                            candidate_reason::NO_MATCH,
                            EVALUATOR_WRITABLE_STATES,
                            evaluation_time,
                        )
                        .await?
                    {
                        counts.canceled += 1;
                    } else {
                        *incomplete = true;
                    }
                }
            }
            LifecycleOutcome::Unknown => {
                // Unknown holds: never advance, never cancel (RFC 7.5).
                counts.unknown += 1;
                if let Some(candidate) = active {
                    candidates
                        .hold_candidate(&candidate.id, evaluation_time, evaluation_time)
                        .await?;
                    counts.held += 1;
                    return Ok(stored_decision(&candidate, true));
                }
            }
        }

        Ok(None)
    }

    /// One batched media-file load per chunk, grouped by title, exactly as
    /// preview does. A per-title query here would make the job's cost scale
    /// with the library.
    pub(crate) async fn lifecycle_files_for_titles(
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
    pub(crate) async fn lifecycle_series_movies_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<LifecycleSeriesMovieDoc>>> {
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

        let mut by_title: HashMap<String, Vec<LifecycleSeriesMovieDoc>> = HashMap::new();
        for link in links {
            by_title
                .entry(link.series_title_id.clone())
                .or_default()
                .push(LifecycleSeriesMovieDoc {
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
    pub(crate) async fn lifecycle_requesters_for_titles(
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

    /// One batched lifecycle-claim load per chunk, keyed by title.
    ///
    /// Two reads, merged: the live claims of every kind (which is what a keep
    /// claim and a running lease look like) and the retention history (which is
    /// what a *lapsed* lease looks like — the live read cannot see it, and
    /// without it "expired" and "never had one" are the same answer). Merged
    /// rather than handed over separately because the fact builder asks one
    /// question of them — is anything holding this title, and if not, did
    /// something used to.
    pub(crate) async fn lifecycle_claims_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<scryer_domain::LifecycleClaim>>> {
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let claims = &self.services.catalog.lifecycle_claims;
        let mut by_title = claims.list_live_for_titles(&title_ids).await?;
        for (title_id, history) in claims.list_retention_history_for_titles(&title_ids).await? {
            let entry = by_title.entry(title_id).or_default();
            for claim in history {
                if !entry.iter().any(|stored| stored.id == claim.id) {
                    entry.push(claim);
                }
            }
        }
        Ok(by_title)
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
    pub(crate) async fn lifecycle_watch_context(&self) -> AppResult<LifecycleWatchContext> {
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
        // The roster the *facts* answer from, too: a signal row written by a
        // connection that is no longer enabled is not part of this run's watch
        // picture, and the gate below already drops those connections.
        let enabled_connection_ids: HashSet<String> = connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        let favorite_connection_ids: HashSet<String> = connections
            .iter()
            .filter(|connection| connection.provider.has_favorites())
            .map(|connection| connection.id.clone())
            .collect();
        let plex_connected = connections
            .iter()
            .any(|connection| !connection.provider.has_favorites());
        if connections.is_empty() {
            return Ok(LifecycleWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::NO_MEDIA_SERVER_CONNECTION,
                ),
                linked_user_ids: HashSet::new(),
                unswept_user_ids: HashSet::new(),
                enabled_connection_ids: HashSet::new(),
                audience: WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ),
                favorite_connection_ids: HashSet::new(),
                favorite_user_ids: HashSet::new(),
                favorite_accounts: HashMap::new(),
                favorite_unread_user_ids: HashSet::new(),
                audience_favorites: WatchSignalFreshness::Unavailable(
                    unknown_reason::AUDIENCE_FAVORITES_NOT_READ,
                ),
                plex_connected,
            });
        }

        let sync_states = self
            .services
            .integrations
            .media_server_signals
            .signal_sync_states()
            .await?;
        if sync_states.iter().any(|state| {
            state.last_error.is_some()
                && connections
                    .iter()
                    .any(|connection| connection.id == state.connection_id)
        }) {
            return Ok(LifecycleWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::SIGNAL_SYNC_INCOMPLETE,
                ),
                linked_user_ids: HashSet::new(),
                unswept_user_ids: HashSet::new(),
                enabled_connection_ids: enabled_connection_ids.clone(),
                audience: WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ),
                favorite_connection_ids: favorite_connection_ids.clone(),
                favorite_user_ids: HashSet::new(),
                favorite_accounts: HashMap::new(),
                favorite_unread_user_ids: HashSet::new(),
                audience_favorites: WatchSignalFreshness::Unavailable(
                    unknown_reason::AUDIENCE_FAVORITES_NOT_READ,
                ),
                plex_connected,
            });
        }
        // The stored `enabled` is the connection's state *as of that sweep*, so
        // a state row that says disabled describes a picture built while the
        // connection was off. A re-enabled connection therefore has no usable
        // success timestamp until its first post-re-enable sweep lands: taking
        // the pre-disable one would report a watch picture up to
        // `WATCH_SIGNAL_FRESHNESS_HOURS` older than the gate believes, and
        // those facts gate destructive lifecycle actions.
        let states: HashMap<String, DateTime<Utc>> = sync_states
            .into_iter()
            .filter(|state| state.enabled)
            .filter_map(|state| Some((state.connection_id, state.last_success_at?)))
            .collect();

        // Never-swept is reported ahead of stale: it is the more specific
        // answer, and the one an operator can actually act on.
        if connections
            .iter()
            .any(|connection| !states.contains_key(&connection.id))
        {
            return Ok(LifecycleWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::SIGNAL_SYNC_NEVER_SUCCEEDED,
                ),
                linked_user_ids: HashSet::new(),
                unswept_user_ids: HashSet::new(),
                enabled_connection_ids: enabled_connection_ids.clone(),
                audience: WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ),
                favorite_connection_ids: favorite_connection_ids.clone(),
                favorite_user_ids: HashSet::new(),
                favorite_accounts: HashMap::new(),
                favorite_unread_user_ids: HashSet::new(),
                audience_favorites: WatchSignalFreshness::Unavailable(
                    unknown_reason::AUDIENCE_FAVORITES_NOT_READ,
                ),
                plex_connected,
            });
        }
        let now = Utc::now();
        if connections
            .iter()
            .filter_map(|connection| states.get(&connection.id))
            .any(|last_success_at| !watch_signal_sweep_is_fresh(*last_success_at, now))
        {
            return Ok(LifecycleWatchContext {
                freshness: WatchSignalFreshness::Unavailable(unknown_reason::SIGNALS_STALE),
                linked_user_ids: HashSet::new(),
                unswept_user_ids: HashSet::new(),
                enabled_connection_ids: enabled_connection_ids.clone(),
                audience: WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ),
                favorite_connection_ids: favorite_connection_ids.clone(),
                favorite_user_ids: HashSet::new(),
                favorite_accounts: HashMap::new(),
                favorite_unread_user_ids: HashSet::new(),
                audience_favorites: WatchSignalFreshness::Unavailable(
                    unknown_reason::AUDIENCE_FAVORITES_NOT_READ,
                ),
                plex_connected,
            });
        }

        // Only resolved once the gate has passed: with watch facts already
        // unknown, the roster would answer a question nothing is going to ask.
        //
        // A link counts as observed only when the signal roster says a sweep
        // read it: that row is written in the same transaction as the signals
        // read through it, and retired with them. A link made after the last
        // sweep listed its participants has no row however its timestamps
        // fall, so "this requester did not watch it" is never a known answer
        // about someone whose plays Scryer has not read. The match is on the
        // link and the identity it reads, not on `verified_at`: a login
        // refreshes that without changing whose plays are read, while a link
        // re-pointed at another server account no longer matches its row.
        let read_links: HashSet<(String, String, String, String)> = self
            .services
            .integrations
            .media_server_signals
            .signal_roster()
            .await?
            .into_iter()
            .map(|entry| {
                (
                    entry.connection_id,
                    entry.link.link_id,
                    entry.external_user_id,
                    entry.link.scryer_user_id,
                )
            })
            .collect();
        let mut linked_user_ids = HashSet::new();
        let mut unswept_user_ids = HashSet::new();
        let mut favorite_user_ids = HashSet::new();
        let mut favorite_accounts: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
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
                let read_account =
                    account
                        .external_user_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|external_user_id| {
                            read_links.contains(&(
                                connection.id.clone(),
                                account.id.clone(),
                                (*external_user_id).to_string(),
                                account.user_id.clone(),
                            ))
                        });
                match read_account {
                    None => {
                        unswept_user_ids.insert(account.user_id.clone());
                    }
                    Some(external_user_id) if favorite_connection_ids.contains(&connection.id) => {
                        favorite_user_ids.insert(account.user_id.clone());
                        favorite_accounts
                            .entry((connection.id.clone(), external_user_id.to_string()))
                            .or_default()
                            .insert(account.user_id.clone());
                    }
                    Some(_) => {}
                }
                linked_user_ids.insert(account.user_id);
            }
        }
        // A user is observed only when every one of their links was read.
        linked_user_ids.retain(|user_id| !unswept_user_ids.contains(user_id));
        favorite_user_ids.retain(|user_id| linked_user_ids.contains(user_id));

        // The audience read is every account's, linked or not, so it is
        // judged on its own and stands whatever the links' state.
        let audience = self.lifecycle_audience_freshness(&connections, now).await;
        let (audience_favorites, fresh_favorite_accounts) = self
            .lifecycle_audience_favorite_freshness(&favorite_connection_ids, now)
            .await;
        // A member's favourites are known only while every account they
        // linked on a favourite server had its favourites read in full.
        let favorite_unread_user_ids: HashSet<String> = favorite_accounts
            .iter()
            .filter(|(account, _)| !fresh_favorite_accounts.contains(*account))
            .flat_map(|(_, user_ids)| user_ids.iter().cloned())
            .collect();

        // A fresh sweep over nobody is not a watch picture. With no verified
        // link on any enabled connection, every subject would come back "no
        // plays recorded" — which reads as "nobody watched it" while Scryer is
        // in fact observing nobody at all.
        //
        // A link no sweep has read yet is the same gap with a different
        // remedy (wait for the next sweep). It closes the per-user facts even
        // beside links that were read: that person's plays have never been
        // seen, so "nobody watched it" and every watcher list would leave
        // them out while reading as complete.
        if linked_user_ids.is_empty() || !unswept_user_ids.is_empty() {
            let reason = if unswept_user_ids.is_empty() {
                unknown_reason::NO_LINKED_PARTICIPANTS
            } else {
                unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
            };
            return Ok(LifecycleWatchContext {
                freshness: WatchSignalFreshness::Unavailable(reason),
                linked_user_ids,
                unswept_user_ids,
                enabled_connection_ids,
                audience,
                favorite_connection_ids,
                favorite_user_ids,
                favorite_accounts,
                favorite_unread_user_ids,
                audience_favorites,
                plex_connected,
            });
        }

        Ok(LifecycleWatchContext {
            freshness: WatchSignalFreshness::Fresh,
            linked_user_ids,
            unswept_user_ids,
            enabled_connection_ids,
            audience,
            favorite_connection_ids,
            favorite_user_ids,
            favorite_accounts,
            favorite_unread_user_ids,
            audience_favorites,
            plex_connected,
        })
    }

    /// One batched movie-signal load per chunk, keyed by owning title.
    ///
    /// Only movie-facet titles are asked about, for the same reason only they
    /// get a watch answer: episode signals roll up to nothing this wave can
    /// report, so fetching them would be work whose result is discarded. A
    /// closed gate skips the query entirely — every watch fact is already
    /// unknown, so the rows could not change a decision.
    pub(crate) async fn lifecycle_watch_signals_for_titles(
        &self,
        context: &LifecycleWatchContext,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<scryer_domain::UserMediaSignal>>> {
        if context.freshness != WatchSignalFreshness::Fresh {
            return Ok(HashMap::new());
        }
        self.lifecycle_stored_watch_signals_for_titles(titles).await
    }

    /// Every stored watch row for `titles`, whatever the watch gate says.
    /// Only the live re-check reads these past a closed gate, as the baseline
    /// a live answer is compared against; facts never come from them then.
    pub(crate) async fn lifecycle_stored_watch_signals_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<scryer_domain::UserMediaSignal>>> {
        let title_ids: Vec<String> = titles
            .iter()
            .filter(|title| title.facet == scryer_domain::MediaFacet::Movie)
            .map(|title| title.id.clone())
            .collect();
        let series_ids: Vec<String> = titles
            .iter()
            .filter(|title| title.facet != scryer_domain::MediaFacet::Movie)
            .map(|title| title.id.clone())
            .collect();
        let mut signals = if title_ids.is_empty() {
            HashMap::new()
        } else {
            self.services
                .integrations
                .media_server_signals
                .movie_signals_for_titles(&title_ids)
                .await?
        };
        if !series_ids.is_empty() {
            signals.extend(
                self.services
                    .integrations
                    .media_server_signals
                    .episode_signals_for_titles(&series_ids)
                    .await?,
            );
        }
        Ok(signals)
    }

    /// Whether every enabled connection's full audience was read recently.
    ///
    /// A store that cannot answer leaves only the anonymous facts unknown;
    /// the rest of the watch picture does not depend on it.
    async fn lifecycle_audience_freshness(
        &self,
        connections: &[scryer_domain::MediaServerConnection],
        now: DateTime<Utc>,
    ) -> WatchSignalFreshness {
        let reads = match self
            .services
            .integrations
            .media_server_signals
            .audience_reads()
            .await
        {
            Ok(reads) => reads,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "audience reads could not be loaded; the anonymous watch facts are unknown"
                );
                return WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ);
            }
        };
        let read_at: HashMap<&str, DateTime<Utc>> = reads
            .iter()
            .map(|read| (read.connection_id.as_str(), read.read_at))
            .collect();
        let every_connection_read = connections.iter().all(|connection| {
            read_at
                .get(connection.id.as_str())
                .is_some_and(|read_at| watch_signal_sweep_is_fresh(*read_at, now))
        });
        if every_connection_read {
            WatchSignalFreshness::Fresh
        } else {
            WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ)
        }
    }

    /// Whether every account on every favourite connection had its
    /// favourites read in full recently, and which accounts did. One read
    /// of the stored read states, nothing when no enabled server has
    /// favourites.
    ///
    /// A connection with no account row has not had its favourites read, so
    /// it holds the anonymous fact unknown rather than reading as nobody's.
    async fn lifecycle_audience_favorite_freshness(
        &self,
        favorite_connection_ids: &HashSet<String>,
        now: DateTime<Utc>,
    ) -> (WatchSignalFreshness, HashSet<(String, String)>) {
        let not_read =
            WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_FAVORITES_NOT_READ);
        if favorite_connection_ids.is_empty() {
            return (not_read, HashSet::new());
        }
        let reads = match self
            .services
            .integrations
            .media_server_signals
            .audience_favorite_reads()
            .await
        {
            Ok(reads) => reads,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "audience favourite reads could not be loaded; the favourite facts are unknown"
                );
                return (not_read, HashSet::new());
            }
        };
        let mut listed: HashSet<&str> = HashSet::new();
        let mut every_account_read = true;
        let mut fresh = HashSet::new();
        for read in reads
            .iter()
            .filter(|read| favorite_connection_ids.contains(&read.connection_id))
        {
            listed.insert(read.connection_id.as_str());
            if read
                .read_at
                .is_some_and(|read_at| watch_signal_sweep_is_fresh(read_at, now))
            {
                fresh.insert((read.connection_id.clone(), read.external_account_id.clone()));
            } else {
                every_account_read = false;
            }
        }
        let freshness = if every_account_read && listed.len() == favorite_connection_ids.len() {
            WatchSignalFreshness::Fresh
        } else {
            not_read
        };
        (freshness, fresh)
    }

    /// One batched audience load per chunk, keyed by owning title. Skipped
    /// when the anonymous facts are already unknown.
    ///
    /// A load that fails does not fail the pass: the returned context has the
    /// audience gate closed, so the anonymous facts read unknown for this
    /// chunk rather than "nobody watched it".
    pub(crate) async fn lifecycle_audience_plays_for_titles<'c>(
        &self,
        context: &'c LifecycleWatchContext,
        titles: &[Title],
    ) -> (
        std::borrow::Cow<'c, LifecycleWatchContext>,
        HashMap<String, Vec<crate::ports::AudiencePlay>>,
    ) {
        let unread = || {
            std::borrow::Cow::Owned(LifecycleWatchContext {
                audience: WatchSignalFreshness::Unavailable(unknown_reason::AUDIENCE_NOT_READ),
                ..context.clone()
            })
        };
        if context.audience_unavailable_reason().is_some() {
            return (std::borrow::Cow::Borrowed(context), HashMap::new());
        }
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        if title_ids.is_empty() {
            return (std::borrow::Cow::Borrowed(context), HashMap::new());
        }
        match self
            .services
            .integrations
            .media_server_signals
            .audience_plays_for_titles(&title_ids)
            .await
        {
            Ok(plays) => (std::borrow::Cow::Borrowed(context), plays),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "audience plays could not be loaded; the anonymous watch facts are unknown"
                );
                (unread(), HashMap::new())
            }
        }
    }

    /// Username by user id, read once per evaluation run.
    ///
    /// A user that no longer exists is simply missing from the map, which the
    /// facts builder turns into an unknown name rather than a wrong one.
    pub(crate) async fn lifecycle_usernames_by_id(&self) -> AppResult<HashMap<String, String>> {
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
