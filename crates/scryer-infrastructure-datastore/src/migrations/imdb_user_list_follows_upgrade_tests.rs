//! Upgrade coverage for migration 0268.
//!
//! IMDb user-list follows are removed the way an unfollow removes a follow:
//! the follow and everything that hangs off it go, while every other follow
//! (IMDb charts included), the instance-wide exclusions, and the titles and
//! requests a follow touched stay.
//! The migration deletes the dependent rows itself, so the outcome must be the
//! same whether or not the connection enforces foreign keys.
//!
//! The scenario drives the real migration runner over a database built at the
//! pre-0268 state by the real catalog.

use sqlx::SqlitePool;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0274_delete_imdb_user_list_follows.sql");
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0274_delete_imdb_user_list_follows.sql"
);

/// Version the catalog is replayed to before 0268 is applied.
const PRE_UPGRADE_VERSION: i64 = 267;

const IMDB_CHART: &str = "follow-imdb-chart";
const IMDB_LIST: &str = "follow-imdb-list";
const TMDB_CHART: &str = "follow-tmdb-chart";

/// Tables holding one follow's own rows, and the column naming the follow.
const FOLLOW_TABLES: &[(&str, &str)] = &[
    ("list_subscriptions", "id"),
    ("list_subscription_routes", "subscription_id"),
    ("list_memberships", "subscription_id"),
    ("list_sync_runs", "subscription_id"),
    ("list_exclusions", "subscription_id"),
];

async fn pre_upgrade_pool(foreign_keys: bool) -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");
    crate::migrations::replay_source_catalog_for_fresh_install(
        &pool,
        Some(PRE_UPGRADE_VERSION),
        true,
    )
    .await
    .expect("pre-0268 migration fixture should apply");
    let pragma = if foreign_keys {
        "PRAGMA foreign_keys = ON"
    } else {
        "PRAGMA foreign_keys = OFF"
    };
    sqlx::query(pragma)
        .execute(&pool)
        .await
        .expect("foreign-key mode should set");
    pool
}

async fn exec(pool: &SqlitePool, sql: &str, binds: &[&str]) {
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
    for bind in binds {
        query = query.bind(*bind);
    }
    query
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn count(pool: &SqlitePool, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .bind(bind)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("count failed: {error}\n{sql}"))
}

async fn seed_shared(pool: &SqlitePool) {
    exec(
        pool,
        "INSERT INTO users (id, username, created_at, updated_at)
         VALUES ('list-manager', 'list-manager', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
        &[],
    )
    .await;
    exec(
        pool,
        "INSERT INTO titles (id, name, facet, monitored, created_at, library_id, root_folder_id)
         SELECT 'title-from-imdb-list', 'Synthetic Listed Film', 'movie', 1, '2026-09-01',
                l.id, r.id
         FROM libraries l JOIN library_roots r ON r.library_id = l.id
         WHERE l.facet = 'movie' AND l.is_default = 1 AND r.is_default = 1",
        &[],
    )
    .await;
    exec(
        pool,
        "INSERT INTO media_requests (id, library_id, facet, status, identity_fingerprint, title,
             created_by_user_id, created_at, updated_at, origin_kind, origin_subscription_id)
         SELECT 'request-from-imdb', l.id, 'movie', 'pending', 'fixture-fingerprint',
                'Synthetic Requested Film', 'list-manager', '2026-09-01', '2026-09-01',
                'public_list', ?1
         FROM libraries l WHERE l.facet = 'movie' AND l.is_default = 1",
        &[IMDB_LIST],
    )
    .await;
    // An instance-wide exclusion belongs to no follow and must survive.
    exec(
        pool,
        "INSERT INTO list_exclusions (id, kind, display_title, scope, subscription_id, created_at)
         VALUES ('exclusion-all-lists', 'movie', 'Synthetic Excluded Film', 'all_lists', NULL,
                 '2026-09-01')",
        &[],
    )
    .await;
    exec(
        pool,
        "INSERT INTO list_exclusion_external_ids (exclusion_id, source, value)
         VALUES ('exclusion-all-lists', 'tmdb', '900001')",
        &[],
    )
    .await;
}

async fn seed_follow(
    pool: &SqlitePool,
    id: &str,
    provider: &str,
    source_type: &str,
    source_origin: &str,
    chart_key: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO list_subscriptions (id, scope, owner_user_id, provider, source_type,
             source_origin, chart_key, chart_scope, name, mode, interval_seconds, created_at,
             updated_at)
         VALUES (?1, 'public', 'list-manager', ?2, ?3, ?4, ?5,
                 CASE WHEN ?5 IS NULL THEN NULL ELSE 'global' END,
                 'Synthetic list ' || ?1, 'add', 43200, '2026-09-01', '2026-09-01')",
    )
    .bind(id)
    .bind(provider)
    .bind(source_type)
    .bind(source_origin)
    .bind(chart_key)
    .execute(pool)
    .await
    .expect("follow should insert");
    exec(
        pool,
        "INSERT INTO list_subscription_routes (subscription_id, kind, library_id, monitor_type)
         SELECT ?1, 'movie', id, 'monitored' FROM libraries
         WHERE facet = 'movie' AND is_default = 1",
        &[id],
    )
    .await;
    let title_id = (id == IMDB_LIST).then_some("title-from-imdb-list");
    for (item, title) in [("item-a", title_id), ("item-b", None)] {
        sqlx::query(
            "INSERT INTO list_memberships (subscription_id, item_key, title_id, kind, state,
                 added_by_list, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'movie', 'in_library', 1, '2026-09-01', '2026-09-02')",
        )
        .bind(id)
        .bind(item)
        .bind(title)
        .execute(pool)
        .await
        .expect("membership should insert");
    }
    exec(
        pool,
        "INSERT INTO list_sync_runs (id, subscription_id, started_at, finished_at, outcome)
         VALUES ('run-' || ?1, ?1, '2026-09-02', '2026-09-02', 'succeeded')",
        &[id],
    )
    .await;
    exec(
        pool,
        "INSERT INTO list_exclusions (id, kind, display_title, scope, subscription_id, created_at)
         VALUES ('exclusion-' || ?1, 'movie', 'Synthetic Skipped Film', 'list', ?1,
                 '2026-09-02')",
        &[id],
    )
    .await;
    exec(
        pool,
        "INSERT INTO list_exclusion_external_ids (exclusion_id, source, value, external_kind)
         VALUES ('exclusion-' || ?1, 'tmdb', '900002', 'movie')",
        &[id],
    )
    .await;
}

async fn assert_follow_rows(pool: &SqlitePool, id: &str, expected: [i64; 6], context: &str) {
    let mut actual = [0i64; 6];
    for (slot, (table, column)) in actual.iter_mut().zip(FOLLOW_TABLES) {
        *slot = count(
            pool,
            &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
            id,
        )
        .await;
    }
    actual[5] = count(
        pool,
        "SELECT COUNT(*) FROM list_exclusion_external_ids WHERE exclusion_id = 'exclusion-' || ?1",
        id,
    )
    .await;
    assert_eq!(
        actual, expected,
        "{context}: rows of {id} in subscriptions, routes, memberships, sync runs, \
         exclusions, exclusion ids"
    );
}

async fn run_upgrade(foreign_keys: bool) {
    let pool = pre_upgrade_pool(foreign_keys).await;
    seed_shared(&pool).await;
    seed_follow(
        &pool,
        IMDB_CHART,
        "imdb",
        "smg_chart:imdb.fixture.movies:global",
        "smg_chart",
        Some("imdb.fixture.movies"),
    )
    .await;
    seed_follow(
        &pool,
        IMDB_LIST,
        " IMDb ",
        "user_list",
        "smg_imdb_list",
        None,
    )
    .await;
    seed_follow(
        &pool,
        TMDB_CHART,
        "tmdb",
        "smg_chart:tmdb.fixture.movies:global",
        "smg_chart",
        Some("tmdb.fixture.movies"),
    )
    .await;
    for id in [IMDB_CHART, IMDB_LIST, TMDB_CHART] {
        assert_follow_rows(&pool, id, [1, 1, 2, 1, 1, 1], "before the upgrade").await;
    }

    crate::migrations::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("0268 upgrade should apply");

    assert_follow_rows(
        &pool,
        IMDB_LIST,
        [0; 6],
        "an IMDb user-list follow is gone with its rows",
    )
    .await;
    for (id, provider, source_type) in [
        (IMDB_CHART, "imdb", "smg_chart:imdb.fixture.movies:global"),
        (TMDB_CHART, "tmdb", "smg_chart:tmdb.fixture.movies:global"),
    ] {
        assert_follow_rows(&pool, id, [1, 1, 2, 1, 1, 1], "a chart follow is untouched").await;
        let follow: (String, String, String, String) = sqlx::query_as(
            "SELECT provider, source_type, source_origin, name FROM list_subscriptions
             WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("the chart follow should load");
        assert_eq!(
            follow,
            (
                provider.to_string(),
                source_type.to_string(),
                "smg_chart".to_string(),
                format!("Synthetic list {id}"),
            )
        );
    }

    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM list_exclusion_external_ids WHERE exclusion_id = ?1",
            "exclusion-all-lists",
        )
        .await,
        1,
        "an instance-wide exclusion keeps its ids"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM list_exclusions WHERE id = ?1 AND scope = 'all_lists'",
            "exclusion-all-lists",
        )
        .await,
        1,
        "an instance-wide exclusion stays"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM titles WHERE id = ?1",
            "title-from-imdb-list"
        )
        .await,
        1,
        "a title an IMDb list follow added stays in the library"
    );
    let request: (String, String) = sqlx::query_as(
        "SELECT origin_kind, origin_subscription_id FROM media_requests WHERE id = ?1",
    )
    .bind("request-from-imdb")
    .fetch_one(&pool)
    .await
    .expect("a request an IMDb list follow made stays");
    assert_eq!(request, ("public_list".to_string(), IMDB_LIST.to_string()));
}

#[tokio::test]
async fn the_upgrade_deletes_imdb_user_list_follows_and_keeps_everything_else() {
    assert_eq!(SQLITE, POSTGRES, "both engines delete the same follows");
    run_upgrade(true).await;
}

#[tokio::test]
async fn the_upgrade_does_not_rely_on_foreign_key_cascades() {
    run_upgrade(false).await;
}

#[tokio::test]
async fn postgres_imdb_user_list_cleanup_deletes_only_those_rows() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // Temporary tables shadow persistent names and disappear with this
    // connection. They carry no foreign keys, so only the explicit deletes
    // can remove a dependent row.
    sqlx::raw_sql(
        "CREATE TEMP TABLE list_subscriptions (id text PRIMARY KEY, provider text NOT NULL,
             source_origin text NOT NULL);
         CREATE TEMP TABLE list_subscription_routes (subscription_id text NOT NULL);
         CREATE TEMP TABLE list_memberships (subscription_id text NOT NULL);
         CREATE TEMP TABLE list_sync_runs (subscription_id text NOT NULL);
         CREATE TEMP TABLE list_exclusions (id text PRIMARY KEY, subscription_id text);
         CREATE TEMP TABLE list_exclusion_external_ids (exclusion_id text NOT NULL);
         INSERT INTO list_subscriptions VALUES
             ('follow-imdb-chart', 'imdb', 'smg_chart'),
             ('follow-imdb-list', ' IMDb ', 'smg_imdb_list'),
             ('follow-tmdb-chart', 'tmdb', 'smg_chart');
         INSERT INTO list_subscription_routes SELECT id FROM list_subscriptions;
         INSERT INTO list_memberships SELECT id FROM list_subscriptions;
         INSERT INTO list_sync_runs SELECT id FROM list_subscriptions;
         INSERT INTO list_exclusions SELECT 'exclusion-' || id, id FROM list_subscriptions;
         INSERT INTO list_exclusions VALUES ('exclusion-all-lists', NULL);
         INSERT INTO list_exclusion_external_ids SELECT id FROM list_exclusions;",
    )
    .execute(&pool)
    .await
    .unwrap();
    const CHART_FOLLOWS: [&str; 2] = ["follow-imdb-chart", "follow-tmdb-chart"];
    const KEPT_EXCLUSIONS: [&str; 3] = [
        "exclusion-all-lists",
        "exclusion-follow-imdb-chart",
        "exclusion-follow-tmdb-chart",
    ];
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    for (table, column, expected) in [
        ("list_subscriptions", "id", CHART_FOLLOWS.to_vec()),
        (
            "list_subscription_routes",
            "subscription_id",
            CHART_FOLLOWS.to_vec(),
        ),
        (
            "list_memberships",
            "subscription_id",
            CHART_FOLLOWS.to_vec(),
        ),
        ("list_sync_runs", "subscription_id", CHART_FOLLOWS.to_vec()),
        ("list_exclusions", "id", KEPT_EXCLUSIONS.to_vec()),
        (
            "list_exclusion_external_ids",
            "exclusion_id",
            KEPT_EXCLUSIONS.to_vec(),
        ),
    ] {
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM {table} ORDER BY {column}"
        )))
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows, expected, "{table} keeps every chart follow's rows");
    }
    pool.close().await;
}
