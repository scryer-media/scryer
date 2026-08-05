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
async fn migration_0147_postgres_retires_w500_and_adds_proxy_tables_from_env() -> AppResult<()> {
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
        "scryer_w500_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres test schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres test schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(146)).await?;
        let library_id =
            scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
        let root_folder_id: String = sqlx::query_scalar(
            "SELECT id FROM library_roots
              WHERE library_id = $1
              ORDER BY is_default DESC, path
              LIMIT 1",
        )
        .bind(&library_id)
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let now = chrono::Utc::now();

        for (id, poster_url, local_path) in [
            (
                "pg-title-w500-only",
                "https://image.tmdb.org/t/p/w500/only.jpg",
                "/images/titles/pg-title-w500-only/poster/w500?v=legacy",
            ),
            (
                "pg-title-shared",
                "https://image.tmdb.org/t/p/w500/shared.jpg",
                "/images/titles/pg-title-shared/poster/w500?v=legacy",
            ),
            (
                "pg-title-unrelated-w5000",
                "https://image.tmdb.org/t/p/w500/unrelated.jpg",
                "/images/titles/pg-title-unrelated-w5000/poster/w5000?v=legacy",
            ),
        ] {
            sqlx::query(
                "INSERT INTO titles (
                    id, name, name_normalized, library_id, facet, monitored, status,
                    tags, external_ids, root_folder_id, genres, year, overview, created_at,
                    poster_url, poster_local_path
                 ) VALUES (
                    $1, $1, $1, $2, 'movie', TRUE, 'active', '[]', '[]', $3, '[]', 2024,
                    '', $4, $5, $6
                 )",
            )
            .bind(id)
            .bind(&library_id)
            .bind(&root_folder_id)
            .bind(now)
            .bind(poster_url)
            .bind(local_path)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            sqlx::query(
                "INSERT INTO title_images (
                    id, title_id, provider, kind, source_url, source_format,
                    source_width, source_height, created_at, updated_at
                 ) VALUES ($1, $2, 'tmdb', 'poster', $3, 'jpeg', 500, 750, $4, $4)",
            )
            .bind(format!("image-{id}"))
            .bind(id)
            .bind(poster_url)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        let w500_only_digest = format!("blake3:{}", "d".repeat(64));
        let shared_digest = format!("blake3:{}", "e".repeat(64));
        for (digest, bytes) in [
            (&w500_only_digest, vec![4_u8]),
            (&shared_digest, vec![5_u8]),
        ] {
            sqlx::query(
                "INSERT INTO title_image_blobs (
                    digest, format, width, height, bytes, created_at, updated_at
                 ) VALUES ($1, 'avif', 250, 375, $2, $3, $3)",
            )
            .bind(digest)
            .bind(bytes)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        for (id, image_id, variant, digest) in [
            (
                "pg-variant-w500-only",
                "image-pg-title-w500-only",
                "w500",
                &w500_only_digest,
            ),
            (
                "pg-variant-shared-w500",
                "image-pg-title-shared",
                "w500",
                &shared_digest,
            ),
            (
                "pg-variant-shared-w250",
                "image-pg-title-shared",
                "w250",
                &shared_digest,
            ),
        ] {
            sqlx::query(
                "INSERT INTO title_image_variants (
                    id, title_image_id, variant_key, blob_digest, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $5)",
            )
            .bind(id)
            .bind(image_id)
            .bind(variant)
            .bind(digest)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        let migrated_services = PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;
        let w500_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM title_image_variants WHERE variant_key = 'w500'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let remaining_digests: Vec<String> =
            sqlx::query_scalar("SELECT digest FROM title_image_blobs ORDER BY digest")
                .fetch_all(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
        let local_paths: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, poster_local_path
               FROM titles
              WHERE id IN ('pg-title-shared', 'pg-title-unrelated-w5000', 'pg-title-w500-only')
              ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(w500_count, 0);
        assert_eq!(remaining_digests, vec![shared_digest]);
        assert_eq!(
            local_paths,
            vec![
                (
                    "pg-title-shared".to_string(),
                    Some(
                        "/images/titles/pg-title-shared/poster/w250?v=eeeeeeeeeeeeeeee".to_string()
                    ),
                ),
                (
                    "pg-title-unrelated-w5000".to_string(),
                    Some(
                        "/images/titles/pg-title-unrelated-w5000/poster/w5000?v=legacy".to_string()
                    ),
                ),
                ("pg-title-w500-only".to_string(), None),
            ]
        );

        sqlx::query(
            "INSERT INTO image_proxy_sources (
                token, upstream_url, owner_type, owner_id, image_kind, fallback_class,
                last_seen_at
             ) VALUES ('pg-person-token', NULL, 'person', 'person-1', 'person', 'portrait', $1)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query(
            "INSERT INTO image_proxy_cache_entries (
                token, variant, content_type, byte_size, fetched_at, last_accessed_at
             ) VALUES ('pg-person-token', 'profile', 'image/jpeg', 12, $1, $1)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        migrated_services.pool().close().await;
        Ok::<_, AppError>(())
    }
    .await;

    pool.close().await;
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to drop test schema: {error}")));
    admin_pool.close().await;
    cleanup?;
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
    let root_folder_id: String = sqlx::query_scalar(
        "SELECT id FROM library_roots WHERE library_id = ? ORDER BY is_default DESC, path LIMIT 1",
    )
    .bind(&library_id)
    .fetch_one(&pool)
    .await
    .expect("0.16.8 default movie root should exist");
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
    .bind("/data/movies2/Sasaki and Miyano Graduation/movie.mkv")
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
    assert_eq!(
        media.1,
        "/data/movies2/Sasaki and Miyano Graduation/movie.mkv"
    );

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

#[tokio::test]
async fn migration_0140_postgres_transfers_shared_image_bytes_through_catalog_from_env()
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
        "scryer_image_migration_{}",
        chrono::Utc::now().timestamp_micros()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create postgres test schema: {error}"))
        })?;
    let mut schema_url = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
    schema_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(schema_url.as_str())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to open postgres test schema: {error}"))
        })?;

    let result = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(139)).await?;
        let library_id = scryer_domain::default_library_id_for_facet(
            &scryer_domain::MediaFacet::Movie,
        );
        let root_folder_id: String = sqlx::query_scalar(
            "SELECT id FROM library_roots
              WHERE library_id = $1
              ORDER BY is_default DESC, path
              LIMIT 1",
        )
        .bind(&library_id)
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let now = chrono::Utc::now();

        for (id, name) in [
            ("pg-image-title-a", "Postgres Image A"),
            ("pg-image-title-b", "Postgres Image B"),
            ("pg-image-title-corrupt", "Postgres Image Corrupt"),
        ] {
            sqlx::query(
                "INSERT INTO titles (
                    id, name, name_normalized, library_id, facet, monitored, status,
                    tags, external_ids, root_folder_id, genres, year, overview, created_at
                 ) VALUES ($1, $2, $2, $3, 'movie', TRUE, 'active', '[]', '[]', $4, '[]', 2024, '', $5)",
            )
            .bind(id)
            .bind(name)
            .bind(&library_id)
            .bind(&root_folder_id)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        sqlx::query(
            "INSERT INTO media_files (
                id, title_id, file_path, size_bytes, scan_status, created_at
             ) VALUES ('pg-image-media-a', 'pg-image-title-a', '/data/pg-image-a.mkv', 321, 'complete', $1)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let bytes = vec![4_u8, 5, 6];
        let digest = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        for (title_id, image_id) in [
            ("pg-image-title-a", "pg-image-a"),
            ("pg-image-title-b", "pg-image-b"),
            ("pg-image-title-corrupt", "pg-image-corrupt"),
        ] {
            sqlx::query(
                "INSERT INTO title_images (
                    id, title_id, provider, provider_image_id, kind, source_url,
                    source_etag, source_last_modified, source_format, source_width,
                    source_height, created_at, updated_at
                 ) VALUES ($1, $2, 'tvdb', NULL, 'poster', $3, NULL, NULL, 'jpeg', 1000, 1500, $4, $4)",
            )
            .bind(image_id)
            .bind(title_id)
            .bind(format!("https://artworks.thetvdb.com/{image_id}.jpg"))
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            sqlx::query("UPDATE titles SET poster_local_path = $1 WHERE id = $2")
                .bind(format!(
                    "/images/titles/{title_id}/poster/w250?v=legacy"
                ))
                .bind(title_id)
                .execute(&pool)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        for (variant_id, image_id, variant_bytes) in [
            ("pg-variant-a", "pg-image-a", bytes.clone()),
            ("pg-variant-b", "pg-image-b", bytes.clone()),
            (
                "pg-variant-corrupt",
                "pg-image-corrupt",
                vec![9_u8, 9, 9],
            ),
        ] {
            sqlx::query(
                "INSERT INTO title_image_variants (
                    id, title_image_id, variant_key, path, format, width, height,
                    bytes, digest, created_at, updated_at
                 ) VALUES ($1, $2, 'w250', $3, 'avif', 250, 375, $4, $5, $6, $6)",
            )
            .bind(variant_id)
            .bind(image_id)
            .bind(format!("/legacy-cache/{variant_id}.avif"))
            .bind(variant_bytes)
            .bind(&digest)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        let migrated_services = PostgresServices::new_with_mode(
            schema_url.as_str(),
            crate::types::MigrationMode::Apply,
        )
        .await?;

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
            .fetch_one(&pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let variants: Vec<(String, String)> = sqlx::query_as(
            "SELECT ti.title_id, tiv.blob_digest
               FROM title_image_variants tiv
               JOIN title_images ti ON ti.id = tiv.title_image_id
              ORDER BY ti.title_id",
        )
        .fetch_all(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let corrupt_local_path: Option<String> = sqlx::query_scalar(
            "SELECT poster_local_path FROM titles WHERE id = 'pg-image-title-corrupt'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let media_owner: String = sqlx::query_scalar(
            "SELECT title_id FROM media_files WHERE id = 'pg-image-media-a'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        assert_eq!(blob_count, 1);
        assert_eq!(
            variants,
            vec![
                ("pg-image-title-a".to_string(), digest.clone()),
                ("pg-image-title-b".to_string(), digest),
            ]
        );
        assert!(corrupt_local_path.is_none());
        assert_eq!(media_owner, "pg-image-title-a");
        migrated_services.pool().close().await;
        Ok::<_, AppError>(())
    }
    .await;

    pool.close().await;
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to drop test schema: {error}")));
    admin_pool.close().await;
    cleanup?;
    result
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
