use crate::context::{app_from_ctx, require_config_app_permission, to_gql_error};
use crate::types::*;
use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::AppError;
use scryer_application::lifecycle_rules::{
    LifecycleMatcherDraft, LifecyclePreviewMatcher, LifecyclePreviewRequest,
    LifecyclePreviewSelection, LifecycleRuleDraft,
};
use scryer_domain::AppPermission;

/// Grace periods arrive as GraphQL `Int` and are stored as days; a negative
/// value is rejected by the service, not silently clamped here.
fn grace_days(input: Option<i32>) -> i64 {
    i64::from(input.unwrap_or(0))
}

/// A schedule input in domain form, refused with the same code a scheduled
/// script uses when its kind is missing a field.
fn rule_schedule(
    input: Option<ScriptScheduleInput>,
) -> GqlResult<Option<scryer_domain::ScriptSchedule>> {
    input
        .map(|input| {
            input.into_domain().map_err(|field| {
                to_gql_error(AppError::validation_refused(
                    scryer_interface_core::refusal::INTERFACE_SCRIPT_SCHEDULE_INCOMPLETE,
                    format!("schedule is missing {field} for its kind"),
                ))
            })
        })
        .transpose()
}

fn diagnostic_payload(
    diagnostic: scryer_application::RuleValidationDiagnostic,
) -> RuleValidationDiagnosticPayload {
    RuleValidationDiagnosticPayload {
        code: diagnostic.code.into(),
        message: diagnostic.message,
        line: diagnostic.line.and_then(|v| i32::try_from(v).ok()),
        column: diagnostic.column.and_then(|v| i32::try_from(v).ok()),
        path: diagnostic.path,
    }
}

/// Resolve the matcher a preview should run.
///
/// The two forms are mutually exclusive by construction: previewing a stored
/// rule set and previewing an unsaved draft answer different questions, and
/// accepting both at once would make it ambiguous which source produced the
/// outcomes the caller is about to act on.
fn preview_matcher(input: &mut PreviewLifecycleRuleInput) -> GqlResult<LifecyclePreviewMatcher> {
    let rule_set_id = input.rule_set_id.take();
    let rego_source = input.rego_source.take();

    match (rule_set_id, rego_source) {
        (Some(rule_set_id), None) => Ok(LifecyclePreviewMatcher::Stored {
            rule_set_id: String::from(rule_set_id),
        }),
        (None, Some(rego_source)) => Ok(LifecyclePreviewMatcher::Inline {
            library_ids: input.library_ids.take().unwrap_or_default(),
            rego_source,
            grace_days: grace_days(input.grace_days),
        }),
        (None, None) => Err(to_gql_error(AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_RULE_PREVIEW_TARGET_REQUIRED,
            "preview requires either 'ruleSetId' or 'regoSource'",
        ))),
        (Some(_), Some(_)) => Err(to_gql_error(AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_RULE_PREVIEW_TARGET_CONFLICT,
            "preview accepts either 'ruleSetId' or an unsaved matcher, not both",
        ))),
    }
}

/// Resolve which titles a preview should evaluate. Exactly one selection form
/// is accepted so the caller always knows what the returned rows cover.
fn preview_selection(
    input: &mut PreviewLifecycleRuleInput,
) -> GqlResult<LifecyclePreviewSelection> {
    if let Some(subject_ids) = input.subject_ids.take() {
        if input.library_id.is_some() || input.title_ids.is_none() {
            return Err(to_gql_error(AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_RULE_SUBJECT_SELECTION_INVALID,
                "subjectIds requires titleIds and cannot be combined with libraryId",
            )));
        }
        return Ok(LifecyclePreviewSelection::Subjects {
            title_ids: input
                .title_ids
                .take()
                .unwrap_or_default()
                .into_iter()
                .map(String::from)
                .collect(),
            subject_ids: subject_ids.into_iter().map(String::from).collect(),
        });
    }
    match (input.title_ids.take(), input.library_id.take()) {
        (Some(title_ids), None) => Ok(LifecyclePreviewSelection::Titles(
            title_ids.into_iter().map(String::from).collect(),
        )),
        (None, Some(library_id)) => Ok(LifecyclePreviewSelection::Library {
            library_id: String::from(library_id),
            limit: input
                .limit
                .map(|limit| usize::try_from(limit).unwrap_or_default()),
        }),
        (Some(_), Some(_)) => Err(to_gql_error(AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_RULE_SCOPE_CONFLICT,
            "preview accepts either 'titleIds' or 'libraryId', not both",
        ))),
        (None, None) => Err(to_gql_error(AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_RULE_SCOPE_REQUIRED,
            "preview requires either 'titleIds' or 'libraryId'",
        ))),
    }
}

/// A value below one reaches the service as zero, which refuses it with its
/// own reason, so the API has a single refusal for an unusable maximum.
fn max_titles_per_run(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

/// Omitted keeps the stored maximum, null clears it, a value replaces it.
fn max_titles_per_run_change(value: async_graphql::MaybeUndefined<i32>) -> Option<Option<u32>> {
    match value {
        async_graphql::MaybeUndefined::Undefined => None,
        async_graphql::MaybeUndefined::Null => Some(None),
        async_graphql::MaybeUndefined::Value(max) => Some(Some(max_titles_per_run(max))),
    }
}

#[derive(Default)]
pub(crate) struct LifecycleRuleMutations;

#[Object]
impl LifecycleRuleMutations {
    /// Create a lifecycle rule set together with its first matcher revision.
    async fn create_lifecycle_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule name, source, and optional description, grace period, and library scope."
        )]
        input: CreateLifecycleRuleSetInput,
    ) -> GqlResult<LifecycleRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let schedule = rule_schedule(input.schedule)?;
        let detail = app
            .create_lifecycle_rule_set_with_schedule(
                &actor,
                LifecycleRuleDraft {
                    name: input.name,
                    description: input.description.unwrap_or_default(),
                    rego_source: input.rego_source,
                    grace_days: grace_days(input.grace_days),
                    library_ids: input.library_ids.unwrap_or_default(),
                    max_titles_per_run: input.max_titles_per_run.map(max_titles_per_run),
                },
                super::rules::builder_model_change(input.builder_model),
                schedule,
            )
            .await
            .map_err(to_gql_error)?;

        // A new rule starts disabled, so it has no next run yet.
        Ok(crate::mappers::from_lifecycle_rule_set_detail(detail, None))
    }

    /// Replace a rule set's source and grace period by appending a revision.
    async fn update_lifecycle_rule_matcher(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set identity, replacement source, and optional grace period.")]
        input: UpdateLifecycleRuleMatcherInput,
    ) -> GqlResult<LifecycleRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let detail = app
            .update_lifecycle_rule_matcher_with_builder_model(
                &actor,
                input.id.as_ref(),
                LifecycleMatcherDraft {
                    rego_source: input.rego_source,
                    grace_days: grace_days(input.grace_days),
                    max_titles_per_run: max_titles_per_run_change(input.max_titles_per_run),
                },
                super::rules::builder_model_change(input.builder_model),
            )
            .await
            .map_err(to_gql_error)?;

        let next_run_at = app.lifecycle_rule_next_run_at(&detail.rule_set.id);
        Ok(crate::mappers::from_lifecycle_rule_set_detail(
            detail,
            next_run_at,
        ))
    }

    /// Rename, re-scope or reschedule a rule set without touching its matcher, so no revision is created.
    async fn update_lifecycle_rule_metadata(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule-set identity, replacement name, and optional description, library scope and schedule."
        )]
        input: UpdateLifecycleRuleMetadataInput,
    ) -> GqlResult<LifecycleRuleSet> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let schedule = rule_schedule(input.schedule)?;
        app.update_lifecycle_rule_metadata_with_schedule(
            &actor,
            input.id.as_ref(),
            input.name,
            input.description.unwrap_or_default(),
            input.library_ids.unwrap_or_default(),
            schedule,
            max_titles_per_run_change(input.max_titles_per_run),
        )
        .await
        .map_err(to_gql_error)?;

        // The payload carries the action and grace period of the revision in
        // force, which this edit deliberately does not touch, so the answer is
        // read back through the same detail path the rest of the surface uses.
        let detail = app
            .get_lifecycle_rule_set(&actor, input.id.as_ref())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| {
                to_gql_error(AppError::not_found_reason(
                    scryer_interface_core::refusal::INTERFACE_LIFECYCLE_RULE_NOT_FOUND,
                    format!("lifecycle rule set {} not found", input.id.as_str()),
                ))
            })?;

        Ok(crate::mappers::from_lifecycle_rule_set(
            &detail,
            app.lifecycle_rule_next_run_at(&detail.rule_set.id),
        ))
    }

    /// Delete a lifecycle rule set and every revision it owns.
    ///
    /// Refused for a rule that has already run actions, because the delete
    /// cascades and would take the record of what it did with it, and for one
    /// that is still tracking subjects. Set the rule's mode to `DISABLED`
    /// instead: it stops just as completely and keeps its history.
    async fn delete_lifecycle_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Lifecycle rule-set identity to delete.")] id: ID,
    ) -> GqlResult<DeleteLifecycleRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id = id.to_string();
        app.delete_lifecycle_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteLifecycleRuleSetPayload { id: ID::from(id) })
    }

    /// Compile and check lifecycle rule source without saving it.
    async fn validate_lifecycle_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Matcher source to validate.")] input: ValidateLifecycleRuleInput,
    ) -> GqlResult<LifecycleRuleValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let validation = app
            .validate_lifecycle_rule_source(&actor, &input.rego_source)
            .await
            .map_err(to_gql_error)?;
        let result = validation.result;

        Ok(LifecycleRuleValidationPayload {
            diagnostics: result
                .diagnostics
                .into_iter()
                .map(diagnostic_payload)
                .collect(),
            valid: result.valid,
            errors: result.errors,
            warnings: result
                .warnings
                .into_iter()
                .map(diagnostic_payload)
                .collect(),
            declared_actions: validation
                .declared_actions
                .as_ref()
                .map(crate::mappers::from_lifecycle_declared_actions),
        })
    }

    /// Evaluate one matcher against a bounded title selection without saving anything.
    async fn preview_lifecycle_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Either a stored rule set or an unsaved matcher, plus either the titles or the library to evaluate."
        )]
        mut input: PreviewLifecycleRuleInput,
    ) -> GqlResult<LifecyclePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let matcher = preview_matcher(&mut input)?;
        let selection = preview_selection(&mut input)?;

        let result = app
            .preview_lifecycle_rule(&actor, LifecyclePreviewRequest { matcher, selection })
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_lifecycle_preview_result(result))
    }

    /// Set a lifecycle rule to Disabled, Dry run, or Enabled.
    ///
    /// Enabled arms the effects the rule's own action can have. Enabling a rule
    /// that can delete files requires a confirmation of the reviewed revision,
    /// library scope, and current candidate count. A rule moved back to
    /// Disabled keeps the candidates it already opened.
    async fn set_lifecycle_rule_state(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set ID.")] rule_set_id: ID,
        #[graphql(desc = "The state to store.")] state: LifecycleRuleState,
        #[graphql(desc = "Required when enabling a rule that can delete files.")]
        confirmation: Option<LifecycleRuleArmingConfirmationInput>,
    ) -> GqlResult<LifecycleRuleSet> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let candidate_count = confirmation
            .as_ref()
            .map(|reviewed| i64::from(reviewed.candidate_count));
        let detail = app
            .set_lifecycle_rule_state(
                &actor,
                rule_set_id.as_ref(),
                crate::mappers::lifecycle_rule_state_into_application(state),
                confirmation.map(|reviewed| scryer_domain::LifecycleRuleArmingConfirmation {
                    revision_number: i64::from(reviewed.revision_number),
                    library_ids: reviewed
                        .library_ids
                        .into_iter()
                        .map(|id| id.to_string())
                        .collect(),
                }),
                candidate_count,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_lifecycle_rule_set(
            &detail,
            app.lifecycle_rule_next_run_at(&detail.rule_set.id),
        ))
    }

    /// Fire one rule now instead of waiting for its next scheduled fire: it is
    /// evaluated first, as a scheduled fire does, then its due titles are
    /// acted on, including any the evaluation adds that are already due.
    /// Rules act only through their own fires, so a call that names no rule
    /// is refused with `LIFECYCLE_ACTION_RUN_RULE_REQUIRED`.
    async fn run_lifecycle_action_handler_now(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "The rule to act for. Required: omitting it is refused with LIFECYCLE_ACTION_RUN_RULE_REQUIRED."
        )]
        rule_set_id: Option<ID>,
    ) -> GqlResult<LifecycleEvaluationTriggerPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let trigger = app
            .run_lifecycle_action_handler_now(&actor, rule_set_id.map(String::from))
            .await
            .map_err(to_gql_error)?;

        Ok(LifecycleEvaluationTriggerPayload {
            started: trigger.started,
            message: trigger.message,
        })
    }

    /// Set the instance-wide lifecycle switches. Omitted fields keep their stored value.
    async fn set_lifecycle_instance_gates(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The switches to change; omit a field to leave that switch alone.")]
        input: SetLifecycleInstanceGatesInput,
    ) -> GqlResult<LifecycleInstanceGates> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let gates = app
            .set_lifecycle_instance_gates(
                &actor,
                scryer_application::lifecycle_rules::LifecycleGatesUpdate {
                    enabled: input.enabled,
                    actions_paused: input.actions_paused,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_lifecycle_instance_gates(gates))
    }

    /// Exclude a subject from one lifecycle rule, or from every rule when no rule is named.
    ///
    /// The exclusion takes effect at the next evaluation, which is also where an
    /// existing candidate for the subject moves to the excluded state.
    async fn exclude_lifecycle_subject(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Subject to exclude, the optional rule to confine it to, and a reason.")]
        input: ExcludeLifecycleSubjectInput,
    ) -> GqlResult<LifecycleExclusion> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let subject_kind =
            crate::mappers::lifecycle_rule_subject_kind_into_application(input.subject_kind);
        let subject_id = input
            .subject_id
            .as_ref()
            .map(|id| id.as_str())
            .or_else(|| {
                (subject_kind == scryer_domain::LifecycleRuleSubjectKind::Title)
                    .then(|| input.title_id.as_str())
            })
            .ok_or_else(|| {
                to_gql_error(AppError::validation_refused(
                    scryer_interface_core::refusal::INTERFACE_RULE_EXCLUSION_SUBJECT_REQUIRED,
                    "episode and season exclusions require subjectId",
                ))
            })?;
        let exclusion = app
            .exclude_lifecycle_scoped_subject(
                &actor,
                input.title_id.as_ref(),
                subject_kind,
                subject_id,
                input.rule_set_id.map(String::from),
                input.reason,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_lifecycle_exclusion(exclusion))
    }

    /// Remove a lifecycle exclusion.
    async fn remove_lifecycle_exclusion(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Exclusion identity to remove.")] id: ID,
    ) -> GqlResult<DeleteLifecycleExclusionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let removed = app
            .remove_lifecycle_exclusion(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteLifecycleExclusionPayload {
            id: ID::from(removed),
        })
    }

    /// Run lifecycle evaluation now.
    ///
    /// Without a rule, this starts the ordinary scheduled job so the run shows
    /// up in the system-jobs surface, and returns as soon as it is accepted.
    /// Scoped to one rule it runs inline instead, because the job seam carries
    /// no parameters; that path is bounded by the rule's own library scope.
    async fn run_lifecycle_evaluation_now(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Evaluate only this rule set; omit to evaluate every enabled rule.")]
        rule_set_id: Option<ID>,
    ) -> GqlResult<LifecycleEvaluationTriggerPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let trigger = app
            .run_lifecycle_evaluation_now(&actor, rule_set_id.map(String::from))
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_lifecycle_evaluation_trigger(trigger))
    }
}

#[cfg(test)]
mod lifecycle_scope_tests {
    use super::*;
    use async_graphql::{InputType, value};

    #[test]
    fn lifecycle_preview_of_titles_selects_those_titles() {
        let mut input = PreviewLifecycleRuleInput::parse(Some(
            value!({ "ruleSetId": "rule", "titleIds": ["title"] }),
        ))
        .unwrap_or_else(|_| panic!("fixture preview input must parse"));
        assert!(
            matches!(preview_selection(&mut input).unwrap(), LifecyclePreviewSelection::Titles(ids) if ids == vec!["title"])
        );
    }

    #[test]
    fn lifecycle_scope_exact_subject_requires_owning_titles() {
        let mut input = PreviewLifecycleRuleInput::parse(Some(
            value!({ "ruleSetId": "rule", "titleIds": ["title"], "subjectIds": ["episode"] }),
        ))
        .unwrap_or_else(|_| panic!("fixture preview input must parse"));
        assert!(
            matches!(preview_selection(&mut input).unwrap(), LifecyclePreviewSelection::Subjects { title_ids, subject_ids } if title_ids == vec!["title"] && subject_ids == vec!["episode"])
        );
        for value in [
            value!({ "ruleSetId": "rule", "subjectIds": ["episode"] }),
            value!({ "ruleSetId": "rule", "subjectIds": ["episode"], "libraryId": "library" }),
        ] {
            let mut input = PreviewLifecycleRuleInput::parse(Some(value))
                .unwrap_or_else(|_| panic!("fixture preview input must parse"));
            assert!(preview_selection(&mut input).is_err());
        }
    }
}
