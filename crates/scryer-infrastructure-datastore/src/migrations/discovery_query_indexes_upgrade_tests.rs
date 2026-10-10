use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0276_discovery_query_indexes.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0276_discovery_query_indexes.sql");
const INDEX_NAMES: [&str; 2] = [
    "idx_discovery_item_rank_components_run",
    "idx_discovery_items_generation_relevance",
];

async fn assert_sqlite_indexes(pool: &sqlx::SqlitePool) {
    for name in INDEX_NAMES {
        let definition: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?")
                .bind(name)
                .fetch_one(pool)
                .await
                .expect("discovery index should exist");
        assert!(definition.contains("CREATE INDEX"));
    }
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT item_id FROM discovery_item_rank_components WHERE run_id = ?",
    )
    .bind("fixture-run")
    .fetch_all(pool)
    .await
    .expect("run lookup plan should load");
    assert!(
        plan.iter().any(|row| row.3.contains(INDEX_NAMES[0])),
        "run-scoped rank lookup should not scan the table: {plan:?}"
    );
}

#[tokio::test]
async fn discovery_query_indexes_apply_on_fresh_install() {
    assert_eq!(
        SQLITE, POSTGRES,
        "both engines use the same index definitions"
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("fresh migration catalog should apply");
    assert_sqlite_indexes(&pool).await;
}

#[tokio::test]
async fn discovery_query_indexes_upgrade_from_released_and_integration_catalogs() {
    for from_version in [268, 275] {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        super::replay_source_catalog_for_fresh_install(&pool, Some(from_version), true)
            .await
            .expect("pre-upgrade catalog should apply");
        for name in INDEX_NAMES {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?")
                    .bind(name)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(count, 0, "index must be introduced after {from_version}");
        }
        super::run_migrations(&pool, crate::MigrationMode::Apply)
            .await
            .expect("upgrade must retain the published migration checksums");
        assert_sqlite_indexes(&pool).await;
        super::run_migrations(&pool, crate::MigrationMode::Apply)
            .await
            .expect("later startup must be idempotent");
        assert_sqlite_indexes(&pool).await;
    }
}

#[tokio::test]
async fn discovery_query_indexes_postgres_preserve_rows_and_apply_idempotently() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // Temporary tables keep this migration fixture isolated from persistent data.
    sqlx::raw_sql(
        "CREATE TEMP TABLE discovery_items (
            id TEXT PRIMARY KEY, base_generation_id TEXT NOT NULL,
            owned_in_input BOOLEAN NOT NULL, recommendation_score DOUBLE PRECISION,
            rank_score DOUBLE PRECISION, sort_index INTEGER NOT NULL, tombstoned_at TIMESTAMPTZ);
         CREATE TEMP TABLE discovery_item_rank_components (
            item_id TEXT NOT NULL, run_id TEXT NOT NULL, component_index INTEGER NOT NULL,
            PRIMARY KEY (item_id, component_index));
         INSERT INTO discovery_items VALUES
            ('first', 'generation', FALSE, 0.9, 1.0, 0, NULL),
            ('missing', 'generation', FALSE, NULL, NULL, 0, NULL),
            ('old', 'generation', FALSE, 1.0, 1.0, 0, '2026-01-01');
         INSERT INTO discovery_item_rank_components VALUES ('first', 'fixture-run', 0);",
    )
    .execute(&pool)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(POSTGRES).execute(&pool).await.unwrap();
    }
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname = (SELECT nspname FROM pg_namespace WHERE oid = pg_my_temp_schema())
           AND indexname IN ('idx_discovery_item_rank_components_run',
                             'idx_discovery_items_generation_relevance')
         ORDER BY indexname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(names, INDEX_NAMES.map(str::to_owned));
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM discovery_items ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, ["first", "missing", "old"]);
    let ranked: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM discovery_items
         WHERE base_generation_id = 'generation' AND owned_in_input = FALSE AND tombstoned_at IS NULL
         ORDER BY COALESCE(recommendation_score, -999999999.0) DESC,
                  COALESCE(rank_score, -999999999.0) DESC, sort_index ASC, id ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(ranked, ["first", "missing"]);
}
