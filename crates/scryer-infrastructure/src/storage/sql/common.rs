use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use scryer_application::{AppError, AppResult};

pub(crate) fn parse_utc_datetime(raw: &str) -> AppResult<DateTime<Utc>> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(raw) {
        return Ok(datetime.with_timezone(&Utc));
    }

    if let Ok(datetime) = DateTime::parse_from_rfc2822(raw) {
        return Ok(datetime.with_timezone(&Utc));
    }

    // Shipped migrations wrote SQLite CURRENT_TIMESTAMP values ("YYYY-MM-DD
    // HH:MM:SS", UTC, optional fraction); those rows must stay readable.
    if let Ok(datetime) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(datetime.and_utc());
    }

    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|datetime| datetime.and_utc())
        .ok_or_else(|| AppError::Repository(format!("invalid UTC datetime: {raw}")))
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    #[test]
    fn parses_rfc3339() {
        let parsed = parse_utc_datetime("2026-07-16T12:34:56+02:00").expect("rfc3339 parses");
        assert_eq!(parsed.hour(), 10);
    }

    #[test]
    fn parses_rfc2822_newznab_pub_date() {
        let parsed =
            parse_utc_datetime("Wed, 05 Aug 2026 00:52:15 +0000").expect("newznab pubDate parses");
        assert_eq!(parsed.to_rfc3339(), "2026-08-05T00:52:15+00:00");
    }

    #[test]
    fn parses_sqlite_current_timestamp_format() {
        let parsed = parse_utc_datetime("2026-07-16 12:34:56").expect("sqlite format parses");
        assert_eq!(parsed.to_rfc3339(), "2026-07-16T12:34:56+00:00");
    }

    #[test]
    fn parses_sqlite_fractional_seconds() {
        let parsed = parse_utc_datetime("2026-07-16 12:34:56.123").expect("fractional parses");
        assert_eq!(parsed.timestamp_subsec_millis(), 123);
    }

    #[test]
    fn parses_bare_date_as_midnight_utc() {
        let parsed = parse_utc_datetime("2026-07-16").expect("bare date parses");
        assert_eq!(parsed.to_rfc3339(), "2026-07-16T00:00:00+00:00");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_utc_datetime("not a timestamp").is_err());
        assert!(parse_utc_datetime("").is_err());
    }
}
