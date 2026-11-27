const SQLITE_MIGRATION: &str =
    include_str!("../../../scryer/src/db/migrations/0368_bad_file_reports.sql");
const POSTGRES_MIGRATION: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0368_bad_file_reports.sql");

/// The blocklist and its indexes as the migrations before this one leave them.
const PRIOR_SCHEMA: &str = "CREATE TABLE libraries (id TEXT PRIMARY KEY NOT NULL);
     CREATE TABLE titles (id TEXT PRIMARY KEY NOT NULL);
     CREATE TABLE blocklist (
         id TEXT PRIMARY KEY,
         title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
         release_name TEXT,
         reason TEXT,
         created_at TEXT NOT NULL,
         normalized_release_name TEXT NOT NULL DEFAULT '',
         indexer_id TEXT NOT NULL DEFAULT '',
         info_hash TEXT
     );
     CREATE UNIQUE INDEX idx_blocklist_release_unique
         ON blocklist (title_id, indexer_id, normalized_release_name)
         WHERE info_hash IS NULL;
     CREATE UNIQUE INDEX idx_blocklist_info_hash_unique
         ON blocklist (title_id, info_hash)
         WHERE info_hash IS NOT NULL;
     CREATE TABLE media_files (
         id TEXT PRIMARY KEY NOT NULL,
         title_id TEXT NOT NULL,
         size_bytes BIGINT NOT NULL
     );
     INSERT INTO libraries VALUES ('library');
     INSERT INTO titles VALUES ('title');
     INSERT INTO media_files VALUES ('file-a', 'title', 10), ('file-b', 'title', 10);
     INSERT INTO blocklist (id, title_id, release_name, created_at, normalized_release_name)
     VALUES ('release', 'title', 'Synthetic.Release', '2026-11-24T00:00:00Z', 'synthetic.release');";

macro_rules! verify_bad_file_reports_upgrade {
    ($connection:expr, $migration:expr) => {{
        let connection = &mut $connection;
        sqlx::raw_sql(PRIOR_SCHEMA)
            .execute(&mut **connection)
            .await
            .unwrap();
        sqlx::raw_sql($migration)
            .execute(&mut **connection)
            .await
            .unwrap();

        // A second row for a named release is still refused.
        let duplicate_release = sqlx::raw_sql(
            "INSERT INTO blocklist (id, title_id, release_name, created_at, normalized_release_name)
             VALUES ('release-2', 'title', 'Synthetic.Release', '2026-11-24T00:00:00Z',
                     'synthetic.release')
             ON CONFLICT DO NOTHING",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        assert_eq!(duplicate_release.rows_affected(), 0);

        // Two content-only rows on one title do not collide; a repeated
        // content identity does.
        let content_rows = sqlx::raw_sql(
            "INSERT INTO blocklist
                (id, title_id, created_at, content_scheme, content_size_bytes,
                 content_sample_bytes, content_sample_blake3)
             VALUES ('content-a', 'title', '2026-11-24T00:00:00Z', 'v1', 10, 10, 'aaa'),
                    ('content-b', 'title', '2026-11-24T00:00:00Z', 'v1', 10, 10, 'bbb')
             ON CONFLICT DO NOTHING",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        assert_eq!(content_rows.rows_affected(), 2);
        let repeated = sqlx::raw_sql(
            "INSERT INTO blocklist
                (id, title_id, created_at, content_scheme, content_size_bytes,
                 content_sample_bytes, content_sample_blake3)
             VALUES ('content-c', 'title', '2026-11-24T00:00:00Z', 'v1', 10, 10, 'aaa')
             ON CONFLICT DO NOTHING",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        assert_eq!(repeated.rows_affected(), 0);

        // The existing release row can carry a content identity as well.
        sqlx::raw_sql(
            "UPDATE blocklist
                SET content_scheme = 'v1', content_size_bytes = 10,
                    content_sample_bytes = 10, content_sample_blake3 = 'ccc'
              WHERE id = 'release'",
        )
        .execute(&mut **connection)
        .await
        .unwrap();

        // Deleting a blocklist row clears the stamp on its files only.
        sqlx::raw_sql(
            "UPDATE media_files SET bad_file_blocklist_id = 'content-a' WHERE id = 'file-a';
             UPDATE media_files SET bad_file_blocklist_id = 'content-b' WHERE id = 'file-b';
             DELETE FROM blocklist WHERE id = 'content-a';",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        let stamps: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, bad_file_blocklist_id FROM media_files ORDER BY id",
        )
        .fetch_all(&mut **connection)
        .await
        .unwrap();
        assert_eq!(
            stamps,
            [
                ("file-a".to_string(), None),
                ("file-b".to_string(), Some("content-b".to_string())),
            ]
        );

        sqlx::raw_sql(
            "INSERT INTO media_issues
                (id, library_id, title_id, media_file_id, file_path_snapshot, file_version,
                 content_scheme, size_bytes, sample_bytes, sample_blake3, reporter_user_id,
                 reason, remediation, status, execution_status, blocklist_id,
                 created_at, updated_at)
             VALUES
                ('issue', 'library', 'title', 'file-b', 'synthetic.mkv', 'version', 'v1', 10,
                 10, 'bbb', 'reporter', 'corrupted', 'mark_bad', 'approved', 'blocked',
                 'content-b', '2026-11-24T00:00:00Z', '2026-11-24T00:00:00Z');
             DELETE FROM blocklist WHERE id = 'content-b';",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        let report: (String, Option<String>) =
            sqlx::query_as("SELECT status, blocklist_id FROM media_issues WHERE id = 'issue'")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
        assert_eq!(report, ("approved".to_string(), None));

        let unknown_reason = sqlx::raw_sql(
            "INSERT INTO media_issues
                (id, library_id, title_id, media_file_id, file_path_snapshot, file_version,
                 content_scheme, size_bytes, sample_bytes, sample_blake3, reporter_user_id,
                 reason, remediation, status, execution_status, created_at, updated_at)
             VALUES
                ('issue-2', 'library', 'title', 'file-a', 'synthetic.mkv', 'version', 'v1', 10,
                 10, 'aaa', 'reporter', 'bad_media', 'mark_bad', 'pending', 'none',
                 '2026-11-24T00:00:00Z', '2026-11-24T00:00:00Z')",
        )
        .execute(&mut **connection)
        .await;
        assert!(unknown_reason.is_err());
    }};
}

#[tokio::test]
async fn bad_file_reports_upgrade_sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    verify_bad_file_reports_upgrade!(transaction, SQLITE_MIGRATION);

    // The clear path and the import gate's read are served by the new indexes.
    for (sql, index) in [
        (
            "UPDATE media_files SET bad_file_blocklist_id = NULL \
             WHERE bad_file_blocklist_id = 'x'",
            "idx_media_files_bad_file_blocklist",
        ),
        (
            "SELECT id FROM blocklist \
             WHERE title_id = 'title' AND content_sample_blake3 IS NOT NULL",
            "idx_blocklist_content_identity_unique",
        ),
        (
            "SELECT id FROM media_issues WHERE blocklist_id = 'x'",
            "idx_media_issues_blocklist",
        ),
    ] {
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
                .fetch_all(&mut *transaction)
                .await
                .unwrap();
        assert!(
            rows.iter().any(|row| row.3.contains(index)),
            "{sql} does not use {index}: {rows:?}"
        );
    }
    transaction.rollback().await.unwrap();
}

#[tokio::test]
async fn bad_file_reports_upgrade_postgres() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "CREATE SCHEMA bad_file_reports_test;
         SET LOCAL search_path TO bad_file_reports_test;",
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    verify_bad_file_reports_upgrade!(transaction, POSTGRES_MIGRATION);
    transaction.rollback().await.unwrap();
}
