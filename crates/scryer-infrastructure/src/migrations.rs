use scryer_application::{AppError, AppResult};
use sqlx::SqlitePool;
use std::collections::HashSet;

use crate::{EmbeddedMigrationDescriptor, MigrationMode, MigrationStatus};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../scryer/src/db/migrations");

fn migration_key_from_version_and_desc(version: i64, description: &str) -> String {
    format!("{:04}_{}", version, description.replace(' ', "_"))
}

fn hex_checksum(raw: &[u8]) -> String {
    raw.iter()
        .map(|value| format!("{value:02x}"))
        .collect::<String>()
}

pub fn list_embedded_migrations() -> AppResult<Vec<EmbeddedMigrationDescriptor>> {
    let mut migrations = Vec::new();

    for migration in MIGRATOR.iter() {
        migrations.push(EmbeddedMigrationDescriptor {
            filename: format!(
                "{}.sql",
                migration_key_from_version_and_desc(migration.version, &migration.description)
            ),
            key: migration_key_from_version_and_desc(migration.version, &migration.description),
            checksum: hex_checksum(&migration.checksum),
        });
    }

    Ok(migrations)
}

pub fn list_embedded_migration_keys() -> Vec<String> {
    MIGRATOR
        .iter()
        .map(|migration| {
            migration_key_from_version_and_desc(migration.version, &migration.description)
        })
        .collect()
}

pub(crate) async fn run_migrations(pool: &SqlitePool, mode: MigrationMode) -> AppResult<()> {
    validate_known_migrations(pool).await?;
    let pending = list_pending_migrations(pool).await?;
    if pending.is_empty() {
        return Ok(());
    }

    match mode {
        MigrationMode::ValidateOnly => {
            return Err(AppError::Validation(format!(
                "database migration check failed; pending migrations: {}",
                pending.join(", ")
            )));
        }
        MigrationMode::Apply => {}
    }

    MIGRATOR
        .run(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    Ok(())
}

async fn migration_table_exists(pool: &SqlitePool) -> AppResult<bool> {
    let row = sqlx::query_as::<_, (i32,)>(
        "SELECT 1
           FROM sqlite_master
          WHERE type='table'
            AND name = '_sqlx_migrations'
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(row.is_some())
}

async fn applied_sqlx_migrations(
    pool: &SqlitePool,
) -> AppResult<Vec<(i64, String, String, i64, Vec<u8>)>> {
    if !migration_table_exists(pool).await? {
        return Ok(Vec::new());
    }

    let rows = sqlx::query_as::<_, (i64, String, String, i64, Vec<u8>)>(
        "SELECT version, description, installed_on, success, checksum
           FROM _sqlx_migrations
          ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(rows)
}

async fn list_pending_migrations(pool: &SqlitePool) -> AppResult<Vec<String>> {
    let applied = applied_sqlx_migrations(pool).await?;
    let mut applied_set = HashSet::new();

    for (version, _, _, success, _) in applied {
        if success == 0 {
            continue;
        }
        applied_set.insert(version);
    }

    let mut pending = Vec::new();
    for migration in MIGRATOR.iter() {
        if !applied_set.contains(&migration.version) {
            pending.push(migration_key_from_version_and_desc(
                migration.version,
                &migration.description,
            ));
        }
    }
    Ok(pending)
}

async fn validate_known_migrations(pool: &SqlitePool) -> AppResult<()> {
    let applied = applied_sqlx_migrations(pool).await?;
    let max_supported_version = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
    let mut unknown = Vec::new();
    let mut too_new = Vec::new();
    let mut invalid_checksum = Vec::new();

    for (version, description, _, success, checksum) in applied {
        if success == 0 {
            return Err(AppError::Repository(format!(
                "migration {} was not applied successfully",
                migration_key_from_version_and_desc(version, &description)
            )));
        }

        let key = migration_key_from_version_and_desc(version, &description);
        let row_checksum = hex_checksum(&checksum);

        if let Some(migration) = MIGRATOR
            .iter()
            .find(|migration| migration.version == version)
        {
            let expected_checksum = hex_checksum(&migration.checksum);
            if row_checksum != expected_checksum {
                invalid_checksum.push(key);
            }
            continue;
        }

        if version > max_supported_version {
            too_new.push(key);
            continue;
        }

        unknown.push(key);
    }

    if !invalid_checksum.is_empty() || !unknown.is_empty() || !too_new.is_empty() {
        let mut reasons = Vec::new();

        if !invalid_checksum.is_empty() {
            reasons.push(format!(
                "checksum mismatch for migrations: {}",
                invalid_checksum.join(", ")
            ));
        }
        if !unknown.is_empty() {
            reasons.push(format!(
                "unsupported migration keys: {}",
                unknown.join(", ")
            ));
        }
        if !too_new.is_empty() {
            reasons.push(format!(
                "migrations newer than supported ({max_supported_version}): {}",
                too_new.join(", ")
            ));
        }

        return Err(AppError::Repository(format!(
            "{}. Please update scryer to a newer binary or point this instance at a database created by the current release.",
            reasons.join("; ")
        )));
    }

    Ok(())
}

pub(crate) async fn list_applied_migrations(pool: &SqlitePool) -> AppResult<Vec<MigrationStatus>> {
    let rows = applied_sqlx_migrations(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for (version, description, applied_at, success, checksum) in rows {
        let migration_key = migration_key_from_version_and_desc(version, &description);
        out.push(MigrationStatus {
            migration_key,
            migration_checksum: hex_checksum(&checksum),
            applied_at,
            success: success != 0,
            error_message: None,
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        });
    }
    Ok(out)
}
