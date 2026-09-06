pub mod assets;
pub mod blake3_identities;
#[cfg(test)]
#[path = "blake3_identities_upgrade_tests.rs"]
mod blake3_identities_upgrade_tests;
#[cfg(test)]
#[path = "blocklist_release_identity_upgrade_tests.rs"]
mod blocklist_release_identity_upgrade_tests;
pub mod canonical_download_identity;
pub mod event_storage;
pub mod hook_ids;
pub mod known_bad;
pub mod notification_targets;
pub mod post_0_16_6_prerelease;
pub mod post_processing_output;
pub mod rule_set_runtime_wrapper;
pub mod title_catalog_sort_keys;
pub mod title_folder_ownership;
pub mod title_folder_ownership_safe;
pub mod title_image_blobs;
pub mod title_root_folder_ids;

use crate::encryption::EncryptionKey;
use crate::migration_assets::{
    self, ChecksumAlgorithm, CompiledBaseline, CompiledMigration, CompiledMigrationBundle,
    CompiledMigrationCatalog, CompiledMigrationStep, EngineScope, MigrationInstallKind,
};
use crate::migration_hook_ids;
use crate::{EmbeddedMigrationDescriptor, MigrationMode, MigrationStatus};
use scryer_application::{AppError, AppResult};
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

const EMBEDDED_MIGRATION_CATALOG: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/migration_catalog.json.zst"));
const EMBEDDED_MIGRATION_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/migration_payload.bin.zst"));

#[derive(Debug, Clone)]
struct MigrationLedgerRow {
    version: i64,
    description: String,
    installed_on: String,
    success: bool,
    checksum_algo: String,
    checksum_algo_inferred: bool,
    checksum: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct MigrationHookContext {
    pub encryption_key: Option<EncryptionKey>,
}

pub fn list_embedded_migrations() -> AppResult<Vec<EmbeddedMigrationDescriptor>> {
    let catalog = embedded_catalog()?;
    Ok(catalog
        .migrations
        .iter()
        .map(|migration| EmbeddedMigrationDescriptor {
            filename: migration.filename.clone(),
            key: migration.key.clone(),
            checksum_algo: migration.checksum_algo.as_str().to_string(),
            checksum: migration_assets::checksum_hex(&migration.checksum),
        })
        .collect())
}

pub fn list_embedded_migration_keys() -> Vec<String> {
    embedded_catalog()
        .map(|catalog| {
            catalog
                .migrations
                .iter()
                .map(|migration| migration.key.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_source_migration_catalog() -> AppResult<CompiledMigrationBundle> {
    migration_assets::compile_source_bundle(&source_db_root())
        .map_err(|error| AppError::Repository(error.to_string()))
}

pub async fn replay_source_catalog_for_fresh_install(
    pool: &SqlitePool,
    through_version: Option<i64>,
    enable_baselines: bool,
) -> AppResult<()> {
    let bundle = load_source_migration_catalog()?;
    replay_catalog_into_fresh_db(
        pool,
        &bundle.catalog,
        &bundle.payload_bytes,
        through_version,
        enable_baselines,
    )
    .await
}

pub async fn replay_catalog_into_fresh_db(
    pool: &SqlitePool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    through_version: Option<i64>,
    enable_baselines: bool,
) -> AppResult<()> {
    replay_catalog_into_fresh_db_with_context(
        pool,
        catalog,
        payload_bytes,
        through_version,
        enable_baselines,
        &MigrationHookContext::default(),
    )
    .await
}

async fn replay_catalog_into_fresh_db_with_context(
    pool: &SqlitePool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    through_version: Option<i64>,
    enable_baselines: bool,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
    crate::spellfix::register_spellfix_auto_extension()?;
    ensure_migration_ledger_shape(pool).await?;

    let applied = load_applied_migrations(pool).await?;
    if !applied.is_empty() || app_object_count(pool).await? > 0 {
        return Err(AppError::Repository(
            "replay_catalog_into_fresh_db requires an empty database".to_string(),
        ));
    }

    let target_version = through_version.unwrap_or_else(|| catalog.max_version());
    if target_version <= 0 {
        return Ok(());
    }

    let mut start_version = 1;
    if enable_baselines
        && let Some(baseline) =
            catalog.latest_baseline_at_or_below(target_version, EngineScope::Sqlite)
    {
        apply_baseline(pool, catalog, payload_bytes, baseline).await?;
        start_version = baseline.through_version + 1;
    }

    apply_version_range(
        pool,
        catalog,
        payload_bytes,
        MigrationInstallKind::FreshInstall,
        start_version,
        target_version,
        hook_context,
    )
    .await
}

#[allow(dead_code)]
pub async fn run_migrations(pool: &SqlitePool, mode: MigrationMode) -> AppResult<()> {
    run_migrations_with_hook_context(pool, mode, MigrationHookContext::default()).await
}

pub async fn run_migrations_with_hook_context(
    pool: &SqlitePool,
    mode: MigrationMode,
    hook_context: MigrationHookContext,
) -> AppResult<()> {
    let catalog = embedded_catalog()?;
    let payload_bytes = embedded_payload_bytes()?;
    if !matches!(mode, MigrationMode::ValidateOnly) {
        ensure_migration_ledger_shape(pool).await?;
    }

    let applied = load_applied_migrations(pool).await?;
    validate_known_migrations(&applied, &catalog, &payload_bytes)?;
    let pending = list_pending_migrations_from_applied(&applied, &catalog);
    if pending.is_empty() {
        return Ok(());
    }

    if matches!(mode, MigrationMode::ValidateOnly) {
        return Err(AppError::Validation(format!(
            "database migration check failed; pending migrations: {}",
            pending.join(", ")
        )));
    }

    let install_kind = detect_install_kind(pool, &applied).await?;
    match install_kind {
        MigrationInstallKind::FreshInstall => {
            replay_catalog_into_fresh_db_with_context(
                pool,
                &catalog,
                &payload_bytes,
                None,
                true,
                &hook_context,
            )
            .await?;
        }
        MigrationInstallKind::Upgrade => {
            apply_version_range(
                pool,
                &catalog,
                &payload_bytes,
                MigrationInstallKind::Upgrade,
                1,
                catalog.max_version(),
                &hook_context,
            )
            .await?;
        }
    }

    Ok(())
}

fn source_db_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scryer/src/db")
}

pub fn embedded_catalog() -> AppResult<CompiledMigrationCatalog> {
    let bytes = zstd::stream::decode_all(EMBEDDED_MIGRATION_CATALOG).map_err(|error| {
        AppError::Repository(format!("failed to decompress migration catalog: {error}"))
    })?;

    migration_assets::decode_catalog(&bytes).map_err(AppError::Repository)
}

pub fn embedded_payload_bytes() -> AppResult<Vec<u8>> {
    zstd::stream::decode_all(EMBEDDED_MIGRATION_PAYLOAD).map_err(|error| {
        AppError::Repository(format!("failed to decompress migration payload: {error}"))
    })
}

fn checksum_algo_from_str(value: &str) -> Option<ChecksumAlgorithm> {
    match value.trim() {
        "sha384" => Some(ChecksumAlgorithm::Sha384),
        "blake3" => Some(ChecksumAlgorithm::Blake3),
        _ => None,
    }
}

async fn ensure_migration_ledger_shape(pool: &SqlitePool) -> AppResult<()> {
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL,
    checksum_algo TEXT NOT NULL DEFAULT ''
)
        "#,
    )
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    let checksum_algo_exists = migration_table_has_column(pool, "checksum_algo").await?;

    if !checksum_algo_exists {
        sqlx::query(
            "ALTER TABLE _sqlx_migrations ADD COLUMN checksum_algo TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    }

    Ok(())
}

async fn migration_table_exists(pool: &SqlitePool) -> AppResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE type = 'table'
            AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(count > 0)
}

async fn migration_table_has_column(pool: &SqlitePool, column_name: &str) -> AppResult<bool> {
    if !migration_table_exists(pool).await? {
        return Ok(false);
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = ?1",
    )
    .bind(column_name)
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(count > 0)
}

async fn load_applied_migrations(pool: &SqlitePool) -> AppResult<Vec<MigrationLedgerRow>> {
    if !migration_table_exists(pool).await? {
        return Ok(Vec::new());
    }

    let has_checksum_algo = migration_table_has_column(pool, "checksum_algo").await?;
    let rows = if has_checksum_algo {
        sqlx::query(
            "SELECT
                 version,
                 description,
                 installed_on,
                 success,
                 checksum,
                 COALESCE(TRIM(checksum_algo), '') AS checksum_algo,
                 CASE WHEN COALESCE(TRIM(checksum_algo), '') = '' THEN 1 ELSE 0 END AS checksum_algo_inferred
             FROM _sqlx_migrations
             ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    } else {
        sqlx::query(
            "SELECT
                 version,
                 description,
                 installed_on,
                 success,
                 checksum,
                 '' AS checksum_algo,
                 1 AS checksum_algo_inferred
             FROM _sqlx_migrations
             ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    };

    rows.into_iter()
        .map(|row| {
            Ok(MigrationLedgerRow {
                version: row
                    .try_get("version")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                description: row
                    .try_get("description")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                installed_on: row
                    .try_get("installed_on")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                success: {
                    let success: i64 = row
                        .try_get("success")
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                    success != 0
                },
                checksum: row
                    .try_get("checksum")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                checksum_algo: row
                    .try_get("checksum_algo")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                checksum_algo_inferred: row
                    .try_get::<i64, _>("checksum_algo_inferred")
                    .map_err(|error| AppError::Repository(error.to_string()))?
                    != 0,
            })
        })
        .collect()
}

fn list_pending_migrations_from_applied(
    applied: &[MigrationLedgerRow],
    catalog: &CompiledMigrationCatalog,
) -> Vec<String> {
    let applied_versions: HashSet<i64> = applied
        .iter()
        .filter(|row| row.success)
        .map(|row| row.version)
        .collect();

    catalog
        .migrations
        .iter()
        .filter(|migration| !applied_versions.contains(&migration.version))
        .map(|migration| migration.key.clone())
        .collect()
}

fn validate_known_migrations(
    applied: &[MigrationLedgerRow],
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
) -> AppResult<()> {
    let max_supported_version = catalog.max_version();
    let mut unknown = Vec::new();
    let mut too_new = Vec::new();
    let mut invalid_checksum = Vec::new();

    for row in applied {
        if !row.success {
            return Err(AppError::Repository(format!(
                "migration {} was not applied successfully",
                migration_assets::migration_key_from_version_and_desc(
                    row.version,
                    &row.description
                )
            )));
        }

        let key =
            migration_assets::migration_key_from_version_and_desc(row.version, &row.description);
        let Some(expected) = catalog.find_migration(row.version) else {
            if row.version > max_supported_version {
                too_new.push(key);
            } else {
                unknown.push(key);
            }
            continue;
        };

        if row.checksum_algo_inferred {
            if row.checksum == expected.checksum
                || legacy_sqlite_checksum_matches(
                    ChecksumAlgorithm::Sha384,
                    &row.checksum,
                    expected,
                    payload_bytes,
                )
            {
                continue;
            }
            invalid_checksum.push(key);
            continue;
        }

        let row_algo = match checksum_algo_from_str(&row.checksum_algo) {
            Some(value) => value,
            None => {
                invalid_checksum.push(format!("{key} ({})", row.checksum_algo));
                continue;
            }
        };

        if row_algo == expected.checksum_algo && row.checksum == expected.checksum {
            continue;
        }

        if legacy_sqlite_checksum_matches(row_algo, &row.checksum, expected, payload_bytes) {
            continue;
        }

        {
            invalid_checksum.push(key);
        }
    }

    let mut problems = Vec::new();

    if !invalid_checksum.is_empty() {
        problems.push(format!(
            "checksum mismatch for migrations: {}",
            invalid_checksum.join(", ")
        ));
    }

    if !unknown.is_empty() {
        problems.push(format!(
            "unsupported migration keys: {}. Please update scryer or restore a compatible database snapshot.",
            unknown.join(", ")
        ));
    }

    if !too_new.is_empty() {
        problems.push(format!(
            "migrations newer than supported ({max_supported_version}): {}. Please update scryer.",
            too_new.join(", ")
        ));
    }

    if !problems.is_empty() {
        return Err(AppError::Repository(problems.join(" ")));
    }

    Ok(())
}

fn legacy_sqlite_checksum_matches(
    row_algo: ChecksumAlgorithm,
    row_checksum: &[u8],
    expected: &CompiledMigration,
    payload_bytes: &[u8],
) -> bool {
    if row_algo != ChecksumAlgorithm::Sha384 {
        return false;
    }

    let mut sqlite_sql_step = None;
    for step in &expected.steps {
        match step {
            CompiledMigrationStep::Sql {
                engine, payload, ..
            } if engine.applies_to(EngineScope::Sqlite) => {
                if sqlite_sql_step.is_some() {
                    return false;
                }
                sqlite_sql_step = Some(payload);
            }
            CompiledMigrationStep::Rust { engine, .. }
                if engine.applies_to(EngineScope::Sqlite) =>
            {
                return false;
            }
            _ => {}
        }
    }

    sqlite_sql_step
        .and_then(|payload| payload.bytes(payload_bytes).ok())
        .is_some_and(|bytes| ChecksumAlgorithm::Sha384.digest(bytes) == row_checksum)
}

async fn detect_install_kind(
    pool: &SqlitePool,
    applied: &[MigrationLedgerRow],
) -> AppResult<MigrationInstallKind> {
    if !applied.is_empty() {
        return Ok(MigrationInstallKind::Upgrade);
    }

    let app_objects = app_object_count(pool).await?;
    if app_objects == 0 {
        Ok(MigrationInstallKind::FreshInstall)
    } else {
        Err(AppError::Repository(
            "database contains application schema or data but has no applied migration ledger"
                .to_string(),
        ))
    }
}

async fn app_object_count(pool: &SqlitePool) -> AppResult<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE name NOT LIKE 'sqlite_%'
            AND name NOT LIKE '_sqlx_%'",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))
}

async fn apply_baseline(
    pool: &SqlitePool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    baseline: &CompiledBaseline,
) -> AppResult<()> {
    let sql = baseline
        .payload
        .text(payload_bytes)
        .map_err(AppError::Repository)?
        .to_owned();
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    if !sql.trim().is_empty() {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
    }

    for migration in catalog
        .migrations
        .iter()
        .filter(|migration| migration.version <= baseline.through_version)
    {
        insert_applied_migration(&mut tx, migration, 0).await?;
    }

    tx.commit()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}

async fn apply_version_range(
    pool: &SqlitePool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    install_kind: MigrationInstallKind,
    start_version: i64,
    target_version: i64,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
    if target_version < start_version {
        return Ok(());
    }

    let applied_versions: HashSet<i64> = load_applied_migrations(pool)
        .await?
        .into_iter()
        .filter(|row| row.success)
        .map(|row| row.version)
        .collect();

    for migration in catalog.migrations.iter().filter(|migration| {
        migration.version >= start_version && migration.version <= target_version
    }) {
        if applied_versions.contains(&migration.version) {
            continue;
        }

        apply_single_migration(pool, migration, payload_bytes, install_kind, hook_context).await?;
    }

    Ok(())
}

async fn apply_single_migration(
    pool: &SqlitePool,
    migration: &CompiledMigration,
    payload_bytes: &[u8],
    install_kind: MigrationInstallKind,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
    let start = Instant::now();
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    if let Some(quarantined) = known_bad::known_bad_migration(migration.version) {
        // Recorded exactly as if it had run (same version/description/checksum,
        // so ledger validation on every later boot is unchanged), but none of
        // its steps execute. The replacement version carries the safe work.
        tracing::warn!(
            version = migration.version,
            replacement_version = quarantined.replacement_version,
            reason = quarantined.reason,
            "skipping quarantined migration; recording it as applied without executing its steps"
        );
        insert_applied_migration(&mut tx, migration, 0).await?;
        tx.commit()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        return Ok(());
    }

    for step in &migration.steps {
        if !step.engine().applies_to(EngineScope::Sqlite) {
            continue;
        }

        if !step.scope().applies_to(install_kind) {
            continue;
        }

        match step {
            CompiledMigrationStep::Sql { payload, .. } => {
                let sql = payload
                    .text(payload_bytes)
                    .map_err(AppError::Repository)?
                    .to_owned();
                if sql.trim().is_empty() {
                    continue;
                }
                sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                    .execute(&mut *tx)
                    .await
                    .map_err(|error| {
                        AppError::Repository(map_migration_execute_error(
                            migration.version,
                            error.to_string(),
                        ))
                    })?;
            }
            CompiledMigrationStep::Rust { hook_id, .. } => {
                run_rust_hook(
                    hook_id.clone(),
                    &mut tx,
                    migration.version,
                    install_kind,
                    hook_context,
                )
                .await?;
            }
        }
    }

    let elapsed_ns = start.elapsed().as_nanos().min(i64::MAX as u128) as i64;
    insert_applied_migration(&mut tx, migration, elapsed_ns).await?;
    tx.commit()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(())
}

async fn insert_applied_migration(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    migration: &CompiledMigration,
    execution_time: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, success, checksum, execution_time, checksum_algo)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)",
    )
    .bind(migration.version)
    .bind(&migration.description)
    .bind(&migration.checksum)
    .bind(execution_time)
    .bind(migration.checksum_algo.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}

#[cfg_attr(not(test), allow(unused_variables))]
async fn run_rust_hook(
    hook_id: String,
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    version: i64,
    install_kind: MigrationInstallKind,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
    migration_hook_ids::validate_migration_hook_id(&hook_id).map_err(AppError::Repository)?;
    match hook_id.as_str() {
        "migrate_jellyfin_notification_channels_to_media_server_targets" => {
            notification_targets::migrate_jellyfin_notification_channels_to_media_server_targets_sqlite(
                tx,
                hook_context.encryption_key.as_ref(),
            )
            .await
        }
        "migrate_title_root_folder_ids" => {
            title_root_folder_ids::migrate_title_root_folder_ids_sqlite(tx).await
        }
        "migrate_title_catalog_sort_keys" => {
            title_catalog_sort_keys::migrate_title_catalog_sort_keys_sqlite(tx).await
        }
        "migrate_title_folder_ownership" => {
            title_folder_ownership::migrate_title_folder_ownership_sqlite(tx).await
        }
        "migrate_title_folder_ownership_safe" => {
            title_folder_ownership_safe::migrate_title_folder_ownership_safe_sqlite(tx).await
        }
        "migrate_title_image_blobs" => {
            title_image_blobs::migrate_title_image_blobs_sqlite(tx).await
        }
        "converge_post_0_16_6_prerelease_schema" => {
            post_0_16_6_prerelease::converge_post_0_16_6_prerelease_schema_sqlite(tx).await
        }
        "backfill_canonical_download_identity" => {
            canonical_download_identity::backfill_canonical_download_identity_sqlite(tx).await
        }
        "disable_invalid_user_rule_runtime_wrappers" => {
            rule_set_runtime_wrapper::disable_invalid_user_rule_runtime_wrappers_sqlite(tx).await
        }
        "backfill_blake3_identities" => {
            blake3_identities::backfill_blake3_identities_sqlite(tx).await
        }
        "compact_event_storage" => event_storage::compact_event_storage_sqlite(tx).await,
        "compress_post_processing_output" => {
            post_processing_output::compress_post_processing_output_sqlite(tx).await
        }
        #[cfg(test)]
        "test_insert_hook_marker" => {
            let marker = match install_kind {
                MigrationInstallKind::FreshInstall => "fresh",
                MigrationInstallKind::Upgrade => "upgrade",
            };
            sqlx::query("INSERT INTO migration_hook_markers (version, marker) VALUES (?1, ?2)")
                .bind(version)
                .bind(marker)
                .execute(&mut **tx)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            Ok(())
        }
        _ => Err(AppError::Repository(format!(
            "unknown migration hook id '{hook_id}'"
        ))),
    }
}

fn map_migration_execute_error(version: i64, error_message: String) -> String {
    if version != 79
        && version != 103
        && !is_title_external_id_projection_conflict_error(&error_message)
    {
        return error_message;
    }

    if is_title_external_id_projection_conflict_error(&error_message) {
        format!(
            "{error_message}. Conflicting faceted external IDs detected while rebuilding title_external_ids. Resolve the duplicate entries in titles.external_ids and rerun startup."
        )
    } else {
        error_message
    }
}

fn is_title_external_id_projection_conflict_error(message: &str) -> bool {
    message.contains("UNIQUE constraint failed")
        && (message.contains("_title_external_id_projection_check")
            || message.contains("title_external_ids.facet")
            || message.contains("idx_title_external_ids_facet_lookup"))
}

#[doc(hidden)]
pub async fn title_external_id_projection_conflict_hint(pool: &SqlitePool) -> Option<String> {
    let rows = sqlx::query(
        "SELECT
             facet,
             source,
             external_id,
             GROUP_CONCAT(DISTINCT title_id) AS title_ids
         FROM (
             SELECT
                 t.id AS title_id,
                 t.facet AS facet,
                 LOWER(TRIM(json_extract(external_id.value, '$.source'))) AS source,
                 TRIM(json_extract(external_id.value, '$.value')) AS external_id
             FROM titles AS t
             JOIN json_each(t.external_ids) AS external_id
             WHERE TRIM(COALESCE(json_extract(external_id.value, '$.source'), '')) != ''
               AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
             GROUP BY
                 t.id,
                 t.facet,
                 LOWER(TRIM(json_extract(external_id.value, '$.source'))),
                 TRIM(json_extract(external_id.value, '$.value'))
         ) AS canonical
         GROUP BY facet, source, external_id
         HAVING COUNT(DISTINCT title_id) > 1
         ORDER BY facet, source, external_id
         LIMIT 5",
    )
    .fetch_all(pool)
    .await
    .ok()?;

    if rows.is_empty() {
        return None;
    }

    let collisions = rows
        .into_iter()
        .filter_map(|row| {
            let facet: String = row.try_get("facet").ok()?;
            let source: String = row.try_get("source").ok()?;
            let external_id: String = row.try_get("external_id").ok()?;
            let title_ids: Option<String> = row.try_get("title_ids").ok();
            let title_ids = title_ids.unwrap_or_default();

            Some(if title_ids.is_empty() {
                format!("{facet}/{source}/{external_id}")
            } else {
                format!("{facet}/{source}/{external_id} (titles: {title_ids})")
            })
        })
        .collect::<Vec<_>>();

    if collisions.is_empty() {
        None
    } else {
        Some(collisions.join("; "))
    }
}

pub async fn list_applied_migrations(pool: &SqlitePool) -> AppResult<Vec<MigrationStatus>> {
    let rows = load_applied_migrations(pool).await?;
    let catalog = embedded_catalog()?;
    let payload_bytes = embedded_payload_bytes()?;
    let mut out = Vec::with_capacity(rows.len());

    for row in rows {
        let checksum_algo = if row.checksum_algo_inferred {
            catalog
                .find_migration(row.version)
                .map(|expected| {
                    if row.checksum == expected.checksum {
                        expected.checksum_algo.as_str().to_string()
                    } else if legacy_sqlite_checksum_matches(
                        ChecksumAlgorithm::Sha384,
                        &row.checksum,
                        expected,
                        &payload_bytes,
                    ) {
                        ChecksumAlgorithm::Sha384.as_str().to_string()
                    } else {
                        "inferred".to_string()
                    }
                })
                .unwrap_or_else(|| "inferred".to_string())
        } else {
            row.checksum_algo.clone()
        };
        out.push(MigrationStatus {
            migration_key: migration_assets::migration_key_from_version_and_desc(
                row.version,
                &row.description,
            ),
            migration_checksum_algo: checksum_algo,
            migration_checksum: migration_assets::checksum_hex(&row.checksum),
            applied_at: row.installed_on,
            success: row.success,
            error_message: None,
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        });
    }

    Ok(out)
}
