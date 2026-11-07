//! SQL shared by the catalog ownership reads that a [`CatalogOwnedProbe`]
//! scopes: the probe binds as JSON arrays, and the id expressions here are
//! the ones migration 0328 indexes, so they must keep the same spelling.

use scryer_application::CatalogOwnedProbe;
use serde_json::Value as JsonValue;

use crate::queries::sql_runtime::SqlArg;

/// `probe_values`, `probe_numbers` and `probe_titles`, in that order, each
/// reading one JSON array argument. Rendered without a leading `WITH` or a
/// trailing comma.
pub(crate) fn probe_ctes(postgres: bool) -> &'static str {
    if postgres {
        "probe_values(value) AS MATERIALIZED (
            SELECT probe.value
            FROM jsonb_array_elements_text(CAST({} AS JSONB)) AS probe(value)
         ),
         probe_numbers(value) AS MATERIALIZED (
            SELECT CAST(probe.value AS NUMERIC)
            FROM jsonb_array_elements_text(CAST({} AS JSONB)) AS probe(value)
         ),
         probe_titles(id) AS MATERIALIZED (
            SELECT probe.value
            FROM jsonb_array_elements_text(CAST({} AS JSONB)) AS probe(value)
         )"
    } else {
        "probe_values(value) AS MATERIALIZED (
            SELECT value FROM json_each({})
         ),
         probe_numbers(value) AS MATERIALIZED (
            SELECT value FROM json_each({})
         ),
         probe_titles(id) AS MATERIALIZED (
            SELECT value FROM json_each({})
         )"
    }
}

/// The three arguments [`probe_ctes`] reads.
pub(crate) fn probe_args(probe: &CatalogOwnedProbe) -> [SqlArg; 3] {
    [
        SqlArg::Json(JsonValue::Array(
            probe
                .values
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        )),
        SqlArg::Json(JsonValue::Array(
            probe
                .numeric_values
                .iter()
                .map(|value| JsonValue::from(*value))
                .collect(),
        )),
        SqlArg::Json(JsonValue::Array(
            probe
                .title_ids
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        )),
    ]
}

/// The id as the probe's `values` compare it.
pub(crate) fn probe_value_expression(column: &str) -> String {
    format!("LOWER(TRIM({column}))")
}

/// The number formed by the id's trailing digits, or 0 / `NULL` when it has
/// none. Every id whose ownership key carries a canonical number `n` (its
/// last `:` segment parses to `n`) ends in the digits of `n`, so comparing
/// this against the probe's numbers never misses one.
pub(crate) fn probe_numeric_expression(postgres: bool, column: &str) -> String {
    if postgres {
        format!("CAST(SUBSTRING({column} FROM '([0-9]+)\\s*$') AS NUMERIC)")
    } else {
        format!(
            "CAST(SUBSTR(TRIM({column}), LENGTH(RTRIM(TRIM({column}), '0123456789')) + 1) AS INTEGER)"
        )
    }
}

/// `column` matches the probe by value or by number.
pub(crate) fn probe_id_match_clause(postgres: bool, column: &str) -> String {
    format!(
        "({value} IN (SELECT value FROM probe_values)
          OR {number} IN (SELECT value FROM probe_numbers))",
        value = probe_value_expression(column),
        number = probe_numeric_expression(postgres, column),
    )
}
