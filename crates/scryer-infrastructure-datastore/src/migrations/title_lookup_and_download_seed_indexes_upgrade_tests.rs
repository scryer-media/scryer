use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str = include_str!(
    "../../../scryer/src/db/migrations/0277_title_lookup_and_download_seed_indexes.sql"
);
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0277_title_lookup_and_download_seed_indexes.sql"
);
const INDEX_NAMES: [&str; 2] = [
    "idx_downloads_created_at_id",
    "idx_title_search_terms_kind_literal",
];

/// `sql` is a literal `EXPLAIN QUERY PLAN` statement.
async fn query_plan(pool: &sqlx::SqlitePool, sql: &'static str) -> Vec<String> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(sql)
        .fetch_all(pool)
        .await
        .expect("query plan should load");
    rows.into_iter().map(|row| row.3).collect()
}

async fn assert_sqlite_indexes(pool: &sqlx::SqlitePool) {
    for name in INDEX_NAMES {
        let definition: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?")
                .bind(name)
                .fetch_one(pool)
                .await
                .expect("lookup index should exist");
        assert!(definition.contains("CREATE INDEX"));
    }

    // The exact spelling lane of release and import title resolution.
    let plan = query_plan(
        pool,
        "EXPLAIN QUERY PLAN SELECT DISTINCT title_id FROM title_search_terms \
         WHERE term_kind IN ('name', 'alias', 'tagged_alias') \
         AND literal_term IN ('fixture key', 'other key')",
    )
    .await;
    assert!(
        plan.iter().any(|detail| detail
            .contains("idx_title_search_terms_kind_literal (term_kind=? AND literal_term=?)")),
        "spelling lookup should search the kind/literal index: {plan:?}"
    );

    // The shape lane: each key column answers its own indexed branch.
    let plan = query_plan(
        pool,
        "EXPLAIN QUERY PLAN SELECT title_id FROM title_search_terms \
         WHERE term_kind IN ('name', 'alias', 'tagged_alias') \
         AND literal_term IN ('fixture key') \
         UNION \
         SELECT title_id FROM title_search_terms \
         WHERE term_kind IN ('name', 'alias', 'tagged_alias') \
         AND stripped_year_key IN ('fixture key')",
    )
    .await;
    assert!(
        plan.iter().any(|detail| detail
            .contains("idx_title_search_terms_kind_literal (term_kind=? AND literal_term=?)")),
        "shape lookup spelling branch should search the kind/literal index: {plan:?}"
    );
    assert!(
        plan.iter().any(|detail| detail.contains(
            "idx_title_search_terms_stripped_year_key (term_kind=? AND stripped_year_key=?)"
        )),
        "shape lookup year-stripped branch should search its index: {plan:?}"
    );

    // Cleanup seeding pages downloads in creation order without a sort.
    let plan = query_plan(
        pool,
        "EXPLAIN QUERY PLAN SELECT d.id FROM downloads d JOIN download_submissions s ON s.id = d.id \
         WHERE NOT EXISTS (SELECT 1 FROM download_cleanup c WHERE c.download_id = d.id) \
         ORDER BY d.created_at, d.id LIMIT 100",
    )
    .await;
    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_downloads_created_at_id")),
        "cleanup seeding should walk downloads in index order: {plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.contains("ORDER BY")),
        "cleanup seeding should not sort every download: {plan:?}"
    );
}

#[tokio::test]
async fn title_lookup_and_download_seed_indexes_apply_on_fresh_install() {
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
async fn title_lookup_and_download_seed_indexes_upgrade_from_released_and_integration_catalogs() {
    for from_version in [268, 276] {
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
async fn title_lookup_and_download_seed_indexes_postgres_preserve_rows_and_apply_idempotently() {
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
        "CREATE TEMP TABLE title_search_terms (
            term_id BIGINT PRIMARY KEY, title_id TEXT NOT NULL, term_kind TEXT NOT NULL,
            literal_term TEXT NOT NULL DEFAULT '');
         CREATE TEMP TABLE downloads (id TEXT PRIMARY KEY, created_at TEXT NOT NULL);
         INSERT INTO title_search_terms VALUES
            (1, 'first', 'name', 'fixture key'),
            (2, 'second', 'alias', 'other key');
         INSERT INTO downloads VALUES ('later', '2026-02-01'), ('earlier', '2026-01-01');",
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
           AND indexname IN ('idx_downloads_created_at_id',
                             'idx_title_search_terms_kind_literal')
         ORDER BY indexname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(names, INDEX_NAMES.map(str::to_owned));
    let titles: Vec<String> = sqlx::query_scalar(
        "SELECT title_id FROM title_search_terms
         WHERE term_kind IN ('name', 'alias', 'tagged_alias') AND literal_term IN ('fixture key')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(titles, ["first"]);
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM downloads ORDER BY created_at, id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, ["earlier", "later"]);
}
