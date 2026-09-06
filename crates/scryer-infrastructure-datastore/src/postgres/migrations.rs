use std::collections::HashSet;
use std::time::Instant;

use scryer_application::{AppError, AppResult};
use sqlx::{PgPool, Row};

use crate::migration_assets::{
    self, CompiledBaseline, CompiledMigration, CompiledMigrationCatalog, CompiledMigrationStep,
    EngineScope, MigrationInstallKind,
};
use crate::migrations::MigrationHookContext;
use crate::{MigrationMode, MigrationStatus};

pub async fn replay_source_catalog_for_fresh_install(
    pool: &PgPool,
    through_version: Option<i64>,
) -> AppResult<()> {
    let bundle = crate::migrations::load_source_migration_catalog()?;
    replay_catalog_into_fresh_db(
        pool,
        &bundle.catalog,
        &bundle.payload_bytes,
        through_version,
    )
    .await
}

pub async fn replay_catalog_into_fresh_db(
    pool: &PgPool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    through_version: Option<i64>,
) -> AppResult<()> {
    ensure_migration_ledger_shape(pool).await?;

    let applied = load_applied_migrations(pool).await?;
    if !applied.is_empty() || app_object_count(pool).await? > 0 {
        return Err(AppError::Repository(
            "replay_catalog_into_fresh_db requires an empty PostgreSQL database".to_string(),
        ));
    }

    let target_version = through_version.unwrap_or_else(|| catalog.max_version());
    if target_version <= 0 {
        return Ok(());
    }

    let baseline = catalog
        .latest_baseline_at_or_below(target_version, EngineScope::Postgres)
        .ok_or_else(|| {
            AppError::Repository(format!(
                "missing PostgreSQL baseline at or below {target_version:04}"
            ))
        })?;

    apply_postgres_baseline(pool, catalog, payload_bytes, baseline).await?;
    apply_version_range(
        pool,
        catalog,
        payload_bytes,
        MigrationInstallKind::FreshInstall,
        baseline.through_version + 1,
        target_version,
        &MigrationHookContext::default(),
    )
    .await
}

#[allow(dead_code)]
pub async fn run_migrations(pool: &PgPool, mode: MigrationMode) -> AppResult<()> {
    run_migrations_with_hook_context(pool, mode, MigrationHookContext::default()).await
}

pub async fn run_migrations_with_hook_context(
    pool: &PgPool,
    mode: MigrationMode,
    hook_context: MigrationHookContext,
) -> AppResult<()> {
    let catalog = crate::migrations::embedded_catalog()?;

    if !matches!(mode, MigrationMode::ValidateOnly) {
        ensure_migration_ledger_shape(pool).await?;
    }

    let applied = load_applied_migrations(pool).await?;
    validate_known_migrations(&applied, &catalog)?;
    let pending = list_pending_migrations_from_applied(&applied, &catalog);
    if pending.is_empty() {
        return Ok(());
    }

    if matches!(mode, MigrationMode::ValidateOnly) {
        return Err(AppError::Validation(format!(
            "database migration check failed; pending PostgreSQL migrations: {}",
            pending.join(", ")
        )));
    }

    let install_kind = detect_install_kind(pool, &applied).await?;
    let payload_bytes = crate::migrations::embedded_payload_bytes()?;
    match install_kind {
        MigrationInstallKind::FreshInstall => {
            let target_version = catalog.max_version();
            let baseline = catalog
                .latest_baseline_at_or_below(target_version, EngineScope::Postgres)
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "missing PostgreSQL baseline through {target_version:04}"
                    ))
                })?;
            apply_postgres_baseline(pool, &catalog, &payload_bytes, baseline).await?;
            apply_version_range(
                pool,
                &catalog,
                &payload_bytes,
                MigrationInstallKind::FreshInstall,
                baseline.through_version + 1,
                target_version,
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

async fn ensure_migration_ledger_shape(pool: &PgPool) -> AppResult<()> {
    sqlx::raw_sql(
        r#"
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL,
    checksum_algo TEXT NOT NULL DEFAULT 'sha384',
    runtime_version TEXT NOT NULL DEFAULT '',
    error_message TEXT
)
        "#,
    )
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(())
}

async fn migration_table_exists(pool: &PgPool) -> AppResult<bool> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1
              FROM information_schema.tables
             WHERE table_schema = current_schema()
               AND table_name = '_sqlx_migrations'
        )",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(exists)
}

#[derive(Clone, Debug)]
struct MigrationLedgerRow {
    version: i64,
    description: String,
    installed_on: String,
    success: bool,
    checksum_algo: String,
    checksum: Vec<u8>,
}

async fn load_applied_migrations(pool: &PgPool) -> AppResult<Vec<MigrationLedgerRow>> {
    if !migration_table_exists(pool).await? {
        return Ok(Vec::new());
    }

    let rows = sqlx::query(
        "SELECT
             version,
             description,
             installed_on::TEXT AS installed_on,
             success,
             checksum,
             COALESCE(NULLIF(BTRIM(checksum_algo), ''), 'sha384') AS checksum_algo
         FROM _sqlx_migrations
         ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

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
                success: row
                    .try_get("success")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                checksum: row
                    .try_get("checksum")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                checksum_algo: row
                    .try_get("checksum_algo")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
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
) -> AppResult<()> {
    let max_supported_version = catalog.max_version();
    let mut unknown = Vec::new();
    let mut invalid_checksum = Vec::new();

    for row in applied {
        if !row.success {
            return Err(AppError::Repository(format!(
                "PostgreSQL migration {} was not applied successfully",
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
                unknown.push(key);
            }
            continue;
        };

        if row.checksum_algo != expected.checksum_algo.as_str() || row.checksum != expected.checksum
        {
            invalid_checksum.push(key);
        }
    }

    if !invalid_checksum.is_empty() {
        return Err(AppError::Repository(format!(
            "checksum mismatch for PostgreSQL migrations: {}",
            invalid_checksum.join(", ")
        )));
    }

    if !unknown.is_empty() {
        return Err(AppError::Repository(format!(
            "PostgreSQL migrations newer than supported ({max_supported_version}): {}. Please update scryer.",
            unknown.join(", ")
        )));
    }

    Ok(())
}

async fn detect_install_kind(
    pool: &PgPool,
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
            "PostgreSQL database contains application schema or data but has no applied migration ledger".to_string(),
        ))
    }
}

async fn app_object_count(pool: &PgPool) -> AppResult<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM information_schema.tables
          WHERE table_schema = current_schema()
            AND table_name NOT LIKE '_sqlx_%'",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))
}

async fn apply_postgres_baseline(
    pool: &PgPool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    baseline: &CompiledBaseline,
) -> AppResult<()> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    let baseline_sql = baseline
        .payload
        .text(payload_bytes)
        .map_err(AppError::Repository)?;

    sqlx::raw_sql(sqlx::AssertSqlSafe(baseline_sql.to_owned()))
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

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
    pool: &PgPool,
    catalog: &CompiledMigrationCatalog,
    payload_bytes: &[u8],
    install_kind: MigrationInstallKind,
    start_version: i64,
    target_version: i64,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
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
    pool: &PgPool,
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

    if let Some(quarantined) = crate::migrations::known_bad::known_bad_migration(migration.version)
    {
        // See `migrations::known_bad`: recorded as applied with its original
        // checksum, steps never executed; the replacement version does the
        // safe work.
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
        if !step.engine().applies_to(EngineScope::Postgres)
            || !step.scope().applies_to(install_kind)
        {
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
                        AppError::Repository(format!(
                            "failed to apply PostgreSQL migration {:04}: {error}",
                            migration.version
                        ))
                    })?;
            }
            CompiledMigrationStep::Rust { hook_id, .. } => {
                run_postgres_rust_hook(
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

#[cfg_attr(not(test), allow(unused_variables))]
async fn run_postgres_rust_hook(
    hook_id: String,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version: i64,
    install_kind: MigrationInstallKind,
    hook_context: &MigrationHookContext,
) -> AppResult<()> {
    crate::migration_hook_ids::validate_migration_hook_id(&hook_id)
        .map_err(AppError::Repository)?;
    match hook_id.as_str() {
        "migrate_jellyfin_notification_channels_to_media_server_targets" => {
            crate::migrations::notification_targets::migrate_jellyfin_notification_channels_to_media_server_targets_postgres(
                tx,
                hook_context.encryption_key.as_ref(),
            )
            .await
        }
        "migrate_title_root_folder_ids" => {
            crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_postgres(tx)
                .await
        }
        "migrate_title_catalog_sort_keys" => {
            crate::migrations::title_catalog_sort_keys::migrate_title_catalog_sort_keys_postgres(tx)
                .await
        }
        "migrate_title_folder_ownership" => {
            crate::migrations::title_folder_ownership::migrate_title_folder_ownership_postgres(tx)
                .await
        }
        "migrate_title_folder_ownership_safe" => {
            crate::migrations::title_folder_ownership_safe::migrate_title_folder_ownership_safe_postgres(tx)
                .await
        }
        "migrate_title_image_blobs" => {
            crate::migrations::title_image_blobs::migrate_title_image_blobs_postgres(tx).await
        }
        "converge_post_0_16_6_prerelease_schema" => {
            crate::migrations::post_0_16_6_prerelease::converge_post_0_16_6_prerelease_schema_postgres(tx)
                .await
        }
        "backfill_canonical_download_identity" => {
            crate::migrations::canonical_download_identity::backfill_canonical_download_identity_postgres(tx)
                .await
        }
        "disable_invalid_user_rule_runtime_wrappers" => {
            crate::migrations::rule_set_runtime_wrapper::disable_invalid_user_rule_runtime_wrappers_postgres(tx)
                .await
        }
        "backfill_blake3_identities" => {
            crate::migrations::blake3_identities::backfill_blake3_identities_postgres(tx).await
        }
        "compact_event_storage" => {
            crate::migrations::event_storage::compact_event_storage_postgres(tx).await
        }
        "compress_post_processing_output" => {
            crate::migrations::post_processing_output::compress_post_processing_output_postgres(tx)
                .await
        }
        #[cfg(test)]
        "test_insert_hook_marker" => {
            let marker = match install_kind {
                MigrationInstallKind::FreshInstall => "fresh",
                MigrationInstallKind::Upgrade => "upgrade",
            };
            sqlx::query("INSERT INTO migration_hook_markers (version, marker) VALUES ($1, $2)")
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

async fn insert_applied_migration(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    migration: &CompiledMigration,
    execution_time: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, success, checksum, execution_time, checksum_algo, runtime_version)
         VALUES ($1, $2, TRUE, $3, $4, $5, $6)
         ON CONFLICT (version) DO NOTHING",
    )
    .bind(migration.version)
    .bind(&migration.description)
    .bind(&migration.checksum)
    .bind(execution_time)
    .bind(migration.checksum_algo.as_str())
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}

pub async fn list_applied_migrations(pool: &PgPool) -> AppResult<Vec<MigrationStatus>> {
    let rows = load_applied_migrations(pool).await?;
    let mut out = Vec::with_capacity(rows.len());

    for row in rows {
        out.push(MigrationStatus {
            migration_key: migration_assets::migration_key_from_version_and_desc(
                row.version,
                &row.description,
            ),
            migration_checksum_algo: row.checksum_algo,
            migration_checksum: migration_assets::checksum_hex(&row.checksum),
            applied_at: row.installed_on,
            success: row.success,
            error_message: None,
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        });
    }

    Ok(out)
}
