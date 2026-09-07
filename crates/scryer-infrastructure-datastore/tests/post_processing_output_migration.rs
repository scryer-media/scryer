use std::sync::Arc;

use scryer_infrastructure_datastore::migrations::post_processing_output::compress_post_processing_output_sqlite;
use scryer_infrastructure_sql::script_output::decode_script_output_tail;
use sqlx::{Row, sqlite::SqlitePoolOptions};

const OLD: &str = include_str!("../../scryer/src/db/migrations/0051_post_processing_scripts.sql");
const PRE: &str =
    include_str!("../../scryer/src/db/migrations/0210_compress_post_processing_output_pre.sql");
const POST: &str =
    include_str!("../../scryer/src/db/migrations/0210_compress_post_processing_output_post.sql");

#[tokio::test]
async fn upgrade_preserves_output_and_rolls_back_atomically() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(OLD).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO post_processing_scripts (id, name, created_at, updated_at) VALUES ('script', 'script', '2026-09-01', '2026-09-01')")
        .execute(&pool)
        .await
        .unwrap();
    for index in 0..1001 {
        let output = (index % 3 != 0).then(|| format!("{index}:日本語\n").repeat(100));
        sqlx::query("INSERT INTO post_processing_script_runs (id, script_id, script_name, title_name, status, stdout_tail, stderr_tail, started_at) VALUES (?, 'script', 'name', 'title', 'success', ?, '', '2026-09-01')")
            .bind(format!("run-{index:04}"))
            .bind(output)
            .execute(&pool)
            .await
            .unwrap();
    }
    for commit in [false, true] {
        let mut transaction = pool.begin().await.unwrap();
        sqlx::raw_sql(PRE).execute(&mut *transaction).await.unwrap();
        compress_post_processing_output_sqlite(&mut transaction)
            .await
            .unwrap();
        sqlx::raw_sql(POST)
            .execute(&mut *transaction)
            .await
            .unwrap();
        let rows = sqlx::query(
            "SELECT id, stdout_tail, stderr_tail FROM post_processing_script_runs ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1001);
        for (index, row) in rows.iter().enumerate() {
            let stdout = row.try_get::<Option<Vec<u8>>, _>("stdout_tail").unwrap();
            assert_eq!(
                stdout
                    .as_deref()
                    .map(|value| decode_script_output_tail(value).unwrap()),
                (index % 3 != 0).then(|| format!("{index}:日本語\n").repeat(100))
            );
            assert_eq!(
                decode_script_output_tail(&row.get::<Vec<u8>, _>("stderr_tail")).unwrap(),
                ""
            );
        }
        if commit {
            transaction.commit().await.unwrap();
        } else {
            transaction.rollback().await.unwrap();
            let kind: String = sqlx::query_scalar(
                "SELECT typeof(stdout_tail) FROM post_processing_script_runs WHERE id = 'run-0001'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(kind, "text");
        }
    }
}

#[tokio::test]
async fn repository_reads_compressed_and_legacy_restored_output() {
    use scryer_application::PostProcessingScriptRepository;
    use scryer_infrastructure_configuration::customization::post_processing_script_store::PostProcessingScriptStore;
    use scryer_infrastructure_sql::runtime::StoreDatastore;

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(OLD).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO post_processing_scripts (id, name, created_at, updated_at) VALUES ('script', 'script', '2026-09-01', '2026-09-01')")
        .execute(&pool)
        .await
        .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::raw_sql(PRE).execute(&mut *transaction).await.unwrap();
    compress_post_processing_output_sqlite(&mut transaction)
        .await
        .unwrap();
    sqlx::raw_sql(POST)
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let store = PostProcessingScriptStore::new(StoreDatastore::Sqlite {
        pool: pool.clone(),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    });
    let output = "尾\n".repeat(8192);
    store
        .record_run(scryer_domain::PostProcessingScriptRun {
            id: "new".into(),
            script_id: "script".into(),
            script_name: "script".into(),
            title_id: None,
            title_name: None,
            facet: None,
            file_path: None,
            status: scryer_domain::ScriptRunStatus::Success,
            exit_code: Some(0),
            stdout_tail: Some(output.clone()),
            stderr_tail: None,
            duration_ms: Some(1),
            env_payload_json: None,
            started_at: "2026-09-01T00:00:00Z".into(),
            completed_at: None,
        })
        .await
        .unwrap();
    assert_eq!(
        store.list_runs_for_script("script", 10).await.unwrap()[0]
            .stdout_tail
            .as_deref(),
        Some(output.as_str())
    );
    sqlx::query("UPDATE post_processing_script_runs SET stdout_tail = ? WHERE id = 'new'")
        .bind("legacy 日本語")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.list_runs_for_script("script", 10).await.unwrap()[0]
            .stdout_tail
            .as_deref(),
        Some("legacy 日本語")
    );
}
