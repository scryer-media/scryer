use super::*;

fn fixture_title(id: &str, name: &str, year: Option<i32>) -> Title {
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
