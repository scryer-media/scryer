use super::*;

fn t() -> AcquisitionThresholds {
    AcquisitionThresholds::default()
}

// ── UpgradeDecision code/is_accept ────────────────────────────────────────

#[test]
fn upgrade_decision_codes() {
    assert_eq!(UpgradeDecision::AcceptInitial.code(), "accept_initial");
    assert_eq!(UpgradeDecision::AcceptUpgrade.code(), "accept_upgrade");
    assert_eq!(
        UpgradeDecision::RejectInsufficientDelta.code(),
        "reject_insufficient_delta"
    );
    assert_eq!(UpgradeDecision::RejectCooldown.code(), "reject_cooldown");
    assert_eq!(
        UpgradeDecision::RejectNotAllowed.code(),
        "reject_not_allowed"
    );
}

#[test]
fn upgrade_decision_is_accept() {
    assert!(UpgradeDecision::AcceptInitial.is_accept());
    assert!(UpgradeDecision::AcceptUpgrade.is_accept());
    assert!(!UpgradeDecision::RejectInsufficientDelta.is_accept());
    assert!(!UpgradeDecision::RejectCooldown.is_accept());
    assert!(!UpgradeDecision::RejectNotAllowed.is_accept());
}

// ── evaluate_upgrade ──────────────────────────────────────────────────────

#[test]
fn accept_initial_when_no_current_score() {
    let now = Utc::now();
    assert_eq!(
        evaluate_upgrade(500, None, true, None, &now, &t()),
        UpgradeDecision::AcceptInitial
    );
}

#[test]
fn reject_not_allowed_when_upgrades_disabled() {
    let now = Utc::now();
    assert_eq!(
        evaluate_upgrade(2000, Some(1000), false, None, &now, &t()),
        UpgradeDecision::RejectNotAllowed
    );
}

#[test]
fn reject_when_candidate_score_lower() {
    let now = Utc::now();
    assert_eq!(
        evaluate_upgrade(500, Some(1000), true, None, &now, &t()),
        UpgradeDecision::RejectInsufficientDelta
    );
}

#[test]
fn reject_when_candidate_score_equal() {
    let now = Utc::now();
    assert_eq!(
        evaluate_upgrade(1000, Some(1000), true, None, &now, &t()),
        UpgradeDecision::RejectInsufficientDelta
    );
}

#[test]
fn accept_same_tier_upgrade_with_sufficient_delta() {
    let now = Utc::now();
    // delta = 200, same_tier_min_delta = 200 (Balanced) → accept
    assert_eq!(
        evaluate_upgrade(1200, Some(1000), true, None, &now, &t()),
        UpgradeDecision::AcceptUpgrade
    );
}

#[test]
fn reject_same_tier_upgrade_with_insufficient_delta() {
    let now = Utc::now();
    // delta = 50, same_tier_min_delta = 200 (Balanced) → reject
    assert_eq!(
        evaluate_upgrade(1050, Some(1000), true, None, &now, &t()),
        UpgradeDecision::RejectInsufficientDelta
    );
}

#[test]
fn accept_cross_tier_upgrade_with_lower_threshold() {
    let now = Utc::now();
    // delta = 1050, cross_tier_min_delta = 30 → accept
    assert_eq!(
        evaluate_upgrade(2050, Some(1000), true, None, &now, &t()),
        UpgradeDecision::AcceptUpgrade
    );
}

#[test]
fn reject_cooldown_when_recently_imported() {
    let now = Utc::now();
    let recent_import = (now - Duration::hours(1)).to_rfc3339();
    // delta = 200 (sufficient), but within 24h cooldown and < 400 bypass
    assert_eq!(
        evaluate_upgrade(1200, Some(1000), true, Some(&recent_import), &now, &t()),
        UpgradeDecision::RejectCooldown
    );
}

#[test]
fn forced_bypass_during_cooldown() {
    let now = Utc::now();
    let recent_import = (now - Duration::hours(1)).to_rfc3339();
    // delta = 500 >= forced_upgrade_delta_bypass (400) → accept even during cooldown
    assert_eq!(
        evaluate_upgrade(1500, Some(1000), true, Some(&recent_import), &now, &t()),
        UpgradeDecision::AcceptUpgrade
    );
}

#[test]
fn accept_after_cooldown_expires() {
    let now = Utc::now();
    let old_import = (now - Duration::hours(25)).to_rfc3339();
    // delta = 200, cooldown expired → accept
    assert_eq!(
        evaluate_upgrade(1200, Some(1000), true, Some(&old_import), &now, &t()),
        UpgradeDecision::AcceptUpgrade
    );
}

#[test]
fn bad_import_time_treated_as_no_cooldown() {
    let now = Utc::now();
    // Invalid RFC3339 → treated as no cooldown
    assert_eq!(
        evaluate_upgrade(1200, Some(1000), true, Some("not-a-date"), &now, &t()),
        UpgradeDecision::AcceptUpgrade
    );
}

// ── Custom thresholds ─────────────────────────────────────────────────────

#[test]
fn custom_thresholds_same_tier() {
    let now = Utc::now();
    let thresholds = AcquisitionThresholds {
        same_tier_min_delta: 50,
        ..Default::default()
    };
    // delta = 60, custom same_tier_min_delta = 50 → accept
    assert_eq!(
        evaluate_upgrade(1060, Some(1000), true, None, &now, &thresholds),
        UpgradeDecision::AcceptUpgrade
    );
}

#[test]
fn custom_thresholds_short_cooldown() {
    let now = Utc::now();
    let thresholds = AcquisitionThresholds {
        upgrade_cooldown_hours: 1,
        ..Default::default()
    };
    let old_import = (now - Duration::hours(2)).to_rfc3339();
    // cooldown = 1h, import was 2h ago → cooldown expired
    assert_eq!(
        evaluate_upgrade(1200, Some(1000), true, Some(&old_import), &now, &thresholds),
        UpgradeDecision::AcceptUpgrade
    );
}

// ── compute_search_schedule: movie ────────────────────────────────────────

#[test]
fn movie_schedule_no_baseline() {
    let now = Utc::now();
    let schedule = compute_search_schedule("movie", None, "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Primary);
}

#[test]
fn movie_schedule_before_pre_release() {
    let now = Utc::now();
    let baseline = (now + Duration::days(30)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::PreRelease);
}

#[test]
fn movie_schedule_in_pre_release_window() {
    let now = Utc::now();
    let baseline = (now + Duration::hours(12)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::PreRelease);
}

#[test]
fn movie_schedule_primary_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::hours(1)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Primary);
}

#[test]
fn movie_schedule_secondary_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::days(10)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Secondary);
}

#[test]
fn movie_schedule_long_tail_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::days(60)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::LongTail);
}

#[test]
fn movie_schedule_old_release_stays_long_tail() {
    let now = Utc::now();
    let baseline = (now - Duration::days(200)).to_rfc3339();
    let schedule = compute_search_schedule("movie", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::LongTail);
}

// ── compute_search_schedule: episode ──────────────────────────────────────

#[test]
fn episode_schedule_no_baseline() {
    let now = Utc::now();
    let schedule = compute_search_schedule("episode", None, "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Primary);
}

#[test]
fn episode_schedule_before_pre_air() {
    let now = Utc::now();
    let baseline = (now + Duration::days(7)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::PreAir);
}

#[test]
fn episode_schedule_in_pre_air_window() {
    let now = Utc::now();
    let baseline = (now + Duration::hours(3)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::PreAir);
}

#[test]
fn episode_schedule_primary_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::hours(1)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Primary);
}

#[test]
fn episode_schedule_secondary_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::days(5)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Secondary);
}

#[test]
fn episode_schedule_long_tail_phase() {
    let now = Utc::now();
    let baseline = (now - Duration::days(30)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::LongTail);
}

#[test]
fn episode_schedule_old_airing_stays_long_tail() {
    let now = Utc::now();
    let baseline = (now - Duration::days(150)).to_rfc3339();
    let schedule = compute_search_schedule("episode", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::LongTail);
}

// ── unknown media type defaults to episode schedule ───────────────────────

#[test]
fn unknown_media_type_uses_episode_schedule() {
    let now = Utc::now();
    let baseline = (now - Duration::hours(1)).to_rfc3339();
    let schedule = compute_search_schedule("unknown", Some(&baseline), "primary", &now);
    assert_eq!(schedule.search_phase, SearchPhase::Primary);
}
