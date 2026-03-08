use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_notification_channel, from_notification_subscription};
use crate::types::*;

#[derive(Default)]
pub(crate) struct NotificationMutations;

#[Object]
impl NotificationMutations {
    async fn create_notification_channel(
        &self,
        ctx: &Context<'_>,
        input: CreateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let channel = app
            .create_notification_channel(
                &actor,
                input.name,
                input.channel_type,
                input.config_json,
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_channel(channel))
    }

    async fn update_notification_channel(
        &self,
        ctx: &Context<'_>,
        input: UpdateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let channel = app
            .update_notification_channel(
                &actor,
                input.id,
                input.name,
                input.config_json,
                input.is_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_channel(channel))
    }

    async fn delete_notification_channel(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn test_notification_channel(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.test_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn create_notification_subscription(
        &self,
        ctx: &Context<'_>,
        input: CreateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let sub = app
            .create_notification_subscription(
                &actor,
                input.channel_id,
                input.event_type,
                input.scope,
                input.scope_id,
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    async fn update_notification_subscription(
        &self,
        ctx: &Context<'_>,
        input: UpdateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let sub = app
            .update_notification_subscription(
                &actor,
                input.id,
                input.event_type,
                input.scope,
                input.scope_id.map(Some),
                input.is_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    async fn delete_notification_subscription(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_notification_subscription(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }
}
