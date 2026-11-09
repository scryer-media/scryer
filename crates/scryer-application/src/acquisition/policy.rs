use chrono::{DateTime, Duration, NaiveDate, Utc};

use crate::quality_profile::{DEFAULT_MIN_UPGRADE_SCORE_DELTA, QualityProfileCriteria};

/// Episodes become searchable twelve hours before air time, for active search
/// and for automatic RSS grabs alike.
pub(crate) const EPISODE_PRE_AIR_WINDOW_HOURS: i64 = 12;

/// Thresholds for the acquisition upgrade policy.
///
/// Only a same-tier score delta is configurable, and it belongs to the title's
/// quality profile. Tier is compared before score, in [`crate::admission`], so a
/// better tier admits outright and a delta only ever describes a same-tier
/// comparison. There is no upgrade cooldown, and so no large-delta bypass of
/// one; [`crate::admission`] explains why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionThresholds {
    pub same_tier_min_delta: i32,
}

impl Default for AcquisitionThresholds {
    fn default() -> Self {
        Self {
            same_tier_min_delta: DEFAULT_MIN_UPGRADE_SCORE_DELTA,
        }
    }
}

impl AcquisitionThresholds {
    /// The thresholds a title's quality profile asks for.
    pub fn for_profile(criteria: &QualityProfileCriteria) -> Self {
        Self {
            same_tier_min_delta: criteria.min_upgrade_score_delta_or_default(),
        }
    }
}

pub(crate) fn parse_schedule_baseline_date(baseline_date: Option<&str>) -> Option<DateTime<Utc>> {
    baseline_date.and_then(|d| {
        // Try RFC 3339 first, then fall back to "YYYY-MM-DD" (midnight UTC).
        DateTime::parse_from_rfc3339(d)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                NaiveDate::parse_from_str(d, "%Y-%m-%d")
                    .ok()
                    .and_then(|nd| nd.and_hms_opt(0, 0, 0))
                    .map(|ndt| ndt.and_utc())
            })
    })
}

pub(crate) fn episode_search_window_is_open(
    baseline_date: Option<&str>,
    now: &DateTime<Utc>,
) -> bool {
    episode_search_not_before(baseline_date).is_some_and(|opens_at| *now >= opens_at)
}

pub(crate) fn episode_search_not_before(baseline_date: Option<&str>) -> Option<DateTime<Utc>> {
    parse_schedule_baseline_date(baseline_date)
        .map(|baseline| baseline - Duration::hours(EPISODE_PRE_AIR_WINDOW_HOURS))
}

/// How old a file has to be before Scryer stops chasing PROPERs for it.
///
/// Sonarr's `ProperSpecification`: `file.DateAdded < DateTime.Today.AddDays(-7)`
/// rejects `ProperForOldFile`. The reasoning is that a PROPER posted a week
/// after the fact is usually a re-release of an old encode rather than a fix an
/// operator is waiting for, and the bandwidth is better spent elsewhere.
pub(crate) const PROPER_MAX_FILE_AGE_DAYS: i64 = 7;

/// Whether an incumbent landed before the PROPER window closed.
///
/// Measured from **UTC midnight** rather than Sonarr's local `DateTime.Today`,
/// because every timestamp Scryer stores is RFC 3339 UTC and reading a stored
/// instant against a local calendar day would make the guard fire at a
/// different age depending on the host's zone.
pub(crate) fn file_predates_proper_window(created_at: Option<&str>, now: &DateTime<Utc>) -> bool {
    let Some(created) = parse_schedule_baseline_date(created_at) else {
        // An unparseable or absent import time is not evidence of age. The
        // guard only ever *refuses*, so the safe reading is "recent enough".
        return false;
    };
    let Some(today) = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|day| day.and_utc())
    else {
        return false;
    };
    created < today - Duration::days(PROPER_MAX_FILE_AGE_DAYS)
}

/// Grace window before an air date at which a season stops counting as "still
/// airing" for pack purposes. Sonarr's `FullSeasonSpecification` uses
/// `AirDateUtc > UtcNow.AddHours(24)`: the finale's own air day does not block
/// the pack, because by the time a pack for it is posted the episode exists.
pub(crate) const SEASON_PACK_AIRED_GRACE_HOURS: i64 = 24;

/// Whether an episode is far enough in the future that no release can contain it.
///
/// An episode with **no air date** is not unaired: an unknown schedule is not
/// evidence of a future one, and treating it as such would block packs for every
/// series whose metadata is thin. Only a date more than
/// [`SEASON_PACK_AIRED_GRACE_HOURS`] ahead counts.
///
/// A **date-only** air date is compared as the *end* of that day. Sonarr reads
/// `AirDateUtc`, a real timestamp; `parse_schedule_baseline_date` turns
/// `YYYY-MM-DD` into midnight UTC, which is up to a full day earlier than the
/// broadcast. Taking midnight literally means a finale airing tomorrow at 21:00
/// looks ~12 h away, clears the 24 h grace, and the pack is admitted the day
/// before the episode exists — precisely the partial fetch this rule prevents.
pub(crate) fn episode_is_unaired(air_date: Option<&str>, now: &DateTime<Utc>) -> bool {
    let Some(air) = parse_schedule_baseline_date(air_date) else {
        return false;
    };
    let air = if air_date_is_date_only(air_date) {
        air + Duration::hours(24)
    } else {
        air
    };
    air > *now + Duration::hours(SEASON_PACK_AIRED_GRACE_HOURS)
}

/// `true` when the catalog gave us a day but not a time.
fn air_date_is_date_only(air_date: Option<&str>) -> bool {
    air_date.is_some_and(|value| {
        let value = value.trim();
        DateTime::parse_from_rfc3339(value).is_err()
            && NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_without_a_delta_uses_the_default() {
        let criteria = QualityProfileCriteria::default();
        assert_eq!(criteria.min_upgrade_score_delta, None);
        assert_eq!(
            AcquisitionThresholds::for_profile(&criteria).same_tier_min_delta,
            DEFAULT_MIN_UPGRADE_SCORE_DELTA
        );
        assert_eq!(
            AcquisitionThresholds::for_profile(&criteria),
            AcquisitionThresholds::default()
        );
    }

    #[test]
    fn a_profile_delta_replaces_the_default() {
        let criteria = QualityProfileCriteria {
            min_upgrade_score_delta: Some(5),
            ..QualityProfileCriteria::default()
        };
        assert_eq!(
            AcquisitionThresholds::for_profile(&criteria).same_tier_min_delta,
            5
        );
        let zero = QualityProfileCriteria {
            min_upgrade_score_delta: Some(0),
            ..QualityProfileCriteria::default()
        };
        assert_eq!(
            AcquisitionThresholds::for_profile(&zero).same_tier_min_delta,
            0
        );
    }
}

#[cfg(test)]
#[path = "acquisition_policy_tests.rs"]
mod acquisition_policy_tests;

#[cfg(test)]
mod unaired_tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-21T12:00:00Z")
            .expect("fixture timestamp")
            .with_timezone(&Utc)
    }

    /// Sonarr's grace window: `AirDateUtc > UtcNow.AddHours(24)`. The finale's
    /// own air day does not block the pack, because by the time a pack for it is
    /// posted the episode exists.
    #[test]
    fn only_air_dates_beyond_the_grace_window_count_as_unaired() {
        let now = now();
        let at = |offset: Duration| (now + offset).to_rfc3339();

        assert!(episode_is_unaired(Some(&at(Duration::days(7))), &now));
        assert!(episode_is_unaired(Some(&at(Duration::hours(25))), &now));
        assert!(!episode_is_unaired(Some(&at(Duration::hours(24))), &now));
        assert!(!episode_is_unaired(Some(&at(Duration::hours(6))), &now));
        assert!(!episode_is_unaired(Some(&at(-Duration::days(30))), &now));
    }

    /// An unknown schedule is not evidence of a future one. Treating a missing
    /// air date as unaired would block packs for every series with thin
    /// metadata, which is most anime.
    #[test]
    fn an_episode_without_an_air_date_is_not_unaired() {
        assert!(!episode_is_unaired(None, &now()));
        assert!(!episode_is_unaired(Some(""), &now()));
        assert!(!episode_is_unaired(Some("not-a-date"), &now()));
    }

    /// Date-only air dates (the common catalog shape) are compared as the **end**
    /// of the air day, not midnight.
    ///
    /// `now` is 2026-08-21 12:00 UTC, so the grace window closes at
    /// 2026-08-22 12:00. An episode dated 2026-08-22 broadcasts at some unknown
    /// hour of that day; midnight would put it inside the window and admit a
    /// pack for a season whose next episode has not aired. End-of-day
    /// (2026-08-23 00:00) keeps it out, which is what the rule is for.
    #[test]
    fn date_only_air_dates_are_measured_from_the_end_of_the_air_day() {
        let now = now();
        assert!(episode_is_unaired(Some("2026-08-25"), &now));
        assert!(
            episode_is_unaired(Some("2026-08-22"), &now),
            "an episode airing at some hour of tomorrow must still block a pack"
        );
        // Today's episode: the day ends 12 h from now, inside the grace window.
        assert!(!episode_is_unaired(Some("2026-08-21"), &now));
        assert!(!episode_is_unaired(Some("2026-08-01"), &now));
    }

    /// A real timestamp is taken at face value — the end-of-day correction is
    /// only for values that carry no time.
    #[test]
    fn a_timestamped_air_date_is_not_pushed_to_the_end_of_its_day() {
        let now = now();
        // 2026-08-22 06:00 is 18 h away: inside the grace window, so not unaired.
        // A date-only "2026-08-22" would be, which is the whole distinction.
        assert!(!episode_is_unaired(Some("2026-08-22T06:00:00Z"), &now));
        assert!(episode_is_unaired(Some("2026-08-22"), &now));
    }
}
