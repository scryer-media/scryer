//! Schedule evaluation for scripts started by the job scheduler.
//!
//! Every [`ScriptSchedule`] variant answers the same question: when does the
//! script next fire strictly after a given instant. Daily, weekly and cron
//! schedules evaluate in host-local time, the same way the daily backup job
//! does.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc, Weekday};
use croner::Cron;
use croner::parser::{CronParser, Seconds, Year};
use scryer_domain::{ScheduleWeekday, ScriptSchedule};

use crate::AppError;

/// Shortest interval a scheduled script may repeat at.
pub const MIN_INTERVAL_SECONDS: i64 = 60;

/// How far past a nonexistent local time (a daylight-saving gap) a local
/// schedule slides to find the first real instant.
const DST_GAP_SEARCH_MINUTES: i64 = 180;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ScheduleError(String);

impl ScheduleError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl ScheduleError {
    pub fn into_app_error(self) -> AppError {
        AppError::Validation(self.0)
    }
}

/// Outcome of checking a schedule a user is editing: whether it is valid,
/// why not, how it reads, and when it would next fire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptScheduleValidation {
    pub valid: bool,
    pub error: Option<String>,
    pub description: Option<String>,
    pub next_runs: Vec<DateTime<Utc>>,
}

/// How many upcoming fire times a schedule check previews.
pub const PREVIEW_RUN_COUNT: usize = 3;

/// Validates `schedule` and previews its next fire times after `after`.
pub fn check_schedule(schedule: &ScriptSchedule, after: DateTime<Utc>) -> ScriptScheduleValidation {
    match upcoming_fires(schedule, after, PREVIEW_RUN_COUNT) {
        Ok(next_runs) => ScriptScheduleValidation {
            valid: true,
            error: None,
            description: Some(describe_schedule(schedule)),
            next_runs,
        },
        Err(error) => ScriptScheduleValidation {
            valid: false,
            error: Some(error.0),
            description: None,
            next_runs: Vec::new(),
        },
    }
}

/// Checks that a schedule can be evaluated. Cron expressions are five-field
/// crontab lines (minute, hour, day of month, month, day of week); a leading
/// seconds field is also accepted.
pub fn validate_schedule(schedule: &ScriptSchedule) -> Result<(), ScheduleError> {
    match schedule {
        ScriptSchedule::Manual => Ok(()),
        ScriptSchedule::Interval { every_seconds } => {
            if *every_seconds < MIN_INTERVAL_SECONDS {
                return Err(ScheduleError::new(format!(
                    "interval must be at least {MIN_INTERVAL_SECONDS} seconds"
                )));
            }
            Ok(())
        }
        ScriptSchedule::Daily { time_local } => parse_local_time_of_day(time_local).map(|_| ()),
        ScriptSchedule::Weekly { days, time_local } => {
            if days.is_empty() {
                return Err(ScheduleError::new("weekly schedule needs at least one day"));
            }
            parse_local_time_of_day(time_local).map(|_| ())
        }
        ScriptSchedule::Cron { expression } => parse_cron(expression).map(|_| ()),
    }
}

/// The first time `schedule` fires strictly after `after`, or `None` for a
/// manual-only schedule.
pub fn next_fire_after(
    schedule: &ScriptSchedule,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, ScheduleError> {
    validate_schedule(schedule)?;
    match schedule {
        ScriptSchedule::Manual => Ok(None),
        ScriptSchedule::Interval { every_seconds } => after
            .checked_add_signed(chrono::Duration::seconds(*every_seconds))
            .map(Some)
            .ok_or_else(|| ScheduleError::new("interval is out of range")),
        ScriptSchedule::Daily { time_local } => {
            let (hour, minute) = parse_local_time_of_day(time_local)?;
            next_local_time_on_days(after, hour, minute, |_| true).map(Some)
        }
        ScriptSchedule::Weekly { days, time_local } => {
            let (hour, minute) = parse_local_time_of_day(time_local)?;
            next_local_time_on_days(after, hour, minute, |weekday| {
                days.iter().any(|day| to_chrono_weekday(*day) == weekday)
            })
            .map(Some)
        }
        ScriptSchedule::Cron { expression } => {
            let cron = parse_cron(expression)?;
            let after_local = after.with_timezone(&Local);
            cron.find_next_occurrence(&after_local, false)
                .map(|next| Some(next.with_timezone(&Utc)))
                .map_err(|error| {
                    ScheduleError::new(format!("cron expression has no next run: {error}"))
                })
        }
    }
}

/// Up to `count` consecutive fire times after `after`. Empty for a
/// manual-only schedule.
pub fn upcoming_fires(
    schedule: &ScriptSchedule,
    after: DateTime<Utc>,
    count: usize,
) -> Result<Vec<DateTime<Utc>>, ScheduleError> {
    let mut fires = Vec::with_capacity(count);
    let mut cursor = after;
    while fires.len() < count {
        match next_fire_after(schedule, cursor)? {
            Some(next) => {
                fires.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    Ok(fires)
}

/// Short English description of a schedule. Cron schedules return the raw
/// expression; the web client renders those itself.
pub fn describe_schedule(schedule: &ScriptSchedule) -> String {
    match schedule {
        ScriptSchedule::Manual => "Manual only".to_string(),
        ScriptSchedule::Interval { every_seconds } => describe_interval(*every_seconds),
        ScriptSchedule::Daily { time_local } => {
            format!("Daily at {}", display_time(time_local))
        }
        ScriptSchedule::Weekly { days, time_local } => {
            let mut days = days.clone();
            days.sort();
            days.dedup();
            let time = display_time(time_local);
            if days.len() == 7 {
                return format!("Daily at {time}");
            }
            let names = days
                .iter()
                .map(|day| weekday_abbreviation(*day))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{names} at {time}")
        }
        ScriptSchedule::Cron { expression } => expression.trim().to_string(),
    }
}

/// Parses a 24-hour `HH:MM` local time of day.
pub(crate) fn parse_local_time_of_day(value: &str) -> Result<(u32, u32), ScheduleError> {
    let (hour, minute) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| ScheduleError::new("time must use HH:MM format"))?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| ScheduleError::new("time hour must be numeric"))?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| ScheduleError::new("time minute must be numeric"))?;
    if hour > 23 || minute > 59 {
        return Err(ScheduleError::new("time must be between 00:00 and 23:59"));
    }
    Ok((hour, minute))
}

/// The local instant for `hour:minute` on `date`. A time inside a
/// daylight-saving gap slides forward to the first instant that exists; an
/// ambiguous time resolves to the earlier instant.
pub(crate) fn resolve_local_scheduled_time(
    date: NaiveDate,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    for minute_offset in 0..=DST_GAP_SEARCH_MINUTES {
        let candidate = naive + chrono::Duration::minutes(minute_offset);
        match Local.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(value) => return Some(value),
            chrono::LocalResult::Ambiguous(first, second) => {
                return Some(if first <= second { first } else { second });
            }
            chrono::LocalResult::None => continue,
        }
    }
    None
}

fn next_local_time_on_days(
    after: DateTime<Utc>,
    hour: u32,
    minute: u32,
    day_matches: impl Fn(Weekday) -> bool,
) -> Result<DateTime<Utc>, ScheduleError> {
    let mut date = after.with_timezone(&Local).date_naive();
    // Eight days covers "later today" through "same weekday next week".
    for _ in 0..=7 {
        if day_matches(date.weekday())
            && let Some(candidate) = resolve_local_scheduled_time(date, hour, minute)
        {
            let candidate = candidate.with_timezone(&Utc);
            if candidate > after {
                return Ok(candidate);
            }
        }
        date = date
            .succ_opt()
            .ok_or_else(|| ScheduleError::new("schedule date is out of range"))?;
    }
    Err(ScheduleError::new(
        "failed to resolve the scheduled local time",
    ))
}

fn parse_cron(expression: &str) -> Result<Cron, ScheduleError> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err(ScheduleError::new("cron expression is required"));
    }
    CronParser::builder()
        .seconds(Seconds::Optional)
        .year(Year::Disallowed)
        .build()
        .parse(expression)
        .map_err(|error| ScheduleError::new(format!("invalid cron expression: {error}")))
}

fn describe_interval(every_seconds: i64) -> String {
    const UNITS: [(i64, &str); 4] = [
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
        (1, "second"),
    ];
    for (unit_seconds, unit) in UNITS {
        if every_seconds > 0 && every_seconds % unit_seconds == 0 {
            let count = every_seconds / unit_seconds;
            return if count == 1 {
                format!("Every {unit}")
            } else {
                format!("Every {count} {unit}s")
            };
        }
    }
    format!("Every {every_seconds} seconds")
}

fn display_time(time_local: &str) -> String {
    match parse_local_time_of_day(time_local) {
        Ok((hour, minute)) => format!("{hour:02}:{minute:02}"),
        Err(_) => time_local.trim().to_string(),
    }
}

fn weekday_abbreviation(day: ScheduleWeekday) -> &'static str {
    match day {
        ScheduleWeekday::Monday => "Mon",
        ScheduleWeekday::Tuesday => "Tue",
        ScheduleWeekday::Wednesday => "Wed",
        ScheduleWeekday::Thursday => "Thu",
        ScheduleWeekday::Friday => "Fri",
        ScheduleWeekday::Saturday => "Sat",
        ScheduleWeekday::Sunday => "Sun",
    }
}

fn to_chrono_weekday(day: ScheduleWeekday) -> Weekday {
    match day {
        ScheduleWeekday::Monday => Weekday::Mon,
        ScheduleWeekday::Tuesday => Weekday::Tue,
        ScheduleWeekday::Wednesday => Weekday::Wed,
        ScheduleWeekday::Thursday => Weekday::Thu,
        ScheduleWeekday::Friday => Weekday::Fri,
        ScheduleWeekday::Saturday => Weekday::Sat,
        ScheduleWeekday::Sunday => Weekday::Sun,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate, Timelike};

    use super::*;

    /// A UTC instant for a host-local wall-clock time. Mid-January dates
    /// keep these clear of daylight-saving transitions.
    fn local_instant(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .expect("valid fixture date");
        Local
            .from_local_datetime(&naive)
            .single()
            .expect("unambiguous fixture local time")
            .with_timezone(&Utc)
    }

    fn daily(time: &str) -> ScriptSchedule {
        ScriptSchedule::Daily {
            time_local: time.to_string(),
        }
    }

    fn weekly(days: &[ScheduleWeekday], time: &str) -> ScriptSchedule {
        ScriptSchedule::Weekly {
            days: days.to_vec(),
            time_local: time.to_string(),
        }
    }

    fn cron(expression: &str) -> ScriptSchedule {
        ScriptSchedule::Cron {
            expression: expression.to_string(),
        }
    }

    #[test]
    fn manual_schedule_never_fires() {
        let after = local_instant(2026, 1, 14, 10, 0);
        assert_eq!(next_fire_after(&ScriptSchedule::Manual, after), Ok(None));
        assert_eq!(
            upcoming_fires(&ScriptSchedule::Manual, after, 3),
            Ok(vec![])
        );
    }

    #[test]
    fn interval_fires_after_the_interval_elapses() {
        let after = local_instant(2026, 1, 14, 10, 0);
        let schedule = ScriptSchedule::Interval { every_seconds: 900 };
        assert_eq!(
            next_fire_after(&schedule, after),
            Ok(Some(after + Duration::seconds(900)))
        );
        assert_eq!(
            upcoming_fires(&schedule, after, 3),
            Ok(vec![
                after + Duration::seconds(900),
                after + Duration::seconds(1800),
                after + Duration::seconds(2700),
            ])
        );
    }

    #[test]
    fn interval_below_floor_is_rejected() {
        let schedule = ScriptSchedule::Interval { every_seconds: 59 };
        assert!(validate_schedule(&schedule).is_err());
        assert!(next_fire_after(&schedule, Utc::now()).is_err());
        assert!(validate_schedule(&ScriptSchedule::Interval { every_seconds: 60 }).is_ok());
    }

    #[test]
    fn daily_later_today_fires_today() {
        let after = local_instant(2026, 1, 14, 1, 0);
        assert_eq!(
            next_fire_after(&daily("03:30"), after),
            Ok(Some(local_instant(2026, 1, 14, 3, 30)))
        );
    }

    #[test]
    fn daily_time_already_passed_fires_next_day() {
        let after = local_instant(2026, 1, 14, 10, 0);
        assert_eq!(
            next_fire_after(&daily("03:30"), after),
            Ok(Some(local_instant(2026, 1, 15, 3, 30)))
        );
    }

    #[test]
    fn daily_fire_is_strictly_after_the_given_instant() {
        let after = local_instant(2026, 1, 14, 3, 30);
        assert_eq!(
            next_fire_after(&daily("03:30"), after),
            Ok(Some(local_instant(2026, 1, 15, 3, 30)))
        );
    }

    #[test]
    fn daily_and_weekly_reject_malformed_times() {
        for time in ["", "3", "24:00", "12:60", "ab:cd"] {
            assert!(validate_schedule(&daily(time)).is_err(), "{time}");
            assert!(
                validate_schedule(&weekly(&[ScheduleWeekday::Monday], time)).is_err(),
                "{time}"
            );
        }
    }

    #[test]
    fn weekly_requires_a_day() {
        assert!(validate_schedule(&weekly(&[], "03:30")).is_err());
    }

    #[test]
    fn weekly_fires_on_the_next_listed_day() {
        // 2026-01-14 is a Wednesday.
        let after = local_instant(2026, 1, 14, 10, 0);
        let schedule = weekly(&[ScheduleWeekday::Monday, ScheduleWeekday::Friday], "03:30");
        assert_eq!(
            next_fire_after(&schedule, after),
            Ok(Some(local_instant(2026, 1, 16, 3, 30)))
        );
    }

    #[test]
    fn weekly_wraps_across_the_week_boundary() {
        // 2026-01-17 is a Saturday; the next Monday is 2026-01-19.
        let after = local_instant(2026, 1, 17, 10, 0);
        let schedule = weekly(&[ScheduleWeekday::Monday], "03:30");
        assert_eq!(
            next_fire_after(&schedule, after),
            Ok(Some(local_instant(2026, 1, 19, 3, 30)))
        );
    }

    #[test]
    fn weekly_same_day_after_the_time_waits_a_full_week() {
        // 2026-01-19 is a Monday.
        let after = local_instant(2026, 1, 19, 10, 0);
        let schedule = weekly(&[ScheduleWeekday::Monday], "03:30");
        assert_eq!(
            next_fire_after(&schedule, after),
            Ok(Some(local_instant(2026, 1, 26, 3, 30)))
        );
    }

    #[test]
    fn cron_rejects_invalid_expressions() {
        for expression in [
            "",
            "not a cron",
            "61 * * * *",
            "* * * *",
            "0 0 0 * * * 2026",
        ] {
            assert!(
                validate_schedule(&cron(expression)).is_err(),
                "{expression}"
            );
        }
    }

    #[test]
    fn cron_every_fifteen_minutes_fires_on_the_next_quarter_hour() {
        let after = local_instant(2026, 1, 14, 10, 7);
        let next = next_fire_after(&cron("*/15 * * * *"), after)
            .expect("valid cron")
            .expect("cron fires");
        assert_eq!(next, local_instant(2026, 1, 14, 10, 15));

        let fires = upcoming_fires(&cron("*/15 * * * *"), after, 3).expect("valid cron");
        assert_eq!(
            fires,
            vec![
                local_instant(2026, 1, 14, 10, 15),
                local_instant(2026, 1, 14, 10, 30),
                local_instant(2026, 1, 14, 10, 45),
            ]
        );
    }

    #[test]
    fn cron_weekly_expression_fires_on_monday_at_the_local_time() {
        // 2026-01-14 is a Wednesday; `30 3 * * 1` is Mondays at 03:30.
        let after = local_instant(2026, 1, 14, 10, 0);
        let next = next_fire_after(&cron("30 3 * * 1"), after)
            .expect("valid cron")
            .expect("cron fires");
        assert_eq!(next, local_instant(2026, 1, 19, 3, 30));
        let next_local = next.with_timezone(&Local);
        assert_eq!(next_local.weekday(), Weekday::Mon);
        assert_eq!((next_local.hour(), next_local.minute()), (3, 30));
    }

    #[test]
    fn cron_accepts_an_optional_seconds_field() {
        assert!(validate_schedule(&cron("0 */15 * * * *")).is_ok());
    }

    #[test]
    fn describes_non_cron_schedules_in_english() {
        assert_eq!(describe_schedule(&ScriptSchedule::Manual), "Manual only");
        assert_eq!(
            describe_schedule(&ScriptSchedule::Interval { every_seconds: 900 }),
            "Every 15 minutes"
        );
        assert_eq!(
            describe_schedule(&ScriptSchedule::Interval {
                every_seconds: 3600
            }),
            "Every hour"
        );
        assert_eq!(describe_schedule(&daily("3:30")), "Daily at 03:30");
        assert_eq!(
            describe_schedule(&weekly(
                &[
                    ScheduleWeekday::Friday,
                    ScheduleWeekday::Monday,
                    ScheduleWeekday::Wednesday,
                ],
                "03:30",
            )),
            "Mon, Wed, Fri at 03:30"
        );
        assert_eq!(describe_schedule(&cron(" 30 3 * * 1 ")), "30 3 * * 1");
    }
}
