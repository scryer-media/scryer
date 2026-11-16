use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use scryer_application::{
    AppError, AppResult, BACKUP_TABLE_CATALOG, BLOB_MARKER_BASE64, BLOB_MARKER_TYPE,
    BackupBundleExportRequest, BackupBundleStaging, BackupRestorePreparedBundle,
    BackupTableClassification, EXPORT_BATCH_SIZE, LogicalBackupExporter,
    backup_table_part_filename, prepare_backup_restore_payload,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row, TypeInfo, ValueRef};

use crate::backup_import_normalization::{
    ImportColumnKind, ImportColumnRule, InsertStatements, RestoreRowReader,
    strip_nonportable_backup_fields, validate_restore_manifest_table_set,
};

#[derive(Clone, Debug)]
pub struct SqliteLogicalBackupExporter {
    db_path: String,
}

#[derive(Clone, Debug)]
struct SqliteForeignKeyViolation {
    table: String,
    rowid: Option<i64>,
    parent: String,
    fkid: i64,
}

impl SqliteLogicalBackupExporter {
    pub fn new(db_path: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }
}

#[async_trait]
impl LogicalBackupExporter for SqliteLogicalBackupExporter {
    async fn export_backup_bundle(
        &self,
        request: BackupBundleExportRequest,
    ) -> AppResult<scryer_application::BackupExportOutcome> {
        export_backup_bundle_from_sqlite(&self.db_path, request).await
    }
}

pub async fn export_backup_bundle_from_sqlite(
    db_path: &str,
    request: BackupBundleExportRequest,
) -> AppResult<scryer_application::BackupExportOutcome> {
    let mut staging = BackupBundleStaging::new()?;

    let mut connect_options = db_connect_options(db_path)?;
    connect_options = connect_options.read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to open source database for backup: {error}"
            ))
        })?;

    let export_result = export_backup_tables_from_pool(&pool, &mut staging).await;
    pool.close().await;
    export_result?;

    staging.finish_off_runtime(request).await
}

async fn export_backup_tables_from_pool(
    pool: &sqlx::SqlitePool,
    staging: &mut BackupBundleStaging,
) -> AppResult<()> {
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let tables_dir = staging.tables_dir();

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to acquire source database connection: {error}"
        ))
    })?;
    sqlx::query("BEGIN")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to begin backup snapshot: {error}"))
        })?;

    let mut export_result = Ok(());
    for table in &export_tables {
        let table_result = async {
            let (row_count, checksum) = export_table_part(&mut conn, table, &tables_dir).await?;
            staging.record_table_part(table, row_count, checksum)
        }
        .await;

        if let Err(error) = table_result {
            export_result = Err(error);
            break;
        }
    }

    let rollback_result = sqlx::query("ROLLBACK")
        .execute(&mut *conn)
        .await
        .map(|_| ())
        .map_err(|error| AppError::Repository(format!("failed to close backup snapshot: {error}")));

    match (export_result, rollback_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

pub async fn restore_backup_bundle_into_sqlite_pool(
    pool: &sqlx::SqlitePool,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupRestorePreparedBundle> {
    let payload = prepare_backup_restore_payload(bundle_path, passphrase)?;
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let restore_tables = ordered_restore_tables(pool).await?;
    validate_restore_manifest_table_set(
        &payload.manifest().row_counts,
        &export_tables,
        payload.manifest().source_migration_key.as_deref(),
    )?;

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!("failed to acquire restore connection: {error}"))
    })?;

    if let Err(error) = sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await {
        return Err(AppError::Repository(format!(
            "failed to begin SQLite restore transaction: {error}"
        )));
    }

    let tables_dir = payload.tables_dir();
    let restore_work = async {
        sqlx::query("PRAGMA defer_foreign_keys = ON")
            .execute(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to defer SQLite foreign keys for restore: {error}"
                ))
            })?;

        for table in restore_tables.iter().rev() {
            let sql = format!("DELETE FROM {}", quote_identifier(table));
            sqlx::query(sqlx::AssertSqlSafe(&*sql))
                .execute(&mut *conn)
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to clear restore table {table}: {error}"))
                })?;
        }

        let progress = scryer_application::current_restore_deadline();
        if let Some(progress) = &progress {
            progress.begin_import(
                payload
                    .manifest()
                    .row_counts
                    .values()
                    .copied()
                    .fold(0_u64, u64::saturating_add),
            );
        }
        for table in export_tables
            .iter()
            .filter(|table| payload.manifest().row_counts.contains_key(*table))
        {
            import_table_part(
                &mut conn,
                table,
                &tables_dir.join(backup_table_part_filename(table)),
            )
            .await?;
        }

        if let Some(progress) = &progress {
            progress.finish_import();
        }
        let violations = foreign_key_violations(&mut conn).await?;
        if !violations.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::DATASTORE_BACKUP_FOREIGN_KEY_INVALID,
                format!(
                    "restored database failed foreign key validation: {}",
                    format_foreign_key_violations(&violations)
                ),
            ));
        }

        for table in export_tables
            .iter()
            .filter(|table| payload.manifest().row_counts.contains_key(*table))
        {
            let expected_rows = payload.manifest().row_counts.get(table).ok_or_else(|| {
                AppError::validation_refused(crate::error_reasons::DATASTORE_BACKUP_TABLE_SET_MISMATCH, format!(
                    "backup bundle table set does not match the current restore catalog: missing [{}], unexpected []",
                    table
                ))
            })?;
            let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
            let actual_rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(&*sql))
                .fetch_one(&mut *conn)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to validate restored table {table}: {error}"
                    ))
                })?;
            if actual_rows as u64 != *expected_rows {
                return Err(AppError::validation_refused(
                    crate::error_reasons::DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH,
                    format!(
                        "restored table {table} row count mismatch: expected {expected_rows}, got {actual_rows}"
                    ),
                ));
            }
        }

        crate::queries::title_search::rebuild_title_search_projection_on_connection(&mut conn)
            .await?;
        scryer_application::check_restore_deadline()?;
        AppResult::Ok(())
    };
    let restore_result = match scryer_application::current_restore_deadline() {
        Some(deadline) => deadline.run(restore_work).await,
        None => restore_work.await,
    };

    let transaction_result = match restore_result {
        Ok(()) => match sqlx::query("COMMIT").execute(&mut *conn).await {
            Ok(_) => Ok(()),
            Err(commit_error) => {
                let rollback_result = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                match rollback_result {
                    Ok(_) => Err(AppError::Repository(format!(
                        "failed to commit SQLite restore: {commit_error}"
                    ))),
                    Err(rollback_error) => Err(AppError::Repository(format!(
                        "failed to commit SQLite restore: {commit_error}; rollback also failed: {rollback_error}"
                    ))),
                }
            }
        },
        Err(error) => {
            let rollback_result = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            match rollback_result {
                Ok(_) => Err(error),
                Err(rollback_error) => Err(AppError::Repository(format!(
                    "SQLite restore failed: {error}; rollback also failed: {rollback_error}"
                ))),
            }
        }
    };

    transaction_result?;

    Ok(
        BackupRestorePreparedBundle::from_summary_and_instance_secrets_env(
            payload.summary(),
            payload.instance_secrets_env()?,
        ),
    )
}

fn db_connect_options(db_path: &str) -> AppResult<SqliteConnectOptions> {
    db_path.parse::<SqliteConnectOptions>().map_err(|error| {
        AppError::Repository(format!("invalid sqlite database path {db_path}: {error}"))
    })
}

async fn validate_backup_catalog(pool: &sqlx::SqlitePool) -> AppResult<()> {
    let actual_tables = application_tables(pool).await?;
    let mut classified = BTreeSet::new();
    for entry in BACKUP_TABLE_CATALOG {
        classified.insert(entry.table.to_string());
    }

    let unclassified = actual_tables
        .into_iter()
        .filter(|table| !classified.contains(table))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        return Err(AppError::Repository(format!(
            "backup catalog is missing classifications for tables: {}",
            unclassified.join(", ")
        )));
    }

    Ok(())
}

async fn application_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
          ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Repository(format!("failed to inspect sqlite schema: {error}")))?;

    let mut tables = Vec::new();
    for row in rows {
        let table: String = row.try_get("name").map_err(|error| {
            AppError::Repository(format!("failed to decode sqlite schema row: {error}"))
        })?;
        if is_engine_internal_table(&table) {
            continue;
        }
        tables.push(table);
    }
    Ok(tables)
}

fn is_engine_internal_table(table: &str) -> bool {
    table.starts_with("sqlite_")
}

async fn ordered_export_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    ordered_catalog_tables(pool, &[BackupTableClassification::Export]).await
}

async fn ordered_restore_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    ordered_catalog_tables(
        pool,
        &[
            BackupTableClassification::Export,
            BackupTableClassification::ResetOnRestore,
        ],
    )
    .await
}

async fn ordered_catalog_tables(
    pool: &sqlx::SqlitePool,
    classifications: &[BackupTableClassification],
) -> AppResult<Vec<String>> {
    let catalog_tables = BACKUP_TABLE_CATALOG
        .iter()
        .filter(|entry| classifications.contains(&entry.classification))
        .map(|entry| entry.table.to_string())
        .collect::<BTreeSet<_>>();

    let mut incoming = BTreeMap::<String, usize>::new();
    let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
    for table in &catalog_tables {
        incoming.insert(table.clone(), 0);
        outgoing.insert(table.clone(), BTreeSet::new());
    }

    for table in &catalog_tables {
        let pragma = format!("PRAGMA foreign_key_list({})", quote_identifier(table));
        let rows = sqlx::query(sqlx::AssertSqlSafe(&*pragma))
            .fetch_all(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign keys for {table}: {error}"
                ))
            })?;
        for row in rows {
            let referenced: String = row.try_get("table").map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign key for {table}: {error}"
                ))
            })?;
            if !catalog_tables.contains(&referenced) {
                continue;
            }
            if referenced == *table {
                continue;
            }
            if outgoing
                .get_mut(&referenced)
                .expect("known table")
                .insert(table.clone())
            {
                *incoming.get_mut(table).expect("known table") += 1;
            }
        }
    }

    let mut ready = incoming
        .iter()
        .filter_map(|(table, count)| (*count == 0).then_some(table.clone()))
        .collect::<VecDeque<_>>();
    let mut ordered = Vec::new();

    while let Some(table) = ready.pop_front() {
        ordered.push(table.clone());
        let dependents = outgoing.get(&table).cloned().unwrap_or_default();
        for dependent in dependents {
            let count = incoming.get_mut(&dependent).expect("known dependent");
            *count -= 1;
            if *count == 0 {
                let insert_at = ready
                    .iter()
                    .position(|candidate| candidate > &dependent)
                    .unwrap_or(ready.len());
                ready.insert(insert_at, dependent.clone());
            }
        }
    }

    if ordered.len() != catalog_tables.len() {
        return Err(AppError::Repository(
            "backup catalog dependencies contain a cycle".into(),
        ));
    }

    Ok(ordered)
}

/// One table part being written: the JSON-lines file, its running checksum
/// and row count. Encoding and file writes run on the blocking pool, one page
/// at a time, so a large table never holds an async worker.
pub(crate) struct TablePartWriter {
    table: String,
    writer: BufWriter<File>,
    hasher: blake3::Hasher,
    line_buffer: Vec<u8>,
    count: u64,
}

impl TablePartWriter {
    pub(crate) async fn create(
        table: &str,
        tables_dir: &Path,
        create_label: &'static str,
    ) -> AppResult<Self> {
        let table = table.to_string();
        let output_path = tables_dir.join(backup_table_part_filename(&table));
        run_export_blocking(move || {
            let file = File::create(&output_path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to create {create_label} {}: {error}",
                    output_path.display()
                ))
            })?;
            Ok(Self {
                table,
                writer: BufWriter::new(file),
                hasher: blake3::Hasher::new(),
                line_buffer: Vec::with_capacity(16 * 1024),
                count: 0,
            })
        })
        .await
    }

    /// Encode and append one page of rows. `encoded_columns` leaves out any
    /// trailing columns selected only to page by.
    pub(crate) async fn write_page<R: Send + 'static>(
        mut self,
        rows: Vec<R>,
        encode: fn(&R, usize) -> AppResult<JsonValue>,
        encoded_columns: usize,
    ) -> AppResult<Self> {
        run_export_blocking(move || {
            for row in &rows {
                self.write_row(encode(row, encoded_columns)?)?;
            }
            Ok(self)
        })
        .await
    }

    fn write_row(&mut self, mut value: JsonValue) -> AppResult<()> {
        let table = self.table.as_str();
        if let JsonValue::Object(object) = &mut value {
            strip_nonportable_backup_fields(table, object);
        }
        self.line_buffer.clear();
        serde_json::to_writer(&mut self.line_buffer, &value).map_err(|error| {
            AppError::Repository(format!("failed to encode backup row for {table}: {error}"))
        })?;
        self.line_buffer.push(b'\n');
        self.writer.write_all(&self.line_buffer).map_err(|error| {
            AppError::Repository(format!("failed to write backup row for {table}: {error}"))
        })?;
        self.hasher.update(&self.line_buffer);
        self.count += 1;
        Ok(())
    }

    pub(crate) async fn finish(self) -> AppResult<(u64, String)> {
        run_export_blocking(move || {
            let Self {
                table,
                mut writer,
                hasher,
                count,
                ..
            } = self;
            writer.flush().map_err(|error| {
                AppError::Repository(format!("failed to flush table export for {table}: {error}"))
            })?;
            Ok((count, hasher.finalize().to_hex().to_string()))
        })
        .await
    }
}

async fn run_export_blocking<T: Send + 'static>(
    work: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => Err(AppError::Repository(format!(
            "backup table export task failed: {error}"
        ))),
    }
}

/// How a table's rows are paged for export, always in `order_by` order.
struct TableExportOrder {
    order_by: String,
    paging: ExportPaging,
}

enum ExportPaging {
    /// The ordering is a unique, non-null key in this snapshot, so each page
    /// resumes after the previous page's last key. `rowid` means the key is
    /// the implicit rowid, selected as one extra trailing column.
    Keyset { key: Vec<String>, rowid: bool },
    /// The ordering is not a unique non-null key (a plain `id` column, a
    /// primary key holding NULLs, or a column shadowing `rowid`), so pages
    /// are read with `LIMIT … OFFSET …` exactly as before.
    Offset,
}

async fn export_table_part(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    tables_dir: &Path,
) -> AppResult<(u64, String)> {
    let order = table_export_order(conn, table).await?;
    let mut part = TablePartWriter::create(table, tables_dir, "table export").await?;
    match order.paging {
        ExportPaging::Offset => {
            let paged_sql = format!(
                "{} LIMIT ? OFFSET ?",
                select_in_order(table, "*", None, &order.order_by)
            );
            let mut offset = 0_i64;
            loop {
                let rows = sqlx::query(sqlx::AssertSqlSafe(&*paged_sql))
                    .bind(EXPORT_BATCH_SIZE)
                    .bind(offset)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(|error| export_query_error(table, error))?;
                if rows.is_empty() {
                    break;
                }
                offset += rows.len() as i64;
                let columns = rows[0].columns().len();
                part = part.write_page(rows, encode_row_columns, columns).await?;
            }
        }
        ExportPaging::Keyset { key, rowid } => {
            let selected = if rowid { "*, rowid" } else { "*" };
            let first_sql = format!(
                "{} LIMIT ?",
                select_in_order(table, selected, None, &order.order_by)
            );
            let key_list = key
                .iter()
                .map(|column| quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = vec!["?"; key.len()].join(", ");
            let after = format!("({key_list}) > ({placeholders})");
            let next_sql = format!(
                "{} LIMIT ?",
                select_in_order(table, selected, Some(&after), &order.order_by)
            );
            let mut last_key: Option<Vec<SqliteKeyValue>> = None;
            loop {
                let rows = match &last_key {
                    None => {
                        sqlx::query(sqlx::AssertSqlSafe(&*first_sql))
                            .bind(EXPORT_BATCH_SIZE)
                            .fetch_all(&mut *conn)
                            .await
                    }
                    Some(values) => {
                        let mut query = sqlx::query(sqlx::AssertSqlSafe(&*next_sql));
                        for value in values {
                            query = value.bind_to(query);
                        }
                        query.bind(EXPORT_BATCH_SIZE).fetch_all(&mut *conn).await
                    }
                }
                .map_err(|error| export_query_error(table, error))?;
                let Some(last) = rows.last() else {
                    break;
                };
                let total_columns = last.columns().len();
                let columns = if rowid {
                    total_columns - 1
                } else {
                    total_columns
                };
                last_key = Some(if rowid {
                    vec![SqliteKeyValue::read(last, total_columns - 1, "rowid")?]
                } else {
                    key.iter()
                        .map(|column| {
                            let index = last
                                .try_column(column.as_str())
                                .map_err(|error| {
                                    AppError::Repository(format!(
                                        "failed to read backup key column {column}: {error}"
                                    ))
                                })?
                                .ordinal();
                            SqliteKeyValue::read(last, index, column)
                        })
                        .collect::<AppResult<Vec<_>>>()?
                });
                part = part.write_page(rows, encode_row_columns, columns).await?;
            }
        }
    }
    part.finish().await
}

fn select_in_order(table: &str, selected: &str, filter: Option<&str>, order_by: &str) -> String {
    let mut sql = format!("SELECT {selected} FROM {}", quote_identifier(table));
    if let Some(filter) = filter {
        sql.push_str(" WHERE ");
        sql.push_str(filter);
    }
    if !order_by.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(order_by);
    }
    sql
}

fn export_query_error(table: &str, error: sqlx::Error) -> AppError {
    AppError::Repository(format!("failed to export table {table}: {error}"))
}

/// One key value of the last exported row, kept in its stored class so the
/// next page's comparison follows the same ordering as `ORDER BY`.
enum SqliteKeyValue {
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqliteKeyValue {
    fn read(row: &SqliteRow, index: usize, column: &str) -> AppResult<Self> {
        let key_error = |error: sqlx::Error| {
            AppError::Repository(format!(
                "failed to read backup key column {column}: {error}"
            ))
        };
        let raw = row.try_get_raw(index).map_err(key_error)?;
        if raw.is_null() {
            return Err(AppError::Repository(format!(
                "backup key column {column} is unexpectedly NULL"
            )));
        }
        Ok(match raw.type_info().name() {
            "INTEGER" => Self::Integer(row.try_get(index).map_err(key_error)?),
            "REAL" => Self::Real(row.try_get(index).map_err(key_error)?),
            "BLOB" => Self::Blob(row.try_get(index).map_err(key_error)?),
            _ => Self::Text(row.try_get(index).map_err(key_error)?),
        })
    }

    fn bind_to<'q>(
        &self,
        query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
        match self {
            Self::Integer(value) => query.bind(*value),
            Self::Real(value) => query.bind(*value),
            Self::Text(value) => query.bind(value.clone()),
            Self::Blob(value) => query.bind(value.clone()),
        }
    }
}

async fn table_export_order(
    executor: &mut sqlx::SqliteConnection,
    table: &str,
) -> AppResult<TableExportOrder> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*pragma))
        .fetch_all(&mut *executor)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to inspect table info for {table}: {error}"))
        })?;

    let mut pk_columns = rows
        .iter()
        .filter_map(|row| {
            let pk: i64 = row.try_get("pk").ok()?;
            let name: String = row.try_get("name").ok()?;
            let not_null = row.try_get::<i64, _>("notnull").ok() == Some(1);
            (pk > 0).then_some((pk, name, not_null))
        })
        .collect::<Vec<_>>();
    pk_columns.sort_by_key(|(pk, _, _)| *pk);

    if !pk_columns.is_empty() {
        let order_by = pk_columns
            .iter()
            .map(|(_, column, _)| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        // A primary key is unique, but SQLite lets a rowid table store NULL in
        // one that is not declared NOT NULL, and NULLs neither compare nor stay
        // unique. Such a table pages by offset only when it actually holds one.
        let nullable = pk_columns
            .iter()
            .filter(|(_, _, not_null)| !not_null)
            .map(|(_, column, _)| format!("{} IS NULL", quote_identifier(column)))
            .collect::<Vec<_>>();
        let holds_null = if nullable.is_empty() {
            false
        } else {
            let probe = format!(
                "SELECT EXISTS (SELECT 1 FROM {} WHERE {})",
                quote_identifier(table),
                nullable.join(" OR ")
            );
            sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(&*probe))
                .fetch_one(&mut *executor)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to inspect primary key values for {table}: {error}"
                    ))
                })?
        };
        let paging = if holds_null {
            ExportPaging::Offset
        } else {
            ExportPaging::Keyset {
                key: pk_columns
                    .into_iter()
                    .map(|(_, column, _)| column)
                    .collect(),
                rowid: false,
            }
        };
        return Ok(TableExportOrder { order_by, paging });
    }

    let column_names = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    if column_names.iter().any(|name| name == "id") {
        return Ok(TableExportOrder {
            order_by: quote_identifier("id"),
            paging: ExportPaging::Offset,
        });
    }

    let shadows_rowid = column_names.iter().any(|name| {
        ["rowid", "_rowid_", "oid"]
            .iter()
            .any(|alias| name.eq_ignore_ascii_case(alias))
    });
    Ok(TableExportOrder {
        order_by: "rowid".to_string(),
        paging: if shadows_rowid {
            ExportPaging::Offset
        } else {
            ExportPaging::Keyset {
                key: vec!["rowid".to_string()],
                rowid: true,
            }
        },
    })
}

#[cfg(test)]
fn encode_row(row: &SqliteRow) -> AppResult<JsonValue> {
    encode_row_columns(row, row.columns().len())
}

fn encode_row_columns(row: &SqliteRow, encoded_columns: usize) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate().take(encoded_columns) {
        let raw = row.try_get_raw(index).map_err(|error| {
            AppError::Repository(format!(
                "failed to read backup column {} from row: {error}",
                column.name()
            ))
        })?;

        let value = if raw.is_null() {
            JsonValue::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => JsonValue::from(row.try_get::<i64, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode integer column {}: {error}",
                        column.name()
                    ))
                })?),
                "REAL" => {
                    let value = row.try_get::<f64, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode real column {}: {error}",
                            column.name()
                        ))
                    })?;
                    JsonValue::from(value)
                }
                "BLOB" => {
                    encode_blob_value(&row.try_get::<Vec<u8>, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode blob column {}: {error}",
                            column.name()
                        ))
                    })?)
                }
                _ => JsonValue::String(row.try_get::<String, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode text column {}: {error}",
                        column.name()
                    ))
                })?),
            }
        };

        object.insert(column.name().to_string(), value);
    }

    Ok(JsonValue::Object(object))
}

fn encode_blob_value(bytes: &[u8]) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert(
        BLOB_MARKER_TYPE.to_string(),
        JsonValue::String("blob".to_string()),
    );
    object.insert(
        BLOB_MARKER_BASE64.to_string(),
        JsonValue::String(STANDARD.encode(bytes)),
    );
    JsonValue::Object(object)
}

async fn import_table_part(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
    part_path: &Path,
) -> AppResult<()> {
    let target_columns = table_columns(conn, table).await?;
    let mut rows = RestoreRowReader::open(table, part_path, target_columns.clone()).await?;
    let progress = scryer_application::current_restore_deadline();
    let mut statements = InsertStatements::new();
    let mut present = Vec::with_capacity(target_columns.len());

    loop {
        let batch = rows.next_batch().await?;
        if batch.is_empty() {
            return Ok(());
        }
        for row in batch {
            let (line_number, mut object) = row?;
            present.clear();
            present.extend(
                target_columns
                    .iter()
                    .enumerate()
                    .filter(|(_, column)| object.contains_key(column.name.as_str()))
                    .map(|(index, _)| index),
            );
            if present.is_empty() {
                continue;
            }

            let insert_sql = statements.get(&present, || {
                format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    quote_identifier(table),
                    present
                        .iter()
                        .map(|&index| quote_identifier(&target_columns[index].name))
                        .collect::<Vec<_>>()
                        .join(", "),
                    std::iter::repeat_n("?", present.len())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });

            let mut query = sqlx::query(sqlx::AssertSqlSafe(insert_sql));
            for &index in &present {
                let value = object
                    .remove(&target_columns[index].name)
                    .unwrap_or(JsonValue::Null);
                query = bind_json_value(query, value)?;
            }
            query.execute(&mut **conn).await.map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::DATASTORE_BACKUP_ROW_IMPORT_FAILED,
                    format!("failed to import backup row for {table}:{line_number}: {error}"),
                )
            })?;
            if let Some(progress) = &progress {
                progress.imported_row();
            }
        }
    }
}

async fn table_columns(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
) -> AppResult<Vec<ImportColumnRule>> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let table_rows = sqlx::query(sqlx::AssertSqlSafe(&*pragma))
        .fetch_all(&mut **conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect table columns for {table}: {error}"
            ))
        })?;

    let fk_pragma = format!("PRAGMA foreign_key_list({})", quote_identifier(table));
    let foreign_key_rows = sqlx::query(sqlx::AssertSqlSafe(&*fk_pragma))
        .fetch_all(&mut **conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect foreign keys for {table}: {error}"
            ))
        })?;
    let foreign_key_columns = foreign_key_rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("from").ok())
        .collect::<BTreeSet<_>>();

    Ok(table_rows
        .into_iter()
        .filter_map(|row| {
            let name = row.try_get::<String, _>("name").ok()?;
            let nullable = row.try_get::<i64, _>("notnull").ok()? == 0;
            Some(ImportColumnRule {
                has_default: row
                    .try_get::<Option<String>, _>("dflt_value")
                    .ok()
                    .flatten()
                    .is_some(),
                kind: ImportColumnKind::Generic,
                nullable_foreign_key: nullable && foreign_key_columns.contains(&name),
                nullable,
                name,
            })
        })
        .collect())
}

async fn foreign_key_violations(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
) -> AppResult<Vec<SqliteForeignKeyViolation>> {
    let rows = sqlx::query(
        "SELECT \"table\" AS table_name, rowid, parent, fkid FROM pragma_foreign_key_check",
    )
    .fetch_all(&mut **conn)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to validate restored foreign keys: {error}"))
    })?;
    rows.into_iter()
        .map(|row| {
            Ok(SqliteForeignKeyViolation {
                table: row.try_get("table_name").map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode SQLite foreign key check table: {error}"
                    ))
                })?,
                rowid: row.try_get("rowid").map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode SQLite foreign key check rowid: {error}"
                    ))
                })?,
                parent: row.try_get("parent").map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode SQLite foreign key check parent: {error}"
                    ))
                })?,
                fkid: row.try_get("fkid").map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode SQLite foreign key check fkid: {error}"
                    ))
                })?,
            })
        })
        .collect()
}

fn format_foreign_key_violations(violations: &[SqliteForeignKeyViolation]) -> String {
    const LIMIT: usize = 8;
    let sample = violations
        .iter()
        .take(LIMIT)
        .map(|violation| {
            let rowid = violation
                .rowid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!(
                "{} rowid {} -> {} (fk {})",
                violation.table, rowid, violation.parent, violation.fkid
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    if violations.len() > LIMIT {
        format!(
            "{}; and {} more violation(s)",
            sample,
            violations.len() - LIMIT
        )
    } else {
        sample
    }
}

fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    value: JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>> {
    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(if value { 1_i64 } else { 0_i64 }),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    AppError::validation_refused(
                        crate::error_reasons::DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE,
                        "backup row contains an integer outside SQLite i64 range",
                    )
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(AppError::validation_refused(
                    crate::error_reasons::DATASTORE_BACKUP_NUMBER_UNSUPPORTED,
                    "backup row contains an unsupported numeric value",
                ));
            }
        }
        JsonValue::String(value) => query.bind(value),
        JsonValue::Object(ref object)
            if object.get(BLOB_MARKER_TYPE).and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get(BLOB_MARKER_BASE64)
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::DATASTORE_BACKUP_BLOB_BYTES_MISSING,
                        "backup blob payload is missing base64 bytes",
                    )
                })?;
            let bytes = STANDARD.decode(encoded).map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::DATASTORE_BACKUP_BLOB_BASE64_INVALID,
                    format!("backup blob payload is invalid base64: {error}"),
                )
            })?;
            query.bind(bytes)
        }
        JsonValue::Array(_) | JsonValue::Object(_) => query.bind(value.to_string()),
    })
}

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use serde_json::{Map as JsonMap, Value as JsonValue};

    use std::collections::BTreeSet;

    use scryer_application::BACKUP_TABLE_CATALOG;

    use super::{
        SqliteForeignKeyViolation, application_tables, format_foreign_key_violations,
        validate_backup_catalog,
    };
    use crate::backup_import_normalization::{
        ImportColumnKind, ImportColumnRule, normalize_import_object_for_target,
    };

    #[tokio::test]
    async fn background_restore_import_percentage_counts_inserted_rows() {
        let directory = tempfile::tempdir().unwrap();
        let part = directory.path().join("rows.jsonl");
        std::fs::write(&part, "{\"id\":\"one\"}\n{\"id\":\"two\"}\n").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE restore_fixture (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        let deadline = scryer_application::RestoreDeadline::new();
        deadline.begin_import(4);
        deadline
            .scope(super::import_table_part(
                &mut connection,
                "restore_fixture",
                &part,
            ))
            .await
            .unwrap();
        assert_eq!(deadline.import_percent(), Some(50));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM restore_fixture")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(count, 2);
        drop(connection);
        pool.close().await;
    }

    const ROUND_TRIP_TABLE: &str = "CREATE TABLE restore_round_trip (
        id TEXT PRIMARY KEY,
        label TEXT,
        amount INTEGER,
        ratio REAL,
        payload BLOB,
        note TEXT DEFAULT 'synthetic-default',
        flag INTEGER NOT NULL DEFAULT 0
    )";

    async fn round_trip_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(ROUND_TRIP_TABLE).execute(&pool).await.unwrap();
        pool
    }

    async fn round_trip_rows(connection: &mut sqlx::SqliteConnection) -> Vec<JsonValue> {
        sqlx::query("SELECT * FROM restore_round_trip ORDER BY id")
            .fetch_all(connection)
            .await
            .unwrap()
            .iter()
            .map(|row| super::encode_row(row).unwrap())
            .collect()
    }

    /// Exporting a table and importing the part into an empty copy gives
    /// back every row and value, across several read batches and with rows
    /// that leave different columns to their defaults.
    #[tokio::test]
    async fn exported_rows_restore_unchanged_across_read_batches() {
        let source = round_trip_pool().await;
        for index in 0..600_i64 {
            let mut query = sqlx::query(if index % 3 == 0 {
                "INSERT INTO restore_round_trip (id, label, amount, ratio, payload) VALUES (?, ?, ?, ?, ?)"
            } else {
                "INSERT INTO restore_round_trip (id, label, amount, ratio, payload, note, flag) VALUES (?, ?, ?, ?, ?, ?, 1)"
            })
            .bind(format!("synthetic-{index:04}"))
            .bind((index % 5 != 0).then(|| format!("Synthetic \"label\" é {index}\t\\")))
            .bind(match index % 4 {
                0 => None,
                1 => Some(i64::MAX - index),
                2 => Some(-index),
                _ => Some(0),
            })
            .bind((index % 7 != 0).then_some(index as f64 / 3.0 - 50.0))
            .bind(match index % 6 {
                0 => None,
                1 => Some(Vec::new()),
                _ => Some((0..(index % 200) as u8).collect::<Vec<u8>>()),
            });
            if index % 3 != 0 {
                query = query.bind(if index % 2 == 0 { "" } else { "synthetic note" });
            }
            query.execute(&source).await.unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let mut source_connection = source.acquire().await.unwrap();
        let (exported, _) = super::export_table_part(
            &mut source_connection,
            "restore_round_trip",
            directory.path(),
        )
        .await
        .unwrap();
        assert_eq!(exported, 600);
        let expected = round_trip_rows(&mut source_connection).await;

        let target = round_trip_pool().await;
        let mut target_connection = target.acquire().await.unwrap();
        let part = directory
            .path()
            .join(scryer_application::backup_table_part_filename(
                "restore_round_trip",
            ));
        super::import_table_part(&mut target_connection, "restore_round_trip", &part)
            .await
            .unwrap();
        assert_eq!(round_trip_rows(&mut target_connection).await, expected);
    }

    /// A bad line stops the restore with the line's reason and number, after
    /// every row before it, and none after it, has been written.
    #[tokio::test]
    async fn a_bad_row_stops_the_restore_after_the_rows_before_it() {
        let directory = tempfile::tempdir().unwrap();
        let part = directory.path().join("rows.jsonl");
        let mut lines = Vec::new();
        for index in 0..400 {
            if index == 10 {
                lines.push(String::new());
            }
            if index == 300 {
                lines.push("{\"id\": ".to_string());
            }
            lines.push(if index % 2 == 0 {
                format!("{{\"id\":\"synthetic-{index:04}\",\"amount\":{index}}}")
            } else {
                format!("{{\"id\":\"synthetic-{index:04}\",\"flag\":1,\"note\":null}}")
            });
        }
        std::fs::write(&part, lines.join("\n")).unwrap();
        let pool = round_trip_pool().await;
        let mut connection = pool.acquire().await.unwrap();
        let error = super::import_table_part(&mut connection, "restore_round_trip", &part)
            .await
            .unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::DATASTORE_BACKUP_ROW_INVALID)
        );
        // Line 301 of the file, counting the blank line, is the bad one.
        assert!(
            error
                .to_string()
                .contains("invalid backup row for restore_round_trip:301:"),
            "{error}"
        );
        let rows = round_trip_rows(&mut connection).await;
        assert_eq!(rows.len(), 300);
        assert_eq!(rows[1]["note"], JsonValue::Null);
        assert_eq!(rows[0]["note"], JsonValue::from("synthetic-default"));
        assert_eq!(rows[299]["id"], JsonValue::from("synthetic-0299"));
    }

    /// Migrate a throwaway SQLite database to head and hand back its pool.
    ///
    /// The tempdir must outlive the pool, so it is returned alongside it.
    async fn migrated_sqlite_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("create migration tempdir");
        let db_path = temp.path().join("catalog-check.db");
        let services = scryer_infrastructure_datastore::SqliteServices::new_with_mode(
            format!("sqlite://{}", db_path.display()),
            crate::MigrationMode::Apply,
        )
        .await
        .expect("migrate sqlite schema to head");
        (services.pool().clone(), temp)
    }

    #[cfg(feature = "runtime-backups")]
    mod affinity_revision_restore;
    #[cfg(feature = "runtime-backups")]
    mod hydration_derived_reset;
    #[cfg(feature = "runtime-backups")]
    mod search_rebuild;

    /// Every table the migrations create must be classified in the backup
    /// catalog.
    ///
    /// This is the guard that was missing. `validate_backup_catalog` runs only
    /// when a backup is actually taken, so a migration that adds a table
    /// without a catalog entry ships green and then fails EVERY backup at
    /// runtime — the whole feature, not just the new table's data. Migration
    /// 0151 (`manual_import_selections`,
    /// `manual_import_selection_candidates`) did exactly that, and it was not
    /// the first time.
    ///
    /// Asserting through the production function rather than reimplementing
    /// the comparison keeps the test honest: if the runtime rule changes, this
    /// changes with it. A failure here means the new table needs an entry in
    /// `BACKUP_TABLE_CATALOG` — see the classification guidance there
    /// (`Export` for user data, `ResetOnRestore` for regenerable local state,
    /// `Ignore` for short-lived secrets).
    #[tokio::test]
    async fn backup_catalog_classifies_every_migrated_table() {
        let (pool, _temp) = migrated_sqlite_pool().await;

        if let Err(error) = validate_backup_catalog(&pool).await {
            panic!(
                "migrated schema has tables with no BACKUP_TABLE_CATALOG entry, so every backup \
                 would fail at runtime: {error}"
            );
        }
    }

    /// Catalog entries a fresh migration run legitimately does not produce.
    ///
    /// `_sqlx_migrations` is created by the migrator itself rather than by a
    /// migration. The rest are legacy tables that no current migration creates
    /// but that upgraded installs may still carry: their entries are retained
    /// on purpose, because dropping an entry while any deployment still has
    /// the table would make `validate_backup_catalog` reject that schema and
    /// fail every backup it takes. Each must be classified `Ignore` — a
    /// retained entry may not claim to back anything up.
    const CATALOG_ENTRIES_ABSENT_FROM_A_FRESH_SCHEMA: &[&str] =
        &["_sqlx_migrations", "subtitle_providers"];

    /// The catalog must not accumulate entries for tables nothing creates.
    ///
    /// A stale entry is quieter than a missing one — backups still run — but it
    /// silently claims coverage that no longer exists and hides the fact that a
    /// table's data stopped being backed up. Genuinely legacy entries are
    /// allowed, but only deliberately, via the list above.
    #[tokio::test]
    async fn backup_catalog_has_no_undeclared_entries_for_absent_tables() {
        let (pool, _temp) = migrated_sqlite_pool().await;
        let actual = application_tables(&pool)
            .await
            .expect("read migrated schema tables")
            .into_iter()
            .collect::<BTreeSet<_>>();

        let undeclared = BACKUP_TABLE_CATALOG
            .iter()
            .map(|entry| entry.table)
            .filter(|table| !CATALOG_ENTRIES_ABSENT_FROM_A_FRESH_SCHEMA.contains(table))
            .filter(|table| !actual.contains(*table))
            .collect::<Vec<_>>();

        assert!(
            undeclared.is_empty(),
            "BACKUP_TABLE_CATALOG names tables no migration creates: {}. If a table was dropped, \
             keep its entry as `Ignore` (upgraded installs may still have it) and add it to \
             CATALOG_ENTRIES_ABSENT_FROM_A_FRESH_SCHEMA with the reason.",
            undeclared.join(", ")
        );
    }

    /// A retained legacy entry must never claim to back its table up.
    ///
    /// `Export` would tell the exporter to read a table that does not exist on
    /// a fresh install; `ResetOnRestore` would tell the restorer to clear one.
    /// Only `Ignore` is inert enough to be safe for a table whose presence
    /// varies by install age.
    #[tokio::test]
    async fn retained_legacy_catalog_entries_are_ignored_not_exported() {
        for table in CATALOG_ENTRIES_ABSENT_FROM_A_FRESH_SCHEMA {
            let Some(entry) = BACKUP_TABLE_CATALOG
                .iter()
                .find(|entry| entry.table == *table)
            else {
                panic!("{table} is declared absent-from-fresh-schema but is not in the catalog");
            };
            assert_eq!(
                entry.classification,
                scryer_application::BackupTableClassification::Ignore,
                "{table} may exist only on some installs, so it must be Ignore"
            );
        }
    }

    #[test]
    fn nullable_foreign_keys_convert_blank_strings_to_null() {
        let now = chrono::Utc::now();
        let mut object = JsonMap::from_iter([
            (
                "episode_id".to_string(),
                JsonValue::String("   ".to_string()),
            ),
            (
                "title_id".to_string(),
                JsonValue::String("title-1".to_string()),
            ),
        ]);
        let columns = vec![
            ImportColumnRule {
                kind: ImportColumnKind::Generic,
                has_default: false,
                name: "episode_id".to_string(),
                nullable_foreign_key: true,
                nullable: true,
            },
            ImportColumnRule {
                kind: ImportColumnKind::Generic,
                has_default: false,
                name: "title_id".to_string(),
                nullable_foreign_key: false,
                nullable: false,
            },
        ];

        normalize_import_object_for_target("file_episode_map", &mut object, now, &columns, 2)
            .expect("normalization should succeed");

        assert_eq!(object.get("episode_id"), Some(&JsonValue::Null));
        assert_eq!(
            object.get("title_id"),
            Some(&JsonValue::String("title-1".to_string()))
        );
    }

    #[test]
    fn foreign_key_violation_formatter_includes_sample_details() {
        let rendered = format_foreign_key_violations(&[
            SqliteForeignKeyViolation {
                table: "file_episode_map".to_string(),
                rowid: Some(12),
                parent: "episodes".to_string(),
                fkid: 0,
            },
            SqliteForeignKeyViolation {
                table: "settings_values".to_string(),
                rowid: None,
                parent: "settings_definitions".to_string(),
                fkid: 1,
            },
        ]);

        assert!(rendered.contains("file_episode_map rowid 12 -> episodes (fk 0)"));
        assert!(rendered.contains("settings_values rowid ? -> settings_definitions (fk 1)"));
    }

    /// The offset-paged export as it was before keyset paging, kept verbatim
    /// so the keyset path can be compared with it byte for byte.
    mod reference_export {
        use std::fs::File;
        use std::io::{BufWriter, Write};
        use std::path::Path;

        use scryer_application::{
            AppError, AppResult, EXPORT_BATCH_SIZE, backup_table_part_filename,
        };
        use serde_json::Value as JsonValue;
        use sqlx::Row;

        use super::super::{encode_row, quote_identifier};
        use crate::backup_import_normalization::strip_nonportable_backup_fields;

        pub(super) async fn export_table_part(
            conn: &mut sqlx::SqliteConnection,
            table: &str,
            tables_dir: &Path,
        ) -> AppResult<(u64, String)> {
            let order_by = table_row_order_clause(conn, table).await?;
            let sql = if order_by.is_empty() {
                format!("SELECT * FROM {}", quote_identifier(table))
            } else {
                format!(
                    "SELECT * FROM {} ORDER BY {}",
                    quote_identifier(table),
                    order_by
                )
            };

            let output_path = tables_dir.join(backup_table_part_filename(table));
            let file = File::create(&output_path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to create table export {}: {error}",
                    output_path.display()
                ))
            })?;
            let mut writer = BufWriter::new(file);
            let mut hasher = blake3::Hasher::new();
            let mut line_buffer = Vec::with_capacity(16 * 1024);

            let mut count = 0_u64;
            let mut offset = 0_i64;
            let paged_sql = format!("{sql} LIMIT ? OFFSET ?");
            loop {
                let rows = sqlx::query(sqlx::AssertSqlSafe(&*paged_sql))
                    .bind(EXPORT_BATCH_SIZE)
                    .bind(offset)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!("failed to export table {table}: {error}"))
                    })?;

                if rows.is_empty() {
                    break;
                }

                let row_count = rows.len() as i64;
                for row in rows {
                    let mut value = encode_row(&row)?;
                    if let JsonValue::Object(object) = &mut value {
                        strip_nonportable_backup_fields(table, object);
                    }
                    line_buffer.clear();
                    serde_json::to_writer(&mut line_buffer, &value).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to encode backup row for {table}: {error}"
                        ))
                    })?;
                    line_buffer.push(b'\n');
                    writer.write_all(&line_buffer).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to write backup row for {table}: {error}"
                        ))
                    })?;
                    hasher.update(&line_buffer);
                    count += 1;
                }
                offset += row_count;
            }

            writer.flush().map_err(|error| {
                AppError::Repository(format!("failed to flush table export for {table}: {error}"))
            })?;
            Ok((count, hasher.finalize().to_hex().to_string()))
        }

        async fn table_row_order_clause(
            executor: &mut sqlx::SqliteConnection,
            table: &str,
        ) -> AppResult<String> {
            let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
            let rows = sqlx::query(sqlx::AssertSqlSafe(&*pragma))
                .fetch_all(&mut *executor)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to inspect table info for {table}: {error}"
                    ))
                })?;

            let mut pk_columns = rows
                .iter()
                .filter_map(|row| {
                    let pk: i64 = row.try_get("pk").ok()?;
                    let name: String = row.try_get("name").ok()?;
                    (pk > 0).then_some((pk, name))
                })
                .collect::<Vec<_>>();
            pk_columns.sort_by_key(|(pk, _)| *pk);

            if !pk_columns.is_empty() {
                return Ok(pk_columns
                    .into_iter()
                    .map(|(_, column)| quote_identifier(&column))
                    .collect::<Vec<_>>()
                    .join(", "));
            }

            if rows
                .iter()
                .any(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("id"))
            {
                return Ok(quote_identifier("id"));
            }

            Ok("rowid".to_string())
        }
    }

    /// Keyset paging writes the same bytes as offset paging over the same
    /// order, for every key shape: composite, WITHOUT ROWID with a collation,
    /// nullable but NULL-free, mixed storage classes, the implicit rowid with
    /// gaps, and the offset fallbacks.
    #[tokio::test]
    async fn keyset_export_matches_offset_export_byte_for_byte() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for ddl in [
            "CREATE TABLE paging_composite (series_key TEXT NOT NULL, ordinal INTEGER NOT NULL, label TEXT, PRIMARY KEY (series_key, ordinal))",
            "CREATE TABLE paging_without_rowid (a INTEGER, b TEXT COLLATE NOCASE, note TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID",
            "CREATE TABLE paging_text_key (id TEXT PRIMARY KEY, payload BLOB)",
            "CREATE TABLE paging_mixed_key (k NUMERIC PRIMARY KEY NOT NULL, note TEXT)",
            "CREATE TABLE paging_integer_key (id INTEGER PRIMARY KEY, amount REAL)",
            "CREATE TABLE paging_rowid_only (label TEXT, amount REAL)",
            "CREATE TABLE paging_nullable_key (code TEXT PRIMARY KEY, note TEXT)",
            "CREATE TABLE paging_plain_id (id TEXT, value INTEGER)",
            "CREATE TABLE paging_shadowed_rowid (rowid TEXT, value INTEGER)",
            "CREATE TABLE paging_empty (id TEXT PRIMARY KEY NOT NULL)",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(ddl))
                .execute(&pool)
                .await
                .unwrap();
        }

        // Rows go in out of key order, so the export order comes from the key.
        let scrambled = |count: i64| (0..count).map(move |index| (index * 7_919) % count);
        for index in scrambled(2_600) {
            sqlx::query("INSERT INTO paging_composite VALUES (?, ?, ?)")
                .bind(format!(
                    "{}series-{:02}",
                    if index % 2 == 0 { "S" } else { "s" },
                    index % 13
                ))
                .bind(index / 13 - 40)
                .bind((index % 5 != 0).then(|| format!("synthetic label {index}")))
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_300) {
            sqlx::query("INSERT INTO paging_without_rowid VALUES (?, ?, ?)")
                .bind(index % 4)
                .bind(format!(
                    "{}ey-{index:05}",
                    if index % 3 == 0 { "K" } else { "k" }
                ))
                .bind(format!("note {index}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_500) {
            sqlx::query("INSERT INTO paging_text_key VALUES (?, ?)")
                .bind(format!("synthetic-{index:05}"))
                .bind((0..(index % 40) as u8).collect::<Vec<u8>>())
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_100) {
            let query = sqlx::query("INSERT INTO paging_mixed_key VALUES (?, ?)");
            let query = match index % 4 {
                0 => query.bind(index - 500),
                1 => query.bind(index as f64 / 3.0 + 0.5),
                2 => query.bind(format!("text-{index:04}")),
                _ => query.bind(vec![(index % 251) as u8, (index / 251) as u8]),
            };
            query
                .bind(format!("mixed {index}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_050) {
            sqlx::query("INSERT INTO paging_integer_key VALUES (?, ?)")
                .bind(index * 3 - 900)
                .bind(index as f64 / 7.0)
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in 0..2_100_i64 {
            sqlx::query("INSERT INTO paging_rowid_only VALUES (?, ?)")
                .bind(format!("rowid row {index}"))
                .bind(index as f64)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM paging_rowid_only WHERE rowid % 9 = 0")
            .execute(&pool)
            .await
            .unwrap();
        for index in scrambled(1_200) {
            sqlx::query("INSERT INTO paging_nullable_key VALUES (?, ?)")
                .bind((index % 400 != 0).then(|| format!("code-{index:05}")))
                .bind(format!("nullable {index}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_100) {
            sqlx::query("INSERT INTO paging_plain_id VALUES (?, ?)")
                .bind(format!("plain-{:04}", index % 900))
                .bind(index)
                .execute(&pool)
                .await
                .unwrap();
        }
        for index in scrambled(1_050) {
            sqlx::query("INSERT INTO paging_shadowed_rowid VALUES (?, ?)")
                .bind(format!("shadow-{:04}", index % 700))
                .bind(index)
                .execute(&pool)
                .await
                .unwrap();
        }

        let mut connection = pool.acquire().await.unwrap();
        let reference_dir = tempfile::tempdir().unwrap();
        let keyset_dir = tempfile::tempdir().unwrap();
        for (table, rows, keyset) in [
            ("paging_composite", 2_600, true),
            ("paging_without_rowid", 1_300, true),
            ("paging_text_key", 1_500, true),
            ("paging_mixed_key", 1_100, true),
            ("paging_integer_key", 1_050, true),
            ("paging_rowid_only", 1_867, true),
            ("paging_nullable_key", 1_200, false),
            ("paging_plain_id", 1_100, false),
            ("paging_shadowed_rowid", 1_050, false),
            ("paging_empty", 0, true),
        ] {
            let order = super::table_export_order(&mut connection, table)
                .await
                .unwrap();
            assert_eq!(
                matches!(order.paging, super::ExportPaging::Keyset { .. }),
                keyset,
                "{table} paging",
            );

            let expected =
                reference_export::export_table_part(&mut connection, table, reference_dir.path())
                    .await
                    .unwrap();
            let actual = super::export_table_part(&mut connection, table, keyset_dir.path())
                .await
                .unwrap();
            assert_eq!(expected.0, rows, "{table} reference row count");
            assert_eq!(actual, expected, "{table} count and checksum");

            let part = scryer_application::backup_table_part_filename(table);
            assert_eq!(
                std::fs::read(keyset_dir.path().join(&part)).unwrap(),
                std::fs::read(reference_dir.path().join(&part)).unwrap(),
                "{table} part bytes",
            );
        }
    }
}
