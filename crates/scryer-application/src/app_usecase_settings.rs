use std::collections::HashSet;

use serde::{Serialize, de::DeserializeOwned};
use tracing::warn;

use super::*;
use crate::acquisition_policy::AcquisitionThresholds;
use crate::subtitles::{normalize_subtitle_language_code, wanted::SubtitleLanguagePref};
use crate::scoring_weights::ScoringPersona;

const SETTINGS_SOURCE_TYPED_GRAPHQL: &str = "typed_graphql";

const ACQUISITION_ENABLED_KEY: &str = "acquisition.enabled";
const ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY: &str = "acquisition.upgrade_cooldown_hours";
const ACQUISITION_SAME_TIER_MIN_DELTA_KEY: &str = "acquisition.same_tier_min_delta";
const ACQUISITION_CROSS_TIER_MIN_DELTA_KEY: &str = "acquisition.cross_tier_min_delta";
const ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY: &str =
    "acquisition.forced_upgrade_delta_bypass";
const ACQUISITION_POLL_INTERVAL_SECONDS_KEY: &str = "acquisition.poll_interval_seconds";
const ACQUISITION_SYNC_INTERVAL_SECONDS_KEY: &str = "acquisition.sync_interval_seconds";
const ACQUISITION_BATCH_SIZE_KEY: &str = "acquisition.batch_size";

const SUBTITLES_ENABLED_KEY: &str = "subtitles.enabled";
const SUBTITLES_OPENSUBTITLES_API_KEY: &str = "subtitles.opensubtitles_api_key";
const SUBTITLES_OPENSUBTITLES_USERNAME_KEY: &str = "subtitles.opensubtitles_username";
const SUBTITLES_OPENSUBTITLES_PASSWORD_KEY: &str = "subtitles.opensubtitles_password";
const SUBTITLES_LANGUAGES_KEY: &str = "subtitles.languages";
const SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY: &str = "subtitles.auto_download_on_import";
const SUBTITLES_MINIMUM_SCORE_SERIES_KEY: &str = "subtitles.minimum_score_series";
const SUBTITLES_MINIMUM_SCORE_MOVIE_KEY: &str = "subtitles.minimum_score_movie";
const SUBTITLES_SEARCH_INTERVAL_HOURS_KEY: &str = "subtitles.search_interval_hours";
const SUBTITLES_INCLUDE_AI_TRANSLATED_KEY: &str = "subtitles.include_ai_translated";
const SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY: &str = "subtitles.include_machine_translated";
const SUBTITLES_SYNC_ENABLED_KEY: &str = "subtitles.sync_enabled";
const SUBTITLES_SYNC_THRESHOLD_SERIES_KEY: &str = "subtitles.sync_threshold_series";
const SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY: &str = "subtitles.sync_threshold_movie";
const SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY: &str = "subtitles.sync_max_offset_seconds";

#[derive(Debug, Clone)]
pub struct SubtitleSettings {
    pub enabled: bool,
    pub open_subtitles_api_key: Option<String>,
    pub open_subtitles_username: Option<String>,
    pub open_subtitles_password: Option<String>,
    pub languages: Vec<SubtitleLanguagePref>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct UpdateSubtitleSettings {
    pub enabled: bool,
    pub open_subtitles_api_key: Option<String>,
    pub open_subtitles_username: String,
    pub open_subtitles_password: Option<String>,
    pub languages: Vec<SubtitleLanguagePref>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct AcquisitionSettings {
    pub enabled: bool,
    pub upgrade_cooldown_hours: i32,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
    pub poll_interval_seconds: i32,
    pub sync_interval_seconds: i32,
    pub batch_size: i32,
}

impl AcquisitionSettings {
    pub fn thresholds(&self) -> AcquisitionThresholds {
        AcquisitionThresholds {
            upgrade_cooldown_hours: self.upgrade_cooldown_hours as i64,
            same_tier_min_delta: self.same_tier_min_delta,
            cross_tier_min_delta: self.cross_tier_min_delta,
            forced_upgrade_delta_bypass: self.forced_upgrade_delta_bypass,
        }
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_subtitle_languages(languages: Vec<SubtitleLanguagePref>) -> Vec<SubtitleLanguagePref> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(languages.len());

    for language in languages {
        let Some(code) = normalize_subtitle_language_code(&language.code) else {
            continue;
        };
        let key = format!(
            "{}:{}:{}",
            code, language.hearing_impaired, language.forced
        );
        if seen.insert(key) {
            normalized.push(SubtitleLanguagePref {
                code,
                hearing_impaired: language.hearing_impaired,
                forced: language.forced,
            });
        }
    }

    normalized
}

fn normalize_delay_profile(mut profile: crate::DelayProfile) -> crate::DelayProfile {
    profile.id = profile.id.trim().to_string();
    profile.name = profile.name.trim().to_string();

    let mut seen_facets = HashSet::new();
    profile.applies_to_facets = profile
        .applies_to_facets
        .into_iter()
        .filter_map(|facet| MediaFacet::parse(&facet).map(|parsed| parsed.as_str().to_string()))
        .filter(|facet| seen_facets.insert(facet.clone()))
        .collect();

    let mut seen_tags = HashSet::new();
    profile.tags = profile
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| seen_tags.insert(tag.to_ascii_lowercase()))
        .collect();

    profile
}

impl AppUseCase {
    pub(crate) async fn read_setting_bool_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }))
    }

    pub(crate) async fn read_setting_i64_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<i64>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| value.parse::<i64>().ok()))
    }

    pub(crate) async fn read_setting_json_value<T: DeserializeOwned>(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<T>> {
        let Some(raw_value) = self.read_setting_string_value(key_name, scope_id).await? else {
            return Ok(None);
        };
        serde_json::from_str::<T>(&raw_value)
            .map(Some)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{key_name}' JSON value: {error}"
                ))
            })
    }

    async fn upsert_system_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                key_name,
                None,
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }

    async fn load_subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        Ok(SubtitleSettings {
            enabled: self
                .read_setting_bool_value(SUBTITLES_ENABLED_KEY, None)
                .await?
                .unwrap_or(false),
            open_subtitles_api_key: normalize_optional_string(
                self.read_setting_string_value(SUBTITLES_OPENSUBTITLES_API_KEY, None)
                    .await?,
            ),
            open_subtitles_username: normalize_optional_string(
                self.read_setting_string_value(SUBTITLES_OPENSUBTITLES_USERNAME_KEY, None)
                    .await?,
            ),
            open_subtitles_password: normalize_optional_string(
                self.read_setting_string_value(SUBTITLES_OPENSUBTITLES_PASSWORD_KEY, None)
                    .await?,
            ),
            languages: normalize_subtitle_languages(
                self.read_setting_json_value::<Vec<SubtitleLanguagePref>>(
                    SUBTITLES_LANGUAGES_KEY,
                    None,
                )
                .await?
                .unwrap_or_default(),
            ),
            auto_download_on_import: self
                .read_setting_bool_value(SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY, None)
                .await?
                .unwrap_or(false),
            minimum_score_series: self
                .read_setting_i64_value(SUBTITLES_MINIMUM_SCORE_SERIES_KEY, None)
                .await?
                .unwrap_or(240) as i32,
            minimum_score_movie: self
                .read_setting_i64_value(SUBTITLES_MINIMUM_SCORE_MOVIE_KEY, None)
                .await?
                .unwrap_or(70) as i32,
            search_interval_hours: self
                .read_setting_i64_value(SUBTITLES_SEARCH_INTERVAL_HOURS_KEY, None)
                .await?
                .unwrap_or(6) as i32,
            include_ai_translated: self
                .read_setting_bool_value(SUBTITLES_INCLUDE_AI_TRANSLATED_KEY, None)
                .await?
                .unwrap_or(false),
            include_machine_translated: self
                .read_setting_bool_value(SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY, None)
                .await?
                .unwrap_or(false),
            sync_enabled: self
                .read_setting_bool_value(SUBTITLES_SYNC_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
            sync_threshold_series: self
                .read_setting_i64_value(SUBTITLES_SYNC_THRESHOLD_SERIES_KEY, None)
                .await?
                .unwrap_or(90) as i32,
            sync_threshold_movie: self
                .read_setting_i64_value(SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY, None)
                .await?
                .unwrap_or(70) as i32,
            sync_max_offset_seconds: self
                .read_setting_i64_value(SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY, None)
                .await?
                .unwrap_or(60) as i32,
        })
    }

    async fn load_acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        Ok(AcquisitionSettings {
            enabled: self
                .read_setting_bool_value(ACQUISITION_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
            upgrade_cooldown_hours: self
                .read_setting_i64_value(ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY, None)
                .await?
                .unwrap_or(24) as i32,
            same_tier_min_delta: self
                .read_setting_i64_value(ACQUISITION_SAME_TIER_MIN_DELTA_KEY, None)
                .await?
                .unwrap_or(120) as i32,
            cross_tier_min_delta: self
                .read_setting_i64_value(ACQUISITION_CROSS_TIER_MIN_DELTA_KEY, None)
                .await?
                .unwrap_or(30) as i32,
            forced_upgrade_delta_bypass: self
                .read_setting_i64_value(ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY, None)
                .await?
                .unwrap_or(400) as i32,
            poll_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_POLL_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(60) as i32,
            sync_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_SYNC_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(3600) as i32,
            batch_size: self
                .read_setting_i64_value(ACQUISITION_BATCH_SIZE_KEY, None)
                .await?
                .unwrap_or(50) as i32,
        })
    }

    pub(crate) async fn subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        self.load_subtitle_settings().await
    }

    pub(crate) async fn acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        self.load_acquisition_settings().await
    }

    pub(crate) async fn delay_profiles(&self) -> AppResult<Vec<crate::DelayProfile>> {
        let profiles = self
            .read_setting_json_value::<Vec<crate::DelayProfile>>(
                crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
                None,
            )
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(normalize_delay_profile)
            .collect::<Vec<_>>();

        crate::validate_delay_profile_catalog(&profiles)
            .map_err(AppError::Validation)?;

        Ok(profiles)
    }

    pub async fn get_subtitle_settings(&self, actor: &User) -> AppResult<SubtitleSettings> {
        require(actor, &Entitlement::ManageConfig)?;
        self.load_subtitle_settings().await
    }

    pub async fn get_acquisition_settings(&self, actor: &User) -> AppResult<AcquisitionSettings> {
        require(actor, &Entitlement::ManageConfig)?;
        self.load_acquisition_settings().await
    }

    pub async fn get_delay_profiles(&self, actor: &User) -> AppResult<Vec<crate::DelayProfile>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.delay_profiles().await
    }

    pub async fn update_subtitle_settings(
        &self,
        actor: &User,
        input: UpdateSubtitleSettings,
    ) -> AppResult<SubtitleSettings> {
        require(actor, &Entitlement::ManageConfig)?;

        if input.search_interval_hours < 1 {
            return Err(AppError::Validation(
                "subtitle search interval must be at least 1 hour".to_string(),
            ));
        }
        if input.minimum_score_series < 0 || input.minimum_score_movie < 0 {
            return Err(AppError::Validation(
                "subtitle minimum scores cannot be negative".to_string(),
            ));
        }
        if input.sync_threshold_series < 0
            || input.sync_threshold_movie < 0
            || input.sync_max_offset_seconds < 0
        {
            return Err(AppError::Validation(
                "subtitle sync settings cannot be negative".to_string(),
            ));
        }

        let current = self.load_subtitle_settings().await?;
        let username = normalize_optional_string(Some(input.open_subtitles_username));
        let languages = normalize_subtitle_languages(input.languages);
        let should_update_api_key = input.open_subtitles_api_key.is_some();
        let should_update_password = input.open_subtitles_password.is_some();
        let api_key_update = normalize_optional_string(input.open_subtitles_api_key);
        let password_update = normalize_optional_string(input.open_subtitles_password);

        self.upsert_system_setting_json(SUBTITLES_ENABLED_KEY, &input.enabled, Some(actor.id.clone()))
            .await?;
        self.upsert_system_setting_json(
            SUBTITLES_OPENSUBTITLES_USERNAME_KEY,
            &username,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_LANGUAGES_KEY,
            &languages,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY,
            &input.auto_download_on_import,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_MINIMUM_SCORE_SERIES_KEY,
            &input.minimum_score_series,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_MINIMUM_SCORE_MOVIE_KEY,
            &input.minimum_score_movie,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SEARCH_INTERVAL_HOURS_KEY,
            &input.search_interval_hours,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_INCLUDE_AI_TRANSLATED_KEY,
            &input.include_ai_translated,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY,
            &input.include_machine_translated,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_ENABLED_KEY,
            &input.sync_enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_THRESHOLD_SERIES_KEY,
            &input.sync_threshold_series,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY,
            &input.sync_threshold_movie,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY,
            &input.sync_max_offset_seconds,
            Some(actor.id.clone()),
        )
        .await?;

        let mut changed_keys = vec![
            SUBTITLES_ENABLED_KEY.to_string(),
            SUBTITLES_OPENSUBTITLES_USERNAME_KEY.to_string(),
            SUBTITLES_LANGUAGES_KEY.to_string(),
            SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY.to_string(),
            SUBTITLES_MINIMUM_SCORE_SERIES_KEY.to_string(),
            SUBTITLES_MINIMUM_SCORE_MOVIE_KEY.to_string(),
            SUBTITLES_SEARCH_INTERVAL_HOURS_KEY.to_string(),
            SUBTITLES_INCLUDE_AI_TRANSLATED_KEY.to_string(),
            SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY.to_string(),
            SUBTITLES_SYNC_ENABLED_KEY.to_string(),
            SUBTITLES_SYNC_THRESHOLD_SERIES_KEY.to_string(),
            SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY.to_string(),
            SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY.to_string(),
        ];

        if should_update_api_key {
            let next_api_key = api_key_update.or(current.open_subtitles_api_key);
            self.upsert_system_setting_json(
                SUBTITLES_OPENSUBTITLES_API_KEY,
                &next_api_key,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(SUBTITLES_OPENSUBTITLES_API_KEY.to_string());
        }

        if should_update_password {
            let next_password = password_update.or(current.open_subtitles_password);
            self.upsert_system_setting_json(
                SUBTITLES_OPENSUBTITLES_PASSWORD_KEY,
                &next_password,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(SUBTITLES_OPENSUBTITLES_PASSWORD_KEY.to_string());
        }

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                ActivityKind::SettingSaved,
                "subtitle settings updated".to_string(),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await;
        let _ = self.services.settings_changed_broadcast.send(changed_keys);

        self.load_subtitle_settings().await
    }

    pub async fn update_acquisition_settings(
        &self,
        actor: &User,
        settings: AcquisitionSettings,
    ) -> AppResult<AcquisitionSettings> {
        require(actor, &Entitlement::ManageConfig)?;

        if settings.upgrade_cooldown_hours < 0
            || settings.same_tier_min_delta < 0
            || settings.cross_tier_min_delta < 0
            || settings.forced_upgrade_delta_bypass < 0
        {
            return Err(AppError::Validation(
                "acquisition thresholds cannot be negative".to_string(),
            ));
        }
        if settings.poll_interval_seconds < 1 || settings.sync_interval_seconds < 1 {
            return Err(AppError::Validation(
                "acquisition intervals must be at least 1 second".to_string(),
            ));
        }
        if settings.batch_size < 1 {
            return Err(AppError::Validation(
                "acquisition batch size must be at least 1".to_string(),
            ));
        }

        self.upsert_system_setting_json(ACQUISITION_ENABLED_KEY, &settings.enabled, Some(actor.id.clone()))
            .await?;
        self.upsert_system_setting_json(
            ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY,
            &settings.upgrade_cooldown_hours,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY,
            &settings.same_tier_min_delta,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_CROSS_TIER_MIN_DELTA_KEY,
            &settings.cross_tier_min_delta,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY,
            &settings.forced_upgrade_delta_bypass,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY,
            &settings.poll_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_SYNC_INTERVAL_SECONDS_KEY,
            &settings.sync_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_BATCH_SIZE_KEY,
            &settings.batch_size,
            Some(actor.id.clone()),
        )
        .await?;

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                ActivityKind::SettingSaved,
                "acquisition settings updated".to_string(),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await;
        let _ = self.services.settings_changed_broadcast.send(vec![
            ACQUISITION_ENABLED_KEY.to_string(),
            ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY.to_string(),
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_CROSS_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY.to_string(),
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_SYNC_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_BATCH_SIZE_KEY.to_string(),
        ]);
        self.services.acquisition_wake.notify_one();

        self.load_acquisition_settings().await
    }

    pub async fn upsert_delay_profile(
        &self,
        actor: &User,
        profile: crate::DelayProfile,
    ) -> AppResult<crate::DelayProfile> {
        require(actor, &Entitlement::ManageConfig)?;

        let profile = normalize_delay_profile(profile);
        if profile.id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let mut profiles = self.delay_profiles().await?;
        if let Some(existing) = profiles.iter_mut().find(|existing| existing.id == profile.id) {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }

        crate::validate_delay_profile_catalog(&profiles)
            .map_err(AppError::Validation)?;
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &profiles,
            Some(actor.id.clone()),
        )
        .await?;

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                ActivityKind::SettingSaved,
                format!("delay profile '{}' saved", profile.name),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await;
        let _ = self.services.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.services.acquisition_wake.notify_one();

        Ok(profile)
    }

    pub async fn delete_delay_profile(
        &self,
        actor: &User,
        profile_id: &str,
    ) -> AppResult<String> {
        require(actor, &Entitlement::ManageConfig)?;

        let profile_id = profile_id.trim().to_string();
        if profile_id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let profiles = self.delay_profiles().await?;
        if !profiles.iter().any(|profile| profile.id == profile_id) {
            return Err(AppError::NotFound(format!(
                "delay profile {profile_id}"
            )));
        }

        let next_profiles: Vec<crate::DelayProfile> = profiles
            .into_iter()
            .filter(|profile| profile.id != profile_id)
            .collect();
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &next_profiles,
            Some(actor.id.clone()),
        )
        .await?;

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                None,
                ActivityKind::SettingSaved,
                format!("delay profile '{}' deleted", profile_id),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await;
        let _ = self.services.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.services.acquisition_wake.notify_one();

        Ok(profile_id)
    }

    pub(crate) async fn acquisition_thresholds(
        &self,
        persona: &ScoringPersona,
    ) -> AcquisitionThresholds {
        match self.load_acquisition_settings().await {
            Ok(settings) => settings.thresholds(),
            Err(error) => {
                warn!(error = %error, "failed to load acquisition settings, using persona defaults");
                AcquisitionThresholds::for_persona(persona)
            }
        }
    }
}
