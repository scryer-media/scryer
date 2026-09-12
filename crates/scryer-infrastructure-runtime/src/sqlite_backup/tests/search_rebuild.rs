use super::migrated_sqlite_pool;
use crate::sqlite_backup::{
    db_connect_options, export_backup_tables_from_pool, restore_backup_bundle_into_sqlite_pool,
};
use scryer_application::{BackupBundleExportRequest, BackupBundleStaging};
use sqlx::sqlite::SqlitePoolOptions;

async fn seed_title(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status,
            tags, external_ids, created_at, library_id, root_folder_id)
         VALUES (?, ?, ?, 'series', 1, 'active', '[]', '[]',
            '2026-01-01T00:00:00Z', 'library-fixture', 'root-fixture')",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    crate::queries::title_search::rebuild_title_search_projection(pool)
        .await
        .unwrap();
}

async fn backup_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let (pool, temp) = migrated_sqlite_pool().await;
    seed_title(&pool, "restored-title").await;
    let mut staging = BackupBundleStaging::new().unwrap();
    export_backup_tables_from_pool(&pool, &mut staging)
        .await
        .unwrap();
    let bundle = temp.path().join("restore-test.scryer");
    staging
        .finish(BackupBundleExportRequest {
            output_path: bundle.clone(),
            passphrase: "fixture-passphrase".into(),
            source_migration_key: Some("0236".into()),
            source_scryer_version: "test".into(),
            source_engine: "sqlite".into(),
            secrets: scryer_application::BackupExportSecrets {
                encryption_master_key: "fixture-master-key".into(),
                jwt_signing_secret: "fixture-jwt-key".into(),
                smg_registration_secret: None,
                smg_gateway_url: None,
            },
        })
        .unwrap();
    pool.close().await;
    (temp, bundle)
}

async fn indexed_titles(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT DISTINCT terms.title_id FROM title_search_terms terms
         JOIN title_search_spellfix spellfix ON spellfix.rowid = terms.term_id
         WHERE spellfix.word = terms.normalized_term ORDER BY terms.title_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn restore_search_rebuild_failure_rolls_back_catalog_and_index_together() {
    let (_source, bundle) = backup_fixture().await;
    let (pool, _target) = migrated_sqlite_pool().await;
    seed_title(&pool, "original-title").await;
    sqlx::query(
        "CREATE TRIGGER fail_search_rebuild BEFORE INSERT ON title_search_terms
        BEGIN SELECT RAISE(ABORT, 'injected search rebuild failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let error = restore_backup_bundle_into_sqlite_pool(&pool, &bundle, Some("fixture-passphrase"))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected search rebuild failure"),
        "{error}"
    );
    let titles: Vec<String> = sqlx::query_scalar("SELECT id FROM titles ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        titles,
        ["original-title"],
        "failed index rebuild must roll back the catalog too"
    );
    assert_eq!(indexed_titles(&pool).await, ["original-title"]);
    sqlx::query("DROP TRIGGER fail_search_rebuild")
        .execute(&pool)
        .await
        .unwrap();
    restore_backup_bundle_into_sqlite_pool(&pool, &bundle, Some("fixture-passphrase"))
        .await
        .unwrap();
    assert_eq!(indexed_titles(&pool).await, ["restored-title"]);
    pool.close().await;
}

#[tokio::test]
async fn restore_search_rebuild_uses_the_existing_connection() {
    let (_source, bundle) = backup_fixture().await;
    let (original_pool, target) = migrated_sqlite_pool().await;
    original_pool.close().await;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(250))
        .connect_with(
            db_connect_options(target.path().join("catalog-check.db").to_str().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    restore_backup_bundle_into_sqlite_pool(&pool, &bundle, Some("fixture-passphrase"))
        .await
        .expect("restore must not acquire a second connection while holding the only one");
    assert_eq!(indexed_titles(&pool).await, ["restored-title"]);
    pool.close().await;
}
