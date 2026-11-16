//! The wanted list and its count over seeded SQLite data: every filter, the
//! title search, paging, and the row each scope carries.

use std::sync::Arc;

use scryer_application::{AcquisitionScopeStateRepository, AcquisitionScopeStatesQuery};
use scryer_infrastructure_library_search::TitleFuzzyIndex;

use super::catalog_order_tests::fixture_title;
use super::*;
use crate::media::libraries::state_store::{WantedStore, encode_release_decision_explanation};
use crate::media::titles::fuzzy_source::DatastoreTitleTermSource;

/// (id, name, facet)
const TITLES: &[(&str, &str, MediaFacet)] = &[
    ("wl-movie-a", "Amber Road", MediaFacet::Movie),
    ("wl-movie-b", "Copper Lane", MediaFacet::Movie),
    ("wl-series-a", "Amber Harbor", MediaFacet::Series),
    ("wl-series-b", "Granite Hollow", MediaFacet::Series),
    ("wl-anime-a", "Quiet Amber", MediaFacet::Anime),
];

/// (id, title_id, season, episode)
const EPISODES: &[(&str, &str, &str, &str)] = &[
    ("wl-ep-1", "wl-series-a", "1", "1"),
    ("wl-ep-2", "wl-series-a", "1", "2"),
    ("wl-ep-3", "wl-anime-a", "2", "7"),
];

/// (id, title_id, episode_id, media_type, status, last_search_at, grabbed_release, updated_at)
type WantedFixture = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
);
const WANTED: &[WantedFixture] = &[
    (
        "w1",
        "wl-movie-a",
        None,
        "movie",
        "wanted",
        None,
        None,
        "2031-01-01T00:00:01+00:00",
    ),
    (
        "w2",
        "wl-movie-b",
        None,
        "movie",
        "completed",
        Some("2031-01-02T08:00:00+00:00"),
        None,
        "2031-01-01T00:00:02+00:00",
    ),
    (
        "w3",
        "wl-series-a",
        Some("wl-ep-1"),
        "episode",
        "wanted",
        Some("2031-01-03T08:00:00+00:00"),
        None,
        "2031-01-01T00:00:03+00:00",
    ),
    (
        "w4",
        "wl-series-a",
        Some("wl-ep-2"),
        "episode",
        "grabbed",
        None,
        Some(r#"{"title":"Synthetic.Release.S01E02"}"#),
        "2031-01-01T00:00:04+00:00",
    ),
    (
        "w5",
        "wl-anime-a",
        Some("wl-ep-3"),
        "episode",
        "paused",
        None,
        None,
        "2031-01-01T00:00:05+00:00",
    ),
    (
        "w6",
        "wl-series-b",
        None,
        "series",
        "wanted",
        None,
        None,
        "2031-01-01T00:00:06+00:00",
    ),
];

/// How a decision's explanation is stored.
enum Explanation {
    None,
    Encoded(&'static str),
    Undecodable,
}

/// (id, wanted_item_id, title_id, code, url, size, candidate, current, delta, explanation, created_at)
type DecisionFixture = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<i64>,
    i64,
    Option<i64>,
    Option<i64>,
    Explanation,
    &'static str,
);
const DECISIONS: &[DecisionFixture] = &[
    (
        "d1",
        "w1",
        "wl-movie-a",
        "title_mismatch",
        None,
        None,
        0,
        None,
        None,
        Explanation::None,
        "2031-02-01T00:00:00+00:00",
    ),
    (
        "d2",
        "w3",
        "wl-series-a",
        "rejected_quality",
        Some("https://indexer.invalid/a"),
        Some(1_000),
        10,
        Some(5),
        Some(5),
        Explanation::None,
        "2031-02-01T00:00:01+00:00",
    ),
    (
        "d3",
        "w3",
        "wl-series-a",
        "accepted",
        Some("https://indexer.invalid/b"),
        Some(2_000),
        40,
        Some(10),
        Some(30),
        Explanation::Encoded(r#"{"reason":"synthetic","score":40}"#),
        "2031-02-01T00:00:02+00:00",
    ),
    (
        "d4",
        "w4",
        "wl-series-a",
        "accepted",
        None,
        Some(3_000),
        -7,
        None,
        None,
        Explanation::Undecodable,
        "2031-02-01T00:00:03+00:00",
    ),
    (
        "d5",
        "w6",
        "wl-series-b",
        "title_mismatch",
        None,
        None,
        1,
        None,
        None,
        Explanation::None,
        "2031-02-01T00:00:04+00:00",
    ),
    (
        "d6",
        "w6",
        "wl-series-b",
        "title_mismatch",
        None,
        None,
        2,
        None,
        None,
        Explanation::None,
        "2031-02-01T00:00:05+00:00",
    ),
];

async fn seed(datastore: &StoreDatastore, store: &TitleStore) {
    for (id, name, facet) in TITLES {
        let mut title = fixture_title(id, name, None);
        title.facet = facet.clone();
        title.library_id = scryer_domain::default_library_id_for_facet(facet);
        title.root_folder_id =
            super::release_day_tests::root_for(datastore, &title.library_id).await;
        TitleRepository::create(store, title).await.unwrap();
    }
    for (id, title_id, season, episode) in EPISODES {
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO episodes (id, title_id, episode_type, season_number, episode_number, created_at)
             VALUES ({}, {}, 'standard', {}, {}, '2031-01-01T00:00:00+00:00')",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(season.to_string()),
                SqlArg::Text(episode.to_string()),
            ],
        )
        .await
        .unwrap();
    }
    for (id, title_id, episode_id, media_type, status, last_search_at, grabbed, updated_at) in
        WANTED
    {
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO wanted_items
             (id, title_id, episode_id, media_type, last_search_at, status, grabbed_release,
              created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, '2031-01-01T00:00:00+00:00', {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::OptText(episode_id.map(str::to_string)),
                SqlArg::Text(media_type.to_string()),
                SqlArg::OptText(last_search_at.map(str::to_string)),
                SqlArg::Text(status.to_string()),
                SqlArg::OptText(grabbed.map(str::to_string)),
                SqlArg::Text(updated_at.to_string()),
            ],
        )
        .await
        .unwrap();
    }
    for (
        id,
        wanted_item_id,
        title_id,
        code,
        url,
        size,
        candidate,
        current,
        delta,
        explanation,
        created_at,
    ) in DECISIONS
    {
        let explanation = match explanation {
            Explanation::None => None,
            Explanation::Encoded(json) => encode_release_decision_explanation(Some(json)).unwrap(),
            Explanation::Undecodable => Some(vec![2, 3]),
        };
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO release_decisions
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              decision_code, candidate_score, current_score, score_delta, explanation_json,
              created_at)
             VALUES ({}, {}, {}, 'Synthetic.Release', {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(wanted_item_id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::OptText(url.map(str::to_string)),
                SqlArg::OptI64(*size),
                SqlArg::Text(code.to_string()),
                SqlArg::I64(*candidate),
                SqlArg::OptI64(*current),
                SqlArg::OptI64(*delta),
                SqlArg::OptBytes(explanation),
                SqlArg::Text(created_at.to_string()),
            ],
        )
        .await
        .unwrap();
    }
}

fn query() -> AcquisitionScopeStatesQuery {
    AcquisitionScopeStatesQuery {
        limit: 100,
        ..Default::default()
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// (label, query, expected ids in order, expected count)
fn cases() -> Vec<(
    &'static str,
    AcquisitionScopeStatesQuery,
    Vec<&'static str>,
    i64,
)> {
    let series_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    vec![
        (
            "default",
            query(),
            vec!["w6", "w5", "w4", "w3", "w2", "w1"],
            6,
        ),
        (
            "status",
            AcquisitionScopeStatesQuery {
                statuses: strings(&["wanted"]),
                ..query()
            },
            vec!["w6", "w3", "w1"],
            3,
        ),
        (
            "statuses",
            AcquisitionScopeStatesQuery {
                statuses: strings(&["grabbed", "completed"]),
                ..query()
            },
            vec!["w4", "w2"],
            2,
        ),
        (
            "media type",
            AcquisitionScopeStatesQuery {
                media_types: strings(&["episode"]),
                ..query()
            },
            vec!["w5", "w4", "w3"],
            3,
        ),
        (
            "title",
            AcquisitionScopeStatesQuery {
                title_id: Some("wl-series-a".into()),
                ..query()
            },
            vec!["w4", "w3"],
            2,
        ),
        (
            "libraries",
            AcquisitionScopeStatesQuery {
                library_ids: vec![series_library.clone(), anime_library.clone()],
                ..query()
            },
            vec!["w6", "w5", "w4", "w3"],
            4,
        ),
        (
            "decision code",
            AcquisitionScopeStatesQuery {
                latest_decision_codes: strings(&["accepted"]),
                ..query()
            },
            vec!["w4", "w3"],
            2,
        ),
        (
            "mismatch code",
            AcquisitionScopeStatesQuery {
                latest_decision_codes: strings(&["title_mismatch", "rejected_quality"]),
                ..query()
            },
            vec!["w6", "w1"],
            2,
        ),
        (
            "search",
            AcquisitionScopeStatesQuery {
                title_search: Some("amber".into()),
                ..query()
            },
            vec!["w5", "w4", "w3", "w1"],
            4,
        ),
        (
            "search phrase",
            AcquisitionScopeStatesQuery {
                title_search: Some("Amber Road".into()),
                ..query()
            },
            vec!["w1"],
            1,
        ),
        (
            "search typo",
            AcquisitionScopeStatesQuery {
                title_search: Some("granit hollow".into()),
                ..query()
            },
            vec!["w6"],
            1,
        ),
        (
            "search without a match",
            AcquisitionScopeStatesQuery {
                title_search: Some("zzzz".into()),
                ..query()
            },
            vec![],
            0,
        ),
        (
            "search with filters",
            AcquisitionScopeStatesQuery {
                title_search: Some("amber".into()),
                statuses: strings(&["wanted", "grabbed"]),
                media_types: strings(&["episode"]),
                library_ids: vec![series_library.clone()],
                latest_decision_codes: strings(&["accepted"]),
                ..query()
            },
            vec!["w4", "w3"],
            2,
        ),
        (
            "mismatch recovery eligible",
            AcquisitionScopeStatesQuery {
                mismatch_recovery_eligible_only: true,
                ..query()
            },
            vec!["w6", "w1"],
            2,
        ),
        (
            "first page",
            AcquisitionScopeStatesQuery {
                limit: 2,
                ..query()
            },
            vec!["w6", "w5"],
            6,
        ),
        (
            "second page",
            AcquisitionScopeStatesQuery {
                limit: 2,
                offset: 2,
                ..query()
            },
            vec!["w4", "w3"],
            6,
        ),
        (
            "page past the end",
            AcquisitionScopeStatesQuery {
                limit: 2,
                offset: 6,
                ..query()
            },
            vec![],
            6,
        ),
        (
            "searched page",
            AcquisitionScopeStatesQuery {
                title_search: Some("amber".into()),
                limit: 2,
                offset: 1,
                ..query()
            },
            vec!["w4", "w3"],
            4,
        ),
    ]
}

#[tokio::test]
async fn sqlite_wanted_list_filters_pages_and_counts_in_sql() {
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
    let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
    let directory = tempfile::tempdir().unwrap();
    let index = TitleFuzzyIndex::open(
        directory.path(),
        Arc::new(DatastoreTitleTermSource::new(datastore.clone())),
    )
    .await
    .unwrap();
    let titles = TitleStore::new(datastore.clone()).with_fuzzy_index(index.clone());
    seed(&datastore, &titles).await;
    let wanted = WantedStore::new(datastore.clone()).with_fuzzy_index(index.clone());

    for (label, query, expected, expected_count) in cases() {
        let listed = wanted
            .list_acquisition_scope_states(query.clone())
            .await
            .unwrap();
        let count = wanted
            .count_acquisition_scope_states(query.clone())
            .await
            .unwrap();

        assert_eq!(
            listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            expected,
            "{label}"
        );
        assert_eq!(count, expected_count, "{label}");

        // The count leaves the latest-decision join out unless a decision
        // code filters on it; it still counts exactly the rows the list,
        // which always joins the latest decision, returns unpaged.
        let unpaged = wanted
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                limit: i64::MAX,
                offset: 0,
                ..query.clone()
            })
            .await
            .unwrap();
        assert_eq!(count, unpaged.len() as i64, "{label}");
    }

    // The row each scope carries, decision and all.
    let all = wanted.list_acquisition_scope_states(query()).await.unwrap();
    let by_id = |id: &str| all.iter().find(|row| row.id == id).unwrap();

    let episode = by_id("w3");
    assert_eq!(episode.title_name.as_deref(), Some("Amber Harbor"));
    assert_eq!(episode.library_slug.as_deref(), Some("series"));
    assert_eq!(episode.season_number.as_deref(), Some("1"));
    assert_eq!(episode.episode_number.as_deref(), Some("1"));
    assert_eq!(
        episode.last_search_at.as_deref(),
        Some("2031-01-03T08:00:00+00:00")
    );
    assert!(!episode.mismatch_recovery_eligible);
    let decision = episode.latest_release_decision.as_ref().unwrap();
    assert_eq!(decision.id, "d3");
    assert_eq!(decision.decision_code, "accepted");
    assert_eq!(decision.candidate_score, 40);
    assert_eq!(decision.current_score, Some(10));
    assert_eq!(decision.score_delta, Some(30));
    assert_eq!(decision.release_size_bytes, Some(2_000));
    assert_eq!(
        decision
            .explanation_json
            .as_ref()
            .map(|value| value.as_str()),
        Some(r#"{"reason":"synthetic","score":40}"#)
    );

    // An explanation that cannot be decoded reads as none; the decision stays.
    let grabbed = by_id("w4");
    let decision = grabbed.latest_release_decision.as_ref().unwrap();
    assert_eq!(decision.id, "d4");
    assert_eq!(decision.candidate_score, -7);
    assert!(decision.explanation_json.is_none());
    assert_eq!(
        grabbed.grabbed_release.as_deref(),
        Some(r#"{"title":"Synthetic.Release.S01E02"}"#)
    );

    // Only title mismatches on a wanted scope make it eligible for recovery.
    assert!(by_id("w1").mismatch_recovery_eligible);
    assert!(by_id("w6").mismatch_recovery_eligible);
    assert_eq!(
        by_id("w6").latest_release_decision.as_ref().unwrap().id,
        "d6"
    );
    assert!(by_id("w2").latest_release_decision.is_none());
    assert!(!by_id("w5").mismatch_recovery_eligible);
}

/// Seeds a series with `episodes` episode scopes. Episode `n` has `n % 3`
/// decisions, the newest last, so some scopes have none, one, or several.
async fn seed_series_with_many_episodes(
    datastore: &StoreDatastore,
    store: &TitleStore,
    episodes: usize,
) {
    let mut title = fixture_title("wl-series-many", "Slate Meadow", None);
    title.facet = MediaFacet::Series;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    title.root_folder_id = super::release_day_tests::root_for(datastore, &title.library_id).await;
    TitleRepository::create(store, title).await.unwrap();
    for n in 0..episodes {
        let episode_id = format!("wl-many-ep-{n}");
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO episodes (id, title_id, episode_type, season_number, episode_number, created_at)
             VALUES ({}, 'wl-series-many', 'standard', '1', {}, '2031-01-01T00:00:00+00:00')",
            &[SqlArg::Text(episode_id.clone()), SqlArg::Text(n.to_string())],
        )
        .await
        .unwrap();
        let wanted_id = format!("wl-many-w-{n}");
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO wanted_items
             (id, title_id, episode_id, media_type, status, created_at, updated_at)
             VALUES ({}, 'wl-series-many', {}, 'episode', 'wanted',
                     '2031-01-01T00:00:00+00:00', '2031-01-01T00:00:00+00:00')",
            &[SqlArg::Text(wanted_id.clone()), SqlArg::Text(episode_id)],
        )
        .await
        .unwrap();
        for k in 0..(n % 3) {
            let explanation = encode_release_decision_explanation(Some(&format!(
                r#"{{"reason":"synthetic","episode":{n},"attempt":{k}}}"#
            )))
            .unwrap();
            SqlRuntime::execute(
                datastore.read_exec(),
                "INSERT INTO release_decisions
                 (id, wanted_item_id, title_id, release_title, decision_code, candidate_score,
                  explanation_json, created_at)
                 VALUES ({}, {}, 'wl-series-many', 'Synthetic.Release', {}, {}, {}, {})",
                &[
                    SqlArg::Text(format!("wl-many-d-{n}-{k}")),
                    SqlArg::Text(wanted_id.clone()),
                    SqlArg::Text(
                        if k == 0 {
                            "rejected_quality"
                        } else {
                            "accepted"
                        }
                        .to_string(),
                    ),
                    SqlArg::I64((n * 10 + k) as i64),
                    SqlArg::OptBytes(explanation),
                    SqlArg::Text(format!("2031-03-01T00:00:{k:02}+00:00")),
                ],
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn sqlite_latest_decisions_for_kept_scopes_match_the_full_title_read() {
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
    let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
    let titles = TitleStore::new(datastore.clone());
    seed(&datastore, &titles).await;
    seed_series_with_many_episodes(&datastore, &titles, 90).await;
    let wanted = WantedStore::new(datastore.clone());

    let mut title_ids = TITLES
        .iter()
        .map(|(id, _, _)| id.to_string())
        .collect::<Vec<_>>();
    title_ids.push("wl-series-many".to_string());

    // The old read: every row of the titles with its latest decision.
    let mut full = wanted
        .list_acquisition_scope_states_for_title_ids(&title_ids)
        .await
        .unwrap();
    full.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(full.len(), WANTED.len() + 90);

    // The new read: the rows without decisions, then the latest decision of
    // the kept rows only, in one statement and split across several.
    for max_binds in [None, Some(7)] {
        let _lowered = max_binds.map(crate::media::id_list_statements::lower_max_binds_for_tests);
        let mut lean = wanted
            .list_acquisition_scope_states_for_title_ids_without_decisions(&title_ids)
            .await
            .unwrap();
        lean.sort_by(|a, b| a.id.cmp(&b.id));
        assert!(lean.iter().all(|row| row.latest_release_decision.is_none()));
        let ids = lean.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        let mut decisions = wanted
            .list_latest_release_decisions_for_acquisition_scope_states(&ids)
            .await
            .unwrap();
        for row in &mut lean {
            row.latest_release_decision = decisions.remove(&row.id);
        }
        assert!(decisions.is_empty(), "{max_binds:?}");
        assert_eq!(format!("{lean:?}"), format!("{full:?}"), "{max_binds:?}");

        // A subset gets exactly its own decisions.
        let kept = ["wl-many-w-2", "wl-many-w-3", "w3", "w2", "missing-scope"].map(str::to_string);
        let subset = wanted
            .list_latest_release_decisions_for_acquisition_scope_states(&kept)
            .await
            .unwrap();
        let mut subset_ids = subset
            .iter()
            .map(|(scope, decision)| (scope.as_str(), decision.id.as_str()))
            .collect::<Vec<_>>();
        subset_ids.sort();
        assert_eq!(
            subset_ids,
            vec![("w3", "d3"), ("wl-many-w-2", "wl-many-d-2-1")],
            "{max_binds:?}"
        );
    }
}

#[tokio::test]
async fn sqlite_mismatch_recovery_filter_takes_every_decision_and_only_wanted_scopes() {
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
    let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
    let titles = TitleStore::new(datastore.clone());
    // Three episode scopes without decisions, given decisions below.
    seed_series_with_many_episodes(&datastore, &titles, 3).await;
    SqlRuntime::execute(datastore.read_exec(), "DELETE FROM release_decisions", &[])
        .await
        .unwrap();
    SqlRuntime::execute(
        datastore.read_exec(),
        "UPDATE wanted_items SET status = 'grabbed' WHERE id = 'wl-many-w-1'",
        &[],
    )
    .await
    .unwrap();
    let decide = |scope: &'static str, n: usize, code: &'static str, at: String| {
        let datastore = datastore.clone();
        async move {
            SqlRuntime::execute(
                datastore.read_exec(),
                "INSERT INTO release_decisions
                 (id, wanted_item_id, title_id, release_title, decision_code, candidate_score,
                  created_at)
                 VALUES ({}, {}, 'wl-series-many', 'Synthetic.Release', {}, 0, {})",
                &[
                    SqlArg::Text(format!("{scope}-d-{n}")),
                    SqlArg::Text(scope.to_string()),
                    SqlArg::Text(code.to_string()),
                    SqlArg::Text(at),
                ],
            )
            .await
            .unwrap();
        }
    };
    // w-0: ten recent title mismatches over one older rejection.
    decide(
        "wl-many-w-0",
        0,
        "rejected_quality",
        "2031-04-01T00:00:00+00:00".into(),
    )
    .await;
    for n in 1..=10 {
        decide(
            "wl-many-w-0",
            n,
            "title_mismatch",
            format!("2031-04-02T00:00:{n:02}+00:00"),
        )
        .await;
    }
    // w-1: only title mismatches, but grabbed.
    for n in 0..3 {
        decide(
            "wl-many-w-1",
            n,
            "title_mismatch",
            format!("2031-04-02T00:00:{n:02}+00:00"),
        )
        .await;
    }
    // w-2: eleven title mismatches and nothing else.
    for n in 0..11 {
        decide(
            "wl-many-w-2",
            n,
            "title_mismatch",
            format!("2031-04-02T00:00:{n:02}+00:00"),
        )
        .await;
    }

    let wanted = WantedStore::new(datastore.clone());
    let eligible = AcquisitionScopeStatesQuery {
        title_id: Some("wl-series-many".into()),
        mismatch_recovery_eligible_only: true,
        ..query()
    };
    let listed = wanted
        .list_acquisition_scope_states(eligible.clone())
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec!["wl-many-w-2"]
    );
    assert!(listed[0].mismatch_recovery_eligible);
    assert_eq!(
        wanted
            .count_acquisition_scope_states(eligible)
            .await
            .unwrap(),
        1
    );

    // The flag on every row agrees with the filter.
    let all = wanted
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some("wl-series-many".into()),
            ..query()
        })
        .await
        .unwrap();
    let mut flags = all
        .iter()
        .map(|row| (row.id.as_str(), row.mismatch_recovery_eligible))
        .collect::<Vec<_>>();
    flags.sort();
    assert_eq!(
        flags,
        vec![
            ("wl-many-w-0", false),
            ("wl-many-w-1", false),
            ("wl-many-w-2", true)
        ]
    );
}

#[tokio::test]
async fn sqlite_title_scope_tally_counts_every_scope_past_the_old_500_row_read() {
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
    let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
    let titles = TitleStore::new(datastore.clone());
    seed(&datastore, &titles).await;
    const SCOPES: usize = 620;
    seed_series_with_many_episodes(&datastore, &titles, SCOPES).await;
    for n in 0..SCOPES {
        let status = match n % 4 {
            0 => "grabbed",
            1 => "paused",
            _ => "wanted",
        };
        SqlRuntime::execute(
            datastore.read_exec(),
            "UPDATE wanted_items SET status = {}, last_search_at = {} WHERE id = {}",
            &[
                SqlArg::Text(status.to_string()),
                SqlArg::OptText(
                    (n % 7 != 0).then(|| format!("2031-05-{:02}T00:00:00+00:00", 1 + n % 28)),
                ),
                SqlArg::Text(format!("wl-many-w-{n}")),
            ],
        )
        .await
        .unwrap();
        // Scopes without decisions get title mismatches only, so wanted ones
        // among them are eligible for recovery.
        if n % 3 == 0 {
            SqlRuntime::execute(
                datastore.read_exec(),
                "INSERT INTO release_decisions
                 (id, wanted_item_id, title_id, release_title, decision_code, candidate_score,
                  created_at)
                 VALUES ({}, {}, 'wl-series-many', 'Synthetic.Release', 'title_mismatch', 0,
                         '2031-04-01T00:00:00+00:00')",
                &[
                    SqlArg::Text(format!("wl-many-mismatch-{n}")),
                    SqlArg::Text(format!("wl-many-w-{n}")),
                ],
            )
            .await
            .unwrap();
        }
    }
    let wanted = WantedStore::new(datastore.clone());

    // What the diagnostics computed from the rows, without the row cap.
    let rows = wanted
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some("wl-series-many".into()),
            limit: i64::MAX,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), SCOPES);
    let mut expected = std::collections::BTreeMap::<&str, (i64, i64, Option<String>)>::new();
    for row in &rows {
        let entry = expected.entry(row.status.as_str()).or_default();
        entry.0 += 1;
        entry.1 += i64::from(row.mismatch_recovery_eligible);
        entry.2 = entry.2.clone().max(row.last_search_at.clone());
    }
    assert!(expected["wanted"].1 > 0, "fixture has eligible scopes");

    let tallies = wanted
        .tally_acquisition_scope_states_for_title("wl-series-many")
        .await
        .unwrap();
    let actual = tallies
        .into_iter()
        .map(|tally| {
            (
                tally.status.as_str(),
                (
                    tally.count,
                    tally.mismatch_recovery_eligible,
                    tally.latest_search_at,
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.values().map(|tally| tally.0).sum::<i64>(),
        SCOPES as i64
    );

    // Another title's rows stay out of the tally.
    let movie = wanted
        .tally_acquisition_scope_states_for_title("wl-movie-a")
        .await
        .unwrap();
    assert_eq!(movie.len(), 1);
    assert_eq!(movie[0].count, 1);
    assert_eq!(movie[0].mismatch_recovery_eligible, 1);
}
