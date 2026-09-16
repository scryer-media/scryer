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
