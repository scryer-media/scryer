const SQLITE_BLOCKS: &str =
    include_str!("../../../scryer/src/db/migrations/0345_media_content_blocks.sql");
const POSTGRES_BLOCKS: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0345_media_content_blocks.sql");
const SQLITE_RULES: &str = include_str!(
    "../../../scryer/src/db/migrations/0346_request_rule_media_issue_applicability.sql"
);
const POSTGRES_RULES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0346_request_rule_media_issue_applicability.sql"
);

// Both engines run the same assertions against an old rule and a synthetic
// catalog. PostgreSQL uses a rolled-back schema so parallel tests are isolated.
macro_rules! verify_media_content_blocks_upgrade {
    ($connection:expr, $blocks:expr, $rules:expr) => {{
        let connection = &mut $connection;
        sqlx::raw_sql(
            "CREATE TABLE libraries (id TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE titles (id TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE media_files (
                 id TEXT PRIMARY KEY NOT NULL, title_id TEXT NOT NULL, size_bytes BIGINT NOT NULL
             );
             CREATE TABLE request_rule_sets (id TEXT PRIMARY KEY NOT NULL);
             INSERT INTO libraries VALUES ('a'), ('b');
             INSERT INTO titles VALUES ('title');
             INSERT INTO media_files VALUES ('file', 'title', 10);
             INSERT INTO request_rule_sets VALUES ('old');",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        sqlx::raw_sql($blocks)
            .execute(&mut **connection)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO media_issues
                (id, library_id, title_id, media_file_id, file_path_snapshot, file_version,
                 size_bytes, scheme, sample_bytes, sample_blake3, reporter_user_id, reason_code,
                 remediation, status, execution_status, created_at, updated_at)
             VALUES
                ('issue','a','title','file','test.mkv','version',10,'v1',10,'abc','reporter',
                 'bad_media','mark_bad','pending','none','2026-11-24','2026-11-24')",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        sqlx::raw_sql($rules)
            .execute(&mut **connection)
            .await
            .unwrap();

        let applicability: String =
            sqlx::query_scalar("SELECT applicability FROM request_rule_sets WHERE id = 'old'")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
        assert_eq!(applicability, "acquisition");
        let authorized: Option<String> = sqlx::query_scalar(
            "SELECT authorized_remediation FROM media_issues WHERE id = 'issue'",
        )
        .fetch_one(&mut **connection)
        .await
        .unwrap();
        assert!(authorized.is_none());

        sqlx::raw_sql(
            "INSERT INTO library_content_blocks
                (id, library_id, scheme, size_bytes, sample_bytes, sample_blake3, created_at)
             VALUES ('block-a', 'a', 'v1', 10, 10, 'abc', '2026-11-24'),
                    ('block-b', 'b', 'v1', 10, 10, 'abc', '2026-11-24'),
                    ('duplicate', 'b', 'v1', 10, 10, 'abc', '2026-11-24')
             ON CONFLICT (library_id, scheme, size_bytes, sample_bytes, sample_blake3) DO NOTHING;
             UPDATE media_files SET content_block_id = 'block-a' WHERE id = 'file';
             DELETE FROM library_content_blocks WHERE id = 'block-a';",
        )
        .execute(&mut **connection)
        .await
        .unwrap();
        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT id FROM library_content_blocks ORDER BY id")
                .fetch_all(&mut **connection)
                .await
                .unwrap();
        assert_eq!(remaining, ["block-b"]);
        let file_block: Option<String> =
            sqlx::query_scalar("SELECT content_block_id FROM media_files WHERE id = 'file'")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
        assert!(file_block.is_none());
    }};
}

#[tokio::test]
async fn media_content_blocks_upgrade_sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    verify_media_content_blocks_upgrade!(transaction, SQLITE_BLOCKS, SQLITE_RULES);
    transaction.rollback().await.unwrap();
}

#[tokio::test]
async fn media_content_blocks_upgrade_postgres() {
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
        "CREATE SCHEMA media_content_blocks_test;
         SET LOCAL search_path TO media_content_blocks_test;",
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    verify_media_content_blocks_upgrade!(transaction, POSTGRES_BLOCKS, POSTGRES_RULES);
    transaction.rollback().await.unwrap();
}
