//! A restore drops every `ResetOnRestore` table but keeps `titles` — including
//! each title's `metadata_fetched_at`. State that only title hydration writes
//! must therefore be rebuilt by scheduling hydration, or a restored title looks
//! freshly hydrated while its derived rows stay missing until the metadata
//! refresh interval lapses (weeks for an ended series).

use crate::sqlite_backup::{
    export_backup_tables_from_pool, restore_backup_bundle_into_sqlite_pool,
};
use crate::{MigrationMode, TitleStore};
use scryer_application::{BackupBundleExportRequest, BackupBundleStaging, TitleRepository};
use scryer_domain::MediaFacet;
use scryer_infrastructure_datastore::SqliteServices;

const FETCHED_AT: &str = "2026-01-02T03:04:05Z";

async fn migrated_services(temp: &tempfile::TempDir) -> SqliteServices {
    let db_path = temp.path().join("catalog.db");
    SqliteServices::new_with_mode(
        format!("sqlite://{}", db_path.display()),
        MigrationMode::Apply,
    )
    .await
    .expect("migrate sqlite schema to head")
}

async fn seed_hydrated_title(pool: &sqlx::SqlitePool, id: &str, facet: &str, library_id: &str) {
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status,
            tags, external_ids, created_at, library_id, root_folder_id,
            metadata_fetched_at, metadata_hydration_next_attempt_at,
            metadata_hydration_attempt_count)
         VALUES (?, ?, ?, ?, 1, 'active', '[]', '[{\"source\":\"tvdb\",\"value\":\"4242\"}]',
            '2026-01-01T00:00:00Z', ?, 'root-fixture', ?, NULL, 0)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(facet)
    .bind(library_id)
    .bind(FETCHED_AT)
    .execute(pool)
    .await
    .unwrap();
}

async fn bridge_row_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM title_anime_numbering_bridges")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn backup_of_hydrated_anime_and_series() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let services = migrated_services(&temp).await;
    let pool = services.pool().clone();
    seed_hydrated_title(&pool, "anime-title", "anime", "library-anime").await;
    seed_hydrated_title(&pool, "series-title", "series", "library-series").await;
    sqlx::query(
        "INSERT INTO title_anime_numbering_bridges
            (title_id, generated_on, corroborating_order, seasons_json, updated_at)
         VALUES ('anime-title', '2026-01-02', NULL,
            '[{\"index\":1,\"anidb_id\":1,\"anilist_id\":null,\"mal_id\":null,\"titles\":[\"Cour 1\"],\"ranges\":[],\"absolute_start\":1,\"episode_count\":12}]',
            '2026-01-02T03:04:05Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    crate::queries::title_search::rebuild_title_search_projection(&pool)
        .await
        .unwrap();
    assert_eq!(
        bridge_row_count(&pool).await,
        1,
        "fixture must carry a bridge"
    );

    let mut staging = BackupBundleStaging::new().unwrap();
    export_backup_tables_from_pool(&pool, &mut staging)
        .await
        .unwrap();
    let bundle = temp.path().join("hydration-derived-reset.scryer");
    staging
        .finish(BackupBundleExportRequest {
            output_path: bundle.clone(),
            passphrase: "fixture-passphrase".into(),
            source_migration_key: Some("0236".into()),
            source_scryer_version: "test".into(),
            source_engine: "sqlite".into(),
            secrets: scryer_application::BackupExportSecrets {
                encryption_master_key: "fixture-master-key".into(),
                jwt_signing_secret: Some("fixture-jwt-key".into()),
                smg_registration_secret: None,
                smg_gateway_url: None,
            },
        })
        .unwrap();
    pool.close().await;
    (temp, bundle)
}

#[tokio::test]
async fn restore_schedules_hydration_for_titles_whose_numbering_bridge_was_reset() {
    let (_source, bundle) = backup_of_hydrated_anime_and_series().await;
    let target = tempfile::tempdir().unwrap();
    let services = migrated_services(&target).await;
    let pool = services.pool().clone();

    restore_backup_bundle_into_sqlite_pool(&pool, &bundle, Some("fixture-passphrase"))
        .await
        .expect("restore bundle");

    assert_eq!(
        bridge_row_count(&pool).await,
        0,
        "the numbering bridge is ResetOnRestore, so the restore must not carry it"
    );

    let titles = TitleStore::new(services.datastore());
    let due_for_user_refresh = titles
        .list_title_ids_with_metadata_hydration_due(
            Some(MediaFacet::Anime),
            &["library-anime".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        due_for_user_refresh,
        ["anime-title"],
        "a restored anime title lost its bridge, so a user refresh must re-hydrate it"
    );

    let due_in_background = titles
        .list_titles_due_for_hydration(10, &[])
        .await
        .unwrap()
        .into_iter()
        .map(|pending| pending.title.id)
        .collect::<Vec<_>>();
    assert_eq!(
        due_in_background,
        ["anime-title"],
        "background hydration must rebuild the bridge; a series title lost nothing \
         hydration-only and must not be re-hydrated by a restore"
    );

    // The title keeps its hydrated marker: until the rebuild lands it still
    // searches and imports under plain TVDB numbering, exactly as the bridge
    // table documents for a missing row.
    let fetched_at: Option<String> =
        sqlx::query_scalar("SELECT metadata_fetched_at FROM titles WHERE id = 'anime-title'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fetched_at.as_deref(), Some(FETCHED_AT));

    pool.close().await;
}
