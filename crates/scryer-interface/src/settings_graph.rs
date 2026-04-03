use std::collections::HashMap;

use async_graphql::{Error, Result as GqlResult};
use scryer_application::{
    AppUseCase, QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY, QUALITY_PROFILE_INHERIT_VALUE,
    QualityProfile, QualityProfileCriteria,
};
use scryer_domain::RootFolderEntry;
use scryer_infrastructure::{SettingsValueRecord, SqliteServices};

use crate::mappers::from_quality_profile;
use crate::types::*;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const SETTINGS_SCOPE_MEDIA: &str = "media";
const SETTINGS_SOURCE_TYPED_GRAPHQL: &str = "typed_graphql";
const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const INDEXER_ROUTING_SETTINGS_KEY: &str = "indexer.routing";
const MOVIES_PATH_KEY: &str = "movies.path";
const SERIES_PATH_KEY: &str = "series.path";
const ANIME_PATH_KEY: &str = "anime.path";
const MOVIES_ROOT_FOLDERS_KEY: &str = "movies.root_folders";
const SERIES_ROOT_FOLDERS_KEY: &str = "series.root_folders";
const ANIME_ROOT_FOLDERS_KEY: &str = "anime.root_folders";
const TLS_CERT_PATH_KEY: &str = "tls.cert_path";
const TLS_KEY_PATH_KEY: &str = "tls.key_path";
const RENAME_TEMPLATE_KEY: &str = "rename.template";
const RENAME_TEMPLATE_MOVIE_GLOBAL_KEY: &str = "rename.template.movie.global";
const RENAME_TEMPLATE_SERIES_GLOBAL_KEY: &str = "rename.template.series.global";
const RENAME_TEMPLATE_ANIME_GLOBAL_KEY: &str = "rename.template.anime.global";
const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY: &str = "rename.collision_policy.movie.global";
const RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY: &str = "rename.collision_policy.series.global";
const RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY: &str = "rename.collision_policy.anime.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.movie.global";
const RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.series.global";
const RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY: &str =
    "rename.missing_metadata_policy.anime.global";
const ANIME_FILLER_POLICY_KEY: &str = "anime.filler_policy";
const ANIME_RECAP_POLICY_KEY: &str = "anime.recap_policy";
const ANIME_MONITOR_SPECIALS_KEY: &str = "anime.monitor_specials";
const ANIME_INTER_SEASON_MOVIES_KEY: &str = "anime.inter_season_movies";
const ANIME_MONITOR_FILLER_MOVIES_KEY: &str = "anime.monitor_filler_movies";
const NFO_WRITE_ON_IMPORT_MOVIE_KEY: &str = "nfo.write_on_import.movie";
const NFO_WRITE_ON_IMPORT_SERIES_KEY: &str = "nfo.write_on_import.series";
const NFO_WRITE_ON_IMPORT_ANIME_KEY: &str = "nfo.write_on_import.anime";
const PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY: &str = "plexmatch.write_on_import.series";
const PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY: &str = "plexmatch.write_on_import.anime";
const DEFAULT_MOVIE_LIBRARY_PATH: &str = "/data/movies";
const DEFAULT_SERIES_LIBRARY_PATH: &str = "/data/series";
const DEFAULT_ANIME_LIBRARY_PATH: &str = "/data/anime";
const DEFAULT_RENAME_TEMPLATE_MOVIE: &str = "{title} ({year}) - {quality}.{ext}";
const DEFAULT_RENAME_TEMPLATE_SERIES: &str = "{title} - S{season:2}E{episode:2} - {quality}.{ext}";
const DEFAULT_RENAME_TEMPLATE_ANIME: &str =
    "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}";
const DEFAULT_RENAME_COLLISION_POLICY: &str = "skip";
const DEFAULT_RENAME_MISSING_METADATA_POLICY: &str = "fallback_title";
const DEFAULT_FILLER_POLICY: &str = "download_all";
const DEFAULT_RECAP_POLICY: &str = "download_all";

fn parse_string_json(raw_json: &str) -> Option<String> {
    serde_json::from_str::<String>(raw_json)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_effective_setting_string(record: &Option<SettingsValueRecord>) -> Option<String> {
    record
        .as_ref()
        .and_then(|record| parse_string_json(&record.effective_value_json))
}

fn read_effective_setting_json_text(record: &Option<SettingsValueRecord>) -> Option<String> {
    record
        .as_ref()
        .map(|record| record.effective_value_json.trim().to_string())
        .filter(|value| !value.is_empty() && value != "null")
}

fn parse_bool_json(raw_json: &str) -> Option<bool> {
    serde_json::from_str::<bool>(raw_json).ok().or_else(|| {
        serde_json::from_str::<String>(raw_json)
            .ok()
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            })
    })
}

fn read_effective_setting_bool(record: &Option<SettingsValueRecord>) -> Option<bool> {
    record
        .as_ref()
        .and_then(|record| parse_bool_json(&record.effective_value_json))
}

fn read_override_setting_string(record: &Option<SettingsValueRecord>) -> Option<String> {
    record
        .as_ref()
        .and_then(|record| record.value_json.as_deref())
        .and_then(parse_string_json)
}

fn normalize_quality_profile(profile: QualityProfile) -> QualityProfile {
    let normalize_list = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.to_ascii_lowercase()))
            .collect::<Vec<_>>()
    };

    let normalize_quality_tiers = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.clone()))
            .collect::<Vec<_>>()
    };

    let criteria = profile.criteria;
    let mut facet_persona_overrides = HashMap::new();
    for (scope, persona) in criteria.facet_persona_overrides {
        if let Some(scope) = ContentScopeValue::parse(&scope) {
            facet_persona_overrides.insert(scope.as_scope_id().to_string(), persona);
        }
    }

    QualityProfile {
        id: profile.id.trim().to_string(),
        name: profile.name.trim().to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: normalize_quality_tiers(criteria.quality_tiers),
            archival_quality: criteria
                .archival_quality
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: normalize_list(criteria.source_allowlist),
            source_blocklist: normalize_list(criteria.source_blocklist),
            video_codec_allowlist: normalize_list(criteria.video_codec_allowlist),
            video_codec_blocklist: normalize_list(criteria.video_codec_blocklist),
            audio_codec_allowlist: normalize_list(criteria.audio_codec_allowlist),
            audio_codec_blocklist: normalize_list(criteria.audio_codec_blocklist),
            atmos_preferred: criteria.atmos_preferred,
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: criteria.prefer_dual_audio,
            required_audio_languages: normalize_list(criteria.required_audio_languages),
            scoring_persona: criteria.scoring_persona,
            scoring_overrides: criteria.scoring_overrides,
            cutoff_tier: criteria
                .cutoff_tier
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides,
        },
    }
}

fn ensure_quality_profiles_exist(mut profiles: Vec<QualityProfile>) -> Vec<QualityProfile> {
    if profiles.is_empty() {
        profiles.push(scryer_application::default_quality_profile_for_search());
        profiles.push(scryer_application::default_quality_profile_1080p_for_search());
    }

    profiles
}

fn resolve_global_profile_id(profiles: &[QualityProfile], candidate: Option<String>) -> String {
    let trimmed = candidate.unwrap_or_default();
    if profiles.iter().any(|profile| profile.id == trimmed) {
        return trimmed;
    }

    profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) async fn load_quality_profile_settings_payload(
    app: &AppUseCase,
    db: &SqliteServices,
) -> GqlResult<QualityProfileSettingsPayload> {
    let profiles = ensure_quality_profiles_exist(
        app.services
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
            .map_err(|error| Error::new(error.to_string()))?,
    );

    let global_setting = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let global_profile_id =
        resolve_global_profile_id(&profiles, read_effective_setting_string(&global_setting));

    let mut category_selections = Vec::with_capacity(3);
    for scope in [
        ContentScopeValue::Movie,
        ContentScopeValue::Series,
        ContentScopeValue::Anime,
    ] {
        let record = db
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                Some(scope.as_scope_id().to_string()),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;

        let override_profile_id = read_override_setting_string(&record)
            .filter(|value| value != QUALITY_PROFILE_INHERIT_VALUE)
            .filter(|value| profiles.iter().any(|profile| profile.id == *value));
        let effective_profile_id = override_profile_id
            .clone()
            .unwrap_or_else(|| global_profile_id.clone());
        category_selections.push(QualityProfileSelectionPayload {
            scope,
            override_profile_id,
            effective_profile_id,
            inherits_global: true, // patched below
        });
    }

    for selection in &mut category_selections {
        selection.inherits_global = selection.override_profile_id.is_none();
    }

    Ok(QualityProfileSettingsPayload {
        profiles: profiles.into_iter().map(from_quality_profile).collect(),
        global_profile_id,
        category_selections,
    })
}

pub(crate) fn quality_profile_from_input(
    input: QualityProfileInput,
    existing: Option<&QualityProfile>,
) -> GqlResult<QualityProfile> {
    let criteria = input.criteria;
    let mut facet_persona_overrides = HashMap::new();
    for override_entry in criteria.facet_persona_overrides {
        facet_persona_overrides.insert(
            override_entry.scope.as_scope_id().to_string(),
            override_entry.persona.into_application(),
        );
    }

    let profile = normalize_quality_profile(QualityProfile {
        id: input.id,
        name: input.name,
        criteria: QualityProfileCriteria {
            quality_tiers: criteria.quality_tiers,
            archival_quality: criteria.archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: criteria.source_allowlist,
            source_blocklist: criteria.source_blocklist,
            video_codec_allowlist: criteria.video_codec_allowlist,
            video_codec_blocklist: criteria.video_codec_blocklist,
            audio_codec_allowlist: criteria.audio_codec_allowlist,
            audio_codec_blocklist: criteria.audio_codec_blocklist,
            atmos_preferred: criteria.atmos_preferred.unwrap_or(
                existing
                    .map(|profile| profile.criteria.atmos_preferred)
                    .unwrap_or(false),
            ),
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: criteria.prefer_dual_audio.unwrap_or(
                existing
                    .map(|profile| profile.criteria.prefer_dual_audio)
                    .unwrap_or(false),
            ),
            required_audio_languages: criteria.required_audio_languages,
            scoring_persona: criteria.scoring_persona.into_application(),
            scoring_overrides: criteria.scoring_overrides.into_application(),
            cutoff_tier: criteria.cutoff_tier,
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides,
        },
    });

    if profile.id.is_empty() {
        return Err(Error::new("quality profile id is required"));
    }
    if profile.name.is_empty() {
        return Err(Error::new("quality profile name is required"));
    }
    if profile.criteria.quality_tiers.is_empty() {
        return Err(Error::new(
            "quality profile must include at least one quality tier",
        ));
    }

    Ok(profile)
}

pub(crate) async fn persist_quality_profile_catalog(
    db: &SqliteServices,
    profiles: &[QualityProfile],
    updated_by_user_id: Option<String>,
    replace_existing: bool,
) -> GqlResult<()> {
    if replace_existing {
        db.replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, profiles.to_vec())
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    } else {
        db.upsert_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, profiles.to_vec())
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    }

    let catalog_json = serde_json::to_string(profiles)
        .map_err(|error| Error::new(format!("failed to encode quality profiles: {error}")))?;
    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        QUALITY_PROFILE_CATALOG_KEY,
        None,
        catalog_json,
        SETTINGS_SOURCE_TYPED_GRAPHQL,
        updated_by_user_id,
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;

    Ok(())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn library_path_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => MOVIES_PATH_KEY,
        ContentScopeValue::Series => SERIES_PATH_KEY,
        ContentScopeValue::Anime => ANIME_PATH_KEY,
    }
}

fn root_folders_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => MOVIES_ROOT_FOLDERS_KEY,
        ContentScopeValue::Series => SERIES_ROOT_FOLDERS_KEY,
        ContentScopeValue::Anime => ANIME_ROOT_FOLDERS_KEY,
    }
}

fn default_library_path(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => DEFAULT_MOVIE_LIBRARY_PATH,
        ContentScopeValue::Series => DEFAULT_SERIES_LIBRARY_PATH,
        ContentScopeValue::Anime => DEFAULT_ANIME_LIBRARY_PATH,
    }
}

fn rename_template_global_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => RENAME_TEMPLATE_MOVIE_GLOBAL_KEY,
        ContentScopeValue::Series => RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
        ContentScopeValue::Anime => RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    }
}

fn default_rename_template(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => DEFAULT_RENAME_TEMPLATE_MOVIE,
        ContentScopeValue::Series => DEFAULT_RENAME_TEMPLATE_SERIES,
        ContentScopeValue::Anime => DEFAULT_RENAME_TEMPLATE_ANIME,
    }
}

fn legacy_collision_policy_global_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY,
        ContentScopeValue::Series => RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY,
        ContentScopeValue::Anime => RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY,
    }
}

fn legacy_missing_metadata_policy_global_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
        ContentScopeValue::Series => RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY,
        ContentScopeValue::Anime => RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY,
    }
}

fn nfo_write_on_import_key(scope: ContentScopeValue) -> &'static str {
    match scope {
        ContentScopeValue::Movie => NFO_WRITE_ON_IMPORT_MOVIE_KEY,
        ContentScopeValue::Series => NFO_WRITE_ON_IMPORT_SERIES_KEY,
        ContentScopeValue::Anime => NFO_WRITE_ON_IMPORT_ANIME_KEY,
    }
}

fn plexmatch_write_on_import_key(scope: ContentScopeValue) -> Option<&'static str> {
    match scope {
        ContentScopeValue::Movie => None,
        ContentScopeValue::Series => Some(PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY),
        ContentScopeValue::Anime => Some(PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY),
    }
}

fn encode_json<T: serde::Serialize>(value: &T) -> GqlResult<String> {
    serde_json::to_string(value)
        .map_err(|error| Error::new(format!("failed to encode setting value: {error}")))
}

fn normalize_root_folders(entries: Vec<RootFolderInput>) -> GqlResult<Vec<RootFolderEntry>> {
    let mut normalized = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut default_index: Option<usize> = None;

    for entry in entries {
        let path = entry.path.trim().to_string();
        if path.is_empty() {
            return Err(Error::new("root folder path is required"));
        }
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        if entry.is_default && default_index.is_none() {
            default_index = Some(normalized.len());
        }
        normalized.push(RootFolderEntry {
            path,
            is_default: false,
        });
    }

    if normalized.is_empty() {
        return Err(Error::new("at least one root folder is required"));
    }

    let default_index = default_index.unwrap_or(0);
    for (index, entry) in normalized.iter_mut().enumerate() {
        entry.is_default = index == default_index;
    }

    Ok(normalized)
}

pub(crate) async fn load_media_settings_payload(
    app: &AppUseCase,
    db: &SqliteServices,
    scope: ContentScopeValue,
) -> GqlResult<MediaSettingsPayload> {
    let library_record = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_MEDIA,
            library_path_key(scope),
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let scoped_rename_template = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_TEMPLATE_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let global_rename_template = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            rename_template_global_key(scope),
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let scoped_collision_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_COLLISION_POLICY_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let global_collision_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_COLLISION_POLICY_GLOBAL_KEY,
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let legacy_collision_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            legacy_collision_policy_global_key(scope),
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let scoped_missing_metadata_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_MISSING_METADATA_POLICY_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let global_missing_metadata_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY,
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let legacy_missing_metadata_policy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            legacy_missing_metadata_policy_global_key(scope),
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let nfo_write_on_import = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            nfo_write_on_import_key(scope),
            None::<String>,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let plexmatch_write_on_import = if let Some(key) = plexmatch_write_on_import_key(scope) {
        Some(
            db.get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None::<String>)
                .await
                .map_err(|error| Error::new(error.to_string()))?,
        )
    } else {
        None
    };

    let (filler_policy, recap_policy, monitor_specials, inter_season_movies, monitor_filler_movies) =
        if scope == ContentScopeValue::Anime {
            let filler_policy = db
                .get_setting_with_defaults(
                    SETTINGS_SCOPE_SYSTEM,
                    ANIME_FILLER_POLICY_KEY,
                    Some(scope.as_scope_id().to_string()),
                )
                .await
                .map_err(|error| Error::new(error.to_string()))?;
            let recap_policy = db
                .get_setting_with_defaults(
                    SETTINGS_SCOPE_SYSTEM,
                    ANIME_RECAP_POLICY_KEY,
                    Some(scope.as_scope_id().to_string()),
                )
                .await
                .map_err(|error| Error::new(error.to_string()))?;
            let monitor_specials = db
                .get_setting_with_defaults(
                    SETTINGS_SCOPE_SYSTEM,
                    ANIME_MONITOR_SPECIALS_KEY,
                    Some(scope.as_scope_id().to_string()),
                )
                .await
                .map_err(|error| Error::new(error.to_string()))?;
            let inter_season_movies = db
                .get_setting_with_defaults(
                    SETTINGS_SCOPE_SYSTEM,
                    ANIME_INTER_SEASON_MOVIES_KEY,
                    Some(scope.as_scope_id().to_string()),
                )
                .await
                .map_err(|error| Error::new(error.to_string()))?;
            let monitor_filler_movies = db
                .get_setting_with_defaults(
                    SETTINGS_SCOPE_SYSTEM,
                    ANIME_MONITOR_FILLER_MOVIES_KEY,
                    None::<String>,
                )
                .await
                .map_err(|error| Error::new(error.to_string()))?;

            (
                Some(
                    read_effective_setting_string(&filler_policy)
                        .unwrap_or_else(|| DEFAULT_FILLER_POLICY.to_string()),
                ),
                Some(
                    read_effective_setting_string(&recap_policy)
                        .unwrap_or_else(|| DEFAULT_RECAP_POLICY.to_string()),
                ),
                Some(read_effective_setting_bool(&monitor_specials).unwrap_or(false)),
                Some(read_effective_setting_bool(&inter_season_movies).unwrap_or(true)),
                Some(read_effective_setting_bool(&monitor_filler_movies).unwrap_or(false)),
            )
        } else {
            (None, None, None, None, None)
        };

    let library_path = read_effective_setting_string(&library_record)
        .unwrap_or_else(|| default_library_path(scope).to_string());
    let root_folders = app
        .root_folders_for_facet(&scope.into_media_facet())
        .await
        .map_err(|error| Error::new(error.to_string()))?
        .into_iter()
        .map(|entry| RootFolderPayload {
            path: entry.path,
            is_default: entry.is_default,
        })
        .collect();

    Ok(MediaSettingsPayload {
        scope,
        library_path,
        root_folders,
        rename_template: read_effective_setting_string(&scoped_rename_template)
            .or_else(|| read_effective_setting_string(&global_rename_template))
            .unwrap_or_else(|| default_rename_template(scope).to_string()),
        rename_collision_policy: read_effective_setting_string(&scoped_collision_policy)
            .or_else(|| read_effective_setting_string(&global_collision_policy))
            .or_else(|| read_effective_setting_string(&legacy_collision_policy))
            .unwrap_or_else(|| DEFAULT_RENAME_COLLISION_POLICY.to_string()),
        rename_missing_metadata_policy: read_effective_setting_string(
            &scoped_missing_metadata_policy,
        )
        .or_else(|| read_effective_setting_string(&global_missing_metadata_policy))
        .or_else(|| read_effective_setting_string(&legacy_missing_metadata_policy))
        .unwrap_or_else(|| DEFAULT_RENAME_MISSING_METADATA_POLICY.to_string()),
        filler_policy,
        recap_policy,
        monitor_specials,
        inter_season_movies,
        monitor_filler_movies,
        nfo_write_on_import: read_effective_setting_bool(&nfo_write_on_import).unwrap_or(false),
        plexmatch_write_on_import: plexmatch_write_on_import
            .as_ref()
            .map(|record| read_effective_setting_bool(record).unwrap_or(false)),
    })
}

pub(crate) async fn load_library_paths_payload(
    db: &SqliteServices,
) -> GqlResult<LibraryPathsPayload> {
    let movie = db
        .get_setting_with_defaults(SETTINGS_SCOPE_MEDIA, MOVIES_PATH_KEY, None::<String>)
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let series = db
        .get_setting_with_defaults(SETTINGS_SCOPE_MEDIA, SERIES_PATH_KEY, None::<String>)
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let anime = db
        .get_setting_with_defaults(SETTINGS_SCOPE_MEDIA, ANIME_PATH_KEY, None::<String>)
        .await
        .map_err(|error| Error::new(error.to_string()))?;

    Ok(LibraryPathsPayload {
        movie_path: read_effective_setting_string(&movie)
            .unwrap_or_else(|| DEFAULT_MOVIE_LIBRARY_PATH.to_string()),
        series_path: read_effective_setting_string(&series)
            .unwrap_or_else(|| DEFAULT_SERIES_LIBRARY_PATH.to_string()),
        anime_path: read_effective_setting_string(&anime)
            .unwrap_or_else(|| DEFAULT_ANIME_LIBRARY_PATH.to_string()),
    })
}

pub(crate) async fn load_service_settings_payload(
    db: &SqliteServices,
) -> GqlResult<ServiceSettingsPayload> {
    let tls_cert_path = db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, TLS_CERT_PATH_KEY, None::<String>)
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let tls_key_path = db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, TLS_KEY_PATH_KEY, None::<String>)
        .await
        .map_err(|error| Error::new(error.to_string()))?;

    Ok(ServiceSettingsPayload {
        tls_cert_path: read_effective_setting_string(&tls_cert_path).unwrap_or_default(),
        tls_key_path: read_effective_setting_string(&tls_key_path).unwrap_or_default(),
    })
}

pub(crate) async fn persist_media_settings(
    db: &SqliteServices,
    scope: ContentScopeValue,
    input: UpdateMediaSettingsInput,
    updated_by_user_id: Option<String>,
) -> GqlResult<Vec<String>> {
    let scope_id = Some(scope.as_scope_id().to_string());
    let mut changed_keys = Vec::new();

    if let Some(root_folders) = input.root_folders {
        let normalized = normalize_root_folders(root_folders)?;
        db.upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            root_folders_key(scope),
            None,
            encode_json(&normalized)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(root_folders_key(scope).to_string());

        let default_path = normalized
            .iter()
            .find(|entry| entry.is_default)
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| normalized[0].path.clone());
        db.upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            library_path_key(scope),
            None,
            encode_json(&default_path)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        if !changed_keys
            .iter()
            .any(|key| key == library_path_key(scope))
        {
            changed_keys.push(library_path_key(scope).to_string());
        }
    } else if let Some(library_path) = normalize_optional_text(input.library_path) {
        db.upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            library_path_key(scope),
            None,
            encode_json(&library_path)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(library_path_key(scope).to_string());
    }

    if let Some(rename_template) = normalize_optional_text(input.rename_template) {
        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_TEMPLATE_KEY,
            scope_id.clone(),
            encode_json(&rename_template)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(RENAME_TEMPLATE_KEY.to_string());
    }

    if let Some(policy) = normalize_optional_text(input.rename_collision_policy) {
        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_COLLISION_POLICY_KEY,
            scope_id.clone(),
            encode_json(&policy)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(RENAME_COLLISION_POLICY_KEY.to_string());
    }

    if let Some(policy) = normalize_optional_text(input.rename_missing_metadata_policy) {
        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            RENAME_MISSING_METADATA_POLICY_KEY,
            scope_id.clone(),
            encode_json(&policy)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(RENAME_MISSING_METADATA_POLICY_KEY.to_string());
    }

    if let Some(value) = input.nfo_write_on_import {
        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            nfo_write_on_import_key(scope),
            None,
            encode_json(&value)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(nfo_write_on_import_key(scope).to_string());
    }

    if let Some(value) = input.plexmatch_write_on_import {
        let Some(key) = plexmatch_write_on_import_key(scope) else {
            return Err(Error::new(
                "plexmatch_write_on_import is only valid for series and anime",
            ));
        };
        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            key,
            None,
            encode_json(&value)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id.clone(),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(key.to_string());
    }

    if scope == ContentScopeValue::Anime {
        if let Some(value) = normalize_optional_text(input.filler_policy) {
            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ANIME_FILLER_POLICY_KEY,
                scope_id.clone(),
                encode_json(&value)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id.clone(),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;
            changed_keys.push(ANIME_FILLER_POLICY_KEY.to_string());
        }
        if let Some(value) = normalize_optional_text(input.recap_policy) {
            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ANIME_RECAP_POLICY_KEY,
                scope_id.clone(),
                encode_json(&value)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id.clone(),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;
            changed_keys.push(ANIME_RECAP_POLICY_KEY.to_string());
        }
        if let Some(value) = input.monitor_specials {
            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ANIME_MONITOR_SPECIALS_KEY,
                scope_id.clone(),
                encode_json(&value)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id.clone(),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;
            changed_keys.push(ANIME_MONITOR_SPECIALS_KEY.to_string());
        }
        if let Some(value) = input.inter_season_movies {
            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ANIME_INTER_SEASON_MOVIES_KEY,
                scope_id.clone(),
                encode_json(&value)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id.clone(),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;
            changed_keys.push(ANIME_INTER_SEASON_MOVIES_KEY.to_string());
        }
        if let Some(value) = input.monitor_filler_movies {
            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ANIME_MONITOR_FILLER_MOVIES_KEY,
                None,
                encode_json(&value)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id.clone(),
            )
            .await
            .map_err(|error| Error::new(error.to_string()))?;
            changed_keys.push(ANIME_MONITOR_FILLER_MOVIES_KEY.to_string());
        }
    } else if input.filler_policy.is_some()
        || input.recap_policy.is_some()
        || input.monitor_specials.is_some()
        || input.inter_season_movies.is_some()
        || input.monitor_filler_movies.is_some()
    {
        return Err(Error::new("anime-specific settings require scope anime"));
    }

    if changed_keys.is_empty() {
        return Err(Error::new("at least one media setting change is required"));
    }

    Ok(changed_keys)
}

pub(crate) async fn persist_library_paths(
    db: &SqliteServices,
    input: &UpdateLibraryPathsInput,
    updated_by_user_id: Option<String>,
) -> GqlResult<Vec<String>> {
    let movie_path = input.movie_path.trim().to_string();
    let series_path = input.series_path.trim().to_string();
    if movie_path.is_empty() || series_path.is_empty() {
        return Err(Error::new("movie_path and series_path are required"));
    }

    let mut changed_keys = Vec::new();
    db.upsert_setting_value(
        SETTINGS_SCOPE_MEDIA,
        MOVIES_PATH_KEY,
        None,
        encode_json(&movie_path)?,
        SETTINGS_SOURCE_TYPED_GRAPHQL,
        updated_by_user_id.clone(),
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;
    changed_keys.push(MOVIES_PATH_KEY.to_string());

    db.upsert_setting_value(
        SETTINGS_SCOPE_MEDIA,
        SERIES_PATH_KEY,
        None,
        encode_json(&series_path)?,
        SETTINGS_SOURCE_TYPED_GRAPHQL,
        updated_by_user_id.clone(),
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;
    changed_keys.push(SERIES_PATH_KEY.to_string());

    if let Some(anime_path) = normalize_optional_text(input.anime_path.clone()) {
        db.upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            ANIME_PATH_KEY,
            None,
            encode_json(&anime_path)?,
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            updated_by_user_id,
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
        changed_keys.push(ANIME_PATH_KEY.to_string());
    }

    Ok(changed_keys)
}

pub(crate) async fn persist_service_settings(
    db: &SqliteServices,
    input: &UpdateServiceSettingsInput,
    updated_by_user_id: Option<String>,
) -> GqlResult<Vec<String>> {
    let tls_cert_path = input.tls_cert_path.trim().to_string();
    let tls_key_path = input.tls_key_path.trim().to_string();

    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        TLS_CERT_PATH_KEY,
        None,
        encode_json(&tls_cert_path)?,
        SETTINGS_SOURCE_TYPED_GRAPHQL,
        updated_by_user_id.clone(),
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;
    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        TLS_KEY_PATH_KEY,
        None,
        encode_json(&tls_key_path)?,
        SETTINGS_SOURCE_TYPED_GRAPHQL,
        updated_by_user_id,
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;

    Ok(vec![
        TLS_CERT_PATH_KEY.to_string(),
        TLS_KEY_PATH_KEY.to_string(),
    ])
}

fn parse_json_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .as_object()
        .cloned()
}

pub(crate) async fn load_download_client_routing(
    db: &SqliteServices,
    scope: ContentScopeValue,
) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
    let primary = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let legacy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let raw_json = read_effective_setting_json_text(&primary)
        .or_else(|| read_effective_setting_json_text(&legacy));
    let Some(raw_json) = raw_json else {
        return Ok(Vec::new());
    };

    let Some(entries) = parse_json_object(&raw_json) else {
        return Ok(Vec::new());
    };

    let mut payloads: Vec<DownloadClientRoutingEntryPayload> = entries
        .into_iter()
        .map(|(client_id, config)| DownloadClientRoutingEntryPayload {
            client_id,
            enabled: config
                .get("enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(true),
            category: normalize_optional_text(
                config
                    .get("category")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
            ),
            recent_queue_priority: normalize_optional_text(
                config
                    .get("recentQueuePriority")
                    .or_else(|| config.get("recentPriority"))
                    .or_else(|| config.get("recent_priority"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
            ),
            older_queue_priority: normalize_optional_text(
                config
                    .get("olderQueuePriority")
                    .or_else(|| config.get("olderPriority"))
                    .or_else(|| config.get("older_priority"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
            ),
            remove_completed: config
                .get("removeCompleted")
                .or_else(|| config.get("remove_completed"))
                .or_else(|| config.get("removeComplete"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            remove_failed: config
                .get("removeFailed")
                .or_else(|| config.get("remove_failed"))
                .or_else(|| config.get("removeFailure"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        })
        .collect();
    payloads.sort_by(|left, right| left.client_id.cmp(&right.client_id));
    Ok(payloads)
}

pub(crate) fn serialize_download_client_routing(
    entries: Vec<DownloadClientRoutingEntryInput>,
) -> GqlResult<String> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let client_id = entry.client_id.trim();
        if client_id.is_empty() {
            return Err(Error::new(
                "download client routing entry requires client_id",
            ));
        }

        payload.insert(
            client_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "category": normalize_optional_text(entry.category),
                "recentQueuePriority": normalize_optional_text(entry.recent_queue_priority),
                "olderQueuePriority": normalize_optional_text(entry.older_queue_priority),
                "removeCompleted": entry.remove_completed,
                "removeFailed": entry.remove_failed,
            }),
        );
    }

    serde_json::to_string(&payload)
        .map_err(|error| Error::new(format!("failed to encode download client routing: {error}")))
}

pub(crate) async fn load_indexer_routing(
    db: &SqliteServices,
    scope: ContentScopeValue,
) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
    let record = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            Some(scope.as_scope_id().to_string()),
        )
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let Some(raw_json) = read_effective_setting_json_text(&record) else {
        return Ok(Vec::new());
    };
    let Some(entries) = parse_json_object(&raw_json) else {
        return Ok(Vec::new());
    };

    let mut payloads: Vec<IndexerRoutingEntryPayload> = entries
        .into_iter()
        .map(|(indexer_id, config)| IndexerRoutingEntryPayload {
            indexer_id,
            enabled: config
                .get("enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(true),
            categories: config
                .get("categories")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
            priority: config
                .get("priority")
                .and_then(|value| value.as_i64())
                .unwrap_or(1) as i32,
        })
        .collect();
    payloads.sort_by_key(|entry| (entry.priority, entry.indexer_id.clone()));
    Ok(payloads)
}

pub(crate) fn serialize_indexer_routing(
    entries: Vec<IndexerRoutingEntryInput>,
) -> GqlResult<String> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let indexer_id = entry.indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(Error::new("indexer routing entry requires indexer_id"));
        }

        let categories: Vec<String> = entry
            .categories
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        payload.insert(
            indexer_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "categories": categories,
                "priority": entry.priority,
            }),
        );
    }

    serde_json::to_string(&payload)
        .map_err(|error| Error::new(format!("failed to encode indexer routing: {error}")))
}
