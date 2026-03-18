use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_collection, from_episode};
use crate::types::*;

#[derive(Default)]
pub(crate) struct CollectionMutations;

#[Object]
impl CollectionMutations {
    async fn create_collection(
        &self,
        ctx: &Context<'_>,
        input: CreateCollectionInput,
    ) -> GqlResult<CollectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .create_collection(
                &actor,
                input.title_id,
                input.collection_type,
                input.collection_index,
                input.label,
                input.ordered_path,
                input.first_episode_number,
                input.last_episode_number,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_collection(collection))
    }

    async fn update_collection(
        &self,
        ctx: &Context<'_>,
        input: UpdateCollectionInput,
    ) -> GqlResult<CollectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .update_collection(
                &actor,
                input.collection_id,
                input.collection_type,
                input.collection_index,
                input.label,
                input.ordered_path,
                input.first_episode_number,
                input.last_episode_number,
                input.monitored,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_collection(collection))
    }

    async fn delete_collection(
        &self,
        ctx: &Context<'_>,
        input: DeleteCollectionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_collection(&actor, &input.collection_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn set_collection_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetCollectionMonitoredInput,
    ) -> GqlResult<SetCollectionMonitoredPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .set_collection_monitored(&actor, &input.collection_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        let episodes = app
            .list_episodes(&actor, &input.collection_id)
            .await
            .map_err(to_gql_error)?;
        Ok(SetCollectionMonitoredPayload {
            id: collection.id,
            monitored: collection.monitored,
            episodes: episodes.into_iter().map(from_episode).collect(),
        })
    }

    async fn create_episode(
        &self,
        ctx: &Context<'_>,
        input: CreateEpisodeInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .create_episode(
                &actor,
                input.title_id,
                input.collection_id,
                input.episode_type,
                input.episode_number,
                input.season_number,
                input.episode_label,
                input.title,
                input.air_date,
                input.duration_seconds,
                input.has_multi_audio,
                input.has_subtitle,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(episode))
    }

    async fn update_episode(
        &self,
        ctx: &Context<'_>,
        input: UpdateEpisodeInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .update_episode(
                &actor,
                input.episode_id,
                input.episode_type,
                input.episode_number,
                input.season_number,
                input.episode_label,
                input.title,
                input.air_date,
                input.duration_seconds,
                input.has_multi_audio,
                input.has_subtitle,
                input.monitored,
                input.collection_id,
                input.overview,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(episode))
    }

    async fn delete_episode(
        &self,
        ctx: &Context<'_>,
        input: DeleteEpisodeInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_episode(&actor, &input.episode_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn set_episode_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetEpisodeMonitoredInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .set_episode_monitored(&actor, &input.episode_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(episode))
    }
}
