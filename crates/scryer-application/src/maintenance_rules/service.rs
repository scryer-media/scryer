//! Authoring and on-demand preview for maintenance rule sets.
//!
//! Everything here is authoring-time. Nothing schedules evaluation, writes
//! candidates, or executes an action: [`preview_maintenance_rule`] is the only
//! path that runs a matcher, it runs on request against a bounded selection,
//! and it persists nothing.
//!
//! [`preview_maintenance_rule`]: AppUseCase::preview_maintenance_rule

use std::collections::HashMap;

use chrono::Utc;
use scryer_domain::{
    AppPermission, Id, MaintenanceEvaluationMode, MaintenanceRuleRevision, MaintenanceRuleSet,
    MaintenanceRuleSubjectKind, MediaFacet, Title, User,
};
use scryer_rules::maintenance::{
    MaintenanceOutcome, MaintenancePolicy, MaintenanceRulesEngine,
    PERSON_TARGETED_MAINTENANCE_FACTS, rewrite_package_declaration,
};
use scryer_rules::runtime::content_hash;
use scryer_rules::validation::{
    ValidationResult, maintenance_referenced_facts, validate_maintenance_rule,
};

use crate::maintenance_rules::facts::{
    MaintenanceLibraryRef, MaintenanceTitleClaims, MaintenanceTitlePeople, MaintenanceTitleWatch,
    build_subject_input,
};
use crate::maintenance_rules::storage::{MaintenanceRootFileSummary, MaintenanceStorageFactCache};
use crate::maintenance_rules::{
    MaintenanceActionDefinition, MaintenanceDeclaredActions, action_definition_from_persisted_json,
    head_supported_for_scope,
};
use crate::{AppError, AppResult, AppUseCase};
use scryer_rules::maintenance_actions::scan_maintenance_action_heads;
use scryer_rules::validation::ValidationDiagnostic;

/// Upper bound on how many titles one preview may evaluate. Preview is
/// synchronous and builds a fact snapshot per title, so the cap is what keeps a
/// request from turning into a library-wide scan.
pub const MAINTENANCE_PREVIEW_MAX_TITLES: usize = 50;

/// Selection size used when the caller does not ask for one.
pub const MAINTENANCE_PREVIEW_DEFAULT_TITLES: usize = 20;

/// Upper bound on a rule's grace period, in days — ten years.
///
/// The grace period is materialized into a candidate's `due_at` as
/// `first_matched_at + Duration::days(grace_days)`, and `chrono` *panics* rather
/// than saturating once that addition leaves the representable range. A rule
/// authored with a nonsense grace period would therefore not be a slow rule; it
/// would take down the evaluation pass. Ten years is far past any real retention
/// policy and nowhere near the overflow.
pub const MAINTENANCE_MAX_GRACE_DAYS: i64 = 3650;

/// Facts whose semantics depend on one configured library root. A policy that
/// reaches any of these must persist that root identity with its revision.
const STORAGE_MAINTENANCE_FACTS: [&str; 4] = [
    "storage_root_id",
    "storage_available_bytes",
    "storage_total_bytes",
    "storage_available_percent",
];

const STORAGE_ROOT_MEMBERSHIP_UNAVAILABLE: &str = "storage_root_membership_unavailable";

/// Name of the maintenance rule every instance ships with.
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
pub struct MaintenanceRuleDraft {
    pub subject_kind: MaintenanceRuleSubjectKind,
    pub name: String,
    pub description: String,
    /// The rule source. Its action heads say what a match does.
    pub rego_source: String,
    pub grace_days: i64,
    /// Configured root for storage-capacity facts and root-filtered deletion.
    pub storage_root_id: Option<String>,
    /// Empty means every library.
    pub library_ids: Vec<String>,
}

/// A replacement matcher for an existing rule set. Applying one always appends
/// a revision; it never edits the revision in force.
#[derive(Clone, Debug)]
pub struct MaintenanceMatcherDraft {
    pub rego_source: String,
    pub grace_days: i64,
    /// `None` preserves the stored root for older GraphQL clients. `Some(None)`
    /// explicitly clears it, and `Some(Some(_))` replaces it.
    pub storage_root_id: Option<Option<String>>,
}

/// Which matcher a preview should run.
#[derive(Clone, Debug)]
pub enum MaintenancePreviewMatcher {
    /// The current revision of a stored rule set.
    Stored { rule_set_id: String },
    /// An unsaved draft from the editor. Validated and compiled, never stored.
    Inline {
        library_ids: Vec<String>,
        subject_kind: MaintenanceRuleSubjectKind,
        rego_source: String,
        grace_days: i64,
        storage_root_id: Option<String>,
    },
}

/// Which titles a preview should evaluate.
#[derive(Clone, Debug)]
pub enum MaintenancePreviewSelection {
    /// Exactly these titles, at most [`MAINTENANCE_PREVIEW_MAX_TITLES`].
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
pub struct MaintenancePreviewRequest {
    pub matcher: MaintenancePreviewMatcher,
    pub selection: MaintenancePreviewSelection,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A rule set with the revision currently in force and its action definition
/// decoded so no caller downstream handles the persisted JSON shape.
#[derive(Clone, Debug)]
pub struct MaintenanceRuleSetDetail {
    pub rule_set: MaintenanceRuleSet,
    pub revision: MaintenanceRuleRevision,
    pub action_definition: MaintenanceActionDefinition,
}

/// What validating a draft source found: the validation result, and the
/// actions the source declares. `declared_actions` is `None` only when the
/// source does not parse.
#[derive(Clone, Debug)]
pub struct MaintenanceRuleSourceValidation {
    pub result: ValidationResult,
    pub declared_actions: Option<MaintenanceDeclaredActions>,
}

#[derive(Clone, Debug)]
pub struct MaintenancePreviewResult {
    /// The stored rule set's id, or the throwaway id an inline draft compiled
    /// under.
    pub rule_set_id: String,
    /// Hash of the exact source that produced these outcomes.
    pub matcher_content_hash: String,
    pub evaluated_at: chrono::DateTime<Utc>,
    pub titles: Vec<MaintenancePreviewTitleResult>,
}

/// One title's outcome. `outcome` is `None` exactly when `error` is set: a rule
/// that failed produced no decision, and a failure is never rendered as
/// no-match.
#[derive(Clone, Debug)]
pub struct MaintenancePreviewTitleResult {
    pub excluded: bool,
    pub due_at: Option<chrono::DateTime<Utc>>,
    pub due_at_is_estimate: bool,
    pub subject_kind: MaintenanceRuleSubjectKind,
    pub subject_id: String,
    pub subject_label: String,
    pub file_count: i64,
    pub total_size_bytes: i64,
    /// Files eligible for deletion on the revision's selected root. `None`
    /// means this preview is not storage-scoped or the root cannot be resolved.
    pub storage_root_file_count: Option<i64>,
    pub storage_root_total_size_bytes: Option<i64>,
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub library_id: String,
    pub outcome: Option<MaintenanceOutcome>,
    pub reason_codes: Vec<String>,
    pub error: Option<String>,
    /// What the rule decided for this subject. `None` unless it matched.
    pub actions: Option<scryer_rules::maintenance_actions::MaintenanceActions>,
    /// The input document the rule evaluated for this subject, facts and
    /// observations both. `None` unless the caller may read facts about
    /// specific people, because the document carries them.
    pub facts: Option<serde_json::Value>,
}

// ── Service ─────────────────────────────────────────────────────────────────

impl AppUseCase {
    pub async fn list_maintenance_rule_sets(
        &self,
        actor: &User,
    ) -> AppResult<Vec<MaintenanceRuleSet>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .maintenance_rule_sets
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
    pub async fn list_maintenance_rule_set_details(
        &self,
        actor: &User,
    ) -> AppResult<Vec<MaintenanceRuleSetDetail>> {
        let rule_sets = self.list_maintenance_rule_sets(actor).await?;
        let keys: Vec<(String, i64)> = rule_sets
            .iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set.current_revision_number))
            .collect();
        let revisions = self
            .services
            .customization
            .maintenance_rule_sets
            .get_revisions(&keys)
            .await?;
        rule_sets
            .into_iter()
            .zip(revisions)
            .map(|(rule_set, revision)| maintenance_rule_detail(rule_set, revision))
            .collect()
    }

    pub async fn get_maintenance_rule_set(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<MaintenanceRuleSetDetail>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let Some(rule_set) = self
            .services
            .customization
            .maintenance_rule_sets
            .get_rule_set(id)
            .await?
        else {
            return Ok(None);
        };
        self.load_maintenance_rule_detail(rule_set).await.map(Some)
    }

    pub async fn list_maintenance_rule_revisions(
        &self,
        actor: &User,
        rule_set_id: &str,
    ) -> AppResult<Vec<MaintenanceRuleRevision>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .maintenance_rule_sets
            .list_revisions(rule_set_id)
            .await
    }

    pub async fn create_maintenance_rule_set(
        &self,
        actor: &User,
        draft: MaintenanceRuleDraft,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let name = require_non_empty(&draft.name, "name")?;
        let storage_root_id = draft.storage_root_id;

        let id = Id::new_rego_safe().0;
        let prepared = prepare_matcher(
            &id,
            draft.subject_kind,
            &draft.rego_source,
            draft.grace_days,
            storage_root_id.is_some(),
        )?;
        self.require_declared_action_targets(&prepared.declared, &prepared.scan)
            .await?;
        self.require_person_fact_authority(actor, &prepared.rego_source, &id)
            .await?;
        self.require_storage_root_for_matcher(
            &prepared.rego_source,
            &id,
            storage_root_id.as_deref(),
        )
        .await?;

        let now = Utc::now();
        let rule_set = MaintenanceRuleSet {
            id: id.clone(),
            name,
            description: draft.description,
            // Ships dark: nothing evaluates stored rules yet, so a rule that
            // persisted as enabled would be advertising a capability that does
            // not exist.
            enabled: false,
            evaluation_mode: MaintenanceEvaluationMode::Disabled,
            effect_arming: scryer_domain::MaintenanceEffectArming::None,
            destructive_rearm_required: false,
            library_ids: draft.library_ids,
            subject_kind: draft.subject_kind,
            system_owned: false,
            current_revision_number: 1,
            created_at: now,
            updated_at: now,
        };
        let revision = build_revision(
            &id,
            1,
            &prepared,
            draft.grace_days,
            storage_root_id,
            Some(actor.id.clone()),
            now,
        );

        self.services
            .customization
            .maintenance_rule_sets
            .create_rule_set(&rule_set, &revision)
            .await?;

        Ok(MaintenanceRuleSetDetail {
            rule_set,
            revision,
            action_definition: MaintenanceActionDefinition::Declared(prepared.declared),
        })
    }

    /// Appends revision N+1. Revision N is left exactly as it was written, so a
    /// candidate recorded against it stays attributable.
    ///
    /// When the new source defines a delete action, appending it also disarms
    /// the rule ([`MaintenanceEffectArming::None`]), atomically with the
    /// append. The destructive acknowledgement is bound to the matcher it was
    /// granted against, so a replacement matcher that can delete has to be
    /// confirmed again on its own terms. Without this, catalog-settings
    /// authority alone would be enough to swap an armed rule's matcher and have
    /// the next handler pass delete under someone else's acknowledgement. A
    /// rule that was armed is flagged as edited since it was enabled, so the
    /// operator is told to review it and enable it again.
    ///
    /// A new source that defines no delete action keeps the rule's state: it
    /// cannot delete anything, so there is no destructive blast radius to
    /// acknowledge again. A metadata rename
    /// does *not* disarm: it changes neither what the rule matches nor what it
    /// does, so the acknowledgement still describes the same blast radius.
    ///
    /// [`MaintenanceEffectArming::None`]: scryer_domain::MaintenanceEffectArming::None
    pub async fn update_maintenance_rule_matcher(
        &self,
        actor: &User,
        rule_set_id: &str,
        draft: MaintenanceMatcherDraft,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        let current_revision = self
            .services
            .customization
            .maintenance_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "maintenance rule set {} has no revision {}",
                    rule_set.id, rule_set.current_revision_number
                ))
            })?;
        let storage_root_id = draft
            .storage_root_id
            .clone()
            .unwrap_or(current_revision.storage_root_id);
        let prepared = prepare_matcher(
            &rule_set.id,
            rule_set.subject_kind,
            &draft.rego_source,
            draft.grace_days,
            storage_root_id.is_some(),
        )?;
        if rule_set.system_owned
            && (prepared.action_spec_json != current_revision.action_spec_json
                || scryer_rules::strip_editor_source(&prepared.rego_source)
                    != scryer_rules::strip_editor_source(&current_revision.rego_source))
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_SHIPS_SCRYER,
                "This rule ships with Scryer: its matcher and actions cannot be changed. Its grace \
                 period, libraries and state can.",
            ));
        }
        self.require_declared_action_targets(&prepared.declared, &prepared.scan)
            .await?;
        self.require_person_fact_authority(actor, &prepared.rego_source, &rule_set.id)
            .await?;
        self.require_storage_root_for_matcher(
            &prepared.rego_source,
            &rule_set.id,
            storage_root_id.as_deref(),
        )
        .await?;

        let now = Utc::now();
        let revision_number = rule_set.current_revision_number + 1;
        let revision = build_revision(
            &rule_set.id,
            revision_number,
            &prepared,
            draft.grace_days,
            storage_root_id,
            Some(actor.id.clone()),
            now,
        );

        let destructive = prepared.declared.destructive;
        let rules = &self.services.customization.maintenance_rule_sets;
        if destructive {
            rules.add_revision(&revision, now).await?;
        } else {
            rules.add_revision_keeping_arming(&revision, now).await?;
        }

        rule_set.current_revision_number = revision_number;
        if destructive {
            // Mirrors what `add_revision` wrote in the same transaction, so the
            // detail this returns can never claim an arming the store just
            // cleared.
            rule_set.disarm_after_edit();
        }
        rule_set.updated_at = now;
        Ok(MaintenanceRuleSetDetail {
            rule_set,
            revision,
            action_definition: MaintenanceActionDefinition::Declared(prepared.declared),
        })
    }

    /// Renames and re-scopes without touching the matcher, action, or grace
    /// period, so no revision is created.
    ///
    /// A *scope* change disarms the rule ([`MaintenanceEffectArming::None`]),
    /// atomically with the metadata write. Library scope is the rule's blast
    /// radius: an operator armed one matcher plus action over one set of
    /// libraries after acknowledging what that would reach, so widening the
    /// scope would put libraries nobody acknowledged inside an armed rule, and
    /// narrowing it changes the acknowledged set just as materially. A scope
    /// change therefore invalidates the acknowledgement exactly like a new
    /// revision does, and arming has to be granted again on the new terms.
    ///
    /// A name or description edit changes neither what the rule matches, what it
    /// does, nor where it reaches, so it leaves arming alone. The comparison is
    /// set-based: reordering the same libraries is not a scope change.
    ///
    /// [`MaintenanceEffectArming::None`]: scryer_domain::MaintenanceEffectArming::None
    pub async fn update_maintenance_rule_metadata(
        &self,
        actor: &User,
        rule_set_id: &str,
        name: String,
        description: String,
        library_ids: Vec<String>,
    ) -> AppResult<MaintenanceRuleSet> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        let name = require_non_empty(&name, "name")?;
        if rule_set.system_owned && name != rule_set.name {
            return Err(AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_SHIPS_SCRYER_CANNOT_RENAMED,
                "This rule ships with Scryer and cannot be renamed.",
            ));
        }
        let now = Utc::now();
        let scope_changed = library_scope_changed(&rule_set.library_ids, &library_ids);

        self.services
            .customization
            .maintenance_rule_sets
            .update_rule_set_metadata(
                &rule_set.id,
                &name,
                &description,
                &library_ids,
                scope_changed,
                now,
            )
            .await?;

        rule_set.name = name;
        rule_set.description = description;
        rule_set.library_ids = library_ids;
        if scope_changed {
            // Mirrors what the store wrote in the same statement, so the rule
            // this returns can never claim an arming the store just cleared.
            rule_set.disarm_after_edit();
        }
        rule_set.updated_at = now;
        Ok(rule_set)
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
    pub async fn delete_maintenance_rule_set(
        &self,
        actor: &User,
        rule_set_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let rule_set = self.require_maintenance_rule_set(rule_set_id).await?;
        if rule_set.system_owned {
            return Err(AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_SHIPS_SCRYER_CANNOT_DELETED,
                "This rule ships with Scryer and cannot be deleted. Set it to Disabled instead.",
            ));
        }

        // Bounded existence reads: one row is enough to answer both questions.
        let has_history = !self
            .services
            .customization
            .maintenance_evaluation
            .list_action_runs(Some(&rule_set.id), None, Some(1))
            .await?
            .is_empty();
        if has_history {
            return Err(AppError::validation_refused(crate::error_reasons::maintenance::ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID, format!(
                "\"{}\" has already run actions on your library, and deleting it would erase the \
                 record of what it did. Set it to Disabled instead: the rule stops \
                 immediately and its history is kept.",
                rule_set.name
            )));
        }

        let active_candidates = self
            .count_active_maintenance_candidates(&rule_set.id)
            .await?;
        if active_candidates > 0 {
            return Err(AppError::validation_refused(crate::error_reasons::maintenance::CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE, format!(
                "\"{}\" is currently tracking {active_candidates} title(s), and deleting it would \
                 drop them without a trace. Set it to Disabled instead, or clear the \
                 candidates first.",
                rule_set.name
            )));
        }

        self.services
            .customization
            .maintenance_rule_sets
            .delete_rule_set_if_unused(rule_set_id)
            .await
    }

    /// Create the rule every instance ships with: titles whose request lease
    /// expired are deleted with their files after a seven-day grace period.
    /// It starts Disabled, like every other rule: an operator enables it on
    /// purpose. Startup calls this on every boot, so once the rule exists it
    /// only brings a previously shipped matcher up to date.
    pub async fn ensure_expired_request_leases_rule(&self) -> AppResult<()> {
        let rules = &self.services.customization.maintenance_rule_sets;
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
        let prepared = prepare_matcher(
            &id,
            MaintenanceRuleSubjectKind::Title,
            EXPIRED_REQUEST_LEASES_REGO,
            grace_days,
            false,
        )?;
        let now = Utc::now();
        let revision = build_revision(&id, 1, &prepared, grace_days, None, None, now);
        let rule_set = MaintenanceRuleSet {
            id,
            name: EXPIRED_REQUEST_LEASES_RULE_NAME.into(),
            description: EXPIRED_REQUEST_LEASES_DESCRIPTION.into(),
            enabled: false,
            evaluation_mode: MaintenanceEvaluationMode::Disabled,
            effect_arming: scryer_domain::MaintenanceEffectArming::None,
            destructive_rearm_required: false,
            library_ids: Vec::new(),
            subject_kind: MaintenanceRuleSubjectKind::Title,
            system_owned: true,
            current_revision_number: 1,
            created_at: now,
            updated_at: now,
        };
        rules.create_rule_set(&rule_set, &revision).await
    }

    /// Bring the shipped rule onto the current source, keeping its identity,
    /// state, grace period, libraries and storage root. The rule's matcher
    /// and action could never be edited, so a stored source is one of the
    /// sources the rule shipped with:
    ///
    /// - the current one: left alone;
    /// - the `match` form, whose separate action deleted the title and its
    ///   files: replaced, keeping the rule's arming, because the new source
    ///   deletes exactly the same titles the operator armed it for;
    /// - the keep-claim matcher: replaced and disarmed, because it also
    ///   spared kept titles, so the operator re-arms it against the new one;
    /// - anything else: replaced and disarmed, the same conservative way.
    async fn upgrade_expired_request_leases_rule(
        &self,
        rule_set: MaintenanceRuleSet,
    ) -> AppResult<()> {
        let rules = &self.services.customization.maintenance_rule_sets;
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

        let prepared = prepare_matcher(
            &rule_set.id,
            rule_set.subject_kind,
            EXPIRED_REQUEST_LEASES_REGO,
            current.grace_days,
            current.storage_root_id.is_some(),
        )?;
        let now = Utc::now();
        let revision = build_revision(
            &rule_set.id,
            rule_set.current_revision_number + 1,
            &prepared,
            current.grace_days,
            current.storage_root_id.clone(),
            None,
            now,
        );
        if keeps_arming {
            rules.add_revision_keeping_arming(&revision, now).await?;
        } else {
            rules.add_revision(&revision, now).await?;
        }

        if rule_set.description == PREVIOUS_EXPIRED_REQUEST_LEASES_DESCRIPTION {
            rules
                .update_rule_set_metadata(
                    &rule_set.id,
                    &rule_set.name,
                    EXPIRED_REQUEST_LEASES_DESCRIPTION,
                    &rule_set.library_ids,
                    false,
                    now,
                )
                .await?;
        }
        Ok(())
    }

    /// Editor support: compile and check a draft without storing it. The
    /// throwaway id only satisfies the package-declaration check.
    pub async fn validate_maintenance_rule_source(
        &self,
        actor: &User,
        rego_source: &str,
    ) -> AppResult<MaintenanceRuleSourceValidation> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let scratch_id = Id::new_rego_safe().0;
        let rewritten = rewrite_package_declaration(rego_source, &scratch_id);
        let result = validate_maintenance_rule(&rewritten, &scratch_id).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {e}"),
            )
        })?;
        // A source that does not parse declares nothing; one that parses
        // declares its heads even while some other check refuses it, so the
        // editor can show what the rule would do while the author fixes it.
        let declared_actions = scan_maintenance_action_heads(&rewritten, &scratch_id)
            .ok()
            .map(|scan| MaintenanceDeclaredActions::from_scan(&scan));
        Ok(MaintenanceRuleSourceValidation {
            result,
            declared_actions,
        })
    }

    /// Evaluate one matcher against a bounded title selection. Nothing is
    /// written: preview answers "what would this match", not "what happened".
    pub async fn preview_maintenance_rule(
        &self,
        actor: &User,
        request: MaintenancePreviewRequest,
    ) -> AppResult<MaintenancePreviewResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let (policy, subject_kind, library_ids, grace_days, storage_root_id) =
            self.resolve_preview_policy(request.matcher).await?;
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
            MaintenancePreviewSelection::Library { limit, .. } => (
                limit
                    .unwrap_or(MAINTENANCE_PREVIEW_DEFAULT_TITLES)
                    .clamp(1, MAINTENANCE_PREVIEW_MAX_TITLES),
                None,
            ),
            MaintenancePreviewSelection::Subjects { subject_ids, .. } => {
                if subject_ids.is_empty() || subject_ids.len() > MAINTENANCE_PREVIEW_MAX_TITLES {
                    return Err(AppError::validation_refused(crate::error_reasons::maintenance::PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS, "preview requires between one and fifty subjects"));
                }
                (MAINTENANCE_PREVIEW_MAX_TITLES, Some(subject_ids))
            }
            MaintenancePreviewSelection::Titles(_) => (MAINTENANCE_PREVIEW_MAX_TITLES, None),
        };
        let libraries = self.maintenance_library_refs().await?;
        let exclusions = self
            .services
            .customization
            .maintenance_evaluation
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
        let mut episode_inventories_by_title = HashMap::new();
        for title in &titles {
            if title.facet != MediaFacet::Movie {
                episode_inventories_by_title.insert(
                    title.id.clone(),
                    self.maintenance_load_title_episode_inventory(title).await?,
                );
            }
        }
        // Preview must see exactly what the evaluator sees, so the same two
        // batched people lookups run here, once for the whole selection, and so
        // does the series-movie load.
        let series_movies_by_title = self.maintenance_series_movies_for_titles(&titles).await?;
        let requesters_by_title = self.maintenance_requesters_for_titles(&titles).await?;
        let usernames = self.maintenance_usernames_by_id().await?;
        // Watch signals go through the same run-scoped gate the evaluator uses,
        // so a preview cannot show a match the scheduled pass would hold.
        let watch_context = self.maintenance_watch_context().await?;
        let signals_by_title = self
            .maintenance_watch_signals_for_titles(&watch_context, &titles)
            .await?;
        let (watch_context, audience_by_title) = self
            .maintenance_audience_plays_for_titles(&watch_context, &titles)
            .await;
        // Claims are read exactly as the pass reads them, including what
        // happens when the store is down: preview degrades to unknown claim
        // facts rather than erroring, so an operator previewing a lease rule
        // during an outage sees the same hold the scheduled pass would produce.
        let claims_by_title = match self.maintenance_claims_for_titles(&titles).await {
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
        let lists_by_title = self.maintenance_list_facts_or_unknown(&titles).await;

        let matcher_content_hash = content_hash(&policy.rego_source);
        let rule_set_id = policy.id.clone();
        let engine = MaintenanceRulesEngine::build(std::slice::from_ref(&policy)).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_FAILED_COMPILE,
                format!("rule failed to compile: {e}"),
            )
        })?;
        let mut evaluator = engine.evaluator();
        let mut storage_facts = MaintenanceStorageFactCache::new();

        let evaluated_at = Utc::now();
        let mut results = Vec::with_capacity(titles.len());
        for title in titles {
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
            let subjects = if title.facet == MediaFacet::Movie {
                self.maintenance_subjects_for_title(subject_kind, &title, files)
                    .await?
            } else {
                let inventory = episode_inventories_by_title.get(&title.id).ok_or_else(|| {
                    AppError::Repository("maintenance preview lost title episode inventory".into())
                })?;
                self.maintenance_subjects_from_episode_inventory(
                    subject_kind,
                    &title,
                    files,
                    inventory,
                )?
            };
            for subject in subjects {
                if results.len() >= result_limit {
                    break;
                }
                if selected_subjects.is_some_and(|ids| !ids.contains(&subject.id)) {
                    continue;
                }
                let root_file_summary = match storage_root_id.as_deref() {
                    None => {
                        self.maintenance_root_file_summary(&subject.files, None)
                            .await?
                    }
                    Some(root_id) if subject.kind == MaintenanceRuleSubjectKind::Title => {
                        match self.maintenance_title_deletion_files(&title).await {
                            Ok(owned_files) => {
                                self.maintenance_root_file_summary(&owned_files, Some(root_id))
                                    .await?
                            }
                            Err(error) => {
                                tracing::warn!(
                                    rule_set_id = policy.id.as_str(),
                                    title_id = title.id.as_str(),
                                    error = %error,
                                    "could not prove title file ownership for storage preview"
                                );
                                MaintenanceRootFileSummary::default()
                            }
                        }
                    }
                    Some(_) if subject.ambiguous_files => MaintenanceRootFileSummary::default(),
                    Some(root_id) => {
                        self.maintenance_root_file_summary(&subject.files, Some(root_id))
                            .await?
                    }
                };
                if storage_root_id.is_some() && matches!(root_file_summary.file_count, Some(0)) {
                    continue;
                }
                let storage_root_membership_unknown =
                    storage_root_id.is_some() && root_file_summary.file_count.is_none();
                let mut input = build_subject_input(
                    evaluated_at,
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
                );
                super::list_facts::apply_list_facts(&mut input, &lists_by_title, &title.id);
                self.populate_maintenance_storage_facts_from_cache(
                    &mut input,
                    storage_root_id.as_deref(),
                    &mut storage_facts,
                )
                .await?;

                let evaluation = evaluator.evaluate(&input).map_err(|e| {
                    AppError::validation_refused(
                        crate::error_reasons::maintenance::RULE_EVALUATION_FAILED,
                        format!("rule evaluation failed: {e}"),
                    )
                })?;

                let (outcome, reason_codes, error, actions) = if storage_root_membership_unknown {
                    (
                        Some(MaintenanceOutcome::Unknown),
                        vec![STORAGE_ROOT_MEMBERSHIP_UNAVAILABLE.to_string()],
                        None,
                        None,
                    )
                } else {
                    match (evaluation.records.first(), evaluation.errors.first()) {
                        (Some(record), _) => (
                            Some(record.decision.outcome),
                            record.decision.reason_codes.clone(),
                            None,
                            (record.decision.outcome == MaintenanceOutcome::Match)
                                .then(|| record.decision.actions.clone()),
                        ),
                        (None, Some(failure)) => {
                            (None, Vec::new(), Some(failure.message.clone()), None)
                        }
                        (None, None) => (
                            None,
                            Vec::new(),
                            Some("rule produced no decision".to_string()),
                            None,
                        ),
                    }
                };

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
                let active = self
                    .services
                    .customization
                    .maintenance_evaluation
                    .get_active_subject_candidate(
                        &rule_set_id,
                        subject_kind.as_storage_str(),
                        &subject.id,
                    )
                    .await?
                    .filter(|candidate| candidate.matcher_content_hash == matcher_content_hash);
                let due_at = if !excluded && outcome == Some(MaintenanceOutcome::Match) {
                    Some(active.as_ref().map_or(
                        evaluated_at + chrono::Duration::days(grace_days),
                        |candidate| candidate.due_at,
                    ))
                } else {
                    None
                };
                results.push(MaintenancePreviewTitleResult {
                    excluded,
                    due_at,
                    due_at_is_estimate: due_at.is_some() && active.is_none(),
                    subject_kind,
                    subject_id: subject.id,
                    subject_label: subject.label,
                    file_count: subject.files.len() as i64,
                    total_size_bytes: subject.files.iter().map(|file| file.size_bytes).sum(),
                    storage_root_file_count: root_file_summary.file_count,
                    storage_root_total_size_bytes: root_file_summary.total_size_bytes,
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    library_id: title.library_id.clone(),
                    outcome,
                    reason_codes,
                    error,
                    actions,
                    facts,
                });
            }
        }

        if selected_subjects.is_some_and(|ids| {
            ids.iter()
                .any(|id| !results.iter().any(|result| &result.subject_id == id))
        }) {
            return Err(AppError::validation_refused(
                crate::error_reasons::maintenance::PREVIEW_SUBJECT_OUTSIDE_SCOPE,
                "preview subject is missing, outside the rule libraries, or does not belong to the selected titles and scope",
            ));
        }
        Ok(MaintenancePreviewResult {
            rule_set_id,
            matcher_content_hash,
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
        let referenced = maintenance_referenced_facts(rego_source, rule_set_id).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::maintenance::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {e}"),
            )
        })?;
        let person_facts: Vec<&str> = PERSON_TARGETED_MAINTENANCE_FACTS
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
            crate::error_reasons::maintenance::PERSON_TARGETING_PERMISSION_REQUIRED,
            format!(
                "This rule reads facts about specific people ({}), so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
                person_facts.join(", ")
            ),
        ))
    }

    /// Resolve a selected root by its durable ID and reject a storage matcher
    /// whose revision omits one. Root selection is global because a rule may
    /// cover more than one library while pressure is measured on one mounted
    /// filesystem.
    async fn require_storage_root_for_matcher(
        &self,
        rego_source: &str,
        rule_set_id: &str,
        storage_root_id: Option<&str>,
    ) -> AppResult<()> {
        let referenced =
            maintenance_referenced_facts(rego_source, rule_set_id).map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::maintenance::RULE_VALIDATION_FAILED,
                    format!("rule validation failed: {error}"),
                )
            })?;
        let reads_storage = STORAGE_MAINTENANCE_FACTS
            .iter()
            .any(|fact| referenced.contains(*fact));
        if reads_storage && storage_root_id.is_none() {
            return Err(AppError::validation_refused(crate::error_reasons::maintenance::MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT, "a matcher that reads storage facts must select a configured library root"));
        }
        let Some(storage_root_id) = storage_root_id else {
            return Ok(());
        };
        let roots = self.services.catalog.libraries.list(None).await?;
        if roots
            .iter()
            .flat_map(|library| library.roots.iter())
            .any(|root| root.id == storage_root_id)
        {
            Ok(())
        } else {
            Err(AppError::validation_refused(
                crate::error_reasons::maintenance::STORAGE_ROOT_NOT_CONFIGURED,
                format!("storage root '{storage_root_id}' is not configured"),
            ))
        }
    }

    pub(crate) async fn require_maintenance_rule_set(
        &self,
        id: &str,
    ) -> AppResult<MaintenanceRuleSet> {
        self.services
            .customization
            .maintenance_rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::maintenance::RULE_SET_NOT_FOUND,
                    format!("maintenance rule set {id} not found"),
                )
            })
    }

    pub(crate) async fn load_maintenance_rule_detail(
        &self,
        rule_set: MaintenanceRuleSet,
    ) -> AppResult<MaintenanceRuleSetDetail> {
        let revision = self
            .services
            .customization
            .maintenance_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?;
        maintenance_rule_detail(rule_set, revision)
    }

    async fn resolve_preview_policy(
        &self,
        matcher: MaintenancePreviewMatcher,
    ) -> AppResult<(
        MaintenancePolicy,
        MaintenanceRuleSubjectKind,
        Vec<String>,
        i64,
        Option<String>,
    )> {
        match matcher {
            MaintenancePreviewMatcher::Stored { rule_set_id } => {
                let rule_set = self.require_maintenance_rule_set(&rule_set_id).await?;
                let detail = self.load_maintenance_rule_detail(rule_set).await?;
                self.require_storage_root_for_matcher(
                    &detail.revision.rego_source,
                    &detail.rule_set.id,
                    detail.revision.storage_root_id.as_deref(),
                )
                .await?;
                Ok((
                    MaintenancePolicy {
                        id: detail.rule_set.id,
                        name: detail.rule_set.name,
                        rego_source: detail.revision.rego_source,
                    },
                    detail.rule_set.subject_kind,
                    detail.rule_set.library_ids,
                    detail.revision.grace_days,
                    detail.revision.storage_root_id,
                ))
            }
            MaintenancePreviewMatcher::Inline {
                library_ids,
                subject_kind,
                rego_source,
                grace_days,
                storage_root_id,
            } => {
                let scratch_id = Id::new_rego_safe().0;
                let prepared = prepare_matcher(
                    &scratch_id,
                    subject_kind,
                    &rego_source,
                    grace_days,
                    storage_root_id.is_some(),
                )?;
                self.require_storage_root_for_matcher(
                    &prepared.rego_source,
                    &scratch_id,
                    storage_root_id.as_deref(),
                )
                .await?;
                Ok((
                    MaintenancePolicy {
                        id: scratch_id,
                        name: "preview".to_string(),
                        rego_source: prepared.rego_source,
                    },
                    subject_kind,
                    library_ids,
                    grace_days,
                    storage_root_id,
                ))
            }
        }
    }

    async fn select_preview_titles(
        &self,
        selection: &MaintenancePreviewSelection,
    ) -> AppResult<Vec<Title>> {
        match selection {
            MaintenancePreviewSelection::Titles(title_ids)
            | MaintenancePreviewSelection::Subjects { title_ids, .. } => {
                if title_ids.len() > MAINTENANCE_PREVIEW_MAX_TITLES {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::maintenance::PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED,
                        format!(
                            "preview accepts at most {MAINTENANCE_PREVIEW_MAX_TITLES} titles, {} were requested",
                            title_ids.len()
                        ),
                    ));
                }
                self.services.catalog.titles.get_by_ids(title_ids).await
            }
            MaintenancePreviewSelection::Library { library_id, limit } => {
                let limit = limit
                    .unwrap_or(MAINTENANCE_PREVIEW_DEFAULT_TITLES)
                    .clamp(1, MAINTENANCE_PREVIEW_MAX_TITLES);
                let mut titles = Vec::new();
                let mut after_id = None;
                loop {
                    let page = self
                        .services
                        .catalog
                        .titles
                        .list_page_after_id(after_id, MAINTENANCE_PREVIEW_MAX_TITLES)
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
    pub(crate) async fn maintenance_library_refs(
        &self,
    ) -> AppResult<HashMap<String, MaintenanceLibraryRef>> {
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
                    MaintenanceLibraryRef {
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
        declared: &MaintenanceDeclaredActions,
        scan: &scryer_rules::maintenance_actions::MaintenanceActionScan,
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
        if declared.quality_profiles.is_empty() {
            return Ok(());
        }
        let profiles = self.maintenance_quality_profile_ids_by_name().await?;
        let diagnostics: Vec<ValidationDiagnostic> = scan
            .quality_profiles
            .iter()
            .filter(|literal| !profiles.contains_key(&literal.value))
            .map(|literal| {
                let mut diagnostic = ValidationDiagnostic::new(
                    crate::error_reasons::maintenance::QUALITY_PROFILE_NOT_FOUND,
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
            crate::error_reasons::maintenance::QUALITY_PROFILE_NOT_FOUND,
            "The rule names a quality profile that does not exist.",
            diagnostics,
        ))
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

/// A matcher that passed every authoring-time check, ready to persist.
struct PreparedMatcher {
    rego_source: String,
    content_hash: String,
    action_spec_json: String,
    declared: MaintenanceDeclaredActions,
    scan: scryer_rules::maintenance_actions::MaintenanceActionScan,
}

/// Rewrite the package to the system-assigned id, then run every check that
/// must hold before a revision is written: the matcher compiles, reads only
/// documented facts and defines at least one action head, every head it
/// defines is one its scope can run, and the grace period is a real duration.
fn prepare_matcher(
    rule_id: &str,
    subject_kind: MaintenanceRuleSubjectKind,
    rego_source: &str,
    grace_days: i64,
    storage_root_selected: bool,
) -> AppResult<PreparedMatcher> {
    if grace_days < 0 {
        return Err(AppError::validation_refused(
            crate::error_reasons::maintenance::GRACE_PERIOD_INVALID,
            "grace period must be zero or more days".to_string(),
        ));
    }
    if grace_days > MAINTENANCE_MAX_GRACE_DAYS {
        return Err(AppError::validation_refused(
            crate::error_reasons::maintenance::GRACE_PERIOD_OUT_OF_RANGE,
            format!("grace period must be at most {MAINTENANCE_MAX_GRACE_DAYS} days (ten years)"),
        ));
    }

    let rewritten = rewrite_package_declaration(rego_source, rule_id);
    let validation = validate_maintenance_rule(&rewritten, rule_id).map_err(|e| {
        AppError::validation_refused(
            crate::error_reasons::maintenance::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {e}"),
        )
    })?;
    if !validation.valid {
        let reason = if validation.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == scryer_rules::diagnostic_codes::RULE_ACTION_REQUIRED
        }) {
            crate::error_reasons::maintenance::ACTION_HEAD_REQUIRED
        } else {
            crate::error_reasons::maintenance::RULE_SOURCE_INVALID
        };
        return Err(AppError::validation_refused_with_diagnostics(
            reason,
            format_maintenance_validation_errors(&validation),
            validation.diagnostics,
        ));
    }
    let scan = scan_maintenance_action_heads(&rewritten, rule_id).map_err(|e| {
        AppError::validation_refused(
            crate::error_reasons::maintenance::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {e}"),
        )
    })?;

    let unsupported: Vec<ValidationDiagnostic> = scan
        .defined_heads
        .iter()
        .filter(|head| !head_supported_for_scope(**head, subject_kind, storage_root_selected))
        .map(|head| {
            let mut diagnostic = ValidationDiagnostic::new(
                crate::error_reasons::maintenance::ACTION_HEAD_UNSUPPORTED_FOR_SCOPE,
                format!("{} cannot run for this rule's scope", head.rule_name()),
            );
            if let Some((line, column)) = scan.head_positions.get(head) {
                diagnostic.line = Some(*line);
                diagnostic.column = Some(*column);
            }
            diagnostic
        })
        .collect();
    if !unsupported.is_empty() {
        return Err(AppError::validation_refused_with_diagnostics(
            crate::error_reasons::maintenance::ACTION_HEAD_UNSUPPORTED_FOR_SCOPE,
            "The rule defines an action its scope cannot run.",
            unsupported,
        ));
    }

    let declared = MaintenanceDeclaredActions::from_scan(&scan);
    let action_spec_json = MaintenanceActionDefinition::Declared(declared.clone())
        .to_persisted_json()
        .map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::maintenance::ACTION_NOT_STORABLE,
                format!("maintenance action is not storable: {e}"),
            )
        })?;

    Ok(PreparedMatcher {
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

fn require_non_empty(value: &str, field: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::maintenance::FIELD_REQUIRED,
            format!("{field} must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

fn build_revision(
    rule_set_id: &str,
    revision_number: i64,
    prepared: &PreparedMatcher,
    grace_days: i64,
    storage_root_id: Option<String>,
    created_by: Option<String>,
    created_at: chrono::DateTime<Utc>,
) -> MaintenanceRuleRevision {
    MaintenanceRuleRevision {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number,
        rego_source: prepared.rego_source.clone(),
        action_spec_json: prepared.action_spec_json.clone(),
        grace_days,
        storage_root_id,
        matcher_content_hash: prepared.content_hash.clone(),
        created_by,
        created_at,
    }
}

/// A rule set with its current revision; a missing revision is a repository
/// fault.
fn maintenance_rule_detail(
    rule_set: MaintenanceRuleSet,
    revision: Option<MaintenanceRuleRevision>,
) -> AppResult<MaintenanceRuleSetDetail> {
    let revision = revision.ok_or_else(|| {
        AppError::Repository(format!(
            "maintenance rule set {} has no revision {}",
            rule_set.id, rule_set.current_revision_number
        ))
    })?;
    let action_definition = decode_action_definition(&revision)?;
    Ok(MaintenanceRuleSetDetail {
        rule_set,
        revision,
        action_definition,
    })
}

/// Read the stored action back through the closed catalog. Stored JSON that no
/// longer deserializes is a repository fault, not a validation failure: it
/// means a spec was written by a build whose catalog this one does not have.
fn decode_action_definition(
    revision: &MaintenanceRuleRevision,
) -> AppResult<MaintenanceActionDefinition> {
    action_definition_from_persisted_json(&revision.action_spec_json).map_err(|error| {
        AppError::Repository(format!(
            "maintenance rule revision {} has an unreadable action definition: {error}",
            revision.id
        ))
    })
}

fn format_maintenance_validation_errors(validation: &ValidationResult) -> String {
    format!(
        "Maintenance rule validation failed:\n- {}",
        validation.errors.join("\n- ")
    )
}

#[cfg(test)]
#[path = "web_template_tests.rs"]
mod web_template_tests;
