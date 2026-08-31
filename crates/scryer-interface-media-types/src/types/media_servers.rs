use super::{
    AppPermissionValue, EmbyConnectAddressStatusValue, EmbyConnectUserTypeValue,
    EmbyConnectionModeValue, EmbyLocalSetupMethodValue, ExternalAccountProviderValue,
    LibraryPermissionValue, MediaServerProviderValue,
};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

#[derive(SimpleObject, Clone)]
/// A provider-native playback link for a server linked to the current user.
pub struct MediaServerPlaybackLinkPayload {
    /// Connection identity. This is not a credential or provider token.
    pub connection_id: ID,
    /// User-configured server name.
    pub display_name: String,
    /// Provider that owns the destination item.
    pub provider: MediaServerProviderValue,
    /// Browser URL for the exact provider media item.
    pub href: String,
}

#[derive(SimpleObject, Clone)]
/// Runtime state for one external authentication connection.
pub struct ExternalAuthRuntimeConnectionPayload {
    /// Connection identity.
    pub id: ID,
    /// External provider type.
    pub provider: ExternalAccountProviderValue,
    /// Display name of the connection.
    pub display_name: String,
    /// Whether login through this connection is enabled.
    pub login_enabled: bool,
    /// Whether account linking through this connection is enabled.
    pub linking_enabled: bool,
    /// Whether Emby Connect is enabled for this connection.
    pub emby_connect_enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Runtime authentication providers, linking providers, and connections.
pub struct ExternalAuthRuntimeSettingsPayload {
    /// Providers enabled for login.
    pub login_providers: Vec<ExternalAccountProviderValue>,
    /// Providers enabled for account linking.
    pub linking_providers: Vec<ExternalAccountProviderValue>,
    /// Configured external authentication connections.
    pub connections: Vec<ExternalAuthRuntimeConnectionPayload>,
}

#[derive(InputObject, Clone)]
/// Invitation linking a user to an external provider account.
pub struct CreateExternalAccountInviteInput {
    /// User identity receiving the invitation.
    pub user_id: ID,
    /// External authentication connection identity.
    pub connection_id: ID,
    /// Provider represented by the connection.
    pub provider: ExternalAccountProviderValue,
    /// Provider-side user identifier used to match the account.
    pub provider_user_identifier: String,
    /// Optional provider-native user id.
    pub provider_user_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Mapping from an external media-server path to a local path.
pub struct MediaServerPathMappingPayload {
    /// Path reported by the external server.
    pub source_path: String,
    /// Corresponding local filesystem path.
    pub destination_path: String,
}

#[derive(InputObject, Clone)]
/// Path mapping used when connecting an external media server.
pub struct MediaServerPathMappingInput {
    /// Path reported by the external server.
    pub source_path: String,
    /// Corresponding local filesystem path.
    pub destination_path: String,
}

#[derive(SimpleObject, Clone)]
/// Default library permission grant for a media-server connection.
pub struct MediaServerDefaultLibraryGrantPayload {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Permissions granted by default.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
/// Default library permission grant to create with a media-server connection.
pub struct MediaServerDefaultLibraryGrantInput {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Permissions granted by default.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(SimpleObject, Clone)]
/// Configured media-server connection with secrets represented by presence flags.
pub struct MediaServerConnectionPayload {
    /// Connection identity.
    pub id: ID,
    /// Media-server provider type.
    pub provider: MediaServerProviderValue,
    /// Display name of the connection.
    pub display_name: String,
    /// Server base URL.
    pub base_url: String,
    /// Browser-facing URL used for playback deep links.
    pub external_url: Option<String>,
    /// Whether the connection is active.
    pub enabled: bool,
    /// Whether login through this server is enabled.
    pub login_enabled: bool,
    /// Whether account linking through this server is enabled.
    pub linking_enabled: bool,
    /// Whether automatic account addition is enabled.
    pub auto_add_enabled: bool,
    /// Default application permissions for auto-added users.
    pub default_app_permissions: Vec<AppPermissionValue>,
    /// Default library grants for auto-added users.
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrantPayload>,
    /// Whether a machine identity is configured.
    pub machine_id_present: bool,
    /// Whether an API key is configured.
    pub api_key_present: bool,
    /// Whether an Emby server id is configured.
    pub emby_server_id_present: bool,
    /// Whether Emby Connect is enabled.
    pub emby_connect_enabled: bool,
    /// Configured external-to-local path mappings.
    pub path_mappings: Vec<MediaServerPathMappingPayload>,
    /// Creation timestamp in RFC 3339 format.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp in RFC 3339 format.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a media-server connection.
pub struct DeleteMediaServerConnectionPayload {
    /// Deleted connection identity.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// User discovered on a Jellyfin server.
pub struct JellyfinServerUserPayload {
    /// Server-side user identity.
    pub id: String,
    /// Username on the server.
    pub username: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional avatar URL.
    pub avatar_url: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Availability state for grouped media-server users.
pub enum MediaServerUserGroupStatusValue {
    /// User discovery completed successfully.
    Ready,
    /// Discovery needs credentials.
    MissingCredentials,
    /// Discovery failed.
    Error,
}

#[derive(SimpleObject, Clone)]
/// User returned from a media-server account discovery operation.
pub struct MediaServerUserPayload {
    /// Server-side user identity.
    pub id: String,
    /// Username on the server.
    pub username: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional avatar URL.
    pub avatar_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Grouped media-server users and discovery status.
pub struct MediaServerUserGroupPayload {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Connection display name.
    pub connection_name: String,
    /// External provider type.
    pub provider: ExternalAccountProviderValue,
    /// Discovery status.
    pub status: MediaServerUserGroupStatusValue,
    /// Error detail when discovery failed.
    pub error_message: Option<String>,
    /// Users discovered from the server.
    pub users: Vec<MediaServerUserPayload>,
}

#[derive(SimpleObject, Clone)]
/// A media server discovered through Plex.
pub struct PlexServerDiscoveryPayload {
    /// Discovered server identity.
    pub id: String,
    /// Discovered server name.
    pub name: String,
}

#[derive(SimpleObject, Clone)]
/// Emby Connect server and its reachable addresses.
pub struct EmbyConnectServerPayload {
    /// Emby Connect server identity.
    pub server_id: String,
    /// Server display name.
    pub name: String,
    /// Emby Connect user category.
    pub user_type: EmbyConnectUserTypeValue,
    /// Local server address, when advertised.
    pub local_address: Option<String>,
    /// Remote server address, when advertised.
    pub remote_address: Option<String>,
    /// Local API base URL, when reachable.
    pub local_api_base_url: Option<String>,
    /// Remote API base URL, when reachable.
    pub remote_api_base_url: Option<String>,
    /// Local address probe result.
    pub local_status: EmbyConnectAddressStatusValue,
    /// Remote address probe result.
    pub remote_status: EmbyConnectAddressStatusValue,
    /// Server address selected by connection logic, when one is available.
    pub suggested_base_url: Option<String>,
}

#[derive(InputObject, Clone)]
/// Credentials used to discover Emby Connect servers.
pub struct DiscoverEmbyConnectServersInput {
    /// Emby Connect username or email.
    pub username_or_email: String,
    /// Emby Connect password.
    pub password: String,
}

#[derive(InputObject, Clone)]
/// Credentials used to test an Emby Connect server.
pub struct TestEmbyConnectInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Emby Connect username or email.
    pub username_or_email: String,
    /// Emby Connect password.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Result of a media-server connection test.
pub struct MediaServerConnectionTestPayload {
    /// Machine-readable test status.
    pub status: String,
    /// Optional human-readable detail.
    pub message: Option<String>,
}

#[derive(InputObject, Clone)]
/// Configuration and credentials for a new media-server connection.
pub struct CreateMediaServerConnectionInput {
    /// Media-server provider type.
    pub provider: MediaServerProviderValue,
    /// Display name for the connection.
    pub display_name: String,
    /// Server base URL.
    pub base_url: String,
    /// Browser-facing URL used for playback deep links. Omit to disable deep links for Jellyfin and Emby.
    pub external_url: Option<String>,
    /// Whether the connection is enabled, defaulting to true.
    pub enabled: Option<bool>,
    /// Whether login through the server is enabled, defaulting to false.
    pub login_enabled: Option<bool>,
    /// Whether account linking is enabled, defaulting to false.
    pub linking_enabled: Option<bool>,
    /// Whether automatic account addition is enabled, defaulting to false.
    pub auto_add_enabled: Option<bool>,
    /// Default application permissions for auto-added users.
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    /// Default library grants for auto-added users.
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    /// Provider machine identity, when required.
    pub machine_id: Option<String>,
    /// Plex authentication token; stored as a secret and not returned.
    pub plex_auth_token: Option<String>,
    /// Plex server identity.
    pub plex_server_id: Option<String>,
    /// Provider API key; stored as a secret and not returned.
    pub api_key: Option<String>,
    /// Provider administrator username.
    pub admin_username: Option<String>,
    /// Provider administrator password; stored as a secret and not returned.
    pub admin_password: Option<String>,
    /// Address-selection mode used for the Emby connection.
    pub emby_connection_mode: Option<EmbyConnectionModeValue>,
    /// Credential flow used to configure a local Emby server.
    pub emby_local_setup_method: Option<EmbyLocalSetupMethodValue>,
    /// Whether Emby Connect is enabled.
    pub emby_connect_enabled: Option<bool>,
    /// Emby Connect account name used to authenticate the connection.
    pub emby_connect_username_or_email: Option<String>,
    /// Emby Connect password; stored as a secret and not returned.
    pub emby_connect_password: Option<String>,
    /// Emby Connect server identity.
    pub emby_connect_server_id: Option<String>,
    /// External-to-local filesystem path mappings.
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
/// Patch for an existing media-server connection.
pub struct UpdateMediaServerConnectionInput {
    /// Media-server connection identity to patch.
    pub id: ID,
    /// Replacement provider type; omission preserves the current value.
    pub provider: Option<MediaServerProviderValue>,
    /// Replacement display name; omission preserves the current value.
    pub display_name: Option<String>,
    /// Replacement base URL; omission preserves the current value.
    pub base_url: Option<String>,
    /// Replacement browser-facing URL; omission preserves the current value and an empty value clears it.
    pub external_url: Option<String>,
    /// Replacement enabled state; omission preserves the current value.
    pub enabled: Option<bool>,
    /// Replacement login-enabled state; omission preserves the current value.
    pub login_enabled: Option<bool>,
    /// Replacement linking-enabled state; omission preserves the current value.
    pub linking_enabled: Option<bool>,
    /// Replacement auto-add state; omission preserves the current value.
    pub auto_add_enabled: Option<bool>,
    /// Replacement default application permissions; omission preserves the current list.
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    /// Replacement default library grants; omission preserves the current list.
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    /// Replacement machine identity; omission preserves it.
    pub machine_id: Option<String>,
    /// Whether the stored machine identity should be cleared.
    pub clear_machine_id: Option<bool>,
    /// Replacement Plex authentication token; omission preserves it and the value is never returned.
    pub plex_auth_token: Option<String>,
    /// Replacement Plex server identity; omission preserves it.
    pub plex_server_id: Option<String>,
    /// Replacement provider API key; omission preserves it and the value is never returned.
    pub api_key: Option<String>,
    /// Whether the stored API key should be cleared.
    pub clear_api_key: Option<bool>,
    /// Replacement provider administrator username; omission preserves it.
    pub admin_username: Option<String>,
    /// Replacement provider administrator password; omission preserves it and the value is never returned.
    pub admin_password: Option<String>,
    /// Replacement Emby connection mode; omission preserves it.
    pub emby_connection_mode: Option<EmbyConnectionModeValue>,
    /// Replacement Emby local setup method; omission preserves it.
    pub emby_local_setup_method: Option<EmbyLocalSetupMethodValue>,
    /// Replacement Emby Connect enabled state; omission preserves it.
    pub emby_connect_enabled: Option<bool>,
    /// Replacement Emby Connect username or email; omission preserves it.
    pub emby_connect_username_or_email: Option<String>,
    /// Replacement Emby Connect password; omission preserves it and the value is never returned.
    pub emby_connect_password: Option<String>,
    /// Replacement Emby Connect server identity; omission preserves it.
    pub emby_connect_server_id: Option<String>,
    /// Replacement path mappings; omission preserves the current mappings.
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
/// Credentials used to test an existing media-server connection.
pub struct TestMediaServerConnectionInput {
    /// Media-server connection identity to test.
    pub id: ID,
    /// Optional Plex authentication token used for this test.
    pub plex_auth_token: Option<String>,
}

#[derive(InputObject, Clone)]
/// Plex credentials used to link an external account.
pub struct LinkPlexAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Plex authentication token; used for linking and not returned.
    pub plex_auth_token: String,
}

#[derive(InputObject, Clone)]
/// Jellyfin credentials used to link an external account.
pub struct LinkJellyfinAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Jellyfin username.
    pub username: String,
    /// Jellyfin password; used for linking and not returned.
    pub password: String,
}

#[derive(InputObject, Clone)]
/// Emby credentials used to link an external account.
pub struct LinkEmbyAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Emby connection mode.
    pub mode: EmbyConnectionModeValue,
    /// Emby username.
    pub username: String,
    /// Emby password; used for linking and not returned.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after unlinking an external account.
pub struct UnlinkExternalAccountPayload {
    /// Linked-account identity that was removed.
    pub linked_account_id: ID,
}
