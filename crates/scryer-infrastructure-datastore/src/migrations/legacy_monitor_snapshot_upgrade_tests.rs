use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0265_discard_legacy_monitor_snapshots.sql");
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0265_discard_legacy_monitor_snapshots.sql"
);

#[tokio::test]
async fn legacy_monitor_snapshot_upgrade_discards_old_rows_and_preserves_new_attempts() {
    assert_eq!(
        SQLITE, POSTGRES,
        "both engines discard the same legacy state"
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(264), true)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO titles (id, name, facet, monitored, created_at, library_id, root_folder_id)
         SELECT CASE l.facet WHEN 'series' THEN 'off' ELSE 'on' END,
                'Monitoring fixture', l.facet, CASE l.facet WHEN 'series' THEN 0 ELSE 1 END,
                '2026-09-25', l.id, r.id
         FROM libraries l JOIN library_roots r ON r.library_id = l.id
         WHERE l.facet IN ('series', 'movie') AND l.is_default = 1 AND r.is_default = 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    for session in [
        "external-import-monitor-apply",
        "external-import-monitor-apply:library-a",
        "external-import-monitor-apply-consumed:6c696272617279:attempt",
        "source-session",
    ] {
        for facet in ["movie", "series", "anime"] {
            sqlx::query(
                "INSERT INTO external_import_monitor_snapshot_chunks
                (session_id, facet, entry_kind, chunk_index, payload_ndjson, created_at)
                VALUES (?, ?, ?, 0, '{}', '2026-09-25')",
            )
            .bind(session)
            .bind(facet)
            .bind(if facet == "movie" { "movie" } else { "series" })
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    // The runner owns the transaction: a failed upgrade can roll the cleanup back.
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(SQLITE).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM external_import_monitor_snapshot_chunks"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        12
    );

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM external_import_monitor_snapshot_chunks"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let titles: Vec<(String, i64)> = sqlx::query_as("SELECT id, monitored FROM titles ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(titles, vec![("off".into(), 0), ("on".into(), 1)]);

    sqlx::query("INSERT INTO external_import_monitor_snapshot_chunks
        (session_id, facet, entry_kind, chunk_index, payload_ndjson, created_at)
        VALUES ('external-import-monitor-apply:library-a', 'series', 'series', 0, '{}', '2026-09-26')")
        .execute(&pool).await.unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM external_import_monitor_snapshot_chunks"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1,
        "later startup must preserve a fresh migration attempt"
    );
}

#[tokio::test]
async fn postgres_legacy_monitor_snapshot_upgrade_is_transactional() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // Temporary tables shadow persistent names and disappear with this connection.
    sqlx::raw_sql(
        "CREATE TEMP TABLE external_import_monitor_snapshot_chunks (
        session_id TEXT NOT NULL, facet TEXT NOT NULL, entry_kind TEXT NOT NULL,
        chunk_index INTEGER NOT NULL, payload_ndjson TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL,
        PRIMARY KEY (session_id, facet, entry_kind, chunk_index));
        INSERT INTO external_import_monitor_snapshot_chunks VALUES
        ('external-import-monitor-apply', 'series', 'series', 0, '{}', '2026-09-25'),
        ('external-import-monitor-apply:library-a', 'movie', 'movie', 0, '{}', '2026-09-25'),
        ('source-session', 'anime', 'series', 0, '{}', '2026-09-25');
        CREATE TEMP TABLE titles (id TEXT PRIMARY KEY, monitored BOOLEAN NOT NULL);
        INSERT INTO titles VALUES ('off', FALSE), ('on', TRUE);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM external_import_monitor_snapshot_chunks"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        3
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM external_import_monitor_snapshot_chunks"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let titles: Vec<(String, bool)> =
        sqlx::query_as("SELECT id, monitored FROM titles ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(titles, vec![("off".into(), false), ("on".into(), true)]);
    pool.close().await;
}
