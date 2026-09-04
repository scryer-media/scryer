//! Shared settings scopes, keys, and defaults used across application,
//! interface, and bootstrap layers.

pub const SETTINGS_SCOPE_SYSTEM: &str = "system";
pub const SETTINGS_SCOPE_MEDIA: &str = "media";
pub const SETTINGS_SOURCE_TYPED_GRAPHQL: &str = "typed_graphql";

pub const SCORING_PERSONA_KEY: &str = "quality.scoring_persona";
pub const REQUIRED_AUDIO_LANGUAGES_KEY: &str = "audio.required_languages";
pub const TITLE_REQUIRED_AUDIO_OVERRIDE_KEY: &str = "audio.required_languages.title_override";
pub const AUDIO_PERSONA_MIGRATION_SENTINEL_KEY: &str = "audio_persona_settings_migrated";

pub const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
pub const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
pub const DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY: &str = "download_client.default_category";
pub const DEFAULT_SEEDING_PROFILE_SETTING_KEY: &str = "download_client.default_seeding_profile";
/// Fewest seeders a torrent candidate may report when no seeding profile
/// resolves. Sonarr defaults every torrent indexer to 1; this is the same
/// guarantee expressed once instead of per indexer.
pub const MINIMUM_SEEDERS_FLOOR_SETTING_KEY: &str = "download_client.minimum_seeders_floor";
/// Value [`MINIMUM_SEEDERS_FLOOR_SETTING_KEY`] falls back to. Bootstrap seeds
/// the row with it and the resolver reads it when the row is missing or
/// unparseable, so losing the setting cannot silently turn the protection off.
pub const MINIMUM_SEEDERS_FLOOR_DEFAULT: i32 = 1;
/// The same number in the JSON encoding a settings row stores. Rust cannot
/// stringify a constant at compile time without a dependency, so the pair lives
/// here — one place, side by side — and `the_floor_default_json_is_the_floor_default`
/// keeps the two encodings from drifting.
pub const MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON: &str = "1";
pub const LEGACY_NZBGET_CATEGORY_SETTING_KEY: &str = "nzbget.category";
pub const NZBGET_RECENT_PRIORITY_SETTING_KEY: &str = "nzbget.recent_priority";
pub const NZBGET_OLDER_PRIORITY_SETTING_KEY: &str = "nzbget.older_priority";
pub const INDEXER_ROUTING_SETTINGS_KEY: &str = "indexer.routing";
pub(crate) const INDEXER_ROUTING_MOVIE_DEFAULT_CATEGORIES: &[&str] = &["2000"];
pub(crate) const INDEXER_ROUTING_SERIES_DEFAULT_CATEGORIES: &[&str] = &["5000"];
pub(crate) const INDEXER_ROUTING_ANIME_DEFAULT_CATEGORIES: &[&str] = &["5070"];
pub const METADATA_LANGUAGE_KEY: &str = "metadata_language";
pub const TITLE_METADATA_LANGUAGE_OVERRIDE_KEY: &str = "metadata_language.title_override";
pub const USE_SEASON_FOLDERS_KEY: &str = "rename.use_season_folders";
// Discovery region seam. Read like metadata_language; a future
// preferences UI only has to write this key (defaults to "US" -> unchanged).
pub const DISCOVERY_REGION_KEY: &str = "discovery.region";
// Instance-wide opt in for surfaces that are still being finished. Absent row
// means disabled, so existing installs keep the unfinished surfaces hidden.
pub const EXPERIMENTAL_FEATURES_ENABLED_KEY: &str = "ui.experimental_features_enabled";
// Instance-wide opt out for sending the library context to the metadata
// gateway. Stored in the positive sense; absent row means enabled, so existing
// installs keep personalized discovery working with no data change.
pub const DISCOVERY_PERSONALIZED_ENABLED_KEY: &str = "discovery.personalized_enabled";
pub const HISTORY_KEEP_FOREVER_KEY: &str = "history.keep_forever";
pub const HISTORY_RETENTION_DAYS_KEY: &str = "history.retention_days";
pub const IMAGE_CACHE_MAX_SIZE_MB_KEY: &str = "images.cache.max_size_mb";
pub const DEFAULT_IMAGE_CACHE_MAX_SIZE_MB: i32 = 256;
pub const IMAGE_CACHE_MAX_BYTES_ENV: &str = "SCRYER_IMAGE_CACHE_MAX_BYTES";
pub const PLUGIN_HTTP_CA_BUNDLE_PEM_KEY: &str = "plugins.http.ca_bundle_pem";
pub const PLUGIN_AUTO_UPDATE_ENABLED_KEY: &str = "plugins.auto_update.enabled";
pub const AUTO_BACKUP_ENABLED_KEY: &str = "backup.auto.enabled";
pub const AUTO_BACKUP_DAILY_TIME_LOCAL_KEY: &str = "backup.auto.daily_time_local";
pub const AUTO_BACKUP_KEY_KEY: &str = "backup.auto.key";
pub const AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY: &str =
    "backup.auto.disabled_missing_key_notice";
pub const BACKUP_PATH_KEY: &str = "backup.path";
pub const AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY: &str =
    "backup.auto.post_upgrade_pending_version";
pub const FORM_LOGIN_ENABLED_KEY: &str = "auth.form_login_enabled";
pub const PASSWORD_MIN_LENGTH_KEY: &str = "auth.password_min_length";
pub const PASSWORD_MIN_LENGTH_MIN: i64 = 8;
pub const SKIP_LOGIN_FOR_LOCAL_IPS_KEY: &str = "auth.skip_login_for_local_ips";
pub const MFA_REQUIRE_CONFIG_STEP_UP_KEY: &str = "auth.mfa.require_config_step_up";
pub const API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY: &str =
    "auth.api_keys.restrict_to_system_settings_users";
pub const MFA_REQUIRE_PASSWORD_LOGIN_KEY: &str = "auth.mfa.require_password_login";
pub const LEGACY_TOTP_REQUIRE_CONFIG_STEP_UP_KEY: &str = "auth.totp.require_config_step_up";
pub const LEGACY_TOTP_REQUIRE_PASSWORD_LOGIN_KEY: &str = "auth.totp.require_local_login";
pub const TOTP_REQUIRE_JELLYFIN_LOGIN_KEY: &str = "auth.totp.require_jellyfin_login";
pub const TOTP_REQUIRE_EMBY_LOGIN_KEY: &str = "auth.totp.require_emby_login";
// ── Maintenance instance gates (RFC 137 section 10) ─────────────────────────
// Five independent instance-wide switches, every one of them off unless a row
// says otherwise. They are deliberately not one JSON blob: each gate authorizes
// a different blast radius, and a partially-written blob must never be able to
// turn a stronger one on. A missing row reads as off, so losing the settings
// table disarms maintenance rather than arming it.
/// Lets the scheduled evaluator run at all. Off means no rule is evaluated and
/// nothing is recorded.
pub const MAINTENANCE_GATE_EVALUATION_KEY: &str = "maintenance.gate.evaluation";
/// Lets candidate results reach the API for rules that are not in shadow mode.
pub const MAINTENANCE_GATE_RESULT_DISPLAY_KEY: &str = "maintenance.gate.result_display";
/// Reserved for the executor wave: provider collection projection and lifecycle
/// notifications. Stored now, consumed by nothing in this build.
pub const MAINTENANCE_GATE_PRESENTATION_EFFECTS_KEY: &str = "maintenance.gate.presentation_effects";
/// Reserved for the executor wave: low and medium risk actions.
pub const MAINTENANCE_GATE_REVERSIBLE_EFFECTS_KEY: &str = "maintenance.gate.reversible_effects";
/// Reserved for the executor wave: high risk actions.
pub const MAINTENANCE_GATE_DESTRUCTIVE_EFFECTS_KEY: &str = "maintenance.gate.destructive_effects";

pub const RECYCLE_BIN_ENABLED_KEY: &str = "recycle_bin.enabled";
pub const RECYCLE_BIN_PATH_KEY: &str = "recycle_bin.path";
pub const RECYCLE_BIN_RETENTION_DAYS_KEY: &str = "recycle_bin.retention_days";
pub const VERIFICATION_DEPTH_KEY: &str = "verification.depth";
/// Where the full-hash backfill job resumes (FR-047). Internal bookkeeping, not
/// an operator-facing preference: it holds the media-file id the last run
/// stopped after, and is cleared when a sweep reaches the end of the queue.
pub const FULL_HASH_BACKFILL_CURSOR_KEY: &str = "verification.full_hash_backfill.cursor";

pub(crate) fn default_indexer_routing_categories_for_scope(scope_id: &str) -> Vec<String> {
    match scope_id {
        "movie" => INDEXER_ROUTING_MOVIE_DEFAULT_CATEGORIES,
        "series" => INDEXER_ROUTING_SERIES_DEFAULT_CATEGORIES,
        "anime" => INDEXER_ROUTING_ANIME_DEFAULT_CATEGORIES,
        _ => &[],
    }
    .iter()
    .map(|value| (*value).to_string())
    .collect()
}

pub const MOVIES_PATH_KEY: &str = "movies.path";
pub const SERIES_PATH_KEY: &str = "series.path";
pub const ANIME_PATH_KEY: &str = "anime.path";
pub const MOVIES_ROOT_FOLDERS_KEY: &str = "movies.root_folders";
pub const SERIES_ROOT_FOLDERS_KEY: &str = "series.root_folders";
pub const ANIME_ROOT_FOLDERS_KEY: &str = "anime.root_folders";

pub const TLS_CERT_PATH_KEY: &str = "tls.cert_path";
pub const TLS_KEY_PATH_KEY: &str = "tls.key_path";

pub const RENAME_ENABLED_KEY: &str = "rename.enabled";
pub const RENAME_TEMPLATE_KEY: &str = "rename.template";
pub const RENAME_TEMPLATE_MOVIE_GLOBAL_KEY: &str = "rename.template.movie.global";
pub const RENAME_TEMPLATE_SERIES_GLOBAL_KEY: &str = "rename.template.series.global";
pub const RENAME_TEMPLATE_ANIME_GLOBAL_KEY: &str = "rename.template.anime.global";
pub const FOLDER_TEMPLATE_KEY: &str = "folder.template";
pub const SEASON_FOLDER_TEMPLATE_KEY: &str = "folder.season_template";
pub const SPECIALS_FOLDER_TEMPLATE_KEY: &str = "folder.specials_template";

pub const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
pub const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
pub const RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY: &str = "rename.collision_policy.movie.global";
pub const RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY: &str = "rename.collision_policy.series.global";
pub const RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY: &str = "rename.collision_policy.anime.global";

pub const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
pub const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
pub const RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.movie.global";
pub const RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.series.global";
pub const RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.anime.global";

pub const ANIME_FILLER_POLICY_KEY: &str = "anime.filler_policy";
pub const ANIME_RECAP_POLICY_KEY: &str = "anime.recap_policy";
pub const ANIME_MONITOR_SPECIALS_KEY: &str = "anime.monitor_specials";
pub const ANIME_INTER_SEASON_MOVIES_KEY: &str = "anime.inter_season_movies";
pub const ANIME_MONITOR_FILLER_MOVIES_KEY: &str = "anime.monitor_filler_movies";

pub const NFO_WRITE_ON_IMPORT_MOVIE_KEY: &str = "nfo.write_on_import.movie";
pub const NFO_WRITE_ON_IMPORT_SERIES_KEY: &str = "nfo.write_on_import.series";
pub const NFO_WRITE_ON_IMPORT_ANIME_KEY: &str = "nfo.write_on_import.anime";
pub const PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY: &str = "plexmatch.write_on_import.series";
pub const PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY: &str = "plexmatch.write_on_import.anime";
pub const IMPORT_MODE_KEY: &str = "import.mode";
pub const SET_PERMISSIONS_LINUX_KEY: &str = "permissions.set_linux";
pub const FILE_CHMOD_KEY: &str = "permissions.file_chmod";
pub const FOLDER_CHMOD_KEY: &str = "permissions.folder_chmod";
pub const CHOWN_GROUP_KEY: &str = "permissions.chown_group";

pub const POST_PROCESSING_SCRIPT_MOVIE_KEY: &str = "post_processing.script.movie";
pub const POST_PROCESSING_SCRIPT_SERIES_KEY: &str = "post_processing.script.series";
pub const POST_PROCESSING_SCRIPT_ANIME_KEY: &str = "post_processing.script.anime";
pub const POST_PROCESSING_TIMEOUT_KEY: &str = "post_processing.timeout_secs";
pub const SETUP_COMPLETE_KEY: &str = "setup.complete";

pub const DEFAULT_MOVIE_LIBRARY_PATH: &str = "/data/movies";
pub const DEFAULT_SERIES_LIBRARY_PATH: &str = "/data/series";
pub const DEFAULT_ANIME_LIBRARY_PATH: &str = "/data/anime";
pub const DEFAULT_RENAME_TEMPLATE_MOVIE: &str = "{title} ({year}) - {quality}.{ext}";
pub const DEFAULT_RENAME_TEMPLATE_SERIES: &str =
    "{title} - S{season:2}E{episode:2} - {quality}.{ext}";
pub const DEFAULT_RENAME_TEMPLATE_ANIME: &str = "{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}";
pub const DEFAULT_FOLDER_TEMPLATE_MOVIE: &str = "{title} ({year})";
pub const DEFAULT_FOLDER_TEMPLATE_SERIES: &str = "{title} ({year})";
pub const DEFAULT_FOLDER_TEMPLATE_ANIME: &str = "{title} ({year})";
pub const DEFAULT_SEASON_FOLDER_TEMPLATE: &str = "Season {season}";
pub const DEFAULT_SPECIALS_FOLDER_TEMPLATE: &str = "Specials";
pub const DEFAULT_RENAME_COLLISION_POLICY: &str = "skip";
pub const DEFAULT_RENAME_MISSING_METADATA_POLICY: &str = "fallback_title";
pub const DEFAULT_FILLER_POLICY: &str = "download_all";
pub const DEFAULT_RECAP_POLICY: &str = "download_all";
pub const DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL: &str = "03:00";

#[cfg(test)]
mod tests {
    use super::{MINIMUM_SEEDERS_FLOOR_DEFAULT, MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON};

    #[test]
    fn the_floor_default_json_is_the_floor_default() {
        // The bootstrap seed stores the JSON form and the resolver falls back to
        // the number; if these ever disagree, a fresh install and a lost
        // settings row would enforce different floors.
        assert_eq!(
            serde_json::from_str::<i32>(MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON).unwrap(),
            MINIMUM_SEEDERS_FLOOR_DEFAULT
        );
    }
}
