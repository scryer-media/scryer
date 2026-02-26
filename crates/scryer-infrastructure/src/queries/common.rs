use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};

pub(crate) fn parse_utc_datetime(raw: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| AppError::Repository(err.to_string()))
}

pub(crate) fn parse_optional_utc_datetime(raw: Option<String>) -> AppResult<Option<DateTime<Utc>>> {
    match raw {
        Some(raw) if !raw.trim().is_empty() => Ok(Some(parse_utc_datetime(&raw)?)),
        Some(_) | None => Ok(None),
    }
}
