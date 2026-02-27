use chrono::{DateTime, Duration, Utc};

/// Flat polling interval for movies without a baseline date.
const MOVIE_FALLBACK_INTERVAL_HOURS: i64 = 12;

/// Flat polling interval for episodes without a baseline date.
const EPISODE_FALLBACK_INTERVAL_HOURS: i64 = 24;

/// Configurable thresholds for the acquisition upgrade policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionThresholds {
    pub upgrade_cooldown_hours: i64,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
}

impl Default for AcquisitionThresholds {
    fn default() -> Self {
        Self {
            upgrade_cooldown_hours: 24,
            same_tier_min_delta: 120,
            cross_tier_min_delta: 30,
            forced_upgrade_delta_bypass: 400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeDecision {
    AcceptInitial,
    AcceptUpgrade,
    RejectInsufficientDelta,
    RejectCooldown,
    RejectNotAllowed,
}

impl UpgradeDecision {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AcceptInitial => "accept_initial",
            Self::AcceptUpgrade => "accept_upgrade",
            Self::RejectInsufficientDelta => "reject_insufficient_delta",
            Self::RejectCooldown => "reject_cooldown",
            Self::RejectNotAllowed => "reject_not_allowed",
        }
    }

    pub fn is_accept(&self) -> bool {
        matches!(self, Self::AcceptInitial | Self::AcceptUpgrade)
    }
}

pub fn evaluate_upgrade(
    candidate_score: i32,
    current_score: Option<i32>,
    allow_upgrades: bool,
    last_import_at: Option<&str>,
    now: &DateTime<Utc>,
    thresholds: &AcquisitionThresholds,
) -> UpgradeDecision {
    let Some(current) = current_score else {
        return UpgradeDecision::AcceptInitial;
    };

    if !allow_upgrades {
        return UpgradeDecision::RejectNotAllowed;
    }

    let delta = candidate_score - current;
    if delta <= 0 {
        return UpgradeDecision::RejectInsufficientDelta;
    }

    // Check cooldown
    if let Some(import_time_str) = last_import_at {
        if let Ok(import_time) = DateTime::parse_from_rfc3339(import_time_str) {
            let cooldown_end = import_time + Duration::hours(thresholds.upgrade_cooldown_hours);
            if *now < cooldown_end.with_timezone(&Utc) {
                if delta >= thresholds.forced_upgrade_delta_bypass {
                    return UpgradeDecision::AcceptUpgrade;
                }
                return UpgradeDecision::RejectCooldown;
            }
        }
    }

    // Cross-tier upgrades (delta >= 1000 due to tier_weight) use the lower threshold.
    let min_delta = if delta >= 1000 {
        thresholds.cross_tier_min_delta
    } else {
        thresholds.same_tier_min_delta
    };

    if delta >= min_delta {
        return UpgradeDecision::AcceptUpgrade;
    }

    UpgradeDecision::RejectInsufficientDelta
}

#[derive(Debug, Clone)]
pub struct SearchSchedule {
    pub next_search_at: String,
    pub search_phase: String,
}

/// Compute the next search schedule based on media type, baseline date, and current phase.
pub fn compute_search_schedule(
    media_type: &str,
    baseline_date: Option<&str>,
    current_phase: &str,
    now: &DateTime<Utc>,
) -> SearchSchedule {
    let baseline = baseline_date
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
        .map(|dt| dt.with_timezone(&Utc));

    match media_type {
        "movie" => compute_movie_schedule(baseline, current_phase, now),
        "episode" => compute_episode_schedule(baseline, current_phase, now),
        _ => compute_episode_schedule(baseline, current_phase, now),
    }
}

fn compute_movie_schedule(
    baseline: Option<DateTime<Utc>>,
    _current_phase: &str,
    now: &DateTime<Utc>,
) -> SearchSchedule {
    let Some(baseline) = baseline else {
        return SearchSchedule {
            next_search_at: (*now + Duration::hours(MOVIE_FALLBACK_INTERVAL_HOURS)).to_rfc3339(),
            search_phase: "primary".to_string(),
        };
    };

    // Movie phases:
    // pre_release: baseline -24h to baseline, every 60m
    // primary: baseline to +7d, every 15m
    // secondary: +7d to +30d, every 2h
    // long_tail: +30d to +180d, every 12h

    let pre_release_start = baseline - Duration::hours(24);
    let primary_end = baseline + Duration::days(7);
    let secondary_end = baseline + Duration::days(30);
    let long_tail_end = baseline + Duration::days(180);

    if *now < pre_release_start {
        // Not yet in any phase — schedule for pre_release start
        return SearchSchedule {
            next_search_at: pre_release_start.to_rfc3339(),
            search_phase: "pre_release".to_string(),
        };
    }

    // Determine current effective phase based on time
    let (phase, interval) = if *now < baseline {
        ("pre_release", Duration::minutes(60))
    } else if *now < primary_end {
        ("primary", Duration::minutes(15))
    } else if *now < secondary_end {
        ("secondary", Duration::hours(2))
    } else if *now < long_tail_end {
        ("long_tail", Duration::hours(12))
    } else {
        return SearchSchedule {
            next_search_at: (*now + Duration::hours(24)).to_rfc3339(),
            search_phase: "paused".to_string(),
        };
    };

    SearchSchedule {
        next_search_at: (*now + interval).to_rfc3339(),
        search_phase: phase.to_string(),
    }
}

fn compute_episode_schedule(
    baseline: Option<DateTime<Utc>>,
    _current_phase: &str,
    now: &DateTime<Utc>,
) -> SearchSchedule {
    let Some(baseline) = baseline else {
        return SearchSchedule {
            next_search_at: (*now + Duration::hours(EPISODE_FALLBACK_INTERVAL_HOURS)).to_rfc3339(),
            search_phase: "primary".to_string(),
        };
    };

    // Episode phases:
    // pre_air: air -6h to air, every 30m
    // primary: air to +72h, every 10m
    // secondary: +72h to +21d, every 2h
    // long_tail: +21d to +120d, every 12h

    let pre_air_start = baseline - Duration::hours(6);
    let primary_end = baseline + Duration::hours(72);
    let secondary_end = baseline + Duration::days(21);
    let long_tail_end = baseline + Duration::days(120);

    if *now < pre_air_start {
        return SearchSchedule {
            next_search_at: pre_air_start.to_rfc3339(),
            search_phase: "pre_air".to_string(),
        };
    }

    let (phase, interval) = if *now < baseline {
        ("pre_air", Duration::minutes(30))
    } else if *now < primary_end {
        ("primary", Duration::minutes(10))
    } else if *now < secondary_end {
        ("secondary", Duration::hours(2))
    } else if *now < long_tail_end {
        ("long_tail", Duration::hours(12))
    } else {
        return SearchSchedule {
            next_search_at: (*now + Duration::hours(24)).to_rfc3339(),
            search_phase: "paused".to_string(),
        };
    };

    SearchSchedule {
        next_search_at: (*now + interval).to_rfc3339(),
        search_phase: phase.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> AcquisitionThresholds {
        AcquisitionThresholds::default()
    }

    #[test]
    fn test_accept_initial_when_no_current() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1000, None, true, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::AcceptInitial);
    }

    #[test]
    fn test_reject_lower_score() {
        let now = Utc::now();
        let decision = evaluate_upgrade(500, Some(1000), true, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_accept_sufficient_delta() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1200, Some(1000), true, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_reject_insufficient_delta() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1050, Some(1000), true, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_reject_cooldown() {
        let now = Utc::now();
        let recent_import = (now - Duration::hours(1)).to_rfc3339();
        let decision = evaluate_upgrade(1200, Some(1000), true, Some(&recent_import), &now, &t());
        assert_eq!(decision, UpgradeDecision::RejectCooldown);
    }

    #[test]
    fn test_accept_forced_bypass_during_cooldown() {
        let now = Utc::now();
        let recent_import = (now - Duration::hours(1)).to_rfc3339();
        let decision = evaluate_upgrade(1500, Some(1000), true, Some(&recent_import), &now, &t());
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_reject_upgrades_not_allowed() {
        let now = Utc::now();
        let decision = evaluate_upgrade(2000, Some(1000), false, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::RejectNotAllowed);
    }

    #[test]
    fn test_cross_tier_upgrade_uses_lower_delta() {
        let now = Utc::now();
        // Cross-tier: delta >= 1000, so cross_tier_min_delta (30) applies
        let decision = evaluate_upgrade(2050, Some(1000), true, None, &now, &t());
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_movie_schedule_before_baseline() {
        let now = Utc::now();
        let baseline = (now + Duration::days(7)).to_rfc3339();
        let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
        assert_eq!(schedule.search_phase, "pre_release");
    }

    #[test]
    fn test_episode_schedule_after_air() {
        let now = Utc::now();
        let baseline = (now - Duration::hours(1)).to_rfc3339();
        let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
        assert_eq!(schedule.search_phase, "primary");
    }

    #[test]
    fn test_episode_schedule_no_baseline() {
        let now = Utc::now();
        let schedule = compute_search_schedule("episode", None, "primary", &now);
        assert_eq!(schedule.search_phase, "primary");
    }
}

#[cfg(test)]
#[path = "acquisition_policy_tests.rs"]
mod acquisition_policy_tests;
