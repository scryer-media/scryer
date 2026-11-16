use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Column, Row, TypeInfo, ValueRef};

use scryer_application::{
    AppError, AppResult, BACKUP_TABLE_CATALOG, BackupBundleExportRequest, BackupBundleStaging,
    BackupExportOutcome, BackupRestorePreparedBundle, BackupTableClassification,
    LogicalBackupExporter, PreparedBackupBundleDirectory, backup_table_part_filename,
    prepare_backup_restore_payload,
};

use crate::backup_import_normalization::{
    ImportColumnKind, ImportColumnRule, InsertStatements, RestoreRowReader,
    validate_restore_manifest_table_set,
};
use crate::postgres::PostgresServices;
use crate::queries::title_search::rebuild_title_search_projection_pg_on_connection;
use crate::sqlite_backup::TablePartWriter;

const EXPORT_BATCH_SIZE: i64 = 500;

#[derive(Clone)]
pub struct PostgresLogicalBackupExporter {
    pool: PgPool,
}

#[derive(Clone, Debug)]
struct PgColumnInfo {
    name: String,
    has_default: bool,
    nullable: bool,
    nullable_foreign_key: bool,
    udt_name: String,
}

impl PgColumnInfo {
    fn import_rule(&self) -> ImportColumnRule {
        ImportColumnRule {
            name: self.name.clone(),
            nullable: self.nullable,
            has_default: self.has_default,
            nullable_foreign_key: self.nullable_foreign_key,
            kind: if is_timestamp_column(self) {
                ImportColumnKind::TimestampLike
            } else {
                ImportColumnKind::Generic
            },
        }
    }
}

impl PostgresLogicalBackupExporter {
    pub fn new(db: &PostgresServices) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl LogicalBackupExporter for PostgresLogicalBackupExporter {
    async fn export_backup_bundle(
        &self,
        request: BackupBundleExportRequest,
    ) -> AppResult<BackupExportOutcome> {
        export_backup_bundle_from_postgres_pool(&self.pool, request).await
    }
}

pub async fn export_backup_bundle_from_postgres_pool(
    pool: &PgPool,
    request: BackupBundleExportRequest,
) -> AppResult<BackupExportOutcome> {
    let mut staging = BackupBundleStaging::new()?;
    export_backup_tables_from_pool(pool, &mut staging).await?;
    staging.finish_off_runtime(request).await
}

async fn export_backup_tables_from_pool(
    pool: &PgPool,
    staging: &mut BackupBundleStaging,
) -> AppResult<()> {
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let tables_dir = staging.tables_dir();

    let mut tx = pool.begin().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to begin PostgreSQL backup snapshot: {error}"
        ))
    })?;

    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to configure PostgreSQL backup snapshot: {error}"
            ))
        })?;

    for table in &export_tables {
        let (row_count, checksum) = export_table_part(&mut tx, table, &tables_dir).await?;
        staging.record_table_part(table, row_count, checksum)?;
    }

    tx.rollback().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to close PostgreSQL backup snapshot: {error}"
        ))
    })?;
    Ok(())
}

pub async fn restore_backup_bundle_into_postgres_pool(
    pool: &PgPool,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupRestorePreparedBundle> {
    let payload = prepare_backup_restore_payload(bundle_path, passphrase)?;
    restore_bundle_parts_into_postgres_pool(
        pool,
        &payload.manifest().row_counts,
        payload.manifest().source_migration_key.as_deref(),
        &payload.tables_dir(),
    )
    .await?;

    Ok(
        BackupRestorePreparedBundle::from_summary_and_instance_secrets_env(
            payload.summary(),
            payload.instance_secrets_env()?,
        ),
    )
}

pub async fn restore_prepared_backup_directory_into_postgres_pool(
    pool: &PgPool,
    prepared_root: &Path,
) -> AppResult<BackupRestorePreparedBundle> {
    let payload = PreparedBackupBundleDirectory::load(prepared_root)?;
    restore_bundle_parts_into_postgres_pool(
        pool,
        &payload.manifest().row_counts,
        payload.manifest().source_migration_key.as_deref(),
        &payload.tables_dir(),
    )
    .await?;

    Ok(
        BackupRestorePreparedBundle::from_summary_and_instance_secrets_env(
            payload.summary(),
            payload.instance_secrets_env()?,
        ),
    )
}

async fn restore_bundle_parts_into_postgres_pool(
    pool: &PgPool,
    row_counts: &BTreeMap<String, u64>,
    source_migration_key: Option<&str>,
    tables_dir: &Path,
) -> AppResult<()> {
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let restore_tables = ordered_restore_tables(pool).await?;
    validate_restore_manifest_table_set(row_counts, &export_tables, source_migration_key)?;

    let mut tx = pool.begin().await.map_err(|error| {
        AppError::Repository(format!("failed to begin PostgreSQL restore: {error}"))
    })?;

    let deadline = scryer_application::current_restore_deadline();
    let restore_work = async {
        if let Some(deadline) = &deadline {
            configure_restore_transaction(&mut tx, deadline).await?;
        }
        for table in restore_tables.iter().rev() {
            let sql = format!("DELETE FROM {}", quote_identifier(table));
            sqlx::query(sqlx::AssertSqlSafe(&*sql))
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to clear restore table {table}: {error}"))
                })?;
        }

        if let Some(progress) = &deadline {
            progress.begin_import(
                row_counts
                    .values()
                    .copied()
                    .fold(0_u64, u64::saturating_add),
            );
        }
        for table in export_tables
            .iter()
            .filter(|table| row_counts.contains_key(*table))
        {
            import_table_part(
                &mut tx,
                table,
                &tables_dir.join(backup_table_part_filename(table)),
            )
            .await?;
        }

        if let Some(progress) = &deadline {
            progress.finish_import();
        }
        rebuild_title_search_projection(&mut tx).await?;
        repair_sequences(&mut tx).await?;

        for table in export_tables
            .iter()
            .filter(|table| row_counts.contains_key(*table))
        {
            let expected_rows = row_counts.get(table).ok_or_else(|| {
            AppError::validation_refused(crate::error_reasons::DATASTORE_BACKUP_TABLE_SET_MISMATCH, format!(
                "backup bundle table set does not match the current restore catalog: missing [{}], unexpected []",
                table
            ))
        })?;
            let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
            let actual_rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(&*sql))
                .fetch_one(&mut *tx)
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

        scryer_application::check_restore_deadline()?;
        Ok(())
    };
    let result = match &deadline {
        Some(deadline) => deadline.run(restore_work).await,
        None => restore_work.await,
    };
    if let Err(error) = result {
        tx.rollback().await.map_err(|rollback| {
            AppError::validation_refused(
                scryer_application::restore_refusal::RESTORE_RECOVERY_REQUIRED,
                format!("restore rollback failed: {rollback}"),
            )
        })?;
        return Err(error);
    }
    tx.commit().await.map_err(|error| {
        AppError::validation_refused(
            scryer_application::restore_refusal::RESTORE_RECOVERY_REQUIRED,
            format!("failed to commit PostgreSQL restore: {error}"),
        )
    })?;

    Ok(())
}

async fn validate_backup_catalog(pool: &PgPool) -> AppResult<()> {
    let actual_tables = application_tables(pool).await?;
    let classified = BACKUP_TABLE_CATALOG
        .iter()
        .map(|entry| entry.table.to_string())
        .collect::<BTreeSet<_>>();
    let unclassified = actual_tables
        .into_iter()
        .filter(|table| !classified.contains(table))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        return Err(AppError::Repository(format!(
            "backup catalog is missing classifications for PostgreSQL tables: {}",
            unclassified.join(", ")
        )));
    }
    Ok(())
}

async fn application_tables(pool: &PgPool) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT table_name
           FROM information_schema.tables
          WHERE table_schema = current_schema()
            AND table_type = 'BASE TABLE'
          ORDER BY table_name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect PostgreSQL schema: {error}"))
    })?;

    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("table_name").ok())
        .filter(|table| !is_engine_internal_table(table))
        .collect())
}

fn is_engine_internal_table(table: &str) -> bool {
    table == "_sqlx_migrations"
}

async fn ordered_export_tables(pool: &PgPool) -> AppResult<Vec<String>> {
    ordered_catalog_tables(pool, &[BackupTableClassification::Export]).await
}

async fn ordered_restore_tables(pool: &PgPool) -> AppResult<Vec<String>> {
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
    pool: &PgPool,
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

    let rows = sqlx::query(
        "SELECT tc.table_name AS table_name,
                ccu.table_name AS referenced_table
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
             ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
           JOIN information_schema.constraint_column_usage ccu
             ON ccu.constraint_name = tc.constraint_name
            AND ccu.table_schema = tc.table_schema
          WHERE tc.constraint_type = 'FOREIGN KEY'
            AND tc.table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PostgreSQL foreign keys: {error}"
        ))
    })?;

    for row in rows {
        let table: String = row.try_get("table_name").map_err(repo_err)?;
        let referenced: String = row.try_get("referenced_table").map_err(repo_err)?;
        if !catalog_tables.contains(&table) || !catalog_tables.contains(&referenced) {
            continue;
        }
        if referenced == table {
            continue;
        }
        if outgoing
            .get_mut(&referenced)
            .expect("known table")
            .insert(table.clone())
        {
            *incoming.get_mut(&table).expect("known table") += 1;
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
                ready.insert(insert_at, dependent);
            }
        }
    }

    if ordered.len() != catalog_tables.len() {
        return Err(AppError::Repository(
            "backup catalog dependencies contain a PostgreSQL cycle".into(),
        ));
    }

    Ok(ordered)
}

async fn export_table_part(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    tables_dir: &Path,
) -> AppResult<(u64, String)> {
    let order = table_export_order(tx, table).await?;
    let mut part = TablePartWriter::create(table, tables_dir, "PostgreSQL table export").await?;
    match order.keyset {
        None => {
            let sql = format!(
                "SELECT * FROM {} ORDER BY {} LIMIT $1 OFFSET $2",
                quote_identifier(table),
                order.order_by
            );
            let mut offset = 0_i64;
            loop {
                let rows = sqlx::query(sqlx::AssertSqlSafe(&*sql))
                    .bind(EXPORT_BATCH_SIZE)
                    .bind(offset)
                    .fetch_all(&mut **tx)
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
        Some(key) => {
            // The key travels as text and is cast back to its column's type,
            // which round-trips exactly for the key types allowed here. The
            // text copies are trailing columns that are never encoded.
            let key_text = key
                .iter()
                .map(|(column, _)| format!("{}::text", quote_identifier(column)))
                .collect::<Vec<_>>()
                .join(", ");
            let key_list = key
                .iter()
                .map(|(column, _)| quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            let key_params = key
                .iter()
                .enumerate()
                .map(|(index, (_, cast))| format!("${}::{cast}", index + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let first_sql = format!(
                "SELECT *, {key_text} FROM {} ORDER BY {} LIMIT $1",
                quote_identifier(table),
                order.order_by
            );
            let next_sql = format!(
                "SELECT *, {key_text} FROM {} WHERE ({key_list}) > ({key_params}) ORDER BY {} LIMIT ${}",
                quote_identifier(table),
                order.order_by,
                key.len() + 1
            );
            let mut last_key: Option<Vec<String>> = None;
            loop {
                let rows = match &last_key {
                    None => {
                        sqlx::query(sqlx::AssertSqlSafe(&*first_sql))
                            .bind(EXPORT_BATCH_SIZE)
                            .fetch_all(&mut **tx)
                            .await
                    }
                    Some(values) => {
                        let mut query = sqlx::query(sqlx::AssertSqlSafe(&*next_sql));
                        for value in values {
                            query = query.bind(value.clone());
                        }
                        query.bind(EXPORT_BATCH_SIZE).fetch_all(&mut **tx).await
                    }
                }
                .map_err(|error| export_query_error(table, error))?;
                let Some(last) = rows.last() else {
                    break;
                };
                let columns = last.columns().len() - key.len();
                last_key = Some(
                    (columns..last.columns().len())
                        .zip(&key)
                        .map(|(index, (column, _))| {
                            last.try_get::<String, _>(index).map_err(|error| {
                                AppError::Repository(format!(
                                    "failed to read backup key column {column} of {table}: {error}"
                                ))
                            })
                        })
                        .collect::<AppResult<Vec<_>>>()?,
                );
                part = part.write_page(rows, encode_row_columns, columns).await?;
            }
        }
    }
    part.finish().await
}

fn export_query_error(table: &str, error: sqlx::Error) -> AppError {
    AppError::Repository(format!(
        "failed to export PostgreSQL table {table}: {error}"
    ))
}

/// How a table's rows are paged for export, always in `order_by` order.
/// `keyset` holds the primary key's columns and the type each is cast back
/// to, when every one of them is a type whose text form round-trips exactly;
/// otherwise (a non-key `id`, `ctid`, or another key type) pages are read
/// with `LIMIT … OFFSET …` as before.
struct TableExportOrder {
    order_by: String,
    keyset: Option<Vec<(String, &'static str)>>,
}

async fn table_export_order(
    executor: &mut sqlx::PgConnection,
    table: &str,
) -> AppResult<TableExportOrder> {
    let pk_rows = sqlx::query(
        "SELECT kcu.column_name
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
             ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
          WHERE tc.constraint_type = 'PRIMARY KEY'
            AND tc.table_schema = current_schema()
            AND tc.table_name = $1
          ORDER BY kcu.ordinal_position ASC",
    )
    .bind(table)
    .fetch_all(&mut *executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect primary key for {table}: {error}"
        ))
    })?;

    let pk_columns = pk_rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("column_name").ok())
        .collect::<Vec<_>>();
    if !pk_columns.is_empty() {
        let order_by = pk_columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let type_rows = sqlx::query(
            "SELECT column_name, data_type
               FROM information_schema.columns
              WHERE table_schema = current_schema()
                AND table_name = $1",
        )
        .bind(table)
        .fetch_all(&mut *executor)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect primary key types for {table}: {error}"
            ))
        })?;
        let types = type_rows
            .into_iter()
            .filter_map(|row| {
                Some((
                    row.try_get::<String, _>("column_name").ok()?,
                    row.try_get::<String, _>("data_type").ok()?,
                ))
            })
            .collect::<BTreeMap<_, _>>();
        // A primary key is unique and NOT NULL in PostgreSQL, so it can page
        // by key whenever its values survive a text round trip.
        let keyset = pk_columns
            .iter()
            .map(|column| {
                let cast = match types.get(column)?.as_str() {
                    "text" | "character varying" => "text",
                    "uuid" => "uuid",
                    "smallint" | "integer" | "bigint" => "bigint",
                    _ => return None,
                };
                Some((column.clone(), cast))
            })
            .collect::<Option<Vec<_>>>();
        return Ok(TableExportOrder { order_by, keyset });
    }

    let has_id: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM information_schema.columns
              WHERE table_schema = current_schema()
                AND table_name = $1
                AND column_name = 'id'
         )",
    )
    .bind(table)
    .fetch_one(&mut *executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect id column for {table}: {error}"))
    })?;
    Ok(TableExportOrder {
        order_by: if has_id {
            quote_identifier("id")
        } else {
            "ctid".to_string()
        },
        keyset: None,
    })
}

fn encode_row_columns(row: &PgRow, encoded_columns: usize) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate().take(encoded_columns) {
        let raw = row.try_get_raw(index).map_err(|error| {
            AppError::Repository(format!(
                "failed to read backup column {} from PostgreSQL row: {error}",
                column.name()
            ))
        })?;
        let value = if raw.is_null() {
            JsonValue::Null
        } else {
            encode_pg_value(row, index, column.name(), raw.type_info().name())?
        };
        object.insert(column.name().to_string(), value);
    }
    Ok(JsonValue::Object(object))
}

fn encode_pg_value(
    row: &PgRow,
    index: usize,
    column: &str,
    type_name: &str,
) -> AppResult<JsonValue> {
    let normalized = type_name.to_ascii_uppercase();
    match normalized.as_str() {
        "BOOL" | "BOOLEAN" => Ok(JsonValue::Bool(row.try_get::<bool, _>(index).map_err(
            |error| AppError::Repository(format!("failed to decode bool column {column}: {error}")),
        )?)),
        "INT2" | "SMALLINT" => Ok(JsonValue::from(row.try_get::<i16, _>(index).map_err(
            |error| {
                AppError::Repository(format!(
                    "failed to decode smallint column {column}: {error}"
                ))
            },
        )?)),
        "INT4" | "INTEGER" => Ok(JsonValue::from(row.try_get::<i32, _>(index).map_err(
            |error| {
                AppError::Repository(format!("failed to decode integer column {column}: {error}"))
            },
        )?)),
        "INT8" | "BIGINT" => Ok(JsonValue::from(row.try_get::<i64, _>(index).map_err(
            |error| {
                AppError::Repository(format!("failed to decode bigint column {column}: {error}"))
            },
        )?)),
        "FLOAT4" | "FLOAT8" | "REAL" | "DOUBLE PRECISION" => Ok(JsonValue::from(
            row.try_get::<f64, _>(index).map_err(|error| {
                AppError::Repository(format!("failed to decode float column {column}: {error}"))
            })?,
        )),
        "BYTEA" => Ok(encode_blob_value(
            &row.try_get::<Vec<u8>, _>(index).map_err(|error| {
                AppError::Repository(format!("failed to decode bytea column {column}: {error}"))
            })?,
        )),
        "JSON" | "JSONB" => row.try_get::<JsonValue, _>(index).map_err(|error| {
            AppError::Repository(format!("failed to decode JSON column {column}: {error}"))
        }),
        "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => Ok(JsonValue::String(
            row.try_get::<chrono::DateTime<chrono::Utc>, _>(index)
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode timestamptz column {column}: {error}"
                    ))
                })?
                .to_rfc3339(),
        )),
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => Ok(JsonValue::String(
            row.try_get::<chrono::NaiveDateTime, _>(index)
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode timestamp column {column}: {error}"
                    ))
                })?
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        )),
        _ => row
            .try_get::<String, _>(index)
            .map(JsonValue::String)
            .map_err(|error| {
                AppError::Repository(format!("failed to decode text column {column}: {error}"))
            }),
    }
}

fn encode_blob_value(bytes: &[u8]) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert(
        "__scryer_type".to_string(),
        JsonValue::String("blob".to_string()),
    );
    object.insert(
        "base64".to_string(),
        JsonValue::String(STANDARD.encode(bytes)),
    );
    JsonValue::Object(object)
}

async fn import_table_part(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    part_path: &Path,
) -> AppResult<()> {
    let target_columns = table_columns(tx, table).await?;
    let target_column_rules = target_columns
        .iter()
        .map(PgColumnInfo::import_rule)
        .collect::<Vec<_>>();
    let mut rows = RestoreRowReader::open(table, part_path, target_column_rules).await?;
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
                    .filter(|(_, column)| object.contains_key(&column.name))
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
                    present
                        .iter()
                        .enumerate()
                        .map(|(position, &index)| {
                            format!("${}{}", position + 1, pg_cast(&target_columns[index]))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });

            let mut query = sqlx::query(sqlx::AssertSqlSafe(insert_sql));
            for &index in &present {
                let column = &target_columns[index];
                let value = object.remove(&column.name).unwrap_or(JsonValue::Null);
                query = bind_pg_value(query, column, value)?;
            }
            query.execute(&mut **tx).await.map_err(|error| {
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
    executor: &mut sqlx::PgConnection,
    table: &str,
) -> AppResult<Vec<PgColumnInfo>> {
    let foreign_key_rows = sqlx::query(
        "SELECT kcu.column_name
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
             ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
          WHERE tc.constraint_type = 'FOREIGN KEY'
            AND tc.table_schema = current_schema()
            AND tc.table_name = $1",
    )
    .bind(table)
    .fetch_all(&mut *executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PostgreSQL foreign keys for {table}: {error}"
        ))
    })?;
    let foreign_key_columns = foreign_key_rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("column_name").ok())
        .collect::<BTreeSet<_>>();

    let rows = sqlx::query(
        "SELECT column_name, is_nullable, column_default, udt_name
           FROM information_schema.columns
          WHERE table_schema = current_schema()
            AND table_name = $1
          ORDER BY ordinal_position ASC",
    )
    .bind(table)
    .fetch_all(executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PostgreSQL table columns for {table}: {error}"
        ))
    })?;
    rows.into_iter()
        .map(|row| {
            let name: String = row.try_get("column_name").map_err(repo_err)?;
            let nullable = row.try_get::<String, _>("is_nullable").map_err(repo_err)? == "YES";
            Ok(PgColumnInfo {
                has_default: row
                    .try_get::<Option<String>, _>("column_default")
                    .map_err(repo_err)?
                    .is_some(),
                name: name.clone(),
                nullable,
                nullable_foreign_key: nullable && foreign_key_columns.contains(&name),
                udt_name: row.try_get("udt_name").map_err(repo_err)?,
            })
        })
        .collect::<AppResult<Vec<_>>>()
}

fn bind_pg_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    column: &PgColumnInfo,
    value: JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    if is_json_column(column) {
        return match logical_json_value(value) {
            Some(value) => Ok(query.bind(sqlx::types::Json(value))),
            None => Ok(query.bind(None::<sqlx::types::Json<JsonValue>>)),
        };
    }
    if is_bytea_column(column) {
        return match blob_bytes(value)? {
            Some(bytes) => Ok(query.bind(bytes)),
            None => Ok(query.bind(None::<Vec<u8>>)),
        };
    }

    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(value.to_string()),
        JsonValue::Number(value) => query.bind(value.to_string()),
        JsonValue::String(value) if value.is_empty() && is_timestamp_column(column) => {
            query.bind(None::<String>)
        }
        JsonValue::String(value) => query.bind(value),
        JsonValue::Array(_) | JsonValue::Object(_) => query.bind(value.to_string()),
    })
}

fn logical_json_value(value: JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(value) => {
            Some(serde_json::from_str(&value).unwrap_or(JsonValue::String(value)))
        }
        value => Some(value),
    }
}

fn blob_bytes(value: JsonValue) -> AppResult<Option<Vec<u8>>> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Object(ref object)
            if object.get("__scryer_type").and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get("base64")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::DATASTORE_BACKUP_BLOB_BYTES_MISSING,
                        "backup blob payload is missing base64 bytes",
                    )
                })?;
            STANDARD.decode(encoded).map(Some).map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::DATASTORE_BACKUP_BLOB_BASE64_INVALID,
                    format!("backup blob payload is invalid base64: {error}"),
                )
            })
        }
        JsonValue::String(value) => Ok(Some(value.into_bytes())),
        _ => Err(AppError::validation_refused(
            crate::error_reasons::DATASTORE_BACKUP_BYTEA_UNSUPPORTED,
            "backup row contains an unsupported PostgreSQL bytea value",
        )),
    }
}

fn pg_cast(column: &PgColumnInfo) -> &'static str {
    match column.udt_name.as_str() {
        "bool" => "::boolean",
        "int2" => "::smallint",
        "int4" => "::integer",
        "int8" => "::bigint",
        "float4" => "::real",
        "float8" => "::double precision",
        "numeric" => "::numeric",
        "timestamp" => "::timestamp",
        "timestamptz" => "::timestamptz",
        "date" => "::date",
        "json" => "::json",
        "jsonb" => "::jsonb",
        _ => "",
    }
}

fn is_json_column(column: &PgColumnInfo) -> bool {
    matches!(column.udt_name.as_str(), "json" | "jsonb")
}

fn is_bytea_column(column: &PgColumnInfo) -> bool {
    column.udt_name == "bytea"
}

fn is_timestamp_column(column: &PgColumnInfo) -> bool {
    matches!(
        column.udt_name.as_str(),
        "timestamp" | "timestamptz" | "date"
    )
}

async fn repair_sequences(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT ns.nspname AS sequence_schema,
                seq.relname AS sequence_name,
                tbl.relname AS table_name,
                att.attname AS column_name
           FROM pg_class seq
           JOIN pg_namespace ns ON ns.oid = seq.relnamespace
           JOIN pg_depend dep ON dep.objid = seq.oid AND dep.deptype = 'a'
           JOIN pg_class tbl ON tbl.oid = dep.refobjid
           JOIN pg_attribute att ON att.attrelid = tbl.oid AND att.attnum = dep.refobjsubid
          WHERE seq.relkind = 'S'
            AND ns.nspname = current_schema()",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect PostgreSQL sequences: {error}"))
    })?;

    for row in rows {
        let schema: String = row.try_get("sequence_schema").map_err(repo_err)?;
        let sequence: String = row.try_get("sequence_name").map_err(repo_err)?;
        let table: String = row.try_get("table_name").map_err(repo_err)?;
        let column: String = row.try_get("column_name").map_err(repo_err)?;
        let qualified = format!(
            "{}.{}",
            quote_identifier(&schema),
            quote_identifier(&sequence)
        );
        let table_name = quote_identifier(&table);
        let column_name = quote_identifier(&column);
        let sql = format!(
            "SELECT setval(
                $1::regclass,
                GREATEST(COALESCE((SELECT MAX({column_name}) FROM {table_name}), 0), 1),
                COALESCE((SELECT MAX({column_name}) FROM {table_name}), 0) > 0
             )"
        );
        sqlx::query(sqlx::AssertSqlSafe(&*sql))
            .bind(qualified)
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to repair PostgreSQL sequence {sequence}: {error}"
                ))
            })?;
    }
    Ok(())
}

/// Rebuild the restored database's title-search projection.
///
/// Delegates to the search crate's paged rebuild so restore and start-up share
/// one implementation: the same columns, the same collation-version stamp, and
/// a catalog read in pages rather than one `fetch_all` of every title.
async fn rebuild_title_search_projection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    rebuild_title_search_projection_pg_on_connection(&mut *tx)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to rebuild PostgreSQL title search projection: {error}"
            ))
        })
}

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

async fn configure_restore_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    deadline: &scryer_application::RestoreDeadline,
) -> AppResult<()> {
    deadline.check()?;
    let milliseconds = deadline.remaining().as_millis().max(1).to_string();
    sqlx::query("SELECT set_config('statement_timeout', $1, true), set_config('idle_in_transaction_session_timeout', $1, true)")
        .bind(&milliseconds).execute(&mut **tx).await.map_err(repo_err)?;
    // PostgreSQL 17+ also has a total transaction limit. SET LOCAL restores
    // the caller's settings on both commit and rollback.
    sqlx::query("SELECT set_config('transaction_timeout', $1, true) WHERE current_setting('transaction_timeout', true) IS NOT NULL")
        .bind(&milliseconds).execute(&mut **tx).await.map_err(repo_err)?;
    Ok(())
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires SCRYER_TEST_POSTGRES_URL pointing to an isolated PostgreSQL fixture"]
    async fn background_restore_postgres_import_rollback_progress_and_timeouts() -> AppResult<()> {
        let raw_url = std::env::var("SCRYER_TEST_POSTGRES_URL").expect("isolated fixture URL");
        tokio::time::timeout(std::time::Duration::from_secs(180), async {
            let admin = PgPool::connect(&raw_url).await.map_err(repo_err)?;
            let schema = format!(
                "restore_test_{}",
                scryer_domain::Id::new().0.replace('-', "_")
            );
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .map_err(repo_err)?;
            let mut url = url::Url::parse(&raw_url).unwrap();
            url.query_pairs_mut()
                .append_pair("options", &format!("-csearch_path={schema}"));
            let services =
                PostgresServices::new_with_mode(url.as_str(), crate::MigrationMode::Apply).await?;
            let pool = services.pool();
            sqlx::query(
                "INSERT INTO indexer_monthly_usage
                 (indexer_id, month_start, request_count, grab_count)
                 VALUES ('backup-quota-fixture', '2026-01-01', 123, 7)",
            )
            .execute(pool)
            .await
            .map_err(repo_err)?;
            validate_backup_catalog(pool).await?;
            let tables = ordered_export_tables(pool).await?;
            let directory = tempfile::tempdir().unwrap();
            let mut counts = BTreeMap::new();
            let mut tx = pool.begin().await.map_err(repo_err)?;
            for table in &tables {
                let (rows, _) = export_table_part(&mut tx, table, directory.path()).await?;
                counts.insert(table.clone(), rows);
            }
            tx.rollback().await.map_err(repo_err)?;
            assert_eq!(counts.get("indexer_monthly_usage"), Some(&1));
            sqlx::query(
                "UPDATE indexer_monthly_usage SET request_count = 199, grab_count = 19
                 WHERE indexer_id = 'backup-quota-fixture'",
            )
            .execute(pool)
            .await
            .map_err(repo_err)?;
            let deadline = scryer_application::RestoreDeadline::new();
            deadline
                .scope(restore_bundle_parts_into_postgres_pool(
                    pool,
                    &counts,
                    None,
                    directory.path(),
                ))
                .await?;
            assert_eq!(deadline.import_percent(), Some(100));
            let usage: (String, i64, i64) = sqlx::query_as(
                "SELECT month_start, request_count, grab_count FROM indexer_monthly_usage
                 WHERE indexer_id = 'backup-quota-fixture'",
            )
            .fetch_one(pool)
            .await
            .map_err(repo_err)?;
            assert_eq!(usage, ("2026-01-01".into(), 123, 7));

            // A bad row is encountered after the transaction clears its tables.
            // The failed import must restore every original row count.
            let part = directory
                .path()
                .join(backup_table_part_filename(&tables[0]));
            std::fs::write(part, b"not-json\n").unwrap();
            let retry = scryer_application::RestoreDeadline::new();
            assert!(
                retry
                    .scope(restore_bundle_parts_into_postgres_pool(
                        pool,
                        &counts,
                        None,
                        directory.path()
                    ))
                    .await
                    .is_err()
            );
            for (table, expected) in &counts {
                let actual: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT COUNT(*) FROM {}",
                    quote_identifier(table)
                )))
                .fetch_one(pool)
                .await
                .map_err(repo_err)?;
                assert_eq!(actual as u64, *expected, "{table}");
            }

            let mut connection = pool.acquire().await.map_err(repo_err)?;
            sqlx::query("SET statement_timeout = '310s'")
                .execute(&mut *connection)
                .await
                .map_err(repo_err)?;
            use sqlx::Acquire;
            let mut tx = connection.begin().await.map_err(repo_err)?;
            configure_restore_transaction(&mut tx, &deadline).await?;
            let configured: String =
                sqlx::query_scalar("SELECT current_setting('statement_timeout')")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_err)?;
            assert_ne!(configured, "310s");
            let seconds: f64 = sqlx::query_scalar(
                "SELECT EXTRACT(EPOCH FROM current_setting('statement_timeout')::interval)::float8",
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
            assert!(seconds > 310.0 && seconds <= 3600.0);
            tx.rollback().await.map_err(repo_err)?;
            let restored: String =
                sqlx::query_scalar("SELECT current_setting('statement_timeout')")
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(repo_err)?;
            assert_eq!(restored, "310s");
            drop(connection);
            pool.close().await;
            sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
                .execute(&admin)
                .await
                .map_err(repo_err)?;
            admin.close().await;
            Ok(())
        })
        .await
        .expect("isolated PostgreSQL restore test must finish")
    }
}
