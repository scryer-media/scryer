//! Authoring and on-demand preview for lifecycle rule sets.
//!
//! Everything here is authoring-time. Nothing schedules evaluation, writes
//! candidates, or executes an action: [`preview_lifecycle_rule`] is the only
//! path that runs a matcher, it runs on request against a bounded selection,
//! and it persists nothing.
//!
//! [`preview_lifecycle_rule`]: AppUseCase::preview_lifecycle_rule

use std::collections::HashMap;

use chrono::Utc;
use scryer_domain::{
    AppPermission, Id, LifecycleRuleRevision, LifecycleRuleSet, LifecycleRuleState,
    LifecycleRuleSubjectKind, MediaFacet, ScriptSchedule, Title, User,
};
use scryer_rules::lifecycle::{
    LifecycleOutcome, LifecyclePolicy, LifecycleRulesEngine, PERSON_TARGETED_LIFECYCLE_FACTS,
    rewrite_package_declaration,
};
use scryer_rules::runtime::content_hash;
use scryer_rules::validation::{
    ValidationResult, lifecycle_referenced_facts, validate_lifecycle_rule,
};

use crate::lifecycle_rules::LifecycleDeclaredActions;
use crate::lifecycle_rules::facts::{
    LifecycleLibraryRef, LifecycleTitleClaims, LifecycleTitlePeople, LifecycleTitleWatch,
    build_subject_input,
};
use crate::rules::builder_model::{BuilderFamily, BuilderModelChange, resolved_builder_model};
use crate::{AppError, AppResult, AppUseCase};
use scryer_rules::lifecycle_actions::scan_lifecycle_action_heads;
use scryer_rules::validation::ValidationDiagnostic;

/// Upper bound on how many titles one preview may evaluate. Preview is
/// synchronous and builds a fact snapshot per title, so the cap is what keeps a
/// request from turning into a library-wide scan.
pub const LIFECYCLE_PREVIEW_MAX_TITLES: usize = 50;

/// Selection size used when the caller does not ask for one.
pub const LIFECYCLE_PREVIEW_DEFAULT_TITLES: usize = 20;

/// Upper bound on a rule's grace period, in days — ten years.
///
/// The grace period is materialized into a candidate's `due_at` as
/// `first_matched_at + Duration::days(grace_days)`, and `chrono` *panics* rather
/// than saturating once that addition leaves the representable range. A rule
/// authored with a nonsense grace period would therefore not be a slow rule; it
/// would take down the evaluation pass. Ten years is far past any real retention
/// policy and nowhere near the overflow.
pub const LIFECYCLE_MAX_GRACE_DAYS: i64 = 3650;

/// Name of the lifecycle rule every instance ships with.
pub(crate) const EXPIRED_REQUEST_LEASES_RULE_NAME: &str = "Expired request leases";

/// Matcher of that rule. The web editor's `expired-request-leases` template
/// carries the same source; keep the two identical.
pub const EXPIRED_REQUEST_LEASES_REGO: &str = "package rules\nimport rego.v1\n\ndelete_title if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n";

/// The matcher that rule shipped with while a separate action definition said
/// what a match did: delete the title and its files. Startup replaces it with
/// [`EXPIRED_REQUEST_LEASES_REGO`].
pub(crate) const MATCH_FORM_EXPIRED_REQUEST_LEASES_REGO: &str = "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n";

pub(crate) const EXPIRED_REQUEST_LEASES_DESCRIPTION: &str =
    "Deletes a requested title and its files once its request lease has expired.";

/// The description that rule shipped with while a request could also write a
/// keep claim. Startup replaces it with the current one.
pub(crate) const PREVIOUS_EXPIRED_REQUEST_LEASES_DESCRIPTION: &str = "Deletes a requested title and its files once its request lease has expired, unless someone keeps it.";

// ── Request models ──────────────────────────────────────────────────────────

/// A new rule set plus its first matcher revision.
#[derive(Clone, Debug)]
pub struct LifecycleRuleDraft {
    pub name: String,
    pub description: String,
    /// The rule source. Its action heads say what a match does.
    pub rego_source: String,
    pub grace_days: i64,
    /// Empty means every library.
    pub library_ids: Vec<String>,
    /// The most titles the rule acts on in one fire. Required when the rule
    /// can delete; `None` is no limit otherwise.
    pub max_titles_per_run: Option<u32>,
}

/// A replacement matcher for an existing rule set. Applying one always appends
/// a revision; it never edits the revision in force.
#[derive(Clone, Debug)]
pub struct LifecycleSourceDraft {
    pub rego_source: String,
    pub grace_days: i64,
    /// The most titles the rule may act on in one fire, saved with the new
    /// source so a delete head and its maximum arrive in one save. `None`
    /// keeps the stored maximum, `Some(None)` clears it, `Some(Some(_))`
    /// replaces it.
    pub max_titles_per_run: Option<Option<u32>>,
}

/// Which matcher a preview should run.
#[derive(Clone, Debug)]
pub enum LifecyclePreviewSource {
    /// The current revision of a stored rule set.
    Stored { rule_set_id: String },
    /// An unsaved draft from the editor. Validated and compiled, never stored.
    Inline {
        library_ids: Vec<String>,
        rego_source: String,
        grace_days: i64,
    },
}

/// Which titles a preview should evaluate.
#[derive(Clone, Debug)]
pub enum LifecyclePreviewSelection {
    /// Exactly these titles, at most [`LIFECYCLE_PREVIEW_MAX_TITLES`].
    Titles(Vec<String>),
    /// Exact subjects within explicitly selected owning titles.
    Subjects {
        title_ids: Vec<String>,
        subject_ids: Vec<String>,
    },
    /// The first `limit` titles of one library, clamped to the cap.
    Library {
        library_id: String,
        limit: Option<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct LifecyclePreviewRequest {
    pub source: LifecyclePreviewSource,
    pub selection: LifecyclePreviewSelection,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A rule set with the revision currently in force and the actions its source
/// declares, decoded so no caller downstream handles the persisted JSON shape.
#[derive(Clone, Debug)]
pub struct LifecycleRuleSetDetail {
    pub rule_set: LifecycleRuleSet,
    pub revision: LifecycleRuleRevision,
    pub declared_actions: LifecycleDeclaredActions,
    /// Set when the current revision cannot be acted on, with the reason
    /// code. Such a rule declares no action, is never evaluated, never runs
    /// and cannot be enabled; replacing its source is how it recovers.
    pub load_refusal: Option<&'static str>,
    /// Set when the rule still evaluates and previews but its actions must not
    /// run, with the reason code: an armed rule that deletes with no most
    /// titles per run. Re-saving the maximum or disarming the rule clears it.
    pub action_refusal: Option<&'static str>,
}

impl LifecycleRuleSetDetail {
    /// Whether the evaluator and executor may act on this rule at all.
    pub fn is_actionable(&self) -> bool {
        self.load_refusal.is_none()
    }

    /// The reason this rule cannot be used as it stands, for the operator:
    /// a load refusal first, then a refusal that only stops its actions.
    pub fn refusal(&self) -> Option<&'static str> {
        self.load_refusal.or(self.action_refusal)
    }
}

/// The action refusal for an enabled rule that runs a deleting action with no
/// most titles per run. Saving and enabling refuse that shape; a row that
/// reached it anyway evaluates and previews as usual and acts on nothing.
pub(crate) fn action_refusal_for(
    rule_set: &LifecycleRuleSet,
    declared: &LifecycleDeclaredActions,
) -> Option<&'static str> {
    (rule_set.state == LifecycleRuleState::Enabled
        && declared.destructive
        && rule_set.max_titles_per_run.is_none())
    .then_some(crate::error_reasons::lifecycle::RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN)
}

/// What validating a draft source found: the validation result, and the
/// actions the source declares. `declared_actions` is `None` only when the
/// source does not parse.
#[derive(Clone, Debug)]
pub struct LifecycleRuleSourceValidation {
    pub result: ValidationResult,
    pub declared_actions: Option<LifecycleDeclaredActions>,
}

#[derive(Clone, Debug)]
pub struct LifecyclePreviewResult {
    /// The stored rule set's id, or the throwaway id an inline draft compiled
    /// under.
    pub rule_set_id: String,
    /// Hash of the exact source that produced these outcomes.
    pub source_content_hash: String,
    pub evaluated_at: chrono::DateTime<Utc>,
    pub titles: Vec<LifecyclePreviewTitleResult>,
}

/// One title's outcome. `outcome` is `None` exactly when `error` is set: a rule
/// that failed produced no decision, and a failure is never rendered as
/// no-match.
#[derive(Clone, Debug)]
pub struct LifecyclePreviewTitleResult {
    pub excluded: bool,
    pub due_at: Option<chrono::DateTime<Utc>>,
    pub due_at_is_estimate: bool,
    pub subject_kind: LifecycleRuleSubjectKind,
    pub subject_id: String,
    pub subject_label: String,
    pub file_count: i64,
    pub total_size_bytes: i64,
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub library_id: String,
    pub outcome: Option<LifecycleOutcome>,
    pub reason_codes: Vec<String>,
    pub error: Option<String>,
    /// Stable code for `error`; `None` exactly when `error` is.
    pub error_code: Option<&'static str>,
    /// What the rule decided for this subject. `None` unless it matched.
    pub actions: Option<scryer_rules::lifecycle_actions::LifecycleActions>,
    /// The input document the rule evaluated for this subject, facts and
    /// observations both. `None` unless the caller may read facts about
    /// specific people, because the document carries them.
    pub facts: Option<serde_json::Value>,
}

// ── Service ─────────────────────────────────────────────────────────────────

impl AppUseCase {
    pub async fn list_lifecycle_rule_sets(&self, actor: &User) -> AppResult<Vec<LifecycleRuleSet>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .lifecycle_rule_sets
            .list_rule_sets()
            .await
    }

    /// Every rule set with the revision currently in force.
    ///
    /// A list view still has to show what each rule *does* (its action and
    /// grace period), and those live on the revision. Resolving them here keeps
    /// that from becoming one detail round trip per row; a rule set whose
    /// current revision is missing is the same repository fault the detail path
    /// reports, never a row silently rendered without its action.
    pub async fn list_lifecycle_rule_set_details(
        &self,
        actor: &User,
    ) -> AppResult<Vec<LifecycleRuleSetDetail>> {
        let rule_sets = self.list_lifecycle_rule_sets(actor).await?;
        let keys: Vec<(String, i64)> = rule_sets
            .iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set.current_revision_number))
            .collect();
        let revisions = self
            .services
            .customization
            .lifecycle_rule_sets
            .get_revisions(&keys)
            .await?;
        rule_sets
            .into_iter()
            .zip(revisions)
            .map(|(rule_set, revision)| lifecycle_rule_detail(rule_set, revision))
            .collect()
    }

    pub async fn get_lifecycle_rule_set(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<LifecycleRuleSetDetail>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let Some(rule_set) = self
            .services
            .customization
            .lifecycle_rule_sets
            .get_rule_set(id)
            .await?
        else {
            return Ok(None);
        };
        self.load_lifecycle_rule_detail(rule_set).await.map(Some)
    }

    pub async fn list_lifecycle_rule_revisions(
        &self,
        actor: &User,
        rule_set_id: &str,
    ) -> AppResult<Vec<LifecycleRuleRevision>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .lifecycle_rule_sets
            .list_revisions(rule_set_id)
            .await
    }

    pub async fn create_lifecycle_rule_set(
        &self,
        actor: &User,
        draft: LifecycleRuleDraft,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.create_lifecycle_rule_set_with_builder_model(actor, draft, BuilderModelChange::Keep)
            .await
    }

    /// Create a rule set and store the simple builder's document for its
    /// source alongside the first revision.
    pub async fn create_lifecycle_rule_set_with_builder_model(
        &self,
        actor: &User,
        draft: LifecycleRuleDraft,
        builder_model: BuilderModelChange,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.create_lifecycle_rule_set_with_schedule(actor, draft, builder_model, None)
            .await
    }

    /// Create a rule set that runs on `schedule`, or on the default schedule
    /// when it is `None`.
    pub async fn create_lifecycle_rule_set_with_schedule(
        &self,
        actor: &User,
        draft: LifecycleRuleDraft,
        builder_model: BuilderModelChange,
        schedule: Option<ScriptSchedule>,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let schedule = validated_rule_schedule(schedule)?
            .unwrap_or_else(scryer_domain::default_lifecycle_rule_schedule);
        let builder_model_json =
            resolved_builder_model(&builder_model, None, BuilderFamily::Lifecycle)?;

        let name = require_non_empty(&draft.name, "name")?;

        let id = Id::new_rego_safe().0;
        let prepared = prepare_source(&id, &draft.rego_source, draft.grace_days)?;
        require_max_titles_per_run(&prepared.declared, draft.max_titles_per_run)?;
        self.require_declared_action_targets(
            &prepared.declared,
            &prepared.scan,
            &draft.library_ids,
        )
        .await?;
        self.require_person_fact_authority(actor, &prepared.rego_source, &id)
            .await?;

        let now = Utc::now();
        let rule_set = LifecycleRuleSet {
            id: id.clone(),
            name,
            description: draft.description,
            // Ships dark: nothing evaluates stored rules yet, so a rule that
            // persisted as enabled would be advertising a capability that does
            // not exist.
            state: LifecycleRuleState::Disabled,
            edited_since_enabled: false,
            destructive_confirmed_revision: None,
            library_ids: draft.library_ids,
            system_owned: false,
            current_revision_number: 1,
            schedule,
            max_titles_per_run: draft.max_titles_per_run,
            created_at: now,
            updated_at: now,
        };
        let revision = build_revision(
            &id,
            1,
            &prepared,
            draft.grace_days,
            Some(actor.id.clone()),
            now,
            builder_model_json,
        );

        self.services
            .customization
            .lifecycle_rule_sets
            .create_rule_set(&rule_set, &revision)
            .await?;
        // A new rule starts disabled, so the scheduler has nothing to add
        // until it is enabled.

        Ok(LifecycleRuleSetDetail {
            action_refusal: action_refusal_for(&rule_set, &prepared.declared),
            rule_set,
            revision,
            declared_actions: prepared.declared,
            load_refusal: None,
        })
    }

    /// Appends revision N+1. Revision N is left exactly as it was written, so a
    /// candidate recorded against it stays attributable.
    ///
    /// When the new source defines a delete action, appending it also drops an
    /// enabled rule to dry run and clears its deletion confirmation,
    /// atomically with the append. The destructive acknowledgement is bound to
    /// the revision it was given for, so a replacement source that can delete
    /// has to be confirmed again on its own terms. Without this,
    /// catalog-settings authority alone would be enough to swap an enabled
    /// rule's source and have the next handler pass delete under someone
    /// else's acknowledgement. A rule that was enabled is flagged as edited
    /// since it was enabled, so the operator is told to review it and enable
    /// it again.
    ///
    /// A new source that defines no delete action keeps the rule's state: it
    /// cannot delete anything, so there is no destructive blast radius to
    /// acknowledge again. A source that declares no action at all leaves an
    /// enabled rule in dry run, since enabled would do nothing. A metadata
    /// rename does *not* disarm: it changes neither what the rule matches nor
    /// what it does, so the acknowledgement still describes the same blast
    /// radius.
    pub async fn update_lifecycle_rule_source(
        &self,
        actor: &User,
        rule_set_id: &str,
        draft: LifecycleSourceDraft,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.update_lifecycle_rule_source_with_builder_model(
            actor,
            rule_set_id,
            draft,
            BuilderModelChange::Keep,
        )
        .await
    }

    /// Append a revision as [`Self::update_lifecycle_rule_source`] does.
    /// [`BuilderModelChange::Keep`] carries the current revision's builder
    /// document onto the new one only when the source is unchanged; a changed
    /// source leaves the new revision without one.
    pub async fn update_lifecycle_rule_source_with_builder_model(
        &self,
        actor: &User,
        rule_set_id: &str,
        draft: LifecycleSourceDraft,
        builder_model: BuilderModelChange,
    ) -> AppResult<LifecycleRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_lifecycle_rule_set(rule_set_id).await?;
        let current_revision = self
            .services
            .customization
            .lifecycle_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "lifecycle rule set {} has no revision {}",
                    rule_set.id, rule_set.current_revision_number
                ))
            })?;
        let prepared = prepare_source(&rule_set.id, &draft.rego_source, draft.grace_days)?;
        let builder_model_json = resolved_builder_model(
            &builder_model.after_source_edit(&current_revision.rego_source, &prepared.rego_source),
            current_revision.builder_model_json.clone(),
            BuilderFamily::Lifecycle,
        )?;
        if rule_set.system_owned
            && (prepared.action_spec_json != current_revision.action_spec_json
                || scryer_rules::strip_editor_source(&prepared.rego_source)
                    != scryer_rules::strip_editor_source(&current_revision.rego_source))
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::SYSTEM_RULE_READ_ONLY,
                "This rule ships with Scryer: its matcher and actions cannot be changed. Its grace \
                 period, libraries and state can.",
            ));
        }
        // The maximum is judged against the new source together: a save may
        // add a delete head and its maximum at once. This early check reads the
        // row as it was; the store repeats it inside the write's transaction,
        // so a maximum cleared by another save in between is refused too. A
        // given maximum is always written, never dropped for matching a value
        // that may already be stale.
        let max_titles_per_run = draft.max_titles_per_run;
        require_max_titles_per_run(
            &prepared.declared,
            max_titles_per_run.unwrap_or(rule_set.max_titles_per_run),
        )?;
        self.require_declared_action_targets(
            &prepared.declared,
            &prepared.scan,
            &rule_set.library_ids,
        )
        .await?;
        self.require_person_fact_authority(actor, &prepared.rego_source, &rule_set.id)
            .await?;

        let now = Utc::now();
        let revision_number = rule_set.current_revision_number + 1;
        let revision = build_revision(
            &rule_set.id,
            revision_number,
            &prepared,
            draft.grace_days,
            Some(actor.id.clone()),
            now,
            builder_model_json,
        );

        let destructive = prepared.declared.destructive;
        let rules = &self.services.customization.lifecycle_rule_sets;
        if destructive {
            rules
                .add_revision(&revision, max_titles_per_run, now)
                .await?;
        } else {
            // An enabled rule stays enabled only while the new source still
            // declares an action; a rule this save read as not enabled is
            // never raised to enabled by it.
            let keep_enabled = rule_set.state == LifecycleRuleState::Enabled
                && !prepared.declared.heads.is_empty();
            rules
                .add_revision_keeping_state(&revision, keep_enabled, false, max_titles_per_run, now)
                .await?;
            // The store kept the lower of this state and whatever another
            // admin wrote since the read above; report what it holds.
            match rules.get_rule_set(&rule_set.id).await? {
                Some(fresh) => {
                    rule_set.state = fresh.state;
                    rule_set.edited_since_enabled = fresh.edited_since_enabled;
                    rule_set.destructive_confirmed_revision = fresh.destructive_confirmed_revision;
                }
                None => {
                    if !keep_enabled && rule_set.state == LifecycleRuleState::Enabled {
                        rule_set.state = LifecycleRuleState::DryRun;
                    }
                    rule_set.destructive_confirmed_revision = None;
                }
            }
        }

        rule_set.current_revision_number = revision_number;
        if let Some(max) = max_titles_per_run {
            rule_set.max_titles_per_run = max;
        }
        if destructive {
            // Mirrors what `add_revision` wrote in the same transaction, so the
            // detail this returns can never claim a state the store just
            // cleared.
            rule_set.disarm_after_edit();
        }
        rule_set.updated_at = now;
        Ok(LifecycleRuleSetDetail {
            action_refusal: action_refusal_for(&rule_set, &prepared.declared),
            rule_set,
            revision,
            declared_actions: prepared.declared,
            load_refusal: None,
        })
    }

    /// Renames and re-scopes without touching the matcher, action, or grace
    /// period, so no revision is created.
    ///
    /// A *scope* change drops an enabled rule to dry run and clears its
    /// deletion confirmation, atomically with the metadata write. Library
    /// scope is the rule's blast radius: an operator enabled one source over
    /// one set of libraries after acknowledging what that would reach, so
    /// widening the scope would put libraries nobody acknowledged inside an
    /// enabled rule, and narrowing it changes the acknowledged set just as
    /// materially. A scope change therefore invalidates the acknowledgement
    /// exactly like a new revision does, and the rule has to be enabled again
    /// on the new terms.
    ///
    /// A name or description edit changes neither what the rule matches, what it
    /// does, nor where it reaches, so it leaves the state alone. The comparison
    /// is set-based: reordering the same libraries is not a scope change.
    pub async fn update_lifecycle_rule_metadata(
        &self,
        actor: &User,
        rule_set_id: &str,
        name: String,
        description: String,
        library_ids: Vec<String>,
    ) -> AppResult<LifecycleRuleSet> {
        self.update_lifecycle_rule_metadata_with_schedule(
            actor,
            rule_set_id,
            name,
            description,
            library_ids,
            None,
            None,
        )
        .await
    }

    /// As [`Self::update_lifecycle_rule_metadata`], also replacing the
    /// rule's schedule when `schedule` is `Some`, and its per-fire maximum
    /// when `max_titles_per_run` is `Some` (`Some(None)` clears it). Both are
    /// limits on when and how much the rule acts, not on what it matches or
    /// does, so changing them leaves the state alone.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_lifecycle_rule_metadata_with_schedule(
        &self,
        actor: &User,
        rule_set_id: &str,
        name: String,
        description: String,
        library_ids: Vec<String>,
        schedule: Option<ScriptSchedule>,
        max_titles_per_run: Option<Option<u32>>,
    ) -> AppResult<LifecycleRuleSet> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let schedule = validated_rule_schedule(schedule)?;

        let mut rule_set = self.require_lifecycle_rule_set(rule_set_id).await?;
        let name = require_non_empty(&name, "name")?;
        if rule_set.system_owned && name != rule_set.name {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::SYSTEM_RULE_RENAME_REFUSED,
                "This rule ships with Scryer and cannot be renamed.",
            ));
        }
        // An early check against the revision as read; the store repeats it in
        // the write's transaction against the revision then in force.
        if let Some(max) = max_titles_per_run {
            let declared = self
                .load_lifecycle_rule_detail(rule_set.clone())
                .await?
                .declared_actions;
            require_max_titles_per_run(&declared, max)?;
        }
        let now = Utc::now();
        let scope_changed = library_scope_changed(&rule_set.library_ids, &library_ids);
        if scope_changed {
            self.require_move_targets_for_scope(&rule_set, &library_ids)
                .await?;
        }

        self.services
            .customization
            .lifecycle_rule_sets
            .update_rule_set_metadata(
                &rule_set.id,
                &name,
                &description,
                &library_ids,
                schedule.as_ref(),
                max_titles_per_run,
                scope_changed,
                now,
            )
            .await?;
        if let Some(max) = max_titles_per_run {
            rule_set.max_titles_per_run = max;
        }

        let renamed = rule_set.name != name;
        rule_set.name = name;
        rule_set.description = description;
        rule_set.library_ids = library_ids;
        let rescheduled = schedule
            .as_ref()
            .is_some_and(|schedule| *schedule != rule_set.schedule);
        if let Some(schedule) = schedule {
            rule_set.schedule = schedule;
        }
        if scope_changed {
            // Mirrors what the store wrote in the same statement, so the rule
            // this returns can never claim a state the store just cleared.
            rule_set.disarm_after_edit();
        }
        rule_set.updated_at = now;
        if rescheduled || renamed {
            self.reload_lifecycle_rule_schedule().await;
        }
        Ok(rule_set)
    }

    /// When a rule next runs on its schedule, read from the in-memory
    /// scheduler; no store read.
    pub fn lifecycle_rule_next_run_at(&self, rule_set_id: &str) -> Option<chrono::DateTime<Utc>> {
        self.runtime
            .jobs
            .custom_job_scheduler
            .lifecycle_rule_next_run_at(rule_set_id)
    }

    /// Bring the in-memory schedule up to date after a rule's schedule,
    /// name or state changed. A failed reload is retried by the scheduler,
    /// so it never fails the edit that caused it.
    pub(crate) async fn reload_lifecycle_rule_schedule(&self) {
        if let Err(error) = self.reload_custom_job_schedule().await {
            tracing::warn!(error = %error, "lifecycle rule schedule reload failed; the scheduler retries it");
        }
    }

    /// Delete a rule set that never did anything, and refuse one that did.
    ///
    /// The rule set's tables cascade: deleting the row takes its revisions, its
    /// candidates, *and* its action runs with it. For a rule that has executed,
    /// those action runs are the only record that the deletion, unmonitor, or
    /// profile change ever happened and who authorized it — so deleting the rule
    /// would erase the evidence of what it did. That is refused outright rather
    /// than archived, because there is no state in which erasing an executed
    /// action's audit trail is the right answer to "I am done with this rule":
    /// disabling the rule stops it just as completely and keeps the history.
    ///
    /// Non-terminal candidates are refused for the adjacent reason: they are
    /// live membership the rule is still the authorization for, and cascading
    /// them away silently drops subjects an operator can currently see.
    pub async fn delete_lifecycle_rule_set(
        &self,
        actor: &User,
        rule_set_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let rule_set = self.require_lifecycle_rule_set(rule_set_id).await?;
        if rule_set.system_owned {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::SYSTEM_RULE_DELETE_REFUSED,
                "This rule ships with Scryer and cannot be deleted. Set it to Disabled instead.",
            ));
        }

        // Bounded existence reads: one row is enough to answer both questions.
        let has_history = !self
            .services
            .customization
            .lifecycle_evaluation
            .list_action_runs(Some(&rule_set.id), None, Some(1))
            .await?
            .is_empty();
        if has_history {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_DELETE_HAS_ACTION_HISTORY,
                format!(
                    "\"{}\" has already run actions on your library, and deleting it would erase the \
                 record of what it did. Set it to Disabled instead: the rule stops \
                 immediately and its history is kept.",
                    rule_set.name
                ),
            ));
        }

        let active_candidates = self.count_active_lifecycle_candidates(&rule_set.id).await?;
        if active_candidates > 0 {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_DELETE_HAS_ACTIVE_CANDIDATES,
                format!(
                    "\"{}\" is currently tracking {active_candidates} title(s), and deleting it would \
                 drop them without a trace. Set it to Disabled instead, or clear the \
                 candidates first.",
                    rule_set.name
                ),
            ));
        }

        self.services
            .customization
            .lifecycle_rule_sets
            .delete_rule_set_if_unused(rule_set_id)
            .await?;
        if rule_set.evaluates() {
            self.reload_lifecycle_rule_schedule().await;
        }
        Ok(())
    }

    /// Create the rule every instance ships with: titles whose request lease
    /// expired are deleted with their files after a seven-day grace period.
    /// It starts Disabled, like every other rule: an operator enables it on
    /// purpose. Startup calls this on every boot, so once the rule exists it
    /// only brings a previously shipped matcher up to date.
    pub async fn ensure_expired_request_leases_rule(&self) -> AppResult<()> {
        let rules = &self.services.customization.lifecycle_rule_sets;
        if let Some(existing) = rules
            .list_rule_sets()
            .await?
            .into_iter()
            .find(|rule| rule.system_owned && rule.name == EXPIRED_REQUEST_LEASES_RULE_NAME)
        {
            return self.upgrade_expired_request_leases_rule(existing).await;
        }
        let id = Id::new_rego_safe().0;
        let grace_days = 7;
        let prepared = prepare_source(&id, EXPIRED_REQUEST_LEASES_REGO, grace_days)?;
        let now = Utc::now();
        let revision = build_revision(&id, 1, &prepared, grace_days, None, now, None);
        let rule_set = LifecycleRuleSet {
            id,
            name: EXPIRED_REQUEST_LEASES_RULE_NAME.into(),
            description: EXPIRED_REQUEST_LEASES_DESCRIPTION.into(),
            state: LifecycleRuleState::Disabled,
            edited_since_enabled: false,
            destructive_confirmed_revision: None,
            library_ids: Vec::new(),
            system_owned: true,
            current_revision_number: 1,
            schedule: scryer_domain::default_lifecycle_rule_schedule(),
            max_titles_per_run: Some(scryer_domain::SYSTEM_LIFECYCLE_RULE_MAX_TITLES_PER_RUN),
            created_at: now,
            updated_at: now,
        };
        rules.create_rule_set(&rule_set, &revision).await
    }

    /// Bring the shipped rule onto the current source, keeping its identity,
    /// state, grace period and libraries. The rule's matcher
    /// and action could never be edited, so a stored source is one of the
    /// sources the rule shipped with:
    ///
    /// - the current one: left alone;
    /// - the `match` form, whose separate action deleted the title and its
    ///   files: replaced, keeping the rule's state and moving its deletion
    ///   confirmation onto the new revision, because the new source deletes
    ///   exactly the same titles the operator confirmed it for;
    /// - the keep-claim matcher: replaced and disarmed, because it also
    ///   spared kept titles, so the operator enables it again against the new
    ///   one;
    /// - anything else: replaced and disarmed, the same conservative way.
    async fn upgrade_expired_request_leases_rule(
        &self,
        rule_set: LifecycleRuleSet,
    ) -> AppResult<()> {
        let rules = &self.services.customization.lifecycle_rule_sets;
        let Some(current) = rules
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?
        else {
            return Ok(());
        };
        let stored = scryer_rules::strip_editor_source(&current.rego_source);
        let is_stored = |source: &str| {
            stored
                == scryer_rules::strip_editor_source(&rewrite_package_declaration(
                    source,
                    &rule_set.id,
                ))
        };
        if is_stored(EXPIRED_REQUEST_LEASES_REGO) {
            return Ok(());
        }
        let keeps_arming = is_stored(MATCH_FORM_EXPIRED_REQUEST_LEASES_REGO);

        let prepared = prepare_source(
            &rule_set.id,
            EXPIRED_REQUEST_LEASES_REGO,
            current.grace_days,
        )?;
        let now = Utc::now();
        let revision = build_revision(
            &rule_set.id,
            rule_set.current_revision_number + 1,
            &prepared,
            current.grace_days,
            None,
            now,
            // The shipped source is replaced, so no stored builder document
            // describes it any more.
            None,
        );
        if keeps_arming {
            let keep_enabled = rule_set.state == LifecycleRuleState::Enabled
                && !prepared.declared.heads.is_empty();
            // The confirmation moves only onto a source that still deletes.
            let carry_confirmation = keep_enabled && prepared.declared.destructive;
            rules
                .add_revision_keeping_state(&revision, keep_enabled, carry_confirmation, None, now)
                .await?;
        } else {
            rules.add_revision(&revision, None, now).await?;
        }

        if rule_set.description == PREVIOUS_EXPIRED_REQUEST_LEASES_DESCRIPTION {
            rules
                .update_rule_set_metadata(
                    &rule_set.id,
                    &rule_set.name,
                    EXPIRED_REQUEST_LEASES_DESCRIPTION,
                    &rule_set.library_ids,
                    None,
                    None,
                    false,
                    now,
                )
                .await?;
        }
        Ok(())
    }

    /// Editor support: compile and check a draft without storing it. The
    /// throwaway id only satisfies the package-declaration check.
    pub async fn validate_lifecycle_rule_source(
        &self,
        actor: &User,
        rego_source: &str,
    ) -> AppResult<LifecycleRuleSourceValidation> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let scratch_id = Id::new_rego_safe().0;
        let rewritten = rewrite_package_declaration(rego_source, &scratch_id);
        let result = validate_lifecycle_rule(&rewritten, &scratch_id).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {e}"),
            )
        })?;
        // A source that does not parse declares nothing; one that parses
        // declares its heads even while some other check refuses it, so the
        // editor can show what the rule would do while the author fixes it.
        let declared_actions = scan_lifecycle_action_heads(&rewritten, &scratch_id)
            .ok()
            .map(|scan| LifecycleDeclaredActions::from_scan(&scan));
        Ok(LifecycleRuleSourceValidation {
            result,
            declared_actions,
        })
    }

    /// Evaluate one matcher against a bounded title selection. Nothing is
    /// written: preview answers "what would this match", not "what happened".
    pub async fn preview_lifecycle_rule(
        &self,
        actor: &User,
        request: LifecyclePreviewRequest,
    ) -> AppResult<LifecyclePreviewResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let (policy, declared, library_ids, grace_days) =
            self.resolve_preview_policy(request.source).await?;
        // Preview evaluates against real subjects and reports per-title
        // outcomes, so an ungated one is a person-fact oracle: ask "did user x
        // watch this" title by title and read the answers off the results. The
        // same bar authoring meets applies here, for a stored rule as much as
        // for a draft.
        self.require_person_fact_authority(actor, &policy.rego_source, &policy.id)
            .await?;
        // The per-subject fact document names who watched what and who asked
        // for it, whatever the rule reads, so only a caller who may author
        // person-targeted rules is shown it.
        let facts_visible = self
            .has_app_permission(actor, AppPermission::ManagePermissions)
            .await?;

        let mut titles = self.select_preview_titles(&request.selection).await?;
        titles.retain(|title| library_ids.is_empty() || library_ids.contains(&title.library_id));
        let (result_limit, selected_subjects) = match &request.selection {
            LifecyclePreviewSelection::Library { limit, .. } => (
                limit
                    .unwrap_or(LIFECYCLE_PREVIEW_DEFAULT_TITLES)
                    .clamp(1, LIFECYCLE_PREVIEW_MAX_TITLES),
                None,
            ),
            LifecyclePreviewSelection::Subjects { subject_ids, .. } => {
                if subject_ids.is_empty() || subject_ids.len() > LIFECYCLE_PREVIEW_MAX_TITLES {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::lifecycle::PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE,
                        "preview requires between one and fifty subjects",
                    ));
                }
                (LIFECYCLE_PREVIEW_MAX_TITLES, Some(subject_ids))
            }
            LifecyclePreviewSelection::Titles(_) => (LIFECYCLE_PREVIEW_MAX_TITLES, None),
        };
        let libraries = self.lifecycle_library_refs().await?;
        let exclusions = self
            .services
            .customization
            .lifecycle_evaluation
            .list_exclusions(Some(&policy.id))
            .await?;

        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        // One batched load for the whole selection; a per-title query here
        // would make the preview cost scale with the cap.
        let mut files_by_title: HashMap<String, Vec<crate::types::TitleMediaFile>> = HashMap::new();
        if !title_ids.is_empty() {
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
        }
        // Show inventory includes full episode/file ownership and watch
        // coverage. Keep one immutable snapshot per selected title so every
        // scoped subject in this preview derives from the same observation.
        // The shows are read together; one the batch could not build is read
        // on its own, which fails the preview exactly as it always did.
        let mut episode_inventories_by_title = self
            .lifecycle_load_title_episode_inventories(&titles, None)
            .await;
        for title in &titles {
            if title.facet != MediaFacet::Movie
                && !episode_inventories_by_title.contains_key(&title.id)
            {
                episode_inventories_by_title.insert(
                    title.id.clone(),
                    self.lifecycle_load_title_episode_inventory(title).await?,
                );
            }
        }
        // Preview must see exactly what the evaluator sees, so the same two
        // batched people lookups run here, once for the whole selection, and so
        // does the series-movie load.
        let series_movies_by_title = self.lifecycle_series_movies_for_titles(&titles).await?;
        let requesters_by_title = self.lifecycle_requesters_for_titles(&titles).await?;
        let usernames = self.lifecycle_usernames_by_id().await?;
        // Watch signals go through the same run-scoped gate the evaluator uses,
        // so a preview cannot show a match the scheduled pass would hold.
        let watch_context = self.lifecycle_watch_context().await?;
        let signals_by_title = self
            .lifecycle_watch_signals_for_titles(&watch_context, &titles)
            .await?;
        let (watch_context, audience_by_title) = self
            .lifecycle_audience_plays_for_titles(&watch_context, &titles)
            .await;
        // Claims are read exactly as the pass reads them, including what
        // happens when the store is down: preview degrades to unknown claim
        // facts rather than erroring, so an operator previewing a lease rule
        // during an outage sees the same hold the scheduled pass would produce.
        let claims_by_title = match self.lifecycle_claims_for_titles(&titles).await {
            Ok(claims) => Some(claims),
            Err(error) => {
                tracing::warn!(
                    rule_set_id = policy.id.as_str(),
                    error = %error,
                    "could not read lifecycle claims; preview reports claim facts as unknown"
                );
                None
            }
        };
        let lists_by_title = self
            .lifecycle_list_facts_loading(
                &titles,
                super::list_facts::LifecycleFactLoads::for_rule(&policy.rego_source, &policy.id),
                &watch_context,
                Utc::now(),
            )
            .await;

        let source_content_hash = content_hash(&policy.rego_source);
        let rule_set_id = policy.id.clone();
        let engine = LifecycleRulesEngine::build(std::slice::from_ref(&policy)).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_COMPILE_FAILED,
                format!("rule failed to compile: {e}"),
            )
        })?;
        let mut evaluator = engine.evaluator();
        // The levels the rule acts at, read off its heads, exactly as the
        // scheduled pass reads them.
        let levels = declared.levels();
        let narrow_levels: Vec<_> = levels
            .iter()
            .copied()
            .filter(|level| *level != scryer_rules::lifecycle::LifecycleSubjectKind::Title)
            .collect();
        let evaluates_titles =
            levels.contains(&scryer_rules::lifecycle::LifecycleSubjectKind::Title);
        // One read of the configured roots and at most one capacity read per
        // root for the whole preview.
        let mut storage = super::evaluation::LifecyclePassStorage::default();

        let evaluated_at = Utc::now();
        let mut results = Vec::with_capacity(titles.len());
        for title in titles {
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
            let series_movies = series_movies_by_title
                .get(&title.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut subjects = Vec::new();
            if title.facet == MediaFacet::Movie {
                if evaluates_titles {
                    subjects = self
                        .lifecycle_subjects_for_title(
                            LifecycleRuleSubjectKind::Title,
                            &title,
                            files,
                        )
                        .await?;
                }
            } else {
                let inventory = episode_inventories_by_title.get(&title.id).ok_or_else(|| {
                    AppError::Repository("lifecycle preview lost title episode inventory".into())
                })?;
                if evaluates_titles || !narrow_levels.is_empty() {
                    subjects.extend(self.lifecycle_subjects_from_episode_inventory(
                        LifecycleRuleSubjectKind::Title,
                        &title,
                        files,
                        inventory,
                    )?);
                }
                for level in &narrow_levels {
                    subjects.extend(self.lifecycle_subjects_from_episode_inventory(
                        super::evaluation::level_scope(*level),
                        &title,
                        files,
                        inventory,
                    )?);
                }
            }
            let build_input = |subject: &super::subjects::LifecycleSubject| {
                let mut input = build_subject_input(
                    evaluated_at,
                    &title,
                    &library,
                    subject,
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
                input
            };
            // The title's own input, beside which its seasons and episodes
            // are evaluated.
            let mut title_input = None;
            if !narrow_levels.is_empty()
                && let Some(title_subject) = subjects
                    .iter()
                    .find(|subject| subject.kind == LifecycleRuleSubjectKind::Title)
            {
                let mut input = build_input(title_subject);
                self.lifecycle_pass_storage_facts(
                    &mut storage,
                    &mut input.facts,
                    &title_subject.files,
                    crate::lifecycle_rules::storage::LifecycleStorageRead::Refresh,
                )
                .await;
                title_input = Some(input);
            }
            for subject in subjects {
                if results.len() >= result_limit {
                    break;
                }
                if subject.kind == LifecycleRuleSubjectKind::Title && !evaluates_titles {
                    continue;
                }
                if selected_subjects.is_some_and(|ids| !ids.contains(&subject.id)) {
                    continue;
                }
                let mut input = build_input(&subject);
                self.lifecycle_pass_storage_facts(
                    &mut storage,
                    &mut input.facts,
                    &subject.files,
                    crate::lifecycle_rules::storage::LifecycleStorageRead::Refresh,
                )
                .await;
                if let Some(title_input) = title_input.as_ref()
                    && subject.kind != LifecycleRuleSubjectKind::Title
                {
                    input = super::facts::nest_subject_input(title_input.clone(), input, &subject);
                }

                let evaluation = evaluator.evaluate(&input).map_err(|e| {
                    AppError::validation_refused(
                        crate::error_reasons::lifecycle::RULE_EVALUATION_FAILED,
                        format!("rule evaluation failed: {e}"),
                    )
                })?;

                let (outcome, reason_codes, error, actions) =
                    match (evaluation.records.first(), evaluation.errors.first()) {
                        (Some(record), _) => (
                            Some(record.decision.outcome),
                            record.decision.reason_codes.clone(),
                            None,
                            (record.decision.outcome == LifecycleOutcome::Match)
                                .then(|| record.decision.actions.clone()),
                        ),
                        (None, Some(failure)) => (
                            None,
                            Vec::new(),
                            Some((failure.code, failure.message.clone())),
                            None,
                        ),
                        (None, None) => (
                            None,
                            Vec::new(),
                            Some((
                                crate::error_reasons::lifecycle::PREVIEW_NO_DECISION,
                                "rule produced no decision".to_string(),
                            )),
                            None,
                        ),
                    };
                let (error_code, error) = error.unzip();

                let facts = if facts_visible {
                    serde_json::to_value(&input)
                        .inspect_err(|error| {
                            tracing::warn!(
                                rule_set_id = policy.id.as_str(),
                                subject_id = subject.id.as_str(),
                                error = %error,
                                "could not render the preview fact document"
                            );
                        })
                        .ok()
                } else {
                    None
                };
                let excluded = subject.excluded_by(&title.id, &exclusions);
                // A match is first given the estimate a new candidate would
                // get; the open candidates are read once for the whole page
                // below and replace it with their real due time.
                let due_at = (!excluded && outcome == Some(LifecycleOutcome::Match))
                    .then(|| evaluated_at + chrono::Duration::days(grace_days));
                results.push(LifecyclePreviewTitleResult {
                    excluded,
                    due_at,
                    due_at_is_estimate: due_at.is_some(),
                    subject_kind: subject.kind,
                    subject_id: subject.id,
                    subject_label: subject.label,
                    file_count: subject.files.len() as i64,
                    total_size_bytes: subject.files.iter().map(|file| file.size_bytes).sum(),
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    library_id: title.library_id.clone(),
                    outcome,
                    reason_codes,
                    error,
                    error_code,
                    actions,
                    facts,
                });
            }
        }

        // One read for the open candidates of every matched subject on this
        // page; their due time replaces the estimate. A candidate opened by
        // another revision of the source decides nothing here.
        // One read per subject level present on the page, at most three.
        let mut matched_by_kind: Vec<(LifecycleRuleSubjectKind, Vec<String>)> = Vec::new();
        for result in results.iter().filter(|result| result.due_at.is_some()) {
            match matched_by_kind
                .iter_mut()
                .find(|(kind, _)| *kind == result.subject_kind)
            {
                Some((_, ids)) => ids.push(result.subject_id.clone()),
                None => {
                    matched_by_kind.push((result.subject_kind, vec![result.subject_id.clone()]))
                }
            }
        }
        let mut open: HashMap<(&'static str, String), chrono::DateTime<Utc>> = HashMap::new();
        for (kind, matched_ids) in &matched_by_kind {
            for candidate in self
                .services
                .customization
                .lifecycle_evaluation
                .list_active_subject_candidates(&rule_set_id, kind.as_storage_str(), matched_ids)
                .await?
                .into_iter()
                .filter(|candidate| candidate.source_content_hash == source_content_hash)
            {
                open.insert(
                    (kind.as_storage_str(), candidate.subject_id),
                    candidate.due_at,
                );
            }
        }
        if !open.is_empty() {
            for result in &mut results {
                if result.due_at.is_some()
                    && let Some(due_at) = open.get(&(
                        result.subject_kind.as_storage_str(),
                        result.subject_id.clone(),
                    ))
                {
                    result.due_at = Some(*due_at);
                    result.due_at_is_estimate = false;
                }
            }
        }

        if selected_subjects.is_some_and(|ids| {
            ids.iter()
                .any(|id| !results.iter().any(|result| &result.subject_id == id))
        }) {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::PREVIEW_SUBJECT_OUTSIDE_LIBRARIES,
                "preview subject is missing, outside the rule libraries, or does not belong to the selected titles and scope",
            ));
        }
        Ok(LifecyclePreviewResult {
            rule_set_id,
            source_content_hash,
            evaluated_at,
            titles: results,
        })
    }

    /// Gate authoring — or previewing — a matcher that reads person-targeted
    /// facts.
    ///
    /// Catalog-settings management is enough to write a rule about *media*.
    /// Writing one about *people* — who added a title, who asked for it, who
    /// watched it — is a use of the instance's identity records, so it asks for
    /// the same authority that hands out permissions in the first place.
    /// Preview meets the same bar because it answers the same question against
    /// real subjects, one title at a time.
    ///
    /// The bar is on reaching the facts, not on running: a stored revision
    /// keeps evaluating under the system principal, because revoking an
    /// author's permission must not silently stop a rule an operator armed.
    ///
    /// The fact set comes from the same static extraction the engine holds
    /// rules on, so a rule cannot reach a person fact by a path this check
    /// cannot see — the source that would allow it does not validate.
    async fn require_person_fact_authority(
        &self,
        actor: &User,
        rego_source: &str,
        rule_set_id: &str,
    ) -> AppResult<()> {
        let referenced = lifecycle_referenced_facts(rego_source, rule_set_id).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::lifecycle::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {e}"),
            )
        })?;
        let person_facts: Vec<&str> = PERSON_TARGETED_LIFECYCLE_FACTS
            .into_iter()
            .filter(|fact| referenced.contains(*fact))
            .collect();
        if person_facts.is_empty() {
            return Ok(());
        }

        if self
            .has_app_permission(actor, AppPermission::ManagePermissions)
            .await?
        {
            return Ok(());
        }

        Err(AppError::unauthorized_reason(
            crate::error_reasons::lifecycle::PERSON_TARGETING_PERMISSION_REQUIRED,
            format!(
                "This rule reads facts about specific people ({}), so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
                person_facts.join(", ")
            ),
        ))
    }

    pub(crate) async fn require_lifecycle_rule_set(&self, id: &str) -> AppResult<LifecycleRuleSet> {
        self.services
            .customization
            .lifecycle_rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::lifecycle::RULE_SET_NOT_FOUND,
                    format!("lifecycle rule set {id} not found"),
                )
            })
    }

    pub(crate) async fn load_lifecycle_rule_detail(
        &self,
        rule_set: LifecycleRuleSet,
    ) -> AppResult<LifecycleRuleSetDetail> {
        let revision = self
            .services
            .customization
            .lifecycle_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?;
        lifecycle_rule_detail(rule_set, revision)
    }

    async fn resolve_preview_policy(
        &self,
        source: LifecyclePreviewSource,
    ) -> AppResult<(LifecyclePolicy, LifecycleDeclaredActions, Vec<String>, i64)> {
        match source {
            LifecyclePreviewSource::Stored { rule_set_id } => {
                let rule_set = self.require_lifecycle_rule_set(&rule_set_id).await?;
                let detail = self.load_lifecycle_rule_detail(rule_set).await?;
                if let Some(reason) = detail.load_refusal {
                    return Err(stored_actions_refused(reason));
                }
                Ok((
                    LifecyclePolicy {
                        id: detail.rule_set.id,
                        name: detail.rule_set.name,
                        rego_source: detail.revision.rego_source,
                    },
                    detail.declared_actions,
                    detail.rule_set.library_ids,
                    detail.revision.grace_days,
                ))
            }
            LifecyclePreviewSource::Inline {
                library_ids,
                rego_source,
                grace_days,
            } => {
                let scratch_id = Id::new_rego_safe().0;
                let prepared = prepare_source(&scratch_id, &rego_source, grace_days)?;
                Ok((
                    LifecyclePolicy {
                        id: scratch_id,
                        name: "preview".to_string(),
                        rego_source: prepared.rego_source,
                    },
                    prepared.declared,
                    library_ids,
                    grace_days,
                ))
            }
        }
    }

    async fn select_preview_titles(
        &self,
        selection: &LifecyclePreviewSelection,
    ) -> AppResult<Vec<Title>> {
        match selection {
            LifecyclePreviewSelection::Titles(title_ids)
            | LifecyclePreviewSelection::Subjects { title_ids, .. } => {
                if title_ids.len() > LIFECYCLE_PREVIEW_MAX_TITLES {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::lifecycle::PREVIEW_TITLE_LIMIT_EXCEEDED,
                        format!(
                            "preview accepts at most {LIFECYCLE_PREVIEW_MAX_TITLES} titles, {} were requested",
                            title_ids.len()
                        ),
                    ));
                }
                self.services.catalog.titles.get_by_ids(title_ids).await
            }
            LifecyclePreviewSelection::Library { library_id, limit } => {
                let limit = limit
                    .unwrap_or(LIFECYCLE_PREVIEW_DEFAULT_TITLES)
                    .clamp(1, LIFECYCLE_PREVIEW_MAX_TITLES);
                let mut titles = Vec::new();
                let mut after_id = None;
                loop {
                    let page = self
                        .services
                        .catalog
                        .titles
                        .list_page_after_id(after_id, LIFECYCLE_PREVIEW_MAX_TITLES)
                        .await?;
                    if page.is_empty() {
                        break;
                    }
                    after_id = page.last().map(|title| title.id.clone());
                    titles.extend(
                        page.into_iter()
                            .filter(|title| &title.library_id == library_id)
                            .take(limit - titles.len()),
                    );
                    if titles.len() >= limit {
                        break;
                    }
                }
                Ok(titles)
            }
        }
    }

    /// Every library, resolved once per preview or evaluation pass.
    pub(crate) async fn lifecycle_library_refs(
        &self,
    ) -> AppResult<HashMap<String, LifecycleLibraryRef>> {
        Ok(self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .map(|library| {
                (
                    library.id.clone(),
                    LifecycleLibraryRef {
                        id: library.id,
                        name: library.name,
                    },
                )
            })
            .collect())
    }
}

impl AppUseCase {
    /// The live half of declared-action validation.
    ///
    /// Whether a tag label is registered and whether a quality profile name
    /// exists are live questions, so they are checked here when a revision is
    /// authored; the executor repeats both before it changes a title. Each is
    /// one read for every literal in the source.
    async fn require_declared_action_targets(
        &self,
        declared: &LifecycleDeclaredActions,
        scan: &scryer_rules::lifecycle_actions::LifecycleActionScan,
        library_ids: &[String],
    ) -> AppResult<()> {
        let labels: Vec<String> = declared
            .add_tags
            .iter()
            .chain(declared.remove_tags.iter())
            .cloned()
            .collect();
        if !labels.is_empty() {
            self.require_registered_title_tags(&labels).await?;
        }
        self.require_declared_move_targets(scan, library_ids)
            .await?;
        if declared.quality_profiles.is_empty() {
            return Ok(());
        }
        let profiles = self.lifecycle_quality_profile_ids_by_name().await?;
        let diagnostics: Vec<ValidationDiagnostic> = scan
            .quality_profiles
            .iter()
            .filter(|literal| !profiles.contains_key(&literal.value))
            .map(|literal| {
                let mut diagnostic = ValidationDiagnostic::new(
                    crate::error_reasons::lifecycle::QUALITY_PROFILE_NOT_FOUND,
                    format!(
                        "set_quality_profile names \"{}\", which is not a quality profile",
                        literal.value
                    ),
                );
                diagnostic.line = Some(literal.line);
                diagnostic.column = Some(literal.column);
                diagnostic
            })
            .collect();
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(AppError::validation_refused_with_diagnostics(
            crate::error_reasons::lifecycle::QUALITY_PROFILE_NOT_FOUND,
            "The rule names a quality profile that does not exist.",
            diagnostics,
        ))
    }

    /// A new library scope is checked against the move targets the current
    /// revision names, with the same codes a source save uses: the scope
    /// decides which kind of library a `move_to_library` may name. One
    /// revision read; the library read happens only for a source that moves.
    /// A stored source the scan cannot read is left to the executor, which
    /// holds a move it cannot resolve.
    async fn require_move_targets_for_scope(
        &self,
        rule_set: &LifecycleRuleSet,
        library_ids: &[String],
    ) -> AppResult<()> {
        let Some(revision) = self
            .services
            .customization
            .lifecycle_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?
        else {
            return Ok(());
        };
        let Ok(scan) = scan_lifecycle_action_heads(&revision.rego_source, &rule_set.id) else {
            return Ok(());
        };
        self.require_declared_move_targets(&scan, library_ids).await
    }

    /// The move targets a source names must exist when it is saved: each
    /// `move_to_library` name is exactly one library, of the media kind of
    /// every library the rule is confined to, and each `move_to_root` path is
    /// a configured root. One library read, only for a source that moves.
    /// The executor checks again against the title it moves.
    async fn require_declared_move_targets(
        &self,
        scan: &scryer_rules::lifecycle_actions::LifecycleActionScan,
        library_ids: &[String],
    ) -> AppResult<()> {
        use crate::error_reasons::lifecycle as reasons;
        if scan.libraries.is_empty() && scan.roots.is_empty() {
            return Ok(());
        }
        let libraries = self.services.catalog.libraries.list(None).await?;
        let scope_facets: std::collections::HashSet<&scryer_domain::MediaFacet> = libraries
            .iter()
            .filter(|library| library_ids.contains(&library.id))
            .map(|library| &library.facet)
            .collect();
        let mut diagnostics: Vec<(&'static str, ValidationDiagnostic)> = Vec::new();
        let mut refuse =
            |code: &'static str,
             message: String,
             literal: &scryer_rules::lifecycle_actions::LifecycleActionLiteral| {
                let mut diagnostic = ValidationDiagnostic::new(code, message);
                diagnostic.line = Some(literal.line);
                diagnostic.column = Some(literal.column);
                diagnostics.push((code, diagnostic));
            };
        for literal in &scan.libraries {
            let named: Vec<_> = libraries
                .iter()
                .filter(|library| library.name == literal.value)
                .collect();
            match named.as_slice() {
                [library] => {
                    if scope_facets.iter().any(|facet| **facet != library.facet) {
                        refuse(
                            reasons::MOVE_LIBRARY_FACET_MISMATCH,
                            format!(
                                "move_to_library names \"{}\", which holds another kind of media \
                                 than the rule's libraries",
                                literal.value
                            ),
                            literal,
                        );
                    }
                }
                _ => refuse(
                    reasons::MOVE_LIBRARY_NOT_FOUND,
                    format!(
                        "move_to_library names \"{}\", which is not exactly one library",
                        literal.value
                    ),
                    literal,
                ),
            }
        }
        for literal in &scan.roots {
            let wanted = super::title_actions::comparable_root_path(&literal.value);
            let configured = libraries.iter().any(|library| {
                library
                    .roots
                    .iter()
                    .any(|root| super::title_actions::comparable_root_path(&root.path) == wanted)
            });
            if !configured {
                refuse(
                    reasons::MOVE_ROOT_NOT_FOUND,
                    format!(
                        "move_to_root names \"{}\", which is not a configured library root",
                        literal.value
                    ),
                    literal,
                );
            }
        }
        let Some((code, _)) = diagnostics.first() else {
            return Ok(());
        };
        let code = *code;
        Err(AppError::validation_refused_with_diagnostics(
            code,
            "The rule names a move target that does not exist.",
            diagnostics
                .into_iter()
                .map(|(_, diagnostic)| diagnostic)
                .collect(),
        ))
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

/// A matcher that passed every authoring-time check, ready to persist.
struct PreparedSource {
    rego_source: String,
    content_hash: String,
    action_spec_json: String,
    declared: LifecycleDeclaredActions,
    scan: scryer_rules::lifecycle_actions::LifecycleActionScan,
}

/// Rewrite the package to the system-assigned id, then run every check that
/// must hold before a revision is written: the matcher compiles, reads only
/// documented facts and defines at least one action head, and the grace
/// period is a real duration. Where each head acts is its name.
fn prepare_source(rule_id: &str, rego_source: &str, grace_days: i64) -> AppResult<PreparedSource> {
    if grace_days < 0 {
        return Err(AppError::validation_refused(
            crate::error_reasons::lifecycle::GRACE_PERIOD_INVALID,
            "grace period must be zero or more days".to_string(),
        ));
    }
    if grace_days > LIFECYCLE_MAX_GRACE_DAYS {
        return Err(AppError::validation_refused(
            crate::error_reasons::lifecycle::GRACE_PERIOD_OUT_OF_RANGE,
            format!("grace period must be at most {LIFECYCLE_MAX_GRACE_DAYS} days (ten years)"),
        ));
    }

    let rewritten = rewrite_package_declaration(rego_source, rule_id);
    let validation = validate_lifecycle_rule(&rewritten, rule_id).map_err(|e| {
        AppError::validation_refused(
            crate::error_reasons::lifecycle::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {e}"),
        )
    })?;
    if !validation.valid {
        let reason = if validation.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == scryer_rules::diagnostic_codes::RULE_ACTION_REQUIRED
        }) {
            crate::error_reasons::lifecycle::ACTION_HEAD_REQUIRED
        } else {
            crate::error_reasons::lifecycle::RULE_SOURCE_INVALID
        };
        return Err(AppError::validation_refused_with_diagnostics(
            reason,
            format_lifecycle_validation_errors(&validation),
            validation.diagnostics,
        ));
    }
    let scan = scan_lifecycle_action_heads(&rewritten, rule_id).map_err(|e| {
        AppError::validation_refused(
            crate::error_reasons::lifecycle::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {e}"),
        )
    })?;

    let declared = LifecycleDeclaredActions::from_scan(&scan);
    let action_spec_json = declared.to_persisted_json();

    Ok(PreparedSource {
        content_hash: content_hash(&rewritten),
        rego_source: rewritten,
        action_spec_json,
        declared,
        scan,
    })
}

/// Whether two library scopes describe different sets of libraries.
///
/// Compared as sets, not as sequences: the stored order is an artifact of how a
/// client serialized the list, and re-saving the same libraries in a different
/// order is not a change to what the rule can reach.
fn library_scope_changed(stored: &[String], replacement: &[String]) -> bool {
    let stored: std::collections::BTreeSet<&str> = stored.iter().map(String::as_str).collect();
    let replacement: std::collections::BTreeSet<&str> =
        replacement.iter().map(String::as_str).collect();
    stored != replacement
}

/// Normalize and validate a rule schedule exactly as a scheduled script's
/// is, refusing with `SCHEDULE_INVALID`.
fn validated_rule_schedule(schedule: Option<ScriptSchedule>) -> AppResult<Option<ScriptSchedule>> {
    let Some(mut schedule) = schedule else {
        return Ok(None);
    };
    if let ScriptSchedule::Weekly { days, .. } = &mut schedule {
        days.sort();
        days.dedup();
    }
    crate::scripts::schedule::normalize_schedule_time(&mut schedule)
        .map_err(crate::scripts::schedule::ScheduleError::into_app_error)?;
    crate::scripts::schedule::validate_schedule(&schedule)
        .map_err(crate::scripts::schedule::ScheduleError::into_app_error)?;
    Ok(Some(schedule))
}

/// A rule's per-fire maximum is at least one title, and a rule that can
/// delete must carry one. There is no upper bound: the operator picks it.
pub fn require_max_titles_per_run(
    declared: &super::LifecycleDeclaredActions,
    max_titles_per_run: Option<u32>,
) -> AppResult<()> {
    match max_titles_per_run {
        Some(0) => Err(AppError::validation_refused(
            crate::error_reasons::lifecycle::RULE_MAX_TITLES_PER_RUN_INVALID,
            "the most titles a rule acts on in one run must be at least 1",
        )),
        None if declared.destructive => Err(AppError::validation_refused_with_facts(
            crate::error_reasons::lifecycle::RULE_MAX_TITLES_PER_RUN_REQUIRED,
            "a rule that deletes must say the most titles it may act on in one run",
            vec![crate::RefusalFact::list(
                crate::error_reasons::lifecycle::RULE_MAX_TITLES_PER_RUN_DESTRUCTIVE_ACTIONS_FACT,
                declared
                    .heads
                    .iter()
                    .filter(|head| head.is_destructive())
                    .map(|head| head.rule_name().to_string())
                    .collect(),
            )],
        )),
        _ => Ok(()),
    }
}

fn require_non_empty(value: &str, field: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::lifecycle::FIELD_REQUIRED,
            format!("{field} must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

fn build_revision(
    rule_set_id: &str,
    revision_number: i64,
    prepared: &PreparedSource,
    grace_days: i64,
    created_by: Option<String>,
    created_at: chrono::DateTime<Utc>,
    builder_model_json: Option<String>,
) -> LifecycleRuleRevision {
    LifecycleRuleRevision {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number,
        rego_source: prepared.rego_source.clone(),
        action_spec_json: prepared.action_spec_json.clone(),
        grace_days,
        source_content_hash: prepared.content_hash.clone(),
        created_by,
        created_at,
        builder_model_json,
    }
}

/// A rule set with its current revision; a missing revision is a repository
/// fault.
fn lifecycle_rule_detail(
    rule_set: LifecycleRuleSet,
    revision: Option<LifecycleRuleRevision>,
) -> AppResult<LifecycleRuleSetDetail> {
    let revision = revision.ok_or_else(|| {
        AppError::Repository(format!(
            "lifecycle rule set {} has no revision {}",
            rule_set.id, rule_set.current_revision_number
        ))
    })?;
    let (declared_actions, load_refusal) = match decode_declared_actions(&revision)? {
        Some(declared) => (declared, None),
        None => (
            LifecycleDeclaredActions::none(),
            Some(crate::error_reasons::lifecycle::RULE_ACTIONS_UNREADABLE),
        ),
    };
    let action_refusal = action_refusal_for(&rule_set, &declared_actions);
    Ok(LifecycleRuleSetDetail {
        rule_set,
        revision,
        declared_actions,
        load_refusal,
        action_refusal,
    })
}

/// The refusal for acting on a rule that cannot be acted on, with the sentence
/// chosen by its reason code.
pub(crate) fn stored_actions_refused(reason: &'static str) -> AppError {
    use crate::error_reasons::lifecycle::{
        RULE_ACTIONS_UNREADABLE, RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN,
    };
    let message = match reason {
        RULE_ACTIONS_UNREADABLE => {
            "this rule was saved before rules declared their own actions; save its source again to use it"
        }
        RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN => {
            "this rule deletes but has no most titles per run, so its actions do not run; set one to use it"
        }
        _ => "this rule cannot be acted on as it is stored",
    };
    AppError::validation_refused(reason, message)
}

/// Read the stored actions back. `None` is a revision from before rules
/// declared their own actions. Stored JSON that no longer deserializes is a
/// repository fault, not a validation failure: it means the document was
/// written by a build whose head set this one does not have.
fn decode_declared_actions(
    revision: &LifecycleRuleRevision,
) -> AppResult<Option<LifecycleDeclaredActions>> {
    LifecycleDeclaredActions::from_persisted_json(&revision.action_spec_json).map_err(|error| {
        AppError::Repository(format!(
            "lifecycle rule revision {} has an unreadable action definition: {error}",
            revision.id
        ))
    })
}

fn format_lifecycle_validation_errors(validation: &ValidationResult) -> String {
    format!(
        "Lifecycle rule validation failed:\n- {}",
        validation.errors.join("\n- ")
    )
}

#[cfg(test)]
#[path = "web_template_tests.rs"]
mod web_template_tests;
