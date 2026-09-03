//! Shared persistence for advanced-monitoring selections.
//!
//! One table (`monitor_selections`) holds both owners: a title
//! (`owner_kind = 'title'`) and a still-pending media request
//! (`owner_kind = 'media_request'`). Rows are the things the owner monitors;
//! anything absent stays unmonitored.

use std::collections::HashMap;

use scryer_application::{AppError, AppResult};
use scryer_domain::{ExternalId, MonitorSelection, MonitorSelectionMovie};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx};

pub(crate) const OWNER_KIND_TITLE: &str = "title";
pub(crate) const OWNER_KIND_MEDIA_REQUEST: &str = "media_request";

const ENTRY_KIND_SEASON: &str = "season";
const ENTRY_KIND_SERIES_MOVIE: &str = "series_movie";

const SELECT_COLUMNS: &str = "owner_id, entry_kind, entry_key, label, external_ids_json";

/// Replace every row for one owner. `None` (or an empty selection) leaves the
/// owner with no rows at all, which the readers report back as `None`.
pub(crate) async fn replace_monitor_selection_tx(
    tx: &mut SqlTx<'_>,
    owner_kind: &str,
    owner_id: &str,
    selection: Option<&MonitorSelection>,
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM monitor_selections WHERE owner_kind = {} AND owner_id = {}",
        &[
            SqlArg::Text(owner_kind.to_string()),
            SqlArg::Text(owner_id.to_string()),
        ],
    )
    .await?;

    let Some(selection) = selection else {
        return Ok(());
    };
    let selection = selection.normalized();
    if selection.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();
    for season in &selection.seasons {
        tx.execute(
            "INSERT INTO monitor_selections (
                owner_kind, owner_id, entry_kind, entry_key, label, external_ids_json,
                created_at, updated_at
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(owner_kind.to_string()),
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(ENTRY_KIND_SEASON.to_string()),
                SqlArg::Text(season.to_string()),
                SqlArg::OptText(None),
                SqlArg::Text("[]".to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    for movie in &selection.series_movies {
        // `normalized()` drops movies without a canonical key, so this cannot
        // be `None` here; skip defensively rather than persist a blank key.
        let Some(entry_key) = movie.canonical_key() else {
            continue;
        };
        let external_ids_json = serde_json::to_string(&movie.external_ids).map_err(|error| {
            AppError::Repository(format!("serialize monitor selection movie ids: {error}"))
        })?;
        tx.execute(
            "INSERT INTO monitor_selections (
                owner_kind, owner_id, entry_kind, entry_key, label, external_ids_json,
                created_at, updated_at
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(owner_kind.to_string()),
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(ENTRY_KIND_SERIES_MOVIE.to_string()),
                SqlArg::Text(entry_key),
                SqlArg::OptText(Some(movie.name.clone())),
                SqlArg::Text(external_ids_json),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

/// Deleting a library cascades to its titles and media requests, but
/// `monitor_selections` has no foreign key to either, so its rows have to be
/// cleared explicitly before the cascade removes the owners.
pub(crate) async fn delete_monitor_selections_for_library_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
) -> AppResult<()> {
    for owners in [
        (OWNER_KIND_TITLE, "titles"),
        (OWNER_KIND_MEDIA_REQUEST, "media_requests"),
    ] {
        let (owner_kind, table) = owners;
        tx.execute(
            &format!(
                "DELETE FROM monitor_selections
                  WHERE owner_kind = {{}}
                    AND owner_id IN (SELECT id FROM {table} WHERE library_id = {{}})"
            ),
            &[
                SqlArg::Text(owner_kind.to_string()),
                SqlArg::Text(library_id.to_string()),
            ],
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn load_monitor_selection(
    exec: SqlExec<'_, '_>,
    owner_kind: &str,
    owner_id: &str,
) -> AppResult<Option<MonitorSelection>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        &format!(
            "SELECT {SELECT_COLUMNS}
               FROM monitor_selections
              WHERE owner_kind = {{}} AND owner_id = {{}}
              ORDER BY entry_kind, entry_key"
        ),
        &[
            SqlArg::Text(owner_kind.to_string()),
            SqlArg::Text(owner_id.to_string()),
        ],
    )
    .await?;
    let mut by_owner = collect_selections(&rows)?;
    Ok(by_owner.remove(owner_id))
}

/// Batch read for list endpoints: one query for every owner instead of one per
/// row.
pub(crate) async fn load_monitor_selections_for_owners(
    exec: SqlExec<'_, '_>,
    owner_kind: &str,
    owner_ids: &[String],
) -> AppResult<HashMap<String, MonitorSelection>> {
    if owner_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("{}", owner_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {SELECT_COLUMNS}
           FROM monitor_selections
          WHERE owner_kind = {{}} AND owner_id IN ({placeholders})
          ORDER BY owner_id, entry_kind, entry_key"
    );
    let mut args = vec![SqlArg::Text(owner_kind.to_string())];
    args.extend(owner_ids.iter().cloned().map(SqlArg::Text));
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    collect_selections(&rows)
}

fn collect_selections(rows: &[SqlRow]) -> AppResult<HashMap<String, MonitorSelection>> {
    let mut selections: HashMap<String, MonitorSelection> = HashMap::new();
    for row in rows {
        let owner_id = row.text("owner_id")?;
        let entry_kind = row.text("entry_kind")?;
        let entry_key = row.text("entry_key")?;
        let entry = selections.entry(owner_id).or_default();
        match entry_kind.as_str() {
            ENTRY_KIND_SEASON => {
                let season = entry_key.trim().parse::<i32>().map_err(|error| {
                    AppError::Repository(format!(
                        "monitor selection season key {entry_key} is not a number: {error}"
                    ))
                })?;
                entry.seasons.push(season);
            }
            ENTRY_KIND_SERIES_MOVIE => {
                let raw_ids = row.text("external_ids_json")?;
                let external_ids: Vec<ExternalId> =
                    serde_json::from_str(&raw_ids).map_err(|error| {
                        AppError::Repository(format!(
                            "deserialize monitor selection movie ids: {error}"
                        ))
                    })?;
                entry.series_movies.push(MonitorSelectionMovie {
                    name: row.opt_text("label")?.unwrap_or_default(),
                    external_ids,
                });
            }
            other => {
                return Err(AppError::Repository(format!(
                    "unknown monitor selection entry kind {other}"
                )));
            }
        }
    }
    Ok(selections
        .into_iter()
        .map(|(owner_id, selection)| (owner_id, selection.normalized()))
        .collect())
}
