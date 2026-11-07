//! A restore must leave Discovery's affinity revision different from the one
//! the target held before, or a running process could keep serving affinity
//! profiles computed from the replaced catalog.

use crate::sqlite_backup::{
    export_backup_tables_from_pool, restore_backup_bundle_into_sqlite_pool,
};
use crate::{MigrationMode, TitleStore};
use scryer_application::{BackupBundleExportRequest, BackupBundleStaging, TitleRepository};
use scryer_infrastructure_datastore::SqliteServices;

async fn migrated_services(temp: &tempfile::TempDir) -> SqliteServices {
    let db_path = temp.path().join("catalog.db");
    SqliteServices::new_with_mode(
        format!("sqlite://{}", db_path.display()),
        MigrationMode::Apply,
    )
    .await
    .expect("migrate sqlite schema to head")
}

async fn seed_title(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status,
            tags, external_ids, created_at, library_id, root_folder_id)
         VALUES (?, ?, ?, 'movie', 1, 'active', '[]', '[]', '2026-01-01T00:00:00Z',
            'library-movie', 'root-fixture')",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO title_metadata_tags (title_id, tag_key, category, name)
         VALUES (?, 'canonical:genre:drama', 'genre', 'Drama')",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn revision(services: &SqliteServices) -> Option<String> {
    TitleStore::new(services.datastore())
        .discovery_affinity_revision()
        .await
        .unwrap()
}

#[tokio::test]
async fn restore_replaces_the_affinity_revision() {
    let source = tempfile::tempdir().unwrap();
    let source_services = migrated_services(&source).await;
    let source_pool = source_services.pool().clone();
    seed_title(&source_pool, "source-title").await;
    let source_revision = revision(&source_services).await;
    let mut staging = BackupBundleStaging::new().unwrap();
    export_backup_tables_from_pool(&source_pool, &mut staging)
        .await
        .unwrap();
    let bundle = source.path().join("affinity-revision.scryer");
    staging
        .finish(BackupBundleExportRequest {
            output_path: bundle.clone(),
            passphrase: "fixture-passphrase".into(),
            source_migration_key: Some("0329".into()),
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
    source_pool.close().await;

    let target = tempfile::tempdir().unwrap();
    let services = migrated_services(&target).await;
    let pool = services.pool().clone();
    seed_title(&pool, "target-title").await;
    let before = revision(&services).await;
    assert!(before.is_some(), "a migrated database keeps a revision");

    restore_backup_bundle_into_sqlite_pool(&pool, &bundle, Some("fixture-passphrase"))
        .await
        .expect("restore bundle");

    // Either the restored rows minted a fresh token or the reset left none,
    // which turns caching off; the pre-restore token never survives.
    let after = revision(&services).await;
    assert_ne!(
        after, before,
        "the restored catalog is a new affinity input"
    );
    assert!(
        after.is_none() || after != source_revision,
        "the token is never carried over from the backup"
    );
    seed_title(&pool, "after-restore-title").await;
    let rewritten = revision(&services).await;
    assert!(
        rewritten.is_some(),
        "the next input write mints a token again"
    );
    assert_ne!(rewritten, after);
}
