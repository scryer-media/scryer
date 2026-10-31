use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use sqlx::ConnectOptions;
use sqlx::postgres::PgConnectOptions;
use tracing::log::LevelFilter;

use crate::encryption::{EncryptionKey, load_existing_encryption_key_without_generation};
use crate::migrations::{MigrationHookContext, MigrationProgress};
use crate::types::MigrationMode;

const DEFAULT_POSTGRES_MAX_CONNECTIONS: u32 = 16;
const MAX_POSTGRES_CONNECTIONS_CAP: u32 = 128;
const POSTGRES_SLOW_STATEMENT_MS: u64 = 2000;

fn postgres_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_POSTGRES_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_POSTGRES_MAX_CONNECTIONS)
        .clamp(1, MAX_POSTGRES_CONNECTIONS_CAP)
}

#[derive(Clone)]
pub struct PostgresServices {
    pool: sqlx::PgPool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl PostgresServices {
    pub async fn new_with_mode(
        database_url: impl AsRef<str>,
        migration_mode: MigrationMode,
    ) -> Result<Self, AppError> {
        Self::new_with_mode_and_data_dir(database_url, migration_mode, None).await
    }

    pub async fn new_with_mode_and_data_dir(
        database_url: impl AsRef<str>,
        migration_mode: MigrationMode,
        data_dir: Option<PathBuf>,
    ) -> Result<Self, AppError> {
        Self::new_with_migration_progress(
            database_url,
            migration_mode,
            data_dir,
            MigrationProgress::default(),
        )
        .await
    }

    /// Connects like [`Self::new_with_mode_and_data_dir`], counting the
    /// migrations it applies into `migration_progress`.
    pub async fn new_with_migration_progress(
        database_url: impl AsRef<str>,
        migration_mode: MigrationMode,
        data_dir: Option<PathBuf>,
        migration_progress: MigrationProgress,
    ) -> Result<Self, AppError> {
        let mut connect_options: PgConnectOptions =
            database_url
                .as_ref()
                .parse()
                .map_err(|error: sqlx::Error| {
                    AppError::Repository(format!("invalid PostgreSQL database URL: {error}"))
                })?;
        connect_options = connect_options.log_slow_statements(
            LevelFilter::Debug,
            std::time::Duration::from_millis(POSTGRES_SLOW_STATEMENT_MS),
        );

        let pool = super::startup::connect_pool_with_startup_retry(
            connect_options,
            postgres_max_connections_from_env(),
            super::startup::StartupRetryPolicy::from_env(),
        )
        .await?;

        let migration_encryption_key = load_existing_encryption_key_without_generation(data_dir)
            .map_err(AppError::Repository)?;
        super::migrations::run_migrations_with_hook_context(
            &pool,
            migration_mode,
            MigrationHookContext {
                encryption_key: migration_encryption_key,
                progress: migration_progress,
            },
        )
        .await?;
        if matches!(migration_mode, MigrationMode::Apply) {
            // Same contract as SQLite: rebuild when the projection is empty or
            // was stamped with different collation data than this build
            // produces.
            crate::queries::title_search::seed_title_search_projection_if_stale_pg(&pool).await?;
        }

        Ok(Self {
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
        })
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    pub fn datastore(&self) -> crate::queries::sql_runtime::StoreDatastore {
        crate::queries::sql_runtime::StoreDatastore::Postgres {
            pool: self.pool.clone(),
        }
    }

    pub fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
        self.encryption_key.clone()
    }

    pub async fn set_encryption_key(&self, key: EncryptionKey) -> AppResult<()> {
        *self
            .encryption_key
            .write()
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))? =
            Some(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use super::*;
    use scryer_application::{
        LibraryRepository, LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
        LibraryScanUnmatchedSearchAttempt, MetadataFieldUpdate, PendingImportStatus,
        QualityProfileRepository, SettingsRepository, SystemInfoProvider, TitleImageKind,
        TitleImageRepository, TitleImageSourceResult, TitleImageVariantRecord, TitleMetadataUpdate,
        TitleRepository, UserRepository, builtin_4k_profile,
    };
    use scryer_domain::{
        Id, LibraryGrant, LibraryPermission, LibraryPermissionMask, MediaFacet, Title, User,
    };
    use sqlx::Row;
    use tokio::task::JoinSet;

    use scryer_infrastructure_configuration::settings::{
        quality_profile_store::QualityProfileStore, settings_store::SettingsStore,
    };
    use scryer_infrastructure_identity::users::store::UserStore;
    use scryer_infrastructure_library::media::{
        images::title_image_store::TitleImageStore,
        libraries::scan_unmatched_store::LibraryScanUnmatchedStore, libraries::store::LibraryStore,
        titles::store::TitleStore,
    };
    use scryer_infrastructure_sql::types::SettingDefinitionSeed;

    fn title_store(services: &PostgresServices) -> TitleStore {
        TitleStore::new(services.datastore())
    }

    fn library_store(services: &PostgresServices) -> LibraryStore {
        LibraryStore::new(services.datastore())
    }

    fn settings_store(services: &PostgresServices) -> SettingsStore {
        SettingsStore::new(services.datastore(), services.encryption_key_state())
    }

    fn user_store(services: &PostgresServices) -> UserStore {
        UserStore::new(services.datastore())
    }

    #[tokio::test]
    async fn postgres_blank_install_smoke_from_env_url() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL smoke test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = title_store(&services);
            let images = TitleImageStore::new(services.datastore());
            let settings = settings_store(&services);
            let initial_setup: String = sqlx::query_scalar("SELECT CAST(value_json AS TEXT) FROM settings_values WHERE id = 'initial-authentication-setup'")
                .fetch_one(services.pool()).await.map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&initial_setup).unwrap(), serde_json::json!({"stage": "choice"}));
            crate::postgres::migrations::run_migrations(services.pool(), MigrationMode::Apply).await?;
            assert_eq!(user_store(&services).initial_authentication_state().await?, Some(scryer_application::initial_auth::SetupState::Choice));
            let info = settings.datastore_info().await?;
            assert_eq!(info.engine, "postgres");
            let current_migration_key = info.current_migration_key.as_deref().ok_or_else(|| {
                AppError::Repository(
                    "expected PostgreSQL blank install to record a current migration key"
                        .to_string(),
                )
            })?;
            let current_migration_version = current_migration_key
                .split('_')
                .next()
                .and_then(|value| value.parse::<i64>().ok())
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "failed to parse PostgreSQL migration key {current_migration_key}"
                    ))
                })?;
            assert_eq!(
                current_migration_version,
                crate::migrations::embedded_catalog()?.max_version(),
                "PostgreSQL blank install must apply the entire current catalog",
            );
            assert_postgres_runtime_schema_columns(services.pool()).await?;
            let default_library_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM libraries WHERE is_default = TRUE")
                    .fetch_one(services.pool())
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(
                default_library_count, 3,
                "expected PostgreSQL blank install to seed canonical default libraries"
            );
            let default_root_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM library_roots
                 WHERE is_default = TRUE
                   AND library_id IN ('movie_default_library', 'series_default_library', 'anime_default_library')",
            )
            .fetch_one(services.pool())
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(
                default_root_count, 3,
                "expected PostgreSQL blank install to seed canonical default library roots"
            );

            let title = sample_title("pg-blank-install-movie");
            TitleRepository::create(&catalog, title.clone()).await?;
            let title_search_term_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM title_search_terms WHERE title_id = $1")
                    .bind(&title.id)
                    .fetch_one(services.pool())
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            assert!(
                title_search_term_count > 0,
                "PostgreSQL title create should seed canonical title_search_terms"
            );
            catalog
                .mark_title_metadata_hydration_due_now(&title.id)
                .await?;

            let due = catalog.list_titles_due_for_hydration(10, &[]).await?;
            assert_eq!(due.len(), 1, "expected one title due for hydration");
            assert_eq!(due[0].title.id, title.id);

            catalog
                .update_title_hydrated_metadata(
                    &title.id,
                    TitleMetadataUpdate {
                        name: None,
                        year: Some(2024),
                        overview: Some("postgres blank install smoke".to_string()),
                        poster_url: Some("https://image.tmdb.org/t/p/original/scryer-pg-blank-install-poster.jpg".to_string()),
                        background_url: Some("https://image.tmdb.org/t/p/original/scryer-pg-blank-install-fanart.jpg".to_string()),
                        sort_title: Some("Postgres Blank Install Smoke".to_string()),
                        slug: Some("postgres-blank-install-smoke".to_string()),
                        imdb_id: Some("tt1234567".to_string()),
                        runtime_minutes: Some(123),
                        popularity: None,
                        canonical_tags: vec![],
                        content_status: Some("released".to_string()),
                        language: MetadataFieldUpdate::Set("eng".to_string()),
                        first_aired: Some("2024-01-01".to_string()),
                        network: Some("Example Network".to_string()),
                        studio: Some("Example Studio".to_string()),
                        country: Some("US".to_string()),
                        aliases: vec!["Smoke Movie".to_string()],
                        tagged_aliases: Vec::new(),
                        metadata_language: Some("eng".to_string()),
                        metadata_fetched_at: Some(chrono::Utc::now().to_rfc3339()),
                        digital_release_date: Some("2024-01-15".to_string()),
                        min_availability: None,
                        movie_release_dates: None,
                        ratings: None,
                        credits: None,
                        extra_external_ids: Vec::new(),
                        extra_tags: Vec::new(),
                    },
                )
                .await?;
            catalog
                .clear_title_metadata_hydration_retry_state(&title.id)
                .await?;

            let poster_tasks = images
                .list_title_image_refresh_work(10, &[])
                .await?;
            assert_eq!(
                poster_tasks.len(),
                1,
                "expected hydrated poster metadata to drive image refresh work"
            );
            assert_eq!(poster_tasks[0].title_id, title.id);
            assert_eq!(
                poster_tasks[0].source_url,
                "https://image.tmdb.org/t/p/original/scryer-pg-blank-install-poster.jpg".to_string()
            );

            let variant_bytes = vec![5, 6, 7, 8];
            let variant_digest = format!("blake3:{}", blake3::hash(&variant_bytes).to_hex());
            images
                .upsert_title_image_source_result(
                    &title.id,
                    TitleImageSourceResult {
                        kind: TitleImageKind::Poster,
                        requested_source_url: "https://image.tmdb.org/t/p/original/scryer-pg-blank-install-poster.jpg".to_string(),
                        source_url: "https://image.tmdb.org/t/p/original/scryer-pg-blank-install-poster.jpg".to_string(),
                        source_etag: Some("source-etag".to_string()),
                        source_last_modified: Some("Wed, 14 May 2026 03:00:00 GMT".to_string()),
                        source_format: "jpeg".to_string(),
                        source_width: 1200,
                        source_height: 1800,
                        variants: vec![TitleImageVariantRecord {
                            variant_key: "w250".to_string(),
                            format: "avif".to_string(),
                            width: 250,
                            height: 375,
                            bytes: variant_bytes,
                            digest: variant_digest.clone(),
                        }],
                    },
                    None,
                )
                .await?;

            let original_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
                .await?;
            assert!(original_blob.is_none());

            let thumb_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "w250")
                .await?
                .ok_or_else(|| {
                    AppError::Repository(
                        "expected PostgreSQL blank install to persist title image variants"
                            .to_string(),
                    )
                })?;
            assert_eq!(thumb_blob.content_type, "image/avif");
            assert_eq!(thumb_blob.etag, variant_digest);
            assert_eq!(thumb_blob.bytes, vec![5, 6, 7, 8]);

            // A second apply is a no-op, and the resulting full catalog validates.
            let ledger_snapshot = |rows: Vec<crate::MigrationStatus>| {
                rows.into_iter()
                    .map(|row| (
                        row.migration_key,
                        row.migration_checksum_algo,
                        row.migration_checksum,
                        row.applied_at,
                        row.success,
                        row.error_message,
                        row.runtime_version,
                    ))
                    .collect::<Vec<_>>()
            };
            let before = ledger_snapshot(crate::postgres::migrations::list_applied_migrations(services.pool()).await?);
            crate::postgres::migrations::run_migrations(services.pool(), MigrationMode::Apply).await?;
            crate::postgres::migrations::run_migrations(services.pool(), MigrationMode::ValidateOnly).await?;
            let after = ledger_snapshot(crate::postgres::migrations::list_applied_migrations(services.pool()).await?);
            assert_eq!(before, after, "reapplying the current catalog must preserve its ledger");
            assert_postgres_runtime_schema_columns(services.pool()).await?;

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_0198_preserves_conflicting_custom_defaults_from_env_url() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL 0198 conflict test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let pool = sqlx::PgPool::connect(&schema_url)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            crate::postgres_history_test_support::replay_historical_postgres(&pool, 197).await?;

            sqlx::query(
                "INSERT INTO libraries
                    (id, facet, name, slug, is_default, created_at, updated_at)
                 VALUES
                    ('custom_movies', 'movie', 'Custom Movies', 'movies', false,
                     '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
                    ('custom_series_paths', 'series', 'Custom Series Paths', 'custom-series', false,
                     '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            )
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            sqlx::query(
                "INSERT INTO library_roots
                    (id, library_id, path, normalized_path, is_default, created_at, updated_at)
                 VALUES
                    ('custom_movies_root', 'custom_movies', '/custom/movies', '/custom/movies', false,
                     '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
                    ('custom_series_root', 'custom_series_paths', '/data/series', '/data/series', false,
                     '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            )
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;

            crate::postgres::migrations::run_migrations(&pool, MigrationMode::Apply).await?;
            crate::postgres::migrations::run_migrations(&pool, MigrationMode::Apply).await?;
            crate::postgres::migrations::run_migrations(&pool, MigrationMode::ValidateOnly).await?;

            let custom_movies: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM libraries
                  WHERE id = 'custom_movies' AND facet = 'movie' AND slug = 'movies'",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(custom_movies, 1);
            let canonical_movie_rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM libraries WHERE id = 'movie_default_library'",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(canonical_movie_rows, 0);
            let conflicting_canonical_roots: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM library_roots
                  WHERE id IN (
                    'canonical_root_for_movie_default_library',
                    'canonical_root_for_series_default_library'
                  )",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(conflicting_canonical_roots, 0);
            let preserved_custom_roots: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM library_roots
                  WHERE id IN ('custom_movies_root', 'custom_series_root')",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(preserved_custom_roots, 2);
            let canonical_anime_root: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM library_roots
                  WHERE id = 'canonical_root_for_anime_default_library'",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(canonical_anime_root, 1);

            pool.close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_execute_batch_insert_multi_chunk_renumbers_placeholders() -> AppResult<()> {
        use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL batch-insert test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let pool = sqlx::PgPool::connect(&schema_url).await.map_err(|error| {
                AppError::Repository(format!("failed to connect with search_path: {error}"))
            })?;

            sqlx::query(
                "CREATE TABLE batch_probe_pg (n INTEGER NOT NULL, label TEXT NOT NULL, note TEXT NOT NULL)",
            )
            .execute(&pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create scratch table: {error}"))
            })?;

            // 700 rows x 3 cols, cap 1000 => 333 rows/chunk => three chunks.
            // Each chunk statement is rendered independently, so its
            // $-placeholders restart at $1; a correct multi-chunk insert proves
            // per-statement renumbering lands every bind in the right slot.
            let row_count = 700usize;
            let rows: Vec<Vec<SqlArg>> = (0..row_count)
                .map(|index| {
                    vec![
                        SqlArg::I32(index as i32),
                        SqlArg::Text(format!("row-{index}")),
                        SqlArg::Text(format!("note-{index}")),
                    ]
                })
                .collect();

            let datastore = StoreDatastore::Postgres { pool: pool.clone() };
            let affected =
                SqlRuntime::run_in_transaction(&datastore, "pg_batch_probe_insert", move |tx| {
                    let rows = rows.clone();
                    Box::pin(async move {
                        SqlRuntime::execute_batch_insert(
                            tx,
                            "INSERT INTO batch_probe_pg (n, label, note)",
                            3,
                            rows,
                            "",
                        )
                        .await
                    })
                })
                .await?;
            assert_eq!(
                affected, row_count as u64,
                "rows_affected must sum across all three chunks"
            );

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM batch_probe_pg")
                .fetch_one(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(count, row_count as i64);

            let first = sqlx::query("SELECT label, note FROM batch_probe_pg WHERE n = $1")
                .bind(0_i32)
                .fetch_one(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(first.get::<String, _>("label"), "row-0");
            assert_eq!(first.get::<String, _>("note"), "note-0");

            let last = sqlx::query("SELECT label, note FROM batch_probe_pg WHERE n = $1")
                .bind((row_count - 1) as i32)
                .fetch_one(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(last.get::<String, _>("label"), format!("row-{}", row_count - 1));
            assert_eq!(last.get::<String, _>("note"), format!("note-{}", row_count - 1));

            pool.close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_settings_accept_raw_strings_and_read_them_back_as_json_strings()
    -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL raw-string settings test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = settings_store(&services);

            settings
                .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                    category: "tests".to_string(),
                    scope: "tests".to_string(),
                    key_name: "raw_string_setting".to_string(),
                    data_type: "string".to_string(),
                    default_value_json: "postgres-default".to_string(),
                    is_sensitive: false,
                    validation_json: None,
                }])
                .await?;

            assert_eq!(
                SettingsRepository::get_setting_json(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None
                )
                .await?,
                Some("\"postgres-default\"".to_string()),
                "default string settings should remain valid JSON strings for application readers",
            );

            SettingsRepository::upsert_setting_json(
                &settings,
                "tests",
                "raw_string_setting",
                None,
                "preferred-profile".to_string(),
                "test",
                None,
            )
            .await?;

            assert_eq!(
                SettingsRepository::get_setting_json(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None
                )
                .await?,
                Some("\"preferred-profile\"".to_string()),
                "raw string writes should round-trip as JSON strings so application readers stay engine-neutral",
            );
            assert_eq!(
                SettingsRepository::get_setting_json_explicit(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None,
                )
                .await?,
                Some("\"preferred-profile\"".to_string()),
                "explicit raw string writes should round-trip as JSON strings so application readers stay engine-neutral",
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_library_scoped_download_client_routing_round_trips_explicit_json()
    -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL library download client routing test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = settings_store(&services);

            settings
                .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                    category: "media".to_string(),
                    scope: "system".to_string(),
                    key_name: "download_client.routing".to_string(),
                    data_type: "json".to_string(),
                    default_value_json: "{}".to_string(),
                    is_sensitive: false,
                    validation_json: None,
                }])
                .await?;

            let library_id = "series_default_library";
            let value_json = serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "category": "series",
                    "recentQueuePriority": "high",
                    "olderQueuePriority": "normal",
                    "removeCompleted": true,
                    "removeFailed": false
                }
            })
            .to_string();

            SettingsRepository::upsert_setting_json(
                &settings,
                "system",
                "download_client.routing",
                Some(library_id.to_string()),
                value_json.clone(),
                "test",
                None,
            )
            .await?;

            let explicit = SettingsRepository::get_setting_json_explicit(
                &settings,
                "system",
                "download_client.routing",
                Some(library_id.to_string()),
            )
            .await?;
            // `settings_values.value_json` is `jsonb` on PostgreSQL, which
            // normalizes key order on write; compare parsed values rather
            // than the raw string (unlike SQLite's TEXT column, which does
            // preserve the exact bytes — see the sqlite-side counterpart of
            // this test).
            let explicit_value: serde_json::Value = explicit
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| {
                    AppError::Repository(format!("failed to parse stored setting JSON: {error}"))
                })?
                .ok_or_else(|| {
                    AppError::Repository("expected an explicit library-scoped setting".to_string())
                })?;
            let expected_value: serde_json::Value =
                serde_json::from_str(&value_json).expect("test fixture value is valid JSON");
            assert_eq!(explicit_value, expected_value);

            let default_lookup = SettingsRepository::get_setting_json(
                &settings,
                "system",
                "download_client.routing",
                Some("another_library".to_string()),
            )
            .await?;
            assert_eq!(default_lookup.as_deref(), Some("{}"));

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_quality_profile_catalog_upsert_round_trips_on_blank_install() -> AppResult<()>
    {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL quality-profile catalog test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let quality_profiles = QualityProfileStore::new(services.datastore());
            let profiles = vec![builtin_4k_profile()];

            quality_profiles
                .replace_quality_profiles("system", None, profiles.clone())
                .await?;

            let stored = quality_profiles
                .list_quality_profiles("system", None)
                .await?;
            assert_eq!(stored, profiles);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_blank_install_applies_parity_secondary_indexes() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL parity-index test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;

            let actual_indexes = sqlx::query_scalar::<_, String>(
                "SELECT indexname
                   FROM pg_indexes
                  WHERE schemaname = current_schema()",
            )
            .fetch_all(services.pool())
            .await
            .map(BTreeSet::from_iter)
            .map_err(|error| AppError::Repository(error.to_string()))?;

            let expected_indexes = postgres_parity_index_names_from_source();
            let missing: Vec<String> = expected_indexes
                .difference(&actual_indexes)
                .cloned()
                .collect();
            assert!(
                missing.is_empty(),
                "expected PostgreSQL blank install to apply parity secondary indexes; missing {missing:?}"
            );
            assert!(
                actual_indexes.contains("idx_library_scan_unmatched_items_facet_title_status_updated"),
                "expected PostgreSQL blank install to restore the title-aware unmatched-items index"
            );
            assert!(
                !actual_indexes.contains("idx_title_search_terms_title_kind_normalized"),
                "0278 retires the folded-term index so native spellings can coexist"
            );

            // Protect the replacement index's behavior, including its expression
            // key: folded spellings and language profiles coexist, but NULL and
            // the empty language tag identify the same native spelling.
            let title = sample_title("pg-parity-index-title");
            TitleRepository::create(&title_store(&services), title.clone()).await?;
            let insert_term = "INSERT INTO title_search_terms
                (title_id, facet, term_kind, raw_term, normalized_term, weight, literal_term, language_tag)
                VALUES ($1, 'movie', 'alias', $2, 'shared_folded', 1, $2, $3)";
            for (literal, language) in [
                ("Native spelling A", None),
                ("Native spelling B", None),
                ("Native spelling A", Some("fra")),
            ] {
                sqlx::query(insert_term)
                    .bind(&title.id)
                    .bind(literal)
                    .bind(language)
                    .execute(services.pool())
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            }
            let duplicate = sqlx::query(insert_term)
                .bind(&title.id)
                .bind("Native spelling A")
                .bind("")
                .execute(services.pool())
                .await
                .expect_err("NULL and empty language tags must share the unique index key");
            assert!(
                duplicate.as_database_error().is_some_and(|error| error.is_unique_violation()),
                "expected the language-aware unique index to refuse a duplicate: {duplicate}"
            );
            let term_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM title_search_terms WHERE title_id = $1 AND normalized_term = 'shared_folded'",
            )
            .bind(&title.id)
            .fetch_one(services.pool())
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(term_count, 3, "native spelling and language columns must remain independent keys");
            let orphan = sqlx::query(insert_term)
                .bind("missing-parity-index-title")
                .bind("Orphan")
                .bind(Option::<&str>::None)
                .execute(services.pool())
                .await
                .expect_err("search terms must retain their title foreign key");
            assert!(
                orphan.as_database_error().is_some_and(|error| error.is_foreign_key_violation()),
                "expected the title foreign key to refuse an orphan term: {orphan}"
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_library_scan_unmatched_items_round_trip_title_binding() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL unmatched-item title-binding test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let store = LibraryScanUnmatchedStore::new(services.datastore());
            let now = chrono::Utc::now().to_rfc3339();
            let item = LibraryScanUnmatchedItem {
                id: Id::new().0,
                library_id: "movie_default_library".to_string(),
                facet: MediaFacet::Movie,
                status: PendingImportStatus::Pending,
                title_id: Some("pg-title-binding".to_string()),
                scan_session_id: Id::new().0,
                scan_root: "/tmp/scryer-test/movies".to_string(),
                item_path: "/tmp/scryer-test/movies/Test Movie (2024)".to_string(),
                display_name: "Test Movie (2024)".to_string(),
                query: "Test Movie".to_string(),
                year_hint: Some(2024),
                reason_code: "title_binding_check".to_string(),
                error_message: Some("no exact metadata match".to_string()),
                search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
                    query: "Test Movie".to_string(),
                    result_count: 1,
                    top_results: vec!["Test Movie (2024)".to_string()],
                }],
                size_bytes: Some(4_294_967_296),
                created_at: now.clone(),
                updated_at: now,
            };

            let id = LibraryScanUnmatchedItemRepository::upsert_library_scan_unmatched_item(
                &store, &item,
            )
            .await?;
            assert_eq!(id, item.id);

            let stored = LibraryScanUnmatchedItemRepository::get_library_scan_unmatched_item(
                &store, &item.id,
            )
            .await?
            .expect("stored unmatched item");
            assert_eq!(stored.title_id, item.title_id);
            assert_eq!(stored.search_attempts, item.search_attempts);
            // Exercises the 0159 bigint column on the PostgreSQL dialect.
            assert_eq!(stored.size_bytes, item.size_bytes);

            let listed = LibraryScanUnmatchedItemRepository::list_library_scan_unmatched_items(
                &store,
                Some(MediaFacet::Movie),
                Some(item.scan_root.as_str()),
                Some(PendingImportStatus::Pending),
                10,
                0,
            )
            .await?;
            assert_eq!(listed.len(), 1, "expected one unmatched scan item");
            assert_eq!(listed[0].title_id, item.title_id);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_set_grants_for_user_is_idempotent_under_concurrency() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL concurrency test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let users = user_store(&services);
            let libraries = library_store(&services);
            let movie_library = LibraryRepository::default_for_facet(&libraries, MediaFacet::Movie)
                .await?
                .expect("blank install seeds the default movie library");
            assert_eq!(movie_library.id, "movie_default_library");
            assert_eq!(movie_library.facet, MediaFacet::Movie);
            assert_eq!(movie_library.slug, "movies");
            assert!(movie_library.is_default);
            assert_eq!(movie_library.roots.len(), 1);
            assert!(movie_library.roots[0].is_default);
            assert_eq!(movie_library.roots[0].path, "/data/movies");
            let user = UserRepository::create(&users, User::new_admin("admin-concurrency")).await?;
            let user_id = user.id.clone();
            let permissions = LibraryPermissionMask::from_permissions([
                LibraryPermission::View,
                LibraryPermission::ManageTitles,
                LibraryPermission::ResolveImports,
                LibraryPermission::ManageLibrary,
                LibraryPermission::Request,
                LibraryPermission::AutoApproveRequests,
                LibraryPermission::ManageSubtitles,
            ]);

            let mut tasks = JoinSet::new();
            for _ in 0..8 {
                let libraries = libraries.clone();
                let user_id = user_id.clone();
                tasks.spawn(async move {
                    LibraryRepository::set_grants_for_user(
                        &libraries,
                        &user_id,
                        vec![LibraryGrant {
                            user_id: user_id.clone(),
                            library_id: "movie_default_library".to_string(),
                            permissions,
                        }],
                    )
                    .await
                });
            }

            while let Some(result) = tasks.join_next().await {
                result
                    .map_err(|error| AppError::Repository(error.to_string()))?
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            }

            let grants = LibraryRepository::permission_masks_for_user(&libraries, &user.id).await?;
            assert_eq!(grants.len(), 1, "expected exactly one library grant row");
            assert_eq!(grants[0].library_id, "movie_default_library");
            assert_eq!(grants[0].permissions, permissions);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    fn postgres_url_with_search_path(raw_url: &str, schema: &str) -> AppResult<String> {
        let mut url = url::Url::parse(raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        Ok(url.to_string())
    }

    fn next_test_schema_name() -> String {
        format!(
            "scryer_test_{}_{}",
            std::process::id(),
            Id::new().0.replace('-', "_")
        )
    }

    fn postgres_parity_index_names_from_source() -> BTreeSet<String> {
        let mut indexes: BTreeSet<String> = std::fs::read_to_string(postgres_0254_baseline_path())
            .expect("read PostgreSQL 0254 baseline")
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("CREATE UNIQUE INDEX ")
                    .or_else(|| trimmed.strip_prefix("CREATE INDEX "))
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_string)
            })
            .collect();
        // 0278 replaces folded-term uniqueness with native spelling/language
        // uniqueness. Every other baseline parity index is still required.
        assert!(indexes.remove("idx_title_search_terms_title_kind_normalized"));
        assert!(indexes.insert("idx_title_search_terms_title_kind_language".to_string()));
        indexes
    }

    fn postgres_0254_baseline_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scryer/src/db/postgres/baselines/0254_baseline.sql")
    }

    async fn assert_postgres_runtime_schema_columns(postgres_pool: &sqlx::PgPool) -> AppResult<()> {
        // Each engine independently applies its current baseline, engine-specific
        // SQL steps, and hooks. Compare the complete logical schema, rather than
        // rejecting every column introduced after the old 0254 baseline.
        let sqlite = crate::sqlite_services::SqliteServices::new_with_mode(
            "sqlite://:memory:",
            MigrationMode::Apply,
        )
        .await?;
        let mut sqlite_schema = sqlite_table_columns(sqlite.pool()).await?;
        let mut postgres_schema = postgres_table_columns(postgres_pool).await?;
        let actual_tables: BTreeSet<String> = postgres_schema.keys().cloned().collect();

        // The PostgreSQL ledger intentionally retains two runtime diagnostic
        // columns. Assert both exact ledger shapes before comparing application
        // tables; an arbitrary schema difference must never be filtered away.
        let sqlite_ledger = sqlite_schema
            .remove("_sqlx_migrations")
            .expect("SQLite migration ledger");
        let postgres_ledger = postgres_schema
            .remove("_sqlx_migrations")
            .expect("PostgreSQL migration ledger");
        let common_ledger: BTreeSet<String> = [
            "version",
            "description",
            "installed_on",
            "success",
            "checksum",
            "execution_time",
            "checksum_algo",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(sqlite_ledger, common_ledger);
        let mut expected_postgres_ledger = common_ledger;
        expected_postgres_ledger
            .extend(["runtime_version".to_string(), "error_message".to_string()]);
        assert_eq!(postgres_ledger, expected_postgres_ledger);

        let table_names: BTreeSet<&String> =
            sqlite_schema.keys().chain(postgres_schema.keys()).collect();
        let mismatches: Vec<String> = table_names
            .into_iter()
            .filter(|table| sqlite_schema.get(*table) != postgres_schema.get(*table))
            .map(|table| {
                format!(
                    "{table}: sqlite={:?} postgres={:?}",
                    sqlite_schema.get(table),
                    postgres_schema.get(table)
                )
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "current SQLite/PostgreSQL logical schema parity mismatch: {mismatches:#?}"
        );

        for removed_table in [
            "subtitle_providers",
            "import_artifacts",
            "job_runs",
            "quality_profiles_json",
            "mediarr_schema_migrations",
            "discovery_raw_pages",
            "event_outboxes",
        ] {
            assert!(
                !actual_tables.contains(removed_table),
                "PostgreSQL blank install still exposes removed table {removed_table}"
            );
        }
        sqlite.pool().close().await;
        Ok(())
    }

    async fn sqlite_table_columns(
        pool: &sqlx::SqlitePool,
    ) -> AppResult<BTreeMap<String, BTreeSet<String>>> {
        let rows = sqlx::query(
            "SELECT m.name AS table_name, p.name AS column_name
               FROM sqlite_master m
               JOIN pragma_table_info(m.name) p
              WHERE m.type = 'table'
                AND m.name NOT LIKE 'sqlite_%'",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let mut schema = BTreeMap::new();
        for row in rows {
            let table_name: String = row
                .try_get("table_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let column_name: String = row
                .try_get("column_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            schema
                .entry(table_name)
                .or_insert_with(BTreeSet::new)
                .insert(column_name);
        }
        Ok(schema)
    }

    async fn postgres_table_columns(
        pool: &sqlx::PgPool,
    ) -> AppResult<BTreeMap<String, BTreeSet<String>>> {
        let rows = sqlx::query(
            "SELECT table_name, column_name
               FROM information_schema.columns
              WHERE table_schema = current_schema()",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let mut schema = BTreeMap::new();
        for row in rows {
            let table_name: String = row
                .try_get("table_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let column_name: String = row
                .try_get("column_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            schema
                .entry(table_name)
                .or_insert_with(BTreeSet::new)
                .insert(column_name);
        }
        Ok(schema)
    }

    fn sample_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            library_id: "movie_default_library".to_string(),
            name: "Postgres Blank Install Smoke".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: Vec::new(),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: Some("postgres-blank-install-smoke".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            movie_release_dates: None,
            folder_path: None,
        }
    }
}
