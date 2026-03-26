use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::*;

fn parse_facets(input: Option<Vec<String>>) -> Vec<scryer_domain::MediaFacet> {
    input
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| scryer_domain::MediaFacet::parse(&s))
        .collect()
}

#[derive(Default)]
pub(crate) struct RulesMutations;

#[Object]
impl RulesMutations {
    async fn create_rule_set(
        &self,
        ctx: &Context<'_>,
        input: CreateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set = app
            .create_rule_set(
                &actor,
                input.name,
                input.description.unwrap_or_default(),
                input.rego_source,
                parse_facets(input.applied_facets),
                input.priority.unwrap_or(0),
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    async fn update_rule_set(
        &self,
        ctx: &Context<'_>,
        input: UpdateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set = app
            .update_rule_set(
                &actor,
                input.id,
                input.name,
                input.description,
                input.rego_source,
                input.applied_facets.map(|f| parse_facets(Some(f))),
                input.priority,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    async fn delete_rule_set(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        app.delete_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(true)
    }

    async fn toggle_rule_set(
        &self,
        ctx: &Context<'_>,
        input: ToggleRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set = app
            .toggle_rule_set(&actor, &input.id, input.enabled)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    // ── Convenience settings ───────────────────────────────────────────

    async fn set_convenience_required_audio(
        &self,
        ctx: &Context<'_>,
        input: SetConvenienceRequiredAudioInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.set_convenience_required_audio(&actor, &input.scope, input.languages)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn set_title_required_audio(
        &self,
        ctx: &Context<'_>,
        input: SetTitleRequiredAudioInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet = input.facet.into_domain();
        app.set_title_required_audio(&actor, &input.title_id, facet.as_str(), input.languages)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn set_convenience_prefer_dual_audio(
        &self,
        ctx: &Context<'_>,
        input: SetConveniencePreferDualAudioInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.set_convenience_prefer_dual_audio(&actor, &input.scope, input.enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn validate_rule_set(
        &self,
        ctx: &Context<'_>,
        input: ValidateRuleSetInput,
    ) -> GqlResult<RuleValidationResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set_id = input
            .rule_set_id
            .unwrap_or_else(|| "r_validation_test".to_string());
        let result = app
            .validate_rule_set(&actor, &input.rego_source, &rule_set_id)
            .await
            .map_err(to_gql_error)?;

        Ok(RuleValidationResultPayload {
            valid: result.valid,
            errors: result.errors,
        })
    }
}
