//! The batched library-presence read against the single-library lookup it
//! stands in for, over a real sqlite database with the full migration set.

use std::collections::HashSet;

use scryer_application::{TitleExternalIdPresence, TitleExternalIdPresenceLookup, TitleRepository};
use scryer_domain::MediaFacet;
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::{TitleStore, external_id_presence_sql};

const MOVIES_A: &str = "fixture-movies-a";
const MOVIES_B: &str = "fixture-movies-b";
const SERIES_A: &str = "fixture-series-a";

async fn run(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "presence_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn insert_library(datastore: &StoreDatastore, id: &str, facet: &str) {
    run(
        datastore,
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await;
}

/// A title and its external-id projection rows, written lowercase and trimmed
/// the way the title writers store them.
async fn insert_title(
    datastore: &StoreDatastore,
    id: &str,
    facet: &str,
    library_id: &str,
    external_ids: &[(&str, &str)],
) {
    run(
        datastore,
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
    for (source, value) in external_ids {
        run(
            datastore,
            "INSERT INTO title_external_ids (id, title_id, source, external_id, created_at,
                                             facet, library_id)
             VALUES ({}, {}, {}, {}, '2026-01-01T00:00:00Z', {}, {})",
            vec![
                SqlArg::Text(format!("{id}-{source}-{value}")),
                SqlArg::Text(id.to_string()),
                SqlArg::Text((*source).to_string()),
                SqlArg::Text((*value).to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::Text(library_id.to_string()),
            ],
        )
        .await;
    }
}

async fn seeded_store() -> (TitleStore, sqlx::SqlitePool) {
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

    insert_library(&datastore, MOVIES_A, "movie").await;
    insert_library(&datastore, MOVIES_B, "movie").await;
    insert_library(&datastore, SERIES_A, "series").await;

    insert_title(
        &datastore,
        "fixture-movie-alpha",
        "movie",
        MOVIES_A,
        &[("tvdb", "1001"), ("imdb", "tt0001001")],
    )
    .await;
    insert_title(
        &datastore,
        "fixture-movie-beta",
        "movie",
        MOVIES_B,
        &[("tvdb", "1002")],
    )
    .await;
    // The same TVDB id in a series library: a movie lookup must not see it.
    insert_title(
        &datastore,
        "fixture-series-alpha",
        "series",
        SERIES_A,
        &[("tvdb", "1001")],
    )
    .await;
    // Far down the identity list, so it lands in a later chunk.
    insert_title(
        &datastore,
        "fixture-movie-gamma",
        "movie",
        MOVIES_A,
        &[("tvdb", "1650")],
    )
    .await;

    (TitleStore::new(datastore), pool)
}

fn lookup(facet: MediaFacet, source: &str, value: &str) -> TitleExternalIdPresenceLookup {
    TitleExternalIdPresenceLookup {
        facet,
        source: source.to_string(),
        value: value.to_string(),
    }
}

fn presence(library_id: &str, lookup: TitleExternalIdPresenceLookup) -> TitleExternalIdPresence {
    TitleExternalIdPresence {
        library_id: library_id.to_string(),
        lookup,
    }
}

fn fixture_lookups() -> Vec<TitleExternalIdPresenceLookup> {
    let mut lookups = (1000..=1700)
        .map(|value| lookup(MediaFacet::Movie, "tvdb", &value.to_string()))
        .collect::<Vec<_>>();
    lookups.push(lookup(MediaFacet::Movie, "IMDB", " tt0001001 "));
    lookups.push(lookup(MediaFacet::Series, "tvdb", "1001"));
    lookups.push(lookup(MediaFacet::Series, "tvdb", "1002"));
    lookups.push(lookup(MediaFacet::Movie, "tmdb", "1001"));
    lookups
}

/// The answer the single-library method gives, asked once per library and
/// `(facet, source)`.
async fn per_library_answer(
    store: &TitleStore,
    library_ids: &[String],
    lookups: &[TitleExternalIdPresenceLookup],
) -> HashSet<TitleExternalIdPresence> {
    let mut grouped = std::collections::BTreeMap::<(String, String), Vec<String>>::new();
    for lookup in lookups {
        grouped
            .entry((
                lookup.facet.as_str().to_string(),
                lookup.source.trim().to_ascii_lowercase(),
            ))
            .or_default()
            .push(lookup.value.clone());
    }
    let mut answer = HashSet::new();
    for library_id in library_ids {
        for ((facet, source), values) in &grouped {
            let facet = lookups
                .iter()
                .find(|lookup| lookup.facet.as_str() == facet)
                .map(|lookup| lookup.facet.clone())
                .expect("facet came from the lookups");
            for value in store
                .list_existing_external_ids_in_library_and_facet(
                    library_id,
                    facet.clone(),
                    source,
                    values,
                )
                .await
                .expect("single-library lookup should succeed")
            {
                answer.insert(presence(library_id, lookup(facet.clone(), source, &value)));
            }
        }
    }
    answer
}

#[tokio::test]
async fn batched_presence_matches_the_single_library_lookup_across_chunks() {
    let (store, _pool) = seeded_store().await;
    let library_ids = vec![
        MOVIES_A.to_string(),
        MOVIES_B.to_string(),
        SERIES_A.to_string(),
    ];
    let lookups = fixture_lookups();

    let expected = HashSet::from([
        presence(MOVIES_A, lookup(MediaFacet::Movie, "tvdb", "1001")),
        presence(MOVIES_A, lookup(MediaFacet::Movie, "imdb", "tt0001001")),
        presence(MOVIES_A, lookup(MediaFacet::Movie, "tvdb", "1650")),
        presence(MOVIES_B, lookup(MediaFacet::Movie, "tvdb", "1002")),
        presence(SERIES_A, lookup(MediaFacet::Series, "tvdb", "1001")),
    ]);
    assert_eq!(
        per_library_answer(&store, &library_ids, &lookups).await,
        expected
    );

    let one_statement = store
        .list_existing_external_ids_in_libraries(&library_ids, &lookups)
        .await
        .expect("batched lookup should succeed");
    assert_eq!(one_statement, expected);

    // Eight binds per statement: libraries split two and one, and two
    // identities per statement, so the 1650 hit is many chunks in.
    let chunked = {
        let _lowered = crate::media::id_list_statements::lower_max_binds_for_tests(8);
        store
            .list_existing_external_ids_in_libraries(&library_ids, &lookups)
            .await
            .expect("chunked batched lookup should succeed")
    };
    assert_eq!(chunked, expected);
}

#[tokio::test]
async fn batched_presence_answers_only_for_the_libraries_asked() {
    let (store, _pool) = seeded_store().await;
    let lookups = fixture_lookups();

    let only_b = store
        .list_existing_external_ids_in_libraries(&[MOVIES_B.to_string()], &lookups)
        .await
        .expect("batched lookup should succeed");
    assert_eq!(
        only_b,
        HashSet::from([presence(
            MOVIES_B,
            lookup(MediaFacet::Movie, "tvdb", "1002")
        )])
    );

    assert!(
        store
            .list_existing_external_ids_in_libraries(&[], &lookups)
            .await
            .expect("no libraries reads nothing")
            .is_empty()
    );
    assert!(
        store
            .list_existing_external_ids_in_libraries(&[MOVIES_A.to_string()], &[])
            .await
            .expect("no identities reads nothing")
            .is_empty()
    );
}

/// The read probes the external-id projection by library, source and value
/// through an index rather than scanning it.
#[tokio::test]
async fn batched_presence_is_served_by_the_library_source_value_index() {
    let (_store, pool) = seeded_store().await;
    let sql = external_id_presence_sql(2, 2).replace("{}", "?");
    // The statement is the store's own fixed SQL; only binds vary.
    let mut query = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")));
    for value in [
        "movie", "tvdb", "1001", "movie", "tvdb", "1002", MOVIES_A, MOVIES_B,
    ] {
        query = query.bind(value);
    }
    let details = query
        .fetch_all(&pool)
        .await
        .expect("query plan should read")
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>();
    assert!(
        details.iter().any(|detail| detail.contains(
            "title_external_ids USING INDEX idx_title_external_ids_library_source_value"
        ) || detail.contains(
            "title_external_ids USING COVERING INDEX idx_title_external_ids_library_source_value"
        )),
        "expected an index probe on title_external_ids, got {details:?}"
    );
    assert!(
        !details
            .iter()
            .any(|detail| detail.starts_with("SCAN title_external_ids")),
        "title_external_ids must not be scanned: {details:?}"
    );
}
