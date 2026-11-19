//! Maintainerr rule groups imported as lifecycle rules: one disabled
//! lifecycle rule per rule group, on the default schedule and every library.
//! A group tied to one media-server library carries a warning saying so.
//!
//! A group the translator cannot express is still saved, as a disabled
//! starter rule named `(Maintainerr) FAILED - <name>` whose description says
//! what did not translate. A group whose rule already exists under either
//! name is skipped, checked again under the lifecycle mutation lock in the
//! same transaction as the write.

use crate::lifecycle_rules::maintainerr::{
    ArrKind, MAINTAINERR_FAILED_NAME_PREFIX, MAINTAINERR_NAME_PREFIX, MaintainerrDiagnostic,
    MaintainerrDiagnosticSeverity, MaintainerrRuleStatus, MaintainerrTranslateContext,
    MaintainerrTranslatedRule, codes, translate_maintainerr_rule_group_values,
};
use crate::lifecycle_rules::service::LifecycleRuleDraft;
use crate::rule_import::{
    FetchedMaintainerrRules, MAX_ITEMS, PlannedRuleImportItem, PlannedRuleImportTranslation,
    RuleImportFact, RuleImportItemState, RuleImportPlanContext, RuleImportSaveContext,
    RuleImportSaveOutcome, refusal,
};
use crate::rules::builder_model::BuilderModelChange;
use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::User;
use serde_json::json;

pub mod reasons {
    /// The rule group could not be translated, so it was saved as a disabled
    /// starter rule; the item's facts carry the translator's diagnostics.
    pub const GROUP_UNTRANSLATABLE: &str = "RULE_IMPORT_GROUP_UNTRANSLATABLE";
    pub const ALL: &[&str] = &[GROUP_UNTRANSLATABLE];
}

/// What one rule group saves as.
#[derive(Debug)]
pub struct PlannedMaintainerrLifecycleRule {
    pub name: String,
    /// Every name that means this group was imported already: the translated
    /// and the failed name.
    pub taken_names: [String; 2],
    pub description: String,
    pub rego_source: String,
    pub grace_days: i64,
    pub max_titles_per_run: Option<u32>,
    /// `Saved` or `SavedFailedTranslation`.
    pub state: RuleImportItemState,
    pub reason: Option<&'static str>,
    pub facts: Vec<RuleImportFact>,
}

fn diagnostic_fact(diagnostic: MaintainerrDiagnostic) -> RuleImportFact {
    let severity = match diagnostic.severity {
        MaintainerrDiagnosticSeverity::Error => "ERROR",
        MaintainerrDiagnosticSeverity::Warning => "WARNING",
    };
    RuleImportFact::new(diagnostic.code, diagnostic.message).with_details(json!({
        "severity": severity,
        "ruleIndex": diagnostic.rule_index,
    }))
}

/// The warning that a group tied to one media-server library imports as a
/// rule on every Scryer library.
fn scope_warning(library_id: &str) -> MaintainerrDiagnostic {
    MaintainerrDiagnostic {
        code: codes::SCOPE_ALL_LIBRARIES,
        severity: MaintainerrDiagnosticSeverity::Warning,
        message: format!(
            "The group applies to media-server library \"{}\" in Maintainerr; the imported rule applies to every Scryer library.",
            library_id
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect::<String>()
        ),
        rule_index: None,
    }
}

fn planned_item(mut rule: MaintainerrTranslatedRule) -> PlannedRuleImportItem {
    if let Some(library_id) = rule
        .source_library_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        let warning = scope_warning(library_id);
        rule.diagnostics.push(warning);
    }
    let display = rule.display_name.clone();
    let (state, reason) = match rule.status {
        MaintainerrRuleStatus::Translated => (RuleImportItemState::Saved, None),
        MaintainerrRuleStatus::Failed => (
            RuleImportItemState::SavedFailedTranslation,
            Some(reasons::GROUP_UNTRANSLATABLE),
        ),
    };
    PlannedRuleImportItem {
        key: format!("rulegroup:{}", rule.source_group_id),
        source_name: display.clone(),
        translation: PlannedRuleImportTranslation::MaintainerrLifecycleRule(
            PlannedMaintainerrLifecycleRule {
                name: rule.name,
                taken_names: [
                    format!("{MAINTAINERR_NAME_PREFIX}{display}"),
                    format!("{MAINTAINERR_FAILED_NAME_PREFIX}{display}"),
                ],
                description: rule.description,
                rego_source: rule.rego_source,
                grace_days: rule.grace_days,
                max_titles_per_run: rule.max_titles_per_run,
                state,
                reason,
                facts: rule.diagnostics.into_iter().map(diagnostic_fact).collect(),
            },
        ),
    }
}

/// Plans one disabled lifecycle rule per rule group, in Maintainerr's order.
pub fn plan_maintainerr_rule_import(
    fetched: &FetchedMaintainerrRules,
    context: &RuleImportPlanContext,
) -> AppResult<Vec<PlannedRuleImportItem>> {
    if fetched.rule_groups.len() > MAX_ITEMS {
        return Err(AppError::validation_refused(
            refusal::TOO_MANY_ITEMS,
            "the source holds more objects than one import accepts",
        ));
    }
    let mut translate = MaintainerrTranslateContext {
        scryer_quality_profile_names: context.quality_profile_names.clone(),
        scryer_usernames: context.usernames.clone(),
        ..MaintainerrTranslateContext::default()
    };
    for (kind, profiles) in [
        (ArrKind::Radarr, &fetched.radarr_profiles),
        (ArrKind::Sonarr, &fetched.sonarr_profiles),
    ] {
        for (settings_id, body) in profiles {
            translate
                .add_arr_profile_values(kind, *settings_id, body)
                .map_err(|refusal| refusal.into_app_error())?;
        }
    }
    let translated = translate_maintainerr_rule_group_values(
        &fetched.rule_groups,
        &fetched.constants,
        &translate,
    )
    .map_err(|refusal| refusal.into_app_error())?;
    Ok(translated.into_iter().map(planned_item).collect())
}

impl AppUseCase {
    /// Saves one rule group's lifecycle rule as the acting user, through the
    /// same checks as any lifecycle rule create, unless a rule of either of
    /// its names exists.
    pub(crate) async fn save_maintainerr_lifecycle_rule(
        &self,
        actor: &User,
        context: &mut RuleImportSaveContext,
        rule: PlannedMaintainerrLifecycleRule,
    ) -> RuleImportSaveOutcome {
        let skipped = |rule: PlannedMaintainerrLifecycleRule| RuleImportSaveOutcome {
            state: RuleImportItemState::SkippedExists,
            saved_name: Some(rule.name),
            rule_set_id: None,
            reason: Some(refusal::ALREADY_EXISTS),
            facts: rule.facts,
        };
        if rule
            .taken_names
            .iter()
            .any(|name| context.existing_lifecycle_names.contains(name))
        {
            return skipped(rule);
        }
        let draft = LifecycleRuleDraft {
            name: rule.name.clone(),
            description: rule.description.clone(),
            rego_source: rule.rego_source.clone(),
            grace_days: rule.grace_days,
            library_ids: Vec::new(),
            max_titles_per_run: rule.max_titles_per_run,
        };
        // Both names are checked again under the lifecycle mutation lock, in
        // the insert's transaction, so overlapping runs write a group once.
        let created = self
            .create_lifecycle_rule_set_unless_named(
                actor,
                draft,
                BuilderModelChange::Keep,
                None,
                &rule.taken_names,
            )
            .await;
        if matches!(created, Ok(_)) {
            // Saved now, or by someone else since the run loaded the names.
            context
                .existing_lifecycle_names
                .extend(rule.taken_names.iter().cloned());
        }
        match created {
            Ok(Some(detail)) => RuleImportSaveOutcome {
                state: rule.state,
                saved_name: Some(rule.name),
                rule_set_id: Some(detail.rule_set.id),
                reason: rule.reason,
                facts: rule.facts,
            },
            Ok(None) => skipped(rule),
            Err(error) => {
                let reason = error.reason().unwrap_or(refusal::ITEM_FAILED);
                let mut facts = rule.facts;
                facts.push(RuleImportFact::new(
                    reason,
                    crate::rule_import::bounded_message(&error),
                ));
                RuleImportSaveOutcome {
                    state: RuleImportItemState::Failed,
                    saved_name: None,
                    rule_set_id: None,
                    reason: Some(reason),
                    facts,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "rule_import_maintainerr_tests.rs"]
mod tests;
