//! The one write path for title folders, run against a real sqlite database
//! with the full migration set, so the checks, the key column and the unique
//! index are the ones that ship.

use scryer_application::error_reasons::folder_ownership;
use scryer_application::{AppError, TitleFolderWrite, TitleRepository};
use sqlx::sqlite::SqlitePoolOptions;

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::TitleStore;

async fn store_with_titles(ids: &[&str]) -> (TitleStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .expect("fresh migrations should apply");
    let datastore = StoreDatastore::Sqlite {
        pool: pool.clone(),
        writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    };
    SqlRuntime::execute_write(
        &datastore,
        "folder_write_fixture",
        "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES ('root-fixture', 'movie_default_library', '/srv/fixture-root',
                 '/srv/fixture-root', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await
    .expect("fixture root should insert");
    for id in ids {
        SqlRuntime::execute_write(
            &datastore,
            "folder_write_fixture",
            "INSERT INTO titles
                 (id, library_id, name, facet, monitored, root_folder_id, created_at,
                  updated_at)
             VALUES ({}, 'movie_default_library', {}, 'movie', 1, 'root-fixture',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            vec![SqlArg::Text(id.to_string()), SqlArg::Text(id.to_string())],
        )
        .await
        .expect("fixture title should insert");
    }
    (TitleStore::new(datastore), pool)
}

async fn folder_of(pool: &sqlx::SqlitePool, id: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as("SELECT folder_path, folder_path_key FROM titles WHERE id = ?1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn reason(error: AppError) -> String {
    error
        .validation_reason()
        .map(str::to_string)
        .unwrap_or_else(|| panic!("expected a coded refusal, got {error:?}"))
}

#[tokio::test]
async fn equal_folder_is_refused_with_its_code() {
    let (store, pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/media/Emberfall")])
        .await
        .unwrap();
    let error = store
        .assign_title_folders(&[TitleFolderWrite::assign("cinderwake", "/media/Emberfall")])
        .await
        .unwrap_err();
    assert_eq!(reason(error), folder_ownership::FOLDER_ALREADY_OWNED_TITLE);
    assert_eq!(folder_of(&pool, "cinderwake").await, (None, None));
}

#[tokio::test]
async fn folders_nested_either_way_are_refused() {
    let (store, _pool) = store_with_titles(&["emberfall", "inner", "outer"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/media/Emberfall")])
        .await
        .unwrap();
    let inner = store
        .assign_title_folders(&[TitleFolderWrite::assign(
            "inner",
            "/media/Emberfall/Season 1",
        )])
        .await
        .unwrap_err();
    assert_eq!(
        reason(inner),
        folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
    );
    let outer = store
        .assign_title_folders(&[TitleFolderWrite::assign("outer", "/media")])
        .await
        .unwrap_err();
    assert_eq!(
        reason(outer),
        folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
    );
}

#[tokio::test]
async fn windows_folders_differing_in_case_are_the_same_folder() {
    let (store, _pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", r"D:\Media\Emberfall")])
        .await
        .unwrap();
    let error = store
        .assign_title_folders(&[TitleFolderWrite::assign(
            "cinderwake",
            r"d:\media\EMBERFALL",
        )])
        .await
        .unwrap_err();
    assert_eq!(reason(error), folder_ownership::FOLDER_ALREADY_OWNED_TITLE);
}

#[tokio::test]
async fn posix_folders_differing_in_case_are_different_folders() {
    let (store, pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/media/Show")])
        .await
        .unwrap();
    store
        .assign_title_folders(&[TitleFolderWrite::assign("cinderwake", "/media/show")])
        .await
        .unwrap();
    assert_eq!(
        folder_of(&pool, "cinderwake").await,
        (Some("/media/show".into()), Some("/media/show".into()))
    );
}

#[tokio::test]
async fn trailing_separator_and_unicode_variants_are_the_same_folder() {
    let (store, _pool) = store_with_titles(&["composed", "trailing", "decomposed"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("composed", "/media/Caf\u{e9}")])
        .await
        .unwrap();
    for (id, folder) in [
        ("trailing", "/media/Caf\u{e9}/"),
        ("decomposed", "/media/Cafe\u{301}"),
    ] {
        let error = store
            .assign_title_folders(&[TitleFolderWrite::assign(id, folder)])
            .await
            .unwrap_err();
        assert_eq!(
            reason(error),
            folder_ownership::FOLDER_ALREADY_OWNED_TITLE,
            "{folder}"
        );
    }
}

#[tokio::test]
async fn a_library_root_is_never_a_title_folder() {
    let (store, pool) = store_with_titles(&["emberfall"]).await;
    sqlx::query(
        "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES ('root-emberfall', 'movie_default_library', '/srv/films', '/srv/films', 0,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let error = store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/srv/films/")])
        .await
        .unwrap_err();
    assert_eq!(
        reason(error),
        folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER
    );
}

#[tokio::test]
async fn a_stale_expected_folder_is_refused() {
    let (store, pool) = store_with_titles(&["emberfall"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/media/Emberfall")])
        .await
        .unwrap();
    let error = store
        .assign_title_folders(&[
            TitleFolderWrite::assign("emberfall", "/media/Emberfall (2031)").expecting(None),
        ])
        .await
        .unwrap_err();
    assert_eq!(reason(error), folder_ownership::TITLE_FOLDER_CHANGED);
    assert_eq!(
        folder_of(&pool, "emberfall").await.0.as_deref(),
        Some("/media/Emberfall")
    );
}

#[tokio::test]
async fn two_titles_swap_folders_in_one_write() {
    let (store, pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    store
        .assign_title_folders(&[
            TitleFolderWrite::assign("emberfall", "/media/A"),
            TitleFolderWrite::assign("cinderwake", "/media/B"),
        ])
        .await
        .unwrap();
    store
        .assign_title_folders(&[
            TitleFolderWrite::assign("emberfall", "/media/B").expecting(Some("/media/A".into())),
            TitleFolderWrite::assign("cinderwake", "/media/A").expecting(Some("/media/B".into())),
        ])
        .await
        .unwrap();
    assert_eq!(
        folder_of(&pool, "emberfall").await.0.as_deref(),
        Some("/media/B")
    );
    assert_eq!(
        folder_of(&pool, "cinderwake").await.0.as_deref(),
        Some("/media/A")
    );

    // A swap that fails one check changes neither title.
    let error = store
        .assign_title_folders(&[
            TitleFolderWrite::assign("emberfall", "/media/A").expecting(Some("/media/B".into())),
            TitleFolderWrite::assign("cinderwake", "/media/B").expecting(None),
        ])
        .await
        .unwrap_err();
    assert_eq!(reason(error), folder_ownership::TITLE_FOLDER_CHANGED);
    assert_eq!(
        folder_of(&pool, "emberfall").await.0.as_deref(),
        Some("/media/B")
    );
    assert_eq!(
        folder_of(&pool, "cinderwake").await.0.as_deref(),
        Some("/media/A")
    );
}

#[tokio::test]
async fn a_folder_check_refuses_what_the_write_would_and_writes_nothing() {
    let (store, pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/media/Emberfall")])
        .await
        .unwrap();
    let error = store
        .check_title_folder_writes(&[TitleFolderWrite::assign(
            "cinderwake",
            "/media/Emberfall/Extras",
        )])
        .await
        .unwrap_err();
    assert_eq!(
        reason(error),
        folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
    );
    // A title about to be created is checked as one with no folder.
    let error = store
        .check_title_folder_writes(&[TitleFolderWrite::assign(
            "not-created-yet",
            "/media/Emberfall",
        )
        .expecting(None)])
        .await
        .unwrap_err();
    assert_eq!(reason(error), folder_ownership::FOLDER_ALREADY_OWNED_TITLE);
    store
        .check_title_folder_writes(&[TitleFolderWrite::assign("cinderwake", "/media/Cinderwake")])
        .await
        .unwrap();
    assert_eq!(folder_of(&pool, "cinderwake").await, (None, None));
}

#[tokio::test]
async fn a_second_claim_of_the_same_folder_by_the_same_title_succeeds() {
    let (store, pool) = store_with_titles(&["emberfall"]).await;
    for _ in 0..2 {
        store
            .assign_title_folders(&[
                TitleFolderWrite::assign("emberfall", "/media/Emberfall").expecting(None)
            ])
            .await
            .expect("a title already holding the folder has its claim");
    }
    let owners: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE folder_path = '/media/Emberfall'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(owners, 1);
    assert_eq!(
        folder_of(&pool, "emberfall").await,
        (
            Some("/media/Emberfall".into()),
            Some("/media/Emberfall".into())
        )
    );
}

#[tokio::test]
async fn a_title_recording_a_library_root_does_not_block_folders_inside_it() {
    let (store, pool) = store_with_titles(&["stale", "emberfall"]).await;
    store
        .assign_title_folders(&[TitleFolderWrite::assign("stale", "/srv/films")])
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES ('root-films', 'movie_default_library', '/srv/films', '/srv/films', 0,
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    store
        .assign_title_folders(&[TitleFolderWrite::assign(
            "emberfall",
            "/srv/films/Emberfall",
        )])
        .await
        .expect("a root-sized record owns nothing under the root");
    let error = store
        .assign_title_folders(&[TitleFolderWrite::assign("emberfall", "/srv/films")])
        .await
        .unwrap_err();
    assert_eq!(
        reason(error),
        folder_ownership::CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER
    );
    assert_eq!(
        folder_of(&pool, "emberfall").await.0.as_deref(),
        Some("/srv/films/Emberfall")
    );
}

/// Two titles claim one folder: exactly one owns it and the other is refused.
/// The store's writer gate runs the claims one after the other, so this
/// proves the second claim is checked against the first, not how a race
/// between them resolves.
#[tokio::test]
async fn two_claims_of_one_folder_leave_exactly_one_owner() {
    let (store, pool) = store_with_titles(&["emberfall", "cinderwake"]).await;
    let first = [TitleFolderWrite::assign("emberfall", "/media/Shared").expecting(None)];
    let second = [TitleFolderWrite::assign("cinderwake", "/media/Shared").expecting(None)];
    let (left, right) = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        tokio::join!(
            store.assign_title_folders(&first),
            store.assign_title_folders(&second)
        )
    })
    .await
    .expect("both claims should finish");
    let outcomes = [left, right];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let loser = outcomes
        .into_iter()
        .find_map(Result::err)
        .expect("one claim must lose");
    assert_eq!(reason(loser), folder_ownership::FOLDER_ALREADY_OWNED_TITLE);
    let owners: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE folder_path = '/media/Shared'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(owners, 1);
}

/// Asserts that each step advanced the store's folder-write counter, or left
/// it alone.
struct CounterWatch(u64);

impl CounterWatch {
    fn new(store: &TitleStore) -> Self {
        Self(
            store
                .title_folder_write_generation()
                .expect("the store keeps a counter while no write is open"),
        )
    }

    fn advanced(&mut self, store: &TitleStore, step: &str) {
        let now = store.title_folder_write_generation().expect(step);
        assert!(now > self.0, "{step} must advance the counter");
        self.0 = now;
    }

    fn unchanged(&self, store: &TitleStore, step: &str) {
        assert_eq!(
            store.title_folder_write_generation(),
            Some(self.0),
            "{step}"
        );
    }
}

fn titled(id: &str, folder: Option<&str>) -> scryer_domain::Title {
    let mut title = super::catalog_order_tests::fixture_title(id, id, Some(2001));
    title.root_folder_id = "root-fixture".into();
    title.folder_path = folder.map(str::to_owned);
    title
}

#[tokio::test]
async fn every_folder_and_title_set_write_advances_the_counter() {
    let (store, _pool) = store_with_titles(&["emberfall", "cinderwake", "ashgrove"]).await;
    let mut watch = CounterWatch::new(&store);

    store
        .assign_title_folders(&[TitleFolderWrite::assign(
            "emberfall",
            "/srv/fixture-root/Emberfall",
        )])
        .await
        .unwrap();
    watch.advanced(&store, "assign");
    let (result, generation) = store
        .assign_title_folders_with_generation(&[TitleFolderWrite::assign(
            "cinderwake",
            "/srv/fixture-root/Cinderwake",
        )])
        .await;
    result.unwrap();
    watch.advanced(&store, "assign with generation");
    assert_eq!(
        generation,
        Some(watch.0),
        "the call reports its own advance"
    );
    store
        .set_folder_path("ashgrove", "/srv/fixture-root/Ashgrove")
        .await
        .unwrap();
    watch.advanced(&store, "set folder");
    store.clear_folder_path("ashgrove").await.unwrap();
    watch.advanced(&store, "clear folder");
    store
        .assign_title_folders(&[TitleFolderWrite::assign(
            "ashgrove",
            "/srv/fixture-root/Emberfall",
        )])
        .await
        .unwrap_err();
    watch.advanced(&store, "a refused write still ends its scope");
    store.assign_title_folders(&[]).await.unwrap();
    watch.unchanged(&store, "an empty write writes nothing");

    store.create(titled("duskmere", None)).await.unwrap();
    watch.unchanged(&store, "a title created without a folder owns nothing");
    store
        .create(titled("gloamridge", Some("/srv/fixture-root/Gloamridge")))
        .await
        .unwrap();
    watch.advanced(&store, "create with a folder");
    store
        .create_or_get_existing(titled("pinecrest", Some("/srv/fixture-root/Pinecrest")))
        .await
        .unwrap();
    watch.advanced(&store, "create or get existing");
    store
        .create_or_get_existing_with_options_patch(
            titled("thornvale", Some("/srv/fixture-root/Thornvale")),
            Default::default(),
        )
        .await
        .unwrap();
    watch.advanced(&store, "create or get existing with options");
    store
        .create_or_get_existing_preserving_options(
            titled("hollowmere", Some("/srv/fixture-root/Hollowmere")),
            Default::default(),
        )
        .await
        .unwrap();
    watch.advanced(&store, "create or get existing preserving options");
    // No pending import has that id, so the transaction rolls back after the
    // title was inserted with its folder; the scope still advances.
    store
        .create_or_get_existing_and_bind_pending_import(
            titled("briarwick", Some("/srv/fixture-root/Briarwick")),
            "synthetic-missing-pending-import",
        )
        .await
        .unwrap_err();
    watch.advanced(&store, "create and bind pending import");

    store.delete("emberfall").await.unwrap();
    watch.advanced(&store, "delete");
    assert!(
        store
            .delete_titles_with_records(&["cinderwake".to_string()], None)
            .await
            .unwrap()
    );
    watch.advanced(&store, "delete titles with records");
}

#[tokio::test]
async fn an_open_folder_write_hides_the_counter_until_it_ends() {
    let (store, _pool) = store_with_titles(&[]).await;
    let before = store.title_folder_write_generation().unwrap();
    let scope = super::TitleFolderWriteScope::open();
    assert_eq!(store.title_folder_write_generation(), Some(before));
    scope.proof().mark();
    assert_eq!(
        store.title_folder_write_generation(),
        None,
        "a marked write may commit before it advances"
    );
    assert_eq!(scope.end(), Some(before + 1));
    assert_eq!(store.title_folder_write_generation(), Some(before + 1));
    // Dropped without ending, as on an early return or a cancelled call.
    let scope = super::TitleFolderWriteScope::marked();
    assert_eq!(store.title_folder_write_generation(), None);
    drop(scope);
    assert_eq!(store.title_folder_write_generation(), Some(before + 2));
    drop(super::TitleFolderWriteScope::open());
    assert_eq!(store.title_folder_write_generation(), Some(before + 2));
}

#[tokio::test]
async fn folder_owner_read_matches_the_delete_preview_read() {
    let (store, _pool) = store_with_titles(&["emberfall", "cinderwake", "ashgrove"]).await;
    store
        .assign_title_folders(&[
            TitleFolderWrite::assign("emberfall", "/srv/fixture-root/Emberfall"),
            TitleFolderWrite::assign("ashgrove", "/srv/fixture-root/Ashgrove"),
        ])
        .await
        .unwrap();
    let expected: Vec<(String, String)> = store
        .list_delete_preview_info()
        .await
        .unwrap()
        .into_iter()
        .filter_map(|owner| {
            owner
                .folder_path
                .filter(|path| !path.is_empty())
                .map(|path| (owner.title_id, path))
        })
        .collect();
    assert_eq!(expected.len(), 2);
    assert_eq!(store.list_title_folder_owners().await.unwrap(), expected);
}
