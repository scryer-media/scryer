//! Five-field crontab expressions: parsing and next-occurrence search.
//!
//! The grammar and the search reproduce the `croner` crate, configured with
//! seconds and years disallowed, rule for rule. Saved schedules were written
//! against those rules, so every quirk is kept on purpose; the differential
//! test at the bottom of this file runs both implementations side by side.
//!
//! Grammar, per whitespace-separated field (minute, hour, day of month,
//! month, day of week):
//! - `*`, a number, a range `a-b`, a stepped range `a-b/n` or `*/n`, and
//!   comma-separated lists of those. A bare `a/n` or `/n` is refused.
//! - Month and weekday names (`JAN`..`DEC`, `SUN`..`SAT`), in any case.
//!   Weekdays count from Sunday as 0; 7 is also Sunday.
//! - `?` in the day fields reads as `*`.
//! - Day of month: `L` (last day), `LW` (last weekday), `nW` (weekday
//!   closest to day n, never leaving the month).
//! - Day of week: `d#n` (nth such weekday of the month), `dL` or `d#L` (last
//!   such weekday), and a leading `+` to require both day fields to match.
//! - `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily`, `@hourly`.
//!
//! When both day fields are restricted a day matches if either does, unless
//! the day-of-week field starts with `+`. A day field written as exactly `*`
//! is ignored.
//!
//! The search walks the wall clock of the given time zone. An expression
//! whose minute and hour are each one plain value is a fixed-time job: it
//! fires once, at the earlier instant, on a repeated wall-clock time, and at
//! the first real instant after a gap that swallows it. Any other expression
//! fires on both instants of a repeated time and skips times inside a gap.

// The parser and the search are ported from croner 4.0.3
// (https://github.com/hexagon/croner-rust), which is distributed under the
// following licence:
//
// The MIT License (MIT)
//
// Copyright (c) 2023 Hexagon <github.com/Hexagon>
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::cmp::Ordering;

use chrono::{
    DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone,
    Timelike,
};

/// A value matches on its own.
const ALL_BIT: u8 = 1;
/// A weekday matches on its first to fifth occurrence in the month.
const NTH_1ST_BIT: u8 = 1 << 1;
const NTH_2ND_BIT: u8 = 1 << 2;
const NTH_3RD_BIT: u8 = 1 << 3;
const NTH_4TH_BIT: u8 = 1 << 4;
const NTH_5TH_BIT: u8 = 1 << 5;
const NTH_ALL: u8 = NTH_1ST_BIT | NTH_2ND_BIT | NTH_3RD_BIT | NTH_4TH_BIT | NTH_5TH_BIT;
/// Last day of the month, or the last occurrence of a weekday.
const LAST_BIT: u8 = 1 << 6;
/// The weekday closest to a day of the month.
const CLOSEST_WEEKDAY_BIT: u8 = 1 << 7;

/// The search gives up once it would step past this year.
const YEAR_UPPER_LIMIT: i32 = 5000;
const YEAR_LOWER_LIMIT: i32 = 1;

/// Upper bound on search passes for one occurrence.
const MAX_SEARCH_ITERATIONS: u32 = 366 * 24 * 60 * 60;
/// How far past a nonexistent wall-clock time a fixed-time job looks for the
/// first real instant.
const MAX_GAP_SEARCH_SECONDS: u32 = 3600 * 2;
/// The widest stretch of repeated wall-clock time the search allows for.
const MAX_OVERLAP_SECONDS: i64 = 86_400;

/// Why an expression was refused or has no next occurrence. The sentences
/// are shown to the operator as they are.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CronError {
    #[error("CronPattern cannot be an empty string.")]
    EmptyPattern,
    #[error("CronScheduler encountered an invalid date.")]
    InvalidDate,
    #[error("CronScheduler encountered an invalid time.")]
    InvalidTime,
    #[error("CronScheduler time search limit exceeded.")]
    TimeSearchLimitExceeded,
    #[error("Invalid pattern: {0}")]
    InvalidPattern(String),
    #[error("Pattern contains illegal characters: {0}")]
    IllegalCharacters(String),
    #[error("Component error: {0}")]
    Component(String),
}

fn component_error(message: impl Into<String>) -> CronError {
    CronError::Component(message.into())
}

/// One field of an expression: a set of flag bits per value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    /// Flag bits per value, indexed by the value itself.
    bits: Vec<u8>,
    min: u16,
    max: u16,
    /// The step of the last stepped part, 1 when there is none.
    step: u16,
    /// Whether any part was `*` or `*/n`.
    from_wildcard: bool,
    /// Flag bits this field accepts.
    features: u8,
    /// Field-wide flags (`L`, `LW`) that belong to no single value.
    enabled_features: u8,
}

impl Field {
    fn new(min: u16, max: u16, features: u8) -> Self {
        Self {
            bits: vec![0; usize::from(max) + 1],
            min,
            max,
            step: 1,
            from_wildcard: false,
            features: features | ALL_BIT | LAST_BIT,
            enabled_features: 0,
        }
    }

    fn parsed(min: u16, max: u16, features: u8, text: &str) -> Result<Self, CronError> {
        let mut field = Self::new(min, max, features);
        field.parse(text)?;
        Ok(field)
    }

    fn check_position(&self, pos: u16, bit: u8) -> Result<(), CronError> {
        if pos < self.min || pos > self.max {
            return Err(component_error(format!(
                "Position {} is out of bounds for the current range ({}-{}).",
                pos, self.min, self.max
            )));
        }
        if self.features & bit != bit {
            return Err(component_error(format!(
                "Bit 0b{:08b} is not supported by the current features 0b{:08b}.",
                bit, self.features
            )));
        }
        Ok(())
    }

    fn set_bit(&mut self, pos: u16, bit: u8) -> Result<(), CronError> {
        self.check_position(pos, bit)?;
        self.bits[usize::from(pos)] |= bit;
        Ok(())
    }

    fn unset_bit(&mut self, pos: u16, bit: u8) -> Result<(), CronError> {
        self.check_position(pos, bit)?;
        self.bits[usize::from(pos)] &= !bit;
        Ok(())
    }

    /// Whether `bit` is set for `pos`. A position outside the field has no
    /// bits.
    fn has(&self, pos: u32, bit: u8) -> bool {
        usize::try_from(pos)
            .ok()
            .and_then(|index| self.bits.get(index))
            .is_some_and(|bits| bits & bit != 0)
    }

    fn enable_feature(&mut self, feature: u8) -> Result<(), CronError> {
        if self.features & feature != feature {
            return Err(component_error(format!(
                "Feature 0b{:08b} is not supported by the current features 0b{:08b}.",
                feature, self.features
            )));
        }
        self.enabled_features |= feature;
        Ok(())
    }

    fn is_feature_enabled(&self, feature: u8) -> bool {
        self.enabled_features & feature == feature
    }

    fn allows(&self, feature: u8) -> bool {
        self.features & feature == feature
    }

    /// Whether the field is one plain value: no wildcard, no step, no list.
    fn is_fixed(&self) -> bool {
        self.step == 1
            && !self.from_wildcard
            && (u32::from(self.min)..=u32::from(self.max))
                .filter(|value| self.has(*value, ALL_BIT))
                .count()
                == 1
    }

    /// The first plainly matching value at or after `value`.
    fn next_match(&self, value: u32) -> Option<u32> {
        (value..=u32::from(self.max)).find(|candidate| self.has(*candidate, ALL_BIT))
    }

    fn parse(&mut self, field: &str) -> Result<(), CronError> {
        if field == "*" {
            self.from_wildcard = true;
            for value in self.min..=self.max {
                self.set_bit(value, ALL_BIT)?;
            }
            return Ok(());
        }

        for part in field.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if part.contains('/') {
                self.parse_stepped(part)?;
            } else if part.contains('-') {
                self.parse_range(part)?;
            } else if part.contains('W') {
                self.parse_closest_weekday(part)?;
            } else if part == "L" {
                self.enable_feature(LAST_BIT)?;
            } else {
                // "5L" is shorthand for "5#L".
                let part = if part.ends_with('L') && !part.contains('#') {
                    part.replace('L', "#L")
                } else {
                    part.to_string()
                };
                if part.contains('#') && !self.allows(NTH_ALL) {
                    return Err(component_error(
                        "Nth specifier # not allowed in the current field.",
                    ));
                }
                if part.contains('L') && !self.allows(NTH_ALL) {
                    return Err(component_error("L not allowed in the current field."));
                }
                self.parse_number(&part)?;
            }
        }
        Ok(())
    }

    /// The flag a part sets: the last or nth occurrence when it carries an
    /// `L` or `#n` suffix, a plain match otherwise.
    fn occurrence_bit(part: &str) -> Result<u8, CronError> {
        if part.ends_with('L') {
            return Ok(LAST_BIT);
        }
        let Some(position) = part.find('#') else {
            return Ok(ALL_BIT);
        };
        let nth = part[position + 1..]
            .parse::<u8>()
            .map_err(|_| component_error("Invalid nth specifier."))?;
        match nth {
            1 => Ok(NTH_1ST_BIT),
            2 => Ok(NTH_2ND_BIT),
            3 => Ok(NTH_3RD_BIT),
            4 => Ok(NTH_4TH_BIT),
            5 => Ok(NTH_5TH_BIT),
            _ => Err(component_error("Nth specifier out of bounds.")),
        }
    }

    /// The part without its `#n` suffix.
    fn without_occurrence(part: &str) -> &str {
        part.split('#').next().unwrap_or("")
    }

    fn parse_closest_weekday(&mut self, part: &str) -> Result<(), CronError> {
        let Some(position) = part.find('W') else {
            return self.parse_number(part);
        };
        let day = &part[..position];
        if day == "L" {
            self.enable_feature(LAST_BIT)?;
            self.enable_feature(CLOSEST_WEEKDAY_BIT)?;
            return Ok(());
        }
        let day = day
            .parse::<u16>()
            .map_err(|_| component_error("Invalid day for closest weekday."))?;
        if day < self.min || day > self.max {
            return Err(component_error("Day for closest weekday out of bounds."));
        }
        self.set_bit(day, CLOSEST_WEEKDAY_BIT)
    }

    fn parse_range(&mut self, part: &str) -> Result<(), CronError> {
        let bit = Self::occurrence_bit(part)?;
        let bounds: Vec<&str> = Self::without_occurrence(part)
            .split('-')
            .map(str::trim)
            .collect();
        let [start, end] = bounds[..] else {
            return Err(component_error("Invalid range syntax."));
        };
        let start = start
            .parse::<u16>()
            .map_err(|_| component_error("Invalid start of range."))?;
        let end = end
            .parse::<u16>()
            .map_err(|_| component_error("Invalid end of range."))?;
        if start > end || start < self.min || end > self.max {
            return Err(component_error("Range out of bounds."));
        }
        for value in start..=end {
            self.set_bit(value, bit)?;
        }
        Ok(())
    }

    fn parse_number(&mut self, part: &str) -> Result<(), CronError> {
        let bit = Self::occurrence_bit(part)?;
        let value = Self::without_occurrence(part)
            .parse::<u16>()
            .map_err(|_| component_error("Invalid number."))?;
        if value < self.min || value > self.max {
            return Err(component_error("Number out of bounds."));
        }
        self.set_bit(value, bit)
    }

    fn parse_stepped(&mut self, part: &str) -> Result<(), CronError> {
        let bit = Self::occurrence_bit(part)?;
        let pieces: Vec<&str> = Self::without_occurrence(part).split('/').collect();
        let [range, step_text] = pieces[..] else {
            return Err(component_error("Invalid stepped range syntax."));
        };
        let step = step_text
            .parse::<u16>()
            .map_err(|_| component_error("Invalid step."))?;
        self.step = step;
        if step == 0 {
            return Err(component_error("Step cannot be zero."));
        }

        let (start, end) = if range == "*" {
            self.from_wildcard = true;
            (self.min, self.max)
        } else if range.contains('-') {
            let bounds: Vec<&str> = range.split('-').collect();
            let [start, end] = bounds[..] else {
                return Err(component_error("Invalid range syntax in stepping."));
            };
            (
                start
                    .parse::<u16>()
                    .map_err(|_| component_error("Invalid range start."))?,
                end.parse::<u16>()
                    .map_err(|_| component_error("Invalid range end."))?,
            )
        } else if range.is_empty() {
            return Err(component_error(format!(
                "Invalid step syntax: omitting the starting point like \"/{step_text}\" is not allowed. Use \"*/{step_text}\" for wildcard steps or \"X-Y/{step_text}\" for range steps."
            )));
        } else {
            return Err(component_error(format!(
                "Invalid step syntax: single number steps like \"{range}/{step_text}\" are not allowed. Use \"*/{step_text}\" for wildcard steps or \"X-Y/{step_text}\" for range steps."
            )));
        };

        if start < self.min || end > self.max || start > end {
            return Err(component_error("Range is out of bounds in stepping."));
        }
        let mut value = start;
        while value <= end {
            self.set_bit(value, bit)?;
            value = value
                .checked_add(step)
                .ok_or_else(|| component_error("Value exceeded max after stepping."))?;
        }
        Ok(())
    }
}

/// The unit a search step moves by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

/// A parsed five-field cron expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CronExpression {
    seconds: Field,
    minutes: Field,
    hours: Field,
    days: Field,
    months: Field,
    days_of_week: Field,
    /// The day-of-month field is exactly `*`.
    star_dom: bool,
    /// The day-of-week field is exactly `*`.
    star_dow: bool,
    /// Both day fields must match, from a `+` before the day-of-week field.
    dom_and_dow: bool,
}

impl CronExpression {
    /// Parses a five-field expression or an `@nickname`.
    pub(crate) fn parse(expression: &str) -> Result<Self, CronError> {
        let mut pattern = expression.to_uppercase().trim().to_string();
        if pattern.is_empty() {
            return Err(CronError::EmptyPattern);
        }
        if pattern.contains('@') {
            pattern = expand_nickname(&pattern).to_string();
        }
        // Names are replaced across the whole expression, not per field.
        for (name, value) in WEEKDAY_NAMES.iter().chain(MONTH_NAMES.iter()) {
            pattern = pattern.replace(name, value);
        }

        let parts: Vec<&str> = pattern.split_whitespace().collect();
        let [minute, hour, day, month, day_of_week] = parts[..] else {
            return Err(CronError::InvalidPattern(
                "Pattern must have 5 fields when seconds are disallowed.".to_string(),
            ));
        };
        let day = day.replace('?', "*");
        let mut day_of_week = day_of_week.replace('?', "*");
        let dom_and_dow = day_of_week.starts_with('+');
        if dom_and_dow {
            day_of_week.remove(0);
        }

        reject_illegal_characters(minute, &[])?;
        reject_illegal_characters(hour, &[])?;
        reject_illegal_characters(&day, &['L', 'W', '?'])?;
        reject_illegal_characters(month, &[])?;
        reject_illegal_characters(&day_of_week, &['#', 'L', '?'])?;

        let seconds = Field::parsed(0, 59, 0, "0")?;
        let minutes = Field::parsed(0, 59, 0, minute)?;
        let hours = Field::parsed(0, 23, 0, hour)?;
        let days = Field::parsed(1, 31, LAST_BIT | CLOSEST_WEEKDAY_BIT, &day)?;
        let months = Field::parsed(1, 12, 0, month)?;
        let mut days_of_week = Field::parsed(0, 7, LAST_BIT | NTH_ALL, &day_of_week)?;

        // 7 is Sunday too; fold it onto 0.
        for bit in [
            ALL_BIT,
            NTH_1ST_BIT,
            NTH_2ND_BIT,
            NTH_3RD_BIT,
            NTH_4TH_BIT,
            NTH_5TH_BIT,
            LAST_BIT,
        ] {
            if days_of_week.has(7, bit) {
                days_of_week.unset_bit(7, bit)?;
                days_of_week.set_bit(0, bit)?;
            }
        }

        Ok(Self {
            seconds,
            minutes,
            hours,
            star_dom: day == "*",
            star_dow: day_of_week == "*",
            days,
            months,
            days_of_week,
            dom_and_dow,
        })
    }

    /// The first occurrence strictly after `after`, on the wall clock of its
    /// time zone.
    pub(crate) fn next_after<Tz: TimeZone>(
        &self,
        after: &DateTime<Tz>,
    ) -> Result<DateTime<Tz>, CronError> {
        let fixed_time = self.is_fixed_time();
        let in_fold = self.walk(after, false, fixed_time)?;
        if fixed_time {
            return Ok(in_fold);
        }
        // A wall-clock walk covers one instance of a repeated time. When
        // `after` sits on the earlier instance, the later one still lies
        // ahead and is walked from its first instant.
        if !on_earlier_fold(after) {
            return Ok(in_fold);
        }
        let later_fold_start = later_fold_start(after)?;
        if in_fold < later_fold_start {
            return Ok(in_fold);
        }
        let in_later_fold = self.walk(&later_fold_start, true, fixed_time)?;
        Ok(if in_later_fold < in_fold {
            in_later_fold
        } else {
            in_fold
        })
    }

    /// Whether second, minute and hour are each one plain value.
    fn is_fixed_time(&self) -> bool {
        self.seconds.is_fixed() && self.minutes.is_fixed() && self.hours.is_fixed()
    }

    fn year_matches(year: i32) -> bool {
        (YEAR_LOWER_LIMIT..=YEAR_UPPER_LIMIT).contains(&year)
    }

    fn month_matches(&self, month: u32) -> bool {
        self.months.has(month, ALL_BIT)
    }

    fn day_matches(&self, date: NaiveDate) -> bool {
        let day = date.day();
        let month_length = days_in_month(date.year(), date.month());
        let weekday = date.weekday().num_days_from_sunday();

        let mut dom_matches = self.days.has(day, ALL_BIT);
        let last = self.days.is_feature_enabled(LAST_BIT);
        let closest_weekday = self.days.is_feature_enabled(CLOSEST_WEEKDAY_BIT);
        if !dom_matches && last {
            dom_matches = if closest_weekday {
                // "LW": the last Monday-to-Friday day of the month.
                let last_weekday = match shift_weekday(weekday, month_length - day) {
                    6 => month_length - 1,
                    0 => month_length - 2,
                    _ => month_length,
                };
                day == last_weekday
            } else {
                day == month_length
            };
        }
        if !dom_matches {
            dom_matches = self.closest_weekday_matches(month_length, day, weekday);
        }

        // Days 1-7 hold the first occurrence of every weekday, 8-14 the
        // second, and so on.
        let nth_bit = match (day - 1) / 7 {
            0 => NTH_1ST_BIT,
            1 => NTH_2ND_BIT,
            2 => NTH_3RD_BIT,
            3 => NTH_4TH_BIT,
            _ => NTH_5TH_BIT,
        };
        let dow_matches = self.days_of_week.has(weekday, nth_bit)
            // The last occurrence has no room for another a week later.
            || (self.days_of_week.has(weekday, LAST_BIT) && day + 7 > month_length)
            || self.days_of_week.has(weekday, ALL_BIT);

        if (dom_matches && self.star_dow) || (dow_matches && self.star_dom) {
            true
        } else if !self.star_dom && !self.star_dow {
            if self.dom_and_dow {
                dom_matches && dow_matches
            } else {
                dom_matches || dow_matches
            }
        } else {
            false
        }
    }

    /// Whether an `nW` day resolves to `day`, whose weekday (from Sunday) is
    /// `weekday`.
    fn closest_weekday_matches(&self, month_length: u32, day: u32, weekday: u32) -> bool {
        // The rule never moves a date by more than two days.
        let first = day.saturating_sub(2).max(1);
        let last = (day + 2).min(month_length);
        (first..=last).any(|pattern_day| {
            if !self.days.has(pattern_day, CLOSEST_WEEKDAY_BIT) {
                return false;
            }
            let pattern_weekday =
                (i64::from(weekday) + i64::from(pattern_day) - i64::from(day)).rem_euclid(7);
            let target = match pattern_weekday {
                // Saturday moves back to Friday, or on to Monday when Friday
                // is in the previous month.
                6 if pattern_day > 1 => pattern_day - 1,
                6 => pattern_day + 2,
                // Sunday moves on to Monday, or back to Friday when Monday
                // is in the next month.
                0 if pattern_day < month_length => pattern_day + 1,
                0 => pattern_day - 2,
                _ => pattern_day,
            };
            target == day
        })
    }

    fn matches(&self, cursor: NaiveDateTime) -> bool {
        self.seconds.has(cursor.second(), ALL_BIT)
            && self.minutes.has(cursor.minute(), ALL_BIT)
            && self.hours.has(cursor.hour(), ALL_BIT)
            && self.day_matches(cursor.date())
            && self.month_matches(cursor.month())
            && Self::year_matches(cursor.year())
    }

    /// Walks the wall clock forward from `origin` to the first matching
    /// time. An exclusive walk first steps off `origin`'s own second.
    fn walk<Tz: TimeZone>(
        &self,
        origin: &DateTime<Tz>,
        inclusive: bool,
        fixed_time: bool,
    ) -> Result<DateTime<Tz>, CronError> {
        let tz = origin.timezone();
        let mut cursor = wall_clock(origin);
        if !inclusive {
            cursor = add_seconds(cursor, 1)?;
        }

        let mut iterations = 0;
        loop {
            iterations += 1;
            if iterations > MAX_SEARCH_ITERATIONS {
                return Err(CronError::TimeSearchLimitExceeded);
            }

            let mut date_changed = self.seek_date(&mut cursor, Unit::Year)?;
            if !date_changed {
                date_changed = self.seek_date(&mut cursor, Unit::Month)?;
            }
            if !date_changed {
                date_changed = self.seek_date(&mut cursor, Unit::Day)?;
            }
            if date_changed {
                cursor = cursor.date().and_time(NaiveTime::MIN);
            }

            let mut time_changed = self.seek_time(&mut cursor, Unit::Hour)?;
            if !time_changed {
                time_changed = self.seek_time(&mut cursor, Unit::Minute)?;
            }
            if !time_changed {
                self.seek_time(&mut cursor, Unit::Second)?;
            }

            match tz.from_local_datetime(&cursor) {
                LocalResult::Single(instant) => {
                    if self.matches(cursor) {
                        return Ok(instant);
                    }
                    cursor = add_seconds(cursor, 1)?;
                }
                LocalResult::Ambiguous(earlier, later) => {
                    if self.matches(cursor) {
                        // An instant counts unless `origin` is a full second
                        // or more past it.
                        let behind = |instant: &DateTime<Tz>| -> Result<bool, CronError> {
                            let nudged = instant
                                .clone()
                                .checked_add_signed(TimeDelta::seconds(1))
                                .ok_or(CronError::InvalidTime)?;
                            Ok(*origin >= nudged)
                        };
                        if !behind(&earlier)? {
                            return Ok(earlier);
                        }
                        if !fixed_time && !behind(&later)? {
                            return Ok(later);
                        }
                    }
                    cursor = add_seconds(cursor, 1)?;
                }
                LocalResult::None => {
                    if !(fixed_time && self.matches(cursor)) {
                        cursor = add_seconds(cursor, 1)?;
                        continue;
                    }
                    // A fixed-time job swallowed by the gap runs at the
                    // first real instant after it.
                    let mut after_gap = cursor;
                    let mut searched = 0;
                    let resolved = loop {
                        after_gap = add_seconds(after_gap, 1)?;
                        searched += 1;
                        if searched > MAX_GAP_SEARCH_SECONDS {
                            return Err(CronError::TimeSearchLimitExceeded);
                        }
                        match tz.from_local_datetime(&after_gap) {
                            LocalResult::Single(instant) => break instant,
                            LocalResult::Ambiguous(earlier, _) => break earlier,
                            LocalResult::None => {}
                        }
                    };
                    // The date must still be a scheduled one; the hour and
                    // minute are the ones that went missing.
                    if self.day_matches(after_gap.date())
                        && self.month_matches(after_gap.month())
                        && Self::year_matches(after_gap.year())
                    {
                        let fires = match resolved.cmp(origin) {
                            Ordering::Less => false,
                            Ordering::Equal => inclusive,
                            Ordering::Greater => true,
                        };
                        if fires {
                            return Ok(resolved);
                        }
                    }
                    cursor = after_gap;
                }
            }
        }
    }

    /// Moves `cursor` forward until its year, month or day matches. Returns
    /// whether it moved.
    fn seek_date(&self, cursor: &mut NaiveDateTime, unit: Unit) -> Result<bool, CronError> {
        let mut changed = false;
        while !match unit {
            Unit::Year => Self::year_matches(cursor.year()),
            Unit::Month => self.month_matches(cursor.month()),
            _ => self.day_matches(cursor.date()),
        } {
            step_forward(cursor, unit)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Moves `cursor` to the next matching hour, minute or second within the
    /// enclosing unit, or steps the enclosing unit when there is none.
    /// Returns whether the enclosing unit was stepped.
    fn seek_time(&self, cursor: &mut NaiveDateTime, unit: Unit) -> Result<bool, CronError> {
        let (hour, minute, second) = (cursor.hour(), cursor.minute(), cursor.second());
        let (field, current, enclosing) = match unit {
            Unit::Hour => (&self.hours, hour, Unit::Day),
            Unit::Minute => (&self.minutes, minute, Unit::Hour),
            _ => (&self.seconds, second, Unit::Minute),
        };
        let Some(value) = field.next_match(current) else {
            step_forward(cursor, enclosing)?;
            return Ok(true);
        };
        if value != current {
            // Everything below the unit that moved restarts from zero.
            let time = match unit {
                Unit::Hour => NaiveTime::from_hms_opt(value, 0, 0),
                Unit::Minute => NaiveTime::from_hms_opt(hour, value, 0),
                _ => NaiveTime::from_hms_opt(hour, minute, value),
            }
            .ok_or(CronError::InvalidTime)?;
            *cursor = cursor.date().and_time(time);
        }
        Ok(false)
    }
}

/// Steps `cursor` to the start of the next `unit`.
fn step_forward(cursor: &mut NaiveDateTime, unit: Unit) -> Result<(), CronError> {
    if cursor.year() >= YEAR_UPPER_LIMIT {
        return Err(CronError::TimeSearchLimitExceeded);
    }
    *cursor = match unit {
        Unit::Hour | Unit::Minute => {
            let stepped = add_seconds(*cursor, if unit == Unit::Hour { 3600 } else { 60 })?;
            let minute = if unit == Unit::Hour {
                0
            } else {
                stepped.minute()
            };
            NaiveTime::from_hms_opt(stepped.hour(), minute, 0)
                .map(|time| stepped.date().and_time(time))
                .ok_or(CronError::InvalidTime)?
        }
        Unit::Year => NaiveDate::from_ymd_opt(cursor.year() + 1, 1, 1)
            .ok_or(CronError::InvalidDate)?
            .and_time(NaiveTime::MIN),
        Unit::Month => {
            let (year, month) = if cursor.month() == 12 {
                (cursor.year() + 1, 1)
            } else {
                (cursor.year(), cursor.month() + 1)
            };
            NaiveDate::from_ymd_opt(year, month, 1)
                .ok_or(CronError::InvalidDate)?
                .and_time(NaiveTime::MIN)
        }
        Unit::Day => cursor
            .date()
            .succ_opt()
            .ok_or(CronError::InvalidDate)?
            .and_time(NaiveTime::MIN),
        Unit::Second => return Err(CronError::InvalidTime),
    };
    Ok(())
}

/// The wall-clock time of `instant`, to the second.
fn wall_clock<Tz: TimeZone>(instant: &DateTime<Tz>) -> NaiveDateTime {
    let local = instant.naive_local();
    local.date().and_time(
        NaiveTime::from_hms_opt(local.hour(), local.minute(), local.second())
            .unwrap_or(NaiveTime::MIN),
    )
}

fn add_seconds(cursor: NaiveDateTime, seconds: i64) -> Result<NaiveDateTime, CronError> {
    cursor
        .checked_add_signed(TimeDelta::seconds(seconds))
        .ok_or(CronError::InvalidTime)
}

/// Whether `instant` is the earlier instance of a repeated wall-clock time.
fn on_earlier_fold<Tz: TimeZone>(instant: &DateTime<Tz>) -> bool {
    match instant.timezone().from_local_datetime(&wall_clock(instant)) {
        // Ordered against the later twin: `instant` may carry sub-seconds
        // the wall clock drops.
        LocalResult::Ambiguous(_, later) => *instant < later,
        _ => false,
    }
}

/// The moment the clock goes back, given `origin` on the earlier instance of
/// the repeated range: the first instant that is no longer on it.
fn later_fold_start<Tz: TimeZone>(origin: &DateTime<Tz>) -> Result<DateTime<Tz>, CronError> {
    let at_offset = |offset: i64| -> Result<DateTime<Tz>, CronError> {
        origin
            .clone()
            .checked_add_signed(TimeDelta::seconds(offset))
            .ok_or(CronError::InvalidTime)
    };
    let is_inside =
        |offset: i64| -> Result<bool, CronError> { Ok(on_earlier_fold(&at_offset(offset)?)) };

    if is_inside(MAX_OVERLAP_SECONDS)? {
        return Err(CronError::TimeSearchLimitExceeded);
    }
    let mut inside_offset = 0;
    let mut outside_offset = MAX_OVERLAP_SECONDS;
    while outside_offset - inside_offset > 1 {
        let middle = inside_offset + (outside_offset - inside_offset) / 2;
        if is_inside(middle)? {
            inside_offset = middle;
        } else {
            outside_offset = middle;
        }
    }
    at_offset(outside_offset)
}

/// The weekday (from Sunday) `days` after `weekday`.
fn shift_weekday(weekday: u32, days: u32) -> u32 {
    (weekday + days) % 7
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// `-SUN` comes first so a range ending on Sunday reads as `-7`.
const WEEKDAY_NAMES: [(&str, &str); 8] = [
    ("-SUN", "-7"),
    ("SUN", "0"),
    ("MON", "1"),
    ("TUE", "2"),
    ("WED", "3"),
    ("THU", "4"),
    ("FRI", "5"),
    ("SAT", "6"),
];

const MONTH_NAMES: [(&str, &str); 12] = [
    ("JAN", "1"),
    ("FEB", "2"),
    ("MAR", "3"),
    ("APR", "4"),
    ("MAY", "5"),
    ("JUN", "6"),
    ("JUL", "7"),
    ("AUG", "8"),
    ("SEP", "9"),
    ("OCT", "10"),
    ("NOV", "11"),
    ("DEC", "12"),
];

/// The five-field form of an `@nickname`, or the pattern unchanged.
fn expand_nickname(pattern: &str) -> &str {
    match pattern.trim() {
        "@YEARLY" | "@ANNUALLY" => "0 0 1 1 *",
        "@MONTHLY" => "0 0 1 * *",
        "@WEEKLY" => "0 0 * * SUN",
        "@DAILY" => "0 0 * * *",
        "@HOURLY" => "0 * * * *",
        other => other,
    }
}

fn reject_illegal_characters(part: &str, extra_allowed: &[char]) -> Result<(), CronError> {
    const BASE_ALLOWED: [char; 14] = [
        '*', '/', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ',', '-',
    ];
    match part
        .chars()
        .find(|ch| !BASE_ALLOWED.contains(ch) && !extra_allowed.contains(ch))
    {
        Some(ch) => Err(CronError::IllegalCharacters(format!(
            "CronPattern contains illegal character '{ch}' in part '{part}'"
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use chrono_tz::Tz;
    use croner::Cron;
    use croner::parser::{CronParser, Seconds, Year};

    use super::*;

    fn reference(expression: &str) -> Result<Cron, String> {
        CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build()
            .parse(expression)
            .map_err(|error| error.to_string())
    }

    fn instant(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap_or_else(|error| panic!("fixture instant {text}: {error}"))
            .with_timezone(&Utc)
    }

    /// Zones with ordinary, half-hour and irregular daylight-saving rules,
    /// odd fixed offsets, and none at all.
    const ZONES: [Tz; 9] = [
        Tz::UTC,
        Tz::America__New_York,
        Tz::Europe__Paris,
        Tz::Australia__Lord_Howe,
        Tz::Asia__Kolkata,
        Tz::Asia__Kathmandu,
        Tz::America__St_Johns,
        Tz::Africa__Casablanca,
        Tz::Pacific__Apia,
    ];

    /// Start instants in and around the 2026 transitions of the zones above,
    /// at month and year ends, on leap days, and between whole seconds.
    const INSTANTS: [&str; 36] = [
        "2026-01-14T10:07:00Z",
        "2026-06-15T12:00:00.999Z",
        // New York springs forward at 07:00Z on 8 March.
        "2026-03-08T05:00:00Z",
        "2026-03-08T06:29:59Z",
        "2026-03-08T06:59:59.500Z",
        "2026-03-08T07:00:00Z",
        // New York falls back at 06:00Z on 1 November; 01:00-02:00 repeats.
        "2026-11-01T04:00:00Z",
        "2026-11-01T05:00:00Z",
        "2026-11-01T05:30:00Z",
        "2026-11-01T05:30:00.500Z",
        "2026-11-01T05:59:59Z",
        "2026-11-01T06:00:00Z",
        "2026-11-01T06:30:00Z",
        "2026-11-01T06:59:59.750Z",
        // Paris springs forward at 01:00Z on 29 March and falls back at
        // 01:00Z on 25 October; 02:00-03:00 repeats.
        "2026-03-29T00:30:00Z",
        "2026-03-29T00:59:59Z",
        "2026-10-24T23:10:00Z",
        "2026-10-25T00:15:00Z",
        "2026-10-25T00:59:59.250Z",
        "2026-10-25T01:20:00Z",
        // Lord Howe falls back half an hour at 15:00Z on 4 April and springs
        // forward half an hour at 15:30Z on 3 October.
        "2026-04-04T14:45:00Z",
        "2026-04-04T15:10:00Z",
        "2026-10-03T15:20:00Z",
        // St John's and Casablanca transitions.
        "2026-03-08T05:15:00Z",
        "2026-11-01T03:45:00Z",
        "2026-02-15T01:30:00Z",
        "2026-03-22T01:30:00Z",
        // Month ends, leap days and a year end.
        "2026-01-31T23:59:30Z",
        "2026-02-28T12:00:00Z",
        "2024-02-28T23:00:00Z",
        "2024-02-29T23:59:59Z",
        "2100-02-27T08:00:00Z",
        "2027-04-30T22:00:00Z",
        "2026-12-31T23:59:59Z",
        // Apia skipped the whole of 30 December 2011.
        "2011-12-29T09:00:00Z",
        "2011-12-30T09:59:30Z",
    ];

    /// Asserts both implementations accept or refuse `expression` with the
    /// same sentence. Returns the parsed pair when both accept.
    fn assert_same_parse(expression: &str) -> Option<(CronExpression, Cron)> {
        let ours = CronExpression::parse(expression).map_err(|error| error.to_string());
        let theirs = reference(expression);
        match (ours, theirs) {
            (Ok(ours), Ok(theirs)) => Some((ours, theirs)),
            (Err(ours), Err(theirs)) => {
                assert_eq!(ours, theirs, "refusal of {expression:?}");
                None
            }
            (ours, theirs) => panic!(
                "{expression:?}: accepted by one implementation only: ours {:?}, croner {:?}",
                ours.map(|_| ()),
                theirs.map(|_| ())
            ),
        }
    }

    /// Asserts both implementations produce the same run of `count`
    /// occurrences after `start` in `tz`, or fail at the same point with the
    /// same sentence. Returns how many occurrences were found.
    fn assert_same_occurrences(
        expression: &str,
        ours: &CronExpression,
        theirs: &Cron,
        tz: Tz,
        start: DateTime<Utc>,
        count: usize,
    ) -> usize {
        let mut cursor = start.with_timezone(&tz);
        for step in 0..count {
            let our_next = ours.next_after(&cursor).map_err(|error| error.to_string());
            let their_next = theirs
                .find_next_occurrence(&cursor, false)
                .map_err(|error| error.to_string());
            assert_eq!(
                our_next.as_ref().map(|next| next.with_timezone(&Utc)),
                their_next.as_ref().map(|next| next.with_timezone(&Utc)),
                "{expression:?} in {tz}, occurrence {step} after {cursor} (started at {start})"
            );
            match our_next {
                Ok(next) => cursor = next,
                Err(_) => return step,
            }
        }
        count
    }

    /// Expressions whose behaviour depends on the time of day, so on
    /// daylight-saving gaps and repeated hours.
    const CLOCK_SENSITIVE: &[&str] = &[
        "* * * * *",
        "*/15 * * * *",
        "*/7 * * * *",
        "0 * * * *",
        "30 * * * *",
        "0-59/20 1-3 * * *",
        "0 0 * * *",
        "30 1 * * *",
        "30 2 * * *",
        "0 2 * * *",
        "59 2 * * *",
        "0 3 * * *",
        "15 1 * * *",
        "45 1 * * *",
        "0 1 * * *",
        "30 0 * * *",
        "45 23 * * *",
        "30 1,2 * * *",
        "30 1-3 * * *",
        "30 */2 * * *",
        "0,30 2 * * *",
        "*/30 2 * * *",
        "15-45/15 1 * * *",
        "30 2 8 3 *",
        "30 1 1 11 *",
        "30 2 * * 0",
        "30 2 * * SUN#2",
        "30 2 L * *",
        "30 2 29 3 *",
        "30 1 * * 7",
        "0 2 * 3 *",
        "30 2 25 10 *",
        "45 1 5 4 *",
        "45 1 4 10 *",
        "0 0 30 12 *",
        "30 12 30 12 *",
        "0 0 29-31 12 *",
        "0 2-3 * * *",
        "0-0/5 2 * * *",
        "5,5 2 * * *",
        "30 2-2 * * *",
        "@hourly",
        "@daily",
    ];

    /// Accepted expressions covering the rest of the grammar.
    const CALENDAR: &[&str] = &[
        "0 0 1 * *",
        "0 0 15 * *",
        "0 0 31 * *",
        "0 0 30 * *",
        "0 0 29 2 *",
        "0 12 28-31 * *",
        "0 0 L * *",
        "0 0 LW * *",
        "0 0 1W * *",
        "0 0 15W * *",
        "0 0 31W * *",
        "0 0 30W 4,6 *",
        "0 0 1W,15W,LW * *",
        "0 0 L,15 * *",
        "0 0 15W,L * *",
        "0 0 1W5 * *",
        "0 0 */10 * *",
        "0 0 1-7 * *",
        "0 0 10-20/3 * *",
        "0 0 ? * *",
        "0 0 ?/2 * *",
        "0 0 * * ?",
        "0 0 ? * ?",
        "0 0 1 1 *",
        "0 0 1 */3 *",
        "0 0 1 JAN *",
        "0 0 1 jan,Jul *",
        "0 0 1 MAR-OCT *",
        "0 0 1 Jun-Aug/2 *",
        "0 0 1 2-12/5 *",
        "0 0 * * 0",
        "0 0 * * 7",
        "0 0 * * 1",
        "0 0 * * 6",
        "0 0 * * MON",
        "0 0 * * sun",
        "0 0 * * 1-5",
        "0 0 * * MON-FRI",
        "0 0 * * FRI-SUN",
        "0 0 * * 5-7",
        "0 0 * * 0-7",
        "0 0 * * */2",
        "0 0 * * */7",
        "0 0 * * 0-6/3",
        "0 0 * * 6,0",
        "0 0 * * mon,wed,fri",
        "0 0 * * 5L",
        "0 0 * * FRIL",
        "0 0 * * 5#L",
        "0 0 * * 7L",
        "0 0 * * 0L",
        "0 0 * * 1#1",
        "0 0 * * 5#5",
        "0 0 * * 2#3",
        "0 0 * * 0#2",
        "0 0 * * 7#1",
        "0 0 * * SUN#5",
        "0 0 * * TUE#2,THU#4",
        "0 0 * * 1-5#2",
        "0 0 * * */2#1",
        "0 0 * * 1-5/2#3",
        "0 0 * * 1,5L",
        "0 0 * * 1#1,1#3",
        "0 0 13 * 5",
        "0 0 13 * +5",
        "0 0 1 * 1",
        "0 0 1 * +1",
        "0 0 L * +5",
        "0 0 LW * +FRI",
        "0 0 1-7 * +MON",
        "0 0 15W * +1-5",
        "0 0 * * +1",
        "0 0 * * +*",
        "0 0 1 * +*",
        "0 0 1 * +?",
        "0 0 */2 * 1",
        "0 0 */2 * +1",
        "0 0 29 2 1",
        "0 0 29 2 +1",
        "0 0 31 * 5#5",
        "0 0 31 * +5#5",
        "0 0 L * 0L",
        "0 0 L 2 *",
        "0 0 LW 2 *",
        "0 0 15W 2 +3",
        "17 4 * * 1-5",
        "5 4 * * sun",
        "0 22 * * 1-5",
        "23 0-20/2 * * *",
        "0 0,12 1 */2 *",
        "0 4 8-14 * *",
        "0 9-17 * * MON-FRI",
        "*/5 9-17 * * 1-5",
        "0 0 1,15 * 3",
        "0 0 ,1 * *",
        "0 0 1,,15 * *",
        "0 ,0 * * *",
        "0, 0 * * *",
        "0 0 1 DEC *",
        "0 0 1 sep,OCT,nov *",
        "JAN FEB * * *",
        "MON TUE * * *",
        "0 0 * JAN-MAR MON-WED",
        "0 0 * * sat-SUN",
        "  0   0  *  *  *  ",
        "0\t0\t*\t*\t*",
        "@yearly",
        "@annually",
        "@monthly",
        "@weekly",
        "@Daily",
        " @weekly ",
        "1-5 0-3 1-2 1-2 0-1",
        "0-59 0-23 1-31 1-12 0-7",
        "0 0 1-31/1 * *",
        "0 0 28-31/3 * *",
        "0 0 5-5 5-5 *",
        "0 0 31 1,3,5,7,8,10,12 *",
        "0 0 30 4,6,9,11 *",
        "0 0 29 2 0#1",
        "0 0 * 2 5#5",
        "0 0 1W 1 *",
        "0 0 31W 12 *",
        "0 0 LW 12 +5L",
    ];

    /// Refused expressions, and oddities worth pinning either way.
    const MALFORMED: &[&str] = &[
        "",
        "   ",
        "not a cron",
        "* * * *",
        "* * *",
        "*",
        "* * * * * *",
        "0 */15 * * * *",
        "0 0 0 * * * 2026",
        "* * * * * * *",
        "61 * * * *",
        "60 * * * *",
        "-1 * * * *",
        "* 24 * * *",
        "* * 0 * *",
        "* * 32 * *",
        "* * * 0 *",
        "* * * 13 *",
        "* * * * 8",
        "*/0 * * * *",
        "*/60 * * * *",
        "*/61 * * * *",
        "*/65536 * * * *",
        "7/5 * * * *",
        "/5 * * * *",
        "0/10 * * * *",
        "*/ * * * *",
        "*/a * * * *",
        "*/5/2 * * * *",
        "1- * * * *",
        "-5 * * * *",
        "1-2-3 * * * *",
        "5-1 * * * *",
        "59-0 * * * *",
        "0-60 * * * *",
        "0-60/5 * * * *",
        "5-1/2 * * * *",
        "1-2-3/2 * * * *",
        "-/2 * * * *",
        "1-/2 * * * *",
        "** * * * *",
        "*,5 * * * *",
        "5,* * * * *",
        "1.5 * * * *",
        "a * * * *",
        "L * * * *",
        "* L * * *",
        "* * * L *",
        "W * * * *",
        "* * W * *",
        "* * 0W * *",
        "* * 32W * *",
        "* * WL * *",
        "* * 5L * *",
        "* * L5 * *",
        "* * 1#2 * *",
        "* * LW5 * *",
        "* * 15WW * *",
        "* * L-3 * *",
        "* * * * W",
        "* * * * 1W",
        "* * * * #2",
        "* * * * 1#",
        "* * * * 1#0",
        "* * * * 1#6",
        "* * * * 1#a",
        "* * * * 1#22",
        "* * * * 8#1",
        "* * * * 1#2#3",
        "* * * * L5",
        "* * * * LL",
        "* * * * 8L",
        "* * * * 1-5L",
        "* * * * 1-L",
        "* * * * +",
        "* * * * ++1",
        "* * * * 1+",
        "+* * * * *",
        "+5 0 * * 1",
        "* * +1 * *",
        "* * * * MONDAY",
        "* * * * MO",
        "* * * JANUARY *",
        "* * * JA *",
        "* * * DECEMBER *",
        "* * * * SUN-MON",
        "* * * * 7-1",
        "* * * * SAT-SUN-MON",
        "? * * * *",
        "* ? * * *",
        "* * * ? *",
        "* * 5? * *",
        "* * ?? * *",
        "* * * * ?5",
        "@reboot",
        "@every 5m",
        "@daily * * * *",
        "@",
        "@yearly @yearly",
        "0 0 * * * @daily",
        "* * * * * ?",
        "0 0 1 1 * 2026",
        "65536 * * * *",
        "99999999999 * * * *",
        "0x10 * * * *",
        "１ * * * *",
        "* * * * ſun",
        "* * * * 1;",
        "*\u{a0}* * * *",
        "*/15 * * * * ",
        "TZ=UTC * * * * *",
    ];

    /// Accepted expressions that have no occurrence: the search ends at the
    /// year limit.
    const NEVER: &[&str] = &[
        "0 0 30 2 *",
        "0 0 31 2,4,6,9,11 *",
        "0 0 31 4 *",
        "0 0 * * L",
        "0 0 1 * +L",
        "0 0 1 * +5#5",
        "0 0 , * *",
        "0 0 * * ,",
        "0 0 * , *",
        "0 0 30 2 +1",
    ];

    /// Accepted expressions whose minute or hour field selects nothing, so
    /// the search steps an hour or a day at a time to the year limit.
    const NEVER_BY_TIME: &[&str] = &[", * * * *", "* , * * *", ", , * * *", ",, 3 1 1 *"];

    #[test]
    fn clock_sensitive_expressions_match_croner_in_every_zone_at_every_instant() {
        for expression in CLOCK_SENSITIVE {
            let (ours, theirs) = assert_same_parse(expression)
                .unwrap_or_else(|| panic!("{expression:?} should be accepted"));
            for tz in ZONES {
                for start in INSTANTS {
                    assert_same_occurrences(expression, &ours, &theirs, tz, instant(start), 4);
                }
            }
        }
    }

    #[test]
    fn calendar_expressions_match_croner() {
        for (index, expression) in CALENDAR.iter().enumerate() {
            let (ours, theirs) = assert_same_parse(expression)
                .unwrap_or_else(|| panic!("{expression:?} should be accepted"));
            // Every expression meets every zone and every instant, on a
            // rotating pairing rather than the full product.
            for (offset, start) in INSTANTS.iter().enumerate() {
                let tz = ZONES[(index + offset) % ZONES.len()];
                assert_same_occurrences(expression, &ours, &theirs, tz, instant(start), 6);
            }
        }
    }

    #[test]
    fn malformed_expressions_are_refused_like_croner() {
        let mut refused = 0;
        for expression in MALFORMED {
            match assert_same_parse(expression) {
                None => refused += 1,
                Some((ours, theirs)) => {
                    for (offset, start) in INSTANTS.iter().enumerate().take(6) {
                        let tz = ZONES[offset % ZONES.len()];
                        assert_same_occurrences(expression, &ours, &theirs, tz, instant(start), 3);
                    }
                }
            }
        }
        // The list is mostly refusals; a parser that accepted everything
        // would not pass by agreeing with itself.
        assert!(refused > MALFORMED.len() * 3 / 4, "{refused} refused");
    }

    #[test]
    fn expressions_with_no_occurrence_fail_like_croner() {
        for (index, expression) in NEVER.iter().enumerate() {
            let (ours, theirs) = assert_same_parse(expression)
                .unwrap_or_else(|| panic!("{expression:?} should be accepted"));
            let start = if index % 2 == 0 {
                instant("2026-01-14T10:07:00Z").with_timezone(&Tz::UTC)
            } else {
                instant("2026-11-01T05:30:00Z").with_timezone(&Tz::America__New_York)
            };
            assert_eq!(
                ours.next_after(&start),
                Err(CronError::TimeSearchLimitExceeded),
                "{expression:?}"
            );
            assert_eq!(
                theirs
                    .find_next_occurrence(&start, false)
                    .map_err(|error| error.to_string()),
                Err(CronError::TimeSearchLimitExceeded.to_string()),
                "{expression:?}"
            );
        }
        // These step through every hour or day to the limit, so they start
        // close to it.
        for expression in NEVER_BY_TIME {
            let (ours, theirs) = assert_same_parse(expression)
                .unwrap_or_else(|| panic!("{expression:?} should be accepted"));
            for tz in [Tz::UTC, Tz::Europe__Paris] {
                assert_same_occurrences(
                    expression,
                    &ours,
                    &theirs,
                    tz,
                    instant("4999-03-01T00:00:00Z"),
                    1,
                );
            }
        }
    }

    #[test]
    fn searches_near_the_year_limit_match_croner() {
        for expression in [
            "* * * * *",
            "0 0 * * *",
            "30 2 * * *",
            "0 0 1 1 *",
            "0 0 L * 5L",
        ] {
            let (ours, theirs) = assert_same_parse(expression)
                .unwrap_or_else(|| panic!("{expression:?} should be accepted"));
            for tz in ZONES {
                for start in [
                    "4999-12-31T23:58:30Z",
                    "5000-12-30T10:00:00Z",
                    "5000-12-31T23:58:30Z",
                    "5001-06-01T00:00:00Z",
                ] {
                    assert_same_occurrences(expression, &ours, &theirs, tz, instant(start), 4);
                }
            }
        }
    }

    /// A fixed-seed xorshift generator, so the generated corpus is the same
    /// on every run.
    struct Generator(u64);

    impl Generator {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a>(&mut self, pool: &[&'a str]) -> &'a str {
            pool[(self.next() % pool.len() as u64) as usize]
        }
    }

    const MINUTE_PARTS: &[&str] = &[
        "*",
        "*",
        "0",
        "0",
        "5",
        "30",
        "59",
        "*/15",
        "*/7",
        "0-59/20",
        "10-20",
        "1,31",
        "0,15,30,45",
        "5-10/2",
        "0-0/5",
        "30-30",
        "60",
        "*/0",
        "7/5",
        "1-",
        "x",
    ];
    const HOUR_PARTS: &[&str] = &[
        "*", "*", "0", "1", "2", "3", "12", "23", "*/6", "9-17", "0-23/5", "1,2", "2-3", "1-1",
        "22-23/1", "24", "/2",
    ];
    const DAY_PARTS: &[&str] = &[
        "*", "*", "*", "?", "1", "15", "28", "29", "30", "31", "L", "LW", "1W", "15W", "31W",
        "*/10", "1-7", "10-20/3", "1,15", "L,15", "15W,L", "1W,LW", "?/2", "32", "0", "5L", "W",
    ];
    const MONTH_PARTS: &[&str] = &[
        "*",
        "*",
        "*",
        "*",
        "1",
        "3",
        "6",
        "12",
        "JAN",
        "feb",
        "*/3",
        "1-6",
        "MAR-OCT",
        "2,4,6",
        "Jun-Aug/2",
        "11-12",
        "13",
        "0",
    ];
    const WEEKDAY_PARTS: &[&str] = &[
        "*",
        "*",
        "*",
        "?",
        "0",
        "1",
        "5",
        "6",
        "7",
        "MON",
        "sun",
        "1-5",
        "MON-FRI",
        "FRI-SUN",
        "5L",
        "FRIL",
        "7L",
        "1#1",
        "5#5",
        "2#3",
        "0#2",
        "7#1",
        "SUN#5",
        "*/2",
        "0-6/3",
        "6,0",
        "1,3,5",
        "mon,wed",
        "TUE#2,THU#4",
        "1-5#2",
        "*/2#1",
        "+1",
        "+MON-FRI",
        "+5L",
        "+*",
        "+2#3",
        "+0",
        "1#6",
        "1#0",
        "8",
        "#2",
        "+",
    ];

    #[test]
    fn generated_expressions_match_croner() {
        let mut generator = Generator(0x9E37_79B9_7F4A_7C15);
        let (mut accepted, mut refused, mut never) = (0, 0, 0);
        for index in 0..1000 {
            let expression = format!(
                "{} {} {} {} {}",
                generator.pick(MINUTE_PARTS),
                generator.pick(HOUR_PARTS),
                generator.pick(DAY_PARTS),
                generator.pick(MONTH_PARTS),
                generator.pick(WEEKDAY_PARTS),
            );
            let Some((ours, theirs)) = assert_same_parse(&expression) else {
                refused += 1;
                continue;
            };
            accepted += 1;
            for offset in 0..3 {
                let tz = ZONES[(index + offset) % ZONES.len()];
                let start = instant(INSTANTS[(index * 7 + offset * 11) % INSTANTS.len()]);
                let fired = assert_same_occurrences(&expression, &ours, &theirs, tz, start, 3);
                // An expression with no occurrence walks to the year limit;
                // once is enough.
                if fired == 0 {
                    never += 1;
                    break;
                }
            }
        }
        assert!(
            accepted > 300 && refused > 300 && never < accepted / 4,
            "{accepted} accepted, {refused} refused, {never} without an occurrence"
        );
    }
}
