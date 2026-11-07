//! Store-level tests for the catalogue's file-presence and attention filters.
//!
//! These run the real predicates against a real sqlite database with the full
//! migration set replayed, because every state is a comparison over `media_files`,
//! `episodes` and the mapping between them. A string assertion on the generated SQL
//! cannot tell a title whose unmonitored backlog is on disk from one that is
//! finished, which is how the first version of these filters shipped three wrong
//! states.

use scryer_application::{
    SortDirection, TitleCatalogAggregates, TitleCatalogFilter, TitleCatalogPresence,
    TitleCatalogResult, TitleCatalogSort, TitleCatalogSortKey, TitleListProjection,
    TitleRepository,
};
use sqlx::sqlite::SqlitePoolOptions;

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::TitleStore;

const LIBRARY_ID: &str = "library-presence";
const ROOT_FOLDER_ID: &str = "root-library-presence";
/// A file parked in the recycle bin, and one replaced by a staged upgrade. Neither
/// counts as being on disk, for presence or for attention.
const RECYCLED_PATH: &str = "/media/Series/Recycled/.scryer-recycle/S01E01.mkv";
const STAGED_PATH: &str = "/media/Series/Staged/.scryer-upgrade-replacement-old/S01E01.mkv";
const AIRED: &str = "2001-02-03";
const UNAIRED: &str = "2099-01-01";
const SEEDED_TITLES: usize = 10;

/// Titles holding nothing on disk: a movie with no file, and two whose only file is
/// somewhere presence does not count.
const EXPECTED_MISSING: &[&str] = &[
    "movie-missing",
    "series-scan-failed-recycled",
    "series-scan-failed-staged",
];

/// Titles holding something on disk and short of a monitored episode that has aired.
const EXPECTED_PARTIAL: &[&str] = &[
    "series-owned-backlog",
    "series-partial",
    "series-specials-only",
];

/// Titles holding something on disk and short of nothing.
const EXPECTED_COMPLETE: &[&str] = &[
    "movie-complete",
    "series-caught-up",
    "series-complete",
    "series-scan-failed",
];

async fn seeded_catalogue() -> TitleStore {
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
        pool,
        writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    };

    run(
        &datastore,
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, 'series', 'Presence', 'presence', 0, '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![SqlArg::Text(LIBRARY_ID.to_string())],
    )
    .await;

    for (id, facet) in [
        ("movie-complete", "movie"),
        ("movie-missing", "movie"),
        ("series-complete", "series"),
        ("series-partial", "series"),
        ("series-owned-backlog", "series"),
        ("series-specials-only", "series"),
        ("series-caught-up", "series"),
        ("series-scan-failed", "series"),
        ("series-scan-failed-recycled", "series"),
        ("series-scan-failed-staged", "series"),
    ] {
        insert_title(&datastore, id, facet).await;
    }

    // A movie holds one file, so it is complete the moment that file is on disk.
    insert_file(
        &datastore,
        "file-movie-complete",
        "movie-complete",
        "/media/Movies/Complete/movie.mkv",
        "complete",
    )
    .await;

    // Every monitored episode that has aired holds a file.
    let complete = insert_season(&datastore, "series-complete", "col-complete", "1").await;
    for number in 1..=2 {
        add_owned_episode(
            &datastore,
            EpisodeFixture {
                title_id: "series-complete",
                collection_id: &complete,
                episode_id: &format!("ep-complete-{number}"),
                file_id: &format!("file-complete-{number}"),
                number: &number.to_string(),
                air_date: AIRED,
                monitored: true,
                scan_status: "complete",
                path: &format!("/media/Series/Complete/S01E0{number}.mkv"),
            },
        )
        .await;
    }

    // Four monitored episodes have aired; two of them hold a file.
    let partial = insert_season(&datastore, "series-partial", "col-partial", "1").await;
    for number in 1..=4 {
        let episode_id = format!("ep-partial-{number}");
        insert_episode(
            &datastore,
            &episode_id,
            "series-partial",
            &partial,
            &number.to_string(),
            AIRED,
            true,
        )
        .await;
        if number > 2 {
            continue;
        }
        let file_id = format!("file-partial-{number}");
        insert_file(
            &datastore,
            &file_id,
            "series-partial",
            &format!("/media/Series/Partial/S01E0{number}.mkv"),
            "complete",
        )
        .await;
        link_file_to_episode(&datastore, &file_id, &episode_id).await;
    }

    // Three unmonitored episodes are on disk and the one monitored episode is not.
    // Counting every owned episode against the monitored ones is what used to read
    // this title as complete.
    let backlog = insert_season(&datastore, "series-owned-backlog", "col-backlog", "1").await;
    for number in 1..=3 {
        add_owned_episode(
            &datastore,
            EpisodeFixture {
                title_id: "series-owned-backlog",
                collection_id: &backlog,
                episode_id: &format!("ep-backlog-{number}"),
                file_id: &format!("file-backlog-{number}"),
                number: &number.to_string(),
                air_date: AIRED,
                monitored: false,
                scan_status: "complete",
                path: &format!("/media/Series/Backlog/S01E0{number}.mkv"),
            },
        )
        .await;
    }
    insert_episode(
        &datastore,
        "ep-backlog-monitored",
        "series-owned-backlog",
        &backlog,
        "4",
        AIRED,
        true,
    )
    .await;

    // The only file is a special, which the episode counts leave out: the title holds
    // something, and none of the monitored episodes is in it.
    let specials = insert_collection(
        &datastore,
        "col-specials",
        "series-specials-only",
        "specials",
        "0",
    )
    .await;
    insert_episode(
        &datastore,
        "ep-specials-1",
        "series-specials-only",
        &specials,
        "1",
        AIRED,
        true,
    )
    .await;
    insert_file(
        &datastore,
        "file-specials",
        "series-specials-only",
        "/media/Series/Specials/S00E01.mkv",
        "complete",
    )
    .await;
    link_file_to_episode(&datastore, "file-specials", "ep-specials-1").await;
    let specials_season =
        insert_season(&datastore, "series-specials-only", "col-season", "1").await;
    for number in 1..=2 {
        insert_episode(
            &datastore,
            &format!("ep-specials-season-{number}"),
            "series-specials-only",
            &specials_season,
            &number.to_string(),
            AIRED,
            true,
        )
        .await;
    }

    // Caught up, with the next season announced and monitored. The episode that has
    // not aired is not outstanding work.
    let caught_up = insert_season(&datastore, "series-caught-up", "col-caught-up", "1").await;
    for number in 1..=2 {
        add_owned_episode(
            &datastore,
            EpisodeFixture {
                title_id: "series-caught-up",
                collection_id: &caught_up,
                episode_id: &format!("ep-caught-up-{number}"),
                file_id: &format!("file-caught-up-{number}"),
                number: &number.to_string(),
                air_date: AIRED,
                monitored: true,
                scan_status: "complete",
                path: &format!("/media/Series/CaughtUp/S01E0{number}.mkv"),
            },
        )
        .await;
    }
    insert_episode(
        &datastore,
        "ep-caught-up-3",
        "series-caught-up",
        &caught_up,
        "3",
        UNAIRED,
        true,
    )
    .await;

    // A failed scan is still a file on disk; the attention filter is what names it.
    let scan_failed = insert_season(&datastore, "series-scan-failed", "col-scan-failed", "1").await;
    add_owned_episode(
        &datastore,
        EpisodeFixture {
            title_id: "series-scan-failed",
            collection_id: &scan_failed,
            episode_id: "ep-scan-failed",
            file_id: "file-scan-failed",
            number: "1",
            air_date: AIRED,
            monitored: true,
            scan_status: "scan_failed",
            path: "/media/Series/Failed/S01E01.mkv",
        },
    )
    .await;

    // The same failed file, once in the recycle bin and once replaced by a staged
    // upgrade: neither is on disk, so the title holds nothing and is not flagged.
    for (title_id, collection_id, episode_id, file_id, path) in [
        (
            "series-scan-failed-recycled",
            "col-recycled",
            "ep-recycled",
            "file-recycled",
            RECYCLED_PATH,
        ),
        (
            "series-scan-failed-staged",
            "col-staged",
            "ep-staged",
            "file-staged",
            STAGED_PATH,
        ),
    ] {
        let season = insert_season(&datastore, title_id, collection_id, "1").await;
        insert_episode(&datastore, episode_id, title_id, &season, "1", AIRED, true).await;
        insert_file(&datastore, file_id, title_id, path, "scan_failed").await;
        link_file_to_episode(&datastore, file_id, episode_id).await;
    }

    TitleStore::new(datastore)
}

/// Presence is a partition: every title in scope is exactly one of the three states,
/// so the counts add up to what the page shows and no title falls between them.
#[tokio::test]
async fn presence_states_partition_the_catalogue_and_match_the_page() {
    let store = seeded_catalogue().await;

    let counts = catalog(&store, TitleCatalogFilter::default())
        .await
        .filter_counts;
    assert_eq!(counts.all, SEEDED_TITLES);
    assert_eq!(counts.missing, EXPECTED_MISSING.len());
    assert_eq!(counts.partial, EXPECTED_PARTIAL.len());
    assert_eq!(counts.complete, EXPECTED_COMPLETE.len());
    assert_eq!(
        counts.missing + counts.partial + counts.complete,
        counts.all,
        "the three states have to cover every title in scope"
    );

    for (state, expected_names) in [
        (TitleCatalogPresence::Missing, EXPECTED_MISSING),
        (TitleCatalogPresence::Partial, EXPECTED_PARTIAL),
        (TitleCatalogPresence::Complete, EXPECTED_COMPLETE),
    ] {
        assert_eq!(
            names_for(&store, presence_filter(vec![state])).await,
            expected(expected_names),
            "{state:?}"
        );
    }

    // Any-of, because the states are toggles an operator combines.
    let mut combined = [EXPECTED_MISSING, EXPECTED_PARTIAL].concat();
    combined.sort_unstable();
    assert_eq!(
        names_for(
            &store,
            presence_filter(vec![
                TitleCatalogPresence::Missing,
                TitleCatalogPresence::Partial,
            ]),
        )
        .await,
        expected(&combined)
    );
}

/// Attention asks the same question of the same rows presence does, so a failed scan
/// whose file is not on disk does not keep a title flagged.
#[tokio::test]
async fn attention_counts_only_files_that_are_on_disk() {
    let store = seeded_catalogue().await;

    assert_eq!(
        names_for(&store, attention_filter(Some(true))).await,
        expected(&["series-scan-failed"])
    );
    assert_eq!(
        catalog(&store, attention_filter(Some(true)))
            .await
            .filter_counts
            .needs_attention,
        1
    );

    let clear = names_for(&store, attention_filter(Some(false))).await;
    assert_eq!(clear.len(), SEEDED_TITLES - 1);
    assert!(clear.contains(&"series-scan-failed-recycled".to_string()));
    assert!(clear.contains(&"series-scan-failed-staged".to_string()));
}

/// A client refreshing a few titles asks for them by id under its other filters, so
/// the id list narrows the page and the other filters still apply to it.
#[tokio::test]
async fn title_ids_narrow_the_page_under_the_other_filters() {
    let store = seeded_catalogue().await;
    let by_ids = |ids: &[&str], presences: Vec<TitleCatalogPresence>| TitleCatalogFilter {
        title_ids: expected(ids),
        presences,
        ..TitleCatalogFilter::default()
    };

    assert_eq!(
        names_for(
            &store,
            by_ids(&["movie-missing", "series-complete"], Vec::new())
        )
        .await,
        expected(&["movie-missing", "series-complete"])
    );
    // A requested title the other filters exclude is left out, which is how the
    // client learns it no longer belongs in the list.
    assert_eq!(
        names_for(
            &store,
            by_ids(
                &["movie-missing", "series-complete"],
                vec![TitleCatalogPresence::Complete],
            ),
        )
        .await,
        expected(&["series-complete"])
    );
    assert!(
        names_for(&store, by_ids(&["no-such-title"], Vec::new()))
            .await
            .is_empty()
    );
}

fn expected(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

fn presence_filter(presences: Vec<TitleCatalogPresence>) -> TitleCatalogFilter {
    TitleCatalogFilter {
        presences,
        ..TitleCatalogFilter::default()
    }
}

fn attention_filter(needs_attention: Option<bool>) -> TitleCatalogFilter {
    TitleCatalogFilter {
        needs_attention,
        ..TitleCatalogFilter::default()
    }
}

async fn catalog(store: &TitleStore, filter: TitleCatalogFilter) -> TitleCatalogResult {
    store
        .list_for_libraries_catalog(
            None,
            &[LIBRARY_ID.to_string()],
            None,
            filter,
            TitleCatalogSort::new(TitleCatalogSortKey::Title, SortDirection::Asc),
            50,
            0,
            TitleListProjection::default(),
            TitleCatalogAggregates {
                total_count: true,
                filter_counts: true,
                managed_bytes: false,
            },
        )
        .await
        .expect("the catalogue query should run")
}

async fn names_for(store: &TitleStore, filter: TitleCatalogFilter) -> Vec<String> {
    let mut names: Vec<String> = catalog(store, filter)
        .await
        .items
        .into_iter()
        .map(|title| title.name)
        .collect();
    names.sort_unstable();
    names
}

struct EpisodeFixture<'a> {
    title_id: &'a str,
    collection_id: &'a str,
    episode_id: &'a str,
    file_id: &'a str,
    number: &'a str,
    air_date: &'a str,
    monitored: bool,
    scan_status: &'a str,
    path: &'a str,
}

async fn add_owned_episode(datastore: &StoreDatastore, fixture: EpisodeFixture<'_>) {
    insert_episode(
        datastore,
        fixture.episode_id,
        fixture.title_id,
        fixture.collection_id,
        fixture.number,
        fixture.air_date,
        fixture.monitored,
    )
    .await;
    insert_file(
        datastore,
        fixture.file_id,
        fixture.title_id,
        fixture.path,
        fixture.scan_status,
    )
    .await;
    link_file_to_episode(datastore, fixture.file_id, fixture.episode_id).await;
}

async fn insert_title(datastore: &StoreDatastore, id: &str, facet: &str) {
    run(
        datastore,
        // `root_folder_id` is non-null by trigger.
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, created_at, library_id, root_folder_id)
         VALUES ({}, {}, {}, {}, 1, 'active', '[]', '[]', '2026-01-01T00:00:00Z', {}, {})",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(LIBRARY_ID.to_string()),
            SqlArg::Text(ROOT_FOLDER_ID.to_string()),
        ],
    )
    .await;
}

async fn insert_season(
    datastore: &StoreDatastore,
    title_id: &str,
    id: &str,
    index: &str,
) -> String {
    insert_collection(datastore, id, title_id, "season", index).await
}

async fn insert_collection(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    collection_type: &str,
    index: &str,
) -> String {
    run(
        datastore,
        "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at)
         VALUES ({}, {}, {}, {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(collection_type.to_string()),
            SqlArg::Text(index.to_string()),
        ],
    )
    .await;
    id.to_string()
}

async fn insert_episode(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    collection_id: &str,
    number: &str,
    air_date: &str,
    monitored: bool,
) {
    run(
        datastore,
        "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number,
                               season_number, title, air_date, monitored, created_at)
         VALUES ({}, {}, {}, 'standard', {}, '1', {}, {}, {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(collection_id.to_string()),
            SqlArg::Text(number.to_string()),
            SqlArg::Text(format!("Episode {number}")),
            SqlArg::Text(air_date.to_string()),
            SqlArg::I64(i64::from(monitored)),
        ],
    )
    .await;
}

async fn insert_file(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    path: &str,
    scan_status: &str,
) {
    run(
        datastore,
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, role,
                                  created_at)
         VALUES ({}, {}, {}, 100, {}, 'primary', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(path.to_string()),
            SqlArg::Text(scan_status.to_string()),
        ],
    )
    .await;
}

async fn link_file_to_episode(datastore: &StoreDatastore, file_id: &str, episode_id: &str) {
    run(
        datastore,
        "INSERT INTO file_episode_map (file_id, episode_id, role, is_filler)
         VALUES ({}, {}, 'primary', 0)",
        vec![
            SqlArg::Text(file_id.to_string()),
            SqlArg::Text(episode_id.to_string()),
        ],
    )
    .await;
}

async fn run(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "presence_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}
