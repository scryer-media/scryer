use crate::acquisition::convergence::{
    ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
    ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE,
};

const ACQUISITION_ENABLED_KEY: &str = "acquisition.enabled";
const ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY: &str = "acquisition.upgrade_cooldown_hours";
const ACQUISITION_SAME_TIER_MIN_DELTA_KEY: &str = "acquisition.same_tier_min_delta";
const ACQUISITION_CROSS_TIER_MIN_DELTA_KEY: &str = "acquisition.cross_tier_min_delta";
const ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY: &str = "acquisition.forced_upgrade_delta_bypass";
const ACQUISITION_POLL_INTERVAL_SECONDS_KEY: &str = "acquisition.poll_interval_seconds";

#[derive(Debug, Clone)]
pub struct AcquisitionSettings {
    pub enabled: bool,
    pub upgrade_cooldown_hours: i32,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
    pub poll_interval_seconds: i32,
    /// Per-cycle evaluation cost ceiling for the convergence cursor — how many scopes may be evaluated per tick, not a rate limiter.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Dormant slow re-converge backstop: coverage older than
    /// this many days re-converges. `0` = off, the intended steady state.
    pub long_tail_reconverge_days: i32,
}
impl AcquisitionSettings {
    /// The subset of these settings the acquisition gates actually read.
    ///
    /// `cross_tier_min_delta` is deliberately absent: since the quality tier
    /// left the score, a better tier admits outright in [`crate::admission`] and
    /// no delta threshold ever sees a cross-tier comparison. The setting and its
    /// GraphQL field are retained so stored values and clients keep working;
    /// nothing reads them.
    pub fn thresholds(&self) -> AcquisitionThresholds {
        AcquisitionThresholds {
            upgrade_cooldown_hours: self.upgrade_cooldown_hours as i64,
            same_tier_min_delta: self.same_tier_min_delta,
            forced_upgrade_delta_bypass: self.forced_upgrade_delta_bypass,
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

        if settings.upgrade_cooldown_hours < 0
            || settings.same_tier_min_delta < 0
            || settings.cross_tier_min_delta < 0
            || settings.forced_upgrade_delta_bypass < 0
        {
            return Err(AppError::Validation(
                "acquisition thresholds cannot be negative".to_string(),
            ));
        }
        if settings.poll_interval_seconds < 1 {
            return Err(AppError::Validation(
                "acquisition poll interval must be at least 1 second".to_string(),
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

        self.upsert_system_setting_json(
            ACQUISITION_ENABLED_KEY,
            &settings.enabled,
            Some(actor.id.clone()),
        )
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
            ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY.to_string(),
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_CROSS_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY.to_string(),
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY.to_string(),
            ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY.to_string(),
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
