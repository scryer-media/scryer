use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
    LibraryScanUnmatchedSearchAttempt, PendingImportStatus,
};
use scryer_domain::MediaFacet;

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
}
