use scryer_application::{
    default_quality_profile_1080p_for_search, default_quality_profile_for_search,
    QualityProfile,
};
use scryer_infrastructure::{SettingsValueRecord, SqliteServices};
use serde_json::{json, Value};

use crate::{normalize_env_option, normalize_env_option_with_legacy};

pub(crate) const SETTINGS_SCOPE_SYSTEM: &str = "system";
pub(crate) const SETTINGS_SCOPE_MEDIA: &str = "media";
pub(crate) const SETTINGS_CATEGORY_SERVICE: &str = "service";
pub(crate) const SETTINGS_CATEGORY_MEDIA: &str = "media";
pub(crate) const MOVIES_PATH_KEY: &str = "movies.path";
pub(crate) const SERIES_PATH_KEY: &str = "series.path";
pub(crate) const ANIME_PATH_KEY: &str = "anime.path";
pub(crate) const QUALITY_PROFILE_ID_KEY: &str = "quality.profile_id";
pub(crate) const QUALITY_PROFILE_CATALOG_KEY: &str = "quality.profiles";
pub(crate) const NZBGET_CATEGORY_SETTING_KEY: &str = "nzbget.category";
pub(crate) const NZBGET_RECENT_PRIORITY_SETTING_KEY: &str = "nzbget.recent_priority";
pub(crate) const NZBGET_OLDER_PRIORITY_SETTING_KEY: &str = "nzbget.older_priority";
pub(crate) const NZBGET_TAGS_SETTING_KEY: &str = "nzbget.tags";
pub(crate) const NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
pub(crate) const INDEXER_ROUTING_SETTINGS_KEY: &str = "indexer.routing";
pub(crate) const TLS_CERT_KEY: &str = "tls.cert_path";
pub(crate) const TLS_KEY_KEY: &str = "tls.key_path";
pub(crate) const RENAME_TEMPLATE_KEY: &str = "rename.template";
pub(crate) const RENAME_TEMPLATE_MOVIE_GLOBAL_KEY: &str = "rename.template.movie.global";
pub(crate) const RENAME_TEMPLATE_SERIES_GLOBAL_KEY: &str = "rename.template.series.global";
pub(crate) const RENAME_TEMPLATE_ANIME_GLOBAL_KEY: &str = "rename.template.anime.global";
pub(crate) const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
pub(crate) const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
pub(crate) const RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY: &str = "rename.collision_policy.movie.global";
pub(crate) const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
pub(crate) const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
pub(crate) const RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.movie.global";
pub(crate) const QUALITY_PROFILE_INHERIT_VALUE: &str = "__inherit__";
pub(crate) const SETTINGS_CATEGORY_ACQUISITION: &str = "acquisition";
pub(crate) const SETTINGS_CATEGORY_POST_PROCESSING: &str = "post_processing";
pub(crate) const POST_PROCESSING_SCRIPT_MOVIE_KEY:  &str = "post_processing.script.movie";
pub(crate) const POST_PROCESSING_SCRIPT_SERIES_KEY: &str = "post_processing.script.series";
pub(crate) const POST_PROCESSING_SCRIPT_ANIME_KEY:  &str = "post_processing.script.anime";
pub(crate) const POST_PROCESSING_TIMEOUT_KEY:       &str = "post_processing.timeout_secs";
pub(crate) const SETUP_COMPLETE_KEY: &str = "setup.complete";

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
            default_value_json: "\"/media/movies\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: SERIES_PATH_KEY,
            data_type: "string",
            default_value_json: "\"/media/series\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_MEDIA,
            key_name: ANIME_PATH_KEY,
            data_type: "string",
            default_value_json: "\"/media/anime\"",
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
            default_value_json: "\"{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}\"",
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
            default_value_json: "\"4k\"",
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
            key_name: NZBGET_CATEGORY_SETTING_KEY,
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
            key_name: NZBGET_TAGS_SETTING_KEY,
            data_type: "string",
            default_value_json: "\"\"",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
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
            key_name: "jwt.hmac_secret",
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
            default_value_json: "\"download_all\"",
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
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_MEDIA,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "anime.preferred_sub_group",
            data_type: "string",
            default_value_json: "\"\"",
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
            key_name: "acquisition.sync_interval_seconds",
            data_type: "number",
            default_value_json: "3600",
            is_sensitive: false,
        },
        ServiceSettingSeed {
            category: SETTINGS_CATEGORY_ACQUISITION,
            scope: SETTINGS_SCOPE_SYSTEM,
            key_name: "acquisition.batch_size",
            data_type: "number",
            default_value_json: "50",
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
    ]
}

pub(crate) async fn seed_service_setting_definitions(database: &SqliteServices) -> Result<(), String> {
    let definitions: Vec<scryer_infrastructure::SettingDefinitionSeed> = service_setting_seeds()
        .iter()
        .map(|seed| scryer_infrastructure::SettingDefinitionSeed {
            category: seed.category.to_string(),
            scope: seed.scope.to_string(),
            key_name: seed.key_name.to_string(),
            data_type: seed.data_type.to_string(),
            default_value_json: seed.default_value_json.to_string(),
            is_sensitive: seed.is_sensitive,
            validation_json: None,
        })
        .collect();

    database
        .batch_ensure_setting_definitions(definitions)
        .await
        .map_err(|error| format!("failed to batch seed setting definitions: {error}"))
}

pub(crate) async fn seed_service_settings_from_environment(database: &SqliteServices) -> Result<(), String> {
    let env_settings: Vec<(&str, &str, Option<Value>)> = vec![
        (SETTINGS_SCOPE_SYSTEM, "nzbget.url", normalize_env_option("SCRYER_NZBGET_URL").map(Value::String)),
        (SETTINGS_SCOPE_SYSTEM, "nzbget.username", normalize_env_option("SCRYER_NZBGET_USERNAME").map(Value::String)),
        (SETTINGS_SCOPE_SYSTEM, "nzbget.password", normalize_env_option("SCRYER_NZBGET_PASSWORD").map(Value::String)),
        (SETTINGS_SCOPE_SYSTEM, "nzbget.dupe_mode", normalize_env_option_with_legacy(["SCRYER_NZBGET_DUPE_MODE", "SCRYER_NZBGET_DUPEMODE"]).map(|v: String| Value::String(v.to_uppercase()))),
        (SETTINGS_SCOPE_MEDIA, MOVIES_PATH_KEY, normalize_env_option("SCRYER_MOVIES_PATH").map(Value::String)),
        (SETTINGS_SCOPE_MEDIA, SERIES_PATH_KEY, normalize_env_option("SCRYER_SERIES_PATH").map(Value::String)),
        (SETTINGS_SCOPE_MEDIA, ANIME_PATH_KEY, normalize_env_option("SCRYER_ANIME_PATH").map(Value::String)),
        (SETTINGS_SCOPE_SYSTEM, TLS_CERT_KEY, normalize_env_option("SCRYER_TLS_CERT").map(Value::String)),
        (SETTINGS_SCOPE_SYSTEM, TLS_KEY_KEY, normalize_env_option("SCRYER_TLS_KEY").map(Value::String)),
    ];

    let entries: Vec<(String, String, String, String)> = env_settings
        .into_iter()
        .filter_map(|(scope, key, value)| {
            value.map(|v| (scope.to_string(), key.to_string(), v.to_string(), "env".to_string()))
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

pub(crate) async fn normalize_media_path_setting(
    database: &SqliteServices,
    key_name: &str,
) -> Result<(), String> {
    let media_path = database
        .get_setting_with_defaults(SETTINGS_SCOPE_MEDIA, key_name, None)
        .await
        .map_err(|error| {
            format!("failed to read media {key_name} setting during bootstrap: {error}")
        })?;

    if media_path
        .as_ref()
        .is_none_or(|record| record.value_json.is_none())
    {
        let system_record = database
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await
            .map_err(|error| {
                format!("failed to read legacy system {key_name} setting during bootstrap: {error}")
            })?;

        if let Some(system_record) = system_record {
            if let Some(value_json) = system_record.value_json {
                database
                    .upsert_setting_value(
                        SETTINGS_SCOPE_MEDIA,
                        key_name,
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
    }

    Ok(())
}

pub(crate) async fn normalize_quality_profile_settings(database: &SqliteServices, scope_ids: &[&str]) -> Result<(), String> {
    let mut profiles = database
        .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
        .await
        .map_err(|error| format!("failed to list system quality profiles: {error}"))?;

    let default_profiles = vec![
        default_quality_profile_for_search(),
        default_quality_profile_1080p_for_search(),
    ];

    let (final_profiles, changed) =
        merge_default_quality_profiles(std::mem::take(&mut profiles), default_profiles);
    if changed {
        database
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, final_profiles.clone())
            .await
            .map_err(|error| {
                format!("failed to persist default system quality profiles: {error}")
            })?;
    }

    let profile_ids = collect_profile_ids(&final_profiles);
    normalize_quality_profile_id_setting(database, None, &profile_ids).await?;

    for scope_id in scope_ids {
        normalize_quality_profile_id_setting(database, Some(scope_id), &profile_ids).await?;
    }

    // Anime defaults to 1080p (not 4K) when the user hasn't chosen a profile
    if profile_ids.iter().any(|id| id == "1080p") {
        seed_scope_default_if_unset(database, "anime", "1080p").await?;
    }

    sync_quality_profile_catalog_setting(database, &final_profiles).await?;

    Ok(())
}

pub(crate) async fn sync_quality_profile_catalog_setting(
    database: &SqliteServices,
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
    // Only seed defaults into an empty catalog. If profiles already exist
    // (wizard-created, user-created, or previously seeded), leave them alone.
    // This prevents the bootstrap from re-adding the basic 4K/1080P defaults
    // after the setup wizard has replaced them with per-facet profiles.
    if !profiles.is_empty() {
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        return (profiles, false);
    }

    for profile in default_profiles {
        profiles.push(profile);
    }

    profiles.sort_by(|a, b| a.id.cmp(&b.id));

    if profiles.is_empty() {
        profiles.push(default_quality_profile_for_search());
    }

    (profiles, true)
}

pub(crate) async fn normalize_quality_profile_id_setting(
    database: &SqliteServices,
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

    let next_profile = if scope_id.is_none() {
        match current_profile.as_deref() {
            Some(value) if valid_profile_ids.iter().any(|id| id == value) => value.to_string(),
            _ => valid_profile_ids
                .first()
                .cloned()
                .unwrap_or_else(|| "4k".to_string()),
        }
    } else if matches!(current_profile.as_deref(), Some(value) if value == QUALITY_PROFILE_INHERIT_VALUE)
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

    let current_for_compare = current_profile.unwrap_or_else(|| {
        if scope_id.is_none() {
            valid_profile_ids
                .first()
                .cloned()
                .unwrap_or_else(|| QUALITY_PROFILE_INHERIT_VALUE.to_string())
        } else {
            QUALITY_PROFILE_INHERIT_VALUE.to_string()
        }
    });

    if current_for_compare == next_profile {
        return Ok(());
    }

    upsert_quality_profile_setting(database, scope_id.map(str::to_string), &next_profile).await
}

async fn seed_scope_default_if_unset(
    database: &SqliteServices,
    scope_id: &str,
    default_profile_id: &str,
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

    if record.as_ref().is_none_or(|r| r.value_json.is_none()) {
        upsert_quality_profile_setting(
            database,
            Some(scope_id.to_string()),
            default_profile_id,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn upsert_quality_profile_setting(
    database: &SqliteServices,
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
        ids.push("4k".to_string());
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

pub(crate) async fn load_service_runtime_settings(
    database: &SqliteServices,
) -> Result<ServiceRuntimeSettings, String> {
    let keys = vec![
        (SETTINGS_SCOPE_SYSTEM.to_string(), "nzbget.url".to_string(), None),
        (SETTINGS_SCOPE_SYSTEM.to_string(), "nzbget.username".to_string(), None),
        (SETTINGS_SCOPE_SYSTEM.to_string(), "nzbget.password".to_string(), None),
        (SETTINGS_SCOPE_SYSTEM.to_string(), "nzbget.dupe_mode".to_string(), None),
        (SETTINGS_SCOPE_SYSTEM.to_string(), TLS_CERT_KEY.to_string(), None),
        (SETTINGS_SCOPE_SYSTEM.to_string(), TLS_KEY_KEY.to_string(), None),
    ];

    let results = database
        .batch_get_settings_with_defaults(keys)
        .await
        .map_err(|error| format!("failed to batch load runtime settings: {error}"))?;

    let nzbget_url_record = results[0]
        .as_ref()
        .ok_or_else(|| "missing setting: system.nzbget.url".to_string())?;
    let nzbget_username_record = results[1]
        .as_ref()
        .ok_or_else(|| "missing setting: system.nzbget.username".to_string())?;
    let nzbget_password_record = results[2]
        .as_ref()
        .ok_or_else(|| "missing setting: system.nzbget.password".to_string())?;
    let nzbget_dupe_mode_record = results[3]
        .as_ref()
        .ok_or_else(|| "missing setting: system.nzbget.dupe_mode".to_string())?;

    let nzbget_url = setting_record_to_string(nzbget_url_record, "system.nzbget.url", false)?;
    let nzbget_username = setting_record_to_optional_string(nzbget_username_record)?;
    let nzbget_password = setting_record_to_optional_string(nzbget_password_record)?;
    let nzbget_dupe_mode =
        setting_record_to_string(nzbget_dupe_mode_record, "system.nzbget.dupe_mode", false)?;

    let tls_cert_path = results[4]
        .as_ref()
        .and_then(|record| setting_record_to_optional_string(record).ok().flatten());
    let tls_key_path = results[5]
        .as_ref()
        .and_then(|record| setting_record_to_optional_string(record).ok().flatten());

    Ok(ServiceRuntimeSettings {
        nzbget_url,
        nzbget_username,
        nzbget_password,
        nzbget_dupe_mode,
        tls_cert_path,
        tls_key_path,
    })
}

pub(crate) fn parse_json_setting_value(record: &SettingsValueRecord) -> Result<Value, String> {
    serde_json::from_str(&record.effective_value_json).map_err(|error| {
        format!(
            "setting value for {}.{} is invalid JSON: {error}",
            record.scope, record.key_name
        )
    })
}

pub(crate) fn setting_record_to_string(
    record: &SettingsValueRecord,
    path: &str,
    allow_empty: bool,
) -> Result<String, String> {
    let value = parse_json_setting_value(record)?;
    let value = match value {
        Value::String(value) => value,
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => return Err(format!("setting {path} must be set and cannot be null")),
        _ => return Err(format!("setting {path} must be a string")),
    };
    if !allow_empty && value.trim().is_empty() {
        return Err(format!("setting {path} cannot be empty"));
    }
    Ok(value)
}

pub(crate) fn setting_record_to_optional_string(
    record: &SettingsValueRecord,
) -> Result<Option<String>, String> {
    let value = parse_json_setting_value(record)?;
    match value {
        Value::Null => Ok(None),
        Value::String(value) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        Value::Bool(value) => Ok(Some(value.to_string())),
        Value::Number(value) => Ok(Some(value.to_string())),
        other => Ok(Some(other.to_string())),
    }
}

pub(crate) fn parse_migration_mode(raw: Option<String>) -> scryer_infrastructure::MigrationMode {
    match raw.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("validate") => {
            scryer_infrastructure::MigrationMode::ValidateOnly
        }
        Some(value) if value.eq_ignore_ascii_case("apply") => {
            scryer_infrastructure::MigrationMode::Apply
        }
        Some(value) if value.eq_ignore_ascii_case("auto") => {
            scryer_infrastructure::MigrationMode::Apply
        }
        Some("0") => scryer_infrastructure::MigrationMode::ValidateOnly,
        Some("1") => scryer_infrastructure::MigrationMode::Apply,
        Some(value) => {
            tracing::warn!(value = value, "unknown migration mode, defaulting to apply");
            scryer_infrastructure::MigrationMode::Apply
        }
        None => scryer_infrastructure::MigrationMode::Apply,
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

#[derive(Debug)]
pub(crate) struct ServiceRuntimeSettings {
    pub(crate) nzbget_url: String,
    pub(crate) nzbget_username: Option<String>,
    pub(crate) nzbget_password: Option<String>,
    pub(crate) nzbget_dupe_mode: String,
    #[allow(dead_code)]
    pub(crate) tls_cert_path: Option<String>,
    #[allow(dead_code)]
    pub(crate) tls_key_path: Option<String>,
}
