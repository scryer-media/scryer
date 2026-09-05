use crate::context::{app_from_ctx, require_config_app_permission, to_gql_error};
use crate::types::*;
use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::AppError;
use scryer_application::request_rules::{RequestRuleDraft, RequestRulePreviewMatcher};
use scryer_domain::{AppPermission, RequestRuleEvaluationMode};

/// Resolve the matcher a preview should run.
///
/// The two forms are mutually exclusive by construction: previewing a stored
/// rule set and previewing an unsaved draft answer different questions, and
/// accepting both at once would make it ambiguous which source produced the
/// verdict the author is about to act on.
fn preview_matcher(input: &mut PreviewRequestRuleInput) -> GqlResult<RequestRulePreviewMatcher> {
    match (input.rule_set_id.take(), input.rego_source.take()) {
        (Some(rule_set_id), None) => Ok(RequestRulePreviewMatcher::Stored {
            rule_set_id: String::from(rule_set_id),
        }),
        (None, Some(rego_source)) => Ok(RequestRulePreviewMatcher::Inline { rego_source }),
        (Some(_), Some(_)) => Err(to_gql_error(AppError::Validation(
            "preview accepts either 'ruleSetId' or 'regoSource', not both".to_string(),
        ))),
        (None, None) => Err(to_gql_error(AppError::Validation(
            "preview requires either 'ruleSetId' or 'regoSource'".to_string(),
        ))),
    }
}

#[derive(Default)]
pub(crate) struct RequestRuleMutations;

#[Object]
impl RequestRuleMutations {
    /// Create a request rule set together with its first matcher revision.
    ///
    /// The rule is created disabled. Arming it is a second, deliberate call, and
    /// the instance gate is a third.
    async fn create_request_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule name, matcher source, and optional description and library scope.")]
        input: CreateRequestRuleSetInput,
    ) -> GqlResult<RequestRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let detail = app
            .create_request_rule_set(
                &actor,
                RequestRuleDraft {
                    name: input.name,
                    description: input.description.unwrap_or_default(),
                    rego_source: input.rego_source,
                    library_ids: input.library_ids.unwrap_or_default(),
                },
            )
            .await
            .map_err(to_gql_error)?;

        // A rule that has just been created has decided nothing.
        Ok(crate::mappers::from_request_rule_set_detail(detail, 0))
    }

    /// Replace a rule set's matcher by appending revision N+1.
    ///
    /// Revision N is left exactly as written, so a decision recorded against it
    /// stays attributable.
    async fn update_request_rule_matcher(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set identity and the replacement matcher source.")]
        input: UpdateRequestRuleMatcherInput,
    ) -> GqlResult<RequestRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set_id = String::from(input.rule_set_id);
        let detail = app
            .update_request_rule_matcher(&actor, &rule_set_id, &input.rego_source)
            .await
            .map_err(to_gql_error)?;
        let count = decision_count(&app, &actor, &rule_set_id).await?;

        Ok(crate::mappers::from_request_rule_set_detail(detail, count))
    }

    /// Rename and re-scope a rule set without touching its matcher, so no revision is created.
    async fn update_request_rule_metadata(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule-set identity, replacement name, and optional description and library scope."
        )]
        input: UpdateRequestRuleMetadataInput,
    ) -> GqlResult<RequestRuleSet> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set_id = String::from(input.rule_set_id);
        let rule_set = app
            .update_request_rule_metadata(
                &actor,
                &rule_set_id,
                input.name,
                input.description.unwrap_or_default(),
                input.library_ids.unwrap_or_default(),
            )
            .await
            .map_err(to_gql_error)?;
        let count = decision_count(&app, &actor, &rule_set_id).await?;

        Ok(crate::mappers::from_request_rule_set(&rule_set, count))
    }

    /// Move a request rule set between evaluation modes.
    async fn set_request_rule_mode(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set identity and the evaluation mode to store.")]
        input: SetRequestRuleModeInput,
    ) -> GqlResult<RequestRuleSetDetail> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set_id = String::from(input.rule_set_id);
        let detail = app
            .set_request_rule_mode(
                &actor,
                &rule_set_id,
                crate::mappers::request_rule_evaluation_mode_into_application(input.mode),
            )
            .await
            .map_err(to_gql_error)?;
        let count = decision_count(&app, &actor, &rule_set_id).await?;

        Ok(crate::mappers::from_request_rule_set_detail(detail, count))
    }

    /// Delete a request rule set and every revision it owns.
    ///
    /// The decisions it took part in are **kept**: a trace is the explanation of
    /// a decision already made, and it outlives the rule that produced it.
    async fn delete_request_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Request rule-set identity to delete.")] id: ID,
    ) -> GqlResult<DeleteRequestRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id = id.to_string();
        app.delete_request_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteRequestRuleSetPayload { id: ID::from(id) })
    }

    /// Compile and check request rule source without saving it.
    async fn validate_request_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Matcher source to validate.")] input: ValidateRequestRuleInput,
    ) -> GqlResult<RequestRuleValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let result = app
            .validate_request_rule_source(&actor, &input.rego_source)
            .await
            .map_err(to_gql_error)?;

        Ok(RequestRuleValidationPayload {
            valid: result.valid,
            errors: result.errors,
        })
    }

    /// Evaluate one matcher against one hypothetical request without saving anything.
    ///
    /// This is the author-side preview: it returns the vote, the reasons, the
    /// tags, **and** the exact input document the rule saw. The requester-facing
    /// equivalent is the `previewMyRequestDecision` query, which returns none of
    /// the internals.
    async fn preview_request_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Either a stored rule set or an unsaved matcher, plus the sample request to evaluate it against."
        )]
        mut input: PreviewRequestRuleInput,
    ) -> GqlResult<RequestRulePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let matcher = preview_matcher(&mut input)?;
        let stored_rule_set_id = match &matcher {
            RequestRulePreviewMatcher::Stored { rule_set_id } => Some(rule_set_id.clone()),
            RequestRulePreviewMatcher::Inline { .. } => None,
        };
        let sample = crate::mappers::request_rule_sample_into_application(input.sample)
            .map_err(|message| to_gql_error(AppError::Validation(message)))?;

        let result = app
            .preview_request_rule(
                &actor,
                scryer_application::RequestRulePreviewRequest { matcher, sample },
            )
            .await
            .map_err(to_gql_error)?;

        // An unsaved draft has no stored name and no stored mode: it is not
        // armed, because it is not stored.
        let (rule_set_name, mode) = match stored_rule_set_id {
            Some(rule_set_id) => app
                .get_request_rule_set(&actor, &rule_set_id)
                .await
                .map_err(to_gql_error)?
                .map(|detail| (detail.rule_set.name, detail.rule_set.evaluation_mode))
                .unwrap_or_else(|| (String::new(), RequestRuleEvaluationMode::Disabled)),
            None => (
                "Unsaved draft".to_string(),
                RequestRuleEvaluationMode::Disabled,
            ),
        };

        Ok(crate::mappers::from_request_rule_preview_result(
            result,
            rule_set_name,
            mode,
        ))
    }

    /// Arm or disarm the instance-wide request-rule gate. Omitted fields keep their stored value.
    ///
    /// While the gate is off every rule is still evaluated and every decision is
    /// still recorded; only the *effect* is suspended, so an operator can read
    /// what policy would have done before letting it do it.
    async fn set_request_rule_instance_gates(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The gate to change; omit the field to leave it alone.")]
        input: SetRequestRuleInstanceGatesInput,
    ) -> GqlResult<RequestRuleInstanceGates> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let gates = app
            .set_request_rule_instance_gates(
                &actor,
                scryer_application::RequestRuleGatesUpdate {
                    evaluation_enabled: input.evaluation_enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_request_rule_instance_gates(gates))
    }

    /// Push a live claim's window out. Requires title management on the claim's own library.
    async fn extend_title_claim(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Claim identity and its new expiry.")] input: ExtendTitleClaimInput,
    ) -> GqlResult<TitleClaim> {
        let app = app_from_ctx(ctx)?;
        let actor = crate::context::actor_from_ctx(ctx)?;

        let claim = app
            .extend_title_claim(&actor, input.claim_id.as_ref(), input.expires_at)
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_title_claim(claim))
    }

    /// Replace a live claim with a permanent operator keep.
    ///
    /// The replacement is a new claim rather than a mutated one, so the original
    /// stays as history and the trail still says a request produced the hold and
    /// an administrator chose to make it permanent.
    async fn convert_title_claim_to_permanent(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Claim identity to convert.")] input: ConvertTitleClaimToPermanentInput,
    ) -> GqlResult<TitleClaim> {
        let app = app_from_ctx(ctx)?;
        let actor = crate::context::actor_from_ctx(ctx)?;

        let claim = app
            .convert_title_claim_to_permanent(&actor, input.claim_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_title_claim(claim))
    }

    /// Withdraw a claim by hand. The reason is required and kept as history.
    async fn release_title_claim(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Claim identity and why it is being released.")]
        input: ReleaseTitleClaimInput,
    ) -> GqlResult<TitleClaim> {
        let app = app_from_ctx(ctx)?;
        let actor = crate::context::actor_from_ctx(ctx)?;

        let claim = app
            .release_title_claim(
                &actor,
                input.claim_id.as_ref(),
                &input.reason.unwrap_or_default(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_title_claim(claim))
    }
}

/// How many decisions one rule took part in.
///
/// Read after every mutation that returns a rule set so the payload a client
/// stores is the same shape the list query hands it.
async fn decision_count(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    rule_set_id: &str,
) -> GqlResult<u64> {
    let counts = app
        .request_rule_decision_counts(actor, std::slice::from_ref(&rule_set_id.to_string()))
        .await
        .map_err(to_gql_error)?;
    Ok(counts.get(rule_set_id).copied().unwrap_or_default())
}
