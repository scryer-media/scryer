use super::*;
use crate::rules::builder_model::{BuilderFamily, BuilderModelChange};
use scryer_domain::RuleSet;
use scryer_rules::validation::{
    ValidationResult, retired_release_input_fields, validate_user_rule,
};

const RETIRED_GUIDE_FACTS_REASON: &str =
    "uses retired input.release.guide_facts; update the rule source before enabling it";

fn format_rule_validation_errors(validation: &ValidationResult) -> String {
    format!(
        "Rule validation failed:\n- {}",
        validation.errors.join("\n- ")
    )
}

/// Rewrites the package declaration to the system-assigned `id` and applies
/// the validation every new rule set passes before it is written.
fn validated_new_rule_source(rego_source: &str, id: &str) -> AppResult<String> {
    let rewritten_source = scryer_rules::rewrite_package_declaration(rego_source, id);

    if !retired_release_input_fields(&rewritten_source)
        .map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::rules::VALIDATION_FAILED,
                format!("rule validation failed: {error}"),
            )
        })?
        .is_empty()
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::INPUT_RETIRED,
            RETIRED_GUIDE_FACTS_REASON.to_string(),
        ));
    }

    // Validate the rewritten Rego source
    let validation = validate_user_rule(&rewritten_source, id).map_err(|e| {
        AppError::validation_refused(
            crate::error_reasons::rules::VALIDATION_FAILED,
            format!("rule validation failed: {e}"),
        )
    })?;
    if !validation.valid {
        return Err(AppError::validation_refused_with_diagnostics(
            crate::error_reasons::rules::SOURCE_INVALID,
            format_rule_validation_errors(&validation),
            validation.diagnostics,
        ));
    }
    Ok(rewritten_source)
}

/// One rule set [`AppUseCase::create_rule_sets_batch`] creates.
#[derive(Clone, Debug)]
pub struct NewRuleSet {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub applied_facets: Vec<MediaFacet>,
    pub priority: i32,
    pub enabled: bool,
}

/// A draft [`AppUseCase::create_rule_sets_batch_claiming_names`] writes only
/// while no rule set holds any of its claimed names.
#[derive(Clone, Debug)]
pub struct ClaimedNewRuleSet {
    pub rule_set: NewRuleSet,
    /// Names that, once any rule set or an earlier draft of the same batch
    /// holds one of them, skip this draft. The draft's own name is always one.
    pub claimed_names: Vec<String>,
}

/// What [`AppUseCase::create_rule_sets_batch_claiming_names`] did with one draft.
#[derive(Clone, Debug)]
pub enum ClaimedRuleSetOutcome {
    Created(RuleSet),
    /// Nothing was written: a rule set, or an earlier draft of the batch,
    /// already holds `name`.
    NameTaken {
        name: String,
    },
}

fn prepared_rule_set(draft: NewRuleSet) -> AppResult<RuleSet> {
    let id = Id::new_rego_safe().0;
    let rego_source = validated_new_rule_source(&draft.rego_source, &id)?;
    let now = Utc::now();
    Ok(RuleSet {
        id,
        name: draft.name,
        description: draft.description,
        rego_source,
        enabled: draft.enabled,
        priority: draft.priority,
        evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
        exclusive_group: None,
        disabled_reason_code: None,
        disabled_reason: None,
        applied_facets: draft.applied_facets,
        created_at: now,
        updated_at: now,
        is_managed: false,
        managed_key: None,
        managed_tag_filter: None,
    })
}

impl AppUseCase {
    /// Creates several rule sets as [`Self::create_rule_set`] would, one result
    /// per draft in order. Every source passes the same validation first.
    /// The valid rule sets and their history are written in one transaction
    /// under one acquisition of the rule mutation lock; only when that write
    /// fails are they retried one row at a time, each row with its history in
    /// its own transaction. The engine is rebuilt once, and only when an
    /// enabled rule set was written: it is built from the enabled rule sets
    /// alone, so disabled ones cannot change it.
    pub async fn create_rule_sets_batch(
        &self,
        actor: &User,
        drafts: Vec<NewRuleSet>,
    ) -> AppResult<Vec<AppResult<RuleSet>>> {
        let drafts = drafts
            .into_iter()
            .map(|rule_set| (rule_set, None))
            .collect();
        Ok(self
            .create_rule_sets_batch_inner(actor, drafts)
            .await?
            .into_iter()
            .map(|result| match result {
                Ok(ClaimedRuleSetOutcome::Created(rule_set)) => Ok(rule_set),
                // Unclaimed drafts are never skipped.
                Ok(ClaimedRuleSetOutcome::NameTaken { name }) => {
                    Err(AppError::Repository(format!("rule set {name} was skipped")))
                }
                Err(error) => Err(error),
            })
            .collect())
    }

    /// [`Self::create_rule_sets_batch`] that skips every draft whose claimed
    /// names a rule set already holds, or an earlier draft of the batch
    /// claimed. Names are read once, after the rule mutation lock is taken,
    /// so no concurrent create can slip in between the check and the write.
    pub async fn create_rule_sets_batch_claiming_names(
        &self,
        actor: &User,
        drafts: Vec<ClaimedNewRuleSet>,
    ) -> AppResult<Vec<AppResult<ClaimedRuleSetOutcome>>> {
        let drafts = drafts
            .into_iter()
            .map(|draft| (draft.rule_set, Some(draft.claimed_names)))
            .collect();
        self.create_rule_sets_batch_inner(actor, drafts).await
    }

    async fn create_rule_sets_batch_inner(
        &self,
        actor: &User,
        drafts: Vec<(NewRuleSet, Option<Vec<String>>)>,
    ) -> AppResult<Vec<AppResult<ClaimedRuleSetOutcome>>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let (drafts, claims): (Vec<NewRuleSet>, Vec<Option<Vec<String>>>) =
            drafts.into_iter().unzip();
        let any_claims = claims.iter().any(Option::is_some);
        // Validation compiles every module; keep it off the async workers.
        let mut results: Vec<AppResult<ClaimedRuleSetOutcome>> =
            tokio::task::spawn_blocking(move || {
                drafts
                    .into_iter()
                    .map(|draft| prepared_rule_set(draft).map(ClaimedRuleSetOutcome::Created))
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|error| AppError::Repository(format!("rule validation failed: {error}")))?;
        let repository = &self.services.customization.rule_sets;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        if any_claims {
            let mut taken: std::collections::HashSet<String> = repository
                .list_rule_set_names()
                .await?
                .into_iter()
                .collect();
            // `taken` holds only names a rule set holds or this batch writes, so
            // a skip always reports a name the operator can find.
            for (result, claim) in results.iter_mut().zip(claims) {
                let Ok(ClaimedRuleSetOutcome::Created(rule_set)) = result.as_ref() else {
                    continue;
                };
                let held = claim.as_ref().and_then(|claim| {
                    claim
                        .iter()
                        .chain(std::iter::once(&rule_set.name))
                        .find(|name| taken.contains(*name))
                        .cloned()
                });
                match held {
                    Some(name) => *result = Ok(ClaimedRuleSetOutcome::NameTaken { name }),
                    None => {
                        taken.insert(rule_set.name.clone());
                    }
                }
            }
        }
        let valid: Vec<RuleSet> = results
            .iter()
            .filter_map(|result| match result {
                Ok(ClaimedRuleSetOutcome::Created(rule_set)) => Some(rule_set.clone()),
                _ => None,
            })
            .collect();
        if valid.is_empty() {
            return Ok(results);
        }
        let history_of = |rule_set: &RuleSet| RuleSetHistoryChange {
            rule_set_id: rule_set.id.clone(),
            action: "created".to_string(),
            rego_source: Some(rule_set.rego_source.clone()),
            actor_id: Some(actor.id.clone()),
        };
        let history: Vec<RuleSetHistoryChange> = valid.iter().map(history_of).collect();
        let mut wrote_enabled = false;
        match repository.create_rule_sets(&valid, &history).await {
            Ok(()) => wrote_enabled = valid.iter().any(|rule_set| rule_set.enabled),
            Err(batch_error) => {
                tracing::warn!(
                    rule_sets = valid.len(),
                    reason = batch_error.reason().unwrap_or("none"),
                    error = %batch_error,
                    "batched rule set write failed; writing rule sets one at a time"
                );
                // The batch rolled back as a whole; write what can be written,
                // each row together with its history.
                for result in &mut results {
                    let Ok(ClaimedRuleSetOutcome::Created(rule_set)) = result.as_ref() else {
                        continue;
                    };
                    match repository
                        .create_rule_sets(std::slice::from_ref(rule_set), &[history_of(rule_set)])
                        .await
                    {
                        Ok(()) => wrote_enabled |= rule_set.enabled,
                        Err(error) => *result = Err(error),
                    }
                }
            }
        }
        if wrote_enabled {
            self.rebuild_user_rules_engine_unlocked().await?;
        }
        Ok(results)
    }

    pub async fn list_rule_sets(&self, actor: &User) -> AppResult<Vec<RuleSet>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.rule_sets.list_rule_sets().await
    }

    pub async fn get_rule_set(&self, actor: &User, id: &str) -> AppResult<Option<RuleSet>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.rule_sets.get_rule_set(id).await
    }

    /// The simple builder's stored document for one rule set, if any.
    pub async fn get_rule_set_builder_model(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<serde_json::Value>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let stored = self
            .services
            .customization
            .rule_sets
            .get_rule_set_builder_model(id)
            .await?;
        Ok(crate::rules::builder_model::stored_builder_model(
            stored.as_deref(),
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the rule-set creation contract field for field"
    )]
    pub async fn create_rule_set(
        &self,
        actor: &User,
        name: String,
        description: String,
        rego_source: String,
        applied_facets: Vec<MediaFacet>,
        priority: i32,
        enabled: Option<bool>,
    ) -> AppResult<RuleSet> {
        self.create_rule_set_with_builder_model(
            actor,
            name,
            description,
            rego_source,
            applied_facets,
            priority,
            enabled,
            BuilderModelChange::Keep,
        )
        .await
    }

    /// [`Self::create_rule_set`] that also stores the simple builder's
    /// document. `Keep` and `Clear` both store none. The document never
    /// changes what the rule's source validates or scores to.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the rule-set creation contract field for field"
    )]
    pub async fn create_rule_set_with_builder_model(
        &self,
        actor: &User,
        name: String,
        description: String,
        rego_source: String,
        applied_facets: Vec<MediaFacet>,
        priority: i32,
        enabled: Option<bool>,
        builder_model: BuilderModelChange,
    ) -> AppResult<RuleSet> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let builder_model = crate::rules::builder_model::resolved_builder_model(
            &builder_model,
            None,
            BuilderFamily::Scoring,
        )?;

        let id = Id::new_rego_safe().0;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;

        let rewritten_source = validated_new_rule_source(&rego_source, &id)?;

        let now = Utc::now();
        let rule_set = RuleSet {
            id,
            name,
            description,
            rego_source: rewritten_source.clone(),
            enabled: enabled.unwrap_or(true),
            priority,
            evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
            exclusive_group: None,
            disabled_reason_code: None,
            disabled_reason: None,
            applied_facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
            managed_tag_filter: None,
        };

        match builder_model.as_deref() {
            None => {
                self.services
                    .customization
                    .rule_sets
                    .create_rule_set(&rule_set)
                    .await?
            }
            Some(builder_model) => {
                self.services
                    .customization
                    .rule_sets
                    .create_rule_set_with_builder_model(&rule_set, Some(builder_model))
                    .await?
            }
        }
        self.services
            .customization
            .rule_sets
            .record_rule_set_history(
                &rule_set.id,
                "created",
                Some(&rewritten_source),
                Some(&actor.id),
            )
            .await?;

        self.rebuild_user_rules_engine_unlocked().await?;
        Ok(rule_set)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the application boundary mirrors the editable rule-set fields explicitly"
    )]
    pub async fn update_rule_set(
        &self,
        actor: &User,
        id: String,
        name: Option<String>,
        description: Option<String>,
        rego_source: Option<String>,
        applied_facets: Option<Vec<MediaFacet>>,
        priority: Option<i32>,
        managed_tag_filter: Option<Vec<String>>,
    ) -> AppResult<RuleSet> {
        self.update_rule_set_with_builder_model(
            actor,
            id,
            name,
            description,
            rego_source,
            applied_facets,
            priority,
            managed_tag_filter,
            BuilderModelChange::Keep,
        )
        .await
    }

    /// [`Self::update_rule_set`] that also keeps, clears or replaces the
    /// stored builder document. Keeping it takes the write path a rule edit
    /// has always taken.
    #[expect(
        clippy::too_many_arguments,
        reason = "the application boundary mirrors the editable rule-set fields explicitly"
    )]
    pub async fn update_rule_set_with_builder_model(
        &self,
        actor: &User,
        id: String,
        name: Option<String>,
        description: Option<String>,
        rego_source: Option<String>,
        applied_facets: Option<Vec<MediaFacet>>,
        priority: Option<i32>,
        managed_tag_filter: Option<Vec<String>>,
        builder_model: BuilderModelChange,
    ) -> AppResult<RuleSet> {
        let edits_builder_model = builder_model != BuilderModelChange::Keep;
        let edits_authored_fields = name.is_some()
            || description.is_some()
            || rego_source.is_some()
            || applied_facets.is_some()
            || priority.is_some()
            || edits_builder_model;
        if !edits_authored_fields && managed_tag_filter.is_none() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::SET_FIELD_REQUIRED,
                "at least one rule set field must be provided",
            ));
        }
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let mut rule_set = self
            .services
            .customization
            .rule_sets
            .get_rule_set(&id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::rules::SET_NOT_FOUND,
                    format!("rule set {id} not found"),
                )
            })?;

        let tracked_pack = self
            .services
            .customization
            .rule_sets
            .find_rule_pack_installation_by_rule_set_id(&rule_set.id)
            .await?;

        if tracked_pack.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TRACKED_COMMUNITY_PACK,
                "This rule is tracked by a community pack. Change its settings through the tracked pack so membership revisions remain atomic, or copy it to customize its content.",
            ));
        }

        if rule_set.is_managed {
            if edits_authored_fields {
                return Err(AppError::validation_refused(
                    crate::error_reasons::rules::MANAGED_CONVENIENCE_SETTING,
                    "This rule is managed by a convenience setting. Change the setting instead of editing the rule directly.",
                ));
            }
            if managed_tag_filter.is_some() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::rules::MANAGED_LOCALE_FILTER_FORBIDDEN,
                    "Managed TRaSH Guides locale packs use their predefined locale policy and cannot be filtered.",
                ));
            }
        } else if managed_tag_filter.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS,
                "A tag filter only applies to managed rule sets.",
            ));
        }

        let previous_source = rule_set.rego_source.clone();
        let mut repaired_retired_source = false;
        if let Some(new_source) = &rego_source {
            // Rewrite the package declaration to match the existing rule ID.
            let rewritten = scryer_rules::rewrite_package_declaration(new_source, &rule_set.id);
            let retired = retired_release_input_fields(&rewritten).map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::rules::VALIDATION_FAILED,
                    format!("rule validation failed: {error}"),
                )
            })?;
            if !retired.is_empty() {
                rule_set.rego_source = rewritten;
                rule_set.enabled = false;
                rule_set.disabled_reason = Some(RETIRED_GUIDE_FACTS_REASON.to_string());
                rule_set.disabled_reason_code =
                    Some(crate::error_reasons::rules::INPUT_RETIRED.to_string());
            } else {
                let validation = validate_user_rule(&rewritten, &rule_set.id).map_err(|e| {
                    AppError::validation_refused(
                        crate::error_reasons::rules::VALIDATION_FAILED,
                        format!("rule validation failed: {e}"),
                    )
                })?;
                if !validation.valid {
                    return Err(AppError::validation_refused_with_diagnostics(
                        crate::error_reasons::rules::SOURCE_INVALID,
                        format_rule_validation_errors(&validation),
                        validation.diagnostics,
                    ));
                }
                rule_set.rego_source = rewritten;
                repaired_retired_source = true;
            }
        }
        if let Some(n) = name {
            rule_set.name = n;
        }
        if let Some(d) = description {
            rule_set.description = d;
        }
        if let Some(f) = applied_facets {
            rule_set.applied_facets = f;
        }
        if let Some(p) = priority {
            rule_set.priority = p;
        }
        if repaired_retired_source {
            rule_set.disabled_reason = None;
            rule_set.disabled_reason_code = None;
        }
        rule_set.updated_at = Utc::now();
        // A managed rule never reaches here with a builder change: editing
        // one is refused above as an authored-field edit, and so is a source
        // edit, so its omitted field stays a keep.
        let builder_model =
            builder_model.after_source_edit(&previous_source, &rule_set.rego_source);
        let builder_model = match &builder_model {
            BuilderModelChange::Keep => None,
            change => Some(crate::rules::builder_model::resolved_builder_model(
                change,
                None,
                BuilderFamily::Scoring,
            )?),
        };
        let engine = self.prospective_engine(&[rule_set.clone()], &[]).await?;

        match builder_model {
            None => {
                self.services
                    .customization
                    .rule_sets
                    .update_rule_set(&rule_set)
                    .await?
            }
            Some(builder_model) => {
                self.services
                    .customization
                    .rule_sets
                    .update_rule_set_with_builder_model(&rule_set, builder_model.as_deref())
                    .await?
            }
        }
        self.services
            .customization
            .rule_sets
            .record_rule_set_history(
                &rule_set.id,
                "updated",
                Some(&rule_set.rego_source),
                Some(&actor.id),
            )
            .await?;

        self.swap_user_rules_engine(engine);
        Ok(rule_set)
    }

    pub async fn delete_rule_set(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;

        if self
            .services
            .customization
            .rule_sets
            .find_rule_pack_installation_by_rule_set_id(id)
            .await?
            .is_some()
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TRACKED_COMMUNITY_PACK,
                "This rule is tracked by a community pack. Uninstall the pack or copy the rule to customize it.",
            ));
        }

        if let Some(rule_set) = self
            .services
            .customization
            .rule_sets
            .get_rule_set(id)
            .await?
            && rule_set.is_managed
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::MANAGED_CONVENIENCE_SETTING,
                "This rule is managed by a convenience setting. Remove the setting instead of deleting the rule directly.",
            ));
        }

        self.services
            .customization
            .rule_sets
            .delete_rule_set(id)
            .await?;
        self.services
            .customization
            .rule_sets
            .record_rule_set_history(id, "deleted", None, Some(&actor.id))
            .await?;

        self.rebuild_user_rules_engine_unlocked().await?;
        Ok(())
    }

    pub async fn toggle_rule_set(
        &self,
        actor: &User,
        id: &str,
        enabled: bool,
    ) -> AppResult<RuleSet> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;

        let mut rule_set = self
            .services
            .customization
            .rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::rules::SET_NOT_FOUND,
                    format!("rule set {id} not found"),
                )
            })?;

        if self
            .services
            .customization
            .rule_sets
            .find_rule_pack_installation_by_rule_set_id(&rule_set.id)
            .await?
            .is_some()
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TRACKED_COMMUNITY_PACK,
                "This rule is tracked by a community pack. Change its settings through the tracked pack so membership revisions remain atomic.",
            ));
        }

        if enabled
            && !rule_set.enabled
            && !retired_release_input_fields(&rule_set.rego_source)
                .map_err(|error| {
                    AppError::validation_refused(
                        crate::error_reasons::rules::VALIDATION_FAILED,
                        format!("rule validation failed: {error}"),
                    )
                })?
                .is_empty()
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::INPUT_RETIRED,
                RETIRED_GUIDE_FACTS_REASON.to_string(),
            ));
        }

        rule_set.enabled = enabled;
        if enabled {
            rule_set.disabled_reason = None;
            rule_set.disabled_reason_code = None;
        }
        rule_set.updated_at = Utc::now();
        let engine = self.prospective_engine(&[rule_set.clone()], &[]).await?;

        self.services
            .customization
            .rule_sets
            .update_rule_set(&rule_set)
            .await?;
        let action = if enabled { "enabled" } else { "disabled" };
        self.services
            .customization
            .rule_sets
            .record_rule_set_history(&rule_set.id, action, None, Some(&actor.id))
            .await?;

        self.swap_user_rules_engine(engine);
        Ok(rule_set)
    }

    /// Translates pasted Arr custom-format JSON into Rego for the rule editor.
    /// Nothing is saved.
    pub async fn translate_arr_custom_formats(
        &self,
        actor: &User,
        source: crate::arr_custom_format::ArrSourceApplication,
        json: String,
        scores: std::collections::HashMap<String, i32>,
    ) -> AppResult<crate::arr_custom_format::ArrCustomFormatTranslation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        // A large paste parses and compiles for a while; keep it off the
        // async workers.
        tokio::task::spawn_blocking(move || {
            crate::arr_custom_format::translate_arr_custom_formats(source, &json, &scores)
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("custom-format translation failed: {error}"))
        })?
        .map_err(|refusal| refusal.into_app_error())
    }

    pub async fn validate_rule_set(
        &self,
        actor: &User,
        rego_source: &str,
        rule_set_id: &str,
    ) -> AppResult<ValidationResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        // Rewrite the package declaration so validation works regardless of
        // what the user typed.
        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, rule_set_id);
        let validation = validate_user_rule(&rewritten, rule_set_id).map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::rules::VALIDATION_ERROR,
                format!("rule validation error: {e}"),
            )
        })?;
        if !validation.valid {
            return Ok(validation);
        }
        let retired = retired_release_input_fields(&rewritten).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::rules::VALIDATION_ERROR,
                format!("rule validation error: {error}"),
            )
        })?;
        if !retired.is_empty() {
            return Ok(ValidationResult::invalid(format!(
                "uses retired release input field(s): {}",
                retired.join(", ")
            )));
        }
        let Some(rule_set) = self
            .services
            .customization
            .rule_sets
            .get_rule_set(rule_set_id)
            .await?
        else {
            return Ok(validation);
        };
        if rule_set.evaluation_phase != scryer_domain::RuleEvaluationPhase::Baseline {
            return Ok(validation);
        }
        let additional_rule_ids = self
            .services
            .customization
            .rule_sets
            .list_enabled_rule_sets()
            .await?
            .into_iter()
            .filter(|rule| {
                rule.evaluation_phase == scryer_domain::RuleEvaluationPhase::Additional
                    && rule.id != rule_set.id
            })
            .map(|rule| rule.id)
            .collect::<Vec<_>>();
        match scryer_rules::validation::validate_baseline_dependencies(
            &rewritten,
            &additional_rule_ids,
        ) {
            Ok(()) => Ok(validation),
            Err(error) => Ok(ValidationResult::invalid(error)),
        }
    }

    // ── Convenience settings ───────────────────────────────────────────────
    /// Set or remove a title-level required audio language override.
    ///
    /// `languages = Some(vec![...])` stores an explicit title override.
    /// `languages = Some(vec![])` stores an explicit "no required language" override.
    /// `languages = None` removes the override and restores inheritance.
    pub async fn set_title_required_audio(
        &self,
        actor: &User,
        title_id: &str,
        facet: &str,
        languages: Option<Vec<String>>,
    ) -> AppResult<()> {
        let _ = facet;
        self.set_title_required_audio_override(actor, title_id, languages)
            .await
    }

    pub async fn migrate_legacy_persona_preferences(&self) -> AppResult<()> {
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        const SYSTEM_SCOPE: &str = "system";

        let mut existing_rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        let profiles = self
            .services
            .config
            .quality_profiles
            .list_quality_profiles(SYSTEM_SCOPE, None)
            .await?;

        for profile in &profiles {
            if profile.criteria.prefer_dual_audio {
                let marker = format!("legacy-prefer-dual-audio:profile:{}", profile.id);
                self.ensure_migrated_rule(
                    &mut existing_rules,
                    &marker,
                    &format!("Migrated: Prefer Multi-Audio ({})", profile.name),
                    "Auto-migrated from the deprecated multi-audio preference toggle.",
                    &generate_profile_prefer_multi_audio_rego(&profile.id),
                    Vec::new(),
                )
                .await?;
            }

            if profile.criteria.scoring_persona == ScoringPersona::Audiophile {
                if !profile.criteria.atmos_preferred {
                    let marker = format!("legacy-atmos-disabled:profile:{}", profile.id);
                    self.ensure_migrated_rule(
                        &mut existing_rules,
                        &marker,
                        &format!("Migrated: Disable Atmos Persona Bias ({})", profile.name),
                        "Auto-migrated from the deprecated Atmos preference toggle.",
                        &generate_profile_cancel_atmos_rego(&profile.id, 150, 30),
                        Vec::new(),
                    )
                    .await?;
                }
            } else if profile.criteria.atmos_preferred {
                let (bonus, penalty) = legacy_atmos_rule_values(&profile.criteria.scoring_persona);
                let marker = format!("legacy-atmos-preferred:profile:{}", profile.id);
                self.ensure_migrated_rule(
                    &mut existing_rules,
                    &marker,
                    &format!("Migrated: Prefer Atmos ({})", profile.name),
                    "Auto-migrated from the deprecated Atmos preference toggle.",
                    &generate_profile_prefer_atmos_rego(&profile.id, bonus, penalty),
                    Vec::new(),
                )
                .await?;
            }
        }

        let legacy_dual_managed = self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:prefer-dual-audio:")
            .await?;
        for rule_set in &legacy_dual_managed {
            let Some(managed_key) = rule_set.managed_key.as_deref() else {
                continue;
            };

            let marker = format!("legacy-convenience-prefer-dual-audio:{managed_key}");
            self.ensure_migrated_rule(
                &mut existing_rules,
                &marker,
                &format!("Migrated: {}", rule_set.name),
                "Auto-migrated from the deprecated managed convenience rule.",
                &generate_prefer_multi_audio_rego(&marker),
                rule_set.applied_facets.clone(),
            )
            .await?;
        }

        for rule_set in legacy_dual_managed {
            self.services
                .customization
                .rule_sets
                .delete_rule_set(&rule_set.id)
                .await?;
        }

        for rule_set in existing_rules {
            if is_legacy_prefer_dual_audio_cleanup_candidate(&rule_set) {
                self.services
                    .customization
                    .rule_sets
                    .delete_rule_set(&rule_set.id)
                    .await?;
            }
        }

        Ok(())
    }
    async fn ensure_migrated_rule(
        &self,
        existing_rules: &mut Vec<RuleSet>,
        migration_key: &str,
        name: &str,
        description_prefix: &str,
        rego_source: &str,
        applied_facets: Vec<MediaFacet>,
    ) -> AppResult<()> {
        if existing_rules.iter().any(|rule| {
            rule.description.contains(migration_key) || rule.rego_source.contains(migration_key)
        }) {
            return Ok(());
        }

        let now = Utc::now();
        let id = Id::new_rego_safe().0;
        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, &id);
        let rule_set = RuleSet {
            id,
            name: name.to_string(),
            description: format!("{description_prefix} [scryer-migration:{migration_key}]"),
            rego_source: rewritten,
            enabled: true,
            priority: 0,
            evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
            exclusive_group: None,
            disabled_reason_code: None,
            disabled_reason: None,
            applied_facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
            managed_tag_filter: None,
        };
        self.services
            .customization
            .rule_sets
            .create_rule_set(&rule_set)
            .await?;
        existing_rules.push(rule_set);
        Ok(())
    }

    pub async fn rebuild_user_rules_engine(&self) -> AppResult<()> {
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        self.rebuild_user_rules_engine_unlocked().await
    }

    async fn rebuild_user_rules_engine_unlocked(&self) -> AppResult<()> {
        let enabled = self
            .services
            .customization
            .rule_sets
            .list_enabled_rule_sets()
            .await?;

        let engine = self.prepare_user_rules_engine(enabled).await?;
        self.swap_user_rules_engine(engine);
        Ok(())
    }

    pub(crate) async fn prepare_user_rules_engine(
        &self,
        rule_sets: Vec<RuleSet>,
    ) -> AppResult<scryer_rules::UserRulesEngine> {
        let plugin_policies = self
            .services
            .integrations
            .plugin_provider
            .available()
            .map(|provider| provider.scoring_policies())
            .unwrap_or_default();
        Self::build_user_rules_engine(rule_sets, plugin_policies)
    }

    /// Build an engine from an already captured policy snapshot. This is pure
    /// CPU work so previews can call it from `spawn_blocking` after releasing
    /// the rule mutation lock.
    pub(crate) fn build_user_rules_engine(
        rule_sets: Vec<RuleSet>,
        plugin_policies: Vec<scryer_rules::UserPolicy>,
    ) -> AppResult<scryer_rules::UserRulesEngine> {
        Self::build_user_rules_engine_for_purpose(
            rule_sets,
            plugin_policies,
            super::metrics::Purpose::Live,
        )
    }

    pub(crate) fn build_user_rules_engine_for_purpose(
        rule_sets: Vec<RuleSet>,
        plugin_policies: Vec<scryer_rules::UserPolicy>,
        purpose: super::metrics::Purpose,
    ) -> AppResult<scryer_rules::UserRulesEngine> {
        let mut timer = super::metrics::StageTimer::new("engine_build", purpose);
        let result = Self::build_observed_user_rules_engine(rule_sets, plugin_policies, purpose);
        if result.is_err() {
            timer.outcome("error");
        }
        result
    }

    fn build_observed_user_rules_engine(
        mut rule_sets: Vec<RuleSet>,
        plugin_policies: Vec<scryer_rules::UserPolicy>,
        purpose: super::metrics::Purpose,
    ) -> AppResult<scryer_rules::UserRulesEngine> {
        let mut exclusive_groups = std::collections::BTreeMap::<String, Vec<String>>::new();
        for rule_set in rule_sets.iter().filter(|rule_set| rule_set.enabled) {
            if let Some(group) = rule_set.exclusive_group.as_ref() {
                exclusive_groups
                    .entry(group.clone())
                    .or_default()
                    .push(rule_set.name.clone());
            }
        }
        if let Some((group, members)) = exclusive_groups
            .into_iter()
            .find(|(_, members)| members.len() > 1)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP,
                format!(
                    "only one enabled rule may use exclusive group {group}: {}",
                    members.join(", ")
                ),
            ));
        }
        rule_sets.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.name.cmp(&right.name))
        });
        let baseline_ids = rule_sets
            .iter()
            .filter(|rule| {
                rule.enabled
                    && rule.evaluation_phase == scryer_domain::RuleEvaluationPhase::Baseline
            })
            .map(|rule| rule.id.clone())
            .collect();
        let tag_filters = rule_sets
            .iter()
            .filter(|rule| rule.enabled)
            .filter_map(|rule| {
                rule.managed_tag_filter
                    .as_ref()
                    .filter(|tags| !tags.is_empty())
                    .map(|tags| (rule.id.clone(), tags.clone()))
            })
            .collect();
        let mut policies: Vec<scryer_rules::UserPolicy> = rule_sets
            .into_iter()
            .filter(|rule| rule.enabled)
            .map(|rs| scryer_rules::UserPolicy {
                id: rs.id,
                name: rs.name,
                rego_source: rs.rego_source,
                origin: if rs.is_managed {
                    scryer_rules::PolicyOrigin::System
                } else {
                    scryer_rules::PolicyOrigin::User
                },
                applied_facets: rs
                    .applied_facets
                    .iter()
                    .map(|f| format!("{:?}", f).to_lowercase())
                    .collect(),
            })
            .collect();

        let user_count = policies.len();

        // Append scoring policies from loaded WASM plugins.
        // Rewrite package declarations so the Rego package path matches the
        // system-assigned ID, same as we do for user-authored rules.
        if !plugin_policies.is_empty() {
            tracing::info!(
                plugin_policy_count = plugin_policies.len(),
                "including plugin-supplied scoring policies"
            );
            for mut p in plugin_policies {
                p.rego_source = scryer_rules::rewrite_package_declaration(&p.rego_source, &p.id);
                policies.push(p);
            }
        }

        let engine = scryer_rules::UserRulesEngine::build_observed(
            &policies,
            &baseline_ids,
            &tag_filters,
            Some(std::sync::Arc::new(super::metrics::RulesObserver(purpose))),
        )
        .map_err(|e| {
            AppError::validation_refused(
                crate::error_reasons::rules::FAILED_BUILD_RULES_ENGINE,
                format!("failed to build rules engine: {e}"),
            )
        })?;

        tracing::info!(
            user_rule_count = user_count,
            total_rule_count = policies.len(),
            "user rules engine prepared"
        );
        Ok(engine)
    }

    pub(crate) fn swap_user_rules_engine(&self, engine: scryer_rules::UserRulesEngine) {
        let reads_clock = engine.reads_clock();
        let lock = &self.services.customization.user_rules;
        let mut guard = lock
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = engine;
        lock.clear_poison();
        drop(guard);
        // After the engine is in place: a bar derived against the old engine
        // holds a ticket from before this reset and cannot be installed.
        self.runtime
            .acquisition
            .landed_bar_memo
            .reset_for_engine(reads_clock);
        // Format scores come from the rules, so the format-cutoff half of the
        // walk's target set follows the engine.
        self.runtime.acquisition.target_cache.mark_cutoff();
    }
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn generate_profile_prefer_multi_audio_rego(profile_id: &str) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-prefer-dual-audio:profile:{profile_id}\n\nscore_entry[\"migrated_prefer_multi_audio\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_dual_audio\n}}\n\nscore_entry[\"migrated_prefer_multi_audio_file\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_dual_audio\n    input.file != null\n    input.file.has_multiaudio\n}}\n"
    )
}

fn generate_prefer_multi_audio_rego(migration_key: &str) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:{migration_key}\n\nscore_entry[\"migrated_prefer_multi_audio\"] := 200 if {{\n    input.release.is_dual_audio\n}}\n\nscore_entry[\"migrated_prefer_multi_audio_file\"] := 200 if {{\n    not input.release.is_dual_audio\n    input.file != null\n    input.file.has_multiaudio\n}}\n"
    )
}

fn generate_profile_prefer_atmos_rego(profile_id: &str, bonus: i32, penalty: i32) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-atmos-preferred:profile:{profile_id}\n\nscore_entry[\"migrated_atmos_match\"] := {bonus} if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_atmos\n}}\n\nscore_entry[\"migrated_atmos_missing\"] := {penalty} if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_atmos\n}}\n"
    )
}

fn generate_profile_cancel_atmos_rego(
    profile_id: &str,
    match_penalty: i32,
    missing_bonus: i32,
) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-atmos-disabled:profile:{profile_id}\n\nscore_entry[\"migrated_atmos_cancel_match\"] := -{match_penalty} if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_atmos\n}}\n\nscore_entry[\"migrated_atmos_cancel_missing\"] := {missing_bonus} if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_atmos\n}}\n"
    )
}

fn legacy_atmos_rule_values(persona: &ScoringPersona) -> (i32, i32) {
    match persona {
        ScoringPersona::Balanced => (100, -20),
        ScoringPersona::Audiophile => (150, -30),
        ScoringPersona::Efficient => (40, -5),
        ScoringPersona::Compatible => (50, -10),
    }
}

fn is_legacy_prefer_dual_audio_cleanup_candidate(rule_set: &RuleSet) -> bool {
    let has_legacy_marker = rule_set.description.contains("legacy-prefer-dual-audio:")
        || rule_set.rego_source.contains("legacy-prefer-dual-audio:");
    let is_migrated_rule = rule_set
        .description
        .contains("scryer-migration:legacy-prefer-dual-audio:")
        || rule_set
            .rego_source
            .contains("scryer-migration:legacy-prefer-dual-audio:");

    has_legacy_marker && !is_migrated_rule
}

#[cfg(test)]
pub(crate) mod tests {
    include!("tracked_pack_workflow_tests.rs");
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullReleaseAttemptRepository, NullShowRepository, NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestQualityProfileRepo {
        profiles: Vec<QualityProfile>,
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl QualityProfileRepository for TestQualityProfileRepo {
        async fn list_quality_profiles(
            &self,
            _scope: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(self.profiles.clone())
        }

        async fn replace_quality_profiles(
            &self,
            _scope: &str,
            _scope_id: Option<String>,
            _profiles: Vec<QualityProfile>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    pub(crate) struct TestRuleSetRepo {
        rules: Mutex<Vec<RuleSet>>,
        packs: Mutex<Vec<scryer_domain::RulePackInstallation>>,
        fail_pack_apply: std::sync::atomic::AtomicBool,
        builder_models: Mutex<std::collections::HashMap<String, String>>,
        /// A batched write holding a rule set of one of these names fails whole.
        rejected_batch_names: std::sync::Mutex<std::collections::HashSet<String>>,
    }

    impl TestRuleSetRepo {
        pub(crate) fn new(rules: Vec<RuleSet>) -> Self {
            Self {
                rules: Mutex::new(rules),
                packs: Mutex::new(Vec::new()),
                fail_pack_apply: std::sync::atomic::AtomicBool::new(false),
                builder_models: Mutex::new(std::collections::HashMap::new()),
                rejected_batch_names: std::sync::Mutex::new(std::collections::HashSet::new()),
            }
        }

        async fn store_builder_model(&self, id: &str, builder_model: Option<&str>) {
            let mut models = self.builder_models.lock().await;
            match builder_model {
                Some(model) => {
                    models.insert(id.to_string(), model.to_string());
                }
                None => {
                    models.remove(id);
                }
            }
        }

        pub(crate) async fn rules_snapshot(&self) -> Vec<RuleSet> {
            self.rules.lock().await.clone()
        }
    }

    #[async_trait]
    impl RuleSetRepository for TestRuleSetRepo {
        async fn list_rule_pack_installations(
            &self,
        ) -> AppResult<Vec<scryer_domain::RulePackInstallation>> {
            Ok(self.packs.lock().await.clone())
        }
        async fn get_rule_pack_installation(
            &self,
            id: &str,
        ) -> AppResult<Option<scryer_domain::RulePackInstallation>> {
            Ok(self
                .packs
                .lock()
                .await
                .iter()
                .find(|pack| pack.pack_id == id)
                .cloned())
        }
        async fn find_rule_pack_installation_by_rule_set_id(
            &self,
            id: &str,
        ) -> AppResult<Option<scryer_domain::RulePackInstallation>> {
            Ok(self
                .packs
                .lock()
                .await
                .iter()
                .find(|pack| pack.members.iter().any(|member| member.rule_set_id == id))
                .cloned())
        }
        async fn apply_rule_pack_installation(
            &self,
            pack: &scryer_domain::RulePackInstallation,
            revision: Option<i64>,
            changed: &[RuleSet],
            _history: &[RuleSetHistoryChange],
        ) -> AppResult<bool> {
            if self
                .fail_pack_apply
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(AppError::Repository("injected pack commit failure".into()));
            }
            let mut packs = self.packs.lock().await;
            if packs
                .iter()
                .find(|old| old.pack_id == pack.pack_id)
                .map(|old| old.revision)
                != revision
            {
                return Ok(false);
            }
            let mut rules = self.rules.lock().await;
            rules.retain(|rule| !changed.iter().any(|new| new.id == rule.id));
            rules.extend_from_slice(changed);
            packs.retain(|old| old.pack_id != pack.pack_id);
            packs.push(pack.clone());
            Ok(true)
        }
        async fn uninstall_rule_pack(
            &self,
            id: &str,
            revision: i64,
            _history: &[RuleSetHistoryChange],
        ) -> AppResult<bool> {
            let mut packs = self.packs.lock().await;
            let Some(pack) = packs
                .iter()
                .find(|pack| pack.pack_id == id && pack.revision == revision)
            else {
                return Ok(false);
            };
            self.rules.lock().await.retain(|rule| {
                !pack
                    .members
                    .iter()
                    .any(|member| member.rule_set_id == rule.id)
            });
            packs.retain(|pack| pack.pack_id != id);
            Ok(true)
        }
        async fn copy_rule_pack_rule_set_to_custom(
            &self,
            id: &str,
            source: &str,
            custom: &RuleSet,
            revision: i64,
            _history: &[RuleSetHistoryChange],
        ) -> AppResult<bool> {
            let mut packs = self.packs.lock().await;
            let Some(pack) = packs
                .iter_mut()
                .find(|pack| pack.pack_id == id && pack.revision == revision)
            else {
                return Ok(false);
            };
            let mut rules = self.rules.lock().await;
            rules
                .iter_mut()
                .find(|rule| rule.id == source)
                .unwrap()
                .enabled = false;
            rules.push(custom.clone());
            pack.revision += 1;
            Ok(true)
        }
        async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
            Ok(self.rules.lock().await.clone())
        }

        async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .filter(|rule| rule.enabled)
                .cloned()
                .collect())
        }

        async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .find(|rule| rule.id == id)
                .cloned())
        }

        async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
            self.rules.lock().await.push(rule_set.clone());
            Ok(())
        }

        async fn create_rule_sets(
            &self,
            rule_sets: &[RuleSet],
            _history: &[RuleSetHistoryChange],
        ) -> AppResult<()> {
            let rejected = {
                let names = self.rejected_batch_names.lock().unwrap();
                rule_sets
                    .iter()
                    .any(|rule_set| names.contains(&rule_set.name))
            };
            if rejected {
                return Err(AppError::Repository("injected batch failure".into()));
            }
            self.rules.lock().await.extend_from_slice(rule_sets);
            Ok(())
        }

        async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
            let mut rules = self.rules.lock().await;
            let existing = rules
                .iter_mut()
                .find(|candidate| candidate.id == rule_set.id)
                .ok_or_else(|| AppError::NotFound(rule_set.id.clone()))?;
            *existing = rule_set.clone();
            Ok(())
        }

        async fn get_rule_set_builder_model(&self, id: &str) -> AppResult<Option<String>> {
            Ok(self.builder_models.lock().await.get(id).cloned())
        }

        async fn create_rule_set_with_builder_model(
            &self,
            rule_set: &RuleSet,
            builder_model: Option<&str>,
        ) -> AppResult<()> {
            self.create_rule_set(rule_set).await?;
            self.store_builder_model(&rule_set.id, builder_model).await;
            Ok(())
        }

        async fn update_rule_set_with_builder_model(
            &self,
            rule_set: &RuleSet,
            builder_model: Option<&str>,
        ) -> AppResult<()> {
            self.update_rule_set(rule_set).await?;
            self.store_builder_model(&rule_set.id, builder_model).await;
            Ok(())
        }

        async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
            self.rules.lock().await.retain(|rule| rule.id != id);
            self.builder_models.lock().await.remove(id);
            Ok(())
        }

        async fn record_rule_set_history(
            &self,
            _rule_set_id: &str,
            _action: &str,
            _rego_source: Option<&str>,
            _actor_id: Option<&str>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .find(|rule| rule.managed_key.as_deref() == Some(key))
                .cloned())
        }

        async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
            self.rules
                .lock()
                .await
                .retain(|rule| rule.managed_key.as_deref() != Some(key));
            Ok(())
        }

        async fn list_rule_sets_by_managed_key_prefix(
            &self,
            prefix: &str,
        ) -> AppResult<Vec<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .filter(|rule| {
                    rule.managed_key
                        .as_deref()
                        .is_some_and(|key| key.starts_with(prefix))
                })
                .cloned()
                .collect())
        }
    }

    fn build_test_app(profiles: Vec<QualityProfile>, rules: Vec<RuleSet>) -> AppUseCase {
        build_test_app_with_rule_repo(profiles, rules).0
    }

    fn build_test_app_with_rule_repo(
        profiles: Vec<QualityProfile>,
        rules: Vec<RuleSet>,
    ) -> (AppUseCase, Arc<TestRuleSetRepo>) {
        let rule_sets = Arc::new(TestRuleSetRepo::new(rules));
        let app = build_test_app_with_existing_rule_repo(profiles, rule_sets.clone());
        (app, rule_sets)
    }

    fn build_test_app_with_existing_rule_repo(
        profiles: Vec<QualityProfile>,
        rule_sets: Arc<TestRuleSetRepo>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(TestQualityProfileRepo { profiles }),
            String::new(),
        )
        .with_rule_sets(rule_sets.clone())
        .build_partial_for_tests();

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn test_profile(
        id: &str,
        name: &str,
        persona: ScoringPersona,
        atmos_preferred: bool,
        prefer_dual_audio: bool,
    ) -> QualityProfile {
        QualityProfile {
            id: id.to_string(),
            name: name.to_string(),
            criteria: QualityProfileCriteria {
                scoring_persona: persona,
                atmos_preferred,
                prefer_dual_audio,
                ..QualityProfileCriteria::default()
            },
        }
    }

    fn legacy_managed_rule(id: &str, managed_key: &str, name: &str, rego_source: &str) -> RuleSet {
        let now = Utc::now();
        RuleSet {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            rego_source: rego_source.to_string(),
            enabled: true,
            priority: -100,
            evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
            exclusive_group: None,
            disabled_reason_code: None,
            disabled_reason: None,
            applied_facets: vec![MediaFacet::Anime],
            created_at: now,
            updated_at: now,
            is_managed: true,
            managed_key: Some(managed_key.to_string()),
            managed_tag_filter: None,
        }
    }

    fn multi_audio_rule_input(
        profile_id: &str,
        release_is_dual_audio: bool,
        file_has_multiaudio: bool,
    ) -> scryer_rules::UserRuleInput {
        scryer_rules::UserRuleInput {
            release: scryer_rules::ReleaseDoc {
                protocol: None,
                stereoscopy_tokens: vec![],
                stereoscopy: None,
                raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265".to_string(),
                normalized_tokens: vec![],
                quality: Some("2160P".to_string()),
                source: Some("WEB-DL".to_string()),
                video_codec: Some("H.265".to_string()),
                audio: Some("DDP".to_string()),
                audio_codecs: vec!["DDP".to_string()],
                audio_channels: Some("5.1".to_string()),
                languages_audio: vec!["eng".to_string()],
                indexer_languages: vec![],
                languages_subtitles: vec![],
                is_dual_audio: release_is_dual_audio,
                is_atmos: false,
                is_dolby_vision: false,
                has_hdr_fallback: false,
                detected_hdr: false,
                is_remux: false,
                is_bd_disk: false,
                is_proper_upload: false,
                is_repack: false,
                revision: 0,
                is_ai_enhanced: false,
                is_hardcoded_subs: false,
                is_password_protected: None,
                is_hdr10plus: false,
                is_hlg: false,
                is_10bit: false,
                is_uncensored: false,
                is_dubs_only: false,
                has_release_group: true,
                is_obfuscated: false,
                is_retagged: false,
                streaming_service: None,
                edition: None,
                anime_version: None,
                episode_release_type: Some("single_episode".to_string()),
                is_season_pack: false,
                is_multi_episode: false,
                release_group: Some("TestGroup".to_string()),
                year: Some(2024),
                parse_confidence: 0.9,
                size_bytes: Some(8_000_000_000),
                age_days: Some(5),
                thumbs_up: None,
                thumbs_down: None,
                extra: Default::default(),
            },
            profile: scryer_rules::ProfileDoc {
                id: profile_id.to_string(),
                name: "Test Profile".to_string(),
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
                archival_quality: Some("2160P".to_string()),
                allow_unknown_quality: false,
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: "balanced".to_string(),
                scoring_overrides: Default::default(),
            },
            context: scryer_rules::ContextDoc {
                title_id: Some("tt1234567".to_string()),
                library_name: Some("Movies".to_string()),
                media_type: "movie".to_string(),
                category: "movie".to_string(),
                original_language: Some("eng".to_string()),
                original_country: Some("US".to_string()),
                inferred_original_audio_language: "eng".to_string(),
                tags: vec![],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto".to_string(),
                runtime_minutes: Some(120),
                coverage_total_runtime_minutes: Some(120),
                coverage_member_runtime_minutes: Some(120),
                coverage_member_count: Some(1),
                is_anime: false,
                is_filler: false,
            },
            builtin_score: scryer_rules::BuiltinScoreDoc {
                total: 0,
                blocked: false,
                codes: vec![],
            },
            file: Some(scryer_rules::FileDoc {
                video_codec: Some("hevc".to_string()),
                video_width: Some(3840),
                video_height: Some(2160),
                video_bitrate_kbps: Some(40000),
                video_bit_depth: Some(10),
                video_hdr_format: Some("HDR10".to_string()),
                dovi_profile: Some(8),
                dovi_bl_compat_id: Some(1),
                video_frame_rate: Some("23.976".to_string()),
                video_profile: Some("Main 10".to_string()),
                audio_codec: Some("eac3".to_string()),
                audio_profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                audio_channels: Some(6),
                audio_bitrate_kbps: Some(640),
                audio_languages: vec!["eng".to_string(), "jpn".to_string()],
                audio_streams: vec![scryer_rules::AudioStreamDoc {
                    codec: Some("eac3".to_string()),
                    profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                    channels: Some(6),
                    language: Some("eng".to_string()),
                    name: None,
                    bitrate_kbps: Some(640),
                }],
                subtitle_languages: vec!["eng".to_string()],
                subtitle_codecs: vec!["subrip".to_string()],
                subtitle_streams: vec![scryer_rules::SubtitleStreamDoc {
                    codec: Some("subrip".to_string()),
                    language: Some("eng".to_string()),
                    name: Some("English".to_string()),
                    forced: false,
                    default: true,
                }],
                has_multiaudio: file_has_multiaudio,
                duration_seconds: Some(7200),
                num_chapters: Some(12),
                container_format: Some("matroska".to_string()),
            }),
        }
    }

    #[tokio::test]
    async fn migration_creates_profile_scoped_multi_audio_rule() {
        let app = build_test_app(
            vec![test_profile(
                "balanced-legacy",
                "Balanced Legacy",
                ScoringPersona::Balanced,
                false,
                true,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Prefer Multi-Audio (Balanced Legacy)")
            .expect("expected migrated multi-audio rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-prefer-dual-audio:profile:balanced-legacy")
        );
        assert!(migrated.rego_source.contains("input.release.is_dual_audio"));
        assert!(
            migrated
                .rego_source
                .contains("not input.release.is_dual_audio")
        );
        assert!(migrated.rego_source.contains("input.file.has_multiaudio"));
        assert!(!migrated.is_managed);
    }

    #[tokio::test]
    async fn migration_creates_profile_scoped_atmos_rule_for_non_audiophile_profiles() {
        let app = build_test_app(
            vec![test_profile(
                "balanced-atmos",
                "Balanced Atmos",
                ScoringPersona::Balanced,
                true,
                false,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Prefer Atmos (Balanced Atmos)")
            .expect("expected migrated atmos rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-atmos-preferred:profile:balanced-atmos")
        );
        assert!(migrated.rego_source.contains("migrated_atmos_match"));
        assert!(migrated.rego_source.contains(":= 100 if"));
        assert!(migrated.rego_source.contains(":= -20 if"));
    }

    #[tokio::test]
    async fn migration_creates_cancel_rule_for_audiophile_profiles_that_disabled_atmos() {
        let app = build_test_app(
            vec![test_profile(
                "audiophile-no-atmos",
                "Audiophile No Atmos",
                ScoringPersona::Audiophile,
                false,
                false,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Disable Atmos Persona Bias (Audiophile No Atmos)")
            .expect("expected cancel-atmos migration rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-atmos-disabled:profile:audiophile-no-atmos")
        );
        assert!(migrated.rego_source.contains("migrated_atmos_cancel_match"));
        assert!(migrated.rego_source.contains(":= -150 if"));
        assert!(migrated.rego_source.contains(":= 30 if"));
    }

    #[tokio::test]
    async fn migration_converts_legacy_managed_multi_audio_rules_once() {
        let legacy = legacy_managed_rule(
            "legacy-rule",
            "convenience:prefer-dual-audio:anime",
            "Prefer Dual Audio (Anime)",
            "import rego.v1\n\nscore_entry[\"managed_dual_audio_preferred\"] := 200 if {\n    input.release.is_dual_audio\n}\n",
        );
        let app = build_test_app(vec![], vec![legacy]);

        app.migrate_legacy_persona_preferences().await.unwrap();
        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        assert_eq!(
            rules.iter().filter(|rule| rule.is_managed).count(),
            0,
            "legacy managed rule should be removed"
        );
        let migrated: Vec<_> = rules
            .iter()
            .filter(|rule| rule.name == "Migrated: Prefer Dual Audio (Anime)")
            .collect();
        assert_eq!(migrated.len(), 1, "migration should be idempotent");
        assert!(
            migrated[0].description.contains(
                "legacy-convenience-prefer-dual-audio:convenience:prefer-dual-audio:anime"
            )
        );
    }

    #[test]
    fn migrated_multi_audio_rule_scores_once_when_both_release_and_file_match() {
        let policy = scryer_rules::UserPolicy {
            id: "legacy_multi_audio".to_string(),
            name: "Legacy Multi-Audio".to_string(),
            rego_source: scryer_rules::rewrite_package_declaration(
                &generate_profile_prefer_multi_audio_rego("profile-1"),
                "legacy_multi_audio",
            ),
            origin: scryer_rules::PolicyOrigin::User,
            applied_facets: vec![],
        };

        let engine = scryer_rules::UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let result = evaluator
            .evaluate(&multi_audio_rule_input("profile-1", true, true), "movie")
            .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].delta, 200);
    }

    fn config_admin() -> User {
        User {
            id: scryer_domain::Id::new().0,
            username: "config-admin".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: scryer_domain::UserAuthorization {
                app: scryer_domain::AppPermissionMask::from_permissions([
                    scryer_domain::AppPermission::ManageCatalogSettings,
                ]),
                loaded: true,
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn managed_tag_filter_is_rejected_for_user_rule_sets() {
        let app = build_test_app(vec![], vec![]);
        let actor = config_admin();
        let rule_set = app
            .create_rule_set(
                &actor,
                "User Rule".to_string(),
                String::new(),
                r#"score_entry["bonus"] := 10"#.to_string(),
                vec![],
                0,
                Some(true),
            )
            .await
            .unwrap();

        let error = app
            .update_rule_set(
                &actor,
                rule_set.id,
                None,
                None,
                None,
                None,
                None,
                Some(vec!["locale:french".to_string()]),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("only applies to managed"),
            "{error}"
        );
    }

    fn batch_draft(name: &str) -> NewRuleSet {
        NewRuleSet {
            name: name.to_string(),
            description: String::new(),
            rego_source: r#"score_entry["bonus"] := 10"#.to_string(),
            applied_facets: vec![],
            priority: 0,
            enabled: false,
        }
    }

    fn claimed(name: &str, other: &str) -> ClaimedNewRuleSet {
        ClaimedNewRuleSet {
            rule_set: batch_draft(name),
            claimed_names: vec![name.to_string(), other.to_string()],
        }
    }

    #[tokio::test]
    async fn claimed_rule_set_batch_skips_names_taken_before_or_within_the_batch() {
        let existing = |id: &str, name: &str| {
            legacy_managed_rule(id, "", name, r#"score_entry["bonus"] := 1"#)
        };
        let (app, repo) = build_test_app_with_rule_repo(
            vec![],
            vec![
                existing("existing-1", "(Sample) Existing"),
                existing("existing-2", "(Sample) Prior"),
            ],
        );
        let results = app
            .create_rule_sets_batch_claiming_names(
                &config_admin(),
                vec![
                    claimed("(Sample) Existing", "(Sample) REVIEW - Existing"),
                    claimed("(Sample) REVIEW - Other", "(Sample) Other"),
                    claimed("(Sample) Other", "(Sample) REVIEW - Other"),
                    claimed("(Sample) REVIEW - Prior", "(Sample) Prior"),
                    claimed("(Sample) Fresh", "(Sample) REVIEW - Fresh"),
                ],
            )
            .await
            .expect("batch");
        let outcomes: Vec<Option<&str>> = results
            .iter()
            .map(|result| match result.as_ref().expect("no failure") {
                ClaimedRuleSetOutcome::Created(_) => None,
                ClaimedRuleSetOutcome::NameTaken { name } => Some(name.as_str()),
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                Some("(Sample) Existing"),
                None,
                Some("(Sample) REVIEW - Other"),
                Some("(Sample) Prior"),
                None,
            ]
        );
        let mut names: Vec<String> = repo
            .rules_snapshot()
            .await
            .into_iter()
            .map(|rule| rule.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "(Sample) Existing",
                "(Sample) Fresh",
                "(Sample) Prior",
                "(Sample) REVIEW - Other",
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_rule_set_batch_writes_each_row_with_its_history_or_not_at_all() {
        let (app, repo) = build_test_app_with_rule_repo(vec![], vec![]);
        repo.rejected_batch_names
            .lock()
            .unwrap()
            .insert("Sample Poison".to_string());
        let results = app
            .create_rule_sets_batch(
                &config_admin(),
                vec![
                    batch_draft("Sample Good A"),
                    batch_draft("Sample Poison"),
                    batch_draft("Sample Good B"),
                ],
            )
            .await
            .expect("batch");
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
        let mut names: Vec<String> = repo
            .rules_snapshot()
            .await
            .into_iter()
            .map(|rule| rule.name)
            .collect();
        names.sort();
        assert_eq!(names, ["Sample Good A", "Sample Good B"]);
    }
}
