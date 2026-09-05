use async_graphql::{Context, ID, Object, Result as GqlResult};

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::{
    from_media_request, monitor_selection_from_input, submit_media_request_input_into_application,
};
use scryer_interface_media::types::{
    ApproveMediaRequestInput, ApproveMediaRequestPayload, MediaRequestActionPayload,
    MediaRequestPayload, SubmitMediaRequestInput, SubmitMediaRequestPayload,
    UpdateMediaRequestInput,
};

/// Resolve an approver's lease override into the application layer's
/// `Option<Option<i64>>`.
///
/// The double option is the honest shape and GraphQL has no way to spell it, so
/// two fields carry it: `None` means "grant what the requester asked for",
/// `Some(None)` is an explicit forever, and `Some(Some(n))` is a finite
/// override. Supplying both at once is refused rather than resolved by
/// precedence — an approver who typed a number *and* ticked forever has not
/// said what they want.
fn lease_override(
    lease_days: Option<i32>,
    lease_forever: Option<bool>,
) -> GqlResult<Option<Option<i64>>> {
    match (lease_days, lease_forever.unwrap_or(false)) {
        (Some(_), true) => Err(to_gql_error(scryer_application::AppError::Validation(
            "approve accepts either 'leaseDays' or 'leaseForever', not both".to_string(),
        ))),
        (Some(days), false) => Ok(Some(Some(i64::from(days)))),
        (None, true) => Ok(Some(None)),
        (None, false) => Ok(None),
    }
}

#[derive(Default)]
pub struct MediaRequestMutations;

#[Object]
impl MediaRequestMutations {
    /// Create a media request with the requested metadata and acquisition preferences.
    async fn submit_media_request(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Library, facet, title metadata, and optional quality and monitoring preferences."
        )]
        input: SubmitMediaRequestInput,
    ) -> GqlResult<SubmitMediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let outcome = app
            .submit_media_request(
                &actor,
                // The same conversion the pre-flight query uses, so a draft
                // previewed and a draft submitted are literally the same value.
                submit_media_request_input_into_application(input),
            )
            .await
            .map_err(to_gql_error)?;

        Ok(SubmitMediaRequestPayload {
            request_id: ID::from(outcome.request_id),
        })
    }

    /// Approve a request, create or update its title, and report any queued search attempt.
    async fn approve_media_request(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Request identity and optional quality and monitoring overrides.")]
        input: ApproveMediaRequestInput,
    ) -> GqlResult<ApproveMediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request_id = String::from(input.request_id);
        let outcome = app
            .approve_media_request(
                &actor,
                &request_id,
                input.quality_profile_id.as_ref(),
                input
                    .monitor_type
                    .map(|value| value.as_tag_value().to_string()),
                input.monitor_selection.map(monitor_selection_from_input),
                lease_override(input.lease_days, input.lease_forever)?,
                input.tags,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(ApproveMediaRequestPayload {
            title_id: outcome.title_id.into(),
            wanted_search: outcome
                .wanted_search
                .map(super::wanted::wanted_search_payload),
            search_error: outcome.search_error,
            claim_error: outcome.claim_error,
        })
    }

    /// Dismiss a media request without creating a catalog title.
    async fn dismiss_media_request(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Request identity to dismiss.")] request_id: ID,
    ) -> GqlResult<MediaRequestActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request_id = String::from(request_id);
        app.dismiss_media_request(&actor, &request_id)
            .await
            .map_err(to_gql_error)?;

        Ok(MediaRequestActionPayload {
            request_id: ID::from(request_id),
        })
    }

    /// Update the caller's pending request preferences and return the current request.
    async fn update_my_media_request(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Request identity and the replacement quality and monitoring preferences."
        )]
        input: UpdateMediaRequestInput,
    ) -> GqlResult<MediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request = app
            .update_my_media_request(
                &actor,
                scryer_application::UpdateMediaRequestInput {
                    request_id: String::from(input.request_id),
                    requested_quality_profile_id: String::from(input.requested_quality_profile_id),
                    requested_monitor_type: input
                        .requested_monitor_type
                        .map(|value| value.as_tag_value().to_string()),
                    requested_monitor_selection: input
                        .requested_monitor_selection
                        .map(monitor_selection_from_input),
                    requested_lease_days: input.requested_lease_days.map(i64::from),
                },
            )
            .await
            .map_err(to_gql_error)?;

        // One request, so the batched policy read is a batch of one; the edit
        // re-evaluates the request, and returning it without the fresh verdict
        // would show the requester the decision they had before they edited.
        let policy = app
            .media_request_policy_facts(&actor, std::slice::from_ref(&request))
            .await
            .unwrap_or_default();
        let facts = policy.get(&request.id).cloned().unwrap_or_default();
        Ok(from_media_request(&app, request, Some(&facts)))
    }

    /// Cancel the caller's request without deleting an already-created title.
    async fn cancel_my_media_request(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Request identity to cancel.")] request_id: ID,
    ) -> GqlResult<MediaRequestActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request_id = String::from(request_id);
        app.cancel_my_media_request(&actor, &request_id)
            .await
            .map_err(to_gql_error)?;

        Ok(MediaRequestActionPayload {
            request_id: ID::from(request_id),
        })
    }
}
