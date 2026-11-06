//! The probe-scoped catalog ownership read against a real sqlite database.
//!
//! Discovery used to decide ownership from every title in the readable
//! libraries. The probe-scoped read must decide exactly the same for the
//! candidates it was built from, across every id spelling the ownership keys
//! accept: case, a facet or source prefix in the stored id, leading zeros, a
//! sign, and the movies linked to an anime series.

use scryer_application::{
    CatalogOwnedSeriesMovieRecord, DiscoveryItemTargetRecord, ShowRepository, TitleRepository,
    catalog_owned_probe_for_targets, catalog_ownership_decisions,
};
use sqlx::sqlite::SqlitePoolOptions;

use crate::media::shows::store::ShowStore;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::{TitleStore, catalog_owned_title_records_matching_sql};

const READABLE_MOVIES: &str = "library-probe-movies";
const READABLE_ANIME: &str = "library-probe-anime";
const UNREADABLE: &str = "library-probe-hidden";

pub(super) async fn datastore() -> StoreDatastore {
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
        writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    }
}

pub(super) async fn run(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "owned_probe_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

pub(super) async fn insert_library(datastore: &StoreDatastore, id: &str, facet: &str) {
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

pub(super) async fn insert_title(
    datastore: &StoreDatastore,
    id: &str,
    library_id: &str,
    facet: &str,
    imdb_id: Option<&str>,
    external_ids: &[(&str, &str)],
) {
    run(
        datastore,
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, imdb_id, created_at, library_id, root_folder_id)
         VALUES ({}, {}, {}, {}, 1, 'active', '[]', '[]', {}, '2026-01-01T00:00:00Z', {},
                 'root-probe')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::OptText(imdb_id.map(str::to_string)),
            SqlArg::Text(library_id.to_string()),
        ],
    )
    .await;
    for (index, (source, value)) in external_ids.iter().enumerate() {
        run(
            datastore,
            "INSERT INTO title_external_ids (id, title_id, library_id, facet, source,
                                             external_kind, external_id, external_key,
                                             created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, '', {}, {}, '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z')",
            vec![
                SqlArg::Text(format!("{id}-ext-{index}")),
                SqlArg::Text(id.to_string()),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::Text(source.to_string()),
                SqlArg::Text(value.to_string()),
                SqlArg::Text(format!("{source}:{value}")),
            ],
        )
        .await;
    }
}

async fn link_movie(
    datastore: &StoreDatastore,
    series_title_id: &str,
    movie_id: &str,
    ids: [Option<&str>; 5],
) {
    let [imdb, tvdb, tmdb, mal, anidb] = ids.map(|id| SqlArg::OptText(id.map(str::to_string)));
    run(
        datastore,
        "INSERT INTO movie_entities (id, title, imdb_id, tvdb_id, tmdb_id, mal_id, anidb_id,
                                     created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(movie_id.to_string()),
            SqlArg::Text(movie_id.to_string()),
            imdb,
            tvdb,
            tmdb,
            mal,
            anidb,
        ],
    )
    .await;
    run(
        datastore,
        "INSERT INTO series_movie_links (id, series_title_id, movie_entity_id, created_at,
                                         updated_at)
         VALUES ({}, {}, {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(format!("link-{movie_id}")),
            SqlArg::Text(series_title_id.to_string()),
            SqlArg::Text(movie_id.to_string()),
        ],
    )
    .await;
}

/// Synthetic titles covering each id spelling the ownership keys accept.
async fn seeded() -> StoreDatastore {
    let datastore = datastore().await;
    insert_library(&datastore, READABLE_MOVIES, "movie").await;
    insert_library(&datastore, READABLE_ANIME, "anime").await;
    insert_library(&datastore, UNREADABLE, "movie").await;
    for (id, library, facet, imdb, external_ids) in [
        (
            "probe-plain",
            READABLE_MOVIES,
            "movie",
            Some("tt0000603"),
            &[("tmdb", "603")][..],
        ),
        (
            "probe-zero",
            READABLE_MOVIES,
            "movie",
            None,
            &[("tmdb", "0604")][..],
        ),
        (
            "probe-prefixed",
            READABLE_MOVIES,
            "movie",
            None,
            &[("tvdb", "movie:606")][..],
        ),
        (
            "probe-upper",
            READABLE_MOVIES,
            "movie",
            None,
            &[("imdb", "TT0000607")][..],
        ),
        (
            "probe-signed",
            READABLE_MOVIES,
            "movie",
            None,
            &[("tmdb", "+615")][..],
        ),
        (
            "probe-alias-source",
            READABLE_MOVIES,
            "movie",
            None,
            &[("themoviedb", "616")][..],
        ),
        (
            "probe-imdb-column",
            READABLE_MOVIES,
            "movie",
            Some("TT0000614"),
            &[][..],
        ),
        (
            "probe-unrelated",
            READABLE_MOVIES,
            "movie",
            None,
            &[("tmdb", "777")][..],
        ),
        (
            "probe-anime",
            READABLE_ANIME,
            "anime",
            None,
            &[("anilist", "608"), ("mal", "609")][..],
        ),
        (
            "probe-anime-quiet",
            READABLE_ANIME,
            "anime",
            None,
            &[("anidb", "778")][..],
        ),
        (
            "probe-hidden",
            UNREADABLE,
            "movie",
            None,
            &[("tmdb", "617")][..],
        ),
    ] {
        insert_title(&datastore, id, library, facet, imdb, external_ids).await;
    }
    link_movie(
        &datastore,
        "probe-anime",
        "probe-linked-movie",
        [Some("tt0000611"), None, Some("610"), None, Some("612")],
    )
    .await;
    link_movie(
        &datastore,
        "probe-anime-quiet",
        "probe-linked-quiet",
        [None, None, Some("779"), None, None],
    )
    .await;
    link_movie(
        &datastore,
        "probe-hidden",
        "probe-linked-hidden",
        [None, None, Some("618"), None, None],
    )
    .await;
    datastore
}

fn targets() -> Vec<DiscoveryItemTargetRecord> {
    let unresolved = [
        "tmdb:movie:603",
        "tmdb:603",
        "TMDB:Movie:603",
        "tmdb: movie : 603 ",
        "imdb:movie:tt0000603",
        "tmdb:movie:604",
        "tmdb:movie:0604",
        "tvdb:movie:606",
        "tvdb:606",
        "imdb:movie:tt0000607",
        "tmdb:movie:615",
        "tmdb:movie:616",
        "imdb:movie:tt0000614",
        "anilist:anime:608",
        "anilist:series:608",
        "mal:anime:609",
        "mal:movie:609",
        "tmdb:movie:610",
        "imdb:movie:tt0000611",
        "anidb:movie:612",
        "tmdb:movie:617",
        "tmdb:movie:618",
        "tmdb:movie:999",
        "tvdb:series:603",
        "somewhere:movie:603",
        "plain-target",
    ];
    let mut targets = unresolved
        .iter()
        .map(|key| DiscoveryItemTargetRecord {
            target_key: key.to_string(),
            resolved_title_id: None,
        })
        .collect::<Vec<_>>();
    for (key, resolved) in [
        ("tmdb:movie:5000", "probe-unrelated"),
        ("tmdb:movie:5001", "probe-hidden"),
        ("tmdb:movie:5002", "probe-missing"),
    ] {
        targets.push(DiscoveryItemTargetRecord {
            target_key: key.to_string(),
            resolved_title_id: Some(resolved.to_string()),
        });
    }
    targets
}

#[tokio::test]
async fn probe_scoped_ownership_decides_what_the_whole_catalog_decides() {
    let datastore = seeded().await;
    let titles = TitleStore::new(datastore.clone());
    let shows = ShowStore::new(datastore.clone());
    let libraries = vec![READABLE_ANIME.to_string(), READABLE_MOVIES.to_string()];
    let targets = targets();

    let all_titles = titles
        .list_catalog_owned_title_records(&libraries)
        .await
        .unwrap();
    let all_title_ids = all_titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let all_series_movies = shows
        .list_series_movie_links_for_titles(&all_title_ids)
        .await
        .unwrap()
        .iter()
        .map(CatalogOwnedSeriesMovieRecord::from)
        .collect::<Vec<_>>();
    let whole_catalog = catalog_ownership_decisions(&all_titles, &all_series_movies, &targets);

    let probe = catalog_owned_probe_for_targets(&targets);
    let scoped_titles = titles
        .list_catalog_owned_title_records_matching(&libraries, &probe)
        .await
        .unwrap();
    let scoped_series_movies = shows
        .list_catalog_owned_series_movies_matching(&libraries, &probe)
        .await
        .unwrap()
        .expect("the sql store scopes series movies by the probe");
    let scoped = catalog_ownership_decisions(&scoped_titles, &scoped_series_movies, &targets);

    assert_eq!(scoped, whole_catalog);

    // The fixture exercises both answers, and the scoped read skipped the
    // titles no candidate names.
    let owned = targets
        .iter()
        .zip(&whole_catalog.owned)
        .filter(|(_, owned)| **owned)
        .map(|(target, _)| target.target_key.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "tmdb:movie:603",
        "tmdb:movie:604",
        "tvdb:movie:606",
        "imdb:movie:tt0000607",
        "tmdb:movie:615",
        "tmdb:movie:616",
        "imdb:movie:tt0000614",
        "anilist:series:608",
        "tmdb:movie:610",
        "anidb:movie:612",
        "tmdb:movie:5000",
    ] {
        assert!(
            owned.contains(&expected),
            "{expected} should be owned: {owned:?}"
        );
    }
    for unowned in [
        "tmdb:movie:617",
        "tmdb:movie:618",
        "tmdb:movie:999",
        "tmdb:movie:5001",
        "tmdb:movie:5002",
        "plain-target",
    ] {
        assert!(!owned.contains(&unowned), "{unowned} should not be owned");
    }
    let scoped_ids = scoped_titles
        .iter()
        .map(|title| title.id.as_str())
        .collect::<Vec<_>>();
    assert!(!scoped_ids.contains(&"probe-anime-quiet"), "{scoped_ids:?}");
    assert!(!scoped_ids.contains(&"probe-hidden"), "{scoped_ids:?}");
    assert!(
        scoped_ids.contains(&"probe-unrelated"),
        "resolved ids are read"
    );
    assert_eq!(scoped_series_movies.len(), 1, "{scoped_series_movies:?}");
}

#[tokio::test]
async fn probe_scoped_ownership_reads_through_the_0328_indexes() {
    let datastore = seeded().await;
    let StoreDatastore::Sqlite { pool, .. } = &datastore else {
        unreachable!("the fixture is sqlite");
    };
    let sql = catalog_owned_title_records_matching_sql(false, 1);
    let mut rendered = String::new();
    let mut placeholder = 0;
    let args = ["[\"603\"]", "[603]", "[]", "'library-probe-movies'"];
    for (index, part) in sql.split("{}").enumerate() {
        if index > 0 {
            let arg = args[placeholder];
            if arg.starts_with('\'') {
                rendered.push_str(arg);
            } else {
                rendered.push_str(&format!("'{arg}'"));
            }
            placeholder += 1;
        }
        rendered.push_str(part);
    }
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(format!(
        "EXPLAIN QUERY PLAN {rendered}"
    )))
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.3)
    .collect::<Vec<_>>();
    for index in [
        "idx_title_external_ids_value_nocase",
        "idx_title_external_ids_numeric_suffix",
        "idx_titles_imdb_id_nocase",
        "idx_titles_imdb_id_numeric_suffix",
    ] {
        assert!(
            plan.iter().any(|detail| detail.contains(index)),
            "{index} should serve its branch: {plan:#?}"
        );
    }
    assert!(
        !plan
            .iter()
            .any(|detail| detail.starts_with("SCAN titles")
                || detail.starts_with("SCAN external_ids")),
        "no branch scans a catalog table: {plan:#?}"
    );
}
