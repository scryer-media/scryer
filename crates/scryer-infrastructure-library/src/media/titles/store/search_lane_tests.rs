//! The direct lanes of the library search read the fuzzy index for the rows
//! that can match and the projection for the rows that do. These tests hold
//! that to the query it replaced: a pattern test of every projected name.

use std::collections::BTreeMap;
use std::sync::Arc;

use scryer_infrastructure_library_search::{
    TitleFuzzyIndex, build_title_search_plan, fuzzy_typo_hits, resolve_title_match_ranks,
    typo_title_ranks,
};

use super::catalog_order_tests::fixture_title;
use super::*;
use crate::media::titles::fuzzy_source::DatastoreTitleTermSource;
use crate::queries::title_search::ResolvedTitleSearch;

/// (id, name, facet, aliases)
const FIXTURES: &[(&str, &str, MediaFacet, &[&str])] = &[
    ("lane-exact", "Amber Road", MediaFacet::Movie, &[]),
    ("lane-prefix", "Amber Road North", MediaFacet::Movie, &[]),
    (
        "lane-inside",
        "The Amber Road North",
        MediaFacet::Movie,
        &[],
    ),
    (
        "lane-shuffled",
        "Road Amber Roa Mber R",
        MediaFacet::Movie,
        &[],
    ),
    (
        "lane-alias",
        "Copper Lane",
        MediaFacet::Movie,
        &["Amber Road"],
    ),
    ("lane-series", "Amber Road", MediaFacet::Series, &[]),
    ("lane-anime", "Quiet Amber Harbor", MediaFacet::Anime, &[]),
    (
        "lane-accent",
        "Café Émber 100% Under_Score",
        MediaFacet::Movie,
        &[],
    ),
    ("lane-short", "Up", MediaFacet::Movie, &[]),
    ("lane-article", "The Long Thaw", MediaFacet::Series, &[]),
    ("lane-other", "Granite Hollow", MediaFacet::Series, &[]),
];

const QUERIES: &[&str] = &[
    "amber road",
    "Amber",
    "amber road north",
    "a",
    "ro",
    "up",
    "the",
    "the ",
    "é",
    "cafe ember",
    "100%",
    "under_score",
    "r r",
    "ambr road",
    "granit hollow",
    "zzzz",
    "%",
];

/// The direct lanes as they were read before: every full name pattern-tested.
async fn scanned_direct_ranks(
    datastore: &StoreDatastore,
    facet: Option<&MediaFacet>,
    normalized: &str,
) -> BTreeMap<String, i64> {
    let mut sql = String::from(
        "SELECT title_id,
                MIN(CASE WHEN normalized_term = {} THEN 0
                         WHEN normalized_term LIKE {} THEN 1000
                         ELSE 2000 END + weight) AS match_rank
           FROM title_search_terms
          WHERE term_kind NOT LIKE '%_token' AND normalized_term LIKE {}",
    );
    let mut args = vec![
        SqlArg::Text(normalized.to_string()),
        SqlArg::Text(format!("{normalized}%")),
        SqlArg::Text(format!("%{normalized}%")),
    ];
    if let Some(facet) = facet {
        sql.push_str(" AND facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    sql.push_str(" GROUP BY title_id");
    SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.text("title_id").unwrap(),
                row.i64("match_rank").unwrap(),
            )
        })
        .collect()
}

async fn create(
    store: &TitleStore,
    datastore: &StoreDatastore,
    fixture: &(&str, &str, MediaFacet, &[&str]),
) {
    let (id, name, facet, aliases) = fixture;
    let mut title = fixture_title(id, name, None);
    title.facet = facet.clone();
    title.library_id = scryer_domain::default_library_id_for_facet(facet);
    title.root_folder_id = super::release_day_tests::root_for(datastore, &title.library_id).await;
    title.aliases = aliases.iter().map(|alias| alias.to_string()).collect();
    TitleRepository::create(store, title).await.unwrap();
}

async fn assert_search_lanes(datastore: StoreDatastore) {
    let directory = tempfile::tempdir().unwrap();
    let index = TitleFuzzyIndex::open(
        directory.path(),
        Arc::new(DatastoreTitleTermSource::new(datastore.clone())),
    )
    .await
    .unwrap();
    let store = TitleStore::new(datastore.clone()).with_fuzzy_index(index.clone());
    // Written after the index opened and never synced by hand: a search has to
    // see them the way the pattern test of the projection did.
    for fixture in FIXTURES {
        create(&store, &datastore, fixture).await;
    }

    let mut matched_something = 0;
    for query in QUERIES {
        for facet in [
            None,
            Some(MediaFacet::Movie),
            Some(MediaFacet::Series),
            Some(MediaFacet::Anime),
        ] {
            let context = format!("query {query:?}, facet {facet:?}");
            let Some(plan) = build_title_search_plan(facet.clone(), query) else {
                let resolved =
                    ResolvedTitleSearch::resolve(Some(&index), facet.clone(), Some(query))
                        .await
                        .unwrap()
                        .expect("a query that is not blank resolves");
                assert_eq!(resolved.predicate("titles.id").0, "1 = 0", "{context}");
                continue;
            };
            // Resolved first: it drains the index queue, and the typo lane read
            // below for the expectation then sees the same index.
            let ranks = resolve_title_match_ranks(Some(&index), &plan)
                .await
                .unwrap()
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let normalized = crate::queries::title_search::normalize_title_search_text(query);
            let mut expected = scanned_direct_ranks(&datastore, facet.as_ref(), &normalized).await;
            for (title_id, rank) in
                typo_title_ranks(&plan, &fuzzy_typo_hits(&index, &plan).await.unwrap())
            {
                expected
                    .entry(title_id)
                    .and_modify(|current| *current = (*current).min(rank))
                    .or_insert(rank);
            }
            matched_something += usize::from(!expected.is_empty());

            assert_eq!(ranks, expected, "{context}");

            // The ranked list: the same titles, best rank first.
            let listed = TitleRepository::list(&store, facet.clone(), Some(query.to_string()))
                .await
                .unwrap();
            let listed_ranks = listed
                .iter()
                .map(|title| expected[&title.id])
                .collect::<Vec<_>>();
            assert!(listed_ranks.is_sorted(), "{context}: {listed_ranks:?}");
            assert_eq!(listed.len(), expected.len(), "{context}");

            let library_ids = FIXTURES
                .iter()
                .map(|(_, _, facet, _)| scryer_domain::default_library_id_for_facet(facet))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let scoped = TitleRepository::list_for_libraries(
                &store,
                facet.clone(),
                &library_ids,
                Some(query.to_string()),
            )
            .await
            .unwrap();
            assert_eq!(
                scoped.iter().map(|title| &title.id).collect::<Vec<_>>(),
                listed.iter().map(|title| &title.id).collect::<Vec<_>>(),
                "{context}"
            );

            // The filter the catalog and wanted pages splice in.
            let resolved = ResolvedTitleSearch::resolve(Some(&index), facet.clone(), Some(query))
                .await
                .unwrap()
                .unwrap();
            let (predicate, predicate_args) = resolved.predicate("titles.id");
            let filtered = SqlRuntime::fetch_all(
                datastore.read_exec(),
                &format!("SELECT id FROM titles WHERE {predicate} ORDER BY id"),
                &predicate_args
                    .into_iter()
                    .map(SqlArg::Text)
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.text("id").unwrap())
            .collect::<Vec<_>>();
            assert_eq!(
                filtered,
                expected.keys().cloned().collect::<Vec<_>>(),
                "{context}"
            );
        }
    }
    assert!(
        matched_something > 20,
        "the fixtures must exercise the lanes"
    );

    // A title that leaves stops matching without the index being synced by hand.
    TitleRepository::delete(&store, "lane-short").await.unwrap();
    let plan = build_title_search_plan(None, "up").unwrap();
    assert!(
        resolve_title_match_ranks(Some(&index), &plan)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_search_lanes_match_the_pattern_test_they_replaced() {
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
    assert_search_lanes(StoreDatastore::sqlite(
        pool,
        std::sync::Arc::new(tokio::sync::Mutex::new(())),
    ))
    .await;
}

#[tokio::test]
async fn postgres_search_lanes_match_the_pattern_test_they_replaced() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("search_lanes_{}", uuid::Uuid::new_v4().simple());
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
    assert_search_lanes(StoreDatastore::Postgres { pool: pool.clone() }).await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
