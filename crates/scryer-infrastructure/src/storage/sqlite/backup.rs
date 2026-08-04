use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
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
    ImportColumnKind, ImportColumnRule, normalize_import_object_for_target,
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

    staging.finish(request)
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
    validate_restore_manifest_table_set(&payload.manifest().row_counts, &export_tables)?;

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!("failed to acquire restore connection: {error}"))
    })?;

    if let Err(error) = sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await {
        return Err(AppError::Repository(format!(
            "failed to begin SQLite restore transaction: {error}"
        )));
    }

    let tables_dir = payload.tables_dir();
    let restore_result = async {
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

        for table in &export_tables {
            import_table_part(
                &mut conn,
                table,
                &tables_dir.join(backup_table_part_filename(table)),
            )
            .await?;
        }

        let violations = foreign_key_violations(&mut conn).await?;
        if !violations.is_empty() {
            return Err(AppError::Validation(format!(
                "restored database failed foreign key validation: {}",
                format_foreign_key_violations(&violations)
            )));
        }

        for table in &export_tables {
            let expected_rows = payload.manifest().row_counts.get(table).ok_or_else(|| {
                AppError::Validation(format!(
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
                return Err(AppError::Validation(format!(
                    "restored table {table} row count mismatch: expected {expected_rows}, got {actual_rows}"
                )));
            }
        }

        AppResult::Ok(())
    }
    .await;

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

    crate::queries::title_search::rebuild_title_search_projection(pool).await?;

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
    table.starts_with("sqlite_") || table.starts_with("title_search_spellfix_")
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

async fn export_table_part(
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
                AppError::Repository(format!("failed to encode backup row for {table}: {error}"))
            })?;
            line_buffer.push(b'\n');
            writer.write_all(&line_buffer).map_err(|error| {
                AppError::Repository(format!("failed to write backup row for {table}: {error}"))
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
            AppError::Repository(format!("failed to inspect table info for {table}: {error}"))
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

fn encode_row(row: &SqliteRow) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate() {
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

    let file = File::open(part_path).map_err(|error| {
        AppError::Validation(format!("backup table payload missing for {table}: {error}"))
    })?;
    let reader = BufReader::new(file);

    for (line_number, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| {
            AppError::Validation(format!(
                "failed to read backup row {table}:{line_number}: {error}"
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let value: JsonValue = serde_json::from_str(&line).map_err(|error| {
            AppError::Validation(format!(
                "invalid backup row for {table}:{line_number}: {error}"
            ))
        })?;
        let mut object = value.as_object().cloned().ok_or_else(|| {
            AppError::Validation(format!(
                "backup row for {table}:{line_number} is not an object"
            ))
        })?;
        normalize_import_object_for_target(
            table,
            &mut object,
            chrono::Utc::now(),
            &target_columns,
            line_number,
        )?;

        let columns = target_columns
            .iter()
            .filter(|column| object.contains_key(column.name.as_str()))
            .collect::<Vec<_>>();
        if columns.is_empty() {
            continue;
        }

        let insert_sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(table),
            columns
                .iter()
                .map(|column| quote_identifier(&column.name))
                .collect::<Vec<_>>()
                .join(", "),
            std::iter::repeat_n("?", columns.len())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let mut query = sqlx::query(sqlx::AssertSqlSafe(&*insert_sql));
        for column in &columns {
            let value = object.get(&column.name).unwrap_or(&JsonValue::Null);
            query = bind_json_value(query, value)?;
        }
        query.execute(&mut **conn).await.map_err(|error| {
            AppError::Validation(format!(
                "failed to import backup row for {table}:{line_number}: {error}"
            ))
        })?;
    }

    Ok(())
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
    value: &JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>> {
    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    AppError::Validation(
                        "backup row contains an integer outside SQLite i64 range".into(),
                    )
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(AppError::Validation(
                    "backup row contains an unsupported numeric value".into(),
                ));
            }
        }
        JsonValue::String(value) => query.bind(value.clone()),
        JsonValue::Object(object)
            if object.get(BLOB_MARKER_TYPE).and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get(BLOB_MARKER_BASE64)
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::Validation("backup blob payload is missing base64 bytes".into())
                })?;
            let bytes = STANDARD.decode(encoded).map_err(|error| {
                AppError::Validation(format!("backup blob payload is invalid base64: {error}"))
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

    /// Migrate a throwaway SQLite database to head and hand back its pool.
    ///
    /// The tempdir must outlive the pool, so it is returned alongside it.
    async fn migrated_sqlite_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("create migration tempdir");
        let db_path = temp.path().join("catalog-check.db");
        let services = crate::storage::sqlite::services::SqliteServices::new_with_mode(
            format!("sqlite://{}", db_path.display()),
            crate::MigrationMode::Apply,
        )
        .await
        .expect("migrate sqlite schema to head");
        (services.pool().clone(), temp)
    }

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
}
