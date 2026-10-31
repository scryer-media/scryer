const SUBTITLES_ENABLED_KEY: &str = "subtitles.enabled";
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
fn normalize_subtitle_languages(languages: Vec<SubtitleLanguagePref>) -> Vec<SubtitleLanguagePref> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(languages.len());

    for language in languages {
        let Some(code) = normalize_subtitle_language_code(&language.code) else {
            continue;
        };
        let key = format!("{}:{}:{}", code, language.hearing_impaired, language.forced);
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
impl AppUseCase {
    async fn load_subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        Ok(SubtitleSettings {
            enabled: self
                .read_setting_bool_value(SUBTITLES_ENABLED_KEY, None)
                .await?
                .unwrap_or(false),
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
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| (0..=100).contains(value))
                .unwrap_or(90),
            minimum_score_movie: self
                .read_setting_i64_value(SUBTITLES_MINIMUM_SCORE_MOVIE_KEY, None)
                .await?
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| (0..=100).contains(value))
                .unwrap_or(70),
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
}
impl AppUseCase {
    pub(crate) async fn subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        self.load_subtitle_settings().await
    }
}
impl AppUseCase {
    pub async fn get_subtitle_settings(&self, actor: &User) -> AppResult<SubtitleSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.load_subtitle_settings().await
    }
}
impl AppUseCase {
    pub async fn update_subtitle_settings(
        &self,
        actor: &User,
        input: UpdateSubtitleSettings,
    ) -> AppResult<SubtitleSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if input.search_interval_hours < 1 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE, "subtitle search interval must be at least 1 hour".to_string()));
        }
        if !(0..=100).contains(&input.minimum_score_series)
            || !(0..=100).contains(&input.minimum_score_movie)
        {
            return Err(AppError::validation_refused(crate::error_reasons::settings::SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE, "subtitle minimum score percentages must be between 0 and 100".to_string()));
        }
        if !(0..=100).contains(&input.sync_threshold_series)
            || !(0..=100).contains(&input.sync_threshold_movie)
        {
            return Err(AppError::validation_refused(crate::error_reasons::settings::SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE, "subtitle sync threshold percentages must be between 0 and 100".to_string()));
        }
        if input.sync_max_offset_seconds < 0 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE, "subtitle sync max offset cannot be negative".to_string()));
        }

        let languages = normalize_subtitle_languages(input.languages);
        self.upsert_system_setting_json(
            SUBTITLES_ENABLED_KEY,
            &input.enabled,
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

        let changed_keys = vec![
            SUBTITLES_ENABLED_KEY.to_string(),
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

        self.emit_configuration_changed_event(
            actor,
            "subtitle_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(changed_keys);
        self.load_subtitle_settings().await
    }
}
