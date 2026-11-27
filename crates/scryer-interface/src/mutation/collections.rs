use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_episode, from_series_movie_link};
use crate::types::*;
use async_graphql::{Context, Object, Result as GqlResult};

#[derive(Default)]
pub(crate) struct CollectionMutations;

#[Object]
impl CollectionMutations {
    /// Set collection monitoring and return the affected collection episodes.
    async fn set_collection_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Collection identity and desired monitored state.")]
        input: SetCollectionMonitoredInput,
    ) -> GqlResult<SetCollectionMonitoredPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection_id = input.collection_id.to_string();
        let collection = app
            .set_collection_monitored(&actor, &collection_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        let episodes = app
            .list_episodes(&actor, &collection_id)
            .await
            .map_err(to_gql_error)?;
        Ok(SetCollectionMonitoredPayload {
            id: collection.id.into(),
            monitored: collection.monitored,
            episodes: episodes
                .into_iter()
                .map(|episode| from_episode(&app, episode))
                .collect(),
        })
    }

    /// Set episode monitoring and return the updated episode.
    async fn set_episode_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Episode identity and desired monitored state.")]
        input: SetEpisodeMonitoredInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode_id = input.episode_id.to_string();
        let episode = app
            .set_episode_monitored(&actor, &episode_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(&app, episode))
    }

    /// Set monitoring for several episodes and return them, in request order.
    /// Checks and side effects are those of `setEpisodeMonitored`, done once
    /// for the whole list; a repeated id is answered once. The request is
    /// refused whole and nothing changes when it names more than 10000
    /// distinct episodes (reason `CATALOG_EPISODE_MONITOR_BATCH_TOO_LARGE`,
    /// with the `maxEpisodeIds` and `requestedEpisodeIds` extensions), when
    /// any id matches no episode (reason
    /// `CATALOG_EPISODE_MONITOR_BATCH_UNKNOWN_EPISODES`, with those ids in the
    /// `unknownEpisodeIds` extension), or when the actor cannot manage one of
    /// the episodes' titles.
    async fn set_episodes_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Episode identities and desired monitored state.")]
        input: SetEpisodesMonitoredInput,
    ) -> GqlResult<Vec<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode_ids = input
            .episode_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        let episodes = app
            .set_episodes_monitored(&actor, &episode_ids, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes
            .into_iter()
            .map(|episode| from_episode(&app, episode))
            .collect())
    }

    /// Set monitoring for a series-movie link and return the updated link.
    async fn set_series_movie_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Series-movie link identity and desired monitored state.")]
        input: SetSeriesMovieMonitoredInput,
    ) -> GqlResult<SeriesMovieLinkPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let series_movie_link_id = input.series_movie_link_id.to_string();
        let link = app
            .set_series_movie_monitored(&actor, &series_movie_link_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_series_movie_link(&app, link))
    }

    /// Adds and/or removes user tags across a set of series movies.
    ///
    /// Series-movie tags live on the link rather than on a title, so this is a
    /// separate mutation from `updateTitleTags`. The rules are the same ones:
    /// labels must already exist in the title-tag registry, they are normalized
    /// before they are stored, removals are applied before additions, and every
    /// link's series is checked for title-management rights before the first
    /// write.
    async fn update_series_movie_tags(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Series movies to patch plus the labels to add and remove.")]
        input: UpdateSeriesMovieTagsInput,
    ) -> GqlResult<Vec<SeriesMovieLinkPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let link_ids = input
            .series_movie_link_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        let add = input.add.unwrap_or_default();
        let remove = input.remove.unwrap_or_default();
        let links = app
            .update_series_movie_tags(&actor, &link_ids, &add, &remove)
            .await
            .map_err(to_gql_error)?;
        Ok(links
            .into_iter()
            .map(|link| from_series_movie_link(&app, link))
            .collect())
    }
}
