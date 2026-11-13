use super::*;

pub(super) fn fixture_title(id: &str, name: &str, year: Option<i32>) -> Title {
    Title {
        id: id.into(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        name: name.into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        root_folder_id: String::new(),
        created_by: None,
        created_at: Utc::now(),
        year,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: "untrusted-caller-key".into(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: Some(1.0),
        canonical_tags: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: Some("eng".into()),
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        movie_release_dates: None,
        folder_path: None,
    }
}

async fn assert_catalog_order(datastore: StoreDatastore) {
    let dialect = title_catalog_dialect_for_datastore(&datastore);
    let library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let root = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT id FROM library_roots WHERE library_id = {} LIMIT 1",
        &[SqlArg::Text(library.clone())],
    )
    .await
    .unwrap()
    .unwrap()
    .text("id")
    .unwrap();
    let store = TitleStore::new(datastore.clone());
    let mut titles = Vec::new();
    for (i, name) in [
        "The Meridian",
        "Meridian",
        "Ｍｅｒｉｄｉａｎ",
        "ÉCLAIR",
        "éclair",
        "Eclair",
        "鋼の錬金術師",
        "  Meridian  ",
    ]
    .iter()
    .enumerate()
    {
        for year in [Some(1999), Some(2020), None] {
            let mut title = fixture_title(&format!("title-{i}-{year:?}"), name, year);
            title.root_folder_id = root.clone();
            TitleRepository::create(&store, title.clone())
                .await
                .unwrap();
            titles.push(title);
        }
    }
    for direction in [SortDirection::Asc, SortDirection::Desc] {
        let directed = |order: std::cmp::Ordering| match direction {
            SortDirection::Asc => order,
            SortDirection::Desc => order.reverse(),
        };
        let mut expected = titles.clone();
        expected.sort_by(|a, b| {
            directed(
                title_catalog_sort_key(&a.name, Some("eng"))
                    .cmp(&title_catalog_sort_key(&b.name, Some("eng"))),
            )
            .then_with(|| {
                directed(
                    title_catalog_name_tie_key(&a.name).cmp(&title_catalog_name_tie_key(&b.name)),
                )
            })
            .then_with(|| a.year.is_none().cmp(&b.year.is_none()))
            .then_with(|| directed(a.year.cmp(&b.year)))
            .then_with(|| directed(a.id.cmp(&b.id)))
        });
        let mut actual = Vec::new();
        for offset in (0..titles.len()).step_by(5) {
            let (sql, args) = build_title_catalog_page_sql(
                Some(MediaFacet::Movie),
                std::slice::from_ref(&library),
                None,
                &TitleCatalogFilter::default(),
                TitleCatalogSort {
                    key: TitleCatalogSortKey::Title,
                    direction,
                    ..Default::default()
                },
                5,
                offset,
                dialect,
                TitleListProjection::default(),
            );
            actual.extend(
                SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| row.text("id").unwrap()),
            );
        }
        assert_eq!(
            actual,
            expected.iter().map(|t| t.id.clone()).collect::<Vec<_>>()
        );
    }
    // The same page path must still honor library authorization and active filters.
    for (libraries, filter) in [
        (
            vec!["invisible-library".into()],
            TitleCatalogFilter::default(),
        ),
        (
            vec![library.clone()],
            TitleCatalogFilter {
                monitored: Some(false),
                ..Default::default()
            },
        ),
    ] {
        let (sql, args) = build_title_catalog_page_sql(
            Some(MediaFacet::Movie),
            &libraries,
            None,
            &filter,
            TitleCatalogSort::default(),
            100,
            0,
            dialect,
            TitleListProjection::default(),
        );
        assert!(
            SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let id = &titles[0].id;
    TitleRepository::update_metadata(&store, id, Some("ＺÉＢＲＡ".into()), None, None, None)
        .await
        .unwrap();
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT name, catalog_sort_key, catalog_name_tie_key FROM titles WHERE id = {}",
        &[SqlArg::Text(id.clone())],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        row.text("catalog_name_tie_key").unwrap(),
        title_catalog_name_tie_key("ＺÉＢＲＡ")
    );
    assert_eq!(
        row.text("catalog_sort_key").unwrap(),
        title_catalog_sort_key("ＺÉＢＲＡ", Some("eng"))
    );
    TitleRepository::update_title_hydrated_metadata(
        &store,
        id,
        TitleMetadataUpdate {
            name: Some("Ångström".into()),
            metadata_language: Some("swe".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT catalog_sort_key, catalog_name_tie_key FROM titles WHERE id = {}",
        &[SqlArg::Text(id.clone())],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        row.text("catalog_sort_key").unwrap(),
        title_catalog_sort_key("Ångström", Some("swe"))
    );
    assert_eq!(
        row.text("catalog_name_tie_key").unwrap(),
        title_catalog_name_tie_key("Ångström")
    );

    assert_language_clear_catalog_keys(&store, &datastore, &root).await;
}

async fn assert_language_clear_catalog_keys(
    store: &TitleStore,
    datastore: &StoreDatastore,
    root: &str,
) {
    let root = root.to_owned();
    SqlRuntime::run_in_transaction(datastore, "language_clear_fixture", move |tx| {
        let root = root.clone();
        Box::pin(async move {
            for index in 0..1005 {
                let id = if index == 0 {
                    String::new()
                } else {
                    format!("language-clear-{index:04}")
                };
                let name = match index {
                    0 => "阿",
                    1 => "安",
                    _ => "ＺÉＢＲＡ",
                };
                let mut title = fixture_title(&id, name, None);
                title.root_folder_id = root.clone();
                title.metadata_language = Some("jpn".into());
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    TITLE_INSERT_SQL,
                    &title_write_args(&title, None, 0),
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
    .unwrap();

    assert!(title_catalog_sort_key("阿", Some("jpn")) < title_catalog_sort_key("安", Some("jpn")));
    assert!(title_catalog_sort_key("阿", None) > title_catalog_sort_key("安", None));
    let expected = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT id FROM titles WHERE metadata_language IS NOT NULL",
        &[],
    )
    .await
    .unwrap()
    .len() as u64;
    assert_eq!(
        TitleRepository::clear_metadata_language_for_all(store)
            .await
            .unwrap(),
        expected
    );
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT name, metadata_language, catalog_sort_key, catalog_name_tie_key FROM titles",
        &[],
    )
    .await
    .unwrap();
    for row in rows {
        let name = row.text("name").unwrap();
        assert_eq!(row.opt_text("metadata_language").unwrap(), None);
        assert_eq!(
            row.text("catalog_sort_key").unwrap(),
            title_catalog_sort_key(&name, None)
        );
        assert_eq!(
            row.text("catalog_name_tie_key").unwrap(),
            title_catalog_name_tie_key(&name)
        );
    }
    let sql = format!(
        "SELECT name FROM titles WHERE name IN ('阿', '安') ORDER BY {}",
        title_catalog_title_order_sql(title_catalog_dialect_for_datastore(datastore), "ASC")
    );
    let names = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &[])
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.text("name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["安", "阿"]);
    assert_eq!(
        TitleRepository::clear_metadata_language_for_all(store)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn sqlite_catalog_order_fresh_schema_writes_and_pagination() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .unwrap();
    assert_catalog_order(StoreDatastore::sqlite(
        pool,
        Arc::new(tokio::sync::Mutex::new(())),
    ))
    .await;
}

#[tokio::test]
async fn postgres_catalog_order_fresh_schema_writes_and_pagination() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("catalog_order_{}", uuid::Uuid::new_v4().simple());
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
    assert_catalog_order(StoreDatastore::Postgres { pool: pool.clone() }).await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}

// Opt-in, reproducible database-only benchmark. No timing assertions: shared
// runners may be slow. Correctness and index plans are asserted independently.
async fn benchmark_catalog_order(datastore: StoreDatastore) {
    let dialect = title_catalog_dialect_for_datastore(&datastore);
    SqlRuntime::execute_write(&datastore, "fixture", "CREATE TEMP TABLE titles (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, facet TEXT NOT NULL, library_id TEXT NOT NULL, year INTEGER, popularity REAL, catalog_sort_key TEXT NOT NULL, catalog_name_tie_key TEXT NOT NULL)", vec![]).await.unwrap();
    SqlRuntime::run_in_transaction(&datastore, "fixture", |tx| {
        Box::pin(async move {
            for i in 0..10_000 {
                let name = format!(
                    "{}Fixture {:05}",
                    if i % 2 == 0 { "The " } else { "" },
                    i / 4
                );
                tx.execute(
                    "INSERT INTO titles VALUES ({}, {}, 'movie', 'fixture', {}, {}, {}, {})",
                    &[
                        SqlArg::Text(format!("title-{i:05}")),
                        SqlArg::Text(name.clone()),
                        SqlArg::OptI64((i % 3 != 0).then_some(1990 + i % 30)),
                        SqlArg::F64((i % 100) as f64),
                        SqlArg::Text(title_catalog_sort_key(&name, Some("eng"))),
                        SqlArg::Text(title_catalog_name_tie_key(&name)),
                    ],
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
    .unwrap();
    SqlRuntime::execute_write(
        &datastore,
        "fixture",
        "CREATE INDEX idx_titles_catalog_sort_key ON titles(catalog_sort_key, name, year, id)",
        vec![],
    )
    .await
    .unwrap();
    let mut old_name = "name".to_owned();
    for value in std::iter::once('\u{3000}').chain('\u{ff01}'..='\u{ff5e}') {
        let replacement = if value == '\u{3000}' {
            ' '
        } else {
            char::from_u32(value as u32 - 0xfee0).unwrap()
        };
        old_name = format!(
            "REPLACE({old_name}, {}, {})",
            sql_string_literal(&value.to_string()),
            sql_string_literal(&replacement.to_string())
        );
    }
    let mut before_results = Vec::new();
    for optimized in [false, true] {
        if optimized {
            SqlRuntime::execute_write(
                &datastore,
                "fixture",
                "DROP INDEX idx_titles_catalog_sort_key",
                vec![],
            )
            .await
            .unwrap();
            let indexes = match dialect {
                TitleCatalogSqlDialect::Sqlite => include_str!(
                    "../../../../../scryer/src/db/migrations/0327_catalog_order_indexes.sql"
                ),
                TitleCatalogSqlDialect::Postgres => include_str!(
                    "../../../../../scryer/src/db/postgres/migrations/0327_catalog_order_indexes.sql"
                ),
            };
            for statement in indexes.split(';').filter(|s| !s.trim().is_empty()) {
                SqlRuntime::execute_write(&datastore, "fixture", statement, vec![])
                    .await
                    .unwrap();
            }
        }
        SqlRuntime::execute_write(&datastore, "fixture", "ANALYZE titles", vec![])
            .await
            .unwrap();
        for (case, direction) in [("title", "ASC"), ("title", "DESC"), ("popularity", "DESC")] {
            let tie_direction = if case == "title" { direction } else { "ASC" };
            let order = if optimized {
                title_catalog_title_order_sql(dialect, tie_direction)
            } else {
                format!(
                    "catalog_sort_key {tie_direction}, LOWER(TRIM({old_name})) {tie_direction}, CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year {tie_direction}, id {tie_direction}"
                )
            };
            let prefix = if case == "popularity" {
                "CASE WHEN popularity IS NULL THEN 1 ELSE 0 END ASC, popularity DESC, "
            } else {
                ""
            };
            for page_size in [300, 10_000] {
                let sql = format!(
                    "SELECT id, name, year FROM titles WHERE facet = 'movie' AND library_id = 'fixture' ORDER BY {prefix}{order} LIMIT {{}} OFFSET {{}}"
                );
                let explain = match dialect {
                    TitleCatalogSqlDialect::Sqlite => "EXPLAIN QUERY PLAN",
                    TitleCatalogSqlDialect::Postgres => "EXPLAIN (ANALYZE, BUFFERS)",
                };
                let plan = SqlRuntime::fetch_all(
                    datastore.read_exec(),
                    &format!("{explain} {sql}"),
                    &[SqlArg::I64(page_size), SqlArg::I64(0)],
                )
                .await
                .unwrap();
                let column = match dialect {
                    TitleCatalogSqlDialect::Sqlite => "detail",
                    TitleCatalogSqlDialect::Postgres => "QUERY PLAN",
                };
                let plan: Vec<_> = plan.iter().map(|r| r.text(column).unwrap()).collect();
                eprintln!(
                    "optimized={optimized} {case} {direction} page={page_size} plan={plan:?}"
                );
                if optimized && case == "title" && page_size == 300 {
                    assert!(
                        plan.iter().any(|p| p.contains(&format!(
                            "idx_titles_catalog_order_{}",
                            direction.to_lowercase()
                        ))),
                        "{plan:?}"
                    );
                    assert!(
                        !plan
                            .iter()
                            .any(|p| p.contains("Sort") || p.contains("TEMP B-TREE")),
                        "{plan:?}"
                    );
                }
                let mut timings = Vec::new();
                for _ in 0..5 {
                    let start = std::time::Instant::now();
                    let mut ids = Vec::new();
                    for offset in (0..10_000).step_by(page_size as usize) {
                        let rows = SqlRuntime::fetch_all(
                            datastore.read_exec(),
                            &sql,
                            &[SqlArg::I64(page_size), SqlArg::I64(offset)],
                        )
                        .await
                        .unwrap();
                        ids.extend(rows.into_iter().map(|r| r.text("id").unwrap()));
                    }
                    timings.push(start.elapsed().as_secs_f64() * 1000.0);
                    assert_eq!(ids.len(), 10_000);
                    if optimized {
                        assert!(before_results.contains(&ids));
                    } else if !before_results.contains(&ids) {
                        before_results.push(ids);
                    }
                }
                timings.sort_by(f64::total_cmp);
                eprintln!(
                    "CATALOG_BENCH optimized={optimized} sort={case}-{direction} page={page_size} requests={} median_ms={:.3}",
                    10_000_usize.div_ceil(page_size as usize),
                    timings[2]
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "synthetic 10k-title benchmark"]
async fn sqlite_catalog_order_benchmark() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    benchmark_catalog_order(StoreDatastore::sqlite(
        pool,
        Arc::new(tokio::sync::Mutex::new(())),
    ))
    .await;
}

#[tokio::test]
#[ignore = "synthetic 10k-title benchmark"]
async fn postgres_catalog_order_benchmark() {
    let url = std::env::var("SCRYER_TEST_POSTGRES_URL").expect("isolated test database required");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    benchmark_catalog_order(StoreDatastore::Postgres { pool }).await;
}

/// What a fixture title is expected to sort by. `None` is a title the aggregate
/// finds nothing for, which every sort places as the "missing" value.
struct AggregateFixture {
    id: &'static str,
    name: &'static str,
    facet: MediaFacet,
    monitored: bool,
    year: Option<i32>,
    size: Option<i64>,
    episodes: Option<(i64, i64)>,
    quality: Option<i64>,
    rating: Option<f64>,
}

const AGGREGATE_FIXTURES: &[AggregateFixture] = &[
    AggregateFixture {
        id: "s-emberfall",
        name: "Emberfall",
        facet: MediaFacet::Series,
        monitored: true,
        year: Some(2001),
        size: Some(300),
        episodes: Some((2, 2)),
        quality: Some(4),
        rating: Some(7.5),
    },
    AggregateFixture {
        id: "s-cinderwake",
        name: "Cinderwake",
        facet: MediaFacet::Series,
        monitored: false,
        year: None,
        size: Some(300),
        episodes: Some((1, 4)),
        quality: Some(6),
        rating: None,
    },
    AggregateFixture {
        id: "s-ashgrove",
        name: "Ashgrove",
        facet: MediaFacet::Series,
        monitored: true,
        year: Some(1999),
        size: None,
        episodes: None,
        quality: None,
        rating: None,
    },
    AggregateFixture {
        id: "s-ashgrove-two",
        name: "Ashgrove",
        facet: MediaFacet::Series,
        monitored: true,
        year: Some(1999),
        size: None,
        episodes: None,
        quality: None,
        rating: None,
    },
    AggregateFixture {
        id: "s-emberfall-deep",
        name: "Emberfall Deep",
        facet: MediaFacet::Series,
        monitored: true,
        year: Some(2010),
        size: Some(300),
        episodes: Some((2, 2)),
        quality: Some(4),
        rating: None,
    },
    AggregateFixture {
        id: "m-cinderwake-film",
        name: "Cinderwake Film",
        facet: MediaFacet::Movie,
        monitored: true,
        year: Some(2005),
        size: Some(500),
        episodes: None,
        quality: Some(8),
        rating: Some(9.0),
    },
    AggregateFixture {
        id: "s-ashgrove-link",
        name: "Ashgrove Link",
        facet: MediaFacet::Series,
        monitored: true,
        year: None,
        size: Some(700),
        episodes: Some((1, 1)),
        quality: None,
        rating: None,
    },
    AggregateFixture {
        id: "a-emberfall",
        name: "Emberfall",
        facet: MediaFacet::Anime,
        monitored: true,
        year: Some(2001),
        size: Some(300),
        episodes: Some((1, 2)),
        quality: Some(6),
        rating: Some(7.5),
    },
    AggregateFixture {
        id: "a-cinderwake",
        name: "Cinderwake Ember",
        facet: MediaFacet::Anime,
        monitored: false,
        year: None,
        size: Some(70),
        episodes: Some((0, 1)),
        quality: Some(8),
        rating: None,
    },
    AggregateFixture {
        id: "a-ashgrove",
        name: "Ashgrove",
        facet: MediaFacet::Anime,
        monitored: true,
        year: Some(1999),
        size: None,
        episodes: None,
        quality: None,
        rating: None,
    },
];

fn aggregate_fixture_library(fixture: &AggregateFixture) -> String {
    scryer_domain::default_library_id_for_facet(&fixture.facet)
}

async fn aggregate_exec(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "aggregate_sort_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn aggregate_collection(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    collection_type: &str,
    index: &str,
    ordered_path: Option<&str>,
) {
    aggregate_exec(
        datastore,
        "INSERT INTO collections (id, title_id, collection_type, collection_index, ordered_path,
                                  created_at)
         VALUES ({}, {}, {}, {}, {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.into()),
            SqlArg::Text(title_id.into()),
            SqlArg::Text(collection_type.into()),
            SqlArg::Text(index.into()),
            SqlArg::OptText(ordered_path.map(str::to_string)),
        ],
    )
    .await;
}

async fn aggregate_episode(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    collection_id: &str,
    number: &str,
    title: &str,
) {
    aggregate_exec(
        datastore,
        "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number,
                               season_number, title, air_date, monitored, created_at)
         VALUES ({}, {}, {}, 'standard', {}, '1', {}, '2001-02-03', {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.into()),
            SqlArg::Text(title_id.into()),
            SqlArg::Text(collection_id.into()),
            SqlArg::Text(number.into()),
            SqlArg::Text(title.into()),
            SqlArg::Bool(true),
        ],
    )
    .await;
}

struct AggregateFile<'a> {
    id: &'a str,
    title_id: &'a str,
    path: &'a str,
    size: i64,
    role: &'a str,
    scan_status: &'a str,
    width: i64,
    height: i64,
    quality_id: &'a str,
}

async fn aggregate_file(datastore: &StoreDatastore, file: AggregateFile<'_>) {
    aggregate_exec(
        datastore,
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, role,
                                  video_width, video_height, quality_id, created_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(file.id.into()),
            SqlArg::Text(file.title_id.into()),
            SqlArg::Text(file.path.into()),
            SqlArg::I64(file.size),
            SqlArg::Text(file.scan_status.into()),
            SqlArg::Text(file.role.into()),
            SqlArg::I64(file.width),
            SqlArg::I64(file.height),
            SqlArg::Text(file.quality_id.into()),
        ],
    )
    .await;
}

async fn aggregate_map(datastore: &StoreDatastore, file_id: &str, episode_id: &str) {
    aggregate_map_as(datastore, file_id, episode_id, "primary").await;
}

/// An episode holds one primary mapping; any further file maps as additional.
async fn aggregate_map_as(datastore: &StoreDatastore, file_id: &str, episode_id: &str, role: &str) {
    aggregate_exec(
        datastore,
        "INSERT INTO file_episode_map (file_id, episode_id, role, is_filler)
         VALUES ({}, {}, {}, {})",
        vec![
            SqlArg::Text(file_id.into()),
            SqlArg::Text(episode_id.into()),
            SqlArg::Text(role.into()),
            SqlArg::Bool(false),
        ],
    )
    .await;
}

fn file<'a>(
    id: &'a str,
    title_id: &'a str,
    size: i64,
    (width, height): (i64, i64),
) -> AggregateFile<'a> {
    AggregateFile {
        id,
        title_id,
        path: id,
        size,
        role: "primary",
        scan_status: "complete",
        width,
        height,
        quality_id: "",
    }
}

/// Files, episodes, links and ratings that produce exactly the values in
/// [`AGGREGATE_FIXTURES`], including every row the aggregates must leave out.
async fn seed_aggregate_fixtures(datastore: &StoreDatastore) {
    let store = TitleStore::new(datastore.clone());
    for fixture in AGGREGATE_FIXTURES {
        let library = aggregate_fixture_library(fixture);
        let root = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT id FROM library_roots WHERE library_id = {} LIMIT 1",
            &[SqlArg::Text(library.clone())],
        )
        .await
        .unwrap()
        .unwrap()
        .text("id")
        .unwrap();
        let mut title = fixture_title(fixture.id, fixture.name, fixture.year);
        title.library_id = library;
        title.facet = fixture.facet.clone();
        title.monitored = fixture.monitored;
        title.root_folder_id = root;
        TitleRepository::create(&store, title).await.unwrap();
    }

    // Emberfall: both episodes owned; the lowest primary is 720p.
    aggregate_collection(
        datastore,
        "c-s-emberfall",
        "s-emberfall",
        "season",
        "1",
        None,
    )
    .await;
    for (episode, file_id, size, dims) in [
        ("e-s-emberfall-1", "f-s-emberfall-1", 100, (1920, 1080)),
        ("e-s-emberfall-2", "f-s-emberfall-2", 200, (1280, 720)),
    ] {
        aggregate_episode(
            datastore,
            episode,
            "s-emberfall",
            "c-s-emberfall",
            "1",
            "Pilot",
        )
        .await;
        aggregate_file(datastore, file(file_id, "s-emberfall", size, dims)).await;
        aggregate_map(datastore, file_id, episode).await;
    }

    // Cinderwake: one of four episodes owned, by a 1080p file and an empty file
    // waiting for review (which quality ignores).
    aggregate_collection(
        datastore,
        "c-s-cinderwake",
        "s-cinderwake",
        "season",
        "1",
        None,
    )
    .await;
    for number in 1..=4 {
        aggregate_episode(
            datastore,
            &format!("e-s-cinderwake-{number}"),
            "s-cinderwake",
            "c-s-cinderwake",
            &number.to_string(),
            "Cinders",
        )
        .await;
    }
    aggregate_file(
        datastore,
        AggregateFile {
            quality_id: "1080p",
            ..file("f-s-cinderwake-1", "s-cinderwake", 300, (0, 0))
        },
    )
    .await;
    aggregate_file(
        datastore,
        AggregateFile {
            scan_status: "review_required",
            ..file("f-s-cinderwake-2", "s-cinderwake", 0, (3840, 2160))
        },
    )
    .await;
    aggregate_map(datastore, "f-s-cinderwake-1", "e-s-cinderwake-1").await;
    aggregate_map_as(
        datastore,
        "f-s-cinderwake-2",
        "e-s-cinderwake-1",
        "additional",
    )
    .await;

    // Emberfall Deep: one double-episode file, also the season's ordered path, so
    // the old joins fanned it out; a recycled file; an unlinked file that quality
    // still reads and size does not; a special and a TBA episode the counts skip.
    aggregate_collection(
        datastore,
        "c-s-deep",
        "s-emberfall-deep",
        "season",
        "1",
        Some("f-s-deep-double"),
    )
    .await;
    aggregate_collection(
        datastore,
        "c-s-deep-specials",
        "s-emberfall-deep",
        "specials",
        "0",
        None,
    )
    .await;
    aggregate_episode(
        datastore,
        "e-s-deep-1",
        "s-emberfall-deep",
        "c-s-deep",
        "1",
        "Ash",
    )
    .await;
    aggregate_episode(
        datastore,
        "e-s-deep-2",
        "s-emberfall-deep",
        "c-s-deep",
        "2",
        "Ember",
    )
    .await;
    aggregate_episode(
        datastore,
        "e-s-deep-3",
        "s-emberfall-deep",
        "c-s-deep",
        "3",
        "TBA",
    )
    .await;
    aggregate_episode(
        datastore,
        "e-s-deep-special",
        "s-emberfall-deep",
        "c-s-deep-specials",
        "1",
        "Special",
    )
    .await;
    aggregate_file(
        datastore,
        file("f-s-deep-double", "s-emberfall-deep", 300, (1920, 1080)),
    )
    .await;
    aggregate_map(datastore, "f-s-deep-double", "e-s-deep-1").await;
    aggregate_map(datastore, "f-s-deep-double", "e-s-deep-2").await;
    aggregate_map(datastore, "f-s-deep-double", "e-s-deep-special").await;
    aggregate_file(
        datastore,
        AggregateFile {
            path: "/media/Series/Deep/.scryer-recycle/S01E03.mkv",
            ..file("f-s-deep-recycled", "s-emberfall-deep", 1000, (3840, 2160))
        },
    )
    .await;
    aggregate_map(datastore, "f-s-deep-recycled", "e-s-deep-3").await;
    aggregate_file(
        datastore,
        file("f-s-deep-unlinked", "s-emberfall-deep", 999, (1280, 720)),
    )
    .await;

    // The movie's file is reached only through its collection's ordered path.
    aggregate_collection(
        datastore,
        "c-m-film",
        "m-cinderwake-film",
        "movie",
        "1",
        Some("f-m-film"),
    )
    .await;
    aggregate_file(
        datastore,
        file("f-m-film", "m-cinderwake-film", 500, (3840, 2160)),
    )
    .await;

    // Ashgrove Link: one file through a series-movie link, one with a negative size
    // owning the only episode; neither has anything quality can rank.
    aggregate_exec(
        datastore,
        "INSERT INTO movie_entities (id, title, created_at, updated_at)
         VALUES ('me-ashgrove', 'Ashgrove Interlude', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    aggregate_exec(
        datastore,
        "INSERT INTO series_movie_links (id, series_title_id, movie_entity_id, created_at,
                                         updated_at)
         VALUES ('l-ashgrove', 's-ashgrove-link', 'me-ashgrove', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    aggregate_file(
        datastore,
        file("f-s-link-movie", "s-ashgrove-link", 700, (0, 0)),
    )
    .await;
    aggregate_exec(
        datastore,
        "INSERT INTO file_series_movie_link_map (file_id, series_movie_link_id)
         VALUES ('f-s-link-movie', 'l-ashgrove')",
        vec![],
    )
    .await;
    aggregate_collection(
        datastore,
        "c-s-link",
        "s-ashgrove-link",
        "season",
        "1",
        None,
    )
    .await;
    aggregate_episode(
        datastore,
        "e-s-link-1",
        "s-ashgrove-link",
        "c-s-link",
        "1",
        "Grove",
    )
    .await;
    aggregate_file(
        datastore,
        file("f-s-link-negative", "s-ashgrove-link", -5, (0, 0)),
    )
    .await;
    aggregate_map(datastore, "f-s-link-negative", "e-s-link-1").await;

    // Anime Emberfall: one of two episodes owned.
    aggregate_collection(
        datastore,
        "c-a-emberfall",
        "a-emberfall",
        "season",
        "1",
        None,
    )
    .await;
    aggregate_episode(
        datastore,
        "e-a-emberfall-1",
        "a-emberfall",
        "c-a-emberfall",
        "1",
        "Kindle",
    )
    .await;
    aggregate_episode(
        datastore,
        "e-a-emberfall-2",
        "a-emberfall",
        "c-a-emberfall",
        "2",
        "Flare",
    )
    .await;
    aggregate_file(
        datastore,
        file("f-a-emberfall", "a-emberfall", 300, (1920, 1080)),
    )
    .await;
    aggregate_map(datastore, "f-a-emberfall", "e-a-emberfall-1").await;

    // Cinderwake Ember: additional files only, so no episode is owned and quality
    // takes the best additional file.
    aggregate_collection(datastore, "c-a-cinder", "a-cinderwake", "season", "1", None).await;
    aggregate_episode(
        datastore,
        "e-a-cinder-1",
        "a-cinderwake",
        "c-a-cinder",
        "1",
        "Smoke",
    )
    .await;
    for (file_id, size, dims) in [
        ("f-a-cinder-uhd", 50, (3840, 2160)),
        ("f-a-cinder-hd", 20, (1280, 720)),
    ] {
        aggregate_file(
            datastore,
            AggregateFile {
                role: "additional",
                ..file(file_id, "a-cinderwake", size, dims)
            },
        )
        .await;
        aggregate_map_as(datastore, file_id, "e-a-cinder-1", "additional").await;
    }

    // Ratings: a tie across libraries, a NULL rating row, and a movie entity's
    // summary that has no title.
    for (title_id, rating) in [
        ("s-emberfall", Some(7.5)),
        ("a-emberfall", Some(7.5)),
        ("m-cinderwake-film", Some(9.0)),
        ("s-cinderwake", None),
    ] {
        aggregate_exec(
            datastore,
            "INSERT INTO title_metadata_rating_summaries (title_id, rating, created_at, updated_at)
             VALUES ({}, {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            vec![SqlArg::Text(title_id.into()), SqlArg::OptF64(rating)],
        )
        .await;
    }
    aggregate_exec(
        datastore,
        "INSERT INTO title_metadata_rating_summaries (movie_entity_id, rating, created_at,
                                                     updated_at)
         VALUES ('me-ashgrove', 1.0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
}

/// The size join as it was before it read only the page's scope: every file in
/// the database, deduplicated after the link joins fanned it out.
fn previous_media_size_subquery(dialect: TitleCatalogSqlDialect, scope_join: &str) -> String {
    format!(
        "SELECT matched.title_id, {} AS total_size_bytes
           FROM (
                SELECT DISTINCT mf.id, mf.title_id,
                       CASE WHEN mf.size_bytes > 0 THEN mf.size_bytes ELSE 0 END AS size_bytes
                  FROM media_files mf
                  {scope_join}
             LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
             LEFT JOIN collections c ON c.title_id = mf.title_id AND c.ordered_path = mf.file_path
             LEFT JOIN file_series_movie_link_map fsmlm ON fsmlm.file_id = mf.id
                 WHERE {}
                   AND (fem.file_id IS NOT NULL OR c.id IS NOT NULL OR fsmlm.file_id IS NOT NULL)
           ) matched
          GROUP BY matched.title_id",
        title_catalog_total_size_sum_expression(dialect, "matched.size_bytes"),
        title_catalog_live_media_file_predicate(dialect, "mf")
    )
}

/// The page query as it was before: the sort's join aggregated every title.
fn previous_page_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    filter: &TitleCatalogFilter,
    sort: &TitleCatalogSort,
    dialect: TitleCatalogSqlDialect,
) -> (String, Vec<SqlArg>) {
    let join = match sort.key {
        TitleCatalogSortKey::Size => format!(
            " LEFT JOIN ({}) catalog_media_size ON catalog_media_size.title_id = titles.id",
            previous_media_size_subquery(dialect, "")
        ),
        TitleCatalogSortKey::Episodes => format!(
            " LEFT JOIN ({}) catalog_episode_progress ON catalog_episode_progress.title_id = titles.id",
            title_catalog_episode_progress_subquery_scoped(dialect, "")
        ),
        TitleCatalogSortKey::Quality => format!(
            " LEFT JOIN ({}) catalog_media_quality ON catalog_media_quality.title_id = titles.id",
            title_catalog_media_quality_subquery_scoped(dialect, "")
        ),
        TitleCatalogSortKey::RatingScryer => " LEFT JOIN (
                SELECT rating.title_id, MAX(rating.rating) AS rating
                  FROM title_metadata_rating_summaries rating
                 GROUP BY rating.title_id
            ) catalog_rating_scryer ON catalog_rating_scryer.title_id = titles.id"
            .to_string(),
        other => panic!("no previous form kept for {other:?}"),
    };
    let (where_sql, mut args) =
        build_title_catalog_where_sql(facet, library_ids, None, filter, dialect);
    let order = build_title_catalog_order_sql(sort, dialect, &mut args);
    (
        format!("SELECT titles.id FROM titles{join} WHERE {where_sql} {order}"),
        args,
    )
}

/// Managed bytes as they were computed before.
fn previous_managed_bytes_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    filter: &TitleCatalogFilter,
    dialect: TitleCatalogSqlDialect,
) -> (String, Vec<SqlArg>) {
    let (scope_sql, args) =
        build_title_catalog_where_sql(facet, library_ids, None, filter, dialect);
    (
        format!(
            "WITH scoped_titles AS (SELECT id FROM titles WHERE {scope_sql}) \
             SELECT CAST(COALESCE(SUM(catalog_media_size.total_size_bytes), 0) AS BIGINT) \
                    AS managed_bytes \
               FROM ({}) catalog_media_size",
            previous_media_size_subquery(
                dialect,
                "JOIN scoped_titles ON scoped_titles.id = mf.title_id"
            )
        ),
        args,
    )
}

/// The order a sort promises, computed from the fixture table: the sort value
/// (missing values where each sort puts them), then the ascending title order.
fn expected_aggregate_order(
    members: &[&AggregateFixture],
    key: TitleCatalogSortKey,
    direction: SortDirection,
) -> Vec<String> {
    use std::cmp::Ordering;
    let directed = |order: Ordering| match direction {
        SortDirection::Asc => order,
        SortDirection::Desc => order.reverse(),
    };
    // Missing numeric values go last whatever the direction.
    let nullable = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => directed(a.partial_cmp(&b).unwrap()),
        (a, b) => a.is_none().cmp(&b.is_none()),
    };
    let mut sorted = members.to_vec();
    sorted.sort_by(|a, b| {
        let value = match key {
            // A title with no counted file sorts as -1, below an empty file's 0.
            TitleCatalogSortKey::Size => directed(a.size.unwrap_or(-1).cmp(&b.size.unwrap_or(-1))),
            TitleCatalogSortKey::Episodes => {
                let ratio = |f: &AggregateFixture| match f.episodes {
                    Some((owned, total)) if total > 0 => owned as f64 / total as f64,
                    _ => -1.0,
                };
                let owned = |f: &AggregateFixture| f.episodes.map_or(0, |(owned, _)| owned);
                let total = |f: &AggregateFixture| f.episodes.map_or(0, |(_, total)| total);
                directed(ratio(a).partial_cmp(&ratio(b)).unwrap())
                    .then_with(|| directed(owned(a).cmp(&owned(b))))
                    .then_with(|| directed(total(a).cmp(&total(b))))
            }
            TitleCatalogSortKey::Quality => {
                nullable(a.quality.map(|q| q as f64), b.quality.map(|q| q as f64))
            }
            TitleCatalogSortKey::RatingScryer => nullable(a.rating, b.rating),
            other => panic!("no expected order for {other:?}"),
        };
        value
            .then_with(|| {
                title_catalog_sort_key(a.name, Some("eng"))
                    .cmp(&title_catalog_sort_key(b.name, Some("eng")))
            })
            .then_with(|| {
                title_catalog_name_tie_key(a.name).cmp(&title_catalog_name_tie_key(b.name))
            })
            .then_with(|| a.year.is_none().cmp(&b.year.is_none()))
            .then_with(|| a.year.cmp(&b.year))
            .then_with(|| a.id.cmp(b.id))
    });
    sorted.iter().map(|f| f.id.to_string()).collect()
}

async fn assert_catalog_aggregate_sorts(datastore: StoreDatastore) {
    let dialect = title_catalog_dialect_for_datastore(&datastore);
    seed_aggregate_fixtures(&datastore).await;
    let series = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let movie = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let store = TitleStore::new(datastore.clone());

    type Scope<'a> = (
        &'a str,
        Option<MediaFacet>,
        Vec<String>,
        TitleCatalogFilter,
        Vec<&'a str>,
    );
    let scopes: Vec<Scope<'_>> = vec![
        (
            "one library, one facet",
            Some(MediaFacet::Series),
            vec![series.clone()],
            TitleCatalogFilter::default(),
            vec![
                "s-emberfall",
                "s-cinderwake",
                "s-ashgrove",
                "s-ashgrove-two",
                "s-emberfall-deep",
                "s-ashgrove-link",
            ],
        ),
        (
            "two libraries, one facet",
            Some(MediaFacet::Series),
            vec![series.clone(), movie.clone()],
            TitleCatalogFilter::default(),
            vec![
                "s-emberfall",
                "s-cinderwake",
                "s-ashgrove",
                "s-ashgrove-two",
                "s-emberfall-deep",
                "s-ashgrove-link",
            ],
        ),
        (
            "two libraries, every facet",
            None,
            vec![series.clone(), anime.clone()],
            TitleCatalogFilter::default(),
            AGGREGATE_FIXTURES
                .iter()
                .filter(|f| f.facet != MediaFacet::Movie)
                .map(|f| f.id)
                .collect(),
        ),
        (
            "three libraries",
            None,
            vec![series.clone(), anime.clone(), movie.clone()],
            TitleCatalogFilter::default(),
            AGGREGATE_FIXTURES.iter().map(|f| f.id).collect(),
        ),
        (
            "three libraries, monitored",
            None,
            vec![series.clone(), anime.clone(), movie.clone()],
            TitleCatalogFilter {
                monitored: Some(true),
                ..Default::default()
            },
            AGGREGATE_FIXTURES
                .iter()
                .filter(|f| f.monitored)
                .map(|f| f.id)
                .collect(),
        ),
        (
            "three libraries, missing files",
            None,
            vec![series.clone(), anime.clone(), movie.clone()],
            TitleCatalogFilter {
                presences: vec![TitleCatalogPresence::Missing],
                ..Default::default()
            },
            vec!["s-ashgrove", "s-ashgrove-two", "a-cinderwake", "a-ashgrove"],
        ),
        (
            "anime library, picked titles",
            Some(MediaFacet::Anime),
            vec![anime.clone()],
            TitleCatalogFilter {
                title_ids: vec![
                    "a-emberfall".into(),
                    "a-ashgrove".into(),
                    "s-emberfall".into(),
                ],
                ..Default::default()
            },
            vec!["a-emberfall", "a-ashgrove"],
        ),
    ];

    for (label, facet, libraries, filter, member_ids) in &scopes {
        let members = AGGREGATE_FIXTURES
            .iter()
            .filter(|f| member_ids.contains(&f.id))
            .collect::<Vec<_>>();
        assert_eq!(members.len(), member_ids.len(), "{label}");

        for key in [
            TitleCatalogSortKey::Size,
            TitleCatalogSortKey::Episodes,
            TitleCatalogSortKey::Quality,
            TitleCatalogSortKey::RatingScryer,
        ] {
            for direction in [SortDirection::Asc, SortDirection::Desc] {
                let sort = TitleCatalogSort {
                    key,
                    direction,
                    ..Default::default()
                };
                let mut paged = Vec::new();
                for offset in (0..members.len() + 3).step_by(3) {
                    let (sql, args) = build_title_catalog_page_sql(
                        facet.clone(),
                        libraries,
                        None,
                        filter,
                        sort.clone(),
                        3,
                        offset,
                        dialect,
                        TitleListProjection::default(),
                    );
                    paged.extend(
                        SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
                            .await
                            .unwrap_or_else(|error| panic!("{label} {key:?}: {error}\n{sql}"))
                            .into_iter()
                            .map(|row| row.text("id").unwrap()),
                    );
                }
                assert_eq!(
                    paged,
                    expected_aggregate_order(&members, key, direction),
                    "{label}, {key:?} {direction:?}"
                );

                let (sql, args) =
                    previous_page_sql(facet.clone(), libraries, filter, &sort, dialect);
                let previous = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| row.text("id").unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    paged, previous,
                    "{label}, {key:?} {direction:?} vs previous SQL"
                );
            }
        }

        let expected_bytes = members.iter().map(|f| f.size.unwrap_or(0)).sum::<i64>();
        let (sql, args) = previous_managed_bytes_sql(facet.clone(), libraries, filter, dialect);
        let previous_bytes = SqlRuntime::fetch_optional(datastore.read_exec(), &sql, &args)
            .await
            .unwrap()
            .unwrap()
            .i64("managed_bytes")
            .unwrap();
        let result = TitleRepository::list_for_libraries_catalog(
            &store,
            facet.clone(),
            libraries,
            None,
            filter.clone(),
            TitleCatalogSort::default(),
            0,
            0,
            TitleListProjection::default(),
            scryer_application::TitleCatalogAggregates {
                total_count: true,
                filter_counts: false,
                file_state_counts: false,
                managed_bytes: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result.managed_bytes, expected_bytes,
            "{label} managed bytes"
        );
        assert_eq!(
            previous_bytes, expected_bytes,
            "{label} previous managed bytes"
        );
        assert_eq!(result.total_count, members.len(), "{label} total");
    }
}

/// The Episodes sort counts an episode as owned under exactly the file
/// conditions the card's displayed counts use: a primary map row to a live file
/// that is not waiting for review, whatever the file's own role.
async fn assert_episode_sort_counts_match_displayed_counts(datastore: StoreDatastore) {
    use scryer_application::MediaFileRepository;

    let dialect = title_catalog_dialect_for_datastore(&datastore);
    let library = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let root = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT id FROM library_roots WHERE library_id = {} LIMIT 1",
        &[SqlArg::Text(library.clone())],
    )
    .await
    .unwrap()
    .unwrap()
    .text("id")
    .unwrap();
    let mut title = fixture_title("s-emberfall-review", "Emberfall Review", Some(2001));
    title.library_id = library;
    title.facet = MediaFacet::Series;
    title.root_folder_id = root;
    TitleRepository::create(&TitleStore::new(datastore.clone()), title)
        .await
        .unwrap();
    aggregate_collection(
        &datastore,
        "c-review",
        "s-emberfall-review",
        "season",
        "1",
        None,
    )
    .await;
    // Owned: a complete primary file mapped as primary.
    // Not owned: a primary map to a file waiting for review.
    // Not owned: an additional map to a file whose own role is primary.
    // Owned: a primary map to a file whose own role is additional.
    for (number, file_id, file_role, scan_status, map_role) in [
        ("1", "f-review-1", "primary", "complete", "primary"),
        ("2", "f-review-2", "primary", "review_required", "primary"),
        ("3", "f-review-3", "primary", "complete", "additional"),
        ("4", "f-review-4", "additional", "complete", "primary"),
    ] {
        let episode = format!("e-review-{number}");
        aggregate_episode(
            &datastore,
            &episode,
            "s-emberfall-review",
            "c-review",
            number,
            "Embers",
        )
        .await;
        aggregate_file(
            &datastore,
            AggregateFile {
                role: file_role,
                scan_status,
                ..file(file_id, "s-emberfall-review", 100, (1920, 1080))
            },
        )
        .await;
        aggregate_map_as(&datastore, file_id, &episode, map_role).await;
    }

    let sql = format!(
        "SELECT owned_episodes, total_episodes FROM ({}) progress WHERE progress.title_id = {{}}",
        title_catalog_episode_progress_subquery_scoped(dialect, "")
    );
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &sql,
        &[SqlArg::Text("s-emberfall-review".into())],
    )
    .await
    .unwrap()
    .unwrap();
    let sorted_by = (
        row.i64("owned_episodes").unwrap(),
        row.i64("total_episodes").unwrap(),
    );
    let displayed = crate::media::search::media_file_store::MediaFileStore::new(datastore.clone())
        .list_title_episode_progress_summaries(&["s-emberfall-review".to_string()])
        .await
        .unwrap();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        sorted_by,
        (displayed[0].owned_episodes, displayed[0].total_episodes)
    );
    assert_eq!(sorted_by, (2, 4));
}

#[tokio::test]
async fn sqlite_episode_sort_counts_match_displayed_counts() {
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
    assert_episode_sort_counts_match_displayed_counts(StoreDatastore::sqlite(
        pool,
        Arc::new(tokio::sync::Mutex::new(())),
    ))
    .await;
}

#[tokio::test]
async fn postgres_episode_sort_counts_match_displayed_counts() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("episode_sort_counts_{}", uuid::Uuid::new_v4().simple());
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
    assert_episode_sort_counts_match_displayed_counts(StoreDatastore::Postgres {
        pool: pool.clone(),
    })
    .await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_catalog_aggregate_sorts_read_only_the_page_scope() {
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
    assert_catalog_aggregate_sorts(StoreDatastore::sqlite(
        pool,
        Arc::new(tokio::sync::Mutex::new(())),
    ))
    .await;
}

#[tokio::test]
async fn postgres_catalog_aggregate_sorts_read_only_the_page_scope() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("catalog_aggregate_{}", uuid::Uuid::new_v4().simple());
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
    assert_catalog_aggregate_sorts(StoreDatastore::Postgres { pool: pool.clone() }).await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
