//! `title_release_days` is what a release-date window reads. These tests hold
//! it to the scan it replaced: the same titles for every window, kept right by
//! the triggers on `titles` through the real title writers, and rebuilt by the
//! migration for titles stored before it.

use super::catalog_order_tests::fixture_title;
use super::*;

const SQLITE_MIGRATION: &str =
    include_str!("../../../../../scryer/src/db/migrations/0343_title_release_days.sql");
const POSTGRES_MIGRATION: &str =
    include_str!("../../../../../scryer/src/db/postgres/migrations/0343_title_release_days.sql");

const RELEASE_DATE_LISTS: &str = "'theatrical_dates', 'digital_dates', 'physical_dates', \
     'any_market_theatrical_dates', 'any_market_digital_dates', 'any_market_physical_dates'";

/// The window filter as it was before the index: every stored date is tested
/// in place. Kept only as the reference the indexed filter is compared with.
fn scanned_release_date_window_clause(
    dialect: TitleCatalogSqlDialect,
    window: &scryer_application::TitleReleaseDateWindow,
    args: &mut Vec<SqlArg>,
) -> String {
    let mut date_may_fall_inside = |value: &str| -> String {
        let leading_date = match dialect {
            TitleCatalogSqlDialect::Sqlite => {
                format!("{value} GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*'")
            }
            TitleCatalogSqlDialect::Postgres => {
                format!("{value} ~ '^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'")
            }
        };
        let mut bounds = Vec::new();
        if let Some(earliest) = window.earliest.as_ref() {
            bounds.push(format!("SUBSTR({value}, 1, 10) >= {{}}"));
            args.push(SqlArg::Text(earliest.clone()));
        }
        if let Some(latest) = window.latest.as_ref() {
            bounds.push(format!("SUBSTR({value}, 1, 10) <= {{}}"));
            args.push(SqlArg::Text(latest.clone()));
        }
        let bounds = bounds.join(" AND ");
        format!("(CASE WHEN {leading_date} THEN {bounds} ELSE TRIM({value}) <> '' END)")
    };
    let first_aired = date_may_fall_inside("titles.first_aired");
    let digital = date_may_fall_inside("titles.digital_release_date");
    let listed = date_may_fall_inside("release_date.value");
    let listed = match dialect {
        TitleCatalogSqlDialect::Sqlite => format!(
            "(CASE WHEN json_valid(titles.movie_release_dates_json) THEN EXISTS (
                SELECT 1
                  FROM json_each(titles.movie_release_dates_json) AS release_list,
                       json_each(CASE WHEN release_list.type = 'array'
                                      THEN release_list.value ELSE '[]' END) AS release_date
                 WHERE release_list.key IN ({RELEASE_DATE_LISTS})
                   AND {listed}
            ) ELSE titles.movie_release_dates_json IS NOT NULL END)"
        ),
        TitleCatalogSqlDialect::Postgres => format!(
            "EXISTS (
                SELECT 1
                  FROM jsonb_each(
                           CASE WHEN jsonb_typeof(titles.movie_release_dates_json::jsonb) = 'object'
                                THEN titles.movie_release_dates_json::jsonb
                                ELSE jsonb_build_object() END
                       ) AS release_list(key, value)
                 CROSS JOIN LATERAL jsonb_array_elements_text(
                           CASE WHEN jsonb_typeof(release_list.value) = 'array'
                                THEN release_list.value
                                ELSE jsonb_build_array() END
                       ) AS release_date(value)
                 WHERE release_list.key IN ({RELEASE_DATE_LISTS})
                   AND {listed}
            )"
        ),
    };
    format!("({first_aired} OR {digital} OR {listed})")
}

/// The one fixture whose document is not JSON. The scan kept it on SQLite and
/// raised on PostgreSQL; the index keeps it on both.
const UNPARSEABLE: &str = "unparseable-json";

/// `(id, first_aired, digital_release_date, movie_release_dates_json)`, written
/// straight into the columns so the stored text is exactly what is listed.
const MOVIE_FIXTURES: &[(&str, Option<&str>, Option<&str>, Option<&str>)] = &[
    (
        "listed-theatrical-inside",
        None,
        None,
        Some(r#"{"market":"US","theatrical_dates":["2031-03-15"]}"#),
    ),
    (
        "listed-any-market-digital-on-latest",
        None,
        None,
        Some(r#"{"any_market_digital_dates":["2031-04-30T22:00:00Z"]}"#),
    ),
    (
        "listed-digital-inside",
        None,
        None,
        Some(r#"{"digital_dates":["2031-03-20"]}"#),
    ),
    (
        "listed-physical-inside",
        None,
        None,
        Some(r#"{"physical_dates":["2031-03-21"]}"#),
    ),
    (
        "listed-any-market-theatrical-inside",
        None,
        None,
        Some(r#"{"any_market_theatrical_dates":["2031-03-22"]}"#),
    ),
    (
        "listed-any-market-physical-inside",
        None,
        None,
        Some(r#"{"any_market_physical_dates":["2031-03-23"]}"#),
    ),
    ("first-aired-on-earliest", Some("2031-03-01"), None, None),
    ("digital-inside", None, Some("2031-04-02"), None),
    (
        "digital-timestamp-inside",
        None,
        Some("2031-04-03T23:30:00-07:00"),
        None,
    ),
    (
        "every-date-outside",
        Some("2030-12-31"),
        Some("2031-05-01"),
        Some(r#"{"physical_dates":["2031-02-28","2032-01-01"]}"#),
    ),
    (
        "dates-either-side-and-one-inside",
        Some("2029-01-01"),
        Some("2033-01-01"),
        Some(r#"{"theatrical_dates":["2028-06-01","2031-04-10","2034-06-01"]}"#),
    ),
    ("non-canonical-date", Some("Spring 2031"), None, None),
    ("leading-space-date", None, Some(" 2031-03-15"), None),
    ("blank-dates", Some("  "), Some(""), None),
    ("no-dates", None, None, None),
    (
        "date-only-outside-the-lists",
        None,
        None,
        Some(r#"{"market":"2031-03-15","theatrical_dates":"2031-03-15"}"#),
    ),
    (
        "listed-blank-and-null-members",
        None,
        None,
        Some(r#"{"theatrical_dates":["", "  ", null]}"#),
    ),
    (
        "listed-non-date-member",
        None,
        None,
        Some(r#"{"digital_dates":["soon", 20310315]}"#),
    ),
    (
        "document-is-an-array",
        None,
        None,
        Some(r#"["2031-03-15"]"#),
    ),
    ("document-is-a-string", None, None, Some(r#""2031-03-15""#)),
    (UNPARSEABLE, None, None, Some("{not json")),
];

async fn root_for(datastore: &StoreDatastore, library_id: &str) -> String {
    SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT id FROM library_roots WHERE library_id = {} LIMIT 1",
        &[SqlArg::Text(library_id.to_string())],
    )
    .await
    .unwrap()
    .expect("a fresh install has a root for each default library")
    .text("id")
    .unwrap()
}

async fn write_source_columns(
    datastore: &StoreDatastore,
    id: &str,
    first_aired: Option<&str>,
    digital: Option<&str>,
    listed: Option<&str>,
) {
    let updated = SqlRuntime::execute_write(
        datastore,
        "fixture",
        "UPDATE titles SET first_aired = {}, digital_release_date = {}, \
         movie_release_dates_json = {} WHERE id = {}",
        vec![
            SqlArg::OptText(first_aired.map(str::to_string)),
            SqlArg::OptText(digital.map(str::to_string)),
            SqlArg::OptText(listed.map(str::to_string)),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await
    .unwrap();
    assert_eq!(updated, 1, "{id}");
}

async fn release_days(datastore: &StoreDatastore, title_id: &str) -> Vec<String> {
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT release_day FROM title_release_days WHERE title_id = {} ORDER BY release_day",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await
    .unwrap()
    .iter()
    .map(|row| row.text("release_day").unwrap())
    .collect()
}

async fn every_release_day(datastore: &StoreDatastore) -> Vec<(String, String)> {
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT title_id, release_day FROM title_release_days ORDER BY title_id, release_day",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.text("title_id").unwrap(),
            row.text("release_day").unwrap(),
        )
    })
    .collect()
}

async fn ids(datastore: &StoreDatastore, sql: &str, args: &[SqlArg]) -> Vec<String> {
    SqlRuntime::fetch_all(datastore.read_exec(), sql, args)
        .await
        .unwrap()
        .iter()
        .map(|row| row.text("id").unwrap())
        .collect()
}

fn window(
    earliest: Option<&str>,
    latest: Option<&str>,
) -> scryer_application::TitleReleaseDateWindow {
    scryer_application::TitleReleaseDateWindow {
        earliest: earliest.map(str::to_string),
        latest: latest.map(str::to_string),
    }
}

/// The movies the indexed filter keeps for `window`, through the catalog's own
/// WHERE builder.
async fn indexed_movies(
    datastore: &StoreDatastore,
    library: &str,
    window: &scryer_application::TitleReleaseDateWindow,
) -> Vec<String> {
    let filter = TitleCatalogFilter {
        release_date_window: Some(window.clone()),
        ..Default::default()
    };
    let (predicate, args) = build_title_catalog_where_sql(
        Some(MediaFacet::Movie),
        &[library.to_string()],
        None,
        &filter,
        title_catalog_dialect_for_datastore(datastore),
    );
    ids(
        datastore,
        &format!("SELECT id FROM titles WHERE {predicate} ORDER BY id"),
        &args,
    )
    .await
}

/// The movies the scan kept for `window`.
async fn scanned_movies(
    datastore: &StoreDatastore,
    window: &scryer_application::TitleReleaseDateWindow,
) -> Vec<String> {
    let dialect = title_catalog_dialect_for_datastore(datastore);
    let mut args = Vec::new();
    let clause = scanned_release_date_window_clause(dialect, window, &mut args);
    let source = match dialect {
        TitleCatalogSqlDialect::Sqlite => "titles".to_string(),
        // The scan cast every document to jsonb and raised on one that is not
        // JSON. That row is held back from it here, behind a fence the planner
        // cannot move the cast across, and asserted on its own below.
        TitleCatalogSqlDialect::Postgres => {
            format!("(SELECT * FROM titles WHERE id <> '{UNPARSEABLE}' OFFSET 0) AS titles")
        }
    };
    ids(
        datastore,
        &format!("SELECT id FROM {source} WHERE titles.facet = 'movie' AND {clause} ORDER BY id"),
        &args,
    )
    .await
}

async fn run_migration_again(datastore: &StoreDatastore) {
    match datastore {
        StoreDatastore::Sqlite { pool, .. } => {
            sqlx::raw_sql(SQLITE_MIGRATION).execute(pool).await.unwrap();
        }
        StoreDatastore::Postgres { pool } => {
            sqlx::raw_sql(POSTGRES_MIGRATION)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn assert_release_days(datastore: StoreDatastore) {
    let dialect = title_catalog_dialect_for_datastore(&datastore);
    let store = TitleStore::new(datastore.clone());
    let movies = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let movie_root = root_for(&datastore, &movies).await;

    for (id, first_aired, digital, listed) in MOVIE_FIXTURES {
        let mut title = fixture_title(id, &format!("Fixture {id}"), None);
        title.root_folder_id = movie_root.clone();
        TitleRepository::create(&store, title).await.unwrap();
        assert!(
            release_days(&datastore, id).await.is_empty(),
            "{id}: a title without dates holds no days"
        );
        write_source_columns(&datastore, id, *first_aired, *digital, *listed).await;
    }
    // A series whose date is inside every window below: the facet term keeps
    // it out of a movie read whatever the index holds for it.
    let series = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut series_title = fixture_title("series-inside", "Fixture series", None);
    series_title.facet = MediaFacet::Series;
    series_title.library_id = series.clone();
    series_title.root_folder_id = root_for(&datastore, &series).await;
    series_title.first_aired = Some("2031-03-15".into());
    TitleRepository::create(&store, series_title).await.unwrap();
    assert_eq!(
        release_days(&datastore, "series-inside").await,
        ["2031-03-15"],
        "every facet is indexed"
    );

    // What each kind of stored value turns into.
    for (id, expected) in [
        ("listed-any-market-digital-on-latest", vec!["2031-04-30"]),
        ("digital-timestamp-inside", vec!["2031-04-03"]),
        (
            "every-date-outside",
            vec!["2030-12-31", "2031-02-28", "2031-05-01", "2032-01-01"],
        ),
        ("non-canonical-date", vec![""]),
        ("leading-space-date", vec![""]),
        ("blank-dates", vec![]),
        ("no-dates", vec![]),
        ("date-only-outside-the-lists", vec![]),
        ("listed-blank-and-null-members", vec![]),
        ("listed-non-date-member", vec![""]),
        ("document-is-an-array", vec![]),
        ("document-is-a-string", vec![]),
        (UNPARSEABLE, vec![""]),
    ] {
        assert_eq!(release_days(&datastore, id).await, expected, "{id}");
    }

    // The window the scan's own test pinned, with the titles it kept.
    let pinned = window(Some("2031-03-01"), Some("2031-04-30"));
    assert_eq!(
        indexed_movies(&datastore, &movies, &pinned).await,
        [
            "dates-either-side-and-one-inside",
            "digital-inside",
            "digital-timestamp-inside",
            "first-aired-on-earliest",
            "leading-space-date",
            "listed-any-market-digital-on-latest",
            "listed-any-market-physical-inside",
            "listed-any-market-theatrical-inside",
            "listed-digital-inside",
            "listed-non-date-member",
            "listed-physical-inside",
            "listed-theatrical-inside",
            "non-canonical-date",
            UNPARSEABLE,
        ]
    );

    // The index against the scan, window by window.
    for (earliest, latest) in [
        (Some("2031-03-01"), Some("2031-04-30")),
        (Some("2031-03-15"), Some("2031-03-15")),
        (Some("2031-03-02"), Some("2031-03-14")),
        (Some("2031-05-01"), Some("2031-05-01")),
        (Some("1900-01-01"), Some("1900-12-31")),
        (Some("2031-04-30"), None),
        (Some("2034-06-02"), None),
        (None, Some("2031-02-28")),
        (None, Some("2028-05-31")),
        (Some("0000-01-01"), Some("9999-12-31")),
    ] {
        let window = window(earliest, latest);
        let indexed = indexed_movies(&datastore, &movies, &window).await;
        let scanned = scanned_movies(&datastore, &window).await;
        match dialect {
            TitleCatalogSqlDialect::Sqlite => assert_eq!(indexed, scanned, "{window:?}"),
            TitleCatalogSqlDialect::Postgres => {
                assert!(
                    indexed.iter().any(|id| id == UNPARSEABLE),
                    "{window:?}: a document that is not JSON is kept, not raised on"
                );
                let judged = indexed
                    .into_iter()
                    .filter(|id| id != UNPARSEABLE)
                    .collect::<Vec<_>>();
                assert_eq!(judged, scanned, "{window:?}");
            }
        }
    }

    // A window with no bound is no filter at all.
    let predicate_for = |release_date_window| {
        build_title_catalog_where_sql(
            Some(MediaFacet::Movie),
            std::slice::from_ref(&movies),
            None,
            &TitleCatalogFilter {
                release_date_window,
                ..Default::default()
            },
            dialect,
        )
        .0
    };
    assert_eq!(
        predicate_for(Some(scryer_application::TitleReleaseDateWindow::default())),
        predicate_for(None)
    );

    // The migration rebuilds, for titles stored before it, exactly what the
    // triggers derive, and running it again changes nothing.
    let derived = every_release_day(&datastore).await;
    assert!(!derived.is_empty());
    SqlRuntime::execute_write(
        &datastore,
        "fixture",
        "DELETE FROM title_release_days",
        vec![],
    )
    .await
    .unwrap();
    run_migration_again(&datastore).await;
    assert_eq!(every_release_day(&datastore).await, derived);
    run_migration_again(&datastore).await;
    assert_eq!(every_release_day(&datastore).await, derived);

    // The title writers keep the days through the triggers. The stored
    // document here is the one the store serialises, not a hand-written one.
    let mut written = fixture_title("written-by-the-store", "Fixture written", None);
    written.root_folder_id = movie_root.clone();
    written.first_aired = Some("2031-06-01".into());
    written.digital_release_date = Some("2031-09-01T04:00:00Z".into());
    written.movie_release_dates = Some(scryer_domain::MovieReleaseDates {
        market: "US".into(),
        fetched: true,
        theatrical_dates: vec!["2031-06-01".into(), "2031-06-05".into()],
        digital_dates: vec!["2031-09-01".into()],
        physical_dates: vec!["2031-10-01".into()],
        any_market_theatrical_dates: vec!["2031-05-20".into()],
        any_market_digital_dates: vec!["2031-08-15".into()],
        any_market_physical_dates: vec!["2031-09-20".into()],
        any_market_home_dates_unusable: false,
    });
    TitleRepository::create(&store, written).await.unwrap();
    let written_days = [
        "2031-05-20",
        "2031-06-01",
        "2031-06-05",
        "2031-08-15",
        "2031-09-01",
        "2031-09-20",
        "2031-10-01",
    ];
    assert_eq!(
        release_days(&datastore, "written-by-the-store").await,
        written_days
    );

    // A row no derivation produces marks whether the days were rewritten.
    let marker = "9999-12-31";
    SqlRuntime::execute_write(
        &datastore,
        "fixture",
        "INSERT INTO title_release_days (title_id, release_day) VALUES ({}, {})",
        vec![
            SqlArg::Text("written-by-the-store".into()),
            SqlArg::Text(marker.into()),
        ],
    )
    .await
    .unwrap();
    // A rename restates every column of the title, the dates unchanged.
    TitleRepository::update_metadata(
        &store,
        "written-by-the-store",
        Some("Fixture renamed".into()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        release_days(&datastore, "written-by-the-store")
            .await
            .iter()
            .any(|day| day == marker),
        "a write that leaves the dates alone must not rewrite the days"
    );

    // A metadata refresh changes the dates through the title upsert, where a
    // day repeated across the sources must not fail the write: the column and
    // two lists share one day, and two values are not dates at all.
    let mut refreshed = fixture_title("refreshed-by-the-store", "Fixture refreshed", None);
    refreshed.root_folder_id = movie_root.clone();
    refreshed.first_aired = Some("2032-01-01".into());
    TitleRepository::create(&store, refreshed).await.unwrap();
    TitleRepository::update_title_hydrated_metadata(
        &store,
        "refreshed-by-the-store",
        scryer_application::TitleMetadataUpdate {
            first_aired: Some("2032-02-02".into()),
            digital_release_date: Some("Spring 2032".into()),
            movie_release_dates: Some(scryer_domain::MovieReleaseDates {
                market: "US".into(),
                fetched: true,
                theatrical_dates: vec!["2032-02-02".into(), "TBA".into()],
                digital_dates: vec![],
                physical_dates: vec![],
                any_market_theatrical_dates: vec!["2032-02-02".into()],
                any_market_digital_dates: vec![],
                any_market_physical_dates: vec![],
                any_market_home_dates_unusable: false,
            }),
            ..Default::default()
        },
    )
    .await
    .expect("a refresh that repeats a day must still save");
    assert_eq!(
        release_days(&datastore, "refreshed-by-the-store").await,
        ["", "2032-02-02"]
    );

    // Each source column, changed on its own, rewrites them.
    for (column, value, expected) in [
        (
            "first_aired",
            Some("2031-07-04"),
            vec![
                "2031-05-20",
                "2031-06-01",
                "2031-06-05",
                "2031-07-04",
                "2031-08-15",
                "2031-09-01",
                "2031-09-20",
                "2031-10-01",
            ],
        ),
        (
            "digital_release_date",
            None,
            vec![
                "2031-05-20",
                "2031-06-01",
                "2031-06-05",
                "2031-07-04",
                "2031-08-15",
                "2031-09-01",
                "2031-09-20",
                "2031-10-01",
            ],
        ),
        (
            "movie_release_dates_json",
            Some(r#"{"theatrical_dates":["2031-11-11"]}"#),
            vec!["2031-07-04", "2031-11-11"],
        ),
        ("movie_release_dates_json", None, vec!["2031-07-04"]),
        ("first_aired", None, vec![]),
    ] {
        SqlRuntime::execute_write(
            &datastore,
            "fixture",
            &format!("UPDATE titles SET {column} = {{}} WHERE id = {{}}"),
            vec![
                SqlArg::OptText(value.map(str::to_string)),
                SqlArg::Text("written-by-the-store".into()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            release_days(&datastore, "written-by-the-store").await,
            expected,
            "{column} = {value:?}"
        );
    }

    // A deleted title takes its days with it.
    assert!(
        !release_days(&datastore, "every-date-outside")
            .await
            .is_empty()
    );
    SqlRuntime::execute_write(
        &datastore,
        "fixture",
        "DELETE FROM titles WHERE id = {}",
        vec![SqlArg::Text("every-date-outside".into())],
    )
    .await
    .unwrap();
    assert!(
        release_days(&datastore, "every-date-outside")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_release_days_match_the_scan_and_follow_title_writes() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .unwrap();
    let datastore = StoreDatastore::sqlite(
        pool.clone(),
        std::sync::Arc::new(tokio::sync::Mutex::new(())),
    );
    assert_release_days(datastore).await;

    // Both arms of the filter read the day index rather than the table.
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM titles WHERE titles.id IN (
            SELECT title_id FROM title_release_days WHERE release_day >= ? AND release_day <= ?
            UNION ALL
            SELECT title_id FROM title_release_days WHERE release_day = '')",
    )
    .bind("2031-03-01")
    .bind("2031-04-30")
    .fetch_all(&pool)
    .await
    .unwrap();
    let plan = plan.into_iter().map(|row| row.3).collect::<Vec<_>>();
    let day_index_searches = plan
        .iter()
        .filter(|detail| {
            detail.contains("SEARCH title_release_days")
                && detail.contains("idx_title_release_days_day")
        })
        .count();
    assert_eq!(day_index_searches, 2, "{plan:?}");
    assert!(
        !plan
            .iter()
            .any(|detail| detail.contains("SCAN title_release_days")),
        "{plan:?}"
    );
}

#[tokio::test]
async fn postgres_release_days_match_the_scan_and_follow_title_writes() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("release_days_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .unwrap();
    scryer_infrastructure_datastore::postgres::replay_source_catalog_for_fresh_install(&pool, None)
        .await
        .unwrap();
    assert_release_days(StoreDatastore::Postgres { pool: pool.clone() }).await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
