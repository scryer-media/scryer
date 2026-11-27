//! When an episode counts as aired, as SQL every count shares.

/// SQL that is true when the episode at `alias` has aired by `day`.
///
/// An episode has aired when it has a full air date, a `YYYY-MM-DD` day with
/// or without a time after it, whose day is on or before `day`. A year or a
/// month alone has not aired. `day` is a text SQL expression: a bind
/// placeholder, or the dialect's current-date text. The SQL is the same on
/// SQLite and Postgres.
pub(crate) fn episode_aired_by_sql(alias: &str, day: &str) -> String {
    format!(
        "(length(trim(COALESCE({alias}.air_date, ''))) >= 10 \
          AND substr(trim({alias}.air_date), 1, 10) <= {day})"
    )
}
