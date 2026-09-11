use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::{
    NotificationScopeIdUpdate, NotificationSubscriptionTargetCreate,
    NotificationSubscriptionTargetUpdate,
};
use scryer_domain::AppPermission;

use crate::context::{app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_notification_channel_with_fields, from_notification_subscription,
    provider_config_values_to_json,
};
use crate::types::*;

fn notification_config_key_looks_secret(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

fn merge_omitted_notification_secrets(
    incoming_json: String,
    existing_json: &str,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> scryer_application::AppResult<String> {
    let mut incoming = serde_json::from_str::<serde_json::Value>(&incoming_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let existing = serde_json::from_str::<serde_json::Value>(existing_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let Some(incoming_object) = incoming.as_object_mut() else {
        return Ok(incoming_json);
    };
    let Some(existing_object) = existing.as_object() else {
        return Ok(incoming_json);
    };
    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();

    for (key, value) in existing_object {
        let is_secret = configured_secret_keys.contains(key.as_str())
            || notification_config_key_looks_secret(key);
        if is_secret && !incoming_object.contains_key(key) {
            incoming_object.insert(key.clone(), value.clone());
        }
    }

    serde_json::to_string(&incoming)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))
}

#[derive(Default)]
pub(crate) struct NotificationMutations;

#[Object]
impl NotificationMutations {
    /// Create an enabled notification channel with provider configuration.
    async fn create_notification_channel(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Channel name, provider type, configuration values, optional media-server identity, and enabled state."
        )]
        input: CreateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let channel = app
            .create_notification_channel_with_media_server_connection_id(
                &actor,
                input.name,
                input.channel_type,
                config_json,
                input.media_server_connection_id.map(String::from),
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        let fields = app.notification_provider_config_fields(channel.channel_type.as_str());
        let mut payload = from_notification_channel_with_fields(channel, &fields);
        payload.blocked_reason = app
            .plugin_component_blocker_for_provider(&payload.channel_type)
            .await;
        Ok(payload)
    }

    /// Patch a notification channel while preserving omitted fields and provider secrets.
    async fn update_notification_channel(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Channel identity and optional replacement fields; omitted provider secrets remain stored and null clears the media-server link."
        )]
        input: UpdateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let existing = app
            .list_notification_channels(&actor)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .find(|channel| channel.id == input.id.as_ref())
            .ok_or_else(|| {
                to_gql_error(scryer_application::AppError::NotFound(format!(
                    "notification channel {}",
                    input.id.as_ref()
                )))
            })?;
        let fields = app.notification_provider_config_fields(existing.channel_type.as_str());
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?
            .map(|config_json| {
                merge_omitted_notification_secrets(config_json, &existing.config_json, &fields)
                    .map_err(to_gql_error)
            })
            .transpose()?;
        let channel = app
            .update_notification_channel_with_media_server_connection_id(
                &actor,
                input.id.to_string(),
                input.name,
                config_json,
                input
                    .media_server_connection_id
                    .map(|value| value.map(String::from)),
                input.is_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        let fields = app.notification_provider_config_fields(channel.channel_type.as_str());
        let mut payload = from_notification_channel_with_fields(channel, &fields);
        payload.blocked_reason = app
            .plugin_component_blocker_for_provider(&payload.channel_type)
            .await;
        Ok(payload)
    }

    /// Delete a notification channel.
    async fn delete_notification_channel(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Notification-channel identity to delete.")] id: ID,
    ) -> GqlResult<DeleteNotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteNotificationChannelPayload { id: ID::from(id) })
    }

    /// Test a notification channel and return the delivery result.
    async fn test_notification_channel(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Notification-channel identity to test.")] id: ID,
    ) -> GqlResult<NotificationChannelTestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.test_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(NotificationChannelTestPayload {
            id: ID::from(id),
            status: "ok".to_string(),
            message: Some("Notification channel test delivered.".to_string()),
            retry_after_seconds: None,
        })
    }

    /// Create an enabled notification subscription for a target and event.
    async fn create_notification_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional channel and target identities, event type, scope, and enabled state."
        )]
        input: CreateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let sub = app
            .create_notification_subscription_for_target(
                &actor,
                NotificationSubscriptionTargetCreate {
                    channel_id: input.channel_id.map(String::from),
                    target_kind: input.target_kind,
                    target_id: input.target_id.map(String::from),
                    event_type: input.event_type,
                    scope: input.scope,
                    scope_id: input.scope_id,
                    is_enabled: input.is_enabled.unwrap_or(true),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    /// Patch a notification subscription while preserving omitted fields.
    async fn update_notification_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Subscription identity and optional target, event, scope, or enabled-state replacements."
        )]
        input: UpdateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let sub = app
            .update_notification_subscription_target(
                &actor,
                NotificationSubscriptionTargetUpdate {
                    id: input.id.to_string(),
                    target_kind: input.target_kind,
                    target_id: input.target_id.map(String::from),
                    event_type: input.event_type,
                    scope: input.scope,
                    scope_id: input
                        .scope_id
                        .map(NotificationScopeIdUpdate::Set)
                        .unwrap_or(NotificationScopeIdUpdate::NoChange),
                    is_enabled: input.is_enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    /// Delete a notification subscription.
    async fn delete_notification_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Notification-subscription identity to delete.")] id: ID,
    ) -> GqlResult<DeleteNotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_notification_subscription(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteNotificationSubscriptionPayload { id: ID::from(id) })
    }
}
