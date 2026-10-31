//! Lenient reading of the publication times indexers and plugins report.
//!
//! Feeds, trackers and plugins write dates in whatever shape their upstream
//! uses: RFC 3339, RFC 2822 with or without a weekday, bare epochs, naive
//! `YYYY-MM-DD HH:MM:SS`, US slash dates, month names, zone abbreviations.
//! Everything that orders, displays or re-serializes a publication time reads
//! it through [`parse_publish_time`], so one tolerant reading applies
//! everywhere and nothing downstream parses date text again.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};

/// Publication times before this are not believable for a release listing.
const EARLIEST_YEAR: i32 = 1970;
/// Publication times after this are not believable either; this also keeps a
/// run of digits from being mistaken for an epoch in the wrong unit.
const LATEST_YEAR: i32 = 2200;

/// Read a publication time written in any reasonable form. Times without a
/// zone are taken as UTC. Returns `None` for text that is not a date, or for
/// one outside the believable range.
pub fn parse_publish_time(value: &str) -> Option<DateTime<Utc>> {
    let cleaned = clean(value);
    if cleaned.is_empty() {
        return None;
    }
    let parsed = parse_compact(&cleaned)
        .or_else(|| parse_epoch(&cleaned))
        .or_else(|| parse_rfc3339(&cleaned))
        .or_else(|| parse_rfc2822(&cleaned))
        .or_else(|| parse_with_offset(&cleaned))
        .or_else(|| parse_with_zone_token(&cleaned))
        .or_else(|| parse_naive(&cleaned).map(|naive| Utc.from_utc_datetime(&naive)))?;
    believable(parsed)
}

/// The RFC 3339 form of a publication time when it can be read, else the
/// text as given so an unreadable value stays visible instead of vanishing.
pub fn normalize_publish_time(value: String) -> String {
    parse_publish_time(&value)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or(value)
}

fn believable(time: DateTime<Utc>) -> Option<DateTime<Utc>> {
    use chrono::Datelike;
    (EARLIEST_YEAR..=LATEST_YEAR)
        .contains(&time.year())
        .then_some(time)
}

/// Trim, drop a trailing parenthetical comment such as `(UTC)`, collapse
/// whitespace, drop a leading weekday, ordinal suffixes, commas and the word
/// `at`, and uppercase a lowercase `t` separator or `z` designator.
fn clean(value: &str) -> String {
    let mut text = value.trim();
    if let Some(open) = text.rfind('(')
        && text.ends_with(')')
    {
        text = text[..open].trim_end();
    }
    let mut tokens: Vec<String> = text
        .split_whitespace()
        .map(|token| token.trim_matches(',').to_owned())
        .filter(|token| !token.is_empty() && !token.eq_ignore_ascii_case("at"))
        .collect();
    if tokens.first().is_some_and(|token| is_weekday(token)) {
        tokens.remove(0);
    }
    for token in &mut tokens {
        strip_ordinal(token);
        if token.len() >= 11
            && let Some(tee) = token.find(['t', 'T'])
            && token[..tee].chars().all(|c| c.is_ascii_digit() || c == '-')
        {
            token.replace_range(tee..=tee, "T");
        }
        if token.ends_with('z') {
            token.replace_range(token.len() - 1.., "Z");
        }
    }
    tokens.join(" ")
}

fn is_weekday(token: &str) -> bool {
    const WEEKDAYS: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let lower = token.to_ascii_lowercase();
    let lower = lower.trim_end_matches('.');
    WEEKDAYS
        .iter()
        .any(|day| *day == lower || day.get(..3) == Some(lower))
}

/// `1st`, `2nd`, `3rd`, `4th` → `1`, `2`, `3`, `4`.
fn strip_ordinal(token: &mut String) {
    let lower = token.to_ascii_lowercase();
    for suffix in ["st", "nd", "rd", "th"] {
        if let Some(digits) = lower.strip_suffix(suffix)
            && !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
        {
            token.truncate(digits.len());
            return;
        }
    }
}

/// `YYYYMMDD`, `YYYYMMDDHHMMSS` and `YYYYMMDDTHHMMSS[Z]`.
fn parse_compact(text: &str) -> Option<DateTime<Utc>> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let shape = text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, 'T' | 'Z'));
    if !shape {
        return None;
    }
    let naive = match digits.len() {
        8 => NaiveDate::parse_from_str(&digits, "%Y%m%d")
            .ok()?
            .and_hms_opt(0, 0, 0)?,
        12 => NaiveDateTime::parse_from_str(&digits, "%Y%m%d%H%M").ok()?,
        14 => NaiveDateTime::parse_from_str(&digits, "%Y%m%d%H%M%S").ok()?,
        _ => return None,
    };
    Some(Utc.from_utc_datetime(&naive))
}

/// Seconds, milliseconds, microseconds or nanoseconds since the Unix epoch,
/// chosen by magnitude; a fractional seconds value is read as seconds.
fn parse_epoch(text: &str) -> Option<DateTime<Utc>> {
    let body = text.strip_prefix('-').unwrap_or(text);
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if body.contains('.') {
        let seconds: f64 = text.parse().ok()?;
        if !seconds.is_finite() {
            return None;
        }
        if seconds.abs() < 100_000_000.0 {
            return None;
        }
        let whole = seconds.trunc() as i64;
        let nanos = ((seconds - seconds.trunc()).abs() * 1e9) as u32;
        return DateTime::from_timestamp(whole, nanos);
    }
    let value: i64 = text.parse().ok()?;
    let magnitude = value.unsigned_abs();
    // Anything under nine digits is a count, an id or a typo, not a time.
    if magnitude < 100_000_000 {
        None
    } else if magnitude < 100_000_000_000 {
        DateTime::from_timestamp(value, 0)
    } else if magnitude < 100_000_000_000_000 {
        DateTime::from_timestamp_millis(value)
    } else if magnitude < 100_000_000_000_000_000 {
        DateTime::from_timestamp_micros(value)
    } else {
        Some(DateTime::from_timestamp_nanos(value))
    }
}

fn parse_rfc3339(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn parse_rfc2822(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// Date-time formats carrying a numeric offset.
const OFFSET_FORMATS: [&str; 10] = [
    "%Y-%m-%dT%H:%M:%S%.f%z",
    "%Y-%m-%d %H:%M:%S%.f%z",
    "%Y-%m-%d %H:%M:%S%.f %z",
    "%Y-%m-%dT%H:%M%z",
    "%Y-%m-%d %H:%M%z",
    "%Y-%m-%d %H:%M %z",
    "%Y/%m/%d %H:%M:%S%.f %z",
    "%d %b %Y %H:%M:%S %z",
    "%d %b %Y %H:%M %z",
    "%b %d %Y %H:%M:%S %z",
];

fn parse_with_offset(text: &str) -> Option<DateTime<Utc>> {
    OFFSET_FORMATS.iter().find_map(|format| {
        DateTime::parse_from_str(text, format)
            .ok()
            .map(|time| time.with_timezone(&Utc))
    })
}

/// Date-time and date formats without any zone; read as UTC.
const NAIVE_DATETIME_FORMATS: [&str; 30] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
    "%Y/%m/%d %H:%M:%S%.f",
    "%Y/%m/%d %H:%M",
    "%Y.%m.%d %H:%M:%S",
    "%Y.%m.%d %H:%M",
    "%d %b %Y %H:%M:%S",
    "%d %b %Y %H:%M",
    "%d %B %Y %H:%M:%S",
    "%d %B %Y %H:%M",
    "%d-%b-%Y %H:%M:%S",
    "%b %d %Y %H:%M:%S",
    "%b %d %Y %H:%M",
    "%B %d %Y %H:%M:%S",
    "%B %d %Y %H:%M",
    "%b %d %H:%M:%S %Y",
    "%B %d %H:%M:%S %Y",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y %H:%M",
    "%m/%d/%Y %I:%M:%S %p",
    "%m/%d/%Y %I:%M %p",
    "%d/%m/%Y %H:%M:%S",
    "%d/%m/%Y %H:%M",
    "%d.%m.%Y %H:%M:%S",
    "%d.%m.%Y %H:%M",
    "%Y-%m-%d %I:%M:%S %p",
    "%Y-%m-%d %I:%M %p",
    "%b %d %Y %I:%M %p",
];

const NAIVE_DATE_FORMATS: [&str; 14] = [
    "%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d", "%d %b %Y", "%d %B %Y", "%d-%b-%Y", "%b %d %Y", "%B %d %Y",
    "%m/%d/%Y", "%d/%m/%Y", "%d.%m.%Y", "%m-%d-%Y", "%d-%m-%Y", "%m/%d/%y",
];

fn parse_naive(text: &str) -> Option<NaiveDateTime> {
    NAIVE_DATETIME_FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .or_else(|| {
            NAIVE_DATE_FORMATS
                .iter()
                .find_map(|format| NaiveDate::parse_from_str(text, format).ok())
                .and_then(|date| date.and_hms_opt(0, 0, 0))
        })
}

/// A single zone token anywhere in the text: an abbreviation such as `UTC`,
/// `PDT` or `CEST`, or an offset such as `+0200`, `+02:00`, `UTC+2` or
/// `GMT-05:00`. The remainder is read as a naive time in that zone.
fn parse_with_zone_token(text: &str) -> Option<DateTime<Utc>> {
    let tokens: Vec<&str> = text.split(' ').collect();
    let (index, offset) = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| zone_offset_seconds(token).map(|offset| (index, offset)))
        .next()?;
    let remainder = tokens
        .iter()
        .enumerate()
        .filter(|(position, _)| *position != index)
        .map(|(_, token)| *token)
        .collect::<Vec<_>>()
        .join(" ");
    let naive = parse_naive(&remainder)?;
    let zone = FixedOffset::east_opt(offset)?;
    zone.from_local_datetime(&naive)
        .single()
        .map(|time| time.with_timezone(&Utc))
}

fn zone_offset_seconds(token: &str) -> Option<i32> {
    let upper = token.to_ascii_uppercase();
    let (base, rest) = ["UTC", "GMT", "UT", "Z"]
        .iter()
        .find_map(|prefix| upper.strip_prefix(prefix).map(|rest| (*prefix, rest)))
        .unwrap_or(("", upper.as_str()));
    if !base.is_empty() && rest.is_empty() {
        return Some(0);
    }
    if let Some(offset) = numeric_offset_seconds(rest) {
        return (base.is_empty() || !rest.is_empty()).then_some(offset);
    }
    if base.is_empty() {
        return zone_abbreviation_seconds(&upper);
    }
    None
}

/// `+HH`, `+HHMM` or `+HH:MM`.
fn numeric_offset_seconds(text: &str) -> Option<i32> {
    let (sign, digits) = match text.chars().next()? {
        '+' => (1, &text[1..]),
        '-' => (-1, &text[1..]),
        _ => return None,
    };
    let digits = digits.replace(':', "");
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = if digits.len() <= 2 {
        (digits.parse::<i32>().ok()?, 0)
    } else {
        let (hours, minutes) = digits.split_at(digits.len() - 2);
        (hours.parse::<i32>().ok()?, minutes.parse::<i32>().ok()?)
    };
    (hours <= 14 && minutes < 60).then_some(sign * (hours * 3600 + minutes * 60))
}

/// Common abbreviations. An unknown all-letter token of two to five
/// characters that is not a month or weekday counts as an unknown zone and is
/// read as UTC, the way RFC 2822 treats zones it does not know.
fn zone_abbreviation_seconds(upper: &str) -> Option<i32> {
    const KNOWN: [(&str, i32); 30] = [
        ("UTC", 0),
        ("UCT", 0),
        ("GMT", 0),
        ("UT", 0),
        ("Z", 0),
        ("WET", 0),
        ("WEST", 1),
        ("BST", 1),
        ("CET", 1),
        ("CEST", 2),
        ("EET", 2),
        ("EEST", 3),
        ("MSK", 3),
        ("IST", 5),
        ("JST", 9),
        ("KST", 9),
        ("AEST", 10),
        ("AEDT", 11),
        ("NZST", 12),
        ("NZDT", 13),
        ("AST", -4),
        ("ADT", -3),
        ("EST", -5),
        ("EDT", -4),
        ("CST", -6),
        ("CDT", -5),
        ("MST", -7),
        ("MDT", -6),
        ("PST", -8),
        ("PDT", -7),
    ];
    if let Some((_, hours)) = KNOWN.iter().find(|(name, _)| *name == upper) {
        // India runs on a half-hour offset.
        return Some(if upper == "IST" {
            5 * 3600 + 1800
        } else {
            hours * 3600
        });
    }
    let looks_like_zone = (2..=5).contains(&upper.len())
        && upper.chars().all(|c| c.is_ascii_uppercase())
        && !is_month_or_weekday(upper);
    looks_like_zone.then_some(0)
}

fn is_month_or_weekday(upper: &str) -> bool {
    const NAMES: [&str; 19] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "SEPT", "OCT", "NOV", "DEC",
        "MON", "TUE", "WED", "THU", "FRI", "SAT",
    ];
    NAMES.contains(&upper) || upper == "SUN" || upper == "AM" || upper == "PM"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> String {
        parse_publish_time(text)
            .unwrap_or_else(|| panic!("{text:?} should parse"))
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    #[test]
    fn rfc3339_and_its_relaxed_spellings() {
        assert_eq!(at("2026-05-01T15:11:56+00:00"), "2026-05-01T15:11:56Z");
        assert_eq!(at("2026-05-01T15:11:56.250Z"), "2026-05-01T15:11:56Z");
        assert_eq!(at("2026-05-01t15:11:56z"), "2026-05-01T15:11:56Z");
        assert_eq!(at("2026-05-01 15:11:56+02:00"), "2026-05-01T13:11:56Z");
        assert_eq!(at("2026-05-01 15:11:56 -0700"), "2026-05-01T22:11:56Z");
        assert_eq!(at("2026-05-01T15:11:56"), "2026-05-01T15:11:56Z");
        assert_eq!(at("2026-05-01 15:11"), "2026-05-01T15:11:00Z");
        assert_eq!(at("2026-05-01"), "2026-05-01T00:00:00Z");
    }

    #[test]
    fn rfc2822_with_and_without_weekday_zone_or_comment() {
        assert_eq!(
            at("Fri, 01 May 2026 15:11:56 +0000"),
            "2026-05-01T15:11:56Z"
        );
        assert_eq!(at("01 May 2026 15:11:56 GMT"), "2026-05-01T15:11:56Z");
        assert_eq!(
            at("Fri, 01 May 2026 15:11:56 -0600 (MDT)"),
            "2026-05-01T21:11:56Z"
        );
        assert_eq!(at("Fri, 1 May 2026 15:11:56 PDT"), "2026-05-01T22:11:56Z");
        assert_eq!(at("1 May 2026 15:11:56 CEST"), "2026-05-01T13:11:56Z");
        assert_eq!(
            at("Friday, May 1st, 2026 at 3:11 PM"),
            "2026-05-01T15:11:00Z"
        );
    }

    #[test]
    fn epochs_in_each_unit_and_compact_digits() {
        assert_eq!(at("1777648316"), "2026-05-01T15:11:56Z");
        assert_eq!(at("1777648316.5"), "2026-05-01T15:11:56Z");
        assert_eq!(at("1777648316250"), "2026-05-01T15:11:56Z");
        assert_eq!(at("1777648316250000"), "2026-05-01T15:11:56Z");
        assert_eq!(at("1777648316250000000"), "2026-05-01T15:11:56Z");
        assert_eq!(at("20260501"), "2026-05-01T00:00:00Z");
        assert_eq!(at("20260501151156"), "2026-05-01T15:11:56Z");
        assert_eq!(at("20260501T151156Z"), "2026-05-01T15:11:56Z");
    }

    #[test]
    fn month_names_slashes_dots_and_unix_date_output() {
        assert_eq!(at("May 1, 2026 15:11:56"), "2026-05-01T15:11:56Z");
        assert_eq!(at("May 1 2026"), "2026-05-01T00:00:00Z");
        assert_eq!(at("1 May 2026"), "2026-05-01T00:00:00Z");
        assert_eq!(at("01-May-2026 15:11:56"), "2026-05-01T15:11:56Z");
        assert_eq!(at("Fri May 1 15:11:56 2026"), "2026-05-01T15:11:56Z");
        assert_eq!(at("Fri May 1 15:11:56 UTC 2026"), "2026-05-01T15:11:56Z");
        assert_eq!(at("Fri May 1 15:11:56 MDT 2026"), "2026-05-01T21:11:56Z");
        assert_eq!(at("05/01/2026 15:11:56"), "2026-05-01T15:11:56Z");
        assert_eq!(at("05/01/2026 3:11 PM"), "2026-05-01T15:11:00Z");
        assert_eq!(at("25/05/2026"), "2026-05-25T00:00:00Z");
        assert_eq!(at("01.05.2026 15:11"), "2026-05-01T15:11:00Z");
        assert_eq!(at("2026/05/01 15:11:56"), "2026-05-01T15:11:56Z");
        assert_eq!(at("2026-05-01 15:11:56 UTC+2"), "2026-05-01T13:11:56Z");
        assert_eq!(at("2026-05-01 15:11:56 GMT-05:00"), "2026-05-01T20:11:56Z");
    }

    #[test]
    fn nonsense_and_unbelievable_years_are_refused() {
        for text in [
            "",
            "   ",
            "soon",
            "n/a",
            "2026-13-45",
            "1",
            "0001-01-01T00:00:00Z",
        ] {
            assert!(
                parse_publish_time(text).is_none(),
                "{text:?} should not parse"
            );
        }
    }

    #[test]
    fn normalization_rewrites_readable_values_and_keeps_the_rest() {
        assert_eq!(
            normalize_publish_time("Fri, 01 May 2026 15:11:56 +0000".into()),
            "2026-05-01T15:11:56Z"
        );
        assert_eq!(normalize_publish_time("whenever".into()), "whenever");
    }
}
