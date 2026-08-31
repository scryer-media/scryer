use crate::context::{app_from_ctx, require_config_app_permission, to_gql_error};
use crate::types::*;
use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::AppError;
use scryer_application::maintenance_rules::{
    MaintenanceMatcherDraft, MaintenancePreviewMatcher, MaintenancePreviewRequest,
    MaintenancePreviewSelection, MaintenanceRuleDraft,
};
use scryer_domain::AppPermission;

/// Grace periods arrive as GraphQL `Int` and are stored as days; a negative
/// value is rejected by the service, not silently clamped here.
fn grace_days(input: Option<i32>) -> i64 {
    i64::from(input.unwrap_or(0))
}

/// Resolve the matcher a preview should run.
///
/// The two forms are mutually exclusive by construction: previewing a stored
/// rule set and previewing an unsaved draft answer different questions, and
/// accepting both at once would make it ambiguous which source produced the
/// outcomes the caller is about to act on.
fn preview_matcher(
    input: &mut PreviewMaintenanceRuleInput,
) -> GqlResult<MaintenancePreviewMatcher> {
    let rule_set_id = input.rule_set_id.take();
    let rego_source = input.rego_source.take();
    let action = input.action.take();

    match (rule_set_id, rego_source, action) {
        (Some(rule_set_id), None, None) => Ok(MaintenancePreviewMatcher::Stored {
            rule_set_id: String::from(rule_set_id),
        }),
        (None, Some(rego_source), Some(action)) => Ok(MaintenancePreviewMatcher::Inline {
            rego_source,
            action_spec: crate::mappers::maintenance_action_spec_from_input(action),
            grace_days: grace_days(input.grace_days),
        }),
        (None, Some(_), None) => Err(to_gql_error(AppError::Validation(
            "previewing an unsaved matcher requires 'action'".to_string(),
        ))),
        (None, None, Some(_)) => Err(to_gql_error(AppError::Validation(
            "previewing an unsaved matcher requires 'regoSource'".to_string(),
        ))),
        (None, None, None) => Err(to_gql_error(AppError::Validation(
            "preview requires either 'ruleSetId' or 'regoSource' with 'action'".to_string(),
        ))),
        (Some(_), _, _) => Err(to_gql_error(AppError::Validation(
            "preview accepts either 'ruleSetId' or an unsaved matcher, not both".to_string(),
        ))),
    }
}

/// Resolve which titles a preview should evaluate. Exactly one selection form
/// is accepted so the caller always knows what the returned rows cover.
fn preview_selection(
    input: &mut PreviewMaintenanceRuleInput,
) -> GqlResult<MaintenancePreviewSelection> {
    match (input.title_ids.take(), input.library_id.take()) {
        (Some(title_ids), None) => Ok(MaintenancePreviewSelection::Titles(
            title_ids.into_iter().map(String::from).collect(),
        )),
        (None, Some(library_id)) => Ok(MaintenancePreviewSelection::Library {
            library_id: String::from(library_id),
            limit: input
                .limit
                .map(|limit| usize::try_from(limit).unwrap_or_default()),
        }),
        (Some(_), Some(_)) => Err(to_gql_error(AppError::Validation(
            "preview accepts either 'titleIds' or 'libraryId', not both".to_string(),
        ))),
        (None, None) => Err(to_gql_error(AppError::Validation(
            "preview requires either 'titleIds' or 'libraryId'".to_string(),
        ))),
    }
}

#[derive(Default)]
pub(crate) struct MaintenanceRuleMutations;

#[Object]
impl MaintenanceRuleMutations {
    /// Create a maintenance rule set together with its first matcher revision.
    async fn create_maintenance_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule name, matcher source, action, and optional description, grace period, and library scope."
        )]
        input: CreateMaintenanceRuleSetInput,
    ) -> GqlResult<MaintenanceRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let detail = app
            .create_maintenance_rule_set(
                &actor,
                MaintenanceRuleDraft {
                    name: input.name,
                    description: input.description.unwrap_or_default(),
                    rego_source: input.rego_source,
                    action_spec: crate::mappers::maintenance_action_spec_from_input(input.action),
                    grace_days: grace_days(input.grace_days),
                    library_ids: input.library_ids.unwrap_or_default(),
                    // Maintenance rules ship dark: the service accepts only the
                    // disabled mode, so the client never chooses one.
                    evaluation_mode: None,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_maintenance_rule_set_detail(detail))
    }

    /// Replace a rule set's matcher, action, and grace period by appending a revision.
    async fn update_maintenance_rule_matcher(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule-set identity, replacement matcher source and action, and optional grace period."
        )]
        input: UpdateMaintenanceRuleMatcherInput,
    ) -> GqlResult<MaintenanceRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let detail = app
            .update_maintenance_rule_matcher(
                &actor,
                input.id.as_ref(),
                MaintenanceMatcherDraft {
                    rego_source: input.rego_source,
                    action_spec: crate::mappers::maintenance_action_spec_from_input(input.action),
                    grace_days: grace_days(input.grace_days),
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_maintenance_rule_set_detail(detail))
    }

    /// Rename and re-scope a rule set without touching its matcher, so no revision is created.
    async fn update_maintenance_rule_metadata(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule-set identity, replacement name, and optional description and library scope."
        )]
        input: UpdateMaintenanceRuleMetadataInput,
    ) -> GqlResult<MaintenanceRuleSet> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        app.update_maintenance_rule_metadata(
            &actor,
            input.id.as_ref(),
            input.name,
            input.description.unwrap_or_default(),
            input.library_ids.unwrap_or_default(),
        )
        .await
        .map_err(to_gql_error)?;

        // The payload carries the action and grace period of the revision in
        // force, which this edit deliberately does not touch, so the answer is
        // read back through the same detail path the rest of the surface uses.
        let detail = app
            .get_maintenance_rule_set(&actor, input.id.as_ref())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| {
                to_gql_error(AppError::NotFound(format!(
                    "maintenance rule set {} not found",
                    input.id.as_str()
                )))
            })?;

        Ok(crate::mappers::from_maintenance_rule_set(&detail))
    }

    /// Delete a maintenance rule set and every revision it owns.
    async fn delete_maintenance_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Maintenance rule-set identity to delete.")] id: ID,
    ) -> GqlResult<DeleteMaintenanceRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id = id.to_string();
        app.delete_maintenance_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteMaintenanceRuleSetPayload { id: ID::from(id) })
    }

    /// Compile and check maintenance rule source without saving it.
    async fn validate_maintenance_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Matcher source to validate.")] input: ValidateMaintenanceRuleInput,
    ) -> GqlResult<MaintenanceRuleValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let result = app
            .validate_maintenance_rule_source(&actor, &input.rego_source)
            .await
            .map_err(to_gql_error)?;

        Ok(MaintenanceRuleValidationPayload {
            valid: result.valid,
            errors: result.errors,
        })
    }

    /// Evaluate one matcher against a bounded title selection without saving anything.
    async fn preview_maintenance_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Either a stored rule set or an unsaved matcher, plus either the titles or the library to evaluate."
        )]
        mut input: PreviewMaintenanceRuleInput,
    ) -> GqlResult<MaintenancePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let matcher = preview_matcher(&mut input)?;
        let selection = preview_selection(&mut input)?;

        let result = app
            .preview_maintenance_rule(&actor, MaintenancePreviewRequest { matcher, selection })
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_maintenance_preview_result(result))
    }
}
