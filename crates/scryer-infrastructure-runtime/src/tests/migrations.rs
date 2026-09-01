use super::*;

#[tokio::test]
async fn migration_validate_mode_rejects_pending_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    assert!(
        result.is_err(),
        "validate mode should reject unapplied migrations"
    );
    let err = match result {
        Ok(_) => panic!("validate mode should reject unapplied migrations"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("pending migration"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_validate_mode_does_not_mutate_legacy_sqlx_ledger() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_legacy_ledger_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    sqlx::query("ALTER TABLE _sqlx_migrations RENAME TO _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("legacy ledger rename should succeed");
    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration ledger should be created");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         SELECT version, description, installed_on, success, checksum, execution_time
           FROM _sqlx_migrations_current
          WHERE version <= 102",
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration rows should be copied");
    sqlx::query("DROP TABLE _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("temporary migration ledger should be dropped");

    drop(services);

    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    let err = match result {
        Ok(_) => panic!("validate mode should reject missing migration 0103"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("0103_custom_migrator_runtime_cutover"),
        "validate mode should report the pending custom migration, got {err:?}"
    );

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    let applied_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration row count should load");
    assert_eq!(applied_rows, 102);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_rejects_unknown_or_newer_schema_history() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_compat_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let too_new_key = "999999_too_new";
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(Vec::<u8>::new())
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("tamper first migration checksum");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
        (version, description, installed_on, success, checksum, execution_time)
        VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(999999i64)
    .bind(too_new_key)
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await
    .expect("insert new migration");

    let result = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::Apply).await;
    assert!(result.is_err());
    let err = match result {
        Ok(_) => panic!("bad migration history should fail compatibility check"),
        Err(err) => err,
    };

    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(message.contains("migrations newer than supported"));
    assert!(message.contains("Please update scryer"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_status_listing_reads_legacy_ledger_without_mutating_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_status_legacy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&pool)
    .await
    .expect("legacy migration ledger should be created");

    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         VALUES (1, 'init', CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(vec![1u8, 2, 3])
    .execute(&pool)
    .await
    .expect("legacy migration row should be inserted");

    let statuses = crate::migrations::list_applied_migrations(&pool)
        .await
        .expect("status listing should succeed");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].migration_checksum_algo, "inferred");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[test]
fn compile_source_bundle_rejects_unknown_rust_hook_ids() {
    let db_root = std::env::temp_dir().join(format!(
        "scryer_migration_hook_fixture_{}",
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(db_root.join("migrations")).expect("fixture migrations dir");
    std::fs::write(
        db_root.join("migrations/0001_initial.sql"),
        "CREATE TABLE example (id INTEGER PRIMARY KEY);\n",
    )
    .expect("write legacy migration");
    std::fs::write(
        db_root.join("migration_manifest.toml"),
        r#"
format_version = 1

[legacy_sql]
path = "migrations"
through_version = 1

[[migration]]
version = 2
description = "bad hook"
checksum_algo = "blake3"
steps = [
  { kind = "rust", hook_id = "missing_hook", engine = "all", scope = "all" },
]
"#,
    )
    .expect("write manifest");

    let error = crate::migration_assets::compile_source_bundle(&db_root)
        .expect_err("unknown hook id should fail manifest compilation");
    assert!(error.contains("unknown migration hook id 'missing_hook'"));

    let _ = std::fs::remove_dir_all(db_root);
}

#[tokio::test]
async fn specials_convergence_migration_repoints_legacy_season_zero_references() {
    let db = std::env::temp_dir().join(format!(
        "scryer_specials_convergence_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS title_history (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            episode_id TEXT,
            collection_id TEXT,
            event_type TEXT NOT NULL,
            source_title TEXT,
            quality TEXT,
            download_id TEXT,
            data_json TEXT,
            occurred_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
        )",
    )
    .execute(&pool)
    .await
    .expect("create legacy title_history compatibility table");

    for statement in [
        "CREATE TABLE IF NOT EXISTS releases (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS workflow_operations (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS download_submissions (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("create legacy compatibility table");
    }

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO titles (
            id, name, name_normalized, library_id, facet, monitored, status,
            tags, external_ids, root_folder_id, created_at
         )
         VALUES (?, ?, ?, ?, ?, 1, 'active', '[]', '[]', ?, ?)",
    )
    .bind("title-series")
    .bind("Legacy Series")
    .bind("legacy series")
    .bind(scryer_domain::default_library_id_for_facet(
        &scryer_domain::MediaFacet::Series,
    ))
    .bind("series")
    .bind(scryer_domain::root_folder_id_for_path("/data/series"))
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert title");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-specials")
    .bind("title-series")
    .bind("season")
    .bind("0")
    .bind("Season 0")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert legacy specials");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("canonical-specials")
    .bind("title-series")
    .bind("specials")
    .bind("0")
    .bind("Specials")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert canonical specials");

    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("episode-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("special")
    .bind("1")
    .bind("0")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy episode");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-legacy")
    .bind("title-series")
    .bind("episode")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("legacy-specials")
    .execute(&pool)
    .await
    .expect("insert legacy wanted item");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-canonical")
    .bind("title-series")
    .bind("episode")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("canonical-specials")
    .execute(&pool)
    .await
    .expect("insert canonical wanted item");

    sqlx::query(
        "INSERT INTO title_history
         (id, title_id, collection_id, event_type, occurred_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("history-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("imported")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy title history row");

    let migration_sql =
        include_str!("../../../scryer/src/db/migrations/0070_specials_collection_convergence.sql");
    for statement in migration_sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("run migration statement");
    }

    let collections: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, collection_type FROM collections WHERE title_id = ? ORDER BY id",
    )
    .bind("title-series")
    .fetch_all(&pool)
    .await
    .expect("load collections");
    assert_eq!(
        collections,
        vec![("canonical-specials".to_string(), "specials".to_string())]
    );

    let episode_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM episodes WHERE id = ?")
            .bind("episode-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated episode collection");
    assert_eq!(episode_collection, "canonical-specials");

    let wanted_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM wanted_items WHERE collection_id = ? ORDER BY id")
            .bind("canonical-specials")
            .fetch_all(&pool)
            .await
            .expect("load wanted items");
    assert_eq!(wanted_ids, vec!["wanted-canonical".to_string()]);

    let history_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM title_history WHERE id = ?")
            .bind("history-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated title history collection");
    assert_eq!(history_collection, "canonical-specials");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_apply_then_validate_is_idempotent() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_then_apply_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    drop(services);

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("applied DB should pass validate mode");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0155_allows_emby_external_accounts_and_preserves_legacy_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("create SQLite migration fixture");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    sqlx::raw_sql(
        "CREATE TABLE users (id TEXT PRIMARY KEY);
         CREATE TABLE media_server_connections (id TEXT PRIMARY KEY);
         CREATE TABLE emby_media_server_details (
             connection_id TEXT PRIMARY KEY,
             api_key_encrypted TEXT NOT NULL
         );
         CREATE TABLE user_external_accounts (
             id TEXT PRIMARY KEY,
             user_id TEXT NOT NULL,
             provider TEXT NOT NULL,
             connection_id TEXT NOT NULL,
             external_user_id TEXT,
             username TEXT NOT NULL,
             display_name TEXT,
             avatar_url TEXT,
             status TEXT NOT NULL,
             verified_at TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             last_login_at TEXT,
             FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
             FOREIGN KEY (connection_id) REFERENCES media_server_connections(id),
             CHECK (provider IN ('plex', 'jellyfin')),
             CHECK (status IN ('pending_claim', 'active', 'disabled'))
         );
         CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
             ON user_external_accounts (provider, connection_id, LOWER(username))
             WHERE status = 'pending_claim' AND external_user_id IS NULL;
         CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
             ON user_external_accounts (provider, connection_id, external_user_id);
         CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
             ON user_external_accounts (user_id, provider, connection_id);
         CREATE INDEX idx_user_external_accounts_user_status
             ON user_external_accounts (user_id, status);
         INSERT INTO users (id) VALUES ('plex-user'), ('jellyfin-user'), ('emby-user');
         INSERT INTO media_server_connections (id)
             VALUES ('plex-main'), ('jellyfin-main'), ('emby-main');
         INSERT INTO emby_media_server_details (connection_id, api_key_encrypted)
             VALUES ('emby-main', 'encrypted-key');
         INSERT INTO user_external_accounts (
             id, user_id, provider, connection_id, external_user_id, username,
             display_name, avatar_url, status, verified_at, created_at, updated_at,
             last_login_at
         ) VALUES
             ('plex-account', 'plex-user', 'plex', 'plex-main', 'plex-id', 'Plex User',
              'Plex Display', '/plex-avatar', 'active', '2026-01-01T00:00:00Z',
              '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '2026-01-03T00:00:00Z'),
             ('jellyfin-invite', 'jellyfin-user', 'jellyfin', 'jellyfin-main',
              'jellyfin-id', 'Jellyfin User', NULL, NULL, 'pending_claim', NULL,
              '2026-02-01T00:00:00Z', '2026-02-02T00:00:00Z', NULL);",
    )
    .execute(&pool)
    .await
    .expect("initialize pre-0155 schema");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0155_emby_first_class.sql"),
    )
    .await;

    let legacy: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, provider, status, last_login_at
           FROM user_external_accounts
          ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read preserved legacy accounts");
    assert_eq!(
        legacy,
        vec![
            (
                "jellyfin-invite".into(),
                "jellyfin".into(),
                "pending_claim".into(),
                None,
            ),
            (
                "plex-account".into(),
                "plex".into(),
                "active".into(),
                Some("2026-01-03T00:00:00Z".into()),
            ),
        ]
    );

    sqlx::query(
        "INSERT INTO user_external_accounts (
             id, user_id, provider, connection_id, external_user_id, username,
             status, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("emby-invite")
    .bind("emby-user")
    .bind("emby")
    .bind("emby-main")
    .bind("emby-local-user-id")
    .bind("Emby User")
    .bind("pending_claim")
    .bind("2026-03-01T00:00:00Z")
    .bind("2026-03-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("0155 must allow Emby invite rows");
    let emby_provider: String =
        sqlx::query_scalar("SELECT provider FROM user_external_accounts WHERE id = 'emby-invite'")
            .fetch_one(&pool)
            .await
            .expect("round-trip Emby invite provider");
    assert_eq!(emby_provider, "emby");

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master
          WHERE type = 'index' AND name LIKE 'idx_user_external_accounts_%'
          ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("list restored external account indexes");
    assert_eq!(
        indexes.len(),
        4,
        "all external account indexes must survive"
    );

    let invalid_provider = sqlx::query(
        "INSERT INTO user_external_accounts (
             id, user_id, provider, connection_id, username, status, created_at, updated_at
         ) VALUES ('invalid', 'emby-user', 'unknown', 'emby-main', 'Invalid',
                   'pending_claim', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z')",
    )
    .execute(&pool)
    .await;
    assert!(
        invalid_provider.is_err(),
        "unknown providers remain rejected"
    );
}

#[tokio::test]
async fn migration_0140_rollup_creates_scheduler_tables_and_rss_gap_columns() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0140_scheduler_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    for table in [
        "upstream_scheduler_states",
        "upstream_destination_cooldowns",
        "upstream_scheduler_rss_cadence",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name = ?",
        )
        .bind(table)
        .fetch_one(&services.pool)
        .await
        .expect("sqlite_master query should succeed");
        assert_eq!(exists, 1, "{table} should exist after migrations apply");
    }

    let rss_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('upstream_scheduler_rss_cadence')")
            .fetch_all(&services.pool)
            .await
            .expect("rss cadence columns should load");
    for column in [
        "host_key",
        "account_quota_key",
        "destination_key",
        "rss_request_key",
        "target_interval_seconds",
        "latest_safe_poll_at",
        "last_seen_release_identity",
        "last_seen_release_published_at",
        "last_feed_gap_start_at",
        "last_feed_gap_end_at",
    ] {
        assert!(
            rss_columns.iter().any(|name| name == column),
            "upstream_scheduler_rss_cadence should include {column}; columns were {rss_columns:?}"
        );
    }

    let destination_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('upstream_destination_cooldowns')")
            .fetch_all(&services.pool)
            .await
            .expect("destination cooldown columns should load");
    for column in [
        "destination_key",
        "cooldown_until",
        "retry_after_seconds",
        "source",
        "observed_at",
    ] {
        assert!(
            destination_columns.iter().any(|name| name == column),
            "upstream_destination_cooldowns should include {column}; columns were {destination_columns:?}"
        );
    }

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0140_uses_owner_scoped_metadata_storage_only() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0140_owner_metadata_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    for table in [
        "title_metadata_tags",
        "title_metadata_tag_sources",
        "title_metadata_tag_source_keys",
        "title_metadata_rating_summaries",
        "title_metadata_rating_sources",
        "title_metadata_external_ratings",
        "discovery_title_metadata_tags",
        "discovery_title_metadata_tag_sources",
        "discovery_title_metadata_tag_source_keys",
        "discovery_title_metadata_rating_summaries",
        "discovery_title_metadata_rating_sources",
        "discovery_title_metadata_external_ratings",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name = ?",
        )
        .bind(table)
        .fetch_one(&services.pool)
        .await
        .expect("owner metadata table lookup should succeed");
        assert_eq!(exists, 1, "{table} should exist after migrations apply");
    }

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0147_retires_w500_variants_and_0148_adds_extensible_proxy_tables() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0147_w500_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pre-0147 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(146), true)
        .await
        .expect("migrations through 0146 should apply");

    let now = chrono::Utc::now().to_rfc3339();
    let library_id = scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
    let root_folder_id: String = sqlx::query_scalar(
        "SELECT id FROM library_roots
          WHERE library_id = ?1
          ORDER BY is_default DESC, path
          LIMIT 1",
    )
    .bind(&library_id)
    .fetch_one(&pool)
    .await
    .expect("default movie root should exist");
    for (id, poster_url, local_path) in [
        (
            "title-w500-only",
            "https://image.tmdb.org/t/p/w500/only.jpg",
            "/images/titles/title-w500-only/poster/w500?v=legacy",
        ),
        (
            "title-shared",
            "https://image.tmdb.org/t/p/w500/shared.jpg",
            "/images/titles/title-shared/poster/w500?v=legacy",
        ),
        (
            "title-supported",
            "https://image.tmdb.org/t/p/w300/supported.jpg",
            "/images/titles/title-supported/poster/w250?v=legacy",
        ),
        (
            "title-unrelated-w5000",
            "https://image.tmdb.org/t/p/w500/unrelated.jpg",
            "/images/titles/title-unrelated-w5000/poster/w5000?v=legacy",
        ),
    ] {
        sqlx::query(
            "INSERT INTO titles (
                id, name, name_normalized, library_id, root_folder_id, facet,
                created_at, poster_url, poster_local_path
             ) VALUES (?1, ?1, ?1, ?2, ?3, 'movie', ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(&library_id)
        .bind(&root_folder_id)
        .bind(&now)
        .bind(poster_url)
        .bind(local_path)
        .execute(&pool)
        .await
        .expect("title fixture should insert");
        sqlx::query(
            "INSERT INTO title_images (
                id, title_id, provider, kind, source_url, source_format,
                source_width, source_height, created_at, updated_at
             ) VALUES (?1, ?2, 'tmdb', 'poster', ?3, 'jpeg', 500, 750, ?4, ?4)",
        )
        .bind(format!("image-{id}"))
        .bind(id)
        .bind(poster_url)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("title image fixture should insert");
    }

    let w500_only_digest = format!("blake3:{}", "a".repeat(64));
    let supported_digest = format!("blake3:{}", "b".repeat(64));
    let shared_digest = format!("blake3:{}", "c".repeat(64));
    for (digest, bytes) in [
        (&w500_only_digest, vec![1_u8]),
        (&supported_digest, vec![2_u8]),
        (&shared_digest, vec![3_u8]),
    ] {
        sqlx::query(
            "INSERT INTO title_image_blobs (
                digest, format, width, height, bytes, created_at, updated_at
             ) VALUES (?1, 'avif', 250, 375, ?2, ?3, ?3)",
        )
        .bind(digest)
        .bind(bytes)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("title image blob fixture should insert");
    }
    for (id, image_id, variant, digest) in [
        (
            "variant-w500-only",
            "image-title-w500-only",
            "w500",
            &w500_only_digest,
        ),
        (
            "variant-shared-w500",
            "image-title-shared",
            "w500",
            &shared_digest,
        ),
        (
            "variant-shared-w250",
            "image-title-shared",
            "w250",
            &shared_digest,
        ),
        (
            "variant-supported-w250",
            "image-title-supported",
            "w250",
            &supported_digest,
        ),
    ] {
        sqlx::query(
            "INSERT INTO title_image_variants (
                id, title_image_id, variant_key, blob_digest, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        )
        .bind(id)
        .bind(image_id)
        .bind(variant)
        .bind(digest)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("title image variant fixture should insert");
    }
    pool.close().await;

    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("0147 and 0148 should apply");
    let w500_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants WHERE variant_key = 'w500'")
            .fetch_one(&services.pool)
            .await
            .expect("retired variant count should load");
    assert_eq!(w500_count, 0);

    let remaining_digests: Vec<String> =
        sqlx::query_scalar("SELECT digest FROM title_image_blobs ORDER BY digest")
            .fetch_all(&services.pool)
            .await
            .expect("remaining blob digests should load");
    assert_eq!(remaining_digests, vec![supported_digest, shared_digest]);

    let local_paths: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, poster_local_path
           FROM titles
          WHERE id IN ('title-shared', 'title-unrelated-w5000', 'title-w500-only')
          ORDER BY id",
    )
    .fetch_all(&services.pool)
    .await
    .expect("migrated local paths should load");
    assert_eq!(
        local_paths,
        vec![
            (
                "title-shared".to_string(),
                Some("/images/titles/title-shared/poster/w250?v=cccccccccccccccc".to_string()),
            ),
            (
                "title-unrelated-w5000".to_string(),
                Some("/images/titles/title-unrelated-w5000/poster/w5000?v=legacy".to_string()),
            ),
            ("title-w500-only".to_string(), None),
        ]
    );
    let retained_source_url: String =
        sqlx::query_scalar("SELECT poster_url FROM titles WHERE id = 'title-w500-only'")
            .fetch_one(&services.pool)
            .await
            .expect("source URL should load");
    assert_eq!(
        retained_source_url,
        "https://image.tmdb.org/t/p/w500/only.jpg"
    );

    sqlx::query(
        "INSERT INTO image_proxy_sources (
            token, upstream_url, owner_type, owner_id, image_kind, fallback_class, last_seen_at
         ) VALUES ('person-token', NULL, 'person', 'person-1', 'person', 'portrait', ?1)",
    )
    .bind(&now)
    .execute(&services.pool)
    .await
    .expect("future image kind should fit generic source table");
    sqlx::query(
        "INSERT INTO image_proxy_cache_entries (
            token, variant, content_type, byte_size, fetched_at, last_accessed_at
         ) VALUES ('person-token', 'profile', 'image/jpeg', 12, ?1, ?1)",
    )
    .bind(&now)
    .execute(&services.pool)
    .await
    .expect("future variant should fit generic cache table");

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0154_backfills_only_unique_indexer_names_and_enforces_delete_safety_net() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database should open");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys should enable");
    sqlx::raw_sql(
        "CREATE TABLE download_clients (id TEXT PRIMARY KEY);
         CREATE TABLE indexers (id TEXT PRIMARY KEY, name TEXT NOT NULL, updated_at TEXT NOT NULL);
         CREATE TABLE pending_releases (id TEXT PRIMARY KEY, indexer_source TEXT);
         INSERT INTO download_clients (id) VALUES ('client-1');
         INSERT INTO indexers (id, name, updated_at) VALUES
             ('unique-id', 'Unique', '2026-01-01T00:00:00Z'),
             ('dup-a', 'Duplicate', '2026-01-01T00:00:00Z'),
             ('dup-b', 'Duplicate', '2026-01-01T00:00:00Z');
         INSERT INTO pending_releases (id, indexer_source) VALUES
             ('pending-unique', 'Unique'),
             ('pending-duplicate', 'Duplicate'),
             ('pending-missing', 'Missing');",
    )
    .execute(&pool)
    .await
    .expect("legacy fixture should initialize");

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0154_indexer_download_client_mapping.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0154 should apply");

    let backfilled: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, indexer_id FROM pending_releases ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("pending provenance should load");
    assert_eq!(
        backfilled,
        vec![
            ("pending-duplicate".to_string(), None),
            ("pending-missing".to_string(), None),
            ("pending-unique".to_string(), Some("unique-id".to_string())),
        ]
    );

    let default_mapping: Option<String> =
        sqlx::query_scalar("SELECT download_client_id FROM indexers WHERE id = 'unique-id'")
            .fetch_one(&pool)
            .await
            .expect("new mapping column should load");
    assert_eq!(default_mapping, None);
    sqlx::query("UPDATE indexers SET download_client_id = 'client-1' WHERE id = 'unique-id'")
        .execute(&pool)
        .await
        .expect("mapping should set");
    sqlx::query("DELETE FROM download_clients WHERE id = 'client-1'")
        .execute(&pool)
        .await
        .expect("client deletion should use database safety net");
    let cleared_mapping: Option<String> =
        sqlx::query_scalar("SELECT download_client_id FROM indexers WHERE id = 'unique-id'")
            .fetch_one(&pool)
            .await
            .expect("cleared mapping should load");
    assert_eq!(cleared_mapping, None);
}

#[tokio::test]
async fn migration_0180_postgres_rekeys_constraints_and_compares_fresh_indexes() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_0180_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    let fresh_schema = format!("{schema}_fresh");
    for schema_name in [&schema, &fresh_schema] {
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema_name}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create postgres schema: {error}"))
            })?;
    }

    let schema_url = |schema_name: &str| -> AppResult<url::Url> {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema_name}"));
        Ok(url)
    };
    let upgraded_url = schema_url(&schema)?;
    let fresh_url = schema_url(&fresh_schema)?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(upgraded_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres schema: {error}"))
        })?;
    let fresh_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(fresh_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open fresh postgres schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(179)).await?;
        let now = "2026-08-24T12:00:00Z";
        let first_id = "00000000-0000-4000-8000-000000000001";
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES ($1, 'scryer_submission', ($2::text)::timestamptz)",
        )
        .bind(first_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, submitted_at
             ) VALUES ($1, 'pg-title-0180', 'series', 'pg-client-0180', 'qbittorrent',
                       'reused-native', ($2::text)::timestamptz)",
        )
        .bind(first_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_submission_episode_links (
                download_client_id, download_client_type, download_client_item_id, episode_id
             ) VALUES ('pg-client-0180', 'qbittorrent', 'reused-native', 'pg-episode-0180')",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at
             ) VALUES ($1, 'pg-client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ($2::text)::timestamptz)",
        )
        .bind(first_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, client_id, client_type,
                download_client_item_id, tracked_state, created_at, updated_at
             ) VALUES ('pg-state-0180', 'download:pg-0180', $1, 'legacy-pg-0180',
                       'pg-client-0180', 'qbittorrent', 'reused-native', 'queued', ($2::text)::timestamptz, ($2::text)::timestamptz)",
        )
        .bind(first_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO imports (
                id, source_system, source_ref, import_type, payload_json, created_at, updated_at,
                canonical_download_id
             ) VALUES ('pg-import-0180', 'qbittorrent', 'reused-native', 'series_download',
                       '{}', ($1::text)::timestamptz, ($1::text)::timestamptz, $2)",
        )
        .bind(now)
        .bind(first_id)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_import_artifacts (
                id, source_system, source_ref, normalized_file_name, media_kind, result, created_at,
                canonical_download_id
             ) VALUES ('pg-artifact-0180', 'qbittorrent', 'reused-native', 'episode.mkv',
                       'episode', 'imported', ($1::text)::timestamptz, $2)",
        )
        .bind(now)
        .bind(first_id)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_queue_commands (
                id, action, client_type, download_client_item_id, status, created_at, updated_at,
                canonical_download_id
             ) VALUES ('pg-queue-0180', 'remove', 'qbittorrent', 'reused-native', 'queued',
                       ($1::text)::timestamptz, ($1::text)::timestamptz, $2)",
        )
        .bind(now)
        .bind(first_id)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let services = crate::PostgresServices::new_with_mode(
            upgraded_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);

        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT download_id FROM download_submission_episode_links
                  WHERE episode_id = 'pg-episode-0180'",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?,
            first_id
        );
        let first_fk: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
              FROM pg_constraint con
              JOIN pg_class relation ON relation.oid = con.conrelid
              JOIN pg_namespace ns ON ns.oid = relation.relnamespace
              WHERE con.conname = 'download_submissions_id_fkey'
                AND ns.nspname = $1",
        )
        .bind(&schema)
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(first_fk, 1);
        let index_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_indexes
              WHERE schemaname = $1
                AND indexname = 'idx_download_client_bindings_active_locator_unique'",
        )
        .bind(&schema)
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(index_count, 1);

        let second_id = "00000000-0000-4000-8000-000000000002";
        let third_id = "00000000-0000-4000-8000-000000000003";
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES ($1, 'scryer_submission', ($3::text)::timestamptz), ($2, 'scryer_submission', ($3::text)::timestamptz)",
        )
        .bind(second_id)
        .bind(third_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let active_collision = sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at
             ) VALUES ($1, 'pg-client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ($2::text)::timestamptz)",
        )
        .bind(second_id)
        .bind(now)
        .execute(&pool)
        .await;
        assert!(active_collision.is_err(), "partial active-binding index must reject a collision");
        sqlx::query("UPDATE download_client_bindings SET ended_at = ($1::text)::timestamptz WHERE download_id = $2")
            .bind(now)
            .bind(first_id)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at
             ) VALUES ($1, 'pg-client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ($2::text)::timestamptz)",
        )
        .bind(second_id)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, submitted_at
             ) VALUES ($1, 'pg-title-0180-readd', 'series', 'pg-client-0180', 'qbittorrent',
                       'reused-native', '2026-08-24T12:00:01Z')",
        )
        .bind(second_id)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let second_active_collision = sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at
             ) VALUES ($1, 'pg-client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ($2::text)::timestamptz)",
        )
        .bind(third_id)
        .bind(now)
        .execute(&pool)
        .await;
        assert!(second_active_collision.is_err(), "only one active binding may remain");
        let null_canonical = sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state, created_at, updated_at
             ) VALUES ('pg-state-null', 'download:pg-null', NULL, 'legacy-pg-null', 'queued',
                       ($1::text)::timestamptz, ($1::text)::timestamptz)",
        )
        .bind(now)
        .execute(&pool)
        .await;
        assert!(null_canonical.is_err(), "canonical download id must be non-null");
        let invalid_import = sqlx::query(
            "INSERT INTO imports (
                id, source_system, source_ref, import_type, payload_json, created_at, updated_at,
                canonical_download_id
             ) VALUES ('pg-import-invalid', 'qbittorrent', 'missing-native', 'series_download',
                       '{}', ($1::text)::timestamptz, ($1::text)::timestamptz, 'missing-download')",
        )
        .bind(now)
        .execute(&pool)
        .await;
        assert!(invalid_import.is_err(), "dependent canonical foreign key must reject unknown IDs");

        let services = crate::PostgresServices::new_with_mode(
            upgraded_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);
        let ledger_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 179 AND success = TRUE",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(ledger_count, 1);

        crate::postgres::replay_source_catalog_for_fresh_install(&fresh_pool, None).await?;
        let table_query = "SELECT table_name, column_name, data_type, is_nullable
                           FROM information_schema.columns
                           WHERE table_schema = $1
                             AND table_name IN (
                                 'downloads', 'download_client_bindings', 'download_submissions',
                                 'download_submission_episode_links', 'download_identity_states',
                                 'imports', 'download_import_artifacts', 'download_queue_commands'
                             )
                           ORDER BY table_name, ordinal_position";
        let upgraded_tables: Vec<(String, String, String, String)> = sqlx::query_as(table_query)
            .bind(&schema)
            .fetch_all(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let fresh_tables: Vec<(String, String, String, String)> = sqlx::query_as(table_query)
            .bind(&fresh_schema)
            .fetch_all(&fresh_pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(fresh_tables, upgraded_tables);

        let index_query = "SELECT tablename, indexname, indexdef
                           FROM pg_indexes
                           WHERE schemaname = $1
                             AND tablename IN (
                                 'downloads', 'download_client_bindings', 'download_submissions',
                                 'download_submission_episode_links', 'download_identity_states',
                                 'imports', 'download_import_artifacts', 'download_queue_commands'
                             )
                           ORDER BY tablename, indexname";
        let upgraded_indexes: Vec<(String, String, String)> =
            sqlx::query_as::<_, (String, String, String)>(index_query)
                .bind(&schema)
                .fetch_all(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .into_iter()
            .map(|(table, name, definition)| (table, name, definition.replace(&schema, "<schema>")))
            .collect();
        let fresh_indexes: Vec<(String, String, String)> =
            sqlx::query_as::<_, (String, String, String)>(index_query)
                .bind(&fresh_schema)
                .fetch_all(&fresh_pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .into_iter()
            .map(|(table, name, definition)| {
                (table, name, definition.replace(&fresh_schema, "<schema>"))
            })
            .collect();
        assert_eq!(fresh_indexes, upgraded_indexes);
        Ok(())
    }
    .await;

    drop(fresh_pool);
    drop(pool);
    let fresh_cleanup = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {fresh_schema} CASCADE"
    )))
    .execute(&admin_pool)
    .await;
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    drop(admin_pool);
    fresh_cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop fresh postgres schema: {error}"))
    })?;
    cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop postgres schema: {error}"))
    })?;
    result
}

#[tokio::test]
async fn migration_0140_upgrades_v0_16_8_title_metadata_and_media_in_place() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0140_v0_16_8_upgrade_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("0.16.8 database should open");

    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(139), true)
        .await
        .expect("migrations through the 0.16.8 head should apply");

    let library_id = scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
    let (root_folder_id, root_path): (String, String) = sqlx::query_as(
        "SELECT id, path FROM library_roots WHERE library_id = ? ORDER BY is_default DESC, path LIMIT 1",
    )
    .bind(&library_id)
    .fetch_one(&pool)
    .await
    .expect("0.16.8 default movie root should exist");
    let media_path = format!(
        "{}/Sasaki and Miyano Graduation/movie.mkv",
        root_path.trim_end_matches('/')
    );
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO titles (
            id, name, name_normalized, library_id, facet, monitored, status,
            tags, external_ids, root_folder_id, genres, year, overview, created_at
         ) VALUES (?, ?, ?, ?, 'movie', 1, 'active', '[]', ?, ?, ?, ?, ?, ?)",
    )
    .bind("title-v0-16-8")
    .bind("Sasaki and Miyano: Graduation")
    .bind("sasaki and miyano graduation")
    .bind(&library_id)
    .bind(r#"[{"source":"tmdb","value":"998731"}]"#)
    .bind(&root_folder_id)
    .bind(r#"["Drama","Slice of Life"]"#)
    .bind(2023i32)
    .bind("A preserved 0.16.8 overview")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("0.16.8 title should insert");

    for (id, name, normalized) in [
        (
            "title-v0-16-8-image-copy",
            "Sasaki and Miyano Image Copy",
            "sasaki and miyano image copy",
        ),
        (
            "title-v0-16-8-corrupt-image",
            "Sasaki and Miyano Corrupt Image",
            "sasaki and miyano corrupt image",
        ),
    ] {
        sqlx::query(
            "INSERT INTO titles (
                id, name, name_normalized, library_id, facet, monitored, status,
                tags, external_ids, root_folder_id, genres, year, overview, created_at
             ) VALUES (?, ?, ?, ?, 'movie', 1, 'active', '[]', '[]', ?, '[]', 2023, '', ?)",
        )
        .bind(id)
        .bind(name)
        .bind(normalized)
        .bind(&library_id)
        .bind(&root_folder_id)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("0.16.8 image fixture title should insert");
    }

    sqlx::query(
        "INSERT INTO media_files (
            id, title_id, file_path, size_bytes, scan_status, created_at
         ) VALUES (?, ?, ?, ?, 'complete', ?)",
    )
    .bind("file-v0-16-8")
    .bind("title-v0-16-8")
    .bind(&media_path)
    .bind(1234i64)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("0.16.8 media file should insert");

    let legacy_image_bytes = vec![4_u8, 5, 6];
    let legacy_image_digest = format!("blake3:{}", blake3::hash(&legacy_image_bytes).to_hex());
    for (title_id, image_id) in [
        ("title-v0-16-8", "image-v0-16-8-a"),
        ("title-v0-16-8-image-copy", "image-v0-16-8-b"),
        ("title-v0-16-8-corrupt-image", "image-v0-16-8-corrupt"),
    ] {
        sqlx::query(
            "INSERT INTO title_images (
                id, title_id, provider, provider_image_id, kind, source_url,
                source_etag, source_last_modified, source_format, source_width,
                source_height, created_at, updated_at
             ) VALUES (?, ?, 'tvdb', NULL, 'poster', ?, NULL, NULL, 'jpeg', 1000, 1500, ?, ?)",
        )
        .bind(image_id)
        .bind(title_id)
        .bind(format!("https://artworks.thetvdb.com/{image_id}.jpg"))
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("0.16.8 title image should insert");
        sqlx::query("UPDATE titles SET poster_local_path = ? WHERE id = ?")
            .bind(format!("/images/titles/{title_id}/poster/w250?v=legacy"))
            .bind(title_id)
            .execute(&pool)
            .await
            .expect("0.16.8 local image path should update");
    }
    for (variant_id, image_id, bytes) in [
        (
            "variant-v0-16-8-a",
            "image-v0-16-8-a",
            legacy_image_bytes.clone(),
        ),
        (
            "variant-v0-16-8-b",
            "image-v0-16-8-b",
            legacy_image_bytes.clone(),
        ),
        (
            "variant-v0-16-8-corrupt",
            "image-v0-16-8-corrupt",
            vec![9_u8, 9, 9],
        ),
    ] {
        sqlx::query(
            "INSERT INTO title_image_variants (
                id, title_image_id, variant_key, path, format, width, height,
                bytes, digest, created_at, updated_at
             ) VALUES (?, ?, 'w250', ?, 'avif', 250, 375, ?, ?, ?, ?)",
        )
        .bind(variant_id)
        .bind(image_id)
        .bind(format!("/legacy-cache/{variant_id}.avif"))
        .bind(bytes)
        .bind(&legacy_image_digest)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("0.16.8 title image variant should insert");
    }

    pool.close().await;

    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("0.16.8 database should upgrade to 0.17");
    let title: (String, String, String, i32, String) = sqlx::query_as(
        "SELECT library_id, genres, external_ids, year, overview
           FROM titles
          WHERE id = 'title-v0-16-8'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("upgraded title should remain");
    assert_eq!(title.0, library_id);
    assert_eq!(title.1, r#"["Drama","Slice of Life"]"#);
    assert_eq!(title.2, r#"[{"source":"tmdb","value":"998731"}]"#);
    assert_eq!(title.3, 2023);
    assert_eq!(title.4, "A preserved 0.16.8 overview");

    let metadata_tags: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT tag_key, category, name
           FROM title_metadata_tags
          WHERE title_id = 'title-v0-16-8'
          ORDER BY sort_index",
    )
    .fetch_all(&services.pool)
    .await
    .expect("legacy genres should seed title-owned metadata tags");
    assert_eq!(
        metadata_tags,
        vec![
            (
                "metadata:genre:drama".to_string(),
                "genre".to_string(),
                "Drama".to_string(),
            ),
            (
                "metadata:genre:slice_of_life".to_string(),
                "genre".to_string(),
                "Slice of Life".to_string(),
            ),
        ]
    );

    let media: (String, String) =
        sqlx::query_as("SELECT title_id, file_path FROM media_files WHERE id = 'file-v0-16-8'")
            .fetch_one(&services.pool)
            .await
            .expect("upgraded media file should remain attached to its title");
    assert_eq!(media.0, "title-v0-16-8");
    assert_eq!(media.1, media_path);

    let image_blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
        .fetch_one(&services.pool)
        .await
        .expect("migrated image blob count should load");
    let migrated_image_variants: Vec<(String, String)> = sqlx::query_as(
        "SELECT ti.title_id, tiv.blob_digest
           FROM title_image_variants tiv
           JOIN title_images ti ON ti.id = tiv.title_image_id
          ORDER BY ti.title_id",
    )
    .fetch_all(&services.pool)
    .await
    .expect("migrated image variants should load");
    assert_eq!(image_blob_count, 1);
    assert_eq!(
        migrated_image_variants,
        vec![
            ("title-v0-16-8".to_string(), legacy_image_digest.clone(),),
            ("title-v0-16-8-image-copy".to_string(), legacy_image_digest,),
        ]
    );
    let image_paths: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, poster_local_path
           FROM titles
          WHERE id IN (
              'title-v0-16-8',
              'title-v0-16-8-image-copy',
              'title-v0-16-8-corrupt-image'
          )
          ORDER BY id",
    )
    .fetch_all(&services.pool)
    .await
    .expect("upgraded image paths should load");
    assert_eq!(
        image_paths,
        vec![
            (
                "title-v0-16-8".to_string(),
                Some("/images/titles/title-v0-16-8/poster/w250?v=legacy".to_string(),),
            ),
            ("title-v0-16-8-corrupt-image".to_string(), None),
            (
                "title-v0-16-8-image-copy".to_string(),
                Some("/images/titles/title-v0-16-8-image-copy/poster/w250?v=legacy".to_string(),),
            ),
        ]
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[test]
fn migration_0140_sqlite_and_postgres_rollup_sources_include_scheduler_columns() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate should live under repo/crates");
    let sqlite = std::fs::read_to_string(
        repo_root.join("crates/scryer/src/db/migrations/0140_0_17_release_rollup.sql"),
    )
    .expect("sqlite 0140 rollup migration should load");
    let postgres = std::fs::read_to_string(
        repo_root.join("crates/scryer/src/db/postgres/migrations/0140_0_17_release_rollup.sql"),
    )
    .expect("postgres 0140 rollup migration should load");

    for sql in [&sqlite, &postgres] {
        for required in [
            "CREATE TABLE IF NOT EXISTS upstream_scheduler_states",
            "CREATE TABLE IF NOT EXISTS upstream_destination_cooldowns",
            "CREATE TABLE IF NOT EXISTS upstream_scheduler_rss_cadence",
            "CREATE TABLE IF NOT EXISTS user_ui_settings",
            "CREATE TABLE IF NOT EXISTS user_ui_table_columns",
            "CREATE TABLE IF NOT EXISTS title_metadata_tags",
            "CREATE TABLE IF NOT EXISTS title_metadata_tag_sources",
            "CREATE TABLE IF NOT EXISTS title_metadata_tag_source_keys",
            "CREATE TABLE IF NOT EXISTS title_metadata_rating_summaries",
            "CREATE TABLE IF NOT EXISTS title_metadata_rating_sources",
            "CREATE TABLE IF NOT EXISTS title_metadata_external_ratings",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_tags",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_tag_sources",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_tag_source_keys",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_rating_summaries",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_rating_sources",
            "CREATE TABLE IF NOT EXISTS discovery_title_metadata_external_ratings",
            "quota_observed_at",
            "quota_probe_after",
            "quota_reset_at",
            "retry_after_seconds",
            "rss_request_key",
            "host_key",
            "last_seen_release_identity",
            "last_seen_release_published_at",
            "last_feed_gap_start_at",
            "last_feed_gap_end_at",
        ] {
            assert!(
                sql.contains(required),
                "0140 rollup migration source should include {required}"
            );
        }
    }
}

#[tokio::test]
async fn migration_0079_faceted_projection_allows_cross_facet_duplicates_and_seeds_only_tvdb_titles()
 {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-1")
    .bind("Series")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-1")
    .bind("Movie")
    .bind("movie")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-imdb")
    .bind("IMDb Only")
    .bind("movie")
    .bind(r#"[{"source":"imdb","value":"tt1234567"}]"#)
    .execute(&pool)
    .await
    .expect("insert legacy titles");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"),
    )
    .await;

    let faceted_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT title_id, facet, external_id
         FROM title_external_ids
         WHERE source = 'tvdb'
         ORDER BY facet, title_id",
    )
    .fetch_all(&pool)
    .await
    .expect("load projected faceted tvdb ids");
    assert_eq!(
        faceted_rows,
        vec![
            (
                "movie-1".to_string(),
                "movie".to_string(),
                "123".to_string()
            ),
            (
                "series-1".to_string(),
                "series".to_string(),
                "123".to_string()
            ),
        ]
    );

    let due_now: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("load hydration due markers");
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-imdb")
            .expect("imdb title marker")
            .1
            .is_none()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-1")
            .expect("movie tvdb marker")
            .1
            .is_some()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "series-1")
            .expect("series tvdb marker")
            .1
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_same_facet_duplicate_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_duplicate_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on same-facet duplicate")
    };
    assert!(
        err.to_string().contains("UNIQUE"),
        "expected uniqueness failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let legacy_external_id: String =
        sqlx::query_scalar("SELECT external_id FROM title_external_ids WHERE id = 'legacy-row'")
            .fetch_one(&pool)
            .await
            .expect("legacy row should remain");
    assert_eq!(legacy_external_id, "legacy");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_conflict_hint_lists_colliding_title_ids() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_conflict_hint_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let hint = crate::migrations::title_external_id_projection_conflict_hint(&pool)
        .await
        .expect("conflict hint should be present");
    assert!(hint.contains("series/tvdb/999"));
    assert!(hint.contains("series-a"));
    assert!(hint.contains("series-b"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_invalid_projection_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_invalid_json_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind("series-bad")
    .bind("Broken Series")
    .bind("series")
    .bind("{not-valid-json")
    .execute(&pool)
    .await
    .expect("insert malformed legacy title");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on malformed external_ids json")
    };
    assert!(
        err.to_string().contains("malformed"),
        "expected malformed json failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0104_accepts_plain_path_settings_without_choking_on_unrelated_invalid_json() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE settings_definitions (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            scope TEXT NOT NULL,
            key_name TEXT NOT NULL,
            data_type TEXT NOT NULL,
            default_value_json TEXT,
            is_sensitive INTEGER NOT NULL DEFAULT 0,
            validation_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_definitions should create");

    sqlx::query(
        "CREATE TABLE settings_values (
            id TEXT PRIMARY KEY,
            setting_definition_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_id TEXT,
            value_json TEXT NOT NULL,
            source TEXT NOT NULL,
            updated_by_user_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_values should create");

    sqlx::query(
        "CREATE TEMP TABLE _default_library_roots (
            library_id TEXT NOT NULL,
            path TEXT NOT NULL,
            is_default INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("_default_library_roots should create");

    for (id, key_name) in [
        ("def-movies-path", "movies.path"),
        ("def-series-path", "series.path"),
        ("def-unrelated", "service:system:smg.client_key"),
    ] {
        sqlx::query(
            "INSERT INTO settings_definitions (
                id, category, scope, key_name, data_type, default_value_json,
                is_sensitive, validation_json, created_at, updated_at
            ) VALUES (?, 'test', 'system', ?, 'string', '\"\"', 0, NULL, 'now', 'now')",
        )
        .bind(id)
        .bind(key_name)
        .execute(&pool)
        .await
        .expect("setting definition should insert");
    }

    sqlx::query(
        "INSERT INTO settings_values (
            id, setting_definition_id, scope, scope_id, value_json, source,
            updated_by_user_id, created_at, updated_at
        ) VALUES
            ('row-movies', 'def-movies-path', 'media', NULL, '\"/Volumes/Media/Movies\"', 'test', NULL, 'now', 'now'),
            ('row-series', 'def-series-path', 'media', NULL, '/Volumes/Media/TV', 'test', NULL, 'now', 'now'),
            ('row-unrelated', 'def-unrelated', 'system', NULL, 'enc:v1:not-json', 'test', NULL, 'now', 'now')",
    )
    .execute(&pool)
    .await
    .expect("setting values should insert");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0104_first_class_libraries_and_permissions.sql"
    );
    let statement = migration_sql
        .split(';')
        .map(str::trim)
        .find(|statement| statement.starts_with("INSERT INTO _default_library_roots (library_id, path, is_default)\nSELECT\n    CASE sd.key_name\n        WHEN 'movies.path'"))
        .expect("0104 path backfill statement should exist");

    sqlx::query(statement)
        .execute(&pool)
        .await
        .expect("legacy plain path values should backfill without malformed json errors");

    let roots: Vec<(String, String)> =
        sqlx::query_as("SELECT library_id, path FROM _default_library_roots ORDER BY library_id")
            .fetch_all(&pool)
            .await
            .expect("backfilled roots should load");
    assert_eq!(
        roots,
        vec![
            (
                "movie_default_library".to_string(),
                "/Volumes/Media/Movies".to_string()
            ),
            (
                "series_default_library".to_string(),
                "/Volumes/Media/TV".to_string()
            ),
        ]
    );
}

async fn create_0136_test_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE titles (
            id TEXT PRIMARY KEY,
            library_id TEXT,
            facet TEXT NOT NULL,
            tags TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("titles should create");
    sqlx::query(
        "CREATE TABLE libraries (
            id TEXT PRIMARY KEY,
            facet TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("libraries should create");
    sqlx::query(
        "CREATE TABLE library_roots (
            id TEXT PRIMARY KEY,
            library_id TEXT NOT NULL,
            path TEXT NOT NULL,
            normalized_path TEXT,
            is_default INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00Z',
            updated_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00Z'
        )",
    )
    .execute(&pool)
    .await
    .expect("library_roots should create");
    pool
}

async fn run_0136_sqlite(pool: &sqlx::SqlitePool) -> Result<(), AppError> {
    run_embedded_migration(
        pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;

    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx).await?;
    tx.commit()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0136_title_root_folder_id_post.sql"
    ))
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}

#[tokio::test]
async fn migration_0136_rekeys_roots_and_backfills_concrete_title_root_ids() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");

    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('random-default-id', 'anime-library', '/Library/Default', '/library/default', 1),
            ('random-custom-id', 'anime-library', '/Library/Custom', '/library/custom', 0)",
    )
    .execute(&pool)
    .await
    .expect("library roots should insert");
    sqlx::query(
        "INSERT INTO titles (id, library_id, facet, tags)
         VALUES
            ('title-default', 'anime-library', 'anime', '[\"keep-default\"]'),
            ('title-custom', 'anime-library', 'anime', '[\"scryer:root-folder:/Library/Custom/\",\"keep-custom\"]'),
            ('title-unmatched', 'anime-library', 'anime', '[\"scryer:root-folder:/Library/Missing\",\"keep-unmatched\"]')",
    )
    .execute(&pool)
    .await
    .expect("titles should insert");

    run_0136_sqlite(&pool)
        .await
        .expect("0136 migration should run");

    let root_rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, path, normalized_path, is_default
           FROM library_roots
          ORDER BY path",
    )
    .fetch_all(&pool)
    .await
    .expect("migrated roots should query");
    let root_ids_by_path = root_rows
        .iter()
        .map(|(id, path, _, _)| (path.clone(), id.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        root_ids_by_path["/Library/Default"],
        scryer_domain::root_folder_id_for_path("/Library/Default")
    );
    assert_eq!(
        root_ids_by_path["/Library/Custom"],
        scryer_domain::root_folder_id_for_path("/Library/Custom")
    );
    assert_eq!(
        root_ids_by_path["/Library/Missing"],
        scryer_domain::root_folder_id_for_path("/Library/Missing")
    );
    assert!(
        root_rows
            .iter()
            .all(|(id, _, _, _)| id != "random-default-id" && id != "random-custom-id")
    );

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, root_folder_id, tags
           FROM titles
          ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("migrated titles should query");

    assert_eq!(rows[0].0, "title-custom");
    assert_eq!(rows[0].1, root_ids_by_path["/Library/Custom"]);
    let custom_tags: Vec<String> =
        serde_json::from_str(&rows[0].2).expect("custom tags should decode");
    assert_eq!(custom_tags, vec!["keep-custom".to_string()]);

    assert_eq!(rows[1].0, "title-default");
    assert_eq!(rows[1].1, root_ids_by_path["/Library/Default"]);
    let default_tags: Vec<String> =
        serde_json::from_str(&rows[1].2).expect("default tags should decode");
    assert_eq!(default_tags, vec!["keep-default".to_string()]);

    assert_eq!(rows[2].0, "title-unmatched");
    assert_eq!(rows[2].1, root_ids_by_path["/Library/Missing"]);
    let unmatched_tags: Vec<String> =
        serde_json::from_str(&rows[2].2).expect("unmatched tags should decode");
    assert_eq!(unmatched_tags, vec!["keep-unmatched".to_string()]);

    let orphan_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE root_folder_id IS NULL
             OR NOT EXISTS (
                SELECT 1 FROM library_roots
                 WHERE library_roots.id = titles.root_folder_id
                   AND library_roots.library_id = titles.library_id
             )",
    )
    .fetch_one(&pool)
    .await
    .expect("orphan count should query");
    assert_eq!(orphan_count, 0);
}

#[tokio::test]
async fn migration_0136_rejects_legacy_root_path_from_another_library() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('movie-root', 'movie-library', '/shared/root', '/shared/root', 1)",
    )
    .execute(&pool)
    .await
    .expect("movie root should insert");
    sqlx::query(
        "INSERT INTO titles (id, library_id, facet, tags)
         VALUES
            ('title-cross-root', 'anime-library', 'anime', '[\"scryer:root-folder:/shared/root\"]')",
    )
    .execute(&pool)
    .await
    .expect("title should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;
    let mut tx = pool.begin().await.expect("transaction should begin");
    let err =
        crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx)
            .await
            .expect_err("cross-library legacy root should fail");
    assert!(
        err.to_string()
            .contains("configured on library movie-library"),
        "unexpected migration error: {err}"
    );
}

#[tokio::test]
async fn migration_0136_rejects_duplicate_existing_root_paths_before_rekey() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('anime-root', 'anime-library', '/shared/root', '/shared/root', 1),
            ('movie-root', 'movie-library', '/shared/root/', '/shared/root', 1)",
    )
    .execute(&pool)
    .await
    .expect("roots should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;
    let mut tx = pool.begin().await.expect("transaction should begin");
    let err =
        crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx)
            .await
            .expect_err("duplicate root paths should fail before rekey");
    assert!(
        err.to_string()
            .contains("duplicate root paths must be merged before migration"),
        "unexpected migration error: {err}"
    );
}

#[tokio::test]
async fn migration_0156_queues_tvdb_titles_without_clearing_last_fetch() {
    let db = std::env::temp_dir().join(format!(
        "scryer_original_language_rehydration_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut tvdb_title = make_test_title("title-tvdb-original-language", None);
    tvdb_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    TitleRepository::create(&catalog, tvdb_title.clone())
        .await
        .expect("TVDB title should insert");
    let local_title = make_test_title("title-local-original-language", None);
    TitleRepository::create(&catalog, local_title.clone())
        .await
        .expect("local title should insert");

    sqlx::query(
        "UPDATE titles
            SET metadata_fetched_at = '2026-01-01T00:00:00Z',
                metadata_hydration_next_attempt_at = NULL,
                metadata_hydration_attempt_count = 7
          WHERE id IN (?, ?)",
    )
    .bind(&tvdb_title.id)
    .bind(&local_title.id)
    .execute(&services.pool)
    .await
    .expect("hydration state should seed");

    run_embedded_migration(
        &services.pool,
        include_str!("../../../scryer/src/db/migrations/0156_rehydrate_original_languages.sql"),
    )
    .await;

    let tvdb_state: (Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_fetched_at, metadata_hydration_next_attempt_at,
                metadata_hydration_attempt_count
           FROM titles WHERE id = ?",
    )
    .bind(&tvdb_title.id)
    .fetch_one(&services.pool)
    .await
    .expect("TVDB hydration state should load");
    assert_eq!(tvdb_state.0.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert!(tvdb_state.1.is_some());
    assert_eq!(tvdb_state.2, 0);

    let local_state: (Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_fetched_at, metadata_hydration_next_attempt_at,
                metadata_hydration_attempt_count
           FROM titles WHERE id = ?",
    )
    .bind(&local_title.id)
    .fetch_one(&services.pool)
    .await
    .expect("local hydration state should load");
    assert_eq!(local_state.0.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(local_state.1, None);
    assert_eq!(local_state.2, 7);

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn quarantined_0157_is_recorded_without_running_and_0160_normalizes_without_deleting() {
    // A pre-0157 database (0.18.11 and earlier) upgraded by the current binary:
    // 0157 must be recorded as applied — same version and checksum as the
    // catalog, so ledger validation stays green forever — without executing,
    // and 0160 must set unambiguous folders while touching no media_files row.
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0157_quarantine_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pre-0157 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(156), true)
        .await
        .expect("migrations through 0156 should apply");

    let now = chrono::Utc::now().to_rfc3339();
    let library_id =
        scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Series);
    let (root_folder_id, root_path): (String, String) = sqlx::query_as(
        "SELECT id, path FROM library_roots
          WHERE library_id = ?1
          ORDER BY is_default DESC, path
          LIMIT 1",
    )
    .bind(&library_id)
    .fetch_one(&pool)
    .await
    .expect("default series root should exist");
    let root = root_path.trim_end_matches('/').to_string();

    // "split": media in two folders — 0157 would keep the bigger folder and
    // DELETE the other row. "single": one folder, no stored folder_path.
    for id in ["split", "single"] {
        sqlx::query(
            "INSERT INTO titles (
                id, name, name_normalized, library_id, root_folder_id, facet, created_at
             ) VALUES (?1, ?1, ?1, ?2, ?3, 'series', ?4)",
        )
        .bind(id)
        .bind(&library_id)
        .bind(&root_folder_id)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("title fixture should insert");
    }
    for (id, title_id, path) in [
        (
            "split-a1",
            "split",
            format!("{root}/Split Show/Season 1/e1.mkv"),
        ),
        (
            "split-a2",
            "split",
            format!("{root}/Split Show/Season 1/e2.mkv"),
        ),
        (
            "split-b1",
            "split",
            format!("{root}/Split Show (2019)/e1.mkv"),
        ),
        ("single-1", "single", format!("{root}/Single Show/e1.mkv")),
    ] {
        sqlx::query(
            "INSERT INTO media_files (
                id, title_id, file_path, size_bytes, scan_status, created_at
             ) VALUES (?1, ?2, ?3, 100, 'complete', ?4)",
        )
        .bind(id)
        .bind(title_id)
        .bind(&path)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("media file fixture should insert");
    }

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("upgrade to head should apply");

    // Nothing was deleted.
    let media_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_files")
        .fetch_one(&pool)
        .await
        .expect("count media files");
    assert_eq!(media_count, 4, "0160 must never delete media_files rows");

    // 0157 is in the ledger with the catalog's own checksum, marked successful,
    // and 0160 ran after it.
    let catalog = crate::migrations::embedded_catalog().expect("embedded catalog");
    let expected_157 = catalog
        .migrations
        .iter()
        .find(|migration| migration.version == 157)
        .expect("0157 stays in the catalog");
    let (checksum_157, success_157): (Vec<u8>, i64) =
        sqlx::query_as("SELECT checksum, success FROM _sqlx_migrations WHERE version = 157")
            .fetch_one(&pool)
            .await
            .expect("0157 ledger row");
    assert_eq!(success_157, 1);
    assert_eq!(checksum_157, expected_157.checksum);
    let applied_160: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 160")
            .fetch_one(&pool)
            .await
            .expect("0160 ledger row");
    assert_eq!(applied_160, 1);

    // The unambiguous title got its folder; the split one was left for repair.
    let single_folder: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM titles WHERE id = 'single'")
            .fetch_one(&pool)
            .await
            .expect("single title");
    assert_eq!(
        single_folder.as_deref(),
        Some(format!("{root}/Single Show").as_str())
    );
    let split_folder: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM titles WHERE id = 'split'")
            .fetch_one(&pool)
            .await
            .expect("split title");
    assert_eq!(
        split_folder, None,
        "ambiguous ownership is not resolved by guessing"
    );

    // A second boot is a no-op: the ledger validates and nothing is pending.
    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::ValidateOnly)
        .await
        .expect("ledger with the recorded 0157 must validate");

    drop(pool);
    let _ = std::fs::remove_file(db);
}

/// Migration 0171: clear the score floor users calibrated against the old,
/// tier-inclusive scale.
///
/// A floor of a few thousand used to exclude the bottom of the barrel; with the
/// tier out of the number it excludes everything, as a hard block with no error
/// anywhere. The value lives inside the `scoring_config` JSON blob, so the key is
/// removed rather than set to null — `ScoringConfig` reads an absent key as
/// `None`. Profiles that never set it, and every other key in the blob, must be
/// left exactly as they were.
#[tokio::test]
async fn migration_0171_clears_only_the_configured_score_floor() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database should open");
    sqlx::raw_sql(
        r#"CREATE TABLE quality_profiles (
               id TEXT PRIMARY KEY,
               scoring_config TEXT NOT NULL DEFAULT '{}'
           );
           INSERT INTO quality_profiles (id, scoring_config) VALUES
             ('configured', '{"scoring_persona":"efficient","cutoff_tier":"1080P","min_score_to_grab":2500}'),
             ('explicit-null', '{"scoring_persona":"balanced","min_score_to_grab":null}'),
             ('untouched', '{"scoring_persona":"balanced","cutoff_tier":"2160P"}'),
             ('empty', '{}');"#,
    )
    .execute(&pool)
    .await
    .expect("quality-profile fixture should initialize");

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0171_quality_profile_reset_min_score_to_grab.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0171 should apply");

    let floors: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT id, json_extract(scoring_config, '$.min_score_to_grab')
           FROM quality_profiles ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("score floors should load");
    assert_eq!(
        floors,
        vec![
            ("configured".to_string(), None),
            ("empty".to_string(), None),
            ("explicit-null".to_string(), None),
            ("untouched".to_string(), None),
        ],
        "no profile may keep an old-scale score floor"
    );

    // Everything else in the blob survives untouched.
    let persona: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(scoring_config, '$.scoring_persona')
           FROM quality_profiles WHERE id = 'configured'",
    )
    .fetch_one(&pool)
    .await
    .expect("persona should load");
    assert_eq!(persona.as_deref(), Some("efficient"));
    let cutoff: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(scoring_config, '$.cutoff_tier')
           FROM quality_profiles WHERE id = 'configured'",
    )
    .fetch_one(&pool)
    .await
    .expect("cutoff should load");
    assert_eq!(cutoff.as_deref(), Some("1080P"));

    // Re-applying is a no-op rather than an error.
    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0171_quality_profile_reset_min_score_to_grab.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0171 should be re-runnable");
}

#[tokio::test]
async fn migration_0173_requeues_only_unhydrated_movie_titles_without_tvdb_ids() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database should open");
    sqlx::raw_sql(
        r#"CREATE TABLE titles (
               id TEXT PRIMARY KEY,
               facet TEXT NOT NULL,
               external_ids TEXT NOT NULL,
               metadata_fetched_at TEXT,
               metadata_hydration_next_attempt_at TEXT,
               metadata_hydration_attempt_count INTEGER NOT NULL DEFAULT 0
           );
           INSERT INTO titles VALUES
             ('tmdb-only', 'movie', '[{"source":"tmdb","value":"101"}]', NULL, NULL, 7),
             ('imdb-only', 'movie', '[{"source":"imdb","value":"tt0000102"}]', NULL, NULL, 6),
             ('tvdb-backed', 'movie', '[{"source":"tmdb","value":"103"},{"source":"tvdb","value":"203"}]', NULL, 'preserve', 5),
             ('series-only', 'series', '[{"source":"tmdb","value":"104"}]', NULL, NULL, 4),
             ('already-hydrated', 'movie', '[{"source":"tmdb","value":"105"}]', '2026-01-01T00:00:00Z', NULL, 3);"#,
    )
    .execute(&pool)
    .await
    .expect("title fixture should initialize");

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0177_movie_smg_identity_backfill.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0177 should apply");

    let states: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
           FROM titles ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("hydration states should load");
    assert!(states[1].1.is_some(), "IMDb-only movie should be requeued");
    assert_eq!(states[1].2, 0);
    assert!(states[3].1.is_some(), "TMDB-only movie should be requeued");
    assert_eq!(states[3].2, 0);
    assert_eq!(states[0], ("already-hydrated".to_string(), None, 3));
    assert_eq!(states[2], ("series-only".to_string(), None, 4));
    assert_eq!(
        states[4],
        ("tvdb-backed".to_string(), Some("preserve".to_string()), 5)
    );
}

#[tokio::test]
async fn migration_0200_queues_idle_supported_movies_without_interrupting_active_retries() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database should open");
    sqlx::raw_sql(
        r#"CREATE TABLE titles (
               id TEXT PRIMARY KEY,
               facet TEXT NOT NULL,
               external_ids TEXT NOT NULL,
               metadata_fetched_at TEXT,
               metadata_hydration_next_attempt_at TEXT,
               metadata_hydration_attempt_count INTEGER NOT NULL DEFAULT 0
           );
           INSERT INTO titles VALUES
             ('smg-only', 'movie', '[{"source":"smg","value":"101"}]', NULL, NULL, 0),
             ('tmdb-only', 'movie', '[{"source":"tmdb","value":"102"}]', NULL, NULL, 0),
             ('imdb-only', 'movie', '[{"source":"imdb","value":"tt0000103"}]', NULL, NULL, 0),
             ('tvdb-only', 'movie', '[{"source":"tvdb","value":"104"}]', NULL, NULL, 0),
             ('blank-id', 'movie', '[{"source":"tmdb","value":"  "}]', NULL, NULL, 0),
             ('unsupported-id', 'movie', '[{"source":"wikidata","value":"Q105"}]', NULL, NULL, 0),
             ('series-smg', 'series', '[{"source":"smg","value":"106"}]', NULL, NULL, 0),
             ('already-fetched', 'movie', '[{"source":"smg","value":"107"}]', '2026-01-01T00:00:00Z', NULL, 0),
             ('already-scheduled', 'movie', '[{"source":"tmdb","value":"108"}]', NULL, 'preserve', 0),
             ('already-attempted', 'movie', '[{"source":"imdb","value":"tt0000109"}]', NULL, NULL, 2);"#,
    )
    .execute(&pool)
    .await
    .expect("title fixture should initialize");

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0200_movie_supported_identity_hydration.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0200 should apply");

    let states = sqlx::query_as::<_, (String, Option<String>, i64)>(
        "SELECT id, metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
           FROM titles ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("hydration states should load")
    .into_iter()
    .map(|(id, next_attempt, attempt_count)| (id, (next_attempt, attempt_count)))
    .collect::<std::collections::BTreeMap<_, _>>();

    for id in ["smg-only", "tmdb-only", "imdb-only", "tvdb-only"] {
        assert!(states[id].0.is_some(), "{id} should be queued");
        assert_eq!(states[id].1, 0);
    }
    for id in [
        "blank-id",
        "unsupported-id",
        "series-smg",
        "already-fetched",
        "already-attempted",
    ] {
        assert_eq!(states[id].0, None, "{id} should remain unscheduled");
    }
    assert_eq!(states["already-attempted"].1, 2);
    assert_eq!(
        states["already-scheduled"],
        (Some("preserve".to_string()), 0)
    );
}

#[tokio::test]
async fn migrations_0179_and_0180_backfill_and_finalize_canonical_download_identity() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0179_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pre-0179 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(177), true)
        .await
        .expect("migrations through 0177 should apply");

    let now = "2026-08-24T12:00:00Z";
    for (id, name, client_type) in [
        ("client-one", "NZBGet One", "nzbget"),
        ("client-two", "qBittorrent Two", "qbittorrent"),
    ] {
        sqlx::query(
            "INSERT INTO download_clients (
                id, name, client_type, config_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, '{}', ?4, ?4)",
        )
        .bind(id)
        .bind(name)
        .bind(client_type)
        .bind(now)
        .execute(&pool)
        .await
        .expect("configured client should insert");
    }

    let token_id = "11111111-1111-4111-8111-111111111111";
    let token_row_id = "22222222-2222-4222-8222-222222222222";
    let torrent_id = "33333333-3333-4333-8333-333333333333";
    let stub_id = "44444444-4444-4444-8444-444444444444";
    let deleted_client_id = "55555555-5555-4555-8555-555555555555";
    let empty_config_id = "66666666-6666-4666-8666-666666666666";
    let same_native_one = "77777777-7777-4777-8777-777777777777";
    let same_native_two = "88888888-8888-4888-8888-888888888888";
    let torrent_hash = "0123456789abcdef0123456789abcdef01234567";
    let submissions = [
        (
            token_row_id,
            "title-token",
            "client-one",
            "nzbget",
            Some("native-token"),
            Some(format!("scryer-download:{token_id}")),
        ),
        (
            torrent_id,
            "title-torrent",
            "client-two",
            "qbittorrent",
            Some("torrent-native"),
            Some(torrent_hash.to_string()),
        ),
        (
            stub_id,
            "",
            "client-one",
            "nzbget",
            Some("stub-native"),
            None,
        ),
        (
            deleted_client_id,
            "title-deleted-client",
            "deleted-client",
            "qbittorrent",
            Some("deleted-native"),
            None,
        ),
        (
            empty_config_id,
            "title-empty-config",
            "",
            "sabnzbd",
            Some("empty-config-native"),
            None,
        ),
        (
            same_native_one,
            "title-shared-one",
            "client-one",
            "qbittorrent",
            Some("same-native"),
            None,
        ),
        (
            same_native_two,
            "title-shared-two",
            "client-two",
            "qbittorrent",
            Some("same-native"),
            None,
        ),
        (
            "legacy-row-id",
            "title-legacy-id",
            "client-one",
            "nzbget",
            Some("legacy-native"),
            None,
        ),
    ];
    for (id, title_id, client_id, client_type, item_id, download_id) in submissions {
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id, submitted_at
             ) VALUES (?1, ?2, 'series', ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(id)
        .bind(title_id)
        .bind(client_id)
        .bind(client_type)
        .bind(item_id)
        .bind(download_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("legacy submission should insert");
    }

    sqlx::query(
        "INSERT INTO download_submission_episode_links (
            download_client_id, download_client_type, download_client_item_id, episode_id
         ) VALUES ('client-one', 'nzbget', 'native-token', 'episode-token')",
    )
    .execute(&pool)
    .await
    .expect("tuple link should insert before the rebuild");

    sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, download_id, client_id, client_type,
            download_client_item_id, tracked_state, created_at, updated_at
         ) VALUES
            ('state-token', 'download:' || ?1, ?1, NULL, NULL, NULL, 'queued', ?2, ?2),
            ('state-foreign', 'client:foreign', 'foreign-native', 'foreign-client', 'weaver',
             'foreign-native', 'queued', ?2, ?2)",
    )
    .bind(format!("scryer-download:{token_id}"))
    .bind(now)
    .execute(&pool)
    .await
    .expect("identity states should insert");
    sqlx::query(
        "INSERT INTO imports (
            id, source_system, source_ref, import_type, payload_json, source_client_id,
            created_at, updated_at
         ) VALUES ('import-token', 'nzbget', 'native-token', 'series_download', '{}',
                   'client-one', ?1, ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("download import should insert");
    sqlx::query(
        "INSERT INTO download_import_artifacts (
            id, source_system, source_ref, source_client_id, normalized_file_name, media_kind,
            result, created_at
         ) VALUES ('artifact-token', 'nzbget', 'native-token', 'client-one', 'episode.mkv',
                   'episode', 'imported', ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("download artifact should insert");
    sqlx::query(
        "INSERT INTO download_queue_commands (
            id, action, client_id, client_type, download_client_item_id, status,
            created_at, updated_at
         ) VALUES ('queue-foreign', 'remove', 'foreign-client', 'weaver', 'foreign-native',
                   'queued', ?1, ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("foreign queue command should insert");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0179 and 0180 upgrade should apply");

    let token_submission_id: String = sqlx::query_scalar(
        "SELECT id FROM download_submissions WHERE download_client_item_id = 'native-token'",
    )
    .fetch_one(&pool)
    .await
    .expect("token submission should load");
    assert_eq!(token_submission_id, token_id);
    let torrent_submission_id: String = sqlx::query_scalar(
        "SELECT id FROM download_submissions WHERE download_client_item_id = 'torrent-native'",
    )
    .fetch_one(&pool)
    .await
    .expect("torrent submission should load");
    assert_eq!(
        torrent_submission_id, torrent_id,
        "hashes are never adopted as IDs"
    );
    let legacy_submission_id: String = sqlx::query_scalar(
        "SELECT id FROM download_submissions WHERE download_client_item_id = 'legacy-native'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy-id submission should load");
    assert!(
        legacy_submission_id.len() == 36
            && legacy_submission_id
                .chars()
                .enumerate()
                .all(|(index, character)| {
                    matches!(index, 8 | 13 | 18 | 23) && character == '-'
                        || !matches!(index, 8 | 13 | 18 | 23) && character.is_ascii_hexdigit()
                }),
        "non-UUID legacy IDs should be replaced with a UUID"
    );

    let origins: Vec<(String, String)> = sqlx::query_as(
        "SELECT ds.download_client_item_id, d.origin
           FROM download_submissions ds
           JOIN downloads d ON d.id = ds.id
          ORDER BY ds.download_client_item_id",
    )
    .fetch_all(&pool)
    .await
    .expect("submission origins should load");
    assert!(origins.contains(&("native-token".to_string(), "scryer_submission".to_string())));
    assert!(origins.contains(&("stub-native".to_string(), "foreign_observation".to_string())));
    assert!(origins.contains(&("same-native".to_string(), "scryer_submission".to_string())));

    let empty_binding: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT client_config_id, ended_at
           FROM download_client_bindings
          WHERE download_id = ?1",
    )
    .bind(empty_config_id)
    .fetch_one(&pool)
    .await
    .expect("empty-config binding should load");
    assert_eq!(empty_binding.0, None);
    assert!(empty_binding.1.is_some());
    let deleted_snapshot: String = sqlx::query_scalar(
        "SELECT client_name_snapshot FROM download_client_bindings WHERE download_id = ?1",
    )
    .bind(deleted_client_id)
    .fetch_one(&pool)
    .await
    .expect("deleted-client snapshot should load");
    assert_eq!(deleted_snapshot, "qbittorrent");

    let shared_bindings: Vec<String> = sqlx::query_scalar(
        "SELECT download_id FROM download_client_bindings
          WHERE native_item_id = 'same-native' ORDER BY download_id",
    )
    .fetch_all(&pool)
    .await
    .expect("same-native bindings should load");
    assert_eq!(shared_bindings.len(), 2);
    assert_ne!(shared_bindings[0], shared_bindings[1]);

    let global_state_canonical: String = sqlx::query_scalar(
        "SELECT canonical_download_id FROM download_identity_states WHERE id = 'state-token'",
    )
    .fetch_one(&pool)
    .await
    .expect("global token state should load");
    assert_eq!(global_state_canonical, token_id);
    let foreign_state_canonical: String = sqlx::query_scalar(
        "SELECT canonical_download_id FROM download_identity_states WHERE id = 'state-foreign'",
    )
    .fetch_one(&pool)
    .await
    .expect("foreign state should load");
    let foreign_queue_canonical: String = sqlx::query_scalar(
        "SELECT canonical_download_id FROM download_queue_commands WHERE id = 'queue-foreign'",
    )
    .fetch_one(&pool)
    .await
    .expect("foreign queue command should load");
    assert_eq!(foreign_state_canonical, foreign_queue_canonical);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT canonical_download_id FROM imports WHERE id = 'import-token'",
        )
        .fetch_one(&pool)
        .await
        .expect("import backfill should load"),
        token_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT canonical_download_id FROM download_import_artifacts WHERE id = 'artifact-token'",
        )
        .fetch_one(&pool)
        .await
        .expect("artifact backfill should load"),
        token_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM download_submission_episode_links
              WHERE download_id = ?1",
        )
        .bind(token_id)
        .fetch_one(&pool)
        .await
        .expect("tuple link should survive the rebuild"),
        1
    );

    let mut transaction = pool
        .begin()
        .await
        .expect("idempotency transaction should begin");
    let changes_before: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(&mut *transaction)
        .await
        .expect("sqlite changes should load");
    crate::migrations::canonical_download_identity::backfill_canonical_download_identity_sqlite(
        &mut transaction,
    )
    .await
    .expect("completed 0179 hook should be idempotent");
    let changes_after: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(&mut *transaction)
        .await
        .expect("sqlite changes should load after idempotency check");
    assert_eq!(
        changes_after, changes_before,
        "a second hook run must not write"
    );
    transaction
        .commit()
        .await
        .expect("idempotency transaction should commit");

    for id in ["null-one", "null-two"] {
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES (?1, 'scryer_submission', ?2)",
        )
        .bind(id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("canonical parent should insert");
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id
             ) VALUES (?1, 'title-null', 'series', 'client-one', 'nzbget', NULL)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("nullable tuple snapshots should permit distinct rows");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM download_submissions
              WHERE download_client_id = 'client-one'
                AND download_client_type = 'nzbget'
                AND download_client_item_id IS NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("NULL-distinct rows should count"),
        2
    );
    for id in ["readd-first", "readd-second"] {
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES (?1, 'scryer_submission', ?2)",
        )
        .bind(id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("re-add canonical parent should insert");
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id
             ) VALUES (?1, 'title-readd', 'series', 'client-one', 'nzbget', 'readd-native')",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("reused native tuple should coexist after 0180");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM download_submissions
              WHERE download_client_id = 'client-one'
                AND download_client_type = 'nzbget'
                AND download_client_item_id = 'readd-native'",
        )
        .fetch_one(&pool)
        .await
        .expect("coexisting re-add rows should count"),
        2
    );

    sqlx::query(
        "INSERT INTO downloads (id, origin, created_at)
         VALUES ('active-binding-first', 'scryer_submission', ?1),
                ('active-binding-second', 'scryer_submission', ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("active-binding parents should insert");
    sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES ('active-binding-first', 'client-one', 'qbittorrent', 'qBittorrent One',
                   'readd-active-native', ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("first active binding should insert");
    let active_collision = sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES ('active-binding-second', 'client-one', 'qbittorrent', 'qBittorrent One',
                   'readd-active-native', ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        active_collision.is_err(),
        "active locator index must reject a collision"
    );

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT [notnull] FROM pragma_table_info('download_identity_states')
              WHERE name = 'canonical_download_id'",
        )
        .fetch_one(&pool)
        .await
        .expect("identity-state nullability should load"),
        1
    );
    let null_canonical = sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state, created_at, updated_at
         ) VALUES ('state-null-canonical', 'download:state-null-canonical', NULL,
                   'legacy-null-canonical', 'queued', ?1, ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        null_canonical.is_err(),
        "canonical download id must be required"
    );

    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("foreign-key check should run");
    assert!(
        foreign_key_violations.is_empty(),
        "0180 foreign keys must validate existing rows"
    );

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0180 rerun should be idempotently skipped by the manifest ledger");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 179 AND success = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("0180 migration ledger entry should load"),
        1
    );

    let fresh_db = std::env::temp_dir().join(format!(
        "scryer_migration_0180_fresh_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let fresh_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(fresh_db.to_string_lossy().as_ref()))
        .await
        .expect("fresh 0180 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&fresh_pool, None, true)
        .await
        .expect("fresh install through 0180 should apply");
    let schema_query = "SELECT type, name, sql FROM sqlite_master
                         WHERE type IN ('table', 'index')
                           AND tbl_name IN (
                               'downloads', 'download_client_bindings', 'download_submissions',
                               'download_submission_episode_links', 'download_identity_states',
                               'imports', 'download_import_artifacts', 'download_queue_commands'
                           )
                         ORDER BY type, name";
    let upgraded_schema: Vec<(String, String, Option<String>)> = sqlx::query_as(schema_query)
        .fetch_all(&pool)
        .await
        .expect("upgraded 0180 schema and indexes should load");
    let fresh_schema: Vec<(String, String, Option<String>)> = sqlx::query_as(schema_query)
        .fetch_all(&fresh_pool)
        .await
        .expect("fresh 0180 schema and indexes should load");
    assert_eq!(fresh_schema, upgraded_schema);

    drop(fresh_pool);
    drop(pool);
    let _ = std::fs::remove_file(fresh_db);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0180_rekeys_a_populated_0179_database_and_validates_constraints() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0180_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("0179 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(179), true)
        .await
        .expect("fresh 0179 fixture should apply");

    let now = "2026-08-24T12:00:00Z";
    let first_id = "00000000-0000-4000-8000-000000000001";
    sqlx::query(
        "INSERT INTO downloads (id, origin, created_at)
         VALUES (?1, 'scryer_submission', ?2)",
    )
    .bind(first_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("post-0179 canonical parent should insert");
    sqlx::query(
        "INSERT INTO download_submissions (
            id, title_id, facet, download_client_id, download_client_type,
            download_client_item_id, submitted_at
         ) VALUES (?1, 'title-0180', 'series', 'client-0180', 'qbittorrent',
                   'reused-native', ?2)",
    )
    .bind(first_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("post-0179 submission should insert");
    sqlx::query(
        "INSERT INTO download_submission_episode_links (
            download_client_id, download_client_type, download_client_item_id, episode_id
         ) VALUES ('client-0180', 'qbittorrent', 'reused-native', 'episode-0180')",
    )
    .execute(&pool)
    .await
    .expect("post-0179 tuple link should insert");
    sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES (?1, 'client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ?2)",
    )
    .bind(first_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("post-0179 active binding should insert");
    sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, client_id, client_type,
            download_client_item_id, tracked_state, created_at, updated_at
         ) VALUES ('state-0180', 'download:0180', ?1, 'legacy-0180', 'client-0180',
                   'qbittorrent', 'reused-native', 'queued', ?2, ?2)",
    )
    .bind(first_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("post-0179 identity state should insert");
    sqlx::query(
        "INSERT INTO imports (
            id, source_system, source_ref, import_type, payload_json, created_at, updated_at,
            canonical_download_id
         ) VALUES ('import-0180', 'qbittorrent', 'reused-native', 'series_download', '{}',
                   ?1, ?1, ?2)",
    )
    .bind(now)
    .bind(first_id)
    .execute(&pool)
    .await
    .expect("post-0179 import should insert");
    sqlx::query(
        "INSERT INTO download_import_artifacts (
            id, source_system, source_ref, normalized_file_name, media_kind, result, created_at,
            canonical_download_id
         ) VALUES ('artifact-0180', 'qbittorrent', 'reused-native', 'episode.mkv', 'episode',
                   'imported', ?1, ?2)",
    )
    .bind(now)
    .bind(first_id)
    .execute(&pool)
    .await
    .expect("post-0179 artifact should insert");
    sqlx::query(
        "INSERT INTO download_queue_commands (
            id, action, client_type, download_client_item_id, status, created_at, updated_at,
            canonical_download_id
         ) VALUES ('queue-0180', 'remove', 'qbittorrent', 'reused-native', 'queued', ?1, ?1,
                   ?2)",
    )
    .bind(now)
    .bind(first_id)
    .execute(&pool)
    .await
    .expect("post-0179 queue command should insert");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0180 upgrade should apply");

    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT download_id FROM download_submission_episode_links
              WHERE episode_id = 'episode-0180'",
        )
        .fetch_one(&pool)
        .await
        .expect("re-keyed link should load"),
        first_id
    );
    let link_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('download_submission_episode_links') ORDER BY cid",
    )
    .fetch_all(&pool)
    .await
    .expect("re-keyed link columns should load");
    assert_eq!(link_columns, vec!["download_id", "episode_id"]);

    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("foreign-key check should run");
    assert!(
        foreign_key_violations.is_empty(),
        "0180 foreign keys must validate fixture rows"
    );

    let second_id = "00000000-0000-4000-8000-000000000002";
    let third_id = "00000000-0000-4000-8000-000000000003";
    sqlx::query(
        "INSERT INTO downloads (id, origin, created_at)
         VALUES (?1, 'scryer_submission', ?3), (?2, 'scryer_submission', ?3)",
    )
    .bind(second_id)
    .bind(third_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("re-add canonical parents should insert");
    let active_collision = sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES (?1, 'client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ?2)",
    )
    .bind(second_id)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        active_collision.is_err(),
        "partial active-binding index should reject a second row"
    );
    sqlx::query("UPDATE download_client_bindings SET ended_at = ?1 WHERE download_id = ?2")
        .bind(now)
        .bind(first_id)
        .execute(&pool)
        .await
        .expect("completed delete should end the first binding");
    sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES (?1, 'client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ?2)",
    )
    .bind(second_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("ended binding should admit the re-added locator");
    sqlx::query(
        "INSERT INTO download_submissions (
            id, title_id, facet, download_client_id, download_client_type,
            download_client_item_id, submitted_at
         ) VALUES (?1, 'title-0180-readd', 'series', 'client-0180', 'qbittorrent',
                   'reused-native', '2026-08-24T12:00:01Z')",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("re-added submission should coexist with the ended one");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM download_submissions
              WHERE download_client_item_id = 'reused-native'",
        )
        .fetch_one(&pool)
        .await
        .expect("re-added submissions should count"),
        2
    );
    let second_active_collision = sqlx::query(
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at
         ) VALUES (?1, 'client-0180', 'qbittorrent', 'qBittorrent', 'reused-native', ?2)",
    )
    .bind(third_id)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        second_active_collision.is_err(),
        "new active binding must remain unique"
    );

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0180 rerun should be skipped by the manifest ledger");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 179 AND success = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("0180 migration ledger should load"),
        1
    );

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0179_rejects_duplicate_adopted_token_ids_without_partial_writes() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0179_collision_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pre-0179 collision database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(177), true)
        .await
        .expect("migrations through 0177 should apply");
    let token_id = "99999999-9999-4999-8999-999999999999";
    for (id, item_id) in [
        ("collision-row-one", "collision-one"),
        ("collision-row-two", "collision-two"),
    ] {
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id
             ) VALUES (?1, 'title-collision', 'series', 'client-collision', 'nzbget', ?2, ?3)",
        )
        .bind(id)
        .bind(item_id)
        .bind(format!("scryer-download:{token_id}"))
        .execute(&pool)
        .await
        .expect("collision fixture should insert");
    }

    let error = crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect_err("duplicate adopted token IDs must abort 0179");
    let message = error.to_string();
    assert!(message.contains("collision-row-one"));
    assert!(message.contains("collision-row-two"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'downloads'",
        )
        .fetch_one(&pool)
        .await
        .expect("rollback schema check should load"),
        0,
        "the DDL and hook writes must roll back together"
    );
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM download_submissions WHERE id LIKE 'collision-%' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("original collision rows should survive rollback");
    assert_eq!(ids, vec!["collision-row-one", "collision-row-two"]);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0179_postgres_backfills_token_identity_from_env() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_0179_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(177)).await?;
        sqlx::query(
            "INSERT INTO download_clients (
                id, name, client_type, config_json, created_at, updated_at
             ) VALUES ('pg-client', 'Postgres NZBGet', 'nzbget', '{}', now(), now())",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let token_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id
             ) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'pg-title', 'series',
                       'pg-client', 'nzbget', 'pg-item', $1)",
        )
        .bind(format!("scryer-download:{token_id}"))
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let services = crate::PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);

        let submission_id: String = sqlx::query_scalar(
            "SELECT id FROM download_submissions WHERE download_client_item_id = 'pg-item'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(submission_id, token_id);
        let binding_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_client_bindings WHERE download_id = $1",
        )
        .bind(token_id)
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(binding_count, 1);
        Ok(())
    }
    .await;

    drop(pool);
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    drop(admin_pool);
    cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop postgres schema: {error}"))
    })?;
    result
}

#[tokio::test]
async fn migration_0186_admits_token_less_identity_states_without_disturbing_existing_rows() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0186_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("0183 database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(185), true)
        .await
        .expect("fresh 0183 fixture should apply");

    let now = "2026-08-25T12:00:00Z";
    let token_download = "00000000-0000-4000-8000-000000000101";
    let plugin_download = "00000000-0000-4000-8000-000000000102";
    sqlx::query(
        "INSERT INTO downloads (id, origin, created_at)
         VALUES (?1, 'scryer_submission', ?3), (?2, 'scryer_submission', ?3)",
    )
    .bind(token_download)
    .bind(plugin_download)
    .bind(now)
    .execute(&pool)
    .await
    .expect("canonical parents should insert");

    // A token-bearing row written by the shipped code, with the identity_key
    // exactly as the store produces it today.
    let token_identity_key = format!("download:{token_download}");
    sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, client_id, client_type,
            download_client_item_id, tracked_state, reason, detail, created_at, updated_at
         ) VALUES ('state-token-0186', ?1, ?2, 'legacy-token-0186', 'client-0186', 'nzbget',
                   'native-0186', 'imported', 'reason-0186', 'detail-0186', ?3, ?3)",
    )
    .bind(&token_identity_key)
    .bind(token_download)
    .bind(now)
    .execute(&pool)
    .await
    .expect("token-bearing identity state should insert");

    // Before 0186 a plugin client that omits the wire token cannot record any
    // durable state at all: the legacy CHECK rejects the row.
    let pre_upgrade_token_less = sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state,
            created_at, updated_at
         ) VALUES ('state-plugin-0186', ?1, ?2, NULL, 'imported', ?3, ?3)",
    )
    .bind(format!("download:{plugin_download}"))
    .bind(plugin_download)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        pre_upgrade_token_less.is_err(),
        "the pre-0186 schema must still require the legacy wire token"
    );

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0186 upgrade should apply");

    // Restart continuity: the existing row keeps its identity_key byte for byte
    // and every other column with it.
    let (
        preserved_key,
        preserved_canonical,
        preserved_download_id,
        preserved_state,
        preserved_reason,
        preserved_detail,
    ): (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT identity_key, canonical_download_id, download_id, tracked_state, reason, detail
           FROM download_identity_states WHERE id = 'state-token-0186'",
    )
    .fetch_one(&pool)
    .await
    .expect("token-bearing row should survive the rebuild");
    assert_eq!(preserved_key, token_identity_key);
    assert_eq!(preserved_canonical, token_download);
    assert_eq!(preserved_download_id.as_deref(), Some("legacy-token-0186"));
    assert_eq!(preserved_state, "imported");
    assert_eq!(preserved_reason.as_deref(), Some("reason-0186"));
    assert_eq!(preserved_detail.as_deref(), Some("detail-0186"));

    // The token-less row the plugin path needs now inserts.
    sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state,
            created_at, updated_at
         ) VALUES ('state-plugin-0186', ?1, ?2, NULL, 'imported', ?3, ?3)",
    )
    .bind(format!("download:{plugin_download}"))
    .bind(plugin_download)
    .bind(now)
    .execute(&pool)
    .await
    .expect("0186 must admit a token-less canonical identity state");

    // canonical_download_id stays mandatory.
    let null_canonical = sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state,
            created_at, updated_at
         ) VALUES ('state-null-0186', 'download:null-0186', NULL, 'legacy', 'imported', ?1, ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        null_canonical.is_err(),
        "canonical download id must remain non-null"
    );

    // identity_key uniqueness still holds for the canonical-derived keys.
    let duplicate_key = sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state,
            created_at, updated_at
         ) VALUES ('state-duplicate-0186', ?1, ?2, NULL, 'failed', ?3, ?3)",
    )
    .bind(format!("download:{plugin_download}"))
    .bind(plugin_download)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        duplicate_key.is_err(),
        "identity_key must stay unique for token-less rows"
    );

    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("foreign-key check should run");
    assert!(
        foreign_key_violations.is_empty(),
        "0186 foreign keys must validate the fixture rows"
    );
    let orphan_canonical = sqlx::query(
        "INSERT INTO download_identity_states (
            id, identity_key, canonical_download_id, download_id, tracked_state,
            created_at, updated_at
         ) VALUES ('state-orphan-0186', 'download:orphan-0186', 'missing-download', NULL,
                   'imported', ?1, ?1)",
    )
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        orphan_canonical.is_err(),
        "the new canonical foreign key must reject an unknown downloads(id)"
    );

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0186 rerun should be skipped by the manifest ledger");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 186 AND success = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("0186 migration ledger should load"),
        1
    );

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0201_compacts_discovery_payloads_and_rekeys_recommendation_cards() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0201_discovery_storage_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("migration test database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(200), true)
        .await
        .expect("0200 fixture schema should install");

    sqlx::query(
        "INSERT INTO discovery_sync_runs (
            id, kind, status, trigger_source, region, language,
            raw_ack_json, created_at, updated_at
         ) VALUES (
            'acked-run', 'context_snapshot', 'complete', 'test', 'US', 'eng',
            '{\"status\":\"ACKNOWLEDGED\"}',
            '2026-08-01T00:00:00Z', '2026-08-02T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("legacy sync run should insert");
    sqlx::query(
        "INSERT INTO titles (
            id, library_id, name, name_normalized, facet, root_folder_id, created_at
         ) VALUES (
            'source-title', 'movie_default_library', 'Source', 'source', 'movie',
            'canonical_root_for_movie_default_library', '2026-08-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("source title should insert");
    sqlx::query(
        "INSERT INTO discovery_titles (
            id, target_key, target_key_norm, language, target_kind,
            display_title, created_at, updated_at
         ) VALUES (
            'legacy-card', 'tmdb:movie:10', 'tmdb:movie:10', 'eng', 'movie',
            'Legacy Card', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("legacy discovery title should insert");
    sqlx::query(
        "INSERT INTO title_more_like_this_items (
            source_title_id, discovery_title_id, sort_index, created_at, updated_at
         ) VALUES (
            'source-title', 'legacy-card', 0,
            '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("legacy recommendation edge should insert");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0201 should apply");

    let acknowledged_at: Option<String> = sqlx::query_scalar(
        "SELECT acknowledged_at FROM discovery_sync_runs WHERE id = 'acked-run'",
    )
    .fetch_one(&pool)
    .await
    .expect("acknowledgement timestamp should load");
    assert_eq!(acknowledged_at.as_deref(), Some("2026-08-02T00:00:00Z"));

    let columns = sqlx::query("PRAGMA table_info(discovery_sync_runs)")
        .fetch_all(&pool)
        .await
        .expect("sync-run columns should load")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<HashSet<_>>();
    assert!(columns.contains("acknowledged_at"));
    for removed in [
        "raw_submit_json",
        "raw_changes_json",
        "raw_final_status_json",
        "raw_ack_json",
    ] {
        assert!(!columns.contains(removed), "{removed} should be removed");
    }

    let card: (i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT payload_version, payload_blob
         FROM title_recommendation_cards
         WHERE discovery_title_id = 'legacy-card'",
    )
    .fetch_one(&pool)
    .await
    .expect("recommendation card placeholder should load");
    assert_eq!(card, (1, None));
    let recommendation_fk: String = sqlx::query_scalar(
        "SELECT \"table\"
         FROM pragma_foreign_key_list('title_more_like_this_items')
         WHERE \"from\" = 'discovery_title_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("recommendation foreign key should load");
    assert_eq!(recommendation_fk, "title_recommendation_cards");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("integrity check should run");
    assert_eq!(integrity, "ok");

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0201_postgres_uses_native_payload_and_timestamp_types() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_0201_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(200)).await?;
        sqlx::query(
            "INSERT INTO discovery_sync_runs (
                id, kind, status, trigger_source, region, language,
                raw_ack_json, created_at, updated_at
             ) VALUES (
                'acked-run', 'context_snapshot', 'complete', 'test', 'US', 'eng',
                '{\"status\":\"ACKNOWLEDGED\"}'::jsonb,
                '2026-08-01T00:00:00Z'::timestamptz,
                '2026-08-02T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO titles (
                id, library_id, name, name_normalized, facet, root_folder_id, created_at
             ) VALUES (
                'source-title', 'movie_default_library', 'Source', 'source', 'movie',
                'canonical_root_for_movie_default_library',
                '2026-08-01T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO discovery_titles (
                id, target_key, target_key_norm, language, target_kind,
                display_title, created_at, updated_at
             ) VALUES (
                'legacy-card', 'tmdb:movie:10', 'tmdb:movie:10', 'eng', 'movie',
                'Legacy Card', '2026-08-01T00:00:00Z'::timestamptz,
                '2026-08-01T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO title_more_like_this_items (
                source_title_id, discovery_title_id, sort_index, created_at, updated_at
             ) VALUES (
                'source-title', 'legacy-card', 0,
                '2026-08-01T00:00:00Z'::timestamptz,
                '2026-08-01T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let services = crate::PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);

        let acknowledged_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT acknowledged_at FROM discovery_sync_runs WHERE id = 'acked-run'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(acknowledged_at.to_rfc3339(), "2026-08-02T00:00:00+00:00");

        let card: (i32, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT payload_version, payload_blob
             FROM title_recommendation_cards
             WHERE discovery_title_id = 'legacy-card'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(card, (1, None));

        let column_types: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name, data_type
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name IN ('title_recommendation_cards', 'discovery_sync_runs')
               AND column_name IN ('payload_blob', 'acknowledged_at')
             ORDER BY column_name",
        )
        .fetch_all(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(
            column_types,
            vec![
                (
                    "acknowledged_at".to_string(),
                    "timestamp with time zone".to_string()
                ),
                ("payload_blob".to_string(), "bytea".to_string()),
            ]
        );

        let raw_columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'discovery_sync_runs'
               AND column_name IN (
                   'raw_submit_json', 'raw_changes_json',
                   'raw_final_status_json', 'raw_ack_json'
               )",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(raw_columns, 0);

        let recommendation_fk: String = sqlx::query_scalar(
            "SELECT ccu.table_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.constraint_column_usage ccu
               ON ccu.constraint_schema = tc.constraint_schema
              AND ccu.constraint_name = tc.constraint_name
             WHERE tc.table_schema = current_schema()
               AND tc.table_name = 'title_more_like_this_items'
               AND tc.constraint_type = 'FOREIGN KEY'
               AND ccu.column_name = 'discovery_title_id'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(recommendation_fk, "title_recommendation_cards");
        Ok(())
    }
    .await;

    drop(pool);
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    drop(admin_pool);
    cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop postgres schema: {error}"))
    })?;
    result
}

#[tokio::test]
async fn migrations_0202_and_0203_bind_factor_state_and_session_epochs() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0202_0203_factor_state_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("migration test database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(201), true)
        .await
        .expect("0201 fixture schema should install");

    sqlx::query(
        "INSERT INTO totp_credentials (
            id, user_id, secret_base32, algorithm, digits, period_seconds,
            created_at, updated_at
         ) VALUES (
            'credential', '00000000000000000000000000000001', 'JBSWY3DPEHPK3PXP',
            'SHA1', 6, 30, '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("legacy TOTP credential should insert");
    for (id, expires_at) in [
        ("expired-challenge", "2000-01-01T00:00:00Z"),
        ("stale-challenge", "2999-01-01T00:00:00Z"),
        ("newest-challenge", "2999-06-01T00:00:00Z"),
    ] {
        sqlx::query(
            "INSERT INTO totp_enrollment_challenges (
                id, user_id, secret_base32, algorithm, digits, period_seconds,
                created_at, expires_at
             ) VALUES (
                $1, '00000000000000000000000000000001', 'JBSWY3DPEHPK3PXP',
                'SHA1', 6, 30, '2026-08-01T00:00:00Z', $2
             )",
        )
        .bind(id)
        .bind(expires_at)
        .execute(&pool)
        .await
        .expect("legacy enrollment challenge should insert");
    }
    sqlx::query(
        "INSERT INTO oauth_authorization_codes (
            id, code_hash, client_id, user_id, redirect_uri, scope,
            code_challenge, code_challenge_method, created_at, expires_at
         ) VALUES (
            'stale-code', 'stale-hash', 'client', '00000000000000000000000000000001',
            'https://client.test/callback', 'profile', 'challenge', 'S256',
            '2026-08-01T00:00:00Z', '2999-01-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("pre-epoch authorization code should insert");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0202 and 0203 should apply");

    let credential: (Option<String>, i64) = sqlx::query_as(
        "SELECT attempt_window_started_at, attempt_count
         FROM totp_credentials WHERE id = 'credential'",
    )
    .fetch_one(&pool)
    .await
    .expect("credential attempt state should load");
    assert_eq!(credential, (None, 0));

    let challenge_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM totp_enrollment_challenges")
            .fetch_all(&pool)
            .await
            .expect("surviving enrollment challenges should load");
    assert_eq!(challenge_ids, vec!["newest-challenge".to_string()]);
    let unique_index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM pragma_index_list('totp_enrollment_challenges')
         WHERE name = 'totp_enrollment_challenges_one_active_per_user'
           AND \"unique\" = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("one-active-per-user index should load");
    assert_eq!(unique_index, 1);

    for table in ["totp_enrollment_challenges", "webauthn_challenges"] {
        let has_session_version: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info($1)
             WHERE name = 'auth_session_version'",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .expect("challenge session-version column should load");
        assert_eq!(has_session_version, 1, "{table} should carry the epoch");
    }

    let remaining_codes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oauth_authorization_codes")
        .fetch_one(&pool)
        .await
        .expect("authorization code count should load");
    assert_eq!(remaining_codes, 0);
    sqlx::query(
        "INSERT INTO oauth_authorization_codes (
            id, code_hash, client_id, user_id, redirect_uri, scope,
            code_challenge, code_challenge_method, created_at, expires_at
         ) VALUES (
            'epochless-code', 'epochless-hash', 'client',
            '00000000000000000000000000000001', 'https://client.test/callback',
            'profile', 'challenge', 'S256',
            '2026-08-01T00:00:00Z', '2999-01-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("epoch column default should admit legacy-shaped inserts");
    let default_epoch: String = sqlx::query_scalar(
        "SELECT auth_session_version FROM oauth_authorization_codes
         WHERE id = 'epochless-code'",
    )
    .fetch_one(&pool)
    .await
    .expect("defaulted epoch should load");
    assert_eq!(default_epoch, "");

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("integrity check should run");
    assert_eq!(integrity, "ok");

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_0202_and_0203_postgres_bind_factor_state_and_session_epochs() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_0202_0203_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(201)).await?;
        for (id, expires_at) in [
            ("expired-challenge", "2000-01-01T00:00:00Z"),
            ("stale-challenge", "2999-01-01T00:00:00Z"),
            ("newest-challenge", "2999-06-01T00:00:00Z"),
        ] {
            sqlx::query(
                "INSERT INTO totp_enrollment_challenges (
                    id, user_id, secret_base32, algorithm, digits, period_seconds,
                    created_at, expires_at
                 ) VALUES (
                    $1, '00000000000000000000000000000001', 'JBSWY3DPEHPK3PXP',
                    'SHA1', 6, 30, '2026-08-01T00:00:00Z'::timestamptz,
                    $2::timestamptz
                 )",
            )
            .bind(id)
            .bind(expires_at)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        sqlx::query(
            "INSERT INTO oauth_authorization_codes (
                id, code_hash, client_id, user_id, redirect_uri, scope,
                code_challenge, code_challenge_method, created_at, expires_at
             ) VALUES (
                'stale-code', 'stale-hash', 'client',
                '00000000000000000000000000000001', 'https://client.test/callback',
                'profile', 'challenge', 'S256',
                '2026-08-01T00:00:00Z'::timestamptz, '2999-01-01T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let services = crate::PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);

        let challenge_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM totp_enrollment_challenges")
                .fetch_all(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(challenge_ids, vec!["newest-challenge".to_string()]);
        let unique_index: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM pg_indexes
             WHERE schemaname = current_schema()
               AND tablename = 'totp_enrollment_challenges'
               AND indexname = 'totp_enrollment_challenges_one_active_per_user'
               AND indexdef LIKE 'CREATE UNIQUE INDEX%'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(unique_index, 1);

        let epoch_column: (String, Option<String>) = sqlx::query_as(
            "SELECT is_nullable, column_default
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'oauth_authorization_codes'
               AND column_name = 'auth_session_version'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(epoch_column.0, "NO");
        assert_eq!(epoch_column.1.as_deref(), Some("''::text"));

        let remaining_codes: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM oauth_authorization_codes")
                .fetch_one(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(remaining_codes, 0);
        sqlx::query(
            "INSERT INTO oauth_authorization_codes (
                id, code_hash, client_id, user_id, redirect_uri, scope,
                code_challenge, code_challenge_method, created_at, expires_at
             ) VALUES (
                'epochless-code', 'epochless-hash', 'client',
                '00000000000000000000000000000001', 'https://client.test/callback',
                'profile', 'challenge', 'S256',
                '2026-08-01T00:00:00Z'::timestamptz, '2999-01-01T00:00:00Z'::timestamptz
             )",
        )
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let default_epoch: String = sqlx::query_scalar(
            "SELECT auth_session_version FROM oauth_authorization_codes
             WHERE id = 'epochless-code'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(default_epoch, "");

        let attempt_state_columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'totp_credentials'
               AND column_name IN ('attempt_window_started_at', 'attempt_count')",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(attempt_state_columns, 2);
        let challenge_epoch_columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name IN ('totp_enrollment_challenges', 'webauthn_challenges')
               AND column_name = 'auth_session_version'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(challenge_epoch_columns, 2);
        Ok(())
    }
    .await;

    drop(pool);
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    drop(admin_pool);
    cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop postgres schema: {error}"))
    })?;
    result
}

#[tokio::test]
async fn migration_0186_postgres_relaxes_the_token_check_and_adds_the_canonical_foreign_key()
-> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_0186_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(185)).await?;
        let now = "2026-08-25T12:00:00Z";
        let token_download = "00000000-0000-4000-8000-000000000101";
        let plugin_download = "00000000-0000-4000-8000-000000000102";
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES ($1, 'scryer_submission', ($3::text)::timestamptz),
                    ($2, 'scryer_submission', ($3::text)::timestamptz)",
        )
        .bind(token_download)
        .bind(plugin_download)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let token_identity_key = format!("download:{token_download}");
        sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state,
                created_at, updated_at
             ) VALUES ('pg-state-token-0186', $1, $2, 'legacy-token-0186', 'imported',
                       ($3::text)::timestamptz, ($3::text)::timestamptz)",
        )
        .bind(&token_identity_key)
        .bind(token_download)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let pre_upgrade_token_less = sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state,
                created_at, updated_at
             ) VALUES ('pg-state-plugin-0186', $1, $2, NULL, 'imported',
                       ($3::text)::timestamptz, ($3::text)::timestamptz)",
        )
        .bind(format!("download:{plugin_download}"))
        .bind(plugin_download)
        .bind(now)
        .execute(&pool)
        .await;
        assert!(
            pre_upgrade_token_less.is_err(),
            "the pre-0186 schema must still require the legacy wire token"
        );

        let services = crate::PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        drop(services);

        let preserved_key: String = sqlx::query_scalar(
            "SELECT identity_key FROM download_identity_states WHERE id = 'pg-state-token-0186'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(preserved_key, token_identity_key);

        sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state,
                created_at, updated_at
             ) VALUES ('pg-state-plugin-0186', $1, $2, NULL, 'imported',
                       ($3::text)::timestamptz, ($3::text)::timestamptz)",
        )
        .bind(format!("download:{plugin_download}"))
        .bind(plugin_download)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let null_canonical = sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state,
                created_at, updated_at
             ) VALUES ('pg-state-null-0186', 'download:pg-null-0186', NULL, 'legacy', 'imported',
                       ($1::text)::timestamptz, ($1::text)::timestamptz)",
        )
        .bind(now)
        .execute(&pool)
        .await;
        assert!(
            null_canonical.is_err(),
            "canonical download id must remain non-null"
        );

        let orphan_canonical = sqlx::query(
            "INSERT INTO download_identity_states (
                id, identity_key, canonical_download_id, download_id, tracked_state,
                created_at, updated_at
             ) VALUES ('pg-state-orphan-0186', 'download:pg-orphan-0186', 'missing-download', NULL,
                       'imported', ($1::text)::timestamptz, ($1::text)::timestamptz)",
        )
        .bind(now)
        .execute(&pool)
        .await;
        assert!(
            orphan_canonical.is_err(),
            "the new canonical foreign key must reject an unknown downloads(id)"
        );
        Ok(())
    }
    .await;

    drop(pool);
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    drop(admin_pool);
    cleanup.map_err(|error| {
        AppError::Repository(format!("failed to drop postgres schema: {error}"))
    })?;
    result
}

#[tokio::test]
async fn migration_0208_relaxes_the_proxy_protocol_and_preserves_solver_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database should open");
    // The pre-0208 shape, verbatim from the 0198 baseline.
    sqlx::raw_sql(
        "CREATE TABLE indexer_proxy_configs (
             id TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             provider_type TEXT NOT NULL,
             protocol TEXT NOT NULL,
             base_url TEXT NOT NULL,
             request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
             is_enabled INTEGER NOT NULL DEFAULT 1,
             last_health_status TEXT,
             last_error_message TEXT,
             last_error_at TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX idx_indexer_proxy_configs_provider_type
             ON indexer_proxy_configs(provider_type);
         INSERT INTO indexer_proxy_configs (
             id, name, provider_type, protocol, base_url, request_timeout_seconds,
             is_enabled, last_health_status, last_error_message, last_error_at,
             created_at, updated_at
         ) VALUES (
             'solver-1', 'Trawl', 'trawl', 'request_solution_v1', 'http://trawl:8191',
             45, 1, 'healthy', NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z'
         );",
    )
    .execute(&pool)
    .await
    .expect("initialize pre-0208 schema");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0208_indexer_transport_proxies.sql"),
    )
    .await;

    let solver: (
        String,
        Option<String>,
        i64,
        i64,
        Option<String>,
        Option<String>,
        i64,
    ) = sqlx::query_as(
        "SELECT provider_type, protocol, request_timeout_seconds, is_enabled,
                    username_encrypted, password_encrypted, remote_dns
               FROM indexer_proxy_configs
              WHERE id = 'solver-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("the existing solver row must survive the rebuild");
    assert_eq!(
        solver,
        (
            "trawl".into(),
            Some("request_solution_v1".into()),
            45,
            1,
            None,
            None,
            0,
        )
    );

    // The point of the rebuild: a transport row with no protocol is now legal.
    sqlx::query(
        "INSERT INTO indexer_proxy_configs (
             id, name, provider_type, protocol, base_url, request_timeout_seconds,
             is_enabled, username_encrypted, password_encrypted, remote_dns,
             created_at, updated_at
         ) VALUES (
             'socks-1', 'Gateway', 'socks5', NULL, 'socks5://gateway:1080', 30,
             1, 'enc:user', 'enc:pass', 1, '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z'
         )",
    )
    .execute(&pool)
    .await
    .expect("a transport proxy stores no solver protocol");

    let index_present: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
          WHERE type = 'index' AND name = 'idx_indexer_proxy_configs_provider_type'",
    )
    .fetch_one(&pool)
    .await
    .expect("read index catalog");
    assert_eq!(
        index_present, 1,
        "the rebuild must recreate the provider_type index it dropped"
    );
}
