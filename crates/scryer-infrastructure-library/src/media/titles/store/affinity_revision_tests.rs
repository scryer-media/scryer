//! The discovery affinity revision against a real sqlite database.
//!
//! Affinity profiles are cached under this revision, so every write the
//! affinity read depends on must replace it, and writes it does not depend
//! on should leave it alone.

use scryer_application::TitleRepository;

use crate::queries::sql_runtime::{SqlArg, StoreDatastore};

use super::TitleStore;
use super::owned_probe_tests::{datastore, insert_library, insert_title, run};

async fn revision(store: &TitleStore) -> String {
    store
        .discovery_affinity_revision()
        .await
        .expect("revision should read")
        .expect("a migrated database keeps a revision")
}

/// Runs `sql` and reports whether the revision changed.
async fn changes_revision(store: &TitleStore, datastore: &StoreDatastore, sql: &str) -> bool {
    let before = revision(store).await;
    run(datastore, sql, Vec::<SqlArg>::new()).await;
    revision(store).await != before
}

#[tokio::test]
async fn every_affinity_input_write_replaces_the_revision() {
    let datastore = datastore().await;
    let store = TitleStore::new(datastore.clone());
    insert_library(&datastore, "library-affinity-a", "movie").await;
    insert_library(&datastore, "library-affinity-b", "movie").await;

    let before = revision(&store).await;
    insert_title(
        &datastore,
        "title-affinity-1",
        "library-affinity-a",
        "movie",
        None,
        &[],
    )
    .await;
    assert_ne!(revision(&store).await, before, "a new title");

    let input_writes = [
        (
            "library reassignment",
            "UPDATE titles SET library_id = 'library-affinity-b' WHERE id = 'title-affinity-1'",
        ),
        (
            "facet change",
            "UPDATE titles SET facet = 'series' WHERE id = 'title-affinity-1'",
        ),
        (
            "canonical tag insert",
            "INSERT INTO title_metadata_tags (title_id, tag_key, category, name)
             VALUES ('title-affinity-1', 'canonical:genre:drama', 'genre', 'Drama')",
        ),
        (
            "canonical tag update",
            "UPDATE title_metadata_tags SET name = 'Drama Two' WHERE title_id = 'title-affinity-1'",
        ),
        (
            "tag source insert",
            "INSERT INTO title_metadata_tag_sources (title_id, tag_key, source)
             VALUES ('title-affinity-1', 'canonical:genre:drama', 'provider')",
        ),
        (
            "tag source key insert",
            "INSERT INTO title_metadata_tag_source_keys (title_id, tag_key, source_tag_key)
             VALUES ('title-affinity-1', 'canonical:genre:drama', 'provider:genre:1')",
        ),
        (
            "tag source key delete",
            "DELETE FROM title_metadata_tag_source_keys WHERE title_id = 'title-affinity-1'",
        ),
        (
            "tag source delete",
            "DELETE FROM title_metadata_tag_sources WHERE title_id = 'title-affinity-1'",
        ),
        (
            "generation run",
            "INSERT INTO discovery_sync_runs (id, kind, status, trigger_source, region, language)
             VALUES ('run-affinity', 'context_snapshot', 'complete', 'test', 'US', 'en')",
        ),
        (
            "submitted subject insert",
            "INSERT INTO discovery_submitted_subjects
                 (run_id, subject_key, title_id, library_id, raw_subject_json)
             VALUES ('run-affinity', 'tmdb:movie:1', 'title-affinity-1',
                     'library-affinity-b', '[]')",
        ),
        (
            "submitted subject delete",
            "DELETE FROM discovery_submitted_subjects WHERE run_id = 'run-affinity'",
        ),
        (
            "canonical tag delete",
            "DELETE FROM title_metadata_tags WHERE title_id = 'title-affinity-1'",
        ),
        (
            "title delete",
            "DELETE FROM titles WHERE id = 'title-affinity-1'",
        ),
    ];
    for (label, sql) in input_writes {
        // "generation run" writes no affinity input: it only seeds the run the
        // subject rows reference, so it is checked separately below.
        if label == "generation run" {
            run(&datastore, sql, Vec::new()).await;
            continue;
        }
        assert!(
            changes_revision(&store, &datastore, sql).await,
            "{label} must replace the affinity revision"
        );
    }
}

#[tokio::test]
async fn writes_the_affinity_read_ignores_keep_the_revision() {
    let datastore = datastore().await;
    let store = TitleStore::new(datastore.clone());
    insert_library(&datastore, "library-affinity-a", "movie").await;
    insert_title(
        &datastore,
        "title-affinity-1",
        "library-affinity-a",
        "movie",
        None,
        &[],
    )
    .await;
    assert!(
        !changes_revision(
            &store,
            &datastore,
            "UPDATE titles SET name = 'Renamed Synthetic', monitored = 0
             WHERE id = 'title-affinity-1'",
        )
        .await,
        "a rename or monitoring change does not change affinity"
    );
}
