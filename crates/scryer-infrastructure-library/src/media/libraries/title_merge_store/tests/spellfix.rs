use super::*;
use scryer_application::TitleRepository;
use scryer_infrastructure_library_search::replace_title_search_projection_tx;
use sqlx::SqlitePool;

fn pool(datastore: &StoreDatastore) -> &SqlitePool {
    let StoreDatastore::Sqlite { pool, .. } = datastore else {
        panic!("SQLite fixture required")
    };
    pool
}

async fn vocabulary(datastore: &StoreDatastore) -> Vec<(i64, String, i64, i64)> {
    sqlx::query_as("SELECT rowid, word, rank, langid FROM title_search_spellfix ORDER BY rowid")
        .fetch_all(pool(datastore))
        .await
        .unwrap()
}

async fn seed_search(datastore: &StoreDatastore) {
    insert_title(datastore, SOURCE, "library-a", &[]).await;
    insert_title(datastore, DESTINATION, "library-b", &[]).await;
    run(datastore,
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ('library-a', 'series', 'A', 'a', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![]).await;
    let titles = crate::media::titles::store::TitleStore::new(datastore.clone());
    // The retired source owns the low IDs. Rebuilding the survivor will reuse
    // them, reproducing the real merge-then-transfer failure.
    for id in [SOURCE, DESTINATION] {
        let title = titles.get_by_id(id).await.unwrap().unwrap();
        let mut tx = pool(datastore).begin().await.unwrap();
        replace_title_search_projection_tx(&mut tx, &title)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    assert!(!vocabulary(datastore).await.is_empty());
}

async fn transfer_survivor(datastore: &StoreDatastore) -> AppResult<()> {
    crate::media::titles::store::TitleStore::new(datastore.clone())
        .transfer_to_library(DESTINATION, "library-a", "root-library-a", None, &[])
        .await
}

#[tokio::test]
async fn spellfix_cleanup_follows_merge_rollback_retry_and_title_cascade() {
    let (store, datastore) = test_store().await;
    seed_search(&datastore).await;
    let before = vocabulary(&datastore).await;
    let plan = plan_for(&store).await;
    run(
        &datastore,
        "CREATE TRIGGER fail_title_retirement BEFORE DELETE ON titles
         BEGIN SELECT RAISE(ABORT, 'injected retirement failure'); END",
        vec![],
    )
    .await;
    store.execute_title_merge(&plan).await.unwrap_err();
    assert_eq!(
        vocabulary(&datastore).await,
        before,
        "cleanup must roll back with the merge"
    );
    run(&datastore, "DROP TRIGGER fail_title_retirement", vec![]).await;

    store.execute_title_merge(&plan).await.unwrap();
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM title_search_spellfix
         WHERE rowid NOT IN (SELECT term_id FROM title_search_terms)",
            vec![]
        )
        .await,
        0
    );
    transfer_survivor(&datastore).await.unwrap();
    transfer_survivor(&datastore).await.unwrap();
    assert_eq!(
        text(
            &datastore,
            "SELECT library_id AS value FROM titles WHERE id = {}",
            vec![SqlArg::Text(DESTINATION.into())]
        )
        .await
        .as_deref(),
        Some("library-a")
    );
    assert!(!vocabulary(&datastore).await.is_empty());

    // Deletions that rely on ON DELETE CASCADE must clean the vocabulary too.
    run(
        &datastore,
        "DELETE FROM titles WHERE id = {}",
        vec![SqlArg::Text(DESTINATION.into())],
    )
    .await;
    assert!(vocabulary(&datastore).await.is_empty());
}

#[tokio::test]
async fn spellfix_migration_repairs_legacy_merge_orphans_and_allows_transfer_retry() {
    let (store, datastore) = test_store().await;
    seed_search(&datastore).await;
    // Recreate the pre-migration schema and execute the actual faulty merge.
    run(
        &datastore,
        "DROP TRIGGER title_search_terms_delete_spellfix",
        vec![],
    )
    .await;
    let plan = plan_for(&store).await;
    store.execute_title_merge(&plan).await.unwrap();
    let orphan_count = scalar(
        &datastore,
        "SELECT COUNT(*) AS row_count FROM title_search_spellfix
         WHERE rowid NOT IN (SELECT term_id FROM title_search_terms)",
        vec![],
    )
    .await;
    assert!(orphan_count > 0);
    let error = transfer_survivor(&datastore).await.unwrap_err();
    assert!(
        error.to_string().contains("1555"),
        "expected the original collision: {error}"
    );
    assert_eq!(
        text(
            &datastore,
            "SELECT library_id AS value FROM titles WHERE id = {}",
            vec![SqlArg::Text(DESTINATION.into())]
        )
        .await
        .as_deref(),
        Some("library-b")
    );
    let surviving_entries: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT rowid, word, rank, langid FROM title_search_spellfix
         WHERE rowid IN (SELECT term_id FROM title_search_terms) ORDER BY rowid",
    )
    .fetch_all(pool(&datastore))
    .await
    .unwrap();

    let mut tx = pool(&datastore).begin().await.unwrap();
    sqlx::raw_sql(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scryer/src/db/migrations/0236_title_search_spellfix_cleanup.sql"
    )))
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        vocabulary(&datastore).await,
        surviving_entries,
        "repair must preserve the survivor's words, ranks, language IDs and row IDs"
    );
    transfer_survivor(&datastore).await.unwrap();
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM title_search_terms AS terms
         LEFT JOIN title_search_spellfix AS spellfix ON spellfix.rowid = terms.term_id
         WHERE spellfix.rowid IS NULL OR spellfix.word <> terms.normalized_term",
            vec![]
        )
        .await,
        0
    );
}
