use crate::acquisition::convergence::{
    ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
    ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE,
};

pub(crate) const ACQUISITION_ENABLED_KEY: &str = "acquisition.enabled";

const ACQUISITION_POLL_INTERVAL_SECONDS_KEY: &str = "acquisition.poll_interval_seconds";
pub(crate) const ACQUISITION_WALK_INTERVAL_SECONDS_KEY: &str = "acquisition.walk_interval_seconds";
pub(crate) const ACQUISITION_WALK_INTENSITY_KEY: &str = "acquisition.walk_intensity";
/// Download-client failure check cadence when nothing is stored.
pub(crate) const DEFAULT_ACQUISITION_POLL_INTERVAL_SECONDS: i32 = 60;
/// Catalog walk cadence when nothing is stored.
pub(crate) const DEFAULT_ACQUISITION_WALK_INTERVAL_SECONDS: i32 = 300;

#[derive(Debug, Clone)]
pub struct AcquisitionSettings {
    pub enabled: bool,
    /// How often the download clients are read and failed grabs are handled.
    pub poll_interval_seconds: i32,
    /// How often the catalog is scanned for missing and upgradable scopes and
    /// a batch of titles is walked. Wakes still walk at once.
    pub walk_interval_seconds: i32,
    /// How much of the machine the walk may take: its share of wall time,
    /// titles at once and indexer pages per cold scope.
    pub walk_intensity: crate::acquisition::walk_intensity::AcquisitionWalkIntensity,
    /// Per-cycle evaluation cost ceiling for the convergence cursor — how many scopes may be evaluated per tick, not a rate limiter.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Dormant slow re-converge backstop: coverage older than
    /// this many days re-converges. `0` = off, the intended steady state.
    pub long_tail_reconverge_days: i32,
}
// Rows left behind by earlier releases under
// `acquisition.upgrade_cooldown_hours`, `acquisition.cross_tier_min_delta`,
// `acquisition.forced_upgrade_delta_bypass` and `acquisition.same_tier_min_delta`
// are never loaded. The first three no longer mean anything; the same-tier
// delta moved to `QualityProfileCriteria::min_upgrade_score_delta`, and the
// migration that introduced it carried any non-default value into the profiles.
impl AppUseCase {
    async fn load_acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        Ok(AcquisitionSettings {
            enabled: self
                .read_setting_bool_value(ACQUISITION_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
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
            walk_intensity: self.acquisition_walk_intensity().await?,
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
        })
    }
}
impl AppUseCase {
    /// The stored walk intensity; a missing or unrecognised row reads as the
    /// default.
    pub(crate) async fn acquisition_walk_intensity(
        &self,
    ) -> AppResult<crate::acquisition::walk_intensity::AcquisitionWalkIntensity> {
        Ok(self
            .read_setting_string_value(ACQUISITION_WALK_INTENSITY_KEY, None)
            .await?
            .as_deref()
            .and_then(crate::acquisition::walk_intensity::AcquisitionWalkIntensity::parse)
            .unwrap_or_default())
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
        settings: AcquisitionSettings,
    ) -> AppResult<AcquisitionSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if settings.poll_interval_seconds < 1 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE, "acquisition poll interval must be at least 1 second".to_string()));
        }
        if settings.walk_interval_seconds < 1 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE, "acquisition walk interval must be at least 1 second".to_string()));
        }
        if settings.long_tail_backfill_max_scopes_per_cycle < 1 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE, "convergence per-cycle scope ceiling must be at least 1".to_string()));
        }
        if settings.long_tail_reconverge_days < 0 {
            return Err(AppError::validation_refused(crate::error_reasons::settings::RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE, "re-converge backstop cannot be negative".to_string()));
        }

        self.upsert_system_setting_json(
            ACQUISITION_ENABLED_KEY,
            &settings.enabled,
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
            ACQUISITION_WALK_INTENSITY_KEY,
            &settings.walk_intensity.as_str(),
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

        self.emit_configuration_changed_event(
            actor,
            "acquisition_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            ACQUISITION_ENABLED_KEY.to_string(),
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_WALK_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_WALK_INTENSITY_KEY.to_string(),
            ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY.to_string(),
            ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        self.load_acquisition_settings().await
    }
}
