use super::{
    MediaServerProviderValue, PluginConfigFieldPayload, ProviderConfigValueInput,
    ProviderConfigValuePayload,
};
use async_graphql::{ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

// ── Notification types ─────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Configured notification channel and its redacted provider settings.
pub struct NotificationChannelPayload {
    /// Notification channel ID.
    pub id: ID,
    /// Channel display name.
    pub name: String,
    /// Provider channel type.
    pub channel_type: String,
    /// Non-secret channel configuration values.
    pub config: Vec<ProviderConfigValuePayload>,
    /// Configuration keys whose secret values are stored but not returned.
    pub stored_secret_keys: Vec<String>,
    /// Media-server connection used by this channel, when applicable.
    pub media_server_connection_id: Option<ID>,
    /// Whether notifications are enabled for this channel.
    pub is_enabled: bool,
    /// Why the channel's plugin is blocked on this version of Scryer, or null when it is running.
    pub blocked_reason: Option<String>,
    /// Channel creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Time of the latest channel update in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Subscription connecting a notification channel to an event target and scope.
pub struct NotificationSubscriptionPayload {
    /// Subscription ID.
    pub id: ID,
    /// Notification channel ID, or null when the subscription has no channel.
    pub channel_id: Option<ID>,
    /// Target category for the subscription.
    pub target_kind: String,
    /// ID of the subscribed target.
    pub target_id: ID,
    /// Event type that triggers delivery.
    pub event_type: String,
    /// Scope category used to limit matching events.
    pub scope: String,
    /// Scope key, or null when the scope is global.
    pub scope_id: Option<String>,
    /// Whether this subscription is enabled.
    pub is_enabled: bool,
    /// Subscription creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Time of the latest subscription update in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted notification channel.
pub struct DeleteNotificationChannelPayload {
    /// Deleted notification channel ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of testing a notification channel configuration.
pub struct NotificationChannelTestPayload {
    /// Tested notification channel ID.
    pub id: async_graphql::ID,
    /// Test outcome status.
    pub status: String,
    /// Provider response or failure detail, when available.
    pub message: Option<String>,
    /// Suggested retry delay in seconds, when the provider requests one.
    pub retry_after_seconds: Option<i64>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted notification subscription.
pub struct DeleteNotificationSubscriptionPayload {
    /// Deleted notification subscription ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Enabled notification target available for subscription.
pub struct NotificationTargetPayload {
    /// Target ID.
    pub id: ID,
    /// Target category.
    pub target_kind: String,
    /// Target display name.
    pub name: String,
    /// Provider type associated with the target.
    pub provider_type: String,
    /// Media-server provider, when the target is a media-server connection.
    pub media_server_provider: Option<MediaServerProviderValue>,
    /// Media-server connection ID, when applicable.
    pub media_server_connection_id: Option<ID>,
    /// Whether this target can receive notifications.
    pub is_enabled: bool,
}

#[derive(InputObject)]
/// Values required to create a notification channel.
pub struct CreateNotificationChannelInput {
    /// Channel display name.
    pub name: String,
    /// Provider channel type.
    pub channel_type: String,
    /// Channel configuration values; secret values are stored securely.
    pub config: Vec<ProviderConfigValueInput>,
    /// Media-server connection ID for channel providers that use one.
    pub media_server_connection_id: Option<ID>,
    /// Whether the new channel starts enabled; omitted uses the service default.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing notification channel.
pub struct UpdateNotificationChannelInput {
    /// Notification channel ID to update.
    pub id: ID,
    /// Replacement channel display name, or null to leave it unchanged.
    pub name: Option<String>,
    /// Replacement configuration, or null to leave it unchanged.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement media-server connection; an explicit null clears it.
    pub media_server_connection_id: Option<Option<ID>>,
    /// Replacement enabled state, or null to leave it unchanged.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values required to subscribe a notification channel to an event target.
pub struct CreateNotificationSubscriptionInput {
    /// Notification channel ID, or null for target-only event handling.
    pub channel_id: Option<ID>,
    /// Target category, or null when the event is not target-specific.
    pub target_kind: Option<String>,
    /// Target ID, or null when the event is not target-specific.
    pub target_id: Option<ID>,
    /// Event type that triggers delivery.
    pub event_type: String,
    /// Scope category used to limit matching events.
    pub scope: String,
    /// Scope key, or null for a global scope.
    pub scope_id: Option<String>,
    /// Whether the new subscription starts enabled; omitted uses the service default.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing notification subscription.
pub struct UpdateNotificationSubscriptionInput {
    /// Notification subscription ID to update.
    pub id: ID,
    /// Replacement target category, or null to leave it unchanged.
    pub target_kind: Option<String>,
    /// Replacement target ID, or null to leave it unchanged.
    pub target_id: Option<ID>,
    /// Replacement event type, or null to leave it unchanged.
    pub event_type: Option<String>,
    /// Replacement scope category, or null to leave it unchanged.
    pub scope: Option<String>,
    /// Replacement scope key, or null to leave it unchanged.
    pub scope_id: Option<String>,
    /// Replacement enabled state, or null to leave it unchanged.
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Notification provider type and the fields accepted in its configuration.
pub struct NotificationProviderTypePayload {
    /// Stable notification provider implementation key.
    pub provider_type: String,
    /// Provider display name.
    pub name: String,
    /// Configuration field definitions for this provider.
    pub config_fields: Vec<PluginConfigFieldPayload>,
}
