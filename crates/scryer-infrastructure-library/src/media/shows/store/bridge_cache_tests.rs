//! The bridge read cache against the real schema: reads are served from
//! memory, and every in-process path that changes or removes a bridge row
//! drops the cached value. Rows are changed behind the store's back with raw
//! SQL, so a read that still returns the old value proves it never reached the
//! database.

use std::sync::Arc;

use scryer_application::{ShowRepository, TitleRepository};
use scryer_domain::{AnimeCommunitySeason, AnimeNumberingBridge};
use sqlx::sqlite::SqlitePoolOptions;

use super::{ShowStore, get_anime_numbering_bridge_query};
use crate::media::titles::store::TitleStore;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

const TITLE: &str = "title-synthetic-bridge";

async fn datastore() -> StoreDatastore {
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
    StoreDatastore::Sqlite {
        pool,
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    }
}

async fn run(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "bridge_cache_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn insert_title(datastore: &StoreDatastore, id: &str) {
    run(
        datastore,
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, created_at, library_id, root_folder_id)
         VALUES ({}, {}, {}, 'anime', 1, 'active', '[]', '[]', '2026-01-01T00:00:00Z',
                 'library-synthetic', 'root-synthetic')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(format!("Synthetic Show {id}")),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await;
}

/// Overwrite (or create) the row without going through any store.
async fn write_row_behind_the_store(
    datastore: &StoreDatastore,
    id: &str,
    bridge: &AnimeNumberingBridge,
) {
    run(
        datastore,
        "DELETE FROM title_anime_numbering_bridges WHERE title_id = {}",
        vec![SqlArg::Text(id.to_string())],
    )
    .await;
    run(
        datastore,
        "INSERT INTO title_anime_numbering_bridges
             (title_id, generated_on, corroborating_order, source, seasons_json, updated_at)
         VALUES ({}, {}, NULL, 'anime_community', {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(bridge.generated_on.clone()),
            SqlArg::Text(serde_json::to_string(&bridge.seasons).unwrap()),
        ],
    )
    .await;
}

fn bridge(tag: &str) -> AnimeNumberingBridge {
    AnimeNumberingBridge {
        generated_on: tag.to_string(),
        seasons: vec![AnimeCommunitySeason {
            index: 1,
            anidb_id: None,
            anilist_id: None,
            mal_id: None,
            titles: vec![format!("Synthetic Cour {tag}")],
            ranges: Vec::new(),
            absolute_start: Some(1),
            episode_count: Some(12),
        }],
        ..Default::default()
    }
}

async fn read(shows: &ShowStore, id: &str) -> Option<AnimeNumberingBridge> {
    shows
        .get_anime_numbering_bridge(id)
        .await
        .expect("bridge read")
}

/// A show store and a title store wired to one cache, as the assembly does.
fn stores(datastore: &StoreDatastore) -> (ShowStore, TitleStore) {
    let shows = ShowStore::new(datastore.clone());
    let titles = TitleStore::new(datastore.clone())
        .with_anime_numbering_bridge_cache(shows.anime_numbering_bridge_cache());
    (shows, titles)
}

#[tokio::test]
async fn repeated_reads_are_served_without_the_database() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, _) = stores(&datastore);
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("first")))
        .await
        .unwrap();

    assert_eq!(read(&shows, TITLE).await, Some(bridge("first")));
    write_row_behind_the_store(&datastore, TITLE, &bridge("behind")).await;
    for _ in 0..3 {
        assert_eq!(
            read(&shows, TITLE).await,
            Some(bridge("first")),
            "a cached read must not reach the database"
        );
    }
}

#[tokio::test]
async fn an_absent_bridge_is_cached_until_a_bridge_is_written() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, _) = stores(&datastore);

    assert_eq!(read(&shows, TITLE).await, None);
    write_row_behind_the_store(&datastore, TITLE, &bridge("behind")).await;
    assert_eq!(
        read(&shows, TITLE).await,
        None,
        "the absence is cached, so the row written behind the store stays unseen"
    );

    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("hydrated")))
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("hydrated")));
}

#[tokio::test]
async fn replacing_or_removing_a_bridge_invalidates_it() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, _) = stores(&datastore);

    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("one")))
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("one")));
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("two")))
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("two")));
    shows
        .replace_anime_numbering_bridge(TITLE, None)
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, None);
}

#[tokio::test]
async fn a_failed_replace_still_drops_the_cached_value() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, _) = stores(&datastore);
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("cached")))
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("cached")));

    write_row_behind_the_store(&datastore, TITLE, &bridge("behind")).await;
    run(
        &datastore,
        "CREATE TRIGGER fail_bridge_write BEFORE INSERT ON title_anime_numbering_bridges
         BEGIN SELECT RAISE(ABORT, 'injected bridge write failure'); END",
        vec![],
    )
    .await;
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("rejected")))
        .await
        .unwrap_err();
    assert_eq!(
        read(&shows, TITLE).await,
        Some(bridge("behind")),
        "a write that failed must not leave the pre-write value cached"
    );
}

#[tokio::test]
async fn deleting_the_title_drops_its_cascaded_bridge() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, titles) = stores(&datastore);
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("doomed")))
        .await
        .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("doomed")));

    TitleRepository::delete(&titles, TITLE).await.unwrap();
    assert_eq!(
        read(&shows, TITLE).await,
        None,
        "the row cascaded away with the title, and the cache must follow"
    );
}

#[tokio::test]
async fn editing_the_title_picks_up_a_bridge_written_outside_the_process() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, titles) = stores(&datastore);
    assert_eq!(read(&shows, TITLE).await, None);

    write_row_behind_the_store(&datastore, TITLE, &bridge("external")).await;
    TitleRepository::update_metadata(
        &titles,
        TITLE,
        Some(format!("Synthetic Show {TITLE}")),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(read(&shows, TITLE).await, Some(bridge("external")));
}

/// A read that fetched the old row, then lost the race to a write that
/// committed and invalidated before the read installed, must not install.
/// The interleaving is driven inside the loader, so it is exact.
#[tokio::test]
async fn a_read_racing_a_write_never_installs_the_stale_value() {
    let datastore = datastore().await;
    insert_title(&datastore, TITLE).await;
    let (shows, _) = stores(&datastore);
    shows
        .replace_anime_numbering_bridge(TITLE, Some(&bridge("old")))
        .await
        .unwrap();
    let cache = shows.anime_numbering_bridge_cache();
    cache.clear();

    let raced = cache
        .get_or_load(TITLE, || async {
            let stale = get_anime_numbering_bridge_query(shows.read_target(), TITLE).await?;
            shows
                .replace_anime_numbering_bridge(TITLE, Some(&bridge("new")))
                .await?;
            Ok(stale)
        })
        .await
        .unwrap();
    assert_eq!(
        raced,
        Some(bridge("old")),
        "the racing reader itself saw the pre-write row"
    );
    assert_eq!(cache.lookup(TITLE), None, "but it must not have cached it");
    assert_eq!(read(&shows, TITLE).await, Some(bridge("new")));
}
