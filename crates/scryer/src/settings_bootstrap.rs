use std::sync::Arc;

use scryer_application::{
    ANIME_PATH_KEY, ANIME_ROOT_FOLDERS_KEY, API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY,
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
    AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY, AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
    BACKUP_PATH_KEY, CHOWN_GROUP_KEY, DEFAULT_SEEDING_PROFILE_SETTING_KEY,
    DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
    FILE_CHMOD_KEY, FOLDER_CHMOD_KEY, FOLDER_TEMPLATE_KEY, FORM_LOGIN_ENABLED_KEY,
    HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, IMAGE_CACHE_MAX_SIZE_MB_KEY,
    IMPORT_MODE_KEY, INDEXER_ROUTING_SETTINGS_KEY, LEGACY_NZBGET_CATEGORY_SETTING_KEY,
    LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY,
    MAINTENANCE_GATE_EVALUATION_KEY, MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY,
    MAINTENANCE_GATE_RESULT_DISPLAY_KEY, MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY,
    METADATA_LANGUAGE_KEY, MFA_REQUIRE_CONFIG_STEP_UP_KEY, MFA_REQUIRE_PASSWORD_LOGIN_KEY,
    MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON, MINIMUM_SEEDERS_FLOOR_SETTING_KEY, MOVIES_ROOT_FOLDERS_KEY,
    NZBGET_OLDER_PRIORITY_SETTING_KEY, NZBGET_RECENT_PRIORITY_SETTING_KEY, PASSWORD_MIN_LENGTH_KEY,
    POST_PROCESSING_SCRIPT_ANIME_KEY, POST_PROCESSING_SCRIPT_MOVIE_KEY,
    POST_PROCESSING_SCRIPT_SERIES_KEY, POST_PROCESSING_TIMEOUT_KEY, QUALITY_PROFILE_CATALOG_KEY,
    QUALITY_PROFILE_ID_KEY, QUALITY_PROFILE_INHERIT_VALUE, QualityProfile,
    QualityProfileRepository, RECYCLE_BIN_ENABLED_KEY, RENAME_COLLISION_POLICY_GLOBAL_KEY,
    RENAME_COLLISION_POLICY_KEY, RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY, RENAME_ENABLED_KEY,
    RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, RENAME_MISSING_METADATA_POLICY_KEY,
    RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY, RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    RENAME_TEMPLATE_KEY, RENAME_TEMPLATE_MOVIE_GLOBAL_KEY, RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
    REQUEST_QUALITY_PROFILE_IDS_KEY, REQUIRED_AUDIO_LANGUAGES_KEY, SCORING_PERSONA_KEY,
    SEASON_FOLDER_TEMPLATE_KEY, SERIES_ROOT_FOLDERS_KEY, SET_PERMISSIONS_LINUX_KEY,
    SETUP_COMPLETE_KEY, SKIP_LOGIN_FOR_LOCAL_IPS_KEY, SPECIALS_FOLDER_TEMPLATE_KEY,
    TITLE_METADATA_LANGUAGE_OVERRIDE_KEY, TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
    TLS_CERT_PATH_KEY as TLS_CERT_KEY, TLS_KEY_PATH_KEY as TLS_KEY_KEY,
    TOTP_REQUIRE_EMBY_LOGIN_KEY, TOTP_REQUIRE_JELLYFIN_LOGIN_KEY, USE_SEASON_FOLDERS_KEY,
    builtin_4k_profile, builtin_1080p_profile, builtin_anime_profile,
    builtin_default_quality_profile,
};
pub(crate) use scryer_application::{
    MOVIES_PATH_KEY, SERIES_PATH_KEY, SETTINGS_SCOPE_MEDIA, SETTINGS_SCOPE_SYSTEM,
};
use scryer_infrastructure_configuration::settings::{
    quality_profile_store::QualityProfileStore, settings_store::SettingsStore,
};
use serde_json::{Value, json};

use crate::{
    normalize_env_option, normalize_env_option_with_legacy,
    startup_migrations::_0002_enhanced_subsync_plugin_016::ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY,
    startup_migrations::_0003_title_image_artwork_url_refresh::TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY,
};

pub(crate) const SETTINGS_CATEGORY_SERVICE: &str = "service";
pub(crate) const SETTINGS_CATEGORY_MEDIA: &str = "media";
pub(crate) const SETTINGS_CATEGORY_ACQUISITION: &str = "acquisition";
pub(crate) const SETTINGS_CATEGORY_POST_PROCESSING: &str = "post_processing";
pub(crate) const SETTINGS_CATEGORY_SUBTITLES: &str = "subtitles";
pub(crate) const SETTINGS_CATEGORY_GENERAL: &str = "general";
pub(crate) const SETTINGS_CATEGORY_SECURITY: &str = "security";

#[derive(Debug)]
pub(crate) struct ServiceSettingSeed {
    pub(crate) category: &'static str,
    pub(crate) scope: &'static str,
    pub(crate) key_name: &'static str,
    pub(crate) data_type: &'static str,
    pub(crate) default_value_json: &'static str,
    pub(crate) is_sensitive: bool,
}

pub(crate) fn service_setting_seeds() -> &'static [ServiceSettingSeed] {
    &[
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nzbget.url",
            data_type: "string",
            default_value_json: "\"http://127.0.0.1:6789\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nzbget.username",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nzbget.password",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nzbget.dupe_mode",
            data_type: "string",
            default_value_json: "\"SCORE\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: MOVIES_PATH_KEY,
            data_type: "string",
            default_value_json: "\"/data/movies\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: SERIES_PATH_KEY,
            data_type: "string",
            default_value_json: "\"/data/series\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: ANIME_PATH_KEY,
            data_type: "string",
            default_value_json: "\"/data/anime\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: MOVIES_ROOT_FOLDERS_KEY,
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: SERIES_ROOT_FOLDERS_KEY,
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: ANIME_ROOT_FOLDERS_KEY,
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: RECYCLE_BIN_ENABLED_KEY,
            data_type: "boolean",
            default_value_json: "true",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: METADATA_LANGUAGE_KEY,
            data_type: "string",
            default_value_json: "\"eng\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: USE_SEASON_FOLDERS_KEY,
            data_type: "boolean",
            default_value_json: "true",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "catalog.title_metadata_rehydration_017_state",
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: crate::startup_migrations::_0007_emby_plugin_compatibility::MIGRATION_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: crate::startup_migrations::_0010_download_client_remove_failed_default::DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: crate::startup_migrations::_0008_title_credits_rehydration_018::TITLE_CREDITS_REHYDRATION_018_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: IMPORT_MODE_KEY,
            data_type: "string",
            default_value_json: "\"hardlink_or_copy\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SET_PERMISSIONS_LINUX_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: FILE_CHMOD_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: FOLDER_CHMOD_KEY,
            data_type: "string",
            default_value_json: "\"755\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: CHOWN_GROUP_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: HISTORY_KEEP_FOREVER_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: HISTORY_RETENTION_DAYS_KEY,
            data_type: "number",
            default_value_json: "180",
            is_sensitive: false,
        },
        // Seeding this for existing installs as well as fresh ones is what
        // gives every torrent indexer Sonarr's default of 1 without a
        // per-indexer migration. Operators who want the old behaviour set it
        // to 0.
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MINIMUM_SEEDERS_FLOOR_SETTING_KEY,
            data_type: "number",
            default_value_json: MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON,
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: IMAGE_CACHE_MAX_SIZE_MB_KEY,
            data_type: "number",
            default_value_json: "256",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: scryer_application::PLUGIN_HTTP_CA_BUNDLE_PEM_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: scryer_application::PLUGIN_AUTO_UPDATE_ENABLED_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: AUTO_BACKUP_ENABLED_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
            data_type: "string",
            default_value_json: "\"03:00\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: AUTO_BACKUP_KEY_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: BACKUP_PATH_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: scryer_application::AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: FORM_LOGIN_ENABLED_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: PASSWORD_MIN_LENGTH_KEY,
            data_type: "integer",
            default_value_json: "8",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SKIP_LOGIN_FOR_LOCAL_IPS_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TOTP_REQUIRE_JELLYFIN_LOGIN_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TOTP_REQUIRE_EMBY_LOGIN_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SECURITY,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MFA_REQUIRE_PASSWORD_LOGIN_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_TEMPLATE_MOVIE_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"{title} ({year}) - {quality}.{ext}\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"{title} - S{season:2}E{episode:2} - {quality}.{ext}\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_TEMPLATE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_ENABLED_KEY,
            data_type: "boolean",
            default_value_json: "true",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: FOLDER_TEMPLATE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SEASON_FOLDER_TEMPLATE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SPECIALS_FOLDER_TEMPLATE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_COLLISION_POLICY_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"skip\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"skip\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_COLLISION_POLICY_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"fallback_title\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
            data_type: "string",
            default_value_json: "\"fallback_title\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: RENAME_MISSING_METADATA_POLICY_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: QUALITY_PROFILE_ID_KEY,
            data_type: "string",
            // Must stay in lockstep with BUILTIN_DEFAULT_QUALITY_PROFILE_ID;
            // a guardrail test asserts the two agree.
            default_value_json: "\"1080p\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: crate::startup_migrations::_0006_quality_profile_default_1080p::QUALITY_PROFILE_DEFAULT_1080P_MIGRATION_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: REQUEST_QUALITY_PROFILE_IDS_KEY,
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: QUALITY_PROFILE_CATALOG_KEY,
            data_type: "string",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SCORING_PERSONA_KEY,
            data_type: "string",
            default_value_json: "\"Balanced\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: REQUIRED_AUDIO_LANGUAGES_KEY,
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TITLE_METADATA_LANGUAGE_OVERRIDE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: AUDIO_PERSONA_MIGRATION_SENTINEL_KEY,
            data_type: "bool",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: LEGACY_NZBGET_CATEGORY_SETTING_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: NZBGET_RECENT_PRIORITY_SETTING_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: NZBGET_OLDER_PRIORITY_SETTING_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: DEFAULT_SEEDING_PROFILE_SETTING_KEY,
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            data_type: "string",
            default_value_json: "{}",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            data_type: "string",
            default_value_json: "{}",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: INDEXER_ROUTING_SETTINGS_KEY,
            data_type: "string",
            default_value_json: "{}",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TLS_CERT_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TLS_KEY_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "encryption.master_key",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "last_run_version",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "scheduler.instance_id",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        // SMG (Scryer Metadata Gateway) enrollment
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.instance_id",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.client_key",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.client_cert",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.cert_expires_at",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.ca_cert",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.pq_seed",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.pq_public_key",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.pq_key_id",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.pq_enrollment_generation",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.version_compatibility_notice",
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "smg.scryer_update_notice",
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        // Anime settings
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.filler_policy",
            data_type: "string",
            default_value_json: "\"download_all\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.recap_policy",
            data_type: "string",
            default_value_json: "\"skip_recap\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.monitor_specials",
            data_type: "string",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.inter_season_movies",
            data_type: "string",
            default_value_json: "\"true\"",
            is_sensitive: false,
        },
        // Acquisition settings
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.enabled",
            data_type: "boolean",
            default_value_json: "true",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.upgrade_cooldown_hours",
            data_type: "number",
            default_value_json: "24",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.same_tier_min_delta",
            data_type: "number",
            default_value_json: "120",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.cross_tier_min_delta",
            data_type: "number",
            default_value_json: "30",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.forced_upgrade_delta_bypass",
            data_type: "number",
            default_value_json: "400",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.poll_interval_seconds",
            data_type: "number",
            default_value_json: "60",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.long_tail_backfill_max_scopes_per_cycle",
            data_type: "number",
            default_value_json: "500",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.long_tail_reconverge_days",
            data_type: "number",
            default_value_json: "30",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: crate::startup_migrations::_0011_long_tail_reconverge_default::MIGRATION_STATE_KEY,
            data_type: "string",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.convergence_seeded_at",
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.convergence_resume_after",
            data_type: "json",
            default_value_json: "null",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.delay_profiles",
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        // NFO sidecar writing on import
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nfo.write_on_import.movie",
            data_type: "boolean",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nfo.write_on_import.series",
            data_type: "boolean",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "nfo.write_on_import.anime",
            data_type: "boolean",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        // Plexmatch hint writing on import (series/anime only — Plex does not
        // support .plexmatch for movies)
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "plexmatch.write_on_import.series",
            data_type: "boolean",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "plexmatch.write_on_import.anime",
            data_type: "boolean",
            default_value_json: "\"false\"",
            is_sensitive: false,
        },
        // Setup wizard
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SERVICE,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: SETUP_COMPLETE_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        // Post-processing scripts
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_POST_PROCESSING,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: POST_PROCESSING_SCRIPT_MOVIE_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_POST_PROCESSING,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: POST_PROCESSING_SCRIPT_SERIES_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_POST_PROCESSING,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: POST_PROCESSING_SCRIPT_ANIME_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_POST_PROCESSING,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: POST_PROCESSING_TIMEOUT_KEY,
            data_type: "number",
            default_value_json: "1800",
            is_sensitive: false,
        },
        // ── Anime ─────────────────────────────────────────────────────
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.monitor_filler_movies",
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        // ── Subtitles ──────────────────────────────────────────────────
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.enabled",
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.opensubtitles_api_key",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.opensubtitles_username",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.opensubtitles_password",
            data_type: "string",
            default_value_json: "null",
            is_sensitive: true,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.languages",
            data_type: "json",
            default_value_json: "[]",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.auto_download_on_import",
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.minimum_score_series",
            data_type: "number",
            default_value_json: "90",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.minimum_score_movie",
            data_type: "number",
            default_value_json: "70",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.search_interval_hours",
            data_type: "number",
            default_value_json: "6",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.include_ai_translated",
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.include_machine_translated",
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.sync_enabled",
            data_type: "boolean",
            default_value_json: "true",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.sync_threshold_series",
            data_type: "number",
            default_value_json: "90",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.sync_threshold_movie",
            data_type: "number",
            default_value_json: "70",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "subtitles.sync_max_offset_seconds",
            data_type: "number",
            default_value_json: "60",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_SUBTITLES,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY,
            data_type: "string",
            default_value_json: "\"none\"",
            is_sensitive: false,
        },
        // The five maintenance gates (RFC 137 section 10). Every default is
        // false, and they are seeded separately rather than as one blob so a
        // partial write can never arm a gate the operator did not ask for.
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MAINTENANCE_GATE_EVALUATION_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MAINTENANCE_GATE_RESULT_DISPLAY_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_GENERAL,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY,
            data_type: "boolean",
            default_value_json: "false",
            is_sensitive: false,
        },
    ]
}

pub(crate) async fn seed_service_setting_definitions(
    database: Arc<SettingsStore>,
) -> Result<(), String> {
    let definitions: Vec<scryer_infrastructure_sql::types::SettingDefinitionSeed> =
        service_setting_seeds()
            .iter()
            .map(
                |seed| scryer_infrastructure_sql::types::SettingDefinitionSeed {
                    category: seed.category.to_string(),
                    scope: seed.scope.to_string(),
                    key_name: seed.key_name.to_string(),
                    data_type: seed.data_type.to_string(),
                    default_value_json: seed.default_value_json.to_string(),
                    is_sensitive: seed.is_sensitive,
                    validation_json: None,
                },
            )
            .collect();

    database
        .batch_ensure_setting_definitions(definitions)
        .await
        .map_err(|error| format!("failed to batch seed setting definitions: {error}"))
}

pub(crate) async fn seed_service_settings_from_environment(
    database: Arc<SettingsStore>,
) -> Result<(), String> {
    let env_settings: Vec<(&str, &str, Option<Value>)> = vec![
        (
            SETTINGS_SCOPE_SYSTEM,
            "nzbget.url",
            normalize_env_option("SCRYER_NZBGET_URL").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_SYSTEM,
            "nzbget.username",
            normalize_env_option("SCRYER_NZBGET_USERNAME").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_SYSTEM,
            "nzbget.password",
            normalize_env_option("SCRYER_NZBGET_PASSWORD").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_SYSTEM,
            "nzbget.dupe_mode",
            normalize_env_option_with_legacy(["SCRYER_NZBGET_DUPE_MODE", "SCRYER_NZBGET_DUPEMODE"])
                .map(|v: String| Value::String(v.to_uppercase())),
        ),
        (
            SETTINGS_SCOPE_MEDIA,
            MOVIES_PATH_KEY,
            normalize_env_option("SCRYER_MOVIES_PATH").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_MEDIA,
            SERIES_PATH_KEY,
            normalize_env_option("SCRYER_SERIES_PATH").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_MEDIA,
            ANIME_PATH_KEY,
            normalize_env_option("SCRYER_ANIME_PATH").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_SYSTEM,
            TLS_CERT_KEY,
            normalize_env_option("SCRYER_TLS_CERT").map(Value::String),
        ),
        (
            SETTINGS_SCOPE_SYSTEM,
            TLS_KEY_KEY,
            normalize_env_option("SCRYER_TLS_KEY").map(Value::String),
        ),
    ];

    let entries: Vec<(String, String, String, String)> = env_settings
        .into_iter()
        .filter_map(|(scope, key, value)| {
            value.map(|v| {
                (
                    scope.to_string(),
                    key.to_string(),
                    v.to_string(),
                    "env".to_string(),
                )
            })
        })
        .collect();

    if entries.is_empty() {
        return Ok(());
    }

    database
        .batch_upsert_settings_if_not_overridden(entries)
        .await
        .map_err(|error| format!("failed to batch persist env settings: {error}"))
}

pub(crate) async fn migrate_legacy_download_client_routing_settings(
    database: Arc<SettingsStore>,
) -> Result<(), String> {
    for scope_id in [None, Some("movie"), Some("series"), Some("anime")] {
        let scope_id_string = scope_id.map(str::to_string);
        let current = database
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id_string.clone(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to read {DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY} during bootstrap migration for scope {:?}: {error}",
                    scope_id
                )
            })?;

        if current
            .as_ref()
            .and_then(|record| record.value_json.as_ref())
            .is_some()
        {
            continue;
        }

        let legacy = database
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id_string.clone(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to read {LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY} during bootstrap migration for scope {:?}: {error}",
                    scope_id
                )
            })?;

        let Some(legacy_value_json) = legacy.and_then(|record| record.value_json) else {
            continue;
        };

        database
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id_string,
                legacy_value_json,
                "legacy-migration",
                None,
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to persist migrated {DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY} for scope {:?}: {error}",
                    scope_id
                )
            })?;
    }

    Ok(())
}

pub(crate) async fn migrate_legacy_download_client_default_category_settings(
    database: Arc<SettingsStore>,
) -> Result<(), String> {
    for scope_id in [None, Some("movie"), Some("series"), Some("anime")] {
        let scope_id_string = scope_id.map(str::to_string);
        let current = database
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
                scope_id_string.clone(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to read {DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY} during bootstrap migration for scope {:?}: {error}",
                    scope_id
                )
            })?;

        if current
            .as_ref()
            .and_then(|record| record.value_json.as_ref())
            .is_some()
        {
            continue;
        }

        let legacy = database
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                LEGACY_NZBGET_CATEGORY_SETTING_KEY,
                scope_id_string.clone(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to read {LEGACY_NZBGET_CATEGORY_SETTING_KEY} during bootstrap migration for scope {:?}: {error}",
                    scope_id
                )
            })?;

        let Some(legacy_value_json) = legacy.and_then(|record| record.value_json) else {
            continue;
        };

        database
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
                scope_id_string,
                legacy_value_json,
                "legacy-migration",
                None,
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to persist migrated {DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY} for scope {:?}: {error}",
                    scope_id
                )
            })?;
    }

    Ok(())
}

pub(crate) async fn normalize_media_path_setting(
    database: Arc<SettingsStore>,
    key_name: String,
) -> Result<(), String> {
    let media_key = key_name.clone();
    let media_path = database
        .get_setting_with_defaults(SETTINGS_SCOPE_MEDIA, media_key, None)
        .await
        .map_err(|error| {
            format!("failed to read media {key_name} setting during bootstrap: {error}")
        })?;

    if media_path
        .as_ref()
        .is_none_or(|record| record.value_json.is_none())
    {
        let system_key = key_name.clone();
        let system_record = database
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, system_key, None)
            .await
            .map_err(|error| {
                format!("failed to read legacy system {key_name} setting during bootstrap: {error}")
            })?;

        if let Some(system_record) = system_record
            && let Some(value_json) = system_record.value_json
        {
            let upsert_key = key_name.clone();
            database
                .upsert_setting_value(
                    SETTINGS_SCOPE_MEDIA,
                    upsert_key,
                    None,
                    value_json,
                    "legacy-migration",
                    None,
                )
                .await
                .map_err(|error| {
                    format!("failed to persist migrated media {key_name} setting: {error}")
                })?;
        }
    }

    Ok(())
}

pub(crate) async fn normalize_quality_profile_settings(
    settings: Arc<SettingsStore>,
    quality_profiles: Arc<QualityProfileStore>,
    scope_ids: Vec<String>,
) -> Result<(), String> {
    let mut profiles = quality_profiles
        .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
        .await
        .map_err(|error| format!("failed to list system quality profiles: {error}"))?;

    // The built-in default leads so first-profile ordering agrees with the
    // canonical default on fresh installs.
    let default_profiles = vec![
        builtin_default_quality_profile(),
        builtin_4k_profile(),
        builtin_anime_profile(),
    ];

    let (final_profiles, changed) =
        merge_default_quality_profiles(std::mem::take(&mut profiles), default_profiles);
    if changed {
        quality_profiles
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, final_profiles.clone())
            .await
            .map_err(|error| {
                format!("failed to persist default system quality profiles: {error}")
            })?;
    }

    let profile_ids = collect_profile_ids(&final_profiles);
    normalize_quality_profile_id_setting(settings.as_ref(), None, &profile_ids).await?;

    for scope_id in &scope_ids {
        normalize_quality_profile_id_setting(
            settings.as_ref(),
            Some(scope_id.as_str()),
            &profile_ids,
        )
        .await?;
    }

    if profile_ids.iter().any(|id| id == "anime") {
        seed_scope_default_if_unset(settings.as_ref(), "anime", "anime", Some("1080p")).await?;
    }

    sync_quality_profile_catalog_setting(settings.as_ref(), &final_profiles).await?;

    Ok(())
}

pub(crate) async fn sync_quality_profile_catalog_setting(
    database: &SettingsStore,
    profiles: &[QualityProfile],
) -> Result<(), String> {
    let catalog: Vec<serde_json::Value> = profiles
        .iter()
        .map(|profile| {
            let criteria = &profile.criteria;
            json!({
                "id": profile.id,
                "name": profile.name,
                "criteria": {
                    "quality_tiers": criteria.quality_tiers,
                    "archival_quality": criteria.archival_quality.clone(),
                    "allow_unknown_quality": criteria.allow_unknown_quality,
                    "source_allowlist": criteria.source_allowlist,
                    "source_blocklist": criteria.source_blocklist,
                    "video_codec_allowlist": criteria.video_codec_allowlist,
                    "video_codec_blocklist": criteria.video_codec_blocklist,
                    "audio_codec_allowlist": criteria.audio_codec_allowlist,
                    "audio_codec_blocklist": criteria.audio_codec_blocklist,
                    "atmos_preferred": criteria.atmos_preferred,
                    "dolby_vision_allowed": criteria.dolby_vision_allowed,
                    "detected_hdr_allowed": criteria.detected_hdr_allowed,
                    "prefer_remux": criteria.prefer_remux,
                    "allow_bd_disk": criteria.allow_bd_disk,
                    "allow_upgrades": criteria.allow_upgrades,
                    "prefer_dual_audio": criteria.prefer_dual_audio,
                    "required_audio_languages": criteria.required_audio_languages,
                }
            })
        })
        .collect();

    let catalog_json = serde_json::to_string(&catalog).map_err(|error| {
        format!("failed to serialize quality profile catalog for settings: {error}")
    })?;

    database
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_CATALOG_KEY,
            None,
            catalog_json,
            "bootstrap-normalization",
            None,
        )
        .await
        .map_err(|error| {
            format!(
                "failed to persist quality profile catalog setting {}: {error}",
                QUALITY_PROFILE_CATALOG_KEY
            )
        })?;

    Ok(())
}

pub(crate) fn merge_default_quality_profiles(
    mut profiles: Vec<QualityProfile>,
    default_profiles: Vec<QualityProfile>,
) -> (Vec<QualityProfile>, bool) {
    let mut changed =
        normalize_legacy_seeded_default_quality_profiles(&mut profiles, &default_profiles);

    // Add newly introduced defaults only when every existing profile still
    // exactly matches a system default. Any customized or wizard-created
    // profile makes the catalog user-owned and leaves it untouched.
    if !profiles.is_empty() {
        let system_owned = profiles
            .iter()
            .all(|profile| default_profiles.iter().any(|default| default == profile));
        if system_owned {
            for default in &default_profiles {
                if !profiles.iter().any(|profile| profile.id == default.id) {
                    profiles.push(default.clone());
                    changed = true;
                }
            }
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        return (profiles, changed);
    }

    for profile in default_profiles {
        profiles.push(profile);
    }
    changed = true;

    profiles.sort_by(|a, b| a.id.cmp(&b.id));

    if profiles.is_empty() {
        profiles.push(builtin_default_quality_profile());
        changed = true;
    }

    (profiles, changed)
}

fn normalize_legacy_seeded_default_quality_profiles(
    profiles: &mut [QualityProfile],
    default_profiles: &[QualityProfile],
) -> bool {
    let mut changed = false;

    for profile in profiles.iter_mut() {
        let Some(default_profile) = default_profiles
            .iter()
            .find(|candidate| candidate.id == profile.id)
        else {
            continue;
        };

        let legacy_profile = match profile.id.as_str() {
            "4k" => legacy_seeded_builtin_4k_profile(),
            "1080p" => legacy_seeded_builtin_1080p_profile(),
            _ => continue,
        };

        if *profile != legacy_profile {
            continue;
        }

        *profile = default_profile.clone();
        changed = true;
    }

    changed
}

fn legacy_seeded_builtin_4k_profile() -> QualityProfile {
    let mut profile = builtin_4k_profile();
    profile.criteria.atmos_preferred = true;
    profile.criteria.prefer_remux = true;
    profile
}

fn legacy_seeded_builtin_1080p_profile() -> QualityProfile {
    let mut profile = builtin_1080p_profile();
    profile.criteria.atmos_preferred = true;
    profile.criteria.prefer_remux = true;
    profile
}

pub(crate) async fn normalize_quality_profile_id_setting(
    database: &SettingsStore,
    scope_id: Option<&str>,
    valid_profile_ids: &[String],
) -> Result<(), String> {
    let scope_id_owned = scope_id.map(str::to_string);
    let scope_label = scope_id.unwrap_or("system");
    let record = database
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            scope_id_owned,
        )
        .await
        .map_err(|error| {
            format!("failed to read {QUALITY_PROFILE_ID_KEY} for scope {scope_label}: {error}")
        })?;

    let record = match record {
        Some(record) => record,
        None => return Ok(()),
    };

    if scope_id.is_some() && record.value_json.is_none() {
        return Ok(());
    }

    let current_profile = parse_quality_profile_id(
        record
            .value_json
            .as_deref()
            .unwrap_or(record.effective_value_json.as_str()),
    );

    if scope_id.is_none() {
        // Global scope. A valid effective choice (explicit or the definition
        // default) is left alone. An unresolvable one is repaired: when the
        // canonical built-in default exists in the catalog, deleting any
        // explicit row lets reads fall back to it; when the catalog replaced
        // the built-ins (the setup wizard does), an explicit valid profile is
        // materialized instead so the effective global always names a real
        // profile.
        let resolves = current_profile
            .as_deref()
            .is_some_and(|value| valid_profile_ids.iter().any(|id| id == value));
        if resolves {
            return Ok(());
        }
        let builtin_default_available = valid_profile_ids
            .iter()
            .any(|id| id == scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID);
        if record.value_json.is_some() && builtin_default_available {
            return database
                .delete_setting_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
                .await
                .map_err(|error| {
                    format!("failed to clear invalid global {QUALITY_PROFILE_ID_KEY}: {error}")
                });
        }
        let repaired = if builtin_default_available {
            scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string()
        } else {
            valid_profile_ids.first().cloned().unwrap_or_else(|| {
                scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string()
            })
        };
        return upsert_quality_profile_setting(database, None, &repaired).await;
    }

    let next_profile = if matches!(current_profile.as_deref(), Some(value) if value == QUALITY_PROFILE_INHERIT_VALUE)
    {
        QUALITY_PROFILE_INHERIT_VALUE.to_string()
    } else if current_profile
        .as_ref()
        .is_some_and(|value| valid_profile_ids.contains(value))
    {
        current_profile.clone().unwrap()
    } else {
        QUALITY_PROFILE_INHERIT_VALUE.to_string()
    };

    let current_for_compare =
        current_profile.unwrap_or_else(|| QUALITY_PROFILE_INHERIT_VALUE.to_string());

    if current_for_compare == next_profile {
        return Ok(());
    }

    upsert_quality_profile_setting(database, scope_id.map(str::to_string), &next_profile).await
}

async fn seed_scope_default_if_unset(
    database: &SettingsStore,
    scope_id: &str,
    default_profile_id: &str,
    previous_system_default_id: Option<&str>,
) -> Result<(), String> {
    let record = database
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            Some(scope_id.to_string()),
        )
        .await
        .map_err(|error| {
            format!("failed to read {QUALITY_PROFILE_ID_KEY} for scope {scope_id}: {error}")
        })?;

    let should_migrate_system_default = record.as_ref().is_some_and(|record| {
        record.updated_by_user_id.is_none()
            && record.source.as_deref() == Some("bootstrap-normalization")
            && record
                .value_json
                .as_deref()
                .and_then(parse_quality_profile_id)
                .as_deref()
                .is_some_and(|current| Some(current) == previous_system_default_id)
    });

    if should_migrate_system_default || record.as_ref().is_none_or(|r| r.value_json.is_none()) {
        upsert_quality_profile_setting(database, Some(scope_id.to_string()), default_profile_id)
            .await?;
    }

    Ok(())
}

pub(crate) async fn upsert_quality_profile_setting(
    database: &SettingsStore,
    scope_id: Option<String>,
    value: &str,
) -> Result<(), String> {
    database
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            scope_id,
            value,
            "bootstrap-normalization",
            None,
        )
        .await
        .map_err(|error| {
            format!(
                "failed to persist normalized setting {}:{}: {error}",
                SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY
            )
        })?;

    Ok(())
}

pub(crate) fn collect_profile_ids(profiles: &[QualityProfile]) -> Vec<String> {
    let mut ids = Vec::new();
    for profile in profiles {
        let id = profile.id.trim();
        if id.is_empty() {
            continue;
        }

        if !ids.contains(&id.to_string()) {
            ids.push(id.to_string());
        }
    }

    if ids.is_empty() {
        ids.push(scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string());
    }

    ids
}

pub(crate) fn parse_quality_profile_id(raw_value: impl AsRef<str>) -> Option<String> {
    let trimmed = raw_value.as_ref().trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::String(value)) => {
            let normalized = value.trim();
            if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            }
        }
        Ok(_) => None,
        Err(_) => Some(trimmed.to_string()),
    }
}

pub(crate) fn parse_migration_mode(
    raw: Option<String>,
) -> scryer_infrastructure_datastore::MigrationMode {
    match raw.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("validate") => {
            scryer_infrastructure_datastore::MigrationMode::ValidateOnly
        }
        Some(value) if value.eq_ignore_ascii_case("apply") => {
            scryer_infrastructure_datastore::MigrationMode::Apply
        }
        Some(value) if value.eq_ignore_ascii_case("auto") => {
            scryer_infrastructure_datastore::MigrationMode::Apply
        }
        Some("0") => scryer_infrastructure_datastore::MigrationMode::ValidateOnly,
        Some("1") => scryer_infrastructure_datastore::MigrationMode::Apply,
        Some(value) => {
            tracing::warn!(value = value, "unknown migration mode, defaulting to apply");
            scryer_infrastructure_datastore::MigrationMode::Apply
        }
        None => scryer_infrastructure_datastore::MigrationMode::Apply,
    }
}

pub(crate) fn extract_pending_migration_ids(message: &str) -> Option<Vec<String>> {
    let (_, pending_part) = message.split_once("pending migrations: ")?;
    let pending = pending_part
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    if pending.is_empty() {
        None
    } else {
        Some(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_application::SettingsRepository;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

    async fn bootstrap_settings_store() -> (tempfile::TempDir, Arc<SettingsStore>) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("scryer.db");
        let services = SqliteServices::new_with_mode(
            db_path.to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let store = Arc::new(SettingsStore::new(
            services.datastore(),
            services.encryption_key_state(),
        ));
        seed_service_setting_definitions(store.clone())
            .await
            .expect("seed setting definitions");
        (temp, store)
    }

    #[test]
    fn service_setting_seeds_include_audio_persona_migration_sentinel() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == AUDIO_PERSONA_MIGRATION_SENTINEL_KEY
                && seed.data_type == "bool"
        }));
    }

    #[test]
    fn service_setting_seeds_include_scheduler_instance_id() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.category == SETTINGS_CATEGORY_SERVICE
                && seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == "scheduler.instance_id"
                && seed.data_type == "string"
                && seed.default_value_json == "null"
                && !seed.is_sensitive
        }));
    }

    #[test]
    fn service_setting_seeds_register_acquisition_convergence_keys() {
        let seeds = service_setting_seeds();
        for key in [
            "acquisition.convergence_seeded_at",
            "acquisition.convergence_resume_after",
        ] {
            let seed = seeds
                .iter()
                .find(|seed| seed.scope == SETTINGS_SCOPE_SYSTEM && seed.key_name == key)
                .unwrap_or_else(|| panic!("missing convergence setting seed {key}"));
            assert_eq!(seed.category, SETTINGS_CATEGORY_ACQUISITION);
            assert_eq!(seed.data_type, "json");
            assert_eq!(seed.default_value_json, "null");
            assert!(!seed.is_sensitive);
        }
    }

    #[test]
    fn quality_profile_default_seed_matches_the_builtin_default() {
        let seed = service_setting_seeds()
            .iter()
            .find(|seed| {
                seed.scope == SETTINGS_SCOPE_SYSTEM && seed.key_name == QUALITY_PROFILE_ID_KEY
            })
            .expect("global quality profile definition should exist");
        assert_eq!(
            seed.default_value_json,
            format!(
                "\"{}\"",
                scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID
            ),
            "the seeded default must stay in lockstep with the canonical built-in default"
        );
        assert_eq!(
            builtin_default_quality_profile().id,
            scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID
        );
    }

    #[tokio::test]
    async fn anime_rename_template_default_migrates_without_overwriting_explicit_overrides() {
        let (_temp, store) = bootstrap_settings_store().await;
        let old_default =
            "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}";
        store
            .batch_ensure_setting_definitions(vec![
                scryer_infrastructure_sql::types::SettingDefinitionSeed {
                    category: SETTINGS_CATEGORY_MEDIA.to_string(),
                    scope: SETTINGS_SCOPE_SYSTEM.to_string(),
                    key_name: RENAME_TEMPLATE_ANIME_GLOBAL_KEY.to_string(),
                    data_type: "string".to_string(),
                    default_value_json: serde_json::json!(old_default).to_string(),
                    is_sensitive: false,
                    validation_json: None,
                },
            ])
            .await
            .expect("seed the previous inherited default");

        seed_service_setting_definitions(store.clone())
            .await
            .expect("seed the updated default");
        let inherited = store
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
                None,
            )
            .await
            .expect("read inherited template")
            .expect("inherited template definition");
        assert_eq!(
            inherited.effective_value_json,
            serde_json::json!(scryer_application::DEFAULT_RENAME_TEMPLATE_ANIME).to_string()
        );

        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
                None,
                serde_json::json!(old_default).to_string(),
                "user",
                None,
            )
            .await
            .expect("save an explicit legacy template override");
        seed_service_setting_definitions(store.clone())
            .await
            .expect("reseed definitions without touching overrides");

        let explicit = store
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
                None,
            )
            .await
            .expect("read explicit template")
            .expect("explicit template value");
        assert_eq!(
            explicit.effective_value_json,
            serde_json::json!(old_default).to_string()
        );
        assert_eq!(
            explicit.value_json,
            Some(serde_json::json!(old_default).to_string())
        );

        let scoped_template = "{title} - {episode_title}.{ext}";
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                RENAME_TEMPLATE_KEY,
                Some("anime".to_string()),
                serde_json::json!(scoped_template).to_string(),
                "user",
                None,
            )
            .await
            .expect("save an explicit scoped template override");
        seed_service_setting_definitions(store.clone())
            .await
            .expect("reseed definitions without touching scoped overrides");

        let scoped = store
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                RENAME_TEMPLATE_KEY,
                Some("anime".to_string()),
            )
            .await
            .expect("read scoped template")
            .expect("scoped template value");
        assert_eq!(
            scoped.effective_value_json,
            serde_json::json!(scoped_template).to_string()
        );
        assert_eq!(
            scoped.value_json,
            Some(serde_json::json!(scoped_template).to_string())
        );
    }

    #[tokio::test]
    async fn normalize_deletes_a_dangling_explicit_global_profile() {
        let (_temp, store) = bootstrap_settings_store().await;
        upsert_quality_profile_setting(store.as_ref(), None, "missing-profile")
            .await
            .expect("seed dangling global profile");

        normalize_quality_profile_id_setting(
            store.as_ref(),
            None,
            &["1080p".to_string(), "4k".to_string()],
        )
        .await
        .expect("normalize global profile");

        let record = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
            .await
            .expect("read global profile")
            .expect("definition exists");
        assert!(
            record.value_json.is_none(),
            "a dangling explicit global must be deleted, not rewritten"
        );
        assert_eq!(
            parse_quality_profile_id(&record.effective_value_json).as_deref(),
            Some(scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID),
            "resolution falls back to the built-in default after the repair"
        );
    }

    #[tokio::test]
    async fn normalize_materializes_a_global_when_the_builtin_default_is_absent() {
        let (_temp, store) = bootstrap_settings_store().await;
        // No explicit global; the definition default (1080p) is missing from
        // this wizard-style catalog, so a valid explicit global must be
        // materialized to keep resolution working.
        normalize_quality_profile_id_setting(
            store.as_ref(),
            None,
            &["wizard-ANIME".to_string(), "wizard-MOVIE".to_string()],
        )
        .await
        .expect("normalize global profile");

        let record = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
            .await
            .expect("read global profile")
            .expect("definition exists");
        assert_eq!(
            record
                .value_json
                .as_deref()
                .and_then(parse_quality_profile_id),
            Some("wizard-ANIME".to_string()),
            "an unresolvable definition default is repaired to the first catalog profile"
        );
    }

    #[tokio::test]
    async fn normalize_repairs_a_dangling_global_when_the_builtin_default_is_absent() {
        let (_temp, store) = bootstrap_settings_store().await;
        upsert_quality_profile_setting(store.as_ref(), None, "missing-profile")
            .await
            .expect("seed dangling global profile");

        normalize_quality_profile_id_setting(
            store.as_ref(),
            None,
            &["wizard-ANIME".to_string(), "wizard-MOVIE".to_string()],
        )
        .await
        .expect("normalize global profile");

        let record = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
            .await
            .expect("read global profile")
            .expect("definition exists");
        assert_eq!(
            record
                .value_json
                .as_deref()
                .and_then(parse_quality_profile_id),
            Some("wizard-ANIME".to_string()),
            "deleting the row would leave the dangling definition default in charge"
        );
    }

    #[tokio::test]
    async fn normalize_preserves_a_valid_explicit_global_profile() {
        let (_temp, store) = bootstrap_settings_store().await;
        upsert_quality_profile_setting(store.as_ref(), None, "4k")
            .await
            .expect("seed explicit global profile");

        normalize_quality_profile_id_setting(
            store.as_ref(),
            None,
            &["1080p".to_string(), "4k".to_string()],
        )
        .await
        .expect("normalize global profile");

        let record = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
            .await
            .expect("read global profile")
            .expect("definition exists");
        assert_eq!(
            record
                .value_json
                .as_deref()
                .and_then(parse_quality_profile_id),
            Some("4k".to_string()),
            "a valid explicit global choice is preserved"
        );
    }

    #[tokio::test]
    async fn startup_migration_state_definitions_accept_writes() {
        let keys = [
            crate::startup_migrations::_0007_emby_plugin_compatibility::MIGRATION_STATE_KEY,
            crate::startup_migrations::_0010_download_client_remove_failed_default::DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY,
        ];
        for key in keys {
            let seed = service_setting_seeds()
                .iter()
                .find(|seed| seed.scope == SETTINGS_SCOPE_SYSTEM && seed.key_name == key)
                .unwrap_or_else(|| {
                    panic!("startup migration state definition should exist: {key}")
                });
            assert_eq!(seed.category, SETTINGS_CATEGORY_SERVICE);
            assert_eq!(seed.data_type, "string");
            assert_eq!(seed.default_value_json, "\"none\"");
            assert!(!seed.is_sensitive);
        }

        let (_temp, store) = bootstrap_settings_store().await;
        for key in keys {
            SettingsRepository::upsert_setting_json(
                &*store,
                SETTINGS_SCOPE_SYSTEM,
                key,
                None,
                "\"pending\"".to_string(),
                "system",
                None,
            )
            .await
            .unwrap_or_else(|error| panic!("startup migration state should persist: {error}"));
        }
    }

    #[tokio::test]
    async fn service_setting_definitions_allow_title_metadata_rehydration_state_to_persist() {
        const KEY: &str = "catalog.title_metadata_rehydration_017_state";

        let seed = service_setting_seeds()
            .iter()
            .find(|seed| seed.scope == SETTINGS_SCOPE_SYSTEM && seed.key_name == KEY)
            .expect("title metadata rehydration state definition should exist");
        assert_eq!(seed.category, SETTINGS_CATEGORY_MEDIA);
        assert_eq!(seed.data_type, "string");
        assert_eq!(seed.default_value_json, "\"none\"");
        assert!(!seed.is_sensitive);

        let (_temp, store) = bootstrap_settings_store().await;
        SettingsRepository::upsert_setting_json(
            &*store,
            SETTINGS_SCOPE_SYSTEM,
            KEY,
            None,
            "\"pending\"".to_string(),
            "system",
            None,
        )
        .await
        .expect("title metadata rehydration state should persist");
    }

    #[tokio::test]
    async fn service_setting_definitions_allow_title_credits_rehydration_state_to_persist() {
        const KEY: &str =
            crate::startup_migrations::_0008_title_credits_rehydration_018::TITLE_CREDITS_REHYDRATION_018_STATE_KEY;

        let seed = service_setting_seeds()
            .iter()
            .find(|seed| seed.scope == SETTINGS_SCOPE_SYSTEM && seed.key_name == KEY)
            .expect("title credits rehydration state definition should exist");
        assert_eq!(seed.category, SETTINGS_CATEGORY_MEDIA);
        assert_eq!(seed.data_type, "string");
        assert_eq!(seed.default_value_json, "\"none\"");
        assert!(!seed.is_sensitive);

        let (_temp, store) = bootstrap_settings_store().await;
        SettingsRepository::upsert_setting_json(
            &*store,
            SETTINGS_SCOPE_SYSTEM,
            KEY,
            None,
            "\"pending\"".to_string(),
            "system",
            None,
        )
        .await
        .expect("title credits rehydration state should persist");
    }

    #[tokio::test]
    async fn service_setting_definitions_allow_scheduler_instance_id_to_persist() {
        let (_temp, store) = bootstrap_settings_store().await;

        SettingsRepository::upsert_setting_json(
            &*store,
            SETTINGS_SCOPE_SYSTEM,
            "scheduler.instance_id",
            None,
            "\"scheduler-seed\"".to_string(),
            "system",
            None,
        )
        .await
        .expect("scheduler instance id should persist");

        let stored = SettingsRepository::get_setting_json(
            &*store,
            SETTINGS_SCOPE_SYSTEM,
            "scheduler.instance_id",
            None,
        )
        .await
        .expect("scheduler instance id should load");
        assert_eq!(stored.as_deref(), Some("\"scheduler-seed\""));
    }

    #[test]
    fn service_setting_seeds_include_title_image_artwork_url_refresh_state() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.category == SETTINGS_CATEGORY_MEDIA
                && seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "\"none\""
                && !seed.is_sensitive
        }));
    }

    /// Every maintenance gate ships disarmed. This is the one seed list where a
    /// wrong default is not a cosmetic bug: a `true` here would arm an
    /// instance-wide capability on upgrade without anyone asking for it.
    #[test]
    fn every_maintenance_gate_is_seeded_and_defaults_to_off() {
        for key_name in [
            MAINTENANCE_GATE_EVALUATION_KEY,
            MAINTENANCE_GATE_RESULT_DISPLAY_KEY,
            MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY,
            MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY,
            MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY,
        ] {
            assert!(
                service_setting_seeds().iter().any(|seed| {
                    seed.scope == SETTINGS_SCOPE_SYSTEM
                        && seed.key_name == key_name
                        && seed.data_type == "boolean"
                        && seed.default_value_json == "false"
                        && !seed.is_sensitive
                }),
                "{key_name} must be registered and default to false"
            );
        }
    }

    #[test]
    fn service_setting_seeds_include_metadata_language() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == METADATA_LANGUAGE_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "\"eng\""
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == TITLE_METADATA_LANGUAGE_OVERRIDE_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "null"
        }));
    }

    #[test]
    fn service_setting_seeds_include_history_retention_defaults() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == HISTORY_KEEP_FOREVER_KEY
                && seed.data_type == "boolean"
                && seed.default_value_json == "false"
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == HISTORY_RETENTION_DAYS_KEY
                && seed.data_type == "number"
                && seed.default_value_json == "180"
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == IMAGE_CACHE_MAX_SIZE_MB_KEY
                && seed.data_type == "number"
                && seed.default_value_json == "256"
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == scryer_application::PLUGIN_HTTP_CA_BUNDLE_PEM_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "\"\""
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == scryer_application::PLUGIN_AUTO_UPDATE_ENABLED_KEY
                && seed.data_type == "boolean"
                && seed.default_value_json == "false"
                && !seed.is_sensitive
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == AUTO_BACKUP_ENABLED_KEY
                && seed.data_type == "boolean"
                && seed.default_value_json == "false"
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == AUTO_BACKUP_DAILY_TIME_LOCAL_KEY
                && seed.data_type == "string"
                && seed.default_value_json
                    == format!(
                        "\"{}\"",
                        scryer_application::DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL
                    )
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == AUTO_BACKUP_KEY_KEY
                && seed.data_type == "string"
                && seed.is_sensitive
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == scryer_application::AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY
                && seed.data_type == "boolean"
                && seed.default_value_json == "false"
                && !seed.is_sensitive
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_MEDIA
                && seed.key_name == RECYCLE_BIN_ENABLED_KEY
                && seed.data_type == "boolean"
                && seed.default_value_json == "true"
        }));
    }

    #[test]
    fn service_setting_seeds_include_legacy_download_client_keys() {
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == LEGACY_NZBGET_CATEGORY_SETTING_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "\"\""
        }));
        assert!(service_setting_seeds().iter().any(|seed| {
            seed.scope == SETTINGS_SCOPE_SYSTEM
                && seed.key_name == LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY
                && seed.data_type == "string"
                && seed.default_value_json == "{}"
        }));
    }

    #[test]
    fn merge_default_quality_profiles_normalizes_exact_legacy_seeded_defaults() {
        let existing_profiles = vec![
            legacy_seeded_builtin_4k_profile(),
            legacy_seeded_builtin_1080p_profile(),
        ];
        let mut expected_profiles = vec![builtin_4k_profile(), builtin_1080p_profile()];
        expected_profiles.sort_by(|a, b| a.id.cmp(&b.id));

        let (profiles, changed) = merge_default_quality_profiles(
            existing_profiles,
            vec![builtin_4k_profile(), builtin_1080p_profile()],
        );

        assert!(changed);
        assert_eq!(profiles, expected_profiles);
    }

    #[test]
    fn merge_default_quality_profiles_preserves_nonseeded_existing_profiles() {
        let mut customized_profile = builtin_4k_profile();
        customized_profile.criteria.atmos_preferred = true;
        customized_profile.criteria.prefer_remux = true;
        customized_profile.criteria.allow_unknown_quality = true;

        let (profiles, changed) = merge_default_quality_profiles(
            vec![customized_profile.clone()],
            vec![builtin_4k_profile(), builtin_1080p_profile()],
        );

        assert!(!changed);
        assert_eq!(profiles, vec![customized_profile]);
    }

    #[test]
    fn merge_default_quality_profiles_adds_anime_only_to_untouched_system_catalogs() {
        let existing_profiles = vec![builtin_4k_profile(), builtin_1080p_profile()];
        let defaults = vec![
            builtin_4k_profile(),
            builtin_1080p_profile(),
            builtin_anime_profile(),
        ];

        let (profiles, changed) = merge_default_quality_profiles(existing_profiles, defaults);

        assert!(changed);
        assert!(profiles.iter().any(|profile| profile.id == "anime"));
        assert_eq!(
            profiles
                .iter()
                .find(|profile| profile.id == "anime")
                .map(|profile| profile.criteria.quality_tiers.as_slice()),
            Some(["1080P", "720P", "576P"].map(str::to_string).as_slice())
        );
    }

    #[test]
    fn merge_default_quality_profiles_seeds_standard_defaults_when_empty() {
        let (profiles, changed) = merge_default_quality_profiles(
            Vec::new(),
            vec![builtin_4k_profile(), builtin_1080p_profile()],
        );

        assert!(changed);
        assert!(profiles.iter().any(|profile| profile.id == "4k"));
        assert!(profiles.iter().any(|profile| profile.id == "1080p"));
        assert!(!profiles.iter().any(|profile| profile.id == "8k"));
    }
}
