use super::*;

#[cfg(not(test))]
const MEDIA_SERVER_USER_LIST_TIMEOUT: std::time::Duration =
    scryer_outbound_http::STANDARD_HTTP_TIMEOUT;
#[cfg(test)]
const MEDIA_SERVER_USER_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbyConnectionMode {
    Local,
    Connect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbyLocalSetupMethod {
    ApiKey,
    AdminCredentials,
}

#[derive(Clone)]
pub struct MediaServerConnectionDraft {
    pub provider: MediaServerProvider,
    pub display_name: String,
    pub base_url: String,
    pub external_url: Option<String>,
    pub enabled: bool,
    pub login_enabled: bool,
    pub linking_enabled: bool,
    pub auto_add_enabled: bool,
    pub default_app_permissions: AppPermissionMask,
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrant>,
    pub machine_id: Option<String>,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub emby_connection_mode: Option<EmbyConnectionMode>,
    pub emby_local_setup_method: Option<EmbyLocalSetupMethod>,
    pub emby_connect_enabled: Option<bool>,
    pub emby_connect_username_or_email: Option<String>,
    pub emby_connect_password: Option<String>,
    pub emby_connect_server_id: Option<String>,
    pub path_mappings: Vec<MediaServerPathMapping>,
}

#[derive(Clone, Default)]
pub struct MediaServerConnectionPatch {
    pub id: String,
    pub provider: Option<MediaServerProvider>,
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    pub external_url: Option<String>,
    pub enabled: Option<bool>,
    pub login_enabled: Option<bool>,
    pub linking_enabled: Option<bool>,
    pub auto_add_enabled: Option<bool>,
    pub default_app_permissions: Option<AppPermissionMask>,
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrant>>,
    pub machine_id: Option<String>,
    pub clear_machine_id: bool,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub emby_connection_mode: Option<EmbyConnectionMode>,
    pub emby_local_setup_method: Option<EmbyLocalSetupMethod>,
    pub emby_connect_enabled: Option<bool>,
    pub emby_connect_username_or_email: Option<String>,
    pub emby_connect_password: Option<String>,
    pub emby_connect_server_id: Option<String>,
    pub path_mappings: Option<Vec<MediaServerPathMapping>>,
}

#[derive(Clone, Debug, Default)]
struct ResolvedPlexServerSelection {
    machine_id: Option<String>,
}

struct ResolvedEmbyCredentials {
    base_url: String,
    api_key: String,
    server_id: String,
    connect_enabled: bool,
    cleanup: Option<EmbyApiKeyExchangeCleanup>,
}

mod connections;
mod emby;
mod jellyfin;
mod playback;
mod plex;
mod policy;
mod scanner;
mod users;

pub use playback::MediaServerPlaybackLink;
pub use scanner::start_background_media_server_playback_reconciliation_loop;

use policy::*;

#[cfg(test)]
mod tests;
