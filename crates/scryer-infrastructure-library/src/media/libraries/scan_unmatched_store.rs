use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
    LibraryScanUnmatchedSearchAttempt, PendingImportStatus,
};
use scryer_domain::MediaFacet;

use crate::media::id_list_statements::{id_list_statements, max_binds};
use crate::queries::common::parse_utc_datetime;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sql::json::{canonical_json_arg, json_text_or};

const UNMATCHED_COLUMNS: &str = "id, library_id, facet, title_id, scan_session_id, scan_root,
    item_path, display_name, query, year_hint, reason_code, error_message,
    search_attempts_json, status, size_bytes, created_at, updated_at";

#[derive(Clone)]
pub struct LibraryScanUnmatchedStore {
    datastore: StoreDatastore,
}

impl LibraryScanUnmatchedStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for LibraryScanUnmatchedStore {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let args = vec![
            SqlArg::Text(item.id.clone()),
            SqlArg::Text(item.library_id.clone()),
            SqlArg::Text(item.facet.as_str().to_string()),
            SqlArg::OptText(item.title_id.clone()),
            SqlArg::Text(item.scan_session_id.clone()),
            SqlArg::Text(item.scan_root.clone()),
            SqlArg::Text(item.item_path.clone()),
            SqlArg::Text(item.display_name.clone()),
            SqlArg::Text(item.query.clone()),
            SqlArg::OptI32(item.year_hint),
            SqlArg::Text(item.reason_code.clone()),
            SqlArg::OptText(item.error_message.clone()),
            canonical_json_arg(&item.search_attempts)?,
            SqlArg::Text(item.status.as_str().to_string()),
            SqlArg::OptI64(item.size_bytes),
            timestamp_arg_for_datastore(&self.datastore, &item.created_at)?,
            timestamp_arg_for_datastore(&self.datastore, &item.updated_at)?,
        ];

        execute_write(
            &self.datastore,
            "upsert_library_scan_unmatched_item",
            "INSERT INTO library_scan_unmatched_items
             (id, library_id, facet, title_id, scan_session_id, scan_root, item_path, display_name,
              query, year_hint, reason_code, error_message, search_attempts_json, status,
              size_bytes, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT(library_id, item_path) DO UPDATE SET
                id = excluded.id,
                library_id = excluded.library_id,
                facet = excluded.facet,
                title_id = excluded.title_id,
                scan_session_id = excluded.scan_session_id,
                scan_root = excluded.scan_root,
                item_path = excluded.item_path,
                display_name = excluded.display_name,
                query = excluded.query,
                year_hint = excluded.year_hint,
                reason_code = excluded.reason_code,
                error_message = excluded.error_message,
                search_attempts_json = excluded.search_attempts_json,
                -- A rescan that cannot determine a size (folder-shaped items,
                -- unreadable files) must not erase a size an earlier scan knew.
                size_bytes = COALESCE(excluded.size_bytes, library_scan_unmatched_items.size_bytes),
                status = CASE
                    WHEN library_scan_unmatched_items.status = 'ignored' AND excluded.status = 'pending'
                        THEN library_scan_unmatched_items.status
                    ELSE excluded.status
                END,
                updated_at = excluded.updated_at",
            args,
        )
        .await?;

        Ok(item.id.clone())
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        let sql = format!(
            "SELECT {UNMATCHED_COLUMNS}
             FROM library_scan_unmatched_items
             WHERE id = {{}}"
        );
        fetch_optional_unmatched_item(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_library_scan_unmatched_item",
            "DELETE FROM library_scan_unmatched_items
             WHERE library_id = {} AND facet = {} AND item_path = {}",
            vec![
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(item_path.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_library_scan_unmatched_items(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_paths: &[String],
    ) -> AppResult<()> {
        // Keeps each statement well under both engines' bind limits.
        const PATHS_PER_STATEMENT: usize = 500;
        if item_paths.is_empty() {
            return Ok(());
        }
        let library_id = library_id.to_string();
        let facet = facet.as_str().to_string();
        let item_paths = item_paths.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_library_scan_unmatched_items",
            move |tx| {
                let library_id = library_id.clone();
                let facet = facet.clone();
                let item_paths = item_paths.clone();
                Box::pin(async move {
                    for chunk in item_paths.chunks(PATHS_PER_STATEMENT) {
                        let placeholders = std::iter::repeat_n("{}", chunk.len())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let mut args = vec![
                            SqlArg::Text(library_id.clone()),
                            SqlArg::Text(facet.clone()),
                        ];
                        args.extend(chunk.iter().cloned().map(SqlArg::Text));
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            &format!(
                                "DELETE FROM library_scan_unmatched_items
                                 WHERE library_id = {{}} AND facet = {{}}
                                   AND item_path IN ({placeholders})"
                            ),
                            &args,
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        execute_write(
            &self.datastore,
            "delete_library_scan_unmatched_items_for_library",
            "DELETE FROM library_scan_unmatched_items WHERE library_id = {}",
            vec![SqlArg::Text(library_id.to_string())],
        )
        .await
        .map(|rows| rows as u32)
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<u32> {
        execute_write(
            &self.datastore,
            "delete_library_scan_unmatched_items_for_title",
            "DELETE FROM library_scan_unmatched_items WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await
        .map(|rows| rows as u32)
    }

    async fn list_pending_folder_conflicts_for_title(
        &self,
        library_id: &str,
        title_id: &str,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        fetch_unmatched_items(self.datastore.read_exec(), &format!("SELECT {UNMATCHED_COLUMNS} FROM library_scan_unmatched_items WHERE library_id = {{}} AND title_id = {{}} AND status = 'pending' AND reason_code = {{}} ORDER BY id LIMIT 1001"), &[SqlArg::Text(library_id.to_owned()), SqlArg::Text(title_id.to_owned()), SqlArg::Text(scryer_application::location::conflicts::PENDING_REASON.to_owned())]).await
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        let (sql, args) = build_unmatched_list_sql(facet, scan_root, status, Some((limit, offset)));
        fetch_unmatched_items(self.datastore.read_exec(), &sql, &args).await
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        let (sql, args) = build_unmatched_count_sql(facet, scan_root, status);
        let row = SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .ok_or_else(|| AppError::Repository("missing unmatched item count".to_string()))?;
        row.i64("count")
    }

    async fn count_pending_library_scan_items_requiring_action(
        &self,
        folder_ownership_reason_codes: &[&str],
    ) -> AppResult<Option<Vec<(String, MediaFacet, i64)>>> {
        let facets = [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime];
        let mut args = vec![SqlArg::Text(
            PendingImportStatus::Pending.as_str().to_string(),
        )];
        args.extend(
            facets
                .iter()
                .map(|facet| SqlArg::Text(facet.as_str().to_string())),
        );
        let reason_sql = if folder_ownership_reason_codes.is_empty() {
            "1 = 0".to_string()
        } else {
            args.extend(
                folder_ownership_reason_codes
                    .iter()
                    .map(|code| SqlArg::Text((*code).to_string())),
            );
            format!(
                "reason_code IN ({})",
                std::iter::repeat_n("{}", folder_ownership_reason_codes.len())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        args.push(SqlArg::Text(MediaFacet::Movie.as_str().to_string()));
        let sql = format!(
            "SELECT library_id, facet, COUNT(*) AS count
               FROM library_scan_unmatched_items
              WHERE status = {{}}
                AND facet IN ({{}}, {{}}, {{}})
                AND ({reason_sql} OR NOT (facet = {{}} AND title_id IS NOT NULL))
              GROUP BY library_id, facet"
        );
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut counts = Vec::with_capacity(rows.len());
        for row in &rows {
            let facet_raw = row.text("facet")?;
            let facet = MediaFacet::parse(&facet_raw).ok_or_else(|| {
                AppError::Repository(format!(
                    "library scan unmatched item has invalid facet '{facet_raw}'"
                ))
            })?;
            let library_id = row
                .opt_text("library_id")?
                .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet));
            counts.push((library_id, facet, row.i64("count")?));
        }
        Ok(Some(counts))
    }

    async fn page_library_scan_items_requiring_action(
        &self,
        facet: MediaFacet,
        status: PendingImportStatus,
        library_ids: &[String],
        folder_ownership_reason_codes: &[&str],
        limit: i64,
        offset: i64,
    ) -> AppResult<Option<(i64, Vec<LibraryScanUnmatchedItem>)>> {
        if library_ids.is_empty() {
            return Ok(Some((0, Vec::new())));
        }
        let (where_sql, args) = requiring_action_page_filter(
            &facet,
            &status,
            library_ids,
            folder_ownership_reason_codes,
        );
        let count_sql =
            format!("SELECT COUNT(*) AS count FROM library_scan_unmatched_items WHERE {where_sql}");
        let total = SqlRuntime::fetch_optional(self.datastore.read_exec(), &count_sql, &args)
            .await?
            .ok_or_else(|| AppError::Repository("missing unmatched item count".to_string()))?
            .i64("count")?;
        if total == 0 || limit <= 0 || offset >= total {
            return Ok(Some((total, Vec::new())));
        }
        let page_sql = format!(
            "SELECT {UNMATCHED_COLUMNS}
               FROM library_scan_unmatched_items
              WHERE {where_sql}
              ORDER BY updated_at DESC, item_path ASC
              LIMIT {{}} OFFSET {{}}"
        );
        let mut page_args = args;
        page_args.push(SqlArg::I64(limit));
        page_args.push(SqlArg::I64(offset));
        let items =
            fetch_unmatched_items(self.datastore.read_exec(), &page_sql, &page_args).await?;
        Ok(Some((total, items)))
    }

    async fn get_library_scan_unmatched_items(
        &self,
        ids: &[String],
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (ids, per_statement) = id_list_statements(
            ids,
            1,
            0,
            max_binds(&self.datastore.read_exec()),
            UNMATCHED_ID_LIST_BIND_CHUNK,
        );
        let mut items = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(per_statement) {
            let sql = format!(
                "SELECT {UNMATCHED_COLUMNS}
                   FROM library_scan_unmatched_items
                  WHERE id IN ({})",
                std::iter::repeat_n("{}", chunk.len())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
            items.extend(fetch_unmatched_items(self.datastore.read_exec(), &sql, &args).await?);
        }
        Ok(items)
    }

    async fn ignore_library_scan_unmatched_items(
        &self,
        rows: &[(String, String)],
    ) -> AppResult<Option<()>> {
        if rows.is_empty() {
            return Ok(Some(()));
        }
        // Each row binds its id twice (the CASE arm and the IN list) and its
        // timestamp once; a statement stays well under both engines' limits.
        const ROWS_PER_STATEMENT: usize = 1_000;
        let mut bound = Vec::with_capacity(rows.len());
        for (id, updated_at) in rows {
            bound.push((
                id.clone(),
                timestamp_arg_for_datastore(&self.datastore, updated_at)?,
            ));
        }
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "ignore_library_scan_unmatched_items",
            move |tx| {
                let bound = bound.clone();
                Box::pin(async move {
                    let ignored = PendingImportStatus::Ignored.as_str();
                    for chunk in bound.chunks(ROWS_PER_STATEMENT) {
                        let mut args = vec![SqlArg::Text(ignored.to_string())];
                        for (id, updated_at) in chunk {
                            args.push(SqlArg::Text(id.clone()));
                            args.push(updated_at.clone());
                        }
                        args.push(SqlArg::Text(ignored.to_string()));
                        args.extend(chunk.iter().map(|(id, _)| SqlArg::Text(id.clone())));
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            &format!(
                                "UPDATE library_scan_unmatched_items
                                    SET status = {{}},
                                        updated_at = CASE id {} END
                                  WHERE status <> {{}} AND id IN ({})",
                                std::iter::repeat_n("WHEN {} THEN {}", chunk.len())
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                std::iter::repeat_n("{}", chunk.len())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            &args,
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await?;
        Ok(Some(()))
    }
}

/// Ids per statement once an id list is too long for one.
const UNMATCHED_ID_LIST_BIND_CHUNK: usize = 1_000;

/// The `WHERE` clause and its binds for
/// `page_library_scan_items_requiring_action`: the facet and status the
/// `(facet, status, updated_at)` index serves, the caller's libraries (a row
/// without one is in the facet's default library), and the requires-action
/// predicate `count_pending_library_scan_items_requiring_action` counts.
fn requiring_action_page_filter(
    facet: &MediaFacet,
    status: &PendingImportStatus,
    library_ids: &[String],
    folder_ownership_reason_codes: &[&str],
) -> (String, Vec<SqlArg>) {
    let placeholders = |count: usize| {
        std::iter::repeat_n("{}", count)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut args = vec![
        SqlArg::Text(facet.as_str().to_string()),
        SqlArg::Text(status.as_str().to_string()),
    ];
    args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    let default_library = scryer_domain::default_library_id_for_facet(facet);
    let null_library_sql = if library_ids.contains(&default_library) {
        " OR library_id IS NULL"
    } else {
        ""
    };
    let reason_sql = if folder_ownership_reason_codes.is_empty() {
        "1 = 0".to_string()
    } else {
        args.extend(
            folder_ownership_reason_codes
                .iter()
                .map(|code| SqlArg::Text((*code).to_string())),
        );
        format!(
            "reason_code IN ({})",
            placeholders(folder_ownership_reason_codes.len())
        )
    };
    args.push(SqlArg::Text(MediaFacet::Movie.as_str().to_string()));
    (
        format!(
            "facet = {{}} AND status = {{}}
               AND (library_id IN ({}){null_library_sql})
               AND ({reason_sql} OR NOT (facet = {{}} AND title_id IS NOT NULL))",
            placeholders(library_ids.len())
        ),
        args,
    )
}

fn build_unmatched_list_sql(
    facet: Option<MediaFacet>,
    scan_root: Option<&str>,
    status: Option<PendingImportStatus>,
    limit_offset: Option<(i64, i64)>,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!(
        "SELECT {UNMATCHED_COLUMNS}
         FROM library_scan_unmatched_items
         WHERE 1=1"
    );
    let mut args = push_unmatched_filters(&mut sql, facet, scan_root, status);
    sql.push_str(" ORDER BY updated_at DESC, item_path ASC");
    if let Some((limit, offset)) = limit_offset {
        sql.push_str(" LIMIT {} OFFSET {}");
        args.push(SqlArg::I64(limit));
        args.push(SqlArg::I64(offset));
    }
    (sql, args)
}

fn build_unmatched_count_sql(
    facet: Option<MediaFacet>,
    scan_root: Option<&str>,
    status: Option<PendingImportStatus>,
) -> (String, Vec<SqlArg>) {
    let mut sql = "SELECT COUNT(*) AS count
         FROM library_scan_unmatched_items
         WHERE 1=1"
        .to_string();
    let args = push_unmatched_filters(&mut sql, facet, scan_root, status);
    (sql, args)
}

fn push_unmatched_filters(
    sql: &mut String,
    facet: Option<MediaFacet>,
    scan_root: Option<&str>,
    status: Option<PendingImportStatus>,
) -> Vec<SqlArg> {
    let mut args = Vec::new();
    if let Some(facet) = facet {
        sql.push_str(" AND facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    if let Some(scan_root) = scan_root {
        sql.push_str(" AND scan_root = {}");
        args.push(SqlArg::Text(scan_root.to_string()));
    }
    if let Some(status) = status {
        sql.push_str(" AND status = {}");
        args.push(SqlArg::Text(status.as_str().to_string()));
    }
    args
}

async fn fetch_unmatched_items(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_library_scan_unmatched_item)
        .collect()
}

async fn fetch_optional_unmatched_item(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<LibraryScanUnmatchedItem>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_library_scan_unmatched_item)
        .transpose()
}

fn row_to_library_scan_unmatched_item(row: &SqlRow) -> AppResult<LibraryScanUnmatchedItem> {
    let facet_raw = row.text("facet")?;
    let facet = MediaFacet::parse(&facet_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "library scan unmatched item has invalid facet '{facet_raw}'"
        ))
    })?;
    let status_raw = row.text("status")?;
    let status = PendingImportStatus::parse(&status_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "library scan unmatched item has invalid status '{status_raw}'"
        ))
    })?;
    let search_attempts_json = json_text_or(row, "search_attempts_json", "[]")?;
    let search_attempts =
        serde_json::from_str::<Vec<LibraryScanUnmatchedSearchAttempt>>(&search_attempts_json)
            .map_err(repo_err)?;

    Ok(LibraryScanUnmatchedItem {
        id: row.text("id")?,
        library_id: row
            .opt_text("library_id")?
            .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet)),
        facet,
        status,
        title_id: row.opt_text("title_id")?,
        scan_session_id: row.text("scan_session_id")?,
        scan_root: row.text("scan_root")?,
        item_path: row.text("item_path")?,
        display_name: row.text("display_name")?,
        query: row.text("query")?,
        year_hint: row.opt_i32("year_hint")?,
        reason_code: row.text("reason_code")?,
        error_message: row.opt_text("error_message")?,
        search_attempts,
        size_bytes: row.opt_i64("size_bytes")?,
        created_at: timestamp_text(row, "created_at")?,
        updated_at: timestamp_text(row, "updated_at")?,
    })
}

fn timestamp_arg_for_datastore(datastore: &StoreDatastore, value: &str) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::Text(value.to_string())),
        StoreDatastore::Postgres { .. } => parse_utc_datetime(value).map(SqlArg::Timestamp),
    }
}

fn timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: impl Into<String>,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    let sql = sql.into();
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use scryer_application::{
        PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES, pending_import_item_requires_action,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn test_store() -> LibraryScanUnmatchedStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .expect("fresh migrations should apply");
        LibraryScanUnmatchedStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    async fn seed_row(
        store: &LibraryScanUnmatchedStore,
        id: &str,
        library_id: Option<&str>,
        facet: &str,
        status: &str,
        title_id: Option<&str>,
        reason_code: &str,
    ) {
        execute_write(
            &store.datastore,
            "seed_library_scan_unmatched_item",
            "INSERT INTO library_scan_unmatched_items
             (id, library_id, facet, title_id, scan_session_id, scan_root, item_path,
              display_name, query, reason_code, search_attempts_json, status,
              created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(id.to_string()),
                SqlArg::OptText(library_id.map(str::to_string)),
                SqlArg::Text(facet.to_string()),
                SqlArg::OptText(title_id.map(str::to_string)),
                SqlArg::Text("scan-session".to_string()),
                SqlArg::Text("/library/root".to_string()),
                SqlArg::Text(format!("/library/root/{id}")),
                SqlArg::Text("Emberfall".to_string()),
                SqlArg::Text("Emberfall".to_string()),
                SqlArg::Text(reason_code.to_string()),
                SqlArg::Text("[]".to_string()),
                SqlArg::Text(status.to_string()),
                SqlArg::Text("2026-01-01T00:00:00Z".to_string()),
                SqlArg::Text("2026-01-01T00:00:00Z".to_string()),
            ],
        )
        .await
        .expect("seed unmatched item");
    }

    /// The previous badge path: list every pending item per facet and keep
    /// the ones the Rust predicate says need a person.
    async fn counts_via_rust_predicate(
        store: &LibraryScanUnmatchedStore,
    ) -> BTreeMap<(String, String), i64> {
        let mut counts = BTreeMap::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let items = store
                .list_library_scan_unmatched_items(
                    Some(facet.clone()),
                    None,
                    Some(PendingImportStatus::Pending),
                    i64::MAX,
                    0,
                )
                .await
                .expect("list pending items");
            for item in items
                .iter()
                .filter(|item| pending_import_item_requires_action(item))
            {
                *counts
                    .entry((item.library_id.clone(), facet.as_str().to_string()))
                    .or_insert(0) += 1;
            }
        }
        counts
    }

    async fn counts_via_sql(store: &LibraryScanUnmatchedStore) -> BTreeMap<(String, String), i64> {
        let mut counts = BTreeMap::new();
        for (library_id, facet, count) in store
            .count_pending_library_scan_items_requiring_action(
                &PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES,
            )
            .await
            .expect("grouped count")
            .expect("sql store counts in place")
        {
            *counts
                .entry((library_id, facet.as_str().to_string()))
                .or_insert(0) += count;
        }
        counts
    }

    #[tokio::test]
    async fn grouped_requires_action_count_matches_rust_predicate() {
        let store = test_store().await;
        assert!(counts_via_sql(&store).await.is_empty());
        assert_eq!(
            counts_via_sql(&store).await,
            counts_via_rust_predicate(&store).await
        );

        let ownership = PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES;
        let mut n = 0;
        for library in [Some("lib-cinderwake"), Some("lib-ashgrove"), None] {
            for facet in ["movie", "series", "anime", "music"] {
                for status in ["pending", "ignored"] {
                    for title_id in [None, Some(""), Some("title-emberfall")] {
                        for reason in [ownership[0], ownership[1], "no_match", ""] {
                            n += 1;
                            seed_row(
                                &store,
                                &format!("item-{n:04}"),
                                library,
                                facet,
                                status,
                                title_id,
                                reason,
                            )
                            .await;
                        }
                    }
                }
            }
        }

        let via_sql = counts_via_sql(&store).await;
        let via_rust = counts_via_rust_predicate(&store).await;
        assert_eq!(via_sql, via_rust);
        // Every branch is represented: movies bound to a title only count
        // for ownership codes, NULL libraries fall back to the default.
        assert_eq!(
            via_rust.get(&("lib-cinderwake".to_string(), "movie".to_string())),
            Some(&(3 * 4 - 2 * 2))
        );
        assert_eq!(
            via_rust.get(&("lib-cinderwake".to_string(), "series".to_string())),
            Some(&12)
        );
        assert!(via_rust.keys().any(|(library_id, _)| library_id
            == &scryer_domain::default_library_id_for_facet(&MediaFacet::Anime)));
    }

    /// Seeds the requires-action matrix with distinct `updated_at` values and
    /// some shared ones, so the page order's tie-break on `item_path` is
    /// exercised too.
    async fn seed_page_matrix(store: &LibraryScanUnmatchedStore) {
        let ownership = PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES;
        let mut n = 0;
        for library in [Some("lib-cinderwake"), Some("lib-ashgrove"), None] {
            for facet in ["movie", "series", "anime"] {
                for status in ["pending", "ignored"] {
                    for title_id in [None, Some("title-emberfall")] {
                        for reason in [ownership[0], "no_match", ""] {
                            n += 1;
                            let id = format!("item-{n:04}");
                            seed_row(store, &id, library, facet, status, title_id, reason).await;
                            let minute = n % 7;
                            execute_write(
                                &store.datastore,
                                "seed_updated_at",
                                "UPDATE library_scan_unmatched_items SET updated_at = {} WHERE id = {}",
                                vec![
                                    SqlArg::Text(format!("2026-01-01T00:{minute:02}:00Z")),
                                    SqlArg::Text(id),
                                ],
                            )
                            .await
                            .expect("set updated_at");
                        }
                    }
                }
            }
        }
    }

    /// The previous pending-imports page: every row of the facet and status,
    /// filtered and paged in memory.
    async fn page_via_rust(
        store: &LibraryScanUnmatchedStore,
        facet: MediaFacet,
        status: PendingImportStatus,
        library_ids: &[String],
        limit: i64,
        offset: i64,
    ) -> (i64, Vec<String>) {
        let filtered = store
            .list_library_scan_unmatched_items(Some(facet), None, Some(status), i64::MAX, 0)
            .await
            .expect("list items")
            .into_iter()
            .filter(|item| {
                library_ids.contains(&item.library_id) && pending_import_item_requires_action(item)
            })
            .collect::<Vec<_>>();
        let total = filtered.len() as i64;
        let ids = filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|item| item.id)
            .collect();
        (total, ids)
    }

    #[tokio::test]
    async fn requiring_action_page_matches_in_memory_paging() {
        let store = test_store().await;
        seed_page_matrix(&store).await;

        let default_movie = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
        let default_series = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
        let library_sets = [
            vec!["lib-cinderwake".to_string()],
            vec!["lib-ashgrove".to_string(), "lib-cinderwake".to_string()],
            vec![default_movie.clone(), default_series.clone()],
            vec![
                default_movie,
                default_series,
                "lib-ashgrove".to_string(),
                "lib-cinderwake".to_string(),
            ],
            vec!["lib-absent".to_string()],
        ];
        let mut compared = 0;
        let mut non_empty = 0;
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            for status in [PendingImportStatus::Pending, PendingImportStatus::Ignored] {
                for library_ids in &library_sets {
                    for (limit, offset) in [(50, 0), (2, 0), (2, 1), (3, 4), (0, 0), (5, 100)] {
                        let (total, items) = store
                            .page_library_scan_items_requiring_action(
                                facet.clone(),
                                status,
                                library_ids,
                                &PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES,
                                limit,
                                offset,
                            )
                            .await
                            .expect("page items")
                            .expect("sql store pages in place");
                        let via_sql = (total, items.into_iter().map(|item| item.id).collect());
                        let via_rust = page_via_rust(
                            &store,
                            facet.clone(),
                            status,
                            library_ids,
                            limit,
                            offset,
                        )
                        .await;
                        assert_eq!(
                            via_sql, via_rust,
                            "{facet:?} {status:?} {library_ids:?} limit {limit} offset {offset}"
                        );
                        compared += 1;
                        non_empty += usize::from(!via_rust.1.is_empty());
                    }
                }
            }
        }
        assert_eq!(compared, 3 * 2 * 5 * 6);
        assert!(
            non_empty > 40,
            "matrix must exercise real pages: {non_empty}"
        );

        let (total, items) = store
            .page_library_scan_items_requiring_action(
                MediaFacet::Movie,
                PendingImportStatus::Pending,
                &[],
                &PENDING_IMPORT_FOLDER_OWNERSHIP_REASON_CODES,
                50,
                0,
            )
            .await
            .expect("page items")
            .expect("sql store pages in place");
        assert_eq!((total, items.len()), (0, 0));
    }

    #[tokio::test]
    async fn batched_unmatched_read_matches_reads_by_id() {
        let store = test_store().await;
        seed_page_matrix(&store).await;
        let mut ids = (1..=40).map(|n| format!("item-{n:04}")).collect::<Vec<_>>();
        ids.push("item-absent".to_string());

        let mut via_batch = store
            .get_library_scan_unmatched_items(&ids)
            .await
            .expect("batched read");
        via_batch.sort_by(|left, right| left.id.cmp(&right.id));
        let mut via_single = Vec::new();
        for id in &ids {
            if let Some(item) = store
                .get_library_scan_unmatched_item(id)
                .await
                .expect("single read")
            {
                via_single.push(item);
            }
        }
        via_single.sort_by(|left, right| left.id.cmp(&right.id));
        assert_eq!(via_batch.len(), 40);
        assert_eq!(format!("{via_batch:?}"), format!("{via_single:?}"));
    }

    async fn all_rows(store: &LibraryScanUnmatchedStore) -> Vec<LibraryScanUnmatchedItem> {
        let mut rows = store
            .list_library_scan_unmatched_items(None, None, None, i64::MAX, 0)
            .await
            .expect("list rows");
        rows.sort_by(|left, right| left.id.cmp(&right.id));
        rows
    }

    #[tokio::test]
    async fn batched_ignore_writes_the_rows_a_serial_upsert_writes() {
        let batched = test_store().await;
        let serial = test_store().await;
        seed_page_matrix(&batched).await;
        seed_page_matrix(&serial).await;

        // Pending rows only: the service skips the write for an ignored row.
        let targets = all_rows(&serial)
            .await
            .into_iter()
            .filter(|item| item.status == PendingImportStatus::Pending)
            .take(25)
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 25);
        let rows = targets
            .iter()
            .enumerate()
            .map(|(k, item)| (item.id.clone(), format!("2026-02-01T00:00:{k:02}+00:00")))
            .collect::<Vec<_>>();

        for (item, (_, updated_at)) in targets.iter().zip(&rows) {
            let mut item = item.clone();
            item.status = PendingImportStatus::Ignored;
            item.updated_at = updated_at.clone();
            serial
                .upsert_library_scan_unmatched_item(&item)
                .await
                .expect("serial upsert");
        }
        assert_eq!(
            batched
                .ignore_library_scan_unmatched_items(&rows)
                .await
                .expect("batched ignore"),
            Some(())
        );

        let after_batched = all_rows(&batched).await;
        let after_serial = all_rows(&serial).await;
        assert_eq!(format!("{after_batched:?}"), format!("{after_serial:?}"));
        assert_eq!(
            after_batched
                .iter()
                .filter(|item| item.status == PendingImportStatus::Ignored)
                .count(),
            after_serial
                .iter()
                .filter(|item| item.status == PendingImportStatus::Ignored)
                .count()
        );
    }
}
