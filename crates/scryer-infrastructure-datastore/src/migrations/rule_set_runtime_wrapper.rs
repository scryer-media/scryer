//! Disable persisted user rules that cannot execute through the generated
//! `eval_rule` runtime wrapper.
//!
//! The migration intentionally leaves source intact and does not touch managed
//! rules. A user can inspect and repair a disabled rule after upgrade; an
//! incompatible legacy rule must not prevent the runtime engine from starting.

use scryer_application::{AppError, AppResult};
use sqlx::Row;
use tracing::warn;

#[derive(Debug)]
struct PersistedUserRule {
    id: String,
    rego_source: String,
    enabled: bool,
}

pub async fn disable_invalid_user_rule_runtime_wrappers_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rules = sqlite_user_rules(tx).await?;
    for rule in rules {
        if !runtime_wrapper_is_valid(&rule) && rule.enabled {
            warn!(
                rule_id = rule.id.as_str(),
                "disabled user rule that is incompatible with the eval_rule runtime wrapper"
            );
            sqlx::query(
                "UPDATE rule_sets
                    SET enabled = 0,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                  WHERE id = ?1
                    AND enabled != 0",
            )
            .bind(&rule.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        }
    }
    Ok(())
}

pub async fn disable_invalid_user_rule_runtime_wrappers_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rules = postgres_user_rules(tx).await?;
    for rule in rules {
        if !runtime_wrapper_is_valid(&rule) && rule.enabled {
            warn!(
                rule_id = rule.id.as_str(),
                "disabled user rule that is incompatible with the eval_rule runtime wrapper"
            );
            sqlx::query(
                "UPDATE rule_sets
                    SET enabled = FALSE,
                        updated_at = NOW()
                  WHERE id = $1
                    AND enabled",
            )
            .bind(&rule.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        }
    }
    Ok(())
}

fn runtime_wrapper_is_valid(rule: &PersistedUserRule) -> bool {
    match scryer_rules::validation::validate_runtime_wrapper(&rule.rego_source, &rule.id) {
        Ok(validation) if validation.valid => true,
        Ok(_) | Err(_) => false,
    }
}

async fn sqlite_user_rules(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<PersistedUserRule>> {
    let rows = sqlx::query(
        "SELECT id, rego_source, enabled
           FROM rule_sets
          WHERE is_managed = 0
          ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter().map(sqlite_row).collect()
}

async fn postgres_user_rules(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<PersistedUserRule>> {
    let rows = sqlx::query(
        "SELECT id, rego_source, enabled
           FROM rule_sets
          WHERE is_managed = FALSE
          ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter().map(postgres_row).collect()
}

fn sqlite_row(row: sqlx::sqlite::SqliteRow) -> AppResult<PersistedUserRule> {
    Ok(PersistedUserRule {
        id: row.try_get("id").map_err(repo_err)?,
        rego_source: row.try_get("rego_source").map_err(repo_err)?,
        enabled: row.try_get::<i64, _>("enabled").map_err(repo_err)? != 0,
    })
}

fn postgres_row(row: sqlx::postgres::PgRow) -> AppResult<PersistedUserRule> {
    Ok(PersistedUserRule {
        id: row.try_get("id").map_err(repo_err)?,
        rego_source: row.try_get("rego_source").map_err(repo_err)?,
        enabled: row.try_get("enabled").map_err(repo_err)?,
    })
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_assets::{
        CompiledMigrationStep, EngineScope, MigrationInstallKind, StepScope,
    };

    #[derive(Debug, Eq, PartialEq)]
    struct RuleState {
        rego_source: String,
        enabled: bool,
        updated_at: String,
    }

    #[tokio::test]
    async fn sqlite_hook_disables_only_user_rules_that_fail_the_runtime_wrapper() {
        let pool = test_pool().await;

        insert_rule(
            &pool,
            "valid",
            "package scryer.rules.user.valid\nimport rego.v1\nscore_entry[\"bonus\"] := 100",
            true,
            false,
        )
        .await;
        insert_rule(
            &pool,
            "runtime_failure",
            "package scryer.rules.user.runtime_failure\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)",
            true,
            false,
        )
        .await;
        insert_rule(
            &pool,
            "managed_runtime_failure",
            "package scryer.rules.user.managed_runtime_failure\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)",
            true,
            true,
        )
        .await;
        insert_rule(
            &pool,
            "already_disabled",
            "package scryer.rules.user.already_disabled\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)",
            false,
            false,
        )
        .await;

        run_sqlite_hook(&pool).await;

        assert!(rule_enabled(&pool, "valid").await);
        assert!(!rule_enabled(&pool, "runtime_failure").await);
        assert!(rule_enabled(&pool, "managed_runtime_failure").await);
        assert!(!rule_enabled(&pool, "already_disabled").await);
    }

    #[tokio::test]
    async fn sqlite_hook_handles_all_wrapper_failure_modes_without_changing_sources() {
        let pool = test_pool().await;
        let valid_source =
            "package scryer.rules.user.valid\nimport rego.v1\nscore_entry[\"bonus\"] := 100";
        let runtime_failure_source = "package scryer.rules.user.runtime_failure\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)";
        let compilation_failure_source = "package scryer.rules.user.compilation_failure\nimport rego.v1\nscore_entry[\"bonus\"] :=";
        let invalid_shape_source = "package scryer.rules.user.invalid_shape\nimport rego.v1\nscore_entry[\"bonus\"] := \"not-an-integer\"";
        let package_mismatch_source =
            "package scryer.rules.user.other_rule\nimport rego.v1\nscore_entry[\"bonus\"] := 100";

        for (id, source) in [
            ("valid", valid_source),
            ("runtime_failure", runtime_failure_source),
            ("compilation_failure", compilation_failure_source),
            ("invalid_shape", invalid_shape_source),
            ("package_mismatch", package_mismatch_source),
        ] {
            insert_rule(&pool, id, source, true, false).await;
        }
        insert_rule(
            &pool,
            "managed_runtime_failure",
            runtime_failure_source
                .replace("runtime_failure", "managed_runtime_failure")
                .as_str(),
            true,
            true,
        )
        .await;
        insert_rule(
            &pool,
            "already_disabled",
            runtime_failure_source
                .replace("runtime_failure", "already_disabled")
                .as_str(),
            false,
            false,
        )
        .await;
        for index in 0..32 {
            let id = format!("bulk_broken_{index}");
            let source = format!(
                "package scryer.rules.user.{id}\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)"
            );
            insert_rule(&pool, &id, &source, true, false).await;
        }

        let valid_before = rule_state(&pool, "valid").await;
        let managed_before = rule_state(&pool, "managed_runtime_failure").await;
        let already_disabled_before = rule_state(&pool, "already_disabled").await;

        run_sqlite_hook(&pool).await;

        assert_eq!(rule_state(&pool, "valid").await, valid_before);
        assert_eq!(
            rule_state(&pool, "managed_runtime_failure").await,
            managed_before
        );
        assert_eq!(
            rule_state(&pool, "already_disabled").await,
            already_disabled_before
        );
        for (id, source) in [
            ("runtime_failure", runtime_failure_source),
            ("compilation_failure", compilation_failure_source),
            ("invalid_shape", invalid_shape_source),
            ("package_mismatch", package_mismatch_source),
        ] {
            let state = rule_state(&pool, id).await;
            assert!(!state.enabled, "{id} should be disabled");
            assert_eq!(state.rego_source, source, "{id} source must be preserved");
            assert_ne!(state.updated_at, "2000-01-01T00:00:00Z");
        }
        let bulk_disabled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rule_sets WHERE id LIKE 'bulk_broken_%' AND enabled = 0",
        )
        .fetch_one(&pool)
        .await
        .expect("bulk disabled count should load");
        assert_eq!(bulk_disabled, 32);

        let state_after_first_run = rule_state(&pool, "runtime_failure").await;
        run_sqlite_hook(&pool).await;
        assert_eq!(
            rule_state(&pool, "runtime_failure").await,
            state_after_first_run,
            "a completed migration hook must not drift rule state on a rerun"
        );
    }

    #[tokio::test]
    async fn sqlite_hook_rolls_back_all_disables_when_one_write_fails() {
        let pool = test_pool().await;
        let first_source = "package scryer.rules.user.a_first\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)";
        let blocked_source = "package scryer.rules.user.z_blocked\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)";
        insert_rule(&pool, "a_first", first_source, true, false).await;
        insert_rule(&pool, "z_blocked", blocked_source, true, false).await;
        sqlx::raw_sql(
            "
            CREATE TRIGGER reject_z_blocked_disable
            BEFORE UPDATE OF enabled ON rule_sets
            WHEN NEW.id = 'z_blocked'
            BEGIN
                SELECT RAISE(ABORT, 'forced rule-set update failure');
            END;
            ",
        )
        .execute(&pool)
        .await
        .expect("failure trigger should be created");

        let mut tx = pool.begin().await.expect("transaction should begin");
        let error = disable_invalid_user_rule_runtime_wrappers_sqlite(&mut tx)
            .await
            .expect_err("the forced update failure should abort the hook");
        assert!(error.to_string().contains("forced rule-set update failure"));
        drop(tx);

        assert!(rule_enabled(&pool, "a_first").await);
        assert!(rule_enabled(&pool, "z_blocked").await);
    }

    #[tokio::test]
    async fn sqlite_upgrade_runs_the_registered_runtime_wrapper_hook() {
        crate::spellfix::register_spellfix_auto_extension()
            .expect("spellfix extension should register before the migration fixture");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite should open");
        crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(188), true)
            .await
            .expect("pre-0189 migration fixture should apply");
        sqlx::query(
            "INSERT INTO rule_sets (id, name, rego_source, enabled, is_managed)
             VALUES (?1, 'Broken legacy rule', ?2, 1, 0)",
        )
        .bind("upgrade_runtime_failure")
        .bind(
            "package scryer.rules.user.upgrade_runtime_failure\nimport rego.v1\nscore_entry[\"bonus\"] := lower(input.release.year)",
        )
        .execute(&pool)
        .await
        .expect("legacy rule should insert");

        crate::migrations::run_migrations(&pool, crate::MigrationMode::Apply)
            .await
            .expect("0189 upgrade should apply");

        assert!(!rule_enabled(&pool, "upgrade_runtime_failure").await);
        let applied: i64 =
            sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = 189")
                .fetch_one(&pool)
                .await
                .expect("0189 migration ledger entry should load");
        assert_eq!(applied, 1);
    }

    #[test]
    fn runtime_wrapper_migration_is_an_upgrade_only_cross_engine_hook() {
        let bundle = crate::migrations::load_source_migration_catalog()
            .expect("source migration catalog should compile");
        let migration = bundle
            .catalog
            .find_migration(189)
            .expect("migration 0189 should be registered");

        assert_eq!(migration.steps.len(), 1);
        let CompiledMigrationStep::Rust {
            hook_id,
            engine,
            scope,
        } = &migration.steps[0]
        else {
            panic!("migration 0189 should contain a Rust hook");
        };
        assert_eq!(hook_id, "disable_invalid_user_rule_runtime_wrappers");
        assert_eq!(*engine, EngineScope::All);
        assert_eq!(*scope, StepScope::UpgradeOnly);
        assert!(scope.applies_to(MigrationInstallKind::Upgrade));
        assert!(!scope.applies_to(MigrationInstallKind::FreshInstall));
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite should open");
        sqlx::raw_sql(
            "
            CREATE TABLE rule_sets (
                id TEXT PRIMARY KEY,
                rego_source TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                is_managed INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
        )
        .execute(&pool)
        .await
        .expect("rule_sets table should be created");
        pool
    }

    async fn run_sqlite_hook(pool: &sqlx::SqlitePool) {
        let mut tx = pool.begin().await.expect("transaction should begin");
        disable_invalid_user_rule_runtime_wrappers_sqlite(&mut tx)
            .await
            .expect("migration hook should complete");
        tx.commit().await.expect("transaction should commit");
    }

    async fn insert_rule(
        pool: &sqlx::SqlitePool,
        id: &str,
        source: &str,
        enabled: bool,
        is_managed: bool,
    ) {
        sqlx::query(
            "INSERT INTO rule_sets (id, rego_source, enabled, is_managed, updated_at)
             VALUES (?1, ?2, ?3, ?4, '2000-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(source)
        .bind(enabled)
        .bind(is_managed)
        .execute(pool)
        .await
        .expect("rule should be inserted");
    }

    async fn rule_enabled(pool: &sqlx::SqlitePool, id: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT enabled FROM rule_sets WHERE id = ?1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("rule enabled state should load")
            != 0
    }

    async fn rule_state(pool: &sqlx::SqlitePool, id: &str) -> RuleState {
        let (rego_source, enabled, updated_at): (String, i64, String) =
            sqlx::query_as("SELECT rego_source, enabled, updated_at FROM rule_sets WHERE id = ?1")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("rule state should load");
        RuleState {
            rego_source,
            enabled: enabled != 0,
            updated_at,
        }
    }
}
