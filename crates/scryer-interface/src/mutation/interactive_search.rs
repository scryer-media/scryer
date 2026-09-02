use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::query::from_interactive_release_search_snapshot;
use crate::types::*;

#[derive(Default)]
pub(crate) struct InteractiveSearchMutations;

#[Object]
impl InteractiveSearchMutations {
    /// Start a background interactive release-search job for the requested scopes.
    /// Results accumulate as indexers complete, and a new search cancels the caller's running job for the same scope.
    async fn start_interactive_release_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Either a title (with optional series-movie link, season and episode) or a raw query and kind, plus an optional indexer restriction, categories and result limit."
        )]
        input: SearchReleasesInput,
    ) -> GqlResult<InteractiveReleaseSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let SearchReleasesInput {
            title_id,
            series_movie_link_id,
            season,
            episode,
            limit,
            query,
            kind,
            indexer_ids,
            categories,
        } = input;
        let request = scryer_application::InteractiveReleaseSearchRequest {
            title_id: title_id.map(String::from),
            series_movie_link_id: series_movie_link_id.map(String::from),
            season,
            episode,
            limit,
            query,
            kind: kind.map(interactive_search_kind_from_value),
            indexer_ids: indexer_ids
                .map(|ids| ids.into_iter().map(String::from).collect::<Vec<_>>()),
            categories,
        };
        let snapshot = app
            .start_interactive_release_search(&actor, request)
            .await
            .map_err(to_gql_error)?;
        Ok(from_interactive_release_search_snapshot(snapshot))
    }

    /// Cancel a running interactive release-search job.
    async fn cancel_interactive_release_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Interactive release-search job identity to cancel.")] id: ID,
    ) -> GqlResult<CancelInteractiveReleaseSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .cancel_interactive_release_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(CancelInteractiveReleaseSearchPayload { id, accepted })
    }

    /// Mint a candidate token for one release of an interactive search, bound to the chosen title
    /// and season/episode target, so it can be queued with the existing download mutations.
    async fn issue_interactive_release_candidate_token(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Search job, release download URL, and the title (with optional season and episode) the release is assigned to."
        )]
        input: IssueInteractiveReleaseCandidateTokenInput,
    ) -> GqlResult<IndexerSearchResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let IssueInteractiveReleaseCandidateTokenInput {
            search_id,
            download_url,
            title_id,
            season,
            episode,
        } = input;
        let result = app
            .issue_interactive_release_candidate_token(
                &actor,
                search_id.as_ref(),
                &download_url,
                title_id.as_ref(),
                season,
                episode,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_search_result(result))
    }
}

/// GraphQL search kinds map one-to-one onto the application's.
fn interactive_search_kind_from_value(
    value: InteractiveSearchKindValue,
) -> scryer_application::InteractiveSearchKind {
    match value {
        InteractiveSearchKindValue::Movie => scryer_application::InteractiveSearchKind::Movie,
        InteractiveSearchKindValue::Series => scryer_application::InteractiveSearchKind::Series,
        InteractiveSearchKindValue::Anime => scryer_application::InteractiveSearchKind::Anime,
        InteractiveSearchKindValue::Raw => scryer_application::InteractiveSearchKind::Raw,
    }
}
