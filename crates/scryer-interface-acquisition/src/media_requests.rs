use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_domain::{ExternalId, TitleExternalRating, TitleRatingSummary};

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::from_media_request;
use scryer_interface_media::types::{
    ApproveMediaRequestInput, ApproveMediaRequestPayload, MediaRequestActionPayload,
    MediaRequestPayload, SubmitMediaRequestInput, SubmitMediaRequestPayload,
    UpdateMediaRequestInput,
};

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
                scryer_application::SubmitMediaRequestInput {
                    library_id: String::from(input.library_id),
                    facet: input.facet.into_domain(),
                    title: input.title,
                    sort_title: input.sort_title,
                    slug: input.slug,
                    year: input.year,
                    overview: input.overview,
                    runtime_minutes: input.runtime_minutes,
                    language: input.language,
                    content_status: input.content_status,
                    rating_summary: TitleRatingSummary {
                        rating: input.rating,
                        rating_sources: input.rating_sources.unwrap_or_default(),
                        external_ratings: input
                            .external_ratings
                            .unwrap_or_default()
                            .into_iter()
                            .map(|rating| TitleExternalRating {
                                source: rating.source,
                                value: rating.value,
                                score: rating.score,
                                normalized: rating.normalized,
                                votes: rating.votes,
                                url: rating.url,
                            })
                            .collect(),
                    },
                    requested_quality_profile_id: input
                        .requested_quality_profile_id
                        .map(String::from),
                    requested_monitor_type: input
                        .requested_monitor_type
                        .map(|value| value.as_tag_value().to_string()),
                    external_ids: input
                        .external_ids
                        .into_iter()
                        .map(|external_id| ExternalId {
                            source: external_id.source,
                            value: external_id.value,
                        })
                        .collect(),
                },
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
            )
            .await
            .map_err(to_gql_error)?;

        Ok(ApproveMediaRequestPayload {
            title_id: outcome.title_id.into(),
            wanted_search: outcome
                .wanted_search
                .map(super::wanted::wanted_search_payload),
            search_error: outcome.search_error,
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
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_media_request(&app, request))
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
