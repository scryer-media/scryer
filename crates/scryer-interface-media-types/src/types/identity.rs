use super::{AppPermissionValue, LibraryPermissionValue, UserAccountKindValue};
use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

/// Local username and password credentials with an optional TOTP code.
#[derive(InputObject)]
pub struct LoginInput {
    /// Local username to authenticate.
    pub username: String,
    /// Local password; never returned in a payload.
    pub password: String,
    /// Optional six-digit TOTP code, required only when password-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// External media provider used for account linking or login.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountProviderValue {
    /// Plex provider.
    Plex,
    /// Jellyfin provider.
    Jellyfin,
    /// Emby provider.
    Emby,
}

/// Media-server provider configured by the application.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaServerProviderValue {
    /// Jellyfin server.
    Jellyfin,
    /// Plex server.
    Plex,
    /// Emby server.
    Emby,
}

/// Emby connection mode.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectionModeValue {
    /// Connect directly to a local or remote Emby server.
    Local,
    /// Resolve and connect through Emby Connect.
    Connect,
}

/// Credential method for local Emby setup.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyLocalSetupMethodValue {
    /// Use an Emby API key.
    ApiKey,
    /// Use an Emby administrator username and password.
    AdminCredentials,
}

/// Reachability result for an Emby Connect address.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectAddressStatusValue {
    /// Address responded successfully.
    Reachable,
    /// Address could not be reached.
    Unreachable,
    /// Address is not a valid URL.
    InvalidUrl,
    /// Address responded for a different server ID.
    ServerIdMismatch,
}

/// Emby Connect user classification.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectUserTypeValue {
    /// User is linked to the server.
    LinkedUser,
    /// User is a guest.
    Guest,
    /// Provider did not classify the user.
    Unknown,
}

impl MediaServerProviderValue {
    pub fn into_domain(self) -> scryer_domain::MediaServerProvider {
        match self {
            Self::Jellyfin => scryer_domain::MediaServerProvider::Jellyfin,
            Self::Plex => scryer_domain::MediaServerProvider::Plex,
            Self::Emby => scryer_domain::MediaServerProvider::Emby,
        }
    }

    pub fn from_domain(provider: scryer_domain::MediaServerProvider) -> Self {
        match provider {
            scryer_domain::MediaServerProvider::Jellyfin => Self::Jellyfin,
            scryer_domain::MediaServerProvider::Plex => Self::Plex,
            scryer_domain::MediaServerProvider::Emby => Self::Emby,
        }
    }
}

impl ExternalAccountProviderValue {
    pub fn into_domain(self) -> scryer_domain::ExternalAccountProvider {
        match self {
            Self::Plex => scryer_domain::ExternalAccountProvider::Plex,
            Self::Jellyfin => scryer_domain::ExternalAccountProvider::Jellyfin,
            Self::Emby => scryer_domain::ExternalAccountProvider::Emby,
        }
    }

    pub fn from_domain(provider: scryer_domain::ExternalAccountProvider) -> Self {
        match provider {
            scryer_domain::ExternalAccountProvider::Plex => Self::Plex,
            scryer_domain::ExternalAccountProvider::Jellyfin => Self::Jellyfin,
            scryer_domain::ExternalAccountProvider::Emby => Self::Emby,
        }
    }
}

/// State of an external account link.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountStatusValue {
    /// Invite or link is awaiting a claim.
    PendingClaim,
    /// Link is active.
    Active,
    /// Link has been disabled.
    Disabled,
}

impl ExternalAccountStatusValue {
    pub fn from_domain(status: scryer_domain::ExternalAccountStatus) -> Self {
        match status {
            scryer_domain::ExternalAccountStatus::PendingClaim => Self::PendingClaim,
            scryer_domain::ExternalAccountStatus::Active => Self::Active,
            scryer_domain::ExternalAccountStatus::Disabled => Self::Disabled,
        }
    }
}

/// Plex credentials and the configured connection target used for login.
#[derive(InputObject)]
pub struct LoginWithPlexInput {
    /// ID of the configured Plex connection.
    pub connection_id: ID,
    /// Plex authentication token used only for login.
    pub plex_auth_token: String,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// Jellyfin credentials and optional MFA data for login.
#[derive(InputObject)]
pub struct LoginWithJellyfinInput {
    /// ID of the configured Jellyfin connection.
    pub connection_id: ID,
    /// Jellyfin username.
    pub username: String,
    /// Jellyfin password; never returned in a payload.
    pub password: String,
    /// Optional TOTP code used when Jellyfin-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// Emby credentials, connection mode, and optional MFA data for login.
#[derive(InputObject)]
pub struct LoginWithEmbyInput {
    /// ID of the configured Emby connection.
    pub connection_id: ID,
    /// Whether to use local Emby access or Emby Connect.
    pub mode: EmbyConnectionModeValue,
    /// Emby username.
    pub username: String,
    /// Emby password; never returned in a payload.
    pub password: String,
    /// Optional TOTP code used when Emby-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// WebAuthn assertion response paired with a previously issued challenge.
#[derive(InputObject)]
pub struct WebauthnCompleteInput {
    /// ID of the WebAuthn challenge to complete.
    pub challenge_id: ID,
    /// Browser assertion JSON; credential material is consumed for verification and not echoed.
    pub response_json: Json<serde_json::Value>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// TOTP or recovery-code completion for a previously verified primary login.
#[derive(InputObject)]
pub struct LoginVerificationTotpCompleteInput {
    /// Opaque ID returned in the MFA step-up error after primary authentication.
    pub login_challenge_id: ID,
    /// Current authenticator or recovery code; never returned in a payload.
    pub code: String,
}

/// Passkey completion for a previously verified primary login.
#[derive(InputObject)]
pub struct LoginVerificationPasskeyCompleteInput {
    /// Opaque ID returned in the MFA step-up error after primary authentication.
    pub login_challenge_id: ID,
    /// ID returned by `loginVerificationPasskeyStart`.
    pub webauthn_challenge_id: ID,
    /// Browser assertion JSON; credential material is consumed for verification and not echoed.
    pub response_json: Json<serde_json::Value>,
}

/// WebAuthn registration response paired with a previously issued challenge.
#[derive(InputObject)]
pub struct WebauthnRegisterCompleteInput {
    /// ID of the WebAuthn registration challenge.
    pub challenge_id: ID,
    /// Browser registration JSON; credential material is consumed for verification and not echoed.
    pub response_json: Json<serde_json::Value>,
    /// Optional display name for the new passkey; absent or null leaves it unset.
    pub friendly_name: Option<String>,
}

/// Session or restricted authentication result returned after authentication.
#[derive(SimpleObject, Clone)]
pub struct LoginPayload {
    /// Access token or short-lived MFA-enrollment token; clients must treat it as secret.
    pub token: String,
    /// Authenticated user summary without password or credential secrets.
    pub user: UserPayload,
    /// Token expiry as a UTC timestamp.
    pub expires_at: DateTime<Utc>,
    /// UTC time through which MFA verification remains fresh, or null when not verified.
    pub mfa_verified_until: Option<DateTime<Utc>>,
    /// UTC time through which account-security changes may be authorized.
    pub security_action_verified_until: Option<DateTime<Utc>>,
    /// True when the token can only complete MFA enrollment.
    pub mfa_enrollment_required: bool,
    /// True when the token can only replace an administrator-provided temporary password.
    pub password_change_required: bool,
    /// Whether the session was requested to persist.
    pub persist_session: bool,
}

/// TOTP enrollment completion request.
#[derive(InputObject)]
pub struct TotpEnrollmentCompleteInput {
    /// ID of the pending TOTP enrollment challenge.
    pub challenge_id: ID,
    /// Current TOTP code used to verify the enrollment.
    pub code: String,
}

/// TOTP verification request.
#[derive(InputObject)]
pub struct TotpVerifyInput {
    /// Current TOTP code; never returned in a payload.
    pub code: String,
}

/// New password selected after signing in with an administrator-provided temporary password.
#[derive(InputObject)]
pub struct CompleteRequiredPasswordChangeInput {
    /// New password; stored securely and never returned.
    pub password: String,
}

/// TOTP enrollment and usage status without exposing the shared secret.
#[derive(SimpleObject, Clone)]
pub struct TotpStatusPayload {
    /// Whether TOTP is enabled.
    pub enabled: bool,
    /// UTC time when TOTP was enrolled, or null when never enrolled.
    pub created_at: Option<DateTime<Utc>>,
    /// UTC time when TOTP was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Number of unused recovery codes remaining.
    pub recovery_codes_remaining: i32,
}

/// One-time values returned when TOTP enrollment starts.
#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentStartPayload {
    /// ID of the short-lived enrollment challenge.
    pub challenge_id: ID,
    /// `otpauth` URI for authenticator setup; treat it as secret.
    pub otpauth_url: String,
    /// Base32 TOTP secret; returned only for enrollment and must be protected as a secret.
    pub secret_base32: String,
    /// UTC time when the enrollment challenge expires.
    pub expires_at: DateTime<Utc>,
}

/// Result of completing TOTP enrollment or regenerating recovery codes.
#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentCompletePayload {
    /// Updated TOTP status without the shared secret.
    pub status: TotpStatusPayload,
    /// Newly generated one-time recovery codes; they are not returned again.
    pub recovery_codes: Vec<String>,
}

/// Result of completing MFA enrollment during login.
#[derive(SimpleObject, Clone)]
pub struct LoginMfaEnrollmentCompletePayload {
    /// Updated TOTP status without the shared secret.
    pub status: TotpStatusPayload,
    /// Newly generated one-time recovery codes.
    pub recovery_codes: Vec<String>,
    /// Authenticated login payload issued after enrollment completes.
    pub login: LoginPayload,
}

/// Result of completing passkey enrollment during a restricted login enrollment session.
#[derive(SimpleObject, Clone)]
pub struct LoginPasskeyEnrollmentCompletePayload {
    /// Registered passkey summary.
    pub passkey: PasskeySummaryPayload,
    /// Authenticated login payload issued after enrollment completes.
    pub login: LoginPayload,
}

/// WebAuthn registration or authentication challenge options.
#[derive(SimpleObject, Clone)]
pub struct WebauthnChallengePayload {
    /// ID of the short-lived WebAuthn challenge.
    pub challenge_id: ID,
    /// Browser options JSON; it contains challenge data and should not be persisted as a credential.
    pub options_json: Json<serde_json::Value>,
    /// UTC time when the challenge expires.
    pub expires_at: DateTime<Utc>,
}

/// Non-secret summary of a registered passkey.
#[derive(SimpleObject, Clone)]
pub struct PasskeySummaryPayload {
    /// ID of the passkey.
    pub id: ID,
    /// Optional user-assigned passkey name; null when none was saved.
    pub friendly_name: Option<String>,
    /// UTC time when the passkey was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the passkey was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Acknowledgement containing the ID of a deleted passkey.
#[derive(SimpleObject, Clone)]
pub struct DeleteMyPasskeyPayload {
    /// ID of the deleted passkey.
    pub id: ID,
}

/// Non-secret summary of an OAuth grant.
#[derive(SimpleObject, Clone)]
pub struct OAuthConnectedAppPayload {
    /// Grant ID used to revoke this authorization.
    pub grant_id: ID,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client display name.
    pub client_name: String,
    /// UTC time when authorization was granted.
    pub authorized_at: DateTime<Utc>,
    /// UTC time when the grant was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Expiration choices available when creating an API key.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ApiKeyExpiryPresetValue {
    /// Expires in 30 days.
    Days30,
    /// Expires in 90 days.
    Days90,
    /// Expires in one year.
    Days365,
    /// Does not expire automatically.
    Never,
}

/// Input for creating an API key owned by the interactive actor.
#[derive(InputObject)]
pub struct CreateMyApiKeyInput {
    /// A human-readable label to distinguish the integration.
    pub label: String,
    /// Expiration policy. Omission uses 90 days.
    pub expiry: Option<ApiKeyExpiryPresetValue>,
}

/// Non-secret API-key metadata.
#[derive(SimpleObject, Clone)]
pub struct ApiKeyPayload {
    /// API-key record ID.
    pub id: ID,
    /// Human-readable label.
    pub label: String,
    /// Auditable identity used for requests made with this key.
    pub actor: String,
    /// UTC expiry time, or null when the key does not expire.
    pub expires_at: Option<DateTime<Utc>>,
    /// UTC revocation time, or null while active.
    pub revoked_at: Option<DateTime<Utc>>,
    /// UTC last successful-use time, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// Provisioning source (`user` or `environment`).
    pub provisioning_source: String,
}

/// Creation result. `apiKey` is shown once and must be stored by the caller.
#[derive(SimpleObject)]
pub struct CreateMyApiKeyPayload {
    /// Generated raw API key. This is never returned again.
    pub api_key: String,
    /// Non-secret metadata for the generated key.
    pub key: ApiKeyPayload,
}

/// Revocation result for a user-owned API key.
#[derive(SimpleObject)]
pub struct RevokeMyApiKeyPayload {
    /// ID of the revoked key.
    pub id: ID,
    /// Whether the active key was revoked.
    pub revoked: bool,
}

/// Result of revoking an OAuth grant.
#[derive(SimpleObject, Clone)]
pub struct RevokeMyOauthAppPayload {
    /// Grant ID targeted by the revoke request.
    pub grant_id: ID,
    /// False when the grant was already revoked or was not owned by the caller.
    pub revoked: bool,
}

/// User summary with authorization and credential-presence flags, never credential values.
#[derive(SimpleObject, Clone)]
pub struct UserPayload {
    /// ID of the user.
    pub id: ID,
    /// Login username.
    pub username: String,
    /// Whether password or external login is enabled.
    pub login_enabled: bool,
    /// Whether this is the default administrator account.
    pub is_default_admin: bool,
    /// Whether a password is configured, without revealing it.
    pub has_password: bool,
    /// Whether a local password must be replaced before the next local-password session is full.
    pub password_change_required: bool,
    /// Whether MFA is configured, without revealing its secret.
    pub has_mfa: bool,
    /// Whether a passkey is configured.
    pub has_passkey: bool,
    /// Local or externally provisioned account origin.
    pub account_kind: UserAccountKindValue,
    /// Application-wide permissions granted to the user.
    pub app_permissions: Vec<AppPermissionValue>,
    /// Library-specific permission grants.
    pub library_permissions: Vec<UserLibraryPermissionGrantPayload>,
}

/// External account link summary without provider credentials.
#[derive(SimpleObject, Clone)]
pub struct LinkedAccountPayload {
    /// ID of the linked account.
    pub id: ID,
    /// ID of the linked local user.
    pub user_id: ID,
    /// External provider.
    pub provider: ExternalAccountProviderValue,
    /// ID of the configured media-server connection.
    pub connection_id: ID,
    /// Provider-specific user ID, or null when unavailable.
    pub external_user_id: Option<String>,
    /// Provider username.
    pub username: String,
    /// Provider display name, or null when unavailable.
    pub display_name: Option<String>,
    /// Provider avatar URL, or null when unavailable.
    pub avatar_url: Option<String>,
    /// Current link status.
    pub status: ExternalAccountStatusValue,
    /// UTC verification time, or null when not verified.
    pub verified_at: Option<DateTime<Utc>>,
    /// UTC time of the last successful login, or null when unused.
    pub last_login_at: Option<DateTime<Utc>>,
    /// UTC time when the link was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the link was last changed.
    pub updated_at: DateTime<Utc>,
}

/// Permissions granted to one library for one user.
#[derive(SimpleObject, Clone)]
pub struct UserLibraryPermissionGrantPayload {
    /// ID of the library receiving the grant.
    pub library_id: ID,
    /// Permissions granted within that library.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject)]
/// New user account and permission grants.
pub struct CreateUserInput {
    /// Login username.
    pub username: String,
    /// Temporary password; the user must replace it after local-password sign-in.
    pub password: String,
    /// Application permissions granted to the user.
    pub app_permissions: Vec<AppPermissionValue>,
    /// Library permissions granted to the user.
    pub library_permissions: Vec<LibraryPermissionGrantInput>,
}

#[derive(InputObject)]
/// Enable or disable login for a user identity.
pub struct SetUserLoginEnabledInput {
    /// User identity to update.
    pub user_id: ID,
    /// Whether login is enabled.
    pub enabled: bool,
}

#[derive(InputObject)]
/// Password replacement for a user account. Administrator-provided passwords are temporary.
pub struct SetUserPasswordInput {
    /// User identity whose password changes.
    pub user_id: ID,
    /// New password; stored securely and never returned.
    pub password: String,
    /// Current password when required by the authorization policy.
    pub current_password: Option<String>,
}

#[derive(InputObject)]
/// Replacement application permissions for a user.
pub struct SetUserAppPermissionsInput {
    /// User identity to update.
    pub user_id: ID,
    /// Application permissions to store.
    pub permissions: Vec<AppPermissionValue>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a user.
pub struct DeleteUserPayload {
    /// Deleted user identity.
    pub id: ID,
}

#[derive(InputObject, Clone)]
/// Library permission grant for a user.
pub struct LibraryPermissionGrantInput {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Library permissions to store.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
/// Replacement library permission grants for a user.
pub struct SetUserLibraryPermissionsInput {
    /// User identity to update.
    pub user_id: ID,
    /// Library grants to store.
    pub grants: Vec<LibraryPermissionGrantInput>,
}
