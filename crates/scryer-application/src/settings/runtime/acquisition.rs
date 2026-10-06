use crate::acquisition::convergence::{
    ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
    ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE,
};

const ACQUISITION_ENABLED_KEY: &str = "acquisition.enabled";
const ACQUISITION_SAME_TIER_MIN_DELTA_KEY: &str = "acquisition.same_tier_min_delta";
const ACQUISITION_POLL_INTERVAL_SECONDS_KEY: &str = "acquisition.poll_interval_seconds";
const ACQUISITION_DEFAULT_MOVIE_AVAILABILITY_KEY: &str = "acquisition.default_movie_availability";
const ACQUISITION_MOVIE_RELEASE_MARKET_KEY: &str = "acquisition.movie_release_market";
const ACQUISITION_MOVIE_AVAILABILITY_DELAY_DAYS_KEY: &str =
    "acquisition.movie_availability_delay_days";
pub(crate) const ACQUISITION_WALK_INTERVAL_SECONDS_KEY: &str = "acquisition.walk_interval_seconds";
/// Download-client failure check cadence when nothing is stored.
pub(crate) const DEFAULT_ACQUISITION_POLL_INTERVAL_SECONDS: i32 = 60;
/// Catalog walk cadence when nothing is stored.
pub(crate) const DEFAULT_ACQUISITION_WALK_INTERVAL_SECONDS: i32 = 300;
pub const DEFAULT_MOVIE_AVAILABILITY: &str = "announced";
pub const DEFAULT_MOVIE_RELEASE_MARKET: &str = "US";

#[derive(Debug, Clone)]
pub struct AcquisitionSettings {
    pub enabled: bool,
    pub same_tier_min_delta: i32,
    /// How often the download clients are read and failed grabs are handled.
    pub poll_interval_seconds: i32,
    /// How often the catalog is scanned for missing and upgradable scopes and
    /// a batch of titles is walked. Wakes still walk at once.
    pub walk_interval_seconds: i32,
    /// Per-cycle evaluation cost ceiling for the convergence cursor — how many scopes may be evaluated per tick, not a rate limiter.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Dormant slow re-converge backstop: coverage older than
    /// this many days re-converges. `0` = off, the intended steady state.
    pub long_tail_reconverge_days: i32,
    /// Default minimum movie availability applied only when creating new movies.
    pub default_movie_availability: String,
    /// ISO-3166-1 alpha-2 market used for movie release-date availability.
    pub movie_release_market: String,
    /// Signed whole-day offset from the selected movie release date.
    pub movie_availability_delay_days: i32,
}
impl AcquisitionSettings {
    /// The subset of these settings the acquisition gates read.
    ///
    /// Rows left behind by earlier releases under
    /// `acquisition.upgrade_cooldown_hours`, `acquisition.cross_tier_min_delta`
    /// and `acquisition.forced_upgrade_delta_bypass` are never loaded: no gate
    /// consults them any more, so they were removed rather than kept as inert
    /// knobs.
    pub fn thresholds(&self) -> AcquisitionThresholds {
        AcquisitionThresholds {
            same_tier_min_delta: self.same_tier_min_delta,
        }
    }
}
impl AppUseCase {
    async fn load_acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        Ok(AcquisitionSettings {
            enabled: self
                .read_setting_bool_value(ACQUISITION_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
            same_tier_min_delta: self
                .read_setting_i64_value(ACQUISITION_SAME_TIER_MIN_DELTA_KEY, None)
                .await?
                .unwrap_or(120) as i32,
            poll_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_POLL_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(DEFAULT_ACQUISITION_POLL_INTERVAL_SECONDS as i64)
                as i32,
            walk_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_WALK_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(DEFAULT_ACQUISITION_WALK_INTERVAL_SECONDS as i64)
                as i32,
            long_tail_backfill_max_scopes_per_cycle: self
                .read_setting_i64_value(
                    ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
                    None,
                )
                .await?
                .unwrap_or(DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE)
                as i32,
            long_tail_reconverge_days: self
                .read_setting_i64_value(ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, None)
                .await?
                .unwrap_or(0) as i32,
            default_movie_availability: self
                .read_setting_string_value(ACQUISITION_DEFAULT_MOVIE_AVAILABILITY_KEY, None)
                .await?
                .unwrap_or_else(|| DEFAULT_MOVIE_AVAILABILITY.to_string()),
            movie_release_market: self
                .read_setting_string_value(ACQUISITION_MOVIE_RELEASE_MARKET_KEY, None)
                .await?
                .unwrap_or_else(|| DEFAULT_MOVIE_RELEASE_MARKET.to_string()),
            movie_availability_delay_days: self
                .read_setting_i64_value(ACQUISITION_MOVIE_AVAILABILITY_DELAY_DAYS_KEY, None)
                .await?
                .unwrap_or(0) as i32,
        })
    }
}
impl AppUseCase {
    pub(crate) async fn acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        self.load_acquisition_settings().await
    }
}
impl AppUseCase {
    pub async fn get_acquisition_settings(&self, actor: &User) -> AppResult<AcquisitionSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.load_acquisition_settings().await
    }
}
impl AppUseCase {
    pub async fn update_acquisition_settings(
        &self,
        actor: &User,
        mut settings: AcquisitionSettings,
    ) -> AppResult<AcquisitionSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let previous_settings = self.load_acquisition_settings().await?;

        if settings.same_tier_min_delta < 0 {
            return Err(AppError::Validation(
                "acquisition thresholds cannot be negative".to_string(),
            ));
        }
        if settings.poll_interval_seconds < 1 {
            return Err(AppError::Validation(
                "acquisition poll interval must be at least 1 second".to_string(),
            ));
        }
        if settings.walk_interval_seconds < 1 {
            return Err(AppError::Validation(
                "acquisition walk interval must be at least 1 second".to_string(),
            ));
        }
        if settings.long_tail_backfill_max_scopes_per_cycle < 1 {
            return Err(AppError::Validation(
                "convergence per-cycle scope ceiling must be at least 1".to_string(),
            ));
        }
        if settings.long_tail_reconverge_days < 0 {
            return Err(AppError::Validation(
                "re-converge backstop cannot be negative".to_string(),
            ));
        }
        if !matches!(
            settings.default_movie_availability.as_str(),
            "announced" | "in_cinemas" | "released"
        ) {
            return Err(AppError::Validation(
                "default movie availability must be announced, in_cinemas, or released".into(),
            ));
        }
        let release_market = settings.movie_release_market.trim().to_ascii_uppercase();
        if release_market.len() != 2 || !release_market.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(AppError::Validation(
                "movie release market must be a two-letter ISO-3166 country code".into(),
            ));
        }
        settings.movie_release_market = release_market;

        self.upsert_system_setting_json(
            ACQUISITION_ENABLED_KEY,
            &settings.enabled,
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
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY,
            &settings.poll_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_WALK_INTERVAL_SECONDS_KEY,
            &settings.walk_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
            &settings.long_tail_backfill_max_scopes_per_cycle,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY,
            &settings.long_tail_reconverge_days,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_DEFAULT_MOVIE_AVAILABILITY_KEY,
            &settings.default_movie_availability,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_MOVIE_RELEASE_MARKET_KEY,
            &settings.movie_release_market,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_MOVIE_AVAILABILITY_DELAY_DAYS_KEY,
            &settings.movie_availability_delay_days,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "acquisition_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        if previous_settings.movie_release_market != settings.movie_release_market {
            self.queue_movie_release_market_refresh().await?;
        }
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            ACQUISITION_ENABLED_KEY.to_string(),
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_WALK_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY.to_string(),
            ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY.to_string(),
            ACQUISITION_DEFAULT_MOVIE_AVAILABILITY_KEY.to_string(),
            ACQUISITION_MOVIE_RELEASE_MARKET_KEY.to_string(),
            ACQUISITION_MOVIE_AVAILABILITY_DELAY_DAYS_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        self.load_acquisition_settings().await
    }
}
impl AppUseCase {
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
