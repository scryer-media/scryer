use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0344_catalog_sort_indexes.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0344_catalog_sort_indexes.sql");

const ADDED: [&str; 15] = [
    "idx_blocklist_created_at",
    "idx_domain_events_media_request_sequence",
    "idx_download_import_artifacts_canonical_download",
    "idx_media_requests_status_created",
    "idx_media_requests_updated_order",
    "idx_title_metadata_external_ratings_title_source_key",
    "idx_titles_library_popularity_order_asc",
    "idx_titles_library_popularity_order_desc",
    "idx_titles_library_runtime_order_asc",
    "idx_titles_library_runtime_order_desc",
    "idx_titles_library_status_order_asc",
    "idx_titles_library_status_order_desc",
    "idx_titles_library_year_order_asc",
    "idx_titles_library_year_order_desc",
    "idx_wanted_items_updated_at",
];

const STATUS: &str = "CASE LOWER(TRIM(COALESCE(content_status, ''))) \
     WHEN 'returning' THEN 'continuing' WHEN 'finished' THEN 'ended' \
     ELSE LOWER(TRIM(COALESCE(content_status, ''))) END";

/// One library's catalog page in each indexed sort, in the order the store
/// builds, with the index that serves it.
fn catalog_page_queries(collation: &str) -> Vec<(String, String)> {
    let name_tie = format!(
        "catalog_sort_key COLLATE {collation} ASC, catalog_name_tie_key COLLATE {collation} ASC"
    );
    let title_tie = format!(
        "{name_tie}, CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year ASC, \
         titles.id COLLATE {collation} ASC"
    );
    let mut queries = Vec::new();
    for (direction, suffix) in [("ASC", "asc"), ("DESC", "desc")] {
        let orders = [
            (
                "year",
                format!(
                    "CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year {direction}, \
                     {name_tie}, titles.id COLLATE {collation} ASC"
                ),
            ),
            (
                "runtime",
                format!(
                    "CASE WHEN runtime_minutes IS NULL THEN 1 ELSE 0 END ASC, \
                     runtime_minutes {direction}, {title_tie}"
                ),
            ),
            (
                "popularity",
                format!(
                    "CASE WHEN popularity IS NULL THEN 1 ELSE 0 END ASC, \
                     popularity {direction}, {title_tie}"
                ),
            ),
            (
                "status",
                format!(
                    "CASE WHEN {STATUS} = '' THEN 1 ELSE 0 END {direction}, \
                     {STATUS} {direction}, {title_tie}"
                ),
            ),
        ];
        for (name, order) in orders {
            queries.push((
                format!(
                    "SELECT titles.id FROM titles \
                     WHERE library_id IN ('library-emberfall') AND facet = 'movie' \
                     ORDER BY {order} LIMIT 50 OFFSET 100"
                ),
                format!("idx_titles_library_{name}_order_{suffix}"),
            ));
        }
    }
    queries
}

/// The rating join of a catalog rating sort, reading only its scope.
const SCOPED_RATINGS: &str = "WITH scoped_titles AS (
        SELECT id FROM titles WHERE library_id IN ('library-cinderwake'))
     SELECT ter.title_id, MAX(ter.normalized) AS normalized
       FROM title_metadata_external_ratings ter
       JOIN scoped_titles ON scoped_titles.id = ter.title_id
      WHERE LOWER(REPLACE(REPLACE(REPLACE(TRIM(ter.source), ' ', ''), '_', ''), '-', ''))
            IN ('tvdb', 'thetvdb')
      GROUP BY ter.title_id";

const ARTIFACTS_OF_DOWNLOAD: &str = "SELECT id FROM download_import_artifacts \
     WHERE canonical_download_id = 'download-7' ORDER BY created_at";

const BLOCKLIST_PAGE: &str = "SELECT id FROM blocklist ORDER BY created_at DESC LIMIT 50 OFFSET 0";

const WANTED_PAGE: &str = "SELECT w.id FROM wanted_items w \
     LEFT JOIN titles t ON t.id = w.title_id \
     WHERE 1=1 ORDER BY w.updated_at DESC LIMIT 50 OFFSET 0";

/// The requests list in its default order, as the store builds it.
const REQUESTS_PAGE: &str = "SELECT id FROM media_requests WHERE 1 = 1 \
     ORDER BY updated_at DESC, created_at DESC, id DESC LIMIT 50 OFFSET 100";

/// The dashboard's pending requests, longest waiting first.
const PENDING_REQUESTS: &str = "SELECT id FROM media_requests WHERE 1 = 1 AND status = 'pending' \
     ORDER BY created_at ASC, id ASC LIMIT 15 OFFSET 0";

/// The same page for an account that sees only some libraries.
const PENDING_REQUESTS_OF_LIBRARIES: &str = "SELECT id FROM media_requests WHERE 1 = 1 \
     AND status = 'pending' AND library_id IN ('library-emberfall', 'library-cinderwake') \
     ORDER BY created_at ASC, id ASC LIMIT 15 OFFSET 0";

/// One title's request submissions, read by request key.
const SUBMISSIONS_OF_REQUESTS: &str = "SELECT sequence FROM domain_events \
     WHERE event_type || '' IN ('media_request_submitted') \
       AND media_request_id IN ('request-8', 'request-80', 'request-800') \
     ORDER BY sequence DESC LIMIT 500";

/// The same read when requests removed since they were submitted are wanted
/// too.
const SUBMISSIONS_OF_REQUESTS_AND_REMOVED: &str = "SELECT sequence FROM domain_events \
     WHERE event_type || '' IN ('media_request_submitted') \
       AND media_request_id IS NOT NULL \
       AND (media_request_id IN ('request-8', 'request-80', 'request-800') \
            OR NOT EXISTS (SELECT 1 FROM media_requests \
                            WHERE media_requests.id = domain_events.media_request_id)) \
     ORDER BY +sequence DESC LIMIT 500";

async fn query_plan(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    let rows: Vec<(i64, i64, i64, String)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(pool)
            .await
            .unwrap_or_else(|error| panic!("query plan should load for {sql}: {error}"));
    rows.into_iter().map(|row| row.3).collect()
}

async fn assert_plan_uses(pool: &sqlx::SqlitePool, state: &str, sql: &str, index: &str) {
    let plan = query_plan(pool, sql).await;
    assert!(
        plan.iter().any(|detail| detail.contains(index)),
        "{state}: expected {index} in the plan of {sql}: {plan:?}"
    );
}

async fn assert_plan_uses_without_sort(
    pool: &sqlx::SqlitePool,
    state: &str,
    sql: &str,
    index: &str,
) {
    let plan = query_plan(pool, sql).await;
    assert!(
        plan.iter().any(|detail| detail.contains(index)),
        "{state}: expected {index} in the plan of {sql}: {plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.contains("TEMP B-TREE")),
        "{state}: {index} should return rows already in order for {sql}: {plan:?}"
    );
}

async fn sqlite_indexes_present(pool: &sqlx::SqlitePool) -> Vec<&'static str> {
    let mut present = Vec::new();
    for name in ADDED {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap();
        if count == 1 {
            present.push(name);
        }
    }
    present
}

/// Plans that must not depend on whether `ANALYZE` has run.
async fn assert_sqlite_plans(pool: &sqlx::SqlitePool, state: &str) {
    for (sql, index) in catalog_page_queries("BINARY") {
        assert_plan_uses_without_sort(pool, state, &sql, &index).await;
    }
    assert_plan_uses(
        pool,
        state,
        SCOPED_RATINGS,
        "idx_title_metadata_external_ratings_title_source_key",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        ARTIFACTS_OF_DOWNLOAD,
        "idx_download_import_artifacts_canonical_download",
    )
    .await;
    assert_plan_uses_without_sort(pool, state, BLOCKLIST_PAGE, "idx_blocklist_created_at").await;
    assert_plan_uses_without_sort(pool, state, WANTED_PAGE, "idx_wanted_items_updated_at").await;
    assert_plan_uses_without_sort(
        pool,
        state,
        REQUESTS_PAGE,
        "idx_media_requests_updated_order",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        PENDING_REQUESTS,
        "idx_media_requests_status_created",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        PENDING_REQUESTS_OF_LIBRARIES,
        "idx_media_requests_status_created",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        SUBMISSIONS_OF_REQUESTS,
        "idx_domain_events_media_request_sequence",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        SUBMISSIONS_OF_REQUESTS_AND_REMOVED,
        "idx_domain_events_media_request_sequence",
    )
    .await;
    // The store binds the request keys; the partial index must still apply.
    let bound: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT sequence FROM domain_events \
          WHERE event_type || '' IN (?1) AND media_request_id IN (?2, ?3) \
          ORDER BY sequence DESC LIMIT ?4",
    )
    .bind("media_request_submitted")
    .bind("request-8")
    .bind("request-80")
    .bind(500_i64)
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(
        bound
            .iter()
            .any(|row| row.3.contains("idx_domain_events_media_request_sequence")),
        "{state}: bound request keys should read the request index: {bound:?}"
    );
}

/// Synthetic rows with the skew a real library has: most titles in one
/// library, a few ratings per title, a few artifacts per download.
const SEED: &str = "PRAGMA foreign_keys = OFF;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
     INSERT INTO titles (id, name, facet, created_at, root_folder_id, library_id,
                         catalog_sort_key, catalog_name_tie_key, year, runtime_minutes,
                         popularity, content_status)
     SELECT 'title-' || i,
            CASE i % 3 WHEN 0 THEN 'Emberfall ' WHEN 1 THEN 'Cinderwake ' ELSE 'Ashgrove ' END || i,
            'movie', '2026-01-01T00:00:00Z', 'root-1',
            CASE WHEN i % 10 < 8 THEN 'library-emberfall'
                 WHEN i % 10 = 8 THEN 'library-cinderwake' ELSE 'library-ashgrove' END,
            printf('emberfall %05d', i), printf('emberfall %05d', i),
            CASE WHEN i % 10 = 0 THEN NULL ELSE 1990 + i % 30 END,
            CASE WHEN i % 7 = 0 THEN NULL ELSE 80 + i % 90 END,
            CASE WHEN i % 5 = 0 THEN NULL ELSE (i % 400) / 4.0 END,
            CASE i % 6 WHEN 0 THEN NULL WHEN 1 THEN '' WHEN 2 THEN 'Returning'
                       WHEN 3 THEN ' ended ' WHEN 4 THEN 'finished' ELSE 'Upcoming' END
       FROM n;
     WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 8999)
     INSERT INTO title_metadata_external_ratings (title_id, source, sort_index, normalized)
     SELECT 'title-' || (1 + i % 3000),
            CASE i / 3000 WHEN 0 THEN 'IMDb' WHEN 1 THEN 'The TVDB' ELSE 'rotten_tomatoes' END,
            i / 3000, (i % 100) / 10.0
       FROM n;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
     INSERT INTO download_import_artifacts (id, source_system, source_ref, normalized_file_name,
                                            media_kind, result, created_at,
                                            canonical_download_id)
     SELECT 'artifact-' || i, 'nzbget', 'job-' || (i % 600), 'emberfall-' || i || '.mkv',
            'movie', CASE WHEN i % 4 = 0 THEN 'rejected' ELSE 'imported' END,
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
            CASE WHEN i % 9 = 0 THEN NULL ELSE 'download-' || (i % 600) END
       FROM n;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
     INSERT INTO blocklist (id, title_id, release_name, reason, created_at,
                            normalized_release_name, indexer_id)
     SELECT 'blocked-' || i, 'title-' || (1 + i % 300), 'Cinderwake.' || i, 'failed',
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
            'cinderwake.' || i, 'indexer-' || (i % 4)
       FROM n;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
     INSERT INTO media_requests (id, library_id, facet, status, identity_fingerprint, title,
                                 policy_tags_json, created_by_user_id, created_at, updated_at)
     SELECT 'request-' || i, 'library-emberfall', 'movie',
            CASE i % 5 WHEN 0 THEN 'pending' WHEN 1 THEN 'rejected' ELSE 'approved' END,
            'fingerprint-' || i, 'Ashgrove ' || i, '[]', 'user-quillmere',
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 11, 1 + i % 27)
       FROM n;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
     INSERT INTO wanted_items (id, title_id, media_type, created_at, updated_at, status)
     SELECT 'wanted-' || i, 'title-' || i, 'movie', '2026-01-01T00:00:00Z',
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
            CASE WHEN i % 50 = 0 THEN 'grabbed' ELSE 'wanted' END
       FROM n;
     PRAGMA foreign_keys = ON;";

async fn seed(pool: &sqlx::SqlitePool) {
    sqlx::raw_sql(SEED)
        .execute(pool)
        .await
        .expect("synthetic rows should load");
}

/// What the seeded rows hold, read without any of the added indexes mattering.
async fn stored_rows(pool: &sqlx::SqlitePool) -> Vec<(String, i64, String)> {
    sqlx::query_as(
        "SELECT 'titles', COUNT(*), COALESCE(MAX(content_status), '') FROM titles
         UNION ALL
         SELECT 'ratings', COUNT(*), MAX(source) FROM title_metadata_external_ratings
         UNION ALL
         SELECT 'artifacts', COUNT(*), MAX(created_at) FROM download_import_artifacts
         UNION ALL
         SELECT 'blocklist', COUNT(*), MAX(created_at) FROM blocklist
         UNION ALL
         SELECT 'wanted', COUNT(*), MAX(updated_at) FROM wanted_items
         UNION ALL
         SELECT 'requests', COUNT(*), MAX(updated_at) FROM media_requests
         UNION ALL
         SELECT 'events', COUNT(*), MAX(event_id) FROM domain_events",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Request submissions stored the way a release before the request key
/// stored them: the request is named only inside the encoded payload.
async fn seed_submissions(pool: &sqlx::SqlitePool) {
    for n in 0..SUBMISSIONS {
        let payload = serde_json::json!({"type": "media_request_submitted", "data": {
            "request_id": format!("request-{n}"), "library_id": "library-emberfall"
        }});
        sqlx::query(
            "INSERT INTO domain_events (event_id, occurred_at, schema_version, stream_kind,
                                        event_type, payload_json)
             VALUES (?1, '2026-01-01T00:00:00Z', 1, 'global', ?2, ?3)",
        )
        .bind(format!("event-{n:05}"))
        // Every seventh event is another type that also names a request.
        .bind(if n % 7 == 0 {
            "media_request_approved"
        } else {
            "media_request_submitted"
        })
        .bind(
            scryer_infrastructure_sql::domain_event_payload::encode_domain_event_payload(&payload)
                .unwrap(),
        )
        .execute(pool)
        .await
        .unwrap();
    }
}

/// More than two backfill batches.
const SUBMISSIONS: i64 = 2100;

/// Events carrying a request key, and how many of them carry the key their
/// payload names.
async fn request_keys(pool: &sqlx::SqlitePool) -> (i64, i64) {
    sqlx::query_as(
        "SELECT COUNT(media_request_id),
                COALESCE(SUM(event_type = 'media_request_submitted'
                             AND media_request_id = 'request-' || CAST(SUBSTR(event_id, 7) AS INTEGER)), 0)
           FROM domain_events",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn memory_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

#[tokio::test]
async fn catalog_sort_indexes_apply_on_fresh_install_with_and_without_statistics() {
    assert_eq!(
        SQLITE.replace("COLLATE BINARY", "COLLATE \"C\"").replace(
            "ADD COLUMN media_request_id",
            "ADD COLUMN IF NOT EXISTS media_request_id"
        ),
        POSTGRES,
        "both engines make the same changes, differing only in collation spelling and in \
         how an existing column is tolerated"
    );
    let pool = memory_pool().await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("fresh migration catalog should apply");
    assert_eq!(sqlite_indexes_present(&pool).await, ADDED);
    assert_sqlite_plans(&pool, "without statistics").await;
    seed(&pool).await;
    assert_sqlite_plans(&pool, "seeded, without statistics").await;
    sqlx::raw_sql("ANALYZE").execute(&pool).await.unwrap();
    let analyzed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_stat1 WHERE idx = 'idx_titles_library_year_order_asc'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        analyzed, 1,
        "ANALYZE should record statistics for the new indexes"
    );
    assert_sqlite_plans(&pool, "after ANALYZE").await;
}

#[tokio::test]
async fn catalog_sort_indexes_upgrade_from_the_previous_catalog() {
    let pool = memory_pool().await;
    super::replay_source_catalog_for_fresh_install(&pool, Some(343), true)
        .await
        .expect("pre-upgrade catalog should apply");
    assert!(
        sqlite_indexes_present(&pool).await.is_empty(),
        "every index must be introduced after 343"
    );
    seed(&pool).await;
    seed_submissions(&pool).await;
    let before = stored_rows(&pool).await;
    assert_eq!(before.len(), 7);
    assert!(
        before.iter().all(|(_, count, _)| *count >= 2000),
        "{before:?}"
    );

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade must apply over stored rows");
    assert_eq!(sqlite_indexes_present(&pool).await, ADDED);
    assert_eq!(
        stored_rows(&pool).await,
        before,
        "adding indexes keeps every row"
    );
    // Every stored submission now carries its request, and nothing else does.
    let submissions = SUBMISSIONS - (SUBMISSIONS + 6) / 7;
    assert_eq!(request_keys(&pool).await, (submissions, submissions));
    assert_sqlite_plans(&pool, "upgraded").await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("later startup must be idempotent");
    assert_eq!(sqlite_indexes_present(&pool).await, ADDED);
    assert_eq!(stored_rows(&pool).await, before);
    assert_eq!(request_keys(&pool).await, (submissions, submissions));

    // The migration's index statements are safe to run again over its result.
    // SQLite cannot add a column twice; the migration ledger runs it once.
    let indexes_only = SQLITE.replace(
        "ALTER TABLE domain_events ADD COLUMN media_request_id TEXT;",
        "",
    );
    assert_ne!(indexes_only, SQLITE);
    sqlx::raw_sql(sqlx::AssertSqlSafe(indexes_only))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(sqlite_indexes_present(&pool).await, ADDED);
    assert_sqlite_plans(&pool, "reapplied").await;
}

#[tokio::test]
async fn catalog_sort_indexes_postgres_match_the_catalog_queries_and_apply_idempotently() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // Temporary tables plus a rolled-back transaction keep this fixture isolated.
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "CREATE TEMP TABLE titles (
            id TEXT PRIMARY KEY, library_id TEXT, facet TEXT NOT NULL,
            catalog_sort_key TEXT NOT NULL DEFAULT '', catalog_name_tie_key TEXT NOT NULL DEFAULT '',
            year INTEGER, runtime_minutes INTEGER, popularity DOUBLE PRECISION,
            content_status TEXT);
         CREATE TEMP TABLE title_metadata_external_ratings (
            title_id TEXT, movie_entity_id TEXT, source TEXT NOT NULL,
            sort_index INTEGER NOT NULL DEFAULT 0, normalized DOUBLE PRECISION NOT NULL);
         CREATE TEMP TABLE download_import_artifacts (
            id TEXT PRIMARY KEY, canonical_download_id TEXT, created_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE blocklist (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE wanted_items (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, status TEXT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE media_requests (
            id TEXT PRIMARY KEY, library_id TEXT NOT NULL, status TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE domain_events (
            sequence BIGINT PRIMARY KEY, event_type TEXT NOT NULL);
         INSERT INTO titles VALUES
            ('title-1', 'library-emberfall', 'movie', 'cinderwake', 'cinderwake', NULL, 95, NULL,
             ' Finished '),
            ('title-2', 'library-emberfall', 'movie', 'ashgrove', 'ashgrove', 2001, NULL, 4.5,
             NULL),
            ('title-3', 'library-emberfall', 'movie', 'emberfall', 'emberfall', 1987, 61, 9.0,
             'Returning'),
            ('title-4', 'library-cinderwake', 'movie', 'quillmere', 'quillmere', 1987, 61, 9.0,
             'ended');
         INSERT INTO title_metadata_external_ratings VALUES
            ('title-4', NULL, 'The TVDB', 0, 7.5),
            ('title-4', NULL, 'IMDb', 1, 9.9),
            ('title-1', NULL, 'tvdb', 0, 2.0);",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    }
    let present: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname = (SELECT nspname FROM pg_namespace WHERE oid = pg_my_temp_schema())
           AND indexname LIKE 'idx_%'
         ORDER BY indexname",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(present, ADDED);

    let rated: Vec<(String, f64)> = sqlx::query_as(sqlx::AssertSqlSafe(SCOPED_RATINGS))
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(rated, [("title-4".to_string(), 7.5)]);
    let page = |name: &str, suffix: &str| {
        let index = format!("idx_titles_library_{name}_order_{suffix}");
        catalog_page_queries("\"C\"")
            .into_iter()
            .find(|(_, served_by)| *served_by == index)
            .map(|(sql, _)| sql.replace(" LIMIT 50 OFFSET 100", ""))
            .unwrap()
    };
    for (name, suffix, expected) in [
        ("year", "asc", ["title-3", "title-2", "title-1"]),
        ("year", "desc", ["title-2", "title-3", "title-1"]),
        ("runtime", "asc", ["title-3", "title-1", "title-2"]),
        ("runtime", "desc", ["title-1", "title-3", "title-2"]),
        ("popularity", "asc", ["title-2", "title-3", "title-1"]),
        ("popularity", "desc", ["title-3", "title-2", "title-1"]),
        ("status", "asc", ["title-3", "title-1", "title-2"]),
        ("status", "desc", ["title-2", "title-1", "title-3"]),
    ] {
        let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(page(name, suffix)))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(ids, expected, "{name} {suffix}");
    }

    // A keyed submission read must sort a handful of rows rather than walk
    // every event backwards, so it is planned over stored rows with real
    // statistics and with sorts still allowed.
    sqlx::raw_sql(
        "INSERT INTO domain_events (sequence, event_type, media_request_id)
         SELECT i,
                CASE WHEN i % 4 = 0 THEN 'media_request_submitted' ELSE 'media_file_analyzed' END,
                CASE WHEN i % 4 = 0 THEN 'request-' || i END
           FROM generate_series(1, 20000) AS i;
         ANALYZE domain_events;",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let keyed: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(SUBMISSIONS_OF_REQUESTS))
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(keyed, [800, 80, 8]);
    let keyed_plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {SUBMISSIONS_OF_REQUESTS}"
    )))
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert!(
        keyed_plan
            .iter()
            .any(|line| line.contains("idx_domain_events_media_request_sequence")),
        "{keyed_plan:?}"
    );

    // Most requests are still stored; one submission in a hundred is of a
    // request since removed.
    sqlx::raw_sql(
        "INSERT INTO media_requests (id, library_id, status, created_at, updated_at)
         SELECT 'request-' || i, 'library-emberfall', 'approved', now(), now()
           FROM generate_series(4, 20000, 4) AS i
          WHERE i % 400 <> 0;
         ANALYZE media_requests;",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let with_removed: Vec<i64> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(SUBMISSIONS_OF_REQUESTS_AND_REMOVED))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    let mut expected = (1..=50)
        .map(|n| n * 400)
        .chain([8, 80])
        .collect::<Vec<i64>>();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(with_removed, expected);
    let with_removed_plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {SUBMISSIONS_OF_REQUESTS_AND_REMOVED}"
    )))
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert!(
        with_removed_plan
            .iter()
            .any(|line| line.contains("idx_domain_events_media_request_sequence")),
        "{with_removed_plan:?}"
    );

    // With sequential scans and sorts priced out, each statement shows the
    // index it can use: column order, collation, expression and direction match.
    sqlx::raw_sql("SET LOCAL enable_seqscan = off; SET LOCAL enable_sort = off;")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut statements = catalog_page_queries("\"C\"");
    statements.extend([
        (
            SCOPED_RATINGS.to_string(),
            "idx_title_metadata_external_ratings_title_source_key".to_string(),
        ),
        (
            ARTIFACTS_OF_DOWNLOAD.to_string(),
            "idx_download_import_artifacts_canonical_download".to_string(),
        ),
        (
            BLOCKLIST_PAGE.to_string(),
            "idx_blocklist_created_at".to_string(),
        ),
        (
            WANTED_PAGE.to_string(),
            "idx_wanted_items_updated_at".to_string(),
        ),
        (
            REQUESTS_PAGE.to_string(),
            "idx_media_requests_updated_order".to_string(),
        ),
        (
            PENDING_REQUESTS.to_string(),
            "idx_media_requests_status_created".to_string(),
        ),
        (
            PENDING_REQUESTS_OF_LIBRARIES.to_string(),
            "idx_media_requests_status_created".to_string(),
        ),
    ]);
    for (sql, index) in statements {
        let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("EXPLAIN {sql}")))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert!(
            plan.iter().any(|line| line.contains(&index)),
            "expected {index} in the plan of {sql}: {plan:?}"
        );
    }
    tx.rollback().await.unwrap();
}
