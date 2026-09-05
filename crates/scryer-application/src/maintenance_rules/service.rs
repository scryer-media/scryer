//! Authoring and on-demand preview for maintenance rule sets (RFC 137 tracks
//! A3/A4).
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

use crate::maintenance_rules::action_execution::{
    EXECUTABLE_TITLE_RULE_ACTIONS, title_rule_action_not_executable,
};
use crate::maintenance_rules::facts::{
    MaintenanceLibraryRef, MaintenanceTitlePeople, MaintenanceTitleWatch, build_title_input,
};
use crate::maintenance_rules::{
    MaintenanceActionSpec, MaintenanceSubjectKind as ActionSubjectKind,
};
use crate::{AppError, AppResult, AppUseCase};

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

// ── Request models ──────────────────────────────────────────────────────────

/// A new rule set plus its first matcher revision.
#[derive(Clone, Debug)]
pub struct MaintenanceRuleDraft {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub action_spec: MaintenanceActionSpec,
    pub grace_days: i64,
    /// Empty means every library.
    pub library_ids: Vec<String>,
    /// Only [`MaintenanceEvaluationMode::Disabled`] is accepted in this wave.
    pub evaluation_mode: Option<MaintenanceEvaluationMode>,
}

/// A replacement matcher for an existing rule set. Applying one always appends
/// a revision; it never edits the revision in force.
#[derive(Clone, Debug)]
pub struct MaintenanceMatcherDraft {
    pub rego_source: String,
    pub action_spec: MaintenanceActionSpec,
    pub grace_days: i64,
}

/// Which matcher a preview should run.
#[derive(Clone, Debug)]
pub enum MaintenancePreviewMatcher {
    /// The current revision of a stored rule set.
    Stored { rule_set_id: String },
    /// An unsaved draft from the editor. Validated and compiled, never stored.
    Inline {
        rego_source: String,
        action_spec: MaintenanceActionSpec,
        grace_days: i64,
    },
}

/// Which titles a preview should evaluate.
#[derive(Clone, Debug)]
pub enum MaintenancePreviewSelection {
    /// Exactly these titles, at most [`MAINTENANCE_PREVIEW_MAX_TITLES`].
    Titles(Vec<String>),
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

/// A rule set with the revision currently in force, action spec already
/// decoded so no caller downstream handles the stored JSON.
#[derive(Clone, Debug)]
pub struct MaintenanceRuleSetDetail {
    pub rule_set: MaintenanceRuleSet,
    pub revision: MaintenanceRuleRevision,
    pub action_spec: MaintenanceActionSpec,
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
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub library_id: String,
    pub outcome: Option<MaintenanceOutcome>,
    pub reason_codes: Vec<String>,
    pub error: Option<String>,
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
        let mut details = Vec::with_capacity(rule_sets.len());
        for rule_set in rule_sets {
            details.push(self.load_maintenance_rule_detail(rule_set).await?);
        }
        Ok(details)
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

        require_dormant_evaluation_mode(draft.evaluation_mode)?;
        let name = require_non_empty(&draft.name, "name")?;

        let id = Id::new_rego_safe().0;
        let prepared = prepare_matcher(
            &id,
            &draft.rego_source,
            &draft.action_spec,
            draft.grace_days,
        )?;
        self.require_registered_action_tags(&draft.action_spec)
            .await?;
        self.require_person_fact_authority(actor, &prepared.rego_source, &id)
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
            library_ids: draft.library_ids,
            subject_kind: MaintenanceRuleSubjectKind::Title,
            current_revision_number: 1,
            created_at: now,
            updated_at: now,
        };
        let revision = build_revision(&id, 1, &prepared, draft.grace_days, actor, now);

        self.services
            .customization
            .maintenance_rule_sets
            .create_rule_set(&rule_set, &revision)
            .await?;

        Ok(MaintenanceRuleSetDetail {
            rule_set,
            revision,
            action_spec: draft.action_spec,
        })
    }

    /// Appends revision N+1. Revision N is left exactly as it was written, so a
    /// candidate recorded against it stays attributable (RFC 7.1).
    ///
    /// Appending a revision also disarms the rule
    /// ([`MaintenanceEffectArming::None`]), atomically with the append. Arming
    /// never outlives the revision it was granted against: an operator armed a
    /// specific matcher plus action after acknowledging *that* matcher's blast
    /// radius, so a replacement matcher has to be armed again on its own terms.
    /// Without this, catalog-settings authority alone would be enough to swap an
    /// armed rule's matcher and have the next handler pass act destructively
    /// under someone else's acknowledgement.
    ///
    /// A mode change or a metadata rename does *not* disarm: neither changes
    /// what the rule matches or what it does, so the acknowledgement still
    /// describes the same blast radius.
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
        let prepared = prepare_matcher(
            &rule_set.id,
            &draft.rego_source,
            &draft.action_spec,
            draft.grace_days,
        )?;
        self.require_registered_action_tags(&draft.action_spec)
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
            actor,
            now,
        );

        self.services
            .customization
            .maintenance_rule_sets
            .add_revision(&revision, now)
            .await?;

        rule_set.current_revision_number = revision_number;
        // Mirrors what `add_revision` wrote in the same transaction, so the
        // detail this returns can never claim an arming the store just cleared.
        rule_set.effect_arming = scryer_domain::MaintenanceEffectArming::None;
        rule_set.updated_at = now;
        Ok(MaintenanceRuleSetDetail {
            rule_set,
            revision,
            action_spec: draft.action_spec,
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
            rule_set.effect_arming = scryer_domain::MaintenanceEffectArming::None;
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

        // Bounded existence reads: one row is enough to answer both questions.
        let has_history = !self
            .services
            .customization
            .maintenance_evaluation
            .list_action_runs(Some(&rule_set.id), None, Some(1))
            .await?
            .is_empty();
        if has_history {
            return Err(AppError::Validation(format!(
                "\"{}\" has already run actions on your library, and deleting it would erase the \
                 record of what it did. Set its mode to disabled instead: the rule stops \
                 immediately and its history is kept.",
                rule_set.name
            )));
        }

        let active_candidates = self
            .count_active_maintenance_candidates(&rule_set.id)
            .await?;
        if active_candidates > 0 {
            return Err(AppError::Validation(format!(
                "\"{}\" is currently tracking {active_candidates} title(s), and deleting it would \
                 drop them without a trace. Set its mode to disabled instead, or clear the \
                 candidates first.",
                rule_set.name
            )));
        }

        self.services
            .customization
            .maintenance_rule_sets
            .delete_rule_set(rule_set_id)
            .await
    }

    /// Editor support: compile and check a draft without storing it. The
    /// throwaway id only satisfies the package-declaration check.
    pub async fn validate_maintenance_rule_source(
        &self,
        actor: &User,
        rego_source: &str,
    ) -> AppResult<ValidationResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let scratch_id = Id::new_rego_safe().0;
        let rewritten = rewrite_package_declaration(rego_source, &scratch_id);
        validate_maintenance_rule(&rewritten, &scratch_id)
            .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))
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

        let policy = self.resolve_preview_policy(request.matcher).await?;
        // Preview evaluates against real subjects and reports per-title
        // outcomes, so an ungated one is a person-fact oracle: ask "did user x
        // watch this" title by title and read the answers off the results. The
        // same bar authoring meets applies here, for a stored rule as much as
        // for a draft.
        self.require_person_fact_authority(actor, &policy.rego_source, &policy.id)
            .await?;

        let titles = self.select_preview_titles(&request.selection).await?;
        let libraries = self.maintenance_library_refs().await?;

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

        let matcher_content_hash = content_hash(&policy.rego_source);
        let rule_set_id = policy.id.clone();
        let engine = MaintenanceRulesEngine::build(std::slice::from_ref(&policy))
            .map_err(|e| AppError::Validation(format!("rule failed to compile: {e}")))?;
        let mut evaluator = engine.evaluator();

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
            };
            let input = build_title_input(
                evaluated_at,
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

            let evaluation = evaluator
                .evaluate(&input)
                .map_err(|e| AppError::Validation(format!("rule evaluation failed: {e}")))?;

            let (outcome, reason_codes, error) =
                match (evaluation.records.first(), evaluation.errors.first()) {
                    (Some(record), _) => (
                        Some(record.decision.outcome),
                        record.decision.reason_codes.clone(),
                        None,
                    ),
                    (None, Some(failure)) => (None, Vec::new(), Some(failure.message.clone())),
                    (None, None) => (
                        None,
                        Vec::new(),
                        Some("rule produced no decision".to_string()),
                    ),
                };

            results.push(MaintenancePreviewTitleResult {
                title_id: title.id,
                title_name: title.name,
                facet: title.facet,
                library_id: title.library_id,
                outcome,
                reason_codes,
                error,
            });
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
        let referenced = maintenance_referenced_facts(rego_source, rule_set_id)
            .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
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

        Err(AppError::Unauthorized(format!(
            "This rule reads facts about specific people ({}), so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
            person_facts.join(", ")
        )))
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
            .ok_or_else(|| AppError::NotFound(format!("maintenance rule set {id} not found")))
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
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "maintenance rule set {} has no revision {}",
                    rule_set.id, rule_set.current_revision_number
                ))
            })?;
        let action_spec = decode_action_spec(&revision)?;
        Ok(MaintenanceRuleSetDetail {
            rule_set,
            revision,
            action_spec,
        })
    }

    async fn resolve_preview_policy(
        &self,
        matcher: MaintenancePreviewMatcher,
    ) -> AppResult<MaintenancePolicy> {
        match matcher {
            MaintenancePreviewMatcher::Stored { rule_set_id } => {
                let rule_set = self.require_maintenance_rule_set(&rule_set_id).await?;
                let detail = self.load_maintenance_rule_detail(rule_set).await?;
                Ok(MaintenancePolicy {
                    id: detail.rule_set.id,
                    name: detail.rule_set.name,
                    rego_source: detail.revision.rego_source,
                })
            }
            MaintenancePreviewMatcher::Inline {
                rego_source,
                action_spec,
                grace_days,
            } => {
                let scratch_id = Id::new_rego_safe().0;
                let prepared =
                    prepare_matcher(&scratch_id, &rego_source, &action_spec, grace_days)?;
                Ok(MaintenancePolicy {
                    id: scratch_id,
                    name: "preview".to_string(),
                    rego_source: prepared.rego_source,
                })
            }
        }
    }

    async fn select_preview_titles(
        &self,
        selection: &MaintenancePreviewSelection,
    ) -> AppResult<Vec<Title>> {
        match selection {
            MaintenancePreviewSelection::Titles(title_ids) => {
                if title_ids.len() > MAINTENANCE_PREVIEW_MAX_TITLES {
                    return Err(AppError::Validation(format!(
                        "preview accepts at most {MAINTENANCE_PREVIEW_MAX_TITLES} titles, {} were requested",
                        title_ids.len()
                    )));
                }
                self.services.catalog.titles.get_by_ids(title_ids).await
            }
            MaintenancePreviewSelection::Library { library_id, limit } => {
                let limit = limit
                    .unwrap_or(MAINTENANCE_PREVIEW_DEFAULT_TITLES)
                    .clamp(1, MAINTENANCE_PREVIEW_MAX_TITLES);
                let mut titles = self
                    .services
                    .catalog
                    .titles
                    .list_for_libraries(None, std::slice::from_ref(library_id), None)
                    .await?;
                titles.truncate(limit);
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
    /// The registry half of tag-action validation.
    ///
    /// `MaintenanceActionSpec::validate` is static — shape, normalization, and
    /// the per-title ceiling — because a stored spec is re-validated on every
    /// read and must not need a database to be readable. Whether a label is
    /// *defined* is a live question, so it is asked here, once, when a revision
    /// is written, and asked again by the executor before it acts: a tag
    /// deleted in between holds the candidate rather than writing a label the
    /// assignment path would itself refuse.
    async fn require_registered_action_tags(
        &self,
        action_spec: &MaintenanceActionSpec,
    ) -> AppResult<()> {
        let labels = action_spec.parameters.tag_labels();
        if labels.is_empty() {
            return Ok(());
        }
        self.require_registered_title_tags(labels).await
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

/// A matcher that passed every authoring-time check, ready to persist.
struct PreparedMatcher {
    rego_source: String,
    content_hash: String,
    action_spec_json: String,
}

/// Rewrite the package to the system-assigned id, then run every check that
/// must hold before a revision is written: the matcher compiles and reads only
/// documented facts, the action is legal for a title-scoped rule, and the grace
/// period is a real duration.
fn prepare_matcher(
    rule_id: &str,
    rego_source: &str,
    action_spec: &MaintenanceActionSpec,
    grace_days: i64,
) -> AppResult<PreparedMatcher> {
    if grace_days < 0 {
        return Err(AppError::Validation(
            "grace period must be zero or more days".to_string(),
        ));
    }
    if grace_days > MAINTENANCE_MAX_GRACE_DAYS {
        return Err(AppError::Validation(format!(
            "grace period must be at most {MAINTENANCE_MAX_GRACE_DAYS} days (ten years)"
        )));
    }
    validate_title_scope_action(action_spec)?;

    let rewritten = rewrite_package_declaration(rego_source, rule_id);
    let validation = validate_maintenance_rule(&rewritten, rule_id)
        .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
    if !validation.valid {
        return Err(AppError::Validation(format_maintenance_validation_errors(
            &validation,
        )));
    }

    let action_spec_json = serde_json::to_string(action_spec)
        .map_err(|e| AppError::Validation(format!("maintenance action is not storable: {e}")))?;

    Ok(PreparedMatcher {
        content_hash: content_hash(&rewritten),
        rego_source: rewritten,
        action_spec_json,
    })
}

/// A title-scoped rule set spans both movie and show libraries, so its action
/// need only be legal for one of them. Which one applies to a given candidate
/// is decided per title from its facet at evaluation time; rejecting an action
/// here because it is movie-only would make it unusable in movie libraries.
///
/// Passing the descriptor's subject check is necessary but not sufficient: the
/// action also has to be one the title executor dispatches. The show-subject
/// `unmonitor_show_delete_existing_files` used to pass here and then hard-fail
/// at execution, burning its three attempts into a terminal `Failed` candidate,
/// so [`EXECUTABLE_TITLE_RULE_ACTIONS`] — the executor's own list — is the gate
/// on both sides.
pub(super) fn validate_title_scope_action(spec: &MaintenanceActionSpec) -> AppResult<()> {
    if !EXECUTABLE_TITLE_RULE_ACTIONS.contains(&spec.kind) {
        return Err(AppError::Validation(title_rule_action_not_executable(
            spec.kind,
        )));
    }

    if spec.validate(ActionSubjectKind::Movie).is_ok()
        || spec.validate(ActionSubjectKind::Show).is_ok()
    {
        return Ok(());
    }

    // Both failed; the movie error names the reason (schema version and
    // parameter shape are subject-independent, so it is the same either way).
    let error = spec
        .validate(ActionSubjectKind::Movie)
        .expect_err("action rejected for both movie and show subjects");
    Err(AppError::Validation(format!(
        "maintenance action is not valid for a title-scoped rule: {error}"
    )))
}

fn require_dormant_evaluation_mode(mode: Option<MaintenanceEvaluationMode>) -> AppResult<()> {
    match mode {
        None | Some(MaintenanceEvaluationMode::Disabled) => Ok(()),
        Some(_) => Err(AppError::Validation(
            "shadow and observe evaluation are not yet available; maintenance rules can only be saved as disabled".to_string(),
        )),
    }
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
        return Err(AppError::Validation(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn build_revision(
    rule_set_id: &str,
    revision_number: i64,
    prepared: &PreparedMatcher,
    grace_days: i64,
    actor: &User,
    created_at: chrono::DateTime<Utc>,
) -> MaintenanceRuleRevision {
    MaintenanceRuleRevision {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number,
        rego_source: prepared.rego_source.clone(),
        action_spec_json: prepared.action_spec_json.clone(),
        grace_days,
        matcher_content_hash: prepared.content_hash.clone(),
        created_by: Some(actor.id.clone()),
        created_at,
    }
}

/// Read the stored action back through the closed catalog. Stored JSON that no
/// longer deserializes is a repository fault, not a validation failure: it
/// means a spec was written by a build whose catalog this one does not have.
fn decode_action_spec(revision: &MaintenanceRuleRevision) -> AppResult<MaintenanceActionSpec> {
    serde_json::from_str(&revision.action_spec_json).map_err(|e| {
        AppError::Repository(format!(
            "maintenance rule revision {} has an unreadable action spec: {e}",
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
