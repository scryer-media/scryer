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

/// The single counts statement has to give the numbers the per-dimension COUNT
/// queries gave: each count is the page predicate with one dimension swapped.
#[tokio::test]
async fn combined_counts_match_one_count_per_dimension() {
    use scryer_application::TitleCatalogContentStatus as Status;

    let store = seeded_catalogue().await;
    for (id, monitored, status) in [
        ("movie-complete", 0, Some("Ended")),
        ("series-complete", 1, Some(" returning ")),
        ("series-partial", 0, Some("continuing")),
        ("series-owned-backlog", 1, Some("finished")),
        ("series-caught-up", 0, Some("in production")),
        ("series-scan-failed", 1, None),
    ] {
        run(
            &store.datastore,
            "UPDATE titles SET monitored = {}, content_status = {} WHERE id = {}",
            vec![
                SqlArg::I64(monitored),
                SqlArg::OptText(status.map(str::to_string)),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await;
    }

    let filters = [
        TitleCatalogFilter::default(),
        TitleCatalogFilter {
            monitored: Some(true),
            ..TitleCatalogFilter::default()
        },
        TitleCatalogFilter {
            monitored: Some(false),
            content_statuses: vec![Status::Continuing],
            ..TitleCatalogFilter::default()
        },
        TitleCatalogFilter {
            content_statuses: vec![Status::Ended, Status::Continuing],
            ..TitleCatalogFilter::default()
        },
        TitleCatalogFilter {
            monitored: Some(true),
            content_statuses: vec![Status::Ended],
            presences: vec![TitleCatalogPresence::Partial, TitleCatalogPresence::Missing],
            ..TitleCatalogFilter::default()
        },
        TitleCatalogFilter {
            needs_attention: Some(false),
            title_ids: expected(&["movie-complete", "series-partial", "series-scan-failed"]),
            ..TitleCatalogFilter::default()
        },
    ];

    for filter in filters {
        let one = |monitored: Option<bool>, statuses: Vec<Status>| TitleCatalogFilter {
            monitored,
            content_statuses: statuses,
            ..filter.clone()
        };
        let separately = [
            one(None, Vec::new()),
            one(Some(true), filter.content_statuses.clone()),
            one(Some(false), filter.content_statuses.clone()),
            one(filter.monitored, vec![Status::Continuing]),
            one(filter.monitored, vec![Status::Ended]),
            filter.clone(),
        ];
        let mut expected_counts = Vec::new();
        for dimension in &separately {
            expected_counts.push(
                super::fetch_title_catalog_count(
                    &store.datastore,
                    None,
                    &[LIBRARY_ID.to_string()],
                    None,
                    dimension,
                )
                .await
                .expect("a per-dimension count should run"),
            );
        }

        let result = catalog(&store, filter.clone()).await;
        let counts = &result.filter_counts;
        assert_eq!(
            vec![
                counts.all,
                counts.monitored,
                counts.unmonitored,
                counts.continuing,
                counts.ended,
                result.total_count,
            ],
            expected_counts,
            "{filter:?}"
        );
        assert_eq!(result.total_count, result.items.len(), "{filter:?}");
    }
}

/// The file-state counts run only when asked for, and asking for them changes none
/// of the other counts.
#[tokio::test]
async fn file_state_counts_run_only_when_requested() {
    let store = seeded_catalogue().await;
    let library_ids = [LIBRARY_ID.to_string()];
    let counts_only = |file_state_counts: bool| {
        store.list_for_libraries_catalog(
            None,
            &library_ids,
            None,
            TitleCatalogFilter::default(),
            TitleCatalogSort::new(TitleCatalogSortKey::Title, SortDirection::Asc),
            0,
            0,
            TitleListProjection::default(),
            TitleCatalogAggregates {
                total_count: true,
                filter_counts: true,
                file_state_counts,
                managed_bytes: false,
            },
        )
    };
    let dimensions = |result: &TitleCatalogResult| {
        let counts = &result.filter_counts;
        [
            counts.all,
            counts.monitored,
            counts.unmonitored,
            counts.continuing,
            counts.ended,
            result.total_count,
        ]
    };

    let with = counts_only(true).await.expect("counts with file states");
    let counts = &with.filter_counts;
    assert_eq!(
        [
            counts.missing,
            counts.partial,
            counts.complete,
            counts.needs_attention
        ],
        [
            EXPECTED_MISSING.len(),
            EXPECTED_PARTIAL.len(),
            EXPECTED_COMPLETE.len(),
            1
        ]
    );
    assert_eq!(dimensions(&with)[0], SEEDED_TITLES);

    let without = counts_only(false)
        .await
        .expect("counts without file states");
    assert_eq!(dimensions(&without), dimensions(&with));
    let counts = &without.filter_counts;
    assert_eq!(
        [
            counts.missing,
            counts.partial,
            counts.complete,
            counts.needs_attention
        ],
        [0, 0, 0, 0]
    );

    // Every file-state count reads media_files and nothing else here does, so with the
    // table gone only a request that issues a file-state statement can fail.
    run(
        &store.datastore,
        "ALTER TABLE media_files RENAME TO unavailable_media_files",
        Vec::new(),
    )
    .await;
    let without = counts_only(false)
        .await
        .expect("no file-state statement should run");
    assert_eq!(dimensions(&without), dimensions(&with));
    assert!(
        counts_only(true).await.is_err(),
        "requested file states must still be counted"
    );
}

/// The narrow library lookup permission checks use answers what the full rows
/// answered, across libraries, unknown and repeated ids, and more ids than one
/// statement binds.
#[tokio::test]
async fn library_ids_for_titles_match_the_full_rows_across_chunks() {
    let store = seeded_catalogue().await;
    let other_library = "library-cinderwake";
    run(
        &store.datastore,
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, 'movie', 'Cinderwake', 'cinderwake', 0, '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![SqlArg::Text(other_library.to_string())],
    )
    .await;
    let mut ids = Vec::new();
    for number in 0..1_000 {
        let (id, library) = if number % 2 == 0 {
            (format!("emberfall-{number}"), LIBRARY_ID)
        } else {
            (format!("ashgrove-{number}"), other_library)
        };
        insert_title_in(&store.datastore, &id, "movie", library).await;
        ids.push(id);
    }
    ids.extend(expected(&[
        "movie-complete",
        "series-partial",
        "no-such-title",
    ]));
    ids.extend((0..1_000).map(|number| format!("cinderwake-missing-{number}")));
    ids.push("emberfall-0".to_string());

    let mut full_rows = store
        .get_by_ids(&ids)
        .await
        .expect("full rows should load")
        .into_iter()
        .map(|title| (title.id, title.library_id))
        .collect::<Vec<_>>();
    let mut narrow = store
        .list_library_ids_for_titles(&ids)
        .await
        .expect("library ids should load");
    full_rows.sort();
    narrow.sort();
    assert_eq!(narrow, full_rows);
    assert_eq!(narrow.len(), 1_002, "each existing id once");
    assert!(narrow.contains(&("ashgrove-1".to_string(), other_library.to_string())));
    assert!(narrow.contains(&("emberfall-998".to_string(), LIBRARY_ID.to_string())));

    // The single statement the lookup used to be, on the same ids.
    let distinct = ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let placeholders = std::iter::repeat_n("{}", distinct.len())
        .collect::<Vec<_>>()
        .join(", ");
    let mut previous = SqlRuntime::fetch_all(
        store.datastore.read_exec(),
        &format!("SELECT id, library_id FROM titles WHERE id IN ({placeholders})"),
        &distinct.into_iter().map(SqlArg::Text).collect::<Vec<_>>(),
    )
    .await
    .expect("one statement should run")
    .iter()
    .map(|row| (row.text("id").unwrap(), row.text("library_id").unwrap()))
    .collect::<Vec<_>>();
    previous.sort();
    assert_eq!(narrow, previous);
}

/// A title row with no library reads as its facet's default library, through
/// the narrow lookups exactly as through the full row decode.
#[tokio::test]
async fn narrow_library_reads_fall_back_to_the_facet_default_like_the_full_rows() {
    let store = seeded_catalogue().await;
    insert_title_in(&store.datastore, "emberfall-orphan", "series", LIBRARY_ID).await;
    insert_title_in(&store.datastore, "cinderwake-orphan", "anime", LIBRARY_ID).await;
    run(
        &store.datastore,
        "UPDATE titles SET library_id = NULL WHERE id IN ('emberfall-orphan', 'cinderwake-orphan')",
        Vec::new(),
    )
    .await;
    let ids = expected(&["emberfall-orphan", "cinderwake-orphan", "movie-complete"]);

    let mut full_rows = store
        .get_by_ids(&ids)
        .await
        .expect("full rows should load")
        .into_iter()
        .map(|title| (title.id, title.library_id))
        .collect::<Vec<_>>();
    let mut narrow = store
        .list_library_ids_for_titles(&ids)
        .await
        .expect("library ids should load");
    full_rows.sort();
    narrow.sort();
    assert_eq!(narrow, full_rows);
    assert!(narrow.contains(&(
        "emberfall-orphan".to_string(),
        scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Series),
    )));

    let mut scopes_from_full_rows = store
        .list_for_matching(None, None)
        .await
        .expect("full rows should load")
        .into_iter()
        .filter(|title| title.monitored)
        .map(|title| (title.library_id, title.facet.as_str().to_string()))
        .collect::<Vec<_>>();
    scopes_from_full_rows.sort();
    scopes_from_full_rows.dedup();
    let scopes = store
        .monitored_library_scopes()
        .await
        .expect("scopes should load");
    assert_eq!(scopes, scopes_from_full_rows);
    assert!(scopes.contains(&(
        scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Anime),
        "anime".to_string(),
    )));
}

/// Paging by id inside a set of libraries visits the same titles in the same
/// order as paging every title and keeping those in the set, including rows
/// with no library, which read as their facet's default library.
#[tokio::test]
async fn library_scoped_id_pages_match_filtering_every_page() {
    let store = seeded_catalogue().await;
    let other_library = "library-cinderwake";
    run(
        &store.datastore,
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, 'movie', 'Cinderwake', 'cinderwake', 0, '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![SqlArg::Text(other_library.to_string())],
    )
    .await;
    for number in 0..40 {
        let library = if number % 3 == 0 {
            other_library
        } else {
            LIBRARY_ID
        };
        insert_title_in(
            &store.datastore,
            &format!("ashgrove-{number:02}"),
            "movie",
            library,
        )
        .await;
    }
    for (id, facet) in [
        ("emberfall-orphan-series", "series"),
        ("emberfall-orphan-anime", "anime"),
        ("cinderwake-orphan-movie", "movie"),
    ] {
        insert_title_in(&store.datastore, id, facet, LIBRARY_ID).await;
    }
    run(
        &store.datastore,
        "UPDATE titles SET library_id = NULL WHERE id LIKE '%-orphan-%'",
        Vec::new(),
    )
    .await;
    let series_default =
        scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Series);

    async fn walk(
        store: &TitleStore,
        libraries: &[String],
        page_size: usize,
        scoped: bool,
    ) -> Vec<String> {
        let mut seen = Vec::new();
        let mut after_id = None;
        loop {
            let page = if scoped {
                store
                    .list_page_after_id_in_libraries(after_id.clone(), libraries, page_size)
                    .await
                    .expect("scoped page should load")
            } else {
                store
                    .list_page_after_id(after_id.clone(), page_size)
                    .await
                    .expect("page should load")
            };
            if page.is_empty() {
                break;
            }
            assert!(page.len() <= page_size);
            after_id = page.last().map(|title| title.id.clone());
            seen.extend(
                page.into_iter()
                    .filter(|title| {
                        scoped || libraries.is_empty() || libraries.contains(&title.library_id)
                    })
                    .map(|title| title.id),
            );
        }
        seen
    }

    for libraries in [
        vec![],
        vec![other_library.to_string()],
        vec![LIBRARY_ID.to_string(), series_default.clone()],
        vec!["library-unknown".to_string()],
    ] {
        for page_size in [1, 2, 7, 500] {
            let old = walk(&store, &libraries, page_size, false).await;
            let new = walk(&store, &libraries, page_size, true).await;
            assert_eq!(new, old, "libraries {libraries:?}, page size {page_size}");
        }
    }
    let with_orphan = walk(
        &store,
        &[LIBRARY_ID.to_string(), series_default.clone()],
        3,
        true,
    )
    .await;
    assert!(with_orphan.contains(&"emberfall-orphan-series".to_string()));
    assert!(!with_orphan.contains(&"emberfall-orphan-anime".to_string()));
    assert!(
        walk(&store, &[other_library.to_string()], 3, true)
            .await
            .contains(&"ashgrove-00".to_string())
    );
}

/// Reading episode files for several titles at once returns, per title, what the
/// per-title read returned, including a file's role for the episodes asked about.
#[tokio::test]
async fn episode_files_for_several_titles_match_the_per_title_read() {
    use scryer_application::MediaFileRepository;

    let store = seeded_catalogue().await;
    // One file covering two episodes: primary for the third, extra for the fourth.
    insert_file(
        &store.datastore,
        "file-partial-double",
        "series-partial",
        "/media/Series/Partial/S01E03-E04.mkv",
        "complete",
    )
    .await;
    link_file_to_episode(&store.datastore, "file-partial-double", "ep-partial-3").await;
    run(
        &store.datastore,
        "INSERT INTO file_episode_map (file_id, episode_id, role, is_filler)
         VALUES ('file-partial-double', 'ep-partial-4', 'additional', 0)",
        Vec::new(),
    )
    .await;
    let files =
        crate::media::search::media_file_store::MediaFileStore::new(store.datastore.clone());

    let requests = vec![
        (
            "series-partial".to_string(),
            expected(&["ep-partial-1", "ep-partial-4"]),
        ),
        ("series-complete".to_string(), expected(&["ep-complete-2"])),
        ("series-partial".to_string(), expected(&["ep-partial-2"])),
        // An episode of another title finds nothing under this one.
        ("series-caught-up".to_string(), expected(&["ep-partial-3"])),
        (
            "series-scan-failed-recycled".to_string(),
            expected(&["ep-recycled"]),
        ),
        ("no-such-title".to_string(), expected(&["ep-partial-1"])),
    ];
    let summary = |files: &[scryer_application::EpisodeScopedMediaFile]| {
        let mut rows = files
            .iter()
            .map(|file| {
                (
                    file.media_file.id.clone(),
                    file.media_file.role,
                    file.episode_ids.clone(),
                    file.primary_episode_ids.clone(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    };

    let batched = files
        .list_live_media_files_for_titles_episode_ids(&requests)
        .await
        .expect("batched read should run");
    assert_eq!(
        batched
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "series-partial",
            "series-complete",
            "series-caught-up",
            "series-scan-failed-recycled",
            "no-such-title",
        ]
    );
    for (title_id, files_for_title) in &batched {
        let episode_ids = requests
            .iter()
            .filter(|(id, _)| id == title_id)
            .flat_map(|(_, episode_ids)| episode_ids.iter().cloned())
            .collect::<Vec<_>>();
        let one_title = files
            .list_live_media_files_for_episode_ids(title_id, &episode_ids)
            .await
            .expect("per-title read should run");
        assert_eq!(summary(files_for_title), summary(&one_title), "{title_id}");
    }
    let partial = &batched[0].1;
    assert_eq!(
        summary(partial)
            .into_iter()
            .map(|(id, role, _, _)| (id, role))
            .collect::<Vec<_>>(),
        vec![
            (
                "file-partial-1".to_string(),
                scryer_application::MediaFileRole::Primary
            ),
            (
                "file-partial-2".to_string(),
                scryer_application::MediaFileRole::Primary
            ),
            (
                "file-partial-double".to_string(),
                scryer_application::MediaFileRole::Additional
            ),
        ]
    );
}

/// Reading episode files for several titles returns each title's files newest
/// first, in the same order whether the request is one statement or is split
/// across many, and in the order the per-title read returns them.
#[tokio::test]
async fn episode_files_for_several_titles_keep_the_newest_first_order_when_split() {
    use scryer_application::MediaFileRepository;

    let store = seeded_catalogue().await;
    insert_file(
        &store.datastore,
        "file-partial-double",
        "series-partial",
        "/media/Series/Partial/S01E03-E04.mkv",
        "complete",
    )
    .await;
    link_file_to_episode(&store.datastore, "file-partial-double", "ep-partial-3").await;
    // Older files hold earlier episodes, so statements read one episode at a
    // time return them oldest first and only the merge puts the newest first.
    run(
        &store.datastore,
        "UPDATE media_files SET created_at = printf('2026-01-01T00:%02d:00Z', rowid)",
        Vec::new(),
    )
    .await;
    let files =
        crate::media::search::media_file_store::MediaFileStore::new(store.datastore.clone());
    let requests = vec![
        (
            "series-partial".to_string(),
            expected(&["ep-partial-1", "ep-partial-2", "ep-partial-3"]),
        ),
        (
            "series-complete".to_string(),
            expected(&["ep-complete-1", "ep-complete-2"]),
        ),
    ];
    let ordered = |read: &[(String, Vec<scryer_application::EpisodeScopedMediaFile>)]| {
        read.iter()
            .map(|(title_id, files)| {
                (
                    title_id.clone(),
                    files
                        .iter()
                        .map(|file| file.media_file.id.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };

    let one = ordered(
        &files
            .list_live_media_files_for_titles_episode_ids(&requests)
            .await
            .expect("one statement should run"),
    );
    for (title_id, episode_ids) in &requests {
        let per_title = files
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
            .expect("per-title read should run")
            .into_iter()
            .map(|file| file.media_file.id)
            .collect::<Vec<_>>();
        let batched = &one.iter().find(|(id, _)| id == title_id).unwrap().1;
        assert!(batched.len() > 1, "{title_id} needs several files");
        assert_eq!(batched, &per_title, "{title_id}");
    }

    // Two binds per statement: one title and one episode each.
    let _lowered = crate::media::id_list_statements::lower_max_binds_for_tests(2);
    let split = ordered(
        &files
            .list_live_media_files_for_titles_episode_ids(&requests)
            .await
            .expect("split statements should run"),
    );
    assert_eq!(split, one);
}

/// The batch readers split long id lists across statements. Each one, given
/// enough ids to need several statements, with every seeded title in a different
/// one and some ids repeated, returns exactly what it returns for the seeded ids
/// alone in one statement: same rows, same order where the reader orders them.
#[tokio::test]
async fn batch_readers_split_across_statements_match_one_statement() {
    use scryer_application::{MediaFileRepository, ShowRepository};
    use std::fmt::Debug;

    let store = seeded_catalogue().await;
    let datastore = &store.datastore;
    // Distinct stamps and a resolution, so ordering and quality reads have
    // something to decide on.
    run(
        datastore,
        "UPDATE media_files
            SET created_at = printf('2026-01-01T00:%02d:00Z', rowid),
                video_width = 1920,
                video_height = 1080",
        Vec::new(),
    )
    .await;
    for (series, movie) in [
        ("series-partial", "movie-entity-emberfall"),
        ("series-partial", "movie-entity-cinderwake"),
        ("series-complete", "movie-entity-ashgrove"),
    ] {
        run(
            datastore,
            "INSERT INTO movie_entities (id, title, created_at, updated_at)
             VALUES ({}, {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            vec![
                SqlArg::Text(movie.to_string()),
                SqlArg::Text(movie.to_string()),
            ],
        )
        .await;
        run(
            datastore,
            "INSERT INTO series_movie_links (id, series_title_id, movie_entity_id, created_at,
                                             updated_at)
             VALUES ({}, {}, {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            vec![
                SqlArg::Text(format!("link-{movie}")),
                SqlArg::Text(series.to_string()),
                SqlArg::Text(movie.to_string()),
            ],
        )
        .await;
        run(
            datastore,
            "INSERT INTO title_metadata_rating_sources (movie_entity_id, source, sort_index)
             VALUES ({}, 'tmdb', 0)",
            vec![SqlArg::Text(movie.to_string())],
        )
        .await;
    }
    for (title, rating) in [("movie-complete", 7.5), ("series-partial", 8.0)] {
        run(
            datastore,
            "INSERT INTO title_metadata_rating_summaries (title_id, rating) VALUES ({}, {})",
            vec![SqlArg::Text(title.to_string()), SqlArg::F64(rating)],
        )
        .await;
        for (index, source) in ["imdb", "tmdb"].into_iter().enumerate() {
            run(
                datastore,
                "INSERT INTO title_metadata_rating_sources (title_id, source, sort_index)
                 VALUES ({}, {}, {})",
                vec![
                    SqlArg::Text(title.to_string()),
                    SqlArg::Text(source.to_string()),
                    SqlArg::I64(index as i64),
                ],
            )
            .await;
            run(
                datastore,
                "INSERT INTO title_metadata_external_ratings
                     (title_id, source, sort_index, value, normalized, votes)
                 VALUES ({}, {}, {}, {}, {}, 10)",
                vec![
                    SqlArg::Text(title.to_string()),
                    SqlArg::Text(source.to_string()),
                    SqlArg::I64(index as i64),
                    SqlArg::F64(rating),
                    SqlArg::F64(rating / 10.0),
                ],
            )
            .await;
        }
    }

    let seeded = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT id FROM titles ORDER BY id",
        &[],
    )
    .await
    .expect("seeded ids should load")
    .iter()
    .map(|row| row.text("id").unwrap())
    .collect::<Vec<_>>();
    // Three hundred unknown ids after each seeded id, in the byte order the readers
    // split on, spread the seeded titles across statements; the seeded ids are
    // listed again at the end.
    let mut padded = Vec::new();
    for id in &seeded {
        padded.push(id.clone());
        padded.extend((0..300).map(|number| format!("{id}-unknown-{number:03}")));
    }
    padded.extend(seeded.iter().rev().cloned());
    let episode_ids = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT id FROM episodes ORDER BY id",
        &[],
    )
    .await
    .expect("seeded episodes should load")
    .iter()
    .map(|row| row.text("id").unwrap())
    .collect::<Vec<_>>();
    let mut padded_episodes = Vec::new();
    for id in &episode_ids {
        padded_episodes.push(id.clone());
        padded_episodes.extend((0..1_000).map(|number| format!("{id}-unknown-{number:04}")));
    }
    padded_episodes.extend(episode_ids.iter().cloned());

    // At the engine's real ceiling even the padded lists are one statement,
    // with the ids exactly as given: titles bound up to three times each (the
    // ratings read), episodes once.
    let real_limit = crate::media::id_list_statements::max_binds(&datastore.read_exec());
    for (list, binds_per_id) in [(&padded, 3), (&padded_episodes, 1)] {
        let (statement_ids, per_statement) = crate::media::id_list_statements::id_list_statements(
            list,
            binds_per_id,
            0,
            real_limit,
            300,
        );
        assert!(matches!(statement_ids, std::borrow::Cow::Borrowed(_)));
        assert_eq!(statement_ids.chunks(per_statement).count(), 1);
        assert_eq!(statement_ids.as_ref(), list.as_slice());
    }

    // A ceiling the seeded lists fit under and the padded ones do not, so the
    // seeded reads are one statement and the padded reads split.
    let limit = 3 * seeded.len().max(episode_ids.len());
    assert!(padded.len() > limit && padded_episodes.len() > limit);
    let _lowered = crate::media::id_list_statements::lower_max_binds_for_tests(limit);

    fn rows<T: Debug>(values: &[T]) -> Vec<String> {
        values.iter().map(|value| format!("{value:?}")).collect()
    }
    fn sorted<T: Debug>(values: &[T]) -> Vec<String> {
        let mut rows = rows(values);
        rows.sort();
        rows
    }

    let files = crate::media::search::media_file_store::MediaFileStore::new(datastore.clone());
    let shows = crate::media::shows::store::ShowStore::new(datastore.clone());
    macro_rules! same {
        ($store:ident, $read:ident, $shape:ident, $ids:expr, $padded:expr) => {{
            let one = $shape(&$store.$read($ids).await.expect("one statement should run"));
            let split = $shape(
                &$store
                    .$read($padded)
                    .await
                    .expect("split statements should run"),
            );
            assert!(!one.is_empty(), "{} needs seeded rows", stringify!($read));
            assert_eq!(split, one, "{}", stringify!($read));
        }};
    }

    same!(files, list_media_files_for_titles, rows, &seeded, &padded);
    same!(
        files,
        list_title_media_size_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_collection_media_size_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_episode_media_size_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_title_quality_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_title_movie_media_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_title_episode_progress_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_cutoff_unmet_quality_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_collection_episode_progress_summaries,
        sorted,
        &seeded,
        &padded
    );
    same!(
        files,
        list_episode_media_availability,
        sorted,
        &seeded,
        &padded
    );
    same!(
        shows,
        list_series_movie_links_for_titles,
        rows,
        &seeded,
        &padded
    );
    same!(shows, list_episodes_for_titles, rows, &seeded, &padded);
    same!(
        shows,
        get_episodes_by_ids,
        sorted,
        &episode_ids,
        &padded_episodes
    );
    let one = shows
        .list_collections_for_titles(&seeded)
        .await
        .expect("one statement should run");
    let split = shows
        .list_collections_for_titles(&padded)
        .await
        .expect("split statements should run");
    assert!(!one.is_empty());
    assert_eq!(
        sorted(&split.into_iter().collect::<Vec<_>>()),
        sorted(&one.into_iter().collect::<Vec<_>>())
    );

    let one =
        crate::media::canonical_tags::load_title_metadata_ratings(datastore.read_exec(), &seeded)
            .await
            .expect("one statement should run");
    let split =
        crate::media::canonical_tags::load_title_metadata_ratings(datastore.read_exec(), &padded)
            .await
            .expect("split statements should run");
    assert_eq!(one.len(), 2);
    assert_eq!(format!("{split:?}"), format!("{one:?}"));
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
                file_state_counts: true,
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
    insert_title_in(datastore, id, facet, LIBRARY_ID).await;
}

async fn insert_title_in(datastore: &StoreDatastore, id: &str, facet: &str, library_id: &str) {
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
            SqlArg::Text(library_id.to_string()),
            SqlArg::Text(format!("root-{library_id}")),
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
