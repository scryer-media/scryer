use chrono::Utc;
use scryer_application::SETTINGS_SCOPE_SYSTEM;
use scryer_domain::Id;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

pub const INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY: &str =
    "indexer.request_accounting_v2_started_at";

/// Establishes the observation epoch for the corrected indexer request tally.
///
/// Provider-reported quota readings are intentionally left alone. Only the
/// locally observed request counter is cleared, so provider limits and their
/// existing enforcement remain authoritative across the correction.
pub async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    SqlRuntime::run_in_transaction(datastore, "indexer_request_accounting_v2", |tx| {
        Box::pin(async move {
            let existing_epoch = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT settings_values.id, CAST(settings_values.value_json AS TEXT) AS value_json
                   FROM settings_values
                   JOIN settings_definitions
                     ON settings_definitions.id = settings_values.setting_definition_id
                  WHERE settings_definitions.category = {}
                    AND settings_definitions.scope = {}
                    AND settings_definitions.key_name = {}
                    AND settings_values.scope = {}
                    AND settings_values.scope_id IS NULL",
                &[
                    SqlArg::Text("service".to_string()),
                    SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                    SqlArg::Text(INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string()),
                    SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                ],
            )
            .await?;
            let existing_epoch_id = existing_epoch
                .as_ref()
                .map(|row| row.text("id"))
                .transpose()?;
            let epoch_is_set = existing_epoch
                .as_ref()
                .map(|row| row.text("value_json"))
                .transpose()?
                .and_then(|value| serde_json::from_str::<Option<String>>(&value).ok())
                .flatten()
                .is_some();
            if epoch_is_set {
                return Ok(());
            }

            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE indexer_api_quotas SET queries_today = 0",
                &[],
            )
            .await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "DELETE FROM indexer_api_quotas WHERE indexer_id IN ({}, {})",
                &[
                    SqlArg::Text(scryer_application::CONNECTION_TEST_INDEXER_ID.to_string()),
                    SqlArg::Text(scryer_application::PREVIEW_MANAGED_SYNC_INDEXER_ID.to_string()),
                ],
            )
            .await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE indexers
                    SET proxy_config_id = NULL
                  WHERE proxy_config_id IN (
                      SELECT id
                        FROM proxy_configs
                       WHERE LOWER(TRIM(provider_type)) IN ('byparr', 'trawl')
                  )
                    AND (
                        LOWER(TRIM(provider_type)) = 'prowlarr'
                        OR managed_parent_config_id IN (
                            SELECT id
                              FROM indexers
                             WHERE LOWER(TRIM(provider_type)) = 'prowlarr'
                        )
                    )",
                &[],
            )
            .await?;

            let definition_id = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT id
                   FROM settings_definitions
                  WHERE category = {} AND scope = {} AND key_name = {}",
                &[
                    SqlArg::Text("service".to_string()),
                    SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                    SqlArg::Text(INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string()),
                ],
            )
            .await?
            .ok_or_else(|| {
                scryer_application::AppError::Repository(
                    "missing indexer request accounting epoch setting definition".to_string(),
                )
            })?
            .text("id")?;
            let now = Utc::now();
            let epoch_value = SqlArg::Json(serde_json::Value::String(now.to_rfc3339()));
            if let Some(existing_epoch_id) = existing_epoch_id {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE settings_values
                        SET value_json = {}, source = {}, updated_at = {}
                      WHERE id = {}",
                    &[
                        epoch_value,
                        SqlArg::Text("system".to_string()),
                        SqlArg::Timestamp(now),
                        SqlArg::Text(existing_epoch_id),
                    ],
                )
                .await?;
            } else {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO settings_values
                    (id, setting_definition_id, scope, scope_id, value_json, source,
                     updated_by_user_id, created_at, updated_at)
                 VALUES ({}, {}, {}, NULL, {}, {}, NULL, {}, {})",
                    &[
                        SqlArg::Text(Id::new().0),
                        SqlArg::Text(definition_id),
                        SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                        epoch_value,
                        SqlArg::Text("system".to_string()),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                    ],
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
    .map_err(|error| format!("failed to establish indexer request accounting epoch: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
    use scryer_infrastructure_sql::runtime::SqlRow;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::settings_bootstrap::seed_service_setting_definitions;

    async fn quota_row(datastore: &StoreDatastore, id: &str) -> SqlRow {
        SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT * FROM indexer_api_quotas WHERE indexer_id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await
        .expect("read quota")
        .expect("quota row")
    }

    async fn assert_epoch_once(datastore: &StoreDatastore) {
        migrate(datastore).await.expect("first migration");
        let quota = quota_row(datastore, "kept").await;
        assert_eq!(quota.i64("queries_today").expect("queries"), 0);
        assert_eq!(quota.opt_i64("api_current").expect("api current"), Some(7));
        assert_eq!(quota.opt_i64("api_max").expect("api max"), Some(70));
        assert_eq!(
            quota.opt_i64("grab_current").expect("grab current"),
            Some(3)
        );
        assert_eq!(quota.opt_i64("grab_max").expect("grab max"), Some(30));
        assert!(
            SqlRuntime::fetch_optional(
                datastore.read_exec(),
                "SELECT indexer_id FROM indexer_api_quotas WHERE indexer_id = {}",
                &[SqlArg::Text("test-connection".to_string())],
            )
            .await
            .expect("read synthetic quota")
            .is_none()
        );
        assert!(
            SqlRuntime::fetch_optional(
                datastore.read_exec(),
                "SELECT indexer_id FROM indexer_api_quotas WHERE indexer_id = {}",
                &[SqlArg::Text("preview-managed-sync".to_string())],
            )
            .await
            .expect("read synthetic preview quota")
            .is_none()
        );

        for (id, expected) in [
            ("prowlarr-byparr", None),
            ("managed-trawl", None),
            ("managed-nonprowlarr", Some("byparr")),
            ("ordinary-byparr", Some("byparr")),
            ("prowlarr-other", Some("other")),
        ] {
            let actual = SqlRuntime::fetch_optional(
                datastore.read_exec(),
                "SELECT proxy_config_id FROM indexers WHERE id = {}",
                &[SqlArg::Text(id.to_string())],
            )
            .await
            .expect("read indexer")
            .expect("indexer")
            .opt_text("proxy_config_id")
            .expect("proxy id");
            assert_eq!(actual.as_deref(), expected);
        }

        let epoch = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT CAST(settings_values.value_json AS TEXT) AS value_json
               FROM settings_values
               JOIN settings_definitions
                 ON settings_definitions.id = settings_values.setting_definition_id
              WHERE settings_definitions.key_name = {}",
            &[SqlArg::Text(
                INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string(),
            )],
        )
        .await
        .expect("read epoch")
        .expect("epoch")
        .text("value_json")
        .expect("epoch json");

        SqlRuntime::execute_write(
            datastore,
            "restore_observed_quota_for_idempotence_test",
            "UPDATE indexer_api_quotas SET queries_today = 5 WHERE indexer_id = {}",
            vec![SqlArg::Text("kept".to_string())],
        )
        .await
        .expect("restore observed count");
        migrate(datastore).await.expect("idempotent migration");
        assert_eq!(
            quota_row(datastore, "kept")
                .await
                .i64("queries_today")
                .expect("queries"),
            5
        );
        let rerun_epoch = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT CAST(settings_values.value_json AS TEXT) AS value_json
               FROM settings_values
               JOIN settings_definitions
                 ON settings_definitions.id = settings_values.setting_definition_id
              WHERE settings_definitions.key_name = {}",
            &[SqlArg::Text(
                INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string(),
            )],
        )
        .await
        .expect("read rerun epoch")
        .expect("epoch")
        .text("value_json")
        .expect("epoch json");
        assert_eq!(rerun_epoch, epoch);
    }

    async fn seed_fixture(datastore: &StoreDatastore) {
        let epoch_definition_id = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT id FROM settings_definitions
              WHERE category = {} AND scope = {} AND key_name = {}",
            &[
                SqlArg::Text("service".to_string()),
                SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                SqlArg::Text(INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string()),
            ],
        )
        .await
        .expect("read epoch definition")
        .expect("epoch definition")
        .text("id")
        .expect("epoch definition id");
        SqlRuntime::execute_write(
            datastore,
            "seed_null_indexer_request_accounting_epoch",
            "INSERT INTO settings_values
                (id, setting_definition_id, scope, scope_id, value_json, source,
                 updated_by_user_id, created_at, updated_at)
             VALUES ({}, {}, {}, NULL, {}, {}, NULL, {}, {})",
            vec![
                SqlArg::Text("null-epoch".to_string()),
                SqlArg::Text(epoch_definition_id),
                SqlArg::Text(SETTINGS_SCOPE_SYSTEM.to_string()),
                SqlArg::Json(serde_json::Value::Null),
                SqlArg::Text("system".to_string()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .expect("seed null epoch value");
        for (id, provider) in [("byparr", "Byparr"), ("trawl", "Trawl"), ("other", "Other")] {
            SqlRuntime::execute_write(
                datastore,
                "seed_indexer_proxy_config",
                "INSERT INTO proxy_configs
                    (id, name, provider_type, protocol, base_url, created_at, updated_at)
                 VALUES ({}, {}, {}, {}, {}, {}, {})",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(provider.to_string()),
                    SqlArg::Text("request_solution_v1".to_string()),
                    SqlArg::Text("http://proxy.test".to_string()),
                    SqlArg::Timestamp(Utc::now()),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await
            .expect("seed proxy");
        }
        for (id, provider, parent, proxy) in [
            ("prowlarr-byparr", "Prowlarr", None, Some("byparr")),
            ("parent", "Prowlarr", None, None),
            ("managed-trawl", "newznab", Some("parent"), Some("trawl")),
            ("nonprowlarr-parent", "newznab", None, None),
            (
                "managed-nonprowlarr",
                "newznab",
                Some("nonprowlarr-parent"),
                Some("byparr"),
            ),
            ("ordinary-byparr", "newznab", None, Some("byparr")),
            ("prowlarr-other", "prowlarr", None, Some("other")),
        ] {
            SqlRuntime::execute_write(
                datastore,
                "seed_indexer",
                "INSERT INTO indexers
                    (id, name, provider_type, base_url, managed_parent_config_id,
                     proxy_config_id, created_at, updated_at)
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(provider.to_string()),
                    SqlArg::Text("http://indexer.test".to_string()),
                    SqlArg::OptText(parent.map(str::to_string)),
                    SqlArg::OptText(proxy.map(str::to_string)),
                    SqlArg::Timestamp(Utc::now()),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await
            .expect("seed indexer");
        }
        for (id, queries) in [
            ("kept", 12_i64),
            ("test-connection", 4_i64),
            ("preview-managed-sync", 3_i64),
        ] {
            SqlRuntime::execute_write(
                datastore,
                "seed_indexer_quota",
                "INSERT INTO indexer_api_quotas
                    (indexer_id, api_current, api_max, grab_current, grab_max, queries_today,
                     last_reset_at, updated_at)
                 VALUES ({}, 7, 70, 3, 30, {}, {}, {})",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::I64(queries),
                    SqlArg::Timestamp(Utc::now()),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await
            .expect("seed quota");
        }
    }

    #[tokio::test]
    async fn sqlite_resets_only_observed_counts_once_and_preserves_provider_quota() {
        let temp = tempfile::tempdir().expect("tempdir");
        let services = SqliteServices::new_with_mode(
            temp.path().join("scryer.db").to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let settings = Arc::new(SettingsStore::new(
            services.datastore(),
            services.encryption_key_state(),
        ));
        seed_service_setting_definitions(settings)
            .await
            .expect("seed settings");
        let datastore = services.datastore();
        seed_fixture(&datastore).await;
        assert_epoch_once(&datastore).await;
    }

    #[tokio::test]
    async fn postgres_resets_only_observed_counts_once_and_preserves_provider_quota() {
        let Some(database_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL indexer request accounting migration test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return;
        };
        // Temporary tables keep this fixture isolated in the supplied test
        // database. One pooled connection keeps the tables visible to both the
        // migration transaction and the assertions below.
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("postgres should connect");
        for statement in [
            "CREATE TEMP TABLE settings_definitions (
                id text PRIMARY KEY, category text NOT NULL, scope text NOT NULL,
                key_name text NOT NULL, data_type text NOT NULL, default_value_json text,
                is_sensitive boolean NOT NULL DEFAULT false, validation_json text,
                created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
                UNIQUE(category, scope, key_name)) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE settings_values (
                id text PRIMARY KEY, setting_definition_id text NOT NULL, scope text NOT NULL,
                scope_id text, value_json jsonb NOT NULL, source text NOT NULL,
                updated_by_user_id text, created_at timestamptz NOT NULL,
                updated_at timestamptz NOT NULL) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE indexer_api_quotas (
                indexer_id text PRIMARY KEY, api_current bigint, api_max bigint,
                grab_current bigint, grab_max bigint, queries_today bigint NOT NULL DEFAULT 0,
                last_query_at timestamptz, last_reset_at timestamptz NOT NULL,
                updated_at timestamptz NOT NULL) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE proxy_configs (
                id text PRIMARY KEY, name text NOT NULL, provider_type text NOT NULL,
                protocol text NOT NULL, base_url text NOT NULL, created_at timestamptz NOT NULL,
                updated_at timestamptz NOT NULL) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE indexers (
                id text PRIMARY KEY, name text NOT NULL, provider_type text NOT NULL,
                base_url text NOT NULL, managed_parent_config_id text,
                proxy_config_id text, created_at timestamptz NOT NULL,
                updated_at timestamptz NOT NULL) ON COMMIT PRESERVE ROWS",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create PostgreSQL fixture table");
        }
        sqlx::query(
            "INSERT INTO settings_definitions
                (id, category, scope, key_name, data_type, default_value_json,
                 is_sensitive, created_at, updated_at)
             VALUES ('epoch-definition', 'service', 'system',
                     'indexer.request_accounting_v2_started_at', 'string', 'null', false,
                     CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .expect("seed epoch definition");
        let datastore = StoreDatastore::Postgres { pool: pool.clone() };
        seed_fixture(&datastore).await;
        assert_epoch_once(&datastore).await;
        pool.close().await;
    }
}
